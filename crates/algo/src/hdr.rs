//! File › Automate › Merge to HDR Pro: exposure estimation, alignment, camera response recovery,
//! radiance merge, ghost removal and the 16/8-bit tone mapping methods.
//!
//! * **Alignment**: G. Ward, *Fast, Robust Image Registration for Compositing High Dynamic Range
//!   Photographs from Hand-Held Exposures*, JGT 2003 — median threshold bitmaps (invariant to
//!   exposure) with an exclusion band around the median, XOR-counted over an image pyramid.
//! * **Response curve**: P. Debevec, J. Malik, *Recovering High Dynamic Range Radiance Maps from
//!   Photographs*, SIGGRAPH 1997 — `g(Z) = ln E + ln Δt` solved by weighted linear least squares
//!   with a smoothness term and `g(128) = 0`, per channel, on stratified sample pixels.
//! * **Merge**: the hat-weighted average of `g(Z) − ln Δt` over the exposures.
//! * **Ghosts**: where the per-exposure radiance estimates disagree (high weighted variance of
//!   their logarithms) the result comes from one base exposure only, as Photoshop's *Remove
//!   ghosts* does; the mask is feathered.
//! * **Tone mapping** for 16/8-bit output: Local Adaptation ([`crate::tone::hdr_toning`]),
//!   Exposure and Gamma, Highlight Compression (E. Reinhard et al., *Photographic Tone
//!   Reproduction for Digital Images*, SIGGRAPH 2002, global operator) and Equalize Histogram
//!   (histogram equalisation of log luminance, after G. Ward Larson et al., *A Visibility
//!   Matching Tone Reproduction Operator*, TVCG 1997, without the ceiling).
//!
//! Pixels are straight RGBA in `0..=1` (display-encoded inputs); radiance is linear.

#![allow(clippy::needless_range_loop)] // index loops mirror the maths (pixels × channels)

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::photo_util::{Normal, linear_to_srgb, par_map, par_rows, par_rows2, srgb_to_linear};
use crate::tone::HdrToning;

/// Pixels per parallel chunk of the element-wise passes (64 KiB of RGBA f32: big enough to
/// amortise the scheduling, small enough to balance 16+ threads on a small image).
const PX_CHUNK: usize = 4096;

fn luma(c: [f32; 4]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// Hat weight on a 0..=255 code value.
fn hat(z: f64) -> f64 {
    if z <= 127.5 { z } else { 255.0 - z }.max(0.0)
}

/// Relative exposures (linear factors, the darkest = 1) estimated from the images themselves:
/// consecutive images (sorted by brightness) are compared on pixels well exposed in both.
pub fn estimate_exposures(imgs: &[&[[f32; 4]]]) -> Vec<f64> {
    let n = imgs.len();
    let mean: Vec<f64> = imgs.iter().map(|im| im.iter().map(|p| luma(*p) as f64).sum::<f64>() / im.len().max(1) as f64).collect();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| mean[a].total_cmp(&mean[b]));
    let mut ex = vec![1.0f64; n];
    for k in 1..n {
        let (a, b) = (order[k - 1], order[k]);
        let mut ratios: Vec<f64> = imgs[a]
            .iter()
            .zip(imgs[b].iter())
            .step_by(7)
            .filter_map(|(pa, pb)| {
                let (la, lb) = (luma(*pa), luma(*pb));
                ((0.08..0.92).contains(&la) && (0.08..0.92).contains(&lb)).then(|| srgb_to_linear(lb) as f64 / srgb_to_linear(la).max(1e-6) as f64)
            })
            .collect();
        ratios.sort_by(f64::total_cmp);
        let r = if ratios.is_empty() { 2.0 } else { ratios[ratios.len() / 2].max(1.0) };
        ex[b] = ex[a] * r;
    }
    ex
}

// ---------- alignment ----------

fn shrink(w: usize, h: usize, g: &[f32]) -> (usize, usize, Vec<f32>) {
    let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
    let mut out = vec![0.0f32; nw * nh];
    for y in 0..nh {
        for x in 0..nw {
            let p = |xx: usize, yy: usize| g[yy.min(h - 1) * w + xx.min(w - 1)];
            out[y * nw + x] = 0.25 * (p(2 * x, 2 * y) + p(2 * x + 1, 2 * y) + p(2 * x, 2 * y + 1) + p(2 * x + 1, 2 * y + 1));
        }
    }
    (nw, nh, out)
}

fn bitmaps(g: &[f32]) -> (Vec<bool>, Vec<bool>) {
    let mut s: Vec<f32> = g.to_vec();
    s.sort_by(f32::total_cmp);
    let med = s[s.len() / 2];
    (g.iter().map(|v| *v > med).collect(), g.iter().map(|v| (v - med).abs() > 4.0 / 255.0).collect())
}

/// Translation `(dx, dy)` that aligns grey image `img` to `reference` (Ward's MTB), searching
/// up to `2^levels` px.
pub fn mtb_offset(w: usize, h: usize, reference: &[f32], img: &[f32], levels: usize) -> (i32, i32) {
    if levels == 0 || w < 16 || h < 16 {
        return (0, 0);
    }
    let (cur_x, cur_y) = {
        let (sw, sh, sr) = shrink(w, h, reference);
        let (_, _, si) = shrink(w, h, img);
        let (x, y) = mtb_offset(sw, sh, &sr, &si, levels - 1);
        (x * 2, y * 2)
    };
    let (tr, er) = bitmaps(reference);
    let (ti, ei) = bitmaps(img);
    let mut best = (u64::MAX, 0, 0);
    for dy in -1..=1 {
        for dx in -1..=1 {
            let (ox, oy) = (cur_x + dx, cur_y + dy);
            let mut err = 0u64;
            for y in 0..h as i32 {
                let sy = y - oy;
                if sy < 0 || sy >= h as i32 {
                    continue;
                }
                for x in 0..w as i32 {
                    let sx = x - ox;
                    if sx < 0 || sx >= w as i32 {
                        continue;
                    }
                    let (a, b) = ((y as usize) * w + x as usize, (sy as usize) * w + sx as usize);
                    if (tr[a] != ti[b]) && er[a] && ei[b] {
                        err += 1;
                    }
                }
            }
            if err < best.0 {
                best = (err, ox, oy);
            }
        }
    }
    (best.1, best.2)
}

