//! FILE-215-3: commands on a smart object's filter mask (`"target":"filterMask"`), and the mask
//! saved with the document. Painting it with the brush: `brush_cmds/filter_mask_stroke_tests.rs`.

use super::*;

/// A vertical black stroke over x = 8 into the filter mask, as the Brush tool sends it.
fn stroke(s: &mut Session, target: Option<&str>) -> Result<Value> {
    let mut p = json!({"points": [[8, 2], [8, 46]], "size": 8, "hardness": 1, "color": "#000000"});
    if let Some(t) = target {
        p["target"] = json!(t);
    }
    s.execute("paint.stroke", p)
}

/// The filter mask a vertical black stroke of width 8 over x = 8 leaves, set as the paint command
/// would: a white mask is made first, then the column is blacked and the object re-rendered.
fn black_filter_mask_column(s: &mut Session) {
    let id = s.active().unwrap().active_layer.unwrap();
    s.edit("setup", |doc, _| {
        let mask = crate::smart_cmds::filter_mask_surface(doc, id)?;
        let black = photocraft_raster::from_rgba(&mask.format(), [0.0, 0.0, 0.0, 1.0]);
        let r = Rect::new(4, 2, 12, 46);
        let data: Vec<f32> = (0..(r.width() * r.height()) as usize).flat_map(|_| black.iter().copied()).collect();
        mask.write_region(r, &data);
        crate::smart_cmds::refresh_after_filter_mask_edit(doc, id)
    })
    .unwrap();
}

/// The filter mask is saved: .pcraft and PSD give it back.
#[test]
fn a_filter_mask_survives_pcraft_and_psd() {
    for depth in DEPTHS {
        let mut s = session(depth);
        paint(&mut s);
        convert(&mut s);
        s.execute("filter.blur.gaussianBlur", json!({"radius": 3})).unwrap();
        black_filter_mask_column(&mut s);
        let mask = active_smart(&s).filter_mask.expect("a filter mask");
        let doc = s.active().unwrap().doc.clone();
        let id = s.active().unwrap().active_layer.unwrap();
        let back = photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap()).unwrap();
        assert_eq!(smart(&back, id).unwrap().filter_mask, smart(&doc, id).unwrap().filter_mask, "depth {depth}: .pcraft");
        let psd = photocraft_io::export(&doc, "psd", &Default::default()).unwrap();
        let back = photocraft_io::import("x.psd", &psd.bytes).unwrap().document;
        let sm = back.walk().into_iter().find_map(|(_, _, l)| if let LayerContent::Smart(sm) = &l.content { Some(sm.clone()) } else { None }).unwrap();
        let (mut a, mut b) = (Vec::new(), Vec::new());
        sm.filter_mask.as_ref().unwrap().values_into(doc.bounds(), &mut a);
        mask.values_into(doc.bounds(), &mut b);
        assert_eq!(a, b, "depth {depth}: PSD");
    }
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

/// With its filter mask targeted, the commands that don't edit a filter mask act on the smart
/// object as without the target: Edit › Transform flips it, and Convert for Smart Filters stays
/// disabled (it would nest the smart object).
#[test]
fn the_filter_mask_target_leaves_the_smart_object_commands_alone() {
    let mut s = session(8);
    paint(&mut s);
    convert(&mut s);
    s.execute("filter.blur.gaussianBlur", json!({"radius": 3})).unwrap();
    black_filter_mask_column(&mut s);
    let steps = s.active().unwrap().history.past_len();
    assert!(
        matches!(s.execute("filter.convertForSmartFilters", json!({"target": "filterMask"})), Err(EngineError::Disabled(..))),
        "the layer is already a smart object"
    );
    assert_eq!(s.active().unwrap().history.past_len(), steps);
    let before = active_smart(&s).transform;
    s.execute("edit.transform.flipHorizontal", json!({"target": "filterMask"})).unwrap();
    assert_ne!(active_smart(&s).transform, before, "the smart object flips");
    // A shown filter mask doesn't send them the target either.
    s.execute("view.layerMask", json!({"mode": "gray", "mask": "filter"})).unwrap();
    assert_eq!(crate::channel_cmds::inject_target(&s, "edit.transform.flipHorizontal", json!({})), json!({}));
    assert_eq!(crate::channel_cmds::inject_target(&s, "filter.convertForSmartFilters", json!({})), json!({}));
    assert_eq!(crate::channel_cmds::inject_target(&s, "paint.stroke", json!({}))["target"], "filterMask");
    // Curves on the shown filter mask edits that gray plane, not the layer mask or the colours.
    let curve = crate::channel_cmds::isolated_curve_channel(s.active().unwrap()).expect("a gray target");
    assert_eq!(curve.gray, Some(crate::channel_cmds::GraySurface::FilterMask));
    let flipped = active_smart(&s).transform;
    s.execute("edit.transform.flipHorizontal", json!({})).unwrap();
    assert_ne!(active_smart(&s).transform, flipped, "the smart object flips again");
    // Moving the smart object doesn't re-render its filters as a filter-mask edit.
    let p = json!({"dx": 2, "target": "filterMask"});
    assert_eq!(crate::channel_cmds::filter_mask_layer(&s, "layer.translate", &p), None);
    assert!(crate::channel_cmds::filter_mask_layer(&s, "paint.stroke", &p).is_some());
    // Every other routed command edits it and re-renders the object: Equalize too, though it
    // takes no layer mask target by default.
    for cmd in ["image.adjustments.shadowsHighlights", "plugin.run", "filter.blur.average"] {
        assert!(crate::channel_cmds::filter_mask_layer(&s, cmd, &p).is_some(), "{cmd}");
    }
    let steps = s.active().unwrap().history.past_len();
    s.execute("image.adjustments.equalize", json!({"target": "filterMask"})).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
    assert_eq!(active_smart(&s).smart_filters.len(), 1, "no new smart filter: the filter mask was equalized");
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
