//! #12630: the Art History Brush's Area takes whole pixels 0..500 and Tolerance whole percents
//! 0..100; a value typed past either range becomes the closest one with the reference's "Closest
//! value inserted" alert. Opacity, Area and Tolerance carry the reference's tooltips, and the bar
//! has no hint text.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::retouch_ui::bar::history::{ART_HISTORY_AREA_ALERT, ART_HISTORY_TOLERANCE_ALERT};
use crate::PhotocraftApp;
use crate::state::Tool;

type H = Harness<'static, Box<PhotocraftApp>>;

/// The options bar with the Art History Brush active, and the dialog host its alerts open on.
fn bar() -> H {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = Tool::ArtHistoryBrush;
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

/// The description (tooltip) of the number field named `label`.
fn tip(h: &H, label: &str) -> Option<String> {
    field(h, label)?.accesskit_node().description().map(|d| d.to_string())
}

#[test]
fn area_past_0_500_takes_the_closest_value_and_alerts() {
    for (typed, want) in [("600", 500.0), ("-5", 0.0)] {
        let mut h = bar();
        type_into(&mut h, "Area", typed);
        assert_eq!(h.state().ui.tool_options.art_history_area, want, "{typed}");
        assert_eq!(alerts(&h), vec![ART_HISTORY_AREA_ALERT.to_string()], "{typed}: one alert");
    }
}

#[test]
fn area_in_range_is_a_whole_number_without_an_alert() {
    for (typed, want) in [("500", 500.0), ("250", 250.0), ("0", 0.0), ("37.6", 38.0)] {
        let mut h = bar();
        type_into(&mut h, "Area", typed);
        assert_eq!(h.state().ui.tool_options.art_history_area, want, "{typed}");
        assert!(alerts(&h).is_empty(), "{typed} is in range: {:?}", alerts(&h));
    }
}

#[test]
fn tolerance_past_0_100_takes_the_closest_value_and_alerts() {
    for (typed, want) in [("150", 100.0), ("-3", 0.0)] {
        let mut h = bar();
        type_into(&mut h, "Tolerance", typed);
        assert_eq!(h.state().ui.tool_options.art_history_tolerance, want, "{typed}");
        assert_eq!(alerts(&h), vec![ART_HISTORY_TOLERANCE_ALERT.to_string()], "{typed}: one alert");
    }
}

#[test]
fn tolerance_in_range_is_a_whole_number_without_an_alert() {
    for (typed, want) in [("100", 100.0), ("12.4", 12.0)] {
        let mut h = bar();
        type_into(&mut h, "Tolerance", typed);
        assert_eq!(h.state().ui.tool_options.art_history_tolerance, want, "{typed}");
        assert!(alerts(&h).is_empty(), "{typed} is in range");
    }
}

#[test]
fn opacity_area_and_tolerance_have_the_reference_tooltips() {
    let h = bar();
    assert_eq!(tip(&h, "Opacity").as_deref(), Some("Set opacity for stroke"));
    assert_eq!(tip(&h, "Area").as_deref(), Some("Painting area diameter"));
    assert_eq!(tip(&h, "Tolerance").as_deref(), Some("Painting tolerance"));
}

#[test]
fn the_bar_has_no_hint_text() {
    let h = bar();
    assert!(h.query_by_label("Stylized dabs from a history state").is_none());
}

#[test]
fn the_command_takes_an_area_up_to_500() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    let stroke = |area: f64| json!({"points": [[10, 40], [70, 40]], "size": 8, "area": area});
    assert!(app.run("paint.artHistoryBrush", stroke(500.0)).is_ok());
    assert!(app.run("paint.artHistoryBrush", stroke(250.0)).is_ok());
    assert!(app.run("paint.artHistoryBrush", stroke(500.5)).is_err());
    assert!(app.run("paint.artHistoryBrush", stroke(-1.0)).is_err());
}
