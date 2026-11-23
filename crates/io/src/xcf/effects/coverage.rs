//! Private, source-bound coverage for the proven XCF22 U8/U16 Exposure subset.
use super::installer::ResolvedMask;
use crate::xcf::raster::TileWriter;
use craft_xcf::{
    BaseType, Error, Xcf,
    effects::StoredEffect,
    records,
    samples::{EffectMaskError, EffectMaskHeader, SampleOptions},
};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::LayerMask;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, tile_storage_bytes};

pub(in crate::xcf) struct Admission<'a> {
    pub retained: &'a mut usize,
    pub source: &'a mut usize,
    pub metadata: &'a mut records::Budget,
    pub options: SampleOptions,
}
pub(in crate::xcf) struct Decoded<'a> {
    pub header: EffectMaskHeader<'a>,
    pub native: ResolvedMask,
}
#[derive(Debug)]
pub(in crate::xcf) enum Failure {
    Cancelled,
    Invalid,
    Unsupported,
    Limit,
    Resource,
}

/// The header's private source seal is checked by craft-xcf. Neither raw pointer
/// arithmetic nor source policy normalization lives in this consumer.
pub(in crate::xcf) fn decode<'a>(
    xcf: &Xcf<'a>,
    effect: &StoredEffect<'_>,
    admission: &mut Admission<'_>,
    retained: &mut usize,
    work: &mut usize,
    ctl: &Interrupt<'_>,
) -> Result<Decoded<'a>, Failure> {
    ctl.check().map_err(|_| Failure::Cancelled)?;
    let image = xcf.image();
    if image.version != 22 || image.base_type != BaseType::Rgb || image.precision.float || !matches!(image.precision.bytes, 1 | 2) {
        return Err(Failure::Unsupported);
    }
    let header = xcf
        .effect_mask_header(effect, admission.metadata, &|| !ctl.cancelled())
        .map_err(|error| match error {
            EffectMaskError::Source(Error::Cancelled) => Failure::Cancelled,
            EffectMaskError::Source(Error::Invalid(_)) => Failure::Invalid,
            EffectMaskError::Metadata(records::StopReason::Malformed) => Failure::Invalid,
            EffectMaskError::Metadata(records::StopReason::Unsupported) => Failure::Unsupported,
            EffectMaskError::Metadata(_) => Failure::Limit,
        })?
        .ok_or(Failure::Invalid)?;
    // This slice proves image-global selection snapshots only. Channel framing
    // ends at its hierarchy pointer; trailing bytes are not child effect lists.
    // Do not guess a different frame or custom placement from absent metadata.
    if (header.width(), header.height()) != (image.width, image.height) {
        return Err(Failure::Unsupported);
    }
    for property in header.properties() {
        ctl.check().map_err(|_| Failure::Cancelled)?;
        if !matches!(property.kind, 3 | 4 | 6 | 8 | 14 | 16 | 20 | 28 | 32 | 33 | 34 | 38 | 42) {
            return Err(Failure::Unsupported);
        }
    }
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false);
    let frame = Rect::from_xywh(0, 0, header.width(), header.height());
    let storage = usize::try_from(tile_storage_bytes(frame, &format)).map_err(|_| Failure::Limit)?;
    let logical = (header.width() as usize).checked_mul(header.height() as usize).and_then(|pixels| pixels.checked_mul(4)).ok_or(Failure::Limit)?;
    let cost = storage.checked_mul(2).and_then(|bytes| bytes.checked_add(logical)).ok_or(Failure::Limit)?;
    if storage > *retained || storage > crate::xcf::MAX_PIXEL_BYTES {
        return Err(Failure::Limit);
    }
    match work.checked_sub(cost) {
        Some(left) => *work = left,
        None => {
            *work = 0;
            return Err(Failure::Limit);
        }
    }
    let mut writer = TileWriter::new(format, (0, 0), &[], false, true, *retained);
    let mut consumer_failed = false;
    xcf.visit_effect_mask_tiles(&header, admission.options, admission.source, &|| !ctl.cancelled(), |tile| {
        writer.write(tile).inspect_err(|_| consumer_failed = true)
    })
    .map_err(|error| match error {
        Error::Cancelled => Failure::Cancelled,
        Error::Invalid(_) if consumer_failed => Failure::Resource,
        Error::Invalid(_) => Failure::Invalid,
    })?;
    let (surface, clamped) = writer.finish(retained);
    if clamped {
        return Err(Failure::Invalid);
    }
    ctl.check().map_err(|_| Failure::Cancelled)?;
    // Channel overlay opacity/color/visibility are retained as source evidence;
    // they are display controls, not extra factors in the coverage samples.
    let native = ResolvedMask { source_offset: header.source_offset(), mask: LayerMask { surface, enabled: true, linked: true, density: 1., feather: 0. } };
    Ok(Decoded { header, native })
}
