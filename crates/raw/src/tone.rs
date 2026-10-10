//! Camera Raw's Shadows and Highlights on raw files, applied while developing (in linear light,
//! before the tone curve), and its adaptive black point.
//!
//! Measured black-box from Photoshop 25.4 opening 29 Nikon D4 NEFs through Camera Raw (each slider
//! at ±100 and +50, exposure ±1), against PhotoCraft's own linear development of the same files:
//!
//! * The change is a gain in linear light: exposure −1 gives the same gains, so Camera Raw applies
//!   it before its tone curve.
//! * log₂ of the gain is `slider / 100 · C(B − anchor)`. `B` is an edge-preserving base of the log₂
//!   luminance (K. He's guided filter, radius 32 and ε 1 stop² on a copy 1232 px on its long
//!   side): whole areas move together and edges stay sharp. The change is linear in the slider;
//!   each direction has its own curve `C`.
//! * The anchor follows the image, so exposure changes nothing: for Shadows the log₂ power mean
//!   (p = 1.5) of the linear luminance, for Highlights the mean of the log₂ luminance.
//!
//! On frames left out of the fit the curves explain 80–90 % of the per-pixel change (a fixed,
//! absolute anchor: 60–75 %).
//!
//! The black point: Camera Raw's default rendering follows the frame's darkest tones. A third of
//! the 0.1th percentile of the linear luminance is subtracted; that halves the deep-shadow
//! error of a fixed curve.

use crate::par;

/// Camera Raw's Shadows and Highlights (Basic panel), −100…100.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Tone {
    pub shadows: f32,
    pub highlights: f32,
}

impl Tone {
    pub fn is_neutral(&self) -> bool {
        self.shadows == 0.0 && self.highlights == 0.0
    }
}

/// Long side of the copy the base is computed on, and the guided filter's radius and ε there.
const WORK_SIDE: usize = 1232;
const BASE_RADIUS: f32 = 32.0;
const BASE_EPS: f32 = 1.0;
/// The curves' first knot (stops from the anchor) and knot spacing.
const LO: f32 = -12.0;
const STEP: f32 = 0.25;

// log₂ gain at +100 / −100 against B − anchor, knots LO, LO + STEP, … (measured; flat or
// extended linearly beyond the measured range, never steeper than 0.8 stop per stop so tones
// keep their order).
const SHADOWS_UP: [f32; 73] = [
    2.142, 2.142, 2.142, 2.142, 2.142, 2.142, 2.142, 2.142, 2.142, 2.142, 2.142, 2.142, 2.142, 2.142, 2.142, 2.142, 2.142, 2.142, 2.142, 2.142, 2.142, 2.142,
    2.142, 2.142, 2.142, 2.142, 2.138, 2.117, 2.074, 2.010, 1.918, 1.797, 1.668, 1.546, 1.429, 1.315, 1.204, 1.093, 0.979, 0.860, 0.734, 0.604, 0.482, 0.378,
    0.288, 0.211, 0.157, 0.127, 0.104, 0.078, 0.056, 0.046, 0.044, 0.044, 0.044, 0.044, 0.044, 0.041, 0.030, 0.014, 0.003, 0.000, 0.000, 0.000, 0.000, 0.000,
    0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
];
const SHADOWS_DOWN: [f32; 73] = [
    -2.365, -2.365, -2.365, -2.365, -2.365, -2.365, -2.365, -2.365, -2.365, -2.365, -2.365, -2.365, -2.365, -2.365, -2.365, -2.365, -2.365, -2.365, -2.365,
    -2.365, -2.365, -2.365, -2.365, -2.346, -2.270, -2.155, -2.078, -2.054, -2.031, -1.973, -1.872, -1.739, -1.604, -1.483, -1.372, -1.266, -1.161, -1.053,
    -0.940, -0.821, -0.696, -0.570, -0.455, -0.356, -0.270, -0.197, -0.146, -0.117, -0.098, -0.076, -0.057, -0.048, -0.046, -0.046, -0.046, -0.046, -0.046,
    -0.043, -0.032, -0.014, -0.003, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
];
const HIGHLIGHTS_UP: [f32; 73] = [
    0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
    0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.001, 0.002, 0.003, 0.004, 0.008, 0.017, 0.030, 0.043, 0.055, 0.070, 0.085, 0.097, 0.106,
    0.114, 0.120, 0.127, 0.142, 0.167, 0.199, 0.239, 0.301, 0.381, 0.470, 0.569, 0.677, 0.796, 0.925, 1.056, 1.176, 1.268, 1.319, 1.339, 1.347, 1.351, 1.357,
    1.364, 1.372, 1.380, 1.388, 1.396, 1.404, 1.412,
];
#[allow(clippy::approx_constant)] // measured values, one of them happens to be close to e
const HIGHLIGHTS_DOWN: [f32; 73] = [
    0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
    0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, -0.004, -0.020, -0.046, -0.068, -0.086, -0.104, -0.116, -0.118, -0.118, -0.118, -0.118, -0.118,
    -0.119, -0.120, -0.123, -0.127, -0.131, -0.144, -0.169, -0.204, -0.245, -0.301, -0.375, -0.452, -0.537, -0.639, -0.753, -0.872, -0.997, -1.132, -1.294,
    -1.475, -1.637, -1.751, -1.836, -1.936, -2.059, -2.191, -2.323, -2.454, -2.586, -2.718, -2.850,
];

