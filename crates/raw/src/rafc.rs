//! Fujifilm's striped predictive raw compression (lossless and lossy RAF).
//!
//! Ported from LightCraft's clean-room decoder (`crates/raw/src/vendor/rafc.rs`,
//! storytold/lightcraft, MIT OR Apache-2.0, same organisation). LightCraft took the broad
//! structure from Fabian's prose description of lossless RAF compression
//! (<https://capnfabs.net/posts/fuji-raf-compression-algorithm/>) and recovered the headers,
//! colour-vector placement, predictors, contexts and adaptation from sample bytes and
//! black-box comparisons of decoded sensor arrays (rawpy as an oracle only); no other raw
//! decoder's source was consulted. LightCraft's `docs/raf-compression.md` records the
//! verification: 24 files from 13 bodies (X-Trans II–V, GFX Bayer, 14- and 16-bit, lossless
//! and lossy) matched the reference sensor arrays sample for sample.
//!
//! The stream:
//!
//! * A 16-byte `IS` header: compression flag (0 lossy, 1 lossless), sensor type (0 Bayer, 16
//!   X-Trans), bit depth, height, padded width, width, 768-pixel stripe width, stripe count and
//!   six-row group count. It must agree with the raw IFD.
//! * A big-endian stripe size table, padded to 16 bytes; lossy files then carry one quantizer
//!   byte per six-row group for each stripe, each table padded to 16 bytes.
//! * Independent stripes 768 pixels wide. Each six-row group is collated into three red, six
//!   green and three blue vectors (384 samples on Bayer; 512 slots on X-Trans, some of them
//!   interpolated without consuming bits). Vector pairs alternate R/G and G/B; even columns
//!   lead odd ones by seven positions. Prediction errors use adaptive Rice coding with a
//!   bounded escape, in 41 folded gradient contexts per pair phase and column parity.

use crate::error::{RawError, Result};
use crate::par;

const BLOCK: usize = 768;
/// The CFA the X-Trans stream assumes at the data origin (0 = R, 1 = G, 2 = B).
pub(crate) const XTRANS: [[usize; 6]; 6] =
    [[1, 1, 0, 1, 1, 2], [1, 1, 2, 1, 1, 0], [2, 0, 1, 0, 2, 1], [1, 1, 2, 1, 1, 0], [1, 1, 0, 1, 1, 2], [0, 2, 1, 2, 0, 1]];
/// Which X-Trans vector slots hold real samples, per pair and colour, by column mod 4.
const MASKS: [[u8; 2]; 6] = [[10, 15], [15, 10], [14, 10], [15, 11], [11, 15], [10, 14]];

/// A malformed-stream error.
fn corrupt(why: &str) -> RawError {
    RawError::malformed(format!("Fujifilm compressed RAF: {why}"))
}

/// Big-endian 16-bit value at `at`.
fn be16(src: &[u8], at: usize) -> Result<usize> {
    let s = src.get(at..at.saturating_add(2)).ok_or_else(|| corrupt("truncated header"))?;
    let [a, b] = s else { return Err(corrupt("truncated header")) };
    Ok(u16::from_be_bytes([*a, *b]) as usize)
}

/// The compression flag (byte 2 of the stream): 1 lossless, 0 lossy.
pub(crate) fn stream_flag(src: &[u8]) -> Option<u8> {
    src.starts_with(b"IS").then(|| src.get(2).copied()).flatten()
}

struct Stripe<'a> {
    bytes: &'a [u8],
    quantizers: Option<&'a [u8]>,
    width: usize,
}

