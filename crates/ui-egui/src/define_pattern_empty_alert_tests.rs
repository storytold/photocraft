//! #14299: OK in Edit › Define Pattern… over an area with nothing visible shows the reference
//! application's error as an alert with OK, not only in the status bar, and adds no pattern.

use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;
use photocraft_engine::pattern_cmds::EMPTY_AREA;

fn alerts(app: &PhotocraftApp) -> Vec<&str> {
    app.ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| d.fields.get("message").and_then(Value::as_str)).collect()
}

/// Edit › Define Pattern… then OK in the Pattern Name dialog.
fn define_from_menu(app: &mut PhotocraftApp) -> Result<Value, String> {
    let ctx = egui::Context::default();
    crate::menus::invoke(app, &ctx, "edit.definePattern", Value::Null).unwrap();
    let id = app.ui.dialogs.last().expect("Define Pattern opens its dialog").id;
    crate::dialogs::confirm(app, id)
}

#[test]
fn ok_over_a_transparent_area_shows_the_empty_area_alert() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 30, "background": "transparent"})).unwrap();
    let patterns = app.session.patterns.items.len();
    assert_eq!(define_from_menu(&mut app).unwrap_err(), EMPTY_AREA);
    assert_eq!(alerts(&app), [EMPTY_AREA]);
    assert_eq!(app.session.patterns.items.len(), patterns, "no pattern added");
}

#[test]
fn ok_over_visible_pixels_defines_without_an_alert() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 30, "background": "transparent"})).unwrap();
    app.run("select.rect", json!({"x": 5, "y": 5, "width": 20, "height": 20})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let patterns = app.session.patterns.items.len();
    define_from_menu(&mut app).unwrap();
    assert!(alerts(&app).is_empty());
    assert_eq!(app.session.patterns.items.len(), patterns + 1);
}
