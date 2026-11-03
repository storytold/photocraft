//! Shared native-conversion helpers; mapping registration follows separately.

#[allow(unused_imports)]
use photocraft_algo::{FilterParams as F, GalleryEffect, GalleryFilter, LensType, SpherizeMode, UndefinedAreas};

#[allow(unused_imports)]
use serde_json::Value;

/// Reads a finite geometry scalar within the native command's bounds, defaulting only missing keys.
fn number(p: &Value, key: &str, default: f64, low: f64, high: f64) -> Result<f64, String> {
    let v = match p.get(key) {
        None => default,
        Some(v) => v.as_f64().ok_or_else(|| format!("Native {key} is not numeric"))?,
    };
    if v.is_finite() && (low..=high).contains(&v) { Ok(v) } else { Err(format!("Native {key} is outside its finite range")) }
}

/// Reads optional native text without coercing malformed present values into defaults.
fn text<'a>(p: &'a Value, key: &str, default: &'a str) -> Result<&'a str, String> {
    match p.get(key) {
        None => Ok(default),
        Some(v) => v.as_str().ok_or_else(|| format!("Native {key} is not text")),
    }
}

/// Reads a native Boolean with explicit missing-value behavior and type rejection.
fn flag(p: &Value, key: &str, default: bool) -> Result<bool, String> {
    match p.get(key) {
        None => Ok(default),
        Some(v) => v.as_bool().ok_or_else(|| format!("Native {key} is not boolean")),
    }
}

/// Rejects external geometry resource locators so coverage replay cannot resolve files or document
/// references.
fn local(p: &Value) -> Result<(), String> {
    if !p.is_object() {
        return Err("Native geometry parameters must be an object".into());
    }
    for key in ["mapPath", "mapDocument", "mapLayer", "file", "path", "textureFile", "profilePath"] {
        if p.get(key).is_some() {
            return Err(format!("Native coverage does not resolve resource {key}"));
        }
    }
    Ok(())
}

/// Maps the native undefined-area text choice to its exact existing sampling policy.
fn undefined(p: &Value, default: &str) -> Result<UndefinedAreas, String> {
    match text(p, "undefinedAreas", default)? {
        "wrap" => Ok(UndefinedAreas::Wrap),
        "repeat" => Ok(UndefinedAreas::Repeat),
        "transparent" => Ok(UndefinedAreas::Transparent),
        _ => Err("Native undefinedAreas choice is invalid".into()),
    }
}

/// Reads the native deterministic seed as a bounded unsigned integer.
fn seed(p: &Value) -> Result<u32, String> {
    match p.get("seed") {
        None => Ok(0),
        Some(v) => v.as_u64().and_then(|v| u32::try_from(v).ok()).ok_or_else(|| "Native seed must be a u32".into()),
    }
}

/// Recognizes owned native geometry commands and decodes their controls, returning errors for unusable
/// resources or domains.
pub(super) fn filter(command: &str, p: &Value) -> Option<Result<F, String>> {
    if !matches!(
        command,
        "filter.distort.displace"
            | "filter.distort.pinch"
            | "filter.distort.twirl"
            | "filter.distort.spherize"
            | "filter.gallery.texturizer"
            | "filter.other.offset"
            | "filter.pixelate.crystallize"
            | "filter.render.lensFlare"
    ) {
        return None;
    }
    Some((|| {
        local(p)?;
        Ok(match command {
            "filter.distort.displace" => {
                let data = p
                    .get("__displacePsd")
                    .and_then(Value::as_str)
                    .filter(|v| v.len() <= super::MAX_METADATA_BYTES)
                    .ok_or("Native Displace requires bounded embedded map data")?;
                // The ordinary native decoder validates the descriptor and two stored planes.
                // The hex cap bounds descriptor, decoded bytes and float-map allocations.
                if !data.len().is_multiple_of(2) {
                    return Err("Native Displace map hex length is invalid".into());
                }
                let map = crate::smart_map::displace_embedded_map(p).ok_or("Native embedded Displace map is invalid")?;
                F::Displace {
                    horizontal: number(p, "horizontal", 10., -999., 999.)? as f32,
                    vertical: number(p, "vertical", 10., -999., 999.)? as f32,
                    stretch: match text(p, "fit", "stretch")? {
                        "stretch" => true,
                        "tile" => false,
                        _ => return Err("Native Displace fit is invalid".into()),
                    },
                    undefined: undefined(p, "repeat")?,
                    map: Some(map),
                }
            }
            "filter.distort.pinch" => F::Pinch { amount: number(p, "amount", 50., -100., 100.)? as f32 },
            "filter.distort.twirl" => F::Twirl { angle: number(p, "angle", 50., -999., 999.)? as f32 },
            "filter.distort.spherize" => F::Spherize {
                amount: number(p, "amount", 100., -100., 100.)? as f32,
                mode: match text(p, "mode", "normal")? {
                    "normal" => SpherizeMode::Normal,
                    "horizontalOnly" => SpherizeMode::HorizontalOnly,
                    "verticalOnly" => SpherizeMode::VerticalOnly,
                    _ => return Err("Native Spherize mode is invalid".into()),
                },
            },
            "filter.gallery.texturizer" => {
                let mut e = GalleryEffect::new(GalleryFilter::Texturizer);
                e.set("scaling", number(p, "scaling", 100., 50., 200.)? as f32);
                e.set("relief", number(p, "relief", 4., 0., 50.)? as f32);
                e.set("invert", u8::from(flag(p, "invert", false)?) as f32);
                for (key, default, count) in [("texture", "canvas", 4), ("light", "top", 8)] {
                    match p.get(key) {
                        Some(Value::Number(_)) => e.set(key, number(p, key, 0., 0., f64::from(count - 1))? as f32),
                        _ => {
                            if !e.set_choice(key, text(p, key, default)?) {
                                return Err(format!("Native Texturizer {key} is invalid"));
                            }
                        }
                    }
                }
                F::FilterGallery { effects: vec![e] }
            }
            "filter.other.offset" => F::Offset {
                horizontal: number(p, "horizontal", 0., f64::from(i32::MIN), f64::from(i32::MAX))?.round() as i32,
                vertical: number(p, "vertical", 0., f64::from(i32::MIN), f64::from(i32::MAX))?.round() as i32,
                undefined: undefined(p, "wrap")?,
            },
            "filter.pixelate.crystallize" => F::Crystallize { cell_size: number(p, "cellSize", 10., 3., 300.)? as f32, seed: seed(p)? },
            "filter.render.lensFlare" => F::LensFlare {
                brightness: number(p, "brightness", 100., 10., 300.)? as f32,
                center_x: number(p, "centerX", 0.5, -0.5, 1.5)? as f32,
                center_y: number(p, "centerY", 0.5, -0.5, 1.5)? as f32,
                lens: match text(p, "lens", "zoom")? {
                    "zoom" => LensType::Zoom,
                    "prime35" => LensType::Prime35,
                    "prime105" => LensType::Prime105,
                    "moviePrime" => LensType::MoviePrime,
                    _ => return Err("Native Lens Flare lens is invalid".into()),
                },
            },
            _ => return Err("Native geometry command is invalid".into()),
        })
    })())
}

