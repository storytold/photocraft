//! Free Transform: projective warps of surfaces.
//!
//! A transform is described by where the four corners of a source rectangle land (a quad). Scale,
//! rotate, skew and flips are affine special cases; Distort and Perspective use the full
//! homography. Warping is an inverse mapping with premultiplied-alpha sampling (nearest, bilinear
//! or Photoshop's Keys bicubic), processed per destination tile (in parallel on native) so memory stays
//! bounded by the tile working set. Bicubic reductions are pre-filtered, so shrinking a layer
//! doesn't alias: an affine map's source is resized first, each axis by its own scale, so an axis
//! that doesn't shrink stays sharp, and a perspective map filters each pixel for its own reduction.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use photocraft_color::PixelFormat;
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{AllocationError, Interrupt, Surface, Tile};
use serde::{Deserialize, Serialize};

use crate::resample::{keys_weight, resize_surface_per_axis, scaled_rect};

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
/// infinity (the result is then empty). Use [`warp_output_size`] to price the full result before
/// admitting a transform: clipping these bounds must not hide an excessive scale.
pub fn warp_output_bounds(src_rect: Rect, h: &Homography) -> Option<Rect> {
    corner_bounds(&warp_corners(src_rect, h)?)
}

/// Unclipped output dimensions, including the same filter support as [`warp_output_bounds`].
/// Position does not affect the price, so translating a small layer far away stays cheap.
/// Unrepresentable or non-finite dimensions saturate at `u64::MAX`, allowing a caller to refuse
/// the warp with a saturating pixel-budget comparison. Empty and singular inputs have no output.
pub fn warp_output_size(src_rect: Rect, h: &Homography) -> Option<(u64, u64)> {
    let corners = warp_corners(src_rect, h)?;
    if !corners.iter().all(|(x, y)| x.is_finite() && y.is_finite()) {
        return Some((u64::MAX, u64::MAX));
    }
    let x0 = corners.iter().map(|c| c.0).fold(f64::MAX, f64::min).floor();
    let y0 = corners.iter().map(|c| c.1).fold(f64::MAX, f64::min).floor();
    let x1 = corners.iter().map(|c| c.0).fold(f64::MIN, f64::max).ceil();
    let y1 = corners.iter().map(|c| c.1).fold(f64::MIN, f64::max).ceil();
    // Float-to-integer casts saturate, including when the finite endpoints' difference overflows.
    Some((((x1 - x0) as u64).saturating_add(2), ((y1 - y0) as u64).saturating_add(2)))
}

/// The farthest from the origin (px) [`warp_surface`] places a pixel: each side of
/// [`warp_output_bounds`] is clamped to ±2²⁰.
pub const WARP_REACH: i32 = 1 << 20;

/// Would [`warp_surface`] clip the result of warping `src_rect` by `h`: does a warped corner land
/// past [`WARP_REACH`], whether the result is that large or only placed that far out? A caller
/// refuses such a warp rather than lose the pixels beyond it. Empty and singular inputs have no
/// output to clip.
pub fn warp_output_clipped(src_rect: Rect, h: &Homography) -> bool {
    let Some(corners) = warp_corners(src_rect, h) else { return false };
    let lim = f64::from(WARP_REACH);
    let within = |v: f64| v.is_finite() && v.floor() >= -lim && v.ceil() <= lim;
    !corners.iter().all(|&(x, y)| within(x) && within(y))
}

fn warp_corners(src_rect: Rect, h: &Homography) -> Option<[(f64, f64); 4]> {
    if src_rect.is_empty() || h.inverse().is_none() {
        return None;
    }
    Some(
        [(src_rect.x0, src_rect.y0), (src_rect.x1, src_rect.y0), (src_rect.x1, src_rect.y1), (src_rect.x0, src_rect.y1)]
            .map(|(x, y)| h.apply(x as f64, y as f64)),
    )
}

/// The pixel rectangle over the warped `corners` plus a pixel for filter support, each side
/// clamped to [`WARP_REACH`]; `None` when a corner is not finite.
fn corner_bounds(corners: &[(f64, f64); 4]) -> Option<Rect> {
    if !corners.iter().all(|c| c.0.is_finite() && c.1.is_finite()) {
        return None;
    }
    let lim = f64::from(WARP_REACH);
    let bx0 = corners.iter().map(|c| c.0).fold(f64::MAX, f64::min).floor().max(-lim) as i32 - 1;
    let by0 = corners.iter().map(|c| c.1).fold(f64::MAX, f64::min).floor().max(-lim) as i32 - 1;
    let bx1 = corners.iter().map(|c| c.0).fold(f64::MIN, f64::max).ceil().min(lim) as i32 + 1;
    let by1 = corners.iter().map(|c| c.1).fold(f64::MIN, f64::max).ceil().min(lim) as i32 + 1;
    Some(Rect::new(bx0, by0, bx1, by1))
}

/// How a Bicubic warp prefilters its source.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Prefilter {
    /// The whole source once, reduced by these factors along x and y (1 for an axis that doesn't
    /// shrink).
    Axes(f64, f64),
    /// Each destination pixel for its own reduction.
    PerPixel,
}

/// How a Bicubic warp by `h` prefilters its source. An affine map (scale, skew, rotation) reduces
/// the source the same everywhere, so it is reduced once, each axis by its own factor, the length
/// of its Jacobian column: one factor for both axes (√|det J|) blurred an axis the map keeps 1:1
/// while it squashes the other, and filtered the squashed axis for less than its reduction (#4864).
/// A map that squashes along a slant (a rotated squash, or a skew that also flattens) shrinks a
/// direction between the axes far more than either axis, which no axis-aligned prefilter follows;
/// there both axes come down together until the warp shrinks no direction below what the one
/// factor leaves it, or below 50 % (where a 4×4 kernel starts to alias), whichever is lower. Where
/// the one factor leaves more than 50 %, the pair keeps more detail than it does, with a little
/// more room to alias along the slant. A perspective map (a Distort) reduces each part of the
/// source by a different amount, which no single prefilter follows: one factor taken at the centre
/// blurred the side the map magnifies and left the far side, which it shrinks more, aliasing
/// (#4864). Its pixels are each filtered for their own reduction instead.
fn prefilter_scales(h: &Homography, src_rect: Rect, clip: Option<Rect>) -> Prefilter {
    let r = [f64::from(src_rect.x0), f64::from(src_rect.y0), f64::from(src_rect.x1), f64::from(src_rect.y1)];
    let (cx, cy) = prefilter_centre(h, r, clip);
    // Affine when w is the same over the whole source. A Distort's quad can leave the projective
    // terms at rounding level, so this compares w itself rather than testing them for zero.
    let w0 = h.w(cx, cy);
    let affine =
        w0.is_finite() && w0 != 0.0 && [(r[0], r[1]), (r[2], r[1]), (r[2], r[3]), (r[0], r[3])].iter().all(|&(x, y)| (h.w(x, y) - w0).abs() <= 1e-9 * w0.abs());
    if !affine {
        return Prefilter::PerPixel;
    }
    let j = h.jacobian(cx, cy);
    let one = snap(h.local_scale(cx, cy));
    if !j.iter().all(|e| e.is_finite()) {
        return Prefilter::Axes(one, one);
    }
    // A uniform scale, turned or not, keeps the one factor: its column lengths differ from it only
    // by rounding, which would otherwise pick the order of the two passes.
    let (lx, ly) = (snap(j[0].hypot(j[2])), snap(j[1].hypot(j[3])));
    if (lx - one).abs() <= 1e-9 * one && (ly - one).abs() <= 1e-9 * one {
        return Prefilter::Axes(one, one);
    }
    let (fx, fy) = axis_factors(j, snap, f64::hypot);
    Prefilter::Axes(fx, fy)
}

