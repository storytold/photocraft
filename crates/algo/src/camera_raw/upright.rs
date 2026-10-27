//! Geometry › Upright: a camera rotation that makes the picture's lines plumb and level.
//!
//! The model is the one of S. Chaudhury, S. DiVerdi, S. Ioffe, *Auto-rectification of user
//! photos*, ICIP 2014: a photo of a scene taken with a tilted camera is corrected by the
//! homography `K·R·K⁻¹` of a rotation `R` (pitch about the horizontal axis, yaw about the vertical
//! axis, roll about the optical axis) for a pinhole camera `K` of focal length `f`. Upright's modes
//! choose which of the three angles are free:
//!
//! * **Level**: roll only;
//! * **Vertical**: roll and pitch (converging verticals made parallel);
//! * **Full**: roll, pitch and yaw (receding horizontals made level too);
//! * **Auto**: Full, without a turn of more than 20°;
//! * **Guided**: all three, fitted to the guides the user drew (two or more, at most four).
//!
//! The angles are the least-squares fit (Levenberg–Marquardt on three parameters) of each line's
//! deviation from vertical or horizontal after the warp, with a small penalty on each angle so an
//! under-determined fit stays near no rotation. The automatic modes fit in two stages, as the
//! paper does: roll and pitch level the camera from the verticals (and lines already within 8° of
//! level), then yaw, a turn about the vertical that keeps verticals plumb and the horizon level,
//! levels the receding horizontals. The automatic modes find their lines with a
//! compact region-growing segment detector in the spirit of LSD (R. Grompone von Gioi,
//! J. Jakubowicz, J.-M. Morel, G. Randall, *LSD: a Fast Line Segment Detector with a False
//! Detection Control*, PAMI 2010: pixels whose level-line angles agree within 22.5° grow into a
//! region, which is kept when it is long and thin), on a copy reduced to 800 pixels on its long
//! side so a preview proxy and the full image find the same lines, then reweight the fit to
//! ignore lines that disagree (Cauchy weights), as diagonal edges in a scene do.
//!
//! Coordinates are [`crate::lens::LensMap`]'s normalised frame: the frame's centre is the origin
//! and its half diagonal is 1. The result keeps the centre where it is and its local area, so the
//! Manual Transformations (Scale, Offset, …) still act from the same place.

use crate::transform::Homography;

/// Focal length of the assumed camera in half diagonals (about 32 mm on full frame).
const FOCAL: f64 = 1.5;
/// Long side the line detector works at.
const DETECT_SIDE: usize = 800;
/// Angles beyond this many degrees are not a plausible tilt to correct.
const MAX_ANGLE: f64 = 40.0;

/// A line segment, endpoints in normalised frame coordinates.
pub type Segment = [[f64; 2]; 2];

/// Which way a segment should end up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Vertical,
    Horizontal,
}

/// Pixel → normalised frame coordinates for a `w × h` frame.
pub fn to_normalised(p: [f64; 2], w: usize, h: usize) -> [f64; 2] {
    let r = (w as f64).hypot(h as f64).max(2.0) / 2.0;
    [(p[0] - w as f64 / 2.0) / r, (p[1] - h as f64 / 2.0) / r]
}

/// The axis a guide or detected segment is closer to.
pub fn axis_of(s: &Segment) -> Axis {
    let (dx, dy) = (s[1][0] - s[0][0], s[1][1] - s[0][1]);
    if dy.abs() >= dx.abs() { Axis::Vertical } else { Axis::Horizontal }
}

