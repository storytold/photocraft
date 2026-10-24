//! Import-time use of existing native Gray adjustments; no source evaluator.
use super::{IoError, Result, effects, invalid};
use photocraft_color::{BlendMode, ColorMode, SampleType};
use photocraft_doc::adjust::{CurvePoint, LevelsChannel, ToneSpace};
use photocraft_doc::{Adjustment, Document, Layer, LayerContent, LayerMask};
use photocraft_geom::{Rect, Size};
use photocraft_raster::{Interrupt, Surface};
use serde_json::Value;

pub(super) struct Budgets<'a> {
    pub workspace: &'a mut usize,
    pub metadata: &'a mut usize,
}

fn number(p: &Value, name: &str, default: f32, lo: f32, hi: f32) -> std::result::Result<f32, String> {
    let value = match p.get(name) {
        None => default,
        Some(v) => v.as_f64().ok_or_else(|| format!("native {name} is not numeric"))? as f32,
    };
    if value.is_finite() && (lo..=hi).contains(&value) { Ok(value) } else { Err(format!("native {name} is outside its supported range")) }
}

pub(super) fn adjustment(command: &str, p: &Value) -> std::result::Result<Adjustment, String> {
    Ok(match command {
        "image.adjustments.invert" => Adjustment::Invert,
        "image.adjustments.brightnessContrast" => {
            let legacy = p.get("legacy").and_then(Value::as_bool).unwrap_or(false);
            Adjustment::BrightnessContrast {
                brightness: number(p, "brightness", 0., -150., 150.)?,
                contrast: number(p, "contrast", 0., if legacy { -100. } else { -50. }, 100.)?,
                legacy,
            }
        }
        "image.adjustments.exposure" => Adjustment::Exposure {
            exposure: number(p, "exposure", 0., -20., 20.)?,
            offset: number(p, "offset", 0., -0.5, 0.5)?,
            gamma: number(p, "gamma", 1., 0.01, 9.99)?,
        },
        "image.adjustments.channelMixer" => {
            let monochrome = match p.get("monochrome") {
                None => false,
                Some(v) => v.as_bool().ok_or("Native monochrome flag is not boolean")?,
            };
            let mut matrix = [[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.]];
            if monochrome && p.get("gray").is_none() {
                matrix[0] = [0.4, 0.4, 0.2, 0.];
            }
            for (index, key) in ["red", "green", "blue"].iter().enumerate() {
                if let Some(value) = p.get(key) {
                    matrix[index] = mixer_row(value)?;
                }
            }
            if let Some(value) = p.get("gray") {
                matrix[0] = mixer_row(value)?;
            }
            Adjustment::ChannelMixer { matrix, monochrome }
        }
        "image.adjustments.colorLookup" => {
            if p.get("file").is_some() || p.get("lut").is_some() {
                return Err("Native coverage Color Lookup requires its already embedded table".into());
            }
            let data = p
                .get("data")
                .and_then(Value::as_str)
                .filter(|v| v.len() <= super::MAX_METADATA_BYTES)
                .ok_or("Native coverage lookup table is absent or oversized")?;
            let file = p.get("fileName").and_then(Value::as_str).unwrap_or("native.cube");
            let lookup = photocraft_cms::lutfile::parse(file, data.as_bytes()).map_err(|e| e.0)?;
            if lookup.size > 33 || !lookup.domain_is_default() {
                return Err("Native Color Lookup requires its ordinary0..1 input domain".into());
            }
            let tetrahedral = match p.get("interpolation").and_then(Value::as_str) {
                Some("tetrahedral") => true,
                Some("trilinear") | None => false,
                _ => return Err("Native Color Lookup interpolation is invalid".into()),
            };
            let dither = match p.get("dither") {
                Some(v) => v.as_bool().ok_or("Native Color Lookup dither is not boolean")?,
                None => false,
            };
            Adjustment::ColorLookup { name: lookup.title, size: lookup.size as u32, lut: Some(std::sync::Arc::new(lookup.data)), tetrahedral, dither }
        }
        "image.adjustments.posterize" => Adjustment::Posterize { levels: number(p, "levels", 4., 2., 255.)?.round() as u32 },
        "image.adjustments.threshold" => Adjustment::Threshold { level: number(p, "level", 128., 1., 255.)?.round() / 255. },
        "image.adjustments.levels" => Adjustment::Levels {
            master: levels(p.get("gray").unwrap_or(p))?,
            per_channel: [levels(&p["red"])?, levels(&p["green"])?, levels(&p["blue"])?],
            space: ToneSpace::Rgb,
            black: LevelsChannel::default(),
        },
        "image.adjustments.curves" => {
            let pencil = p.get("pencil").and_then(Value::as_bool).unwrap_or(false);
            Adjustment::Curves {
                master: curve(p.get("points").or_else(|| p.get("gray")).unwrap_or(&Value::Null), pencil)?,
                per_channel: [curve(&p["red"], pencil)?, curve(&p["green"], pencil)?, curve(&p["blue"], pencil)?],
                space: ToneSpace::Rgb,
                black: Vec::new(),
                pencil,
            }
        }
        _ => {
            return super::coverage_color::adjustment(command, p)
                .unwrap_or_else(|| Err(format!("Native coverage command adapter is unfinished for {command}")));
        }
    })
}

fn mixer_row(value: &Value) -> std::result::Result<[f32; 4], String> {
    let values = value.as_array().filter(|row| row.len() == 4).ok_or("Native Channel Mixer needs four coefficients")?;
    let mut result = [0.; 4];
    for (out, value) in result.iter_mut().zip(values) {
        let value =
            value.as_f64().filter(|v| v.is_finite() && (-200.0..=200.0).contains(v)).ok_or("Native Channel Mixer coefficient is outside +/-200 percent")?;
        *out = value as f32 / 100.;
    }
    Ok(result)
}

fn levels(p: &Value) -> std::result::Result<LevelsChannel, String> {
    let value = LevelsChannel {
        in_black: number(p, "inBlack", 0., 0., 253.)? / 255.,
        in_white: number(p, "inWhite", 255., 2., 255.)? / 255.,
        gamma: number(p, "gamma", 1., 0.01, 9.99)?,
        out_black: number(p, "outBlack", 0., 0., 255.)? / 255.,
        out_white: number(p, "outWhite", 255., 0., 255.)? / 255.,
    };
    if value.in_white - value.in_black < 2. / 255. - f32::EPSILON {
        return Err("Native coverage Levels interval is too narrow".into());
    }
    Ok(value)
}
fn curve(value: &Value, pencil: bool) -> std::result::Result<Vec<CurvePoint>, String> {
    if value.is_null() {
        return Ok(Vec::new());
    }
    let list = value.as_array().ok_or("Native curve is not an array")?;
    if list.len() > 256 || (pencil && list.len() != 256) {
        return Err("Native coverage curve length is invalid".into());
    }
    let scalar = |value: &Value| -> std::result::Result<f32, String> {
        let n = value.as_f64().ok_or("Native curve coordinate is not numeric")?;
        if !n.is_finite() || !(0.0..=255.0).contains(&n) {
            return Err("Native curve coordinate is out of range".into());
        }
        Ok(n as f32 / 255.)
    };
    let mut points = Vec::with_capacity(list.len());
    for (index, value) in list.iter().enumerate() {
        let point = if pencil {
            CurvePoint { input: index as f32 / 255., output: scalar(value)? }
        } else {
            let pair = value.as_array().filter(|p| p.len() == 2).ok_or("Native curve point is not a pair")?;
            CurvePoint { input: scalar(pair.first().ok_or("missing curve input")?)?, output: scalar(pair.get(1).ok_or("missing curve output")?)? }
        };
        if points.last().is_some_and(|last: &CurvePoint| last.input >= point.input) {
            return Err("Native curve inputs are not increasing".into());
        }
        points.push(point);
    }
    Ok(points)
}

enum Operation {
    Adjustment(Adjustment),
    Filter(photocraft_algo::FilterParams),
    Auto(photocraft_algo::tone::auto_levels::Kind),
    Hdr(photocraft_algo::tone::HdrToning),
    Shadows(photocraft_algo::tone::ShadowsHighlights),
    Lens(photocraft_algo::lens::LensCorrection),
    RemoveBlackMatte,
    Average,
}
fn operation(command: &str, p: &Value) -> std::result::Result<Operation, String> {
    use photocraft_algo::FilterParams as F;
    let filter = match command {
        "layer.matting.removeBlackMatte" => {
            if !p.as_object().is_some_and(|v| v.is_empty()) {
                return Err("Native Remove Black Matte needs empty parameters".into());
            }
            return Ok(Operation::RemoveBlackMatte);
        }
        "image.adjustments.shadowsHighlights" => return super::coverage_spatial::shadows_highlights(p).map(Operation::Shadows),
        "filter.lensCorrection" => return super::coverage_geometry::lens_correction(p).map(Operation::Lens),
        "filter.other.clampAlpha" => {
            let low = number(p, "low", 0., 0., 1.)?;
            let high = number(p, "high", 1., 0., 1.)?;
            if low > high {
                return Err("Native Clamp Alpha low exceeds high".into());
            }
            F::ClampAlpha { low, high }
        }
        "filter.other.thresholdAlpha" => F::ThresholdAlpha { value: number(p, "value", 0.5, 0., 1.)? },
        "filter.other.fillHoles" => {
            let connectivity = match p.get("connectivity") {
                None => 4,
                Some(v) => v.as_u64().filter(|n| matches!(n, 4 | 8)).ok_or("Native Fill Holes connectivity must be4or8")? as u8,
            };
            let alpha_only = match p.get("channel").and_then(Value::as_str) {
                None if p.get("channel").is_none() => false,
                Some("each") => false,
                Some("alpha") => true,
                _ => return Err("Native Fill Holes channel must be each or alpha".into()),
            };
            F::FillHoles { connectivity, alpha_only }
        }
        "filter.other.hsbHsl" => {
            let model = |key, default| match p.get(key) {
                None => Ok(default),
                Some(v) => match v.as_str() {
                    Some("rgb") => Ok(photocraft_algo::HsbModel::Rgb),
                    Some("hsb") => Ok(photocraft_algo::HsbModel::Hsb),
                    Some("hsl") => Ok(photocraft_algo::HsbModel::Hsl),
                    _ => Err("Native HSB/HSL model must be rgb, hsb or hsl"),
                },
            };
            F::HsbHsl { input: model("inputMode", photocraft_algo::HsbModel::Rgb)?, output: model("rowOrder", photocraft_algo::HsbModel::Hsb)? }
        }
        "filter.other.colorToAlpha" => {
            let mut color = [1.; 4];
            if let Some(v) = p.get("color") {
                let input = v.as_array().filter(|v| matches!(v.len(), 3 | 4)).ok_or("Native Color to Alpha needs RGB(A) color")?;
                for (out, v) in color.iter_mut().zip(input) {
                    *out = v.as_f64().filter(|v| v.is_finite() && (0.0..=1.0).contains(v)).ok_or("Native Color to Alpha needs finite unit color")? as f32;
                }
            }
            color[3] = 1.;
            F::ColorToAlpha {
                color,
                transparency_threshold: number(p, "transparencyThreshold", 0., 0., 1.)?,
                opacity_threshold: number(p, "opacityThreshold", 1., 0., 1.)?,
            }
        }
        "filter.other.custom" => {
            let input = p.get("kernel").and_then(Value::as_array).filter(|v| v.len() == 25).ok_or("Native Custom requires25 coefficients")?;
            let kernel = input
                .iter()
                .map(|v| {
                    v.as_f64()
                        .filter(|n| n.is_finite() && (-999.0..=999.0).contains(n))
                        .map(|n| n as f32)
                        .ok_or("Native Custom coefficient is outside its ordinary dialog")
                })
                .collect::<std::result::Result<Vec<_>, _>>()?;
            F::Custom { kernel, scale: number(p, "scale", 1., 1., 9999.)?, offset: number(p, "offset", 0., -9999., 9999.)? }
        }
        "filter.other.percentile" => {
            if ["colorPercentile", "alphaPercentile"].iter().any(|key| p.get(key).is_none()) {
                return Err("Native percentile requires explicit color and alpha percentiles".into());
            }
            let radius =
                p.get("radius").and_then(Value::as_u64).filter(|v| (1..=500).contains(v)).ok_or("Native percentile radius must be an integer1..500")? as u16;
            let channels = match p.get("channels") {
                None => None,
                Some(v) => Some(native_channels(v, true)?),
            };
            let shape = match p.get("shape").and_then(Value::as_str) {
                Some("disc") => photocraft_algo::rank::Neighborhood::Disc,
                Some("square") => photocraft_algo::rank::Neighborhood::Square,
                Some("diamond") => photocraft_algo::rank::Neighborhood::Diamond,
                _ => return Err("Native percentile shape is invalid".into()),
            };
            let percentile = |key| {
                p.get(key)
                    .and_then(Value::as_f64)
                    .filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
                    .map(|v| v as f32)
                    .ok_or_else(|| format!("Native {key} must be finite0..100"))
            };
            let options = photocraft_algo::rank::Params {
                radius,
                color_percentile: percentile("colorPercentile")?,
                alpha_percentile: percentile("alphaPercentile")?,
                shape,
                channels,
            };
            photocraft_algo::rank::validate(&options).map_err(|e| e.to_string())?;
            F::Percentile(options)
        }
        "filter.noise.channelNoise" => {
            let input = p
                .get("amounts")
                .and_then(Value::as_array)
                .filter(|v| !v.is_empty() && v.len() <= 5)
                .ok_or("Native channel-noise amounts must contain1..5 channels")?;
            if input.iter().any(|v| v.as_f64().is_none_or(|v| !v.is_finite() || !(0.0..=400.0).contains(&v))) {
                return Err("Native channel-noise amounts must be finite0..400".into());
            }
            let amounts: Vec<f32> = serde_json::from_value(Value::Array(input.clone())).map_err(|e| format!("Native channel-noise amounts: {e}"))?;
            let distribution = match p.get("distribution").and_then(Value::as_str) {
                Some("uniform") => photocraft_algo::Distribution::Uniform,
                Some("gaussian") => photocraft_algo::Distribution::Gaussian,
                _ => return Err("Native channel-noise distribution is invalid".into()),
            };
            let boolean = |key| p.get(key).and_then(Value::as_bool).ok_or_else(|| format!("Native channel-noise {key} must be boolean"));
            let seed = p.get("seed").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()).ok_or("Native channel-noise seed must be unsigned32-bit")?;
            let options = photocraft_algo::channel_noise::Params {
                amounts,
                distribution,
                monochromatic: boolean("monochromatic")?,
                relative: boolean("relative")?,
                seed,
            };
            photocraft_algo::channel_noise::validate(&options, options.amounts.len()).map_err(|e| e.to_string())?;
            F::ChannelNoise(options)
        }
        "filter.other.convolution" => {
            let input = p.get("kernel").and_then(Value::as_array).filter(|v| v.len() == 25).ok_or("Native convolution kernel must contain25 coefficients")?;
            let kernel: Vec<f64> = serde_json::from_value(Value::Array(input.clone())).map_err(|e| format!("Native convolution kernel: {e}"))?;
            let channels = native_channels(p.get("channels").ok_or("Native convolution needs channel selection")?, false)?;
            let scalar = |key| p.get(key).and_then(Value::as_f64).ok_or_else(|| format!("Native convolution {key} must be numeric"));
            let edge = match p.get("edge").and_then(Value::as_str) {
                Some("transparent") => photocraft_algo::convolution::Edge::Transparent,
                Some("repeat") => photocraft_algo::convolution::Edge::Repeat,
                Some("wrap") => photocraft_algo::convolution::Edge::Wrap,
                Some("black") => photocraft_algo::convolution::Edge::Black,
                Some("white") => photocraft_algo::convolution::Edge::White,
                _ => return Err("Native convolution edge is invalid".into()),
            };
            let options = photocraft_algo::convolution::Params {
                kernel,
                channels,
                scale: scalar("scale")?,
                offset: scalar("offset")?,
                alpha_weighted: p.get("alphaWeighted").and_then(Value::as_bool).ok_or("Native convolution alphaWeighted must be boolean")?,
                edge,
            };
            photocraft_algo::convolution::validate(&options, options.channels.len()).map_err(|e| e.to_string())?;
            F::Convolution(options)
        }
        "filter.blur.average" => {
            if !p.as_object().is_some_and(|object| object.is_empty()) {
                return Err("Native coverage Average requires empty parameters".into());
            }
            return Ok(Operation::Average);
        }
        "image.autoTone" | "image.autoContrast" => {
            if !p.as_object().is_some_and(|object| object.is_empty()) {
                return Err("Native coverage automatic analysis requires empty parameters".into());
            }
            return Ok(Operation::Auto(if command == "image.autoContrast" {
                photocraft_algo::tone::auto_levels::Kind::Contrast
            } else {
                photocraft_algo::tone::auto_levels::Kind::Tone
            }));
        }
        "filter.other.hdrToning" => {
            let options: photocraft_algo::tone::HdrToning =
                serde_json::from_value(p.clone()).map_err(|error| format!("Native coverage HDR parameters: {error}"))?;
            photocraft_algo::tone::checked_hdr::validate(&options).map_err(|error| error.to_string())?;
            return Ok(Operation::Hdr(options));
        }
        "filter.other.alphaCurve" => {
            let input =
                p.get("values").and_then(Value::as_array).filter(|values| values.len() == 256).ok_or("Native coverage Alpha Curve requires 256 values")?;
            let values = input
                .iter()
                .map(|value| {
                    value
                        .as_f64()
                        .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
                        .map(|value| value as f32)
                        .ok_or("Native Alpha Curve values require finite 0..1 samples".to_owned())
                })
                .collect::<std::result::Result<Vec<_>, _>>()?;
            F::AlphaCurve { values }
        }
        "filter.other.alphaToColor" => {
            if !p.as_object().is_some_and(|object| object.is_empty()) {
                return Err("Native Alpha to Color requires empty parameters".into());
            }
            F::AlphaToColor
        }
        "filter.other.setAlpha" => F::SetAlpha { value: number(p, "value", 1., 0., 1.)? },
        "filter.blur.gaussianBlur" => F::GaussianBlur { radius: number(p, "radius", 1., 0.1, 1000.)? },
        "filter.pixelate.mosaic" => F::Mosaic { cell_size: number(p, "cellSize", 12., 2., 200.)? },
        "filter.blur.boxBlur" => F::BoxBlur { radius: number(p, "radius", 1., 1., 2000.)? },
        "filter.blur.motionBlur" => F::MotionBlur { angle: number(p, "angle", 0., -360., 360.)?, distance: number(p, "distance", 1., 1., 2000.)? },
        "filter.blur.surfaceBlur" => F::SurfaceBlur { radius: number(p, "radius", 1., 1., 100.)?, threshold: number(p, "threshold", 2., 2., 255.)? },
        "filter.noise.median" => F::Median { radius: number(p, "radius", 1., 1., 500.)? },
        "filter.other.highPass" => F::HighPass { radius: number(p, "radius", 1., 0.1, 1000.)? },
        "filter.noise.addNoise" => F::AddNoise {
            amount: number(p, "amount", 1., 0., 400.)?,
            distribution: match p.get("distribution").and_then(Value::as_str).unwrap_or("uniform") {
                "uniform" => photocraft_algo::Distribution::Uniform,
                "gaussian" => photocraft_algo::Distribution::Gaussian,
                _ => return Err("Native coverage noise distribution is invalid".into()),
            },
            monochromatic: p.get("monochromatic").and_then(Value::as_bool).unwrap_or(false),
            seed: seed(p)?,
        },
        "filter.stylize.diffuse" => F::Diffuse {
            mode: match p.get("mode").and_then(Value::as_str).unwrap_or("normal") {
                "normal" => photocraft_algo::DiffuseMode::Normal,
                "darkenOnly" => photocraft_algo::DiffuseMode::DarkenOnly,
                "lightenOnly" => photocraft_algo::DiffuseMode::LightenOnly,
                "anisotropic" => photocraft_algo::DiffuseMode::Anisotropic,
                _ => return Err("Native coverage Diffuse mode is invalid".into()),
            },
            seed: seed(p)?,
        },
        "filter.stylize.wind" => F::Wind {
            method: match p.get("method").and_then(Value::as_str).unwrap_or("wind") {
                "wind" => photocraft_algo::WindMethod::Wind,
                "blast" => photocraft_algo::WindMethod::Blast,
                "stagger" => photocraft_algo::WindMethod::Stagger,
                _ => return Err("Native coverage Wind method is invalid".into()),
            },
            from_right: match p.get("direction").and_then(Value::as_str).unwrap_or("fromRight") {
                "fromRight" => true,
                "fromLeft" => false,
                _ => return Err("Native coverage Wind direction is invalid".into()),
            },
            seed: seed(p)?,
        },
        "filter.gallery.halftonePattern" => {
            let mut effect = photocraft_algo::GalleryEffect::new(photocraft_algo::GalleryFilter::HalftonePattern);
            effect.set("size", number(p, "size", 1., 1., 12.)?);
            effect.set("contrast", number(p, "contrast", 5., 0., 50.)?);
            if !effect.set_choice("patternType", p.get("patternType").and_then(Value::as_str).unwrap_or("dot")) {
                return Err("Native coverage halftone pattern type is invalid".into());
            }
            let color = |name| -> std::result::Result<Option<[f32; 4]>, String> {
                let Some(value) = p.get(name) else {
                    return Ok(None);
                };
                let values = value.as_array().filter(|v| v.len() == 3 || v.len() == 4).ok_or("Native coverage halftone color is not RGB(A)")?;
                let mut rgba = [0., 0., 0., 1.];
                for (output, input) in rgba.iter_mut().zip(values) {
                    let value = input.as_f64().ok_or("Native coverage halftone color is not numeric")?;
                    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                        return Err("Native coverage halftone color is outside native display range".into());
                    }
                    *output = value as f32;
                }
                Ok(Some(rgba))
            };
            if let Some(color) = color("foreground")? {
                effect.foreground = color;
            }
            if let Some(color) = color("background")? {
                effect.background = color;
            }
            F::FilterGallery { effects: vec![effect] }
        }
        "filter.sharpen.unsharpMask" => F::UnsharpMask {
            amount: number(p, "amount", 100., 1., 500.)?,
            radius: number(p, "radius", 1., 0.1, 1000.)?,
            threshold: number(p, "threshold", 0., 0., 255.)?,
        },
        _ => {
            if let Some(result) = super::coverage_spatial::filter(command, p).or_else(|| super::coverage_geometry::filter(command, p)) {
                return result.map(Operation::Filter);
            }
            return adjustment(command, p).map(Operation::Adjustment);
        }
    };
    Ok(Operation::Filter(filter))
}

fn native_channels(value: &Value, empty: bool) -> std::result::Result<Vec<bool>, String> {
    let input = value.as_array().filter(|v| v.len() <= 5 && (empty || !v.is_empty())).ok_or("Native channel selection has an invalid length")?;
    serde_json::from_value(Value::Array(input.clone())).map_err(|e| format!("Native channel selection: {e}"))
}

fn native_channel_semantics(operation: &Operation, channels: usize) -> std::result::Result<(), effects::MappingFailure> {
    if channels == 1
        && matches!(
            operation,
            Operation::Filter(
                photocraft_algo::FilterParams::AlphaCurve { .. }
                    | photocraft_algo::FilterParams::AlphaToColor
                    | photocraft_algo::FilterParams::SetAlpha { .. }
                    | photocraft_algo::FilterParams::ClampAlpha { .. }
                    | photocraft_algo::FilterParams::ThresholdAlpha { .. }
                    | photocraft_algo::FilterParams::ColorToAlpha { .. }
                    | photocraft_algo::FilterParams::FillHoles { alpha_only: true, .. }
            )
        )
    {
        return Err("Native alpha operation requires alpha-bearing pixels; standalone scalar coverage is preserved without inventing alpha".into());
    }
    let expected = match operation {
        Operation::Filter(photocraft_algo::FilterParams::Percentile(p)) => p.channels.as_ref().filter(|v| !v.is_empty()).map(Vec::len),
        Operation::Filter(photocraft_algo::FilterParams::ChannelNoise(p)) => Some(p.amounts.len()),
        Operation::Filter(photocraft_algo::FilterParams::Convolution(p)) => Some(p.channels.len()),
        _ => None,
    };
    if let Some(expected) = expected
        && expected != channels
    {
        return Err(effects::MappingFailure {
            reason: photocraft_doc::import_conversion::ConversionReason::MissingMapping,
            message: format!(
                "Native coverage stage describes {expected} color/alpha channels but this actual surface has {channels}. A layer's alpha is not a standalone scalar coverage plane; original coverage and source configuration are retained without dropping channel controls or inventing alpha"
            ),
        });
    }
    Ok(())
}

/// A native color-only filter may explicitly leave an owner's alpha alone.
/// Scalar coverage has no alpha operand; dropping only that disabled trailing
/// selector preserves every selected color channel and never invents alpha.
fn scalar_channel_selection(operation: &mut Operation, channels: usize) {
    if channels != 1 {
        return;
    }
    let selection = match operation {
        Operation::Filter(photocraft_algo::FilterParams::Convolution(p)) => Some(&mut p.channels),
        Operation::Filter(photocraft_algo::FilterParams::Percentile(p)) => p.channels.as_mut(),
        _ => None,
    };
    if let Some(selection) = selection
        && selection.len() == 2
        && selection.last() == Some(&false)
    {
        selection.pop();
    }
}

fn embedded_map_workspace(filters: &[photocraft_doc::SmartFilter], budget: usize) -> std::result::Result<usize, effects::MappingFailure> {
    let cost = filters.iter().try_fold(0usize, |sum, f| {
        let bytes = f.params.get("__displacePsd").and_then(Value::as_str).map_or(Some(0), |text| text.len().checked_mul(4).and_then(|n| n.checked_add(4096)));
        bytes.and_then(|n| sum.checked_add(n))
    });
    cost.filter(|n| *n <= budget).ok_or_else(|| effects::MappingFailure::resource("embedded native coordinate maps exceed actual remaining coverage workspace"))
}
fn native_filter_channel_semantics(
    filters: &[photocraft_doc::SmartFilter],
    channels: usize,
    budget: usize,
) -> std::result::Result<(), effects::MappingFailure> {
    embedded_map_workspace(filters, budget)?;
    for filter in filters {
        let mut typed = operation(&filter.command, &filter.params).map_err(effects::MappingFailure::from)?;
        scalar_channel_selection(&mut typed, channels);
        native_channel_semantics(&typed, channels)?;
    }
    Ok(())
}

/// Actual native filter geometry/context, separate from scalar import policy.
pub(in crate::xcf) struct NativeFilterDomain<'a> {
    pub reference_bounds: Rect,
    pub extent: Rect,
    pub document: &'a Document,
}

/// Replay one ordinary native command only. The snapshot caller applies shared
/// native Smart Filter opacity/blend and filter_mask after this raw operation.
pub(in crate::xcf) fn raw_filter(
    surface: &Surface,
    filter: &photocraft_doc::SmartFilter,
    domain: NativeFilterDomain<'_>,
    budget: &mut usize,
    ctl: &Interrupt<'_>,
) -> Result<Surface> {
    validate_native_frame(domain.reference_bounds)?;
    validate_native_frame(domain.extent)?;
    if surface.format().mode != domain.document.mode || surface.format().sample != domain.document.depth {
        return Err(invalid("native snapshot filter document and surface formats differ"));
    }
    validate_native_document(domain.document)?;
    let mut remaining = *budget;
    let pixels = native_filters_inner(surface, domain.reference_bounds, std::slice::from_ref(filter), &mut remaining, ctl, Some(domain.extent))?
        .map_err(|failure| invalid(failure.message))?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    *budget = remaining;
    Ok(pixels)
}

fn native_filters(
    surface: &Surface,
    bounds: Rect,
    filters: &[photocraft_doc::SmartFilter],
    budget: &mut usize,
    ctl: &Interrupt<'_>,
) -> Result<std::result::Result<Surface, effects::MappingFailure>> {
    native_filters_inner(surface, bounds, filters, budget, ctl, None)
}

