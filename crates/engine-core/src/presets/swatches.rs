//! Window › Swatches: named colours in groups (folders), as in Photoshop's Swatches panel.
//!
//! The swatch model is craft-color's (`craft::color_ui::palette`), re-exported here: a swatch keeps
//! its colour in the model it was made in (RGB, HSB, CMYK, Lab or grayscale) as `f32` components
//! that hold 16-bit file values exactly, plus colour-book entries we cannot evaluate kept
//! verbatim. Nothing assumes sRGB: RGB and HSB components are in the RGB working space, CMYK and
//! gray in the CMYK and gray working spaces (Edit › Color Settings), Lab is D50. Turning a swatch
//! into the RGB the foreground colour holds goes through `craft-cms` ([`SwatchConv`]).
//!
//! The library persists with the preferences document (`presets.swatches`, see
//! [`super::PresetState::to_json`]). The default set is `default_swatches()`. The commands live in
//! `crate::swatch_cmds`; the file formats are craft-color's.

use crate::color_cmds::ColorExt;
use std::sync::Arc;

use craft_cms::{Builtin, Profile, Transform, TransformOptions};
use photocraft_color::ColorMode;
use serde_json::{Value, json};

use crate::Session;

pub use craft::color_ui::palette::{SwatchColor, SwatchEntry, SwatchGroup, SwatchKind};

/// Rewrites a saved `swatches` value from the shape before craft-color's model (groups holding
/// `items`) to the current one (`entries`). Called once, where the preferences are read.
pub fn upgrade_saved(saved: &mut Value) {
    let Some(groups) = saved.as_array_mut() else { return };
    for group in groups {
        if let Some(map) = group.as_object_mut()
            && !map.contains_key("entries")
            && let Some(items) = map.remove("items")
        {
            map.insert("entries".into(), items);
        }
    }
}

/// The id of a swatch kind (`process`, `global`, `spot`).
pub fn kind_id(kind: SwatchKind) -> &'static str {
    match kind {
        SwatchKind::Process => "process",
        SwatchKind::Global => "global",
        SwatchKind::Spot => "spot",
    }
}

/// The swatch kind an id names (`normal` is a synonym of `process`).
pub fn parse_kind(s: &str) -> Option<SwatchKind> {
    Some(match s {
        "process" | "normal" => SwatchKind::Process,
        "global" => SwatchKind::Global,
        "spot" => SwatchKind::Spot,
        _ => return None,
    })
}

fn finite(v: f32) -> f32 {
    if v.is_finite() { v } else { 0.0 }
}

fn unit(v: f32) -> f32 {
    finite(v).clamp(0.0, 1.0)
}

/// HSB (hue in degrees) to RGB, both in the same RGB space.
pub fn hsb_to_rgb([h, s, v]: [f32; 3]) -> [f32; 3] {
    let (s, v) = (unit(s), unit(v));
    let h = finite(h).rem_euclid(360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [r + m, g + m, b + m].map(unit)
}

/// Model id of a colour (`rgb`, `hsb`, `cmyk`, `lab`, `gray`, `book`).
pub fn color_model(c: &SwatchColor) -> &'static str {
    match c {
        SwatchColor::Rgb(_) => "rgb",
        SwatchColor::Hsb(_) => "hsb",
        SwatchColor::Cmyk(_) => "cmyk",
        SwatchColor::Lab(_) => "lab",
        SwatchColor::Gray(_) => "gray",
        SwatchColor::Book { .. } => "book",
    }
}

/// Components in the units the Color Picker shows: RGB 0..255, HSB degrees and percent,
/// CMYK and gray ink percent, Lab as is.
pub fn display_values(c: &SwatchColor) -> Vec<f64> {
    let r = |v: f32, k: f64| (f64::from(finite(v)) * k * 1000.0).round() / 1000.0;
    match *c {
        SwatchColor::Rgb(c) => c.iter().map(|v| r(*v, 255.0)).collect(),
        SwatchColor::Hsb([h, s, b]) => vec![r(h, 1.0), r(s, 100.0), r(b, 100.0)],
        SwatchColor::Cmyk(c) => c.iter().map(|v| r(*v, 100.0)).collect(),
        SwatchColor::Lab(c) => c.iter().map(|v| r(*v, 1.0)).collect(),
        SwatchColor::Gray(k) => vec![r(k, 100.0)],
        SwatchColor::Book { space, values } => std::iter::once(f64::from(space)).chain(values.iter().map(|v| f64::from(*v))).collect(),
        // The enum may grow; an unknown model shows no values.
        #[allow(unreachable_patterns)]
        _ => Vec::new(),
    }
}

