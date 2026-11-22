//! #14462: Image › Image Rotation › Arbitrary opens the reference application's Rotate Canvas
//! dialog: titled "Rotate Canvas", one row with the Angle field (`°`, two decimals) behind a
//! scrubby label, then the Clockwise and Counter clockwise radios, OK and Cancel. No slider, no
//! Interpolation control, no Preview, and Alt does not turn Cancel into Reset. The command and its
//! params stay as they are.

use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue};
use egui::accesskit::Role;
use egui::{Event, Key, Modifiers, PointerButton, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::rotate_canvas_ui::COMMAND;
use crate::PhotocraftApp;
use crate::new_layer_ui::testing::{fields, set, top};

type H = Harness<'static, Box<PhotocraftApp>>;

/// The whole app on a 64x32 document with the dialog opened through the menu, drawn a few frames.
fn dialog() -> (H, DialogId) {
    let mut h = Harness::builder().with_size(vec2(1600.0, 900.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), crate::Services::default());
        app.background_jobs = false;
        app.run("file.new", json!({"width": 64, "height": 32, "background": "white"})).unwrap();
        app
    });
    h.run_steps(3);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, COMMAND, json!({})).unwrap();
    let id = top(h.state());
    h.run_steps(4);
    (h, id)
}

fn field(h: &H, id: DialogId, key: &str) -> Option<FieldValue> {
    fields(h.state(), id).get(key).cloned()
}

fn angle(h: &H, id: DialogId) -> Option<f64> {
    fields(h.state(), id).number("angle")
}

fn direction(h: &H, id: DialogId) -> Option<String> {
    fields(h.state(), id).text("direction").map(str::to_owned)
}

fn set_angle(h: &mut H, id: DialogId, v: f64) {
    set(h.state_mut(), id, "angle", json!(v)).unwrap();
    h.run_steps(2);
}

/// The dialog window's rectangle: the area that draws its OK button.
fn dialog_rect(h: &H) -> egui::Rect {
    let ok = h.get_by_role_and_label(Role::Button, "OK").rect();
    h.ctx.layer_id_at(ok.center()).and_then(|layer| h.ctx.memory(|m| m.area_rect(layer.id))).expect("the dialog's window")
}

