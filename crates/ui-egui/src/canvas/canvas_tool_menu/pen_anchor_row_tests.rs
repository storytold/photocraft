//! #12915: with the Pen (Path mode) a right-click on a part of the active CLOSED path heads the
//! path menu with one entry above the separator and the usual Pen list: Add Anchor Point on a
//! segment, Delete Anchor Point on an anchor; choosing it runs the engine command, one history
//! step. On empty canvas, on an open subpath, while a path is being drawn, and in Shape or Pixels
//! mode the menu keeps today's list with no top entry; with ⌥ held the row stays, as the menu does
//! not read the click's modifiers (pinned below). The shared path document and menu assertions
//! come from `delete_anchor_menu_tests` (#12771).

use egui_kittest::kittest::{NodeT, Queryable as _};
use serde_json::json;

use super::delete_anchor_menu_tests::{assert_top_entry, saved, setup, steps};
use super::tests::{harness, pen_clicks, right_click, screen_of};
use super::{PEN_MENU, rows};
use crate::PhotocraftApp;
use crate::state::Tool;
use egui::Modifiers;

/// The Pen in the measured configuration: reset (Auto Add/Delete on) in Path mode, Path 1 targeted.
fn pen_setup() -> Box<PhotocraftApp> {
    let mut app = setup(Tool::Pen);
    app.ui.tool_options.vector_mode = craft::draw::ui::VectorMode::Path;
    app.ui.tool_options.pen_auto_add_delete = true;
    app
}

/// A harness over [`pen_setup`] with a Pen right-click at document point `at` done.
fn pen_right_clicked(at: [f32; 2]) -> egui_kittest::Harness<'static, Box<PhotocraftApp>> {
    let mut h = harness(pen_setup());
    let p = screen_of(h.state(), at);
    right_click(&mut h, p, Modifiers::NONE);
    h
}

/// The Pen's list is open with no anchor-point entry above it.
fn assert_no_top_entry(h: &egui_kittest::Harness<'static, Box<PhotocraftApp>>, tool: Tool) {
    assert!(h.state().ui.canvas_tool_menu.as_ref().is_some_and(|m| m.tool == tool && rows(m) == PEN_MENU), "{tool:?}: the path menu is open");
    assert!(h.query_by_label("Add Anchor Point").is_none() && h.query_by_label("Delete Anchor Point").is_none(), "{tool:?}: no anchor row on top");
}

/// Right-click the middle of the square's top edge (200,300): Add Anchor Point heads the menu,
/// above Create Vector Mask; choosing it adds one anchor there, one history step of that name,
/// and undo restores the whole path.
#[test]
fn pen_right_click_on_a_segment_offers_add_anchor_point_on_top_and_it_adds() {
    let mut h = pen_right_clicked([200.0, 300.0]);
    assert_top_entry(&h, Tool::Pen, "Add Anchor Point");
    let before = steps(h.state());
    let original = saved(h.state());
    h.get_by_label("Add Anchor Point").click();
    h.run_steps(2);
    let path = saved(h.state());
    assert_eq!(path.subpaths[0].knots.len(), 5);
    let added = &path.subpaths[0].knots[1].anchor;
    assert!((added.x - 200.0).abs() < 3.0 && (added.y - 300.0).abs() < 3.0, "the anchor lands where it was clicked: {added:?}");
    assert_eq!(steps(h.state()), before + 1);
    assert_eq!(h.state().session.active().unwrap().history.undo_label(), Some("Add Anchor Point"));
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(&saved(h.state()), &original, "undo restores the whole path");
}

/// Right-click the corner (300,300): Delete Anchor Point heads the menu; choosing it leaves the
/// square closed with three anchors, one history step of that name.
#[test]
fn pen_right_click_on_an_anchor_offers_delete_anchor_point_on_top_and_it_deletes() {
    let mut h = pen_right_clicked([300.0, 300.0]);
    assert_top_entry(&h, Tool::Pen, "Delete Anchor Point");
    let before = steps(h.state());
    let original = saved(h.state());
    h.get_by_label("Delete Anchor Point").click();
    h.run_steps(2);
    let path = saved(h.state());
    assert!(path.subpaths[0].closed, "the square stays closed");
    assert_eq!(path.subpaths[0].knots.len(), 3);
    assert!(path.subpaths[0].knots.iter().all(|k| (k.anchor.x, k.anchor.y) != (300.0, 300.0)), "(300,300) is gone");
    assert_eq!(steps(h.state()), before + 1);
    assert_eq!(h.state().session.active().unwrap().history.undo_label(), Some("Delete Anchor Point"));
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(&saved(h.state()), &original, "undo restores the whole path");
}

/// Right-click on empty canvas: today's list, no top row, and opening it records no step.
#[test]
fn pen_right_click_on_empty_canvas_keeps_todays_list() {
    let mut h = harness(pen_setup());
    let before = steps(h.state());
    let p = screen_of(h.state(), [600.0, 100.0]);
    right_click(&mut h, p, Modifiers::NONE);
    assert_no_top_entry(&h, Tool::Pen);
    assert_eq!(steps(h.state()), before, "opening the menu records no history step");
}

/// The click on an OPEN subpath is not measured: it keeps today's list (the guard for the
/// closed-path gate in the menu's point entry).
#[test]
fn pen_right_click_on_an_open_subpath_keeps_todays_list() {
    let h = pen_right_clicked([550.0, 300.0]);
    assert_no_top_entry(&h, Tool::Pen);
}

