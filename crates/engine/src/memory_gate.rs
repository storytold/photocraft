//! Refuses a big operation that cannot fit in memory, with an error, before it starts (#10024).
//!
//! On the web build a failed allocation aborts the whole module and the page freezes without a
//! message. [`photocraft_raster::headroom`] tells whether a given number of bytes can still be
//! allocated; an operation that is about to allocate a lot asks here first, with a rough upper
//! bound of what it will need. A refusal changes nothing (the document, its history and the
//! selection stay as they were) and says what the user can do. Off the web every check passes.
//!
//! The estimates are deliberately rough and err high: they count the big buffers an operation
//! holds at once, not every temporary. Each function says what it counts.

use photocraft_doc::{Document, LayerContent, SmartSource};
use photocraft_geom::TILE_SIZE;

use crate::{EngineError, Result};

/// Peak bytes per source pixel while a smart object is rendered from its source: the decoded
/// source, its composite, the converted surface and the transformed result, measured on a large
/// embedded file. Lower it when the decoded originals stop being kept alongside (#10030).
pub const SMART_RENDER_BYTES_PER_PX: u64 = 41;

/// Bytes per pixel of the compositor's working buffer (`RGBA`, `f32`) that flattening a placed
/// file fills.
const COMPOSITE_BYTES_PER_PX: u64 = 16;

/// Pixels assumed per byte of a file whose size can't be read from its header (a rough bound: a
/// compressed image holds a few pixels per byte).
const FALLBACK_PX_PER_FILE_BYTE: u64 = 4;

/// `Ok` when `need` bytes can be allocated; otherwise an error naming `what` (for example
/// `place “x.png”`), the need, and what the user can do. The document is not touched.
pub fn ensure(what: &str, need: u64) -> Result<()> {
    check(what, need, "Save your work, close other documents or use Edit › Purge › All, then try again.")
}

/// [`ensure`] for saving and exporting: the user must be able to keep their work, so the advice
/// leads with freeing memory. The document stays open and unsaved.
pub fn ensure_for_save(what: &str, need: u64) -> Result<()> {
    check(what, need, "Use Edit › Purge › All or close other documents to free memory, then save again. The document is still open and unchanged.")
}

fn check(what: &str, need: u64, advice: &str) -> Result<()> {
    photocraft_raster::headroom::ensure(need).map_err(|_| EngineError::Other(format!("Not enough memory to {what} (it needs about {} MB). {advice}", mb(need))))
}

