//! Resource effects become ordinary native styles and parameter color lookups.
//! No source resource name, path or operation is an executable or filesystem capability.

use super::{ArgumentValue, ColorLiteral, Config, ConfigExpr, MappingContext, MappingFailure, MappingResult, NativeMapping, StoredEffect, native_filter};
use photocraft_cms::{Builtin, ColorSpace, Curve, Intent, Profile, Transform};
use serde_json::json;

/// Decode only portable byte encodings. Native-endian source float blobs have no
/// byte-order marker; guessing from the importing machine would corrupt colors.
pub(in crate::xcf) fn literal_rgb(value: &ColorLiteral<'_>) -> Result<[f32; 4], String> {
    // IEEE positive zero has identical bytes in either byte order. This exact
    // transparent default is portable without guessing the author's host.
    if matches!(value.encoding, "RGBA float" | "R'G'B'A float" | "R~G~B~A float")
        && value.profile.is_empty()
        && value.pixels.len() == 16
        && value.pixels.iter().all(|byte| *byte == 0)
    {
        return Ok([0.0; 4]);
    }
    let (channels, alpha, linear, gray) = match value.encoding {
        "R'G'B'A u8" | "R~G~B~A u8" => (3, true, false, false),
        "R'G'B' u8" | "R~G~B~ u8" => (3, false, false, false),
        "RGBA u8" => (3, true, true, false),
        "RGB u8" => (3, false, true, false),
        "Y'A u8" | "Y~A u8" => (1, true, false, true),
        "Y' u8" | "Y~ u8" => (1, false, false, true),
        "YA u8" => (1, true, true, true),
        "Y u8" => (1, false, true, true),
        _ => {
            return Err(format!(
                "stored color encoding {} is not independently portable; native-endian float colors need source byte-order evidence",
                value.encoding
            ));
        }
    };
    if value.pixels.len() != channels + usize::from(alpha) {
        return Err("stored color byte count does not match its encoding".into());
    }
    let fallback = if gray {
        Builtin::SGray
    } else if linear {
        Builtin::LinearSrgb
    } else {
        Builtin::Srgb
    };
    let mut source = if value.profile.is_empty() {
        fallback.profile().clone()
    } else {
        Profile::parse(value.profile).map_err(|error| format!("stored literal ICC: {error}"))?
    };
    let expected = if gray { ColorSpace::Gray } else { ColorSpace::Rgb };
    if source.color_space != expected {
        return Err("stored literal profile model does not match its encoding".into());
    }
    // Encoding TRCs are independent of assigned colorants, exactly as for source pixels.
    if linear || value.encoding.contains('~') {
        if !source.is_matrix_shaper() || source.b2a.iter().any(Option::is_some) {
            return Err("stored literal ICC has no separable native encoding representation".into());
        }
        let curve = if linear { Curve::Identity } else { photocraft_cms::curve::srgb_trc() };
        if gray {
            source.gray_trc = Some(curve);
        } else {
            source.trc = Some([curve.clone(), curve.clone(), curve]);
        }
    }
    let transform = Transform::new(&source, Builtin::Srgb.profile(), Intent::RelativeColorimetric, false)
        .map_err(|error| format!("stored literal native color conversion: {error}"))?;
    let samples: Vec<_> = value.pixels.iter().take(channels).map(|v| f32::from(*v) / 255.0).collect();
    let mut rgb = [0.0; 3];
    transform.convert_f32(&samples, channels, &mut rgb, 3, false);
    if rgb.iter().any(|v| !v.is_finite()) {
        return Err("stored literal conversion is nonfinite".into());
    }
    let [r, g, b] = rgb;
    let a = if alpha { f32::from(*value.pixels.get(channels).ok_or("stored alpha is missing")?) / 255.0 } else { 1.0 };
    Ok([r, g, b, a])
}

pub(super) fn color(effect: &StoredEffect<'_>, name: &str) -> Result<[f32; 4], String> {
    match effect.argument(name)? {
        Some(ArgumentValue::Color(Some(value))) => literal_rgb(value),
        Some(ArgumentValue::LiteralColor { rgba, linear }) => {
            if rgba.iter().any(|value| !value.is_finite()) || !(0.0..=1.0).contains(&rgba[3]) {
                return Err(format!("source color {name} has nonfinite components or invalid alpha"));
            }
            if !linear {
                return Ok(*rgba);
            }
            // The graph parser has already decoded portable numeric components; these
            // are linear-light sRGB, not host-endian bytes or CSS integer channels.
            let transform = Transform::new(Builtin::LinearSrgb.profile(), Builtin::Srgb.profile(), Intent::RelativeColorimetric, false)
                .map_err(|error| format!("literal native color conversion: {error}"))?;
            let mut rgb = [0.; 3];
            transform.convert_f32(&rgba[..3], 3, &mut rgb, 3, false);
            if rgb.iter().any(|value| !value.is_finite()) {
                return Err("literal native color conversion is nonfinite".into());
            }
            Ok([rgb[0], rgb[1], rgb[2], rgba[3]])
        }
        Some(ArgumentValue::Color(None)) | None => Err(format!("resource unavailable: source color {name} is absent")),
        _ => Err(format!("source color {name} has the wrong stored type")),
    }
}

fn number(effect: &StoredEffect<'_>, name: &str, default: f64) -> Result<f64, String> {
    let value = effect.number(name)?.unwrap_or(default);
    if value.is_finite() { Ok(value) } else { Err(format!("nonfinite resource effect argument {name}")) }
}

/// A fixed, small native table sampled from explicit parameter conversion, never
/// source-image pixels or a source renderer. Red varies fastest, as native CUBE expects.
pub(super) fn lookup(size: u32, label: &str, notes: Vec<String>, sample: impl Fn([f64; 3]) -> [f64; 3]) -> Result<NativeMapping, String> {
    use std::fmt::Write;
    if !(2..=33).contains(&size) {
        return Err("native parameter lookup size is outside 2..33".into());
    }
    if label.len() > 128 || label.chars().any(|c| c.is_control() || matches!(c, '"' | '\\')) {
        return Err("native lookup title is not plain bounded text".into());
    }
    let mut data = format!("TITLE \"{label}\"\nLUT_3D_SIZE {size}\n");
    let denominator = f64::from(size - 1);
    for b in 0..size {
        for g in 0..size {
            for r in 0..size {
                let [r, g, b] = sample([f64::from(r) / denominator, f64::from(g) / denominator, f64::from(b) / denominator]);
                if !r.is_finite() || !g.is_finite() || !b.is_finite() {
                    return Err("native resource lookup contains nonfinite colors".into());
                }
                writeln!(&mut data, "{:.9} {:.9} {:.9}", r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0)).map_err(|error| error.to_string())?;
            }
        }
    }
    Ok(NativeMapping {
        filters: vec![native_filter(
            "image.adjustments.colorLookup",
            json!({"data": data, "fileName": "imported-colors.cube", "interpolation": "tetrahedral", "dither": false}),
        )],
        notes,
        layer_effects: Vec::new(),
        ..Default::default()
    })
}

