//! Edit › Content-Aware Fill: patch-based completion ([`crate::inpaint::complete_masked`]) with
//! Photoshop's sampling area, colour adaptation and geometric adaptation.
//!
//! Photoshop 25.4 was measured as a black box (synthetic and public-domain images, results read
//! back exactly; none of its code or assets were used):
//!
//! * **Sampling window** ([`sampling_window`]): the fill copies only from a square centred on the
//!   selection's bounding box, of side `4·√(max(w, 50)·max(h, 50))`: four times a square
//!   selection's side, 200 px at least, and for a long thin selection four times the geometric
//!   mean of its sides. At the canvas edge the square slides inwards rather than being cut. Pixels
//!   outside it never change the result (Edit › Fill and the workspace's Rectangular area alike).
//! * **Auto sampling area** ([`auto_sampling`]): the workspace's default keeps, within that
//!   square, only what looks like the selection's surroundings: a region of another brightness or
//!   another texture is left out along its edges, wherever it lies (a bright band through the
//!   window is excluded, the matching area beyond it is not).
//! * **The fill** copies pieces of the sampling area pixel for pixel (15–40 px across), joined by
//!   seams a few pixels wide: see [`crate::inpaint`].
//! * **Colour adaptation** None / Default / High / Very High: with None the copies are exact; the
//!   others let each copied patch take a gain and a bias towards its surroundings, more at each
//!   level ([`color_level`]). The patch transform is that of S. Darabi et al., *Image Melding*,
//!   SIGGRAPH 2012.
//! * **Rotation / scale / mirror adaptation**: the sampling window is extended with rotated,
//!   rescaled and mirrored copies of itself, laid out side by side (separated by gaps that are no
//!   image at all), so the patch search can pick transformed source patches, as in the
//!   generalised PatchMatch of C. Barnes, E. Shechtman, D. B. Goldman, A. Finkelstein, *The
//!   Generalized PatchMatch Correspondence Algorithm*, ECCV 2010 (which searches over rotations
//!   and scales; we enumerate a small set instead).
//!
//! Buffers are interleaved `w × h × ch` normalised floats (any model/depth); deterministic.

use photocraft_geom::Rect;

use crate::inpaint::{Adapt, CompleteParams, Px, complete_masked};
use crate::poisson::membrane_fill_with;

/// Options for [`fill`].
#[derive(Clone, Debug, PartialEq)]
pub struct FillOptions {
    /// Colour adaptation of the copies ([`color_level`]).
    pub color_adaptation: Adapt,
    /// Extra source rotations in degrees (Photoshop's rotation adaptation levels).
    pub rotations: Vec<f32>,
    /// Add downscaled and upscaled source copies.
    pub scale: bool,
    /// Add a horizontally mirrored source copy.
    pub mirror: bool,
    pub seed: u64,
}

impl Default for FillOptions {
    fn default() -> Self {
        Self { color_adaptation: color_level("default"), rotations: Vec::new(), scale: false, mirror: false, seed: 1 }
    }
}

/// Rotation set for Photoshop's adaptation levels: none, low, medium, high, full.
pub fn rotation_level(level: &str) -> Vec<f32> {
    match level {
        "low" => vec![-10.0, 10.0],
        "medium" => vec![-20.0, 20.0],
        "high" => vec![-35.0, 35.0],
        "full" => vec![90.0, 180.0, 270.0],
        _ => Vec::new(),
    }
}

/// Colour adaptation for Photoshop's levels: none, default, high, veryHigh. Measured on a texture
/// under a brightness ramp: with None the copies are exact; Default shifts them by up to about
/// 10/255, High about 20, Very High about 35, scaling their contrast a little more at each level.
pub fn color_level(level: &str) -> Adapt {
    match level {
        "none" => Adapt::NONE,
        "high" => Adapt { gain: (0.8, 1.25), bias: 0.08 },
        "veryHigh" => Adapt { gain: (0.67, 1.5), bias: 0.14 },
        _ => Adapt { gain: (0.9, 1.1), bias: 0.04 },
    }
}

