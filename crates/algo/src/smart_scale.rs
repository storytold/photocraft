//! A smart object scaled without rotation, drawn as the reference application draws one. Measured
//! byte for byte at 8 bits against 76 of its renders (scales from 12.5 % to 200 %, each axis on
//! its own, content with and without transparency), and every one matches:
//!
//! 1. **Bleed.** Each fully transparent pixel first takes the colour of the nearest visible pixel
//!    (Manhattan distance). Of several equally near, it takes one below if there is any, else one
//!    above, else one level with it; of those, one to the right if any, else one to the left;
//!    then the first in raster order. ([`bleed`])
//! 2. **Horizontal pass, then vertical**, each axis with Image Size's Bicubic Automatic filter
//!    for its own scale: Bicubic Sharper where it shrinks, Bicubic Smoother where it grows. An
//!    axis that keeps its size is not filtered.
//! 3. **Each pass:** output sample `o` reads the source at `(o + ½)/k − ½`, clamped to the first
//!    and last sample and rounded to 1/128 pixel. The kernel's weights are 14-bit integers: each
//!    normalised weight times 16384, truncated toward zero, except the largest, which takes what
//!    the others leave of 16384. Colour is filtered straight (not premultiplied) with the edge
//!    pixels repeated outwards; alpha is filtered on its own, reading transparency beyond the
//!    edges. Each result is rounded, halves up, and clipped to the sample range.
//! 4. **Below ¼ along an axis**, the image is first averaged down to four times the target (each
//!    sample the plain mean of the samples it covers, rounded), then filtered as at ¼.
//!
//! At 8 and 16 bits the arithmetic is exact in whole sample levels; at 32 bits the same steps run
//! unrounded.

use photocraft_color::SampleType;
use photocraft_geom::Rect;
use photocraft_raster::Surface;

/// The weights' unit: 14 bits.
pub(crate) const ONE: i64 = 1 << 14;

/// Samples (pixels × channels) one scaled image may hold at any step; larger ones return `None`
/// and take the general transform path, which bounds its own work.
pub(crate) const MAX_SAMPLES: usize = 1 << 31;

/// An image of whole sample levels (at 8 and 16 bits) or plain values (at 32 bits), interleaved,
/// the last channel alpha.
#[derive(Clone)]
pub(crate) struct Image {
    pub(crate) w: usize,
    pub(crate) h: usize,
    pub(crate) n: usize,
    pub(crate) px: Vec<f32>,
}

impl Image {
    pub(crate) fn at(&self, x: usize, y: usize, c: usize) -> f32 {
        self.px.get((y * self.w + x) * self.n + c).copied().unwrap_or(0.0)
    }
}

/// How one depth's samples are stored and rounded.
#[derive(Clone, Copy)]
pub(crate) struct Depth {
    /// The largest level (255, 65535), or `None` at 32 bits (unrounded, colour unclipped).
    pub(crate) levels: Option<f32>,
}

/// 1.5 × 2⁵²: an f64 this large has no fraction bits, so adding it to a far smaller number and
/// taking it away again leaves that number rounded to a whole one.
const ROUNDER: f64 = 6_755_399_441_055_744.0;
/// Sums below this (2⁴⁰) are far smaller than [`ROUNDER`], and so are their halves and quotients.
const WHOLE_LIMIT: f64 = 1_099_511_627_776.0;

impl Depth {
    pub(crate) fn of(sample: SampleType) -> Self {
        match sample {
            SampleType::U8 => Depth { levels: Some(255.0) },
            SampleType::U16 => Depth { levels: Some(65535.0) },
            SampleType::F32 => Depth { levels: None },
        }
    }

    /// `sum / ONE`, rounded halves up and clipped to the levels (alpha to [0, 1] at 32 bits).
    pub(crate) fn settle(self, sum: f64, alpha: bool) -> f32 {
        match self.levels {
            Some(top) => {
                // A whole sum (as every sum of whole levels is) settles without leaving f64:
                // adding and taking away `ROUNDER` rounds to a whole number, and adding the half
                // and scaling by a power of two are exact.
                let whole = (sum + ROUNDER) - ROUNDER;
                if whole == sum && sum.abs() < WHOLE_LIMIT {
                    let scaled = (sum + (ONE / 2) as f64) * (1.0 / ONE as f64);
                    let near = (scaled + ROUNDER) - ROUNDER;
                    let floor = if near > scaled { near - 1.0 } else { near };
                    return (floor as f32).clamp(0.0, top);
                }
                (nearest(sum).saturating_add(ONE / 2).div_euclid(ONE) as f32).clamp(0.0, top)
            }
            None if alpha => (sum / ONE as f64).clamp(0.0, 1.0) as f32,
            None => (sum / ONE as f64) as f32,
        }
    }

    /// The rounded mean of `count` samples summing to `sum`.
    pub(crate) fn mean(self, sum: f64, count: usize) -> f32 {
        let count = count.max(1) as f64;
        match self.levels {
            Some(top) => ((sum / count + 0.5).floor() as f32).clamp(0.0, top),
            None => (sum / count) as f32,
        }
    }
}

/// `level.round()` for a sample of a whole-number depth scaled to its levels (never negative,
/// never NaN, far below 2³¹), without the call into the maths library.
#[inline]
fn whole(level: f32) -> f32 {
    // Exact: an f64 holds an f32 plus a half, and cutting that off rounds halves up.
    (f64::from(level) + 0.5) as i32 as f32
}

/// `sum` rounded to the nearest whole number, halves away from zero: `sum.round() as i64` (NaN is
/// 0, the ends saturate) without the library call rounding costs where the target has no
/// instruction for it. A pass settles every sample it writes, so this is its inner loop.
pub(crate) fn nearest(sum: f64) -> i64 {
    let whole = sum as i64;
    // Exact: `whole` is `sum` truncated, which an f64 holds.
    let rest = sum - whole as f64;
    whole.saturating_add(i64::from(rest >= 0.5)).saturating_sub(i64::from(rest <= -0.5))
}

