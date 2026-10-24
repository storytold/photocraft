//! Import-time decoding of ordinary native spatial filter parameters.
//! Defaults follow engine filters/filters_ext; no source operation is evaluated.
use photocraft_algo::{self as algo, FilterParams};
use serde_json::Value;

type Result<T> = std::result::Result<T, String>;

fn fields(p: &Value, allowed: &[&str]) -> Result<()> {
    let object = p.as_object().ok_or("native spatial params must be an object")?;
    if let Some(key) = object.keys().find(|k| !allowed.contains(&k.as_str())) {
        return Err(format!("unmapped native spatial parameter {key}"));
    }
    Ok(())
}
fn number(p: &Value, key: &str, default: f32, min: f64, max: f64) -> Result<f32> {
    let Some(v) = p.get(key) else { return Ok(default) };
    let n = v.as_f64().filter(|n| n.is_finite() && *n >= min && *n <= max).ok_or_else(|| format!("native {key} must be finite in {min}..{max}"))?;
    Ok(n as f32)
}
fn boolean(p: &Value, key: &str, default: bool) -> Result<bool> {
    p.get(key).map_or(Ok(default), |v| v.as_bool().ok_or_else(|| format!("native {key} must be boolean")))
}
fn choice<'a>(p: &'a Value, key: &str, default: &'a str, choices: &[&str]) -> Result<&'a str> {
    let s = match p.get(key) {
        None => default,
        Some(v) => v.as_str().ok_or_else(|| format!("native {key} must be a string"))?,
    };
    if choices.contains(&s) { Ok(s) } else { Err(format!("unknown native {key} choice {s}")) }
}
fn unsigned(p: &Value, key: &str, default: u32, min: u32, max: u32) -> Result<u32> {
    match p.get(key) {
        None => Ok(default),
        Some(v) => v
            .as_u64()
            .filter(|v| *v >= u64::from(min) && *v <= u64::from(max))
            .map(|v| v as u32)
            .ok_or_else(|| format!("native {key} must be an integer in {min}..{max}")),
    }
}
fn color(p: &Value, key: &str, default: [f32; 4]) -> Result<[f32; 4]> {
    let Some(v) = p.get(key) else { return Ok(default) };
    if let Some(s) = v.as_str() {
        if s.len() != 7 || !s.starts_with('#') {
            return Err(format!("native {key} must be #rrggbb"));
        }
        let channel = |start| {
            s.get(start..start + 2)
                .and_then(|s| u8::from_str_radix(s, 16).ok())
                .map(|v| f32::from(v) / 255.)
                .ok_or_else(|| format!("invalid native {key} color"))
        };
        return Ok([channel(1)?, channel(3)?, channel(5)?, 1.]);
    }
    let values = v.as_array().filter(|v| matches!(v.len(), 3 | 4)).ok_or_else(|| format!("native {key} needs3 or4 components"))?;
    let mut result = [0., 0., 0., 1.];
    for (index, (out, v)) in result.iter_mut().zip(values).enumerate() {
        *out = v
            .as_f64()
            .filter(|v| v.is_finite() && v.abs() <= f64::from(f32::MAX) && (index != 3 || (0.0..=1.0).contains(v)))
            .ok_or_else(|| format!("invalid native {key} component"))? as f32;
    }
    Ok(result)
}
fn iris(p: &Value, nested: bool) -> Result<algo::IrisPin> {
    if nested {
        fields(p, &["x", "y", "radiusX", "radiusY", "angle", "roundness", "feather", "blur"])?;
    }
    Ok(algo::IrisPin {
        x: number(p, if nested { "x" } else { "centerX" }, 0.5, -f64::from(f32::MAX), f64::from(f32::MAX))?,
        y: number(p, if nested { "y" } else { "centerY" }, 0.5, -f64::from(f32::MAX), f64::from(f32::MAX))?,
        radius_x: number(p, "radiusX", 0.35, 0.001, f64::from(f32::MAX))?,
        radius_y: number(p, "radiusY", 0.25, 0.001, f64::from(f32::MAX))?,
        angle: number(p, "angle", 0., -f64::from(f32::MAX), f64::from(f32::MAX))?,
        roundness: number(p, "roundness", 0., 0., 100.)?,
        feather: number(p, "feather", 0.5, 0., 0.99)?,
        blur: number(p, "blur", 15., 0., 500.)?,
    })
}

