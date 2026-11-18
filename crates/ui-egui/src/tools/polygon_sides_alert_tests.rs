//! #13304: the Polygon tool's Sides takes whole numbers 3..100; a value typed past the range becomes
//! the closest one and, once the field is left, the reference's "Closest value inserted" alert says
//! so. The field's own range behaviour (steps, scrubs and the wheel stop at an end without an alert)
//! is the framework `NumberField`'s.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// The options bar with the Polygon tool active.
fn bar() -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = Tool::Polygon;
    let mut h =
        Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1800.0, 300.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    h.run_steps(4);
    h
}

/// Types `text` into Sides, then Tabs out of it.
fn type_sides(h: &mut Harness<'static, Box<PhotocraftApp>>, text: &str) {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Sides")).expect("the Sides field").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

/// How many alerts are open.
fn alerts(h: &Harness<'static, Box<PhotocraftApp>>) -> usize {
    crate::dialogs::alerts::alerts(h.state()).len()
}

fn sides(h: &Harness<'static, Box<PhotocraftApp>>) -> u32 {
    h.state().ui.tool_options.polygon_sides
}

#[test]
fn sides_past_the_range_takes_the_closest_value_and_alerts() {
    for (typed, want) in [("0", 3), ("2", 3), ("-5", 3), ("101", 100), ("1000", 100)] {
        let mut h = bar();
        type_sides(&mut h, typed);
        assert_eq!(sides(&h), want, "{typed}");
        assert_eq!(alerts(&h), 1, "{typed}: one alert");
    }
}

#[test]
fn sides_in_range_are_stored_without_an_alert() {
    for (typed, want) in [("3", 3), ("7", 7), ("100", 100)] {
        let mut h = bar();
        type_sides(&mut h, typed);
        assert_eq!(sides(&h), want, "{typed}");
        assert_eq!(alerts(&h), 0, "{typed} is in range");
    }
}

#[test]
fn sides_that_are_no_number_keep_the_value_without_an_alert() {
    let mut h = bar();
    type_sides(&mut h, "abc");
    assert_eq!(sides(&h), 5, "the previous value stays");
    assert_eq!(alerts(&h), 0, "no number, no alert");
}
