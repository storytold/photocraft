//! Fujifilm's lossless RAF compression (the `IS` strips written by X-Trans III
//! and later bodies, and by the GFX cameras).
//!
//! Sources (clean-room): Fabian Tamp's published description of the scheme
//! (<https://capnfabs.net/posts/fuji-raf-compression-algorithm/>): the image is
//! cut into vertical blocks that are coded independently, each block into lines
//! of six sensor rows whose samples are collated into colour vectors, two colour
//! vectors are coded interleaved with the even positions first and the odd
//! positions five steps behind, each sample is predicted from already coded
//! neighbours, and the difference is sent as a unary count plus a fixed number
//! of bits, that number following the mean difference seen so far in one of 81
//! gradient buckets, with a run of 41 zeros escaping to a direct 14-bit value.
//! Everything else here (the header, the colour-vector layout, the predictors,
//! the bucket thresholds, the initial statistics, the border handling, the
//! escape payload, and the 16-bit scaling) was established by observation of
//! sample files: an X-T30 file and the CC0 raw.pixls.us X-T30, GFX 50S and
//! GFX 100 samples. The code is self-checking: a wrong detail desynchronises
//! the bit stream within a few samples, and a right decoder consumes each
//! block's byte count to the padding, so every detail below was fixed by that
//! test. No decoder source was consulted.
//!
//! The stream, all big-endian:
//!
//! * A 16-byte header: `IS`, version (1 = lossless), raw type (16 = X-Trans,
//!   0 = Bayer), bits per sample, height, width rounded up to whole blocks,
//!   width, block width in columns, block count, line count (height / 6).
//! * One 32-bit byte count per block, then (from the next 16-byte boundary)
//!   the blocks back to back, each padded to 16 bytes.
//! * A block codes its six-row lines in order. A line holds 12 colour vectors
//!   (3 red, 6 green, 3 blue), `line_width` samples each: the block width
//!   halved for Bayer (one vector per colour per row) or two thirds of it for
//!   X-Trans, where column `c` of a six-column cell lands in slot
//!   `(2c + 1) / 3`, green vectors take one row each and red and blue vectors
//!   a pair of rows, so a slot that holds no sensor sample is interpolated by
//!   the decoder rather than coded. Six passes code the pairs (R2, G2),
//!   (G3, B2), (R3, G4), (G5, B3), (R4, G6), (G7, B4); the passes use three
//!   gradient-context sets in turn. Two earlier vectors of each colour are kept
//!   for prediction; they start as zeros.
//! * Even positions are predicted from the vector above (left, centre, right)
//!   and the one above that; odd positions from both same-vector neighbours and
//!   the vector above. The context is the quantised pair of gradients
//!   (|d| < 18, 67, 276 → 1, 2, 3, else 4, signed), 9·q1 + q2; a negative
//!   index negates the residual. Each context keeps a sum of absolute residuals
//!   (starting at 2^(bits − 6)) and a count (starting at 1, halving both when it
//!   reaches 64); the suffix length is the smallest `k` with `count << k` ≥ sum
//!   (at most bits − 1). The residual's zigzag code `v` is sent as `v >> k`
//!   zeros, a one, and `k` low bits; 3·bits − 1 or more zeros mean a one and
//!   then `v − 1` in `bits` bits.

use crate::error::{RawError, Result};
use crate::sensor::Cfa;
use crate::{Limits, par};

/// The first two bytes of a compressed strip.
pub(crate) const SIGNATURE: &[u8] = b"IS";

/// Sensor rows per coded line.
const ROWS_PER_LINE: usize = 6;
/// Colour vectors kept per block: R0–R4, G0–G7, B0–B4 (two previous and the
/// current line's per colour).
const NLINES: usize = 18;
const R0: usize = 0;
const R2: usize = 2;
const R3: usize = 3;
const R4: usize = 4;
const G0: usize = 5;
const G2: usize = 7;
const G3: usize = 8;
const G4: usize = 9;
const G5: usize = 10;
const G6: usize = 11;
const G7: usize = 12;
const B0: usize = 13;
const B2: usize = 15;
const B3: usize = 16;
const B4: usize = 17;
/// The six passes of a line: the two vectors and the gradient set each uses.
const PASSES: [(usize, usize, usize); 6] = [(R2, G2, 0), (G3, B2, 1), (R3, G4, 2), (G5, B3, 0), (R4, G6, 1), (G7, B4, 2)];
/// Odd positions start once the even position has passed this (after five evens), or
/// once the evens are done in a vector shorter than that.
const ODD_START: usize = 8;
/// A context's count halves (with its sum) when it reaches this.
const CONTEXT_MAX_COUNT: i32 = 64;
/// Gradient quantisation thresholds (the same at 14 and 16 bits).
const Q_THRESHOLDS: [i32; 3] = [18, 67, 276];
/// Contexts per set: |9·q1 + q2| for q in −4..=4.
const CONTEXTS: usize = 41;

