//! Local adjustments with no global slider: Color (a tint), Moiré reduction and Defringe.
//!
//! Ours, modelled on what the controls do in Camera Raw (not measured): Color pushes the
//! pixel's chroma towards the swatch's hue, keeping luminance; Moiré blurs colour (not
//! luminance) over a radius growing with the amount; Defringe takes the colour out of purple and
//! green pixels that sit on a strong luminance edge.

use super::super::luma;
use crate::photo_util::par_rows;

/// Linear-free RGB of a hue (degrees) at full saturation and value.
fn hue_rgb(h: f32) -> [f32; 3] {
    let h = h.rem_euclid(360.0) / 60.0;
    let x = 1.0 - (h % 2.0 - 1.0).abs();
    match h as u32 {
        0 => [1.0, x, 0.0],
        1 => [x, 1.0, 0.0],
        2 => [0.0, 1.0, x],
        3 => [0.0, x, 1.0],
        4 => [x, 0.0, 1.0],
        _ => [1.0, 0.0, x],
    }
}

/// The Color swatch: `sat` 0 … 100 of `hue`.
pub fn tint(px: &mut [[f32; 4]], w: usize, hue: f32, sat: f32) {
    let s = (sat.clamp(0.0, 100.0) / 100.0) * 0.35;
    // `is_nan` first: a NaN amount is neutral, not a NaN tint.
    if s.is_nan() || s <= 0.0 || !hue.is_finite() {
        return;
    }
    let c = hue_rgb(hue);
    let l = luma(c);
    let off = c.map(|v| (v - l) * s);
    par_rows(px, w, 1, |_, row| {
        for q in row.iter_mut() {
            let y = luma([q[0], q[1], q[2]]).clamp(0.0, 1.0);
            // Strongest in the midtones, as a tint over a whole tone range would clip.
            let k = 4.0 * y * (1.0 - y);
            for ch in 0..3 {
                q[ch] += off[ch] * k;
            }
        }
    });
}

fn box_blur(v: &mut [f32], w: usize, h: usize, r: usize) {
    let Some(n) = w.checked_mul(h) else { return };
    if r == 0 || w == 0 || h == 0 || v.len() < n {
        return;
    }
    let mut tmp = vec![0.0f32; n];
    for y in 0..h {
        let row = &v[y * w..(y + 1) * w];
        let mut acc: f32 = row.iter().take(r.min(w - 1) + 1).sum();
        let mut n = (r.min(w - 1) + 1) as f32;
        for x in 0..w {
            tmp[y * w + x] = acc / n;
            if x + r + 1 < w {
                acc += row[x + r + 1];
                n += 1.0;
            }
            if x >= r {
                acc -= row[x - r];
                n -= 1.0;
            }
        }
    }
    for x in 0..w {
        let mut acc = 0.0f32;
        let mut n = 0.0f32;
        for y in 0..=r.min(h - 1) {
            acc += tmp[y * w + x];
            n += 1.0;
        }
        for y in 0..h {
            v[y * w + x] = acc / n;
            if y + r + 1 < h {
                acc += tmp[(y + r + 1) * w + x];
                n += 1.0;
            }
            if y >= r {
                acc -= tmp[(y - r) * w + x];
                n -= 1.0;
            }
        }
    }
}

/// Moiré reduction: colour blurred over up to 0.6 % of the image's long side (`long_side`, not
/// that of the `w × h` region being processed) at 100.
pub fn moire(px: &mut [[f32; 4]], w: usize, h: usize, long_side: usize, amount: f32) {
    let a = amount.clamp(0.0, 100.0) / 100.0;
    let Some(n) = w.checked_mul(h) else { return };
    if a.is_nan() || a <= 0.0 || px.len() < n {
        return;
    }
    let r = ((long_side as f32 * 0.006 * a).round() as usize).max(1);
    let (mut cr, mut cb) = (vec![0.0f32; n], vec![0.0f32; n]);
    for (i, q) in px[..n].iter().enumerate() {
        let y = luma([q[0], q[1], q[2]]);
        cr[i] = q[0] - y;
        cb[i] = q[2] - y;
    }
    box_blur(&mut cr, w, h, r);
    box_blur(&mut cb, w, h, r);
    for (i, q) in px[..n].iter_mut().enumerate() {
        let y = luma([q[0], q[1], q[2]]);
        let (r_, b_) = (y + cr[i], y + cb[i]);
        // Green from luminance: y = 0.2126 r + 0.7152 g + 0.0722 b.
        let g_ = (y - 0.2126 * r_ - 0.0722 * b_) / 0.7152;
        let new = [r_, g_, b_];
        for ch in 0..3 {
            q[ch] += (new[ch] - q[ch]) * a;
        }
    }
}

