//! #12946: the Sharpen tool's options bar has the brush Angle field right after Strength, with the
//! reference application's tooltip, 0° by default, whole degrees -180..180 with the reference's
//! alert past that range. It edits the Sharpen tool's own brush tip angle, the value the Brush
//! Settings panel shows.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

type H = Harness<'static, Box<PhotocraftApp>>;

/// The options bar with `tool` active, and the dialog host its alerts open on.
fn bar(tool: Tool) -> H {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = tool;
    crate::theme::install_fonts(app.ctx());
    crate::theme::apply(app.ctx(), &crate::theme::AppTheme::pro());
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1800.0, 300.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        move |ui, app: &mut Box<PhotocraftApp>| {
            crate::panels::options_bar(app, ui);
            crate::dialogs::run(app, ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

/// The number field named `label`, with or without its caption's colon.
fn field<'a>(h: &'a H, label: &str) -> Option<egui_kittest::Node<'a>> {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().is_some_and(|l| l.trim_end_matches(':') == label))
}

fn shown_angle(h: &H) -> Option<f64> {
    field(h, "Angle").and_then(|n| n.accesskit_node().numeric_value())
}

fn type_angle(h: &mut H, text: &str) {
    field(h, "Angle").expect("the Angle field").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
}

/// The messages of the open framework alerts.
fn alerts(h: &H) -> Vec<String> {
    h.state().framework.dialogs.open_dialogs().iter().filter_map(|d| d.fields.as_ref()?.get("message")?.as_str().map(str::to_string)).collect()
}

#[test]
fn the_angle_field_follows_strength_at_0_with_the_reference_tooltip() {
    let h = bar(Tool::Sharpen);
    let strength = field(&h, "Strength").expect("the Strength field").rect();
    let angle = field(&h, "Angle").expect("the Angle field");
    let sample_all = h.get_by_label("Sample All Layers").rect();
    assert!(strength.right() <= angle.rect().left() && angle.rect().right() <= sample_all.left(), "Strength, Angle, Sample All Layers");
    assert_eq!(angle.accesskit_node().description().as_deref(), Some("Set the brush angle"));
    assert_eq!(shown_angle(&h), Some(0.0), "the field shows 0°");
}

#[test]
fn a_typed_angle_past_the_range_keeps_the_closest_with_the_reference_alert() {
    // (typed, kept, alerts)
    for (typed, kept, alerted) in [("200", 180.0, true), ("-200", -180.0, true), ("45", 45.0, false)] {
        let mut h = bar(Tool::Sharpen);
        type_angle(&mut h, typed);
        assert_eq!(h.state().session.tools.brush.angle, kept, "{typed}: the closest value is kept");
        let shown = alerts(&h);
        if !alerted {
            assert!(shown.is_empty(), "{typed}: no alert, got {shown:?}");
            continue;
        }
        assert_eq!(shown, ["An integer between -180 ° and 180 ° is required. Closest value inserted."], "{typed}");
        h.get_by_label("OK").click();
        h.run_steps(4);
        assert!(alerts(&h).is_empty(), "{typed}: OK closes the alert");
        assert_eq!(h.state().session.tools.brush.angle, kept, "{typed}: the value stays after OK");
    }
}

#[test]
fn the_field_and_the_brush_settings_share_the_sharpen_tools_own_angle() {
    let mut h = bar(Tool::Brush);
    // Sharpen's brush exists from its first use on; only then does the Brush get its own angle.
    h.state_mut().ui.tool = Tool::Sharpen;
    h.run_steps(3);
    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(3);
    h.state_mut().run("tools.setBrush", json!({"brush": {"angle": -20.0}})).unwrap();
    h.state_mut().ui.tool = Tool::Sharpen;
    h.run_steps(3);
    assert_eq!(shown_angle(&h), Some(0.0), "Sharpen shows its own brush's angle, not the Brush's");
    // The Brush Settings panel's edit, through the command it sends.
    h.state_mut().run("tools.setBrush", json!({"brush": {"angle": 60.0}})).unwrap();
    h.run_steps(2);
    assert_eq!(shown_angle(&h), Some(60.0), "the field shows the panel's angle");
    type_angle(&mut h, "45");
    assert_eq!(h.state().session.tools.brush.angle, 45.0, "the field sets the tip angle");
    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(3);
    assert_eq!(h.state().session.tools.brush.angle, -20.0, "the Brush keeps its own angle");
    h.state_mut().ui.tool = Tool::Sharpen;
    h.run_steps(3);
    assert_eq!(shown_angle(&h), Some(45.0), "Sharpen keeps its own angle");
}
