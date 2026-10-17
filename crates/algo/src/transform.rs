//! Free Transform: projective warps of surfaces.
//!
//! A transform is described by where the four corners of a source rectangle land (a quad). Scale,
//! rotate, skew and flips are affine special cases; Distort and Perspective use the full
//! homography. Warping is an inverse mapping with premultiplied-alpha sampling (nearest, bilinear
//! or Photoshop's Keys bicubic), processed per destination tile (in parallel on native) so memory stays
//! bounded by the tile working set. Bicubic reductions are pre-filtered with a proper resize
//! first, so shrinking a layer doesn't alias; an affine map reduces each axis by its own scale, so
//! an axis that doesn't shrink stays sharp.

use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
use serde::{Deserialize, Serialize};

use crate::resample::{resize_surface_per_axis, scaled_rect};

/// 3×3 projective matrix, row-major: `[x', y', w'] = H · [x, y, 1]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Homography(pub [f64; 9]);

impl Homography {
    pub const IDENTITY: Homography = Homography([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);

    pub fn is_identity(&self) -> bool {
        let m = &self.0;
        if m[8].abs() < 1e-12 {
            return false;
        }
        let w = m[8];
        ((m[0] / w) - 1.0).abs() < 1e-9
            && ((m[4] / w) - 1.0).abs() < 1e-9
            && (m[1] / w).abs() < 1e-9
            && (m[2] / w).abs() < 1e-9
            && (m[3] / w).abs() < 1e-9
            && (m[5] / w).abs() < 1e-9
            && (m[6] / w).abs() < 1e-9
            && (m[7] / w).abs() < 1e-9
    }

    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        let m = &self.0;
        let w = m[6] * x + m[7] * y + m[8];
        ((m[0] * x + m[1] * y + m[2]) / w, (m[3] * x + m[4] * y + m[5]) / w)
    }

    /// The homogeneous `w` of `(x, y)`: its sign is the side of the horizon the point maps to
    /// (0 on the horizon itself).
    pub fn w(&self, x: f64, y: f64) -> f64 {
        let m = &self.0;
        m[6] * x + m[7] * y + m[8]
    }

    /// The side of the horizon (+1 or −1) that most corners of `r` (`[x0, y0, x1, y1]`) map to,
    /// the centre's on a tie. A Distort dragged past the horizon maps the rest of the source to the
    /// far side, where it would show as a mirrored copy wrapped round infinity; drawing keeps to
    /// this side, as Photoshop's does (#4863).
    pub fn front_side(&self, r: [f64; 4]) -> f64 {
        let ahead = [(r[0], r[1]), (r[2], r[1]), (r[2], r[3]), (r[0], r[3])].iter().filter(|(x, y)| self.w(*x, *y) > 0.0).count();
        match ahead {
            3 | 4 => 1.0,
            0 | 1 => -1.0,
            _ if self.w((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0) >= 0.0 => 1.0,
            _ => -1.0,
        }
    }

    pub fn mul(&self, o: &Homography) -> Homography {
        let (a, b) = (&self.0, &o.0);
        let mut r = [0.0; 9];
        for i in 0..3 {
            for j in 0..3 {
                r[i * 3 + j] = (0..3).map(|k| a[i * 3 + k] * b[k * 3 + j]).sum();
            }
        }
        Homography(r)
    }

    pub fn inverse(&self) -> Option<Homography> {
        let m = &self.0;
        let c = [
            m[4] * m[8] - m[5] * m[7],
            m[2] * m[7] - m[1] * m[8],
            m[1] * m[5] - m[2] * m[4],
            m[5] * m[6] - m[3] * m[8],
            m[0] * m[8] - m[2] * m[6],
            m[2] * m[3] - m[0] * m[5],
            m[3] * m[7] - m[4] * m[6],
            m[1] * m[6] - m[0] * m[7],
            m[0] * m[4] - m[1] * m[3],
        ];
        let det = m[0] * c[0] + m[1] * c[3] + m[2] * c[6];
        if det.abs() < 1e-12 {
            return None;
        }
        Some(Homography(c.map(|v| v / det)))
    }

    /// Unit square → quad (corners in order: (0,0), (1,0), (1,1), (0,1)).
    fn square_to_quad(q: [[f64; 2]; 4]) -> Option<Homography> {
        let [[x0, y0], [x1, y1], [x2, y2], [x3, y3]] = q;
        let (sx, sy) = (x0 - x1 + x2 - x3, y0 - y1 + y2 - y3);
        if sx.abs() < 1e-12 && sy.abs() < 1e-12 {
            // Affine.
            return Some(Homography([x1 - x0, x3 - x0, x0, y1 - y0, y3 - y0, y0, 0.0, 0.0, 1.0]));
        }
        let (dx1, dx2, dy1, dy2) = (x1 - x2, x3 - x2, y1 - y2, y3 - y2);
        let den = dx1 * dy2 - dx2 * dy1;
        if den.abs() < 1e-12 {
            return None;
        }
        let g = (sx * dy2 - dx2 * sy) / den;
        let h = (dx1 * sy - sx * dy1) / den;
        Some(Homography([x1 - x0 + g * x1, x3 - x0 + h * x3, x0, y1 - y0 + g * y1, y3 - y0 + h * y3, y0, g, h, 1.0]))
    }

    /// The transform taking rectangle `r` (x0, y0, x1, y1) onto `quad` (corners clockwise from
    /// top-left). `None` for degenerate quads.
    pub fn rect_to_quad(r: [f64; 4], quad: [[f64; 2]; 4]) -> Option<Homography> {
        let (w, h) = (r[2] - r[0], r[3] - r[1]);
        if w <= 0.0 || h <= 0.0 {
            return None;
        }
        let to_unit = Homography([1.0 / w, 0.0, -r[0] / w, 0.0, 1.0 / h, -r[1] / h, 0.0, 0.0, 1.0]);
        Some(Self::square_to_quad(quad)?.mul(&to_unit))
    }

    /// Approximate linear scale of the mapping near `(x, y)` (sqrt of the Jacobian determinant).
    pub fn local_scale(&self, x: f64, y: f64) -> f64 {
        let e = 0.5;
        let (a, b) = (self.apply(x - e, y), self.apply(x + e, y));
        let (c, d) = (self.apply(x, y - e), self.apply(x, y + e));
        let (jx, jy) = ((b.0 - a.0, b.1 - a.1), (d.0 - c.0, d.1 - c.1));
        (jx.0 * jy.1 - jx.1 * jy.0).abs().sqrt()
    }

    /// The map's Jacobian at `(x, y)`, `[∂X/∂x, ∂X/∂y, ∂Y/∂x, ∂Y/∂y]`, exact (an affine map gives
    /// the same matrix everywhere). Its columns' lengths are how far a step of one source pixel
    /// along x, and one along y, moves in the destination, which [`Self::local_scale`]'s area
    /// measure can't tell apart.
    pub(crate) fn jacobian(&self, x: f64, y: f64) -> [f64; 4] {
        let m = &self.0;
        let w = self.w(x, y);
        let (px, py) = self.apply(x, y);
        [(m[0] - m[6] * px) / w, (m[1] - m[7] * px) / w, (m[3] - m[6] * py) / w, (m[4] - m[7] * py) / w]
    }
}

/// Resampling used by transforms (Photoshop's interpolation menu).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Interp {
    Nearest,
    Bilinear,
    #[default]
    Bicubic,
}

impl Interp {
    pub fn parse(s: &str) -> Self {
        match s {
            "nearest" | "nearestNeighbor" => Interp::Nearest,
            "bilinear" => Interp::Bilinear,
            _ => Interp::Bicubic,
        }
    }
}

/// Photoshop's Bicubic: the Keys cubic at a = −0.75 (Catmull-Rom is −0.5), the weights of the
/// four source pixels around a sample `t` (0..1) past the second. Measured on Photoshop 27.11's
/// Free Transform: a 200 % step edge matches within 0.5 levels; Catmull-Rom was 6 off.
pub(crate) fn bicubic(t: f64) -> [f64; 4] {
    const A: f64 = -0.75;
    let near = |x: f64| (A + 2.0) * x * x * x - (A + 3.0) * x * x + 1.0;
    let far = |x: f64| A * (x * x * x - 5.0 * x * x + 8.0 * x - 4.0);
    [far(1.0 + t), near(t), near(1.0 - t), far(2.0 - t)]
}

