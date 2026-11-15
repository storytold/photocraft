//! #13871: with View › Snap on, a frame being drawn or an empty frame being moved snaps its edges
//! onto other layers' edges; with Snap off it goes where the pointer does.
use super::*;
use crate::canvas::tool_event;

/// A 600 × 400 document with a layer whose pixels span x 53..136 and y 40..89 (edges at 53, 137,
/// 40 and 90), viewed at 100 % so the snap distance is 8 document pixels.
fn app(snap: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":600,"height":400,"background":"white"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x":53,"y":40,"width":84,"height":50})).unwrap();
    app.run("edit.fill", json!({"color":"#2266aa"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.sync_views();
    app.ui.views[0].zoom = 1.0;
    app.ui.extras.snap = snap;
    app.ui.tool = Tool::Frame;
    app
}

fn frames(app: &PhotocraftApp) -> Vec<[i32; 4]> {
    let st = app.session.active().unwrap();
    st.doc.walk().into_iter().filter_map(|(_, _, l)| l.frame.as_ref().map(|f| f.bounds())).map(|b| [b.x0, b.y0, b.x1, b.y1]).collect()
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    let m = egui::Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, m);
    let mid = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
    tool_event(app, ToolEvent::Move { x: mid[0], y: mid[1], pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, m);
}

#[test]
fn a_frame_drawn_beside_a_layer_takes_its_edges() {
    let mut app = app(true);
    assert!((crate::snap_ui::tolerance(&app) - 8.0).abs() < 1e-9, "100 %: 8 px");
    // The measured case: x 60 to 140 beside a layer spanning x 53..136 (its edges 53 and 137).
    drag(&mut app, [60.0, 200.0], [140.0, 260.0]);
    assert_eq!(frames(&app), vec![[53, 200, 137, 260]]);
}

#[test]
fn an_empty_frame_moved_snaps_its_edges() {
    let mut app = app(false);
    // Clear of the layer, so the frame stays empty (a frame drawn over the active layer's pixels
    // takes them in).
    drag(&mut app, [150.0, 100.0], [190.0, 130.0]);
    assert_eq!(frames(&app), vec![[150, 100, 190, 130]]);
    app.ui.extras.snap = true;
    // -10,-7 by the pointer: the left edge (140) lands on the layer's right edge (137) and the
    // top (93) on its bottom edge (90).
    drag(&mut app, [170.0, 115.0], [160.0, 108.0]);
    assert_eq!(frames(&app), vec![[137, 90, 177, 120]], "moved -13,-10");
    let entries = app.session.active().unwrap().history.entries();
    assert_eq!(entries.last().map(String::as_str), Some("Move"), "one Move step");
}

#[test]
fn with_snap_off_nothing_snaps() {
    let mut app = app(false);
    drag(&mut app, [60.0, 200.0], [140.0, 260.0]);
    assert_eq!(frames(&app), vec![[60, 200, 140, 260]]);
    drag(&mut app, [100.0, 230.0], [120.0, 240.0]);
    assert_eq!(frames(&app), vec![[80, 210, 160, 270]], "moved by the pointer, +20,+10");
}

#[test]
fn far_from_any_edge_the_frame_goes_where_the_pointer_does() {
    let mut app = app(true);
    drag(&mut app, [300.0, 200.0], [380.0, 260.0]);
    assert_eq!(frames(&app), vec![[300, 200, 380, 260]]);
}

#[test]
fn guides_and_the_document_edges_are_not_targets() {
    // Not measured, so they keep today's behaviour: no snapping for the Frame tool.
    let mut app = app(true);
    app.run("view.newGuide", json!({"orientation": "vertical", "position": 304})).unwrap();
    drag(&mut app, [300.0, 4.0], [380.0, 60.0]);
    assert_eq!(frames(&app), vec![[300, 4, 380, 60]], "not onto the guide at 304 nor the top edge");
}
