//! Mesh warps (Edit › Transform › Warp, smart-object warps): resampling a surface through any
//! forward map, such as a [`photocraft_geom::warp::Warp`] (bicubic Bezier patches or a preset).
//!
//! The source rectangle is subdivided into a fine grid whose vertices are mapped forward; each
//! cell's two triangles are rasterized in destination space with the source position
//! interpolated barycentrically, then the source is sampled there (premultiplied, nearest /
//! bilinear / Photoshop's Keys bicubic, like Free Transform). Cells are a few pixels across in
//! both the source and the destination (a thin layer stretched far gets as many cells as its
//! warped size needs), so the piecewise-affine inverse is within a small fraction of a pixel of
//! the true one; affine
//! maps (the identity included) are reproduced exactly. Where the warp shrinks the source, each
//! pixel instead averages a grid of bilinear subsamples over its source footprint (about one per
//! source pixel), so thin lines don't alias. Where the warp folds over itself, [`Fold`] decides
//! what shows: for Edit › Transform › Warp and smart objects the part dragged furthest is on top and
//! the layers composite; for other mesh maps (lens corrections, distortions) the later
//! (lower-right) cell wins. Work is split per destination tile (parallel on native).
//!
//! A smart object's warp ([`place_source`]) is cut as the reference application cuts it instead
//! ([`warp_patches_surface`]): cells that halve until they are within half a pixel of the surface,
//! small where the warp bends and large where it is flat.

use photocraft_color::PixelFormat;
use photocraft_geom::{Rect, TileCoord};
use photocraft_raster::{Interrupt, Surface, Tile};
use std::sync::Arc;

use crate::transform::{Interp, bicubic};

/// Grid cell size (source px) used to approximate the map.
const CELL: f64 = 4.0;
/// Most cells along one axis (bounds memory and time on huge layers).
const MAX_CELLS: usize = 512;
/// How a mesh warp stacks the picture where it folds over itself.
#[derive(Clone, Copy)]
pub enum Fold<'a> {
    /// Cells in row order, the last one drawn wins (lens corrections, distortions).
    Rows,
    /// A warp of the picture in place (Edit › Transform › Warp, a smart object's warp): cells in
    /// [`cell_order`] (a part the fold turns over on top, else the part dragged furthest), and a fold's layers composite, so transparent
    /// parts show what is under them, as Warp mode's preview draws them. The map is where a
    /// source point lands without the warp: the identity for a layer, a smart object's placement.
    Stack(&'a (dyn Fn(f64, f64) -> (f64, f64) + Sync)),
}

impl Fold<'_> {
    /// [`Fold::Stack`] about the identity: a warp of a layer where it is.
    pub const IN_PLACE: Fold<'static> = Fold::Stack(&|x: f64, y: f64| (x, y));
}

/// Most subsamples per axis when a warp shrinks the source; also the longest source step per
/// destination pixel the filter follows (bounds the footprint each tile reads).
const MAX_SS: f64 = 8.0;

/// Places a smart object's source image (`src`, its pixels in `src_rect`) in the document:
/// through `warp` (source space) and then the affine `t` (source → document) in one resampling
/// pass, an exact shift for whole-pixel translations (so conversions and re-renders are
/// lossless), a resize into a box snapped to whole pixels when it is only scaled and moved
/// ([`place_scaled`]), bicubic otherwise. A warp samples bilinearly, as the reference application draws a
/// warped smart object: with bicubic, the sharp detail of a warp (cell borders, a fold's edges)
/// came out crisper than its render. Shared by the engine's re-render and PSD export's filter
/// cache.
pub fn place_source(
    src: &Surface,
    src_rect: Rect,
    t: &photocraft_geom::Affine,
    warp: Option<&photocraft_geom::warp::Warp>,
) -> Result<Surface, photocraft_raster::AllocationError> {
    let [a, b, c, d, e, f] = t.m;
    if let Some(w) = warp.filter(|w| !w.is_identity()) {
        let map = |x: f64, y: f64| {
            let (u, v) = w.map(x, y);
            (a * u + c * v + e, b * u + d * v + f)
        };
        // A fold's layers are ordered by how far the warp moved them, not the placement.
        let rest = |x: f64, y: f64| (a * x + c * y + e, b * x + d * y + f);
        let (cu, cv) = patch_cuts(w, src_rect);
        let sampling = Sampling::for_scale((a * d - b * c).abs().sqrt());
        return Ok(warp_patches_surface(src, src_rect, &map, [&cu, &cv], Fold::Stack(&rest), sampling, facets_of(w)));
    }
    let near = |x: f64, y: f64| (x - y).abs() < 1e-9;
    if near(a, 1.0) && near(b, 0.0) && near(c, 0.0) && near(d, 1.0) && near(e, e.round()) && near(f, f.round()) {
        return Ok(crate::resample::translate_surface(src, e.round() as i32, f.round() as i32));
    }
    if near(b, 0.0)
        && near(c, 0.0)
        && let Some(out) = place_scaled(src, src_rect, [a, d, e, f])
    {
        return Ok(out);
    }
    // Rotated, skewed or both (not mirrored): as the reference application draws it.
    let (bx, by) = (f64::from(src_rect.x0), f64::from(src_rect.y0));
    if let Some(out) = crate::smart_affine::render(src, src_rect, [a, b, c, d, e + a * bx + c * by, f + b * bx + d * by]) {
        return Ok(out);
    }
    crate::transform::warp_surface(src, src_rect, &crate::transform::Homography([a, c, e, b, d, f, 0.0, 0.0, 1.0]), Interp::Bicubic)
}

/// Placed sizes past this (px a side) take the general path, which bounds its own work.
const MAX_SCALED_SIDE: f64 = (1u32 << 20) as f64;

/// A smart object scaled and moved (no rotation, skew or flip: `[sx, sy, dx, dy]`, source →
/// document), as the reference application draws one ([`scale_snapped`]), its contents being the
/// pixels of `src` inside `src_rect`. `None` for a flip or a size this path doesn't take.
fn place_scaled(src: &Surface, src_rect: Rect, t: [f64; 4]) -> Option<Surface> {
    snapped_scale(src_rect, t)?;
    scale_snapped(&crate::resample::crop_surface(src, src_rect), src_rect, t)
}

/// Where the reference application puts a smart object whose box (its contents' canvas) is
/// `bounds`, scaled and moved by `[sx, sy, dx, dy]` (source → document): the box's left and top
/// edges and its width and height are each rounded to whole pixels, halves up (a move to 106.5
/// lands on 107). Returns the map that fills that snapped box, as `[sx, sy, dx, dy]`, or `None`
/// for a flip, an empty box, or a size past [`MAX_SCALED_SIDE`].
pub fn snapped_scale(bounds: Rect, [sx, sy, dx, dy]: [f64; 4]) -> Option<[f64; 4]> {
    if bounds.is_empty() || !(sx > 0.0 && sy > 0.0) {
        return None;
    }
    let (w, h) = (f64::from(bounds.width()), f64::from(bounds.height()));
    let snap = |v: f64| (v + 0.5).floor();
    let (x0, y0) = (snap(dx + sx * f64::from(bounds.x0)), snap(dy + sy * f64::from(bounds.y0)));
    let (pw, ph) = (snap(sx * w).max(1.0), snap(sy * h).max(1.0));
    let fits = |v: f64| v.is_finite() && v.abs() < MAX_SCALED_SIDE;
    if !(fits(x0) && fits(y0) && fits(pw) && fits(ph)) {
        return None;
    }
    let (kx, ky) = (pw / w, ph / h);
    Some([kx, ky, x0 - kx * f64::from(bounds.x0), y0 - ky * f64::from(bounds.y0)])
}

/// `src` scaled and moved by `t` the way the reference application draws a smart object with
/// the box `bounds` ([`snapped_scale`], [`crate::smart_scale`]): resized to fill the snapped box.
/// Shared with Convert to Layers, so the layers it makes look as the smart object did. `None`
/// where [`snapped_scale`] or [`crate::smart_scale::scale`] take no part.
pub fn scale_snapped(src: &Surface, bounds: Rect, t: [f64; 4]) -> Option<Surface> {
    let [kx, ky, dx, dy] = snapped_scale(bounds, t)?;
    let fmt = src.format();
    let with_alpha;
    let src = if fmt.alpha {
        src
    } else {
        with_alpha = src.convert(PixelFormat::new(fmt.mode, fmt.sample, true));
        &with_alpha
    };
    // The snapped box: whole pixels by construction.
    let (bx, by) = (f64::from(bounds.x0), f64::from(bounds.y0));
    let (x0, y0) = ((dx + kx * bx).round() as i32, (dy + ky * by).round() as i32);
    let (w, h) = ((kx * f64::from(bounds.width())).round() as u32, (ky * f64::from(bounds.height())).round() as u32);
    if kx == 1.0 && ky == 1.0 {
        return Some(crate::resample::translate_surface(&crate::resample::crop_surface(src, bounds), x0 - bounds.x0, y0 - bounds.y0));
    }
    crate::smart_scale::scale(src, bounds, w, h, (x0, y0))
}

/// [`place_source`] through a projective map `h` (source → document; Distort, Perspective): the
/// warp first (in source space), then `h`; bilinear through a warp, as there.
pub fn place_source_projective(
    src: &Surface,
    src_rect: Rect,
    h: &crate::transform::Homography,
    warp: Option<&photocraft_geom::warp::Warp>,
) -> Result<Surface, photocraft_raster::AllocationError> {
    if let Some(w) = warp.filter(|w| !w.is_identity()) {
        let map = |x: f64, y: f64| {
            let (u, v) = w.map(x, y);
            h.apply(u, v)
        };
        let (cu, cv) = patch_cuts(w, src_rect);
        // The placement's scale where the source's middle lands.
        let (cx, cy) = (f64::from(src_rect.x0 + src_rect.x1) / 2.0, f64::from(src_rect.y0 + src_rect.y1) / 2.0);
        let ((x0, y0), (x1, y1), (x2, y2)) = (h.apply(cx, cy), h.apply(cx + 1.0, cy), h.apply(cx, cy + 1.0));
        let scale = ((x1 - x0) * (y2 - y0) - (x2 - x0) * (y1 - y0)).abs().sqrt();
        let rest = |x: f64, y: f64| h.apply(x, y);
        return Ok(warp_patches_surface(src, src_rect, &map, [&cu, &cv], Fold::Stack(&rest), Sampling::for_scale(scale), facets_of(w)));
    }
    // A one-way perspective: as the reference application draws it.
    let corner = crate::smart_perspective::from_corner(h.0, f64::from(src_rect.x0), f64::from(src_rect.y0));
    if let Some(out) = crate::smart_perspective::render(src, src_rect, corner) {
        return Ok(out);
    }
    crate::transform::warp_surface(src, src_rect, h, Interp::Bicubic)
}

/// Warps the content of `src` inside `src_rect` through the forward map `f` (source document
/// coordinates → destination document coordinates). Output has `src`'s format with alpha
/// added; pixels outside the warped area are transparent.
pub fn warp_mesh_surface(src: &Surface, src_rect: Rect, f: &(dyn Fn(f64, f64) -> (f64, f64) + Sync), interp: Interp) -> Surface {
    mesh(src, src_rect, f, Fold::Rows, interp, &Interrupt::NONE)
}

/// [`warp_mesh_surface`] with folds stacked as `fold` says, reporting progress per destination
/// tile; `None` once `ctl` is cancelled.
pub fn warp_mesh_surface_with(
    src: &Surface,
    src_rect: Rect,
    f: &(dyn Fn(f64, f64) -> (f64, f64) + Sync),
    fold: Fold,
    interp: Interp,
    ctl: &Interrupt,
) -> Option<Surface> {
    let out = mesh(src, src_rect, f, fold, interp, ctl);
    (!ctl.cancelled()).then_some(out)
}

/// The order Warp draws the cells of a vertex grid in (`row` vertices a row; `pos` where the warp
/// put each vertex, `rest` where it would be without it). Where a warp folds the picture over
/// itself, the part turned over lies on top, like a folded-back flap: a corner or edge dragged
/// across the picture, and also the edge row that folds back when the middle is pulled out past
/// it, which leaves the pulled part under that row. Within each, the cells moved least come first,
/// so the part dragged furthest is on top. Each cell is named by its top-left vertex. Shared by
/// the render and Warp mode's preview, so they stack folds alike.
pub fn cell_order(pos: &[[f64; 2]], rest: &[[f64; 2]], row: usize) -> Vec<usize> {
    if row < 2 || pos.len() != rest.len() {
        return Vec::new();
    }
    let rows = pos.len() / row;
    // A vertex with no finite movement (sent to infinity, as near a Distort's horizon) goes under,
    // not on top.
    let moved = |k: usize| match (pos.get(k), rest.get(k)) {
        (Some(p), Some(r)) => Some((p[0] - r[0]).hypot(p[1] - r[1])).filter(|m| m.is_finite()).unwrap_or(f64::NEG_INFINITY),
        _ => f64::NEG_INFINITY,
    };
    // Twice the signed area of a cell's quad: its sign is the cell's orientation (NaN when a corner
    // isn't finite, which counts as not turned over).
    let area = |pts: &[[f64; 2]], a: usize| {
        let q = [a, a + 1, a + row + 1, a + row].map(|k| pts.get(k).copied().unwrap_or([f64::NAN; 2]));
        (0..4).map(|i| q[i][0] * q[(i + 1) % 4][1] - q[(i + 1) % 4][0] * q[i][1]).sum::<f64>()
    };
    let mut cells: Vec<(bool, f64, usize)> = Vec::with_capacity(rows.saturating_sub(1) * (row - 1));
    for j in 0..rows.saturating_sub(1) {
        for i in 0..row - 1 {
            let a = j * row + i;
            let turned = area(pos, a) * area(rest, a) < 0.0;
            cells.push((turned, moved(a) + moved(a + 1) + moved(a + row) + moved(a + row + 1), a));
        }
    }
    let by = |x: &(bool, f64, usize), y: &(bool, f64, usize)| x.0.cmp(&y.0).then(x.1.total_cmp(&y.1));
    // A stable sort either way, so equal cells keep row order.
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        cells.par_sort_by(by);
    }
    #[cfg(target_arch = "wasm32")]
    cells.sort_by(by);
    cells.into_iter().map(|(_, _, a)| a).collect()
}

/// Folds stack as `fold` says.
fn mesh(src: &Surface, src_rect: Rect, f: &(dyn Fn(f64, f64) -> (f64, f64) + Sync), fold: Fold, interp: Interp, ctl: &Interrupt) -> Surface {
    if src_rect.is_empty() {
        let fmt = src.format();
        return Surface::new(PixelFormat::new(fmt.mode, fmt.sample, true));
    }
    let (sw, sh) = (f64::from(src_rect.width()), f64::from(src_rect.height()));
    // Cells are a few pixels across on both sides of the map: a thin source stretched far (a
    // 3 px column scaled into a wide band, then bent) needs many cells, or it comes out straight.
    let (lu, lv) = mapped_lengths(src_rect, f);
    let cells = |s: f64, d: f64| ((s.max(d) / CELL).ceil() as usize).clamp(1, MAX_CELLS);
    let (nx, ny) = (cells(sw, lu), cells(sh, lv));
    // Forward-mapped vertices (and their source positions).
    let mut verts = Vec::with_capacity((nx + 1) * (ny + 1));
    for j in 0..=ny {
        for i in 0..=nx {
            let sx = f64::from(src_rect.x0) + sw * i as f64 / nx as f64;
            let sy = f64::from(src_rect.y0) + sh * j as f64 / ny as f64;
            let (dx, dy) = f(sx, sy);
            verts.push(([dx, dy], [sx, sy]));
        }
    }
    let w1 = nx + 1;
    // Drawn in row order; for a fold, a tile where two parts of the picture meet is drawn again in
    // [`cell_order`], each cell's two triangles ranked together.
    let mut tris = Vec::with_capacity(nx * ny * 2);
    for j in 0..ny {
        for i in 0..nx {
            let a = j * w1 + i;
            tris.push([a, a + 1, a + w1 + 1]);
            tris.push([a, a + w1 + 1, a + w1]);
        }
    }
    let rank = || -> Vec<u32> {
        let Fold::Stack(rest) = fold else { return Vec::new() };
        let pos: Vec<[f64; 2]> = verts.iter().map(|(d, _)| *d).collect();
        let at_rest: Vec<[f64; 2]> = verts
            .iter()
            .map(|(_, s)| {
                let r = rest(s[0], s[1]);
                [r.0, r.1]
            })
            .collect();
        let mut rank = vec![0u32; tris.len()];
        for (r, a) in cell_order(&pos, &at_rest, w1).into_iter().enumerate() {
            let c = (a / w1) * nx + a % w1;
            for (t, k) in [(2 * c, 2 * r), (2 * c + 1, 2 * r + 1)] {
                if let Some(slot) = rank.get_mut(t) {
                    *slot = u32::try_from(k).unwrap_or(u32::MAX);
                }
            }
        }
        rank
    };
    let stack: Option<&(dyn Fn() -> Vec<u32> + Sync)> = matches!(fold, Fold::Stack(_)).then_some(&rank);
    triangles(src, src_rect, &verts, &tris, interp, ctl, Draw { stack, ..Draw::default() })
}

