//! Tests that run paint commands, moved from the engine-core `smart_cmds` tests: 5 test(s).

use photocraft_doc::{LayerContent, SmartObject};
use photocraft_engine::{Result, Session};
use photocraft_geom::Rect;
use serde_json::{Value, json};

const DEPTHS: [u64; 3] = [8, 16, 32];

const W: i32 = 64;

const H: i32 = 48;

/// A `depth`-bit document with one empty layer.
fn session(depth: u64) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": W, "height": H, "depth": depth})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s
}

/// Paints a textured block on the active layer, partly outside the canvas (top-left), so
/// conversion has to keep off-canvas pixels and crop/translate correctly.
fn paint(s: &mut Session) {
    s.edit("setup", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let r = Rect::new(-6, 5, 40, 38);
        let mut data = Vec::new();
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let v = ((x * 7 + y * 13) % 23) as f32 / 22.0;
                let a = if (x + y) % 5 == 0 { 0.5 } else { 1.0 };
                data.extend(photocraft_raster::from_rgba(&fmt, [v, 1.0 - v, ((x * y) % 7) as f32 / 6.0, a]));
            }
        }
        surf.write_region(r, &data);
        Ok(())
    })
    .unwrap();
}

fn flat(s: &Session) -> Vec<[f32; 4]> {
    photocraft_compose::flatten(&s.active().unwrap().doc).into_px()
}

fn active_smart(s: &Session) -> SmartObject {
    let d = s.active().unwrap();
    match &d.doc.layer(d.active_layer.unwrap()).unwrap().content {
        LayerContent::Smart(sm) => sm.clone(),
        other => panic!("not smart: {}", other.kind_name()),
    }
}

fn convert(s: &mut Session) -> u64 {
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap()["layer"].as_u64().unwrap()
}

/// A vertical black stroke over x = 8 into the filter mask, as the Brush tool sends it.
fn stroke(s: &mut Session, target: Option<&str>) -> Result<Value> {
    let mut p = json!({"points": [[8, 2], [8, 46]], "size": 8, "hardness": 1, "color": "#000000"});
    if let Some(t) = target {
        p["target"] = json!(t);
    }
    s.execute("paint.stroke", p)
}

/// ⌥-click on the filter mask thumbnail shows it; painting then goes into it without a target.
#[test]
fn a_shown_filter_mask_takes_the_paint() {
    let mut s = session(8);
    paint(&mut s);
    convert(&mut s);
    s.execute("filter.blur.gaussianBlur", json!({"radius": 3})).unwrap();
    let before = s.active().unwrap().channel_view.clone();
    assert!(s.execute("view.layerMask", json!({"mode": "gray", "mask": "filter"})).is_err(), "no filter mask yet");
    assert_eq!(s.active().unwrap().channel_view, before);
    stroke(&mut s, Some("filterMask")).unwrap();
    let v = s.execute("view.layerMask", json!({"mode": "gray", "mask": "filter"})).unwrap();
    assert_eq!(v["mode"], "gray");
    assert!(s.active().unwrap().channel_view.shown_layer_mask(&s.active().unwrap().doc).is_some());
    s.execute("paint.stroke", json!({"points": [[30, 2], [30, 46]], "size": 8, "hardness": 1, "color": "#000000"})).unwrap();
    let mask = active_smart(&s).filter_mask.unwrap();
    assert_eq!(mask.value(30, 20), 0.0, "the stroke went into the filter mask");
    let d = s.active().unwrap();
    assert!(matches!(d.doc.layer(d.active_layer.unwrap()).unwrap().content, LayerContent::Smart(_)), "still a smart object");
    // Bad params are refused, not a panic.
    let before = s.active().unwrap().channel_view.clone();
    assert!(s.execute("view.layerMask", json!({"mask": 3})).is_err());
    assert_eq!(s.active().unwrap().channel_view, before);
}

