//! #13336: the Rectangle tool's Radius takes 0..150000 px, decimals included; a value typed past
//! the range becomes the closest end and, once the field is left, the reference's "Closest value
//! inserted" alert says so. Stepping at an end is the framework `NumberField`'s and raises none.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// The options bar with the Rectangle tool active and a pixel layer targeted.
fn bar() -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = Tool::Rectangle;
    let mut h =
        Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1800.0, 300.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    h.run_steps(4);
    h
}

/// Types `text` into Radius, then Tabs out of it.
fn type_radius(h: &mut Harness<'static, Box<PhotocraftApp>>, text: &str) {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Radius")).expect("the Radius field").click();
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

fn radius(h: &Harness<'static, Box<PhotocraftApp>>) -> f32 {
    h.state().ui.tool_options.corner_radius
}

#[test]
fn radius_in_range_is_stored_without_an_alert() {
    for (typed, want) in [("0", 0.0), ("12.5", 12.5), ("5000", 5000.0), ("150000", 150_000.0)] {
        let mut h = bar();
        type_radius(&mut h, typed);
        assert_eq!(radius(&h), want, "{typed}");
        assert_eq!(alerts(&h), 0, "{typed} is in range");
    }
}

#[test]
fn radius_past_the_range_takes_the_closest_value_and_alerts() {
    for (typed, want) in [("-5", 0.0), ("200000", 150_000.0)] {
        let mut h = bar();
        type_radius(&mut h, typed);
        assert_eq!(radius(&h), want, "{typed}");
        assert_eq!(alerts(&h), 1, "{typed}: one alert");
    }
}