fn be16(b: &[u8], at: usize) -> Option<usize> {
    let s: [u8; 2] = b.get(at..at.checked_add(2)?)?.try_into().ok()?;
    Some(usize::from(u16::from_be_bytes(s)))
}

fn be32(b: &[u8], at: usize) -> Option<usize> {
    let s: [u8; 4] = b.get(at..at.checked_add(4)?)?.try_into().ok()?;
    usize::try_from(u32::from_be_bytes(s)).ok()
}

/// The stream header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Header {
    pub version: u8,
    pub raw_type: u8,
    pub bits: u32,
    pub height: usize,
    pub rounded_width: usize,
    pub width: usize,
    pub block_size: usize,
    pub total_lines: usize,
    /// Byte count of each block.
    pub sizes: Vec<usize>,
    /// Offset of the first block.
    pub data_start: usize,
}

pub(crate) fn header(strip: &[u8]) -> Result<Header> {
    let bad = || RawError::malformed("Fujifilm compressed RAF: truncated stream header");
    if !strip.starts_with(SIGNATURE) {
        return Err(RawError::malformed("Fujifilm compressed RAF: missing IS signature"));
    }
    let byte = |at: usize| strip.get(at).copied().ok_or_else(bad);
    let (version, raw_type, bits) = (byte(2)?, byte(3)?, u32::from(byte(4)?));
    let height = be16(strip, 5).ok_or_else(bad)?;
    let rounded_width = be16(strip, 7).ok_or_else(bad)?;
    let width = be16(strip, 9).ok_or_else(bad)?;
    let block_size = be16(strip, 11).ok_or_else(bad)?;
    let blocks = usize::from(byte(13)?);
    let total_lines = be16(strip, 14).ok_or_else(bad)?;
    let sizes = (0..blocks).map(|i| be32(strip, 16 + 4 * i).ok_or_else(bad)).collect::<Result<Vec<usize>>>()?;
    let data_start = (16 + 4 * blocks).next_multiple_of(16);
    Ok(Header { version, raw_type, bits, height, rounded_width, width, block_size, total_lines, sizes, data_start })
}

/// Per-bit-depth constants.
#[derive(Debug, Clone, Copy)]
struct Params {
    bits: u32,
    /// `2^bits - 1`.
    max: i32,
    /// `2^bits`.
    total: i32,
    /// Initial residual sum of every context.
    init_sum: i32,
    /// Zero run that escapes to a direct value.
    escape: u32,
    /// Longest suffix.
    max_suffix: u32,
}

impl Params {
    fn new(bits: u32) -> Result<Params> {
        if !matches!(bits, 14 | 16) {
            return Err(RawError::unsupported(format!("Fujifilm compressed RAF with {bits}-bit samples (14 and 16 are decoded)")));
        }
        Ok(Params { bits, max: (1 << bits) - 1, total: 1 << bits, init_sum: 1 << (bits - 6), escape: 3 * bits - 1, max_suffix: bits - 1 })
    }

    /// Quantised gradient: 0, ±1..±4.
    #[inline]
    fn quant(v: i32) -> i32 {
        let a = v.abs();
        let q = if a == 0 {
            0
        } else if a < Q_THRESHOLDS[0] {
            1
        } else if a < Q_THRESHOLDS[1] {
            2
        } else if a < Q_THRESHOLDS[2] {
            3
        } else {
            4
        };
        if v < 0 { -q } else { q }
    }

    /// Context index and whether the residual is negated.
    #[inline]
    fn context(v1: i32, v2: i32) -> (usize, bool) {
        let g = 9 * Self::quant(v1) + Self::quant(v2);
        (g.unsigned_abs() as usize, g < 0)
    }
}

/// One gradient context's statistics.
#[derive(Debug, Clone, Copy)]
struct Context {
    sum: i32,
    count: i32,
}

impl Context {
    /// Smallest `k` with `count << k >= sum`, capped.
    #[inline]
    fn suffix_bits(&self, cap: u32) -> u32 {
        let mut k = 0;
        if self.count < self.sum {
            while k < cap {
                k += 1;
                if (self.count << k) >= self.sum {
                    break;
                }
            }
        }
        k
    }

