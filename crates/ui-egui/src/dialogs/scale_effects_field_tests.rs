//! #14995: Layer Style › Scale Effects' Scale field takes whole percents 1..1000. A value typed
//! past the range or with decimals ("33,6") becomes the closest whole number in range, and once
//! the field is left an alert titled "Scale Layer Effects" says so over the still-open dialog. A
//! typed "." is ignored ("33.6" reads 336). Up/Down step by 1, Shift+Up/Down by 10, and stop at
//! the ends without an alert. Other dialogs' wide-range fields keep their decimals.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Map, Value, json};

use super::{SCALE_ALERT, SCALE_ALERT_TEXT, SCALE_ALERT_TITLE, body, open, params_of};
use crate::PhotocraftApp;

const CMD: &str = "layer.layerStyle.scaleEffects";

/// The dialog's fields, whether an Enter reached the dialog's buttons after the body, and an Esc.
type State = (Map<String, Value>, bool, bool);

/// The fields Scale Effects opens with, on a layer that has an effect.
fn scale_fields() -> Map<String, Value> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 30, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({"name": "Shape"})).unwrap();
    app.run("layer.layerStyle.dropShadow", json!({"distance": 10, "size": 4})).unwrap();
    let id = open(&mut app, CMD).unwrap();
    app.ui.dialog_mut(id).unwrap().fields.clone()
}

/// The dialog body, as a dialog draws it; after it, an Enter still pending would press OK.
fn harness(fields: Map<String, Value>) -> Harness<'static, State> {
    let mut h = Harness::builder().with_size(vec2(520.0, 360.0)).build_ui_state(
        |ui, s: &mut State| {
            body(ui, &mut s.0);
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                s.1 = true;
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                s.2 = true;
            }
        },
        (fields, false, false),
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(3);
    h
}

fn field<'a>(h: &'a Harness<'static, State>, name: &str) -> egui_kittest::Node<'a> {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().is_some_and(|l| l.trim_end_matches(':') == name)).expect("the field")
}

/// Types `text` into the field named `name`, then leaves it with `leave`.
fn type_into(h: &mut Harness<'static, State>, name: &str, text: &str, leave: egui::Key) {
    field(h, name).click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(leave);
    h.run_steps(3);
}

fn scale(h: &Harness<'static, State>) -> f64 {
    h.state().0.get("scale").and_then(Value::as_f64).unwrap()
}

fn alert_up(h: &Harness<'static, State>) -> bool {
    h.state().0.get(SCALE_ALERT).and_then(Value::as_bool) == Some(true)
}

#[test]
fn values_past_the_range_insert_the_closest_and_raise_the_alert() {
    for (text, want) in [("0", 1.0), ("-5", 1.0), ("1001", 1000.0)] {
        let mut h = harness(scale_fields());
        type_into(&mut h, "Scale", text, egui::Key::Tab);
        assert_eq!(scale(&h), want, "{text}");
        assert!(alert_up(&h), "{text} raises the alert");
        let shown = h.query_by_label(SCALE_ALERT_TITLE).is_some() && h.query_by_label(SCALE_ALERT_TEXT).is_some();
        assert!(shown, "{text}: the alert is titled and worded as measured");
    }
}

#[test]
fn a_decimal_comma_rounds_to_34_with_the_alert() {
    let mut h = harness(scale_fields());
    type_into(&mut h, "Scale", "33,6", egui::Key::Tab);
    assert_eq!(scale(&h), 34.0);
    assert!(alert_up(&h));
}

#[test]
fn a_typed_point_is_ignored() {
    let mut h = harness(scale_fields());
    type_into(&mut h, "Scale", "33.6", egui::Key::Tab);
    assert_eq!(scale(&h), 336.0);
    assert!(!alert_up(&h), "336 is in range");
}

#[test]
fn a_whole_value_in_range_says_nothing() {
    let mut h = harness(scale_fields());
    type_into(&mut h, "Scale", "250", egui::Key::Tab);
    assert_eq!(scale(&h), 250.0);
    assert!(!alert_up(&h));
    assert_eq!(params_of(&h.state().0), json!({"scale": 250}), "OK sends a whole percent, as an integer");
}