/// Photoshop's sampling window for a selection with bounding box `hole`: the square of side
/// `4·√(max(w, 50)·max(h, 50))` centred on it, slid inside `canvas` (and clipped to it only where
/// the canvas is smaller than the square). A selection longer than that square (more than 16
/// times as long as it is wide, which was not measured) keeps its whole length.
pub fn sampling_window(hole: Rect, canvas: Rect) -> Rect {
    if hole.is_empty() || canvas.is_empty() {
        return Rect::EMPTY;
    }
    let (w, h) = (f64::from(hole.width().max(50)), f64::from(hole.height().max(50)));
    let side = (4.0 * (w * h).sqrt()).round().min(i32::MAX as f64) as i64;
    // Twice the centre, so that odd sizes stay exact.
    let span = |c2: i64, extent: u32, lo: i32, hi: i32| -> (i32, i32) {
        let (lo, hi) = (i64::from(lo), i64::from(hi));
        let side = side.max(i64::from(extent));
        if side >= hi - lo {
            return (lo as i32, hi as i32);
        }
        let a = ((c2 - side) / 2).clamp(lo, hi - side);
        (a as i32, (a + side) as i32)
    };
    let (x0, x1) = span(i64::from(hole.x0) + i64::from(hole.x1), hole.width(), canvas.x0, canvas.x1);
    let (y0, y1) = span(i64::from(hole.y0) + i64::from(hole.y1), hole.height(), canvas.y0, canvas.y1);
    Rect::new(x0, y0, x1, y1)
}

/// One source variant: pixels and which of them are usable.
struct Variant {
    w: usize,
    h: usize,
    img: Vec<f32>,
    ok: Vec<bool>,
}

/// A source buffer and which of its pixels may be sampled.
#[derive(Clone, Copy)]
struct Src<'a> {
    w: usize,
    h: usize,
    ch: usize,
    img: &'a [f32],
    ok: &'a [bool],
}

/// Sample the source bilinearly at (x, y); false when any tap is unusable.
fn sample(src: Src, x: f32, y: f32, out: &mut [f32]) -> bool {
    let Src { w, h, ch, img, ok } = src;
    if x < 0.0 || y < 0.0 || x > (w - 1) as f32 || y > (h - 1) as f32 {
        return false;
    }
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (tx, ty) = (x - x0 as f32, y - y0 as f32);
    for (xx, yy) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
        if !ok[yy * w + xx] {
            return false;
        }
    }
    for (c, o) in out.iter_mut().enumerate().take(ch) {
        let p = |xx: usize, yy: usize| img[(yy * w + xx) * ch + c];
        let top = p(x0, y0) + (p(x1, y0) - p(x0, y0)) * tx;
        let bot = p(x0, y1) + (p(x1, y1) - p(x0, y1)) * tx;
        *o = top + (bot - top) * ty;
    }
    true
}

/// Transformed copy of the source: rotation (degrees) and scale about the centre, optional mirror.
fn variant(src: Src, deg: f32, scale: f32, mirror: bool) -> Variant {
    let Src { w, h, ch, .. } = src;
    let (s, c) = (-deg.to_radians()).sin_cos();
    // Output size: the rotated, scaled bounding box.
    let (fw, fh) = (w as f32 * scale, h as f32 * scale);
    let ow = ((fw * c.abs() + fh * s.abs()).round() as usize).max(1);
    let oh = ((fw * s.abs() + fh * c.abs()).round() as usize).max(1);
    let (icx, icy) = ((w as f32 - 1.0) / 2.0, (h as f32 - 1.0) / 2.0);
    let (ocx, ocy) = ((ow as f32 - 1.0) / 2.0, (oh as f32 - 1.0) / 2.0);
    let mut out = Variant { w: ow, h: oh, img: vec![0.0; ow * oh * ch], ok: vec![false; ow * oh] };
    let mut px = vec![0.0f32; ch];
    for y in 0..oh {
        for x in 0..ow {
            let (dx, dy) = ((x as f32 - ocx) / scale, (y as f32 - ocy) / scale);
            let mut sx = icx + dx * c - dy * s;
            let sy = icy + dx * s + dy * c;
            if mirror {
                sx = (w as f32 - 1.0) - sx;
            }
            if sample(src, sx, sy, &mut px) {
                let i = y * ow + x;
                out.ok[i] = true;
                out.img[i * ch..(i + 1) * ch].copy_from_slice(&px);
            }
        }
    }
    out
}

/// Fill `hole` in `img`, copying only from pixels where `source` is true (and not in the hole).
/// Returns the full buffer (unchanged outside the hole). Falls back to the membrane fill when
/// nothing can be sampled.
pub fn fill(w: usize, h: usize, ch: usize, img: &[f32], hole: &[bool], source: &[bool], opts: &FillOptions) -> Vec<f32> {
    // Never cancelled; the fallback (the input unchanged) is unreachable.
    fill_with(w, h, ch, img, hole, source, opts, &photocraft_raster::Interrupt::NONE).unwrap_or_else(|_| img.to_vec())
}