/// Parse a command's colour: `"#rrggbb"` (RGB working space), or
/// `{"model":"rgb|hsb|cmyk|lab|gray","values":[…]}` in [`display_values`] units.
pub fn color_from_param(v: &Value) -> Result<SwatchColor, String> {
    if let Some(s) = v.as_str() {
        let h = s.trim().trim_start_matches('#');
        if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("bad colour `{s}` (\"#rrggbb\" or {{\"model\",\"values\"}})"));
        }
        let b = |i: usize| h.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok()).map_or(0.0, |x| f32::from(x) / 255.0);
        return Ok(SwatchColor::Rgb([b(0), b(2), b(4)]));
    }
    let model = v.get("model").and_then(Value::as_str).ok_or("a colour needs `model` (rgb|hsb|cmyk|lab|gray) and `values`")?;
    let vals: Vec<f32> = v
        .get("values")
        .and_then(Value::as_array)
        .ok_or("a colour needs `values`")?
        .iter()
        // Beyond f32 range a value turns into inf as f32 (an HSB hue then wraps to NaN, which
        // the prefs file can't store), so reject it here.
        .map(|x| {
            x.as_f64()
                .filter(|f| f.is_finite() && f.abs() <= f64::from(f32::MAX))
                .map(|f| f as f32)
                .ok_or("colour values must be finite numbers within f32 range")
        })
        .collect::<Result<_, _>>()?;
    let need = |n: usize| if vals.len() == n { Ok(()) } else { Err(format!("a {model} colour has {n} values, got {}", vals.len())) };
    let pct = |v: f32| (v / 100.0).clamp(0.0, 1.0);
    Ok(match model {
        "rgb" => {
            need(3)?;
            SwatchColor::Rgb([0, 1, 2].map(|i| (vals[i] / 255.0).clamp(0.0, 1.0)))
        }
        "hsb" => {
            need(3)?;
            SwatchColor::Hsb([vals[0].rem_euclid(360.0), pct(vals[1]), pct(vals[2])])
        }
        "cmyk" => {
            need(4)?;
            SwatchColor::Cmyk([0, 1, 2, 3].map(|i| pct(vals[i])))
        }
        "lab" => {
            need(3)?;
            SwatchColor::Lab([vals[0].clamp(0.0, 100.0), vals[1].clamp(-128.0, 127.0), vals[2].clamp(-128.0, 127.0)])
        }
        "gray" => {
            need(1)?;
            SwatchColor::Gray(pct(vals[0]))
        }
        m => return Err(format!("unknown colour model `{m}` (rgb|hsb|cmyk|lab|gray)")),
    })
}

/// A colour as the commands report it: `{model, values}` in Color Picker units.
pub fn color_json(c: &SwatchColor) -> Value {
    json!({"model": color_model(c), "values": display_values(c)})
}

// ------------------------------------------------------------------ colour management

/// Converts swatch colours to the RGB working space through `craft-cms` (relative
/// colorimetric + black point compensation, Photoshop's default). Build one per library or
/// colour-settings change; evaluating is cheap.
pub struct SwatchConv {
    cmyk: Option<Arc<Transform>>,
    lab: Option<Arc<Transform>>,
    gray: Option<Arc<Transform>>,
}

fn working(s: Option<&Session>, mode: ColorMode) -> Arc<Profile> {
    let spec = s.map(|s| {
        let c = &s.color().settings;
        match mode {
            ColorMode::Cmyk => c.working_cmyk.clone(),
            ColorMode::Grayscale => c.working_gray.clone(),
            _ => c.working_rgb.clone(),
        }
    });
    spec.and_then(|sp| crate::color_cmds::resolve_profile(&sp, None, Some(mode)).ok())
        .filter(|p| p.color_space == crate::color_cmds::mode_space(mode))
        .unwrap_or_else(|| crate::color_cmds::working_profile(mode))
}

impl SwatchConv {
    /// Conversions into the session's RGB working space from its CMYK and gray working spaces.
    pub fn new(s: &Session) -> Self {
        Self::with(Some(s))
    }

    /// The built-in working spaces (sRGB, coated CMYK, sGray).
    pub fn builtin() -> Self {
        Self::with(None)
    }

