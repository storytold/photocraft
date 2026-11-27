//! #12106: Healing's bar opens Clone Source and edits only its own brush's angle and pressure.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::app::framework::BuildApp as _;
use crate::state::Tool;
use crate::theme::AppTheme;

const PANEL: &str = "Toggle the Clone Source panel";
const SETTINGS: &str = "Toggle the Brush Settings panel";
const PRESSURE: &str = "Always use pressure for size";
const PRESSURE_TIP: &str = "Always use Pressure for Size. When off, Brush Preset controls pressure";
const ANGLE_ALERT: &str = "An integer between -180 ° and 180 ° is required. Closest value inserted.";

fn bar(tool: Tool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = tool;
    let mut h = Harness::builder().with_size(vec2(1800.0, 300.0)).build_app(
        |ui, app| {
            crate::panels::options_bar(app, ui);
            crate::dialogs::run(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, AppTheme::pro_medium());
    h.run_steps(4);
    h
}

fn angle<'a>(h: &'a Harness<'static, Box<PhotocraftApp>>) -> egui_kittest::Node<'a> {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().description().as_deref() == Some("Set the brush angle")).expect("Healing Angle")
}

fn has_control(h: &Harness<'static, Box<PhotocraftApp>>, role: Role, label: &str) -> bool {
    h.query_all_by_role(role).any(|node| node.accesskit_node().label().is_some_and(|name| name.trim_end_matches(':') == label))
}

fn clone_source_is_on(h: &Harness<'static, Box<PhotocraftApp>>) -> bool {
    h.get_by_role_and_label(Role::Button, PANEL).accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
}

fn type_angle(h: &mut Harness<'static, Box<PhotocraftApp>>, text: &str) {
    angle(h).click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
}

fn alerts(h: &Harness<'static, Box<PhotocraftApp>>) -> Vec<String> {
    h.state().framework.dialogs.open_dialogs().iter().filter_map(|d| d.fields.as_ref()?.get("message")?.as_str().map(str::to_owned)).collect()
}

#[test]
fn healing_bar_orders_source_angle_and_pressure_without_extra_brush_controls() {
    let h = bar(Tool::Healing);
    let picker = h.get_by_label("Brush Preset picker").rect();
    let panel = h.get_by_role_and_label(Role::Button, PANEL).rect();
    let mode = h
        .query_all_by_role(Role::ComboBox)
        .find(|n| n.accesskit_node().label().is_some_and(|label| label.trim_end_matches(':') == "Mode"))
        .expect("Mode")
        .rect();
    let sample = h.query_all_by_role(Role::ComboBox).find(|n| n.value().as_deref() == Some("Current Layer")).expect("Sample").rect();
    let field = angle(&h).rect();
    let pressure = h.get_by_role_and_label(Role::Button, PRESSURE);
    assert!(picker.right() <= panel.left() && panel.right() <= mode.left(), "picker, Clone Source, Mode");
    assert!(sample.right() <= field.left() && field.right() <= pressure.rect().left(), "Sample, Angle, Pressure");
    assert_eq!(h.state().session.tools.brush.angle, 0.0);
    assert!(!h.state().session.tools.brush.pressure_size);
    assert_eq!(pressure.accesskit_node().description().as_deref(), Some(PRESSURE_TIP));
    assert!(!has_control(&h, Role::Button, SETTINGS));
    for (role, label) in
        [(Role::SpinButton, "Opacity"), (Role::SpinButton, "Flow"), (Role::SpinButton, "Smoothing"), (Role::Button, "Always use pressure for opacity")]
    {
        assert!(!has_control(&h, role, label), "Healing has no {label}");
    }
}

#[test]
fn clone_source_toggle_uses_the_existing_panel_command_in_both_directions() {
    let mut h = bar(Tool::Healing);
    assert!(!crate::panel_docking::is_shown(h.state(), "cloneSource"));
    assert!(!clone_source_is_on(&h));
    h.get_by_role_and_label(Role::Button, PANEL).click();
    h.run_steps(3);
    assert!(crate::panel_docking::is_shown(h.state(), "cloneSource"));
    assert!(clone_source_is_on(&h));
    assert!(!crate::panel_docking::is_shown(h.state(), "brushSettings"));
    h.get_by_role_and_label(Role::Button, PANEL).click();
    h.run_steps(3);
    assert!(!crate::panel_docking::is_shown(h.state(), "cloneSource"));
    assert!(!clone_source_is_on(&h));
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "window.panel.cloneSource", json!({"show": true})).unwrap();
    h.run_steps(3);
    assert!(clone_source_is_on(&h));
    h.get_by_role_and_label(Role::Button, PANEL).click();
    h.run_steps(3);
    assert!(!crate::panel_docking::is_shown(h.state(), "cloneSource"), "the button reads the panel's current state");
    assert!(!clone_source_is_on(&h));
    assert!(!h.state().ui.status_error, "{}", h.state().ui.status);
}

