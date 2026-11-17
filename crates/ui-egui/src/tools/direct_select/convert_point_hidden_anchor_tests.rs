//! #12743: while Convert Point is active, a drag on an anchor that is not shown moves it, as
//! Direct Selection would, and a click on empty canvas hides the targeted path's anchors. The
//! issue's two measured documents: the 800x600 document with saved path "Path 1" (closed square
//! (100,300) (300,300) (300,500) (100,500)) targeted in the Paths panel, and the solid-colour
//! layer "Shape" with a vector mask (closed square (450,100) (650,100) (650,250) (450,250)).

use egui::Modifiers;
use serde_json::json;

use super::anchors_shown;
use super::convert_point_history_tests::{click, drag};
use super::PathRef;
use crate::PhotocraftApp;
use crate::anchor_tools::delete_anchor_targeted_path_tests::steps;
use crate::state::Tool;

fn new_doc(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.sync_views();
    app.ui.tool = tool;
    app
}

/// Saved path "Path 1", targeted in the Paths panel: the issue's reference document.
fn saved_doc() -> PhotocraftApp {
    let mut app = new_doc(Tool::ConvertPoint);
    let path = json!({"subpaths": [{"closed": true, "knots": [[100, 300], [300, 300], [300, 500], [100, 500]]}]});
    app.run("path.set", json!({"name": "Path 1", "path": path})).unwrap();
    app.ui.selected_path = Some("Path 1".into());
    app
}

/// The solid-colour layer "Shape" with the vector-mask square, the mask targeted.
fn mask_doc(tool: Tool) -> (PhotocraftApp, u64) {
    let mut app = new_doc(tool);
    app.run("layer.new.layer", json!({})).unwrap();
    let id = app.session.active().unwrap().active_layer.unwrap().0;
    app.run("layer.renameLayer", json!({"layer": id, "name": "Shape"})).unwrap();
    let rect = json!({"subpaths": [{"closed": true, "knots": [[450, 100], [650, 100], [650, 250], [450, 250]]}]});
    app.run("layer.vectorMask.add", json!({"layer": id, "path": rect})).unwrap();
    app.ui.vector_mask_target = true;
    (app, id)
}

fn saved_knot(app: &PhotocraftApp, k: usize) -> photocraft_doc::Knot {
    app.session.active().unwrap().doc.paths[0].path.subpaths[0].knots[k].clone()
}

fn mask_knot(app: &PhotocraftApp, k: usize) -> photocraft_doc::Knot {
    let st = app.session.active().unwrap();
    st.active_layer.and_then(|id| st.doc.layer(id)).unwrap().vector_mask.as_ref().unwrap().path.subpaths[0].knots[k].clone()
}

/// Both control points sit on the anchor: the handles are retracted, a corner.
fn corner(k: &photocraft_doc::Knot) -> bool {
    k.in_ctrl == k.anchor && k.out_ctrl == k.anchor
}

// ---- the measured rules ----

#[test]
fn a_click_on_empty_canvas_hides_the_targeted_paths_anchors_and_keeps_it_targeted() {
    let mut app = saved_doc();
    assert!(anchors_shown(&app, PathRef::Saved(0), 0), "right after targeting, the anchors are drawn");
    let n = steps(&app);
    click(&mut app, [400.0, 100.0], Modifiers::NONE);
    assert_eq!(steps(&app), n, "no History step");
    assert_eq!(app.ui.selected_path.as_deref(), Some("Path 1"), "the path stays targeted");
    assert_eq!(app.ui.direct_selection.target, Some(PathRef::Saved(0)));
    assert!(!anchors_shown(&app, PathRef::Saved(0), 0), "its anchors are hidden");
}

