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
/// Scratch a column gather may hold: L2-resident, so the transposed strip stays hot while it is
/// read back. Larger strips evict it and lose the point of gathering.
const STRIP_BYTES: usize = 64 * 1024;

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

/// One line of `w` interleaved `n`-channel samples, blurred in place by a running sum (edges
/// clamped). `scratch` holds the untouched line so an in-place sweep can still read behind it.
fn box_line_n(line: &mut [f32], w: usize, n: usize, r: usize, scratch: &mut Vec<f32>) {
    debug_assert!(n <= MAXC, "one accumulator per channel");
    scratch.clear();
    scratch.extend_from_slice(&line[..w * n]);
    let src = &scratch[..];
    let norm = 1.0 / (2 * r + 1) as f64;
    // A tap clamps only within `r` (or `r + 1`) of an end. Between those the index is plain, so
    // the bulk of the line skips the clamp and the bounds check; the arithmetic is unchanged.
    // The invariant that makes the plain indices safe: the middle runs while `x >= r` and
    // `x <= w - r - 2`, where `x - r >= 0` and `x + r + 1 <= w - 1`. `first` is `r` (a line
    // shorter than the radius has no middle: `first = w`) and `last` is `w - (r + 1)`, never
    // below `first`; outside that span both taps clamp, which is the same sum, so every index
    // stays inside the untouched copy `src[..w * n]` for any `w`, `r` and `n <= MAXC`.
    let first = r.min(w);
    let last = w.saturating_sub(r + 1).max(first);
    // f64 sums: rounding must not depend on where a window starts (tile independence).
    let mut acc = [0.0f64; MAXC];
    for c in 0..n {
        let at = |x: isize| src[(x.clamp(0, w as isize - 1) as usize) * n + c] as f64;
        acc[c] = (-(r as isize)..=r as isize).map(at).sum();
    }
    for x in 0..w {
        for c in 0..n {
            line[x * n + c] = (acc[c] * norm) as f32;
        }
        if x >= first && x < last {
            let (out, inp) = ((x + r + 1) * n, (x - r) * n);
            for c in 0..n {
                acc[c] += src[out + c] as f64 - src[inp + c] as f64;
            }
        } else {
            for c in 0..n {
                let at = |i: isize| src[(i.clamp(0, w as isize - 1) as usize) * n + c] as f64;
                acc[c] += at(x as isize + r as isize + 1) - at(x as isize - r as isize);
            }
        }
    }
}

/// In-place box blur of every row of a `w × h × n` interleaved buffer with radius `r` (edges
/// clamped).
fn box_rows_n(buf: &mut [f32], w: usize, n: usize, r: usize) {
    let band = w * n;
    if r == 0 || band == 0 {
        return;
    }
    // Trailing samples that do not fill a row are left alone, as `chunks_exact_mut` did.
    let full = buf.len() - buf.len() % band;
    let mut scratch = Vec::with_capacity(band);
    for row in buf[..full].chunks_exact_mut(band) {
        box_line_n(row, w, n, r, &mut scratch);
    }
}

