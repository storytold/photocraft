//! Conservative preflight for the main PSD pixel paths. This bounds decoded layer/mask/
//! merged and pattern pixels, not parser metadata or smart-filter caches (separate limits).
//!
//! Reserve the sum of all possible retained tiles and the largest workspace, without credit
//! for pruning or tile sharing. Reserving before any worker starts makes scheduling irrelevant.

use craft_psd::{
    ChannelData, Compression, PsdFile,
    layer::{CHANNEL_REAL_USER_MASK, CHANNEL_TRANSPARENCY, CHANNEL_USER_MASK, Rect},
};
use photocraft_geom::TILE_SIZE;

use crate::IoError;

/// Default allowance for PSD layer, mask and merged-image pixels, including conversion
/// workspace and tile padding. Other format/parser limits still apply. Callers may lower it.
pub const DEFAULT_PSD_IMPORT_PIXEL_BYTES: u64 = if cfg!(target_arch = "wasm32") { 1 << 30 } else { 2 << 30 };

/// Structured memory estimate for a PSD/PSB import.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PsdMemoryEstimate {
    /// Bytes required for all possible retained layer, mask, and merged surfaces.
    pub retained_bytes: u64,
    /// Largest scratch/conversion workspace required during decoding.
    pub workspace_bytes: u64,
    /// Total peak requirement (retained + workspace).
    pub total_bytes: u64,
    /// The part of `retained_bytes` that is tile pixels (layer, mask, channel, fallback and
    /// pattern surfaces). Tiles can be spilled to the scratch disk while it is on, so admission
    /// does not bound them by physical RAM; the merged planes, pattern decodes and the workspace
    /// are ordinary memory.
    pub tile_bytes: u64,
}

#[derive(Default)]
struct ImportBudget {
    retained: u64,
    workspace: u64,
    tiles: u64,
}

#[cfg(any(test, feature = "testgen"))]
thread_local! {
    /// Extra bytes added to every estimate on this thread by [`with_test_estimate_charge`].
    static TEST_ESTIMATE_CHARGE: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// Extra tile-pixel bytes added by [`with_test_tile_charge`].
    static TEST_TILE_CHARGE: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Adds `bytes` to every PSD memory estimate made on this thread while `f` runs.
///
/// A test seam: an admission-threshold test can use a tiny fixture, so nothing near a gigabyte
/// is ever allocated, while still exercising the real import path with its approval flags and
/// gate ordering. Scoped to the thread and restored on return or panic.
///
/// The charge lives on the calling thread. A real import that runs on another thread (or a
/// worker the engine starts for it) does not see it unless the caller propagates an explicitly
/// captured scope: [`crate::memory::TestMemoryScope`] captures this charge together with the
/// fixed mock snapshot, so an engine worker that enters that scope makes the same estimates as
/// the test thread. This is the seam engine and UI dev-dependency tests use through the
/// `testgen` feature; it is never process-global.
#[cfg(any(test, feature = "testgen"))]
pub fn with_test_estimate_charge<R>(bytes: u64, f: impl FnOnce() -> R) -> R {
    struct Reset(u64);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_ESTIMATE_CHARGE.with(|c| c.set(self.0));
        }
    }
    let prev = TEST_ESTIMATE_CHARGE.with(|c| c.replace(bytes));
    let _reset = Reset(prev);
    f()
}

/// [`with_test_estimate_charge`] for tile pixels: `bytes` are added to every estimate made on
/// this thread while `f` runs, as spillable tile bytes ([`PsdMemoryEstimate::tile_bytes`]), so a
/// scratch-aware admission test needs no huge fixture. Captured by
/// [`crate::memory::TestMemoryScope`] like the plain charge.
#[cfg(any(test, feature = "testgen"))]
pub fn with_test_tile_charge<R>(bytes: u64, f: impl FnOnce() -> R) -> R {
    struct Reset(u64);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_TILE_CHARGE.with(|c| c.set(self.0));
        }
    }
    let prev = TEST_TILE_CHARGE.with(|c| c.replace(bytes));
    let _reset = Reset(prev);
    f()
}

