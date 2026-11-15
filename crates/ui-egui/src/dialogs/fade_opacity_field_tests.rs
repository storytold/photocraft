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
