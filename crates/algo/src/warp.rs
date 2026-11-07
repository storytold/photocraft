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
//! source pixel), so thin lines don't alias. A strong shrink (four or more source pixels per
//! destination pixel along both axes) averages those subsamples from a box-reduced copy of the
//! source instead (`Level`), which reads a far smaller footprint for the same area average. Where
//! the warp folds over itself the later (lower-right) cell wins. Work is split per destination
//! tile (parallel on native).

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
/// lossless), bicubic otherwise. Shared by the engine's re-render and PSD export's filter cache.
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
        return Ok(warp_mesh_surface(src, src_rect, &map, Interp::Bicubic));
    }
    let near = |x: f64, y: f64| (x - y).abs() < 1e-9;
    if near(a, 1.0) && near(b, 0.0) && near(c, 0.0) && near(d, 1.0) && near(e, e.round()) && near(f, f.round()) {
        return Ok(crate::resample::translate_surface(src, e.round() as i32, f.round() as i32));
    }
    crate::transform::warp_surface(src, src_rect, &crate::transform::Homography([a, c, e, b, d, f, 0.0, 0.0, 1.0]), Interp::Bicubic)
}

/// [`place_source`] through a projective map `h` (source → document; Distort, Perspective): the
/// warp first (in source space), then `h`.
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
        return Ok(warp_mesh_surface(src, src_rect, &map, Interp::Bicubic));
    }
    crate::transform::warp_surface(src, src_rect, h, Interp::Bicubic)
}

/// Warps the content of `src` inside `src_rect` through the forward map `f` (source document
/// coordinates → destination document coordinates). Output has `src`'s format with alpha
/// added; pixels outside the warped area are transparent.
pub fn warp_mesh_surface(src: &Surface, src_rect: Rect, f: &(dyn Fn(f64, f64) -> (f64, f64) + Sync), interp: Interp) -> Surface {
    warp_mesh_surface_mips(src, src_rect, f, interp, true)
}

/// [`warp_mesh_surface`] reporting progress per destination tile; `None` once `ctl` is cancelled.
pub fn warp_mesh_surface_with(src: &Surface, src_rect: Rect, f: &(dyn Fn(f64, f64) -> (f64, f64) + Sync), interp: Interp, ctl: &Interrupt) -> Option<Surface> {
    let out = mesh(src, src_rect, f, interp, true, ctl);
    (!ctl.cancelled()).then_some(out)
}

/// [`warp_mesh_surface`], with the reduced-level sampling of strong shrinks switched by `mips`
/// (off: every shrink averages subsamples of the full-resolution source, the exact reference).
pub(crate) fn warp_mesh_surface_mips(src: &Surface, src_rect: Rect, f: &(dyn Fn(f64, f64) -> (f64, f64) + Sync), interp: Interp, mips: bool) -> Surface {
    mesh(src, src_rect, f, interp, mips, &Interrupt::NONE)
}

fn mesh(src: &Surface, src_rect: Rect, f: &(dyn Fn(f64, f64) -> (f64, f64) + Sync), interp: Interp, mips: bool, ctl: &Interrupt) -> Surface {
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
    triangles(src, src_rect, &verts, &tris, interp, mips, ctl)
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
    triangles(src, src_rect, verts, tris, interp, true, &Interrupt::NONE)
}

/// Destination tiles one triangle render may keep bookkeeping for (a 1M-tile render is 69 Gpx).
const MAX_DST_TILES: u64 = 1 << 20;
/// Source pixels one destination tile may read densely.
const MAX_FOOTPRINT_PIXELS: u64 = 1 << 25;