/// None means this module does not own the command. Invalid owned parameters
/// return an error so callers retain the stage rather than replaying defaults.
pub(super) fn filter(command: &str, p: &Value) -> Option<Result<FilterParams>> {
    let keys: &[&str] = match command {
        "filter.blur.lensBlur" => &[
            "radius",
            "shape",
            "bladeCurvature",
            "rotation",
            "depthMap",
            "focalDistance",
            "invert",
            "brightness",
            "threshold",
            "noise",
            "distribution",
            "monochromatic",
            "seed",
        ],
        "filter.blur.radialBlur" => &["amount", "method", "quality", "centerX", "centerY"],
        "filter.blur.smartBlur" => &["radius", "threshold", "quality", "mode"],
        "filter.blurGallery.irisBlur" => &["pins", "centerX", "centerY", "radiusX", "radiusY", "angle", "roundness", "feather", "blur"],
        "filter.blurGallery.tiltShift" => &["blur", "centerX", "centerY", "angle", "focus", "transition"],
        "filter.noise.reduceNoise" => &["strength", "preserveDetails", "reduceColorNoise", "sharpenDetails", "removeJpegArtifact"],
        "filter.other.maximum" | "filter.other.minimum" => &["radius", "preserve"],
        "filter.stylize.findEdges" => &[],
        "filter.stylize.emboss" => &["angle", "height", "amount"],
        "filter.stylize.oilPaint" => &["stylization", "cleanliness", "scale", "bristleDetail", "lighting", "angle", "shine"],
        "filter.stylize.tiles" => &["count", "maxOffset", "fill", "foreground", "background", "seed"],
        "filter.video.deInterlace" => &["eliminate", "createBy"],
        _ => return None,
    };
    Some((|| {
        fields(p, keys)?;
        Ok(match command {
            "filter.blur.lensBlur" => {
                let depth = match choice(p, "depthMap", "none", &["none", "transparency", "layerMask", "channel"])? {
                    "transparency" => algo::DepthSource::Transparency,
                    "layerMask" | "channel" => return Err("native Lens Blur auxiliary depth requires an explicitly installed native resource".into()),
                    _ => algo::DepthSource::None,
                };
                FilterParams::LensBlur {
                    radius: number(p, "radius", 15., 0., 100.)?,
                    blades: match choice(p, "shape", "hexagon", &["triangle", "square", "pentagon", "hexagon", "heptagon", "octagon"])? {
                        "triangle" => 3,
                        "square" => 4,
                        "pentagon" => 5,
                        "heptagon" => 7,
                        "octagon" => 8,
                        _ => 6,
                    },
                    curvature: number(p, "bladeCurvature", 0., 0., 100.)?,
                    rotation: number(p, "rotation", 0., -f64::from(f32::MAX), f64::from(f32::MAX))?,
                    depth,
                    focal_distance: number(p, "focalDistance", 0., 0., 255.)?,
                    invert_depth: boolean(p, "invert", false)?,
                    brightness: number(p, "brightness", 0., 0., 100.)?,
                    threshold: number(p, "threshold", 255., 0., 255.)?,
                    noise: number(p, "noise", 0., 0., 100.)?,
                    distribution: if choice(p, "distribution", "uniform", &["uniform", "gaussian"])? == "gaussian" {
                        algo::Distribution::Gaussian
                    } else {
                        algo::Distribution::Uniform
                    },
                    monochromatic: boolean(p, "monochromatic", false)?,
                    seed: unsigned(p, "seed", 0, 0, u32::MAX)?,
                    depth_map: None,
                }
            }
            "filter.blur.radialBlur" => FilterParams::RadialBlur {
                amount: number(p, "amount", 10., 1., 100.)?,
                method: if choice(p, "method", "spin", &["spin", "zoom"])? == "zoom" { algo::RadialMethod::Zoom } else { algo::RadialMethod::Spin },
                quality: match choice(p, "quality", "good", &["draft", "good", "best"])? {
                    "draft" => algo::RadialQuality::Draft,
                    "best" => algo::RadialQuality::Best,
                    _ => algo::RadialQuality::Good,
                },
                center_x: number(p, "centerX", 0.5, 0., 1.)?,
                center_y: number(p, "centerY", 0.5, 0., 1.)?,
            },
            "filter.blur.smartBlur" => FilterParams::SmartBlur {
                radius: number(p, "radius", 3., 0.1, 100.)?,
                threshold: number(p, "threshold", 25., 0.1, 100.)?,
                quality: match choice(p, "quality", "high", &["low", "medium", "high"])? {
                    "low" => algo::BlurQuality::Low,
                    "medium" => algo::BlurQuality::Medium,
                    _ => algo::BlurQuality::High,
                },
                mode: match choice(p, "mode", "normal", &["normal", "edgeOnly", "overlayEdge"])? {
                    "edgeOnly" => algo::SmartBlurMode::EdgeOnly,
                    "overlayEdge" => algo::SmartBlurMode::OverlayEdge,
                    _ => algo::SmartBlurMode::Normal,
                },
            },
            "filter.blurGallery.irisBlur" => {
                let pins = if let Some(v) = p.get("pins") {
                    let input = v.as_array().filter(|v| !v.is_empty() && v.len() <= 256).ok_or("native Iris pins need1..256 entries")?;
                    let mut pins = Vec::new();
                    pins.try_reserve_exact(input.len()).map_err(|_| "native Iris allocation unavailable")?;
                    for pin in input {
                        pins.push(iris(pin, true)?)
                    }
                    pins
                } else {
                    vec![iris(p, false)?]
                };
                FilterParams::IrisBlur { pins }
            }
            "filter.blurGallery.tiltShift" => FilterParams::TiltShift {
                blur: number(p, "blur", 15., 0., 500.)?,
                center_x: number(p, "centerX", 0.5, -f64::from(f32::MAX), f64::from(f32::MAX))?,
                center_y: number(p, "centerY", 0.5, -f64::from(f32::MAX), f64::from(f32::MAX))?,
                angle: number(p, "angle", 0., -f64::from(f32::MAX), f64::from(f32::MAX))?,
                focus: number(p, "focus", 0.1, 0., 2.)?,
                transition: number(p, "transition", 0.15, 0.001, 2.)?,
            },
            "filter.noise.reduceNoise" => FilterParams::ReduceNoise {
                strength: number(p, "strength", 6., 0., 10.)?,
                preserve_details: number(p, "preserveDetails", 60., 0., 100.)?,
                reduce_color_noise: number(p, "reduceColorNoise", 45., 0., 100.)?,
                sharpen_details: number(p, "sharpenDetails", 25., 0., 100.)?,
                remove_jpeg_artifact: boolean(p, "removeJpegArtifact", false)?,
            },
            "filter.other.maximum" | "filter.other.minimum" => {
                let radius = number(p, "radius", 1., 0.2, 500.)?;
                let preserve = if choice(p, "preserve", "squareness", &["squareness", "roundness"])? == "roundness" {
                    algo::Preserve::Roundness
                } else {
                    algo::Preserve::Squareness
                };
                if command == "filter.other.maximum" { FilterParams::Maximum { radius, preserve } } else { FilterParams::Minimum { radius, preserve } }
            }
            "filter.stylize.findEdges" => FilterParams::FindEdges,
            "filter.stylize.emboss" => FilterParams::Emboss {
                angle: number(p, "angle", 135., -180., 180.)?,
                height: number(p, "height", 3., 1., 100.)?,
                amount: number(p, "amount", 100., 1., 500.)?,
            },
            "filter.stylize.oilPaint" => FilterParams::OilPaint {
                stylization: number(p, "stylization", 4., 0.1, 10.)?,
                cleanliness: number(p, "cleanliness", 5., 0., 10.)?,
                scale: number(p, "scale", 1., 0.1, 10.)?,
                bristle_detail: number(p, "bristleDetail", 5., 0., 10.)?,
                lighting: boolean(p, "lighting", true)?,
                angle: number(p, "angle", -60., -f64::from(f32::MAX), f64::from(f32::MAX))?,
                shine: number(p, "shine", 1., 0., 10.)?,
            },
            "filter.stylize.tiles" => FilterParams::Tiles {
                count: unsigned(p, "count", 10, 1, 99)?,
                max_offset: number(p, "maxOffset", 10., 1., 99.)?,
                fill: match choice(p, "fill", "background", &["background", "foreground", "inverse", "unaltered"])? {
                    "foreground" => algo::TileFill::Foreground,
                    "inverse" => algo::TileFill::Inverse,
                    "unaltered" => algo::TileFill::Unaltered,
                    _ => algo::TileFill::Background,
                },
                foreground: color(p, "foreground", [0., 0., 0., 1.])?,
                background: color(p, "background", [1.; 4])?,
                seed: unsigned(p, "seed", 0, 0, u32::MAX)?,
            },
            _ => FilterParams::DeInterlace {
                eliminate_even: choice(p, "eliminate", "oddFields", &["oddFields", "evenFields"])? == "evenFields",
                interpolate: choice(p, "createBy", "interpolation", &["interpolation", "duplication"])? == "interpolation",
            },
        })
    })())
}

