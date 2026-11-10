//! #12893: Shift-click constrains a new Pen anchor to 0/45/90 degrees from the previous one.
//! At 45° the anchor lands at min(|dx|, |dy|) on both axes, not at the projection onto the ray.

use egui::Modifiers;

use super::*;
use crate::canvas::{ToolEvent, tool_event};
use crate::tools::vector_ui::pen::{pen_down, pen_up};

/// A 800 × 600 document with the Pen tool active.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.ui.tool = Tool::Pen;
    app
}

/// Shift-click a new anchor at `click` after the previous anchor at `prev`.
fn shift_click(app: &mut PhotocraftApp, _prev: [f64; 2], click: [f64; 2]) {
    tool_event(app, ToolEvent::Down { x: click[0], y: click[1], pressure: 1.0 }, Modifiers::SHIFT);
    tool_event(app, ToolEvent::Up { x: click[0], y: click[1] }, Modifiers::SHIFT);
}

#[test]
fn shift_click_45_from_300_300_to_400_440_lands_at_400_400() {
    // From (300,300), Shift-click at (400,440) gives (400,400): min(|100|,|140|)=100 on both axes.
    let mut app = app();
    app.ui.tool = Tool::Pen;
    pen_down(&mut app, 300.0, 300.0, false);
    pen_up(&mut app);
    shift_click(&mut app, [300.0, 300.0], [400.0, 440.0]);
    let p = app.ui.pen.as_ref().unwrap().knots.last().unwrap()[0];
    assert_eq!((p[0], p[1]), (400.0, 400.0), "45° shift-click: {p:?}");
}

#[test]
fn shift_click_45_from_100_300_to_200_360_lands_at_160_360() {
    // From (100,300), Shift-click at (200,360) gives (160,360): min(|100|,|60|)=60 on both axes.
    let mut app = app();
    app.ui.tool = Tool::Pen;
    pen_down(&mut app, 100.0, 300.0, false);
    pen_up(&mut app);
    shift_click(&mut app, [100.0, 300.0], [200.0, 360.0]);
    let p = app.ui.pen.as_ref().unwrap().knots.last().unwrap()[0];
    assert_eq!((p[0], p[1]), (160.0, 360.0), "45° shift-click: {p:?}");
}

#[test]
fn shift_click_45_chain_from_100_300() {
    // From (100,300): click (200,360) -> (160,360); then click (240,500) -> (240,440).
    let mut app = app();
    app.ui.tool = Tool::Pen;
    pen_down(&mut app, 100.0, 300.0, false);
    pen_up(&mut app);
    shift_click(&mut app, [100.0, 300.0], [200.0, 360.0]);
    let p1 = app.ui.pen.as_ref().unwrap().knots.last().unwrap()[0];
    assert_eq!((p1[0], p1[1]), (160.0, 360.0), "first shift-click: {p1:?}");
    shift_click(&mut app, [160.0, 360.0], [240.0, 500.0]);
    let p2 = app.ui.pen.as_ref().unwrap().knots.last().unwrap()[0];
    assert_eq!((p2[0], p2[1]), (240.0, 440.0), "second shift-click: {p2:?}");
}

#[test]
fn shift_click_0_degree_still_matches() {
    // From (100,300), Shift-click at (300,340) gives (300,300): dx=200, dy=40, project to (300,300).
    let mut app = app();
    app.ui.tool = Tool::Pen;
    pen_down(&mut app, 100.0, 300.0, false);
    pen_up(&mut app);
    shift_click(&mut app, [100.0, 300.0], [300.0, 340.0]);
    let p = app.ui.pen.as_ref().unwrap().knots.last().unwrap()[0];
    assert_eq!((p[0], p[1]), (300.0, 300.0), "0° shift-click: {p:?}");
}

#[test]
fn click_without_shift_is_unchanged() {
    // Without ⇧ the click lands exactly where it was pressed.
    let mut app = app();
    app.ui.tool = Tool::Pen;
    pen_down(&mut app, 300.0, 300.0, false);
    pen_up(&mut app);
    tool_event(&mut app, ToolEvent::Down { x: 400.0, y: 440.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 400.0, y: 440.0 }, Modifiers::NONE);
    let p = app.ui.pen.as_ref().unwrap().knots.last().unwrap()[0];
    assert_eq!((p[0], p[1]), (400.0, 440.0), "no shift: {p:?}");
}
