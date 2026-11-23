//! Optional private preparation of ordinary native raster Smart Objects.
//! No source operation is evaluated and no owner is mutated by this module.
use super::{Mapping, MaskRequirement};
use craft_xcf::effects::{Owner, OwnerKind};
use photocraft_color::{BlendMode, ColorMode, SampleType};
use photocraft_compose::{
    Buffer,
    adjust::{self, Transfer},
};
use photocraft_doc::{
    Affine, Document, Layer, LayerContent, LayerMask, SmartFilter, SmartObject, SmartSource,
    adjust::{Adjustment, LevelsChannel, ToneSpace},
};
use photocraft_geom::{Rect, Size, TILE_SIZE};
use photocraft_raster::{Interrupt, Surface};
use std::sync::Arc;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub(in crate::xcf) enum Refusal {
    #[error("cancelled")]
    Cancelled,
    #[error("native raster stage is not admitted: {0}")]
    Unsupported(&'static str),
    #[error("native raster stage allowance exhausted")]
    Limit,
    #[error("native raster preparation failed: {0}")]
    Preparation(&'static str),
}
type Result<T> = std::result::Result<T, Refusal>;

/// Per-import counters supplied by the importer. Byte-work is prepaid and never
/// refunded (including late failure/cancellation). Retained storage is committed
/// only after complete private preparation; these counters are not process RSS.
pub(in crate::xcf) struct Budgets<'a> {
    pub retained: &'a mut usize,
    pub work: &'a mut usize,
}

pub(in crate::xcf) struct PreparedRaster {
    pub source_owner: Owner,
    pub smart: SmartObject,
    pub notes: Vec<&'static str>,
}

fn check(ctl: &Interrupt<'_>) -> Result<()> {
    ctl.check().map_err(|_| Refusal::Cancelled)
}
fn add(a: usize, b: usize) -> Result<usize> {
    a.checked_add(b).ok_or(Refusal::Limit)
}
fn mul(a: usize, b: usize) -> Result<usize> {
    a.checked_mul(b).ok_or(Refusal::Limit)
}

pub(in crate::xcf) struct ResolvedMask {
    pub source_offset: u64,
    pub mask: LayerMask,
}
pub(in crate::xcf) struct MaskedExposure<'a> {
    pub stage: &'a Mapping,
    pub coverage: ResolvedMask,
}
enum Plan<'a> {
    Plain(&'a [Mapping]),
    Masked(MaskedExposure<'a>),
}

pub(in crate::xcf) fn prepare_masked_exposure(
    source_owner: Owner,
    owner: &Layer,
    bounds: Rect,
    document: &Document,
    exposure: MaskedExposure<'_>,
    budgets: &mut Budgets<'_>,
    ctl: &Interrupt<'_>,
) -> Result<PreparedRaster> {
    prepare_plan(source_owner, owner, bounds, document, Plan::Masked(exposure), budgets, ctl)
}

/// Keep unsupported settings unapplied. A future compound/mask installer can
/// extend this subset; flattening a partial envelope into every filter is wrong.
/// `bounds` is the authoritative drawable extent from its source header, not the
/// canvas or a report identity. `stages` is already in native execution order.
pub(in crate::xcf) fn prepare(
    source_owner: Owner,
    owner: &Layer,
    bounds: Rect,
    document: &Document,
    stages: &[Mapping],
    budgets: &mut Budgets<'_>,
    ctl: &Interrupt<'_>,
) -> Result<PreparedRaster> {
    prepare_plan(source_owner, owner, bounds, document, Plan::Plain(stages), budgets, ctl)
}

fn prepare_plan(
    source_owner: Owner,
    owner: &Layer,
    bounds: Rect,
    document: &Document,
    plan: Plan<'_>,
    budgets: &mut Budgets<'_>,
    ctl: &Interrupt<'_>,
) -> Result<PreparedRaster> {
    let (stages, coverage) = match plan {
        Plan::Plain(stages) => (stages, None),
        Plan::Masked(MaskedExposure { stage, coverage }) => (std::slice::from_ref(stage), Some(coverage)),
    };
    check(ctl)?;
    #[cfg(any(test, feature = "testgen"))]
    crate::native_xcf_tones::note_preparation();
    let LayerContent::Raster(source) = &owner.content else {
        return Err(Refusal::Unsupported("only raster owners"));
    };
    if !matches!(source_owner.kind, OwnerKind::Layer) || stages.is_empty() || stages.len() > 32 {
        return Err(Refusal::Unsupported("owner or stage count"));
    }
    let format = source.format();
    if !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale) || format.mode != document.mode || format.sample != document.depth {
        return Err(Refusal::Unsupported("native RGB/Gray document format must match source"));
    }
    if let Some(mask) = &coverage {
        let stage = stages.first().ok_or(Refusal::Unsupported("one masked Exposure"))?;
        if format.mode != ColorMode::Rgb
            || !matches!(format.sample, SampleType::U8 | SampleType::U16)
            || !format.alpha
            || stage.filters.len() != 1
            || stage.filters.first().is_none_or(|filter| filter.command != "image.adjustments.exposure")
            || stage.envelope.mask != MaskRequirement::Required(mask.source_offset)
            || mask.mask.surface.format().mode != ColorMode::Grayscale
            || mask.mask.surface.format().sample != SampleType::F32
            || mask.mask.surface.channels() != 1
            || !mask.mask.enabled
            || mask.mask.density != 1.
            || mask.mask.feather != 0.
        {
            return Err(Refusal::Unsupported("only proven U8/U16 opaque Exposure coverage"));
        }
    }
    let width = usize::try_from(i64::from(bounds.x1) - i64::from(bounds.x0)).map_err(|_| Refusal::Limit)?;
    let height = usize::try_from(i64::from(bounds.y1) - i64::from(bounds.y0)).map_err(|_| Refusal::Limit)?;
    if width == 0 || height == 0 || width > craft_xcf::MAX_SIDE as usize || height > craft_xcf::MAX_SIDE as usize {
        return Err(Refusal::Unsupported("finite nonempty owner extent"));
    }
    let local = Rect::new(0, 0, width as i32, height as i32);
    let mut filters = Vec::new();
    let mut adjustments = Vec::new();
    let mut notes = Vec::new();
    for stage in stages {
        check(ctl)?;
        if stage.owner != source_owner
            || stage.envelope.blend != BlendMode::Normal
            || (!stage.envelope.opacity.is_finite() || !(0.0..=1.0).contains(&stage.envelope.opacity))
            || (coverage.is_none() && (stage.envelope.opacity != 1. || stage.envelope.mask != MaskRequirement::None))
        {
            return Err(Refusal::Unsupported("source owner or compound envelope requires later installation"));
        }
        if stage.filters.is_empty() || stage.filters.len() > 8 || stage.notes.len() > 1 {
            return Err(Refusal::Unsupported("bounded mapped tone sequence"));
        }
        for filter in &stage.filters {
            if filter.blend != BlendMode::Normal || filter.opacity != 1. || !filter.visible {
                return Err(Refusal::Unsupported("mapped controls must be full-strength"));
            }
            // Integer Levels constructs depth LUTs per apply_depth call. This
            // row-bounded installer has not admitted that extra allocation/work.
            // RGB/F32 uses the existing direct scalar path; other contexts keep
            // their original pixels until a reusable priced native LUT seam exists.
            if filter.command == "image.adjustments.levels" && (format.mode != ColorMode::Rgb || format.sample != SampleType::F32) {
                return Err(Refusal::Unsupported("native Levels preparation is proven only for RGB/F32"));
            }
            if super::point_color::native(&filter.command) && format.mode != ColorMode::Rgb {
                return Err(Refusal::Unsupported("point-color preparation is proven only for RGB owners"));
            }
            let adjustment = native_adjustment(filter)?;
            let mut filter = filter.clone();
            filter.visible = stage.envelope.visible;
            if coverage.is_some() {
                filter.opacity = stage.envelope.opacity;
            }
            filters.push(filter);
            adjustments.push((adjustment, stage.envelope.visible));
        }
        notes.extend_from_slice(&stage.notes);
    }
    let profile_bytes = document.icc_profile.as_ref().map_or(0, |profile| profile.len());
    if profile_bytes > 4 << 20 {
        return Err(Refusal::Limit);
    }
    if !document.resolution_dpi.is_finite() || document.resolution_dpi <= 0. || document.resolution_dpi > 1_000_000. {
        return Err(Refusal::Unsupported("bounded native profile/resolution"));
    }
    let pixels = mul(width, height)?;
    let row_bytes = mul(width, add(mul(format.channels(), 4)?, 16)?)?;
    let local_tiles = usize::try_from(local.tile_count()).map_err(|_| Refusal::Limit)?;
    let cache_tiles = usize::try_from(bounds.tile_count()).map_err(|_| Refusal::Limit)?;
    // XCF source tiles are 64px, but retained native tiles use TILE_SIZE.
    let tile_pixels = mul(TILE_SIZE as usize, TILE_SIZE as usize)?;
    let tile_bytes = mul(tile_pixels, format.bytes_per_pixel())?;
    let local_bytes = mul(local_tiles, add(tile_bytes, 512)?)?;
    let cache_bytes = mul(cache_tiles, add(tile_bytes, 512)?)?;
    // Only a newly constructed one-layer native document is serialized. Reserve
    // two raw payload envelopes plus 32KiB per tile and fixed metadata/profile
    // room before invoking the existing writer. This is admission for this
    // bounded representation, not a claim about the writer's complete peak RSS.
    let encoded_bound = add(add(mul(local_bytes, 2)?, mul(local_tiles, 32 * 1024)?)?, add(1 << 20, mul(profile_bytes, 2)?)?)?;
    let extra = if coverage.is_some() { add(mul(cache_bytes, 2)?, mul(tile_pixels, 4 * 4 * 4)?)? } else { 0 };
    let temporary = add(add(add(local_bytes, cache_bytes)?, add(encoded_bound, row_bytes)?)?, extra)?;
    if temporary > crate::xcf::MAX_PIXEL_BYTES || temporary > *budgets.retained {
        return Err(Refusal::Limit);
    }
    let mix_work = if coverage.is_some() { mul(pixels, 128)? } else { 0 };
    let work = add(add(mul(pixels, mul(add(adjustments.len(), 4)?, 16)?)?, mul(encoded_bound, 2)?)?, mix_work)?;
    // Caller counters can be stricter, never expand the importer's absolute
    // retained/work ceilings even if accidentally initialized to usize::MAX.
    if work > crate::xcf::MAX_PIXEL_BYTES.saturating_mul(8) {
        return Err(Refusal::Limit);
    }
    match budgets.work.checked_sub(work) {
        Some(left) => *budgets.work = left,
        None => {
            *budgets.work = 0;
            return Err(Refusal::Limit);
        }
    }
    let mut normalized = Surface::new(format);
    let mut cache = Surface::new(format);
    let mut row = Vec::new();
    row.try_reserve_exact(mul(width, format.channels())?).map_err(|_| Refusal::Limit)?;
    row.resize(mul(width, format.channels())?, 0.);
    let mut rgba = Vec::new();
    rgba.try_reserve_exact(width).map_err(|_| Refusal::Limit)?;
    rgba.resize(width, [0.; 4]);
    let mut buffer = Buffer { rect: Rect::new(0, 0, width as i32, 1), px: rgba };
    for y in 0..height {
        check(ctl)?;
        let source_y = bounds.y0.checked_add(y as i32).ok_or(Refusal::Limit)?;
        for (x, pixel) in buffer.px.iter_mut().enumerate() {
            let source_x = bounds.x0.checked_add(x as i32).ok_or(Refusal::Limit)?;
            let start = mul(x, format.channels())?;
            let values = row.get_mut(start..add(start, format.channels())?).ok_or(Refusal::Limit)?;
            source.read_pixel(source_x, source_y, values);
            if values.iter().any(|value| !value.is_finite()) {
                return Err(Refusal::Preparation("nonfinite source sample"));
            }
            if coverage.is_some() && values.last().copied() != Some(1.) {
                return Err(Refusal::Unsupported("partial-alpha masked Exposure is not proven"));
            }
            *pixel = photocraft_raster::to_rgba(&format, values);
        }
        normalized.try_write_region(Rect::from_xywh(0, y as i32, width as u32, 1), &row).map_err(|_| Refusal::Limit)?;
        for (adjustment, visible) in &adjustments {
            check(ctl)?;
            if !visible {
                continue;
            }
            adjust::apply_depth(adjustment, &mut buffer, Transfer::for_document(document.mode, document.depth), Some(document.depth));
            // Ordinary native filter stages quantize to the owner's depth.
            for (x, pixel) in buffer.px.iter_mut().enumerate() {
                if pixel.iter().any(|value| !value.is_finite()) {
                    return Err(Refusal::Preparation("nonfinite native output"));
                }
                let mut values = [0f32; 4];
                photocraft_raster::from_rgba_into(&format, *pixel, &mut values);
                let mut encoded = [0u8; 16];
                photocraft_raster::encode_pixel(&format, values.get(..format.channels()).ok_or(Refusal::Limit)?, &mut encoded);
                for (channel, value) in values.iter_mut().take(format.channels()).enumerate() {
                    *value = photocraft_color::read_sample(&encoded, format.sample, channel);
                }
                *pixel = photocraft_raster::to_rgba(&format, &values);
                let start = mul(x, format.channels())?;
                row.get_mut(start..add(start, format.channels())?)
                    .ok_or(Refusal::Limit)?
                    .copy_from_slice(values.get(..format.channels()).ok_or(Refusal::Limit)?);
            }
        }
        cache.try_write_region(Rect::from_xywh(bounds.x0, source_y, width as u32, 1), &row).map_err(|_| Refusal::Limit)?;
    }
    let mut contents = Document::new("Imported raster contents", Size::new(width as u32, height as u32), document.mode, document.depth);
    contents.resolution_dpi = document.resolution_dpi;
    contents.icc_profile = document.icc_profile.clone();
    contents.layers.push(Layer::new("Original pixels", LayerContent::Raster(normalized)));
    check(ctl)?;
    let bytes = photocraft_format::save_to_bytes(&contents, &Default::default()).map_err(|_| Refusal::Preparation("native bundle serialization"))?;
    check(ctl)?;
    if bytes.capacity() > encoded_bound {
        return Err(Refusal::Limit);
    }
    if let Some(resolved) = &coverage {
        let opacity = stages.first().ok_or(Refusal::Unsupported("one masked Exposure"))?.envelope.opacity;
        cache = photocraft_compose::smart_mix::mix(source, &cache, bounds, BlendMode::Normal, opacity, ctl).map_err(|error| {
            if matches!(error, photocraft_compose::smart_mix::Error::Cancelled) { Refusal::Cancelled } else { Refusal::Preparation("native strength mixing") }
        })?;
        check(ctl)?;
        cache = photocraft_compose::filter_mask::mix_in(source, &cache, &resolved.mask, bounds, ctl).map_err(|error| {
            if matches!(error, photocraft_compose::filter_mask::MixError::Cancelled) {
                Refusal::Cancelled
            } else {
                Refusal::Preparation("native coverage mixing")
            }
        })?;
        check(ctl)?;
    }
    let retained = add(bytes.capacity(), cache_bytes)?;
    let remaining = budgets.retained.checked_sub(retained).ok_or(Refusal::Limit)?;
    let mut smart = SmartObject::new(
        SmartSource::Embedded { file_name: "xcf-raster-contents.pcraft".into(), bytes: Arc::new(bytes) },
        Affine::translate(f64::from(bounds.x0), f64::from(bounds.y0)),
        Some(cache),
    );
    smart.smart_filters = filters;
    smart.filter_mask = coverage.map(|resolved| resolved.mask);
    check(ctl)?;
    *budgets.retained = remaining;
    Ok(PreparedRaster { source_owner, smart, notes })
}

