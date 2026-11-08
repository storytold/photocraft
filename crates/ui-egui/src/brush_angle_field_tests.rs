//! #11811: the Brush options bar has an Angle field after the Smoothing gear and before Pressure
//! for Size: 0° by default, "Set the brush angle", whole degrees from −180 to 180, on the same
//! tip angle as the Brush Settings panel.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;
use crate::theme::ThemeKind;

fn bar(tool: Tool) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = tool;
    let mut h = Harness::builder().with_size(vec2(1600.0, 300.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, ThemeKind::ProMedium);
    h.run_steps(4);
    h
}

fn angle_field<'a>(h: &'a Harness<'static, PhotocraftApp>) -> Option<egui_kittest::Node<'a>> {
    h.query_all_by_role(egui::accesskit::Role::SpinButton).find(|n| n.accesskit_node().description().as_deref() == Some("Set the brush angle"))
}

fn type_angle(h: &mut Harness<'static, PhotocraftApp>, text: &str) -> f32 {
    angle_field(h).expect("the Angle field").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    h.state().session.tools.brush.angle
}

#[test]
fn the_field_sits_between_smoothing_and_pressure_for_size() {
    let h = bar(Tool::Brush);
    let field = angle_field(&h).expect("the Angle field");
    let shown = field.accesskit_node().value().map(|v| v.to_string()).unwrap_or_default();
    assert!(shown.starts_with('0'), "default 0°: {shown}");
    let x = |label: &str| h.get_by_label(label).accesskit_node().bounding_box().unwrap().x0;
    let angle = field.accesskit_node().bounding_box().unwrap().x0;
    assert!(x("Set additional smoothing options") < angle && angle < x("Always use pressure for size"));
    assert_eq!(h.state().session.tools.brush.angle, 0.0);
}

#[test]
fn typed_values_are_whole_degrees_within_range() {
    let mut h = bar(Tool::Brush);
    for (typed, want) in [("400", 180.0), ("-200", -180.0), ("45.5", 45.0), ("-30", -30.0)] {
        assert_eq!(type_angle(&mut h, typed), want, "{typed}");
    }
}

#[test]
fn it_edits_the_brush_settings_panels_angle() {
    let mut h = bar(Tool::Brush);
    h.state_mut().run("tools.setBrush", json!({"brush": {"angle": 60.0}})).unwrap();
    h.run_steps(2);
    let shown = angle_field(&h).unwrap().accesskit_node().value().map(|v| v.to_string()).unwrap_or_default();
    assert!(shown.starts_with("60"), "{shown}");
}

#[test]
fn the_eraser_has_no_angle_field() {
    assert!(angle_field(&bar(Tool::Eraser)).is_none());
}
