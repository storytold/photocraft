//! Straight line segments for Upright: gradient pixels grown into regions of one level-line
//! direction, each region fitted with a segment. A simplified form of R. Grompone von Gioi,
//! J. Jakubowicz, J.-M. Morel, G. Randall, *LSD: a Line Segment Detector*, IPOL 2012 (region
//! growing at a 22.5° tolerance, a rectangle fitted by the region's moments), with plain
//! length and density thresholds in place of LSD's a-contrario test.

/// A detected segment in pixel coordinates of the analysed image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub a: [f64; 2],
    pub b: [f64; 2],
}

impl Segment {
    pub fn length(&self) -> f64 {
        (self.b[0] - self.a[0]).hypot(self.b[1] - self.a[1])
    }
    /// The segment's line `[a, b, c]` with `a·x + b·y + c = 0` and `a² + b² = 1`, or `None` for a
    /// degenerate segment.
    pub fn line(&self) -> Option<[f64; 3]> {
        let (dx, dy) = (self.b[0] - self.a[0], self.b[1] - self.a[1]);
        let n = dx.hypot(dy);
        if !(n > 1e-9) {
            return None;
        }
        let (a, b) = (dy / n, -dx / n);
        Some([a, b, -(a * self.a[0] + b * self.a[1])])
    }
    pub fn mid(&self) -> [f64; 2] {
        [(self.a[0] + self.b[0]) / 2.0, (self.a[1] + self.b[1]) / 2.0]
    }
}

/// Largest image the detector reads; callers downsample to this first.
pub const MAX_SIDE: usize = 1024;

/// Segments in a `w × h` luminance image (values 0 … 1), longest first. `min_len` is in pixels.
pub fn detect(lum: &[f32], w: usize, h: usize, min_len: f64) -> Vec<Segment> {
    if w < 4 || h < 4 || lum.len() < w.saturating_mul(h) || w > 8 * MAX_SIDE || h > 8 * MAX_SIDE {
        return Vec::new();
    }
    let n = w * h;
    let mut mag = vec![0.0f32; n];
    let mut ang = vec![0.0f32; n];
    // 2 × 2 gradient (LSD's), at the corner between four pixels.
    for y in 0..h - 1 {
        for x in 0..w - 1 {
            let i = y * w + x;
            let (p, q, r, s) = (lum[i], lum[i + 1], lum[i + w], lum[i + w + 1]);
            let gx = (q + s - p - r) * 0.5;
            let gy = (r + s - p - q) * 0.5;
            let m = gx.hypot(gy);
            mag[i] = m;
            // Level-line angle: along the edge.
            ang[i] = gx.atan2(-gy);
        }
    }
    // LSD's gradient threshold for a quantization error of 2/255 at a 22.5° tolerance.
    let tau = 22.5f32.to_radians();
    let rho = (2.0 / 255.0) / tau.sin();
    let mut order: Vec<u32> = (0..n as u32).filter(|&i| mag.get(i as usize).copied().unwrap_or(0.0) > rho).collect();
    order.sort_unstable_by(|a, b| mag[*b as usize].total_cmp(&mag[*a as usize]));
    let mut used = vec![false; n];
    let mut out = Vec::new();
    let mut region: Vec<u32> = Vec::new();
    let angle_close = |a: f32, b: f32| {
        let mut d = (a - b).abs();
        if d > std::f32::consts::PI {
            d = 2.0 * std::f32::consts::PI - d;
        }
        d <= tau
    };
    for &seed in &order {
        let s = seed as usize;
        if used[s] {
            continue;
        }
        region.clear();
        region.push(seed);
        used[s] = true;
        let mut theta = ang[s];
        let (mut sx, mut sy) = (theta.cos(), theta.sin());
        let mut k = 0;
        while k < region.len() {
            let p = region[k] as usize;
            k += 1;
            let (px, py) = ((p % w) as i64, (p / w) as i64);
            for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    let (xx, yy) = (px + dx, py + dy);
                    if xx < 0 || yy < 0 || xx >= w as i64 - 1 || yy >= h as i64 - 1 {
                        continue;
                    }
                    let j = yy as usize * w + xx as usize;
                    if used[j] || mag[j] <= rho || !angle_close(ang[j], theta) {
                        continue;
                    }
                    used[j] = true;
                    region.push(j as u32);
                    sx += ang[j].cos();
                    sy += ang[j].sin();
                    theta = sy.atan2(sx);
                }
            }
            if region.len() > 200_000 {
                break;
            }
        }
        if region.len() < 12 {
            continue;
        }
        if let Some(seg) = fit(&region, &mag, w, min_len) {
            out.push(seg);
        }
    }
    out.sort_by(|a, b| b.length().total_cmp(&a.length()));
    out
}

