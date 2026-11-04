//! Local geodesic brush charts. Shared 3D edges are unfolded into the tangent
//! plane of the hit triangle, retaining each triangle's own UV coordinates.
use crate::{Error, Result, Scene};
use glam::{DVec2, DVec3};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub struct Mesh {
    adjacent: Vec<[Option<(usize, usize)>; 3]>,
}
pub struct Patch {
    pub triangle: usize,
    pub plane: [[f64; 2]; 3],
}
impl Mesh {
    pub fn new(scene: &Scene) -> Result<Self> {
        scene.validate()?;
        let mut edges = BTreeMap::<(usize, usize), Vec<(usize, usize)>>::new();
        let mut adjacent = crate::filled(scene.triangles.len(), [None; 3])?;
        for (id, t) in scene.triangles.iter().enumerate() {
            for edge in 0..3 {
                let (Some(a), Some(b)) = (t.corners.get(edge), t.corners.get((edge + 1) % 3)) else { continue };
                edges.entry((a.vertex.min(b.vertex), a.vertex.max(b.vertex))).or_default().push((id, edge));
            }
        }
        for list in edges.values() {
            // Ambiguous non-manifold edges are barriers, never an arbitrary branch.
            if let [a, b] = list.as_slice() {
                if let Some(e) = adjacent.get_mut(a.0).and_then(|t| t.get_mut(a.1)) {
                    *e = Some(*b);
                }
                if let Some(e) = adjacent.get_mut(b.0).and_then(|t| t.get_mut(b.1)) {
                    *e = Some(*a);
                }
            }
        }
        Ok(Self { adjacent })
    }
    pub fn chart(&self, scene: &Scene, start: usize, hit: [f64; 3], size: [u32; 2], reach: f64) -> Result<Vec<Patch>> {
        self.chart_scaled(scene, start, hit, size, reach, None)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn chart_scaled(&self, scene: &Scene, start: usize, hit: [f64; 3], size: [u32; 2], reach: f64, pixels_per_unit: Option<f64>) -> Result<Vec<Patch>> {
        if pixels_per_unit.is_some_and(|v| !v.is_finite() || v <= 0.0 || v > 1e20) {
            return Err(Error("Invalid wrap texel density".into()));
        }
        if !reach.is_finite() || !(0.0..=16384.0).contains(&reach) || hit.iter().any(|v| !v.is_finite()) || size.contains(&0) {
            return Err(Error("Invalid wrap footprint".into()));
        }
        let t = scene.triangles.get(start).ok_or_else(|| Error("Invalid wrap triangle".into()))?;
        let points = world_points(scene, start)?;
        let [a, b, c] = points;
        let n = (b - a).cross(c - a).normalize_or_zero();
        let uv = t.corners.map(|c| DVec2::new(c.uv[0] * f64::from(size[0]), -c.uv[1] * f64::from(size[1])));
        let det = (uv[1] - uv[0]).perp_dot(uv[2] - uv[0]);
        let area = (b - a).cross(c - a).length();
        if det.abs() < 1e-14 || area < 1e-24 {
            return Err(Error("Cannot paint a triangle with zero UV or geometric area".into()));
        }
        let scale = pixels_per_unit.unwrap_or_else(|| (det.abs() / area).sqrt());
        let u = (((b - a) * (uv[2].y - uv[0].y) - (c - a) * (uv[1].y - uv[0].y)) / det).normalize_or_zero();
        let v = n.cross(u) * det.signum();
        let hit = DVec3::from_array(hit);
        let plane = points.map(|p| [(p - hit).dot(u) * scale, (p - hit).dot(v) * scale]);
        let centroid = std::array::from_fn::<_, 2, _>(|i| plane.iter().map(|p| p[i]).sum::<f64>() / 3.0);
        let expanded = plane.map(|p| std::array::from_fn(|i| centroid[i] + (p[i] - centroid[i]) * 1.0001));
        if crate::topology::barycentric([0.0, 0.0], expanded).is_none() {
            return Err(Error("Brush hit lies outside the starting triangle".into()));
        }
        let mut queue = VecDeque::from([Patch { triangle: start, plane }]);
        let mut visited = BTreeSet::from([start]);
        let mut out = Vec::new();
        while let Some(patch) = queue.pop_front() {
            if out.len() >= 4096 {
                return Err(Error("Wrap footprint exceeds 4096 faces; reduce brush size".into()));
            }
            let old = scene.triangles.get(patch.triangle).ok_or_else(|| Error("Invalid wrap topology".into()))?;
            for edge in 0..3 {
                let Some((id, _)) = self.adjacent.get(patch.triangle).and_then(|a| a.get(edge)).copied().flatten() else { continue };
                if visited.contains(&id) {
                    continue;
                }
                let Some(next) = scene.triangles.get(id).filter(|n| n.material == t.material) else { continue };
                let (Some(ca), Some(cb), Some(pc), Some(pa), Some(pb)) = (
                    old.corners.get(edge),
                    old.corners.get((edge + 1) % 3),
                    patch.plane.get((edge + 2) % 3),
                    patch.plane.get(edge),
                    patch.plane.get((edge + 1) % 3),
                ) else {
                    continue;
                };
                let (pa, pb, pc) = (DVec2::from_array(*pa), DVec2::from_array(*pb), DVec2::from_array(*pc));
                let e = pb - pa;
                let len = e.length();
                if len < 1e-12 {
                    continue;
                }
                let distance = (pa + e * ((-pa).dot(e) / e.length_squared()).clamp(0.0, 1.0)).length();
                if distance > reach {
                    continue;
                }
                let Some(na) = next.corners.iter().position(|c| c.vertex == ca.vertex) else { continue };
                let Some(nb) = next.corners.iter().position(|c| c.vertex == cb.vertex) else { continue };
                let nc = 3usize.saturating_sub(na + nb);
                let np = world_points(scene, id)?;
                let (Some(a), Some(b), Some(c)) = (np.get(na), np.get(nb), np.get(nc)) else { continue };
                let (ra, rb) = ((*c - *a).length() * scale, (*c - *b).length() * scale);
                let x = (ra * ra - rb * rb + len * len) / (2.0 * len);
                let y = (ra * ra - x * x).max(0.0).sqrt();
                let dir = e / len;
                let side = e.perp_dot(pc - pa).signum();
                let q = pa + dir * x + DVec2::new(-dir.y, dir.x) * (-side * y);
                let mut plane = [[0.0; 2]; 3];
                for (i, p) in [(na, pa), (nb, pb), (nc, q)] {
                    if let Some(dst) = plane.get_mut(i) {
                        *dst = p.to_array();
                    }
                }
                visited.insert(id);
                queue.push_back(Patch { triangle: id, plane });
            }
            out.push(patch);
        }
        Ok(out)
    }
}
fn world_points(scene: &Scene, id: usize) -> Result<[DVec3; 3]> {
    let t = scene.triangles.get(id).ok_or_else(|| Error("Invalid wrap triangle".into()))?;
    let [Some(a), Some(b), Some(c)] = t.corners.map(|c| scene.positions.get(c.vertex).copied().map(DVec3::from_array)) else {
        return Err(Error("Invalid wrap vertex".into()));
    };
    Ok([a, b, c])
}
impl Patch {
    /// Tight UV bounds of the footprint intersection: zooming in never scans an
    /// entire atlas triangle for a tiny brush. No allocations depend on file extents.
    pub fn rasterize(&self, scene: &Scene, size: [u32; 2], reach: f64, mut emit: impl FnMut(i32, i32, [f64; 2])) -> Result<()> {
        self.rasterize_with_padding(scene, size, reach, None, 0.0, &mut emit)
    }

