//! A smart object under an affine transform (rotation, skew, scale with either), drawn as the
//! reference application draws one. Measured byte for byte at 8 bits against 44 of its renders
//! (rotations from 5° to 80°, skews, rotations with scale from 70 % to 120 %, skews with shrinks
//! from 30 % to 75 %), and every one matches:
//!
//! 1. **Bleed** as for scaling ([`crate::smart_scale`]).
//! 2. **Quarter turns first:** past 45° the source is turned by whole quarter turns (an exact
//!    pixel shuffle), so the passes see a rotation within ±45°.
//! 3. **Two passes:** pass 1 resamples each source row onto the document columns
//!    (`x' = a x + c y + e`), pass 2 each document column onto the document rows. Plain Bicubic
//!    (Keys −0.75, unstretched), 14-bit weights, colour straight with the edges repeated, alpha
//!    transparent beyond; each pass rounds to the sample levels.
//! 4. **Positions in fixed point:** each line's shift and scaled length are rounded to 1/256 of a
//!    document pixel. A pass that keeps its scale shifts each line by whole 1/128 phases (the
//!    floor). Otherwise a line walks its pixels from the first one of the clip box: the start
//!    rounded up and the step truncated, both to 2⁻³² of a source sample, the phase the floor to
//!    1/128.
//! 5. **Strong shrinks:** a pass shrinking below 0.7 first averages its lines down to
//!    `round(n·scale/0.7)` samples, each the plain mean of the samples whose centres it covers.
//! 6. **Clip:** columns between the corners' rounded extremes; rows between the rounded column
//!    tops at the first and last column and the bottoms below them.
//!
//! Mirror flips (a negative determinant) are not handled here.

use photocraft_geom::Rect;
use photocraft_raster::Surface;

use crate::smart_scale::{Depth, Image, MAX_SAMPLES, ONE, bleed, keys, load, store};

/// Below this scale a pass first averages its lines down (rule 5).
const SHRINK_LIMIT: f64 = 0.7;
/// Fractional bits of a phase (1/128 sample) in the line walk: 32 bits of a sample in all.
const WALK_BITS: u32 = 25;

/// 14-bit Bicubic weights of the six taps `floor(u) − 2 ..= floor(u) + 3` for each of the 128
/// phases of `u`.
pub(crate) fn weight_table() -> Vec<[i64; 6]> {
    (0..128)
        .map(|ph| {
            let u = f64::from(ph) / 128.0;
            let w: Vec<f64> = (0..6).map(|t| keys(u + 2.0 - f64::from(t), -0.75)).collect();
            let total: f64 = w.iter().sum();
            let mut q = [0i64; 6];
            for (qq, ww) in q.iter_mut().zip(&w) {
                *qq = (ww / total * ONE as f64).trunc() as i64;
            }
            let mut big = 0;
            for (i, v) in w.iter().enumerate() {
                if w.get(big).is_some_and(|b| v > b) {
                    big = i;
                }
            }
            let others: i64 = q.iter().enumerate().filter(|&(i, _)| i != big).map(|(_, v)| v).sum();
            if let Some(b) = q.get_mut(big) {
                *b = ONE - others;
            }
            q
        })
        .collect()
}

/// Rounds to whole 1/256 px, halves up.
pub(crate) fn r256(v: f64) -> i64 {
    (v * 256.0 + 0.5).floor() as i64
}

pub(crate) fn round_half_up(v: f64) -> i64 {
    (v + 0.5).floor() as i64
}

/// The walk of rule 4 along lines of one length: any pixel's phase, without walking to it.
#[derive(Clone, Copy)]
struct Stride {
    /// The line keeps its scale, so a phase is the pixel's distance itself.
    even: bool,
    /// Phases per pixel, with [`WALK_BITS`] more fractional bits.
    step: i128,
    /// Phases per 1/256 px, as `num / den`, with the same bits.
    num: i128,
    den: i128,
}

impl Stride {
    /// Lines whose `n` samples span `length` (1/256 px); `None` for no length.
    fn new(length: i64, n: usize) -> Option<Self> {
        if length <= 0 {
            return None;
        }
        let num = n as i128 * 128 * 256;
        let len = i128::from(length);
        Some(Stride { even: length == n as i64 * 256, step: (num << WALK_BITS) / len, num: num << WALK_BITS, den: len * 256 })
    }

    /// Where the walk starts on a line whose first pixel lies `dist0` (1/256 px) from its origin.
    fn start(self, dist0: i64) -> i128 {
        if self.even {
            return i128::from(dist0);
        }
        -(-(i128::from(dist0) * self.num)).div_euclid(self.den)
    }

