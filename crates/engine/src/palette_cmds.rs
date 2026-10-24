//! Shared palette and mapping adapters for Indexed Color and indexed exports.
//!
//! This module owns command-facing compatibility and the Indexed format's palette rules. The
//! actual mapping kernel is [`photocraft_algo::palette_map`]; callers exchange ordered encoded-sRGB
//! palettes and floating-point RGBA pixels with that kernel. Palette Map's surface/ICC adapter is
//! intentionally implemented by its later feature change, not here.

use photocraft_algo::palette_map::{self, AlphaMode, Method, Palette, Settings};
use photocraft_doc::ColorTable;
use serde_json::{Map, Value};

use crate::{EngineError, Result};

const ERROR_COMMAND: &str = "palette.mapping";
const INDEXED_ID: &str = "image.mode.indexedColor";

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: ERROR_COMMAND.to_string(), msg: msg.into() }
}

fn command_params(p: &Value) -> Result<&Map<String, Value>> {
    p.as_object().ok_or_else(|| bad("params must be a JSON object"))
}

/// Resolve a palette snapshot or bundled catalog ID.
///
/// A snapshot is authoritative and is checked before `paletteId`, so a saved operation remains
/// deterministic if its catalog entry is later removed or changed. Neither field is valid when it
/// has the wrong JSON type; callers must not silently fall back to generated colours.
pub fn resolve_palette(p: &Value) -> Result<Option<Palette>> {
    let object = command_params(p)?;
    if let Some(snapshot) = object.get("paletteSnapshot")
        && !snapshot.is_null()
    {
        let palette: Palette = serde_json::from_value(snapshot.clone()).map_err(|e| bad(format!("paletteSnapshot is not a valid palette: {e}")))?;
        palette.validate().map_err(|e| bad(format!("invalid paletteSnapshot: {e}")))?;
        return Ok(Some(palette));
    }
    let Some(value) = object.get("paletteId").filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let id = value.as_str().ok_or_else(|| bad("paletteId must be a string"))?;
    if id.is_empty() {
        return Err(bad("paletteId must not be empty"));
    }
    let palette =
        photocraft_algo::palettes::builtin(id).ok_or_else(|| bad(format!("unknown paletteId `{id}`; choose a bundled palette or provide paletteSnapshot")))?;
    let palette = palette.palette.clone();
    palette.validate().map_err(|e| bad(format!("bundled palette `{id}` is invalid: {e}")))?;
    Ok(Some(palette))
}

fn legacy_method(p: &Map<String, Value>) -> Result<Method> {
    if p.get("noDither").and_then(Value::as_bool) == Some(true) {
        return Ok(Method::None);
    }
    let name = p.get("dither").and_then(Value::as_str).unwrap_or("diffusion");
    let method = match name {
        "none" | "noDither" => Method::None,
        "diffusion" => Method::FloydSteinberg,
        "pattern" => Method::Bayer,
        "noise" => Method::WhiteNoise,
        other => Method::from_id(other)
            .ok_or_else(|| bad(format!("dither/method `{other}` is not supported; use none, diffusion, pattern, noise, or a palette-map method id")))?,
    };
    Ok(method)
}

fn legacy_amount(p: &Map<String, Value>, amount_key: &str) -> Result<f32> {
    let value = p
        .get(amount_key)
        .filter(|value| !value.is_null())
        .or_else(|| p.get("amount").filter(|value| !value.is_null()))
        .or_else(|| p.get("ditherAmount").filter(|value| !value.is_null()));
    let Some(value) = value else {
        return Ok(if amount_key == "ditherAmount" { 0.88 } else { 0.75 });
    };
    let number = value.as_f64().ok_or_else(|| bad(format!("{amount_key} must be a finite number from 0 to 100")))?;
    if !number.is_finite() || !(0.0..=100.0).contains(&number) {
        return Err(bad(format!("{amount_key} must be a finite number from 0 to 100")));
    }
    Ok((number as f32 / 100.0).clamp(0.0, 1.0))
}

