//! #11965: on the Pencil's bar, Opacity (1-100), Smoothing (0-100) and Angle (-180 to 180) typed
//! out of range keep the closest value and show the reference's "Closest value inserted" alert,
//! once, with OK; values in range show none.

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::eraser_range_alert_tests::{alerts, bar, ok, type_into};
use super::{ONE_TO_100, ZERO_TO_100};
use crate::PhotocraftApp;
use crate::eraser_ui::ANGLE_RANGE_MESSAGE;
use crate::state::Tool;

fn pencil() -> egui_kittest::Harness<'static, PhotocraftApp> {
    let mut h = bar(Tool::Pencil);
    crate::paint_mouse::sync_tool_brush(h.state_mut());
    h.run_steps(2);
    h
}

/// Types into the Angle field (named by its tooltip), then Tabs out.
fn type_angle(h: &mut egui_kittest::Harness<'static, PhotocraftApp>, text: &str) {
    h.query_all_by_role(Role::SpinButton)
        .find(|n| n.accesskit_node().description().as_deref() == Some("Set the brush angle"))
        .expect("the Angle field")
        .click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

fn values(h: &egui_kittest::Harness<'static, PhotocraftApp>) -> (f32, f32, f32) {
    let b = &h.state().session.tools.brush;
    ((b.opacity * 100.0).round(), (b.smoothing.amount * 100.0).round(), b.angle)
}

#[test]
fn opacity_and_smoothing_clamp_with_their_alerts() {
    for (label, typed, want, alert) in [
        ("Opacity", "0", 1.0, ONE_TO_100),
        ("Opacity", "150", 100.0, ONE_TO_100),
        ("Smoothing", "150", 100.0, ZERO_TO_100),
        ("Smoothing", "-5", 0.0, ZERO_TO_100),
    ] {
        let mut h = pencil();
        type_into(&mut h, label, typed);
        let (o, s, _) = values(&h);
        assert_eq!(if label == "Opacity" { o } else { s }, want, "{label} {typed}");
        assert_eq!(alerts(&h), [alert], "{label} {typed}: the alert, once");
        ok(&mut h);
        assert!(alerts(&h).is_empty(), "{label} {typed}: OK closes it");
        let (o, s, _) = values(&h);
        assert_eq!(if label == "Opacity" { o } else { s }, want, "{label} {typed}: the clamped value stays after OK");
    }
}

#[test]
fn angle_clamps_with_its_alert() {
    for (typed, want) in [("400", 180.0), ("-200", -180.0)] {
        let mut h = pencil();
        type_angle(&mut h, typed);
        assert_eq!(values(&h).2, want, "{typed}");
        assert_eq!(alerts(&h), [ANGLE_RANGE_MESSAGE], "{typed}");
        ok(&mut h);
        assert_eq!(values(&h).2, want);
    }
    assert_eq!(ANGLE_RANGE_MESSAGE, "An integer between -180 ° and 180 ° is required. Closest value inserted.");
}

#[test]
fn values_in_range_show_no_alert_and_the_fields_still_work_after_one() {
    let mut h = pencil();
    type_into(&mut h, "Opacity", "40");
    type_into(&mut h, "Smoothing", "25");
    type_angle(&mut h, "-45");
    assert!(alerts(&h).is_empty(), "{:?}", alerts(&h));
    assert_eq!(values(&h), (40.0, 25.0, -45.0));
    // After an alert and its OK, the keyboard still edits the field.
    type_into(&mut h, "Opacity", "150");
    ok(&mut h);
    type_into(&mut h, "Opacity", "30");
    assert_eq!(values(&h).0, 30.0);
    assert!(alerts(&h).is_empty());
}

#[test]
fn the_alert_is_listed_over_the_control_channel() {
    let mut h = pencil();
    type_into(&mut h, "Smoothing", "150");
    let ctx = h.ctx.clone();
    let (req, _rx) = crate::control::ControlRequest::new("ui.inspect", json!({}));
    let crate::control::Outcome::Done(v) = crate::control::handle(h.state_mut(), &ctx, &req) else { panic!("ui.inspect replies at once") };
    assert!(v.to_string().contains(ZERO_TO_100), "{v}");
}
