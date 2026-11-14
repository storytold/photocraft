//! #12629: the Art History Brush's options bar has a Pressure for Opacity toggle right after
//! Opacity, and an Angle field and a Pressure for Size toggle after Tolerance, with the reference
//! application's tooltips, off / 0° by default and after Reset Tool. They edit the tool's own
//! brush, not the History Brush's or the Brush's, and the bar has no Flow and no Airbrush.

use egui::accesskit::Role;
use egui::{Modifiers, Rect};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::eraser_range_alert_tests::bar;
use crate::PhotocraftApp;
use crate::state::Tool;

const PRESSURE_OPACITY: &str = "Always use pressure for opacity";
const PRESSURE_SIZE: &str = "Always use pressure for size";

fn rect(h: &Harness<'static, PhotocraftApp>, label: &str) -> Rect {
    h.get_by_label(label).rect()
}

/// The field captioned `label` ("Opacity", "Tolerance"), by its caption's name.
fn field<'a>(h: &'a Harness<'static, PhotocraftApp>, label: &str) -> Option<egui_kittest::Node<'a>> {
    let label = format!("{label}:");
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some(label.as_str()))
}

fn angle_field<'a>(h: &'a Harness<'static, PhotocraftApp>) -> Option<egui_kittest::Node<'a>> {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().description().as_deref() == Some("Set the brush angle"))
}

fn type_angle(h: &mut Harness<'static, PhotocraftApp>, text: &str) {
    angle_field(h).expect("the Angle field").click();
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
}

/// The live brush's Pressure for Opacity, angle and Pressure for Size.
fn controls(app: &PhotocraftApp) -> (bool, f32, bool) {
    let b = &app.session.tools.brush;
    (b.pressure_opacity, b.angle, b.pressure_size)
}

fn switch(h: &mut Harness<'static, PhotocraftApp>, tool: Tool) {
    h.state_mut().ui.tool = tool;
    h.run_steps(3);
}

#[test]
fn pressure_for_opacity_follows_opacity_and_angle_and_pressure_for_size_follow_tolerance() {
    let h = bar(Tool::ArtHistoryBrush);
    let opacity = field(&h, "Opacity").expect("the Opacity field").rect();
    let pressure_opacity = rect(&h, PRESSURE_OPACITY);
    let style = h.query_all_by_role(Role::ComboBox).next().expect("the Style dropdown").rect();
    assert!(opacity.right() <= pressure_opacity.left() && pressure_opacity.right() <= style.left(), "Opacity, Pressure for Opacity, Style");
    let angle = angle_field(&h).expect("the Angle field").rect();
    // The Tolerance field takes its caption's name.
    let tolerance = rect(&h, "Tolerance");
    let pressure_size = rect(&h, PRESSURE_SIZE);
    assert!(tolerance.right() <= angle.left() && angle.right() <= pressure_size.left(), "Tolerance, Angle, Pressure for Size");
}

#[test]
fn the_controls_carry_the_reference_tooltips() {
    let h = bar(Tool::ArtHistoryBrush);
    let tip = |label: &str| h.get_by_label(label).accesskit_node().description().map(|d| d.to_string());
    assert_eq!(tip(PRESSURE_OPACITY).as_deref(), Some("Always use Pressure for Opacity. When off, Brush Preset controls pressure"));
    assert_eq!(tip(PRESSURE_SIZE).as_deref(), Some("Always use Pressure for Size. When off, Brush Preset controls pressure"));
    assert!(angle_field(&h).is_some(), "the Angle field says \"Set the brush angle\"");
}

#[test]
fn the_controls_are_off_and_0_by_default_and_after_reset_tool() {
    let mut h = bar(Tool::ArtHistoryBrush);
    assert_eq!(controls(h.state()), (false, 0.0, false));
    assert!(angle_field(&h).and_then(|n| n.accesskit_node().numeric_value()) == Some(0.0), "the field shows 0°");
    h.get_by_label(PRESSURE_OPACITY).click();
    h.run_steps(2);
    h.get_by_label(PRESSURE_SIZE).click();
    h.run_steps(2);
    type_angle(&mut h, "45");
    assert_eq!(controls(h.state()), (true, 45.0, true));
    crate::burn_default_tip_tests::invoke(h.state_mut(), "tool.reset", json!({}));
    h.run_steps(3);
    assert_eq!(controls(h.state()), (false, 0.0, false));
}

#[test]
fn they_edit_the_art_history_brush_only() {
    let mut h = bar(Tool::Brush);
    // Each tool keeps its own brush from its first use on.
    switch(&mut h, Tool::HistoryBrush);
    switch(&mut h, Tool::ArtHistoryBrush);
    h.get_by_label(PRESSURE_OPACITY).click();
    h.run_steps(2);
    h.get_by_label(PRESSURE_SIZE).click();
    h.run_steps(2);
    type_angle(&mut h, "-30");
    assert_eq!(controls(h.state()), (true, -30.0, true));
    for other in [Tool::HistoryBrush, Tool::Brush] {
        switch(&mut h, other);
        assert_eq!(controls(h.state()), (false, 0.0, false), "{other:?} keeps its own brush");
    }
    switch(&mut h, Tool::ArtHistoryBrush);
    assert_eq!(controls(h.state()), (true, -30.0, true), "the Art History Brush keeps its settings");
}

#[test]
fn the_bar_has_no_flow_and_no_airbrush() {
    let h = bar(Tool::ArtHistoryBrush);
    assert!(field(&h, "Flow").is_none(), "no Flow field");
    assert!(h.query_by_label("Enable airbrush-style build-up effects").is_none(), "no Airbrush toggle");
}
