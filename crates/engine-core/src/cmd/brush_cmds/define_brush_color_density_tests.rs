//! Numeric falsifier for issue #14273; this does not change brush capture.
//!
//! Inputs are encoded sRGB RGB8 samples, composited over encoded white before
//! the cached sRGB -> sGray transform. Density is one minus encoded gray, then
//! multiplied by selection coverage exactly once. Arbitrary document profiles,
//! HDR/non-RGB sources and actual stamping remain outside this probe.

use std::sync::Arc;

use craft_cms::{Builtin, Intent, TransformOptions};

#[test]
fn sgray_matches_six_literal_density_targets_and_alpha_selection_controls() {
    let source = Arc::new(Builtin::Srgb.profile().clone());
    let gray = Arc::new(Builtin::SGray.profile().clone());
    let options = TransformOptions { intent: Intent::RelativeColorimetric, bpc: true, ..Default::default() };
    let transform = craft_cms::cached(&source, &gray, options).unwrap();
    let gray_value = |rgb: [f32; 3]| {
        // Same public API and output scratch size as color_cmds::swatch_in_doc.
        let mut output = [0.0f32; 16];
        transform.eval(&rgb, &mut output).unwrap();
        assert!(output[0].is_finite(), "the CMS must return a finite gray value");
        output[0].clamp(0.0, 1.0)
    };
    let density = |rgb: [f32; 3], alpha: f32, selection: f32| {
        let over_white = rgb.map(|channel| channel * alpha + (1.0 - alpha));
        (1.0 - gray_value(over_white)) * selection
    };

    // Independent measured targets from #14273, not values derived from the
    // candidate transform or the command's existing fixed luminance weights.
    let mut mismatches = Vec::new();
    for (name, rgb, expected, tolerance) in [
        ("red", [255u8, 0, 0], 0.49f32, 0.03f32),
        ("green", [0, 255, 0], 0.14, 0.03),
        ("blue", [0, 0, 255], 0.72, 0.03),
        ("gray 128", [128, 128, 128], 0.50, 0.01),
        ("gray 200", [200, 200, 200], 0.22, 0.01),
        ("black", [0, 0, 0], 1.0, 0.0001),
    ] {
        let actual = density(rgb.map(|value| f32::from(value) / 255.0), 1.0, 1.0);
        eprintln!("{name}: density {actual:.6}, target {expected:.6} +/- {tolerance:.6}");
        if (actual - expected).abs() > tolerance {
            mismatches.push(format!("{name}: {actual:.6}, expected {expected:.6} +/- {tolerance:.6}"));
        }
    }
    assert!(mismatches.is_empty(), "sGray candidate fails measured targets:\n{}", mismatches.join("\n"));

    // Transparency is white, not an extra multiplier applied after the CMS.
    assert!(density([0.0, 0.0, 1.0], 0.0, 1.0).abs() <= 0.0001);
    assert!((density([0.0; 3], 0.5, 1.0) - 0.5).abs() <= 0.01);
    assert!((density([0.0; 3], 1.0, 0.25) - 0.25).abs() <= 0.0001);
    assert!((density([0.0; 3], 0.5, 0.5) - 0.25).abs() <= 0.01, "alpha or selection must not be applied twice");
    assert_eq!(density([1.0, 0.0, 0.0], 1.0, 0.0), 0.0);

    // This is an ordering control, not a new measured coloured-alpha target:
    // half-transparent red is the explicit RGB sample [1, .5, .5] over white.
    let half_red = density([1.0, 0.0, 0.0], 0.5, 1.0);
    let explicit_over_white = 1.0 - gray_value([1.0, 0.5, 0.5]);
    assert_eq!(half_red, explicit_over_white);
}

use super::{GrayTile, Session, TipShape, json};

const COMMANDS: [&str; 2] = ["brush.defineFromSelection", "edit.defineBrushPreset"];

fn rgb_source(depth: u32, rgba: [f32; 4]) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 40, "height": 40, "depth": depth, "background": "#ffffff"})).unwrap();
    s.edit("Colour square", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(10, 10, 30, 30), &rgba);
        Ok(())
    })
    .unwrap();
    s
}