/// Defringe (0 … 100): purple and green fringes along strong edges lose their colour.
pub fn defringe(px: &mut [[f32; 4]], w: usize, h: usize, amount: f32) {
    let a = amount.clamp(0.0, 100.0) / 100.0;
    let Some(n) = w.checked_mul(h) else { return };
    if a.is_nan() || a <= 0.0 || w < 3 || h < 3 || px.len() < n {
        return;
    }
    let lum: Vec<f32> = px[..n].iter().map(|q| luma([q[0], q[1], q[2]])).collect();
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let i = y * w + x;
            let g = ((lum[i + 1] - lum[i - 1]).abs()).max((lum[i + w] - lum[i - w]).abs());
            let edge = ((g - 0.08) / 0.2).clamp(0.0, 1.0);
            if edge <= 0.0 {
                continue;
            }
            let q = px[i];
            let (mx, mn) = (q[0].max(q[1]).max(q[2]), q[0].min(q[1]).min(q[2]));
            if mx - mn < 1e-4 {
                continue;
            }
            // Purple: blue and red above green; green: green above both.
            let purple = (q[0].min(q[2]) - q[1]).max(0.0);
            let green = (q[1] - q[0].max(q[2])).max(0.0);
            let fringe = ((purple + green) / (mx - mn)).clamp(0.0, 1.0);
            let k = a * edge * fringe;
            let l = lum[i];
            for ch in 0..3 {
                px[i][ch] = q[ch] + (l - q[ch]) * k;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tint_keeps_grey_luminance_and_adds_hue() {
        let mut px = vec![[0.5f32, 0.5, 0.5, 1.0]; 4];
        tint(&mut px, 2, 0.0, 100.0);
        assert!(px[0][0] > 0.55 && px[0][1] < 0.5);
        assert!((luma([px[0][0], px[0][1], px[0][2]]) - 0.5).abs() < 1e-4);
        tint(&mut px, 2, f32::NAN, 100.0);
    }

    #[test]
    fn moire_removes_colour_noise_not_luminance() {
        let (w, h) = (64, 64);
        let mut px: Vec<[f32; 4]> = (0..w * h).map(|i| if (i % w + i / w) % 2 == 0 { [0.6, 0.4, 0.5, 1.0] } else { [0.4, 0.6, 0.5, 1.0] }).collect();
        let before: Vec<f32> = px.iter().map(|q| luma([q[0], q[1], q[2]])).collect();
        moire(&mut px, w, h, w, 100.0);
        let mid = px[32 * w + 32];
        assert!((mid[0] - mid[1]).abs() < 0.1, "{mid:?}");
        let after = luma([mid[0], mid[1], mid[2]]);
        assert!((after - before[32 * w + 32]).abs() < 1e-3);
    }

    #[test]
    fn defringe_greys_purple_edges_only() {
        let (w, h) = (8, 8);
        let mut px: Vec<[f32; 4]> = (0..w * h).map(|i| if i % w < 4 { [0.05, 0.05, 0.05, 1.0] } else { [0.95, 0.95, 0.95, 1.0] }).collect();
        px[3 * w + 4] = [0.7, 0.3, 0.8, 1.0];
        px[3 * w + 1] = [0.7, 0.3, 0.8, 1.0];
        defringe(&mut px, w, h, 100.0);
        let e = px[3 * w + 4];
        assert!((e[0] - e[1]).abs() < 0.2, "edge pixel greyer: {e:?}");
        assert_eq!(px[3 * w + 1], [0.7, 0.3, 0.8, 1.0], "away from edges: kept");
        defringe(&mut [], 0, 0, 50.0);
        moire(&mut [], 0, 0, 0, 50.0);
    }

    #[test]
    fn moire_radius_follows_the_image_not_the_region() {
        // A colour checker: radius 1 (a 20 px region's own long side) and radius 6 (0.6 % of a
        // 1000 px image) blur it differently.
        let (w, h) = (20, 20);
        let src: Vec<[f32; 4]> = (0..w * h).map(|i| if (i % w / 3 + i / w / 3) % 2 == 0 { [0.7, 0.3, 0.5, 1.0] } else { [0.3, 0.7, 0.5, 1.0] }).collect();
        let (mut small, mut large) = (src.clone(), src);
        moire(&mut small, w, h, 20, 100.0);
        moire(&mut large, w, h, 1000, 100.0);
        assert!(small.iter().zip(&large).any(|(a, b)| (a[0] - b[0]).abs() > 0.01));
    }

    #[test]
    fn non_finite_amounts_are_neutral() {
        let src = vec![[0.6f32, 0.2, 0.4, 1.0]; 16];
        let mut px = src.clone();
        tint(&mut px, 4, 0.0, f32::NAN);
        moire(&mut px, 4, 4, 4, f32::NAN);
        defringe(&mut px, 4, 4, f32::NAN);
        assert!(px.iter().zip(&src).all(|(a, b)| a.map(f32::to_bits) == b.map(f32::to_bits)), "{px:?}");
    }

    #[test]
    fn oversized_dimensions_do_not_overflow() {
        moire(&mut [], usize::MAX, 3, 1, 50.0);
        defringe(&mut [], usize::MAX, 3, 50.0);
        let mut v = [0.0f32; 4];
        box_blur(&mut v, usize::MAX, 3, 1);
    }
}