/// The calling thread's current (plain, tile) estimate charges, for an explicit scope to capture.
#[cfg(any(test, feature = "testgen"))]
pub(crate) fn test_estimate_charge() -> (u64, u64) {
    (TEST_ESTIMATE_CHARGE.with(|c| c.get()), TEST_TILE_CHARGE.with(|c| c.get()))
}

/// Enters captured estimate charges (an explicit [`crate::memory::TestMemoryScope`] worker) and
/// returns the charges they replaced, to restore after the scope ends.
#[cfg(any(test, feature = "testgen"))]
pub(crate) fn enter_test_estimate_charge(bytes: u64, tiles: u64) -> (u64, u64) {
    (TEST_ESTIMATE_CHARGE.with(|c| c.replace(bytes)), TEST_TILE_CHARGE.with(|c| c.replace(tiles)))
}

/// The test seam's extra (plain, tile) charges; zero in every other build.
fn test_estimate_charge_value() -> (u64, u64) {
    #[cfg(any(test, feature = "testgen"))]
    {
        test_estimate_charge()
    }
    #[cfg(not(any(test, feature = "testgen")))]
    {
        (0, 0)
    }
}

fn overflow() -> IoError {
    IoError::Unsupported("PSD import pixel memory estimate overflow".into())
}

// inflate_checked uses at least 8 KiB per stream, even for a one-pixel plane.
fn inflate_scratch(plane_bytes: u64, channels: u64) -> Result<u64, IoError> {
    mul(plane_bytes.saturating_add(1).clamp(8192, 1 << 20), channels)
}

/// How a plane is stored: its compression method, and the bytes it takes in the file.
type Encoded = (Option<Compression>, u64);

fn encoding(channel: &ChannelData) -> Encoded {
    (channel.compression, channel.encoded_len())
}

/// Peak bytes the row decoders of `channels` hold while their `w` × `h` planes are decoded
/// together, `band` rows at a time (`craft_psd::RowDecoder`).
///
/// Only a ZIP stream is inflated whole, because it cannot be entered part way. Raw and RLE
/// channels are read a band at a time, so they cost a band whatever the size of the layer:
/// reserving a whole plane for them refuses large documents that decode in a few megabytes.
fn decode_workspace(channels: impl Iterator<Item = Encoded>, w: u64, h: u64, band: u64, sample: u64) -> Result<u64, IoError> {
    let plane = mul(mul(w, h)?, sample)?;
    let band_plane = mul(mul(w, band)?, sample)?;
    let mut total = 0u64;
    for (compression, encoded) in channels {
        let held = match compression {
            // The encoded stream, the inflated plane and the inflater's own buffer.
            Some(Compression::Zip | Compression::ZipPrediction) => {
                let inflated = plane.checked_add(inflate_scratch(plane, 1)?).ok_or_else(overflow)?;
                inflated.checked_add(encoded).ok_or_else(overflow)?
            }
            // The row count table (up to 4 bytes a row) beside the row offsets made from it
            // (8 bytes a row), then the encoded rows of a band. PackBits writes at most two
            // bytes for a decoded one, and a band can never hold more than the channel does.
            Some(Compression::Rle) => mul(h.saturating_add(1), 12)?.checked_add(encoded.min(mul(band_plane, 2)?)).ok_or_else(overflow)?,
            // The band's bytes as read from the source.
            Some(Compression::Raw) => band_plane,
            // No decoder is made for these: the layer is left empty.
            Some(Compression::Unknown(_)) | None => 0,
        };
        // Every decoder hands out its band as a new buffer.
        total = total.checked_add(held).and_then(|v| v.checked_add(band_plane)).ok_or_else(overflow)?;
    }
    Ok(total)
}

fn mul(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_mul(b).ok_or_else(overflow)
}

impl ImportBudget {
    fn retain(&mut self, bytes: u64) -> Result<(), IoError> {
        self.retained = self.retained.checked_add(bytes).ok_or_else(overflow)?;
        Ok(())
    }

    fn scratch(&mut self, bytes: u64) {
        self.workspace = self.workspace.max(bytes);
    }

    fn required(&self) -> Result<u64, IoError> {
        self.retained.checked_add(self.workspace).ok_or_else(overflow)
    }

