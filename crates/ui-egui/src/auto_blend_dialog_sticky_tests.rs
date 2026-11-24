//! #14209: after Auto-Blend Layers runs from an action, a script or the control channel, its dialog reopens with the
//! values that run used, as after a dialog run. A run that omits a value keeps the remembered one; a failed run and
//! Cancel store nothing; other commands' remembered values don't change.

use craft::ui::patterns::form_dialogs::HostedParamDialog;
use serde_json::{Value, json};

use crate::PhotocraftApp;

const AUTO_BLEND: &str = "edit.autoBlendLayers";

/// Two overlapping pixel layers, both selected, so Auto-Blend runs.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 48, "background": "transparent"})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 40, "height": 48})).unwrap();
    app.run("edit.fill", json!({"color": "#cc3311"})).unwrap();
    let first = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 24, "y": 0, "width": 40, "height": 48})).unwrap();
    app.run("edit.fill", json!({"color": "#1133cc"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("layer.select", json!({"layer": first.0, "mode": "add"})).unwrap();
    app
}

/// Opens `command`'s dialog from the menu, with every layer selected so Auto-Blend is available.
fn open(app: &mut PhotocraftApp, command: &str) -> craft::ui::dialog::DialogId {
    app.run("select.allLayers", json!({})).unwrap();
    let ctx = app.ctx().clone();
    let reply = crate::menus::invoke(app, &ctx, command, json!({})).unwrap();
    let want = reply["dialog"].as_str().unwrap().to_owned();
    app.framework.dialogs.open_dialogs().into_iter().map(|d| d.id).find(|d| d.value().to_string() == want).unwrap()
}

/// Opens `command`'s dialog from the menu and returns the values it shows, then cancels it.
fn shown(app: &mut PhotocraftApp, command: &str) -> Value {
    let ctx = app.ctx().clone();
    let id = open(app, command);
    let fields = serde_json::to_value(app.framework.dialogs.get_by_id::<HostedParamDialog>(id).unwrap().fields()).unwrap();
    crate::dialogs::cancel(app, &ctx, id).unwrap();
    fields
}

fn method_and_tones(app: &mut PhotocraftApp) -> (Value, Value) {
    let f = shown(app, AUTO_BLEND);
    (f["method"].clone(), f["seamlessTones"].clone())
}

#[test]
fn a_scripted_run_is_what_the_dialog_reopens_with() {
    let mut app = app();
    app.run(AUTO_BLEND, json!({"method": "stack", "seamlessTones": false})).unwrap();
    assert_eq!(method_and_tones(&mut app), (json!("stack"), json!(false)));
    app.run("history.undo", json!({})).unwrap();
    app.run(AUTO_BLEND, json!({"method": "panorama", "seamlessTones": true})).unwrap();
    assert_eq!(method_and_tones(&mut app), (json!("panorama"), json!(true)));
}

#[test]
fn a_run_that_omits_a_value_keeps_the_remembered_one() {
    let mut app = app();
    app.run(AUTO_BLEND, json!({"method": "stack", "seamlessTones": false})).unwrap();
    app.run("history.undo", json!({})).unwrap();
    app.run(AUTO_BLEND, json!({"seamlessTones": true})).unwrap();
    assert_eq!(method_and_tones(&mut app), (json!("stack"), json!(true)), "only the passed value changed");
}

#[test]
fn a_failed_run_and_cancel_store_nothing() {
    let mut app = app();
    app.run(AUTO_BLEND, json!({"method": "stack", "seamlessTones": false})).unwrap();
    app.run("history.undo", json!({})).unwrap();
    // One layer selected: the run fails.
    let only = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.select", json!({"layer": only.0})).unwrap();
    assert!(app.run(AUTO_BLEND, json!({"method": "panorama", "seamlessTones": true})).is_err());
    assert_eq!(method_and_tones(&mut app), (json!("stack"), json!(false)), "the failed run stored nothing");
    // Changing the values in the dialog, then Cancel, stores nothing either.
    let ctx = app.ctx().clone();
    let id = open(&mut app, AUTO_BLEND);
    app.framework.dialogs.set_json_field(&ctx, id, "method", json!("panorama")).unwrap();
    app.framework.dialogs.set_json_field(&ctx, id, "seamlessTones", json!(true)).unwrap();
    crate::dialogs::cancel(&mut app, &ctx, id).unwrap();
    assert_eq!(method_and_tones(&mut app), (json!("stack"), json!(false)), "Cancel stored nothing");
}

#[test]
fn other_commands_remembered_values_are_unchanged() {
    let mut app = app();
    let before = shown(&mut app, "filter.blur.gaussianBlur");
    assert_ne!(before["radius"], json!(37.5));
    app.run("filter.blur.gaussianBlur", json!({"radius": 37.5})).unwrap();
    assert_eq!(shown(&mut app, "filter.blur.gaussianBlur"), before, "a scripted Gaussian Blur leaves its dialog's values");
}

#[test]
fn a_played_action_is_what_the_dialog_reopens_with() {
    let mut app = app();
    app.run("actions.record", json!({"name": "Blend"})).unwrap();
    app.run(AUTO_BLEND, json!({"method": "stack", "seamlessTones": false})).unwrap();
    app.run("actions.stop", json!({})).unwrap();
    app.run("history.undo", json!({})).unwrap();
    app.run(AUTO_BLEND, json!({"method": "panorama", "seamlessTones": true})).unwrap();
    app.run("history.undo", json!({})).unwrap();
    assert_eq!(method_and_tones(&mut app), (json!("panorama"), json!(true)));
    let list = &app.session.actions.list;
    let blend = (0..64).find(|&i| list.action(i).is_some_and(|a| a.name == "Blend")).unwrap();
    app.run("select.allLayers", json!({})).unwrap();
    crate::panels::actions::runner::play_targets(&mut app, &[blend], 0, 0).unwrap();
    assert_eq!(method_and_tones(&mut app), (json!("stack"), json!(false)), "the action's step is what the dialog shows");
}
