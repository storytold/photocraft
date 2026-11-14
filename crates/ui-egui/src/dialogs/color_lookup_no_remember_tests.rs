//! #14422: Image › Adjustments › Color Lookup opens at its defaults every time.
//!
//! A schema dialog normally reopens with what its last OK kept in the preferences
//! (`filter_dialog::remember` → `filter_dialog::restore_remembered`). The reference application's
//! Color Lookup does not: after an OK with a built-in look or a LUT file it opens again with no LUT
//! chosen and Dither at its default. What stays as today is the prefill a recorded or scripted
//! Action step carries, and every other dialog's own memory.

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::dialogs::filter_dialog::{open, remember, reset_fields};

const COLOR_LOOKUP: &str = "image.adjustments.colorLookup";

/// The fields Color Lookup opens with: no 3D LUT (`lut` = none, no `file`), Dither off (the
/// schema default; #14419 is the item that changes it) and trilinear interpolation.
fn assert_defaults(fields: &Map<String, Value>, opened: &str) {
    assert_eq!(fields["lut"], json!("none"), "{opened}: lut");
    assert_eq!(fields["file"], json!(""), "{opened}: file");
    assert_eq!(fields["dither"], json!(false), "{opened}: dither");
    assert_eq!(fields["interpolation"], json!("trilinear"), "{opened}: interpolation");
}

/// An 8×8 RGB document, so `image.adjustments.colorLookup` has something to adjust.
fn app_with_doc() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8, "background": "#000000"})).unwrap();
    app
}

/// Set the dialog's fields and confirm it, as clicking OK does.
fn ok_with(app: &mut PhotocraftApp, id: u64, changed: Value) {
    let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
    fields.extend(changed.as_object().unwrap().clone());
    app.ui.dialog_mut(id).unwrap().fields = fields;
    crate::dialogs::confirm(app, id).unwrap();
}

/// Play a one-step action whose step shows its dialog, and return that dialog's id.
fn step_dialog(app: &mut PhotocraftApp, params: Value) -> u64 {
    use photocraft_engine::actions_cmds::Action;
    let mut action = Action::new("Color lookup step", vec![(COLOR_LOOKUP.into(), params)]);
    action.dialogs.insert(0);
    app.session.actions.list.push(action);
    let played = crate::panels::actions::play(app, &json!({"action": "Color lookup step"})).unwrap();
    played["paused"]["dialog"].as_u64().expect("the toggled step opens its dialog")
}

/// The measured case: OK with a built-in look, and the next opening shows no LUT again instead of
/// that look. Dither and Interpolation are not the LUT and keep today's behaviour: they come back
/// as they were OK'd, and the preferences keep no LUT choice.
#[test]
fn color_lookup_ok_with_a_builtin_look_reopens_with_no_lut() {
    let mut app = app_with_doc();
    let id = open(&mut app, COLOR_LOOKUP).unwrap();
    assert_defaults(&app.ui.dialog_mut(id).unwrap().fields, "the first opening");
    ok_with(&mut app, id, json!({"lut": "warm", "dither": true, "interpolation": "tetrahedral"}));

    let id = open(&mut app, COLOR_LOOKUP).unwrap();
    let fields = &app.ui.dialog_mut(id).unwrap().fields;
    assert_eq!(fields["lut"], json!("none"), "after OK with the built-in look warm: lut");
    assert_eq!(fields["file"], json!(""), "after OK with the built-in look warm: file");
    assert_eq!(fields["dither"], json!(true), "Dither keeps its remembered value");
    assert_eq!(fields["interpolation"], json!("tetrahedral"), "Interpolation keeps its remembered value");
    let saved = app.session.prefs().dialogs.get(COLOR_LOOKUP).and_then(Value::as_object).cloned().unwrap_or_default();
    assert!(!saved.contains_key("lut") && !saved.contains_key("file"), "no LUT choice is kept: {saved:?}");
}

/// The other measured opening: a LUT file loaded and OK'd. The next opening shows no file and no
/// built-in look.
#[test]
fn color_lookup_ok_with_a_lut_file_reopens_at_the_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("film.cube");
    std::fs::write(&path, "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n").unwrap();
    let mut app = app_with_doc();
    let id = open(&mut app, COLOR_LOOKUP).unwrap();
    ok_with(&mut app, id, json!({"file": path.to_str().unwrap()}));

    let id = open(&mut app, COLOR_LOOKUP).unwrap();
    assert_defaults(&app.ui.dialog_mut(id).unwrap().fields, "after OK with a LUT file");
    assert!(app.ui.dialog_mut(id).unwrap().fields.get("__folderLut").is_none(), "the LUT folder pick is not kept either");
}

/// A scripted step with no values of its own and its dialog shown opens at the defaults, even
/// after a menu OK had chosen a look.
#[test]
fn color_lookup_step_dialog_without_recorded_values_opens_at_the_defaults() {
    let mut app = app_with_doc();
    let id = open(&mut app, COLOR_LOOKUP).unwrap();
    ok_with(&mut app, id, json!({"lut": "cool"}));

    let id = step_dialog(&mut app, json!({}));
    assert_defaults(&app.ui.dialog_mut(id).unwrap().fields, "the scripted step with no values");
}

/// The prefill a step carries is not the remembered-dialog mechanism: a recorded look still shows
/// up in its dialog.
#[test]
fn color_lookup_step_dialog_still_prefills_its_recorded_values() {
    let mut app = app_with_doc();
    let id = step_dialog(&mut app, json!({"lut": "dayForNight", "dither": true, "interpolation": "tetrahedral"}));
    let fields = &app.ui.dialog_mut(id).unwrap().fields;
    assert_eq!(
        (&fields["lut"], &fields["dither"], &fields["interpolation"]),
        (&json!("dayForNight"), &json!(true), &json!("tetrahedral")),
        "the step's own values still prefill"
    );
}

/// Alt+Cancel (Reset) returns the changed fields to the defaults, and that OK keeps nothing for
/// the opening after it.
#[test]
fn color_lookup_alt_reset_returns_to_the_defaults() {
    let mut app = app_with_doc();
    let id = open(&mut app, COLOR_LOOKUP).unwrap();
    let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
    fields.insert("lut".into(), json!("dayForNight"));
    fields.insert("file".into(), json!("night.cube"));
    fields.insert("dither".into(), json!(true));
    fields.insert("interpolation".into(), json!("tetrahedral"));
    reset_fields(&app, &mut fields);
    assert_defaults(&fields, "Alt+Cancel (Reset)");
    assert!(fields.contains_key("__lutFolder"), "the dialog's own chrome stays");
    app.ui.dialog_mut(id).unwrap().fields = fields;
    crate::dialogs::confirm(&mut app, id).unwrap();

    let id = open(&mut app, COLOR_LOOKUP).unwrap();
    assert_defaults(&app.ui.dialog_mut(id).unwrap().fields, "the opening after the Reset OK");
}

/// Only Color Lookup stopped remembering: another schema dialog still reopens with what its OK
/// kept.
#[test]
fn other_schema_dialogs_still_remember_their_choices() {
    let mut app = app_with_doc();
    let mut fields = Map::new();
    fields.insert("__filter".into(), json!(true));
    fields.insert("radius".into(), json!(11.0));
    remember(&mut app, "filter.blur.gaussianBlur", &fields);
    let id = open(&mut app, "filter.blur.gaussianBlur").unwrap();
    assert_eq!(app.ui.dialog_mut(id).unwrap().fields["radius"], json!(11.0), "the blur dialog still remembers");
}