fn native_filters_inner(
    surface: &Surface,
    bounds: Rect,
    filters: &[photocraft_doc::SmartFilter],
    budget: &mut usize,
    ctl: &Interrupt<'_>,
    raw_extent: Option<Rect>,
) -> Result<std::result::Result<Surface, effects::MappingFailure>> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let embedded = match embedded_map_workspace(filters, *budget) {
        Ok(n) => n,
        Err(e) => return Ok(Err(e)),
    };
    // Typed embedded maps remain live throughout this filter stack.
    *budget = budget.saturating_sub(embedded);
    let mut adjustments = match filters.iter().map(|filter| operation(&filter.command, &filter.params)).collect::<std::result::Result<Vec<_>, _>>() {
        Ok(value) => value,
        Err(error) => return Ok(Err(error.into())),
    };
    ctl.check().map_err(|_| IoError::Cancelled)?;
    for operation in &mut adjustments {
        scalar_channel_selection(operation, surface.channels());
        if let Err(failure) = native_channel_semantics(operation, surface.channels()) {
            return Ok(Err(failure));
        }
    }
    let mut processed = surface.clone();
    let fmt = surface.format();
    let reserve_frame = |bounds: Rect, budget: &mut usize| {
        if raw_extent.is_some() { reserve_native_frame(bounds, fmt, budget) } else { reserve_working_frame(bounds, budget, fmt.mode) }
    };
    for (filter, operation) in filters.iter().zip(adjustments) {
        if raw_extent.is_none() && (!filter.visible || filter.opacity == 0.0) {
            continue;
        }
        if !filter.opacity.is_finite() || !(0.0..=1.0).contains(&filter.opacity) {
            return Ok(Err("Native coverage component opacity is invalid".into()));
        }
        let before = processed.clone();
        let extent = raw_extent.unwrap_or_else(|| bounds.union(&processed.content_bounds()));
        if let Err(error) = reserve_frame(extent, budget) {
            return Ok(Err(effects::MappingFailure::resource(error.to_string())));
        }
        let adj = match operation {
            Operation::RemoveBlackMatte | Operation::Shadows(_) | Operation::Lens(_) => {
                let next = match operation {
                    Operation::RemoveBlackMatte => photocraft_algo::matting::remove_black_matte_with(&processed, extent, ctl).map_err(|e| {
                        if matches!(e, photocraft_algo::matting::Error::Cancelled) { IoError::Cancelled } else { invalid(format!("native matte: {e}")) }
                    }),
                    Operation::Shadows(p) => native_shadows(&processed, extent, &p, budget, ctl),
                    Operation::Lens(p) => native_lens(&processed, extent, bounds, &p, budget, ctl),
                    _ => return Ok(Err("invalid native whole-component stage".into())),
                };
                processed = match next {
                    Ok(value) => value,
                    Err(IoError::Cancelled) => return Err(IoError::Cancelled),
                    Err(error) => return Ok(Err(effects::MappingFailure::resource(error.to_string()))),
                };
                if raw_extent.is_none() && (filter.opacity < 1. || filter.blend != BlendMode::Normal) {
                    processed = native_mix(
                        &before,
                        &processed,
                        extent,
                        super::native_effects::NativeEnvelope { opacity: filter.opacity, blend: filter.blend, visible: true, mask: None },
                        ctl,
                    )?;
                }
                continue;
            }
            Operation::Average => {
                processed = match photocraft_algo::average::apply_with(&processed, extent, bounds, None, ctl) {
                    Ok(surface) => surface,
                    Err(photocraft_algo::average::Error::Cancelled) => return Err(IoError::Cancelled),
                    Err(error) => return Ok(Err(error.to_string().into())),
                };
                if raw_extent.is_none() && (filter.opacity < 1. || filter.blend != BlendMode::Normal) {
                    processed = native_mix(
                        &before,
                        &processed,
                        extent,
                        super::native_effects::NativeEnvelope { opacity: filter.opacity, blend: filter.blend, visible: true, mask: None },
                        ctl,
                    )?;
                }
                continue;
            }
            Operation::Auto(kind) => {
                let rows = match photocraft_algo::tone::auto_levels::analyze(&processed, bounds, kind, ctl) {
                    Ok(rows) => rows,
                    Err(photocraft_algo::tone::auto_levels::Error::Cancelled) => return Err(IoError::Cancelled),
                    Err(error) => return Ok(Err(error.to_string().into())),
                };
                Adjustment::Levels {
                    master: LevelsChannel::default(),
                    per_channel: rows.map(|[in_black, in_white, gamma]| LevelsChannel { in_black, in_white, gamma, ..Default::default() }),
                    space: ToneSpace::Rgb,
                    black: LevelsChannel::default(),
                }
            }
            Operation::Hdr(options) => {
                processed = match photocraft_algo::tone::checked_hdr::apply_with(&processed, &options, extent, bounds, None, ctl) {
                    Ok(surface) => surface,
                    Err(photocraft_algo::tone::checked_hdr::Error::Cancelled) => return Err(IoError::Cancelled),
                    Err(error) => return Ok(Err(error.to_string().into())),
                };
                if raw_extent.is_none() && (filter.opacity < 1. || filter.blend != BlendMode::Normal) {
                    processed = native_mix(
                        &before,
                        &processed,
                        extent,
                        super::native_effects::NativeEnvelope { opacity: filter.opacity, blend: filter.blend, visible: true, mask: None },
                        ctl,
                    )?;
                }
                continue;
            }
            Operation::Filter(params) => {
                ctl.check().map_err(|_| IoError::Cancelled)?;
                if matches!(
                    params,
                    photocraft_algo::FilterParams::AlphaCurve { .. }
                        | photocraft_algo::FilterParams::AlphaToColor
                        | photocraft_algo::FilterParams::SetAlpha { .. }
                ) && !fmt.alpha
                {
                    return Ok(Err("Native alpha operation requires alpha-bearing pixels; standalone scalar coverage is preserved".into()));
                }
                if let Err(error) = native_filter_workspace(&params, extent, bounds, fmt.channels(), *budget) {
                    return Ok(Err(effects::MappingFailure::resource(error.to_string())));
                }
                let area = photocraft_algo::output_area(&params, processed.content_bounds(), bounds, None).intersect(&extent);
                processed = match photocraft_algo::apply_in_with(&processed, &params, area, bounds, None, extent, ctl) {
                    Some(value) => value,
                    None if ctl.cancelled() => return Err(IoError::Cancelled),
                    None => return Ok(Err("Native coverage filter rejected its geometry or allocation".into())),
                };
                if raw_extent.is_none() && (filter.opacity < 1.0 || filter.blend != BlendMode::Normal) {
                    processed = native_mix(
                        &before,
                        &processed,
                        extent,
                        super::native_effects::NativeEnvelope { opacity: filter.opacity, blend: filter.blend, visible: true, mask: None },
                        ctl,
                    )?;
                }
                continue;
            }
            Operation::Adjustment(adj) => adj,
        };
        let mut y = extent.y0;
        while y < extent.y1 {
            ctl.check().map_err(|_| IoError::Cancelled)?;
            let band = Rect::new(extent.x0, y, extent.x1, y.saturating_add(64).min(extent.y1));
            let raw = processed.read_region(band);
            let mut buf =
                photocraft_compose::Buffer { rect: band, px: raw.chunks_exact(fmt.channels()).map(|pixel| photocraft_raster::to_rgba(&fmt, pixel)).collect() };
            let depth = match (fmt.sample, &adj) {
                (SampleType::F32, _) | (SampleType::U16, Adjustment::Posterize { .. } | Adjustment::Threshold { .. }) => Some(fmt.sample),
                _ => None,
            };
            photocraft_compose::adjust::apply_in(&adj, &mut buf, photocraft_compose::adjust::Transfer::for_document(fmt.mode, fmt.sample), depth, fmt.mode);
            encode(&mut processed, &buf);
            y = band.y1;
        }
        if raw_extent.is_none() && (filter.opacity < 1.0 || filter.blend != BlendMode::Normal) {
            processed = native_mix(
                &before,
                &processed,
                extent,
                super::native_effects::NativeEnvelope { opacity: filter.opacity, blend: filter.blend, visible: true, mask: None },
                ctl,
            )?;
        }
    }
    Ok(Ok(processed))
}

fn temporary_workspace(bytes: Option<u64>, budget: usize) -> Result<()> {
    if bytes.is_none_or(|n| n > budget as u64) {
        return Err(invalid("native coverage temporary workspace exceeds shared allowance"));
    }
    Ok(())
}

fn native_shadows(
    surface: &Surface,
    bounds: Rect,
    params: &photocraft_algo::tone::ShadowsHighlights,
    budget: &mut usize,
    ctl: &Interrupt<'_>,
) -> Result<Surface> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    // Native neighborhood uses17 brightness grids,2 grid buffers, guide/output,
    // retained luma fields and RGBA conversion. This bounds both neighborhoods
    // and their native Gaussian line scratch without charging freed temporaries.
    let pixels = u64::from(bounds.width()).checked_mul(u64::from(bounds.height()));
    temporary_workspace(
        pixels.and_then(|n| n.checked_mul(160)).and_then(|n| n.checked_add(u64::from(bounds.width().max(bounds.height())) * 32 + 32768)),
        *budget,
    )?;
    let raw = surface.try_read_region(bounds).map_err(|e| invalid(format!("native Shadows input: {e}")))?;
    let fmt = surface.format();
    let count = usize::try_from(pixels.ok_or_else(|| invalid("native Shadows extent overflow"))?).map_err(|_| invalid("native Shadows extent overflow"))?;
    let mut rgba = Vec::new();
    rgba.try_reserve_exact(count).map_err(|_| invalid("native Shadows RGBA allocation"))?;
    rgba.extend(raw.chunks_exact(fmt.channels()).map(|p| photocraft_raster::to_rgba(&fmt, p)));
    photocraft_algo::tone::shadows_highlights(&mut rgba, bounds.width() as usize, bounds.height() as usize, params);
    // The existing native whole-owner API has no Interrupt parameter. A request
    // observed during that kernel aborts publication immediately after it returns.
    ctl.check().map_err(|_| IoError::Cancelled)?;
    if rgba.iter().flatten().any(|v| !v.is_finite()) {
        return Err(invalid("native Shadows produced nonfinite pixels"));
    }
    let mut output = Surface::new(fmt);
    let width = bounds.width() as usize;
    let len = width.checked_mul(fmt.channels()).ok_or_else(|| invalid("native Shadows row overflow"))?;
    let mut row = Vec::new();
    row.try_reserve_exact(len).map_err(|_| invalid("native Shadows row allocation"))?;
    row.resize(len, 0.);
    for (y, pixels) in (bounds.y0..bounds.y1).zip(rgba.chunks_exact(width.max(1))) {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        for (pixel, out) in pixels.iter().zip(row.chunks_exact_mut(fmt.channels())) {
            photocraft_raster::from_rgba_into(&fmt, *pixel, out);
        }
        output.try_write_region(Rect::new(bounds.x0, y, bounds.x1, y.saturating_add(1)), &row).map_err(|e| invalid(format!("native Shadows output: {e}")))?;
    }
    Ok(output)
}

fn native_lens(
    surface: &Surface,
    extent: Rect,
    frame: Rect,
    params: &photocraft_algo::lens::LensCorrection,
    budget: &mut usize,
    ctl: &Interrupt<'_>,
) -> Result<Surface> {
    use photocraft_algo::lens::{EdgeMode, LensMap, Sample};
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let fmt = surface.format();
    let output_format = photocraft_color::PixelFormat::new(fmt.mode, fmt.sample, fmt.alpha || matches!(params.edge, EdgeMode::Transparency));
    // Execute the existing native remapper one native128px tile at a time.
    // Its worst sampling window is the whole frame; this avoids unbounded
    // simultaneous full-frame source windows from the parallel whole-frame API.
    let temporary = u64::from(frame.width())
        .checked_mul(u64::from(frame.height()))
        .and_then(|n| n.checked_mul(fmt.channels() as u64 * 4))
        .and_then(|n| n.checked_add(128 * 128 * (std::mem::size_of::<Option<Sample>>() as u64 + output_format.channels() as u64 * 16)))
        .and_then(|n| n.checked_add((photocraft_geom::TILE_SIZE as u64).pow(2) * output_format.bytes_per_pixel() as u64));
    temporary_workspace(temporary, *budget)?;
    let map = LensMap::new(params, frame);
    let mut output = Surface::new(output_format);
    let mut y = extent.y0;
    while y < extent.y1 {
        let endy = y.saturating_add(128).min(extent.y1);
        let mut x = extent.x0;
        while x < extent.x1 {
            ctl.check().map_err(|_| IoError::Cancelled)?;
            let endx = x.saturating_add(128).min(extent.x1);
            let tile = Rect::new(x, y, endx, endy);
            let native = photocraft_algo::lens::remap(surface, frame, tile, params.edge, &|x, y| if ctl.cancelled() { None } else { map.sample(x, y) });
            ctl.check().map_err(|_| IoError::Cancelled)?;
            let data = native.try_read_region(tile).map_err(|e| invalid(format!("native Lens output read: {e}")))?;
            if data.iter().any(|v| !v.is_finite()) {
                return Err(invalid("native Lens produced nonfinite pixels"));
            }
            output.try_write_region(tile, &data).map_err(|e| invalid(format!("native Lens output: {e}")))?;
            x = endx;
        }
        y = endy;
    }
    Ok(output)
}

fn native_scalar_commit(mut surface: Surface, bounds: Rect, ctl: &Interrupt<'_>) -> Result<(Surface, bool)> {
    let mut clipped = false;
    for y in bounds.y0..bounds.y1 {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let line = Rect::new(bounds.x0, y, bounds.x1, y.saturating_add(1));
        let mut row = surface.try_read_region(line).map_err(|e| invalid(format!("native coverage commit read: {e}")))?;
        if row.iter().any(|v| !v.is_finite()) {
            return Err(invalid("native coverage stage produced nonfinite pixels"));
        }
        if surface.channels() == 1 && row.iter().any(|v| !(0.0..=1.0).contains(v)) {
            row.iter_mut().for_each(|v| *v = v.clamp(0., 1.));
            surface.try_write_region(line, &row).map_err(|e| invalid(format!("native coverage commit write: {e}")))?;
            clipped = true;
        }
    }
    Ok((surface, clipped))
}

/// Admission follows the ordinary algo dispatcher: broad Gaussian/Box use
/// running-sum strips; every other Radius path reads the full inflated tile.
/// Checking all tile windows also bounds arbitrary native worker concurrency.
fn native_filter_workspace(params: &photocraft_algo::FilterParams, area: Rect, bounds: Rect, channels: usize, budget: usize) -> Result<()> {
    use photocraft_algo::{FilterParams as F, Halo, RadialMethod, RadialQuality};
    let w = u64::from(area.width());
    let h = u64::from(area.height());
    let bytes = channels as u64 * 4;
    if let F::Percentile(p) = params {
        let r = u64::from(p.radius);
        // Checked Rank's sequential256px tiles hold source,values,sorted,ranks
        // and Fenwick tree; rankusize entries are eightbytes on64bit targets.
        let cost = (w.min(256) + 2 * r).checked_mul(h.min(256) + 2 * r).and_then(|n| n.checked_mul(bytes + 32));
        return temporary_workspace(cost, budget);
    }
    let halo = params.halo_for(bounds);
    let tile = match halo {
        Halo::Radius(r) => ((2 * r.max(0) as u64).div_ceil(64) * 64).clamp(256, 2048),
        Halo::Bounds => 256,
    };
    let count = w.div_ceil(tile).checked_mul(h.div_ceil(tile));
    let workers = photocraft_algo::filter_worker_count().max(1) as u64;
    let full_tile = tile.checked_mul(tile).and_then(|n| n.checked_mul(bytes));
    let group = full_tile.map(|n| (photocraft_algo::FILTER_RESULT_BUDGET as u64 / n.max(1)).max(workers).max(1));
    let result = count
        .and_then(|k| group.map(|g| k.min(g)))
        .and_then(|k| w.min(tile).checked_mul(h.min(tile)).and_then(|n| n.checked_mul(bytes)).and_then(|n| n.checked_mul(k)));
    let cost = match halo {
        Halo::Radius(radius) => {
            let r = radius.max(0) as u64;
            if matches!(params,F::GaussianBlur{radius}|F::BoxBlur{radius} if *radius>4.) {
                // The actual running-sum fastpath streams strips, not2D halos.
                w.checked_mul(h)
                    .and_then(|n| (w + 2 * r).checked_mul(h).and_then(|x| n.checked_add(x)))
                    .and_then(|n| (h + 2 * r).checked_mul(w).and_then(|x| n.checked_add(x)))
                    .and_then(|n| n.checked_mul(bytes * 3))
            } else {
                let scratch = if matches!(params, F::OilPaint { .. } | F::ReduceNoise { .. } | F::LensBlur { .. } | F::SmartBlur { .. }) {
                    channels.max(4) as u64 * 4 * 16
                } else {
                    bytes * 5
                };
                (w.min(tile) + 2 * r)
                    .checked_mul(h.min(tile) + 2 * r)
                    .and_then(|n| n.checked_mul(scratch))
                    .and_then(|n| count.and_then(|k| n.checked_mul(k.min(workers))))
                    .and_then(|n| result.and_then(|x| n.checked_add(x)))
            }
        }
        Halo::Bounds => {
            let frame = bounds.union(&area);
            let shared = u64::from(frame.width()).checked_mul(u64::from(frame.height())).and_then(|n| n.checked_mul(bytes));
            let plan = if let F::RadialBlur { amount, method, quality, center_x, center_y } = params {
                // Exact native Plan interval admission: maximumcorner path,
                // qualitycap and triangular(n+1) pair tables. No pixels evaluated.
                let cx = bounds.x0 as f32 + bounds.width() as f32 * center_x;
                let cy = bounds.y0 as f32 + bounds.height() as f32 * center_y;
                let mut radius = 0f32;
                for y in [area.y0, area.y1.saturating_sub(1)] {
                    for x in [area.x0, area.x1.saturating_sub(1)] {
                        let dx = x as f32 + 0.5 - cx;
                        let dy = y as f32 + 0.5 - cy;
                        radius = radius.max((dx * dx + dy * dy).sqrt());
                    }
                }
                let extent = match method {
                    RadialMethod::Spin => amount.clamp(0., 100.).to_radians(),
                    RadialMethod::Zoom => amount.clamp(0., 100.) / 200.,
                };
                let max = match quality {
                    RadialQuality::Draft => 64.,
                    RadialQuality::Good => 256.,
                    RadialQuality::Best => 4096.,
                };
                let n = (extent * radius).ceil().clamp(1., max) as u64;
                n.checked_mul(n + 3).and_then(|x| x.checked_div(2)).and_then(|x| x.checked_mul(8)).and_then(|x| x.checked_add((n + 1) * 16 + 4096))
            } else {
                Some(0)
            };
            shared.and_then(|n| plan.and_then(|x| n.checked_add(x))).and_then(|n| result.and_then(|x| n.checked_add(x)))
        }
    };
    temporary_workspace(cost, budget)
}

fn native_warp(surface: &Surface, source: Rect, placement: photocraft_geom::Affine, destination: Rect, ctl: &Interrupt<'_>) -> Result<Surface> {
    let [a, b, c, d, e, f] = placement.m;
    let matrix = photocraft_algo::transform::Homography([a, c, e, b, d, f, 0., 0., 1.]);
    native_projective_warp(surface, source, &matrix, destination, ctl)
}

/// The ordinary native projective kernel, retaining its entire off-canvas
/// result. Affine whole-pixel translation fast paths remain the caller's policy.
pub(in crate::xcf) fn native_projective_warp_full(
    surface: &Surface,
    source: Rect,
    matrix: &photocraft_algo::transform::Homography,
    budget: &mut usize,
    ctl: &Interrupt<'_>,
) -> Result<Surface> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    validate_native_frame(source)?;
    if matrix.0.iter().any(|n| !n.is_finite() || n.abs() > 1.0e10)
        || matrix.inverse().is_none_or(|inverse| inverse.0.iter().any(|n| !n.is_finite()))
        || photocraft_algo::transform::warp_output_clipped(source, matrix)
    {
        return Err(invalid("native snapshot placement has an invalid or clipped projective extent"));
    }
    let output = photocraft_algo::transform::warp_output_bounds(source, matrix).ok_or_else(|| invalid("native snapshot projective bounds overflow"))?;
    validate_native_frame(output)?;
    let scratch = photocraft_algo::transform::warp_output_size(source, matrix)
        .and_then(|(width, height)| width.checked_mul(height))
        .and_then(|area| area.checked_mul(72))
        .and_then(|bytes| usize::try_from(bytes).ok())
        .ok_or_else(|| invalid("native snapshot projective workspace overflows"))?;
    let mut remaining = budget.checked_sub(scratch).ok_or_else(|| invalid("native snapshot projective workspace exceeds shared allowance"))?;
    reserve_native_frame(output, photocraft_color::PixelFormat { alpha: true, ..surface.format() }, &mut remaining)?;
    if !surface.format().alpha {
        reserve_native_frame(source, photocraft_color::PixelFormat { alpha: true, ..surface.format() }, &mut remaining)?;
    }
    let pixels = photocraft_algo::transform::warp_surface_with(surface, source, matrix, photocraft_algo::transform::Interp::Bicubic, ctl)
        .map_err(|error| invalid(format!("native snapshot placement: {error}")))?
        .ok_or(IoError::Cancelled)?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    *budget = remaining;
    Ok(pixels)
}

fn native_projective_warp(
    surface: &Surface,
    source: Rect,
    matrix: &photocraft_algo::transform::Homography,
    destination: Rect,
    ctl: &Interrupt<'_>,
) -> Result<Surface> {
    let output = photocraft_algo::transform::warp_surface_with(surface, source, matrix, photocraft_algo::transform::Interp::Bicubic, ctl)
        .map_err(|error| invalid(format!("native coverage transform: {error}")))?
        .ok_or(IoError::Cancelled)?;
    let output = output.try_convert(surface.format()).map_err(|error| invalid(format!("native coverage transform format: {error}")))?;
    Ok(photocraft_algo::resample::crop_surface(&output, destination))
}

fn seed(p: &Value) -> std::result::Result<u32, String> {
    match p.get("seed") {
        None => Ok(0),
        Some(value) => value.as_u64().and_then(|n| u32::try_from(n).ok()).ok_or_else(|| "Native coverage seed is not a u32".into()),
    }
}

fn native_mix(input: &Surface, output: &Surface, bounds: Rect, envelope: super::native_effects::NativeEnvelope, ctl: &Interrupt<'_>) -> Result<Surface> {
    if !envelope.opacity.is_finite() || !(0.0..=1.0).contains(&envelope.opacity) {
        return Err(invalid("native coverage envelope opacity is invalid"));
    }
    let dx = bounds.x0.checked_neg().ok_or_else(|| invalid("coverage origin overflow"))?;
    let dy = bounds.y0.checked_neg().ok_or_else(|| invalid("coverage origin overflow"))?;
    let mut doc = Document::new("Native coverage envelope", Size::new(bounds.width(), bounds.height()), input.format().mode, input.format().sample);
    doc.layers.push(Layer::new("Unchanged native input", LayerContent::Raster(photocraft_algo::resample::translate_surface(input, dx, dy))));
    let mut layer = Layer::new("Native processed coverage", LayerContent::Raster(photocraft_algo::resample::translate_surface(output, dx, dy)));
    layer.opacity = envelope.opacity;
    layer.blend = envelope.blend;
    layer.visible = envelope.visible;
    layer.mask = envelope.mask.map(|mut mask| {
        mask.surface = photocraft_algo::resample::translate_surface(&mask.surface, dx, dy);
        mask
    });
    doc.layers.push(layer);
    render_native(&doc, bounds, input.format(), ctl)
}

fn render_native(doc: &Document, bounds: Rect, format: photocraft_color::PixelFormat, ctl: &Interrupt<'_>) -> Result<Surface> {
    let mut output = Surface::new(format);
    photocraft_compose::render_bands(doc, doc.bounds(), 64, |mut band| -> Result<()> {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        band.rect = band.rect.translate(bounds.x0, bounds.y0);
        encode(&mut output, &band);
        Ok(())
    })?;
    Ok(output)
}

fn validate_native_frame(bounds: Rect) -> Result<()> {
    let width = i64::from(bounds.x1) - i64::from(bounds.x0);
    let height = i64::from(bounds.y1) - i64::from(bounds.y0);
    if width < 1 || height < 1 || width > i64::from(super::MAX_SIDE) || height > i64::from(super::MAX_SIDE) {
        return Err(invalid("native snapshot frame dimensions exceed bounded native extent"));
    }
    Ok(())
}

fn validate_native_document(document: &Document) -> Result<()> {
    validate_native_frame(document.bounds())?;
    if !document.resolution_dpi.is_finite() || document.resolution_dpi <= 0. {
        return Err(invalid("native snapshot document DPI is invalid"));
    }
    Ok(())
}

fn validate_prepared_layers(layers: &[Layer], depth: usize, nodes: &mut usize, ctl: &Interrupt<'_>) -> Result<()> {
    if depth > super::MAX_DEPTH {
        return Err(invalid("native snapshot prepared document exceeds group depth"));
    }
    for layer in layers {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        *nodes = nodes.checked_sub(1).ok_or_else(|| invalid("native snapshot prepared document exceeds layer count"))?;
        if !layer.visible || layer.opacity == 0. {
            continue;
        }
        match &layer.content {
            LayerContent::Text(text) if text.cache.is_none() => return Err(invalid("native snapshot Text has not been freshly prepared")),
            LayerContent::Shape(shape) if shape.cache.is_none() => return Err(invalid("native snapshot Shape has not been freshly prepared")),
            LayerContent::Smart(smart) if smart.cache.is_none() => return Err(invalid("native snapshot Smart has not been freshly prepared")),
            LayerContent::Group(group) => validate_prepared_layers(&group.children, depth + 1, nodes, ctl)?,
            _ => {}
        }
    }
    Ok(())
}

/// Render already prepared ordinary native contents, with the actual document
/// resources, ICC, DPI and style context. This does not project modes/depths.
pub(in crate::xcf) fn render_prepared_document(
    document: &Document,
    format: photocraft_color::PixelFormat,
    budget: &mut usize,
    ctl: &Interrupt<'_>,
) -> Result<Surface> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    validate_native_document(document)?;
    if format.mode != document.mode || format.sample != document.depth {
        return Err(invalid("native snapshot projection must follow rendering in its own document format"));
    }
    let mut nodes = super::MAX_LAYERS;
    validate_prepared_layers(&document.layers, 0, &mut nodes, ctl)?;
    let mut remaining = *budget;
    reserve_native_frame(document.bounds(), format, &mut remaining)?;
    let pixels = render_native(document, document.bounds(), format, ctl)?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    *budget = remaining;
    Ok(pixels)
}

fn encode(surface: &mut Surface, buffer: &photocraft_compose::Buffer) {
    let fmt = surface.format();
    let mut data = Vec::with_capacity(buffer.px.len().saturating_mul(fmt.channels()));
    let mut encoded = [0.; 5];
    for &pixel in &buffer.px {
        let count = photocraft_raster::from_rgba_into(&fmt, pixel, &mut encoded);
        data.extend_from_slice(encoded.get(..count).unwrap_or_default());
    }
    surface.write_region(buffer.rect, &data);
}

/// Price actual tiled surfaces as well as native compositor scratch, including
/// the full tile allocation for small or off-grid coverage rectangles.
fn reserve_frame(bounds: Rect, budget: &mut usize) -> Result<()> {
    if bounds.is_empty() {
        return Ok(());
    }
    let tile = i64::from(photocraft_geom::TILE_SIZE);
    let columns = i64::from(bounds.x1 - 1).div_euclid(tile) - i64::from(bounds.x0).div_euclid(tile) + 1;
    let rows = i64::from(bounds.y1 - 1).div_euclid(tile) - i64::from(bounds.y0).div_euclid(tile) + 1;
    let bytes = columns
        .checked_mul(rows)
        .and_then(|n| n.checked_mul(tile * tile * 8))
        .and_then(|n| {
            i64::from(bounds.width()).checked_mul(i64::from(bounds.height())).and_then(|area| area.checked_mul(64)).and_then(|scratch| n.checked_add(scratch))
        })
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| invalid("native coverage workspace overflows"))?;
    *budget = budget.checked_sub(bytes).ok_or_else(|| invalid("native coverage tiled workspace exceeds shared allowance"))?;
    Ok(())
}

