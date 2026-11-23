use photocraft_geom::Rect;
use photocraft_raster::Surface;

use super::{Encoding, luma, percentile};

const PATCH_SIDE: usize = 6;
const PATCH_STEP: usize = 56;

/// Estimate high-frequency variation on a bounded grid of native-pixel shadow patches.
/// The predicate selects patch centres inside any selection or filter mask.
pub fn measure_shadow_noise(surface: &Surface, area: Rect, gamma: f32, included: impl Fn(i32, i32) -> bool) -> f32 {
    let step = |len: u32| (len as usize).div_ceil(PATCH_STEP).max(1);
    let side = PATCH_SIDE as i32;
    let mut estimates = Vec::new();
    let mut patch = [[0.0; 4]; PATCH_SIDE * PATCH_SIDE];
    let enc = Encoding::of(gamma);
    for y in (area.y0..area.y1.saturating_sub(side - 1)).step_by(step(area.height())) {
        for x in (area.x0..area.x1.saturating_sub(side - 1)).step_by(step(area.width())) {
            if !included(x + side / 2, y + side / 2) {
                continue;
            }
            surface.read_rgba_into(Rect::new(x, y, x + side, y + side), &mut patch);
            if let Some(value) = patch_noise(&mut patch, enc, gamma == 1.8) {
                estimates.push(value);
            }
        }
    }
    if estimates.len() < 48 {
        return 0.0;
    }
    // A lower quartile resists fine texture and isolated edges while allowing for a
    // few nearly uniform or clipped patches. Below it we cannot reliably infer noise.
    percentile(&mut estimates, 0.25)
}

fn patch_noise(patch: &mut [[f32; 4]; PATCH_SIDE * PATCH_SIDE], enc: Encoding, gamma_18: bool) -> Option<f32> {
    let mut sum = 0.0;
    for [r, g, b, a] in &mut *patch {
        if *a < 0.5 || !a.is_finite() || !r.is_finite() || !g.is_finite() || !b.is_finite() {
            return None;
        }
        for v in [&mut *r, &mut *g, &mut *b] {
            *v = v.clamp(0.0, 1.0);
            if !gamma_18 {
                // Use a common transfer curve so the noise thresholds have the same
                // meaning in linear and gamma-encoded documents.
                *v = enc.decode(*v).powf(1.0 / 1.8);
            }
        }
        sum += luma([*r, *g, *b]);
    }
    if !(0.06..0.25).contains(&(sum / patch.len() as f32)) {
        return None;
    }
    let mut residual = 0.0;
    let (rows, _) = patch.as_chunks::<PATCH_SIDE>();
    for [top, bottom] in rows.as_chunks::<2>().0 {
        for ([a, b], [c, d]) in top.as_chunks::<2>().0.iter().zip(bottom.as_chunks::<2>().0) {
            // The diagonal sum cancels a local plane, including linear gradients.
            for i in 0..3 {
                residual += (a[i] + d[i] - b[i] - c[i]).abs();
            }
        }
    }
    Some(residual / (3 * (patch.len() / 4)) as f32)
}
