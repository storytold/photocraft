//! Clean-room Aseprite document I/O, based only on the public binary specification:
//! <https://github.com/aseprite/aseprite/blob/main/docs/ase-file-specs.md>.

mod decode;
mod encode;
mod reader;
#[cfg(test)]
mod tests;

use crate::{ExportResult, ImportResult, IoError};
use photocraft_color::BlendMode;
use photocraft_doc::Document;
use photocraft_raster::Interrupt;

type Result<T> = std::result::Result<T, IoError>;
const MAX_FILE: usize = 512 << 20;
const MAX_LAYERS: usize = 4096;
const MAX_CHUNKS: usize = 65_536;
const MAX_PIXELS: usize = photocraft_doc::sprite::MAX_BYTES;
fn invalid(message: impl Into<String>) -> IoError {
    IoError::Aseprite(message.into())
}

pub(crate) fn import(name: &str, bytes: &[u8], ctl: &Interrupt<'_>) -> Result<ImportResult> {
    decode::import(name, bytes, ctl)
}

pub(crate) fn export(doc: &Document, ctl: &Interrupt<'_>) -> Result<ExportResult> {
    encode::export(doc, ctl)
}

/// Features Aseprite cannot represent. Such an export is refused rather than silently
/// discarding appearance or frames; save a native document or rasterize a copy first.
pub fn losses(doc: &Document) -> Vec<String> {
    let mut out = Vec::new();
    if doc.depth != photocraft_color::SampleType::U8 {
        out.push("Aseprite requires 8-bit samples".into());
    }
    if !matches!(doc.mode, photocraft_color::ColorMode::Rgb | photocraft_color::ColorMode::Grayscale | photocraft_color::ColorMode::Indexed) {
        out.push("Aseprite requires RGB, grayscale or indexed color".into());
    }
    for (_, _, layer) in doc.walk() {
        if !matches!(layer.content, photocraft_doc::LayerContent::Raster(_) | photocraft_doc::LayerContent::Group(_))
            || layer.mask.is_some()
            || layer.vector_mask.is_some()
            || layer.clipped
            || layer.fill_opacity != 1.0
            || layer.effects.enabled && !layer.effects.items.is_empty()
            || layer.excluded_channels != 0
            || layer.blend_if != Default::default()
            || layer.advanced != Default::default()
            || layer.video.is_some()
            || layer.xmp.is_some()
        {
            out.push("Aseprite cannot retain this document's non-raster content, masks or Photoshop blending/effects; rasterize a copy first".into());
            break;
        }
    }
    if doc.sprite.is_none() && doc.timeline.is_some() {
        out.push("a video timeline must be converted to cel animation before Aseprite export".into());
    }
    if !doc.channels.is_empty() {
        out.push("Aseprite cannot retain extra alpha or spot channels; save a .pcraft copy instead".into());
    }
    if !doc.paths.is_empty() || !doc.notes.is_empty() {
        out.push("Aseprite cannot retain named paths or notes; save a .pcraft copy instead".into());
    }
    if !doc.metadata.text.is_empty() || doc.metadata.xmp.is_some() || doc.metadata.exif.is_some() {
        out.push("Aseprite cannot retain document text, XMP or EXIF metadata; save a .pcraft copy instead".into());
    }
    out
}

/// Recognize the document signature, independently of its extension. Adobe Swatch Exchange
/// starts with `ASEF`, not Aseprite's little-endian magic at offset four. Full validation follows.
pub fn is_aseprite(bytes: &[u8]) -> bool {
    let word = |at| bytes.get(at..at + 2).and_then(|b| <[u8; 2]>::try_from(b).ok()).map(u16::from_le_bytes);
    let size = bytes.get(..4).and_then(|b| <[u8; 4]>::try_from(b).ok()).map(u32::from_le_bytes);
    bytes.len() >= 144
        && size.is_some_and(|n| n as u64 == bytes.len() as u64)
        && word(4) == Some(0xa5e0)
        && word(6).is_some_and(|n| n > 0)
        && word(8).is_some_and(|n| n > 0)
        && word(10).is_some_and(|n| n > 0)
        && matches!(word(12), Some(8 | 16 | 32))
        && word(132) == Some(0xf1fa)
}

fn blend_mode(mode: u16) -> Result<BlendMode> {
    use BlendMode as B;
    // IDs are specified by the format, not PhotoCraft's enum discriminants.
    const MODES: [B; 19] = [
        B::Normal,
        B::Multiply,
        B::Screen,
        B::Overlay,
        B::Darken,
        B::Lighten,
        B::ColorDodge,
        B::ColorBurn,
        B::HardLight,
        B::SoftLight,
        B::Difference,
        B::Exclusion,
        B::Hue,
        B::Saturation,
        B::Color,
        B::Luminosity,
        B::LinearDodge,
        B::Subtract,
        B::Divide,
    ];
    MODES.get(usize::from(mode)).copied().ok_or_else(|| invalid(format!("unsupported blend mode {mode}")))
}
