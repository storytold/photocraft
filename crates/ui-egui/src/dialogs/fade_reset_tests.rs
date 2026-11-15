//! #13812: Alt + Reset in the Fade dialog goes back to the values the dialog opened with (the last
//! confirmed Fade's), not to 100 % Normal.

use egui_kittest::{Harness, kittest::Queryable};
use serde_json::{Value, json};

use crate::{PhotocraftApp, Services, dialogs::show};

const FADE: &str = "edit.fade";

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width": 40, "height": 30, "background": "white"})).unwrap();
    let h = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h
}

/// Invert, then open Fade from the menu; returns the dialog's id.
fn invert_and_open_fade(h: &mut Harness<'static, PhotocraftApp>) -> u64 {
    h.state_mut().run("image.adjustments.invert", json!({})).unwrap();
    let ctx = h.ctx.clone();
    let id = crate::menus::invoke(h.state_mut(), &ctx, FADE, json!({})).unwrap()["dialog"].as_u64().unwrap();
    h.run_steps(4);
    id
}

fn set(h: &mut Harness<'static, PhotocraftApp>, id: u64, opacity: i64, mode: &str) {
    let d = h.state_mut().ui.dialogs.iter_mut().find(|d| d.id == id).unwrap();
    d.fields.insert("opacity".into(), json!(opacity));
    d.fields.insert("mode".into(), json!(mode));
    h.run_steps(2);
}

/// The dialog's Opacity (as a number: the field stores it whole or not) and Mode.
fn values(h: &Harness<'_, PhotocraftApp>, id: u64) -> (f64, Value) {
    let d = h.state().ui.dialogs.iter().find(|d| d.id == id).expect("the dialog stays open");
    (d.fields["opacity"].as_f64().unwrap(), d.fields["mode"].clone())
}

fn click(h: &mut Harness<'static, PhotocraftApp>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(4);
}

/// Holds Alt, which turns Cancel into Reset, and clicks it.
fn alt_reset(h: &mut Harness<'static, PhotocraftApp>) {
    let alt = egui::Modifiers::ALT;
    h.event(egui::Event::ModifiersChanged(alt));
    h.run_steps(2);
    let at = h.get_by_label("Reset").rect().center();
    h.event(egui::Event::PointerMoved(at));
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: alt });
        h.run_steps(1);
    }
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    h.run_steps(3);
}

#[test]
fn reset_returns_to_the_last_confirmed_values() {
    let mut h = harness();
    let id = invert_and_open_fade(&mut h);
    set(&mut h, id, 20, "screen");
    click(&mut h, "OK");
    assert!(h.state().ui.dialogs.iter().all(|d| d.id != id), "OK closes the dialog");

    let id = invert_and_open_fade(&mut h);
    assert_eq!(values(&h, id), (20.0, json!("screen")), "it opens with the last values");
    set(&mut h, id, 60, "screen");
    alt_reset(&mut h);
    assert_eq!(values(&h, id), (20.0, json!("screen")));
    set(&mut h, id, 45, "multiply");
    alt_reset(&mut h);
    assert_eq!(values(&h, id), (20.0, json!("screen")), "a second Reset too");
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
    assert_eq!(values(&h, id), (100.0, json!("normal")));
}

#[test]
fn on_a_first_open_reset_returns_to_100_normal() {
    let mut h = harness();
    let id = invert_and_open_fade(&mut h);
    assert_eq!(values(&h, id), (100.0, json!("normal")));
    set(&mut h, id, 30, "multiply");
    alt_reset(&mut h);
    assert_eq!(values(&h, id), (100.0, json!("normal")));
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
    let id = invert_and_open_fade(&mut h);
    assert_eq!(values(&h, id), (20.0, json!("screen")), "Cancel drops its values");
    set(&mut h, id, 5, "lighten");
    alt_reset(&mut h);
    assert_eq!(values(&h, id), (20.0, json!("screen")));
}

#[test]
fn other_filter_dialogs_still_reset_to_their_defaults() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    let id = crate::menus::invoke(h.state_mut(), &ctx, "filter.blur.gaussianBlur", json!({})).unwrap()["dialog"].as_u64().unwrap();
    h.run_steps(4);
    let default = h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields["radius"].clone();
    h.state_mut().ui.dialogs.iter_mut().find(|d| d.id == id).unwrap().fields.insert("radius".into(), json!(42.5));
    h.run_steps(2);
    alt_reset(&mut h);
    assert_eq!(h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields["radius"], default);
    assert!(!h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.contains_key("__fadeOpened"));
}
