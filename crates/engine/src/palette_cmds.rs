//! Palette Map's engine adapters.
//!
//! This module owns command-facing compatibility and the Indexed format's palette rules. The
//! actual mapping kernel is [`photocraft_algo::palette_map`]; callers exchange ordered encoded-sRGB
//! palettes and floating-point RGBA pixels with that kernel. Palette Map stays separate from the
//! tiled FilterParams pipeline; its adapters own command decoding, palette snapshots and
//! conversion back to the document's native model/depth. Indexed advanced mapping performs its
//! document-profile/sRGB adaptation at the command boundary; stock quantization does not.

use std::sync::Arc;

use photocraft_algo::palette_map::{self, AlphaMode, Method, Palette, Settings};
use photocraft_cms::{Builtin, Profile};
use photocraft_doc::{ColorTable, LayerContent, LayerId};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Map, Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

pub const FILTER_ID: &str = "filter.paletteMap";
const INDEXED_ID: &str = "image.mode.indexedColor";

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.to_string(), msg: msg.into() }
}

const FIXED_MODE_UNSUPPORTED: &str = "Palette Map is not supported for Indexed Color or Bitmap documents";

fn check_document_mode(s: &Session) -> Result<()> {
    if s.active().is_some_and(|state| matches!(state.doc.mode, photocraft_color::ColorMode::Indexed | photocraft_color::ColorMode::Bitmap)) {
        return Err(EngineError::Disabled(FILTER_ID.into(), FIXED_MODE_UNSUPPORTED.into()));
    }
    Ok(())
}
const ERROR_COMMAND: &str = "palette.mapping";

fn shared_bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: ERROR_COMMAND.to_string(), msg: msg.into() }
}

fn as_filter_error(error: EngineError) -> EngineError {
    match error {
        EngineError::BadParams { msg, .. } => bad(FILTER_ID, msg),
        other => other,
    }
}

fn filter_params(p: &Value) -> Result<&Map<String, Value>> {
    command_params(p).map_err(as_filter_error)
}

fn command_params(p: &Value) -> Result<&Map<String, Value>> {
    p.as_object().ok_or_else(|| shared_bad("params must be a JSON object"))
}
const MAX_SOURCE_PROFILE_BYTES: usize = 4 * 1024 * 1024;
const MAX_WORK_BYTES: usize = if cfg!(target_pointer_width = "64") { 8 << 30 } else { 512 << 20 };

/// Parse the optional ICC snapshot carried by a saved Palette Map filter.
///
/// Profile bytes are intentionally part of the params rather than an ambient document lookup:
/// smart-filter replay has only its cached surface, and an edit to the document profile must not
/// silently change an existing effect. The bounded JSON byte array also keeps hostile records from
/// allocating an unbounded profile.
fn source_profile_bytes_from_params(p: &Value) -> Result<Option<Arc<Vec<u8>>>> {
    let object = filter_params(p)?;
    let Some(value) = object.get("sourceProfile").filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let bytes = value.as_array().ok_or_else(|| bad(FILTER_ID, "sourceProfile must be an ICC byte array"))?;
    if bytes.len() > MAX_SOURCE_PROFILE_BYTES {
        return Err(bad(FILTER_ID, format!("sourceProfile exceeds the {} MiB limit", MAX_SOURCE_PROFILE_BYTES / (1024 * 1024))));
    }
    let mut raw = Vec::with_capacity(bytes.len());
    for (index, value) in bytes.iter().enumerate() {
        let byte = value
            .as_u64()
            .and_then(|value| u8::try_from(value).ok())
            .ok_or_else(|| bad(FILTER_ID, format!("sourceProfile byte {index} must be an integer from 0 to 255")))?;
        raw.push(byte);
    }
    Ok(Some(Arc::new(raw)))
}

fn source_profile_from_params(p: &Value) -> Result<Option<Arc<Profile>>> {
    let Some(raw) = source_profile_bytes_from_params(p)? else {
        return Ok(None);
    };
    crate::color_cmds::profile_from_bytes(&raw).map(Some).map_err(|error| bad(FILTER_ID, format!("sourceProfile is not a valid ICC profile: {error}")))
}

