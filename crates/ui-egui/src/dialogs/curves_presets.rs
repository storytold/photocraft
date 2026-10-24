//! Curves presets (#9221): starting curves for the Curves editor's Preset menu, named for the looks
//! they give.
//!
//! These curves are PhotoCraft's own. Each was designed for its look: a gentle S that steepens the
//! midtones for the contrast presets, a midtone lift or drop for Lighter and Darker, an inverted
//! line for Negative, and per-channel curves after the photographic processes for Color Negative
//! (an inverted image on an orange base) and Cross Process (contrasty red and green, blue shadows
//! and yellow highlights). None of them is taken or measured from another application's presets.

use serde_json::{Value, json};

/// A preset: its id (also the menu value), label, the composite curve and, for colour presets,
/// the red, green and blue curves (RGB documents only). Points are 8-bit `[input, output]`.
pub struct CurvesPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub composite: &'static [[f32; 2]],
    pub channels: Option<[&'static [[f32; 2]]; 3]>,
}

const IDENTITY: &[[f32; 2]] = &[[0.0, 0.0], [255.0, 255.0]];

/// The menu, Default first.
pub const PRESETS: &[CurvesPreset] = &[
    CurvesPreset { id: "default", label: "Default", composite: IDENTITY, channels: None },
    CurvesPreset {
        id: "colorNegative",
        label: "Color Negative",
        composite: IDENTITY,
        channels: Some([&[[0.0, 240.0], [255.0, 48.0]], &[[0.0, 176.0], [255.0, 24.0]], &[[0.0, 112.0], [255.0, 8.0]]]),
    },
    CurvesPreset {
        id: "crossProcess",
        label: "Cross Process",
        composite: IDENTITY,
        channels: Some([
            &[[0.0, 0.0], [64.0, 44.0], [192.0, 220.0], [255.0, 255.0]],
            &[[0.0, 0.0], [64.0, 48.0], [192.0, 212.0], [255.0, 240.0]],
            &[[0.0, 36.0], [255.0, 200.0]],
        ]),
    },
    CurvesPreset { id: "darker", label: "Darker", composite: &[[0.0, 0.0], [128.0, 104.0], [255.0, 255.0]], channels: None },
    CurvesPreset { id: "increaseContrast", label: "Increase Contrast", composite: &[[0.0, 0.0], [64.0, 56.0], [192.0, 200.0], [255.0, 255.0]], channels: None },
    CurvesPreset { id: "lighter", label: "Lighter", composite: &[[0.0, 0.0], [128.0, 152.0], [255.0, 255.0]], channels: None },
    CurvesPreset { id: "linearContrast", label: "Linear Contrast", composite: &[[0.0, 0.0], [32.0, 16.0], [224.0, 240.0], [255.0, 255.0]], channels: None },
    CurvesPreset { id: "mediumContrast", label: "Medium Contrast", composite: &[[0.0, 0.0], [64.0, 48.0], [192.0, 208.0], [255.0, 255.0]], channels: None },
    CurvesPreset { id: "negative", label: "Negative", composite: &[[0.0, 255.0], [255.0, 0.0]], channels: None },
    CurvesPreset { id: "strongContrast", label: "Strong Contrast", composite: &[[0.0, 0.0], [64.0, 36.0], [192.0, 220.0], [255.0, 255.0]], channels: None },
];

const CHANNEL_KEYS: [&str; 3] = ["red", "green", "blue"];

/// The presets offered for a document: colour presets need RGB channels.
pub fn available(gray: bool) -> impl Iterator<Item = &'static CurvesPreset> {
    PRESETS.iter().filter(move |p| !gray || p.channels.is_none())
}

pub fn find(id: &str) -> Option<&'static CurvesPreset> {
    PRESETS.iter().find(|p| p.id == id)
}

fn points(curve: &[[f32; 2]]) -> Value {
    json!(curve.iter().map(|q| [q[0], q[1]]).collect::<Vec<_>>())
}

/// Sets the Curves values to `preset`: point curves (the pencil off), the composite and, in colour,
/// each channel's curve (straight where the preset leaves a channel alone).
pub fn apply(v: &mut Value, preset: &CurvesPreset, gray: bool) {
    v["pencil"] = json!(false);
    v["points"] = points(preset.composite);
    if !gray {
        for (i, key) in CHANNEL_KEYS.into_iter().enumerate() {
            v[key] = points(preset.channels.map_or(IDENTITY, |c| c[i]));
        }
    }
}

