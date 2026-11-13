//! File › Automate › Batch: Starting Serial # takes whole numbers 0..9999 (#13662). A value typed
//! past them becomes the closest one and, once the field is left, the reference's "Closest value
//! inserted" alert says so; an emptied field counts as 0, and letters are not taken.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::STARTING_SERIAL_ALERT;
use crate::PhotocraftApp;
use crate::state::DialogKind;

const FIELD: &str = "Starting serial";

/// The Batch dialog, opened from its menu command over a document with one recorded action.
fn batch() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    app.run("actions.record", json!({"name": "Invert"})).unwrap();
    app.run("image.adjustments.invert", json!({})).unwrap();
    app.run("actions.stop", json!({})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1100.0, 800.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    // The dialog opens once the app's fonts are in: the harness draws a first frame as it is built.
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.automate.batch", json!({})).expect("the Batch dialog opens");
    h.run_steps(4);
    h
}

/// Replaces the field's text with `text` (empty clears it), leaving the field focused.
fn type_serial(h: &mut Harness<'static, PhotocraftApp>, text: &str) {
    h.get_by_role_and_label(Role::SpinButton, FIELD).click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    if text.is_empty() {
        h.key_press(egui::Key::Backspace);
    } else {
        h.event(egui::Event::Text(text.into()));
    }
    h.run_steps(1);
}

/// Types `text`, then leaves the field by clicking its caption (Enter would also run the dialog).
fn enter_serial(h: &mut Harness<'static, PhotocraftApp>, text: &str) {
    type_serial(h, text);
    h.get_by_role_and_label(Role::Label, FIELD).click();
    h.run_steps(3);
}

/// The Batch dialog's stored Starting Serial #.
fn serial(h: &Harness<'static, PhotocraftApp>) -> serde_json::Value {
    let batch = h.state().ui.dialogs.iter().find(|d| d.fields.get("__command").and_then(|c| c.as_str()) == Some("file.automate.batch"));
    batch.expect("the Batch dialog is open").fields["startingSerial"].clone()
}

/// The messages of the alerts showing.
fn alerts(h: &Harness<'static, PhotocraftApp>) -> Vec<String> {
    h.state().ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| d.fields.get("message")?.as_str().map(str::to_owned)).collect()
}

/// OK on the alert, as `ui.dialog.confirm` presses it (the Batch dialog has an OK button too).
fn ok(h: &mut Harness<'static, PhotocraftApp>) {
    let id = h.state().ui.dialogs.iter().find(|d| d.kind == DialogKind::Error).map(|d| d.id).expect("an alert");
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    h.run_steps(3);
}

#[test]
fn starting_serial_defaults_to_one() {
    let h = batch();
    assert_eq!(serial(&h), json!(1));
    assert!(alerts(&h).is_empty());
}

#[test]
fn a_starting_serial_past_the_range_takes_the_closest_value_with_the_alert() {
    for (typed, want) in [("12345", 9999), ("10000", 9999), ("-5", 0), ("-1", 0)] {
        let mut h = batch();
        enter_serial(&mut h, typed);
        assert_eq!(serial(&h), json!(want), "{typed}");
        assert_eq!(alerts(&h), vec![STARTING_SERIAL_ALERT.to_string()], "{typed}: one alert");
        ok(&mut h);
        assert!(alerts(&h).is_empty(), "{typed}: OK closes the alert");
        assert_eq!(serial(&h), json!(want), "{typed}: the closest value is kept");
    }
}

#[test]
fn starting_serials_from_zero_to_9999_are_kept_without_an_alert() {
    for (typed, want) in [("0", 0), ("1", 1), ("1000", 1000), ("9999", 9999)] {
        let mut h = batch();
        enter_serial(&mut h, typed);
        assert_eq!(serial(&h), json!(want), "{typed}");
        assert!(alerts(&h).is_empty(), "{typed} is in range");
    }
}

#[test]
fn an_emptied_starting_serial_counts_as_zero() {
    let mut h = batch();
    enter_serial(&mut h, "");
    assert_eq!(serial(&h), json!(0));
    assert!(alerts(&h).is_empty(), "an empty field is no value past the range");
}

#[test]
fn letters_are_not_taken() {
    let mut h = batch();
    enter_serial(&mut h, "42");
    enter_serial(&mut h, "abc");
    assert_eq!(serial(&h), json!(42), "the previous value stays");
    assert!(alerts(&h).is_empty(), "no number, no alert");
}

/// Enter in the field shows the alert over the dialog rather than also running the batch with
/// the corrected value; the dialog stays open with 9999 until the user presses OK there.
#[test]
fn enter_after_a_value_past_the_range_shows_the_alert_and_keeps_the_dialog() {
    let mut h = batch();
    type_serial(&mut h, "12345");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(alerts(&h), vec![STARTING_SERIAL_ALERT.to_string()]);
    assert_eq!(serial(&h), json!(9999), "the dialog is still open, with the closest value");
    ok(&mut h);
    assert!(alerts(&h).is_empty());
    assert_eq!(serial(&h), json!(9999), "OK on the alert leaves the Batch dialog open");
}