/// How far (destination px) a cell's straight edges and middle may stray from the warped surface
/// before the cell is split, in [`warp_patches_surface`], for an envelope ([`Facets::Envelope`]).
const FLAT_TOL: f64 = 0.5;
/// The same for a full mesh ([`Facets::Mesh`]), whose edges alone decide (measured between 0.381
/// and 0.396 on the reference's coordinate probes).
const MESH_TOL: f64 = 0.3873;

/// How the reference application tessellates a warp, by the kind of mesh it was given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Facets {
    /// A mesh with fewer than four control points along a side (the presets' envelopes: Arc,
    /// Arch, Bulge, Flag, Wave, Fish, Rise, Inflate, Squeeze and their kin, stored as 4 × 2, 3 × 3
    /// or 4 × 3 grids): a cell splits along both axes when its middle strays, along one when that
    /// axis's edges do, by [`FLAT_TOL`].
    Envelope,
    /// A full 4 × 4 mesh (custom warps, Fisheye, Twist, Shell): each axis splits only when its own
    /// edges stray, by [`MESH_TOL`]; the middle is not tested.
    Mesh,
}

/// The [`Facets`] rule for `warp`: [`Facets::Envelope`] for a mesh given with fewer than four
/// control points along a side ([`photocraft_geom::warp::Warp::envelope`]), else [`Facets::Mesh`].
pub fn facets_of(warp: &photocraft_geom::warp::Warp) -> Facets {
    if warp.envelope { Facets::Envelope } else { Facets::Mesh }
}

/// How [`warp_patches_surface`] samples the source (bilinearly either way).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sampling {
    /// One bilinear sample per pixel, as the reference application's renders of its warps show;
    /// where the warp squeezes the source 2× or more along a destination row, from a box mip
    /// level (see `mip_level`).
    Single,
    /// The pixel's source footprint averaged, so a strong reduction doesn't alias.
    Footprint,
}

impl Sampling {
    /// [`Sampling::Single`] for a smart object placed at least half its size (linear `scale`),
    /// else [`Sampling::Footprint`] (as the reference prefilters a plain downscale; no render of a
    /// warp placed that small has been measured).
    pub fn for_scale(scale: f64) -> Sampling {
        if scale.is_finite() && scale >= 0.5 { Sampling::Single } else { Sampling::Footprint }
    }
}

/// Most cells one [`warp_patches_surface`] tessellation makes; past it, cells stay as they are.
const MAX_LEAVES: usize = 1 << 18;
/// Cells narrower than this (source px) aren't split further.
const MIN_CELL: f64 = 0.5;
/// Cells each patch starts as, per axis: a curve that crosses its chord at the midpoint (a flag's
/// S-shaped edge, a twist) looks straight to a test of the midpoint alone.
const START_CELLS: usize = 4;

/// A cell of the source rectangle in normalised coordinates: `[u0, u1, v0, v1]`.
type Cell = [f64; 4];
/// Mesh vertices as `(destination, source)`, triangles of vertex indices, and each triangle's rank
/// where a fold stacks them.
type TriMesh = (Vec<([f64; 2], [f64; 2])>, Vec<[usize; 3]>, Vec<u32>);

/// Warps `src` through a smart object's warp, `f`, as the reference application draws one: the
/// surface is cut into cells, each drawn as two flat triangles (split along the diagonal from the
/// top-left corner): each patch starts as [`START_CELLS`] × [`START_CELLS`] cells, and a cell is
/// halved, along an axis or both, until its edges (and, for an envelope, its middle) stray at
/// most the tolerance of `facets` from the surface. So cells are small where the warp bends hard and
/// large where it is flat, and the picture keeps the slight facets of that tessellation, which
/// cells of a few pixels everywhere ([`warp_mesh_surface`]) smoothed away. `cuts_u`/`cuts_v` are
/// the patch borders inside the rectangle (normalised, `0..1`): the cells start from them.
/// Where a cell meets two smaller ones, the point between them is put on the bigger cell's edge,
/// so no crack opens.
pub fn warp_patches_surface(
    src: &Surface,
    src_rect: Rect,
    f: &(dyn Fn(f64, f64) -> (f64, f64) + Sync),
    cuts: [&[f64]; 2],
    fold: Fold,
    sampling: Sampling,
    facets: Facets,
) -> Surface {
    if src_rect.is_empty() {
        let fmt = src.format();
        return Surface::new(PixelFormat::new(fmt.mode, fmt.sample, true));
    }
    let [cuts_u, cuts_v] = cuts;
    let (x0, y0) = (f64::from(src_rect.x0), f64::from(src_rect.y0));
    let (sw, sh) = (f64::from(src_rect.width()), f64::from(src_rect.height()));
    let at = |u: f64, v: f64| {
        let (x, y) = f(x0 + sw * u, y0 + sh * v);
        [x, y]
    };
    // An envelope, or a full mesh of one patch, is cut as the reference application cuts one (see
    // `warp_tess`).
    let one_patch = cuts_u.iter().chain(cuts_v).all(|&c| c <= 1e-9 || c >= 1.0 - 1e-9);
    let envelope = (facets == Facets::Envelope || one_patch).then(|| crate::warp_tess::envelope_mesh(&at, facets != Facets::Envelope)).flatten();
    let (verts, tris, rank) = match envelope {
        Some(m) => {
            let verts: Vec<([f64; 2], [f64; 2])> = m.uv.iter().zip(&m.pos).map(|(p, &d)| (d, [x0 + sw * p[0], y0 + sh * p[1]])).collect();
            let (tris, rank) = ordered_triangles(&verts, &m.pos, &m.quads, &fold);
            (verts, tris, rank)
        }
        None => match quilt_mesh(&at, cuts_u, cuts_v).filter(|_| facets == Facets::Mesh) {
            Some(m) => {
                let verts: Vec<([f64; 2], [f64; 2])> = m.uv.iter().zip(&m.pos).map(|(p, &d)| (d, [x0 + sw * p[0], y0 + sh * p[1]])).collect();
                let (tris, rank) = ordered_triangles(&verts, &m.pos, &m.quads, &fold);
                // Under the cells, the slivers that close a T-junction's crack.
                let fill = t_slivers(&m.uv, &m.quads);
                let rank_of = |q: usize| rank.get(2 * q).copied().unwrap_or(0);
                let (fill_tris, fill_rank): (Vec<[usize; 3]>, Vec<u32>) = fill.iter().map(|&(t, q)| (t, rank_of(q))).unzip();
                (verts, fill_tris.into_iter().chain(tris).collect(), fill_rank.into_iter().chain(rank).collect())
            }
            None => {
                let leaves = tessellate(&at, cuts_u, cuts_v, sw, sh, facets);
                cell_triangles(&at, &leaves, [x0, y0, sw, sh], &fold)
            }
        },
    };
    let rank_of_tris = move || rank.clone();
    // The reference application draws a warp's border half a pixel out (see `Draw::fringe`).
    triangles(
        src,
        src_rect,
        &verts,
        &tris,
        Interp::Bilinear,
        &Interrupt::NONE,
        Draw { single: sampling == Sampling::Single, fringe: true, stack: matches!(fold, Fold::Stack(_)).then_some(&rank_of_tris) },
    )
}

/// A mesh split into several patches (`cuts_u`/`cuts_v` inside `0..1`), cut patch by patch as
/// the reference application cuts a quilt (see [`crate::warp_tess::quilt_patch_mesh`]), in the
/// whole surface's normalised coordinates. `None` when a patch can't be evaluated.
fn quilt_mesh(at: &dyn Fn(f64, f64) -> [f64; 2], cuts_u: &[f64], cuts_v: &[f64]) -> Option<crate::warp_tess::EnvelopeMesh> {
    let (us, vs) = (cut_list(cuts_u), cut_list(cuts_v));
    let mut all = crate::warp_tess::EnvelopeMesh { uv: Vec::new(), pos: Vec::new(), quads: Vec::new() };
    for v in vs.windows(2) {
        for u in us.windows(2) {
            let (&[ua, ub], &[va, vb]) = (u, v) else { continue };
            let sub = |s: f64, t: f64| at(ua + (ub - ua) * s, va + (vb - va) * t);
            let m = crate::warp_tess::quilt_patch_mesh(&sub)?;
            let base = all.uv.len();
            all.uv.extend(m.uv.iter().map(|p| [ua + (ub - ua) * p[0], va + (vb - va) * p[1]]));
            all.pos.extend(m.pos);
            all.quads.extend(m.quads.iter().map(|q| q.map(|i| i + base)));
        }
    }
    Some(stitch_patches(all, &us, &vs))
}

/// Triangles closing the cracks at T-junctions of `quads` (vertex indices into `uv`, normalised
/// source positions): where points of smaller cells lie inside a cell's edge but stay on the
/// surface (as the reference application leaves them), the gap between that straight edge and the
/// points is fanned from the edge's first end. Each comes with the index of its quad.
fn t_slivers(uv: &[[f64; 2]], quads: &[[usize; 4]]) -> Vec<([usize; 3], usize)> {
    use std::collections::HashMap;
    let q = |t: f64| (t * f64::from(1u32 << 30)).round() as i64;
    // Points along each line of constant u (by v) and constant v (by u).
    let mut on_u: HashMap<i64, Vec<(f64, usize)>> = HashMap::new();
    let mut on_v: HashMap<i64, Vec<(f64, usize)>> = HashMap::new();
    for (i, p) in uv.iter().enumerate() {
        on_u.entry(q(p[0])).or_default().push((p[1], i));
        on_v.entry(q(p[1])).or_default().push((p[0], i));
    }
    for l in on_u.values_mut().chain(on_v.values_mut()) {
        l.sort_by(|a, b| a.0.total_cmp(&b.0));
    }
    let mut out = Vec::new();
    for (k, &[tl, tr, bl, br]) in quads.iter().enumerate() {
        // Top and bottom edges run along u at constant v; left and right along v at constant u.
        for (a, b, along_u) in [(tl, tr, true), (bl, br, true), (tl, bl, false), (tr, br, false)] {
            let (Some(pa), Some(pb)) = (uv.get(a), uv.get(b)) else { continue };
            let (line, from, to, lines) = if along_u { (q(pa[1]), pa[0], pb[0], &on_v) } else { (q(pa[0]), pa[1], pb[1], &on_u) };
            let (lo, hi) = (from.min(to), from.max(to));
            let Some(points) = lines.get(&line) else { continue };
            let mut inside: Vec<usize> = points.iter().filter(|(t, _)| *t > lo + 1e-12 && *t < hi - 1e-12).map(|&(_, i)| i).collect();
            if inside.is_empty() {
                continue;
            }
            if from > to {
                inside.reverse();
            }
            inside.push(b);
            let mut prev = inside[0];
            for &next in inside.iter().skip(1) {
                out.push(([a, prev, next], k));
                prev = next;
            }
        }
    }
    out
}

/// Joins the patches of a quilt: a point the patches share becomes one vertex, and a point on a
/// patch border inside a neighbouring patch's longer edge is moved onto that straight edge, so no
/// crack opens where the two sides were cut differently.
fn stitch_patches(m: crate::warp_tess::EnvelopeMesh, us: &[f64], vs: &[f64]) -> crate::warp_tess::EnvelopeMesh {
    use std::collections::HashMap;
    let q = |t: f64| (t * f64::from(1u32 << 30)).round() as i64;
    let mut index: HashMap<(i64, i64), usize> = HashMap::new();
    let (mut uv, mut pos) = (Vec::new(), Vec::new());
    let mut remap = Vec::with_capacity(m.uv.len());
    for (p, &d) in m.uv.iter().zip(&m.pos) {
        let id = *index.entry((q(p[0]), q(p[1]))).or_insert_with(|| {
            uv.push(*p);
            pos.push(d);
            uv.len() - 1
        });
        remap.push(id);
    }
    let quads: Vec<[usize; 4]> = m.quads.iter().map(|c| c.map(|i| remap.get(i).copied().unwrap_or(0))).collect();
    // Edges lying on a patch border: (axis of the line, line, from, to, from vertex, to vertex).
    let inner = |cuts: &[f64]| -> Vec<i64> { cuts.iter().skip(1).take(cuts.len().saturating_sub(2)).map(|&c| q(c)).collect() };
    let (lines_u, lines_v) = (inner(us), inner(vs));
    let mut edges: Vec<(usize, i64, f64, f64, usize, usize)> = Vec::new();
    for c in &quads {
        let [tl, tr, bl, br] = *c;
        for (a, b) in [(tl, bl), (tr, br)] {
            if let (Some(pa), Some(pb)) = (uv.get(a), uv.get(b))
                && q(pa[0]) == q(pb[0])
                && lines_u.contains(&q(pa[0]))
            {
                edges.push((0, q(pa[0]), pa[1], pb[1], a, b));
            }
        }
        for (a, b) in [(tl, tr), (bl, br)] {
            if let (Some(pa), Some(pb)) = (uv.get(a), uv.get(b))
                && q(pa[1]) == q(pb[1])
                && lines_v.contains(&q(pa[1]))
            {
                edges.push((1, q(pa[1]), pa[0], pb[0], a, b));
            }
        }
    }
    // Longest edges first, so an edge's own ends are already where they belong.
    edges.sort_by(|a, b| (b.3 - b.2).abs().total_cmp(&(a.3 - a.2).abs()));
    // The points on each border line, with their place along it.
    let mut on_line: HashMap<(usize, i64), Vec<(f64, usize)>> = HashMap::new();
    for (k, p) in uv.iter().enumerate() {
        if lines_u.contains(&q(p[0])) {
            on_line.entry((0, q(p[0]))).or_default().push((p[1], k));
        }
        if lines_v.contains(&q(p[1])) {
            on_line.entry((1, q(p[1]))).or_default().push((p[0], k));
        }
    }
    let mut moved = vec![false; uv.len()];
    for &(axis, line, from, to, a, b) in &edges {
        let (lo, hi) = (from.min(to), from.max(to));
        let Some(points) = on_line.get(&(axis, line)) else { continue };
        for &(t, k) in points {
            if t <= lo + 1e-12 || t >= hi - 1e-12 || moved.get(k).copied().unwrap_or(true) {
                continue;
            }
            let (Some(&pa), Some(&pb)) = (pos.get(a), pos.get(b)) else { continue };
            let s = (t - from) / (to - from);
            if let Some(slot) = pos.get_mut(k) {
                *slot = [pa[0] + (pb[0] - pa[0]) * s, pa[1] + (pb[1] - pa[1]) * s];
            }
            if let Some(m) = moved.get_mut(k) {
                *m = true;
            }
        }
    }
    crate::warp_tess::EnvelopeMesh { uv, pos, quads }
}

/// The sorted, deduplicated cuts of `0..1` from `inner` (values outside or non-finite dropped).
fn cut_list(inner: &[f64]) -> Vec<f64> {
    let mut c = vec![0.0, 1.0];
    c.extend(inner.iter().copied().filter(|t| t.is_finite() && *t > 0.0 && *t < 1.0));
    c.sort_by(f64::total_cmp);
    c.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    c
}

/// The [`START_CELLS`] spans a patch span (`[from, to]`) starts as.
fn start_cells(w: &[f64]) -> Vec<[f64; 2]> {
    let (a, b) = (w.first().copied().unwrap_or(0.0), w.last().copied().unwrap_or(0.0));
    let at = |k: usize| a + (b - a) * k as f64 / START_CELLS as f64;
    (0..START_CELLS).map(|k| [at(k), at(k + 1)]).collect()
}

/// The leaf cells of the adaptive tessellation, in row order (top to bottom, then left to right).
fn tessellate(at: &dyn Fn(f64, f64) -> [f64; 2], cuts_u: &[f64], cuts_v: &[f64], sw: f64, sh: f64, facets: Facets) -> Vec<Cell> {
    let (tol, centre) = match facets {
        Facets::Envelope => (FLAT_TOL, true),
        Facets::Mesh => (MESH_TOL, false),
    };
    let gap = |p: [f64; 2], a: [f64; 2], b: [f64; 2]| {
        let d = (p[0] - (a[0] + b[0]) / 2.0).hypot(p[1] - (a[1] + b[1]) / 2.0);
        // A non-finite surface can't be judged: leave the cell as it is.
        if d.is_finite() { d } else { 0.0 }
    };
    let (us, vs) = (cut_list(cuts_u), cut_list(cuts_v));
    let mut queue: std::collections::VecDeque<Cell> = std::collections::VecDeque::new();
    for v in vs.windows(2).flat_map(start_cells) {
        for u in us.windows(2).flat_map(start_cells) {
            queue.push_back([u[0], u[1], v[0], v[1]]);
        }
    }
    let mut leaves = Vec::new();
    // Breadth first, so a cell budget running out leaves the whole surface equally fine.
    while let Some(c) = queue.pop_front() {
        let [u0, u1, v0, v1] = c;
        let (um, vm) = ((u0 + u1) / 2.0, (v0 + v1) / 2.0);
        let (p00, p10, p01, p11) = (at(u0, v0), at(u1, v0), at(u0, v1), at(u1, v1));
        let eu = gap(at(um, v0), p00, p10).max(gap(at(um, v1), p01, p11));
        let ev = gap(at(u0, vm), p00, p01).max(gap(at(u1, vm), p10, p11));
        let mid = [(p00[0] + p10[0] + p01[0] + p11[0]) / 4.0, (p00[1] + p10[1] + p01[1] + p11[1]) / 4.0];
        let ec = if centre { gap(at(um, vm), mid, mid) } else { 0.0 };
        let room = leaves.len() + queue.len() + 3 < MAX_LEAVES;
        let split_u = room && (eu > tol || ec > tol) && (u1 - u0) * sw > MIN_CELL;
        let split_v = room && (ev > tol || ec > tol) && (v1 - v0) * sh > MIN_CELL;
        if !split_u && !split_v {
            leaves.push(c);
            continue;
        }
        let su: &[[f64; 2]] = if split_u { &[[u0, um], [um, u1]] } else { &[[u0, u1]] };
        let sv: &[[f64; 2]] = if split_v { &[[v0, vm], [vm, v1]] } else { &[[v0, v1]] };
        for v in sv {
            for u in su {
                queue.push_back([u[0], u[1], v[0], v[1]]);
            }
        }
    }
    leaves.sort_by(|a, b| a[2].total_cmp(&b[2]).then(a[0].total_cmp(&b[0])));
    leaves
}