fn same(a: &[[f32; 2]], b: &[[f32; 2]]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(p, q)| (p[0] - q[0]).abs() < 0.01 && (p[1] - q[1]).abs() < 0.01)
}

/// The preset the Curves values are, or `None` (Custom) once edited away from every preset.
pub fn matching(v: &Value, gray: bool) -> Option<&'static CurvesPreset> {
    if v.get("pencil").and_then(Value::as_bool).unwrap_or(false) {
        return None;
    }
    let curve = |key: &str| crate::adjust_editors::read_curve(v, key);
    available(gray).find(|p| {
        same(&curve("points"), p.composite)
            && (gray || CHANNEL_KEYS.iter().enumerate().all(|(i, key)| same(&curve(key), p.channels.map_or(IDENTITY, |c| c[i]))))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::adjust::CurvePoint;

    fn lut(curve: &[[f32; 2]]) -> Vec<f32> {
        let cp: Vec<CurvePoint> = curve.iter().map(|q| CurvePoint { input: q[0] / 255.0, output: q[1] / 255.0 }).collect();
        photocraft_compose::adjust::curve_lut(&cp)
    }

    fn at(lut: &[f32], x: f32) -> f32 {
        let i = ((x / 255.0) * (lut.len() - 1) as f32).round() as usize;
        lut[i] * 255.0
    }

    /// Each preset does what its name says, and is the preset the editor shows once applied.
    #[test]
    fn presets_give_their_looks_and_are_recognised() {
        let mid = |id: &str| at(&lut(find(id).unwrap().composite), 128.0);
        let spread = |id: &str| {
            let l = lut(find(id).unwrap().composite);
            at(&l, 192.0) - at(&l, 64.0)
        };
        assert!((mid("default") - 128.0).abs() < 0.5);
        assert!(mid("darker") < 115.0 && mid("lighter") > 141.0, "darker {} lighter {}", mid("darker"), mid("lighter"));
        let (inc, med, strong, linear) = (spread("increaseContrast"), spread("mediumContrast"), spread("strongContrast"), spread("linearContrast"));
        assert!(128.0 < inc && inc < med && med < strong, "contrast rises: {inc} {med} {strong}");
        assert!(linear > 128.0, "Linear Contrast steepens the midtones: {linear}");
        assert!(at(&lut(find("negative").unwrap().composite), 0.0) > 250.0 && at(&lut(find("negative").unwrap().composite), 255.0) < 5.0);
        // Color Negative turns the image over on an orange base: black becomes orange.
        let base: Vec<f32> = find("colorNegative").unwrap().channels.unwrap().iter().map(|c| at(&lut(c), 0.0)).collect();
        assert!(base[0] > base[1] && base[1] > base[2] && base[2] > 0.0, "orange base {base:?}");
        // Cross Process: blue lifted in the shadows and held down in the highlights.
        let blue = lut(find("crossProcess").unwrap().channels.unwrap()[2]);
        assert!(at(&blue, 0.0) > 20.0 && at(&blue, 255.0) < 220.0);
        for p in PRESETS {
            let gray = p.channels.is_none();
            for g in [false, true].into_iter().filter(|g| !g || gray) {
                let mut v = json!({"points": [[0, 0], [100, 140], [255, 255]], "pencil": true});
                apply(&mut v, p, g);
                assert_eq!(matching(&v, g).map(|m| m.id), Some(p.id), "{} (gray {g})", p.id);
            }
        }
    }

    /// A hand edit (or a drawn curve) is Custom; a document in Gray is offered no colour preset.
    #[test]
    fn edits_are_custom_and_gray_documents_get_tone_presets_only() {
        let mut v = json!({});
        apply(&mut v, find("strongContrast").unwrap(), false);
        v["points"][1] = json!([64, 40]);
        assert!(matching(&v, false).is_none());
        apply(&mut v, find("darker").unwrap(), false);
        v["pencil"] = json!(true);
        assert!(matching(&v, false).is_none(), "a drawn curve is Custom");
        assert!(available(true).all(|p| p.channels.is_none()));
        assert_eq!(available(false).count(), PRESETS.len());
        assert_eq!(matching(&json!({}), false).map(|p| p.id), Some("default"), "a new Curves layer is Default");
    }
}
