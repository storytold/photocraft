//! #1044: the Add Anchor Point and Delete Anchor Point tools insert and remove anchors on the
//! paths Direct Selection edits, one history step per click.

use egui::Modifiers;
use photocraft_doc::vector::Path;
use photocraft_geom::Point;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::direct_select::PathRef;
use crate::state::Tool;

fn app(tool: Tool) -> Box<PhotocraftApp> {
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

pub(super) fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

pub(super) fn click(app: &mut PhotocraftApp, at: [f64; 2], mods: Modifiers) {
    testing::click(app, at, mods);
}

#[test]
fn add_on_a_straight_segment_inserts_a_smooth_anchor_in_one_step() {
    let mut app = app(Tool::AddAnchorPoint);
    with_path(&mut app);
    let (before, n) = (work(&app), steps(&app));
    click(&mut app, [80.0, 121.0], Modifiers::NONE);
    let p = work(&app);
    assert_eq!(p.subpaths[0].knots.len(), 5);
    let k = p.subpaths[0].knots[3];
    assert!(k.smooth && (k.anchor.y - 120.0).abs() < 1e-9 && (k.anchor.x - 80.0).abs() < 1.0, "a smooth anchor on the bottom edge: {k:?}");
    assert!(
        (k.in_ctrl.y - 120.0).abs() < 1e-9 && (k.out_ctrl.y - 120.0).abs() < 1e-9 && k.in_ctrl.x != k.anchor.x && k.out_ctrl.x != k.anchor.x,
        "with handles along the edge: {k:?}"
    );
    assert_eq!(steps(&app), n + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Add Anchor Point"));
    assert!(app.session.undo());
    assert_eq!(work(&app), before);
}

#[test]
fn delete_removes_the_clicked_anchor_and_alt_makes_it_add() {
    let mut app = app(Tool::DeleteAnchorPoint);
    with_path(&mut app);
    let n = steps(&app);
    click(&mut app, [120.0, 120.0], Modifiers::NONE);
    let p = work(&app);
    assert_eq!((p.subpaths[0].knots.len(), p.subpaths[0].closed), (3, true));
    assert_eq!(p.subpaths[0].knots[2].anchor, Point::new(40.0, 120.0));
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Delete Anchor Point"));
    // ⌥ with Delete adds; ⌥ with Add deletes nothing (#12709).
    click(&mut app, [80.0, 120.0], Modifiers::ALT);
    assert_eq!(work(&app).subpaths[0].knots.len(), 3, "the bottom edge is gone; ⌥ on empty canvas adds nothing");
    click(&mut app, [40.0, 80.0], Modifiers::ALT);
    assert_eq!(work(&app).subpaths[0].knots.len(), 4);
    app.ui.tool = Tool::AddAnchorPoint;
    click(&mut app, [40.0, 80.0], Modifiers::ALT);
    assert_eq!(work(&app).subpaths[0].knots.len(), 4);
    assert_eq!(steps(&app), n + 2);
}

#[test]
fn shape_layer_paths_are_edited_and_the_added_anchor_replaces_the_selection() {
    let mut app = app(Tool::DirectSelection);
    let id = app.run("shape.create", json!({"kind": "rect", "rect": [20, 20, 60, 60], "fill": "#3070c0"})).unwrap()["layer"].as_u64().unwrap();
    click(&mut app, [80.0, 80.0], Modifiers::NONE);
    assert_eq!((app.ui.direct_selection.target, app.ui.direct_selection.anchors.clone()), (Some(PathRef::Layer(id)), vec![[0, 2]]));
    app.ui.tool = Tool::AddAnchorPoint;
    click(&mut app, [50.0, 20.0], Modifiers::NONE);
    let info = app.run("shape.info", json!({"layer": id})).unwrap();
    assert_eq!(info["path"]["subpaths"][0]["knots"].as_array().unwrap().len(), 5);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 1]], "the new anchor is selected, not a shifted old one");
    app.ui.tool = Tool::DeleteAnchorPoint;
    click(&mut app, [20.0, 80.0], Modifiers::NONE);
    let info = app.run("shape.info", json!({"layer": id})).unwrap();
    assert_eq!(info["path"]["subpaths"][0]["knots"].as_array().unwrap().len(), 4);
}

/// #4144: with Auto Add/Delete (on by default) and no path being drawn, a Pen click on a segment of
/// the active path adds an anchor and a click on one of its anchors deletes it, one step each, and
/// no Pen path starts.
#[test]
fn the_pen_adds_and_deletes_anchors_on_the_active_path() {
    let mut app = app(Tool::Pen);
    assert!(app.ui.tool_options.pen_auto_add_delete, "on by default, as in Photoshop");
    with_path(&mut app);
    let n = steps(&app);
    click(&mut app, [80.0, 120.0], Modifiers::NONE);
    assert_eq!(work(&app).subpaths[0].knots.len(), 5);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Add Anchor Point"));
    assert!(app.ui.pen.is_none(), "no Pen path was started");
    click(&mut app, [40.0, 120.0], Modifiers::NONE);
    let p = work(&app);
    assert_eq!(p.subpaths[0].knots.len(), 4);
    assert!(!p.subpaths[0].knots.iter().any(|k| k.anchor == Point::new(40.0, 120.0)));
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Delete Anchor Point"));
    assert_eq!(steps(&app), n + 2);
    assert!(app.ui.pen.is_none());
    // Off the path the Pen draws.
    click(&mut app, [170.0, 170.0], Modifiers::NONE);
    assert_eq!(app.ui.pen.as_ref().map(|p| p.draft.knots.len()), Some(1));
}

