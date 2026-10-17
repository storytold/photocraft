//! Unsharp Mask, Smart Sharpen (Gaussian unsharp, or Richardson–Lucy deconvolution for Lens and Motion Blur), High Pass.

use photocraft_geom::Rect;

use crate::blur::{conv_sep, gaussian_kernel};
use crate::image::Image;
use crate::{Ctx, SharpenRemove};

pub(crate) mod deconv;
mod smallfft;

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

#[allow(clippy::too_many_arguments)]
pub(crate) fn smart(
    src: &Image,
    out: Rect,
    ctx: &Ctx,
    amount: f32,
    radius: f32,
    reduce_noise: f32,
    remove: SharpenRemove,
    angle: f32,
    more_accurate: bool,
) -> Vec<f32> {
    let k = amount / 100.0;
    let nr = (reduce_noise / 100.0).clamp(0.0, 1.0);
    // Suppress small (noise-level) differences.
    let keep = |d: f32| 1.0 - nr * (-(d.abs() * 40.0)).exp();
    if remove == SharpenRemove::Gaussian {
        return with_blur(src, out, ctx, radius, |o, b| {
            let d = o - b;
            o + k * d * keep(d)
        });
    }
    let taps = deconv::kernel(remove, radius, angle);
    let sharp = deconv::deconvolve(src, out, ctx, &taps, deconv::iterations(more_accurate));
    let mut res = src.crop(out);
    let n = src.ch;
    let cc = if ctx.alpha { n - 1 } else { n };
    for (px, sh) in res.chunks_exact_mut(n).zip(sharp.chunks_exact(n)) {
        if ctx.alpha && px[n - 1] <= 0.0 {
            continue;
        }
        for c in 0..cc {
            let d = sh[c] - px[c];
            px[c] += k * d * keep(d);
        }
    }
    res
}

pub(crate) fn high_pass(src: &Image, out: Rect, ctx: &Ctx, radius: f32) -> Vec<f32> {
    with_blur(src, out, ctx, radius, |o, b| 0.5 + (o - b))
}
