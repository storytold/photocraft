//! DNG chapter 6 HSV tables, sRGB value indexing and a shape-preserving cubic tone curve.
use super::CameraProfile;
use crate::RawError;

#[derive(Debug, Clone, PartialEq)]
pub struct HueSatMap {
    pub dims: [usize; 3],
    pub srgb_value: bool,
    pub values: Vec<[f32; 3]>,
}
fn encode(v: f32) -> f32 {
    if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}
fn decode(v: f32) -> f32 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}
fn hsv([r, g, b]: [f32; 3]) -> [f32; 3] {
    let v = r.max(g).max(b);
    let d = v - r.min(g).min(b);
    if d <= 1e-10 {
        return [0.0, 0.0, v];
    }
    let h = if v == r {
        (g - b) / d
    } else if v == g {
        2.0 + (b - r) / d
    } else {
        4.0 + (r - g) / d
    };
    [(h / 6.0).rem_euclid(1.0), d / v.max(1e-10), v]
}
fn rgb([h, s, v]: [f32; 3]) -> [f32; 3] {
    let h = h.rem_euclid(1.0) * 6.0;
    let f = h.fract();
    let a = v * (1.0 - s);
    let b = v * (1.0 - s * f);
    let c = v * (1.0 - s * (1.0 - f));
    match h as u32 {
        0 => [v, c, a],
        1 => [b, v, a],
        2 => [a, v, c],
        3 => [a, b, v],
        4 => [c, a, v],
        _ => [v, a, b],
    }
}
impl HueSatMap {
    pub(crate) fn validate(&self) -> Result<(), RawError> {
        let [h, s, v] = self.dims;
        let n = h.checked_mul(s).and_then(|n| n.checked_mul(v));
        if !(1..=360).contains(&h) || !(2..=128).contains(&s) || !(1..=128).contains(&v) || n != Some(self.values.len()) || self.values.len() > 1_048_576 {
            return Err(RawError::malformed("invalid DCP HSV table dimensions"));
        }
        for [shift, sat, value] in &self.values {
            if !shift.is_finite()
                || shift.abs() > 720.0
                || !sat.is_finite()
                || !(0.0..=1024.0).contains(sat)
                || !value.is_finite()
                || !(0.0..=1024.0).contains(value)
            {
                return Err(RawError::malformed("invalid DCP HSV table coefficient"));
            }
        }
        Ok(())
    }
    fn at(&self, h: usize, s: usize, v: usize) -> [f32; 3] {
        let [nh, ns, _] = self.dims;
        let i = v.checked_mul(nh).and_then(|n| n.checked_add(h)).and_then(|n| n.checked_mul(ns)).and_then(|n| n.checked_add(s));
        i.and_then(|i| self.values.get(i)).copied().unwrap_or([0.0, 1.0, 1.0])
    }
    fn sample(&self, [h, s, v]: [f32; 3]) -> [f32; 3] {
        let [nh, ns, nv] = self.dims;
        let axis = |x: f32, n: usize| {
            let f = x.clamp(0.0, 1.0) * n.saturating_sub(1) as f32;
            let a = f.floor() as usize;
            (a, (a + 1).min(n.saturating_sub(1)), f - a as f32)
        };
        let hp = h.rem_euclid(1.0) * nh as f32;
        let ha = (hp.floor() as usize).min(nh.saturating_sub(1));
        let hb = if ha + 1 == nh { 0 } else { ha + 1 };
        let (sa, sb, sw) = axis(s, ns);
        let (va, vb, vw) = axis(v, nv);
        let hw = hp - ha as f32;
        let mut out = [0.0; 3];
        for (h, wh) in [(ha, 1.0 - hw), (hb, hw)] {
            for (s, ws) in [(sa, 1.0 - sw), (sb, sw)] {
                for (v, wv) in [(va, 1.0 - vw), (vb, vw)] {
                    for (o, x) in out.iter_mut().zip(self.at(h, s, v)) {
                        *o += x * wh * ws * wv;
                    }
                }
            }
        }
        out
    }
    fn apply(&self, input: [f32; 3], other: Option<(&HueSatMap, f32)>) -> [f32; 3] {
        let mut c = hsv(input.map(|v| v.max(0.0)));
        let encoded = self.srgb_value && self.dims[2] > 1;
        if encoded {
            c[2] = encode(c[2]);
        }
        let mut delta = self.sample(c);
        if let Some((other, w)) = other {
            let b = other.sample(c);
            for (a, b) in delta.iter_mut().zip(b) {
                *a = *a * w + b * (1.0 - w);
            }
        }
        c[0] += delta[0] / 360.0;
        c[1] = (c[1] * delta[1]).clamp(0.0, 1.0);
        c[2] = (c[2] * delta[2]).clamp(0.0, 1.0);
        if encoded {
            c[2] = decode(c[2]);
        }
        rgb(c)
    }
}

