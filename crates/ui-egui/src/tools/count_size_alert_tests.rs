//! #11768: the Count tool's Marker Size (1..10) and Label Size (8..72) take a value typed past the range as the closest
//! one and, once the field is left, the reference's alert with OK says so. Digits-only entry, IME commits and the
//! history step of a typed entry belong to the framework's number field.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

const MARKER: &str = "Marker Size";
const LABEL: &str = "Label Size";

/// A 100 x 100 document with the Count tool and one count group; the harness draws the options bar.
fn harness() -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("count.add", json!({"x": 10, "y": 10})).unwrap();
    app.ui.tool = Tool::Count;
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1600.0, 120.0)).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::options_bar(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::studio());
    h.run_steps(4);
    h
}

/// Types `text` into the field named `field`, then Tabs out of it.
fn type_into(h: &mut Harness<'static, Box<PhotocraftApp>>, field: &str, text: &str) {
    h.query_all_by_role(egui::accesskit::Role::SpinButton)
        .find(|n| n.accesskit_node().label().is_some_and(|l| l.starts_with(field)))
        .unwrap_or_else(|| panic!("the {field} field"))
        .click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

/// The current group's (Marker Size, Label Size).
fn sizes(h: &Harness<'static, Box<PhotocraftApp>>) -> (u32, u32) {
    let g = &h.state().session.active().unwrap().doc.measurement.count_groups[0];
    (g.marker_size, g.label_size)
}

#[test]
fn an_out_of_range_size_alerts_and_takes_the_closest_value() {
    let cases = [(MARKER, "0", 1), (MARKER, "-4", 1), (MARKER, "20", 10), (LABEL, "4", 8), (LABEL, "100", 72)];
    for (field, typed, want) in cases {
        let mut h = harness();
        type_into(&mut h, field, typed);
        let got = if field == MARKER { sizes(&h).0 } else { sizes(&h).1 };
        assert_eq!(got, want, "{field} {typed}");
        let open = crate::dialogs::alerts::alerts(h.state());
        assert_eq!(open.len(), 1, "{field} {typed}: one alert");
        crate::dialogs::testing::confirm(h.state_mut(), open[0]).unwrap();
        assert!(crate::dialogs::alerts::alerts(h.state()).is_empty(), "{field} {typed}: OK closes the alert");
        let kept = if field == MARKER { sizes(&h).0 } else { sizes(&h).1 };
        assert_eq!(kept, want, "{field} {typed}: the closest value stays");
    }
}

#[test]
fn in_range_sizes_are_set_without_an_alert() {
    let mut h = harness();
    type_into(&mut h, MARKER, "7");
    type_into(&mut h, LABEL, "30");
    assert_eq!(sizes(&h), (7, 30));
    type_into(&mut h, MARKER, "10");
    type_into(&mut h, LABEL, "8");
    assert_eq!(sizes(&h), (10, 8), "the ends of the ranges are in them");
    assert!(crate::dialogs::alerts::alerts(h.state()).is_empty());
}

#[test]
fn the_alert_shows_once_not_once_per_frame() {
    let mut h = harness();
    type_into(&mut h, LABEL, "100");
    h.run_steps(10);
    assert_eq!(crate::dialogs::alerts::alerts(h.state()).len(), 1);
}
