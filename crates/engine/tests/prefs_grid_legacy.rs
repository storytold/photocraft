//! The per-axis grid preference (#5476) migrates legacy single-spacing preferences.json files.

use photocraft_engine::Session;
use serde_json::{Value, json};

#[test]
fn legacy_gridline_every_becomes_the_per_axis_pair() {
    let mut s = Session::new();
    s.load_prefs_json(
        json!({"guidesGridAndSlices": {"gridlineEvery": 3.5, "gridUnit": "pixels", "subdivisions": 6}})
            .to_string()
            .as_str(),
    )
    .unwrap();
    let g = &s.prefs().guides_grid_and_slices;
    assert_eq!(g.gridline_every_x, 3.5);
    assert_eq!(g.gridline_every_y, 3.5);
    assert_eq!(g.grid_unit, photocraft_engine::prefs::Unit::Pixels);
    assert_eq!(g.subdivisions, 6);
}

#[test]
fn legacy_key_is_dropped_from_the_view_but_the_pair_is_readable() {
    let mut s = Session::new();
    s.load_prefs_json(r#"{"guidesGridAndSlices": {"gridlineEvery": 3.5}}"#).unwrap();
    let prefs = &s.prefs.get();
    assert_eq!(prefs.get("guidesGridAndSlices.gridlineEveryX"), Some(json!(3.5)));
    assert_eq!(prefs.get("guidesGridAndSlices.gridlineEveryY"), Some(json!(3.5)));
    assert_eq!(prefs.get("guidesGridAndSlices.gridlineEvery"), None);
}

#[test]
fn per_axis_values_round_trip() {
    let mut s = Session::new();
    s.execute("prefs.set", json!({"path": "guidesGridAndSlices.gridlineEveryX", "value": 8.0})).unwrap();
    s.execute("prefs.set", json!({"path": "guidesGridAndSlices.gridlineEveryY", "value": 2.5})).unwrap();
    let json = s.prefs_to_json();
    let legacy = json.parse::<Value>().unwrap()["guidesGridAndSlices"]["gridlineEvery"].is_null();
    assert!(legacy, "the single-axis key must not be written anymore");

    let mut restored = Session::new();
    restored.load_prefs_json(&json).unwrap();
    let g = &restored.prefs().guides_grid_and_slices;
    assert_eq!(g.gridline_every_x, 8.0);
    assert_eq!(g.gridline_every_y, 2.5);
}
