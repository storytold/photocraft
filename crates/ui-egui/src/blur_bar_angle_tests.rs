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

fn angle(h: &Harness<'static, Box<PhotocraftApp>>) -> egui_kittest::Node<'_> {
    h.get_by_role_and_label(Role::SpinButton, "Angle")
}

fn type_angle(h: &mut Harness<'static, Box<PhotocraftApp>>, value: &str) {
    angle(h).click();
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

    type_angle(&mut h, "-47");
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
    let bound = crate::i18n::framework_i18n().format().number_ungrouped(180.0, 0);
    for (typed, expected) in [("200", 180.0), ("-200", -180.0)] {
        type_angle(&mut h, typed);
        assert_eq!(h.state().session.tools.brush.angle, expected);
        let dialogs = h.state().framework.dialogs.open_dialogs();
        assert_eq!(dialogs.len(), 1);
        let message = dialogs[0].fields.as_ref().unwrap()["message"].as_str().unwrap();
        assert!(message.contains(&bound) && message.contains("Closest value inserted"), "{message}");

        h.get_by_role_and_label(Role::Button, "OK").click();
        h.run_steps(4);
        assert!(h.state().framework.dialogs.open_dialogs().is_empty());
        assert_eq!(angle(&h).accesskit_node().numeric_value(), Some(f64::from(expected)));
    }
}
