//! #14243: Color Settings' Blend Text Colors Using Gamma is a checkbox (on by default) with a
//! 1.00 to 3.00 field beside it. The field stays editable while the box is off; typing a value
//! checks the box, focusing the field alone does not. A value typed out of range alerts "A number
//! between 1.00 and 3.00 is required. Closest value inserted." and puts in the closest end; 1.234
//! becomes 1.23 without one; an empty field puts back its value. OK sends both, so the box off
//! keeps the value and on again uses it.

use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use egui::accesskit::{Role, Toggled};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_engine::color_cmds::{ColorExt, ColorSettings};
use serde_json::{Value, json};

use crate::PhotocraftApp;

const COMMAND: &str = "edit.colorSettings";
const BOX: &str = "Blend Text Colors Using Gamma";

type H = Harness<'static, Box<PhotocraftApp>>;

/// Color Settings holding `settings`, opened from the menu and shown through `dialogs::run`.
fn dialog(settings: ColorSettings) -> (H, DialogId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.color_mut().settings = settings;
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 1100.0))
        .with_step_dt(1.0 / 60.0)
        .with_context(app.ctx().clone())
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    let ctx = h.ctx.clone();
    let reply = crate::menus::invoke(h.state_mut(), &ctx, COMMAND, json!({})).unwrap();
    let want = reply["dialog"].as_str().unwrap().to_owned();
    let id = h.state().framework.dialogs.open_dialogs().into_iter().map(|d| d.id).find(|d| d.value().to_string() == want).unwrap();
    h.run_steps(4);
    (h, id)
}

fn fields(h: &H, id: DialogId) -> Value {
    let form = h.state().framework.dialogs.get_by_id::<HostedParamDialog>(id).expect("Color Settings is open");
    serde_json::to_value(form.fields()).unwrap()
}

/// The box and the field, the field in hundredths.
fn shown(h: &H, id: DialogId) -> (bool, i64) {
    let f = fields(h, id);
    (f["blendTextGammaOn"].as_bool().expect("the box"), (f["blendTextGamma"].as_f64().expect("the field") * 100.0).round() as i64)
}

fn checked(h: &H) -> bool {
    h.get_by_role_and_label(Role::CheckBox, BOX).accesskit_node().toggled() == Some(Toggled::True)
}

/// The gamma field, on the checkbox's line.
fn field(h: &H) -> egui_kittest::Node<'_> {
    let line = h.get_by_role_and_label(Role::CheckBox, BOX).rect();
    let field = h.get_by_role_and_label(Role::SpinButton, BOX);
    let at = field.rect().center();
    assert!(at.y > line.top() - 4.0 && at.y < line.bottom() + 4.0 && at.x > line.right(), "the field is beside the checkbox");
    field
}

/// `number` written with the decimal mark the fields read on this system.
fn local(number: &str) -> String {
    let mark = craft::i18n::Region::system().decimal.filter(|c| *c == ',').unwrap_or('.');
    number.replace('.', &mark.to_string())
}

fn focus_field(h: &mut H) {
    field(h).click();
    h.run_steps(2);
}

/// Types `text` over the field's value, then leaves it with Tab.
fn type_into(h: &mut H, text: &str) {
    focus_field(h);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    if text.is_empty() {
        h.key_press(egui::Key::Backspace);
    } else {
        h.event(egui::Event::Text(local(text)));
    }
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(4);
}

/// The messages of the open alerts.
fn alerts(h: &H) -> Vec<String> {
    h.state().framework.dialogs.open_dialogs().iter().filter_map(|d| d.fields.as_ref()?.get("message")?.as_str().map(str::to_owned)).collect()
}

fn close_alert(h: &mut H) {
    h.get_by_role(Role::AlertDialog).get_by_label("OK").click();
    h.run_steps(4);
}

fn toggle_box(h: &mut H) {
    h.get_by_role_and_label(Role::CheckBox, BOX).click();
    h.run_steps(4);
}