/// Clears the colour of every fully transparent pixel in `data` (interleaved pixels of `fmt`): the
/// renders bleed the nearest visible colour into transparent source pixels, as the reference
/// application filters them, and that colour must not stay behind under alpha 0, where it would
/// count as the layer's content.
pub(crate) fn clear_hidden(data: &mut [f32], fmt: &photocraft_color::PixelFormat) {
    let n = fmt.channels();
    if !fmt.alpha || n < 2 {
        return;
    }
    crate::par::chunks_mut(data, n << 14, true, |_, part| {
        for p in part.chunks_exact_mut(n) {
            if let Some((al, colour)) = p.split_last_mut()
                && *al <= 0.0
            {
                colour.fill(0.0);
                *al = 0.0;
            }
        }
    });
}

/// Rows read per task by [`load`].
const LOAD_ROWS: usize = 32;

/// The pixels of `src` inside `bounds` as an [`Image`]: whole levels at 8 and 16 bits, the stored
/// values at 32. Bands of rows are read on all cores.
pub(crate) fn load(src: &Surface, bounds: Rect, depth: Depth) -> Image {
    /// Rows read at a time.
    const READ_ROWS: usize = 4;
    let (w, h, n) = (bounds.width() as usize, bounds.height() as usize, src.channels());
    let mut px = vec![0.0f32; w * h * n];
    crate::par::chunks_mut(&mut px, LOAD_ROWS * w * n, true, |i, part| {
        // A few rows at a time, through one buffer that stays in the cache.
        let mut band = Vec::new();
        for (j, rows) in part.chunks_mut((READ_ROWS * w * n).max(1)).enumerate() {
            let count = rows.len() / (w * n).max(1);
            let (Ok(top), Ok(count)) = (i32::try_from(i * LOAD_ROWS + j * READ_ROWS), i32::try_from(count)) else { return };
            let y = bounds.y0.saturating_add(top);
            src.read_region_into(Rect::new(bounds.x0, y, bounds.x1, y.saturating_add(count)), &mut band);
            match depth.levels {
                Some(unit) => rows.iter_mut().zip(&band).for_each(|(o, v)| *o = whole(v.clamp(0.0, 1.0) * unit)),
                None => rows.iter_mut().zip(&band).for_each(|(o, v)| *o = *v),
            }
        }
    });
    Image { w, h, n, px }
}

/// A surface of `fmt` whose `rect` holds the rows `fill` computes: `fill(y, x, out)` writes the
/// pixels of row `y` from column `x` on (both counted from `rect`'s corner) into `out`, a whole
/// number of pixels in an [`Image`]'s samples, zero to start with. With `clear`, fully transparent
/// pixels lose their colour ([`clear_hidden`]). Rows go straight into the surface's tiles, on all
/// cores, so `fill` must not depend on call order. `None` if the tiles can't be allocated.
pub(crate) fn store(
    fmt: photocraft_color::PixelFormat,
    rect: Rect,
    depth: Depth,
    clear: bool,
    fill: impl Fn(usize, usize, &mut [f32]) + Sync,
) -> Option<Surface> {
    let n = fmt.channels();
    let size = fmt.sample.bytes();
    let unit = depth.levels.unwrap_or(1.0);
    let hide = clear && fmt.alpha && n >= 2;
    let mut surface = Surface::new(fmt);
    let rows = |y: i32, x: i32, dst: &mut [u8]| {
        let (Some(row), Some(col)) = (y.checked_sub(rect.y0), x.checked_sub(rect.x0)) else { return };
        let (Ok(row), Ok(col)) = (usize::try_from(row), usize::try_from(col)) else { return };
        let mut px = vec![0.0f32; dst.len() / size.max(1)];
        fill(row, col, &mut px);
        for (p, d) in px.chunks_exact_mut(n).zip(dst.chunks_exact_mut(n * size)) {
            if hide && p.last().is_some_and(|al| *al <= 0.0) {
                p.fill(0.0);
            }
            for (i, v) in p.iter().enumerate() {
                photocraft_color::write_sample(d, fmt.sample, i, *v / unit);
            }
        }
    };
    surface.try_write_rows_with(rect, rows).ok()?;
    Some(surface)
}

/// The pixels of `src` inside `bounds` (its contents' canvas; `src` has alpha), resized to
/// `width` × `height` as the reference application draws a scaled smart object, placed with its
/// top-left corner at `origin`. `None` for an empty size or one past [`MAX_SAMPLES`].
pub fn scale(src: &Surface, bounds: Rect, width: u32, height: u32, origin: (i32, i32)) -> Option<Surface> {
    let fmt = src.format();
    let n = src.channels();
    if !fmt.alpha || n == 0 || bounds.is_empty() || width == 0 || height == 0 {
        return None;
    }
    let (w, h) = (bounds.width() as usize, bounds.height() as usize);
    let (nx, ny) = (width as usize, height as usize);
    let biggest = w.max(nx).checked_mul(h.max(ny))?.checked_mul(n)?;
    if biggest > MAX_SAMPLES {
        return None;
    }
    let depth = Depth::of(fmt.sample);
    let mut img = load(src, bounds, depth);
    bleed(&mut img);
    if nx != w {
        img = axis(&img, nx, depth, false);
    }
    let rect = Rect::new(origin.0, origin.1, origin.0.checked_add(i32::try_from(width).ok()?)?, origin.1.checked_add(i32::try_from(height).ok()?)?);
    if ny == h {
        return store(fmt, rect, depth, true, |y, x, out| {
            if let Some(row) = img.px.get((y * img.w + x) * n..).and_then(|s| s.get(..out.len())) {
                out.copy_from_slice(row);
            }
        });
    }
    // The rows are filtered as they are written: no image of the full size in between.
    let (mid, k, a, stretch) = prepared(&img, ny, depth, true);
    let img = mid.unwrap_or(img);
    let taps: Vec<Taps> = (0..ny).map(|o| taps(o, img.h, k, a, stretch)).collect();
    store(fmt, rect, depth, true, |y, x, out| {
        if let Some(t) = taps.get(y) {
            blend_rows(&img, t, x, depth, out);
        }
    })
}

