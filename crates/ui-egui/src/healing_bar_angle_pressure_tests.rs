//! #12106: Healing's bar opens Clone Source and edits only its own brush's angle and pressure.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::{DialogKind, Tool};
use crate::theme::ThemeKind;

const PANEL: &str = "Toggle the Clone Source panel";
const SETTINGS: &str = "Toggle the Brush Settings panel";
const PRESSURE: &str = "Always use pressure for size";
const PRESSURE_TIP: &str = "Always use Pressure for Size. When off, Brush Preset controls pressure";
const ANGLE_ALERT: &str = "An integer between -180 ° and 180 ° is required. Closest value inserted.";

fn bar(tool: Tool) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = tool;
    let mut h = Harness::builder().with_size(vec2(1800.0, 300.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, ThemeKind::ProMedium);
    h.run_steps(4);
    h
}

fn angle<'a>(h: &'a Harness<'static, PhotocraftApp>) -> egui_kittest::Node<'a> {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().description().as_deref() == Some("Set the brush angle")).expect("Healing Angle")
}

fn type_angle(h: &mut Harness<'static, PhotocraftApp>, text: &str) {
    angle(h).click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
}

fn alerts(h: &Harness<'static, PhotocraftApp>) -> Vec<String> {
    h.state().ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| d.fields.get("message")?.as_str().map(str::to_owned)).collect()
}

#[test]
fn healing_bar_orders_source_angle_and_pressure_without_extra_brush_controls() {
    let h = bar(Tool::Healing);
    let picker = h.get_by_label("Brush Preset picker").rect();
    let panel = h.get_by_label(PANEL).rect();
    let mode = h
        .query_all_by_role(Role::ComboBox)
        .find(|n| n.accesskit_node().label().is_some_and(|label| label.trim_end_matches(':') == "Mode"))
        .expect("Mode")
        .rect();
    let sample = h.query_all_by_role(Role::ComboBox).find(|n| n.value().as_deref() == Some("Current Layer")).expect("Sample").rect();
    let field = angle(&h).rect();
    let pressure = h.get_by_label(PRESSURE);
    assert!(picker.right() <= panel.left() && panel.right() <= mode.left(), "picker, Clone Source, Mode");
    assert!(sample.right() <= field.left() && field.right() <= pressure.rect().left(), "Sample, Angle, Pressure");
    assert_eq!(h.state().session.tools.brush.angle, 0.0);
    assert!(!h.state().session.tools.brush.pressure_size);
    assert_eq!(pressure.accesskit_node().description().as_deref(), Some(PRESSURE_TIP));
    assert!(h.query_by_label(SETTINGS).is_none());
    for label in ["Opacity:", "Flow:", "Smoothing", "Always use pressure for opacity"] {
        assert!(h.query_by_label(label).is_none(), "Healing has no {label}");
    }
}

#[test]
fn clone_source_toggle_uses_the_existing_panel_command_in_both_directions() {
    let mut h = bar(Tool::Healing);
    assert!(!h.state().ui.presets_ui.clone_source);
    h.get_by_label(PANEL).click();
    h.run_steps(3);
    assert!(h.state().ui.presets_ui.clone_source);
    assert!(!h.state().ui.panels.brush_settings);
    h.get_by_label(PANEL).click();
    h.run_steps(3);
    assert!(!h.state().ui.presets_ui.clone_source);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "window.panel.cloneSource", json!({"show": true})).unwrap();
    h.run_steps(3);
    h.get_by_label(PANEL).click();
    h.run_steps(3);
    assert!(!h.state().ui.presets_ui.clone_source, "the button reads the panel's current state");
    assert!(!h.state().ui.status_error, "{}", h.state().ui.status);
}

#[test]
fn healing_angle_truncates_fractions_and_clamps_typed_limits_with_one_ok_alert() {
    for (text, want, alert) in [("45.5", 45.0, false), ("-45.5", -45.0, false), ("400", 180.0, true), ("-400", -180.0, true)] {
        let mut h = bar(Tool::Healing);
        type_angle(&mut h, text);
        assert_eq!(h.state().session.tools.brush.angle, want, "{text}");
        assert_eq!(angle(&h).accesskit_node().numeric_value(), Some(f64::from(want)));
        if alert {
            assert_eq!(alerts(&h), vec![ANGLE_ALERT.to_string()], "{text}");
            let id = h.state().ui.dialogs.iter().find(|d| d.kind == DialogKind::Error).expect("range alert").id;
            crate::dialogs::confirm(h.state_mut(), id).unwrap();
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
    h.get_by_label(PRESSURE).click();
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
    h.get_by_label(PRESSURE).click();
    h.run_steps(2);
    assert!(!h.state().session.tools.brush.pressure_size);
    let clone = bar(Tool::CloneStamp);
    assert!(clone.query_by_label(SETTINGS).is_some());
    assert!(clone.query_by_label(PANEL).is_none());
    assert!(clone.query_by_label("Angle").is_none());
    assert!(clone.query_by_label(PRESSURE).is_none());
    assert!(clone.query_by_label("Opacity:").is_some());
    assert!(clone.query_by_label("Flow:").is_some());
    assert!(clone.get_by_role_and_label(Role::CheckBox, "Aligned").accesskit_node().toggled() == Some(egui::accesskit::Toggled::True));
}