/// Resolve the explicit palette carried by a command.
///
/// `paletteSnapshot` is authoritative and is intentionally checked before `paletteId`: a saved
/// smart filter therefore remains deterministic even when a catalog entry changes or disappears.
/// When neither field is present this returns `Ok(None)` so generated Indexed Color palettes can
/// continue to use their historical `palette` selector.
pub fn resolve_palette(p: &Value) -> Result<Option<Palette>> {
    let object = command_params(p)?;
    let palette_id = object.get("paletteId").filter(|value| !value.is_null());
    if let Some(value) = palette_id {
        let id = value.as_str().ok_or_else(|| shared_bad("paletteId must be a string"))?;
        if id.is_empty() {
            return Err(shared_bad("paletteId must not be empty"));
        }
    }
    if let Some(snapshot) = object.get("paletteSnapshot")
        && !snapshot.is_null()
    {
        let palette: Palette = serde_json::from_value(snapshot.clone()).map_err(|e| shared_bad(format!("paletteSnapshot is not a valid palette: {e}")))?;
        palette.validate().map_err(|e| shared_bad(format!("invalid paletteSnapshot: {e}")))?;
        return Ok(Some(palette));
    }
    let Some(value) = palette_id else {
        return Ok(None);
    };
    let id = value.as_str().ok_or_else(|| shared_bad("paletteId must be a string"))?;
    let palette = photocraft_algo::palettes::builtin(id)
        .ok_or_else(|| shared_bad(format!("unknown paletteId `{id}`; choose a bundled palette or provide paletteSnapshot")))?;
    let palette = palette.palette.clone();
    palette.validate().map_err(|e| shared_bad(format!("bundled palette `{id}` is invalid: {e}")))?;
    Ok(Some(palette))
}

fn legacy_method(p: &Map<String, Value>) -> Result<Method> {
    if let Some(value) = p.get("noDither").filter(|value| !value.is_null())
        && value.as_bool().ok_or_else(|| shared_bad("noDither must be a boolean"))?
    {
        return Ok(Method::None);
    }
    let name = match p.get("dither").filter(|value| !value.is_null()) {
        None => "diffusion",
        Some(value) => value.as_str().ok_or_else(|| shared_bad("dither must be a string"))?,
    };
    let method = match name {
        "none" | "noDither" => Method::None,
        "diffusion" => Method::FloydSteinberg,
        "pattern" => Method::Bayer,
        "noise" => Method::WhiteNoise,
        other => Method::from_id(other)
            .ok_or_else(|| shared_bad(format!("dither/method `{other}` is not supported; use none, diffusion, pattern, noise, or a palette-map method id")))?,
    };
    Ok(method)
}

fn legacy_amount(p: &Map<String, Value>, amount_key: &str) -> Result<f32> {
    for key in [amount_key, "amount", "ditherAmount"] {
        if let Some(value) = p.get(key).filter(|value| !value.is_null()) {
            let number = value.as_f64().ok_or_else(|| shared_bad(format!("{key} must be a finite number from 0 to 100")))?;
            if !number.is_finite() || !(0.0..=100.0).contains(&number) {
                return Err(shared_bad(format!("{key} must be a finite number from 0 to 100")));
            }
        }
    }
    let value = p
        .get(amount_key)
        .filter(|value| !value.is_null())
        .or_else(|| p.get("amount").filter(|value| !value.is_null()))
        .or_else(|| p.get("ditherAmount").filter(|value| !value.is_null()));
    let Some(value) = value else {
        return Ok(if amount_key == "ditherAmount" { 0.88 } else { 0.75 });
    };
    let number = value.as_f64().ok_or_else(|| shared_bad(format!("{amount_key} must be a finite number from 0 to 100")))?;
    Ok((number as f32 / 100.0).clamp(0.0, 1.0))
}

/// Decode the shared mapping settings used by Indexed Color, Palette Map and Save for Web.
///
/// A serialized `mapping` object wins over legacy `dither` plus the percentage amount field.  If
/// no mapping is supplied, the old command spellings remain accepted and are translated to the
/// single palette-map kernel (diffusion → Floyd–Steinberg, pattern → Bayer, noise → White Noise).
pub fn mapping_settings(p: &Value, amount_key: &str) -> Result<Settings> {
    let object = command_params(p)?;
    if let Some(mapping) = object.get("mapping")
        && !mapping.is_null()
    {
        let settings: Settings = serde_json::from_value(mapping.clone()).map_err(|e| shared_bad(format!("mapping is not valid palette-map settings: {e}")))?;
        settings.validate().map_err(|e| shared_bad(format!("invalid mapping: {e}")))?;
        return Ok(settings);
    }
    let settings = Settings { method: legacy_method(object)?, amount: legacy_amount(object, amount_key)?, ..Settings::default() };
    settings.validate().map_err(|e| shared_bad(format!("invalid mapping: {e}")))?;
    Ok(settings)
}

