//! A layer scaled along its axes with the Image Size bicubics, as the reference application draws a
//! pixel layer under Free Transform with Bicubic Smoother, Bicubic Sharper or Bicubic Automatic.
//! Measured byte for byte at 8 bits against its renders (scales from 37 % to 250 %, each axis on
//! its own and both, smooth gradients and hard-edged cells: 0 pixels apart in every case):
//!
//! 1. **The box.** The result covers the whole-pixel box whose edges are the transformed edges,
//!    each rounded where it falls.
//! 2. **Horizontal pass, then vertical**, each axis with its filter: Smoother, Sharper, or for
//!    Automatic Sharper where the axis shrinks and Smoother where it grows. An axis that keeps its
//!    size in whole pixels is copied.
//! 3. **Each pass:** output sample `o` reads the source where the exact (unrounded) placement puts
//!    it, `(o + ½ − phase)/k − ½`, clamped to the first and last sample and rounded to 1/128 pixel.
//!    The kernel is Image Size's Keys cubic for the filter and scale ([`Filter::kernel`]); its
//!    weights are 14-bit integers: each normalised weight times 16384, truncated toward zero,
//!    except the largest, which takes what the others leave of 16384. Colour is filtered straight
//!    (not premultiplied) with the edge pixels repeated outwards; alpha is filtered on its own,
//!    reading transparency beyond the edges. Each result is rounded, halves up, and clipped to the
//!    sample range.
//! 4. **Below ¼ along an axis**, the image is first averaged down to four times the target (each
//!    sample the plain mean of the samples it covers, rounded), then filtered as at ¼.
//!
//! At 8 and 16 bits the arithmetic is exact in whole sample levels; at 32 bits the same steps run
//! unrounded.

use photocraft_color::SampleType;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};

/// The bicubic an axis is filtered with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    /// Bicubic Smoother.
    Smoother,
    /// Bicubic Sharper.
    Sharper,
    /// Bicubic Automatic: Sharper where the axis shrinks, Smoother where it grows.
    Automatic,
}

impl Filter {
    /// The Keys parameter and stretch (in source samples) for scale `k`: Image Size's filters,
    /// widened by 1/k when reducing, down to ¼ (below that the axis is averaged first).
    fn kernel(self, k: f64) -> (f64, f64) {
        let filter = match self {
            Filter::Automatic if k < 1.0 => Filter::Sharper,
            Filter::Automatic => Filter::Smoother,
            f => f,
        };
        let s = k.clamp(0.25, 1.0);
        let (a, stretch) = match filter {
            Filter::Sharper => (-1.0 - 1.6 * (1.0 - s), 1.05),
            _ => (-0.625 - 0.5 * (1.0 - s), 1.15),
        };
        (a, stretch / s)
    }
}

/// The pixels of `src` inside `bounds`, scaled by `sx` × `sy` (both positive) and moved so that
/// `bounds`' top-left corner lands at `(x0, y0)`, with `filter` on each axis (the module's rules).
/// `None` when the source has no alpha, the result would be empty or larger than
/// [`MAX_SAMPLES`], the placement isn't finite or `ctl` is cancelled: the caller draws those another way (and
/// reports a cancel).
pub fn scale(src: &Surface, bounds: Rect, place: [f64; 4], filter: Filter, ctl: &Interrupt) -> Option<Surface> {
    scale_in_bands(src, bounds, place, filter, BAND_SAMPLES, ctl)
}