pub(crate) struct Prepared<'a> {
    profile: &'a CameraProfile,
    weight: f32,
    gain: f32,
    tone: Vec<f32>,
}
impl<'a> Prepared<'a> {
    pub(super) fn new(profile: &'a CameraProfile, weight: f32, gain: f32) -> Self {
        let points = &profile.tone_curve;
        let mut slopes = vec![0.0; points.len()];
        for (i, m) in slopes.iter_mut().enumerate() {
            let before = i.checked_sub(1).and_then(|j| points.get(j)).zip(points.get(i));
            let after = points.get(i).zip(points.get(i + 1));
            let slope = |(a, b): (&[f64; 2], &[f64; 2])| (b[1] - a[1]) / (b[0] - a[0]);
            *m = match (before, after) {
                (Some(a), Some(b)) => {
                    let (da, db) = (slope(a), slope(b));
                    if da * db <= 0.0 { 0.0 } else { 2.0 * da * db / (da + db) }
                }
                (Some(a), None) => slope(a),
                (None, Some(b)) => slope(b),
                _ => 1.0,
            };
        }
        let tone = (0..=65536)
            .map(|i| {
                let x = i as f64 / 65536.0;
                let j = points.partition_point(|p| p[0] < x);
                if points.is_empty() {
                    return x as f32;
                }
                let Some(([ax, ay], [bx, by])) = j.checked_sub(1).and_then(|a| points.get(a)).zip(points.get(j)) else {
                    return if j == 0 { points.first().map_or(x, |p| p[1]) } else { points.last().map_or(x, |p| p[1]) } as f32;
                };
                let d = bx - ax;
                let t = (x - ax) / d;
                let (a, b) = (slopes.get(j - 1).copied().unwrap_or(1.0), slopes.get(j).copied().unwrap_or(1.0));
                ((2.0 * t * t * t - 3.0 * t * t + 1.0) * ay
                    + (t * t * t - 2.0 * t * t + t) * d * a
                    + (-2.0 * t * t * t + 3.0 * t * t) * by
                    + (t * t * t - t * t) * d * b)
                    .clamp(ay.min(*by), ay.max(*by)) as f32
            })
            .collect();
        Self { profile, weight, gain, tone }
    }
    pub(crate) fn render(&self, mut rgb: [f32; 3]) -> [f32; 3] {
        if let Some(first) = self.profile.hue_sat_maps.first() {
            rgb = first.apply(rgb, self.profile.hue_sat_maps.get(1).map(|b| (b, self.weight)));
        }
        rgb = rgb.map(|v| (v * self.gain).clamp(0.0, 1.0));
        if let Some(look) = &self.profile.look_table {
            rgb = look.apply(rgb, None);
        }
        rgb.map(|v| {
            let x = v.clamp(0.0, 1.0) * 65536.0;
            let i = x as usize;
            let a = self.tone.get(i).copied().unwrap_or(v);
            let b = self.tone.get(i + 1).copied().unwrap_or(a);
            a + (b - a) * (x - i as f32)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hsv_hue_shift_and_srgb_value_encoding_follow_the_profile() {
        let hue = HueSatMap { dims: [1, 2, 1], srgb_value: false, values: vec![[60.0, 1.0, 1.0]; 2] };
        hue.validate().unwrap();
        let p = hue.apply([0.5, 0.0, 0.0], None);
        assert!((p[0] - 0.5).abs() < 1e-6 && (p[1] - 0.5).abs() < 1e-6 && p[2] < 1e-6);
        let value = HueSatMap { dims: [1, 2, 2], srgb_value: true, values: vec![[0.0, 1.0, 1.0], [0.0, 1.0, 0.5], [0.0, 1.0, 1.0], [0.0, 1.0, 0.5]] };
        value.validate().unwrap();
        let p = value.apply([0.25, 0.0, 0.0], None);
        assert!((p[0] - decode(encode(0.25) * 0.5)).abs() < 1e-6);
    }
    #[test]
    fn cubic_tone_lookup_is_monotone_and_preserves_endpoints() {
        let profile = CameraProfile {
            name: "test".into(),
            camera_model: "test".into(),
            calibrations: Vec::new(),
            exposure_offset: 0.0,
            tone_curve: vec![[0.0, 0.0], [0.25, 0.5], [1.0, 1.0]],
            hue_sat_maps: Vec::new(),
            look_table: None,
            default_black_render: 1,
            embed_policy: 3,
            copyright: "Original synthetic test data".into(),
        };
        let p = Prepared::new(&profile, 1.0, 1.0);
        assert_eq!(p.render([0.0; 3]), [0.0; 3]);
        assert_eq!(p.render([1.0; 3]), [1.0; 3]);
        assert_eq!(p.render([0.25; 3]), [0.5; 3]);
        assert!(p.tone.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn clipped_domains_and_small_real_profile_reversals_do_not_panic_or_get_replaced() {
        let profile = CameraProfile {
            name: "synthetic clipped curve".into(),
            camera_model: "test".into(),
            calibrations: Vec::new(),
            exposure_offset: 0.0,
            tone_curve: vec![[0.0, 0.0], [0.2, 0.5], [0.3, 0.49], [0.9, 1.0]],
            hue_sat_maps: Vec::new(),
            look_table: None,
            default_black_render: 1,
            embed_policy: 3,
            copyright: "Original synthetic test data".into(),
        };
        let p = Prepared::new(&profile, 1.0, 1.0);
        assert!(p.tone.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
        assert!(p.render([0.2; 3])[0] > p.render([0.3; 3])[0]);
        assert_eq!(p.render([0.99; 3]), [1.0; 3]);
    }
}
