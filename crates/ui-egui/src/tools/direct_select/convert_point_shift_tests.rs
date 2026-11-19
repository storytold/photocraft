//! #12739: ⇧ while dragging with the Convert Point tool locks the handle to the nearest 45°
//! step about its anchor, keeping the drag's x component (measured in the reference app on the
//! document below: (+60,+25) gives (+60,0) and (−78,+140) gives (−78,+78)). ⇧ decides at the
//! gesture's release: pressed during the drag it constrains from then on, released during it the
//! drag ends as today. Without ⇧, with other tools, and for the vertical band whose component
//! choice was not measured, the behaviour stays today's.

use egui::Modifiers;
use photocraft_geom::Point;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::state::Tool;

/// The issue's measured test document: 800×600 px, a Work Path with a closed corner square
/// (100,300)(300,300)(300,500)(100,500) and an open S-curve (400,450) smooth with out-handle
/// (500,350) to (700,450) smooth with in-handle (600,550).
fn app(tool: Tool) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    let path = json!({"subpaths": [
        {"closed": true, "knots": [[100, 300], [300, 300], [300, 500], [100, 500]]},
        {"closed": false, "knots": [
            {"anchor": [400, 450], "out": [500, 350], "smooth": true},
            {"anchor": [700, 450], "in": [600, 550], "smooth": true},
        ]},
    ]});
    app.run("path.set", json!({"name": "work", "path": path})).unwrap();
    app.sync_views();
    app.ui.tool = tool;
    app
}

fn work(app: &PhotocraftApp) -> photocraft_doc::vector::Path {
    app.session.active().unwrap().doc.work_path.clone().unwrap()
}

fn knot(app: &PhotocraftApp, subpath: usize, k: usize) -> photocraft_doc::Knot {
    work(app).subpaths[subpath].knots[k]
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

fn p(x: f64, y: f64) -> Point {
    Point::new(x, y)
}

/// The pointer round-trips doc→screen→doc through the fit-zoomed view, so a dragged point can
/// land a few 1e-5 off; the measured steps are a whole component apart (25 px and more), so a
/// 1e-3 tolerance pins them exactly.
fn close(a: Point, b: Point) -> bool {
    (a.x - b.x).abs() < 1e-3 && (a.y - b.y).abs() < 1e-3
}

/// A four-point drag with `mods` held on every event.
fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], mods: Modifiers) {
    testing::press(app, from, mods);
    let mid = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
    testing::move_to(app, mid, mods);
    testing::move_to(app, to, mods);
    testing::release(app, to, mods);
}

#[test]
fn shift_drag_from_a_corner_lands_its_handles_on_the_horizontal_step_keeping_the_x_component() {
    // Measured: a ⇧-drag from the corner (300,300) by (+60,+25) gives handles (360,300) and
    // (240,300) in one history step.
    let mut app = app(Tool::ConvertPoint);
    let before = work(&app);
    let n = steps(&app);
    drag(&mut app, [300.0, 300.0], [360.0, 325.0], Modifiers::SHIFT);
    let k = knot(&app, 0, 1);
    assert!(close(k.anchor, p(300.0, 300.0)), "the anchor stays: {k:?}");
    assert!(close(k.out_ctrl, p(360.0, 300.0)), "out on the horizontal step, x kept: {k:?}");
    assert!(close(k.in_ctrl, p(240.0, 300.0)), "the in handle mirrors it: {k:?}");
    assert!(k.smooth, "the drag makes a smooth point: {k:?}");
    assert_eq!(steps(&app), n + 1, "one history step for the drag, ⇧ or not");
    assert!(app.session.undo());
    assert_eq!(work(&app), before);
}

#[test]
fn shift_drag_of_a_handle_end_lands_on_the_diagonal_step_keeping_the_x_component() {
    // Measured: the S-curve's end anchor (700,450), its in-handle dragged to (622,590), ends at
    // (622,528) on the 135° ray (x kept at −78); the opposite handle stays.
    let mut app = app(Tool::ConvertPoint);
    testing::click(&mut app, [700.0, 450.0], Modifiers::COMMAND);
    assert_eq!(app.ui.direct_selection.anchors, vec![[1, 1]], "⌘-click shows the handles");
    let n = steps(&app);
    drag(&mut app, [600.0, 550.0], [622.0, 590.0], Modifiers::SHIFT);
    let k = knot(&app, 1, 1);
    assert!(close(k.in_ctrl, p(622.0, 528.0)), "the handle ends on the 135° ray, x kept: {k:?}");
    assert!(close(k.out_ctrl, p(700.0, 450.0)), "the opposite handle is unchanged: {k:?}");
    assert!(close(knot(&app, 1, 0).out_ctrl, p(500.0, 350.0)), "the neighbour knot is unchanged");
    assert_eq!(steps(&app), n + 1, "one history step for the handle drag");
}

