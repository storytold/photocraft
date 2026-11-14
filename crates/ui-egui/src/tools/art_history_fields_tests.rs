//! #12630: the Art History Brush's Area takes whole pixels 0..500 and Tolerance whole percents
//! 0..100; a value typed past either range becomes the closest one with the reference's "Closest
//! value inserted" alert. Opacity, Area and Tolerance carry the reference's tooltips, and the bar
//! has no hint text.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::retouch_ui::ART_HISTORY_AREA_ALERT;
use crate::PhotocraftApp;
use crate::panels::options_bar::ZERO_TO_100;
use crate::state::{DialogKind, Tool};

/// The options bar with the Art History Brush active.
fn bar() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = Tool::ArtHistoryBrush;
    let mut h = Harness::builder().with_size(vec2(1800.0, 300.0)).build_ui_state(
        |ui, app| {
            crate::field_wheel::begin_frame(ui.ctx());
            crate::panels::options_bar(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h
}

/// Types `text` into the field labelled `label`, then Tabs out of it.
fn type_into(h: &mut Harness<'static, PhotocraftApp>, label: &str, text: &str) {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some(label)).expect("the field").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

fn alerts(h: &Harness<'static, PhotocraftApp>) -> Vec<String> {
    h.state().ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| d.fields.get("message")?.as_str().map(str::to_owned)).collect()
}

/// The descriptions (tooltips) of the controls with `role` labelled `label`.
fn tips(h: &Harness<'static, PhotocraftApp>, role: Role, label: &str) -> Vec<String> {
    h.query_all_by_role(role)
        .filter(|n| n.accesskit_node().label().as_deref() == Some(label))
        .filter_map(|n| n.accesskit_node().description().map(|d| d.to_string()))
        .collect()
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
        assert_eq!(alerts(&h), vec![ZERO_TO_100.to_string()], "{typed}: one alert");
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
    assert!(tips(&h, Role::SpinButton, "Opacity:").contains(&"Set opacity for stroke".to_string()));
    assert!(tips(&h, Role::SpinButton, "Area").contains(&"Painting area diameter".to_string()));
    assert!(tips(&h, Role::SpinButton, "Tolerance").contains(&"Painting tolerance".to_string()));
    assert!(tips(&h, Role::Button, "Tolerance slider").contains(&"Painting tolerance".to_string()), "the slider arrow");
    assert!(tips(&h, Role::Button, "Area").is_empty(), "Area has no slider");
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
