//! Shared helpers for the second filter batch: colour-model access, window
//! buffers, single-channel box/Gaussian blurs and jittered cell points.

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{from_rgba_into, to_rgba};

use crate::Ctx;
use crate::image::Image;
use crate::noise::hash01;

/// Max channels per pixel we keep on the stack (CMYK + alpha + spare).
pub(crate) const MAXC: usize = 8;

/// The context's model as an `f32` format (for colour conversions).
#[inline]
pub(crate) fn fmt(ctx: &Ctx) -> PixelFormat {
    PixelFormat::new(ctx.mode, SampleType::F32, ctx.alpha)
}

/// Colour channels (excluding alpha).
#[inline]
pub(crate) fn ncol(ctx: &Ctx, n: usize) -> usize {
    if ctx.alpha { n - 1 } else { n }
}

/// Whether channel values are ink amounts (1 = dark) rather than light.
#[inline]
pub(crate) fn subtractive(ctx: &Ctx) -> bool {
    ctx.mode == ColorMode::Cmyk
}

/// Whether per-channel "brightness" operations should run on an RGB
/// conversion instead of the native channels (Lab's a/b are not intensities).
#[inline]
pub(crate) fn via_rgb(ctx: &Ctx) -> bool {
    ctx.mode == ColorMode::Lab
}

/// Document coordinates of the `i`th pixel of `out`.
#[inline]
pub(crate) fn xy(out: Rect, i: usize) -> (i32, i32) {
    let w = out.width() as usize;
    (out.x0 + (i % w) as i32, out.y0 + (i / w) as i32)
}

/// Straight sRGB RGBA of a native pixel.
#[inline]
pub(crate) fn rgba(ctx: &Ctx, px: &[f32]) -> [f32; 4] {
    to_rgba(&fmt(ctx), px)
}

/// Writes straight sRGB RGBA into a native pixel.
#[inline]
pub(crate) fn set_rgba(ctx: &Ctx, px: &mut [f32], c: [f32; 4]) {
    let mut tmp = [0.0f32; MAXC];
    let n = from_rgba_into(&fmt(ctx), c, &mut tmp);
    let m = n.min(px.len());
    px[..m].copy_from_slice(&tmp[..m]);
}

/// A straight sRGB RGBA colour in the native model.
pub(crate) fn native(ctx: &Ctx, c: [f32; 4]) -> [f32; MAXC] {
    let mut tmp = [0.0f32; MAXC];
    from_rgba_into(&fmt(ctx), c, &mut tmp);
    tmp
}

/// Rec. 601 luma of a native pixel.
#[inline]
pub(crate) fn luma(ctx: &Ctx, px: &[f32]) -> f32 {
    match ctx.mode {
        ColorMode::Grayscale | ColorMode::Bitmap | ColorMode::Duotone => px[0],
        ColorMode::Lab => px[0],
        ColorMode::Rgb => 0.299 * px[0] + 0.587 * px[1] + 0.114 * px[2],
        _ => {
            let c = rgba(ctx, px);
            0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]
        }
    }
}

/// Interleaved premultiplied copy of `src` over `win` (zero outside).
pub(crate) fn premul_window(src: &Image, win: Rect, alpha: bool) -> Vec<f32> {
    let n = src.ch;
    let mut v = Vec::with_capacity(win.width() as usize * win.height() as usize * n);
    for y in win.y0..win.y1 {
        for x in win.x0..win.x1 {
            let a = if alpha { src.get(x, y, n - 1) } else { 1.0 };
            for c in 0..n {
                let s = src.get(x, y, c);
                v.push(if alpha && c < n - 1 { s * a } else { s });
            }
        }
    }
    v
}

/// Unpremultiplies one interleaved pixel in place.
#[inline]
pub(crate) fn unpremul_px(px: &mut [f32], alpha: bool) {
    if !alpha {
        return;
    }
    let n = px.len();
    let a = px[n - 1];
    for v in px.iter_mut().take(n - 1) {
        *v = if a > 1e-7 { *v / a } else { 0.0 };
    }
}

/// In-place running-sum box blur of every row of a `w × h × n` interleaved
/// buffer with radius `r` (edges clamped).
fn box_rows_n(buf: &mut [f32], w: usize, n: usize, r: usize) {
    box_rows_multi(buf, w, n, &[r]);
}