/// Admission for actual native output tiles plus compositor scratch. Scalar
/// coverage keeps its existing conservative working-frame pricing separately.
pub(in crate::xcf) fn reserve_native_frame(bounds: Rect, format: photocraft_color::PixelFormat, budget: &mut usize) -> Result<()> {
    if bounds.is_empty() {
        return Ok(());
    }
    validate_native_frame(bounds)?;
    let mut remaining = *budget;
    reserve_frame(bounds, &mut remaining)?;
    let extra = format.bytes_per_pixel().saturating_sub(8);
    if extra != 0 {
        let tile = i64::from(photocraft_geom::TILE_SIZE);
        let columns = (i64::from(bounds.x1) - 1).div_euclid(tile) - i64::from(bounds.x0).div_euclid(tile) + 1;
        let rows = (i64::from(bounds.y1) - 1).div_euclid(tile) - i64::from(bounds.y0).div_euclid(tile) + 1;
        let bytes = columns
            .checked_mul(rows)
            .and_then(|tiles| tiles.checked_mul(tile * tile))
            .and_then(|area| usize::try_from(area).ok())
            .and_then(|area| area.checked_mul(extra))
            .ok_or_else(|| invalid("native snapshot typed tile storage overflows"))?;
        remaining = remaining.checked_sub(bytes).ok_or_else(|| invalid("native snapshot typed tiles exceed shared allowance"))?;
    }
    *budget = remaining;
    Ok(())
}

fn reserve_working_frame(bounds: Rect, budget: &mut usize, mode: ColorMode) -> Result<()> {
    let mut remaining = *budget;
    reserve_frame(bounds, &mut remaining)?;
    if mode == ColorMode::Rgb && !bounds.is_empty() {
        let tile = i64::from(photocraft_geom::TILE_SIZE);
        let columns = (i64::from(bounds.x1) - 1).div_euclid(tile) - i64::from(bounds.x0).div_euclid(tile) + 1;
        let rows = (i64::from(bounds.y1) - 1).div_euclid(tile) - i64::from(bounds.y0).div_euclid(tile) + 1;
        let extra = columns
            .checked_mul(rows)
            .and_then(|n| n.checked_mul(tile * tile * 8))
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| invalid("native RGB coverage tile storage overflows"))?;
        remaining = remaining.checked_sub(extra).ok_or_else(|| invalid("native RGB coverage tiled workspace exceeds shared allowance"))?;
    }
    *budget = remaining;
    Ok(())
}

/// Same source/destination ICC profiles, intent and BPC as native Smart RGB
/// contents projected into a native Gray parent. Alpha is copied, never CMS'd.
fn rgb_to_native_gray(surface: &Surface, bounds: Rect, ctl: &Interrupt<'_>) -> Result<Surface> {
    let transform = photocraft_cms::Transform::new(
        photocraft_cms::Builtin::Srgb.profile(),
        photocraft_cms::Builtin::DotGain20Compat.profile(),
        photocraft_cms::Intent::RelativeColorimetric,
        true,
    )
    .map_err(|e| invalid(format!("native coverage color conversion: {e}")))?;
    let fmt = surface.format();
    if fmt.mode != ColorMode::Rgb || !fmt.alpha {
        return Err(invalid("native RGB coverage conversion requires RGBA contents"));
    }
    let output_format = photocraft_color::PixelFormat::new(ColorMode::Grayscale, fmt.sample, true);
    let mut output = Surface::new(output_format);
    let width = bounds.width() as usize;
    let output_len = width.checked_mul(2).ok_or_else(|| invalid("native Gray coverage row overflows"))?;
    let mut row = Vec::new();
    row.try_reserve_exact(output_len).map_err(|_| invalid("native Gray coverage row allocation"))?;
    row.resize(output_len, 0.);
    for y in bounds.y0..bounds.y1 {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let line = Rect::new(bounds.x0, y, bounds.x1, y + 1);
        let input = surface.try_read_region(line).map_err(|e| invalid(format!("native coverage RGB read: {e}")))?;
        match fmt.sample {
            SampleType::F32 => transform.convert_f32(&input, 4, &mut row, 2, true),
            SampleType::U8 => {
                for (pixel, out) in input.as_chunks::<4>().0.iter().zip(row.as_chunks_mut::<2>().0) {
                    let mut encoded = [0u8; 4];
                    for (value, source) in encoded.iter_mut().zip(pixel) {
                        *value = (source.clamp(0., 1.) * 255. + 0.5) as u8;
                    }
                    let mut converted = [0u8; 2];
                    transform.convert_u8(&encoded, 4, &mut converted, 2, true);
                    for (value, source) in out.iter_mut().zip(converted) {
                        *value = f32::from(source) / 255.;
                    }
                }
            }
            SampleType::U16 => {
                for (pixel, out) in input.as_chunks::<4>().0.iter().zip(row.as_chunks_mut::<2>().0) {
                    let mut encoded = [0u16; 4];
                    for (value, source) in encoded.iter_mut().zip(pixel) {
                        *value = (source.clamp(0., 1.) * 65535. + 0.5) as u16;
                    }
                    let mut converted = [0u16; 2];
                    transform.convert_u16(&encoded, 4, &mut converted, 2, true);
                    for (value, source) in out.iter_mut().zip(converted) {
                        *value = f32::from(source) / 65535.;
                    }
                }
            }
        }
        if row.iter().any(|v| !v.is_finite()) {
            return Err(invalid("native RGB coverage conversion produced a nonfinite sample"));
        }
        output.try_write_region(line, &row).map_err(|e| invalid(format!("native coverage Gray write: {e}")))?;
    }
    Ok(output)
}

fn validate_scalar_coverage(surface: &Surface, bounds: Rect, ctl: &Interrupt<'_>) -> Result<()> {
    for y in bounds.y0..bounds.y1 {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let row = surface.try_read_region(Rect::new(bounds.x0, y, bounds.x1, y + 1)).map_err(|e| invalid(format!("native scalar coverage read: {e}")))?;
        if row.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
            return Err(invalid("native RGB working stage produced scalar coverage outside finite0..1; original coverage retained"));
        }
    }
    Ok(())
}

fn prepare_native_text(text: &mut photocraft_doc::TextLayer, document: &Document, budget: &mut usize, ctl: &Interrupt<'_>) -> Result<()> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    if text.text.len() > 65_536
        || text.font_family.len() > 4096
        || text.runs.len() > 4096
        || text.paragraphs.len() > 4096
        || !text.size_pt.is_finite()
        || text.size_pt <= 0.
        || text.transform.m.iter().any(|n| !n.is_finite())
        || !document.resolution_dpi.is_finite()
        || document.resolution_dpi <= 0.
        || text.runs.iter().any(|run| {
            run.style.font_family.len() > 4096
                || !run.style.size_pt.is_finite()
                || run.style.size_pt <= 0.
                || !run.style.horizontal_scale.is_finite()
                || !run.style.vertical_scale.is_finite()
        })
    {
        return Err(invalid("native coverage Text parameters exceed bounded layout admission"));
    }
    // Native shaping/layout owns several glyph/cluster/run vectors. Price those
    // before calling it; font bytes belong to the ordinary shared native font DB.
    let layout_bytes = text
        .text
        .len()
        .checked_mul(1024)
        .and_then(|bytes| text.runs.len().checked_mul(2048).and_then(|runs| bytes.checked_add(runs)))
        .and_then(|bytes| text.paragraphs.len().checked_mul(512).and_then(|runs| bytes.checked_add(runs)))
        .and_then(|bytes| bytes.checked_add(4096))
        .ok_or_else(|| invalid("native Text layout workspace overflows"))?;
    let mut remaining = budget.checked_sub(layout_bytes).ok_or_else(|| invalid("native Text layout exceeds shared coverage allowance"))?;
    let mut engine = photocraft_text::shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let layout = engine.layout(text, document.resolution_dpi);
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let warp = photocraft_text::render::layout_deform(&layout, text);
    let ink = photocraft_text::render::ink_rect_warped(&layout, &text.transform, warp.as_ref());
    if !ink.is_empty() {
        let width = i64::from(ink.x1) - i64::from(ink.x0);
        let height = i64::from(ink.y1) - i64::from(ink.y0);
        if width < 1 || height < 1 || width > i64::from(super::MAX_SIDE) || height > i64::from(super::MAX_SIDE) {
            return Err(invalid("native Text ink extent exceeds bounded coverage raster admission"));
        }
        reserve_native_frame(ink, document.pixel_format(), &mut remaining)?;
        // Besides output tiles and compositor scratch, the native rasterizer
        // retains an f32 color accumulator and up to three f32 bold-cover maps.
        let scratch = (width as usize)
            .checked_mul(height as usize)
            .and_then(|area| area.checked_mul(document.pixel_format().channels().saturating_mul(4).saturating_add(12)))
            .ok_or_else(|| invalid("native Text raster workspace overflows"))?;
        remaining = remaining.checked_sub(scratch).ok_or_else(|| invalid("native Text raster exceeds shared coverage allowance"))?;
    }
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let rendered = photocraft_text::render::rasterize_warped(&layout, &text.transform, document.pixel_format(), text.antialias, warp.as_ref());
    ctl.check().map_err(|_| IoError::Cancelled)?;
    if !ink.is_empty() && rendered.rect.is_empty() {
        return Err(invalid("native Text raster could not allocate its admitted ink extent"));
    }
    text.cache = Some(rendered.surface);
    *budget = remaining;
    Ok(())
}

pub(in crate::xcf) fn prepare_direct_layer(
    layer: &mut Layer,
    document: &Document,
    budget: &mut usize,
    depth: usize,
    nodes: &mut usize,
    ctl: &Interrupt<'_>,
) -> Result<()> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let reserve_frame = |bounds: Rect, budget: &mut usize| reserve_native_frame(bounds, document.pixel_format(), budget);
    if depth > super::MAX_DEPTH {
        return Err(invalid("native coverage groups exceed depth limit"));
    }
    *nodes = nodes.checked_sub(1).ok_or_else(|| invalid("native coverage structure exceeds layer count"))?;
    if !layer.opacity.is_finite() || !(0.0..=1.0).contains(&layer.opacity) {
        return Err(invalid("native coverage layer opacity is invalid"));
    }
    match &mut layer.content {
        LayerContent::Shape(shape) => {
            if shape.fill.as_ref().is_some_and(|fill| matches!(fill, photocraft_doc::Fill::Pattern { .. }))
                || shape.stroke.as_ref().is_some_and(|stroke| matches!(stroke.paint, photocraft_doc::Fill::Pattern { .. }))
            {
                return Err(invalid("native coverage shape pattern requires a separate native Pattern Fill mask"));
            }
            reserve_frame(document.bounds(), budget)?;
            let compiled = photocraft_vector::CompiledShape::new(shape, photocraft_vector::DEFAULT_TOLERANCE, document.bounds());
            ctl.check().map_err(|_| IoError::Cancelled)?;
            let mut cache = Surface::new(document.pixel_format());
            if let Some(bounds) = compiled.bounds() {
                let bounds = bounds.intersect(&document.bounds());
                let mut y = bounds.y0;
                while y < bounds.y1 {
                    ctl.check().map_err(|_| IoError::Cancelled)?;
                    let band = Rect::new(bounds.x0, y, bounds.x1, y.saturating_add(64).min(bounds.y1));
                    encode(&mut cache, &photocraft_compose::Buffer { rect: band, px: compiled.render_rgba(band) });
                    y = band.y1;
                }
            }
            shape.cache = Some(cache);
        }
        LayerContent::Group(group) => {
            for child in &mut group.children {
                prepare_direct_layer(child, document, budget, depth + 1, nodes, ctl)?;
            }
        }
        LayerContent::Raster(_) | LayerContent::Fill(_) | LayerContent::Adjustment(_) => {}
        LayerContent::Text(text) => prepare_native_text(text, document, budget, ctl)?,
        LayerContent::Smart(_) => return Err(invalid("native coverage needs prepared direct native data; deferred Smart caches are unavailable in IO")),
    }
    if !layer.effects.items.is_empty() {
        let margin = photocraft_compose::effects::margin(layer).max(0);
        let bounds = document.bounds();
        let expanded =
            Rect::new(bounds.x0.saturating_sub(margin), bounds.y0.saturating_sub(margin), bounds.x1.saturating_add(margin), bounds.y1.saturating_add(margin));
        for _ in &layer.effects.items {
            reserve_frame(expanded, budget)?;
        }
    }
    Ok(())
}

/// Both binary native constructions use the ordinary two-layer compositor.
/// Knockout differs only in standard advanced blending and zero fill opacity.
fn native_two_layers(
    below: &Surface,
    above: &Surface,
    bounds: Rect,
    appearance: (BlendMode, f32, photocraft_doc::Knockout),
    budget: &mut usize,
    ctl: &Interrupt<'_>,
) -> Result<std::result::Result<Surface, effects::MappingFailure>> {
    if below.format().mode != above.format().mode || below.format().sample != above.format().sample {
        return Ok(Err("Native coverage join requires compatible native surface formats".into()));
    }
    reserve_working_frame(bounds, budget, below.format().mode)?;
    let dx = bounds.x0.checked_neg().ok_or_else(|| invalid("native coverage join origin overflows"))?;
    let dy = bounds.y0.checked_neg().ok_or_else(|| invalid("native coverage join origin overflows"))?;
    let mut document = Document::new("Native coverage blend", Size::new(bounds.width(), bounds.height()), below.format().mode, below.format().sample);
    document.layers.push(Layer::new("Native lower result", LayerContent::Raster(photocraft_algo::resample::translate_surface(below, dx, dy))));
    let mut layer = Layer::new("Native upper result", LayerContent::Raster(photocraft_algo::resample::translate_surface(above, dx, dy)));
    let (blend, opacity, knockout) = appearance;
    layer.blend = blend;
    layer.opacity = opacity;
    if knockout != photocraft_doc::Knockout::None {
        layer.advanced.knockout = knockout;
        layer.fill_opacity = 0.;
    }
    document.layers.push(layer);
    Ok(Ok(render_native(&document, bounds, document.pixel_format(), ctl)?))
}

/// A flat native construction plan. This executes no stored source operation:
/// every Apply already contains ordinary native commands/layers and every Join
/// is a standard native layer blend. Earlier references make cycles impossible.
fn native_plan(
    surface: &Surface,
    bounds: Rect,
    steps: Vec<effects::NativeStage>,
    output: usize,
    budget: &mut usize,
    ctl: &Interrupt<'_>,
) -> Result<std::result::Result<Surface, effects::MappingFailure>> {
    use effects::NativeStage;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    if let Err(failure) = effects::validate_stages(&steps, output) {
        return Ok(Err(failure));
    }
    let mut remaining = *budget;
    let count = steps.len().checked_add(1).ok_or_else(|| invalid("native coverage plan length overflows"))?;
    // Surface clones share pixel tiles but allocate sparse-tree nodes/default
    // pixels. Charge those as well as the actual Vec capacity before allocation.
    let headers = count
        .checked_mul(std::mem::size_of::<Surface>())
        .and_then(|bytes| surface.tile_count().checked_mul(128).and_then(|tree| bytes.checked_add(tree)))
        .and_then(|bytes| bytes.checked_add(surface.format().bytes_per_pixel()))
        .ok_or_else(|| invalid("native coverage plan workspace overflows"))?;
    remaining = remaining.checked_sub(headers).ok_or_else(|| invalid("native coverage plan headers exceed shared allowance"))?;
    let mut results = Vec::new();
    results.try_reserve_exact(count).map_err(|_| invalid("native coverage plan result allocation failed"))?;
    results.push(surface.clone());
    for step in steps {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let result = match step {
            NativeStage::Align { input, target, horizontal, vertical, margin_x, margin_y, snap } => {
                let input = results.get(input).ok_or_else(|| invalid("native coverage alignment input disappeared"))?;
                let target = results.get(target).ok_or_else(|| invalid("native coverage alignment target disappeared"))?;
                let controls = super::native_alignment::Controls { horizontal, vertical, margin_x, margin_y, snap };
                let mapping = super::native_alignment::mapping(input, target, bounds, controls, ctl);
                ctl.check().map_err(|_| IoError::Cancelled)?;
                let mapping = match mapping {
                    Ok(mapping) => mapping,
                    Err(failure) => return Ok(Err(failure)),
                };
                match materialize_mapping(input, bounds, mapping, &mut remaining, ctl)? {
                    Ok((pixels, _notes)) => pixels,
                    Err(failure) => return Ok(Err(failure)),
                }
            }
            NativeStage::Channel { input, component } => {
                let input = results.get(input).ok_or_else(|| invalid("native coverage channel reference disappeared"))?;
                super::native_masks::channel(input, bounds, component, &mut remaining, ctl)?
            }
            NativeStage::Displace { input, map, coordinates, scale, wrap, metadata_limit } => {
                let input = results.get(input).ok_or_else(|| invalid("native coverage displacement input disappeared"))?;
                let map = results.get(map).ok_or_else(|| invalid("native coverage displacement map disappeared"))?;
                let mapping = super::native_displacement::mapping(
                    map,
                    bounds,
                    super::native_displacement::Controls { coordinates, scale, wrap, metadata_limit },
                    &mut remaining,
                    ctl,
                )?;
                match materialize_mapping(input, bounds, mapping, &mut remaining, ctl)? {
                    Ok((pixels, _notes)) => pixels,
                    Err(failure) => return Ok(Err(failure)),
                }
            }
            NativeStage::Mask { input, mask, component, invert, scale } => {
                let input = results.get(input).ok_or_else(|| invalid("native coverage masked input disappeared"))?;
                let mask = results.get(mask).ok_or_else(|| invalid("native coverage mask reference disappeared"))?;
                let mut mask = super::native_masks::mask(mask, bounds, component, invert, scale, &mut remaining, ctl)?;
                reserve_native_frame(bounds, input.format(), &mut remaining)?;
                let dx = bounds.x0.checked_neg().ok_or_else(|| invalid("native coverage masked origin overflows"))?;
                let dy = bounds.y0.checked_neg().ok_or_else(|| invalid("native coverage masked origin overflows"))?;
                mask.surface = photocraft_algo::resample::translate_surface(&mask.surface, dx, dy);
                let mut document =
                    Document::new("Native static masked coverage", Size::new(bounds.width(), bounds.height()), input.format().mode, input.format().sample);
                let mut layer = Layer::new("Native current masked input", LayerContent::Raster(photocraft_algo::resample::translate_surface(input, dx, dy)));
                layer.mask = Some(mask);
                document.layers.push(layer);
                render_native(&document, bounds, input.format(), ctl)?
            }
            NativeStage::Apply { input, mapping } => {
                let input = results.get(input).ok_or_else(|| invalid("native coverage input reference disappeared"))?;
                match materialize_mapping(input, bounds, *mapping, &mut remaining, ctl)? {
                    Ok((pixels, _notes)) => pixels,
                    Err(failure) => return Ok(Err(failure)),
                }
            }
            NativeStage::Join { below, above, blend, opacity } => {
                let below = results.get(below).ok_or_else(|| invalid("native coverage lower reference disappeared"))?;
                let above = results.get(above).ok_or_else(|| invalid("native coverage upper reference disappeared"))?;
                match native_two_layers(below, above, bounds, (blend, opacity, photocraft_doc::Knockout::None), &mut remaining, ctl)? {
                    Ok(pixels) => pixels,
                    Err(failure) => return Ok(Err(failure)),
                }
            }
            NativeStage::Knockout { below, above, opacity } => {
                let below = results.get(below).ok_or_else(|| invalid("native coverage lower reference disappeared"))?;
                let above = results.get(above).ok_or_else(|| invalid("native coverage upper reference disappeared"))?;
                match native_two_layers(below, above, bounds, (BlendMode::Normal, opacity, photocraft_doc::Knockout::Shallow), &mut remaining, ctl)? {
                    Ok(pixels) => pixels,
                    Err(failure) => return Ok(Err(failure)),
                }
            }
        };
        results.push(result);
    }
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let result = results.into_iter().nth(output).ok_or_else(|| invalid("native coverage output reference disappeared"))?;
    *budget = remaining;
    Ok(Ok(result))
}

/// Materialize fixed ordinary native layers only. Each composite branch starts
/// with the same input; branch order, clipping, masks, and blend are native data.
fn native_structure(
    surface: &Surface,
    bounds: Rect,
    structure: effects::NativeStructure,
    document: &mut Document,
    budget: &mut usize,
    ctl: &Interrupt<'_>,
) -> Result<std::result::Result<Surface, effects::MappingFailure>> {
    use effects::NativeStructure;
    let structure = match structure {
        NativeStructure::Stages { steps, output } => return native_plan(surface, bounds, steps, output, budget, ctl),
        structure => structure,
    };
    let mode = document.mode;
    let reserve_frame = |bounds: Rect, budget: &mut usize| reserve_working_frame(bounds, budget, mode);
    reserve_frame(bounds, budget)?;
    let mut nodes = super::MAX_LAYERS;
    match structure {
        NativeStructure::Stages { .. } => return Ok(Err("Nested native coverage plans require a flat construction".into())),
        NativeStructure::Replace { mut layer, patterns } => {
            super::native_effects::add_patterns(document, patterns, budget, ctl)?;
            prepare_direct_layer(&mut layer, document, budget, 0, &mut nodes, ctl)?;
            document.layers.push(*layer);
        }
        NativeStructure::Composite { branches } => {
            if branches.is_empty() || branches.len() > 64 {
                return Ok(Err("Native coverage composite requires 1..64 branches".into()));
            }
            let mut source_document = Document::new("Native coverage branch input", document.size, document.mode, surface.format().sample);
            source_document.layers.push(Layer::new("Original native coverage", LayerContent::Raster(surface.clone())));
            let input = render_native(&source_document, bounds, document.pixel_format(), ctl)?;
            for branch in branches {
                ctl.check().map_err(|_| IoError::Cancelled)?;
                if branch.filters.len() > 64
                    || branch.layer_effects.len() > 64
                    || branch.post_layer_effects.len() > 64
                    || !branch.opacity.is_finite()
                    || !(0.0..=1.0).contains(&branch.opacity)
                    || !branch.fill_opacity.is_finite()
                    || !(0.0..=1.0).contains(&branch.fill_opacity)
                {
                    return Ok(Err("Native coverage branch count or opacity is invalid".into()));
                }
                reserve_frame(bounds, budget)?;
                let mut layer = if let Some(fill) = branch.fill {
                    if branch.transform != photocraft_geom::Affine::IDENTITY
                        || branch.perspective.is_some()
                        || branch.filter_frame.is_some()
                        || !branch.filters.is_empty()
                    {
                        return Ok(Err("Native coverage fill branches require identity placement and separate postfilters".into()));
                    }
                    Layer::new("Native coverage fill", LayerContent::Fill(fill))
                } else {
                    if branch.transform.m.iter().any(|n| !n.is_finite() || n.abs() > 1.0e10) || branch.transform.inverse().is_none() {
                        return Ok(Err("Native coverage branch placement is invalid".into()));
                    }
                    let (native_input, source_bounds, placement) = if let Some(frame) = branch.filter_frame {
                        if branch.perspective.is_some() {
                            return Ok(Err("Native coverage framed perspective requires a separate native coordinate conversion".into()));
                        }
                        let width = i64::from(frame.x1) - i64::from(frame.x0);
                        let height = i64::from(frame.y1) - i64::from(frame.y0);
                        if width < 1 || height < 1 || width > i64::from(super::MAX_SIDE) || height > i64::from(super::MAX_SIDE) {
                            return Ok(Err("Native coverage filter frame dimensions are invalid".into()));
                        }
                        let dx = frame.x0.checked_neg().ok_or_else(|| invalid("native coverage filter frame origin overflows"))?;
                        let dy = frame.y0.checked_neg().ok_or_else(|| invalid("native coverage filter frame origin overflows"))?;
                        for (coordinate, delta) in [(bounds.x0, dx), (bounds.x1, dx), (bounds.y0, dy), (bounds.y1, dy)] {
                            coordinate.checked_add(delta).ok_or_else(|| invalid("native framed coverage placement overflows"))?;
                        }
                        let local_frame = Rect::new(0, 0, width as i32, height as i32);
                        reserve_frame(local_frame, budget)?;
                        let shifted = photocraft_algo::resample::translate_surface(&input, dx, dy);
                        let filtered = match native_filters(&shifted, local_frame, &branch.filters, budget, ctl)? {
                            Ok(value) => value,
                            Err(failure) => return Ok(Err(failure)),
                        };
                        let mut framed = Document::new("Native Gray filter frame", Size::new(width as u32, height as u32), document.mode, document.depth);
                        framed.layers.push(Layer::new("Native framed filters", LayerContent::Raster(filtered)));
                        let pixels = render_native(&framed, local_frame, framed.pixel_format(), ctl)?;
                        let placement = branch.transform.mul(&photocraft_geom::Affine::translate(f64::from(frame.x0), f64::from(frame.y0)));
                        (pixels, local_frame, placement)
                    } else {
                        (input.clone(), bounds, branch.transform)
                    };
                    let placed = if placement == photocraft_geom::Affine::IDENTITY && branch.perspective.is_none() {
                        native_input
                    } else {
                        let [a, b, c, d, e, f] = placement.m;
                        // Native Smart placement uses its projective placement
                        // when present; affine remains the editable fallback.
                        let matrix = photocraft_algo::transform::Homography(branch.perspective.unwrap_or([a, c, e, b, d, f, 0., 0., 1.]));
                        if matrix.0.iter().any(|n| !n.is_finite() || n.abs() > 1.0e10)
                            || matrix.inverse().is_none_or(|inverse| inverse.0.iter().any(|n| !n.is_finite()))
                        {
                            return Ok(Err("Native coverage projective branch placement is invalid".into()));
                        }
                        let required = photocraft_algo::transform::warp_output_size(source_bounds, &matrix)
                            .and_then(|(w, h)| w.checked_mul(h))
                            .and_then(|pixels| pixels.checked_mul(72));
                        if required.is_none_or(|bytes| bytes > *budget as u64) || photocraft_algo::transform::warp_output_clipped(source_bounds, &matrix) {
                            return Ok(Err(effects::MappingFailure::resource(
                                "Native coverage projective extent exceeds shared allowance or native transform reach",
                            )));
                        }
                        let output = photocraft_algo::transform::warp_output_bounds(source_bounds, &matrix)
                            .ok_or_else(|| invalid("native coverage branch bounds overflow"))?;
                        reserve_frame(output, budget)?;
                        native_projective_warp(&native_input, source_bounds, &matrix, output, ctl)?
                    };
                    let filtered = match native_filters(&placed, bounds, if branch.filter_frame.is_some() { &[] } else { &branch.filters }, budget, ctl)? {
                        Ok(value) => value,
                        Err(failure) => return Ok(Err(failure)),
                    };
                    Layer::new("Native coverage branch", LayerContent::Raster(filtered))
                };
                layer.opacity = branch.opacity;
                layer.fill_opacity = branch.fill_opacity;
                layer.advanced = branch.advanced;
                layer.blend = branch.blend;
                layer.clipped = branch.clipped;
                layer.mask = branch.mask;
                layer.vector_mask = branch.vector_mask;
                layer.effects.items = branch.layer_effects;
                if !branch.post_layer_effects.is_empty() {
                    reserve_frame(bounds, budget)?;
                    let mut inner_document = Document::new("Native Gray pre-style stage", document.size, document.mode, document.depth);
                    inner_document.patterns = document.patterns.clone();
                    let content = std::mem::replace(&mut layer.content, LayerContent::Raster(Surface::new(document.pixel_format())));
                    let mut inner = Layer::new("Native pre-style branch", content);
                    inner.effects = std::mem::take(&mut layer.effects);
                    inner.fill_opacity = layer.fill_opacity;
                    prepare_direct_layer(&mut inner, &inner_document, budget, 0, &mut nodes, ctl)?;
                    inner_document.layers.push(inner);
                    layer.content = LayerContent::Raster(render_native(&inner_document, bounds, document.pixel_format(), ctl)?);
                    layer.fill_opacity = 1.;
                    layer.effects.enabled = true;
                    layer.effects.items = branch.post_layer_effects;
                }
                prepare_direct_layer(&mut layer, document, budget, 0, &mut nodes, ctl)?;
                document.layers.push(layer);
            }
        }
    }
    // Keep structural alpha until the final native stage envelope. Dropping it
    // here would erase antialiasing, branch opacity, and masks on empty backdrop.
    Ok(Ok(render_native(document, bounds, document.pixel_format(), ctl)?))
}

