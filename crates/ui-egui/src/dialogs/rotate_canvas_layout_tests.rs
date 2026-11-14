//! #14462: Image › Image Rotation › Arbitrary opens the reference application's Rotate Canvas
//! dialog: titled "Rotate Canvas", one row with the Angle field (`°`, two decimals) behind a
//! scrubby label (1° a point), then the Clockwise and Counter clockwise radios, OK and Cancel. No
//! slider, no Interpolation control, no Preview, and Alt does not turn Cancel into Reset. The
//! command and its params stay as they are.

use egui::accesskit::Role;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::rotate_canvas_ui::{COMMAND, TITLE};
use crate::PhotocraftApp;

/// The whole app on a 64x32 document with the dialog opened through the menu, drawn a few frames.
fn dialog() -> (Harness<'static, PhotocraftApp>, u64) {
    let mut h = Harness::builder().with_size(vec2(1600.0, 900.0)).with_step_dt(1.0 / 60.0).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.background_jobs = false;
        app.run("file.new", json!({"width": 64, "height": 32, "background": "white"})).unwrap();
        app
    });
    h.run_steps(3);
    let ctx = h.ctx.clone();
    let r = crate::menus::invoke(h.state_mut(), &ctx, COMMAND, json!({})).unwrap();
    let id = r["dialog"].as_u64().expect("the menu opens a dialog");
    h.run_steps(4);
    (h, id)
}

fn field(h: &Harness<'static, PhotocraftApp>, id: u64, key: &str) -> Value {
    h.state().ui.dialogs.iter().find(|d| d.id == id).and_then(|d| d.fields.get(key)).cloned().unwrap_or(Value::Null)
}

fn set(h: &mut Harness<'static, PhotocraftApp>, id: u64, key: &str, v: Value) {
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert(key.into(), v);
    h.run_steps(2);
}

/// Drags the Angle label `dx` points to the right (left when negative), in small moves.
fn scrub(h: &mut Harness<'static, PhotocraftApp>, dx: f32) {
    let label = h.query_all_by_role_and_label(Role::Label, "Angle").next().expect("the Angle label").rect();
    let from = label.center();
    h.event(Event::PointerMoved(from));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    let steps = 10;
    for k in 1..=steps {
        h.event(Event::PointerMoved(from + vec2(dx * k as f32 / steps as f32, 0.0)));
        h.run_steps(1);
    }
    let to: Pos2 = from + vec2(dx, 0.0);
    h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

#[test]
fn the_dialog_is_titled_rotate_canvas() {
    let (h, id) = dialog();
    let d = h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap();
    assert_eq!(crate::dialogs::title(d), TITLE);
    assert!(h.query_by_label("Rotate Canvas").is_some(), "the window shows the title");
    assert!(h.query_by_label("Arbitrary").is_none(), "the old title is gone");
}

#[test]
fn the_controls_are_the_angle_field_two_radios_ok_and_cancel() {
    let (h, id) = dialog();
    let angle = h.get_by_role_and_label(Role::SpinButton, "Angle").rect();
    let cw = h.get_by_role_and_label(Role::RadioButton, "Clockwise").rect();
    let ccw = h.get_by_role_and_label(Role::RadioButton, "Counter clockwise").rect();
    // One row, left to right.
    for r in [cw, ccw] {
        assert!((r.center().y - angle.center().y).abs() < 6.0, "the radios share the Angle row: {r:?} vs {angle:?}");
    }
    assert!(angle.right() <= cw.left() && cw.right() <= ccw.left(), "Angle, Clockwise, Counter clockwise in order");
    h.get_by_role_and_label(Role::Button, "OK");
    h.get_by_role_and_label(Role::Button, "Cancel");
    assert!(h.query_all_by_role(Role::Slider).next().is_none(), "no slider");
    assert!(h.query_by_label("Interpolation").is_none(), "no Interpolation control");
    assert!(h.query_by_label("Direction").is_none(), "no Direction dropdown");
    assert!(h.query_by_label("Preview").is_none(), "no Preview checkbox");
    assert_eq!(field(&h, id, "interpolation"), Value::Null, "the dialog sends no interpolation");
    assert_eq!(field(&h, id, "__preview"), Value::Null, "no preview field");
    assert!(!crate::filter_dialog::PREVIEWED.contains(&COMMAND));
    assert!(crate::filter_dialog::has_dialog(COMMAND), "the menu still opens the dialog");
    // The command keeps its param for scripts and recorded actions.
    let spec = photocraft_engine::commands::find(COMMAND).unwrap();
    assert!(crate::filter_dialog::parse_spec(spec.params).iter().any(|p| p.key == "interpolation"));
}

#[test]
fn the_radios_set_cw_and_ccw_and_ok_runs_them() {
    let (mut h, id) = dialog();
    assert_eq!(field(&h, id, "direction"), json!("cw"), "a fresh dialog is Clockwise");
    h.get_by_role_and_label(Role::RadioButton, "Counter clockwise").click();
    h.run_steps(2);
    assert_eq!(field(&h, id, "direction"), json!("ccw"));
    h.get_by_role_and_label(Role::RadioButton, "Clockwise").click();
    h.run_steps(2);
    assert_eq!(field(&h, id, "direction"), json!("cw"));
    h.get_by_role_and_label(Role::RadioButton, "Counter clockwise").click();
    set(&mut h, id, "angle", json!(90.0));
    let params = crate::filter_dialog::params_of(&h.state_mut().ui.dialog_mut(id).unwrap().fields.clone());
    assert_eq!(params, json!({"angle": 90.0, "direction": "ccw"}), "OK sends the angle and the radio's direction");
    let r = crate::dialogs::confirm(h.state_mut(), id).unwrap();
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(32), Some(64)), "{r}");
}

