//! Per-pixel and per-dab kernels for the toning and focus retouching tools: Dodge, Burn, Sponge,
//! Blur, Sharpen. The brush engine (`photocraft-paint`) decides *where* and *how strongly* each dab
//! applies; these functions decide *what* happens to the pixels.
//!
//! Tone curves are our own design, chosen to match the observed behaviour of Photoshop's tools:
//! the selected range is affected most, pure black and white stay put for Midtones, every curve
//! keeps tones in order at every strength, and "Protect Tones" works on luminance (moving luma
//! along the curve and scaling the colour around it as much as the curve stretches the colour's
//! own channel range, so hue is kept and a faint cast in the shadows can't grow into a saturated
//! colour; gamut compression replaces per-channel clipping).

use serde::{Deserialize, Serialize};

/// Tonal range targeted by Dodge/Burn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ToneRange {
    Shadows,
    #[default]
    Midtones,
    Highlights,
}

/// Rec. 601 luma of a straight RGB triple.
#[inline]
pub fn luma(c: [f32; 3]) -> f32 {
    0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]
}

/// Strength of the Shadows Dodge and Highlights Burn curves for exposure `e`. Their slope is
/// `1 − 3s·(…)²`, so `s` must stay at or below 1/3 for the curve to keep tones in order (#11526);
/// `e / (1 + 2e)` reaches 1/3 at full exposure and stays close to `e` at low exposure.
#[inline]
fn end_strength(e: f32) -> f32 {
    e / (1.0 + 2.0 * e)
}

/// The dodge/burn transfer for one value `v` (0..1) with strength `e` (0..1). Monotone in `v`.
#[inline]
pub fn tone_curve(v: f32, e: f32, range: ToneRange, burn: bool) -> f32 {
    let v = v.max(0.0);
    let e = e.clamp(0.0, 1.0);
    let u = v.min(1.0);
    match (range, burn) {
        // Gamma bend, weighted to peak at mid-grey: 0 and 1 are fixed points.
        (ToneRange::Midtones, _) => {
            let w = (4.0 * v * (1.0 - v)).clamp(0.0, 1.0);
            v + w * (u.powf(midtone_gamma(e, burn)) - v)
        }
        // Shadows / highlights: weight (1−v)² or v² times the distance to the target, so the chosen
        // end moves most (black lifts when dodging shadows, white drops when burning highlights).
        (ToneRange::Shadows, false) => v + end_strength(e) * (1.0 - u).powi(3),
        (ToneRange::Shadows, true) => v - e * (1.0 - u).powi(2) * v,
        (ToneRange::Highlights, false) => v + e * u.powi(2) * (1.0 - u),
        (ToneRange::Highlights, true) => v - end_strength(e) * u.powi(2) * v,
    }
}

#[inline]
fn midtone_gamma(e: f32, burn: bool) -> f32 {
    if burn { 1.0 + e } else { 1.0 / (1.0 + e) }
}

/// Bring an RGB triple back into 0..1 while keeping its luma (mix toward grey), instead of clipping
/// each channel (which would shift hue).
#[inline]
fn compress_gamut(c: [f32; 3], l: f32) -> [f32; 3] {
    let l = l.clamp(0.0, 1.0);
    let (lo, hi) = (c[0].min(c[1]).min(c[2]), c[0].max(c[1]).max(c[2]));
    let mut t = 1.0f32;
    if hi > 1.0 && hi - l > 1e-9 {
        t = t.min((1.0 - l) / (hi - l));
    }
    if lo < 0.0 && l - lo > 1e-9 {
        t = t.min(l / (l - lo));
    }
    [l + (c[0] - l) * t, l + (c[1] - l) * t, l + (c[2] - l) * t]
}

