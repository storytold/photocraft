//! #12683 and #12667: the toning tools' options bar edits the active tool's brush.

use egui::accesskit::{Role, Toggled};
use egui::{Key, Modifiers, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_engine::actions_cmds::ActionsExt as _;
use serde_json::json;

use crate::{PhotocraftApp, Tool};

type H = Harness<'static, Box<PhotocraftApp>>;

const PRESSURE: &str = "Always use pressure for size";

fn bar() -> H {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 160, "height": 160})).unwrap();
    crate::theme::install_fonts(app.ctx());
    crate::theme::apply(app.ctx(), &crate::theme::AppTheme::pro());
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1800.0, 300.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            crate::panels::options_bar(app, ui);
            crate::dialogs::run(app, ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

fn select(h: &mut H, tool: Tool) {
    h.state_mut().ui.tool = tool;
    h.run_steps(3);
}

fn type_angle(h: &mut H, text: &str) {
    h.get_by_role_and_label(Role::SpinButton, "Angle").click();
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(Key::Enter);
    h.run_steps(3);
}

fn toggle_pressure(h: &mut H) {
    h.get_by_role_and_label(Role::Button, PRESSURE).click();
    h.run_steps(2);
}

fn assert_controls(h: &H, degrees: f32, pressure_size: bool) {
    let brush = &h.state().session.tools.brush;
    assert_eq!((brush.angle, brush.pressure_size), (degrees, pressure_size));
    let angle = h.get_by_role_and_label(Role::SpinButton, "Angle");
    assert_eq!(angle.accesskit_node().numeric_value(), Some(f64::from(degrees)));
    let pressure = h.get_by_role_and_label(Role::Button, PRESSURE);
    assert_eq!(pressure.accesskit_node().toggled() == Some(Toggled::True), pressure_size);
}

pub(super) fn check_bar_edits(tool: Tool, other: Tool) {
    let mut h = bar();
    h.state_mut().run("tools.setBrush", json!({"brush": {"angle": -30.0, "pressureSize": true}})).unwrap();
    select(&mut h, tool);
    assert_controls(&h, 0.0, false);

    let exposure = h.get_by_role_and_label(Role::SpinButton, "Exposure").rect();
    let angle = h.get_by_role_and_label(Role::SpinButton, "Angle");
    let protect = h.get_by_role_and_label(Role::CheckBox, "Protect Tones").rect();
    let pressure = h.get_by_role_and_label(Role::Button, PRESSURE);
    assert!(exposure.right() <= angle.rect().left() && angle.rect().right() <= protect.left() && protect.right() <= pressure.rect().left());
    assert_eq!(angle.accesskit_node().description().as_deref(), Some("Set the brush angle"));
    assert_eq!(pressure.accesskit_node().description().as_deref(), Some("Always use Pressure for Size. When off, Brush Preset controls pressure"));

    let app = h.state_mut();
    let past_len = app.session.active().unwrap().history.past_len();
    app.run("actions.record", json!({"name": "Set brush options"})).unwrap();
    type_angle(&mut h, "-45");
    toggle_pressure(&mut h);
    assert_controls(&h, -45.0, true);
    let app = h.state_mut();
    app.run("actions.stop", json!({})).unwrap();
    let recorded = app.session.actions().list.actions().iter().find(|action| action.name == "Set brush options").expect("the recorded brush action");
    let steps = &recorded.steps;
    assert_eq!(steps.len(), 2, "one recorded step per edit");
    assert!(steps.iter().all(|step| step.command == "tools.setBrush"));
    assert_eq!(steps[0].params["brush"]["angle"], json!(-45.0));
    assert_eq!(steps[1].params["brush"]["pressureSize"], json!(true));
    assert_eq!(app.session.active().unwrap().history.past_len(), past_len, "brush controls do not change document History");

    select(&mut h, other);
    assert_controls(&h, 0.0, false);
    select(&mut h, Tool::Brush);
    let brush = &h.state().session.tools.brush;
    assert_eq!((brush.angle, brush.pressure_size), (-30.0, true));
    select(&mut h, tool);
    assert_controls(&h, -45.0, true);

    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "tool.reset", json!({})).unwrap();
    h.run_steps(3);
    assert_controls(&h, 0.0, false);
}

pub(super) fn check_angle_range(tool: Tool) {
    let mut h = bar();
    select(&mut h, tool);
    for (typed, bound) in [("200", 180.0), ("-200", -180.0)] {
        type_angle(&mut h, typed);
        let dialogs = h.state().framework.dialogs.open_dialogs();
        assert_eq!(dialogs.len(), 1, "one range alert for {typed}");
        let message = dialogs[0].fields.as_ref().and_then(|fields| fields.get("message")).and_then(serde_json::Value::as_str);
        assert!(message.is_some_and(|s| s.contains("Closest value inserted")), "{message:?}");
        h.get_by_label("OK").click();
        h.run_steps(4);
        assert!(h.state().framework.dialogs.open_dialogs().is_empty());
        assert_controls(&h, bound, false);
    }
}

fn affected_height(app: &mut PhotocraftApp, command: &str, y: i32, force: f32) -> usize {
    app.run(command, json!({"points": [[80.5, f64::from(y) + 0.5, force]]})).unwrap();
    let state = app.session.active().unwrap();
    let surface = state.doc.layer(state.active_layer.unwrap()).unwrap().surface().unwrap();
    let untouched = surface.rgba(0, 0)[0];
    (y - 14..=y + 14).filter(|row| surface.rgba(80, *row)[0] != untouched).count()
}

pub(super) fn check_stroke(tool: Tool, command: &str) {
    let mut h = bar();
    select(&mut h, tool);
    let app = h.state_mut();
    app.run("edit.fill", json!({"color": "#808080"})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 24.0, "hardness": 1.0, "roundness": 0.5}})).unwrap();
    let default_low = affected_height(app, command, 18, 0.25);
    let default_full = affected_height(app, command, 48, 1.0);
    assert!(default_full > 0);
    assert_eq!(default_low, default_full, "with the toggle off, pressure does not change size");

    toggle_pressure(&mut h);
    let small = affected_height(h.state_mut(), command, 78, 0.25);
    assert!((1..default_full).contains(&small), "with the toggle on, the low-pressure dab is narrower");

    type_angle(&mut h, "90");
    let rotated = affected_height(h.state_mut(), command, 118, 1.0);
    assert!(rotated > default_full, "turning the elliptical tip makes its mark taller");
}

#[test]
fn dodge_bar_edits_its_own_brush_and_records_each_gesture() {
    check_bar_edits(Tool::Dodge, Tool::Burn);
}

#[test]
fn dodge_angle_range_clamps_and_opens_one_alert() {
    check_angle_range(Tool::Dodge);
}

#[test]
fn dodge_stroke_uses_the_bar_angle_and_pressure() {
    check_stroke(Tool::Dodge, "paint.dodge");
}
