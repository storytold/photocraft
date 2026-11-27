//! Exact positive-alpha coverage, shared with a tile's backing bytes rather than a query rect.

use photocraft_color::{PixelFormat, SampleType, read_sample};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};

use crate::Surface;

const SIDE: usize = TILE_SIZE as usize;
const WORD_BITS: usize = u64::BITS as usize;
const WORDS_PER_ROW: usize = SIDE / WORD_BITS;
const WORDS: usize = SIDE * WORDS_PER_ROW;

pub(crate) struct AlphaIndex {
    pub(crate) format: PixelFormat,
    coverage: Coverage,
}

enum Coverage {
    Empty,
    Full,
    Bits(Vec<u64>),
}

/// Integer alpha only needs a zero test; float alpha follows the same `> 0` predicate as rgba.
fn positive_alpha(pixel: &[u8], format: PixelFormat) -> bool {
    let offset = format.mode.color_channels() * format.sample.bytes();
    let Some(alpha) = pixel.get(offset..offset + format.sample.bytes()) else { return false };
    match format.sample {
        SampleType::U8 | SampleType::U16 => alpha.iter().any(|&byte| byte != 0),
        SampleType::F32 => alpha.try_into().is_ok_and(|bytes| f32::from_ne_bytes(bytes) > 0.0),
    }
}

impl AlphaIndex {
    /// At most one bit per tile pixel; uniformly empty/full tiles need no heap storage.
    /// A failed reservation leaves the caller free to answer by scanning the requested region.
    pub(crate) fn build(bytes: &[u8], format: PixelFormat) -> Option<Self> {
        let mut words = [0u64; WORDS];
        for (i, pixel) in bytes.chunks_exact(format.bytes_per_pixel()).take(SIDE * SIDE).enumerate() {
            if positive_alpha(pixel, format) {
                // i is bounded by the fixed tile dimensions, not the input byte length.
                if let Some(word) = words.get_mut(i / WORD_BITS) {
                    *word |= 1 << (i % WORD_BITS);
                }
            }
        }
        let coverage = if words.iter().all(|&word| word == 0) {
            Coverage::Empty
        } else if words.iter().all(|&word| word == u64::MAX) {
            Coverage::Full
        } else {
            let mut bits = Vec::new();
            bits.try_reserve_exact(WORDS).ok()?;
            bits.extend_from_slice(&words);
            Coverage::Bits(bits)
        };
        Some(Self { format, coverage })
    }

    pub(crate) fn allocated_bytes(&self) -> usize {
        match &self.coverage {
            Coverage::Bits(bits) => bits.capacity() * size_of::<u64>(),
            Coverage::Empty | Coverage::Full => 0,
        }
    }

    pub(crate) fn contains(&self, area: Rect) -> bool {
        let area = area.intersect(&Rect::new(0, 0, TILE_SIZE, TILE_SIZE));
        if area.is_empty() {
            return false;
        }
        let Coverage::Bits(words) = &self.coverage else { return matches!(self.coverage, Coverage::Full) };
        let first = area.x0 as usize / WORD_BITS;
        let last = (area.x1 as usize - 1) / WORD_BITS;
        let left = u64::MAX << (area.x0 as usize % WORD_BITS);
        let right = u64::MAX >> (WORD_BITS - 1 - (area.x1 as usize - 1) % WORD_BITS);
        for y in area.y0 as usize..area.y1 as usize {
            let Some(row) = words.get(y * WORDS_PER_ROW + first..=y * WORDS_PER_ROW + last) else { continue };
            for (i, &word) in row.iter().enumerate() {
                let mask = (if i == 0 { left } else { u64::MAX }) & (if i == last - first { right } else { u64::MAX });
                if word & mask != 0 {
                    return true;
                }
            }
        }
        false
    }
}

/// Exact, allocation-free fallback if the index cannot reserve its fixed-size bit buffer.
pub(crate) fn scan(bytes: &[u8], format: PixelFormat, area: Rect) -> bool {
    let bpp = format.bytes_per_pixel();
    for y in area.y0 as usize..area.y1 as usize {
        let start = (y * SIDE + area.x0 as usize) * bpp;
        let end = (y * SIDE + area.x1 as usize) * bpp;
        if bytes.get(start..end.min(bytes.len())).is_some_and(|row| row.chunks_exact(bpp).any(|pixel| positive_alpha(pixel, format))) {
            return true;
        }
    }
    false
}

impl Surface {
    /// Whether any pixel in the half-open `area` has alpha strictly greater than zero.
    /// Formats without alpha are opaque, including missing tiles; an empty area is never a hit.
    ///
    /// Stored tiles lazily index their encoded alpha, then query exact bits without decoding
    /// pixels or allocating. Heap-backed indexes count toward residency and are discarded with
    /// spilled pixels; allocation-free uniform coverage can stay warm on disk. Every byte write
    /// invalidates that backing's index, including raw tile writes and copy-on-write edits.
    /// Moving or replacing tiles carries the appropriate index. Huge sparse areas visit stored
    /// tiles, never an unbounded grid of missing tiles. Failed scratch reads invalidate coverage
    /// and never seed an index from the display API's temporary transparent placeholder.
    pub fn has_positive_alpha(&self, area: Rect) -> bool {
        if area.is_empty() {
            return false;
        }
        if !self.format.alpha {
            return true;
        }
        let default_positive = read_sample(&self.default_pixel, self.format.sample, self.format.mode.color_channels()) > 0.0;
        let tile_count = area.tile_count();
        let has_ink = |coord: TileCoord, tile: &crate::Tile| {
            let origin = coord.rect();
            let overlap = area.intersect(&origin);
            // Saturating rects of out-of-range tile coordinates must not alias addressable pixels.
            if overlap.is_empty() || TileCoord::containing(overlap.x0, overlap.y0) != coord {
                return false;
            }
            let local = Rect::new(overlap.x0 - origin.x0, overlap.y0 - origin.y0, overlap.x1 - origin.x0, overlap.y1 - origin.y0);
            tile.cell.has_positive_alpha(self.format, local)
        };
        if tile_count <= self.tiles.len() as u64 {
            area.tiles().any(|coord| self.tiles.get(&coord).map_or(default_positive, |tile| has_ink(coord, tile)))
        } else {
            // More query tiles than stored tiles guarantees a missing tile, without searching
            // the empty grid. Transparent defaults need only the stored tiles' exact coverage.
            default_positive || self.tiles.iter().any(|(&coord, tile)| has_ink(coord, tile))
        }
    }
}
