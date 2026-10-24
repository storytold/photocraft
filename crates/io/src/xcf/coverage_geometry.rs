//! Decoders for ordinary native geometry filters on imported coverage surfaces.
//! These consume PhotoCraft parameters, never source-operation names or resources.
use photocraft_algo::{FilterParams as F, GalleryEffect, GalleryFilter, LensType, SpherizeMode, UndefinedAreas};
use serde_json::Value;

fn number(p: &Value, key: &str, default: f64, low: f64, high: f64) -> Result<f64, String> {
    let v = match p.get(key) {
        None => default,
        Some(v) => v.as_f64().ok_or_else(|| format!("Native {key} is not numeric"))?,
    };
    if v.is_finite() && (low..=high).contains(&v) { Ok(v) } else { Err(format!("Native {key} is outside its finite range")) }
}
fn text<'a>(p: &'a Value, key: &str, default: &'a str) -> Result<&'a str, String> {
    match p.get(key) {
        None => Ok(default),
        Some(v) => v.as_str().ok_or_else(|| format!("Native {key} is not text")),
    }
}
fn flag(p: &Value, key: &str, default: bool) -> Result<bool, String> {
    match p.get(key) {
        None => Ok(default),
        Some(v) => v.as_bool().ok_or_else(|| format!("Native {key} is not boolean")),
    }
}
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
fn undefined(p: &Value, default: &str) -> Result<UndefinedAreas, String> {
    match text(p, "undefinedAreas", default)? {
        "wrap" => Ok(UndefinedAreas::Wrap),
        "repeat" => Ok(UndefinedAreas::Repeat),
        "transparent" => Ok(UndefinedAreas::Transparent),
        _ => Err("Native undefinedAreas choice is invalid".into()),
    }
}
fn seed(p: &Value) -> Result<u32, String> {
    match p.get("seed") {
        None => Ok(0),
        Some(v) => v.as_u64().and_then(|v| u32::try_from(v).ok()).ok_or_else(|| "Native seed must be a u32".into()),
    }
}

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
                        *out = input.as_f64().filter(|v| v.is_finite() && (0.0..=1.0).contains(v)).ok_or("Native edge color is outside0..1")? as f32;
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

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_geom::Rect;
    use photocraft_raster::{Interrupt, Surface};
    use serde_json::json;

    fn source(sample: SampleType) -> Surface {
        let mut s = Surface::new(PixelFormat::new(ColorMode::Grayscale, sample, false));
        s.write_region(Rect::new(0, 0, 12, 9), &(0..108).map(|v| ((v * 17) % 107) as f32 / 106.).collect::<Vec<_>>());
        s
    }
    #[test]
    fn decoders_match_independent_native_parameters_and_coverage_at_all_depths() {
        let mut texture = GalleryEffect::new(GalleryFilter::Texturizer);
        texture.set("relief", 7.);
        texture.set("scaling", 125.);
        assert!(texture.set_choice("texture", "brick"));
        assert!(texture.set_choice("light", "left"));
        texture.set("invert", 1.);
        let map = photocraft_algo::Image { rect: Rect::new(0, 0, 2, 2), ch: 3, data: vec![0., 1., 0.5, 1., 0., 0.5, 0.25, 0.75, 0.5, 0.75, 0.25, 0.5] };
        let displace = crate::smart_map::displace_embed_map(&json!({"horizontal":2.,"vertical":-3.,"fit":"tile","undefinedAreas":"wrap"}), &map).unwrap();
        let embedded = crate::smart_map::displace_embedded_map(&displace).unwrap();
        let cases = [
            ("filter.distort.pinch", json!({"amount":-25.}), F::Pinch { amount: -25. }),
            ("filter.distort.twirl", json!({"angle":120.}), F::Twirl { angle: 120. }),
            ("filter.distort.spherize", json!({"amount":-60.,"mode":"verticalOnly"}), F::Spherize { amount: -60., mode: SpherizeMode::VerticalOnly }),
            (
                "filter.other.offset",
                json!({"horizontal":2,"vertical":-3,"undefinedAreas":"wrap"}),
                F::Offset { horizontal: 2, vertical: -3, undefined: UndefinedAreas::Wrap },
            ),
            ("filter.pixelate.crystallize", json!({"cellSize":4.,"seed":u32::MAX}), F::Crystallize { cell_size: 4., seed: u32::MAX }),
            (
                "filter.render.lensFlare",
                json!({"brightness":80.,"centerX":0.3,"centerY":0.7,"lens":"prime35"}),
                F::LensFlare { brightness: 80., center_x: 0.3, center_y: 0.7, lens: LensType::Prime35 },
            ),
            (
                "filter.gallery.texturizer",
                json!({"texture":"brick","light":"left","scaling":125.,"relief":7.,"invert":true}),
                F::FilterGallery { effects: vec![texture] },
            ),
            (
                "filter.distort.displace",
                displace,
                F::Displace { horizontal: 2., vertical: -3., stretch: false, undefined: UndefinedAreas::Wrap, map: Some(embedded) },
            ),
        ];
        let bounds = Rect::new(0, 0, 12, 9);
        for (id, params, expected) in cases {
            let decoded = filter(id, &params).unwrap().unwrap();
            assert_eq!(decoded, expected, "{id}");
            for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let src = source(sample);
                let actual = photocraft_algo::apply_in_with(&src, &decoded, bounds, bounds, None, bounds, &Interrupt::NONE).unwrap();
                let reference = photocraft_algo::apply_in_with(&src, &expected, bounds, bounds, None, bounds, &Interrupt::NONE).unwrap();
                assert_eq!(actual.read_region(bounds), reference.read_region(bounds), "{id}:{sample:?}");
                assert!(actual.read_region(bounds).iter().all(|v| v.is_finite()));
            }
        }
    }
    #[test]
    fn lens_decodes_native_defaults_and_resolved_parameters_without_source_resources() {
        use photocraft_algo::lens::{EdgeMode, LensCorrection};
        assert_eq!(lens_correction(&json!({})).unwrap(), LensCorrection::default());
        let params = json!({"distortion":23.,"vignetteAmount":-15.,"scale":110.,"angle":4.,"edge":"color","edgeColor":[0.2,0.4,0.6,1.]});
        let expected =
            LensCorrection { distortion: 23., vignette_amount: -15., scale: 110., angle: 4., edge: EdgeMode::Color([0.2, 0.4, 0.6, 1.]), ..Default::default() };
        let actual = lens_correction(&params).unwrap();
        assert_eq!(actual, expected);
        let bounds = Rect::new(0, 0, 12, 9);
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let src = source(sample);
            assert_eq!(
                photocraft_algo::lens::correct(&src, bounds, &actual).read_region(bounds),
                photocraft_algo::lens::correct(&src, bounds, &expected).read_region(bounds)
            );
        }
    }
    #[test]
    fn rejects_external_resources_malformed_types_and_nonfinite_or_outside_native_domains() {
        assert!(filter("unrecognized", &json!({})).is_none());
        for (id, params) in [
            ("filter.distort.displace", json!({"mapPath":"C:/private/source.psd"})),
            ("filter.distort.displace", json!({"__displacePsd":"00"})),
            ("filter.distort.twirl", json!({"angle":1e300})),
            ("filter.distort.pinch", json!({"amount":"10"})),
            ("filter.distort.spherize", json!({"mode":"unknown"})),
            ("filter.other.offset", json!({"horizontal":2147483648u64})),
            ("filter.pixelate.crystallize", json!({"seed":-1})),
            ("filter.gallery.texturizer", json!({"invert":1})),
            ("filter.render.lensFlare", json!({"centerX":2.})),
        ] {
            assert!(filter(id, &params).unwrap().is_err(), "{id}:{params}");
        }
        for p in [
            json!({"profile":"auto"}),
            json!({"autoScale":true}),
            json!({"edgeColor":[1.,2.,3.],"edge":"color"}),
            json!({"scale":1e300}),
            json!({"correctCA":0}),
        ] {
            assert!(lens_correction(&p).is_err(), "{p}");
        }
    }
}