/// Several [`box_rows_n`] passes (one per radius, in order; a radius of 0 is skipped). Each row
/// is independent, so rows run in parallel and a row goes through every pass while it is
/// still in cache. A row's arithmetic is the same, in the same order, as one pass at a time.
fn box_rows_multi(buf: &mut [f32], w: usize, n: usize, radii: &[usize]) {
    if w == 0 || n == 0 || radii.iter().all(|&r| r == 0) {
        return;
    }
    let pass = |row: &mut [f32], scratch: &mut Vec<f32>| {
        for &r in radii.iter().filter(|&&r| r != 0) {
            let norm = 1.0 / (2 * r + 1) as f64;
            scratch.clear();
            scratch.extend_from_slice(row);
            // f64 sums: rounding must not depend on where a window starts (tile independence).
            let at = |x: isize, c: usize| scratch[(x.clamp(0, w as isize - 1) as usize) * n + c] as f64;
            for c in 0..n {
                let mut acc: f64 = (-(r as isize)..=r as isize).map(|x| at(x, c)).sum();
                for x in 0..w {
                    row[x * n + c] = (acc * norm) as f32;
                    acc += at(x as isize + r as isize + 1, c) - at(x as isize - r as isize, c);
                }
            }
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        buf.par_chunks_exact_mut(w * n).for_each_init(Vec::new, |scratch, row| pass(row, scratch));
    }
    #[cfg(target_arch = "wasm32")]
    {
        let mut scratch = Vec::new();
        buf.chunks_exact_mut(w * n).for_each(|row| pass(row, &mut scratch));
    }
}

/// Elements (`columns × channels`) in one band of [`box_cols_multi`]: a few cache lines per
/// row, so the strided walk down a band touches whole lines and its working set stays in L2.
const COL_BAND_ELEMS: usize = 64;

/// Same as [`box_rows_n`] over columns.
fn box_cols_n(buf: &mut [f32], w: usize, h: usize, n: usize, r: usize) {
    box_cols_multi(buf, w, h, n, &[r]);
}

/// Several [`box_cols_n`] passes (one per radius, in order; a radius of 0 is skipped).
///
/// The columns are cut into bands. A band is gathered into a small contiguous buffer (every
/// row's slice of the band), run through all the passes there, and written back, so neither the
/// strided walk nor the intermediate passes touch the full image. Bands are independent and run
/// in parallel; each one holds the `&mut` slices of its own columns, so no `unsafe` is needed.
/// Every column goes through exactly the arithmetic of the one-column-at-a-time version: an f64
/// running sum started from the same ordered sum of its first window and updated in the same
/// order. Only the loop nest is turned around (the inner loop runs across the band's columns,
/// so it vectorises), which cannot change the value of any single column's sum.
fn box_cols_multi(buf: &mut [f32], w: usize, h: usize, n: usize, radii: &[usize]) {
    if w == 0 || h == 0 || n == 0 || radii.iter().all(|&r| r == 0) {
        return;
    }
    let band_cols = (COL_BAND_ELEMS / n).max(1);
    let mut bands: Vec<Vec<&mut [f32]>> = (0..w.div_ceil(band_cols)).map(|_| Vec::with_capacity(h)).collect();
    for row in buf[..w * h * n].chunks_exact_mut(w * n) {
        for (band, seg) in bands.iter_mut().zip(row.chunks_mut(band_cols * n)) {
            band.push(seg);
        }
    }
    let run = |mut segs: Vec<&mut [f32]>, a: &mut Vec<f32>, b: &mut Vec<f32>, acc: &mut Vec<f64>| {
        let bw = segs.first().map_or(0, |s| s.len());
        a.clear();
        for s in &segs {
            a.extend_from_slice(s);
        }
        b.clear();
        b.resize(a.len(), 0.0);
        for &r in radii.iter().filter(|&&r| r != 0) {
            let norm = 1.0 / (2 * r + 1) as f64;
            let row = |y: isize| (y.clamp(0, h as isize - 1) as usize) * bw;
            // The first window, summed in order (the same `Sum` as the per-column version).
            acc.clear();
            acc.resize(bw, std::iter::empty::<f64>().sum());
            for y in -(r as isize)..=r as isize {
                for (t, &v) in acc.iter_mut().zip(&a[row(y)..row(y) + bw]) {
                    *t += v as f64;
                }
            }
            for y in 0..h {
                let (add, sub) = (&a[row(y as isize + r as isize + 1)..][..bw], &a[row(y as isize - r as isize)..][..bw]);
                for (((o, t), &p), &m) in b[y * bw..(y + 1) * bw].iter_mut().zip(acc.iter_mut()).zip(add).zip(sub) {
                    *o = (*t * norm) as f32;
                    *t += p as f64 - m as f64;
                }
            }
            std::mem::swap(a, b);
        }
        for (y, s) in segs.iter_mut().enumerate() {
            s.copy_from_slice(&a[y * bw..(y + 1) * bw]);
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        bands.into_par_iter().for_each_init(|| (Vec::new(), Vec::new(), Vec::new()), |(a, b, acc), segs| run(segs, a, b, acc));
    }
    #[cfg(target_arch = "wasm32")]
    {
        let (mut a, mut b, mut acc) = (Vec::new(), Vec::new(), Vec::new());
        for segs in bands {
            run(segs, &mut a, &mut b, &mut acc);
        }
    }
}

/// Box blur of an interleaved buffer (radius `r` in both directions).
pub(crate) fn box_blur_n(buf: &mut [f32], w: usize, h: usize, n: usize, r: usize) {
    box_rows_n(buf, w, n, r);
    box_cols_n(buf, w, h, n, r);
}

/// Odd box widths whose 3-fold convolution approximates a Gaussian of `sigma`
/// (Kovesi 2010), returned as radii.
pub(crate) fn gauss_box_radii(sigma: f32) -> [usize; 3] {
    let n = 3.0f32;
    let w_ideal = (12.0 * sigma * sigma / n + 1.0).sqrt();
    let mut wl = w_ideal.floor() as i32;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wl = wl.max(1);
    let wu = wl + 2;
    let m = ((12.0 * sigma * sigma - n * (wl * wl) as f32 - 4.0 * n * wl as f32 - 3.0 * n) / (-4.0 * wl as f32 - 4.0)).round().clamp(0.0, n) as usize;
    let mut radii = std::array::from_fn(|i| if i < m { (wl as usize - 1) / 2 } else { (wu as usize - 1) / 2 });
    // Kovesi's rounded triple degenerates below sigma ~ 0.577: `wl` floors to 1 and every
    // radius becomes 0, so the passes would be no-ops and callers with documented small
    // sigmas (Camera Raw `sharpenRadius` 0.5, colour noise, HDR Toning `radius` 1) would
    // silently change nothing (issue #706). One radius-1 pass is the smallest real blur;
    // it only engages where the ideal widths are already at their floor, leaving larger
    // sigmas exactly as the paper prescribes.
    if radii.iter().all(|&r| r == 0) {
        radii[0] = 1;
    }
    radii
}

/// Approximate Gaussian blur (three box passes) of an interleaved buffer.
/// Exact small kernels are not needed here: callers use it for smoothing
/// fields and blur levels, where cost independent of `sigma` matters more.
pub(crate) fn gauss_blur_n(buf: &mut [f32], w: usize, h: usize, n: usize, sigma: f32) {
    if sigma < 0.3 {
        return;
    }
    // The three passes of each direction run together (see `box_rows_multi`): one trip through
    // memory per direction instead of three, with identical results.
    let radii = gauss_box_radii(sigma);
    box_rows_multi(buf, w, n, &radii);
    box_cols_multi(buf, w, h, n, &radii);
}

/// Jittered feature point of grid cell `(cx, cy)` (cells of `cell` px anchored at `origin`).
#[inline]
pub(crate) fn cell_point(cx: i32, cy: i32, cell: f32, seed: u32, origin: (f32, f32)) -> (f32, f32) {
    (origin.0 + (cx as f32 + 0.1 + 0.8 * hash01(cx, cy, 11, seed)) * cell, origin.1 + (cy as f32 + 0.1 + 0.8 * hash01(cx, cy, 12, seed)) * cell)
}

#[inline]
pub(crate) fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    if e1 <= e0 {
        return if x >= e1 { 1.0 } else { 0.0 };
    }
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Adds the premultiplied bilinear sample of `src` at `(x, y)` (pixel centres at .5, edges
/// clamped to `clip`, which must lie inside `src.rect`) into `acc`. A fast path for blurs that
/// average many samples; unpremultiply the sum once at the end.
#[inline]
pub(crate) fn add_sample_premul(src: &Image, x: f32, y: f32, clip: Rect, alpha: bool, acc: &mut [f32]) {
    if !x.is_finite() || !y.is_finite() || clip.is_empty() {
        return;
    }
    let n = src.ch;
    let fx = (x - 0.5).clamp(clip.x0 as f32, (clip.x1 - 1) as f32);
    let fy = (y - 0.5).clamp(clip.y0 as f32, (clip.y1 - 1) as f32);
    let (x0, y0) = (fx.floor() as i32, fy.floor() as i32);
    let (x1, y1) = ((x0 + 1).min(clip.x1 - 1), (y0 + 1).min(clip.y1 - 1));
    let (ax, ay) = (fx - x0 as f32, fy - y0 as f32);
    let w = src.rect.width() as usize;
    let idx = |xx: i32, yy: i32| ((yy - src.rect.y0) as usize * w + (xx - src.rect.x0) as usize) * n;
    for (i, k) in [(idx(x0, y0), (1.0 - ax) * (1.0 - ay)), (idx(x1, y0), ax * (1.0 - ay)), (idx(x0, y1), (1.0 - ax) * ay), (idx(x1, y1), ax * ay)] {
        if k <= 0.0 {
            continue;
        }
        let px = &src.data[i..i + n];
        let a = if alpha { px[n - 1] * k } else { k };
        if alpha {
            for c in 0..n - 1 {
                acc[c] += px[c] * a;
            }
            acc[n - 1] += a;
        } else {
            for c in 0..n {
                acc[c] += px[c] * a;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The serial box passes exactly as they were before the row / column-band parallelisation.
    // Every output must stay bit for bit the same, so the new code is compared against these.
    fn ref_rows(buf: &mut [f32], w: usize, n: usize, r: usize) {
        if r == 0 || w == 0 {
            return;
        }
        let mut scratch: Vec<f32> = Vec::new();
        let norm = 1.0 / (2 * r + 1) as f64;
        for row in buf.chunks_exact_mut(w * n) {
            scratch.clear();
            scratch.extend_from_slice(row);
            let at = |x: isize, c: usize| scratch[(x.clamp(0, w as isize - 1) as usize) * n + c] as f64;
            for c in 0..n {
                let mut acc: f64 = (-(r as isize)..=r as isize).map(|x| at(x, c)).sum();
                for x in 0..w {
                    row[x * n + c] = (acc * norm) as f32;
                    acc += at(x as isize + r as isize + 1, c) - at(x as isize - r as isize, c);
                }
            }
        }
    }

    fn ref_cols(buf: &mut [f32], w: usize, h: usize, n: usize, r: usize) {
        if r == 0 || h == 0 {
            return;
        }
        let mut scratch: Vec<f32> = Vec::new();
        let norm = 1.0 / (2 * r + 1) as f64;
        for x in 0..w {
            scratch.clear();
            for y in 0..h {
                scratch.extend_from_slice(&buf[(y * w + x) * n..(y * w + x + 1) * n]);
            }
            let at = |y: isize, c: usize| scratch[(y.clamp(0, h as isize - 1) as usize) * n + c] as f64;
            for c in 0..n {
                let mut acc: f64 = (-(r as isize)..=r as isize).map(|y| at(y, c)).sum();
                for y in 0..h {
                    buf[(y * w + x) * n + c] = (acc * norm) as f32;
                    acc += at(y as isize + r as isize + 1, c) - at(y as isize - r as isize, c);
                }
            }
        }
    }

    fn ref_gauss(buf: &mut [f32], w: usize, h: usize, n: usize, sigma: f32) {
        if sigma < 0.3 {
            return;
        }
        for r in gauss_box_radii(sigma) {
            ref_rows(buf, w, n, r);
        }
        for r in gauss_box_radii(sigma) {
            ref_cols(buf, w, h, n, r);
        }
    }

    fn mix(mut z: u64) -> u64 {
        z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Deterministic data from integer arithmetic only (no libm), with signed zeros, negatives
    /// and large values so rounding differences cannot hide.
    fn noise(len: usize, seed: u64) -> Vec<f32> {
        (0..len)
            .map(|i| {
                let m = mix(i as u64 ^ seed.wrapping_mul(0x9E37));
                match m % 23 {
                    0 => 0.0,
                    1 => -0.0,
                    2 => -(((m >> 8) & 0xFFFF) as f32) / 100.0,
                    3 => 1e6 * ((m >> 8) & 0xFF) as f32,
                    _ => ((m >> 8) & 0xFF_FFFF) as f32 / 1e6 - 3.0,
                }
            })
            .collect()
    }

    fn fnv(v: &[f32]) -> u64 {
        v.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, x| (h ^ x.to_bits() as u64).wrapping_mul(0x0000_0100_0000_01b3))
    }

    fn rows(buf: &mut [f32], w: usize, n: usize, r: usize) {
        box_rows_n(buf, w, n, r);
    }

    fn cols(buf: &mut [f32], w: usize, h: usize, n: usize, r: usize) {
        box_cols_n(buf, w, h, n, r);
    }

    /// Every shape around the column-band / row boundaries, every radius that matters (0, 1, a
    /// window wider than the image), 1 to 5 channels: new passes == the old serial ones, bit for bit.
    #[test]
    fn box_passes_match_the_serial_reference_bit_for_bit() {
        for w in (1..=70).chain([95, 127, 128, 129, 130, 255, 256, 257]) {
            for h in [1usize, 5, 64] {
                for n in [1usize, 3, 5] {
                    let src = noise(w * h * n, (w * 131 + h * 7 + n) as u64);
                    for r in [0usize, 1, 3, 7, 60, 1000] {
                        let (mut a, mut b) = (src.clone(), src.clone());
                        rows(&mut a, w, n, r);
                        ref_rows(&mut b, w, n, r);
                        assert!(a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()), "rows w={w} h={h} n={n} r={r}");
                        let (mut a, mut b) = (src.clone(), src.clone());
                        cols(&mut a, w, h, n, r);
                        ref_cols(&mut b, w, h, n, r);
                        assert!(a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()), "cols w={w} h={h} n={n} r={r}");
                    }
                }
            }
        }
    }

    /// The blurs the filters call, on a buffer big enough for several bands and many rows.
    #[test]
    fn box_and_gauss_blur_match_the_serial_reference_bit_for_bit() {
        for (w, h, n) in [(1usize, 300usize, 1usize), (300, 1, 3), (97, 61, 3), (203, 131, 2), (257, 190, 1)] {
            let src = noise(w * h * n, 5);
            for r in [0usize, 1, 5, 40, 5000] {
                let mut a = src.clone();
                box_blur_n(&mut a, w, h, n, r);
                let mut b = src.clone();
                ref_rows(&mut b, w, n, r);
                ref_cols(&mut b, w, h, n, r);
                assert!(a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()), "box_blur_n w={w} h={h} n={n} r={r}");
            }
            for s in [0.2f32, 0.5, 1.0, 2.5, 9.0, 40.0] {
                let mut a = src.clone();
                gauss_blur_n(&mut a, w, h, n, s);
                let mut b = src.clone();
                ref_gauss(&mut b, w, h, n, s);
                assert!(a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()), "gauss_blur_n w={w} h={h} n={n} sigma={s}");
            }
        }
    }

    /// Hashes recorded from the code before the passes were parallelised (plain `+ - * /` in
    /// f64, so the same on every platform).
    #[test]
    fn box_passes_keep_their_recorded_output() {
        let (w, h, n) = (97usize, 61usize, 3usize);
        let src = noise(w * h * n, 9);
        let hash = |f: &dyn Fn(&mut [f32])| {
            let mut b = src.clone();
            f(&mut b);
            fnv(&b)
        };
        assert_eq!(hash(&|b| rows(b, w, n, 5)), 0x5acb99b1782b10d7, "rows r=5");
        assert_eq!(hash(&|b| cols(b, w, h, n, 5)), 0xb873613520df2673, "cols r=5");
        assert_eq!(hash(&|b| cols(b, w, h, n, 1000)), 0x7e956815578db119, "cols r=1000");
        assert_eq!(hash(&|b| box_blur_n(b, w, h, n, 2)), 0xb9506c5bf5a668a4, "box_blur r=2");
        assert_eq!(hash(&|b| gauss_blur_n(b, w, h, n, 3.5)), 0xc8732aea70315a4c, "gauss s=3.5");
        let one = noise(70, 4);
        let mut b = one.clone();
        box_blur_n(&mut b, 1, 70, 1, 3);
        assert_eq!(fnv(&b), 0x4258ed17aa75ac1d, "1-wide");
    }
}