/// The vertices (`(destination, source)`, as [`warp_triangles`] takes them) and triangles of
/// `leaves`. `rect` is the source rectangle as `[x0, y0, width, height]`. A corner lying inside
/// a bigger cell's edge is moved onto that straight edge.
fn cell_triangles(at: &dyn Fn(f64, f64) -> [f64; 2], leaves: &[Cell], rect: [f64; 4], fold: &Fold) -> TriMesh {
    use std::collections::HashMap;
    let mut index: HashMap<(u64, u64), usize> = HashMap::new();
    let mut uv: Vec<[f64; 2]> = Vec::new();
    let mut corners = Vec::with_capacity(leaves.len());
    for &[u0, u1, v0, v1] in leaves {
        let mut id = |u: f64, v: f64| {
            *index.entry((u.to_bits(), v.to_bits())).or_insert_with(|| {
                uv.push([u, v]);
                uv.len() - 1
            })
        };
        corners.push([id(u0, v0), id(u1, v0), id(u0, v1), id(u1, v1)]);
    }
    let mut pos: Vec<[f64; 2]> = uv.iter().map(|p| at(p[0], p[1])).collect();
    // Vertices along each vertical line (by u) and horizontal line (by v), sorted along it.
    let mut on_u: HashMap<u64, Vec<(f64, usize)>> = HashMap::new();
    let mut on_v: HashMap<u64, Vec<(f64, usize)>> = HashMap::new();
    for (i, p) in uv.iter().enumerate() {
        on_u.entry(p[0].to_bits()).or_default().push((p[1], i));
        on_v.entry(p[1].to_bits()).or_default().push((p[0], i));
    }
    for line in on_u.values_mut().chain(on_v.values_mut()) {
        line.sort_by(|a, b| a.0.total_cmp(&b.0));
    }
    // Biggest cells first, so an edge's own ends are already where they belong.
    let mut order: Vec<usize> = (0..leaves.len()).collect();
    let size = |c: &Cell| (c[1] - c[0]).max(c[3] - c[2]);
    order.sort_by(|&a, &b| size(&leaves[b]).total_cmp(&size(&leaves[a])));
    for k in order {
        let ([u0, u1, v0, v1], [c00, c10, c01, c11]) = (leaves[k], corners[k]);
        let edges = [
            (on_v.get(&v0.to_bits()), u0, u1, c00, c10),
            (on_v.get(&v1.to_bits()), u0, u1, c01, c11),
            (on_u.get(&u0.to_bits()), v0, v1, c00, c01),
            (on_u.get(&u1.to_bits()), v0, v1, c10, c11),
        ];
        for (line, lo, hi, a, b) in edges {
            let Some(line) = line else { continue };
            let start = line.partition_point(|p| p.0 <= lo);
            for &(t, i) in line.iter().skip(start).take_while(|p| p.0 < hi) {
                let s = (t - lo) / (hi - lo);
                let (pa, pb) = (pos[a], pos[b]);
                if let Some(p) = pos.get_mut(i) {
                    *p = [pa[0] + (pb[0] - pa[0]) * s, pa[1] + (pb[1] - pa[1]) * s];
                }
            }
        }
    }
    let [x0, y0, sw, sh] = rect;
    let verts: Vec<([f64; 2], [f64; 2])> = uv.iter().zip(&pos).map(|(p, &d)| (d, [x0 + sw * p[0], y0 + sh * p[1]])).collect();
    let (tris, rank) = ordered_triangles(&verts, &pos, &corners, fold);
    (verts, tris, rank)
}

/// The triangles of the quads `corners` (`[top-left, top-right, bottom-left, bottom-right]` vertex
/// indices), two each split from the top-left corner, in the quads' order, and each one's rank in
/// a fold's order: for [`Fold::Stack`] a cell turned over on top, else the cell moved furthest (as
/// [`cell_order`]); the quads' order otherwise.
fn ordered_triangles(verts: &[([f64; 2], [f64; 2])], pos: &[[f64; 2]], corners: &[[usize; 4]], fold: &Fold) -> RankedMesh {
    let mut order: Vec<usize> = (0..corners.len()).collect();
    if let Fold::Stack(rest) = fold {
        // As [`cell_order`]: a cell turned over on top, else the cell moved furthest.
        let at_rest: Vec<[f64; 2]> = verts.iter().map(|(_, s)| rest(s[0], s[1])).map(|(x, y)| [x, y]).collect();
        let moved =
            |k: usize| Some(pos[k][0] - at_rest[k][0]).map(|dx| dx.hypot(pos[k][1] - at_rest[k][1])).filter(|m| m.is_finite()).unwrap_or(f64::NEG_INFINITY);
        let area = |pts: &[[f64; 2]], q: [usize; 4]| {
            let q = q.map(|k| pts[k]);
            (0..4).map(|i| q[i][0] * q[(i + 1) % 4][1] - q[(i + 1) % 4][0] * q[i][1]).sum::<f64>()
        };
        let key: Vec<(bool, f64)> = corners
            .iter()
            .map(|&[c00, c10, c01, c11]| {
                let q = [c00, c10, c11, c01];
                (area(pos, q) * area(&at_rest, q) < 0.0, q.iter().map(|&k| moved(k)).sum())
            })
            .collect();
        order.sort_by(|&a, &b| key[a].0.cmp(&key[b].0).then(key[a].1.total_cmp(&key[b].1)));
    }
    // The triangles in the quads' own order, each ranked where its quad comes in `order`.
    let tris = corners.iter().flat_map(|&[c00, c10, c01, c11]| [[c00, c10, c11], [c00, c11, c01]]).collect();
    let mut rank = vec![0u32; corners.len() * 2];
    for (r, &k) in order.iter().enumerate() {
        for (t, v) in [(2 * k, 2 * r), (2 * k + 1, 2 * r + 1)] {
            if let Some(slot) = rank.get_mut(t) {
                *slot = u32::try_from(v).unwrap_or(u32::MAX);
            }
        }
    }
    (tris, rank)
}

/// Triangles in drawing order without a fold, and each one's rank where a fold stacks them.
type RankedMesh = (Vec<[usize; 3]>, Vec<u32>);

/// The patch borders of `warp` inside `src_rect`, normalised to it (`(u cuts, v cuts)`), for
/// [`warp_patches_surface`]: a preset is one patch; a custom or cylinder mesh has its own.
pub fn patch_cuts(warp: &photocraft_geom::warp::Warp, src_rect: Rect) -> (Vec<f64>, Vec<f64>) {
    use photocraft_geom::warp::WarpStyle;
    let Some(m) = warp.mesh.as_ref().filter(|_| matches!(warp.style, WarpStyle::Custom | WarpStyle::Cylinder)) else {
        return (Vec::new(), Vec::new());
    };
    let b = warp.bounds;
    let (x0, y0) = (f64::from(src_rect.x0), f64::from(src_rect.y0));
    let (sw, sh) = (f64::from(src_rect.width()), f64::from(src_rect.height()));
    if sw <= 0.0 || sh <= 0.0 {
        return (Vec::new(), Vec::new());
    }
    let u = m.us.iter().map(|s| (b[0] + s * (b[2] - b[0]) - x0) / sw).collect();
    let v = m.vs.iter().map(|t| (b[1] + t * (b[3] - b[1]) - y0) / sh).collect();
    (u, v)
}

/// Probe lines per axis [`mapped_lengths`] follows through the map.
const PROBE: usize = 16;

/// The longest destination length of a source row and of a source column under `f`, measured
/// along polylines through a [`PROBE`]-step grid. Non-finite steps count as zero.
fn mapped_lengths(src_rect: Rect, f: &(dyn Fn(f64, f64) -> (f64, f64) + Sync)) -> (f64, f64) {
    let (x0, y0) = (f64::from(src_rect.x0), f64::from(src_rect.y0));
    let (sw, sh) = (f64::from(src_rect.width()), f64::from(src_rect.height()));
    let n = PROBE + 1;
    let mut pts = Vec::with_capacity(n * n);
    for j in 0..n {
        for i in 0..n {
            let (dx, dy) = f(x0 + sw * i as f64 / PROBE as f64, y0 + sh * j as f64 / PROBE as f64);
            pts.push([dx, dy]);
        }
    }
    let step = |a: [f64; 2], b: [f64; 2]| {
        let d = (b[0] - a[0]).hypot(b[1] - a[1]);
        if d.is_finite() { d } else { 0.0 }
    };
    let (mut lu, mut lv) = (0.0f64, 0.0f64);
    for k in 0..n {
        let (mut row, mut col) = (0.0, 0.0);
        for m in 1..n {
            if let (Some(&a), Some(&b)) = (pts.get(k * n + m - 1), pts.get(k * n + m)) {
                row += step(a, b);
            }
            if let (Some(&a), Some(&b)) = (pts.get((m - 1) * n + k), pts.get(m * n + k)) {
                col += step(a, b);
            }
        }
        lu = lu.max(row);
        lv = lv.max(col);
    }
    (lu, lv)
}

/// Rasterizes textured triangles: each vertex is `(destination, source)` in document pixels and
/// each triangle samples `src` (restricted to `src_rect`) through the barycentric
/// interpolation of its source positions. Later triangles in `tris` draw over earlier ones (the
/// order is the depth order). Output has `src`'s format with alpha added.
pub fn warp_triangles(src: &Surface, src_rect: Rect, verts: &[([f64; 2], [f64; 2])], tris: &[[usize; 3]], interp: Interp) -> Surface {
    triangles(src, src_rect, verts, tris, interp, &Interrupt::NONE, Draw::default())
}

/// [`warp_triangles`], reporting progress per destination tile to `ctl`. A cancelled `ctl` skips
/// the remaining tiles; the caller discards the result.
/// Destination tiles one triangle render may keep bookkeeping for (a 1M-tile render is 69 Gpx).
const MAX_DST_TILES: u64 = 1 << 20;
/// How close (source px) two triangles' source points under one pixel are when they are the
/// same layer of the picture meeting on a shared edge, rather than two layers of a fold.
const SAME_LAYER: f64 = 1e-3;
/// Source pixels one destination tile may read densely.
const MAX_FOOTPRINT_PIXELS: u64 = 1 << 25;
/// How far (px) past a border edge an envelope's fringe reaches (see `Draw::fringe`).
const FRINGE: f64 = 0.5;
/// Coarsest mip level a [`Sampling::Single`] warp samples (a 1024× reduction).
const MAX_MIP_LEVEL: u32 = 10;

/// The mip level a single-sample warp reads where one destination pixel to the right steps `jx`
/// through the source: `floor(log2 |jx|)`, 0 below a 2× reduction. The reference application's
/// warp probes pick it from that step alone: a strong vertical squeeze (a top or bottom fold)
/// stays at full resolution.
fn mip_level(jx: [f64; 2]) -> u32 {
    let l = jx[0].hypot(jx[1]);
    if l.is_finite() && l >= 2.0 { (l.log2().floor() as u32).min(MAX_MIP_LEVEL) } else { 0 }
}

/// Box-filtered premultiplied levels `1..` of a window whose origin lies on the level grid of the
/// whole source: level `k`'s texel `(i, j)` averages the texels of level `k - 1` below it, as one
/// pyramid of the whole source would. A block the source's far edge cuts averages only the texels
/// inside it (an odd-sized source's last row and column keep their alpha, as the reference
/// application's probes at 99 and 90 px show); past the source, a tap reads transparency.
/// With `steps`, every level is rounded to that many steps per unit, as the reference application
/// keeps an 8-bit source's levels in 8 bits (its renders match only so).
fn mip_levels(px: &[f32], fw: usize, fh: usize, n: usize, levels: u32, steps: Option<f32>) -> Vec<(Vec<f32>, usize, usize)> {
    let mut out: Vec<(Vec<f32>, usize, usize)> = Vec::new();
    for _ in 0..levels {
        let (prev, pw, ph) = match out.last() {
            Some((p, w, h)) => (p.as_slice(), *w, *h),
            None => (px, fw, fh),
        };
        let (nw, nh) = (pw.div_ceil(2), ph.div_ceil(2));
        let mut next = vec![0.0f32; nw * nh * n];
        for y in 0..nh {
            for x in 0..nw {
                let o = (y * nw + x) * n;
                let (kw, kh) = (if 2 * x + 1 < pw { 2 } else { 1 }, if 2 * y + 1 < ph { 2 } else { 1 });
                let k = 1.0 / (kw * kh) as f32;
                for (sy, sx) in [(2 * y, 2 * x), (2 * y, 2 * x + 1), (2 * y + 1, 2 * x), (2 * y + 1, 2 * x + 1)] {
                    if sx < pw && sy < ph {
                        let i = (sy * pw + sx) * n;
                        if let (Some(d), Some(s)) = (next.get_mut(o..o + n), prev.get(i..i + n)) {
                            for (d, s) in d.iter_mut().zip(s) {
                                *d += k * s;
                            }
                        }
                    }
                }
            }
        }
        if let Some(k) = steps {
            for v in &mut next {
                *v = (*v * k).round() / k;
            }
        }
        out.push((next, nw, nh));
    }
    out
}

/// The fringe of [`Draw::fringe`] in tile `t` (`w` px wide): fills the uncovered pixels of `uv` and
/// `owner` within [`FRINGE`] px of a border edge from the nearest one's triangle, and returns each
/// pixel's alpha cap (1 elsewhere).
fn fringe(tris: &[[usize; 3]], verts: &[([f64; 2], [f64; 2])], src_rect: Rect, t: &Rect, w: usize, uv: &mut [[f64; 2]], owner: &mut [u32]) -> Vec<f64> {
    let mut cover = vec![1.0f64; uv.len()];
    let (sx0, sy0, sx1, sy1) = (f64::from(src_rect.x0), f64::from(src_rect.y0), f64::from(src_rect.x1), f64::from(src_rect.y1));
    let on = |a: f64, b: f64, e: f64| (a - e).abs() < 1e-6 && (b - e).abs() < 1e-6;
    let on_border = |a: [f64; 2], b: [f64; 2]| on(a[0], b[0], sx0) || on(a[0], b[0], sx1) || on(a[1], b[1], sy0) || on(a[1], b[1], sy1);
    let mut near = vec![(FRINGE, [f64::NAN; 2], 0u32); uv.len()];
    for (k, tri) in tris.iter().enumerate() {
        let [(p0, s0), (p1, s1), (p2, s2)] = tri.map(|v| verts[v]);
        let det = (p1[0] - p0[0]) * (p2[1] - p0[1]) - (p2[0] - p0[0]) * (p1[1] - p0[1]);
        if det.abs() < 1e-12 {
            continue;
        }
        let border: Vec<([f64; 2], [f64; 2])> =
            [(p0, p1, s0, s1), (p1, p2, s1, s2), (p2, p0, s2, s0)].iter().filter(|e| on_border(e.2, e.3)).map(|e| (e.0, e.1)).collect();
        if border.is_empty() {
            continue;
        }
        let bx0 = (p0[0].min(p1[0]).min(p2[0]) - 1.0).floor().max(f64::from(t.x0)) as i32;
        let by0 = (p0[1].min(p1[1]).min(p2[1]) - 1.0).floor().max(f64::from(t.y0)) as i32;
        let bx1 = (p0[0].max(p1[0]).max(p2[0]) + 1.0).ceil().min(f64::from(t.x1)) as i32;
        let by1 = (p0[1].max(p1[1]).max(p2[1]) + 1.0).ceil().min(f64::from(t.y1)) as i32;
        for y in by0..by1 {
            let py = f64::from(y) + 0.5;
            for x in bx0..bx1 {
                let o = (y - t.y0) as usize * w + (x - t.x0) as usize;
                if !uv.get(o).is_some_and(|q| q[0].is_nan()) {
                    continue;
                }
                let px = f64::from(x) + 0.5;
                let d = border
                    .iter()
                    .map(|&(a, b)| {
                        let ab = [b[0] - a[0], b[1] - a[1]];
                        let l = ab[0] * ab[0] + ab[1] * ab[1];
                        let s = if l > 0.0 { (((px - a[0]) * ab[0] + (py - a[1]) * ab[1]) / l).clamp(0.0, 1.0) } else { 0.0 };
                        (px - a[0] - s * ab[0]).hypot(py - a[1] - s * ab[1])
                    })
                    .fold(f64::INFINITY, f64::min);
                let Some(slot) = near.get_mut(o) else { continue };
                if d < slot.0 {
                    let l1 = ((px - p0[0]) * (p2[1] - p0[1]) - (p2[0] - p0[0]) * (py - p0[1])) / det;
                    let l2 = ((p1[0] - p0[0]) * (py - p0[1]) - (px - p0[0]) * (p1[1] - p0[1])) / det;
                    let l0 = 1.0 - l1 - l2;
                    *slot = (d, [l0 * s0[0] + l1 * s1[0] + l2 * s2[0], l0 * s0[1] + l1 * s1[1] + l2 * s2[1]], k as u32);
                }
            }
        }
    }
    for (o, (d, q, k)) in near.into_iter().enumerate() {
        if !q[0].is_nan()
            && let (Some(u), Some(ow), Some(c)) = (uv.get_mut(o), owner.get_mut(o), cover.get_mut(o))
        {
            *u = q;
            *ow = k;
            *c = 0.5 - d;
        }
    }
    cover
}

