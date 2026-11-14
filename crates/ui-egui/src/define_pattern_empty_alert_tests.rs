use serde_json::Value;

use crate::PhotocraftApp;
use crate::state::DialogKind;

#[test]
fn empty_area_shows_error_alert_with_ok_not_only_status_bar() {
    let ctx = egui::Context::default();
    let mut app = PhotocraftApp::default();
    app.run("file.new", serde_json::json!({"width": 40, "height": 30, "depth": 8, "background": "transparent"})).unwrap();
    let before = app.session.patterns.items.len();

    crate::menus::invoke(&mut app, &ctx, "edit.definePattern", Value::Null).unwrap();
    let dialog_id = app.ui.dialogs.iter().find(|d| d.kind == DialogKind::Command).map(|d| d.id).expect("Pattern Name dialog");
    let err = crate::dialogs::confirm(&mut app, dialog_id).unwrap_err();
    assert!(err.contains(photocraft_engine::pattern_cmds::EMPTY_AREA_ERROR), "{err}");

    let alerts: Vec<&str> =
        app.ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| d.fields.get("message").and_then(Value::as_str)).collect();
    assert_eq!(alerts, [photocraft_engine::pattern_cmds::EMPTY_AREA_ERROR]);
    assert!(app.ui.status.is_empty(), "error should not be relegated to status bar: {:?}", app.ui.status);
    assert_eq!(app.session.patterns.items.len(), before);
}
