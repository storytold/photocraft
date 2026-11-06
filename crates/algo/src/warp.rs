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
//! source pixel), so thin lines don't alias. Where the warp folds over itself the
//! later (lower-right) cell wins. Work is split per destination tile (parallel on native).

use photocraft_color::PixelFormat;
use photocraft_geom::{Rect, TileCoord};
use photocraft_raster::{Interrupt, Surface, Tile};
use std::sync::Arc;

use crate::transform::{Interp, bicubic};

/// Grid cell size (source px) used to approximate the map.
const CELL: f64 = 4.0;
/// Most cells along one axis (bounds memory and time on huge layers).
const MAX_CELLS: usize = 512;
/// Most subsamples per axis when a warp shrinks the source; also the longest source step per
/// destination pixel the filter follows (bounds the footprint each tile reads).
const MAX_SS: f64 = 8.0;

/// Places a smart object's source image (`src`, its pixels in `src_rect`) in the document:
/// through `warp` (source space) and then the affine `t` (source → document) in one resampling
/// pass, an exact shift for whole-pixel translations (so conversions and re-renders are
/// lossless), bicubic otherwise. A warp samples bilinearly, as the reference application draws a
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
        let (cu, cv) = patch_cuts(w, src_rect);
        return Ok(warp_patches_surface(src, src_rect, &map, &cu, &cv, Interp::Bilinear));
    }
    let near = |x: f64, y: f64| (x - y).abs() < 1e-9;
    if near(a, 1.0) && near(b, 0.0) && near(c, 0.0) && near(d, 1.0) && near(e, e.round()) && near(f, f.round()) {
        return Ok(crate::resample::translate_surface(src, e.round() as i32, f.round() as i32));
    }
    crate::transform::warp_surface(src, src_rect, &crate::transform::Homography([a, c, e, b, d, f, 0.0, 0.0, 1.0]), Interp::Bicubic)
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
        return Ok(warp_patches_surface(src, src_rect, &map, &cu, &cv, Interp::Bilinear));
    }
    crate::transform::warp_surface(src, src_rect, h, Interp::Bicubic)
}

/// Warps the content of `src` inside `src_rect` through the forward map `f` (source document
/// coordinates → destination document coordinates). Output has `src`'s format with alpha
/// added; pixels outside the warped area are transparent.
pub fn warp_mesh_surface(src: &Surface, src_rect: Rect, f: &(dyn Fn(f64, f64) -> (f64, f64) + Sync), interp: Interp) -> Surface {
    mesh(src, src_rect, f, interp, &Interrupt::NONE)
}

/// [`warp_mesh_surface`] reporting progress per destination tile; `None` once `ctl` is cancelled.
pub fn warp_mesh_surface_with(src: &Surface, src_rect: Rect, f: &(dyn Fn(f64, f64) -> (f64, f64) + Sync), interp: Interp, ctl: &Interrupt) -> Option<Surface> {
    let out = mesh(src, src_rect, f, interp, ctl);
    (!ctl.cancelled()).then_some(out)
}

fn mesh(src: &Surface, src_rect: Rect, f: &(dyn Fn(f64, f64) -> (f64, f64) + Sync), interp: Interp, ctl: &Interrupt) -> Surface {
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
    let mut tris = Vec::with_capacity(nx * ny * 2);
    for j in 0..ny {
        for i in 0..nx {
            let a = j * w1 + i;
            tris.push([a, a + 1, a + w1 + 1]);
            tris.push([a, a + w1 + 1, a + w1]);
        }
    }
    triangles(src, src_rect, &verts, &tris, interp, ctl)
}

/// How far (destination px) a cell's straight edges and middle may stray from the warped surface
/// before the cell is split, in [`warp_patches_surface`].
const FLAT_TOL: f64 = 0.5;
/// Most cells one [`warp_patches_surface`] tessellation makes; past it, cells stay as they are.
const MAX_LEAVES: usize = 1 << 18;
/// Cells narrower than this (source px) aren't split further.
const MIN_CELL: f64 = 0.5;
/// Cells each patch starts as, per axis: a curve that crosses its chord at the midpoint (a flag's
/// S-shaped edge, a twist) looks straight to a test of the midpoint alone.
const START_CELLS: usize = 4;

