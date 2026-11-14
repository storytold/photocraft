//! The Levels and Curves eyedroppers' target colours (#10091): the `targetColors` preference and
//! the sample-to-target mapping at every bit depth.

use super::*;

fn pixel(s: &mut crate::Session) -> [f32; 3] {
    let px: Vec<f32> = serde_json::from_value(s.execute("document.pixel", json!({"x": 2, "y": 2})).unwrap()).unwrap();
    [px[0], px[1], px[2]]
}

fn filled(depth: u32, color: [f32; 3]) -> crate::Session {
    let mut s = crate::Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "depth": depth})).unwrap();
    s.execute("edit.fill", json!({"color": color})).unwrap();
    s
}

fn set_target(s: &mut crate::Session, key: &str, color: &str) {
    s.execute("prefs.set", json!({"path": format!("targetColors.{key}"), "value": color})).unwrap();
}

fn rgb(hex: &str) -> [f32; 3] {
    color_of("test", "hex", &json!(hex)).unwrap()
}

fn close(a: [f32; 3], b: [f32; 3], tolerance: f32) -> bool {
    a.iter().zip(b).all(|(a, b)| (a - b).abs() <= tolerance)
}

#[test]
fn target_colors_default_to_photoshops_and_persist() {
    let s = crate::Session::new();
    let t = &s.prefs().target_colors;
    assert_eq!((t.shadows.as_str(), t.midtones.as_str(), t.highlights.as_str()), ("#000000", "#808080", "#ffffff"));
    for picker in [CurvesEyedropper::Black, CurvesEyedropper::NeutralGray, CurvesEyedropper::White] {
        assert_eq!(picker.target_in(&s), picker.default_target(), "{picker:?}");
    }
    assert_eq!(t.rgb("nope"), None);

    let mut s = crate::Session::new();
    set_target(&mut s, "shadows", "#102030");
    set_target(&mut s, "midtones", "#a08060");
    set_target(&mut s, "highlights", "#f0e0c8");
    let saved = s.prefs_to_json();
    let mut restored = crate::Session::new();
    assert!(restored.load_prefs_json(&saved).unwrap().is_empty());
    assert_eq!(restored.prefs().target_colors, s.prefs().target_colors, "kept across restarts");
    assert_eq!(CurvesEyedropper::Black.target_in(&restored), rgb("#102030"));
    assert_eq!(s.execute("prefs.get", json!({"path": "targetColors.midtones"})).unwrap(), json!("#a08060"));

    // A stored value that isn't a colour (a hand-edited file) falls back to the default.
    let mut odd = s.prefs().clone();
    odd.target_colors.highlights = "white".into();
    assert_eq!(odd.target_colors.rgb("highlights"), Some([1.0; 3]));

    s.execute("prefs.reset", json!({"path": "targetColors"})).unwrap();
    assert_eq!(s.prefs().target_colors, crate::prefs::TargetColors::default());
}

#[test]
fn bad_target_colors_are_errors_not_panics() {
    let mut s = filled(8, [0.6, 0.5, 0.4]);
    for (path, value) in [
        ("targetColors.shadows", json!("red")),
        ("targetColors.shadows", json!("#12345")),
        ("targetColors.midtones", json!(5)),
        ("targetColors.highlights", json!([1, 1, 1])),
        ("targetColors.highlights", json!("#ﬀﬀﬀ")),
        ("targetColors.nope", json!("#000000")),
    ] {
        assert!(s.execute("prefs.set", json!({"path": path, "value": value})).is_err(), "{path} = {value}");
    }
    assert_eq!(s.prefs().target_colors, crate::prefs::TargetColors::default(), "nothing stored");
    for target in [json!("nope"), json!(5), json!([1, "x", 0]), json!([0.5]), json!({"r": 1}), json!("#ééé")] {
        for kind in ["curves", "levels"] {
            let p = json!({"eyedropper": {"point": "white", "at": [2, 2], "target": target}});
            assert!(s.execute(&format!("image.adjustments.{kind}"), p).is_err(), "{kind}: {target}");
        }
    }
    for picker in [CurvesEyedropper::Black, CurvesEyedropper::NeutralGray, CurvesEyedropper::White] {
        for target in [[f32::NAN, 0.0, 0.0], [0.0, f32::INFINITY, 0.0]] {
            assert!(curves_eyedropper_to(&json!({}), picker, [0.4; 3], target, ColorMode::Rgb).is_err());
            assert!(levels_eyedropper_to(&json!({}), picker, [0.4; 3], target, ColorMode::Rgb).is_err());
        }
        // Out-of-range targets are clamped, not refused.
        assert!(curves_eyedropper_to(&json!({}), picker, [0.4; 3], [-3.0, 2.0, 0.5], ColorMode::Rgb).is_ok());
    }
}

