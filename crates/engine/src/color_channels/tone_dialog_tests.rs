//! Dialog normalization must retain the command's native per-component interpretation.
use super::*;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;

/// Map inputs, explicit point precedence and native records survive normalization for every
/// multi-component subset, including full explicit sets and CMYK Black.
#[test]
fn tone_dialog_normalization_matches_native_execution() {
    for mode in [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab] {
        let count = mode.color_channels();
        let keys = crate::adjust_params::channel_keys(crate::adjust_params::default_space(mode));
        for bits in 1_u32..(1 << count) {
            if bits.count_ones() < 2 {
                continue;
            }
            let colors = ColorChannels::from_indices((0..count).filter(|k| bits & (1 << k) != 0), count).unwrap();
            for raw in [
                json!({"gray":{"gamma":2},"gamma":1.25,"inBlack":8}),
                json!({"gray":{"gamma":1.234567},"gamma":1.25,"inBlack":8.123456}),
                json!({"gray":"ignored","points":[[0,20],[255,240]]}),
                json!({"gray":[[0,0],[100.123456,200.654321],[255,255]]}),
                json!({"gray":(0..256).map(|i| (i * 37 % 256) as f32).collect::<Vec<_>>(),"pencil":true,"smooth":3}),
            ] {
                let kind = if raw.get("gamma").is_some() { "levels" } else { "curves" };
                let mut raw = raw;
                for key in keys {
                    raw[*key] = if kind == "levels" {
                        json!({"gamma":1.5})
                    } else if raw["pencil"] == true {
                        json!((0..256).map(|i| (i * 17 % 256) as f32).collect::<Vec<_>>())
                    } else {
                        json!([[0, 0], [100, 180], [255, 255]])
                    };
                }
                let canonical = tone_dialog_params(kind, &raw, mode, colors).unwrap();
                assert!(canonical.get("gray").is_none() && canonical.get("smooth").is_none());
                let parsed = |p| tone_adjustment(kind, p, crate::adjust_params::from_params(kind, p, None, mode).unwrap(), mode, colors).unwrap();
                for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                    let bounds = Rect::new(0, 0, 8, 8);
                    let mut original = Surface::new(PixelFormat::new(mode, depth, true));
                    let values: Vec<_> = (0..count).map(|i| (i as f32 + 1.0) / 7.0).chain([0.65]).collect();
                    original.fill_rect(bounds, &values);
                    let mut expected = original.clone();
                    tone::adjust(&mut expected, colors, &parsed(&raw), None, bounds).unwrap();
                    let mut actual = original;
                    tone::adjust(&mut actual, colors, &parsed(&canonical), None, bounds).unwrap();
                    assert_eq!(actual, expected, "{mode:?} {depth:?} {bits} {kind}");
                }
            }
        }
    }
}

/// Invalid kinds, component sets and malformed aliases are returned as errors, never panics.
#[test]
fn tone_dialog_normalization_rejects_invalid_requests() {
    let rgb = ColorChannels::from_indices([0, 2], 3).unwrap();
    assert!(tone_dialog_params("invert", &json!({}), ColorMode::Rgb, rgb).is_err());
    assert!(tone_dialog_params("levels", &json!({}), ColorMode::Grayscale, rgb).is_err());
    assert!(tone_dialog_params("levels", &json!({}), ColorMode::Rgb, ColorChannels::from_indices([0, 3], 4).unwrap()).is_err());
    assert!(tone_dialog_params("levels", &json!({}), ColorMode::Rgb, ColorChannels::from_indices([0], 3).unwrap()).is_err());
    for mode in [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab] {
        for (kind, p) in [("levels", json!({"gray":{"gamma":"bad"}})), ("curves", json!({"gray":[[0,0]]})), ("curves", json!({"gray":[0,1,2],"pencil":true}))] {
            assert!(tone_dialog_params(kind, &p, mode, rgb).is_err(), "{mode:?} {kind}");
        }
    }
}
