//! #14720: the HDR Toning dialog opens with the factory Radius 7.68 and Strength 0.5296 unrounded,
//! takes whole numbers in Radius, Detail, Shadow, Highlight, Vibrance and Saturation and two
//! decimals in Strength, Gamma and Exposure, and a typed value past a range shows the reference
//! application's alert and inserts the closest limit.

use egui::{Event, Key, Modifiers, accesskit::Role};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use serde_json::{Value, json};

use crate::{PhotocraftApp, Services, dialogs::show, state::DialogKind};

const CMD: &str = "image.adjustments.hdrToning";

fn open() -> (Harness<'static, PhotocraftApp>, u64) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 900.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    let ctx = h.ctx.clone();
    let id = crate::menus::invoke(h.state_mut(), &ctx, CMD, Value::Null).unwrap()["dialog"].as_u64().unwrap();
    h.run_steps(4);
    (h, id)
}

fn type_field(h: &mut Harness<'static, PhotocraftApp>, label: &str, input: &str) {
    let field = h.query_all_by_role(Role::SpinButton).find(|node| node.accesskit_node().label().as_deref() == Some(label)).unwrap();
    field.click();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(Event::Text(input.into()));
    h.run_steps(2);
    h.key_press(Key::Tab);
    h.run_steps(3);
}

fn field_val(h: &Harness<'_, PhotocraftApp>, id: u64, key: &str) -> f64 {
    h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields[key].as_f64().unwrap()
}

fn alerts(h: &Harness<'_, PhotocraftApp>) -> Vec<String> {
    h.state().ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| d.fields.get("message")?.as_str().map(String::from)).collect()
}

fn dismiss_alerts(h: &mut Harness<'static, PhotocraftApp>) {
    let ids: Vec<u64> = h.state().ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).map(|d| d.id).collect();
    for id in ids {
        h.state_mut().ui.close_dialog(id);
    }
    h.run_steps(2);
}

#[test]
fn a_fresh_dialog_keeps_the_factory_values_unrounded() {
    // Exactly as the spec gives them, not the widened f32 (7.679999828…) or the shown "8".
    let (h, id) = open();
    for (key, want) in [("radius", 7.68), ("strength", 0.5296), ("gamma", 1.0), ("exposure", 0.0), ("detail", 30.0), ("saturation", 20.0)] {
        assert_eq!(field_val(&h, id, key), want, "{key}");
    }
}

#[test]
fn out_of_range_values_show_the_alert_and_insert_the_closest_limit() {
    let (mut h, id) = open();
    let cases = [
        ("Radius", "radius", "600", 500.0, "An integer between 1 and 500 is required. Closest value inserted."),
        ("Radius", "radius", "0", 1.0, "An integer between 1 and 500 is required. Closest value inserted."),
        ("Saturation", "saturation", "-200", -100.0, "An integer between -100 and 100 is required. Closest value inserted."),
        ("Detail", "detail", "400", 300.0, "An integer between -100 and 300 is required. Closest value inserted."),
        ("Gamma", "gamma", "20", 9.99, "A number between 0.01 and 9.99 is required. Closest value inserted."),
        ("Strength", "strength", "0", 0.1, "A number between 0.10 and 4.00 is required. Closest value inserted."),
        ("Exposure", "exposure", "-9", -5.0, "A number between -5.00 and 5.00 is required. Closest value inserted."),
    ];
    for (label, key, typed, limit, alert) in cases {
        type_field(&mut h, label, typed);
        assert_eq!(alerts(&h), vec![alert.to_string()], "{label} {typed}");
        assert!((field_val(&h, id, key) - limit).abs() < 1e-4, "{label} {typed}: {}", field_val(&h, id, key));
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
        assert!((field_val(&h, id, key) - want).abs() < 1e-6, "{label} {typed}: {}", field_val(&h, id, key));
    }
    assert!(alerts(&h).is_empty(), "in-range values show no alert");
}

#[test]
fn tabbing_through_the_fields_does_not_round_the_factory_values() {
    // Radius has the keyboard when the dialog opens; leaving fields that show "8" and "0.53"
    // without typing must keep 7.68 and 0.5296.
    let (mut h, id) = open();
    for _ in 0..6 {
        h.key_press(Key::Tab);
        h.run_steps(2);
    }
    assert!((field_val(&h, id, "radius") - 7.68).abs() < 1e-4, "radius {}", field_val(&h, id, "radius"));
    assert!((field_val(&h, id, "strength") - 0.5296).abs() < 1e-4, "strength {}", field_val(&h, id, "strength"));
    assert!(alerts(&h).is_empty());
}

#[test]
fn an_inserted_limit_is_a_value_the_dialog_can_remember() {
    // The stored limit must sit inside the spec's (f32) range, or `remember` drops it.
    let (mut h, id) = open();
    let spec = photocraft_engine::commands::find(CMD).unwrap().params;
    let params = super::parse_spec(spec);
    for (label, key, typed) in [("Gamma", "gamma", "20"), ("Strength", "strength", "0"), ("Exposure", "exposure", "9"), ("Radius", "radius", "900")] {
        type_field(&mut h, label, typed);
        dismiss_alerts(&mut h);
        let value = h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields[key].clone();
        let p = params.iter().find(|p| p.key == key).unwrap();
        assert!(super::rememberable(&p.kind, &value), "{label} {typed} stored {value}");
    }
}

