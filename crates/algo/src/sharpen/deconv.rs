//! Richardson–Lucy deconvolution for Smart Sharpen's Lens Blur and Motion Blur removal.
//!
//! The blur is modelled as a small point-spread function (a disc for a lens that is out of focus,
//! a line for camera or subject motion) and undone iteratively, so detail comes back without the
//! bright and dark halos an unsharp mask leaves around edges. Colour is deconvolved premultiplied,
//! one channel at a time; alpha is untouched.
//!
//! Every iteration blurs and correlates with the point-spread function by FFT, so the cost doesn't
//! depend on the radius (Photoshop's range is 0.1–64 px). A pixel depends on the input up to
//! `2 × iterations × kernel radius` away; the filter's halo is that, up to [`MAX_APRON`], and the
//! layer is processed in blocks of at most [`MAX_SIDE`] px with that apron, so memory stays bounded
//! on large layers. Within the apron a tile agrees with a whole-layer run.

use photocraft_geom::Rect;
use rustfft::num_complex::Complex;
#[cfg(not(target_arch = "wasm32"))]
use rustfft::{Fft, FftPlanner};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;

use crate::image::{Image, premultiply, unpremultiply};
use crate::photo_util::par_rows;
use crate::sharpen::smallfft::SmallFft;
use crate::{Ctx, SharpenRemove};

/// Largest point-spread radius in pixels (Smart Sharpen's own range).
const MAX_RADIUS: f32 = 64.0;
/// Largest apron read around a block. A radius-64 "More Accurate" run would want 3,000 px; the
/// influence of a pixel that far away is negligible.
const MAX_APRON: i32 = 768;
/// Largest transform side, bounding a block's working memory to about 190 MB (120 MB in the
/// browser, which has less to spare).
const MAX_SIDE: usize = if cfg!(target_arch = "wasm32") { 2304 } else { 3072 };
/// Floor of the blurred estimate, so dark areas don't divide by zero.
const EPS: f32 = 1e-5;
/// Largest correction one iteration may apply to a pixel (keeps 32-bit highlights stable).
const MAX_RATIO: f32 = 8.0;

type C = Complex<f32>;

/// One non-zero entry of the point-spread function: offset and weight.
pub(crate) type Tap = (i32, i32, f32);

/// Iteration count: "More Accurate" runs three times as many.
pub(crate) fn iterations(more_accurate: bool) -> usize {
    if more_accurate { 24 } else { 8 }
}

fn clamp_radius(radius: f32) -> f32 {
    if radius.is_finite() { radius.clamp(0.1, MAX_RADIUS) } else { 0.1 }
}

/// Pixels read outside an output tile.
pub(crate) fn reach(radius: f32, more_accurate: bool) -> i32 {
    let r = (clamp_radius(radius) + 1.0).ceil() as i32;
    (2 * iterations(more_accurate) as i32 * r + 1).min(MAX_APRON)
}

/// The point-spread function, normalised to sum 1. `Gaussian` has no kernel here (Smart Sharpen
/// keeps its unsharp path for it).
pub(crate) fn kernel(remove: SharpenRemove, radius: f32, angle: f32) -> Vec<Tap> {
    let r = clamp_radius(radius);
    let mut taps = match remove {
        SharpenRemove::Gaussian | SharpenRemove::Lens => disc(r),
        SharpenRemove::Motion => line(r, if angle.is_finite() { angle } else { 0.0 }),
    };
    let sum: f32 = taps.iter().map(|t| t.2).sum();
    if sum > 0.0 {
        for t in &mut taps {
            t.2 /= sum;
        }
    } else {
        taps = vec![(0, 0, 1.0)];
    }
    taps
}

