//! #11964: the Pencil's options bar has the brush Angle field (an angle icon and whole degrees from
//! −180 to 180, 0° by default) after the Smoothing gear and before Auto Erase, on the same tip
//! angle the Brush Settings panel edits.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;
use crate::theme::ThemeKind;

fn bar(theme: ThemeKind) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = Tool::Pencil;
    crate::paint_mouse::sync_tool_brush(&mut app);
    let mut h = Harness::builder().with_size(vec2(1800.0, 300.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, theme);
    h.run_steps(4);
    h
}

fn angle_field<'a>(h: &'a Harness<'static, PhotocraftApp>) -> egui_kittest::Node<'a> {
    h.query_all_by_role(egui::accesskit::Role::SpinButton)
        .find(|n| n.accesskit_node().description().as_deref() == Some("Set the brush angle"))
        .expect("the Angle field")
}

fn shown(h: &Harness<'static, PhotocraftApp>) -> String {
    angle_field(h).accesskit_node().value().map(|v| v.to_string()).unwrap_or_default()
}

fn x0(h: &Harness<'static, PhotocraftApp>, label: &str) -> f64 {
    h.query_all_by_label(label).next().unwrap_or_else(|| panic!("{label}")).accesskit_node().bounding_box().expect(label).x0
}

fn type_angle(h: &mut Harness<'static, PhotocraftApp>, text: &str) -> f32 {
    angle_field(h).click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    h.state().session.tools.brush.angle
}

#[test]
fn the_field_sits_after_the_smoothing_gear_and_before_auto_erase_at_0() {
    for theme in [ThemeKind::ProMedium, ThemeKind::Studio] {
        let h = bar(theme);
        let angle = angle_field(&h).accesskit_node().bounding_box().unwrap().x0;
        assert!(x0(&h, "Set additional smoothing options") < angle && angle < x0(&h, "Auto Erase"), "{theme:?}: gear < Angle < Auto Erase");
        assert!(shown(&h).starts_with('0'), "{theme:?}: default 0°, {}", shown(&h));
    }
}

#[test]
fn typed_values_are_whole_degrees_clamped_to_plus_minus_180() {
    let mut h = bar(ThemeKind::ProMedium);
    assert_eq!(type_angle(&mut h, "45.5"), 45.0);
    assert_eq!(type_angle(&mut h, "-30"), -30.0);
    assert_eq!(type_angle(&mut h, "400"), 180.0);
    assert!(shown(&h).starts_with("180"), "{}", shown(&h));
    assert_eq!(type_angle(&mut h, "-200"), -180.0);
}

#[test]
fn the_field_and_the_brush_settings_panel_share_the_pencils_angle() {
    let mut h = bar(ThemeKind::ProMedium);
    assert_eq!(type_angle(&mut h, "30"), 30.0);
    assert_eq!(h.state_mut().run("brush.get", json!({})).unwrap()["angle"], json!(30.0), "the tip angle the panel reads");
    // A change from the panel (it writes the same brush field) shows in the field.
    h.state_mut().session.tools.brush.angle = -75.0;
    h.run_steps(2);
    assert!(shown(&h).starts_with("-75"), "{}", shown(&h));
    // It's the Pencil's own: the Brush keeps its angle.
    h.state_mut().ui.tool = Tool::Brush;
    crate::paint_mouse::sync_tool_brush(h.state_mut());
    assert_eq!(h.state().session.tools.brush.angle, 0.0);
    h.state_mut().ui.tool = Tool::Pencil;
    crate::paint_mouse::sync_tool_brush(h.state_mut());
    assert_eq!(h.state().session.tools.brush.angle, -75.0);
}
