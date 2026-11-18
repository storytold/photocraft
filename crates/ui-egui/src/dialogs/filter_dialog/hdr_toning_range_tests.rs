//! #14720: the HDR Toning dialog opens with the factory Radius 7.68 and Strength 0.5296 unrounded,
//! takes whole numbers in Radius, Detail, Shadow, Highlight, Vibrance and Saturation and two
//! decimals in Strength, Gamma and Exposure, and a typed value past a range shows the reference
//! application's alert and inserts the closest limit. The alert writes the limits the shared form
//! framework's way (no trailing zeros, Radius in px), not the reference's "0.10 and 4.00".

use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use egui::accesskit::Role;
use egui::{Event, Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use crate::PhotocraftApp;

const CMD: &str = "image.adjustments.hdrToning";

type H = Harness<'static, Box<PhotocraftApp>>;

fn open() -> (H, DialogId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    let id = super::open(&mut app, CMD).unwrap();
    let mut h = Harness::builder()
        .with_context(app.ctx().clone())
        .with_size(egui::vec2(1000.0, 900.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h.run_steps(4);
    (h, id)
}

fn field(h: &H, id: DialogId, key: &str) -> Value {
    let fields = h.state().framework.dialogs.get_by_id::<HostedParamDialog>(id).expect("open dialog").fields().clone();
    serde_json::to_value(&fields).unwrap().get(key).cloned().unwrap_or(Value::Null)
}

fn val(h: &H, id: DialogId, key: &str) -> f64 {
    field(h, id, key).as_f64().unwrap_or_else(|| panic!("{key} is a number"))
}

fn alerts(h: &H) -> Vec<String> {
    h.state().framework.dialogs.open_dialogs().iter().filter_map(|d| d.fields.as_ref()?.get("message")?.as_str().map(str::to_string)).collect()
}

fn dismiss_alerts(h: &mut H) {
    while !alerts(h).is_empty() {
        h.key_press(Key::Enter);
        h.run_steps(4);
    }
}

fn focus(h: &mut H, name: &str) {
    h.query_all_by_role(Role::SpinButton)
        .find(|n| n.accesskit_node().label().as_deref().is_some_and(|l| l.trim_end_matches(':') == name))
        .unwrap_or_else(|| panic!("the {name} field"))
        .click();
    h.run_steps(2);
}

/// Types `input` over the field named `label` and leaves it with Tab.
fn type_field(h: &mut H, label: &str, input: &str) {
    focus(h, label);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(Event::Text(input.into()));
    h.run_steps(2);
    h.key_press(Key::Tab);
    h.run_steps(4);
}

#[test]
fn a_fresh_dialog_keeps_the_factory_values_unrounded() {
    // Exactly as the spec gives them, not the widened f32 (7.679999828…) or the shown "8".
    let (h, id) = open();
    for (key, want) in [("radius", 7.68), ("strength", 0.5296), ("gamma", 1.0), ("exposure", 0.0), ("detail", 30.0), ("saturation", 20.0)] {
        assert_eq!(val(&h, id, key), want, "{key}");
    }
}

#[test]
fn out_of_range_values_show_the_alert_and_insert_the_closest_limit() {
    let (mut h, id) = open();
    let cases = [
        ("Radius", "radius", "600", 500.0, "An integer between 1 px and 500 px is required. Closest value inserted."),
        ("Radius", "radius", "0", 1.0, "An integer between 1 px and 500 px is required. Closest value inserted."),
        ("Saturation", "saturation", "-200", -100.0, "An integer between -100 and 100 is required. Closest value inserted."),
        ("Detail", "detail", "400", 300.0, "An integer between -100 and 300 is required. Closest value inserted."),
        ("Gamma", "gamma", "20", 9.99, "A number between 0.01 and 9.99 is required. Closest value inserted."),
        ("Strength", "strength", "0", 0.1, "A number between 0.1 and 4 is required. Closest value inserted."),
        ("Exposure", "exposure", "-9", -5.0, "A number between -5 and 5 is required. Closest value inserted."),
    ];
    for (label, key, typed, limit, alert) in cases {
        type_field(&mut h, label, typed);
        assert_eq!(alerts(&h), vec![alert.to_string()], "{label} {typed}");
        assert!((val(&h, id, key) - limit).abs() < 1e-4, "{label} {typed}: {}", val(&h, id, key));
        dismiss_alerts(&mut h);
    }
}

#[test]
fn whole_number_fields_round_and_decimal_fields_keep_two_places() {
    let (mut h, id) = open();
    for (label, key, typed, want) in
        [("Radius", "radius", "12.6", 13.0), ("Shadow", "shadow", "-4.4", -4.0), ("Strength", "strength", "1.234", 1.23), ("Gamma", "gamma", "2.567", 2.57)]
    {
        type_field(&mut h, label, typed);
        assert_eq!(val(&h, id, key), want, "{label} {typed}");
    }
    assert!(alerts(&h).is_empty(), "in-range values show no alert: {:?}", alerts(&h));
}

#[test]
fn tabbing_through_the_fields_does_not_round_the_factory_values() {
    // Leaving fields that show "8" and "0.53" without typing keeps 7.68 and 0.5296.
    let (mut h, id) = open();
    focus(&mut h, "Radius");
    for _ in 0..12 {
        h.key_press(Key::Tab);
        h.run_steps(2);
    }
    assert_eq!(val(&h, id, "radius"), 7.68);
    assert_eq!(val(&h, id, "strength"), 0.5296);
    assert!(alerts(&h).is_empty(), "{:?}", alerts(&h));
}

#[test]
fn a_typed_number_sets_it() {
    let (mut h, id) = open();
    type_field(&mut h, "Radius", "9");
    assert_eq!(val(&h, id, "radius"), 9.0);
    type_field(&mut h, "Strength", "0.6");
    assert_eq!(val(&h, id, "strength"), 0.6);
}

#[test]
fn text_that_does_not_parse_keeps_the_value() {
    let (mut h, id) = open();
    type_field(&mut h, "Radius", "x");
    assert_eq!(val(&h, id, "radius"), 7.68);
    assert!(alerts(&h).is_empty(), "{:?}", alerts(&h));
}

#[test]
fn arrow_keys_step_the_decimal_fields_by_a_hundredth() {
    let (mut h, id) = open();
    focus(&mut h, "Gamma");
    h.key_press(Key::ArrowUp);
    h.run_steps(2);
    h.key_press(Key::Tab);
    h.run_steps(4);
    assert_eq!(val(&h, id, "gamma"), 1.01, "stored as shown, not the widened f32");
}

#[test]
fn an_ime_commit_counts_as_typing() {
    let (mut h, id) = open();
    focus(&mut h, "Radius");
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(Event::Ime(egui::ImeEvent::Commit("9".into())));
    h.run_steps(2);
    h.key_press(Key::Tab);
    h.run_steps(4);
    assert_eq!(val(&h, id, "radius"), 9.0);
}

#[test]
fn each_slider_is_announced_by_its_name() {
    let (h, _) = open();
    for name in ["Radius", "Strength", "Gamma", "Exposure", "Detail", "Shadow", "Highlight", "Vibrance", "Saturation"] {
        assert!(
            h.query_all_by_role(Role::Slider).any(|n| n.accesskit_node().label().as_deref().is_some_and(|l| l.trim_end_matches(':') == name)),
            "{name} slider"
        );
    }
}

#[test]
fn a_value_that_rounds_to_zero_is_stored_as_zero() {
    let (mut h, id) = open();
    type_field(&mut h, "Exposure", "-0.004");
    let v = val(&h, id, "exposure");
    assert!(v == 0.0 && v.is_sign_positive(), "{v}");
}

#[test]
fn typing_key_by_key_ends_at_the_typed_number() {
    let (mut h, id) = open();
    focus(&mut h, "Strength");
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    for c in ["0", ".", "7", "3"] {
        h.event(Event::Text(c.into()));
        h.run_steps(1);
    }
    h.key_press(Key::Tab);
    h.run_steps(4);
    assert_eq!(val(&h, id, "strength"), 0.73);
    assert!(alerts(&h).is_empty(), "{:?}", alerts(&h));
}

#[test]
fn ok_applies_the_inserted_limits() {
    let (mut h, id) = open();
    type_field(&mut h, "Gamma", "20");
    dismiss_alerts(&mut h);
    type_field(&mut h, "Radius", "900");
    dismiss_alerts(&mut h);
    let before = h.state().session.active().unwrap().history.past_len();
    let ctx = h.ctx.clone();
    let _ = crate::dialogs::confirm(h.state_mut(), &ctx, id);
    h.run_steps(4);
    assert!(h.state().framework.dialogs.get_by_id::<HostedParamDialog>(id).is_none(), "OK closed the dialog");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), before + 1, "OK applied Gamma 9.99 and Radius 500");
}
