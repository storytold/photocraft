//! Upright: homographies that make the image's lines level and plumb.
//!
//! Ours, after the idea of H. Lee, E. Shechtman, J. Wang, S. Lee, *Automatic Upright Adjustment
//! of Photographs*, CVPR 2012: line segments ([`super::lines`]) vote for a vertical and a
//! horizontal vanishing point (RANSAC over pairs, then a least-squares refinement), and the
//! correction sends them to infinity. What each mode corrects was measured as a black box on
//! Photoshop 2026 (a synthetic facade tilted by known amounts, the correction recovered by image
//! alignment); Photoshop's line analysis itself is not public, so on photographs the two pick
//! somewhat different lines and results are close in spirit, not identical.
//!
//! * **Level**: a rotation only, from the near-horizontal and near-vertical segments.
//! * **Vertical**: rotation and keystone so converging verticals become parallel and plumb
//!   (Photoshop corrects them fully).
//! * **Full**: also the horizontal vanishing point (two-point rectification).
//! * **Auto**: Vertical, plus [`AUTO_HORIZONTAL`] of the horizontal keystone, and that only
//!   when the verticals needed correcting (Photoshop left a pure horizontal tilt alone and took
//!   about 70 % of it off a tilt in both directions).
//! * **Guided**: the user's guides (up to four, in 0 … 1 image coordinates) instead of detected
//!   lines; each becomes vertical or horizontal, whichever it is nearer.
//!
//! The keystone stretches along its direction by `√(1 + (f·c·r)²)` (a camera turned about an
//! axis in the image plane, `f` = [`FOCAL`] half diagonals: about a 35 mm lens on a full
//! frame), which is what Photoshop's Upright measured, not its manual sliders' stretch.

use super::lines::{self, Segment};
use super::{Upright, mul};

/// Segments within this angle of vertical (or horizontal) vote for that vanishing point.
const CANDIDATE_DEG: f64 = 22.5;
/// Largest roll Upright applies (Camera Raw's Rotate slider ends at 10°): a bigger one means the
/// lines that voted were not the scene's verticals.
const MAX_ROLL_DEG: f64 = 10.0;
/// A vanishing point needs this many supporting segments, spread at least this far apart (half
/// diagonals), so one slanted object (an umbrella's ribs) cannot outvote the scene.
const MIN_SUPPORTERS: usize = 3;
const MIN_SPREAD: f64 = 0.3;
/// Share of a family's segment length its vanishing point must gather.
const VERTICAL_SHARE: f64 = 0.3;
const HORIZONTAL_SHARE: f64 = 0.5;
/// A segment supports a vanishing point when it points at it within this angle.
const INLIER_DEG: f64 = 1.5;
/// Largest keystone Upright applies, in slider units (Camera Raw's sliders end at 100).
const MAX_KEYSTONE: f64 = 100.0;
/// Auto's share of the horizontal keystone (when it also corrects verticals).
pub const AUTO_HORIZONTAL: f64 = 0.7;
/// Upright's focal length in half diagonals (measured: 1.4 … 1.7 over four tilts).
pub const FOCAL: f64 = 1.55;
/// Upright zooms in to fill the frame when that needs at most this much (measured: 1.076 was
/// filled, 1.12 was not).
pub const MAX_FILL: f64 = 1.10;

/// Upright's stretch for keystone `c` (1/px) on an image of half diagonal `r`.
fn upright_stretch(c: f64, r: f64) -> f64 {
    let t = FOCAL * c * r;
    if t.is_finite() { (1.0 + t * t).sqrt() } else { 1.0 }
}

fn luminance_proxy(px: &[[f32; 4]], w: usize, h: usize) -> Option<(Vec<f32>, usize, usize, f64)> {
    if w == 0 || h == 0 || px.len() < w.checked_mul(h)? {
        return None;
    }
    let f = w.max(h).div_ceil(lines::MAX_SIDE).max(1);
    let (lw, lh) = (w / f, h / f);
    if lw < 8 || lh < 8 {
        return None;
    }
    let mut out = vec![0.0f32; lw * lh];
    for y in 0..lh {
        for x in 0..lw {
            let mut s = 0.0f32;
            for j in 0..f {
                for i in 0..f {
                    let q = px.get((y * f + j) * w + x * f + i).copied().unwrap_or([0.0; 4]);
                    let l = 0.2126 * q[0] + 0.7152 * q[1] + 0.0722 * q[2];
                    s += if l.is_finite() { l.clamp(0.0, 1.0) * q[3].clamp(0.0, 1.0) } else { 0.0 };
                }
            }
            out[y * lw + x] = s / (f * f) as f32;
        }
    }
    Some((out, lw, lh, f as f64))
}

