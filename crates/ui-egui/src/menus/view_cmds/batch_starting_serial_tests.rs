//! File › Automate › Batch: Starting Serial # takes whole numbers 0..9999 (#13662). A value typed
//! past them becomes the closest one and, once the field is left, the reference's "Closest value
//! inserted" alert says so; letters are not taken.

use craft::ui::dialog::{DialogId, HostedAlert, StackDialog};
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use crate::PhotocraftApp;

const BATCH: &str = "file.automate.batch";
const FIELD: &str = "Starting Serial Number";
/// The reference's alert for a Starting Serial # typed past 0..9999.
const ALERT: &str = "An integer between 0 and 9999 is required. Closest value inserted.";

type H = Harness<'static, Box<PhotocraftApp>>;

/// The Batch dialog, opened from its menu command over a document with one recorded action.
fn batch() -> (H, DialogId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    app.run("actions.record", json!({"name": "Invert"})).unwrap();
    app.run("image.adjustments.invert", json!({})).unwrap();
    app.run("actions.stop", json!({})).unwrap();
    app.sync_views();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1100.0, 900.0))
        .with_max_steps(64)
        .with_context(app.ctx().clone())
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    let ctx = h.ctx.clone();
    let reply = crate::menus::invoke(h.state_mut(), &ctx, BATCH, json!({})).expect("the Batch dialog opens");
    let want = reply["dialog"].as_str().expect("a hosted dialog").to_owned();
    let dialog = h.state().framework.dialogs.open_dialogs().into_iter().map(|d| d.id).find(|d| d.value().to_string() == want).unwrap();
    h.run_steps(4);
    (h, dialog)
}

/// Replaces the field's text with `text`, leaving the field focused.
fn type_serial(h: &mut H, text: &str) {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some(FIELD)).expect("the Starting Serial field").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
}

/// Types `text`, then leaves the field with Tab (Enter would also answer the dialog).
fn enter_serial(h: &mut H, text: &str) {
    type_serial(h, text);
    h.key_press(egui::Key::Tab);
    h.run_steps(4);
}

/// What OK would send for Starting Serial #; `None` while the Batch dialog is closed.
fn serial(h: &H, dialog: DialogId) -> Option<Value> {
    let form = h.state().framework.dialogs.get_by_id::<HostedParamDialog>(dialog)?;
    Some(crate::filter_dialog::command_params(BATCH, form.fields())["startingSerial"].clone())
}

/// The messages of the alerts showing.
fn alerts(h: &H) -> Vec<String> {
    let dialogs = &h.state().framework.dialogs;
    dialogs
        .stack()
        .ids()
        .filter_map(|id| dialogs.get_by_id::<HostedAlert>(id))
        .filter_map(|alert| alert.json_fields()?.get("message")?.as_str().map(String::from))
        .collect()
}

/// OK on the alert.
fn ok(h: &mut H) {
    let id = *crate::dialogs::alerts::alerts(h.state()).first().expect("an alert");
    crate::dialogs::testing::confirm(h.state_mut(), id).unwrap();
    h.run_steps(4);
}

#[test]
fn starting_serial_defaults_to_one() {
    let (h, dialog) = batch();
    assert_eq!(serial(&h, dialog), Some(json!(1)));
    assert!(alerts(&h).is_empty());
}

#[test]
fn a_starting_serial_past_the_range_takes_the_closest_value_with_the_alert() {
    for (typed, want) in [("12345", 9999), ("10000", 9999), ("-5", 0), ("-1", 0)] {
        let (mut h, dialog) = batch();
        enter_serial(&mut h, typed);
        assert_eq!(serial(&h, dialog), Some(json!(want)), "{typed}");
        assert_eq!(alerts(&h), vec![ALERT.to_string()], "{typed}: one alert");
        ok(&mut h);
        assert!(alerts(&h).is_empty(), "{typed}: OK closes the alert");
        assert_eq!(serial(&h, dialog), Some(json!(want)), "{typed}: the closest value is kept");
    }
}

#[test]
fn starting_serials_from_zero_to_9999_are_kept_without_an_alert() {
    for (typed, want) in [("0", 0), ("1", 1), ("1000", 1000), ("9999", 9999)] {
        let (mut h, dialog) = batch();
        enter_serial(&mut h, typed);
        assert_eq!(serial(&h, dialog), Some(json!(want)), "{typed}");
        assert!(alerts(&h).is_empty(), "{typed} is in range");
    }
}

#[test]
fn letters_are_not_taken() {
    let (mut h, dialog) = batch();
    enter_serial(&mut h, "42");
    enter_serial(&mut h, "abc");
    assert_eq!(serial(&h, dialog), Some(json!(42)), "the previous value stays");
    assert!(alerts(&h).is_empty(), "no number, no alert");
}
