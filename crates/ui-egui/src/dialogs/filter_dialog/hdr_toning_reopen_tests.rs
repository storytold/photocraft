//! #14737: after OK, the next HDR Toning dialog opens at its factory defaults, as in the reference
//! application, and values an earlier version remembered for it are ignored. Other schema dialogs
//! still reopen with their last values.

use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog};
use craft::ui::runtime::memory::{SaveKey, set_saved_value};
use serde_json::{Value, json};

use crate::PhotocraftApp;

const HDR: &str = "image.adjustments.hdrToning";
const KEYS: [&str; 3] = ["radius", "strength", "saturation"];

fn new_app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app
}

fn fields_of(app: &PhotocraftApp, id: DialogId) -> DialogFields {
    app.framework.dialogs.get_by_id::<HostedParamDialog>(id).expect("open dialog").fields().clone()
}

fn set(app: &mut PhotocraftApp, id: DialogId, key: &str, value: Value) {
    let ctx = app.ctx().clone();
    app.framework.dialogs.set_json_field(&ctx, id, key, value).unwrap_or_else(|e| panic!("{key}: {e}"));
}

fn passes(app: &mut PhotocraftApp, n: usize) {
    let ctx = app.ctx().clone();
    for _ in 0..n {
        crate::testing::run_ui(&ctx, Default::default(), |ui| crate::dialogs::run(app, ui));
    }
}

/// OK, then the passes in which the command runs; asserts that it applied.
fn ok(app: &mut PhotocraftApp, id: DialogId) {
    let before = app.session.active().unwrap().history.past_len();
    let ctx = app.ctx().clone();
    let _ = crate::dialogs::confirm(app, &ctx, id);
    passes(app, 3);
    assert!(app.framework.dialogs.get_by_id::<HostedParamDialog>(id).is_none(), "OK closed the dialog");
    assert_eq!(app.session.active().unwrap().history.past_len(), before + 1, "OK applied");
}

fn open(app: &mut PhotocraftApp, command: &str) -> DialogId {
    let id = super::open(app, command).unwrap();
    passes(app, 2);
    id
}

fn values(app: &PhotocraftApp, id: DialogId) -> Vec<Option<f64>> {
    let f = fields_of(app, id);
    KEYS.iter().map(|k| f.number(k).or_else(|| f.int(k).map(|i| i as f64))).collect()
}

fn cancel(app: &mut PhotocraftApp, id: DialogId) {
    let ctx = app.ctx().clone();
    app.framework.dialogs.cancel_by_id(&ctx, id);
    passes(app, 2);
}

#[test]
fn hdr_toning_reopens_at_its_defaults_after_ok() {
    let mut app = new_app();
    let id = open(&mut app, HDR);
    let defaults = values(&app, id);
    for (key, value) in KEYS.into_iter().zip([json!(50), json!(2.0), json!(-30)]) {
        set(&mut app, id, key, value);
    }
    assert_eq!(values(&app, id), vec![Some(50.0), Some(2.0), Some(-30.0)], "the dialog holds the new values before OK");
    ok(&mut app, id);
    let again = open(&mut app, HDR);
    assert_eq!(values(&app, again), defaults);
}

#[test]
fn values_remembered_by_an_earlier_version_are_ignored() {
    let mut app = new_app();
    let id = open(&mut app, HDR);
    let defaults = values(&app, id);
    cancel(&mut app, id);
    let key = SaveKey::named(crate::dialogs::params::memory_scope(HDR));
    let mut old = DialogFields::new();
    old.set("radius", FieldValue::Number(50.0));
    old.set("strength", FieldValue::Number(2.0));
    old.set("saturation", FieldValue::Number(-30.0));
    set_saved_value::<Option<DialogFields>>(app.ctx(), &key, Some(old));
    let again = open(&mut app, HDR);
    assert_eq!(values(&app, again), defaults);
}

#[test]
fn other_dialogs_still_reopen_with_their_last_values() {
    let mut app = new_app();
    let id = open(&mut app, "filter.blur.gaussianBlur");
    set(&mut app, id, "radius", json!(3.0));
    ok(&mut app, id);
    let again = open(&mut app, "filter.blur.gaussianBlur");
    assert_eq!(fields_of(&app, again).number("radius"), Some(3.0));
}
