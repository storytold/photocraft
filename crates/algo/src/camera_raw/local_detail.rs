//! Texture and Clarity as fitted multi-scale models of local detail.
//!
//! Measured as a black box on Photoshop 2026's Camera Raw Filter (Camera Raw 18.6): no Adobe
//! code or data was used. What the measurements show:
//!
//! - Texture works on detail about 0.4 % of the image's long side across (a Gaussian of that
//!   size explains three quarters of the change on its own), boosts small details more than
//!   strong edges, and moves colour detail as well as lightness (matching the lightness exactly
//!   still leaves a mean ΔE76 of 1.5 at +100).
//! - Clarity works on larger detail (2–3 % of the long side), on lightness, and also shifts
//!   whole regions lighter or darker in a way that depends on the picture.
//!
//! The model, on the sRGB-encoded pixel with encoded luminance `s_y`, L* `l` (0 … 1):
//! `move = Σ_j b_j(l) · (k0_j + Σ_s f_sj(D_s) [+ Σ_s g_sj(C_s)])`, where `b_j` are linear
//! interpolation weights over [`KNOTS`] knots on 0 … 1, `D_s = s_y − G_s(s_y)` the luminance
//! detail at Gaussian scale `s` (a share of the long side), and each `f` is
//! `kp · max(x, 0) + kn · min(x, 0) + Σ_a ka · clamp(x, −a, a)`. Texture works per channel: `C_s`
//! is the channel's colour detail `(s_c − s_y) − G_s(s_c − s_y)` and `s_c += move_c`. Clarity
//! moves the luminance only: `s_y += move` with the pixel's chromaticity kept (linear RGB scaled).
//! The coefficients were fitted by least squares (smoothness penalty along `l`) to Photoshop's
//! results at −100, −50, +50 and +100 on 16 photographs and renders; between those the
//! coefficients interpolate, and below ±50 they scale toward 0.
//!
//! Mean ΔE76 against Photoshop on the 17 images (full size, before → after): Texture −100 1.87
//! → 0.71, −50 1.06 → 0.45, +50 1.15 → 0.51, +100 1.93 → 0.78.

use super::Encoding;
use super::local_detail_tables as t;
use crate::fxutil::try_gauss_blur_n;
use crate::photo_util::{linear_to_srgb, srgb_to_linear};

/// Knots of `l`: 0, 1/7, … 1.
pub(super) const KNOTS: usize = 8;

/// One slider's model.
pub(super) struct Model {
    scales: &'static [f32],
    amps: &'static [f32],
    /// Per channel with colour detail (Texture), or luminance only (Clarity).
    colour: bool,
    down_50: &'static [f32],
    down_100: &'static [f32],
    up_50: &'static [f32],
    up_100: &'static [f32],
}

pub(super) const TEXTURE: Model = Model {
    scales: &[0.001, 0.002, 0.004, 0.008, 0.016],
    amps: &[0.02],
    colour: true,
    down_50: &t::TEXTURE_DOWN_50,
    down_100: &t::TEXTURE_DOWN_100,
    up_50: &t::TEXTURE_UP_50,
    up_100: &t::TEXTURE_UP_100,
};

pub(super) const CLARITY: Model = Model {
    scales: &[0.004, 0.008, 0.016, 0.03, 0.06, 0.12],
    amps: &[0.02, 0.08],
    colour: false,
    down_50: &t::CLARITY_DOWN_50,
    down_100: &t::CLARITY_DOWN_100,
    up_50: &t::CLARITY_UP_50,
    up_100: &t::CLARITY_UP_100,
};

impl Model {
    /// Feature blocks per scale and plane: max, min, then one per clamp amplitude.
    fn per_scale(&self) -> usize {
        2 + self.amps.len()
    }

    /// Coefficients in one table.
    fn coeffs(&self) -> usize {
        (1 + (1 + usize::from(self.colour)) * self.scales.len() * self.per_scale()) * KNOTS
    }

    /// The coefficients at slider `v` (−100 … 100), `None` at 0 or for a malformed table.
    fn coefficients(&self, v: f32) -> Option<Vec<f32>> {
        if !v.is_finite() || v == 0.0 {
            return None;
        }
        let v = v.clamp(-100.0, 100.0);
        let (t50, t100) = if v < 0.0 { (self.down_50, self.down_100) } else { (self.up_50, self.up_100) };
        if t50.len() != self.coeffs() || t100.len() != self.coeffs() {
            return None;
        }
        let a = v.abs();
        Some(if a <= 50.0 {
            t50.iter().map(|k| k * a / 50.0).collect()
        } else {
            let f = (a - 50.0) / 50.0;
            t50.iter().zip(t100).map(|(x, y)| x + (y - x) * f).collect()
        })
    }
}