/// Reduce a palette only when a caller has explicitly requested reduction.
///
/// The reduction uses the same deterministic adaptive palette builder used by Indexed Color.  It
/// is deliberately not a slice/truncate operation: every source entry participates in the
/// reduction, including custom snapshots with arbitrary floating-point colours.
pub fn reduce_palette(palette: &Palette, capacity: usize) -> Result<Palette> {
    palette.validate().map_err(|e| shared_bad(format!("invalid palette: {e}")))?;
    if capacity == 0 || capacity > 256 {
        return Err(shared_bad("palette reduction capacity must be from 1 to 256"));
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
            .map_err(|e| shared_bad(format!("could not reduce palette: {e}")))?;
    let out = Palette { name: palette.name.clone(), colors: reduced.into_iter().map(|c| c.map(|v| f32::from(v) / 255.0)).collect() };
    out.validate().map_err(|e| shared_bad(format!("reduced palette is invalid: {e}")))?;
    Ok(out)
}

fn palette_snapshot_value(palette: &Palette) -> Result<Value> {
    serde_json::to_value(palette).map_err(|e| bad(FILTER_ID, format!("could not store palette snapshot: {e}")))
}

/// Add the active document ICC bytes to a new Smart Filter record when they are a valid
/// mode-matching profile. Invalid/mismatched legacy tags continue to use the mode's working
/// profile instead of making a command unusable.
fn add_document_profile(params: &mut Value, doc: &photocraft_doc::Document) -> Result<()> {
    let Value::Object(object) = params else { return Err(bad(FILTER_ID, "normalised filter params are not an object")) };
    if object.contains_key("sourceProfile") {
        return Ok(());
    }
    let Some(bytes) = doc.icc_profile.as_ref() else { return Ok(()) };
    if bytes.len() > MAX_SOURCE_PROFILE_BYTES {
        return Err(bad(FILTER_ID, format!("document ICC profile exceeds the {} MiB limit", MAX_SOURCE_PROFILE_BYTES / (1024 * 1024))));
    }
    let raw = Arc::new(bytes.clone());
    let Some(profile) = crate::color_cmds::profile_from_bytes(&raw).ok() else { return Ok(()) };
    if profile.color_space != crate::color_cmds::mode_space(doc.mode) {
        return Ok(());
    }
    object.insert("sourceProfile".into(), serde_json::to_value(bytes.as_slice()).map_err(|e| bad(FILTER_ID, format!("could not store source profile: {e}")))?);
    Ok(())
}

/// Fill in deterministic records before a Palette Map smart filter is stored.
fn normalized_filter_params(p: &Value) -> Result<(Value, Palette, Settings)> {
    let _ = source_profile_from_params(p)?;
    let palette = resolve_palette(p).map_err(as_filter_error)?.ok_or_else(|| bad(FILTER_ID, "paletteSnapshot or paletteId is required"))?;
    let settings = mapping_settings(p, "amount").map_err(as_filter_error)?;
    let mut params = filter_params(p)?.clone();
    params.insert("paletteSnapshot".into(), palette_snapshot_value(&palette)?);
    params.insert("mapping".into(), serde_json::to_value(&settings).map_err(|e| bad(FILTER_ID, format!("could not store mapping: {e}")))?);
    Ok((Value::Object(params), palette, settings))
}

pub(crate) fn palette_from_snapshot_for_indexed(p: &Value) -> Result<Option<Palette>> {
    resolve_palette(p).map_err(|e| match e {
        EngineError::BadParams { msg, .. } => bad(INDEXED_ID, msg),
        other => other,
    })
}

/// Build a palette suitable for Indexed Color, preserving a fixed palette unless reduction was
/// explicitly requested. `transparent` is the caller's explicit transparent slot, if any.
pub fn indexed_palette(palette: Palette, reduce: bool, transparent: Option<usize>) -> Result<(Palette, Option<usize>)> {
    palette.validate().map_err(|e| shared_bad(format!("invalid palette: {e}")))?;
    let capacity = 256usize;
    if palette.colors.len() <= capacity && transparent.is_none() {
        return Ok((palette, None));
    }
    let Some(index) = transparent else {
        if palette.colors.len() > capacity && !reduce {
            return Err(shared_bad(format!(
                "fixed palette has {} entries; Indexed Color supports at most 256 (including transparency). Enable reducePalette for deliberate reduction",
                palette.colors.len()
            )));
        }
        return Ok((if reduce { reduce_palette(&palette, capacity)? } else { palette }, None));
    };
    if index >= palette.colors.len() {
        return Err(shared_bad(format!("transparentIndex {index} is outside the {}-entry palette", palette.colors.len())));
    }
    if palette.colors.len() > capacity && !reduce {
        return Err(shared_bad(format!(
            "fixed palette has {} entries including transparent index {index}; Indexed Color supports at most 256. Enable reducePalette for deliberate reduction",
            palette.colors.len()
        )));
    }
    if !reduce && palette.colors.len() <= capacity {
        return Ok((palette, Some(index)));
    }
    let transparent_color = palette.colors.get(index).copied().ok_or_else(|| shared_bad("transparentIndex is outside the palette"))?;
    let opaque =
        Palette { name: palette.name.clone(), colors: palette.colors.into_iter().enumerate().filter_map(|(i, c)| (i != index).then_some(c)).collect() };
    let mut reduced = reduce_palette(&opaque, capacity - 1)?;
    reduced.colors.push(transparent_color);
    let transparent = reduced.colors.len() - 1;
    Ok((reduced, Some(transparent)))
}

pub(crate) fn append_transparency(palette: &mut Palette) -> Option<usize> {
    if palette.colors.len() >= 256 {
        return None;
    }
    // The RGB triplet in a transparent slot is never displayed; using the final opaque colour
    // keeps the table stable for formats that do not carry a separate transparent marker.
    let c = palette.colors.last().copied().unwrap_or([0.0; 3]);
    palette.colors.push(c);
    Some(palette.colors.len() - 1)
}
pub(crate) fn alpha_for_indexed(settings: &mut Settings, transparent: Option<usize>) {
    if transparent.is_some() && matches!(settings.alpha, AlphaMode::Preserve) {
        settings.alpha = AlphaMode::Binary;
    }
}

pub(crate) fn palette_to_color_table(palette: &Palette) -> ColorTable {
    ColorTable { colors: palette.colors.iter().map(|c| c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)).collect(), transparent: None }
}