/// One axis resized to `len` samples (`vertical`: the rows), with the Automatic filter for its
/// scale, through the averaged image below ¼.
fn axis(img: &Image, len: usize, depth: Depth, vertical: bool) -> Image {
    let (mid, k, a, stretch) = prepared(img, len, depth, vertical);
    pass(mid.as_ref().unwrap_or(img), len, k, a, stretch, depth, vertical)
}

/// What an axis's filtering pass to `len` samples works with: the averaged image it reads (below
/// ¼; otherwise it reads `img` itself), then its scale, Keys parameter and stretch.
fn prepared(img: &Image, len: usize, depth: Depth, vertical: bool) -> (Option<Image>, f64, f64, f64) {
    let have = if vertical { img.h } else { img.w };
    let k = len as f64 / have as f64;
    if k < 0.25 {
        let (a, stretch) = automatic(0.25);
        (Some(average(img, 4 * len, depth, vertical)), len as f64 / (4 * len) as f64, a, stretch)
    } else {
        let (a, stretch) = automatic(k);
        (None, k, a, stretch)
    }
}

/// Bicubic Automatic's Keys parameter and stretch for scale `k`: Sharper shrinking (designed for
/// `k`, down to ¼), Smoother growing.
fn automatic(k: f64) -> (f64, f64) {
    if k < 1.0 {
        let s = k.clamp(0.25, 1.0);
        (-1.0 - 1.6 * (1.0 - s), 1.05 / s)
    } else {
        (-0.625, 1.15)
    }
}

pub(crate) fn keys(x: f64, a: f64) -> f64 {
    let x = x.abs();
    if x < 1.0 {
        (a + 2.0) * x * x * x - (a + 3.0) * x * x + 1.0
    } else if x < 2.0 {
        a * (x * x * x - 5.0 * x * x + 8.0 * x - 4.0)
    } else {
        0.0
    }
}

/// One output sample's source window: the first source index and the 14-bit weights.
struct Taps {
    start: i64,
    q: Vec<i64>,
}

/// The taps of output sample `o` of `have` source samples at scale `k`.
fn taps(o: usize, have: usize, k: f64, a: f64, stretch: f64) -> Taps {
    let last = have.saturating_sub(1) as f64;
    let u = ((o as f64 + 0.5) / k - 0.5).clamp(0.0, last);
    let j = u.floor();
    let u = j + ((u - j) * 128.0).round_ties_even() / 128.0;
    let (lo, hi) = ((u - 2.0 * stretch).floor() as i64, (u + 2.0 * stretch).ceil() as i64);
    let w: Vec<f64> = (lo..=hi).map(|i| keys((u - i as f64) / stretch, a)).collect();
    let total: f64 = w.iter().sum();
    let mut q: Vec<i64> = w.iter().map(|v| (v / total * ONE as f64).trunc() as i64).collect();
    // The largest weight (the first, if several) takes the remainder.
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
    Taps { start: lo, q }
}

/// One filtering pass along an axis to `len` samples at scale `k` (rule 3 of the module).
fn pass(img: &Image, len: usize, k: f64, a: f64, stretch: f64, depth: Depth, vertical: bool) -> Image {
    let have = if vertical { img.h } else { img.w };
    let n = img.n;
    let taps: Vec<Taps> = (0..len).map(|o| taps(o, have, k, a, stretch)).collect();
    let (ow, oh) = if vertical { (img.w, len) } else { (len, img.h) };
    let mut px = vec![0.0f32; ow * oh * n];
    crate::par::chunks_mut(&mut px, ow * n, true, |y, row| {
        if vertical {
            if let Some(t) = taps.get(y) {
                blend_rows(img, t, 0, depth, row);
            }
        } else if let Some(line) = img.px.get(y * img.w * n..(y + 1) * img.w * n) {
            blend_line(line, n, &taps, depth, row);
        }
    });
    Image { w: ow, h: oh, n, px }
}

/// One row of a horizontal pass: output pixel `o` of `row` is `taps[o]` over the pixels of `line`
/// (`n` channels each). Colour repeats the edge outwards; alpha reads transparency there.
fn blend_line(line: &[f32], n: usize, taps: &[Taps], depth: Depth, row: &mut [f32]) {
    let alpha = n.saturating_sub(1);
    let last = (line.len() / n.max(1)) as i64 - 1;
    let mut acc = vec![0.0f64; n];
    for (out, t) in row.chunks_exact_mut(n).zip(taps) {
        acc.fill(0.0);
        let window = usize::try_from(t.start).ok().filter(|_| t.start + t.q.len() as i64 - 1 <= last).and_then(|s| line.get(s * n..(s + t.q.len()) * n));
        if let Some(window) = window {
            // Every tap inside the line: its pixels lie side by side.
            for (p, &q) in window.chunks_exact(n).zip(&t.q) {
                let q = q as f64;
                acc.iter_mut().zip(p).for_each(|(s, v)| *s += f64::from(*v) * q);
            }
        } else {
            for (step, &q) in t.q.iter().enumerate() {
                let i = t.start + step as i64;
                let at = i.clamp(0, last.max(0)) as usize * n;
                let Some(p) = line.get(at..at + n) else { continue };
                let channels = if (0..=last).contains(&i) { n } else { alpha };
                let q = q as f64;
                acc.iter_mut().zip(p).take(channels).for_each(|(s, v)| *s += f64::from(*v) * q);
            }
        }
        for (c, (v, s)) in out.iter_mut().zip(&acc).enumerate() {
            *v = depth.settle(*s, c == alpha);
        }
    }
}

