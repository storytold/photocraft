//! The Optics panel's manual corrections: Distortion and lens Vignetting.
//!
//! Measured as a black box on Photoshop 2026's Camera Raw (18.6); no Adobe code or data was
//! used. Distortion: a 21 × 14 grid of dots went through Camera Raw at −100, −50, −25, 25, 50 and
//! 100, and the dots' positions fit, to 0.44 px everywhere, a radial map from each result pixel
//! back to the source: `source = c + (p − c) · (a + (1 − a) · r²)`, where `c` is the image centre,
//! `r` the distance from it in half-diagonals (the corners stay where they are) and `a = 1 +
//! amount / 250` above 0, `1 + amount / 200` below. Where the source falls outside the picture
//! (raising the amount pulls the edges' middles inward) the result is transparent, as Camera
//! Raw leaves it.
//!
//! Vignetting: flat greys (four levels) through the Camera Raw Filter at ±25, ±50 and ±100 and
//! midpoints 0 … 100, on three image shapes. It is a local Exposure: at each pixel the change
//! is what the Exposure slider does at a number of stops that is the same for every grey level
//! (to 0.02 stops), proportional to the amount, the same either way, and a function of the
//! distance from the centre in half-diagonals (the same on 600 × 400, 400 × 600 and 900 × 300)
//! and of the midpoint: about 1.9 stops in the corners at ±100, rising from the centre at
//! midpoint 0 and only near the corners at 100, gathering further toward the corners at small
//! amounts ([`VIGNETTE_STOPS`]). So it runs with the
//! Exposure slider, each pixel at the slider's stops plus its own.

use super::{CameraRaw, Encoding, exposure};
use crate::photo_util::par_rows;

/// Stops of lens vignetting per 100 of amount, at amounts 25, 50 and 100 (outer), midpoints 0,
/// 25, 50, 75, 100 and radii 0, 0.05, … 1 in half-diagonals (inner); measured (both signs
/// averaged: the effect is the same either way).
#[rustfmt::skip]
const VIGNETTE_STOPS: [[[f32; 21]; 5]; 3] = [
    [
        [0.000, 0.000, 0.000, 0.062, 0.098, 0.129, 0.207, 0.276, 0.375, 0.472, 0.568, 0.662, 0.776, 0.909, 1.049, 1.197, 1.333, 1.496, 1.664, 1.829, 2.000],
        [0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.059, 0.062, 0.105, 0.130, 0.213, 0.298, 0.399, 0.529, 0.714, 0.888, 1.123, 1.375, 1.661, 1.923],
        [0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.062, 0.062, 0.125, 0.186, 0.266, 0.400, 0.598, 0.826, 1.135, 1.511, 1.894],
        [0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.062, 0.062, 0.125, 0.237, 0.386, 0.610, 0.934, 1.382, 1.888],
        [0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.033, 0.062, 0.125, 0.250, 0.446, 0.776, 1.259, 1.861],
    ],
    [
        [0.000, 0.000, 0.031, 0.062, 0.115, 0.177, 0.249, 0.326, 0.431, 0.526, 0.642, 0.758, 0.879, 1.009, 1.149, 1.287, 1.428, 1.571, 1.707, 1.839, 1.959],
        [0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.031, 0.037, 0.066, 0.119, 0.177, 0.250, 0.350, 0.470, 0.618, 0.790, 0.983, 1.206, 1.453, 1.705, 1.930],
        [0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.031, 0.042, 0.076, 0.128, 0.205, 0.319, 0.469, 0.670, 0.916, 1.225, 1.581, 1.914],
        [0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.031, 0.049, 0.091, 0.163, 0.274, 0.447, 0.688, 1.022, 1.461, 1.895],
        [0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.023, 0.031, 0.079, 0.158, 0.290, 0.512, 0.854, 1.344, 1.867],
    ],
    [
        [0.000, 0.007, 0.038, 0.093, 0.163, 0.248, 0.345, 0.450, 0.564, 0.680, 0.800, 0.922, 1.047, 1.177, 1.303, 1.426, 1.546, 1.659, 1.769, 1.883, 1.977],
        [0.000, 0.000, 0.000, 0.000, 0.000, 0.010, 0.035, 0.064, 0.103, 0.166, 0.248, 0.349, 0.472, 0.615, 0.776, 0.953, 1.151, 1.356, 1.567, 1.773, 1.963],
        [0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.008, 0.016, 0.033, 0.065, 0.115, 0.188, 0.293, 0.434, 0.617, 0.834, 1.083, 1.373, 1.673, 1.951],
        [0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.010, 0.035, 0.067, 0.131, 0.230, 0.378, 0.585, 0.851, 1.188, 1.572, 1.926],
        [0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.009, 0.024, 0.056, 0.115, 0.225, 0.402, 0.662, 1.019, 1.474, 1.909],
    ],
];

