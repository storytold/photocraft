//! #12708: Add Anchor Point selects, moves and reshapes existing anchors and handles as Direct
//! Selection does, and a click on a segment still adds an anchor.

use egui::Modifiers;
use photocraft_doc::vector::Path;
use photocraft_geom::Point;
use photocraft_vector::edit::{Handle, Hit};
use serde_json::json;

use super::delete_anchor_targeted_path_tests::{setup, steps};
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::direct_select::PathRef;
use crate::state::Tool;

/// Saved path "Path 1", targeted in the Paths panel: the closed corner square of the issue and an
/// open curve through the measured smooth anchor (549.5,340), in (474.7,340), out (624.7,340).
fn target_path_1(app: &mut PhotocraftApp) {
    let path = json!({"subpaths": [
        {"closed": true, "knots": [[100, 300], [300, 300], [300, 500], [100, 500]]},
        {"closed": false, "knots": [
            [450, 450],
            {"anchor": [549.5, 340], "in": [474.7, 340], "out": [624.7, 340], "smooth": true},
            [650, 450]]},
    ]});
    app.run("path.set", json!({"name": "Path 1", "path": path})).unwrap();
    app.ui.selected_path = Some("Path 1".into());
}

fn tool_app(tool: Tool) -> PhotocraftApp {
    let (mut app, _) = setup(tool);
    target_path_1(&mut app);
    app
}

fn path(app: &PhotocraftApp) -> Path {
    app.session.active().unwrap().doc.paths[0].path.clone()
}

fn knot(app: &PhotocraftApp, s: usize, k: usize) -> photocraft_doc::Knot {
    path(app).subpaths[s].knots[k]
}

fn label(app: &PhotocraftApp) -> Option<String> {
    app.session.active().unwrap().history.undo_label().map(str::to_string)
}

fn click(app: &mut PhotocraftApp, at: [f64; 2], mods: Modifiers) {
    drag(app, at, at, mods);
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
}

/// The History step Direct Selection records for the same gesture: Add Anchor Point names its
/// anchor and handle drags as Direct Selection does.
fn direct_selection_label(gesture: impl Fn(&mut PhotocraftApp)) -> Option<String> {
    let mut app = tool_app(Tool::DirectSelection);
    gesture(&mut app);
    label(&app)
}

fn close(got: Point, want: [f64; 2], what: &str) {
    assert!((got.x - want[0]).abs() <= 0.5 && (got.y - want[1]).abs() <= 0.5, "{what}: got ({}, {}), want {want:?}", got.x, got.y);
}

#[test]
fn after_a_click_on_a_segment_the_new_anchor_is_the_only_selected_one_and_its_handles_show() {
    let mut app = tool_app(Tool::AddAnchorPoint);
    click(&mut app, [100.0, 300.0], Modifiers::NONE);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 0]]);
    let n = steps(&app);
    // The curve's first segment, (450,450) to the smooth anchor.
    let on_curve = photocraft_vector::edit::eval(&path(&app).subpaths[1].segments()[0], 0.5);
    click(&mut app, [on_curve.x, on_curve.y], Modifiers::NONE);
    assert_eq!(steps(&app), n + 1);
    assert_eq!(label(&app).as_deref(), Some("Add Anchor Point"));
    assert_eq!(app.ui.direct_selection.target, Some(PathRef::Saved(0)));
    assert_eq!(app.ui.direct_selection.anchors, vec![[1, 1]], "the new anchor is the only selected one");
    let added = knot(&app, 1, 1);
    let out = [added.out_ctrl.x, added.out_ctrl.y];
    assert_ne!(added.out_ctrl, added.anchor, "a curve split gives the anchor handles");
    assert_eq!(crate::direct_select::hit_active_handles(&app, out).map(|h| h.2), Some(Hit::Handle([1, 1], Handle::Out)), "its handles are shown");
}

#[test]
fn a_click_on_an_anchor_selects_it_shift_adds_and_empty_canvas_deselects_without_steps() {
    let mut app = tool_app(Tool::AddAnchorPoint);
    let (before, n) = (path(&app), steps(&app));
    click(&mut app, [300.0, 300.0], Modifiers::NONE);
    assert_eq!(app.ui.direct_selection.target, Some(PathRef::Saved(0)));
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 1]]);
    click(&mut app, [300.0, 500.0], Modifiers::SHIFT);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 1], [0, 2]]);
    click(&mut app, [100.0, 300.0], Modifiers::NONE);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 0]], "a plain click selects that anchor alone");
    click(&mut app, [50.0, 50.0], Modifiers::NONE);
    assert!(app.ui.direct_selection.anchors.is_empty());
    assert_eq!(app.ui.selected_path.as_deref(), Some("Path 1"), "the path stays targeted");
    assert_eq!((path(&app), steps(&app)), (before, n), "selecting changes nothing and records nothing");
}

#[test]
fn dragging_an_anchor_moves_it_with_its_handles_in_one_step() {
    let gesture = |app: &mut PhotocraftApp| drag(app, [300.0, 300.0], [330.0, 280.0], Modifiers::NONE);
    let mut app = tool_app(Tool::AddAnchorPoint);
    let n = steps(&app);
    gesture(&mut app);
    let k = knot(&app, 0, 1);
    assert_eq!((k.anchor, k.in_ctrl, k.out_ctrl), (Point::new(330.0, 280.0), Point::new(330.0, 280.0), Point::new(330.0, 280.0)));
    assert_eq!(path(&app).subpaths[0].knots.len(), 4, "no anchor added");
    assert_eq!(steps(&app), n + 1);
    let name = label(&app);
    assert!(matches!(name.as_deref(), Some("Drag Anchor Point" | "Drag Anchor Points")), "{name:?}");
    assert_eq!(name, direct_selection_label(gesture));
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 1]], "the dragged anchor stays selected");
}