/// Materialize ordinary native data without a source envelope or scalar commit.
/// Shared by the top-level conversion and finite, earlier-reference native plans.
fn materialize_mapping(
    surface: &Surface,
    bounds: Rect,
    mut mapping: effects::NativeMapping,
    workspace: &mut usize,
    ctl: &Interrupt<'_>,
) -> Result<std::result::Result<(Surface, Vec<String>), effects::MappingFailure>> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let working_rgb = mapping.working_rgb && surface.format().mode == ColorMode::Grayscale;
    mapping.working_rgb = false;
    let working_mode = if working_rgb { ColorMode::Rgb } else { surface.format().mode };
    let working_channels = if working_mode == ColorMode::Rgb {
        4
    } else if mapping.structure.is_some() {
        2
    } else {
        surface.channels()
    };
    let reserve_frame = |bounds: Rect, budget: &mut usize| reserve_working_frame(bounds, budget, working_mode);
    if let Err(failure) = native_filter_channel_semantics(&mapping.filters, working_channels, *workspace) {
        return Ok(Err(failure));
    }
    if let Some(effects::NativeStructure::Composite { branches }) = &mapping.structure {
        for branch in branches {
            if let Err(failure) = native_filter_channel_semantics(&branch.filters, working_channels, *workspace) {
                return Ok(Err(failure));
            }
        }
    }
    let structured = mapping.structure.is_some();
    let degrees = mapping.rotation_degrees;
    if mapping.quarter_turns > 3 || !degrees.is_finite() || degrees.abs() > 360. || (mapping.quarter_turns != 0 && degrees != 0.) {
        return Ok(Err("Native coverage orientation is invalid or ambiguous".into()));
    }
    let turns = if degrees != 0. && (degrees / 90.).fract() == 0. { ((degrees / 90.).rem_euclid(4.)) as u8 } else { mapping.quarter_turns };
    let arbitrary = if degrees != 0. && (degrees / 90.).fract() != 0. { degrees } else { 0. };
    let rotated = turns != 0 || arbitrary != 0.;
    if rotated && mapping.structure.is_some() {
        return Ok(Err("Native rotated coverage structure requires a separate native placement conversion".into()));
    }
    if rotated && !mapping.layer_effects.is_empty() {
        return Ok(Err("Native rotated coverage styles need a separate lighting/paint conversion".into()));
    }
    let (placement, size) = if arbitrary != 0. {
        match super::native_effects::arbitrary_orientation(Size::new(bounds.width(), bounds.height()), arbitrary) {
            Ok(value) => value,
            Err(error) => return Ok(Err(error.to_string().into())),
        }
    } else {
        super::native_effects::orientation(Size::new(bounds.width(), bounds.height()), turns)
    };
    let local_bounds = Rect::new(0, 0, bounds.width() as i32, bounds.height() as i32);
    let evaluation_bounds = if rotated { Rect::new(0, 0, size.width as i32, size.height as i32) } else { local_bounds };
    if let Err(error) = reserve_frame(bounds, workspace) {
        return Ok(Err(effects::MappingFailure::resource(error.to_string())));
    }
    let dx = bounds.x0.checked_neg().ok_or_else(|| invalid("native coverage origin overflow"))?;
    let dy = bounds.y0.checked_neg().ok_or_else(|| invalid("native coverage origin overflow"))?;
    let local_source = photocraft_algo::resample::translate_surface(surface, dx, dy);
    let local_source = if working_rgb {
        // Ordinary Smart Gray->RGB input conversion replicates native Gray
        // channels; RGB contents are alpha-bearing so joint components can
        // use ordinary clipping without inventing a scalar alpha channel.
        local_source
            .try_convert(photocraft_color::PixelFormat::new(ColorMode::Rgb, surface.format().sample, true))
            .map_err(|e| invalid(format!("native RGB coverage input allocation: {e}")))?
    } else {
        local_source
    };
    let mut processed = if rotated {
        if let Err(error) = reserve_frame(evaluation_bounds, workspace) {
            return Ok(Err(effects::MappingFailure::resource(error.to_string())));
        }
        let mut input = Document::new("Native Gray transform input", Size::new(bounds.width(), bounds.height()), working_mode, surface.format().sample);
        input.layers.push(Layer::new("Original coverage", LayerContent::Raster(local_source)));
        let alpha_source = render_native(&input, local_bounds, input.pixel_format(), ctl)?;
        native_warp(&alpha_source, local_bounds, placement, evaluation_bounds, ctl)?
    } else {
        local_source
    };
    let fmt = surface.format();
    let mut resources = Document::new("Native Gray coverage structure", Size::new(bounds.width(), bounds.height()), working_mode, fmt.sample);
    match super::native_effects::add_patterns(&mut resources, mapping.patterns, workspace, ctl) {
        Ok(()) => {}
        Err(IoError::Cancelled) => return Err(IoError::Cancelled),
        Err(error) => return Ok(Err(effects::MappingFailure::resource(error.to_string()))),
    }
    if let Some(structure) = mapping.structure.take() {
        processed = match native_structure(&processed, evaluation_bounds, structure, &mut resources, workspace, ctl) {
            Ok(Ok(value)) => value,
            Ok(Err(failure)) => return Ok(Err(failure)),
            Err(IoError::Cancelled) => return Err(IoError::Cancelled),
            Err(error) => return Ok(Err(effects::MappingFailure::resource(error.to_string()))),
        };
    }
    processed = match native_filters(&processed, evaluation_bounds, &mapping.filters, workspace, ctl)? {
        Ok(value) => value,
        Err(failure) => return Ok(Err(failure)),
    };
    if rotated {
        let inverse = placement.inverse().ok_or_else(|| invalid("native coverage orientation is singular"))?;
        processed = native_warp(&processed, evaluation_bounds, inverse, local_bounds, ctl)?;
    }
    if !mapping.layer_effects.is_empty() {
        let mut document = Document::new("Native Gray coverage styles", Size::new(bounds.width(), bounds.height()), working_mode, fmt.sample);
        document.patterns = resources.patterns;
        let mut layer = Layer::new("Native coverage appearance", LayerContent::Raster(processed.clone()));
        layer.effects.items = mapping.layer_effects;
        let margin = u64::try_from(photocraft_compose::effects::margin(&layer).max(0)).map_err(|_| invalid("native style margin overflows"))?;
        let cost = u64::from(bounds.width())
            .checked_add(margin.saturating_mul(2))
            .and_then(|w| u64::from(bounds.height()).checked_add(margin.saturating_mul(2)).and_then(|h| w.checked_mul(h)))
            .and_then(|n| n.checked_mul(128))
            .and_then(|n| n.checked_mul(layer.effects.items.len() as u64));
        let Some(remaining) = cost.and_then(|n| usize::try_from(n).ok()).and_then(|n| workspace.checked_sub(n)) else {
            return Ok(Err(effects::MappingFailure::resource("Native Gray style maps exceed the shared coverage workspace allowance")));
        };
        *workspace = remaining;
        document.layers.push(layer);
        processed = render_native(&document, local_bounds, document.pixel_format(), ctl)?;
    }
    if working_rgb {
        reserve_frame(local_bounds, workspace)?;
        processed = rgb_to_native_gray(&processed, local_bounds, ctl)?;
    }
    processed = photocraft_algo::resample::translate_surface(&processed, bounds.x0, bounds.y0);
    let mut notes = mapping.notes;
    if working_rgb {
        notes.push("Independent components run in ordinary native RGB/sRGB contents, then PhotoCraft's standard RelativeColorimetric/BPC RGB-to-working-Gray conversion extracts scalar coverage before its unchanged whole-stage mask/opacity/blend envelope. The native default Gray profile and native depth/alpha/clipping rules apply; no source evaluator or persistent working-mode rule remains".into());
    }
    if structured {
        notes.push("Native Gray structure/style alpha, including antialiasing, branch opacity and masks, composites over unchanged scalar coverage through the ordinary native whole-stage envelope".into());
    }
    if rotated {
        notes.push("Native Gray Free Transform preserves alpha through rotation/filter/inverse; transform-edge transparency composites over unchanged coverage in the native stage envelope".into());
    }
    Ok(Ok((processed, notes)))
}

