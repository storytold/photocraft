//! Frozen B02 identities O057/O058/O064/O122/O142/O170/O181.
//! Source values become existing native commands. Curves remain ordinary editable
//! native point or pencil curves; no source evaluator is installed.

use photocraft_color::ColorMode;
use serde_json::{Value, json};

use super::{ConfigExpr, MappingContext, MappingResult, NativeMapping, StoredEffect, native_filter};

pub(super) fn map(effect: &StoredEffect<'_>, context: &MappingContext) -> MappingResult {
    match effect.operation {
        "gegl:invert-gamma" | "gegl:invert-linear" | "gegl:invert" => Some(invert(effect, context).map_err(Into::into)),
        "gegl:value-invert" => Some(value_invert(effect, context).map_err(Into::into)),
        "gegl:local-threshold" => Some(local_threshold(effect, context).map_err(Into::into)),
        "gimp:posterize" => Some(posterize(effect).map_err(Into::into)),
        "gimp:threshold" => Some(threshold(effect, context).map_err(Into::into)),
        "gimp:curves" => Some(curves(effect).map_err(Into::into)),
        _ => None,
    }
}

fn allowed(effect: &StoredEffect<'_>, names: &[&str]) -> Result<(), String> {
    for argument in &effect.arguments {
        if !names.contains(&argument.name) {
            return Err(format!("Unmapped {} argument {}", effect.operation, argument.name));
        }
        effect.argument(argument.name)?;
    }
    Ok(())
}

fn finite(value: f64, min: f64, max: f64, label: &str) -> Result<f64, String> {
    if !value.is_finite() || !(min..=max).contains(&value) {
        return Err(format!("{label} must be finite in {min}..={max}"));
    }
    Ok(value)
}