    /// The phase index (1/128 sample) of pixel `k` of the line that starts at `start`.
    fn phase(self, start: i128, k: usize) -> i64 {
        if self.even {
            return ((start + 256 * k as i128) * 128).div_euclid(256) as i64;
        }
        (start + k as i128 * self.step).div_euclid(1 << WALK_BITS) as i64
    }
}

/// Rows of `img` (its lines) averaged down to `n1` samples each (rule 5).
fn reduce(img: &Image, n1: usize, depth: Depth) -> Image {
    let (w, h, n) = (img.w, img.h, img.n);
    let node: Vec<usize> = (0..w).map(|r| (((r + 1) * n1).saturating_sub(1) / w).min(n1.saturating_sub(1))).collect();
    let mut px = vec![0.0f32; n1 * h * n];
    crate::par::chunks_mut(&mut px, n1 * n, true, |y, row| {
        let mut sums = vec![0.0f64; n1 * n];
        let mut counts = vec![0usize; n1];
        for (r, &j) in node.iter().enumerate() {
            if let Some(c) = counts.get_mut(j) {
                *c += 1;
            }
            for ch in 0..n {
                if let Some(s) = sums.get_mut(j * n + ch) {
                    *s += f64::from(img.at(r, y, ch));
                }
            }
        }
        for (j, out) in row.chunks_exact_mut(n).enumerate() {
            let c = counts.get(j).copied().unwrap_or(0);
            for (ch, v) in out.iter_mut().enumerate() {
                *v = depth.mean(sums.get(j * n + ch).copied().unwrap_or(0.0), c);
            }
        }
    });
    Image { w: n1, h, n, px }
}

/// One pass along the rows of `img`: row `y` produces `count` pixels, the first lying `dist0[y]`
/// (1/256 px) from that row's origin, the row's `img.w` samples spanning `length` (1/256 px).
fn pass(img: &Image, dist0: &[i64], count: usize, length: i64, depth: Depth, table: &[[i64; 6]]) -> Option<Image> {
    if let Some(n1) = reduced(img.w, length) {
        return walk(&reduce(img, n1, depth), dist0, count, &vec![length; img.h], depth, table);
    }
    walk(img, dist0, count, &vec![length; img.h], depth, table)
}

/// The samples a line of `n` is first averaged down to when it spans `length` (1/256 px), if it
/// shrinks that far (rule 5).
fn reduced(n: usize, length: i64) -> Option<usize> {
    let scale = length as f64 / (n as f64 * 256.0);
    (scale < SHRINK_LIMIT).then(|| round_half_up(n as f64 * scale / SHRINK_LIMIT).max(1) as usize)
}

/// [`reduce`] for the pass down the columns: the columns of `img` averaged down to `n1` samples each.
fn reduce_columns(img: &Image, n1: usize, depth: Depth) -> Image {
    let (w, h, n) = (img.w, img.h, img.n);
    // A row's node never falls as the rows go down, so each node averages one run of rows.
    let mut runs = vec![(0usize, 0usize); n1];
    for r in 0..h {
        let j = (((r + 1) * n1).saturating_sub(1) / h.max(1)).min(n1.saturating_sub(1));
        if let Some(run) = runs.get_mut(j) {
            if run.1 == 0 {
                run.0 = r;
            }
            run.1 = r + 1;
        }
    }
    let mut px = vec![0.0f32; n1 * w * n];
    crate::par::chunks_mut(&mut px, w * n, true, |j, row| {
        let Some(&(a, b)) = runs.get(j) else { return };
        let mut sums = vec![0.0f64; w * n];
        for r in a..b {
            let Some(src) = img.px.get(r * w * n..(r + 1) * w * n) else { continue };
            sums.iter_mut().zip(src).for_each(|(s, v)| *s += f64::from(*v));
        }
        for (v, s) in row.iter_mut().zip(&sums) {
            *v = depth.mean(*s, b - a);
        }
    });
    Image { w, h: n1, n, px }
}