/// Scales premultiplied `px` (alpha last) down so its alpha is at most `cap` (none: unchanged).
fn cap_alpha(px: &mut [f64], cap: Option<f64>) {
    let (Some(c), Some(&al)) = (cap, px.last()) else { return };
    if al > c {
        let k = if al > 0.0 { c.max(0.0) / al } else { 0.0 };
        for v in px.iter_mut() {
            *v *= k;
        }
    }
}

/// Puts a pixel's previous top layer `top` (source position, triangle) on the end of its list of
/// fold layers in `under` (linked from `head`, see [`Draw::stack`]), first taking out any layer the
/// new top is part of (`same`).
fn stack_layer(under: &mut Vec<([f64; 2], u32, u32)>, head: &mut u32, top: ([f64; 2], u32), same: &dyn Fn([f64; 2]) -> bool) {
    let Ok(node) = u32::try_from(under.len()) else { return };
    let (mut at, mut last) = (*head, u32::MAX);
    *head = u32::MAX;
    while let Some(&(p, _, next)) = under.get(at as usize) {
        if same(p) {
            if let Some(e) = under.get_mut(at as usize) {
                e.0 = [f64::NAN; 2];
            }
        } else {
            match under.get_mut(last as usize) {
                Some(prev) => prev.2 = at,
                None => *head = at,
            }
            last = at;
        }
        at = next;
    }
    under.push((top.0, top.1, u32::MAX));
    match under.get_mut(last as usize) {
        Some(prev) => prev.2 = node,
        None => *head = node,
    }
}

/// How [`triangles`] draws a warp.
#[derive(Clone, Copy, Default)]
struct Draw<'a> {
    /// One sample per pixel (from a mip level where the warp squeezes rows), not a footprint.
    single: bool,
    /// Pixels just outside an edge of the mesh that lies on the source's border are drawn too, from
    /// the nearest such triangle's map carried past the edge, their alpha at most `0.5 − d` for a
    /// centre `d` px out (and the source's transparent surround lowers it further): the outline
    /// the reference application's warp renders show.
    fringe: bool,
    /// The picture can fold over itself: each triangle's rank in the order a fold stacks them
    /// (asked once, when a tile first needs it). A tile is drawn in the triangles' own order;
    /// where two parts of the source meet at a pixel (a fold), the tile is drawn again in rank
    /// order, and triangles drawn later composite over what they cover instead of replacing it.
    /// Without a fold the order makes no difference, so a warp that doesn't fold costs no more
    /// than one that can't.
    stack: Option<&'a (dyn Fn() -> Vec<u32> + Sync)>,
}

/// Source points a destination tile samples, grouped into connected regions of the source, so
/// each region is read as a window of its own: a fold joins distant parts of the source in one
/// tile, and one window over both read everything between them (a corner dragged over the
/// middle of a 24 MP layer read seven times its pixels). Points are joined through the cells of a
/// coarse grid over their bounding box that their footprints touch.
struct Regions {
    /// The grid's origin, cell size, columns and rows.
    origin: [f64; 2],
    cell: f64,
    cols: usize,
    rows: usize,
    /// Each cell's region (`u32::MAX`: none). Empty for a single region, which every point is in.
    of_cell: Vec<u32>,
    /// Each region's source rectangle (the points' footprints, a 3 px border, inside `src_rect`).
    rects: Vec<Rect>,
}

/// Visits points for [`Regions::new`]: each with the half extent of its footprint along x and y.
type VisitPoints<'a> = &'a dyn Fn(&mut dyn FnMut([f64; 2], [f64; 2]));

/// Most cells per axis of a [`Regions`] grid.
const REGION_GRID: f64 = 256.0;
/// Smallest [`Regions`] cell (source px).
const REGION_CELL: f64 = 32.0;
/// A [`Regions`] bounding box this small (source px) stays one region.
const SINGLE_REGION_PIXELS: f64 = 1024.0 * 1024.0;

impl Regions {
    /// `points` visits each point with the half extent of its footprint along x and y (it is
    /// called again only when the points form more than one region).
    fn new(points: VisitPoints, src_rect: Rect) -> Regions {
        let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        let mut count = 0usize;
        points(&mut |q, e| {
            b = [b[0].min(q[0] - e[0]), b[1].min(q[1] - e[1]), b[2].max(q[0] + e[0]), b[3].max(q[1] + e[1])];
            count += 1;
        });
        let empty = Regions { origin: [0.0; 2], cell: 1.0, cols: 0, rows: 0, of_cell: Vec::new(), rects: Vec::new() };
        if !(b[0] <= b[2] && b.iter().all(|v| v.is_finite())) {
            return empty;
        }
        let rect =
            |r: [f64; 4]| Rect::new(r[0].floor() as i32 - 3, r[1].floor() as i32 - 3, r[2].ceil() as i32 + 3, r[3].ceil() as i32 + 3).intersect(&src_rect);
        // Most tiles sample one stretch of the source: no more than a few times the source
        // pixels there are points (the footprints' own spread), or a small window, is one region.
        let area = (b[2] - b[0] + 7.0) * (b[3] - b[1] + 7.0);
        if area <= SINGLE_REGION_PIXELS.max(4.0 * count as f64) {
            return Regions { rects: vec![rect(b)], ..empty };
        }
        let cell = ((b[2] - b[0]).max(b[3] - b[1]) / REGION_GRID).max(REGION_CELL);
        let (cols, rows) = (((b[2] - b[0]) / cell).floor() as usize + 1, ((b[3] - b[1]) / cell).floor() as usize + 1);
        let to = |v: f64, o: f64, n: usize| (((v - o) / cell).floor().max(0.0) as usize).min(n - 1);
        // Cells a footprint touches, marked (0) then labelled by region.
        let mut of_cell = vec![u32::MAX; cols * rows];
        points(&mut |q, e| {
            for y in to(q[1] - e[1], b[1], rows)..=to(q[1] + e[1], b[1], rows) {
                for x in to(q[0] - e[0], b[0], cols)..=to(q[0] + e[0], b[0], cols) {
                    if let Some(c) = of_cell.get_mut(y * cols + x) {
                        *c = 0;
                    }
                }
            }
        });
        let mut regions = 0u32;
        let mut stack = Vec::new();
        for start in 0..of_cell.len() {
            if of_cell.get(start) != Some(&0) {
                continue;
            }
            regions += 1;
            stack.push(start);
            while let Some(c) = stack.pop() {
                if of_cell.get(c) != Some(&0) {
                    continue;
                }
                of_cell[c] = regions;
                let (x, y) = ((c % cols) as i64, (c / cols) as i64);
                for (dx, dy) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx >= 0 && ny >= 0 && (nx as usize) < cols && (ny as usize) < rows {
                        stack.push(ny as usize * cols + nx as usize);
                    }
                }
            }
        }
        // Labels 1.. become indices 0..; each region's rectangle covers its points' footprints.
        for c in of_cell.iter_mut().filter(|c| **c != u32::MAX) {
            *c -= 1;
        }
        let mut boxes = vec![[f64::MAX, f64::MAX, f64::MIN, f64::MIN]; regions as usize];
        let mut out = Regions { origin: [b[0], b[1]], cell, cols, rows, of_cell, rects: Vec::new() };
        points(&mut |q, e| {
            if let Some(r) = out.of(q).and_then(|r| boxes.get_mut(r)) {
                *r = [r[0].min(q[0] - e[0]), r[1].min(q[1] - e[1]), r[2].max(q[0] + e[0]), r[3].max(q[1] + e[1])];
            }
        });
        out.rects = boxes.into_iter().map(rect).collect();
        out
    }

    /// The region of a point given to [`Regions::new`].
    fn of(&self, q: [f64; 2]) -> Option<usize> {
        if self.of_cell.is_empty() {
            return (!self.rects.is_empty()).then_some(0);
        }
        if !(q[0].is_finite() && q[1].is_finite()) {
            return None;
        }
        let x = (((q[0] - self.origin[0]) / self.cell).floor().max(0.0) as usize).min(self.cols - 1);
        let y = (((q[1] - self.origin[1]) / self.cell).floor().max(0.0) as usize).min(self.rows - 1);
        self.of_cell.get(y * self.cols + x).filter(|&&r| r != u32::MAX).map(|&r| r as usize)
    }

    /// Source pixels all the regions' rectangles hold.
    fn area(&self) -> u64 {
        self.rects.iter().map(|r| r.size().area()).sum()
    }
}

/// A premultiplied window of the source that a destination tile samples.
struct Win {
    px: Vec<f32>,
    foot: Rect,
    fw: usize,
    /// Its mip levels `1..` (see [`mip_levels`]): texels, width, height.
    mips: Vec<(Vec<f32>, usize, usize)>,
}

