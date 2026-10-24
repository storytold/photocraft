//! Edit › Auto: image-adaptive Light settings (and a little Vibrance) like Adobe Camera Raw's
//! Auto button. The settings are fitted by running [`develop`] itself on a small copy of the
//! image, so they agree with what the preview and the filter will show:
//!
//! 1. **Exposure** moves the median luminance toward middle grey (damped, so a deliberately dark
//!    or bright photo keeps some of its key).
//! 2. **Highlights / Shadows** recover the share of pixels left near white / black, and
//!    **Contrast** widens or narrows the interquartile spread toward a typical photograph's.
//! 3. **Whites / Blacks** are then searched (bisection) so the 99.5th / 0.5th luminance
//!    percentiles land just inside the range, the way Camera Raw sets its clipping points.
//! 4. **Vibrance** and **Saturation** lift dull colour a little; never on a black-and-white
//!    treatment.
//!
//! [`auto_white_balance`] is White Balance › Auto (Temperature and Tint), kept separate as in
//! Camera Raw.
//!
//! Values are rounded to the sliders' steps. Neutral for an empty or fully transparent image.

use super::{CameraRaw, develop, luma};

/// The settings Auto owns. Every other control is left as it was.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct AutoTone {
    pub exposure: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    pub vibrance: f32,
    pub saturation: f32,
}

impl AutoTone {
    /// The Auto-owned settings of `p`.
    pub fn of(p: &CameraRaw) -> Self {
        AutoTone {
            exposure: p.exposure,
            contrast: p.contrast,
            highlights: p.highlights,
            shadows: p.shadows,
            whites: p.whites,
            blacks: p.blacks,
            vibrance: p.vibrance,
            saturation: p.saturation,
        }
    }
    /// Writes these settings into `p`.
    pub fn apply(self, p: &mut CameraRaw) {
        p.exposure = self.exposure;
        p.contrast = self.contrast;
        p.highlights = self.highlights;
        p.shadows = self.shadows;
        p.whites = self.whites;
        p.blacks = self.blacks;
        p.vibrance = self.vibrance;
        p.saturation = self.saturation;
    }
}

/// Longest side of the copy the settings are fitted on.
const FIT_SIDE: usize = 192;
/// Middle grey in the display encoding (18 % linear through the sRGB curve).
const MID_GREY: f32 = 0.46;

