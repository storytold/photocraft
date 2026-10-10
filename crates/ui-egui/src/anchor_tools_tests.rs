//! #1044: the Add Anchor Point and Delete Anchor Point tools insert and remove anchors on the
//! paths Direct Selection edits, one history step per click.

use egui::Modifiers;
use photocraft_doc::vector::Path;
use photocraft_geom::Point;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::direct_select::PathRef;
use crate::state::Tool;

fn app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app.ui.tool = tool;
    app
}

/// A square work path; knot 1 at (120, 40) is smooth, so the top edge (0 → 1) is a curve.
fn with_path(app: &mut PhotocraftApp) {
    let path = json!({"subpaths": [{"closed": true, "knots": [
        [40, 40],
        {"anchor": [120, 40], "in": [100, 20], "out": [140, 60], "smooth": true},
        [120, 120],
        [40, 120],
    ]}]});
    app.run("path.set", json!({"name": "work", "path": path})).unwrap();
}

fn work(app: &PhotocraftApp) -> Path {
    app.session.active().unwrap().doc.work_path.clone().unwrap()
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

fn click(app: &mut PhotocraftApp, at: [f64; 2], mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x: at[0], y: at[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x: at[0], y: at[1] }, mods);
}

#[test]
fn both_tools_join_the_pen_group() {
    let group: Vec<Tool> = Tool::ALL.iter().copied().filter(|t| t.key() == 'P').collect();
    assert_eq!(group, vec![Tool::Pen, Tool::AddAnchorPoint, Tool::DeleteAnchorPoint], "P cycles Pen → Add → Delete");
    let mut app = app(Tool::Brush);
    app.ui.tool = Tool::from_name("addAnchorPoint").unwrap();
    assert_eq!(app.ui.tool, Tool::AddAnchorPoint);
}

#[test]
fn add_on_a_straight_segment_inserts_a_corner_in_one_step() {
    let mut app = app(Tool::AddAnchorPoint);
    with_path(&mut app);
    let (before, n) = (work(&app), steps(&app));
    click(&mut app, [80.0, 121.0], Modifiers::NONE);
    let p = work(&app);
    assert_eq!(p.subpaths[0].knots.len(), 5);
    let k = p.subpaths[0].knots[3];
    assert!(!k.smooth && (k.anchor.y - 120.0).abs() < 1e-9 && (k.anchor.x - 80.0).abs() < 1.0, "a corner on the bottom edge: {k:?}");
    assert_eq!(steps(&app), n + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Add Anchor Point"));
    assert!(app.session.undo());
    assert_eq!(work(&app), before);
}

#[test]
fn add_on_a_curve_splits_it_where_clicked() {
    let mut app = app(Tool::AddAnchorPoint);
    with_path(&mut app);
    let seg = work(&app).subpaths[0].segments()[0];
    let on = photocraft_vector::edit::eval(&seg, 0.5);
    click(&mut app, [on.x, on.y], Modifiers::NONE);
    let p = work(&app);
    let k = p.subpaths[0].knots[1];
    assert!(k.smooth && (k.anchor.x - on.x).hypot(k.anchor.y - on.y) < 0.2, "a smooth anchor where clicked: {k:?}");
    // The two halves still pass through the old curve's quarter points.
    let halves = &p.subpaths[0].segments()[..2];
    for t in [0.25, 0.75] {
        let old = photocraft_vector::edit::eval(&seg, t);
        let near = halves
            .iter()
            .flat_map(|s| (0..=400).map(move |i| photocraft_vector::edit::eval(s, f64::from(i) / 400.0)))
            .fold(f64::MAX, |d, q| d.min((q.x - old.x).hypot(q.y - old.y)));
        assert!(near < 0.1, "t={t}: {near}");
    }
}

#[test]
fn delete_removes_the_clicked_anchor_and_alt_swaps_the_tools() {
    let mut app = app(Tool::DeleteAnchorPoint);
    with_path(&mut app);
    let n = steps(&app);
    click(&mut app, [120.0, 120.0], Modifiers::NONE);
    let p = work(&app);
    assert_eq!((p.subpaths[0].knots.len(), p.subpaths[0].closed), (3, true));
    assert_eq!(p.subpaths[0].knots[2].anchor, Point::new(40.0, 120.0));
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Delete Anchor Point"));
    // ⌥ with Delete adds; ⌥ with Add deletes.
    click(&mut app, [80.0, 120.0], Modifiers::ALT);
    assert_eq!(work(&app).subpaths[0].knots.len(), 3, "the bottom edge is gone; ⌥ on empty canvas adds nothing");
    click(&mut app, [40.0, 80.0], Modifiers::ALT);
    assert_eq!(work(&app).subpaths[0].knots.len(), 4);
    app.ui.tool = Tool::AddAnchorPoint;
    click(&mut app, [40.0, 80.0], Modifiers::ALT);
    assert_eq!(work(&app).subpaths[0].knots.len(), 3);
    assert_eq!(steps(&app), n + 3);
}

#[test]
fn clicks_on_nothing_editable_change_nothing() {
    let mut app = app(Tool::AddAnchorPoint);
    with_path(&mut app);
    let (before, n) = (work(&app), steps(&app));
    click(&mut app, [80.0, 80.0], Modifiers::NONE);
    click(&mut app, [40.0, 40.0], Modifiers::NONE);
    app.ui.tool = Tool::DeleteAnchorPoint;
    click(&mut app, [80.0, 80.0], Modifiers::NONE);
    click(&mut app, [40.0, 80.0], Modifiers::NONE);
    assert_eq!((work(&app), steps(&app)), (before, n), "empty canvas, an anchor with Add, a segment with Delete");
}

#[test]
fn shape_layer_paths_are_edited_and_a_direct_selection_is_cleared() {
    let mut app = app(Tool::DirectSelection);
    let id = app.run("shape.create", json!({"kind": "rect", "rect": [20, 20, 60, 60], "fill": "#3070c0"})).unwrap()["layer"].as_u64().unwrap();
    click(&mut app, [80.0, 80.0], Modifiers::NONE);
    assert_eq!((app.ui.direct_selection.target, app.ui.direct_selection.anchors.clone()), (Some(PathRef::Layer(id)), vec![[0, 2]]));
    app.ui.tool = Tool::AddAnchorPoint;
    click(&mut app, [50.0, 20.0], Modifiers::NONE);
    let info = app.run("shape.info", json!({"layer": id})).unwrap();
    assert_eq!(info["path"]["subpaths"][0]["knots"].as_array().unwrap().len(), 5);
    assert!(app.ui.direct_selection.anchors.is_empty(), "the selection no longer points at a shifted anchor");
    app.ui.tool = Tool::DeleteAnchorPoint;
    click(&mut app, [20.0, 80.0], Modifiers::NONE);
    let info = app.run("shape.info", json!({"layer": id})).unwrap();
    assert_eq!(info["path"]["subpaths"][0]["knots"].as_array().unwrap().len(), 4);
}

#[test]
fn the_cursor_shows_plus_or_minus() {
    use crate::tool_feedback::{Badge, badge};
    let app = app(Tool::AddAnchorPoint);
    assert_eq!(badge(&app, Tool::AddAnchorPoint, Modifiers::NONE), Some(Badge::Add));
    assert_eq!(badge(&app, Tool::AddAnchorPoint, Modifiers::ALT), Some(Badge::Subtract));
    assert_eq!(badge(&app, Tool::DeleteAnchorPoint, Modifiers::NONE), Some(Badge::Subtract));
    assert_eq!(badge(&app, Tool::DeleteAnchorPoint, Modifiers::COMMAND), None, "⌘ is Direct Selection");
}