    fn with(s: Option<&Session>) -> Self {
        let rgb = working(s, ColorMode::Rgb);
        let opts = TransformOptions { intent: craft_cms::Intent::RelativeColorimetric, bpc: true, ..Default::default() };
        let link = |src: &Profile| craft_cms::cached(src, &rgb, opts).ok();
        SwatchConv { cmyk: link(&working(s, ColorMode::Cmyk)), lab: link(Builtin::LabD50.profile()), gray: link(&working(s, ColorMode::Grayscale)) }
    }

    /// The colour in the RGB working space (what the foreground colour holds), 0..1.
    /// Colour-book entries, which have no colour of their own here, are mid gray.
    pub fn rgb(&self, c: &SwatchColor) -> [f32; 3] {
        let eval = |t: &Option<Arc<Transform>>, input: &[f32]| -> Option<[f32; 3]> {
            let t = t.as_ref()?;
            let mut o = [0.0f32; 16];
            t.eval(input, &mut o).ok()?;
            Some([unit(o[0]), unit(o[1]), unit(o[2])])
        };
        match *c {
            SwatchColor::Rgb(v) => v.map(unit),
            SwatchColor::Hsb(v) => hsb_to_rgb(v),
            SwatchColor::Cmyk(v) => {
                let v = v.map(unit);
                eval(&self.cmyk, &v).unwrap_or_else(|| photocraft_color::convert::cmyk_to_rgb(v))
            }
            SwatchColor::Lab([l, a, b]) => {
                // ICC v4 Lab encoding (the Lab D50 profile's device values).
                let enc = [unit(finite(l) / 100.0), unit((finite(a) + 128.0) / 255.0), unit((finite(b) + 128.0) / 255.0)];
                eval(&self.lab, &enc).unwrap_or_else(|| photocraft_color::convert::lab_to_srgb([finite(l), finite(a), finite(b)]))
            }
            SwatchColor::Gray(k) => {
                let level = 1.0 - unit(k);
                eval(&self.gray, &[level]).unwrap_or([level; 3])
            }
            SwatchColor::Book { .. } => [0.5; 3],
        }
    }
}