    #[inline]
    fn update(&mut self, abs_residual: i32) {
        self.sum = self.sum.saturating_add(abs_residual);
        if self.count == CONTEXT_MAX_COUNT {
            self.sum >>= 1;
            self.count >>= 1;
        }
        self.count += 1;
    }
}

/// Which sensor samples the colour vectors hold.
#[derive(Debug, Clone)]
struct Geometry {
    xtrans: bool,
    /// (vector, slot within the cell) of each (row, column) of the cell.
    map: [[(usize, usize); 6]; 6],
    /// Per vector, which slots of a cell hold a sensor sample (bit per slot).
    real: [u8; NLINES],
    /// (row, column) within the cell behind each (vector, slot), if any.
    inverse: [[Option<(usize, usize)>; 4]; NLINES],
}

impl Geometry {
    fn new(cfa: &Cfa, raw_type: u8) -> Result<Geometry> {
        let xtrans = match (cfa.width, cfa.height, raw_type) {
            (6, 6, 16) => true,
            (2, 2, 0) => false,
            (w, h, t) => return Err(RawError::unsupported(format!("Fujifilm compressed RAF with raw type {t} and a {w}x{h} CFA"))),
        };
        let cols = if xtrans { 6 } else { 2 };
        let mut map = [[(0usize, 0usize); 6]; 6];
        let mut real = [0u8; NLINES];
        let mut inverse = [[None; 4]; NLINES];
        for r in 0..ROWS_PER_LINE {
            for c in 0..cols {
                let slot = if xtrans { (2 * c + 1) / 3 } else { 0 };
                let line = match cfa.color(c, r) {
                    0 => R2 + r / 2,
                    1 => G2 + r,
                    _ => B2 + r / 2,
                };
                let (Some(mask), Some(inv)) = (real.get_mut(line), inverse.get_mut(line).and_then(|v| v.get_mut(slot))) else {
                    return Err(RawError::malformed("Fujifilm compressed RAF: CFA layout out of range"));
                };
                if *mask & (1 << slot) != 0 {
                    return Err(RawError::unsupported("Fujifilm compressed RAF: CFA layout maps two samples to one slot"));
                }
                *mask |= 1 << slot;
                *inv = Some((r, c));
                if let Some(m) = map.get_mut(r).and_then(|row| row.get_mut(c)) {
                    *m = (line, slot);
                }
            }
        }
        // The odd pass never interpolates: every odd slot must hold a sample.
        let odd_slots: u8 = if xtrans { 0b1010 } else { 0b1 };
        for line in [R2, R3, R4, G2, G3, G4, G5, G6, G7, B2, B3, B4] {
            if real.get(line).is_some_and(|m| m & odd_slots != odd_slots) {
                return Err(RawError::unsupported("Fujifilm compressed RAF: unexpected CFA layout"));
            }
        }
        Ok(Geometry { xtrans, map, real, inverse })
    }

    fn cell_cols(&self) -> usize {
        if self.xtrans { 6 } else { 2 }
    }

    fn slots_per_cell(&self) -> usize {
        if self.xtrans { 4 } else { 1 }
    }

    /// Colour vector length for a block `block_size` columns wide.
    fn line_width(&self, block_size: usize) -> Result<usize> {
        let (cols, slots) = (self.cell_cols(), self.slots_per_cell());
        let lw = block_size / cols * slots;
        if block_size == 0 || !block_size.is_multiple_of(cols) || !lw.is_multiple_of(2) || lw < 2 {
            return Err(RawError::unsupported(format!("Fujifilm compressed RAF with {block_size}-column blocks")));
        }
        Ok(lw)
    }

    /// Whether slot `pos` of `line` holds a sensor sample.
    #[inline]
    fn is_real(&self, line: usize, pos: usize) -> bool {
        let slot = pos % self.slots_per_cell();
        self.real.get(line).is_some_and(|m| m & (1 << slot) != 0)
    }

    /// Column within the block of vector `line`'s slot `pos`, and the row of
    /// the six it comes from.
    #[inline]
    fn source(&self, line: usize, pos: usize) -> Option<(usize, usize)> {
        let spc = self.slots_per_cell();
        let (r, c) = (*self.inverse.get(line)?.get(pos % spc)?)?;
        Some((r, pos / spc * self.cell_cols() + c))
    }
}

