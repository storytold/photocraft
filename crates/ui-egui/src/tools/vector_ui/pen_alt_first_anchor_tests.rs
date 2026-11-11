//! Measured first-anchor Alt-drag behaviour from #12895, through canvas tool dispatch.

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;
use egui::Modifiers;
use serde_json::json;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Pen;
    app
}

fn down(app: &mut PhotocraftApp, x: f64, y: f64) {
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, Modifiers::NONE);
}

fn move_to(app: &mut PhotocraftApp, x: f64, y: f64, mods: Modifiers) {
    tool_event(app, ToolEvent::Move { x, y, pressure: 1.0 }, mods);
}

fn up(app: &mut PhotocraftApp, x: f64, y: f64, mods: Modifiers) {
    tool_event(app, ToolEvent::Up { x, y }, mods);
}

#[test]
fn alt_during_first_anchor_drag_collapses_incoming_and_keeps_outgoing_live() {
    let mut app = app();
    down(&mut app, 100.0, 300.0);
    move_to(&mut app, 200.0, 300.0, Modifiers::NONE);
    assert_eq!(app.ui.pen.as_ref().unwrap().knots[0][1], [0.0, 300.0]);
    move_to(&mut app, 200.0, 200.0, Modifiers::ALT);
    assert_eq!(app.ui.pen.as_ref().unwrap().knots[0], [[100.0, 300.0], [100.0, 300.0], [200.0, 200.0]]);
    // Repeated Alt moves must keep the outgoing handle live without relinking the pair.
    move_to(&mut app, 210.0, 210.0, Modifiers::ALT);
    move_to(&mut app, 200.0, 200.0, Modifiers::ALT);
    up(&mut app, 200.0, 200.0, Modifiers::ALT);
    let pen = app.ui.pen.as_ref().unwrap();
    assert_eq!(pen.knots[0], [[100.0, 300.0], [100.0, 300.0], [200.0, 200.0]]);
    assert_eq!(pen.unlinked, vec![0]);
    assert!(!pen.dragging);
}

#[test]
fn alt_during_later_anchor_drag_keeps_incoming_frozen() {
    let mut app = app();
    down(&mut app, 100.0, 300.0);
    up(&mut app, 100.0, 300.0, Modifiers::NONE);
    let first = app.ui.pen.as_ref().unwrap().knots[0];
    down(&mut app, 300.0, 300.0);
    move_to(&mut app, 400.0, 300.0, Modifiers::NONE);
    move_to(&mut app, 400.0, 200.0, Modifiers::ALT);
    up(&mut app, 400.0, 200.0, Modifiers::ALT);
    let pen = app.ui.pen.as_ref().unwrap();
    assert_eq!(pen.knots[0], first);
    assert_eq!(pen.knots[1], [[300.0, 300.0], [200.0, 300.0], [400.0, 200.0]]);
    assert_eq!(pen.unlinked, vec![1]);
}

#[test]
fn first_anchor_drag_without_alt_keeps_mirrored_handles() {
    let mut app = app();
    down(&mut app, 100.0, 300.0);
    move_to(&mut app, 200.0, 200.0, Modifiers::NONE);
    up(&mut app, 200.0, 200.0, Modifiers::NONE);
    let pen = app.ui.pen.as_ref().unwrap();
    assert_eq!(pen.knots[0], [[100.0, 300.0], [0.0, 400.0], [200.0, 200.0]]);
    assert!(pen.unlinked.is_empty());
}