/// [`scale`], writing at most `band_samples` samples of output rows at a time.
fn scale_in_bands(src: &Surface, bounds: Rect, [sx, sy, x0, y0]: [f64; 4], filter: Filter, band_samples: usize, ctl: &Interrupt) -> Option<Surface> {
    if ![sx, sy, x0, y0].iter().all(|v| v.is_finite()) || sx <= 0.0 || sy <= 0.0 {
        return None;
    }
    let fmt = src.format();
    let n = src.channels();
    if !fmt.alpha || n == 0 || bounds.is_empty() {
        return None;
    }
    // Rule 1: each edge rounded where it falls.
    let edge = |v: f64| -> Option<i32> {
        let r = (v + 0.5).floor();
        (r.abs() < f64::from(i32::MAX / 2)).then_some(r as i32)
    };
    let (bw, bh) = (f64::from(bounds.width()), f64::from(bounds.height()));
    let rect = Rect::new(edge(x0)?, edge(y0)?, edge(x0 + sx * bw)?, edge(y0 + sy * bh)?);
    if rect.is_empty() {
        return None;
    }
    let (w, h) = (bounds.width() as usize, bounds.height() as usize);
    let (nx, ny) = (rect.width() as usize, rect.height() as usize);
    let biggest = w.max(nx).checked_mul(h.max(ny))?.checked_mul(n)?;
    if biggest > MAX_SAMPLES {
        return None;
    }
    let along_x = Exact { scale: sx, phase: x0 - f64::from(rect.x0) };
    let along_y = Exact { scale: sy, phase: y0 - f64::from(rect.y0) };
    let depth = Depth::of(fmt.sample);
    // Source rows `[r0, r1)` (counted from `bounds`' top), loaded and filtered along x.
    let rows = |r0: usize, r1: usize| -> Option<Image> {
        let top = bounds.y0.checked_add(i32::try_from(r0).ok()?)?;
        let bottom = bounds.y0.checked_add(i32::try_from(r1).ok()?)?;
        let img = load(src, Rect::new(bounds.x0, top, bounds.x1, bottom), depth)?;
        if nx != w { axis(&img, nx, depth, false, along_x, filter) } else { Some(img) }
    };
    let mut out = Surface::new(fmt);
    let band = (band_samples / nx.saturating_mul(n).max(1)).clamp(1, ny);
    if ny == h {
        // Rows copied: each band reads its own rows.
        for y0 in (0..ny).step_by(band) {
            let y1 = (y0 + band).min(ny);
            if ctl.cancelled() {
                return None;
            }
            let img = rows(y0, y1)?;
            write_band(&mut out, rect, y0, y1, depth, |y, x, px| {
                if let Some(line) = img.px.get(((y - y0) * img.w + x) * n..).and_then(|p| p.get(..px.len())) {
                    px.copy_from_slice(line);
                }
            })?;
        }
        return Some(out);
    }
    if (ny as f64) < 0.25 * h as f64 {
        // Rows averaged first (rule 4): the whole source, once; the result is small.
        if ctl.cancelled() {
            return None;
        }
        let img = rows(0, h)?;
        let (mid, a, stretch) = prepared(&img, ny, depth, true, filter)?;
        let along_y = along_y.read(h, mid.as_ref().map_or(h, |m| m.h));
        let img = mid.unwrap_or(img);
        let taps = tap_table(ny, img.h, a, stretch, along_y)?;
        for y0 in (0..ny).step_by(band) {
            let y1 = (y0 + band).min(ny);
            if ctl.cancelled() {
                return None;
            }
            write_band(&mut out, rect, y0, y1, depth, |y, x, px| {
                if let Some(t) = taps.get(y) {
                    blend_rows(&img, t, 0, img.h, x, depth, px);
                }
            })?;
        }
        return Some(out);
    }
    // Each band of output rows loads and filters along x only the source rows its taps read.
    let (a, stretch) = filter.kernel(ny as f64 / h as f64);
    let taps = tap_table(ny, h, a, stretch, along_y)?;
    let last = h.saturating_sub(1) as i64;
    for y0 in (0..ny).step_by(band) {
        let y1 = (y0 + band).min(ny);
        let reach = taps.get(y0..y1).unwrap_or(&[]);
        let r0 = reach.iter().map(|t| t.start.clamp(0, last)).min().unwrap_or(0) as usize;
        let r1 = reach.iter().map(|t| (t.start + t.q.len() as i64 - 1).clamp(0, last)).max().unwrap_or(0) as usize + 1;
        if ctl.cancelled() {
            return None;
        }
        let img = rows(r0, r1)?;
        write_band(&mut out, rect, y0, y1, depth, |y, x, px| {
            if let Some(t) = taps.get(y) {
                blend_rows(&img, t, r0, h, x, depth, px);
            }
        })?;
    }
    Some(out)
}