/// Part of one row of a vertical pass: `out`'s pixels, from column `x` on, are the taps `t` over
/// the rows of `img`. Colour repeats the edge outwards; alpha reads transparency there.
fn blend_rows(img: &Image, t: &Taps, x: usize, depth: Depth, out: &mut [f32]) {
    let n = img.n;
    let alpha = n.saturating_sub(1);
    let last = img.h as i64 - 1;
    let mut acc = vec![0.0f64; out.len()];
    for (step, &q) in t.q.iter().enumerate() {
        let i = t.start + step as i64;
        let at = (i.clamp(0, last.max(0)) as usize * img.w + x) * n;
        let Some(src) = img.px.get(at..).and_then(|s| s.get(..out.len())) else { continue };
        let q = q as f64;
        if (0..=last).contains(&i) {
            acc.iter_mut().zip(src).for_each(|(s, v)| *s += f64::from(*v) * q);
        } else {
            for (s, p) in acc.chunks_exact_mut(n).zip(src.chunks_exact(n)) {
                s.iter_mut().zip(p).take(alpha).for_each(|(s, v)| *s += f64::from(*v) * q);
            }
        }
    }
    for (o, s) in out.chunks_exact_mut(n).zip(acc.chunks_exact(n)) {
        for (c, (v, s)) in o.iter_mut().zip(s).enumerate() {
            *v = depth.settle(*s, c == alpha);
        }
    }
}

/// An axis averaged down to `len` samples: each the rounded mean of the samples it covers
/// (rule 4 of the module).
fn average(img: &Image, len: usize, depth: Depth, vertical: bool) -> Image {
    let have = if vertical { img.h } else { img.w };
    let n = img.n;
    let spans: Vec<(usize, usize)> = (0..len)
        .map(|o| {
            let a = o * have / len;
            let b = ((o + 1) * have / len).max(a + 1).min(have);
            (a, b)
        })
        .collect();
    let (ow, oh) = if vertical { (img.w, len) } else { (len, img.h) };
    let mut px = vec![0.0f32; ow * oh * n];
    crate::par::chunks_mut(&mut px, ow * n, true, |y, row| {
        for (x, out) in row.chunks_exact_mut(n).enumerate() {
            let (o, line) = if vertical { (y, x) } else { (x, y) };
            let Some(&(a, b)) = spans.get(o) else { continue };
            for (c, v) in out.iter_mut().enumerate() {
                let sum: f64 = (a..b).map(|i| f64::from(if vertical { img.at(line, i, c) } else { img.at(i, line, c) })).sum();
                *v = depth.mean(sum, b - a);
            }
        }
    });
    Image { w: ow, h: oh, n, px }
}

/// No seed: a packed seed is `(y << 32) | x`.
const NONE: u64 = u64::MAX;

fn unpack(s: u64) -> (i64, i64) {
    ((s & 0xffff_ffff) as i64, (s >> 32) as i64)
}

/// The order in which equally near seeds win, by where they lie from the pixel (`dy`, `dx` the
/// seed's offset, `+y` down): below, then above, then level; within each, right, left, in line.
fn rank(dy: i64, dx: i64) -> u8 {
    let row = if dy > 0 {
        0
    } else if dy < 0 {
        1
    } else {
        2
    };
    let col = if dx > 0 {
        0
    } else if dx < 0 {
        1
    } else {
        2
    };
    row * 3 + col
}

/// How seed `s` ranks for the pixel `(x, y)`: nearer first, then by [`rank`], then the topmost
/// (in one direction, at one distance, the topmost is the first in raster order).
fn standing(s: u64, x: i64, y: i64) -> (i64, u8, i64) {
    if s == NONE {
        return (i64::MAX, u8::MAX, i64::MAX);
    }
    let (sx, sy) = unpack(s);
    let (dx, dy) = (sx - x, sy - y);
    (dx.abs() + dy.abs(), rank(dy, dx), sy)
}

/// One quadrant's cone: of the seeds at or beyond a pixel in its direction, the nearest, and of
/// equally near ones the topmost. `down`/`right` name the quadrant.
#[derive(Clone, Copy)]
struct Cone {
    down: bool,
    right: bool,
}

impl Cone {
    /// What the cone minimises, for seed `s`: the distance along the quadrant's diagonal (up to a
    /// constant of the pixel) and then the row.
    fn key(self, s: u64) -> (i64, i64) {
        if s == NONE {
            return (i64::MAX, i64::MAX);
        }
        let (sx, sy) = unpack(s);
        let along = match (self.down, self.right) {
            (true, true) => sy + sx,
            (true, false) => sy - sx,
            (false, true) => sx - sy,
            (false, false) => -(sy + sx),
        };
        (along, sy)
    }

    fn better(self, a: u64, b: u64) -> u64 {
        if self.key(a) <= self.key(b) { a } else { b }
    }
}

/// Rule 1 of the module: every fully transparent pixel takes the colour of the nearest visible
/// one. Exact and linear in the pixels: two sweeps, up the image and down it, each keep (per
/// column) the cones of the two quadrants behind them and the nearest seed straight behind, and
/// (per row) the nearest seeds left and right; a pixel takes the best of the eight directions.
/// Column strips run in parallel: a strip also tracks the column on each side of it, and starts
/// its row scans from the nearest seeds beyond those.
pub(crate) fn bleed(img: &mut Image) {
    #[cfg(not(target_arch = "wasm32"))]
    let threads = rayon::current_num_threads().max(1);
    #[cfg(target_arch = "wasm32")]
    let threads = 1;
    let width = img.w.div_ceil((threads * 2).min(img.w).max(1)).max(64);
    bleed_in_strips(img, width);
}