/// Decodes a `width × height` compressed strip. The caller has checked the size against the
/// [`crate::Limits`].
pub(crate) fn decode(src: &[u8], width: usize, height: usize, bits: u32, xtrans: bool) -> Result<Vec<u16>> {
    let h = src.get(..16).ok_or_else(|| corrupt("truncated header"))?;
    if !h.starts_with(b"IS") {
        return Err(corrupt("missing compression signature"));
    }
    let flag = h.get(2).copied().ok_or_else(|| corrupt("truncated header"))?;
    if flag > 1 {
        return Err(corrupt("invalid compression flag"));
    }
    let sensor = h.get(3).copied().ok_or_else(|| corrupt("truncated header"))?;
    if sensor != if xtrans { 16 } else { 0 } {
        return Err(corrupt("compression sensor type disagrees with the CFA"));
    }
    if !matches!(bits, 12 | 14 | 16) {
        return Err(RawError::unsupported(format!("Fujifilm compressed RAF at {bits} bits")));
    }
    let count = h.get(13).copied().ok_or_else(|| corrupt("truncated header"))? as usize;
    if h.get(4).copied() != Some(bits as u8)
        || be16(h, 5)? != height
        || be16(h, 9)? != width
        || count == 0
        || count != width.div_ceil(BLOCK)
        || be16(h, 7)? != count * BLOCK
        || be16(h, 11)? != BLOCK
        || height == 0
        || !height.is_multiple_of(6)
        || be16(h, 14)? != height / 6
    {
        return Err(corrupt("inconsistent compression dimensions"));
    }
    let n = width.checked_mul(height).ok_or_else(|| RawError::LimitExceeded("image too large".into()))?;
    let table_end = 16 + count * 4;
    let mut offset = table_end.div_ceil(16) * 16;
    let quantizer_start = offset;
    let quantizer_stride = (height / 6).div_ceil(16) * 16;
    if flag == 0 {
        offset += quantizer_stride * count;
        if src.get(quantizer_start..offset).is_none() {
            return Err(corrupt("truncated quantizer tables"));
        }
    }
    let mut stripes = Vec::with_capacity(count);
    for i in 0..count {
        let entry = src.get(16 + i * 4..20 + i * 4).ok_or_else(|| corrupt("truncated stripe size table"))?;
        let [a, b, c, d] = entry else { return Err(corrupt("truncated stripe size table")) };
        let size = u32::from_be_bytes([*a, *b, *c, *d]) as usize;
        let end = offset.checked_add(size).ok_or_else(|| corrupt("stripe size overflow"))?;
        let bytes = src.get(offset..end).filter(|s| !s.is_empty()).ok_or_else(|| corrupt("stripe outside the file"))?;
        let quantizers = if flag == 0 {
            let q = quantizer_start + i * quantizer_stride;
            Some(src.get(q..q + height / 6).ok_or_else(|| corrupt("truncated quantizer table"))?)
        } else {
            None
        };
        stripes.push(Stripe { bytes, quantizers, width: (width - i * BLOCK).min(BLOCK) });
        offset = end;
    }
    let decoded = par::map(stripes.len(), |i| match stripes.get(i) {
        Some(s) => decode_stripe(s, height, bits, xtrans),
        None => Err(corrupt("invalid stripe")),
    });
    let mut output = vec![0; n];
    for (i, (stripe, data)) in stripes.iter().zip(decoded).enumerate() {
        let data = data?;
        for (row, samples) in output.chunks_exact_mut(width).zip(data.chunks_exact(stripe.width)) {
            row.get_mut(i * BLOCK..i * BLOCK + stripe.width).ok_or_else(|| corrupt("invalid stripe placement"))?.copy_from_slice(samples);
        }
    }
    Ok(output)
}

/// Big-endian bit reader.
struct Bits<'a> {
    src: &'a [u8],
    pos: usize,
    acc: u64,
    available: u32,
}

impl<'a> Bits<'a> {
    fn new(src: &'a [u8]) -> Self {
        Self { src, pos: 0, acc: 0, available: 0 }
    }

    #[inline]
    fn get(&mut self, n: u32) -> Result<u32> {
        while self.available < n {
            let b = self.src.get(self.pos).copied().ok_or_else(|| corrupt("truncated entropy stream"))?;
            self.pos += 1;
            self.acc = (self.acc << 8) | u64::from(b);
            self.available += 8;
        }
        self.available -= n;
        Ok(((self.acc >> self.available) & ((1u64 << n) - 1)) as u32)
    }

    /// One Rice-coded residual: `zeros` unary prefix then `k` bits, or after `limit` zeros
    /// an escaped `bits`-bit code.
    fn residual(&mut self, k: u32, bits: u32, range: u32, limit: u32) -> Result<i32> {
        let mut zeros = 0;
        while self.get(1)? == 0 {
            zeros += 1;
            if zeros > limit {
                return Err(corrupt("invalid residual escape"));
            }
        }
        let code = if zeros == limit { self.get(bits)? + 1 } else { (zeros << k) | self.get(k)? };
        if code > range {
            return Err(corrupt("residual outside the sample range"));
        }
        Ok(if code & 1 == 0 { (code / 2) as i32 } else { -((code / 2) as i32) - 1 })
    }
}

/// Adaptive Rice parameter statistics of one context.
#[derive(Clone, Copy)]
struct State {
    sum: u32,
    count: u32,
}

impl State {
    fn k(self) -> u32 {
        let mut k = 0;
        while k < 31 && self.count << k < self.sum {
            k += 1;
        }
        k
    }

