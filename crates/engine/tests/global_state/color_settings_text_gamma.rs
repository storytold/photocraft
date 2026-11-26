//! Color Settings › Blend Text Colors Using Gamma is a checkbox with a 1.00 to 3.00 field
//! (photocraft#14243): the box off keeps the field's value, a number in range sets the value and
//! checks the box, the compositor takes up to 3.00, and settings saved as one number (1 = off)
//! still load.
//!
//! In the `global_state` binary, beside `text_gamma.rs`: the text gamma is process-wide and would
//! change how other tests' type layers composite.

use photocraft_compose::psblend::{MAX_TEXT_GAMMA, TEXT_GAMMA, set_text_gamma, text_gamma};
use photocraft_engine::Session;
use photocraft_engine::color_cmds::{ColorExt, ColorSettings, MIN_TEXT_GAMMA};
use photocraft_engine::prefs::SavedPrefsExt;
use serde_json::{Value, json};

const CMD: &str = "edit.colorSettings";

/// The box and the field as the command reports them, the field in hundredths.
fn shown(r: &Value) -> (bool, i64) {
    let s = &r["settings"];
    (s["blendTextGammaOn"].as_bool().expect("the box"), (s["blendTextGamma"].as_f64().expect("the field") * 100.0).round() as i64)
}

#[test]
fn the_field_takes_1_to_3_with_two_decimals_and_checks_the_box() {
    let _process = crate::lock(&crate::PROCESS);
    let mut s = Session::new();
    assert_eq!((MIN_TEXT_GAMMA, MAX_TEXT_GAMMA), (1.0, 3.0));
    assert_eq!(shown(&s.execute(CMD, json!({})).unwrap()), (true, 145), "on at 1.45 by default");
    for (gamma, hundredths) in [(1.0, 100), (3.0, 300), (2.5, 250), (1.234, 123), (2.996, 300)] {
        s.execute(CMD, json!({"blendTextGamma": false})).unwrap();
        let r = s.execute(CMD, json!({"blendTextGamma": gamma})).unwrap();
        assert_eq!(shown(&r), (true, hundredths), "{gamma}: a number checks the box");
        assert!((text_gamma() - hundredths as f32 / 100.0).abs() < 1e-6, "{gamma}: the compositor follows");
    }
    // The dialog sends the field and the box: a value with the box off keeps it off.
    let r = s.execute(CMD, json!({"blendTextGamma": 2.2, "blendTextGammaOn": false})).unwrap();
    assert_eq!(shown(&r), (false, 220));
    assert_eq!(text_gamma(), 1.0, "the box off blends in gamma 1");
    s.execute(CMD, json!({"reset": true})).unwrap();
}

#[test]
fn the_box_off_keeps_the_value_and_on_again_uses_it() {
    let _process = crate::lock(&crate::PROCESS);
    let mut s = Session::new();
    s.execute(CMD, json!({"blendTextGamma": 2.8})).unwrap();
    let r = s.execute(CMD, json!({"blendTextGamma": false})).unwrap();
    assert_eq!(shown(&r), (false, 280));
    assert_eq!(text_gamma(), 1.0);
    let r = s.execute(CMD, json!({"blendTextGammaOn": true})).unwrap();
    assert_eq!(shown(&r), (true, 280));
    assert!((text_gamma() - 2.8).abs() < 1e-6, "2.8 is no longer clamped to 2.2");
    s.execute(CMD, json!({"blendTextGammaOn": false})).unwrap();
    let r = s.execute(CMD, json!({"blendTextGamma": true})).unwrap();
    assert_eq!(shown(&r), (true, 280), "`true` checks the box and keeps the value");
    s.execute(CMD, json!({"reset": true})).unwrap();
}

#[test]
fn bad_params_are_errors_and_change_nothing() {
    let _process = crate::lock(&crate::PROCESS);
    let mut s = Session::new();
    s.execute(CMD, json!({"blendTextGamma": 1.8})).unwrap();
    let before = s.color().settings.clone();
    for bad in [
        json!({"blendTextGamma": 0.99}),
        json!({"blendTextGamma": 3.01}),
        json!({"blendTextGamma": -1.45}),
        json!({"blendTextGamma": 0}),
        json!({"blendTextGamma": 9}),
        json!({"blendTextGamma": 1e308}),
        // A NaN or an infinity reaches JSON as null.
        json!({"blendTextGamma": f64::NAN}),
        json!({"blendTextGamma": f64::INFINITY}),
        json!({"blendTextGamma": null}),
        json!({"blendTextGamma": "1.8"}),
        json!({"blendTextGamma": "on"}),
        json!({"blendTextGamma": [1.8]}),
        json!({"blendTextGamma": {"value": 1.8}}),
        json!({"blendTextGammaOn": "yes"}),
        json!({"blendTextGammaOn": 1}),
        json!({"blendTextGammaOn": null}),
    ] {
        assert!(s.execute(CMD, bad.clone()).is_err(), "{bad}");
        assert_eq!(s.color().settings, before, "{bad}");
    }
    assert!((text_gamma() - 1.8).abs() < 1e-6);
    s.execute(CMD, json!({"reset": true})).unwrap();
}

