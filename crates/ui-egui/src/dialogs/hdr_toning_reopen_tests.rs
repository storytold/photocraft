//! #14737: after OK, the next HDR Toning dialog opens at its factory defaults, as in the reference
//! application; dialogs that remember their values keep doing so.

use serde_json::{Value, json};

use super::*;
use crate::control::{ControlRequest, handle};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app
}

/// A control request as the control channel sends it.
fn control(app: &mut PhotocraftApp, method: &str, params: Value) {
    let (req, _rx) = ControlRequest::new(method, params);
    handle(app, &egui::Context::default(), &req);
}

/// The fields `command`'s dialog opens with; the dialog is closed again.
fn opened(app: &mut PhotocraftApp, command: &str) -> Map<String, Value> {
    let id = open(app, command).unwrap();
    let fields = app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.clone();
    app.ui.dialogs.retain(|d| d.id != id);
    fields
}

/// Open `command`'s dialog, set `field` to `value` and press OK, as an agent or the user does.
fn ok_with(app: &mut PhotocraftApp, command: &str, field: &str, value: f64) {
    let id = open(app, command).unwrap();
    control(app, "ui.dialog.set", json!({"dialog": id, "field": field, "value": value}));
    let set = app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields[field].as_f64();
    assert_eq!(set, Some(value), "{command}: `{field}` is set before OK");
    control(app, "ui.dialog.confirm", json!({"dialog": id}));
    assert!(app.ui.dialogs.iter().all(|d| d.id != id), "{command}: OK closed the dialog");
}

fn num(fields: &Map<String, Value>, key: &str) -> f64 {
    fields[key].as_f64().unwrap()
}

#[test]
fn hdr_toning_reopens_at_its_defaults_after_ok() {
    let mut app = app();
    assert_eq!(num(&opened(&mut app, HDR_TONING), "radius"), 30.0);
    ok_with(&mut app, HDR_TONING, "radius", 50.0);
    let next = opened(&mut app, HDR_TONING);
    assert_eq!(num(&next, "radius"), 30.0, "Radius back at its default");
    assert_eq!(num(&next, "strength"), 0.5);
    assert_eq!(num(&next, "saturation"), 20.0);
    assert!(!app.session.prefs().dialogs.contains_key(HDR_TONING), "nothing is stored for it");
}

#[test]
fn values_an_earlier_version_remembered_are_ignored() {
    let mut app = app();
    app.session.prefs.edit(|prefs| prefs.dialogs.insert(HDR_TONING.into(), json!({"radius": 50.0, "strength": 2.0})));
    let fields = opened(&mut app, HDR_TONING);
    assert_eq!(num(&fields, "radius"), 30.0);
    assert_eq!(num(&fields, "strength"), 0.5);
}

#[test]
fn other_dialogs_still_remember_their_values() {
    let mut app = app();
    let blur = "filter.blur.gaussianBlur";
    ok_with(&mut app, blur, "radius", 3.0);
    assert_eq!(num(&opened(&mut app, blur), "radius"), 3.0);
}
