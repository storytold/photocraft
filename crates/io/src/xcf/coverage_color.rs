//! Shared native-conversion helpers; mapping registration follows separately.

#[allow(unused_imports)]
use photocraft_doc::{Adjustment, adjust::HueRange};

#[allow(unused_imports)]
use serde_json::Value;

type Result<T> = std::result::Result<T, String>;

const RANGES: [&str; 6] = ["reds", "yellows", "greens", "cyans", "blues", "magentas"];

const BW_DEFAULTS: [f32; 6] = [40., 60., 40., 60., 20., 80.];

/// Treat absent and null native parameters alike, matching the ordinary native adjustment defaults.
fn field<'a>(p: &'a Value, key: &str) -> Option<&'a Value> {
    p.get(key).filter(|v| !v.is_null())
}

/// Decode a finite native parameter and apply the native field bounds; wrong types and nonfinite values fail.
fn number(p: &Value, key: &str, default: f32, lo: f32, hi: f32) -> Result<f32> {
    match field(p, key) {
        None => Ok(default.clamp(lo, hi)),
        Some(v) => v.as_f64().filter(|v| v.is_finite()).map(|v| (v as f32).clamp(lo, hi)).ok_or_else(|| format!("Native {key} must be a finite number")),
    }
}

/// Read a native boolean with its default while rejecting supplied values of other types.
fn boolean(p: &Value, key: &str, default: bool) -> Result<bool> {
    match field(p, key) {
        None => Ok(default),
        Some(v) => v.as_bool().ok_or_else(|| format!("Native {key} must be boolean")),
    }
}

/// Require exactly the declared number of finite native coefficients and apply the adapter magnitude bound.
fn array<const N: usize>(v: &Value, key: &str) -> Result<[f32; N]> {
    let input = v.as_array().filter(|a| a.len() == N).ok_or_else(|| format!("Native {key} needs exactly {N} numbers"))?;
    let mut output = [0.; N];
    for (target, value) in output.iter_mut().zip(input) {
        *target = value.as_f64().filter(|n| n.is_finite()).map(|v| (v as f32).clamp(-1e6, 1e6)).ok_or_else(|| format!("Native {key} needs finite numbers"))?;
    }
    Ok(output)
}

/// Accept native hexadecimal or numeric RGB paint parameters, validating their shape and applying native color-component bounds.
fn color(p: &Value, key: &str, default: [f32; 3]) -> Result<[f32; 3]> {
    let Some(v) = field(p, key) else { return Ok(default) };
    if let Some(text) = v.as_str() {
        let h = text.trim_start_matches('#');
        if !matches!(h.len(), 6 | 8) {
            return Err(format!("Native {key} needs hex RGB or RGB(A)"));
        }
        let mut rgb = [0.; 3];
        for (index, target) in rgb.iter_mut().enumerate() {
            *target = h
                .get(index * 2..index * 2 + 2)
                .and_then(|s| u8::from_str_radix(s, 16).ok())
                .map(|v| f32::from(v) / 255.)
                .ok_or_else(|| format!("Native {key} has invalid hex RGB"))?;
        }
        return Ok(rgb);
    }
    let a = v.as_array().filter(|a| a.len() >= 3).ok_or_else(|| format!("Native {key} needs RGB components"))?;
    let mut rgb = [0.; 3];
    for (target, value) in rgb.iter_mut().zip(a) {
        *target =
            value.as_f64().filter(|v| v.is_finite()).map(|v| (v as f32).clamp(0., 1.)).ok_or_else(|| format!("Native {key} needs finite RGB components"))?;
    }
    Ok(rgb)
}

/// None means another adapter owns this ordinary native command.
pub(super) fn adjustment(command: &str, p: &Value) -> Option<Result<Adjustment>> {
    if !matches!(
        command,
        "image.adjustments.blackWhite"
            | "image.adjustments.colorBalance"
            | "image.adjustments.hueSaturation"
            | "image.adjustments.photoFilter"
            | "image.adjustments.vibrance"
    ) {
        return None;
    }
    Some((|| {
        if !p.is_object() {
            return Err("Native color adjustment parameters must be an object".into());
        }
        Ok(match command {
            "image.adjustments.vibrance" => {
                Adjustment::Vibrance { vibrance: number(p, "vibrance", 0., -100., 100.)?, saturation: number(p, "saturation", 0., -100., 100.)? }
            }
            "image.adjustments.blackWhite" => {
                let mut weights = BW_DEFAULTS;
                for (target, key) in weights.iter_mut().zip(RANGES) {
                    *target = number(p, key, *target, -200., 300.)?;
                }
                let tint_color = color(p, "tintColor", [0.882, 0.827, 0.702])?;
                Adjustment::BlackWhite { weights, tint: boolean(p, "tint", false)?.then_some(tint_color) }
            }
            "image.adjustments.colorBalance" => {
                let mut tones = [[0.; 3]; 3];
                for (target, key) in tones.iter_mut().zip(["shadows", "midtones", "highlights"]) {
                    if let Some(v) = field(p, key) {
                        *target = array(v, key)?.map(|v: f32| v.clamp(-100., 100.));
                    }
                }
                Adjustment::ColorBalance {
                    shadows: tones[0],
                    midtones: tones[1],
                    highlights: tones[2],
                    preserve_luminosity: boolean(p, "preserveLuminosity", true)?,
                }
            }
            "image.adjustments.photoFilter" => {
                // XCF native mappings embed explicit RGB; do not silently ignore a preset
                // or look it up using an external file/resource policy.
                if field(p, "filter").is_some() {
                    return Err("Native coverage Photo Filter requires its embedded explicit color rather than a preset id".into());
                }
                Adjustment::PhotoFilter {
                    color: color(p, "color", [236. / 255., 138. / 255., 0.])?,
                    density: number(p, "density", 25., 0., 100.)? / 100.,
                    preserve_luminosity: boolean(p, "preserveLuminosity", true)?,
                }
            }
            _ => {
                let colorize = boolean(p, "colorize", false)?;
                let mut ranges = HueRange::defaults();
                for (range, key) in ranges.iter_mut().zip(RANGES) {
                    let Some(v) = field(p, key) else { continue };
                    if !v.is_object() {
                        return Err(format!("Native {key} hue range must be an object"));
                    }
                    range.hue = number(v, "hue", range.hue, -180., 180.)?;
                    range.saturation = number(v, "saturation", range.saturation, -100., 100.)?;
                    range.lightness = number(v, "lightness", range.lightness, -100., 100.)?;
                    if let Some(bounds) = field(v, "range") {
                        range.bounds = HueRange::canonical_bounds(array(bounds, "range")?);
                    }
                }
                Adjustment::HueSaturation {
                    hue: number(p, "hue", 0., if colorize { 0. } else { -180. }, if colorize { 360. } else { 180. })?,
                    saturation: number(p, "saturation", if colorize { 25. } else { 0. }, if colorize { 0. } else { -100. }, 100.)?,
                    lightness: number(p, "lightness", 0., -100., 100.)?,
                    colorize,
                    ranges,
                }
            }
        })
    })())
}