/// The tone change of one image: the change at a reduced size, carried to full size along the
/// image's own edges (guided upsampling), and the black point.
pub(crate) struct ToneMap {
    w: usize,
    h: usize,
    sw: usize,
    sh: usize,
    /// Local linear model `change = a · log₂ Y + b` per reduced pixel; empty: no tone change.
    a: Vec<f32>,
    b: Vec<f32>,
    /// log₂ luminance floor (the reduced copy's 0.1th percentile).
    floor: f32,
    /// Linear black point to subtract (0: none).
    pub(crate) black: f32,
}

impl ToneMap {
    /// `lum(i)`: linear luminance of full-size pixel `i` (row-major, `w × h`). `black_point`: the
    /// camera profile's black-point factor (see [`crate::Sensor::black_point`]). `None` when
    /// nothing changes.
    pub(crate) fn new(w: usize, h: usize, lum: impl Fn(usize) -> f32 + Sync, tone: Tone, black_point: f32) -> Option<Self> {
        if w == 0 || h == 0 || (tone.is_neutral() && black_point <= 0.0) {
            return None;
        }
        let f = w.max(h).div_ceil(WORK_SIDE).max(1);
        let (sw, sh) = (w.div_ceil(f), h.div_ceil(f));
        let rows = par::map(sh, |sy| {
            let (y0, y1) = (sy * f, ((sy + 1) * f).min(h));
            (0..sw)
                .map(|sx| {
                    let (x0, x1) = (sx * f, ((sx + 1) * f).min(w));
                    let mut s = 0.0f64;
                    for y in y0..y1 {
                        for x in x0..x1 {
                            s += f64::from(lum(y * w + x).max(0.0));
                        }
                    }
                    (s / ((x1 - x0) * (y1 - y0)) as f64) as f32
                })
                .collect::<Vec<f32>>()
        });
        let y: Vec<f32> = rows.concat();
        let p01 = quantile(&y, 0.001).max(2f32.powi(-20));
        let black = if black_point > 0.0 { black_point * p01 } else { 0.0 };
        let floor = p01.log2();
        if tone.is_neutral() {
            return Some(ToneMap { w, h, sw, sh, a: Vec::new(), b: Vec::new(), floor, black });
        }
        let l: Vec<f32> = y.iter().map(|v| v.max(p01).log2()).collect();
        let n = l.len() as f64;
        let geo = (l.iter().map(|v| f64::from(*v)).sum::<f64>() / n) as f32;
        let pm = ((l.iter().map(|v| f64::from(v.exp2()).powf(1.5)).sum::<f64>() / n).log2() / 1.5) as f32;
        let r = ((BASE_RADIUS * sw.max(sh) as f32 / WORK_SIDE as f32).round() as usize).max(1);
        let base = guided(&l, &l, sw, sh, r, BASE_EPS);
        let change: Vec<f32> = base.iter().map(|b| shadows(tone.shadows, b - pm) + highlights(tone.highlights, b - geo)).collect();
        // Fit change ≈ a · l + b locally (radius 2, ε 0.01), averaged; evaluated at full size.
        let (a, b) = linear_fit(&l, &change, sw, sh, 2, 0.01);
        Some(ToneMap { w, h, sw, sh, a, b, floor, black })
    }