/// Stops of vignetting at `amount` (−100 … 100) and `midpoint` (0 … 100), at radius `r`
/// (half-diagonals): the measured profiles interpolated in amount, midpoint and radius (below 25
/// the profile at 25, scaled).
fn vignette_stops(amount: f32, midpoint: f32, r: f32) -> f32 {
    let a = amount.clamp(-100.0, 100.0);
    let mag = a.abs();
    let (ia, ta) = if mag <= 25.0 {
        (0, 0.0)
    } else if mag <= 50.0 {
        (0, (mag - 25.0) / 25.0)
    } else {
        (1, (mag - 50.0) / 50.0)
    };
    let fm = (midpoint.clamp(0.0, 100.0) / 25.0).min(4.0);
    let fr = (r.clamp(0.0, 1.0) * 20.0).min(20.0);
    let (im, ir) = ((fm as usize).min(3), (fr as usize).min(19));
    let (tm, tr) = (fm - im as f32, fr - ir as f32);
    let at = |ai: usize, m: usize, k: usize| VIGNETTE_STOPS.get(ai).and_then(|t| t.get(m)).and_then(|row| row.get(k)).copied().unwrap_or(0.0);
    let radial = |ai: usize, m: usize| at(ai, m, ir) + (at(ai, m, ir + 1) - at(ai, m, ir)) * tr;
    let profile = |ai: usize| radial(ai, im) + (radial(ai, im + 1) - radial(ai, im)) * tm;
    let per100 = profile(ia) + (profile(ia + 1) - profile(ia)) * ta;
    per100 * a / 100.0
}

/// Whether lens Vignetting changes nothing.
pub(crate) fn vignette_neutral(p: &CameraRaw) -> bool {
    !p.lens_vignette.is_finite() || p.lens_vignette == 0.0
}

/// Exposure (the slider's stops) with lens Vignetting's stops added per pixel, on straight RGBA
/// pixels (`w × h`) encoded with `enc`.
pub(crate) fn exposure_with_vignette(px: &mut [[f32; 4]], w: usize, h: usize, p: &CameraRaw, enc: Encoding) {
    if w == 0 || h == 0 || px.len() != w.saturating_mul(h) {
        return;
    }
    // Exposure rows every 1/200 stop over the range the pixels need.
    const STEP: f32 = 0.005;
    let base = if p.exposure.is_finite() { p.exposure } else { 0.0 };
    let reach = vignette_stops(p.lens_vignette, p.lens_vignette_midpoint, 1.0);
    let (lo, hi) = (base + reach.min(0.0), base + reach.max(0.0));
    let n = (((hi - lo) / STEP).ceil() as usize).saturating_add(1).min(4001);
    let rows: Vec<_> = (0..n).map(|k| exposure::row(lo + k as f32 * STEP)).collect();
    let m = exposure::primaries(enc);
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let half_diag = cx.hypot(cy).max(1e-6);
    par_rows(px, w, 1, |y, line| {
        for (x, q) in line.iter_mut().enumerate() {
            let r = (x as f32 + 0.5 - cx).hypot(y as f32 + 0.5 - cy) / half_diag;
            let e = base + vignette_stops(p.lens_vignette, p.lens_vignette_midpoint, r);
            let k = (((e - lo) / STEP).round().max(0.0) as usize).min(n.saturating_sub(1));
            if let Some(row) = rows.get(k) {
                *q = exposure::pixel(*q, row, enc, m.as_ref());
            }
        }
    });
}

/// The centre's sampling scale at Distortion `amount` (−100 … 100): `a` in the module docs.
fn centre_scale(amount: f32) -> f64 {
    let v = f64::from(amount.clamp(-100.0, 100.0));
    if v >= 0.0 { 1.0 + v / 250.0 } else { 1.0 + v / 200.0 }
}

