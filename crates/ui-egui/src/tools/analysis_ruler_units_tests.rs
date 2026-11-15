//! #11989: the Ruler's options-bar values follow Preferences › Units & Rulers with the reference
//! application's decimals (trailing zeros kept), A has one decimal and a degree sign, the system's
//! decimal separator is used, and with no ruler the fields read 0.00 / 0.0° with L2 blank. With
//! Use Measurement Scale on they stay in the scale's units with 2 decimals.

use super::{PhotocraftApp, json, ruler_readouts};

/// An 800 × 600 document at 72 ppi with the ruler from 100,100 to 300,250, rulers in `unit`.
fn app(unit: &str) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600, "resolution": 72})).unwrap();
    app.run("image.analysis.rulerTool", json!({"start": [100, 100], "end": [300, 250]})).unwrap();
    app.run("prefs.set", json!({"path": "unitsAndRulers.rulers", "value": unit})).unwrap();
    app.ui.analysis.use_measurement_scale = false;
    app
}

/// X, Y, W, H, A, L1, L2 as the bar shows them.
fn shown(app: &PhotocraftApp) -> Vec<Option<String>> {
    let doc = app.session.active().unwrap().doc.clone();
    ruler_readouts(app, &doc).to_vec()
}

fn s(v: &str) -> Option<String> {
    Some(v.to_string())
}

#[test]
fn values_follow_the_ruler_unit_with_its_decimals() {
    // The reference reads the start at 99.5 px in the finer units (not measured, see the issue);
    // ours is at 100, so X and Y are those of 100 px. The decimals are the reference's.
    for (unit, x, y, w, h, l1) in [
        ("pixels", "100", "100", "200", "150", s("250.00")),
        ("inches", "1.389", "1.389", "2.778", "2.083", s("3.472")),
        ("cm", "3.53", "3.53", "7.06", "5.29", s("8.82")),
        ("mm", "35.3", "35.3", "70.6", "52.9", s("88.2")),
        ("points", "100.0", "100.0", "200.0", "150.0", s("250.0")),
        ("picas", "8.33", "8.33", "16.67", "12.50", s("20.83")),
        ("percent", "12.5%", "16.7%", "25.0%", "25.0%", None),
    ] {
        let app = app(unit);
        assert_eq!(shown(&app), vec![s(x), s(y), s(w), s(h), s("-36.9°"), l1, None], "{unit}");
    }
}

#[test]
fn no_ruler_reads_zeros_and_a_blank_l2() {
    let mut app = app("inches");
    app.run("image.analysis.rulerTool", json!({"clear": true})).unwrap();
    assert_eq!(shown(&app), vec![s("0.00"), s("0.00"), s("0.00"), s("0.00"), s("0.0°"), s("0.00"), None]);
}

#[test]
fn a_protractor_shows_its_angle_and_both_lengths() {
    let mut app = app("pixels");
    app.run("image.analysis.rulerTool", json!({"start": [100, 100], "end": [300, 100], "protractor": [100, 300]})).unwrap();
    let v = shown(&app);
    assert_eq!((v[4].clone(), v[5].clone(), v[6].clone()), (s("90.0°"), s("200.00"), s("200.00")));
}

#[test]
fn with_the_measurement_scale_the_values_keep_two_decimals() {
    let mut app = app("inches");
    app.ui.analysis.use_measurement_scale = true;
    // The default scale is 1 pixel = 1 pixel.
    assert_eq!(shown(&app), vec![s("100.00"), s("100.00"), s("200.00"), s("150.00"), s("-36.9°"), s("250.00"), None]);
}

#[test]
fn the_systems_decimal_separator_is_used() {
    let app = app("inches");
    let v = crate::i18n::with_decimal_separator(Some(','), || shown(&app));
    assert_eq!((v[0].clone(), v[4].clone(), v[5].clone()), (s("1,389"), s("-36,9°"), s("3,472")));
}

#[test]
fn the_shared_formatters_are_unchanged_for_their_other_callers() {
    use photocraft_engine::prefs::Unit;
    assert_eq!(Unit::Millimeters.decimals(), 2, "the Info panel and Free Transform keep theirs");
    let units = photocraft_engine::prefs::UnitsAndRulers { rulers: Unit::Millimeters, ..Default::default() };
    assert_eq!(units.format(100.0, 72.0, 800.0), "35.28");
}
