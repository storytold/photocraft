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

fn click(app: &mut PhotocraftApp, at: [f64; 2]) {
    let m = egui::Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: at[0], y: at[1], pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Up { x: at[0], y: at[1] }, m);
}

/// CodeRabbit on #15141: what a press hits is decided at the pointer, not where Snap puts a new frame's
/// corner. A press just outside an empty frame, whose edge it snaps onto, starts a new frame; it does not
/// edit (move or resize) the frame it snapped to.
#[test]
fn a_press_that_snaps_onto_an_existing_frames_edge_still_draws_a_new_frame() {
    let mut app = app(false);
    drag(&mut app, [137.0, 200.0], [217.0, 260.0]);
    assert_eq!(frames(&app), vec![[137, 200, 217, 260]]);
    app.ui.extras.snap = true;
    // Outside the frame (x 130), 7 px from its left edge: the corner snaps to x 137.
    drag(&mut app, [130.0, 220.0], [150.0, 240.0]);
    let all = frames(&app);
    assert_eq!(all.len(), 2, "a second frame, not an edit of the first: {all:?}");
    assert!(all.contains(&[137, 200, 217, 260]), "the first frame is where it was: {all:?}");
    assert!(all.contains(&[137, 220, 150, 240]), "the new frame starts at the snapped corner: {all:?}");
}

/// CodeRabbit on #15141: a moved frame snaps by its edges. Its centre line near another layer's edge is no
/// alignment: a 40 px frame centred on x 137 (the layer's right edge), nudged 3 px, has no edge near one.
#[test]
fn a_moved_frame_does_not_snap_its_centre_to_an_edge() {
    let mut app = app(false);
    drag(&mut app, [117.0, 100.0], [157.0, 130.0]);
    assert_eq!(frames(&app), vec![[117, 100, 157, 130]]);
    app.ui.extras.snap = true;
    drag(&mut app, [135.0, 115.0], [138.0, 115.0]);
    assert_eq!(frames(&app), vec![[120, 100, 160, 130]], "moved by the pointer, +3 px: its centre did not jump to 137");
}

/// CodeRabbit on #15141: a click on an empty frame does not move it, even with a layer edge in reach of the
/// frame's own edge.
#[test]
fn a_click_on_an_empty_frame_does_not_move_it() {
    let mut app = app(false);
    drag(&mut app, [140.0, 100.0], [190.0, 130.0]);
    assert_eq!(frames(&app), vec![[140, 100, 190, 130]]);
    app.ui.extras.snap = true;
    let before = app.session.active().unwrap().history.entries();
    click(&mut app, [150.0, 110.0]);
    assert_eq!(frames(&app), vec![[140, 100, 190, 130]], "still where it was, 3 px from the layer's edge at 137");
    assert_eq!(app.session.active().unwrap().history.entries(), before, "no Move step");
}

/// CodeRabbit on #15141: Ctrl held while drawing turns snapping off, from the press on. A press 7 px
/// from the layer's edge at x 137 keeps its x with Ctrl held through Down, Move and Up, and takes the
/// edge without Ctrl.
#[test]
fn a_ctrl_held_press_does_not_snap_the_start_corner() {
    let ctrl = egui::Modifiers { ctrl: true, ..Default::default() };
    let draw = |m: egui::Modifiers| {
        let mut app = app(true);
        tool_event(&mut app, ToolEvent::Down { x: 130.0, y: 200.0, pressure: 1.0 }, m);
        tool_event(&mut app, ToolEvent::Move { x: 170.0, y: 230.0, pressure: 1.0 }, m);
        tool_event(&mut app, ToolEvent::Move { x: 200.0, y: 250.0, pressure: 1.0 }, m);
        tool_event(&mut app, ToolEvent::Up { x: 200.0, y: 250.0 }, m);
        frames(&app)
    };
    assert_eq!(draw(egui::Modifiers::NONE), vec![[137, 200, 200, 250]], "without Ctrl the start takes the edge");
    assert_eq!(draw(ctrl), vec![[130, 200, 200, 250]], "with Ctrl the press stays where it was");
}

fn shift_drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) -> Vec<(bool, f64)> {
    let m = egui::Modifiers { shift: true, ..Default::default() };
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, m);
    let mut lines: Vec<(bool, f64)> = app.prefs_rt.snap_lines.iter().map(|l| (l.vertical, l.pos)).collect();
    lines.sort_by(|a, b| a.partial_cmp(b).unwrap());
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, m);
    lines
}

/// CodeRabbit on #15160: with Shift the frame is squared from the drag's shorter side, so the corner that
/// snaps is the squared one. Drawing from (200, 150) to (134, 100) squares to (150, 100): 13 px from the
/// layer's edge at x 137, so nothing snaps and no alignment is shown (the pointer's x of 134 used to
/// show a line the frame did not reach).
#[test]
fn a_shift_drawn_frame_snaps_its_squared_corner_not_the_pointer() {
    let mut app = app(true);
    let lines = shift_drag(&mut app, [200.0, 150.0], [134.0, 100.0]);
    assert_eq!(frames(&app), vec![[150, 100, 200, 150]]);
    assert!(lines.is_empty(), "no alignment for an edge the frame does not reach: {lines:?}");
}

/// The squared corner does snap when it comes near an edge: dragging to (141, 60) squares to (141, 91),
/// 1 px from the layer's bottom edge at y 90 and 4 from its right edge at x 137. The nearer snap decides, the
/// frame stays square, and only the alignment it reaches (y 90) is shown.
#[test]
fn a_shift_drawn_frame_keeps_its_square_when_the_corner_snaps() {
    let mut app = app(true);
    let lines = shift_drag(&mut app, [200.0, 150.0], [141.0, 60.0]);
    assert_eq!(frames(&app), vec![[140, 90, 200, 150]]);
    assert_eq!(lines, vec![(false, 90.0)], "only the y alignment");
}

/// CodeRabbit on #15160: Shift moves a frame along the axis the raw drag is longer on. A frame at
/// (140, 100)-(190, 130), pressed at (150, 110) and dragged to (151, 112), goes down by 2: the left edge's
/// snap onto x 137 must not turn the move into a horizontal one.
#[test]
fn a_shift_moved_frame_keeps_the_axis_of_the_raw_drag() {
    let mut app = app(false);
    drag(&mut app, [140.0, 100.0], [190.0, 130.0]);
    assert_eq!(frames(&app), vec![[140, 100, 190, 130]]);
    app.ui.extras.snap = true;
    let lines = shift_drag(&mut app, [150.0, 110.0], [151.0, 112.0]);
    assert_eq!(frames(&app), vec![[140, 102, 190, 132]], "moved down by 2 only");
    assert!(lines.is_empty(), "no alignment on the axis the frame did not move on: {lines:?}");
}