/// Whether Distortion changes nothing.
fn distortion_neutral(p: &CameraRaw) -> bool {
    !p.lens_distortion.is_finite() || p.lens_distortion == 0.0
}

/// Whether the Optics panel changes nothing.
pub(crate) fn neutral(p: &CameraRaw) -> bool {
    distortion_neutral(p) && vignette_neutral(p)
}

/// Rejects settings the Optics panel cannot take.
pub(crate) fn validate(p: &CameraRaw) -> Result<(), String> {
    if !p.lens_distortion.is_finite() || !(-100.0..=100.0).contains(&p.lens_distortion) {
        return Err(format!("lensDistortion: expected −100 … 100, got {}", p.lens_distortion));
    }
    if !p.lens_vignette.is_finite() || !(-100.0..=100.0).contains(&p.lens_vignette) {
        return Err(format!("lensVignette: expected −100 … 100, got {}", p.lens_vignette));
    }
    if !p.lens_vignette_midpoint.is_finite() || !(0.0..=100.0).contains(&p.lens_vignette_midpoint) {
        return Err(format!("lensVignetteMidpoint: expected 0 … 100, got {}", p.lens_vignette_midpoint));
    }
    Ok(())
}

/// Bilinear sample of straight RGBA `src` (`w × h`) at `(x, y)` (pixel edges at whole numbers),
/// with alpha weighting; a point outside the picture is transparent, one inside its border half
/// pixel takes the edge pixels.
fn sample(src: &[[f32; 4]], w: usize, h: usize, x: f64, y: f64) -> [f32; 4] {
    if !(0.0..=w as f64).contains(&x) || !(0.0..=h as f64).contains(&y) {
        return [0.0; 4];
    }
    let (fx, fy) = ((x - 0.5).clamp(0.0, (w - 1) as f64), (y - 0.5).clamp(0.0, (h - 1) as f64));
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = ((fx - x0) as f32, (fy - y0) as f32);
    let mut acc = [0.0f32; 4];
    for (dy, wy) in [(0.0, 1.0 - ty), (1.0, ty)] {
        for (dx, wx) in [(0.0, 1.0 - tx), (1.0, tx)] {
            let (sx, sy) = (x0 + dx, y0 + dy);
            if sx < 0.0 || sy < 0.0 || sx >= w as f64 || sy >= h as f64 {
                continue;
            }
            let Some(q) = src.get(sy as usize * w + sx as usize) else { continue };
            let wgt = wx * wy * q[3];
            for c in 0..3 {
                acc[c] += q[c] * wgt;
            }
            acc[3] += wgt;
        }
    }
    if acc[3] > 1e-6 {
        for c in 0..3 {
            acc[c] /= acc[3];
        }
    }
    acc
}