    fn update(&mut self, delta: i32) {
        self.sum = self.sum.saturating_add(delta.unsigned_abs());
        if self.count == 64 {
            self.sum >>= 1;
            self.count >>= 1;
        }
        self.count += 1;
    }
}

/// Gradient bucket (−4..=4) for the context of quantizer `near`.
#[inline]
fn quant(v: i32, near: u32) -> i32 {
    let t = [18 + 3 * near, 67 + 5 * near, 276 + 7 * near];
    let a = v.unsigned_abs();
    let bucket = if a <= near {
        0
    } else if a < t[0] {
        1
    } else if a < t[1] {
        2
    } else if a < t[2] {
        3
    } else {
        4
    };
    if v < 0 { -bucket } else { bucket }
}

/// Prediction for an even column from its north-west, north, north-east and north-north
/// neighbours: the pair across the smallest gradient, averaged with the north sample.
#[inline]
fn even_prediction(nw: i32, n: i32, ne: i32, nn: i32) -> i32 {
    let (a, b, c) = ((nw - n).abs(), (ne - n).abs(), (nn - n).abs());
    let pair = if a > b && a > c {
        ne + nn
    } else if b > a && b > c {
        nw + nn
    } else {
        nw + ne
    };
    (pair + 2 * n) >> 2
}

/// Sample `x` of line `y` of a colour vector.
#[inline]
fn sample(rows: &[Vec<i32>], y: usize, x: usize) -> Result<i32> {
    rows.get(y).and_then(|r| r.get(x)).copied().ok_or_else(|| corrupt("invalid predictor position"))
}

/// Decodes one stripe, six rows at a time, into `height × stripe.width` samples.
fn decode_stripe(stripe: &Stripe<'_>, height: usize, bits: u32, xtrans: bool) -> Result<Vec<u16>> {
    let size = if xtrans { BLOCK * 2 / 3 } else { BLOCK / 2 };
    let mut previous = vec![vec![vec![0; size + 2]; 2]; 3];
    let max = (1u32 << bits) - 1;
    let range = |near: u32| (max + 2 * near) / (2 * near + 1) + 1;
    let seed = |near: u32| State { sum: ((range(near) + 32) / 64).max(2), count: 1 };
    let mut states = [[[seed(0); 41]; 2]; 3];
    // Smooth regions keep finer precision and their own state across block quantizer changes.
    let mut fine = [[[[seed(0); 5]; 2]; 3], [[[seed(1); 5]; 2]; 3], [[[seed(2); 5]; 2]; 3]];
    let mut last_near = None;
    let mut reader = Bits::new(stripe.bytes);
    let mut output = Vec::with_capacity(height.saturating_mul(stripe.width));
    for group in 0..height / 6 {
        let near = match stripe.quantizers {
            Some(q) => u32::from(*q.get(group).ok_or_else(|| corrupt("truncated quantizer table"))?),
            None => 0,
        };
        if last_near != Some(near) {
            states = [[[seed(near); 41]; 2]; 3];
            last_near = Some(near);
        }
        let mut rows = previous;
        for pair in 0..6 {
            let colours = if pair % 2 == 0 { [0, 1] } else { [1, 2] };
            for &colour in &colours {
                let lines = rows.get_mut(colour).ok_or_else(|| corrupt("invalid colour vector"))?;
                let mut row = vec![0; size + 2];
                let prev = lines.len() - 1;
                let left = sample(lines, prev, 1)?;
                let right = sample(lines, prev, size)?;
                if let Some(v) = row.first_mut() {
                    *v = left;
                }
                if let Some(v) = row.last_mut() {
                    *v = right;
                }
                lines.push(row);
            }
            for even in (0..size + 8).step_by(2) {
                for (odd, position) in [(0, Some(even)), (1, even.checked_sub(7))] {
                    let Some(pos) = position.filter(|&x| x < size) else { continue };
                    let x = pos + 1;
                    for (which, &colour) in colours.iter().enumerate() {
                        let lines = rows.get_mut(colour).ok_or_else(|| corrupt("invalid colour vector"))?;
                        let y = lines.len() - 1;
                        let (nw, north, ne, nn) =
                            (sample(lines, y - 1, x - 1)?, sample(lines, y - 1, x)?, sample(lines, y - 1, x + 1)?, sample(lines, y - 2, x)?);
                        let west = sample(lines, y, x - 1)?;
                        let east = sample(lines, y, x + 1)?;
                        let (predict, d1, d2) = if odd == 0 {
                            (even_prediction(nw, north, ne, nn), north - nn, nw - north)
                        } else {
                            let p = if (nw.min(ne)..=nw.max(ne)).contains(&north) { (west + east) >> 1 } else { (west + east + 2 * north) >> 2 };
                            (p, north - nw, nw - west)
                        };
                        let real = !xtrans || MASKS.get(pair).and_then(|m| m.get(which)).is_some_and(|m| (m >> (pos % 4)) & 1 != 0);
                        let value = if real {
                            let contrast = d1.unsigned_abs() + d2.unsigned_abs();
                            let precision = match contrast {
                                0..=5 => 0,
                                6 => near.min(1),
                                7 => near.min(2),
                                _ => near,
                            };
                            let (context, state) = if precision < near {
                                let sign = |v: i32| if v.unsigned_abs() <= precision { 0 } else { v.signum() };
                                let context = 3 * sign(d1) + sign(d2);
                                let state = fine
                                    .get_mut(precision as usize)
                                    .and_then(|f| f.get_mut(pair % 3))
                                    .and_then(|s| s.get_mut(odd))
                                    .and_then(|s| s.get_mut(context.unsigned_abs() as usize))
                                    .ok_or_else(|| corrupt("invalid fine coding context"))?;
                                (context, state)
                            } else {
                                let context = 9 * quant(d1, near) + quant(d2, near);
                                let state = states
                                    .get_mut(pair % 3)
                                    .and_then(|s| s.get_mut(odd))
                                    .and_then(|s| s.get_mut(context.unsigned_abs() as usize))
                                    .ok_or_else(|| corrupt("invalid coding context"))?;
                                (context, state)
                            };
                            let coded_range = range(precision);
                            let coded_bits = 32 - (coded_range - 1).leading_zeros();
                            let mut delta = reader.residual(state.k(), coded_bits, coded_range, 4 * bits - coded_bits - 1)?;
                            state.update(delta);
                            if context < 0 {
                                delta = -delta;
                            }
                            let step = (2 * precision + 1) as i32;
                            let mut value = predict + delta * step;
                            if value < -(precision as i32) {
                                value += coded_range as i32 * step;
                            } else if value > (max + precision) as i32 {
                                value -= coded_range as i32 * step;
                            }
                            value.clamp(0, max as i32)
                        } else {
                            predict
                        };
                        *lines.get_mut(y).and_then(|r| r.get_mut(x)).ok_or_else(|| corrupt("invalid predictor position"))? = value;
                    }
                }
            }
        }
        for y in 0..6 {
            for x in 0..stripe.width {
                let colour = if xtrans {
                    XTRANS.get(y).and_then(|r| r.get(x % 6)).copied().ok_or_else(|| corrupt("invalid CFA position"))?
                } else {
                    match (y % 2, x % 2) {
                        (0, 0) => 0,
                        (1, 1) => 2,
                        _ => 1,
                    }
                };
                let row = 2 + if colour == 1 { y } else { y / 2 };
                let col = 1 + if xtrans { 2 * (x / 3) + (x % 3).min(1) } else { x / 2 };
                output.push(sample(rows.get(colour).ok_or_else(|| corrupt("invalid colour vector"))?, row, col)? as u16);
            }
        }
        previous = rows.into_iter().map(|mut r| r.split_off(r.len() - 2)).collect();
    }
    Ok(output)
}