/// Decode mapping settings shared by Indexed Color and Save for Web.
///
/// A serialized `mapping` object wins over legacy `dither` and percentage amount fields. Legacy
/// spellings are translated to the one palette-map kernel (diffusion → Floyd–Steinberg, pattern →
/// Bayer and noise → White Noise).
pub fn mapping_settings(p: &Value, amount_key: &str) -> Result<Settings> {
    let object = command_params(p)?;
    if let Some(mapping) = object.get("mapping")
        && !mapping.is_null()
    {
        let settings: Settings = serde_json::from_value(mapping.clone()).map_err(|e| bad(format!("mapping is not valid palette-map settings: {e}")))?;
        settings.validate().map_err(|e| bad(format!("invalid mapping: {e}")))?;
        return Ok(settings);
    }
    let settings = Settings { method: legacy_method(object)?, amount: legacy_amount(object, amount_key)?, ..Settings::default() };
    settings.validate().map_err(|e| bad(format!("invalid mapping: {e}")))?;
    Ok(settings)
}

/// Reduce a palette only when a caller explicitly requests it.
///
/// Reduction uses the deterministic adaptive palette builder used by Indexed Color; it is not a
/// slice/truncate operation, so every source entry participates, including arbitrary custom
/// snapshot colours.
pub fn reduce_palette(palette: &Palette, capacity: usize) -> Result<Palette> {
    palette.validate().map_err(|e| bad(format!("invalid palette: {e}")))?;
    if capacity == 0 || capacity > 256 {
        return Err(bad("palette reduction capacity must be from 1 to 256"));
    }
    if palette.colors.len() <= capacity {
        return Ok(palette.clone());
    }
    if capacity == 1 {
        let mut sum = [0.0f64; 3];
        for color in &palette.colors {
            for (acc, value) in sum.iter_mut().zip(color) {
                *acc += f64::from(value.clamp(0.0, 1.0));
            }
        }
        let count = palette.colors.len() as f64;
        return Ok(Palette { name: palette.name.clone(), colors: vec![sum.map(|v| (v / count) as f32)] });
    }
    let samples: Vec<[f32; 4]> = palette.colors.iter().map(|c| [c[0], c[1], c[2], 1.0]).collect();
    let reduced =
        photocraft_algo::quantize::build_palette(&samples, photocraft_algo::quantize::PaletteKind::Adaptive, capacity, photocraft_algo::quantize::Forced::None)
            .map_err(|e| bad(format!("could not reduce palette: {e}")))?;
    let out = Palette { name: palette.name.clone(), colors: reduced.into_iter().map(|c| c.map(|v| f32::from(v) / 255.0)).collect() };
    out.validate().map_err(|e| bad(format!("reduced palette is invalid: {e}")))?;
    Ok(out)
}

/// Resolve a palette carried by Indexed Color parameters, preserving its command-specific errors.
pub(crate) fn palette_from_snapshot_for_indexed(p: &Value) -> Result<Option<Palette>> {
    resolve_palette(p).map_err(|error| match error {
        EngineError::BadParams { msg, .. } => EngineError::BadParams { cmd: INDEXED_ID.to_string(), msg },
        other => other,
    })
}