    /// The gain at full-size pixel (`x`, `y`) of linear luminance `lum`.
    pub(crate) fn gain(&self, x: usize, y: usize, lum: f32) -> f32 {
        if self.a.is_empty() {
            return 1.0;
        }
        let sx = ((x as f32 + 0.5) * self.sw as f32 / self.w as f32 - 0.5).clamp(0.0, (self.sw - 1) as f32);
        let sy = ((y as f32 + 0.5) * self.sh as f32 / self.h as f32 - 0.5).clamp(0.0, (self.sh - 1) as f32);
        let (x0, y0) = (sx as usize, sy as usize);
        let (x1, y1) = ((x0 + 1).min(self.sw - 1), (y0 + 1).min(self.sh - 1));
        let (fx, fy) = (sx - x0 as f32, sy - y0 as f32);
        let at = |v: &[f32]| {
            let top = v[y0 * self.sw + x0] * (1.0 - fx) + v[y0 * self.sw + x1] * fx;
            let bottom = v[y1 * self.sw + x0] * (1.0 - fx) + v[y1 * self.sw + x1] * fx;
            top * (1.0 - fy) + bottom * fy
        };
        let l = if lum > 0.0 { lum.log2().max(self.floor) } else { self.floor };
        (at(&self.a) * l + at(&self.b)).clamp(-6.0, 6.0).exp2()
    }

    /// Black-point subtraction of one linear value.
    #[inline]
    pub(crate) fn unblack(&self, v: f32) -> f32 {
        if self.black > 0.0 { ((v - self.black) / (1.0 - self.black)).max(0.0) } else { v }
    }
}

fn curve(c: &[f32; 73], d: f32) -> f32 {
    let t = ((d - LO) / STEP).clamp(0.0, (c.len() - 1) as f32);
    let i = (t as usize).min(c.len() - 2);
    let f = t - i as f32;
    c[i] + (c[i + 1] - c[i]) * f
}

/// log₂ gain of Shadows `s` (−100…100) at `d` stops from its anchor.
fn shadows(s: f32, d: f32) -> f32 {
    let s = s.clamp(-100.0, 100.0) / 100.0;
    if s > 0.0 {
        s * curve(&SHADOWS_UP, d)
    } else if s < 0.0 {
        -s * curve(&SHADOWS_DOWN, d)
    } else {
        0.0
    }
}

/// log₂ gain of Highlights `h` (−100…100) at `d` stops from its anchor.
fn highlights(h: f32, d: f32) -> f32 {
    let h = h.clamp(-100.0, 100.0) / 100.0;
    if h > 0.0 {
        h * curve(&HIGHLIGHTS_UP, d)
    } else if h < 0.0 {
        -h * curve(&HIGHLIGHTS_DOWN, d)
    } else {
        0.0
    }
}

/// The `q` quantile (0…1) of `v`.
fn quantile(v: &[f32], q: f32) -> f32 {
    let mut s = v.to_vec();
    let k = ((s.len() - 1) as f32 * q).round() as usize;
    let (_, m, _) = s.select_nth_unstable_by(k, |a, b| a.total_cmp(b));
    *m
}

/// Index `i` mirrored into `0..n` (half-sample symmetric: … 1 0 | 0 1 … n−1 | n−1 n−2 …).
fn mirror(i: isize, n: usize) -> usize {
    let n = n as isize;
    let p = i.rem_euclid(2 * n);
    (if p < n { p } else { 2 * n - 1 - p }) as usize
}

/// Box mean of radius `r` with mirrored edges.
fn box_mean(v: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let k = (2 * r + 1) as f64;
    let ri = r as isize;
    let rows = par::map(h, |y| {
        let src = &v[y * w..(y + 1) * w];
        let mut acc: f64 = (-ri..=ri).map(|i| f64::from(src[mirror(i, w)])).sum();
        let mut out = vec![0.0f32; w];
        for (x, o) in out.iter_mut().enumerate() {
            *o = (acc / k) as f32;
            let x = x as isize;
            acc += f64::from(src[mirror(x + ri + 1, w)]) - f64::from(src[mirror(x - ri, w)]);
        }
        out
    });
    let tmp = rows.concat();
    let cols = par::map(w, |x| {
        let at = |y: isize| f64::from(tmp[mirror(y, h) * w + x]);
        let mut acc: f64 = (-ri..=ri).map(at).sum();
        let mut out = vec![0.0f32; h];
        for (y, o) in out.iter_mut().enumerate() {
            *o = (acc / k) as f32;
            let y = y as isize;
            acc += at(y + ri + 1) - at(y - ri);
        }
        out
    });
    let mut out = vec![0.0f32; w * h];
    for (x, col) in cols.iter().enumerate() {
        for (y, v) in col.iter().enumerate() {
            out[y * w + x] = *v;
        }
    }
    out
}

