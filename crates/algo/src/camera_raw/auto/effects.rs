use super::{CameraRaw, Image, luma, percentile};

pub(super) fn suggest(image: &Image<'_>, out: &mut CameraRaw) {
    let mut scene = Scene::measure(image);
    scene.detail.apply(out, image.shadow_noise);
    out.dehaze = scene.dehaze();
    scene.radial.apply(out);
}

struct Scene {
    detail: Detail,
    radial: Radial,
    dark: Vec<f32>,
    light: Vec<f32>,
    minima: Vec<f32>,
}

impl Scene {
    fn measure(image: &Image<'_>) -> Self {
        let mut scene = Self { detail: Detail::default(), radial: Radial::default(), dark: Vec::new(), light: Vec::new(), minima: Vec::new() };
        let radius = (image.w.min(image.h) / 40).clamp(2, 18);
        for (x, y, rgb) in image.samples(96) {
            let rgb = rgb.map(|v| v.clamp(0.0, 1.0));
            let center = luma(rgb);
            scene.minima.push(rgb[0].min(rgb[1]).min(rgb[2]));
            scene.radial.add((x as f32 + 0.5) / image.w as f32, (y as f32 + 0.5) / image.h as f32, center);
            if let (Some(fine), Some(coarse)) = (cross(image, x, y, 1), cross(image, x, y, radius)) {
                scene.detail.add(center, fine.map(luma), coarse.map(luma));
                let dark = coarse.into_iter().chain([rgb]).flatten().fold(1.0, f32::min);
                scene.dark.push(dark);
                scene.light.push(center);
            }
        }
        scene
    }

    fn dehaze(&mut self) -> f32 {
        if self.dark.len() < 64 || self.detail.structured * 20 < self.dark.len() {
            return 0.0;
        }
        // A raised dark channel across many regions is evidence of a veil. Flat fields,
        // naturally bright scenes and strong transitions readily violate that prior.
        let dark = percentile(&mut self.dark, 0.12);
        let low = percentile(&mut self.light, 0.10);
        let high = percentile(&mut self.light, 0.90);
        if !(0.12..0.43).contains(&dark) || !(0.14..0.51).contains(&(high - low)) || high >= 0.85 {
            return 0.0;
        }
        // The local minima omit the edges of the frame. Check the full sample too:
        // removing a veil subtracts light and can crush already-dark colors there.
        let floor = percentile(&mut self.minima, 0.02);
        let headroom = ((floor - 0.05) / 0.05).clamp(0.0, 1.0);
        (((dark - 0.12) * 108.0).clamp(0.0, 24.0) * headroom).round()
    }
}

fn cross(image: &Image<'_>, x: usize, y: usize, r: usize) -> Option<[[f32; 3]; 4]> {
    Some(
        [image.rgb(x.checked_sub(r)?, y)?, image.rgb(x.checked_add(r)?, y)?, image.rgb(x, y.checked_sub(r)?)?, image.rgb(x, y.checked_add(r)?)?]
            .map(|rgb| rgb.map(|v| v.clamp(0.0, 1.0))),
    )
}

#[derive(Default)]
struct Detail {
    count: usize,
    structured: usize,
    fine: usize,
    coarse: usize,
    banded: usize,
    flat: usize,
    flat_energy: f32,
}

impl Detail {
    fn add(&mut self, center: f32, fine: [f32; 4], coarse: [f32; 4]) {
        if !(0.15..=0.85).contains(&center) {
            return;
        }
        self.count += 1;
        let fine = curvature(center, fine);
        let medium = curvature(center, coarse);
        let (fine_strength, medium_strength) = (fine[0].abs().max(fine[1].abs()), medium[0].abs().max(medium[1].abs()));
        self.structured += usize::from(medium_strength >= 0.010 && fine_strength < medium_strength * 0.65);
        let edge = coarse.iter().any(|&v| (v - center).abs() > 0.14);
        // Suppress one-pixel noise by looking for coherent structure at a wider scale.
        if !edge && (0.010..0.060).contains(&medium_strength) && fine_strength < medium_strength * 0.56 {
            self.coarse += 1;
        }
        if !edge && (0.004..0.040).contains(&fine_strength) && coherent(fine, medium) {
            self.fine += 1;
        }
        if medium_strength < 0.007 {
            self.flat += 1;
            self.flat_energy += fine_strength;
        }
        // Two opposing terraces with a small jump indicate quantization in a smooth ramp.
        // One large step or isolated edges are rejected by the subsequent coverage check.
        self.banded += usize::from(band(center, [coarse[0], coarse[1]], fine[0]) || band(center, [coarse[2], coarse[3]], fine[1]));
    }