/// The source point whose reduction stands for the whole source: the centre of `r`, or with a clip
/// the source point landing in the clip's centre.
fn prefilter_centre(h: &Homography, r: [f64; 4], clip: Option<Rect>) -> (f64, f64) {
    match clip.and_then(|c| Some((c, h.inverse()?))) {
        Some((c, inv)) => inv.apply((f64::from(c.x0) + f64::from(c.x1)) / 2.0, (f64::from(c.y0) + f64::from(c.y1)) / 2.0),
        None => ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0),
    }
}

/// A prefilter factor as `resize_surface_per_axis` takes it: exactly 1 (a copy) for an axis that
/// doesn't shrink, and at least 1/64, which bounds the filter's width.
fn snap(f: f64) -> f64 {
    if f > 0.0 && f < 0.999 { f.max(1.0 / 64.0) } else { 1.0 }
}

/// The one factor every map used before each axis got its own, √|det J| at [`prefilter_centre`].
/// A perspective map whose cells the valve refuses still falls back to it (see [`warp_per_pixel`]).
fn centre_factor(h: &Homography, src_rect: Rect, clip: Option<Rect>) -> f64 {
    let r = [f64::from(src_rect.x0), f64::from(src_rect.y0), f64::from(src_rect.x1), f64::from(src_rect.y1)];
    let (cx, cy) = prefilter_centre(h, r, clip);
    snap(h.local_scale(cx, cy))
}

/// The per-axis reductions of a map whose Jacobian is `j`, each kept in range by `fix`: an axis
/// shrinks by the length of its Jacobian column (measured by `len`), and both come down together
/// along a slant.
fn axis_factors(j: [f64; 4], fix: impl Fn(f64) -> f64, len: impl Fn(f64, f64) -> f64) -> (f64, f64) {
    let (fx, fy) = (fix(len(j[0], j[2])), fix(len(j[1], j[3])));
    // The warp is left `J · diag(1/fx, 1/fy)`. Along a slant both columns stay long while the map
    // squashes the direction between them, so both axes come down together until the warp shrinks
    // no direction below √(σmin / σmax), what the one factor √|det J| leaves, or below 50 %,
    // whichever is lower. A map that collapses (c = 0) reduces nothing, as before.
    let (hi, lo) = singular_values(j);
    let (_, left) = singular_values([j[0] / fx, j[1] / fy, j[2] / fx, j[3] / fy]);
    let target = if hi > 0.0 { (lo / hi).sqrt().min(0.5) } else { 0.0 };
    let c = if target > 0.0 && left.is_finite() { left / target } else { 0.0 };
    if c < 1.0 { (fix(fx * c), fix(fy * c)) } else { (fx, fy) }
}

/// A pixel's reduction kept within [1/64, 1], without [`snap`]'s jump to 1 just below 1, so the
/// filter changes continuously from pixel to pixel; one that isn't finite and positive reduces
/// nothing.
fn clamp_factor(f: f64) -> f64 {
    if f.is_finite() && f > 0.0 { f.clamp(1.0 / 64.0, 1.0) } else { 1.0 }
}

/// A Jacobian column's length on the per-pixel path, which measures two for every pixel twice:
/// `hypot`'s extra care only matters when the squares overflow or vanish.
fn column_length(a: f64, b: f64) -> f64 {
    let l = (a * a + b * b).sqrt();
    if l.is_finite() && l > 1e-150 { l } else { a.hypot(b) }
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

// Per-pixel prefiltering of perspective maps (#4864). The source is reduced by 2⁻ᵃ along x and
// 2⁻ᵇ along y into octave cells (a, b), (0, 0) being the source itself; each destination pixel
// samples, along each axis, the finest octave no smaller than its own factor (2⁻ᵃ ≥ f), with a
// Keys kernel widened by what that octave leaves (2⁻ᵃ / f), so every part of the source is
// filtered for its own reduction. Cells are built only where some pixel reads them.

/// The coarsest octave, 2⁻⁶: the 1/64 the one-factor prefilter stopped at.
const MAX_LEVEL: u8 = 6;
/// Where in an octave the cross-fade into the next one starts: its top quarter.
const FADE_START: f64 = 0.75;
/// Octave `a`'s reduction, 2⁻ᵃ.
const OCTAVE: [f64; 7] = [1.0, 0.5, 0.25, 0.125, 0.0625, 0.03125, 0.015625];
/// How far a destination pixel can read from its source point, in source pixels, at the coarsest
/// octave: a cell's own filter reaches 2 · 2ᵃ + 4, and the sampler's widened kernel under 4 · 2ᵃ.
const REACH: i32 = 6 * 64 + 8;
/// Bytes of windows one destination tile reads at once; a tile that needs more is split in four,
/// down to `SPLIT_DEPTH` times.
const WINDOW_CAP: usize = 64 << 20;
const SPLIT_DEPTH: u32 = 4;
/// Rows of cell pixels one build job makes: a tile row of the cell.
const BAND: i32 = TILE_SIZE;
/// Columns of cell pixels one build job makes, at most: four tiles, so the resize's row buffers
/// stay small however wide the layer is.
const CHUNK: i32 = 4 * TILE_SIZE;
/// The source area (pixels) below which the valve never refuses the cells: a small layer's cells
/// cost little, however many of them its map needs.
const VALVE_FLOOR: f64 = (1u32 << 22) as f64;
/// The valve's probe plans every 8th pixel each way, about 1/64 of a full plan's work.
const PROBE_STEP: usize = 8;

fn octave_scale(a: usize) -> f64 {
    OCTAVE.get(a).copied().unwrap_or(1.0)
}

/// One axis of a pixel's filter: the octave cells it samples (two in the top quarter of an
/// octave, cross-fading into the next), each with the factor `r` (1 to 2) its kernel is widened by
/// and its weight.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Axis {
    lv: [u8; 2],
    r: [f64; 2],
    w: [f64; 2],
    n: usize,
}

