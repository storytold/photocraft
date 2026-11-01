//! Immutable, centred geometry and a bounded BVH shared by GPU upload and click picking.
use crate::{
    Error, Result, Scene,
    preview::{Camera, Materials, sample},
    topology::Topology,
};
use glam::DVec3;

pub struct Triangle {
    pub positions: [[f32; 3]; 3],
    pub normals: [[f32; 3]; 3],
    pub uv: [[f32; 2]; 3],
    pub face: [f32; 3],
    pub tangent: [f32; 3],
    pub bitangent: [f32; 3],
    pub material: usize,
    pub shell: u32,
}
struct Node {
    lo: DVec3,
    hi: DVec3,
    range: std::ops::Range<usize>,
    children: Option<[usize; 2]>,
}
pub struct Geometry {
    pub triangles: Vec<Triangle>,
    pub center: [f64; 3],
    pub diameter: f64,
    /// Per-axis prepared-coordinate bounds, normalized around `center` by `diameter`.
    pub bounds: [[f64; 2]; 3],
    nodes: Vec<Node>,
    order: Vec<usize>,
}
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub triangle: usize,
    pub uv: [f64; 2],
    /// Surface position in the source model coordinate system.
    pub position: [f64; 3],
}
fn v(p: [f32; 3]) -> DVec3 {
    DVec3::from_array(p.map(f64::from))
}
fn f(p: DVec3) -> [f32; 3] {
    p.to_array().map(|x| x as f32)
}
pub fn rotate(p: DVec3, camera: Camera) -> DVec3 {
    let (sy, cy) = camera.yaw.sin_cos();
    let (sp, cp) = camera.pitch.sin_cos();
    let x = cy * p.x + sy * p.z;
    let z = -sy * p.x + cy * p.z;
    DVec3::new(x, cp * p.y - sp * z, sp * p.y + cp * z)
}
fn inverse(p: DVec3, camera: Camera) -> DVec3 {
    let (sy, cy) = camera.yaw.sin_cos();
    let (sp, cp) = camera.pitch.sin_cos();
    let y = cp * p.y + sp * p.z;
    let z = -sp * p.y + cp * p.z;
    DVec3::new(cy * p.x - sy * z, y, sy * p.x + cy * z)
}

