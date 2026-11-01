//! Perspective reference rasterizer: near/far clipping and perspective-correct attributes.
use crate::{
    Result, Scene, filled,
    prepared::{Geometry, rotate},
    preview::{Camera, Frame, Materials, sample},
    topology::{Topology, barycentric},
};
use glam::DVec3;
#[derive(Clone, Copy, Default)]
struct Vertex {
    p: DVec3,
    uv: [f64; 2],
    n: DVec3,
}
impl Vertex {
    fn mix(self, b: Self, t: f64) -> Self {
        Self { p: self.p.lerp(b.p, t), uv: std::array::from_fn(|i| self.uv[i] + (b.uv[i] - self.uv[i]) * t), n: self.n.lerp(b.n, t) }
    }
}
fn clip(vertices: &[Vertex], plane: f64, greater: bool) -> ([Vertex; 6], usize) {
    let mut out = [Vertex::default(); 6];
    let mut len = 0;
    let Some(mut a) = vertices.last().copied() else { return (out, 0) };
    for &b in vertices {
        let inside = |p: Vertex| if greater { p.p.z >= plane } else { p.p.z <= plane };
        if inside(a) != inside(b) {
            let t = (plane - a.p.z) / (b.p.z - a.p.z);
            if let Some(v) = out.get_mut(len) {
                *v = a.mix(b, t);
                len += 1;
            }
        }
        if inside(b)
            && let Some(v) = out.get_mut(len)
        {
            *v = b;
            len += 1;
        }
        a = b;
    }
    (out, len)
}
pub(crate) fn render(scene: &Scene, camera: Camera, maps: &Materials<'_>, selected: &[bool], width: usize, height: usize) -> Result<Frame> {
    let geometry = Geometry::new(scene, &Topology::build(scene)?)?;
    let count = width * height; // already bounded by preview::render_materials
    let mut rgba = filled(count, [35, 35, 35, 255])?;
    let mut triangles = filled(count, None)?;
    let mut depth = filled(count, f64::INFINITY)?;
    let c = geometry.camera_values(camera, [width as u32, height as u32]);
    let light = DVec3::new(0.3, 0.6, 0.75).normalize();
    let sign = if camera.flip_faces { -1.0 } else { 1.0 };
    let focal = width.min(height) as f64 * 1.2;
    for (id, t) in geometry.triangles.iter().enumerate() {
        if maps.hidden.contains(&t.material) {
            continue;
        }
        let face = DVec3::from_array(t.face.map(f64::from)) * sign;
        let positions = t.positions.map(|p| rotate(DVec3::from_array(p.map(f64::from)), camera));
        let camera_to_vertex =
            DVec3::new(positions[0].x + f64::from(c[6]), positions[0].y + f64::from(c[7]), positions[0].z - f64::from(c[15]) - camera.distance());
        if camera.cull_backfaces && rotate(face, camera).dot(camera_to_vertex) >= 0.0 {
            continue;
        }
        let vertices = std::array::from_fn::<_, 3, _>(|i| Vertex {
            p: DVec3::new(positions[i].x + f64::from(c[6]), positions[i].y + f64::from(c[7]), camera.distance() + f64::from(c[15]) - positions[i].z),
            uv: t.uv[i].map(f64::from),
            n: if camera.authored_normals { DVec3::from_array(t.normals[i].map(f64::from)) * sign } else { face },
        });
        let (near, n) = clip(&vertices, camera.near(), true);
        let (poly, n) = clip(near.get(..n).unwrap_or_default(), camera.distance() + 2.0, false);
        let Some(a) = poly.first().copied() else {
            continue;
        };
        for pair in poly.get(1..n).unwrap_or_default().windows(2) {
            let Some((&b, &d)) = pair.first().zip(pair.get(1)) else {
                continue;
            };
            let v = [a, b, d];
            let pts = v.map(|v| [width as f64 * 0.5 + v.p.x * focal / v.p.z, height as f64 * 0.5 - v.p.y * focal / v.p.z]);
            let lo = [pts[0][0].min(pts[1][0]).min(pts[2][0]), pts[0][1].min(pts[1][1]).min(pts[2][1])];
            let hi = [pts[0][0].max(pts[1][0]).max(pts[2][0]), pts[0][1].max(pts[1][1]).max(pts[2][1])];
            let Some(material) = scene.materials.get(t.material) else {
                continue;
            };
            let texture = maps.colors.get(t.material).and_then(Option::as_ref);
            let normal_map = maps.normals.get(t.material).and_then(Option::as_ref);
            for y in lo[1].floor().clamp(0.0, height as f64) as usize..hi[1].ceil().clamp(0.0, height as f64) as usize {
                for x in lo[0].floor().clamp(0.0, width as f64) as usize..hi[0].ceil().clamp(0.0, width as f64) as usize {
                    let Some(w) = barycentric([x as f64 + 0.5, y as f64 + 0.5], pts) else {
                        continue;
                    };
                    let reciprocal = std::array::from_fn::<_, 3, _>(|i| w[i] / v[i].p.z);
                    let sum = reciprocal.iter().sum::<f64>();
                    let z = 1.0 / sum;
                    let index = y * width + x;
                    let Some(previous) = depth.get_mut(index) else {
                        continue;
                    };
                    if z >= *previous {
                        continue;
                    }
                    let w = reciprocal.map(|w| w / sum);
                    let uv = std::array::from_fn(|axis| v.iter().zip(w).map(|(v, w)| v.uv[axis] * w).sum());
                    let texel = texture.map_or([1.0; 4], |img| sample(img, uv));
                    if texel[3] * material.color[3] < 0.5 {
                        continue;
                    }
                    let texel =
                        if camera.channel == super::preview::Channel::Normal { normal_map.map_or([0.5, 0.5, 1.0, 1.0], |img| sample(img, uv)) } else { texel };
                    let mut n = v.iter().zip(w).fold(DVec3::ZERO, |n, (v, w)| n + v.n * w).normalize_or_zero();
                    if let Some(map) = normal_map {
                        let tangent = DVec3::from_array(t.tangent.map(f64::from));
                        let bitangent = DVec3::from_array(t.bitangent.map(f64::from));
                        let tangent = (tangent - n * n.dot(tangent)).normalize_or_zero();
                        let bitangent = n.cross(tangent) * if n.cross(tangent).dot(bitangent) < 0.0 { -1.0 } else { 1.0 };
                        let rgb = sample(map, uv);
                        let strength = camera.normal_strength.clamp(0.0, 4.0);
                        let bump = DVec3::new(
                            (f64::from(rgb[0]) * 2.0 - 1.0) * strength,
                            (f64::from(rgb[1]) * 2.0 - 1.0) * strength * if camera.flip_normal_y { -1.0 } else { 1.0 },
                            f64::from(rgb[2]) * 2.0 - 1.0,
                        )
                        .normalize_or_zero();
                        if tangent != DVec3::ZERO && bump != DVec3::ZERO {
                            n = (tangent * bump.x + bitangent * bump.y + n * bump.z).normalize_or_zero();
                        }
                    }
                    let lighting = if camera.shaded && camera.channel == super::preview::Channel::Material {
                        (0.25 + rotate(n, camera).dot(light).max(0.0) * 0.75) as f32
                    } else {
                        1.0
                    };
                    let pixel = std::array::from_fn(|i| {
                        let mut value = if i == 3 {
                            1.0
                        } else {
                            texel[i] * if camera.channel != super::preview::Channel::Material { 1.0 } else { material.color[i] } * lighting
                        };
                        if i < 3 && selected.get(id).copied().unwrap_or(false) {
                            value = value * 0.82 + [0.12, 0.55, 0.9, 0.0][i] * 0.18;
                        }
                        (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
                    });
                    *previous = z;
                    if let Some(p) = rgba.get_mut(index) {
                        *p = pixel;
                    }
                    if let Some(p) = triangles.get_mut(index) {
                        *p = Some(id);
                    }
                }
            }
        }
    }
    Ok(Frame { width, height, rgba, triangles })
}
