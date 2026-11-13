//! #12709: ⌥ with Add Anchor Point works on a whole subpath, as in the reference application. An
//! ⌥-click on a segment selects the subpath and records nothing; an ⌥-drag on a segment copies the
//! subpath and moves the copy (one step, Drag Path). ⌥ never turns the tool into Delete Anchor Point.

use egui::Modifiers;
use photocraft_doc::vector::Path;
use photocraft_geom::Point;
use serde_json::json;

use super::add_anchor_lock_tests::click_with_modifiers;
use super::delete_anchor_targeted_path_tests::{select_layer, setup, shape_path, steps};
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::direct_select::PathRef;
use crate::state::Tool;

/// The epic's test document: the saved path "Path 1", targeted in the Paths panel, with the closed
/// square A (100,300)-(300,500), the open line B (400,300)→(700,300) and the open S-curve C
/// (400,450)→(700,450).
fn path_one() -> PhotocraftApp {
    let (mut app, _) = setup(Tool::AddAnchorPoint);
    let path = json!({"subpaths": [
        {"closed": true, "knots": [[100, 300], [300, 300], [300, 500], [100, 500]]},
        {"closed": false, "knots": [[400, 300], [700, 300]]},
        {"closed": false, "knots": [{"anchor": [400, 450], "in": [400, 450], "out": [500, 350]}, {"anchor": [700, 450], "in": [600, 550], "out": [700, 450]}]},
    ]});
    app.run("path.set", json!({"name": "Path 1", "path": path})).unwrap();
    app.ui.selected_path = Some("Path 1".into());
    app
}

fn path(app: &PhotocraftApp) -> Path {
    app.session.active().unwrap().doc.paths[0].path.clone()
}

fn undo_label(app: &PhotocraftApp) -> Option<&str> {
    app.session.active().unwrap().history.undo_label()
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: (from[0] + to[0]) / 2.0, y: (from[1] + to[1]) / 2.0, pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
}

#[test]
fn alt_click_on_a_segment_selects_the_whole_subpath_and_adds_nothing() {
    let mut app = path_one();
    let (before, n) = (path(&app), steps(&app));
    click_with_modifiers(&mut app, [200.0, 300.0], Modifiers::ALT);
    app.sync_views();
    assert_eq!(path(&app), before, "no anchor added");
    assert_eq!(steps(&app), n, "no History step");
    let ds = &app.ui.direct_selection;
    assert_eq!(ds.target, Some(PathRef::Saved(0)));
    assert_eq!(ds.anchors, vec![[0, 0], [0, 1], [0, 2], [0, 3]], "A's four anchors");
}

#[test]
fn alt_drag_on_a_segment_copies_the_subpath_and_moves_the_copy_in_one_drag_path_step() {
    let mut app = path_one();
    let (before, n) = (path(&app), steps(&app));
    // C's middle: the S-curve passes (550,450) at t = 0.5.
    drag(&mut app, [550.0, 450.0], [550.0, 500.0], Modifiers::ALT);
    app.sync_views();
    let p = path(&app);
    assert_eq!(p.subpaths.len(), 4, "a fourth subpath");
    assert_eq!(p.subpaths[..3], before.subpaths[..], "A, B and C stay where they were");
    let copy = &p.subpaths[3];
    assert!(!copy.closed);
    assert_eq!(copy.knots[0].anchor, Point::new(400.0, 500.0));
    assert_eq!(copy.knots[1].anchor, Point::new(700.0, 500.0));
    assert_eq!((copy.knots[0].out_ctrl, copy.knots[1].in_ctrl), (Point::new(500.0, 400.0), Point::new(600.0, 600.0)), "handles move with the copy");
    assert_eq!(steps(&app), n + 1);
    assert_eq!(undo_label(&app), Some("Drag Path"));
    assert_eq!(app.ui.direct_selection.anchors, vec![[3, 0], [3, 1]], "the copy is selected");
    assert!(app.session.undo());
    assert_eq!(path(&app), before);
}

