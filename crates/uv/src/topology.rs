//! UV shell connectivity and edge protection masks.

use crate::{Error, MAX_PIXELS, Result, Scene, filled};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

const MAX_OCCUPANCY_PIXELS: usize = 16_384 * 16_384;
const MAX_SHELL_ID_PIXELS: usize = 64 * 1024 * 1024;
const MAX_PADDING_CANDIDATES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Shell {
    pub material: usize,
    pub triangles: Vec<usize>,
    pub neighbors: BTreeSet<usize>,
}

#[derive(Clone, Debug)]
pub struct Boundary {
    pub shell: usize,
    pub uv: [[f64; 2]; 2],
    pub shared: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Topology {
    pub shells: Vec<Shell>,
    pub triangle_shell: Vec<usize>,
    pub boundaries: Vec<Boundary>,
    pub non_manifold_edges: usize,
}

/// Compact atlas occupancy for collision checks. It is built once for a scene/material/texture
/// size and reused by shell selection and wrapped painting.
#[derive(Clone, Debug)]
pub struct OccupancyMask {
    width: u32,
    height: u32,
    material: usize,
    tile: [i32; 2],
    bits: Vec<u8>,
}
impl OccupancyMask {
    fn insert(&mut self, x: usize, y: usize) {
        let width = self.width as usize;
        let Some(index) = y.checked_mul(width).and_then(|v| v.checked_add(x)) else { return };
        if let Some(byte) = self.bits.get_mut(index / 8) {
            *byte |= 1 << (index % 8);
        }
    }
    pub fn contains(&self, x: i32, y: i32) -> bool {
        if x < 0 || y < 0 || x >= i32::try_from(self.width).unwrap_or(i32::MAX) || y >= i32::try_from(self.height).unwrap_or(i32::MAX) {
            return false;
        }
        let width = self.width as usize;
        let Some(index) = (y as usize).checked_mul(width).and_then(|v| v.checked_add(x as usize)) else { return false };
        self.bits.get(index / 8).is_some_and(|byte| byte & (1 << (index % 8)) != 0)
    }
}

/// Sparse outward UV padding owned by a single shell. A row of 64 bits is stored only for atlas
/// tiles touched by that shell's boundary, so a narrow apron does not allocate a full atlas mask.
#[derive(Clone, Debug, Default)]
pub struct PaddingMask {
    tiles: BTreeMap<(i32, i32), Vec<u64>>,
}
impl PaddingMask {
    fn insert(&mut self, x: i32, y: i32) {
        if x < 0 || y < 0 {
            return;
        }
        let key = (x.div_euclid(64), y.div_euclid(64));
        let row = y.rem_euclid(64) as usize;
        let bit = 1u64 << x.rem_euclid(64);
        let rows = self.tiles.entry(key).or_insert_with(|| vec![0; 64]);
        if let Some(value) = rows.get_mut(row) {
            *value |= bit;
        }
    }
    pub fn contains(&self, x: i32, y: i32) -> bool {
        if x < 0 || y < 0 {
            return false;
        }
        self.tiles
            .get(&(x.div_euclid(64), y.div_euclid(64)))
            .and_then(|rows| rows.get(y.rem_euclid(64) as usize))
            .is_some_and(|row| row & (1u64 << x.rem_euclid(64)) != 0)
    }
    pub fn for_each(&self, mut visit: impl FnMut(i32, i32)) {
        for ((tile_x, tile_y), rows) in &self.tiles {
            for (row, bits) in rows.iter().copied().enumerate() {
                let mut remaining = bits;
                while remaining != 0 {
                    let column = remaining.trailing_zeros() as i32;
                    visit(tile_x * 64 + column, tile_y * 64 + row as i32);
                    remaining &= remaining - 1;
                }
            }
        }
    }
    pub fn estimated_bytes(&self) -> usize {
        self.tiles.len().saturating_mul(64 * std::mem::size_of::<u64>() + 48)
    }
}

#[derive(Clone, Copy)]
struct HalfEdge {
    triangle: usize,
    uv: [[f64; 2]; 2],
}

fn root(parents: &[usize], mut i: usize) -> usize {
    while let Some(&p) = parents.get(i) {
        if i == p {
            break;
        }
        i = p;
    }
    i
}

fn continuous(a: &HalfEdge, b: &HalfEdge) -> bool {
    a.uv.iter().flatten().zip(b.uv.iter().flatten()).all(|(x, y)| (x - y).abs() < 1e-7)
}

impl Topology {
    pub fn build(scene: &Scene) -> Result<Self> {
        scene.validate()?;
        let mut parents: Vec<usize> = (0..scene.triangles.len()).collect();
        let mut sizes = filled(scene.triangles.len(), 1usize)?;
        let mut edges: BTreeMap<(usize, usize), Vec<HalfEdge>> = BTreeMap::new();
        for (i, t) in scene.triangles.iter().enumerate() {
            for (a, b) in [(0, 1), (1, 2), (2, 0)] {
                let (Some(a), Some(b)) = (t.corners.get(a), t.corners.get(b)) else { continue };
                if a.vertex == b.vertex {
                    continue;
                }
                let (key, uv) = if a.vertex < b.vertex { ((a.vertex, b.vertex), [a.uv, b.uv]) } else { ((b.vertex, a.vertex), [b.uv, a.uv]) };
                edges.entry(key).or_default().push(HalfEdge { triangle: i, uv });
            }
        }
        for list in edges.values() {
            if let [a, b] = list.as_slice()
                && continuous(a, b)
                && scene.triangles.get(a.triangle).map(|t| t.material) == scene.triangles.get(b.triangle).map(|t| t.material)
            {
                let (mut a, mut b) = (root(&parents, a.triangle), root(&parents, b.triangle));
                if a == b {
                    continue;
                }
                if sizes.get(a) < sizes.get(b) {
                    std::mem::swap(&mut a, &mut b);
                }
                let total = sizes.get(a).copied().unwrap_or(1).saturating_add(sizes.get(b).copied().unwrap_or(1));
                if let Some(p) = parents.get_mut(b) {
                    *p = a;
                }
                if let Some(size) = sizes.get_mut(a) {
                    *size = total;
                }
            }
        }
        let mut result = Self::default();
        let mut roots = BTreeMap::new();
        for (i, t) in scene.triangles.iter().enumerate() {
            let r = root(&parents, i);
            let shell = *roots.entry(r).or_insert_with(|| {
                let id = result.shells.len();
                result.shells.push(Shell { material: t.material, triangles: Vec::new(), neighbors: BTreeSet::new() });
                id
            });
            result.triangle_shell.push(shell);
            if let Some(s) = result.shells.get_mut(shell) {
                s.triangles.push(i);
            }
        }
        for list in edges.values() {
            if list.len() > 2 {
                result.non_manifold_edges += 1;
            }
            let same_uv = matches!(list.as_slice(), [a,b] if continuous(a,b));
            for a in list {
                let Some(&shell) = result.triangle_shell.get(a.triangle) else { continue };
                let peers: BTreeSet<usize> = list.iter().filter_map(|b| result.triangle_shell.get(b.triangle).copied()).filter(|s| *s != shell).collect();
                if let Some(s) = result.shells.get_mut(shell) {
                    s.neighbors.extend(peers.iter().copied());
                }
                if list.len() != 2 || !same_uv || !peers.is_empty() {
                    result.boundaries.push(Boundary { shell, uv: a.uv, shared: list.len() > 1 });
                }
            }
        }
        Ok(result)
    }

