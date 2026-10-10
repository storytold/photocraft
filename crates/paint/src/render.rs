//! Dab rasterisation and stroke compositing.
//!
//! A stroke paints into a sparse, tiled *stroke buffer* (coverage plus, with Color Dynamics, a
//! premultiplied colour), exactly like a private stroke layer: dabs build up with flow up to their
//! opacity ceiling, stroke-level masks (Dual Brush, Texture) apply when the buffer is composited,
//! and the whole buffer composites onto the pre-stroke pixels at the stroke opacity. Because the
//! buffer is sparse and compositing always starts from the pre-stroke pixels, strokes can be fed in
//! chunks and re-composited incrementally for interactive painting (see [`StrokeRenderer`]).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;

use photocraft_color::{BlendMode, PixelFormat};
use photocraft_geom::Rect;
use photocraft_raster::{Surface, from_rgba_into, to_rgba};

use crate::brush::{BrushSettings, MaskMode, Pattern, TipShape};
use crate::dynamics::DabGenerator;
use crate::retouch::{Footprint, alpha_index, over_native};
use crate::rng::hash2;
use crate::tile::{Mips, PatternImage};
use crate::{Dab, StrokePoint, dab_coverage, tip_falloff};

/// Edge of a stroke-buffer tile.
pub const COV_TILE: i32 = 64;
/// Computed tips with a smaller radius (in pixels) are rasterised by area, not at pixel centres.
const SMALL_TIP_RADIUS: f32 = 3.0;
/// Sub-pixel sample offsets (a 4×4 grid) for the area coverage of small tips.
const SUBPIXEL: [f32; 4] = [-0.375, -0.125, 0.125, 0.375];
const NOISE_SALT: u64 = 0x006E_6F69_7365;

/// One independent coverage tile offered to a hardware rasterizer. Dabs remain ordered.
#[derive(Debug)]
pub struct RasterTile {
    pub origin: [i32; 2],
    pub dabs: Vec<usize>,
    pub coverage: Vec<f32>,
}

/// Optional acceleration of computed-tip coverage. Returning false must leave tiles unchanged;
/// the CPU then processes the batch. Native pixels, colour conversion and history stay generic.
pub trait CoverageAccelerator: std::fmt::Debug + Send + Sync {
    fn supports(&self, brush: &BrushSettings, dual: bool, dabs: &[Dab], tiles: usize) -> bool;
    fn rasterize(&self, brush: &BrushSettings, dual: bool, dabs: &[Dab], tiles: &mut [RasterTile]) -> bool;
}

#[inline]
/// Where an aliased (Pencil) dab of `diameter` pixels centred near (`x`, `y`) lands on the pixel
/// grid: the centre of the pixel holding the point for odd diameters, the nearest pixel corner
/// for even ones. Its footprint is then a whole-pixel block around that point, the same square
/// the Pencil cursor shows ([`grid_square`]).
pub fn grid_center(x: f64, y: f64, diameter: f32) -> (f32, f32) {
    let n = if diameter.is_finite() { diameter.round().max(1.0) as i64 } else { 1 };
    let snap = |v: f64| if n % 2 == 1 { v.floor() + 0.5 } else { v.round() };
    (snap(x) as f32, snap(y) as f32)
}