/// Closest point on a triangle and its barycentric weights. Degenerate triangles fall back to
/// their three edges so malformed or collapsed geometry cannot introduce NaNs into picking.
fn closest_point_on_triangle(point: DVec3, a: DVec3, b: DVec3, c: DVec3) -> (DVec3, DVec3) {
    let ab = b - a;
    let ac = c - a;
    let ap = point - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return (a, DVec3::new(1.0, 0.0, 0.0));
    }

    let bp = point - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return (b, DVec3::new(0.0, 1.0, 0.0));
    }

    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return (a + ab * v, DVec3::new(1.0 - v, v, 0.0));
    }

    let cp = point - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return (c, DVec3::new(0.0, 0.0, 1.0));
    }

    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return (a + ac * w, DVec3::new(1.0 - w, 0.0, w));
    }

    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        let w = (d4 - d3) / (d4 - d3 + d5 - d6);
        return (b + (c - b) * w, DVec3::new(0.0, 1.0 - w, w));
    }

    let denominator = va + vb + vc;
    if denominator.abs() > 1e-30 {
        let denominator = denominator.recip();
        let v = vb * denominator;
        let w = vc * denominator;
        return (a + ab * v + ac * w, DVec3::new(1.0 - v - w, v, w));
    }

    let closest_on_segment = |start: DVec3, end: DVec3, start_weight: DVec3, end_weight: DVec3| {
        let edge = end - start;
        let t = if edge.length_squared() > 1e-30 { ((point - start).dot(edge) / edge.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
        (start + edge * t, start_weight.lerp(end_weight, t))
    };
    let candidates = [
        closest_on_segment(a, b, DVec3::new(1.0, 0.0, 0.0), DVec3::new(0.0, 1.0, 0.0)),
        closest_on_segment(b, c, DVec3::new(0.0, 1.0, 0.0), DVec3::new(0.0, 0.0, 1.0)),
        closest_on_segment(c, a, DVec3::new(0.0, 0.0, 1.0), DVec3::new(1.0, 0.0, 0.0)),
    ];
    candidates
        .into_iter()
        .min_by(|left, right| point.distance_squared(left.0).total_cmp(&point.distance_squared(right.0)))
        .unwrap_or((a, DVec3::new(1.0, 0.0, 0.0)))
}
impl Geometry {
    pub fn new(scene: &Scene, topology: &Topology) -> Result<Self> {
        scene.validate()?;
        if topology.triangle_shell.len() != scene.triangles.len() {
            return Err(Error("UV topology does not match geometry".into()));
        }
        let (lo, hi) = scene
            .positions
            .iter()
            .map(|p| DVec3::from_array(*p))
            .fold((DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)), |(lo, hi), p| (lo.min(p), hi.max(p)));
        let center = (lo + hi) * 0.5;
        let diameter = (hi - lo).length().max(1e-12);
        let mut triangles = Vec::new();
        triangles.try_reserve_exact(scene.triangles.len()).map_err(|_| Error("Not enough memory for prepared geometry".into()))?;
        for (id, t) in scene.triangles.iter().enumerate() {
            let [Some(a), Some(b), Some(c)] = t.corners.map(|c| scene.positions.get(c.vertex).map(|p| (DVec3::from_array(*p) - center) / diameter)) else {
                return Err(Error("Invalid geometry index".into()));
            };
            let e1 = b - a;
            let e2 = c - a;
            let face = e1.cross(e2).normalize_or_zero();
            let [u0, u1, u2] = t.corners.map(|c| c.uv);
            let (du1, dv1, du2, dv2) = (u1[0] - u0[0], u1[1] - u0[1], u2[0] - u0[0], u2[1] - u0[1]);
            let det = du1 * dv2 - du2 * dv1;
            let (tangent, bitangent) = if det.abs() > 1e-15 {
                (f(((e1 * dv2 - e2 * dv1) / det).normalize_or_zero()), f(((e2 * du1 - e1 * du2) / det).normalize_or_zero()))
            } else {
                ([0.0; 3], [0.0; 3])
            };
            let shell = topology.triangle_shell.get(id).copied().and_then(|s| u32::try_from(s).ok()).ok_or_else(|| Error("Invalid shell index".into()))?;
            triangles.push(Triangle {
                positions: [f(a), f(b), f(c)],
                normals: t.corners.map(|c| f(c.normal.map(DVec3::from_array).unwrap_or(face).normalize_or_zero())),
                uv: t.corners.map(|c| c.uv.map(|x| x as f32)),
                face: f(face),
                tangent,
                bitangent,
                material: t.material,
                shell,
            });
        }
        let len = triangles.len();
        let bounds = std::array::from_fn(|axis| [(lo[axis] - center[axis]) / diameter, (hi[axis] - center[axis]) / diameter]);
        let mut result = Self { triangles, center: center.to_array(), diameter, bounds, nodes: Vec::new(), order: (0..len).collect() };
        result.build(0..len, 0);
        Ok(result)
    }
    fn build(&mut self, range: std::ops::Range<usize>, depth: usize) -> usize {
        let mut lo = DVec3::splat(f64::INFINITY);
        let mut hi = DVec3::splat(f64::NEG_INFINITY);
        for id in self.order.get(range.clone()).unwrap_or_default() {
            if let Some(t) = self.triangles.get(*id) {
                for p in t.positions {
                    lo = lo.min(v(p));
                    hi = hi.max(v(p));
                }
            }
        }
        let id = self.nodes.len();
        self.nodes.push(Node { lo, hi, range: range.clone(), children: None });
        if range.len() > 8 && depth < 32 {
            let extent = hi - lo;
            let axis = if extent.x >= extent.y && extent.x >= extent.z {
                0
            } else if extent.y >= extent.z {
                1
            } else {
                2
            };
            let mid = range.start + range.len() / 2;
            let triangles = &self.triangles;
            if let Some(order) = self.order.get_mut(range.clone()) {
                order.select_nth_unstable_by(range.len() / 2, |a, b| {
                    let centroid = |id: usize| triangles.get(id).map_or(0.0, |t| t.positions.iter().map(|p| p.get(axis).copied().unwrap_or(0.0)).sum::<f32>());
                    centroid(*a).total_cmp(&centroid(*b)).then(a.cmp(b))
                });
            }
            let a = self.build(range.start..mid, depth + 1);
            let b = self.build(mid..range.end, depth + 1);
            if let Some(node) = self.nodes.get_mut(id) {
                node.children = Some([a, b]);
            }
        }
        id
    }
    /// The same world-space orbit target and orthographic projection as the reference renderer.
    pub fn camera_values(&self, camera: Camera, size: [u32; 2]) -> [f32; 16] {
        let target = camera.target.map_or(DVec3::ZERO, |p| (DVec3::from_array(p) - DVec3::from_array(self.center)) / self.diameter);
        let target = rotate(target, camera);
        let width = f64::from(size[0].max(1));
        let height = f64::from(size[1].max(1));
        let scale = width.min(height) * if camera.perspective { 1.2 } else { 0.8 * camera.zoom.clamp(0.01, 100.0) };
        let (sy, cy) = camera.yaw.sin_cos();
        let (sp, cp) = camera.pitch.sin_cos();
        [
            sy as f32,
            cy as f32,
            sp as f32,
            cp as f32,
            (2.0 * scale / width) as f32,
            (2.0 * scale / height) as f32,
            (camera.pan[0] / self.diameter - target.x) as f32,
            (camera.pan[1] / self.diameter - target.y) as f32,
            if camera.shaded && camera.channel == crate::preview::Channel::Material { 1.0 } else { 0.0 },
            if camera.authored_normals { 1.0 } else { 0.0 },
            if camera.flip_faces { -1.0 } else { 1.0 },
            match camera.channel {
                crate::preview::Channel::Material => 0.0,
                crate::preview::Channel::Normal => 2.0,
                _ => 1.0,
            },
            camera.normal_strength.clamp(0.0, 4.0) as f32,
            if camera.flip_normal_y { -1.0 } else { 1.0 },
            if camera.perspective { camera.distance() as f32 } else { 0.0 },
            target.z as f32,
        ]
    }
    /// Project normalized prepared-geometry coordinates into top-left-origin [0,1] view coordinates.
    pub fn project_normalized(&self, camera: Camera, position: [f64; 3], size: [u32; 2]) -> Option<[f64; 2]> {
        let p = rotate(DVec3::from_array(position), camera);
        let c = self.camera_values(camera, size);
        let depth = if camera.perspective { f64::from(c[14]) + f64::from(c[15]) - p.z } else { 1.0 };
        if !depth.is_finite() || depth <= camera.near() && camera.perspective {
            return None;
        }
        Some([((p.x + f64::from(c[6])) * f64::from(c[4]) / depth + 1.0) * 0.5, (1.0 - (p.y + f64::from(c[7])) * f64::from(c[5]) / depth) * 0.5])
    }
    /// Change the orbit target without moving any projected point in the current view.
    pub fn retarget(&self, mut camera: Camera, position: [f64; 3]) -> Camera {
        if position.iter().any(|x| !x.is_finite()) {
            return camera;
        }
        let delta = DVec3::from_array(position) - DVec3::from_array(camera.target.unwrap_or(self.center));
        let shift = rotate(delta, camera);
        camera.pan[0] += shift.x;
        camera.pan[1] += shift.y;
        if camera.perspective {
            let distance = (camera.distance() - shift.z / self.diameter).max(1.5e-6);
            camera.zoom = 1.5 / distance;
        }
        camera.target = Some(position);
        camera
    }
    pub fn pick(&self, scene: &Scene, camera: Camera, maps: &Materials<'_>, at: [f64; 2], size: [u32; 2]) -> Option<usize> {
        self.hit(scene, camera, maps, at, size).map(|hit| hit.triangle)
    }
    /// Mirror a surface hit across a model-space plane perpendicular to `axis`.
    ///
    /// `offset` is expressed in the source model's coordinates, just like `Hit::position`;
    /// the result is in this prepared geometry's centred, diameter-normalized coordinates.
    pub fn mirror_position(&self, source: &Hit, axis: usize, offset: f64) -> Option<[f64; 3]> {
        if axis >= 3 || !offset.is_finite() || source.position.iter().any(|value| !value.is_finite()) || !self.diameter.is_finite() || self.diameter <= 0.0 {
            return None;
        }
        let center = DVec3::from_array(self.center);
        let mut mirrored = (DVec3::from_array(source.position) - center) / self.diameter;
        let normalized_offset = (offset - self.center[axis]) / self.diameter;
        mirrored[axis] = 2.0 * normalized_offset - mirrored[axis];
        mirrored.is_finite().then_some(mirrored.to_array())
    }
    /// Project the reflected point using the same normalized camera space as the preview.
    pub fn mirror_projection(&self, source: &Hit, axis: usize, offset: f64, camera: Camera, size: [u32; 2]) -> Option<[f64; 2]> {
        self.project_normalized(camera, self.mirror_position(source, axis, offset)?, size)
    }
    /// Pick the nearest surface point to the reflection anywhere in the prepared geometry.
    ///
    /// `max_distance` is measured in the source model's coordinates. The returned `Hit::position`
    /// remains in those coordinates. `scene`, `camera` and `size` are retained for call-site
    /// compatibility; correspondence is geometric and does not depend on visibility or viewport.
    pub fn mirror_hit(
        &self,
        _scene: &Scene,
        _camera: Camera,
        maps: &Materials<'_>,
        source: &Hit,
        axis: usize,
        offset: f64,
        _size: [u32; 2],
        max_distance: f64,
    ) -> Option<Hit> {
        if !max_distance.is_finite() || max_distance < 0.0 {
            return None;
        }
        let mirrored = self.mirror_position(source, axis, offset)?;
        self.nearest_surface_hit(maps, DVec3::from_array(mirrored), max_distance / self.diameter)
    }
    fn nearest_surface_hit(&self, maps: &Materials<'_>, point: DVec3, max_distance: f64) -> Option<Hit> {
        if !point.is_finite() || !max_distance.is_finite() || max_distance < 0.0 || self.nodes.is_empty() {
            return None;
        }
        let max_distance_squared = max_distance * max_distance;
        let mut nearest_squared = max_distance_squared;
        let mut found = None;
        let mut stack = vec![0usize];
        while let Some(node_id) = stack.pop() {
            let node = self.nodes.get(node_id)?;
            let nearest_in_bounds = point.clamp(node.lo, node.hi);
            if point.distance_squared(nearest_in_bounds) > nearest_squared {
                continue;
            }
            if let Some(children) = node.children {
                stack.extend(children);
                continue;
            }
            for &triangle_id in self.order.get(node.range.clone()).unwrap_or_default() {
                let Some(triangle) = self.triangles.get(triangle_id) else { continue };
                if maps.hidden.contains(&triangle.material) {
                    continue;
                }
                let [a, b, c] = triangle.positions.map(v);
                let (closest, barycentric) = closest_point_on_triangle(point, a, b, c);
                let distance_squared = point.distance_squared(closest);
                if !distance_squared.is_finite() || distance_squared > nearest_squared {
                    continue;
                }
                nearest_squared = distance_squared;
                let uv = std::array::from_fn(|i| {
                    f64::from(triangle.uv[0][i]) * barycentric.x + f64::from(triangle.uv[1][i]) * barycentric.y + f64::from(triangle.uv[2][i]) * barycentric.z
                });
                let position = (DVec3::from_array(self.center) + closest * self.diameter).to_array();
                found = Some(Hit { triangle: triangle_id, uv, position });
            }
        }
        found
    }
    /// Visible surface and barycentrically interpolated UV for texture-space tools.
    pub fn hit(&self, scene: &Scene, camera: Camera, maps: &Materials<'_>, at: [f64; 2], size: [u32; 2]) -> Option<Hit> {
        self.hit_ray(scene, camera, maps, at, size)
    }
    fn hit_ray(&self, scene: &Scene, camera: Camera, maps: &Materials<'_>, at: [f64; 2], size: [u32; 2]) -> Option<Hit> {
        if at.iter().any(|v| !v.is_finite()) || size.contains(&0) {
            return None;
        }
        let c = self.camera_values(camera, size);
        let x = (at[0] * 2.0 - 1.0) / f64::from(c[4]);
        let y = (1.0 - at[1] * 2.0) / f64::from(c[5]);
        let (origin, dir) = if camera.perspective {
            (
                inverse(DVec3::new(-f64::from(c[6]), -f64::from(c[7]), f64::from(c[14]) + f64::from(c[15])), camera),
                inverse(DVec3::new(x, y, -1.0).normalize(), camera),
            )
        } else {
            (inverse(DVec3::new(x - f64::from(c[6]), y - f64::from(c[7]), 2.0), camera), inverse(DVec3::new(0.0, 0.0, -1.0), camera))
        };
        let mut nearest = f64::INFINITY;
        let mut found = None;
        let mut stack = vec![0usize];
        while let Some(id) = stack.pop() {
            let node = self.nodes.get(id)?;
            let (mut near, mut far) = (0.0f64, nearest);
            for axis in 0..3 {
                let (o, d, l, h) = (origin[axis], dir[axis], node.lo[axis], node.hi[axis]);
                if d.abs() < 1e-15 {
                    if o < l || o > h {
                        far = -1.0;
                    }
                } else {
                    let (a, b) = ((l - o) / d, (h - o) / d);
                    near = near.max(a.min(b));
                    far = far.min(a.max(b));
                }
            }
            if near > far {
                continue;
            }
            if let Some(children) = node.children {
                stack.extend(children);
                continue;
            }
            for &id in self.order.get(node.range.clone()).unwrap_or_default() {
                let Some(t) = self.triangles.get(id) else { continue };
                if maps.hidden.contains(&t.material) || (camera.cull_backfaces && -v(t.face).dot(dir) * if camera.flip_faces { -1.0 } else { 1.0 } <= 0.0) {
                    continue;
                }
                let [a, b, vertex_c] = t.positions.map(v);
                let (e1, e2) = (b - a, vertex_c - a);
                let p = dir.cross(e2);
                let det = e1.dot(p);
                if det.abs() < 1e-15 {
                    continue;
                }
                let q = origin - a;
                let u = q.dot(p) / det;
                let cross = q.cross(e1);
                let vv = dir.dot(cross) / det;
                let distance = e2.dot(cross) / det;
                if u < -1e-8 || vv < -1e-8 || u + vv > 1.0 + 1e-8 || distance < 0.0 || distance >= nearest {
                    continue;
                }
                if camera.perspective {
                    let depth = f64::from(c[14]) + f64::from(c[15]) - rotate(a + e1 * u + e2 * vv, camera).z;
                    if depth < camera.near() || depth > camera.distance() + 2.0 {
                        continue;
                    }
                }
                let uv = std::array::from_fn(|i| f64::from(t.uv[0][i]) * (1.0 - u - vv) + f64::from(t.uv[1][i]) * u + f64::from(t.uv[2][i]) * vv);
                let alpha = maps.colors.get(t.material).and_then(Option::as_ref).map_or(1.0, |img| sample(img, uv)[3])
                    * scene.materials.get(t.material).map_or(1.0, |m| m.color[3]);
                if alpha < 0.5 {
                    continue;
                }
                nearest = distance;
                let position = (DVec3::from_array(self.center) + (a + e1 * u + e2 * vv) * self.diameter).to_array();
                found = Some(Hit { triangle: id, uv, position });
            }
        }
        found
    }
}