/// Reads the bit stream most significant bit first; bits past the end read as
/// zero and are counted, so a truncated block is detected afterwards.
struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> BitReader<'a> {
    /// The next 32 bits.
    #[inline]
    fn peek32(&self) -> u32 {
        let byte = self.pos >> 3;
        let shift = (self.pos & 7) as u32;
        let mut w: u64 = 0;
        for k in 0..5 {
            w = (w << 8) | u64::from(self.data.get(byte + k).copied().unwrap_or(0));
        }
        ((w >> (8 - shift)) & 0xFFFF_FFFF) as u32
    }

    #[inline]
    fn read(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        let v = self.peek32() >> (32 - n.min(32));
        self.pos += n as usize;
        v
    }

    /// Counts zeros up to a one (consumed); at `cap` zeros it stops counting
    /// and consumes one more bit.
    #[inline]
    fn zeros(&mut self, cap: u32) -> u32 {
        let mut n = 0;
        loop {
            let z = self.peek32().leading_zeros();
            if n + z >= cap {
                self.pos += (cap - n) as usize + 1;
                return cap;
            }
            if z < 32 {
                self.pos += z as usize + 1;
                return n + z;
            }
            n += 32;
            self.pos += 32;
        }
    }

    fn overran(&self) -> bool {
        self.pos > self.data.len().saturating_mul(8)
    }
}

/// Writes a bit stream most significant bit first.
#[cfg(any(test, feature = "testgen"))]
#[derive(Default)]
struct BitWriter {
    out: Vec<u8>,
    acc: u64,
    nbits: u32,
}

#[cfg(any(test, feature = "testgen"))]
impl BitWriter {
    fn put(&mut self, value: u32, n: u32) {
        for i in (0..n).rev() {
            self.acc = (self.acc << 1) | u64::from((value >> i) & 1);
            self.nbits += 1;
            if self.nbits == 8 {
                self.out.push(self.acc as u8);
                self.acc = 0;
                self.nbits = 0;
            }
        }
    }

    fn zeros(&mut self, n: u32) {
        for _ in 0..n {
            self.put(0, 1);
        }
    }

    /// Pads to a 16-byte boundary and returns the bytes.
    fn finish(mut self) -> Vec<u8> {
        while self.nbits != 0 {
            self.put(0, 1);
        }
        let len = self.out.len().next_multiple_of(16);
        self.out.resize(len, 0);
        self.out
    }
}

/// Reads residuals (decoding) or writes them (encoding).
trait Codec {
    /// The value of a sample predicted as `pred` in context `ctx`; `truth` is
    /// the sample the encoder must code.
    fn sample(&mut self, ctx: &mut Context, p: &Params, pred: i32, negate: bool, truth: Option<i32>) -> Result<i32>;
}

struct Decoder<'a>(BitReader<'a>);

impl Codec for Decoder<'_> {
    #[inline]
    fn sample(&mut self, ctx: &mut Context, p: &Params, pred: i32, negate: bool, _truth: Option<i32>) -> Result<i32> {
        let zeros = self.0.zeros(p.escape);
        let code = if zeros < p.escape {
            let k = ctx.suffix_bits(p.max_suffix);
            ((zeros as i32) << k) | self.0.read(k) as i32
        } else {
            self.0.read(p.bits) as i32 + 1
        };
        if code > p.max {
            return Err(RawError::malformed("Fujifilm compressed RAF: residual out of range (corrupt stream)"));
        }
        let residual = if code & 1 == 1 { -1 - code / 2 } else { code / 2 };
        ctx.update(residual.abs());
        let mut v = if negate { pred - residual } else { pred + residual };
        if v < 0 {
            v += p.total;
        } else if v > p.max {
            v -= p.total;
        }
        if v < 0 || v > p.max {
            return Err(RawError::malformed("Fujifilm compressed RAF: sample out of range (corrupt stream)"));
        }
        Ok(v)
    }
}

#[cfg(any(test, feature = "testgen"))]
struct Encoder(BitWriter);

#[cfg(any(test, feature = "testgen"))]
impl Codec for Encoder {
    fn sample(&mut self, ctx: &mut Context, p: &Params, pred: i32, negate: bool, truth: Option<i32>) -> Result<i32> {
        let want = truth.ok_or_else(|| RawError::malformed("encoder without a sample"))?.clamp(0, p.max);
        let mut residual = if negate { pred - want } else { want - pred };
        let half = p.total / 2;
        if residual < -half {
            residual += p.total;
        } else if residual >= half {
            residual -= p.total;
        }
        let code = if residual >= 0 { 2 * residual } else { -2 * residual - 1 };
        let k = ctx.suffix_bits(p.max_suffix);
        let q = (code >> k) as u32;
        if q < p.escape {
            self.0.zeros(q);
            self.0.put(1, 1);
            self.0.put((code & ((1 << k) - 1)) as u32, k);
        } else {
            self.0.zeros(p.escape);
            self.0.put(1, 1);
            self.0.put((code - 1) as u32, p.bits);
        }
        ctx.update(residual.abs());
        Ok(want)
    }
}