/// A disc of radius `r` with an anti-aliased rim (the shape of an out-of-focus point of light).
fn disc(r: f32) -> Vec<Tap> {
    let edge = (r + 0.5).ceil() as i32;
    let mut taps = Vec::new();
    for dy in -edge..=edge {
        for dx in -edge..=edge {
            let d = ((dx * dx + dy * dy) as f32).sqrt();
            let w = (r + 0.5 - d).clamp(0.0, 1.0);
            if w > 0.0 {
                taps.push((dx, dy, w));
            }
        }
    }
    taps
}

/// A line `2r` long through the origin at `angle` degrees (counter-clockwise from horizontal),
/// each sample spread over its four neighbouring pixels.
fn line(r: f32, angle: f32) -> Vec<Tap> {
    let (sin, cos) = angle.to_radians().sin_cos();
    let side = 2 * (r.ceil() as i32 + 1) + 1;
    let half = side / 2;
    let mut grid = vec![0.0f32; (side * side) as usize];
    let steps = ((2.0 * r) * 4.0).ceil().max(1.0) as i32;
    for i in 0..=steps {
        let t = -r + 2.0 * r * i as f32 / steps as f32;
        let (x, y) = (t * cos, -t * sin);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        for (ox, oy, w) in [(0, 0, (1.0 - fx) * (1.0 - fy)), (1, 0, fx * (1.0 - fy)), (0, 1, (1.0 - fx) * fy), (1, 1, fx * fy)] {
            let (gx, gy) = (x0 as i32 + ox + half, y0 as i32 + oy + half);
            if (0..side).contains(&gx)
                && (0..side).contains(&gy)
                && let Some(cell) = grid.get_mut((gy * side + gx) as usize)
            {
                *cell += w;
            }
        }
    }
    let mut taps = Vec::new();
    for gy in 0..side {
        for gx in 0..side {
            let w = grid.get((gy * side + gx) as usize).copied().unwrap_or(0.0);
            if w > 0.0 {
                taps.push((gx - half, gy - half, w));
            }
        }
    }
    taps
}

/// Smallest transform length that is at least `n` with only the factors 2, 3 and 5.
fn fft_len(n: usize) -> usize {
    let mut best = n.next_power_of_two().max(2);
    let mut a = 1usize;
    while a < best {
        let mut b = a;
        while b < best {
            let mut c = b;
            while c < best {
                if c >= n {
                    best = c;
                }
                c *= 5;
            }
            b *= 3;
        }
        a *= 2;
    }
    best.max(2)
}

/// `dst[c * rows + r] = src[r * cols + c]`, in cache-sized blocks.
fn transpose(src: &[C], dst: &mut [C], rows: usize, cols: usize) {
    const T: usize = 32;
    for r0 in (0..rows).step_by(T) {
        for c0 in (0..cols).step_by(T) {
            for r in r0..(r0 + T).min(rows) {
                for c in c0..(c0 + T).min(cols) {
                    if let (Some(s), Some(d)) = (src.get(r * cols + c), dst.get_mut(c * rows + r)) {
                        *d = *s;
                    }
                }
            }
        }
    }
}

/// Transforms of a real `w × h` grid (`w` even). The pixels are real, so a row is transformed
/// at half length: pixel pairs travel as one complex number (`pack`), and half a row's spectrum
/// stands for all of it. A plane is `h` rows of `w / 2 + 1`; its spectrum lives transposed.
struct Plans {
    w: usize,
    h: usize,
    /// `exp(-2πi k / w)` for `k <= w / 4`.
    twiddle: Vec<C>,
    engine: Engine,
}

enum Engine {
    /// `rustfft`: SIMD. Not built for the browser.
    #[cfg(not(target_arch = "wasm32"))]
    Fast { row_fwd: Arc<dyn Fft<f32>>, row_inv: Arc<dyn Fft<f32>>, col_fwd: Arc<dyn Fft<f32>>, col_inv: Arc<dyn Fft<f32>> },
    /// The small mixed-radix transform.
    Small { row: SmallFft, col: SmallFft },
}

