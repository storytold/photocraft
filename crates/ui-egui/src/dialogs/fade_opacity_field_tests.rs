//! #13811: Edit › Fade's Opacity takes whole percents from 0 to 100. A typed value past the range
//! becomes the closest end and, once the field is left, shows the reference application's alert.

use egui::{Event, Key, Modifiers, accesskit::Role};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use serde_json::{Value, json};

use super::FADE_OPACITY_RANGE_ALERT;
use crate::{PhotocraftApp, Services, dialogs::show, state::DialogKind};

const FADE: &str = "edit.fade";

fn open() -> (Harness<'static, PhotocraftApp>, u64) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width": 40, "height": 30, "background": "white"})).unwrap();
    app.run("image.adjustments.invert", json!({})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    let ctx = h.ctx.clone();
    let id = crate::menus::invoke(h.state_mut(), &ctx, FADE, json!({})).unwrap()["dialog"].as_u64().unwrap();
    h.run_steps(4);
    (h, id)
}

fn type_opacity(h: &mut Harness<'static, PhotocraftApp>, input: &str) {
    let field = h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Opacity")).expect("an Opacity field");
    field.click();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(Event::Text(input.into()));
    h.run_steps(1);
}

fn leave(h: &mut Harness<'static, PhotocraftApp>) {
    h.key_press(Key::Tab);
    h.run_steps(3);
}

fn opacity(h: &Harness<'_, PhotocraftApp>, id: u64) -> Value {
    h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields["opacity"].clone()
}

fn alerts(h: &Harness<'_, PhotocraftApp>) -> usize {
    h.state().ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error && d.fields.get("message") == Some(&json!(FADE_OPACITY_RANGE_ALERT))).count()
}

#[test]
fn values_past_the_range_take_the_closest_end_and_alert_once_the_field_is_left() {
    for (typed, closest) in [("150", 100), ("101", 100), ("-5", 0)] {
        let (mut h, id) = open();
        type_opacity(&mut h, typed);
        assert_eq!(alerts(&h), 0, "{typed}: no alert while typing");
        leave(&mut h);
        assert_eq!(opacity(&h, id), json!(closest), "{typed}");
        assert_eq!(alerts(&h), 1, "{typed}: one alert");
        assert_eq!(h.state().ui.dialogs.last().unwrap().kind, DialogKind::Error);
    }
}

#[test]
fn whole_values_in_range_are_kept_without_an_alert() {
    for (typed, kept) in [("0", 0), ("37", 37), ("100", 100)] {
        let (mut h, id) = open();
        type_opacity(&mut h, typed);
        leave(&mut h);
        assert_eq!(opacity(&h, id), json!(kept), "{typed}");
        assert_eq!(alerts(&h), 0, "{typed}");
    }
}

#[test]
fn a_typed_fraction_is_read_without_its_dot() {
    let (mut h, id) = open();
    type_opacity(&mut h, "0.4");
    leave(&mut h);
    assert_eq!(opacity(&h, id), json!(4));
    assert_eq!(alerts(&h), 0);

    let (mut h, id) = open();
    type_opacity(&mut h, "12.3");
    leave(&mut h);
    assert_eq!(opacity(&h, id), json!(100), "12.3 reads as 123");
    assert_eq!(alerts(&h), 1);
}

#[test]
fn text_that_is_not_a_number_keeps_the_value() {
    let (mut h, id) = open();
    type_opacity(&mut h, "42");
    leave(&mut h);
    type_opacity(&mut h, "abc");
    leave(&mut h);
    assert_eq!(opacity(&h, id), json!(42));
    assert_eq!(alerts(&h), 0);
}

#[test]
fn the_stored_opacity_is_always_a_whole_number() {
    let (mut h, id) = open();
    assert_eq!(opacity(&h, id), json!(100), "the default");
    h.state_mut().ui.dialogs.iter_mut().find(|d| d.id == id).unwrap().fields.insert("opacity".into(), json!(33.6));
    h.run_steps(2);
    assert_eq!(opacity(&h, id), json!(34));
}

#[test]
fn the_schema_keeps_the_0_to_100_range_with_100_by_default() {
    let spec = photocraft_engine::commands::find(FADE).unwrap();
    let p = crate::filter_dialog::parse_spec(spec.params).into_iter().find(|p| p.key == "opacity").unwrap();
    assert!(matches!(p.kind, crate::filter_dialog::Kind::Range { min: 0.0, max: 100.0, default: 100.0 }));
    assert!(spec.params.contains("whole %"));
}

