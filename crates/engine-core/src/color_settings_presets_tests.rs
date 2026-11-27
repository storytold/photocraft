//! Edit > Color Settings' Settings menu (#14229): the 13 named setups, the stored preset name,
//! and "Custom" once anything changes.
//!
//! Running a setup sets the process-wide Spot dot gain, which races every test that renders a
//! Multichannel document. So the command runs here only with setups whose Spot is the default
//! Dot Gain 20 %; `crates/engine/tests/global_state/color_settings_presets.rs` runs all of them
//! in the `global_state` binary.

use super::*;

const CMD: &str = "edit.colorSettings";

fn setup(name: &str) -> ColorSettings {
    let s = crate::session();
    s.color().preset_settings(&s.color().settings, name).unwrap()
}

#[test]
fn the_menu_lists_the_thirteen_setups_in_order() {
    assert_eq!(COLOR_SETTINGS_PRESETS.len(), 13);
    assert_eq!(COLOR_SETTINGS_PRESETS.map(|n| n.to_string()), SETTINGS_PRESETS.map(|p| p.name.to_string()));
    assert_eq!(COLOR_SETTINGS_PRESETS[0], "Europe General Purpose 3");
    assert_eq!(COLOR_SETTINGS_PRESETS[3], "Monitor Color");
    assert_eq!(COLOR_SETTINGS_PRESETS[12], "North America Web/Internet");
    // The params doc offers Custom, then the same names.
    crate::install().unwrap();
    let spec = crate::commands::find(CMD).unwrap().params;
    let names = format!("\"preset\":\"{CUSTOM_SETTINGS}|{}\"", COLOR_SETTINGS_PRESETS.join("|"));
    assert!(spec.contains(&names), "{spec}");
}