/// The default targets give the same adjustment as the plain eyedropper: an explicit default target
/// and the preference at its default.
#[test]
fn default_targets_leave_the_eyedroppers_unchanged() {
    let base = json!({"points": [[0, 8], [120, 150], [255, 245]], "red": [[0, 0], [100, 130], [255, 255]]});
    let levels_base = json!({"outBlack": 10, "red": {"outWhite": 240}});
    for picker in [CurvesEyedropper::Black, CurvesEyedropper::NeutralGray, CurvesEyedropper::White] {
        let s = crate::Session::new();
        let target = picker.target_in(&s);
        let sample = [0.3, 0.45, 0.6];
        assert_eq!(
            curves_eyedropper_to(&base, picker, sample, target, ColorMode::Rgb).unwrap(),
            curves_eyedropper(&base, picker, sample, ColorMode::Rgb).unwrap()
        );
        assert_eq!(
            levels_eyedropper_to(&levels_base, picker, sample, target, ColorMode::Rgb).unwrap(),
            levels_eyedropper(&levels_base, picker, sample, ColorMode::Rgb).unwrap()
        );
    }
    // Default black/white targets keep a Levels channel's own output levels.
    let Adjustment::Levels { per_channel, .. } = levels_eyedropper(&levels_base, CurvesEyedropper::White, [0.6; 3], ColorMode::Rgb).unwrap() else { panic!() };
    assert!((per_channel[0].out_white - 240.0 / 255.0).abs() < 1e-6);
    // The plain command with the default preference and with an explicit default target agree.
    for kind in ["curves", "levels"] {
        let mut a = filled(8, [0.6, 0.5, 0.4]);
        a.execute(&format!("image.adjustments.{kind}"), json!({"eyedropper": {"point": "white", "at": [2, 2]}})).unwrap();
        let mut b = filled(8, [0.6, 0.5, 0.4]);
        b.execute(&format!("image.adjustments.{kind}"), json!({"eyedropper": {"point": "white", "at": [2, 2], "target": "#ffffff"}})).unwrap();
        assert_eq!(pixel(&mut a), pixel(&mut b), "{kind}");
    }
}

/// Set Black and Set White Point map the sampled pixel to a non-default target colour, through the
/// preference, in Curves and Levels, at 8, 16 and 32 bits.
#[test]
fn black_and_white_points_map_the_sample_to_the_target_at_every_depth() {
    let cases = [("black", "shadows", [0.3, 0.35, 0.45], "#302010"), ("white", "highlights", [0.6, 0.5, 0.4], "#f0e0c8")];
    for depth in [8, 16, 32] {
        for kind in ["curves", "levels"] {
            for (point, key, fill, target) in cases {
                let mut s = filled(depth, fill);
                let sample = pixel(&mut s);
                set_target(&mut s, key, target);
                s.execute(&format!("image.adjustments.{kind}"), json!({"eyedropper": {"point": point, "at": [2, 2]}})).unwrap();
                let got = pixel(&mut s);
                assert!(close(got, rgb(target), 1.5 / 255.0), "{kind} {point} @{depth}: {sample:?} → {got:?}, want {target}");
            }
        }
    }
}

/// Set Gray Point with a coloured midtone target keeps the sample's luma and gives it the target's
/// cast: each channel sits as far from the luma as the target's does from its own.
#[test]
fn gray_point_gives_the_sample_the_targets_cast_at_every_depth() {
    let luma = |c: [f32; 3]| c[0] * 0.2126 + c[1] * 0.7152 + c[2] * 0.0722;
    let target = rgb("#a08060");
    for depth in [8, 16, 32] {
        for kind in ["curves", "levels"] {
            let mut s = filled(depth, [0.45, 0.5, 0.6]);
            let before = pixel(&mut s);
            set_target(&mut s, "midtones", "#a08060");
            s.execute(&format!("image.adjustments.{kind}"), json!({"eyedropper": {"point": "gray", "at": [2, 2]}})).unwrap();
            let got = pixel(&mut s);
            let want = target.map(|t| luma(before) + t - luma(target));
            assert!(close(got, want, 2.5 / 255.0), "{kind} @{depth}: {before:?} → {got:?}, want {want:?}");
        }
    }
}

/// The Properties panel's pickers on an adjustment layer use the same targets (one undoable step).
#[test]
fn adjustment_layer_pickers_use_the_target_colors() {
    for kind in ["curves", "levels"] {
        let mut s = filled(16, [0.6, 0.5, 0.4]);
        set_target(&mut s, "highlights", "#e0d0c0");
        s.execute(&format!("layer.newAdjustmentLayer.{kind}"), json!({})).unwrap();
        s.execute("layer.setAdjustment", json!({"eyedropper": {"point": "white", "at": [2, 2]}})).unwrap();
        assert!(close(pixel(&mut s), rgb("#e0d0c0"), 1.5 / 255.0), "{kind}: {:?}", pixel(&mut s));
        // An explicit target overrides the preference.
        s.execute("layer.setAdjustment", json!({"eyedropper": {"point": "white", "at": [2, 2], "target": "#c0c8d0"}})).unwrap();
        assert!(close(pixel(&mut s), rgb("#c0c8d0"), 1.5 / 255.0), "{kind}: {:?}", pixel(&mut s));
    }
}

/// Grayscale documents read a target as its gray level.
#[test]
fn grayscale_pickers_map_to_the_targets_gray_level() {
    for kind in ["curves", "levels"] {
        let mut s = crate::Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8, "mode": "grayscale"})).unwrap();
        s.execute("edit.fill", json!({"color": [0.3, 0.3, 0.3]})).unwrap();
        set_target(&mut s, "shadows", "#404040");
        s.execute(&format!("image.adjustments.{kind}"), json!({"eyedropper": {"point": "black", "at": [2, 2]}})).unwrap();
        let got = pixel(&mut s);
        assert!((got[0] - 64.0 / 255.0).abs() < 1.5 / 255.0, "{kind}: {got:?}");
    }
}