/// Warp `src` (its content inside `src_rect`) by `h` (source → destination document space).
/// The result has the same format as `src`, with alpha added if `src` had none; pixels outside
/// the warped quad are transparent.
pub fn warp_surface(src: &Surface, src_rect: Rect, h: &Homography, interp: Interp) -> Result<Surface, photocraft_raster::AllocationError> {
    warp(src, src_rect, h, interp, None, &Interrupt::NONE)
}

/// [`warp_surface`] reporting progress per destination tile; `Ok(None)` once `ctl` is cancelled.
pub fn warp_surface_with(
    src: &Surface,
    src_rect: Rect,
    h: &Homography,
    interp: Interp,
    ctl: &Interrupt,
) -> Result<Option<Surface>, photocraft_raster::AllocationError> {
    let out = warp(src, src_rect, h, interp, None, ctl)?;
    Ok((!ctl.cancelled()).then_some(out))
}

/// [`warp_surface`] keeping only the destination pixels inside `clip`. The work is bounded by
/// `clip` even when part of the source lies at or beyond the map's horizon (a Perspective Crop of a
/// layer that reaches far past the crop quad), which would make the full warped extent unbounded.
pub fn warp_surface_clipped(src: &Surface, src_rect: Rect, h: &Homography, interp: Interp, clip: Rect) -> Result<Surface, photocraft_raster::AllocationError> {
    warp(src, src_rect, h, interp, Some(clip), &Interrupt::NONE)
}

/// Is `q` (TL, TR, BR, BL) a usable Perspective Crop quad: finite, strictly convex with every
/// corner angle above ~0.06°, wound clockwise on screen (y down) and at least 1 px² in area?
pub fn convex_quad(q: &[[f64; 2]; 4]) -> bool {
    if q.iter().flatten().any(|v| !v.is_finite()) {
        return false;
    }
    let mut area2 = 0.0;
    for i in 0..4 {
        let (a, b, c) = (q[i], q[(i + 1) % 4], q[(i + 2) % 4]);
        let (e1, e2) = ([b[0] - a[0], b[1] - a[1]], [c[0] - b[0], c[1] - b[1]]);
        let cross = e1[0] * e2[1] - e1[1] * e2[0];
        // sin(turn) = cross / (|e1| |e2|): positive (clockwise on screen) and not a straight angle.
        if cross.is_nan() || cross <= 1e-3 * e1[0].hypot(e1[1]) * e2[0].hypot(e2[1]) {
            return false;
        }
        area2 += a[0] * b[1] - b[0] * a[1];
    }
    area2.is_finite() && area2 >= 2.0
}

/// The output size a Perspective Crop of `q` (TL, TR, BR, BL) takes when none is given: the mean
/// length of the top and bottom edges by the mean length of the left and right edges.
pub fn quad_size(q: &[[f64; 2]; 4]) -> (f64, f64) {
    let len = |a: [f64; 2], b: [f64; 2]| (b[0] - a[0]).hypot(b[1] - a[1]);
    let [tl, tr, br, bl] = *q;
    ((len(tl, tr) + len(bl, br)) / 2.0, (len(tl, bl) + len(tr, br)) / 2.0)
}

/// The homography taking the quad `q` (TL, TR, BR, BL) onto the rectangle (0, 0)–(`w`, `h`), corner
/// to corner: the inverse of the unit-square-to-quad map (Heckbert, "Fundamentals of Texture
/// Mapping and Image Warping", 1989, §2.2.3) scaled to the rectangle. `None` when `q` isn't a
/// [`convex_quad`] or the size isn't positive.
pub fn quad_to_rect(q: &[[f64; 2]; 4], w: f64, h: f64) -> Option<Homography> {
    if !(convex_quad(q) && w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
        return None;
    }
    Homography::rect_to_quad([0.0, 0.0, w, h], *q)?.inverse()
}

/// The destination rectangle [`warp_surface`] fills for `src_rect` under `h`: the warped corners
/// plus a pixel for filter support, each side clamped to ±2²⁰. `None` when a corner maps to
/// infinity (the result is then empty). Lets a caller price a warp before allocating it.
pub fn warp_output_bounds(src_rect: Rect, h: &Homography) -> Option<Rect> {
    if src_rect.is_empty() || h.inverse().is_none() {
        return None;
    }
    let corners = [(src_rect.x0, src_rect.y0), (src_rect.x1, src_rect.y0), (src_rect.x1, src_rect.y1), (src_rect.x0, src_rect.y1)]
        .map(|(x, y)| h.apply(x as f64, y as f64));
    corner_bounds(&corners)
}

fn corner_bounds(corners: &[(f64, f64); 4]) -> Option<Rect> {
    if !corners.iter().all(|c| c.0.is_finite() && c.1.is_finite()) {
        return None;
    }
    let lim = (1 << 20) as f64;
    let bx0 = corners.iter().map(|c| c.0).fold(f64::MAX, f64::min).floor().max(-lim) as i32 - 1;
    let by0 = corners.iter().map(|c| c.1).fold(f64::MAX, f64::min).floor().max(-lim) as i32 - 1;
    let bx1 = corners.iter().map(|c| c.0).fold(f64::MIN, f64::max).ceil().min(lim) as i32 + 1;
    let by1 = corners.iter().map(|c| c.1).fold(f64::MIN, f64::max).ceil().min(lim) as i32 + 1;
    Some(Rect::new(bx0, by0, bx1, by1))
}

/// The factors a Bicubic warp pre-reduces its source by along x and along y, 1 for an axis that
/// doesn't shrink. An affine map (scale, skew, rotation) reduces the source the same everywhere, so
/// each axis gets its own factor, the length of its Jacobian column: one factor for both axes
/// (√|det J|) blurred an axis the map keeps 1:1 while it squashes the other, and filtered the
/// squashed axis for less than its reduction (#4864). A map that squashes along a slant (a rotated
/// squash, or a skew that also flattens) shrinks a direction between the axes far more than either
/// axis, which no axis-aligned prefilter follows; there both axes come down together until the
/// warp shrinks no direction below what the one factor leaves it, or below 50 % (where a 4×4
/// kernel starts to alias), whichever is lower. Where the one factor leaves more than 50 %, the
/// pair keeps more detail than it does, with a little more room to alias along the slant. A
/// perspective map (a Distort) reduces each part of the source by a different amount, which no
/// single prefilter follows either, so it keeps the one factor at the source point that matters,
/// as before.
fn prefilter_scales(h: &Homography, src_rect: Rect, clip: Option<Rect>) -> (f64, f64) {
    let r = [f64::from(src_rect.x0), f64::from(src_rect.y0), f64::from(src_rect.x1), f64::from(src_rect.y1)];
    // With a clip, the scale that matters is the one at the source point landing in the clip's centre.
    let (cx, cy) = match clip.and_then(|c| Some((c, h.inverse()?))) {
        Some((c, inv)) => inv.apply((f64::from(c.x0) + f64::from(c.x1)) / 2.0, (f64::from(c.y0) + f64::from(c.y1)) / 2.0),
        None => ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0),
    };
    // `resize_surface_per_axis` copies an axis whose scale is exactly 1; 1/64 bounds the filter's width.
    let snap = |f: f64| if f > 0.0 && f < 0.999 { f.max(1.0 / 64.0) } else { 1.0 };
    let one = snap(h.local_scale(cx, cy));
    // Affine when w is the same over the whole source. A Distort's quad can leave the projective
    // terms at rounding level, so this compares w itself rather than testing them for zero.
    let w0 = h.w(cx, cy);
    let affine =
        w0.is_finite() && w0 != 0.0 && [(r[0], r[1]), (r[2], r[1]), (r[2], r[3]), (r[0], r[3])].iter().all(|&(x, y)| (h.w(x, y) - w0).abs() <= 1e-9 * w0.abs());
    let j = h.jacobian(cx, cy);
    if !affine || !j.iter().all(|e| e.is_finite()) {
        return (one, one);
    }
    let (fx, fy) = (snap(j[0].hypot(j[2])), snap(j[1].hypot(j[3])));
    // A uniform scale, turned or not, keeps the one factor: its column lengths differ from it only
    // by rounding, which would otherwise pick the order of the two passes.
    if (fx - one).abs() <= 1e-9 * one && (fy - one).abs() <= 1e-9 * one {
        return (one, one);
    }
    // The warp is left `J · diag(1/fx, 1/fy)`. Along a slant both columns stay long while the map
    // squashes the direction between them, so both axes come down together until the warp shrinks
    // no direction below √(σmin / σmax), what the one factor √|det J| leaves, or below 50 %,
    // whichever is lower. A map that collapses (c = 0) reduces nothing, as before.
    let (hi, lo) = singular_values(j);
    let (_, left) = singular_values([j[0] / fx, j[1] / fy, j[2] / fx, j[3] / fy]);
    let target = if hi > 0.0 { (lo / hi).sqrt().min(0.5) } else { 0.0 };
    let c = if target > 0.0 && left.is_finite() { left / target } else { 0.0 };
    if c < 1.0 { (snap(fx * c), snap(fy * c)) } else { (fx, fy) }
}

