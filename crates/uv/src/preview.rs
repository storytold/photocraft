//! Depth-buffered orthographic preview with authored normals and tangent-space normal maps.
use crate::{Error, Result, Scene, filled, topology::barycentric};
use glam::DVec3;
use std::collections::BTreeSet;
#[derive(Clone, Debug)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<[f32; 4]>,
}
/// Available material maps can also be inspected as unlit numeric RGB.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
pub enum Channel {
    #[default]
    Material,
    Color,
    Normal,
    Id,
    Ao,
    Curvature,
    Opacity,
    Metallic,
    Roughness,
    Emissive,
}
impl Channel {
    pub const MAPS: [Self; 9] =
        [Self::Color, Self::Normal, Self::Id, Self::Ao, Self::Curvature, Self::Opacity, Self::Metallic, Self::Roughness, Self::Emissive];
    pub fn label(self) -> &'static str {
        match self {
            Self::Material => "Material",
            Self::Color => "Color",
            Self::Normal => "Normal",
            Self::Id => "ID",
            Self::Ao => "AO",
            Self::Curvature => "CV",
            Self::Opacity => "Opacity",
            Self::Metallic => "Metallic",
            Self::Roughness => "Roughness",
            Self::Emissive => "Emissive",
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::Material => "material",
            Self::Color => "baseColor",
            Self::Normal => "normal",
            Self::Id => "id",
            Self::Ao => "ao",
            Self::Curvature => "curvature",
            Self::Opacity => "opacity",
            Self::Metallic => "metallic",
            Self::Roughness => "roughness",
            Self::Emissive => "emissive",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub perspective: bool,
    pub yaw: f64,
    pub pitch: f64,
    pub zoom: f64,
    pub pan: [f64; 2],
    pub target: Option<[f64; 3]>,
    pub shaded: bool,
    /// Material rendering or an individual map without lighting / RGB tint.
    pub channel: Channel,
    pub cull_backfaces: bool,
    pub flip_faces: bool,
    pub authored_normals: bool,
    pub normal_strength: f64,
    pub flip_normal_y: bool,
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            perspective: false,
            yaw: 0.65,
            pitch: 0.65,
            zoom: 1.0,
            pan: [0.0; 2],
            target: None,
            shaded: false,
            channel: Channel::Material,
            cull_backfaces: true,
            flip_faces: false,
            authored_normals: true,
            normal_strength: 1.0,
            flip_normal_y: false,
        }
    }
}
impl Camera {
    pub fn distance(self) -> f64 {
        1.5 / self.zoom.clamp(0.01, 1_000_000.0)
    }
    pub fn near(self) -> f64 {
        (self.distance() * 0.001).max(1e-8)
    }
}

pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<[u8; 4]>,
    pub triangles: Vec<Option<usize>>,
}
#[derive(Default)]
pub struct Materials<'a> {
    pub colors: &'a [Option<Image>],
    pub normals: &'a [Option<Image>],
    pub hidden: BTreeSet<usize>,
}
pub const MAX_PREVIEW_PIXELS: usize = 16_777_216;