/// A segment through a region's pixels (weighted by gradient magnitude), if it is long and
/// thin enough.
fn fit(region: &[u32], mag: &[f32], w: usize, min_len: f64) -> Option<Segment> {
    let (mut sw, mut mx, mut my) = (0.0f64, 0.0f64, 0.0f64);
    for &p in region {
        let p = p as usize;
        let m = f64::from(*mag.get(p)?);
        sw += m;
        mx += m * ((p % w) as f64 + 1.0);
        my += m * ((p / w) as f64 + 1.0);
    }
    if !(sw > 0.0) {
        return None;
    }
    let (cx, cy) = (mx / sw, my / sw);
    let (mut ixx, mut iyy, mut ixy) = (0.0, 0.0, 0.0);
    for &p in region {
        let p = p as usize;
        let m = f64::from(mag[p]);
        let (dx, dy) = ((p % w) as f64 + 1.0 - cx, (p / w) as f64 + 1.0 - cy);
        ixx += m * dx * dx;
        iyy += m * dy * dy;
        ixy += m * dx * dy;
    }
    // Principal axis of the region's inertia.
    let t = 0.5 * (2.0 * ixy).atan2(ixx - iyy);
    let (ux, uy) = (t.cos(), t.sin());
    let (mut lo, mut hi, mut wlo, mut whi) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for &p in region {
        let p = p as usize;
        let (dx, dy) = ((p % w) as f64 + 1.0 - cx, (p / w) as f64 + 1.0 - cy);
        let l = dx * ux + dy * uy;
        let n = -dx * uy + dy * ux;
        lo = lo.min(l);
        hi = hi.max(l);
        wlo = wlo.min(n);
        whi = whi.max(n);
    }
    let len = hi - lo + 1.0;
    let width = (whi - wlo + 1.0).max(1.0);
    if len < min_len || width > 0.25 * len || (region.len() as f64) < 0.6 * len {
        return None;
    }
    Some(Segment { a: [cx + lo * ux, cy + lo * uy], b: [cx + hi * ux, cy + hi * uy] })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas(w: usize, h: usize, f: impl Fn(usize, usize) -> f32) -> Vec<f32> {
        (0..w * h).map(|i| f(i % w, i / w)).collect()
    }

    #[test]
    fn finds_a_vertical_and_a_horizontal_edge() {
        let (w, h) = (200, 160);
        let img = canvas(w, h, |x, y| if x > 100 || y > 120 { 0.9 } else { 0.1 });
        let segs = detect(&img, w, h, 30.0);
        assert!(segs.iter().any(|s| (s.a[0] - s.b[0]).abs() < 2.0 && s.length() > 80.0), "{segs:?}");
        assert!(segs.iter().any(|s| (s.a[1] - s.b[1]).abs() < 2.0 && s.length() > 60.0), "{segs:?}");
    }

    #[test]
    fn a_slanted_edge_keeps_its_angle() {
        let (w, h) = (300, 300);
        // Anti-aliased, as photographs are: a hard staircase has no single direction.
        let img = canvas(w, h, |x, y| (0.5 + (x as f32 + 0.5 - 100.0 - 0.2 * (y as f32 + 0.5))).clamp(0.0, 1.0) * 0.6 + 0.2);
        let segs = detect(&img, w, h, 50.0);
        let s = segs.first().expect("a segment");
        let slope = (s.b[0] - s.a[0]) / (s.b[1] - s.a[1]);
        assert!((slope - 0.2).abs() < 0.02, "{slope}");
    }

    #[test]
    fn hostile_inputs_give_nothing() {
        assert!(detect(&[], 0, 0, 10.0).is_empty());
        assert!(detect(&[0.5; 10], 100, 100, 10.0).is_empty());
        let nan = vec![f32::NAN; 64 * 64];
        assert!(detect(&nan, 64, 64, 5.0).is_empty());
        let flat = vec![0.5; 64 * 64];
        assert!(detect(&flat, 64, 64, 5.0).is_empty());
    }
}
