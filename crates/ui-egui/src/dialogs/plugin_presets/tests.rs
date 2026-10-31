use super::*;
use egui_kittest::kittest::Queryable;

const MENU: &str = "plugin.filter.local.skinsmooth.skin-smooth";
const ID: &str = "local.skinsmooth.skin-smooth";

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("plugin.install", json!({"path":concat!(env!("CARGO_MANIFEST_DIR"), "/../plugins/tests/fixtures/skin-smooth.wasm")})).unwrap();
    app.run("file.new", json!({"width":16,"height":16})).unwrap();
    app
}

fn harness(size: egui::Vec2, theme: crate::theme::ThemeKind) -> egui_kittest::Harness<'static, PhotocraftApp> {
    let mut h = egui_kittest::Harness::builder().with_size(size).build_ui_state(|ui, app| super::super::show(app, ui.ctx()), app());
    PhotocraftApp::setup_context(&h.ctx, theme);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, MENU, json!({})).unwrap();
    h.run_steps(12);
    h
}

fn choose(h: &mut egui_kittest::Harness<'static, PhotocraftApp>, name: &str) {
    h.query_all_by_label("Preset").last().unwrap().click();
    h.run_steps(3);
    h.get_by_label(name).click();
    h.run_steps(3);
}

#[test]
fn an_unmanaged_plugin_keeps_its_own_preset_parameter() {
    let mut app = app();
    app.run("plugin.install", json!({"path":concat!(env!("CARGO_MANIFEST_DIR"), "/../plugins/tests/fixtures/invert.wasm")})).unwrap();
    let fields = json!({"id":"org.photocraft.example.invert", "__command":"plugin.run",
        "__spec":r#"{"preset":"first|second"}"#, "preset":"first", "__presetValues":{"preset":"first"}});
    let mut h = egui_kittest::Harness::builder().build_ui_state(crate::filter_dialog::body, fields.as_object().cloned().unwrap());
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(3);
    h.get_by_role(egui::accesskit::Role::ComboBox);
    h.state_mut().insert("preset".into(), json!("second"));
    sync_values(h.state_mut());
    assert_eq!(h.state()["__presetDirty"], true, "the plug-in parameter is an editable setting");
}

#[test]
fn selected_builtin_survives_edits_and_reset_preserves_switches() {
    let mut app = app();
    crate::menus::invoke(&mut app, &egui::Context::default(), MENU, json!({})).unwrap();
    let mut fields = app.ui.dialogs[0].fields.clone();
    assert_eq!(fields["__pluginPreset"], "builtin:natural");
    assert_eq!(fields["preset"], "manual");
    fields.insert("fine".into(), json!(12.0));
    fields.insert("skinOnly".into(), json!(false));
    fields.insert("__preview".into(), json!(true));
    sync_values(&mut fields);
    assert_eq!(fields["__presetDirty"], true);
    assert_eq!(fields["__pluginPreset"], "builtin:natural");
    reset(&app, &mut fields);
    assert_eq!(fields["fine"], 45.0);
    assert_eq!(fields["skinOnly"], false);
    assert_eq!(fields["__preview"], true);
    assert_eq!(fields["__presetDirty"], false);
    assert!(select(&app, &mut fields, "builtin:soft"));
    assert_eq!(fields["fine"], 70.0);
    assert_eq!(fields["skinOnly"], false);
    fields.insert("fine".into(), json!(70.0));
    sync_values(&mut fields);
    assert_eq!(fields["__presetDirty"], false, "slider float and manifest integer are equal");
    fields.insert("fine".into(), json!(13));
    crate::filter_dialog::reset_fields(&app, &mut fields);
    assert_eq!(fields["fine"], 70.0, "Alt+Reset uses the same selected baseline");
    assert_eq!(fields["skinOnly"], false);
    let m = photocraft_plugins::registry::get(ID).unwrap();
    assert_eq!(m.manifest().preset_values["soft"]["fine"], 70, "built-ins were not changed");
}

#[test]
fn merged_menu_new_cancel_save_reset_and_delete() {
    let mut h = harness(egui::vec2(1280.0, 800.0), crate::theme::ThemeKind::ALL[0]);
    assert!(h.query_by_label("Saved presets").is_none());
    h.query_all_by_label("Preset").last().unwrap().click();
    h.run_steps(3);
    assert!(h.query_by_label("Manual").is_none());
    for name in ["Natural", "Soft", "Clean"] {
        assert!(h.query_by_label(name).is_some());
    }
    h.get_by_label("Soft").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.dialogs[0].fields["fine"], 70.0);
    let revision = h.state().session.active().unwrap().revision;
    h.get_by_label("New…").click();
    h.run_steps(12);
    let name = h.query_all_by_label("Preset name").last().unwrap();
    assert!(name.is_focused());
    h.event(egui::Event::Text("Discarded".into()));
    h.run_steps(3);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert_eq!(h.state().ui.dialogs.len(), 1);
    assert!(!h.state().session.prefs().dialogs.contains_key(&format!("plugin.presets.{ID}")));
    h.get_by_label("New…").click();
    h.run_steps(12);
    h.event(egui::Event::Text("Portrait".into()));
    h.run_steps(3);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().ui.dialogs.len(), 1);
    assert_eq!(h.state().ui.dialogs[0].fields["__pluginPreset"], "saved:Portrait");
    h.state_mut().ui.dialogs[0].fields.insert("fine".into(), json!(12));
    h.run_steps(3);
    assert_eq!(h.state().ui.dialogs[0].fields["__presetDirty"], true);
    h.get_by_label("Reset preset").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.dialogs[0].fields["fine"], 70.0);
    choose(&mut h, "Natural");
    choose(&mut h, "Portrait");
    assert_eq!(h.state().ui.dialogs[0].fields["fine"], 70.0);
    h.get_by_label("Delete").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.dialogs[0].fields["__pluginPreset"], "builtin:natural");
    assert_eq!(h.state().ui.dialogs[0].fields["fine"], 45.0);
    assert!(h.state().session.prefs().dialogs[&format!("plugin.presets.{ID}")].as_object().unwrap().is_empty());
    assert_eq!(h.state().session.active().unwrap().revision, revision, "preset operations never commit pixels");
}