#[test]
fn dragging_a_smooth_anchors_out_handle_turns_the_in_handle_and_keeps_its_length() {
    let gesture = |app: &mut PhotocraftApp| {
        click(app, [549.5, 340.0], Modifiers::NONE);
        drag(app, [624.7, 340.0], [624.7, 300.0], Modifiers::NONE);
    };
    let mut app = tool_app(Tool::AddAnchorPoint);
    let n = steps(&app);
    gesture(&mut app);
    let k = knot(&app, 1, 1);
    close(k.anchor, [549.5, 340.0], "anchor");
    close(k.out_ctrl, [624.7, 300.0], "out");
    close(k.in_ctrl, [483.4, 375.2], "in");
    assert_eq!(steps(&app), n + 1, "the selecting click records nothing, the drag one step");
    let name = label(&app);
    assert!(matches!(name.as_deref(), Some("Drag Control Point" | "Drag Direction Point")), "{name:?}");
    assert_eq!(name, direct_selection_label(gesture));
}

#[test]
fn alt_dragging_a_handle_moves_only_that_handle() {
    let gesture = |app: &mut PhotocraftApp| {
        click(app, [549.5, 340.0], Modifiers::NONE);
        drag(app, [474.7, 340.0], [474.7, 300.0], Modifiers::ALT);
    };
    let mut app = tool_app(Tool::AddAnchorPoint);
    let n = steps(&app);
    gesture(&mut app);
    let k = knot(&app, 1, 1);
    close(k.in_ctrl, [474.7, 300.0], "in");
    close(k.out_ctrl, [624.7, 340.0], "out unchanged");
    close(k.anchor, [549.5, 340.0], "anchor");
    assert_eq!(path(&app).subpaths[1].knots.len(), 3, "⌥ on a handle neither adds nor deletes");
    assert_eq!(steps(&app), n + 1);
    let name = label(&app);
    assert!(matches!(name.as_deref(), Some("Drag Control Point" | "Drag Direction Point")), "{name:?}");
    assert_eq!(name, direct_selection_label(gesture));
}

#[test]
fn a_click_on_a_segment_still_adds_an_anchor_with_no_anchor_selected() {
    let mut app = tool_app(Tool::AddAnchorPoint);
    click(&mut app, [300.0, 300.0], Modifiers::NONE);
    click(&mut app, [50.0, 50.0], Modifiers::NONE);
    assert!(app.ui.direct_selection.anchors.is_empty());
    let n = steps(&app);
    click(&mut app, [200.0, 300.0], Modifiers::NONE);
    assert_eq!(path(&app).subpaths[0].knots.len(), 5);
    assert_eq!(steps(&app), n + 1);
    assert_eq!(label(&app).as_deref(), Some("Add Anchor Point"));
}

/// Not measured, so as today: ⌥-click on an anchor with Add Anchor Point is Delete Anchor Point.
#[test]
fn alt_click_on_an_anchor_still_deletes_it() {
    let mut app = tool_app(Tool::AddAnchorPoint);
    let n = steps(&app);
    click(&mut app, [300.0, 300.0], Modifiers::ALT);
    assert_eq!(path(&app).subpaths[0].knots.len(), 3);
    assert_eq!(steps(&app), n + 1);
    assert_eq!(label(&app).as_deref(), Some("Delete Anchor Point"));
}

/// A press on a handle that is not shown (its anchor is not selected) grabs nothing: the press
/// falls through to the segment test, and off every segment it only deselects.
#[test]
fn a_hidden_handle_is_not_grabbed() {
    let mut app = tool_app(Tool::AddAnchorPoint);
    let (before, n) = (path(&app), steps(&app));
    drag(&mut app, [624.7, 340.0], [624.7, 300.0], Modifiers::NONE);
    assert_eq!((path(&app), steps(&app)), (before, n));
}

/// ⌥-drag on a shown handle that lies on a straight segment moves the handle alone; the segment
/// under it is not bent (⌥ makes the press Delete Anchor Point, whose segment drag must not win).
#[test]
fn alt_drag_on_a_shown_handle_over_a_straight_segment_moves_the_handle() {
    let (mut app, _) = setup(Tool::AddAnchorPoint);
    let doc_path = json!({"subpaths": [
        {"closed": true, "knots": [[100, 300], [300, 300], [300, 500], [100, 500]]},
        {"closed": false, "knots": [
            [100, 200],
            {"anchor": [200, 250], "in": [150, 250], "out": [200, 300], "smooth": false},
            [300, 200]]},
    ]});
    app.run("path.set", json!({"name": "Path 1", "path": doc_path})).unwrap();
    app.ui.selected_path = Some("Path 1".into());
    click(&mut app, [200.0, 250.0], Modifiers::NONE);
    let (square, n) = (path(&app).subpaths[0].clone(), steps(&app));
    drag(&mut app, [200.0, 300.0], [220.0, 330.0], Modifiers::ALT);
    assert_eq!(path(&app).subpaths[0], square, "the square's top edge is not bent");
    assert_eq!(knot(&app, 1, 1).out_ctrl, Point::new(220.0, 330.0));
    assert_eq!(knot(&app, 1, 1).in_ctrl, Point::new(150.0, 250.0));
    assert_eq!(steps(&app), n + 1);
}