/// Local linear fit `p ≈ a · i + b` over boxes of radius `r` (regularized by `eps`), with `a`
/// and `b` box-averaged (K. He, J. Sun, X. Tang, *Guided Image Filtering*, 2010).
fn linear_fit(i: &[f32], p: &[f32], w: usize, h: usize, r: usize, eps: f32) -> (Vec<f32>, Vec<f32>) {
    let ip: Vec<f32> = i.iter().zip(p).map(|(a, b)| a * b).collect();
    let ii: Vec<f32> = i.iter().map(|a| a * a).collect();
    let (mi, mp, mip, mii) = (box_mean(i, w, h, r), box_mean(p, w, h, r), box_mean(&ip, w, h, r), box_mean(&ii, w, h, r));
    let a: Vec<f32> = (0..w * h).map(|k| (mip[k] - mi[k] * mp[k]) / ((mii[k] - mi[k] * mi[k]).max(0.0) + eps)).collect();
    let b: Vec<f32> = (0..w * h).map(|k| mp[k] - a[k] * mi[k]).collect();
    (box_mean(&a, w, h, r), box_mean(&b, w, h, r))
}

/// Guided filter of `p` with guide `i`.
fn guided(i: &[f32], p: &[f32], w: usize, h: usize, r: usize, eps: f32) -> Vec<f32> {
    let (a, b) = linear_fit(i, p, w, h, r, eps);
    a.iter().zip(&b).zip(i).map(|((a, b), i)| a * i + b).collect()
}