#[cfg(test)]
mod symmetry_tests {
    use super::*;
    use crate::{Corner, Material, Triangle as SceneTriangle};

    fn triangle(corners: [usize; 3]) -> SceneTriangle {
        SceneTriangle { corners: corners.map(|vertex| Corner { vertex, uv: [0.5, 0.5], normal: None }), material: 0 }
    }

    fn symmetric_scene() -> Scene {
        // The symmetry plane is x=10, far from the origin. The target side has a
        // second, closer surface to ensure mirror picking considers all BVH hits.
        Scene {
            positions: vec![
                [9.0, 4.5, -3.0],
                [9.9, 4.5, -3.0],
                [9.0, 5.5, -3.0],
                [10.1, 4.5, -3.0],
                [11.0, 4.5, -3.0],
                [10.1, 5.5, -3.0],
                [10.1, 4.5, -2.6],
                [11.0, 4.5, -2.6],
                [10.1, 5.5, -2.6],
            ],
            triangles: vec![triangle([0, 1, 2]), triangle([3, 4, 5]), triangle([6, 7, 8])],
            materials: vec![Material::default()],
            warnings: Vec::new(),
        }
    }

    #[test]
    fn mirror_position_converts_world_point_and_plane_to_normalized_space() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let scene = symmetric_scene();
        let topology = Topology::build(&scene)?;
        let geometry = Geometry::new(&scene, &topology)?;
        let source = Hit { triangle: 0, uv: [0.5, 0.5], position: [9.5, 4.75, -3.0] };
        let mirrored = geometry.mirror_position(&source, 0, 10.0).ok_or("valid reflection was rejected")?;
        let restored = (DVec3::from_array(geometry.center) + DVec3::from_array(mirrored) * geometry.diameter).to_array();
        assert!((restored[0] - 10.5).abs() < 1e-6);
        assert!((restored[1] - 4.75).abs() < 1e-6);
        assert!((restored[2] + 3.0).abs() < 1e-6);
        assert!(geometry.mirror_position(&source, 3, 10.0).is_none());
        Ok(())
    }

    #[test]
    fn mirror_hit_selects_surface_nearest_to_reflected_point() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let scene = symmetric_scene();
        let topology = Topology::build(&scene)?;
        let geometry = Geometry::new(&scene, &topology)?;
        let mut camera = Camera::default();
        camera.yaw = 0.0;
        camera.pitch = 0.0;
        let maps = Materials::default();
        let size = [400, 400];
        let source_normalized = (DVec3::new(9.5, 4.75, -3.0) - DVec3::from_array(geometry.center)) / geometry.diameter;
        let source_screen = geometry.project_normalized(camera, source_normalized.to_array(), size).ok_or("source point was not projectable")?;
        let source = geometry.hit(&scene, camera, &maps, source_screen, size).ok_or("source surface was not picked")?;
        assert_eq!(source.triangle, 0);

        let target_screen = geometry.mirror_projection(&source, 0, 10.0, camera, size).ok_or("reflected point was not projectable")?;
        let frontmost = geometry.hit(&scene, camera, &maps, target_screen, size).ok_or("target surface was not picked")?;
        assert_eq!(frontmost.triangle, 2);

        let reflected = geometry.mirror_hit(&scene, camera, &maps, &source, 0, 10.0, size, 0.1).ok_or("near reflected surface was not picked")?;
        assert_eq!(reflected.triangle, 1);
        assert!((reflected.position[0] - 10.5).abs() < 1e-5);
        assert!((reflected.position[1] - 4.75).abs() < 1e-5);
        assert!((reflected.position[2] + 3.0).abs() < 1e-5);
        Ok(())
    }

    #[test]
    fn mirror_hit_finds_offscreen_backface_geometry_but_respects_hidden_materials() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let mut scene = symmetric_scene();
        // Make the reflected surface face away from the camera. Picking is geometric and must
        // not depend on camera culling or whether its projection falls inside the viewport.
        scene.triangles[1].corners = [3, 5, 4].map(|vertex| Corner { vertex, uv: [0.5, 0.5], normal: None });
        let topology = Topology::build(&scene)?;
        let geometry = Geometry::new(&scene, &topology)?;
        let source = Hit { triangle: 0, uv: [0.5, 0.5], position: [9.5, 4.75, -3.0] };
        let mut camera = Camera::default();
        camera.pan = [10_000.0, -10_000.0];
        let mut maps = Materials::default();
        let reflected = geometry.mirror_hit(&scene, camera, &maps, &source, 0, 10.0, [0, 0], 0.1).ok_or("offscreen backface was not picked")?;
        assert_eq!(reflected.triangle, 1);

        maps.hidden.insert(0);
        assert!(geometry.mirror_hit(&scene, camera, &maps, &source, 0, 10.0, [0, 0], 0.1).is_none());
        Ok(())
    }
}