/// Detected segments in centred full-image coordinates.
fn segments(px: &[[f32; 4]], w: usize, h: usize) -> Vec<Segment> {
    let Some((lum, lw, lh, f)) = luminance_proxy(px, w, h) else { return Vec::new() };
    // Long lines only: short ones come in crowds from textures (planks, rugs, fringes).
    let min_len = (lw.min(lh) as f64 * 0.07).max(16.0);
    let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
    let mut segs = lines::detect(&lum, lw, lh, min_len);
    // Edges along the frame (a cut-off post, the picture's own border) say nothing about the
    // scene's verticals.
    let margin = (lw.min(lh) as f64 * 0.03).max(3.0);
    let (fw, fh) = (lw as f64, lh as f64);
    segs.retain(|s| [s.a, s.b].iter().all(|p| p[0] > margin && p[1] > margin && p[0] < fw - margin && p[1] < fh - margin));
    segs.truncate(600);
    segs.into_iter().map(|s| Segment { a: [s.a[0] * f - cx, s.a[1] * f - cy], b: [s.b[0] * f - cx, s.b[1] * f - cy] }).collect()
}

/// Angle of a segment from vertical (`vertical`) or horizontal, in degrees, folded to −90 … 90.
fn tilt(s: &Segment, vertical: bool) -> f64 {
    let (dx, dy) = (s.b[0] - s.a[0], s.b[1] - s.a[1]);
    let a = if vertical { (-dx).atan2(dy) } else { dy.atan2(dx) }.to_degrees();
    let mut a = a % 180.0;
    if a > 90.0 {
        a -= 180.0;
    } else if a < -90.0 {
        a += 180.0;
    }
    a
}

/// Whether segment `s` points at the homogeneous point `v` within `tol` degrees.
fn supports(s: &Segment, v: &[f64; 3], tol: f64) -> bool {
    let m = s.mid();
    let (dx, dy) = (s.b[0] - s.a[0], s.b[1] - s.a[1]);
    // Direction from the midpoint to v (v at infinity: its direction).
    let (tx, ty) = (v[0] - m[0] * v[2], v[1] - m[1] * v[2]);
    let (n1, n2) = (dx.hypot(dy), tx.hypot(ty));
    if !(n1 > 1e-9 && n2 > 1e-12) {
        return false;
    }
    let sin = ((dx * ty - dy * tx) / (n1 * n2)).abs();
    sin <= tol.to_radians().sin()
}