/// Render one complete rectangle through the palette-map kernel, then encode it back to the
/// surface's original native format and depth. Patterns use the rectangle's document-space origin
/// so moving the preview viewport does not move the dither.
pub(crate) fn render_surface(
    surface: &Surface,
    area: Rect,
    palette: &Palette,
    settings: &Settings,
    source_profile: Option<&Profile>,
    selection: Option<&Surface>,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<Surface> {
    if cancelled() {
        return Err(EngineError::Cancelled);
    }
    if area.is_empty() {
        return Ok(surface.clone());
    }
    palette.validate().map_err(|e| bad(FILTER_ID, format!("invalid palette: {e}")))?;
    settings.validate().map_err(|e| bad(FILTER_ID, format!("invalid mapping: {e}")))?;
    let count = usize::try_from(area.width())
        .ok()
        .and_then(|w| usize::try_from(area.height()).ok().and_then(|h| w.checked_mul(h)))
        .ok_or_else(|| bad(FILTER_ID, "palette-map region is too large"))?;
    let fmt = surface.format();
    let channels = fmt.channels();
    let colour_channels = fmt.mode.color_channels();
    if channels < colour_channels {
        return Err(bad(FILTER_ID, "surface format has fewer channels than its colour model"));
    }
    if colour_channels == 0 || colour_channels > 16 || channels > 16 {
        return Err(bad(FILTER_ID, "Palette Map supports native colour models with 1 to 16 channels"));
    }
    if selection.is_some_and(|mask| mask.channels() == 0 || mask.channels() > 16) {
        return Err(bad(FILTER_ID, "invalid selection mask channel count"));
    }
    let fallback = source_profile.is_none().then(|| crate::color_cmds::working_profile(fmt.mode));
    let profile = source_profile.or(fallback.as_deref()).ok_or_else(|| bad(FILTER_ID, "no source colour profile is available"))?;
    let expected = crate::color_cmds::mode_space(fmt.mode);
    if profile.color_space != expected {
        return Err(bad(FILTER_ID, format!("source profile is {:?}, but the surface needs {:?}", profile.color_space, expected)));
    }
    let srgb = Builtin::Srgb.profile();
    let identity = fmt.mode == photocraft_color::ColorMode::Rgb && profile.content_hash() == srgb.content_hash();
    let transforms = if identity {
        None
    } else {
        let to = photocraft_cms::transform::cached(profile, srgb, Default::default())
            .map_err(|error| bad(FILTER_ID, format!("could not convert the source profile to sRGB: {error}")))?;
        let from = photocraft_cms::transform::cached(srgb, profile, Default::default())
            .map_err(|error| bad(FILTER_ID, format!("could not convert sRGB to the source profile: {error}")))?;
        if to.inputs() != colour_channels || from.outputs() != colour_channels {
            return Err(bad(FILTER_ID, "the source colour model does not match its ICC transform"));
        }
        Some((to, from))
    };
    let encoded_bytes = if identity && fmt.alpha { 0 } else { channels * std::mem::size_of::<f32>() };
    let selection_bytes = selection.map_or(0, |mask| 16 + mask.channels() * std::mem::size_of::<f32>());
    let retained_native_bytes = if transforms.is_some() && selection.is_some() { encoded_bytes } else { 0 };
    let bytes_per_pixel = 16 + encoded_bytes.max(retained_native_bytes + selection_bytes);
    if count.checked_mul(bytes_per_pixel).is_none_or(|bytes| bytes > MAX_WORK_BYTES) {
        return Err(bad(
            FILTER_ID,
            format!("palette-map region exceeds the {} MiB working-memory limit; use a smaller layer or selection", MAX_WORK_BYTES / (1024 * 1024)),
        ));
    }
    let mut mapped = Vec::new();
    mapped.try_reserve_exact(count).map_err(|error| bad(FILTER_ID, format!("could not allocate palette-map pixels: {error}")))?;
    mapped.resize(count, [0.0f32; 4]);
    let mut native = None;
    if let Some((to, _)) = &transforms {
        let raw = surface.try_read_region(area).map_err(|error| bad(FILTER_ID, error.to_string()))?;
        for (index, (src, dst)) in raw.chunks_exact(channels).zip(&mut mapped).enumerate() {
            if index.is_multiple_of(256) && cancelled() {
                return Err(EngineError::Cancelled);
            }
            let mut rgb = [0.0f32; 16];
            to.eval(&src[..colour_channels], &mut rgb);
            dst[..3].copy_from_slice(&rgb[..3]);
            dst[3] = if fmt.alpha { src.get(colour_channels).copied().unwrap_or(1.0) } else { 1.0 };
        }
        if selection.is_some() {
            native = Some(raw);
        }
    } else {
        surface.read_rgba_into(area, &mut mapped);
    }
    let original = if selection.is_some() {
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(count).map_err(|error| bad(FILTER_ID, format!("could not allocate selected palette-map pixels: {error}")))?;
        pixels.extend_from_slice(&mapped);
        Some(pixels)
    } else {
        None
    };
    if cancelled() {
        return Err(EngineError::Cancelled);
    }
    palette_map::render_pixels_with(&mut mapped, area.width() as usize, palette, settings, [area.x0, area.y0], None, cancelled)
        .map_err(|e| if cancelled() { EngineError::Cancelled } else { bad(FILTER_ID, format!("palette map failed: {e}")) })?;
    let selection_values = selection.map(|mask| mask.try_read_region(area).map_err(|error| bad(FILTER_ID, error.to_string()))).transpose()?;
    if let (Some(selection), Some(original), Some(values)) = (selection, original, selection_values.as_ref()) {
        let selection_channels = selection.channels();
        for (index, ((out, old), coverage)) in
            mapped.iter_mut().zip(original).zip(values.chunks_exact(selection_channels).map(|v| v.first().copied().unwrap_or(0.0))).enumerate()
        {
            if index.is_multiple_of(256) && cancelled() {
                return Err(EngineError::Cancelled);
            }
            let k = coverage.clamp(0.0, 1.0);
            if k >= 1.0 {
                continue;
            }
            if k <= 0.0 {
                *out = old;
                continue;
            }
            let oa = old[3].clamp(0.0, 1.0);
            let na = out[3].clamp(0.0, 1.0);
            let alpha = oa + (na - oa) * k;
            let mut rgb = [0.0; 3];
            for c in 0..3 {
                let premul = old[c] * oa + (out[c] * na - old[c] * oa) * k;
                rgb[c] = if alpha > 1e-7 { premul / alpha } else { 0.0 };
            }
            *out = [rgb[0], rgb[1], rgb[2], alpha];
        }
    }
    if cancelled() {
        return Err(EngineError::Cancelled);
    }
    let mut result = surface.clone();
    if identity && fmt.alpha {
        drop(selection_values);
        result.try_write_region(area, mapped.as_flattened()).map_err(|error| bad(FILTER_ID, error.to_string()))?;
    } else {
        let length = count.checked_mul(channels).ok_or_else(|| bad(FILTER_ID, "palette-map output is too large"))?;
        let mut native_coverage =
            if native.is_some() { selection_values.as_deref().map(|values| values.chunks_exact(selection.map_or(1, Surface::channels))) } else { None };
        let mut encoded = if let Some(native) = native {
            native
        } else {
            let mut encoded = Vec::new();
            encoded.try_reserve_exact(length).map_err(|error| bad(FILTER_ID, format!("could not allocate palette-map output: {error}")))?;
            encoded.resize(length, 0.0f32);
            encoded
        };
        for (index, (rgba, out)) in mapped.iter().zip(encoded.chunks_exact_mut(channels)).enumerate() {
            if index.is_multiple_of(256) && cancelled() {
                return Err(EngineError::Cancelled);
            }
            // Keep native samples in selection holes; even an ICC round trip can alter them.
            if native_coverage.as_mut().and_then(|values| values.next()).and_then(|coverage| coverage.first()).is_some_and(|coverage| *coverage <= 0.0) {
                continue;
            }
            if let Some((_, from)) = &transforms {
                let mut native = [0.0f32; 16];
                from.eval(&rgba[..3], &mut native);
                out[..colour_channels].copy_from_slice(&native[..colour_channels]);
            } else {
                out[..3].copy_from_slice(&rgba[..3]);
            }
            if fmt.alpha {
                out[colour_channels] = rgba[3];
            }
        }
        drop(selection_values);
        result.try_write_region(area, &encoded).map_err(|error| bad(FILTER_ID, error.to_string()))?;
    }
    if cancelled() {
        return Err(EngineError::Cancelled);
    }
    result.prune();
    Ok(result)
}

fn filter_area(surface: &Surface, selection: Option<&Surface>) -> Rect {
    let content = surface.content_bounds();
    selection.map_or(content, |s| content.intersect(&s.content_bounds()))
}

/// Dedicated Palette Map command path. It shares the usual job, selection, lock and smart-filter
/// lifecycle but never enters the tiled `FilterParams` renderer.
pub(crate) fn run_filter(s: &mut Session, p: &Value) -> Result<Value> {
    check_document_mode(s)?;
    let (mut params, palette, settings) = normalized_filter_params(p)?;
    let requested_profile = source_profile_from_params(&params)?;
    let doc_profile = {
        let active = s.active().ok_or(EngineError::NoDocument)?;
        if requested_profile.is_none() {
            add_document_profile(&mut params, &active.doc)?;
        }
        crate::color_cmds::document_profile(&active.doc)
    };
    let source_profile = requested_profile.unwrap_or(doc_profile);
    let layer = match p.get("layer").and_then(Value::as_u64) {
        Some(id) => LayerId(id),
        None => s.active().and_then(|d| d.active_layer).ok_or(EngineError::Other("no active layer".into()))?,
    };
    let layer_id = layer.0;
    let palette_name = palette.name.clone();
    let result_settings = settings.clone();
    let label = "Palette Map".to_string();
    crate::jobs::edit_job(
        s,
        &label,
        move |doc, _, ctx| {
            let cmyk = photocraft_compose::cmyk_space(doc);
            photocraft_color::convert::with_cmyk_space(cmyk.as_ref(), || {
                let selection = doc.selection.clone();
                if let Some(target) = crate::channel_cmds::channel_surface_for_filter(doc, Some(layer), &params)? {
                    let area = filter_area(target, selection.as_ref());
                    let target_profile = if source_profile.color_space == crate::color_cmds::mode_space(target.format().mode) {
                        source_profile.clone()
                    } else {
                        crate::color_cmds::working_profile(target.format().mode)
                    };
                    *target = render_surface(target, area, &palette, &settings, Some(target_profile.as_ref()), selection.as_ref(), &|| ctx.cancelled())?;
                    return Ok(());
                }
                let area = filter_area(
                    doc.layer(layer).and_then(|l| l.surface()).ok_or_else(|| EngineError::Other("Palette Map needs a pixel layer".into()))?,
                    selection.as_ref(),
                );
                let locks = doc.effective_locks(layer);
                let l = doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?;
                if locks.pixels || locks.all {
                    return Err(EngineError::Other(format!("Could not complete your request because the layer \"{}\" is locked", l.name)));
                }
                if locks.transparency && !matches!(settings.alpha, AlphaMode::Preserve) && matches!(&l.content, LayerContent::Raster(_)) {
                    return Err(EngineError::Other(format!("Could not complete your request because the transparency of layer \"{}\" is locked", l.name)));
                }
                match &mut l.content {
                    LayerContent::Raster(surface) => {
                        let before = locks.transparency.then(|| surface.clone());
                        let out = render_surface(surface, area, &palette, &settings, Some(source_profile.as_ref()), selection.as_ref(), &|| ctx.cancelled())?;
                        *surface = out;
                        if let Some(before) = &before {
                            crate::filters::keep_alpha(before, surface, &|| ctx.cancelled())?;
                        }
                        Ok(())
                    }
                    LayerContent::Smart(_) => {
                        let sf = photocraft_doc::SmartFilter {
                            command: FILTER_ID.into(),
                            params: params.clone(),
                            blend: photocraft_color::BlendMode::Normal,
                            opacity: 1.0,
                            visible: true,
                        };
                        crate::smart_cmds::add_smart_filter(doc, layer, sf, selection.as_ref(), Some(ctx))
                    }
                    _ => Err(EngineError::Other("Palette Map needs a pixel layer".into())),
                }
            })?;
            Ok(())
        },
        move |_| json!({"layer": layer_id, "filter": FILTER_ID, "palette": palette_name, "mapping": result_settings}),
    )
}

/// Smart-filter replay adapter. It returns a complete replacement surface so callers can keep the
/// same unknown-filter semantics as the ordinary filter path.
pub(crate) fn apply_to_surface(
    params: &Value,
    surface: &Surface,
    selection: Option<&Surface>,
    canvas: Rect,
    extent: Rect,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<Surface> {
    if matches!(surface.format().mode, photocraft_color::ColorMode::Indexed | photocraft_color::ColorMode::Bitmap) {
        return Err(bad(FILTER_ID, "Palette Map cannot replay on an Indexed Color or Bitmap surface"));
    }
    let palette = resolve_palette(params).map_err(as_filter_error)?.ok_or_else(|| bad(FILTER_ID, "paletteSnapshot or paletteId is required"))?;
    let settings = mapping_settings(params, "amount").map_err(as_filter_error)?;
    let source_profile = source_profile_from_params(params)?;
    let area = filter_area(surface, selection).intersect(&extent).intersect(&canvas.union(&surface.content_bounds()));
    render_surface(surface, area, &palette, &settings, source_profile.as_deref(), selection, cancelled)
}

/// Indexed Color uses the same float/CMS kernel as Palette Map while retaining its expanded RGB
/// storage and ColorTable index plane for existing import/export adapters.
pub(crate) fn indexed_render(px: &mut [[f32; 4]], width: usize, palette: &Palette, settings: &Settings, transparent: Option<usize>) -> Result<()> {
    palette_map::render_pixels(px, width, palette, settings, [0, 0], transparent).map_err(|e| bad(INDEXED_ID, format!("indexed palette mapping failed: {e}")))
}

fn enabled(s: &Session) -> std::result::Result<(), String> {
    crate::filters::has_filterable_layer(s)?;
    let doc = s.active().ok_or("no document open")?;
    if matches!(doc.doc.mode, photocraft_color::ColorMode::Indexed | photocraft_color::ColorMode::Bitmap) {
        return Err(FIXED_MODE_UNSUPPORTED.into());
    }
    Ok(())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: FILTER_ID,
        label: "Palette Map…",
        menu: &["Filter", "PhotoCraft"],
        shortcut: None,
        params: r##"{"paletteId":string?,"paletteSnapshot":json?,"mapping":json?,"dither":"none|diffusion|pattern|noise"="diffusion","amount":0..100=75} (full-region palette mapping; mapping overrides legacy dither/amount)"##,
        enabled,
        run: |s, p| run_filter(s, p),
        journal: true,
    }]
}