#[test]
fn alt_click_on_an_anchor_deletes_nothing() {
    let mut app = path_one();
    let (before, n) = (path(&app), steps(&app));
    click_with_modifiers(&mut app, [100.0, 300.0], Modifiers::ALT);
    assert_eq!(path(&app), before);
    assert_eq!(steps(&app), n);
    assert_ne!(undo_label(&app), Some("Delete Anchor Point"));
}

#[test]
fn alt_drag_on_an_anchor_or_empty_canvas_edits_nothing() {
    // Not measured in the reference: such a drag neither deletes nor copies.
    let mut app = path_one();
    let (before, n) = (path(&app), steps(&app));
    drag(&mut app, [100.0, 300.0], [100.0, 350.0], Modifiers::ALT);
    drag(&mut app, [200.0, 400.0], [200.0, 450.0], Modifiers::ALT);
    assert_eq!((path(&app), steps(&app)), (before, n));
}

#[test]
fn delete_anchor_point_with_alt_still_adds_an_anchor() {
    let mut app = path_one();
    app.ui.tool = Tool::DeleteAnchorPoint;
    let n = steps(&app);
    click_with_modifiers(&mut app, [200.0, 300.0], Modifiers::ALT);
    assert_eq!(path(&app).subpaths[0].knots.len(), 5);
    assert_eq!(steps(&app), n + 1);
    assert_eq!(undo_label(&app), Some("Add Anchor Point"));
}

/// The anchors of subpath `s` of a path in the compact JSON form (`[x, y]` corners or `{"anchor"}`).
fn json_anchors(v: &serde_json::Value, s: usize) -> Vec<[f64; 2]> {
    let knots = v["subpaths"][s]["knots"].as_array().cloned().unwrap_or_default();
    knots.iter().map(|k| if k.is_array() { k } else { &k["anchor"] }).map(|a| [a[0].as_f64().unwrap(), a[1].as_f64().unwrap()]).collect()
}

#[test]
fn alt_drag_copies_a_subpath_of_the_active_shape_layer() {
    let (mut app, shape) = setup(Tool::AddAnchorPoint);
    select_layer(&mut app, shape);
    let (before, n) = (shape_path(&mut app, shape), steps(&app));
    drag(&mut app, [550.0, 100.0], [550.0, 60.0], Modifiers::ALT);
    let after = shape_path(&mut app, shape);
    assert_eq!(after["subpaths"].as_array().map(Vec::len), Some(2), "{after}");
    assert_eq!(json_anchors(&after, 0), json_anchors(&before, 0), "the original stays");
    let moved: Vec<[f64; 2]> = json_anchors(&before, 0).into_iter().map(|[x, y]| [x, y - 40.0]).collect();
    assert_eq!(json_anchors(&after, 1), moved);
    assert_eq!(steps(&app), n + 1);
    assert_eq!(undo_label(&app), Some("Drag Path"));
}

#[test]
fn alt_shows_no_anchor_to_add_and_the_plus_badge_everywhere() {
    let mut app = path_one();
    for at in [[200.0, 300.0], [200.0, 400.0]] {
        app.hover_doc = Some(at);
        assert_eq!(crate::canvas::tool_feedback::badge(&app, Tool::AddAnchorPoint, Modifiers::ALT), Some(crate::canvas::tool_feedback::Badge::Add), "{at:?}");
    }
    assert_eq!(crate::anchor_tools::effective(Tool::AddAnchorPoint, Modifiers::ALT), Some(Tool::AddAnchorPoint));
    assert_eq!(crate::anchor_tools::effective(Tool::DeleteAnchorPoint, Modifiers::ALT), Some(Tool::AddAnchorPoint));
}