    /// All shells under a UV point, so stacked/mirrored UVs are never silently ambiguous.
    pub fn pick_uv(&self, scene: &Scene, uv: [f64; 2], material: usize) -> Vec<usize> {
        let mut shells = BTreeSet::new();
        for (i, t) in scene.triangles.iter().enumerate().filter(|(_, t)| t.material == material) {
            if barycentric(uv, t.corners.map(|c| c.uv)).is_some()
                && let Some(&s) = self.triangle_shell.get(i)
            {
                shells.insert(s);
            }
        }
        shells.into_iter().collect()
    }

    /// Rasterize the union first, then remove protected bands. Other selected islands cannot
    /// re-enable a protected seam. Distances use the actual width AND height of the texture.
    pub fn mask(&self, scene: &Scene, options: &MaskOptions) -> Result<Vec<f32>> {
        let occupancy = self.occupancy_mask(scene, options.width, options.height, options.material, options.tile)?;
        self.mask_with_occupancy(scene, options, &occupancy)
    }

    /// Rasterize all UV coverage for one material. Callers that repeatedly evaluate masks or
    /// wrapped brush stamps should cache this by scene, material, tile and texture dimensions.
    pub fn occupancy_mask(&self, scene: &Scene, width: u32, height: u32, material: usize, tile: [i32; 2]) -> Result<OccupancyMask> {
        scene.validate()?;
        let (w, h) = (width as usize, height as usize);
        let n = w
            .checked_mul(h)
            .filter(|n| *n > 0 && *n <= MAX_OCCUPANCY_PIXELS && width <= 16_384 && height <= 16_384)
            .ok_or_else(|| Error("UV occupancy is limited to 16384 by 16384 pixels".into()))?;
        if material >= scene.materials.len() {
            return Err(Error("Material is out of range".into()));
        }
        let mut occupancy = OccupancyMask { width, height, material, tile, bits: filled(n.div_ceil(8), 0u8)? };
        let point = |uv: [f64; 2]| [(uv[0] - f64::from(tile[0])) * w as f64, (1.0 - (uv[1] - f64::from(tile[1]))) * h as f64];
        for triangle in scene.triangles.iter().filter(|t| t.material == material) {
            let pts = triangle.corners.map(|c| point(c.uv));
            let (x0, y0, x1, y1) = bounds(&pts, 0.0, w, h);
            for y in y0..y1 {
                for x in x0..x1 {
                    let covered = [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)]
                        .iter()
                        .any(|(dx, dy)| barycentric([x as f64 + dx, y as f64 + dy], pts).is_some());
                    if covered {
                        occupancy.insert(x, y);
                    }
                }
            }
        }
        Ok(occupancy)
    }