/// [`fill`] that can be cancelled (see [`crate::inpaint::complete_with`]) and reports progress.
#[allow(clippy::too_many_arguments)]
pub fn fill_with(
    w: usize,
    h: usize,
    ch: usize,
    img: &[f32],
    hole: &[bool],
    source: &[bool],
    opts: &FillOptions,
    ctl: &photocraft_raster::Interrupt,
) -> Result<Vec<f32>, photocraft_raster::Cancelled> {
    assert_eq!(img.len(), w * h * ch);
    assert_eq!(hole.len(), w * h);
    assert_eq!(source.len(), w * h);
    if !hole.iter().any(|h| *h) {
        return Ok(img.to_vec());
    }
    let params = CompleteParams { seed: opts.seed, adapt: opts.color_adaptation, ..Default::default() };
    let gap = 2 * params.patch_radius + 2;
    let ok: Vec<bool> = hole.iter().zip(source).map(|(h, s)| !h && *s).collect();
    let src = Src { w, h, ch, img, ok: &ok };
    // Variants: the identity (which holds the hole) first, then the adaptations.
    let mut extra: Vec<Variant> = Vec::new();
    if opts.mirror {
        extra.push(variant(src, 0.0, 1.0, true));
    }
    for &deg in &opts.rotations {
        extra.push(variant(src, deg, 1.0, false));
    }
    if opts.scale {
        extra.push(variant(src, 0.0, 0.8, false));
        extra.push(variant(src, 0.0, 1.25, false));
    }
    // Mosaic: identity at the left, variants in a column to its right, gaps that are no image.
    let col_w = extra.iter().map(|v| v.w).max().unwrap_or(0);
    let col_h: usize = extra.iter().map(|v| v.h + gap).sum();
    let mw = if extra.is_empty() { w } else { w + gap + col_w };
    let mh = h.max(col_h);
    let mut mimg = vec![0.0f32; mw * mh * ch];
    let mut roles = vec![Px::Void; mw * mh];
    for y in 0..h {
        for x in 0..w {
            let (s, d) = (y * w + x, y * mw + x);
            mimg[d * ch..(d + 1) * ch].copy_from_slice(&img[s * ch..(s + 1) * ch]);
            roles[d] = if hole[s] {
                Px::Hole
            } else if ok[s] {
                Px::Source
            } else {
                Px::Known
            };
        }
    }
    let mut oy = 0;
    for v in &extra {
        for y in 0..v.h {
            for x in 0..v.w {
                let (s, d) = (y * v.w + x, (oy + y) * mw + w + gap + x);
                mimg[d * ch..(d + 1) * ch].copy_from_slice(&v.img[s * ch..(s + 1) * ch]);
                if v.ok[s] {
                    roles[d] = Px::Source;
                }
            }
        }
        oy += v.h + gap;
    }
    ctl.check()?;
    // The completion is nearly all the work.
    let cancel = || ctl.cancelled();
    let progress = |f: f32| ctl.progress(f * 0.99);
    let filled = complete_masked(mw, mh, ch, &mimg, &roles, &params, &photocraft_raster::Interrupt::new(&cancel, &progress))?;
    let mut out = img.to_vec();
    match filled {
        Some(f) => {
            for y in 0..h {
                for x in 0..w {
                    if hole[y * w + x] {
                        let (s, d) = ((y * mw + x) * ch, (y * w + x) * ch);
                        out[d..d + ch].copy_from_slice(&f[s..s + ch]);
                    }
                }
            }
        }
        None => out = membrane_fill_with(w, h, ch, img, hole, ctl)?,
    }
    ctl.check()?;
    ctl.progress(1.0);
    Ok(out)
}

/// Box mean of radius `r` over a `w × h` plane (edge-clamped), by summed areas.
fn box_mean(w: usize, h: usize, v: &[f32], r: usize) -> Vec<f32> {
    let w1 = w + 1;
    let mut s = vec![0.0f64; w1 * (h + 1)];
    for y in 0..h {
        let mut row = 0.0f64;
        for x in 0..w {
            row += f64::from(v[y * w + x]);
            s[(y + 1) * w1 + x + 1] = s[y * w1 + x + 1] + row;
        }
    }
    let mut out = vec![0.0f32; w * h];
    for y in 0..h {
        let (y0, y1) = (y.saturating_sub(r), (y + r + 1).min(h));
        for x in 0..w {
            let (x0, x1) = (x.saturating_sub(r), (x + r + 1).min(w));
            let sum = s[y1 * w1 + x1] + s[y0 * w1 + x0] - s[y0 * w1 + x1] - s[y1 * w1 + x0];
            out[y * w + x] = (sum / ((x1 - x0) * (y1 - y0)) as f64) as f32;
        }
    }
    out
}

