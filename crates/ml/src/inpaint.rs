//! Large-hole inpainting with LaMa (Suvorov et al., "Resolution-robust Large Mask Inpainting with
//! Fourier Convolutions", WACV 2022; big-lama weights, Apache-2.0).
//!
//! The network is fixed at 512 × 512. A tight context window preserves as much scale as possible;
//! native-resolution high-pass texture from known surroundings restores detail lost on enlargement.
//! Only the masked pixels (plus a hair of feather) are composited back.

use crate::inpaint_detail::restore_detail;
use crate::resize::bilinear;
use crate::store::Model;
use crate::{Cancel, MlError, Rgb, backend, check_cancel};

/// The network's fixed input side.
pub const INPAINT_SIZE: usize = 512;

/// `img` with the pixels where `mask` (`w × h`, `>= 0.5` = hole) is set replaced by what fits their
/// surroundings. An empty mask returns `img` unchanged.
pub fn inpaint(img: &Rgb, mask: &[f32]) -> Result<Rgb, MlError> {
    inpaint_with(img, mask, &|| false)
}

/// [`inpaint`] that stops with [`MlError::Cancelled`] between stages once `cancel` says so.
pub fn inpaint_with(img: &Rgb, mask: &[f32], cancel: Cancel<'_>) -> Result<Rgb, MlError> {
    let (w, h) = (img.w, img.h);
    Rgb::check_dims(w, h)?;
    if img.px.len() != w * h * 3 || mask.len() != w * h {
        return Err(MlError::BadInput("the pixel and mask buffers do not match the size".into()));
    }
    check_cancel(cancel)?;
    let Some(hole) = bounds(mask, w, h) else { return Ok(img.clone()) };
    let crop = context_window(hole, w, h);
    let (cw, ch) = (crop.2 - crop.0, crop.3 - crop.1);

    // Crop, scale to the network's size.
    let mut cimg = Vec::with_capacity(cw * ch * 3);
    let mut cmask = Vec::with_capacity(cw * ch);
    for y in crop.1..crop.3 {
        check_cancel(cancel)?;
        let row = y * w;
        cimg.extend_from_slice(img.px.get((row + crop.0) * 3..(row + crop.2) * 3).ok_or_else(|| MlError::BadInput("crop outside the image".into()))?);
        cmask.extend(
            mask.get(row + crop.0..row + crop.2)
                .ok_or_else(|| MlError::BadInput("crop outside the mask".into()))?
                .iter()
                .map(|m| if *m >= 0.5 { 1.0f32 } else { 0.0 }),
        );
    }
    check_cancel(cancel)?;
    let n = INPAINT_SIZE;
    let small = bilinear(&cimg, cw, ch, 3, n, n);
    // Nearest-ish for the mask: bilinear then threshold low, so a thin hole never vanishes.
    let small_mask: Vec<f32> = bilinear(&cmask, cw, ch, 1, n, n).iter().map(|m| if *m > 0.1 { 1.0 } else { 0.0 }).collect();
    let mut nchw = vec![0.0f32; 3 * n * n];
    for (i, px) in small.as_chunks::<3>().0.iter().enumerate() {
        for (c, v) in px.iter().enumerate() {
            if let Some(o) = nchw.get_mut(c * n * n + i) {
                *o = *v;
            }
        }
    }
    check_cancel(cancel)?;
    let out = backend::run_with(Model::Lama, &[(&[1, 3, n, n], nchw), (&[1, 1, n, n], small_mask)], cancel)?;
    check_cancel(cancel)?;
    if out.len() != 3 * n * n {
        return Err(MlError::Backend(format!("unexpected output of {} values", out.len())));
    }
    // The export returns 0..255; scale to 0..1 (values are clamped, so a 0..1 export would show as
    // a black fill rather than corrupt data, which the tests would catch).
    let scale = if out.iter().cloned().fold(0.0f32, f32::max) > 2.0 { 1.0 / 255.0 } else { 1.0 };
    let mut filled = vec![0.0f32; n * n * 3];
    for i in 0..n * n {
        for c in 0..3 {
            if let (Some(o), Some(v)) = (filled.get_mut(i * 3 + c), out.get(c * n * n + i)) {
                *o = (v * scale).clamp(0.0, 1.0);
            }
        }
    }
    check_cancel(cancel)?;
    let mut big = bilinear(&filled, n, n, 3, cw, ch);
    check_cancel(cancel)?;
    restore_detail(&cimg, &cmask, &mut big, cw, ch, cw.max(ch) as f32 / n as f32, cancel)?;
    check_cancel(cancel)?;

    // Composite: the hole plus a 2 px feather takes the fill; everything else is untouched.
    let alpha = feathered(&cmask, cw, ch, 2);
    check_cancel(cancel)?;
    let mut res = img.clone();
    for y in 0..ch {
        check_cancel(cancel)?;
        for x in 0..cw {
            let a = alpha.get(y * cw + x).copied().unwrap_or(0.0);
            if a <= 0.0 {
                continue;
            }
            let di = ((y + crop.1) * w + x + crop.0) * 3;
            for c in 0..3 {
                if let (Some(d), Some(f)) = (res.px.get_mut(di + c), big.get((y * cw + x) * 3 + c)) {
                    *d += (f - *d) * a;
                }
            }
        }
    }
    Ok(res)
}

