//! #12707: a drag from a segment with Add Anchor Point inserts the anchor and pulls out its
//! handles, as the Pen does: the outgoing handle follows the pointer and the incoming one mirrors
//! it. ⇧ turns the handle to 45°, ⌥ breaks the pair, Space moves the anchor; the add and the drag
//! are one history step, Add Anchor Point.

use egui::Modifiers;
use photocraft_doc::Knot;
use photocraft_geom::Point;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::direct_select::PathRef;
use crate::state::Tool;

/// 600 x 600 with the closed corner square (100,300) (300,300) (300,500) (100,500) as the saved
/// path "Path 1", selected in the Paths panel, and Add Anchor Point picked.
fn square() -> PhotocraftApp {
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
    tool_event(app, ToolEvent::Down { x: at[0], y: at[1], pressure: 1.0 }, Modifiers::NONE);
}

fn to(app: &mut PhotocraftApp, at: [f64; 2], mods: Modifiers) {
    tool_event(app, ToolEvent::Move { x: at[0], y: at[1], pressure: 1.0 }, mods);
}

fn up(app: &mut PhotocraftApp, at: [f64; 2], mods: Modifiers) {
    tool_event(app, ToolEvent::Up { x: at[0], y: at[1] }, mods);
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
fn space_moves_the_anchor_with_its_handles_and_letting_go_returns_to_the_handle() {
    let mut app = square();
    let n = steps(&app);
    down(&mut app, [300.0, 400.0]);
    to(&mut app, [340.0, 400.0], Modifiers::NONE);
    assert!(crate::anchor_tools::set_reposition(&mut app, true), "the canvas keeps Space from panning");
    to(&mut app, [340.0, 420.0], Modifiers::NONE);
    to(&mut app, [340.0, 440.0], Modifiers::NONE);
    crate::anchor_tools::set_reposition(&mut app, false);
    to(&mut app, [352.0, 440.0], Modifiers::NONE);
    up(&mut app, [352.0, 440.0], Modifiers::NONE);
    let k = knot_at(&saved(&app), [300.0, 440.0]);
    assert_knot(&k, [300.0, 440.0], [352.0, 440.0], [248.0, 440.0]);
    assert_eq!((steps(&app), undo_label(&app).as_deref()), (n + 1, Some("Add Anchor Point")));
    assert!(!crate::anchor_tools::set_reposition(&mut app, true), "no drag, nothing to reposition");
}

/// A pull whose anchor Space moved is kept when the pointer comes back to the press point before
/// the release: the release is not a click.
#[test]
fn space_moves_are_kept_when_the_pointer_returns_to_the_press() {
    let mut app = square();
    down(&mut app, [300.0, 400.0]);
    to(&mut app, [340.0, 400.0], Modifiers::NONE);
    crate::anchor_tools::set_reposition(&mut app, true);
    to(&mut app, [340.0, 440.0], Modifiers::NONE);
    crate::anchor_tools::set_reposition(&mut app, false);
    to(&mut app, [300.0, 400.0], Modifiers::NONE);
    up(&mut app, [300.0, 400.0], Modifiers::NONE);
    let k = knot_at(&saved(&app), [300.0, 440.0]);
    assert_knot(&k, [300.0, 440.0], [300.0, 400.0], [300.0, 480.0]);
}

/// The Space case through the real canvas: the held key reaches the drag, and the view does not pan.
#[test]
fn space_in_the_canvas_moves_the_anchor_instead_of_panning() {
    use egui::{Event, Key, PointerButton, pos2};
    let mut a = square();
    a.ui.extras.rulers = false;
    a.ui.views = vec![crate::state::View {
        zoom: 1.0,
        center: [300.0, 300.0],
        fit_pending: false,
        fill_pending: false,
        doc_size: [600, 600],
        rotation: 0.0,
        zoom_animating: false,
        zoom_anchor: None,
        display_camera: None,
        zoom_transition: None,
    }];
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(600.0, 600.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let v = app.ui.views[0].clone();
            crate::canvas::canvas_view(app, ui, 0, ui.max_rect(), v, true);
        },
        a,
    );
    h.run_steps(2);
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 600.0));
    let xf = crate::canvas::ViewXform { rect, zoom: 1.0, center: [300.0, 300.0], flip: false, rotation: 0.0, aspect: 1.0 };
    let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    let space = |pressed| Event::Key { key: Key::Space, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
    let press = xf.to_screen(300.0, 400.0);
    h.event(Event::PointerMoved(press));
    h.step();
    h.event(button(press, true));
    h.step();
    let mv = |h: &mut egui_kittest::Harness<'_, PhotocraftApp>, x: f32, y: f32| {
        let p = xf.to_screen(x, y);
        h.event(Event::PointerMoved(pos2(p.x, p.y)));
        h.step();
    };
    for x in [320.0, 340.0] {
        mv(&mut h, x, 400.0);
    }
    h.event(space(true));
    h.step();
    for y in [420.0, 440.0] {
        mv(&mut h, 340.0, y);
    }
    h.event(space(false));
    h.step();
    mv(&mut h, 352.0, 440.0);
    let end = xf.to_screen(352.0, 440.0);
    h.event(button(end, false));
    h.run_steps(2);
    assert_eq!(h.state().ui.views[0].center, [300.0, 300.0], "Space did not pan");
    let k = knot_at(&saved(h.state()), [300.0, 440.0]);
    assert_knot(&k, [300.0, 440.0], [352.0, 440.0], [248.0, 440.0]);
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
    assert_eq!(saved(&app), want, "only the add");
    assert_eq!((steps(&app), undo_label(&app).as_deref()), (n + 1, Some("Add Anchor Point")));
}

#[test]
fn non_finite_pointer_positions_leave_the_drag_alone() {
    let mut app = square();
    down(&mut app, [200.0, 500.0]);
    to(&mut app, [200.0, 560.0], Modifiers::NONE);
    to(&mut app, [f64::NAN, f64::INFINITY], Modifiers::SHIFT);
    up(&mut app, [200.0, 560.0], Modifiers::NONE);
    assert_knot(&knot_at(&saved(&app), [200.0, 500.0]), [200.0, 500.0], [200.0, 560.0], [200.0, 440.0]);
}

/// The Space case driven by an agent: `ui.pointer` with `space: true` moves the anchor.
#[test]
fn an_agents_space_moves_the_anchor() {
    let mut app = square();
    let ctx = egui::Context::default();
    let pointer = |app: &mut PhotocraftApp, params: serde_json::Value| {
        let (req, _rx) = crate::control::ControlRequest::new("ui.pointer", params);
        match crate::control::handle(app, &ctx, &req) {
            crate::control::Outcome::Done(v) => assert_eq!(v["ok"], true, "{v}"),
            _ => panic!("expected an immediate reply"),
        }
    };
    pointer(&mut app, json!({"events": [{"kind": "down", "x": 300, "y": 400}, {"kind": "move", "x": 340, "y": 400}]}));
    pointer(&mut app, json!({"space": true, "events": [{"kind": "move", "x": 340, "y": 440}]}));
    pointer(&mut app, json!({"events": [{"kind": "move", "x": 352, "y": 440}, {"kind": "up", "x": 352, "y": 440}]}));
    assert_knot(&knot_at(&saved(&app), [300.0, 440.0]), [300.0, 440.0], [352.0, 440.0], [248.0, 440.0]);
}
