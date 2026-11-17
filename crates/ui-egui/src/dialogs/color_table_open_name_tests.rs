use serde_json::json;

use crate::PhotocraftApp;

fn new_indexed_doc(app: &mut PhotocraftApp, mode: &str, palette: &str) {
    app.run("file.new", json!({"width": 8, "height": 8, "mode": mode, "background": "white"})).unwrap();
    if mode != "grayscale" {
        app.run("image.mode.indexedColor", json!({"palette": palette})).unwrap();
    } else {
        app.run("image.mode.indexedColor", json!({})).unwrap();
    }
}

fn open_table(app: &PhotocraftApp, id: u64) -> Option<String> {
    app.ui.dialogs.iter().find(|d| d.id == id)?.fields.get("table")?.as_str().map(str::to_owned)
}

#[test]
fn color_table_does_not_remember_last_preset_and_leaves_document_unchanged_on_ok() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();

    new_indexed_doc(&mut app, "rgb", "web");
    let id = crate::menus::invoke(&mut app, &ctx, "image.mode.colorTable", json!({})).unwrap()["dialog"].as_u64().unwrap();
    assert_eq!(open_table(&app, id).as_deref(), Some("custom"));
    app.ui.dialog_mut(id).unwrap().fields.insert("table".into(), json!("spectrum"));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert!(!app.session.prefs().dialogs.get("image.mode.colorTable").is_some_and(|v| v.get("table").is_some()));

    new_indexed_doc(&mut app, "rgb", "web");
    let before_table = app.session.active().unwrap().doc.color_table.clone();
    let before_history = app.session.active().unwrap().history.past_len();

    let id2 = crate::menus::invoke(&mut app, &ctx, "image.mode.colorTable", json!({})).unwrap()["dialog"].as_u64().unwrap();
    assert_eq!(open_table(&app, id2).as_deref(), Some("custom"));
    crate::dialogs::confirm(&mut app, id2).unwrap();

    assert_eq!(app.session.active().unwrap().doc.color_table, before_table);
    assert_eq!(app.session.active().unwrap().history.past_len(), before_history);
}

#[test]
fn color_table_opens_on_grayscale_for_grayscale_ramp_and_custom_for_other_tables() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();

    new_indexed_doc(&mut app, "grayscale", "");
    app.run("image.mode.colorTable", json!({"table": "grayscale"})).unwrap();
    let before_table = app.session.active().unwrap().doc.color_table.clone();
    assert_eq!(before_table.as_ref().map(|t| t.colors.as_slice()), photocraft_engine::mode_cmds::preset_table("grayscale").as_deref());
    let before_history = app.session.active().unwrap().history.past_len();

    let id = crate::menus::invoke(&mut app, &ctx, "image.mode.colorTable", json!({})).unwrap()["dialog"].as_u64().unwrap();
    assert_eq!(open_table(&app, id).as_deref(), Some("grayscale"));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(app.session.active().unwrap().doc.color_table, before_table);
    assert_eq!(app.session.active().unwrap().history.past_len(), before_history);

    for preset in ["blackBody", "spectrum", "systemMac", "systemWindows", "web"] {
        app.run("image.mode.colorTable", json!({"table": preset})).unwrap();
        let id = crate::menus::invoke(&mut app, &ctx, "image.mode.colorTable", json!({})).unwrap()["dialog"].as_u64().unwrap();
        assert_eq!(open_table(&app, id).as_deref(), Some("custom"), "preset {preset}");
        app.ui.dialogs.retain(|d| d.id != id);
    }
}

#[test]
fn other_schema_dialogs_still_remember_settings() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 32, "height": 32, "background": "white"})).unwrap();
    let id = crate::menus::invoke(&mut app, &ctx, "filter.blur.gaussianBlur", json!({})).unwrap()["dialog"].as_u64().unwrap();
    app.ui.dialog_mut(id).unwrap().fields.insert("radius".into(), json!(12.5));
    crate::dialogs::confirm(&mut app, id).unwrap();

    let id2 = crate::menus::invoke(&mut app, &ctx, "filter.blur.gaussianBlur", json!({})).unwrap()["dialog"].as_u64().unwrap();
    let remembered = app.ui.dialogs.iter().find(|d| d.id == id2).and_then(|d| d.fields.get("radius")).and_then(|v| v.as_f64());
    assert_eq!(remembered, Some(12.5));
}
