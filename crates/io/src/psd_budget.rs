//! Conservative preflight for the main PSD pixel paths. This bounds decoded layer/mask/
//! merged and pattern pixels, not parser metadata or smart-filter caches (separate limits).
//!
//! Reserve the sum of all possible retained tiles and the largest workspace, without credit
//! for pruning or tile sharing. Reserving before any worker starts makes scheduling irrelevant.

use photocraft_geom::TILE_SIZE;
use photocraft_psd::{
    Compression, PsdFile,
    layer::{CHANNEL_REAL_USER_MASK, CHANNEL_TRANSPARENCY, CHANNEL_USER_MASK, ChannelData, LayerRecord, Rect},
};
use std::borrow::Cow;

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

fn mul(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_mul(b).ok_or_else(overflow)
}

/// Reads the bounded RLE row-count table without inflating channel pixels.
/// If the table cannot be trusted, the caller retains the conservative full-plane estimate.
fn rle_row_table<'a>(channel: &'a ChannelData, height: usize, count_bytes: usize) -> Option<Cow<'a, [u8]>> {
    let len = height.checked_mul(count_bytes)?;
    if u64::try_from(len).ok()? > channel.encoded_len() {
        return None;
    }
    if let Some(stored) = &channel.stored { Some(Cow::Owned(stored.read_part(0, len as u64).ok()?)) } else { Some(Cow::Borrowed(channel.data.get(..len)?)) }
}

/// Peak workspace for the exact row bands used by the importer. It includes per-channel row
/// offsets, concurrently live encoded/decoded bands, and the interleaved output when present.
/// Invalid or unreadable RLE tables return `None`, leaving the conservative estimate in place.
struct RleWorkspace {
    bands: Vec<(usize, usize)>,
    width: usize,
    height: usize,
    sample: u64,
    version: photocraft_psd::Version,
    output_channels: u64,
    interleave_output: bool,
}

fn rle_bands_workspace(channels: &[&ChannelData], layout: RleWorkspace) -> Option<u64> {
    let RleWorkspace { bands, width, height, sample, version, output_channels, interleave_output } = layout;
    if channels.is_empty() || channels.iter().any(|channel| channel.compression != Some(Compression::Rle)) || bands.is_empty() {
        return None;
    }

    let count_bytes = if version.is_psb() { 4usize } else { 2usize };
    let table_len = height.checked_mul(count_bytes)?;
    let mut encoded_by_band = Vec::new();
    if encoded_by_band.try_reserve_exact(bands.len()).is_err() {
        return None;
    }
    encoded_by_band.resize(bands.len(), 0u64);

    let mut table_bytes = 0u64;
    let mut row_index_bytes = 0u64;
    for channel in channels {
        let table = rle_row_table(channel, height, count_bytes)?;
        let payload_len = channel.encoded_len().checked_sub(u64::try_from(table_len).ok()?)?;
        let mut total_encoded = 0u64;
        for (band_index, &(b0, b1)) in bands.iter().enumerate() {
            let mut band_encoded = 0u64;
            for row in b0..b1 {
                let start = row.checked_mul(count_bytes)?;
                let end = start.checked_add(count_bytes)?;
                let bytes = table.get(start..end)?;
                let len = match bytes {
                    [a, b] => u64::from(u16::from_be_bytes([*a, *b])),
                    [a, b, c, d] => u64::from(u32::from_be_bytes([*a, *b, *c, *d])),
                    _ => return None,
                };
                band_encoded = band_encoded.checked_add(len)?;
            }
            total_encoded = total_encoded.checked_add(band_encoded)?;
            let slot = encoded_by_band.get_mut(band_index)?;
            *slot = slot.checked_add(band_encoded)?;
        }
        // A short/inconsistent row table must never lower the estimate.
        if total_encoded > payload_len {
            return None;
        }
        table_bytes = table_bytes.checked_add(u64::try_from(table_len).ok()?)?;
        row_index_bytes = row_index_bytes.checked_add(mul((height as u64).checked_add(1)?, 8).ok()?)?;
    }

    let pixel_channels = u64::try_from(channels.len()).ok()?;
    let range_storage = u64::try_from(bands.capacity()).ok()?.checked_mul(std::mem::size_of::<(usize, usize)>() as u64)?;
    let mut peak = table_bytes.checked_add(row_index_bytes)?.checked_add(range_storage)?;
    for (band_index, &(b0, b1)) in bands.iter().enumerate() {
        let band_rows = u64::try_from(b1.checked_sub(b0)?).ok()?;
        let band_pixels = mul(width as u64, band_rows).ok()?;
        let decoded = mul(mul(band_pixels, pixel_channels).ok()?, sample).ok()?;
        let interleaved = if interleave_output { mul(mul(band_pixels, output_channels).ok()?, sample).ok()? } else { 0 };
        let encoded = *encoded_by_band.get(band_index)?;
        // Row decoding holds encoded input beside decoded output. On return, encoded input is
        // dropped while planar outputs overlap the interleaved output.
        let decoding = row_index_bytes.checked_add(range_storage)?.checked_add(encoded)?.checked_add(decoded)?;
        let interleave = row_index_bytes.checked_add(range_storage)?.checked_add(decoded)?.checked_add(interleaved)?;
        peak = peak.max(decoding).max(interleave);
    }
    Some(peak)
}

