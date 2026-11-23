//! Helpers of the refusal-alert tests: a document with a painted layer, a stroke, and the alerts it opened. The alert
//! is opened once through `dialogs::alerts::alert`, so a second press while it is open adds none.

use egui::Modifiers;
use serde_json::json;

use super::*;
use crate::canvas::{ToolEvent, tool_event};

/// A 64 × 64 document with a painted layer active, black foreground, `tool`.
pub(in crate::tools) fn app(tool: Tool) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#dc1e1e"})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#000000"})).unwrap();
    app.ui.tool = tool;
    app.sync_views();
    app
}

pub(super) fn active(app: &PhotocraftApp) -> u64 {
    app.session.active().unwrap().active_layer.unwrap().0
}

pub(in crate::tools) fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

/// A press, a short drag and the release with the active tool.
pub(in crate::tools) fn stroke(app: &mut PhotocraftApp) {
    tool_event(app, ToolEvent::Down { x: 20.0, y: 32.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Move { x: 44.0, y: 32.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: 44.0, y: 32.0 }, Modifiers::NONE);
}

/// The messages of the open framework alerts.
pub(in crate::tools) fn alerts(app: &PhotocraftApp) -> Vec<String> {
    app.framework.dialogs.open_dialogs().iter().filter_map(|d| d.fields.as_ref()?.get("message")?.as_str().map(str::to_string)).collect()
}

pub(in crate::tools) fn refused(app: &mut PhotocraftApp, want: &str) {
    let before = steps(app);
    stroke(app);
    assert_eq!(alerts(app), [want], "the alert, once");
    assert_eq!(steps(app), before, "nothing painted, no History step");
    let id = app.framework.dialogs.open_dialogs().first().map(|d| d.id).unwrap();
    let ctx = app.ctx().clone();
    crate::dialogs::confirm(app, &ctx, id).unwrap();
    assert!(alerts(app).is_empty(), "OK closes the alert");
}