/// The anchor tools and Direct Selection leave every press to the path overlay: a click off the paths, or with no path
/// at all, starts no Pen path and changes nothing.
#[test]
fn a_click_off_the_paths_starts_no_pen_path() {
    for tool in [Tool::AddAnchorPoint, Tool::DeleteAnchorPoint, Tool::ConvertPoint, Tool::DirectSelection] {
        let mut app = app(tool);
        click(&mut app, [170.0, 170.0], Modifiers::NONE);
        assert!(app.ui.pen.is_none(), "{tool:?}: no path");
        with_path(&mut app);
        let (before, n) = (work(&app), steps(&app));
        click(&mut app, [170.0, 170.0], Modifiers::NONE);
        assert!(app.ui.pen.is_none(), "{tool:?}");
        assert_eq!((work(&app), steps(&app)), (before, n), "{tool:?}");
    }
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    testing::press(app, from, Modifiers::NONE);
    let mid = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
    for at in [mid, to] {
        testing::move_to(app, at, Modifiers::NONE);
    }
    testing::release(app, to, Modifiers::NONE);
}

/// A smooth knot: both handles pulled out, on one line through the anchor, on opposite sides.
fn assert_smooth(k: &photocraft_doc::Knot) {
    let (vi, vo) = ((k.in_ctrl.x - k.anchor.x, k.in_ctrl.y - k.anchor.y), (k.out_ctrl.x - k.anchor.x, k.out_ctrl.y - k.anchor.y));
    assert!(k.smooth, "{k:?}");
    assert!(vi.0.hypot(vi.1) > 1.0 && vo.0.hypot(vo.1) > 1.0, "both handles pulled out: {k:?}");
    let cross = (vi.0 * vo.1 - vi.1 * vo.0) / (vi.0.hypot(vi.1) * vo.0.hypot(vo.1));
    assert!(cross.abs() < 1e-9 && vi.0 * vo.0 + vi.1 * vo.1 < 0.0, "collinear, opposite handles: {k:?}");
}

/// The same knots within a pixel: the Pen's presses and drags snap (View › Snap), so its anchor
/// lands up to a pixel from the raw pointer.
fn assert_close(got: &Path, want: &Path, what: &str) {
    let knots = |p: &Path| -> Vec<(Point, Point, Point, bool)> {
        p.subpaths.iter().flat_map(|s| s.knots.iter().map(|k| (k.anchor, k.in_ctrl, k.out_ctrl, k.smooth))).collect()
    };
    let (g, w) = (knots(got), knots(want));
    assert_eq!(g.len(), w.len(), "{what}: {got:?}");
    for (a, b) in g.iter().zip(&w) {
        let near = |p: Point, q: Point| (p.x - q.x).hypot(p.y - q.y) <= 1.5;
        assert!(near(a.0, b.0) && near(a.1, b.1) && near(a.2, b.2) && a.3 == b.3, "{what}: {a:?} vs {b:?}");
    }
}

/// `path` with the anchor a click at `at` adds, as the vector edit makes it, and that anchor.
fn added_at(path: &Path, at: Point) -> (Path, [usize; 2]) {
    let mut p = path.clone();
    let Some(photocraft_vector::edit::Hit::Segment(k, t)) = photocraft_vector::edit::hit(&p, at, 6.0, &[]) else { panic!("no segment at {at:?}") };
    let added = photocraft_vector::edit::add_anchor(&mut p, k, t).unwrap();
    (p, added)
}

/// #8653: a drag from the click moves the anchor the Pen's Auto Add adds, handles and all; for the Add Anchor Point
/// tool it pulls the new anchor's handles out instead (#12707). The add and the drag are one history step.
#[test]
fn dragging_from_an_add_moves_the_new_anchor_in_one_step() {
    for tool in [Tool::AddAnchorPoint, Tool::Pen] {
        let mut app = app(tool);
        with_path(&mut app);
        let (before, n) = (work(&app), steps(&app));
        let on = photocraft_vector::edit::eval(&before.subpaths[0].segments()[0], 0.5);
        drag(&mut app, [on.x, on.y], [on.x + 5.0, on.y + 15.0]);
        let (mut want, k) = added_at(&before, on);
        if tool == Tool::AddAnchorPoint {
            let knot = &mut want.subpaths[k[0]].knots[k[1]];
            let out = Point::new(knot.anchor.x + 5.0, knot.anchor.y + 15.0);
            *knot = photocraft_doc::Knot::smooth(knot.anchor, Point::new(2.0 * knot.anchor.x - out.x, 2.0 * knot.anchor.y - out.y), out);
            assert_close(&work(&app), &want, "AddAnchorPoint: the new anchor's handles pulled out");
        } else {
            photocraft_vector::edit::move_anchors(&mut want, &[k], [5.0, 15.0]).unwrap();
            assert_close(&work(&app), &want, &format!("{tool:?}: the new anchor moved with its handles"));
        }
        assert_smooth(&work(&app).subpaths[0].knots[1]);
        assert_eq!(steps(&app), n + 1, "{tool:?}: one step");
        assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Add Anchor Point"));
        assert!(app.ui.pen.is_none(), "{tool:?}: no Pen path was started");
        assert_eq!(app.ui.direct_selection.anchors, vec![k], "{tool:?}: it stays selected");
        assert!(app.session.undo());
        assert_eq!(work(&app), before, "{tool:?}: one undo takes back the add and the move");
    }
}
