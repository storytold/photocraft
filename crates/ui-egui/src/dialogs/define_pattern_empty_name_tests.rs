//! Define Pattern and Define Brush Preset refuse an empty or blank name at the dialog (#14302,
//! #14279); the programmatic commands keep their default-name fallbacks.

use craft::ui::dialog::{DialogId, HostedAlert, StackDialog};
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use serde_json::{Value, json};

use crate::{PhotocraftApp, Services};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width": 16, "height": 16, "name": "doc.jpg"})).unwrap();
    app.run("edit.fill", json!({"color": "#000000"})).unwrap();
    app.run("select.rect", json!({"x": 2, "y": 2, "width": 10, "height": 10})).unwrap();
    *app
}

fn open(app: &mut PhotocraftApp, command: &str) -> DialogId {
    let ctx = app.ctx().clone();
    let reply = crate::menus::invoke(app, &ctx, command, Value::Null).unwrap();
    let want = reply["dialog"].as_str().unwrap().to_owned();
    let id = app.framework.dialogs.open_dialogs().into_iter().map(|d| d.id).find(|d| d.value().to_string() == want).unwrap();
    crate::dialogs::testing::show(app);
    id
}

fn set_name(app: &mut PhotocraftApp, id: DialogId, name: &str) {
    let ctx = app.ctx().clone();
    app.framework.dialogs.set_json_field(&ctx, id, "name", json!(name)).unwrap();
}

fn still_open(app: &PhotocraftApp, id: DialogId) -> bool {
    app.framework.dialogs.get_by_id::<HostedParamDialog>(id).is_some()
}

fn alerts(app: &PhotocraftApp) -> usize {
    app.framework.dialogs.open_dialogs().len() - usize::from(app.framework.dialogs.get::<HostedParamDialog>().is_some())
}

#[test]
fn empty_or_blank_pattern_name_alerts_and_keeps_the_dialog() {
    for name in ["", "   "] {
        let mut app = app();
        let id = open(&mut app, "edit.definePattern");
        set_name(&mut app, id, name);
        let before = app.session.patterns.items.len();
        assert!(crate::dialogs::testing::confirm(&mut app, id).is_err(), "{name:?}");
        assert!(still_open(&app, id), "{name:?}: dialog stays open");
        let alert = app.framework.dialogs.get::<HostedAlert>().expect("an alert is open");
        let fields = alert.json_fields().unwrap();
        assert_eq!(fields["message"], "You must specify a name for the pattern.");
        assert_eq!(app.session.patterns.items.len(), before, "{name:?}: nothing defined");
    }
}

#[test]
fn named_pattern_still_defines() {
    let mut app = app();
    let id = open(&mut app, "edit.definePattern");
    set_name(&mut app, id, "Bricks");
    crate::dialogs::testing::confirm(&mut app, id).unwrap();
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
fn repeated_ok_with_a_blank_name_shows_one_alert() {
    let mut app = app();
    let id = open(&mut app, "edit.definePattern");
    set_name(&mut app, id, "");
    for _ in 0..3 {
        assert!(crate::dialogs::testing::confirm(&mut app, id).is_err());
    }
    assert_eq!(alerts(&app), 1);
}

#[test]
fn empty_or_blank_brush_preset_name_does_nothing_and_keeps_the_dialog() {
    for name in ["", "  \t "] {
        let mut app = app();
        let id = open(&mut app, "edit.defineBrushPreset");
        set_name(&mut app, id, name);
        let before = app.session.tools.presets.len();
        assert!(crate::dialogs::testing::confirm(&mut app, id).is_err(), "{name:?}");
        assert!(still_open(&app, id), "{name:?}: dialog stays open");
        assert_eq!(alerts(&app), 0, "{name:?}: no alert");
        assert_eq!(app.session.tools.presets.len(), before, "{name:?}: no preset");
    }
}

#[test]
fn named_brush_preset_defines_and_a_long_name_is_accepted() {
    let mut app = app();
    let long = "n".repeat(300);
    for name in ["Chalk", long.as_str()] {
        let id = open(&mut app, "edit.defineBrushPreset");
        set_name(&mut app, id, name);
        crate::dialogs::testing::confirm(&mut app, id).unwrap();
        assert!(app.session.tools.presets.iter().any(|b| b.name == name), "{} chars", name.len());
    }
}

#[test]
fn programmatic_define_brush_preset_keeps_the_default_name() {
    let mut app = app();
    app.run("edit.defineBrushPreset", json!({})).unwrap();
    assert!(app.session.tools.presets.iter().any(|b| b.name == "Sampled Brush 1"));
}