/// One source stage becomes native pixels. Unsupported structures fail before
/// mutation; component envelopes use ordinary native layers, then native layers
/// mix the whole stage once. The original configuration stays in the report.
pub(super) fn apply(
    surface: &Surface,
    bounds: Rect,
    effect: &effects::StoredEffect<'_>,
    auxiliary: Option<&LayerMask>,
    linear: bool,
    budgets: &mut Budgets<'_>,
    ctl: &Interrupt<'_>,
) -> Result<std::result::Result<(Surface, String), effects::MappingFailure>> {
    if bounds.is_empty() || bounds.x0 == i32::MIN || bounds.y0 == i32::MIN || bounds.width() > super::MAX_SIDE || bounds.height() > super::MAX_SIDE {
        return Ok(Err("Native coverage owner extent cannot be safely rebased; original coverage is preserved".into()));
    }
    validate_scalar_coverage(surface, bounds, ctl)?;
    if !effect.visible || effect.opacity == 0. {
        return Ok(Ok((surface.clone(), "Disabled source stage left inactive".into())));
    }
    if effect.mask_offset != 0 && auxiliary.is_none() {
        return Ok(Err("Coverage stage's stored auxiliary mask was not decoded; stage remains inactive".into()));
    }
    let context = effects::MappingContext {
        mode: ColorMode::Grayscale,
        linear,
        resolution_dpi: 72.0,
        owner_bounds: Rect::new(0, 0, bounds.width() as i32, bounds.height() as i32),
        native_filter_bounds: Rect::new(0, 0, bounds.width() as i32, bounds.height() as i32),
        target_profile: None,
        remaining_generation_bytes: *budgets.workspace,
        remaining_metadata_bytes: *budgets.metadata,
        interrupt: *ctl,
    };
    let mapped = effects::map(effect, &context);
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let mapping = match mapped {
        Ok(mapping) => mapping,
        Err(error) => return Ok(Err(error)),
    };
    let result = (|| {
        let mut remaining_workspace = *budgets.workspace;
        let mut remaining_metadata = *budgets.metadata;
        let parameter_bytes = super::native_parameter_bytes(&mapping)?;
        let Some(next_metadata) = remaining_metadata.checked_sub(parameter_bytes) else {
            return Ok(Err(effects::MappingFailure {
                reason: photocraft_doc::import_conversion::ConversionReason::RetentionLimit,
                message: "Generated native coverage parameters exceed the shared metadata allowance".into(),
            }));
        };
        remaining_metadata = next_metadata;
        let (blend, warning) = if effect.mode == 62 { (BlendMode::Normal, None) } else { super::blend_mode(effect.mode as u32) };
        if warning.is_some() {
            return Ok(Err("Coverage blend mode has no established native conversion".into()));
        }
        let (processed, mut notes) = match materialize_mapping(surface, bounds, mapping, &mut remaining_workspace, ctl)? {
            Ok(value) => value,
            Err(failure) => return Ok(Err(failure)),
        };
        let output = native_mix(
            surface,
            &processed,
            bounds,
            super::native_effects::NativeEnvelope { opacity: effect.opacity, blend, visible: true, mask: auxiliary.cloned() },
            ctl,
        )?;
        let (output, clipped) = native_scalar_commit(output, bounds, ctl)?;
        if let Err(error) = validate_scalar_coverage(&output, bounds, ctl) {
            if matches!(error, IoError::Cancelled) {
                return Err(error);
            }
            return Ok(Err(effects::MappingFailure {
                reason: photocraft_doc::import_conversion::ConversionReason::InvalidOptionalData,
                message: error.to_string(),
            }));
        }
        ctl.check().map_err(|_| IoError::Cancelled)?;
        *budgets.workspace = remaining_workspace;
        *budgets.metadata = remaining_metadata;
        if clipped {
            notes.push("Native scalar channel commit clips finite coverage to0..1, matching ordinary PhotoCraft channel/mask editing; native HDR intermediate color remains a working-stage detail".into());
        }
        Ok(Ok((output, notes.join("; "))))
    })();
    match result {
        Err(IoError::Cancelled) => Err(IoError::Cancelled),
        Err(error) => Ok(Err(effects::MappingFailure::resource(format!("Native coverage stage remains inactive: {error}")))),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use effects::{Argument, ArgumentValue, StoredEffect};
    use photocraft_color::PixelFormat;

    #[test]
    fn snapshot_raw_native_filter_omits_component_envelope_and_preserves_actual_document_context() {
        for mode in [ColorMode::Grayscale, ColorMode::Rgb] {
            for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let format = PixelFormat::new(mode, sample, true);
                let bounds = Rect::new(0, 0, 3, 1);
                let extent = Rect::new(-1, 0, 4, 1);
                let mut source = Surface::new(format);
                let rgba = [[0.1, 0.2, 0.4, 0.25], [0.2, 0.4, 0.8, 0.5], [0.8, 0.4, 0.2, 1.]];
                let raw: Vec<_> = rgba.into_iter().flat_map(|pixel| photocraft_raster::from_rgba(&format, pixel)).collect();
                source.write_region(bounds, &raw);
                let mut document = Document::new("Actual native filter frame", Size::new(3, 1), mode, sample);
                document.resolution_dpi = 300.;
                document.icc_profile = Some(if mode == ColorMode::Rgb {
                    photocraft_cms::Builtin::DisplayP3.profile().to_bytes()
                } else {
                    photocraft_cms::Builtin::GrayGamma22.profile().to_bytes()
                });
                let profile = document.icc_profile.clone();
                let mut filter = effects::native_filter("image.adjustments.exposure", serde_json::json!({"exposure":1.0}));
                filter.opacity = 0.25;
                filter.blend = BlendMode::Multiply;
                let mut budget = super::super::MAX_PIXEL_BYTES;
                let actual =
                    raw_filter(&source, &filter, NativeFilterDomain { reference_bounds: bounds, extent, document: &document }, &mut budget, &Interrupt::NONE)
                        .unwrap();
                let raw = source.read_region(extent);
                let mut buffer = photocraft_compose::Buffer {
                    rect: extent,
                    px: raw.chunks_exact(format.channels()).map(|pixel| photocraft_raster::to_rgba(&format, pixel)).collect(),
                };
                photocraft_compose::adjust::apply_in(
                    &Adjustment::Exposure { exposure: 1., offset: 0., gamma: 1. },
                    &mut buffer,
                    photocraft_compose::adjust::Transfer::for_document(mode, sample),
                    (sample == SampleType::F32).then_some(sample),
                    mode,
                );
                let mut expected = Surface::new(format);
                encode(&mut expected, &buffer);
                assert_eq!(actual.read_region(extent), expected.read_region(extent), "{mode:?}/{sample:?}");
                assert_eq!(document.icc_profile, profile);
                assert_eq!(document.resolution_dpi, 300.);
                for (before, after) in
                    source.read_region(bounds).chunks_exact(format.channels()).zip(actual.read_region(bounds).chunks_exact(format.channels()))
                {
                    assert_eq!(before.last(), after.last(), "raw native adjustment must not duplicate same-pixel alpha");
                }
            }
        }
    }

    #[test]
    fn snapshot_prepared_native_render_keeps_pattern_resources_profile_dpi_and_rejects_missing_caches() {
        use photocraft_doc::{Color, Fill, Pattern, TextLayer};
        let bounds = Rect::new(0, 0, 64, 32);
        let mut document = Document::new("Actual native snapshot frame", Size::new(64, 32), ColorMode::Rgb, SampleType::U16);
        document.resolution_dpi = 300.;
        document.icc_profile = Some(photocraft_cms::Builtin::DisplayP3.profile().to_bytes());
        let mut tile = Surface::new(document.pixel_format());
        tile.write_region(Rect::new(0, 0, 2, 1), &[0.9, 0.2, 0.1, 1., 0.1, 0.4, 0.8, 1.]);
        let pattern = Pattern::new("Original native snapshot resource", tile, 2, 1);
        document.layers.push(Layer::new(
            "Actual Pattern Fill",
            LayerContent::Fill(Fill::Pattern { name: pattern.name.clone(), id: pattern.id.clone(), scale: 1., angle: 0., link: false, phase: (0., 0.) }),
        ));
        document.patterns.push(pattern);
        let text = TextLayer {
            text: "A".into(),
            font_family: "Inter".into(),
            size_pt: 6.,
            color: Color::WHITE,
            transform: photocraft_geom::Affine::translate(3., 25.),
            ..Default::default()
        };
        document.layers.push(Layer::new("Native text model", LayerContent::Text(text.clone())));
        let mut workspace = super::super::MAX_PIXEL_BYTES;
        assert!(render_prepared_document(&document, document.pixel_format(), &mut workspace, &Interrupt::NONE).is_err());
        assert_eq!(workspace, super::super::MAX_PIXEL_BYTES);
        let mut reference = document.clone();
        let LayerContent::Text(reference_text) = &mut reference.layers[1].content else { panic!("native text") };
        photocraft_text::shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner).render_layer(reference_text, 300., document.pixel_format());
        let mut layer = document.layers.remove(1);
        let mut nodes = super::super::MAX_LAYERS;
        prepare_direct_layer(&mut layer, &document, &mut workspace, 0, &mut nodes, &Interrupt::NONE).unwrap();
        document.layers.push(layer);
        let actual = render_prepared_document(&document, document.pixel_format(), &mut workspace, &Interrupt::NONE).unwrap();
        let mut expected = Surface::new(reference.pixel_format());
        encode(&mut expected, &photocraft_compose::render(&reference, bounds));
        assert_eq!(actual, expected);
        assert_ne!(actual.pixel(0, 0), actual.pixel(1, 0), "actual Pattern resource must be used");
        assert_eq!(document.icc_profile, reference.icc_profile);
        let wrong = PixelFormat::new(ColorMode::Grayscale, SampleType::U16, true);
        let original = workspace;
        assert!(render_prepared_document(&document, wrong, &mut workspace, &Interrupt::NONE).is_err());
        assert_eq!(workspace, original);
    }

    #[test]
    fn snapshot_full_native_projective_placement_retains_offcanvas_pixels_and_is_atomic() {
        use photocraft_algo::transform::Homography;
        let bounds = Rect::new(0, 0, 3, 2);
        let matrix = Homography([1., 0., -4.5, 0., 1., 2.25, 0.02, 0.01, 1.]);
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut source = Surface::new(PixelFormat::new(ColorMode::Grayscale, sample, true));
            source.write_region(bounds, &[0.2, 1., 0.4, 0.5, 0.6, 0.25, 0.8, 0.75, 1., 1., 0.5, 0.3]);
            let mut workspace = super::super::MAX_PIXEL_BYTES;
            let actual = native_projective_warp_full(&source, bounds, &matrix, &mut workspace, &Interrupt::NONE).unwrap();
            let expected = photocraft_algo::warp::place_source_projective(&source, bounds, &matrix, None).unwrap();
            assert_eq!(actual, expected);
            assert!(actual.content_bounds().x0 < 0, "offcanvas native placement must not be cropped to the owner canvas");
            for initial in [0, 1024] {
                let mut budget = initial;
                assert!(native_projective_warp_full(&source, bounds, &matrix, &mut budget, &Interrupt::NONE).is_err());
                assert_eq!(budget, initial);
            }
            let mut budget = super::super::MAX_PIXEL_BYTES;
            assert!(matches!(native_projective_warp_full(&source, bounds, &matrix, &mut budget, &Interrupt::cancel_only(&|| true)), Err(IoError::Cancelled)));
            assert_eq!(budget, super::super::MAX_PIXEL_BYTES);
        }
    }

    #[test]
    fn snapshot_raw_replay_and_prepared_render_do_not_publish_budget_on_failure_or_cancel() {
        use photocraft_doc::{SmartObject, SmartSource};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let bounds = Rect::new(0, 0, 3, 2);
        let mut source = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, true));
        source.write_region(bounds, &[0.2, 1., 0.4, 0.5, 0.6, 0.25, 0.8, 0.75, 1., 1., 0.5, 0.3]);
        let original = source.read_region(bounds);
        // Native RGBA/CMYKA F32 tiles exceed the scalar Gray storage floor.
        let mut gray = super::super::MAX_PIXEL_BYTES;
        let mut rgb = gray;
        let mut cmyk = gray;
        reserve_native_frame(bounds, PixelFormat::new(ColorMode::Grayscale, SampleType::F32, true), &mut gray).unwrap();
        reserve_native_frame(bounds, PixelFormat::new(ColorMode::Rgb, SampleType::F32, true), &mut rgb).unwrap();
        reserve_native_frame(bounds, PixelFormat::new(ColorMode::Cmyk, SampleType::F32, true), &mut cmyk).unwrap();
        assert!(gray > rgb && rgb > cmyk);
        let mut insufficient = super::super::MAX_PIXEL_BYTES - rgb - 1;
        let before = insufficient;
        assert!(reserve_native_frame(bounds, PixelFormat::new(ColorMode::Rgb, SampleType::F32, true), &mut insufficient).is_err());
        assert_eq!(insufficient, before);
        let mut document = Document::new("Actual native frame", Size::new(3, 2), ColorMode::Grayscale, SampleType::F32);
        document.layers.push(Layer::new("Original pixels", LayerContent::Raster(source.clone())));
        let filter = effects::native_filter("image.adjustments.exposure", serde_json::json!({"exposure":1.0}));
        for initial in [0, 1024] {
            let mut budget = initial;
            assert!(
                raw_filter(
                    &source,
                    &filter,
                    NativeFilterDomain { reference_bounds: bounds, extent: bounds, document: &document },
                    &mut budget,
                    &Interrupt::NONE
                )
                .is_err()
            );
            assert_eq!(budget, initial);
        }
        for after in [0, 3] {
            let calls = AtomicUsize::new(0);
            let cancel = || calls.fetch_add(1, Ordering::Relaxed) >= after;
            let mut budget = super::super::MAX_PIXEL_BYTES;
            assert!(matches!(
                raw_filter(
                    &source,
                    &filter,
                    NativeFilterDomain { reference_bounds: bounds, extent: bounds, document: &document },
                    &mut budget,
                    &Interrupt::cancel_only(&cancel)
                ),
                Err(IoError::Cancelled)
            ));
            assert_eq!(budget, super::super::MAX_PIXEL_BYTES);
        }
        let mut budget = super::super::MAX_PIXEL_BYTES;
        assert!(matches!(
            render_prepared_document(&document, document.pixel_format(), &mut budget, &Interrupt::cancel_only(&|| true)),
            Err(IoError::Cancelled)
        ));
        assert_eq!(budget, super::super::MAX_PIXEL_BYTES);
        let smart = SmartObject::new(SmartSource::Linked { path: "unread-native-snapshot.pcraft".into() }, photocraft_geom::Affine::IDENTITY, None);
        document.layers.push(Layer::new("Unprepared native Smart", LayerContent::Smart(smart)));
        let error = render_prepared_document(&document, document.pixel_format(), &mut budget, &Interrupt::NONE).unwrap_err();
        assert!(error.to_string().contains("not been freshly prepared"));
        assert_eq!(budget, super::super::MAX_PIXEL_BYTES);
        assert_eq!(source.read_region(bounds), original);
    }

    fn plan_adjustment_counterpart(surface: &Surface, bounds: Rect, adjustment: Adjustment) -> Surface {
        let channels = surface.channels();
        let raw = surface.read_region(bounds);
        let mut buffer = photocraft_compose::Buffer {
            rect: bounds,
            px: raw.chunks_exact(channels).map(|pixel| [pixel[0], pixel[0], pixel[0], if channels == 2 { pixel[1] } else { 1. }]).collect(),
        };
        let sample = surface.format().sample;
        photocraft_compose::adjust::apply_in(
            &adjustment,
            &mut buffer,
            photocraft_compose::adjust::Transfer::for_document(ColorMode::Grayscale, sample),
            (sample == SampleType::F32).then_some(sample),
            ColorMode::Grayscale,
        );
        let mut result = Surface::new(surface.format());
        encode(&mut result, &buffer);
        result
    }

    #[test]
    fn native_text_replacement_renders_editable_model_instead_of_a_saved_cache_at_each_depth() {
        use photocraft_doc::{Color, TextLayer};
        use photocraft_geom::Affine;
        let bounds = Rect::new(0, 0, 64, 32);
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut source = Surface::new(PixelFormat::new(ColorMode::Grayscale, sample, false));
            source.write_region(bounds, &vec![0.2; 64 * 32]);
            let text = TextLayer {
                text: "Native".into(),
                font_family: "Inter".into(),
                size_pt: 12.,
                color: Color::rgb(0.75, 0.75, 0.75),
                transform: Affine::translate(1., 18.),
                // A stale cached source appearance must never become authority.
                cache: Some(Surface::with_default(PixelFormat::new(ColorMode::Grayscale, sample, true), &[1., 1.])),
                ..Default::default()
            };
            let mut reference_text = text.clone();
            reference_text.cache = None;
            photocraft_text::shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner).render_layer(
                &mut reference_text,
                72.,
                PixelFormat::new(ColorMode::Grayscale, sample, true),
            );
            let mut document = Document::new("Independent native text", Size::new(64, 32), ColorMode::Grayscale, sample);
            document.layers.push(Layer::new("Native model rendered normally", LayerContent::Text(reference_text)));
            let expected = photocraft_compose::render(&document, bounds);
            let mapping = effects::NativeMapping {
                structure: Some(effects::NativeStructure::Replace { layer: Box::new(Layer::new("Native text", LayerContent::Text(text))), patterns: vec![] }),
                ..Default::default()
            };
            let mut workspace = super::super::MAX_PIXEL_BYTES;
            let (actual, _) = materialize_mapping(&source, bounds, mapping, &mut workspace, &Interrupt::NONE).unwrap().unwrap();
            let mut reference = Surface::new(document.pixel_format());
            encode(&mut reference, &expected);
            assert_eq!(actual.read_region(bounds), reference.read_region(bounds), "{sample:?}");
            assert!(actual.read_region(bounds).as_chunks::<2>().0.iter().any(|pixel| pixel[1] > 0.), "native glyph pixels must exist");
            assert!(actual.read_region(bounds).as_chunks::<2>().0.iter().any(|pixel| pixel[1] == 0.), "stale fullwhite cache must not be used");
            assert!(workspace < super::super::MAX_PIXEL_BYTES);
        }
    }

    #[test]
    fn native_text_preparation_obeys_frame_dpi_and_budget_cancel_atomicity() {
        use photocraft_doc::TextLayer;
        use photocraft_geom::Affine;
        use std::sync::atomic::{AtomicUsize, Ordering};
        let original =
            TextLayer { text: "Native".into(), font_family: "Inter".into(), size_pt: 10., transform: Affine::translate(1., 20.), ..Default::default() };
        let mut document = Document::new("Native frame DPI", Size::new(96, 48), ColorMode::Grayscale, SampleType::F32);
        for dpi in [72., 144.] {
            document.resolution_dpi = dpi;
            let mut actual = original.clone();
            let mut budget = super::super::MAX_PIXEL_BYTES;
            prepare_native_text(&mut actual, &document, &mut budget, &Interrupt::NONE).unwrap();
            let mut expected = original.clone();
            photocraft_text::shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner).render_layer(&mut expected, dpi, document.pixel_format());
            assert_eq!(actual.cache, expected.cache);
        }
        for allowance in [0, 1024] {
            let mut actual = original.clone();
            let mut budget = allowance;
            assert!(prepare_native_text(&mut actual, &document, &mut budget, &Interrupt::NONE).is_err());
            assert_eq!(actual, original);
            assert_eq!(budget, allowance);
        }
        for after in [0, 2] {
            let mut actual = original.clone();
            let mut budget = super::super::MAX_PIXEL_BYTES;
            let calls = AtomicUsize::new(0);
            let cancel = || calls.fetch_add(1, Ordering::Relaxed) >= after;
            assert!(matches!(prepare_native_text(&mut actual, &document, &mut budget, &Interrupt::cancel_only(&cancel)), Err(IoError::Cancelled)));
            assert_eq!(actual, original);
            assert_eq!(budget, super::super::MAX_PIXEL_BYTES);
        }
    }

    fn sample_native_plan() -> effects::NativeStructure {
        use effects::{NativeMapping, NativeStage, NativeStructure, native_filter};
        NativeStructure::Stages {
            steps: vec![
                NativeStage::Apply {
                    input: 0,
                    mapping: Box::new(NativeMapping {
                        filters: vec![native_filter("image.adjustments.exposure", serde_json::json!({"exposure":1.0}))],
                        ..Default::default()
                    }),
                },
                NativeStage::Apply {
                    input: 0,
                    mapping: Box::new(NativeMapping { filters: vec![native_filter("image.adjustments.invert", serde_json::json!({}))], ..Default::default() }),
                },
                NativeStage::Join { below: 1, above: 2, blend: BlendMode::Multiply, opacity: 1. },
                NativeStage::Apply {
                    input: 3,
                    mapping: Box::new(NativeMapping { filters: vec![native_filter("image.adjustments.invert", serde_json::json!({}))], ..Default::default() }),
                },
            ],
            output: 4,
        }
    }

    #[test]
    fn native_stages_match_independent_adjustments_blend_and_one_outer_scalar_envelope() {
        use photocraft_doc::AlphaChannel;
        let bounds = Rect::new(0, 0, 3, 2);
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let format = PixelFormat::new(ColorMode::Grayscale, sample, false);
            let mut source = Surface::new(format);
            source.write_region(bounds, &[0.1, 0.25, 0.4, 0.5, 0.75, 0.9]);
            let original = source.read_region(bounds);
            let mut workspace = super::super::MAX_PIXEL_BYTES;
            let mapping = effects::NativeMapping { structure: Some(sample_native_plan()), ..Default::default() };
            let (actual, _) = materialize_mapping(&source, bounds, mapping, &mut workspace, &Interrupt::NONE).unwrap().unwrap();

            let exposure = plan_adjustment_counterpart(&source, bounds, Adjustment::Exposure { exposure: 1., offset: 0., gamma: 1. });
            let inverse = plan_adjustment_counterpart(&source, bounds, Adjustment::Invert);
            let mut joined = Document::new("Independent ordinary native blend", Size::new(3, 2), ColorMode::Grayscale, sample);
            joined.layers.push(Layer::new("Native exposure result", LayerContent::Raster(exposure)));
            let mut upper = Layer::new("Native inverted input", LayerContent::Raster(inverse));
            upper.blend = BlendMode::Multiply;
            joined.layers.push(upper);
            let mut joined_surface = Surface::new(joined.pixel_format());
            encode(&mut joined_surface, &photocraft_compose::render(&joined, bounds));
            let reference = plan_adjustment_counterpart(&joined_surface, bounds, Adjustment::Invert);
            assert_eq!(actual.read_region(bounds), reference.read_region(bounds), "{sample:?}");

            let mut mask = Surface::new(format);
            mask.write_region(bounds, &[0., 0.2, 0.5, 0.7, 1., 0.4]);
            let auxiliary = LayerMask { surface: mask, ..LayerMask::reveal_all() };
            let envelope =
                super::super::native_effects::NativeEnvelope { opacity: 0.35, blend: BlendMode::Normal, visible: true, mask: Some(auxiliary.clone()) };
            let actual = native_mix(&source, &actual, bounds, envelope, &Interrupt::NONE).unwrap();
            let (actual, _) = native_scalar_commit(actual, bounds, &Interrupt::NONE).unwrap();
            let mut reference_document = Document::new("Independent whole native envelope", Size::new(3, 2), ColorMode::Grayscale, sample);
            reference_document.layers.push(Layer::new("Unchanged scalar", LayerContent::Raster(source.clone())));
            let mut upper = Layer::new("Complete native result", LayerContent::Raster(reference));
            upper.opacity = 0.35;
            upper.mask = Some(auxiliary);
            reference_document.layers.push(upper);
            let mut reference = Surface::new(format);
            encode(&mut reference, &photocraft_compose::render(&reference_document, bounds));
            assert_eq!(actual.read_region(bounds), reference.read_region(bounds), "single source envelope {sample:?}");
            assert_eq!(source.read_region(bounds), original);

            let mut native = Document::new("Native scalar owners", Size::new(3, 2), ColorMode::Grayscale, sample);
            let mut layer = Layer::new("Native pixels", LayerContent::Raster(source));
            layer.mask = Some(LayerMask { surface: actual.clone(), ..LayerMask::reveal_all() });
            native.layers.push(layer);
            native.channels.push(AlphaChannel::new("Native scalar channel", actual.clone()));
            native.selection = Some(actual.clone());
            let bytes = photocraft_format::save_to_bytes(&native, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
            assert_eq!(reopened.layers[0].mask.as_ref().unwrap().surface, actual);
            assert_eq!(reopened.channels[0].surface, actual);
            assert_eq!(reopened.selection.as_ref().unwrap(), &actual);
        }
    }

    #[test]
    fn native_stages_join_opacity_is_the_above_native_layers_opacity() {
        use effects::{NativeMapping, NativeStage};
        let bounds = Rect::new(-2, 3, 1, 4);
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut source = Surface::new(PixelFormat::new(ColorMode::Grayscale, sample, false));
            source.write_region(bounds, &[0.125, 0.5, 0.875]);
            for opacity in [0., 0.4, 1.] {
                let steps = vec![
                    NativeStage::Apply {
                        input: 0,
                        mapping: Box::new(NativeMapping {
                            filters: vec![effects::native_filter("image.adjustments.invert", serde_json::json!({}))],
                            ..Default::default()
                        }),
                    },
                    NativeStage::Join { below: 0, above: 1, blend: BlendMode::Multiply, opacity },
                ];
                let mut workspace = super::super::MAX_PIXEL_BYTES;
                let actual = native_plan(&source, bounds, steps, 2, &mut workspace, &Interrupt::NONE).unwrap().unwrap();
                let inverse = plan_adjustment_counterpart(&source, bounds, Adjustment::Invert);
                let mut document = Document::new("Independent native layer opacity", Size::new(3, 1), ColorMode::Grayscale, sample);
                document.layers.push(Layer::new("Native lower pixels", LayerContent::Raster(photocraft_algo::resample::translate_surface(&source, 2, -3))));
                let mut upper = Layer::new("Native upper pixels", LayerContent::Raster(photocraft_algo::resample::translate_surface(&inverse, 2, -3)));
                upper.blend = BlendMode::Multiply;
                upper.opacity = opacity;
                document.layers.push(upper);
                let mut buffer = photocraft_compose::render(&document, document.bounds());
                buffer.rect = bounds;
                let mut reference = Surface::new(document.pixel_format());
                encode(&mut reference, &buffer);
                assert_eq!(actual.read_region(bounds), reference.read_region(bounds), "{sample:?}/opacity{opacity}");
                if opacity == 0.4 {
                    assert_ne!(actual.pixel(-1, 3)[0], source.pixel(-1, 3)[0], "fractional native blend must change the result");
                }
            }
        }
    }

    #[test]
    fn native_stages_channel_and_mask_match_independent_native_data_at_each_depth() {
        use effects::{NativeMaskComponent, NativeStage};
        let bounds = Rect::new(-2, 3, 1, 5);
        for mode in [ColorMode::Grayscale, ColorMode::Rgb] {
            for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let format = PixelFormat::new(mode, sample, true);
                let mut source = Surface::new(format);
                let rgba =
                    [[-0.4, 0.3, 1.4, 0.], [0.2, 0.4, 0.8, 0.25], [1.4, 0.5, -0.2, 0.5], [0.8, 0.1, 0.2, 0.75], [0.4, 0.5, 0.6, 1.], [0.1, 0.7, 0.3, 0.4]];
                let data: Vec<_> = rgba.into_iter().flat_map(|pixel| photocraft_raster::from_rgba(&format, pixel)).collect();
                source.write_region(bounds, &data);
                let original = source.read_region(bounds);
                for component in [NativeMaskComponent::Alpha, NativeMaskComponent::Composite] {
                    let selected: Vec<f32> = original
                        .chunks_exact(format.channels())
                        .map(|pixel| {
                            match component {
                                NativeMaskComponent::Alpha => *pixel.last().unwrap(),
                                NativeMaskComponent::Composite => pixel[..mode.color_channels()].iter().sum::<f32>() / mode.color_channels() as f32,
                            }
                            .clamp(0., 1.)
                        })
                        .collect();
                    let mut expected_channel = Surface::new(format);
                    let pixels: Vec<_> =
                        selected.iter().flat_map(|&value| std::iter::repeat_n(value, mode.color_channels()).chain(std::iter::once(1.))).collect();
                    expected_channel.write_region(bounds, &pixels);
                    let mut workspace = super::super::MAX_PIXEL_BYTES;
                    let actual =
                        native_plan(&source, bounds, vec![NativeStage::Channel { input: 0, component }], 1, &mut workspace, &Interrupt::NONE).unwrap().unwrap();
                    assert_eq!(actual, expected_channel, "channel {mode:?}/{sample:?}/{component:?}");
                    for invert in [false, true] {
                        let scale = 0.6;
                        let steps = vec![NativeStage::Mask { input: 0, mask: 0, component, invert, scale }];
                        let mut workspace = super::super::MAX_PIXEL_BYTES;
                        let actual = native_plan(&source, bounds, steps, 1, &mut workspace, &Interrupt::NONE).unwrap().unwrap();
                        let mut mask = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
                        let values: Vec<_> = selected.iter().map(|&value| (if invert { 1. - value } else { value }) * scale).collect();
                        mask.write_region(Rect::new(0, 0, 3, 2), &values);
                        let mut document = Document::new("Independent ordinary native mask", Size::new(3, 2), mode, sample);
                        let mut layer =
                            Layer::new("Native original pixels", LayerContent::Raster(photocraft_algo::resample::translate_surface(&source, 2, -3)));
                        layer.mask = Some(LayerMask { surface: mask, ..LayerMask::reveal_all() });
                        document.layers.push(layer);
                        let mut buffer = photocraft_compose::render(&document, document.bounds());
                        buffer.rect = buffer.rect.translate(bounds.x0, bounds.y0);
                        let mut expected = Surface::new(format);
                        encode(&mut expected, &buffer);
                        assert_eq!(actual, expected, "mask {mode:?}/{sample:?}/{component:?}/invert{invert}");
                    }
                }
                assert_eq!(source.read_region(bounds), original);
            }
        }
    }

    #[test]
    fn native_stages_alignment_uses_actual_earlier_native_bounds() {
        use effects::{NativeMapping, NativeStage, NativeStructure};
        let bounds = Rect::new(0, 0, 6, 4);
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let format = PixelFormat::new(ColorMode::Grayscale, sample, true);
            let mut source = Surface::new(format);
            source.write_region(Rect::new(0, 0, 2, 1), &[0.3, 0.5, 1.4, 1.]);
            let mut target = Surface::new(format);
            target.write_region(Rect::new(4, 2, 6, 3), &[0.8, 1., 0.2, 1.]);
            let steps = vec![
                NativeStage::Apply {
                    input: 0,
                    mapping: Box::new(NativeMapping {
                        structure: Some(NativeStructure::Replace {
                            layer: Box::new(Layer::new("Actual earlier alignment target", LayerContent::Raster(target))),
                            patterns: vec![],
                        }),
                        ..Default::default()
                    }),
                },
                NativeStage::Align { input: 0, target: 1, horizontal: 0., vertical: 0., margin_x: 0., margin_y: 0., snap: true },
            ];
            let mut workspace = super::super::MAX_PIXEL_BYTES;
            let actual = native_plan(&source, bounds, steps, 2, &mut workspace, &Interrupt::NONE).unwrap().unwrap();
            let mut document = Document::new("Independent ordinary native translated layer", Size::new(6, 4), ColorMode::Grayscale, sample);
            document.layers.push(Layer::new("Native translation4,2", LayerContent::Raster(photocraft_algo::resample::translate_surface(&source, 4, 2))));
            let mut expected = Surface::new(format);
            encode(&mut expected, &photocraft_compose::render(&document, bounds));
            assert_eq!(actual, expected, "actual native content bounds {sample:?}");
        }
    }

    #[test]
    fn native_stages_displacement_uses_captured_native_channel_and_ordinary_displace() {
        use effects::{NativeMapCoordinates, NativeMaskComponent, NativeStage};
        let bounds = Rect::new(0, 0, 4, 3);
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let format = PixelFormat::new(ColorMode::Rgb, sample, true);
            let mut source = Surface::new(format);
            let pixels: Vec<_> = (0..12).flat_map(|index| [index as f32 / 10. - 0.1, (index % 3) as f32 / 3., (index % 4) as f32 / 4., 1.]).collect();
            source.write_region(bounds, &pixels);
            let steps = vec![
                NativeStage::Channel { input: 0, component: NativeMaskComponent::Alpha },
                NativeStage::Displace { input: 0, map: 1, coordinates: NativeMapCoordinates::Offsets, scale: 1., wrap: true, metadata_limit: 32_768 },
            ];
            let mut workspace = super::super::MAX_PIXEL_BYTES;
            let actual = native_plan(&source, bounds, steps, 2, &mut workspace, &Interrupt::NONE).unwrap().unwrap();
            // Independently author the ordinary native one-pixel XY offset map.
            // Native U8 map quantization and Displace sampling are authoritative.
            let image = photocraft_algo::Image { rect: bounds, ch: 3, data: [0.890625, 0.890625, 0.5].repeat(12) };
            let params =
                crate::smart_map::displace_embed_map(&serde_json::json!({"horizontal":1,"vertical":1,"fit":"stretch","undefinedAreas":"wrap"}), &image)
                    .unwrap();
            let map = crate::smart_map::displace_embedded_map(&params).unwrap();
            let typed = photocraft_algo::FilterParams::Displace {
                horizontal: 1.,
                vertical: 1.,
                stretch: true,
                undefined: photocraft_algo::UndefinedAreas::Wrap,
                map: Some(map),
            };
            let expected = photocraft_algo::apply_in_with(&source, &typed, bounds, bounds, None, bounds, &Interrupt::NONE).unwrap();
            assert_eq!(actual.read_region(bounds), expected.read_region(bounds), "native captured XY {sample:?}");
            if sample == SampleType::F32 {
                let mut xy = Surface::new(format);
                xy.write_region(bounds, &[1.28, -1.28, 0., 1.].repeat(12));
                let steps = vec![
                    NativeStage::Apply {
                        input: 0,
                        mapping: Box::new(effects::NativeMapping {
                            structure: Some(effects::NativeStructure::Replace {
                                layer: Box::new(Layer::new("Independent native signed XY", LayerContent::Raster(xy))),
                                patterns: vec![],
                            }),
                            ..Default::default()
                        }),
                    },
                    NativeStage::Displace { input: 0, map: 1, coordinates: NativeMapCoordinates::Offsets, scale: 1., wrap: false, metadata_limit: 32_768 },
                ];
                let mut workspace = super::super::MAX_PIXEL_BYTES;
                let actual = native_plan(&source, bounds, steps, 2, &mut workspace, &Interrupt::NONE).unwrap().unwrap();
                let image = photocraft_algo::Image { rect: bounds, ch: 3, data: [1., 0., 0.5].repeat(12) };
                let params =
                    crate::smart_map::displace_embed_map(&serde_json::json!({"horizontal":1,"vertical":1,"fit":"stretch","undefinedAreas":"repeat"}), &image)
                        .unwrap();
                let map = crate::smart_map::displace_embedded_map(&params).unwrap();
                let typed = photocraft_algo::FilterParams::Displace {
                    horizontal: 1.,
                    vertical: 1.,
                    stretch: true,
                    undefined: photocraft_algo::UndefinedAreas::Repeat,
                    map: Some(map),
                };
                let expected = photocraft_algo::apply_in_with(&source, &typed, bounds, bounds, None, bounds, &Interrupt::NONE).unwrap();
                assert_eq!(actual.read_region(bounds), expected.read_region(bounds), "native signed XY endpoints");
            }
        }
    }

    #[test]
    fn native_stages_channel_mask_and_displacement_failure_and_cancel_are_atomic() {
        use effects::{NativeMaskComponent, NativeStage};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let bounds = Rect::new(0, 0, 3, 2);
        let mut source = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, true));
        source.write_region(bounds, &[1.4, 0., -0.2, 0.25, 0.5, 0.5, 0.8, 0.75, 1., 1., 0.1, 0.4]);
        let original = source.read_region(bounds);
        for scale in [f32::NAN, -0.1, 1.1] {
            let mut workspace = super::super::MAX_PIXEL_BYTES;
            let steps = vec![NativeStage::Mask { input: 0, mask: 0, component: NativeMaskComponent::Alpha, invert: false, scale }];
            assert!(native_plan(&source, bounds, steps, 1, &mut workspace, &Interrupt::NONE).unwrap().is_err());
            assert_eq!(workspace, super::super::MAX_PIXEL_BYTES);
        }
        for kind in 0..4 {
            let step = || match kind {
                0 => NativeStage::Channel { input: 0, component: NativeMaskComponent::Composite },
                1 => NativeStage::Mask { input: 0, mask: 0, component: NativeMaskComponent::Alpha, invert: true, scale: 0.4 },
                2 => NativeStage::Displace {
                    input: 0,
                    map: 0,
                    coordinates: effects::NativeMapCoordinates::Offsets,
                    scale: 1.,
                    wrap: true,
                    metadata_limit: 32_768,
                },
                _ => NativeStage::Align { input: 0, target: 0, horizontal: 0., vertical: 0., margin_x: 0., margin_y: 0., snap: true },
            };
            for initial in [0, 1024] {
                let mut workspace = initial;
                let result = native_plan(&source, bounds, vec![step()], 1, &mut workspace, &Interrupt::NONE);
                if kind == 3 && initial == 1024 {
                    // Alignment resolves its native transform before the existing
                    // materializer reports insufficient pixel workspace as a
                    // retained resource failure rather than an I/O decode error.
                    let failure = result.unwrap().unwrap_err();
                    assert_eq!(failure.reason, photocraft_doc::import_conversion::ConversionReason::ResourceUnavailable);
                    assert!(failure.message.contains("tiled workspace exceeds shared allowance"), "{}", failure.message);
                } else {
                    let IoError::Xcf(message) = result.unwrap_err() else { panic!("expected bounded native XCF resource error") };
                    let expected = if initial == 0 {
                        "plan headers exceed shared allowance"
                    } else if kind == 2 {
                        "displacement map exceeds remaining import allowance"
                    } else {
                        "tiled workspace exceeds shared allowance"
                    };
                    assert!(message.contains(expected), "kind{kind}/budget{initial}: {message}");
                }
                assert_eq!(workspace, initial);
            }
            for after in [0, 4] {
                let calls = AtomicUsize::new(0);
                let cancel = || calls.fetch_add(1, Ordering::Relaxed) >= after;
                let mut workspace = super::super::MAX_PIXEL_BYTES;
                assert!(matches!(native_plan(&source, bounds, vec![step()], 1, &mut workspace, &Interrupt::cancel_only(&cancel)), Err(IoError::Cancelled)));
                assert_eq!(workspace, super::super::MAX_PIXEL_BYTES);
            }
        }
        assert_eq!(source.read_region(bounds), original);
    }

    #[test]
    fn native_stages_knockout_matches_independent_native_layers_at_each_depth() {
        use effects::{NativeMapping, NativeStage, NativeStructure};
        use photocraft_doc::Knockout;
        let bounds = Rect::new(0, 0, 3, 2);
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let format = PixelFormat::new(ColorMode::Grayscale, sample, false);
            let mut source = Surface::new(format);
            source.write_region(bounds, &[0.2, 0.4, 0.6, 0.8, 1., 0.5]);
            let original = source.read_region(bounds);
            let mut upper = Surface::new(PixelFormat::new(ColorMode::Grayscale, sample, true));
            upper.write_region(bounds, &[0.95, 0., 0.7, 0.25, 0.1, 1., 0.3, 0.75, 0.9, 0.5, 0.2, 0.2]);
            for opacity in [0., 0.5, 1.] {
                let steps = vec![
                    NativeStage::Apply {
                        input: 0,
                        mapping: Box::new(NativeMapping {
                            structure: Some(NativeStructure::Replace {
                                layer: Box::new(Layer::new("Native erasing coverage", LayerContent::Raster(upper.clone()))),
                                patterns: vec![],
                            }),
                            ..Default::default()
                        }),
                    },
                    NativeStage::Knockout { below: 0, above: 1, opacity },
                ];
                let mut workspace = super::super::MAX_PIXEL_BYTES;
                let actual = native_plan(&source, bounds, steps, 2, &mut workspace, &Interrupt::NONE).unwrap().unwrap();
                let mut expected = Document::new("Independent native shallow knockout", Size::new(3, 2), ColorMode::Grayscale, sample);
                expected.layers.push(Layer::new("Native lower pixels", LayerContent::Raster(source.clone())));
                let mut knockout = Layer::new("Native upper coverage", LayerContent::Raster(upper.clone()));
                knockout.blend = BlendMode::Normal;
                knockout.opacity = opacity;
                knockout.fill_opacity = 0.;
                knockout.advanced.knockout = Knockout::Shallow;
                expected.layers.push(knockout);
                let mut reference = Surface::new(expected.pixel_format());
                encode(&mut reference, &photocraft_compose::render(&expected, bounds));
                assert_eq!(actual.read_region(bounds), reference.read_region(bounds), "{sample:?}/opacity{opacity}");
                if opacity == 1. {
                    let erased = actual.pixel(2, 0);
                    assert!(erased[0] < 0.3 || erased[1] < 0.5, "full native knockout must change the covered lower pixel");
                }

                let mut mask = Surface::new(format);
                mask.write_region(bounds, &[0., 0.25, 0.5, 0.75, 1., 0.4]);
                let auxiliary = LayerMask { surface: mask, ..LayerMask::reveal_all() };
                let actual = native_mix(
                    &source,
                    &actual,
                    bounds,
                    super::super::native_effects::NativeEnvelope { opacity: 0.35, blend: BlendMode::Normal, visible: true, mask: Some(auxiliary.clone()) },
                    &Interrupt::NONE,
                )
                .unwrap();
                let (actual, _) = native_scalar_commit(actual, bounds, &Interrupt::NONE).unwrap();
                let mut document = Document::new("Independent native final envelope", Size::new(3, 2), ColorMode::Grayscale, sample);
                document.layers.push(Layer::new("Original scalar", LayerContent::Raster(source.clone())));
                let mut layer = Layer::new("Completed knockout pixels", LayerContent::Raster(reference));
                layer.opacity = 0.35;
                layer.mask = Some(auxiliary);
                document.layers.push(layer);
                let mut reference = Surface::new(format);
                encode(&mut reference, &photocraft_compose::render(&document, bounds));
                assert_eq!(actual.read_region(bounds), reference.read_region(bounds), "one outer envelope {sample:?}/opacity{opacity}");
                assert_eq!(source.read_region(bounds), original);
            }
        }
    }

    #[test]
    fn native_stages_knockout_budget_cancel_and_invalid_opacity_are_atomic() {
        use effects::NativeStage;
        use std::sync::atomic::{AtomicUsize, Ordering};
        let bounds = Rect::new(0, 0, 3, 2);
        let mut source = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        source.write_region(bounds, &[0.2, 0.4, 0.6, 0.8, 1., 0.5]);
        let original = source.read_region(bounds);
        for opacity in [f32::NAN, -0.1, 1.1] {
            let mut workspace = super::super::MAX_PIXEL_BYTES;
            let steps = vec![NativeStage::Knockout { below: 0, above: 0, opacity }];
            assert!(native_plan(&source, bounds, steps, 1, &mut workspace, &Interrupt::NONE).unwrap().is_err());
            assert_eq!(workspace, super::super::MAX_PIXEL_BYTES);
        }
        for allowance in [0, 1024] {
            let mut workspace = allowance;
            let steps = vec![NativeStage::Knockout { below: 0, above: 0, opacity: 0.5 }];
            assert!(native_plan(&source, bounds, steps, 1, &mut workspace, &Interrupt::NONE).is_err());
            assert_eq!(workspace, allowance);
        }
        for after in [0, 3] {
            let mut workspace = super::super::MAX_PIXEL_BYTES;
            let calls = AtomicUsize::new(0);
            let cancel = || calls.fetch_add(1, Ordering::Relaxed) >= after;
            let steps = vec![NativeStage::Knockout { below: 0, above: 0, opacity: 0.5 }];
            assert!(matches!(native_plan(&source, bounds, steps, 1, &mut workspace, &Interrupt::cancel_only(&cancel)), Err(IoError::Cancelled)));
            assert_eq!(workspace, super::super::MAX_PIXEL_BYTES);
        }
        assert_eq!(source.read_region(bounds), original);
    }

    #[test]
    fn native_stages_validate_flat_references_and_rollback_workspace_on_any_failure() {
        use effects::{NativeMapping, NativeStage, NativeStructure};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let bounds = Rect::new(0, 0, 3, 2);
        let mut source = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        source.write_region(bounds, &[0.1, 0.25, 0.4, 0.5, 0.75, 0.9]);
        let original = source.read_region(bounds);
        let plans = [
            (vec![NativeStage::Apply { input: 1, mapping: Box::new(NativeMapping::default()) }], 1),
            (vec![NativeStage::Join { below: 0, above: 1, blend: BlendMode::Normal, opacity: 1. }], 1),
            (vec![NativeStage::Join { below: 0, above: 0, blend: BlendMode::PassThrough, opacity: 1. }], 1),
            (vec![NativeStage::Join { below: 0, above: 0, blend: BlendMode::Normal, opacity: f32::NAN }], 1),
            (vec![NativeStage::Join { below: 0, above: 0, blend: BlendMode::Normal, opacity: 1.01 }], 1),
            (vec![], 1),
            (
                vec![NativeStage::Apply {
                    input: 0,
                    mapping: Box::new(NativeMapping { structure: Some(NativeStructure::Stages { steps: vec![], output: 0 }), ..Default::default() }),
                }],
                1,
            ),
        ];
        for (steps, output) in plans {
            let mut workspace = super::super::MAX_PIXEL_BYTES;
            assert!(native_plan(&source, bounds, steps, output, &mut workspace, &Interrupt::NONE).unwrap().is_err());
            assert_eq!(workspace, super::super::MAX_PIXEL_BYTES);
            assert_eq!(source.read_region(bounds), original);
        }
        for allowance in [0, 1024] {
            let mut workspace = allowance;
            let NativeStructure::Stages { steps, output } = sample_native_plan() else { panic!("native plan") };
            let result = native_plan(&source, bounds, steps, output, &mut workspace, &Interrupt::NONE);
            assert!(matches!(result, Err(_) | Ok(Err(_))));
            assert_eq!(workspace, allowance);
        }
        let mut workspace = super::super::MAX_PIXEL_BYTES;
        let steps = vec![
            NativeStage::Apply { input: 0, mapping: Box::new(NativeMapping::default()) },
            NativeStage::Apply {
                input: 1,
                mapping: Box::new(NativeMapping { filters: vec![effects::native_filter("not.a.native.command", serde_json::json!({}))], ..Default::default() }),
            },
        ];
        assert!(native_plan(&source, bounds, steps, 2, &mut workspace, &Interrupt::NONE).unwrap().is_err());
        assert_eq!(workspace, super::super::MAX_PIXEL_BYTES);
        for after in [0, 12] {
            let calls = AtomicUsize::new(0);
            let cancel = || calls.fetch_add(1, Ordering::Relaxed) >= after;
            let NativeStructure::Stages { steps, output } = sample_native_plan() else { panic!("native plan") };
            assert!(matches!(native_plan(&source, bounds, steps, output, &mut workspace, &Interrupt::cancel_only(&cancel)), Err(IoError::Cancelled)));
            assert_eq!(workspace, super::super::MAX_PIXEL_BYTES);
            assert_eq!(source.read_region(bounds), original);
        }
    }

    #[test]
    fn native_large_morphology_rejects_inflated_window_before_allocating_or_publishing() {
        let bounds = Rect::new(0, 0, 1, 1);
        let mut source = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        source.write_region(bounds, &[0.5]);
        let original = source.read_region(bounds);
        // A tiny source still needs the complete native1003x1003 inflated
        // read window for radius500. Samplingclamps; allocationdoesnotshrink.
        let params = photocraft_algo::FilterParams::Maximum { radius: 500., preserve: photocraft_algo::Preserve::Roundness };
        assert!(native_filter_workspace(&params, bounds, bounds, 1, 1_200_000).is_err());
        let effect = StoredEffect::test(
            "gimp:grow",
            vec![Argument { name: "radius-x", value: ArgumentValue::Int(500) }, Argument { name: "radius-y", value: ArgumentValue::Int(500) }],
        );
        let mut workspace = 1_200_000;
        let mut metadata = super::super::MAX_METADATA_BYTES;
        let result =
            apply(&source, bounds, &effect, None, false, &mut Budgets { workspace: &mut workspace, metadata: &mut metadata }, &Interrupt::NONE).unwrap();
        let failure = result.unwrap_err();
        assert!(failure.message.contains("temporary workspace"));
        assert_eq!((workspace, metadata), (1_200_000, super::super::MAX_METADATA_BYTES));
        assert_eq!(source.read_region(bounds), original);
        let skinny = Rect::new(0, 0, 8000, 1);
        let radial = photocraft_algo::FilterParams::RadialBlur {
            amount: 100.,
            method: photocraft_algo::RadialMethod::Spin,
            quality: photocraft_algo::RadialQuality::Best,
            center_x: 0.5,
            center_y: 0.5,
        };
        assert!(native_filter_workspace(&radial, skinny, skinny, 1, 20_000_000).is_err());
        let mut skinny_source = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        skinny_source.write_region(skinny, &vec![0.5; 8000]);
        let effect = StoredEffect::test("gegl:motion-blur-circular", vec![Argument { name: "angle", value: ArgumentValue::Float(100.) }]);
        let mut workspace = 40_000_000;
        let mut metadata = super::super::MAX_METADATA_BYTES;
        let failure =
            apply(&skinny_source, skinny, &effect, None, false, &mut Budgets { workspace: &mut workspace, metadata: &mut metadata }, &Interrupt::NONE)
                .unwrap()
                .unwrap_err();
        assert!(failure.message.contains("temporary workspace"));
        assert_eq!((workspace, metadata), (40_000_000, super::super::MAX_METADATA_BYTES));
        assert_eq!(skinny_source.pixel(4000, 0), vec![0.5]);
    }

    #[test]
    fn ca04_native_alpha_holes_hsb_and_matting_match_existing_algorithms_at_all_depths() {
        use photocraft_algo::{FilterParams as F, HsbModel};
        let bounds = Rect::new(0, 0, 5, 1);
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for alpha in [false, true] {
                let fmt = PixelFormat::new(ColorMode::Grayscale, depth, alpha);
                let mut source = Surface::new(fmt);
                let data: Vec<f32> = [0., 0.25, 0.5, 0.75, 1.].into_iter().flat_map(|a| if alpha { vec![0.5, a] } else { vec![a] }).collect();
                source.write_region(bounds, &data);
                let original = source.read_region(bounds);
                for (id, params, native) in [
                    ("filter.other.clampAlpha", serde_json::json!({"low":0.25,"high":0.75}), F::ClampAlpha { low: 0.25, high: 0.75 }),
                    ("filter.other.thresholdAlpha", serde_json::json!({"value":0.5}), F::ThresholdAlpha { value: 0.5 }),
                    (
                        "filter.other.colorToAlpha",
                        serde_json::json!({"color":[1.,1.,1.],"transparencyThreshold":0.,"opacityThreshold":1.}),
                        F::ColorToAlpha { color: [1.; 4], transparency_threshold: 0., opacity_threshold: 1. },
                    ),
                    ("filter.other.fillHoles", serde_json::json!({"connectivity":8,"channel":"each"}), F::FillHoles { connectivity: 8, alpha_only: false }),
                    ("filter.other.hsbHsl", serde_json::json!({"inputMode":"rgb","rowOrder":"hsb"}), F::HsbHsl { input: HsbModel::Rgb, output: HsbModel::Hsb }),
                ] {
                    let mut budget = super::super::MAX_PIXEL_BYTES;
                    let result = native_filters(&source, bounds, &[effects::native_filter(id, params)], &mut budget, &Interrupt::NONE).unwrap();
                    if !alpha && matches!(native, F::ClampAlpha { .. } | F::ThresholdAlpha { .. } | F::ColorToAlpha { .. }) {
                        assert!(result.unwrap_err().message.contains("without inventing alpha"));
                        assert_eq!(budget, super::super::MAX_PIXEL_BYTES);
                    } else {
                        let actual = result.unwrap();
                        let expected = photocraft_algo::apply_in_with(&source, &native, bounds, bounds, None, bounds, &Interrupt::NONE).unwrap();
                        assert_eq!(actual.read_region(bounds), expected.read_region(bounds), "{id} {depth:?} alpha={alpha}");
                    }
                    assert_eq!(source.read_region(bounds), original);
                }
                let mut budget = super::super::MAX_PIXEL_BYTES;
                let actual = native_filters(
                    &source,
                    bounds,
                    &[effects::native_filter("layer.matting.removeBlackMatte", serde_json::json!({}))],
                    &mut budget,
                    &Interrupt::NONE,
                )
                .unwrap()
                .unwrap();
                let expected = photocraft_algo::matting::remove_black_matte_with(&source, bounds, &Interrupt::NONE).unwrap();
                assert_eq!(actual.read_region(bounds), expected.read_region(bounds));
            }
        }
        for (id, params) in [
            ("filter.other.clampAlpha", serde_json::json!({"low":0.8,"high":0.2})),
            ("filter.other.thresholdAlpha", serde_json::json!({"value":2})),
            ("filter.other.fillHoles", serde_json::json!({"connectivity":7})),
            ("filter.other.hsbHsl", serde_json::json!({"inputMode":"unknown"})),
            ("layer.matting.removeBlackMatte", serde_json::json!({"file":"untrusted"})),
        ] {
            assert!(operation(id, &params).is_err());
        }
    }

    #[test]
    fn native_whole_owner_shadows_and_tiled_lens_preserve_existing_pixels_depth_and_alpha() {
        let bounds = Rect::new(0, 0, 5, 3);
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let fmt = PixelFormat::new(ColorMode::Grayscale, depth, true);
            let mut source = Surface::new(fmt);
            source.write_region(bounds, &(0..15).flat_map(|i| [0.1 + i as f32 / 20., 0.25 + i as f32 / 20.]).collect::<Vec<_>>());
            let shadows = photocraft_algo::tone::ShadowsHighlights {
                shadow_amount: 30.,
                shadow_radius: 1.,
                highlight_amount: 15.,
                highlight_radius: 2.,
                black_clip: 0.,
                white_clip: 0.,
                ..Default::default()
            };
            let mut budget = super::super::MAX_PIXEL_BYTES;
            let actual = native_shadows(&source, bounds, &shadows, &mut budget, &Interrupt::NONE).unwrap();
            let mut pixels: Vec<[f32; 4]> = source.read_region(bounds).as_chunks::<2>().0.iter().map(|p| [p[0], p[0], p[0], p[1]]).collect();
            photocraft_algo::tone::shadows_highlights(&mut pixels, 5, 3, &shadows);
            let mut expected = Surface::new(fmt);
            encode(&mut expected, &photocraft_compose::Buffer { rect: bounds, px: pixels });
            assert_eq!(actual.read_region(bounds), expected.read_region(bounds));
            let lens = photocraft_algo::lens::LensCorrection { vignette_amount: 20., distortion: 5., ..Default::default() };
            let actual = native_lens(&source, bounds, bounds, &lens, &mut budget, &Interrupt::NONE).unwrap();
            let expected = photocraft_algo::lens::correct(&source, bounds, &lens);
            assert_eq!(actual.read_region(bounds), expected.read_region(bounds));
            let cancel = || true;
            assert!(matches!(native_lens(&source, bounds, bounds, &lens, &mut budget, &Interrupt::cancel_only(&cancel)), Err(IoError::Cancelled)));
            assert!(native_shadows(&source, bounds, &shadows, &mut 1, &Interrupt::NONE).is_err());
        }
    }

    #[test]
    fn native_scalar_commit_clips_only_native_outputs_and_preserves_invalid_source_and_budgets() {
        let bounds = Rect::new(0, 0, 3, 1);
        let fmt = PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false);
        let mut source = Surface::new(fmt);
        source.write_region(bounds, &[0.125, 0.5, 0.875]);
        let effect = StoredEffect::test("gegl:exposure", vec![Argument { name: "exposure", value: ArgumentValue::Float(1.) }]);
        let mut budget = super::super::MAX_PIXEL_BYTES;
        let mut meta = super::super::MAX_METADATA_BYTES;
        let (actual, note) =
            apply(&source, bounds, &effect, None, false, &mut Budgets { workspace: &mut budget, metadata: &mut meta }, &Interrupt::NONE).unwrap().unwrap();
        assert_eq!(actual.read_region(bounds), vec![0.25, 1., 1.]);
        assert!(note.contains("Native scalar channel commit clips"));
        let original = source.read_region(bounds);
        source.write_region(bounds, &[0.125, f32::INFINITY, 0.875]);
        let mut budget = super::super::MAX_PIXEL_BYTES;
        let mut meta = super::super::MAX_METADATA_BYTES;
        assert!(apply(&source, bounds, &effect, None, false, &mut Budgets { workspace: &mut budget, metadata: &mut meta }, &Interrupt::NONE).is_err());
        assert_eq!((budget, meta), (super::super::MAX_PIXEL_BYTES, super::super::MAX_METADATA_BYTES));
        source.write_region(bounds, &original);
        let mut budget = 1;
        let mut meta = super::super::MAX_METADATA_BYTES;
        assert!(apply(&source, bounds, &effect, None, false, &mut Budgets { workspace: &mut budget, metadata: &mut meta }, &Interrupt::NONE).unwrap().is_err());
        assert_eq!((budget, meta), (1, super::super::MAX_METADATA_BYTES));
        assert_eq!(source.read_region(bounds), original);
    }

    #[test]
    fn scalar_rgb_working_stage_uses_native_joint_colors_masks_channels_selection_and_persistence() {
        use photocraft_doc::AlphaChannel;
        let bounds = Rect::new(-2, 3, 1, 5);
        let local = Rect::new(0, 0, 3, 2);
        let transform = photocraft_cms::Transform::new(
            photocraft_cms::Builtin::Srgb.profile(),
            photocraft_cms::Builtin::DotGain20Compat.profile(),
            photocraft_cms::Intent::RelativeColorimetric,
            true,
        )
        .unwrap();
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let fmt = PixelFormat::new(ColorMode::Grayscale, depth, false);
            let mut source = Surface::new(fmt);
            source.write_region(bounds, &[0., 0.25, 1., 0.5, 0.75, 0.125]);
            let original = source.read_region(bounds);
            let mut mask = Surface::new(fmt);
            mask.write_region(bounds, &[1., 0., 0.5, 0.25, 0.75, 1.]);
            let mask = LayerMask { surface: mask, density: 0.8, ..LayerMask::reveal_all() };
            let mut effect = StoredEffect::test("gegl:normal-map", vec![Argument { name: "scale", value: ArgumentValue::Float(0.) }]);
            effect.opacity = 0.4;
            let mut workspace = super::super::MAX_PIXEL_BYTES;
            let mut metadata = super::super::MAX_METADATA_BYTES;
            let (actual, note) =
                apply(&source, bounds, &effect, Some(&mask), false, &mut Budgets { workspace: &mut workspace, metadata: &mut metadata }, &Interrupt::NONE)
                    .unwrap()
                    .unwrap();
            assert!(note.contains("RGB-to-working-Gray"));
            assert_eq!(source.read_region(bounds), original);
            // Independently authored ordinary native flat forward-normal RGB
            // contents: zero source gain means X=.5,Y=.5,Z=1, not scalar .5.
            let mut rgb = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
            rgb.write_region(local, &[0.5, 0.5, 1., 1.].repeat(6));
            let mut gray = Surface::new(PixelFormat::new(ColorMode::Grayscale, depth, true));
            let mut projected = Vec::new();
            for pixel in rgb.read_region(local).as_chunks::<4>().0 {
                let pair = match depth {
                    SampleType::U8 => {
                        let input = pixel.iter().map(|v| (v * 255. + 0.5) as u8).collect::<Vec<_>>();
                        let mut out = [0u8; 2];
                        transform.convert_u8(&input, 4, &mut out, 2, true);
                        out.map(|v| f32::from(v) / 255.)
                    }
                    SampleType::U16 => {
                        let input = pixel.iter().map(|v| (v * 65535. + 0.5) as u16).collect::<Vec<_>>();
                        let mut out = [0u16; 2];
                        transform.convert_u16(&input, 4, &mut out, 2, true);
                        out.map(|v| f32::from(v) / 65535.)
                    }
                    SampleType::F32 => {
                        let mut out = [0.; 2];
                        transform.convert_f32(pixel, 4, &mut out, 2, true);
                        out
                    }
                };
                projected.extend(pair);
            }
            gray.write_region(local, &projected);
            let mut reference = Document::new("Independent native Gray parent", Size::new(3, 2), ColorMode::Grayscale, depth);
            reference.layers.push(Layer::new("Original native scalar", LayerContent::Raster(photocraft_algo::resample::translate_surface(&source, 2, -3))));
            let mut upper = Layer::new("Native RGB contents projected to Gray", LayerContent::Raster(gray));
            upper.opacity = 0.4;
            let mut native_mask = mask.clone();
            native_mask.surface = photocraft_algo::resample::translate_surface(&native_mask.surface, 2, -3);
            upper.mask = Some(native_mask);
            reference.layers.push(upper);
            let buffer = photocraft_compose::render(&reference, local);
            let mut expected = Surface::new(fmt);
            let mut native_buffer = buffer.clone();
            native_buffer.rect = bounds;
            encode(&mut expected, &native_buffer);
            assert_eq!(actual.read_region(bounds), expected.read_region(bounds));
            assert!(workspace < super::super::MAX_PIXEL_BYTES && metadata < super::super::MAX_METADATA_BYTES);
            let mut native = Document::new("Native converted coverage", Size::new(3, 2), ColorMode::Grayscale, depth);
            let mut layer = Layer::new("Editable native mask", LayerContent::Raster(source));
            layer.mask = Some(LayerMask { surface: actual.clone(), ..LayerMask::reveal_all() });
            native.layers.push(layer);
            native.channels.push(AlphaChannel::new("Native converted channel", actual.clone()));
            native.selection = Some(actual.clone());
            let bytes = photocraft_format::save_to_bytes(&native, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
            assert_eq!(reopened.layers[0].mask.as_ref().unwrap().surface.read_region(bounds), actual.read_region(bounds));
            assert_eq!(reopened.channels[0].surface.read_region(bounds), actual.read_region(bounds));
            assert_eq!(reopened.selection.unwrap().read_region(bounds), actual.read_region(bounds));
        }
    }

    #[test]
    fn scalar_rgb_working_stage_budget_cancel_and_invalid_coverage_are_atomic() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let bounds = Rect::new(0, 0, 3, 2);
        let mut source = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        source.write_region(bounds, &[0., 0.25, 1., 0.5, 0.75, 0.125]);
        let original = source.read_region(bounds);
        let effect = StoredEffect::test("gegl:normal-map", vec![Argument { name: "scale", value: ArgumentValue::Float(0.) }]);
        // Enough for source LUT metadata generation, insufficient for RGBA
        // tiled native stage storage: its private ledger must not be published.
        for (work, meta) in [(600_000, super::super::MAX_METADATA_BYTES), (super::super::MAX_PIXEL_BYTES, 1)] {
            let mut workspace = work;
            let mut metadata = meta;
            let result =
                apply(&source, bounds, &effect, None, false, &mut Budgets { workspace: &mut workspace, metadata: &mut metadata }, &Interrupt::NONE).unwrap();
            assert!(result.is_err());
            assert_eq!((workspace, metadata), (work, meta));
            assert_eq!(source.read_region(bounds), original);
        }
        let calls = AtomicUsize::new(0);
        let cancel = || calls.fetch_add(1, Ordering::Relaxed) > 16;
        let mut workspace = super::super::MAX_PIXEL_BYTES;
        let mut metadata = super::super::MAX_METADATA_BYTES;
        assert!(matches!(
            apply(&source, bounds, &effect, None, false, &mut Budgets { workspace: &mut workspace, metadata: &mut metadata }, &Interrupt::cancel_only(&cancel)),
            Err(IoError::Cancelled)
        ));
        assert_eq!((workspace, metadata), (super::super::MAX_PIXEL_BYTES, super::super::MAX_METADATA_BYTES));
        assert_eq!(source.read_region(bounds), original);
        let mut invalid_surface = source.clone();
        invalid_surface.write_region(bounds, &[0., 0., f32::NAN, 1., -0.1, 1.1]);
        assert!(validate_scalar_coverage(&invalid_surface, bounds, &Interrupt::NONE).is_err());
        let mut gray = super::super::MAX_PIXEL_BYTES;
        let mut rgb = gray;
        reserve_working_frame(bounds, &mut gray, ColorMode::Grayscale).unwrap();
        reserve_working_frame(bounds, &mut rgb, ColorMode::Rgb).unwrap();
        assert_eq!(gray - rgb, photocraft_geom::TILE_SIZE as usize * photocraft_geom::TILE_SIZE as usize * 8);
    }

    #[test]
    fn canonical_pixelize_coverage_matches_existing_native_mosaic_with_alpha_at_each_depth() {
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let bounds = Rect::new(0, 0, 4, 2);
            for alpha in [false, true] {
                let format = PixelFormat::new(ColorMode::Grayscale, sample, alpha);
                let mut source = Surface::new(format);
                let mut pixels = Vec::new();
                for value in [if alpha { -0.5 } else { 0. }, 0.25, if alpha { 1.5 } else { 1. }, 0.75, 0.1, 0.4, 0.8, 1.] {
                    pixels.push(value);
                    if alpha {
                        pixels.push(0.625);
                    }
                }
                source.write_region(bounds, &pixels);
                let mut budget = super::super::MAX_PIXEL_BYTES;
                let actual = native_filters(
                    &source,
                    bounds,
                    &[effects::native_filter("filter.pixelate.mosaic", serde_json::json!({"cellSize":2}))],
                    &mut budget,
                    &Interrupt::NONE,
                )
                .unwrap()
                .unwrap();
                let expected = photocraft_algo::apply_in_with(
                    &source,
                    &photocraft_algo::FilterParams::Mosaic { cell_size: 2. },
                    bounds,
                    bounds,
                    None,
                    bounds,
                    &Interrupt::NONE,
                )
                .unwrap();
                assert_eq!(actual.read_region(bounds), expected.read_region(bounds));
                if alpha {
                    for p in actual.read_region(bounds).chunks_exact(format.channels()) {
                        assert!((p[1] - source.pixel(0, 0)[1]).abs() < 1e-6);
                    }
                } else {
                    let mut workspace = super::super::MAX_PIXEL_BYTES;
                    let mut metadata = super::super::MAX_METADATA_BYTES;
                    let effect = StoredEffect::test(
                        "gegl:pixelize",
                        vec![Argument { name: "size-x", value: ArgumentValue::Int(2) }, Argument { name: "size-y", value: ArgumentValue::Int(2) }],
                    );
                    let (imported, _) =
                        apply(&source, bounds, &effect, None, false, &mut Budgets { workspace: &mut workspace, metadata: &mut metadata }, &Interrupt::NONE)
                            .unwrap()
                            .unwrap();
                    assert_eq!(imported.read_region(bounds), expected.read_region(bounds));
                }
            }
        }
        assert!(operation("filter.pixelate.mosaic", &serde_json::json!({"cellSize":201})).is_err());
    }

    #[test]
    fn coverage_average_uses_native_weighted_mean_and_component_envelope() {
        let bounds = Rect::new(-2, 3, 1, 4);
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for alpha in [false, true] {
                let fmt = PixelFormat::new(ColorMode::Grayscale, sample, alpha);
                let mut source = Surface::new(fmt);
                source.write_region(bounds, if alpha { &[0.0, 1.0, 0.5, 0.5, 1.0, 0.0] } else { &[0.0, 0.5, 1.0] });
                let mut filter = effects::native_filter("filter.blur.average", serde_json::json!({}));
                filter.opacity = 0.5;
                let mut budget = super::super::MAX_PIXEL_BYTES;
                let result = native_filters(&source, bounds, &[filter], &mut budget, &Interrupt::NONE).expect("coverage").expect("native Average");
                // The ordinary native alpha-weighted mean ignores the transparent
                // white pixel: (0*1 + .5*.5)/(1+.5), mean alpha=(1+.5+0)/3.
                // Read quantized source first, independently sum that native data.
                let pixels = source.read_region(bounds);
                let mut weighted = 0.0f64;
                let mut weight = 0.0f64;
                let mut alpha_sum = 0.0f64;
                for p in pixels.chunks_exact(fmt.channels()) {
                    let rgba = photocraft_raster::to_rgba(&fmt, p);
                    weighted += f64::from(rgba[0]) * f64::from(rgba[3]);
                    weight += f64::from(rgba[3]);
                    alpha_sum += f64::from(rgba[3]);
                }
                let mean = (weighted / weight) as f32;
                let mean_alpha = (alpha_sum / 3.0) as f32;
                let mut averaged = Surface::new(fmt);
                let mut raw = Vec::new();
                for _ in 0..3 {
                    let mut pixel = vec![0.; fmt.channels()];
                    photocraft_raster::from_rgba_into(&fmt, [mean, mean, mean, mean_alpha], &mut pixel);
                    raw.extend(pixel);
                }
                averaged.write_region(bounds, &raw);
                let independent = native_mix(
                    &source,
                    &averaged,
                    bounds,
                    super::super::native_effects::NativeEnvelope { opacity: 0.5, blend: BlendMode::Normal, visible: true, mask: None },
                    &Interrupt::NONE,
                )
                .expect("ordinary native mix");
                for (actual, expected) in result.read_region(bounds).iter().zip(independent.read_region(bounds)) {
                    // The independent scalar mean does not repeat the native
                    // neutral-RGB/Gray conversion's final F32 rounding.
                    let tolerance = if sample == SampleType::F32 { 1e-6 } else { 0.0 };
                    assert!((actual - expected).abs() <= tolerance, "{sample:?} alpha={alpha}: {actual} vs {expected}");
                }
                let mut budget = 0;
                assert!(
                    native_filters(&source, bounds, &[effects::native_filter("filter.blur.average", serde_json::json!({}))], &mut budget, &Interrupt::NONE)
                        .unwrap()
                        .is_err()
                );
                let mut budget = super::super::MAX_PIXEL_BYTES;
                assert!(matches!(
                    native_filters(
                        &source,
                        bounds,
                        &[effects::native_filter("filter.blur.average", serde_json::json!({}))],
                        &mut budget,
                        &Interrupt::cancel_only(&|| true)
                    ),
                    Err(IoError::Cancelled)
                ));
            }
        }
        assert!(operation("filter.blur.average", &serde_json::json!({"invented":1})).is_err());
    }

    #[test]
    fn coverage_exposure_matches_native_gray_adjustment_at_each_depth() {
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let fmt = PixelFormat::new(ColorMode::Grayscale, sample, false);
            let bounds = Rect::new(-2, 3, 1, 4);
            let mut source = Surface::new(fmt);
            source.write_region(bounds, &[0.125, 0.5, 0.875]);
            let effect = StoredEffect::test("gegl:exposure", vec![Argument { name: "exposure", value: ArgumentValue::Float(1.) }]);
            let mut budget = super::super::MAX_PIXEL_BYTES;
            let mut metadata = super::super::MAX_METADATA_BYTES;
            let (got, _) = apply(&source, bounds, &effect, None, false, &mut Budgets { workspace: &mut budget, metadata: &mut metadata }, &Interrupt::NONE)
                .unwrap()
                .unwrap();
            let mut expected = photocraft_compose::Buffer { rect: bounds, px: source.read_region(bounds).iter().map(|v| [*v, *v, *v, 1.]).collect() };
            photocraft_compose::adjust::apply_in(
                &Adjustment::Exposure { exposure: 1., offset: 0., gamma: 1. },
                &mut expected,
                photocraft_compose::adjust::Transfer::for_document(ColorMode::Grayscale, sample),
                if sample == SampleType::F32 { Some(sample) } else { None },
                ColorMode::Grayscale,
            );
            let mut reference = Surface::new(fmt);
            encode(&mut reference, &expected);
            let (reference, _) = native_scalar_commit(reference, bounds, &Interrupt::NONE).unwrap();
            for (a, b) in got.read_region(bounds).iter().zip(reference.read_region(bounds)) {
                assert!((a - b).abs() < 0.00001, "{sample:?}: {a} vs {b}");
            }
        }
    }

    #[test]
    fn unknown_native_adapter_leaves_source_unchanged() {
        let bounds = Rect::new(0, 0, 1, 1);
        let mut source = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        source.write_region(bounds, &[0.4]);
        let effect = StoredEffect::test("unknown:operation", Vec::new());
        let mut budget = super::super::MAX_PIXEL_BYTES;
        let mut metadata = super::super::MAX_METADATA_BYTES;
        assert!(
            apply(&source, bounds, &effect, None, true, &mut Budgets { workspace: &mut budget, metadata: &mut metadata }, &Interrupt::NONE).unwrap().is_err()
        );
        assert_eq!(source.read_region(bounds), vec![0.4]);
        assert!(matches!(
            apply(&source, bounds, &effect, None, true, &mut Budgets { workspace: &mut budget, metadata: &mut metadata }, &Interrupt::cancel_only(&|| true)),
            Err(IoError::Cancelled)
        ));
    }

    #[test]
    fn coverage_gaussian_uses_native_gray_filter_and_cancellation() {
        let bounds = Rect::new(0, 0, 3, 1);
        let fmt = PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false);
        let mut source = Surface::new(fmt);
        source.write_region(bounds, &[0., 1., 0.]);
        let effect = StoredEffect::test(
            "gegl:gaussian-blur",
            vec![Argument { name: "std-dev-x", value: ArgumentValue::Float(1.) }, Argument { name: "std-dev-y", value: ArgumentValue::Float(1.) }],
        );
        let mut budget = super::super::MAX_PIXEL_BYTES;
        let mut metadata = super::super::MAX_METADATA_BYTES;
        let (got, _) =
            apply(&source, bounds, &effect, None, true, &mut Budgets { workspace: &mut budget, metadata: &mut metadata }, &Interrupt::NONE).unwrap().unwrap();
        let reference = photocraft_algo::apply_in_with(
            &source,
            &photocraft_algo::FilterParams::GaussianBlur { radius: 1. },
            bounds,
            bounds,
            None,
            bounds,
            &Interrupt::NONE,
        )
        .unwrap();
        for (a, b) in got.read_region(bounds).iter().zip(reference.read_region(bounds)) {
            assert!((a - b).abs() < 0.00001);
        }
        assert!(matches!(
            apply(&source, bounds, &effect, None, true, &mut Budgets { workspace: &mut budget, metadata: &mut metadata }, &Interrupt::cancel_only(&|| true)),
            Err(IoError::Cancelled)
        ));
        let mut empty_budget = 0;
        assert!(
            apply(&source, bounds, &effect, None, true, &mut Budgets { workspace: &mut empty_budget, metadata: &mut metadata }, &Interrupt::NONE)
                .unwrap()
                .is_err()
        );
        assert_eq!(source.read_region(bounds), vec![0., 1., 0.]);
    }

    #[test]
    fn native_interval_curve_translates_coverage_threshold() {
        let bounds = Rect::new(0, 0, 3, 1);
        let mut source = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U16, false));
        source.write_region(bounds, &[0.2, 0.5, 0.9]);
        let effect = StoredEffect::test(
            "gimp:threshold",
            vec![Argument { name: "low", value: ArgumentValue::Float(0.3) }, Argument { name: "high", value: ArgumentValue::Float(0.7) }],
        );
        let mut budget = super::super::MAX_PIXEL_BYTES;
        let mut metadata = super::super::MAX_METADATA_BYTES;
        let (got, _) =
            apply(&source, bounds, &effect, None, false, &mut Budgets { workspace: &mut budget, metadata: &mut metadata }, &Interrupt::NONE).unwrap().unwrap();
        assert_eq!(got.read_region(bounds), vec![0., 1., 0.]);
    }
    fn independent_native_mix(input: &Surface, result: &Surface, opacity: f32, mask: Option<&Surface>) -> Surface {
        let mut doc = Document::new(
            "Independent native Gray mix",
            Size::new(input.content_bounds().width(), input.content_bounds().height()),
            ColorMode::Grayscale,
            input.format().sample,
        );
        doc.layers.push(Layer::new("Original", LayerContent::Raster(input.clone())));
        let mut layer = Layer::new("Processed", LayerContent::Raster(result.clone()));
        layer.opacity = opacity;
        layer.mask = mask.map(|surface| LayerMask { surface: surface.clone(), ..LayerMask::reveal_all() });
        doc.layers.push(layer);
        photocraft_compose::flatten_to_surface(&doc, input.format(), None)
    }

    #[test]
    fn component_filter_probability_and_parent_coverage_apply_once_with_native_gray_rules() {
        let bounds = Rect::new(0, 0, 5, 2);
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let fmt = PixelFormat::new(ColorMode::Grayscale, sample, false);
            let mut source = Surface::new(fmt);
            source.write_region(bounds, &[0., 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 1.]);
            let mut mask = LayerMask::hide_all();
            mask.surface.write_region(bounds, &[0., 0.25, 0.5, 0.75, 1., 1., 0.75, 0.5, 0.25, 0.]);
            let mut effect = StoredEffect::test(
                "gegl:noise-pick",
                vec![
                    Argument { name: "pct-random", value: ArgumentValue::Float(25.) },
                    Argument { name: "repeat", value: ArgumentValue::Int(2) },
                    Argument { name: "seed", value: ArgumentValue::UInt(7) },
                ],
            );
            effect.opacity = 0.5;
            effect.mode = 62;
            effect.mask_offset = 123;
            let mut budget = super::super::MAX_PIXEL_BYTES;
            let mut metadata = super::super::MAX_METADATA_BYTES;
            let actual =
                apply(&source, bounds, &effect, Some(&mask), false, &mut Budgets { workspace: &mut budget, metadata: &mut metadata }, &Interrupt::NONE)
                    .unwrap()
                    .unwrap()
                    .0;
            let mut expected = source.clone();
            for seed in [7, 8] {
                let filtered = photocraft_algo::apply_in_with(
                    &expected,
                    &photocraft_algo::FilterParams::Diffuse { mode: photocraft_algo::DiffuseMode::Normal, seed },
                    bounds,
                    bounds,
                    None,
                    bounds,
                    &Interrupt::NONE,
                )
                .unwrap();
                expected = independent_native_mix(&expected, &filtered, 0.25, None);
            }
            expected = independent_native_mix(&source, &expected, 0.5, Some(&mask.surface));
            assert_eq!(actual.read_region(bounds), expected.read_region(bounds), "{sample:?}");
        }
    }
    #[test]
    fn native_gray_bevel_is_composited_before_the_single_parent_mask_and_opacity() {
        use photocraft_doc::effects::{Bevel, BevelStyle, BevelTechnique, Contour, Effect, FxCommon};
        let bounds = Rect::new(0, 0, 5, 2);
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let fmt = PixelFormat::new(ColorMode::Grayscale, sample, false);
            let mut source = Surface::new(fmt);
            source.write_region(bounds, &[0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.]);
            let mut mask = LayerMask::hide_all();
            mask.surface.write_region(bounds, &[0., 0.25, 0.5, 0.75, 1., 1., 0.75, 0.5, 0.25, 0.]);
            let mut effect = StoredEffect::test("gegl:bevel", vec![]);
            effect.opacity = 0.5;
            effect.mask_offset = 123;
            let mut budget = super::super::MAX_PIXEL_BYTES;
            let mut metadata = super::super::MAX_METADATA_BYTES;
            let actual =
                apply(&source, bounds, &effect, Some(&mask), false, &mut Budgets { workspace: &mut budget, metadata: &mut metadata }, &Interrupt::NONE)
                    .unwrap()
                    .unwrap()
                    .0;
            let mut document = Document::new("Independent native bevel", Size::new(5, 2), ColorMode::Grayscale, sample);
            let mut layer = Layer::new("Native Gray layer", LayerContent::Raster(source.clone()));
            layer.effects.items.push(Effect::BevelEmboss(Bevel {
                enabled: true,
                style: BevelStyle::InnerBevel,
                technique: BevelTechnique::ChiselHard,
                depth: 0.4,
                up: true,
                size: 3.,
                soften: 0.,
                angle: 68.,
                altitude: 25.,
                use_global_light: false,
                gloss_contour: Contour::Linear,
                gloss_anti_alias: false,
                highlight: FxCommon::new(BlendMode::HardLight, 1.),
                highlight_color: photocraft_color::Color::gray(1.),
                shadow: FxCommon::new(BlendMode::Multiply, 1.),
                shadow_color: photocraft_color::Color::gray(0.),
                contour: None,
                texture: None,
            }));
            document.layers.push(layer);
            let styled = photocraft_compose::flatten_to_surface(&document, fmt, None);
            let expected = independent_native_mix(&source, &styled, 0.5, Some(&mask.surface));
            assert_eq!(actual.read_region(bounds), expected.read_region(bounds), "{sample:?}");
        }
    }
    #[test]
    fn native_quarter_turn_wind_preserves_gray_orientation_and_component_probability() {
        use photocraft_algo::transform::{Homography, Interp, warp_surface};
        let bounds = Rect::new(0, 0, 5, 2);
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let fmt = PixelFormat::new(ColorMode::Grayscale, sample, false);
            let mut source = Surface::new(fmt);
            source.write_region(bounds, &[0., 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 1.]);
            let effect = StoredEffect::test(
                "gegl:noise-slur",
                vec![Argument { name: "pct-random", value: ArgumentValue::Float(25.) }, Argument { name: "seed", value: ArgumentValue::UInt(7) }],
            );
            let mut budget = super::super::MAX_PIXEL_BYTES;
            let mut metadata = super::super::MAX_METADATA_BYTES;
            let actual = apply(&source, bounds, &effect, None, false, &mut Budgets { workspace: &mut budget, metadata: &mut metadata }, &Interrupt::NONE)
                .unwrap()
                .unwrap()
                .0;
            // Independent native Free Transform matrices: clockwise quarter turn,
            // ordinary native Wind with a25% component envelope, then inverse.
            let rotated_bounds = Rect::new(0, 0, 2, 5);
            let rotated = warp_surface(&source, bounds, &Homography([0., -1., 2., 1., 0., 0., 0., 0., 1.]), Interp::Bicubic).unwrap().try_convert(fmt).unwrap();
            let rotated = photocraft_algo::resample::crop_surface(&rotated, rotated_bounds);
            let wind = photocraft_algo::apply_in_with(
                &rotated,
                &photocraft_algo::FilterParams::Wind { method: photocraft_algo::WindMethod::Wind, from_right: true, seed: 7 },
                rotated_bounds,
                rotated_bounds,
                None,
                rotated_bounds,
                &Interrupt::NONE,
            )
            .unwrap();
            let mixed = independent_native_mix(&rotated, &wind, 0.25, None);
            let restored =
                warp_surface(&mixed, rotated_bounds, &Homography([0., 1., 0., -1., 0., 2., 0., 0., 1.]), Interp::Bicubic).unwrap().try_convert(fmt).unwrap();
            let expected = photocraft_algo::resample::crop_surface(&restored, bounds);
            assert_eq!(actual.read_region(bounds), expected.read_region(bounds), "{sample:?}");
        }
    }

    #[test]
    fn direct_native_pattern_replacement_keeps_resources_and_parent_mask_once() {
        use photocraft_doc::{Fill, Pattern};
        let bounds = Rect::new(-2, 3, 3, 5);
        let local = Rect::new(0, 0, 5, 2);
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let fmt = PixelFormat::new(ColorMode::Grayscale, sample, false);
            let mut source = Surface::new(fmt);
            source.write_region(bounds, &[0.2; 10]);
            let mut tile = Surface::new(PixelFormat::new(ColorMode::Grayscale, sample, true));
            tile.write_region(Rect::new(0, 0, 2, 1), &[0.1, 1., 0.9, 1.]);
            let pattern = Pattern::new("Independent alternating Gray", tile, 2, 1);
            let layer = Layer::new(
                "Native Pattern Fill",
                LayerContent::Fill(Fill::Pattern { name: pattern.name.clone(), id: pattern.id.clone(), scale: 1., angle: 0., link: false, phase: (0., 0.) }),
            );
            let mut reference = Document::new("Independent pattern", Size::new(5, 2), ColorMode::Grayscale, sample);
            reference.patterns.push(pattern.clone());
            reference.layers.push(layer.clone());
            let expected = photocraft_compose::flatten_to_surface(&reference, reference.pixel_format(), None);
            let mut document = Document::new("Materialized native coverage", Size::new(5, 2), ColorMode::Grayscale, sample);
            let mut budget = super::super::MAX_PIXEL_BYTES;
            let local_source = photocraft_algo::resample::translate_surface(&source, 2, -3);
            let actual = native_structure(
                &local_source,
                local,
                effects::NativeStructure::Replace { layer: Box::new(layer), patterns: vec![pattern] },
                &mut document,
                &mut budget,
                &Interrupt::NONE,
            )
            .unwrap()
            .unwrap();
            assert_eq!(actual.read_region(local), expected.read_region(local), "{sample:?}");
            assert!((actual.pixel(0, 0)[0] - actual.pixel(1, 0)[0]).abs() > 0.1, "native alternating pattern: {:?}", actual.read_region(local));
            assert!(budget < super::super::MAX_PIXEL_BYTES);
            let mut mask = Surface::new(fmt);
            mask.write_region(bounds, &[0., 0.25, 0.5, 0.75, 1., 1., 0.75, 0.5, 0.25, 0.]);
            let mask = LayerMask { surface: mask, ..LayerMask::reveal_all() };
            let actual = native_mix(
                &source,
                &photocraft_algo::resample::translate_surface(&actual, -2, 3),
                bounds,
                super::super::native_effects::NativeEnvelope { opacity: 0.5, blend: BlendMode::Normal, visible: true, mask: Some(mask.clone()) },
                &Interrupt::NONE,
            )
            .unwrap();
            let mut expected_document = Document::new("Independent complete stage", Size::new(5, 2), ColorMode::Grayscale, sample);
            expected_document.layers.push(Layer::new("Original", LayerContent::Raster(local_source)));
            let mut result = Layer::new("Pattern result", LayerContent::Raster(expected));
            result.opacity = 0.5;
            result.mask = Some(LayerMask { surface: photocraft_algo::resample::translate_surface(&mask.surface, 2, -3), ..mask });
            expected_document.layers.push(result);
            assert_eq!(actual.read_region(bounds), photocraft_compose::flatten_to_surface(&expected_document, fmt, None).read_region(local));
        }
    }

    #[test]
    fn native_composite_preserves_branch_clipping_vector_coverage_and_postfilter_order() {
        use photocraft_doc::{Color, Fill, VectorMask};
        use photocraft_geom::Affine;
        let bounds = Rect::new(0, 0, 5, 2);
        let fmt = PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false);
        let mut source = Surface::new(fmt);
        source.write_region(bounds, &[0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.]);
        let mut mask = Surface::new(fmt);
        mask.write_region(bounds, &[0., 0.25, 0.5, 0.75, 1., 0., 0.25, 0.5, 0.75, 1.]);
        let vector = VectorMask {
            path: photocraft_doc::Path { subpaths: vec![photocraft_doc::Subpath::polygon(&[(0., 0.), (4., 0.), (4., 2.), (0., 2.)])], ..Default::default() },
            enabled: true,
            linked: true,
            density: 1.,
            feather: 0.,
        };
        let mut document = Document::new("Coverage native branches", Size::new(5, 2), ColorMode::Grayscale, SampleType::F32);
        let mut budget = super::super::MAX_PIXEL_BYTES;
        let structure = effects::NativeStructure::Composite {
            branches: vec![
                effects::NativeBranch { fill: Some(Fill::Solid(Color::rgb(0.3, 0.3, 0.3))), ..Default::default() },
                effects::NativeBranch {
                    transform: Affine::translate(1., 0.),
                    filters: vec![effects::native_filter("image.adjustments.invert", serde_json::json!({}))],
                    opacity: 0.6,
                    blend: BlendMode::Multiply,
                    clipped: true,
                    mask: Some(LayerMask { surface: mask.clone(), ..LayerMask::reveal_all() }),
                    vector_mask: Some(vector.clone()),
                    ..Default::default()
                },
            ],
        };
        let actual = native_structure(&source, bounds, structure, &mut document, &mut budget, &Interrupt::NONE).unwrap().unwrap();
        // Ordinary native reference: independently compose source alpha, place,
        // invert, and use native clipping/raster+vector masks on the result.
        let mut reference = Document::new("Independent native branches", Size::new(5, 2), ColorMode::Grayscale, SampleType::F32);
        reference.layers.push(Layer::new("Underlay", LayerContent::Fill(Fill::Solid(Color::rgb(0.3, 0.3, 0.3)))));
        let mut input = Document::new("Native branch input", Size::new(5, 2), ColorMode::Grayscale, SampleType::F32);
        input.layers.push(Layer::new("Input", LayerContent::Raster(source.clone())));
        let alpha = photocraft_compose::flatten_to_surface(&input, input.pixel_format(), None);
        let mut placed = photocraft_algo::resample::translate_surface(&alpha, 1, 0);
        let raw = placed.read_region(bounds);
        let mut buffer = photocraft_compose::Buffer { rect: bounds, px: raw.as_chunks::<2>().0.iter().map(|p| [p[0], p[0], p[0], p[1]]).collect() };
        photocraft_compose::adjust::apply_in(
            &Adjustment::Invert,
            &mut buffer,
            photocraft_compose::adjust::Transfer::for_document(ColorMode::Grayscale, SampleType::F32),
            Some(SampleType::F32),
            ColorMode::Grayscale,
        );
        encode(&mut placed, &buffer);
        let mut layer = Layer::new("Placed/inverted native input", LayerContent::Raster(placed));
        layer.opacity = 0.6;
        layer.blend = BlendMode::Multiply;
        layer.clipped = true;
        layer.mask = Some(LayerMask { surface: mask, ..LayerMask::reveal_all() });
        layer.vector_mask = Some(vector);
        reference.layers.push(layer);
        assert_eq!(actual.read_region(bounds), photocraft_compose::flatten_to_surface(&reference, actual.format(), None).read_region(bounds));
        let post = vec![effects::native_filter("image.adjustments.exposure", serde_json::json!({"exposure":-1.}))];
        let processed = native_filters(&actual, bounds, &post, &mut budget, &Interrupt::NONE).unwrap().unwrap();
        assert!(processed.pixel(2, 0)[0] < actual.pixel(2, 0)[0]);
    }

    #[test]
    fn native_direct_group_shapes_prepare_native_caches_with_bounded_workspace() {
        use photocraft_doc::{Color, Fill, Group, Path, ShapeLayer, Subpath};
        let bounds = Rect::new(0, 0, 5, 2);
        let mut shape = ShapeLayer {
            path: Path { subpaths: vec![Subpath::polygon(&[(1., 0.), (4., 0.), (4., 2.), (1., 2.)])], ..Default::default() },
            fill: Some(Fill::Solid(Color::rgb(0.8, 0.8, 0.8))),
            ..Default::default()
        };
        let layer = Layer::new(
            "Direct native group",
            LayerContent::Group(Group { children: vec![Layer::new("Native outline", LayerContent::Shape(shape.clone()))], expanded: true, artboard: None }),
        );
        let mut document = Document::new("Native group coverage", Size::new(5, 2), ColorMode::Grayscale, SampleType::F32);
        let input = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        let mut budget = super::super::MAX_PIXEL_BYTES;
        let actual = native_structure(
            &input,
            bounds,
            effects::NativeStructure::Replace { layer: Box::new(layer.clone()), patterns: vec![] },
            &mut document,
            &mut budget,
            &Interrupt::NONE,
        )
        .unwrap()
        .unwrap();
        shape.cache = Some(photocraft_vector::render_shape(&shape, document.pixel_format(), bounds));
        let mut reference = Document::new("Independent native vector", Size::new(5, 2), ColorMode::Grayscale, SampleType::F32);
        reference.layers.push(Layer::new("Native outline", LayerContent::Shape(shape)));
        assert_eq!(actual.read_region(bounds), photocraft_compose::flatten_to_surface(&reference, actual.format(), None).read_region(bounds));
        assert!(actual.pixel(2, 0)[0] > actual.pixel(0, 0)[0]);
        let mut tiny = 1;
        let mut empty = Document::new("No budget", Size::new(5, 2), ColorMode::Grayscale, SampleType::F32);
        assert!(
            native_structure(
                &input,
                bounds,
                effects::NativeStructure::Replace { layer: Box::new(layer), patterns: vec![] },
                &mut empty,
                &mut tiny,
                &Interrupt::NONE
            )
            .is_err()
        );
        assert!(empty.layers.is_empty());
    }

    #[test]
    fn scalar_typed_rank_noise_and_convolution_match_existing_checked_native_algorithms() {
        let bounds = Rect::new(0, 0, 3, 3);
        let rank = photocraft_algo::rank::Params {
            radius: 1,
            color_percentile: 75.,
            alpha_percentile: 25.,
            shape: photocraft_algo::rank::Neighborhood::Diamond,
            channels: Some(vec![true]),
        };
        let noise = photocraft_algo::channel_noise::Params {
            amounts: vec![20.],
            distribution: photocraft_algo::Distribution::Uniform,
            monochromatic: false,
            relative: false,
            seed: 7,
        };
        let mut kernel = vec![0.; 25];
        kernel[7] = 0.25;
        kernel[12] = 0.5;
        kernel[17] = 0.25;
        let convolution = photocraft_algo::convolution::Params {
            kernel: kernel.clone(),
            scale: 1.,
            offset: 0.,
            channels: vec![true],
            alpha_weighted: false,
            edge: photocraft_algo::convolution::Edge::Repeat,
        };
        let filters = [
            effects::native_filter(
                "filter.other.percentile",
                serde_json::json!({"radius":1,"colorPercentile":75.,"alphaPercentile":25.,"shape":"diamond","channels":[true]}),
            ),
            effects::native_filter(
                "filter.noise.channelNoise",
                serde_json::json!({"amounts":[20.],"distribution":"uniform","monochromatic":false,"relative":false,"seed":7}),
            ),
            effects::native_filter(
                "filter.other.convolution",
                serde_json::json!({"kernel":kernel,"scale":1.,"offset":0.,"channels":[true],"alphaWeighted":false,"edge":"repeat"}),
            ),
        ];
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, sample, false));
            surface.write_region(bounds, &[0.1, 0.5, 0.9, 0.2, 0.6, 0.8, 0.3, 0.4, 0.7]);
            let expected = [
                photocraft_algo::rank::apply_with(&surface, &rank, bounds, bounds, None, &Interrupt::NONE).unwrap(),
                photocraft_algo::channel_noise::apply_with(&surface, &noise, bounds, bounds, None, &Interrupt::NONE).unwrap(),
                photocraft_algo::convolution::apply_with(&surface, &convolution, bounds, bounds, None, &Interrupt::NONE).unwrap(),
            ];
            for (filter, expected) in filters.iter().zip(expected) {
                let mut budget = super::super::MAX_PIXEL_BYTES;
                let actual = native_filters(&surface, bounds, std::slice::from_ref(filter), &mut budget, &Interrupt::NONE).unwrap().unwrap();
                assert_eq!(actual.read_region(bounds), expected.read_region(bounds), "{} {sample:?}", filter.command);
            }
            let mut workspace = super::super::MAX_PIXEL_BYTES;
            let mut metadata = super::super::MAX_METADATA_BYTES;
            let effect = StoredEffect::test(
                "gegl:median-blur",
                vec![
                    Argument { name: "radius", value: ArgumentValue::Int(1) },
                    Argument { name: "percentile", value: ArgumentValue::Float(75.) },
                    Argument { name: "alpha-percentile", value: ArgumentValue::Float(25.) },
                    Argument { name: "neighborhood", value: ArgumentValue::Enum(2) },
                ],
            );
            let (actual, _) =
                apply(&surface, bounds, &effect, None, false, &mut Budgets { workspace: &mut workspace, metadata: &mut metadata }, &Interrupt::NONE)
                    .unwrap()
                    .unwrap();
            let expected = photocraft_algo::rank::apply_with(&surface, &rank, bounds, bounds, None, &Interrupt::NONE).unwrap();
            for (a, b) in actual.read_region(bounds).iter().zip(expected.read_region(bounds)) {
                assert!((a - b).abs() < 1e-6, "{sample:?}: {a} vs {b}");
            }
        }
    }

    #[test]
    fn absent_disabled_alpha_levels_selection_preserves_scalar_signed_hdr_endpoints_and_native_alpha() {
        let bounds = Rect::new(-1, 2, 3, 3);
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for (lo, hi, outlo, outhi) in [(-1., 1., 0., 1.), (1., -1., 0., 1.), (-2., 2., -0.5, 1.5)] {
                let effect = StoredEffect::test(
                    "gegl:levels",
                    vec![
                        Argument { name: "in-low", value: ArgumentValue::Float(lo) },
                        Argument { name: "in-high", value: ArgumentValue::Float(hi) },
                        Argument { name: "out-low", value: ArgumentValue::Float(outlo) },
                        Argument { name: "out-high", value: ArgumentValue::Float(outhi) },
                    ],
                );
                let gain = f64::from(outhi - outlo) / f64::from(hi - lo);
                let offset = f64::from(outlo) - gain * f64::from(lo);
                let mut kernel = vec![0.; 25];
                kernel[12] = gain;
                for alpha in [false, true] {
                    let fmt = PixelFormat::new(ColorMode::Grayscale, depth, alpha);
                    let mut source = Surface::new(fmt);
                    let data: Vec<f32> = [0., 0.25, 0.5, 1.]
                        .into_iter()
                        .enumerate()
                        .flat_map(|(i, v)| {
                            let mut p = vec![v];
                            if alpha {
                                p.push((i + 1) as f32 / 4.);
                            }
                            p
                        })
                        .collect();
                    source.write_region(bounds, &data);
                    let original = source.read_region(bounds);
                    let filter = effects::native_filter(
                        "filter.other.convolution",
                        serde_json::json!({"kernel":kernel,"scale":1.,"offset":offset,"channels":[true,false],"alphaWeighted":false,"edge":"repeat"}),
                    );
                    let mut workspace = super::super::MAX_PIXEL_BYTES;
                    let actual = native_filters(&source, bounds, &[filter], &mut workspace, &Interrupt::NONE).unwrap().unwrap();
                    let params = photocraft_algo::convolution::Params {
                        kernel: kernel.clone(),
                        scale: 1.,
                        offset,
                        channels: if alpha { vec![true, false] } else { vec![true] },
                        alpha_weighted: false,
                        edge: photocraft_algo::convolution::Edge::Repeat,
                    };
                    let expected = photocraft_algo::convolution::apply_with(&source, &params, bounds, bounds, None, &Interrupt::NONE).unwrap();
                    assert_eq!(actual.read_region(bounds), expected.read_region(bounds));
                    let mut scalar = Surface::new(fmt);
                    let mut expected_values = Vec::new();
                    for pixel in original.chunks_exact(fmt.channels()) {
                        expected_values.push((f64::from(pixel[0]) * gain + offset) as f32);
                        if alpha {
                            expected_values.push(pixel[1]);
                        }
                    }
                    scalar.write_region(bounds, &expected_values);
                    assert_eq!(actual.read_region(bounds), scalar.read_region(bounds));
                    if !alpha {
                        let mut metadata = super::super::MAX_METADATA_BYTES;
                        let (converted, _) =
                            apply(&source, bounds, &effect, None, false, &mut Budgets { workspace: &mut workspace, metadata: &mut metadata }, &Interrupt::NONE)
                                .unwrap()
                                .unwrap();
                        assert_eq!(converted.read_region(bounds), expected.read_region(bounds));
                    }
                    assert_eq!(source.read_region(bounds), original);
                }
            }
        }
    }

    #[test]
    fn layer_alpha_channel_arrays_do_not_become_scalar_coverage_or_consume_shared_budget() {
        let bounds = Rect::new(0, 0, 3, 1);
        let mut source = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        source.write_region(bounds, &[0.2, 0.5, 0.8]);
        let original = source.read_region(bounds);
        let mut kernel = vec![0.; 25];
        kernel[12] = 1.;
        let filters = [
            effects::native_filter(
                "filter.other.percentile",
                serde_json::json!({"radius":1,"colorPercentile":50.,"alphaPercentile":50.,"shape":"disc","channels":[true,true]}),
            ),
            effects::native_filter(
                "filter.noise.channelNoise",
                serde_json::json!({"amounts":[20.,0.],"distribution":"uniform","monochromatic":false,"relative":false,"seed":0}),
            ),
            effects::native_filter(
                "filter.other.convolution",
                serde_json::json!({"kernel":kernel,"scale":1.,"offset":0.,"channels":[true,true],"alphaWeighted":false,"edge":"repeat"}),
            ),
        ];
        for filter in filters {
            let mut budget = 123456;
            let failure = native_filters(&source, bounds, &[filter], &mut budget, &Interrupt::NONE).unwrap().unwrap_err();
            assert_eq!(failure.reason, photocraft_doc::import_conversion::ConversionReason::MissingMapping);
            assert!(failure.message.contains("layer's alpha is not a standalone scalar"));
            assert_eq!(budget, 123456);
        }
        for operation in ["gegl:noise-rgb", "gegl:convolution-matrix", "gegl:value-propagate"] {
            let mut workspace = super::super::MAX_PIXEL_BYTES;
            let mut metadata = super::super::MAX_METADATA_BYTES;
            let failure = apply(
                &source,
                bounds,
                &StoredEffect::test(operation, vec![]),
                None,
                false,
                &mut Budgets { workspace: &mut workspace, metadata: &mut metadata },
                &Interrupt::NONE,
            )
            .unwrap()
            .unwrap_err();
            assert_eq!(failure.reason, photocraft_doc::import_conversion::ConversionReason::MissingMapping, "{operation}: {}", failure.message);
            assert!(failure.message.contains("actual surface has 1"), "{operation}: {}", failure.message);
            assert_eq!(workspace, super::super::MAX_PIXEL_BYTES);
            assert_eq!(metadata, super::super::MAX_METADATA_BYTES);
            assert_eq!(source.read_region(bounds), original);
        }
        for (command, params) in [
            ("filter.other.percentile", serde_json::json!({"radius":0,"colorPercentile":50,"alphaPercentile":50,"shape":"disc"})),
            ("filter.other.percentile", serde_json::json!({"radius":1,"colorPercentile":100.000001,"alphaPercentile":50,"shape":"disc"})),
            ("filter.noise.channelNoise", serde_json::json!({"amounts":vec![1;6]})),
            ("filter.noise.channelNoise", serde_json::json!({"amounts":[400.000001],"distribution":"uniform","monochromatic":false,"relative":false,"seed":0})),
            ("filter.other.convolution", serde_json::json!({"kernel":vec![0;26]})),
        ] {
            assert!(operation(command, &params).is_err());
        }
    }

    #[test]
    fn channel_mixer_and_set_alpha_use_existing_native_rules_and_one_component_envelope() {
        let bounds = Rect::new(0, 0, 3, 1);
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let format = PixelFormat::new(ColorMode::Grayscale, sample, true);
            let mut source = Surface::new(format);
            source.write_region(bounds, &[0.125, 0.5, 0.5, 0.75, 0.875, 1.]);
            let mut filter = effects::native_filter("image.adjustments.channelMixer", serde_json::json!({"monochrome":true,"gray":[25.,50.,25.,10.]}));
            filter.opacity = 0.5;
            let mut budget = super::super::MAX_PIXEL_BYTES;
            let actual = native_filters(&source, bounds, &[filter], &mut budget, &Interrupt::NONE).unwrap().unwrap();
            let mut buffer = photocraft_compose::Buffer {
                rect: bounds,
                px: source.read_region(bounds).as_chunks::<2>().0.iter().map(|p| [p[0], p[0], p[0], p[1]]).collect(),
            };
            photocraft_compose::adjust::apply_in(
                &Adjustment::ChannelMixer { matrix: [[0.25, 0.5, 0.25, 0.1], [0.; 4], [0.; 4]], monochrome: true },
                &mut buffer,
                photocraft_compose::adjust::Transfer::for_document(ColorMode::Grayscale, sample),
                if sample == SampleType::F32 { Some(sample) } else { None },
                ColorMode::Grayscale,
            );
            let mut processed = Surface::new(format);
            encode(&mut processed, &buffer);
            let mut reference = Document::new("Independent native mixer envelope", Size::new(3, 1), ColorMode::Grayscale, sample);
            reference.layers.push(Layer::new("Original", LayerContent::Raster(source.clone())));
            let mut layer = Layer::new("Native mixer", LayerContent::Raster(processed));
            layer.opacity = 0.5;
            reference.layers.push(layer);
            let expected = photocraft_compose::flatten_to_surface(&reference, format, None);
            assert_eq!(actual.read_region(bounds), expected.read_region(bounds));
            let actual = native_filters(
                &source,
                bounds,
                &[effects::native_filter("filter.other.setAlpha", serde_json::json!({"value":1.}))],
                &mut budget,
                &Interrupt::NONE,
            )
            .unwrap()
            .unwrap();
            for (p, original) in actual.read_region(bounds).as_chunks::<2>().0.iter().zip(source.read_region(bounds).as_chunks::<2>().0) {
                assert_eq!(p[0], original[0]);
                assert_eq!(p[1], 1.);
            }
        }
        for invalid in [serde_json::json!({"gray":[1,2,3]}), serde_json::json!({"red":[201,0,0,0]}), serde_json::json!({"monochrome":"yes"})] {
            assert!(adjustment("image.adjustments.channelMixer", &invalid).is_err());
        }
    }

    #[test]
    fn source_gray_newsprint_matches_independent_native_hard_mix_pattern_document() {
        use photocraft_doc::Pattern;
        use photocraft_doc::effects::{Effect, FxCommon};
        let bounds = Rect::new(0, 0, 8, 4);
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let format = PixelFormat::new(ColorMode::Grayscale, sample, false);
            let mut source = Surface::new(format);
            source.write_region(bounds, &(0..32).map(|i| i as f32 / 31.).collect::<Vec<_>>());
            let mut workspace = super::super::MAX_PIXEL_BYTES;
            let mut metadata = super::super::MAX_METADATA_BYTES;
            let effect = StoredEffect::test("gegl:newsprint", vec![Argument { name: "angle", value: ArgumentValue::Float(45.) }]);
            let (actual, _) =
                apply(&source, bounds, &effect, None, false, &mut Budgets { workspace: &mut workspace, metadata: &mut metadata }, &Interrupt::NONE)
                    .unwrap()
                    .unwrap();
            // Independent ordinary native line-screen resource: one linear
            // threshold ramp per period, not pixels obtained from the mapper.
            let mut tile = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
            for y in 0..64 {
                tile.write_region(Rect::new(0, y, 64, y + 1), &[(y as f32 + 0.5) / 64.; 64]);
            }
            let pattern = Pattern::new("Independent linear threshold", tile, 64, 64);
            let mut doc = Document::new("Independent native Gray screen", Size::new(8, 4), ColorMode::Grayscale, sample);
            // Independently authored result of constant native Curves, inside
            // the base content. An outer Color Overlay would repaint the
            // clipping group after its screen layers and is a different model.
            let mut black = Surface::new(PixelFormat::new(ColorMode::Grayscale, sample, true));
            black.write_region(bounds, &[0., 1.].repeat(32));
            doc.layers.push(Layer::new("Source-shaped constant native black", LayerContent::Raster(black)));
            let mut buffer = photocraft_compose::Buffer { rect: bounds, px: source.read_region(bounds).iter().map(|v| [*v, *v, *v, 1.]).collect() };
            photocraft_compose::adjust::apply_in(
                &Adjustment::ChannelMixer { matrix: [[0.2126, 0.7152, 0.0722, 0.], [0.; 4], [0.; 4]], monochrome: true },
                &mut buffer,
                photocraft_compose::adjust::Transfer::for_document(ColorMode::Grayscale, sample),
                if sample == SampleType::F32 { Some(sample) } else { None },
                ColorMode::Grayscale,
            );
            let mut monochrome = Surface::new(PixelFormat::new(ColorMode::Grayscale, sample, true));
            encode(&mut monochrome, &buffer);
            let mut screen = Layer::new("Opaque native threshold screen", LayerContent::Raster(monochrome));
            screen.clipped = true;
            screen.effects.items.push(Effect::PatternOverlay {
                common: FxCommon::new(BlendMode::HardMix, 1.),
                name: pattern.name.clone(),
                id: pattern.id.clone(),
                scale: 12. / 64.,
                angle: 45.,
                link: false,
                phase: (0., 0.),
            });
            doc.patterns.push(pattern);
            doc.layers.push(screen);
            let expected = photocraft_compose::flatten_to_surface(&doc, format, None);
            assert_eq!(actual.read_region(bounds), expected.read_region(bounds), "{sample:?}");
        }
    }

    #[test]
    fn arbitrary_screen_orientation_matches_native_alpha_transform_filter_inverse() {
        use photocraft_algo::transform::{Homography, Interp, warp_surface};
        let bounds = Rect::new(0, 0, 8, 4);
        let rotated = Rect::new(0, 0, 9, 9);
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let fmt = PixelFormat::new(ColorMode::Grayscale, sample, false);
            let mut source = Surface::new(fmt);
            source.write_region(bounds, &(0..32).map(|i| i as f32 / 31.).collect::<Vec<_>>());
            let mut budget = super::super::MAX_PIXEL_BYTES;
            let mut input = Document::new("Independent native alpha", Size::new(8, 4), ColorMode::Grayscale, sample);
            input.layers.push(Layer::new("Coverage", LayerContent::Raster(source.clone())));
            // Exercise the ordinary native orientation primitive independently
            // of source Newsprint, whose mapping now uses Pattern Overlay.
            let (placement, size) = super::super::native_effects::arbitrary_orientation(input.size, 45.).unwrap();
            assert_eq!(size, Size::new(9, 9));
            let actual_alpha = render_native(&input, bounds, input.pixel_format(), &Interrupt::NONE).unwrap();
            let placed_actual = native_warp(&actual_alpha, bounds, placement, rotated, &Interrupt::NONE).unwrap();
            let filters = [effects::native_filter("filter.gallery.halftonePattern", serde_json::json!({"size":5.25,"contrast":50.,"patternType":"line"}))];
            let screened_actual = native_filters(&placed_actual, rotated, &filters, &mut budget, &Interrupt::NONE).unwrap().unwrap();
            let restored_actual = native_warp(&screened_actual, rotated, placement.inverse().unwrap(), bounds, &Interrupt::NONE).unwrap();
            let actual = native_mix(
                &source,
                &restored_actual,
                bounds,
                super::super::native_effects::NativeEnvelope { opacity: 1., blend: BlendMode::Normal, visible: true, mask: None },
                &Interrupt::NONE,
            )
            .unwrap();
            let alpha = photocraft_compose::flatten_to_surface(&input, input.pixel_format(), None);
            let s = std::f64::consts::FRAC_1_SQRT_2;
            let placed = warp_surface(&alpha, bounds, &Homography([s, -s, 3., s, s, 0., 0., 0., 1.]), Interp::Bicubic)
                .unwrap()
                .try_convert(input.pixel_format())
                .unwrap();
            let placed = photocraft_algo::resample::crop_surface(&placed, rotated);
            let mut gallery = photocraft_algo::GalleryEffect::new(photocraft_algo::GalleryFilter::HalftonePattern);
            gallery.set("size", 5.25);
            gallery.set("contrast", 50.);
            assert!(gallery.set_choice("patternType", "line"));
            let screened = photocraft_algo::apply_in_with(
                &placed,
                &photocraft_algo::FilterParams::FilterGallery { effects: vec![gallery] },
                rotated,
                rotated,
                None,
                rotated,
                &Interrupt::NONE,
            )
            .unwrap();
            let restored = warp_surface(&screened, rotated, &Homography([s, s, -3. * s, -s, s, 3. * s, 0., 0., 1.]), Interp::Bicubic)
                .unwrap()
                .try_convert(input.pixel_format())
                .unwrap();
            let mut reference = Document::new("Independent native screen", Size::new(8, 4), ColorMode::Grayscale, sample);
            reference.layers.push(Layer::new("Unchanged native coverage", LayerContent::Raster(source)));
            reference.layers.push(Layer::new("Restored", LayerContent::Raster(photocraft_algo::resample::crop_surface(&restored, bounds))));
            let expected = photocraft_compose::flatten_to_surface(&reference, fmt, None);
            for (a, b) in actual.read_region(bounds).iter().zip(expected.read_region(bounds)) {
                assert!((a - b).abs() < 0.00001, "{sample:?}: {a} vs {b}");
            }
        }
    }

    #[test]
    fn source_bayer_coverage_uses_native_pattern_and_gray_cms_at_each_depth() {
        use photocraft_cms::{Builtin, Intent, Transform};
        use photocraft_doc::{Fill, Pattern};
        let bounds = Rect::new(-2, 3, 4, 7);
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let fmt = PixelFormat::new(ColorMode::Grayscale, sample, false);
            let mut source = Surface::new(fmt);
            source.write_region(bounds, &[0.4; 24]);
            let effect = StoredEffect::test("gegl:bayer-matrix", vec![]);
            let mut budget = super::super::MAX_PIXEL_BYTES;
            let mut metadata = super::super::MAX_METADATA_BYTES;
            let actual = apply(&source, bounds, &effect, None, false, &mut Budgets { workspace: &mut budget, metadata: &mut metadata }, &Interrupt::NONE)
                .unwrap()
                .unwrap()
                .0;
            // Independently authored2x2 native Pattern data: normalized Bayer
            // ranks, then standard source-sRGB to native SGray conversion.
            let mut rgba = Vec::new();
            for value in [0.125, 0.625, 0.875, 0.375] {
                rgba.extend_from_slice(&[value, value, value, 1.]);
            }
            let mut gray = [0.; 8];
            Transform::new(Builtin::Srgb.profile(), Builtin::SGray.profile(), Intent::RelativeColorimetric, false)
                .unwrap()
                .convert_f32(&rgba, 4, &mut gray, 2, true);
            let mut tile = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, true));
            tile.write_region(Rect::new(0, 0, 2, 2), &gray);
            let pattern = Pattern::new("Independent native Bayer", tile, 2, 2);
            let mut reference = Document::new("Independent native Gray Pattern", Size::new(6, 4), ColorMode::Grayscale, sample);
            reference.layers.push(Layer::new(
                "Native Pattern Fill",
                LayerContent::Fill(Fill::Pattern { name: pattern.name.clone(), id: pattern.id.clone(), scale: 1., angle: 0., link: true, phase: (0., 0.) }),
            ));
            reference.patterns.push(pattern);
            assert_eq!(actual.read_region(bounds), photocraft_compose::flatten_to_surface(&reference, fmt, None).read_region(reference.bounds()), "{sample:?}");
        }
    }

    #[test]
    fn native_projective_branch_uses_projective_placement_instead_of_affine_fallback() {
        use photocraft_algo::transform::{Homography, Interp, warp_surface};
        let bounds = Rect::new(0, 0, 5, 2);
        let fmt = PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false);
        let mut source = Surface::new(fmt);
        source.write_region(bounds, &[0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.]);
        let matrix = Homography([1., 0.1, 0., 0., 1., 0., 0.08, 0., 1.]);
        let mut budget = super::super::MAX_PIXEL_BYTES;
        let mut document = Document::new("Native projective coverage", Size::new(5, 2), ColorMode::Grayscale, SampleType::F32);
        let actual = native_structure(
            &source,
            bounds,
            effects::NativeStructure::Composite {
                branches: vec![effects::NativeBranch {
                    transform: photocraft_geom::Affine::translate(50., 0.),
                    perspective: Some(matrix.0),
                    ..Default::default()
                }],
            },
            &mut document,
            &mut budget,
            &Interrupt::NONE,
        )
        .unwrap()
        .unwrap();
        let mut reference = Document::new("Independent projective input", Size::new(5, 2), ColorMode::Grayscale, SampleType::F32);
        reference.layers.push(Layer::new("Input", LayerContent::Raster(source.clone())));
        let input = photocraft_compose::flatten_to_surface(&reference, reference.pixel_format(), None);
        let expected = warp_surface(&input, bounds, &matrix, Interp::Bicubic).unwrap().try_convert(reference.pixel_format()).unwrap();
        reference.layers = vec![Layer::new("Native projective result", LayerContent::Raster(expected))];
        assert_eq!(actual.read_region(bounds), photocraft_compose::flatten_to_surface(&reference, actual.format(), None).read_region(bounds));
        for matrix in [[f64::NAN; 9], [0.; 9], [1e9, 0., 0., 0., 1e9, 0., 0., 0., 1.]] {
            let mut empty = Document::new("Invalid projection", Size::new(5, 2), ColorMode::Grayscale, SampleType::F32);
            let result = native_structure(
                &source,
                bounds,
                effects::NativeStructure::Composite { branches: vec![effects::NativeBranch { perspective: Some(matrix), ..Default::default() }] },
                &mut empty,
                &mut budget,
                &Interrupt::NONE,
            );
            assert!(matches!(result, Ok(Err(_))));
            assert!(empty.layers.is_empty());
        }
        assert_eq!(source.pixel(0, 0), vec![0.1]);
    }

    #[test]
    fn source_automatic_tone_coverage_matches_independent_native_levels_at_each_depth() {
        for sample in SampleType::ALL {
            let fmt = PixelFormat::new(ColorMode::Grayscale, sample, false);
            let bounds = Rect::new(-2, 3, 2, 4);
            let mut source = Surface::new(fmt);
            source.write_region(bounds, &[0.2, 0.3, 0.4, 0.6]);
            for operation in ["gegl:color-enhance", "gegl:stretch-contrast"] {
                let effect = StoredEffect::test(operation, vec![]);
                let mut budget = super::super::MAX_PIXEL_BYTES;
                let mut metadata = super::super::MAX_METADATA_BYTES;
                let actual = apply(&source, bounds, &effect, None, false, &mut Budgets { workspace: &mut budget, metadata: &mut metadata }, &Interrupt::NONE)
                    .unwrap()
                    .unwrap()
                    .0;
                let mut buffer =
                    photocraft_compose::Buffer { rect: bounds, px: source.read_region(bounds).iter().map(|value| [*value, *value, *value, 1.]).collect() };
                let row = LevelsChannel { in_black: 51. / 255., in_white: 153. / 255., ..Default::default() };
                photocraft_compose::adjust::apply_in(
                    &Adjustment::Levels {
                        master: LevelsChannel::default(),
                        per_channel: std::array::from_fn(|_| row.clone()),
                        space: ToneSpace::Rgb,
                        black: LevelsChannel::default(),
                    },
                    &mut buffer,
                    photocraft_compose::adjust::Transfer::for_document(ColorMode::Grayscale, sample),
                    if sample == SampleType::F32 { Some(sample) } else { None },
                    ColorMode::Grayscale,
                );
                let mut expected = Surface::new(fmt);
                encode(&mut expected, &buffer);
                assert_eq!(actual.read_region(bounds), expected.read_region(bounds), "{sample:?}");
                assert!(actual.pixel(-2, 3)[0] < source.pixel(-2, 3)[0]);
                assert!(actual.pixel(1, 3)[0] > source.pixel(1, 3)[0]);
            }
        }
    }

    #[test]
    fn source_hdr_coverage_reuses_checked_native_local_adaptation_at_each_depth() {
        use photocraft_algo::tone::{HdrToning, checked_hdr};
        let bounds = Rect::new(0, 0, 4, 2);
        for sample in SampleType::ALL {
            let fmt = PixelFormat::new(ColorMode::Grayscale, sample, false);
            let mut source = Surface::new(fmt);
            source.write_region(bounds, &[0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.8, 1.]);
            let effect = StoredEffect::test("gegl:reinhard05", vec![]);
            let mut budget = super::super::MAX_PIXEL_BYTES;
            let mut metadata = super::super::MAX_METADATA_BYTES;
            let actual = apply(&source, bounds, &effect, None, false, &mut Budgets { workspace: &mut budget, metadata: &mut metadata }, &Interrupt::NONE)
                .unwrap()
                .unwrap()
                .0;
            let options = HdrToning { strength: 0.5, saturation: 0., ..Default::default() };
            let expected = checked_hdr::apply_with(&source, &options, bounds, bounds, None, &Interrupt::NONE).unwrap();
            let expected = independent_native_mix(&source, &expected, 1., None);
            assert_eq!(actual.read_region(bounds), expected.read_region(bounds), "{sample:?}");
        }
    }

    #[test]
    fn native_composite_carries_fill_opacity_and_advanced_knockout() {
        use photocraft_doc::{AdvancedBlending, Color, Fill, Knockout};
        let bounds = Rect::new(0, 0, 2, 1);
        let fmt = PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false);
        let source = Surface::new(fmt);
        let advanced = AdvancedBlending { knockout: Knockout::Deep, ..Default::default() };
        let fill = Fill::Solid(Color::rgb(0.2, 0.2, 0.2));
        let base = Fill::Solid(Color::rgb(0.8, 0.8, 0.8));
        let mut document = Document::new("Native knockout coverage", Size::new(2, 1), ColorMode::Grayscale, SampleType::F32);
        let mut budget = super::super::MAX_PIXEL_BYTES;
        let actual = native_structure(
            &source,
            bounds,
            effects::NativeStructure::Composite {
                branches: vec![
                    effects::NativeBranch { fill: Some(base.clone()), ..Default::default() },
                    effects::NativeBranch { fill: Some(fill.clone()), fill_opacity: 0., advanced, ..Default::default() },
                ],
            },
            &mut document,
            &mut budget,
            &Interrupt::NONE,
        )
        .unwrap()
        .unwrap();
        let mut reference = Document::new("Independent native knockout", Size::new(2, 1), ColorMode::Grayscale, SampleType::F32);
        reference.layers.push(Layer::new("Underlay", LayerContent::Fill(base)));
        let mut layer = Layer::new("Native knockout", LayerContent::Fill(fill));
        layer.fill_opacity = 0.;
        layer.advanced = advanced;
        reference.layers.push(layer);
        assert_eq!(actual.read_region(bounds), photocraft_compose::flatten_to_surface(&reference, actual.format(), None).read_region(bounds));
        assert!(actual.pixel(0, 0)[0] < 0.01);
    }

    #[test]
    fn source_alpha_curve_does_not_invent_alpha_for_a_scalar_coverage_plane() {
        let bounds = Rect::new(0, 0, 2, 1);
        let mut source = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        source.write_region(bounds, &[0.2, 0.8]);
        let config = effects::parse_config("(channel alpha) (curve (curve-type smooth) (points 4 0 1 1 0))", &mut (32 << 20)).unwrap();
        let effect = StoredEffect::test("gimp:curves", vec![Argument { name: "config", value: ArgumentValue::Config(config) }]);
        let mut budget = super::super::MAX_PIXEL_BYTES;
        let mut metadata = super::super::MAX_METADATA_BYTES;
        let failure = apply(&source, bounds, &effect, None, false, &mut Budgets { workspace: &mut budget, metadata: &mut metadata }, &Interrupt::NONE)
            .unwrap()
            .unwrap_err();
        assert!(failure.message.contains("alpha-bearing pixels"));
        assert_eq!(source.read_region(bounds), vec![0.2, 0.8]);
    }

    #[test]
    fn native_branch_post_style_runs_after_inner_fill_and_style_before_outer_coverage() {
        use photocraft_doc::{
            Color,
            effects::{Effect, FxCommon},
        };
        let bounds = Rect::new(0, 0, 2, 1);
        let fmt = PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false);
        let mut source = Surface::new(fmt);
        source.write_region(bounds, &[0.2, 0.6]);
        let inner = Effect::ColorOverlay { common: FxCommon { enabled: true, blend: BlendMode::Normal, opacity: 0.5 }, color: Color::rgb(0.9, 0.9, 0.9) };
        let post = Effect::ColorOverlay { common: FxCommon { enabled: true, blend: BlendMode::Normal, opacity: 0.7 }, color: Color::rgb(0.3, 0.3, 0.3) };
        let mut mask = Surface::new(fmt);
        mask.write_region(bounds, &[0.25, 0.75]);
        let mask = LayerMask { surface: mask, ..LayerMask::reveal_all() };
        let mut document = Document::new("Native post-style coverage", Size::new(2, 1), ColorMode::Grayscale, SampleType::F32);
        let mut budget = super::super::MAX_PIXEL_BYTES;
        let actual = native_structure(
            &source,
            bounds,
            effects::NativeStructure::Composite {
                branches: vec![effects::NativeBranch {
                    fill_opacity: 0.25,
                    opacity: 0.6,
                    mask: Some(mask.clone()),
                    layer_effects: vec![inner.clone()],
                    post_layer_effects: vec![post.clone()],
                    ..Default::default()
                }],
            },
            &mut document,
            &mut budget,
            &Interrupt::NONE,
        )
        .unwrap()
        .unwrap();
        let mut input = Document::new("Independent native alpha", Size::new(2, 1), ColorMode::Grayscale, SampleType::F32);
        input.layers.push(Layer::new("Input", LayerContent::Raster(source)));
        let alpha = photocraft_compose::flatten_to_surface(&input, input.pixel_format(), None);
        let mut inner_document = Document::new("Independent inner native styles", Size::new(2, 1), ColorMode::Grayscale, SampleType::F32);
        let mut inner_layer = Layer::new("Native inner branch", LayerContent::Raster(alpha));
        inner_layer.fill_opacity = 0.25;
        inner_layer.effects.items = vec![inner];
        inner_document.layers.push(inner_layer);
        let rendered = photocraft_compose::flatten_to_surface(&inner_document, inner_document.pixel_format(), None);
        let mut reference = Document::new("Independent outer native styles", Size::new(2, 1), ColorMode::Grayscale, SampleType::F32);
        let mut outer_layer = Layer::new("Native outer branch", LayerContent::Raster(rendered));
        outer_layer.effects.items = vec![post];
        outer_layer.opacity = 0.6;
        outer_layer.mask = Some(mask);
        reference.layers.push(outer_layer);
        assert_eq!(actual.read_region(bounds), photocraft_compose::flatten_to_surface(&reference, actual.format(), None).read_region(bounds));
        assert!(budget < super::super::MAX_PIXEL_BYTES);
    }

    #[test]
    fn native_filter_frame_precedes_filters_and_restores_outer_placement_and_mask_once() {
        use photocraft_algo::transform::{Homography, Interp, warp_surface};
        use photocraft_geom::Affine;
        let bounds = Rect::new(0, 0, 5, 2);
        let frame = Rect::new(-2, -1, 7, 3);
        let local_frame = Rect::new(0, 0, 9, 4);
        let fmt = PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false);
        let mut source = Surface::new(fmt);
        source.write_region(bounds, &[0., 0.2, 0.4, 0.6, 0.8, 1., 0.7, 0.5, 0.3, 0.1]);
        let mut mask = Surface::new(fmt);
        mask.write_region(bounds, &[0., 0.25, 0.5, 0.75, 1., 1., 0.75, 0.5, 0.25, 0.]);
        let mask = LayerMask { surface: mask, ..LayerMask::reveal_all() };
        let branch = effects::NativeBranch {
            filter_frame: Some(frame),
            transform: Affine::translate(1., 0.),
            opacity: 0.6,
            mask: Some(mask.clone()),
            filters: vec![
                effects::native_filter("filter.blur.gaussianBlur", serde_json::json!({"radius":1.})),
                effects::native_filter("image.adjustments.invert", serde_json::json!({})),
            ],
            ..Default::default()
        };
        let mut document = Document::new("Native framed coverage", Size::new(5, 2), ColorMode::Grayscale, SampleType::F32);
        let mut budget = super::super::MAX_PIXEL_BYTES;
        let actual =
            native_structure(&source, bounds, effects::NativeStructure::Composite { branches: vec![branch] }, &mut document, &mut budget, &Interrupt::NONE)
                .unwrap()
                .unwrap();
        let mut input = Document::new("Independent native source", Size::new(5, 2), ColorMode::Grayscale, SampleType::F32);
        input.layers.push(Layer::new("Original", LayerContent::Raster(source.clone())));
        let alpha = photocraft_compose::flatten_to_surface(&input, input.pixel_format(), None);
        let placed = photocraft_algo::resample::translate_surface(&alpha, 2, 1);
        let mut filtered = photocraft_algo::apply_in_with(
            &placed,
            &photocraft_algo::FilterParams::GaussianBlur { radius: 1. },
            local_frame,
            local_frame,
            None,
            local_frame,
            &Interrupt::NONE,
        )
        .unwrap();
        let raw = filtered.read_region(local_frame);
        let mut buffer =
            photocraft_compose::Buffer { rect: local_frame, px: raw.as_chunks::<2>().0.iter().map(|pixel| [pixel[0], pixel[0], pixel[0], pixel[1]]).collect() };
        photocraft_compose::adjust::apply_in(
            &Adjustment::Invert,
            &mut buffer,
            photocraft_compose::adjust::Transfer::Gamma(1.),
            Some(SampleType::F32),
            ColorMode::Grayscale,
        );
        encode(&mut filtered, &buffer);
        let mut framed = Document::new("Independent native filter parent", Size::new(9, 4), ColorMode::Grayscale, SampleType::F32);
        framed.layers.push(Layer::new("Filtered", LayerContent::Raster(filtered)));
        let framed_pixels = photocraft_compose::flatten_to_surface(&framed, framed.pixel_format(), None);
        let restored = warp_surface(&framed_pixels, local_frame, &Homography([1., 0., -1., 0., 1., -1., 0., 0., 1.]), Interp::Bicubic)
            .unwrap()
            .try_convert(input.pixel_format())
            .unwrap();
        let mut reference = Document::new("Independent native outer placement", Size::new(5, 2), ColorMode::Grayscale, SampleType::F32);
        let mut layer = Layer::new("Native restored branch", LayerContent::Raster(restored));
        layer.opacity = 0.6;
        layer.mask = Some(mask);
        reference.layers.push(layer);
        assert_eq!(actual.read_region(bounds), photocraft_compose::flatten_to_surface(&reference, actual.format(), None).read_region(bounds));
        let invalid = effects::NativeBranch { filter_frame: Some(frame), perspective: Some(Homography::IDENTITY.0), ..Default::default() };
        let mut empty = Document::new("Unsupported framed perspective", Size::new(5, 2), ColorMode::Grayscale, SampleType::F32);
        let failure =
            native_structure(&source, bounds, effects::NativeStructure::Composite { branches: vec![invalid] }, &mut empty, &mut budget, &Interrupt::NONE)
                .unwrap()
                .unwrap_err();
        assert!(failure.message.contains("framed perspective"));
        assert!(empty.layers.is_empty());
    }
}
