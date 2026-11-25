//! Per-pixel and per-dab kernels for the toning and focus retouching tools: Dodge, Burn, Sponge,
//! Blur, Sharpen. The brush engine (`photocraft-paint`) decides *where* and *how strongly* each dab
//! applies; these functions decide *what* happens to the pixels.
//!
//! Shadows Dodge and Highlights Burn are fitted to strokes measured in the reference application
//! (8-bit RGB, with and without Protect Tones); the other four curves are our own design. Every
//! curve keeps tones in order at every strength. "Protect Tones" moves luma along its curve and
//! scales the channels by new ÷ old luma, as the reference does, so a colour keeps its hue and
//! proportions; gamut compression replaces per-channel clipping.

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

#[inline]
fn smoothstep(lo: f32, hi: f32, v: f32) -> f32 {
    let t = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The dodge/burn transfer for one value `v` (0..1) with strength `e` (0..1). Monotone in `v`.
/// `protect` selects the Protect Tones curve, which is applied to luma.
///
/// The constants of Shadows Dodge and Highlights Burn come from reference strokes at one Exposure
/// each (Shadows Dodge 50 %, Highlights Burn 100 %); their strength grows linearly with Exposure,
/// as the reference's Highlights Burn does between 50 % and 100 % (#12659).
#[inline]
pub fn tone_curve(v: f32, e: f32, range: ToneRange, burn: bool, protect: bool) -> f32 {
    let v = v.max(0.0);
    let e = e.clamp(0.0, 1.0);
    let u = v.min(1.0);
    match (range, burn, protect) {
        // Protected, black stays put and the lift fades out above the shadows; the slope at black
        // stays at 1 + 0.8e, so scaling a near-black colour by its luma ratio keeps a cast faint.
        (ToneRange::Shadows, false, true) => v + 0.8 * e * v * (1.0 - smoothstep(0.0, 0.7, u)),
        (ToneRange::Shadows, false, false) => v + 0.55 * e * (1.0 - u),
        (ToneRange::Highlights, true, true) => v - 0.195 * e * v * smoothstep(0.15, 0.64, u),
        (ToneRange::Highlights, true, false) => v * (1.0 - 0.418 * e),
        // Gamma bend, weighted to peak at mid-grey: 0 and 1 are fixed points.
        (ToneRange::Midtones, _, _) => {
            let w = (4.0 * v * (1.0 - v)).clamp(0.0, 1.0);
            v + w * (u.powf(midtone_gamma(e, burn)) - v)
        }
        (ToneRange::Shadows, true, _) => v - e * (1.0 - u).powi(2) * v,
        (ToneRange::Highlights, false, _) => v + e * u.powi(2) * (1.0 - u),
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
/// With `protect_tones`, luma goes through the protected curve and the channels scale by new ÷ old
/// luma (hue and proportions kept, gamut-compressed); otherwise each channel goes through the curve.
pub fn dodge_burn(c: [f32; 3], e: f32, range: ToneRange, burn: bool, protect_tones: bool) -> [f32; 3] {
    if e <= 0.0 {
        return c;
    }
    let curve = |v: f32| tone_curve(v, e, range, burn, protect_tones);
    if !protect_tones {
        return c.map(curve);
    }
    let l = luma(c);
    // Every protected curve keeps black at 0, so a colour this dark has no ratio to scale by.
    if l <= 1e-6 {
        return c.map(curve);
    }
    let nl = curve(l);
    let k = nl / l;
    compress_gamut(c.map(|v| v * k), nl)
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

/// Compound a reference dab's gain, with closed forms for common spacings.
#[inline]
fn sponge_gain(gain: f32, steps: f32) -> f32 {
    match steps {
        0.5 => gain.sqrt(),
        1.0 => gain,
        2.0 => gain * gain,
        4.0 => {
            let squared = gain * gain;
            squared * squared
        }
        _ => gain.powf(steps),
    }
}

/// Sum the local affine response over the dab's fractional number of reference steps.
#[inline]
fn sponge_affine_scale(change: f32, steps: f32) -> f32 {
    let gain = 1.0 + change;
    match steps {
        0.5 => 1.0 / (gain.sqrt() + 1.0),
        2.0 => gain + 1.0,
        4.0 => (gain + 1.0) * (gain * gain + 1.0),
        // The first terms of ((1 + change)^steps - 1) / change avoid cancellation near zero.
        _ if change.abs() < 0.001 => {
            let second = (steps - 1.0) * (steps - 2.0) / 6.0;
            steps * (1.0 + change * (0.5 * (steps - 1.0) + change * second))
        }
        _ => (gain.powf(steps) - 1.0) / change,
    }
}

/// Adjust a normalized RGB dab using the measured RGB Sponge response (#12972, #12973).
/// `coverage` includes Flow and the brush falloff. `spacing` is in brush diameters.
pub fn sponge_rgb(c: [f32; 3], coverage: f32, spacing: f32, saturate: bool, vibrance: bool) -> [f32; 3] {
    let [r, g, b] = c;
    let lo = r.min(g).min(b);
    let hi = r.max(g).max(b);
    let chroma = hi - lo;
    if coverage <= 0.0 || chroma <= f32::EPSILON {
        return c;
    }

    let flow = coverage.min(1.0);
    // The measured dab covers a quarter of a brush diameter along the stroke. Scale its gain
    // multiplicatively so the same distance gets the same strength from more or fewer dabs.
    let steps = 4.0 * spacing.clamp(0.01, 1.0);
    let grey = luma(c);
    if !vibrance {
        let reference_gain = if saturate { 1.0 + 0.4 * flow } else { 1.0 - 0.2 * flow };
        let mut gain = sponge_gain(reference_gain, steps);
        if saturate {
            // Limit the gain before applying it to all three channels, using the same luma pivot.
            let shadow = grey - lo;
            if gain * shadow > grey {
                gain = grey / shadow;
            }
            let highlight = hi - grey;
            if gain * highlight > 1.0 - grey {
                gain = (1.0 - grey) / highlight;
            }
        }
        return c.map(|v| grey + (v - grey) * gain);
    }

    // Vibrance changes the channel extrema asymmetrically. The shadow response tapers near
    // black and near grey. The highlight response is limited by headroom in Saturate and by
    // brightness and chroma in Desaturate. Fit those bounds to the measured single dabs.
    let mid = r.min(g).max(r.max(g).min(b));
    let shadow = (((lo - 0.125).max(0.0) / 12.0) + (mid - lo) * 0.025).min(chroma / 6.0).min(lo);
    let (low, high) = if saturate {
        let highlight = (0.4 * (1.0 - hi)).min(0.5 * (hi - grey));
        ((lo - flow * shadow).max(0.0), hi + flow * highlight)
    } else {
        let weight = (0.5 * lo / hi).max(0.2);
        let highlight = (0.06 * hi + 0.03 * lo).min(weight * (hi - grey));
        (lo + flow * shadow, hi - flow * highlight)
    };
    // Compose the measured affine response for this dab's distance, holding its locally
    // computed coefficients fixed, then cap the endpoints at the RGB gamut.
    let (low, high) = if steps == 1.0 {
        (low, high)
    } else {
        let change = ((high - hi) - (low - lo)) / chroma;
        let scale = sponge_affine_scale(change, steps);
        ((lo + (low - lo) * scale).max(0.0), (hi + (high - hi) * scale).min(1.0))
    };
    // One affine map keeps the intermediate channel in the same position within the span.
    let gain = (high - low).max(0.0) / chroma;
    c.map(|v| low + (v - lo) * gain)
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

/// Gaussian sigma of the Protect Detail sharpen in [`local_sharpen`], in pixels. It reads
/// `ceil(3·sigma)` pixels around each output pixel.
pub const PROTECT_DETAIL_SIGMA: f32 = 2.5;

/// Largest change one Protect Detail pass makes to a pixel (6 levels of 255), approached softly.
const PROTECT_DETAIL_LIMIT: f32 = 6.0 / 255.0;

/// Unsharp-mask sharpen of a window (same layout as [`local_blur`]): `v + amount·d` with
/// `d = v − blur(v)`, against a Gaussian of sigma 1.
///
/// With `protect_detail` (the Sharpen tool's default) it sharpens gently, as the reference
/// application does (#12938): the Gaussian is wider ([`PROTECT_DETAIL_SIGMA`]), so an edge gets a
/// low halo a few pixels wide, and each difference is soft-limited to `L` = 6 levels:
/// `v + amount·d·L/(L + |d|)`. Fine detail is sharpened about as much as without it, while a
/// strong edge or a lone bright pixel moves by less than `L` per pass instead of overshooting.
/// Uniform areas stay exactly as they are. Alpha is left unchanged.
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
    let blur = local_blur(data, w, h, ch, alpha, win, if protect_detail { PROTECT_DETAIL_SIGMA } else { 1.0 });
    let mut out = vec![0.0f32; blur.len()];
    let stride = ow * ch;

    // Rows are independent, so process them in parallel with rayon (#5426).
    let process_row = |(oy, dst): (usize, &mut [f32])| {
        let y = y0 + oy;
        let blur_row = &blur[oy * stride..(oy + 1) * stride];
        for ox in 0..ow {
            let o = ox * ch;
            let i = (y * w + x0 + ox) * ch;
            for c in 0..ch {
                let v = data[i + c];
                if Some(c) == alpha {
                    dst[o + c] = v;
                    continue;
                }
                let d = v - blur_row[o + c];
                // `PROTECT_DETAIL_LIMIT + |d|` is never zero, so this can't divide by zero.
                let d = if protect_detail { d * PROTECT_DETAIL_LIMIT / (PROTECT_DETAIL_LIMIT + d.abs()) } else { d };
                dst[o + c] = v + amount * d;
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
#[path = "retouch_sharpen_protect_detail_tests.rs"]
mod sharpen_protect_detail_tests;

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
        let d = |v: f32, r| tone_curve(v, 0.5, r, false, false) - v;
        assert!(d(0.1, ToneRange::Shadows) > d(0.9, ToneRange::Shadows));
        assert!(d(0.9, ToneRange::Highlights) > d(0.1, ToneRange::Highlights));
        let b = |v: f32, r| v - tone_curve(v, 0.5, r, true, false);
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
    fn protect_tones_keeps_black_and_a_cast_in_proportion() {
        let casts = [[8.0f32, 8.0, 13.0], [10.0, 8.0, 8.0], [26.0, 20.0, 20.0]].map(|c| c.map(|v| v / 255.0));
        let near_black = [[0.0f32, 0.0007, 0.0007], [0.0005, 0.0, 0.0010], [0.002, 0.002, 0.005]];
        for (range, burn) in CURVES {
            for e in [0.1, 0.3, 0.6, 1.0] {
                assert_eq!(dodge_burn([0.0; 3], e, range, burn, true), [0.0; 3], "{range:?} burn={burn} e={e}");
                for c in casts.iter().chain(&near_black) {
                    let p = dodge_burn(*c, e, range, burn, true);
                    let k = p[0].max(p[1]).max(p[2]) / c[0].max(c[1]).max(c[2]);
                    for i in 0..3 {
                        assert!((p[i] - c[i] * k).abs() < 1e-5, "{range:?} burn={burn} e={e}: {c:?} -> {p:?}");
                    }
                    assert!(k <= 2.0, "{range:?} burn={burn} e={e}: {c:?} scaled by {k}");
                }
            }
        }
    }

    #[test]
    fn every_curve_keeps_tones_in_order_at_every_exposure() {
        for (range, burn) in CURVES {
            for protect in [false, true] {
                for e in 1..=100 {
                    let e = e as f32 / 100.0;
                    let mut prev = tone_curve(0.0, e, range, burn, protect);
                    for i in 1..=1000 {
                        let v = tone_curve(i as f32 / 1000.0, e, range, burn, protect);
                        assert!(v >= prev - 1e-6, "{range:?} burn={burn} protect={protect} e={e} at {i}: {v} < {prev}");
                        prev = v;
                    }
                }
            }
        }
    }

    /// One stroke, hard 60 px brush, 8-bit RGB, measured in the reference application (#11533).
    /// Unprotected, the measured greys scatter by a few levels around a straight line.
    #[test]
    fn shadows_dodge_and_highlights_burn_match_the_reference() {
        type Sample = ([u8; 3], [u8; 3]);
        let grey = |pairs: &[(u8, u8)]| pairs.iter().map(|&(a, b)| ([a; 3], [b; 3])).collect::<Vec<Sample>>();
        let cases: [(ToneRange, bool, f32, bool, f32, Vec<Sample>); 4] = [
            (ToneRange::Shadows, false, 0.5, true, 1.0, {
                let mut s = grey(&[(0, 0), (32, 44), (64, 81), (128, 138), (192, 193), (230, 230), (255, 255)]);
                s.extend([([8, 8, 13], [12, 12, 19]), ([10, 8, 8], [14, 11, 11]), ([26, 20, 20], [36, 28, 28])]);
                s
            }),
            (ToneRange::Shadows, false, 0.5, false, 4.0, grey(&[(0, 74), (32, 89), (64, 120), (128, 165), (192, 208), (255, 255)])),
            (ToneRange::Highlights, true, 1.0, true, 1.0, {
                let mut s = grey(&[(0, 0), (32, 32), (64, 63), (128, 108), (192, 155), (230, 184), (255, 204)]);
                s.push(([230, 200, 180], [184, 161, 146]));
                s
            }),
            (ToneRange::Highlights, true, 1.0, false, 3.0, grey(&[(32, 19), (64, 36), (128, 73), (192, 115), (255, 145)])),
        ];
        for (range, burn, e, protect, levels, samples) in cases {
            for (input, want) in samples {
                let got = dodge_burn(input.map(|v| f32::from(v) / 255.0), e, range, burn, protect).map(|v| (v * 255.0).round());
                for (g, w) in got.iter().zip(want) {
                    assert!((g - f32::from(w)).abs() <= levels, "{range:?} burn={burn} e={e} protect={protect}: {input:?} -> {got:?}, reference {want:?}");
                }
            }
        }
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
        // Protect Detail sharpens too, but moves no pixel by more than its limit per unit of amount.
        let sp = local_sharpen(&smooth, w, h, ch, None, win, 4.0, true);
        assert!(var(&sp) > var(&so));
        assert!(sp.iter().zip(&so).all(|(a, b)| (a - b).abs() < 4.0 * PROTECT_DETAIL_LIMIT));
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

#[cfg(test)]
#[path = "retouch_sponge_desaturate_tests.rs"]
mod sponge_desaturate_tests;
#[cfg(test)]
#[path = "retouch_sponge_saturate_tests.rs"]
mod sponge_saturate_tests;