/// The whole-pixel square `[x0, y0, x1, y1]` an aliased dab of `diameter` pixels at (`x`, `y`)
/// can touch (see [`grid_center`]): the Pencil's cursor.
pub fn grid_square(x: f64, y: f64, diameter: f32) -> [f64; 4] {
    let n = if diameter.is_finite() { f64::from(diameter.round().clamp(1.0, 100_000.0)) } else { 1.0 };
    let (cx, cy) = grid_center(x, y, n as f32);
    let h = n / 2.0;
    [f64::from(cx) - h, f64::from(cy) - h, f64::from(cx) + h, f64::from(cy) + h]
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Wet Edges: how much paint stays at a pixel whose nearest dab reaches it at `rn` (0 at a dab's
/// centre, 1 at its rim). The interior keeps half the paint, the rim all of it.
#[inline]
fn wet_factor(rn: f32) -> f32 {
    0.5 + 0.5 * smoothstep(0.5, 1.0, rn)
}

/// Combine brush coverage `v` with a mask value `t` (texture/dual tip) at `depth`.
/// Every mode maps `v = 0` to 0 (a mask never adds paint where the brush has none).
pub fn mask_combine(mode: MaskMode, v: f32, t: f32, d: f32) -> f32 {
    if v <= 0.0 {
        return 0.0;
    }
    let lerp = |a: f32, b: f32| a + (b - a) * d;
    let r = match mode {
        MaskMode::Multiply => v * (1.0 - d + d * t),
        MaskMode::Subtract => (v - d * (1.0 - t)).max(0.0),
        MaskMode::Darken => v.min(1.0 - d + d * t),
        MaskMode::Overlay => {
            let o = if v < 0.5 { 2.0 * v * t } else { 1.0 - 2.0 * (1.0 - v) * (1.0 - t) };
            lerp(v, o)
        }
        MaskMode::ColorDodge => lerp(v, (v / (1.0 - t).max(1e-3)).min(1.0)),
        MaskMode::ColorBurn => lerp(v, 1.0 - ((1.0 - v) / t.max(1e-3)).min(1.0)),
        MaskMode::LinearBurn => lerp(v, (v + t - 1.0).max(0.0)),
        MaskMode::HardMix => lerp(v, if v + t > 1.0 { 1.0 } else { 0.0 }),
        MaskMode::LinearHeight => v * ((t - (1.0 - d * v)) * 4.0 + 0.5).clamp(0.0, 1.0),
        MaskMode::Height => {
            if t >= 1.0 - d * v {
                v
            } else {
                0.0
            }
        }
    };
    r.clamp(0.0, 1.0)
}

/// A stroke-ready brush: the settings plus prepared tip mips and texture.
#[derive(Clone, Debug)]
pub struct BrushContext {
    pub brush: BrushSettings,
    tip: Option<Arc<Mips>>,
    dual_tip: Option<Arc<Mips>>,
    texture: Option<Arc<PatternImage>>,
}

fn prepare_pattern(b: &BrushSettings) -> Option<Arc<PatternImage>> {
    let t = &b.texture;
    if !t.enabled {
        return None;
    }
    let mut img = match &t.pattern {
        Pattern::Procedural { style, size, seed } => crate::procedural::pattern(*style, *size, *seed),
        p => match p.bitmap() {
            Some(g) if g.is_valid() => PatternImage { width: g.width as usize, height: g.height as usize, data: g.to_f32() },
            _ => return None,
        },
    };
    let (br, ct) = (t.brightness.clamp(-1.0, 1.0), t.contrast.clamp(-1.0, 1.0));
    for v in &mut img.data {
        let mut x = (*v - 0.5) * (1.0 + ct) + 0.5 + br;
        x = x.clamp(0.0, 1.0);
        *v = if t.invert { 1.0 - x } else { x };
    }
    Some(Arc::new(img))
}

/// The pixel rectangle `reach` (plus 1 px of anti-aliasing slack) around a dab's centre. The casts
/// saturate and the arithmetic saturates, so a far or non-finite centre or reach from a direct
/// caller gives a (possibly empty) rectangle instead of an `i32` overflow (#977).
pub(crate) fn rect_around(d: &Dab, reach: f32) -> Rect {
    let rr = (reach.ceil() as i32).saturating_add(1);
    let (cx, cy) = (d.center.x.floor() as i32, d.center.y.floor() as i32);
    Rect::new(cx.saturating_sub(rr), cy.saturating_sub(rr), cx.saturating_add(rr).saturating_add(1), cy.saturating_add(rr).saturating_add(1))
}

impl BrushContext {
    pub fn new(brush: &BrushSettings) -> Self {
        let brush = brush.bounded_for_render();
        let mips = |t: &TipShape| match t.bitmap() {
            Some(g) if g.is_valid() => Some(Arc::new(Mips::new(g))),
            _ => None,
        };
        Self {
            tip: mips(&brush.tip),
            dual_tip: if brush.dual_brush.enabled { mips(&brush.dual_brush.tip) } else { None },
            texture: prepare_pattern(&brush),
            brush,
        }
    }

    /// Texture value at a document pixel (bilinear, tiled).
    #[inline]
    pub fn texture_at(&self, x: i32, y: i32) -> f32 {
        let Some(p) = &self.texture else { return 1.0 };
        let s = self.brush.texture.scale.max(0.01);
        if s == 1.0 {
            // Pixel centres land exactly on texels at 100 %.
            return p.data[y.rem_euclid(p.height as i32) as usize * p.width + x.rem_euclid(p.width as i32) as usize];
        }
        let (fx, fy) = ((x as f32 + 0.5) / s - 0.5, (y as f32 + 0.5) / s - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let a = p.sample_wrap(x0, y0) + (p.sample_wrap(x0 + 1.0, y0) - p.sample_wrap(x0, y0)) * tx;
        let b = p.sample_wrap(x0, y0 + 1.0) + (p.sample_wrap(x0 + 1.0, y0 + 1.0) - p.sample_wrap(x0, y0 + 1.0)) * tx;
        a + (b - a) * ty
    }

    /// Bounding rectangle of a dab (rotated sampled tips need the corner reach).
    pub fn dab_rect(&self, d: &Dab, dual: bool) -> Rect {
        let sampled = if dual { self.dual_tip.is_some() } else { self.tip.is_some() };
        let reach = if sampled { d.radius * std::f32::consts::SQRT_2 } else { d.radius };
        rect_around(d, reach)
    }

    /// Rasterise a dab over `rect` into `out` (tip shape × noise × per-tip texture × wet edges × flow).
    /// Wet Edges shapes this one dab; a stroke applies it to its accumulated coverage instead.
    pub fn rasterize(&self, d: &Dab, dual: bool, rect: Rect, out: &mut Vec<f32>) {
        self.rasterize_into(d, dual, rect, out, None);
    }

    /// [`rasterize`](Self::rasterize); with `depth`, Wet Edges is left out of `out` and each
    /// painted pixel's depth inside the dab (`1 − rn`) goes to `depth`, for the stroke to apply it.
    fn rasterize_into(&self, d: &Dab, dual: bool, rect: Rect, out: &mut Vec<f32>, mut depth: Option<&mut Vec<f32>>) {
        let (w, h) = (rect.width() as usize, rect.height() as usize);
        out.clear();
        out.resize(w * h, 0.0);
        if let Some(dp) = depth.as_deref_mut() {
            dp.clear();
            dp.resize(w * h, 0.0);
        }
        let capture_depth = depth.is_some();
        self.rasterize_with(d, dual, rect, capture_depth, |i, v, dp| {
            if let Some(dst) = out.get_mut(i) {
                *dst = v;
            }
            if let Some(dst) = depth.as_deref_mut().and_then(|depth| depth.get_mut(i)) {
                *dst = dp;
            }
        });
    }

    /// Shared sampling math; absolute coordinates preserve sampling phase across tiles.
    fn rasterize_with(&self, d: &Dab, dual: bool, rect: Rect, capture_depth: bool, mut emit: impl FnMut(usize, f32, f32)) {
        let (w, h) = (rect.width() as usize, rect.height() as usize);
        let b = &self.brush;
        let (hardness, mips) = if dual { (b.dual_brush.hardness, self.dual_tip.as_deref()) } else { (b.hardness, self.tip.as_deref()) };
        let aliased = b.aliased && !dual;
        let wet = b.wet_edges && !dual && !capture_depth;
        let noise = b.noise && !dual;
        let tex_tip = !dual && b.texture.enabled && b.texture.each_tip && self.texture.is_some();
        let (cx, cy) = if aliased { grid_center(d.center.x, d.center.y, 2.0 * d.radius) } else { (d.center.x as f32, d.center.y as f32) };
        let (sn, cs) = d.angle.sin_cos();
        let (fx, fy) = (if d.flip_x { -1.0 } else { 1.0 }, if d.flip_y { -1.0 } else { 1.0 });
        // Brush Projection: stretch the sampling coordinates along the tilt direction, which
        // foreshortens the tip there by `proj_scale`.
        let proj = (!dual && d.proj_scale < 0.999).then(|| {
            let (ps, pc) = d.proj_angle.sin_cos();
            (pc, ps, 1.0 / d.proj_scale.max(0.05) - 1.0)
        });
        let r = d.radius;
        let ro = d.roundness.max(0.5 / r).min(1.0);
        let rm = (r * ro).max(0.5);
        let reach2 = (rm + 0.5) * (rm + 0.5);
        // Sampled-tip mapping.
        let (level, inv_scale, tw, th) = match mips {
            Some(m) => {
                let (w0, h0, _) = &m.levels[0];
                let big = *w0.max(h0) as f32;
                (m.level_for(2.0 * r * ro.max(0.25)), big / (2.0 * r), *w0 as f32, *h0 as f32)
            }
            None => (0, 1.0, 1.0, 1.0),
        };
        // A pixel offset from the dab centre in tip space: to y-up, project, rotate by -angle, flip.
        let to_tip = |dx: f32, dy: f32| {
            let (mut ux, mut uy) = (dx, -dy);
            if let Some((ax, ay, k)) = proj {
                let t = (ux * ax + uy * ay) * k;
                ux += t * ax;
                uy += t * ay;
            }
            ((ux * cs + uy * sn) * fx, (-ux * sn + uy * cs) * fy)
        };
        // A tip a few pixels across takes each pixel's covered area (a 4×4 grid inside it):
        // sampling only the pixel centre made a tiny dab's total coverage swing with its sub-pixel
        // position, and thin lines beaded.
        let small = mips.is_none() && !aliased && rm < SMALL_TIP_RADIUS;
        let small_reach2 = (rm + 0.75) * (rm + 0.75);
        for yy in 0..h {
            let y = rect.y0 + yy as i32;
            let dy = y as f32 + 0.5 - cy;
            for xx in 0..w {
                let x = rect.x0 + xx as i32;
                let dx = x as f32 + 0.5 - cx;
                let (u, v) = to_tip(dx, dy);
                let (mut val, rn) = match mips {
                    // Block: the whole-pixel square, hard (the Eraser's Block mode).
                    _ if aliased && b.square => {
                        if dx.abs() >= r || dy.abs() >= r {
                            continue;
                        }
                        (1.0, 0.0)
                    }
                    None if small => {
                        let d2 = (u * ro).powi(2) + v * v;
                        if d2 >= small_reach2 {
                            continue;
                        }
                        let mut sum = 0.0;
                        for sy in SUBPIXEL {
                            for sx in SUBPIXEL {
                                let (su, sv) = to_tip(dx + sx, dy + sy);
                                let sd = ((su * ro).powi(2) + sv * sv).sqrt();
                                // Each sample anti-aliases over its own quarter pixel, so the
                                // covered area changes smoothly with the dab's position.
                                let inside = ((rm - sd) * 4.0 + 0.5).clamp(0.0, 1.0);
                                if inside > 0.0 {
                                    sum += inside * tip_falloff(sd.min(rm), rm, hardness);
                                }
                            }
                        }
                        (sum / (SUBPIXEL.len() * SUBPIXEL.len()) as f32, d2.sqrt() / rm)
                    }
                    None => {
                        let d2 = (u * ro).powi(2) + v * v;
                        if d2 >= reach2 {
                            continue;
                        }
                        let dd = d2.sqrt();
                        let val = if aliased { if dd <= rm { 1.0 } else { 0.0 } } else { dab_coverage(dd, rm, hardness) };
                        (val, dd / rm)
                    }
                    Some(m) => {
                        let (px, py) = (u * inv_scale + tw / 2.0, -(v / ro) * inv_scale + th / 2.0);
                        if px < -1.0 || py < -1.0 || px > tw + 1.0 || py > th + 1.0 {
                            continue;
                        }
                        let s = m.sample_clamped(level, px / tw, py / th);
                        let val = if aliased { if s >= 0.5 { 1.0 } else { 0.0 } } else { s };
                        (val, 1.0 - s)
                    }
                };
                if val <= 0.0 {
                    continue;
                }
                if noise && val < 1.0 {
                    let n = hash2(x, y, NOISE_SALT);
                    let hard = if n < val { 1.0 } else { 0.0 };
                    val += (hard - val) * 0.7;
                }
                if tex_tip {
                    val = mask_combine(b.texture.mode, val, self.texture_at(x, y), d.depth);
                }
                if wet {
                    val *= wet_factor(rn);
                }
                emit(yy * w + xx, val * d.alpha, (1.0 - rn).clamp(0.0, 1.0));
            }
        }
    }

    /// One dab's dense footprint (for the sequential retouch tools).
    pub fn footprint(&self, d: &Dab, index: usize) -> Footprint {
        let rect = self.dab_rect(d, false);
        let mut cov = Vec::new();
        self.rasterize(d, false, rect, &mut cov);
        Footprint { dab: *d, index, rect, cov }
    }

    /// Stroke-level masks applied to accumulated coverage `c` at a pixel.
    #[inline]
    pub fn stroke_mask(&self, c: f32, dual: Option<f32>, x: i32, y: i32) -> f32 {
        self.stroke_mask_wet(c, None, dual, x, y)
    }

    /// [`stroke_mask`](Self::stroke_mask) with Wet Edges from the stroke's depth at the pixel
    /// (the deepest any dab reaches it, see [`CoverageMap::depth`]).
    #[inline]
    fn stroke_mask_wet(&self, c: f32, depth: Option<f32>, dual: Option<f32>, x: i32, y: i32) -> f32 {
        let b = &self.brush;
        let mut m = match depth {
            Some(dp) => c * wet_factor(1.0 - dp),
            None => c,
        };
        if let Some(dv) = dual {
            m = mask_combine(b.dual_brush.mode, m, dv, 1.0);
        }
        if self.texture.is_some() && !b.texture.each_tip {
            m = mask_combine(b.texture.mode, m, self.texture_at(x, y), b.texture.depth.clamp(0.0, 1.0));
        }
        m
    }
}

// ---------------------------------------------------------------------------
// Sparse stroke buffer
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct CovTile {
    cov: Vec<f32>,
    /// Wet Edges: the deepest any dab reaches each pixel (`1 − rn` of the nearest dab), empty
    /// without Wet Edges. The stroke's interior is deep, its rim shallow, whatever the dabs' overlap.
    depth: Vec<f32>,
    /// Premultiplied native colour channels (`nc` per pixel), empty without per-dab colour.
    col: Vec<f32>,
}

/// A sparse tiled coverage (+ optional colour) buffer.
#[derive(Clone, Debug, Default)]
pub struct CoverageMap {
    tiles: HashMap<(i32, i32), CovTile>,
    nc: usize,
    bounds: Rect,
    dirty: HashSet<(i32, i32)>,
}

impl CoverageMap {
    /// Bin a bounded batch to independent tiles. Within each tile dabs retain their original
    /// order; only disjoint pixels run concurrently. No dab-sized float masks are materialised.
    fn raster_batch(&mut self, ctx: &BrushContext, dabs: &[Dab], dual: bool, colors: &[[f32; 8]], accelerator: Option<&dyn CoverageAccelerator>) {
        let mut bins: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for (i, d) in dabs.iter().enumerate() {
            let r = ctx.dab_rect(d, dual);
            self.bounds = self.bounds.union(&r);
            if r.is_empty() {
                continue;
            }
            for ty in r.y0.div_euclid(COV_TILE)..=(r.y1 - 1).div_euclid(COV_TILE) {
                for tx in r.x0.div_euclid(COV_TILE)..=(r.x1 - 1).div_euclid(COV_TILE) {
                    bins.entry((tx, ty)).or_default().push(i);
                }
            }
        }
        if self.nc == 0
            && let Some(accelerator) = accelerator.filter(|a| a.supports(&ctx.brush, dual, dabs, bins.len()))
        {
            let mut tiles: Vec<_> = bins
                .iter()
                .map(|(&(tx, ty), indices)| RasterTile {
                    origin: [tx * COV_TILE, ty * COV_TILE],
                    dabs: indices.clone(),
                    coverage: self.tiles.get(&(tx, ty)).map_or_else(|| vec![0.0; (COV_TILE * COV_TILE) as usize], |t| t.cov.clone()),
                })
                .collect();
            if accelerator.rasterize(&ctx.brush, dual, dabs, &mut tiles) {
                for tile in tiles {
                    let key = (tile.origin[0].div_euclid(COV_TILE), tile.origin[1].div_euclid(COV_TILE));
                    if tile.coverage.iter().any(|v| *v > 0.0) {
                        self.tiles.insert(key, CovTile { cov: tile.coverage, depth: Vec::new(), col: Vec::new() });
                        self.dirty.insert(key);
                    }
                }
                return;
            }
        }
        let jobs: Vec<_> = bins.into_iter().map(|(key, indices)| (key, self.tiles.remove(&key), indices)).collect();
        let nc = self.nc;
        let work = |(key, mut tile, indices): ((i32, i32), Option<CovTile>, Vec<usize>)| {
            let (x0, y0) = (key.0 * COV_TILE, key.1 * COV_TILE);
            let tr = Rect::new(x0, y0, x0.saturating_add(COV_TILE), y0.saturating_add(COV_TILE));
            let mut touched = false;
            for i in indices {
                let Some(d) = dabs.get(i) else { continue };
                let r = tr.intersect(&ctx.dab_rect(d, dual));
                let w = r.width() as usize;
                let color = colors.get(i).and_then(|c| c.get(..nc));
                let want_col = color.is_some() && nc > 0;
                let ceil = if dual { 1.0 } else { d.opacity };
                ctx.rasterize_with(d, dual, r, false, |i, v, _| {
                    if v <= 0.0 || w == 0 {
                        return;
                    }
                    touched = true;
                    let t = tile.get_or_insert_with(|| CovTile { cov: vec![0.0; (COV_TILE * COV_TILE) as usize], depth: Vec::new(), col: Vec::new() });
                    if want_col && t.col.is_empty() {
                        t.col.resize((COV_TILE * COV_TILE) as usize * nc, 0.0);
                    }
                    let ti = ((r.y0 - y0) as usize + i / w) * COV_TILE as usize + (r.x0 - x0) as usize + i % w;
                    let Some(c) = t.cov.get_mut(ti) else { return };
                    let nv = if !dual && ctx.brush.wet_edges {
                        c.max(v * ceil)
                    } else if ceil > *c {
                        *c + v * (ceil - *c)
                    } else {
                        *c
                    };
                    if nv <= *c {
                        return;
                    }
                    if let (true, Some(col)) = (want_col, color) {
                        let k = if *c < 1.0 { (nv - *c) / (1.0 - *c) } else { 0.0 };
                        if let Some(p) = t.col.get_mut(ti * nc..(ti + 1) * nc) {
                            for (p, col) in p.iter_mut().zip(col) {
                                *p = col * k + *p * (1.0 - k);
                            }
                        }
                    }
                    *c = nv;
                });
            }
            (key, tile, touched)
        };
        #[cfg(not(target_arch = "wasm32"))]
        let results: Vec<_> = jobs.into_par_iter().with_min_len(4).map(work).collect();
        #[cfg(target_arch = "wasm32")]
        let results: Vec<_> = jobs.into_iter().map(work).collect();
        for (key, tile, touched) in results {
            if let Some(tile) = tile {
                self.tiles.insert(key, tile);
            }
            if touched {
                self.dirty.insert(key);
            }
        }
    }

    pub fn new(color_channels: usize) -> Self {
        Self { nc: color_channels, ..Default::default() }
    }
    pub fn bounds(&self) -> Rect {
        self.bounds
    }
    #[inline]
    pub fn get(&self, x: i32, y: i32) -> f32 {
        let (tx, ty) = (x.div_euclid(COV_TILE), y.div_euclid(COV_TILE));
        self.tiles.get(&(tx, ty)).map_or(0.0, |t| t.cov[((y - ty * COV_TILE) * COV_TILE + (x - tx * COV_TILE)) as usize])
    }

    /// Wet Edges depth at a pixel (see [`CovTile::depth`]); `None` when the stroke has none.
    #[inline]
    pub fn depth(&self, x: i32, y: i32) -> Option<f32> {
        let (tx, ty) = (x.div_euclid(COV_TILE), y.div_euclid(COV_TILE));
        self.tiles.get(&(tx, ty)).and_then(|t| t.depth.get(((y - ty * COV_TILE) * COV_TILE + (x - tx * COV_TILE)) as usize).copied())
    }

    /// Accumulate dab values over `rect`: flow builds up towards the dab's opacity ceiling
    /// (`c ← c + v·(ceil − c)`). `color` = native colour for per-dab colour. `depth` (Wet Edges, one
    /// value per pixel of `rect`) keeps the deepest any dab reaches each pixel.
    pub fn accumulate(&mut self, rect: Rect, vals: &[f32], ceil: f32, color: Option<&[f32]>, depth: Option<&[f32]>) {
        if rect.is_empty() {
            return;
        }
        self.bounds = self.bounds.union(&rect);
        let w = rect.width() as usize;
        let nc = self.nc;
        let (t0x, t0y) = (rect.x0.div_euclid(COV_TILE), rect.y0.div_euclid(COV_TILE));
        let (t1x, t1y) = ((rect.x1 - 1).div_euclid(COV_TILE), (rect.y1 - 1).div_euclid(COV_TILE));
        let want_col = color.is_some() && nc > 0;
        for ty in t0y..=t1y {
            for tx in t0x..=t1x {
                let tr = Rect::new(tx * COV_TILE, ty * COV_TILE, (tx + 1) * COV_TILE, (ty + 1) * COV_TILE).intersect(&rect);
                if tr.is_empty() {
                    continue;
                }
                // Skip tiles this dab doesn't touch.
                let any = (tr.y0..tr.y1).any(|y| {
                    let row = (y - rect.y0) as usize * w;
                    vals[row + (tr.x0 - rect.x0) as usize..row + (tr.x1 - rect.x0) as usize].iter().any(|&v| v > 0.0)
                });
                if !any {
                    continue;
                }
                let tile = self.tiles.entry((tx, ty)).or_insert_with(|| CovTile {
                    cov: vec![0.0; (COV_TILE * COV_TILE) as usize],
                    depth: Vec::new(),
                    col: if want_col { vec![0.0; (COV_TILE * COV_TILE) as usize * nc] } else { Vec::new() },
                });
                if want_col && tile.col.is_empty() {
                    tile.col = vec![0.0; (COV_TILE * COV_TILE) as usize * nc];
                }
                if depth.is_some() && tile.depth.is_empty() {
                    tile.depth = vec![0.0; (COV_TILE * COV_TILE) as usize];
                }
                self.dirty.insert((tx, ty));
                for y in tr.y0..tr.y1 {
                    let row = (y - rect.y0) as usize * w;
                    let trow = ((y - ty * COV_TILE) * COV_TILE) as usize;
                    for x in tr.x0..tr.x1 {
                        let v = vals[row + (x - rect.x0) as usize];
                        if v <= 0.0 {
                            continue;
                        }
                        let ti = trow + (x - tx * COV_TILE) as usize;
                        if let (Some(dv), Some(slot)) = (depth.and_then(|dp| dp.get(row + (x - rect.x0) as usize)), tile.depth.get_mut(ti)) {
                            *slot = slot.max(*dv);
                        }
                        let c = tile.cov[ti];
                        let nv = if ceil > c { c + v * (ceil - c) } else { c };
                        if nv <= c {
                            continue;
                        }
                        if let (true, Some(col)) = (want_col, color) {
                            let k = if c < 1.0 { (nv - c) / (1.0 - c) } else { 0.0 };
                            let p = &mut tile.col[ti * nc..(ti + 1) * nc];
                            for i in 0..nc {
                                p[i] = col[i] * k + p[i] * (1.0 - k);
                            }
                        }
                        tile.cov[ti] = nv;
                    }
                }
            }
        }
    }

    fn take_dirty(&mut self) -> Vec<(i32, i32)> {
        let mut v: Vec<_> = self.dirty.drain().collect();
        v.sort_unstable();
        v
    }

    /// Union two passes of the same brush stroke without applying opacity twice where they
    /// overlap. The stronger coverage wins, including its per-dab colour when present.
    fn union_max(&mut self, other: &Self) {
        self.bounds = self.bounds.union(&other.bounds);
        for (&key, source) in &other.tiles {
            let Some(target) = self.tiles.get_mut(&key) else {
                self.tiles.insert(key, source.clone());
                self.dirty.insert(key);
                continue;
            };
            if target.depth.is_empty() {
                target.depth.clone_from(&source.depth);
            } else {
                for (dst, src) in target.depth.iter_mut().zip(&source.depth) {
                    *dst = dst.max(*src);
                }
            }
            for (index, (dst, src)) in target.cov.iter_mut().zip(&source.cov).enumerate() {
                if *src > *dst {
                    *dst = *src;
                    let start = index.saturating_mul(self.nc);
                    let end = start.saturating_add(self.nc);
                    if let (Some(dst_color), Some(src_color)) = (target.col.get_mut(start..end), source.col.get(start..end)) {
                        dst_color.copy_from_slice(src_color);
                    }
                }
            }
            self.dirty.insert(key);
        }
    }
}

// ---------------------------------------------------------------------------
// Stroke renderer
// ---------------------------------------------------------------------------

/// Incremental stroke rendering: feed points, composite dirty tiles from the pre-stroke pixels.
#[derive(Clone, Debug)]
pub struct StrokeRenderer {
    accelerator: Option<Arc<dyn CoverageAccelerator>>,
    pub ctx: BrushContext,
    generator: DabGenerator,
    cov: CoverageMap,
    dual: Option<CoverageMap>,
    fmt: Option<PixelFormat>,
    per_dab_color: bool,
    dabs_done: usize,
    scratch: Vec<f32>,
    /// Wet Edges depth of the dab being accumulated (see [`CovTile::depth`]).
    depth_scratch: Vec<f32>,
    dab_buf: Vec<Dab>,
    dual_buf: Vec<Dab>,
    all_dabs: Option<Vec<Dab>>,
}

impl StrokeRenderer {
    /// Composite the union of this stroke and a mirrored pass as one stroke. This keeps
    /// overlapping dabs on a symmetry axis under one opacity ceiling at every bit depth.
    pub fn composite_union(&self, other: &Self, pre: &Surface, target: &mut Surface, selection: Option<&Surface>, lock_transparency: bool) -> Rect {
        self.composite_union_many(std::iter::once(other), pre, target, selection, lock_transparency)
    }

    /// Combine all symmetry passes before applying brush opacity and selection once.
    pub fn composite_union_many<'a>(
        &self,
        others: impl IntoIterator<Item = &'a Self>,
        pre: &Surface,
        target: &mut Surface,
        selection: Option<&Surface>,
        lock_transparency: bool,
    ) -> Rect {
        let mut merged = self.clone();
        for other in others {
            merged.cov.union_max(&other.cov);
            if let (Some(to), Some(from)) = (&mut merged.dual, &other.dual) {
                to.union_max(from);
            }
        }
        merged.composite(pre, target, selection, lock_transparency, true)
    }
    /// `fmt` = target pixel format (needed for per-dab colour); `zoom` for smoothing.
    pub fn new(brush: &BrushSettings, fmt: Option<PixelFormat>, zoom: f32) -> Self {
        let per_dab_color = brush.color_dynamics.enabled && !brush.erase && fmt.is_some();
        let nc = if per_dab_color { fmt.map_or(0, |f| f.mode.color_channels()) } else { 0 };
        Self {
            accelerator: None,
            ctx: BrushContext::new(brush),
            generator: DabGenerator::new(brush, zoom),
            cov: CoverageMap::new(nc),
            dual: brush.dual_brush.enabled.then(|| CoverageMap::new(0)),
            fmt,
            per_dab_color,
            dabs_done: 0,
            scratch: Vec::new(),
            depth_scratch: Vec::new(),
            dab_buf: Vec::new(),
            dual_buf: Vec::new(),
            all_dabs: None,
        }
    }

    /// Use an optional hardware coverage backend; unsupported batches keep the CPU oracle.
    pub fn with_accelerator(mut self, accelerator: Option<Arc<dyn CoverageAccelerator>>) -> Self {
        self.accelerator = accelerator;
        self
    }

    /// Keep a copy of every primary dab (for tests and sequential tools).
    pub fn record_dabs(mut self) -> Self {
        self.all_dabs = Some(Vec::new());
        self
    }
    pub fn dabs(&self) -> &[Dab] {
        self.all_dabs.as_deref().unwrap_or(&[])
    }
    pub fn dab_count(&self) -> usize {
        self.dabs_done
    }
    /// Union of the dab rectangles so far.
    pub fn bounds(&self) -> Rect {
        self.cov.bounds
    }

    fn raster_pending(&mut self) {
        let mut dabs = std::mem::take(&mut self.dab_buf);
        let mut duals = std::mem::take(&mut self.dual_buf);
        // Small tips are faster without scheduling/binning. Large tips use bounded batches so
        // queue metadata never grows with the complete stroke's pixel area.
        let large = !self.ctx.brush.wet_edges && dabs.iter().chain(&duals).any(|d| d.radius >= 128.0);
        if large {
            for batch in dabs.chunks(64) {
                // Colour conversion uses the caller's ICC scope, before crossing worker threads.
                let colors: Vec<_> = if self.per_dab_color {
                    batch
                        .iter()
                        .map(|d| {
                            let mut native = [0.0; 8];
                            if let Some(f) = self.fmt {
                                from_rgba_into(&f, d.color, &mut native);
                            }
                            native
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                self.cov.raster_batch(&self.ctx, batch, false, &colors, self.accelerator.as_deref());
            }
            if let Some(dm) = self.dual.as_mut() {
                for batch in duals.chunks(64) {
                    dm.raster_batch(&self.ctx, batch, true, &[], self.accelerator.as_deref());
                }
            }
        } else {
            self.raster_serial(&dabs, &duals);
        }
        self.dabs_done += dabs.len();
        if let Some(all) = self.all_dabs.as_mut() {
            all.extend_from_slice(&dabs);
        }
        dabs.clear();
        duals.clear();
        self.dab_buf = dabs;
        self.dual_buf = duals;
    }

    fn raster_serial(&mut self, dabs: &[Dab], duals: &[Dab]) {
        let wet = self.ctx.brush.wet_edges;
        let mut native = [0.0f32; 8];
        for d in dabs {
            let rect = self.ctx.dab_rect(d, false);
            // Wet Edges acts on the whole stroke (#2088): the dab records how deep it reaches each
            // pixel and the stroke darkens its own rim when composited, not every dab's.
            self.ctx.rasterize_into(d, false, rect, &mut self.scratch, wet.then_some(&mut self.depth_scratch));
            let col = match (self.per_dab_color, self.fmt) {
                (true, Some(f)) => {
                    from_rgba_into(&f, d.color, &mut native);
                    Some(&native[..f.mode.color_channels()])
                }
                _ => None,
            };
            self.cov.accumulate(rect, &self.scratch, d.opacity, col, wet.then_some(&self.depth_scratch[..]));
            // Bounds track the full dab rectangle (even fully transparent parts), like the damage.
            self.cov.bounds = self.cov.bounds.union(&rect);
        }
        if let Some(dm) = self.dual.as_mut() {
            for d in duals {
                let rect = self.ctx.dab_rect(d, true);
                self.ctx.rasterize(d, true, rect, &mut self.scratch);
                dm.accumulate(rect, &self.scratch, 1.0, None, None);
            }
        }
    }

    /// See [`DabGenerator::wants_time`].
    pub fn wants_time(&self) -> bool {
        self.generator.wants_time()
    }

    /// Feed input points (any chunking gives the same result).
    pub fn push(&mut self, pts: &[StrokePoint]) {
        self.generator.push(pts, &mut self.dab_buf, &mut self.dual_buf);
        self.raster_pending();
    }

    /// End of stroke (smoothing catch-up, a lone first point).
    pub fn finish(&mut self) {
        self.generator.finish(&mut self.dab_buf, &mut self.dual_buf);
        self.raster_pending();
    }

    /// What finishing the stroke now would add (the smoothing catch-up tail to the last point, or
    /// a lone first dab), for live previews: a renderer holding copies of the coverage tiles the
    /// tail touches with the tail rendered in, so its `composite` draws exactly those tiles as
    /// [`finish`](Self::finish) would leave them. `None` when finishing adds nothing.
    pub fn tail_preview(&self) -> Option<StrokeRenderer> {
        let mut generator = self.generator.clone();
        let (mut dabs, mut duals) = (Vec::new(), Vec::new());
        generator.finish(&mut dabs, &mut duals);
        if dabs.is_empty() && duals.is_empty() {
            return None;
        }
        let mut keys = HashSet::new();
        for r in dabs.iter().map(|d| self.ctx.dab_rect(d, false)).chain(duals.iter().map(|d| self.ctx.dab_rect(d, true))) {
            if r.is_empty() {
                continue;
            }
            for ty in r.y0.div_euclid(COV_TILE)..=(r.y1 - 1).div_euclid(COV_TILE) {
                for tx in r.x0.div_euclid(COV_TILE)..=(r.x1 - 1).div_euclid(COV_TILE) {
                    keys.insert((tx, ty));
                }
            }
        }
        let subset = |m: &CoverageMap| CoverageMap {
            tiles: keys.iter().filter_map(|k| m.tiles.get(k).map(|t| (*k, t.clone()))).collect(),
            nc: m.nc,
            bounds: m.bounds,
            dirty: HashSet::new(),
        };
        let mut t = StrokeRenderer {
            accelerator: self.accelerator.clone(),
            ctx: self.ctx.clone(),
            generator,
            cov: subset(&self.cov),
            dual: self.dual.as_ref().map(subset),
            fmt: self.fmt,
            per_dab_color: self.per_dab_color,
            dabs_done: self.dabs_done,
            scratch: Vec::new(),
            depth_scratch: Vec::new(),
            dab_buf: dabs,
            dual_buf: duals,
            all_dabs: None,
        };
        t.raster_pending();
        // Tiles whose coverage the tail doesn't change still composite the same: redraw them all.
        t.cov.dirty.extend(t.cov.tiles.keys().copied());
        Some(t)
    }

    /// Mark the coverage tiles over `r` for the next `composite` (e.g. to redraw over a preview).
    pub fn mark_dirty(&mut self, r: Rect) {
        if r.is_empty() {
            return;
        }
        for ty in r.y0.div_euclid(COV_TILE)..=(r.y1 - 1).div_euclid(COV_TILE) {
            for tx in r.x0.div_euclid(COV_TILE)..=(r.x1 - 1).div_euclid(COV_TILE) {
                if self.cov.tiles.contains_key(&(tx, ty)) {
                    self.cov.dirty.insert((tx, ty));
                }
            }
        }
    }

    /// The coverage tiles touched since the last call (or `composite`), as one rectangle clipped to
    /// the stroke bounds, and forget them: what a live preview that composites its own paint has
    /// to redraw.
    pub fn take_dirty_rect(&mut self) -> Rect {
        let r = self
            .cov
            .take_dirty()
            .into_iter()
            .fold(Rect::EMPTY, |acc, (tx, ty)| acc.union(&Rect::new(tx * COV_TILE, ty * COV_TILE, (tx + 1) * COV_TILE, (ty + 1) * COV_TILE)));
        if let Some(d) = self.dual.as_mut() {
            d.dirty.clear();
        }
        r.intersect(&self.bounds())
    }

    /// Final coverage over `r`, one value per pixel in rows (as [`dense_coverage`](Self::dense_coverage)
    /// over its bounds).
    pub fn coverage_in(&self, r: Rect) -> Vec<f32> {
        (r.y0..r.y1).flat_map(|y| (r.x0..r.x1).map(move |x| (x, y))).map(|(x, y)| self.coverage_at(x, y)).collect()
    }

    /// Final stroke coverage at a pixel (stroke-level masks applied, before opacity/selection).
    pub fn coverage_at(&self, x: i32, y: i32) -> f32 {
        let c = self.cov.get(x, y);
        if c <= 0.0 {
            return 0.0;
        }
        self.ctx.stroke_mask_wet(c, self.cov.depth(x, y), self.dual.as_ref().map(|d| d.get(x, y)), x, y)
    }

    /// Dense final coverage over the stroke bounds.
    pub fn dense_coverage(&self) -> (Rect, Vec<f32>) {
        let b = self.bounds();
        let w = b.width() as usize;
        let mut out = vec![0.0f32; w * b.height() as usize];
        for (&(tx, ty), t) in &self.cov.tiles {
            let tr = Rect::new(tx * COV_TILE, ty * COV_TILE, (tx + 1) * COV_TILE, (ty + 1) * COV_TILE).intersect(&b);
            for y in tr.y0..tr.y1 {
                for x in tr.x0..tr.x1 {
                    let ti = ((y - ty * COV_TILE) * COV_TILE + (x - tx * COV_TILE)) as usize;
                    let c = t.cov[ti];
                    if c > 0.0 {
                        out[(y - b.y0) as usize * w + (x - b.x0) as usize] =
                            self.ctx.stroke_mask_wet(c, t.depth.get(ti).copied(), self.dual.as_ref().map(|d| d.get(x, y)), x, y);
                    }
                }
            }
        }
        (b, out)
    }

    /// Composite the stroke buffer onto `target`, reading original pixels from `pre` (the
    /// pre-stroke surface; pass a clone of the target taken before the first `push`). With
    /// `all = false` only tiles touched since the last composite are processed (interactive use).
    /// Returns the damaged rectangle.
    pub fn composite(&mut self, pre: &Surface, target: &mut Surface, selection: Option<&Surface>, lock_transparency: bool, all: bool) -> Rect {
        let keys: Vec<(i32, i32)> = if all {
            self.cov.dirty.clear();
            let mut k: Vec<_> = self.cov.tiles.keys().copied().collect();
            k.sort_unstable();
            k
        } else {
            self.cov.take_dirty()
        };
        if let Some(d) = self.dual.as_mut() {
            d.dirty.clear();
        }
        let bounds = self.bounds();
        let fmt = target.format();
        let n = fmt.channels();
        let a_idx = alpha_index(&fmt);
        let nc = fmt.mode.color_channels();
        let b = &self.ctx.brush;
        let opacity = b.opacity.clamp(0.0, 1.0);
        let mut src = [0.0f32; 8];
        from_rgba_into(&fmt, b.color, &mut src);
        if let Some(a) = a_idx {
            src[a] = b.color[3];
        }
        let initial_src = src;
        let cmyk = photocraft_color::convert::active_cmyk_space();
        let work = |(tx, ty): (i32, i32)| {
            photocraft_color::convert::with_cmyk_space(cmyk.as_ref(), || {
                let mut src = initial_src;
                let mut region = Vec::new();
                let tile = self.cov.tiles.get(&(tx, ty))?;
                let tr = Rect::new(tx * COV_TILE, ty * COV_TILE, (tx + 1) * COV_TILE, (ty + 1) * COV_TILE).intersect(&bounds);
                if tr.is_empty() {
                    return None;
                }
                pre.read_region_into(tr, &mut region);
                let sel = selection.map(|s| (s.channels(), s.read_region(tr)));
                let w = tr.width() as usize;
                for y in tr.y0..tr.y1 {
                    for x in tr.x0..tr.x1 {
                        let ti = ((y - ty * COV_TILE) * COV_TILE + (x - tx * COV_TILE)) as usize;
                        let c = tile.cov[ti];
                        if c <= 0.0 {
                            continue;
                        }
                        let i = (y - tr.y0) as usize * w + (x - tr.x0) as usize;
                        let m = self.ctx.stroke_mask_wet(c, tile.depth.get(ti).copied(), self.dual.as_ref().map(|d| d.get(x, y)), x, y);
                        let s = sel.as_ref().map_or(1.0, |(sc, v)| v[i * sc]);
                        let mut k = (m * opacity * s).min(1.0);
                        if k <= 0.0 {
                            continue;
                        }
                        if b.mode == BlendMode::Dissolve {
                            k = if hash2(x, y, b.seed) < k { 1.0 } else { 0.0 };
                            if k == 0.0 {
                                continue;
                            }
                        }
                        let px = &mut region[i * n..(i + 1) * n];
                        if b.erase {
                            if let (Some(a), false) = (a_idx, lock_transparency) {
                                px[a] *= 1.0 - k;
                            }
                            continue;
                        }
                        if lock_transparency && a_idx.is_some_and(|a| px[a] <= 0.0) {
                            continue;
                        }
                        if self.per_dab_color && !tile.col.is_empty() {
                            let p = &tile.col[ti * nc..(ti + 1) * nc];
                            for ch in 0..nc {
                                src[ch] = p[ch] / c;
                            }
                        }
                        if matches!(b.mode, BlendMode::Normal | BlendMode::Dissolve) {
                            over_native(&fmt, px, &src[..n], k, lock_transparency);
                        } else {
                            let d = to_rgba(&fmt, px);
                            let sr = to_rgba(&fmt, &src[..n]);
                            let mut o = photocraft_color::blend::composite(b.mode, d, sr, k);
                            if lock_transparency {
                                o[3] = d[3];
                            }
                            let mut enc = [0.0f32; 8];
                            from_rgba_into(&fmt, o, &mut enc);
                            px.copy_from_slice(&enc[..n]);
                        }
                    }
                }
                // Encode on the worker too; publishing is then a row copy, rather than a serial
                // per-sample conversion over every dirty pixel on the input/UI thread.
                let mut bytes = vec![0; region.len() * fmt.sample.bytes()];
                for (i, value) in region.into_iter().enumerate() {
                    photocraft_color::write_sample(&mut bytes, fmt.sample, i, value);
                }
                Some((tr, bytes))
            })
        };
        let mut dmg = Rect::EMPTY;
        // Bounded output batches prevent a long stroke from allocating another full float
        // image. Small edits avoid Rayon scheduling; large edits compose disjoint tiles.
        for batch in keys.chunks(64) {
            #[cfg(not(target_arch = "wasm32"))]
            let results: Vec<_> =
                if keys.len() >= 64 { batch.par_iter().copied().filter_map(work).collect() } else { batch.iter().copied().filter_map(work).collect() };
            #[cfg(target_arch = "wasm32")]
            let results: Vec<_> = batch.iter().copied().filter_map(work).collect();
            for (tr, region) in results {
                target.write_interleaved(tr, &region);
                dmg = dmg.union(&tr);
            }
        }
        if all { bounds } else { dmg }
    }
}

/// Render a whole stroke onto `target` (one-shot). Returns the damaged rectangle.
pub fn render_stroke(
    target: &mut Surface,
    brush: &BrushSettings,
    points: &[StrokePoint],
    selection: Option<&Surface>,
    lock_transparency: bool,
    zoom: f32,
) -> Rect {
    if points.is_empty() {
        return Rect::EMPTY;
    }
    let pre = target.clone();
    let mut r = StrokeRenderer::new(brush, Some(target.format()), zoom);
    r.push(points);
    r.finish();
    r.composite(&pre, target, selection, lock_transparency, true)
}

#[cfg(test)]
mod tiled_tests {
    use super::*;
    use crate::{ColorDynamics, DualBrush, Dynamic, GrayTile, ShapeDynamics, Texture, TipShape};
    use photocraft_color::SampleType;

    fn compare(brush: &BrushSettings, fmt: PixelFormat, points: &[StrokePoint]) {
        let mut serial = StrokeRenderer::new(brush, Some(fmt), 1.0);
        let mut tiled = StrokeRenderer::new(brush, Some(fmt), 1.0);
        let (mut dabs, mut duals) = (Vec::new(), Vec::new());
        serial.generator.push(points, &mut dabs, &mut duals);
        serial.generator.finish(&mut dabs, &mut duals);
        serial.raster_serial(&dabs, &duals);
        tiled.push(points);
        tiled.finish();
        assert_eq!(serial.bounds(), tiled.bounds());
        assert_eq!(serial.cov.tiles.len(), tiled.cov.tiles.len());
        for (key, a) in &serial.cov.tiles {
            let b = &tiled.cov.tiles[key];
            assert_eq!(a.cov, b.cov, "coverage {key:?}");
            assert_eq!(a.col, b.col, "colour {key:?}");
        }
        let pre = Surface::with_default(fmt, &from_rgba_into_test(fmt, [0.2, 0.3, 0.7, 0.8]));
        let mut a = pre.clone();
        let mut b = pre.clone();
        serial.composite(&pre, &mut a, None, false, true);
        tiled.composite(&pre, &mut b, None, false, true);
        assert_eq!(a, b, "native pixels {fmt:?}");
    }

    fn from_rgba_into_test(fmt: PixelFormat, rgba: [f32; 4]) -> Vec<f32> {
        let mut out = vec![0.0; fmt.channels()];
        from_rgba_into(&fmt, rgba, &mut out);
        out
    }

    #[test]
    fn tiled_large_dabs_are_bit_identical_to_serial_at_every_depth() {
        let pts = [StrokePoint::new(-73.25, -40.75, 1.0), StrokePoint::new(250.125, 170.375, 0.7)];
        let base = BrushSettings { size: 350.0, pressure_size: false, spacing: 0.23, flow: 0.31, opacity: 0.57, seed: 37, ..Default::default() };
        let sampled = TipShape::Sampled(GrayTile::from_fn(53, 31, |x, y| ((x * 7 + y * 3) % 23) as f32 / 23.0));
        let brushes = [
            base.clone(),
            BrushSettings { hardness: 1.0, roundness: 0.37, angle: 43.0, noise: true, wet_edges: true, ..base.clone() },
            BrushSettings { tip: sampled.clone(), texture: Texture { enabled: true, each_tip: true, scale: 0.7, ..Default::default() }, ..base.clone() },
            BrushSettings {
                dual_brush: DualBrush { enabled: true, size: 290.0, tip: sampled, ..Default::default() },
                color_dynamics: ColorDynamics { enabled: true, hue_jitter: 0.7, brightness_jitter: 0.6, fg_bg: Dynamic::jitter(0.8), ..Default::default() },
                shape_dynamics: ShapeDynamics { enabled: true, angle: Dynamic::jitter(0.7), ..Default::default() },
                ..base.clone()
            },
            BrushSettings { aliased: true, erase: true, ..base },
        ];
        for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA8.with_sample(SampleType::U16), PixelFormat::RGBA32F, PixelFormat::CMYKA8] {
            for brush in &brushes {
                compare(brush, fmt, &pts);
            }
        }
    }

    #[test]
    fn tiled_updates_keep_coverage_and_colour_order_across_batches() {
        let brush = BrushSettings {
            size: 300.0,
            pressure_size: false,
            spacing: 0.01,
            flow: 0.13,
            color_dynamics: ColorDynamics { enabled: true, hue_jitter: 0.7, brightness_jitter: 0.6, fg_bg: Dynamic::jitter(0.8), ..Default::default() },
            ..Default::default()
        };
        compare(&brush, PixelFormat::RGBA8, &[StrokePoint::new(0.25, 0.75, 1.0), StrokePoint::new(220.5, 20.25, 1.0)]);
    }

    #[test]
    #[ignore = "release performance comparison"]
    fn thousand_pixel_brush_release_comparison() {
        let points: Vec<_> = (0..40).map(|i| StrokePoint::new(10_000.25 + f64::from(i) * 30.0, 10_000.75, 1.0)).collect();
        for textured in [false, true] {
            let brush = BrushSettings {
                size: 1000.0,
                pressure_size: false,
                spacing: 0.05,
                flow: 0.3,
                texture: Texture { enabled: textured, each_tip: textured, ..Default::default() },
                ..Default::default()
            };
            let ctx = BrushContext::new(&brush);
            let mut generator = DabGenerator::new(&brush, 1.0);
            let (mut dabs, mut duals) = (Vec::new(), Vec::new());
            generator.push(&points, &mut dabs, &mut duals);
            generator.finish(&mut dabs, &mut duals);
            let mut serial = StrokeRenderer::new(&brush, Some(PixelFormat::RGBA8), 1.0);
            let start = std::time::Instant::now();
            serial.raster_serial(&dabs, &duals);
            let before = start.elapsed();
            let mut tiled = CoverageMap::new(0);
            let start = std::time::Instant::now();
            tiled.raster_batch(&ctx, &dabs, false, &[], None);
            let after = start.elapsed();
            for (key, tile) in &serial.cov.tiles {
                assert_eq!(tile.cov, tiled.tiles[key].cov);
            }
            println!(
                "1000px textured={textured} dabs={} serial={:.3}ms tiled={:.3}ms speedup={:.2}x",
                dabs.len(),
                before.as_secs_f64() * 1000.0,
                after.as_secs_f64() * 1000.0,
                before.as_secs_f64() / after.as_secs_f64()
            );
        }
    }
}