/// Resolved ordinary native Lens Correction settings (a separate native API).
pub(super) fn lens_correction(p: &Value) -> Result<photocraft_algo::lens::LensCorrection, String> {
    use photocraft_algo::lens::{self, EdgeMode, LensCorrection};
    local(p)?;
    if flag(p, "autoScale", false)? || p.get("straighten").is_some() {
        return Err("Native coverage Lens Correction requires resolved scale/angle".into());
    }
    let profile = match text(p, "profile", "none")? {
        "none" | "" => None,
        "generic" => {
            let focal = number(p, "focalLength", 0., 0., f64::MAX)?;
            Some(lens::generic_profile(if focal > 0. { focal } else { 50. }))
        }
        _ => return Err("Native coverage Lens Correction requires a resolved local profile".into()),
    };
    let edge = match text(p, "edge", "transparency")? {
        "transparency" => EdgeMode::Transparency,
        "edgeExtension" | "extension" => EdgeMode::Extension,
        "black" => EdgeMode::Color([0., 0., 0., 1.]),
        "white" => EdgeMode::Color([1.; 4]),
        "background" | "color" => {
            let mut color = [0., 0., 0., 1.];
            if let Some(value) = p.get("edgeColor") {
                if let Some(hex) = value.as_str().filter(|v| v.len() == 7 && v.starts_with('#')) {
                    for (i, out) in color.iter_mut().take(3).enumerate() {
                        *out =
                            hex.get(1 + i * 2..3 + i * 2).and_then(|v| u8::from_str_radix(v, 16).ok()).ok_or("Native edge color hex is invalid")? as f32 / 255.;
                    }
                } else {
                    let values = value.as_array().filter(|v| matches!(v.len(), 3 | 4)).ok_or("Native edge color needs RGB or RGBA")?;
                    for (out, input) in color.iter_mut().zip(values) {
                        *out = input.as_f64().filter(|v| v.is_finite() && (0.0..=1.0).contains(v)).ok_or("Native edge color is outside 0..1")? as f32;
                    }
                }
            }
            EdgeMode::Color(color)
        }
        _ => return Err("Native Lens Correction edge mode is invalid".into()),
    };
    Ok(LensCorrection {
        profile,
        edge,
        correct_distortion: flag(p, "correctDistortion", true)?,
        correct_vignette: flag(p, "correctVignette", true)?,
        correct_ca: flag(p, "correctCA", true)?,
        distortion: number(p, "distortion", 0., -100., 100.)?,
        red_cyan: number(p, "redCyan", 0., -100., 100.)?,
        blue_yellow: number(p, "blueYellow", 0., -100., 100.)?,
        vignette_amount: number(p, "vignetteAmount", 0., -100., 100.)?,
        vignette_midpoint: number(p, "vignetteMidpoint", 50., 0., 100.)?,
        vertical: number(p, "vertical", 0., -100., 100.)?,
        horizontal: number(p, "horizontal", 0., -100., 100.)?,
        angle: number(p, "angle", 0., -f64::MAX, f64::MAX)?,
        scale: number(p, "scale", 100., 50., 150.)?,
    })
}