/// The forward Upright homography (source → corrected, normalised coordinates) for `mode`, or
/// `None` when it changes nothing: `off`, too few lines or guides, or no tilt found. `guides` are
/// fractions of the frame (`0..=1`); `px` is only read by the automatic modes.
pub fn homography(mode: &str, guides: &[[[f32; 2]; 2]], px: &[[f32; 4]], w: usize, h: usize) -> Option<Homography> {
    if mode == "guided" {
        let lines: Vec<Line> = guides
            .iter()
            .take(4)
            .map(|g| {
                let s = [0, 1].map(|i| to_normalised([f64::from(g[i][0]) * w as f64, f64::from(g[i][1]) * h as f64], w, h));
                (s, axis_of(&s), 1.0)
            })
            .filter(|(s, _, _)| seg_len(s) > 1e-3)
            .collect();
        if lines.len() < 2 {
            return None;
        }
        // Three or more guides in both directions also tell the focal length.
        let both = lines.iter().any(|l| l.1 == Axis::Vertical) && lines.iter().any(|l| l.1 == Axis::Horizontal);
        let free = [true, true, true, both && lines.len() >= 3];
        return rotation_homography(fit(&lines, free, [0.0; 4], false)?);
    }
    if !["level", "vertical", "full", "auto"].contains(&mode) {
        return None;
    }
    let found = detected(px, w, h);
    // Stage 1, the camera levelled: verticals plumb and lines already nearly level (within 8°)
    // level. Receding horizontals stay out: they would turn the horizon.
    let levelling: Vec<Line> = found.iter().filter(|l| l.1 == Axis::Vertical || l.3 <= 8.0).map(|l| (l.0, l.1, l.2)).collect();
    let free = if mode == "level" { [false, false, true, false] } else { [true, false, true, false] };
    let level = fit(&levelling, free, [0.0; 4], true).unwrap_or([0.0; 4]);
    let mut pose = level;
    if mode == "full" || mode == "auto" {
        // Stage 2, a turn about the vertical (yaw) that levels the dominant receding horizontals,
        // fitted with everything else so verticals stay plumb, and with the focal length, which
        // the perpendicular vertical and receding directions pin down.
        let all: Vec<Line> = found.iter().map(|l| (l.0, l.1, l.2)).collect();
        let receding = found.iter().any(|l| l.1 == Axis::Horizontal && l.3 > 8.0);
        if receding && let Some(turned) = fit(&all, [true, true, true, true], level, true) {
            // Auto keeps a natural look: no turn beyond 20°.
            if mode == "full" || turned[1].abs() <= 20f64.to_radians() {
                pose = turned;
            }
        }
    }
    rotation_homography(pose)
}

/// The detected lines within 25° of vertical or 35° of horizontal: segment, axis, length (the
/// weight) and degrees off the axis.
fn detected(px: &[[f32; 4]], w: usize, h: usize) -> Vec<(Segment, Axis, f64, f64)> {
    detect_segments(px, w, h)
        .into_iter()
        .filter_map(|s| {
            let (dx, dy) = (s[1][0] - s[0][0], s[1][1] - s[0][1]);
            let off_vertical = dx.abs().atan2(dy.abs()).to_degrees();
            let len = seg_len(&s);
            if off_vertical <= 25.0 {
                Some((s, Axis::Vertical, len, off_vertical))
            } else if off_vertical >= 55.0 {
                Some((s, Axis::Horizontal, len, 90.0 - off_vertical))
            } else {
                None
            }
        })
        .collect()
}

fn seg_len(s: &Segment) -> f64 {
    (s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1])
}

/// A camera pose: pitch, yaw and roll in radians, and the focal length as `ln(f / FOCAL)`.
pub type Pose = [f64; 4];

/// `K·R·K⁻¹` for `pose`, then shifted and scaled so the frame's centre stays put with its local
/// area.
pub fn rotation_homography(pose: Pose) -> Option<Homography> {
    if pose[..3].iter().all(|a| a.abs() < 1e-6) {
        return None;
    }
    let m = raw_rotation(pose);
    // Where the centre goes, and the local Jacobian there.
    let (cx, cy) = (m[2] / m[8], m[5] / m[8]);
    let e = 1e-4;
    let at = |x: f64, y: f64| {
        let w = m[6] * x + m[7] * y + m[8];
        [(m[0] * x + m[1] * y + m[2]) / w, (m[3] * x + m[4] * y + m[5]) / w]
    };
    let (px, py) = (at(e, 0.0), at(0.0, e));
    let det = ((px[0] - cx) * (py[1] - cy) - (px[1] - cy) * (py[0] - cx)) / (e * e);
    if !det.is_finite() || det <= 1e-6 {
        return None;
    }
    let s = 1.0 / det.sqrt();
    let fix = Homography([s, 0.0, -s * cx, 0.0, s, -s * cy, 0.0, 0.0, 1.0]);
    let out = fix.mul(&Homography(m));
    out.0.iter().all(|v| v.is_finite()).then_some(out)
}