/// A cell of the source rectangle in normalised coordinates: `[u0, u1, v0, v1]`.
type Cell = [f64; 4];
/// Mesh vertices as `(destination, source)` and triangles of vertex indices.
type TriMesh = (Vec<([f64; 2], [f64; 2])>, Vec<[usize; 3]>);

/// Warps `src` through a smart object's warp, `f`, as the reference application draws one: the
/// surface is cut into cells, each drawn as two flat triangles (split along the diagonal from the
/// top-left corner): each patch starts as [`START_CELLS`] × [`START_CELLS`] cells, and a cell is
/// halved, along an axis or both, until its edges and its middle stray at most [`FLAT_TOL`] from
/// the surface. So cells are small where the warp bends hard and
/// large where it is flat, and the picture keeps the slight facets of that tessellation, which
/// cells of a few pixels everywhere ([`warp_mesh_surface`]) smoothed away. `cuts_u`/`cuts_v` are
/// the patch borders inside the rectangle (normalised, `0..1`): the cells start from them.
/// Where a cell meets two smaller ones, the point between them is put on the bigger cell's edge,
/// so no crack opens.
pub fn warp_patches_surface(src: &Surface, src_rect: Rect, f: &(dyn Fn(f64, f64) -> (f64, f64) + Sync), cuts_u: &[f64], cuts_v: &[f64], interp: Interp) -> Surface {
    if src_rect.is_empty() {
        let fmt = src.format();
        return Surface::new(PixelFormat::new(fmt.mode, fmt.sample, true));
    }
    let (x0, y0) = (f64::from(src_rect.x0), f64::from(src_rect.y0));
    let (sw, sh) = (f64::from(src_rect.width()), f64::from(src_rect.height()));
    let at = |u: f64, v: f64| {
        let (x, y) = f(x0 + sw * u, y0 + sh * v);
        [x, y]
    };
    let leaves = tessellate(&at, cuts_u, cuts_v, sw, sh);
    let (verts, tris) = cell_triangles(&at, &leaves, [x0, y0, sw, sh]);
    triangles(src, src_rect, &verts, &tris, interp, &Interrupt::NONE)
}

/// The sorted, deduplicated cuts of `0..1` from `inner` (values outside or non-finite dropped).
fn cut_list(inner: &[f64]) -> Vec<f64> {
    let mut c = vec![0.0, 1.0];
    c.extend(inner.iter().copied().filter(|t| t.is_finite() && *t > 0.0 && *t < 1.0));
    c.sort_by(f64::total_cmp);
    c.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    c
}