/// CodeRabbit on #15099 and #15123: scrubbing the field is not typing. A scrub's fractions (37.5, from the
/// drag speed) are rounded like any field's drag, not read without their dot (375, clamped to 100); typed
/// text still is, read from the text as typed (`0.40` is 40, `0.4` is 4).
#[test]
fn a_scrub_drag_rounds_and_does_not_drop_the_dot() {
    use super::read_fade_opacity;
    assert_eq!(read_fade_opacity(12.3, Some("12.3"), true), 123.0);
    assert_eq!(read_fade_opacity(0.4, Some("0.4"), true), 4.0);
    assert_eq!(read_fade_opacity(0.4, Some("0.40"), true), 40.0, "trailing zeros count");
    assert_eq!(read_fade_opacity(0.1, Some("0.10"), true), 10.0);
    assert_eq!(read_fade_opacity(50.0, Some("50"), true), 50.0);
    assert_eq!(read_fade_opacity(0.4, None, true), 0.4, "without the text nothing was typed: the number itself");
    // A drag: the number itself, which the field rounds (37.5 -> 38, never 375 -> 100).
    assert_eq!(read_fade_opacity(37.5, None, false), 37.5);
    assert_eq!(read_fade_opacity(37.5, Some("37.5"), false).round().clamp(0.0, 100.0), 38.0);
    assert_eq!(read_fade_opacity(12.3, None, false).round(), 12.0);
}

/// CodeRabbit on #15123: typing `0.40` into the field (through the widget, so its text is what was typed) gives 40.
#[test]
fn typing_a_decimal_with_a_trailing_zero_keeps_the_zero() {
    let (mut h, id) = open();
    type_opacity(&mut h, "0.40");
    leave(&mut h);
    assert_eq!(opacity(&h, id), json!(40));
    type_opacity(&mut h, "0.4");
    leave(&mut h);
    assert_eq!(opacity(&h, id), json!(4));
}

/// CodeRabbit on #15123: `0.4` then a `0` typed in a later frame is `0.40` to the parser but 0.4 to the number,
/// which does not change. The text is what is read, so opacity goes from 4 to 40.
#[test]
fn appending_a_zero_in_a_later_frame_reads_the_whole_text() {
    let (mut h, id) = open();
    type_opacity(&mut h, "0.4");
    h.run_steps(2);
    assert_eq!(opacity(&h, id), json!(4), "0.4 is 4");
    h.event(Event::Text("0".into()));
    h.run_steps(2);
    leave(&mut h);
    assert_eq!(opacity(&h, id), json!(40), "0.40 is 40");
    assert_eq!(alerts(&h), 0);
}

/// The text the field held before it had the focus is not an entry: clicking into it after `150` was
/// entered does not read that text again (the value stays 100, no second alert).
#[test]
fn clicking_back_into_the_field_does_not_reread_the_old_text() {
    let (mut h, id) = open();
    type_opacity(&mut h, "150");
    leave(&mut h);
    assert_eq!(opacity(&h, id), json!(100));
    h.state_mut().ui.dialogs.retain(|d| d.kind != DialogKind::Error);
    h.state_mut().ui.dialogs.iter_mut().find(|d| d.id == id).unwrap().fields.insert("opacity".into(), json!(60));
    h.run_steps(2);
    let field = h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Opacity")).expect("an Opacity field");
    field.click();
    h.run_steps(3);
    assert_eq!(opacity(&h, id), json!(60), "gaining the focus changes nothing");
    assert_eq!(alerts(&h), 0);
}

/// CodeRabbit on #15123: after `0.4` (opacity 4, the number's draft 0.4) the text is replaced by something that is not a
/// number. The parser keeps the draft 0.4, which must not be read again as an entry (0.4 rounds to 0): opacity stays 4.
#[test]
fn replacing_a_fraction_with_text_that_is_not_a_number_keeps_the_value() {
    let (mut h, id) = open();
    type_opacity(&mut h, "0.4");
    h.run_steps(2);
    assert_eq!(opacity(&h, id), json!(4), "0.4 is 4");
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(Event::Text("not a number".into()));
    h.run_steps(2);
    assert_eq!(opacity(&h, id), json!(4), "while typing");
    leave(&mut h);
    assert_eq!(opacity(&h, id), json!(4), "once the field is left");
    assert_eq!(alerts(&h), 0);
}

