//! #16745: the Color Picker's Add to Swatches asks for the swatch's name first, in the "Color Swatch Name" dialog: the
//! new colour's chip and `Name:`, pre-filled with the first free "Swatch N" and selected so typing replaces it. OK adds
//! the swatch under that name and returns to the picker, which stays open; Cancel adds nothing. Over the control
//! channel the dialog is listed by `ui.inspect` and confirmed with a `name` field.

use craft::color_ui::ui::ColorPickerDialog;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::*;
use crate::control::{ControlRequest, Reply, handle};

/// A harness running the app's dialog pass, so clicks and keys reach the hosted dialogs.
fn harness() -> Harness<'static, Box<PhotocraftApp>> {
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let mut h = Harness::builder()
        .with_context(app.ctx().clone())
        .with_size(vec2(900.0, 640.0))
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::studio());
    h.run_steps(2);
    h
}

/// Opens the foreground picker on #336699 and clicks its Add to Swatches; returns the picker's id.
fn add_to_swatches(h: &mut Harness<'static, Box<PhotocraftApp>>) -> DialogId {
    let picker = open(h.state_mut(), "foreground");
    testing::set_color(h.state_mut(), picker, "#336699");
    h.run_steps(4);
    h.get_by_label("Add to Swatches").click();
    h.run_steps(4);
    picker
}

/// Every swatch as (name, hex).
fn swatches(app: &mut PhotocraftApp) -> Vec<(String, String)> {
    let listed = app.session.execute("swatches.list", json!({})).unwrap();
    let groups = listed["groups"].as_array().cloned().unwrap_or_default();
    groups
        .iter()
        .flat_map(|g| g["swatches"].as_array().cloned().unwrap_or_default())
        .map(|s| (s["name"].as_str().unwrap_or_default().to_owned(), s["hex"].as_str().unwrap_or_default().to_owned()))
        .collect()
}

/// The top hosted dialog as `ui.inspect` lists it.
fn top(app: &mut PhotocraftApp) -> Value {
    let ctx = app.ctx().clone();
    let (req, _rx) = ControlRequest::new("ui.inspect", json!({}));
    let Reply::Done(reply) = handle(app, &ctx, &req) else { panic!("ui.inspect replies at once") };
    reply["result"]["dialogs"].as_array().and_then(|d| d.last().cloned()).unwrap_or(Value::Null)
}

fn call(app: &mut PhotocraftApp, method: &str, params: Value) -> Value {
    let ctx = app.ctx().clone();
    let (req, _rx) = ControlRequest::new(method, params);
    let Reply::Done(reply) = handle(app, &ctx, &req) else { panic!("{method} replies at once") };
    reply
}

fn picker_open(h: &Harness<'static, Box<PhotocraftApp>>, picker: DialogId) -> bool {
    h.state().framework.dialogs.get_by_id::<ColorPickerDialog>(picker).is_some()
}

#[test]
fn add_to_swatches_asks_for_the_name_and_ok_adds_it_with_the_picker_still_open() {
    let mut h = harness();
    let before = swatches(h.state_mut());
    let picker = add_to_swatches(&mut h);
    let dialog = top(h.state_mut());
    assert_eq!(dialog["key"], "colorPicker.swatchName", "{dialog}");
    assert!(h.query_all_by_label("Color Swatch Name").next().is_some(), "the dialog's title");
    assert_eq!(dialog["fields"]["name"], "Swatch 1", "pre-filled with the first free number: {dialog}");
    assert_eq!(swatches(h.state_mut()), before, "nothing is added before OK");
    assert!(h.get_all_by_role_and_label(egui::accesskit::Role::ColorWell, "Swatch color").next().is_some(), "the chip shows the colour");

    // Enter answers the top dialog's OK; the picker below has its own OK and Cancel.
    h.key_press(egui::Key::Enter);
    h.run_steps(4);
    let after = swatches(h.state_mut());
    assert_eq!(after.len(), before.len() + 1);
    assert!(after.contains(&("Swatch 1".to_owned(), "#336699".to_owned())), "{after:?}");
    assert!(picker_open(&h, picker), "OK returns to the picker, which stays open");
}

#[test]
fn typing_replaces_the_selected_default_name() {
    let mut h = harness();
    let picker = add_to_swatches(&mut h);
    h.event(egui::Event::Text("Sea".into()));
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(4);
    assert!(swatches(h.state_mut()).contains(&("Sea".to_owned(), "#336699".to_owned())), "{:?}", swatches(h.state_mut()));
    assert!(picker_open(&h, picker));
}

#[test]
fn escape_adds_nothing_and_keeps_the_picker() {
    let mut h = harness();
    let before = swatches(h.state_mut());
    let picker = add_to_swatches(&mut h);
    h.key_press(egui::Key::Escape);
    h.run_steps(4);
    assert_eq!(swatches(h.state_mut()), before);
    assert!(picker_open(&h, picker), "Escape returns to the picker");
    assert_eq!(top(h.state_mut())["id"], json!(picker.value().to_string()), "the name dialog is gone");
}

#[test]
fn the_default_is_the_first_free_number() {
    let mut h = harness();
    for name in ["Swatch 1", "Swatch 3"] {
        h.state_mut().run("swatches.add", json!({"color": "#000000", "name": name})).unwrap();
    }
    add_to_swatches(&mut h);
    assert_eq!(top(h.state_mut())["fields"]["name"], "Swatch 2");
}

#[test]
fn the_control_channel_confirms_it_with_a_name() {
    let mut h = harness();
    let picker = add_to_swatches(&mut h);
    let dialog = top(h.state_mut())["id"].clone();
    assert_ne!(dialog, json!(picker.value().to_string()), "the name dialog sits above the picker");
    assert_eq!(call(h.state_mut(), "ui.dialog.set", json!({"dialog": dialog, "field": "name", "value": "Deep Sea"}))["ok"], true);
    assert_eq!(call(h.state_mut(), "ui.dialog.confirm", json!({"dialog": dialog}))["ok"], true);
    h.run_steps(2);
    assert!(swatches(h.state_mut()).contains(&("Deep Sea".to_owned(), "#336699".to_owned())));
    assert!(picker_open(&h, picker));
}

/// A blank name, which `swatches.add` refuses, keeps the dialog open and adds nothing.
#[test]
fn a_blank_name_is_refused_and_the_dialog_stays() {
    let mut h = harness();
    let before = swatches(h.state_mut());
    add_to_swatches(&mut h);
    let dialog = top(h.state_mut())["id"].clone();
    assert_eq!(call(h.state_mut(), "ui.dialog.set", json!({"dialog": dialog, "field": "name", "value": "  "}))["ok"], true);
    call(h.state_mut(), "ui.dialog.confirm", json!({"dialog": dialog}));
    h.run_steps(2);
    assert_eq!(swatches(h.state_mut()), before);
    assert_eq!(top(h.state_mut())["id"], dialog, "the dialog stays open");
}