    /// Rasterize shell ownership into a row-major per-pixel ID grid. Zero means off-shell;
    /// occupied pixels contain `shell_id + 1`. UVs use the same four subpixel samples and
    /// bottom-left to top-left conversion as [`Self::occupancy_mask`]. If shells overlap in UV
    /// space, the smallest assigned shell ID wins, independent of raster iteration order.
    pub fn shell_id_map(&self, scene: &Scene, width: u32, height: u32, material: usize, tile: [i32; 2]) -> Result<Vec<u32>> {
        self.shell_id_map_for_materials(scene, width, height, &[material], tile)
    }

    /// Rasterize shell ownership for several materials bound to one texture document. IDs stay
    /// global to this topology, so blur samples from separate material shells remain isolated.
    pub fn shell_id_map_for_materials(&self, scene: &Scene, width: u32, height: u32, materials: &[usize], tile: [i32; 2]) -> Result<Vec<u32>> {
        scene.validate()?;
        if materials.is_empty() || materials.iter().any(|material| *material >= scene.materials.len()) {
            return Err(Error("Material is out of range".into()));
        }
        let (w, h) = (width as usize, height as usize);
        let n = w
            .checked_mul(h)
            .filter(|n| *n > 0 && *n <= MAX_SHELL_ID_PIXELS && width <= 16_384 && height <= 16_384)
            .ok_or_else(|| Error("UV shell map exceeds its 64M-pixel allocation limit".into()))?;
        if self.triangle_shell.len() != scene.triangles.len() {
            return Err(Error("UV topology does not match the scene".into()));
        }
        let mut ids = filled(n, 0u32)?;
        let point = |uv: [f64; 2]| [(uv[0] - f64::from(tile[0])) * w as f64, (1.0 - (uv[1] - f64::from(tile[1]))) * h as f64];
        for (triangle_id, triangle) in scene.triangles.iter().enumerate().filter(|(_, triangle)| materials.contains(&triangle.material)) {
            let shell_id = self.triangle_shell.get(triangle_id).copied().ok_or_else(|| Error("UV topology does not match the scene".into()))?;
            if self.shells.get(shell_id).is_none_or(|shell| shell.material != triangle.material) {
                return Err(Error("UV topology does not match the scene material".into()));
            }
            let shell = u32::try_from(shell_id).ok().and_then(|id| id.checked_add(1)).ok_or_else(|| Error("UV shell ID exceeds the raster format".into()))?;
            let pts = triangle.corners.map(|corner| point(corner.uv));
            let (x0, y0, x1, y1) = bounds(&pts, 0.0, w, h);
            for y in y0..y1 {
                for x in x0..x1 {
                    let covered = [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)]
                        .iter()
                        .any(|(dx, dy)| barycentric([x as f64 + dx, y as f64 + dy], pts).is_some());
                    if covered {
                        let index = y * w + x;
                        if let Some(current) = ids.get_mut(index)
                            && (*current == 0 || shell < *current)
                        {
                            *current = shell;
                        }
                    }
                }
            }
        }
        Ok(ids)
    }

    /// Build a sparse exterior apron for a single shell. The apron follows only true shell
    /// boundaries (never internal triangulation edges), stays inside the atlas, and excludes
    /// every texel covered by another shell of the same material.
    pub fn outward_padding_mask(&self, options: &MaskOptions, shell_id: usize, occupancy: &OccupancyMask) -> Result<PaddingMask> {
        let MaskOptions { width, height, material, tile, padding, .. } = options;
        if !padding.is_finite() || !(0.0..=1024.0).contains(padding) {
            return Err(Error("Outward padding must be in 0..1024 pixels".into()));
        }
        if *padding == 0.0 {
            return Ok(PaddingMask::default());
        }
        if (occupancy.width, occupancy.height, occupancy.material, occupancy.tile) != (*width, *height, *material, *tile) {
            return Err(Error("UV occupancy key does not match".into()));
        }
        self.shells.get(shell_id).filter(|shell| shell.material == *material).ok_or_else(|| Error("Selected shell does not belong to this material".into()))?;
        let (w, h) = (*width as usize, *height as usize);
        let point = |uv: [f64; 2]| [(uv[0] - f64::from(tile[0])) * w as f64, (1.0 - (uv[1] - f64::from(tile[1]))) * h as f64];
        let mut result = PaddingMask::default();
        let radius = *padding + std::f64::consts::FRAC_1_SQRT_2;
        let mut candidates = 0usize;
        for edge in self.boundaries.iter().filter(|edge| edge.shell == shell_id) {
            let pts = edge.uv.map(point);
            if pts.iter().flatten().any(|value| !value.is_finite()) {
                return Err(Error("UV shell boundary contains invalid coordinates".into()));
            }
            let y0 = (pts[0][1].min(pts[1][1]) - radius - 0.5).ceil().clamp(0.0, h as f64) as i32;
            let y1 = (pts[0][1].max(pts[1][1]) + radius - 0.5).floor().clamp(-1.0, h as f64 - 1.0) as i32;
            for y in y0..=y1 {
                let Some((left, right)) = capsule_row_span(pts, f64::from(y) + 0.5, radius) else { continue };
                let x0 = (left - 0.5).ceil().clamp(0.0, w as f64) as i32;
                let x1 = ((right - 0.5).floor() + 1.0).clamp(0.0, w as f64) as i32;
                let row_candidates = usize::try_from(x1.saturating_sub(x0)).unwrap_or(0);
                candidates = candidates.checked_add(row_candidates).ok_or_else(|| Error("UV shell padding candidate budget overflow".into()))?;
                if candidates > MAX_PADDING_CANDIDATES {
                    return Err(Error("UV shell padding exceeds the 16M candidate budget".into()));
                }
                for x in x0..x1 {
                    if !occupancy.contains(x, y) {
                        result.insert(x, y);
                    }
                }
            }
        }
        Ok(result)
    }

    pub fn mask_with_occupancy(&self, scene: &Scene, options: &MaskOptions, occupancy: &OccupancyMask) -> Result<Vec<f32>> {
        self.mask_with_padding_masks(scene, options, occupancy, None)
    }

    pub fn mask_with_padding_masks(
        &self,
        scene: &Scene,
        options: &MaskOptions,
        occupancy: &OccupancyMask,
        cached_padding: Option<&BTreeMap<usize, Arc<PaddingMask>>>,
    ) -> Result<Vec<f32>> {
        scene.validate()?;
        let MaskOptions { width, height, material, tile, selected, guard, horizontal, vertical, feather, open_borders, padding } = options;
        let (w, h) = (*width as usize, *height as usize);
        let n = w.checked_mul(h).filter(|n| *n > 0 && *n <= MAX_PIXELS).ok_or_else(|| Error("UV mask is limited to 16M pixels".into()))?;
        if *material >= scene.materials.len() || selected.iter().any(|i| self.shells.get(*i).is_none_or(|s| s.material != *material)) {
            return Err(Error("Selected shell does not belong to this material".into()));
        }
        if (occupancy.width, occupancy.height, occupancy.material, occupancy.tile) != (*width, *height, *material, *tile) {
            return Err(Error("UV occupancy key does not match".into()));
        }
        if [*guard, *horizontal, *vertical, *feather, *padding].iter().any(|v| !v.is_finite() || !(0.0..=1024.0).contains(v)) {
            return Err(Error("Margins, inward feather and outward padding must be in 0..1024 pixels".into()));
        }
        let point = |uv: [f64; 2]| [(uv[0] - f64::from(tile[0])) * w as f64, (1.0 - (uv[1] - f64::from(tile[1]))) * h as f64];
        let mut samples = filled(n, 0u8)?;
        for shell in selected.iter().filter_map(|i| self.shells.get(*i)) {
            for t in shell.triangles.iter().filter_map(|i| scene.triangles.get(*i)) {
                let pts = t.corners.map(|c| point(c.uv));
                let (x0, y0, x1, y1) = bounds(&pts, 0.0, w, h);
                for y in y0..y1 {
                    for x in x0..x1 {
                        // Four samples give subpixel shells a useful, antialiased boundary.
                        let coverage = [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)]
                            .iter()
                            .enumerate()
                            .fold(0u8, |bits, (i, (dx, dy))| if barycentric([x as f64 + dx, y as f64 + dy], pts).is_some() { bits | (1 << i) } else { bits });
                        if let Some(v) = samples.get_mut(y * w + x) {
                            *v |= coverage;
                        }
                    }
                }
            }
        }
        let mut mask = filled(n, 0.0f32)?;
        for (value, bits) in mask.iter_mut().zip(samples) {
            *value = bits.count_ones() as f32 * 0.25;
        }
        // Tiny conservative pad covers the entire square texel, not just its center.
        let pad = std::f64::consts::FRAC_1_SQRT_2;
        let radii = [*guard + *horizontal + pad, *guard + *vertical + pad];
        let radius = radii[0].max(radii[1]) + *feather;
        if *guard > 0.0 || *horizontal > 0.0 || *vertical > 0.0 || *feather > 0.0 {
            for e in self.boundaries.iter().filter(|e| selected.contains(&e.shell) && (e.shared || *open_borders)) {
                let pts = e.uv.map(point);
                let (x0, y0, x1, y1) = bounds(&pts, radius, w, h);
                let scaled = |p: [f64; 2]| [p[0] / radii[0], p[1] / radii[1]];
                let scaled_pts = pts.map(scaled);
                for y in y0..y1 {
                    for x in x0..x1 {
                        let delta = segment_delta(scaled([x as f64 + 0.5, y as f64 + 0.5]), scaled_pts);
                        let unit = delta[0].hypot(delta[1]);
                        // Exact elliptical hard margin; feather is a radial pixel distance outside it.
                        // Equal axes reduce to the previous isotropic distance, including texel padding.
                        let d = if unit <= 1.0 { 0.0 } else { (delta[0] * radii[0]).hypot(delta[1] * radii[1]) * (1.0 - 1.0 / unit) };
                        let alpha = if d <= 0.0 {
                            0.0
                        } else if *feather > 0.0 {
                            (d / *feather).clamp(0.0, 1.0) as f32
                        } else {
                            1.0
                        };
                        if let Some(v) = mask.get_mut(y * w + x) {
                            *v = v.min(alpha);
                        }
                    }
                }
            }
        }
        // Add the apron after inward protection so the existing guard remains an interior
        // operation with unchanged semantics.
        if *padding > 0.0 {
            for shell_id in selected {
                let generated;
                let padding_mask = if let Some(cached) = cached_padding.and_then(|masks| masks.get(shell_id)) {
                    cached
                } else {
                    generated = self.outward_padding_mask(options, *shell_id, occupancy)?;
                    &generated
                };
                padding_mask.for_each(|x, y| {
                    if let Some(value) = mask.get_mut(y as usize * w + x as usize) {
                        *value = 1.0;
                    }
                });
            }
        }
        Ok(mask)
    }
}