    fn apply(&self, out: &mut CameraRaw, shadow_noise: f32) {
        let count = self.count.max(1) as f32;
        let noise = self.flat_energy / self.flat.max(1) as f32;
        let structure = (self.coarse as f32 / count - 0.08).max(0.0);
        // A gradual roll-off keeps images with lots of structure from clustering
        // at the same setting. Native noise weakens both detail boosts.
        out.clarity = (22.0 * structure / (structure + 0.22) * (1.0 - 0.75 * shadow_noise)).round();
        let texture = ((self.fine as f32 / count - 0.07) * 100.0 * (1.0 - noise / 0.015).max(0.0)).clamp(0.0, 16.0);
        out.texture = (texture * (1.0 - 0.90 * shadow_noise)).round();
        let bands = (self.banded as f32 / count - 0.045).max(0.0);
        let grain = (120.0 * bands * (1.0 - noise / 0.006).max(0.0)).clamp(0.0, 15.0);
        out.grain_amount = (grain * (1.0 - shadow_noise)).round();
        let active = out.grain_amount > 0.0;
        out.grain_size = if active { (16.0 + 30.0 * bands).clamp(16.0, 25.0).round() } else { 25.0 };
        out.grain_roughness = if active { (42.0 + 36.0 * bands).clamp(42.0, 55.0).round() } else { 50.0 };
    }
}

fn curvature(center: f32, [left, right, top, bottom]: [f32; 4]) -> [f32; 2] {
    [center - (left + right) * 0.5, center - (top + bottom) * 0.5]
}

fn coherent(fine: [f32; 2], medium: [f32; 2]) -> bool {
    fine.into_iter().zip(medium).any(|(f, m)| f * m > 0.0 && f.abs() <= m.abs() * 0.75 && m.abs() >= 0.012)
}

fn band(center: f32, [left, right]: [f32; 2], fine: f32) -> bool {
    let (a, b) = (center - left, right - center);
    let spread = (a + b).abs();
    a * b >= 0.0 && a.abs().min(b.abs()) < 0.0004 && (0.005..0.035).contains(&spread) && (0.0015..0.010).contains(&fine.abs())
}

#[derive(Clone, Copy, Default)]
struct Bin {
    sum: f32,
    squares: f32,
    count: f32,
}

impl Bin {
    fn add(&mut self, luma: f32) {
        self.sum += luma;
        self.squares += luma * luma;
        self.count += 1.0;
    }

    fn estimate(self) -> Option<(f32, f32)> {
        (self.count >= 12.0).then(|| {
            let mean = self.sum / self.count;
            (mean, (self.squares / self.count - mean * mean).max(0.0).sqrt())
        })
    }
}

#[derive(Default)]
struct Radial {
    center: Bin,
    middle: [Bin; 4],
    corners: [Bin; 4],
}

impl Radial {
    fn add(&mut self, x: f32, y: f32, luma: f32) {
        let (x, y) = (2.0 * x - 1.0, 2.0 * y - 1.0);
        let radius = x.hypot(y);
        let quadrant = usize::from(x > 0.0) + 2 * usize::from(y > 0.0);
        match radius {
            r if r < 0.35 => self.center.add(luma),
            r if (0.58..0.82).contains(&r) => self.middle[quadrant].add(luma),
            r if r > 1.04 => self.corners[quadrant].add(luma),
            _ => {}
        }
    }

    fn lift(&self) -> Option<(f32, f32)> {
        let (center, center_std) = self.center.estimate()?;
        let [Some(m0), Some(m1), Some(m2), Some(m3)] = self.middle.map(Bin::estimate) else { return None };
        let [Some(c0), Some(c1), Some(c2), Some(c3)] = self.corners.map(Bin::estimate) else { return None };
        let middle = [m0, m1, m2, m3];
        let corners = [c0, c1, c2, c3];
        let average = |bins: &[(f32, f32); 4]| bins.iter().map(|&(mean, _)| mean).sum::<f32>() * 0.25;
        let mid = average(&middle);
        let corner = average(&corners);
        let drop = center - corner;
        if !(0.25..0.75).contains(&center) || corner < 0.12 || drop < 0.065 || mid > center - 0.018 || mid < corner + 0.018 {
            return None;
        }
        let spread = |bins: &[(f32, f32); 4]| {
            let low = bins.iter().map(|&(mean, _)| mean).fold(1.0, f32::min);
            let high = bins.iter().map(|&(mean, _)| mean).fold(0.0, f32::max);
            high - low
        };
        let deviation = middle.iter().chain(&corners).map(|&(_, std)| std).fold(center_std, f32::max);
        // Four dark corners can come from the scene itself. Require the ring between
        // the centre and the corners to agree across directions as well.
        if spread(&middle).max(spread(&corners)) > drop * 0.35 || deviation > drop * 0.55 {
            return None;
        }
        let intensity = (center / corner).log2() * 35.0;
        let midpoint = 26.0 + 44.0 * (center - mid) / drop;
        Some((intensity.clamp(0.0, 20.0).round(), midpoint.clamp(32.0, 62.0).round()))
    }

    fn apply(&self, out: &mut CameraRaw) {
        let (amount, midpoint) = self.lift().unwrap_or((0.0, 50.0));
        out.vignette_amount = amount;
        out.vignette_midpoint = midpoint;
        // A radial falloff estimate assumes a soft, elliptical footprint aligned to
        // the frame. Highlight recovery applies only to a darkening vignette.
        out.vignette_feather = if amount > 0.0 { 72.0 } else { 50.0 };
        out.vignette_roundness = 0.0;
        out.vignette_highlights = 0.0;
        out.vignette_style = super::super::StoredVignetteStyle::HighlightPriority;
    }
}