/// Auto settings for straight RGBA pixels `w × h` (display-encoded, as [`develop`] takes them).
/// `base` supplies everything Auto does not own (white balance, encoding, pixel scale, the
/// black-and-white treatment), and its Auto-owned settings are ignored.
pub fn auto_tone(px: &[[f32; 4]], w: usize, h: usize, base: &CameraRaw) -> AutoTone {
    let Some((small, sw, sh)) = downsample(px, w, h) else { return AutoTone::default() };
    // Only white balance, the light stage and the encoding matter for the measurement.
    let p = CameraRaw {
        temperature: base.temperature,
        tint: base.tint,
        encoding_gamma: base.encoding_gamma,
        pixel_scale: base.pixel_scale.clamp(0.01, 1.0) * (sw.max(sh) as f32 / w.max(h).max(1) as f32).min(1.0),
        seed: base.seed,
        ..CameraRaw::default()
    };
    let measure = |p: &CameraRaw| -> Option<Vec<f32>> {
        let mut img = small.clone();
        develop(&mut img, sw, sh, p, false);
        let mut lum: Vec<f32> = img.iter().filter(|q| q[3] >= 0.5).map(|q| luma([q[0], q[1], q[2]]).clamp(0.0, 1.0)).filter(|v| v.is_finite()).collect();
        if lum.is_empty() {
            return None;
        }
        lum.sort_by(f32::total_cmp);
        Some(lum)
    };
    let Some(original) = measure(&p) else { return AutoTone::default() };

    // 1. Exposure: median toward middle grey, damped by a quarter, in 0.05 EV steps.
    let median = percentile(&original, 0.5);
    let target = MID_GREY + (median - MID_GREY) * 0.25;
    let exposure = round_to(bisect(-3.0, 3.0, |e| percentile(&measure(&CameraRaw { exposure: e, ..p.clone() }).unwrap_or_default(), 0.5) - target), 0.05);
    let mut fit = CameraRaw { exposure, ..p.clone() };
    let exposed = measure(&fit).unwrap_or(original);

    // 2. Highlights / Shadows from what sits near the ends; Contrast from the quartile spread.
    let share = |test: &dyn Fn(f32) -> bool| exposed.iter().filter(|v| test(**v)).count() as f32 / exposed.len() as f32;
    let bright = share(&|v| v > 0.88);
    let dark = share(&|v| v < 0.12);
    fit.highlights = -(bright * 260.0 + if bright > 0.01 { 8.0 } else { 0.0 }).min(70.0).round();
    fit.shadows = (dark * 260.0 + if dark > 0.01 { 8.0 } else { 0.0 }).min(60.0).round();
    let spread = percentile(&exposed, 0.75) - percentile(&exposed, 0.25);
    fit.contrast = ((0.30 - spread) * 120.0).clamp(-20.0, 30.0).round();

    // 3. Whites, then Blacks, so the clipping points land just inside the range. A (nearly) flat
    // image has no range to stretch: those, and Contrast, stay put.
    if percentile(&exposed, 0.995) - percentile(&exposed, 0.005) < 0.04 {
        fit.contrast = 0.0;
    } else {
        fit.whites = bisect(-60.0, 60.0, |v| percentile(&measure(&CameraRaw { whites: v, ..fit.clone() }).unwrap_or_default(), 0.995) - 0.97).round();
        fit.blacks = bisect(-60.0, 60.0, |v| percentile(&measure(&CameraRaw { blacks: v, ..fit.clone() }).unwrap_or_default(), 0.005) - 0.03).round();
    }

    // 4. Vibrance and a little Saturation for dull colour, never into a black-and-white treatment.
    let (vibrance, saturation) = if base.saturation <= -100.0 {
        (base.vibrance, base.saturation)
    } else {
        let chroma = small.iter().filter(|q| q[3] >= 0.5).map(|q| q[0].max(q[1]).max(q[2]) - q[0].min(q[1]).min(q[2])).sum::<f32>() / exposed.len() as f32;
        (((0.25 - chroma) * 100.0).clamp(0.0, 20.0).round(), ((0.20 - chroma) * 40.0).clamp(0.0, 8.0).round())
    };
    let clean = |v: f32| if v.is_finite() { v + 0.0 } else { 0.0 };
    AutoTone {
        exposure: clean(fit.exposure),
        contrast: clean(fit.contrast),
        highlights: clean(fit.highlights),
        shadows: clean(fit.shadows),
        whites: clean(fit.whites),
        blacks: clean(fit.blacks),
        vibrance: clean(vibrance),
        saturation: clean(saturation),
    }
}

/// White Balance › Auto: the Temperature and Tint (slider units, relative to as shot) that make
/// the image's near-neutral pixels grey. Grey world in linear light over the midtones, limited to
/// the least coloured pixels when there are enough of them (a white wall, not a red sofa, says
/// what grey looks like). `(0, 0)` when nothing qualifies.
pub fn auto_white_balance(px: &[[f32; 4]], w: usize, h: usize) -> (f32, f32) {
    let Some((small, _, _)) = downsample(px, w, h) else { return (0.0, 0.0) };
    let mids: Vec<[f32; 3]> = small
        .iter()
        .filter(|q| q[3] >= 0.5)
        .map(|q| [q[0], q[1], q[2]])
        .filter(|c| {
            let l = luma(*c);
            (0.08..=0.92).contains(&l) && c.iter().all(|v| *v < 0.98)
        })
        .collect();
    let chroma = |c: &[f32; 3]| c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2]);
    let mut by_chroma: Vec<f32> = mids.iter().map(chroma).collect();
    by_chroma.sort_by(f32::total_cmp);
    // The least coloured half, so strong colours do not pull the balance.
    let limit = percentile(&by_chroma, 0.5);
    let mut sum = [0.0f64; 3];
    let mut n = 0usize;
    for c in mids.iter().filter(|c| chroma(c) <= limit) {
        for k in 0..3 {
            sum[k] += f64::from(crate::photo_util::srgb_to_linear(c[k]));
        }
        n += 1;
    }
    if n < 16 || sum.iter().any(|v| *v <= 1e-6) {
        return (0.0, 0.0);
    }
    let [r, g, b] = sum.map(|v| v as f32);
    // Invert `white_balance_gains`: (1 + 0.35 t) R = (1 − 0.35 t) B, then (1 − 0.25 n) G = (1 + 0.35 t) R.
    let k = b / r;
    let t = ((k - 1.0) / (0.35 * (1.0 + k))).clamp(-1.0, 1.0);
    let tint = ((1.0 - r * (1.0 + 0.35 * t) / g) / 0.25).clamp(-1.0, 1.0);
    let slider = |v: f32| if v.is_finite() { (v * 100.0).round() + 0.0 } else { 0.0 };
    (slider(t), slider(tint))
}

