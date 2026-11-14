//! Paint.NET (`.pdn`) import: the PDN3 reader is craft-libs' `craft-pdn`. This maps its layers
//! and BGRA surfaces onto documents and charges the tiles they are stored in to the budget.

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use craft_pdn::{BlendMode as PdnBlend, MAX_PIXEL_BYTES};
use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_geom::{Rect, Size};
use photocraft_raster::{Interrupt, Surface};

use crate::{ImportResult, IoError};

type Result<T> = std::result::Result<T, IoError>;

fn invalid(message: impl Into<String>) -> IoError {
    IoError::Pdn(message.into())
}

/// The format crate's error as this crate's.
fn pdn_err(e: craft_pdn::Error) -> IoError {
    match e {
        craft_pdn::Error::Invalid(m) => invalid(m),
        craft_pdn::Error::Cancelled => IoError::Cancelled,
    }
}

fn blend(mode: PdnBlend) -> BlendMode {
    match mode {
        PdnBlend::Normal => BlendMode::Normal,
        PdnBlend::Multiply => BlendMode::Multiply,
        PdnBlend::Additive => BlendMode::LinearDodge,
        PdnBlend::ColorBurn => BlendMode::PaintNetColorBurn,
        PdnBlend::ColorDodge => BlendMode::PaintNetColorDodge,
        PdnBlend::Reflect => BlendMode::Reflect,
        PdnBlend::Glow => BlendMode::Glow,
        PdnBlend::Overlay => BlendMode::Overlay,
        PdnBlend::Difference => BlendMode::Difference,
        PdnBlend::Negation => BlendMode::Negation,
        PdnBlend::Lighten => BlendMode::Lighten,
        PdnBlend::Darken => BlendMode::Darken,
        PdnBlend::Screen => BlendMode::Screen,
        PdnBlend::Xor => BlendMode::Xor,
    }
}

pub(super) fn import(name: &str, bytes: &[u8], ctl: &Interrupt<'_>) -> Result<ImportResult> {
    let keep_going = || ctl.check().is_ok();
    let doc = craft_pdn::Document::parse(bytes, &keep_going).map_err(pdn_err)?;
    let (width, height) = (doc.width, doc.height);
    let (w, h) = (u32::try_from(width).map_err(|_| invalid("canvas size overflow"))?, u32::try_from(height).map_err(|_| invalid("canvas size overflow"))?);
    let bounds = Rect::from_xywh(0, 0, w, h);
    // Every allocation is checked before any block is decoded: the packed rows plus the whole
    // tiles each surface keeps them in (a narrow canvas stores up to 256 times its packed bytes).
    let tiles = usize::try_from(photocraft_raster::tile_storage_bytes(bounds, &PixelFormat::RGBA8)).unwrap_or(usize::MAX);
    let total = tiles.checked_mul(doc.surface_count()).and_then(|t| t.checked_add(doc.decoded_bytes())).ok_or_else(|| invalid("pixel length overflow"))?;
    if total > MAX_PIXEL_BYTES {
        return Err(invalid("decoded pixels exceed the 1 GiB limit"));
    }
    let count = doc.surface_count();
    let mut surfaces = HashMap::new();
    let mut failed = None;
    doc.read_surfaces(&keep_going, &mut |i, s| {
        if failed.is_some() {
            return;
        }
        let mut surface = Surface::new(PixelFormat::RGBA8);
        let mut row = Vec::with_capacity(width * 4);
        for (y, src) in s.rows().enumerate() {
            if ctl.check().is_err() {
                failed = Some(IoError::Cancelled);
                return;
            }
            row.clear();
            row.extend_from_slice(src);
            for pixel in row.as_chunks_mut::<4>().0 {
                pixel.swap(0, 2);
            }
            surface.write_interleaved(Rect::from_xywh(0, y as i32, w, 1), &row);
        }
        surfaces.insert(i, surface);
        ctl.progress(0.05 + 0.95 * (i + 1) as f32 / count as f32);
    })
    .map_err(pdn_err)?;
    if let Some(e) = failed {
        return Err(e);
    }
    let mut document = Document::new(name, Size::new(w, h), ColorMode::Rgb, SampleType::U8);
    for l in &doc.layers {
        let mut layer = Layer::raster(l.name.clone(), PixelFormat::RGBA8);
        layer.visible = l.visible;
        layer.opacity = f32::from(l.opacity) / 255.0;
        layer.blend = blend(l.blend);
        layer.content = LayerContent::Raster(surfaces.get(&l.surface).cloned().ok_or_else(|| invalid("missing pixel memory block"))?);
        document.layers.push(layer);
    }
    let mut warnings = Vec::new();
    if doc.has_user_metadata {
        warnings.push("Paint.NET document metadata was not imported".into());
    }
    Ok(ImportResult { document, warnings, source_read_only: true, preview_only: false })
}