/// One pixel of the walk: the six weights `q` over samples `base..base + 6` of a line whose
/// samples `0..=last` `sample` finds. Colour repeats the edge outwards; alpha reads transparency
/// there. `acc` is scratch, a slot per channel.
#[inline]
fn six<'a>(sample: impl Fn(usize) -> Option<&'a [f32]>, base: i64, last: i64, q: &[i64; 6], depth: Depth, acc: &mut [f64], out: &mut [f32]) {
    let alpha = out.len().saturating_sub(1);
    acc.fill(0.0);
    if depth.levels.is_some() && base >= 0 && base + 5 <= last {
        // Whole levels, every sample on the line: a weight of zero adds nothing, so no tap needs
        // a test.
        for (t, &qq) in q.iter().enumerate() {
            let Some(px) = sample(base as usize + t) else { continue };
            let qq = qq as f64;
            acc.iter_mut().zip(px).for_each(|(s, v)| *s += f64::from(*v) * qq);
        }
    } else {
        for (t, &qq) in q.iter().enumerate() {
            if qq == 0 {
                continue;
            }
            let i = base + t as i64;
            let Some(px) = sample(i.clamp(0, last.max(0)) as usize) else { continue };
            let channels = if (0..=last).contains(&i) { out.len() } else { alpha };
            let qq = qq as f64;
            acc.iter_mut().zip(px).take(channels).for_each(|(s, v)| *s += f64::from(*v) * qq);
        }
    }
    for (ch, (v, s)) in out.iter_mut().zip(acc.iter()).enumerate() {
        *v = depth.settle(*s, ch == alpha);
    }
}

/// Calls `$f::<N>($nc, ..)` with `N` the pixel width `$nc` when it is a usual one (grey, RGB or
/// Lab, CMYK, each with alpha), and 0 for any other. `$f` takes its width from [`width`], so the
/// usual widths are known when it is compiled and the loops over a pixel's samples unroll.
macro_rules! by_width {
    ($nc:expr, $f:ident($($arg:expr),*)) => {
        match $nc {
            2 => $f::<2>($nc, $($arg),*),
            4 => $f::<4>($nc, $($arg),*),
            5 => $f::<5>($nc, $($arg),*),
            _ => $f::<0>($nc, $($arg),*),
        }
    };
}

/// The pixel width inside a function [`by_width`] calls.
#[inline]
const fn width<const N: usize>(nc: usize) -> usize {
    if N == 0 { nc } else { N }
}

/// One line of [`walk`]: `row`'s pixels from the samples of `line`, `nc` to a pixel.
fn walk_line<const N: usize>(nc: usize, line: &[f32], stride: Stride, start: i128, depth: Depth, table: &[[i64; 6]], row: &mut [f32]) {
    let nc = width::<N>(nc).max(1);
    let last = (line.len() / nc) as i64 - 1;
    let (mut few, mut many) = ([0.0f64; 8], Vec::new());
    let acc = scratch(nc, &mut few, &mut many);
    for (k, out) in row.chunks_exact_mut(nc).enumerate() {
        let ph = stride.phase(start, k);
        let base = ph.div_euclid(128) - 2;
        let Some(q) = table.get(ph.rem_euclid(128) as usize) else { continue };
        six(|i| line.get(i * nc..(i + 1) * nc), base, last, q, depth, acc, out);
    }
}

/// A slot per channel for [`six`]: on the stack for any real pixel.
fn scratch<'a>(nc: usize, few: &'a mut [f64; 8], many: &'a mut Vec<f64>) -> &'a mut [f64] {
    if nc <= few.len() {
        return few.get_mut(..nc).unwrap_or_default();
    }
    many.resize(nc, 0.0);
    many
}

/// The walk of rule 4 down the columns of an image, computed a row of the result at a time (so
/// the rows can go straight into a surface's tiles).
struct Columns<'a> {
    img: &'a Image,
    /// Each column's [`Stride::start`].
    starts: Vec<i128>,
    stride: Stride,
    depth: Depth,
    table: &'a [[i64; 6]],
    /// Fully transparent pixels are cleared afterwards, so those off a column's end stay zero.
    clear: bool,
}

impl Columns<'_> {
    /// The pixels of row `y` from column `x` on, into `out` (zeros to start with).
    fn row(&self, y: usize, x: usize, out: &mut [f32]) {
        by_width!(self.img.n, columns_row(self, y, x, out));
    }
}

/// [`Columns::row`] for pixels of `nc` samples.
fn columns_row<const N: usize>(nc: usize, columns: &Columns, y: usize, x: usize, out: &mut [f32]) {
    columns.row_of(width::<N>(nc).max(1), y, x, out);
}

impl Columns<'_> {
    #[inline]
    fn row_of(&self, nc: usize, y: usize, x: usize, out: &mut [f32]) {
        let w = self.img.w;
        let last = self.img.h as i64 - 1;
        let (mut few, mut many) = ([0.0f64; 8], Vec::new());
        let acc = scratch(nc, &mut few, &mut many);
        for (i, px) in out.chunks_exact_mut(nc).enumerate() {
            let col = x + i;
            let Some(&start) = self.starts.get(col) else { continue };
            let ph = self.stride.phase(start, y);
            let base = ph.div_euclid(128) - 2;
            if self.clear && (base > last || base + 5 < 0) {
                continue;
            }
            let Some(q) = self.table.get(ph.rem_euclid(128) as usize) else { continue };
            six(|r| self.img.px.get((r * w + col) * nc..(r * w + col + 1) * nc), base, last, q, self.depth, acc, px);
        }
    }
}

