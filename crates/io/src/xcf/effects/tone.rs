//! B01: O011/O044/O062/O136/O162, translated to ordinary native adjustments.
use super::{Config, ConfigExpr, MappingContext, MappingResult, NativeMapping, StoredEffect, native_filter};
use serde_json::{Value, json};

type Result<T> = std::result::Result<T, String>;

pub(super) fn map(effect: &StoredEffect<'_>, context: &MappingContext) -> MappingResult {
    Some(
        (match effect.operation {
            "gegl:brightness-contrast" => direct_bc(effect),
            "gegl:exposure" => exposure(effect),
            "gegl:levels" => direct_levels(effect, context),
            "gimp:brightness-contrast" => core_bc(effect),
            "gimp:levels" => core_levels(effect),
            _ => return None,
        })
        .map_err(Into::into),
    )
}
fn allowed(effect: &StoredEffect<'_>, names: &[&str]) -> Result<()> {
    for argument in &effect.arguments {
        if !names.contains(&argument.name) {
            return Err(format!("unmapped tone argument {}", argument.name));
        }
    }
    Ok(())
}
fn bounded(name: &str, value: f64, lo: f64, hi: f64) -> Result<f64> {
    if !value.is_finite() || !(lo..=hi).contains(&value) {
        return Err(format!("{name}={value} is outside supported native conversion range {lo}..{hi}"));
    }
    Ok(value)
}
fn number(effect: &StoredEffect<'_>, name: &str, default: f64, lo: f64, hi: f64) -> Result<f64> {
    // Stock FILTER_PROP_FLOAT stores declared doubles as F32. Compare with the
    // representable stored endpoints, so a valid .1 endpoint is not rejected after rounding.
    bounded(name, effect.number(name)?.unwrap_or(default), f64::from(lo as f32), f64::from(hi as f32))
}
fn mapped(command: &str, params: Value, notes: Vec<String>) -> NativeMapping {
    NativeMapping { filters: vec![native_filter(command, params)], layer_effects: Vec::new(), notes, ..Default::default() }
}
fn direct_bc(effect: &StoredEffect<'_>) -> Result<NativeMapping> {
    allowed(effect, &["brightness", "contrast"])?;
    let brightness = number(effect, "brightness", 0.0, -3.0, 3.0)?;
    let gain = number(effect, "contrast", 1.0, -5.0, 5.0)?;
    let mut filters = Vec::new();
    if gain < 0.0 {
        filters.push(native_filter("image.adjustments.invert", json!({})));
    }
    let magnitude = gain.abs();
    let contrast = if magnitude >= 1.0 { (1.0 - magnitude.recip()) * 100.0 } else { (magnitude - 1.0) * 100.0 };
    filters.push(native_filter("image.adjustments.brightnessContrast", json!({"brightness":0.0,"contrast":contrast,"legacy":true})));
    // Native legacy brightness is additive brightness/255 with neutral contrast. A bounded
    // sequence of ordinary native filters spans the entire declared source offset domain.
    let mut offset = brightness * 255.0;
    for _ in 0..6 {
        if offset == 0.0 {
            break;
        }
        let step = offset.clamp(-150.0, 150.0);
        filters.push(native_filter("image.adjustments.brightnessContrast", json!({"brightness":step,"contrast":0.0,"legacy":true})));
        offset -= step;
    }
    if offset.abs() > f64::EPSILON {
        return Err("native brightness decomposition exceeded its fixed bound".into());
    }
    Ok(NativeMapping {filters,layer_effects:Vec::new(),notes:vec!["GEGL linear brightness/contrast becomes bounded ordinary native legacy contrast, optional Invert and additive brightness filters. Native pivot, integer intermediate clipping, quantization and document encoding differ; source opacity applies once to the whole converted stage".into()],..Default::default()})
}
fn exposure(effect: &StoredEffect<'_>) -> Result<NativeMapping> {
    allowed(effect, &["black-level", "exposure"])?;
    let exposure = number(effect, "exposure", 0.0, -20.0, 20.0)?;
    let black = number(effect, "black-level", 0.0, -0.1, 0.1)?;
    // Native Exposure adds offset after gain. Algebraically move the source black subtraction
    // across that gain; no source evaluator or working-space behavior is installed.
    let offset = -black * exposure.exp2();
    if (-0.5..=0.5).contains(&offset) {
        Ok(mapped(
            "image.adjustments.exposure",
            json!({"exposure":exposure,"offset":offset,"gamma":1.0}),
            vec!["Source black level becomes native post-gain offset; PhotoCraft depth/transfer, HDR and clipping rules remain authoritative".into()],
        ))
    } else {
        Ok(NativeMapping {filters:vec![
            native_filter("image.adjustments.exposure",json!({"exposure":0.0,"offset":-black,"gamma":1.0})),
            native_filter("image.adjustments.exposure",json!({"exposure":exposure,"offset":0.0,"gamma":1.0}))],layer_effects:Vec::new(),
            notes:vec!["Source black subtraction and exposure become two ordinary native Exposure filters to preserve offset range. Native integer intermediate clipping and transfer rules may differ; source opacity applies once to the compound stage".into()],..Default::default()})
    }
}
fn level_record(low: f64, high: f64, gamma: f64, out_low: f64, out_high: f64) -> Result<Value> {
    let low = bounded("input black", low, 0.0, 253.0 / 255.0)?;
    let high = bounded("input white", high, 2.0 / 255.0, 1.0)?;
    if high - low < 2.0 / 255.0 {
        return Err("input levels are narrower than the native two-level minimum; no endpoints were silently moved".into());
    }
    let gamma = bounded("gamma", gamma, 0.01, 9.99)?;
    let out_low = bounded("output black", out_low, 0.0, 1.0)?;
    let out_high = bounded("output white", out_high, 0.0, 1.0)?;
    Ok(json!({"inBlack":low*255.0,"inWhite":high*255.0,"gamma":gamma,"outBlack":out_low*255.0,"outWhite":out_high*255.0}))
}
fn sampled_level_record([lo, hi, gamma, outlo, outhi]: [f64; 5]) -> Result<Vec<f32>> {
    use photocraft_doc::adjust::LevelsChannel;
    for (name, value) in [("input black", lo), ("input white", hi), ("output black", outlo), ("output white", outhi)] {
        bounded(name, value, 0., 1.)?;
    }
    bounded("gamma", gamma, 0.1, 10.)?;
    let record = LevelsChannel { in_black: lo as f32, in_white: hi as f32, gamma: gamma as f32, out_black: outlo as f32, out_white: outhi as f32 };
    let values: Vec<_> = (0..256).map(|index| photocraft_compose::adjust::levels(&record, index as f32 / 255.)).collect();
    if values.iter().any(|value| !value.is_finite() || !(0. ..=1.).contains(value)) {
        return Err("Native sampled Levels produced invalid coverage/curve values".into());
    }
    Ok(values)
}
fn direct_levels(effect: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping> {
    allowed(effect, &["in-low", "in-high", "out-low", "out-high"])?;
    let record = [
        effect.number("in-low")?.unwrap_or(0.),
        effect.number("in-high")?.unwrap_or(1.),
        1.,
        effect.number("out-low")?.unwrap_or(0.),
        effect.number("out-high")?.unwrap_or(1.),
    ];
    if record.iter().any(|value| !value.is_finite()) {
        return Err("Levels endpoints must be finite".into());
    }
    if record.iter().any(|value| !(0.0..=1.0).contains(value)) || record[1] < record[0] {
        return affine_levels(record, context.mode);
    }
    if let Ok(params) = level_record(record[0], record[1], record[2], record[3], record[4]) {
        Ok(mapped("image.adjustments.levels",params,vec!["Source linear endpoint mapping uses native Levels with neutral gamma; native per-depth clamping, endpoint domain and document transfer may differ".into()]))
    } else {
        let values = sampled_level_record(record)?;
        Ok(mapped("image.adjustments.curves",json!({"pencil":true,"points":values.iter().map(|v| f64::from(*v)*255.).collect::<Vec<_>>()}),vec!["Source interval outside the native Levels command's two-level endpoint constraint becomes an ordinary native pencil Curve sampled from PhotoCraft's existing Levels transfer. Endpoints are not moved. Native clipped 0..1 curve domain, 256-sample interpolation, minimum positive span and gamma toe apply; collapsed/reversed source input intervals follow this explicit native step policy rather than source extrapolation".into()]))
    }
}

// Endpoint controls describe an affine interval, represented only by ordinary
// native matrix/Exposure data. No source range evaluator survives import.
fn affine_levels([lo, hi, _, outlo, outhi]: [f64; 5], mode: photocraft_color::ColorMode) -> Result<NativeMapping> {
    let collapsed = hi == lo;
    // The source's steep collapsed affine response is approximated by the
    // existing native Levels minimum span, without a source-specific epsilon.
    let span = if collapsed { 2. / 255. } else { hi - lo };
    let gain = (outhi - outlo) / span;
    let offset = outlo - gain * lo;
    if !gain.is_finite() || !offset.is_finite() {
        return Err("Levels affine coefficients exceed finite native F64 convolution data; source endpoints retained".into());
    }
    let mut kernel = [0.; 25];
    if let Some(center) = kernel.get_mut(12) {
        *center = gain;
    }
    let mut channels = vec![true; mode.color_channels() + 1];
    if let Some(alpha) = channels.last_mut() {
        *alpha = false;
    }
    let mut notes=vec!["Signed/HDR or reversed Levels endpoints become one ordinary editable native center-only Convolution gain/bias with color channels selected and owner alpha unchanged. Native F64 accumulation, storage-depth clipping/precision and native document-channel rules apply. This avoids the distinct native F32 Channel Mixer constant/percentage quantization law; no source range evaluator or source epsilon survives import".into()];
    if collapsed {
        notes.push("Collapsed signed/HDR input uses PhotoCraft's native Levels minimum input span 2/255 anchored at the stored low endpoint. Independent source observation establishes steep affine intent; its much higher source gain is explicitly superseded. Normalized collapsed Levels retain their existing native sampled-Curves policy".into());
    }
    Ok(mapped("filter.other.convolution", json!({"kernel":kernel,"scale":1.,"offset":offset,"channels":channels,"alphaWeighted":false,"edge":"repeat"}), notes))
}

fn scalar<'b, 'a>(values: &'b [ConfigExpr<'a>], name: &str) -> Result<&'b ConfigExpr<'a>> {
    match values {
        [value] => Ok(value),
        _ => Err(format!("tone config {name} needs one scalar")),
    }
}
fn config_number(config: &Config<'_>, name: &str, default: f64) -> Result<f64> {
    let mut fields = config.fields(name);
    let Some(value) = fields.next() else { return Ok(default) };
    if fields.next().is_some() {
        return Err(format!("duplicate tone config field {name}"));
    }
    scalar(value, name)?.as_number().ok_or_else(|| format!("tone config {name} is not numeric"))
}
fn config_fields(config: &Config<'_>, names: &[&str]) -> Result<()> {
    for form in &config.forms {
        let name = form.head().ok_or("tone configuration contains an unnamed form")?;
        if !names.contains(&name) && !["time", "name", "gimp-clip", "gimp-region", "gimp-mode", "gimp-opacity"].contains(&name) {
            return Err(format!("unmapped tone configuration field {name}"));
        }
    }
    Ok(())
}
fn core_bc(effect: &StoredEffect<'_>) -> Result<NativeMapping> {
    allowed(effect, &["config"])?;
    let config = effect.config("config")?.ok_or("missing GimpBrightnessContrastConfig")?;
    config_fields(config, &["brightness", "contrast"])?;
    let brightness = bounded("brightness", config_number(config, "brightness", 0.0)?, -1.0, 1.0)? * 100.0;
    let contrast = bounded("contrast", config_number(config, "contrast", 0.0)?, -1.0, 1.0)? * 100.0;
    let legacy = contrast < -50.0;
    Ok(mapped(
        "image.adjustments.brightnessContrast",
        json!({"brightness":brightness,"contrast":contrast,"legacy":legacy}),
        vec![format!(
            "GIMP normalized brightness/contrast becomes native percentage controls; {} native curve chosen, with native brightness/pivot/transfer differences",
            if legacy { "legacy (source contrast below modern native minimum)" } else { "modern" }
        )],
    ))
}
fn symbol<'a>(value: &'a ConfigExpr<'_>) -> Option<&'a str> {
    match value {
        ConfigExpr::Symbol(s) => Some(s),
        ConfigExpr::String(s) => Some(s),
        _ => None,
    }
}
fn core_levels(effect: &StoredEffect<'_>) -> Result<NativeMapping> {
    allowed(effect, &["trc", "config"])?;
    if effect.enum_value("trc")?.is_some_and(|value| !(0..=2).contains(&value)) {
        return Err("unknown source Levels transfer enum".into());
    }
    let config = effect.config("config")?.ok_or("missing GimpLevelsConfig")?;
    config_fields(config, &["trc", "linear", "channel", "low-input", "high-input", "gamma", "low-output", "high-output", "clamp-input", "clamp-output"])?;
    let mut source_policy = Vec::new();
    for name in ["linear", "clamp-input", "clamp-output", "trc"] {
        let mut fields = config.fields(name);
        if let Some(values) = fields.next() {
            if fields.next().is_some() {
                return Err(format!("duplicate Levels policy {name}"));
            }
            let text = symbol(scalar(values, name)?).ok_or_else(|| format!("invalid Levels policy {name}"))?;
            let allowed = if name == "trc" { &["linear", "non-linear", "perceptual"][..] } else { &["yes", "no", "true", "false"][..] };
            if !allowed.contains(&text) {
                return Err(format!("unknown Levels {name} value {text}"));
            }
            source_policy.push(format!("{name}={text}"));
        }
    }
    let mut records = [[0.0, 1.0, 1.0, 0.0, 1.0]; 5];
    let mut current = 0usize;
    let mut seen = [false; 5];
    let mut record_fields = [[false; 5]; 5];
    for form in &config.forms {
        let list = form.as_list().ok_or("levels config form")?;
        let name = form.head().ok_or("levels config name")?;
        let tail = list.get(1..).ok_or("levels config value")?;
        if name == "channel" {
            let value = scalar(tail, name)?;
            current = match symbol(value) {
                Some("value") => 0,
                Some("red") => 1,
                Some("green") => 2,
                Some("blue") => 3,
                Some("alpha") => 4,
                _ => return Err("unknown Levels channel; no enum-to-index guess".into()),
            };
            if seen.get(current).copied().unwrap_or(true) {
                return Err("duplicate Levels channel group".into());
            }
            if let Some(item) = seen.get_mut(current) {
                *item = true;
            }
        } else if let Some(index) = ["low-input", "high-input", "gamma", "low-output", "high-output"].iter().position(|&key| key == name) {
            let value = scalar(tail, name)?.as_number().ok_or("nonnumeric Levels field")?;
            bounded(name, value, if index == 2 { 0.1 } else { 0.0 }, if index == 2 { 10.0 } else { 1.0 })?;
            let fields = record_fields.get_mut(current).ok_or("Levels channel bounds")?;
            let field = fields.get_mut(index).ok_or("Levels field bounds")?;
            if *field {
                return Err(format!("duplicate Levels {name} for channel"));
            }
            *field = true;
            let record = records.get_mut(current).ok_or("Levels record bounds")?;
            let field = record.get_mut(index).ok_or("Levels value bounds")?;
            *field = value;
        }
    }
    let mut params = serde_json::Map::new();
    let mut sampled = false;
    for (index, key) in ["gray", "red", "green", "blue"].iter().enumerate() {
        let &[lo, hi, gamma, outlo, outhi] = records.get(index).ok_or("Levels record bounds")?;
        match level_record(lo, hi, gamma, outlo, outhi) {
            Ok(record) => {
                params.insert((*key).into(), record);
            }
            Err(_) => {
                sampled = true;
            }
        }
    }
    let mut filters = if sampled {
        let mut curves = serde_json::Map::new();
        curves.insert("pencil".into(), json!(true));
        for (index, key) in ["points", "red", "green", "blue"].iter().enumerate() {
            let values = sampled_level_record(*records.get(index).ok_or("Levels record bounds")?)?;
            curves.insert((*key).into(), json!(values.iter().map(|v| f64::from(*v) * 255.).collect::<Vec<_>>()));
        }
        vec![native_filter("image.adjustments.curves", Value::Object(curves))]
    } else {
        vec![native_filter("image.adjustments.levels", Value::Object(params))]
    };
    let mut notes=vec!["Native Levels preserves composite and RGB channel records with native channel order; saved source TRC/legacy linear and input/output clamp policies are retained as provenance, while native document depth/transfer/clamping rules apply".into()];
    if !source_policy.is_empty() {
        notes.push(format!("Source Levels policies: {}", source_policy.join(", ")));
    }
    if sampled {
        notes.push("Native Levels command interval/gamma limits are represented by native pencil Curves sampled from PhotoCraft Levels channel transfers, preserving channel-then-master order. Native 256-sample interpolation, clipped 0..1 domain, gamma toe and minimum span apply; collapsed/reversed intervals follow native step policy. No source evaluator is retained".into());
    }
    if let Some(record) = records.get(4).filter(|record| **record != [0., 1., 1., 0., 1.]) {
        filters.push(native_filter("filter.other.alphaCurve", json!({"values":sampled_level_record(*record)?})));
        notes.push("Source alpha Levels becomes an ordinary native Alpha Curve sampled from PhotoCraft's existing Levels transfer. Native 256-sample coverage interpolation, clamping, gamma toe and minimum positive input span apply; all existing native color/HDR data remains unchanged by the alpha stage".into());
    }
    Ok(NativeMapping { filters, notes, ..Default::default() })
}
#[cfg(test)]
mod tests {
    use super::super::{Argument, ArgumentValue};
    use super::*;
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_doc::Adjustment;
    use photocraft_geom::Rect;
    fn context() -> MappingContext<'static> {
        MappingContext {
            mode: ColorMode::Rgb,
            linear: true,
            owner_bounds: Rect::new(-2, 3, 15, 12),
            native_filter_bounds: Rect::new(0, 0, 17, 9),
            target_profile: None,
            resolution_dpi: 72.0,
            remaining_generation_bytes: crate::xcf::MAX_PIXEL_BYTES,
            remaining_metadata_bytes: crate::xcf::MAX_METADATA_BYTES,
            interrupt: photocraft_raster::Interrupt::NONE,
        }
    }
    fn arg(name: &'static str, value: f32) -> Argument<'static> {
        Argument { name, value: ArgumentValue::Float(value) }
    }
    fn config(forms: Vec<ConfigExpr<'static>>) -> Config<'static> {
        Config { raw: "independent wire forms", forms }
    }
    fn numeric(name: &'static str, value: f64) -> ConfigExpr<'static> {
        ConfigExpr::List(vec![ConfigExpr::Symbol(name), ConfigExpr::Number(value)])
    }
    fn named(name: &'static str, value: &'static str) -> ConfigExpr<'static> {
        ConfigExpr::List(vec![ConfigExpr::Symbol(name), ConfigExpr::Symbol(value)])
    }
    #[test]
    fn gegl_gain_neutral_is_not_core_contrast_neutral() {
        let source = StoredEffect::test("gegl:brightness-contrast", vec![]);
        let mapped = map(&source, &context()).unwrap().unwrap();
        assert_eq!(mapped.filters[0].command, "image.adjustments.brightnessContrast");
        assert_eq!(mapped.filters[0].params, json!({"brightness":0.0,"contrast":0.0,"legacy":true}));
        let source = StoredEffect::test("gegl:brightness-contrast", vec![arg("brightness", 0.5), arg("contrast", 1.5)]);
        let mapped = map(&source, &context()).unwrap().unwrap();
        assert!((mapped.filters[0].params["contrast"].as_f64().unwrap() - 100.0 / 3.0).abs() < 1e-10);
        assert_eq!(mapped.filters[0].params["brightness"], 0.0);
        assert_eq!(mapped.filters[1].params, json!({"brightness":127.5,"contrast":0.0,"legacy":true}));
    }
    #[test]
    fn full_gegl_gain_and_brightness_domains_use_bounded_native_stages() {
        for gain in [-5.0, -1.0, 0.0, 0.5, 1.0, 5.0] {
            for brightness in [-3.0, 0.0, 3.0] {
                let source = StoredEffect::test("gegl:brightness-contrast", vec![arg("brightness", brightness), arg("contrast", gain)]);
                let converted = map(&source, &context()).unwrap().unwrap();
                assert!(converted.filters.len() <= 8);
                assert_eq!(converted.filters[0].command == "image.adjustments.invert", gain < 0.0);
                let mut additive = 0.0;
                for filter in converted.filters.iter().filter(|filter| filter.command.ends_with("brightnessContrast")) {
                    let b = filter.params["brightness"].as_f64().unwrap();
                    let c = filter.params["contrast"].as_f64().unwrap();
                    assert!((-150.0..=150.0).contains(&b));
                    assert!((-100.0..=80.0).contains(&c));
                    additive += b / 255.0;
                }
                assert!((additive - f64::from(brightness)).abs() < 1e-12);
            }
        }
    }
    #[test]
    fn wide_postgain_offset_uses_two_valid_native_exposure_filters() {
        let source = StoredEffect::test("gegl:exposure", vec![arg("exposure", 4.0), arg("black-level", 0.1)]);
        let converted = map(&source, &context()).unwrap().unwrap();
        assert_eq!(converted.filters.len(), 2);
        assert_eq!(converted.filters[0].params["exposure"], 0.0);
        assert!((converted.filters[0].params["offset"].as_f64().unwrap() + 0.1).abs() < 1e-7);
        assert_eq!(converted.filters[1].params, json!({"exposure":4.0,"offset":0.0,"gamma":1.0}));
    }
    #[test]
    fn exposure_black_offset_matches_independently_constructed_native_hdr_adjustment() {
        let source = StoredEffect::test("gegl:exposure", vec![arg("exposure", 2.0), arg("black-level", 0.03)]);
        let mapped = map(&source, &context()).unwrap().unwrap();
        let params = &mapped.filters[0].params;
        assert_eq!(mapped.filters[0].command, "image.adjustments.exposure");
        assert_eq!(params["exposure"], 2.0);
        assert_eq!(params["gamma"], 1.0);
        assert!((params["offset"].as_f64().unwrap() + 0.12).abs() < 1e-7);
        // Independent ordinary native counterpart: gain four, post-gain offset minus .12.
        let native = Adjustment::Exposure { exposure: 2.0, offset: -0.12, gamma: 1.0 };
        let mut buffer =
            photocraft_compose::Buffer { rect: Rect::new(0, 0, 4, 1), px: vec![[0.0, 0.0, 0.0, 1.0], [0.25, 0.25, 0.25, 1.0], [1.0; 4], [2.0, 2.0, 2.0, 1.0]] };
        photocraft_compose::adjust::apply_depth(&native, &mut buffer, photocraft_compose::adjust::Transfer::Gamma(1.0), Some(SampleType::F32));
        for (pixel, expected) in buffer.px.iter().zip([-0.12, 0.88, 3.88, 7.88]) {
            for value in pixel.iter().take(3) {
                assert!((*value - expected).abs() < 1e-6);
            }
        }
        for (pixel, alpha) in buffer.px.iter().zip([1.0; 4]) {
            assert_eq!(pixel[3], alpha);
        }
    }
    #[test]
    fn endpoints_are_native_levels_not_a_source_gamma_or_clamped_range() {
        let source = StoredEffect::test("gegl:levels", vec![arg("in-low", 0.125), arg("in-high", 0.875), arg("out-low", 0.25), arg("out-high", 0.75)]);
        let mapped = map(&source, &context()).unwrap().unwrap();
        assert_eq!(mapped.filters[0].params, json!({"inBlack":31.875,"inWhite":223.125,"gamma":1.0,"outBlack":63.75,"outWhite":191.25}));
        let hdr = map(&StoredEffect::test("gegl:levels", vec![arg("out-high", 2.)]), &context()).unwrap().unwrap();
        assert_eq!(hdr.filters[0].command, "filter.other.convolution");
        assert_eq!(hdr.filters[0].params["kernel"][12], 2.);
        assert_eq!(hdr.filters[0].params["offset"], 0.);
    }
    #[test]
    fn core_bc_uses_config_and_explicit_legacy_choice_for_native_domain() {
        let source = StoredEffect::test(
            "gimp:brightness-contrast",
            vec![Argument { name: "config", value: ArgumentValue::Config(config(vec![numeric("brightness", 0.25), numeric("contrast", -0.75)])) }],
        );
        let mapped = map(&source, &context()).unwrap().unwrap();
        assert_eq!(mapped.filters[0].params, json!({"brightness":25.0,"contrast":-75.0,"legacy":true}));
        assert!(mapped.notes[0].contains("legacy"));
    }
    #[test]
    fn repeated_levels_channel_groups_preserve_native_channel_order_and_bind_native_alpha() {
        let source = StoredEffect::test(
            "gimp:levels",
            vec![Argument {
                name: "config",
                value: ArgumentValue::Config(config(vec![
                    named("channel", "value"),
                    numeric("low-input", 0.125),
                    numeric("gamma", 1.5),
                    named("channel", "red"),
                    numeric("low-output", 0.25),
                    named("channel", "green"),
                    numeric("high-output", 0.75),
                    named("channel", "blue"),
                    numeric("gamma", 2.0),
                    named("channel", "alpha"),
                    numeric("gamma", 1.25),
                ])),
            }],
        );
        let mapped = map(&source, &context()).unwrap().unwrap();
        let p = &mapped.filters[0].params;
        assert_eq!(p["gray"]["inBlack"], 31.875);
        assert_eq!(p["gray"]["gamma"], 1.5);
        assert_eq!(p["red"]["outBlack"], 63.75);
        assert_eq!(p["green"]["outWhite"], 191.25);
        assert_eq!(p["blue"]["gamma"], 2.0);
        assert!(p.get("alpha").is_none());
        assert_eq!(mapped.filters[1].command, "filter.other.alphaCurve");
        assert_eq!(mapped.filters[1].params["values"].as_array().unwrap().len(), 256);
    }
    #[test]
    fn alpha_levels_is_native_editable_coverage_at_every_depth_and_preserves_hidden_hdr_colors() {
        use photocraft_color::PixelFormat;
        use photocraft_raster::{Interrupt, Surface};
        let source = StoredEffect::test(
            "gimp:levels",
            vec![Argument {
                name: "config",
                value: ArgumentValue::Config(config(vec![
                    named("channel", "alpha"),
                    numeric("gamma", 1.25),
                    numeric("low-output", 0.1),
                    numeric("high-output", 0.9),
                ])),
            }],
        );
        let converted = map(&source, &context()).unwrap().unwrap();
        let filter = converted.filters.iter().find(|filter| filter.command == "filter.other.alphaCurve").unwrap();
        let values: Vec<f32> = filter.params["values"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
        let params = photocraft_algo::FilterParams::AlphaCurve { values };
        let bounds = Rect::new(0, 0, 4, 1);
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut surface = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
            surface.write_region(bounds, &[-2., 3., 5., 0., 0.2, 0.4, 0.6, 0.25, 0.1, 0.3, 0.7, 0.5, 0.9, 0.8, 0.7, 1.]);
            let mapped = photocraft_algo::apply_in_with(&surface, &params, bounds, bounds, None, bounds, &Interrupt::NONE).unwrap();
            for x in 0..4 {
                assert_eq!(&mapped.pixel(x, 0)[..3], &surface.pixel(x, 0)[..3]);
            }
            assert!((mapped.pixel(0, 0)[3] - 0.1).abs() < 1. / 255.);
            assert!((mapped.pixel(3, 0)[3] - 0.9).abs() < 1. / 255.);
            assert!(mapped.pixel(1, 0)[3] > surface.pixel(1, 0)[3]);
            assert!(mapped.pixel(2, 0)[3] > surface.pixel(2, 0)[3]);
        }
        // Ordinary native data, rather than a source Levels config, survives save.
        let mut doc = photocraft_doc::Document::new("Native alpha levels", photocraft_geom::Size::new(4, 1), ColorMode::Rgb, SampleType::F32);
        let contents = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let mut smart = photocraft_doc::SmartObject::new(
            photocraft_doc::SmartSource::Embedded { file_name: "contents.pcraft".into(), bytes: std::sync::Arc::new(contents) },
            photocraft_geom::Affine::IDENTITY,
            None,
        );
        smart.smart_filters.push(filter.clone());
        doc.layers.push(photocraft_doc::Layer::new("Native alpha", photocraft_doc::LayerContent::Smart(smart)));
        let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let restored = photocraft_format::load_from_bytes(&bytes).unwrap();
        let photocraft_doc::LayerContent::Smart(smart) = &restored.layers[0].content else { panic!("native smart") };
        assert_eq!(smart.smart_filters[0].params, filter.params);
    }
    #[test]
    fn narrow_collapsed_levels_and_gamma_endpoint_use_native_sampled_curves() {
        for (low, high) in [(0.5, 0.5001), (0.5, 0.5), (1., 1.)] {
            let source = StoredEffect::test("gegl:levels", vec![arg("in-low", low), arg("in-high", high)]);
            let mapping = map(&source, &context()).unwrap().unwrap();
            assert_eq!(mapping.filters[0].command, "image.adjustments.curves");
            let values = mapping.filters[0].params["points"].as_array().unwrap();
            assert_eq!(values.len(), 256);
            let master = values
                .iter()
                .enumerate()
                .map(|(index, value)| photocraft_doc::adjust::CurvePoint { input: index as f32 / 255., output: value.as_f64().unwrap() as f32 / 255. })
                .collect();
            let adjustment =
                Adjustment::Curves { master, per_channel: Default::default(), space: photocraft_doc::adjust::ToneSpace::Rgb, black: Vec::new(), pencil: true };
            let mut buffer = photocraft_compose::Buffer { rect: Rect::new(0, 0, 2, 1), px: vec![[0., 0., 0., 0.75], [1., 1., 1., 0.75]] };
            photocraft_compose::adjust::apply_depth(&adjustment, &mut buffer, photocraft_compose::adjust::Transfer::Gamma(1.), Some(SampleType::F32));
            assert_eq!(buffer.px[0], [0., 0., 0., 0.75]);
            let expected = if low == 1. { 0. } else { 1. };
            assert_eq!(buffer.px[1], [expected, expected, expected, 0.75]);
        }
        let source = StoredEffect::test(
            "gimp:levels",
            vec![Argument {
                name: "config",
                value: ArgumentValue::Config(config(vec![
                    numeric("gamma", 10.),
                    named("channel", "red"),
                    numeric("low-input", 0.5),
                    numeric("high-input", 0.5001),
                ])),
            }],
        );
        let mapped = map(&source, &context()).unwrap().unwrap();
        assert_eq!(mapped.filters[0].command, "image.adjustments.curves");
        assert_eq!(mapped.filters[0].params["red"][0], 0.);
        assert_eq!(mapped.filters[0].params["red"][255], 255.);
        assert!(mapped.notes.iter().any(|note| note.contains("channel-then-master")));
    }
    #[test]
    fn actual_config_syntax_retains_policies_and_rejects_invalid_policy_values() {
        for policy in ["(trc non-linear) (clamp-input yes) (clamp-output no)", "(linear no)"] {
            let raw = format!("{policy} (channel value) (low-input 0.125) (gamma 1.25) (channel red) (low-output 0.25)");
            let config = super::super::parse_config(&raw, &mut (32 << 20)).unwrap();
            let source = StoredEffect::test("gimp:levels", vec![Argument { name: "config", value: ArgumentValue::Config(config) }]);
            let mapped = map(&source, &context()).unwrap().unwrap();
            assert_eq!(mapped.filters[0].params["gray"]["gamma"], 1.25);
            assert_eq!(mapped.filters[0].params["red"]["outBlack"], 63.75);
            assert!(mapped.notes.iter().any(|note| note.contains("Source Levels policies")));
        }
        for raw in ["(clamp-input maybe)", "(trc mystery)", "(gamma 0.05)", "(channel red) (channel red)"] {
            let config = super::super::parse_config(raw, &mut (32 << 20)).unwrap();
            let source = StoredEffect::test("gimp:levels", vec![Argument { name: "config", value: ArgumentValue::Config(config) }]);
            assert!(map(&source, &context()).unwrap().is_err(), "{raw}");
        }
    }
    #[test]
    fn signed_hdr_and_collapsed_levels_use_native_convolution_endpoints_alpha_depth_and_reopen() {
        use photocraft_color::PixelFormat;
        use photocraft_raster::{Interrupt, Surface};
        fn options(p: &Value) -> photocraft_algo::convolution::Params {
            assert_eq!(p["edge"], "repeat");
            photocraft_algo::convolution::Params {
                kernel: p["kernel"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect(),
                scale: p["scale"].as_f64().unwrap(),
                offset: p["offset"].as_f64().unwrap(),
                channels: p["channels"].as_array().unwrap().iter().map(|v| v.as_bool().unwrap()).collect(),
                alpha_weighted: p["alphaWeighted"].as_bool().unwrap(),
                edge: photocraft_algo::convolution::Edge::Repeat,
            }
        }
        let bounds = Rect::new(0, 0, 2, 1);
        for (lo, hi, outlo, outhi) in
            [(-1., 1., -0.5, 1.5), (1., 0., 0., 1.), (0., 1., -4., 4.), (0.5, 0.5, -1., 2.), (-1., -1., -0.5, 1.5), (2., 2., -1., 2.), (0.5, 0.5, 3., -2.)]
        {
            let source = StoredEffect::test("gegl:levels", vec![arg("in-low", lo), arg("in-high", hi), arg("out-low", outlo), arg("out-high", outhi)]);
            let span = if lo == hi { 2. / 255. } else { f64::from(hi - lo) };
            let gain = f64::from(outhi - outlo) / span;
            let offset = f64::from(outlo) - gain * f64::from(lo);
            for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
                let mut ctx = context();
                ctx.mode = mode;
                let native = map(&source, &ctx).unwrap().unwrap();
                if lo == hi {
                    assert!(native.notes.iter().any(|note| note.contains("minimum input span 2/255")));
                }
                assert_eq!(native.filters.len(), 1);
                let filter = &native.filters[0];
                assert_eq!(filter.command, "filter.other.convolution");
                let options = options(&filter.params);
                let mut kernel = vec![0.; 25];
                kernel[12] = gain;
                let mut channels = vec![true; mode.color_channels() + 1];
                *channels.last_mut().unwrap() = false;
                let independent = photocraft_algo::convolution::Params {
                    kernel,
                    scale: 1.,
                    offset,
                    channels,
                    alpha_weighted: false,
                    edge: photocraft_algo::convolution::Edge::Repeat,
                };
                assert_eq!(options, independent);
                for depth in SampleType::ALL {
                    let mut input = Surface::new(PixelFormat::new(mode, depth, true));
                    let mut samples = Vec::new();
                    for value in [lo, lo + span as f32] {
                        samples.extend(vec![value; mode.color_channels()]);
                        samples.push(0.375);
                    }
                    input.write_region(bounds, &samples);
                    let actual = photocraft_algo::convolution::apply_with(&input, &options, bounds, bounds, None, &Interrupt::NONE).unwrap();
                    let expected = photocraft_algo::convolution::apply_with(&input, &independent, bounds, bounds, None, &Interrupt::NONE).unwrap();
                    assert_eq!(actual, expected);
                    for x in 0..2 {
                        assert_eq!(actual.pixel(x, 0).last(), input.pixel(x, 0).last());
                    }
                    if depth == SampleType::F32 {
                        for (x, endpoint) in [outlo, outhi].into_iter().enumerate() {
                            for channel in actual.pixel(x as i32, 0).iter().take(mode.color_channels()) {
                                assert!((*channel - endpoint).abs() < 0.001, "{lo}: expected{endpoint}, got{channel}");
                            }
                        }
                    }
                    let mut document = photocraft_doc::Document::new("Native affine interval", photocraft_geom::Size::new(2, 1), mode, depth);
                    document.layers.push(photocraft_doc::Layer::new("Original", photocraft_doc::LayerContent::Raster(input)));
                    let bytes = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
                    let mut object = photocraft_doc::SmartObject::new(
                        photocraft_doc::SmartSource::Embedded { file_name: "native.pcraft".into(), bytes: std::sync::Arc::new(bytes) },
                        photocraft_geom::Affine::IDENTITY,
                        None,
                    );
                    object.smart_filters = native.filters.clone();
                    document.layers.push(photocraft_doc::Layer::new("Native affine convolution", photocraft_doc::LayerContent::Smart(object)));
                    let bytes = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
                    let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
                    let photocraft_doc::LayerContent::Smart(object) = &reopened.layers[1].content else { panic!("native filter") };
                    assert_eq!(object.smart_filters, native.filters);
                }
            }
        }
        let failure = affine_levels([0., f64::MIN_POSITIVE, 1., 0., f64::MAX], ColorMode::Rgb).unwrap_err();
        assert!(failure.contains("finite native F64 convolution data"));
        let wide = affine_levels([0., 1., 1., 0., f64::from(f32::MAX)], ColorMode::Rgb).unwrap();
        assert_eq!(wide.filters.len(), 1);
        let options = options(&wide.filters[0].params);
        assert!(photocraft_algo::convolution::validate(&options, 4).is_ok());
    }

    #[test]
    fn subnormal_f32_levels_endpoint_uses_finite_f64_kernel_without_false_range_rejection() {
        use photocraft_color::PixelFormat;
        use photocraft_raster::{Interrupt, Surface};
        let tiny = f32::from_bits(1);
        let bounds = Rect::new(0, 0, 2, 1);
        let source = StoredEffect::test("gegl:levels", vec![arg("in-low", 0.), arg("in-high", tiny), arg("out-low", 0.), arg("out-high", 2.)]);
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            let mut ctx = context();
            ctx.mode = mode;
            let native = map(&source, &ctx).unwrap().unwrap();
            let p = &native.filters[0].params;
            assert_eq!(native.filters[0].command, "filter.other.convolution");
            let coefficient = p["kernel"][12].as_f64().unwrap();
            assert!(coefficient > f64::from(f32::MAX) && coefficient.is_finite());
            assert_eq!(coefficient, 2. / f64::from(tiny));
            let mut kernel = vec![0.; 25];
            kernel[12] = 2. / f64::from(tiny);
            let mut channels = vec![true; mode.color_channels() + 1];
            *channels.last_mut().unwrap() = false;
            let options = photocraft_algo::convolution::Params {
                kernel,
                scale: p["scale"].as_f64().unwrap(),
                offset: p["offset"].as_f64().unwrap(),
                channels,
                alpha_weighted: p["alphaWeighted"].as_bool().unwrap(),
                edge: photocraft_algo::convolution::Edge::Repeat,
            };
            let mut input = Surface::new(PixelFormat::new(mode, SampleType::F32, true));
            let mut pixels = vec![0.; mode.color_channels()];
            pixels.push(0.375);
            pixels.extend(vec![tiny; mode.color_channels()]);
            pixels.push(0.75);
            input.write_region(bounds, &pixels);
            let actual = photocraft_algo::convolution::apply_with(&input, &options, bounds, bounds, None, &Interrupt::NONE).unwrap();
            for (x, expected, alpha) in [(0, 0., 0.375), (1, 2., 0.75)] {
                let pixel = actual.pixel(x, 0);
                assert!(pixel.iter().take(mode.color_channels()).all(|value| (*value - expected).abs() < 1e-6), "{mode:?}: {pixel:?}");
                assert_eq!(pixel.last(), Some(&alpha));
            }
            // The ordinary native parameter JSON retains its exact finite F64
            // coefficient beyond F32, without source-specific serialization.
            let encoded = serde_json::to_vec(p).unwrap();
            let reopened: Value = serde_json::from_slice(&encoded).unwrap();
            assert_eq!(reopened["kernel"][12], p["kernel"][12]);
        }
    }

    #[test]
    fn unsafe_or_unmapped_params_are_retained_instead_of_native_clamping() {
        for source in [
            StoredEffect::test("gegl:brightness-contrast", vec![arg("contrast", -6.0)]),
            StoredEffect::test("gegl:exposure", vec![arg("exposure", 21.0)]),
            StoredEffect::test("gegl:exposure", vec![arg("black-level", 0.2)]),
            StoredEffect::test("gegl:exposure", vec![arg("exposure", f32::NAN)]),
            StoredEffect::test("gegl:exposure", vec![arg("exposure", 1.0), arg("exposure", 2.0)]),
            StoredEffect::test("gegl:levels", vec![arg("made-up-gamma", 2.0)]),
        ] {
            assert!(map(&source, &context()).unwrap().is_err());
        }
        assert!(map(&StoredEffect::test("gegl:other", vec![]), &context()).is_none());
    }
}
