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
                let al = acc.get(alpha).copied().unwrap_or(0.0).clamp(0.0, 1.0);
                if al.is_nan() || al <= 0.0 {
                    continue;
                }
                let o = ((y - top.y0) as usize * w + (x - top.x0) as usize) * n;
                let Some(dst) = buf.get_mut(o..o + n) else { continue };
                any = true;
                for (d, s) in dst.iter_mut().zip(&acc).take(alpha) {
                    // Integer storage clips on write; float colours retain their signed/HDR range.
                    *d = (s / al) as f32;
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
        // The last few percent are the writes.
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

    /// The factors of an affine map's prefilter.
    fn axes(p: Prefilter) -> (f64, f64) {
        match p {
            Prefilter::Axes(fx, fy) => (fx, fy),
            Prefilter::PerPixel => panic!("filtered per pixel"),
        }
    }

    /// #4864: the prefilter's factors, per axis. An affine map gives the same pair with or without
    /// a clip, so a clipped warp matches the full one; a perspective Distort is filtered per pixel
    /// (its valve's one factor still at the centre), and so is a map past the horizon; a skew
    /// reduces nothing, and a squash along a slant keeps the one factor; and a map that isn't finite
    /// reduces nothing.
    #[test]
    fn prefilter_scales_are_per_axis_and_finite() {
        let r = Rect::new(0, 0, 64, 64);
        let quad = |q| Homography::rect_to_quad([0.0, 0.0, 64.0, 64.0], q).unwrap();
        assert_eq!(prefilter_scales(&quad([[0.0, 0.0], [64.0, 0.0], [64.0, 16.0], [0.0, 16.0]]), r, None), Prefilter::Axes(1.0, 0.25));
        let eighth = Homography([0.125, 0.0, 24.0, 0.0, 0.125, 30.0, 0.0, 0.0, 1.0]);
        assert_eq!(prefilter_scales(&eighth, r, None), Prefilter::Axes(0.125, 0.125));
        assert_eq!(prefilter_scales(&eighth, r, Some(Rect::new(10, 10, 52, 52))), Prefilter::Axes(0.125, 0.125));
        let trapezoid = quad([[0.0, 0.0], [64.0, 0.0], [48.0, 24.0], [16.0, 24.0]]);
        let one = trapezoid.local_scale(32.0, 32.0);
        assert!(one > 0.4 && one < 0.5, "{one}");
        assert_eq!(prefilter_scales(&trapezoid, r, None), Prefilter::PerPixel);
        assert_eq!(centre_factor(&trapezoid, r, None), one);
        assert_eq!(prefilter_scales(&quad([[0.0, 0.0], [64.0, 0.0], [256.0, 64.0], [192.0, 64.0]]), r, None), Prefilter::Axes(1.0, 1.0));
        // Skewed by 2 and flattened to 5 %, and a rhombus squashed to 25 % along its diagonal: both
        // keep the one factor, √|det J|. A 4× area rule left the skew's warp 5 % along its slant (the
        // one factor 10 %), and the column lengths left the rhombus's 34 % (the one factor 50 %).
        let (fx, fy) = axes(prefilter_scales(&quad([[0.0, 0.0], [64.0, 0.0], [192.0, 3.2], [128.0, 3.2]]), r, None));
        assert!(fx == fy && (fx * fy - 0.05).abs() < 1e-9, "{fx} x {fy}");
        let h = std::f64::consts::FRAC_1_SQRT_2 * 64.0;
        let (fx, fy) = axes(prefilter_scales(&quad([[0.0, 0.0], [h, h / 4.0], [0.0, h / 2.0], [-h, h / 4.0]]), r, None));
        assert!(fx == fy && (fx - 0.5).abs() < 1e-9, "{fx} x {fy}");
        let past = Homography::rect_to_quad([0.0, 0.0, 100.0, 100.0], [[0.0, 0.0], [300.0, 0.0], [60.0, 60.0], [0.0, 300.0]]).unwrap();
        for clip in [None, Some(Rect::new(-400, -400, 700, 700))] {
            assert_eq!(prefilter_scales(&past, Rect::new(0, 0, 100, 100), clip), Prefilter::PerPixel);
            let one = centre_factor(&past, Rect::new(0, 0, 100, 100), clip);
            assert!(one > 0.0 && one <= 1.0, "{clip:?}: {one}");
        }
        assert_eq!(prefilter_scales(&Homography([f64::INFINITY, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]), r, None), Prefilter::Axes(1.0, 1.0));
        for h in [Homography([f64::NAN; 9]), Homography([f64::INFINITY, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0])] {
            for clip in [None, Some(r)] {
                let p = prefilter_scales(&h, r, clip);
                assert!(p == Prefilter::Axes(1.0, 1.0) || (p == Prefilter::PerPixel && centre_factor(&h, r, clip) == 1.0), "{h:?}: {p:?}");
            }
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
            assert_eq!(prefilter_scales(&h, r, None), Prefilter::Axes(one, one), "{tenth} tenths of a degree");
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
                    let (fx, fy) = match prefilter_scales(&h, r, c) {
                        Prefilter::Axes(fx, fy) => (fx, fy),
                        Prefilter::PerPixel => (centre_factor(&h, r, c), centre_factor(&h, r, c)),
                    };
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
            // A strong perspective, the reporter's wedge, and wedges squeezed far below the 1/64
            // the per-pixel filter stops at.
            Homography::rect_to_quad([0.0, 0.0, 40.0, 30.0], [[0.0, 0.0], [12.0, 10.5], [12.0, 19.5], [0.0, 30.0]]).unwrap(),
            Homography::rect_to_quad([0.0, 0.0, 40.0, 30.0], wedge(40.0, 30.0)).unwrap(),
            Homography::rect_to_quad([0.0, 0.0, 40.0, 30.0], [[0.0, 0.0], [0.4, 10.0], [0.4, 10.3], [0.0, 30.0]]).unwrap(),
            Homography::rect_to_quad([0.0, 0.0, 40.0, 30.0], [[0.0, 0.0], [5000.0, 2000.0], [5000.0, 2000.01], [0.0, 30.0]]).unwrap(),
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
            // The reporter's wedge, on its wide side, in the middle and towards its narrow end.
            (wedge(64.0, 64.0), (20, 35)),
            (wedge(64.0, 64.0), (42, 46)),
            (wedge(64.0, 64.0), (53, 51)),
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

    // #4864, perspective maps: each part of the source filtered for its own reduction.

    use photocraft_color::{ColorMode, SampleType};

    /// FNV-1a, so a golden value pins a warp's output bit for bit.
    fn fnv(h: &mut u64, bytes: &[u8]) {
        for b in bytes {
            *h ^= u64::from(*b);
            *h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }

    /// Feeds `o`'s content bounds and the bits of every pixel in them to `h`.
    fn digest(h: &mut u64, o: &Surface) {
        let b = o.content_bounds();
        for v in [b.x0, b.y0, b.x1, b.y1] {
            fnv(h, &v.to_le_bytes());
        }
        if !b.is_empty() {
            for v in o.read_region(b) {
                fnv(h, &v.to_bits().to_le_bytes());
            }
        }
    }

    /// A 64 × 48 source of `fmt`, every channel varying, with a half-transparent band and a
    /// transparent corner when it has alpha.
    fn golden_source(fmt: PixelFormat) -> Surface {
        let n = fmt.channels();
        let mut px = Vec::with_capacity(64 * 48 * n);
        for y in 0..48 {
            for x in 0..64 {
                for c in 0..n {
                    px.push(if fmt.alpha && c == n - 1 {
                        if x + y < 10 {
                            0.0
                        } else if (20..28).contains(&y) {
                            0.5
                        } else {
                            1.0
                        }
                    } else {
                        ((x * 37 + y * 11 + c as i32 * 53) % 97) as f32 / 96.0
                    });
                }
            }
        }
        let mut s = Surface::new(fmt);
        s.write_region(Rect::new(0, 0, 64, 48), &px);
        s
    }

    fn golden_formats() -> Vec<PixelFormat> {
        let mut v = Vec::new();
        for mode in [ColorMode::Grayscale, ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab] {
            for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
                for alpha in [false, true] {
                    v.push(PixelFormat::new(mode, sample, alpha));
                }
            }
        }
        v
    }

    /// The middle half of what `h` makes of `r`, as a clip.
    fn middle_clip(r: Rect, h: &Homography) -> Rect {
        let b = warp_output_bounds(r, h).unwrap_or(r);
        let (w, ht) = (b.width() as i32 / 4, b.height() as i32 / 4);
        Rect::new(b.x0 + w, b.y0 + ht, b.x1 - w, b.y1 - ht)
    }

    /// The reporter's kind of Distort (#4864): a long, nearly vertical left side, converging into
    /// a thin sliver at the bottom right; 1000 × 750 → (50,50), (880,600), (900,650), (60,700).
    fn wedge(w: f64, h: f64) -> [[f64; 2]; 4] {
        [[0.05 * w, h / 15.0], [0.88 * w, 0.8 * h], [0.9 * w, 0.8 * h + h / 15.0], [0.06 * w, 0.8 * h + 2.0 * h / 15.0]]
    }

    /// The left side magnified 2.2× across x, converging to the right: 1000 × 750 → (20,0),
    /// (300,330), (300,420), (0,750).
    fn q6(w: f64, h: f64) -> [[f64; 2]; 4] {
        [[0.02 * w, 0.0], [0.3 * w, 0.44 * h], [0.3 * w, 0.56 * h], [0.0, h]]
    }

    fn trapezoid(w: f64, h: f64) -> [[f64; 2]; 4] {
        [[0.0, 0.0], [w, 0.0], [0.75 * w, 0.375 * h], [0.25 * w, 0.375 * h]]
    }

    /// A near-uniform shrink to about 55 % with a mild perspective: every pixel in the top quarter
    /// of octave 0 on both axes, so it samples four cells, which the valve turns down.
    fn shrink55(w: f64, h: f64) -> [[f64; 2]; 4] {
        [[0.0, 0.0], [0.55 * w, 0.01 * h], [0.555 * w, 0.555 * h], [0.0, 0.55 * h]]
    }

    fn map(q: fn(f64, f64) -> [[f64; 2]; 4], w: i32, h: i32) -> Homography {
        Homography::rect_to_quad([0.0, 0.0, f64::from(w), f64::from(h)], q(f64::from(w), f64::from(h))).unwrap()
    }

    /// The per-pixel path's geometry for `h` over `r` as `warp_with` builds it (for a map whose
    /// source stays in front of its horizon), and the destination tiles it plans.
    fn setup(h: Homography, r: Rect) -> (Geometry, Vec<Rect>) {
        let (x0, y0, x1, y1) = (f64::from(r.x0) - 1.0, f64::from(r.y0) - 1.0, f64::from(r.x1) + 1.0, f64::from(r.y1) + 1.0);
        let fwd_w = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].map(|(x, y)| h.w(x, y));
        let side = [1.0, -1.0].into_iter().find(|&s| fwd_w.iter().all(|&c| s * c > 0.0));
        assert!(side.is_some(), "{h:?} crosses its horizon");
        let w_lo = 1.0 / fwd_w.iter().fold(0.0f64, |m, c| m.max(c.abs()));
        let cells = std::array::from_fn(|a| std::array::from_fn(|b| scaled_rect(r, octave_scale(a), octave_scale(b))));
        let g = Geometry { h, inv: h.inverse().unwrap(), src_rect: r, side, w_lo, front: None, cells, source_only: false };
        let dst = warp_output_bounds(r, &h).unwrap();
        (g, dst.tiles().map(|tc| tc.rect().intersect(&dst)).filter(|t| !t.is_empty()).collect())
    }

    /// `w` × `h` opaque RGB of `sample` depth whose pixel (x, y) is `f(x, y)`.
    fn paint(w: i32, h: i32, sample: SampleType, f: impl Fn(i32, i32) -> [f32; 4]) -> Surface {
        let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, sample, true));
        let f = &f;
        let px: Vec<f32> = (0..h).flat_map(|y| (0..w).flat_map(move |x| f(x, y))).collect();
        s.write_region(Rect::new(0, 0, w, h), &px);
        s
    }

    /// The reporter's kind of test card, 1000 × 750: white, a 4 px black border, and 50 px red
    /// and blue bars across rows 150..600.
    fn test_card() -> Surface {
        paint(1000, 750, SampleType::U8, |x, y| {
            if x < 4 || y < 4 || x >= 996 || y >= 746 {
                [0.0, 0.0, 0.0, 1.0]
            } else if (150..600).contains(&y) {
                if (x / 50) % 2 == 0 { [1.0, 0.2, 0.2, 1.0] } else { [0.2, 0.2, 1.0, 1.0] }
            } else {
                [1.0, 1.0, 1.0, 1.0]
            }
        })
    }

    /// Where along row `y` (over `xs`) the source column `ue` lands.
    fn crossing(inv: &Homography, y: i32, xs: std::ops::Range<i32>, ue: f64) -> f64 {
        let us: Vec<(i32, f64)> = xs.map(|x| (x, inv.apply(f64::from(x) + 0.5, f64::from(y) + 0.5).0)).collect();
        us.windows(2).find(|w| (w[0].1 - ue) * (w[1].1 - ue) <= 0.0).map_or(f64::NAN, |w| f64::from(w[0].0) + 0.5 + (ue - w[0].1) / (w[1].1 - w[0].1))
    }

    /// An edge in a row profile `(x, value)` within ±45 px of `xe`: its 10-90 % width in pixels,
    /// between the profile's own extremes there (normalised so `lo` is 0 and `hi` is 1).
    fn edge_width(prof: &[(i32, f64)], xe: f64, lo: f64, hi: f64) -> f64 {
        let n: Vec<(i32, f64)> = prof.iter().filter(|(x, _)| (f64::from(*x) - xe).abs() <= 45.0).map(|&(x, v)| (x, (v - lo) / (hi - lo))).collect();
        let mn = n.iter().map(|p| p.1).fold(f64::MAX, f64::min);
        let mx = n.iter().map(|p| p.1).fold(f64::MIN, f64::max);
        let cross = |level: f64| {
            let lv = mn + level * (mx - mn);
            n.windows(2)
                .find(|w| (w[0].1 - lv) * (w[1].1 - lv) <= 0.0 && w[0].1 != w[1].1)
                .map_or(f64::NAN, |w| f64::from(w[0].0) + (lv - w[0].1) / (w[1].1 - w[0].1))
        };
        (cross(0.9) - cross(0.1)).abs()
    }

    /// The edge across the row of `h`'s image of source row `v0` where source column `ue` lands:
    /// its 10-90 % width in `bic` and in `bil`.
    fn row_edge(h: &Homography, bic: &Surface, bil: &Surface, v0: f64, ue: f64, lo: f64, hi: f64) -> (f64, f64) {
        let inv = h.inverse().unwrap();
        let (xd, yd) = h.apply(0.0, v0);
        let (y, x0) = (yd.floor() as i32, xd.floor() as i32);
        let xs = (x0 - 10)..(x0 + 400);
        let row = |s: &Surface| -> Vec<(i32, f64)> { xs.clone().map(|x| (x, f64::from(s.pixel(x, y)[0]))).collect() };
        let xe = crossing(&inv, y, xs.clone(), ue);
        (edge_width(&row(bic), xe, lo, hi), edge_width(&row(bil), xe, lo, hi))
    }

    /// #4864: the reporter's Distort. Its long left side magnifies the source across x, so the black
    /// border should stay about as crisp as a direct (Bilinear) sample. One factor for the whole
    /// warp, taken at the centre where the source shrinks, blurred it into a 49-63 px ramp that
    /// only darkened to 0.34 and softened the outline to a 41 px alpha ramp.
    #[test]
    fn perspective_wide_side_is_sharp() {
        let card = test_card();
        let r = Rect::new(0, 0, 1000, 750);
        let h = map(wedge, 1000, 750);
        let inv = h.inverse().unwrap();
        let bic = warp_surface(&card, r, &h, Interp::Bicubic).unwrap();
        let bil = warp_surface(&card, r, &h, Interp::Bilinear).unwrap();
        for v0 in [80.0, 375.0, 680.0] {
            let (wb, wl) = row_edge(&h, &bic, &bil, v0, 4.0, 0.0, 1.0);
            let (xd, yd) = h.apply(0.0, v0);
            let (y, x0) = (yd.floor() as i32, xd.floor() as i32);
            let outline = crossing(&inv, y, (x0 - 10)..(x0 + 400), 0.0);
            let alpha3 = bic.pixel((outline + 3.0).floor() as i32, y)[3];
            // The border's darkest value, over the pixels inside the outline: a transparent pixel's
            // colour reads 0 and says nothing.
            let darkest = ((x0 - 10)..(x0 + 400))
                .filter(|&x| (0.0..45.0).contains(&(f64::from(x) - outline)) && bic.pixel(x, y)[3] > 0.5)
                .map(|x| f64::from(bic.pixel(x, y)[0]))
                .fold(1.0f64, f64::min);
            let ramp = ((x0 - 10)..(x0 + 400))
                .filter(|&x| (f64::from(x) - outline).abs() < 40.0)
                .filter(|&x| {
                    let a = bic.pixel(x, y)[3];
                    a > 0.05 && a < 0.95
                })
                .count();
            assert!(
                wb <= 1.25 * wl && darkest <= 0.05 && alpha3 >= 0.98 && ramp <= 3,
                "row at source v = {v0}: border 10-90 width {wb:.1} px (Bilinear {wl:.1}), darkest {darkest:.3}, alpha 3 px inside {alpha3:.3}, alpha ramp {ramp} px"
            );
        }
    }

    /// #4864: a Distort whose left side magnifies the source 2.2× across x keeps the border and the
    /// bars there about as sharp as Bilinear does (the one factor left the border 20 px wide and
    /// the first bar edge 10 px).
    #[test]
    fn perspective_q6_bars_are_sharp() {
        let card = test_card();
        let r = Rect::new(0, 0, 1000, 750);
        let h = map(q6, 1000, 750);
        let bic = warp_surface(&card, r, &h, Interp::Bicubic).unwrap();
        let bil = warp_surface(&card, r, &h, Interp::Bilinear).unwrap();
        for (v0, ue, lo, hi) in [(80.0, 4.0, 0.0, 1.0), (375.0, 4.0, 0.0, 1.0), (680.0, 4.0, 0.0, 1.0), (375.0, 50.0, 1.0, 0.2)] {
            let (wb, wl) = row_edge(&h, &bic, &bil, v0, ue, lo, hi);
            assert!(wb <= 1.25 * wl, "edge at source u = {ue}, v = {v0}: 10-90 width {wb:.1} px (Bilinear {wl:.1})");
        }
    }

    /// The fine patterns the aliasing tests warp: 1 px checks and stripes, 16 and 32 px gratings
    /// and a zone plate.
    type Pattern = (&'static str, fn(i32, i32) -> f32);

    fn patterns() -> [Pattern; 7] {
        [
            ("checker1", |x, y| ((x + y) & 1) as f32),
            ("vstripe2", |x, _| (x & 1) as f32),
            ("hstripe2", |_, y| (y & 1) as f32),
            ("vgrat16", |x, _| ((x >> 3) & 1) as f32),
            ("hgrat16", |_, y| ((y >> 3) & 1) as f32),
            ("vgrat32", |x, _| ((x >> 4) & 1) as f32),
            ("zone", |x, y| {
                let (dx, dy) = (f64::from(x) - 499.5, f64::from(y) - 374.5);
                (0.5 + 0.5 * (std::f64::consts::PI * (dx * dx + dy * dy) / 1250.0).cos()) as f32
            }),
        ]
    }

    /// Today's path: one factor for the whole perspective warp (what the valve falls back to).
    fn warp_today(src: &Surface, r: Rect, h: &Homography) -> Surface {
        warp_with(src, r, h, Interp::Bicubic, None, &Interrupt::NONE, Tuning { force_valve: true, ..Tuning::DEFAULT }).unwrap()
    }

    /// One map's and pattern's rmse against a box-filtered reference, per region of the source
    /// (wide, middle, narrow): for this warp and for today's.
    struct Aliasing {
        map: &'static str,
        pattern: &'static str,
        now: [f64; 3],
        today: [f64; 3],
    }

    /// The rmse of the red channel against an adaptive box supersample of the bilinearly
    /// interpolated source (8 to 64 samples per axis, 4 per source pixel of footprint), over the
    /// source's interior (24 px margin), by region: below 15 %, 30-80 % and from 80 % of the width
    /// (the wedge and Q6) or the height (the trapezoid).
    fn aliasing_table() -> Vec<Aliasing> {
        use rayon::prelude::*;
        let (w, ht) = (1000, 750);
        let r = Rect::new(0, 0, w, ht);
        let mut table = Vec::new();
        for (name, q) in [("wedge", wedge as fn(f64, f64) -> [[f64; 2]; 4]), ("q6", q6), ("trapezoid", trapezoid)] {
            let h = map(q, w, ht);
            let inv = h.inverse().unwrap();
            let along_v = name == "trapezoid";
            for (pattern, pf) in patterns() {
                let src = paint(w, ht, SampleType::U8, |x, y| {
                    let v = pf(x, y);
                    [v, v, v, 1.0]
                });
                let (now, today) = (warp_surface(&src, r, &h, Interp::Bicubic).unwrap(), warp_today(&src, r, &h));
                let b = now.content_bounds().union(&today.content_bounds());
                let (pn, pt) = (now.read_region(b), today.read_region(b));
                let bw = b.width() as usize;
                let sample = |x: f64, y: f64| -> f64 {
                    let (u, v) = inv.apply(x, y);
                    let (u, v) = (u - 0.5, v - 0.5);
                    let (ix, iy) = (u.floor(), v.floor());
                    let (fx, fy) = (u - ix, v - iy);
                    let (ix, iy) = (ix as i32, iy as i32);
                    let g = |x: i32, y: i32| f64::from(pf(x.clamp(0, w - 1), y.clamp(0, ht - 1)));
                    (g(ix, iy) * (1.0 - fx) + g(ix + 1, iy) * fx) * (1.0 - fy) + (g(ix, iy + 1) * (1.0 - fx) + g(ix + 1, iy + 1) * fx) * fy
                };
                let acc = (b.y0..b.y1)
                    .into_par_iter()
                    .map(|y| {
                        let mut acc = [[0.0f64; 3]; 3];
                        for x in b.x0..b.x1 {
                            let (fx, fy) = (f64::from(x), f64::from(y));
                            let (u, v) = inv.apply(fx + 0.5, fy + 0.5);
                            if !(u >= 24.0 && u <= f64::from(w) - 24.0 && v >= 24.0 && v <= f64::from(ht) - 24.0) {
                                continue;
                            }
                            let (c, l) = if along_v { (v, f64::from(ht)) } else { (u, f64::from(w)) };
                            let reg = if c < 0.15 * l {
                                0
                            } else if (0.3 * l..0.8 * l).contains(&c) {
                                1
                            } else if c >= 0.8 * l {
                                2
                            } else {
                                continue;
                            };
                            let (u1, v1) = inv.apply(fx + 1.5, fy + 0.5);
                            let (u2, v2) = inv.apply(fx + 0.5, fy + 1.5);
                            let foot = ((u1 - u).abs() + (u2 - u).abs()).max((v1 - v).abs() + (v2 - v).abs());
                            let ns = ((4.0 * foot).ceil() as usize).clamp(8, 64);
                            let mut s = 0.0;
                            for j in 0..ns {
                                for i in 0..ns {
                                    s += sample(fx + (i as f64 + 0.5) / ns as f64, fy + (j as f64 + 0.5) / ns as f64);
                                }
                            }
                            let reference = s / (ns * ns) as f64;
                            let k = ((y - b.y0) as usize * bw + (x - b.x0) as usize) * 4;
                            acc[reg][0] += 1.0;
                            acc[reg][1] += (f64::from(pn[k]) - reference).powi(2);
                            acc[reg][2] += (f64::from(pt[k]) - reference).powi(2);
                        }
                        acc
                    })
                    .reduce(|| [[0.0f64; 3]; 3], |a, b| std::array::from_fn(|i| std::array::from_fn(|j| a[i][j] + b[i][j])));
                let rmse = |k: usize| std::array::from_fn(|reg| (acc[reg][k] / acc[reg][0].max(1.0)).sqrt());
                table.push(Aliasing { map: name, pattern, now: rmse(1), today: rmse(2) });
            }
        }
        for a in &table {
            println!(
                "{:9} {:9} wide {:.3}/{:.3} middle {:.3}/{:.3} narrow {:.3}/{:.3}",
                a.map, a.pattern, a.now[0], a.today[0], a.now[1], a.today[1], a.now[2], a.today[2]
            );
        }
        table
    }

    /// #4864: where a perspective map shrinks the source most (the far, narrow side), fine patterns
    /// alias no more than with today's one factor, and much less on average: today's factor is set
    /// at the centre, so it filtered the narrow side for less than its reduction. The wide side and
    /// the middle, filtered for their own (smaller) reductions, alias no more than today either.
    /// One test, so the table is computed once: nextest runs each test in its own process.
    #[test]
    fn perspective_aliases_no_more_than_today() {
        let table = aliasing_table();
        for name in ["wedge", "q6", "trapezoid"] {
            let rows: Vec<&Aliasing> = table.iter().filter(|a| a.map == name).collect();
            for a in &rows {
                assert!(a.now[2] <= a.today[2] + 0.005, "{name} {}: narrow-side rmse {:.3} vs today's {:.3}", a.pattern, a.now[2], a.today[2]);
            }
            let (now, today) = (rows.iter().map(|a| a.now[2]).sum::<f64>() / 7.0, rows.iter().map(|a| a.today[2]).sum::<f64>() / 7.0);
            assert!(now <= 0.85 * today, "{name}: mean narrow-side rmse {now:.3} vs today's {today:.3}");
        }
        for a in &table {
            for (reg, region) in [(0, "wide"), (1, "middle")] {
                assert!(a.now[reg] <= a.today[reg] + 0.01, "{} {} {region}: rmse {:.3} vs today's {:.3}", a.map, a.pattern, a.now[reg], a.today[reg]);
            }
        }
    }

    /// The source point of destination pixel (x, y) lies in the interior of `r` (24 px margin).
    fn interior(inv: &Homography, r: Rect, x: i32, y: i32) -> bool {
        let (u, v) = inv.apply(f64::from(x) + 0.5, f64::from(y) + 0.5);
        u >= f64::from(r.x0) + 24.0 && u <= f64::from(r.x1) - 24.0 && v >= f64::from(r.y0) + 24.0 && v <= f64::from(r.y1) - 24.0
    }

    /// A pixel and its |second difference|.
    type D2 = (i32, i32, f64);

    /// The largest |second difference| of the pixels `near` picks, and the 99.9th percentile of the
    /// others.
    fn near_max_and_p999(d: &[D2], near: impl Fn(&D2) -> bool) -> (f64, f64) {
        let (near, rest): (Vec<&D2>, Vec<&D2>) = d.iter().partition(|p| near(p));
        let mut rest: Vec<f64> = rest.iter().map(|p| p.2).collect();
        rest.sort_by(f64::total_cmp);
        let p999 = rest.get(((rest.len() as f64 - 1.0) * 0.999) as usize).copied().unwrap_or(0.0);
        (near.iter().map(|p| p.2).fold(0.0f64, f64::max), p999)
    }

    /// |second difference| of the red and green channels along rows and columns at each pixel
    /// whose four neighbours lie in the interior of `r`.
    fn second_differences(o: &Surface, inv: &Homography, r: Rect) -> Vec<(i32, i32, f64)> {
        let b = o.content_bounds();
        let px = o.read_region(b);
        let bw = b.width() as usize;
        let at = |x: i32, y: i32, c: usize| f64::from(px[((y - b.y0) as usize * bw + (x - b.x0) as usize) * 4 + c]);
        let mut out = Vec::new();
        for y in b.y0 + 1..b.y1 - 1 {
            for x in b.x0 + 1..b.x1 - 1 {
                if [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)].iter().all(|&(x, y)| interior(inv, r, x, y)) {
                    let d = (0..2)
                        .flat_map(|c| {
                            [(at(x - 1, y, c) - 2.0 * at(x, y, c) + at(x + 1, y, c)).abs(), (at(x, y - 1, c) - 2.0 * at(x, y, c) + at(x, y + 1, c)).abs()]
                        })
                        .fold(0.0f64, f64::max);
                    out.push((x, y, d));
                }
            }
        }
        out
    }

    /// #4864: a smooth gradient stays smooth under a perspective map. There is no seam next to the
    /// destination tiles, where windows and cells change, no more curvature than today's one factor
    /// leaves, give or take half, and the faint bands next to octave changes don't grow. The
    /// reporter's wedge curves most at its narrow tip, where the octave cells' filters reach the
    /// source's edge: 2.9e-3 at F32 against today's 2.2e-3 and Bilinear's 7.1e-4.
    #[test]
    fn perspective_ramp_is_smooth() {
        let r = Rect::new(0, 0, 1000, 750);
        for sample in [SampleType::F32, SampleType::U16] {
            let src = paint(1000, 750, sample, |x, y| [(x as f32 + 0.5) / 1000.0, (y as f32 + 0.5) / 750.0, 0.5, 1.0]);
            for (name, q) in [("wedge", wedge as fn(f64, f64) -> [[f64; 2]; 4]), ("trapezoid", trapezoid)] {
                let h = map(q, 1000, 750);
                let inv = h.inverse().unwrap();
                let now = second_differences(&warp_surface(&src, r, &h, Interp::Bicubic).unwrap(), &inv, r);
                let today = second_differences(&warp_today(&src, r, &h), &inv, r);
                let max = |d: &[(i32, i32, f64)]| d.iter().map(|p| p.2).fold(0.0f64, f64::max);
                assert!(max(&now) <= 1.5 * max(&today), "{sample:?} {name}: max |second difference| {:.2e} vs today's {:.2e}", max(&now), max(&today));
                let tile_edge = |&(x, y, _): &D2| [x, y].iter().any(|&c| c.rem_euclid(TILE).min(TILE - c.rem_euclid(TILE)) <= 2);
                let (seam, p999) = near_max_and_p999(&now, tile_edge);
                assert!(seam <= 1.5 * p999, "{sample:?} {name}: |second difference| next to tile edges {seam:.2e} vs p99.9 elsewhere {p999:.2e}");
                // Within 2 px of where a pixel's octave or cross-fade changes, switching cells leaves
                // faint bands: off the narrow side (the last 20 %, where the tip's curvature
                // dominates), 2.2× the p99.9 elsewhere under the wedge and 1.3× under the
                // trapezoid. Held there, so they can't grow.
                let (g, _) = setup(h, r);
                let key = |x: i32, y: i32| pixel_lod(&g, x, y).map(|l| (l.ax.lv[0], l.ax.n, l.ay.lv[0], l.ay.n));
                let change = |&(x, y, _): &D2| (-2..=2).any(|dy| (-2..=2).any(|dx| key(x + dx, y + dy) != key(x, y)));
                let narrow = |&(x, y, _): &D2| {
                    let (u, v) = inv.apply(f64::from(x) + 0.5, f64::from(y) + 0.5);
                    if name == "trapezoid" { v >= 600.0 } else { u >= 800.0 }
                };
                let rest: Vec<D2> = now.iter().copied().filter(|p| !narrow(p)).collect();
                let (bands, p999) = near_max_and_p999(&rest, change);
                let bound = if name == "wedge" { 2.5 } else { 1.6 };
                assert!(bands <= bound * p999, "{sample:?} {name}: |second difference| next to octave changes {bands:.2e} vs p99.9 elsewhere {p999:.2e}");
            }
        }
    }

    const TILE: i32 = 256;

    /// #4864: the filtering depends only on where each pixel's source point is, not on the tiles it
    /// falls in: moving the quad by (37, 101) moves the result by exactly that.
    #[test]
    fn perspective_is_translation_equivariant() {
        let card = test_card();
        let ramp = paint(1000, 750, SampleType::F32, |x, y| [(x as f32 + 0.5) / 1000.0, (y as f32 + 0.5) / 750.0, 0.5, 1.0]);
        let r = Rect::new(0, 0, 1000, 750);
        for src in [&card, &ramp] {
            for q in [wedge as fn(f64, f64) -> [[f64; 2]; 4], q6] {
                let quad = q(1000.0, 750.0);
                let h = Homography::rect_to_quad([0.0, 0.0, 1000.0, 750.0], quad).unwrap();
                let moved = Homography::rect_to_quad([0.0, 0.0, 1000.0, 750.0], quad.map(|[x, y]| [x + 37.0, y + 101.0])).unwrap();
                let (a, b) = (warp_surface(src, r, &h, Interp::Bicubic).unwrap(), warp_surface(src, r, &moved, Interp::Bicubic).unwrap());
                let ab = a.content_bounds();
                assert_eq!(ab.translate(37, 101), b.content_bounds());
                let (pa, pb) = (a.read_region(ab), b.read_region(ab.translate(37, 101)));
                let differ = pa.iter().zip(&pb).filter(|(x, y)| x.to_bits() != y.to_bits()).count();
                let worst = pa.iter().zip(&pb).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
                assert_eq!(differ, 0, "{:?}: {differ} of {} values differ, by up to {worst:e}", src.format().sample, pa.len());
            }
        }
    }

    /// Hashes of today's outputs (28d35d2c): affine Bicubic warps (8 maps, Gray/RGB/CMYK/Lab at
    /// 8/16/32 bits with and without alpha, with and without a clip), Bilinear and Nearest
    /// perspective warps, and today's perspective Bicubic (the valve's path).
    const GOLDEN: [(&str, u64); 11] = [
        ("50%", 0x01da_1b84_9529_8f40),
        ("100x25", 0xff63_845c_b1e2_2339),
        ("87.5x25", 0x9047_1420_a2c4_0128),
        ("rhombus", 0x26c1_dbfe_5add_e94f),
        ("skew-flatten", 0xc1b6_bec3_d847_a18d),
        ("translation", 0xbdae_ae89_9c65_a684),
        ("eighth", 0x419d_7a4c_76ec_05b4),
        ("rotate30", 0xda82_a3eb_d6eb_35ca),
        ("perspective bilinear", 0xc7c0_1c32_b56a_fbd7),
        ("perspective nearest", 0x58db_c5fd_901c_3b1a),
        ("perspective bicubic today", 0xb1eb_cdac_c0ca_4dea),
    ];

    fn golden_maps() -> Vec<(&'static str, Homography)> {
        let r = [0.0, 0.0, 64.0, 48.0];
        let quad = |q| Homography::rect_to_quad(r, q).unwrap();
        let k = std::f64::consts::FRAC_1_SQRT_2 * 64.0;
        let (s, c) = (30f64.to_radians().sin(), 30f64.to_radians().cos());
        vec![
            ("50%", quad([[0.0, 0.0], [32.0, 0.0], [32.0, 24.0], [0.0, 24.0]])),
            ("100x25", quad([[0.0, 0.0], [64.0, 0.0], [64.0, 12.0], [0.0, 12.0]])),
            ("87.5x25", quad([[0.0, 0.0], [56.0, 0.0], [56.0, 12.0], [0.0, 12.0]])),
            ("rhombus", quad([[0.0, 0.0], [k, k / 4.0], [0.0, k / 2.0], [-k, k / 4.0]])),
            ("skew-flatten", quad([[0.0, 0.0], [64.0, 0.0], [192.0, 2.4], [128.0, 2.4]])),
            ("translation", Homography([1.0, 0.0, 7.25, 0.0, 1.0, -3.5, 0.0, 0.0, 1.0])),
            ("eighth", Homography([0.125, 0.0, 24.0, 0.0, 0.125, 30.0, 0.0, 0.0, 1.0])),
            ("rotate30", Homography([c, -s, 40.0 - c * 32.0 + s * 24.0, s, c, 30.0 - s * 32.0 - c * 24.0, 0.0, 0.0, 1.0])),
        ]
    }

    /// The hashes [`GOLDEN`] pins, computed with `bicubic_perspective` for the last one.
    fn golden_hashes(bicubic_perspective: impl Fn(&Surface, Rect, &Homography, Option<Rect>) -> Surface) -> Vec<(&'static str, u64)> {
        let r = Rect::new(0, 0, 64, 48);
        let sources: Vec<Surface> = golden_formats().into_iter().map(golden_source).collect();
        let mut out = Vec::new();
        for (name, h) in golden_maps() {
            let mut hash = 0xcbf2_9ce4_8422_2325;
            for s in &sources {
                digest(&mut hash, &warp_surface(s, r, &h, Interp::Bicubic).unwrap());
                digest(&mut hash, &warp_surface_clipped(s, r, &h, Interp::Bicubic, middle_clip(r, &h)).unwrap());
            }
            out.push((name, hash));
        }
        let perspective = [map(wedge, 64, 48), map(trapezoid, 64, 48)];
        for (name, interp) in
            [("perspective bilinear", Some(Interp::Bilinear)), ("perspective nearest", Some(Interp::Nearest)), ("perspective bicubic today", None)]
        {
            let mut hash = 0xcbf2_9ce4_8422_2325;
            for s in &sources {
                for h in &perspective {
                    for clip in [None, Some(middle_clip(r, h))] {
                        let o = match (interp, clip) {
                            (Some(i), None) => warp_surface(s, r, h, i).unwrap(),
                            (Some(i), Some(c)) => warp_surface_clipped(s, r, h, i, c).unwrap(),
                            (None, c) => bicubic_perspective(s, r, h, c),
                        };
                        digest(&mut hash, &o);
                    }
                }
            }
            out.push((name, hash));
        }
        out
    }

    /// #4864: affine Bicubic warps, and Bilinear and Nearest ones of any map, are exactly what they
    /// were before perspective maps were filtered per pixel.
    #[test]
    fn golden_affine_bytes() {
        let got = golden_hashes(|s, r, h, clip| match clip {
            None => warp_surface(s, r, h, Interp::Bicubic).unwrap(),
            Some(c) => warp_surface_clipped(s, r, h, Interp::Bicubic, c).unwrap(),
        });
        let mut bad = Vec::new();
        for ((name, want), (_, hash)) in GOLDEN.iter().zip(&got).take(10) {
            if want != hash {
                bad.push(format!("{name}: {hash:#018x} (want {want:#018x})"));
            }
        }
        println!("{got:#018x?}");
        assert!(bad.is_empty(), "{bad:#?}");
    }

    /// #4864: forcing the valve on a perspective map gives exactly today's (28d35d2c) output.
    #[test]
    fn valve_reproduces_today() {
        let forced = Tuning { force_valve: true, ..Tuning::DEFAULT };
        let got = golden_hashes(|s, r, h, clip| warp_with(s, r, h, Interp::Bicubic, clip, &Interrupt::NONE, forced).unwrap());
        let (name, want) = GOLDEN[10];
        assert_eq!(got.get(10), Some(&(name, want)), "{got:#018x?}");
    }

    /// #4864: when the valve turns the cells down, the warp is today's, bit for bit, through the
    /// valve's own fallback, for a layer with alpha and for one converted once to gain it. The
    /// floor is lowered so this small layer trips it; at the default floor the same map is
    /// filtered per pixel.
    #[test]
    fn the_valve_falls_back_to_todays_output() {
        let (w, hh) = (400, 300);
        let r = Rect::new(0, 0, w, hh);
        let h = map(shrink55, w, hh);
        let low = Tuning { valve_floor: 0.0, ..Tuning::DEFAULT };
        let formats = [(SampleType::U8, true), (SampleType::F32, true), (SampleType::U16, false)];
        for fmt in formats.map(|(sample, alpha)| PixelFormat::new(ColorMode::Rgb, sample, alpha)) {
            let src = paint(w, hh, fmt.sample, |x, y| {
                let v = ((x * 7 + y * 3) % 23) as f32 / 22.0;
                [v, ((x / 3) & 1) as f32, ((y / 2) & 1) as f32, 1.0]
            })
            .try_convert(fmt)
            .unwrap();
            let today = warp_today(&src, r, &h);
            let fell_back = warp_with(&src, r, &h, Interp::Bicubic, None, &Interrupt::NONE, low).unwrap();
            assert!(fell_back == today, "{fmt:?}: the valve's fallback isn't today's output");
            assert!(warp_surface(&src, r, &h, Interp::Bicubic).unwrap() != today, "{fmt:?}: the map isn't filtered per pixel");
        }
    }

    /// #4864: the valve's probe, every 8th pixel each way, plans windows inside the full plan's, so
    /// it never reads or holds more: cells the valve turns down for the probe, it turns down for the
    /// full plan too. A near-uniform 55 % shrink is turned down from the probe alone.
    #[test]
    fn the_valve_probe_plans_inside_the_full_plan() {
        let (w, hh) = (400, 300);
        let r = Rect::new(0, 0, w, hh);
        let src = paint(w, hh, SampleType::U8, |x, y| [((x * 7 + y * 3) % 23) as f32 / 22.0, 0.5, 0.5, 1.0]);
        let low = Tuning { valve_floor: 0.0, ..Tuning::DEFAULT };
        let maps = [("wedge", wedge as fn(f64, f64) -> [[f64; 2]; 4]), ("q6", q6), ("trapezoid", trapezoid), ("shrink55", shrink55)];
        for (name, q) in maps {
            let (g, tiles) = setup(map(q, w, hh), r);
            let plan = |step| build_jobs(&plan_tiles(&tiles, &g, &src, &Interrupt::NONE, step));
            let (probe, full) = (plan(PROBE_STEP), plan(1));
            assert!(!probe.is_empty(), "{name}: the probe reads no cell");
            for (k, p) in &probe {
                assert!(full.get(k).is_some_and(|f| f.union(p) == *f), "{name} {k:?}: the probe's {p:?} lies outside the full plan's {:?}", full.get(k));
            }
            if name == "shrink55" {
                assert!(refuses(&probe, &g, low) && refuses(&full, &g, low), "{name}: not turned down");
            }
        }
    }

    /// The per-axis rule exactly as `prefilter_scales` computed it before it was shared with the
    /// per-pixel path.
    fn factors_before(j: [f64; 4]) -> (f64, f64) {
        let snap = |f: f64| if f > 0.0 && f < 0.999 { f.max(1.0 / 64.0) } else { 1.0 };
        let (fx, fy) = (snap(j[0].hypot(j[2])), snap(j[1].hypot(j[3])));
        let (hi, lo) = singular_values(j);
        let (_, left) = singular_values([j[0] / fx, j[1] / fy, j[2] / fx, j[3] / fy]);
        let target = if hi > 0.0 { (lo / hi).sqrt().min(0.5) } else { 0.0 };
        let c = if target > 0.0 && left.is_finite() { left / target } else { 0.0 };
        if c < 1.0 { (snap(fx * c), snap(fy * c)) } else { (fx, fy) }
    }

    /// #4864: the shared rule gives affine maps bit for bit the factors they had: slants, a
    /// collapse, NaN, infinities and zeros included.
    #[test]
    fn axis_factors_matches_prefilter_scales() {
        let k = std::f64::consts::FRAC_1_SQRT_2;
        let js = [
            [1.0, 0.0, 0.0, 0.25],
            [0.875, 0.0, 0.0, 0.25],
            [0.5, 0.0, 0.0, 0.5],
            [k, -k / 2.0, k / 4.0, k / 4.0],
            [1.0, 2.0, 0.0, 0.05],
            [0.125, 0.0, 0.0, 0.125],
            [1.0, 1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0, 0.0],
            [1e-300, 0.0, 0.0, 1e-300],
            [1e300, 0.0, 0.0, 1e300],
            [f64::NAN, 0.0, 0.0, 1.0],
            [f64::INFINITY, 0.0, 0.0, 1.0],
            [0.3, -0.7, 0.2, 0.9],
            [0.999, 0.0, 0.0, 0.998],
        ];
        for j in js {
            let (a, b) = (axis_factors(j, snap, f64::hypot), factors_before(j));
            assert!(a.0.to_bits() == b.0.to_bits() && a.1.to_bits() == b.1.to_bits(), "{j:?}: {a:?} vs {b:?}");
            if j.iter().all(|v| v.is_finite()) {
                let h = Homography([j[0], j[1], 3.0, j[2], j[3], -2.0, 0.0, 0.0, 1.0]);
                if let Prefilter::Axes(fx, fy) = prefilter_scales(&h, Rect::new(0, 0, 64, 64), None) {
                    assert!(fx.to_bits() == b.0.to_bits() && fy.to_bits() == b.1.to_bits(), "{j:?}: {fx} x {fy} vs {b:?}");
                }
            }
            let (fx, fy) = axis_factors(j, clamp_factor, column_length);
            assert!((1.0 / 64.0..=1.0).contains(&fx) && (1.0 / 64.0..=1.0).contains(&fy), "{j:?}: {fx} x {fy}");
        }
    }

    /// The per-level weights and kernel widenings of an [`Axis`].
    fn spectrum(a: &Axis) -> [(f64, f64); 7] {
        let mut s = [(0.0, 0.0); 7];
        for (l, r, w) in a.entries() {
            s[l] = (s[l].0 + w, r);
        }
        s
    }

    /// #4864: a pixel's filter changes continuously with its reduction: in steps of 10⁻⁴ octave, no
    /// level's weight moves by more than 10⁻³, and a level's kernel widening jumps only where its
    /// weight is (nearly) 0. Values that aren't finite or positive reduce nothing, or the most.
    #[test]
    fn octave_is_continuous() {
        let mut prev = spectrum(&octave(1.0));
        for k in 1..=70_000 {
            let f = (-(k as f64) * 1e-4).exp2();
            let a = octave(f);
            let total: f64 = a.entries().map(|e| e.2).sum();
            assert!((total - 1.0).abs() < 1e-12 && a.lv.iter().all(|&l| l <= MAX_LEVEL), "{f}: {a:?}");
            assert!(a.r.iter().all(|r| (1.0..2.0).contains(r)), "{f}: {a:?}");
            let s = spectrum(&a);
            for (l, (p, c)) in prev.iter().zip(&s).enumerate() {
                assert!((p.0 - c.0).abs() <= 1e-3, "f = {f}, level {l}: weight {} -> {}", p.0, c.0);
                if p.0 > 1e-3 && c.0 > 1e-3 {
                    assert!((p.1 - c.1).abs() <= 1e-3, "f = {f}, level {l}: r {} -> {}", p.1, c.1);
                }
            }
            prev = s;
        }
        for (f, level) in [(f64::NAN, 0), (f64::INFINITY, 0), (-1.0, 0), (0.0, MAX_LEVEL), (1e-300, MAX_LEVEL), (5.0, 0)] {
            let a = octave(f);
            assert!(a.n == 1 && a.lv[0] == level && a.r[0] == 1.0 && a.w[0] == 1.0, "{f}: {a:?}");
        }
    }

    /// #4864: a pixel's filter for a map that isn't finite, collapses or is huge either skips the
    /// pixel or stays in range.
    #[test]
    fn pixel_lod_survives_hostile_maps() {
        let r = Rect::new(0, 0, 64, 48);
        let maps = [
            Homography([f64::NAN; 9]),
            Homography([f64::INFINITY, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]),
            Homography([0.0; 9]),
            Homography([1e300, 0.0, 0.0, 0.0, 1e300, 0.0, 1e-3, 0.0, 1.0]),
            Homography([1e-300, 0.0, 0.0, 0.0, 1e-300, 0.0, 1e-3, 0.0, 1.0]),
            Homography([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0]),
            map(wedge, 64, 48),
        ];
        for h in maps {
            let inv = h.inverse().unwrap_or(Homography([f64::NAN; 9]));
            let cells = std::array::from_fn(|a| std::array::from_fn(|b| scaled_rect(r, octave_scale(a), octave_scale(b))));
            let g = Geometry { h, inv, src_rect: r, side: None, w_lo: 0.0, front: Some(1.0), cells, source_only: false };
            for (x, y) in [(0, 0), (10, 10), (32, 24), (-5, 3), (1000, 1000)] {
                if let Some(l) = pixel_lod(&g, x, y) {
                    assert!(l.u.is_finite() && l.v.is_finite() && (0.0..=1.0).contains(&l.cu) && (0.0..=1.0).contains(&l.cv), "{h:?} ({x}, {y}): {l:?}");
                    for a in [l.ax, l.ay] {
                        assert!(a.n >= 1 && a.lv.iter().all(|&v| v <= MAX_LEVEL) && a.r.iter().all(|r| (1.0..2.0).contains(r)), "{h:?} ({x}, {y}): {a:?}");
                    }
                }
            }
        }
    }

    /// #4864: a cell built from a crop of the source is, inside the part it was built for, bit for
    /// bit what reducing the whole source gives (its edges repeated outwards), at every depth.
    #[test]
    fn cell_build_from_crop_is_exact() {
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let pixel = |x: i32, y: i32| {
                let (x, y) = (x.clamp(0, 699), y.clamp(0, 599));
                let v = ((x * 31 + y * 17) % 101) as f32 / 100.0;
                [v, 1.0 - v, ((x ^ y) & 15) as f32 / 15.0, if (x / 7 + y / 5) % 9 == 0 { 0.5 } else { 1.0 }]
            };
            let src = paint(700, 600, sample, pixel);
            // The source with its edges repeated 300 px outwards, past any cell filter's reach.
            let mut wide = Surface::new(src.format());
            let px: Vec<f32> = (-300..900).flat_map(|y| (-300..1000).flat_map(move |x| pixel(x, y))).collect();
            wide.write_region(Rect::new(-300, -300, 1000, 900), &px);
            for (a, b) in [(0u8, 1u8), (1, 0), (2, 3), (5, 4)] {
                let full = resize_surface_per_axis(&wide, octave_scale(usize::from(a)), octave_scale(usize::from(b)));
                let cell = scaled_rect(Rect::new(0, 0, 700, 600), octave_scale(usize::from(a)), octave_scale(usize::from(b)));
                // Inside the cell, and along its edges.
                let parts = [
                    Rect::new(cell.x0 + cell.width() as i32 / 3, cell.y0 + 1, cell.x1 - cell.width() as i32 / 4, (cell.y0 + 200).min(cell.y1)),
                    Rect::new(cell.x0, cell.y0, cell.x0 + 5, cell.y1.min(256)),
                    Rect::new(cell.x1 - 3, (cell.y1 - 9).max(cell.y0), cell.x1, cell.y1),
                ];
                for r in parts {
                    let j = r.y0.div_euclid(BAND);
                    let r = Rect::new(r.x0, r.y0, r.x1, r.y1.min((j + 1) * BAND));
                    let mut band = Surface::new(src.format());
                    band.put_tiles(build_cell_band(&src, Rect::new(0, 0, 700, 600), a, b, j, r).unwrap());
                    assert_eq!(band.read_region(r), full.read_region(r), "{sample:?} cell ({a}, {b}) {r:?}");
                }
            }
        }
    }

    /// #4864: splitting tiles whose windows pass the cap changes no pixel.
    #[test]
    fn window_cap_split_is_identical() {
        let r = Rect::new(0, 0, 400, 300);
        for sample in [SampleType::U8, SampleType::F32] {
            let src = paint(400, 300, sample, |x, y| {
                let v = ((x * 7 + y * 3) % 23) as f32 / 22.0;
                [v, ((x / 3) & 1) as f32, ((y / 2) & 1) as f32, 1.0]
            });
            for q in [wedge as fn(f64, f64) -> [[f64; 2]; 4], q6] {
                let h = map(q, 400, 300);
                let whole = warp_with(&src, r, &h, Interp::Bicubic, None, &Interrupt::NONE, Tuning::DEFAULT).unwrap();
                let split = warp_with(&src, r, &h, Interp::Bicubic, None, &Interrupt::NONE, Tuning { window_cap: 1024, ..Tuning::DEFAULT }).unwrap();
                assert!(whole == split, "{sample:?}: a split tile changed pixels");
            }
        }
    }

    /// #4864: a Distort that leaves every part of the source at 0.6 or more along each axis (here
    /// magnifying ones, one mirrored) skips planning each pixel, which changes no pixel: every
    /// pixel samples the source alone, and the warp is bit-identical with its tiles planned pixel
    /// by pixel. Maps that reduce more anywhere plan.
    #[test]
    fn source_only_maps_skip_planning_exactly() {
        let (w, hh) = (400, 300);
        let r = Rect::new(0, 0, w, hh);
        let (fw, fh) = (f64::from(w), f64::from(hh));
        let quads = [
            [[0.0, 0.0], [1.3 * fw, 0.1 * fh], [1.4 * fw, 1.1 * fh], [0.0, fh]],
            [[0.0, 0.0], [2.2 * fw, 0.1 * fh], [2.0 * fw, 2.1 * fh], [0.1 * fw, 2.0 * fh]],
            [[1.4 * fw, 0.0], [0.1 * fw, 0.1 * fh], [0.0, 1.1 * fh], [1.4 * fw, fh]],
        ];
        for q in quads {
            let h = Homography::rect_to_quad([0.0, 0.0, fw, fh], q).unwrap();
            assert!(samples_source_only(&h, r), "{q:?}");
            let inv = h.inverse().unwrap();
            let cells = std::array::from_fn(|a| std::array::from_fn(|b| scaled_rect(r, octave_scale(a), octave_scale(b))));
            let g = Geometry { h, inv, src_rect: r, side: Some(1.0), w_lo: 0.0, front: None, cells, source_only: true };
            let b = warp_output_bounds(r, &h).unwrap().inflate(2);
            let one = |a: &Axis| a.n == 1 && a.lv[0] == 0 && a.r[0] < 1.0 / 0.6;
            for y in b.y0..b.y1 {
                for x in b.x0..b.x1 {
                    if let Some(l) = pixel_lod(&g, x, y) {
                        assert!(one(&l.ax) && one(&l.ay), "{q:?} ({x}, {y}): {l:?}");
                    }
                }
            }
            for sample in [SampleType::U8, SampleType::F32] {
                let src = paint(w, hh, sample, |x, y| {
                    let v = ((x * 7 + y * 3) % 23) as f32 / 22.0;
                    [v, ((x / 3) & 1) as f32, ((y / 2) & 1) as f32, 1.0]
                });
                let fast = warp_with(&src, r, &h, Interp::Bicubic, None, &Interrupt::NONE, Tuning::DEFAULT).unwrap();
                let planned = warp_with(&src, r, &h, Interp::Bicubic, None, &Interrupt::NONE, Tuning { plan_pixels: true, ..Tuning::DEFAULT }).unwrap();
                assert!(fast == planned, "{sample:?} {q:?}: skipping the plan changed pixels");
            }
        }
        for q in [wedge as fn(f64, f64) -> [[f64; 2]; 4], q6, trapezoid] {
            assert!(!samples_source_only(&map(q, w, hh), r));
        }
    }

    /// #4864 with #2271 and #4863: a steep Distort keeps every pixel inside its quad, and one dragged
    /// past the horizon draws only its front side, with Bicubic's per-pixel filtering too.
    #[test]
    fn perspective_horizon_bicubic() {
        let mut s = rgba();
        s.fill_rect(Rect::new(0, 0, 100, 100), &[1.0, 0.0, 0.0, 1.0]);
        let r = s.content_bounds();
        let h = Homography::rect_to_quad([0.0, 0.0, 100.0, 100.0], [[0.0, 0.0], [300.0, 0.0], [155.0, 155.0], [0.0, 300.0]]).unwrap();
        assert_eq!(prefilter_scales(&h, r, None), Prefilter::PerPixel);
        let w = warp_surface(&s, r, &h, Interp::Bicubic).unwrap();
        for (x, y) in [(20, 20), (252, 14), (252, 42), (14, 252), (42, 252)] {
            let a = w.read_region(Rect::new(x, y, x + 1, y + 1))[3];
            assert!(a > 0.99, "({x},{y}) inside the quad is transparent: {a}");
        }
        for (x, y) in [(200, 200), (280, 100), (100, 280)] {
            assert_eq!(w.read_region(Rect::new(x, y, x + 1, y + 1))[3], 0.0, "({x},{y}) outside the quad is painted");
        }
        let mut s = rgba();
        s.fill_rect(Rect::new(0, 0, 100, 100), &[0.0, 0.5, 1.0, 1.0]);
        let h = Homography::rect_to_quad([0.0, 0.0, 100.0, 100.0], [[0.0, 0.0], [300.0, 0.0], [60.0, 60.0], [0.0, 300.0]]).unwrap();
        let front = h.front_side([0.0, 0.0, 100.0, 100.0]);
        let inv = h.inverse().unwrap();
        let clip = Rect::new(-400, -400, 700, 700);
        let w = warp_surface_clipped(&s, r, &h, Interp::Bicubic, clip).unwrap();
        let px = w.read_region(clip);
        let (mut drawn, mut behind) = (0, 0);
        for y in clip.y0..clip.y1 {
            for x in clip.x0..clip.x1 {
                let i = ((y - clip.y0) * clip.width() as i32 + (x - clip.x0)) as usize * 4;
                if px[i + 3] > 0.0 {
                    drawn += 1;
                    if front * inv.w(x as f64 + 0.5, y as f64 + 0.5) <= 0.0 {
                        behind += 1;
                    }
                }
            }
        }
        assert!(drawn > 1000 && behind == 0, "{drawn} px drawn, {behind} from behind the horizon");
        assert!(px.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
    }

    /// Every drawn pixel of `o` is finite, its alpha within 0..=1 and its colour within 2.5/255 of
    /// `col` (alpha last).
    fn assert_flat(o: &Surface, col: &[f32], what: &str) {
        let b = o.content_bounds();
        let m = o.format().channels();
        let mut worst = 0.0f32;
        for p in o.read_region(b).chunks_exact(m) {
            assert!(p.iter().all(|v| v.is_finite()) && (0.0..=1.0).contains(&p[m - 1]), "{what}: {p:?}");
            if p[m - 1] > 0.0 {
                worst = p.iter().zip(col).take(m - 1).fold(worst, |w, (v, c)| w.max((v - c).abs()));
            }
        }
        assert!(worst < 2.5 / 255.0, "{what}: colour drifts by {worst}");
    }

    /// #4864: a Distort of a layer smaller than a tile, down to 1 px, anywhere in the document (far
    /// and negative offsets, ±1.5·10⁹), shrunk, magnified and mirrored, keeps flat colour flat in
    /// every mode and depth. Today's path drifted at the outline, where its reduced source faded.
    #[test]
    fn perspective_keeps_flat_colour_on_tiny_far_and_mirrored_layers() {
        let small = [(1, 1), (1, 2), (3, 1), (2, 2), (5, 7)];
        let large = [(255, 3), (3, 257), (300, 40)];
        let offsets = [(0, 0), (-37, -101), (1_000_003, 7), (1_500_000_000, 1_500_000_000), (-1_500_000_000, -64)];
        for fmt in golden_formats() {
            let n = fmt.channels();
            let col: Vec<f32> = (0..n).map(|c| if fmt.alpha && c == n - 1 { 1.0 } else { 0.2 + 0.15 * c as f32 }).collect();
            let opaque: Vec<f32> = col.iter().copied().take(fmt.mode.color_channels()).chain([1.0]).collect();
            // Every size, offset and scale in 8-bit RGB; the small sizes at two offsets elsewhere.
            let rgb8 = fmt.sample == SampleType::U8 && fmt.mode == ColorMode::Rgb;
            for &(w, h) in small.iter().chain(if rgb8 { &large[..] } else { &[] }) {
                for &(ox, oy) in if rgb8 { &offsets[..] } else { &offsets[2..4] } {
                    let r = Rect::new(ox, oy, ox + w, oy + h);
                    let mut s = Surface::new(fmt);
                    s.fill_rect(r, &col);
                    let (fw, fh) = (f64::from(w), f64::from(h));
                    let rect = [f64::from(ox), f64::from(oy), f64::from(ox) + fw, f64::from(oy) + fh];
                    for scale in [0.3, 4.0, if rgb8 { 200.0 / fw.max(fh) } else { 40.0 }] {
                        for mirror in [false, true] {
                            let q = wedge(fw * scale, fh * scale).map(|[x, y]| if mirror { [50.0 - x, y + 5.0] } else { [x + 5.0, y + 5.0] });
                            let hm = Homography::rect_to_quad(rect, q).unwrap();
                            let o = warp_surface(&s, r, &hm, Interp::Bicubic).unwrap();
                            assert_flat(&o, &opaque, &format!("{fmt:?} {w}x{h} at ({ox}, {oy}), ×{scale}, mirrored {mirror}"));
                        }
                    }
                }
            }
        }
    }

    /// #4864: 32-bit colours past 0..1 and a translucent layer stay flat; today's path clamped the
    /// reduced source's ringing alpha but not its colour, so colours drifted at the outline.
    #[test]
    fn perspective_keeps_hdr_and_translucent_colour_flat() {
        let r = Rect::new(0, 0, 400, 300);
        for col in [[3.5f32, -0.25, 12.0, 1.0], [0.5, 0.5, 0.5, 0.4]] {
            let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
            s.fill_rect(r, &col);
            for q in [wedge as fn(f64, f64) -> [[f64; 2]; 4], q6, trapezoid] {
                let o = warp_surface(&s, r, &map(q, 400, 300), Interp::Bicubic).unwrap();
                let mut worst = 0.0f32;
                for p in o.read_region(o.content_bounds()).as_chunks::<4>().0 {
                    assert!(p[3] <= col[3] + 1e-5, "{col:?}: alpha {} past the layer's", p[3]);
                    if p[3] > 0.0 {
                        worst = p.iter().zip(&col).take(3).fold(worst, |w, (v, c)| w.max((v - c).abs()));
                    }
                }
                assert!(worst < 1e-4, "{col:?}: colour drifts by {worst}");
            }
        }
    }

    /// #4864: each pixel's filter is worked out in source space, so mirroring or transposing the
    /// destination mirrors or transposes the result (up to rounding), wherever the tiles fall.
    #[test]
    fn perspective_mirrored_and_transposed_quads_match() {
        let src = paint(400, 300, SampleType::F32, |x, y| {
            let v = ((x * 7 + y * 3) % 23) as f32 / 22.0;
            [v, ((x / 3) & 1) as f32, ((y / 2) & 1) as f32, 1.0]
        });
        let r = Rect::new(0, 0, 400, 300);
        for q in [wedge as fn(f64, f64) -> [[f64; 2]; 4], q6, trapezoid] {
            let quad = q(400.0, 300.0);
            let o = warp_surface(&src, r, &Homography::rect_to_quad([0.0, 0.0, 400.0, 300.0], quad).unwrap(), Interp::Bicubic).unwrap();
            type Back = fn(i32, i32) -> (i32, i32);
            let variants: [(&str, [[f64; 2]; 4], Back); 3] = [
                ("mirror x", quad.map(|[x, y]| [1000.0 - x, y]), |x, y| (999 - x, y)),
                ("mirror y", quad.map(|[x, y]| [x, 1000.0 - y]), |x, y| (x, 999 - y)),
                ("transpose", quad.map(|[x, y]| [y, x]), |x, y| (y, x)),
            ];
            for (name, mq, back) in variants {
                let m = warp_surface(&src, r, &Homography::rect_to_quad([0.0, 0.0, 400.0, 300.0], mq).unwrap(), Interp::Bicubic).unwrap();
                let b = m.content_bounds();
                let px = m.read_region(b);
                let premul = |p: &[f32], k: usize| if k < 3 { p[k] * p[3] } else { p[3] };
                let mut worst = 0.0f32;
                for (i, p) in px.as_chunks::<4>().0.iter().enumerate() {
                    let (x, y) = (b.x0 + (i % b.width() as usize) as i32, b.y0 + (i / b.width() as usize) as i32);
                    let (sx, sy) = back(x, y);
                    let t = o.pixel(sx, sy);
                    worst = (0..4).fold(worst, |w, k| w.max((premul(p, k) - premul(&t, k)).abs()));
                }
                assert!(worst < 1e-6, "{name}: differs by {worst}");
            }
        }
    }

    /// #4864: a tile is skipped only when no source tile lies within a filter's reach, so a source
    /// whose transparent tiles are missing draws exactly what it draws with them written.
    #[test]
    fn perspective_sparse_source_matches_dense() {
        let r = Rect::new(0, 0, 800, 600);
        let blobs = |s: &mut Surface| {
            s.fill_rect(Rect::new(300, 220, 500, 370), &[0.9, 0.3, 0.1, 1.0]);
            s.fill_rect(Rect::new(690, 200, 750, 400), &[0.1, 0.8, 0.4, 1.0]);
            s.fill_rect(Rect::new(780, 590, 800, 600), &[0.2, 0.2, 0.9, 1.0]);
        };
        let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
        let mut sparse = Surface::new(fmt);
        blobs(&mut sparse);
        let mut dense = Surface::new(fmt);
        dense.write_region(r, &vec![0.0f32; 800 * 600 * 4]);
        blobs(&mut dense);
        assert!(dense.tiles().count() > sparse.tiles().count());
        for q in [wedge as fn(f64, f64) -> [[f64; 2]; 4], q6, trapezoid] {
            let h = map(q, 800, 600);
            let (a, b) = (warp_surface(&sparse, r, &h, Interp::Bicubic).unwrap(), warp_surface(&dense, r, &h, Interp::Bicubic).unwrap());
            let u = a.content_bounds().union(&b.content_bounds());
            assert!(a.read_region(u) == b.read_region(u), "sparse and dense sources draw differently");
        }
    }

    /// #4864 with #2271 and #4863: Distorts at the edge of what a map can do (a corner pinched to
    /// 0.02 px, one thrown a million pixels away, a bow tie, quads past the horizon, a 2000 × 3 px
    /// sliver) stay finite and flat at every depth, and any clip keeps the pixels of a larger one.
    #[test]
    fn perspective_extreme_maps_stay_flat_and_clip_consistently() {
        let r = Rect::new(0, 0, 200, 150);
        let col = [0.3f32, 0.6, 0.9, 1.0];
        let quads: [[[f64; 2]; 4]; 6] = [
            [[0.0, 0.0], [200.0, 0.0], [100.01, 150.0], [99.99, 150.0]],
            [[0.0, 0.0], [200.0, 0.0], [1e6, 1e6], [0.0, 150.0]],
            [[0.0, 0.0], [200.0, 0.0], [0.0, 150.0], [200.0, 150.0]],
            [[0.0, 0.0], [600.0, 0.0], [120.0, 120.0], [0.0, 600.0]],
            [[0.0, 0.0], [600.0, 0.0], [310.0, 310.0], [0.0, 600.0]],
            [[0.0, 0.0], [2000.0, 0.0], [2000.0, 3.0], [0.0, 1.0]],
        ];
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, sample, true));
            s.fill_rect(r, &col);
            for q in quads {
                let h = Homography::rect_to_quad([0.0, 0.0, 200.0, 150.0], q).unwrap();
                let big = Rect::new(-700, -700, 2100, 1300);
                let whole = warp_surface_clipped(&s, r, &h, Interp::Bicubic, big).unwrap();
                assert!(!whole.content_bounds().is_empty(), "{sample:?} {q:?}: nothing drawn");
                assert_flat(&whole, &col, &format!("{sample:?} {q:?}"));
                for clip in [Rect::new(-50, -40, 230, 260), Rect::new(90, 60, 130, 160), Rect::new(1500, -3, 2050, 9)] {
                    let part = warp_surface_clipped(&s, r, &h, Interp::Bicubic, clip).unwrap();
                    assert!(whole.read_region(clip) == part.read_region(clip), "{sample:?} {q:?} {clip:?}");
                }
            }
        }
    }
}