#[test]
fn preset_and_switches_survive_cancel_and_reload_without_saving_drafts() {
    let mut h = harness(egui::vec2(1280.0, 1000.0), crate::theme::ThemeKind::ALL[0]);
    choose(&mut h, "Clean");
    h.get_by_label("Skin Only").click();
    h.run_steps(3);
    h.get_by_label("Preview").click();
    h.run_steps(3);
    h.state_mut().ui.dialogs[0].fields.insert("fine".into(), json!(11));
    h.run_steps(3);
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    let mut restored = app();
    restored.session.load_prefs_json(&h.state().session.prefs_to_json()).unwrap();
    crate::menus::invoke(&mut restored, &h.ctx, MENU, json!({})).unwrap();
    let fields = &restored.ui.dialogs[0].fields;
    assert_eq!(fields["__pluginPreset"], "builtin:clean");
    assert_eq!(fields["fine"], 40.0, "the preset is restored, not the unsaved draft");
    assert_eq!(fields["skinOnly"], false);
    assert_eq!(fields["__preview"], true);
}

#[test]
fn duplicate_names_and_orphaned_save_dialogs_do_not_overwrite_presets() {
    let mut h = harness(egui::vec2(1280.0, 800.0), crate::theme::ThemeKind::ALL[0]);
    h.state_mut().run("plugin.preset.save", json!({"id":ID,"name":"Portrait","params":{"fine":13}})).unwrap();
    h.get_by_label("New…").click();
    h.run_steps(12);
    let id = h.state().ui.dialogs.last().unwrap().id;
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("name".into(), json!(" Portrait "));
    assert!(super::super::confirm(h.state_mut(), id).is_err());
    assert_eq!(h.state().ui.dialogs.len(), 2, "invalid input leaves the name editor open");
    assert_eq!(h.state().session.prefs().dialogs[&format!("plugin.presets.{ID}")]["Portrait"]["fine"], 13.0);
    for name in ["", "\n", "a\nb", &"x".repeat(65)] {
        let mut f = h.state().ui.dialogs.last().unwrap().fields.clone();
        f.insert("name".into(), json!(name));
        assert!(validate_new(h.state(), &f).is_err());
    }
    h.state_mut().ui.dialogs.remove(0);
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("name".into(), json!("New portrait"));
    assert!(super::super::confirm(h.state_mut(), id).is_err());
}

#[test]
fn missing_saved_selection_falls_back_and_saved_selection_keeps_independent_skin_only() {
    let mut app = app();
    app.run("plugin.preset.save", json!({"id":ID,"name":"Natural","params":{"fine":17,"skinOnly":true}})).unwrap();
    app.session
        .prefs
        .edit(|prefs| prefs.dialogs.insert(format!("plugin.choices.{ID}"), json!({"__pluginPreset":"saved:Natural","skinOnly":false,"__preview":true})));
    crate::menus::invoke(&mut app, &egui::Context::default(), MENU, json!({})).unwrap();
    assert_eq!(app.ui.dialogs[0].fields["fine"], 17.0);
    assert_eq!(app.ui.dialogs[0].fields["skinOnly"], false);
    assert_eq!(app.ui.dialogs[0].fields["__pluginPreset"], "saved:Natural", "same label as a built-in is unambiguous");
    app.ui.dialogs.clear();
    app.run("plugin.preset.remove", json!({"id":ID,"name":"Natural"})).unwrap();
    crate::menus::invoke(&mut app, &egui::Context::default(), MENU, json!({})).unwrap();
    assert_eq!(app.ui.dialogs[0].fields["__pluginPreset"], "builtin:natural");
    assert_eq!(app.ui.dialogs[0].fields["fine"], 45.0);
    assert_eq!(app.ui.dialogs[0].fields["skinOnly"], false);
}

#[test]
fn preset_header_and_new_dialog_fit_all_themes_and_short_windows() {
    for theme in crate::theme::ThemeKind::ALL {
        for size in [egui::vec2(800.0, 480.0), egui::vec2(1280.0, 800.0)] {
            let mut h = harness(size, theme);
            let bounds = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
            for label in ["Skin Smooth", "New…", "Delete", "Reset preset", "Preview", "OK", "Cancel"] {
                let rect = h.get_by_label(label).rect();
                assert!(bounds.contains_rect(rect), "{theme:?} {size:?}: {label} {rect:?}");
            }
            assert!(h.get_by_label("New…").rect().bottom() < h.query_all_by_label("Fine").next().unwrap().rect().top());
            let long_name = "W".repeat(64);
            h.state_mut().run("plugin.preset.save", json!({"id":ID,"name":long_name,"params":{"fine":27}})).unwrap();
            choose(&mut h, &long_name);
            for label in ["New…", "Delete", "Reset preset"] {
                assert!(bounds.contains_rect(h.get_by_label(label).rect()), "long preset name must not push actions out of view");
            }
            h.get_by_label("New…").click();
            h.run_steps(12);
            for label in ["New Preset", "Save"] {
                assert!(bounds.contains_rect(h.get_by_label(label).rect()));
            }
        }
    }
}