/// Same as [`box_rows_n`] over columns (no transpose of the whole buffer; gathers a strip).
///
/// A column walk reads one sample per row at a stride of `w * n`, so every sample pulls its own
/// cache line and uses `n * 4` of its 64 bytes: on a 1122-wide RGBA buffer that is 8x the
/// image's traffic. Gathering `STRIP_BYTES` worth of neighbouring columns into a transposed
/// scratch first replaces that with one line read per row, and the running sum then walks
/// contiguous runs. `box_line_n`'s rule holds per column too: outside `r` of the ends both taps
/// are in range, so only the ends clamp.
fn box_cols_n(buf: &mut [f32], w: usize, h: usize, n: usize, r: usize) {
    if r == 0 || h == 0 || n == 0 || buf.len() < w.saturating_mul(h).saturating_mul(n) {
        return;
    }
    // Columns per gather, from the scratch budget. Below 2 the gather costs more than it saves.
    let strip = (STRIP_BYTES / (n * h * 4)).clamp(0, w);
    if strip < 2 {
        box_cols_strided(buf, w, h, n, r);
        return;
    }
    let norm = 1.0 / (2 * r + 1) as f64;
    let first = r.min(h);
    let last = h.saturating_sub(r + 1).max(first);
    let mut scratch: Vec<f32> = Vec::with_capacity(strip * n * h);
    let mut x0 = 0;
    while x0 < w {
        let sx = strip.min(w - x0);
        let runs = sx * n;
        // Transposed gather: run `k` (column `x0 + k / n`, channel `k % n`) is contiguous.
        scratch.clear();
        scratch.resize(runs * h, 0.0);
        for y in 0..h {
            let base = (y * w + x0) * n;
            for k in 0..runs {
                scratch[k * h + y] = buf[base + k];
            }
        }
        for k in 0..runs {
            let src = &scratch[k * h..][..h];
            let (col, ch) = (k / n, k % n);
            // f64 sums, as in `box_line_n`: the result must not depend on the walk's direction.
            let mut acc: f64 = (-(r as isize)..=r as isize).map(|y| src[y.clamp(0, h as isize - 1) as usize] as f64).sum();
            for y in 0..h {
                buf[(y * w + x0 + col) * n + ch] = (acc * norm) as f32;
                if y >= first && y < last {
                    acc += src[y + r + 1] as f64 - src[y - r] as f64;
                } else {
                    let hi = (y + r + 1).min(h - 1);
                    let lo = y.saturating_sub(r);
                    acc += src[hi] as f64 - src[lo] as f64;
                }
            }
        }
        x0 += sx;
    }
}