pub struct MaskOptions {
    pub width: u32,
    pub height: u32,
    pub material: usize,
    pub tile: [i32; 2],
    pub selected: BTreeSet<usize>,
    pub guard: f64,
    pub horizontal: f64,
    pub vertical: f64,
    pub feather: f64,
    pub open_borders: bool,
    pub padding: f64,
}

pub fn barycentric(p: [f64; 2], t: [[f64; 2]; 3]) -> Option<[f64; 3]> {
    let weights = barycentric_unbounded(p, t)?;
    if weights.iter().all(|v| *v >= -1e-9) { Some(weights) } else { None }
}

/// Barycentric weights outside the triangle, used only to extrapolate a bounded UV apron.
pub fn barycentric_unbounded(p: [f64; 2], t: [[f64; 2]; 3]) -> Option<[f64; 3]> {
    let [a, b, c] = t;
    let cross = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    let area = cross(a, b, c);
    if area.abs() < 1e-15 {
        return None;
    }
    let v = cross(a, p, c) / area;
    let w = cross(a, b, p) / area;
    let u = 1.0 - v - w;
    Some([u, v, w])
}

fn bounds<const N: usize>(pts: &[[f64; 2]; N], pad: f64, w: usize, h: usize) -> (usize, usize, usize, usize) {
    let lo = pts.iter().fold([f64::INFINITY; 2], |a, p| [a[0].min(p[0]), a[1].min(p[1])]);
    let hi = pts.iter().fold([f64::NEG_INFINITY; 2], |a, p| [a[0].max(p[0]), a[1].max(p[1])]);
    (
        (lo[0] - pad).floor().clamp(0.0, w as f64) as usize,
        (lo[1] - pad).floor().clamp(0.0, h as f64) as usize,
        (hi[0] + pad).ceil().clamp(0.0, w as f64) as usize,
        (hi[1] + pad).ceil().clamp(0.0, h as f64) as usize,
    )
}