fn triangles(src: &Surface, src_rect: Rect, verts: &[([f64; 2], [f64; 2])], tris: &[[usize; 3]], interp: Interp, ctl: &Interrupt, draw: Draw) -> Surface {
    let (single, folds) = (draw.single, draw.stack.is_some());
    let ranks = std::sync::OnceLock::new();
    let rank_of = |t: u32| draw.stack.map(|f| ranks.get_or_init(f)).and_then(|r| r.get(t as usize)).copied().unwrap_or(u32::MAX);
    let mut fmt = src.format();
    let converted;
    let src = if fmt.alpha {
        src
    } else {
        fmt = PixelFormat::new(fmt.mode, fmt.sample, true);
        converted = src.convert(fmt);
        &converted
    };
    let mut out = Surface::new(fmt);
    if src_rect.is_empty() || tris.is_empty() {
        return out;
    }
    if verts.iter().any(|v| !v.0[0].is_finite() || !v.0[1].is_finite()) {
        return out;
    }
    let lim = f64::from(1 << 20);
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for v in tris.iter().flatten().map(|&i| &verts[i]) {
        x0 = x0.min(v.0[0]);
        y0 = y0.min(v.0[1]);
        x1 = x1.max(v.0[0]);
        y1 = y1.max(v.0[1]);
    }
    let dst = Rect::new(x0.max(-lim).floor() as i32 - 1, y0.max(-lim).floor() as i32 - 1, x1.min(lim).ceil() as i32 + 1, y1.min(lim).ceil() as i32 + 1);
    // Bookkeeping is kept per destination tile, so a mesh flung across the whole coordinate range
    // would ask for millions of entries before drawing anything: draw nothing instead.
    let ts_px = photocraft_geom::TILE_SIZE as u64;
    if u64::from(dst.width()).div_ceil(ts_px).saturating_add(1).saturating_mul(u64::from(dst.height()).div_ceil(ts_px).saturating_add(1)) > MAX_DST_TILES {
        return out;
    }
    // Triangles: (vertex indices), binned per destination tile.
    let tiles: Vec<Rect> = dst.tiles().map(|tc| tc.rect().intersect(&dst)).filter(|r| !r.is_empty()).collect();
    // Triangles by index into `tris`, in its order.
    let mut bins: Vec<Vec<u32>> = vec![Vec::new(); tiles.len()];
    let ts = photocraft_geom::TILE_SIZE;
    // Bin origin on the tile grid (the first tile is clipped to `dst`, so its x0/y0 may not be).
    let tx0 = tiles.iter().map(|t| t.x0).min().unwrap_or(0).div_euclid(ts) * ts;
    let ty0 = tiles.iter().map(|t| t.y0).min().unwrap_or(0).div_euclid(ts) * ts;
    let cols = tiles.iter().map(|t| (t.x0 - tx0) / ts).max().unwrap_or(0) as usize + 1;
    let grid_of = |t: &Rect| ((t.y0 - ty0) / ts) as usize * cols + ((t.x0 - tx0) / ts) as usize;
    let mut lookup = vec![usize::MAX; cols * (tiles.iter().map(|t| (t.y0 - ty0) / ts).max().unwrap_or(0) as usize + 1)];
    for (k, t) in tiles.iter().enumerate() {
        lookup[grid_of(t)] = k;
    }
    for (index, &tri) in tris.iter().enumerate() {
        let Ok(index) = u32::try_from(index) else { break };
        let ps = tri.map(|v| verts[v].0);
        // A fringe reaches past the triangle, into a neighbouring tile.
        let grow = if draw.fringe { 1.0 } else { 0.0 };
        let bx0 = (ps.iter().map(|p| p[0]).fold(f64::MAX, f64::min) - grow).floor() as i32;
        let by0 = (ps.iter().map(|p| p[1]).fold(f64::MAX, f64::min) - grow).floor() as i32;
        let bx1 = (ps.iter().map(|p| p[0]).fold(f64::MIN, f64::max) + grow).ceil() as i32;
        let by1 = (ps.iter().map(|p| p[1]).fold(f64::MIN, f64::max) + grow).ceil() as i32;
        let (cx0, cy0) = ((bx0.max(dst.x0) - tx0).div_euclid(ts), (by0.max(dst.y0) - ty0).div_euclid(ts));
        let (cx1, cy1) = ((bx1.min(dst.x1 - 1) - tx0).div_euclid(ts), (by1.min(dst.y1 - 1) - ty0).div_euclid(ts));
        for cy in cy0.max(0)..=cy1 {
            for cx in cx0.max(0)..=cx1 {
                let g = cy as usize * cols + cx as usize;
                if let Some(&k) = lookup.get(g)
                    && k != usize::MAX
                {
                    bins[k].push(index);
                }
            }
        }
    }
    let n = fmt.channels();
    let a = n - 1;
    // Tiles finished so far, for progress reported per tile (from any worker thread).
    let finished = std::sync::atomic::AtomicUsize::new(0);
    let total = tiles.len().max(1);
    let work = |(t, bin): (&Rect, &Vec<u32>)| -> Option<(TileCoord, Arc<Tile>)> {
        if ctl.cancelled() {
            return None;
        }
        let k = finished.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        // The last few percent are putting the tiles in place.
        ctl.progress(0.98 * k as f32 / total as f32);
        if bin.is_empty() {
            return None;
        }
        let w = t.width() as usize;
        let h = t.height() as usize;
        // Subsamples along one destination axis, from the source step `j` it makes: one where
        // the warp keeps or enlarges the source, else about one per source pixel the output pixel
        // spans (an area average, as the transform path gets from its pre-reduction; a single
        // bicubic sample aliases thin lines when shrinking). The subsamples are bilinear: the
        // box does the filtering, and it is ~4x cheaper than bicubic at no loss. A smart object's
        // warp (`single`) takes one bilinear sample per pixel instead, from a mip level where it
        // squeezes the source along a row (see `mip_level`), as the reference application's
        // renders of its warps show.
        let count = |j: [f64; 2]| if interp == Interp::Nearest || single { 1 } else { j[0].hypot(j[1]).round().clamp(1.0, MAX_SS) as usize };
        // Source position per destination pixel (NaN = not covered) and the triangle it came
        // from. Per triangle: its source step per destination pixel along x and y (`[dx_u, dx_v,
        // dy_u, dy_v]`, the inverse map's Jacobian, constant over a triangle; zero when
        // point-sampled) and the subsamples along each.
        // The tile's triangles, first in `tris`' order, then, if a fold shows here, by rank.
        let mut order: Vec<[usize; 3]> = bin.iter().filter_map(|&k| tris.get(k as usize).copied()).collect();
        let mut stacked = false;
        let mut uv = vec![[f64::NAN; 2]; w * h];
        let mut owner = vec![0u32; w * h];
        // The layers of a fold under a pixel's top one, bottom first, as a list linked through
        // `under` (source position, triangle, next node or `u32::MAX`) from the node `under_at`
        // holds for the pixel (`u32::MAX`: none): one arena for the tile rather than a list per
        // folded pixel. A layer taken out again keeps its node, its position NaN.
        let mut under: Vec<([f64; 2], u32, u32)> = Vec::new();
        let mut under_at: Vec<u32> = Vec::new();
        let mut jac: Vec<([f64; 4], usize, usize)> = Vec::with_capacity(order.len());
        // A single-sample warp's mip level per triangle (see `mip_level`).
        let mut lev: Vec<u32> = Vec::with_capacity(order.len());
        'pass: loop {
            for (k, tri) in order.iter().enumerate() {
                let [(p0, s0), (p1, s1), (p2, s2)] = tri.map(|v| verts[v]);
                let det = (p1[0] - p0[0]) * (p2[1] - p0[1]) - (p2[0] - p0[0]) * (p1[1] - p0[1]);
                if det.abs() < 1e-12 {
                    jac.push(([0.0; 4], 1, 1));
                    lev.push(0);
                    continue;
                }
                let (d1, d2) = ([s1[0] - s0[0], s1[1] - s0[1]], [s2[0] - s0[0], s2[1] - s0[1]]);
                let (l1x, l1y, l2x, l2y) = ((p2[1] - p0[1]) / det, (p0[0] - p2[0]) / det, (p0[1] - p1[1]) / det, (p1[0] - p0[0]) / det);
                // Capped, so a sliver triangle can't ask for an unbounded footprint.
                let cap = |j: [f64; 2]| {
                    let l = j[0].hypot(j[1]);
                    if l > MAX_SS { [j[0] * MAX_SS / l, j[1] * MAX_SS / l] } else { j }
                };
                let raw_jx = [l1x * d1[0] + l2x * d2[0], l1x * d1[1] + l2x * d2[1]];
                lev.push(if single && interp == Interp::Bilinear { mip_level(raw_jx) } else { 0 });
                let jx = cap(raw_jx);
                let jy = cap([l1y * d1[0] + l2y * d2[0], l1y * d1[1] + l2y * d2[1]]);
                let (nx, ny) = (count(jx), count(jy));
                jac.push(if nx * ny == 1 { ([0.0; 4], 1, 1) } else { ([jx[0], jx[1], jy[0], jy[1]], nx, ny) });
                let bx0 = (p0[0].min(p1[0]).min(p2[0]) - 0.5).floor().max(f64::from(t.x0)) as i32;
                let by0 = (p0[1].min(p1[1]).min(p2[1]) - 0.5).floor().max(f64::from(t.y0)) as i32;
                let bx1 = (p0[0].max(p1[0]).max(p2[0]) + 0.5).ceil().min(f64::from(t.x1)) as i32;
                let by1 = (p0[1].max(p1[1]).max(p2[1]) + 0.5).ceil().min(f64::from(t.y1)) as i32;
                let eps = -1e-9;
                for y in by0..by1 {
                    let py = f64::from(y) + 0.5;
                    for x in bx0..bx1 {
                        let px = f64::from(x) + 0.5;
                        let l1 = ((px - p0[0]) * (p2[1] - p0[1]) - (p2[0] - p0[0]) * (py - p0[1])) / det;
                        let l2 = ((p1[0] - p0[0]) * (py - p0[1]) - (px - p0[0]) * (p1[1] - p0[1])) / det;
                        let l0 = 1.0 - l1 - l2;
                        if l0 < eps || l1 < eps || l2 < eps {
                            continue;
                        }
                        let o = (y - t.y0) as usize * w + (x - t.x0) as usize;
                        let q = [l0 * s0[0] + l1 * s1[0] + l2 * s2[0], l0 * s0[1] + l1 * s1[1] + l2 * s2[1]];
                        // Another part of the source: a fold. Triangles of one layer meeting at a pixel
                        // (on their shared edge) interpolate the same source point, to float noise,
                        // and replace each other; one drawn again after another layer comes back on
                        // top of it, as painting would put it. Layers of a tight fold can be well under
                        // a source pixel apart and still be layers.
                        let near = |a: [f64; 2]| (a[0] - q[0]).hypot(a[1] - q[1]) <= SAME_LAYER;
                        if folds && !stacked && !uv[o][0].is_nan() && !near(uv[o]) {
                            // A fold shows in this tile: draw it again in the fold's order.
                            let mut ranked: Vec<(u32, [usize; 3])> = bin.iter().filter_map(|&k| tris.get(k as usize).map(|&t| (rank_of(k), t))).collect();
                            ranked.sort_by_key(|e| e.0);
                            order = ranked.into_iter().map(|e| e.1).collect();
                            stacked = true;
                            uv.fill([f64::NAN; 2]);
                            under_at = vec![u32::MAX; w * h];
                            jac.clear();
                            lev.clear();
                            continue 'pass;
                        }
                        if stacked
                            && !uv[o][0].is_nan()
                            && !near(uv[o])
                            && let Some(head) = under_at.get_mut(o)
                        {
                            // This layer replaces a layer of itself already in the list; the old top goes on
                            // the end.
                            stack_layer(&mut under, head, (uv[o], owner[o]), &near);
                        }
                        uv[o] = q;
                        owner[o] = k as u32;
                    }
                }
            }
            break;
        }
        // The fringe (see `Draw::fringe`): per uncovered pixel, its distance to the nearest border
        // edge and that triangle's source position carried out to it.
        let cover = if draw.fringe { fringe(&order, verts, src_rect, t, w, &mut uv, &mut owner) } else { Vec::new() };
        // Where the covered pixels sample the source, with the spread of their subsamples (a mip
        // level's taps reach up to 1.5 texels of 2^level pixels from the sample), in regions read
        // apart ([`Regions`]): the top layer's and, apart, those of the fold layers under it. A
        // layer's windows start on the source's grid for its finest level, since mip levels are
        // built from the window: its blocks are the whole pyramid's (only the source's own far
        // edge leaves part blocks).
        let spread = |(q, k): ([f64; 2], u32)| {
            let j = jac[k as usize].0;
            let l = lev.get(k as usize).copied().unwrap_or(0);
            let m = if l > 0 { 1.5 * f64::from(1u32 << l) } else { 0.0 };
            (q, [0.5 * (j[0].abs() + j[2].abs()) + m, 0.5 * (j[1].abs() + j[3].abs()) + m])
        };
        let snapped = |foot: Rect, level: u32| -> Rect {
            if level == 0 || foot.is_empty() {
                return foot;
            }
            let b = 1i32 << level;
            let snap = |v: i32, o: i32| o + (v - o).div_euclid(b) * b;
            Rect::new(
                snap(foot.x0, src_rect.x0),
                snap(foot.y0, src_rect.y0),
                snap(foot.x1 - 1, src_rect.x0).saturating_add(b),
                snap(foot.y1 - 1, src_rect.y0).saturating_add(b),
            )
            .intersect(&src_rect)
        };
        // An 8-bit source's levels are 8-bit too; deeper ones keep their precision (unmeasured).
        let steps = (fmt.sample == photocraft_color::SampleType::U8).then_some(255.0);
        // Sparse components far apart in the source can land in one tile: the dense window
        // between them is not read (the tile stays empty). The pixel-area cap is checked first:
        // `has_tiles_in` walks the source tile grid, so an oversized footprint must return
        // without paying for that scan.
        let window = |(foot, levels): (Rect, u32)| -> Option<Win> {
            if foot.is_empty() || foot.size().area() > MAX_FOOTPRINT_PIXELS || !src.has_tiles_in(foot) {
                return None;
            }
            let mut px = src.read_region(foot);
            for p in px.chunks_exact_mut(n) {
                let al = p[a];
                for v in &mut p[..a] {
                    *v *= al;
                }
            }
            let fw = foot.width() as usize;
            let mips = if levels > 0 { mip_levels(&px, fw, foot.height() as usize, n, levels, steps) } else { Vec::new() };
            Some(Win { px, foot, fw, mips })
        };
        // A top layer over an empty part of the source has no window: it is transparent there,
        // and the fold layers under it still show. Too much to read leaves the tile empty, as
        // without folds: what is under it can't show through where it may be opaque.
        let top_level = uv.iter().zip(&owner).filter(|(q, _)| !q[0].is_nan()).map(|(_, &k)| lev.get(k as usize).copied().unwrap_or(0)).max().unwrap_or(0);
        let mut top_regions = Regions::new(
            &|f| {
                for (q, &k) in uv.iter().zip(&owner).filter(|(q, _)| !q[0].is_nan()) {
                    let (q, e) = spread((*q, k));
                    f(q, e);
                }
            },
            src_rect,
        );
        for r in &mut top_regions.rects {
            *r = snapped(*r, top_level);
        }
        if top_regions.area() > MAX_FOOTPRINT_PIXELS {
            return None;
        }
        let top: Vec<Option<Win>> = top_regions.rects.iter().map(|&r| window((r, top_level))).collect();
        // Fold layers whose source can't be read in windows of their own, within the one cap for
        // both, are left out (the top one stays).
        let below_level = under.iter().filter(|e| !e.0[0].is_nan()).map(|e| lev.get(e.1 as usize).copied().unwrap_or(0)).max().unwrap_or(0);
        let mut below_regions = Regions::new(
            &|f| {
                for &(q, k, _) in under.iter().filter(|e| !e.0[0].is_nan()) {
                    let (q, e) = spread((q, k));
                    f(q, e);
                }
            },
            src_rect,
        );
        for r in &mut below_regions.rects {
            *r = snapped(*r, below_level);
        }
        let room = MAX_FOOTPRINT_PIXELS.saturating_sub(top_regions.area());
        let below: Vec<Option<Win>> = if !under.is_empty() && below_regions.area() <= room {
            below_regions.rects.iter().map(|&r| window((r, below_level))).collect()
        } else {
            Vec::new()
        };
        if below.iter().all(Option::is_none) {
            under.clear();
        }
        if top.iter().all(Option::is_none) && under.is_empty() {
            return None;
        }
        // The window a point of the top layer, or of a fold layer under it, samples.
        let top_win = |q: [f64; 2]| top_regions.of(q).and_then(|r| top.get(r)).and_then(Option::as_ref);
        let below_win = |q: [f64; 2]| below_regions.of(q).and_then(|r| below.get(r)).and_then(Option::as_ref);
        let at = |win: &Win, x: i32, y: i32, c: usize| -> f64 {
            let foot = win.foot;
            if x < foot.x0 || y < foot.y0 || x >= foot.x1 || y >= foot.y1 {
                0.0
            } else {
                f64::from(win.px[((y - foot.y0) as usize * win.fw + (x - foot.x0) as usize) * n + c])
            }
        };
        // Offset of pixel (x, y) when the `taps` x `taps` block from it lies inside `foot`, so
        // the samplers can index `px` directly instead of bounds-checking every tap.
        let base = |win: &Win, x: i32, y: i32, taps: i32| -> Option<usize> {
            let foot = win.foot;
            (x >= foot.x0 && y >= foot.y0 && x + taps <= foot.x1 && y + taps <= foot.y1).then(|| ((y - foot.y0) as usize * win.fw + (x - foot.x0) as usize) * n)
        };
        let mut outp = vec![0.0f32; w * h * n];
        let mut any = false;
        let mut acc = [0.0f64; 8];
        let mut one = [0.0f64; 8];
        // One premultiplied sample at source position (u, v) (pixel centres at integers).
        let point = |win: &Win, interp: Interp, u: f64, v: f64, acc: &mut [f64; 8]| {
            acc[..n].fill(0.0);
            match interp {
                Interp::Nearest => {
                    let (ix, iy) = ((u + 0.5).floor() as i32, (v + 0.5).floor() as i32);
                    for (c, s) in acc.iter_mut().enumerate().take(n) {
                        *s = at(win, ix, iy, c);
                    }
                }
                Interp::Bilinear => {
                    let (ix, iy) = (u.floor() as i32, v.floor() as i32);
                    // The reference's weights: the fraction in 1/256 steps, over 255 (so the far
                    // sample never quite reaches full weight), as its warp probes show.
                    let (fx, fy) = (((u - f64::from(ix)) * 256.0).floor() / 255.0, ((v - f64::from(iy)) * 256.0).floor() / 255.0);
                    if let Some(o) = base(win, ix, iy, 2) {
                        for (dy, wy) in [(0, 1.0 - fy), (1, fy)] {
                            for (dx, wx) in [(0, 1.0 - fx), (1, fx)] {
                                let p = &win.px[o + (dy * win.fw + dx) * n..][..n];
                                for (s, v) in acc.iter_mut().zip(p) {
                                    *s += f64::from(*v) * wx * wy;
                                }
                            }
                        }
                        return;
                    }
                    for (dy, wy) in [(0, 1.0 - fy), (1, fy)] {
                        for (dx, wx) in [(0, 1.0 - fx), (1, fx)] {
                            for (c, s) in acc.iter_mut().enumerate().take(n) {
                                *s += at(win, ix + dx, iy + dy, c) * wx * wy;
                            }
                        }
                    }
                }
                Interp::Bicubic => {
                    // Snap to the pixel grid when within float noise, so identity is exact.
                    let snap = |t: f64| if (t - t.round()).abs() < 1e-7 { t.round() } else { t };
                    let (u, v) = (snap(u), snap(v));
                    let (ix, iy) = (u.floor() as i32, v.floor() as i32);
                    let (wx, wy) = (bicubic(u - f64::from(ix)), bicubic(v - f64::from(iy)));
                    if let Some(o) = base(win, ix - 1, iy - 1, 4) {
                        for (j, wyj) in wy.iter().enumerate() {
                            for (i, wxi) in wx.iter().enumerate() {
                                let k = wxi * wyj;
                                if k == 0.0 {
                                    continue;
                                }
                                let p = &win.px[o + (j * win.fw + i) * n..][..n];
                                for (s, v) in acc.iter_mut().zip(p) {
                                    *s += f64::from(*v) * k;
                                }
                            }
                        }
                        return;
                    }
                    for (j, wyj) in wy.iter().enumerate() {
                        for (i, wxi) in wx.iter().enumerate() {
                            let k = wxi * wyj;
                            if k == 0.0 {
                                continue;
                            }
                            for (c, s) in acc.iter_mut().enumerate().take(n) {
                                *s += at(win, ix - 1 + i as i32, iy - 1 + j as i32, c) * k;
                            }
                        }
                    }
                }
            }
        };
        // One premultiplied bilinear tap of `win`'s mip level `l` (>= 1) at source position `q`,
        // with the reference's 1/256-step weights; texels past the source are transparent.
        let mip_tap = |win: &Win, l: u32, q: [f64; 2], acc: &mut [f64; 8]| {
            acc[..n].fill(0.0);
            let Some((m, mw, mh)) = win.mips.get(l as usize - 1) else { return };
            let sc = f64::from(1u32 << l);
            let (cx, cy) = ((q[0] - f64::from(win.foot.x0)) / sc - 0.5, (q[1] - f64::from(win.foot.y0)) / sc - 0.5);
            let (ix, iy) = (cx.floor(), cy.floor());
            let (fx, fy) = (((cx - ix) * 256.0).floor() / 255.0, ((cy - iy) * 256.0).floor() / 255.0);
            for (dy, wy) in [(0.0, 1.0 - fy), (1.0, fy)] {
                for (dx, wx) in [(0.0, 1.0 - fx), (1.0, fx)] {
                    let (x, y) = (ix + dx, iy + dy);
                    if x < 0.0 || y < 0.0 || x >= *mw as f64 || y >= *mh as f64 {
                        continue;
                    }
                    let o = (y as usize * mw + x as usize) * n;
                    if let Some(p) = m.get(o..o + n) {
                        for (s, v) in acc.iter_mut().zip(p) {
                            *s += f64::from(*v) * wx * wy;
                        }
                    }
                }
            }
        };
        // A premultiplied sample of triangle `k`'s source at (u, v): a mip level's tap, one point,
        // or the average of its subsamples where the warp shrinks the source.
        let sample = |win: &Win, q: [f64; 2], k: u32, acc: &mut [f64; 8], one: &mut [f64; 8]| {
            let (j, nx, ny) = jac[k as usize];
            let l = lev.get(k as usize).copied().unwrap_or(0);
            if l > 0 {
                mip_tap(win, l, q, acc);
                return;
            }
            if nx * ny == 1 {
                point(win, interp, q[0] - 0.5, q[1] - 0.5, acc);
                return;
            }
            acc[..n].fill(0.0);
            for sy in 0..ny {
                let t = (sy as f64 + 0.5) / ny as f64 - 0.5;
                for sx in 0..nx {
                    let s = (sx as f64 + 0.5) / nx as f64 - 0.5;
                    point(win, Interp::Bilinear, q[0] + s * j[0] + t * j[2] - 0.5, q[1] + s * j[1] + t * j[3] - 0.5, one);
                    for (c, v) in acc.iter_mut().zip(one.iter()).take(n) {
                        *c += v;
                    }
                }
            }
            let k = 1.0 / (nx * ny) as f64;
            for c in acc.iter_mut().take(n) {
                *c *= k;
            }
        };
        let mut layer = [0.0f64; 8];
        for (o, (q, &k)) in uv.iter().zip(&owner).enumerate() {
            if q[0].is_nan() {
                continue;
            }
            match top_win(*q) {
                Some(tw) => sample(tw, *q, k, &mut acc, &mut one),
                None => acc[..n].fill(0.0),
            }
            // A fold: its layers under this one, bottom first, composite under the top (unless
            // the top covers them).
            let first = under_at.get(o).copied().unwrap_or(u32::MAX);
            if first != u32::MAX && !under.is_empty() && acc[a] < 1.0 {
                let mut under_top = [0.0f64; 8];
                let mut at = first;
                while let Some(&(q, k, next)) = under.get(at as usize) {
                    at = next;
                    match below_win(q) {
                        Some(bw) => sample(bw, q, k, &mut layer, &mut one),
                        None => layer[..n].fill(0.0),
                    }
                    let keep = 1.0 - layer[a].clamp(0.0, 1.0);
                    for c in 0..n {
                        under_top[c] = layer[c] + under_top[c] * keep;
                    }
                }
                let keep = 1.0 - acc[a].clamp(0.0, 1.0);
                for c in 0..n {
                    acc[c] += under_top[c] * keep;
                }
            }
            // A fringe pixel's alpha is capped by its distance out.
            cap_alpha(&mut acc[..n], cover.get(o).copied());
            let al = acc[a].clamp(0.0, 1.0);
            // Float noise (a vertex a hair off the pixel grid) must not leave invisible
            // colour behind in transparent pixels.
            if al <= 1e-6 {
                continue;
            }
            any = true;
            let b = o * n;
            for c in 0..a {
                // The unclamped alpha, so a bicubic overshoot past 1 doesn't lighten the colour (#6203).
                outp[b + c] = (acc[c] / acc[a]).clamp(0.0, 1.0) as f32;
            }
            outp[b + a] = al as f32;
        }
        if !any {
            return None;
        }
        // Encoded (and dropped if fully transparent) here, in parallel, rather than one tile at
        // a time once all are done. `t` lies in one tile.
        let mut one = Surface::new(fmt);
        one.write_region(*t, &outp);
        one.prune();
        if one.tile_count() == 0 {
            return None;
        }
        one.take_tiles(*t).pop()
    };
    #[cfg(not(target_arch = "wasm32"))]
    let done: Vec<(TileCoord, Arc<Tile>)> = {
        use rayon::prelude::*;
        tiles.par_iter().zip(bins.par_iter()).filter_map(work).collect()
    };
    #[cfg(target_arch = "wasm32")]
    let done: Vec<(TileCoord, Arc<Tile>)> = tiles.iter().zip(bins.iter()).filter_map(work).collect();
    if ctl.cancelled() {
        return out;
    }
    out.put_tiles(done);
    out
}