#[test]
fn the_compositor_clamps_to_1_to_3() {
    let _process = crate::lock(&crate::PROCESS);
    for (set, read) in [(2.8, 2.8), (3.0, 3.0), (3.5, 3.0), (0.5, 1.0), (f32::NAN, TEXT_GAMMA), (f32::INFINITY, TEXT_GAMMA)] {
        set_text_gamma(set);
        assert!((text_gamma() - read).abs() < 1e-6, "{set} reads back {}", text_gamma());
    }
    set_text_gamma(TEXT_GAMMA);
}

#[test]
fn settings_saved_as_one_number_still_load() {
    let _process = crate::lock(&crate::PROCESS);
    // Before the checkbox, one number: 1 meant off.
    let legacy = |gamma: Value| serde_json::from_value::<ColorSettings>(json!({"workingRgb": "srgb", "blendTextGamma": gamma})).unwrap();
    let off = legacy(json!(1.0));
    assert_eq!((off.blend_text_gamma_on, off.blend_text_gamma), (false, TEXT_GAMMA), "1 loads as the box off with 1.45");
    assert_eq!(off.text_gamma(), 1.0);
    let on = legacy(json!(1.8));
    assert_eq!((on.blend_text_gamma_on, on.blend_text_gamma), (true, 1.8), "any other number loads as the box on with it");
    let default = legacy(json!(1.45));
    assert_eq!((default.blend_text_gamma_on, default.blend_text_gamma), (true, 1.45));
    // Saved without the gamma at all: the defaults.
    let none = serde_json::from_value::<ColorSettings>(json!({"workingRgb": "srgb"})).unwrap();
    assert_eq!((none.blend_text_gamma_on, none.blend_text_gamma), (true, TEXT_GAMMA));
    // Saved with the box: read as saved, 1.00 with the box on included.
    let new = serde_json::from_value::<ColorSettings>(json!({"blendTextGamma": 1.0, "blendTextGammaOn": true})).unwrap();
    assert_eq!((new.blend_text_gamma_on, new.blend_text_gamma), (true, 1.0));
    // And they round-trip.
    let saved = ColorSettings { blend_text_gamma: 2.4, blend_text_gamma_on: false, ..ColorSettings::default() };
    assert_eq!(serde_json::from_value::<ColorSettings>(serde_json::to_value(&saved).unwrap()).unwrap(), saved);

    // Through the settings file, into the session and the compositor.
    let mut s = Session::new();
    let invalid = s.load_prefs_json(r#"{"colorSettings": {"blendTextGamma": 1.0}}"#).unwrap();
    assert!(invalid.is_empty(), "{invalid:?}");
    assert_eq!((s.color().settings.blend_text_gamma_on, s.color().settings.blend_text_gamma), (false, TEXT_GAMMA));
    assert_eq!(text_gamma(), 1.0);
    // Turning the box on uses 1.45, not the old file's 1.
    s.execute(CMD, json!({"blendTextGammaOn": true})).unwrap();
    assert!((text_gamma() - TEXT_GAMMA).abs() < 1e-6);
    s.load_prefs_json(r#"{"colorSettings": {"blendTextGamma": 2.0}}"#).unwrap();
    assert_eq!((s.color().settings.blend_text_gamma_on, s.color().settings.blend_text_gamma), (true, 2.0));
    assert!((text_gamma() - 2.0).abs() < 1e-6);
    // The new format saves and loads both.
    s.execute(CMD, json!({"blendTextGamma": 2.6, "blendTextGammaOn": false})).unwrap();
    let file = s.prefs_to_json();
    let mut reopened = Session::new();
    reopened.load_prefs_json(&file).unwrap();
    assert_eq!((reopened.color().settings.blend_text_gamma_on, reopened.color().settings.blend_text_gamma), (false, 2.6));
    assert_eq!(text_gamma(), 1.0);
    // A field value out of range (a hand-edited file) loads as 1.45 and is reported.
    let invalid = reopened.load_prefs_json(r#"{"colorSettings": {"blendTextGamma": 7.0, "blendTextGammaOn": true}}"#).unwrap();
    assert!(invalid.iter().any(|i| i.keys == ["colorSettings", "blendTextGamma"]), "{invalid:?}");
    assert_eq!(reopened.color().settings.blend_text_gamma, TEXT_GAMMA);
    assert!(reopened.execute(CMD, json!({"bpc": false})).is_ok(), "later edits are accepted");
    s.execute(CMD, json!({"reset": true})).unwrap();
}

#[test]
fn a_legacy_number_below_1_is_reported_not_read_as_off() {
    let _process = crate::lock(&crate::PROCESS);
    // Only 1 meant off in the one-number format; the command never took less, so a smaller
    // number is a hand-edited file and is reported like any other value out of range.
    let mut s = Session::new();
    for gamma in ["0.5", "0", "-2"] {
        let invalid = s.load_prefs_json(&format!(r#"{{"colorSettings": {{"blendTextGamma": {gamma}}}}}"#)).unwrap();
        assert!(invalid.iter().any(|i| i.keys == ["colorSettings", "blendTextGamma"]), "{gamma}: {invalid:?}");
        assert_eq!((s.color().settings.blend_text_gamma_on, s.color().settings.blend_text_gamma), (true, TEXT_GAMMA), "{gamma}");
    }
    s.execute(CMD, json!({"reset": true})).unwrap();
}