/// [`warp_triangles`], with the reduced-level sampling of strong shrinks switched by `mips`, and
/// reporting progress per destination tile to `ctl`. A cancelled `ctl` skips the remaining tiles;
/// the caller discards the result.
fn triangles(src: &Surface, src_rect: Rect, verts: &[([f64; 2], [f64; 2])], tris: &[[usize; 3]], interp: Interp, mips: bool, ctl: &Interrupt) -> Surface {
    // The caller's surface: reduced levels are reused by its tiles, not by those of the copy with
    // alpha made below (new on every call).
    let original = src;
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
    // Strong shrinks sample a box-reduced level of the source (see `Level`): the pixels of a
    // triangle that shrinks by at least 2^(L+1) along every direction. The choice is per triangle,
    // so a region is sampled the same way whichever tiles it falls in. One level serves the whole
    // warp, the finest any triangle can use, so memory stays one reduced copy of the source.
    let tri_level = |tri: &[usize; 3]| -> usize {
        match source_step(&tri.map(|v| verts[v])) {
            Some(s) if mips && interp != Interp::Nearest && s >= 4.0 => ((s / 2.0).log2().floor() as usize).min(MAX_LEVEL),
            _ => 0,
        }
    };
    let level = tris.iter().map(tri_level).filter(|&l| l > 0).min().and_then(|l| level_for(original, src, src_rect, l, n, ctl));
    // Encoded (and dropped if fully transparent) per tile, in parallel, rather than one tile at a
    // time once all are done. `t` lies in one tile.
    let encode = |t: Rect, px: &[f32]| -> Option<(TileCoord, Arc<Tile>)> {
        let mut one = Surface::new(fmt);
        one.write_region(t, px);
        one.prune();
        if one.tile_count() == 0 {
            return None;
        }
        one.take_tiles(t).pop()
    };
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
        // The pixels of triangles that shrink enough take the reduced level; the rest the exact
        // path below.
        let mut outp = vec![0.0f32; w * h * n];
        let mut any = false;
        let use_lv: Vec<bool> = match level.as_ref() {
            Some(lv) => tris.iter().map(|tri| tri_level(tri) >= lv.level).collect(),
            None => vec![false; tris.len()],
        };
        if let Some(lv) = level.as_ref()
            && use_lv.iter().any(|&u| u)
        {
            any |= lv.shade_into(&uv, &owner, tris, verts, n, &use_lv, &mut outp);
        }
        'exact: {
            // Source footprint of the pixels sampled exactly, including the spread of their subsamples.
            let (mut fx0, mut fy0, mut fx1, mut fy1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for (q, j) in uv.iter().zip(&owner).filter(|(q, k)| !q[0].is_nan() && !use_lv[**k as usize]).map(|(q, &k)| (q, jac[k as usize].0)) {
                let (ex, ey) = (0.5 * (j[0].abs() + j[2].abs()), 0.5 * (j[1].abs() + j[3].abs()));
                fx0 = fx0.min(q[0] - ex);
                fy0 = fy0.min(q[1] - ey);
                fx1 = fx1.max(q[0] + ex);
                fy1 = fy1.max(q[1] + ey);
            }
            if fx0 > fx1 {
                break 'exact;
            }
            let foot = Rect::new(fx0.floor() as i32 - 3, fy0.floor() as i32 - 3, fx1.ceil() as i32 + 3, fy1.ceil() as i32 + 3).intersect(&src_rect);
            // Sparse components far apart in the source can land in one tile: the dense window
            // between them is not read (the tile stays empty). The pixel-area cap is checked first:
            // `has_tiles_in` walks the source tile grid, so an oversized footprint must return
            // without paying for that scan.
            if foot.is_empty() || foot.size().area() > MAX_FOOTPRINT_PIXELS || !src.has_tiles_in(foot) {
                break 'exact;
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
                if q[0].is_nan() || use_lv[k as usize] {
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
        }
        if !any {
            return None;
        }
        encode(*t, &outp)
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

/// Coarsest reduced level a strong shrink samples (64× smaller per axis).
const MAX_LEVEL: usize = 6;

/// A reduced level kept while a [`reuse_levels`] scope is open, with what it was built from.
struct SharedLevel {
    /// Every tile of the source, by address: a [`std::sync::Weak`] reserves the allocation, so no
    /// other tile can take its address, and a tile changed in place moves to a new one
    /// (`Arc::make_mut` with weak references left), so equal tile pointers mean equal pixels.
    /// It doesn't keep the pixels alive: only the level's own floats count against the scope
    /// budget, and a source the caller drops is freed while the scope is open.
    tiles: Vec<(photocraft_geom::TileCoord, std::sync::Weak<photocraft_raster::Tile>)>,
    default: Vec<u8>,
    format: PixelFormat,
    rect: Rect,
    n: usize,
    level: std::sync::Arc<Level>,
}

thread_local! {
    /// The levels of the open [`reuse_levels`] scope on this thread (`None`: no scope).
    static LEVELS: std::cell::RefCell<Option<Vec<SharedLevel>>> = const { std::cell::RefCell::new(None) };
}

thread_local! {
    /// Reduced levels built on this thread.
    static LEVEL_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many reduced levels warps on this thread have built (a diagnostic: instances sharing
/// one inside a [`reuse_levels`] scope count once).
pub fn reduced_levels_built() -> usize {
    LEVEL_BUILDS.with(std::cell::Cell::get)
}

/// Open while alive: see [`reuse_levels`]. Bound to the thread that opened it.
pub struct LevelScope {
    owner: bool,
    _thread: std::marker::PhantomData<*const ()>,
}

impl Drop for LevelScope {
    fn drop(&mut self) {
        if self.owner {
            LEVELS.with(|l| *l.borrow_mut() = None);
        }
    }
}

/// While the returned guard lives, warps on this thread reuse the reduced level they built for
/// the same source pixels (the instances of one smart object all warp its contents) instead of
/// reducing the source again. Dropping it frees them. Scopes nest; the outermost one owns them.
pub fn reuse_levels() -> LevelScope {
    LEVELS.with(|l| {
        let mut l = l.borrow_mut();
        let owner = l.is_none();
        if owner {
            *l = Some(Vec::new());
        }
        LevelScope { owner, _thread: std::marker::PhantomData }
    })
}

/// Most floats one reduced level may hold (256 MB); a larger one is not built and the warp takes
/// the exact path.
const MAX_LEVEL_FLOATS: usize = 1 << 26;
/// Most floats the levels of one [`reuse_levels`] scope may hold together (512 MB); past it a new
/// level is used once and not kept.
const MAX_SCOPE_FLOATS: usize = 1 << 27;

/// [`MAX_SCOPE_FLOATS`], or a test's own for its thread.
fn scope_floats() -> usize {
    #[cfg(test)]
    if let Some(b) = SCOPE_FLOATS_OVERRIDE.with(std::cell::Cell::get) {
        return b;
    }
    MAX_SCOPE_FLOATS
}

#[cfg(test)]
thread_local! {
    static SCOPE_FLOATS_OVERRIDE: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Most source pixels one band read of [`Level::build`] takes at once: a band of a very wide source
/// is read in pieces of whole blocks.
const BAND_READ_PIXELS: usize = 1 << 22;

/// [`BAND_READ_PIXELS`], or a test's own for its thread.
fn band_read_pixels() -> usize {
    #[cfg(test)]
    if let Some(b) = BAND_READ_OVERRIDE.with(std::cell::Cell::get) {
        return b;
    }
    BAND_READ_PIXELS
}

#[cfg(test)]
thread_local! {
    static BAND_READ_OVERRIDE: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// [`Level::build`] from `src` (the pixels, with alpha), served from the open [`reuse_levels`]
/// scope when one has it. `key` is the surface the caller passed, which a scope compares by tile:
/// for a source without alpha `src` is a copy made per warp, and would never match.
fn level_for(key: &Surface, src: &Surface, r: Rect, level: usize, n: usize, ctl: &Interrupt) -> Option<std::sync::Arc<Level>> {
    let same = |e: &SharedLevel| {
        e.rect == r
            && e.level.level == level
            && e.n == n
            && e.format == key.format()
            && e.default == key.default_bytes()
            && e.tiles.len() == key.tiles().count()
            && e.tiles.iter().zip(key.tiles()).all(|((ca, ta), (cb, tb))| ca == cb && std::ptr::eq(ta.as_ptr(), std::sync::Arc::as_ptr(tb)))
    };
    if let Some(hit) = LEVELS.with(|l| l.borrow().as_ref().and_then(|v| v.iter().find(|e| same(e)).map(|e| e.level.clone()))) {
        return Some(hit);
    }
    let built = std::sync::Arc::new(Level::build(src, r, level, n, ctl)?);
    LEVELS.with(|l| {
        if let Some(v) = l.borrow_mut().as_mut() {
            // A level whose source was freed can never match again (its tile addresses stay
            // reserved): drop it, so it doesn't hold budget a live source could reuse.
            v.retain(|e| e.tiles.iter().all(|(_, t)| t.strong_count() > 0));
            if v.iter().map(|e| e.level.px.len()).sum::<usize>().saturating_add(built.px.len()) <= scope_floats() {
                let tiles = key.tiles().map(|(c, t)| (*c, std::sync::Arc::downgrade(t))).collect();
                v.push(SharedLevel { tiles, default: key.default_bytes().to_vec(), format: key.format(), rect: r, n, level: built.clone() });
            }
        }
    });
    Some(built)
}

/// The inverse map's Jacobian over one triangle `[(destination, source); 3]`: the source step
/// per destination pixel along x and along y, or `None` for a degenerate triangle. The same
/// arithmetic as the rasterizer's, so both agree on a triangle's scale.
fn jacobian(t: &[([f64; 2], [f64; 2]); 3]) -> Option<([f64; 2], [f64; 2])> {
    let [(p0, s0), (p1, s1), (p2, s2)] = *t;
    let det = (p1[0] - p0[0]) * (p2[1] - p0[1]) - (p2[0] - p0[0]) * (p1[1] - p0[1]);
    if det.abs() < 1e-12 {
        return None;
    }
    let (d1, d2) = ([s1[0] - s0[0], s1[1] - s0[1]], [s2[0] - s0[0], s2[1] - s0[1]]);
    let (l1x, l1y, l2x, l2y) = ((p2[1] - p0[1]) / det, (p0[0] - p2[0]) / det, (p0[1] - p1[1]) / det, (p1[0] - p0[0]) / det);
    Some(([l1x * d1[0] + l2x * d2[0], l1x * d1[1] + l2x * d2[1]], [l1y * d1[0] + l2y * d2[0], l1y * d1[1] + l2y * d2[1]]))
}

/// How many source pixels one destination pixel spans in the direction it spans the fewest: the
/// inverse map's smallest singular value. A reduced level averages 2^L pixels along both source
/// axes, so it must fit that: picked from the long axis, a 16 × 1 shrink would also average ~8
/// rows where one is right, and under a rotation both Jacobian columns can be long while the
/// shrink along one direction is small.
fn source_step(t: &[([f64; 2], [f64; 2]); 3]) -> Option<f64> {
    jacobian(t)
        .map(|(jx, jy)| {
            let sum = jx[0] * jx[0] + jx[1] * jx[1] + jy[0] * jy[0] + jy[1] * jy[1];
            let det = jx[0] * jy[1] - jx[1] * jy[0];
            (((sum - (sum * sum - 4.0 * det * det).max(0.0).sqrt()) / 2.0).max(0.0)).sqrt()
        })
        .filter(|s| s.is_finite())
}

/// The source reduced 2^`level` times per axis by a box filter, premultiplied: pixel `(i, j)`
/// averages the `2^level`-square block of the source rectangle at `(x0, y0) + 2^level·(i, j)`,
/// pixels past the rectangle counting as transparent (as the full-resolution path reads them).
/// A shrink by `s` samples it with steps of `s / 2^level` (2 to 4 subsamples per axis instead of
/// up to `MAX_SS`) and reads a 4^level smaller footprint, which is what makes warping a large
/// smart object into a small placement cheap.
struct Level {
    level: usize,
    scale: f64,
    x0: f64,
    y0: f64,
    w: usize,
    h: usize,
    px: Vec<f32>,
}

impl Level {
    /// `None` when the reduced copy would hold [`MAX_LEVEL_FLOATS`] or more (the exact path is
    /// used), or once `ctl` is cancelled. Bands of the source with no tiles (transparent by
    /// default) are not read.
    fn build(src: &Surface, r: Rect, level: usize, n: usize, ctl: &Interrupt) -> Option<Level> {
        LEVEL_BUILDS.with(|b| b.set(b.get() + 1));
        let k = 1usize << level;
        let (sw, sh) = (r.width() as usize, r.height() as usize);
        let (w, h) = (sw.div_ceil(k), sh.div_ceil(k));
        let len = w.checked_mul(h)?.checked_mul(n)?;
        if len >= MAX_LEVEL_FLOATS || sw == 0 || sh == 0 {
            return None;
        }
        let transparent = src.default_pixel().get(n - 1).is_none_or(|&a| a <= 0.0);
        let inv = 1.0 / (k * k) as f32;
        // A band is read in pieces of whole blocks, at most `BAND_READ_PIXELS` source pixels each.
        let piece = (band_read_pixels() / k / k).max(1) * k;
        let row = |(j, out): (usize, &mut [f32])| {
            if ctl.cancelled() {
                return;
            }
            let y0 = r.y0.saturating_add(i32::try_from(j * k).unwrap_or(i32::MAX));
            let y1 = y0.saturating_add(i32::try_from(k).unwrap_or(i32::MAX)).min(r.y1);
            for px0 in (0..sw).step_by(piece) {
                let x0 = r.x0.saturating_add(i32::try_from(px0).unwrap_or(i32::MAX));
                let x1 = x0.saturating_add(i32::try_from(piece).unwrap_or(i32::MAX)).min(r.x1);
                let part = Rect::new(x0, y0, x1, y1);
                if part.is_empty() || (transparent && !src.has_tiles_in(part)) {
                    continue;
                }
                let pw = part.width() as usize;
                let strip = src.read_region(part);
                for line in strip.chunks_exact(pw * n) {
                    for (x, p) in line.chunks_exact(n).enumerate() {
                        let al = p[n - 1];
                        let o = ((px0 + x) / k) * n;
                        for c in 0..n - 1 {
                            out[o + c] += p[c] * al;
                        }
                        out[o + n - 1] += al;
                    }
                }
            }
            for v in out.iter_mut() {
                *v *= inv;
            }
        };
        if ctl.cancelled() {
            return None;
        }
        let mut px = vec![0.0f32; len];
        #[cfg(not(target_arch = "wasm32"))]
        {
            use rayon::prelude::*;
            px.par_chunks_mut(w * n).enumerate().for_each(row);
        }
        #[cfg(target_arch = "wasm32")]
        px.chunks_mut(w * n).enumerate().for_each(row);
        (!ctl.cancelled()).then_some(Level { level, scale: k as f64, x0: f64::from(r.x0), y0: f64::from(r.y0), w, h, px })
    }

    /// One premultiplied bilinear sample at level position `(u, v)` (pixel centres at integers).
    fn bilinear(&self, u: f64, v: f64, n: usize, acc: &mut [f64; 8]) {
        acc[..n].fill(0.0);
        let (ix, iy) = (u.floor(), v.floor());
        let (fx, fy) = (u - ix, v - iy);
        for (dy, wy) in [(0.0, 1.0 - fy), (1.0, fy)] {
            for (dx, wx) in [(0.0, 1.0 - fx), (1.0, fx)] {
                let (x, y) = (ix + dx, iy + dy);
                if x < 0.0 || y < 0.0 || x >= self.w as f64 || y >= self.h as f64 {
                    continue;
                }
                let o = (y as usize * self.w + x as usize) * n;
                if let Some(p) = self.px.get(o..o + n) {
                    for (s, &c) in acc.iter_mut().zip(p) {
                        *s += f64::from(c) * wx * wy;
                    }
                }
            }
        }
    }

    /// Shades the pixels of a tile whose triangle `use_lv` marks from this level into `outp`: each
    /// averages subsamples spread over its footprint, `uv`/`owner` as the rasterizer left them.
    /// Same layout as the exact path; whether any pixel was written.
    #[allow(clippy::too_many_arguments)]
    fn shade_into(
        &self,
        uv: &[[f64; 2]],
        owner: &[u32],
        tris: &[[usize; 3]],
        verts: &[([f64; 2], [f64; 2])],
        n: usize,
        use_lv: &[bool],
        outp: &mut [f32],
    ) -> bool {
        let a = n - 1;
        let k = self.scale;
        let cap = |j: [f64; 2]| {
            let l = j[0].hypot(j[1]);
            if l > MAX_SS { [j[0] * MAX_SS / l, j[1] * MAX_SS / l] } else { j }
        };
        let count = |j: [f64; 2]| j[0].hypot(j[1]).round().clamp(1.0, MAX_SS) as usize;
        // Per triangle: its steps on this level and subsample counts.
        let steps: Vec<([f64; 2], [f64; 2], usize, usize)> = tris
            .iter()
            .map(|tri| match jacobian(&tri.map(|v| verts[v])) {
                Some((jx, jy)) => {
                    let (jx, jy) = (cap([jx[0] / k, jx[1] / k]), cap([jy[0] / k, jy[1] / k]));
                    (jx, jy, count(jx), count(jy))
                }
                None => ([0.0; 2], [0.0; 2], 1, 1),
            })
            .collect();
        let mut any = false;
        let (mut acc, mut one) = ([0.0f64; 8], [0.0f64; 8]);
        for (o, (q, &kk)) in uv.iter().zip(owner).enumerate() {
            if q[0].is_nan() || !use_lv.get(kk as usize).copied().unwrap_or(false) {
                continue;
            }
            let Some(&(jx, jy, nx, ny)) = steps.get(kk as usize) else { continue };
            let (u, v) = ((q[0] - self.x0) / k, (q[1] - self.y0) / k);
            acc[..n].fill(0.0);
            for sy in 0..ny {
                let tt = (sy as f64 + 0.5) / ny as f64 - 0.5;
                for sx in 0..nx {
                    let ss = (sx as f64 + 0.5) / nx as f64 - 0.5;
                    self.bilinear(u + ss * jx[0] + tt * jy[0] - 0.5, v + ss * jx[1] + tt * jy[1] - 0.5, n, &mut one);
                    for (c, x) in acc.iter_mut().zip(&one).take(n) {
                        *c += x;
                    }
                }
            }
            let inv = 1.0 / (nx * ny) as f64;
            let al = (acc[a] * inv).clamp(0.0, 1.0);
            if al <= 1e-6 {
                continue;
            }
            any = true;
            let b = o * n;
            for c in 0..a {
                // The unclamped alpha, as the exact path divides by (#6203).
                outp[b + c] = (acc[c] / acc[a]).clamp(0.0, 1.0) as f32;
            }
            outp[b + a] = al as f32;
        }
        any
    }
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

    /// Smooth bands plus per-pixel noise (deterministic), opaque.
    fn textured(w: i32, h: i32, seed: u32) -> Surface {
        let mut src = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
        let mut z = seed;
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                z = z.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let n = f64::from(z >> 8) / f64::from(1u32 << 24) - 0.5;
                let v = 0.5 + 0.3 * (f64::from(x) * 0.021).sin() * (f64::from(y) * 0.017).cos() + 0.15 * n;
                px.extend([v as f32, (1.0 - v) as f32, (f64::from(x) / f64::from(w)) as f32, 1.0]);
            }
        }
        src.write_region(Rect::new(0, 0, w, h), &px);
        src
    }

    fn mean_abs_diff(a: &Surface, b: &Surface) -> f64 {
        let r = a.content_bounds().union(&b.content_bounds());
        let (pa, pb) = (a.read_region(r), b.read_region(r));
        pa.iter().zip(&pb).map(|(x, y)| f64::from((x - y).abs())).sum::<f64>() / pa.len().max(1) as f64
    }

    #[test]
    fn a_reuse_scope_builds_a_reduced_level_once_per_source() {
        // The instances of one smart object warp the same source: inside a scope the reduced
        // level is built once, and every warp gives the pixels it gives alone.
        let src = textured(1024, 512, 5);
        let r = Rect::new(0, 0, 1024, 512);
        let f = |x: f64, y: f64| (x / 9.0 + 0.001 * y, y / 9.0);
        let g = |x: f64, y: f64| (x / 8.5, y / 9.5 + 0.002 * x);
        let alone = [warp_mesh_surface(&src, r, &f, Interp::Bicubic), warp_mesh_surface(&src, r, &g, Interp::Bicubic)];
        let builds = reduced_levels_built;
        let before = builds();
        let shared = {
            let _scope = reuse_levels();
            [warp_mesh_surface(&src, r, &f, Interp::Bicubic), warp_mesh_surface(&src, r, &g, Interp::Bicubic)]
        };
        assert_eq!(builds() - before, 1, "one level for both warps");
        for (a, b) in alone.iter().zip(&shared) {
            assert_eq!(a.content_bounds(), b.content_bounds());
            assert_eq!(a.read_region(a.content_bounds()), b.read_region(b.content_bounds()));
        }
        // Outside a scope every warp builds its own.
        let before = builds();
        let _ = warp_mesh_surface(&src, r, &f, Interp::Bicubic);
        let _ = warp_mesh_surface(&src, r, &f, Interp::Bicubic);
        assert_eq!(builds() - before, 2);
    }

    #[test]
    fn a_reuse_scope_does_not_keep_a_dropped_source_alive() {
        // Instances whose sources differ (one converted per instance) each keep a level in the
        // scope; their source tiles must still be freed once the caller drops them, or N
        // instances would hold N sources until the scope closes.
        let r = Rect::new(0, 0, 1024, 512);
        let f = |x: f64, y: f64| (x / 9.0 + 0.001 * y, y / 9.0);
        let _scope = reuse_levels();
        let (tiles, again) = {
            let src = textured(1024, 512, 6);
            let tiles: Vec<_> = src.tiles().map(|(_, t)| std::sync::Arc::downgrade(t)).collect();
            let first = warp_mesh_surface(&src, r, &f, Interp::Bicubic);
            let before = reduced_levels_built();
            let again = warp_mesh_surface(&src, r, &f, Interp::Bicubic);
            assert_eq!(reduced_levels_built(), before, "reused while the source lives");
            assert_eq!(first.read_region(first.content_bounds()), again.read_region(again.content_bounds()));
            (tiles, again)
        };
        assert!(!tiles.is_empty() && tiles.iter().all(|t| t.strong_count() == 0), "the scope holds no source tile");
        // A new source at the same place, with other pixels, never matches the old one's level.
        let other = textured(1024, 512, 7);
        let before = reduced_levels_built();
        let fresh = warp_mesh_surface(&other, r, &f, Interp::Bicubic);
        assert_eq!(reduced_levels_built() - before, 1, "built for the new source");
        assert_ne!(fresh.read_region(fresh.content_bounds()), again.read_region(again.content_bounds()));
    }

    #[test]
    fn a_reuse_scope_never_serves_the_level_of_other_pixels() {
        // A copy sharing all but one tile with the source gets its own level.
        let src = textured(1024, 512, 5);
        let mut other = src.clone();
        other.fill_rect(Rect::new(0, 0, 64, 64), &[1.0, 0.0, 0.0, 1.0]);
        let r = Rect::new(0, 0, 1024, 512);
        let f = |x: f64, y: f64| (x / 9.0, y / 9.0);
        let expect = warp_mesh_surface(&other, r, &f, Interp::Bicubic);
        let _scope = reuse_levels();
        let _ = warp_mesh_surface(&src, r, &f, Interp::Bicubic);
        let got = warp_mesh_surface(&other, r, &f, Interp::Bicubic);
        assert_eq!(got.read_region(got.content_bounds()), expect.read_region(expect.content_bounds()));
    }

    #[test]
    fn a_level_is_not_built_past_its_memory_bound_nor_once_cancelled() {
        let src = textured(64, 64, 1);
        let r = Rect::new(0, 0, 64, 64);
        assert!(Level::build(&src, r, 1, 4, &Interrupt::NONE).is_some());
        // 2^24 × 2^24 at level 1 asks for 2^48 × 4 floats: refused before anything is allocated.
        assert!(Level::build(&src, Rect::new(0, 0, 1 << 24, 1 << 24), 1, 4, &Interrupt::NONE).is_none());
        let always = || true;
        assert!(Level::build(&src, r, 1, 4, &Interrupt::cancel_only(&always)).is_none());
    }

    #[test]
    fn mips_leave_identity_affine_and_mild_shrinks_untouched() {
        let r = Rect::new(0, 0, 300, 200);
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let src = textured(300, 200, 7).convert(PixelFormat::new(ColorMode::Rgb, depth, true));
            // Up to a step of 4 source px (k · |(1, 0.25)| < 4 here) the exact path runs unchanged.
            for k in [1.0, 0.8, 1.5, 2.0, 3.8] {
                let f = move |x: f64, y: f64| (x / k + 0.25 * y / k, y / k);
                let mipped = warp_mesh_surface(&src, r, &f, Interp::Bicubic);
                let exact = warp_mesh_surface_mips(&src, r, &f, Interp::Bicubic, false);
                assert_eq!(mipped.content_bounds(), exact.content_bounds(), "{depth:?}, k = {k}");
                assert_eq!(mipped.read_region(mipped.content_bounds()), exact.read_region(exact.content_bounds()), "{depth:?}, k = {k}");
            }
        }
    }

    #[test]
    fn a_mipped_shrink_stays_close_to_the_exact_area_average() {
        let src = textured(2048, 1024, 3);
        let r = Rect::new(0, 0, 2048, 1024);
        for (kx, ky) in [(7.3, 6.1), (16.0, 16.0), (30.0, 11.0)] {
            // A shear and a bend too, so triangles differ in scale across a tile.
            let f = move |x: f64, y: f64| (x / kx + 0.002 * y, y / ky + 6.0 * (x / 2048.0 * std::f64::consts::PI).sin());
            let mipped = warp_mesh_surface(&src, r, &f, Interp::Bicubic);
            let exact = warp_mesh_surface_mips(&src, r, &f, Interp::Bicubic, false);
            let d = mean_abs_diff(&mipped, &exact) * 255.0;
            assert!(d < 1.0, "{kx}x{ky}: mean diff {d:.3}/255");
        }
    }

    #[test]
    fn a_mipped_shrink_of_a_fine_checker_is_flat_grey() {
        let mut src = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
        let (w, h) = (1024, 1024);
        let px: Vec<f32> = (0..h).flat_map(|y| (0..w).map(move |x| if (x + y) % 2 == 0 { 1.0 } else { 0.0 })).flat_map(|v| [v, v, v, 1.0]).collect();
        src.write_region(Rect::new(0, 0, w, h), &px);
        let f = |x: f64, y: f64| (x / 16.0, y / 16.0);
        let out = warp_mesh_surface(&src, Rect::new(0, 0, w, h), &f, Interp::Bicubic);
        let o = out.read_region(Rect::new(4, 4, 60, 60));
        let worst = o.as_chunks::<4>().0.iter().map(|p| (p[0] - 0.5).abs()).fold(0.0f32, f32::max);
        assert!(worst < 0.02, "worst deviation from grey {worst}");
    }

    #[test]
    fn an_anisotropic_shrink_is_not_blurred_along_its_short_axis() {
        // 16 × 1 keeps the exact path (one row per pixel); 32 × 4 and 16 × 4 take a level, picked
        // from the 4 × axis: one from the 32 × or 16 × axis would also average ~8 rows.
        let src = textured(2048, 512, 9);
        let r = Rect::new(0, 0, 2048, 512);
        for (kx, ky, mipped_path) in [(16.0, 1.0, false), (32.0, 4.0, true), (16.0, 4.0, true)] {
            let f = move |x: f64, y: f64| (x / kx, y / ky);
            let before = reduced_levels_built();
            let mipped = warp_mesh_surface(&src, r, &f, Interp::Bicubic);
            assert_eq!(reduced_levels_built() > before, mipped_path, "{kx} x {ky}");
            let exact = warp_mesh_surface_mips(&src, r, &f, Interp::Bicubic, false);
            let d = mean_abs_diff(&mipped, &exact) * 255.0;
            assert!(d < 1.0, "{kx} x {ky}: mean diff {d:.3}/255");
        }
    }

    #[test]
    fn a_rotated_anisotropic_shrink_picks_its_level_from_the_short_direction() {
        // 32 × 4 turned by 30°: both Jacobian columns are over 16 source pixels long, but the
        // shrink along one direction is only 4 ×, so the level must be the one a 4 × shrink takes.
        let src = textured(2048, 1024, 13);
        let r = Rect::new(0, 0, 2048, 1024);
        let (c, s) = (30f64.to_radians().cos(), 30f64.to_radians().sin());
        let f = move |x: f64, y: f64| {
            let (u, v) = (x / 32.0, y / 4.0);
            (c * u - s * v + 200.0, s * u + c * v + 50.0)
        };
        let mipped = warp_mesh_surface(&src, r, &f, Interp::Bicubic);
        let exact = warp_mesh_surface_mips(&src, r, &f, Interp::Bicubic, false);
        let d = mean_abs_diff(&mipped, &exact) * 255.0;
        assert!(d < 1.0, "mean diff {d:.3}/255");
    }

    #[test]
    fn a_strongly_shrunk_region_is_sampled_the_same_whatever_shares_its_tile() {
        // Left half 16 × smaller both ways, right half only 2 × across: the left half's pixels
        // come from the reduced level even in tiles the right half's triangles share, exactly as
        // when the whole source shrinks 16 × (no seam where tiles change path).
        let src = textured(2048, 1024, 4);
        let r = Rect::new(0, 0, 2048, 1024);
        let grid = |f: &dyn Fn(f64, f64) -> (f64, f64)| {
            let (nx, ny) = (32usize, 16usize);
            let mut verts = Vec::new();
            for j in 0..=ny {
                for i in 0..=nx {
                    let (sx, sy) = (2048.0 * i as f64 / nx as f64, 1024.0 * j as f64 / ny as f64);
                    let (dx, dy) = f(sx, sy);
                    verts.push(([dx, dy], [sx, sy]));
                }
            }
            let w1 = nx + 1;
            let mut tris = Vec::new();
            for j in 0..ny {
                for i in 0..nx {
                    let a = j * w1 + i;
                    tris.push([a, a + 1, a + w1 + 1]);
                    tris.push([a, a + w1 + 1, a + w1]);
                }
            }
            (verts, tris)
        };
        let (vm, tm) = grid(&|x, y| (if x < 1024.0 { x / 16.0 } else { 64.0 + (x - 1024.0) / 2.0 }, y / 16.0));
        let (vs, ts) = grid(&|x, y| (x / 16.0, y / 16.0));
        let mixed = warp_triangles(&src, r, &vm, &tm, Interp::Bicubic);
        let strong = warp_triangles(&src, r, &vs, &ts, Interp::Bicubic);
        // Away from where the halves meet (x = 64).
        let left = Rect::new(0, 0, 60, 64);
        assert_eq!(mixed.read_region(left), strong.read_region(left));
    }

    #[test]
    fn a_source_without_alpha_reuses_its_level_in_a_scope() {
        // The warp adds alpha to such a source on every call; the scope still knows it by the
        // caller's tiles.
        let src = textured(1024, 512, 5).convert(PixelFormat::new(ColorMode::Rgb, SampleType::U8, false));
        let r = Rect::new(0, 0, 1024, 512);
        let f = |x: f64, y: f64| (x / 9.0, y / 9.0);
        let before = reduced_levels_built();
        {
            let _scope = reuse_levels();
            let _ = warp_mesh_surface(&src, r, &f, Interp::Bicubic);
            let _ = warp_mesh_surface(&src, r, &f, Interp::Bicubic);
        }
        assert_eq!(reduced_levels_built() - before, 1);
    }

    #[test]
    fn a_dropped_source_level_makes_room_for_a_live_one() {
        // With room for one level, the level of a source dropped earlier in the scope must not
        // keep a live source's level out: it is pruned, and the live one is reused.
        let r = Rect::new(0, 0, 1024, 512);
        let f = |x: f64, y: f64| (x / 9.0, y / 9.0);
        let _scope = reuse_levels();
        let _ = warp_mesh_surface(&textured(1024, 512, 8), r, &f, Interp::Bicubic);
        let one = LEVELS.with(|l| l.borrow().as_ref().map_or(0, |v| v.iter().map(|e| e.level.px.len()).sum::<usize>()));
        assert!(one > 0, "the dropped source's level was kept");
        SCOPE_FLOATS_OVERRIDE.with(|o| o.set(Some(one)));
        let live = textured(1024, 512, 9);
        let before = reduced_levels_built();
        let _ = [warp_mesh_surface(&live, r, &f, Interp::Bicubic), warp_mesh_surface(&live, r, &f, Interp::Bicubic)];
        SCOPE_FLOATS_OVERRIDE.with(|o| o.set(None));
        assert_eq!(reduced_levels_built() - before, 1, "the live source's level is kept and reused");
    }

    #[test]
    fn a_level_past_the_scope_budget_is_used_but_not_kept() {
        let src = textured(1024, 512, 5);
        let r = Rect::new(0, 0, 1024, 512);
        let f = |x: f64, y: f64| (x / 9.0, y / 9.0);
        let alone = warp_mesh_surface(&src, r, &f, Interp::Bicubic);
        SCOPE_FLOATS_OVERRIDE.with(|o| o.set(Some(1)));
        let before = reduced_levels_built();
        let got = {
            let _scope = reuse_levels();
            [warp_mesh_surface(&src, r, &f, Interp::Bicubic), warp_mesh_surface(&src, r, &f, Interp::Bicubic)]
        };
        SCOPE_FLOATS_OVERRIDE.with(|o| o.set(None));
        assert_eq!(reduced_levels_built() - before, 2, "each warp builds its own: none is kept");
        for g in &got {
            assert_eq!(g.read_region(g.content_bounds()), alone.read_region(alone.content_bounds()));
        }
    }

    #[test]
    fn a_band_read_in_pieces_gives_the_same_level() {
        let src = textured(1000, 300, 2);
        let r = Rect::new(0, 0, 1000, 300);
        let whole = Level::build(&src, r, 2, 4, &Interrupt::NONE).unwrap();
        BAND_READ_OVERRIDE.with(|o| o.set(Some(16 * 7)));
        let pieces = Level::build(&src, r, 2, 4, &Interrupt::NONE).unwrap();
        BAND_READ_OVERRIDE.with(|o| o.set(None));
        assert_eq!(whole.px, pieces.px);
    }

    #[test]
    fn a_level_of_a_rect_ending_at_i32_max_does_not_overflow() {
        // The last band of a rectangle that ends at the edge of the coordinate range starts within
        // 2^level of `i32::MAX`: its bottom edge (`y0 + 2^level`) and its pieces' right edges
        // (`x0 + piece`) must clip to the rectangle, not overflow. Both a transparent default
        // (bands without tiles are skipped) and an opaque one (every band is read).
        let edge = i32::MAX;
        let r = Rect::new(edge - 6, edge - 6, edge, edge);
        for c in [[0.0f32; 4], [0.2, 0.4, 0.6, 1.0]] {
            let src = Surface::with_default(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true), &c);
            let lv = Level::build(&src, r, 2, 4, &Interrupt::NONE).unwrap();
            assert_eq!((lv.w, lv.h), (2, 2));
            // Block (0, 0) is whole; block (1, 1) holds 2 × 2 of its 4 × 4 pixels, the rest
            // past the rectangle counting as transparent.
            let premul = |cover: f32| [c[0] * c[3] * cover, c[1] * c[3] * cover, c[2] * c[3] * cover, c[3] * cover];
            for (got, want) in [(&lv.px[..4], premul(1.0)), (&lv.px[12..], premul(0.25))] {
                assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() < 1e-6), "{got:?} vs {want:?}");
            }
        }
    }

    #[test]
    #[ignore = "benchmark"]
    fn mip_bench() {
        let src = textured(8192, 4096, 11);
        let r = Rect::new(0, 0, 8192, 4096);
        let f = |x: f64, y: f64| (x / 8.0 + 0.01 * y, y / 8.0);
        for (name, mips) in [("exact", false), ("mipped", true)] {
            let t = std::time::Instant::now();
            let out = warp_mesh_surface_mips(&src, r, &f, Interp::Bicubic, mips);
            println!("{name}: {:.0} ms ({:?})", t.elapsed().as_secs_f64() * 1000.0, out.content_bounds());
        }
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
