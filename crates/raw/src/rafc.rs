//! Fujifilm's striped predictive compression ("compressed RAF", the strip that
//! starts with `IS`).
//!
//! Fujifilm publishes no specification. The broad structure follows Fabian's
//! prose description of the lossless scheme
//! (capnfabs.net/posts/fuji-raf-compression-algorithm/): independent stripes of
//! 768 sensor columns, six-row groups collated into colour vectors, gradient
//! predictors with signed contexts and adaptive Rice coding. The headers,
//! colour-vector placement, predictors, contexts and adaptation constants come
//! from LightCraft's MIT-licensed decoder (storytold/lightcraft#282, the same
//! maintainers, recovered there from sample bytes and black-box comparisons of
//! decoded sensor arrays), adapted to this crate's error, limits and
//! parallelism conventions. No code from dcraw, LibRaw, rawspeed, rawler or
//! darktable was used.

use crate::error::{RawError, Result};
use crate::par;

const BLOCK: usize = 768;
const XTRANS: [[usize; 6]; 6] = [[1, 1, 0, 1, 1, 2], [1, 1, 2, 1, 1, 0], [2, 0, 1, 0, 2, 1], [1, 1, 2, 1, 1, 0], [1, 1, 0, 1, 1, 2], [0, 2, 1, 2, 0, 1]];
/// Which of the four slots of each 4-column period hold real samples (bit `p`
/// of the mask) and which are interpolation slots that consume no bits.
pub(crate) const MASKS: [[u8; 2]; 6] = [[10, 15], [15, 10], [14, 10], [15, 11], [11, 15], [10, 14]];

fn be16(src: &[u8], at: usize) -> Result<usize> {
    let s = src.get(at..at.checked_add(2).ok_or_else(|| malformed("truncated header"))?).ok_or_else(|| malformed("truncated header"))?;
    Ok(u16::from_be_bytes([s[0], s[1]]) as usize)
}

fn malformed(why: &str) -> RawError {
    RawError::malformed(format!("Fujifilm compressed RAF: {why}"))
}

/// The stream's own flag (0 lossy, 1 lossless) must agree with the RAF
/// container's declared compression (byte 108: 3 lossy, 2 lossless), when the
/// container declares a known one.
pub(crate) fn decode(src: &[u8], width: usize, height: usize, bits: u32, xtrans: bool, lossy_hint: Option<bool>) -> Result<Vec<u16>> {
    let h = src.get(..16).ok_or_else(|| malformed("truncated header"))?;
    if !h.starts_with(b"IS") {
        return Err(malformed("missing compression signature"));
    }
    let flag = h[2];
    if flag > 1 {
        return Err(malformed("invalid compression flag"));
    }
    if let Some(lossy) = lossy_hint
        && lossy != (flag == 0)
    {
        return Err(malformed("the container's declared compression mode disagrees with the stream"));
    }
    let sensor = h[3];
    if sensor != if xtrans { 16 } else { 0 } {
        return Err(malformed("compression sensor type disagrees with the CFA layout"));
    }
    if !matches!(bits, 12 | 14 | 16) {
        return Err(RawError::unsupported(format!("Fujifilm compressed RAF at {bits} bits")));
    }
    let count = h[13] as usize;
    if h[4] != bits as u8
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
        return Err(malformed("inconsistent compression dimensions"));
    }
    let table_end = 16 + count * 4;
    let mut offset = table_end.div_ceil(16) * 16;
    let quantizer_start = offset;
    let quantizer_stride = (height / 6).div_ceil(16) * 16;
    if flag == 0 {
        offset += quantizer_stride * count;
        if src.get(quantizer_start..offset).is_none() {
            return Err(malformed("truncated quantizer tables"));
        }
    }
    let mut stripes = Vec::with_capacity(count);
    for i in 0..count {
        let at = 16 + i * 4;
        let entry = src.get(at..at + 4).ok_or_else(|| malformed("truncated stripe size table"))?;
        let size = u32::from_be_bytes([entry[0], entry[1], entry[2], entry[3]]) as usize;
        let end = offset.checked_add(size).ok_or_else(|| malformed("stripe size overflow"))?;
        let bytes = src.get(offset..end).filter(|s| !s.is_empty()).ok_or_else(|| malformed("stripe lies outside the file"))?;
        let quantizers = if flag == 0 {
            Some(
                src.get(quantizer_start + i * quantizer_stride..quantizer_start + i * quantizer_stride + height / 6)
                    .ok_or_else(|| malformed("truncated quantizer table"))?,
            )
        } else {
            None
        };
        stripes.push((bytes, quantizers, (width - i * BLOCK).min(BLOCK)));
        offset = end;
    }
    let decoded: Vec<Result<Vec<u16>>> = par::map(stripes.len(), |i| decode_stripe(stripes[i].0, stripes[i].1, stripes[i].2, height, bits, xtrans));
    let mut output = vec![0u16; width.checked_mul(height).ok_or_else(|| malformed("image size overflows"))?];
    for (i, data) in decoded.into_iter().enumerate() {
        let data = data?;
        let stripe_width = stripes[i].2;
        for (row, samples) in output.chunks_exact_mut(width).zip(data.chunks_exact(stripe_width)) {
            row.get_mut(i * BLOCK..i * BLOCK + stripe_width).ok_or_else(|| malformed("invalid stripe placement"))?.copy_from_slice(samples);
        }
    }
    Ok(output)
}

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
            let b = self.src.get(self.pos).copied().ok_or_else(|| malformed("truncated entropy stream"))?;
            self.pos += 1;
            self.acc = (self.acc << 8) | u64::from(b);
            self.available += 8;
        }
        self.available -= n;
        Ok(((self.acc >> self.available) & ((1u64 << n) - 1)) as u32)
    }

    /// One signed residual: a unary prefix (zero bits), then `k` least
    /// significant bits — or, at the escape prefix, `bits` whole bits plus one.
    /// Even codes are positive (0 → 0), odd codes negative (1 → −1).
    fn residual(&mut self, k: u32, bits: u32, range: u32, limit: u32) -> Result<i32> {
        let mut zeros = 0;
        while self.get(1)? == 0 {
            zeros += 1;
            if zeros > limit {
                return Err(malformed("invalid residual escape"));
            }
        }
        let code = if zeros == limit { self.get(bits)? + 1 } else { (zeros << k) | self.get(k)? };
        if code > range {
            return Err(malformed("residual outside the sample range"));
        }
        Ok(if code & 1 == 0 { (code / 2) as i32 } else { -((code / 2) as i32) - 1 })
    }
}

