//! #12766: with Delete Anchor Point, a drag from an anchor deletes it and bends the segment that now
//! joins its neighbours so it follows the pointer: one History step, Delete Anchor Point.

use egui::Modifiers;
use photocraft_doc::Knot;
use photocraft_geom::Point;
use serde_json::json;

use super::delete_anchor_escape_tests::{app, path_1};
use super::tests::{click, drag, steps};
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::direct_select::Drag;

fn knots(app: &PhotocraftApp) -> Vec<Knot> {
    path_1(app).subpaths[0].knots.clone()
}

fn near(p: Point, x: f64, y: f64) -> bool {
    (p.x - x).abs() < 1e-3 && (p.y - y).abs() < 1e-3
}

#[test]
fn a_drag_from_the_corner_bends_the_join_through_the_pointer_in_one_step() {
    let mut app = app();
    let n = steps(&app);
    drag(&mut app, [300.0, 300.0], [330.0, 280.0]);
    let k = knots(&app);
    assert_eq!(k.len(), 3);
    assert!(path_1(&app).subpaths[0].closed);
    let (p0, p3, last) = (k[0], k[1], k[2]);
    assert!(near(p0.out_ctrl, 273.3333, 140.0), "{p0:?}");
    assert!(near(p0.in_ctrl, 100.0, 300.0), "P0 in stays retracted: {p0:?}");
    assert!(near(p3.in_ctrl, 473.3333, 340.0), "{p3:?}");
    assert!(near(p3.out_ctrl, 300.0, 500.0), "P3 out stays retracted: {p3:?}");
    assert!(p0.smooth && p3.smooth);
    assert_eq!(last, Knot::corner(100.0, 500.0), "the fourth anchor is untouched");
    let seg = [p0.anchor, p0.out_ctrl, p3.in_ctrl, p3.anchor];
    let mid = photocraft_vector::edit::eval(&seg, 0.5);
    assert!((mid.x - 330.0).abs() < 1.0 && (mid.y - 280.0).abs() < 1.0, "the segment passes through the pointer: {mid:?}");
    assert_eq!(steps(&app), n + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Delete Anchor Point"));
}

#[test]
fn a_click_without_movement_keeps_the_straight_join_in_one_step() {
    let mut app = app();
    let n = steps(&app);
    click(&mut app, [300.0, 300.0], Modifiers::NONE);
    let k = knots(&app);
    assert_eq!(k.len(), 3);
    assert_eq!((k[0], k[1]), (Knot::corner(100.0, 300.0), Knot::corner(300.0, 500.0)));
    assert_eq!(steps(&app), n + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Delete Anchor Point"));
}

#[test]
fn one_undo_restores_the_square() {
    let mut app = app();
    let before = path_1(&app);
    drag(&mut app, [300.0, 300.0], [330.0, 280.0]);
    assert!(app.session.undo());
    assert_eq!(path_1(&app), before);
}

#[test]
fn the_segment_follows_the_pointer_before_the_release() {
    let mut app = app();
    let n = steps(&app);
    tool_event(&mut app, ToolEvent::Down { x: 300.0, y: 300.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 330.0, y: 280.0, pressure: 1.0 }, Modifiers::NONE);
    let Some(Drag::Edit { preview: Some(p), .. }) = &app.ui.direct_selection.drag else { panic!("no preview mid-drag") };
    let k = &p.subpaths[0].knots;
    assert!(near(k[0].out_ctrl, 273.3333, 140.0) && near(k[1].in_ctrl, 473.3333, 340.0), "{k:?}");
    assert_eq!(steps(&app), n + 1, "only the deletion is in History until the release");
    assert_eq!(knots(&app)[0], Knot::corner(100.0, 300.0), "the document still has the straight join");
    tool_event(&mut app, ToolEvent::Up { x: 330.0, y: 280.0 }, Modifiers::NONE);
    assert_eq!(steps(&app), n + 1);
}

#[test]
fn a_drag_from_an_anchor_whose_join_is_curved_does_what_the_press_did_before() {
    let mut app = app();
    let curved = json!({"subpaths": [{"closed": true, "knots": [
        {"anchor": [100, 300], "in": [100, 300], "out": [150, 250]},
        [300, 300],
        [300, 500],
        [100, 500],
    ]}]});
    app.run("path.set", json!({"name": "Path 1", "path": curved})).unwrap();
    let n = steps(&app);
    drag(&mut app, [300.0, 300.0], [330.0, 280.0]);
    let k = knots(&app);
    assert_eq!(k.len(), 3);
    assert!(near(k[0].out_ctrl, 150.0, 250.0) && near(k[1].in_ctrl, 300.0, 500.0), "{k:?}");
    assert!(!k[0].smooth && !k[1].smooth);
    assert_eq!(steps(&app), n + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Delete Anchor Point"));
}

/// Not measured, so as today (#12766's rules): Alt + Add Anchor Point deletes the anchor but the
/// drag doesn't bend the join; only the Delete Anchor Point tool itself bends it.
#[test]
fn alt_with_add_anchor_point_deletes_without_bending_the_join() {
    let mut app = app();
    app.ui.tool = crate::state::Tool::AddAnchorPoint;
    let (n, alt) = (steps(&app), Modifiers::ALT);
    tool_event(&mut app, ToolEvent::Down { x: 300.0, y: 300.0, pressure: 1.0 }, alt);
    for at in [[315.0, 290.0], [330.0, 280.0]] {
        tool_event(&mut app, ToolEvent::Move { x: at[0], y: at[1], pressure: 1.0 }, alt);
    }
    tool_event(&mut app, ToolEvent::Up { x: 330.0, y: 280.0 }, alt);
    let k = knots(&app);
    assert_eq!(k.len(), 3);
    assert_eq!(k[0].out_ctrl, k[0].anchor, "the join stays straight: {k:?}");
    assert_eq!(steps(&app), n + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Delete Anchor Point"));
}

#[test]
fn a_bend_past_the_coordinate_limit_runs_no_command_and_keeps_the_straight_join() {
    let mut app = app();
    let long = json!({"subpaths": [{"closed": true, "knots": [[998900, 100], [999800, 100], [999900, 100], [999400, 500]]}]});
    app.run("path.set", json!({"name": "Path 1", "path": long})).unwrap();
    app.sync_views();
    let n = steps(&app);
    // The mirrored handle would land at x > 1,000,000 (the release is left of 999,775).
    drag(&mut app, [999800.0, 100.0], [999700.0, 100.0]);
    let k = knots(&app);
    assert_eq!(k.len(), 3);
    assert_eq!((k[0], k[1]), (Knot::corner(998900.0, 100.0), Knot::corner(999900.0, 100.0)), "no partial bend: {k:?}");
    assert_eq!(steps(&app), n + 1, "only the deletion");
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Delete Anchor Point"));
}