/// Dodge (`burn = false`) or Burn one straight-RGB colour with strength `e` (0..1).
///
/// With `protect_tones`, luma goes through the curve and the colour's offset from grey is scaled
/// by the curve's average slope across the colour's channel range (hue kept, gamut-compressed), so
/// the channels spread apart as far as the per-channel curve would spread them; otherwise each
/// channel goes through the curve. Scaling the colour by new ÷ old luma instead multiplied shadow
/// casts by up to 40× (#11522).
pub fn dodge_burn(c: [f32; 3], e: f32, range: ToneRange, burn: bool, protect_tones: bool) -> [f32; 3] {
    if e <= 0.0 {
        return c;
    }
    if protect_tones {
        let l = luma(c);
        let nl = tone_curve(l, e, range, burn);
        // Widened around luma so a grey pixel still gets the local slope instead of 0 ÷ 0. The
        // widening stops at 0 and 1, where the curve bends: past them its slope belongs to no
        // in-gamut tone, and averaging it in skewed near-black and near-white casts.
        let slope = |a: f32, b: f32| ((tone_curve(b, e, range, burn) - tone_curve(a, e, range, burn)) / (b - a)).max(0.0);
        let (cmin, cmax) = (c[0].min(c[1]).min(c[2]), c[0].max(c[1]).max(c[2]));
        let (lo, hi) = (cmin.min((l - 1e-3).max(0.0)), cmax.max((l + 1e-3).min(1.0)));
        let mut k = slope(lo, hi);
        // Where the curve steepens past a narrow cast, the widened slope would spread the cast
        // further than the per-channel curve does (up to 39% near black in Shadows Dodge).
        if (lo, hi) != (cmin, cmax) && cmax > cmin {
            k = k.min(slope(cmin, cmax));
        }
        compress_gamut([nl + (c[0] - l) * k, nl + (c[1] - l) * k, nl + (c[2] - l) * k], nl)
    } else {
        [tone_curve(c[0], e, range, burn), tone_curve(c[1], e, range, burn), tone_curve(c[2], e, range, burn)]
    }
}

/// HSV-style saturation (chroma / max) of an RGB triple.
#[inline]
pub fn saturation(c: [f32; 3]) -> f32 {
    let (lo, hi) = (c[0].min(c[1]).min(c[2]), c[0].max(c[1]).max(c[2]));
    if hi <= 1e-6 { 0.0 } else { ((hi - lo) / hi).clamp(0.0, 1.0) }
}

/// Sponge: move a colour away from (`saturate`) or toward its luma grey by `amount` (0..1).
///
/// `vibrance` damps the change where it would clip: saturating affects already-saturated colours
/// less, desaturating affects near-grey colours less (so they never fully collapse to grey in one
/// pass). Saturation is gamut-compressed around luma, never clipped per channel.
pub fn sponge(c: [f32; 3], amount: f32, saturate: bool, vibrance: bool) -> [f32; 3] {
    let mut a = amount.clamp(0.0, 1.0);
    if a <= 0.0 {
        return c;
    }
    let s = saturation(c);
    if vibrance {
        a *= if saturate { 1.0 - s } else { 0.25 + 0.75 * s };
    }
    let l = luma(c);
    let k = if saturate { 1.0 + a } else { 1.0 - a };
    compress_gamut([l + (c[0] - l) * k, l + (c[1] - l) * k, l + (c[2] - l) * k], l)
}