/// One axis resized to `len` samples (`vertical`: the rows) with `filter`, through the averaged
/// image below ¼. `None` if its pixels can't be allocated.
fn axis(img: &Image, len: usize, depth: Depth, vertical: bool, exact: Exact, filter: Filter) -> Option<Image> {
    let (mid, a, stretch) = prepared(img, len, depth, vertical, filter)?;
    let have = |i: &Image| if vertical { i.h } else { i.w };
    let exact = exact.read(have(img), mid.as_ref().map_or(have(img), have));
    pass(mid.as_ref().unwrap_or(img), len, a, stretch, depth, vertical, exact)
}

/// What an axis's filtering pass to `len` samples works with: the averaged image it reads (below
/// ¼; otherwise it reads `img` itself), then the Keys parameter and stretch for its scale. `None` if the averaged
/// image can't be allocated.
fn prepared(img: &Image, len: usize, depth: Depth, vertical: bool, filter: Filter) -> Option<(Option<Image>, f64, f64)> {
    let have = if vertical { img.h } else { img.w };
    let k = len as f64 / have.max(1) as f64;
    if k < 0.25 {
        let (a, stretch) = filter.kernel(0.25);
        Some((Some(average(img, len.checked_mul(4)?, depth, vertical)?), a, stretch))
    } else {
        let (a, stretch) = filter.kernel(k);
        Some((None, a, stretch))
    }
}

/// `len` zero samples, or `None` if they can't be allocated (a large layer must not abort the app).
fn zeros(len: usize) -> Option<Vec<f32>> {
    let mut px = Vec::new();
    px.try_reserve_exact(len).ok()?;
    px.resize(len, 0.0);
    Some(px)
}

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

/// Rows read per task by [`load`].
const LOAD_ROWS: usize = 32;

/// The pixels of `src` inside `bounds` as an [`Image`]: whole levels at 8 and 16 bits, the stored
/// values at 32. Bands of rows are read on all cores.
pub(crate) fn load(src: &Surface, bounds: Rect, depth: Depth) -> Option<Image> {
    /// Rows read at a time.
    const READ_ROWS: usize = 4;
    let (w, h, n) = (bounds.width() as usize, bounds.height() as usize, src.channels());
    let mut px = zeros(w.checked_mul(h)?.checked_mul(n)?)?;
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
    Some(Image { w, h, n, px })
}

/// Output samples one band of rows covers: each band loads and filters along x only the source
/// rows its own rows read, so the image between the passes stays about this size (64 MB of `f32`)
/// whatever the result's.
const BAND_SAMPLES: usize = 1 << 24;

/// Writes the output rows `[y0, y1)` of `rect` into `out`, straight into its tiles on all cores:
/// `fill(y, x, px)` computes the pixels of row `y` from column `x` on (both counted from `rect`'s
/// corner) as an [`Image`]'s samples, zero to start with. Fully transparent pixels lose their
/// colour. `None` if the tiles can't be allocated.
fn write_band(out: &mut Surface, rect: Rect, y0: usize, y1: usize, depth: Depth, fill: impl Fn(usize, usize, &mut [f32]) + Sync) -> Option<()> {
    let fmt = out.format();
    let n = fmt.channels();
    let size = fmt.sample.bytes();
    let per_level = 1.0 / depth.levels.unwrap_or(1.0);
    let top = rect.y0.checked_add(i32::try_from(y0).ok()?)?;
    let bottom = rect.y0.checked_add(i32::try_from(y1).ok()?)?;
    let rows = |y: i32, x: i32, dst: &mut [u8]| {
        let (Some(row), Some(col)) = (y.checked_sub(rect.y0), x.checked_sub(rect.x0)) else { return };
        let (Ok(row), Ok(col)) = (usize::try_from(row), usize::try_from(col)) else { return };
        let mut px = vec![0.0f32; dst.len() / size.max(1)];
        fill(row, col, &mut px);
        for (p, d) in px.chunks_exact_mut(n).zip(dst.chunks_exact_mut(n * size)) {
            if p.last().is_some_and(|al| *al <= 0.0) {
                p.fill(0.0);
            }
            for (i, v) in p.iter().enumerate() {
                photocraft_color::write_sample(d, fmt.sample, i, *v * per_level);
            }
        }
    };
    out.try_write_rows_with(Rect::new(rect.x0, top, rect.x1, bottom), rows).ok()
}