fn native_notes(detail: &str) -> Vec<String> {
    vec![format!("{detail}; editable native Color Lookup uses native RGB interpolation, bounded 0..1 gamut and PhotoCraft compositing")]
}

pub(super) fn map(effect: &StoredEffect<'_>, context: &MappingContext) -> MappingResult {
    let mapped = match effect.operation {
        "gegl:color-overlay" => overlay(effect, context),
        "gegl:color-exchange" => exchange(effect),
        "gegl:color-rotate" => rotate(effect),
        "gegl:color-warp" => warp(effect),
        "gegl:negative-darkroom" => {
            return Some(Err(MappingFailure::resource(
                "the source names a photographic paper response but does not store its curve; no film preset is guessed or fetched",
            )));
        }
        "gimp:fill-source" => fill(effect, context),
        "gimp:gradient" => {
            return Some(Err(MappingFailure::resource(
                "the source gradient object/stops are not serialized; its name or locator does not authorize resource lookup",
            )));
        }
        _ => return None,
    };
    Some(
        mapped
            .map_err(|message| match message.strip_prefix("resource unavailable: ") {
                Some(reason) => MappingFailure::resource(reason),
                None => MappingFailure::from(message),
            })
            .and_then(|mapping| profile_lookup(mapping, context)),
    )
}

fn profile_lookup(mut mapping: NativeMapping, context: &MappingContext) -> Result<NativeMapping, MappingFailure> {
    // Literal controls and standard hue coordinates above are defined in sRGB.
    // A native Color Lookup instead consumes the stored document coordinates.
    // Translate its parameter table through ordinary native CMS, never pixels.
    if context.mode == photocraft_color::ColorMode::Rgb && !context.linear && context.target_profile.is_none() {
        return Ok(mapping);
    }
    let Some(filter) = mapping.filters.first().filter(|filter| filter.command == "image.adjustments.colorLookup") else {
        return Ok(mapping);
    };
    let data = filter.params.get("data").and_then(serde_json::Value::as_str).ok_or("native resource lookup data missing")?;
    let table = photocraft_cms::lutfile::parse("native.cube", data.as_bytes()).map_err(|error| error.to_string())?;
    mapping.notes.push("Literal and hue-control sRGB coordinates are translated through the actual native document ICC profile into an ordinary native Color Lookup. Native 17-cube resampling, tetrahedral interpolation and bounded gamut/HDR rules apply; Gray results return to the native Gray profile rather than treating Gray samples as untagged RGB".into());
    super::color::native_lookup(context, "Native profiled color controls", mapping.notes, |rgb| {
        photocraft_compose::adjust::lut3d_sample(&table.data, table.size, rgb, true)
    })
}

