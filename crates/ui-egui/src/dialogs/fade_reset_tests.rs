//! #13812: Alt + Reset in the Fade dialog goes back to the values the dialog opened with (the last
//! confirmed Fade's), not to 100 % Normal.

use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;

const FADE: &str = "edit.fade";

fn harness() -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 30, "background": "white"})).unwrap();
    let mut h = Harness::builder()
        .with_context(app.ctx().clone())
        .with_size(egui::vec2(1000.0, 700.0))
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(2);
    h
}

/// Invert, then open Fade; returns the dialog's id.
fn invert_and_open_fade(h: &mut Harness<'static, Box<PhotocraftApp>>) -> DialogId {
    h.state_mut().run("image.adjustments.invert", json!({})).unwrap();
    let id = super::open(h.state_mut(), FADE).expect("Fade opens");
    h.run_steps(4);
    id
}

fn set(h: &mut Harness<'static, Box<PhotocraftApp>>, id: DialogId, opacity: i64, mode: &str) {
    let ctx = h.ctx.clone();
    h.state_mut().framework.dialogs.set_json_field(&ctx, id, "opacity", json!(opacity)).unwrap();
    h.state_mut().framework.dialogs.set_json_field(&ctx, id, "mode", json!(mode)).unwrap();
    h.run_steps(2);
}

fn is_open(h: &Harness<'_, Box<PhotocraftApp>>, id: DialogId) -> bool {
    h.state().framework.dialogs.get_by_id::<HostedParamDialog>(id).is_some()
}

/// The dialog's Opacity and Mode.
fn values(h: &Harness<'_, Box<PhotocraftApp>>, id: DialogId) -> (f64, String) {
    let dialog = h.state().framework.dialogs.get_by_id::<HostedParamDialog>(id).expect("the dialog stays open");
    let f = dialog.fields();
    (f.number("opacity").expect("opacity"), f.text("mode").expect("mode").to_string())
}

fn click(h: &mut Harness<'static, Box<PhotocraftApp>>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(6);
}

/// Alt turns Cancel into Reset.
fn alt_reset(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    h.get_by_label("Cancel").click_modifiers(egui::Modifiers::ALT);
    h.run_steps(4);
}

fn at(opacity: f64, mode: &str) -> (f64, String) {
    (opacity, mode.to_string())
}

#[test]
fn reset_returns_to_the_last_confirmed_values() {
    let mut h = harness();
    let id = invert_and_open_fade(&mut h);
    set(&mut h, id, 20, "screen");
    click(&mut h, "OK");
    assert!(!is_open(&h, id), "OK closes the dialog");

    let id = invert_and_open_fade(&mut h);
    assert_eq!(values(&h, id), at(20.0, "screen"), "it opens with the last values");
    set(&mut h, id, 60, "screen");
    alt_reset(&mut h);
    assert_eq!(values(&h, id), at(20.0, "screen"));
    set(&mut h, id, 45, "multiply");
    alt_reset(&mut h);
    assert_eq!(values(&h, id), at(20.0, "screen"), "a second Reset too");
}

#[test]
fn reset_after_changing_only_the_mode_keeps_todays_defaults() {
    // Not measured in the issue, so this keeps main's Reset: 100 % Normal.
    let mut h = harness();
    let id = invert_and_open_fade(&mut h);
    set(&mut h, id, 20, "screen");
    click(&mut h, "OK");
    let id = invert_and_open_fade(&mut h);
    set(&mut h, id, 20, "multiply");
    alt_reset(&mut h);
    assert_eq!(values(&h, id), at(100.0, "normal"));
}

#[test]
fn on_a_first_open_reset_returns_to_100_normal() {
    let mut h = harness();
    let id = invert_and_open_fade(&mut h);
    assert_eq!(values(&h, id), at(100.0, "normal"));
    set(&mut h, id, 30, "multiply");
    alt_reset(&mut h);
    assert_eq!(values(&h, id), at(100.0, "normal"));
}

#[test]
fn a_cancelled_fade_is_not_what_reset_returns_to() {
    let mut h = harness();
    let id = invert_and_open_fade(&mut h);
    set(&mut h, id, 20, "screen");
    click(&mut h, "OK");
    let id = invert_and_open_fade(&mut h);
    set(&mut h, id, 70, "darken");
    click(&mut h, "Cancel");
    assert!(!is_open(&h, id), "Cancel closes the dialog");
    let id = invert_and_open_fade(&mut h);
    assert_eq!(values(&h, id), at(20.0, "screen"), "Cancel drops its values");
    set(&mut h, id, 5, "lighten");
    alt_reset(&mut h);
    assert_eq!(values(&h, id), at(20.0, "screen"));
}

#[test]
fn other_filter_dialogs_still_reset_to_their_defaults() {
    let mut h = harness();
    let id = super::open(h.state_mut(), "filter.blur.gaussianBlur").expect("Gaussian Blur opens");
    h.run_steps(4);
    let radius = |h: &Harness<'_, Box<PhotocraftApp>>| h.state().framework.dialogs.get_by_id::<HostedParamDialog>(id).unwrap().fields().number("radius");
    let default = radius(&h);
    let ctx = h.ctx.clone();
    h.state_mut().framework.dialogs.set_json_field(&ctx, id, "radius", json!(42.5)).unwrap();
    h.run_steps(2);
    alt_reset(&mut h);
    assert_eq!(radius(&h), default);
}
