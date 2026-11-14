//! Define Pattern (#14302) and Define Brush Preset (#14279) refuse an empty or blank name at the
//! dialog; the programmatic commands keep their default-name fallbacks.

use serde_json::{Value, json};

use crate::{PhotocraftApp, Services, state::DialogKind};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width": 16, "height": 16, "name": "doc.jpg"})).unwrap();
    app.run("edit.fill", json!({"color": "#000000"})).unwrap();
    app.run("select.rect", json!({"x": 2, "y": 2, "width": 10, "height": 10})).unwrap();
    app
}

fn open(app: &mut PhotocraftApp, command: &str) -> u64 {
    let ctx = egui::Context::default();
    crate::menus::invoke(app, &ctx, command, Value::Null).unwrap()["dialog"].as_u64().unwrap()
}

fn set_name(app: &mut PhotocraftApp, id: u64, name: Option<&str>) {
    let fields = &mut app.ui.dialog_mut(id).unwrap().fields;
    match name {
        Some(n) => fields.insert("name".into(), json!(n)),
        None => fields.remove("name"),
    };
}

fn still_open(app: &PhotocraftApp, id: u64) -> bool {
    app.ui.dialogs.iter().any(|d| d.id == id)
}

#[test]
fn empty_or_blank_pattern_name_alerts_and_keeps_the_dialog() {
    for name in [None, Some(""), Some("   ")] {
        let mut app = app();
        let id = open(&mut app, "edit.definePattern");
        set_name(&mut app, id, name);
        let before = app.session.patterns.items.len();
        assert!(crate::dialogs::confirm(&mut app, id).is_err(), "{name:?}");
        assert!(still_open(&app, id), "{name:?}: dialog stays open");
        let alert = app.ui.dialogs.last().unwrap();
        assert_eq!(alert.kind, DialogKind::Error);
        assert_eq!(alert.fields["message"], "You must specify a name for the pattern.");
        assert_eq!(crate::dialogs::title(alert), "Pattern Name");
        assert_eq!(app.session.patterns.items.len(), before, "{name:?}: nothing defined");
    }
}

#[test]
fn named_pattern_still_defines() {
    let mut app = app();
    let id = open(&mut app, "edit.definePattern");
    set_name(&mut app, id, Some("Bricks"));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert!(app.session.patterns.items.iter().any(|p| p.name == "Bricks"));
    assert!(!still_open(&app, id));
}

#[test]
fn programmatic_define_pattern_keeps_the_default_name() {
    let mut app = app();
    let before = app.session.patterns.items.len();
    app.run("edit.definePattern", json!({})).unwrap();
    assert_eq!(app.session.patterns.items.len(), before + 1);
    assert!(app.session.patterns.items.last().is_some_and(|p| !p.name.trim().is_empty()));
}

#[test]
fn empty_or_blank_brush_preset_name_does_nothing_and_keeps_the_dialog() {
    for name in [None, Some(""), Some("  \t ")] {
        let mut app = app();
        let id = open(&mut app, "edit.defineBrushPreset");
        set_name(&mut app, id, name);
        let before = app.session.tools.presets.len();
        assert!(crate::dialogs::confirm(&mut app, id).is_err(), "{name:?}");
        assert!(still_open(&app, id), "{name:?}: dialog stays open");
        assert!(app.ui.dialogs.iter().all(|d| d.kind != DialogKind::Error), "{name:?}: no alert");
        assert_eq!(app.session.tools.presets.len(), before, "{name:?}: no preset");
    }
}

#[test]
fn named_brush_preset_defines_and_a_long_name_is_accepted() {
    let mut app = app();
    let long = "n".repeat(300);
    for name in ["Chalk", long.as_str()] {
        let id = open(&mut app, "edit.defineBrushPreset");
        set_name(&mut app, id, Some(name));
        crate::dialogs::confirm(&mut app, id).unwrap();
        assert!(app.session.tools.presets.iter().any(|b| b.name == name), "{} chars", name.len());
    }
}

#[test]
fn programmatic_define_brush_preset_keeps_the_default_name() {
    let mut app = app();
    app.run("edit.defineBrushPreset", json!({})).unwrap();
    assert!(app.session.tools.presets.iter().any(|b| b.name == "Sampled Brush 1"));
}
