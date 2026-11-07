//! RW2 RawFormat 8: file-described prefix codes and independently predicted 2×2 cells.
//!
//! Layout evidence: LightCraft's measured format notes and fixes in
//! storytold/lightcraft#215, #255 and #1304 (rw2.rs at 12c096669ab76556dcbaf6dbeb8704a3b96f646d).
//! This implementation uses PhotoCraft's TIFF reader and allocation limits. In particular,
//! cells are column-major, and tag 0x0045 contains (x, y) pairs, including tiled high-res files.
//! See docs/rw2-format8.md for provenance, supported layouts and validation.

use crate::error::{RawError, Result};
use crate::tiff::{Ifd, Tiff};
use crate::{Limits, par};

const MAX_TILES: usize = 64;
const PREFIX_BITS: u32 = 12;
const TABLE_SIZE: usize = 1 << PREFIX_BITS;
// These fields have no established interpretation beyond the values observed by LightCraft.
// Reject different values instead of interpreting an unknown predictor as this codec.
const FIXED: [(u16, &[u16]); 8] = [
    (0x0039, &[6, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1]),
    (0x003a, &[6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
    (0x003b, &[65535]),
    (0x003c, &[0]),
    (0x003d, &[0]),
    (0x003e, &[0]),
    (0x003f, &[0]),
    (0x0041, &[17, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
];

/// Attach the codec name to a malformed-container or bitstream error.
fn malformed(message: &str) -> RawError {
    RawError::malformed(format!("RW2 format 8: {message}"))
}

/// Read only bounded SHORTs or packed u16 bytes, never an input-sized TIFF value vector.
fn words(t: &Tiff, ifd: &Ifd, tag: u16) -> Result<Vec<u16>> {
    let e = ifd.get(tag).ok_or_else(|| malformed(&format!("missing tag {tag:#06x}")))?;
    let count = match e.typ {
        3 => e.count as usize,
        1 | 7 if e.count.is_multiple_of(2) => e.count as usize / 2,
        _ => return Err(malformed(&format!("invalid word array in tag {tag:#06x}"))),
    };
    if count == 0 || count > 1 + 2 * MAX_TILES {
        return Err(malformed(&format!("oversized or empty tag {tag:#06x}")));
    }
    (0..count).map(|i| e.at.checked_add(i * 2).and_then(|at| t.u16_at(at)).ok_or_else(|| malformed("truncated tag"))).collect()
}

/// Read a bounded count-prefixed list, combining low/high words for wide entries.
fn list(t: &Tiff, ifd: &Ifd, tag: u16, wide: bool) -> Result<Vec<u32>> {
    let data = words(t, ifd, tag)?;
    let n = usize::from(data.first().copied().unwrap_or(0));
    let stride = if wide { 2 } else { 1 };
    if n == 0 || n > MAX_TILES || data.len() < 1 + n * stride {
        return Err(malformed(&format!("invalid count in tag {tag:#06x}")));
    }
    let mut out = Vec::with_capacity(n);
    for values in data.get(1..1 + n * stride).unwrap_or_default().chunks_exact(stride) {
        let lo = u32::from(values.first().copied().unwrap_or(0));
        let hi = u32::from(values.get(1).copied().unwrap_or(0));
        out.push(lo | (hi << 16));
    }
    Ok(out)
}

/// A borrowed payload and its rectangle in the sensor plane.
struct Tile<'a> {
    data: &'a [u8],
    bits: u64,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

/// Validate payload ranges and tile geometry, requiring exact non-overlapping sensor coverage.
fn tiles<'a>(t: &Tiff<'a>, ifd: &Ifd, width: usize, height: usize) -> Result<Vec<Tile<'a>>> {
    let offsets = list(t, ifd, 0x0044, true)?;
    let origins = list(t, ifd, 0x0045, true)?;
    let lengths = list(t, ifd, 0x0046, true)?;
    let widths = list(t, ifd, 0x0047, false)?;
    let heights = list(t, ifd, 0x0048, false)?;
    if [origins.len(), lengths.len(), widths.len(), heights.len()].iter().any(|&n| n != offsets.len()) {
        return Err(malformed("tile lists have different lengths"));
    }
    let mut out = Vec::with_capacity(offsets.len());
    let mut area = 0u64;
    for ((((offset, origin), bits), w), h) in offsets.into_iter().zip(origins).zip(lengths).zip(widths).zip(heights) {
        let (x, y) = ((origin & 0xffff) as usize, (origin >> 16) as usize);
        let (w, h) = (w as usize, h as usize);
        if w == 0 || h == 0 || x.checked_add(w).is_none_or(|v| v > width) || y.checked_add(h).is_none_or(|v| v > height) {
            return Err(malformed("tile lies outside the sensor"));
        }
        let bits = u64::from(bits);
        let size = usize::try_from(bits.div_ceil(8)).map_err(|_| malformed("tile length overflow"))?;
        let data = t.bytes(offset as usize, size).filter(|_| bits > 0).ok_or_else(|| malformed("tile lies outside the file"))?;
        area += (w as u64) * (h as u64);
        out.push(Tile { data, bits, x, y, width: w, height: h });
    }
    let overlap = out
        .iter()
        .enumerate()
        .any(|(i, a)| out.iter().skip(i + 1).any(|b| a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height));
    // Without exact, non-overlapping coverage a successful import could silently contain holes.
    if overlap || area != (width as u64) * (height as u64) {
        return Err(malformed("tiles do not cover the sensor exactly once"));
    }
    let mut tops: Vec<_> = out.iter().map(|tile| tile.y).collect();
    tops.sort_unstable();
    tops.dedup();
    if ifd.has(0x0043) && words(t, ifd, 0x0043)? != [tops.len() as u16] {
        return Err(RawError::unsupported("RW2 format 8: unknown tile-row layout"));
    }
    Ok(out)
}

/// One lookup-table entry: the consumed prefix length and signed-difference category.
#[derive(Clone, Copy, Default)]
struct Code {
    length: u8,
    category: u8,
}

/// Expand the file's prefix codes into a fixed lookup table, preserving shorter-prefix priority.
fn codes(t: &Tiff, ifd: &Ifd) -> Result<[Code; TABLE_SIZE]> {
    let data = words(t, ifd, 0x0040)?;
    if data.first() != Some(&17) || data.len() < 35 {
        return Err(malformed("expected 17 difference categories"));
    }
    let pairs: Vec<_> = data.get(1..35).unwrap_or_default().as_chunks::<2>().0.iter().map(|&[length, value]| (length, value)).collect();
    let mut table = [Code::default(); TABLE_SIZE];
    for (category, &(length, value)) in pairs.iter().enumerate() {
        if u32::from(length) > PREFIX_BITS {
            return Err(RawError::unsupported("RW2 format 8: prefix longer than 12 bits"));
        }
        if length == 0 {
            continue;
        }
        if value >> length != 0 || pairs.iter().take(category).any(|&p| p == (length, value)) {
            return Err(malformed("invalid or duplicate prefix code"));
        }
        // Camera 12/14-bit tables include unreachable longer codes. A shorter prefix wins,
        // just as in bit-by-bit decoding; rejecting those tables would reject real cameras.
        let start = usize::from(value) << (PREFIX_BITS - u32::from(length));
        let count = 1 << (PREFIX_BITS - u32::from(length));
        let entries = table.get_mut(start..start + count).ok_or_else(|| malformed("invalid prefix range"))?;
        for entry in entries {
            if entry.length == 0 || entry.length > length as u8 {
                *entry = Code { length: length as u8, category: category as u8 };
            }
        }
    }
    Ok(table)
}

/// Bytes arrive LSB-first; the reservoir presents the codes in their usual MSB-first order.
/// Lookup can zero-pad its lookahead, but consuming a code or value must stay inside `remaining`.
struct Bits<'a> {
    bytes: std::slice::Iter<'a, u8>,
    remaining: u64,
    reservoir: u64,
    buffered: u32,
}

impl<'a> Bits<'a> {
    /// Start a tile stream with its exact declared bit length, excluding trailing byte padding.
    fn new(tile: &Tile<'a>) -> Self {
        Self { bytes: tile.data.iter(), remaining: tile.bits, reservoir: 0, buffered: 0 }
    }

    /// Look ahead at up to 16 bits without consuming them; absent lookahead bits are zero.
    fn peek(&mut self, n: u32) -> u32 {
        while self.buffered < n {
            let Some(byte) = self.bytes.next() else { break };
            self.reservoir = (self.reservoir << 8) | u64::from(byte.reverse_bits());
            self.buffered += 8;
        }
        let value = if self.buffered >= n { self.reservoir >> (self.buffered - n) } else { self.reservoir << (n - self.buffered) };
        (value & ((1u64 << n) - 1)) as u32
    }

    /// Consume bits only when both the declared bit length and stored bytes contain them.
    fn take(&mut self, n: u32) -> Option<u32> {
        if u64::from(n) > self.remaining {
            return None;
        }
        let value = self.peek(n);
        if self.buffered < n {
            return None;
        }
        self.buffered -= n;
        self.remaining -= u64::from(n);
        Some(value)
    }

    /// Decode one T.81 signed difference using the file's category prefix and magnitude bits.
    fn difference(&mut self, codes: &[Code; TABLE_SIZE]) -> Option<i32> {
        let code = codes.get(self.peek(PREFIX_BITS) as usize)?;
        if code.length == 0 {
            return None;
        }
        self.take(u32::from(code.length))?;
        let n = u32::from(code.category);
        if n == 0 {
            return Some(0);
        }
        let value = self.take(n)? as i32;
        Some(if value < 1 << (n - 1) { value - (1 << n) + 1 } else { value })
    }
}

/// Reconstruct a tile's column-major cells, rejecting samples outside the declared sensor range.
fn decode_tile(tile: &Tile, table: &[Code; TABLE_SIZE], max_sample: u16) -> Result<Vec<u16>> {
    let mut data = vec![0; tile.width * tile.height];
    let mut bits = Bits::new(tile);
    let mut first = [0i32; 4];
    for row in (0..tile.height).step_by(2) {
        let mut predicted = first;
        for column in (0..tile.width).step_by(2) {
            for (site, value) in predicted.iter_mut().enumerate() {
                let delta = bits.difference(table).ok_or_else(|| malformed("invalid or truncated difference"))?;
                *value = value.checked_add(delta).ok_or_else(|| malformed("predictor overflow"))?;
                // A u16 conversion alone admits malformed 12/14-bit values above the sensor range.
                let sample =
                    u16::try_from(*value).ok().filter(|&sample| sample <= max_sample).ok_or_else(|| malformed("sample outside the declared bit depth"))?;
                let (x, y) = (column + site / 2, row + site % 2);
                if x < tile.width && y < tile.height {
                    let pixel = data.get_mut(y * tile.width + x).ok_or_else(|| malformed("sample outside tile"))?;
                    *pixel = sample;
                }
            }
            if column == 0 {
                first = predicted;
            }
        }
    }
    Ok(data)
}

/// Decode the validated tiles into one sensor plane, charging both retained and final buffers.
pub(super) fn decode(t: &Tiff, ifd: &Ifd, width: usize, height: usize, depth: u32, limits: &Limits) -> Result<Vec<u16>> {
    if !matches!(depth, 12 | 14 | 16) {
        return Err(RawError::unsupported(format!("RW2 format 8 with {depth}-bit samples")));
    }
    let max_sample = ((1u32 << depth) - 1) as u16;
    for (tag, expected) in FIXED {
        if ifd.has(tag) && words(t, ifd, tag)? != expected {
            return Err(RawError::unsupported(format!("RW2 format 8 with unknown tag {tag:#06x} values")));
        }
    }
    let tiles = tiles(t, ifd, width, height)?;
    let table = codes(t, ifd)?;
    // Parallel tile buffers and the assembled plane coexist. Charge both before allocating;
    // metadata is capped at 64 tiles and the lookup table has a fixed size.
    limits.check(width as u64, height as u64, 4)?;
    let decoded = par::map(tiles.len(), |i| tiles.get(i).ok_or_else(|| malformed("missing tile")).and_then(|tile| decode_tile(tile, &table, max_sample)));
    let decoded: Vec<_> = decoded.into_iter().collect::<Result<_>>()?;
    let mut out = vec![0; width * height];
    for (tile, pixels) in tiles.iter().zip(decoded) {
        for (row, source) in pixels.chunks_exact(tile.width).enumerate() {
            let start = (tile.y + row) * width + tile.x;
            let target = out.get_mut(start..start + tile.width).ok_or_else(|| malformed("tile write outside sensor"))?;
            target.copy_from_slice(source);
        }
    }
    Ok(out)
}