/// `#rrggbb` of RGB components.
pub fn hex(c: [f32; 3]) -> String {
    let q = |v: f32| (unit(v) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", q(c[0]), q(c[1]), q(c[2]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::PresetState;
    use crate::presets::PresetsExt;

    #[test]
    fn default_library_has_the_forty_six_colours() {
        let b = PresetState::default().swatches;
        assert_eq!(b.iter().map(|g| g.entries.len()).sum::<usize>(), 46);
        let names: Vec<&str> = b.iter().flat_map(|g| g.entries.iter().map(|s| s.name.as_str())).collect();
        let mut uniq = names.clone();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(uniq.len(), 46, "names are unique: {names:?}");
        assert_eq!(b[3].name, "Pure");
        assert_eq!(b[3].entries[0].color, SwatchColor::Rgb([230.0 / 255.0, 40.0 / 255.0, 40.0 / 255.0]));
    }

    #[test]
    fn the_rgb_group_holds_exact_primaries_and_secondaries() {
        // #4302: picking green should give #00FF00, as in Photoshop's RGB group.
        let b = PresetState::default().swatches;
        let g = &b[1];
        assert_eq!(g.name, "RGB");
        let conv = SwatchConv::builtin();
        let hexes: Vec<String> = g.entries.iter().map(|s| hex(conv.rgb(&s.color))).collect();
        assert_eq!(hexes, ["#ff0000", "#ffff00", "#00ff00", "#00ffff", "#0000ff", "#ff00ff"]);
    }

    #[test]
    fn conversions_go_through_the_cms() {
        let c = SwatchConv::builtin();
        let near = |a: [f32; 3], b: [f32; 3], tol: f32| a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol);
        assert_eq!(c.rgb(&SwatchColor::Rgb([0.1, 0.2, 0.3])), [0.1, 0.2, 0.3]);
        assert!(near(c.rgb(&SwatchColor::Hsb([120.0, 1.0, 1.0])), [0.0, 1.0, 0.0], 1e-6));
        // Lab white and black, gray (no ink = white), paper-white CMYK.
        assert!(near(c.rgb(&SwatchColor::Lab([100.0, 0.0, 0.0])), [1.0; 3], 0.01));
        assert!(near(c.rgb(&SwatchColor::Lab([0.0, 0.0, 0.0])), [0.0; 3], 0.01));
        assert!(near(c.rgb(&SwatchColor::Gray(0.0)), [1.0; 3], 0.01));
        assert!(near(c.rgb(&SwatchColor::Gray(1.0)), [0.0; 3], 0.01));
        assert!(near(c.rgb(&SwatchColor::Cmyk([0.0; 4])), [1.0; 3], 0.02));
        // Full cyan reads as a blue-green, not as naive (0, 1, 1).
        let cy = c.rgb(&SwatchColor::Cmyk([1.0, 0.0, 0.0, 0.0]));
        assert!(cy[0] < 0.3 && cy[2] > cy[0] && cy[1] > cy[0], "{cy:?}");
        // Lab red-ish.
        let red = c.rgb(&SwatchColor::Lab([54.0, 80.0, 67.0]));
        assert!(red[0] > 0.9 && red[1] < 0.2, "{red:?}");
        // Out-of-range and non-finite stored values never escape 0..1.
        for col in [SwatchColor::Rgb([f32::NAN, 9.0, -3.0]), SwatchColor::Hsb([f32::INFINITY, 2.0, -1.0]), SwatchColor::Lab([500.0, f32::NAN, -999.0])] {
            assert!(c.rgb(&col).iter().all(|v| (0.0..=1.0).contains(v)), "{col:?}");
        }
        assert_eq!(c.rgb(&SwatchColor::Book { space: 3, values: [0; 4] }), [0.5; 3]);
    }

    #[test]
    fn a_wide_gamut_working_space_changes_cmyk_but_not_rgb() {
        let mut s = crate::session();
        s.color_mut().settings.working_rgb = "display-p3".into();
        let p3 = SwatchConv::new(&s);
        let srgb = SwatchConv::builtin();
        let cmyk = SwatchColor::Cmyk([1.0, 0.0, 0.0, 0.0]);
        assert_ne!(p3.rgb(&cmyk), srgb.rgb(&cmyk));
        assert_eq!(p3.rgb(&SwatchColor::Rgb([0.2, 0.4, 0.6])), [0.2, 0.4, 0.6]);
    }

    #[test]
    fn colour_params() {
        assert_eq!(color_from_param(&json!("#ff0000")).unwrap(), SwatchColor::Rgb([1.0, 0.0, 0.0]));
        assert_eq!(color_from_param(&json!({"model": "cmyk", "values": [100, 0, 50, 0]})).unwrap(), SwatchColor::Cmyk([1.0, 0.0, 0.5, 0.0]));
        assert_eq!(color_from_param(&json!({"model": "gray", "values": [25]})).unwrap(), SwatchColor::Gray(0.25));
        let lab = color_from_param(&json!({"model": "lab", "values": [50, -20, 30]})).unwrap();
        assert_eq!(display_values(&lab), vec![50.0, -20.0, 30.0]);
        for bad in [
            json!("#ff00"),
            json!("red"),
            json!(5),
            json!({}),
            json!({"model": "rgb"}),
            json!({"model": "rgb", "values": [1, 2]}),
            json!({"model": "rgb", "values": ["a", 2, 3]}),
            json!({"model": "xyz", "values": [1, 2, 3]}),
            json!({"model": "gray", "values": []}),
            json!({"model": "hsb", "values": [1e39, 100, 100]}),
            json!({"model": "rgb", "values": [0, -1e39, 0]}),
        ] {
            assert!(color_from_param(&bad).is_err(), "{bad}");
        }
    }

    /// Preferences saved before the craft-color model hold groups of `items` and no `kind` on
    /// process swatches; they load into `SwatchGroup`s and save back as `entries`.
    #[test]
    fn serde_round_trip() {
        let saved = json!({"swatches": [{"name": "Old", "items": [
            {"name": "a", "color": {"model": "gray", "values": 0.5}},
            {"name": "b", "color": {"model": "lab", "values": [50.0, -1.5, 2.25]}, "kind": "spot"},
            {"name": "c", "color": {"model": "book", "values": {"space": 3, "values": [1, 2, 3, 4]}}},
        ]}]});
        let mut s = crate::session();
        s.load_presets_json(saved);
        let groups = &s.presets().swatches;
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "Old");
        let e = &groups[0].entries;
        assert_eq!((e[0].color, e[0].kind), (SwatchColor::Gray(0.5), SwatchKind::Process));
        assert_eq!((e[1].color, e[1].kind), (SwatchColor::Lab([50.0, -1.5, 2.25]), SwatchKind::Spot));
        assert_eq!(e[2].color, SwatchColor::Book { space: 3, values: [1, 2, 3, 4] });
        let mut t = crate::session();
        t.load_presets_json(s.presets().to_json(&s));
        assert_eq!(t.presets().swatches, s.presets().swatches);
    }
}