/// The singular values of the 2×2 matrix `[a, b, c, d]` (row-major), largest first: how far a
/// unit step is stretched along the map's strongest and weakest directions. 0 for a matrix that
/// isn't finite.
fn singular_values(j: [f64; 4]) -> (f64, f64) {
    // Scaled by the largest entry, so the squares can't overflow.
    let m = j.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    if !(m > 0.0 && m.is_finite()) {
        return (0.0, 0.0);
    }
    let [a, b, c, d] = j.map(|v| v / m);
    let (f, det) = (a * a + b * b + c * c + d * d, (a * d - b * c).abs());
    let hi = ((f + (f * f - 4.0 * det * det).max(0.0).sqrt()) / 2.0).sqrt();
    (hi * m, if hi > 0.0 { det / hi * m } else { 0.0 })
}

/// A cancelled `ctl` skips the remaining tiles; the caller discards the result.
fn warp(
    src: &Surface,
    src_rect: Rect,
    h: &Homography,
    interp: Interp,
    clip: Option<Rect>,
    ctl: &Interrupt,
) -> Result<Surface, photocraft_raster::AllocationError> {
    let mut fmt = src.format();
    let converted;
    let mut src = if fmt.alpha {
        src
    } else {
        fmt = PixelFormat::new(fmt.mode, fmt.sample, true);
        converted = src.try_convert(fmt)?;
        &converted
    };
    let mut out = Surface::new(fmt);
    if src_rect.is_empty() || clip.is_some_and(|c| c.is_empty()) {
        return Ok(out);
    }
    let mut h = *h;
    let mut src_rect = src_rect;
    let reduced;
    // Photoshop reduces with its Bicubic widened by 1 / scale (as Image Size does), so the source is
    // brought down by the local scale first and the warp samples it at about 1:1; a plain 4×4
    // kernel at 50 % aliased (Photoshop 27.11 Free Transform, 50 % of an impulse image). Bilinear
    // samples the source directly, as Photoshop's does (exact at 50 % without a prefilter).
    if interp == Interp::Bicubic {
        let (fx, fy) = prefilter_scales(&h, src_rect, clip);
        if fx < 1.0 || fy < 1.0 {
            reduced = resize_surface_per_axis(src, fx, fy);
            src = &reduced;
            h = h.mul(&Homography([1.0 / fx, 0.0, 0.0, 0.0, 1.0 / fy, 0.0, 0.0, 0.0, 1.0]));
            src_rect = scaled_rect(src_rect, fx, fy);
        }
    }
    let Some(inv) = h.inverse() else { return Ok(out) };
    // Destination bounds: the warped corners (plus a pixel for filter support).
    let corners = [(src_rect.x0, src_rect.y0), (src_rect.x1, src_rect.y0), (src_rect.x1, src_rect.y1), (src_rect.x0, src_rect.y1)]
        .map(|(x, y)| h.apply(x as f64, y as f64));
    // A sampled pixel's inverse w is 1 / (forward w at its source point). While the forward w keeps one sign over the source
    // (plus the margin the sampler reads), every sampled pixel lies at least `w_lo` from the inverse horizon, so clipping a tile
    // there keeps its footprint exact and bounded even when a steep Distort brings that horizon into the tile (#2271).
    let (sx0, sy0, sx1, sy1) = (src_rect.x0 as f64 - 1.0, src_rect.y0 as f64 - 1.0, src_rect.x1 as f64 + 1.0, src_rect.y1 as f64 + 1.0);
    let fwd_w = [(sx0, sy0), (sx1, sy0), (sx1, sy1), (sx0, sy1)].map(|(x, y)| h.0[6] * x + h.0[7] * y + h.0[8]);
    let side = [1.0, -1.0].into_iter().find(|&s| fwd_w.iter().all(|&c| s * c > 0.0));
    let w_lo = 1.0 / fwd_w.iter().fold(0.0f64, |m, c| m.max(c.abs()));
    // A source straddling the horizon draws only its front side: the inverse w at a pixel has the
    // sign of the forward w at the source point it samples, and the other side would show as a
    // mirrored copy wrapped round infinity (#4863).
    let front = side.is_none().then(|| h.front_side([src_rect.x0 as f64, src_rect.y0 as f64, src_rect.x1 as f64, src_rect.y1 as f64]));
    let bounds = corner_bounds(&corners);
    let dst = match (clip, bounds) {
        // The warped corners only bound the result while the whole source is in front of the horizon.
        (Some(c), Some(b)) if side.is_some() => b.intersect(&c),
        (Some(c), _) => c,
        (None, Some(b)) => b,
        (None, None) => return Ok(out),
    };
    if dst.is_empty() {
        return Ok(out);
    }
    let n = fmt.channels();
    let a = n - 1;
    let tile_count = dst.tiles().count();
    let tile_bytes = tile_count.checked_mul(std::mem::size_of::<Rect>()).ok_or(photocraft_raster::AllocationError::SizeOverflow)?;
    let mut tiles = Vec::new();
    tiles.try_reserve_exact(tile_count).map_err(|_| photocraft_raster::AllocationError::NotEnoughMemory { bytes: tile_bytes })?;
    tiles.extend(dst.tiles().map(|tc| tc.rect().intersect(&dst)).filter(|r| !r.is_empty()));
    let src_ref: &Surface = src;
    // Tiles finished so far, for progress reported per tile (from any worker thread).
    let finished = std::sync::atomic::AtomicUsize::new(0);
    let total = tiles.len().max(1);
    let work = |t: &Rect| -> Result<Option<(Rect, Vec<f32>)>, photocraft_raster::AllocationError> {
        if ctl.cancelled() {
            return Ok(None);
        }
        let k = finished.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        // The last few percent are the writes.
        ctl.progress(0.98 * k as f32 / total as f32);
        // Source footprint of this tile (inverse-mapped corners of its sampled part), padded for the filter.
        let tc = [(t.x0, t.y0), (t.x1, t.y0), (t.x1, t.y1), (t.x0, t.y1)].map(|(x, y)| (x as f64, y as f64));
        let wc = tc.map(|(x, y)| inv.0[6] * x + inv.0[7] * y + inv.0[8]);
        let poly = match side {
            // Sutherland-Hodgman against `s * w >= w_lo`.
            Some(s) => {
                let d = wc.map(|w| s * w - w_lo);
                let mut poly = Vec::with_capacity(5);
                for (i, (&p, &dp)) in tc.iter().zip(&d).enumerate() {
                    let (q, dq) = (tc[(i + 1) % 4], d[(i + 1) % 4]);
                    if dp >= 0.0 {
                        poly.push(p);
                    }
                    if (dp >= 0.0) != (dq >= 0.0) {
                        let k = dp / (dp - dq);
                        poly.push((p.0 + (q.0 - p.0) * k, p.1 + (q.1 - p.1) * k));
                    }
                }
                poly
            }
            None => tc.to_vec(),
        };
        if poly.is_empty() {
            return Ok(None);
        }
        let fp: Vec<(f64, f64)> = poly.iter().map(|&(x, y)| inv.apply(x, y)).collect();
        let fx0 = (fp.iter().map(|c| c.0).fold(f64::MAX, f64::min).floor() as i32).saturating_sub(3);
        let fy0 = (fp.iter().map(|c| c.1).fold(f64::MAX, f64::min).floor() as i32).saturating_sub(3);
        let fx1 = (fp.iter().map(|c| c.0).fold(f64::MIN, f64::max).ceil() as i32).saturating_add(3);
        let fy1 = (fp.iter().map(|c| c.1).fold(f64::MIN, f64::max).ceil() as i32).saturating_add(3);
        // Without a one-signed forward w, a tile straddling the inverse horizon has no bounded footprint.
        let straddles = side.is_none() && wc.iter().any(|w| *w <= 0.0) && wc.iter().any(|w| *w >= 0.0);
        let foot = if straddles { src_rect } else { Rect::new(fx0, fy0, fx1, fy1).intersect(&src_rect) };
        if foot.is_empty() || !src_ref.has_tiles_in(foot) {
            return Ok(None);
        }
        // Premultiplied source window.
        let mut px = src_ref.try_read_region(foot)?;
        for p in px.chunks_exact_mut(n) {
            let al = p[a];
            for v in &mut p[..a] {
                *v *= al;
            }
        }
        let fw = foot.width() as usize;
        let at = |x: i32, y: i32, c: usize| -> f64 {
            if x < foot.x0 || y < foot.y0 || x >= foot.x1 || y >= foot.y1 {
                0.0
            } else {
                px[((y - foot.y0) as usize * fw + (x - foot.x0) as usize) * n + c] as f64
            }
        };
        // Samples past the source rect repeat its edge, so a stretched edge row or column keeps its
        // colour and opacity (the "pixel stretch", #3773); the outline is antialiased by coverage
        // instead. `foot_u`/`foot_v` are a destination pixel's extent in source pixels.
        let clamped = |x: i32, y: i32, c: usize| at(x.clamp(src_rect.x0, src_rect.x1 - 1), y.clamp(src_rect.y0, src_rect.y1 - 1), c);
        let (mx, my) = ((t.x0 + t.x1) as f64 / 2.0, (t.y0 + t.y1) as f64 / 2.0);
        let (o, dx, dy) = (inv.apply(mx, my), inv.apply(mx + 1.0, my), inv.apply(mx, my + 1.0));
        let extent = |e: f64| if e.is_finite() && e > 1e-12 { e } else { 1.0 };
        let foot_u = extent((dx.0 - o.0).abs() + (dy.0 - o.0).abs());
        let foot_v = extent((dx.1 - o.1).abs() + (dy.1 - o.1).abs());
        let w = t.width() as usize;
        let len = w.checked_mul(t.height() as usize).and_then(|v| v.checked_mul(n)).ok_or(photocraft_raster::AllocationError::SizeOverflow)?;
        let bytes = len.checked_mul(std::mem::size_of::<f32>()).ok_or(photocraft_raster::AllocationError::SizeOverflow)?;
        let mut outp = Vec::new();
        outp.try_reserve_exact(len).map_err(|_| photocraft_raster::AllocationError::NotEnoughMemory { bytes })?;
        outp.resize(len, 0.0f32);
        let mut any = false;
        let mut acc = [0.0f64; 8];
        for y in t.y0..t.y1 {
            for x in t.x0..t.x1 {
                if front.is_some_and(|s| s * inv.w(x as f64 + 0.5, y as f64 + 0.5) <= 0.0) {
                    continue;
                }
                let (u, v) = inv.apply(x as f64 + 0.5, y as f64 + 0.5);
                let (u, v) = (u - 0.5, v - 0.5);
                if u < src_rect.x0 as f64 - 1.0 || v < src_rect.y0 as f64 - 1.0 || u > src_rect.x1 as f64 || v > src_rect.y1 as f64 {
                    continue;
                }
                acc[..n].fill(0.0);
                match interp {
                    Interp::Nearest => {
                        let (ix, iy) = ((u + 0.5).floor() as i32, (v + 0.5).floor() as i32);
                        for (c, s) in acc.iter_mut().enumerate().take(n) {
                            *s = at(ix, iy, c);
                        }
                    }
                    Interp::Bilinear => {
                        let (ix, iy) = (u.floor() as i32, v.floor() as i32);
                        let (fx, fy) = (u - ix as f64, v - iy as f64);
                        for (dy, wy) in [(0, 1.0 - fy), (1, fy)] {
                            for (dx, wx) in [(0, 1.0 - fx), (1, fx)] {
                                for (c, s) in acc.iter_mut().enumerate().take(n) {
                                    *s += clamped(ix + dx, iy + dy, c) * wx * wy;
                                }
                            }
                        }
                    }
                    Interp::Bicubic => {
                        let (ix, iy) = (u.floor() as i32, v.floor() as i32);
                        let (wx, wy) = (bicubic(u - ix as f64), bicubic(v - iy as f64));
                        for (j, wyj) in wy.iter().enumerate() {
                            for (i, wxi) in wx.iter().enumerate() {
                                let k = wxi * wyj;
                                for (c, s) in acc.iter_mut().enumerate().take(n) {
                                    *s += clamped(ix - 1 + i as i32, iy - 1 + j as i32, c) * k;
                                }
                            }
                        }
                    }
                }
                if interp != Interp::Nearest {
                    // How much of this pixel the warped source rect covers, per axis.
                    let (uc, vc) = (u + 0.5, v + 0.5);
                    let cu = ((uc - src_rect.x0 as f64).min(src_rect.x1 as f64 - uc) / foot_u + 0.5).clamp(0.0, 1.0);
                    let cv = ((vc - src_rect.y0 as f64).min(src_rect.y1 as f64 - vc) / foot_v + 0.5).clamp(0.0, 1.0);
                    for s in acc.iter_mut().take(n) {
                        *s *= cu * cv;
                    }
                }
                let al = acc[a].clamp(0.0, 1.0);
                if al <= 0.0 {
                    continue;
                }
                any = true;
                let o = ((y - t.y0) as usize * w + (x - t.x0) as usize) * n;
                for c in 0..a {
                    // Integer storage clips on write; float colours retain their signed/HDR range.
                    outp[o + c] = (acc[c] / al) as f32;
                }
                outp[o + a] = al as f32;
            }
        }
        Ok(any.then_some((*t, outp)))
    };
    let result_bytes = tiles.len().checked_mul(std::mem::size_of::<(Rect, Vec<f32>)>()).ok_or(photocraft_raster::AllocationError::SizeOverflow)?;
    let mut done = Vec::new();
    done.try_reserve_exact(tiles.len()).map_err(|_| photocraft_raster::AllocationError::NotEnoughMemory { bytes: result_bytes })?;
    let done = std::sync::Mutex::new(done);
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        tiles.into_par_iter().try_for_each(|tile| -> Result<(), photocraft_raster::AllocationError> {
            if let Some(result) = work(&tile)? {
                done.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(result);
            }
            Ok(())
        })?;
    }
    #[cfg(target_arch = "wasm32")]
    for tile in tiles {
        if let Some(result) = work(&tile)? {
            done.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(result);
        }
    }
    let done = done.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner);
    if ctl.cancelled() {
        return Ok(out);
    }
    for (r, v) in done {
        out.try_write_region(r, &v)?;
    }
    out.prune();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_interruptible_warp_reports_progress_and_stops_when_cancelled() {
        let mut s = Surface::new(PixelFormat::RGBA8);
        s.fill_rect(Rect::new(0, 0, 600, 400), &[0.2, 0.4, 0.6, 1.0]);
        let r = s.content_bounds();
        let h = Homography::rect_to_quad([0.0, 0.0, 600.0, 400.0], [[10.0, 0.0], [590.0, 20.0], [600.0, 400.0], [0.0, 380.0]]).unwrap();
        let reported = std::sync::Mutex::new(0.0f32);
        let progress = |p: f32| {
            let mut m = reported.lock().unwrap();
            *m = m.max(p);
        };
        let never = || false;
        let out = warp_surface_with(&s, r, &h, Interp::Bicubic, &Interrupt::new(&never, &progress)).unwrap().expect("not cancelled");
        assert_eq!(out.read_region(r), warp_surface(&s, r, &h, Interp::Bicubic).unwrap().read_region(r));
        assert!(*reported.lock().unwrap() > 0.9);
        let always = || true;
        assert!(warp_surface_with(&s, r, &h, Interp::Bicubic, &Interrupt::cancel_only(&always)).unwrap().is_none());
    }

    /// Perspective Crop's map lands each quad corner exactly on its rectangle corner; degenerate,
    /// self-crossing, counter-clockwise and non-finite quads have none.
    #[test]
    fn quad_to_rect_maps_corners_and_refuses_bad_quads() {
        let q = [[30.0, 12.0], [140.0, 25.0], [128.0, 110.0], [14.0, 95.0]];
        let h = quad_to_rect(&q, 100.0, 80.0).unwrap();
        for (c, want) in q.iter().zip([[0.0, 0.0], [100.0, 0.0], [100.0, 80.0], [0.0, 80.0]]) {
            let (x, y) = h.apply(c[0], c[1]);
            assert!((x - want[0]).abs() < 1e-9 && (y - want[1]).abs() < 1e-9, "{c:?} -> ({x}, {y})");
        }
        let (w, ht) = quad_size(&q);
        assert!((w - 112.874).abs() < 1e-3 && (ht - 85.185).abs() < 1e-3, "{w} x {ht}");
        for bad in [
            [[0.0, 0.0], [50.0, 0.0], [100.0, 0.0], [0.0, 50.0]],
            [[0.0, 0.0], [50.0, 50.0], [50.0, 0.0], [0.0, 50.0]],
            [[0.0, 0.0], [0.0, 50.0], [50.0, 50.0], [50.0, 0.0]],
            [[0.0, 0.0], [f64::NAN, 0.0], [50.0, 50.0], [0.0, 50.0]],
            [[0.0, 0.0]; 4],
        ] {
            assert!(quad_to_rect(&bad, 10.0, 10.0).is_none(), "{bad:?}");
        }
        assert!(quad_to_rect(&q, 0.0, 10.0).is_none());
    }

    fn rgba() -> Surface {
        Surface::new(PixelFormat::RGBA8)
    }

    /// The "pixel stretch" (#3773): a 1 px column scaled 100x stays solid across the stretch, as in
    /// Photoshop. Samples past the source edge repeat the edge, and only the outline is antialiased.
    #[test]
    fn stretched_single_column_stays_opaque() {
        let mut s = rgba();
        s.fill_rect(Rect::new(0, 0, 1, 4), &[0.0, 1.0, 0.0, 1.0]);
        for interp in [Interp::Bicubic, Interp::Bilinear] {
            let h = Homography::rect_to_quad([0.0, 0.0, 1.0, 4.0], [[0.0, 0.0], [100.0, 0.0], [100.0, 4.0], [0.0, 4.0]]).unwrap();
            let w = warp_surface(&s, Rect::new(0, 0, 1, 4), &h, interp).unwrap();
            assert_eq!(w.content_bounds(), Rect::new(0, 0, 100, 4), "{interp:?}");
            for x in [0, 1, 25, 50, 75, 99] {
                for y in 0..4 {
                    assert!((w.sample_channel(x, y, 3) - 1.0).abs() < 1e-6, "{interp:?} alpha at ({x}, {y}) = {}", w.sample_channel(x, y, 3));
                }
            }
        }
    }

    /// Photoshop 27.11's Free Transform, Bicubic, 200 % about the centre of a 64 px image whose left
    /// half is grey 64 and right half grey 192: the row across the edge, columns 26..36.
    #[test]
    fn bicubic_upscale_matches_photoshop() {
        let mut src = rgba();
        let g = |v: f32| v / 255.0;
        src.fill_rect(Rect::new(0, 0, 32, 64), &[g(64.0), g(64.0), g(64.0), 1.0]);
        src.fill_rect(Rect::new(32, 0, 64, 64), &[g(192.0), g(192.0), g(192.0), 1.0]);
        let q = [[-32.0, -32.0], [96.0, -32.0], [96.0, 96.0], [-32.0, 96.0]];
        let h = Homography::rect_to_quad([0.0, 0.0, 64.0, 64.0], q).unwrap();
        let out = warp_surface(&src, Rect::new(0, 0, 64, 64), &h, Interp::Bicubic).unwrap();
        let photoshop = [64.0, 64.0, 64.0, 60.0, 51.0, 93.0, 163.0, 206.0, 197.0, 192.0];
        for (i, want) in photoshop.iter().enumerate() {
            let got = out.pixel(26 + i as i32, 50)[0] * 255.0;
            assert!((got - want).abs() <= 1.5, "column {}: {got} vs {want}", 26 + i);
        }
    }

    #[test]
    fn homography_maps_rect_corners_and_inverts() {
        let r = [10.0, 20.0, 110.0, 70.0];
        let q = [[0.0, 0.0], [200.0, 10.0], [180.0, 150.0], [-20.0, 120.0]];
        let h = Homography::rect_to_quad(r, q).unwrap();
        for (p, e) in [(10.0, 20.0), (110.0, 20.0), (110.0, 70.0), (10.0, 70.0)].iter().zip(q) {
            let (x, y) = h.apply(p.0, p.1);
            assert!((x - e[0]).abs() < 1e-9 && (y - e[1]).abs() < 1e-9, "{p:?} → {x},{y} vs {e:?}");
        }
        let inv = h.inverse().unwrap();
        let (x, y) = inv.apply(90.0, 60.0);
        let (bx, by) = h.apply(x, y);
        assert!((bx - 90.0).abs() < 1e-9 && (by - 60.0).abs() < 1e-9);
        assert!(Homography::rect_to_quad(r, [[0.0, 0.0]; 4]).is_none() || Homography::rect_to_quad(r, [[0.0, 0.0]; 4]).unwrap().inverse().is_none());
    }

    #[test]
    fn steep_distort_keeps_every_pixel_inside_the_quad() {
        // Corner C is almost flat, so the horizon of the inverse map crosses the 256-px tiles that
        // also hold quad pixels; those tiles must still read every source pixel that lands in them.
        let mut s = rgba();
        s.fill_rect(Rect::new(0, 0, 100, 100), &[1.0, 0.0, 0.0, 1.0]);
        let r = s.content_bounds();
        let h = Homography::rect_to_quad([0.0, 0.0, 100.0, 100.0], [[0.0, 0.0], [300.0, 0.0], [155.0, 155.0], [0.0, 300.0]]).unwrap();
        let w = warp_surface(&s, r, &h, Interp::Nearest).unwrap();
        for (x, y) in [(20, 20), (252, 14), (252, 42), (14, 252), (42, 252)] {
            let a = w.read_region(Rect::new(x, y, x + 1, y + 1))[3];
            assert!(a > 0.99, "({x},{y}) inside the quad is transparent: {a}");
        }
        for (x, y) in [(200, 200), (280, 100), (100, 280)] {
            assert_eq!(w.read_region(Rect::new(x, y, x + 1, y + 1))[3], 0.0, "({x},{y}) outside the quad is painted");
        }
    }

    /// #4863: a Distort with the bottom corners pulled in and one dragged past the horizon (a
    /// concave quad) leaves part of the source behind the horizon. That part maps to the far side,
    /// where it showed as a mirrored copy wrapped round infinity; only the front side is drawn.
    #[test]
    fn a_distort_past_the_horizon_draws_only_its_front_side() {
        let mut s = rgba();
        s.fill_rect(Rect::new(0, 0, 100, 100), &[0.0, 0.5, 1.0, 1.0]);
        let r = s.content_bounds();
        let h = Homography::rect_to_quad([0.0, 0.0, 100.0, 100.0], [[0.0, 0.0], [300.0, 0.0], [60.0, 60.0], [0.0, 300.0]]).unwrap();
        let corners = [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)].map(|(x, y)| h.w(x, y));
        assert!(corners.iter().any(|w| *w > 0.0) && corners.iter().any(|w| *w < 0.0), "the source straddles the horizon: {corners:?}");
        let front = h.front_side([0.0, 0.0, 100.0, 100.0]);
        let inv = h.inverse().unwrap();
        let clip = Rect::new(-400, -400, 700, 700);
        for interp in [Interp::Nearest, Interp::Bilinear] {
            let w = warp_surface_clipped(&s, r, &h, interp, clip).unwrap();
            let px = w.read_region(clip);
            let (mut drawn, mut behind) = (0, 0);
            for y in clip.y0..clip.y1 {
                for x in clip.x0..clip.x1 {
                    let i = ((y - clip.y0) * clip.width() as i32 + (x - clip.x0)) as usize * 4;
                    if px[i + 3] > 0.0 {
                        drawn += 1;
                        // The source point this pixel samples is on the far side of the horizon.
                        if front * inv.w(x as f64 + 0.5, y as f64 + 0.5) <= 0.0 {
                            behind += 1;
                        }
                    }
                }
            }
            assert!(drawn > 1000, "{interp:?}: the front side is drawn ({drawn} px)");
            assert_eq!(behind, 0, "{interp:?}: {behind} pixels come from behind the horizon");
        }
        // A quad wholly in front of the horizon is drawn exactly as before.
        let ok = Homography::rect_to_quad([0.0, 0.0, 100.0, 100.0], [[0.0, 0.0], [300.0, 0.0], [155.0, 155.0], [0.0, 300.0]]).unwrap();
        assert_eq!(ok.front_side([0.0, 0.0, 100.0, 100.0]), ok.w(50.0, 50.0).signum());
    }

    #[test]
    fn identity_and_translation_are_exact() {
        let mut s = rgba();
        s.fill_rect(Rect::new(4, 4, 20, 12), &[1.0, 0.5, 0.0, 1.0]);
        let r = s.content_bounds();
        for interp in [Interp::Nearest, Interp::Bilinear, Interp::Bicubic] {
            let h = Homography([1.0, 0.0, 7.0, 0.0, 1.0, -3.0, 0.0, 0.0, 1.0]);
            let o = warp_surface(&s, r, &h, interp).unwrap();
            assert_eq!(o.content_bounds(), Rect::new(11, 1, 27, 9), "{interp:?}");
            assert_eq!(o.pixel(15, 5), vec![1.0, 128.0 / 255.0, 0.0, 1.0], "{interp:?}");
        }
    }

    #[test]
    fn translation_preserves_colour_range_at_every_depth() {
        use photocraft_color::{ColorMode, SampleType};

        let h = Homography([1.0, 0.0, 7.0, 0.0, 1.0, -3.0, 0.0, 0.0, 1.0]);
        for sample in SampleType::ALL {
            for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
                for alpha in [false, true] {
                    let mut s = Surface::new(PixelFormat::new(mode, sample, alpha));
                    let mut pixel = if mode == ColorMode::Rgb { vec![2.0, -0.25, 0.5] } else { vec![2.0] };
                    if alpha {
                        pixel.push(0.5);
                    }
                    s.fill_rect(Rect::new(4, 4, 8, 8), &pixel);
                    let mut expected = s.pixel(5, 5);
                    if !alpha {
                        expected.push(1.0);
                    }
                    for interp in [Interp::Nearest, Interp::Bilinear, Interp::Bicubic] {
                        let out = warp_surface(&s, s.content_bounds(), &h, interp).unwrap();
                        assert_eq!(out.pixel(12, 2), expected, "{sample:?}/{mode:?}/alpha={alpha}/{interp:?}");
                        assert_eq!(out.pixel(0, 0).last(), Some(&0.0));
                    }
                }
            }
        }
    }

    #[test]
    fn bilinear_translation_interpolates_signed_hdr_colours() {
        let mut s = Surface::new(PixelFormat::RGBA32F);
        s.write_pixel(0, 0, &[2.0, -0.5, 0.25, 0.5]);
        s.write_pixel(1, 0, &[4.0, -0.25, 0.75, 0.5]);
        let h = Homography([1.0, 0.0, 0.5, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
        let out = warp_surface(&s, s.content_bounds(), &h, Interp::Bilinear).unwrap();
        assert_eq!(out.pixel(1, 0), vec![3.0, -0.375, 0.5, 0.5]);
    }

    #[test]
    fn scale_doubles_size_and_rotation_keeps_area() {
        let mut s = rgba();
        s.fill_rect(Rect::new(0, 0, 40, 20), &[0.2, 0.4, 0.6, 1.0]);
        let r = s.content_bounds();
        let up = warp_surface(
            &s,
            r,
            &Homography::rect_to_quad([0.0, 0.0, 40.0, 20.0], [[0.0, 0.0], [80.0, 0.0], [80.0, 40.0], [0.0, 40.0]]).unwrap(),
            Interp::Bicubic,
        )
        .unwrap();
        let b = up.content_bounds();
        assert!(b.width() >= 80 && b.width() <= 82 && b.height() >= 40 && b.height() <= 42, "{b:?}");
        // 90° rotation about (20, 10): a 40×20 box becomes 20×40.
        let q = [[30.0, -10.0], [30.0, 30.0], [10.0, 30.0], [10.0, -10.0]];
        let rot = warp_surface(&s, r, &Homography::rect_to_quad([0.0, 0.0, 40.0, 20.0], q).unwrap(), Interp::Bilinear).unwrap();
        let b = rot.content_bounds();
        assert!(b.width().abs_diff(20) <= 2 && b.height().abs_diff(40) <= 2, "{b:?}");
        let p = rot.pixel(20, 10);
        assert!((p[0] - 0.2).abs() < 0.01 && p[3] > 0.99, "{p:?}");
    }

    #[test]
    fn large_downscale_is_prefiltered() {
        // A 1px checkerboard shrunk 8× must come out ~50% grey, not aliased black/white.
        let mut s = rgba();
        for y in 0..64 {
            for x in 0..64 {
                let v = ((x + y) % 2) as f32;
                s.fill_rect(Rect::new(x, y, x + 1, y + 1), &[v, v, v, 1.0]);
            }
        }
        let h = Homography::rect_to_quad([0.0, 0.0, 64.0, 64.0], [[0.0, 0.0], [8.0, 0.0], [8.0, 8.0], [0.0, 8.0]]).unwrap();
        let o = warp_surface(&s, s.content_bounds(), &h, Interp::Bicubic).unwrap();
        let p = o.pixel(4, 4);
        assert!((p[0] - 0.5).abs() < 0.1, "{p:?}");
    }

    /// 64 × 64 opaque white lines on black, `width` px wide every `period` px: columns when
    /// `vertical`, rows otherwise.
    fn lines(sample: photocraft_color::SampleType, vertical: bool, width: i32, period: i32) -> Surface {
        let mut s = Surface::new(PixelFormat::new(photocraft_color::ColorMode::Rgb, sample, true));
        s.fill_rect(Rect::new(0, 0, 64, 64), &[0.0, 0.0, 0.0, 1.0]);
        for i in (0..64).filter(|i| i % period >= period - width) {
            s.fill_rect(if vertical { Rect::new(i, 0, i + 1, 64) } else { Rect::new(0, i, 64, i + 1) }, &[1.0, 1.0, 1.0, 1.0]);
        }
        s
    }

    /// #4864: Free Transform to 100 % × 25 % shrinks only y, so 1 px columns keep their full
    /// contrast along x. The Bicubic prefilter used one factor for both axes, √|det J| = 50 %, which
    /// averaged the columns to flat grey and stretched them back. The shrunk axis is still
    /// prefiltered: rows 3 px apart come out an even third grey instead of aliasing into bands.
    #[test]
    fn a_one_axis_reduction_keeps_the_other_axis_sharp() {
        let h = Homography::rect_to_quad([0.0, 0.0, 64.0, 64.0], [[0.0, 0.0], [64.0, 0.0], [64.0, 16.0], [0.0, 16.0]]).unwrap();
        for sample in photocraft_color::SampleType::ALL {
            let s = lines(sample, true, 1, 2);
            let o = warp_surface(&s, s.content_bounds(), &h, Interp::Bicubic).unwrap();
            for x in 4..60 {
                let (a, b) = (o.pixel(x, 8)[0], o.pixel(x + 1, 8)[0]);
                assert!((a - b).abs() > 0.9, "{sample:?}: columns {x} and {}: {a} vs {b}", x + 1);
            }
            let s = lines(sample, false, 1, 3);
            let o = warp_surface(&s, s.content_bounds(), &h, Interp::Bicubic).unwrap();
            for y in 2..14 {
                let v = o.pixel(32, y)[0];
                assert!((v - 1.0 / 3.0).abs() < 0.1, "{sample:?}: row {y}: {v}");
            }
        }
    }

    /// #4864: both axes shrink, by different amounts (87.5 % × 25 %). Each needs its own filter:
    /// one designed for 25 % blurs 4 px bars that only shrink to 87.5 % along x.
    #[test]
    fn unequal_reductions_filter_each_axis_for_its_own_scale() {
        let h = Homography::rect_to_quad([0.0, 0.0, 64.0, 64.0], [[0.0, 0.0], [56.0, 0.0], [56.0, 16.0], [0.0, 16.0]]).unwrap();
        for sample in photocraft_color::SampleType::ALL {
            let s = lines(sample, true, 4, 8);
            let o = warp_surface(&s, s.content_bounds(), &h, Interp::Bicubic).unwrap();
            let steepest = (4..50).map(|x| (o.pixel(x, 8)[0] - o.pixel(x + 1, 8)[0]).abs()).fold(0.0f32, f32::max);
            assert!(steepest > 0.8, "{sample:?}: bar edges blurred along x, steepest step {steepest}");
        }
    }

    /// #4864: the prefilter's factors, per axis. An affine map gives the same pair with or without
    /// a clip, so a clipped warp matches the full one; a perspective Distort keeps the one factor at
    /// its centre, as before; a skew reduces nothing, and a squash along a slant keeps the one
    /// factor; a map past the horizon stays in range; and a map that isn't finite reduces nothing.
    #[test]
    fn prefilter_scales_are_per_axis_and_finite() {
        let r = Rect::new(0, 0, 64, 64);
        let quad = |q| Homography::rect_to_quad([0.0, 0.0, 64.0, 64.0], q).unwrap();
        assert_eq!(prefilter_scales(&quad([[0.0, 0.0], [64.0, 0.0], [64.0, 16.0], [0.0, 16.0]]), r, None), (1.0, 0.25));
        let eighth = Homography([0.125, 0.0, 24.0, 0.0, 0.125, 30.0, 0.0, 0.0, 1.0]);
        assert_eq!(prefilter_scales(&eighth, r, None), (0.125, 0.125));
        assert_eq!(prefilter_scales(&eighth, r, Some(Rect::new(10, 10, 52, 52))), (0.125, 0.125));
        let trapezoid = quad([[0.0, 0.0], [64.0, 0.0], [48.0, 24.0], [16.0, 24.0]]);
        let one = trapezoid.local_scale(32.0, 32.0);
        assert!(one > 0.4 && one < 0.5, "{one}");
        assert_eq!(prefilter_scales(&trapezoid, r, None), (one, one));
        assert_eq!(prefilter_scales(&quad([[0.0, 0.0], [64.0, 0.0], [256.0, 64.0], [192.0, 64.0]]), r, None), (1.0, 1.0));
        // Skewed by 2 and flattened to 5 %, and a rhombus squashed to 25 % along its diagonal: both
        // keep the one factor, √|det J|. A 4× area rule left the skew's warp 5 % along its slant (the
        // one factor 10 %), and the column lengths left the rhombus's 34 % (the one factor 50 %).
        let (fx, fy) = prefilter_scales(&quad([[0.0, 0.0], [64.0, 0.0], [192.0, 3.2], [128.0, 3.2]]), r, None);
        assert!(fx == fy && (fx * fy - 0.05).abs() < 1e-9, "{fx} x {fy}");
        let h = std::f64::consts::FRAC_1_SQRT_2 * 64.0;
        let (fx, fy) = prefilter_scales(&quad([[0.0, 0.0], [h, h / 4.0], [0.0, h / 2.0], [-h, h / 4.0]]), r, None);
        assert!(fx == fy && (fx - 0.5).abs() < 1e-9, "{fx} x {fy}");
        let past = Homography::rect_to_quad([0.0, 0.0, 100.0, 100.0], [[0.0, 0.0], [300.0, 0.0], [60.0, 60.0], [0.0, 300.0]]).unwrap();
        for clip in [None, Some(Rect::new(-400, -400, 700, 700))] {
            let (fx, fy) = prefilter_scales(&past, Rect::new(0, 0, 100, 100), clip);
            assert!(fx > 0.0 && fx <= 1.0 && fy > 0.0 && fy <= 1.0, "{clip:?}: {fx} x {fy}");
        }
        for h in [Homography([f64::NAN; 9]), Homography([f64::INFINITY, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0])] {
            assert_eq!(prefilter_scales(&h, r, None), (1.0, 1.0), "{h:?}");
            assert_eq!(prefilter_scales(&h, r, Some(r)), (1.0, 1.0), "{h:?}");
        }
    }

    #[test]
    fn surfaces_without_alpha_gain_it() {
        let mut s = Surface::new(PixelFormat::new(photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8, false));
        s.fill_rect(Rect::new(0, 0, 10, 10), &[1.0, 0.0, 0.0]);
        let h = Homography([1.0, 0.0, 5.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
        let o = warp_surface(&s, Rect::new(0, 0, 10, 10), &h, Interp::Nearest).unwrap();
        assert!(o.format().alpha);
        assert_eq!(o.pixel(2, 2)[3], 0.0);
        assert_eq!(o.pixel(7, 2), vec![1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn homography_is_identity() {
        assert!(Homography::IDENTITY.is_identity());
        assert!(Homography([2.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 2.0]).is_identity());
        assert!(!Homography([1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]).is_identity());
        assert!(!Homography([1.0, 0.1, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]).is_identity());
    }

    /// A uniform 50 % scale turned through `rect_to_quad`, as Free Transform builds it, keeps the
    /// one factor exactly. Its column lengths differ from the factor in the last bits at most
    /// angles, which picked the order of the two passes and changed the output by a level.
    #[test]
    fn a_turned_uniform_scale_keeps_the_one_factor() {
        let r = Rect::new(0, 0, 6000, 4000);
        for tenth in (5..3600).step_by(37) {
            let (s, c) = (f64::from(tenth) / 10.0).to_radians().sin_cos();
            let at = |x: f64, y: f64| {
                let (dx, dy) = (x - 3000.0, y - 2000.0);
                [3000.0 + 0.5 * (c * dx - s * dy) + 17.3, 2000.0 + 0.5 * (s * dx + c * dy) - 4.1]
            };
            let h = Homography::rect_to_quad([0.0, 0.0, 6000.0, 4000.0], [at(0.0, 0.0), at(6000.0, 0.0), at(6000.0, 4000.0), at(0.0, 4000.0)]).unwrap();
            let one = h.local_scale(3000.0, 2000.0);
            assert_eq!(prefilter_scales(&h, r, None), (one, one), "{tenth} tenths of a degree");
        }
    }

    /// #4864: a rhombus squashed to 25 % along its diagonal (rotated 45°, then squashed vertically).
    /// Both columns of the map are 73 % long, so prefiltering each axis by its column left the warp
    /// a 34 % reduction along the slant, and 2 px diagonal stripes came out as moiré (standard
    /// deviation 0.27 around the 0.5 grey they average to, at 8 bits); the one factor, 50 %, gives
    /// 0.06, and so does this.
    #[test]
    fn a_squash_along_a_slant_does_not_alias() {
        let k = std::f64::consts::FRAC_1_SQRT_2 * 128.0;
        let h = Homography::rect_to_quad([0.0, 0.0, 128.0, 128.0], [[0.0, 0.0], [k, k / 4.0], [0.0, k / 2.0], [-k, k / 4.0]]).unwrap();
        for sample in photocraft_color::SampleType::ALL {
            let mut s = Surface::new(PixelFormat::new(photocraft_color::ColorMode::Rgb, sample, true));
            s.fill_rect(Rect::new(0, 0, 128, 128), &[0.0, 0.0, 0.0, 1.0]);
            for y in 0..128 {
                for x in (0..128).filter(|x| (x + y) % 4 < 2) {
                    s.fill_rect(Rect::new(x, y, x + 1, y + 1), &[1.0, 1.0, 1.0, 1.0]);
                }
            }
            let o = warp_surface(&s, s.content_bounds(), &h, Interp::Bicubic).unwrap();
            let b = o.content_bounds();
            let opaque = |x: i32, y: i32| (-1..=1).all(|j| (-1..=1).all(|i| o.pixel(x + i, y + j)[3] > 0.999));
            let v: Vec<f64> =
                (b.y0..b.y1).flat_map(|y| (b.x0..b.x1).map(move |x| (x, y))).filter(|&(x, y)| opaque(x, y)).map(|(x, y)| f64::from(o.pixel(x, y)[0])).collect();
            let mean = v.iter().sum::<f64>() / v.len() as f64;
            let sd = (v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / v.len() as f64).sqrt();
            assert!(v.len() > 1000 && (mean - 0.5).abs() < 0.02 && sd < 0.1, "{sample:?}: {} pixels, mean {mean}, sd {sd}", v.len());
        }
    }

    /// #4864: whatever the map, rect or clip (zero, NaN, huge, tiny, on or past the horizon,
    /// collapsed to a line), each factor stays within [1/64, 1].
    #[test]
    fn prefilter_scales_stay_in_range_for_hostile_maps() {
        let big = Rect::new(i32::MIN / 2, i32::MIN / 2, i32::MAX / 2, i32::MAX / 2);
        let rects = [Rect::new(0, 0, 64, 64), Rect::new(-5, -7, -4, -6), big, Rect::new(0, 0, 1, 100_000), Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX)];
        let maps = [
            Homography([0.0; 9]),
            Homography([f64::NAN; 9]),
            Homography([1e300, 0.0, 0.0, 0.0, 1e300, 0.0, 0.0, 0.0, 1.0]),
            Homography([1e-300, 0.0, 0.0, 0.0, 1e-300, 0.0, 0.0, 0.0, 1.0]),
            Homography([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, -1.0]),
            Homography([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0]),
            Homography([1.0, 0.0, 0.0, 0.0, 1e-9, 0.0, 0.0, 0.0, 1.0]),
            Homography([1.0, 1e9, 0.0, 0.0, 1e-9, 0.0, 0.0, 0.0, 1.0]),
            Homography([f64::MAX, f64::MAX, 0.0, f64::MAX, f64::MAX, 0.0, 0.0, 0.0, 1.0]),
            Homography([-0.25, 0.0, 0.0, 0.0, -0.5, 0.0, 0.0, 0.0, 1.0]),
            Homography([0.5, 0.0, 0.0, 0.0, 0.5, 0.0, 1e-3, 1e-3, 1.0]),
            Homography([0.5, 0.0, 0.0, 0.0, 0.5, 0.0, -1e-3, -1e-3, 1.0]),
            Homography([f64::MIN_POSITIVE, 0.0, 0.0, 0.0, f64::MIN_POSITIVE, 0.0, 0.0, 0.0, 1.0]),
            Homography([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, f64::MIN_POSITIVE]),
        ];
        let clips =
            [None, Some(Rect::new(0, 0, 64, 64)), Some(big), Some(Rect::new(1000, 1000, 1001, 1001)), Some(Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX))];
        for r in rects {
            for h in maps {
                for c in clips {
                    let (fx, fy) = prefilter_scales(&h, r, c);
                    assert!((1.0 / 64.0..=1.0).contains(&fx) && (1.0 / 64.0..=1.0).contains(&fy), "{h:?} {r:?} {c:?}: {fx} x {fy}");
                }
            }
        }
    }

    /// #4864: a Bicubic warp by a degenerate map, or one that shrinks a single axis far below the
    /// 1/64 the prefilter stops at, returns finite pixels instead of panicking (32-bit, where a NaN
    /// would show).
    #[test]
    fn bicubic_warp_survives_degenerate_and_extreme_maps() {
        let mut s = Surface::new(PixelFormat::new(photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::F32, true));
        s.fill_rect(Rect::new(0, 0, 40, 30), &[0.3, 0.6, 0.9, 1.0]);
        let maps = [
            Homography([0.0; 9]),
            Homography([f64::NAN; 9]),
            Homography([1e-300, 0.0, 0.0, 0.0, 1e-300, 0.0, 0.0, 0.0, 1.0]),
            Homography([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, -1.0]),
            Homography([1.0, 0.0, 0.0, 0.0, 1e-9, 0.0, 0.0, 0.0, 1.0]),
            Homography([1e-9, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]),
            Homography([1.0, 0.0, 0.0, 0.0, 0.001, 0.0, 0.0, 0.0, 1.0]),
            Homography([0.001, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]),
            Homography([1.0, 3.0, 0.0, 0.0, 0.01, 0.0, 0.0, 0.0, 1.0]),
            Homography([-0.25, 0.0, 0.0, 0.0, -0.5, 0.0, 0.0, 0.0, 1.0]),
            Homography([0.5, 0.0, 0.0, 0.0, 0.5, 0.0, 0.05, 0.0, 1.0]),
            Homography([0.5, 0.0, 0.0, 0.0, 0.5, 0.0, -0.05, -0.05, 1.0]),
        ];
        for h in maps {
            for clip in [None, Some(Rect::new(-50, -50, 50, 50))] {
                let o = match clip {
                    None => warp_surface(&s, s.content_bounds(), &h, Interp::Bicubic),
                    Some(c) => warp_surface_clipped(&s, s.content_bounds(), &h, Interp::Bicubic, c),
                };
                if let Ok(o) = o {
                    let b = o.content_bounds();
                    if !b.is_empty() {
                        assert!(o.read_region(b).iter().all(|v| v.is_finite()), "{h:?}");
                    }
                }
            }
        }
    }

    /// #4864: the per-axis prefilter keeps a flat layer flat in Grayscale, RGB, CMYK and Lab, at
    /// every depth, with or without alpha (2 to 5 channels through each pass).
    #[test]
    fn the_per_axis_prefilter_keeps_flat_colour_in_every_mode() {
        use photocraft_color::{ColorMode, SampleType};
        let quads = [
            ([[0.0, 0.0], [64.0, 0.0], [64.0, 16.0], [0.0, 16.0]], (32, 8)),
            ([[0.0, 0.0], [56.0, 0.0], [56.0, 16.0], [0.0, 16.0]], (28, 8)),
            ([[0.0, 0.0], [64.0, 0.0], [48.0, 24.0], [16.0, 24.0]], (32, 12)),
            ([[0.0, 0.0], [8.0, 0.0], [8.0, 64.0], [0.0, 64.0]], (4, 32)),
        ];
        for mode in [ColorMode::Grayscale, ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab] {
            for sample in SampleType::ALL {
                for alpha in [false, true] {
                    let fmt = PixelFormat::new(mode, sample, alpha);
                    let n = fmt.channels();
                    let col: Vec<f32> = (0..n).map(|c| if alpha && c == n - 1 { 1.0 } else { 0.2 + 0.15 * c as f32 }).collect();
                    let mut s = Surface::new(fmt);
                    s.fill_rect(Rect::new(0, 0, 64, 64), &col);
                    for (q, (x, y)) in quads {
                        let h = Homography::rect_to_quad([0.0, 0.0, 64.0, 64.0], q).unwrap();
                        let o = warp_surface(&s, s.content_bounds(), &h, Interp::Bicubic).unwrap();
                        let p = o.pixel(x, y);
                        let m = o.format().channels();
                        assert_eq!(m, mode.color_channels() + 1);
                        assert!((p[m - 1] - 1.0).abs() < 1e-3, "{mode:?} {sample:?} {alpha} {q:?}: alpha {p:?}");
                        for c in 0..mode.color_channels() {
                            assert!((p[c] - col[c]).abs() < 2.5 / 255.0, "{mode:?} {sample:?} {alpha} {q:?}: channel {c} {p:?} vs {col:?}");
                        }
                    }
                }
            }
        }
    }

    /// #4864: squashed to 100 % × 25 %, the layer's left and right edges stay as crisp as a direct
    /// (Bilinear) sample. The one factor (50 %) also reduced x, which left the first column inside
    /// the left edge at 0.93 alpha against Bilinear's 1.
    #[test]
    fn a_one_axis_reduction_keeps_the_side_edges_sharp() {
        let h = Homography::rect_to_quad([0.0, 0.0, 64.0, 64.0], [[10.0, 0.0], [74.0, 0.0], [74.0, 16.0], [10.0, 16.0]]).unwrap();
        for sample in photocraft_color::SampleType::ALL {
            let mut s = Surface::new(PixelFormat::new(photocraft_color::ColorMode::Rgb, sample, true));
            s.fill_rect(Rect::new(0, 0, 64, 64), &[0.5, 0.5, 0.5, 1.0]);
            let bc = warp_surface(&s, s.content_bounds(), &h, Interp::Bicubic).unwrap();
            let bl = warp_surface(&s, s.content_bounds(), &h, Interp::Bilinear).unwrap();
            for x in [9, 10, 11, 72, 73, 74] {
                let (a, b) = (bc.pixel(x, 8)[3], bl.pixel(x, 8)[3]);
                assert!((a - b).abs() < 0.02, "{sample:?} x {x}: Bicubic alpha {a} vs Bilinear {b}");
            }
        }
    }
}