impl Axis {
    /// The cells this axis samples: (octave, kernel widening, weight).
    fn entries(&self) -> impl Iterator<Item = (usize, f64, f64)> + '_ {
        self.lv.iter().zip(self.r).zip(self.w).take(self.n).map(|((&l, r), w)| (usize::from(l), r, w))
    }
}

/// The cells filtering a reduction `f`: octave a = ⌊−log₂ f⌋ (at most 6), whose own reduction
/// leaves r = 2^(−log₂ f − a) to the sampler. Over the top quarter of the octave it fades into
/// octave a + 1 at r = 1, which is where r reaches 2, so the filter never jumps.
fn octave(f: f64) -> Axis {
    let e = -f.log2();
    let e = if e.is_nan() { 0.0 } else { e.clamp(0.0, f64::from(MAX_LEVEL)) };
    let a = e.floor().min(f64::from(MAX_LEVEL));
    let t = e - a;
    let w = if a < f64::from(MAX_LEVEL) { ((t - FADE_START) / (1.0 - FADE_START)).clamp(0.0, 1.0) } else { 0.0 };
    let lv = a as u8;
    if w > 0.0 { Axis { lv: [lv, lv + 1], r: [t.exp2(), 1.0], w: [1.0 - w, w], n: 2 } } else { Axis { lv: [lv, lv], r: [t.exp2(), 1.0], w: [1.0, 0.0], n: 1 } }
}

/// A destination pixel's filter: its source point, each axis's cells, and how much of the pixel
/// the warped source rect covers along u and v (the outline's antialiasing).
#[derive(Clone, Copy, Debug)]
struct Lod {
    u: f64,
    v: f64,
    ax: Axis,
    ay: Axis,
    cu: f64,
    cv: f64,
}

/// What the per-pixel path reads of a warp's setup, computed as today's path computes it.
struct Geometry {
    h: Homography,
    inv: Homography,
    src_rect: Rect,
    side: Option<f64>,
    w_lo: f64,
    front: Option<f64>,
    /// Cell (a, b)'s rect: the source rect reduced by 2⁻ᵃ along x and 2⁻ᵇ along y.
    cells: [[Rect; 7]; 7],
    /// Every pixel samples the source alone ([`samples_source_only`]), so tiles need no plan.
    source_only: bool,
}

impl Geometry {
    fn cell(&self, a: usize, b: usize) -> Option<Rect> {
        self.cells.get(a).and_then(|row| row.get(b)).copied()
    }
}

/// Whether every pixel a perspective map draws samples the source alone (cell (0, 0)), from bounds
/// over the whole source and the sampler's margin: each axis keeps at least 0.6 of its length (the
/// cross-fade into octave 1 starts below 2^−0.75 ≈ 0.595), and every direction at least 0.51, so
/// the slant rule in [`axis_factors`], which acts only below half, leaves both axes alone. A pixel's
/// kernel is then widened by less than 1 / 0.6, and its taps lie within 4 source pixels of its
/// source point. `false` whenever the bounds can't show it, including when they overflow. The
/// caller checks that w keeps one sign over the source.
fn samples_source_only(h: &Homography, src_rect: Rect) -> bool {
    let m = &h.0;
    let (x0, y0) = (f64::from(src_rect.x0) - 1.0, f64::from(src_rect.y0) - 1.0);
    let (x1, y1) = (f64::from(src_rect.x1) + 1.0, f64::from(src_rect.y1) + 1.0);
    let ws = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].map(|(u, v)| (m[6] * u + m[7] * v + m[8]).abs());
    let (w_min, w_max) = (ws.iter().copied().fold(f64::INFINITY, f64::min), ws.iter().copied().fold(0.0f64, f64::max));
    // The Jacobian's columns times w²: the x column is affine in y alone, the y column in x alone.
    let col_x = |y: f64| [(m[0] * m[7] - m[6] * m[1]) * y + (m[0] * m[8] - m[6] * m[2]), (m[3] * m[7] - m[6] * m[4]) * y + (m[3] * m[8] - m[6] * m[5])];
    let col_y = |x: f64| [(m[1] * m[6] - m[7] * m[0]) * x + (m[1] * m[8] - m[7] * m[2]), (m[4] * m[6] - m[7] * m[3]) * x + (m[4] * m[8] - m[7] * m[5])];
    // The shortest and longest of them along a segment: its nearest point to 0, and its far end.
    let span = |a: [f64; 2], b: [f64; 2]| {
        let d = [b[0] - a[0], b[1] - a[1]];
        let dd = d[0] * d[0] + d[1] * d[1];
        let t = if dd > 0.0 { (-(a[0] * d[0] + a[1] * d[1]) / dd).clamp(0.0, 1.0) } else { 0.0 };
        (f64::hypot(a[0] + t * d[0], a[1] + t * d[1]), f64::hypot(a[0], a[1]).max(f64::hypot(b[0], b[1])))
    };
    let (x_lo, x_hi) = span(col_x(y0), col_x(y1));
    let (y_lo, y_hi) = span(col_y(x0), col_y(x1));
    let det = (m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6]) + m[2] * (m[3] * m[7] - m[4] * m[6])).abs();
    // A column is at least its shortest numerator over the largest w². The weakest direction
    // stretches by |det J| / σmax, where |det J| = |det H| / w³ and σmax is at most the columns'
    // root sum of squares.
    let column = x_lo.min(y_lo) / (w_max * w_max);
    let weakest = det / (w_max * w_max * w_max) * (w_min * w_min) / x_hi.hypot(y_hi);
    w_min > 0.0 && column.is_finite() && weakest.is_finite() && column >= 0.6 && weakest >= 0.51
}