/// The state of one block while its lines are coded.
struct Block<'g, C: Codec> {
    codec: C,
    p: Params,
    g: &'g Geometry,
    lw: usize,
    /// `NLINES` vectors of `lw + 2` samples: a border sample each side.
    lines: Vec<u16>,
    even: [[Context; CONTEXTS]; 3],
    odd: [[Context; CONTEXTS]; 3],
    /// The samples the encoder codes: image, stride, first column of the block,
    /// image width.
    truth: Option<(&'g [u16], usize, usize, usize)>,
    /// The first sensor row of the current line.
    row0: usize,
}

impl<'g, C: Codec> Block<'g, C> {
    fn new(codec: C, p: Params, g: &'g Geometry, lw: usize, truth: Option<(&'g [u16], usize, usize, usize)>) -> Self {
        let ctx = Context { sum: p.init_sum, count: 1 };
        Block { codec, p, g, lw, lines: vec![0; NLINES * (lw + 2)], even: [[ctx; CONTEXTS]; 3], odd: [[ctx; CONTEXTS]; 3], truth, row0: 0 }
    }

    /// Sample `i` of vector `line` (0 and `lw + 1` are the borders).
    #[inline]
    fn at(&self, line: usize, i: usize) -> i32 {
        self.lines.get(line * (self.lw + 2) + i).copied().map_or(0, i32::from)
    }

    #[inline]
    fn set(&mut self, line: usize, i: usize, v: i32) {
        let stride = self.lw + 2;
        if let Some(s) = self.lines.get_mut(line * stride + i) {
            *s = v.clamp(0, i32::from(u16::MAX)) as u16;
        }
    }

    /// The encoder's sample for slot `pos` of `line`, replicated past the
    /// image's right edge.
    fn truth(&self, line: usize, pos: usize) -> Option<i32> {
        let (image, stride, x0, width) = self.truth?;
        let (r, c) = self.g.source(line, pos)?;
        let x = (x0 + c).min(width.saturating_sub(1));
        image.get((self.row0 + r) * stride + x).copied().map(i32::from)
    }

    /// The even predictor's sum (four times the prediction) and the gradient
    /// context of slot `pos`.
    #[inline]
    fn even_sum(&self, line: usize, pos: usize) -> (i32, (usize, bool)) {
        let (rb, rc, rd, rf) = (self.at(line - 1, pos + 1), self.at(line - 1, pos), self.at(line - 1, pos + 2), self.at(line - 2, pos + 1));
        let (dc, df, dd) = ((rc - rb).abs(), (rf - rb).abs(), (rd - rb).abs());
        let sum = if dc > df && dc > dd {
            rf + rd + 2 * rb
        } else if dd > dc && dd > df {
            rf + rc + 2 * rb
        } else {
            rd + rc + 2 * rb
        };
        (sum, Params::context(rb - rf, rc - rb))
    }

    fn even(&mut self, line: usize, pos: usize, set: usize) -> Result<()> {
        if !self.g.is_real(line, pos) {
            let (sum, _) = self.even_sum(line, pos);
            self.set(line, pos + 1, sum >> 2);
            return Ok(());
        }
        let (sum, (ci, negate)) = self.even_sum(line, pos);
        let truth = self.truth(line, pos);
        let p = self.p;
        let ctx = self.even.get_mut(set).and_then(|s| s.get_mut(ci)).ok_or_else(|| RawError::malformed("Fujifilm compressed RAF: context out of range"))?;
        let v = self.codec.sample(ctx, &p, sum >> 2, negate, truth)?;
        self.set(line, pos + 1, v);
        Ok(())
    }

    fn odd(&mut self, line: usize, pos: usize, set: usize) -> Result<()> {
        let (ra, rb, rc, rd, rg) = (self.at(line, pos), self.at(line - 1, pos + 1), self.at(line - 1, pos), self.at(line - 1, pos + 2), self.at(line, pos + 2));
        let pred = if (rb > rc && rb > rd) || (rb < rc && rb < rd) { (rg + ra + 2 * rb) >> 2 } else { (ra + rg) >> 1 };
        let (ci, negate) = Params::context(rb - rc, rc - ra);
        let truth = self.truth(line, pos);
        let p = self.p;
        let ctx = self.odd.get_mut(set).and_then(|s| s.get_mut(ci)).ok_or_else(|| RawError::malformed("Fujifilm compressed RAF: context out of range"))?;
        let v = self.codec.sample(ctx, &p, pred, negate, truth)?;
        self.set(line, pos + 1, v);
        Ok(())
    }

    /// Codes vectors `a` and `b` interleaved.
    fn pass(&mut self, a: usize, b: usize, set: usize) -> Result<()> {
        let lw = self.lw;
        // Borders: the previous vector's edge samples.
        for line in [a, b] {
            let (l, r) = (self.at(line - 1, 1), self.at(line - 1, lw));
            self.set(line, 0, l);
            self.set(line, lw + 1, r);
        }
        let (mut e, mut o) = (0usize, 1usize);
        while e < lw || o < lw {
            if e < lw {
                self.even(a, e, set)?;
                self.even(b, e, set)?;
                e += 2;
            }
            if (e > ODD_START || e >= lw) && o < lw {
                self.odd(a, o, set)?;
                self.odd(b, o, set)?;
                o += 2;
            }
        }
        Ok(())
    }

    /// Codes one six-row line.
    fn line(&mut self) -> Result<()> {
        for (a, b, set) in PASSES {
            self.pass(a, b, set)?;
        }
        Ok(())
    }

    /// Keeps the last two vectors of each colour for the next line.
    fn rotate(&mut self) {
        let w = self.lw + 2;
        for (to, from) in [(R0, R3), (R0 + 1, R4), (G0, G6), (G0 + 1, G7), (B0, B3), (B0 + 1, B4)] {
            self.lines.copy_within(from * w..(from + 1) * w, to * w);
        }
    }

    /// Sensor sample at (`row`, `col`) of the current line.
    #[inline]
    fn sample_at(&self, row: usize, col: usize) -> u16 {
        let cols = self.g.cell_cols();
        let (line, slot) = self.g.map.get(row).and_then(|m| m.get(col % cols)).copied().unwrap_or((G2, 0));
        let pos = col / cols * self.g.slots_per_cell() + slot;
        self.at(line, pos + 1).clamp(0, i32::from(u16::MAX)) as u16
    }
}

