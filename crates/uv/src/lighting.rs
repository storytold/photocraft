//! Bounded, deterministic image-based lighting preparation. No work is repeated during orbit.
use crate::{Error, Result, preview::Image};
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::f32::consts::{PI, TAU};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub pbr: bool,
    pub brightness: f32,
    pub rotation: f32,
    pub background: bool,
    pub background_brightness: f32,
    pub roughness: f32,
    pub metallic: f32,
    pub emissive: f32,
}
impl Default for Settings {
    fn default() -> Self {
        Self { pbr: true, brightness: 1.0, rotation: 0.0, background: false, background_brightness: 1.0, roughness: 0.5, metallic: 0.0, emissive: 1.0 }
    }
}
impl Settings {
    pub fn sanitized(mut self) -> Self {
        fn bound(x: f32, default: f32, lo: f32, hi: f32) -> f32 {
            if x.is_finite() { x.clamp(lo, hi) } else { default }
        }
        self.brightness = bound(self.brightness, 1.0, 0.0, 16.0);
        self.rotation = bound(self.rotation, 0.0, -360.0, 360.0);
        self.background_brightness = bound(self.background_brightness, 1.0, 0.0, 16.0);
        self.roughness = bound(self.roughness, 0.5, 0.045, 1.0);
        self.metallic = bound(self.metallic, 0.0, 0.0, 1.0);
        self.emissive = bound(self.emissive, 1.0, 0.0, 32.0);
        self
    }
}