/// The walk of rule 4 along the rows of `img`, no averaging: row `y` produces `count` pixels, the
/// first lying `dist0[y]` (1/256 px) from that row's origin, its `img.w` samples spanning
/// `lengths[y]` (1/256 px).
pub(crate) fn walk(img: &Image, dist0: &[i64], count: usize, lengths: &[i64], depth: Depth, table: &[[i64; 6]]) -> Option<Image> {
    let (n, nc) = (img.w, img.n);
    let mut px = vec![0.0f32; count.checked_mul(img.h)?.checked_mul(nc)?];
    crate::par::chunks_mut(&mut px, count * nc, true, |y, row| {
        let (Some(&d0), Some(&length)) = (dist0.get(y), lengths.get(y)) else { return };
        let Some(line) = img.px.get(y * n * nc..(y + 1) * n * nc) else { return };
        let Some(stride) = Stride::new(length, n) else { return };
        by_width!(nc, walk_line(line, stride, stride.start(d0), depth, table, row));
    });
    Some(Image { w: count, h: img.h, n: nc, px })
}

pub(crate) fn transpose(img: &Image) -> Image {
    let (w, h, n) = (img.w, img.h, img.n);
    let mut px = vec![0.0f32; w * h * n];
    // Output row x is input column x.
    crate::par::chunks_mut(&mut px, h * n, true, |x, row| {
        for (y, out) in row.chunks_exact_mut(n).enumerate() {
            if let Some(src) = img.px.get((y * w + x) * n..(y * w + x + 1) * n) {
                out.copy_from_slice(src);
            }
        }
    });
    Image { w: h, h: w, n, px }
}

/// A quarter turn clockwise (row `i` of the result is column `i` read bottom to top).
fn turn(img: &Image) -> Image {
    let (w, h, n) = (img.w, img.h, img.n);
    let mut px = vec![0.0f32; w * h * n];
    crate::par::chunks_mut(&mut px, h * n, true, |i, row| {
        for (j, out) in row.chunks_exact_mut(n).enumerate() {
            let y = h - 1 - j;
            if let Some(src) = img.px.get((y * w + i) * n..(y * w + i + 1) * n) {
                out.copy_from_slice(src);
            }
        }
    });
    Image { w: h, h: w, n, px }
}

/// The pixels of `src` inside `bounds` (its contents' canvas; `src` has alpha) drawn through the
/// affine `m = [a, b, c, d, e, f]` (contents → document: `x' = a x + c y + e`,
/// `y' = b x + d y + f`, with `x, y` measured from `bounds`' corner). `None` for a mirror flip, a
/// degenerate map, or a size past [`MAX_SAMPLES`].
pub fn render(src: &Surface, bounds: Rect, m: [f64; 6]) -> Option<Surface> {
    render_with(src, bounds, m, true, true)
}

