//! #12707: a drag from a segment with Add Anchor Point inserts the anchor and pulls out its
//! handles, as the Pen does: the outgoing handle follows the pointer and the incoming one mirrors
//! it. ⇧ turns the handle to 45°, ⌥ breaks the pair; the add and the drag are one history step,
//! Add Anchor Point. The drag is the overlay's (`PathEdit::PullAnchor`, storytold/craft-libs#2285),
//! mapped in `direct_select/edits.rs` (`add_and_pull`).

use egui::Modifiers;
use photocraft_doc::Knot;
use photocraft_geom::Point;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::direct_select::PathRef;
use crate::state::Tool;

/// 600 x 600 with the closed corner square (100,300) (300,300) (300,500) (100,500) as the saved
/// path "Path 1", selected in the Paths panel, and Add Anchor Point picked.
fn square() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 600, "height": 600})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::AddAnchorPoint;
    app.ui.extras.snap = false;
    let path = json!({"subpaths": [{"closed": true, "knots": [[100, 300], [300, 300], [300, 500], [100, 500]]}]});
    app.run("path.set", json!({"name": "Path 1", "path": path})).unwrap();
    app.ui.selected_path = Some("Path 1".into());
    app
}

fn saved(app: &PhotocraftApp) -> photocraft_doc::vector::Path {
    crate::direct_select::path_of(app, &PathRef::Saved(0)).unwrap().into_owned()
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

fn undo_label(app: &PhotocraftApp) -> Option<String> {
    app.session.active().unwrap().history.undo_label().map(str::to_owned)
}

fn down(app: &mut PhotocraftApp, at: [f64; 2]) {
    testing::press(app, at, Modifiers::NONE);
}

fn to(app: &mut PhotocraftApp, at: [f64; 2], mods: Modifiers) {
    testing::move_to(app, at, mods);
}

fn up(app: &mut PhotocraftApp, at: [f64; 2], mods: Modifiers) {
    testing::release(app, at, mods);
}

/// The knot at `anchor` (within a pixel) on `path`.
fn knot_at(path: &photocraft_doc::vector::Path, anchor: [f64; 2]) -> Knot {
    let near = |p: Point| (p.x - anchor[0]).hypot(p.y - anchor[1]) <= 1.0;
    *path.subpaths[0].knots.iter().find(|k| near(k.anchor)).unwrap_or_else(|| panic!("no anchor at {anchor:?}: {path:?}"))
}

#[track_caller]
fn assert_knot(k: &Knot, anchor: [f64; 2], out: [f64; 2], inc: [f64; 2]) {
    let near = |p: Point, q: [f64; 2]| (p.x - q[0]).hypot(p.y - q[1]) <= 0.5;
    assert!(near(k.anchor, anchor) && near(k.out_ctrl, out) && near(k.in_ctrl, inc), "want anchor {anchor:?} out {out:?} in {inc:?}, got {k:?}");
}

#[test]
fn a_drag_from_the_bottom_edge_pulls_out_mirrored_handles_in_one_step() {
    let mut app = square();
    let (before, n) = (saved(&app), steps(&app));
    down(&mut app, [200.0, 500.0]);
    for y in [520.0, 540.0, 560.0] {
        to(&mut app, [200.0, y], Modifiers::NONE);
    }
    up(&mut app, [200.0, 560.0], Modifiers::NONE);
    let path = saved(&app);
    assert_eq!(path.subpaths[0].knots.len(), 5);
    let k = knot_at(&path, [200.0, 500.0]);
    assert_knot(&k, [200.0, 500.0], [200.0, 560.0], [200.0, 440.0]);
    assert!(k.smooth);
    assert_eq!((steps(&app), undo_label(&app).as_deref()), (n + 1, Some("Add Anchor Point")));
    assert!(app.session.undo());
    assert_eq!(saved(&app), before, "one undo takes back the add and the pull");
}

#[test]
fn shift_turns_the_handle_to_45_degrees_and_the_in_handle_mirrors_it() {
    // The anchor goes in at (200,500); the drags are offsets from it.
    for (d, want) in [
        ([30.0, 80.0], [0.0, 80.0]),
        ([70.0, 40.0], [40.0, 40.0]),
        ([-50.0, -60.0], [-50.0, -50.0]),
        ([60.0, -20.0], [60.0, 0.0]),
        ([50.0, 45.0], [45.0, 45.0]),
    ] {
        let mut app = square();
        down(&mut app, [200.0, 500.0]);
        let at = [200.0 + d[0], 500.0 + d[1]];
        to(&mut app, at, Modifiers::SHIFT);
        up(&mut app, at, Modifiers::SHIFT);
        let k = knot_at(&saved(&app), [200.0, 500.0]);
        assert_knot(&k, [200.0, 500.0], [200.0 + want[0], 500.0 + want[1]], [200.0 - want[0], 500.0 - want[1]]);
    }
}

#[test]
fn alt_during_the_drag_breaks_the_pair_so_only_the_out_handle_follows() {
    let mut app = square();
    let n = steps(&app);
    down(&mut app, [100.0, 400.0]);
    to(&mut app, [40.0, 400.0], Modifiers::NONE);
    to(&mut app, [40.0, 420.0], Modifiers::ALT);
    to(&mut app, [40.0, 440.0], Modifiers::ALT);
    up(&mut app, [40.0, 440.0], Modifiers::ALT);
    let k = knot_at(&saved(&app), [100.0, 400.0]);
    assert_knot(&k, [100.0, 400.0], [40.0, 440.0], [160.0, 400.0]);
    assert_eq!((steps(&app), undo_label(&app).as_deref()), (n + 1, Some("Add Anchor Point")));
}

#[test]
fn the_drag_works_on_a_shape_layers_path_in_one_step() {
    // 800 x 600, a shape from (450,100) to (650,250), made the active layer.
    let (mut app, shape) = super::delete_anchor_targeted_path_tests::setup(Tool::AddAnchorPoint);
    super::delete_anchor_targeted_path_tests::select_layer(&mut app, shape);
    app.ui.extras.snap = false;
    let n = steps(&app);
    down(&mut app, [550.0, 250.0]);
    to(&mut app, [550.0, 280.0], Modifiers::NONE);
    to(&mut app, [550.0, 300.0], Modifiers::NONE);
    up(&mut app, [550.0, 300.0], Modifiers::NONE);
    let path = crate::direct_select::path_of(&app, &PathRef::Layer(shape)).unwrap().into_owned();
    assert_knot(&knot_at(&path, [550.0, 250.0]), [550.0, 250.0], [550.0, 300.0], [550.0, 200.0]);
    assert_eq!((steps(&app), undo_label(&app).as_deref()), (n + 1, Some("Add Anchor Point")));
}

#[test]
fn a_click_without_movement_keeps_the_plain_add() {
    let mut app = square();
    let mut want = saved(&app);
    let Some(photocraft_vector::edit::Hit::Segment(k, t)) = photocraft_vector::edit::hit(&want, Point::new(200.0, 500.0), 6.0, &[]) else {
        panic!("no segment")
    };
    photocraft_vector::edit::add_anchor(&mut want, k, t).unwrap();
    let n = steps(&app);
    down(&mut app, [200.0, 500.0]);
    up(&mut app, [200.0, 500.0], Modifiers::NONE);
    // Only the add: the canvas maps the click in f32, so the anchor lands within a hundredth of a pixel.
    let got = saved(&app);
    assert_eq!(got.subpaths[0].knots.len(), want.subpaths[0].knots.len(), "one anchor added");
    for (g, w) in got.subpaths[0].knots.iter().zip(&want.subpaths[0].knots) {
        let near = |p: Point, q: Point| (p.x - q.x).hypot(p.y - q.y) < 0.01;
        assert!(near(g.anchor, w.anchor) && near(g.in_ctrl, w.in_ctrl) && near(g.out_ctrl, w.out_ctrl) && g.smooth == w.smooth, "{g:?} vs {w:?}");
    }
    assert_eq!((steps(&app), undo_label(&app).as_deref()), (n + 1, Some("Add Anchor Point")));
}