/// [`bleed`] in strips of `width` columns.
fn bleed_in_strips(img: &mut Image, width: usize) {
    let (w, h, n) = (img.w, img.h, img.n);
    let alpha = n - 1;
    let Some(len) = w.checked_mul(h) else { return };
    let mut visible = vec![false; len];
    crate::par::chunks_mut(&mut visible, w.max(1), true, |y, row| {
        let Some(line) = img.px.get(y * w * n..(y + 1) * w * n) else { return };
        for (v, px) in row.iter_mut().zip(line.chunks_exact(n)) {
            *v = px.get(alpha).is_some_and(|&a| a > 0.0);
        }
    });
    if !visible.contains(&false) || !visible.contains(&true) {
        return;
    }
    let width = width.max(1);
    let strips: Vec<(usize, usize)> = (0..w).step_by(width).map(|x0| (x0, (x0 + width).min(w))).collect();
    // Per row, for each strip: the nearest seed right of its state columns and left of them.
    let halos: Vec<Vec<(u64, u64)>> = crate::par::map(h, true, |y| {
        let row = &visible[y * w..(y + 1) * w];
        let mut out = vec![(NONE, NONE); strips.len()];
        let mut seen = NONE;
        let mut k = strips.len();
        for x in (0..w).rev() {
            while k > 0 && strips.get(k - 1).is_some_and(|&(_, x1)| (x1 + 1).min(w) > x) {
                k -= 1;
                if let Some(o) = out.get_mut(k) {
                    o.0 = seen;
                }
            }
            if row.get(x) == Some(&true) {
                seen = pack(x, y);
            }
        }
        while k > 0 {
            k -= 1;
            if let Some(o) = out.get_mut(k) {
                o.0 = seen;
            }
        }
        seen = NONE;
        let mut k = 0;
        for x in 0..w {
            while strips.get(k).is_some_and(|&(x0, _)| x0.saturating_sub(1) <= x) {
                if let Some(o) = out.get_mut(k) {
                    o.1 = seen;
                }
                k += 1;
            }
            if row.get(x) == Some(&true) {
                seen = pack(x, y);
            }
        }
        out
    });
    let bests: Vec<Vec<u64>> = crate::par::map(strips.len(), true, |k| {
        let Some(&(x0, x1)) = strips.get(k) else { return Vec::new() };
        strip(&visible, w, h, x0, x1, |y| halos.get(y).and_then(|r| r.get(k)).copied().unwrap_or((NONE, NONE)))
    });
    for (&(x0, x1), best) in strips.iter().zip(&bests) {
        let sw = x1 - x0;
        for (j, &s) in best.iter().enumerate() {
            if s == NONE {
                continue;
            }
            let i = (j / sw) * w + x0 + j % sw;
            let (sx, sy) = unpack(s);
            let from = (sy as usize * w + sx as usize) * n;
            if from + alpha <= img.px.len() && i * n + alpha <= img.px.len() {
                img.px.copy_within(from..from + alpha, i * n);
            }
        }
    }
}

fn pack(x: usize, y: usize) -> u64 {
    ((y as u64) << 32) | x as u64
}

