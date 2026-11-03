//! Shared native-conversion helpers; mapping registration follows separately.

#[allow(unused_imports)]
use super::{IoError, Result, effects, invalid};

#[allow(unused_imports)]
use photocraft_color::{BlendMode, ColorMode, SampleType};

#[allow(unused_imports)]
use photocraft_doc::adjust::{CurvePoint, LevelsChannel, ToneSpace};

#[allow(unused_imports)]
use photocraft_doc::{Adjustment, Document, Layer, LayerContent, LayerMask};

#[allow(unused_imports)]
use photocraft_geom::{Rect, Size};

#[allow(unused_imports)]
use photocraft_raster::{Interrupt, Surface};

#[allow(unused_imports)]
use serde_json::Value;

/// Read a native numeric parameter with its default, rejecting nonfinite values and values outside the command domain.
fn number(p: &Value, name: &str, default: f32, lo: f32, hi: f32) -> std::result::Result<f32, String> {
    let value = match p.get(name) {
        None => default,
        Some(v) => v.as_f64().ok_or_else(|| format!("native {name} is not numeric"))? as f32,
    };
    if value.is_finite() && (lo..=hi).contains(&value) { Ok(value) } else { Err(format!("native {name} is outside its supported range")) }
}

/// Decode the admitted native adjustment commands, including embedded-only lookup tables.
/// Unknown commands or invalid native parameters return a mapping error rather than running source operations.
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
            // These embedded tables are generated native conversion data, not an original LUT file to re-export.
            Adjustment::ColorLookup {
                name: lookup.title,
                size: lookup.size as u32,
                lut: Some(std::sync::Arc::new(lookup.data)),
                tetrahedral,
                dither,
                source: None,
            }
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

/// Require four finite native Channel Mixer percentages and convert them into normalized coefficients.
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

/// Decode one native Levels channel and enforce its minimum input interval before applying it to coverage.
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

/// Validate bounded native curve points or the complete pencil table, rejecting unordered inputs and invalid coordinates.
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

/// Select an existing native algorithm or adjustment from its command parameters.
/// Unimplemented adapters fail explicitly so the caller can retain the original coverage.
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

/// Decode the bounded native channel-selection array, allowing an empty selection only where the caller permits it.
fn native_channels(value: &Value, empty: bool) -> std::result::Result<Vec<bool>, String> {
    let input = value.as_array().filter(|v| v.len() <= 5 && (empty || !v.is_empty())).ok_or("Native channel selection has an invalid length")?;
    serde_json::from_value(Value::Array(input.clone())).map_err(|e| format!("Native channel selection: {e}"))
}

/// Reject alpha-dependent or channel-count mismatched operations for the actual surface.
/// A scalar coverage plane never acquires invented layer alpha.
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

/// Bound the live decoded displacement-map allowance against remaining workspace, including overflow checks.
fn embedded_map_workspace(filters: &[photocraft_doc::SmartFilter], budget: usize) -> std::result::Result<usize, effects::MappingFailure> {
    let cost = filters.iter().try_fold(0usize, |sum, f| {
        let bytes = f.params.get("__displacePsd").and_then(Value::as_str).map_or(Some(0), |text| text.len().checked_mul(4).and_then(|n| n.checked_add(4096)));
        bytes.and_then(|n| sum.checked_add(n))
    });
    cost.filter(|n| *n <= budget).ok_or_else(|| effects::MappingFailure::resource("embedded native coordinate maps exceed actual remaining coverage workspace"))
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

/// Run checked native algorithms with admitted frame and map workspace.
/// Raw snapshots omit component envelopes; coverage replay applies each admitted native component envelope.
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

/// Reject overflowed or oversized temporary allocations before invoking a native whole-frame algorithm.
fn temporary_workspace(bytes: Option<u64>, budget: usize) -> Result<()> {
    if bytes.is_none_or(|n| n > budget as u64) {
        return Err(invalid("native coverage temporary workspace exceeds shared allowance"));
    }
    Ok(())
}

/// Apply existing native Shadows/Highlights after bounding its whole-owner scratch space.
/// Cancellation and nonfinite output prevent publication after the kernel returns.
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

/// Apply ordinary native lens sampling in bounded output blocks, checking cancellation and finite output between blocks.
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

/// Accept an optional native u32 seed with deterministic zero default; reject other types or oversized integers.
fn seed(p: &Value) -> std::result::Result<u32, String> {
    match p.get("seed") {
        None => Ok(0),
        Some(value) => value.as_u64().and_then(|n| u32::try_from(n).ok()).ok_or_else(|| "Native coverage seed is not a u32".into()),
    }
}

/// Build an ordinary two-layer native document to apply visibility, blend, opacity and mask once to processed coverage.
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

/// Compose bounded native bands and restore the caller frame coordinates while observing cancellation between bands.
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

/// Require positive dimensions within the importer side limit before native snapshot allocation or geometry work.
fn validate_native_frame(bounds: Rect) -> Result<()> {
    let width = i64::from(bounds.x1) - i64::from(bounds.x0);
    let height = i64::from(bounds.y1) - i64::from(bounds.y0);
    if width < 1 || height < 1 || width > i64::from(super::MAX_SIDE) || height > i64::from(super::MAX_SIDE) {
        return Err(invalid("native snapshot frame dimensions exceed bounded native extent"));
    }
    Ok(())
}

/// Validate the native canvas and a finite positive DPI before using document-dependent preparation.
fn validate_native_document(document: &Document) -> Result<()> {
    validate_native_frame(document.bounds())?;
    if !document.resolution_dpi.is_finite() || document.resolution_dpi <= 0. {
        return Err(invalid("native snapshot document DPI is invalid"));
    }
    Ok(())
}

/// Bound traversal and reject visible deferred content without prepared caches, preventing silent empty Smart, Text or Shape output.
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

/// Convert a composed RGBA band into the destination native pixel format before writing its original rectangle.
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

/// Admit scalar working storage and the extra padded RGB tile cost atomically against the shared allowance.
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

/// Validate bounded native typography, lay it out at the actual document DPI, and publish its rendered cache only after budget and cancellation checks.
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

/// Prepare native Text and Shape caches recursively with bounded depth, nodes and workspace.
/// Deferred Smart content and unsupported pattern preparation are rejected rather than flattened as empty.
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