/// The column walk one sample at a time, kept for the widths one contiguous gather cannot hold.
fn box_cols_strided(buf: &mut [f32], w: usize, h: usize, n: usize, r: usize) {
    if r == 0 || h == 0 || n == 0 {
        return;
    }
    let norm = 1.0 / (2 * r + 1) as f64;
    let mut scratch = Vec::with_capacity(h * n);
    for x in 0..w {
        scratch.clear();
        for y in 0..h {
            scratch.extend_from_slice(&buf[(y * w + x) * n..(y * w + x + 1) * n]);
        }
        // f64 sums, as in `box_line_n`: the result must not depend on the walk's direction.
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
    let radii = gauss_box_radii(sigma);
    for r in radii {
        box_rows_n(buf, w, n, r);
    }
    for r in radii {
        box_cols_n(buf, w, h, n, r);
    }
}

/// [`gauss_blur_n`] that first checks its scratch memory can be had: `None` (the buffer
/// untouched) when it can't, instead of aborting the app. It needs a copy of the buffer, which
/// lets the passes run in parallel (rows, then strips of columns) with the same result bit for
/// bit.
pub(crate) fn try_gauss_blur_n(buf: &mut [f32], w: usize, h: usize, n: usize, sigma: f32) -> Option<()> {
    if sigma < 0.3 {
        return Some(());
    }
    // A copy of the buffer, which the column passes write their strips into.
    let len = w.checked_mul(h)?.checked_mul(n)?;
    let mut tmp: Vec<f32> = Vec::new();
    tmp.try_reserve_exact(len).ok()?;
    if buf.len() < len {
        gauss_blur_n(buf, w, h, n, sigma);
        return Some(());
    }
    tmp.resize(len, 0.0);
    let radii = gauss_box_radii(sigma);
    for r in radii {
        par_box_rows_n(buf, w, n, r);
    }
    let strip = par_strip(w);
    for r in radii {
        par_box_cols_n(&mut buf[..len], &mut tmp, w, h, n, r, strip);
    }
    Some(())
}

/// Columns per strip in [`par_box_cols_n`]: about two strips a thread, at least 8 columns.
fn par_strip(w: usize) -> usize {
    #[cfg(not(target_arch = "wasm32"))]
    let workers = rayon::current_num_threads().max(1);
    #[cfg(target_arch = "wasm32")]
    let workers: usize = 1;
    w.div_ceil(workers.saturating_mul(2)).max(8).min(w.max(1))
}

/// [`box_rows_n`] with the rows spread over threads: each row is the same running sum, so the
/// result is bit for bit the same.
fn par_box_rows_n(buf: &mut [f32], w: usize, n: usize, r: usize) {
    let band = w * n;
    if r == 0 || band == 0 {
        return;
    }
    let full = buf.len() - buf.len() % band;
    crate::photo_util::par_rows(&mut buf[..full], w, n, |_, row| {
        let mut scratch = Vec::with_capacity(band);
        box_line_n(row, w, n, r, &mut scratch);
    });
}

/// [`box_cols_n`] with strips of `strip` columns spread over threads: each strip is blurred
/// into its own part of `tmp` (as many samples as `buf`, `w × h × n`), then copied back row by
/// row. Each column is the same running sum (f64, the same taps clamped at the ends, added in
/// the same order), so the result is bit for bit the same.
fn par_box_cols_n(buf: &mut [f32], tmp: &mut [f32], w: usize, h: usize, n: usize, r: usize, strip: usize) {
    if r == 0 || h == 0 || n == 0 || w == 0 || tmp.len() != buf.len() || buf.len() != w * h * n {
        return;
    }
    let strip = strip.clamp(1, w);
    {
        let src: &[f32] = buf;
        // Strip `g` takes `strip × n × h` samples of `tmp` (the last one fewer), row by row.
        crate::photo_util::par_rows(tmp, strip * h, n, |g, out| {
            let x0 = g * strip;
            col_strip(src, w, h, n, r, x0..x0 + strip.min(w.saturating_sub(x0)), out);
        });
    }
    let tmp: &[f32] = tmp;
    crate::photo_util::par_rows(buf, w, n, |y, row| {
        let mut x0 = 0;
        while x0 < w {
            let runs = strip.min(w - x0) * n;
            let at = x0 * n * h + y * runs;
            if let (Some(d), Some(s)) = (row.get_mut(x0 * n..x0 * n + runs), tmp.get(at..at + runs)) {
                d.copy_from_slice(s);
            }
            x0 += strip;
        }
    });
}

/// The columns `cols` of `buf` blurred down (radius `r`) into `out`, row by row (`h`
/// rows of `cols.len() * n` samples): one running sum per column and channel, walking whole rows.
fn col_strip(buf: &[f32], w: usize, h: usize, n: usize, r: usize, cols: std::ops::Range<usize>, out: &mut [f32]) {
    let (x0, sx) = (cols.start, cols.len());
    let runs = sx * n;
    if runs == 0 {
        return;
    }
    let row = |y: usize| &buf[(y * w + x0) * n..][..runs];
    let norm = 1.0 / (2 * r + 1) as f64;
    let first = r.min(h);
    let last = h.saturating_sub(r + 1).max(first);
    // As in `box_cols_n`: the window's first sum, then one tap in and one out per row.
    let mut acc = vec![0.0f64; runs];
    for y in -(r as isize)..=r as isize {
        for (a, v) in acc.iter_mut().zip(row(y.clamp(0, h as isize - 1) as usize)) {
            *a += *v as f64;
        }
    }
    for (y, o) in out.chunks_exact_mut(runs).enumerate().take(h) {
        for (o, a) in o.iter_mut().zip(&acc) {
            *o = (*a * norm) as f32;
        }
        let (hi, lo) = if y >= first && y < last { (y + r + 1, y - r) } else { ((y + r + 1).min(h - 1), y.saturating_sub(r)) };
        let (hi, lo) = (row(hi), row(lo));
        for k in 0..runs {
            acc[k] += hi[k] as f64 - lo[k] as f64;
        }
    }
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
    use super::{box_cols_n, box_cols_strided, box_line_n, box_rows_n, gauss_blur_n, par_box_cols_n, try_gauss_blur_n};

    /// The parallel blur (rows over threads, columns in strips) gives the serial one's result
    /// bit for bit: sizes narrower than a strip, odd sizes, radii past the image, every channel
    /// count up to RGBA plus alpha.
    #[test]
    fn the_parallel_blur_matches_the_serial_one_bit_for_bit() {
        for (w, h) in [(1, 1), (3, 2), (7, 5), (9, 31), (37, 11), (130, 17), (257, 64)] {
            for n in 1..=5 {
                let src: Vec<f32> = (0..w * h * n).map(|i| ((i * 7919 + 13) % 1009) as f32 / 1008.0 - 0.1).collect();
                for sigma in [0.6f32, 2.5, 9.0, 140.0] {
                    let mut serial = src.clone();
                    gauss_blur_n(&mut serial, w, h, n, sigma);
                    let mut par = src.clone();
                    assert!(try_gauss_blur_n(&mut par, w, h, n, sigma).is_some());
                    let same = serial.iter().zip(&par).all(|(a, b)| a.to_bits() == b.to_bits());
                    assert!(same, "{w}x{h}x{n} sigma {sigma}");
                }
                // Strips of every width, down to one column.
                for strip in [1, 2, 5, w] {
                    let (mut serial, mut par) = (src.clone(), src.clone());
                    box_cols_n(&mut serial, w, h, n, 3);
                    let mut tmp = vec![0.0; par.len()];
                    par_box_cols_n(&mut par, &mut tmp, w, h, n, 3, strip);
                    assert!(serial.iter().zip(&par).all(|(a, b)| a.to_bits() == b.to_bits()), "{w}x{h}x{n} strip {strip}");
                }
            }
        }
    }

    /// The pre-split row pass, kept verbatim as the reference: one clamped accumulator per
    /// channel, stepped with `clamp(x + r + 1) - clamp(x - r)`.
    fn reference_line(line: &[f32], w: usize, n: usize, r: usize) -> Vec<f32> {
        let norm = 1.0 / (2 * r + 1) as f64;
        let at = |x: isize, c: usize| line[(x.clamp(0, w as isize - 1) as usize) * n + c] as f64;
        let mut out = vec![0.0f32; w * n];
        for c in 0..n {
            let mut acc: f64 = (-(r as isize)..=r as isize).map(|x| at(x, c)).sum();
            for x in 0..w {
                out[x * n + c] = (acc * norm) as f32;
                acc += at(x as isize + r as isize + 1, c) - at(x as isize - r as isize, c);
            }
        }
        out
    }

    /// The bytes of each sample: `-0.0` and `+0.0` compare equal as `f32`, so the differential
    /// checks below compare bit patterns rather than values.
    fn bits(v: &[f32]) -> Vec<u32> {
        v.iter().map(|x| x.to_bits()).collect()
    }

    fn line(w: usize, n: usize, seed: u32) -> Vec<f32> {
        let mut s = seed | 1;
        (0..w * n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                (s >> 8) as f32 / 16_777_216.0
            })
            .collect()
    }

    /// The split row pass is bit-identical to the clamped one it replaced, including the widths
    /// (and radii) where the line is shorter than the window and no plain-index middle exists.
    #[test]
    fn split_row_pass_matches_the_clamped_reference() {
        let mut scratch = Vec::new();
        // `-0.0 == 0.0` as `f32`; comparing bits is what makes the check below fail on a signed-zero
        // regression, so keep the helper rather than a plain `assert_eq!`.
        assert_ne!(bits(&[-0.0]), bits(&[0.0]));
        for r in 1..=50usize {
            for w in [1usize, 2, 3, 5, 7, 13, 32, 64, r.saturating_sub(1).max(1), r, r + 1, r + 2] {
                for n in 1..=4usize {
                    let src = line(w, n, (r * 1013 + w * 31 + n) as u32);
                    let mut got = src.clone();
                    box_line_n(&mut got, w, n, r, &mut scratch);
                    assert_eq!(bits(&got), bits(&reference_line(&src, w, n, r)), "w {w} n {n} r {r}");
                }
            }
        }
    }

    /// A whole buffer, several rows, and a trailing partial row left alone as before.
    #[test]
    fn split_row_pass_matches_the_clamped_reference_over_a_buffer() {
        for (w, h, n, r) in [(9usize, 4usize, 3usize, 2usize), (1, 3, 4, 7), (24, 5, 1, 24), (6, 6, 2, 50)] {
            let src = line(w * h, n, (w * 7 + h * 13 + r) as u32);
            let mut got = src.clone();
            box_rows_n(&mut got, w, n, r);
            for row in 0..h {
                let a = &src[row * w * n..(row + 1) * w * n];
                let b = &got[row * w * n..(row + 1) * w * n];
                assert_eq!(bits(b), bits(&reference_line(a, w, n, r)), "w {w} h {h} n {n} r {r} row {row}");
            }
        }
    }

    /// A buffer with structure, so a wrong tap shows up and not just a flat field.
    fn ramp(w: usize, h: usize, n: usize) -> Vec<f32> {
        (0..w * h * n).map(|i| ((i * 37 % 251) as f32) / 251.0).collect()
    }

    /// The strip gather must produce the strided walk's bytes: same taps, same order.
    #[test]
    fn cols_match_the_strided_walk() {
        for (w, h, n, r) in [(1, 1, 1, 1), (7, 5, 2, 2), (324, 324, 2, 9), (61, 1122, 4, 79), (0, 4, 2, 1), (5, 0, 2, 1), (33, 17, 4, 0), (9, 9, 4, 100)] {
            let mut a = ramp(w, h, n);
            let mut b = a.clone();
            box_cols_n(&mut a, w, h, n, r);
            box_cols_strided(&mut b, w, h, n, r);
            let bits = |v: &Vec<f32>| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
            assert_eq!(bits(&a), bits(&b), "{w}x{h}x{n} r{r} diverged");
        }
        // Every radius 1 to 50 at a width the gather spans in several strips (19 columns, runs of
        // 3), and at a height shorter than the radius: the same bytes as the strided walk.
        for r in 1..=50usize {
            for (w, h, n) in [(19usize, 1122usize, 4usize), (13, 3, 1), (7, 1, 2)] {
                let mut a = ramp(w, h, n);
                let mut b = a.clone();
                box_cols_n(&mut a, w, h, n, r);
                box_cols_strided(&mut b, w, h, n, r);
                let bits = |v: &Vec<f32>| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
                assert_eq!(bits(&a), bits(&b), "{w}x{h}x{n} r{r} diverged");
            }
        }
    }

    /// `cargo test -p photocraft-algo --release --lib -- --nocapture cols_timing`
    #[test]
    fn cols_timing() {
        for (w, h, n, r) in [(324, 324, 2, 9), (1122, 1122, 2, 79), (1122, 786, 2, 79)] {
            // Each pass gets its own copy of the same input: a pass run after the other would
            // otherwise measure a buffer the other one already rewrote.
            let input = ramp(w, h, n);
            let mut out = [0.0f64; 2];
            for (k, f) in [(0usize, box_cols_strided as fn(&mut [f32], usize, usize, usize, usize)), (1, box_cols_n)] {
                let mut buf = input.clone();
                let t = std::time::Instant::now();
                for _ in 0..5 {
                    f(&mut buf, w, h, n, r);
                }
                out[k] = t.elapsed().as_secs_f64() * 200.0;
            }
            println!("{w}x{h}x{n} r{r}: strided {:.3} ms, strip {:.3} ms ({:.2}x)", out[0], out[1], out[0] / out[1]);
        }
    }
}