/// Distortion on straight RGBA pixels (`w × h`), in place.
pub(crate) fn apply(px: &mut [[f32; 4]], w: usize, h: usize, p: &CameraRaw, float: bool) {
    if distortion_neutral(p) || w == 0 || h == 0 || px.len() != w.saturating_mul(h) {
        return;
    }
    let a = centre_scale(p.lens_distortion);
    let src = px.to_vec();
    let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
    let half_diag2 = cx * cx + cy * cy;
    par_rows(px, w, 1, |y, row| {
        for (x, q) in row.iter_mut().enumerate() {
            let (dx, dy) = (x as f64 + 0.5 - cx, y as f64 + 0.5 - cy);
            let k = a + (1.0 - a) * (dx * dx + dy * dy) / half_diag2;
            let v = sample(&src, w, h, cx + dx * k, cy + dy * k);
            if v[3] <= 1e-6 {
                *q = [0.0; 4];
                continue;
            }
            for c in 0..3 {
                q[c] = if float { v[c] } else { v[c].clamp(0.0, 1.0) };
            }
            q[3] = v[3].clamp(0.0, 1.0);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dots(w: usize, h: usize) -> Vec<[f32; 4]> {
        (0..w * h).map(|i| if (i % w) % 16 < 2 || (i / w) % 16 < 2 { [0.0, 0.0, 0.0, 1.0] } else { [1.0; 4] }).collect()
    }

    #[test]
    fn the_scale_follows_the_measurements() {
        assert!((centre_scale(100.0) - 1.4).abs() < 1e-9 && (centre_scale(-100.0) - 0.5).abs() < 1e-9);
        assert!((centre_scale(25.0) - 1.1).abs() < 1e-9 && (centre_scale(-50.0) - 0.75).abs() < 1e-9);
        assert_eq!(centre_scale(1e9), centre_scale(100.0));
    }

    #[test]
    fn corners_stay_the_middle_moves_and_raising_uncovers_the_edges() {
        let (w, h) = (96, 64);
        let src = dots(w, h);
        let mut up = src.clone();
        apply(&mut up, w, h, &CameraRaw { lens_distortion: 100.0, ..Default::default() }, false);
        assert_eq!(up[0], src[0], "the corner stays");
        assert_eq!(up[(h / 2) * w][3], 0.0, "the left edge's middle is uncovered");
        let mut down = src.clone();
        apply(&mut down, w, h, &CameraRaw { lens_distortion: -100.0, ..Default::default() }, false);
        assert!(down.iter().all(|q| q[3] > 0.99), "lowering keeps the frame covered");
        assert_ne!(down, src);
        let mut same = src.clone();
        apply(&mut same, w, h, &CameraRaw::default(), false);
        assert_eq!(same, src);
    }

    #[test]
    fn vignetting_is_a_local_exposure() {
        // The measured corner value and the scaling with the amount.
        assert!((vignette_stops(100.0, 50.0, 1.0) - 1.951).abs() < 1e-3);
        assert!((vignette_stops(-50.0, 50.0, 1.0) + 0.957).abs() < 1e-3);
        // Smaller amounts gather toward the corners.
        assert!(vignette_stops(25.0, 50.0, 0.7) * 4.0 < vignette_stops(100.0, 50.0, 0.7));
        assert_eq!(vignette_stops(100.0, 50.0, 0.0), 0.0);
        assert!(vignette_stops(100.0, 0.0, 0.5) > vignette_stops(100.0, 100.0, 0.5));
        let (w, h) = (60, 40);
        let enc = Encoding::of(0.0);
        let mut px = vec![[0.4, 0.4, 0.4, 1.0]; w * h];
        exposure_with_vignette(&mut px, w, h, &CameraRaw { lens_vignette: 100.0, lens_vignette_midpoint: 50.0, ..Default::default() }, enc);
        let (centre, corner) = (px[(h / 2) * w + w / 2][0], px[0][0]);
        assert!((centre - 0.4).abs() < 0.01 && corner > 0.6, "{centre} {corner}");
        // With the Exposure slider too, the centre takes the slider's stops alone.
        let mut both = vec![[0.4, 0.4, 0.4, 1.0]; w * h];
        exposure_with_vignette(&mut both, w, h, &CameraRaw { exposure: 1.0, lens_vignette: -100.0, ..Default::default() }, enc);
        let mut plain = vec![[0.4, 0.4, 0.4, 1.0]];
        exposure::apply(&mut plain, 1, &exposure::row(1.0), enc);
        assert!((both[(h / 2) * w + w / 2][0] - plain[0][0]).abs() < 0.005);
        assert!(both[0][0] < both[(h / 2) * w + w / 2][0]);
    }

    #[test]
    fn bad_values_are_rejected_and_odd_sizes_are_safe() {
        for (v, m) in [(f32::NAN, 50.0), (150.0, 50.0), (50.0, -1.0), (50.0, f32::INFINITY)] {
            assert!(validate(&CameraRaw { lens_vignette: v, lens_vignette_midpoint: m, ..Default::default() }).is_err());
        }
        exposure_with_vignette(&mut [], 0, 0, &CameraRaw { lens_vignette: 50.0, ..Default::default() }, Encoding::of(0.0));
        for v in [f32::NAN, 101.0, -1e9] {
            assert!(validate(&CameraRaw { lens_distortion: v, ..Default::default() }).is_err());
        }
        let p = CameraRaw { lens_distortion: 50.0, ..Default::default() };
        apply(&mut [], 0, 0, &p, false);
        apply(&mut [[0.5; 4]; 3], 2, 2, &p, false);
        let mut one = vec![[0.3, 0.4, 0.5, 1.0]];
        apply(&mut one, 1, 1, &p, true);
        assert!(one[0].iter().all(|v| v.is_finite()));
    }
}