/// [`bleed`]'s sweeps over the columns `[x0, x1)`: each transparent pixel's best seed (row-major
/// over the strip, [`NONE`] where it is visible). `halo(y)` is the nearest seed in row `y` right
/// of column `x1` and left of column `x0 - 1`.
fn strip(visible: &[bool], w: usize, h: usize, x0: usize, x1: usize, halo: impl Fn(usize) -> (u64, u64)) -> Vec<u64> {
    // State columns: one beyond the strip on each side, where there is one.
    let (s0, s1) = (x0.saturating_sub(1), (x1 + 1).min(w));
    let sn = s1 - s0;
    let sw = x1 - x0;
    let mut best = vec![NONE; sw * h];
    let (mut right, mut left) = (vec![NONE; sn], vec![NONE; sn]);
    let get = |v: &[u64], i: Option<usize>| i.and_then(|i| v.get(i).copied()).unwrap_or(NONE);
    for down in [true, false] {
        let (ahead, beside) = (Cone { down, right: true }, Cone { down, right: false });
        let (mut cone_r, mut cone_l) = (vec![NONE; sn], vec![NONE; sn]);
        let mut behind = vec![NONE; sn];
        let rows: Box<dyn Iterator<Item = usize>> = if down { Box::new((0..h).rev()) } else { Box::new(0..h) };
        for y in rows {
            let row = y * w;
            let (beyond_r, beyond_l) = halo(y);
            let mut seen = beyond_r;
            for c in (0..sn).rev() {
                if visible.get(row + s0 + c) == Some(&true) {
                    seen = pack(s0 + c, y);
                }
                if let Some(r) = right.get_mut(c) {
                    *r = seen;
                }
            }
            seen = beyond_l;
            for c in 0..sn {
                if visible.get(row + s0 + c) == Some(&true) {
                    seen = pack(s0 + c, y);
                }
                if let Some(l) = left.get_mut(c) {
                    *l = seen;
                }
            }
            let yi = y as i64;
            for x in x0..x1 {
                if visible.get(row + x) != Some(&false) {
                    continue;
                }
                let c = x - s0;
                let mut cands = [get(&cone_r, Some(c + 1)), get(&cone_l, c.checked_sub(1)), get(&behind, Some(c)), NONE, NONE];
                if down {
                    cands[3] = get(&right, Some(c + 1));
                    cands[4] = get(&left, c.checked_sub(1));
                    // Past the strip's state columns: the halo seeds.
                    if c + 1 >= sn {
                        cands[3] = beyond_r;
                    }
                    if c == 0 {
                        cands[4] = beyond_l;
                    }
                }
                let xi = x as i64;
                let Some(b) = best.get_mut(y * sw + x - x0) else { continue };
                let mut now = standing(*b, xi, yi);
                for s in cands {
                    let st = standing(s, xi, yi);
                    if st < now {
                        *b = s;
                        now = st;
                    }
                }
            }
            for c in 0..sn {
                if let (Some(cr), Some(&r)) = (cone_r.get_mut(c), right.get(c)) {
                    *cr = ahead.better(r, *cr);
                }
                if let (Some(cl), Some(&l)) = (cone_l.get_mut(c), left.get(c)) {
                    *cl = beside.better(l, *cl);
                }
                if visible.get(row + s0 + c) == Some(&true)
                    && let Some(b) = behind.get_mut(c)
                {
                    *b = pack(s0 + c, y);
                }
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A straight RGBA 8-bit image from levels.
    fn image(w: usize, h: usize, px: &[[f32; 4]]) -> Image {
        Image { w, h, n: 4, px: px.iter().flatten().copied().collect() }
    }

    /// The brute-force rule the sweeps must reproduce.
    fn bleed_slow(img: &Image) -> Vec<f32> {
        let mut out = img.px.clone();
        let seeds: Vec<(i64, i64)> =
            (0..img.h).flat_map(|y| (0..img.w).map(move |x| (x, y))).filter(|&(x, y)| img.at(x, y, 3) > 0.0).map(|(x, y)| (x as i64, y as i64)).collect();
        for y in 0..img.h {
            for x in 0..img.w {
                if img.at(x, y, 3) > 0.0 || seeds.is_empty() {
                    continue;
                }
                let key = |&(sx, sy): &(i64, i64)| {
                    let (dx, dy) = (sx - x as i64, sy - y as i64);
                    (dx.abs() + dy.abs(), rank(dy, dx), sy, sx)
                };
                let s = seeds.iter().min_by_key(|s| key(s)).unwrap();
                for c in 0..3 {
                    out[(y * img.w + x) * 4 + c] = img.at(s.0 as usize, s.1 as usize, c);
                }
            }
        }
        out
    }

    #[test]
    fn bleeding_matches_the_nearest_visible_pixel_rule_everywhere() {
        // Pseudo-random sparse seeds with distinct colours, many ties.
        let mut s = 12345u32;
        for case in 0..40 {
            let (w, h) = (5 + case % 13 + 9 * (case % 3), 4 + case % 11);
            let px: Vec<[f32; 4]> = (0..w * h)
                .map(|i| {
                    s = s.wrapping_mul(1664525).wrapping_add(1013904223);
                    if (s >> 24).is_multiple_of(9) {
                        [(i % 256) as f32, ((i * 7) % 256) as f32, (s >> 8 & 255) as f32, 1.0 + (s >> 16 & 7) as f32]
                    } else {
                        [9.0, 9.0, 9.0, 0.0]
                    }
                })
                .collect();
            let img = image(w, h, &px);
            let want = bleed_slow(&img);
            for width in [1, 2, 3, 4, 64] {
                let mut got = img.clone();
                bleed_in_strips(&mut got, width);
                assert_eq!(got.px, want, "case {case} ({w} x {h}), strips of {width}");
            }
        }
    }

    #[test]
    fn an_equal_tie_goes_below_then_above_then_right() {
        // The centre of a 3 x 3 is 1 from all four sides.
        let t = [0.0, 0.0, 0.0, 0.0];
        let (up, down, left, right) = ([10.0, 0.0, 0.0, 255.0], [20.0, 0.0, 0.0, 255.0], [30.0, 0.0, 0.0, 255.0], [40.0, 0.0, 0.0, 255.0]);
        let mut img = image(3, 3, &[t, up, t, left, t, right, t, down, t]);
        bleed(&mut img);
        assert_eq!(img.at(1, 1, 0), 20.0, "below wins");
        let mut img = image(3, 3, &[t, up, t, left, t, right, t, t, t]);
        bleed(&mut img);
        assert_eq!(img.at(1, 1, 0), 10.0, "then above");
        let mut img = image(3, 3, &[t, t, t, left, t, right, t, t, t]);
        bleed(&mut img);
        assert_eq!(img.at(1, 1, 0), 40.0, "then right");
    }

    #[test]
    fn weights_are_fourteen_bit_and_sum_to_one() {
        for o in 0..40 {
            for &(k, have) in &[(0.5, 96), (0.75, 96), (1.5, 96), (2.0, 96), (0.25, 400)] {
                let (a, st) = automatic(k);
                let t = taps(o, have, k, a, st);
                assert_eq!(t.q.iter().sum::<i64>(), ONE);
            }
        }
    }

    #[test]
    fn the_largest_weight_takes_the_remainder_of_truncated_ones() {
        // 75 %, output 6: Sharper at 0.75 (a = -1.4, stretch 1.4), position 6.5 / 0.75 - 0.5 =
        // 8.1666.. → 8 + 21/128.
        let (a, st) = automatic(0.75);
        let t = taps(6, 96, 0.75, a, st);
        let u = 8.0 + 21.0 / 128.0;
        let w: Vec<f64> = (t.start..t.start + t.q.len() as i64).map(|i| keys((u - i as f64) / st, a)).collect();
        let total: f64 = w.iter().sum();
        let big = (0..w.len()).fold(0, |b, i| if w[i] > w[b] { i } else { b });
        assert_eq!(t.start + big as i64, 8);
        for (i, (&q, &w)) in t.q.iter().zip(&w).enumerate() {
            if i != big {
                assert_eq!(q, (w / total * 16384.0).trunc() as i64, "tap {i}");
            }
        }
    }

    /// The bleed probe's "pocket": a red band on top, a blue one down the right, a green one along
    /// the bottom and a yellow block, on a 96 x 96 transparent canvas.
    fn pocket() -> Surface {
        use photocraft_color::{ColorMode, PixelFormat};
        let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
        s.write_region(Rect::new(0, 0, 96, 96), &[0.0; 96 * 96 * 4]);
        for (r, c) in [
            (Rect::new(0, 0, 96, 20), [230, 20, 20]),
            (Rect::new(70, 20, 96, 96), [30, 60, 240]),
            (Rect::new(0, 80, 70, 96), [20, 200, 40]),
            (Rect::new(20, 40, 45, 60), [250, 220, 10]),
        ] {
            s.fill_rect(r, &[c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0]);
        }
        s
    }

    /// Row `y` of `s` as the reference stores it in its composite: colour matted on white, alpha.
    fn matted_row(s: &Surface, y: i32, width: i32) -> Vec<[u8; 4]> {
        (0..width)
            .map(|x| {
                let p: Vec<f32> = s.pixel(x, y).iter().map(|v| (v * 255.0).round()).collect();
                let a = p[3] / 255.0;
                let m = |c: f32| (c * a + 255.0 * (1.0 - a) + 0.5).floor() as u8;
                [m(p[0]), m(p[1]), m(p[2]), p[3] as u8]
            })
            .collect()
    }

    /// Byte for byte the reference application's render of the pocket scaled to 25 % (row 4,
    /// where the bled colour of the gap pulls the red band's edge) and to 12.5 % (row 10, through
    /// the averaged image): bleed, straight colour, integer weights and rounding all show here.
    #[test]
    fn a_scaled_smart_object_matches_the_reference_byte_for_byte() {
        let src = pocket();
        let bounds = Rect::new(0, 0, 96, 96);
        let quarter = scale(&src, bounds, 24, 24, (0, 0)).unwrap();
        let want: [[u8; 4]; 24] = [
            [233, 44, 44, 229],
            [231, 32, 32, 242],
            [231, 32, 32, 242],
            [231, 32, 32, 242],
            [231, 32, 32, 242],
            [231, 32, 32, 242],
            [231, 32, 32, 242],
            [231, 32, 32, 242],
            [231, 32, 32, 242],
            [231, 32, 32, 242],
            [231, 32, 32, 242],
            [231, 32, 32, 242],
            [231, 32, 32, 242],
            [231, 32, 32, 242],
            [231, 32, 32, 242],
            [227, 33, 35, 242],
            [249, 28, 14, 242],
            [244, 24, 14, 248],
            [213, 24, 34, 255],
            [220, 22, 31, 255],
            [220, 22, 31, 255],
            [220, 22, 31, 255],
            [220, 22, 31, 255],
            [222, 34, 42, 242],
        ];
        assert_eq!(matted_row(&quarter, 4, 24), want);
        let eighth = scale(&src, bounds, 12, 12, (0, 0)).unwrap();
        let want: [[u8; 4]; 12] = [
            [26, 202, 46, 248],
            [20, 200, 40, 255],
            [17, 200, 40, 255],
            [16, 200, 41, 255],
            [16, 200, 41, 255],
            [17, 200, 40, 255],
            [20, 197, 44, 255],
            [17, 231, 0, 255],
            [22, 178, 72, 255],
            [31, 42, 255, 255],
            [30, 57, 244, 255],
            [30, 60, 240, 255],
        ];
        assert_eq!(matted_row(&eighth, 10, 12), want);
    }

    /// Numbers to test with: the same ones every run.
    fn noise(seed: u64) -> impl FnMut() -> u64 {
        let mut s = seed;
        move || {
            s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let z = (s ^ (s >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            let z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }
    }

    #[test]
    fn nearest_rounds_as_the_library_does() {
        let half = 0.5f64;
        let special = [
            0.0,
            -0.0,
            half,
            -half,
            half.next_down(),
            -half.next_down(),
            1.5,
            -1.5,
            2.5,
            -2.5,
            4_503_599_627_370_495.5,
            -4_503_599_627_370_495.5,
            4_503_599_627_370_496.0,
            9_007_199_254_740_993.0,
            9.3e18,
            -9.3e18,
            1e300,
            -1e300,
            f64::MAX,
            f64::MIN,
            f64::MIN_POSITIVE,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
        ];
        for v in special {
            assert_eq!(nearest(v), v.round() as i64, "{v}");
        }
        let mut next = noise(1);
        for _ in 0..200_000 {
            let bits = next();
            // Any number at all, and ones on and beside the halves between whole numbers.
            let any = f64::from_bits(bits);
            let beside = ((bits >> 40) as i64 - (1 << 23)) as f64 / 2.0 + [0.0, 1e-6, -1e-6][(bits % 3) as usize];
            for v in [any, beside] {
                assert_eq!(nearest(v), v.round() as i64, "{v}");
            }
        }
    }

    /// Settling a whole sum stays in floating point; it must give the level the rule in whole
    /// numbers gives, and anything else must still go through that rule.
    #[test]
    fn settling_a_sum_gives_the_level_of_the_whole_number_rule() {
        let rule = |sum: f64, top: f32| ((sum.round() as i64).saturating_add(ONE / 2).div_euclid(ONE) as f32).clamp(0.0, top);
        let mut sums = vec![
            0.0,
            -0.0,
            0.5,
            8191.0,
            8192.0,
            8193.0,
            -8191.0,
            -8192.0,
            -8193.0,
            8191.5,
            8192.5,
            WHOLE_LIMIT - 1.0,
            WHOLE_LIMIT,
            WHOLE_LIMIT + 8192.0,
            -WHOLE_LIMIT,
            1e30,
            -1e30,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
        ];
        let mut next = noise(2);
        for _ in 0..200_000 {
            let bits = next();
            // On and beside the sums where one level becomes the next, below zero and past the
            // top too; halves; sums far too large; any number at all.
            let level = (bits >> 44) as i64 - (1 << 18);
            let edge = (level * ONE + ONE / 2 + (bits % 5) as i64 - 2) as f64;
            sums.extend([edge, edge + 0.5, ((bits >> 8) as i64 - (1 << 55)) as f64, f64::from_bits(bits)]);
        }
        for top in [255.0, 65535.0] {
            let depth = Depth { levels: Some(top) };
            for &sum in &sums {
                assert_eq!(depth.settle(sum, false).to_bits(), rule(sum, top).to_bits(), "{sum} of {top}");
                assert_eq!(depth.settle(sum, true).to_bits(), rule(sum, top).to_bits(), "alpha {sum} of {top}");
            }
        }
    }

    #[test]
    fn a_loaded_level_is_rounded_as_the_library_rounds() {
        assert_eq!(whole(0.5f32.next_down()), 0.0);
        let mut next = noise(3);
        for k in 0..=65535u32 {
            let on_a_half = k as f32 + 0.5;
            let anywhere = (next() >> 40) as f32 / 256.0;
            for top in [255.0f32, 65535.0] {
                // Every sample a surface can hold, scaled as `load` scales it.
                let (eight, sixteen) = ((k % 256) as f32 / 255.0 * top, k as f32 / 65535.0 * top);
                for v in [eight, sixteen, on_a_half, on_a_half.next_down(), on_a_half.next_up(), anywhere] {
                    assert_eq!(whole(v), v.round(), "{v}");
                }
            }
        }
    }

    /// A surface of `fmt` with noise in `r`.
    fn noisy(fmt: photocraft_color::PixelFormat, r: Rect, seed: u64) -> Surface {
        let mut next = noise(seed);
        let mut s = Surface::new(fmt);
        let data: Vec<f32> = (0..r.width() as usize * r.height() as usize * fmt.channels()).map(|_| (next() >> 40) as f32 / (1u64 << 24) as f32).collect();
        s.write_region(r, &data);
        s
    }

    const FORMATS: [photocraft_color::PixelFormat; 4] = {
        use photocraft_color::PixelFormat;
        [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F, PixelFormat::GRAYA8]
    };

    /// `load` reads bands of rows on all cores; the image is the one a single read gives.
    #[test]
    fn loading_gives_the_levels_of_one_whole_read() {
        for fmt in FORMATS {
            let s = noisy(fmt, Rect::new(-40, -30, 300, 310), 4);
            // Past the content on every side, and a height no band size divides.
            let bounds = Rect::new(-57, -45, 280, 326);
            let depth = Depth::of(fmt.sample);
            let mut want = s.read_region(bounds);
            if let Some(unit) = depth.levels {
                want.iter_mut().for_each(|v| *v = (v.clamp(0.0, 1.0) * unit).round());
            }
            let got = load(&s, bounds, depth);
            assert_eq!((got.w, got.h, got.n), (337, 371, fmt.channels()));
            assert!(got.px == want, "{fmt:?}");
        }
    }

    /// `store` converts rows straight into tiles; the surface is the one the whole image written
    /// at once gives, cleared or not.
    #[test]
    fn storing_rows_gives_the_surface_of_one_whole_write() {
        for fmt in FORMATS {
            let depth = Depth::of(fmt.sample);
            let n = fmt.channels();
            let rect = Rect::new(-300, -70, 281, 300);
            let (w, h) = (rect.width() as usize, rect.height() as usize);
            // Each pixel from its place alone: whole levels (any value at 32 bits, alpha below
            // zero too), every third pixel transparent with its colour left in.
            let fill = |y: usize, x: usize, out: &mut [f32]| {
                for (i, p) in out.chunks_exact_mut(n).enumerate() {
                    let mut next = noise((y * w + x + i) as u64);
                    for v in p.iter_mut() {
                        let part = (next() >> 40) as f32 / (1u64 << 24) as f32;
                        *v = depth.levels.map_or(part * 2.0 - 0.5, |top| (part * top).round());
                    }
                    if (y + x + i).is_multiple_of(3) {
                        p[n - 1] = 0.0;
                    }
                }
            };
            for clear in [false, true] {
                let mut data = vec![0.0f32; w * h * n];
                for (y, row) in data.chunks_exact_mut(w * n).enumerate() {
                    fill(y, 0, row);
                }
                let unit = depth.levels.unwrap_or(1.0);
                data.iter_mut().for_each(|v| *v /= unit);
                if clear {
                    clear_hidden(&mut data, &fmt);
                }
                let mut want = Surface::new(fmt);
                want.write_region_par(rect, &data);
                let got = store(fmt, rect, depth, clear, fill).expect("stored");
                assert!(got.read_region(rect) == want.read_region(rect), "{fmt:?}, clear {clear}");
                assert_eq!(got.content_bounds(), want.content_bounds());
                assert_eq!(got.tile_count(), want.tile_count());
            }
        }
    }

    /// A scale is the same picture whichever way it is put together: here against the passes
    /// run one after the other on a whole image, as they first were.
    #[test]
    fn a_scale_gives_the_passes_run_one_by_one() {
        for fmt in FORMATS {
            let bounds = Rect::new(-5, 3, 85, 64);
            let src = noisy(fmt, Rect::new(0, 10, 80, 60), 5);
            let depth = Depth::of(fmt.sample);
            for (width, height) in [(90, 61), (135, 61), (90, 20), (33, 150), (211, 97), (7, 5), (1, 1)] {
                let mut img = load(&src, bounds, depth);
                bleed(&mut img);
                if width != img.w {
                    img = axis(&img, width, depth, false);
                }
                if height != img.h {
                    img = axis(&img, height, depth, true);
                }
                let unit = depth.levels.unwrap_or(1.0);
                let mut data: Vec<f32> = img.px.iter().map(|v| v / unit).collect();
                clear_hidden(&mut data, &fmt);
                let rect = Rect::new(-9, 40, -9 + width as i32, 40 + height as i32);
                let mut want = Surface::new(fmt);
                want.write_region_par(rect, &data);
                let got = scale(&src, bounds, width as u32, height as u32, (-9, 40)).expect("scaled");
                assert!(got.read_region(rect) == want.read_region(rect), "{fmt:?} to {width} x {height}");
                assert_eq!(got.content_bounds(), want.content_bounds());
            }
        }
    }
}
