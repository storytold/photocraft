//! #11730: ⇧ while dragging a Color Sampler point keeps it on its dominant axis: a mostly
//! horizontal drag keeps the point's row, a mostly vertical one its column, for an existing point
//! and for a new one. Without ⇧ the drag is free, and a ⇧-drag that ends off the document does
//! what an unconstrained one does there.

use super::*;
use egui::Modifiers;

const NONE: Modifiers = Modifiers::NONE;
const SHIFT: Modifiers = Modifiers::SHIFT;

/// An 800 × 600 document with the Color Sampler tool.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.ui.tool = Tool::ColorSampler;
    app
}

/// A drag from `from` by `by`, in four moves, with `m` held throughout.
fn drag(app: &mut PhotocraftApp, from: [f64; 2], by: [f64; 2], m: Modifiers) {
    crate::canvas::tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, m);
    for k in 1..=4 {
        let t = f64::from(k) / 4.0;
        crate::canvas::tool_event(app, ToolEvent::Move { x: from[0] + by[0] * t, y: from[1] + by[1] * t, pressure: 1.0 }, m);
    }
    crate::canvas::tool_event(app, ToolEvent::Up { x: from[0] + by[0], y: from[1] + by[1] }, m);
}

fn samplers(app: &PhotocraftApp) -> Vec<[f64; 2]> {
    app.session.active().unwrap().color_samplers.clone()
}

#[test]
fn shift_keeps_an_existing_point_on_its_row_then_its_column() {
    let mut app = app();
    app.run("view.colorSamplers.add", json!({"x": 279.5, "y": 189.5})).unwrap();
    drag(&mut app, [279.5, 189.5], [100.0, 10.0], SHIFT);
    assert_eq!(samplers(&app), vec![[379.5, 189.5]], "+100,+10: horizontal");
    drag(&mut app, [379.5, 189.5], [10.0, 100.0], SHIFT);
    assert_eq!(samplers(&app), vec![[379.5, 289.5]], "+10,+100: vertical");
}

#[test]
fn shift_constrains_a_new_point_from_the_press() {
    let mut app = app();
    drag(&mut app, [279.5, 189.5], [100.0, 10.0], SHIFT);
    assert_eq!(samplers(&app), vec![[379.5, 189.5]]);
    drag(&mut app, [500.5, 100.5], [10.0, 100.0], SHIFT);
    assert_eq!(samplers(&app)[1], [500.5, 200.5]);
}

#[test]
fn the_marker_follows_the_constrained_point_while_dragging() {
    let mut app = app();
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 279.5, y: 189.5, pressure: 1.0 }, SHIFT);
    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 379.5, y: 199.5, pressure: 1.0 }, SHIFT);
    assert!(matches!(app.ui.analysis.drag, Some(Drag::SamplerNew([x, y])) if x == 379.5 && y == 189.5));
}

#[test]
fn without_shift_the_drag_is_free() {
    let mut app = app();
    app.run("view.colorSamplers.add", json!({"x": 279.5, "y": 189.5})).unwrap();
    drag(&mut app, [279.5, 189.5], [100.0, 10.0], NONE);
    assert_eq!(samplers(&app), vec![[379.5, 199.5]]);
}

#[test]
fn a_shift_drag_off_the_document_removes_the_point_as_before() {
    let mut app = app();
    app.run("view.colorSamplers.add", json!({"x": 279.5, "y": 189.5})).unwrap();
    drag(&mut app, [279.5, 189.5], [700.0, 10.0], SHIFT);
    assert!(samplers(&app).is_empty(), "dragged off the document: deleted");
    // A new point dragged off the document places none.
    drag(&mut app, [100.5, 100.5], [-300.0, 5.0], SHIFT);
    assert!(samplers(&app).is_empty());
}