#[test]
fn shift_going_down_mid_drag_constrains_from_then_on() {
    let mut app = app(Tool::ConvertPoint);
    testing::press(&mut app, [300.0, 300.0], Modifiers::NONE);
    let mid = [330.0, 312.5];
    testing::move_to(&mut app, mid, Modifiers::NONE);
    testing::move_to(&mut app, [360.0, 325.0], Modifiers::SHIFT);
    testing::release(&mut app, [360.0, 325.0], Modifiers::SHIFT);
    let k = knot(&app, 0, 1);
    assert!(close(k.out_ctrl, p(360.0, 300.0)), "⇧ pressed during the drag constrains it: {k:?}");
    assert!(close(k.in_ctrl, p(240.0, 300.0)), "and its mirror: {k:?}");
}

#[test]
fn shift_released_mid_drag_gives_todays_result() {
    let mut app = app(Tool::ConvertPoint);
    testing::press(&mut app, [300.0, 300.0], Modifiers::SHIFT);
    let mid = [330.0, 312.5];
    testing::move_to(&mut app, mid, Modifiers::SHIFT);
    testing::move_to(&mut app, [360.0, 325.0], Modifiers::NONE);
    testing::release(&mut app, [360.0, 325.0], Modifiers::NONE);
    let k = knot(&app, 0, 1);
    assert!(close(k.out_ctrl, p(360.0, 325.0)), "⇧ off at the release: the drag stays as today: {k:?}");
    assert!(close(k.in_ctrl, p(240.0, 275.0)), "and its mirror: {k:?}");
}

#[test]
fn a_shift_free_convert_drag_keeps_todays_handles() {
    let mut app = app(Tool::ConvertPoint);
    let n = steps(&app);
    drag(&mut app, [300.0, 300.0], [360.0, 325.0], Modifiers::NONE);
    let k = knot(&app, 0, 1);
    assert!(close(k.anchor, p(300.0, 300.0)), "the anchor stays: {k:?}");
    assert!(close(k.out_ctrl, p(360.0, 325.0)), "without ⇧ the handles follow the pointer: {k:?}");
    assert!(close(k.in_ctrl, p(240.0, 275.0)), "and its mirror: {k:?}");
    assert!(k.smooth, "the drag makes a smooth point: {k:?}");
    assert_eq!(steps(&app), n + 1);
}

#[test]
fn a_shift_convert_drag_in_the_unmeasured_vertical_band_keeps_todays_handles() {
    // The reference keeps the x component at the measured angles, but which component it keeps
    // when the step would be vertical was *not measured* (#12739): that drag keeps today's
    // unconstrained result.
    let mut app = app(Tool::ConvertPoint);
    drag(&mut app, [300.0, 300.0], [310.0, 400.0], Modifiers::SHIFT);
    let k = knot(&app, 0, 1);
    assert!(close(k.out_ctrl, p(310.0, 400.0)), "a near-vertical ⇧-drag stays today's: {k:?}");
    assert!(close(k.in_ctrl, p(290.0, 200.0)), "and its mirror: {k:?}");
}

#[test]
fn direct_selection_shift_anchor_drag_keeps_todays_lock() {
    // The #5475 lock: ⇧ keeps a Direct Selection anchor drag on the axis the framework's 30°
    // AXIS_BAND rule picks (#13243: (+60,+25) flattens to (+60,0)); the Convert Point step lock
    // must not reach it.
    let mut app = app(Tool::DirectSelection);
    drag(&mut app, [300.0, 500.0], [360.0, 525.0], Modifiers::SHIFT);
    assert!(close(knot(&app, 0, 2).anchor, p(360.0, 500.0)), "the anchor drag keeps its own 45° rule");
}

#[test]
fn direct_selection_shift_handle_drag_keeps_todays_turn() {
    // A Direct Selection handle drag with ⇧: not measured for #12739, stays today's free turn
    // (the dragged end follows the pointer; the anchor's retracted other handle has no length to
    // turn, so it stays at the anchor and the point stays smooth).
    let mut app = app(Tool::DirectSelection);
    testing::click(&mut app, [700.0, 450.0], Modifiers::NONE);
    drag(&mut app, [600.0, 550.0], [622.0, 590.0], Modifiers::SHIFT);
    let k = knot(&app, 1, 1);
    assert!(close(k.in_ctrl, p(622.0, 590.0)), "the dragged handle follows the pointer: {k:?}");
    assert!(close(k.out_ctrl, p(700.0, 450.0)), "the retracted other handle stays put: {k:?}");
    assert!(k.smooth, "and the point stays smooth: {k:?}");
}
