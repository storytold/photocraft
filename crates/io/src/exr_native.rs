//! Native OpenEXR blocks go straight into original-depth tiled storage.
use crate::{ImportResult, IoError};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_geom::{Rect, Size, TILE_SIZE};
use photocraft_raster::{Interrupt, Surface};
use std::io::{Read, Seek};

pub(crate) fn import<R: Read + Seek>(source: R, ctl: &Interrupt<'_>) -> Result<Option<ImportResult>, IoError> {
    let _metadata = photocraft_raster::memory::try_reserve(32 << 20).map_err(IoError::Unsupported)?;
    let mut decoder = match photocraft_codecs::exr_stream::Decoder::new(source, &|| ctl.cancelled()) {
        Ok(decoder) => decoder,
        Err(_) if ctl.cancelled() => return Err(IoError::Cancelled),
        Err(photocraft_codecs::CodecError::Unsupported { .. }) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let info = decoder.info();
    let mode = if info.gray { ColorMode::Grayscale } else { ColorMode::Rgb };
    let fmt = PixelFormat::new(mode, SampleType::F32, true);
    let scratch_expected = photocraft_raster::spill::enabled();
    if !scratch_expected {
        let budget = photocraft_raster::memory::refresh();
        let side = TILE_SIZE as u64;
        let tiles = u64::from(info.width).div_ceil(side).saturating_mul(u64::from(info.height).div_ceil(side));
        crate::psd_stream::ram_tile_capacity(
            false,
            photocraft_raster::spill::stats().resident_bytes as u64,
            budget.tile_bytes,
            tiles.saturating_mul(side * side).saturating_mul(fmt.bytes_per_pixel() as u64),
        )?;
    }
    let _working = photocraft_raster::memory::try_reserve(info.working_bytes).map_err(IoError::Unsupported)?;
    let mut surface = Surface::new(fmt);
    let mut done = 0;
    loop {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        crate::psd_stream::require_scratch(scratch_expected)?;
        let band = decoder.next_band(&|| ctl.cancelled()).map_err(|e| if ctl.cancelled() { IoError::Cancelled } else { e.into() })?;
        let Some(band) = band else {
            break;
        };
        let rect = Rect::from_xywh(band.x as i32, band.y as i32, band.width, band.height);
        if !photocraft_raster::spill::enabled() {
            let missing = rect.tiles().filter(|c| surface.tile(*c).is_none()).count() as u64;
            crate::psd_stream::ram_tile_capacity(
                false,
                photocraft_raster::spill::stats().resident_bytes as u64,
                photocraft_raster::memory::stats().1.tile_bytes,
                missing.saturating_mul((TILE_SIZE as u64).pow(2)).saturating_mul(fmt.bytes_per_pixel() as u64),
            )?;
        }
        surface.write_interleaved(rect, &band.data);
        crate::psd_stream::require_scratch(scratch_expected)?;
        done += 1;
        ctl.progress(0.05 + 0.9 * done as f32 / info.blocks.max(1) as f32);
    }
    // Missing alpha is filled with one in every allocated tile. Pruning these opaque
    // tiles cannot remove any and would unnecessarily reload the entire scratch store.
    if info.has_alpha {
        surface.prune();
    }
    crate::psd_stream::require_scratch(scratch_expected)?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let mut document = Document::new("", Size::new(info.width, info.height), mode, SampleType::F32);
    let mut layer = Layer::new("Background", LayerContent::Raster(surface));
    if !info.has_alpha {
        layer.locks.transparency = true;
        layer.locks.position = true;
    }
    document.layers.push(layer);
    if mode == ColorMode::Rgb {
        document.icc_profile = Some(photocraft_cms::Builtin::LinearSrgb.profile().to_bytes());
    }
    let warnings = if info.all_half { vec!["16-bit float samples are stored as 32-bit float".into()] } else { Vec::new() };
    ctl.progress(1.0);
    Ok(Some(ImportResult { document, warnings }))
}
