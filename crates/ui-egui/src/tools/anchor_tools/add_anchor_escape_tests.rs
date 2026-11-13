//! #12728: with Add Anchor Point, Esc first clears anchors selected on the saved path targeted in
//! the Paths panel, then deselects that path, so its outline goes and a click on a segment adds
//! nothing. Esc never adds a History step.

use egui::{Key, Modifiers};
use serde_json::json;

use super::add_anchor_delete_key_tests::{app, ctrl_click, path1, steps};
use super::tests::click;
use crate::PhotocraftApp;
use crate::direct_select::{Drag, PathRef, active_paths, candidates};

fn escape(app: &mut PhotocraftApp) {
    let ctx = egui::Context::default();
    let key = egui::Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    ctx.begin_pass(egui::RawInput { events: vec![key], ..Default::default() });
    crate::shortcuts::handle(app, &ctx);
    ctx.end_pass().textures_delta.clear();
}

#[test]
fn the_first_escape_clears_the_selected_anchor_and_keeps_path_1() {
    let mut app = app();
    // ⌘ is Direct Selection while held: it selects the anchor at (300, 300).
    ctrl_click(&mut app, [300.0, 300.0]);
    assert_eq!((app.ui.direct_selection.target, app.ui.direct_selection.anchors.clone()), (Some(PathRef::Saved(0)), vec![[0, 1]]));
    let n = steps(&app);
    escape(&mut app);
    assert!(app.ui.direct_selection.anchors.is_empty(), "the first Esc clears the anchor");
    assert_eq!(app.ui.selected_path.as_deref(), Some("Path 1"), "Path 1 stays selected in the Paths panel");
    assert!(!active_paths(&app).is_empty(), "its outline stays");
    assert_eq!(steps(&app), n, "Esc adds no History step");
}

#[test]
fn the_second_escape_deselects_path_1_and_a_click_on_a_segment_then_adds_nothing() {
    let mut app = app();
    ctrl_click(&mut app, [300.0, 300.0]);
    let n = steps(&app);
    escape(&mut app);
    escape(&mut app);
    assert_eq!(app.ui.selected_path, None, "the second Esc deselects Path 1 in the Paths panel");
    assert!(candidates(&app).is_empty(), "its outline is hidden");
    assert_eq!(steps(&app), n, "Esc adds no History step");
    let before = path1(&app);
    // The square's top edge runs from (100, 300) to (300, 300).
    click(&mut app, [200.0, 300.0], Modifiers::NONE);
    assert_eq!(path1(&app), before, "the click adds no anchor");
    assert_eq!(steps(&app), n, "and no History step");
}

#[test]
fn an_anchor_just_added_is_cleared_first_then_the_path_is_deselected() {
    let mut app = app();
    let n = steps(&app);
    click(&mut app, [200.0, 300.0], Modifiers::NONE);
    assert_eq!(path1(&app).subpaths[0].knots.len(), 5, "the click on the top edge adds an anchor");
    assert_eq!(steps(&app), n + 1);
    escape(&mut app);
    assert!(app.ui.direct_selection.anchors.is_empty());
    assert_eq!(app.ui.selected_path.as_deref(), Some("Path 1"), "the first Esc keeps Path 1 targeted");
    escape(&mut app);
    assert_eq!(app.ui.selected_path, None, "the second Esc deselects it");
    assert_eq!(steps(&app), n + 1, "neither Esc adds a History step");
}

#[test]
fn an_escape_during_a_direct_selection_drag_keeps_the_anchors_and_the_path() {
    let mut app = app();
    app.run("select.all", json!({})).unwrap();
    ctrl_click(&mut app, [300.0, 300.0]);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 1]]);
    app.ui.direct_selection.drag = Some(Drag::Marquee { start: [200.0, 200.0], end: [400.0, 400.0], add: false });
    escape(&mut app);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 1]], "the drag's release still moves them");
    assert!(app.ui.direct_selection.drag.is_some(), "Esc does not end the drag");
    assert!(app.session.active().unwrap().doc.selection.is_some(), "nor deselects the pixels");
    assert_eq!(app.ui.selected_path.as_deref(), Some("Path 1"), "nor deactivates the path");
}

#[test]
fn escape_still_leaves_full_screen_and_deselects_pixels_first() {
    let mut app = app();
    app.ui.view.screen_mode = "fullScreen".into();
    escape(&mut app);
    assert_eq!(app.ui.view.screen_mode, "standard");
    assert_eq!(app.ui.selected_path.as_deref(), Some("Path 1"), "leaving full screen keeps Path 1 selected");
    app.run("select.all", json!({})).unwrap();
    escape(&mut app);
    assert!(app.session.active().unwrap().doc.selection.is_none(), "Esc deselects the pixels");
    assert_eq!(app.ui.selected_path.as_deref(), Some("Path 1"), "and keeps Path 1 selected");
}

#[test]
fn with_no_path_targeted_escape_still_leaves_full_screen_and_deselects_pixels() {
    let mut app = app();
    app.ui.selected_path = None;
    app.ui.view.screen_mode = "fullScreen".into();
    escape(&mut app);
    assert_eq!(app.ui.view.screen_mode, "standard");
    app.run("select.all", json!({})).unwrap();
    escape(&mut app);
    assert!(app.session.active().unwrap().doc.selection.is_none());
}