fn cross(a: &[f64; 3], b: &[f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn unit(v: [f64; 3]) -> Option<[f64; 3]> {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    (n > 1e-300 && n.is_finite()).then(|| v.map(|x| x / n))
}

/// Eigenvector of the smallest eigenvalue of a symmetric 3 × 3 matrix (Jacobi rotations).
fn smallest_eigenvector(m: [[f64; 3]; 3]) -> Option<[f64; 3]> {
    let mut a = m;
    let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for _ in 0..50 {
        let (mut p, mut q, mut big) = (0, 1, 0.0);
        for (i, j) in [(0, 1), (0, 2), (1, 2)] {
            if a[i][j].abs() > big {
                big = a[i][j].abs();
                p = i;
                q = j;
            }
        }
        if big < 1e-15 {
            break;
        }
        let theta = 0.5 * (2.0 * a[p][q]).atan2(a[q][q] - a[p][p]);
        let (c, s) = (theta.cos(), theta.sin());
        for k in 0..3 {
            let (akp, akq) = (a[k][p], a[k][q]);
            a[k][p] = c * akp - s * akq;
            a[k][q] = s * akp + c * akq;
        }
        for k in 0..3 {
            let (apk, aqk) = (a[p][k], a[q][k]);
            a[p][k] = c * apk - s * aqk;
            a[q][k] = s * apk + c * aqk;
        }
        for k in 0..3 {
            let (vkp, vkq) = (v[k][p], v[k][q]);
            v[k][p] = c * vkp - s * vkq;
            v[k][q] = s * vkp + c * vkq;
        }
    }
    let i = (0..3).min_by(|x, y| a[*x][*x].total_cmp(&a[*y][*y]))?;
    unit([v[0][i], v[1][i], v[2][i]])
}

/// The vanishing point most of `segs` point at (homogeneous, unit length), with the summed
/// length of its supporters, or `None` with too little support.
pub fn vanishing_point(segs: &[Segment], scale: f64) -> Option<([f64; 3], f64)> {
    if segs.len() < 2 {
        return None;
    }
    // Lines in coordinates divided by `scale`, so the point's coordinates stay well conditioned.
    let s = scale.max(1.0);
    let norm: Vec<Segment> = segs.iter().map(|g| Segment { a: g.a.map(|v| v / s), b: g.b.map(|v| v / s) }).collect();
    let lines: Vec<[f64; 3]> = norm.iter().filter_map(Segment::line).collect();
    if lines.len() != norm.len() {
        return None;
    }
    let n = norm.len().min(80);
    let mut best: Option<([f64; 3], f64)> = None;
    for i in 0..n {
        for j in i + 1..n {
            let Some(v) = unit(cross(&lines[i], &lines[j])) else { continue };
            // Length² so a few long lines outweigh many short parallel ones.
            let score: f64 = norm.iter().filter(|g| supports(g, &v, INLIER_DEG)).map(|g| g.length() * g.length()).sum();
            if best.is_none_or(|(_, b)| score > b) {
                best = Some((v, score));
            }
        }
    }
    let (v0, _) = best?;
    // Least squares over the supporters: the point closest to all their lines.
    let mut m = [[0.0; 3]; 3];
    let mut support = 0.0;
    let mut count = 0;
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for (g, l) in norm.iter().zip(&lines) {
        if supports(g, &v0, INLIER_DEG * 2.0) {
            let wgt = g.length();
            support += wgt;
            let wgt = wgt * wgt;
            count += 1;
            let mid = g.mid();
            for k in 0..2 {
                lo[k] = lo[k].min(mid[k]);
                hi[k] = hi[k].max(mid[k]);
            }
            for r in 0..3 {
                for c in 0..3 {
                    m[r][c] += wgt * l[r] * l[c];
                }
            }
        }
    }
    if count < MIN_SUPPORTERS || (hi[0] - lo[0]).hypot(hi[1] - lo[1]) < MIN_SPREAD {
        return None;
    }
    let v = smallest_eigenvector(m).unwrap_or(v0);
    // Back to pixels: (x/s, y/s, w) → (x, y, w/s).
    let v = unit([v[0], v[1], v[2] / s])?;
    Some((v, support * s))
}

/// Rotation (clockwise, degrees) that levels the image by its near-axis segments, length-weighted
/// median of their tilts.
pub fn level_angle(segs: &[Segment]) -> Option<f64> {
    let mut t: Vec<(f64, f64)> = Vec::new();
    for s in segs {
        for vertical in [true, false] {
            let a = tilt(s, vertical);
            if a.abs() < 10.0 {
                t.push((a, s.length()));
            }
        }
    }
    if t.is_empty() {
        return None;
    }
    t.sort_by(|a, b| a.0.total_cmp(&b.0));
    let total: f64 = t.iter().map(|x| x.1).sum();
    let mut acc = 0.0;
    for (a, wgt) in &t {
        acc += wgt;
        if acc >= total / 2.0 {
            // Tilts measured clockwise: undo them.
            return Some(-a);
        }
    }
    None
}

fn rotation(deg: f64) -> [f64; 9] {
    let t = deg.to_radians();
    [t.cos(), -t.sin(), 0.0, t.sin(), t.cos(), 0.0, 0.0, 0.0, 1.0]
}

/// Rotation and vertical keystone sending vanishing point `v` (homogeneous, centred pixels) to
/// infinity straight down / up, at most `limit` slider units of keystone.
pub fn vertical_fix(v: &[f64; 3], w: f64, h: f64, limit: f64) -> Option<[f64; 9]> {
    let r = (w.hypot(h) / 2.0).max(1e-9);
    // Direction of v from the centre.
    let (mut dx, mut dy) = (v[0], v[1]);
    if v[2] < 0.0 {
        dx = -dx;
        dy = -dy;
    }
    if !(dy.abs() > 1e-12) || (dx / dy).abs() > (CANDIDATE_DEG * 1.5).to_radians().tan() {
        return None;
    }
    let theta = (dx / dy).atan();
    if theta.to_degrees().abs() > MAX_ROLL_DEG {
        return None;
    }
    let rot = [theta.cos(), -theta.sin(), 0.0, theta.sin(), theta.cos(), 0.0, 0.0, 0.0, 1.0];
    if v[2].abs() < 1e-12 {
        return Some(rot);
    }
    let y = (theta.sin() * v[0] + theta.cos() * v[1]) / v[2];
    if !(y.abs() > 1e-9) {
        return None;
    }
    // c = −1/y, in slider units −0.006·v/r.
    let slider = (y.recip() * r / 0.006).clamp(-limit, limit);
    let c = -0.006 * slider / r;
    let a = upright_stretch(c, r);
    let p = [1.0, 0.0, 0.0, 0.0, a, 0.0, 0.0, c, 1.0];
    Some(mul(&p, &rot))
}

const SWAP: [f64; 9] = [0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

/// [`vertical_fix`] for a horizontal vanishing point (axes swapped).
pub fn horizontal_fix(v: &[f64; 3], w: f64, h: f64, limit: f64) -> Option<[f64; 9]> {
    let m = vertical_fix(&[v[1], v[0], v[2]], h, w, limit)?;
    Some(mul(&SWAP, &mul(&m, &SWAP)))
}

fn apply_point(m: &[f64; 9], v: &[f64; 3]) -> [f64; 3] {
    [
        m[0] * v[0] + m[1] * v[1] + m[2] * v[2],
        m[3] * v[0] + m[4] * v[1] + m[5] * v[2],
        m[6] * v[0] + m[7] * v[1] + m[8] * v[2],
    ]
}

/// Both vanishing points: the vertical first, then the horizontal one where it lands after it,
/// with a keystone along x and a shear that keep the verticals plumb.
fn full_fix(vv: Option<&[f64; 3]>, vh: Option<&[f64; 3]>, w: f64, h: f64, lim_v: f64, share_h: f64) -> Option<[f64; 9]> {
    let first = vv.and_then(|v| vertical_fix(v, w, h, lim_v));
    // Without a vertical vanishing point to anchor it, Photoshop's Full leaves horizontals alone
    // too (an upright terrace render: no correction).
    let Some(base) = first else {
        return if share_h > 1.0 { vh.and_then(|v| horizontal_fix(v, w, h, MAX_KEYSTONE)) } else { None };
    };
    let Some(v) = vh else { return Some(base) };
    // Auto corrects horizontals only alongside a real vertical keystone (Photoshop left a pure
    // horizontal tilt alone).
    if share_h < 1.0 && base[6].hypot(base[7]) * (w.hypot(h) / 2.0) / 0.006 < 2.0 {
        return Some(base);
    }
    let m = apply_point(&base, v);
    if !(m[2].abs() > 1e-12) {
        return Some(base);
    }
    let (x, y) = (m[0] / m[2], m[1] / m[2]);
    if !(x.abs() > 1e-9) {
        return Some(base);
    }
    let r = (w.hypot(h) / 2.0).max(1e-9);
    // Row (gx, 0, 1) sends the point to infinity; in slider units gx = −0.006·h/r.
    let slider = (x.recip() * r / 0.006 * share_h).clamp(-MAX_KEYSTONE, MAX_KEYSTONE);
    let gx = -0.006 * slider / r;
    let sx = upright_stretch(gx, r);
    // Where it now lands (after the clamp) sets the shear that makes it horizontal.
    let k = -y / x;
    let g = [sx, 0.0, 0.0, k, 1.0, 0.0, gx, 0.0, 1.0];
    Some(mul(&g, &base))
}

/// The Upright homography (centred pixels) for a mode that analyses the image.
pub fn analyse(px: &[[f32; 4]], w: usize, h: usize, mode: Upright) -> Option<[f64; 9]> {
    let segs = segments(px, w, h);
    homography_from_segments(&segs, w as f64, h as f64, mode)
}

/// The Upright homography from segments in centred pixel coordinates.
pub fn homography_from_segments(segs: &[Segment], w: f64, h: f64, mode: Upright) -> Option<[f64; 9]> {
    let scale = w.hypot(h) / 2.0;
    let near = |s: &&Segment, vertical: bool| tilt(s, vertical).abs() < CANDIDATE_DEG;
    let verticals: Vec<Segment> = segs.iter().filter(|s| near(s, true)).copied().collect();
    let horizontals: Vec<Segment> = segs.iter().filter(|s| near(s, false)).copied().collect();
    // A vanishing point needs a little support: 15 % of the half diagonal of segment length.
    // A vanishing point needs some length behind it, and most of its family's length: when
    // horizontals run off to several points (a room, a terrace) Photoshop leaves them alone.
    let total = |set: &[Segment]| set.iter().map(Segment::length).sum::<f64>();
    let (tv, th) = (total(&verticals), total(&horizontals));
    let enough = |v: Option<([f64; 3], f64)>, all: f64, share: f64| v.filter(|(_, s)| *s > 0.15 * scale && *s >= share * all).map(|(v, _)| v);
    match mode {
        Upright::Off | Upright::Guided => None,
        Upright::Level => level_angle(segs).map(rotation),
        Upright::Vertical => {
            let vv = enough(vanishing_point(&verticals, scale), tv, VERTICAL_SHARE);
            vv.and_then(|v| vertical_fix(&v, w, h, MAX_KEYSTONE)).or_else(|| level_angle(segs).map(rotation))
        }
        Upright::Full => {
            let vv = enough(vanishing_point(&verticals, scale), tv, VERTICAL_SHARE);
            let vh = enough(vanishing_point(&horizontals, scale), th, HORIZONTAL_SHARE);
            full_fix(vv.as_ref(), vh.as_ref(), w, h, MAX_KEYSTONE, 1.0).or_else(|| level_angle(segs).map(rotation))
        }
        Upright::Auto => {
            let vv = enough(vanishing_point(&verticals, scale), tv, VERTICAL_SHARE);
            let vh = enough(vanishing_point(&horizontals, scale), th, HORIZONTAL_SHARE);
            full_fix(vv.as_ref(), vh.as_ref(), w, h, MAX_KEYSTONE, AUTO_HORIZONTAL).or_else(|| level_angle(segs).map(rotation))
        }
    }
}

/// Guided Upright from up to four guides `[[x0, y0], [x1, y1]]` in 0 … 1 image coordinates.
pub fn guided(guides: &[[[f32; 2]; 2]], w: f64, h: f64) -> Option<[f64; 9]> {
    let segs: Vec<Segment> = guides
        .iter()
        .take(super::MAX_GUIDES)
        .filter(|g| g.iter().flatten().all(|v| v.is_finite()))
        .map(|g| Segment {
            a: [(f64::from(g[0][0]) - 0.5) * w, (f64::from(g[0][1]) - 0.5) * h],
            b: [(f64::from(g[1][0]) - 0.5) * w, (f64::from(g[1][1]) - 0.5) * h],
        })
        .filter(|s| s.length() > 1.0)
        .collect();
    let (verticals, horizontals): (Vec<Segment>, Vec<Segment>) = segs.iter().partition(|s| tilt(s, true).abs() <= 45.0);
    let vp = |set: &[Segment]| -> Option<[f64; 3]> {
        match set {
            [] => None,
            [one] => {
                // One guide: its direction, as a point at infinity.
                let d = [one.b[0] - one.a[0], one.b[1] - one.a[1], 0.0];
                unit(d)
            }
            [a, b] => unit(cross(&a.line()?, &b.line()?)),
            many => vanishing_point(many, w.hypot(h) / 2.0).map(|(v, _)| v),
        }
    };
    let (vv, vh) = (vp(&verticals), vp(&horizontals));
    match (vv, vh) {
        (None, None) => None,
        (Some(v), None) => vertical_fix(&v, w, h, MAX_KEYSTONE),
        (None, Some(v)) => horizontal_fix(&v, w, h, MAX_KEYSTONE),
        (Some(a), Some(b)) => full_fix(Some(&a), Some(&b), w, h, MAX_KEYSTONE, 1.0),
    }
}

/// Prints what the analysis sees (the `upright_debug` test).
#[cfg(test)]
pub fn debug(px: &[[f32; 4]], w: usize, h: usize) {
    let segs = segments(px, w, h);
    let scale = (w as f64).hypot(h as f64) / 2.0;
    let verticals: Vec<Segment> = segs.iter().filter(|s| tilt(s, true).abs() < CANDIDATE_DEG).copied().collect();
    println!("{} segments, {} near vertical", segs.len(), verticals.len());
    for s in verticals.iter().take(40) {
        println!("  v len {:6.1} tilt {:+6.2} mid {:?}", s.length(), tilt(s, true), s.mid().map(|v| v.round()));
    }
    if let Some((v, sup)) = vanishing_point(&verticals, scale) {
        println!("vertical VP {v:?} support {sup:.0}");
        for s in verticals.iter().filter(|s| supports(s, &v, INLIER_DEG * 2.0)) {
            println!("  supporter len {:6.1} tilt {:+6.2} mid {:?}", s.length(), tilt(s, true), s.mid().map(|v| v.round()));
        }
    }
}