/// Shadows / Highlights on an image [`develop`](crate::develop) already made (gamma-1.8 ProPhoto
/// RGBA, 0…1) with the profile tone `curve`, for a preview of the open-time setting: the curve is
/// undone, the change applied in linear light and the curve redone. The developed pixels already
/// carry the black point, so the change is computed after it rather than before; that only
/// differs in the deepest shadows.
pub fn retone(px: &mut [[f32; 4]], w: usize, h: usize, curve: &[[f32; 2]], tone: Tone) {
    if tone.is_neutral() || w == 0 || h == 0 || px.len() < w * h {
        return;
    }
    let forward = |v: f32| crate::develop::tone(curve, v.clamp(0.0, 1.0)).clamp(0.0, 1.0).powf(1.0 / 1.8);
    // The inverse from a table over s = √v (the encoding is steep near 0).
    const N: usize = 4096;
    let table: Vec<f32> = (0..=N).map(|i| forward((i as f32 / N as f32).powi(2))).collect();
    let inverse = |e: f32| {
        let i = table.partition_point(|t| *t < e).clamp(1, N);
        let (a, b) = (table[i - 1], table[i]);
        let f = if b > a { ((e - a) / (b - a)).clamp(0.0, 1.0) } else { 0.0 };
        ((i - 1) as f32 + f) / N as f32
    };
    let lin: Vec<[f32; 3]> = px.iter().map(|p| [0, 1, 2].map(|k| inverse(p[k]).powi(2))).collect();
    let y = crate::color::rgb_to_xyz(crate::color::ROMM, crate::color::D50_XY)[1].map(|v| v as f32);
    let lum = |c: &[f32; 3]| y[0] * c[0] + y[1] * c[1] + y[2] * c[2];
    let Some(map) = ToneMap::new(w, h, |i| lum(&lin[i]), tone, 0.0) else { return };
    for (i, (p, l)) in px.iter_mut().zip(&lin).enumerate() {
        let g = map.gain(i % w, i / w, lum(l));
        for k in 0..3 {
            p[k] = forward(l[k] * g);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHADOWS: Tone = Tone { shadows: 100.0, highlights: 0.0 };
    const HIGHLIGHTS: Tone = Tone { shadows: 0.0, highlights: -100.0 };

    /// Three vertical bands of linear luminance 2⁻⁷ | 2⁻⁴ | 2⁻¹, times `scale`.
    fn bands(w: usize, h: usize, scale: f32) -> Vec<f32> {
        (0..w * h).map(|i| scale * [2f32.powi(-7), 2f32.powi(-4), 0.5][(i % w) * 3 / w]).collect()
    }

    fn gains(y: &[f32], w: usize, h: usize, tone: Tone) -> Vec<f32> {
        let Some(m) = ToneMap::new(w, h, |i| y[i], tone, 0.0) else { return vec![1.0; w * h] };
        (0..w * h).map(|i| m.gain(i % w, i / w, y[i])).collect()
    }

    #[test]
    fn shadows_lift_the_dark_area_and_keep_the_bright_one() {
        let (w, h) = (300, 120);
        let g = gains(&bands(w, h, 1.0), w, h, SHADOWS);
        let (dark, mid, bright) = (g[60 * w + 50], g[60 * w + 150], g[60 * w + 250]);
        // 5 stops below the anchor: about +2 stops, as Camera Raw lifts such areas; 1 stop above it: almost nothing.
        assert!(dark.log2() > 1.8 && dark.log2() < 2.3, "{dark}");
        assert!(mid > 1.0 && mid < dark, "{mid}");
        assert!(bright.log2() < 0.06, "{bright}");
    }

    #[test]
    fn highlights_lower_the_bright_area_and_keep_the_dark_one() {
        let (w, h) = (300, 120);
        let g = gains(&bands(w, h, 1.0), w, h, HIGHLIGHTS);
        let (dark, bright) = (g[60 * w + 50], g[60 * w + 250]);
        // 3 stops above the anchor about −1 stop; 3 below a slight darkening (Camera Raw lowers a
        // high-contrast frame as a whole a little too).
        assert!(dark.log2() < 0.0 && dark.log2() > -0.15, "{dark}");
        assert!(bright.log2() < -0.8 && bright.log2() > -1.3, "{bright}");
    }

    #[test]
    fn exposure_changes_nothing() {
        let (w, h) = (300, 120);
        for tone in [SHADOWS, HIGHLIGHTS, Tone { shadows: -60.0, highlights: 40.0 }] {
            let (a, b) = (gains(&bands(w, h, 1.0), w, h, tone), gains(&bands(w, h, 0.25), w, h, tone));
            assert!(a.iter().zip(&b).all(|(a, b)| (a - b).abs() < 1e-3 * a), "{tone:?}");
        }
    }

    #[test]
    fn areas_move_as_a_whole_without_a_halo() {
        // Next to an edge the change stays between the two areas' changes (no over- or undershoot)
        // and close to the change inside the area.
        let (w, h) = (600, 120);
        let g: Vec<f32> = gains(&bands(w, h, 1.0), w, h, SHADOWS).iter().map(|g| g.log2()).collect();
        let row = &g[60 * w..61 * w];
        let (dark, mid) = (row[w / 6], row[w / 2]);
        for (x, v) in row.iter().enumerate().take(w / 2).skip(w / 6) {
            assert!(*v <= dark + 1e-3 && *v >= mid - 1e-3, "{x}: {v} outside {mid}…{dark}");
        }
        assert!(dark - row[w / 3 - 2] < 0.25 && row[w / 3 + 2] - mid < 0.25, "{} {} | {} {}", dark, row[w / 3 - 2], row[w / 3 + 2], mid);
    }

    #[test]
    fn the_result_does_not_depend_on_the_image_size() {
        let probe = |w: usize, h: usize| {
            let g = gains(&bands(w, h, 1.0), w, h, SHADOWS);
            [w / 6, w / 2, 5 * w / 6].map(|x| g[(h / 2) * w + x].log2())
        };
        let (a, b) = (probe(4928, 1640), probe(1232, 410));
        assert!(a.iter().zip(&b).all(|(a, b)| (a - b).abs() < 0.02), "{a:?} vs {b:?}");
    }

    #[test]
    fn the_black_point_follows_the_darkest_tones() {
        let (w, h) = (300, 120);
        let y = bands(w, h, 1.0);
        let m = ToneMap::new(w, h, |i| y[i], Tone::default(), 0.33).unwrap();
        assert!((m.black - 0.33 * 2f32.powi(-7)).abs() < 1e-7, "{}", m.black);
        assert!(m.unblack(m.black * 0.5) == 0.0 && (m.unblack(1.0) - 1.0).abs() < 1e-6);
        assert!(ToneMap::new(w, h, |i| y[i], Tone::default(), 0.0).is_none());
    }

    #[test]
    fn retone_changes_nothing_at_zero_and_matches_the_develop_stage() {
        let (w, h) = (300, 120);
        let y = bands(w, h, 1.0);
        let curve = [[0.01, 0.02], [0.1, 0.25], [1.0, 1.0]];
        let enc = |v: f32| crate::develop::tone(&curve, v).powf(1.0 / 1.8);
        let px: Vec<[f32; 4]> = y.iter().map(|v| [enc(*v), enc(*v), enc(*v), 1.0]).collect();
        let mut same = px.clone();
        retone(&mut same, w, h, &curve, Tone::default());
        assert_eq!(same, px);
        let mut out = px.clone();
        retone(&mut out, w, h, &curve, SHADOWS);
        let g = gains(&y, w, h, SHADOWS);
        for i in [60 * w + 50, 60 * w + 150, 60 * w + 250] {
            assert!((out[i][0] - enc(y[i] * g[i])).abs() < 0.003, "{i}: {} vs {}", out[i][0], enc(y[i] * g[i]));
        }
    }
}
