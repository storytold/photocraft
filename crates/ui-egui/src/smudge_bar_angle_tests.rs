//! Smudge Angle (#12960): the existing brush angle field, after Strength, edits only Smudge.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::app::framework::BuildApp;
use crate::state::Tool;

const ANGLE_RANGE_MESSAGE: &str = "An integer between -180 ° and 180 ° is required. Closest value inserted.";

fn bar(tool: Tool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = tool;
    let mut h = Harness::builder().with_size(vec2(1600.0, 400.0)).build_app(
        |ui, app| {
            crate::panels::options_bar(app, ui);
            crate::dialogs::run(app, ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

fn field<'a>(h: &'a Harness<'static, Box<PhotocraftApp>>) -> Option<egui_kittest::Node<'a>> {
    number_field(h, "Angle")
}

fn number_field<'a>(h: &'a Harness<'static, Box<PhotocraftApp>>, label: &str) -> Option<egui_kittest::Node<'a>> {
    h.query_all_by_role(egui::accesskit::Role::SpinButton).find(|node| node.accesskit_node().label().is_some_and(|name| name.trim_end_matches(':') == label))
}

fn type_angle(h: &mut Harness<'static, Box<PhotocraftApp>>, text: &str) {
    field(h).expect("Smudge Angle").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
}

fn alerts(h: &Harness<'static, Box<PhotocraftApp>>) -> Vec<String> {
    h.state().framework.dialogs.open_dialogs().iter().filter_map(|d| d.fields.as_ref()?.get("message")?.as_str().map(str::to_owned)).collect()
}

#[test]
fn angle_starts_at_zero_between_strength_and_sample_all_layers_with_its_tooltip() {
    let h = bar(Tool::Smudge);
    let angle = field(&h).expect("the Angle field");
    assert_eq!(h.state().session.tools.brush.angle, 0.0);
    assert!(angle.accesskit_node().value().unwrap().starts_with('0'));
    assert_eq!(angle.accesskit_node().description().as_deref(), Some("Set the brush angle"));
    assert!(number_field(&h, "Strength").expect("Strength").rect().right() <= angle.rect().left());
    assert!(angle.rect().right() <= h.get_by_label("Sample All Layers").rect().left());
}

#[test]
fn typed_out_of_range_angles_clamp_to_each_end_and_show_the_exact_ok_alert() {
    let mut h = bar(Tool::Smudge);
    for (typed, expected) in [("200", 180.0), ("-200", -180.0)] {
        type_angle(&mut h, typed);
        assert_eq!(h.state().session.tools.brush.angle, expected);
        assert_eq!(alerts(&h), [ANGLE_RANGE_MESSAGE]);
        assert!(h.query_by_label(ANGLE_RANGE_MESSAGE).is_some());
        h.get_by_label("OK").click();
        h.run_steps(3);
        assert!(alerts(&h).is_empty());
        assert!(field(&h).unwrap().accesskit_node().value().unwrap().starts_with(expected.to_string().as_str()));
    }
}

#[test]
fn in_range_edits_are_whole_degrees_and_share_the_brush_settings_value() {
    let mut h = bar(Tool::Smudge);
    h.state_mut().run("tools.setBrush", json!({"brush": {"angle": 60.0}})).unwrap();
    h.run_steps(2);
    assert!(field(&h).unwrap().accesskit_node().value().unwrap().starts_with("60"));
    type_angle(&mut h, "45.5");
    assert_eq!(h.state_mut().run("brush.get", json!({})).unwrap()["angle"], json!(45.0));
    type_angle(&mut h, "-75");
    assert_eq!(h.state().session.tools.brush.angle, -75.0, "the Brush Settings panel reads this same brush");
    assert!(alerts(&h).is_empty());
    for (typed, expected) in [("180.5", 180.0), ("-180.5", -180.0)] {
        type_angle(&mut h, typed);
        assert_eq!(h.state().session.tools.brush.angle, expected);
        assert!(alerts(&h).is_empty(), "a fraction discarded inside the range is not an out-of-range integer");
    }
}

#[test]
fn smudge_keeps_its_own_angle_across_tool_switches_and_reset_tool_restores_zero() {
    let mut h = bar(Tool::Smudge);
    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(3);
    h.state_mut().run("tools.setBrush", json!({"brush": {"angle": -20.0}})).unwrap();
    h.run_steps(2);
    h.state_mut().ui.tool = Tool::Smudge;
    h.run_steps(3);
    assert_eq!(h.state().session.tools.brush.angle, 0.0, "Smudge starts with its own tip");
    type_angle(&mut h, "30");
    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(3);
    assert_eq!(h.state().session.tools.brush.angle, -20.0);
    h.state_mut().ui.tool = Tool::Smudge;
    h.run_steps(3);
    assert_eq!(h.state().session.tools.brush.angle, 30.0);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "tool.reset", json!({})).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().session.tools.brush.angle, 0.0);
}

#[test]
fn blur_still_has_no_angle_and_sharpen_keeps_its_existing_angle() {
    let blur = bar(Tool::Blur);
    assert!(field(&blur).is_none());
    assert!(number_field(&blur, "Strength").is_some());
    assert!(blur.query_by_label("Sample All Layers").is_some());

    let mut sharpen = bar(Tool::Sharpen);
    assert!(field(&sharpen).is_some());
    assert!(number_field(&sharpen, "Strength").is_some());
    assert!(sharpen.query_by_label("Sample All Layers").is_some());
    type_angle(&mut sharpen, "35");
    assert_eq!(sharpen.state().session.tools.brush.angle, 35.0);
    assert!(alerts(&sharpen).is_empty());
}