/// Splits the stream into its blocks.
fn blocks<'a>(strip: &'a [u8], h: &Header) -> Result<Vec<&'a [u8]>> {
    let mut at = h.data_start;
    h.sizes
        .iter()
        .map(|&size| {
            let end = at.checked_add(size).ok_or_else(|| RawError::malformed("Fujifilm compressed RAF: block sizes overflow"))?;
            let b = strip.get(at..end).ok_or_else(|| RawError::malformed("Fujifilm compressed RAF: block lies outside the strip"))?;
            at = end;
            Ok(b)
        })
        .collect()
}

/// Whether `strip` is a compressed stream for a `width × height` image: the
/// signature plus a header that agrees with the IFD (uncompressed samples
/// cannot spell both by accident).
pub(crate) fn is_compressed(strip: &[u8], width: usize, height: usize) -> bool {
    strip.starts_with(SIGNATURE) && header(strip).is_ok_and(|h| h.width == width && h.height == height)
}

/// Decodes a compressed strip into `width × height` samples.
pub(crate) fn decode(strip: &[u8], width: usize, height: usize, bits: u32, cfa: &Cfa, limits: &Limits) -> Result<Vec<u16>> {
    let h = header(strip)?;
    if h.version != 1 {
        return Err(RawError::unsupported(format!("Fujifilm compressed RAF stream version {} (lossy compression)", h.version)));
    }
    let p = Params::new(h.bits)?;
    let g = Geometry::new(cfa, h.raw_type)?;
    let lw = g.line_width(h.block_size)?;
    let blocks_needed = width.div_ceil(h.block_size.max(1));
    if h.width != width || h.height != height || h.bits != bits {
        return Err(RawError::malformed(format!(
            "Fujifilm compressed RAF: stream says {}x{} at {} bits, the IFD {width}x{height} at {bits}",
            h.width, h.height, h.bits
        )));
    }
    if h.total_lines * ROWS_PER_LINE != height || h.rounded_width != h.block_size.saturating_mul(h.sizes.len()) || h.sizes.len() < blocks_needed {
        return Err(RawError::malformed("Fujifilm compressed RAF: inconsistent stream geometry"));
    }
    limits.check(h.rounded_width as u64, height as u64, 2)?;
    let blocks = blocks(strip, &h)?;
    let decoded: Vec<Result<Vec<u16>>> = par::map(blocks_needed, |i| {
        let x0 = i * h.block_size;
        let cols = h.block_size.min(width - x0);
        let data = blocks.get(i).copied().unwrap_or(&[]);
        let mut b = Block::new(Decoder(BitReader { data, pos: 0 }), p, &g, lw, None);
        let mut out = vec![0u16; cols * height];
        for line in 0..h.total_lines {
            b.row0 = line * ROWS_PER_LINE;
            b.line()?;
            for r in 0..ROWS_PER_LINE {
                let y = b.row0 + r;
                for (c, o) in out.iter_mut().skip(y * cols).take(cols).enumerate() {
                    *o = b.sample_at(r, c);
                }
            }
            b.rotate();
        }
        if b.codec.0.overran() {
            return Err(RawError::malformed("Fujifilm compressed RAF: block is truncated"));
        }
        Ok(out)
    });
    let mut image = vec![0u16; width * height];
    for (i, block) in decoded.into_iter().enumerate() {
        let block = block?;
        let x0 = i * h.block_size;
        let cols = h.block_size.min(width - x0);
        for (y, src) in block.chunks_exact(cols.max(1)).enumerate() {
            if let Some(dst) = image.get_mut(y * width + x0..y * width + x0 + cols) {
                dst.copy_from_slice(src);
            }
        }
    }
    Ok(image)
}