/// A box-averaged copy no longer than [`FIT_SIDE`]; `None` when nothing is opaque.
fn downsample(px: &[[f32; 4]], w: usize, h: usize) -> Option<(Vec<[f32; 4]>, usize, usize)> {
    if w == 0 || h == 0 || px.len() < w * h {
        return None;
    }
    let step = w.max(h).div_ceil(FIT_SIDE).max(1);
    let (sw, sh) = (w.div_ceil(step), h.div_ceil(step));
    let mut out = vec![[0.0f32; 4]; sw * sh];
    let mut any = false;
    for sy in 0..sh {
        for sx in 0..sw {
            let (mut sum, mut alpha, mut n) = ([0.0f32; 3], 0.0f32, 0usize);
            for y in sy * step..((sy + 1) * step).min(h) {
                for x in sx * step..((sx + 1) * step).min(w) {
                    let q = px[y * w + x];
                    let a = if q[3].is_finite() { q[3].clamp(0.0, 1.0) } else { 0.0 };
                    for c in 0..3 {
                        if q[c].is_finite() {
                            sum[c] += q[c].max(0.0) * a;
                        }
                    }
                    alpha += a;
                    n += 1;
                }
            }
            let o = &mut out[sy * sw + sx];
            if alpha > 0.0 {
                *o = [sum[0] / alpha, sum[1] / alpha, sum[2] / alpha, alpha / n.max(1) as f32];
            }
            any |= o[3] >= 0.5;
        }
    }
    any.then_some((out, sw, sh))
}

/// The value at fraction `t` of sorted `v` (0 when empty).
fn percentile(v: &[f32], t: f32) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v[((v.len() - 1) as f32 * t).round() as usize]
}

/// The root of increasing `f` in `lo..=hi`, or the nearer end when it has none there.
fn bisect(mut lo: f32, mut hi: f32, f: impl Fn(f32) -> f32) -> f32 {
    if f(lo) >= 0.0 {
        return lo;
    }
    if f(hi) <= 0.0 {
        return hi;
    }
    for _ in 0..14 {
        let mid = (lo + hi) * 0.5;
        if f(mid) < 0.0 { lo = mid } else { hi = mid }
    }
    (lo + hi) * 0.5
}