pub struct Environment {
    /// Linear radiance, GGX-convolved at increasing perceptual roughness.
    pub specular: Vec<Image>,
    /// Cosine-weighted irradiance divided by pi.
    pub diffuse: Image,
}
/// Split-sum GGX scale/bias integrated against the same Smith visibility used by the BRDF.
pub fn brdf_lut() -> Image {
    let size = 64;
    let rgba = (0..size * size)
        .map(|i| {
            let nv = (i % size) as f32 / size as f32 + 0.5 / size as f32;
            let rough = (i / size) as f32 / size as f32 + 0.5 / size as f32;
            let v = Vec3::new((1.0 - nv * nv).sqrt(), 0.0, nv);
            let a2 = rough.powi(4);
            let mut sum = [0.0; 2];
            for j in 0..128u32 {
                let u = (j as f32 + 0.5) / 128.0;
                let xi = j.reverse_bits() as f32 * 2.328_306_4e-10;
                let cos = ((1.0 - xi) / (1.0 + (a2 - 1.0) * xi)).sqrt();
                let sin = (1.0 - cos * cos).max(0.0).sqrt();
                let (s, c) = (TAU * u).sin_cos();
                let h = Vec3::new(sin * c, sin * s, cos);
                let vh = v.dot(h).max(0.0);
                let l = 2.0 * vh * h - v;
                let nl = l.z.max(0.0);
                if nl > 0.0 {
                    let smith = |n: f32| 2.0 * n / (n + (a2 + (1.0 - a2) * n * n).sqrt()).max(1e-6);
                    let visibility = smith(nv) * smith(nl) * vh / (cos * nv).max(1e-6);
                    let f = (1.0 - vh).powi(5);
                    sum[0] += (1.0 - f) * visibility;
                    sum[1] += f * visibility;
                }
            }
            [sum[0] / 128.0, sum[1] / 128.0, 0.0, 1.0]
        })
        .collect();
    Image { width: size, height: size, rgba }
}
fn direction(u: f32, v: f32) -> Vec3 {
    let (s, c) = (TAU * (u - 0.5)).sin_cos();
    let (st, ct) = (PI * v).sin_cos();
    Vec3::new(c * st, ct, s * st)
}
fn sample(im: &Image, d: Vec3) -> Vec3 {
    let u = d.z.atan2(d.x) / TAU + 0.5;
    let v = d.y.clamp(-1.0, 1.0).acos() / PI;
    // Clamp latitude at poles, repeat longitude only.
    let x = u * im.width as f32 - 0.5;
    let y = v * im.height as f32 - 0.5;
    let ix = x.floor() as i64;
    let iy = y.floor() as i64;
    let at = |x: i64, y: i64| {
        let x = x.rem_euclid(im.width as i64) as usize;
        let y = y.clamp(0, im.height.saturating_sub(1) as i64) as usize;
        im.rgba.get(y * im.width + x).map_or(Vec3::ZERO, |p| Vec3::from_array([p[0], p[1], p[2]]))
    };
    at(ix, iy).lerp(at(ix + 1, iy), x - x.floor()).lerp(at(ix, iy + 1).lerp(at(ix + 1, iy + 1), x - x.floor()), y - y.floor())
}
fn image(w: usize, h: usize, f: impl Fn(Vec3) -> Vec3) -> Image {
    let rgba = (0..w * h)
        .map(|i| {
            let c = f(direction((i % w) as f32 / w as f32 + 0.5 / w as f32, (i / w) as f32 / h as f32 + 0.5 / h as f32));
            [c.x, c.y, c.z, 1.0]
        })
        .collect();
    Image { width: w, height: h, rgba }
}
impl Environment {
    pub fn studio() -> Self {
        let source = image(256, 128, |d| {
            let sky = Vec3::new(0.45, 0.52, 0.65).lerp(Vec3::splat(0.18), (-d.y).max(0.0));
            let key = d.dot(Vec3::new(-0.5, 0.7, 0.5).normalize()).max(0.0).powf(48.0) * 10.0;
            let rim = d.dot(Vec3::new(0.7, 0.3, -0.6).normalize()).max(0.0).powf(80.0) * 6.0;
            sky + Vec3::new(1.0, 0.92, 0.82) * key + Vec3::new(0.7, 0.82, 1.0) * rim
        });
        // The generated dimensions/radiance obey the same bounds as imported maps.
        Self::prepare_valid(&source)
    }
    pub fn prepare(source: &Image) -> Result<Self> {
        if source.width == 0
            || source.height == 0
            || source.width.checked_mul(source.height) != Some(source.rgba.len())
            || source.rgba.len() > 33_554_432
            || source.rgba.iter().flatten().any(|v| !v.is_finite() || *v < 0.0 || *v > 65504.0)
        {
            return Err(Error("Invalid environment: expected finite positive linear HDR radiance (up to 32 MP)".into()));
        }
        Ok(Self::prepare_valid(source))
    }
    fn prepare_valid(source: &Image) -> Self {
        let samples: Vec<_> = (0..64u32).map(|i| ((i as f32 + 0.5) / 64.0, i.reverse_bits() as f32 * 2.328_306_4e-10)).collect();
        let convolve = |n: Vec3, roughness: f32, diffuse: bool| {
            let up = if n.y.abs() < 0.999 { Vec3::Y } else { Vec3::X };
            let t = up.cross(n).normalize();
            let b = n.cross(t);
            let (mut sum, mut weight) = (Vec3::ZERO, 0.0);
            for &(u, v) in &samples {
                let a2 = roughness.powi(4);
                let cos = if diffuse { (1.0 - v).sqrt() } else { ((1.0 - v) / (1.0 + (a2 - 1.0) * v)).sqrt() };
                let sin = (1.0 - cos * cos).max(0.0).sqrt();
                let (s, c) = (TAU * u).sin_cos();
                let h = t * (sin * c) + b * (sin * s) + n * cos;
                let l = if diffuse { h } else { 2.0 * n.dot(h) * h - n };
                let w = if diffuse { 1.0 } else { n.dot(l).max(0.0) };
                sum += sample(source, l) * w;
                weight += w;
            }
            sum / weight.max(1e-6)
        };
        let mut specular = Vec::new();
        for level in 0..9 {
            let w = (256 >> level).max(1);
            let h = (128 >> level).max(1);
            specular.push(image(w, h, |d| if level == 0 { sample(source, d) } else { convolve(d, level as f32 / 8.0, false) }));
        }
        Self { specular, diffuse: image(32, 16, |d| convolve(d, 1.0, true)) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn constant_hdr_energy_survives_prefilter_and_invalid_input_is_rejected() {
        let mut source = Image { width: 4, height: 2, rgba: vec![[4.0, 2.0, 0.5, 1.0]; 8] };
        let e = Environment::prepare(&source).unwrap();
        for p in e.specular.iter().chain(std::iter::once(&e.diffuse)).flat_map(|im| &im.rgba) {
            assert!((p[0] - 4.0).abs() < 1e-4 && (p[1] - 2.0).abs() < 1e-4);
        }
        source.rgba[0][0] = f32::NAN;
        assert!(Environment::prepare(&source).is_err());
        source.width = usize::MAX;
        assert!(Environment::prepare(&source).is_err());
        let s = Settings { brightness: f32::INFINITY, roughness: -1.0, ..Default::default() }.sanitized();
        assert_eq!(s.brightness, 1.0);
        assert_eq!(s.roughness, 0.045);
    }
}