fn rle_layer_workspace(rec: &LayerRecord, w: usize, h: usize, cc: u64, sample: u64, file: &PsdFile) -> Option<u64> {
    let mut channels = Vec::new();
    for id in 0..cc {
        channels.extend(i16::try_from(id).ok().and_then(|id| rec.channel(id)));
    }
    channels.extend(rec.channel(CHANNEL_TRANSPARENCY));
    let row_bytes = w.checked_mul(usize::try_from(cc.checked_add(1)?).ok()?)?.checked_mul(usize::try_from(sample).ok()?)?;
    let bands = super::psd_import::band_rows(rec.rect.top, h, row_bytes);
    rle_bands_workspace(
        &channels,
        RleWorkspace { bands, width: w, height: h, sample, version: file.header.version, output_channels: cc.checked_add(1)?, interleave_output: true },
    )
}

fn rle_mask_workspace(channel: &ChannelData, rect: Rect, sample: u64, version: photocraft_psd::Version) -> Option<u64> {
    let (width, height) = rect.size().ok()?;
    let bands = super::psd_import::row_bands(rect.top, height, TILE_SIZE as usize);
    rle_bands_workspace(&[channel], RleWorkspace { bands, width, height, sample, version, output_channels: 1, interleave_output: false })
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
/// No decoding here: even a compact ZIP stream must be refused before inflation. RLE workspace
/// follows the importer's bounded bands; ZIP and other paths retain their conservative estimates.
pub fn estimate(file: &PsdFile) -> Result<PsdMemoryEstimate, IoError> {
    let mut budget = ImportBudget::default();
    let h = &file.header;
    h.validate()?;
    let sample = u64::from(h.depth.div_ceil(8));
    let cc = match h.color_mode {
        // Unknown modes use the RGB fallback, including its alpha channel.
        photocraft_psd::ColorMode::Unknown(_) => 3,
        mode => u64::from(mode.color_channels().unwrap_or(3)),
    };
    if super::psd_import::can_recover_layers(file) {
        for rec in file.layers() {
            if !rec.rect.is_empty() && rec.channels.iter().any(|c| c.id >= 0 || c.id == -1) {
                let (w, hh) = rec.rect.size()?;
                let pixels = mul(w as u64, hh as u64)?;
                budget.surface(rec.rect, cc + 1, sample)?;
                let workspace = match rle_layer_workspace(rec, w, hh, cc, sample, file) {
                    Some(workspace) => workspace,
                    None => {
                        // ZIP, mixed compression, and malformed RLE retain the original
                        // conservative full-plane estimate.
                        mul(pixels, mul(cc + 1, sample * 3 + 4)?)?
                            .checked_add(inflate_scratch(mul(pixels, sample)?, cc + 1)?)
                            .ok_or_else(overflow)?
                            .checked_add(mul(hh as u64 + 1, (cc + 1) * 8)?)
                            .ok_or_else(overflow)?
                    }
                };
                budget.scratch(workspace);
            }
            if let Some(mask) = rec.layer_mask() {
                for (id, rect) in [(CHANNEL_USER_MASK, mask.rect), (CHANNEL_REAL_USER_MASK, mask.real.map_or(mask.rect, |m| m.rect))] {
                    let Some(channel) = rec.channel(id) else { continue };
                    if rect.is_empty() {
                        continue;
                    }
                    let (w, hh) = rect.size()?;
                    budget.surface(rect, 1, sample)?;
                    let pixels = mul(w as u64, hh as u64)?;
                    let workspace = match rle_mask_workspace(channel, rect, sample, h.version) {
                        Some(workspace) => workspace,
                        None => mul(pixels, sample * 3 + 4)?.checked_add(inflate_scratch(mul(pixels, sample)?, 1)?).ok_or_else(overflow)?,
                    };
                    budget.scratch(workspace);
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
        // The merged planes stay alive while layers are built.
        let merged = mul(mul(h.row_bytes() as u64, u64::from(h.height))?, channels)?;
        budget.retain(merged)?;
        // Include float expansion/readback and RGBA fallback, even for packed bitmap input.
        let converted = mul(pixels, mul(channels.max(cc + 1), sample * 3 + 12)?)?;
        budget.scratch(converted.checked_add(inflate_scratch(merged, 1)?).ok_or_else(overflow)?);
    }
    let mut patterns = photocraft_psd::patterns::PatternBudget::default();
    for block in file.global_blocks.iter().filter(|b| crate::pattern_map::PATTERN_KEYS.contains(&b.key)) {
        // Earlier records can decode before a malformed later record discards the block.
        let items = photocraft_psd::patterns::pattern_block_memory(&block.data, &mut patterns);
        for p in items {
            let pixels = mul(u64::from(p.width), u64::from(p.height))?;
            let sample = u64::from(p.depth.div_ceil(8));
            let color = if p.mode == 2 { 3 } else { photocraft_psd::patterns::mode_channels(p.mode) as u64 };
            let alpha = u64::from(p.planes > photocraft_psd::patterns::mode_channels(p.mode));
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
        return Err(IoError::PsdImportBudget { required: est.total_bytes, limit });
    }
    Ok(est)
}

#[cfg(test)]
mod tests;
