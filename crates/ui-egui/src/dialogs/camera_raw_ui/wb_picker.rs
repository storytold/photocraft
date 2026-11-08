//! Camera Raw's White Balance tool (I): while it is on, a click on the preview sets Temperature
//! and Tint so the clicked spot turns neutral. The colour is read from the image before Camera
//! Raw (the proxy), at the source point the shown pixel came from when Geometry moves it, averaged
//! over a 3 × 3 block as Camera Raw's tool reads a small area.

use photocraft_algo::camera_raw::{geometry, white_balance_from_sample};
use photocraft_algo::transform::Homography;

use super::{CameraRawDialog, ERect, Pos2};

/// Where the point `u` (0 … 1 across the shown image) comes from in the proxy (0 … 1).
pub(super) fn source_point(d: &CameraRawDialog, u: [f32; 2]) -> Option<[f32; 2]> {
    if d.params.geometry_neutral() {
        return Some(u);
    }
    let (w, h) = (d.pw.max(1) as f64, d.ph.max(1) as f64);
    let m = geometry::homography(&d.proxy, d.pw.max(1), d.ph.max(1), &d.params);
    let inv = Homography(m).inverse()?.0;
    let (x, y) = (f64::from(u[0]) * w - w / 2.0, f64::from(u[1]) * h - h / 2.0);
    let z = inv[6] * x + inv[7] * y + inv[8];
    if !z.is_finite() || z.abs() < 1e-12 {
        return None;
    }
    let (sx, sy) = ((inv[0] * x + inv[1] * y + inv[2]) / z, (inv[3] * x + inv[4] * y + inv[5]) / z);
    let out = [((sx + w / 2.0) / w) as f32, ((sy + h / 2.0) / h) as f32];
    out.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)).then_some(out)
}

/// The mean colour of the proxy's 3 × 3 block around `u` (0 … 1), ignoring non-finite pixels.
pub(super) fn sample(px: &[[f32; 4]], pw: usize, ph: usize, u: [f32; 2]) -> Option<[f32; 3]> {
    if pw == 0 || ph == 0 || !u.iter().all(|v| v.is_finite()) {
        return None;
    }
    let cx = ((u[0].clamp(0.0, 1.0) * pw as f32) as usize).min(pw - 1);
    let cy = ((u[1].clamp(0.0, 1.0) * ph as f32) as usize).min(ph - 1);
    let (mut sum, mut n) = ([0.0f32; 3], 0u32);
    for y in cy.saturating_sub(1)..=(cy + 1).min(ph - 1) {
        for x in cx.saturating_sub(1)..=(cx + 1).min(pw - 1) {
            let Some(p) = y.checked_mul(pw).and_then(|i| i.checked_add(x)).and_then(|i| px.get(i)) else { continue };
            if p[..3].iter().all(|v| v.is_finite()) {
                for c in 0..3 {
                    sum[c] += p[c];
                }
                n += 1;
            }
        }
    }
    (n > 0).then(|| sum.map(|s| s / n as f32))
}

/// The Temperature and Tint that neutralise the shown point `u` (0 … 1), or `None` when that
/// spot has no colour to read (too dark, clipped white, off the image).
pub(super) fn balance_at(d: &CameraRawDialog, u: [f32; 2]) -> Option<(f32, f32)> {
    let src = source_point(d, u)?;
    let rgb = sample(&d.proxy, d.pw, d.ph, src)?;
    white_balance_from_sample(rgb, d.params.encoding_gamma)
}

/// The tool on the preview: a crosshair over the image `r`; a click sets the balance (one
/// undo step, like a slider drag).
pub(super) fn preview(ui: &egui::Ui, d: &mut CameraRawDialog, r: ERect, response: &egui::Response) {
    let over = response.hover_pos().filter(|p| r.contains(*p));
    if over.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    if response.clicked()
        && let Some(p) = response.interact_pointer_pos().filter(|p| r.contains(*p))
    {
        let u = unit(r, p);
        if let Some((temperature, tint)) = balance_at(d, u)
            && (d.params.temperature, d.params.tint) != (temperature, tint)
        {
            (d.params.temperature, d.params.tint) = (temperature, tint);
            d.dirty = true;
        }
    }
}

fn unit(r: ERect, p: Pos2) -> [f32; 2] {
    [((p.x - r.left()) / r.width().max(1.0)).clamp(0.0, 1.0), ((p.y - r.top()) / r.height().max(1.0)).clamp(0.0, 1.0)]
}

#[cfg(test)]
mod tests {
    use super::sample;

    #[test]
    fn samples_average_the_block_and_stay_in_bounds() {
        let px: Vec<[f32; 4]> = (0..9).map(|i| [i as f32 / 8.0, 0.5, 0.25, 1.0]).collect();
        let mid = sample(&px, 3, 3, [0.5, 0.5]).expect("centre");
        assert!((mid[0] - 0.5).abs() < 1e-6 && (mid[1] - 0.5).abs() < 1e-6);
        assert!(sample(&px, 3, 3, [1.0, 1.0]).is_some(), "the far corner clamps inside");
        assert!(sample(&px, 0, 0, [0.5, 0.5]).is_none(), "an empty proxy has nothing");
        assert!(sample(&px, 3, 3, [f32::NAN, 0.5]).is_none());
        assert!(sample(&px, 10, 10, [0.9, 0.9]).is_none(), "pixels missing from a short buffer are skipped");
        let bad = vec![[f32::NAN, 0.0, 0.0, 1.0]; 9];
        assert!(sample(&bad, 3, 3, [0.5, 0.5]).is_none());
    }
}