fn segment_delta(p: [f64; 2], [a, b]: [[f64; 2]; 2]) -> [f64; 2] {
    let d = [b[0] - a[0], b[1] - a[1]];
    let len = d[0] * d[0] + d[1] * d[1];
    let t = if len > 1e-15 { (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / len).clamp(0.0, 1.0) } else { 0.0 };
    [p[0] - a[0] - t * d[0], p[1] - a[1] - t * d[1]]
}

/// Horizontal cross-section of the capsule around a segment. The rectangular
/// strip and its circular caps are intersected with a scanline so long diagonal
/// edges do work proportional to their covered area rather than their bounding box.
fn capsule_row_span([a, b]: [[f64; 2]; 2], y: f64, radius: f64) -> Option<(f64, f64)> {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let length = dx.hypot(dy);
    if !length.is_finite() || !y.is_finite() || !radius.is_finite() || radius < 0.0 {
        return None;
    }
    if length <= f64::EPSILON {
        return circle_row_span(a, y, radius);
    }

    let normal = [-dy / length, dx / length];
    let rectangle = [
        [a[0] + normal[0] * radius, a[1] + normal[1] * radius],
        [b[0] + normal[0] * radius, b[1] + normal[1] * radius],
        [b[0] - normal[0] * radius, b[1] - normal[1] * radius],
        [a[0] - normal[0] * radius, a[1] - normal[1] * radius],
    ];
    let mut span: Option<(f64, f64)> = None;
    let mut add = |x: f64| {
        if x.is_finite() {
            span = Some(span.map_or((x, x), |(left, right)| (left.min(x), right.max(x))));
        }
    };
    for index in 0..rectangle.len() {
        let p = rectangle[index];
        let q = rectangle[(index + 1) % rectangle.len()];
        if (p[1] - q[1]).abs() <= f64::EPSILON {
            if (y - p[1]).abs() <= f64::EPSILON {
                add(p[0]);
                add(q[0]);
            }
        } else if y >= p[1].min(q[1]) && y <= p[1].max(q[1]) {
            add(p[0] + (y - p[1]) * (q[0] - p[0]) / (q[1] - p[1]));
        }
    }
    for endpoint in [a, b] {
        if let Some((left, right)) = circle_row_span(endpoint, y, radius) {
            add(left);
            add(right);
        }
    }
    span
}