fn raw_rotation([pitch, yaw, roll, log_f]: Pose) -> [f64; 9] {
    let (sp, cp) = pitch.sin_cos();
    let (sy, cy) = yaw.sin_cos();
    let (sr, cr) = roll.sin_cos();
    let rx = [1.0, 0.0, 0.0, 0.0, cp, -sp, 0.0, sp, cp];
    let ry = [cy, 0.0, sy, 0.0, 1.0, 0.0, -sy, 0.0, cy];
    let rz = [cr, -sr, 0.0, sr, cr, 0.0, 0.0, 0.0, 1.0];
    // Roll undone first, then pitch levels the camera, then yaw turns it about the vertical.
    let r = mul3(&ry, &mul3(&rx, &rz));
    let f = FOCAL * log_f.clamp(-1.5, 1.5).exp();
    let k = [f, 0.0, 0.0, 0.0, f, 0.0, 0.0, 0.0, 1.0];
    let kinv = [1.0 / f, 0.0, 0.0, 0.0, 1.0 / f, 0.0, 0.0, 0.0, 1.0];
    mul3(&k, &mul3(&r, &kinv))
}

fn mul3(a: &[f64; 9], b: &[f64; 9]) -> [f64; 9] {
    let mut r = [0.0; 9];
    for i in 0..3 {
        for j in 0..3 {
            r[i * 3 + j] = (0..3).map(|k| a[i * 3 + k] * b[k * 3 + j]).sum();
        }
    }
    r
}

/// Each line's deviation after the rotation: the sine of its angle from its axis (large when an
/// endpoint passes the horizon).
fn residual(m: &[f64; 9], s: &Segment, axis: Axis) -> f64 {
    let map = |p: [f64; 2]| {
        let w = m[6] * p[0] + m[7] * p[1] + m[8];
        (w > 1e-3).then(|| [(m[0] * p[0] + m[1] * p[1] + m[2]) / w, (m[3] * p[0] + m[4] * p[1] + m[5]) / w])
    };
    let (Some(a), Some(b)) = (map(s[0]), map(s[1])) else { return 1.0 };
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len = dx.hypot(dy);
    if len < 1e-9 {
        return 1.0;
    }
    match axis {
        Axis::Vertical => dx / len,
        Axis::Horizontal => dy / len,
    }
}

type Line = (Segment, Axis, f64);

/// Fits the `free` parts of the pose from `start` (the others keep their start value). `None`
/// when there are no lines to fit.
fn fit(lines: &[Line], free: [bool; 4], start: Pose, robust: bool) -> Option<Pose> {
    if lines.is_empty() {
        return None;
    }
    // Under-determined angles stay small and the focal length near its default.
    let total: f64 = lines.iter().map(|l| l.2).sum::<f64>().max(1e-9);
    let penalty = [1e-4 * total, 1e-4 * total, 1e-5 * total, 1e-3 * total];
    let mut weights: Vec<f64> = lines.iter().map(|l| l.2).collect();
    let mut theta = start;
    for round in 0..if robust { 4 } else { 1 } {
        theta = solve(lines, &weights, free, penalty, theta);
        if robust && round < 3 {
            // Cauchy weights with a 3° scale.
            let m = raw_rotation(theta);
            for (wt, l) in weights.iter_mut().zip(lines) {
                let r = residual(&m, &l.0, l.1) / 0.05;
                *wt = l.2 / (1.0 + r * r);
            }
        }
    }
    let limit = MAX_ANGLE.to_radians();
    for a in &mut theta[..3] {
        *a = if a.is_finite() { a.clamp(-limit, limit) } else { 0.0 };
    }
    theta[3] = if theta[3].is_finite() { theta[3].clamp(-1.5, 1.5) } else { 0.0 };
    Some(theta)
}