/// Drags the Angle label `dx` points to the right (left when negative), in small moves.
fn scrub(h: &mut H, dx: f32) {
    let label = h.get_by_role_and_label(Role::Label, "Angle").rect();
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
    let to = from + vec2(dx, 0.0);
    h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

#[test]
fn the_dialog_is_titled_rotate_canvas() {
    let (h, _) = dialog();
    assert!(h.query_all_by_label("Rotate Canvas").next().is_some(), "the window shows the title");
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
    let window = dialog_rect(&h);
    assert!(h.query_all_by_role(Role::Slider).all(|s| !window.intersects(s.rect())), "no slider in the dialog");
    assert!(h.query_by_label("Interpolation").is_none(), "no Interpolation control");
    assert!(h.query_by_label("Preview").is_none(), "no Preview checkbox");
    assert_eq!(field(&h, id, "interpolation"), None, "the dialog sends no interpolation");
    assert!(!crate::filter_dialog::PREVIEWED.contains(&COMMAND));
    assert!(crate::filter_dialog::has_dialog(COMMAND), "the menu still opens the dialog");
    // The command keeps its param for scripts and recorded actions.
    let spec = photocraft_engine::commands::find(COMMAND).unwrap();
    assert!(craft::ui::patterns::form_dialogs::Param::parse_list(spec.params).iter().any(|p| p.key == "interpolation"));
}

#[test]
fn the_radios_set_cw_and_ccw_and_ok_runs_them() {
    let (mut h, id) = dialog();
    assert_eq!(direction(&h, id).as_deref(), Some("cw"), "a fresh dialog is Clockwise");
    h.get_by_role_and_label(Role::RadioButton, "Counter clockwise").click();
    h.run_steps(2);
    assert_eq!(direction(&h, id).as_deref(), Some("ccw"));
    h.get_by_role_and_label(Role::RadioButton, "Clockwise").click();
    h.run_steps(2);
    assert_eq!(direction(&h, id).as_deref(), Some("cw"));
    h.get_by_role_and_label(Role::RadioButton, "Counter clockwise").click();
    h.run_steps(2);
    set_angle(&mut h, id, 90.0);
    let params = crate::filter_dialog::command_params(COMMAND, fields(h.state(), id));
    assert_eq!(params, json!({"angle": 90.0, "direction": "ccw"}), "OK sends the angle and the radio's direction");
    crate::dialogs::testing::confirm(h.state_mut(), id).unwrap();
    h.run_steps(3);
    let st = h.state().session.active().unwrap();
    assert_eq!((st.doc.size.width, st.doc.size.height), (32, 64), "90° Counter clockwise turns 64x32 upright");
}

/// The Angle label scrubs the angle: right raises it, left lowers it, and it keeps its hundredths.
#[test]
fn a_scrub_of_the_angle_label_moves_the_angle() {
    let (mut h, id) = dialog();
    set_angle(&mut h, id, 30.0);
    scrub(&mut h, 50.0);
    let right = angle(&h, id).unwrap();
    assert!(right > 30.0, "dragging right raises it: {right}");
    scrub(&mut h, -20.0);
    let left = angle(&h, id).unwrap();
    assert!(left < right, "dragging left lowers it: {left}");
    assert_eq!((left * 100.0).round() / 100.0, left, "two decimals at most");
}

/// The reference application moves the angle 1° a point of drag on its label (30° and 50 px give 80°), while the arrow
/// keys move it by hundredths. The framework's number field scrubs by its key step, so this waits for a scrub step of
/// its own in craft's `NumberField`.
#[test]
#[ignore = "needs a scrub step apart from the arrow-key step in craft's NumberField"]
fn a_50_point_scrub_of_the_angle_label_turns_30_into_80() {
    let (mut h, id) = dialog();
    set_angle(&mut h, id, 30.0);
    scrub(&mut h, 50.0);
    assert_eq!(angle(&h, id), Some(80.0));
    scrub(&mut h, -20.0);
    assert_eq!(angle(&h, id), Some(60.0), "dragging left lowers it");
}

#[test]
fn a_set_angle_keeps_the_hundredths_and_stops_at_359_99() {
    let (mut h, id) = dialog();
    set_angle(&mut h, id, 33.69);
    assert_eq!(crate::filter_dialog::command_params(COMMAND, fields(h.state(), id))["angle"], json!(33.69));
    // An agent's value reaches the row's key unchecked; OK sends it in the field's range at two decimals.
    for (set_to, sent) in [(400.0, 359.99), (-400.0, -359.99), (12.3456, 12.35)] {
        set_angle(&mut h, id, set_to);
        assert_eq!(crate::filter_dialog::command_params(COMMAND, fields(h.state(), id))["angle"], json!(sent), "{set_to}");
    }
}

#[test]
fn the_arrow_keys_step_hundredths_and_tenths() {
    let (mut h, id) = dialog();
    set_angle(&mut h, id, 30.0);
    h.get_by_role_and_label(Role::SpinButton, "Angle").click();
    h.run_steps(3);
    h.key_press(Key::ArrowUp);
    h.run_steps(3);
    assert_eq!(angle(&h, id), Some(30.01));
    h.key_press(Key::ArrowUp);
    h.run_steps(3);
    assert_eq!(angle(&h, id), Some(30.02), "a second press");
    // Shift moves a tenth, onto the tenths (the framework's large step lands on its grid).
    set_angle(&mut h, id, 30.1);
    h.get_by_role_and_label(Role::SpinButton, "Angle").click();
    h.run_steps(3);
    h.key_press_modifiers(Modifiers::SHIFT, Key::ArrowUp);
    h.run_steps(3);
    assert_eq!(angle(&h, id), Some(30.2));
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
    assert!(h.state().framework.dialogs.open_dialogs().iter().all(|d| d.id != id), "Escape closes the dialog");
    assert_eq!(h.state().session.active().unwrap().revision, revision, "and changes nothing");
}

/// The dialog has no Interpolation row, but values that carry one (a recorded step shown again) still send it.
#[test]
fn a_recorded_interpolation_still_runs() {
    let mut f = DialogFields::new();
    f.set("angle", FieldValue::Number(12.5));
    f.set("direction", FieldValue::Text("ccw".into()));
    f.set("interpolation", FieldValue::Json(json!("nearest")));
    let params = crate::filter_dialog::command_params(COMMAND, &f);
    assert_eq!(params, json!({"angle": 12.5, "direction": "ccw", "interpolation": "nearest"}));
    assert_eq!(params.get("__preview"), None::<&Value>);
}