/// A compressed stream whose residuals are all zero (every sample decodes to 0), for tests
/// and fuzz seeds: two stripes over a 780-pixel-wide image, `6 × groups` rows high. Lossy
/// streams take one quantizer per group.
#[cfg(any(test, feature = "testgen"))]
pub(crate) fn zero_stream(bits: u32, xtrans: bool, groups: usize, quantizers: Option<&[u8]>) -> Vec<u8> {
    assert!(quantizers.is_none_or(|q| q.len() == groups), "one quantizer per six-row group");
    let mut bitstream = Vec::new();
    let mut main = [[(1u32 << (bits - 6), 1u32); 2]; 3];
    let mut fine = main;
    let mut previous = None;
    for group in 0..groups {
        let q = quantizers.and_then(|q| q.get(group)).copied().unwrap_or(0);
        if previous != Some(q) {
            main = [[(1u32 << (bits - 6), 1u32); 2]; 3];
            previous = Some(q);
        }
        for pair in 0..6 {
            let width = if xtrans { 512 } else { 384 };
            for even in (0usize..width + 8).step_by(2) {
                for (parity, column) in [(0, Some(even)), (1, even.checked_sub(7))] {
                    let Some(column) = column.filter(|&x| x < width) else { continue };
                    for &mask in &MASKS[pair % 6] {
                        if xtrans && (mask >> (column % 4)) & 1 == 0 {
                            continue;
                        }
                        let (sum, count) = &mut if q == 0 { &mut main } else { &mut fine }[pair % 3][parity];
                        let k = 32 - (sum.saturating_sub(1) / *count).leading_zeros();
                        bitstream.push(true);
                        bitstream.extend(std::iter::repeat_n(false, k as usize));
                        if *count == 64 {
                            *sum /= 2;
                            *count /= 2;
                        }
                        *count += 1;
                    }
                }
            }
        }
    }
    let stream: Vec<u8> = bitstream.chunks(8).map(|chunk| chunk.iter().enumerate().fold(0, |b, (i, bit)| b | (u8::from(*bit) << (7 - i)))).collect();
    // Two stripes; the second is mostly padding, which must never enter the output.
    let mut out = b"IS".to_vec();
    out.extend_from_slice(&[u8::from(quantizers.is_none()), if xtrans { 16 } else { 0 }, bits as u8]);
    for v in [groups as u16 * 6, 1536, 780, 768] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    out.push(2);
    out.extend_from_slice(&(groups as u16).to_be_bytes());
    for _ in 0..2 {
        out.extend_from_slice(&(stream.len() as u32).to_be_bytes());
    }
    out.resize(32, 0);
    if let Some(q) = quantizers {
        for _ in 0..2 {
            out.extend_from_slice(q);
            out.resize(out.len() + q.len().div_ceil(16) * 16 - q.len(), 0);
        }
    }
    out.extend_from_slice(&stream);
    out.extend_from_slice(&stream);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lossless_and_lossy_bayer_and_xtrans_stripes() {
        for bits in [12, 14, 16] {
            for xtrans in [false, true] {
                for q in [None, Some([2, 0, 5].as_slice())] {
                    let src = zero_stream(bits, xtrans, 3, q);
                    assert_eq!(decode(&src, 780, 18, bits, xtrans).unwrap(), vec![0; 780 * 18], "{bits} {xtrans} {q:?}");
                }
            }
        }
    }

    #[test]
    fn malformed_headers_and_truncated_entropy_are_errors() {
        let valid = zero_stream(14, true, 3, Some(&[1, 2, 0]));
        for len in (0..96).chain([valid.len() / 2, valid.len() - 1]) {
            assert!(decode(&valid[..len], 780, 18, 14, true).is_err(), "length {len}");
        }
        for at in [2, 3, 4, 5, 7, 9, 11, 13, 14, 16] {
            let mut src = valid.clone();
            src[at] ^= 0xff;
            assert!(decode(&src, 780, 18, 14, true).is_err(), "header byte {at}");
        }
        assert!(decode(&valid, usize::MAX, usize::MAX, 14, true).is_err());
        assert!(decode(&valid, 780, 18, 14, false).is_err(), "sensor type disagrees with the CFA");
        let mut reader = Bits::new(&[0; 64]);
        assert!(reader.residual(0, 14, 16384, 41).is_err());
    }

    #[test]
    fn residual_sign_wrap_and_escape() {
        // Ordinary codes 0, 1, 2, 3 with k=2; then the escape for -8192.
        let mut stream = vec![true, false, false, true, false, true, true, true, false, true, true, true];
        stream.extend(std::iter::repeat_n(false, 41));
        stream.push(true);
        let escaped = 16382u32;
        stream.extend((0..14).rev().map(|i| (escaped >> i) & 1 != 0));
        let bytes: Vec<u8> = stream.chunks(8).map(|chunk| chunk.iter().enumerate().fold(0, |b, (i, bit)| b | (u8::from(*bit) << (7 - i)))).collect();
        let mut reader = Bits::new(&bytes);
        for delta in [0, -1, 1, -2, -8192] {
            assert_eq!(reader.residual(2, 14, 16384, 41).unwrap(), delta);
        }
        assert_eq!(even_prediction(1112, 858, 1104, 1152), 983);
    }

    #[test]
    fn random_streams_never_panic() {
        // xorshift: deterministic hostile entropy behind a valid header.
        let mut x = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for case in 0..256 {
            let bits = [12, 14, 16][case % 3];
            let xtrans = case % 2 == 0;
            let quantizer = (next() & 0xFF) as u8;
            let len = (next() % 2048) as usize;
            let data: Vec<u8> = (0..len).map(|_| next() as u8).collect();
            let mut src = zero_stream(bits, xtrans, 1, Some(&[quantizer]));
            src.truncate(64);
            for i in [16, 20] {
                src[i..i + 4].copy_from_slice(&(data.len() as u32).to_be_bytes());
            }
            src.extend_from_slice(&data);
            src.extend_from_slice(&data);
            let _ = decode(&src, 780, 6, bits, xtrans);
        }
    }
}
