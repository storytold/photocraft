//! #14422: Image › Adjustments › Color Lookup opens at its defaults, not at the last LUT.
//!
//! A hosted parameter dialog remembers its rows: reopening starts from the values of the last OK
//! (`crate::dialogs::params::open_param` remembers under `memory_scope`). Color Lookup's LUT rows
//! are exempt: the picker is a `Param::custom` row and its keys (`lut`, `file`, `data`, `fileName`)
//! are never kept, so an OK with a built-in look or a LUT file leaves the next opening at "no LUT",
//! as the reference application does. Dither and Interpolation are remembered like any schema
//! dialog's rows. A recorded or scripted step still opens on its own values
//! (`panels/actions/runner.rs`, pinned by its own tests).

use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::dialogs::filter_dialog::open;

const COLOR_LOOKUP: &str = "image.adjustments.colorLookup";
const GAUSSIAN_BLUR: &str = "filter.blur.gaussianBlur";

fn new_app() -> Box<PhotocraftApp> {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

/// An 8x8 RGB document, so `image.adjustments.colorLookup` has something to adjust.
fn app_with_doc() -> Box<PhotocraftApp> {
    let mut app = new_app();
    app.run("file.new", json!({"width": 8, "height": 8, "background": "#000000"})).unwrap();
    app
}

fn fields_of(app: &PhotocraftApp, id: DialogId) -> DialogFields {
    app.framework.dialogs.get_by_id::<HostedParamDialog>(id).expect("open dialog").fields().clone()
}

fn field_json(app: &PhotocraftApp, id: DialogId, key: &str) -> Value {
    serde_json::to_value(fields_of(app, id)).expect("fields serialize").get(key).cloned().unwrap_or(Value::Null)
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

fn is_open(app: &PhotocraftApp, id: DialogId) -> bool {
    app.framework.dialogs.get_by_id::<HostedParamDialog>(id).is_some()
}

/// OK, then the passes in which the family runs. True when the dialog closed.
fn ok(app: &mut PhotocraftApp, id: DialogId) -> bool {
    let ctx = app.ctx().clone();
    let _ = crate::dialogs::confirm(app, &ctx, id);
    passes(app, 3);
    !is_open(app, id)
}

/// The keys that carry a LUT: each opens at its default (no value, or the spec's empty/`none`
/// default) - never at the look or file of a past OK (#14422).
fn assert_no_lut(app: &PhotocraftApp, id: DialogId, opened: &str) {
    for (key, defaults) in [("lut", vec![json!("none")]), ("file", vec![json!("")]), ("data", vec![json!("")]), ("fileName", vec![json!("")])] {
        let value = field_json(app, id, key);
        assert!(value.is_null() || defaults.contains(&value), "{opened}: {key} holds {value}");
    }
}

/// The first opening shows no LUT and the schema defaults for the other rows.
#[test]
fn color_lookup_opens_with_no_lut_and_its_defaults() {
    let mut app = app_with_doc();
    let id = open(&mut app, COLOR_LOOKUP).unwrap();
    assert_no_lut(&app, id, "the first opening");
    assert_eq!(field_json(&app, id, "dither"), json!(false), "the first opening: dither");
    assert_eq!(field_json(&app, id, "interpolation"), json!("trilinear"), "the first opening: interpolation");
    cancel(&mut app, id);
}

fn cancel(app: &mut PhotocraftApp, id: DialogId) {
    let ctx = app.ctx().clone();
    app.framework.dialogs.cancel_by_id(&ctx, id);
    passes(app, 2);
}

/// The measured case: OK with a built-in look, and the next opening shows no LUT again instead of
/// that look. Dither and Interpolation are not the LUT and keep the remembering behaviour: they
/// come back as they were OK'd.
#[test]
fn color_lookup_ok_with_a_builtin_look_reopens_with_no_lut() {
    let mut app = app_with_doc();
    let id = open(&mut app, COLOR_LOOKUP).unwrap();
    set(&mut app, id, "lut", json!("warm"));
    set(&mut app, id, "dither", json!(true));
    set(&mut app, id, "interpolation", json!("tetrahedral"));
    assert!(ok(&mut app, id), "OK with the built-in look warm closes the dialog");

    let id = open(&mut app, COLOR_LOOKUP).unwrap();
    assert_no_lut(&app, id, "after OK with the built-in look warm");
    assert_eq!(field_json(&app, id, "dither"), json!(true), "Dither keeps its remembered value");
    assert_eq!(field_json(&app, id, "interpolation"), json!("tetrahedral"), "Interpolation keeps its remembered value");
    assert!(!app.session.prefs().dialogs.contains_key(COLOR_LOOKUP), "the LUT memory no longer lives in the preferences");
}

/// The other measured opening: a LUT file picked and OK'd. The next opening shows no file and no
/// built-in look.
#[test]
fn color_lookup_ok_with_a_lut_file_reopens_at_the_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("film.cube");
    std::fs::write(&path, "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n").unwrap();
    let mut app = app_with_doc();
    let id = open(&mut app, COLOR_LOOKUP).unwrap();
    set(&mut app, id, "file", json!(path.to_str().unwrap()));
    assert!(ok(&mut app, id), "OK with the LUT file closes the dialog");

    let id = open(&mut app, COLOR_LOOKUP).unwrap();
    assert_no_lut(&app, id, "after OK with a LUT file");
}

/// Preferences written before the framework memory exist (#14422 held a saved LUT choice): the
/// one-time move takes the entry out of `Preferences.dialogs`, and the dialog still opens at its
/// defaults - a legacy saved look never comes back.
#[test]
fn color_lookup_legacy_saved_preferences_never_restore_a_lut() {
    let mut app = app_with_doc();
    app.session.edit_prefs(|prefs| {
        prefs.dialogs.insert(COLOR_LOOKUP.into(), json!({"lut": "warm", "file": "old.cube", "dither": true, "interpolation": "tetrahedral"}))
    });
    app.framework.move_memories(&mut app.session);
    assert!(!app.session.prefs().dialogs.contains_key(COLOR_LOOKUP), "the entry was moved out of the preferences");

    let id = open(&mut app, COLOR_LOOKUP).unwrap();
    assert_no_lut(&app, id, "with a legacy saved look in the preferences");
    // The move routes `image.adjustments.*` entries to the adjustment panel's memory, not the
    // dialog's scope, so the other rows start at their defaults as well on this first opening.
    assert_eq!(field_json(&app, id, "dither"), json!(false), "a legacy saved Dither does not reach the dialog");
    assert_eq!(field_json(&app, id, "interpolation"), json!("trilinear"), "a legacy saved Interpolation does not reach the dialog");
}

/// Only the LUT row stopped being remembered: another schema dialog still reopens with what its OK
/// kept (#14422 must not reach past Color Lookup).
#[test]
fn other_schema_dialogs_still_remember_their_choices() {
    let mut app = app_with_doc();
    let id = open(&mut app, GAUSSIAN_BLUR).unwrap();
    set(&mut app, id, "radius", json!(11.0));
    assert!(ok(&mut app, id), "OK on Gaussian Blur closes the dialog");
    let id = open(&mut app, GAUSSIAN_BLUR).unwrap();
    assert_eq!(field_json(&app, id, "radius"), json!(11.0), "the blur dialog still remembers");
}