pub fn render(scene: &Scene, camera: Camera, textures: &[Option<Image>], selected: &[bool], width: usize, height: usize) -> Result<Frame> {
    render_materials(scene, camera, &Materials { colors: textures, ..Default::default() }, selected, width, height)
}
pub fn render_materials(scene: &Scene, camera: Camera, maps: &Materials<'_>, selected: &[bool], width: usize, height: usize) -> Result<Frame> {
    scene.validate()?;
    let n = width.checked_mul(height).filter(|n| *n > 0 && *n <= MAX_PREVIEW_PIXELS).ok_or_else(|| Error("Preview is limited to 16M pixels".into()))?;
    if !camera.yaw.is_finite()
        || !camera.pitch.is_finite()
        || !camera.zoom.is_finite()
        || !camera.normal_strength.is_finite()
        || camera.pan.iter().any(|v| !v.is_finite())
        || camera.target.is_some_and(|p| p.iter().any(|v| !v.is_finite() || v.abs() > 1e15))
    {
        return Err(Error("Invalid camera".into()));
    }
    if camera.perspective {
        return crate::perspective::render(scene, camera, maps, selected, width, height);
    }
    let mut rgba = filled(n, [35u8, 35, 35, 255])?;
    let mut triangles = filled(n, None)?;
    let mut depth = filled(n, f64::NEG_INFINITY)?;
    let (lo, hi) = scene
        .positions
        .iter()
        .map(|p| DVec3::from_array(*p))
        .fold((DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)), |(lo, hi), p| (lo.min(p), hi.max(p)));
    let center = camera.target.map_or((lo + hi) * 0.5, DVec3::from_array);
    let scale = width.min(height) as f64 * 0.8 / (hi - lo).length().max(1e-12) * camera.zoom.clamp(0.01, 100.0);
    let (sy, cy) = camera.yaw.sin_cos();
    let (sp, cp) = camera.pitch.sin_cos();
    let rotate = |p: DVec3| {
        let x = cy * p.x + sy * p.z;
        let z = -sy * p.x + cy * p.z;
        DVec3::new(x, cp * p.y - sp * z, sp * p.y + cp * z)
    };
    let projected: Vec<DVec3> = scene
        .positions
        .iter()
        .map(|p| {
            let p = rotate(DVec3::from_array(*p) - center);
            DVec3::new(width as f64 * 0.5 + (p.x + camera.pan[0]) * scale, height as f64 * 0.5 - (p.y + camera.pan[1]) * scale, p.z)
        })
        .collect();
    let light = DVec3::new(0.3, 0.6, 0.75).normalize();
    for (id, t) in scene.triangles.iter().enumerate() {
        if maps.hidden.contains(&t.material) {
            continue;
        }
        let [Some(a), Some(b), Some(c)] = t.corners.map(|v| projected.get(v.vertex).copied()) else { continue };
        let [p0, p1, p2] = t.corners.map(|c| DVec3::from_array(scene.positions.get(c.vertex).copied().unwrap_or([0.0; 3])));
        let e1 = p1 - p0;
        let e2 = p2 - p0;
        let face = e1.cross(e2).normalize_or_zero() * if camera.flip_faces { -1.0 } else { 1.0 };
        if face == DVec3::ZERO || (camera.cull_backfaces && rotate(face).z <= 0.0) {
            continue;
        }
        let ns = t.corners.map(|c| {
            if camera.authored_normals {
                c.normal.map(DVec3::from_array).map(|n| n.normalize_or_zero() * if camera.flip_faces { -1.0 } else { 1.0 }).unwrap_or(face)
            } else {
                face
            }
        });
        let [u0, u1, u2] = t.corners.map(|c| c.uv);
        let du1 = u1[0] - u0[0];
        let dv1 = u1[1] - u0[1];
        let du2 = u2[0] - u0[0];
        let dv2 = u2[1] - u0[1];
        let det = du1 * dv2 - du2 * dv1;
        let basis = if det.abs() > 1e-15 { Some(((e1 * dv2 - e2 * dv1) / det, (e2 * du1 - e1 * du2) / det)) } else { None };
        let pts = [a, b, c].map(|p| [p.x, p.y]);
        let lo = a.min(b).min(c);
        let hi = a.max(b).max(c);
        let Some(material) = scene.materials.get(t.material) else { continue };
        let texture = maps.colors.get(t.material).and_then(Option::as_ref);
        let normal_map = maps.normals.get(t.material).and_then(Option::as_ref);
        for y in lo.y.floor().clamp(0.0, height as f64) as usize..hi.y.ceil().clamp(0.0, height as f64) as usize {
            for x in lo.x.floor().clamp(0.0, width as f64) as usize..hi.x.ceil().clamp(0.0, width as f64) as usize {
                let Some(weights) = barycentric([x as f64 + 0.5, y as f64 + 0.5], pts) else { continue };
                let [wa, wb, wc] = weights;
                let z = wa * a.z + wb * b.z + wc * c.z;
                let index = y * width + x;
                let Some(previous) = depth.get_mut(index) else { continue };
                if z <= *previous {
                    continue;
                }
                let uv = t.corners.iter().zip(weights).fold([0.0; 2], |r, (c, w)| [r[0] + c.uv[0] * w, r[1] + c.uv[1] * w]);
                let mut texel = texture.map_or([1.0; 4], |img| sample(img, uv));
                if camera.channel == Channel::Normal {
                    let rgb = normal_map.map_or([0.5, 0.5, 1.0, 1.0], |img| sample(img, uv));
                    texel[..3].copy_from_slice(&rgb[..3]);
                }
                let mut normal = (ns[0] * wa + ns[1] * wb + ns[2] * wc).normalize_or_zero();
                if let (Some(map), Some((tangent, bitangent))) = (normal_map, basis) {
                    let t = (tangent - normal * normal.dot(tangent)).normalize_or_zero();
                    let b = normal.cross(t) * if normal.cross(t).dot(bitangent) < 0.0 { -1.0 } else { 1.0 };
                    let rgb = sample(map, uv);
                    let strength = camera.normal_strength.clamp(0.0, 4.0);
                    let bump = DVec3::new(
                        (f64::from(rgb[0]) * 2.0 - 1.0) * strength,
                        (f64::from(rgb[1]) * 2.0 - 1.0) * strength * if camera.flip_normal_y { -1.0 } else { 1.0 },
                        f64::from(rgb[2]) * 2.0 - 1.0,
                    )
                    .normalize_or_zero();
                    if t != DVec3::ZERO && bump != DVec3::ZERO {
                        normal = (t * bump.x + b * bump.y + normal * bump.z).normalize_or_zero();
                    }
                }
                let lighting =
                    if camera.shaded && camera.channel == Channel::Material { (0.25 + rotate(normal).dot(light).max(0.0) * 0.75) as f32 } else { 1.0 };
                let mut pixel = [0u8; 4];
                for (channel, value) in pixel.iter_mut().enumerate() {
                    let mut c = texel.get(channel).copied().unwrap_or(1.0)
                        * if camera.channel != Channel::Material && channel < 3 { 1.0 } else { material.color.get(channel).copied().unwrap_or(1.0) };
                    if channel < 3 {
                        c *= lighting;
                        if selected.get(id).copied().unwrap_or(false) {
                            c = c * 0.82 + [0.12, 0.55, 0.9].get(channel).copied().unwrap_or(0.0) * 0.18
                        }
                    }
                    *value = (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                }
                if pixel[3] < 128 {
                    continue;
                }
                pixel[3] = 255;
                *previous = z;
                if let Some(p) = rgba.get_mut(index) {
                    *p = pixel
                }
                if let Some(p) = triangles.get_mut(index) {
                    *p = Some(id)
                }
            }
        }
    }
    Ok(Frame { width, height, rgba, triangles })
}
/// Bilinear repeat sampling in texel-centred coordinates, including the wrap seam.
/// Bilinear numeric sampling with repeat addressing and a safe missing-pixel fallback.
pub fn sample(img: &Image, uv: [f64; 2]) -> [f32; 4] {
    if img.width == 0 || img.height == 0 {
        return [1.0; 4];
    }
    let x = uv[0].rem_euclid(1.0) * img.width as f64 - 0.5;
    let y = (1.0 - uv[1].rem_euclid(1.0)) * img.height as f64 - 0.5;
    let x0 = x.floor();
    let y0 = y.floor();
    let tx = (x - x0) as f32;
    let ty = (y - y0) as f32;
    let at = |x: f64, y: f64| {
        let x = x.rem_euclid(img.width as f64) as usize;
        let y = y.rem_euclid(img.height as f64) as usize;
        img.rgba.get(y.saturating_mul(img.width).saturating_add(x)).copied().unwrap_or([1.0; 4])
    };
    let [a, b, c, d] = [at(x0, y0), at(x0 + 1.0, y0), at(x0, y0 + 1.0), at(x0 + 1.0, y0 + 1.0)];
    std::array::from_fn(|i| (a[i] * (1.0 - tx) + b[i] * tx) * (1.0 - ty) + (c[i] * (1.0 - tx) + d[i] * tx) * ty)
}

#[cfg(test)]
mod sampling_tests {
    use super::*;
    #[test]
    fn bilinear_filter_averages_texels_and_wraps_instead_of_clamping() {
        let image = Image { width: 2, height: 2, rgba: vec![[0.0, 0.0, 0.0, 1.0], [1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0], [1.0, 1.0, 0.0, 1.0]] };
        assert_eq!(sample(&image, [0.5, 0.5]), [0.5, 0.5, 0.0, 1.0]);
        assert_eq!(sample(&image, [0.0, 0.5]), [0.5, 0.5, 0.0, 1.0]);
        assert_eq!(sample(&image, [0.25, 0.75]), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(sample(&image, [-0.5, 1.5]), sample(&image, [0.5, 0.5]));
    }
}