fn tip(s: &Session) -> &GrayTile {
    let TipShape::Sampled(tip) = &*s.tools.brush.tip else { panic!("expected a sampled tip") };
    tip
}

fn define_square(s: &mut Session, command: &str, selected: bool) -> f32 {
    if selected {
        s.execute("select.rect", json!({"x": 8, "y": 8, "width": 24, "height": 24})).unwrap();
    }
    let doc = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.past_len();
    let presets = s.tools.presets.len();
    let result = s.execute(command, json!({"name": "Colour density"})).unwrap();
    assert_eq!((result["width"].as_u64(), result["height"].as_u64()), (Some(20), Some(20)));
    assert_eq!((tip(s).width, tip(s).height), (20, 20));
    assert_eq!(s.tools.presets.len(), presets + 1);
    assert_eq!(s.tools.current_preset.as_deref(), Some("Colour density"));
    assert!(std::sync::Arc::ptr_eq(&doc, &s.active().unwrap().doc));
    assert_eq!(s.active().unwrap().history.past_len(), history);
    assert_eq!(s.active().unwrap().doc.selection.is_some(), selected);
    tip(s).get(10, 10)
}

#[test]
fn defined_tips_match_the_six_measured_density_targets_at_all_depths() {
    for command in COMMANDS {
        for depth in [8, 16, 32] {
            for selected in [false, true] {
                for (name, rgb, expected, tolerance) in [
                    ("red", [255u8, 0, 0], 0.49f32, 0.03f32),
                    ("green", [0, 255, 0], 0.14, 0.03),
                    ("blue", [0, 0, 255], 0.72, 0.03),
                    ("gray 128", [128, 128, 128], 0.50, 0.01),
                    ("gray 200", [200, 200, 200], 0.22, 0.01),
                    ("black", [0, 0, 0], 1.0, 0.0001),
                ] {
                    let [r, g, b] = rgb.map(|v| f32::from(v) / 255.0);
                    let mut s = rgb_source(depth, [r, g, b, 1.0]);
                    let actual = define_square(&mut s, command, selected);
                    assert!((actual - expected).abs() <= tolerance, "{command}, {name}, depth {depth}, selected {selected}: {actual}");
                }
            }
        }
    }
}

#[test]
fn command_density_applies_alpha_before_gray_and_selection_once() {
    for command in COMMANDS {
        for (alpha, coverage, expected) in [(0.5, 1.0, 0.5), (1.0, 0.25, 0.25), (0.5, 0.5, 0.25)] {
            let mut s = rgb_source(32, [0.0, 0.0, 0.0, alpha]);
            s.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
            s.edit("Selection coverage", |doc, _| {
                doc.selection.as_mut().unwrap().fill_rect(photocraft_geom::Rect::new(10, 10, 30, 30), &[coverage]);
                Ok(())
            })
            .unwrap();
            s.execute(command, json!({"name": "Coverage"})).unwrap();
            assert!((tip(&s).get(10, 10) - expected).abs() <= 0.01, "{command}, alpha {alpha}, coverage {coverage}");
        }

        // An explicit over-white RGB square is an independent way to provide the
        // same colour; this is an ordering control, not a new reference target.
        let mut half_red = rgb_source(32, [1.0, 0.0, 0.0, 0.5]);
        let mut over_white = rgb_source(32, [1.0, 0.5, 0.5, 1.0]);
        let half = define_square(&mut half_red, command, false);
        let opaque = define_square(&mut over_white, command, false);
        assert!((half - opaque).abs() <= 1.0 / 65535.0);

        let mut transparent = rgb_source(32, [0.0, 0.0, 1.0, 0.0]);
        let brush = (*transparent.tools.brush).clone();
        let presets = (*transparent.tools.presets).clone();
        let err = transparent.execute(command, json!({"name": "Transparent colour"})).unwrap_err();
        assert!(err.to_string().contains("contain no paint"), "{err}");
        assert_eq!(*transparent.tools.brush, brush);
        assert_eq!(*transparent.tools.presets, presets);
    }
}