/// Largest side [`auto_sampling`] works at: bigger windows are segmented at a reduced scale.
const AUTO_SIDE: usize = 384;

/// Content-Aware Fill's Auto sampling area within a window (the whole `w × h` buffer): the pixels
/// that look like the hole's surroundings. Each pixel is described by its local mean colour and
/// local contrast (7 × 7); the ring around the hole is summarised by a few clusters of those; a
/// pixel near a cluster prefers to be sampled, one far from all of them not, and one min cut
/// (contrast-sensitive, so the boundary follows edges) settles the area. Returns the mask of
/// pixels that may be copied from (never the hole). Falls back to everything outside the hole
/// when the area would be too small to copy from.
pub fn auto_sampling(w: usize, h: usize, ch: usize, img: &[f32], hole: &[bool]) -> Vec<bool> {
    use crate::segment::{FREE, HARD_FG, RgbImage, grid_cut_with};
    assert_eq!(img.len(), w * h * ch);
    assert_eq!(hole.len(), w * h);
    let everything: Vec<bool> = hole.iter().map(|h| !h).collect();
    if w == 0 || h == 0 || ch == 0 || !hole.iter().any(|h| *h) {
        return everything;
    }
    // Work at a reduced scale for big windows (the area is smooth at this scale).
    let k = w.max(h).div_ceil(AUTO_SIDE).max(1);
    let (sw, sh) = (w.div_ceil(k), h.div_ceil(k));
    let nc = ch.min(3);
    let mut planes = vec![vec![0.0f32; sw * sh]; nc];
    let mut shole = vec![false; sw * sh];
    let mut cnt = vec![0.0f32; sw * sh];
    for y in 0..h {
        for x in 0..w {
            let i = (y / k) * sw + x / k;
            let p = y * w + x;
            if hole[p] {
                shole[i] = true;
            }
            for (c, pl) in planes.iter_mut().enumerate() {
                pl[i] += img[p * ch + c];
            }
            cnt[i] += 1.0;
        }
    }
    for pl in &mut planes {
        pl.iter_mut().zip(&cnt).for_each(|(v, n)| *v /= n.max(1.0));
    }
    // Features: local means and local contrast (of the luma-ish average of the colour planes).
    let r = 3;
    let means: Vec<Vec<f32>> = planes.iter().map(|p| box_mean(sw, sh, p, r)).collect();
    let luma: Vec<f32> = (0..sw * sh).map(|i| planes.iter().map(|p| p[i]).sum::<f32>() / nc as f32).collect();
    let luma2: Vec<f32> = luma.iter().map(|v| v * v).collect();
    let (lm, lm2) = (box_mean(sw, sh, &luma, r), box_mean(sw, sh, &luma2, r));
    let contrast: Vec<f32> = lm.iter().zip(&lm2).map(|(m, m2)| (m2 - m * m).max(0.0).sqrt()).collect();
    let nf = nc + 1;
    let feat = |i: usize, out: &mut [f32]| {
        for (c, m) in means.iter().enumerate() {
            out[c] = m[i];
        }
        out[nc] = 2.0 * contrast[i];
    };
    // The ring: outside the hole, within a few pixels of it.
    let dist = crate::selection::edt(&shole, sw, sh);
    let ring_w = 6.0f32 / k as f32 + 1.0;
    let ring: Vec<usize> = (0..sw * sh).filter(|&i| !shole[i] && dist[i] <= ring_w).collect();
    if ring.is_empty() {
        return everything;
    }
    // A few cluster centres of the ring's features (k-means, deterministic start).
    let kc = 6.min(ring.len());
    let mut f = vec![0.0f32; nf];
    let mut centres: Vec<Vec<f32>> = (0..kc)
        .map(|j| {
            feat(ring[j * ring.len() / kc], &mut f);
            f.clone()
        })
        .collect();
    let d2 = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f32>();
    for _ in 0..8 {
        let mut acc = vec![vec![0.0f32; nf]; kc];
        let mut n = vec![0usize; kc];
        for &i in &ring {
            feat(i, &mut f);
            let j = (0..kc).min_by(|&a, &b| d2(&f, &centres[a]).total_cmp(&d2(&f, &centres[b]))).unwrap_or(0);
            acc[j].iter_mut().zip(&f).for_each(|(a, v)| *a += v);
            n[j] += 1;
        }
        for j in 0..kc {
            if n[j] > 0 {
                centres[j] = acc[j].iter().map(|v| v / n[j] as f32).collect();
            }
        }
    }
    let nearest = |i: usize, f: &mut [f32]| -> f32 {
        feat(i, f);
        centres.iter().map(|c| d2(f, c)).fold(f32::INFINITY, f32::min).sqrt()
    };
    // How much the ring itself varies around its clusters sets the scale of "similar".
    let mut spread: Vec<f32> = ring.iter().map(|&i| nearest(i, &mut f)).collect();
    let q = (spread.len() * 9 / 10).min(spread.len() - 1);
    let (_, s90, _) = spread.select_nth_unstable_by(q, |a, b| a.total_cmp(b));
    let tau = (*s90 * 1.6).max(0.03);
    let mut cost_fg = vec![0.0f32; sw * sh];
    let mut cost_bg = vec![0.0f32; sw * sh];
    let mut fixed = vec![FREE; sw * sh];
    for i in 0..sw * sh {
        if shole[i] || dist[i] <= 1.5 {
            fixed[i] = HARD_FG;
            continue;
        }
        let d = nearest(i, &mut f) / tau;
        cost_fg[i] = d * d;
        cost_bg[i] = 1.0;
    }
    let rgb = RgbImage::from_fn(sw, sh, |x, y| {
        let i = y * sw + x;
        let c = |j: usize| means[j.min(nc - 1)][i];
        [c(0), c(1), c(2)]
    });
    // Contrast-sensitive smoothness (the weight of a squared colour difference). The scale of
    // "similar" and this weight were fitted to Photoshop's Auto overlays (mean IoU 0.92 on ten
    // synthetic and public-domain cases).
    let area = grid_cut_with(&rgb, &cost_fg, &cost_bg, &fixed, |d2| 8.0 * (-d2 / (2.0 * 0.05 * 0.05)).exp() + 0.05);
    let mut out = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            out[i] = !hole[i] && area[(y / k) * sw + x / k];
        }
    }
    // Too little to copy from (fewer source pixels than twice the hole): sample everything.
    let n_hole = hole.iter().filter(|h| **h).count();
    if out.iter().filter(|o| **o).count() < 2 * n_hole.max(64) {
        return everything;
    }
    out
}