fn confirm(h: &mut H, id: DialogId) {
    crate::dialogs::testing::confirm(h.state_mut(), id).unwrap();
    h.run_steps(2);
}

#[test]
fn a_checkbox_on_by_default_with_the_field_beside_it() {
    let (h, id) = dialog(ColorSettings::default());
    assert!(checked(&h), "on by default");
    assert_eq!(shown(&h, id), (true, 145));
    assert_eq!(field(&h).accesskit_node().numeric_value().map(|v| (v * 100.0).round()), Some(145.0), "1.45 in the field");
    assert!(h.query_all_by_label("Blend Text Gamma").next().is_none(), "no slider row of the old name");
}

#[test]
fn the_box_off_keeps_the_value_and_on_again_uses_it() {
    let (mut h, id) = dialog(ColorSettings { blend_text_gamma: 2.4, ..ColorSettings::default() });
    toggle_box(&mut h);
    assert!(!checked(&h));
    assert_eq!(shown(&h, id), (false, 240), "the field keeps 2.40 while the box is off");
    confirm(&mut h, id);
    let c = h.state().session.color().settings.clone();
    assert_eq!((c.blend_text_gamma_on, c.blend_text_gamma), (false, 2.4));
    assert_eq!(c.text_gamma(), 1.0);

    let (mut h, id) = dialog(c);
    assert_eq!(shown(&h, id), (false, 240), "reopened with the box off and the value");
    toggle_box(&mut h);
    confirm(&mut h, id);
    let c = &h.state().session.color().settings;
    assert_eq!((c.blend_text_gamma_on, c.text_gamma()), (true, 2.4));
}

#[test]
fn typing_checks_the_box_and_focusing_alone_does_not() {
    let (mut h, id) = dialog(ColorSettings { blend_text_gamma_on: false, ..ColorSettings::default() });
    focus_field(&mut h);
    h.key_press(egui::Key::Tab);
    h.run_steps(4);
    assert_eq!(shown(&h, id), (false, 145), "clicking into the field and tabbing out leaves the box off");
    type_into(&mut h, "2.2");
    assert_eq!(shown(&h, id), (true, 220), "the field is editable with the box off, and typing checks it");
    assert!(checked(&h));
    confirm(&mut h, id);
    let c = &h.state().session.color().settings;
    assert_eq!((c.blend_text_gamma_on, (c.blend_text_gamma * 100.0).round()), (true, 220.0));
}

#[test]
fn out_of_range_alerts_and_puts_in_the_closest_value() {
    // "1.00" and "3.00" in the system's number format.
    let message = format!("A number between {} and {} is required. Closest value inserted.", local("1.00"), local("3.00"));
    for (typed, closest) in [("0", 100), ("9", 300), ("0.5", 100), ("3.01", 300)] {
        let (mut h, id) = dialog(ColorSettings::default());
        type_into(&mut h, typed);
        assert_eq!(alerts(&h), std::slice::from_ref(&message), "{typed}");
        assert_eq!(shown(&h, id).1, closest, "{typed}");
        close_alert(&mut h);
        assert!(alerts(&h).is_empty(), "{typed}: OK closes the alert");
        assert_eq!(shown(&h, id).1, closest, "{typed}: the closest value stays");
    }
    // Two decimals: 1.234 is 1.23, with no alert.
    let (mut h, id) = dialog(ColorSettings::default());
    type_into(&mut h, "1.234");
    assert!(alerts(&h).is_empty(), "{:?}", alerts(&h));
    assert_eq!(shown(&h, id), (true, 123));
    confirm(&mut h, id);
    assert!((h.state().session.color().settings.blend_text_gamma - 1.23).abs() < 1e-6);
}

#[test]
fn an_empty_field_puts_back_its_value() {
    let (mut h, id) = dialog(ColorSettings { blend_text_gamma: 1.8, ..ColorSettings::default() });
    type_into(&mut h, "");
    assert!(alerts(&h).is_empty(), "{:?}", alerts(&h));
    assert_eq!(shown(&h, id), (true, 180));
}
