//! Blur's options bar edits its own brush angle and reports an out-of-range entry.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

fn blur_bar() -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = Tool::Blur;
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(egui::vec2(1200.0, 300.0)).build_ui_state(
        |ui, app| {
            crate::panels::options_bar(app, ui);
            crate::dialogs::run(app, ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

fn angle<'a>(h: &'a Harness<'static, Box<PhotocraftApp>>) -> egui_kittest::Node<'a> {
    h.get_by_role_and_label(Role::SpinButton, "Angle")
}

fn type_number(h: &mut Harness<'static, Box<PhotocraftApp>>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::SpinButton, label).click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(value.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
}

#[test]
fn the_blur_bar_reads_and_edits_the_same_angle_as_brush_settings() {
    let mut h = blur_bar();
    let strength = h.get_by_role_and_label(Role::SpinButton, "Strength").rect().center().x;
    let sample = h.get_by_role_and_label(Role::CheckBox, "Sample All Layers").rect().center().x;
    assert!(strength < angle(&h).rect().center().x && angle(&h).rect().center().x < sample);
    assert_eq!(angle(&h).accesskit_node().numeric_value(), Some(0.0));

    // The Brush Settings panel writes its edits through this same command.
    h.state_mut().run("tools.setBrush", json!({"brush": {"angle": 35.0}})).unwrap();
    h.run_steps(2);
    assert_eq!(angle(&h).accesskit_node().numeric_value(), Some(35.0));

    type_number(&mut h, "Angle", "-47");
    assert_eq!(h.state().session.tools.brush.angle, -47.0);
    assert!(h.state().framework.dialogs.open_dialogs().is_empty());

    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(3);
    h.state_mut().run("tools.setBrush", json!({"brush": {"angle": 12.0}})).unwrap();
    h.state_mut().ui.tool = Tool::Blur;
    h.run_steps(3);
    assert_eq!(angle(&h).accesskit_node().numeric_value(), Some(-47.0));
    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(3);
    assert_eq!(h.state().session.tools.brush.angle, 12.0);
}

#[test]
fn entering_an_angle_past_either_limit_keeps_the_limit_and_shows_one_alert() {
    let mut h = blur_bar();
    for (typed, expected) in [("200", 180.0), ("-200", -180.0)] {
        type_number(&mut h, "Angle", typed);
        assert_eq!(h.state().session.tools.brush.angle, expected);
        let dialogs = h.state().framework.dialogs.open_dialogs();
        assert_eq!(dialogs.len(), 1);
        let message = dialogs[0].fields.as_ref().unwrap()["message"].as_str().unwrap();
        assert_eq!(message, "Invalid numeric entry. An integer between -180 ° and 180 ° is required. Restored last valid value.");
        assert_eq!(h.get_by_role(Role::AlertDialog).accesskit_node().description().as_deref(), Some(message));

        h.get_by_role_and_label(Role::Button, "OK").click();
        h.run_steps(4);
        assert!(h.state().framework.dialogs.open_dialogs().is_empty());
        assert_eq!(angle(&h).accesskit_node().numeric_value(), Some(f64::from(expected)));
    }
}

#[test]
fn blur_strength_still_shows_the_standard_range_alert() {
    let mut h = blur_bar();
    type_number(&mut h, "Strength", "0");
    assert_eq!(h.state().ui.tool_options.strength, 1.0);
    let dialogs = h.state().framework.dialogs.open_dialogs();
    assert_eq!(dialogs.len(), 1);
    assert!(dialogs[0].fields.as_ref().unwrap()["message"].as_str().unwrap().contains("Closest value inserted"));
}

#[test]
fn a_standard_alert_does_not_replace_the_angle_alert() {
    let mut h = blur_bar();
    type_number(&mut h, "Angle", "200");
    let angle = h.state().framework.dialogs.open_dialogs()[0].id;

    // Strength uses a regular hosted alert, which can open in the same options-bar pass.
    let standard = crate::dialogs::alerts::alert(h.state_mut(), "A standard alert");
    let ids: Vec<_> = h.state().framework.dialogs.open_dialogs().iter().map(|dialog| dialog.id).collect();
    assert_eq!(ids, [angle, standard]);
}