/// CodeRabbit on #15106: the Enter that leaves an out-of-range Opacity raises the alert and does not also
/// confirm Fade: the dialog stays open, with the closest value in it, and the alert on top.
#[test]
fn enter_on_an_out_of_range_value_raises_the_alert_and_keeps_fade_open() {
    let (mut h, id) = open();
    type_opacity(&mut h, "150");
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert!(h.state().ui.dialogs.iter().any(|d| d.id == id), "Fade is still open");
    assert_eq!(alerts(&h), 1, "the range alert is up");
    assert_eq!(opacity(&h, id), json!(100), "with the closest value");
}

/// CodeRabbit on #15106: the opacity slider keeps its double-click reset to 100 (`RowGestures::reset`), which the
/// plain `slider` did not carry. The widget's own reset is `slider_with`'s, unchanged; a harness frame is too
/// long (0.25 to 0.5 s) to land two clicks inside its 0.5 s double-click window, so this pins the setting.
#[test]
fn the_opacity_slider_resets_to_100_on_a_double_click() {
    let g = super::fade_slider_gestures();
    assert_eq!(g.reset, Some(100.0));
    assert_eq!(g.wheel_step, None, "nothing else changes about the slider");
}

/// CodeRabbit on #15106: a ⌘/Ctrl arrow step on the focused Opacity field moves it by 0.1, which is not typed text: 50
/// stays 50 (it was read without its dot as 501, clamped to 100, with the range alert).
#[test]
fn a_command_arrow_step_does_not_read_its_fraction_as_typed_text() {
    let (mut h, id) = open();
    type_opacity(&mut h, "50");
    h.key_press_modifiers(Modifiers::COMMAND, Key::ArrowUp);
    h.run_steps(3);
    assert_eq!(opacity(&h, id), json!(50));
    assert_eq!(alerts(&h), 0, "no range alert");
}

/// CodeRabbit on #15106: a pasted value is typed text whatever modifier was held for the paste: `150` is clamped to
/// 100 and the range alert comes once the field is left.
#[test]
fn a_pasted_value_past_the_range_raises_the_alert() {
    let (mut h, id) = open();
    let field = h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Opacity")).expect("an Opacity field");
    field.click();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(Event::Paste("150".into()));
    h.run_steps(2);
    leave(&mut h);
    assert_eq!(opacity(&h, id), json!(100));
    assert_eq!(alerts(&h), 1, "the range alert");
}

/// CodeRabbit on #15099: after `150` set the out-of-range mark, replacing the text with something that is not a number
/// takes the mark back: leaving the field raises no alert.
#[test]
fn text_that_is_not_a_number_clears_an_earlier_range_mark() {
    let (mut h, id) = open();
    type_opacity(&mut h, "150");
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(Event::Text("abc".into()));
    h.run_steps(1);
    leave(&mut h);
    assert_eq!(alerts(&h), 0, "no range alert");
    assert_eq!(opacity(&h, id), json!(100), "the clamped value stays");
}

/// CodeRabbit on #15106: a typed whole number with a dot is read like any typed fraction: `1.0` is 10, `2.0` is 20.
#[test]
fn a_typed_whole_number_with_a_dot_loses_its_dot() {
    let (mut h, id) = open();
    type_opacity(&mut h, "1.0");
    leave(&mut h);
    assert_eq!(opacity(&h, id), json!(10));
    let (mut h, id) = open();
    type_opacity(&mut h, "2.0");
    leave(&mut h);
    assert_eq!(opacity(&h, id), json!(20));
}

/// CodeRabbit on #15106: an arrow key after a typed entry steps the stored percent, not the draft kept from the entry:
/// after `0.4` (stored 4), ArrowUp is 5, not 1.4.
#[test]
fn an_arrow_after_typing_steps_the_stored_value() {
    let (mut h, id) = open();
    type_opacity(&mut h, "0.4");
    assert_eq!(opacity(&h, id), json!(4));
    h.key_press(Key::ArrowUp);
    h.run_steps(3);
    assert_eq!(opacity(&h, id), json!(5));
}

/// CodeRabbit on #15099: a number set without any typed text (an accessibility action, a scrub) is not typed text: its
/// dot stays (37.5 is 37.5, never 375).
#[test]
fn a_number_set_without_typed_text_keeps_its_dot() {
    use super::read_fade_opacity;
    assert_eq!(read_fade_opacity(37.5, None, true), 37.5);
    assert_eq!(read_fade_opacity(1.0, Some("1.0"), true), 10.0);
}