/// `bytes` in whole megabytes, rounded up, with thousands separators.
fn mb(bytes: u64) -> String {
    let n = bytes.div_ceil(1 << 20).to_string();
    let mut out = String::with_capacity(n.len() + n.len() / 3);
    for (i, c) in n.chars().enumerate() {
        if i > 0 && (n.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Pixel count of an image file read from its header, without decoding it. Formats without a
/// header reader here fall back to a size-based guess, so the answer is always an upper-bound
/// style estimate rather than an error.
pub(crate) fn source_pixels(bytes: &[u8]) -> u64 {
    header_pixels(bytes).unwrap_or_else(|| (bytes.len() as u64).saturating_mul(FALLBACK_PX_PER_FILE_BYTE))
}

fn header_pixels(bytes: &[u8]) -> Option<u64> {
    if let Ok((w, h)) = photocraft_codecs::probe_size(bytes) {
        return Some(u64::from(w) * u64::from(h));
    }
    let be32 = |at: usize| -> Option<u64> { Some(u64::from(u32::from_be_bytes(bytes.get(at..at.checked_add(4)?)?.try_into().ok()?))) };
    let be16 = |at: usize| -> Option<u64> { Some(u64::from(u16::from_be_bytes(bytes.get(at..at.checked_add(2)?)?.try_into().ok()?))) };
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        // IHDR: width, height.
        return Some(be32(16)?.saturating_mul(be32(20)?));
    }
    if bytes.starts_with(b"8BPS") {
        // PSD / PSB header: height then width.
        return Some(be32(14)?.saturating_mul(be32(18)?));
    }
    if bytes.starts_with(&[0xff, 0xd8]) {
        // JPEG: the first start-of-frame marker holds height then width.
        let mut at = 2usize;
        while at.checked_add(4)? <= bytes.len() {
            if *bytes.get(at)? != 0xff {
                at += 1;
                continue;
            }
            let marker = *bytes.get(at + 1)?;
            match marker {
                0xff => at += 1,
                0xc0..=0xcf if !matches!(marker, 0xc4 | 0xc8 | 0xcc) => return Some(be16(at + 5)?.saturating_mul(be16(at + 7)?)),
                0xd0..=0xd9 | 0x01 | 0x00 => at += 2,
                _ => at = at.checked_add(2)?.checked_add(usize::try_from(be16(at + 2)?).ok()?)?,
            }
        }
    }
    None
}

/// Place: the file's bytes, plus the decoded file, its composite and the placed pixels, which
/// are alive together (`bytes_per_px` is the document's pixel size, at least 4).
pub(crate) fn place_need(bytes: &[u8], bytes_per_px: usize) -> u64 {
    let px = source_pixels(bytes);
    let per_px = (bytes_per_px.max(4) as u64).saturating_mul(3).saturating_add(COMPOSITE_BYTES_PER_PX);
    (bytes.len() as u64).saturating_add(px.saturating_mul(per_px))
}

/// A smart object's re-render from `bytes`: its source pixels at the measured peak.
pub(crate) fn smart_render_need(bytes: &[u8]) -> u64 {
    source_pixels(bytes).saturating_mul(SMART_RENDER_BYTES_PER_PX)
}

/// Image Size to `nw` x `nh`: the resampled pixel surfaces and masks (each layer's tiles scaled
/// by the change of area, capped at a full canvas) in the document's pixel size, plus every
/// distinct embedded smart object re-rendered from its source at once.
pub(crate) fn image_size_need(doc: &Document, nw: u32, nh: u32) -> u64 {
    let bpp = doc.pixel_format().bytes_per_pixel().max(1) as u64;
    let old_px = (u64::from(doc.size.width) * u64::from(doc.size.height)).max(1);
    let new_px = u64::from(nw) * u64::from(nh);
    let canvas_bytes = new_px.saturating_mul(bpp);
    let tile_bytes = (TILE_SIZE as u64) * (TILE_SIZE as u64) * bpp;
    let resized = |tiles: usize| -> u64 {
        let old = (tiles as u64).saturating_mul(tile_bytes);
        let scaled = (old as u128 * u128::from(new_px) / u128::from(old_px)).min(u128::from(u64::MAX)) as u64;
        scaled.max(tile_bytes.min(canvas_bytes)).min(canvas_bytes)
    };
    let mut need = 0u64;
    let mut seen = std::collections::HashSet::new();
    for (_, _, layer) in doc.walk() {
        if let Some(s) = layer.surface() {
            need = need.saturating_add(resized(s.tile_count()));
        }
        if let Some(m) = &layer.mask {
            need = need.saturating_add(resized(m.surface.tile_count()));
        }
        if let LayerContent::Smart(so) = &layer.content {
            let embedded = match &so.source {
                SmartSource::Embedded { bytes, .. } => Some(bytes.as_slice()),
                SmartSource::Linked { .. } => None,
            };
            if let Some(bytes) = embedded
                && seen.insert(so.contents_id)
            {
                need = need.saturating_add(smart_render_need(bytes));
            }
        }
    }
    need
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn megabytes_are_rounded_up_and_grouped() {
        assert_eq!(mb(0), "0");
        assert_eq!(mb(1), "1");
        assert_eq!(mb(1240 << 20), "1,240");
        assert_eq!(mb(1_000 << 20), "1,000");
        assert_eq!(mb(12_345_678 << 20), "12,345,678");
    }

    #[test]
    fn headers_give_pixel_counts() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&300u32.to_be_bytes());
        png.extend_from_slice(&200u32.to_be_bytes());
        assert_eq!(source_pixels(&png), 60_000);
        let mut psd = b"8BPS\0\x01\0\0\0\0\0\0\0\x03".to_vec();
        psd.extend_from_slice(&50u32.to_be_bytes());
        psd.extend_from_slice(&40u32.to_be_bytes());
        assert_eq!(source_pixels(&psd), 2_000);
        // SOI, an APP0 segment of length 4, then SOF0: precision, height 20, width 30.
        let jpeg = [0xff, 0xd8, 0xff, 0xe0, 0, 4, 0, 0, 0xff, 0xc0, 0, 11, 8, 0, 20, 0, 30, 1];
        assert_eq!(source_pixels(&jpeg), 600);
    }

    #[test]
    fn unreadable_headers_fall_back_to_a_size_guess_without_panicking() {
        assert_eq!(source_pixels(b""), 0);
        assert_eq!(source_pixels(&[0u8; 100]), 400);
        for cut in 0..40 {
            let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x10\0\0\0\x10".to_vec();
            png.truncate(cut);
            let _ = source_pixels(&png);
            let jpeg = [0xff, 0xd8, 0xff, 0xe0, 0, 4, 0, 0, 0xff, 0xc0, 0, 11, 8, 0, 20, 0, 30, 1];
            let _ = source_pixels(&jpeg[..cut.min(jpeg.len())]);
        }
        // A segment length that runs off the end of the file ends the scan.
        let _ = source_pixels(&[0xff, 0xd8, 0xff, 0xe0, 0xff, 0xff]);
    }
}
