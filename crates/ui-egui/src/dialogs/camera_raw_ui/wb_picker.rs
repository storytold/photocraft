//! Camera Raw's White Balance tool (I): while it is on, a click on the preview sets Temperature
//! and Tint so the clicked spot turns neutral. The colour is read from the image before Camera
//! Raw (the proxy), at the source point the shown pixel came from when Geometry or Optics ›
//! Distortion moves it, averaged
//! over a 3 × 3 block as Camera Raw's tool reads a small area.

use photocraft_algo::camera_raw::{CameraRaw, distortion_source, geometry, white_balance_from_sample};
use photocraft_algo::transform::Homography;

use super::{CameraRawDialog, ERect, Pos2};

/// Where the point `u` (0 … 1 across the shown image) comes from in the proxy (0 … 1): through
/// the Geometry and Distortion that are shown (none in the Before view or with their panels
/// hidden).
pub(super) fn source_point(d: &CameraRawDialog, u: [f32; 2]) -> Option<[f32; 2]> {
    let mut p = d.effective_params();
    if d.show_before || !p.moves_pixels() {
        return Some(u);
    }
    // The preview shows Upright's analysis of the full canvas (what OK applies), not the proxy's.
    if !p.geometry_neutral()
        && let Some(fix) = d.upright_source().and_then(|s| s.fix(&p))
    {
        p.upright_fix = Some(fix);
    }
    unwarp(&d.proxy, d.pw, d.ph, d.canvas_frame, &p, u)
}

/// `u` (0 … 1 across the `pw` × `ph` proxy) back through `p`'s Geometry and then its Distortion
/// (develop distorts first), as the preview applies them: when the layer reaches past the canvas,
/// only the canvas's part `frame` (proxy pixels `[x0, y0, x1, y1]`) moves, about its own centre,
/// and the rest stays where it is. `None` when the shown point has no source (uncovered by the
/// warp or the distortion, or the warp folds).
fn unwarp(px: &[[f32; 4]], pw: usize, ph: usize, frame: Option<[usize; 4]>, p: &CameraRaw, u: [f32; 2]) -> Option<[f32; 2]> {
    let (pw, ph) = (pw.max(1), ph.max(1));
    let [x0, y0, x1, y1] = frame.filter(|[x0, y0, x1, y1]| x0 < x1 && y0 < y1 && *x1 <= pw && *y1 <= ph).unwrap_or([0, 0, pw, ph]);
    let (fw, fh) = (x1 - x0, y1 - y0);
    // The shown point in proxy pixels; past the canvas the layer is not warped.
    let (px_x, px_y) = (f64::from(u[0]) * pw as f64, f64::from(u[1]) * ph as f64);
    if px_x < x0 as f64 || px_x > x1 as f64 || px_y < y0 as f64 || px_y > y1 as f64 {
        return Some(u);
    }
    let (w, h) = (fw as f64, fh as f64);
    let (mut sx, mut sy) = (px_x - x0 as f64, px_y - y0 as f64);
    if !p.geometry_neutral() {
        let sub: Vec<[f32; 4]> =
            if (fw, fh) == (pw, ph) { Vec::new() } else { (y0..y1).flat_map(|y| px.get(y * pw + x0..y * pw + x1).unwrap_or(&[]).iter().copied()).collect() };
        let m = geometry::homography(if sub.is_empty() { px } else { &sub }, fw, fh, p);
        let inv = Homography(m).inverse()?.0;
        let (x, y) = (sx - w / 2.0, sy - h / 2.0);
        let z = inv[6] * x + inv[7] * y + inv[8];
        if !z.is_finite() || z.abs() < 1e-12 {
            return None;
        }
        (sx, sy) = ((inv[0] * x + inv[1] * y + inv[2]) / z + w / 2.0, (inv[3] * x + inv[4] * y + inv[5]) / z + h / 2.0);
        if !(sx.is_finite() && sy.is_finite() && (0.0..=w).contains(&sx) && (0.0..=h).contains(&sy)) {
            return None;
        }
    }
    let (sx, sy) = distortion_source(p, fw, fh, sx, sy)?;
    let out = [((sx + x0 as f64) / pw as f64) as f32, ((sy + y0 as f64) / ph as f64) as f32];
    out.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)).then_some(out)
}

/// The mean colour of the proxy's 3 × 3 block around `u` (0 … 1), weighted by alpha and
/// ignoring non-finite and fully transparent pixels (their colour is hidden).
pub(super) fn sample(px: &[[f32; 4]], pw: usize, ph: usize, u: [f32; 2]) -> Option<[f32; 3]> {
    if pw == 0 || ph == 0 || !u.iter().all(|v| v.is_finite()) {
        return None;
    }
    let cx = ((u[0].clamp(0.0, 1.0) * pw as f32) as usize).min(pw - 1);
    let cy = ((u[1].clamp(0.0, 1.0) * ph as f32) as usize).min(ph - 1);
    let (mut sum, mut n) = ([0.0f32; 3], 0.0f32);
    for y in cy.saturating_sub(1)..=(cy + 1).min(ph - 1) {
        for x in cx.saturating_sub(1)..=(cx + 1).min(pw - 1) {
            let Some(p) = y.checked_mul(pw).and_then(|i| i.checked_add(x)).and_then(|i| px.get(i)) else { continue };
            let a = p[3].min(1.0);
            if p.iter().all(|v| v.is_finite()) && a > 0.0 {
                for c in 0..3 {
                    sum[c] += p[c] * a;
                }
                n += a;
            }
        }
    }
    (n > 0.0).then(|| sum.map(|s| s / n))
}