/// Levenberg–Marquardt with a numerical Jacobian.
fn solve(lines: &[Line], weights: &[f64], free: [bool; 4], penalty: [f64; 4], start: Pose) -> Pose {
    let cost = |t: &Pose| {
        let m = raw_rotation(*t);
        let lines_cost: f64 = lines.iter().zip(weights).map(|(l, w)| w * residual(&m, &l.0, l.1).powi(2)).sum();
        lines_cost + (0..4).map(|i| penalty[i] * t[i] * t[i]).sum::<f64>()
    };
    let mut t = start;
    let mut c = cost(&t);
    let mut lambda = 1e-3;
    for _ in 0..80 {
        let m = raw_rotation(t);
        let base: Vec<f64> = lines.iter().map(|l| residual(&m, &l.0, l.1)).collect();
        let mut jac = vec![[0.0f64; 4]; lines.len()];
        for k in (0..4).filter(|&k| free[k]) {
            let mut tk = t;
            tk[k] += 1e-6;
            let mk = raw_rotation(tk);
            for (i, l) in lines.iter().enumerate() {
                jac[i][k] = (residual(&mk, &l.0, l.1) - base[i]) / 1e-6;
            }
        }
        // Normal equations (Jᵀ W J + P + λ·diag) δ = −(Jᵀ W r + P t).
        let mut a = [[0.0f64; 4]; 4];
        let mut g = [0.0f64; 4];
        for ((row, r), w) in jac.iter().zip(&base).zip(weights) {
            for i in 0..4 {
                g[i] += w * row[i] * r;
                for j in 0..4 {
                    a[i][j] += w * row[i] * row[j];
                }
            }
        }
        for i in 0..4 {
            if free[i] {
                a[i][i] += penalty[i];
                g[i] += penalty[i] * t[i];
                a[i][i] = a[i][i] * (1.0 + lambda) + 1e-12;
            } else {
                // A fixed part: its row and column drop out, δᵢ = 0.
                for j in 0..4 {
                    a[i][j] = 0.0;
                    a[j][i] = 0.0;
                }
                a[i][i] = 1.0;
                g[i] = 0.0;
            }
        }
        let Some(delta) = solve4(a, g.map(|v| -v)) else { break };
        let next = [t[0] + delta[0], t[1] + delta[1], t[2] + delta[2], t[3] + delta[3]];
        let nc = cost(&next);
        if nc.is_finite() && nc < c {
            let step = delta.iter().map(|d| d.abs()).fold(0.0, f64::max);
            t = next;
            c = nc;
            lambda = (lambda * 0.3).max(1e-9);
            if step < 1e-9 {
                break;
            }
        } else {
            lambda *= 10.0;
            if lambda > 1e8 {
                break;
            }
        }
    }
    t
}

/// Solves a 4×4 system by Gaussian elimination with partial pivoting.
fn solve4(mut a: [[f64; 4]; 4], mut b: [f64; 4]) -> Option<[f64; 4]> {
    for col in 0..4 {
        let piv = (col..4).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[piv][col].abs() < 1e-15 {
            return None;
        }
        a.swap(col, piv);
        b.swap(col, piv);
        for row in col + 1..4 {
            let f = a[row][col] / a[col][col];
            for k in col..4 {
                a[row][k] -= f * a[col][k];
            }
            b[row] -= f * b[col];
        }
    }
    let mut x = [0.0; 4];
    for row in (0..4).rev() {
        let s: f64 = (row + 1..4).map(|k| a[row][k] * x[k]).sum();
        x[row] = (b[row] - s) / a[row][row];
    }
    x.iter().all(|v| v.is_finite()).then_some(x)
}