#[test]
fn a_shown_filter_mask_wins_over_color_selection_for_cut_and_clear() {
    for depth in DEPTHS {
        for cmd in ["edit.cut", "edit.clear"] {
            let mut s = session(depth);
            paint(&mut s);
            convert(&mut s);
            s.execute("filter.blur.gaussianBlur", json!({"radius":3})).unwrap();
            stroke(&mut s, Some("filterMask")).unwrap();
            s.execute("layer.layerMask.revealAll", json!({})).unwrap();
            let layer = s.active().unwrap().active_layer.unwrap();
            let layer_mask = s.active().unwrap().doc.layer(layer).unwrap().mask.clone();
            let before = active_smart(&s).filter_mask.unwrap();
            s.execute("channel.target", json!({"colors":[0,2]})).unwrap();
            s.execute("view.layerMask", json!({"mode":"gray","mask":"filter"})).unwrap();
            s.tools.background = [0.25, 0.25, 0.25, 1.0];
            let result = s.execute(cmd, json!({})).unwrap();
            if cmd == "edit.cut" {
                assert_eq!(result["target"], "filterMask");
                assert_eq!(s.clipboard.as_ref().unwrap().surface.pixel(8, 20)[0], 0.0);
            }
            assert!((active_smart(&s).filter_mask.unwrap().value(8, 20) - 0.25).abs() < 0.005);
            assert_eq!(s.active().unwrap().doc.layer(layer).unwrap().mask, layer_mask, "{cmd} leaves the layer mask alone");
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(active_smart(&s).filter_mask.unwrap(), before, "undo {cmd} {depth}");
        }
    }
}

#[test]
fn copying_the_filter_mask_never_copies_the_layer_mask() {
    for depth in DEPTHS {
        let mut s = session(depth);
        paint(&mut s);
        convert(&mut s);
        s.execute("filter.blur.gaussianBlur", json!({"radius": 3})).unwrap();
        stroke(&mut s, Some("filterMask")).unwrap();
        for layer_mask in [false, true] {
            if layer_mask {
                s.execute("layer.layerMask.revealAll", json!({})).unwrap();
            }
            let before = s.active().unwrap().doc.clone();
            let steps = s.active().unwrap().history.past_len();
            let result = s.execute("edit.copy", json!({"target": "filterMask"})).unwrap();
            assert_eq!(result["target"], "filterMask");
            let clip = s.clipboard.as_ref().unwrap();
            assert_eq!(clip.surface.rgba(8, 20), [0.0, 0.0, 0.0, 1.0]);
            assert_eq!(clip.surface.rgba(30, 20), [1.0; 4]);
            assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &before));
            assert_eq!(s.active().unwrap().history.past_len(), steps);
        }
    }
}

#[test]
fn an_explicit_layer_checks_filter_mask_eligibility() {
    let mut s = session(8);
    paint(&mut s);
    let id = convert(&mut s);
    s.execute("filter.blur.gaussianBlur", json!({"radius": 3})).unwrap();
    s.execute("shape.create", json!({"kind": "rect", "rect": [1, 1, 20, 20], "fill": "#ff0000"})).unwrap();
    let active = s.active().unwrap().active_layer;
    s.execute("paint.cloneStamp", json!({"layer": id, "target": "filterMask", "source": [8, 8], "points": [[20, 20]], "size": 4})).unwrap();
    assert_eq!(s.active().unwrap().active_layer, active);
    // A shape has no filter mask: rejection must precede recoloring its fill.
    let before = s.active().unwrap().doc.clone();
    let steps = s.active().unwrap().history.past_len();
    assert!(s.execute("edit.fillForeground", json!({"layer": active.unwrap().0, "target": "filterMask"})).is_err());
    assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &before));
    assert_eq!(s.active().unwrap().history.past_len(), steps);
}

/// Gaussian Blur with the filter mask targeted softens the mask's edge (the usual way to feather
/// it), re-renders the smart object, and adds no second smart filter.
#[test]
fn blurring_the_filter_mask_feathers_it() {
    for depth in DEPTHS {
        let mut s = session(depth);
        paint(&mut s);
        convert(&mut s);
        s.execute("filter.blur.gaussianBlur", json!({"radius": 3})).unwrap();
        stroke(&mut s, Some("filterMask")).unwrap();
        let hard = active_smart(&s).filter_mask.unwrap();
        let edge = (9..W).find(|&x| hard.value(x, 20) == 1.0).expect("white right of the stroke");
        let px = flat(&s);
        let steps = s.active().unwrap().history.past_len();

        s.execute("filter.blur.gaussianBlur", json!({"radius": 3, "target": "filterMask"})).unwrap();
        let soft = active_smart(&s).filter_mask.unwrap();
        let v = soft.value(edge, 20);
        assert!(v > 0.0 && v < 1.0, "depth {depth}: the mask edge is soft, got {v}");
        assert_eq!(soft.value(30, 20), 1.0, "depth {depth}: far from the stroke stays white");
        assert_eq!(active_smart(&s).smart_filters.len(), 1, "depth {depth}: no second smart filter");
        assert_ne!(flat(&s), px, "depth {depth}: the smart object is re-rendered");
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "depth {depth}: one step");
    }
}