#[test]
fn a_drag_from_a_hidden_anchor_moves_it_as_one_drag_anchor_point_step_and_shows_them_again() {
    let mut app = saved_doc();
    click(&mut app, [400.0, 100.0], Modifiers::NONE);
    let n = steps(&app);
    drag(&mut app, [300.0, 300.0], [360.0, 325.0], Modifiers::NONE);
    let knot = saved_knot(&app, 1);
    assert_eq!((knot.anchor.x, knot.anchor.y), (360.0, 325.0), "the anchor moved");
    assert!(corner(&knot), "no handles");
    assert_eq!(steps(&app), n + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Drag Anchor Point"));
    assert!(anchors_shown(&app, PathRef::Saved(0), 0), "the anchors are drawn again afterwards");
    // Once they are drawn, the next drag converts (the issue's last Done-when bullet).
    drag(&mut app, [360.0, 325.0], [400.0, 290.0], Modifiers::NONE);
    let knot = saved_knot(&app, 1);
    assert_eq!((knot.anchor.x, knot.anchor.y), (360.0, 325.0), "the second drag did not move it");
    assert_eq!((knot.out_ctrl.x, knot.out_ctrl.y), (400.0, 290.0), "it converted");
    assert_eq!((knot.in_ctrl.x, knot.in_ctrl.y), (320.0, 360.0), "its in-handle mirrored");
}

#[test]
fn the_first_drag_on_a_targeted_vector_mask_moves_the_anchor_and_the_next_drag_converts() {
    let (mut app, id) = mask_doc(Tool::ConvertPoint);
    let n = steps(&app);
    drag(&mut app, [650.0, 100.0], [690.0, 70.0], Modifiers::NONE);
    let knot = mask_knot(&app, 1);
    assert_eq!((knot.anchor.x, knot.anchor.y), (690.0, 70.0), "the mask anchor moved");
    assert!(corner(&knot), "no handles");
    assert_eq!(steps(&app), n + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Drag Anchor Point"));
    assert!(anchors_shown(&app, PathRef::Layer(id), 0), "the anchors are drawn after the move");
    // The next drag converts: the anchors are drawn now.
    drag(&mut app, [690.0, 70.0], [730.0, 50.0], Modifiers::NONE);
    let knot = mask_knot(&app, 1);
    assert_eq!((knot.anchor.x, knot.anchor.y), (690.0, 70.0), "the second drag did not move it");
    assert_eq!((knot.out_ctrl.x, knot.out_ctrl.y), (730.0, 50.0), "it converted");
    assert_eq!((knot.in_ctrl.x, knot.in_ctrl.y), (650.0, 90.0), "its in-handle mirrored");
}

#[test]
fn a_click_on_a_hidden_anchor_still_converts_it_to_a_corner() {
    let mut app = saved_doc();
    // Work the path so its anchors are drawn, then hide them again.
    drag(&mut app, [300.0, 300.0], [360.0, 325.0], Modifiers::NONE);
    assert!(anchors_shown(&app, PathRef::Saved(0), 0));
    let n = steps(&app);
    click(&mut app, [400.0, 100.0], Modifiers::NONE);
    assert_eq!(steps(&app), n);
    assert!(!anchors_shown(&app, PathRef::Saved(0), 0), "the anchors are hidden");
    // A click without movement on the hidden anchor (300,500) converts, as shown anchors do.
    click(&mut app, [300.0, 500.0], Modifiers::NONE);
    assert_eq!(steps(&app), n + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Change Anchor Point"));
    let knot = saved_knot(&app, 3);
    assert_eq!((knot.anchor.x, knot.anchor.y), (300.0, 500.0), "the click did not move it");
    assert!(corner(&knot), "the point is a corner");
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 3]], "the tool selected the anchor it converted");
}