/// Destination pixel (x, y)'s filter, from the map's exact Jacobian at its source point: each axis
/// reduced by its own factor, by the rule [`prefilter_scales`] gives affine maps but without the
/// snap. `None` for a pixel today's path skips (behind the horizon, or off the source), and for a
/// source point that isn't finite.
fn pixel_lod(g: &Geometry, x: i32, y: i32) -> Option<Lod> {
    let (px, py) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
    let m = &g.inv.0;
    // `inv.w`, `inv.apply` and `inv.jacobian` at (px, py), sharing their terms.
    let w = m[6] * px + m[7] * py + m[8];
    if g.front.is_some_and(|s| s * w <= 0.0) {
        return None;
    }
    let (u, v) = ((m[0] * px + m[1] * py + m[2]) / w, (m[3] * px + m[4] * py + m[5]) / w);
    let r = g.src_rect;
    let (x0, y0, x1, y1) = (f64::from(r.x0), f64::from(r.y0), f64::from(r.x1), f64::from(r.y1));
    if !(u - 0.5 >= x0 - 1.0 && v - 0.5 >= y0 - 1.0 && u - 0.5 <= x1 && v - 0.5 <= y1) {
        return None;
    }
    let j = g.h.jacobian(u, v);
    let (fx, fy) = if j.iter().all(|e| e.is_finite()) { axis_factors(j, clamp_factor, column_length) } else { (1.0, 1.0) };
    // The pixel's extent in source pixels along u and v.
    let ji = [(m[0] - m[6] * u) / w, (m[1] - m[7] * u) / w, (m[3] - m[6] * v) / w, (m[4] - m[7] * v) / w];
    let extent = |e: f64| if e.is_finite() && e > 1e-12 { e } else { 1.0 };
    let (foot_u, foot_v) = (extent(ji[0].abs() + ji[1].abs()), extent(ji[2].abs() + ji[3].abs()));
    let cu = ((u - x0).min(x1 - u) / foot_u + 0.5).clamp(0.0, 1.0);
    let cv = ((v - y0).min(y1 - v) / foot_v + 0.5).clamp(0.0, 1.0);
    Some(Lod { u, v, ax: octave(fx), ay: octave(fy), cu, cv })
}

/// The first and last pixel within 2r of position `p`, where a kernel widened by `r` isn't 0.
fn tap_span(p: f64, r: f64) -> (i64, i64) {
    (((p - 2.0 * r).floor() as i64).saturating_add(1), ((p + 2.0 * r).ceil() as i64).saturating_sub(1))
}

/// Pixel `i` of a row `lo..hi`, clamped into it: past the edge the edge repeats, as the pixel
/// stretch does (#3773).
fn edge(i: i64, lo: i32, hi: i32) -> i32 {
    i.min(i64::from(hi) - 1).max(i64::from(lo)) as i32
}

/// The taps of Photoshop's Keys (−0.75) kernel widened by `r` (1 to 2) around position `p` in a
/// row `lo..hi`, weights normalised to 1. At r = 1 these are [`bicubic`]'s four.
fn taps(p: f64, r: f64, lo: i32, hi: i32) -> ([i32; 9], [f64; 9], usize) {
    let (mut idx, mut wt) = ([0i32; 9], [0.0f64; 9]);
    let (first, last) = tap_span(p, r);
    let (mut n, mut sum) = (0, 0.0);
    for i in first..=last {
        let (Some(ix), Some(w)) = (idx.get_mut(n), wt.get_mut(n)) else { break };
        *ix = edge(i, lo, hi);
        *w = keys_weight((p - i as f64) / r, -0.75);
        sum += *w;
        n += 1;
    }
    if sum.is_nan() || sum <= 0.0 {
        // Never for a finite position: its nearest pixel alone.
        let mut one = [0.0; 9];
        one[0] = 1.0;
        return ([edge(p.round() as i64, lo, hi); 9], one, 1);
    }
    for w in wt.iter_mut().take(n) {
        *w /= sum;
    }
    (idx, wt, n)
}

/// Adds `k` times the sample at `(p, q)` of a cell whose rect is `cell` (taps widened by `(rx, ry)`)
/// to `acc`, reading the premultiplied window `win` of the cell's pixels in `rect`. A tap outside
/// the window adds nothing, as today's reads past the footprint do (plans keep them all inside).
fn sample_widened(win: &[f32], rect: Rect, cell: Rect, (p, q): (f64, f64), (rx, ry): (f64, f64), k: f64, acc: &mut [f64]) {
    let n = acc.len();
    let (ix, wx, nx) = taps(p, rx, cell.x0, cell.x1);
    let (iy, wy, ny) = taps(q, ry, cell.y0, cell.y1);
    let (ww, wh) = (rect.width() as usize, rect.height() as usize);
    let at = |i: i32, lo: i32, len: usize| usize::try_from(i64::from(i) - i64::from(lo)).ok().filter(|&d| d < len);
    // Offsets of the taps in the window; `usize::MAX` for one outside it, which reads nothing.
    let mut cols = [usize::MAX; 9];
    for (c, &xi) in cols.iter_mut().zip(&ix).take(nx) {
        *c = at(xi, rect.x0, ww).map_or(usize::MAX, |d| d * n);
    }
    let mut rows = [usize::MAX; 9];
    for (r, &yj) in rows.iter_mut().zip(&iy).take(ny) {
        *r = at(yj, rect.y0, wh).map_or(usize::MAX, |d| d * ww * n);
    }
    let (nx, ny) = (nx.min(9), ny.min(9));
    let t = WindowTaps {
        cols: cols.get(..nx).unwrap_or(&[]),
        wx: wx.get(..nx).unwrap_or(&[]),
        rows: rows.get(..ny).unwrap_or(&[]),
        wy: wy.get(..ny).unwrap_or(&[]),
        k,
    };
    match acc {
        [a, b] => t.add([a, b], win),
        [a, b, c, d] => t.add([a, b, c, d], win),
        [a, b, c, d, e] => t.add([a, b, c, d, e], win),
        // With alpha a pixel has 2, 4 or 5 channels (`ColorMode::color_channels` and alpha).
        _ => {}
    }
}

/// One sample's taps in a window: the offsets and weights of its columns and rows, and its weight.
struct WindowTaps<'a> {
    cols: &'a [usize],
    wx: &'a [f64],
    rows: &'a [usize],
    wy: &'a [f64],
    k: f64,
}

impl WindowTaps<'_> {
    /// Adds the sample to `acc` row by row, unrolled for the common channel counts (gray, RGB or
    /// Lab, and CMYK, each with alpha).
    fn add<const N: usize>(&self, acc: [&mut f64; N], win: &[f32]) {
        let mut sum = [0.0f64; N];
        for (&row, &wy) in self.rows.iter().zip(self.wy) {
            let mut h = [0.0f64; N];
            for (&col, &wx) in self.cols.iter().zip(self.wx) {
                if let Some(px) = row.checked_add(col).and_then(|o| win.get(o..o.checked_add(N)?)) {
                    for (h, &v) in h.iter_mut().zip(px) {
                        *h += f64::from(v) * wx;
                    }
                }
            }
            for (s, h) in sum.iter_mut().zip(h) {
                *s += h * wy;
            }
        }
        for (a, s) in acc.into_iter().zip(sum) {
            *a += s * self.k;
        }
    }
}

