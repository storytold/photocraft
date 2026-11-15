//! #12491: entering Warp again on a smart object warped with a preset shows that preset (style,
//! Bend, H, V and orientation), not Custom with a mesh; committing a changed Bend stores the
//! preset again. A stored custom mesh still opens as Custom, and a rotated placement shows the
//! preset through its mesh as before.

use serde_json::json;

use super::super::tests::app_with_square;
use super::super::*;

/// A 64 × 64 document whose red square 8,8 to 32,32 is a smart object warped with `params`.
fn warped(params: serde_json::Value) -> PhotocraftApp {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 32, 32));
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    app.run("layer.smartObjects.warp", params).unwrap();
    app
}

fn session_warp(app: &PhotocraftApp) -> Warp {
    app.ui.transform.as_ref().and_then(|t| t.warp.clone()).expect("Warp mode")
}

fn stored(app: &PhotocraftApp) -> Warp {
    let st = app.session.active().unwrap();
    match &st.doc.layer(st.active_layer.unwrap()).unwrap().content {
        photocraft_doc::LayerContent::Smart(sm) => sm.warp.clone().expect("a stored warp"),
        _ => panic!("not a smart object"),
    }
}

#[test]
fn re_entering_shows_the_stored_preset() {
    let ctx = egui::Context::default();
    for (style, s) in [("arc", WarpStyle::Arc), ("flag", WarpStyle::Flag), ("twist", WarpStyle::Twist), ("fisheye", WarpStyle::Fisheye)] {
        let mut app = warped(json!({"style": style, "bend": 50, "hDistort": 10, "vDistort": -20, "rect": [8, 8, 32, 32]}));
        begin_warp(&mut app, &ctx).unwrap();
        let w = session_warp(&app);
        assert_eq!((w.style, w.bend, w.h_distort, w.v_distort, w.vertical), (s, 50.0, 10.0, -20.0, false), "{style}");
        assert!(w.mesh.is_none(), "{style}: a preset, not a mesh");
        assert_eq!(w.bounds, [8.0, 8.0, 32.0, 32.0], "{style}: over the box");
    }
    let mut app = warped(json!({"style": "arc", "bend": -30, "vertical": true, "rect": [8, 8, 32, 32]}));
    begin_warp(&mut app, &ctx).unwrap();
    let w = session_warp(&app);
    assert_eq!((w.style, w.bend, w.vertical), (WarpStyle::Arc, -30.0, true), "vertical kept");
}

#[test]
fn a_changed_bend_commits_as_the_preset() {
    let ctx = egui::Context::default();
    let mut app = warped(json!({"style": "arc", "bend": 50, "rect": [8, 8, 32, 32]}));
    begin_warp(&mut app, &ctx).unwrap();
    app.ui.transform.as_mut().unwrap().warp.as_mut().unwrap().bend = 25.0;
    commit(&mut app);
    let w = stored(&app);
    assert_eq!((w.style, w.bend), (WarpStyle::Arc, 25.0), "stored as Arc, not Custom");
    assert!(w.mesh.is_none());
}

#[test]
fn a_stored_custom_mesh_still_opens_as_custom() {
    let ctx = egui::Context::default();
    // An Arc's mesh stored as a custom warp, as a scripted transform with a preset stores it.
    let bounds = [8.0, 8.0, 32.0, 32.0];
    let custom = Warp::custom(Warp::preset(WarpStyle::Arc, 50.0, bounds).to_mesh(1, 1), bounds);
    let mut app = warped(json!({"warp": custom, "rect": bounds}));
    assert_eq!(stored(&app).style, WarpStyle::Custom);
    begin_warp(&mut app, &ctx).unwrap();
    assert_eq!(session_warp(&app).style, WarpStyle::Custom);
}

#[test]
fn a_rotated_placement_keeps_showing_the_mesh() {
    let ctx = egui::Context::default();
    let mut app = warped(json!({"style": "arc", "bend": 50, "rect": [8, 8, 32, 32]}));
    app.run("edit.transform", json!({"rotate": 30})).unwrap();
    begin_warp(&mut app, &ctx).unwrap();
    assert_eq!(session_warp(&app).style, WarpStyle::Custom, "the preset through its rotated placement, as today");
}

/// CodeRabbit on #15010: the arc family (Arc, Arch, Bulge, Arc Lower and Upper, Shell Lower and
/// Upper) depends on its box's aspect, so under a placement that scales x and y differently the
/// preset over the placed box would be a different shape from the one the canvas draws (the stored
/// source-space preset stretched through the placement). They show the mesh then; the other
/// presets don't use the aspect and keep their preset path.
#[test]
fn an_uneven_scale_shows_the_arc_family_as_its_mesh_and_keeps_the_other_presets() {
    let ctx = egui::Context::default();
    for (style, kept) in [("arc", false), ("arch", false), ("bulge", false), ("shellLower", false), ("flag", true), ("fisheye", true), ("twist", true)] {
        let mut app = warped(json!({"style": style, "bend": 50, "rect": [8, 8, 32, 32]}));
        app.run("edit.transform", json!({"scaleX": 200, "scaleY": 100})).unwrap();
        begin_warp(&mut app, &ctx).unwrap();
        let w = session_warp(&app);
        assert_eq!(w.style != WarpStyle::Custom, kept, "{style}: preset kept = {kept}, got {:?}", w.style);
    }
    // An even scale keeps the arc family a preset.
    let mut app = warped(json!({"style": "arc", "bend": 50, "rect": [8, 8, 32, 32]}));
    app.run("edit.transform", json!({"scaleX": 150, "scaleY": 150})).unwrap();
    begin_warp(&mut app, &ctx).unwrap();
    assert_eq!(session_warp(&app).style, WarpStyle::Arc, "uniform scale stays on the preset path");
}