/// The leaf cells of the adaptive tessellation, in row order (top to bottom, then left to right).
fn tessellate(at: &dyn Fn(f64, f64) -> [f64; 2], cuts_u: &[f64], cuts_v: &[f64], sw: f64, sh: f64) -> Vec<Cell> {
    let gap = |p: [f64; 2], a: [f64; 2], b: [f64; 2]| {
        let d = (p[0] - (a[0] + b[0]) / 2.0).hypot(p[1] - (a[1] + b[1]) / 2.0);
        // A non-finite surface can't be judged: leave the cell as it is.
        if d.is_finite() { d } else { 0.0 }
    };
    let (us, vs) = (cut_list(cuts_u), cut_list(cuts_v));
    let mut queue: std::collections::VecDeque<Cell> = std::collections::VecDeque::new();
    let steps = |w: &[f64]| (0..START_CELLS).map(move |k| [w[0] + (w[1] - w[0]) * k as f64 / START_CELLS as f64, w[0] + (w[1] - w[0]) * (k + 1) as f64 / START_CELLS as f64]);
    for v in vs.windows(2).flat_map(steps) {
        for u in us.windows(2).flat_map(steps) {
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
        let ec = gap(at(um, vm), mid, mid);
        let room = leaves.len() + queue.len() + 3 < MAX_LEAVES;
        let split_u = room && (eu > FLAT_TOL || ec > FLAT_TOL) && (u1 - u0) * sw > MIN_CELL;
        let split_v = room && (ev > FLAT_TOL || ec > FLAT_TOL) && (v1 - v0) * sh > MIN_CELL;
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
fn cell_triangles(at: &dyn Fn(f64, f64) -> [f64; 2], leaves: &[Cell], rect: [f64; 4]) -> TriMesh {
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
    let verts = uv.iter().zip(&pos).map(|(p, &d)| (d, [x0 + sw * p[0], y0 + sh * p[1]])).collect();
    let tris = corners.iter().flat_map(|&[c00, c10, c01, c11]| [[c00, c10, c11], [c00, c11, c01]]).collect();
    (verts, tris)
}

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
    triangles(src, src_rect, verts, tris, interp, &Interrupt::NONE)
}

/// [`warp_triangles`], reporting progress per destination tile to `ctl`. A cancelled `ctl` skips
/// the remaining tiles; the caller discards the result.
/// Destination tiles one triangle render may keep bookkeeping for (a 1M-tile render is 69 Gpx).
const MAX_DST_TILES: u64 = 1 << 20;
/// Source pixels one destination tile may read densely.
const MAX_FOOTPRINT_PIXELS: u64 = 1 << 25;

fn triangles(src: &Surface, src_rect: Rect, verts: &[([f64; 2], [f64; 2])], tris: &[[usize; 3]], interp: Interp, ctl: &Interrupt) -> Surface {
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
    let mut bins: Vec<Vec<[usize; 3]>> = vec![Vec::new(); tiles.len()];
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
    for &tri in tris {
        let ps = tri.map(|v| verts[v].0);
        let bx0 = ps.iter().map(|p| p[0]).fold(f64::MAX, f64::min).floor() as i32;
        let by0 = ps.iter().map(|p| p[1]).fold(f64::MAX, f64::min).floor() as i32;
        let bx1 = ps.iter().map(|p| p[0]).fold(f64::MIN, f64::max).ceil() as i32;
        let by1 = ps.iter().map(|p| p[1]).fold(f64::MIN, f64::max).ceil() as i32;
        let (cx0, cy0) = ((bx0.max(dst.x0) - tx0).div_euclid(ts), (by0.max(dst.y0) - ty0).div_euclid(ts));
        let (cx1, cy1) = ((bx1.min(dst.x1 - 1) - tx0).div_euclid(ts), (by1.min(dst.y1 - 1) - ty0).div_euclid(ts));
        for cy in cy0.max(0)..=cy1 {
            for cx in cx0.max(0)..=cx1 {
                let g = cy as usize * cols + cx as usize;
                if let Some(&k) = lookup.get(g)
                    && k != usize::MAX
                {
                    bins[k].push(tri);
                }
            }
        }
    }
    let n = fmt.channels();
    let a = n - 1;
    // Tiles finished so far, for progress reported per tile (from any worker thread).
    let finished = std::sync::atomic::AtomicUsize::new(0);
    let total = tiles.len().max(1);
    let work = |(t, tris): (&Rect, &Vec<[usize; 3]>)| -> Option<(TileCoord, Arc<Tile>)> {
        if ctl.cancelled() {
            return None;
        }
        let k = finished.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        // The last few percent are putting the tiles in place.
        ctl.progress(0.98 * k as f32 / total as f32);
        if tris.is_empty() {
            return None;
        }
        let w = t.width() as usize;
        let h = t.height() as usize;
        // Subsamples along one destination axis, from the source step `j` it makes: one where
        // the warp keeps or enlarges the source, else about one per source pixel the output pixel
        // spans (an area average, as the transform path gets from its pre-reduction; a single
        // bicubic sample aliases thin lines when shrinking). The subsamples are bilinear: the
        // box does the filtering, and it is ~4x cheaper than bicubic at no loss.
        let count = |j: [f64; 2]| if interp == Interp::Nearest { 1 } else { j[0].hypot(j[1]).round().clamp(1.0, MAX_SS) as usize };
        // Source position per destination pixel (NaN = not covered) and the triangle it came
        // from. Per triangle: its source step per destination pixel along x and y (`[dx_u, dx_v,
        // dy_u, dy_v]`, the inverse map's Jacobian, constant over a triangle; zero when
        // point-sampled) and the subsamples along each.
        let mut uv = vec![[f64::NAN; 2]; w * h];
        let mut owner = vec![0u32; w * h];
        let mut jac: Vec<([f64; 4], usize, usize)> = Vec::with_capacity(tris.len());
        for (k, tri) in tris.iter().enumerate() {
            let [(p0, s0), (p1, s1), (p2, s2)] = tri.map(|v| verts[v]);
            let det = (p1[0] - p0[0]) * (p2[1] - p0[1]) - (p2[0] - p0[0]) * (p1[1] - p0[1]);
            if det.abs() < 1e-12 {
                jac.push(([0.0; 4], 1, 1));
                continue;
            }
            let (d1, d2) = ([s1[0] - s0[0], s1[1] - s0[1]], [s2[0] - s0[0], s2[1] - s0[1]]);
            let (l1x, l1y, l2x, l2y) = ((p2[1] - p0[1]) / det, (p0[0] - p2[0]) / det, (p0[1] - p1[1]) / det, (p1[0] - p0[0]) / det);
            // Capped, so a sliver triangle can't ask for an unbounded footprint.
            let cap = |j: [f64; 2]| {
                let l = j[0].hypot(j[1]);
                if l > MAX_SS { [j[0] * MAX_SS / l, j[1] * MAX_SS / l] } else { j }
            };
            let jx = cap([l1x * d1[0] + l2x * d2[0], l1x * d1[1] + l2x * d2[1]]);
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
                    uv[o] = [l0 * s0[0] + l1 * s1[0] + l2 * s2[0], l0 * s0[1] + l1 * s1[1] + l2 * s2[1]];
                    owner[o] = k as u32;
                }
            }
        }
        // Source footprint of the covered pixels, including the spread of their subsamples.
        let (mut fx0, mut fy0, mut fx1, mut fy1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for (q, j) in uv.iter().zip(&owner).filter(|(q, _)| !q[0].is_nan()).map(|(q, &k)| (q, jac[k as usize].0)) {
            let (ex, ey) = (0.5 * (j[0].abs() + j[2].abs()), 0.5 * (j[1].abs() + j[3].abs()));
            fx0 = fx0.min(q[0] - ex);
            fy0 = fy0.min(q[1] - ey);
            fx1 = fx1.max(q[0] + ex);
            fy1 = fy1.max(q[1] + ey);
        }
        if fx0 > fx1 {
            return None;
        }
        let foot = Rect::new(fx0.floor() as i32 - 3, fy0.floor() as i32 - 3, fx1.ceil() as i32 + 3, fy1.ceil() as i32 + 3).intersect(&src_rect);
        // Sparse components far apart in the source can land in one tile: the dense window
        // between them is not read (the tile stays empty). The pixel-area cap is checked first:
        // `has_tiles_in` walks the source tile grid, so an oversized footprint must return
        // without paying for that scan.
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
        let at = |x: i32, y: i32, c: usize| -> f64 {
            if x < foot.x0 || y < foot.y0 || x >= foot.x1 || y >= foot.y1 {
                0.0
            } else {
                f64::from(px[((y - foot.y0) as usize * fw + (x - foot.x0) as usize) * n + c])
            }
        };
        // Offset of pixel (x, y) when the `taps` x `taps` block from it lies inside `foot`, so
        // the samplers can index `px` directly instead of bounds-checking every tap.
        let base = |x: i32, y: i32, taps: i32| -> Option<usize> {
            (x >= foot.x0 && y >= foot.y0 && x + taps <= foot.x1 && y + taps <= foot.y1).then(|| ((y - foot.y0) as usize * fw + (x - foot.x0) as usize) * n)
        };
        let mut outp = vec![0.0f32; w * h * n];
        let mut any = false;
        let mut acc = [0.0f64; 8];
        let mut one = [0.0f64; 8];
        // One premultiplied sample at source position (u, v) (pixel centres at integers).
        let point = |interp: Interp, u: f64, v: f64, acc: &mut [f64; 8]| {
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
                    let (fx, fy) = (u - f64::from(ix), v - f64::from(iy));
                    if let Some(o) = base(ix, iy, 2) {
                        for (dy, wy) in [(0, 1.0 - fy), (1, fy)] {
                            for (dx, wx) in [(0, 1.0 - fx), (1, fx)] {
                                let p = &px[o + (dy * fw + dx) * n..][..n];
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
                                *s += at(ix + dx, iy + dy, c) * wx * wy;
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
                    if let Some(o) = base(ix - 1, iy - 1, 4) {
                        for (j, wyj) in wy.iter().enumerate() {
                            for (i, wxi) in wx.iter().enumerate() {
                                let k = wxi * wyj;
                                if k == 0.0 {
                                    continue;
                                }
                                let p = &px[o + (j * fw + i) * n..][..n];
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
                                *s += at(ix - 1 + i as i32, iy - 1 + j as i32, c) * k;
                            }
                        }
                    }
                }
            }
        };
        for (o, (q, &k)) in uv.iter().zip(&owner).enumerate() {
            if q[0].is_nan() {
                continue;
            }
            let (j, nx, ny) = jac[k as usize];
            if nx * ny == 1 {
                point(interp, q[0] - 0.5, q[1] - 0.5, &mut acc);
            } else {
                acc[..n].fill(0.0);
                for sy in 0..ny {
                    let t = (sy as f64 + 0.5) / ny as f64 - 0.5;
                    for sx in 0..nx {
                        let s = (sx as f64 + 0.5) / nx as f64 - 0.5;
                        point(Interp::Bilinear, q[0] + s * j[0] + t * j[2] - 0.5, q[1] + s * j[1] + t * j[3] - 0.5, &mut one);
                        for (c, v) in acc.iter_mut().zip(&one).take(n) {
                            *c += v;
                        }
                    }
                }
                let k = 1.0 / (nx * ny) as f64;
                for c in acc.iter_mut().take(n) {
                    *c *= k;
                }
            }
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
    let w = warp_mesh_surface(&tmp, src, f, interp);
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
        let out = warp_mesh_surface_with(&s, r, &map, Interp::Bicubic, &Interrupt::new(&never, &progress)).expect("not cancelled");
        let want = warp_mesh_surface(&s, r, &map, Interp::Bicubic);
        assert_eq!(out.content_bounds(), want.content_bounds());
        assert_eq!(out.read_region(want.content_bounds()), want.read_region(want.content_bounds()));
        assert!(*reported.lock().unwrap() > 0.9);
        let always = || true;
        assert!(warp_mesh_surface_with(&s, r, &map, Interp::Bicubic, &Interrupt::cancel_only(&always)).is_none());
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

    #[test]
    fn gray_masks_keep_their_default() {
        let mut m = Surface::with_default(PixelFormat::GRAY8, &[1.0]);
        m.fill_rect(Rect::new(4, 4, 10, 10), &[0.0]);
        let o = warp_mesh_gray(&m, &|x, y| (x + 5.0, y), Interp::Bilinear);
        assert_eq!(o.pixel(0, 0), vec![1.0]);
        assert_eq!(o.pixel(12, 6), vec![0.0]);
        assert_eq!(o.pixel(5, 6), vec![1.0]);
    }
}