#[test]
fn tagged_rgb_uses_the_document_encoding() {
    for command in COMMANDS {
        let mut tagged = rgb_source(32, [0.5, 0.5, 0.5, 1.0]);
        tagged
            .edit("Assign linear RGB", |doc, _| {
                doc.icc_profile = Some(Builtin::LinearSrgb.profile().to_bytes());
                Ok(())
            })
            .unwrap();
        let explicit = craft_cms::cached(Builtin::LinearSrgb.profile(), Builtin::SGray.profile(), TransformOptions::default()).unwrap();
        let mut gray = [0.0f32; 16];
        explicit.eval(&[0.5, 0.5, 0.5], &mut gray).unwrap();
        let actual = define_square(&mut tagged, command, false);
        assert!((actual - (1.0 - gray[0])).abs() <= 1.0 / 65535.0);

        let mut srgb = rgb_source(32, [0.5, 0.5, 0.5, 1.0]);
        let untagged = define_square(&mut srgb, command, false);
        assert!((actual - untagged).abs() > 0.1, "the fixture must distinguish profile encoding");
    }
}

#[test]
fn native_gray_keeps_numeric_density_in_its_existing_rgb_view() {
    for command in COMMANDS {
        for depth in [8, 16, 32] {
            for profile in [None, Some(Builtin::DotGain20Compat), Some(Builtin::GrayGamma22)] {
                for value in [128.0 / 255.0, 200.0 / 255.0, 0.0] {
                    let mut s = crate::session();
                    s.execute("file.new", json!({"width": 40, "height": 40, "mode": "gray", "depth": depth, "background": "white"})).unwrap();
                    s.edit("Native gray square", |doc, active| {
                        doc.icc_profile = profile.map(|p| p.profile().to_bytes());
                        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(10, 10, 30, 30), &[value, 1.0]);
                        Ok(())
                    })
                    .unwrap();
                    let actual = define_square(&mut s, command, false);
                    assert!((actual - (1.0 - value)).abs() <= 0.0001, "{command}, depth {depth}, profile {profile:?}, value {value}: {actual}");
                }
            }
        }
    }
}

#[test]
fn zero_alpha_and_zero_selection_skip_hidden_colour() {
    for command in COMMANDS {
        for transparent in [true, false] {
            for rgb in [[0.0, 0.0, 1.0], [f32::NAN, f32::INFINITY, f32::NEG_INFINITY]] {
                let mut s = rgb_source(32, [0.0, 0.0, 0.0, 1.0]);
                let hole = photocraft_geom::Rect::new(15, 15, 25, 25);
                s.edit("Hidden colour", |doc, active| {
                    let alpha = if transparent { 0.0 } else { 1.0 };
                    doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(hole, &[rgb[0], rgb[1], rgb[2], alpha]);
                    Ok(())
                })
                .unwrap();
                let st = s.active().unwrap();
                let stored = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(20, 20);
                assert_eq!(stored[3], if transparent { 0.0 } else { 1.0 });
                for channel in 0..3 {
                    if rgb[channel].is_nan() {
                        assert!(stored[channel].is_nan(), "the non-finite fixture must reach the source read");
                    } else {
                        assert_eq!(stored[channel], rgb[channel]);
                    }
                }
                if !transparent {
                    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
                    s.edit("Zero coverage hole", |doc, _| {
                        doc.selection.as_mut().unwrap().fill_rect(hole, &[0.0]);
                        Ok(())
                    })
                    .unwrap();
                    // The hole stays inside the sampled area, rather than being
                    // removed by selection bounds before the per-pixel check.
                    assert_eq!(s.active().unwrap().doc.selection.as_ref().unwrap().content_bounds(), photocraft_geom::Rect::new(10, 10, 30, 30));
                }
                let doc = s.active().unwrap().doc.clone();
                let history = s.active().unwrap().history.past_len();
                let presets = s.tools.presets.len();

                let made = s.execute(command, json!({"name": "Visible black border"})).unwrap();

                assert_eq!((made["width"].as_u64(), made["height"].as_u64()), (Some(20), Some(20)));
                assert_eq!(tip(&s).get(0, 0), 1.0);
                assert_eq!(tip(&s).get(19, 19), 1.0);
                assert_eq!(tip(&s).get(5, 5), 0.0, "hidden colour contributes no paint");
                assert_eq!(tip(&s).get(14, 14), 0.0);
                assert_eq!(s.tools.presets.len(), presets + 1);
                assert_eq!(s.tools.current_preset.as_deref(), Some("Visible black border"));
                assert!(std::sync::Arc::ptr_eq(&doc, &s.active().unwrap().doc));
                assert_eq!(s.active().unwrap().history.past_len(), history);
            }
        }
    }
}