#[test]
fn arrows_step_by_1_and_shift_arrows_by_10_and_stop_at_the_ends_quietly() {
    let mut h = harness(scale_fields());
    assert_eq!(scale(&h), 100.0);
    field(&h, "Scale").click();
    h.run_steps(1);
    h.key_press(egui::Key::ArrowUp);
    h.run_steps(2);
    assert_eq!(scale(&h), 101.0);
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::ArrowUp);
    h.run_steps(2);
    assert_eq!(scale(&h), 111.0);
    h.key_press(egui::Key::ArrowDown);
    h.run_steps(2);
    assert_eq!(scale(&h), 110.0);
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::ArrowDown);
    h.run_steps(2);
    assert_eq!(scale(&h), 100.0);

    let mut fields = scale_fields();
    fields.insert("scale".into(), json!(1000));
    let mut h = harness(fields);
    field(&h, "Scale").click();
    h.run_steps(1);
    h.key_press(egui::Key::ArrowUp);
    h.run_steps(2);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    assert_eq!(scale(&h), 1000.0);
    assert!(!alert_up(&h), "a step past the end is no typed value");
}

#[test]
fn the_enter_that_leaves_a_wrong_value_does_not_press_ok() {
    let mut h = harness(scale_fields());
    type_into(&mut h, "Scale", "0", egui::Key::Enter);
    assert_eq!(scale(&h), 1.0);
    assert!(alert_up(&h), "the alert is up");
    assert!(!h.state().1, "the dialog's OK saw no Enter");
}

#[test]
fn ok_enter_and_esc_each_close_the_alert_and_keep_the_dialog() {
    for close in ["OK", "Enter", "Esc"] {
        let mut h = harness(scale_fields());
        type_into(&mut h, "Scale", "1001", egui::Key::Tab);
        assert!(alert_up(&h), "{close}: the alert is up");
        match close {
            "OK" => h.get_by_role_and_label(Role::Button, "OK").click(),
            "Enter" => h.key_press(egui::Key::Enter),
            _ => h.key_press(egui::Key::Escape),
        }
        h.run_steps(3);
        assert!(!alert_up(&h), "{close} closes the alert");
        assert!(!h.state().1, "{close}: the dialog's OK saw no Enter");
        assert_eq!(scale(&h), 1000.0, "{close}: the inserted value stays");
    }
}

#[test]
fn other_wide_range_fields_keep_their_decimals() {
    let mut fields = Map::new();
    fields.insert("__command".into(), json!("select.modify.feather"));
    fields.insert("radius".into(), json!(1.0));
    let mut h = harness(fields);
    type_into(&mut h, "Radius", "33.6", egui::Key::Tab);
    let radius = h.state().0.get("radius").and_then(Value::as_f64).unwrap();
    assert!((radius - 33.6).abs() < 1e-3, "Feather keeps 33.6: {radius}");
    assert!(!alert_up(&h));
}

#[test]
fn esc_after_a_wrong_value_gives_up_the_entry_without_the_alert() {
    let mut h = harness(scale_fields());
    type_into(&mut h, "Scale", "0", egui::Key::Escape);
    assert!(!alert_up(&h), "Esc raises no alert");
    assert!(h.query_by_label(SCALE_ALERT_TITLE).is_none());
    assert!(h.state().2, "the dialog still sees the Esc (it cancels as for any Esc)");
}

#[test]
fn a_held_enter_does_not_close_the_alert_or_press_ok() {
    let mut h = harness(scale_fields());
    field(&h, "Scale").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("1001".into()));
    h.run_steps(1);
    // The Enter goes down and stays down: the first press leaves the field, the rest are repeats.
    let enter = |repeat| egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat, modifiers: egui::Modifiers::NONE };
    h.event(enter(false));
    h.run_steps(2);
    assert!(alert_up(&h), "the first press raises the alert");
    for _ in 0..3 {
        h.event(enter(true));
        h.run_steps(2);
    }
    assert!(alert_up(&h), "repeats do not close the alert");
    assert!(!h.state().1, "nor reach the dialog's OK");
    assert_eq!(scale(&h), 1000.0);
}

fn key(key: egui::Key, pressed: bool, repeat: bool, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key { key, physical_key: None, pressed, repeat, modifiers }
}