/// Straight segments in a `w × h` RGBA image, in normalised frame coordinates.
pub fn detect_segments(px: &[[f32; 4]], w: usize, h: usize) -> Vec<Segment> {
    if w < 8 || h < 8 || px.len() != w.saturating_mul(h) {
        return Vec::new();
    }
    // Luminance, box-reduced so the long side is at most DETECT_SIDE.
    let f = w.max(h).div_ceil(DETECT_SIDE).max(1);
    let (sw, sh) = (w / f, h / f);
    if sw < 8 || sh < 8 {
        return Vec::new();
    }
    let mut lum = vec![0.0f32; sw * sh];
    for (y, row) in lum.chunks_exact_mut(sw).enumerate() {
        for (x, v) in row.iter_mut().enumerate() {
            let mut acc = 0.0;
            for yy in y * f..(y + 1) * f {
                for xx in x * f..(x + 1) * f {
                    let q = px.get(yy * w + xx).copied().unwrap_or_default();
                    acc += 0.2126 * q[0] + 0.7152 * q[1] + 0.0722 * q[2];
                }
            }
            *v = acc / (f * f) as f32;
        }
    }
    // A [1 4 6 4 1] / 16 Gaussian each way, so stair-stepped and noisy edges keep one direction.
    const K: [f32; 5] = [1.0 / 16.0, 4.0 / 16.0, 6.0 / 16.0, 4.0 / 16.0, 1.0 / 16.0];
    let mut tmp = vec![0.0f32; sw * sh];
    for y in 0..sh {
        for x in 0..sw {
            tmp[y * sw + x] = (0..5).map(|k| K[k] * lum[y * sw + (x + k).saturating_sub(2).min(sw - 1)]).sum();
        }
    }
    for y in 0..sh {
        for x in 0..sw {
            lum[y * sw + x] = (0..5).map(|k| K[k] * tmp[(y + k).saturating_sub(2).min(sh - 1) * sw + x]).sum();
        }
    }
    let at = |x: usize, y: usize| lum.get(y * sw + x).copied().unwrap_or(0.0);
    // Gradient (Sobel, per pixel) and the level-line angle.
    let mut mag = vec![0.0f32; sw * sh];
    let mut ang = vec![0.0f32; sw * sh];
    for y in 1..sh - 1 {
        for x in 1..sw - 1 {
            let gx = (at(x + 1, y - 1) + 2.0 * at(x + 1, y) + at(x + 1, y + 1) - at(x - 1, y - 1) - 2.0 * at(x - 1, y) - at(x - 1, y + 1)) / 8.0;
            let gy = (at(x - 1, y + 1) + 2.0 * at(x, y + 1) + at(x + 1, y + 1) - at(x - 1, y - 1) - 2.0 * at(x, y - 1) - at(x + 1, y - 1)) / 8.0;
            let i = y * sw + x;
            mag[i] = gx.hypot(gy);
            ang[i] = gx.atan2(-gy);
        }
    }
    const THRESHOLD: f32 = 0.02;
    let tol = 22.5f32.to_radians();
    let mut seeds: Vec<usize> = (0..mag.len()).filter(|&i| mag[i] > THRESHOLD).collect();
    seeds.sort_by(|&a, &b| mag[b].total_cmp(&mag[a]));
    let mut used = vec![false; mag.len()];
    let min_len = (sw.max(sh) as f64 * 0.04).max(16.0);
    let mut out = Vec::new();
    let mut region = Vec::new();
    let mut stack = Vec::new();
    for seed in seeds {
        if used[seed] {
            continue;
        }
        region.clear();
        stack.clear();
        used[seed] = true;
        stack.push(seed);
        // Region angle from doubled-angle sums (orientation, not direction).
        let (mut sc, mut ss) = ((2.0 * ang[seed]).cos(), (2.0 * ang[seed]).sin());
        while let Some(i) = stack.pop() {
            region.push(i);
            if region.len() > 40_000 {
                break;
            }
            let mean = ss.atan2(sc) / 2.0;
            let (x, y) = (i % sw, i / sw);
            for (dx, dy) in [(-1i32, -1i32), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                if nx < 1 || ny < 1 || nx >= sw as i32 - 1 || ny >= sh as i32 - 1 {
                    continue;
                }
                let j = ny as usize * sw + nx as usize;
                if used[j] || mag[j] <= THRESHOLD {
                    continue;
                }
                let mut d = (ang[j] - mean).rem_euclid(std::f32::consts::PI);
                if d > std::f32::consts::FRAC_PI_2 {
                    d = std::f32::consts::PI - d;
                }
                if d < tol {
                    used[j] = true;
                    sc += (2.0 * ang[j]).cos();
                    ss += (2.0 * ang[j]).sin();
                    stack.push(j);
                }
            }
        }
        if region.len() < min_len as usize {
            continue;
        }
        // Weighted principal axis of the region.
        let (mut sw_, mut mx, mut my) = (0.0f64, 0.0f64, 0.0f64);
        for &i in &region {
            let wt = f64::from(mag[i]);
            sw_ += wt;
            mx += wt * (i % sw) as f64;
            my += wt * (i / sw) as f64;
        }
        if sw_ <= 0.0 {
            continue;
        }
        let (mx, my) = (mx / sw_, my / sw_);
        let (mut cxx, mut cxy, mut cyy) = (0.0f64, 0.0f64, 0.0f64);
        for &i in &region {
            let wt = f64::from(mag[i]);
            let (dx, dy) = ((i % sw) as f64 - mx, (i / sw) as f64 - my);
            cxx += wt * dx * dx;
            cxy += wt * dx * dy;
            cyy += wt * dy * dy;
        }
        let theta = 0.5 * (2.0 * cxy).atan2(cxx - cyy);
        let (dx, dy) = (theta.cos(), theta.sin());
        let (mut lo, mut hi, mut nlo, mut nhi) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for &i in &region {
            let (px_, py_) = ((i % sw) as f64 - mx, (i / sw) as f64 - my);
            let along = px_ * dx + py_ * dy;
            let across = -px_ * dy + py_ * dx;
            lo = lo.min(along);
            hi = hi.max(along);
            nlo = nlo.min(across);
            nhi = nhi.max(across);
        }
        let (len, width) = (hi - lo, (nhi - nlo).max(1.0));
        if len < min_len || len / width < 5.0 {
            continue;
        }
        let scale = f as f64;
        let end = |t: f64| [(mx + t * dx + 0.5) * scale, (my + t * dy + 0.5) * scale];
        out.push([to_normalised(end(lo), w, h), to_normalised(end(hi), w, h)]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(m: &Homography, p: [f64; 2]) -> [f64; 2] {
        let (x, y) = m.apply(p[0], p[1]);
        [x, y]
    }

    fn angle_from(axis: Axis, m: &Homography, s: &Segment) -> f64 {
        let (a, b) = (apply(m, s[0]), apply(m, s[1]));
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        match axis {
            Axis::Vertical => dx.atan2(dy.abs()).to_degrees().abs(),
            Axis::Horizontal => dy.atan2(dx.abs()).to_degrees().abs(),
        }
    }

    /// Lines of a box photographed with the camera tilted by `angles`.
    fn tilted(angles: [f64; 3]) -> (Homography, Vec<Segment>, Vec<Segment>) {
        let tilt = Homography(raw_rotation([angles[0], angles[1], angles[2], 0.0])).inverse().unwrap();
        let v: Vec<Segment> = [-0.4, -0.1, 0.25, 0.45].iter().map(|&x| [apply(&tilt, [x, -0.4]), apply(&tilt, [x, 0.4])]).collect();
        let hz: Vec<Segment> = [-0.3, 0.05, 0.35].iter().map(|&y| [apply(&tilt, [-0.5, y]), apply(&tilt, [0.5, y])]).collect();
        (tilt, v, hz)
    }

    fn guides(segs: &[Segment], w: usize, h: usize) -> Vec<[[f32; 2]; 2]> {
        let r = (w as f64).hypot(h as f64) / 2.0;
        segs.iter().map(|s| s.map(|p| [((p[0] * r + w as f64 / 2.0) / w as f64) as f32, ((p[1] * r + h as f64 / 2.0) / h as f64) as f32])).collect()
    }

    #[test]
    fn guided_verticals_become_plumb() {
        let (_, v, _) = tilted([0.25, 0.0, 0.03]);
        let g = guides(&v[..2], 600, 400);
        let m = homography("guided", &g, &[], 600, 400).expect("a correction");
        for s in &v {
            assert!(angle_from(Axis::Vertical, &m, s) < 0.3, "{}", angle_from(Axis::Vertical, &m, s));
        }
    }

    #[test]
    fn guided_full_fixes_both_directions() {
        let (_, v, hz) = tilted([0.2, -0.25, 0.04]);
        let g = guides(&[v[0], v[3], hz[0], hz[2]], 600, 400);
        let m = homography("guided", &g, &[], 600, 400).expect("a correction");
        for s in &v {
            assert!(angle_from(Axis::Vertical, &m, s) < 0.5);
        }
        for s in &hz {
            assert!(angle_from(Axis::Horizontal, &m, s) < 0.5);
        }
    }

    #[test]
    fn guided_full_finds_a_wide_lens_focal_length() {
        // Shot at a third less than the assumed focal length, turned 25° and tilted up.
        let tilt = Homography(raw_rotation([0.2, -0.45, 0.03, (1.0f64 / 1.5).ln()])).inverse().unwrap();
        let v: Vec<Segment> = [-0.4, 0.4].iter().map(|&x| [apply(&tilt, [x, -0.4]), apply(&tilt, [x, 0.4])]).collect();
        let hz: Vec<Segment> = [-0.3, 0.35].iter().map(|&y| [apply(&tilt, [-0.5, y]), apply(&tilt, [0.5, y])]).collect();
        let m = homography("guided", &guides(&[v[0], v[1], hz[0], hz[1]], 600, 400), &[], 600, 400).expect("a correction");
        for s in &v {
            assert!(angle_from(Axis::Vertical, &m, s) < 0.5, "{}", angle_from(Axis::Vertical, &m, s));
        }
        for s in &hz {
            assert!(angle_from(Axis::Horizontal, &m, s) < 0.5, "{}", angle_from(Axis::Horizontal, &m, s));
        }
    }

    #[test]
    fn one_guide_or_off_does_nothing() {
        let (_, v, _) = tilted([0.2, 0.0, 0.0]);
        assert!(homography("guided", &guides(&v[..1], 600, 400), &[], 600, 400).is_none());
        assert!(homography("off", &guides(&v, 600, 400), &[], 600, 400).is_none());
        assert!(homography("guided", &[], &[], 600, 400).is_none());
    }

    #[test]
    fn the_centre_and_its_scale_stay() {
        let m = rotation_homography([0.3, -0.2, 0.05, 0.0]).unwrap();
        let c = apply(&m, [0.0, 0.0]);
        assert!(c[0].abs() < 1e-9 && c[1].abs() < 1e-9);
        let (a, b) = (apply(&m, [1e-3, 0.0]), apply(&m, [0.0, 1e-3]));
        let det = (a[0] * b[1] - a[1] * b[0]) / 1e-6;
        assert!((det - 1.0).abs() < 1e-3, "{det}");
    }

    /// A synthetic photo: dark vertical posts leaning in (a pitched camera) and turned by `roll`
    /// (x shift per row), drawn with antialiased edges.
    fn picture(w: usize, h: usize, lean: f64, roll: f64) -> Vec<[f32; 4]> {
        let mut px = vec![[0.8f32, 0.8, 0.8, 1.0]; w * h];
        for y in 0..h {
            let t = y as f64 / h as f64 - 0.5;
            for k in 0..6 {
                let base = (k as f64 + 0.5) / 6.0 * w as f64;
                let x0 = base + (base - w as f64 / 2.0) * lean * t + roll * (y as f64 - h as f64 / 2.0);
                for x in (x0 as i64 - 5).max(0)..(x0 as i64 + 6).min(w as i64) {
                    // Coverage of pixel [x, x+1) by the post [x0-3, x0+3].
                    let cover = ((x as f64 + 1.0).min(x0 + 3.0) - (x as f64).max(x0 - 3.0)).clamp(0.0, 1.0) as f32;
                    let q = &mut px[y * w + x as usize];
                    for c in 0..3 {
                        q[c] -= 0.7 * cover;
                    }
                }
            }
        }
        px
    }

    #[test]
    fn detector_finds_the_posts() {
        let (w, h) = (640, 480);
        let segs = detect_segments(&picture(w, h, 0.0, 0.0), w, h);
        let vertical = segs.iter().filter(|s| axis_of(s) == Axis::Vertical && seg_len(s) > 0.5).count();
        assert!(vertical >= 6, "{vertical} of {} segments", segs.len());
    }

    #[test]
    fn auto_modes_straighten_converging_posts() {
        let (w, h) = (640, 480);
        let px = picture(w, h, 0.35, 0.0);
        for mode in ["vertical", "auto", "full"] {
            let m = homography(mode, &[], &px, w, h).expect(mode);
            for s in detect_segments(&px, w, h).iter().filter(|s| axis_of(s) == Axis::Vertical && seg_len(s) > 0.5) {
                assert!(angle_from(Axis::Vertical, &m, s) < 1.0, "{mode}: {}", angle_from(Axis::Vertical, &m, s));
            }
        }
    }

    #[test]
    fn level_only_rolls() {
        let (w, h) = (640, 480);
        let px = picture(w, h, 0.0, 0.05);
        let m = homography("level", &[], &px, w, h).expect("a roll");
        let s = detect_segments(&px, w, h).into_iter().find(|s| axis_of(s) == Axis::Vertical && seg_len(s) > 0.5).unwrap();
        assert!(angle_from(Axis::Vertical, &m, &s) < 0.5);
        // A pure rotation: no perspective row.
        assert!(m.0[6].abs() < 1e-9 && m.0[7].abs() < 1e-9);
    }

    #[test]
    fn a_blank_image_needs_no_correction() {
        let px = vec![[0.5f32; 4]; 64 * 48];
        for mode in ["auto", "level", "vertical", "full"] {
            assert!(homography(mode, &[], &px, 64, 48).is_none());
        }
    }
}
