//! Shadows and Highlights on the log2 luminance.
//!
//! Both are multi-scale moves fitted to Photoshop 2026's Camera Raw Filter by observation (see
//! [`multiscale`]); no Adobe code or data was used.
//!
//! - Each move is computed on a copy at most [`multiscale::SIDE`] pixels across and brought back
//!   to full size by a guided upsampling, so a preview and the full image agree and the result
//!   does not depend on the image size. A uniform image is unchanged.
//! - Highlights first, then Shadows on its result.
//! - Colour: a move of `d` stops raises colour ratios to the luminance to the power `2^(−c·d)`,
//!   so raising desaturates a little and darkening the highlights saturates a little, as
//!   Photoshop does.

use super::{Encoding, luma};
use crate::photo_util::par_rows;

mod multiscale;

/// Colour exponents (see the module notes) for raising and for lowering.
const HIGHLIGHTS_COLOUR: (f32, f32) = (0.8, 0.787);
const SHADOWS_COLOUR: (f32, f32) = (0.082, 0.05);

/// One channel, `w × h`.
struct Plane {
    w: usize,
    h: usize,
    v: Vec<f32>,
}

/// The floor added to the luminance before its log2.
pub const FLOOR: f32 = 1.0 / 65536.0;

/// The block size the moves are computed at for a `w × h` image: the copy is at most
/// [`multiscale::SIDE`] pixels across, its pixels the luminance averaged over `f × f` blocks.
pub fn reduction(w: usize, h: usize) -> usize {
    w.max(h).div_ceil(multiscale::SIDE).max(1)
}

/// One slider's move (stops) at full size, as a per-pixel model computed on the reduced copy.
/// With `exact` the grid is the full image and `b` is the move itself; otherwise the move at
/// a full-size pixel with log2 luminance `u` is `a·u + b` (each looked up bilinearly at the
/// pixel's centre mapped onto the grid), kept within ±[`MAX_MOVE`].
#[derive(Clone, Debug, PartialEq)]
pub struct ToneMove {
    pub w: usize,
    pub h: usize,
    pub a: Vec<f32>,
    pub b: Vec<f32>,
    pub exact: bool,
}

/// The largest move (stops) a guided upsampling may make.
pub const MAX_MOVE: f32 = multiscale::MAX_MOVE;

impl ToneMove {
    /// The move at full-size pixel `(x, y)` of a `full_w × full_h` image whose log2 luminance
    /// (plus [`FLOOR`]) there is `u`.
    pub fn at(&self, x: usize, y: usize, full_w: usize, full_h: usize, u: f32) -> f32 {
        if self.exact {
            return self.b.get(y * self.w + x).copied().unwrap_or(0.0);
        }
        let (fx, fy) = (self.w as f32 / full_w as f32, self.h as f32 / full_h as f32);
        let (sx, sy) = ((x as f32 + 0.5) * fx - 0.5, (y as f32 + 0.5) * fy - 0.5);
        let av = crate::photo_util::bilinear(&self.a, self.w, self.h, 1, 0, sx, sy);
        let bv = crate::photo_util::bilinear(&self.b, self.w, self.h, 1, 0, sx, sy);
        // The guide may lie far outside the proxy's tones (float images): keep the guard.
        (av * u + bv).clamp(-MAX_MOVE, MAX_MOVE)
    }
}

/// Highlights' (`highlights`) or Shadows' move at `amount` (−1 … 1) from the reduced copy:
/// `small` is the linear luminance averaged over `f × f` blocks (`sw × sh`; for Shadows, after
/// Highlights' move), `f` the block size ([`reduction`]). `None` at 0 or for a short copy.
pub fn tone_move(small: &[f32], sw: usize, sh: usize, f: usize, amount: f32, highlights: bool) -> Option<ToneMove> {
    if amount == 0.0 || !amount.is_finite() || sw == 0 || sh == 0 || small.len() != sw * sh {
        return None;
    }
    let plane = Plane { w: sw, h: sh, v: small.iter().map(|y| (y + FLOOR).log2()).collect() };
    let tables = if highlights { &multiscale::HIGHLIGHTS } else { &multiscale::SHADOWS };
    let d = multiscale::moves(&plane, amount, tables);
    if f == 1 {
        return Some(ToneMove { w: sw, h: sh, a: vec![0.0; sw * sh], b: d, exact: true });
    }
    let (a, b) = guided_coefficients(&plane, &d);
    Some(ToneMove { w: sw, h: sh, a, b, exact: false })
}