#[test]
fn a_key_held_past_the_alerts_close_does_not_reach_the_dialog() {
    for k in [egui::Key::Enter, egui::Key::Escape] {
        let mut h = harness(scale_fields());
        type_into(&mut h, "Scale", "1001", egui::Key::Tab);
        assert!(alert_up(&h));
        // The key goes down (it closes the alert) and stays down: its repeats follow.
        h.event(key(k, true, false, egui::Modifiers::NONE));
        h.run_steps(2);
        assert!(!alert_up(&h), "{k:?} closes the alert");
        for _ in 0..3 {
            h.event(key(k, true, true, egui::Modifiers::NONE));
            h.run_steps(2);
        }
        assert!(!h.state().1 && !h.state().2, "{k:?} repeats after the close reach neither OK nor Cancel");
        assert!(!alert_up(&h), "{k:?}: the dialog stays as it was");
        // Once released, the key is the dialog's again.
        h.event(key(k, false, false, egui::Modifiers::NONE));
        h.run_steps(1);
        h.event(key(k, true, false, egui::Modifiers::NONE));
        h.run_steps(2);
        let (enter, esc) = (h.state().1, h.state().2);
        assert_eq!((enter, esc), (k == egui::Key::Enter, k == egui::Key::Escape), "a new {k:?} press reaches the dialog, and only as {k:?}");
    }
}

#[test]
fn shift_enter_with_the_alert_up_closes_the_alert_only() {
    let mut h = harness(scale_fields());
    type_into(&mut h, "Scale", "1001", egui::Key::Tab);
    assert!(alert_up(&h));
    h.event(key(egui::Key::Enter, true, false, egui::Modifiers::SHIFT));
    h.run_steps(2);
    assert!(!alert_up(&h), "Shift+Enter closes the alert");
    assert!(!h.state().1, "and does not press the dialog's OK");
}

#[test]
fn shift_esc_with_the_alert_up_closes_the_alert_only() {
    let mut h = harness(scale_fields());
    type_into(&mut h, "Scale", "1001", egui::Key::Tab);
    assert!(alert_up(&h));
    h.event(key(egui::Key::Escape, true, false, egui::Modifiers::SHIFT));
    h.run_steps(2);
    assert!(!alert_up(&h), "Shift+Esc closes the alert");
    assert!(!h.state().2, "and does not cancel the dialog");
}

#[test]
fn ctrl_enter_with_the_alert_up_is_left_alone() {
    // Not pinned by the reference: Ctrl/Cmd+Enter is not the alert's key. As coded, it passes
    // through to the dialog and the alert stays up. This documents that, it does not endorse it.
    let mut h = harness(scale_fields());
    type_into(&mut h, "Scale", "1001", egui::Key::Tab);
    assert!(alert_up(&h));
    h.event(key(egui::Key::Enter, true, false, egui::Modifiers::COMMAND));
    h.run_steps(2);
    assert!(alert_up(&h), "the alert stays up");
    assert!(h.state().1, "the dialog sees the Ctrl+Enter");
}

#[test]
fn a_held_enter_after_the_alert_closes_does_not_leave_the_focused_field() {
    let mut h = harness(scale_fields());
    type_into(&mut h, "Scale", "1001", egui::Key::Tab);
    h.event(key(egui::Key::Enter, true, false, egui::Modifiers::NONE));
    h.run_steps(2);
    assert!(!alert_up(&h));
    field(&h, "Scale").click();
    h.run_steps(2);
    assert!(field(&h, "Scale").is_focused(), "the field has the keyboard");
    for _ in 0..3 {
        h.event(key(egui::Key::Enter, true, true, egui::Modifiers::NONE));
        h.run_steps(2);
    }
    assert!(field(&h, "Scale").is_focused(), "the held Enter's repeats do not take the field's focus");
    assert!(!alert_up(&h) && !h.state().1);
    assert_eq!(scale(&h), 1000.0);
}

#[test]
fn a_release_carrying_ctrl_still_ends_the_hold() {
    let mut h = harness(scale_fields());
    type_into(&mut h, "Scale", "1001", egui::Key::Tab);
    h.event(key(egui::Key::Enter, true, false, egui::Modifiers::NONE));
    h.run_steps(2);
    assert!(!alert_up(&h) && !h.state().1);
    // Ctrl was pressed before the Enter came up: the release carries it.
    h.event(key(egui::Key::Enter, false, false, egui::Modifiers::COMMAND));
    h.run_steps(2);
    h.event(key(egui::Key::Enter, true, true, egui::Modifiers::NONE));
    h.run_steps(2);
    assert!(h.state().1, "the key is the dialog's again");
}

#[test]
fn a_scale_held_in_range_when_it_is_serialized() {
    for (stored, want) in [(5000, 1000.0), (0, 1.0)] {
        let mut fields = scale_fields();
        fields.insert("scale".into(), json!(stored));
        let h = harness(fields);
        assert_eq!(scale(&h), want, "{stored}");
    }
}