#[test]
fn letting_go_of_alt_before_the_drag_starts_still_duplicates_the_subpath() {
    // Through the real canvas: the tool sees the press only once egui recognises the drag, so the
    // keys held at the press decide, as for the selection tools (CodeRabbit on #13977).
    use egui::{Pos2, Rect, vec2};
    let ctx = egui::Context::default();
    let mut app = path_one();
    app.run("view.actualPixels", json!({})).unwrap();
    app.sync_views();
    let mut view = app.ui.views[0].clone();
    view.zoom = 1.0;
    view.center = [400.0, 300.0];
    view.fit_pending = false;
    view.fill_pending = false;
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
    let frame = |app: &mut PhotocraftApp, events: Vec<egui::Event>| {
        let input = egui::RawInput { screen_rect: Some(rect), events, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            crate::canvas::canvas_view(app, ui, 0, rect, view.clone(), true);
        });
        out.textures_delta.clear();
    };
    let press = |pos: Pos2, pressed: bool, modifiers: Modifiers| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
    let (alt, none) = (Modifiers::ALT, Modifiers::NONE);
    let (before, n) = (path(&app), steps(&app));
    // C's middle, (550,450) in the document, is the same screen point at 100 % centred on the document.
    let start = Pos2::new(550.0, 450.0);
    frame(&mut app, vec![egui::Event::ModifiersChanged(alt), egui::Event::PointerMoved(start)]);
    frame(&mut app, vec![press(start, true, alt)]);
    frame(&mut app, vec![egui::Event::ModifiersChanged(none), egui::Event::PointerMoved(start + vec2(0.0, 25.0))]);
    frame(&mut app, vec![egui::Event::PointerMoved(start + vec2(0.0, 50.0))]);
    frame(&mut app, vec![press(start + vec2(0.0, 50.0), false, none)]);
    let p = path(&app);
    assert_eq!(p.subpaths.len(), 4, "the subpath was copied, no anchor added: {:?}", p.subpaths.iter().map(|s| s.knots.len()).collect::<Vec<_>>());
    assert_eq!(p.subpaths[..3], before.subpaths[..]);
    assert_eq!((p.subpaths[3].knots[0].anchor, p.subpaths[3].knots[1].anchor), (Point::new(400.0, 500.0), Point::new(700.0, 500.0)));
    assert_eq!((steps(&app), undo_label(&app)), (n + 1, Some("Drag Path")));
}

#[test]
fn letting_go_of_alt_before_the_click_ends_still_selects_the_subpath() {
    // Through the real canvas: a click is replayed on release, so the keys held at the press
    // decide, as for Object Selection (CodeRabbit on #13977).
    use egui::{Pos2, Rect, vec2};
    let ctx = egui::Context::default();
    let mut app = path_one();
    app.run("view.actualPixels", json!({})).unwrap();
    app.sync_views();
    let mut view = app.ui.views[0].clone();
    view.zoom = 1.0;
    view.center = [400.0, 300.0];
    view.fit_pending = false;
    view.fill_pending = false;
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
    let frame = |app: &mut PhotocraftApp, events: Vec<egui::Event>| {
        let input = egui::RawInput { screen_rect: Some(rect), events, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            crate::canvas::canvas_view(app, ui, 0, rect, view.clone(), true);
        });
        out.textures_delta.clear();
    };
    let press = |pos: Pos2, pressed: bool, modifiers: Modifiers| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
    let (alt, none) = (Modifiers::ALT, Modifiers::NONE);
    let (before, n) = (path(&app), steps(&app));
    // A's top edge, (200,300) in the document.
    let at = Pos2::new(200.0, 300.0);
    frame(&mut app, vec![egui::Event::ModifiersChanged(alt), egui::Event::PointerMoved(at)]);
    frame(&mut app, vec![press(at, true, alt)]);
    frame(&mut app, vec![egui::Event::ModifiersChanged(none)]);
    frame(&mut app, vec![press(at, false, none)]);
    frame(&mut app, vec![]);
    assert_eq!((path(&app), steps(&app)), (before, n), "no anchor added, no History step");
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 0], [0, 1], [0, 2], [0, 3]], "A's four anchors");
}