pub(super) fn shadows_highlights(p: &Value) -> Result<algo::tone::ShadowsHighlights> {
    fields(
        p,
        &["shadowAmount", "shadowTone", "shadowRadius", "highlightAmount", "highlightTone", "highlightRadius", "color", "midtone", "blackClip", "whiteClip"],
    )?;
    let d = algo::tone::ShadowsHighlights::default();
    Ok(algo::tone::ShadowsHighlights {
        shadow_amount: number(p, "shadowAmount", d.shadow_amount, 0., 100.)?,
        shadow_tone: number(p, "shadowTone", d.shadow_tone, 0., 100.)?,
        shadow_radius: number(p, "shadowRadius", d.shadow_radius, 0., 2500.)?,
        highlight_amount: number(p, "highlightAmount", d.highlight_amount, 0., 100.)?,
        highlight_tone: number(p, "highlightTone", d.highlight_tone, 0., 100.)?,
        highlight_radius: number(p, "highlightRadius", d.highlight_radius, 0., 2500.)?,
        color: number(p, "color", d.color, -100., 100.)?,
        midtone: number(p, "midtone", d.midtone, -100., 100.)?,
        black_clip: number(p, "blackClip", d.black_clip, 0., 50.)?,
        white_clip: number(p, "whiteClip", d.white_clip, 0., 50.)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_geom::Rect;
    use photocraft_raster::{Interrupt, Surface};
    use serde_json::json;

    fn cases() -> Vec<(&'static str, Value, FilterParams)> {
        vec![
            (
                "filter.blur.lensBlur",
                json!({"radius":1,"shape":"square","depthMap":"transparency","brightness":4,"threshold":180,"monochromatic":true,"seed":7}),
                FilterParams::LensBlur {
                    radius: 1.,
                    blades: 4,
                    curvature: 0.,
                    rotation: 0.,
                    depth: algo::DepthSource::Transparency,
                    focal_distance: 0.,
                    invert_depth: false,
                    brightness: 4.,
                    threshold: 180.,
                    noise: 0.,
                    distribution: algo::Distribution::Uniform,
                    monochromatic: true,
                    seed: 7,
                    depth_map: None,
                },
            ),
            (
                "filter.blur.radialBlur",
                json!({"amount":3,"method":"zoom","quality":"best","centerX":0.3,"centerY":0.6}),
                FilterParams::RadialBlur { amount: 3., method: algo::RadialMethod::Zoom, quality: algo::RadialQuality::Best, center_x: 0.3, center_y: 0.6 },
            ),
            (
                "filter.blur.smartBlur",
                json!({"radius":1,"threshold":20,"quality":"low","mode":"overlayEdge"}),
                FilterParams::SmartBlur { radius: 1., threshold: 20., quality: algo::BlurQuality::Low, mode: algo::SmartBlurMode::OverlayEdge },
            ),
            (
                "filter.blurGallery.irisBlur",
                json!({"pins":[{"x":0.3,"y":0.6,"radiusX":0.4,"radiusY":0.2,"angle":15,"roundness":20,"feather":0.2,"blur":1}]}),
                FilterParams::IrisBlur {
                    pins: vec![algo::IrisPin { x: 0.3, y: 0.6, radius_x: 0.4, radius_y: 0.2, angle: 15., roundness: 20., feather: 0.2, blur: 1. }],
                },
            ),
            (
                "filter.blurGallery.tiltShift",
                json!({"blur":1,"centerX":0.3,"centerY":0.6,"angle":15,"focus":0.2,"transition":0.4}),
                FilterParams::TiltShift { blur: 1., center_x: 0.3, center_y: 0.6, angle: 15., focus: 0.2, transition: 0.4 },
            ),
            (
                "filter.noise.reduceNoise",
                json!({"strength":1,"preserveDetails":35,"reduceColorNoise":10,"sharpenDetails":5,"removeJpegArtifact":true}),
                FilterParams::ReduceNoise { strength: 1., preserve_details: 35., reduce_color_noise: 10., sharpen_details: 5., remove_jpeg_artifact: true },
            ),
            ("filter.other.maximum", json!({"radius":1,"preserve":"roundness"}), FilterParams::Maximum { radius: 1., preserve: algo::Preserve::Roundness }),
            ("filter.other.minimum", json!({"radius":1,"preserve":"squareness"}), FilterParams::Minimum { radius: 1., preserve: algo::Preserve::Squareness }),
            ("filter.stylize.findEdges", json!({}), FilterParams::FindEdges),
            ("filter.stylize.emboss", json!({"angle":45,"height":2,"amount":150}), FilterParams::Emboss { angle: 45., height: 2., amount: 150. }),
            (
                "filter.stylize.oilPaint",
                json!({"stylization":0.5,"cleanliness":1,"scale":0.5,"bristleDetail":2,"lighting":false,"angle":30,"shine":0}),
                FilterParams::OilPaint { stylization: 0.5, cleanliness: 1., scale: 0.5, bristle_detail: 2., lighting: false, angle: 30., shine: 0. },
            ),
            (
                "filter.stylize.tiles",
                json!({"count":3,"maxOffset":5,"fill":"foreground","foreground":[0.2,0.4,0.6,0.8],"background":"#abcdef","seed":17}),
                FilterParams::Tiles {
                    count: 3,
                    max_offset: 5.,
                    fill: algo::TileFill::Foreground,
                    foreground: [0.2, 0.4, 0.6, 0.8],
                    background: [171. / 255., 205. / 255., 239. / 255., 1.],
                    seed: 17,
                },
            ),
            (
                "filter.video.deInterlace",
                json!({"eliminate":"evenFields","createBy":"duplication"}),
                FilterParams::DeInterlace { eliminate_even: true, interpolate: false },
            ),
        ]
    }

    #[test]
    fn native_spatial_parameters_and_pixels_match_independent_typed_counterparts() {
        let bounds = Rect::new(-1, 2, 4, 5);
        for (command, p, expected) in cases() {
            let actual = filter(command, &p).unwrap().unwrap();
            assert_eq!(actual, expected, "{command}");
            for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                for mode in [ColorMode::Grayscale, ColorMode::Rgb] {
                    let fmt = PixelFormat::new(mode, depth, mode == ColorMode::Rgb);
                    let mut source = Surface::new(fmt);
                    let values = (0..15)
                        .flat_map(|i| if mode == ColorMode::Grayscale { vec![i as f32 / 14.] } else { vec![i as f32 / 14., 0.25, 0.75, 0.2 + i as f32 / 20.] })
                        .collect::<Vec<_>>();
                    source.write_region(bounds, &values);
                    let apply = |params| algo::apply_in_with(&source, params, bounds, bounds, None, bounds, &Interrupt::NONE).unwrap();
                    let actual = apply(&actual);
                    let expected = apply(&expected);
                    assert_eq!(actual.read_region(bounds), expected.read_region(bounds), "{command}/{mode:?}/{depth:?}");
                    assert_eq!(actual.format(), fmt);
                    assert!(actual.read_region(bounds).iter().all(|v| v.is_finite()));
                }
            }
        }
    }

    #[test]
    fn native_defaults_match_engine_and_bad_data_never_becomes_defaults() {
        assert_eq!(
            filter("filter.blur.radialBlur", &json!({})).unwrap().unwrap(),
            FilterParams::RadialBlur { amount: 10., method: algo::RadialMethod::Spin, quality: algo::RadialQuality::Good, center_x: 0.5, center_y: 0.5 }
        );
        assert_eq!(filter("filter.blurGallery.irisBlur", &json!({})).unwrap().unwrap(), FilterParams::IrisBlur { pins: vec![algo::IrisPin::default()] });
        assert_eq!(filter("filter.video.deInterlace", &json!({})).unwrap().unwrap(), FilterParams::DeInterlace { eliminate_even: false, interpolate: true });
        assert!(filter("source:operation", &json!({})).is_none());
        for (command, _, _) in cases() {
            assert!(filter(command, &json!(null)).unwrap().is_err());
            assert!(filter(command, &json!({"unexpected":1})).unwrap().is_err());
        }
        for (id, p) in [
            ("filter.blur.smartBlur", json!({"radius":0.09})),
            ("filter.blur.smartBlur", json!({"threshold":"20"})),
            ("filter.blur.radialBlur", json!({"quality":"old"})),
            ("filter.blur.radialBlur", json!({"centerX":2})),
            ("filter.blur.lensBlur", json!({"depthMap":"layerMask"})),
            ("filter.blur.lensBlur", json!({"depthMap":"channel"})),
            ("filter.noise.reduceNoise", json!({"removeJpegArtifact":1})),
            ("filter.stylize.tiles", json!({"count":3.5})),
            ("filter.stylize.tiles", json!({"seed":4294967296_u64})),
            ("filter.stylize.tiles", json!({"foreground":[0,0,0,2]})),
            ("filter.blurGallery.irisBlur", json!({"pins":[]})),
            ("filter.blurGallery.irisBlur", json!({"pins":vec![json!({});257]})),
            ("filter.blurGallery.irisBlur", json!({"pins":[{"blur":501}]})),
            ("filter.blurGallery.irisBlur", json!({"pins":[{"x":1e100}]})),
            ("filter.blurGallery.tiltShift", json!({"transition":0})),
            ("filter.other.maximum", json!({"preserve":"unknown"})),
        ] {
            assert!(filter(id, &p).unwrap().is_err(), "{id}: {p}");
        }
        assert!(filter("filter.blur.smartBlur", &json!({"radius":0.1,"threshold":0.1})).unwrap().is_ok());
        assert!(filter("filter.blurGallery.irisBlur", &json!({"pins":[{"radiusX":0.001,"feather":0.99}]})).unwrap().is_ok());
    }

    #[test]
    fn shadows_highlights_parameters_use_existing_native_tone_helper() {
        let p = json!({"shadowAmount":65,"shadowTone":35,"shadowRadius":2,"highlightAmount":20,"highlightTone":60,"highlightRadius":1,"color":-15,"midtone":10,"blackClip":0,"whiteClip":0});
        let actual = shadows_highlights(&p).unwrap();
        let expected = algo::tone::ShadowsHighlights {
            shadow_amount: 65.,
            shadow_tone: 35.,
            shadow_radius: 2.,
            highlight_amount: 20.,
            highlight_tone: 60.,
            highlight_radius: 1.,
            color: -15.,
            midtone: 10.,
            black_clip: 0.,
            white_clip: 0.,
        };
        assert_eq!(actual, expected);
        assert_eq!(shadows_highlights(&json!({})).unwrap(), algo::tone::ShadowsHighlights::default());
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
            let bounds = Rect::new(0, 0, 3, 2);
            source.write_region(bounds, &(0..6).flat_map(|i| [i as f32 / 5., 0.25, 0.75, 0.2 + i as f32 / 8.]).collect::<Vec<_>>());
            let mut output = source.read_region(bounds).as_chunks::<4>().0.to_vec();
            let mut independent = output.clone();
            algo::tone::shadows_highlights(&mut output, 3, 2, &actual);
            algo::tone::shadows_highlights(&mut independent, 3, 2, &expected);
            assert_eq!(output, independent);
            for (out, original) in output.iter().zip(source.read_region(bounds).as_chunks::<4>().0) {
                assert_eq!(out[3], original[3]);
            }
        }
        for p in [json!({"shadowRadius":2501}), json!({"whiteClip":51}), json!({"color":"bad"}), json!({"unknown":true})] {
            assert!(shadows_highlights(&p).is_err());
        }
    }
}