/// The windows a destination tile reads: per cell, the bounding box of every tap its pixels take
/// there (clamped as the sampler clamps them), so the tile never reads past them.
struct TilePlan {
    tile: Rect,
    cells: Vec<(u8, u8, Rect)>,
}

/// Plans tile `t` from every `step`th pixel each way (1 for the plan it renders with; a coarser
/// step's windows lie inside that plan's). `None` when it draws nothing: today's footprint tests,
/// with the source looked for as far as a pixel's filter reaches, so a tile whose own footprint is
/// empty but whose filters reach the source still draws.
fn plan_tile(t: Rect, g: &Geometry, src: &Surface, step: usize) -> Option<TilePlan> {
    let foot = footprint(&t, &g.inv, g.side, g.w_lo, g.src_rect);
    if foot.is_empty() || !src.has_tiles_in(foot.inflate(REACH)) {
        return None;
    }
    if g.source_only {
        // The footprint, padded by 3 and 2 more, holds every tap (within 4 pixels of a source
        // point): planning each pixel would only narrow the window, which changes no pixel.
        let win = foot.inflate(2).intersect(&g.src_rect);
        return (!win.is_empty()).then(|| TilePlan { tile: t, cells: vec![(0, 0, win)] });
    }
    let mut boxes = [[Rect::EMPTY; 7]; 7];
    let step = step.max(1);
    for y in (t.y0..t.y1).step_by(step) {
        for x in (t.x0..t.x1).step_by(step) {
            let Some(l) = pixel_lod(g, x, y) else { continue };
            for (a, rx, wx) in l.ax.entries() {
                for (b, ry, wy) in l.ay.entries() {
                    let (Some(cell), Some(slot)) = (g.cell(a, b), boxes.get_mut(a).and_then(|row| row.get_mut(b))) else { continue };
                    if wx * wy <= 0.0 {
                        continue;
                    }
                    let (fx, lx) = tap_span(l.u * octave_scale(a) - 0.5, rx);
                    let (fy, ly) = tap_span(l.v * octave_scale(b) - 0.5, ry);
                    let (x0, x1) = (edge(fx, cell.x0, cell.x1), edge(lx, cell.x0, cell.x1));
                    let (y0, y1) = (edge(fy, cell.y0, cell.y1), edge(ly, cell.y0, cell.y1));
                    *slot = slot.union(&Rect::new(x0, y0, x1.saturating_add(1), y1.saturating_add(1)));
                }
            }
        }
    }
    let mut cells = Vec::new();
    for (a, row) in boxes.iter().enumerate() {
        for (b, r) in row.iter().enumerate() {
            if !r.is_empty() {
                cells.push((a as u8, b as u8, *r));
            }
        }
    }
    (!cells.is_empty()).then_some(TilePlan { tile: t, cells })
}

/// The source inside `s` as the cells see it: past `src_rect`'s edges each pixel repeats the nearest
/// one inside, as the sampler's taps do at full size (the pixel stretch, #3773), so a cell neither
/// fades nor takes on the default pixel at the source's edges and the outline is antialiased by
/// coverage alone, at every octave. Outside `s` is the default pixel, so resizing this reads,
/// inside `s`, exactly what resizing the edge-repeated source reads. Tiles wholly inside both are
/// shared, not copied.
fn crop_tiles(src: &Surface, src_rect: Rect, s: Rect) -> Result<Surface, AllocationError> {
    let mut crop = Surface::with_default(src.format(), &src.default_pixel());
    let (n, bpp) = (src.format().channels(), src.format().bytes_per_pixel());
    let def = src.default_bytes();
    if def.len() != bpp || src_rect.is_empty() {
        return Err(AllocationError::SizeOverflow);
    }
    let (cx, cy) = (|x: i32| x.clamp(src_rect.x0, src_rect.x1 - 1), |y: i32| y.clamp(src_rect.y0, src_rect.y1 - 1));
    for c in s.tiles() {
        let tr = c.rect();
        let part = tr.intersect(&s);
        if !src_rect.contains_rect(&part) {
            let read = Rect::new(cx(part.x0), cy(part.y0), cx(part.x1 - 1) + 1, cy(part.y1 - 1) + 1);
            let px = src.try_read_region(read)?;
            let rw = read.width() as usize;
            let len = (part.width() as usize).checked_mul(part.height() as usize).and_then(|v| v.checked_mul(n)).ok_or(AllocationError::SizeOverflow)?;
            let mut out = Vec::new();
            out.try_reserve_exact(len).map_err(|_| AllocationError::NotEnoughMemory { bytes: len.saturating_mul(4) })?;
            for y in part.y0..part.y1 {
                let row = (cy(y) - read.y0) as usize * rw;
                for x in part.x0..part.x1 {
                    let o = (row + (cx(x) - read.x0) as usize) * n;
                    out.extend_from_slice(px.get(o..o + n).ok_or(AllocationError::SizeOverflow)?);
                }
            }
            crop.try_write_region(part, &out)?;
            continue;
        }
        let Some(t) = src.tile(c) else { continue };
        if s.contains_rect(&tr) {
            crop.put_tiles([(c, Arc::clone(t))]);
            continue;
        }
        let keep = tr.intersect(&s);
        let (k0, k1) = (usize::try_from(keep.x0 - tr.x0).unwrap_or(0), usize::try_from(keep.x1 - tr.x0).unwrap_or(0));
        let mut copy = t.try_clone()?;
        for (ly, row) in copy.bytes_mut().chunks_exact_mut(TILE_SIZE as usize * bpp).enumerate() {
            let y = tr.y0.saturating_add(ly as i32);
            let inside = y >= keep.y0 && y < keep.y1;
            for (lx, px) in row.chunks_exact_mut(bpp).enumerate() {
                if !inside || lx < k0 || lx >= k1 {
                    px.copy_from_slice(def);
                }
            }
        }
        crop.put_tiles([(c, Arc::new(copy))]);
    }
    Ok(crop)
}

