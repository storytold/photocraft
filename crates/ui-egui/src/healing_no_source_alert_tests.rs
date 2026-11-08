//! #12109: a Healing Brush stroke before any Alt-click shows the reference application's alert
//! instead of a status-bar line, and paints nothing.

use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::{DialogKind, Tool};

const MESSAGE: &str = if cfg!(target_os = "macos") {
    "Option-click to define a source point to be used to repair the image."
} else {
    "Alt-click to define a source point to be used to repair the image."
};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 100, "height": 60})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 16, "hardness": 1.0}})).unwrap();
    for x in (0..100).step_by(4) {
        app.run("paint.pencil", json!({"points": [[x, 0], [x, 60]], "size": 2, "color": "#606060"})).unwrap();
    }
    app.ui.tool = Tool::Healing;
    app
}

fn drag(app: &mut PhotocraftApp, mods: egui::Modifiers) {
    tool_event(app, ToolEvent::Down { x: 10.0, y: 30.0, pressure: 1.0 }, mods);
    for x in [20.0, 30.0, 40.0, 50.0] {
        tool_event(app, ToolEvent::Move { x, y: 30.0, pressure: 1.0 }, mods);
    }
    tool_event(app, ToolEvent::Up { x: 50.0, y: 30.0 }, mods);
}

fn alerts(app: &PhotocraftApp) -> Vec<(u64, String)> {
    app.ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| Some((d.id, d.fields.get("message")?.as_str()?.to_string()))).collect()
}

fn state(app: &PhotocraftApp) -> (usize, Vec<f32>) {
    let st = app.session.active().unwrap();
    let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
    (st.history.past_len(), l.surface().unwrap().read_region(st.doc.bounds()))
}

#[test]
fn a_stroke_without_a_source_alerts_once_and_paints_nothing() {
    let mut app = app();
    let before = state(&app);
    drag(&mut app, egui::Modifiers::NONE);
    let shown = alerts(&app);
    assert_eq!(shown.iter().map(|(_, m)| m.as_str()).collect::<Vec<_>>(), [MESSAGE], "one alert for the whole drag");
    assert!(!app.ui.status_error, "no status-bar error: {}", app.ui.status);
    assert!(state(&app) == before, "nothing painted, no history state");
    // A second stroke while it is up doesn't stack another.
    drag(&mut app, egui::Modifiers::NONE);
    assert_eq!(alerts(&app).len(), 1);
    crate::dialogs::confirm(&mut app, shown[0].0).unwrap();
    assert!(app.ui.dialogs.is_empty());
    assert!(state(&app) == before, "OK changes nothing");
}

#[test]
fn after_an_alt_click_the_healing_brush_paints_without_an_alert() {
    let mut app = app();
    tool_event(&mut app, ToolEvent::Down { x: 70.0, y: 30.0, pressure: 1.0 }, egui::Modifiers::ALT);
    tool_event(&mut app, ToolEvent::Up { x: 70.0, y: 30.0 }, egui::Modifiers::ALT);
    let before = state(&app);
    drag(&mut app, egui::Modifiers::NONE);
    assert!(alerts(&app).is_empty());
    assert_eq!(state(&app).0, before.0 + 1, "one Healing Brush state");
}

#[test]
fn the_clone_stamp_keeps_its_status_line() {
    let mut app = app();
    app.ui.tool = Tool::CloneStamp;
    drag(&mut app, egui::Modifiers::NONE);
    assert!(alerts(&app).is_empty());
    assert!(app.ui.status_error && app.ui.status.contains("source point to clone from"), "{}", app.ui.status);
}