/// Build a palette suitable for Indexed Color's 256-entry table.
///
/// A fixed palette is preserved unless reduction is explicitly requested. With an explicit
/// transparent entry, reduction reserves one slot for that entry and appends it after the opaque
/// reduction so indexed exporters receive a stable marker.
pub fn indexed_palette(palette: Palette, reduce: bool, transparent: Option<usize>) -> Result<(Palette, Option<usize>)> {
    palette.validate().map_err(|e| bad(format!("invalid palette: {e}")))?;
    let capacity = 256usize;
    if palette.colors.len() <= capacity && transparent.is_none() {
        return Ok((palette, None));
    }
    let Some(index) = transparent else {
        if palette.colors.len() > capacity && !reduce {
            return Err(bad(format!(
                "fixed palette has {} entries; Indexed Color supports at most 256 (including transparency). Enable reducePalette for deliberate reduction",
                palette.colors.len()
            )));
        }
        return Ok((if reduce { reduce_palette(&palette, capacity)? } else { palette }, None));
    };
    if index >= palette.colors.len() {
        return Err(bad(format!("transparentIndex {index} is outside the {}-entry palette", palette.colors.len())));
    }
    if palette.colors.len() > capacity && !reduce {
        return Err(bad(format!(
            "fixed palette has {} entries including transparent index {index}; Indexed Color supports at most 256. Enable reducePalette for deliberate reduction",
            palette.colors.len()
        )));
    }
    if !reduce && palette.colors.len() <= capacity {
        return Ok((palette, Some(index)));
    }
    let transparent_color = palette.colors.get(index).copied().ok_or_else(|| bad("transparentIndex is outside the palette"))?;
    let opaque =
        Palette { name: palette.name.clone(), colors: palette.colors.into_iter().enumerate().filter_map(|(i, c)| (i != index).then_some(c)).collect() };
    let mut reduced = reduce_palette(&opaque, capacity - 1)?;
    reduced.colors.push(transparent_color);
    let transparent = reduced.colors.len() - 1;
    Ok((reduced, Some(transparent)))
}

/// Append a duplicate final colour as the transparent slot when one table entry remains available.
pub(crate) fn append_transparency(palette: &mut Palette) -> Option<usize> {
    if palette.colors.len() >= 256 {
        return None;
    }
    let colour = palette.colors.last().copied().unwrap_or([0.0; 3]);
    palette.colors.push(colour);
    Some(palette.colors.len() - 1)
}

/// Indexed output cannot preserve fractional alpha when a transparent slot exists.
pub(crate) fn alpha_for_indexed(settings: &mut Settings, transparent: Option<usize>) {
    if transparent.is_some() && matches!(settings.alpha, AlphaMode::Preserve) {
        settings.alpha = AlphaMode::Binary;
    }
}

/// Convert an encoded-sRGB palette to the document's indexed colour-table bytes.
pub(crate) fn palette_to_color_table(palette: &Palette) -> ColorTable {
    ColorTable { colors: palette.colors.iter().map(|c| c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)).collect(), transparent: None }
}

/// Map one complete row-major rectangle through the shared palette kernel.
pub(crate) fn indexed_render(px: &mut [[f32; 4]], width: usize, palette: &Palette, settings: &Settings, transparent: Option<usize>) -> Result<()> {
    palette_map::render_pixels(px, width, palette, settings, [0, 0], transparent)
        .map_err(|e| EngineError::BadParams { cmd: INDEXED_ID.to_string(), msg: format!("indexed palette mapping failed: {e}") })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn mapping_legacy_spellings_are_shared() {
        let settings = mapping_settings(&json!({"dither": "pattern", "amount": 20}), "amount").ok();
        assert_eq!(settings.as_ref().map(|s| (s.method, s.amount)), Some((Method::Bayer, 0.2)));
    }

    #[test]
    fn snapshot_wins_over_unknown_catalog_id() {
        let palette = Palette { name: "test".into(), colors: vec![[0.0, 0.0, 0.0], [1.0, 1.0, 1.0]] };
        let value = json!({"paletteId": "gimp:does-not-exist", "paletteSnapshot": palette});
        assert_eq!(resolve_palette(&value).ok().flatten(), Some(palette));
    }

    #[test]
    fn indexed_reduction_rejects_out_of_bounds_transparent_slot() {
        let palette = Palette { name: "Two colours".into(), colors: vec![[0.0; 3], [1.0; 3]] };
        let error = indexed_palette(palette, true, Some(2)).unwrap_err().to_string();
        assert!(error.contains("transparentIndex") && error.contains("outside"), "{error}");
    }

    #[test]
    fn wrong_type_palette_id_does_not_fall_back_to_generated_indexing() {
        let mut session = crate::Session::new();
        session.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        let before = session.active().unwrap().revision;
        let error = session.execute(INDEXED_ID, json!({"paletteId": 42})).unwrap_err().to_string();
        assert!(error.contains("paletteId"), "{error}");
        assert_eq!(session.active().unwrap().revision, before);
        assert_eq!(session.active().unwrap().doc.mode, photocraft_color::ColorMode::Rgb);
    }
}