/// Builds the part `r` of cell (a, b) in its tile row `j`: a resize of the source around it (edges
/// repeated, see [`crop_tiles`]), which is anchored at the document origin, so inside `r` it is
/// exactly what reducing the whole edge-repeated source gives. Returns the cell's tiles in that row
/// that `r` touches.
fn build_cell_band(src: &Surface, src_rect: Rect, a: u8, b: u8, j: i32, r: Rect) -> Result<Vec<(TileCoord, Arc<Tile>)>, AllocationError> {
    let (sa, sb) = (1i64 << a, 1i64 << b);
    // The resize's filter reaches under 2 · 2ᵃ source pixels past a cell pixel's span (in its box
    // averaging below a quarter too).
    let (pad_x, pad_y) = (2 * sa + 4, 2 * sb + 4);
    let at = |v: i64| v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
    let s = Rect::new(at(i64::from(r.x0) * sa - pad_x), at(i64::from(r.y0) * sb - pad_y), at(i64::from(r.x1) * sa + pad_x), at(i64::from(r.y1) * sb + pad_y));
    let crop = crop_tiles(src, src_rect, s)?;
    let out = resize_surface_per_axis(&crop, octave_scale(usize::from(a)), octave_scale(usize::from(b)));
    let keep = Rect::new(r.x0, j.saturating_mul(BAND), r.x1, j.saturating_add(1).saturating_mul(BAND));
    Ok(out.tiles().filter(|(c, _)| c.ty == j && !c.rect().intersect(&keep).is_empty()).map(|(c, t)| (*c, Arc::clone(t))).collect())
}

/// A rendered destination tile: its rect and its pixels.
type TileOut = (Rect, Vec<f32>);

/// What rendering a tile reads: the geometry, the source (cell (0, 0)) and the built cells.
struct Render<'a> {
    g: &'a Geometry,
    src: &'a Surface,
    built: &'a [Option<Surface>],
    n: usize,
    cap: usize,
}

impl Render<'_> {
    /// Renders the pixels of `t` (planned as `cells`) into `buf`, which holds the tile `top`.
    /// Whether any pixel was drawn.
    fn tile(&self, t: Rect, cells: &[(u8, u8, Rect)], depth: u32, top: Rect, buf: &mut [f32]) -> Result<bool, AllocationError> {
        let n = self.n;
        let bytes = cells.iter().fold(0usize, |s, (_, _, r)| s.saturating_add((r.width() as usize).saturating_mul(r.height() as usize).saturating_mul(n * 4)));
        if bytes > self.cap && depth < SPLIT_DEPTH && (t.width() > 1 || t.height() > 1) {
            // No pixel depends on the windows it is read through, so splitting changes nothing.
            let (mx, my) = (t.x0 + (t.width() / 2) as i32, t.y0 + (t.height() / 2) as i32);
            let mut any = false;
            for q in [Rect::new(t.x0, t.y0, mx, my), Rect::new(mx, t.y0, t.x1, my), Rect::new(t.x0, my, mx, t.y1), Rect::new(mx, my, t.x1, t.y1)] {
                if q.is_empty() {
                    continue;
                }
                if let Some(p) = plan_tile(q, self.g, self.src, 1) {
                    any |= self.tile(q, &p.cells, depth + 1, top, buf)?;
                }
            }
            return Ok(any);
        }
        // Premultiplied windows, by cell.
        let mut index = [[usize::MAX; 7]; 7];
        let mut wins: Vec<(Rect, Vec<f32>)> = Vec::new();
        for &(a, b, r) in cells {
            let surf = if (a, b) == (0, 0) { Some(self.src) } else { self.built.get(usize::from(a) * 7 + usize::from(b)).and_then(Option::as_ref) };
            let (Some(surf), Some(slot)) = (surf, index.get_mut(usize::from(a)).and_then(|row| row.get_mut(usize::from(b)))) else { continue };
            let mut px = surf.try_read_region(r)?;
            for p in px.chunks_exact_mut(n) {
                if let Some((al, colour)) = p.split_last_mut() {
                    for v in colour {
                        *v *= *al;
                    }
                }
            }
            *slot = wins.len();
            wins.push((r, px));
        }
        let alpha = n - 1;
        let w = top.width() as usize;
        let mut acc = vec![0.0f64; n];
        let mut any = false;
        for y in t.y0..t.y1 {
            for x in t.x0..t.x1 {
                let Some(l) = pixel_lod(self.g, x, y) else { continue };
                acc.fill(0.0);
                for (a, rx, wx) in l.ax.entries() {
                    for (b, ry, wy) in l.ay.entries() {
                        let k = wx * wy;
                        let (Some((rect, win)), Some(cell)) = (index.get(a).and_then(|row| row.get(b)).and_then(|&i| wins.get(i)), self.g.cell(a, b)) else {
                            continue;
                        };
                        if k > 0.0 {
                            sample_widened(win, *rect, cell, (l.u * octave_scale(a) - 0.5, l.v * octave_scale(b) - 0.5), (rx, ry), k, &mut acc);
                        }
                    }
                }
                for s in acc.iter_mut() {
                    *s *= l.cu * l.cv;
                }
                let raw_alpha = acc.get(alpha).copied().unwrap_or(0.0);
                let al = raw_alpha.clamp(0.0, 1.0);
                if al.is_nan() || al <= 0.0 {
                    continue;
                }
                let o = ((y - top.y0) as usize * w + (x - top.x0) as usize) * n;
                let Some(dst) = buf.get_mut(o..o + n) else { continue };
                any = true;
                for (d, s) in dst.iter_mut().zip(&acc).take(alpha) {
                    // Like the affine sampler, un-premultiply before clamping ringing alpha
                    // (#6203); dividing by the clamped value would brighten a soft edge.
                    // Integer storage clips on write; floats retain their signed/HDR range.
                    *d = (s / raw_alpha) as f32;
                }
                if let Some(d) = dst.get_mut(alpha) {
                    *d = al as f32;
                }
            }
        }
        Ok(any)
    }
}

/// Plans each of `tiles` from every `step`th pixel ([`plan_tile`]), in parallel on native.
fn plan_tiles(tiles: &[Rect], g: &Geometry, src: &Surface, ctl: &Interrupt, step: usize) -> Vec<TilePlan> {
    let plan = |t: &Rect| if ctl.cancelled() { None } else { plan_tile(*t, g, src, step) };
    #[cfg(not(target_arch = "wasm32"))]
    let plans = {
        use rayon::prelude::*;
        tiles.par_iter().filter_map(plan).collect()
    };
    #[cfg(target_arch = "wasm32")]
    let plans = tiles.iter().filter_map(plan).collect();
    plans
}