// ---------- response curve ----------

/// Sample pixel indices for response recovery: a spatial grid filtered to pixels whose
/// neighbourhood is flat in the middle exposure (robust to small misalignment), stratified by
/// its intensity.
pub fn sample_pixels(w: usize, h: usize, middle: &[[f32; 4]], count: usize) -> Vec<usize> {
    let mut by_bin: Vec<Vec<usize>> = vec![Vec::new(); 32];
    let step = ((w * h / (count * 40).max(1)) as f64).sqrt().max(1.0) as usize;
    for y in (2..h.saturating_sub(2)).step_by(step) {
        for x in (2..w.saturating_sub(2)).step_by(step) {
            let i = y * w + x;
            let l = luma(middle[i]);
            let flat = [i - 2, i + 2, i - 2 * w, i + 2 * w].iter().all(|&j| (luma(middle[j]) - l).abs() < 0.04);
            if flat {
                by_bin[((l * 31.99) as usize).min(31)].push(i);
            }
        }
    }
    // Round-robin over the bins so every intensity is represented.
    let mut out = Vec::new();
    let mut k = 0;
    while out.len() < count && by_bin.iter().any(|b| k < b.len()) {
        for b in &by_bin {
            if let Some(&i) = b.get(k * 7 % b.len().max(1)).filter(|_| k < b.len()) {
                out.push(i);
                if out.len() >= count {
                    break;
                }
            }
        }
        k += 1;
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Debevec–Malik response curve: `z[j][i]` is the code value (0..=255) of sample `i` in exposure
/// `j`; returns `g(0..=255)` (log exposure per code value, `g(128) = 0`), smoothed by `lambda`.
pub fn response_curve(z: &[Vec<u8>], log_dt: &[f64], lambda: f64) -> Vec<f64> {
    let p = z.len();
    let n = z.first().map_or(0, Vec::len);
    let mut ne = Normal::new(256 + n);
    for j in 0..p {
        for i in 0..n {
            let zz = z[j][i] as usize;
            let w = hat(zz as f64) + 1.0;
            ne.add(&[(zz, 1.0), (256 + i, -1.0)], log_dt[j], w);
        }
    }
    ne.add(&[(128, 1.0)], 0.0, 100.0);
    for k in 1..255 {
        let w = lambda.sqrt() * (hat(k as f64) + 1.0);
        ne.add(&[(k - 1, 1.0), (k, -2.0), (k + 1, 1.0)], 0.0, w);
    }
    let Some(x) = ne.solve() else {
        // Fall back to the sRGB curve.
        return (0..256).map(|k| (srgb_to_linear(k as f32 / 255.0).max(1e-5) as f64).ln() - (srgb_to_linear(128.0 / 255.0) as f64).ln()).collect();
    };
    let mut g: Vec<f64> = x[..256].to_vec();
    // Enforce monotonicity (noise at the extremes can fold the curve).
    for k in (0..128).rev() {
        g[k] = g[k].min(g[k + 1] - 1e-4);
    }
    for k in 129..256 {
        g[k] = g[k].max(g[k - 1] + 1e-4);
    }
    g
}

fn g_at(g: &[f64], v: f32) -> f64 {
    let x = (v.clamp(0.0, 1.0) * 255.0) as f64;
    // `x` is never negative here, so truncating is `floor`; the cast avoids a libm `floor` call
    // per lookup (no SSE4.1 in the baseline x86-64 target). NaN gives 0 either way.
    let i = (x as usize).min(254);
    let t = x - i as f64;
    g[i] * (1.0 - t) + g[i + 1] * t
}

/// Merge options.
#[derive(Clone, Debug)]
pub struct MergeOptions {
    /// Linear exposure factors (time × gain), one per image.
    pub exposures: Vec<f64>,
    pub remove_ghosts: bool,
    /// Exposure used where ghosts are removed (`None`: the one with most well-exposed pixels).
    pub ghost_base: Option<usize>,
    /// Response curves per channel (`None`: recovered from the images).
    pub response: Option<[Vec<f64>; 3]>,
}

/// Result of [`merge`].
#[derive(Clone, Debug)]
pub struct Merged {
    /// Linear radiance, straight RGBA.
    pub px: Vec<[f32; 4]>,
    pub response: [Vec<f64>; 3],
    pub ghost_base: usize,
    /// Fraction of pixels handled as ghosts.
    pub ghost_fraction: f64,
    /// Dynamic range of the result in stops (1st..99.9th percentile of luminance).
    pub stops: f64,
}

/// Merges aligned exposures (`w × h` each) into a radiance map, scaled so the base exposure's
/// well-exposed pixels keep their linear values.
pub fn merge(w: usize, h: usize, imgs: &[&[[f32; 4]]], opts: &MergeOptions) -> Merged {
    let p = imgs.len();
    let log_dt: Vec<f64> = opts.exposures.iter().map(|e| e.max(1e-12).ln()).collect();
    // Counts are integers, so splitting them over chunks (a multiple of the step, so every chunk
    // samples the same pixels) cannot change the total.
    let well = |im: &[[f32; 4]]| -> usize {
        const CH: usize = 3 * 4096;
        par_map(im.len().div_ceil(CH), |c| im[c * CH..(c * CH + CH).min(im.len())].iter().step_by(3).filter(|q| (0.1..0.9).contains(&luma(**q))).count())
            .iter()
            .sum()
    };
    let base = opts.ghost_base.filter(|b| *b < p).unwrap_or_else(|| (0..p).max_by_key(|&j| well(imgs[j])).unwrap_or(0));
    let response = opts.response.clone().unwrap_or_else(|| {
        let mut order: Vec<usize> = (0..p).collect();
        order.sort_by(|&a, &b| opts.exposures[a].total_cmp(&opts.exposures[b]));
        let mid = order[p / 2];
        let samples = sample_pixels(w, h, imgs[mid], 160);
        // The three channels' curves are independent least-squares solves: one per thread.
        let mut curves = par_map(3, |c| {
            let z: Vec<Vec<u8>> = imgs.iter().map(|im| samples.iter().map(|&i| (im[i][c].clamp(0.0, 1.0) * 255.0).round() as u8).collect()).collect();
            response_curve(&z, &log_dt, 40.0)
        })
        .into_iter();
        std::array::from_fn(|_| curves.next().unwrap_or_default())
    });
    let mut px = vec![[0.0f32; 4]; w * h];
    let resp = &response;
    par_rows(&mut px, w, 1, |y, row| {
        for (x, out) in row.iter_mut().enumerate() {
            let i = y * w + x;
            let mut alpha = 0.0f32;
            for c in 0..3 {
                let (mut num, mut den) = (0.0f64, 0.0f64);
                for j in 0..p {
                    let v = imgs[j][i][c];
                    let wt = hat(v.clamp(0.0, 1.0) as f64 * 255.0);
                    num += wt * (g_at(&resp[c], v) - log_dt[j]);
                    den += wt;
                }
                let le = if den > 1e-6 {
                    num / den
                } else {
                    // Saturated everywhere: darkest exposure if bright, brightest if dark.
                    let bright = imgs.iter().map(|im| im[i][c]).sum::<f32>() / p as f32 > 0.5;
                    let j = (0..p).min_by(|&a, &b| if bright { log_dt[a].total_cmp(&log_dt[b]) } else { log_dt[b].total_cmp(&log_dt[a]) }).unwrap_or(0);
                    g_at(&resp[c], imgs[j][i][c]) - log_dt[j]
                };
                out[c] = le.exp() as f32;
            }
            for im in imgs {
                alpha = alpha.max(im[i][3]);
            }
            out[3] = alpha;
        }
    });
    let mut ghost_fraction = 0.0;
    if opts.remove_ghosts && p > 1 {
        // Weighted variance of per-exposure log radiance (luminance), thresholded straight into
        // the 0/1 ghost mask (the intermediate score map is not needed again).
        let mut ghost: Vec<f32> = par_map(w * h, |i| {
            let (mut s, mut s2, mut sw) = (0.0f64, 0.0f64, 0.0f64);
            for j in 0..p {
                let q = imgs[j][i];
                let l = luma(q);
                let wt = hat(l as f64 * 255.0);
                if wt < 20.0 {
                    continue;
                }
                let lr = (0..3).map(|c| 0.33 * (g_at(&response[c], q[c]) - log_dt[j])).sum::<f64>();
                s += wt * lr;
                s2 += wt * lr * lr;
                sw += wt;
            }
            if sw <= 0.0 {
                return 0.0;
            }
            let m = s / sw;
            if ((s2 / sw - m * m).max(0.0)).sqrt() as f32 > 0.35 { 1.0 } else { 0.0 }
        });
        // Grow and feather the mask.
        let r = (w.max(h) as f32 / 200.0).max(2.0);
        crate::fxutil::gauss_blur_n(&mut ghost, w, h, 1, r);
        // Sharpen the mask, count the ghost pixels and blend the base exposure in, one pass over
        // the rows. The count is an integer, so adding the per-row counts in any order is exact.
        let ld = log_dt[base];
        let ghosts = AtomicUsize::new(0);
        par_rows2(&mut px, &mut ghost, w, |y, prow, grow| {
            let mut count = 0usize;
            for (x, (out, g)) in prow.iter_mut().zip(grow.iter_mut()).enumerate() {
                *g = (*g * 3.0).min(1.0);
                count += (*g > 0.5) as usize;
                if *g <= 0.0 {
                    continue;
                }
                let q = imgs[base][y * w + x];
                for c in 0..3 {
                    let e = (g_at(&response[c], q[c]) - ld).exp() as f32;
                    out[c] = out[c] * (1.0 - *g) + e * *g;
                }
            }
            ghosts.fetch_add(count, Ordering::Relaxed);
        });
        ghost_fraction = ghosts.into_inner() as f64 / (w * h).max(1) as f64;
    }
    // Scale per channel so the base exposure's well-exposed pixels map to their linear values.
    // The median of the sampled ratios is all that is needed, so it is selected rather than
    // sorted (the same element, in O(n)), and the three channels' ratios are computed in parallel.
    let scale: [f64; 3] = std::array::from_fn(|c| {
        let samples = par_map((w * h).div_ceil(37), |s| {
            let i = s * 37;
            let v = imgs[base][i][c];
            ((0.15..0.85).contains(&v) && px[i][c] > 0.0).then(|| srgb_to_linear(v) as f64 / (px[i][c] as f64 * opts.exposures[base]))
        });
        let mut r: Vec<f64> = samples.into_iter().flatten().collect();
        let mid = r.len() / 2;
        (if r.is_empty() { 1.0 } else { *r.select_nth_unstable_by(mid, f64::total_cmp).1 }) * opts.exposures[base]
    });
    // Channels do not read each other, so the three scalings are one pass (each `scale[c]` was
    // measured from the unscaled values of its own channel, as when the channels ran in turn).
    par_rows(&mut px, PX_CHUNK, 1, |_, chunk| {
        for q in chunk.iter_mut() {
            for c in 0..3 {
                q[c] = (q[c] as f64 * scale[c]) as f32;
            }
        }
    });
    // 99.9th / 1st percentile of the sampled luminance. Chunks are a multiple of the step so each
    // samples the same pixels; the order of the samples does not matter to a percentile.
    const STOPS_CH: usize = 13 * 1024;
    let mut l: Vec<f32> = par_map(px.len().div_ceil(STOPS_CH), |c| {
        px[c * STOPS_CH..(c * STOPS_CH + STOPS_CH).min(px.len())].iter().step_by(13).filter(|q| q[3] > 0.0).map(|q| luma(*q).max(1e-9)).collect::<Vec<f32>>()
    })
    .concat();
    let stops = if l.is_empty() {
        0.0
    } else {
        let (hi_i, lo_i) = ((l.len() as f64 * 0.999) as usize % l.len(), l.len() / 100);
        // Two order statistics by selection (the elements a full sort would put there; equal
        // elements under `total_cmp` are the same bits) instead of sorting ~2 million samples.
        let (hi, lo) = if lo_i < hi_i {
            let (below, hi, _) = l.select_nth_unstable_by(hi_i, f32::total_cmp);
            let hi = *hi;
            (hi, *below.select_nth_unstable_by(lo_i, f32::total_cmp).1)
        } else {
            l.sort_by(f32::total_cmp);
            (l[hi_i], l[lo_i])
        };
        (hi as f64 / lo as f64).log2()
    };
    Merged { px, response, ghost_base: base, ghost_fraction, stops }
}

// ---------- tone mapping ----------

/// The four 16/8-bit conversion methods of Merge to HDR Pro / HDR Toning.
#[derive(Clone, Debug, PartialEq)]
pub enum ToneMethod {
    LocalAdaptation(HdrToning),
    ExposureGamma { exposure: f32, gamma: f32 },
    HighlightCompression,
    EqualizeHistogram,
}

/// Pixels per block when the log-mean luminance is summed: each block's logarithms are computed
/// in parallel, then added in pixel order.
const LOG_BLOCK: usize = 1 << 16;
/// Pixels per chunk of the Equalize Histogram passes (per-chunk integer histograms and extremes).
const HIST_CHUNK: usize = 1 << 14;

/// Geometric mean of the luminance over the pixels with alpha above 0 (the scene's "key").
///
/// The `ln` of each pixel is the expensive part, so it is computed in parallel, block by block;
/// the terms are then added strictly in pixel order, so the f64 sum is the very sum the serial
/// loop made (a parallel reduction would add in a different order and round differently). A
/// skipped pixel contributes `-0.0`, which leaves any f64 sum unchanged, bit for bit.
fn log_mean_luma(px: &[[f32; 4]]) -> f32 {
    let mut acc: f64 = std::iter::empty::<f64>().sum();
    let mut n = 0usize;
    // One buffer for every block (no allocation per block); each parallel chunk fills its slice.
    let mut terms = vec![0.0f64; LOG_BLOCK.min(px.len())];
    for block in px.chunks(LOG_BLOCK) {
        let terms = &mut terms[..block.len()];
        par_rows(terms, PX_CHUNK, 1, |ci, out| {
            for (t, q) in out.iter_mut().zip(&block[ci * PX_CHUNK..]) {
                *t = if q[3] > 0.0 { (luma(*q).max(1e-6) as f64).ln() } else { -0.0 };
            }
        });
        for (q, t) in block.iter().zip(terms.iter()) {
            n += (q[3] > 0.0) as usize;
            acc += *t;
        }
    }
    (acc / n.max(1) as f64).exp() as f32
}

/// Tone maps linear radiance to display-encoded (sRGB curve) values in `0..=1`.
pub fn tone_map(px: &mut [[f32; 4]], w: usize, h: usize, method: &ToneMethod) {
    match method {
        ToneMethod::LocalAdaptation(p) => {
            // Key the scene to middle grey, compress in log luminance, then encode.
            let k = 0.18 / log_mean_luma(px).max(1e-6);
            par_rows(px, PX_CHUNK, 1, |_, chunk| {
                for q in chunk.iter_mut() {
                    for c in 0..3 {
                        q[c] *= k;
                    }
                }
            });
            crate::tone::hdr_toning(px, w, h, p);
            par_rows(px, PX_CHUNK, 1, |_, chunk| {
                for q in chunk.iter_mut() {
                    for c in 0..3 {
                        q[c] = linear_to_srgb(q[c].clamp(0.0, 1.0));
                    }
                }
            });
        }
        ToneMethod::ExposureGamma { exposure, gamma } => {
            let k = 2f32.powf(*exposure);
            let g = gamma.clamp(0.1, 10.0);
            par_rows(px, PX_CHUNK, 1, |_, chunk| {
                for q in chunk.iter_mut() {
                    for c in 0..3 {
                        q[c] = linear_to_srgb((q[c] * k).clamp(0.0, 1.0)).powf(1.0 / g);
                    }
                }
            });
        }
        ToneMethod::HighlightCompression => {
            let k = 0.18 / log_mean_luma(px).max(1e-6);
            par_rows(px, PX_CHUNK, 1, |_, chunk| {
                for q in chunk.iter_mut() {
                    let l = luma(*q) * k;
                    let ld = l / (1.0 + l);
                    let r = if l > 1e-9 { ld / l * k } else { 0.0 };
                    for c in 0..3 {
                        q[c] = linear_to_srgb((q[c] * r).clamp(0.0, 1.0));
                    }
                }
            });
        }
        ToneMethod::EqualizeHistogram => {
            const BINS: usize = 1024;
            let lv: Vec<f32> = par_map(px.len(), |i| luma(px[i]).max(1e-7).ln());
            let chunks = px.len().div_ceil(HIST_CHUNK);
            let span_of = |c: usize| c * HIST_CHUNK..((c + 1) * HIST_CHUNK).min(px.len());
            // Min / max do not depend on the order they are taken in (no NaN or -0.0 can occur:
            // the values are `ln` of something clamped to at least 1e-7).
            let (lo, hi) = par_map(chunks, |c| {
                lv[span_of(c)].iter().zip(&px[span_of(c)]).filter(|(_, q)| q[3] > 0.0).fold((f32::MAX, f32::MIN), |(a, b), (v, _)| (a.min(*v), b.max(*v)))
            })
            .into_iter()
            .fold((f32::MAX, f32::MIN), |(a, b), (l, h)| (a.min(l), b.max(h)));
            let span = (hi - lo).max(1e-6);
            // Integer counts per chunk, added up: exact, and the f64 histogram the serial loop
            // built by adding 1.0 at a time held exactly these integers.
            let mut counts = vec![0u64; BINS];
            for part in par_map(chunks, |c| {
                let mut hist = vec![0u32; BINS];
                for (v, q) in lv[span_of(c)].iter().zip(&px[span_of(c)]) {
                    if q[3] > 0.0 {
                        hist[(((v - lo) / span) * (BINS - 1) as f32) as usize] += 1;
                    }
                }
                hist
            }) {
                for (t, n) in counts.iter_mut().zip(part) {
                    *t += n as u64;
                }
            }
            let hist: Vec<f64> = counts.iter().map(|&n| n as f64).collect();
            let total: f64 = hist.iter().sum::<f64>().max(1.0);
            let mut cdf = vec![0f32; BINS];
            let mut acc = 0.0;
            for b in 0..BINS {
                acc += hist[b];
                cdf[b] = (acc / total) as f32;
            }
            par_rows(px, PX_CHUNK, 1, |ci, chunk| {
                for (q, v) in chunk.iter_mut().zip(&lv[ci * PX_CHUNK..]) {
                    let target = cdf[(((v - lo) / span).clamp(0.0, 1.0) * (BINS - 1) as f32) as usize];
                    // Display value from the CDF (encoded), keep colour ratios in linear light.
                    let tl = srgb_to_linear(target);
                    let l = luma(*q).max(1e-7);
                    for c in 0..3 {
                        q[c] = linear_to_srgb((q[c] / l * tl).clamp(0.0, 1.0));
                    }
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A radiance map with a wide range, the camera's response a gamma-ish curve.
    fn scene(w: usize, h: usize) -> Vec<[f32; 3]> {
        (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as f32 / w as f32, (i / w) as f32 / h as f32);
                let base = 0.002 * 2f32.powf(x * 12.0); // 12 stops left to right
                let tex = 1.0 + 0.3 * ((y * 40.0).sin());
                [base * tex, base * tex * 0.8, base * tex * 0.6]
            })
            .collect()
    }

    fn camera(e: f32) -> f32 {
        // A smooth non-sRGB response.
        (e.max(0.0).powf(0.45)).min(1.0)
    }

    fn shoot(sc: &[[f32; 3]], dt: f32) -> Vec<[f32; 4]> {
        sc.iter().map(|r| [camera(r[0] * dt), camera(r[1] * dt), camera(r[2] * dt), 1.0]).map(|p| p.map(|v| (v * 255.0).round() / 255.0)).collect()
    }

    #[test]
    fn exposure_estimation_and_merge_recover_radiance() {
        let (w, h) = (192usize, 64usize);
        let sc = scene(w, h);
        let dts = [0.25f32, 1.0, 4.0, 16.0];
        let shots: Vec<Vec<[f32; 4]>> = dts.iter().map(|d| shoot(&sc, *d)).collect();
        let refs: Vec<&[[f32; 4]]> = shots.iter().map(Vec::as_slice).collect();
        let est = estimate_exposures(&refs);
        // Ratios between consecutive exposures are 4× (estimated through an sRGB guess, so loose).
        for k in 1..4 {
            let r = est[k] / est[k - 1];
            assert!((2.0..8.0).contains(&r), "ratio {r} ({est:?})");
        }
        let m =
            merge(w, h, &refs, &MergeOptions { exposures: dts.iter().map(|d| *d as f64).collect(), remove_ghosts: false, ghost_base: None, response: None });
        // Radiance proportional to the scene: check log-ratios across the 12-stop ramp.
        let probe = |x: usize| m.px[(h / 2) * w + x][1] as f64 / sc[(h / 2) * w + x][1] as f64;
        let k0 = probe(w / 2);
        // Within the bracket's range (brighter pixels clip in every exposure).
        for x in [10, 50, 100, 150, 170] {
            let r = probe(x) / k0;
            assert!((r.ln()).abs() < 0.25, "x {x}: ratio {r}");
        }
        assert!(m.stops > 9.0, "stops {}", m.stops);
        // The recovered response is monotonic.
        assert!(m.response[0].windows(2).all(|p| p[1] > p[0]));
    }

    #[test]
    fn ghosts_come_from_the_base_exposure() {
        let (w, h) = (64usize, 64usize);
        let sc: Vec<[f32; 3]> = vec![[0.2, 0.2, 0.2]; w * h];
        let mut shots: Vec<Vec<[f32; 4]>> = [0.5f32, 1.0, 2.0].iter().map(|d| shoot(&sc, *d)).collect();
        // A "person" only in the third exposure.
        for y in 20..40 {
            for x in 20..40 {
                shots[2][y * w + x] = [0.2, 0.2, 0.2, 1.0];
            }
        }
        let refs: Vec<&[[f32; 4]]> = shots.iter().map(Vec::as_slice).collect();
        let opts = |g| MergeOptions { exposures: vec![0.5, 1.0, 2.0], remove_ghosts: g, ghost_base: Some(1), response: None };
        let plain = merge(w, h, &refs, &opts(false));
        let clean = merge(w, h, &refs, &opts(true));
        let at = |m: &Merged, x: usize, y: usize| m.px[y * w + x][1];
        let bg = at(&clean, 5, 5);
        assert!((at(&clean, 30, 30) / bg - 1.0).abs() < 0.15, "ghost kept: {} vs {}", at(&clean, 30, 30), bg);
        assert!((at(&plain, 30, 30) / at(&plain, 5, 5) - 1.0).abs() > 0.2);
        assert!(clean.ghost_fraction > 0.05 && clean.ghost_base == 1);
    }

    #[test]
    fn mtb_finds_shifts() {
        let (w, h) = (128usize, 96usize);
        let img: Vec<f32> = (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as f32, (i / w) as f32);
                0.5 + 0.4 * ((x * 0.21).sin() * (y * 0.17).cos())
            })
            .collect();
        let (dx, dy) = (5i32, -3i32);
        let shifted: Vec<f32> = (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as i32 - dx, (i / w) as i32 - dy);
                // Darker exposure of the same scene, shifted.
                img[(y.clamp(0, h as i32 - 1) as usize) * w + x.clamp(0, w as i32 - 1) as usize] * 0.6
            })
            .collect();
        let (ox, oy) = mtb_offset(w, h, &shifted, &img, 4);
        assert_eq!((ox, oy), (dx, dy));
    }

    #[test]
    fn tone_mapping_methods_produce_display_range() {
        let (w, h) = (96usize, 32usize);
        let sc = scene(w, h);
        for method in [
            ToneMethod::LocalAdaptation(HdrToning { radius: 7.0, strength: 0.52, ..Default::default() }),
            ToneMethod::ExposureGamma { exposure: 0.0, gamma: 1.0 },
            ToneMethod::HighlightCompression,
            ToneMethod::EqualizeHistogram,
        ] {
            let mut px: Vec<[f32; 4]> = sc.iter().map(|r| [r[0], r[1], r[2], 1.0]).collect();
            tone_map(&mut px, w, h, &method);
            assert!(px.iter().all(|q| q[..3].iter().all(|v| (0.0..=1.0).contains(v))));
            let row = |x: usize| px[(h / 2) * w + x][1];
            assert!(row(w - 2) > row(2), "{method:?}: not increasing");
            if !matches!(method, ToneMethod::ExposureGamma { .. }) {
                // Compression keeps both ends of a 12-stop ramp visible.
                assert!(row(4) > 0.01 && row(w - 4) < 0.999, "{method:?}: {} {}", row(4), row(w - 4));
            }
        }
    }

    // ---- the parallel merge / tone mapping must stay bit for bit what the serial code gave ----

    fn mix(mut z: u64) -> u64 {
        z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Three exposures (×0.25, ×1, ×4) of a 12-stop-ish scene built from `+ - * /`, `sqrt` and
    /// `round` only (all exactly rounded, so the inputs are the same on every platform); a block
    /// in the third exposure shows something else (a ghost) and a few alpha holes differ per shot.
    fn exposures(w: usize, h: usize) -> Vec<Vec<[f32; 4]>> {
        [0.25f32, 1.0, 4.0]
            .iter()
            .enumerate()
            .map(|(k, dt)| {
                (0..w * h)
                    .map(|i| {
                        let (x, y) = (i % w, i / w);
                        let u = x as f32 / w as f32;
                        let m = mix(i as u64 ^ 0x5151);
                        let mut r = (0.002 + 0.25 * u * u * u * u) * (1.0 + 0.3 * (y % 16) as f32 / 16.0) * (0.9 + 0.2 * (m & 0xFFFF) as f32 / 65535.0);
                        if k == 2 && x > w / 3 && x < w / 3 + w / 10 + 1 && y > h / 4 && y < h / 4 + h / 8 + 1 {
                            r = 0.05 + 0.01 * ((x ^ y) & 7) as f32;
                        }
                        let cam = |v: f32| ((v * dt).clamp(0.0, 1.0).sqrt() * 255.0).round() / 255.0;
                        let a = match (k, mix(i as u64 ^ 0xA1FA) % 31) {
                            (0, 0) => 0.0,
                            (1, 1) => 0.5,
                            _ => 1.0,
                        };
                        [cam(r), cam(r * 0.8), cam(r * 0.6), a]
                    })
                    .collect()
            })
            .collect()
    }

    /// A radiance map with a wide range, alpha holes, fractional alpha and exact-zero pixels.
    fn radiance(w: usize, h: usize) -> Vec<[f32; 4]> {
        (0..w * h)
            .map(|i| {
                let u = (i % w) as f32 / w as f32;
                let m = mix(i as u64 ^ 0xA17A);
                let r = (0.002 + 6.0 * u * u * u * u) * (0.9 + 0.2 * (m & 0xFFFF) as f32 / 65535.0);
                let a = match m % 16 {
                    0 => 0.0,
                    1 => 0.5,
                    _ => 1.0,
                };
                if m % 97 == 5 { [0.0, 0.0, 0.0, 1.0] } else { [r, r * 0.8, r * 0.6, a] }
            })
            .collect()
    }

    fn all_methods() -> Vec<ToneMethod> {
        vec![
            ToneMethod::LocalAdaptation(HdrToning::default()),
            ToneMethod::LocalAdaptation(HdrToning {
                radius: 7.0,
                strength: 1.3,
                gamma: 0.8,
                exposure: 0.5,
                detail: -20.0,
                shadow: 30.0,
                highlight: 20.0,
                vibrance: 40.0,
                saturation: -10.0,
                curve: vec![(1.0, 1.0), (0.0, 0.1), (0.5, 0.4)],
            }),
            ToneMethod::ExposureGamma { exposure: 0.0, gamma: 1.0 },
            ToneMethod::ExposureGamma { exposure: 1.5, gamma: 2.2 },
            ToneMethod::HighlightCompression,
            ToneMethod::EqualizeHistogram,
        ]
    }

    fn fnv_px(px: &[[f32; 4]]) -> u64 {
        px.iter().flatten().fold(0xcbf2_9ce4_8422_2325u64, |h, v| (h ^ v.to_bits() as u64).wrapping_mul(0x0000_0100_0000_01b3))
    }

    /// Shapes: tiny, odd, one pixel wide / high, and larger than every chunk the parallel code
    /// uses (4096-pixel rows, 16 384-pixel histogram chunks, 65 536-pixel log-mean blocks).
    const SHAPES: [(usize, usize); 7] = [(1, 1), (97, 61), (1, 300), (300, 1), (257, 193), (300, 251), (64, 1100)];

    /// The serial code this module had before it was parallelised, verbatim (only the names are
    /// qualified). The new code must reproduce it exactly.
    mod reference {
        use super::super::*;

        /// `g_at` as it was, with the libm `floor` (the new one truncates).
        fn g_at(g: &[f64], v: f32) -> f64 {
            let x = (v.clamp(0.0, 1.0) * 255.0) as f64;
            let i = (x.floor() as usize).min(254);
            let t = x - i as f64;
            g[i] * (1.0 - t) + g[i + 1] * t
        }

        pub fn merge(w: usize, h: usize, imgs: &[&[[f32; 4]]], opts: &MergeOptions) -> Merged {
            let p = imgs.len();
            let log_dt: Vec<f64> = opts.exposures.iter().map(|e| e.max(1e-12).ln()).collect();
            let well = |im: &[[f32; 4]]| im.iter().step_by(3).filter(|q| (0.1..0.9).contains(&luma(**q))).count();
            let base = opts.ghost_base.filter(|b| *b < p).unwrap_or_else(|| (0..p).max_by_key(|&j| well(imgs[j])).unwrap_or(0));
            let response = opts.response.clone().unwrap_or_else(|| {
                let mut order: Vec<usize> = (0..p).collect();
                order.sort_by(|&a, &b| opts.exposures[a].total_cmp(&opts.exposures[b]));
                let mid = order[p / 2];
                let samples = sample_pixels(w, h, imgs[mid], 160);
                std::array::from_fn(|c| {
                    let z: Vec<Vec<u8>> = imgs.iter().map(|im| samples.iter().map(|&i| (im[i][c].clamp(0.0, 1.0) * 255.0).round() as u8).collect()).collect();
                    response_curve(&z, &log_dt, 40.0)
                })
            });
            let mut px = vec![[0.0f32; 4]; w * h];
            let mut ghost = vec![0.0f32; w * h];
            let resp = &response;
            par_rows(&mut px, w, 1, |y, row| {
                for (x, out) in row.iter_mut().enumerate() {
                    let i = y * w + x;
                    let mut alpha = 0.0f32;
                    for c in 0..3 {
                        let (mut num, mut den) = (0.0f64, 0.0f64);
                        for j in 0..p {
                            let v = imgs[j][i][c];
                            let wt = hat(v.clamp(0.0, 1.0) as f64 * 255.0);
                            num += wt * (g_at(&resp[c], v) - log_dt[j]);
                            den += wt;
                        }
                        let le = if den > 1e-6 {
                            num / den
                        } else {
                            let bright = imgs.iter().map(|im| im[i][c]).sum::<f32>() / p as f32 > 0.5;
                            let j = (0..p).min_by(|&a, &b| if bright { log_dt[a].total_cmp(&log_dt[b]) } else { log_dt[b].total_cmp(&log_dt[a]) }).unwrap_or(0);
                            g_at(&resp[c], imgs[j][i][c]) - log_dt[j]
                        };
                        out[c] = le.exp() as f32;
                    }
                    for im in imgs {
                        alpha = alpha.max(im[i][3]);
                    }
                    out[3] = alpha;
                }
            });
            let mut ghost_fraction = 0.0;
            if opts.remove_ghosts && p > 1 {
                let score: Vec<f32> = crate::photo_util::par_map(w * h, |i| {
                    let (mut s, mut s2, mut sw) = (0.0f64, 0.0f64, 0.0f64);
                    for j in 0..p {
                        let q = imgs[j][i];
                        let l = luma(q);
                        let wt = hat(l as f64 * 255.0);
                        if wt < 20.0 {
                            continue;
                        }
                        let lr = (0..3).map(|c| 0.33 * (g_at(&response[c], q[c]) - log_dt[j])).sum::<f64>();
                        s += wt * lr;
                        s2 += wt * lr * lr;
                        sw += wt;
                    }
                    if sw <= 0.0 {
                        return 0.0;
                    }
                    let m = s / sw;
                    ((s2 / sw - m * m).max(0.0)).sqrt() as f32
                });
                for i in 0..w * h {
                    ghost[i] = if score[i] > 0.35 { 1.0 } else { 0.0 };
                }
                let r = (w.max(h) as f32 / 200.0).max(2.0);
                crate::fxutil::gauss_blur_n(&mut ghost, w, h, 1, r);
                for g in ghost.iter_mut() {
                    *g = (*g * 3.0).min(1.0);
                }
                ghost_fraction = ghost.iter().filter(|g| **g > 0.5).count() as f64 / (w * h).max(1) as f64;
                let ld = log_dt[base];
                for i in 0..w * h {
                    if ghost[i] <= 0.0 {
                        continue;
                    }
                    let q = imgs[base][i];
                    for c in 0..3 {
                        let e = (g_at(&response[c], q[c]) - ld).exp() as f32;
                        px[i][c] = px[i][c] * (1.0 - ghost[i]) + e * ghost[i];
                    }
                }
            }
            for c in 0..3 {
                let mut r: Vec<f64> = (0..w * h)
                    .step_by(37)
                    .filter_map(|i| {
                        let v = imgs[base][i][c];
                        ((0.15..0.85).contains(&v) && px[i][c] > 0.0).then(|| srgb_to_linear(v) as f64 / (px[i][c] as f64 * opts.exposures[base]))
                    })
                    .collect();
                r.sort_by(f64::total_cmp);
                let k = if r.is_empty() { 1.0 } else { r[r.len() / 2] } * opts.exposures[base];
                for q in px.iter_mut() {
                    q[c] = (q[c] as f64 * k) as f32;
                }
            }
            let mut l: Vec<f32> = px.iter().step_by(13).filter(|q| q[3] > 0.0).map(|q| luma(*q).max(1e-9)).collect();
            l.sort_by(f32::total_cmp);
            let stops = if l.is_empty() { 0.0 } else { (l[(l.len() as f64 * 0.999) as usize % l.len()] as f64 / l[l.len() / 100] as f64).log2() };
            Merged { px, response, ghost_base: base, ghost_fraction, stops }
        }

        pub fn tone_map(px: &mut [[f32; 4]], w: usize, h: usize, method: &ToneMethod) {
            let n = px.iter().filter(|q| q[3] > 0.0).count().max(1);
            let log_mean = (px.iter().filter(|q| q[3] > 0.0).map(|q| (luma(*q).max(1e-6) as f64).ln()).sum::<f64>() / n as f64).exp() as f32;
            match method {
                ToneMethod::LocalAdaptation(p) => {
                    let k = 0.18 / log_mean.max(1e-6);
                    for q in px.iter_mut() {
                        for c in 0..3 {
                            q[c] *= k;
                        }
                    }
                    crate::tone::hdr_toning(px, w, h, p);
                    for q in px.iter_mut() {
                        for c in 0..3 {
                            q[c] = linear_to_srgb(q[c].clamp(0.0, 1.0));
                        }
                    }
                }
                ToneMethod::ExposureGamma { exposure, gamma } => {
                    let k = 2f32.powf(*exposure);
                    let g = gamma.clamp(0.1, 10.0);
                    for q in px.iter_mut() {
                        for c in 0..3 {
                            q[c] = linear_to_srgb((q[c] * k).clamp(0.0, 1.0)).powf(1.0 / g);
                        }
                    }
                }
                ToneMethod::HighlightCompression => {
                    let k = 0.18 / log_mean.max(1e-6);
                    for q in px.iter_mut() {
                        let l = luma(*q) * k;
                        let ld = l / (1.0 + l);
                        let r = if l > 1e-9 { ld / l * k } else { 0.0 };
                        for c in 0..3 {
                            q[c] = linear_to_srgb((q[c] * r).clamp(0.0, 1.0));
                        }
                    }
                }
                ToneMethod::EqualizeHistogram => {
                    const BINS: usize = 1024;
                    let lv: Vec<f32> = px.iter().map(|q| luma(*q).max(1e-7).ln()).collect();
                    let (lo, hi) = lv.iter().zip(px.iter()).filter(|(_, q)| q[3] > 0.0).fold((f32::MAX, f32::MIN), |(a, b), (v, _)| (a.min(*v), b.max(*v)));
                    let span = (hi - lo).max(1e-6);
                    let mut hist = vec![0f64; BINS];
                    for (v, q) in lv.iter().zip(px.iter()) {
                        if q[3] > 0.0 {
                            hist[(((v - lo) / span) * (BINS - 1) as f32) as usize] += 1.0;
                        }
                    }
                    let total: f64 = hist.iter().sum::<f64>().max(1.0);
                    let mut cdf = vec![0f32; BINS];
                    let mut acc = 0.0;
                    for b in 0..BINS {
                        acc += hist[b];
                        cdf[b] = (acc / total) as f32;
                    }
                    for (v, q) in lv.iter().zip(px.iter_mut()) {
                        let target = cdf[(((v - lo) / span).clamp(0.0, 1.0) * (BINS - 1) as f32) as usize];
                        let tl = srgb_to_linear(target);
                        let l = luma(*q).max(1e-7);
                        for c in 0..3 {
                            q[c] = linear_to_srgb((q[c] / l * tl).clamp(0.0, 1.0));
                        }
                    }
                }
            }
        }
    }

    fn merged_eq(a: &Merged, b: &Merged) -> bool {
        a.px.iter().flatten().map(|v| v.to_bits()).eq(b.px.iter().flatten().map(|v| v.to_bits()))
            && a.response.iter().flatten().map(|v| v.to_bits()).eq(b.response.iter().flatten().map(|v| v.to_bits()))
            && a.ghost_base == b.ghost_base
            && a.ghost_fraction.to_bits() == b.ghost_fraction.to_bits()
            && a.stops.to_bits() == b.stops.to_bits()
    }

    fn merge_opts(ghost: bool, base: Option<usize>) -> MergeOptions {
        MergeOptions { exposures: vec![0.25, 1.0, 4.0], remove_ghosts: ghost, ghost_base: base, response: None }
    }

    #[test]
    fn merge_is_bit_for_bit_the_serial_merge() {
        for (w, h) in SHAPES {
            let shots = exposures(w, h);
            let refs: Vec<&[[f32; 4]]> = shots.iter().map(Vec::as_slice).collect();
            for (ghost, base) in [(true, None), (true, Some(0)), (true, Some(2)), (false, None), (false, Some(1))] {
                let o = merge_opts(ghost, base);
                assert!(merged_eq(&merge(w, h, &refs, &o), &reference::merge(w, h, &refs, &o)), "merge {w}x{h} ghost={ghost} base={base:?}");
            }
            // One exposure and a supplied response curve take other branches.
            let o = MergeOptions { exposures: vec![1.0], remove_ghosts: true, ghost_base: None, response: None };
            assert!(merged_eq(&merge(w, h, &refs[1..2], &o), &reference::merge(w, h, &refs[1..2], &o)), "merge {w}x{h} one exposure");
        }
    }

    #[test]
    fn tone_map_is_bit_for_bit_the_serial_tone_map() {
        for (w, h) in SHAPES {
            let rad = radiance(w, h);
            let mut holes = rad.clone();
            holes.iter_mut().for_each(|q| q[3] = 0.0);
            let mut solid = rad.clone();
            solid.iter_mut().for_each(|q| q[3] = 1.0);
            for input in [&rad, &holes, &solid] {
                for m in all_methods() {
                    let (mut a, mut b) = (input.clone(), input.clone());
                    tone_map(&mut a, w, h, &m);
                    reference::tone_map(&mut b, w, h, &m);
                    assert!(a.iter().flatten().map(|v| v.to_bits()).eq(b.iter().flatten().map(|v| v.to_bits())), "tone_map {w}x{h} {m:?}");
                }
            }
        }
    }

    /// Hashes recorded from the serial code (Linux x86-64: they pass through `ln`, `exp` and
    /// `powf` of the platform's libm, which other systems may round differently in the last bit;
    /// the reference tests above cover those everywhere).
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    fn merge_and_tone_map_keep_their_recorded_output() {
        let mut got: Vec<(String, u64)> = Vec::new();
        for (w, h) in [(97usize, 61usize), (257, 193)] {
            let shots = exposures(w, h);
            let refs: Vec<&[[f32; 4]]> = shots.iter().map(Vec::as_slice).collect();
            for ghost in [true, false] {
                let m = merge(w, h, &refs, &merge_opts(ghost, None));
                if ghost && w == 257 {
                    assert!(m.ghost_fraction > 0.0, "the ghost block must trigger ghost removal");
                }
                got.push((format!("merge {w}x{h} ghost={ghost} px"), fnv_px(&m.px)));
                got.push((
                    format!("merge {w}x{h} ghost={ghost} frac+stops"),
                    m.ghost_fraction.to_bits() ^ m.stops.to_bits().rotate_left(17) ^ m.ghost_base as u64,
                ));
            }
            let rad = radiance(w, h);
            for (k, m) in all_methods().iter().enumerate() {
                let mut px = rad.clone();
                tone_map(&mut px, w, h, m);
                got.push((format!("tone_map {w}x{h} method#{k}"), fnv_px(&px)));
            }
        }
        const WANT: &[(&str, u64)] = &[
            ("merge 97x61 ghost=true px", 0x2393ce6027d29166),
            ("merge 97x61 ghost=true frac+stops", 0xa44cef3f08d0d67e),
            ("merge 97x61 ghost=false px", 0xd4915a0ee0b30436),
            ("merge 97x61 ghost=false frac+stops", 0x9bc7b8ca4dfe803b),
            ("tone_map 97x61 method#0", 0x2609976e79d893d7),
            ("tone_map 97x61 method#1", 0x0fbdc3db4ffc9ade),
            ("tone_map 97x61 method#2", 0xbd963b754b1afea8),
            ("tone_map 97x61 method#3", 0x0ae12e1bfca33874),
            ("tone_map 97x61 method#4", 0x7de30f4e8ff6f5a9),
            ("tone_map 97x61 method#5", 0x288e046dbe2e8693),
            ("merge 257x193 ghost=true px", 0xfd2139fec23995a3),
            ("merge 257x193 ghost=true frac+stops", 0xa928addc129911a7),
            ("merge 257x193 ghost=false px", 0xeeda918ea3e19d4c),
            ("merge 257x193 ghost=false frac+stops", 0x96b2919daa0a8038),
            ("tone_map 257x193 method#0", 0x6f74f611b9f91461),
            ("tone_map 257x193 method#1", 0x1ca836a89073a647),
            ("tone_map 257x193 method#2", 0x63df95071e47e326),
            ("tone_map 257x193 method#3", 0x50648599207f65fc),
            ("tone_map 257x193 method#4", 0xe7907d7de632689b),
            ("tone_map 257x193 method#5", 0x756fa52494f4da42),
        ];
        let got: Vec<(&str, u64)> = got.iter().map(|(n, v)| (n.as_str(), *v)).collect();
        assert_eq!(got, WANT);
    }
}
