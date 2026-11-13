//! #13285: ⇧ pressed during a Path Selection drag locks the move as the reference application does:
//! (+60,+25) moves the selected components by (60,0) and (+60,+50) by (50,50). The live outline
//! shows the same offset the release applies. ⇧ held at the press keeps adding or removing
//! components, as before; the reference application moves nothing then, which this leaves open.

use egui::Modifiers;
use photocraft_doc::vector::Path;
use photocraft_geom::Affine;
use serde_json::json;

use super::{MovedPath, path_selection_preview};
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// On the square's top edge and on the triangle's top edge.
const ON_SQUARE: [f64; 2] = [200.0, 300.0];
const ON_TRIANGLE: [f64; 2] = [500.0, 300.0];

/// The work path with two components, the square 100,300–300,500 and the triangle 400,300 /
/// 600,300 / 500,500, both selected with Path Selection.
fn setup() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    let square = json!({"closed": true, "knots": [[100.0, 300.0], [300.0, 300.0], [300.0, 500.0], [100.0, 500.0]]});
    let triangle = json!({"closed": true, "knots": [[400.0, 300.0], [600.0, 300.0], [500.0, 500.0]]});
    app.run("path.set", json!({"path": {"subpaths": [square, triangle]}})).unwrap();
    app.ui.tool = Tool::PathSelection;
    app.ui.extras.snap = false;
    click(&mut app, ON_SQUARE, Modifiers::NONE);
    click(&mut app, ON_TRIANGLE, Modifiers::SHIFT);
    app
}

fn click(app: &mut PhotocraftApp, at: [f64; 2], mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x: at[0], y: at[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x: at[0], y: at[1] }, mods);
}

/// Presses at `from` with `press` held, moves half-way with it, then to `from + by` with `held`.
fn drag_to(app: &mut PhotocraftApp, from: [f64; 2], by: [f64; 2], press: Modifiers, held: Modifiers) -> [f64; 2] {
    let to = [from[0] + by[0], from[1] + by[1]];
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, press);
    tool_event(app, ToolEvent::Move { x: from[0] + by[0] / 2.0, y: from[1] + by[1] / 2.0, pressure: 1.0 }, press);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, held);
    to
}

fn work(app: &PhotocraftApp) -> Path {
    app.session.active().unwrap().doc.work_path.clone().unwrap()
}

fn preview(app: &PhotocraftApp) -> Option<Path> {
    let (which, path) = path_selection_preview(app, &app.session.active().unwrap().doc)?;
    assert_eq!(which, MovedPath::Work);
    Some(path)
}

/// A drag of both components by `by` with ⇧ pressed after the press: the path the live outline
/// shows, which must be the one the release leaves.
fn shift_drag(by: [f64; 2]) -> (Path, Path) {
    let mut app = setup();
    let before = work(&app);
    let to = drag_to(&mut app, ON_SQUARE, by, Modifiers::NONE, Modifiers::SHIFT);
    let shown = preview(&app).expect("the drag previews the move");
    tool_event(&mut app, ToolEvent::Up { x: to[0], y: to[1] }, Modifiers::SHIFT);
    assert_eq!(work(&app), shown, "the release lands where the live outline showed");
    (before, shown)
}

#[test]
fn shift_during_a_shallow_drag_keeps_it_horizontal() {
    let (before, after) = shift_drag([60.0, 25.0]);
    assert_eq!(after, before.transform(&Affine::translate(60.0, 0.0)));
}

#[test]
fn shift_during_a_steep_drag_keeps_the_shorter_leg_on_the_diagonal() {
    let (before, after) = shift_drag([60.0, 50.0]);
    assert_eq!(after, before.transform(&Affine::translate(50.0, 50.0)));
}

#[test]
fn shift_during_a_drag_locks_the_other_directions_alike() {
    let (before, after) = shift_drag([-25.0, -60.0]);
    assert_eq!(after, before.transform(&Affine::translate(0.0, -60.0)));
    let (before, after) = shift_drag([-60.0, 50.0]);
    assert_eq!(after, before.transform(&Affine::translate(-50.0, 50.0)));
}

#[test]
fn a_drag_without_shift_still_moves_freely() {
    let mut app = setup();
    let before = work(&app);
    let to = drag_to(&mut app, ON_SQUARE, [60.0, 25.0], Modifiers::NONE, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: to[0], y: to[1] }, Modifiers::NONE);
    assert_eq!(work(&app), before.transform(&Affine::translate(60.0, 25.0)));
}

#[test]
fn shift_released_before_the_release_moves_freely() {
    let mut app = setup();
    let before = work(&app);
    let to = drag_to(&mut app, ON_SQUARE, [60.0, 25.0], Modifiers::NONE, Modifiers::SHIFT);
    tool_event(&mut app, ToolEvent::Up { x: to[0], y: to[1] }, Modifiers::NONE);
    assert_eq!(work(&app), before.transform(&Affine::translate(60.0, 25.0)));
}

#[test]
fn shift_at_the_press_still_removes_the_component() {
    let mut app = setup();
    let before = work(&app);
    // ⇧-press on the selected triangle takes it out of the selection, so it stays where it is.
    let to = drag_to(&mut app, ON_TRIANGLE, [60.0, 25.0], Modifiers::SHIFT, Modifiers::SHIFT);
    let shown = preview(&app).expect("the drag previews the move");
    tool_event(&mut app, ToolEvent::Up { x: to[0], y: to[1] }, Modifiers::SHIFT);
    let after = work(&app);
    assert_eq!(after, shown, "the release lands where the live outline showed");
    assert_eq!(after.subpaths[1], before.subpaths[1], "the triangle was deselected and stays");
}

/// Not measured: the switch from the axis to 45° lies where the shorter leg is half the longer
/// one, and that tie keeps the axis.
#[test]
fn the_axis_holds_until_the_shorter_leg_passes_half_the_longer() {
    let (before, after) = shift_drag([60.0, 30.0]);
    assert_eq!(after, before.transform(&Affine::translate(60.0, 0.0)));
    let (before, after) = shift_drag([60.0, 31.0]);
    assert_eq!(after, before.transform(&Affine::translate(31.0, 31.0)));
}
