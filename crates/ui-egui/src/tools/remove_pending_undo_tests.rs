//! #12544: with Remove after each stroke off, Cmd+Z (Ctrl+Z) takes back the last waiting stroke, one per press, and
//! leaves the document and its history alone; with nothing waiting it undoes the last document step as before.

use egui::{Key, Modifiers};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// A red layer "Empty" over the Background ("New Layer" is the last document step), with the Remove tool collecting
/// strokes.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.run("edit.fill", json!({"color": "#dc1e1e"})).unwrap();
    app.run("layer.new.layer", json!({"name": "Empty"})).unwrap();
    let background = app.session.active().unwrap().doc.layers[0].id.0;
    app.run("layer.select", json!({"layer": background})).unwrap();
    app.ui.tool = Tool::Remove;
    app.ui.tool_options.remove_after_each_stroke = false;
    testing::shown(&mut app);
    app
}

fn stroke(app: &mut PhotocraftApp, y: f64) {
    tool_event(app, ToolEvent::Down { x: 12.0, y, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Move { x: 40.0, y, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: 40.0, y }, Modifiers::NONE);
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn layers(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.layers.len()
}

fn undo(app: &mut PhotocraftApp) {
    testing::key(app, Key::Z, Modifiers::COMMAND);
}

#[test]
fn undo_takes_back_one_waiting_stroke_per_press() {
    let mut app = app();
    for y in [12.0, 30.0, 48.0] {
        stroke(&mut app, y);
    }
    assert_eq!(app.remove_pending.len(), 3, "three strokes wait");
    let (before, count, journal) = (steps(&app), layers(&app), app.session.journal.len());
    undo(&mut app);
    assert_eq!(app.remove_pending.len(), 2);
    assert!(app.ui.status.contains('2'), "the status counts what is left: {}", app.ui.status);
    assert!(app.remove_trail.is_some(), "the strokes left are still tinted");
    assert_eq!((steps(&app), layers(&app), app.session.journal.len()), (before, count, journal), "the document is untouched");
    undo(&mut app);
    assert_eq!(app.remove_pending.len(), 1);
    undo(&mut app);
    assert!(app.remove_pending.is_empty(), "the last one taken back leaves nothing waiting: {}", app.remove_pending.len());
    assert!(app.remove_trail.is_none(), "and no tint");
    assert_eq!((steps(&app), layers(&app)), (before, count), "still no history change");
}

#[test]
fn with_nothing_waiting_undo_steps_back_the_document() {
    let mut app = app();
    let (before, count) = (steps(&app), layers(&app));
    undo(&mut app);
    assert_eq!(steps(&app), before - 1, "New Layer is undone");
    assert_eq!(layers(&app), count - 1, "the layer Empty is gone");
}

#[test]
fn undo_stays_enabled_while_strokes_wait() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.ui.tool = Tool::Remove;
    app.ui.tool_options.remove_after_each_stroke = false;
    testing::shown(&mut app);
    stroke(&mut app, 20.0);
    assert_eq!(app.remove_pending.len(), 1);
    assert!(crate::menus::is_enabled(&app, "edit.undo"), "a waiting stroke can be taken back");
    undo(&mut app);
    assert!(app.remove_pending.is_empty());
}