fn lstar(y: f32) -> f32 {
    if y > 216.0 / 24389.0 { 116.0 * y.cbrt() - 16.0 } else { 24389.0 / 27.0 * y }
}

/// `n` copies of `v`, or `None` when that much memory can't be had (the stage is then skipped
/// rather than aborting the app).
fn filled<T: Clone>(n: usize, v: T) -> Option<Vec<T>> {
    let mut out = Vec::new();
    out.try_reserve_exact(n).ok()?;
    out.resize(n, v);
    Some(out)
}

/// Applies `model` at slider `v` (−100 … 100) to straight RGBA pixels (`w × h`, encoded with
/// `enc`). Without the memory for its working planes (about 80 bytes a pixel for Texture, a
/// little more with transparency) or for a blur's scratch row the pixels are left unchanged.
/// The blurs weight each pixel by its alpha, so the colour hidden under transparent pixels takes
/// no part; a fully opaque image skips the weighting.
pub(super) fn apply(model: &Model, px: &mut [[f32; 4]], w: usize, h: usize, v: f32, enc: Encoding) {
    let Some(k) = model.coefficients(v) else { return };
    let n = w.saturating_mul(h);
    if n == 0 || px.len() != n {
        return;
    }
    let long = w.max(h) as f32;
    let to = |x: f32| if enc.0 > 0.0 { linear_to_srgb(enc.decode(x)) } else { x };
    let from = |x: f32| if enc.0 > 0.0 { enc.encode(srgb_to_linear(x)) } else { x };
    // Planes: encoded luminance, then (Texture) each channel's colour (channel − luminance).
    let np = if model.colour { 4 } else { 1 };
    let cover = |q: &[f32; 4]| if q[3].is_finite() { q[3].clamp(0.0, 1.0) } else { 0.0 };
    // With any transparency the blurs run on alpha-weighted planes plus the alpha itself.
    let weighted = px.iter().any(|q| cover(q) < 1.0);
    let nb = np + usize::from(weighted);
    let (Some(planes_len), Some(blur_len)) = (n.checked_mul(np), n.checked_mul(nb)) else { return };
    let (Some(mut planes), Some(mut knot), Some(mut lum), Some(mut mv), Some(mut blur), Some(mut src)) = (
        filled(planes_len, 0.0f32),
        filled(n, (0usize, 0.0f32)),
        filled(n, 0.0f32),
        filled(n, [0.0f32; 3]),
        filled(blur_len, 0.0f32),
        filled(if weighted { blur_len } else { 0 }, 0.0f32),
    ) else {
        return;
    };
    for (i, q) in px.iter().enumerate() {
        // A value that is not a number would spread through the blurs to its neighbours.
        let s = [0, 1, 2].map(|c| Some(to(q[c])).filter(|v| v.is_finite()).unwrap_or(0.0));
        let lin = s.map(srgb_to_linear);
        let y = (0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2]).max(0.0);
        let sy = linear_to_srgb(y);
        lum[i] = y;
        planes[i * np] = sy;
        if model.colour {
            for c in 0..3 {
                planes[i * np + 1 + c] = s[c] - sy;
            }
        }
        if weighted {
            let a = cover(q);
            for c in 0..np {
                src[i * nb + c] = planes[i * np + c] * a;
            }
            src[i * nb + np] = a;
        }
        let f = (lstar(y) / 100.0).clamp(0.0, 1.0) * (KNOTS - 1) as f32;
        let j = (f as usize).min(KNOTS - 2);
        knot[i] = (j, f - j as f32);
    }
    let coef = |block: usize, i: usize| -> f32 {
        let (j, t) = knot[i];
        let b = block * KNOTS + j;
        k.get(b).copied().unwrap_or(0.0) * (1.0 - t) + k.get(b + 1).copied().unwrap_or(0.0) * t
    };
    let ps = model.per_scale();
    let term = |block: usize, i: usize, d: f32| -> f32 {
        let mut v = coef(block, i) * d.max(0.0) + coef(block + 1, i) * d.min(0.0);
        for (a, amp) in model.amps.iter().enumerate() {
            v += coef(block + 2 + a, i) * d.clamp(-amp, *amp);
        }
        v
    };
    // Moves: one per channel (Texture) or one for the luminance (Clarity, in `mv[i][0]`).
    for (i, m) in mv.iter_mut().enumerate() {
        *m = [coef(0, i); 3];
    }
    let ns = model.scales.len();
    for (si, s) in model.scales.iter().enumerate() {
        blur.copy_from_slice(if weighted { &src } else { &planes });
        // Without the memory for the blur's scratch row, the pixels are left unchanged.
        if try_gauss_blur_n(&mut blur, w, h, nb, s * long).is_none() {
            return;
        }
        // Plane `c` blurred at pixel `i`: the weighted blur over the blurred alpha; where nothing
        // visible is near, no detail.
        let blurred = |i: usize, c: usize| -> f32 {
            if !weighted {
                return blur[i * nb + c];
            }
            let a = blur[i * nb + np];
            if a > 1e-6 { blur[i * nb + c] / a } else { planes[i * np + c] }
        };
        for (i, m) in mv.iter_mut().enumerate() {
            let dl = term(1 + ps * si, i, planes[i * np] - blurred(i, 0));
            for c in 0..3 {
                m[c] += dl;
                if model.colour {
                    m[c] += term(1 + ps * ns + ps * si, i, planes[i * np + 1 + c] - blurred(i, 1 + c));
                }
            }
        }
    }
    for ((q, m), y) in px.iter_mut().zip(&mv).zip(&lum) {
        let inside = q[..3].iter().all(|x| (0.0..=1.0).contains(x));
        let fit = |s: f32| if inside { s.clamp(0.0, 1.0) } else { s };
        if model.colour {
            for c in 0..3 {
                q[c] = from(fit(to(q[c]) + m[c].clamp(-1.0, 1.0)));
            }
        } else {
            // The luminance moves, the chromaticity stays: linear RGB scaled.
            let target = srgb_to_linear(fit(linear_to_srgb(*y) + m[0].clamp(-1.0, 1.0)));
            let r = if *y > 1e-6 { target / y } else { 1.0 };
            for c in 0..3 {
                let lin = srgb_to_linear(to(q[c])) * r;
                q[c] = from(linear_to_srgb(if inside { lin.clamp(0.0, 1.0) } else { lin }));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn working_planes_too_large_for_memory_are_refused() {
        assert!(filled(usize::MAX / 2, 0.0f32).is_none());
        assert_eq!(filled(3, 1u8), Some(vec![1, 1, 1]));
    }

    /// The blur's scratch row is reserved fallibly: a row too large for memory is refused with
    /// the buffer untouched, and otherwise the blur is the plain one.
    #[test]
    fn blur_scratch_too_large_for_memory_is_refused() {
        let mut empty: [f32; 0] = [];
        assert!(try_gauss_blur_n(&mut empty, usize::MAX / 8, 1, 4, 2.0).is_none());
        assert!(try_gauss_blur_n(&mut empty, usize::MAX, 1, 4, 2.0).is_none());
        let src: Vec<f32> = (0..12 * 9 * 2).map(|i| ((i * 37) % 11) as f32 / 10.0).collect();
        let (mut a, mut b) = (src.clone(), src.clone());
        assert!(try_gauss_blur_n(&mut a, 12, 9, 2, 1.5).is_some());
        crate::fxutil::gauss_blur_n(&mut b, 12, 9, 2, 1.5);
        assert_eq!(a, b);
        assert_ne!(a, src);
    }

    /// A pixel that is not a number stays its own problem: its neighbours develop as without it.
    #[test]
    fn a_nan_pixel_does_not_spread() {
        let (w, h) = (48, 48);
        for m in [&TEXTURE, &CLARITY] {
            let mut clean = image(w, h);
            let mut bad = clean.clone();
            bad[24 * w + 24] = [f32::NAN, f32::INFINITY, 0.5, 1.0];
            apply(m, &mut clean, w, h, 80.0, Encoding(0.0));
            apply(m, &mut bad, w, h, 80.0, Encoding(0.0));
            for (i, q) in bad.iter().enumerate().filter(|(i, _)| *i != 24 * w + 24) {
                assert!(q[..3].iter().all(|v| v.is_finite()), "pixel {i}: {q:?}");
            }
            assert!(bad[0][..3].iter().zip(&clean[0][..3]).all(|(a, b)| (a - b).abs() < 1e-6));
        }
    }

    /// The colour hidden under fully transparent pixels doesn't reach the visible pixels next to
    /// them; a fully opaque image develops exactly as without the alpha weighting.
    #[test]
    fn transparent_pixels_stay_out_of_the_blurs() {
        let (w, h) = (48, 48);
        let visible = |i: usize| i % w < 24;
        for m in [&TEXTURE, &CLARITY] {
            let hide = |hidden: [f32; 4]| -> Vec<[f32; 4]> { image(w, h).into_iter().enumerate().map(|(i, q)| if visible(i) { q } else { hidden }).collect() };
            let (mut white, mut black) = (hide([1.0, 1.0, 1.0, 0.0]), hide([0.0, 0.0, 0.0, 0.0]));
            apply(m, &mut white, w, h, 100.0, Encoding(0.0));
            apply(m, &mut black, w, h, 100.0, Encoding(0.0));
            for i in (0..w * h).filter(|i| visible(*i)) {
                assert!(white[i][..3].iter().zip(&black[i][..3]).all(|(a, b)| (a - b).abs() < 1e-5), "pixel {i}: {:?} {:?}", white[i], black[i]);
            }
        }
    }

    /// A fully opaque image develops as before the alpha weighting: these pixels are what the
    /// unweighted blurs gave (tolerance for platform maths libraries only).
    #[test]
    fn opaque_images_develop_as_before() {
        let (w, h) = (48, 48);
        let before: [[[f32; 3]; 5]; 2] = [
            [[0.3492603, 0.39940542, 0.29940543], [0.46303654, 0.48386988, 0.3427373], [0.4463153, 0.3963153, 0.29834855], [0.5589523, 0.48395225, 0.34287918], [0.54581106, 0.3998331, 0.29983312]],
            [[0.35134986, 0.40151656, 0.30118325], [0.45318657, 0.4737776, 0.33540592], [0.4706822, 0.41863444, 0.314539], [0.54839313, 0.4742055, 0.3357218], [0.5495149, 0.40278795, 0.30217522]],
        ];
        for ((m, v), want) in [(&TEXTURE, 70.0), (&CLARITY, -60.0)].into_iter().zip(before) {
            let mut px = image(w, h);
            apply(m, &mut px, w, h, v, Encoding(0.0));
            for (i, b) in [0usize, 5 * w + 7, 24 * w + 24, 47 * w + 30, w * h - 1].into_iter().zip(want) {
                assert!(px[i][..3].iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6), "pixel {i}: {:?} vs {b:?}", px[i]);
            }
        }
    }

    fn image(w: usize, h: usize) -> Vec<[f32; 4]> {
        (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let t = ((x / 3 + y / 3) % 2) as f32 * 0.08;
                [0.35 + t + 0.2 * x as f32 / w as f32, 0.4 + t, 0.3 + t * 0.5, 1.0]
            })
            .collect()
    }

    fn contrast(px: &[[f32; 4]]) -> f32 {
        let m = px.iter().map(|q| q[1]).sum::<f32>() / px.len() as f32;
        px.iter().map(|q| (q[1] - m).abs()).sum::<f32>() / px.len() as f32
    }

    #[test]
    fn tables_fit_their_models() {
        for m in [&TEXTURE, &CLARITY] {
            for v in [-100.0, -30.0, 70.0] {
                assert!(m.coefficients(v).is_some(), "{v}");
            }
        }
    }

    #[test]
    fn texture_sharpens_and_softens_fine_detail() {
        let (w, h) = (300, 200);
        let base = image(w, h);
        let mut up = base.clone();
        apply(&TEXTURE, &mut up, w, h, 100.0, Encoding(0.0));
        let mut down = base.clone();
        apply(&TEXTURE, &mut down, w, h, -100.0, Encoding(0.0));
        assert!(contrast(&up) > contrast(&base) && contrast(&down) < contrast(&base), "{} {} {}", contrast(&up), contrast(&base), contrast(&down));
        assert!(up.iter().chain(&down).all(|q| q[..3].iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)) && q[3] == 1.0));
    }

    #[test]
    fn clarity_raises_and_lowers_midtone_contrast() {
        let (w, h) = (300, 200);
        let base: Vec<[f32; 4]> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let t = ((x / 12 + y / 12) % 2) as f32 * 0.15;
                [0.3 + t, 0.32 + t, 0.28 + t, 1.0]
            })
            .collect();
        let mut up = base.clone();
        apply(&CLARITY, &mut up, w, h, 100.0, Encoding(0.0));
        let mut down = base.clone();
        apply(&CLARITY, &mut down, w, h, -100.0, Encoding(0.0));
        assert!(contrast(&up) > contrast(&base) && contrast(&down) < contrast(&base), "{} {} {}", contrast(&up), contrast(&base), contrast(&down));
    }

    #[test]
    fn a_flat_image_and_hostile_input_are_safe() {
        let (w, h) = (40, 30);
        for m in [&TEXTURE, &CLARITY] {
            let mut flat = vec![[0.5f32, 0.4, 0.3, 1.0]; w * h];
            apply(m, &mut flat, w, h, 100.0, Encoding(2.2));
            assert!(flat.iter().all(|q| (q[1] - 0.4).abs() < 0.05));
            let mut odd = vec![[f32::NAN, 2.0, -1.0, 1.0]; 6];
            apply(m, &mut odd, 3, 2, 60.0, Encoding(0.0));
            apply(m, &mut odd, 5, 5, 60.0, Encoding(0.0)); // wrong size: left alone
            apply(m, &mut [], 0, 0, 60.0, Encoding(0.0));
            assert!(m.coefficients(0.0).is_none() && m.coefficients(f32::NAN).is_none());
            assert_eq!(m.coefficients(300.0), m.coefficients(100.0));
        }
    }
}
