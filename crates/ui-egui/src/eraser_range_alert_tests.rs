//! #11938: on the Eraser's bar, Opacity, Flow and Smoothing typed out of range keep the closest
//! value and show the reference's "Closest value inserted" alert once, with OK; values in range
//! show none. The Brush's bar (not measured) keeps today's silent clamp.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::{ONE_TO_100, ZERO_TO_100};
use crate::PhotocraftApp;
use crate::state::{DialogKind, Tool};

fn bar(tool: Tool) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = tool;
    let mut h = Harness::builder().with_size(vec2(1800.0, 300.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h
}

/// Types `text` into the field named `label`, then Tabs out of it.
fn type_into(h: &mut Harness<'static, PhotocraftApp>, label: &str, text: &str) {
    // The fields take their captions' names ("Opacity:").
    let label = &format!("{label}:");
    h.query_all_by_role(Role::SpinButton)
        .find(|n| n.accesskit_node().label().as_deref() == Some(label.as_str()))
        .unwrap_or_else(|| panic!("the {label} field"))
        .click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

fn alerts(h: &Harness<'static, PhotocraftApp>) -> Vec<String> {
    h.state().ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| d.fields.get("message")?.as_str().map(str::to_owned)).collect()
}

/// OK on the alert, as `ui.dialog.confirm` presses it (this harness draws only the bar).
fn ok(h: &mut Harness<'static, PhotocraftApp>) {
    let id = h.state().ui.dialogs.iter().find(|d| d.kind == DialogKind::Error).map(|d| d.id).expect("an alert");
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    h.run_steps(3);
}

fn brush(h: &Harness<'static, PhotocraftApp>) -> (f32, f32, f32) {
    let b = &h.state().session.tools.brush;
    ((b.opacity * 100.0).round(), (b.flow * 100.0).round(), (b.smoothing.amount * 100.0).round())
}

#[test]
fn eraser_opacity_flow_and_smoothing_clamp_with_the_alert() {
    for (label, typed, want, alert) in [
        ("Opacity", "150", 100.0, ONE_TO_100),
        ("Opacity", "0", 1.0, ONE_TO_100),
        ("Flow", "150", 100.0, ONE_TO_100),
        ("Flow", "0", 1.0, ONE_TO_100),
        ("Smoothing", "150", 100.0, ZERO_TO_100),
        ("Smoothing", "-5", 0.0, ZERO_TO_100),
    ] {
        let mut h = bar(Tool::Eraser);
        type_into(&mut h, label, typed);
        let (o, f, s) = brush(&h);
        let got = match label {
            "Opacity" => o,
            "Flow" => f,
            _ => s,
        };
        assert_eq!(got, want, "{label} {typed}");
        assert_eq!(alerts(&h), [alert], "{label} {typed}: the alert, once");
        ok(&mut h);
        assert!(alerts(&h).is_empty(), "{label} {typed}: OK closes it");
        let (o, f, s) = brush(&h);
        assert_eq!(
            match label {
                "Opacity" => o,
                "Flow" => f,
                _ => s,
            },
            want,
            "{label}: the clamped value is kept"
        );
    }
}

#[test]
fn values_in_range_show_no_alert_and_the_field_keeps_working() {
    let mut h = bar(Tool::Eraser);
    type_into(&mut h, "Opacity", "40");
    assert_eq!(brush(&h).0, 40.0);
    assert!(alerts(&h).is_empty());
    // After an alert the field still takes the keyboard.
    type_into(&mut h, "Flow", "150");
    ok(&mut h);
    type_into(&mut h, "Flow", "55");
    assert_eq!(brush(&h).1, 55.0);
    assert!(alerts(&h).is_empty());
}

#[test]
fn the_alert_is_a_dialog_the_control_channel_lists() {
    let mut h = bar(Tool::Eraser);
    type_into(&mut h, "Smoothing", "150");
    let dialogs = serde_json::to_value(&h.state().ui.dialogs).unwrap();
    assert!(dialogs.to_string().contains(ZERO_TO_100), "{dialogs}");
}

#[test]
fn the_brush_bar_keeps_todays_silent_clamp() {
    let mut h = bar(Tool::Brush);
    type_into(&mut h, "Opacity", "150");
    assert_eq!(brush(&h).0, 100.0);
    assert!(alerts(&h).is_empty(), "not measured for the Brush: no alert");
}

/// A value typed past the range and left by switching tools (the click on the toolbar takes the
/// keyboard) shows the stored value when the Eraser comes back, not the typed one.
#[test]
fn a_typed_value_left_by_a_tool_switch_shows_the_stored_value() {
    let mut h = bar(Tool::Eraser);
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Opacity:")).expect("the Opacity field").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("150".into()));
    h.run_steps(1);
    let switch = |h: &mut Harness<'static, PhotocraftApp>, tool: Tool| {
        h.state_mut().ui.tool = tool;
        crate::paint_mouse::sync_tool_brush(h.state_mut());
    };
    switch(&mut h, Tool::Brush);
    h.ctx.memory_mut(|m| {
        if let Some(id) = m.focused() {
            m.surrender_focus(id);
        }
    });
    h.run_steps(3);
    switch(&mut h, Tool::Eraser);
    h.run_steps(1);
    let shown = h
        .query_all_by_role(Role::SpinButton)
        .find(|n| n.accesskit_node().label().as_deref() == Some("Opacity:"))
        .and_then(|n| n.accesskit_node().numeric_value());
    assert_eq!(brush(&h).0, 100.0, "the stored value");
    assert_eq!(shown, Some(100.0), "the field shows the stored value");
}