impl Plans {
    /// `small` is forced on in the browser build; natively it picks the engine (tests use both).
    /// `None` when a side isn't a length [`fft_len`] gives (or `w` is odd).
    fn new(w: usize, h: usize, small: bool) -> Option<Self> {
        if w < 2 || !w.is_multiple_of(2) || h == 0 {
            return None;
        }
        let half = w / 2;
        let twiddle = (0..=half / 2)
            .map(|k| {
                let a = -2.0 * std::f64::consts::PI * k as f64 / w as f64;
                C::new(a.cos() as f32, a.sin() as f32)
            })
            .collect();
        #[cfg(not(target_arch = "wasm32"))]
        if !small {
            let mut planner = FftPlanner::<f32>::new();
            let engine = Engine::Fast {
                row_fwd: planner.plan_fft_forward(half),
                row_inv: planner.plan_fft_inverse(half),
                col_fwd: planner.plan_fft_forward(h),
                col_inv: planner.plan_fft_inverse(h),
            };
            return Some(Plans { w, h, twiddle, engine });
        }
        let _ = small;
        Some(Plans { w, h, twiddle, engine: Engine::Small { row: SmallFft::new(half)?, col: SmallFft::new(h)? } })
    }

    /// Complex numbers per row of a plane, and rows of its spectrum.
    fn stride(&self) -> usize {
        self.w / 2 + 1
    }

    /// Complex numbers in a plane or its spectrum.
    fn len(&self) -> usize {
        self.stride() * self.h
    }

    fn fft(&self, line: &mut [C], columns: bool, inverse: bool) {
        match &self.engine {
            #[cfg(not(target_arch = "wasm32"))]
            Engine::Fast { row_fwd, row_inv, col_fwd, col_inv } => match (columns, inverse) {
                (false, false) => row_fwd.process(line),
                (false, true) => row_inv.process(line),
                (true, false) => col_fwd.process(line),
                (true, true) => col_inv.process(line),
            },
            Engine::Small { row, col } => if columns { col } else { row }.process(line, inverse),
        }
    }

    /// The half-length transform of a packed row, to the first `w / 2 + 1` terms of the row's
    /// own spectrum (the rest are their mirror images).
    fn split(&self, row: &mut [C]) {
        let half = self.w / 2;
        let (Some(&z0), true) = (row.first(), row.len() == half + 1) else { return };
        if let Some(last) = row.last_mut() {
            *last = C::new(z0.re - z0.im, 0.0);
        }
        if let Some(first) = row.first_mut() {
            *first = C::new(z0.re + z0.im, 0.0);
        }
        let Some(inner) = row.get_mut(1..half) else { return };
        let pairs = inner.len() / 2;
        let (lo, hi) = inner.split_at_mut(pairs);
        if let (true, Some(mid)) = (hi.len() > pairs, hi.first_mut()) {
            *mid = mid.conj();
        }
        for ((k, j), w) in lo.iter_mut().zip(hi.iter_mut().rev()).zip(self.twiddle.iter().skip(1)) {
            let (even, odd) = ((*k + j.conj()) * 0.5, (*k - j.conj()) * 0.5);
            let turned = C::new(odd.im, -odd.re) * w;
            (*k, *j) = (even + turned, (even - turned).conj());
        }
    }

    /// Undoes [`Plans::split`], leaving twice the half-length transform.
    fn merge(&self, row: &mut [C]) {
        let half = self.w / 2;
        let (Some(&x0), Some(&xm), true) = (row.first(), row.last(), row.len() == half + 1) else { return };
        let (even, odd) = (x0 + xm.conj(), x0 - xm.conj());
        if let Some(first) = row.first_mut() {
            *first = even + C::new(-odd.im, odd.re);
        }
        let Some(inner) = row.get_mut(1..half) else { return };
        let pairs = inner.len() / 2;
        let (lo, hi) = inner.split_at_mut(pairs);
        if let (true, Some(mid)) = (hi.len() > pairs, hi.first_mut()) {
            *mid = mid.conj() * 2.0;
        }
        for ((k, j), w) in lo.iter_mut().zip(hi.iter_mut().rev()).zip(self.twiddle.iter().skip(1)) {
            let (even, odd) = (*k + j.conj(), (*k - j.conj()) * w.conj());
            (*k, *j) = (even + C::new(-odd.im, odd.re), even.conj() + C::new(odd.im, odd.re));
        }
    }