/// The issue names an open path's ENDPOINT as not measured: an anchor click there keeps today's
/// list too.
#[test]
fn pen_right_click_on_an_open_paths_endpoint_keeps_todays_list() {
    let h = pen_right_clicked([700.0, 300.0]);
    assert_no_top_entry(&h, Tool::Pen);
}

/// Auto Add/Delete off: the Pen's overlay then senses no part and reports no hit, and the menu
/// invents none either - today's list stays (guard test, passes before and after).
#[test]
fn pen_right_click_without_auto_add_delete_keeps_todays_list() {
    let mut app = pen_setup();
    app.ui.tool_options.pen_auto_add_delete = false;
    let mut h = harness(app);
    let p = screen_of(h.state(), [200.0, 300.0]);
    right_click(&mut h, p, Modifiers::NONE);
    assert_no_top_entry(&h, Tool::Pen);
}

/// While a path is being drawn, the measured case names the plain list: no anchor row, even on
/// the open draft's own segment.
#[test]
fn pen_while_drawing_an_open_path_offers_no_top_entry() {
    let mut app = pen_setup();
    pen_clicks(&mut app, &[(150.0, 150.0), (350.0, 150.0)]);
    assert!(app.ui.pen.is_some(), "a path is being drawn");
    let mut h = harness(app);
    let p = screen_of(h.state(), [250.0, 150.0]);
    right_click(&mut h, p, Modifiers::NONE);
    assert_no_top_entry(&h, Tool::Pen);
}

/// The Shape-mode right-click is not measured: today's list, no top row.
#[test]
fn pen_right_click_in_shape_mode_keeps_todays_list() {
    let mut app = setup(Tool::Pen);
    app.ui.tool_options.vector_mode = craft::draw::ui::VectorMode::Shape;
    let mut h = harness(app);
    let p = screen_of(h.state(), [200.0, 300.0]);
    right_click(&mut h, p, Modifiers::NONE);
    assert_no_top_entry(&h, Tool::Pen);
}

/// Pixels mode is not measured either and keeps today's list (the gate names Path, so every other
/// mode is out). The Pen's options bar shows Pixels disabled, so no click can select it and the
/// option is set directly, as the shape tool's Pixels tests do.
#[test]
fn pen_right_click_in_pixels_mode_keeps_todays_list() {
    let mut app = setup(Tool::Pen);
    app.ui.tool_options.vector_mode = craft::draw::ui::VectorMode::Pixels;
    let mut h = harness(app);
    let p = screen_of(h.state(), [200.0, 300.0]);
    right_click(&mut h, p, Modifiers::NONE);
    assert_no_top_entry(&h, Tool::Pen);
}

/// ⌥ is held for the Convert overlay (craft-libs `pen::mode::resolve`), which senses the closed
/// square's anchors and straight segments too, and the framework reports that secondary click's
/// hit whatever the modifiers are: the menu's gate reads the mode and the subpath, not the click's
/// modifiers, so the row appears. Observed, and pinned as it is today; choosing it with ⌥ down is
/// not measured.
#[test]
fn pen_right_click_with_option_held_still_gets_the_anchor_row() {
    let mut h = harness(pen_setup());
    let p = screen_of(h.state(), [200.0, 300.0]);
    right_click(&mut h, p, Modifiers::ALT);
    assert_top_entry(&h, Tool::Pen, "Add Anchor Point");
}

/// The entry is about the path as it was when the menu opened: a control-channel edit meanwhile
/// shifts the knots, so choosing the stale entry deletes nothing and adds no step. It is drawn
/// disabled for the Pen as well, not only rejected on click (#12771 pattern).
#[test]
fn a_pen_point_entry_expires_once_the_path_is_edited() {
    let mut h = pen_right_clicked([300.0, 300.0]);
    assert_top_entry(&h, Tool::Pen, "Delete Anchor Point");
    assert!(!h.get_by_label("Delete Anchor Point").accesskit_node().is_disabled(), "enabled while the path is as it was");
    h.state_mut().run("path.addAnchor", json!({"name": "Path 1", "subpath": 0, "knot": 0, "t": 0.5})).unwrap();
    let after_add = saved(h.state());
    assert_eq!(after_add.subpaths[0].knots.len(), 5, "the anchor went in");
    h.run();
    assert!(h.get_by_label("Delete Anchor Point").accesskit_node().is_disabled(), "disabled once the path is edited");
    let n = steps(h.state());
    let ctx = h.ctx.clone();
    super::choose(h.state_mut(), &ctx, "path.deleteAnchor");
    assert_eq!(&saved(h.state()), &after_add, "the stale entry deleted nothing");
    assert_eq!(steps(h.state()), n, "and added no history step");
}

/// #1056: the Path Selection tool's path menu does not cover these rows — unchanged, guard test
/// that passes before and after this change.
#[test]
fn the_path_selection_tool_still_gets_no_anchor_point_row() {
    let mut app = setup(Tool::PathSelection);
    app.ui.tool_options.vector_mode = craft::draw::ui::VectorMode::Path;
    let mut h = harness(app);
    let p = screen_of(h.state(), [200.0, 300.0]);
    right_click(&mut h, p, Modifiers::NONE);
    assert_no_top_entry(&h, Tool::PathSelection);
}

/// The Delete Anchor Point tool's menu keeps its point entry (the shared mechanism stays wired) —
/// guard test that passes before and after this change.
#[test]
fn the_delete_anchor_point_tool_keeps_its_point_entry() {
    let mut h = harness(setup(Tool::DeleteAnchorPoint));
    let p = screen_of(h.state(), [200.0, 300.0]);
    right_click(&mut h, p, Modifiers::NONE);
    assert_top_entry(&h, Tool::DeleteAnchorPoint, "Add Anchor Point");
}