/// The cell builds `plans` need: one job per cell, tile row and four tile columns of it, covering
/// what the plans read there; each job makes whole tiles of its own.
fn build_jobs(plans: &[TilePlan]) -> BTreeMap<(u8, u8, i32, i32), Rect> {
    let mut jobs: BTreeMap<(u8, u8, i32, i32), Rect> = BTreeMap::new();
    for p in plans {
        for &(a, b, r) in p.cells.iter().filter(|c| (c.0, c.1) != (0, 0)) {
            for j in r.y0.div_euclid(BAND)..=(r.y1 - 1).div_euclid(BAND) {
                for k in r.x0.div_euclid(CHUNK)..=(r.x1 - 1).div_euclid(CHUNK) {
                    let (x0, x1) = (r.x0.max(k.saturating_mul(CHUNK)), r.x1.min(k.saturating_add(1).saturating_mul(CHUNK)));
                    let part = Rect::new(x0, r.y0.max(j.saturating_mul(BAND)), x1, r.y1.min(j.saturating_add(1).saturating_mul(BAND)));
                    jobs.entry((a, b, j, k)).and_modify(|u| *u = u.union(&part)).or_insert(part);
                }
            }
        }
    }
    jobs
}

/// The valve: whether the cell builds `jobs` would read more than three times the source's area
/// or hold more pixels than the source, the area taken as at least `tuning.valve_floor`.
fn refuses(jobs: &BTreeMap<(u8, u8, i32, i32), Rect>, g: &Geometry, tuning: Tuning) -> bool {
    let area = (f64::from(g.src_rect.width()) * f64::from(g.src_rect.height())).max(tuning.valve_floor);
    let (mut reads, mut held) = (0.0, 0.0);
    for (&(a, b, _, _), r) in jobs {
        let px = f64::from(r.width()) * f64::from(r.height());
        held += px;
        reads += px / (octave_scale(usize::from(a)) * octave_scale(usize::from(b)));
    }
    reads > 3.0 * area || held > area
}

/// Bicubic by a perspective map: each pixel of `tiles` filtered for its own reduction, from octave
/// cells of the source built only where the tiles read them. `Ok(None)` when the valve
/// ([`refuses`]) turns the cells down, as for a near-uniform shrink in the top quarter of an octave
/// on both axes, which samples four cells everywhere; the caller then runs today's one-factor path.
fn warp_per_pixel(src: &Surface, g: &Geometry, tiles: &[Rect], ctl: &Interrupt, tuning: Tuning) -> Result<Option<Vec<TileOut>>, AllocationError> {
    // A probe first, every 8th pixel each way, at about 1/64 the cost: its windows lie inside the
    // full plan's, so cells the valve turns down for it, it would turn down for the full plan too.
    if refuses(&build_jobs(&plan_tiles(tiles, g, src, ctl, PROBE_STEP)), g, tuning) {
        return Ok(None);
    }
    let plans = plan_tiles(tiles, g, src, ctl, 1);
    let jobs = build_jobs(&plans);
    if refuses(&jobs, g, tuning) {
        return Ok(None);
    }
    let jobs: Vec<((u8, u8, i32, i32), Rect)> = jobs.into_iter().collect();
    let started = AtomicUsize::new(0);
    let count = jobs.len().max(1);
    type Band = (u8, u8, Vec<(TileCoord, Arc<Tile>)>);
    let build = |&((a, b, j, _), r): &((u8, u8, i32, i32), Rect)| -> Result<Option<Band>, AllocationError> {
        if ctl.cancelled() {
            return Ok(None);
        }
        let k = started.fetch_add(1, Ordering::Relaxed) + 1;
        ctl.progress(0.2 * k as f32 / count as f32);
        Ok(Some((a, b, build_cell_band(src, g.src_rect, a, b, j, r)?)))
    };
    #[cfg(not(target_arch = "wasm32"))]
    let bands: Vec<Option<Band>> = {
        use rayon::prelude::*;
        jobs.par_iter().map(build).collect::<Result<_, _>>()?
    };
    #[cfg(target_arch = "wasm32")]
    let bands: Vec<Option<Band>> = jobs.iter().map(build).collect::<Result<_, _>>()?;
    if ctl.cancelled() {
        return Ok(Some(Vec::new()));
    }
    let mut built: Vec<Option<Surface>> = (0..49).map(|_| None).collect();
    for (a, b, tiles) in bands.into_iter().flatten() {
        if let Some(slot) = built.get_mut(usize::from(a) * 7 + usize::from(b)) {
            slot.get_or_insert_with(|| Surface::with_default(src.format(), &src.default_pixel())).put_tiles(tiles);
        }
    }
    let render = Render { g, src, built: &built, n: src.format().channels(), cap: tuning.window_cap };
    let finished = AtomicUsize::new(0);
    let total = plans.len().max(1);
    let work = |p: &TilePlan| -> Result<Option<(Rect, Vec<f32>)>, AllocationError> {
        if ctl.cancelled() {
            return Ok(None);
        }
        let k = finished.fetch_add(1, Ordering::Relaxed) + 1;
        ctl.progress(0.2 + 0.78 * k as f32 / total as f32);
        let t = p.tile;
        let len = (t.width() as usize).checked_mul(t.height() as usize).and_then(|v| v.checked_mul(render.n)).ok_or(AllocationError::SizeOverflow)?;
        let bytes = len.checked_mul(std::mem::size_of::<f32>()).ok_or(AllocationError::SizeOverflow)?;
        let mut buf = Vec::new();
        buf.try_reserve_exact(len).map_err(|_| AllocationError::NotEnoughMemory { bytes })?;
        buf.resize(len, 0.0f32);
        Ok(render.tile(t, &p.cells, 0, t, &mut buf)?.then_some((t, buf)))
    };
    let result_bytes = plans.len().checked_mul(std::mem::size_of::<(Rect, Vec<f32>)>()).ok_or(AllocationError::SizeOverflow)?;
    let mut done = Vec::new();
    done.try_reserve_exact(plans.len()).map_err(|_| AllocationError::NotEnoughMemory { bytes: result_bytes })?;
    let done = std::sync::Mutex::new(done);
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        plans.par_iter().try_for_each(|p| -> Result<(), AllocationError> {
            if let Some(result) = work(p)? {
                done.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(result);
            }
            Ok(())
        })?;
    }
    #[cfg(target_arch = "wasm32")]
    for p in &plans {
        if let Some(result) = work(p)? {
            done.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(result);
        }
    }
    Ok(Some(done.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner)))
}