fn circle_row_span(center: [f64; 2], y: f64, radius: f64) -> Option<(f64, f64)> {
    let dy = y - center[1];
    let remainder = radius * radius - dy * dy;
    (remainder >= 0.0 && remainder.is_finite()).then(|| {
        let half_width = remainder.sqrt();
        (center[0] - half_width, center[0] + half_width)
    })
}

#[cfg(test)]
mod capsule_scanline_tests {
    use super::*;

    #[test]
    fn capsule_scanline_matches_distance_oracle_for_axes_diagonals_and_points() {
        let segments: [([[f64; 2]; 2], f64); 4] =
            [([[1.2, 2.7], [9.1, 2.7]], 2.25), ([[4.4, 0.8], [4.4, 10.2]], 1.75), ([[1.0, 1.0], [10.0, 9.0]], 2.5), ([[6.0, 6.0], [6.0, 6.0]], 1.5)];
        for (segment, radius) in segments {
            for y in -3..14 {
                let span = capsule_row_span(segment, f64::from(y) + 0.5, radius);
                for x in -3..14 {
                    let point = [f64::from(x) + 0.5, f64::from(y) + 0.5];
                    let delta = segment_delta(point, segment);
                    let expected = delta[0] * delta[0] + delta[1] * delta[1] <= radius * radius + 1e-10;
                    let actual = span.is_some_and(|(left, right)| point[0] >= left - 1e-10 && point[0] <= right + 1e-10);
                    assert_eq!(actual, expected, "segment={segment:?}, radius={radius}, pixel=({x},{y})");
                }
            }
        }
    }

