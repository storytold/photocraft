//! #13311: Create Polygon's Width, Height, Sides and Star Ratio each say when a typed value was out of
//! range: the reference's "Closest value inserted" alert with OK, then the closest end. Values in range
//! show nothing, and the alert opens once per entry.

use super::create_triangle_range_alert_tests::{SIZE_RANGE_ALERT, alerts, harness, ok, size, type_draft, type_into};
use super::{RATIO_RANGE_ALERT, SIDES_RANGE_ALERT};
use crate::state::Tool;

/// Each field with its stored key, and what typing `typed` into it should end up as.
const OUT_OF_RANGE: [(&str, &str, &str, f64, &str); 8] = [
    ("Width:", "width", "0", 1.0, SIZE_RANGE_ALERT),
    ("Width:", "width", "400000", 300_000.0, SIZE_RANGE_ALERT),
    ("Height:", "height", "-5", 1.0, SIZE_RANGE_ALERT),
    ("Height:", "height", "400000", 300_000.0, SIZE_RANGE_ALERT),
    ("Sides:", "sides", "2", 3.0, SIDES_RANGE_ALERT),
    ("Sides:", "sides", "101", 100.0, SIDES_RANGE_ALERT),
    ("Star Ratio:", "starRatio", "0", 1.0, RATIO_RANGE_ALERT),
    ("Star Ratio:", "starRatio", "101", 100.0, RATIO_RANGE_ALERT),
];

#[test]
fn a_value_in_range_is_kept_without_an_alert() {
    let mut h = harness(Tool::Polygon);
    for (name, key, typed, want) in [
        ("Width:", "width", "250", 250.0),
        ("Height:", "height", "300000", 300_000.0),
        ("Sides:", "sides", "3", 3.0),
        ("Sides:", "sides", "100", 100.0),
        ("Star Ratio:", "starRatio", "1", 1.0),
        ("Star Ratio:", "starRatio", "100", 100.0),
    ] {
        type_into(&mut h, name, typed);
        assert_eq!(size(&h, key), want, "{name} {typed}");
        assert!(alerts(&h).is_empty(), "{name} {typed} is in range");
    }
}

#[test]
fn a_value_past_the_range_takes_the_closest_end_and_alerts_once() {
    for (name, key, typed, want, message) in OUT_OF_RANGE {
        let mut h = harness(Tool::Polygon);
        type_into(&mut h, name, typed);
        assert_eq!(size(&h, key), want, "{name} {typed}");
        assert_eq!(alerts(&h), vec![message.to_string()], "{name} {typed}: one alert");
        h.run_steps(4);
        assert_eq!(alerts(&h).len(), 1, "{name} {typed}: not one per frame");
        ok(&mut h);
        assert!(alerts(&h).is_empty(), "{name} {typed}: OK closes the alert");
        assert_eq!(size(&h, key), want, "{name} {typed}: the closest value stays");
    }
}

#[test]
fn typing_alone_does_not_alert_until_the_field_is_left() {
    for (name, _, typed, _, _) in OUT_OF_RANGE {
        let mut h = harness(Tool::Polygon);
        type_draft(&mut h, name, typed);
        assert!(alerts(&h).is_empty(), "{name} {typed}: still being typed");
    }
}

#[test]
fn the_other_shapes_keep_clamping_silently() {
    let mut h = harness(Tool::Rectangle);
    type_into(&mut h, "Width:", "400000");
    assert_eq!(size(&h, "width"), 300_000.0);
    assert!(alerts(&h).is_empty(), "Create Rectangle has no alert yet");
}