fn overlay(effect: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping, String> {
    let color = color(effect, "value")?;
    let srgb = effect.boolean("srgb")?.unwrap_or(false);
    overlay_color(
        color,
        context,
        format!(
            "Source {} color overlay becomes ordinary native Color Overlay with native owner coverage and compositing",
            if srgb { "sRGB" } else { "linear-light" }
        ),
    )
}

/// Reusable ordinary CMS conversion for bounded native paints/gradient stops.
/// No source identity, resource lookup or operation executes through this mapper.
pub(super) struct NativeColorMapper {
    transform: Transform,
    mode: photocraft_color::ColorMode,
}

impl NativeColorMapper {
    pub(super) fn new(context: &MappingContext<'_>) -> Result<Self, String> {
        use photocraft_color::ColorMode;
        let fallback = match context.mode {
            ColorMode::Rgb => if context.linear { Builtin::LinearSrgb } else { Builtin::Srgb }.profile().clone(),
            ColorMode::Grayscale => {
                if context.linear {
                    photocraft_cms::builtin::linear_gray().clone()
                } else {
                    Builtin::SGray.profile().clone()
                }
            }
            _ => return Err("native paint requires an RGB or Gray target model".into()),
        };
        let destination = match &context.target_profile {
            Some(profile) => Profile::parse(profile).map_err(|error| format!("native effect destination ICC: {error}"))?,
            None => fallback,
        };
        let expected = if context.mode == ColorMode::Grayscale { ColorSpace::Gray } else { ColorSpace::Rgb };
        if destination.color_space != expected {
            return Err("native paint target profile differs from document model".into());
        }
        let transform = Transform::new(Builtin::Srgb.profile(), &destination, Intent::RelativeColorimetric, false)
            .map_err(|error| format!("native paint target conversion: {error}"))?;
        Ok(Self { transform, mode: context.mode })
    }

    pub(super) fn convert(&self, [r, g, b, a]: [f32; 4]) -> Result<photocraft_color::Color, String> {
        if [r, g, b, a].iter().any(|v| !v.is_finite()) || !(0.0..=1.0).contains(&a) {
            return Err("native paint has nonfinite color or invalid alpha".into());
        }
        let mut converted = [0.0; 4];
        self.transform.convert_f32(&[r, g, b], 3, &mut converted, if self.mode == photocraft_color::ColorMode::Grayscale { 1 } else { 3 }, false);
        if converted.iter().any(|v| !v.is_finite()) {
            return Err("native paint color conversion is nonfinite".into());
        }
        Ok(photocraft_color::Color { mode: self.mode, c: converted, alpha: a })
    }
}

/// Convert an already decoded sRGB literal to the actual native document model/profile.
pub(super) fn native_rgba(rgba: [f32; 4], context: &MappingContext<'_>) -> Result<photocraft_color::Color, String> {
    NativeColorMapper::new(context)?.convert(rgba)
}
/// Resolve an actual stored or already decoded portable literal. Defaults belong to the caller.
pub(super) fn native_color(effect: &StoredEffect<'_>, name: &str, context: &MappingContext) -> Result<photocraft_color::Color, String> {
    native_rgba(color(effect, name)?, context)
}

fn overlay_color(rgba: [f32; 4], context: &MappingContext, note: String) -> Result<NativeMapping, String> {
    use photocraft_color::BlendMode;
    use photocraft_doc::effects::{Effect, FxCommon};
    let mut color = native_rgba(rgba, context)?;
    // Native Color Overlay paint takes alpha from FxCommon.opacity.
    let opacity = color.alpha;
    color.alpha = 1.0;
    Ok(NativeMapping {
        filters: Vec::new(),
        layer_effects: vec![Effect::ColorOverlay { common: FxCommon::new(BlendMode::Normal, opacity), color }],
        notes: vec![note],
        ..Default::default()
    })
}

fn exchange(effect: &StoredEffect<'_>) -> Result<NativeMapping, String> {
    let from = color(effect, "from-color")?.map(f64::from);
    let to = color(effect, "to-color")?.map(f64::from);
    let thresholds = [number(effect, "red-threshold", 0.0)?, number(effect, "green-threshold", 0.0)?, number(effect, "blue-threshold", 0.0)?];
    if thresholds.iter().any(|value| !(0.0..=1.0).contains(value)) {
        return Err("source exchange thresholds are outside 0..1".into());
    }
    let [fr, fg, fb, _] = from;
    let [tr, tg, tb, alpha] = to;
    // A zero-width source range otherwise disappears between native lookup grid
    // points. Preserve a localized editable approximation with a half-cell floor
    // independently per channel, and disclose this native sampling difference.
    let [rt, gt, bt] = thresholds.map(|value| value.max(1.0 / 32.0));
    lookup(
        17,
        "Imported color exchange",
        native_notes(
            "Source channel thresholds become a native sampled color replacement; each range has a half-cell minimum and native interpolation softens exact boundaries",
        ),
        |[r, g, b]| {
            if (r - fr).abs() <= rt && (g - fg).abs() <= gt && (b - fb).abs() <= bt {
                [r * (1.0 - alpha) + tr * alpha, g * (1.0 - alpha) + tg * alpha, b * (1.0 - alpha) + tb * alpha]
            } else {
                [r, g, b]
            }
        },
    )
}

fn hsv([r, g, b]: [f64; 3]) -> [f64; 3] {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let h = if delta <= f64::EPSILON {
        0.0
    } else if max == r {
        60.0 * ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    [h, if max > 0.0 { delta / max } else { 0.0 }, max]
}

fn rgb([h, s, v]: [f64; 3]) -> [f64; 3] {
    let h = h.rem_euclid(360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
    let m = v - c;
    let [r, g, b] = if h < 1.0 {
        [c, x, 0.0]
    } else if h < 2.0 {
        [x, c, 0.0]
    } else if h < 3.0 {
        [0.0, c, x]
    } else if h < 4.0 {
        [0.0, x, c]
    } else if h < 5.0 {
        [x, 0.0, c]
    } else {
        [c, 0.0, x]
    };
    [r + m, g + m, b + m]
}

fn rotate(effect: &StoredEffect<'_>) -> Result<NativeMapping, String> {
    let sf = number(effect, "src-from", 0.0)?;
    let st = number(effect, "src-to", 90.0)?;
    let df = number(effect, "dest-from", 0.0)?;
    let dt = number(effect, "dest-to", 90.0)?;
    let sc = effect.boolean("src-clockwise")?.unwrap_or(false);
    let dc = effect.boolean("dest-clockwise")?.unwrap_or(false);
    let threshold = number(effect, "threshold", 0.0)?;
    let gray_hue = number(effect, "hue", 0.0)?;
    let gray_saturation = number(effect, "saturation", 0.0)?;
    let gray_mode = effect.enum_value("gray-mode")?.unwrap_or(1);
    if [sf, st, df, dt, gray_hue].iter().any(|value| !(0.0..=360.0).contains(value))
        || [threshold, gray_saturation].iter().any(|value| !(0.0..=1.0).contains(value))
        || ![0, 1].contains(&gray_mode)
    {
        return Err("source hue-sector or gray parameters are outside the documented domains".into());
    }
    let direction = |clockwise| if clockwise { -1.0 } else { 1.0 };
    let span = |from: f64, to: f64, clockwise| {
        let directed = (to - from) * direction(clockwise);
        let wrapped = directed.rem_euclid(360.0);
        if wrapped <= f64::EPSILON && directed >= 360.0 { 360.0 } else { wrapped }
    };
    let source_span = span(sf, st, sc);
    let dest_span = span(df, dt, dc);
    let mut notes = native_notes(
        "Source hue sectors and gray controls become a native sampled color lookup; directed positive full turns remain full spans while the opposite direction is an empty span",
    );
    if source_span <= f64::EPSILON {
        notes.push("An equal or empty directed source hue sector uses the explicit native empty-sector policy: hue is unchanged while configured gray handling remains active. Stock probes expose invalid-color artifacts at exact equal-sector endpoints; those artifacts are not reproduced.".into());
    }
    if dest_span <= f64::EPSILON {
        notes.push(
            "A collapsed destination sector maps selected native hues to its constant destination hue; native lookup interpolation softens sector boundaries."
                .into(),
        );
    }
    lookup(17, "Imported hue sectors", notes, |input| {
        let [mut h, mut s, v] = hsv(input);
        let low_chroma = s < threshold;
        if low_chroma {
            h = gray_hue;
            s = gray_saturation;
        }
        if source_span > f64::EPSILON && !(low_chroma && gray_mode == 1) {
            let directed_position = (h - sf) * direction(sc);
            // Preserve the distinguishable endpoint of a directed full turn. A hue of
            // zero is its end for360->0 clockwise, rather than the unavailable hue360.
            let position = if source_span >= 360.0 && directed_position >= 360.0 { 360.0 } else { directed_position.rem_euclid(360.0) };
            if position <= source_span {
                h = df + direction(dc) * dest_span * position / source_span;
            }
        }
        rgb([h, s, v])
    })
}

fn warp(effect: &StoredEffect<'_>) -> Result<NativeMapping, String> {
    let mut pairs = Vec::with_capacity(8);
    for index in 0..8 {
        let weight = number(effect, &format!("weight-{index}"), 100.0)?;
        let from = color(effect, &format!("from-{index}"))?.map(f64::from);
        let to = color(effect, &format!("to-{index}"))?.map(f64::from);
        pairs.push((from, to, weight));
    }
    let amount = number(effect, "amount", 1.0)?;
    let global_weight = number(effect, "weight", 1.0)?;
    lookup(
        17,
        "Imported weighted colors",
        native_notes("Source weighted correspondences become native inverse-distance color interpolation; source warp spline evaluation is approximated"),
        |input| {
            let [r, g, b] = input;
            let mut numerator = [0.0; 3];
            let mut denominator = 0.0;
            for (from, to, weight) in &pairs {
                let [fr, fg, fb, _] = *from;
                let [tr, tg, tb, alpha] = *to;
                let distance = (r - fr).powi(2) + (g - fg).powi(2) + (b - fb).powi(2);
                let influence = *weight / (distance + 1.0 / 4096.0);
                for (sum, delta) in numerator.iter_mut().zip([tr - fr, tg - fg, tb - fb]) {
                    *sum += influence * delta * alpha;
                }
                denominator += influence.abs();
            }
            let factor = if denominator > 0.0 { amount * global_weight / denominator } else { 0.0 };
            let [dr, dg, db] = numerator;
            [r + dr * factor, g + dg * factor, b + db * factor]
        },
    )
}

fn field<'a, 'b>(config: &'b Config<'a>, name: &str) -> Result<Option<&'b [ConfigExpr<'a>]>, String> {
    let mut fields = config.fields(name);
    let value = fields.next();
    if fields.next().is_some() {
        return Err(format!("duplicate fill configuration field {name}"));
    }
    Ok(value)
}

fn symbol<'a>(config: &Config<'a>, name: &str, default: &'a str) -> Result<&'a str, String> {
    match field(config, name)? {
        None => Ok(default),
        Some([ConfigExpr::Symbol(value)]) => Ok(value),
        _ => Err(format!("fill configuration {name} is not one symbolic value")),
    }
}

fn fill(effect: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping, String> {
    let options = effect.config("options")?.ok_or("resource unavailable: fill options were not stored")?;
    let style = symbol(options, "style", "fg-color")?;
    let custom = symbol(options, "custom-style", "solid-color")?;
    if custom != "solid-color" && custom != "pattern" {
        return Err("unknown stored custom fill style".into());
    }
    if style == "pattern" || custom == "pattern" {
        return Err("resource unavailable: a fill pattern identity has no stored authorized pixel resource".into());
    }
    let (name, fallback) = match style {
        "fg-color" => ("foreground", [0.0, 0.0, 0.0, 1.0]),
        "bg-color" => ("background", [1.0; 4]),
        _ => return Err("unknown stored fill style".into()),
    };
    let color = match field(options, name)? {
        None => fallback,
        Some([value]) => config_color(value)?,
        Some(_) => return Err("fill color field must contain exactly one structured color".into()),
    };
    overlay_color(color, context, "Stored fill color becomes ordinary native Color Overlay; native coverage replaces source antialias/feather policy".into())
}

fn config_color(value: &ConfigExpr<'_>) -> Result<[f32; 4], String> {
    let values = value.as_list().ok_or("fill color is not structured data")?;
    if matches!(values.first(), Some(ConfigExpr::Symbol("color-rgba"))) {
        let [_, r, g, b, a] = values else {
            return Err("legacy fill color requires four components".into());
        };
        let mut result = [0.0; 4];
        for (target, source) in result.iter_mut().zip([r, g, b, a]) {
            let number =
                source.as_number().filter(|value| value.is_finite() && (0.0..=1.0).contains(value)).ok_or("legacy fill color component is outside 0..1")?;
            *target = number as f32;
        }
        return Ok(result);
    }
    let Some(ConfigExpr::Symbol("color")) = values.first() else {
        return Err("fill color has no typed color record".into());
    };
    let Some(ConfigExpr::String(encoding)) = values.get(1) else {
        return Err("fill color encoding is missing".into());
    };
    let count = values.get(2).and_then(ConfigExpr::as_number).ok_or("fill color byte count is missing")?;
    let bytes = config_bytes(values.get(3).ok_or("fill color bytes are missing")?)?;
    if count != bytes.len() as f64 {
        return Err("fill color byte count differs from its payload".into());
    }
    let profile_count = values.get(4).and_then(ConfigExpr::as_number).ok_or("fill color profile length is missing")?;
    let profile = if profile_count == 0.0 {
        if values.len() != 5 {
            return Err("unexpected fields after the fill color record".into());
        }
        Vec::new()
    } else {
        if values.len() != 6 {
            return Err("embedded fill profile payload is missing".into());
        }
        let profile = config_bytes(values.get(5).ok_or("embedded fill profile is not byte data")?)?;
        if profile_count != profile.len() as f64 {
            return Err("embedded fill profile length differs from its payload".into());
        }
        profile
    };
    literal_rgb(&ColorLiteral { encoding, pixels: &bytes, profile: &profile })
}

fn config_bytes(value: &ConfigExpr<'_>) -> Result<Vec<u8>, String> {
    match value {
        ConfigExpr::String(text) => Ok(text.as_bytes().to_vec()),
        ConfigExpr::Bytes(bytes) => Ok(bytes.clone()),
        _ => Err("fill literal payload is not byte data".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Argument, parse_config};
    use super::*;
    fn portable<'a>(name: &'a str, rgba: [f32; 4], linear: bool) -> Argument<'a> {
        Argument { name, value: ArgumentValue::LiteralColor { rgba, linear } }
    }
    #[test]
    fn portable_numeric_colors_follow_native_cms_without_byte_or_integer_quantization() {
        for rgba in [[2., -0.125, 0.003, 0.375], [0.125, 0.5, 0.75, 1.], [0., -0., f32::from_bits(1), 0.]] {
            let encoded = StoredEffect::test("gegl:color", vec![portable("value", rgba, false)]);
            assert_eq!(color(&encoded, "value").unwrap().map(f32::to_bits), rgba.map(f32::to_bits));
            let linear = StoredEffect::test("gegl:color", vec![portable("value", rgba, true)]);
            let actual = color(&linear, "value").unwrap();
            let transform = Transform::new(Builtin::LinearSrgb.profile(), Builtin::Srgb.profile(), Intent::RelativeColorimetric, false).unwrap();
            let mut expected = [0.; 3];
            transform.convert_f32(&rgba[..3], 3, &mut expected, 3, false);
            assert_eq!(actual[..3], expected);
            assert_eq!(actual[3].to_bits(), rgba[3].to_bits());
        }
        // In particular .125 is a floating numeric component, not 32/255 or .125/255.
        let effect = StoredEffect::test("gegl:color", vec![portable("value", [0.125, 0.25, 0.5, 0.375], false)]);
        assert_eq!(color(&effect, "value").unwrap(), [0.125, 0.25, 0.5, 0.375]);
    }
    #[test]
    fn portable_literal_native_paint_honors_actual_rgb_and_gray_destination_profiles() {
        for (mode, builtin) in [(ColorMode::Rgb, Builtin::DisplayP3), (ColorMode::Grayscale, Builtin::GrayGamma22), (ColorMode::Rgb, Builtin::LinearSrgb)] {
            let rgba = [0.125, 0.5, 2., 0.375];
            let effect = StoredEffect::test("gegl:color", vec![portable("value", rgba, true)]);
            let mut context = context();
            context.mode = mode;
            context.target_profile = Some(builtin.profile().to_bytes());
            let actual = native_color(&effect, "value", &context).unwrap();
            let mut encoded = [0.; 3];
            Transform::new(Builtin::LinearSrgb.profile(), Builtin::Srgb.profile(), Intent::RelativeColorimetric, false).unwrap().convert_f32(
                &rgba[..3],
                3,
                &mut encoded,
                3,
                false,
            );
            let n = if mode == ColorMode::Grayscale { 1 } else { 3 };
            let mut expected = [0.; 4];
            Transform::new(Builtin::Srgb.profile(), &Profile::parse(context.target_profile.as_ref().unwrap()).unwrap(), Intent::RelativeColorimetric, false)
                .unwrap()
                .convert_f32(&encoded, 3, &mut expected, n, false);
            assert_eq!(actual.mode, mode);
            assert_eq!(actual.c, expected);
            assert_eq!(actual.alpha, rgba[3]);
        }
    }
    #[test]
    fn portable_color_rejects_nonfinite_or_invalid_alpha_and_preserves_old_wire_behavior() {
        for linear in [false, true] {
            for rgba in [
                [f32::NAN, 0., 0., 1.],
                [0., f32::INFINITY, 0., 1.],
                [0., 0., f32::NEG_INFINITY, 1.],
                [0., 0., 0., f32::NAN],
                [0., 0., 0., -0.01],
                [0., 0., 0., 1.01],
            ] {
                let effect = StoredEffect::test("gegl:color", vec![portable("value", rgba, linear)]);
                assert!(color(&effect, "value").is_err());
            }
        }
        let wire = StoredEffect::test("gegl:color", vec![literal("value", &[17, 83, 201, 77])]);
        let old = color(&wire, "value").unwrap();
        let expected = literal_rgb(&ColorLiteral { encoding: "R'G'B'A u8", pixels: &[17, 83, 201, 77], profile: &[] }).unwrap();
        assert_eq!(old, expected);
        let absent = StoredEffect::test("gegl:color", vec![Argument { name: "value", value: ArgumentValue::Color(None) }]);
        assert!(color(&absent, "value").unwrap_err().contains("absent"));
        assert!(color(&StoredEffect::test("gegl:color", vec![]), "value").is_err());
        let wrong = StoredEffect::test("gegl:color", vec![Argument { name: "value", value: ArgumentValue::Float(0.5) }]);
        assert!(color(&wrong, "value").is_err());
        let duplicate = StoredEffect::test("gegl:color", vec![portable("value", [0.; 4], false), portable("value", [0.; 4], false)]);
        assert!(color(&duplicate, "value").is_err());
        let ambiguous = [0.25f32, 0.5, 0.75, 1.].into_iter().flat_map(f32::to_le_bytes).collect::<Vec<_>>();
        assert!(literal_rgb(&ColorLiteral { encoding: "RGBA float", pixels: &ambiguous, profile: &[] }).unwrap_err().contains("byte-order"));
    }
    #[test]
    fn direct_color_readers_accept_the_same_validated_portable_literals() {
        let context = context();
        let colorize = StoredEffect::test("gimp:colorize", vec![portable("color", [0.125, 0.25, 0.5, 0.375], true)]);
        assert!(super::super::color::map(&colorize, &context).unwrap().is_ok());
        let lens = StoredEffect::test("gegl:apply-lens", vec![portable("background-color", [0.125, 0.25, 0.5, 0.375], true)]);
        assert!(super::super::residual_tone::map(&lens, &context).unwrap().is_ok());
        let checker = StoredEffect::test("gegl:checkerboard", vec![portable("color1", [0.125, 0.25, 0.5, 0.375], false)]);
        assert!(super::super::residual_tone::map(&checker, &context).unwrap().is_ok());
        let sinus = StoredEffect::test("gegl:sinus", vec![portable("color1", [0.125, 0.25, 0.5, 0.375], false)]);
        assert!(super::super::residual_transform::map(&sinus, &context).unwrap().is_ok());
        for (operation, name) in
            [("gimp:colorize", "color"), ("gegl:apply-lens", "background-color"), ("gegl:checkerboard", "color1"), ("gegl:sinus", "color1")]
        {
            let invalid = StoredEffect::test(operation, vec![portable(name, [f32::NAN, 0., 0., 1.], false)]);
            let result = match operation {
                "gimp:colorize" => super::super::color::map(&invalid, &context),
                "gegl:sinus" => super::super::residual_transform::map(&invalid, &context),
                _ => super::super::residual_tone::map(&invalid, &context),
            };
            assert!(result.unwrap().is_err(), "{operation} must validate a consumed literal");
        }
    }
    #[test]
    fn exact_zero_float_paint_is_portable_but_other_native_float_bytes_are_not() {
        let little = [0.0_f32; 4].into_iter().flat_map(f32::to_le_bytes).collect::<Vec<_>>();
        let big = [0.0_f32; 4].into_iter().flat_map(f32::to_be_bytes).collect::<Vec<_>>();
        assert_eq!(little, big);
        for encoding in ["RGBA float", "R'G'B'A float", "R~G~B~A float"] {
            assert_eq!(literal_rgb(&ColorLiteral { encoding, pixels: &little, profile: &[] }).unwrap(), [0.0; 4]);
            for pixels in [
                [0.25_f32, 0.5, 0.75, 1.0].into_iter().flat_map(f32::to_le_bytes).collect::<Vec<_>>(),
                [0.25_f32, 0.5, 0.75, 1.0].into_iter().flat_map(f32::to_be_bytes).collect::<Vec<_>>(),
            ] {
                assert!(literal_rgb(&ColorLiteral { encoding, pixels: &pixels, profile: &[] }).is_err());
            }
        }
        assert!(literal_rgb(&ColorLiteral { encoding: "RGB float", pixels: &little[..12], profile: &[] }).is_err());
    }
    use photocraft_color::ColorMode;
    use photocraft_doc::{effects::Effect, import_conversion::ConversionReason};

    fn context() -> MappingContext<'static> {
        let bounds = photocraft_geom::Rect::new(0, 0, 11, 7);
        MappingContext {
            mode: ColorMode::Rgb,
            linear: false,
            owner_bounds: bounds,
            native_filter_bounds: bounds,
            target_profile: None,
            resolution_dpi: 72.0,
            remaining_generation_bytes: crate::xcf::MAX_PIXEL_BYTES,
            remaining_metadata_bytes: crate::xcf::MAX_METADATA_BYTES,
            interrupt: photocraft_raster::Interrupt::NONE,
        }
    }
    fn literal<'a>(name: &'a str, pixels: &'a [u8]) -> Argument<'a> {
        Argument { name, value: ArgumentValue::Color(Some(ColorLiteral { encoding: "R'G'B'A u8", pixels, profile: &[] })) }
    }
    fn table(mapping: &NativeMapping) -> photocraft_cms::lutfile::LutFile {
        let filter = &mapping.filters[0];
        assert_eq!(filter.command, "image.adjustments.colorLookup");
        assert!(filter.params.get("file").is_none(), "source identities never become filesystem access");
        photocraft_cms::lutfile::parse("native.cube", filter.params["data"].as_str().unwrap().as_bytes()).unwrap()
    }

    #[test]
    fn resource_tables_translate_literal_srgb_into_assigned_rgb_and_gray_profiles() {
        use photocraft_cms::{Intent, Transform};
        use photocraft_color::ColorMode;
        for (mode, builtin) in [(ColorMode::Rgb, Builtin::DisplayP3), (ColorMode::Grayscale, Builtin::GrayGamma22)] {
            let mut c = context();
            c.mode = mode;
            c.target_profile = Some(builtin.profile().to_bytes());
            let source = StoredEffect::test("gegl:color-exchange", vec![literal("from-color", &[255, 255, 255, 255]), literal("to-color", &[255, 0, 0, 255])]);
            let native = map(&source, &c).unwrap().unwrap();
            let table = table(&native);
            let actual = photocraft_compose::adjust::lut3d_sample(&table.data, table.size, [1., 1., 1.], true);
            let transform = Transform::new(Builtin::Srgb.profile(), builtin.profile(), Intent::RelativeColorimetric, false).unwrap();
            let channels = if mode == ColorMode::Grayscale { 1 } else { 3 };
            let mut expected = [0.; 3];
            transform.convert_f32(&[1., 0., 0.], 3, &mut expected[..channels], channels, false);
            if channels == 1 {
                expected = [expected[0]; 3];
            }
            for (a, b) in actual.iter().zip(expected) {
                assert!((*a - b).abs() < 0.002, "{mode:?}: {actual:?} vs {expected:?}");
            }
            assert!(native.notes.iter().any(|note| note.contains("actual native document ICC")));
            let bounds = photocraft_geom::Rect::new(0, 0, 1, 1);
            let adjustment = photocraft_doc::Adjustment::ColorLookup {
                name: "Native profiled color".into(),
                size: table.size as u32,
                lut: Some(std::sync::Arc::new(table.data)),
                tetrahedral: true,
                dither: false,
            };
            let mut buffer = photocraft_compose::Buffer { rect: bounds, px: vec![[1., 1., 1., 0.375]] };
            photocraft_compose::adjust::apply(&adjustment, &mut buffer);
            assert_eq!(buffer.px[0][3], 0.375);
            for (a, b) in buffer.px[0][..3].iter().zip(expected) {
                assert!((*a - b).abs() < 0.002);
            }
            let mut document =
                photocraft_doc::Document::new("Native profiled resource", photocraft_geom::Size::new(1, 1), mode, photocraft_color::SampleType::F32);
            document.icc_profile = c.target_profile.clone();
            document.layers.push(photocraft_doc::Layer::raster("Original", document.pixel_format()));
            let bytes = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
            let mut object = photocraft_doc::SmartObject::new(
                photocraft_doc::SmartSource::Embedded { file_name: "native.pcraft".into(), bytes: std::sync::Arc::new(bytes) },
                photocraft_geom::Affine::IDENTITY,
                None,
            );
            object.smart_filters = native.filters.clone();
            document.layers.push(photocraft_doc::Layer::new("Native lookup", photocraft_doc::LayerContent::Smart(object)));
            let bytes = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
            assert_eq!(reopened.icc_profile, c.target_profile);
            let photocraft_doc::LayerContent::Smart(object) = &reopened.layers[1].content else { panic!("native lookup") };
            assert_eq!(object.smart_filters, native.filters);
        }
    }

    #[test]
    fn portable_literals_use_their_profile_and_float_order_is_never_guessed() {
        let color = literal_rgb(&ColorLiteral { encoding: "R'G'B'A u8", pixels: &[64, 128, 192, 255], profile: &[] }).unwrap();
        for (got, want) in color.into_iter().zip([64.0 / 255.0, 128.0 / 255.0, 192.0 / 255.0, 1.0]) {
            assert!((got - want).abs() < 0.0001);
        }
        let profile = Builtin::DisplayP3.profile().to_bytes();
        let with_profile = literal_rgb(&ColorLiteral { encoding: "R'G'B'A u8", pixels: &[64, 128, 192, 255], profile: &profile }).unwrap();
        assert!(with_profile.iter().zip(color).any(|(a, b)| (*a - b).abs() > 0.001));
        let ambiguous = [0.25_f32, 0.5, 0.75, 1.0].into_iter().flat_map(f32::to_le_bytes).collect::<Vec<_>>();
        assert!(literal_rgb(&ColorLiteral { encoding: "RGBA float", pixels: &ambiguous, profile: &[] }).unwrap_err().contains("byte-order"));
        assert!(literal_rgb(&ColorLiteral { encoding: "R'G'B'A u8", pixels: &[64, 128, 192], profile: &[] }).is_err());
        assert!(literal_rgb(&ColorLiteral { encoding: "R'G'B'A u8", pixels: &[64, 128, 192, 255], profile: b"not ICC" }).is_err());
    }

    #[test]
    fn reusable_native_color_mapper_honors_linear_gray_and_explicit_profiles() {
        let mut context = context();
        context.mode = ColorMode::Grayscale;
        context.linear = true;
        let mapper = NativeColorMapper::new(&context).unwrap();
        let paint = mapper.convert([0.5, 0.5, 0.5, 0.75]).unwrap();
        assert!((paint.c[0] - 0.214).abs() < 0.002);
        assert_eq!(paint.alpha, 0.75);
        context.target_profile = Some(Builtin::SGray.profile().to_bytes());
        let gamma = NativeColorMapper::new(&context).unwrap().convert([0.5, 0.5, 0.5, 1.0]).unwrap();
        assert!(gamma.c[0] > paint.c[0] + 0.2);
        assert!(mapper.convert([0.0, 0.0, 0.0, f32::NAN]).is_err());
    }

    #[test]
    fn color_overlay_is_an_ordinary_native_style_and_uses_actual_target_profile() {
        let effect = StoredEffect::test("gegl:color-overlay", vec![literal("value", &[64, 128, 192, 128])]);
        let mut context = context();
        context.linear = true;
        context.target_profile = Some(Builtin::LinearSrgb.profile().to_bytes());
        let mapping = map(&effect, &context).unwrap().unwrap();
        assert!(mapping.filters.is_empty());
        let Effect::ColorOverlay { color, ref common } = mapping.layer_effects[0] else { panic!("ordinary color overlay expected") };
        assert_eq!(color.mode, ColorMode::Rgb);
        assert_eq!(color.alpha, 1.0);
        assert!((common.opacity - 128.0 / 255.0).abs() < 0.0001);
        assert!(color.c[0] < 0.1 && color.c[2] > 0.5 && color.c[2] < 0.55);
        context.mode = ColorMode::Grayscale;
        context.target_profile = Some(Builtin::SGray.profile().to_bytes());
        let mapping = map(&effect, &context).unwrap().unwrap();
        assert!(matches!(mapping.layer_effects[0], Effect::ColorOverlay { color, .. } if color.mode == ColorMode::Grayscale));
    }

    #[test]
    fn exchange_keeps_independent_channel_ranges_and_unaffected_colors() {
        let effect = StoredEffect::test(
            "gegl:color-exchange",
            vec![
                literal("from-color", &[0, 0, 0, 255]),
                literal("to-color", &[255, 0, 0, 255]),
                Argument { name: "red-threshold", value: ArgumentValue::Float(0.25) },
                Argument { name: "green-threshold", value: ArgumentValue::Float(0.125) },
                Argument { name: "blue-threshold", value: ArgumentValue::Float(0.0625) },
            ],
        );
        let mapping = map(&effect, &context()).unwrap().unwrap();
        let lut = table(&mapping);
        assert_eq!(lut.size, 17);
        assert_eq!(&lut.data[..3], &[1.0, 0.0, 0.0]);
        assert_eq!(&lut.data[lut.data.len() - 3..], &[1.0, 1.0, 1.0]);
        assert_eq!(&lut.data[3 * (17 * 3)..3 * (17 * 3) + 3], &[0.0, 0.1875, 0.0]);
    }

    #[test]
    fn hue_sector_uses_direction_span_and_explicit_gray_controls() {
        let args = vec![
            Argument { name: "src-to", value: ArgumentValue::Float(120.0) },
            Argument { name: "dest-from", value: ArgumentValue::Float(120.0) },
            Argument { name: "dest-to", value: ArgumentValue::Float(240.0) },
        ];
        let effect = StoredEffect::test("gegl:color-rotate", args);
        let lut = table(&map(&effect, &context()).unwrap().unwrap());
        let red = 16 * 3;
        assert_eq!(&lut.data[red..red + 3], &[0.0, 1.0, 0.0]);
        assert!(
            map(&StoredEffect::test("gegl:color-rotate", vec![Argument { name: "gray-mode", value: ArgumentValue::Enum(9) }]), &context()).unwrap().is_err()
        );
    }

    #[test]
    fn gray_sector_modes_change_the_native_table_independently() {
        let make = |mode| {
            StoredEffect::test(
                "gegl:color-rotate",
                vec![
                    Argument { name: "src-to", value: ArgumentValue::Float(120.0) },
                    Argument { name: "dest-from", value: ArgumentValue::Float(120.0) },
                    Argument { name: "dest-to", value: ArgumentValue::Float(240.0) },
                    Argument { name: "threshold", value: ArgumentValue::Float(0.2) },
                    Argument { name: "hue", value: ArgumentValue::Float(60.0) },
                    Argument { name: "saturation", value: ArgumentValue::Float(1.0) },
                    Argument { name: "gray-mode", value: ArgumentValue::Enum(mode) },
                ],
            )
        };
        let changed = table(&map(&make(1), &context()).unwrap().unwrap());
        let treated = table(&map(&make(0), &context()).unwrap().unwrap());
        let gray = ((8 * 17 + 8) * 17 + 8) * 3;
        assert_eq!(&changed.data[gray..gray + 3], &[0.5, 0.5, 0.0]);
        assert_eq!(&treated.data[gray..gray + 3], &[0.0, 0.5, 0.5]);
    }

    #[test]
    fn full_hue_turn_is_not_lost_as_a_degenerate_sector() {
        let full = StoredEffect::test(
            "gegl:color-rotate",
            vec![Argument { name: "src-to", value: ArgumentValue::Float(360.0) }, Argument { name: "dest-to", value: ArgumentValue::Float(360.0) }],
        );
        let lut = table(&map(&full, &context()).unwrap().unwrap());
        let index = ((12 * 17 + 9) * 17 + 5) * 3;
        assert_eq!(&lut.data[index..index + 3], &[0.3125, 0.5625, 0.75]);
    }

    #[test]
    fn observed_directed_full_empty_and_collapsed_hue_sectors_are_native_data() {
        let effect = |from, to, cw, dfrom, dto, dcw| {
            StoredEffect::test(
                "gegl:color-rotate",
                vec![
                    Argument { name: "src-from", value: ArgumentValue::Float(from) },
                    Argument { name: "src-to", value: ArgumentValue::Float(to) },
                    Argument { name: "src-clockwise", value: ArgumentValue::Bool(cw) },
                    Argument { name: "dest-from", value: ArgumentValue::Float(dfrom) },
                    Argument { name: "dest-to", value: ArgumentValue::Float(dto) },
                    Argument { name: "dest-clockwise", value: ArgumentValue::Bool(dcw) },
                ],
            )
        };
        let red = 16 * 3;
        let yellow = (16 * 17 + 16) * 3;
        for (source, expected) in [
            (effect(0., 0., false, 120., 240., false), [1., 0., 0.]),
            (effect(0., 360., true, 120., 240., false), [1., 0., 0.]),
            (effect(360., 0., false, 120., 240., false), [1., 0., 0.]),
            (effect(0., 360., false, 120., 240., false), [0., 1., 0.]),
            (effect(360., 0., true, 120., 240., false), [0., 0., 1.]),
        ] {
            let mapping = map(&source, &context()).unwrap().unwrap();
            let lut = table(&mapping);
            assert_eq!(&lut.data[red..red + 3], &expected);
            assert!(mapping.notes.iter().all(|v| !v.contains("unresolved")));
        }
        for clockwise in [false, true] {
            let mapping = map(&effect(0., 90., false, 120., 120., clockwise), &context()).unwrap().unwrap();
            let lut = table(&mapping);
            assert_eq!(&lut.data[red..red + 3], &[0., 1., 0.]);
            assert_eq!(&lut.data[yellow..yellow + 3], &[0., 1., 0.]);
            assert!(mapping.notes.iter().any(|v| v.contains("constant destination hue")));
        }
        let empty_destination = table(&map(&effect(0., 90., false, 0., 360., true), &context()).unwrap().unwrap());
        assert_eq!(&empty_destination.data[yellow..yellow + 3], &[1., 0., 0.]);
        let reverse_full = table(&map(&effect(0., 90., false, 360., 0., true), &context()).unwrap().unwrap());
        assert_eq!(&reverse_full.data[yellow..yellow + 3], &[0., 1., 0.]);
        let mut empty_gray = effect(0., 0., false, 120., 240., false);
        empty_gray.arguments.extend([
            Argument { name: "threshold", value: ArgumentValue::Float(0.2) },
            Argument { name: "hue", value: ArgumentValue::Float(60.) },
            Argument { name: "saturation", value: ArgumentValue::Float(1.) },
        ]);
        let lut = table(&map(&empty_gray, &context()).unwrap().unwrap());
        let gray = ((8 * 17 + 8) * 17 + 8) * 3;
        assert_eq!(&lut.data[gray..gray + 3], &[0.5, 0.5, 0.]);
    }
    #[test]
    fn collapsed_hue_sector_has_native_alpha_counterpart_and_smart_persistence() {
        use photocraft_color::{PixelFormat, SampleType};
        use photocraft_doc::{Adjustment, Document, Layer, LayerContent, SmartObject, SmartSource};
        use photocraft_geom::{Affine, Rect, Size};
        use std::sync::Arc;
        let effect = StoredEffect::test(
            "gegl:color-rotate",
            vec![Argument { name: "dest-from", value: ArgumentValue::Float(120.) }, Argument { name: "dest-to", value: ArgumentValue::Float(120.) }],
        );
        let mapping = map(&effect, &context()).unwrap().unwrap();
        let lut = table(&mapping);
        let mut buffer = photocraft_compose::Buffer { rect: Rect::new(0, 0, 1, 1), px: vec![[1., 0., 0., 0.75]] };
        photocraft_compose::adjust::apply(
            &Adjustment::ColorLookup {
                name: "Native constant hue".into(),
                lut: Some(Arc::new(lut.data)),
                size: u32::try_from(lut.size).unwrap(),
                tetrahedral: true,
                dither: false,
            },
            &mut buffer,
        );
        assert_eq!(buffer.px, vec![[0., 1., 0., 0.75]]);
        let format = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
        let mut contents = Document::new("Original native pixels", Size::new(1, 1), ColorMode::Rgb, SampleType::F32);
        let mut raster = Layer::raster("Original", format);
        raster.surface_mut().unwrap().fill_rect(contents.bounds(), &[1., 0., 0., 0.75]);
        contents.layers.push(raster);
        let bytes = photocraft_format::save_to_bytes(&contents, &Default::default()).unwrap();
        let mut smart = SmartObject::new(SmartSource::Embedded { file_name: "contents.pcraft".into(), bytes: Arc::new(bytes) }, Affine::IDENTITY, None);
        smart.smart_filters = mapping.filters.clone();
        let mut native = Document::new("Native hue mapping", Size::new(1, 1), ColorMode::Rgb, SampleType::F32);
        native.layers.push(Layer::new("Editable hue", LayerContent::Smart(smart)));
        let saved = photocraft_format::save_to_bytes(&native, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        let LayerContent::Smart(object) = &reopened.layers[0].content else { panic!("ordinary native smart object") };
        assert_eq!(object.smart_filters, mapping.filters);
        assert!(object.smart_filters[0].params.get("file").is_none());
    }

    #[test]
    fn binary_color_payload_that_is_valid_utf8_keeps_its_original_bytes() {
        let color = ConfigExpr::List(vec![
            ConfigExpr::Symbol("color"),
            ConfigExpr::String("R'G'B'A u8".into()),
            ConfigExpr::Number(4.0),
            ConfigExpr::String("éAB".into()),
            ConfigExpr::Number(0.0),
        ]);
        let value = config_color(&color).unwrap();
        for (got, want) in value.into_iter().zip([195.0 / 255.0, 169.0 / 255.0, 65.0 / 255.0, 66.0 / 255.0]) {
            assert!((got - want).abs() < 0.0001);
        }
    }

    #[test]
    fn zero_weight_warp_stays_native_identity_and_never_resolves_resources() {
        const FROM: [&str; 8] = ["from-0", "from-1", "from-2", "from-3", "from-4", "from-5", "from-6", "from-7"];
        const TO: [&str; 8] = ["to-0", "to-1", "to-2", "to-3", "to-4", "to-5", "to-6", "to-7"];
        const WEIGHT: [&str; 8] = ["weight-0", "weight-1", "weight-2", "weight-3", "weight-4", "weight-5", "weight-6", "weight-7"];
        let mut args = Vec::new();
        for ((from, to), weight) in FROM.into_iter().zip(TO).zip(WEIGHT) {
            args.push(literal(from, &[0, 0, 0, 255]));
            args.push(literal(to, &[255, 255, 255, 255]));
            args.push(Argument { name: weight, value: ArgumentValue::Float(0.0) });
        }
        let lut = table(&map(&StoredEffect::test("gegl:color-warp", args), &context()).unwrap().unwrap());
        assert_eq!(&lut.data[..3], &[0.0, 0.0, 0.0]);
        assert_eq!(&lut.data[lut.data.len() - 3..], &[1.0, 1.0, 1.0]);
    }

    #[test]
    fn unavailable_resources_are_inert_and_distinct_from_missing_mapping() {
        for operation in ["gimp:gradient", "gegl:negative-darkroom"] {
            let failure = map(&StoredEffect::test(operation, Vec::new()), &context()).unwrap().unwrap_err();
            assert_eq!(failure.reason, ConversionReason::ResourceUnavailable);
        }
        assert!(map(&StoredEffect::test("unrelated:operation", Vec::new()), &context()).is_none());
    }

    #[test]
    fn fill_options_are_typed_and_literal_fills_are_native_styles() {
        let mut budget = 1 << 20;
        let config = parse_config(r#"(style bg-color)(background (color "R'G'B'A u8" 4 "\100\200\377\377" 0))"#, &mut budget).unwrap();
        let effect = StoredEffect::test("gimp:fill-source", vec![Argument { name: "options", value: ArgumentValue::Config(config) }]);
        let mapping = map(&effect, &context()).unwrap().unwrap();
        assert!(mapping.filters.is_empty());
        assert!(matches!(mapping.layer_effects[0], Effect::ColorOverlay { .. }));
        for raw in ["(style pattern)", "(style 9)", "(style bg-color)(style fg-color)"] {
            let config = parse_config(raw, &mut budget).unwrap();
            assert!(
                map(&StoredEffect::test("gimp:fill-source", vec![Argument { name: "options", value: ArgumentValue::Config(config) }]), &context())
                    .unwrap()
                    .is_err()
            );
        }
    }

    #[test]
    fn extra_numeric_arguments_and_trailing_fill_color_fields_remain_inert() {
        let effect = StoredEffect::test(
            "gegl:color-overlay",
            vec![literal("value", &[0, 0, 0, 255]), Argument { name: "future-control", value: ArgumentValue::Int(123) }],
        );
        let failure = super::super::map(&effect, &context()).unwrap_err();
        assert!(failure.message.contains("future-control"));
        for raw in ["(style fg-color) (foreground (color-rgba 0.1 0.2 0.3 1) 123)", "(style fg-color) (foreground)"] {
            let config = super::super::parse_config(raw, &mut (1 << 20)).unwrap();
            let effect = StoredEffect::test("gimp:fill-source", vec![Argument { name: "options", value: ArgumentValue::Config(config) }]);
            assert!(map(&effect, &context()).unwrap().is_err());
        }
    }
}