fn native_adjustment(filter: &SmartFilter) -> Result<Adjustment> {
    let fields = filter.params.as_object().ok_or(Refusal::Unsupported("native parameter object"))?;
    let number = |name: &str, low: f64, high: f64| -> Result<f32> {
        let value = fields
            .get(name)
            .and_then(serde_json::Value::as_f64)
            .filter(|value| value.is_finite() && (low..=high).contains(value))
            .ok_or(Refusal::Unsupported("native scalar domain"))?;
        Ok(value as f32)
    };
    match filter.command.as_str() {
        "image.adjustments.invert" if fields.is_empty() => Ok(Adjustment::Invert),
        "image.adjustments.brightnessContrast" if fields.len() == 3 && fields.get("legacy").and_then(serde_json::Value::as_bool) == Some(true) => {
            Ok(Adjustment::BrightnessContrast { brightness: number("brightness", -150., 150.)?, contrast: number("contrast", -100., 100.)?, legacy: true })
        }
        "image.adjustments.exposure" if fields.len() == 3 && fields.get("gamma").and_then(serde_json::Value::as_f64) == Some(1.) => {
            Ok(Adjustment::Exposure { exposure: number("exposure", -20., 20.)?, offset: number("offset", -0.5, 0.5)?, gamma: 1. })
        }
        "image.adjustments.levels" if fields.len() == 5 && fields.get("gamma").and_then(serde_json::Value::as_f64) == Some(1.) => {
            let in_black = number("inBlack", 0., 253.)?;
            let in_white = number("inWhite", 2., 255.)?;
            let out_black = number("outBlack", 0., 255.)?;
            let out_white = number("outWhite", 0., 255.)?;
            if in_white < in_black + 2. || out_white <= out_black {
                return Err(Refusal::Unsupported("native ascending Levels span"));
            }
            Ok(Adjustment::Levels {
                master: LevelsChannel {
                    in_black: in_black / 255.,
                    in_white: in_white / 255.,
                    gamma: 1.,
                    out_black: out_black / 255.,
                    out_white: out_white / 255.,
                },
                per_channel: Default::default(),
                space: ToneSpace::Rgb,
                black: LevelsChannel::default(),
            })
        }
        _ => super::point_color::adjustment(filter).unwrap_or(Err(Refusal::Unsupported("unrecognized native tone data"))),
    }
}

#[cfg(test)]
mod tests;