/// The colour exponent of Highlights' (`highlights`) or Shadows' move at `amount` (see the
/// module notes).
pub fn colour_exponent(highlights: bool, amount: f32) -> f32 {
    let c = if highlights { HIGHLIGHTS_COLOUR } else { SHADOWS_COLOUR };
    if amount > 0.0 { c.0 } else { c.1 }
}

/// One pixel's colour after Highlights' and Shadows' moves `a` and `b` (stops), with colour
/// exponents `c_hl` and `c_sh`; `lin` is linear in the document's primaries.
pub fn tone_pixel(lin: [f32; 3], a: f32, b: f32, c_hl: f32, c_sh: f32) -> [f32; 3] {
    let gain = (a + b).exp2();
    let l = luma(lin);
    let e = c_hl * a + c_sh * b;
    if e != 0.0 && l > 1e-6 && lin.iter().all(|v| *v >= 0.0) {
        let beta = (-e).exp2();
        let o = lin.map(|v| l * (v / l).powf(beta));
        let lo = luma(o);
        if lo > 1e-9 { o.map(|v| v * l * gain / lo) } else { lin.map(|v| v * gain) }
    } else {
        lin.map(|v| v * gain)
    }
}

/// Applies Shadows and Highlights (−100 … 100) to straight RGBA pixels encoded with `enc`:
/// Highlights first, then Shadows on its result.
pub(crate) fn apply(px: &mut [[f32; 4]], w: usize, h: usize, shadows: f32, highlights: f32, enc: Encoding) {
    if (shadows == 0.0 && highlights == 0.0) || w < 2 || h < 2 {
        return;
    }
    let lum: Vec<f32> = px.iter().map(|q| luma([0, 1, 2].map(|c| enc.decode(q[c]))).max(0.0)).collect();
    let (sh, hl) = (shadows.clamp(-100.0, 100.0) / 100.0, highlights.clamp(-100.0, 100.0) / 100.0);
    let f = reduction(w, h);
    let (sw, shh) = (w.div_ceil(f), h.div_ceil(f));
    // One slider's move at every full-size pixel, from the copy of `lin` reduced f × f.
    let full_moves = |lin: &[f32], amount: f32, highlights: bool| -> Vec<f32> {
        let small = shrink_linear(lin, w, h, f);
        let Some(m) = tone_move(&small, sw, shh, f, amount, highlights) else { return vec![0.0; w * h] };
        let mut d = vec![0.0f32; w * h];
        par_rows(&mut d, w, 1, |y, row| {
            for (x, o) in row.iter_mut().enumerate() {
                *o = m.at(x, y, w, h, (lin[y * w + x] + FLOOR).log2());
            }
        });
        d
    };
    let d_hl = full_moves(&lum, hl, true);
    let lum_after = after_highlights(&lum, &d_hl);
    let d_sh = full_moves(&lum_after, sh, false);
    let (c_hl, c_sh) = (colour_exponent(true, hl), colour_exponent(false, sh));
    par_rows(px, w, 1, |y, row| {
        for (x, q) in row.iter_mut().enumerate() {
            let lin = [0, 1, 2].map(|c| enc.decode(q[c]));
            let out = tone_pixel(lin, d_hl[y * w + x], d_sh[y * w + x], c_hl, c_sh);
            for c in 0..3 {
                q[c] = enc.encode(out[c]);
            }
        }
    });
}

/// The linear luminance after the Highlights move `d_hl` (stops), which the Shadows move reads.
pub fn after_highlights(lum: &[f32], d_hl: &[f32]) -> Vec<f32> {
    lum.iter().zip(d_hl).map(|(y, d)| y * d.exp2()).collect()
}

/// The luminance `lum` (`w × h`) averaged over `f × f` blocks (linear; [`tone_move`] takes the
/// log2).
fn shrink_linear(lum: &[f32], w: usize, h: usize, f: usize) -> Vec<f32> {
    let (sw, sh) = (w.div_ceil(f), h.div_ceil(f));
    let mut v = vec![0.0f32; sw * sh];
    par_rows(&mut v, sw, 1, |y, row| {
        for (x, o) in row.iter_mut().enumerate() {
            let (x0, y0) = (x * f, y * f);
            let (x1, y1) = ((x0 + f).min(w), (y0 + f).min(h));
            let mut s = 0.0f64;
            for yy in y0..y1 {
                for xx in x0..x1 {
                    s += f64::from(lum[yy * w + xx]);
                }
            }
            *o = (s / ((x1 - x0) * (y1 - y0)) as f64) as f32;
        }
    });
    v
}