#[test]
fn healing_angle_truncates_fractions_and_clamps_typed_limits_with_one_ok_alert() {
    for (text, want, alert) in
        [("45.5", 45.0, false), ("-45.5", -45.0, false), ("180.5", 180.0, false), ("-180.5", -180.0, false), ("400", 180.0, true), ("-400", -180.0, true)]
    {
        let mut h = bar(Tool::Healing);
        type_angle(&mut h, text);
        assert_eq!(h.state().session.tools.brush.angle, want, "{text}");
        assert_eq!(angle(&h).accesskit_node().numeric_value(), Some(f64::from(want)));
        if alert {
            assert_eq!(alerts(&h), vec![ANGLE_ALERT.to_string()], "{text}");
            let id = h
                .state()
                .framework
                .dialogs
                .open_dialogs()
                .into_iter()
                .find(|d| d.fields.as_ref().and_then(|f| f.get("message")).and_then(|v| v.as_str()) == Some(ANGLE_ALERT))
                .expect("range alert")
                .id;
            let ctx = h.ctx.clone();
            crate::dialogs::confirm(h.state_mut(), &ctx, id).unwrap();
            h.run_steps(3);
            assert!(alerts(&h).is_empty(), "OK closes the alert");
            assert_eq!(h.state().session.tools.brush.angle, want);
        } else {
            assert!(alerts(&h).is_empty(), "{text}");
        }
        let got = h.state_mut().run("brush.get", json!({})).unwrap();
        assert_eq!(got["angle"], json!(want), "Brush Settings reads the bar's brush");
    }
}

#[test]
fn healing_angle_steps_stop_at_the_limit_without_a_typing_alert() {
    let mut h = bar(Tool::Healing);
    type_angle(&mut h, "179");
    angle(&h).click();
    h.run_steps(1);
    for _ in 0..3 {
        h.key_press(egui::Key::ArrowUp);
        h.run_steps(1);
    }
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(h.state().session.tools.brush.angle, 180.0);
    assert!(alerts(&h).is_empty());
}

#[test]
fn healing_angle_pressure_and_reset_preserve_brush_clone_and_pattern_settings() {
    let mut h = bar(Tool::Brush);
    for (tool, value, pressure) in [(Tool::Brush, -20.0, true), (Tool::CloneStamp, 60.0, false), (Tool::PatternStamp, 90.0, true)] {
        h.state_mut().ui.tool = tool;
        h.run_steps(3);
        h.state_mut().run("tools.setBrush", json!({"brush": {"angle": value, "pressureSize": pressure}})).unwrap();
        h.run_steps(2);
    }
    h.state_mut().ui.tool = Tool::Healing;
    h.run_steps(3);
    type_angle(&mut h, "30");
    h.get_by_role_and_label(Role::Button, PRESSURE).click();
    h.run_steps(2);
    let got = h.state_mut().run("brush.get", json!({})).unwrap();
    assert_eq!(got["angle"], json!(30.0));
    assert_eq!(got["pressureSize"], json!(true));
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "tool.reset", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().session.tools.brush.angle, 0.0);
    assert!(!h.state().session.tools.brush.pressure_size);
    for (tool, value, pressure) in [(Tool::Brush, -20.0, true), (Tool::CloneStamp, 60.0, false), (Tool::PatternStamp, 90.0, true)] {
        h.state_mut().ui.tool = tool;
        h.run_steps(3);
        assert_eq!((h.state().session.tools.brush.angle, h.state().session.tools.brush.pressure_size), (value, pressure), "{tool:?}");
    }
    h.state_mut().ui.tool = Tool::Healing;
    h.run_steps(3);
    assert_eq!((h.state().session.tools.brush.angle, h.state().session.tools.brush.pressure_size), (0.0, false));
}

#[test]
fn brush_settings_changes_show_in_healing_and_clone_bar_stays_unchanged() {
    let mut h = bar(Tool::Healing);
    h.state_mut().run("tools.setBrush", json!({"brush": {"angle": -75.0, "pressureSize": true}})).unwrap();
    h.run_steps(3);
    assert_eq!(angle(&h).accesskit_node().numeric_value(), Some(-75.0));
    h.get_by_role_and_label(Role::Button, PRESSURE).click();
    h.run_steps(2);
    assert!(!h.state().session.tools.brush.pressure_size);
    let clone = bar(Tool::CloneStamp);
    assert!(has_control(&clone, Role::Button, SETTINGS));
    assert!(!has_control(&clone, Role::Button, PANEL));
    assert!(!has_control(&clone, Role::SpinButton, "Angle"));
    assert!(!has_control(&clone, Role::Button, PRESSURE));
    assert!(has_control(&clone, Role::SpinButton, "Opacity"));
    assert!(has_control(&clone, Role::SpinButton, "Flow"));
    assert!(clone.get_by_role_and_label(Role::CheckBox, "Aligned").accesskit_node().toggled() == Some(egui::accesskit::Toggled::True));
}