/// The Patch Tool's and the Content-Aware Move Tool's Color option (0–10) as the colour adaptation
/// of the copied content. Measured on Photoshop 25.4 with content brighter than its new place: 0
/// keeps it as it is; 3 moves it about 0.12 (32 of 42 levels, and 31 of 102: a limit, not a
/// fraction), 6 about 80 % of 42 levels, 10 most of the way (35 of 42, 95 of 102). So the shift is
/// limited to about `0.04 · color`, the contrast change likewise widening.
pub fn color_amount(color: u8) -> Adapt {
    if color == 0 {
        return Adapt::NONE;
    }
    let c = f32::from(color.min(10));
    Adapt { gain: (1.0 - 0.033 * c, 1.0 + 0.05 * c), bias: 0.04 * c }
}

/// Gain and bias per channel taking the samples `from` to the level and contrast of the samples
/// `to` (both interleaved, `ch` per sample), within `a`: how copied content is fitted to its new
/// surroundings.
pub fn level_transform(ch: usize, from: &[f32], to: &[f32], a: Adapt) -> Vec<(f32, f32)> {
    let moments = |v: &[f32], c: usize| {
        let (mut s, mut s2, mut n) = (0.0f64, 0.0f64, 0.0f64);
        for x in v.chunks_exact(ch.max(1)) {
            s += f64::from(x[c]);
            s2 += f64::from(x[c]) * f64::from(x[c]);
            n += 1.0;
        }
        (s, s2, n)
    };
    (0..ch)
        .map(|c| {
            let (fs, fs2, fnn) = moments(from, c);
            let (ts, ts2, tn) = moments(to, c);
            if a.is_none() || fnn == 0.0 || tn == 0.0 {
                return (1.0, 0.0);
            }
            // Both sets scaled to the same count for the fit.
            let k = fnn / tn;
            crate::inpaint::fit(a, fnn as f32, (ts * k) as f32, (ts2 * k) as f32, fs as f32, fs2 as f32)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vertical stripes (period 6) in grey, with a hole in the middle.
    fn stripes(w: usize, h: usize) -> (Vec<f32>, Vec<bool>) {
        let img: Vec<f32> = (0..w * h).map(|i| if (i % w) % 6 < 3 { 0.2 } else { 0.8 }).collect();
        let hole: Vec<bool> = (0..w * h).map(|i| (i % w).abs_diff(w / 2) < 4 && (i / w).abs_diff(h / 2) < 4).collect();
        (img, hole)
    }

    #[test]
    fn cancelled_fill_stops_and_progress_reaches_one() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let (w, h) = (60, 40);
        let (img, hole) = stripes(w, h);
        let source = vec![true; w * h];
        let yes = || true;
        let cancelled = photocraft_raster::Interrupt::cancel_only(&yes);
        assert!(fill_with(w, h, 1, &img, &hole, &source, &FillOptions::default(), &cancelled).is_err());
        let last = AtomicU32::new(0);
        let no = || false;
        let progress = |f: f32| {
            assert!(f >= f32::from_bits(last.load(Ordering::Relaxed)), "progress went backwards");
            last.store(f.to_bits(), Ordering::Relaxed);
        };
        let out = fill_with(w, h, 1, &img, &hole, &source, &FillOptions::default(), &photocraft_raster::Interrupt::new(&no, &progress)).unwrap();
        assert_eq!(f32::from_bits(last.load(Ordering::Relaxed)), 1.0);
        assert_eq!(out, fill(w, h, 1, &img, &hole, &source, &FillOptions::default()), "same result as the plain fill");
    }

    #[test]
    fn fills_only_the_hole_with_known_texture() {
        let (w, h) = (48, 32);
        let (img, hole) = stripes(w, h);
        let src = vec![true; w * h];
        let out = fill(w, h, 1, &img, &hole, &src, &FillOptions { color_adaptation: Adapt::NONE, ..Default::default() });
        for i in 0..w * h {
            if !hole[i] {
                assert_eq!(out[i], img[i]);
            }
        }
        // The hole is filled with stripe values, not a flat average.
        let vals: Vec<f32> = (0..w * h).filter(|i| hole[*i]).map(|i| out[i]).collect();
        assert!(vals.iter().any(|v| *v < 0.35) && vals.iter().any(|v| *v > 0.65), "{vals:?}");
    }

    #[test]
    fn sampling_area_restricts_sources() {
        // Left half dark, right half bright; sampling only the right half fills bright.
        let (w, h) = (40, 20);
        let img: Vec<f32> = (0..w * h).map(|i| if i % w < 20 { 0.1 } else { 0.9 }).collect();
        let hole: Vec<bool> = (0..w * h).map(|i| (16..24).contains(&(i % w)) && (6..14).contains(&(i / w))).collect();
        let right: Vec<bool> = (0..w * h).map(|i| i % w >= 24).collect();
        let out = fill(w, h, 1, &img, &hole, &right, &FillOptions { color_adaptation: Adapt::NONE, ..Default::default() });
        let mean: f32 = (0..w * h).filter(|i| hole[*i]).map(|i| out[i]).sum::<f32>() / 64.0;
        assert!(mean > 0.8, "mean {mean}");
    }

    #[test]
    fn adaptations_run_and_stay_deterministic() {
        let (w, h) = (32, 24);
        let (img, hole) = stripes(w, h);
        let rgb: Vec<f32> = img.iter().flat_map(|v| [*v, *v * 0.5, 1.0 - *v]).collect();
        let src = vec![true; w * h];
        let opts = FillOptions { color_adaptation: color_level("high"), rotations: rotation_level("low"), scale: true, mirror: true, seed: 7 };
        let a = fill(w, h, 3, &rgb, &hole, &src, &opts);
        let b = fill(w, h, 3, &rgb, &hole, &src, &opts);
        assert_eq!(a, b);
        assert!(a.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
    }

    #[test]
    fn color_adaptation_follows_a_gradient() {
        // A horizontal brightness ramp with a hole: adaptation keeps the fill near the ramp.
        let (w, h) = (40, 20);
        let img: Vec<f32> = (0..w * h).map(|i| (i % w) as f32 / w as f32).collect();
        let hole: Vec<bool> = (0..w * h).map(|i| (14..26).contains(&(i % w)) && (5..15).contains(&(i / w))).collect();
        let src = vec![true; w * h];
        let out = fill(w, h, 1, &img, &hole, &src, &FillOptions { color_adaptation: color_level("veryHigh"), ..Default::default() });
        let err: f32 = (0..w * h).filter(|i| hole[*i]).map(|i| (out[i] - img[i]).abs()).sum::<f32>() / 120.0;
        assert!(err < 0.12, "mean error {err}");
    }

    /// I.i.d. noise: every value occurs once, so a filled pixel equal to an image pixel was copied.
    fn noise(w: usize, h: usize, seed: u64) -> Vec<f32> {
        let mut z = seed;
        (0..w * h * 3)
            .map(|_| {
                z = z.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                0.2 + 0.6 * ((z >> 40) as f32 / (1u64 << 24) as f32)
            })
            .collect()
    }

    #[test]
    fn sampling_window_is_photoshops() {
        let canvas = Rect::new(0, 0, 800, 800);
        let sq = |x0: i32, y0: i32, w: i32, h: i32| Rect::new(x0, y0, x0 + w, y0 + h);
        // Photoshop 25.4's sampling overlay: a square of side 4·√(max(w,50)·max(h,50)) centred on
        // the selection's bounds.
        assert_eq!(sampling_window(sq(375, 375, 50, 50), canvas), sq(300, 300, 200, 200));
        assert_eq!(sampling_window(sq(350, 350, 100, 100), canvas), sq(200, 200, 400, 400));
        assert_eq!(sampling_window(sq(395, 395, 10, 10), canvas), sq(300, 300, 200, 200));
        assert_eq!(sampling_window(sq(370, 370, 60, 60), canvas), sq(280, 280, 240, 240));
        // Long selections (Photoshop's overlay, read off the screen to a pixel: 281, 399, 691).
        assert_eq!(sampling_window(sq(350, 385, 100, 30), canvas), sq(258, 258, 283, 283));
        assert_eq!(sampling_window(sq(300, 375, 200, 50), canvas), sq(200, 200, 400, 400));
        assert_eq!(sampling_window(sq(250, 350, 300, 100), canvas), sq(53, 53, 693, 693));
        // At the canvas edge the square slides inwards rather than being cut.
        assert_eq!(sampling_window(sq(10, 375, 50, 50), canvas), sq(0, 300, 200, 200));
        assert_eq!(sampling_window(sq(5, 5, 60, 60), canvas), sq(0, 0, 240, 240));
        // A canvas smaller than the square clips it; a very long selection keeps its length.
        assert_eq!(sampling_window(sq(40, 40, 50, 50), Rect::new(0, 0, 150, 120)), Rect::new(0, 0, 150, 120));
        assert_eq!(sampling_window(sq(100, 995, 1800, 10), Rect::new(0, 0, 2000, 2000)), sq(100, 400, 1800, 1200));
        assert_eq!(sampling_window(Rect::EMPTY, canvas), Rect::EMPTY);
    }

    #[test]
    fn auto_sampling_keeps_what_looks_like_the_surroundings() {
        // Grey texture, dark on the left, 50 levels brighter on the right; the hole is in the dark
        // part near the boundary (Photoshop's Auto leaves the bright part out).
        let (w, h) = (200, 200);
        let tex = noise(w, h, 3);
        let shade = |x: usize, band: bool| {
            if band {
                if (130..150).contains(&x) { 0.55 } else { 0.35 }
            } else if x < 135 {
                0.35
            } else {
                0.55
            }
        };
        for band in [false, true] {
            let img: Vec<f32> = (0..w * h * 3).map(|i| shade((i / 3) % w, band) + (tex[i] - 0.5) * 0.1).collect();
            let hole: Vec<bool> = (0..w * h).map(|i| (75..125).contains(&(i % w)) && (75..125).contains(&(i / w))).collect();
            let area = auto_sampling(w, h, 3, &img, &hole);
            let frac = |x0: usize, x1: usize| {
                let n = (0..w * h).filter(|i| (x0..x1).contains(&(i % w)) && !hole[*i]).count();
                (0..w * h).filter(|i| (x0..x1).contains(&(i % w)) && area[*i]).count() as f32 / n as f32
            };
            assert!(hole.iter().zip(&area).all(|(hl, a)| !(*hl && *a)), "never the hole");
            assert!(frac(0, 125) > 0.95, "the dark side is sampled ({})", frac(0, 125));
            if band {
                // A bright band through the window is left out; the dark area beyond it is not.
                assert!(frac(133, 147) < 0.05, "the band is excluded ({})", frac(133, 147));
                assert!(frac(155, 200) > 0.9, "beyond the band is sampled ({})", frac(155, 200));
            } else {
                assert!(frac(140, 200) < 0.05, "the bright side is excluded ({})", frac(140, 200));
            }
        }
    }

    #[test]
    fn without_colour_adaptation_the_fill_copies_pixels_exactly() {
        // A smooth texture with fine noise, so that every value is unique: a filled pixel equal to
        // an image pixel was copied from it. Photoshop's fill (colour adaptation None) is such
        // copies, joined by narrow seams.
        let (w, h) = (120, 120);
        let fine = noise(w, h, 9);
        let img: Vec<f32> = (0..w * h * 3)
            .map(|i| {
                let (x, y, c) = (((i / 3) % w) as f32, ((i / 3) / w) as f32, i % 3);
                0.5 + 0.2 * (x * 0.21 + c as f32).sin() * (y * 0.17).cos() + 0.1 * (x * 0.05 - y * 0.08).sin() + (fine[i] - 0.5) * 0.08
            })
            .collect();
        let hole: Vec<bool> = (0..w * h).map(|i| (40..80).contains(&(i % w)) && (40..80).contains(&(i / w))).collect();
        // Votes of identical copies average to the copy up to float rounding.
        let key = |v: &[f32], i: usize| [0, 1, 2].map(|c| (v[i * 3 + c] * 1e5).round() as i64);
        let known: std::collections::HashSet<[i64; 3]> = (0..w * h).filter(|i| !hole[*i]).map(|i| key(&img, i)).collect();
        let out = fill(w, h, 3, &img, &hole, &vec![true; w * h], &FillOptions { color_adaptation: Adapt::NONE, ..Default::default() });
        let exact = (0..w * h).filter(|i| hole[*i] && known.contains(&key(&out, *i))).count();
        assert!(exact as f32 > 0.2 * 1600.0, "{exact} of 1600 filled pixels are exact copies");
        // And the fill is as busy as the noise around it, not a blur of it.
        let lap = |v: &[f32], x: usize, y: usize| {
            let p = |x: usize, y: usize| v[(y * w + x) * 3];
            (4.0 * p(x, y) - p(x - 1, y) - p(x + 1, y) - p(x, y - 1) - p(x, y + 1)).abs()
        };
        let mean = |inside: bool| {
            let pts: Vec<(usize, usize)> = (1..h - 1).flat_map(|y| (1..w - 1).map(move |x| (x, y))).filter(|&(x, y)| hole[y * w + x] == inside).collect();
            pts.iter().map(|&(x, y)| lap(&out, x, y)).sum::<f32>() / pts.len() as f32
        };
        assert!(mean(true) > 0.6 * mean(false), "fill detail {} against {}", mean(true), mean(false));
    }

    #[test]
    fn patch_color_shifts_up_to_about_four_hundredths_per_step() {
        // Content 0.4 brighter than where it lands (Photoshop's Patch, Color 3 and 10).
        let from: Vec<f32> = (0..300).map(|i| 0.7 + 0.05 * ((i % 7) as f32 / 7.0)).collect();
        let to: Vec<f32> = from.iter().map(|v| v - 0.4).collect();
        let level = from.iter().sum::<f32>() / 300.0;
        let shift = |c: u8| {
            let gb = level_transform(1, &from, &to, color_amount(c));
            level - from.iter().map(|v| gb[0].0 * v + gb[0].1).sum::<f32>() / 300.0
        };
        assert_eq!(shift(0), 0.0);
        assert!((shift(3) - 0.12).abs() < 0.01, "{}", shift(3));
        assert!((shift(10) - 0.4).abs() < 0.01, "{}", shift(10));
    }

    #[test]
    fn variant_geometry() {
        let img = vec![0.5f32; 10 * 6];
        let ok = vec![true; 60];
        let src = Src { w: 10, h: 6, ch: 1, img: &img, ok: &ok };
        let v = variant(src, 90.0, 1.0, false);
        assert_eq!((v.w, v.h), (6, 10));
        let v = variant(src, 0.0, 0.8, true);
        assert_eq!((v.w, v.h), (8, 5));
        assert!(v.ok.iter().filter(|o| **o).count() > 20);
    }
}
