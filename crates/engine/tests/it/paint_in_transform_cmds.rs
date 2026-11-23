//! Tests that run paint commands, moved from the engine-core `transform_cmds` tests: 3 test(s).

use photocraft_color::PixelFormat;
use photocraft_engine::Session;
use photocraft_engine::transform_cmds::*;
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::json;

fn session() -> Session {
    session_depth(8)
}

fn session_depth(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 100, "height": 100, "depth": depth})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit("paint", |doc, active| {
        let l = doc.layer_mut(active.unwrap()).unwrap();
        l.surface_mut().unwrap().fill_rect(Rect::new(10, 10, 30, 20), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn feathered_selection_identity_transform_preserves_pixels() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "background": "transparent"})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.rect", json!({"x": 4, "y": 4, "width": 8, "height": 8})).unwrap();
    s.execute("select.modify.feather", json!({"radius": 1})).unwrap();

    let mut before_px = Vec::new();
    for y in 0..16 {
        for x in 0..16 {
            before_px.push(s.execute("document.pixel", json!({"x": x, "y": y})).unwrap());
        }
    }

    s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 0, 0], "interpolation": "nearest"})).unwrap();

    let mut after_px = Vec::new();
    for y in 0..16 {
        for x in 0..16 {
            after_px.push(s.execute("document.pixel", json!({"x": x, "y": y})).unwrap());
        }
    }

    assert_eq!(before_px, after_px, "feathered selection identity transform must preserve all pixels");
    let p4_8 = s.execute("document.pixel", json!({"x": 4, "y": 8})).unwrap();
    assert_eq!(p4_8, json!([1.0, 0.0, 0.0, 1.0]), "(4,8) must remain opaque red");

    // Undo restores original document.
    s.execute("edit.undo", json!({})).unwrap();
    let undo_p4_8 = s.execute("document.pixel", json!({"x": 4, "y": 8})).unwrap();
    assert_eq!(undo_p4_8, json!([1.0, 0.0, 0.0, 1.0]));

    // Locked layer still refuses transform even if identity.
    s.execute("layer.setProps", json!({"locks": {"position": true}})).unwrap();
    assert!(s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 0, 0]})).is_err(), "locked layer refuses identity transform");
}

#[test]
fn relative_params_scale_and_rotate_about_the_frame_centre() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 100, "height": 100})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 20, "y": 30, "width": 40, "height": 20})).unwrap();
    s.execute("edit.fill", json!({"contents": "black"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    // 50% about the centre (40, 40), then 10 px right: the content ends at x 40..60, y 35..45.
    let r = s.execute("edit.transform", json!({"scaleX": 50, "scaleY": 50, "dx": 10})).unwrap();
    let rect: Vec<f64> = serde_json::from_value(r["rect"].clone()).unwrap();
    assert_eq!(rect, [20.0, 30.0, 60.0, 50.0], "the reported frame is the source frame");
    let bounds = |s: &Session| {
        let d = s.active().unwrap();
        let b = transform_bounds(&d.doc, d.doc.layer(d.active_layer.unwrap()).unwrap());
        (b.x0, b.y0, b.x1, b.y1)
    };
    assert_eq!(bounds(&s), (40, 35, 60, 45));
    // A quarter turn about the centre (50, 40) swaps the frame's sides.
    s.execute("edit.transform", json!({"rotate": 90})).unwrap();
    assert_eq!(bounds(&s), (45, 30, 55, 50));
    for bad in [json!({"scaleX": 0}), json!({"rotate": "x"}), json!({"dx": f64::NAN})] {
        assert!(s.execute("edit.transform", bad.clone()).is_err(), "{bad}");
    }
}

/// Free Transform of a layer's own transparency (⌘-click its thumbnail) takes all of it and leaves
/// no ghost where it was, as Photoshop does and as Move already did (#5595). Photoshop 2025, an
/// anti-aliased 60 px ellipse moved 150 px with Free Transform: no pixel left behind.
#[test]
fn transforming_a_layers_own_transparency_leaves_no_ghost() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 300, "height": 120, "background": "transparent"})).unwrap();
    s.execute("select.rect", json!({"x": 20, "y": 20, "width": 60, "height": 60, "ellipse": true, "antiAlias": true})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#cc2222"})).unwrap();
    s.execute("select.loadSelection", json!({"channel": "transparency"})).unwrap();
    let alpha_sum = |s: &Session, area: Rect| -> f32 {
        let st = s.active().unwrap();
        let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
        surf.read_region(area).as_chunks::<4>().0.iter().map(|p| p[3]).sum()
    };
    let (old, new) = (Rect::new(0, 0, 110, 120), Rect::new(110, 0, 300, 120));
    let before = alpha_sum(&s, old);
    assert!(before > 2000.0, "the ellipse: {before}");
    s.execute("edit.transform", json!({"dx": 150, "dy": 0})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    let st = s.active().unwrap();
    let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    let left = surf.read_region(old).as_chunks::<4>().0.iter().filter(|p| p[3] > 0.0).count();
    assert_eq!(left, 0, "pixels left behind at the old spot");
    let moved = alpha_sum(&s, new);
    assert!((moved - before).abs() < 1.0, "all the coverage moved: {before} → {moved}");
    // An opaque pixel half selected still splits half and half.
    let mut s = session();
    s.edit("half", |doc, _| {
        let mut sel = Surface::new(PixelFormat::GRAY8);
        sel.fill_rect(Rect::new(10, 10, 30, 20), &[0.5]);
        doc.selection = Some(sel);
        Ok(())
    })
    .unwrap();
    s.execute("edit.transform", json!({"dx": 40, "dy": 0})).unwrap();
    let st = s.active().unwrap();
    let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    let rest = surf.read_region(Rect::new(15, 15, 16, 16));
    assert!((rest[3] - 0.5).abs() < 0.01, "half the opaque pixel stays: {rest:?}");
}