#[test]
fn sampled_trcs_cannot_hide_non_finite_contributing_channels() {
    let mut profile = Builtin::Srgb.profile().clone();
    profile.trc = profile.trc.take().map(|curves| curves.map(|curve| craft_cms::Curve::Table(curve.sample(256))));
    // Reset the built-in's encoded bytes and cache key after replacing its curves.
    let icc = profile.with_encoded_bytes().to_bytes();
    let parsed = craft_cms::Profile::parse(&icc).unwrap();
    assert!(parsed.trc.as_ref().unwrap().iter().all(|curve| matches!(curve, craft_cms::Curve::Table(_))));
    let options = TransformOptions { intent: Intent::RelativeColorimetric, bpc: true, ..Default::default() };
    let gray = craft_cms::cached(&parsed, Builtin::SGray.profile(), options).unwrap();
    let mut converted = [0.0f32; 16];
    gray.eval(&[f32::INFINITY, 0.0, 0.0], &mut converted).unwrap();
    assert!(converted[0].is_finite(), "the sampled-TRC fixture must expose clamping before the command's gray check");

    for command in COMMANDS {
        for selected in [false, true] {
            let mut finite = rgb_source(32, [1.0, 0.0, 0.0, 1.0]);
            finite
                .edit("Assign sampled sRGB", |doc, _| {
                    doc.icc_profile = Some(icc.clone());
                    Ok(())
                })
                .unwrap();
            assert!((define_square(&mut finite, command, selected) - 0.49).abs() <= 0.03);

            for rgba in [
                [f32::INFINITY, 0.0, 0.0, 1.0],
                [0.0, f32::INFINITY, 0.0, 1.0],
                [0.0, 0.0, f32::INFINITY, 1.0],
                [0.0, 0.0, 0.0, f32::INFINITY],
                [f32::NAN, 0.0, 0.0, 1.0],
                [f32::NEG_INFINITY, 0.0, 0.0, 1.0],
            ] {
                let mut s = rgb_source(32, rgba);
                s.edit("Assign sampled sRGB", |doc, _| {
                    doc.icc_profile = Some(icc.clone());
                    Ok(())
                })
                .unwrap();
                if selected {
                    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 24, "height": 24})).unwrap();
                }
                // An accepted invalid source would replace this existing preset.
                s.execute("brush.presets.save", json!({"name": "Kept preset"})).unwrap();
                let active = s.active().unwrap().active_layer.unwrap();
                let stored = s.active().unwrap().doc.layer(active).unwrap().surface().unwrap().rgba(20, 20).map(f32::to_bits);
                assert_eq!(stored, rgba.map(f32::to_bits), "non-finite values must reach the source read unchanged");
                let doc = s.active().unwrap().doc.clone();
                let history = s.active().unwrap().history.past_len();
                let brush = (*s.tools.brush).clone();
                let presets = (*s.tools.presets).clone();
                let current = s.tools.current_preset.clone();
                let revision = s.tools.brush_rev;
                let journal = s.journal.clone();

                let error = s.execute(command, json!({"name": "Kept preset"})).unwrap_err();

                assert!(error.to_string().contains("non-finite"), "{command}: {error}");
                assert!(Arc::ptr_eq(&doc, &s.active().unwrap().doc));
                assert_eq!(s.active().unwrap().history.past_len(), history);
                assert_eq!(s.active().unwrap().active_layer, Some(active));
                assert_eq!(s.active().unwrap().doc.selection.is_some(), selected);
                assert_eq!(s.active().unwrap().doc.icc_profile.as_ref(), Some(&icc));
                assert_eq!(s.active().unwrap().doc.layer(active).unwrap().surface().unwrap().rgba(20, 20).map(f32::to_bits), stored);
                assert_eq!(*s.tools.brush, brush);
                assert_eq!(*s.tools.presets, presets);
                assert_eq!(s.tools.current_preset, current);
                assert_eq!(s.tools.brush_rev, revision);
                assert_eq!(s.journal, journal);
            }
        }
    }
}