/// Bounding box `(x0, y0, x1, y1)` (exclusive end) of the set pixels.
fn bounds(mask: &[f32], w: usize, h: usize) -> Option<(usize, usize, usize, usize)> {
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
    for y in 0..h {
        for x in 0..w {
            if mask.get(y * w + x).is_some_and(|m| *m >= 0.5) {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
            }
        }
    }
    (x1 > x0).then_some((x0, y0, x1, y1))
}

/// Small holes retain up to their own width of context; larger ones get 64px per side, rather
/// than spending half the fixed network resolution on context. Keep the complete hole, slide
/// the window inside the image, and retain at least 256px for enough surrounding structure.
fn context_window(hole: (usize, usize, usize, usize), w: usize, h: usize) -> (usize, usize, usize, usize) {
    let side = (hole.2 - hole.0).max(hole.3 - hole.1);
    let want = side.saturating_add(side.min(128)).max(256);
    let place = |lo: usize, hi: usize, limit: usize| {
        let len = want.min(limit);
        let mid = (lo + hi) / 2;
        let start = mid.saturating_sub(len / 2).min(limit - len);
        (start, start + len)
    };
    let (x0, x1) = place(hole.0, hole.2, w);
    let (y0, y1) = place(hole.1, hole.3, h);
    (x0, y0, x1, y1)
}

/// `mask` (0/1) grown outward by up to `r` pixels with a linear falloff; the hole itself stays 1.
fn feathered(mask: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut out = mask.to_vec();
    if r == 0 {
        return out;
    }
    for y in 0..h {
        for x in 0..w {
            if mask.get(y * w + x).is_some_and(|m| *m >= 0.5) {
                continue;
            }
            let mut best = 0.0f32;
            for dy in -(r as isize)..=r as isize {
                for dx in -(r as isize)..=r as isize {
                    let (nx, ny) = (x as isize + dx, y as isize + dy);
                    if nx < 0 || ny < 0 || nx as usize >= w || ny as usize >= h {
                        continue;
                    }
                    if mask.get(ny as usize * w + nx as usize).is_some_and(|m| *m >= 0.5) {
                        let d = ((dx * dx + dy * dy) as f32).sqrt();
                        best = best.max((1.0 - d / (r as f32 + 1.0)).max(0.0));
                    }
                }
            }
            if let Some(o) = out.get_mut(y * w + x) {
                *o = best;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_and_window() {
        let mut m = vec![0.0f32; 100 * 80];
        assert_eq!(bounds(&m, 100, 80), None);
        m[10 * 100 + 20] = 1.0;
        m[30 * 100 + 60] = 1.0;
        assert_eq!(bounds(&m, 100, 80), Some((20, 10, 61, 31)));
        // Window: square of 256 clipped to the image (80 high), slid inside.
        let (x0, y0, x1, y1) = context_window((20, 10, 61, 31), 100, 80);
        assert_eq!((y0, y1), (0, 80));
        assert!(x1 <= 100 && x0 < x1);
        let (x0, y0, x1, y1) = context_window((990, 990, 1000, 1000), 1000, 1000);
        assert_eq!((x1 - x0, y1 - y0), (256, 256));
        assert_eq!((x1, y1), (1000, 1000));
    }

    #[test]
    fn large_holes_keep_context_without_wasting_half_the_network() {
        let hole = (1000, 900, 1490, 1850);
        let crop = context_window(hole, 3000, 3000);
        assert_eq!((crop.2 - crop.0, crop.3 - crop.1), (1078, 1078));
        assert!(crop.0 < hole.0 && crop.1 < hole.1 && crop.2 > hole.2 && crop.3 > hole.3);
        let edge = context_window((0, 0, 950, 950), 3000, 3000);
        assert_eq!(edge, (0, 0, 1078, 1078));
    }

    #[test]
    fn pre_cancelled_inpainting_never_needs_a_model() {
        let img = Rgb::new(6, 6, vec![0.5; 108]).unwrap();
        assert_eq!(inpaint_with(&img, &[1.0; 36], &|| true), Err(MlError::Cancelled));
        assert_eq!(inpaint_with(&img, &[0.0; 36], &|| true), Err(MlError::Cancelled));
    }

    #[test]
    fn empty_mask_is_identity_without_a_model() {
        let img = Rgb::new(6, 6, (0..108).map(|i| i as f32 / 108.0).collect()).unwrap();
        assert_eq!(inpaint(&img, &[0.0; 36]).unwrap(), img);
    }

    #[test]
    fn bad_input_is_an_error() {
        let img = Rgb::new(6, 6, vec![0.5; 108]).unwrap();
        assert!(matches!(inpaint(&img, &[1.0; 5]), Err(MlError::BadInput(_))));
    }

    #[test]
    fn feather_keeps_the_hole_and_fades_outward() {
        let mut m = vec![0.0f32; 11 * 11];
        m[5 * 11 + 5] = 1.0;
        let f = feathered(&m, 11, 11, 2);
        assert_eq!(f[5 * 11 + 5], 1.0);
        assert!(f[5 * 11 + 6] > f[5 * 11 + 7] && f[5 * 11 + 7] > 0.0);
        assert_eq!(f[5 * 11 + 9], 0.0);
    }
}
