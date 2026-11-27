//! Edit › Color Settings' named setups (#14229) through the command, every one of them.
//!
//! In the `global_state` binary, beside `spot_dot_gain.rs` and `text_gamma.rs`, for the same
//! reason: a setup sets the process-wide Spot dot gain and Blend Text Colors Using Gamma, which
//! other tests' Multichannel and type renders read. The setups themselves are checked value by
//! value in `crates/engine/src/color_settings_presets_tests.rs`.

use photocraft_compose::multichannel::{SPOT_DOT_GAIN, spot_dot_gain};
use photocraft_compose::psblend::TEXT_GAMMA;
use photocraft_engine::Session;
use photocraft_engine::color_cmds::{COLOR_SETTINGS_PRESETS, CUSTOM_SETTINGS};
use serde_json::json;

#[test]
fn every_setup_runs_and_sets_the_compositor() {
    let _process = crate::lock(&crate::PROCESS);
    let mut s = Session::new();
    for name in COLOR_SETTINGS_PRESETS {
        // Blend Text Colors Using Gamma off before: a scripted setup turns it on at 1.45.
        s.execute("edit.colorSettings", json!({"blendTextGamma": false})).unwrap();
        let r = s.execute("edit.colorSettings", json!({"preset": name})).unwrap();
        assert_eq!(r["preset"], name);
        assert_eq!(r["settings"]["blendTextGamma"], json!(TEXT_GAMMA), "{name}");
        assert_eq!(r["settings"]["blendTextGammaOn"], json!(true), "{name}");
        assert_eq!(photocraft_compose::psblend::text_gamma(), TEXT_GAMMA, "{name}");
        // The compositor follows the setup's Spot dot gain.
        let spot = r["settings"]["spotDotGain"].as_f64().unwrap() as f32;
        assert_eq!(spot_dot_gain(), spot, "{name}");
        assert!(matches!(spot, 15.0 | 20.0 | 25.0), "{name}: {spot}");
    }
    // Changing Blend Text Colors Using Gamma alone also makes the values Custom.
    s.execute("edit.colorSettings", json!({"preset": "Europe Prepress 3"})).unwrap();
    let r = s.execute("edit.colorSettings", json!({"blendTextGamma": 1.8})).unwrap();
    assert_eq!(r["preset"], CUSTOM_SETTINGS);
    assert_eq!(spot_dot_gain(), 15.0, "the setup's other values stay");
    s.execute("edit.colorSettings", json!({"reset": true})).unwrap();
    assert_eq!(spot_dot_gain(), SPOT_DOT_GAIN);
}