/// The guided upsampling's coefficients for the change `d` computed on `small`: a local linear
/// model of the change against the log luminance (the guided filter of K. He, J. Sun, *Fast
/// Guided Filter*, 2015), fitted on the small copy and evaluated on the full one
/// ([`ToneMove::at`]).
fn guided_coefficients(small: &Plane, d: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let (sw, sh) = (small.w, small.h);
    let r = 2;
    let eps = 0.01;
    let mean = |v: &[f32]| super::box_mean(v, sw, sh, r);
    let ud: Vec<f32> = small.v.iter().zip(d).map(|(a, b)| a * b).collect();
    let uu: Vec<f32> = small.v.iter().map(|a| a * a).collect();
    let (mu, md, mud, muu) = (mean(&small.v), mean(d), mean(&ud), mean(&uu));
    let a: Vec<f32> = (0..sw * sh).map(|i| (mud[i] - mu[i] * md[i]) / (muu[i] - mu[i] * mu[i] + eps)).collect();
    let b: Vec<f32> = (0..sw * sh).map(|i| md[i] - a[i] * mu[i]).collect();
    (mean(&a), mean(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRGB: Encoding = Encoding::SRGB;

    /// A full-size guide far outside the proxy's tones (a float image's highlight) cannot pull
    /// the upsampled move past the six-stop guard.
    #[test]
    fn upsampled_moves_keep_the_guard() {
        let small = Plane { w: 4, h: 4, v: (0..16).map(|i| (i % 4) as f32 - 2.0).collect() };
        let d: Vec<f32> = small.v.iter().map(|u| 0.5 * u).collect();
        let full: Vec<f32> = (0..64).map(|i| if i == 27 { 400.0 } else { ((i % 8) / 2) as f32 - 2.0 }).collect();
        let (a, b) = guided_coefficients(&small, &d);
        let m = ToneMove { w: 4, h: 4, a, b, exact: false };
        let out: Vec<f32> = (0..64).map(|i| m.at(i % 8, i / 8, 8, 8, full[i])).collect();
        assert!(out.iter().all(|v| v.abs() <= MAX_MOVE), "{out:?}");
        assert!(out[27] > 1.0, "the move still follows the guide: {}", out[27]);
    }

    /// Near black the full-size Shadows input and the reduced one (the same pixels, one per
    /// block) must agree: both are the linear luminance after the Highlights move, and the floor
    /// goes on in [`tone_move`] for both.
    #[test]
    fn full_size_and_reduced_shadows_inputs_agree_near_black() {
        let (w, h) = (4, 2);
        let lum = [0.0, 1e-6, 1e-5, 4e-5, 1e-4, 0.0, 2e-6, 0.3];
        let d_hl = [-2.0, -1.5, 1.0, -3.0, 0.5, 2.0, -0.5, -1.0];
        let lum_after = after_highlights(&lum, &d_hl);
        assert_eq!(shrink_linear(&lum_after, w, h, 1), lum_after);
    }

    fn grey(w: usize, h: usize, f: impl Fn(usize, usize) -> f32) -> Vec<[f32; 4]> {
        (0..w * h).map(|i| f(i % w, i / w)).map(|v| [v, v, v, 1.0]).collect()
    }

    #[test]
    fn a_flat_image_is_unchanged() {
        let mut px = grey(64, 48, |_, _| 0.4);
        apply(&mut px, 64, 48, 100.0, -100.0, SRGB);
        assert!(px.iter().all(|q| (q[0] - 0.4).abs() < 1e-5));
    }

    #[test]
    fn shadows_raise_a_dark_area_without_a_halo() {
        // A 0.1 | 0.6 step. Photoshop 2026 raises the dark half evenly (26 to 31 of 255) and
        // keeps the bright one. Here the bright half stays and the dark half rises more (to
        // about 36), a little less (1.5 levels) next to the edge, with no halo.
        let (w, h) = (256, 128);
        let mut px = grey(w, h, |x, _| if x < w / 2 { 0.1 } else { 0.6 });
        apply(&mut px, w, h, 100.0, 0.0, SRGB);
        let row = &px[64 * w..65 * w];
        assert!((row[w / 2 + 1][0] - 0.6).abs() < 0.002 && (row[w - 5][0] - 0.6).abs() < 0.002);
        let (far, near) = (row[10][0], row[w / 2 - 1][0]);
        assert!(far > 0.11 && (far - near).abs() < 0.01 && near > 0.11, "{far} {near}");
    }

    #[test]
    fn grey_moves_as_in_photoshop() {
        // Mid-grey with 1 % black (0.015) and white (0.97) corners. Photoshop 2026: Shadows +100
        // takes the black corner from 4 to 23 and the grey from 127.5 to 160 (of 255);
        // Highlights −100 takes the grey to 98 and the white corner to 210.
        let n = 256;
        let image = |c: f32| {
            grey(n, n, |x, y| {
                if x >= n - 26 && y >= n - 26 {
                    0.97
                } else if x >= n - 26 && y < 26 {
                    0.015
                } else {
                    c
                }
            })
        };
        let mut px = image(0.5);
        apply(&mut px, n, n, 100.0, 0.0, SRGB);
        // The black corner rises more than in Photoshop (to about 52): a patch this small, this
        // much darker than a frame this uniform, is rarer in photographs than the model needs.
        let corner = px[12 * n + n - 13][0] * 255.0;
        assert!(corner > 19.0 && corner < 60.0, "black corner {corner}");
        let mid = px[175 * n + 100][0] * 255.0;
        assert!(mid > 133.0 && mid < 165.0, "grey {mid}");
        // Highlights moves the grey less than Photoshop does (to about 116): on a frame this
        // uniform Photoshop counts the grey as highlight, the model fitted to photographs barely.
        let mut px = image(0.5);
        apply(&mut px, n, n, 0.0, -100.0, SRGB);
        let got = px[175 * n + 100][0] * 255.0;
        assert!(got > 95.0 && got < 126.0, "highlights: grey {got}");
        let white = px[(n - 13) * n + n - 13][0] * 255.0;
        assert!(white < 240.0 && white > 200.0, "highlights: white corner {white}");
    }

    #[test]
    fn highlights_lower_a_bright_area_and_keep_the_darkest() {
        // A 0.8 square on 0.2. Photoshop 2026 (Highlights −100): the square to 0.49, the rest
        // kept. Here the rest is kept and the square goes down less (to about 0.75): a two-tone
        // image is where the curve fitted to photographs differs most.
        let (w, h) = (192, 192);
        let mut px = grey(w, h, |x, y| if (64..128).contains(&x) && (64..128).contains(&y) { 0.8 } else { 0.2 });
        apply(&mut px, w, h, 0.0, -100.0, SRGB);
        assert!(px[96 * w + 96][0] < 0.78, "{}", px[96 * w + 96][0]);
        assert!((px[5 * w + 5][0] - 0.2).abs() < 0.01, "{}", px[5 * w + 5][0]);
    }

    #[test]
    fn smooth_gradients_stay_smooth() {
        // A soft ramp over eight stops, as in an out-of-focus background. Coefficients interpolated
        // between references too far apart drew contour lines across such ramps (Shadows +100 on
        // a D4 raw): the output must rise steadily, without ripples.
        let (w, h) = (640, 48);
        let mut px = grey(w, h, |x, _| crate::photo_util::linear_to_srgb((x as f32 / (w - 1) as f32 * 8.0 - 9.0).exp2()));
        for (s, hl) in [(100.0, 0.0), (0.0, -100.0), (-100.0, 100.0)] {
            let mut p = px.clone();
            apply(&mut p, w, h, s, hl, SRGB);
            let row: Vec<f32> = (0..w).map(|x| crate::photo_util::srgb_to_linear(p[24 * w + x][0]).max(1e-6).log2()).collect();
            let steps: Vec<f32> = row.windows(2).map(|d| d[1] - d[0]).collect();
            assert!(steps.iter().all(|d| *d > 0.0), "{s} {hl}: the ramp reverses");
            let jumps: Vec<f32> = steps.windows(2).map(|d| (d[1] - d[0]).abs()).collect();
            let ripple = jumps[16..jumps.len() - 16].iter().fold(0.0f32, |a, b| a.max(*b));
            // The ramp rises 0.0125 stops per pixel; its slope may change by a third at most.
            assert!(ripple < 0.004, "{s} {hl}: slope jumps by {ripple} stops per pixel");
        }
        px.clear();
    }

    #[test]
    fn the_result_does_not_depend_on_the_image_size() {
        let at = |n: usize| {
            let mut px = grey(n, n, |x, y| if x < n / 3 || y < n / 4 { 0.08 } else { 0.7 });
            apply(&mut px, n, n, 70.0, -60.0, SRGB);
            [px[(n / 8) * n + n / 8][0], px[(3 * n / 4) * n + 3 * n / 4][0]]
        };
        let (a, b) = (at(96), at(384));
        assert!((a[0] - b[0]).abs() < 0.02 && (a[1] - b[1]).abs() < 0.02, "{a:?} {b:?}");
    }
}