pub(crate) fn validate_params(p: &Value) -> Result<()> {
    let _ = filter_params(p)?;
    let _ = resolve_palette(p).map_err(as_filter_error)?;
    let _ = mapping_settings(p, "amount").map_err(as_filter_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapping_legacy_spellings_are_shared() {
        let settings = mapping_settings(&json!({"dither": "pattern", "amount": 20}), "amount").ok();
        assert_eq!(settings.as_ref().map(|s| (s.method, s.amount)), Some((Method::Bayer, 0.2)));
    }

    #[test]
    fn snapshot_wins_over_unknown_catalog_id() {
        let palette = Palette { name: "test".into(), colors: vec![[0.0, 0.0, 0.0], [1.0, 1.0, 1.0]] };
        let value = json!({"paletteId": "missing-palette", "paletteSnapshot": palette});
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
        let mut session = Session::new();
        session.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        let before = session.active().unwrap().revision;
        let error = session.execute(INDEXED_ID, json!({"paletteId": 42})).unwrap_err().to_string();
        assert!(error.contains("paletteId"), "{error}");
        assert_eq!(session.active().unwrap().revision, before);
        assert_eq!(session.active().unwrap().doc.mode, photocraft_color::ColorMode::Rgb);
    }

    #[test]
    fn palette_map_is_disabled_for_fixed_color_documents() {
        for (mode_command, mode, new_params) in [
            (INDEXED_ID, photocraft_color::ColorMode::Indexed, json!({})),
            ("image.mode.bitmap", photocraft_color::ColorMode::Bitmap, json!({"method": "threshold"})),
        ] {
            let mut session = Session::new();
            let mode_params = if mode == photocraft_color::ColorMode::Bitmap {
                json!({"width": 8, "height": 8, "mode": "grayscale"})
            } else {
                json!({"width": 8, "height": 8})
            };
            session.execute("file.new", mode_params).unwrap();
            session.execute(mode_command, new_params).unwrap();
            let before = session.active().unwrap().doc.color_table.clone();

            assert!(!session.is_enabled(FILTER_ID), "{mode:?}");
            let error =
                session.execute(FILTER_ID, json!({"paletteSnapshot": {"name": "two", "colors": [[0.0, 0.0, 0.0], [1.0, 1.0, 1.0]]}})).unwrap_err().to_string();
            assert!(error.contains("Indexed Color or Bitmap"), "{mode:?}: {error}");
            let document = &session.active().unwrap().doc;
            assert_eq!(document.mode, mode);
            assert_eq!(document.color_table, before);
        }
    }

    #[test]
    fn palette_map_replay_rejects_fixed_color_surfaces() {
        let params = json!({"paletteSnapshot": {"name": "two", "colors": [[0.0, 0.0, 0.0], [1.0, 1.0, 1.0]]}});
        for mode in [photocraft_color::ColorMode::Indexed, photocraft_color::ColorMode::Bitmap] {
            let surface = Surface::new(photocraft_color::PixelFormat::new(mode, photocraft_color::SampleType::U8, true));
            let error = crate::filters::apply_filter_to_surface(FILTER_ID, &params, &surface, Rect::new(0, 0, 8, 8), None, Rect::new(0, 0, 8, 8), &|| false)
                .unwrap_err()
                .to_string();
            assert!(error.contains("Indexed Color or Bitmap"), "{mode:?}: {error}");
        }
    }

    #[test]
    fn oversized_palette_region_returns_error_before_allocating() {
        let surface = Surface::new(photocraft_color::PixelFormat::new(photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8, true));
        let palette = Palette { name: "Binary".into(), colors: vec![[0.0; 3], [1.0; 3]] };
        let result = render_surface(&surface, Rect::new(0, 0, i32::MAX, i32::MAX), &palette, &Settings::default(), None, None, &|| false);
        assert!(result.is_err(), "hostile sparse-layer bounds must not attempt an unbounded allocation");
    }

    #[test]
    fn palette_map_cancels_before_source_conversion_and_during_icc_work() {
        use photocraft_color::{ColorMode, PixelFormat, SampleType};
        use std::sync::atomic::{AtomicUsize, Ordering};

        let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
        let area = Rect::new(0, 0, 128, 64);
        let mut surface = Surface::new(fmt);
        surface.fill_rect(area, &[0.4, 0.3, 0.2, 0.75]);
        let before = surface.try_read_region(area).unwrap();
        let palette = Palette { name: "Original binary".into(), colors: vec![[0.0; 3], [1.0; 3]] };
        let profile = Builtin::DisplayP3.profile();

        for cancel_after in [0, 2, 5] {
            let calls = AtomicUsize::new(0);
            let cancelled = || calls.fetch_add(1, Ordering::SeqCst) >= cancel_after;
            let result = render_surface(&surface, area, &palette, &Settings::default(), Some(profile), None, &cancelled);
            assert!(matches!(result, Err(EngineError::Cancelled)), "cancellation after {cancel_after} checks");
            assert_eq!(surface.try_read_region(area).unwrap(), before, "cancelled work cannot mutate the source");
        }
    }

    #[test]
    fn malformed_palette_id_is_rejected_even_when_snapshot_is_present() {
        let palette = Palette { name: "test".into(), colors: vec![[0.0; 3], [1.0; 3]] };
        let error = resolve_palette(&json!({"paletteId": 42, "paletteSnapshot": palette})).unwrap_err().to_string();
        assert!(error.contains("paletteId must be a string"), "{error}");
    }

    #[test]
    fn malformed_legacy_dither_does_not_fall_back_to_stock_mapping() {
        let error = mapping_settings(&json!({"dither": 42}), "amount").unwrap_err().to_string();
        assert!(error.contains("dither must be a string"), "{error}");
        let error = mapping_settings(&json!({"noDither": "yes"}), "amount").unwrap_err().to_string();
        assert!(error.contains("noDither must be a boolean"), "{error}");
    }
}