/// The source footprint of destination tile `t`: its corners mapped back (the part of it in front
/// of the inverse horizon, when the forward w keeps one sign), padded for the filter, within
/// `src_rect`; all of `src_rect` for a tile straddling the inverse horizon, which has no bounded
/// footprint. Empty when no part of the tile samples the source.
fn footprint(t: &Rect, inv: &Homography, side: Option<f64>, w_lo: f64, src_rect: Rect) -> Rect {
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
        return Rect::EMPTY;
    }
    let fp: Vec<(f64, f64)> = poly.iter().map(|&(x, y)| inv.apply(x, y)).collect();
    let fx0 = (fp.iter().map(|c| c.0).fold(f64::MAX, f64::min).floor() as i32).saturating_sub(3);
    let fy0 = (fp.iter().map(|c| c.1).fold(f64::MAX, f64::min).floor() as i32).saturating_sub(3);
    let fx1 = (fp.iter().map(|c| c.0).fold(f64::MIN, f64::max).ceil() as i32).saturating_add(3);
    let fy1 = (fp.iter().map(|c| c.1).fold(f64::MIN, f64::max).ceil() as i32).saturating_add(3);
    // Without a one-signed forward w, a tile straddling the inverse horizon has no bounded footprint.
    let straddles = side.is_none() && wc.iter().any(|w| *w <= 0.0) && wc.iter().any(|w| *w >= 0.0);
    if straddles { src_rect } else { Rect::new(fx0, fy0, fx1, fy1).intersect(&src_rect) }
}

/// Knobs the tests turn: the valve forced (a perspective map takes today's one-factor path, as the
/// valve's own fallback does), the window cap, the valve's floor, and every tile planned pixel by
/// pixel.
#[derive(Clone, Copy, Debug)]
struct Tuning {
    force_valve: bool,
    window_cap: usize,
    /// The source area below which the valve never refuses the cells ([`refuses`]).
    valve_floor: f64,
    /// Plan every tile pixel by pixel, even where [`samples_source_only`] says no pixel needs it.
    plan_pixels: bool,
}

impl Tuning {
    const DEFAULT: Tuning = Tuning { force_valve: false, window_cap: WINDOW_CAP, valve_floor: VALVE_FLOOR, plan_pixels: false };
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
    warp_with(src, src_rect, h, interp, clip, ctl, Tuning::DEFAULT)
}

fn warp_with(
    src: &Surface,
    src_rect: Rect,
    h: &Homography,
    interp: Interp,
    clip: Option<Rect>,
    ctl: &Interrupt,
    tuning: Tuning,
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
    // samples the source directly, as Photoshop's does (exact at 50 % without a prefilter). A
    // perspective map instead filters each pixel for its own reduction (`warp_per_pixel`); the
    // valve falls back to the one factor at the centre, as before.
    let mut per_pixel = false;
    if interp == Interp::Bicubic {
        let (fx, fy) = match prefilter_scales(&h, src_rect, clip) {
            Prefilter::Axes(fx, fy) => (fx, fy),
            Prefilter::PerPixel if tuning.force_valve => {
                let one = centre_factor(&h, src_rect, clip);
                (one, one)
            }
            Prefilter::PerPixel => {
                per_pixel = true;
                (1.0, 1.0)
            }
        };
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
    // A clip wholly on one side of the inverse horizon only samples that side, so that side is drawn
    // (a Perspective Crop low on a converging photo has most of the layer beyond its horizon, #7557).
    let clip_w = clip.map(|c| [(c.x0, c.y0), (c.x1, c.y0), (c.x1, c.y1), (c.x0, c.y1)].map(|(x, y)| inv.w(x as f64, y as f64)));
    let clip_side = clip_w.and_then(|cw| [1.0, -1.0].into_iter().find(|&s| cw.iter().all(|&w| s * w > 0.0)));
    let front =
        side.is_none().then(|| clip_side.unwrap_or_else(|| h.front_side([src_rect.x0 as f64, src_rect.y0 as f64, src_rect.x1 as f64, src_rect.y1 as f64])));
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
    if per_pixel {
        let cells = std::array::from_fn(|a| std::array::from_fn(|b| scaled_rect(src_rect, octave_scale(a), octave_scale(b))));
        let source_only = !tuning.plan_pixels && side.is_some() && samples_source_only(&h, src_rect);
        let g = Geometry { h, inv, src_rect, side, w_lo, front, cells, source_only };
        // The valve's fallback, today's one-factor path, from the source as converted above: the
        // per-pixel path reduces nothing first, so `h` and `src_rect` are still the caller's.
        let Some(done) = warp_per_pixel(src_ref, &g, &tiles, ctl, tuning)? else {
            return warp_with(src_ref, src_rect, &h, interp, clip, ctl, Tuning { force_valve: true, ..tuning });
        };
        if ctl.cancelled() {
            return Ok(out);
        }
        for (r, v) in done {
            out.try_write_region(r, &v)?;
        }
        out.prune();
        return Ok(out);
    }
    // Tiles finished so far, for progress reported per tile (from any worker thread).
    let finished = std::sync::atomic::AtomicUsize::new(0);
    let total = tiles.len().max(1);
    let work = |t: &Rect| -> Result<Option<(Rect, Vec<f32>)>, photocraft_raster::AllocationError> {
        if ctl.cancelled() {
            return Ok(None);
        }
        let k = finished.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        // Short of 100 %: the result is still pruned and handed back after the last tile.
        ctl.progress(0.98 * k as f32 / total as f32);
        let foot = footprint(t, &inv, side, w_lo, src_rect);
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
                    // Un-premultiply by the unclamped alpha: bicubic overshoots it past 1 next to
                    // partly transparent pixels, and dividing by the clamped 1 would lighten the
                    // colour by the overshoot (#6203). Integer storage clips on write; float
                    // colours retain their signed/HDR range.
                    outp[o + c] = (acc[c] / acc[a]) as f32;
                }
                outp[o + a] = al as f32;
            }
        }
        Ok(any.then_some((*t, outp)))
    };
    // Each tile is written as soon as it is warped, so only the tiles in flight hold their `f32`
    // pixels: kept until the last one was done, they cost 16 bytes a pixel of an RGBA result on
    // top of the result's own (#7807).
    let out = std::sync::Mutex::new(out);
    let warp_and_write = |tile: &Rect| -> Result<(), photocraft_raster::AllocationError> {
        if let Some((r, v)) = work(tile)? {
            out.lock().unwrap_or_else(std::sync::PoisonError::into_inner).try_write_region(r, &v)?;
        }
        Ok(())
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        tiles.into_par_iter().try_for_each(|tile| warp_and_write(&tile))?;
    }
    #[cfg(target_arch = "wasm32")]
    for tile in tiles {
        warp_and_write(&tile)?;
    }
    if ctl.cancelled() {
        return Ok(Surface::new(fmt));
    }
    let mut out = out.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner);
    out.prune();
    Ok(out)
}

#[cfg(test)]
mod tests;
