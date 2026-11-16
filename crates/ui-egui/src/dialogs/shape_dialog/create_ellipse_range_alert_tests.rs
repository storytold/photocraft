//! #13230: Create Ellipse's Width and Height take 1..300000 px, like Create Triangle's (#13358): a value
//! typed past that becomes the closest end and, once the field is left, the "Closest value inserted"
//! alert with OK says so. Enter or OK waits for the alert, and Cancel or Escape leaves none behind.

use egui_kittest::kittest::Queryable;

use super::create_triangle_range_alert_tests::{alerts, harness, ok, size, type_draft, type_into};
use super::{owns, size_range_alert};
use crate::state::Tool;

fn key_of(name: &str) -> String {
    name.trim_end_matches(':').to_lowercase()
}

#[test]
fn a_size_in_range_is_kept_without_an_alert() {
    for name in ["Width:", "Height:"] {
        let mut h = harness(Tool::EllipseShape);
        for (typed, want) in [("1", 1.0), ("250", 250.0), ("300000", 300_000.0)] {
            type_into(&mut h, name, typed);
            assert_eq!(size(&h, &key_of(name)), want, "{name} {typed}");
            assert!(alerts(&h).is_empty(), "{name} {typed} is in range");
        }
    }
}

#[test]
fn a_size_past_the_range_takes_the_closest_end_and_alerts_once() {
    for name in ["Width:", "Height:"] {
        for (typed, want) in [("-5", 1.0), ("0", 1.0), ("400000", 300_000.0)] {
            let mut h = harness(Tool::EllipseShape);
            type_into(&mut h, name, typed);
            assert_eq!(size(&h, &key_of(name)), want, "{name} {typed}");
            assert_eq!(alerts(&h), vec![size_range_alert()], "{name} {typed}: one alert");
            h.run_steps(4);
            assert_eq!(alerts(&h).len(), 1, "{name} {typed}: not one per frame");
            ok(&mut h);
            assert!(alerts(&h).is_empty(), "{name} {typed}: OK closes the alert");
            assert_eq!(size(&h, &key_of(name)), want, "{name} {typed}: the closest value stays");
        }
    }
}

#[test]
fn enter_or_ok_on_an_invalid_size_waits_for_the_alert_before_creating() {
    for click_ok in [false, true] {
        let mut h = harness(Tool::EllipseShape);
        let layers = h.state().session.active().unwrap().doc.layers.len();
        type_draft(&mut h, "Width:", "400000");
        if click_ok {
            h.get_by_label("OK").click();
        } else {
            h.key_press(egui::Key::Enter);
        }
        h.run_steps(4);
        assert_eq!(alerts(&h), vec![size_range_alert()]);
        assert_eq!(h.state().session.active().unwrap().doc.layers.len(), layers, "the alert must precede creation");
        ok(&mut h);
        assert!(h.state().ui.dialogs.iter().any(|d| owns(&d.fields)), "acknowledging the alert keeps Create Ellipse open");
        h.get_by_label("OK").click();
        h.run_steps(4);
        assert!(h.state().ui.dialogs.is_empty());
        assert_eq!(h.state().session.active().unwrap().doc.layers.len(), layers + 1);
    }
}

#[test]
fn cancel_or_escape_on_an_invalid_size_leaves_no_alert_or_shape() {
    for click_cancel in [false, true] {
        let mut h = harness(Tool::EllipseShape);
        let layers = h.state().session.active().unwrap().doc.layers.len();
        type_draft(&mut h, "Width:", "400000");
        if click_cancel {
            h.get_by_label("Cancel").click();
        } else {
            h.key_press(egui::Key::Escape);
        }
        h.run_steps(4);
        assert!(h.state().ui.dialogs.is_empty(), "cancelling must not leave an alert behind");
        assert_eq!(h.state().session.active().unwrap().doc.layers.len(), layers);
    }
}