#[derive(Clone, Copy)]
struct State {
    sum: u32,
    count: u32,
}

impl State {
    /// The Rice parameter that keeps the mean absolute residual under one
    /// unary step.
    fn k(self) -> u32 {
        let mut k = 0;
        while self.count << k < self.sum {
            k += 1;
        }
        k
    }

    fn update(&mut self, delta: i32) {
        self.sum += delta.unsigned_abs();
        if self.count == 64 {
            self.sum >>= 1;
            self.count >>= 1;
        }
        self.count += 1;
    }
}

/// The gradient bucket of a prediction difference, for the context number.
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

/// The JPEG-LS-style edge predictor for the even (leading) column of a pair.
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

#[inline]
fn sample(rows: &[Vec<i32>], y: usize, x: usize) -> Result<i32> {
    rows.get(y).and_then(|r| r.get(x)).copied().ok_or_else(|| malformed("invalid predictor position"))
}

fn decode_stripe(bytes: &[u8], quantizers: Option<&[u8]>, width: usize, height: usize, bits: u32, xtrans: bool) -> Result<Vec<u16>> {
    // One colour vector covers a 768-column stripe: half its positions for
    // Bayer (one sample in two), two thirds for X-Trans (interpolation slots
    // fill the rest).
    let size = if xtrans { BLOCK * 2 / 3 } else { BLOCK / 2 };
    let mut previous = vec![vec![vec![0; size + 2]; 2]; 3];
    let max = (1u32 << bits) - 1;
    let range = |near: u32| (max + 2 * near) / (2 * near + 1) + 1;
    let seed = |near: u32| State { sum: ((range(near) + 32) / 64).max(2), count: 1 };
    let mut states = [[[seed(0); 41]; 2]; 3];
    // Smooth regions retain finer precision and independent state across block quantizer changes.
    let mut fine = [[[[seed(0); 5]; 2]; 3], [[[seed(1); 5]; 2]; 3], [[[seed(2); 5]; 2]; 3]];
    let mut last_near = None;
    let mut reader = Bits::new(bytes);
    let mut output = Vec::with_capacity(width.checked_mul(height).ok_or_else(|| malformed("image size overflows"))?);
    for group in 0..height / 6 {
        let near = match quantizers {
            Some(q) => u32::from(*q.get(group).ok_or_else(|| malformed("truncated quantizer table"))?),
            None => 0,
        };
        if last_near != Some(near) {
            states = [[[seed(near); 41]; 2]; 3];
            last_near = Some(near);
        }
        let mut rows = previous;
        for pair in 0..6 {
            // Three pairs of colour vectors: red+green, green+blue, green+red.
            let colours = if pair % 2 == 0 { [0, 1] } else { [1, 2] };
            for &colour in &colours {
                let lines = rows.get_mut(colour).ok_or_else(|| malformed("invalid colour vector"))?;
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
            // Even columns lead their odd partners by seven positions.
            for even in (0..size + 8).step_by(2) {
                for (odd, position) in [(0, Some(even)), (1, even.checked_sub(7))] {
                    let Some(pos) = position.filter(|&x| x < size) else { continue };
                    let x = pos + 1;
                    for (which, &colour) in colours.iter().enumerate() {
                        let lines = rows.get_mut(colour).ok_or_else(|| malformed("invalid colour vector"))?;
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
                                    .ok_or_else(|| malformed("invalid fine coding context"))?;
                                (context, state)
                            } else {
                                let context = 9 * quant(d1, near) + quant(d2, near);
                                let state = states
                                    .get_mut(pair % 3)
                                    .and_then(|s| s.get_mut(odd))
                                    .and_then(|s| s.get_mut(context.unsigned_abs() as usize))
                                    .ok_or_else(|| malformed("invalid coding context"))?;
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
                        *lines.get_mut(y).and_then(|r| r.get_mut(x)).ok_or_else(|| malformed("invalid predictor position"))? = value;
                    }
                }
            }
        }
        // Collate the colour vectors back into sensor order: Bayer keeps one
        // vector per colour and half-rows, X-Trans scatters its 512 slots over
        // the 6×6 pattern through the column masks.
        for y in 0..6 {
            for x in 0..width {
                let colour = if xtrans {
                    XTRANS.get(y).and_then(|r| r.get(x % 6)).copied().ok_or_else(|| malformed("invalid CFA position"))?
                } else {
                    match (y % 2, x % 2) {
                        (0, 0) => 0,
                        (1, 1) => 2,
                        _ => 1,
                    }
                };
                let row = 2 + if colour == 1 { y } else { y / 2 };
                let col = 1 + if xtrans { 2 * (x / 3) + (x % 3).min(1) } else { x / 2 };
                output.push(sample(rows.get(colour).ok_or_else(|| malformed("invalid colour vector"))?, row, col)? as u16);
            }
        }
        previous = rows.into_iter().map(|mut r| r.split_off(r.len() - 2)).collect();
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn residual_sign_wrap_and_escape() {
        // Ordinary codes 0, 1, 2, 3 with k=2; then the escape for -8192.
        let mut stream = vec![true, false, false, true, false, true, true, true, false, true, true, true];
        stream.extend(std::iter::repeat_n(false, 41));
        stream.push(true);
        let escaped = 16382u32;
        stream.extend((0..14).rev().map(|i| (escaped >> i) & 1 != 0));
        let bytes: Vec<u8> = stream.chunks(8).map(|chunk| chunk.iter().enumerate().fold(0u8, |b, (i, bit)| b | (u8::from(*bit) << (7 - i)))).collect();
        let mut reader = Bits::new(&bytes);
        for delta in [0, -1, 1, -2, -8192] {
            assert_eq!(reader.residual(2, 14, 16384, 41).unwrap(), delta);
        }
        assert_eq!(even_prediction(1112, 858, 1104, 1152), 983);
    }

    #[test]
    fn state_k_tracks_the_mean() {
        let mut st = State { sum: 256, count: 1 };
        assert_eq!(st.k(), 8);
        st.update(0);
        assert_eq!(st.k(), 7);
        for _ in 0..62 {
            st.update(4);
        }
        assert_eq!(st.count, 64);
        assert_eq!(st.sum, 256 + 62 * 4);
        // The 65th entry halves the window and keeps counting.
        st.update(0);
        assert_eq!(st.count, 33);
        assert_eq!(st.sum, (256 + 62 * 4) / 2);
    }
}