/// [`render`], bleeding the contents or (for a perspective cascade's intermediate, which keeps
/// its computed colours under transparency) not.
/// `clear`: a final result, whose fully transparent pixels lose their colour ([`crate::smart_scale::clear_hidden`]);
/// not a cascade's intermediate, whose colours its next stage reads.
pub(crate) fn render_with(src: &Surface, bounds: Rect, m: [f64; 6], bleed_contents: bool, clear: bool) -> Option<Surface> {
    let fmt = src.format();
    let nc = src.channels();
    if !fmt.alpha || nc == 0 || bounds.is_empty() || m.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let [a0, b0, c0, d0, e0, f0] = m;
    if a0 * d0 - b0 * c0 <= 1e-12 {
        return None;
    }
    // The source is loaded whole: past the size limit, before anything is allocated.
    let loaded = (bounds.width() as usize).checked_mul(bounds.height() as usize)?.checked_mul(nc)?;
    if loaded > MAX_SAMPLES {
        return None;
    }
    let depth = Depth::of(fmt.sample);
    let mut img = load(src, bounds, depth);
    if bleed_contents {
        bleed(&mut img);
    }
    // Rule 2: quarter turns, and the map from the turned image.
    let ang = b0.atan2(a0).to_degrees();
    let turns = ((ang + 45.0) / 90.0).floor().rem_euclid(4.0) as usize;
    let (mut l, mut t) = ([[a0, c0], [b0, d0]], [e0, f0]);
    for _ in 0..turns {
        img = turn(&img);
        // p' = R p + (H_old, 0) with R = [[0, -1], [1, 0]]: the map from the turned image is
        // L R⁻¹ (p' − (H_old, 0)), R⁻¹ = [[0, 1], [-1, 0]].
        let ri = [[0.0, 1.0], [-1.0, 0.0]];
        let lr = [
            [l[0][0] * ri[0][0] + l[0][1] * ri[1][0], l[0][0] * ri[0][1] + l[0][1] * ri[1][1]],
            [l[1][0] * ri[0][0] + l[1][1] * ri[1][0], l[1][0] * ri[0][1] + l[1][1] * ri[1][1]],
        ];
        let hold = img.w as f64; // the old height is the turned width
        t = [t[0] - lr[0][0] * hold, t[1] - lr[1][0] * hold];
        l = lr;
    }
    let (a, c, b, d, e, f) = (l[0][0], l[0][1], l[1][0], l[1][1], t[0], t[1]);
    if a <= 0.0 {
        return None;
    }
    let k = d - b * c / a;
    if k <= 0.0 {
        return None;
    }
    let (w, h) = (img.w, img.h);
    let (wf, hf) = (w as f64, h as f64);
    let xs = [e, a * wf + e, c * hf + e, a * wf + c * hf + e];
    let x0 = round_half_up(xs.iter().copied().fold(f64::INFINITY, f64::min));
    let x1 = round_half_up(xs.iter().copied().fold(f64::NEG_INFINITY, f64::max));
    if x1 <= x0 {
        return None;
    }
    let nx = (x1 - x0) as usize;
    let top = |x: i64| f + b * (x as f64 + 0.5 - e) / a;
    let (t0, t1) = (top(x0), top(x1 - 1));
    let y0 = round_half_up(t0.min(t1));
    let y1 = round_half_up(t0.max(t1) + hf * k);
    if y1 <= y0 {
        return None;
    }
    // The result's box in document pixels: every position below (in 1/256 px) is reckoned from
    // it, so a placement far off the canvas returns `None` here rather than overflow there.
    let rect = Rect::new(i32::try_from(x0).ok()?, i32::try_from(y0).ok()?, i32::try_from(x1).ok()?, i32::try_from(y1).ok()?);
    let ny = (y1 - y0) as usize;
    let biggest = nx.max(w).checked_mul(ny.max(h))?.checked_mul(nc)?;
    if biggest > MAX_SAMPLES {
        return None;
    }
    let table = weight_table();
    // Pass 1: each source row onto the document columns x0..x1.
    let dist1: Vec<i64> = (0..h).map(|j| x0 * 256 - r256(e + c * (j as f64 + 0.5))).collect();
    let inter = pass(&img, &dist1, nx, r256(wf * a), depth, &table)?;
    // Pass 2: each document column onto the rows y0..y1, a row at a time, straight into the
    // result's tiles.
    let length = r256(hf * k);
    let inter = match reduced(inter.h, length) {
        Some(n1) => reduce_columns(&inter, n1, depth),
        None => inter,
    };
    let Some(stride) = Stride::new(length, inter.h) else {
        return store(fmt, rect, depth, clear, |_, _, _| {});
    };
    let starts = (0..nx).map(|i| stride.start(y0 * 256 - r256(top(x0 + i as i64)))).collect();
    let columns = Columns { img: &inter, starts, stride, depth, table: &table, clear };
    store(fmt, rect, depth, clear, |y, x, out| columns.row(y, x, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The phase indices (1/128 sample) of `count` consecutive pixels of a line whose first pixel
    /// lies `dist0` (1/256 px) from the line's origin; `length` (1/256 px) covers `n` samples.
    fn line_phases(dist0: i64, count: usize, length: i64, n: usize) -> Vec<i64> {
        let Some(stride) = Stride::new(length, n) else { return Vec::new() };
        let start = stride.start(dist0);
        (0..count).map(|k| stride.phase(start, k)).collect()
    }

    /// The same phases as a walk along the line reaches them (rule 4 as first written).
    fn walked_phases(dist0: i64, count: usize, length: i64, n: usize) -> Vec<i64> {
        if length <= 0 {
            return Vec::new();
        }
        if length == n as i64 * 256 {
            return (0..count as i64).map(|k| ((dist0 + 256 * k) * 128).div_euclid(256)).collect();
        }
        let num = n as i128 * 128 * 256;
        let len = i128::from(length);
        let step = (num << WALK_BITS) / len;
        let exact = (i128::from(dist0) * num) << WALK_BITS;
        let den = len * 256;
        let start = -(-exact).div_euclid(den);
        (0..count as i128).map(|k| (start + k * step).div_euclid(1 << WALK_BITS) as i64).collect()
    }

    /// The walk along the rows as first written: every tap of every pixel tested.
    fn walk_slow(img: &Image, dist0: &[i64], count: usize, lengths: &[i64], depth: Depth, table: &[[i64; 6]]) -> Image {
        let (n, nc) = (img.w, img.n);
        let (alpha, last) = (nc - 1, n as i64 - 1);
        let mut px = vec![0.0f32; count * img.h * nc];
        for (y, row) in px.chunks_exact_mut(count * nc).enumerate() {
            let line = &img.px[y * n * nc..(y + 1) * n * nc];
            for (out, ph) in row.chunks_exact_mut(nc).zip(walked_phases(dist0[y], count, lengths[y], n)) {
                let base = ph.div_euclid(128) - 2;
                let mut acc = vec![0.0f64; nc];
                for (t, &qq) in table[ph.rem_euclid(128) as usize].iter().enumerate() {
                    let i = base + t as i64;
                    let at = i.clamp(0, last.max(0)) as usize * nc;
                    for ch in 0..nc {
                        if qq != 0 && (ch != alpha || (0..=last).contains(&i)) {
                            acc[ch] += f64::from(line[at + ch]) * qq as f64;
                        }
                    }
                }
                for (ch, v) in out.iter_mut().enumerate() {
                    *v = depth.settle(acc[ch], ch == alpha);
                }
            }
        }
        Image { w: count, h: img.h, n: nc, px }
    }

    /// An image of noise at `depth` (whole levels, or any value at 32 bits), every fifth pixel
    /// transparent.
    fn noisy(w: usize, h: usize, n: usize, depth: Depth, seed: u64) -> Image {
        let mut s = seed;
        let mut px = Vec::with_capacity(w * h * n);
        for i in 0..w * h * n {
            s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            let part = (s >> 40) as f32 / (1u64 << 24) as f32;
            let hidden = i % n == n - 1 && (i / n).is_multiple_of(5);
            px.push(if hidden { 0.0 } else { depth.levels.map_or(part * 2.0 - 0.5, |top| (part * top).round()) });
        }
        Image { w, h, n, px }
    }

    const DEPTHS: [Depth; 3] = [Depth { levels: Some(255.0) }, Depth { levels: Some(65535.0) }, Depth { levels: None }];

    fn same(a: &[f32], b: &[f32]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.to_bits() == b.to_bits())
    }

    /// Fully transparent pixels of `n` samples lose their colour, as a cleared render's do.
    fn cleared(px: &[f32], n: usize) -> Vec<f32> {
        px.chunks_exact(n).flat_map(|p| if p[n - 1] <= 0.0 { vec![0.0; n] } else { p.to_vec() }).collect()
    }

    #[test]
    fn any_pixels_phase_is_the_one_a_walk_reaches() {
        for (n, length) in [(128, 128 * 256), (128, 96 * 256), (128, 96 * 256 + 1), (7, 3000), (4000, 4000 * 384 + 7), (1, 1), (5, 5 * 256), (6000, 3)] {
            let stride = Stride::new(length, n).expect("a length");
            for dist0 in [0, 1, 64, 255, 256, -1, -64, -100_000, 1_234_567, i64::from(i32::MIN), i64::from(i32::MAX)] {
                let start = stride.start(dist0);
                let got: Vec<i64> = (0..300).map(|k| stride.phase(start, k)).collect();
                assert_eq!(got, walked_phases(dist0, 300, length, n), "{n} samples over {length}, from {dist0}");
            }
        }
        assert!(Stride::new(0, 8).is_none() && Stride::new(-5, 8).is_none());
    }

    /// Pixel widths known when compiled, taps untested inside the line, sums settled without
    /// whole numbers: the walk must still give what testing every tap gives, at every depth.
    #[test]
    fn a_walk_gives_what_testing_every_tap_gives() {
        let table = weight_table();
        // Lines that keep their scale, grow, shrink, start far off either end, or have no length.
        let lengths = [37 * 256, 37 * 768, 37 * 128 + 13, 37 * 256, 5000, 1, 0];
        let dist0 = [-700, 0, 64, 100_000, -3000, 5, 9];
        for (d, depth) in DEPTHS.into_iter().enumerate() {
            for nc in [2, 3, 4, 5, 9] {
                let img = noisy(37, lengths.len(), nc, depth, (d * 16 + nc) as u64);
                let got = walk(&img, &dist0, 90, &lengths, depth, &table).expect("walked");
                let want = walk_slow(&img, &dist0, 90, &lengths, depth, &table);
                assert!(same(&got.px, &want.px), "{nc} samples a pixel, depth {d}");
            }
        }
    }

    #[test]
    fn averaging_columns_is_averaging_the_rows_of_the_image_on_its_side() {
        for (d, depth) in DEPTHS.into_iter().enumerate() {
            let img = noisy(7, 50, 4, depth, 40 + d as u64);
            for n1 in [1, 2, 7, 35, 49, 50] {
                let want = transpose(&reduce(&transpose(&img), n1, depth));
                let got = reduce_columns(&img, n1, depth);
                assert_eq!((got.w, got.h, got.n), (7, n1, 4));
                assert!(same(&got.px, &want.px), "to {n1}, depth {d}");
            }
        }
    }

    /// The second pass goes down the columns a row of the result at a time; it must give the walk
    /// along the rows of the image on its side, and where the render is cleared afterwards, all
    /// that the clearing leaves.
    #[test]
    fn the_pass_down_the_columns_is_a_walk_of_the_image_on_its_side() {
        let table = weight_table();
        let (w, h, rows) = (9, 41, 70);
        // Columns that start before the image, inside it and wholly past it.
        let dist0: Vec<i64> = (0..w as i64).map(|i| i * 2600 - 6000).collect();
        for (d, depth) in DEPTHS.into_iter().enumerate() {
            for nc in [2, 3, 4, 5] {
                let img = noisy(w, h, nc, depth, (d * 16 + nc) as u64);
                for length in [41 * 256, 41 * 512 + 77, 41 * 230, 3] {
                    let stride = Stride::new(length, h).expect("a length");
                    let want = transpose(&walk(&transpose(&img), &dist0, rows, &vec![length; w], depth, &table).expect("walked"));
                    for clear in [false, true] {
                        let columns = Columns { img: &img, starts: dist0.iter().map(|&d| stride.start(d)).collect(), stride, depth, table: &table, clear };
                        let mut got = vec![0.0f32; w * rows * nc];
                        // A row in two spans, as tiles split it.
                        for (y, row) in got.chunks_exact_mut(w * nc).enumerate() {
                            let (left, right) = row.split_at_mut(4 * nc);
                            columns.row(y, 0, left);
                            columns.row(y, 4, right);
                        }
                        let (got, want) = if clear { (cleared(&got, nc), cleared(&want.px, nc)) } else { (got, want.px.clone()) };
                        assert!(same(&got, &want), "{nc} samples a pixel, depth {d}, length {length}, clear {clear}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_line_that_keeps_its_scale_moves_by_whole_phases() {
        // Shift by 0.25 px (64/256): phase 32, then 128 per pixel.
        assert_eq!(line_phases(64, 3, 128 * 256, 128), vec![32, 160, 288]);
    }

    #[test]
    fn a_scaling_line_lands_just_below_exact_boundaries_once_it_has_walked() {
        // 128 samples over 96 px: 4/3 sample per px, not a binary fraction. Starting exactly on a
        // phase, the first pixel keeps it (the start is rounded up); later exact ones fall below.
        let ph = line_phases(0, 4, 96 * 256, 128);
        assert_eq!(ph[0], 0);
        assert_eq!(ph[3], 512 - 1, "three steps of 4/3 = 4 samples exactly, truncated just below");
    }

    #[test]
    fn weights_sum_to_one() {
        for q in weight_table() {
            assert_eq!(q.iter().sum::<i64>(), ONE);
        }
    }

    #[test]
    fn strong_shrinks_average_the_covered_samples() {
        // 128 samples to 91 nodes (50 % / 0.7): ((r + 1) 91 − 1) div 128 groups 1 or 2 samples.
        let img = Image { w: 128, h: 1, n: 1, px: (0..128).map(|i| i as f32).collect() };
        let r = reduce(&img, 91, Depth { levels: Some(255.0) });
        assert_eq!(r.w, 91);
        // Sample 0 goes to node 0 ((1·91 − 1) div 128 = 0), sample 1 to node 1 (181 div 128).
        assert_eq!(r.at(0, 0, 0), 0.0);
        assert_eq!(r.at(1, 0, 0), 1.0);
        assert!(r.px.iter().all(|v| v.fract() == 0.0), "rounded to whole levels");
    }

    /// A rotated render leaves no colour under alpha 0: the bled colour of the transparent
    /// surround is cleared, so the layer's content ends where it is visible.
    #[test]
    fn a_rotation_leaves_no_colour_in_transparent_pixels() {
        use photocraft_color::{ColorMode, PixelFormat, SampleType};
        let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
        s.fill_rect(Rect::new(0, 0, 40, 30), &[1.0, 0.0, 0.0, 1.0]);
        let (sin, cos) = 30f64.to_radians().sin_cos();
        let out = render(&s, Rect::new(0, 0, 40, 30), [cos, sin, -sin, cos, 20.0, 5.0]).expect("rendered");
        let b = out.content_bounds();
        let px = out.read_region(b);
        let hidden = px.chunks(4).filter(|p| p[3] == 0.0 && p[..3].iter().any(|v| *v != 0.0)).count();
        assert_eq!(hidden, 0, "colour under alpha 0");
        assert!(px.chunks(4).any(|p| p[3] > 0.99), "the picture is drawn");
    }

    /// A source past the size limit is refused before it is loaded, and a placement far off the
    /// canvas is refused rather than overflowing the 1/256 px positions.
    #[test]
    fn huge_sources_and_far_placements_are_refused_without_a_panic() {
        use photocraft_color::{ColorMode, PixelFormat, SampleType};
        let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
        s.fill_rect(Rect::new(0, 0, 10, 10), &[1.0, 0.0, 0.0, 1.0]);
        assert!(render(&s, Rect::new(0, 0, 40_000, 40_000), [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]).is_none(), "too big to load");
        // Scaled up, so the box keeps its width that far out (at 1e17 a double steps by 16).
        for e in [1e17, -1e17, 4e9] {
            assert!(render(&s, Rect::new(0, 0, 10, 10), [100.0, 0.0, 0.0, 1.0, e, 0.0]).is_none(), "x offset {e}");
            assert!(render(&s, Rect::new(0, 0, 10, 10), [1.0, 0.0, 0.0, 100.0, 0.0, e]).is_none(), "y offset {e}");
        }
        assert!(render(&s, Rect::new(0, 0, 10, 10), [1.0, 0.0, 0.0, 1.0, 5.0, 5.0]).is_some(), "an ordinary placement still renders");
    }

    /// The probe source: 128 x 128, each row one grey from a 32-bit LCG (seed 777), opaque.
    fn bands() -> Surface {
        use photocraft_color::{ColorMode, PixelFormat, SampleType};
        let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
        let mut seed: u32 = 777;
        let mut data = Vec::with_capacity(128 * 128 * 4);
        for _ in 0..128 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let v = (seed >> 24) as f32 / 255.0;
            for _ in 0..128 {
                data.extend_from_slice(&[v, v, v, 1.0]);
            }
        }
        s.write_region(Rect::new(0, 0, 128, 128), &data);
        s
    }

    fn grey(s: &Surface, x: i32, y: i32) -> u8 {
        (s.pixel(x, y)[0] * 255.0).round() as u8
    }

    /// Byte for byte the reference application's render of the bands rotated 30° about the
    /// document centre (160, 160), source at (96, 96): document row 160, columns 112..136.
    #[test]
    fn a_rotated_smart_object_matches_the_reference_byte_for_byte() {
        let (sn, cs) = 30f64.to_radians().sin_cos();
        let (cx, cy) = (160.0, 160.0);
        let e = cx - (cs * cx - sn * cy) + (cs * 96.0 - sn * 96.0);
        let f = cy - (sn * cx + cs * cy) + (sn * 96.0 + cs * 96.0);
        let out = render(&bands(), Rect::new(0, 0, 128, 128), [cs, sn, -sn, cs, e, f]).unwrap();
        let want = [173, 252, 204, 54, 47, 188, 255, 255, 178, 73, 6, 14, 92, 206, 191, 51, 0, 50, 55, 8, 56, 168, 199, 131];
        let got: Vec<u8> = (112..136).map(|x| grey(&out, x, 160)).collect();
        assert_eq!(got, want);
    }

    /// The bands skewed 10° vertically and shrunk to 50 % (below the strong-shrink limit), with
    /// the reference application's stored corners: document column 160, rows 140..164.
    #[test]
    fn a_strongly_shrunk_skewed_smart_object_matches_the_reference_byte_for_byte() {
        let (top_l, top_r, bot_l) = (116.715073234658, 139.284926765342, 180.715073234658);
        let m = [1.0, (top_r - top_l) / 128.0, 0.0, (bot_l - top_l) / 128.0, 96.0, top_l];
        let out = render(&bands(), Rect::new(0, 0, 128, 128), m).unwrap();
        let want = [147, 147, 176, 200, 214, 164, 94, 112, 98, 99, 81, 129, 212, 141, 18, 206, 104, 79, 222, 232, 65, 229, 222, 18];
        let got: Vec<u8> = (140..164).map(|y| grey(&out, 160, y)).collect();
        assert_eq!(got, want);
    }
}