fn invert(effect: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping, String> {
    allowed(effect, &[])?;
    let source_linear = matches!(effect.operation, "gegl:invert-linear" | "gegl:invert");
    let mut notes = vec!["Native document encoding, depth, range and Photoshop-compatible Invert rules apply".into()];
    if source_linear != context.linear {
        notes.push(
            "Source inversion transfer interpretation differs from the native document; editable native Invert is an explicit transfer-space approximation"
                .into(),
        );
    }
    Ok(NativeMapping { filters: vec![native_filter("image.adjustments.invert", json!({}))], notes, ..Default::default() })
}

fn value_invert(effect: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping, String> {
    allowed(effect, &[])?;
    if context.mode == ColorMode::Grayscale {
        return Ok(NativeMapping {
            filters: vec![native_filter("image.adjustments.invert", json!({}))],
            notes: vec!["Grayscale source value becomes native document-encoding Invert; native depth/range rules apply".into()],
            ..Default::default()
        });
    }
    if context.mode != ColorMode::Rgb {
        return Err("Native HSV value inversion requires an RGB document".into());
    }
    Ok(NativeMapping { filters:vec![
        native_filter("filter.other.hsbHsl",json!({"inputMode":"rgb","rowOrder":"hsb"})),
        native_filter("image.adjustments.curves",json!({"blue":[[0,255],[255,0]]})),
        native_filter("filter.other.hsbHsl",json!({"inputMode":"hsb","rowOrder":"rgb"})),
    ], notes:vec!["Source HSV value inversion becomes native RGB→HSB, editable blue-channel Invert curve, HSB→RGB; native clamping, intermediate precision and color interpretation may differ".into()], ..Default::default() })
}

fn posterize(effect: &StoredEffect<'_>) -> Result<NativeMapping, String> {
    allowed(effect, &["levels"])?;
    let levels = finite(effect.number("levels")?.unwrap_or(3.0), 2.0, 256.0, "posterize levels")?;
    if levels.fract() != 0.0 {
        return Err("Posterize levels must be an integer".into());
    }
    let mut notes = vec!["Native Posterize quantization, channel and HDR behavior apply at the native document depth".into()];
    let native_levels = if levels == 256.0 {
        notes.push(
            "Source 256 levels exceeds native Posterize's editable maximum of 255; explicitly approximated with 255 levels rather than silently clamped".into(),
        );
        255.0
    } else {
        levels
    };
    Ok(NativeMapping { filters: vec![native_filter("image.adjustments.posterize", json!({"levels":native_levels}))], notes, ..Default::default() })
}

fn threshold(effect: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping, String> {
    allowed(effect, &["channel", "low", "high"])?;
    let channel = effect.enum_value("channel")?.unwrap_or(0);
    let low = finite(effect.number("low")?.unwrap_or(0.5), 0.0, 1.0, "threshold low")?;
    let high = finite(effect.number("high")?.unwrap_or(1.0), 0.0, 1.0, "threshold high")?;
    if channel == 4 {
        let values: Vec<f64> = (0..256)
            .map(|i| {
                let value = f64::from(i) / 255.;
                if value >= low && value <= high { 1. } else { 0. }
            })
            .collect();
        return Ok(NativeMapping {
            filters: vec![native_filter("filter.other.alphaCurve", json!({"values":values}))],
            notes: vec![format!(
                "Source alpha interval [{low},{high}] becomes an ordinary native256-sample alpha transfer. Both inclusive interval bounds author the table; crossed bounds explicitly use the native empty-interval policy without reordering. Native piecewise-linear transition/depth quantization and clamped coverage apply. Existing native color/HDR samples are preserved rather than thresholded as RGB."
            )],
            ..Default::default()
        });
    }
    if !(0..=5).contains(&channel) {
        return Err("Unknown source threshold channel".into());
    }
    if context.mode != ColorMode::Rgb && context.mode != ColorMode::Grayscale {
        return Err("Threshold source channels require native RGB/Gray conversion".into());
    }
    let working_rgb = context.mode == ColorMode::Grayscale && matches!(channel, 1..=3);
    let mut filters = Vec::new();
    if context.mode == ColorMode::Rgb || working_rgb {
        let weights = match channel {
            0 => {
                filters.push(native_filter("filter.other.hsbHsl", json!({"inputMode":"rgb","rowOrder":"hsb"})));
                [0.0, 0.0, 100.0, 0.0]
            }
            1 => [100.0, 0.0, 0.0, 0.0],
            2 => [0.0, 100.0, 0.0, 0.0],
            3 => [0.0, 0.0, 100.0, 0.0],
            5 => [21.26, 71.52, 7.22, 0.0],
            _ => return Err("Unsupported threshold channel".into()),
        };
        filters.push(native_filter("image.adjustments.channelMixer", json!({"monochrome":true,"gray":weights})));
    }
    let outputs: Vec<_> = (0..256)
        .map(|i| {
            let x = f64::from(i) / 255.0;
            if x >= low && x <= high { 255 } else { 0 }
        })
        .collect();
    filters.push(native_filter("image.adjustments.curves", json!({"pencil":true,"points":outputs})));
    let mut notes = vec![
        format!(
            "Source channel {channel} interval [{low},{high}] becomes native component extraction and an editable 256-entry pencil curve; crossed bounds use the explicit native empty-interval policy without reordering, while interval edges follow native curve interpolation and depth quantization"
        ),
        "Native color/luminance interpretation and out-of-range/HDR clamping apply; alpha is preserved rather than thresholded".into(),
    ];
    if working_rgb {
        notes.push("An RGB channel selected on a Gray owner becomes an ordinary native RGB/sRGB Smart contents stage. Existing native ICC conversion defines the RGB components; native final conversion restores the Gray owner. This is an explicit native component-space approximation, rather than an invented source Gray-channel interpretation".into());
    }
    Ok(NativeMapping { filters, working_rgb, notes, ..Default::default() })
}

fn local_threshold(effect: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping, String> {
    allowed(effect, &["radius", "aa-factor", "low", "high"])?;
    let radius = finite(effect.number("radius")?.unwrap_or(200.0), 0.0, 1000.0, "local threshold native radius")?;
    let aa = finite(effect.number("aa-factor")?.unwrap_or(1.0), 1.0, 256.0, "local threshold antialias factor")?;
    if aa.fract() != 0.0 {
        return Err("Source local threshold antialias factor must be an integer".into());
    }
    let low = finite(effect.number("low")?.unwrap_or(0.5), 0.0, 1.0, "local threshold low")?;
    let high = finite(effect.number("high")?.unwrap_or(1.0), 0.0, 10.0, "local threshold high")?;
    if !matches!(context.mode, ColorMode::Rgb | ColorMode::Grayscale) {
        return Err("Local threshold requires an RGB/Gray native document".into());
    }
    let mut filters = vec![native_filter("filter.other.highPass", json!({"radius":radius.max(0.1)}))];
    if context.mode == ColorMode::Rgb {
        filters.push(native_filter("image.adjustments.channelMixer", json!({"monochrome":true,"gray":[21.26,71.52,7.22,0.0]})));
    }
    let outputs: Vec<_> = (0..256)
        .map(|i| {
            let x = f64::from(i) / 255.0;
            if x >= low && x <= high { 255 } else { 0 }
        })
        .collect();
    filters.push(native_filter("image.adjustments.curves", json!({"pencil":true,"points":outputs})));
    let mut notes = vec![
        format!(
            "Local threshold becomes native High Pass radius {} pixels, native grayscale contrast and editable interval [{low},{high}] pencil curve; native 50%-centered local contrast replaces source adaptive classification, and crossed bounds explicitly select the native empty-interval policy without reordering",
            radius.max(0.1)
        ),
        format!("Source antialias factor {aa} is retained; ordinary native filter sampling and pencil interpolation apply rather than source supersampling"),
        "Native depth, clipping and HDR behavior apply; this is a disclosed local-binarization approximation, not a source neighborhood evaluator".into(),
    ];
    if radius < 0.1 {
        notes.push("Source radius below 0.1 pixels is explicitly approximated by native High Pass's minimum 0.1 pixels".into());
    }
    if high > 1.0 {
        notes.push(
            "Source high bound exceeds the native pencil curve's 0..1 domain; source HDR classification is retained and native bounded-curve behavior applies"
                .into(),
        );
    }
    Ok(NativeMapping { filters, notes, ..Default::default() })
}

fn scalar<'a>(values: &'a [ConfigExpr<'a>], name: &str) -> Result<Option<&'a ConfigExpr<'a>>, String> {
    let mut found = None;
    for form in values {
        let list = form.as_list().ok_or_else(|| "Curve configuration requires named lists".to_owned())?;
        if list.first().and_then(ConfigExpr::as_symbol) == Some(name) {
            if found.is_some() || list.len() != 2 {
                return Err(format!("Duplicate or invalid curve field {name}"));
            }
            found = list.get(1);
        }
    }
    Ok(found)
}

fn array<'a>(values: &'a [ConfigExpr<'a>], name: &str) -> Result<Option<&'a [ConfigExpr<'a>]>, String> {
    let mut found = None;
    for form in values {
        let Some(list) = form.as_list() else {
            return Err("Curve configuration requires named lists".into());
        };
        if list.first().and_then(ConfigExpr::as_symbol) != Some(name) {
            continue;
        }
        if found.is_some() {
            return Err(format!("Duplicate curve array {name}"));
        }
        let count = list.get(1).and_then(ConfigExpr::as_number).ok_or_else(|| format!("Missing array count {name}"))?;
        if !count.is_finite() || !(0.0..=65536.0).contains(&count) || count.fract() != 0.0 {
            return Err(format!("Invalid array count {name}"));
        }
        let items = list.get(2..).unwrap_or(&[]);
        if items.len() != count as usize {
            return Err(format!("Curve array {name} count mismatch"));
        }
        found = Some(items);
    }
    Ok(found)
}

#[derive(Debug)]
struct Curve {
    params: Value,
    identity: bool,
    note: Option<String>,
}

fn curve(values: &[ConfigExpr<'_>]) -> Result<Curve, String> {
    for form in values {
        if !matches!(form.head(), Some("curve-type" | "n-points" | "points" | "point-types" | "n-samples" | "samples")) {
            return Err(format!("Unmapped curve configuration field {:?}", form.head()));
        }
    }
    let kind = scalar(values, "curve-type")?.and_then(ConfigExpr::as_symbol).unwrap_or("smooth");
    if let Some(n) = scalar(values, "n-samples")?
        && n.as_number() != Some(256.0)
    {
        return Err("Source curve sample count must be 256".into());
    }
    let samples = array(values, "samples")?;
    if let Some(samples) = samples {
        if samples.len() != 256 {
            return Err("Source free-curve table requires 256 samples".into());
        }
        for value in samples {
            finite(value.as_number().ok_or_else(|| "Non-numeric curve sample".to_owned())?, 0.0, 1.0, "curve sample")?;
        }
    }
    if kind == "free" {
        let samples = samples.ok_or_else(|| "Free curve has no stored sample table".to_owned())?;
        let mut identity = true;
        let mut outputs = Vec::new();
        for (i, value) in samples.iter().enumerate() {
            let y = value.as_number().ok_or_else(|| "Invalid free curve sample".to_owned())?;
            identity &= (y - i as f64 / 255.0).abs() < 1e-7;
            outputs.push(y * 255.0);
        }
        return Ok(Curve {
            params: json!({"pencil":true,"points":outputs}),
            identity,
            note: Some("Stored free-curve samples become a native editable 256-entry pencil map with native interpolation and depth rules".into()),
        });
    }
    if kind != "smooth" {
        return Err("Unknown source curve type".into());
    }
    let raw = array(values, "points")?.ok_or_else(|| "Smooth curve has no points".to_owned())?;
    if !raw.len().is_multiple_of(2) {
        return Err("Smooth curve points require XY pairs".into());
    }
    let types = array(values, "point-types")?;
    if types.is_some_and(|v| v.len() != raw.len() / 2) {
        return Err("Curve point-type count mismatch".into());
    }
    let mut corners = false;
    if let Some(types) = types {
        for kind in types {
            match kind.as_symbol() {
                Some("smooth") => {}
                Some("corner") => corners = true,
                _ => return Err("Unknown curve point type".into()),
            }
        }
    }
    let mut points = Vec::new();
    let mut previous = None;
    for pair in raw.as_chunks::<2>().0 {
        let x = finite(pair.first().and_then(ConfigExpr::as_number).ok_or_else(|| "Invalid curve input".to_owned())?, -1.0, 1.0, "curve input")?;
        let y = finite(pair.get(1).and_then(ConfigExpr::as_number).ok_or_else(|| "Invalid curve output".to_owned())?, -1.0, 1.0, "curve output")?;
        if x < 0.0 {
            continue;
        }
        if y < 0.0 {
            return Err("Active smooth curve output is negative".into());
        }
        if previous.is_some_and(|p| x <= p) {
            return Err("Curve inputs must be strictly increasing; duplicate anchors cannot be guessed".into());
        }
        previous = Some(x);
        points.push([x * 255.0, y * 255.0]);
    }
    if points.len() < 2 {
        return Err("Smooth curve requires at least two active anchors".into());
    }
    let identity =
        points.iter().all(|p| (p[0] - p[1]).abs() < 1e-7) && points.first().is_some_and(|p| p[0] == 0.0) && points.last().is_some_and(|p| p[0] == 255.0);
    if points.len() > 19 || corners {
        // Native pencil maps are a normal editable representation. Preserve the
        // stored transfer table rather than implementing GIMP's spline solver.
        let samples = samples.ok_or_else(|| "Dense/corner curve requires its stored 256-sample transfer table".to_owned())?;
        let outputs: Vec<_> = samples.iter().filter_map(ConfigExpr::as_number).map(|v| v * 255.0).collect();
        return Ok(Curve {params:json!({"pencil":true,"points":outputs}),identity,
            note:Some("Dense or mixed-corner source curve becomes a native editable pencil map from its stored transfer samples; source anchor handles are retained inert".into())});
    }
    Ok(Curve {
        params: json!({"points":points}),
        identity,
        note: Some("Source smooth anchors become native editable Curves points; PhotoCraft spline interpolation replaces source interpolation".into()),
    })
}

/// Author a bounded native alpha transfer from the ordinary native Curves
/// adjustment. No source curve configuration survives in the runtime command.
fn native_alpha_curve(params: &Value) -> Result<Vec<f32>, String> {
    let adjustment = crate::xcf::coverage::adjustment("image.adjustments.curves", params)?;
    let mut ramp = photocraft_compose::Buffer {
        rect: photocraft_geom::Rect::new(0, 0, 256, 1),
        px: (0..256).map(|index| [index as f32 / 255., index as f32 / 255., index as f32 / 255., 1.]).collect(),
    };
    photocraft_compose::adjust::apply_in(
        &adjustment,
        &mut ramp,
        photocraft_compose::adjust::Transfer::Gamma(1.),
        Some(photocraft_color::SampleType::F32),
        ColorMode::Grayscale,
    );
    let values: Vec<_> = ramp.px.into_iter().map(|[value, ..]| value).collect();
    if values.len() != 256 || values.iter().any(|value| !value.is_finite() || !(0.0..=1.0).contains(value)) {
        return Err("Native alpha Curves transfer is not a finite normalized 256-sample table".into());
    }
    Ok(values)
}

fn curves(effect: &StoredEffect<'_>) -> Result<NativeMapping, String> {
    allowed(effect, &["trc", "config"])?;
    if let Some(trc) = effect.enum_value("trc")?
        && !(0..=2).contains(&trc)
    {
        return Err("Unknown curve transfer interpretation".into());
    }
    let mut mapped = Vec::new();
    if let Some(config) = effect.config("config")? {
        let mut channel = None;
        let mut seen = [false; 5];
        for form in &config.forms {
            let list = form.as_list().ok_or_else(|| "Curve configuration requires named forms".to_owned())?;
            match form.head() {
                Some("channel") => {
                    if channel.is_some() {
                        return Err("Curve channel has no following curve".into());
                    }
                    let value = list.get(1).and_then(ConfigExpr::as_symbol).ok_or_else(|| "Missing curve channel".to_owned())?;
                    channel = Some(match value {
                        "value" => 0,
                        "red" => 1,
                        "green" => 2,
                        "blue" => 3,
                        "alpha" => 4,
                        _ => return Err("Unknown curve channel".into()),
                    });
                }
                Some("curve") => {
                    let index = channel.take().ok_or_else(|| "Curve missing its channel selector".to_owned())?;
                    let previous = seen.get_mut(index).ok_or_else(|| "Invalid configured curve channel index".to_owned())?;
                    if *previous {
                        return Err("Duplicate configured curve channel".into());
                    }
                    *previous = true;
                    mapped.push((index, curve(list.get(1..).unwrap_or(&[]))?));
                }
                Some("trc") => {
                    if !matches!(list.get(1).and_then(ConfigExpr::as_symbol), Some("linear" | "non-linear" | "perceptual")) {
                        return Err("Unknown configured curve transfer interpretation".into());
                    }
                }
                Some("time" | "gimp-clip" | "gimp-region" | "gimp-mode" | "gimp-opacity") => {}
                _ => return Err(format!("Unmapped curves configuration field {:?}", form.head())),
            }
        }
        if channel.is_some() {
            return Err("Curve channel has no curve".into());
        }
    } else {
        return Err("Curves requires stored configuration".into());
    }
    if mapped.is_empty() {
        return Err("No source curves were decoded".into());
    }
    // GIMP serializes value before components; native evaluation applies the
    // individual components then the composite. Use native stage order explicitly.
    mapped.sort_by_key(|(channel, _)| if *channel == 0 { 4 } else { *channel - 1 });
    let mut filters = Vec::new();
    let mut notes = vec!["Source transfer-space requests remain historical; native document encoding, curve ordering, clamp and HDR rules apply".into()];
    for (channel, curve) in mapped {
        if curve.identity {
            continue;
        }
        if channel == 4 {
            filters.push(native_filter("filter.other.alphaCurve", json!({"values":native_alpha_curve(&curve.params)?})));
            notes.push("Source alpha curve becomes an ordinary native 256-sample Alpha Curve authored by PhotoCraft Curves evaluation; native piecewise-linear alpha interpolation, 0..1 domain and depth clamping apply. Original source anchors/transfer remain historical; RGB color/HDR samples are preserved".into());
            if let Some(note) = curve.note
                && !notes.contains(&note)
            {
                notes.push(note);
            }
            continue;
        }
        let mut params = curve.params;
        if channel != 0 {
            let object = params.as_object_mut().ok_or_else(|| "Invalid native curve params".to_owned())?;
            let points = object.remove("points").ok_or_else(|| "Missing native curve values".to_owned())?;
            let key = match channel {
                1 => "red",
                2 => "green",
                3 => "blue",
                _ => return Err("Invalid native curve channel".into()),
            };
            object.insert(key.into(), points);
        }
        filters.push(native_filter("image.adjustments.curves", params));
        if let Some(note) = curve.note
            && !notes.contains(&note)
        {
            notes.push(note);
        }
    }
    Ok(NativeMapping { filters, notes, ..Default::default() })
}

#[cfg(test)]
mod tests {
    use super::super::{Argument, ArgumentValue, parse_config};
    use super::*;
    fn context(mode: ColorMode) -> MappingContext<'static> {
        MappingContext {
            target_profile: None,
            resolution_dpi: 72.0,
            remaining_generation_bytes: crate::xcf::MAX_PIXEL_BYTES,
            remaining_metadata_bytes: crate::xcf::MAX_METADATA_BYTES,
            interrupt: photocraft_raster::Interrupt::NONE,
            mode,
            linear: false,
            owner_bounds: photocraft_geom::Rect::new(-2, 3, 15, 20),
            native_filter_bounds: photocraft_geom::Rect::new(0, 0, 17, 17),
        }
    }
    #[test]
    fn invert_identities_and_value_component_use_native_commands() {
        for operation in ["gegl:invert-gamma", "gegl:invert-linear", "gegl:invert"] {
            let source = StoredEffect::test(operation, vec![]);
            let native = map(&source, &context(ColorMode::Rgb)).unwrap().unwrap();
            assert_eq!(native.filters.len(), 1);
            assert_eq!(native.filters[0].command, "image.adjustments.invert");
        }
        let source = StoredEffect::test("gegl:value-invert", vec![]);
        let native = map(&source, &context(ColorMode::Rgb)).unwrap().unwrap();
        assert_eq!(
            native.filters.iter().map(|f| f.command.as_str()).collect::<Vec<_>>(),
            ["filter.other.hsbHsl", "image.adjustments.curves", "filter.other.hsbHsl"]
        );
        assert_eq!(native.filters[1].params, json!({"blue":[[0,255],[255,0]]}));
        assert_eq!(map(&source, &context(ColorMode::Grayscale)).unwrap().unwrap().filters.len(), 1);
    }
    #[test]
    fn posterize_boundary_is_explicit_and_invalid_input_is_not_clamped() {
        for (levels, expected) in [(2, 2), (17, 17), (255, 255), (256, 255)] {
            let source = StoredEffect::test("gimp:posterize", vec![Argument { name: "levels", value: ArgumentValue::Int(levels) }]);
            let native = map(&source, &context(ColorMode::Rgb)).unwrap().unwrap();
            assert_eq!(native.filters[0].params["levels"], json!(f64::from(expected)));
            if levels == 256 {
                assert!(native.notes.iter().any(|n| n.contains("256 levels")));
            }
        }
        let source = StoredEffect::test("gimp:posterize", vec![Argument { name: "levels", value: ArgumentValue::Float(f32::NAN) }]);
        assert!(map(&source, &context(ColorMode::Rgb)).unwrap().is_err());
    }
    #[test]
    fn threshold_preserves_selected_channel_and_both_interval_bounds() {
        let source = StoredEffect::test(
            "gimp:threshold",
            vec![
                Argument { name: "channel", value: ArgumentValue::Enum(1) },
                Argument { name: "low", value: ArgumentValue::Float(0.25) },
                Argument { name: "high", value: ArgumentValue::Float(0.75) },
            ],
        );
        let native = map(&source, &context(ColorMode::Rgb)).unwrap().unwrap();
        assert_eq!(native.filters[0].params, json!({"monochrome":true,"gray":[100.0,0.0,0.0,0.0]}));
        let values = native.filters[1].params["points"].as_array().unwrap();
        for (i, value) in values.iter().enumerate() {
            assert_eq!(value, &json!(if (64..=191).contains(&i) { 255 } else { 0 }));
        }
        let source = StoredEffect::test("gimp:threshold", vec![Argument { name: "channel", value: ArgumentValue::Enum(4) }]);
        let alpha = map(&source, &context(ColorMode::Rgb)).unwrap().unwrap();
        assert_eq!(alpha.filters[0].command, "filter.other.alphaCurve");
        assert_eq!(alpha.filters[0].params["values"].as_array().unwrap().len(), 256);
        let local = map(&StoredEffect::test("gegl:local-threshold", vec![]), &context(ColorMode::Rgb)).unwrap().unwrap();
        assert_eq!(local.filters[0].command, "filter.other.highPass");
        assert_eq!(local.filters[0].params, json!({"radius":200.0}));
        assert!(local.notes.iter().any(|note| note.contains("antialias factor 1")));
    }
    #[test]
    fn crossed_threshold_intervals_are_native_empty_transfers_without_reordering() {
        for operation in ["gimp:threshold", "gegl:local-threshold"] {
            let source = StoredEffect::test(
                operation,
                vec![Argument { name: "low", value: ArgumentValue::Float(0.75) }, Argument { name: "high", value: ArgumentValue::Float(0.25) }],
            );
            for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
                let native = map(&source, &context(mode)).unwrap().unwrap();
                let curve = native.filters.last().unwrap();
                assert_eq!(curve.command, "image.adjustments.curves");
                assert_eq!(curve.params["points"], json!(vec![0; 256]));
                let adjustment = crate::xcf::coverage::adjustment(&curve.command, &curve.params).unwrap();
                for depth in photocraft_color::SampleType::ALL {
                    let mut actual =
                        photocraft_compose::Buffer { rect: photocraft_geom::Rect::new(0, 0, 2, 1), px: vec![[0.5, 0.75, 0.25, 0.375], [2., -1., 3., 1.]] };
                    photocraft_compose::adjust::apply_in(
                        &adjustment,
                        &mut actual,
                        photocraft_compose::adjust::Transfer::for_document(mode, depth),
                        Some(depth),
                        mode,
                    );
                    assert_eq!(actual.px, vec![[0., 0., 0., 0.375], [0., 0., 0., 1.]]);
                }
            }
        }
        let source = StoredEffect::test(
            "gimp:threshold",
            vec![
                Argument { name: "channel", value: ArgumentValue::Enum(4) },
                Argument { name: "low", value: ArgumentValue::Float(0.75) },
                Argument { name: "high", value: ArgumentValue::Float(0.25) },
            ],
        );
        let native = map(&source, &context(ColorMode::Rgb)).unwrap().unwrap();
        assert_eq!(native.filters[0].params["values"], json!(vec![0.; 256]));
    }

    #[test]
    fn gray_rgb_threshold_channels_use_native_working_contents_and_persist() {
        use crate::xcf::native_effects::{NativeEnvelope, NativeOwner};
        use photocraft_color::{BlendMode, PixelFormat, SampleType};
        use photocraft_doc::{Document, Layer, LayerContent, SmartSource};
        use photocraft_geom::{Rect, Size};
        use photocraft_raster::{Interrupt, Surface};
        let bounds = Rect::new(0, 0, 3, 1);
        for depth in SampleType::ALL {
            for channel in 1..=3 {
                let source = StoredEffect::test(
                    "gimp:threshold",
                    vec![
                        Argument { name: "channel", value: ArgumentValue::Enum(channel) },
                        Argument { name: "low", value: ArgumentValue::Float(0.25) },
                        Argument { name: "high", value: ArgumentValue::Float(0.75) },
                    ],
                );
                let mapped = map(&source, &context(ColorMode::Grayscale)).unwrap().unwrap();
                assert!(mapped.working_rgb);
                let mut weights = [0.; 4];
                weights[(channel - 1) as usize] = 100.;
                assert_eq!(mapped.filters[0].params, json!({"monochrome":true,"gray":weights}));
                let mut actual = photocraft_compose::Buffer { rect: bounds, px: vec![[0.1, 0.1, 0.1, 0.25], [0.4, 0.4, 0.4, 0.5], [0.9, 0.9, 0.9, 1.]] };
                // Independent native sGray-to-sRGB counterpart is neutral RGB;
                // samples deliberately avoid the pencil interpolation edges.
                for filter in &mapped.filters {
                    let adjustment = crate::xcf::coverage::adjustment(&filter.command, &filter.params).unwrap();
                    photocraft_compose::adjust::apply_in(
                        &adjustment,
                        &mut actual,
                        photocraft_compose::adjust::Transfer::for_document(ColorMode::Rgb, depth),
                        Some(depth),
                        ColorMode::Rgb,
                    );
                }
                assert_eq!(actual.px, vec![[0., 0., 0., 0.25], [1., 1., 1., 0.5], [0., 0., 0., 1.]]);
                let mut doc = Document::new("Native Gray channel threshold", Size::new(3, 1), ColorMode::Grayscale, depth);
                doc.icc_profile = Some(photocraft_cms::Builtin::SGray.profile().to_bytes());
                let mut pixels = Surface::new(PixelFormat::new(ColorMode::Grayscale, depth, true));
                pixels.write_region(bounds, &[0.1, 0.25, 0.4, 0.5, 0.9, 1.]);
                let layer = Layer::new("Original Gray pixels", LayerContent::Raster(pixels));
                let mut budget = crate::xcf::MAX_PIXEL_BYTES;
                let mut owner = NativeOwner::new(&doc, &layer, bounds, &mut budget, &Interrupt::NONE).unwrap();
                let expected_filters = mapped.filters.clone();
                owner.push(mapped, NativeEnvelope { opacity: 1., blend: BlendMode::Normal, visible: true, mask: None }, &mut budget, &Interrupt::NONE).unwrap();
                doc.layers.push(Layer::new("Editable native threshold", owner.content()));
                let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
                let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
                assert_eq!(reopened.mode, ColorMode::Grayscale);
                let LayerContent::Smart(outer) = &reopened.layers[0].content else { panic!("native owner") };
                let SmartSource::Embedded { bytes, .. } = &outer.source else { panic!("embedded native contents") };
                let rgb = photocraft_format::load_from_bytes(bytes).unwrap();
                assert_eq!(rgb.mode, ColorMode::Rgb);
                assert_eq!(rgb.icc_profile, Some(photocraft_cms::Builtin::Srgb.profile().to_bytes()));
                let LayerContent::Smart(stage) = &rgb.layers[0].content else { panic!("native filter stage") };
                assert_eq!(stage.smart_filters, expected_filters);
            }
        }
    }

    #[test]
    fn alpha_threshold_interval_is_native_coverage_data_for_every_document_model() {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
            let source = StoredEffect::test(
                "gimp:threshold",
                vec![
                    Argument { name: "channel", value: ArgumentValue::Enum(4) },
                    Argument { name: "low", value: ArgumentValue::Float(0.25) },
                    Argument { name: "high", value: ArgumentValue::Float(0.75) },
                ],
            );
            let mapped = map(&source, &context(mode)).unwrap().unwrap();
            let values = mapped.filters[0].params["values"].as_array().unwrap();
            for (i, value) in values.iter().enumerate() {
                assert_eq!(value, &json!(if (64..=191).contains(&i) { 1.0 } else { 0.0 }));
            }
            assert!(mapped.notes[0].contains("Both inclusive interval bounds"));
        }
    }
    #[test]
    fn ordered_curve_groups_become_independent_native_channel_stages() {
        let raw = "(trc perceptual) (channel value) (curve (curve-type smooth) (points 6 0 0 0.5 0.7 1 1) (point-types 3 smooth smooth smooth))
            (channel red) (curve (curve-type smooth) (points 4 0 0.1 1 0.8) (point-types 2 smooth smooth))
            (channel alpha) (curve (curve-type smooth) (points 4 0 0 1 1) (point-types 2 smooth smooth))";
        let config = parse_config(raw, &mut (32 << 20)).unwrap();
        let source = StoredEffect::test("gimp:curves", vec![Argument { name: "config", value: ArgumentValue::Config(config) }]);
        let native = map(&source, &context(ColorMode::Rgb)).unwrap().unwrap();
        assert_eq!(native.filters.len(), 2);
        assert_eq!(native.filters[0].params, json!({"red":[[0.0,25.5],[255.0,204.0]]}));
        assert_eq!(native.filters[1].params, json!({"points":[[0.0,0.0],[127.5,178.5],[255.0,255.0]]}));
    }
    #[test]
    fn free_curve_table_keeps_all_samples_without_source_evaluation() {
        let samples = (0..256).map(|i| ((255 - i) as f64 / 255.0).to_string()).collect::<Vec<_>>().join(" ");
        let raw = format!("(channel green) (curve (curve-type free) (n-samples 256) (samples 256 {samples}))");
        let config = parse_config(&raw, &mut (32 << 20)).unwrap();
        let source = StoredEffect::test("gimp:curves", vec![Argument { name: "config", value: ArgumentValue::Config(config) }]);
        let native = map(&source, &context(ColorMode::Rgb)).unwrap().unwrap();
        assert_eq!(native.filters.len(), 1);
        assert_eq!(native.filters[0].params["pencil"], true);
        let values = native.filters[0].params["green"].as_array().unwrap();
        assert_eq!(values.len(), 256);
        for (i, v) in values.iter().enumerate() {
            assert!((v.as_f64().unwrap() - (255 - i) as f64).abs() < 1e-10);
        }
    }
    #[test]
    fn malformed_curves_are_retained_instead_of_misassigned() {
        for raw in [
            "(channel red) (curve (curve-type smooth) (points 4 0 0 0 1))",
            "(channel red) (curve (curve-type smooth) (points 3 0 0 1))",
            "(channel red) (curve (curve-type unknown) (points 4 0 0 1 1))",
        ] {
            let config = parse_config(raw, &mut (32 << 20)).unwrap();
            let source = StoredEffect::test("gimp:curves", vec![Argument { name: "config", value: ArgumentValue::Config(config) }]);
            assert!(map(&source, &context(ColorMode::Rgb)).unwrap().is_err(), "{raw}");
        }
    }

    #[test]
    fn alpha_smooth_curve_is_a_native_transfer_preserving_stored_color_axes_and_depth() {
        use photocraft_color::{PixelFormat, SampleType};
        use photocraft_doc::{
            Adjustment,
            adjust::{CurvePoint, ToneSpace},
        };
        let raw = "(channel alpha) (curve (curve-type smooth) (points 6 0 1 0.5 0.2 1 0))";
        let config = parse_config(raw, &mut (32 << 20)).unwrap();
        let source = StoredEffect::test("gimp:curves", vec![Argument { name: "config", value: ArgumentValue::Config(config) }]);
        let native = map(&source, &context(ColorMode::Rgb)).unwrap().unwrap();
        assert_eq!(native.filters.len(), 1);
        assert_eq!(native.filters[0].command, "filter.other.alphaCurve");
        let values: Vec<f32> = native.filters[0].params["values"].as_array().unwrap().iter().map(|value| value.as_f64().unwrap() as f32).collect();
        assert_eq!(values.len(), 256);
        assert!(values.iter().all(|value| value.is_finite() && (0.0..=1.0).contains(value)));
        let mut expected = photocraft_compose::Buffer {
            rect: photocraft_geom::Rect::new(0, 0, 256, 1),
            px: (0..256)
                .map(|i| {
                    let value = i as f32 / 255.;
                    [value, value, value, 1.]
                })
                .collect(),
        };
        let curve = Adjustment::Curves {
            master: vec![CurvePoint { input: 0., output: 1. }, CurvePoint { input: 0.5, output: 0.2 }, CurvePoint { input: 1., output: 0. }],
            per_channel: std::array::from_fn(|_| Vec::new()),
            space: ToneSpace::Rgb,
            black: Vec::new(),
            pencil: false,
        };
        photocraft_compose::adjust::apply_in(
            &curve,
            &mut expected,
            photocraft_compose::adjust::Transfer::Gamma(1.),
            Some(SampleType::F32),
            ColorMode::Grayscale,
        );
        assert_eq!(values, expected.px.iter().map(|pixel| pixel[0]).collect::<Vec<_>>());
        for sample in SampleType::ALL {
            for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
                let format = PixelFormat::new(mode, sample, true);
                let bounds = photocraft_geom::Rect::new(0, 0, 3, 1);
                let mut input = photocraft_raster::Surface::new(format);
                let mut pixels = Vec::new();
                for alpha in [0., 128. / 255., 1.] {
                    let mut pixel = vec![0.4; format.channels()];
                    *pixel.last_mut().unwrap() = alpha;
                    pixels.extend(pixel);
                }
                input.write_region(bounds, &pixels);
                let result = photocraft_algo::coverage::apply_with(
                    &input,
                    photocraft_algo::coverage::Operation::AlphaCurve(&values),
                    bounds,
                    bounds,
                    None,
                    &photocraft_raster::Interrupt::NONE,
                )
                .unwrap();
                for x in 0..3 {
                    let before = input.pixel(x, 0);
                    let after = result.pixel(x, 0);
                    assert_eq!(&after[..after.len() - 1], &before[..before.len() - 1], "{mode:?} {sample:?}");
                }
                assert_eq!(result.pixel(0, 0).last().copied(), Some(1.));
                assert_eq!(result.pixel(2, 0).last().copied(), Some(0.));
            }
        }
        let mut document = photocraft_doc::Document::new("Native alpha curve", photocraft_geom::Size::new(3, 1), ColorMode::Rgb, SampleType::F32);
        let mut source_document = document.clone();
        let mut pixels = photocraft_raster::Surface::new(source_document.pixel_format());
        pixels.fill_rect(source_document.bounds(), &[0.4, 0.5, 0.6, 1.]);
        source_document.layers.push(photocraft_doc::Layer::new("Original native pixels", photocraft_doc::LayerContent::Raster(pixels)));
        let source_bytes = photocraft_format::save_to_bytes(&source_document, &Default::default()).unwrap();
        let mut smart = photocraft_doc::SmartObject::new(
            photocraft_doc::SmartSource::Embedded { file_name: "native.pcraft".into(), bytes: std::sync::Arc::new(source_bytes) },
            photocraft_geom::Affine::IDENTITY,
            None,
        );
        smart.smart_filters = native.filters.clone();
        document.layers.push(photocraft_doc::Layer::new("Native Alpha Curve", photocraft_doc::LayerContent::Smart(smart)));
        let bytes = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        let photocraft_doc::LayerContent::Smart(smart) = &reopened.layers[0].content else {
            panic!("native smart data");
        };
        assert_eq!(smart.smart_filters, native.filters);
    }

    #[test]
    fn free_alpha_curve_uses_native_map_sampling_and_keeps_rgb_filters_unchanged() {
        let samples = (0..256).map(|index| ((255 - index) as f64 / 255.).to_string()).collect::<Vec<_>>().join(" ");
        let raw = format!(
            "(channel red) (curve (curve-type smooth) (points 4 0 0.1 1 0.8)) (channel alpha) (curve (curve-type free) (samples 256 {samples})) (channel value) (curve (curve-type smooth) (points 6 0 0 0.5 0.7 1 1))"
        );
        let config = parse_config(&raw, &mut (32 << 20)).unwrap();
        let source = StoredEffect::test("gimp:curves", vec![Argument { name: "config", value: ArgumentValue::Config(config) }]);
        let native = map(&source, &context(ColorMode::Rgb)).unwrap().unwrap();
        assert_eq!(native.filters.len(), 3);
        assert_eq!(native.filters[0].params, json!({"red":[[0.0,25.5],[255.0,204.0]]}));
        assert_eq!(native.filters[1].command, "filter.other.alphaCurve");
        let values = native.filters[1].params["values"].as_array().unwrap();
        assert_eq!(values.len(), 256);
        for (index, value) in values.iter().enumerate() {
            assert!((value.as_f64().unwrap() - (255 - index) as f64 / 255.).abs() < 0.00001);
        }
        assert_eq!(native.filters[2].params, json!({"points":[[0.0,0.0],[127.5,178.5],[255.0,255.0]]}));
    }
}