    pub fn rasterize_with_padding(
        &self,
        scene: &Scene,
        size: [u32; 2],
        reach: f64,
        padding_mask: Option<&crate::topology::PaddingMask>,
        padding: f64,
        mut emit: impl FnMut(i32, i32, [f64; 2]),
    ) -> Result<()> {
        if !padding.is_finite() || !(0.0..=1024.0).contains(&padding) {
            return Err(Error("Invalid wrap padding".into()));
        }
        let t = scene.triangles.get(self.triangle).ok_or_else(|| Error("Invalid wrap triangle".into()))?;
        let mut poly: Vec<DVec2> = self.plane.map(DVec2::from_array).to_vec();
        for (axis, sign) in [(0, 1.0), (0, -1.0), (1, 1.0), (1, -1.0)] {
            let mut clipped = Vec::new();
            for (a, b) in poly.iter().zip(poly.iter().cycle().skip(1)).take(poly.len()) {
                let value = |p: DVec2| if axis == 0 { p.x * sign - reach } else { p.y * sign - reach };
                let (va, vb) = (value(*a), value(*b));
                if va <= 0.0 {
                    clipped.push(*a);
                }
                if (va <= 0.0) != (vb <= 0.0) {
                    clipped.push(a.lerp(*b, va / (va - vb)));
                }
            }
            poly = clipped;
        }
        if poly.is_empty() {
            return Ok(());
        }
        let uv = t.corners.map(|c| [c.uv[0] * f64::from(size[0]), (1.0 - c.uv[1]) * f64::from(size[1])]);
        let mut lo = DVec2::splat(f64::INFINITY);
        let mut hi = DVec2::splat(f64::NEG_INFINITY);
        for p in poly {
            if let Some(w) = crate::topology::barycentric(p.to_array(), self.plane) {
                let q = DVec2::from_array(uv[0]) * w[0] + DVec2::from_array(uv[1]) * w[1] + DVec2::from_array(uv[2]) * w[2];
                lo = lo.min(q);
                hi = hi.max(q);
            }
        }
        if !lo.is_finite() || !hi.is_finite() {
            return Ok(());
        }
        let scan_padding = if padding_mask.is_some() && padding > 0.0 { padding + std::f64::consts::FRAC_1_SQRT_2 } else { 0.0 };
        let base_x0 = lo.x.floor().clamp(0.0, f64::from(size[0])) as i32;
        let base_y0 = lo.y.floor().clamp(0.0, f64::from(size[1])) as i32;
        let base_x1 = hi.x.ceil().clamp(0.0, f64::from(size[0])) as i32;
        let base_y1 = hi.y.ceil().clamp(0.0, f64::from(size[1])) as i32;
        let x0 = (lo.x - scan_padding).floor().clamp(0.0, f64::from(size[0])) as i32;
        let y0 = (lo.y - scan_padding).floor().clamp(0.0, f64::from(size[1])) as i32;
        let x1 = (hi.x + scan_padding).ceil().clamp(0.0, f64::from(size[0])) as i32;
        let y1 = (hi.y + scan_padding).ceil().clamp(0.0, f64::from(size[1])) as i32;
        if i64::from(x1 - x0) * i64::from(y1 - y0) > 4_194_304 {
            return Err(Error("Wrap footprint exceeds 4M texels; reduce brush size".into()));
        }
        for y in y0..y1 {
            for x in x0..x1 {
                let Some(w) = crate::topology::barycentric_unbounded([f64::from(x) + 0.5, f64::from(y) + 0.5], uv) else { continue };
                // Match `topology::barycentric`'s existing edge tolerance; otherwise the
                // padding path can emit a subpixel sliver outside the triangle without apron.
                let inside = w.iter().all(|v| *v >= -1e-9);
                if inside {
                    // Preserve the old brush-clipped chart bounds for all interior pixels.
                    if x < base_x0 || x >= base_x1 || y < base_y0 || y >= base_y1 {
                        continue;
                    }
                } else if !padding_mask.is_some_and(|mask| mask.contains(x, y)) {
                    continue;
                }
                let p = std::array::from_fn(|axis| self.plane[0][axis] * w[0] + self.plane[1][axis] * w[1] + self.plane[2][axis] * w[2]);
                emit(x, y, p);
            }
        }
        Ok(())
    }
}
