//! #12570: the Red Eye options bar shows "Pupil Size:" and "Darken Amount:" as whole-percent
//! fields 1..100, each with a popup slider. A value typed past the range becomes the closest one
//! with the reference's "An integer between 1% and 100% is required" alert; text that isn't a
//! number keeps the old value. The bar has no hint text, Reset Tool gives 50 / 50, and editing an
//! option adds no history state.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

type H = Harness<'static, Box<PhotocraftApp>>;

/// The reference's alert for either field typed past 1..100.
const ALERT: &str = "An integer between 1% and 100% is required. Closest value inserted.";

/// The options bar with the Red Eye tool active, and the dialog host its alerts open on.
fn bar() -> H {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = Tool::RedEye;
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

/// Types `text` into the field named `label` and commits it with Enter.
fn type_into(h: &mut H, label: &str, text: &str) {
    field(h, label).unwrap_or_else(|| panic!("the {label} field")).click();
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

/// The stored (Pupil Size, Darken Amount).
fn values(h: &H) -> (f32, f32) {
    let o = &h.state().ui.tool_options;
    (o.red_eye_pupil_size, o.red_eye_darken)
}

/// The value field `label` stores.
fn value(h: &H, label: &str) -> f32 {
    let (pupil, darken) = values(h);
    if label == "Pupil Size" { pupil } else { darken }
}

fn steps(h: &H) -> usize {
    h.state().session.active().unwrap().history.past_len()
}

/// The "%" the fields draw after the number is not in the accessibility tree; the screenshot in
/// the PR shows it.
#[test]
fn the_labels_end_with_a_colon_and_each_field_has_a_slider_arrow() {
    let h = bar();
    for label in ["Pupil Size:", "Darken Amount:"] {
        let node = h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some(label));
        let node = node.unwrap_or_else(|| panic!("a field labelled {label:?}"));
        let arrow = node.accesskit_node().children().any(|c| c.role() == Role::Button && c.label().as_deref() == Some("Slider"));
        assert!(arrow, "{label} has its slider arrow");
    }
}

#[test]
fn each_field_has_a_popup_slider_that_escape_closes() {
    let mut h = bar();
    assert_eq!(h.query_all_by_role_and_label(Role::Button, "Slider").count(), 2, "one slider arrow per field");
    h.query_all_by_role_and_label(Role::Button, "Slider").next().unwrap().click();
    h.run_steps(3);
    assert!(h.query_all_by_role(Role::Slider).next().is_some(), "the arrow opens a slider");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.query_all_by_role(Role::Slider).next().is_none(), "Escape closes the slider");
}

#[test]
fn a_value_typed_past_1_100_takes_the_closest_value_and_alerts() {
    for label in ["Pupil Size", "Darken Amount"] {
        for (typed, want) in [("0", 1.0), ("101", 100.0), ("150", 100.0), ("-5", 1.0)] {
            let mut h = bar();
            type_into(&mut h, label, typed);
            assert_eq!(value(&h, label), want, "{label} {typed}");
            assert_eq!(alerts(&h), vec![ALERT.to_string()], "{label} {typed}: one alert");
        }
    }
}

#[test]
fn a_value_in_range_is_kept_without_an_alert() {
    for label in ["Pupil Size", "Darken Amount"] {
        for (typed, want) in [("1", 1.0), ("100", 100.0), ("50", 50.0), ("37", 37.0)] {
            let mut h = bar();
            type_into(&mut h, label, typed);
            assert_eq!(value(&h, label), want, "{label} {typed}");
            assert!(alerts(&h).is_empty(), "{label} {typed} is in range: {:?}", alerts(&h));
        }
    }
}

#[test]
fn text_that_is_not_a_number_keeps_the_old_value_without_an_alert() {
    for label in ["Pupil Size", "Darken Amount"] {
        let mut h = bar();
        type_into(&mut h, label, "37");
        type_into(&mut h, label, "abc");
        assert_eq!(value(&h, label), 37.0, "{label}");
        assert!(alerts(&h).is_empty(), "{label}: {:?}", alerts(&h));
    }
}

#[test]
fn the_bar_has_no_hint_text() {
    let h = bar();
    assert!(h.query_by_label("Click a red pupil to neutralize it").is_none());
}

#[test]
fn reset_tool_gives_50_50_and_options_add_no_history() {
    let mut h = bar();
    let s0 = steps(&h);
    type_into(&mut h, "Pupil Size", "12");
    type_into(&mut h, "Darken Amount", "88");
    assert_eq!(values(&h), (12.0, 88.0));
    assert_eq!(steps(&h), s0, "changing an option adds no history state");
    let ctx = h.state().ctx().clone();
    let (req, _rx) = crate::control::ControlRequest::new("ui.menu.invoke", json!({"id": "tool.reset"}));
    let _ = crate::control::handle(h.state_mut(), &ctx, &req);
    h.run_steps(2);
    assert_eq!(values(&h), (50.0, 50.0));
}

/// A stored Darken Amount of 0 (an old setting) reaches `paint.redEye` as 1, the bottom of the
/// range.
#[test]
fn a_click_sends_a_darken_amount_of_at_least_1() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 100, "height": 60})).unwrap();
    app.run("paint.pencil", json!({"points": [[50, 30]], "size": 18, "hardness": 100, "color": "#f21818"})).unwrap();
    app.ui.tool = Tool::RedEye;
    app.ui.tool_options.red_eye_darken = 0.0;
    let m = egui::Modifiers::NONE;
    crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Down { x: 50.0, y: 30.0, pressure: 1.0 }, m);
    crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Up { x: 50.0, y: 30.0 }, m);
    let (id, p) = app.session.journal.last().unwrap();
    assert_eq!(id.as_str(), "paint.redEye", "{}", app.ui.status);
    assert_eq!(p["darken"].as_f64(), Some(1.0), "{p}");
}
