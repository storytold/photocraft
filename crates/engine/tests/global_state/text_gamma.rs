//! Edit › Color Settings › "Blend Text Colors Using Gamma" drives the compositor's
//! application-wide text gamma (`photocraft_compose::psblend::text_gamma`).
//!
//! This test needs its own process (nextest runs every test in one): switching
//! the gamma away from the default changes how every type layer composites, so running it next
//! to the engine's unit tests let a concurrent test flatten one document under two gammas
//! (`canvas_geom::tests::turns_at_every_depth_and_mode` compares the composite before an edit
//! with the one after its undo, and failed on Windows CI that way). Keep any other test that
//! changes the gamma in this file or in `color_settings_text_gamma.rs`.

use photocraft_engine::Session;
use photocraft_engine::color_cmds::ColorExt;
use serde_json::json;

#[test]
fn blend_text_gamma_setting() {
    let _process = crate::lock(&crate::PROCESS);
    let mut s = Session::new();
    assert!((s.color().settings.blend_text_gamma - 1.45).abs() < 1e-6);
    assert!(s.color().settings.blend_text_gamma_on);
    assert!(s.execute("edit.colorSettings", json!({"blendTextGamma": 3.01})).is_err());
    // The compositor's (application-wide) gamma follows the setting.
    s.execute("edit.colorSettings", json!({"blendTextGamma": 1.45})).unwrap();
    assert!((photocraft_compose::psblend::text_gamma() - 1.45).abs() < 1e-6);
    // The box off blends in gamma 1 and keeps the field's value.
    let r = s.execute("edit.colorSettings", json!({"blendTextGamma": false})).unwrap();
    assert_eq!((r["settings"]["blendTextGamma"].as_f64().map(|g| (g * 100.0).round()), &r["settings"]["blendTextGammaOn"]), (Some(145.0), &json!(false)));
    assert_eq!(photocraft_compose::psblend::text_gamma(), 1.0);
    s.execute("edit.colorSettings", json!({"blendTextGamma": 1.8})).unwrap();
    assert!((photocraft_compose::psblend::text_gamma() - 1.8).abs() < 1e-6);
    s.execute("edit.colorSettings", json!({"blendTextGamma": false})).unwrap();
    s.execute("edit.colorSettings", json!({"blendTextGamma": true})).unwrap();
    assert!((s.color().settings.blend_text_gamma - 1.8).abs() < 1e-6, "the box on again uses the field's value");
    assert!((photocraft_compose::psblend::text_gamma() - 1.8).abs() < 1e-6);

    // `prefs.set` and `prefs.reset` on `colorSettings` move the compositor's gamma too (#987).
    // Same function, not a second test: tests in one binary run concurrently.
    let in_sync = |s: &Session| (photocraft_compose::psblend::text_gamma() - s.color().settings.text_gamma()).abs() < 1e-6;
    s.execute("edit.colorSettings", json!({"blendTextGamma": 1.8})).unwrap();
    s.execute("prefs.set", json!({"path": "colorSettings.blendTextGammaOn", "value": false})).unwrap();
    assert_eq!(photocraft_compose::psblend::text_gamma(), 1.0);
    assert!(in_sync(&s));
    s.execute("prefs.set", json!({"path": "colorSettings.blendTextGamma", "value": 1.0})).unwrap();
    s.execute("prefs.set", json!({"path": "colorSettings.blendTextGammaOn", "value": true})).unwrap();
    assert_eq!(photocraft_compose::psblend::text_gamma(), 1.0);
    assert!(in_sync(&s));
    s.execute("prefs.reset", json!({"path": "colorSettings.blendTextGamma"})).unwrap();
    assert!((photocraft_compose::psblend::text_gamma() - 1.45).abs() < 1e-6);
    assert!(in_sync(&s));
    for reset in [json!({"path": "colorSettings"}), json!({})] {
        s.execute("edit.colorSettings", json!({"blendTextGamma": 1.8, "blendTextGammaOn": false})).unwrap();
        s.execute("prefs.reset", reset).unwrap();
        assert!((photocraft_compose::psblend::text_gamma() - 1.45).abs() < 1e-6);
        assert!(s.color().settings.blend_text_gamma_on);
        assert!(in_sync(&s));
    }
}