    fn surface(&mut self, rect: Rect, channels: u64, sample: u64) -> Result<(), IoError> {
        if rect.is_empty() {
            return Ok(());
        }
        let tile = i64::from(TILE_SIZE);
        // Division must round down for negative offsets: even a 2x2 rect straddling the
        // origin occupies four tiles. Widen first, including the exclusive right/bottom edge.
        let nx = (i64::from(rect.right) - 1).div_euclid(tile) - i64::from(rect.left).div_euclid(tile) + 1;
        let ny = (i64::from(rect.bottom) - 1).div_euclid(tile) - i64::from(rect.top).div_euclid(tile) + 1;
        let bytes = mul(mul(mul(nx as u64, ny as u64)?, (TILE_SIZE as u64).pow(2))?, mul(channels, sample)?)?;
        self.tile(bytes)
    }

    /// Retained tile pixels: spillable to the scratch disk.
    fn tile(&mut self, bytes: u64) -> Result<(), IoError> {
        self.retain(bytes)?;
        self.tiles = self.tiles.checked_add(bytes).ok_or_else(overflow)?;
        Ok(())
    }
}

/// Computes the conservative preflight pixel memory and workspace estimate for `file`.
///
/// No decoding here: even a compact ZIP stream must be refused before inflation. Layers and
/// masks count what their decoders hold (a whole plane for a ZIP channel, a band of rows for
/// the others) plus the interleaved band, and so does a merged image read in bands
/// (`psd_import::merged_in_bands`); any other merged image counts full decoded planes, with
/// its interleave output and float conversion workspace.
pub fn estimate(file: &PsdFile) -> Result<PsdMemoryEstimate, IoError> {
    let mut budget = ImportBudget::default();
    let h = &file.header;
    h.validate()?;
    let sample = u64::from(h.depth.div_ceil(8));
    let cc = match h.color_mode {
        // Unknown modes use the RGB fallback, including its alpha channel.
        craft_psd::ColorMode::Unknown(_) => 3,
        mode => u64::from(mode.color_channels().unwrap_or(3)),
    };
    if super::psd_import::can_recover_layers(file) {
        for rec in file.layers() {
            if !rec.rect.is_empty() && rec.channels.iter().any(|c| c.id >= 0 || c.id == -1) {
                let (w, hh) = rec.rect.size()?;
                budget.surface(rec.rect, cc + 1, sample)?;
                // The colour channels and transparency decode side by side, a band at a time.
                let row_bytes = mul(w as u64, mul(cc + 1, sample)?)?;
                let band = super::psd_import::max_band_rows(hh, usize::try_from(row_bytes).unwrap_or(usize::MAX)) as u64;
                let ids = (0..cc).filter_map(|id| i16::try_from(id).ok()).chain([CHANNEL_TRANSPARENCY]);
                let decoders = decode_workspace(ids.filter_map(|id| rec.channel(id)).map(encoding), w as u64, hh as u64, band, sample)?;
                // The band interleaved: it is written to the tiles as it is, in its sample type.
                budget.scratch(decoders.checked_add(mul(row_bytes, band)?).ok_or_else(overflow)?);
            }
            if let Some(mask) = rec.layer_mask() {
                for (id, rect) in [(CHANNEL_USER_MASK, mask.rect), (CHANNEL_REAL_USER_MASK, mask.real.map_or(mask.rect, |m| m.rect))] {
                    if rec.channel(id).is_none() || rect.is_empty() {
                        continue;
                    }
                    let (w, hh) = rect.size()?;
                    budget.surface(rect, 1, sample)?;
                    // The layer's ZIP decoders have been dropped before record_mask runs. A
                    // mask decodes one tile row at a time and converts its samples in place.
                    let band = (TILE_SIZE as u64).min(hh as u64);
                    budget.scratch(decode_workspace(rec.channel(id).map(encoding).into_iter(), w as u64, hh as u64, band, sample)?);
                }
            }
        }
    }
    if super::psd_import::merged_needed(file) {
        let pixels = mul(u64::from(h.width), u64::from(h.height))?;
        let channels = u64::from(h.channels);
        let canvas = Rect::from_xywh(0, 0, h.width, h.height);
        // Multichannel and extra channels become individual grayscale surfaces. Reserving
        // their tiles separately matters for images smaller than a tile.
        budget.surface(canvas, channels.max(cc + 1), sample.max(1))?;
        // A fallback surface can coexist with the layers when none produced usable pixels.
        budget.surface(canvas, cc + 1, sample.max(1))?;
        if super::psd_import::merged_in_bands(file) {
            // No plane is held whole: the background layer and the extra channels are written
            // a band at a time from the image's row decoder, which lives until the import ends.
            let (w, rows) = (u64::from(h.width), u64::from(h.height));
            let row_bytes = mul(w, mul(cc + 1, sample)?)?;
            let band = super::psd_import::max_band_rows(h.height as usize, usize::try_from(row_bytes).unwrap_or(usize::MAX)) as u64;
            let stored = (Some(file.image_data.compression), file.image_data.encoded_len());
            let decoders = decode_workspace(std::iter::repeat_n(stored, usize::from(h.channels)), w, rows, band, sample)?;
            if matches!(file.image_data.compression, Compression::Rle) {
                // The RLE decoder keeps the offsets of every plane's rows (8 bytes a row) while
                // the layers are built, so they add to the layers' workspace.
                budget.retain(mul(mul(rows.saturating_add(1), channels)?, 8)?)?;
            }
            // An RLE image has one row table for all its planes, read when the decoder is made.
            let table = mul(mul(rows, channels)?.saturating_add(1), 8)?;
            // The band interleaved, and a tile row of float pixels while transparency is unmatted.
            let converted = mul(row_bytes, band)?.checked_add(mul(mul(w, rows.min(TILE_SIZE as u64))?, (cc + 1) * 4)?).ok_or_else(overflow)?;
            budget.scratch(decoders.checked_add(table).and_then(|v| v.checked_add(converted)).ok_or_else(overflow)?);
        } else {
            // The merged planes stay alive while layers are built.
            let merged = mul(mul(h.row_bytes() as u64, u64::from(h.height))?, channels)?;
            budget.retain(merged)?;
            // Include float expansion/readback and RGBA fallback, even for packed bitmap input.
            let converted = mul(pixels, mul(channels.max(cc + 1), sample * 3 + 12)?)?;
            budget.scratch(converted.checked_add(inflate_scratch(merged, 1)?).ok_or_else(overflow)?);
        }
    }
    let mut patterns = craft_psd::patterns::PatternBudget::default();
    for block in file.global_blocks.iter().filter(|b| crate::pattern_map::PATTERN_KEYS.contains(&b.key)) {
        // Earlier records can decode before a malformed later record discards the block.
        let items = craft_psd::patterns::pattern_block_memory(&block.data, &mut patterns);
        for p in items {
            let pixels = mul(u64::from(p.width), u64::from(p.height))?;
            let sample = u64::from(p.depth.div_ceil(8));
            let color = if p.mode == 2 { 3 } else { craft_psd::patterns::mode_channels(p.mode) as u64 };
            let alpha = u64::from(p.planes > craft_psd::patterns::mode_channels(p.mode));
            budget.surface(Rect::from_xywh(0, 0, p.width, p.height), color + alpha, sample)?;
            // The block parser returns all planes together. Count them as retained so
            // previous converted patterns and later blocks cannot reuse this allowance.
            budget.retain(p.decoded_bytes)?;
            // Channel clones/bitmap expansion, indexed RGB expansion and interleave.
            // Include a source plane during padded channel placement as well.
            let converted = mul(pixels, mul((p.planes as u64).max(color + alpha) + color + alpha + 3, sample)?)?;
            budget.scratch(converted.checked_add(p.decoded_bytes).ok_or_else(overflow)?);
        }
    }
    let (charge, tile_charge) = test_estimate_charge_value();
    budget.retain(charge)?;
    budget.tile(tile_charge)?;
    let total_bytes = budget.required()?;
    Ok(PsdMemoryEstimate { retained_bytes: budget.retained, workspace_bytes: budget.workspace, total_bytes, tile_bytes: budget.tiles })
}

/// Checks that `file`'s estimate does not exceed `limit`.
pub fn check(file: &PsdFile, limit: u64) -> Result<PsdMemoryEstimate, IoError> {
    let est = estimate(file)?;
    if est.total_bytes > limit {
        return Err(IoError::PsdImportBudget { required: est.total_bytes, limit, free_memory: None });
    }
    Ok(est)
}

#[cfg(test)]
mod tests;