#[test]
fn a_50_point_scrub_of_the_angle_label_turns_30_into_80() {
    let (mut h, id) = dialog();
    set(&mut h, id, "angle", json!(30.0));
    scrub(&mut h, 50.0);
    assert_eq!(field(&h, id, "angle").as_f64(), Some(80.0));
    scrub(&mut h, -20.0);
    assert_eq!(field(&h, id, "angle").as_f64(), Some(60.0), "dragging left lowers it");
}

#[test]
fn a_scrub_keeps_the_hundredths_and_stops_at_359_99() {
    let (mut h, id) = dialog();
    set(&mut h, id, "angle", json!(33.69));
    scrub(&mut h, 10.0);
    assert_eq!(field(&h, id, "angle").as_f64(), Some(43.69), "1° a point, the hundredths kept");
    set(&mut h, id, "angle", json!(330.0));
    scrub(&mut h, 100.0);
    assert_eq!(field(&h, id, "angle").as_f64(), Some(359.99), "clamped to the field's range");
    set(&mut h, id, "angle", json!(-330.0));
    scrub(&mut h, -100.0);
    assert_eq!(field(&h, id, "angle").as_f64(), Some(-359.99));
}

#[test]
fn the_arrow_keys_still_step_hundredths_and_tenths() {
    let (mut h, id) = dialog();
    set(&mut h, id, "angle", json!(30.0));
    h.get_by_role_and_label(Role::SpinButton, "Angle").click();
    h.run_steps(3);
    h.key_press(Key::ArrowUp);
    h.run_steps(3);
    assert_eq!(field(&h, id, "angle").as_f64(), Some(30.01));
    h.key_press(Key::ArrowUp);
    h.run_steps(3);
    assert_eq!(field(&h, id, "angle").as_f64(), Some(30.02), "a second press");
    h.key_press_modifiers(Modifiers::SHIFT, Key::ArrowUp);
    h.run_steps(3);
    assert_eq!(field(&h, id, "angle").as_f64(), Some(30.12));
}

#[test]
fn alt_keeps_cancel_and_escape_cancels() {
    let (mut h, id) = dialog();
    h.event(Event::ModifiersChanged(Modifiers::ALT));
    h.run_steps(2);
    assert!(h.query_by_label("Reset").is_none(), "Alt does not turn Cancel into Reset");
    h.get_by_role_and_label(Role::Button, "Cancel");
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(1);
    let revision = h.state().session.active().unwrap().revision;
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(h.state().ui.dialogs.iter().all(|d| d.id != id), "Escape closes the dialog");
    assert_eq!(h.state().session.active().unwrap().revision, revision, "and changes nothing");
}

#[test]
fn ok_remembers_angle_and_direction_and_a_recorded_interpolation_still_runs() {
    let (mut h, id) = dialog();
    set(&mut h, id, "angle", json!(12.5));
    h.get_by_role_and_label(Role::RadioButton, "Counter clockwise").click();
    h.run_steps(2);
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    let ctx = h.ctx.clone();
    let r = crate::menus::invoke(h.state_mut(), &ctx, COMMAND, json!({})).unwrap();
    let again = r["dialog"].as_u64().unwrap();
    h.run_steps(2);
    assert_eq!((field(&h, again, "angle"), field(&h, again, "direction")), (json!(12.5), json!("ccw")));
    // A dialog that carries an interpolation (a recorded step shown again) still sends it.
    set(&mut h, again, "interpolation", json!("nearest"));
    let params = crate::filter_dialog::params_of(&h.state_mut().ui.dialog_mut(again).unwrap().fields.clone());
    assert_eq!(params["interpolation"], json!("nearest"));
}
