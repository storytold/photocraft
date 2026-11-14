//! #12886: When the Pattern Stamp can't paint (hidden target layer, Lock All, a group active),
//! a press opens the reference application's modal alert with OK instead of a status-bar message,
//! and paints nothing. Lock transparent pixels is not a refusal.

use egui::Modifiers;
use serde_json::{Value, json};

use super::*;
use crate::canvas::{ToolEvent, tool_event};

/// A 64 × 64 document with a painted layer active, black foreground.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#dc1e1e"})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#000000"})).unwrap();
    app.ui.tool = Tool::PatternStamp;
    app.sync_views();
    app
}

#[test]
fn hidden_layer() {
    let mut app = app();
    app.run("layer.setProps", json!({"layer": app.session.active().unwrap().active_layer.unwrap().0, "visible": false})).unwrap();
    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 32.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 20.0, y: 32.0 }, Modifiers::NONE);
    let alerts: Vec<String> = app.ui.dialogs.iter().filter(|d| d.kind == crate::state::DialogKind::Error).filter_map(|d| d.fields.get("message").and_then(Value::as_str).map(str::to_string)).collect();
    assert_eq!(alerts, ["Could not use the pattern stamp because the target layer is hidden."]);
}

#[test]
fn lock_all() {
    let mut app = app();
    app.run("layer.lockLayers", json!({"layer": app.session.active().unwrap().active_layer.unwrap().0, "all": true})).unwrap();
    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 32.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 20.0, y: 32.0 }, Modifiers::NONE);
    let alerts: Vec<String> = app.ui.dialogs.iter().filter(|d| d.kind == crate::state::DialogKind::Error).filter_map(|d| d.fields.get("message").and_then(Value::as_str).map(str::to_string)).collect();
    assert_eq!(alerts, ["Could not use the pattern stamp because the layer is locked."]);
}

#[test]
fn active_group() {
    let mut app = app();
    app.run("layer.new.group", json!({})).unwrap();
    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 32.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 20.0, y: 32.0 }, Modifiers::NONE);
    let alerts: Vec<String> = app.ui.dialogs.iter().filter(|d| d.kind == crate::state::DialogKind::Error).filter_map(|d| d.fields.get("message").and_then(Value::as_str).map(str::to_string)).collect();
    assert_eq!(alerts, ["Could not use the pattern stamp because the pixels in a group cannot be edited without merging the group."]);
}

#[test]
fn lock_transparent_pixels_paints() {
    let mut app = app();
    app.run("layer.lockLayers", json!({"layer": app.session.active().unwrap().active_layer.unwrap().0, "transparency": true})).unwrap();
    let journal_len_before = app.session.journal.len();
    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 32.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 20.0, y: 32.0 }, Modifiers::NONE);
    assert!(app.ui.dialogs.iter().filter(|d| d.kind == crate::state::DialogKind::Error).next().is_none());
    assert_eq!(app.session.journal.len(), journal_len_before + 1, "lock transparency should still paint");
}
