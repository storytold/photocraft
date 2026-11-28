//! Unsharp Mask, Smart Sharpen (basic), High Pass.

use photocraft_geom::Rect;

use crate::Ctx;
use crate::blur::{conv_sep, gaussian_kernel};
use crate::image::Image;

pub(crate) mod exact;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod fft;

#[cfg(test)]
mod tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod fft_bench;

fn with_blur(src: &Image, out: Rect, ctx: &Ctx, radius: f32, mut f: impl FnMut(f32, f32) -> f32) -> Vec<f32> {
    let k = gaussian_kernel(radius);
    let blurred = conv_sep(src, out, &k, &k, ctx.alpha);
    let mut res = src.crop(out);
    let n = src.ch;
    let cc = if ctx.alpha { n - 1 } else { n };
    for (px, bl) in res.chunks_exact_mut(n).zip(blurred.chunks_exact(n)) {
        if ctx.alpha && px[n - 1] <= 0.0 {
            continue;
        }
        for c in 0..cc {
            px[c] = f(px[c], bl[c]);
        }
    }
    res
}

pub(crate) fn unsharp(src: &Image, out: Rect, ctx: &Ctx, amount: f32, radius: f32, threshold: f32) -> Vec<f32> {
    exact::run(src, out, ctx, amount, radius, threshold, &photocraft_raster::Interrupt::NONE).unwrap_or_default()
}

pub(crate) fn smart(src: &Image, out: Rect, ctx: &Ctx, amount: f32, radius: f32, reduce_noise: f32) -> Vec<f32> {
    let k = amount / 100.0;
    let nr = (reduce_noise / 100.0).clamp(0.0, 1.0);
    with_blur(src, out, ctx, radius, |o, b| {
        let d = o - b;
        // Suppress small (noise-level) differences.
        let keep = 1.0 - nr * (-(d.abs() * 40.0)).exp();
        o + k * d * keep
    })
}

pub(crate) fn high_pass(src: &Image, out: Rect, ctx: &Ctx, radius: f32) -> Vec<f32> {
    if let Some(boxes) = crate::blur::box_widths(&crate::FilterParams::HighPass { radius }) {
        let mut blurred = crate::blur::box_passes(src, out, ctx.alpha, &boxes);
        high_pass_residual(&mut blurred, &src.crop(out), src.ch, ctx.alpha);
        return blurred;
    }
    with_blur(src, out, ctx, radius, |o, b| 0.5 + (o - b))
}

/// Turn straight blurred colours into the residual before mixing the selection.
pub(crate) fn high_pass_residual(blurred: &mut [f32], original: &[f32], n: usize, alpha: bool) {
    for (dst, src) in blurred.chunks_exact_mut(n).zip(original.chunks_exact(n)) {
        if alpha && src[n - 1] <= 0.0 {
            dst.copy_from_slice(src);
            continue;
        }
        for c in 0..n - usize::from(alpha) {
            dst[c] = 0.5 + (src[c] - dst[c]);
        }
        if alpha {
            dst[n - 1] = src[n - 1];
        }
    }
}
