//! #14625: Edit › Presets lists only Preset Manager… and Migrate Presets, with no separator, as
//! in the reference application; Export/Import Presets stays an engine command for automation.

use serde_json::json;

use crate::PhotocraftApp;

fn presets_rows(app: &PhotocraftApp) -> Vec<(String, String)> {
    crate::menus::menu_items(app).into_iter().filter(|i| i.path.iter().map(String::as_str).eq(["Edit", "Presets"])).map(|i| (i.id, i.label)).collect()
}

fn app() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

#[test]
fn edit_presets_has_two_rows_with_and_without_a_document() {
    let want = vec![
        ("edit.presets.presetManager".to_string(), "Preset Manager…".to_string()),
        ("edit.presets.migratePresets".to_string(), "Migrate Presets".to_string()),
    ];
    let mut app = app();
    assert_eq!(presets_rows(&app), want, "no document");
    app.run("file.new", json!({"width": 32, "height": 24})).unwrap();
    assert_eq!(presets_rows(&app), want, "with a document");
}

#[test]
fn ui_menu_list_reports_the_same_rows() {
    let mut app = app();
    let ctx = egui::Context::default();
    let (req, _rx) = crate::control::ControlRequest::new("ui.menu.list", json!({}));
    let crate::control::Outcome::Done(listed) = crate::control::handle(&mut app, &ctx, &req) else { panic!("ui.menu.list replies at once") };
    let rows: Vec<&str> = listed["result"]
        .as_array()
        .or_else(|| listed.as_array())
        .into_iter()
        .flatten()
        .filter(|i| i["path"] == json!(["Edit", "Presets"]))
        .filter_map(|i| i["id"].as_str())
        .collect();
    assert_eq!(rows, ["edit.presets.presetManager", "edit.presets.migratePresets"], "{listed}");
}

#[test]
fn export_import_presets_is_in_no_menu_nor_in_the_shortcut_and_menus_lists() {
    let app = app();
    assert!(!crate::menus::menu_items(&app).iter().any(|i| i.id == "edit.presets.exportImportPresets" || i.label.contains("Export/Import")));
    // Edit › Keyboard Shortcuts (and the Menus tab, which lists the same rows) has no such row.
    assert!(!crate::prefs_ui::shortcut_items(&app).iter().any(|(id, ..)| id == "edit.presets.exportImportPresets"));
}

#[test]
fn the_engine_command_still_exports_and_imports_for_automation() {
    let mut s = photocraft_engine::Session::new();
    let out = s.execute("edit.presets.exportImportPresets", json!({"action": "export"})).unwrap();
    assert!(out.is_object(), "{out}");
    let data = out.get("data").cloned().unwrap_or(out.clone());
    let back = s.execute("edit.presets.exportImportPresets", json!({"action": "import", "data": data}));
    assert!(back.is_ok(), "{back:?}");
}