#[test]
fn each_setup_sets_the_measured_values() {
    use Policy::{Convert as C, Off, Preserve as P};
    // Name, RGB, CMYK, Gray, Spot, policies, Ask open/paste/missing, intent.
    type Row = (&'static str, &'static str, &'static str, &'static str, f32, [Policy; 3], [bool; 3], &'static str);
    let rows: [Row; 13] = [
        ("Europe General Purpose 3", "srgb", "coated-fogra39", "dot-gain-20-compat", 15.0, [P, P, P], [false, false, false], "relative"),
        ("Europe Prepress 3", "adobe-rgb-compat", "coated-fogra39", "dot-gain-20-compat", 15.0, [P, P, P], [true, true, true], "relative"),
        ("Europe Web/Internet 2", "srgb", "coated-fogra39", "gray-gamma-2.2", 15.0, [C, P, P], [true, true, false], "relative"),
        // Without a display profile, Monitor RGB is sRGB.
        ("Monitor Color", "srgb", "coated-cmyk", "gray-gamma-2.2", 20.0, [Off, Off, Off], [true, false, false], "relative"),
        ("Japan Color for Newspaper", "adobe-rgb-compat", "coated-cmyk", "dot-gain-20-compat", 25.0, [P, P, P], [true, true, true], "perceptual"),
        ("Japan General Purpose 2", "srgb", "coated-cmyk", "dot-gain-20-compat", 15.0, [P, P, P], [false, false, false], "perceptual"),
        ("Japan Magazine Advertisement Color", "adobe-rgb-compat", "coated-cmyk", "dot-gain-20-compat", 15.0, [P, P, P], [true, true, true], "perceptual"),
        ("Japan Prepress 2", "adobe-rgb-compat", "coated-cmyk", "dot-gain-20-compat", 15.0, [P, P, P], [true, true, true], "perceptual"),
        ("Japan Web/Internet", "srgb", "coated-cmyk", "gray-gamma-2.2", 15.0, [C, P, P], [true, true, false], "perceptual"),
        ("North America General Purpose 2", "srgb", "coated-cmyk", "dot-gain-20-compat", 20.0, [P, P, P], [false, false, false], "relative"),
        ("North America Newspaper", "adobe-rgb-compat", "coated-cmyk", "dot-gain-20-compat", 25.0, [P, P, P], [true, true, true], "perceptual"),
        ("North America Prepress 2", "adobe-rgb-compat", "coated-cmyk", "dot-gain-20-compat", 20.0, [P, P, P], [true, true, true], "relative"),
        ("North America Web/Internet", "srgb", "coated-cmyk", "gray-gamma-2.2", 20.0, [C, P, P], [true, true, false], "relative"),
    ];
    for (name, rgb, cmyk, gray, spot, policies, ask, intent) in rows {
        let c = setup(name);
        assert_eq!((c.working_rgb.as_str(), c.working_cmyk.as_str(), c.working_gray.as_str()), (rgb, cmyk, gray), "{name}");
        assert_eq!(c.spot_dot_gain, spot, "{name}");
        assert_eq!([c.policy_rgb, c.policy_cmyk, c.policy_gray], policies, "{name}");
        assert_eq!([c.ask_on_mismatch, c.ask_on_paste, c.ask_on_missing], ask, "{name}");
        assert_eq!(c.intent, intent, "{name}");
        // Every setup: Black Point Compensation and Dither on, Blend Text Colors Using Gamma 1.45.
        assert!(c.bpc && c.dither, "{name}");
        assert_eq!(c.blend_text_gamma, photocraft_compose::psblend::TEXT_GAMMA, "{name}");
        assert_eq!(c.preset, name);
        // Each resolves to working spaces PhotoCraft can use.
        validate_settings(&c).unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}

#[test]
fn monitor_color_takes_a_named_monitor_profile() {
    let mut s = crate::session();
    s.color_mut().settings.monitor_profile = "display-p3".into();
    let c = s.color().preset_settings(&s.color().settings, "Monitor Color").unwrap();
    assert_eq!(c.working_rgb, "display-p3");
    // The monitor profile itself is not part of the setup.
    assert_eq!(c.monitor_profile, "display-p3");
    // Scripted with a Monitor Profile changed in the same call, it follows that profile.
    let r = s.execute(CMD, json!({"preset": "Monitor Color", "monitorProfile": "prophoto-compat"})).unwrap();
    assert_eq!(r["preset"], "Monitor Color");
    assert_eq!(r["settings"]["workingRgb"], "prophoto-compat");
    // With `auto` and no display profile read, it is sRGB.
    let r = s.execute(CMD, json!({"preset": "Monitor Color", "monitorProfile": "auto"})).unwrap();
    assert_eq!((&r["preset"], &r["settings"]["workingRgb"]), (&json!("Monitor Color"), &json!("srgb")));
}

#[test]
fn a_later_monitor_profile_change_makes_monitor_color_custom() {
    let mut s = crate::session();
    let r = s.execute(CMD, json!({"preset": "Monitor Color", "monitorProfile": "srgb"})).unwrap();
    assert_eq!((&r["preset"], &r["settings"]["workingRgb"]), (&json!("Monitor Color"), &json!("srgb")));
    // Monitor Color would now set Display P3, so the sRGB values left behind are Custom.
    let r = s.execute(CMD, json!({"monitorProfile": "display-p3"})).unwrap();
    assert_eq!((&r["preset"], &r["settings"]["workingRgb"]), (&json!(CUSTOM_SETTINGS), &json!("srgb")));
    // A monitor change that leaves the setup as Monitor Color would set it keeps the name.
    let r = s.execute(CMD, json!({"preset": "Monitor Color", "monitorProfile": "srgb"})).unwrap();
    assert_eq!(r["preset"], "Monitor Color");
    let r = s.execute(CMD, json!({"monitorProfile": "auto"})).unwrap();
    assert_eq!(r["preset"], "Monitor Color");
    // Other setups don't follow the monitor profile.
    s.execute(CMD, json!({"preset": "North America Prepress 2"})).unwrap();
    let r = s.execute(CMD, json!({"monitorProfile": "display-p3"})).unwrap();
    assert_eq!(r["preset"], "North America Prepress 2");
}

#[test]
fn fresh_settings_are_custom() {
    let mut s = crate::session();
    assert_eq!(ColorSettings::default().preset, CUSTOM_SETTINGS);
    let r = s.execute(CMD, json!({})).unwrap();
    assert_eq!(r["preset"], CUSTOM_SETTINGS);
    assert_eq!(r["settings"]["preset"], CUSTOM_SETTINGS);
}

#[test]
fn a_scripted_setup_is_stored_and_reported() {
    let mut s = crate::session();
    // Blend Text Colors Using Gamma off at 1.8: a scripted setup turns it back on at 1.45.
    s.color_mut().settings.blend_text_gamma = 1.8;
    s.color_mut().settings.blend_text_gamma_on = false;
    let r = s.execute(CMD, json!({"preset": "North America Prepress 2"})).unwrap();
    assert_eq!(r["preset"], "North America Prepress 2");
    let c = &s.color().settings;
    assert_eq!(c.preset, "North America Prepress 2");
    assert_eq!((c.working_rgb.as_str(), c.policy_rgb, c.ask_on_missing), ("adobe-rgb-compat", Policy::Preserve, true));
    assert_eq!((c.blend_text_gamma, c.blend_text_gamma_on), (photocraft_compose::psblend::TEXT_GAMMA, true));
    // Reading the settings again keeps the name.
    assert_eq!(s.execute(CMD, json!({})).unwrap()["preset"], "North America Prepress 2");
    // So does setting a value to what it already is, and changing the monitor profile.
    assert_eq!(s.execute(CMD, json!({"bpc": true, "monitorProfile": "srgb"})).unwrap()["preset"], "North America Prepress 2");
}

#[test]
fn any_change_makes_it_custom() {
    let changes = [
        json!({"workingRgb": "srgb"}),
        json!({"policyCmyk": "off"}),
        json!({"askOnPaste": false}),
        json!({"intent": "perceptual"}),
        json!({"bpc": false}),
        json!({"dither": false}),
        // Blend Text Colors Using Gamma too: in `tests/global_state/color_settings_presets.rs`, as it is process-wide.
        json!({"reset": true}),
    ];
    for change in changes {
        let mut s = crate::session();
        s.execute(CMD, json!({"preset": "North America Prepress 2"})).unwrap();
        let r = s.execute(CMD, change.clone()).unwrap();
        assert_eq!(r["preset"], CUSTOM_SETTINGS, "{change}");
    }
}

#[test]
fn the_dialogs_ok_keeps_the_name_with_its_own_text_gamma() {
    // OK in the dialog sends the picked setup with every value, and Blend Text Colors Using Gamma
    // as the dialog kept it: the name stays, and so does that gamma.
    let mut s = crate::session();
    let mut params = serde_json::to_value(setup("North America Web/Internet")).unwrap();
    params["blendTextGamma"] = json!(1.45);
    params["preset"] = json!("North America Web/Internet");
    let r = s.execute(CMD, params.clone()).unwrap();
    assert_eq!(r["preset"], "North America Web/Internet");
    // Changed after picking it, the dialog shows and sends Custom.
    params["preset"] = json!(CUSTOM_SETTINGS);
    params["bpc"] = json!(false);
    let r = s.execute(CMD, params).unwrap();
    assert_eq!(r["preset"], CUSTOM_SETTINGS);
    assert!(!s.color().settings.bpc);
}

#[test]
fn a_setup_with_a_differing_value_is_custom() {
    let mut s = crate::session();
    let r = s.execute(CMD, json!({"preset": "Monitor Color", "policyRgb": "preserve"})).unwrap();
    assert_eq!(r["preset"], CUSTOM_SETTINGS);
    // The setup's other values still apply.
    let c = &s.color().settings;
    assert_eq!((c.policy_rgb, c.policy_cmyk, c.working_gray.as_str()), (Policy::Preserve, Policy::Off, "gray-gamma-2.2"));
}

#[test]
fn custom_keeps_the_values() {
    let mut s = crate::session();
    s.execute(CMD, json!({"preset": "North America General Purpose 2"})).unwrap();
    let r = s.execute(CMD, json!({"preset": CUSTOM_SETTINGS})).unwrap();
    assert_eq!(r["preset"], CUSTOM_SETTINGS);
    assert!(!s.color().settings.ask_on_mismatch, "the setup's values stay");
}

#[test]
fn a_bad_preset_is_an_error_and_changes_nothing() {
    let mut s = crate::session();
    s.execute(CMD, json!({"preset": "North America General Purpose 2"})).unwrap();
    let before = s.color().settings.clone();
    for bad in [json!(null), json!("Europe"), json!(""), json!("custom"), json!(3), json!(true), json!(["Monitor Color"]), json!({"name": "Monitor Color"})] {
        let e = s.execute(CMD, json!({"preset": bad.clone(), "bpc": false})).unwrap_err();
        assert!(matches!(e, EngineError::BadParams { .. }), "{bad}: {e}");
        assert_eq!(s.color().settings, before, "{bad}");
    }
    // A stored name the menu doesn't have (a hand-edited preferences file) is refused too.
    let mut c = before.clone();
    c.preset = "Nowhere".into();
    assert!(validate_settings(&c).is_err());
}
