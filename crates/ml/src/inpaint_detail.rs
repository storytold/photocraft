//! Native-resolution texture transfer for an enlarged LaMa prediction.

use crate::{Cancel, MlError, Rgb, check_cancel};

const PATCH: usize = 16;
const STEP: usize = PATCH / 2;
const RADIUS: usize = 2;
const MAX_DONORS: usize = 1024;

struct Donor {
    x: usize,
    y: usize,
    mean: [f32; 3],
    detail: Vec<[f32; 3]>,
}

/// Transfer only a zero-mean, bounded high-pass layer from known 16px patches, selected by
/// colour and proximity, with overlapping tent weights. This restores native photo texture
/// without inventing white noise or copying the removed subject. LaMa keeps the low frequencies;
/// unlike tiled inference this costs no extra model runs and cannot introduce tile colour seams.
pub(crate) fn restore_detail(src: &[f32], mask: &[f32], filled: &mut [f32], w: usize, h: usize, scale: f32, cancel: Cancel<'_>) -> Result<(), MlError> {
    Rgb::check_dims(w, h)?;
    let n = w * h;
    if src.len() != n * 3 || filled.len() != n * 3 || mask.len() != n || !scale.is_finite() {
        return Err(MlError::BadInput("texture buffers do not match the size or scale".into()));
    }
    check_cancel(cancel)?;
    let strength = (scale - 1.0).clamp(0.0, 1.0);
    if strength == 0.0 || w < PATCH + 2 * RADIUS || h < PATCH + 2 * RADIUS {
        return Ok(());
    }
    let donors = donors(src, mask, w, h, cancel)?;
    if donors.is_empty() {
        return Ok(());
    }
    check_cancel(cancel)?;
    // One donor per overlapping patch, then gather at most four contributions per pixel.
    // No native-resolution RGB scratch buffers or unbounded nearest-neighbour search.
    let (nx, ny) = (w.div_ceil(STEP), h.div_ceil(STEP));
    let mut choices = vec![None; nx * ny];
    for gy in 0..ny {
        check_cancel(cancel)?;
        for gx in 0..nx {
            let (x, y) = (gx * STEP, gy * STEP);
            if !(y..(y + PATCH).min(h)).any(|yy| (x..(x + PATCH).min(w)).any(|xx| mask.get(yy * w + xx).is_some_and(|m| *m >= 0.5))) {
                continue;
            }
            let mean = patch_mean(filled, w, h, x, y);
            let seed = hash((gx as u32).wrapping_add((gy as u32).wrapping_mul(0x9e37_79b9)));
            let phase = [seed as usize % PATCH, (seed >> 16) as usize % PATCH];
            let mut best = f32::INFINITY;
            let mut choice = None;
            for (i, d) in donors.iter().enumerate() {
                let colour: f32 = mean.iter().zip(d.mean).map(|(a, b)| (a - b).powi(2)).sum();
                let distance = (x.abs_diff(d.x) as f32 / w as f32).powi(2) + (y.abs_diff(d.y) as f32 / h as f32).powi(2);
                // Break ties among similar materials instead of stamping the same donor on
                // an 8px lattice. Coordinate hashing keeps repeated runs bit-identical.
                let jitter = hash(seed ^ i as u32) as f32 / u32::MAX as f32;
                let score = colour + 0.002 * distance + 0.0005 * jitter;
                if score < best {
                    best = score;
                    // Very different material should not supply texture (e.g. sky vs brick).
                    choice = Some((i, (1.0 - colour.sqrt() / 0.25).clamp(0.0, 1.0), phase));
                }
            }
            if let Some(c) = choices.get_mut(gy * nx + gx) {
                *c = choice;
            }
        }
    }
    check_cancel(cancel)?;
    for y in 0..h {
        check_cancel(cancel)?;
        for x in 0..w {
            if !mask.get(y * w + x).is_some_and(|m| *m >= 0.5) {
                continue;
            }
            let mut detail = [0.0; 3];
            let mut weight = 0.0;
            for gy in (y / STEP).saturating_sub(1)..=y / STEP {
                for gx in (x / STEP).saturating_sub(1)..=x / STEP {
                    let Some(&(di, gain, phase)) = choices.get(gy * nx + gx).and_then(Option::as_ref) else { continue };
                    let (dx, dy) = (x - gx * STEP, y - gy * STEP);
                    let a = tent(dx) * tent(dy);
                    // Wrap only the high-pass residual; its low-frequency colour never tiles.
                    let si = ((dy + phase[1]) % PATCH) * PATCH + (dx + phase[0]) % PATCH;
                    if let Some(p) = donors.get(di).and_then(|d| d.detail.get(si)) {
                        for (v, d) in detail.iter_mut().zip(p) {
                            *v += d * a * gain;
                        }
                        weight += a;
                    }
                }
            }
            if weight > 0.0 {
                for (c, d) in detail.iter().enumerate() {
                    if let Some(v) = filled.get_mut((y * w + x) * 3 + c) {
                        *v = (*v + strength * d / weight).clamp(0.0, 1.0);
                    }
                }
            }
        }
    }
    Ok(())
}

