//! Curves presets (#9221): starting curves for the Curves editor's Preset menu, named as the
//! reference application names them and giving the result it gives.
//!
//! The points are measured from observed output only (#9234 review): each preset was applied to a
//! 0..255 grey ramp (one pixel per level, 8-bit RGB) in Photoshop 2025, the output levels were
//! read back from the saved pixels, and the fewest points whose natural spline
//! ([`photocraft_compose::adjust::curve_lut`]) comes within one level of all 256 outputs were
//! chosen (Color Negative and Cross Process per channel; the tone presets on the composite curve).
//! No preset file or application resource was read. The 17 inputs the tests check are listed in
//! `MEASURED`; the rest of the ramp agrees within one level.

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
        channels: Some([
            &[
                [0.0, 255.0],
                [34.0, 254.0],
                [36.0, 251.0],
                [52.0, 229.0],
                [72.0, 201.0],
                [152.0, 66.0],
                [172.0, 26.0],
                [184.0, 2.0],
                [185.0, 0.0],
                [188.0, 0.0],
                [192.0, 0.0],
                [255.0, 0.0],
            ],
            &[
                [0.0, 255.0],
                [28.0, 254.0],
                [32.0, 244.0],
                [44.0, 212.0],
                [52.0, 191.0],
                [124.0, 18.0],
                [128.0, 9.0],
                [132.0, 0.0],
                [144.0, 0.0],
                [152.0, 0.0],
                [255.0, 0.0],
            ],
            &[
                [0.0, 255.0],
                [20.0, 255.0],
                [24.0, 255.0],
                [25.0, 255.0],
                [26.0, 251.0],
                [28.0, 243.0],
                [52.0, 154.0],
                [60.0, 127.0],
                [68.0, 102.0],
                [72.0, 91.0],
                [76.0, 79.0],
                [80.0, 69.0],
                [88.0, 48.0],
                [100.0, 19.0],
                [107.0, 2.0],
                [108.0, 0.0],
                [120.0, 0.0],
                [124.0, 0.0],
                [255.0, 0.0],
            ],
        ]),
    },
    CurvesPreset {
        id: "crossProcess",
        label: "Cross Process",
        composite: IDENTITY,
        channels: Some([
            &[[0.0, 0.0], [32.0, 16.0], [64.0, 40.0], [96.0, 78.0], [128.0, 125.0], [160.0, 171.0], [192.0, 208.0], [224.0, 235.0], [255.0, 255.0]],
            &[[0.0, 0.0], [32.0, 9.0], [64.0, 48.0], [96.0, 126.0], [128.0, 179.0], [160.0, 202.0], [192.0, 208.0], [224.0, 209.0], [255.0, 208.0]],
            &[[0.0, 0.0], [32.0, 6.0], [64.0, 29.0], [96.0, 77.0], [128.0, 137.0], [160.0, 194.0], [192.0, 234.0], [224.0, 250.0], [255.0, 255.0]],
        ]),
    },
    CurvesPreset { id: "darker", label: "Darker", composite: &[[0.0, 0.0], [128.0, 99.0], [255.0, 255.0]], channels: None },
    CurvesPreset {
        id: "increaseContrast",
        label: "Increase Contrast",
        composite: &[[0.0, 0.0], [44.0, 21.0], [92.0, 68.0], [180.0, 189.0], [220.0, 240.0], [236.0, 253.0], [255.0, 255.0]],
        channels: None,
    },
    CurvesPreset { id: "lighter", label: "Lighter", composite: &[[0.0, 0.0], [104.0, 126.0], [255.0, 255.0]], channels: None },
    CurvesPreset { id: "linearContrast", label: "Linear Contrast", composite: &[[0.0, 0.0], [64.0, 59.0], [192.0, 197.0], [255.0, 255.0]], channels: None },
    CurvesPreset {
        id: "mediumContrast",
        label: "Medium Contrast",
        composite: &[[0.0, 0.0], [76.0, 59.0], [148.0, 146.0], [196.0, 200.0], [255.0, 255.0]],
        channels: None,
    },
    CurvesPreset { id: "negative", label: "Negative", composite: &[[0.0, 255.0], [255.0, 0.0]], channels: None },
    CurvesPreset {
        id: "strongContrast",
        label: "Strong Contrast",
        composite: &[[0.0, 0.0], [72.0, 45.0], [144.0, 142.0], [160.0, 167.0], [192.0, 208.0], [255.0, 255.0]],
        channels: None,
    },
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

    /// The reference's output at inputs 0, 16, ..., 240 and 255 for each preset (the red, green or blue
    /// curve of the colour presets, else the composite), read from a grey ramp run through it.
    const MEASURED: &[(&str, Option<usize>, [u8; 17])] = &[
        ("colorNegative", Some(0), [255, 255, 255, 234, 212, 189, 165, 139, 111, 81, 50, 18, 0, 0, 0, 0, 0]),
        ("colorNegative", Some(1), [255, 255, 244, 202, 160, 120, 81, 45, 9, 0, 0, 0, 0, 0, 0, 0, 0]),
        ("colorNegative", Some(2), [255, 255, 228, 168, 114, 69, 28, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
        ("crossProcess", Some(0), [0, 7, 16, 26, 40, 58, 78, 101, 125, 149, 171, 191, 208, 222, 235, 245, 255]),
        ("crossProcess", Some(1), [0, 3, 9, 23, 48, 86, 126, 157, 179, 194, 202, 206, 208, 209, 209, 209, 208]),
        ("crossProcess", Some(2), [0, 2, 6, 15, 29, 51, 77, 106, 137, 167, 194, 217, 234, 244, 250, 253, 255]),
        ("darker", None, [0, 11, 21, 32, 44, 56, 70, 84, 99, 116, 133, 152, 172, 193, 214, 235, 255]),
        ("increaseContrast", None, [0, 6, 13, 24, 38, 54, 73, 93, 115, 138, 161, 183, 205, 226, 244, 254, 255]),
        ("lighter", None, [0, 21, 41, 61, 81, 99, 117, 134, 150, 166, 180, 194, 207, 219, 232, 244, 255]),
        ("linearContrast", None, [0, 14, 29, 43, 59, 75, 92, 110, 128, 146, 164, 181, 197, 213, 227, 242, 255]),
        ("mediumContrast", None, [0, 10, 22, 34, 47, 63, 81, 101, 121, 141, 160, 179, 196, 212, 227, 241, 255]),
        ("negative", None, [255, 239, 223, 207, 191, 175, 159, 143, 127, 111, 95, 79, 63, 47, 31, 15, 0]),
        ("strongContrast", None, [0, 7, 16, 25, 38, 53, 72, 94, 118, 142, 167, 189, 208, 223, 235, 246, 255]),
    ];

    /// Each preset gives the levels the reference gives, within one level, and is the preset the
    /// editor shows once applied.
    #[test]
    fn presets_give_the_measured_levels_and_are_recognised() {
        for (id, channel, want) in MEASURED {
            let preset = find(id).unwrap();
            let curve = match channel {
                Some(c) => preset.channels.unwrap()[*c],
                None => preset.composite,
            };
            let l = lut(curve);
            for (x, w) in (0..=240).step_by(16).chain([255]).zip(want) {
                let got = at(&l, x as f32);
                assert!((got - f32::from(*w)).abs() <= 1.0, "{id} {channel:?} at {x}: {got} (reference {w})");
            }
        }
        for p in PRESETS {
            let gray = p.channels.is_none();
            for g in [false, true].into_iter().filter(|g| !g || gray) {
                let mut v = json!({"points": [[0, 0], [100, 140], [255, 255]], "pencil": true});
                apply(&mut v, p, g);
                assert_eq!(matching(&v, g).map(|m| m.id), Some(p.id), "{} (gray {g})", p.id);
            }
        }
        assert_eq!(MEASURED.len(), PRESETS.len() - 1 + 4, "every preset but Default, three channels each for the two colour ones");
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