    /// Packed plane `a` to its spectrum in `b`; `a` is left as scratch.
    fn forward(&self, a: &mut [C], b: &mut [C]) {
        let (half, stride) = (self.w / 2, self.stride());
        par_rows(a, stride, 1, |_, row| {
            if let Some(packed) = row.get_mut(..half) {
                self.fft(packed, false, false);
            }
            self.split(row);
        });
        transpose(a, b, self.h, stride);
        par_rows(b, self.h, 1, |_, col| self.fft(col, true, false));
    }

    /// Spectrum `b` back to a packed plane in `a`, unnormalised (scale by `1 / (w × h)`); `b` is
    /// left as scratch.
    fn inverse(&self, b: &mut [C], a: &mut [C]) {
        let (half, stride) = (self.w / 2, self.stride());
        par_rows(b, self.h, 1, |_, col| self.fft(col, true, true));
        transpose(b, a, stride, self.h);
        par_rows(a, stride, 1, |_, row| {
            self.merge(row);
            if let Some(packed) = row.get_mut(..half) {
                self.fft(packed, false, true);
            }
        });
    }

    /// Writes `f(left, right)` of each pixel pair of the real plane `src` into the packed `a`.
    fn pack(&self, src: &[f32], a: &mut [C], f: impl Fn([f32; 2], C) -> C) {
        for (row, line) in a.chunks_exact_mut(self.stride()).zip(src.chunks_exact(self.w)) {
            for (z, pair) in row.iter_mut().zip(line.as_chunks::<2>().0) {
                *z = f(*pair, *z);
            }
        }
    }
}

/// Richardson–Lucy on one plane (`w × h`, already edge-extended): returns the estimate.
fn richardson_lucy(obs: &[f32], plans: &Plans, kernel_spectrum: &[C], iters: usize, a: &mut [C], b: &mut [C]) -> Vec<f32> {
    let norm = 1.0 / (plans.w * plans.h) as f32;
    let mut est = obs.to_vec();
    for _ in 0..iters {
        plans.pack(&est, a, |e, _| C::new(e[0], e[1]));
        plans.forward(a, b);
        for (z, k) in b.iter_mut().zip(kernel_spectrum) {
            *z *= *k;
        }
        plans.inverse(b, a);
        let ratio = |o: f32, blurred: f32| (o / (blurred * norm).max(EPS)).min(MAX_RATIO);
        plans.pack(obs, a, |o, z| C::new(ratio(o[0], z.re), ratio(o[1], z.im)));
        plans.forward(a, b);
        for (z, k) in b.iter_mut().zip(kernel_spectrum) {
            *z *= k.conj();
        }
        plans.inverse(b, a);
        for (row, line) in a.chunks_exact(plans.stride()).zip(est.chunks_exact_mut(plans.w)) {
            for (z, pair) in row.iter().zip(line.as_chunks_mut::<2>().0) {
                pair[0] *= (z.re * norm).max(0.0);
                pair[1] *= (z.im * norm).max(0.0);
            }
        }
    }
    est
}

/// The deconvolved colour of `out` (alpha unchanged), reading `src` (which covers `out` grown by
/// [`reach`]; samples beyond it repeat the edge).
pub(crate) fn deconvolve(src: &Image, out: Rect, ctx: &Ctx, taps: &[Tap], iters: usize) -> Vec<f32> {
    deconvolve_with(src, out, ctx, taps, iters, cfg!(target_arch = "wasm32"))
}