fn tent(i: usize) -> f32 {
    ((i + 1).min(PATCH - i) as f32 / STEP as f32).min(1.0)
}

fn hash(mut v: u32) -> u32 {
    v = (v ^ (v >> 16)).wrapping_mul(0x7feb_352d);
    v = (v ^ (v >> 15)).wrapping_mul(0x846c_a68b);
    v ^ (v >> 16)
}

fn patch_mean(src: &[f32], w: usize, h: usize, x: usize, y: usize) -> [f32; 3] {
    let mut mean = [0.0; 3];
    let mut count = 0;
    for yy in (y..(y + PATCH).min(h)).step_by(2) {
        for xx in (x..(x + PATCH).min(w)).step_by(2) {
            for (c, v) in mean.iter_mut().enumerate() {
                *v += src.get((yy * w + xx) * 3 + c).copied().unwrap_or(0.0);
            }
            count += 1;
        }
    }
    for v in &mut mean {
        *v /= count.max(1) as f32;
    }
    mean
}

fn donors(src: &[f32], mask: &[f32], w: usize, h: usize, cancel: Cancel<'_>) -> Result<Vec<Donor>, MlError> {
    // At most 32x32 candidates, regardless of photo size. Include the far edges as well.
    let stride_x = ((w - PATCH - 2 * RADIUS) / 31).max(STEP);
    let stride_y = ((h - PATCH - 2 * RADIUS) / 31).max(STEP);
    let mut out = Vec::with_capacity(MAX_DONORS);
    for y in (RADIUS..=h - PATCH - RADIUS).step_by(stride_y).take(32) {
        check_cancel(cancel)?;
        for x in (RADIUS..=w - PATCH - RADIUS).step_by(stride_x).take(32) {
            // All blur taps must be known, so even a masked pixel's colour never leaks out.
            if (y - RADIUS..y + PATCH + RADIUS)
                .any(|yy| (x - RADIUS..x + PATCH + RADIUS).any(|xx| !mask.get(yy * w + xx).is_some_and(|m| m.is_finite() && *m < 0.5)))
            {
                continue;
            }
            let mut detail = Vec::with_capacity(PATCH * PATCH);
            let mut dc = [0.0; 3];
            for yy in y..y + PATCH {
                for xx in x..x + PATCH {
                    let mut p = [0.0; 3];
                    for (c, v) in p.iter_mut().enumerate() {
                        let mut sum = 0.0;
                        for by in yy - RADIUS..=yy + RADIUS {
                            for bx in xx - RADIUS..=xx + RADIUS {
                                sum += src.get((by * w + bx) * 3 + c).copied().unwrap_or(0.0);
                            }
                        }
                        *v = (src.get((yy * w + xx) * 3 + c).copied().unwrap_or(0.0) - sum / 25.0).clamp(-0.08, 0.08);
                    }
                    for (s, v) in dc.iter_mut().zip(p) {
                        *s += v / (PATCH * PATCH) as f32;
                    }
                    detail.push(p);
                }
            }
            for p in &mut detail {
                for (v, m) in p.iter_mut().zip(dc) {
                    *v -= m;
                }
            }
            out.push(Donor { x, y, mean: patch_mean(src, w, h, x, y), detail });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Vec<f32>, Vec<f32>, Vec<f32>) {
        let w = 80;
        let mut src = vec![0.5; w * w * 3];
        let mut mask = vec![0.0; w * w];
        for y in 0..w {
            for x in 0..w {
                let hole = (24..56).contains(&x) && (24..56).contains(&y);
                mask[y * w + x] = if hole { 1.0 } else { 0.0 };
                for c in 0..3 {
                    src[(y * w + x) * 3 + c] = if hole { 1.0 } else { 0.5 + if (x + y) % 2 == 0 { 0.04 } else { -0.04 } };
                }
            }
        }
        (src, mask, vec![0.5; w * w * 3])
    }

    #[test]
    fn restores_texture_without_subject_leak_or_changing_known_pixels() {
        let (mut src, mask, mut filled) = fixture();
        restore_detail(&src, &mask, &mut filled, 80, 80, 3.0, &|| false).unwrap();
        let original = filled.clone();
        for (i, px) in src.as_chunks_mut::<3>().0.iter_mut().enumerate() {
            if mask[i] > 0.5 {
                px.fill(0.0);
            }
        }
        let mut second = vec![0.5; src.len()];
        restore_detail(&src, &mask, &mut second, 80, 80, 3.0, &|| false).unwrap();
        assert_eq!(second, original);
        let mut energy = 0.0;
        let mut mean = 0.0;
        for (i, px) in filled.as_chunks::<3>().0.iter().enumerate() {
            if mask[i] < 0.5 {
                assert_eq!(px, &[0.5; 3]);
            } else {
                energy += (px[0] - 0.5).abs();
                mean += px[0] - 0.5;
            }
        }
        assert!(energy / 1024.0 > 0.008, "texture should survive the overlap: {energy}");
        assert!(mean.abs() / 1024.0 < 0.001, "no colour shift: {mean}");
    }

    #[test]
    fn flat_or_fully_masked_images_stay_flat() {
        for mask in [vec![0.0; 80 * 80], vec![1.0; 80 * 80], fixture().1] {
            let src = vec![0.5; 80 * 80 * 3];
            let mut filled = src.clone();
            restore_detail(&src, &mask, &mut filled, 80, 80, 3.0, &|| false).unwrap();
            assert_eq!(src, filled);
        }
    }

    #[test]
    fn no_upscale_is_identity_and_bad_inputs_are_errors() {
        let (src, mask, mut filled) = fixture();
        restore_detail(&src, &mask, &mut filled, 80, 80, 1.0, &|| false).unwrap();
        assert!(filled.iter().all(|v| *v == 0.5));
        assert!(matches!(restore_detail(&src, &[], &mut filled, 80, 80, 3.0, &|| false), Err(MlError::BadInput(_))));
        assert!(matches!(restore_detail(&src, &mask, &mut filled, usize::MAX, 80, 3.0, &|| false), Err(MlError::BadInput(_))));
    }

    #[test]
    fn odd_sizes_and_edge_holes_are_bounded_and_preserve_known_pixels() {
        let (w, h) = (53, 41);
        let src: Vec<f32> = (0..w * h * 3).map(|i| 0.4 + (i % 19) as f32 * 0.01).collect();
        let mask: Vec<f32> = (0..w * h).map(|i| if i % w > 30 && i / w > 20 { 0.5 } else { 0.0 }).collect();
        let mut filled = src.clone();
        restore_detail(&src, &mask, &mut filled, w, h, 2.0, &|| false).unwrap();
        for (i, (&v, &s)) in filled.iter().zip(&src).enumerate() {
            assert!(v.is_finite() && (0.0..=1.0).contains(&v));
            if mask[i / 3] < 0.5 {
                assert_eq!(v, s);
            }
        }
    }

    #[test]
    fn incompatible_material_does_not_supply_texture() {
        let (mut src, mask, mut filled) = fixture();
        for v in &mut src {
            *v -= 0.35;
        }
        restore_detail(&src, &mask, &mut filled, 80, 80, 3.0, &|| false).unwrap();
        assert!(filled.iter().all(|v| *v == 0.5));
    }

    #[test]
    fn stochastic_surroundings_do_not_turn_into_a_repeating_patch_grid() {
        let w = 192;
        let src: Vec<f32> = (0..w * w * 3).map(|i| 0.5 + (hash((i / 3) as u32) as f32 / u32::MAX as f32 - 0.5) * 0.08).collect();
        let mask: Vec<f32> = (0..w * w).map(|i| if (32..160).contains(&(i % w)) && (32..160).contains(&(i / w)) { 1.0 } else { 0.0 }).collect();
        let mut filled = vec![0.5; src.len()];
        restore_detail(&src, &mask, &mut filled, w, w, 3.0, &|| false).unwrap();
        let (mut covariance, mut energy) = (0.0, 0.0);
        for y in 64..128 {
            for x in 64..128 {
                let a = filled[(y * w + x) * 3] - 0.5;
                let b = filled[(y * w + x + STEP) * 3] - 0.5;
                covariance += a * b;
                energy += a * a;
            }
        }
        assert!(energy > 0.01);
        assert!((covariance / energy).abs() < 0.4, "8px repetition: {}", covariance / energy);
    }

    #[test]
    fn cancellation_during_each_stage_is_reported() {
        use std::cell::Cell;
        for stop in [1, 3, 12, 25] {
            let (src, mask, mut filled) = fixture();
            let calls = Cell::new(0);
            let cancel = || {
                calls.set(calls.get() + 1);
                calls.get() == stop
            };
            assert_eq!(restore_detail(&src, &mask, &mut filled, 80, 80, 3.0, &cancel), Err(MlError::Cancelled));
        }
    }
}