/// Gaussian blur of the window `[x0, x1) × [y0, y1)` of an interleaved `w × h × ch` buffer, reading
/// neighbours with edge clamping. With an alpha channel (`alpha = Some(index)`), colour is blurred
/// premultiplied so transparent pixels don't bleed their (meaningless) colour. Returns the window's
/// blurred pixels (`(x1−x0) × (y1−y0) × ch`).
#[allow(clippy::too_many_arguments)]
pub fn local_blur(data: &[f32], w: usize, h: usize, ch: usize, alpha: Option<usize>, win: (usize, usize, usize, usize), sigma: f32) -> Vec<f32> {
    let (x0, y0, x1, y1) = win;
    let (ow, oh) = (x1 - x0, y1 - y0);
    if ow == 0 || oh == 0 || ch == 0 {
        return Vec::new();
    }
    let r = (sigma * 3.0).ceil().max(1.0) as i32;
    let k: Vec<f32> = (-r..=r).map(|i| (-((i * i) as f32) / (2.0 * sigma * sigma).max(1e-6)).exp()).collect();
    let ks: f32 = k.iter().sum();
    let k: Vec<f32> = k.into_iter().map(|v| v / ks).collect();
    // Horizontal pass over the window rows extended vertically by r. Rows are independent, so
    // both passes run them in parallel: a 2500 px Blur dab is ~10⁹ multiply-adds (#5426).
    let th = oh + 2 * r as usize;
    let stride = ow * ch;
    let mut tmp = vec![0.0f32; stride * th];
    let scratch = || vec![0.0f32; (ow + 2 * r as usize) * ch];
    let horizontal = |row: &mut Vec<f32>, (ty, dst): (usize, &mut [f32])| {
        let y = (y0 as i32 + ty as i32 - r).clamp(0, h as i32 - 1) as usize;
        // Clamp and premultiply each source pixel once, rather than once for every kernel tap.
        for (rx, px) in row.chunks_exact_mut(ch).enumerate() {
            let x = (x0 as i32 + rx as i32 - r).clamp(0, w as i32 - 1) as usize;
            let i = (y * w + x) * ch;
            px.copy_from_slice(&data[i..i + ch]);
            if let Some(a) = alpha {
                let av = px[a];
                for (c, v) in px.iter_mut().enumerate() {
                    if c != a {
                        *v *= av;
                    }
                }
            }
        }
        // Contiguous accumulation can be vectorized for any channel layout. Each output still
        // sums the same products in the same kernel order as the scalar convolution.
        for (i, kv) in k.iter().enumerate() {
            for (d, s) in dst.iter_mut().zip(&row[i * ch..i * ch + stride]) {
                *d += s * kv;
            }
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        tmp.par_chunks_mut(stride).enumerate().for_each_init(scratch, horizontal);
    }
    #[cfg(target_arch = "wasm32")]
    {
        let mut row = scratch();
        tmp.chunks_mut(stride).enumerate().for_each(|item| horizontal(&mut row, item));
    }
    let mut out = vec![0.0f32; ow * oh * ch];
    let vertical = |(oy, dst): (usize, &mut [f32])| {
        for (i, kv) in k.iter().enumerate() {
            for (d, s) in dst.iter_mut().zip(&tmp[(oy + i) * stride..(oy + i + 1) * stride]) {
                *d += s * kv;
            }
        }
        if let Some(a) = alpha {
            for (ox, px) in dst.chunks_exact_mut(ch).enumerate() {
                let av = px[a];
                for (c, v) in px.iter_mut().enumerate() {
                    if c != a {
                        *v = if av > 1e-6 { *v / av } else { data[((y0 + oy) * w + x0 + ox) * ch + c] };
                    }
                }
            }
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        out.par_chunks_mut(stride).enumerate().for_each(vertical);
    }
    #[cfg(target_arch = "wasm32")]
    {
        out.chunks_mut(stride).enumerate().for_each(vertical);
    }
    out
}

/// Unsharp-mask sharpen of a window (same layout as [`local_blur`]): `v + amount·(v − blur(v))`.
///
/// With `protect_detail`, each result is clamped to the 3×3 neighbourhood's range, which stops
/// halos and noise from being amplified into new extremes. Alpha is left unchanged.
#[allow(clippy::too_many_arguments)]
pub fn local_sharpen(
    data: &[f32],
    w: usize,
    h: usize,
    ch: usize,
    alpha: Option<usize>,
    win: (usize, usize, usize, usize),
    amount: f32,
    protect_detail: bool,
) -> Vec<f32> {
    let (x0, y0, x1, y1) = win;
    let (ow, oh) = (x1 - x0, y1 - y0);
    if ow == 0 || oh == 0 || ch == 0 {
        return Vec::new();
    }
    let blur = local_blur(data, w, h, ch, alpha, win, 1.0);
    let mut out = vec![0.0f32; blur.len()];
    let stride = ow * ch;

    // Rows are independent, so process them in parallel with rayon (#5426).
    let process_row = |(oy, dst): (usize, &mut [f32])| {
        let y = y0 + oy;
        let blur_row = &blur[oy * stride..(oy + 1) * stride];
        for ox in 0..ow {
            let x = x0 + ox;
            let o = ox * ch;
            let i = (y * w + x) * ch;

            // Precompute 3×3 neighbourhood offsets once per pixel if protecting detail.
            let offsets: [usize; 8] = if protect_detail {
                if x > 0 && x + 1 < w && y > 0 && y + 1 < h {
                    let w_ch = w * ch;
                    [i - w_ch - ch, i - w_ch, i - w_ch + ch, i - ch, i + ch, i + w_ch - ch, i + w_ch, i + w_ch + ch]
                } else {
                    let get_n_idx = |dx: i32, dy: i32| -> usize {
                        let nx = (x as i32 + dx).clamp(0, w as i32 - 1) as usize;
                        let ny = (y as i32 + dy).clamp(0, h as i32 - 1) as usize;
                        (ny * w + nx) * ch
                    };
                    [
                        get_n_idx(-1, -1),
                        get_n_idx(0, -1),
                        get_n_idx(1, -1),
                        get_n_idx(-1, 0),
                        get_n_idx(1, 0),
                        get_n_idx(-1, 1),
                        get_n_idx(0, 1),
                        get_n_idx(1, 1),
                    ]
                }
            } else {
                [0; 8]
            };

            for c in 0..ch {
                let v = data[i + c];
                if Some(c) == alpha {
                    dst[o + c] = v;
                    continue;
                }
                let mut s = v + amount * (v - blur_row[o + c]);
                if protect_detail {
                    let (mut lo, mut hi) = (v, v);
                    for &n_idx in &offsets {
                        let nv = data[n_idx + c];
                        lo = lo.min(nv);
                        hi = hi.max(nv);
                    }
                    s = s.clamp(lo, hi);
                }
                dst[o + c] = s;
            }
        }
    };

    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        out.par_chunks_mut(stride).enumerate().for_each(process_row);
    }
    #[cfg(target_arch = "wasm32")]
    {
        out.chunks_mut(stride).enumerate().for_each(process_row);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dodge_midtones_affects_midtones_most() {
        let g = |v: f32| dodge_burn([v; 3], 0.5, ToneRange::Midtones, false, true)[0] - v;
        assert!(g(0.5) > 0.1, "{}", g(0.5));
        assert!(g(0.5) > 2.0 * g(0.1));
        assert!(g(0.5) > 2.0 * g(0.95));
        assert_eq!(dodge_burn([0.0; 3], 0.5, ToneRange::Midtones, false, true), [0.0; 3]);
        let b = dodge_burn([0.5; 3], 0.5, ToneRange::Midtones, true, true)[0];
        assert!(b < 0.45);
    }

    #[test]
    fn ranges_target_their_tones() {
        let d = |v: f32, r| tone_curve(v, 0.5, r, false) - v;
        assert!(d(0.1, ToneRange::Shadows) > d(0.9, ToneRange::Shadows));
        assert!(d(0.9, ToneRange::Highlights) > d(0.1, ToneRange::Highlights));
        let b = |v: f32, r| v - tone_curve(v, 0.5, r, true);
        assert!(b(0.9, ToneRange::Highlights) > b(0.2, ToneRange::Highlights));
        assert!(b(0.3, ToneRange::Shadows) / 0.3 > b(0.9, ToneRange::Shadows) / 0.9);
    }

    const CURVES: [(ToneRange, bool); 6] = [
        (ToneRange::Shadows, false),
        (ToneRange::Shadows, true),
        (ToneRange::Midtones, false),
        (ToneRange::Midtones, true),
        (ToneRange::Highlights, false),
        (ToneRange::Highlights, true),
    ];

    #[test]
    fn protect_tones_keeps_hue() {
        let c = [0.6, 0.3, 0.2];
        let p = dodge_burn(c, 0.8, ToneRange::Midtones, false, true);
        assert!(p.iter().all(|v| (0.0..=1.0).contains(v)));
        let (l, nl) = (luma(c), luma(p));
        let ratio = (p[0] - nl) / (p[1] - nl);
        assert!((ratio - (c[0] - l) / (c[1] - l)).abs() < 1e-3, "{p:?}");
    }

    #[test]
    fn protect_tones_keeps_a_shadow_cast_faint() {
        for c in [[8.0f32, 8.0, 13.0], [10.0, 8.0, 8.0], [26.0, 20.0, 20.0]] {
            let c = c.map(|v| v / 255.0);
            for (range, burn) in CURVES {
                let p = dodge_burn(c, 1.0, range, burn, true);
                let per_channel = dodge_burn(c, 1.0, range, burn, false);
                let spread = |v: [f32; 3]| v[0].max(v[1]).max(v[2]) - v[0].min(v[1]).min(v[2]);
                assert!(spread(p) <= spread(per_channel) * 1.1 + 1e-4, "{range:?} burn={burn}: {c:?} -> {p:?}, per channel {per_channel:?}");
                assert!(spread(p) <= spread(c) * 2.0, "{range:?} burn={burn}: {c:?} -> {p:?}");
            }
        }
    }

    #[test]
    fn protect_tones_keeps_casts_by_white_and_black_faint() {
        for c in [[1.0f32, 0.9993, 0.9993], [0.9995, 1.0, 0.9990], [0.0, 0.0007, 0.0007], [0.0005, 0.0, 0.0010]] {
            for (range, burn) in CURVES {
                for e in [0.3, 0.6, 1.0] {
                    let p = dodge_burn(c, e, range, burn, true);
                    let per_channel = dodge_burn(c, e, range, burn, false);
                    let spread = |v: [f32; 3]| v[0].max(v[1]).max(v[2]) - v[0].min(v[1]).min(v[2]);
                    assert!(spread(p) <= spread(per_channel) * 1.1 + 1e-5, "{range:?} burn={burn} e={e}: {c:?} -> {p:?}, per channel {per_channel:?}");
                }
            }
        }
    }

    #[test]
    fn protect_tones_never_spreads_a_near_black_cast_past_the_per_channel_curve() {
        let spread = |v: [f32; 3]| v[0].max(v[1]).max(v[2]) - v[0].min(v[1]).min(v[2]);
        for base in [0.0f32, 0.0005, 0.001, 0.002, 0.005, 0.01] {
            for d in [1e-5f32, 1e-4, 3e-4, 7e-4, 1.5e-3, 3e-3] {
                for c in [[base, base, base + d], [base + d, base, base], [base, base + d, base + d], [base + d, base, base + d / 2.0]] {
                    for (range, burn) in CURVES {
                        for e in [0.1, 0.3, 0.6, 1.0] {
                            let p = dodge_burn(c, e, range, burn, true);
                            let per_channel = dodge_burn(c, e, range, burn, false);
                            assert!(spread(p) <= spread(per_channel) + 1e-6, "{range:?} burn={burn} e={e}: {c:?} -> {p:?}, per channel {per_channel:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn every_curve_keeps_tones_in_order_at_every_exposure() {
        for (range, burn) in CURVES {
            for e in 1..=100 {
                let e = e as f32 / 100.0;
                let mut prev = tone_curve(0.0, e, range, burn);
                for i in 1..=1000 {
                    let v = tone_curve(i as f32 / 1000.0, e, range, burn);
                    assert!(v >= prev - 1e-6, "{range:?} burn={burn} e={e} at {i}: {v} < {prev}");
                    prev = v;
                }
            }
        }
    }

    #[test]
    fn full_exposure_keeps_the_far_end_in_place() {
        assert!(tone_curve(1.0, 1.0, ToneRange::Highlights, true) > 0.5);
        assert!(tone_curve(0.0, 1.0, ToneRange::Shadows, false) < 0.5);
        assert!(tone_curve(0.2, 0.5, ToneRange::Shadows, false) > tone_curve(0.0, 0.5, ToneRange::Shadows, false));
    }

    #[test]
    fn sponge_changes_saturation() {
        let c = [0.8, 0.4, 0.3];
        let d = sponge(c, 0.5, false, false);
        assert!(saturation(d) < saturation(c));
        assert!((luma(d) - luma(c)).abs() < 1e-4);
        let s = sponge(c, 0.5, true, false);
        assert!(saturation(s) > saturation(c));
        assert!(s.iter().all(|v| (0.0..=1.0).contains(v)));
        // Vibrance protects already-saturated colours when saturating.
        let v = sponge([1.0, 0.1, 0.1], 0.5, true, true);
        let nv = sponge([1.0, 0.1, 0.1], 0.5, true, false);
        assert!((v[1] - 0.1).abs() <= (nv[1] - 0.1).abs());
        assert_eq!(sponge([0.5; 3], 1.0, false, false), [0.5; 3]);
    }

    #[test]
    fn blur_reduces_variance_and_sharpen_increases_it() {
        let (w, h, ch) = (16, 16, 1);
        let data: Vec<f32> = (0..w * h).map(|i| if (i % w + i / w) % 2 == 0 { 0.2 } else { 0.8 }).collect();
        let var = |v: &[f32]| {
            let m = v.iter().sum::<f32>() / v.len() as f32;
            v.iter().map(|x| (x - m).powi(2)).sum::<f32>() / v.len() as f32
        };
        let win = (4, 4, 12, 12);
        let orig: Vec<f32> = (4..12).flat_map(|y| (4..12).map(move |x| (x, y))).map(|(x, y)| data[y * w + x]).collect();
        let b = local_blur(&data, w, h, ch, None, win, 1.5);
        assert!(var(&b) < var(&orig) * 0.2);
        let smooth: Vec<f32> = (0..w * h).map(|i| 0.3 + 0.03 * (i % w) as f32 + if i % w == 8 { 0.2 } else { 0.0 }).collect();
        let s = local_sharpen(&smooth, w, h, ch, None, win, 1.0, false);
        let so: Vec<f32> = (4..12).flat_map(|y| (4..12).map(move |x| (x, y))).map(|(x, y)| smooth[y * w + x]).collect();
        assert!(var(&s) > var(&so));
        // Protect detail never exceeds the local range.
        let sp = local_sharpen(&smooth, w, h, ch, None, win, 4.0, true);
        let hi = smooth.iter().cloned().fold(0.0, f32::max);
        assert!(sp.iter().all(|v| *v <= hi + 1e-6));
    }

    #[test]
    fn blur_ignores_transparent_colour() {
        // Red opaque next to transparent green: blurred colour stays red.
        let (w, h, ch) = (8, 1, 4);
        let data: Vec<f32> = (0..w).flat_map(|x| if x < 4 { [1.0, 0.0, 0.0, 1.0] } else { [0.0, 1.0, 0.0, 0.0] }).collect();
        let b = local_blur(&data, w, h, ch, Some(3), (2, 0, 6, 1), 1.0);
        for p in b.as_chunks::<4>().0 {
            if p[3] > 0.01 {
                assert!(p[0] > 0.99 && p[1] < 0.01, "{p:?}");
            }
        }
    }
}