/// [`warp_mesh_surface`] for single-channel surfaces whose outside reads as their default value
/// (layer masks, selections): the content is warped with alpha and flattened back onto the
/// default.
pub fn warp_mesh_gray(s: &Surface, f: &(dyn Fn(f64, f64) -> (f64, f64) + Sync), interp: Interp) -> Surface {
    warp_mesh_gray_with(s, f, Fold::Rows, interp)
}

/// [`warp_mesh_gray`] with folds stacked as `fold` says: a layer's mask and its selection follow
/// the layer's pixels, the part on top of a fold included.
pub fn warp_mesh_gray_with(s: &Surface, f: &(dyn Fn(f64, f64) -> (f64, f64) + Sync), fold: Fold, interp: Interp) -> Surface {
    let default = s.default_pixel().first().copied().unwrap_or(0.0);
    let fmt = s.format();
    let src = s.content_bounds();
    let mut out = Surface::with_default(fmt, &[default]);
    if src.is_empty() {
        return out;
    }
    let with_alpha = PixelFormat::new(fmt.mode, fmt.sample, true);
    let mut tmp = Surface::new(with_alpha);
    let v = s.read_region(src);
    tmp.write_region(src, &v.iter().flat_map(|g| [*g, 1.0]).collect::<Vec<f32>>());
    let w = mesh(&tmp, src, f, fold, interp, &Interrupt::NONE);
    // Clear the old region and write the warped one separately, as `warp_gray` does: one dense
    // `src ∪ warped` buffer grows with the square of how far a mesh moves the content (#995).
    // `Surface` is tile-sparse, so a distant region costs only its own tiles.
    out.write_region(src, &vec![default; src.width() as usize * src.height() as usize]);
    let b = w.content_bounds();
    if !b.is_empty() {
        let px = w.read_region(b);
        let flat: Vec<f32> = px.as_chunks::<2>().0.iter().map(|p| p[0] * p[1] + default * (1.0 - p[1])).collect();
        out.write_region(b, &flat);
    }
    out.prune();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_geom::warp::{BezierMesh, Warp, WarpStyle};

    fn sample(s: SampleType) -> Surface {
        let mut surf = Surface::new(PixelFormat::new(ColorMode::Rgb, s, true));
        for y in 0..24 {
            for x in 0..40 {
                let v = ((x * 7 + y * 13) % 17) as f32 / 16.0;
                surf.write_pixel(x + 5, y + 3, &[v, 1.0 - v, (x as f32) / 40.0, 1.0]);
            }
        }
        surf
    }

    #[test]
    fn a_mesh_flung_across_the_coordinate_range_gets_no_per_tile_bookkeeping() {
        // About 8000 × 8000 destination tiles from three vertices.
        let src = sample(SampleType::U8);
        let far = 1_000_000.0;
        let verts = [([-far, -far], [0.0, 0.0]), ([far, -far], [40.0, 0.0]), ([0.0, far], [0.0, 24.0])];
        let t = std::time::Instant::now();
        let out = warp_triangles(&src, Rect::new(0, 0, 40, 24), &verts, &[[0, 1, 2]], Interp::Nearest);
        assert!(out.content_bounds().is_empty());
        assert!(t.elapsed().as_secs() < 5);
    }

    #[test]
    fn a_thin_layer_stretched_into_a_curve_keeps_the_curve() {
        // #5793: a 3 px wide column stretched 100x and bent into an arch. The grid was sized in
        // source pixels (one cell across), so the arch came out as a straight band.
        let mut src = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
        src.fill_rect(Rect::new(0, 0, 3, 40), &[1.0, 0.0, 0.0, 1.0]);
        let arch = |x: f64, y: f64| {
            let dx = x * 100.0;
            let t = (dx - 150.0) / 150.0;
            (dx, y + 40.0 * (1.0 - t * t))
        };
        let out = warp_mesh_surface(&src, Rect::new(0, 0, 3, 40), &arch, Interp::Bilinear);
        let alpha = |x: i32, y: i32| {
            let mut p = [0.0f32; 4];
            out.read_pixel(x, y, &mut p);
            p[3]
        };
        // The band follows the arch: 40 px lower in the middle, about 36 px a third of the way in.
        assert!(alpha(150, 60) > 0.99 && alpha(150, 20) < 0.01, "mid {} {}", alpha(150, 60), alpha(150, 20));
        assert!(alpha(100, 55) > 0.99 && alpha(100, 20) < 0.01, "third {} {}", alpha(100, 55), alpha(100, 20));
    }

    #[test]
    fn the_interruptible_mesh_warp_reports_progress_and_stops_when_cancelled() {
        let mut s = Surface::new(PixelFormat::RGBA8);
        s.fill_rect(Rect::new(0, 0, 600, 400), &[0.2, 0.4, 0.6, 1.0]);
        let r = s.content_bounds();
        let w = Warp::preset(WarpStyle::Arc, 30.0, [0.0, 0.0, 600.0, 400.0]);
        let map = |x: f64, y: f64| w.map(x, y);
        let reported = std::sync::Mutex::new(0.0f32);
        let progress = |p: f32| {
            let mut m = reported.lock().unwrap();
            *m = m.max(p);
        };
        let never = || false;
        let out = warp_mesh_surface_with(&s, r, &map, Fold::Rows, Interp::Bicubic, &Interrupt::new(&never, &progress)).expect("not cancelled");
        let want = warp_mesh_surface(&s, r, &map, Interp::Bicubic);
        assert_eq!(out.content_bounds(), want.content_bounds());
        assert_eq!(out.read_region(want.content_bounds()), want.read_region(want.content_bounds()));
        assert!(*reported.lock().unwrap() > 0.9);
        let always = || true;
        assert!(warp_mesh_surface_with(&s, r, &map, Fold::Rows, Interp::Bicubic, &Interrupt::cancel_only(&always)).is_none());
    }

    #[test]
    fn a_mesh_warp_far_off_canvas_writes_only_the_two_regions() {
        // #995: the old and warped regions were cleared as one dense `src ∪ warped` buffer, about
        // 37 GiB of floats for a 48 px selection moved 100000 px. Same fix as `warp_gray` (b70fbd4).
        for sample in [SampleType::U8, SampleType::F32] {
            let mut sel = Surface::new(PixelFormat::new(ColorMode::Grayscale, sample, false));
            sel.fill_rect(Rect::new(8, 8, 56, 56), &[1.0]);
            for d in [100_000.0, -100_000.0, 300.0] {
                let out = warp_mesh_gray(&sel, &|x, y| (x + d, y + d), Interp::Bilinear);
                let k = d as i32;
                assert_eq!(out.content_bounds(), Rect::new(8 + k, 8 + k, 56 + k, 56 + k), "{sample:?} {d}");
                assert_eq!(out.sample_channel(30 + k, 30 + k, 0), 1.0, "{sample:?} {d}: moved");
                assert_eq!(out.sample_channel(30, 30, 0), 0.0, "{sample:?} {d}: old region cleared");
            }
        }
    }

    #[test]
    fn identity_warp_is_a_no_op_at_every_depth() {
        for s in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let src = sample(s);
            let r = src.content_bounds();
            let b = [f64::from(r.x0), f64::from(r.y0), f64::from(r.x1), f64::from(r.y1)];
            for w in [Warp::none(b), Warp::custom(BezierMesh::identity(b, 1, 1), b), Warp::preset(WarpStyle::Arc, 0.0, b)] {
                for interp in [Interp::Nearest, Interp::Bilinear, Interp::Bicubic] {
                    let out = warp_mesh_surface(&src, r, &|x, y| w.map(x, y), interp);
                    assert_eq!(out.content_bounds(), r, "{s:?} {interp:?}");
                    let (a, o) = (src.read_region(r), out.read_region(r));
                    let worst = a.iter().zip(&o).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
                    assert!(worst <= 1.0 / 255.0, "{s:?} {interp:?}: {worst}");
                }
            }
        }
    }

    #[test]
    fn a_rotated_soft_edged_sprite_keeps_its_colour() {
        // #6203, as in Free Transform: the bicubic alpha overshoots past 1 next to the half
        // transparent edge, and must not lighten the colour.
        let mut src = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
        src.fill_rect(Rect::new(10, 10, 30, 30), &[0.8, 0.6, 0.4, 0.5]);
        src.fill_rect(Rect::new(11, 11, 29, 29), &[0.8, 0.6, 0.4, 1.0]);
        let k = std::f64::consts::FRAC_1_SQRT_2;
        let rotate = |x: f64, y: f64| (20.0 + (x - 20.0) * k - (y - 20.0) * k, 20.0 + (x - 20.0) * k + (y - 20.0) * k);
        let out = warp_mesh_surface(&src, src.content_bounds(), &rotate, Interp::Bicubic);
        let b = out.content_bounds();
        for y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                let p = out.pixel(x, y);
                if p[3] > 0.0 {
                    let off = (0..3).map(|i| (p[i] - [0.8, 0.6, 0.4][i]).abs()).fold(0.0f32, f32::max);
                    assert!(off <= 1.0 / 255.0, "({x}, {y}): {p:?}");
                }
            }
        }
    }

    #[test]
    fn translation_and_known_arc_points() {
        let src = sample(SampleType::U8);
        let r = src.content_bounds();
        let out = warp_mesh_surface(&src, r, &|x, y| (x + 10.0, y - 2.0), Interp::Bilinear);
        assert_eq!(out.content_bounds(), r.translate(10, -2));
        assert_eq!(out.pixel(20, 10), src.pixel(10, 12));
        // Arc 50 % (Photoshop): the bottom corners keep their pixels, the top rises and spreads.
        let b = [f64::from(r.x0), f64::from(r.y0), f64::from(r.x1), f64::from(r.y1)];
        let w = Warp::preset(WarpStyle::Arc, 50.0, b);
        let arc = warp_mesh_surface(&src, r, &|x, y| w.map(x, y), Interp::Bilinear);
        let corner = w.map(f64::from(r.x0), f64::from(r.y1));
        assert!((corner.0 - f64::from(r.x0)).abs() < 1e-9 && (corner.1 - f64::from(r.y1)).abs() < 1e-9, "{corner:?}");
        let (ex, ey) = w.map(f64::from(r.x0) + 2.5, f64::from(r.y0) + 2.5);
        let e = arc.pixel(ex.floor() as i32, ey.floor() as i32);
        assert!(e[3] > 0.2, "a point near the top-left corner lands at its mapped position: {e:?}");
        let c = arc.content_bounds();
        assert!(c.y0 < r.y0 - 2 && c.x0 < r.x0 && c.x1 > r.x1, "the top rises and spreads past the box: {c:?}");
    }

    #[test]
    fn a_warp_samples_once_unless_placed_small() {
        assert_eq!(Sampling::for_scale(1.0), Sampling::Single);
        assert_eq!(Sampling::for_scale(0.5), Sampling::Single);
        assert_eq!(Sampling::for_scale(0.49), Sampling::Footprint);
        assert_eq!(Sampling::for_scale(f64::NAN), Sampling::Footprint);
    }

    #[test]
    fn the_mip_level_follows_the_step_along_a_row() {
        assert_eq!(mip_level([1.0, 0.0]), 0);
        assert_eq!(mip_level([1.99, 0.0]), 0);
        assert_eq!(mip_level([2.0, 0.0]), 1);
        // A flag's left edge at 100 %: one pixel right is 1 px across and 5.7 px down the source.
        assert_eq!(mip_level([1.0, 5.72]), 2);
        assert_eq!(mip_level([0.0, -9.0]), 3);
        assert_eq!(mip_level([f64::NAN, 0.0]), 0);
        assert_eq!(mip_level([f64::INFINITY, 0.0]), 0);
        assert_eq!(mip_level([1e300, 0.0]), MAX_MIP_LEVEL);
    }

    #[test]
    fn mip_levels_average_the_texels_inside_the_source() {
        // A 3 x 3 window of one channel: level 1 is 2 x 2, its far blocks part outside, averaging
        // only what is inside.
        let px: Vec<f32> = (1..=9).map(|v| v as f32).collect();
        let m = mip_levels(&px, 3, 3, 1, 2, None);
        assert_eq!((m[0].1, m[0].2), (2, 2));
        assert_eq!(m[0].0, vec![(1.0 + 2.0 + 4.0 + 5.0) / 4.0, (3.0 + 6.0) / 2.0, (7.0 + 8.0) / 2.0, 9.0]);
        assert_eq!((m[1].1, m[1].2), (1, 1));
        assert!((m[1].0[0] - (3.0 + 4.5 + 7.5 + 9.0) / 4.0).abs() < 1e-6);
        assert!(mip_levels(&[], 0, 0, 4, 3, None).iter().all(|(p, _, _)| p.is_empty()));
        // In 8 bits: 7 / 255 and three zeros average to 1.75 / 255, kept as 2 / 255.
        let m = mip_levels(&[7.0 / 255.0, 0.0, 0.0, 0.0], 2, 2, 1, 1, Some(255.0));
        assert!((m[0].0[0] - 2.0 / 255.0).abs() < 1e-7);
    }

    /// The reference application's warp reads a box mip level where it squeezes the source along a
    /// destination row: 1 px stripes squeezed four times across come out their average grey. A
    /// squeeze down the columns keeps full resolution (one bilinear sample, so stripes stay).
    #[test]
    fn a_warp_squeezed_along_rows_samples_a_mip_level() {
        let stripes = |vertical: bool, w: i32, h: i32| {
            let mut src = Surface::new(PixelFormat::RGBA8);
            let px: Vec<f32> = (0..h)
                .flat_map(|y| (0..w).map(move |x| if (if vertical { x } else { y }) % 2 == 0 { 0.0 } else { 1.0 }))
                .flat_map(|v| [v, v, v, 1.0])
                .collect();
            src.write_region(Rect::new(0, 0, w, h), &px);
            src
        };
        let across = Warp::custom(photocraft_geom::warp::BezierMesh::identity([0.0, 0.0, 16.0, 64.0], 1, 1), [0.0, 0.0, 64.0, 64.0]);
        let out = place_source(&stripes(true, 64, 64), Rect::new(0, 0, 64, 64), &photocraft_geom::Affine::IDENTITY, Some(&across)).unwrap();
        let o = out.read_region(Rect::new(2, 2, 14, 62));
        let worst = o.as_chunks::<4>().0.iter().map(|p| (p[0] - 0.5).abs()).fold(0.0f32, f32::max);
        assert!(worst < 0.01, "squeezed across: worst deviation from grey {worst}");
        // Three times down: each pixel centre lands on a stripe's centre (4x would land between two).
        let down = Warp::custom(photocraft_geom::warp::BezierMesh::identity([0.0, 0.0, 64.0, 16.0], 1, 1), [0.0, 0.0, 64.0, 48.0]);
        let out = place_source(&stripes(false, 64, 48), Rect::new(0, 0, 64, 48), &photocraft_geom::Affine::IDENTITY, Some(&down)).unwrap();
        let o = out.read_region(Rect::new(2, 2, 62, 14));
        let worst = o.as_chunks::<4>().0.iter().map(|p| (p[0] - 0.5).abs()).fold(0.0f32, f32::max);
        assert!(worst > 0.2, "squeezed down: rows are point-sampled, not averaged ({worst})");
    }

    /// The mip levels are those of one pyramid over the whole source, whatever tiles the output and
    /// the source fall in: a flag (levels up to 2) drawn from a source moved by an odd offset is
    /// the same picture moved.
    #[test]
    fn mip_levels_follow_the_source_not_the_tile_grid() {
        let (w, h) = (300, 200);
        let mut seed = 0x2545_f491u32;
        let px: Vec<f32> = (0..w * h)
            .flat_map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                [(seed & 255) as f32 / 255.0, ((seed >> 8) & 255) as f32 / 255.0, ((seed >> 16) & 255) as f32 / 255.0, 1.0]
            })
            .collect();
        let draw = |dx: i32, dy: i32| {
            let mut src = Surface::new(PixelFormat::RGBA8);
            let r = Rect::new(dx, dy, dx + w, dy + h);
            src.write_region(r, &px);
            let warp = Warp::preset(WarpStyle::Flag, 100.0, [f64::from(dx), f64::from(dy), f64::from(dx + w), f64::from(dy + h)]);
            let out = place_source(&src, r, &photocraft_geom::Affine::IDENTITY, Some(&warp)).unwrap();
            out.read_region(Rect::new(dx - 20, dy - 120, dx + w + 20, dy + h + 120))
        };
        let (a, b) = (draw(0, 0), draw(37, 11));
        let diff: Vec<f32> = a.iter().zip(&b).map(|(x, y)| (x - y).abs()).collect();
        let off = diff.iter().filter(|d| **d > 1e-3).count();
        let worst = diff.iter().fold(0.0f32, |m, d| m.max(*d));
        assert!(a.iter().skip(3).step_by(4).filter(|v| **v > 0.5).count() > 40_000, "the flag is drawn");
        // Float noise from the moved vertices tips a rare weight (well under 1 % of values) by one
        // 8-bit step; a pyramid on another grid would average other pixels (tenths off).
        assert!(off * 100 < diff.len() && worst < 1.5 / 255.0, "{off} values differ, worst {worst}");
    }

    /// A warp moved 0.3 px right ends 0.2 px short of the next pixel's centre: that pixel is drawn
    /// as fringe, faint (alpha about 0.3), as the reference application draws warps; the pixel
    /// after it (0.7 px out) is not.
    #[test]
    fn a_warp_draws_a_fringe_past_its_border() {
        let mut src = Surface::new(PixelFormat::RGBA8);
        src.fill_rect(Rect::new(0, 0, 16, 16), &[1.0, 1.0, 1.0, 1.0]);
        let f = |x: f64, y: f64| (x + 0.3, y);
        let alpha_at = |facets: Facets, x: i32| {
            let out = warp_patches_surface(&src, Rect::new(0, 0, 16, 16), &f, [&[], &[]], Fold::IN_PLACE, Sampling::Single, facets);
            out.read_region(Rect::new(x, 8, x + 1, 9))[3]
        };
        let fringe = alpha_at(Facets::Envelope, 16);
        assert!((0.2..=0.31).contains(&fringe), "envelope fringe alpha {fringe}");
        let mesh = alpha_at(Facets::Mesh, 16);
        assert!((0.2..=0.31).contains(&mesh), "mesh fringe alpha {mesh}");
        assert_eq!(alpha_at(Facets::Envelope, 17), 0.0);
        assert_eq!(alpha_at(Facets::Envelope, 8), 1.0);
    }

    #[test]
    fn a_shrinking_mesh_warp_is_prefiltered() {
        // A warped smart object placed at ~1/5 size: a grid of 1 px lines every 4 px must come
        // out its area average (9/16 white) everywhere, not point-sampled moiré (crinkled,
        // doubled or missing lines), like the affine path's `large_downscale_is_prefiltered`.
        for s in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let (w, h) = (400, 320);
            let mut src = Surface::new(PixelFormat::new(ColorMode::Rgb, s, true));
            let px: Vec<f32> =
                (0..h).flat_map(|y| (0..w).map(move |x| if x % 4 == 0 || y % 4 == 0 { 0.0 } else { 1.0 })).flat_map(|v| [v, v, v, 1.0]).collect();
            let r = Rect::new(0, 0, w, h);
            src.write_region(r, &px);
            let warp = Warp::preset(WarpStyle::Arc, 15.0, [0.0, 0.0, f64::from(w), f64::from(h)]);
            let t = photocraft_geom::Affine { m: [0.2, 0.0, 0.0, 0.2, 7.0, 5.0] };
            let out = place_source(&src, r, &t, Some(&warp)).unwrap();
            let b = out.content_bounds();
            let o = out.read_region(b);
            let opaque: Vec<f32> = o.as_chunks::<4>().0.iter().filter(|p| p[3] >= 0.999).map(|p| p[0]).collect();
            assert!(opaque.len() > 2000, "{s:?}: {} opaque pixels", opaque.len());
            let worst = opaque.iter().map(|v| (v - 0.5625).abs()).fold(0.0f32, f32::max);
            // An area average over the ~5 px footprint still covers one or two lines per axis
            // (up to 0.2 off); point sampling lands wholly on or off a line (up to 0.56 off).
            assert!(worst < 0.25, "{s:?}: worst deviation from the area average {worst}");
        }
    }

    /// A warped smart object samples bilinearly: between two source pixels it ramps straight from
    /// one to the other, where bicubic overshoots next to a change (0.957 here, for 0.875).
    #[test]
    fn a_warped_smart_object_samples_bilinearly() {
        let mut src = Surface::new(PixelFormat::RGBA8);
        for x in 0..4 {
            let v = if x % 2 == 1 { 1.0 } else { 0.0 };
            src.fill_rect(Rect::new(x, 0, x + 1, 1), &[v, v, v, 1.0]);
        }
        // The 4 x 1 source (black, white, black, white) scaled four times by a warp: an identity
        // mesh over a box four times as big.
        let warp = Warp::custom(photocraft_geom::warp::BezierMesh::identity([0.0, 0.0, 16.0, 4.0], 1, 1), [0.0, 0.0, 4.0, 1.0]);
        let out = place_source(&src, Rect::new(0, 0, 4, 1), &photocraft_geom::Affine::IDENTITY, Some(&warp)).unwrap();
        // Destination pixel 6 (centre 6.5) is source x = 1.625: an eighth of the way from the white
        // pixel 1 to the black pixel 2.
        let v = out.read_region(Rect::new(6, 1, 7, 2))[0];
        assert!((v - 0.875).abs() < 0.01, "a straight ramp: {v}");
    }

    /// A 1-patch custom warp of the box `[0, 0, s, s]` with control point `k` (row-major, 4 x 4)
    /// moved by `d`, as `tessellate` takes it (normalised in, document px out).
    fn pulled(s: f64, k: usize, d: [f64; 2]) -> impl Fn(f64, f64) -> [f64; 2] {
        let mut mesh = photocraft_geom::warp::BezierMesh::identity([0.0, 0.0, s, s], 1, 1);
        mesh.points[k] = [mesh.points[k][0] + d[0], mesh.points[k][1] + d[1]];
        let w = Warp::custom(mesh, [0.0, 0.0, s, s]);
        move |u, v| {
            let (x, y) = w.map(u * s, v * s);
            [x, y]
        }
    }

    /// The largest distance, over a 9 x 9 grid of points in each cell, between the warped surface
    /// and the cell's two flat triangles.
    fn facet_error(at: &dyn Fn(f64, f64) -> [f64; 2], leaves: &[Cell]) -> f64 {
        let mut worst = 0.0f64;
        for &[u0, u1, v0, v1] in leaves {
            let (p00, p10, p01, p11) = (at(u0, v0), at(u1, v0), at(u0, v1), at(u1, v1));
            for i in 0..=8 {
                for j in 0..=8 {
                    let (fu, fv) = (f64::from(i) / 8.0, f64::from(j) / 8.0);
                    let lin = |k: usize| {
                        if fu >= fv {
                            p00[k] + (p10[k] - p00[k]) * fu + (p11[k] - p10[k]) * fv
                        } else {
                            p00[k] + (p01[k] - p00[k]) * fv + (p11[k] - p01[k]) * fu
                        }
                    };
                    let e = at(u0 + (u1 - u0) * fu, v0 + (v1 - v0) * fv);
                    worst = worst.max((e[0] - lin(0)).hypot(e[1] - lin(1)));
                }
            }
        }
        worst
    }

    /// An affine warp is flat everywhere: it keeps the 4 x 4 cells a patch starts as.
    #[test]
    fn an_affine_warp_keeps_its_starting_cells() {
        let at = |u: f64, v: f64| [10.0 + 300.0 * u + 40.0 * v, 5.0 + 20.0 * u + 200.0 * v];
        let leaves = tessellate(&at, &[], &[], 256.0, 256.0, Facets::Envelope);
        assert_eq!(leaves.len(), 16);
        assert!(leaves.iter().all(|c| (c[1] - c[0] - 0.25).abs() < 1e-12 && (c[3] - c[2] - 0.25).abs() < 1e-12));
    }

    /// Cells are small where a warp bends hard and large where it is flat, as the reference
    /// application cuts them: a corner pulled out gets the smallest cell, the far corner keeps a
    /// starting cell, and every cell is within the tolerance of the surface.
    #[test]
    fn cells_are_small_where_a_warp_bends_and_large_where_it_is_flat() {
        let at = pulled(512.0, 0, [-192.0, -192.0]);
        let leaves = tessellate(&at, &[], &[], 512.0, 512.0, Facets::Envelope);
        let width = |c: &Cell| c[1] - c[0];
        let corner = leaves.iter().find(|c| c[0] == 0.0 && c[2] == 0.0).unwrap();
        let far = leaves.iter().find(|c| c[1] == 1.0 && c[3] == 1.0).unwrap();
        let smallest = leaves.iter().map(width).fold(f64::MAX, f64::min);
        assert_eq!(width(corner), smallest, "{corner:?}");
        assert_eq!(width(far), 0.25, "{far:?}");
        assert!(width(corner) <= 1.0 / 32.0, "{corner:?}");
        let e = facet_error(&at, &leaves);
        assert!(e < 3.0 * FLAT_TOL, "facets stray {e} px");
    }

    /// An S-shaped edge crosses its chord at the midpoint, so a test of one whole patch sees it
    /// straight. Starting from 4 x 4 cells, it is still cut fine enough.
    #[test]
    fn an_s_shaped_edge_is_still_cut_finely() {
        let mut mesh = photocraft_geom::warp::BezierMesh::identity([0.0, 0.0, 256.0, 256.0], 1, 1);
        mesh.points[1][1] -= 60.0;
        mesh.points[2][1] += 60.0;
        let w = Warp::custom(mesh, [0.0, 0.0, 256.0, 256.0]);
        let at = |u: f64, v: f64| {
            let (x, y) = w.map(u * 256.0, v * 256.0);
            [x, y]
        };
        let leaves = tessellate(&at, &[], &[], 256.0, 256.0, Facets::Envelope);
        let e = facet_error(&at, &leaves);
        assert!(e < 3.0 * FLAT_TOL, "facets stray {e} px over {} cells", leaves.len());
    }

    /// Where a big cell meets two smaller ones, the point between the small ones lies on the big
    /// one's edge, so no crack opens: every pixel well inside a warped opaque picture is opaque.
    #[test]
    fn no_crack_opens_between_big_and_small_cells() {
        let mut src = Surface::new(PixelFormat::RGBA8);
        src.fill_rect(Rect::new(0, 0, 200, 200), &[1.0, 0.5, 0.2, 1.0]);
        let mut mesh = photocraft_geom::warp::BezierMesh::identity([0.0, 0.0, 200.0, 200.0], 1, 1);
        mesh.points[0] = [-90.0, -70.0];
        mesh.points[5] = [mesh.points[5][0] + 40.0, mesh.points[5][1] + 25.0];
        let warp = Warp::custom(mesh, [0.0, 0.0, 200.0, 200.0]);
        let out = place_source(&src, Rect::new(0, 0, 200, 200), &photocraft_geom::Affine::IDENTITY, Some(&warp)).unwrap();
        let mut holes = Vec::new();
        for j in 0..=200 {
            for i in 0..=200 {
                let (x, y) = warp.map(2.0 + 196.0 * f64::from(i) / 200.0, 2.0 + 196.0 * f64::from(j) / 200.0);
                let (px, py) = (x.floor() as i32, y.floor() as i32);
                let a = out.pixel(px, py)[3];
                if a < 0.999 {
                    holes.push((px, py, a));
                }
            }
        }
        assert!(holes.is_empty(), "{} see-through pixels, e.g. {:?}", holes.len(), &holes[..holes.len().min(5)]);
    }

    /// The probe the reference application was measured with: a 256 x 32 grey (128) strip with
    /// 1 px white lines at these columns, placed at (192, 32).
    fn line_strip() -> Surface {
        let mut s = Surface::new(PixelFormat::RGBA8);
        let g = 128.0 / 255.0;
        s.fill_rect(Rect::new(192, 32, 448, 64), &[g, g, g, 1.0]);
        for x in [8, 41, 73, 106, 138, 171, 203, 236] {
            s.fill_rect(Rect::new(192 + x, 32, 193 + x, 64), &[1.0, 1.0, 1.0, 1.0]);
        }
        s
    }

    /// The strip scaled horizontally by `k` about x = 320, as Free Transform does it: the row at
    /// y = 48 composited over black, in levels of 255.
    fn scaled_strip(k: f64) -> Vec<f64> {
        let t = photocraft_geom::Affine { m: [k, 0.0, 0.0, 1.0, 320.0 * (1.0 - k), 0.0] };
        let out = place_source(&line_strip(), Rect::new(192, 32, 448, 64), &t, None).unwrap();
        (0..640).map(|x| out.pixel(x, 48)).map(|p| if p.is_empty() { 0.0 } else { f64::from(p[0] * p[3]) * 255.0 }).collect()
    }

    /// A scaled smart object fills a box snapped to whole pixels, as in the reference
    /// application: a third of 256 px (85.33 px, from 277.33) covers exactly 277..362.
    #[test]
    fn a_scaled_smart_object_fills_whole_pixels() {
        let row = scaled_strip(1.0 / 3.0);
        assert_eq!(row[276], 0.0);
        assert!(row[277] > 100.0 && row[361] > 100.0, "{} {}", row[277], row[361]);
        assert_eq!(row[362], 0.0);
    }

    /// The snapped box rounds halves up on both sides of 0, as the reference application does
    /// (a move to 106.5 lands on 107): a box from -6 moved by half a pixel starts at -5, not -6.
    #[test]
    fn snapping_rounds_halves_up() {
        let r = Rect::new(-6, 5, 40, 38);
        assert_eq!(snapped_scale(r, [1.0, 1.0, 0.5, 0.25]), Some([1.0, 1.0, 1.0, 0.0]));
        assert_eq!(snapped_scale(r, [1.0, 1.0, 106.5, -7.25]).map(|t| t[2] - 6.0), Some(101.0));
        assert_eq!(snapped_scale(r, [-1.0, 1.0, 0.0, 0.0]), None, "a flip takes the general path");
        assert_eq!(snapped_scale(r, [f64::NAN, 1.0, 0.0, 0.0]), None);
        assert_eq!(snapped_scale(r, [1e300, 1.0, 0.0, 0.0]), None);
    }

    /// Shrinking resamples with Bicubic Sharper and enlarging with Bicubic Smoother (Image Size's
    /// Bicubic Automatic): pixel values the reference application gave for the same strip, line
    /// at column 41 and the left edge, at 50 % and 150 %. The bicubic warp was 4.75 levels off.
    #[test]
    fn scaling_a_smart_object_resamples_as_the_reference_does() {
        for (k, first, want) in
            [(0.5, 273, [128.0, 128.0, 114.25, 185.21, 156.19, 119.84, 128.0]), (1.5, 187, [121.32, 127.18, 201.72, 233.25, 158.09, 117.78, 127.15])]
        {
            let row = scaled_strip(k);
            for (i, w) in want.iter().enumerate() {
                let got = row[first + i];
                assert!((got - w).abs() < 1.0, "{k}: pixel {}: {got} vs {w}", first + i);
            }
        }
        // The edge pixel reads transparency past the contents, as the reference does.
        let edge = scaled_strip(0.5)[256];
        assert!((edge - 121.67).abs() < 1.0, "{edge}");
    }

    /// A warp that doesn't fold is drawn in row order whether or not folds stack: the result is
    /// the same to the bit, and only a tile where the picture overlaps itself is drawn again.
    #[test]
    fn a_warp_without_a_fold_is_the_same_either_way() {
        let (w, h) = (300, 200);
        let mut src = Surface::new(PixelFormat::RGBA8);
        let px: Vec<f32> = (0..w * h).flat_map(|i| [(i % 7) as f32 / 6.0, (i % 11) as f32 / 10.0, 0.5, 1.0]).collect();
        let r = Rect::new(0, 0, w, h);
        src.write_region(r, &px);
        let bulge = |x: f64, y: f64| {
            let (u, v) = (x / 300.0 - 0.5, y / 200.0 - 0.5);
            let k = 1.0 - 0.6 * (u * u + v * v);
            (300.0 * (0.5 + u * k) + 10.0, 200.0 * (0.5 + v * k) + 10.0)
        };
        let rows = warp_mesh_surface_with(&src, r, &bulge, Fold::Rows, Interp::Bicubic, &Interrupt::NONE).unwrap();
        let stacked = warp_mesh_surface_with(&src, r, &bulge, Fold::IN_PLACE, Interp::Bicubic, &Interrupt::NONE).unwrap();
        let b = rows.content_bounds();
        assert_eq!(b, stacked.content_bounds());
        assert!(rows.read_region(b) == stacked.read_region(b), "the same pixels");
    }

    /// Points in two parts of the source far apart are two regions, each read on its own; points
    /// close together are one, and a point not given has none.
    #[test]
    fn regions_split_distant_parts_of_the_source() {
        let src = Rect::new(0, 0, 6000, 4000);
        let pts: Vec<([f64; 2], [f64; 2])> =
            (0..64).flat_map(|i| [([100.0 + f64::from(i), 100.0], [0.5, 0.5]), ([5000.0 + f64::from(i), 3500.0], [0.5, 0.5])]).collect();
        let r = Regions::new(&|f| pts.iter().for_each(|&(q, e)| f(q, e)), src);
        assert_eq!(r.rects.len(), 2, "{:?}", r.rects);
        assert!(r.area() < 20_000, "small windows: {:?}", r.rects);
        assert_ne!(r.of([100.0, 100.0]), r.of([5000.0, 3500.0]));
        assert_eq!(r.of([120.0, 100.0]), r.of([100.0, 100.0]));
        assert_eq!(r.of([3000.0, 2000.0]), None, "nothing samples between them");
        // A compact patch stays one region; nothing at all is none.
        let near: Vec<([f64; 2], [f64; 2])> = (0..400).map(|i| ([f64::from(i % 20), f64::from(i / 20)], [0.5, 0.5])).collect();
        let one = Regions::new(&|f| near.iter().for_each(|&(q, e)| f(q, e)), src);
        assert_eq!(one.rects.len(), 1);
        assert_eq!(one.of([5.0, 5.0]), Some(0));
        let none = Regions::new(&|_| {}, src);
        assert!(none.rects.is_empty() && none.of([0.0, 0.0]).is_none());
        let bad = Regions::new(&|f| f([f64::NAN, 1.0], [0.5, 0.5]), src);
        assert!(bad.rects.is_empty());
    }

    #[test]
    fn gray_masks_keep_their_default() {
        let mut m = Surface::with_default(PixelFormat::GRAY8, &[1.0]);
        m.fill_rect(Rect::new(4, 4, 10, 10), &[0.0]);
        let o = warp_mesh_gray(&m, &|x, y| (x + 5.0, y), Interp::Bilinear);
        assert_eq!(o.pixel(0, 0), vec![1.0]);
        assert_eq!(o.pixel(12, 6), vec![0.0]);
        assert_eq!(o.pixel(5, 6), vec![1.0]);
    }

    /// A bend preset the reference stores as an envelope (Arc) is tessellated as one; Fisheye,
    /// Twist and the Shells, which it stores as full meshes, and any custom mesh, as a mesh.
    #[test]
    fn facets_follow_the_kind_of_mesh() {
        use photocraft_geom::warp::{BezierMesh, Warp, WarpStyle};
        let b = [96.0, 96.0, 224.0, 224.0];
        assert_eq!(facets_of(&Warp::preset(WarpStyle::Arc, 50.0, b)), Facets::Envelope);
        assert_eq!(facets_of(&Warp::preset(WarpStyle::Inflate, 50.0, b)), Facets::Envelope);
        assert_eq!(facets_of(&Warp::preset(WarpStyle::Fisheye, 50.0, b)), Facets::Mesh);
        assert_eq!(facets_of(&Warp::preset(WarpStyle::Twist, 50.0, b)), Facets::Mesh);
        assert_eq!(facets_of(&Warp::preset(WarpStyle::ShellLower, 50.0, b)), Facets::Mesh);
        let mut bulge = Vec::new();
        for j in 0..4 {
            for i in 0..4 {
                let inner = (1..3).contains(&i) && (1..3).contains(&j);
                let (x, y) = (96.0 + i as f64 * 128.0 / 3.0, 96.0 + j as f64 * 128.0 / 3.0);
                bulge.push(if inner { [x + if i == 1 { -12.0 } else { 12.0 }, y + if j == 1 { -12.0 } else { 12.0 }] } else { [x, y] });
            }
        }
        let fisheye = Warp::custom(BezierMesh { us: vec![0.0, 1.0], vs: vec![0.0, 1.0], points: bulge }, b);
        assert_eq!(facets_of(&fisheye), Facets::Mesh);
        // The same bulge splits more finely as a mesh than it would as an envelope: edges alone
        // decide, at the tighter tolerance.
        let m = fisheye.to_mesh(1, 1);
        let at = |u: f64, v: f64| m.eval(u, v);
        let as_mesh = tessellate(&at, &[], &[], 128.0, 128.0, Facets::Mesh).len();
        let as_env = tessellate(&at, &[], &[], 128.0, 128.0, Facets::Envelope).len();
        assert!(as_mesh != as_env, "{as_mesh} vs {as_env}");
    }

    /// Bilinear warp weights are the reference's: the fraction in 1/256 steps over 255, so a point
    /// halfway between two samples weighs the far one 128/255, not 1/2.
    #[test]
    fn warp_bilinear_weights_are_in_1_256_steps_over_255() {
        use photocraft_geom::warp::{BezierMesh, Warp};
        let mut src = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, true));
        // Two columns, 0 and 1, four rows: a half-pixel shift to the right samples the midpoint.
        src.write_region(Rect::new(0, 0, 2, 4), &[0.0, 1.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0]);
        let shift = BezierMesh {
            us: vec![0.0, 1.0],
            vs: vec![0.0, 1.0],
            points: (0..4).flat_map(|j| (0..4).map(move |i| [0.5 + 2.0 * i as f64 / 3.0, 4.0 * j as f64 / 3.0])).collect(),
        };
        let warp = Warp::custom(shift, [0.0, 0.0, 2.0, 4.0]);
        let out = place_source(&src, Rect::new(0, 0, 2, 4), &photocraft_geom::Affine::IDENTITY, Some(&warp)).expect("placed");
        let px = out.read_region(Rect::new(1, 1, 2, 2));
        let got = f64::from(px[0]) / f64::from(px[1]);
        assert!((got - 128.0 / 255.0).abs() < 1e-5, "weight {got}");
    }

    /// Edit › Transform › Warp folding the picture over itself (#8927): the layers stack as the
    /// preview draws them, the part dragged furthest on top, and composite, so a transparent part on top
    /// shows the content folded under it, also where the top part is a stretch of the source with
    /// no pixels at all (no tiles). Layers aren't composited twice, and other mesh maps keep row
    /// order.
    #[test]
    fn a_fold_shows_what_is_under_its_transparent_parts() {
        let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::U8, true);
        let ts = photocraft_geom::TILE_SIZE;
        for width in [100, 4 * ts] {
            let half = width / 2;
            // The red left half stays; the right half, transparent (no pixels), folds back over it.
            let mut src = Surface::new(fmt);
            src.fill_rect(Rect::new(0, 0, half, 10), &[1.0, 0.0, 0.0, 1.0]);
            let w = f64::from(width);
            let fold = move |x: f64, y: f64| (if x <= w / 2.0 { x } else { w - x }, y);
            let out = warp_mesh_surface_with(&src, Rect::new(0, 0, width, 10), &fold, Fold::IN_PLACE, Interp::Bilinear, &Interrupt::NONE).unwrap();
            let px = out.read_region(Rect::new(0, 5, half, 6));
            let alpha: Vec<f32> = px.as_chunks::<4>().0.iter().map(|p| p[3]).collect();
            let lit = alpha.iter().filter(|a| **a > 0.99).count();
            assert!(lit as i32 >= half - 4, "width {width}: the red under the transparent top shows ({lit} of {half} px)");
            // The generic mesh warp keeps row order: the last drawn (the right half, empty) wins.
            let plain = warp_mesh_surface(&src, Rect::new(0, 0, width, 10), &fold, Interp::Bilinear);
            assert!(plain.read_region(Rect::new(half / 2, 5, half / 2 + 1, 6))[3] < 0.01);
        }
        // Two half-transparent layers composite: 0.75.
        let mut half = Surface::new(fmt);
        half.fill_rect(Rect::new(0, 0, 100, 10), &[0.0, 0.0, 1.0, 0.5]);
        let fold = |x: f64, y: f64| (if x <= 50.0 { x } else { 100.0 - x }, y);
        let out = warp_mesh_surface_with(&half, Rect::new(0, 0, 100, 10), &fold, Fold::IN_PLACE, Interp::Bilinear, &Interrupt::NONE).unwrap();
        let a = out.read_region(Rect::new(20, 5, 21, 6))[3];
        assert!((a - 0.75).abs() < 0.02, "two half-transparent layers: alpha {a}");
        // No fold: every pixel of a half-transparent picture stays at 0.5 (no layer counted twice
        // along the cells' edges).
        let mut big = Surface::new(fmt);
        big.fill_rect(Rect::new(0, 0, 200, 200), &[0.0, 0.0, 1.0, 0.5]);
        let bend = |x: f64, y: f64| (x + 6.0 * (y / 30.0).sin(), y + 6.0 * (x / 40.0).cos());
        let out = warp_mesh_surface_with(&big, Rect::new(0, 0, 200, 200), &bend, Fold::IN_PLACE, Interp::Bilinear, &Interrupt::NONE).unwrap();
        let px = out.read_region(Rect::new(20, 20, 180, 180));
        let worst = px.as_chunks::<4>().0.iter().map(|p| (p[3] - 0.5).abs()).fold(0.0f32, f32::max);
        assert!(worst < 0.02, "a warp without a fold keeps the alpha: off by {worst}");
        // A tight fold, magnified: the layers are under a source pixel apart and still composite.
        let mut thin = Surface::new(fmt);
        thin.fill_rect(Rect::new(0, 0, 20, 2), &[0.0, 0.0, 1.0, 0.5]);
        let tight = |x: f64, y: f64| (50.0 * if x <= 10.0 { x } else { 20.0 - x }, y);
        let out = warp_mesh_surface_with(&thin, Rect::new(0, 0, 20, 2), &tight, Fold::IN_PLACE, Interp::Bilinear, &Interrupt::NONE).unwrap();
        // 20 px from the crease: 0.4 source px into each layer, 0.8 apart.
        let a = out.read_region(Rect::new(480, 1, 481, 2))[3];
        assert!((a - 0.75).abs() < 0.02, "layers 0.8 source px apart: alpha {a}");
    }

    /// A mask or selection warped with the layer follows its pixels: where the warp folds, the
    /// part on top is the part the layer shows on top.
    #[test]
    fn a_fold_stacks_a_mask_as_it_stacks_the_layer() {
        let fmt = PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false);
        let mut m = Surface::with_default(fmt, &[0.0]);
        m.fill_rect(Rect::new(0, 0, 100, 10), &[0.25]);
        m.fill_rect(Rect::new(50, 0, 100, 10), &[1.0]);
        // The left half folds over the right.
        let fold = |x: f64, y: f64| (if x >= 50.0 { x } else { 100.0 - x }, y);
        let top = warp_mesh_gray_with(&m, &fold, Fold::IN_PLACE, Interp::Bilinear).read_region(Rect::new(80, 5, 81, 6))[0];
        assert!((top - 0.25).abs() < 1e-3, "the left part, dragged over, is on top: {top}");
        let rows = warp_mesh_gray(&m, &fold, Interp::Bilinear).read_region(Rect::new(80, 5, 81, 6))[0];
        assert!((rows - 1.0).abs() < 1e-3, "other mesh maps: the last cell wins: {rows}");
    }

    /// A vertex with no finite movement (past a Distort's horizon) goes under, not on top.
    #[test]
    fn cells_moved_without_bound_go_first() {
        // Two cells in a row: the second's right vertices moved to infinity.
        let rest = [[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [0.0, 1.0], [1.0, 1.0], [2.0, 1.0]];
        let mut pos = rest.map(|[x, y]| [x + 1.0, y]);
        pos[2] = [f64::INFINITY, 0.0];
        pos[5] = [f64::NAN, 1.0];
        assert_eq!(cell_order(&pos, &rest, 3), vec![1, 0]);
    }

    /// A cell the warp turns over (mirrored, as a flap folded back) goes on top, even when it moved
    /// less than the cells around it; within each kind, the cell moved least goes first.
    #[test]
    fn a_turned_over_cell_goes_on_top() {
        let rest = [[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [3.0, 0.0], [0.0, 1.0], [1.0, 1.0], [2.0, 1.0], [3.0, 1.0]];
        // Cell 0 moved far and cell 1 a little, both facing as before; cell 2 is mirrored in x and
        // moved less than cell 0.
        let pos = [[-9.0, 0.0], [1.0, 0.0], [2.2, 0.0], [1.6, 0.0], [-9.0, 1.0], [1.0, 1.0], [2.2, 1.0], [1.6, 1.0]];
        assert_eq!(cell_order(&pos, &rest, 4), vec![1, 0, 2]);
        // A mirrored placement turns nothing over: only a change of facing counts.
        let mirrored = rest.map(|[x, y]| [-x, y]);
        assert_eq!(cell_order(&mirrored, &mirrored, 4), vec![0, 1, 2]);
    }

    /// The middle of the picture pulled out past its right edge: the edge strip folds back over it
    /// and lies on top, so the pulled middle is under that strip (by distance moved alone, the middle
    /// would cover it).
    #[test]
    fn the_middle_pulled_past_an_edge_goes_under_the_folded_edge() {
        let fmt = PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false);
        let mut m = Surface::with_default(fmt, &[0.0]);
        m.fill_rect(Rect::new(0, 0, 75, 10), &[0.25]);
        m.fill_rect(Rect::new(75, 0, 100, 10), &[1.0]);
        // 0..75 stretches to 0..90; the edge strip 75..100 folds back onto 90..80; beyond, unchanged.
        let pull = |x: f64, y: f64| {
            (
                if x <= 75.0 {
                    x * 1.2
                } else if x <= 100.0 {
                    90.0 - (x - 75.0) * 0.4
                } else {
                    x - 20.0
                },
                y,
            )
        };
        let at = warp_mesh_gray_with(&m, &pull, Fold::IN_PLACE, Interp::Bilinear).read_region(Rect::new(85, 5, 86, 6))[0];
        assert!((at - 1.0).abs() < 1e-3, "the folded-back edge strip is on top: {at}");
    }

    /// A warp split into two patches, bent by two bumps: each patch is cut on its own (a flat
    /// patch stays whole), and where a smaller cell's point lies inside a bigger cell's edge, the
    /// gap between the edge and the point is filled, so no pixel inside the picture is left
    /// transparent.
    #[test]
    fn a_split_warp_opens_no_crack() {
        use photocraft_geom::warp::BezierMesh;
        let bump = |u: f64, v: f64, cu: f64, cv: f64| (-((u - cu).powi(2) + (v - cv).powi(2)) / 0.03).exp();
        let lumpy = |u: f64, v: f64| [16.0 * bump(u, v, 0.3, 0.7) - 9.0 * bump(u, v, 0.75, 0.3), 12.0 * bump(u, v, 0.6, 0.6) - 14.0 * bump(u, v, 0.2, 0.25)];
        let line = [0.0, 1.0 / 6.0, 2.0 / 6.0, 0.5, 4.0 / 6.0, 5.0 / 6.0, 1.0];
        // Rows at thirds of the whole height (one patch down): v = 0, 1/3, 2/3, 1.
        let points: Vec<[f64; 2]> = (0..4)
            .flat_map(|j| {
                let v = f64::from(j) / 3.0;
                line.iter().map(move |&u| (u, v)).collect::<Vec<_>>()
            })
            .map(|(u, v)| {
                let d = lumpy(u, v);
                [128.0 * u + d[0], 128.0 * v + d[1]]
            })
            .collect();
        let mesh = BezierMesh { us: vec![0.0, 0.5, 1.0], vs: vec![0.0, 1.0], points };
        assert!(mesh.is_valid());
        let f = |x: f64, y: f64| {
            let p = mesh.eval(x / 128.0, y / 128.0);
            (p[0], p[1])
        };
        let mut src = Surface::new(PixelFormat::RGBA8);
        src.fill_rect(Rect::new(0, 0, 128, 128), &[1.0, 1.0, 1.0, 1.0]);
        let out = warp_patches_surface(&src, Rect::new(0, 0, 128, 128), &f, [&[0.5], &[]], Fold::IN_PLACE, Sampling::Single, Facets::Mesh);
        let a = out.read_region(Rect::new(-20, -20, 150, 150));
        let alpha = |x: i32, y: i32| if (0..170).contains(&x) && (0..170).contains(&y) { a[((y * 170 + x) * 4 + 3) as usize] } else { 0.0 };
        let mut holes = Vec::new();
        for y in 1..169 {
            for x in 1..169 {
                let shut = |dx: i32, dy: i32| alpha(x - dx, y - dy) > 0.999 && alpha(x + dx, y + dy) > 0.999;
                if alpha(x, y) < 0.99 && (shut(1, 0) || shut(0, 1) || shut(2, 0) || shut(0, 2)) {
                    holes.push((x - 20, y - 20));
                }
            }
        }
        assert!(holes.is_empty(), "transparent pixels inside the picture: {holes:?}");
    }

    /// Two patches side by side, the left one whole and the right one cut in half down: the point
    /// the right patch adds on the shared border joins the left patch's straight edge (and the
    /// border's ends are shared), so the two sides meet.
    #[test]
    fn a_quilt_border_point_joins_the_bigger_neighbour() {
        use crate::warp_tess::EnvelopeMesh;
        let uv = vec![
            [0.0, 0.0],
            [0.5, 0.0],
            [0.0, 1.0],
            [0.5, 1.0], // left patch
            [0.5, 0.0],
            [1.0, 0.0],
            [0.5, 0.5],
            [1.0, 0.5],
            [0.5, 1.0],
            [1.0, 1.0], // right patch
        ];
        let mut pos: Vec<[f64; 2]> = uv.iter().map(|p| [100.0 * p[0], 100.0 * p[1]]).collect();
        pos[6] = [53.0, 50.0]; // the right patch's border point, on its own (curved) surface
        let quads = vec![[0, 1, 2, 3], [4, 5, 6, 7], [6, 7, 8, 9]];
        let m = stitch_patches(EnvelopeMesh { uv, pos, quads }, &[0.0, 0.5, 1.0], &[0.0, 1.0]);
        assert_eq!(m.uv.len(), 8, "the border's shared ends are one vertex each");
        let k = m.uv.iter().position(|p| *p == [0.5, 0.5]).expect("border point");
        assert_eq!(m.pos[k], [50.0, 50.0]);
        assert_eq!(m.quads[1][0], m.quads[0][1], "the border's top end is shared");
    }
}