#[test]
fn with_the_anchors_drawn_a_drag_from_an_anchor_converts_as_today() {
    // Guard test, passes before and after: the saved path's anchors are drawn right after
    // targeting, so the first drag pulls out handles exactly as measured in #12736 and #12748.
    let mut app = saved_doc();
    let n = steps(&app);
    drag(&mut app, [300.0, 300.0], [360.0, 325.0], Modifiers::NONE);
    let knot = saved_knot(&app, 1);
    assert_eq!((knot.anchor.x, knot.anchor.y), (300.0, 300.0), "the anchor stayed");
    assert_eq!((knot.out_ctrl.x, knot.out_ctrl.y), (360.0, 325.0), "the out handle followed the pointer");
    assert_eq!((knot.in_ctrl.x, knot.in_ctrl.y), (240.0, 275.0), "the in-handle mirrored");
    assert_eq!(steps(&app), n + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Change Anchor Point"));
}

// ---- neighbours: what must NOT change ----

#[test]
fn direct_selection_on_a_targeted_mask_moves_anchors_and_empty_canvas_hides_nothing() {
    // Guard test, passes before and after: the Direct Selection tool keeps its own shown state.
    let (mut app, id) = mask_doc(Tool::DirectSelection);
    let n = steps(&app);
    drag(&mut app, [650.0, 100.0], [690.0, 70.0], Modifiers::NONE);
    let knot = mask_knot(&app, 1);
    assert_eq!((knot.anchor.x, knot.anchor.y), (690.0, 70.0), "Direct Selection moved it");
    assert!(corner(&knot));
    assert_eq!(steps(&app), n + 1);
    click(&mut app, [400.0, 100.0], Modifiers::NONE);
    assert_eq!(steps(&app), n + 1, "a click on empty canvas adds no step");
    assert!(anchors_shown(&app, PathRef::Layer(id), 0), "Direct Selection keeps its anchors drawn");
}

#[test]
fn the_pen_with_alt_still_converts_a_mask_anchor() {
    // Guard test, passes before and after: Pen + ⌥ is not the Convert Point tool.
    let (mut app, _) = mask_doc(Tool::Pen);
    let n = steps(&app);
    drag(&mut app, [650.0, 100.0], [690.0, 70.0], Modifiers::ALT);
    let knot = mask_knot(&app, 1);
    assert_eq!((knot.anchor.x, knot.anchor.y), (650.0, 100.0), "the anchor stayed");
    assert_eq!((knot.out_ctrl.x, knot.out_ctrl.y), (690.0, 70.0), "it converted");
    assert_eq!((knot.in_ctrl.x, knot.in_ctrl.y), (610.0, 130.0));
    assert_eq!(steps(&app), n + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Convert Point"), "the Pen keeps the command name");
}

#[test]
fn command_with_convert_point_on_a_mask_anchor_moves_as_direct_selection() {
    // Guard test, passes before and after: ⌘/Ctrl with the tool is Direct Selection (#790).
    let (mut app, _) = mask_doc(Tool::ConvertPoint);
    let n = steps(&app);
    drag(&mut app, [650.0, 100.0], [690.0, 70.0], Modifiers::COMMAND);
    let knot = mask_knot(&app, 1);
    assert_eq!((knot.anchor.x, knot.anchor.y), (690.0, 70.0), "⌘ moved it");
    assert!(corner(&knot));
    assert_eq!(steps(&app), n + 1);
}

#[test]
fn shift_or_alt_on_a_mask_anchor_keep_todays_convert() {
    // Guard tests, pass before and after: modifier presses were not measured (#12743 "not
    // measured: Shift ..."), and ⌥ belongs to the unmerged #12740 work.
    for mods in [Modifiers::SHIFT, Modifiers::ALT] {
        let (mut app, _) = mask_doc(Tool::ConvertPoint);
        let n = steps(&app);
        drag(&mut app, [650.0, 100.0], [690.0, 70.0], mods);
        let knot = mask_knot(&app, 1);
        assert_eq!((knot.anchor.x, knot.anchor.y), (650.0, 100.0), "{mods:?}: the anchor stayed");
        assert_eq!((knot.out_ctrl.x, knot.out_ctrl.y), (690.0, 70.0), "{mods:?}: it converted as today");
        assert_eq!((knot.in_ctrl.x, knot.in_ctrl.y), (610.0, 130.0), "{mods:?}: both handles, as main has it");
        assert_eq!(steps(&app), n + 1);
    }
}

#[test]
fn a_drag_from_the_corner_of_an_active_shape_layer_still_converts() {
    // Guard test, passes before and after: an active SHAPE layer's path (no targeted vector
    // mask) starts with its anchors drawn, as #12735 measured.
    let (mut app, shape) = crate::anchor_tools::delete_anchor_targeted_path_tests::setup(Tool::ConvertPoint);
    crate::anchor_tools::delete_anchor_targeted_path_tests::select_layer(&mut app, shape);
    let n = steps(&app);
    drag(&mut app, [450.0, 100.0], [490.0, 70.0], Modifiers::NONE);
    let path = crate::anchor_tools::delete_anchor_targeted_path_tests::shape_path(&mut app, shape);
    let knots = path["subpaths"][0]["knots"].as_array().unwrap();
    let knot = knots.iter().find(|k| k["anchor"] == json!([450.0, 100.0])).unwrap();
    assert_eq!(knot["out"], json!([490.0, 70.0]), "the shape path converted");
    assert_eq!(knot["in"], json!([410.0, 130.0]));
    assert_eq!(steps(&app), n + 1);
}