/// Where an axis's output samples read: the exact scale (source → document) and how far the exact
/// placement lies past the whole-pixel box (in output samples).
#[derive(Clone, Copy, Debug)]
struct Exact {
    scale: f64,
    phase: f64,
}

impl Exact {
    /// The same placement over an image averaged from `was` samples down to `now` (below ¼).
    fn read(self, was: usize, now: usize) -> Exact {
        Exact { scale: self.scale * was as f64 / now.max(1) as f64, phase: self.phase }
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

/// Output sample `o`'s taps over `have` source samples: it reads the source at `(o + ½ − phase)/scale − ½` (the exact
/// placement; the filter for the snapped scale sets only the kernel, `a` and `stretch`).
fn taps(o: usize, have: usize, a: f64, stretch: f64, exact: Exact) -> Option<Taps> {
    let last = have.saturating_sub(1) as f64;
    let u = ((o as f64 + 0.5 - exact.phase) / exact.scale - 0.5).clamp(0.0, last);
    let j = u.floor();
    let u = j + ((u - j) * 128.0).round_ties_even() / 128.0;
    let (lo, hi) = ((u - 2.0 * stretch).floor() as i64, (u + 2.0 * stretch).ceil() as i64);
    let n = usize::try_from(hi.checked_sub(lo)?.checked_add(1)?).ok()?;
    let mut w: Vec<f64> = Vec::new();
    w.try_reserve_exact(n).ok()?;
    w.extend((lo..=hi).map(|i| keys((u - i as f64) / stretch, a)));
    let total: f64 = w.iter().sum();
    let mut q: Vec<i64> = Vec::new();
    q.try_reserve_exact(n).ok()?;
    q.extend(w.iter().map(|v| (v / total * ONE as f64).trunc() as i64));
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
    Some(Taps { start: lo, q })
}

/// The taps of `len` output samples over `have` source samples ([`taps`]), or `None` if the table can't be allocated:
/// an extreme enlargement (a 1 px source to a million) builds a table larger than its pixels, and an infallible
/// allocation of it aborted the app instead of falling back.
fn tap_table(len: usize, have: usize, a: f64, stretch: f64, exact: Exact) -> Option<Vec<Taps>> {
    let mut table = Vec::new();
    table.try_reserve_exact(len).ok()?;
    for o in 0..len {
        table.push(taps(o, have, a, stretch, exact)?);
    }
    Some(table)
}

/// One filtering pass along an axis to `len` samples at the placement `exact` (rule 3 of the
/// module).
fn pass(img: &Image, len: usize, a: f64, stretch: f64, depth: Depth, vertical: bool, exact: Exact) -> Option<Image> {
    let have = if vertical { img.h } else { img.w };
    let n = img.n;
    let taps = tap_table(len, have, a, stretch, exact)?;
    let (ow, oh) = if vertical { (img.w, len) } else { (len, img.h) };
    let mut px = zeros(ow.checked_mul(oh)?.checked_mul(n)?)?;
    crate::par::chunks_mut(&mut px, ow * n, true, |y, row| {
        if vertical {
            if let Some(t) = taps.get(y) {
                blend_rows(img, t, 0, img.h, 0, depth, row);
            }
        } else if let Some(line) = img.px.get(y * img.w * n..(y + 1) * img.w * n) {
            blend_line(line, n, &taps, depth, row);
        }
    });
    Some(Image { w: ow, h: oh, n, px })
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

/// Part of one row of a vertical pass: `out`'s pixels, from column `x` on, are the taps `t` over the
/// rows of `img`, which holds the source rows from `first` on of `total`. Colour repeats the edge outwards; alpha reads
/// transparency there.
fn blend_rows(img: &Image, t: &Taps, first: usize, total: usize, x: usize, depth: Depth, out: &mut [f32]) {
    let n = img.n;
    let alpha = n.saturating_sub(1);
    let last = total as i64 - 1;
    let mut acc = vec![0.0f64; out.len()];
    for (step, &q) in t.q.iter().enumerate() {
        let i = t.start + step as i64;
        let Some(local) = (i.clamp(0, last.max(0)) as usize).checked_sub(first) else { continue };
        let at = (local * img.w + x) * n;
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

/// The source samples `[a, b)` output sample `o` of `len` averages over `have` (at least one). The products are
/// taken in 64 bits: on 32-bit targets `(o + 1) * have` overflows `usize` for long axes (100,000 samples to 80,000).
fn span(o: usize, have: usize, len: usize) -> (usize, usize) {
    let (o, have64, len) = (o as u64, have as u64, (len as u64).max(1));
    let a = o * have64 / len;
    let b = ((o + 1) * have64 / len).max(a + 1).min(have64);
    (usize::try_from(a).unwrap_or(have), usize::try_from(b).unwrap_or(have))
}

/// An axis averaged down to `len` samples: each the rounded mean of the samples it covers
/// (rule 4 of the module).
fn average(img: &Image, len: usize, depth: Depth, vertical: bool) -> Option<Image> {
    let have = if vertical { img.h } else { img.w };
    let n = img.n;
    let mut spans: Vec<(usize, usize)> = Vec::new();
    spans.try_reserve_exact(len).ok()?;
    spans.extend((0..len).map(|o| span(o, have, len)));
    let (ow, oh) = if vertical { (img.w, len) } else { (len, img.h) };
    let mut px = zeros(ow.checked_mul(oh)?.checked_mul(n)?)?;
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
    Some(Image { w: ow, h: oh, n, px })
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;

    /// A `w` × `h` layer at (`x0`, `y0`) filled with `rgba` (0..1), on an empty surface of `fmt`.
    fn layer(fmt: PixelFormat, x0: i32, y0: i32, w: i32, h: i32, rgba: [f32; 4]) -> Surface {
        let mut s = Surface::new(fmt);
        s.fill_rect(Rect::new(x0, y0, x0 + w, y0 + h), &rgba);
        s
    }

    /// A horizontal ramp, so resampling changes the values.
    fn ramp(fmt: PixelFormat, w: i32, h: i32) -> Surface {
        let mut s = Surface::new(fmt);
        for x in 0..w {
            let v = x as f32 / (w - 1) as f32;
            s.fill_rect(Rect::new(x, 0, x + 1, h), &[v, 1.0 - v, 0.5, 1.0]);
        }
        s
    }

    fn alpha8(s: &Surface, x: i32, y: i32) -> u8 {
        (s.pixel(x, y)[3] * 255.0).round() as u8
    }

    /// A pass to more samples than can be allocated returns `None`: its tap table (larger than the pixels it fills)
    /// and an averaged axis' spans are allocated fallibly, so the caller falls back instead of the app aborting (a
    /// 1 px source enlarged a millionfold built a 32 MB table before the fallible pixel buffer was asked for).
    #[test]
    fn a_pass_too_long_to_allocate_returns_none() {
        let img = Image { w: 1, h: 1, n: 4, px: vec![0.5; 4] };
        let exact = Exact { scale: 1e9, phase: 0.0 };
        let huge = usize::MAX / 64;
        assert!(pass(&img, huge, 1.0, 1.0, Depth::of(PixelFormat::RGBA8.sample), false, exact).is_none());
        assert!(pass(&img, huge, 1.0, 1.0, Depth::of(PixelFormat::RGBA8.sample), true, exact).is_none());
        assert!(average(&img, huge, Depth::of(PixelFormat::RGBA8.sample), false).is_none());
        assert!(tap_table(huge, 1, 1.0, 1.0, exact).is_none());
    }

    #[test]
    fn each_edge_of_the_box_is_rounded_where_it_falls() {
        // 128 px scaled by 1.73 about x = 160: from 49.28 to 270.72, so 49..271.
        let src = layer(PixelFormat::RGBA8, 96, 96, 128, 128, [0.2, 0.4, 0.6, 1.0]);
        let k = 1.73;
        let at = 160.0 - 64.0 * k;
        let out = scale(&src, Rect::new(96, 96, 224, 224), [k, k, at, at], Filter::Smoother, &Interrupt::NONE).unwrap();
        assert_eq!(out.content_bounds(), Rect::new(49, 49, 271, 271));
    }

    #[test]
    fn the_outline_fades_as_the_reference_draws_it() {
        // Measured on the reference application's Free Transform of an opaque pixel layer: the
        // outermost pixel's alpha is 238 with Smoother when enlarging (173 % and 250 % alike) and
        // 242 with Sharper at 50 %; the next one in is opaque (252 at 250 %, where it is still
        // within reach).
        let src = layer(PixelFormat::RGBA8, 96, 96, 128, 128, [0.2, 0.4, 0.6, 1.0]);
        let bounds = Rect::new(96, 96, 224, 224);
        for (k, filter, first) in [(1.73, Filter::Smoother, 238), (2.5, Filter::Smoother, 238), (0.5, Filter::Sharper, 242)] {
            let at = 160.0 - 64.0 * k;
            let out = scale(&src, bounds, [k, k, at, at], filter, &Interrupt::NONE).unwrap();
            let b = out.content_bounds();
            let mid = (b.y0 + b.y1) / 2;
            assert_eq!(alpha8(&out, b.x0, mid), first, "{k} {filter:?} left");
            assert_eq!(alpha8(&out, b.x1 - 1, mid), first, "{k} {filter:?} right");
            assert_eq!(alpha8(&out, b.x0 + 3, mid), 255, "{k} {filter:?} inside");
            // Colour is filtered straight: a flat layer keeps its colour to the edge.
            let p = out.pixel(b.x0, mid);
            assert!((p[0] - 0.2).abs() < 0.003 && (p[2] - 0.6).abs() < 0.003, "{k} {filter:?} {p:?}");
        }
    }

    #[test]
    fn automatic_is_sharper_shrinking_and_smoother_growing() {
        let src = ramp(PixelFormat::RGBA8, 64, 64);
        let bounds = Rect::new(0, 0, 64, 64);
        for (k, same) in [(0.6, Filter::Sharper), (1.4, Filter::Smoother)] {
            let place = [k, k, 3.25, 1.5];
            let auto = scale(&src, bounds, place, Filter::Automatic, &Interrupt::NONE).unwrap();
            let other = scale(&src, bounds, place, same, &Interrupt::NONE).unwrap();
            let b = auto.content_bounds();
            assert_eq!(auto.read_region(b), other.read_region(b), "{k}");
            let third = scale(&src, bounds, place, if same == Filter::Sharper { Filter::Smoother } else { Filter::Sharper }, &Interrupt::NONE).unwrap();
            assert_ne!(auto.read_region(b), third.read_region(b), "{k}: the filters differ");
        }
    }

    #[test]
    fn an_axis_that_keeps_its_size_is_copied() {
        let src = ramp(PixelFormat::RGBA8, 40, 10);
        let out = scale(&src, Rect::new(0, 0, 40, 10), [1.0, 2.0, 5.0, 0.0], Filter::Smoother, &Interrupt::NONE).unwrap();
        assert_eq!(out.content_bounds(), Rect::new(5, 0, 45, 20));
        for x in 0..40 {
            let (a, b) = (src.pixel(x, 0), out.pixel(x + 5, 10));
            assert!((a[0] - b[0]).abs() < 1e-6, "{x}: {a:?} {b:?}");
        }
    }

    #[test]
    fn every_depth_draws_the_same_picture() {
        let bounds = Rect::new(0, 0, 48, 48);
        let place = [0.37, 1.7, 2.6, 4.1];
        let reference = scale(&ramp(PixelFormat::RGBA8, 48, 48), bounds, place, Filter::Automatic, &Interrupt::NONE).unwrap();
        let b = reference.content_bounds();
        for fmt in [PixelFormat::RGBA16, PixelFormat::RGBA32F] {
            let out = scale(&ramp(fmt, 48, 48), bounds, place, Filter::Automatic, &Interrupt::NONE).unwrap();
            assert_eq!(out.content_bounds(), b, "{fmt:?}");
            for y in b.y0..b.y1 {
                for x in b.x0..b.x1 {
                    let (p, q) = (reference.pixel(x, y), out.pixel(x, y));
                    assert!(p.iter().zip(&q).all(|(a, c)| (a - c).abs() < 1.5 / 255.0), "{fmt:?} ({x}, {y}): {p:?} {q:?}");
                }
            }
        }
    }

    #[test]
    fn far_reductions_average_first() {
        // Below ¼ the axis is averaged to four times the target first: a flat layer stays flat.
        let src = layer(PixelFormat::RGBA8, 0, 0, 400, 40, [0.5, 0.25, 0.75, 1.0]);
        let out = scale(&src, Rect::new(0, 0, 400, 40), [0.05, 1.0, 0.0, 0.0], Filter::Sharper, &Interrupt::NONE).unwrap();
        assert_eq!(out.content_bounds(), Rect::new(0, 0, 20, 40));
        let p = out.pixel(10, 20);
        assert!((p[0] - 0.5).abs() < 0.003 && (p[3] - 1.0).abs() < 1e-6, "{p:?}");
    }

    #[test]
    fn bands_of_rows_draw_the_picture_drawn_at_once() {
        let src = ramp(PixelFormat::RGBA8, 90, 70);
        let bounds = Rect::new(0, 0, 90, 70);
        // Rows filtered (grown and shrunk), copied, and averaged first.
        for place in [[1.3, 1.9, 2.4, 0.7], [0.8, 0.45, 1.1, 3.3], [1.6, 1.0, 0.5, 4.0], [1.2, 0.1, 0.0, 0.25]] {
            let whole = scale_in_bands(&src, bounds, place, Filter::Automatic, usize::MAX, &Interrupt::NONE).unwrap();
            let b = whole.content_bounds();
            for band in [1, 600, 2000] {
                let banded = scale_in_bands(&src, bounds, place, Filter::Automatic, band, &Interrupt::NONE).unwrap();
                assert_eq!(banded.content_bounds(), b, "{place:?} {band}");
                assert_eq!(banded.read_region(b), whole.read_region(b), "{place:?} {band}");
            }
        }
    }

    #[test]
    fn long_axes_average_over_the_right_spans_without_overflow() {
        // 100,000 samples to 80,000: (o + 1) * have passes 2^32 from o = 42,949 on, which a 32-bit usize can't hold.
        let (have, len) = (100_000usize, 80_000usize);
        for o in [0, 1, 42_948, 42_949, 42_950, 79_999] {
            let want = ((o as u128 * have as u128 / len as u128) as usize, ((o as u128 + 1) * have as u128 / len as u128) as usize);
            let (a, b) = span(o, have, len);
            assert_eq!((a, b), (want.0, want.1.max(want.0 + 1).min(have)), "{o}");
        }
        assert_eq!(span(3, 2, 8), (0, 1), "every output averages at least one sample");
    }

    #[test]
    fn a_cancel_stops_the_scale() {
        let src = ramp(PixelFormat::RGBA8, 64, 64);
        let always = || true;
        for place in [[1.6, 1.3, 0.0, 0.0], [1.0, 1.6, 0.0, 0.0], [0.5, 0.1, 0.0, 0.0]] {
            assert!(scale(&src, Rect::new(0, 0, 64, 64), place, Filter::Automatic, &Interrupt::cancel_only(&always)).is_none(), "{place:?}");
        }
    }

    #[test]
    fn unusable_input_is_refused_not_drawn() {
        let src = layer(PixelFormat::RGBA8, 0, 0, 8, 8, [1.0; 4]);
        let bounds = Rect::new(0, 0, 8, 8);
        for place in [[f64::NAN, 1.0, 0.0, 0.0], [1.0, f64::INFINITY, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [-1.0, 1.0, 0.0, 0.0], [1.0, 1.0, 1e300, 0.0]] {
            assert!(scale(&src, bounds, place, Filter::Smoother, &Interrupt::NONE).is_none(), "{place:?}");
        }
        // A box squeezed below a pixel, a result past the sample cap, no alpha, no pixels.
        assert!(scale(&src, bounds, [0.01, 1.0, 0.2, 0.0], Filter::Smoother, &Interrupt::NONE).is_none());
        assert!(scale(&src, bounds, [1e5, 1e5, 0.0, 0.0], Filter::Smoother, &Interrupt::NONE).is_none());
        let mut gray = Surface::new(PixelFormat::GRAY8);
        gray.fill_rect(bounds, &[0.5]);
        assert!(scale(&gray, bounds, [2.0, 2.0, 0.0, 0.0], Filter::Smoother, &Interrupt::NONE).is_none());
        assert!(scale(&src, Rect::EMPTY, [2.0, 2.0, 0.0, 0.0], Filter::Smoother, &Interrupt::NONE).is_none());
    }
}