fn round_to(v: f32, step: f32) -> f32 {
    (v / step).round() * step
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(w: usize, h: usize, f: impl Fn(usize, usize) -> [f32; 4]) -> Vec<[f32; 4]> {
        (0..w * h).map(|i| f(i % w, i / w)).collect()
    }

    fn median_after(px: &[[f32; 4]], w: usize, h: usize, a: AutoTone) -> f32 {
        let mut p = CameraRaw::default();
        a.apply(&mut p);
        let mut out = px.to_vec();
        develop(&mut out, w, h, &p, false);
        let mut lum: Vec<f32> = out.iter().map(|q| luma([q[0], q[1], q[2]])).collect();
        lum.sort_by(f32::total_cmp);
        percentile(&lum, 0.5)
    }

    #[test]
    fn a_dark_photo_is_brightened_and_a_bright_one_darkened() {
        let dark = image(64, 48, |x, y| {
            let v = 0.05 + 0.25 * (x + y) as f32 / 110.0;
            [v, v * 0.9, v * 0.8, 1.0]
        });
        let a = auto_tone(&dark, 64, 48, &CameraRaw::default());
        assert!(a.exposure > 0.3, "{a:?}");
        assert!(median_after(&dark, 64, 48, a) > median_after(&dark, 64, 48, AutoTone::default()));
        let bright = image(64, 48, |x, y| {
            let v = 0.75 + 0.24 * (x + y) as f32 / 110.0;
            [v, v, v * 0.95, 1.0]
        });
        let b = auto_tone(&bright, 64, 48, &CameraRaw::default());
        assert!(b.exposure < 0.0 || b.highlights < 0.0, "{b:?}");
    }

    #[test]
    fn settings_are_finite_bounded_and_rounded() {
        let px = image(300, 200, |x, y| [x as f32 / 300.0, y as f32 / 200.0, 0.4, 1.0]);
        let a = auto_tone(&px, 300, 200, &CameraRaw::default());
        for v in [a.exposure, a.contrast, a.highlights, a.shadows, a.whites, a.blacks, a.vibrance] {
            assert!(v.is_finite(), "{a:?}");
        }
        assert!((-3.0..=3.0).contains(&a.exposure));
        for v in [a.contrast, a.highlights, a.shadows, a.whites, a.blacks, a.vibrance] {
            assert!((-100.0..=100.0).contains(&v) && v == v.round(), "{a:?}");
        }
    }

    #[test]
    fn transparent_empty_and_broken_input_is_neutral() {
        assert_eq!(auto_tone(&[], 0, 0, &CameraRaw::default()), AutoTone::default());
        let clear = image(16, 16, |_, _| [0.2, 0.3, 0.4, 0.0]);
        assert_eq!(auto_tone(&clear, 16, 16, &CameraRaw::default()), AutoTone::default());
        let nan = image(16, 16, |_, _| [f32::NAN, f32::INFINITY, -1.0, 1.0]);
        let a = auto_tone(&nan, 16, 16, &CameraRaw::default());
        assert!([a.exposure, a.contrast, a.whites, a.blacks].iter().all(|v| v.is_finite()), "{a:?}");
        assert_eq!(auto_tone(&clear, 99, 99, &CameraRaw::default()), AutoTone::default(), "short buffer");
    }

    #[test]
    fn black_and_white_keeps_its_vibrance() {
        let px = image(32, 32, |x, _| [0.3, 0.32, 0.31 + x as f32 * 0.001, 1.0]);
        let bw = CameraRaw { saturation: -100.0, vibrance: 0.0, ..Default::default() };
        assert_eq!((auto_tone(&px, 32, 32, &bw).vibrance, auto_tone(&px, 32, 32, &bw).saturation), (0.0, -100.0));
        assert!(auto_tone(&px, 32, 32, &CameraRaw::default()).vibrance > 0.0);
    }

    #[test]
    fn of_and_apply_round_trip_only_the_auto_settings() {
        let mut p = CameraRaw { temperature: -40.0, clarity: 12.0, ..Default::default() };
        let a = AutoTone { exposure: 0.5, contrast: 10.0, highlights: -20.0, shadows: 15.0, whites: 5.0, blacks: -5.0, vibrance: 8.0, saturation: 3.0 };
        a.apply(&mut p);
        assert_eq!(AutoTone::of(&p), a);
        assert_eq!((p.temperature, p.clarity), (-40.0, 12.0));
    }

    #[test]
    fn a_flat_image_keeps_whites_blacks_and_contrast() {
        let flat = image(64, 48, |_, _| [0.29, 0.23, 0.19, 1.0]);
        let a = auto_tone(&flat, 64, 48, &CameraRaw::default());
        assert_eq!((a.whites, a.blacks, a.contrast), (0.0, 0.0, 0.0), "{a:?}");
        assert!(a.exposure > 0.0);
    }

    #[test]
    fn auto_white_balance_neutralises_a_cast() {
        let grey = image(64, 48, |x, _| {
            let v = 0.3 + x as f32 * 0.005;
            [v, v, v, 1.0]
        });
        assert_eq!(auto_white_balance(&grey, 64, 48), (0.0, 0.0));
        // A warm (orange) cast wants a cooler Temperature; a green one a magenta Tint.
        let warm = image(64, 48, |x, _| {
            let v = 0.3 + x as f32 * 0.005;
            [v * 1.15, v, v * 0.8, 1.0]
        });
        let (t, _) = auto_white_balance(&warm, 64, 48);
        assert!(t < -10.0, "{t}");
        let green = image(64, 48, |x, _| {
            let v = 0.3 + x as f32 * 0.005;
            [v * 0.9, v * 1.1, v * 0.9, 1.0]
        });
        let (_, tint) = auto_white_balance(&green, 64, 48);
        assert!(tint > 10.0, "{tint}");
        // Developing with the result brings the cast close to grey.
        let (t, n) = auto_white_balance(&warm, 64, 48);
        let mut out = warm.clone();
        develop(&mut out, 64, 48, &CameraRaw { temperature: t, tint: n, ..Default::default() }, false);
        let q = out[24 * 64 + 32];
        assert!((q[0] - q[2]).abs() < 0.03 && (q[0] - q[1]).abs() < 0.03, "{q:?}");
        assert_eq!(auto_white_balance(&[], 0, 0), (0.0, 0.0));
        assert_eq!(auto_white_balance(&image(8, 8, |_, _| [0.5, 0.4, 0.3, 0.0]), 8, 8), (0.0, 0.0));
    }
}