#[test]
fn a_typed_number_sets_it() {
    let (mut h, id) = open();
    type_field(&mut h, "Radius", "9");
    assert_eq!(field_val(&h, id, "radius"), 9.0);
    type_field(&mut h, "Strength", "0.6");
    assert_eq!(field_val(&h, id, "strength"), 0.6);
}

#[test]
fn text_that_does_not_parse_keeps_the_value() {
    let (mut h, id) = open();
    type_field(&mut h, "Radius", "x");
    assert_eq!(field_val(&h, id, "radius"), 7.68);
    assert!(alerts(&h).is_empty());
}

#[test]
fn an_opened_value_past_the_range_is_clamped() {
    let (mut h, id) = open();
    let d = h.state_mut().ui.dialogs.iter_mut().find(|d| d.id == id).unwrap();
    d.fields.insert("radius".into(), json!(600.0));
    d.fields.insert("gamma".into(), json!(50.0));
    h.run_steps(2);
    assert_eq!(field_val(&h, id, "radius"), 500.0);
    assert!((field_val(&h, id, "gamma") - 9.99).abs() < 1e-6);
}

#[test]
fn edited_decimals_are_stored_as_typed() {
    // Not the widened f32 (2.569999933…), which OK, recorded actions and the control channel send.
    let (mut h, id) = open();
    type_field(&mut h, "Gamma", "2.57");
    assert_eq!(field_val(&h, id, "gamma"), 2.57);
    // The inserted limit stays inside the spec's f32 range, so it can be remembered.
    type_field(&mut h, "Gamma", "20");
    dismiss_alerts(&mut h);
    assert!((field_val(&h, id, "gamma") - 9.99).abs() < 1e-6);
    let kind = super::Kind::Range { min: 0.01, max: 9.99, default: 1.0 };
    assert!(super::rememberable(&kind, &json!(field_val(&h, id, "gamma"))));
}

#[test]
fn clearing_a_field_and_leaving_it_keeps_the_value() {
    let (mut h, id) = open();
    let field = h.query_all_by_role(Role::SpinButton).find(|node| node.accesskit_node().label().as_deref() == Some("Radius")).unwrap();
    field.click();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.key_press(Key::Backspace);
    h.run_steps(2);
    h.key_press(Key::Tab);
    h.run_steps(3);
    assert_eq!(field_val(&h, id, "radius"), 7.68);
}

#[test]
fn arrow_keys_step_the_decimal_fields_by_a_hundredth() {
    let (mut h, id) = open();
    let field = h.query_all_by_role(Role::SpinButton).find(|node| node.accesskit_node().label().as_deref() == Some("Gamma")).unwrap();
    field.click();
    h.run_steps(2);
    h.key_press(Key::ArrowUp);
    h.run_steps(2);
    h.key_press(Key::Tab);
    h.run_steps(3);
    assert_eq!(field_val(&h, id, "gamma"), 1.01, "stored as shown, not the widened f32");
}

#[test]
fn an_ime_commit_counts_as_typing() {
    let (mut h, id) = open();
    let field = h.query_all_by_role(Role::SpinButton).find(|node| node.accesskit_node().label().as_deref() == Some("Radius")).unwrap();
    field.click();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(Event::Ime(egui::ImeEvent::Commit("9".into())));
    h.run_steps(2);
    h.key_press(Key::Tab);
    h.run_steps(3);
    assert_eq!(field_val(&h, id, "radius"), 9.0);
}

#[test]
fn each_slider_is_announced_by_its_name() {
    let (h, _) = open();
    for name in ["Radius", "Strength", "Gamma", "Exposure", "Detail", "Shadow", "Highlight", "Vibrance", "Saturation"] {
        assert!(h.query_all_by_role(Role::Slider).any(|n| n.accesskit_node().label().as_deref() == Some(name)), "{name} slider");
    }
}

#[test]
fn a_value_that_rounds_into_the_range_shows_no_alert() {
    let (mut h, id) = open();
    type_field(&mut h, "Gamma", "9.994");
    assert!(alerts(&h).is_empty(), "{:?}", alerts(&h));
    assert!((field_val(&h, id, "gamma") - 9.99).abs() < 1e-6);
}

#[test]
fn a_value_that_rounds_to_zero_is_stored_as_zero() {
    let (mut h, id) = open();
    type_field(&mut h, "Exposure", "-0.004");
    let v = field_val(&h, id, "exposure");
    assert!(v == 0.0 && v.is_sign_positive(), "{v}");
}

#[test]
fn typing_key_by_key_ends_at_the_typed_number() {
    let (mut h, id) = open();
    let field = h.query_all_by_role(Role::SpinButton).find(|node| node.accesskit_node().label().as_deref() == Some("Strength")).unwrap();
    field.click();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    for c in ["0", ".", "7", "3"] {
        h.event(Event::Text(c.into()));
        h.run_steps(1);
    }
    h.key_press(Key::Tab);
    h.run_steps(3);
    assert_eq!(field_val(&h, id, "strength"), 0.73);
    assert!(alerts(&h).is_empty(), "{:?}", alerts(&h));
}