/// Encodes `width × height` samples as a compressed strip with
/// `block_size`-column blocks (tests and synthetic files).
#[cfg(any(test, feature = "testgen"))]
pub(crate) fn encode(image: &[u16], width: usize, height: usize, bits: u32, cfa: &Cfa, block_size: usize) -> Result<Vec<u8>> {
    let p = Params::new(bits)?;
    let raw_type = if cfa.width == 6 { 16 } else { 0 };
    let g = Geometry::new(cfa, raw_type)?;
    let lw = g.line_width(block_size)?;
    if !height.is_multiple_of(ROWS_PER_LINE) || image.len() != width * height {
        return Err(RawError::malformed("Fujifilm compressed RAF encoder: height must be a multiple of 6"));
    }
    let nblocks = width.div_ceil(block_size);
    let total_lines = height / ROWS_PER_LINE;
    let mut blocks = Vec::with_capacity(nblocks);
    for i in 0..nblocks {
        let x0 = i * block_size;
        let mut b = Block::new(Encoder(BitWriter::default()), p, &g, lw, Some((image, width, x0, width)));
        for line in 0..total_lines {
            b.row0 = line * ROWS_PER_LINE;
            b.line()?;
            b.rotate();
        }
        blocks.push(b.codec.0.finish());
    }
    let mut out = SIGNATURE.to_vec();
    out.push(1);
    out.push(raw_type);
    out.push(bits as u8);
    out.extend_from_slice(&(height as u16).to_be_bytes());
    out.extend_from_slice(&((nblocks * block_size) as u16).to_be_bytes());
    out.extend_from_slice(&(width as u16).to_be_bytes());
    out.extend_from_slice(&(block_size as u16).to_be_bytes());
    out.push(nblocks as u8);
    out.extend_from_slice(&(total_lines as u16).to_be_bytes());
    for b in &blocks {
        out.extend_from_slice(&(b.len() as u32).to_be_bytes());
    }
    out.resize(out.len().next_multiple_of(16), 0);
    for b in &blocks {
        out.extend_from_slice(b);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The X-T30's X-Trans repeat at the data origin.
    const XTRANS: [u8; 36] = [1, 1, 0, 1, 1, 2, 1, 1, 2, 1, 1, 0, 2, 0, 1, 0, 2, 1, 1, 1, 2, 1, 1, 0, 1, 1, 0, 1, 1, 2, 0, 2, 1, 2, 0, 1];

    fn xtrans() -> Cfa {
        Cfa { width: 6, height: 6, colors: XTRANS.to_vec(), origin_x: 0, origin_y: 0 }
    }

    fn pattern(w: usize, h: usize, bits: u32, seed: u32) -> Vec<u16> {
        let max = (1u32 << bits) - 1;
        let mut s = seed;
        (0..w * h)
            .map(|i| {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let (x, y) = ((i % w) as u32, (i / w) as u32);
                let smooth = (x * 97 + y * 61) % max;
                // Mostly smooth with sparse spikes (which exercise the escape code).
                if s >> 29 == 0 { s % (max + 1) } else { smooth }
            })
            .map(|v| v as u16)
            .collect()
    }

    #[test]
    fn xtrans_round_trips() {
        let (w, h) = (30, 18);
        let cfa = xtrans();
        let data = pattern(w, h, 14, 7);
        let strip = encode(&data, w, h, 14, &cfa, 12).unwrap();
        let hdr = header(&strip).unwrap();
        assert_eq!((hdr.raw_type, hdr.block_size, hdr.sizes.len(), hdr.total_lines, hdr.rounded_width), (16, 12, 3, 3, 36));
        assert_eq!(decode(&strip, w, h, 14, &cfa, &Limits::default()).unwrap(), data);
    }

    #[test]
    fn bayer_round_trips_at_14_and_16_bits() {
        let cfa = Cfa::bayer("RGGB").unwrap();
        for bits in [14, 16] {
            let (w, h) = (22, 12);
            let data = pattern(w, h, bits, 3 + bits);
            let strip = encode(&data, w, h, bits, &cfa, 8).unwrap();
            assert_eq!(header(&strip).unwrap().raw_type, 0);
            assert_eq!(decode(&strip, w, h, bits, &cfa, &Limits::default()).unwrap(), data, "{bits} bits");
        }
    }

    #[test]
    fn extremes_round_trip() {
        // All zeros, all white and a checkerboard of both: the largest residuals and wraps.
        let cfa = xtrans();
        let (w, h) = (24, 12);
        for data in [vec![0u16; w * h], vec![16383u16; w * h], (0..w * h).map(|i| if (i + i / w) % 2 == 0 { 0 } else { 16383 }).collect()] {
            let strip = encode(&data, w, h, 14, &cfa, 12).unwrap();
            assert_eq!(decode(&strip, w, h, 14, &cfa, &Limits::default()).unwrap(), data);
        }
    }

    #[test]
    fn damage_fails_cleanly() {
        let (w, h) = (30, 18);
        let cfa = xtrans();
        let data = pattern(w, h, 14, 11);
        let strip = encode(&data, w, h, 14, &cfa, 12).unwrap();
        // Every truncation and a few corruptions decode to Err or a same-sized image, never a panic.
        for n in 0..strip.len() {
            if let Ok(img) = decode(&strip[..n], w, h, 14, &cfa, &Limits::default()) {
                assert_eq!(img.len(), w * h);
            }
        }
        for at in (0..strip.len()).step_by(7) {
            let mut c = strip.clone();
            c[at] ^= 0xA5;
            if let Ok(img) = decode(&c, w, h, 14, &cfa, &Limits::default()) {
                assert_eq!(img.len(), w * h);
            }
        }
        // Wrong geometry, depth and version are reported.
        assert!(matches!(decode(&strip, w + 6, h, 14, &cfa, &Limits::default()), Err(RawError::Malformed(_))));
        assert!(matches!(decode(&strip, w, h, 12, &cfa, &Limits::default()), Err(RawError::Malformed(_))));
        let mut lossy = strip.clone();
        lossy[2] = 0;
        assert!(matches!(decode(&lossy, w, h, 14, &cfa, &Limits::default()), Err(RawError::Unsupported(_))));
        let mut twelve = strip.clone();
        twelve[4] = 12;
        assert!(matches!(decode(&twelve, w, h, 12, &cfa, &Limits::default()), Err(RawError::Unsupported(_))));
        assert!(matches!(decode(&strip, w, h, 14, &cfa, &Limits { max_pixels: 100, ..Limits::default() }), Err(RawError::LimitExceeded(_))));
    }

    #[test]
    fn context_statistics() {
        let mut c = Context { sum: 256, count: 1 };
        assert_eq!(c.suffix_bits(13), 8);
        c.count = 2;
        assert_eq!(c.suffix_bits(13), 7);
        c.count = 4;
        assert_eq!(c.suffix_bits(13), 6);
        let big = Context { sum: 1 << 20, count: 1 };
        assert_eq!(big.suffix_bits(13), 13);
        let mut c = Context { sum: 1000, count: CONTEXT_MAX_COUNT };
        c.update(24);
        assert_eq!((c.sum, c.count), (512, 33));
        assert_eq!(Params::quant(17), 1);
        assert_eq!(Params::quant(18), 2);
        assert_eq!(Params::quant(-67), -3);
        assert_eq!(Params::quant(276), 4);
        assert_eq!(Params::context(-300, 0), (36, true));
    }

    #[test]
    fn bit_reader_counts_zeros_and_reads_past_the_end() {
        let data = [0b0000_0001u8, 0b1010_0000, 0, 0, 0, 0, 0, 0, 0, 0];
        let mut r = BitReader { data: &data, pos: 0 };
        assert_eq!(r.zeros(41), 7);
        assert_eq!(r.zeros(41), 0);
        assert_eq!(r.read(2), 0b01);
        // 68 zeros follow: capped at 41, one more bit consumed.
        assert_eq!(r.zeros(41), 41);
        assert_eq!(r.pos, 8 + 3 + 42);
        assert!(!r.overran());
        r.read(32);
        assert!(r.overran());
    }
}