/// [`deconvolve`] with the transform engine chosen: `small` runs the mixed-radix FFT the browser
/// build uses, otherwise `rustfft`.
pub(super) fn deconvolve_with(src: &Image, out: Rect, ctx: &Ctx, taps: &[Tap], iters: usize, small: bool) -> Vec<f32> {
    let n = src.ch;
    let cc = if ctx.alpha { n - 1 } else { n };
    let mut data = src.data.clone();
    premultiply(&mut data, n, ctx.alpha);
    let premult = Image { rect: src.rect, ch: n, data };
    let mut result = premult.crop(out);
    let (ow, oh) = (out.width() as usize, out.height() as usize);
    let (sw, sh) = (src.rect.width() as usize, src.rect.height() as usize);
    if ow == 0 || oh == 0 || sw == 0 || sh == 0 || iters == 0 {
        unpremultiply(&mut result, n, ctx.alpha);
        return result;
    }
    // The kernel's extent sets the apron; the layer is cut into blocks that fit the transform.
    let radius = taps.iter().map(|t| t.0.abs().max(t.1.abs())).max().unwrap_or(0);
    let apron = (2 * iters as i32 * (radius + 1) + 1).min(MAX_APRON).max(radius + 1) as usize;
    let block = MAX_SIDE.saturating_sub(2 * apron).clamp(64, 2048);
    for by in (0..oh).step_by(block) {
        for bx in (0..ow).step_by(block) {
            let (bw, bh) = (block.min(ow - bx), block.min(oh - by));
            // The window is the block plus its apron, rounded up to a fast transform size; the
            // extra rows and columns read real (or edge-repeated) pixels.
            let (pw, ph) = (2 * fft_len((bw + 2 * apron).div_ceil(2)), fft_len(bh + 2 * apron));
            let (wx, wy) = (out.x0 + bx as i32 - apron as i32, out.y0 + by as i32 - apron as i32);
            let Some(plans) = Plans::new(pw, ph, small) else { continue };
            let mut a = vec![C::new(0.0, 0.0); plans.len()];
            let mut b = vec![C::new(0.0, 0.0); plans.len()];
            let mut obs = vec![0.0f32; pw * ph];
            // Kernel spectrum: the point-spread function wrapped onto the grid, origin at (0, 0).
            for &(dx, dy, k) in taps {
                let (x, y) = (dx.rem_euclid(pw as i32) as usize, dy.rem_euclid(ph as i32) as usize);
                if let Some(cell) = obs.get_mut(y * pw + x) {
                    *cell += k;
                }
            }
            plans.pack(&obs, &mut a, |k, _| C::new(k[0], k[1]));
            plans.forward(&mut a, &mut b);
            let kernel_spectrum = b.clone();
            for c in 0..cc {
                for y in 0..ph {
                    let sy = (wy + y as i32).clamp(src.rect.y0, src.rect.y1 - 1);
                    for x in 0..pw {
                        let sx = (wx + x as i32).clamp(src.rect.x0, src.rect.x1 - 1);
                        let at = ((sy - src.rect.y0) as usize * sw + (sx - src.rect.x0) as usize) * n + c;
                        let v = premult.data.get(at).copied().unwrap_or(0.0);
                        if let Some(o) = obs.get_mut(y * pw + x) {
                            *o = v.max(0.0);
                        }
                    }
                }
                let est = richardson_lucy(&obs, &plans, &kernel_spectrum, iters, &mut a, &mut b);
                for y in 0..bh {
                    for x in 0..bw {
                        let (wxi, wyi) = (x + apron, y + apron);
                        let (Some(e), Some(dst)) = (est.get(wyi * pw + wxi), result.get_mut(((by + y) * ow + bx + x) * n + c)) else { continue };
                        // Samples that were negative (32-bit) are not deconvolved.
                        if *dst >= 0.0 {
                            *dst = *e;
                        }
                    }
                }
            }
        }
    }
    unpremultiply(&mut result, n, ctx.alpha);
    result
}

#[cfg(test)]
#[path = "deconv_tests.rs"]
mod tests;