    #[test]
    fn long_diagonal_capsule_candidate_count_tracks_covered_rows_not_square_bounds() {
        let segment: [[f64; 2]; 2] = [[4.0, 4.0], [4_092.0, 4_092.0]];
        let radius = 8.75;
        let y0 = (segment[0][1] - radius - 0.5).ceil() as i32;
        let y1 = (segment[1][1] + radius - 0.5).floor() as i32;
        let candidates: usize = (y0..=y1)
            .filter_map(|y| capsule_row_span(segment, f64::from(y) + 0.5, radius))
            .map(|(left, right)| ((right - 0.5).floor() - (left - 0.5).ceil() + 1.0).max(0.0) as usize)
            .sum();
        assert!(candidates < 200_000, "scanline candidate count was {candidates}");
        assert!(candidates < MAX_PADDING_CANDIDATES);
    }
}

#[cfg(test)]
mod shell_id_map_tests {
    use super::*;
    use crate::{Corner, Material, Scene, Triangle};

    fn overlapping_scene(reverse: bool) -> Scene {
        let points = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 1.0], [0.0, 1.0, 1.0]];
        let triangle = |offset| Triangle {
            material: 0,
            corners: [
                Corner { vertex: offset, uv: [0.0, 0.0], normal: None },
                Corner { vertex: offset + 1, uv: [1.0, 0.0], normal: None },
                Corner { vertex: offset + 2, uv: [0.0, 1.0], normal: None },
            ],
        };
        Scene {
            positions: points,
            triangles: if reverse { vec![triangle(3), triangle(0)] } else { vec![triangle(0), triangle(3)] },
            materials: vec![Material::default()],
            warnings: Vec::new(),
        }
    }

    #[test]
    fn shell_id_grid_matches_occupancy_convention_and_resolves_overlaps_by_lowest_id() {
        let scene = overlapping_scene(false);
        let topology = Topology::build(&scene).unwrap();
        let ids = topology.shell_id_map(&scene, 8, 8, 0, [0, 0]).unwrap();
        let occupancy = topology.occupancy_mask(&scene, 8, 8, 0, [0, 0]).unwrap();
        assert_eq!(ids.len(), 64);
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(ids[y * 8 + x] != 0, occupancy.contains(x as i32, y as i32), "({x}, {y})");
                if occupancy.contains(x as i32, y as i32) {
                    assert_eq!(ids[y * 8 + x], 1, "overlap deterministically chooses shell zero");
                }
            }
        }
        let reversed = overlapping_scene(true);
        let reversed_topology = Topology::build(&reversed).unwrap();
        assert_eq!(reversed_topology.shell_id_map(&reversed, 8, 8, 0, [0, 0]).unwrap(), ids);
    }
}