/// The Temperature and Tint that neutralise the shown point `u` (0 … 1), or `None` when that
/// spot has no colour to read (too dark, clipped white, off the image).
pub(super) fn balance_at(d: &CameraRawDialog, u: [f32; 2]) -> Option<(f32, f32)> {
    let src = source_point(d, u)?;
    let rgb = sample(&d.proxy, d.pw, d.ph, src)?;
    white_balance_from_sample(rgb, d.params.encoding_gamma)
}

/// The tool on the preview: a crosshair over `active`, the part of the image `r` that shows the
/// After image (all of it, except under Split's Before half); a click there sets the balance
/// (one undo step, like a slider drag).
pub(super) fn preview(ui: &egui::Ui, d: &mut CameraRawDialog, r: ERect, active: ERect, response: &egui::Response) {
    let over = response.hover_pos().filter(|p| active.contains(*p));
    if over.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    if response.clicked()
        && let Some(p) = response.interact_pointer_pos().filter(|p| active.contains(*p))
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
    use super::{CameraRaw, distortion_source, sample, unwarp};

    #[test]
    fn geometry_turns_the_canvas_about_its_own_centre() {
        // A 40 × 20 proxy whose canvas is its left half: Rotate turns that half about (10, 10).
        let px = vec![[0.5, 0.5, 0.5, 1.0]; 40 * 20];
        let p = CameraRaw { perspective_rotate: 8.0, ..CameraRaw::default() };
        let frame = Some([0, 0, 20, 20]);
        let centre = unwarp(&px, 40, 20, frame, &p, [0.25, 0.5]).expect("inside");
        assert!((centre[0] - 0.25).abs() < 1e-4 && (centre[1] - 0.5).abs() < 1e-4, "the canvas centre stays: {centre:?}");
        assert_eq!(unwarp(&px, 40, 20, frame, &p, [0.8, 0.3]), Some([0.8, 0.3]), "past the canvas nothing moves");
        // Without a frame the whole proxy turns about (20, 10), so its centre stays instead.
        let whole = unwarp(&px, 40, 20, None, &p, [0.5, 0.5]).expect("inside");
        assert!((whole[0] - 0.5).abs() < 1e-4 && (whole[1] - 0.5).abs() < 1e-4, "{whole:?}");
        let off = unwarp(&px, 40, 20, None, &p, [0.25, 0.5]).expect("inside");
        assert!((off[1] - 0.5).abs() > 1e-3, "off the turning centre a point moves: {off:?}");
    }

    #[test]
    fn a_click_reads_the_pixel_distortion_showed_there() {
        // A horizontal ramp through Distortion: the proxy at the point the picker maps a shown
        // pixel back to holds the colour shown there.
        let (w, h) = (60, 40);
        let ramp = |x: f32| [x / w as f32, 0.5, 0.25, 1.0];
        let px: Vec<[f32; 4]> = (0..w * h).map(|i| ramp((i % w) as f32 + 0.5)).collect();
        for amount in [-80.0, 60.0] {
            let p = CameraRaw { lens_distortion: amount, ..CameraRaw::default() };
            let mut shown = px.clone();
            photocraft_algo::camera_raw::develop(&mut shown, w, h, &p, true);
            for (x, y) in [(5, 6), (50, 30), (12, 33), (30, 20)] {
                let u = [(x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32];
                let Some(src) = unwarp(&px, w, h, None, &p, u) else {
                    assert!(shown[y * w + x][3] < 1e-6, "only an uncovered pixel has no source: {amount} {x} {y}");
                    continue;
                };
                let expect = shown[y * w + x][0];
                let read = ramp(src[0] * w as f32)[0];
                assert!((read - expect).abs() < 2e-3, "{amount} at ({x}, {y}): read {read}, shown {expect}");
            }
        }
        // Raising it uncovers the edges' middles: nothing to read there.
        let up = CameraRaw { lens_distortion: 100.0, ..CameraRaw::default() };
        assert_eq!(unwarp(&px, w, h, None, &up, [0.001, 0.5]), None);
        assert_eq!(distortion_source(&up, w, h, f64::NAN, 1.0), None);
        // Past the canvas nothing moves, with Distortion too.
        assert_eq!(unwarp(&px, w, h, Some([0, 0, 30, 40]), &up, [0.8, 0.3]), Some([0.8, 0.3]));
    }

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

    #[test]
    fn transparent_pixels_do_not_tint_the_sample() {
        let px: Vec<[f32; 4]> = (0..9).map(|i| if i % 2 == 0 { [0.5, 0.5, 0.5, 1.0] } else { [0.0, 0.0, 1.0, 0.0] }).collect();
        assert_eq!(sample(&px, 3, 3, [0.5, 0.5]), Some([0.5; 3]));
        let clear = vec![[0.3, 0.3, 0.3, 0.0]; 9];
        assert!(sample(&clear, 3, 3, [0.5, 0.5]).is_none(), "nothing visible to read");
    }
}
