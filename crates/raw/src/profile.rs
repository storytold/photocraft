//! The rendering half of a DNG camera profile (Adobe DNG Specification 1.7, chapter 6 and the
//! tag descriptions of ProfileHueSatMapDims / Data1 / Data2, ProfileHueSatMapEncoding,
//! ProfileLookTableDims / Data, ProfileLookTableEncoding and ProfileToneCurve): hue / saturation /
//! value maps applied to linear ProPhoto RGB, and the default tone curve.
//!
//! A map is a table of `[hue shift in degrees, saturation scale, value scale]` indexed by hue,
//! saturation and value of the HSV form of the colour, value outermost, then hue, then saturation.
//! Hue divisions wrap around the colour circle; saturation divisions span 0..1 inclusive, as do
//! value divisions (on linear values, or on sRGB-encoded ones when the encoding tag is 1). Entries
//! are interpolated linearly in each dimension.

/// Most table entries accepted (the largest profiles in use have a few thousand).
const MAX_ENTRIES: usize = 1 << 16;

/// A hue / saturation / value map (ProfileHueSatMap or ProfileLookTable).
#[derive(Debug, Clone, PartialEq)]
pub struct HsvMap {
    hue_divs: usize,
    sat_divs: usize,
    val_divs: usize,
    /// `[hue shift (degrees), saturation scale, value scale]`, value outermost, saturation innermost.
    data: Vec<[f32; 3]>,
    /// The value axis indexes sRGB-encoded values (encoding 1) instead of linear ones.
    srgb_value: bool,
}

impl HsvMap {
    /// The number of values (three per entry) the dimensions tag `dims` calls for, when it is a
    /// shape [`HsvMap::new`] accepts: checked before the data is read.
    pub(crate) fn values_for(dims: &[u32]) -> Option<usize> {
        let [h, s, v] = dims else { return None };
        let (hue_divs, sat_divs, val_divs) = (*h as usize, *s as usize, *v as usize);
        // The value axis may have one division (the map ignores value); saturation needs two.
        if hue_divs == 0 || sat_divs < 2 || val_divs == 0 {
            return None;
        }
        let entries = hue_divs.checked_mul(sat_divs)?.checked_mul(val_divs)?;
        (entries <= MAX_ENTRIES).then(|| entries.checked_mul(3)).flatten()
    }

    /// A map from its dimensions tag, its data and its encoding tag (0 linear, 1 sRGB); `None`
    /// when the dimensions and data don't agree or hold a value the specification excludes
    /// (zero-saturation entries must have a value scale of 1.0, so neutrals keep their value), or
    /// a map with a value axis has an encoding the specification doesn't define.
    pub fn new(dims: &[u32], data: &[f64], encoding: u32) -> Option<HsvMap> {
        let [h, s, v] = dims else { return None };
        let (hue_divs, sat_divs, val_divs) = (*h as usize, *s as usize, *v as usize);
        if data.len() != Self::values_for(dims)? || (val_divs > 1 && encoding > 1) {
            return None;
        }
        let data: Vec<[f32; 3]> = data
            .as_chunks::<3>()
            .0
            .iter()
            .enumerate()
            .map(|(i, e)| {
                // Finite as f32 too: a huge f64 narrows to infinity.
                let v = [e[0] as f32, e[1] as f32, e[2] as f32];
                // Saturation is the innermost loop, so entry `i` is at saturation `i % sat_divs`.
                // A tolerance for the rounding of a value written as 1.0.
                let neutral_ok = i % sat_divs != 0 || (v[2] - 1.0).abs() <= 1e-5;
                let ok = v.iter().all(|x| x.is_finite()) && v[1] >= 0.0 && v[2] >= 0.0 && neutral_ok;
                ok.then_some(v)
            })
            .collect::<Option<_>>()?;
        Some(HsvMap { hue_divs, sat_divs, val_divs, data, srgb_value: encoding == 1 })
    }

    fn same_shape(&self, o: &HsvMap) -> bool {
        self.hue_divs == o.hue_divs && self.sat_divs == o.sat_divs && self.val_divs == o.val_divs && self.srgb_value == o.srgb_value
    }

    /// `self · g + other · (1 − g)`, the DNG interpolation of the two illuminants' maps. Maps of
    /// different shapes can't be interpolated: the one with the larger weight is used (`other`
    /// from an even split on).
    pub(crate) fn blend(&self, other: &HsvMap, g: f64) -> HsvMap {
        let g = if g.is_nan() { 1.0 } else { g.clamp(0.0, 1.0) };
        if !self.same_shape(other) {
            return if g > 0.5 { self.clone() } else { other.clone() };
        }
        let g = g as f32;
        let data = self.data.iter().zip(&other.data).map(|(a, b)| [0, 1, 2].map(|k| a[k] * g + b[k] * (1.0 - g))).collect();
        HsvMap { data, ..self.clone() }
    }

    fn entry(&self, v: usize, h: usize, s: usize) -> [f32; 3] {
        let i = (v * self.hue_divs + h) * self.sat_divs + s;
        self.data.get(i).copied().unwrap_or([0.0, 1.0, 1.0])
    }

    /// The map's `[hue shift, saturation scale, value scale]` at hue `h6` (0..6, sextants),
    /// saturation `s` and value `v` (both 0..1).
    fn lookup(&self, h6: f32, s: f32, v: f32) -> [f32; 3] {
        let hf = h6 * self.hue_divs as f32 / 6.0;
        let h0 = (hf.floor() as usize) % self.hue_divs;
        let h1 = (h0 + 1) % self.hue_divs;
        let ht = hf - hf.floor();
        let sf = s.clamp(0.0, 1.0) * (self.sat_divs - 1) as f32;
        let s0 = (sf as usize).min(self.sat_divs - 2);
        let st = sf - s0 as f32;
        let (v0, v1, vt) = if self.val_divs > 1 {
            let vv = if self.srgb_value { srgb_encode(v) } else { v };
            let vf = vv.clamp(0.0, 1.0) * (self.val_divs - 1) as f32;
            let v0 = (vf as usize).min(self.val_divs - 2);
            (v0, v0 + 1, vf - v0 as f32)
        } else {
            (0, 0, 0.0)
        };
        // As a weighted sum: `b − a` overflows for finite entries of opposite sign near f32::MAX.
        let lerp = |a: [f32; 3], b: [f32; 3], t: f32| [0, 1, 2].map(|k| a[k] * (1.0 - t) + b[k] * t);
        let plane = |vi: usize| {
            let a = lerp(self.entry(vi, h0, s0), self.entry(vi, h0, s0 + 1), st);
            let b = lerp(self.entry(vi, h1, s0), self.entry(vi, h1, s0 + 1), st);
            lerp(a, b, ht)
        };
        if v0 == v1 { plane(v0) } else { lerp(plane(v0), plane(v1), vt) }
    }

    /// Applies the map to a linear ProPhoto RGB colour. A negative (out-of-gamut) channel is
    /// left out of the colour the map sees and kept as it is, so it can't shift the others.
    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        if rgb.iter().any(|v| *v < 0.0) {
            let out = self.apply_nonnegative(rgb.map(|v| v.max(0.0)));
            return std::array::from_fn(|k| out[k] + rgb[k].min(0.0));
        }
        self.apply_nonnegative(rgb)
    }

    fn apply_nonnegative(&self, rgb: [f32; 3]) -> [f32; 3] {
        let [r, g, b] = rgb;
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        if max.is_nan() || max <= 0.0 || !max.is_finite() {
            return rgb;
        }
        let delta = max - min;
        let s = delta / max;
        let h6 = if delta <= 0.0 {
            0.0
        } else if max == r {
            ((g - b) / delta).rem_euclid(6.0)
        } else if max == g {
            (b - r) / delta + 2.0
        } else {
            (r - g) / delta + 4.0
        };
        let [shift, sat_scale, val_scale] = self.lookup(h6, s, max);
        let h = (h6 + shift / 60.0).rem_euclid(6.0);
        let s = (s * sat_scale).clamp(0.0, 1.0);
        let v = max * val_scale;
        hsv_to_rgb(h, s, v)
    }
}

/// HSV (hue in sextants 0..6) → RGB.
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let i = (h.floor() as i32).rem_euclid(6);
    let f = h - h.floor();
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    match i {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

fn srgb_encode(v: f32) -> f32 {
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// The rendering tags of a DNG camera profile.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Profile {
    /// ProfileHueSatMapData1 / 2 with their calibration illuminant's temperature, sorted by it.
    pub hue_sat: Vec<(f64, HsvMap)>,
    /// ProfileLookTableData.
    pub look_table: Option<HsvMap>,
}

impl Profile {
    /// The hue / saturation map at white temperature `t`, interpolated in inverse temperature
    /// between the two illuminants' maps like the colour matrices.
    pub(crate) fn hue_sat_at(&self, t: f64) -> Option<HsvMap> {
        match self.hue_sat.as_slice() {
            [] => None,
            [(_, m)] => Some(m.clone()),
            [(t1, a), (t2, b), ..] => {
                let g = if (t1 - t2).abs() <= 1.0 || t <= *t1 {
                    1.0
                } else if t >= *t2 {
                    0.0
                } else {
                    (1.0 / t - 1.0 / t2) / (1.0 / t1 - 1.0 / t2)
                };
                Some(a.blend(b, g))
            }
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.hue_sat.is_empty() && self.look_table.is_none()
    }
}

/// Points of the dense tone curve made from a ProfileToneCurve.
const CURVE_POINTS: usize = 1024;

/// A ProfileToneCurve (flat `[in, out, in, out, …]` pairs, 0..1, starting at (0, 0) and ending
/// at (1, 1)) interpolated by a natural cubic spline, sampled as increasing `[input, output]`
/// pairs for [`crate::sensor::Sensor::tone_curve`]. The curve maps linear values: a DJI FC9284
/// DNG developed this way matches Camera Raw far better than with the curve on gamma-1.8 values
/// (measured 2026-11-03). `None` when the tag is malformed.
pub(crate) fn tone_curve(values: &[f64]) -> Option<Vec<[f32; 2]>> {
    if values.len() < 4 || !values.len().is_multiple_of(2) || values.len() > 2 * 4096 {
        return None;
    }
    let pts: Vec<(f64, f64)> = values.as_chunks::<2>().0.iter().map(|p| (p[0], p[1])).collect();
    // The specification's endpoints: a curve that doesn't keep black and white is malformed.
    if pts.first() != Some(&(0.0, 0.0)) || pts.last() != Some(&(1.0, 1.0)) {
        return None;
    }
    if pts.iter().any(|(x, y)| !x.is_finite() || !y.is_finite() || !(0.0..=1.0).contains(x) || !(0.0..=1.0).contains(y))
        || pts.windows(2).any(|w| w[1].0 <= w[0].0)
    {
        return None;
    }
    let spline = Spline::new(&pts)?;
    let mut out = Vec::with_capacity(CURVE_POINTS + 1);
    for i in 0..=CURVE_POINTS {
        let x = i as f32 / CURVE_POINTS as f32;
        out.push([x, spline.eval(f64::from(x)).clamp(0.0, 1.0) as f32]);
    }
    Some(out)
}

/// A natural cubic spline through increasing points.
struct Spline {
    x: Vec<f64>,
    y: Vec<f64>,
    /// Second derivatives at the points.
    m: Vec<f64>,
}

impl Spline {
    fn new(pts: &[(f64, f64)]) -> Option<Spline> {
        let n = pts.len();
        if n < 2 {
            return None;
        }
        let x: Vec<f64> = pts.iter().map(|p| p.0).collect();
        let y: Vec<f64> = pts.iter().map(|p| p.1).collect();
        let mut m = vec![0.0; n];
        if n > 2 {
            // Tridiagonal system for the interior second derivatives (Thomas algorithm).
            let mut c = vec![0.0; n];
            let mut d = vec![0.0; n];
            for i in 1..n - 1 {
                let h0 = x[i] - x[i - 1];
                let h1 = x[i + 1] - x[i];
                let a = h0 / 6.0;
                let b = (h0 + h1) / 3.0;
                let cc = h1 / 6.0;
                let r = (y[i + 1] - y[i]) / h1 - (y[i] - y[i - 1]) / h0;
                let denom = b - a * c[i - 1];
                if denom.abs() < 1e-15 {
                    return None;
                }
                c[i] = cc / denom;
                d[i] = (r - a * d[i - 1]) / denom;
            }
            for i in (1..n - 1).rev() {
                m[i] = d[i] - c[i] * m[i + 1];
            }
        }
        m.iter().all(|v| v.is_finite()).then_some(Spline { x, y, m })
    }

    fn eval(&self, t: f64) -> f64 {
        let n = self.x.len();
        let i = self.x.partition_point(|&v| v <= t).clamp(1, n - 1);
        let (x0, x1) = (self.x[i - 1], self.x[i]);
        let h = x1 - x0;
        let a = (x1 - t) / h;
        let b = (t - x0) / h;
        a * self.y[i - 1] + b * self.y[i] + ((a * a * a - a) * self.m[i - 1] + (b * b * b - b) * self.m[i]) * h * h / 6.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(h: u32, s: u32, v: u32) -> Vec<f64> {
        (0..h * s * v).flat_map(|_| [0.0, 1.0, 1.0]).collect()
    }

    #[test]
    fn identity_map_changes_nothing() {
        let m = HsvMap::new(&[6, 3, 1], &identity(6, 3, 1), 0).unwrap();
        for c in [[0.2f32, 0.4, 0.6], [0.9, 0.1, 0.1], [0.5, 0.5, 0.5], [0.0, 0.0, 0.0]] {
            let o = m.apply(c);
            assert!(o.iter().zip(c).all(|(a, b)| (a - b).abs() < 1e-5), "{c:?} -> {o:?}");
        }
    }

    #[test]
    fn saturation_and_value_scales_apply() {
        // Saturation halved everywhere; value kept at saturation 0 (as the specification
        // requires) and doubled at full saturation.
        let data: Vec<f64> = (0..6).flat_map(|_| [0.0, 0.5, 1.0, 0.0, 0.5, 2.0]).collect();
        let m = HsvMap::new(&[6, 2, 1], &data, 0).unwrap();
        let o = m.apply([0.4, 0.2, 0.2]);
        // HSV (0°, 0.5, 0.4), value scale 1.5 halfway → (0°, 0.25, 0.6): (0.6, 0.45, 0.45).
        assert!((o[0] - 0.6).abs() < 1e-5 && (o[1] - 0.45).abs() < 1e-5 && (o[2] - 0.45).abs() < 1e-5, "{o:?}");
        // Grey (saturation 0) is left as it is.
        let g = m.apply([0.3, 0.3, 0.3]);
        assert!(g.iter().all(|v| (v - 0.3).abs() < 1e-5), "{g:?}");
    }

    #[test]
    fn hue_shift_wraps() {
        // +120° everywhere: red becomes green.
        let data: Vec<f64> = (0..4 * 2).flat_map(|_| [120.0, 1.0, 1.0]).collect();
        let m = HsvMap::new(&[4, 2, 1], &data, 0).unwrap();
        let o = m.apply([0.5, 0.0, 0.0]);
        assert!(o[0].abs() < 1e-5 && (o[1] - 0.5).abs() < 1e-5 && o[2].abs() < 1e-5, "{o:?}");
    }

    #[test]
    fn negative_channels_pass_through_an_identity_map() {
        let m = HsvMap::new(&[6, 3, 1], &identity(6, 3, 1), 0).unwrap();
        for c in [[1.0f32, 0.0, -1.0], [0.6, 0.3, -0.05], [-0.2, 0.5, 0.1]] {
            let o = m.apply(c);
            assert!(o.iter().zip(c).all(|(a, b)| (a - b).abs() < 1e-5), "{c:?} -> {o:?}");
        }
    }

    /// The specification requires a value scale of 1.0 at zero saturation: a map that would
    /// brighten or darken neutrals is malformed.
    #[test]
    fn zero_saturation_entries_keep_the_value() {
        for dims in [[6u32, 2, 1], [6, 3, 2]] {
            let mut bad = identity(dims[0], dims[1], dims[2]);
            let last_zero_sat = (dims.iter().product::<u32>() - dims[1]) as usize;
            bad[last_zero_sat * 3 + 2] = 2.0;
            assert!(HsvMap::new(&dims, &bad, 0).is_none(), "{dims:?}");
            // Other saturations may scale the value.
            let mut ok = identity(dims[0], dims[1], dims[2]);
            ok[(last_zero_sat + 1) * 3 + 2] = 2.0;
            assert!(HsvMap::new(&dims, &ok, 0).is_some(), "{dims:?}");
        }
    }

    /// Hue shifts of opposite sign near f32::MAX side by side: their difference overflows, and
    /// the interpolation must still give a finite colour.
    #[test]
    fn extreme_neighbouring_entries_interpolate_without_overflow() {
        let mut data = identity(2, 2, 1);
        for (i, shift) in [(0, 3e38), (1, 3e38), (2, -3e38), (3, -3e38)] {
            data[i * 3] = shift;
        }
        let m = HsvMap::new(&[2, 2, 1], &data, 0).unwrap();
        for c in [[0.5f32, 0.0, 0.0], [0.5, 0.25, 0.25], [0.0, 0.5, 0.5], [0.5, 0.5, 0.0]] {
            let o = m.apply(c);
            assert!(o.iter().all(|v| v.is_finite()), "{c:?} -> {o:?}");
        }
    }

    /// With encoding 1 the value axis is indexed by the sRGB-encoded value: linear 0.2 sits at
    /// encoded 0.4845, so the value scale there is about 1.48, not the 1.2 a linear index gives.
    #[test]
    fn the_encoded_value_axis_is_looked_up_encoded() {
        // Value divisions at 0 and 1 (value scale 1 → 2), saturation divisions at 0 and 1.
        let data: Vec<f64> = [[0.0, 1.0, 1.0], [0.0, 1.0, 1.0], [0.0, 1.0, 1.0], [0.0, 1.0, 2.0]].iter().flat_map(|e| e.iter().copied()).collect();
        let (srgb, linear) = (HsvMap::new(&[1, 2, 2], &data, 1).unwrap(), HsvMap::new(&[1, 2, 2], &data, 0).unwrap());
        // Fully saturated red at linear value 0.2.
        let scale = |m: &HsvMap| m.apply([0.2, 0.0, 0.0])[0] / 0.2;
        let encoded = srgb_encode(0.2);
        assert!((scale(&srgb) - (1.0 + encoded)).abs() < 1e-4, "{}", scale(&srgb));
        assert!((scale(&linear) - 1.2).abs() < 1e-4, "{}", scale(&linear));
    }

    #[test]
    fn entries_too_large_for_f32_are_rejected() {
        let mut data = identity(6, 2, 1);
        data[2] = 1e300;
        assert!(HsvMap::new(&[6, 2, 1], &data, 0).is_none());
    }

    #[test]
    fn value_axes_need_a_defined_encoding() {
        assert!(HsvMap::new(&[6, 2, 3], &identity(6, 2, 3), 2).is_none());
        assert!(HsvMap::new(&[6, 2, 3], &identity(6, 2, 3), 1).is_some());
        // Without a value axis the encoding does not matter.
        assert!(HsvMap::new(&[6, 2, 1], &identity(6, 2, 1), 2).is_some());
    }

    #[test]
    fn tone_curves_keep_black_and_white() {
        assert!(tone_curve(&[0.0, 0.0, 0.5, 0.6, 1.0, 1.0]).is_some());
        assert!(tone_curve(&[0.0, 0.2, 1.0, 0.8]).is_none());
        assert!(tone_curve(&[0.0, 0.0, 0.5, 0.6, 1.0, 0.9]).is_none());
        assert!(tone_curve(&[0.1, 0.0, 1.0, 1.0]).is_none());
    }

    #[test]
    fn malformed_maps_are_rejected() {
        assert!(HsvMap::new(&[6, 1, 1], &identity(6, 1, 1), 0).is_none());
        assert!(HsvMap::new(&[0, 2, 1], &[], 0).is_none());
        assert!(HsvMap::new(&[6, 2, 1], &identity(6, 2, 2), 0).is_none());
        assert!(HsvMap::new(&[6, 2], &identity(6, 2, 1), 0).is_none());
        assert!(HsvMap::new(&[u32::MAX, u32::MAX, u32::MAX], &[], 0).is_none());
        let mut bad = identity(6, 2, 1);
        bad[0] = f64::NAN;
        assert!(HsvMap::new(&[6, 2, 1], &bad, 0).is_none());
        // Hostile colours do not panic.
        let m = HsvMap::new(&[6, 2, 3], &identity(6, 2, 3), 1).unwrap();
        for c in [[f32::NAN, 0.0, 0.0], [f32::INFINITY, 1.0, 0.0], [-1.0, -2.0, -3.0], [-0.5, 0.5, 2.0], [1e30, 0.0, 0.0]] {
            let _ = m.apply(c);
        }
    }

    #[test]
    fn maps_blend_by_temperature() {
        let a: Vec<f64> = (0..6 * 2).flat_map(|_| [0.0, 1.0, 1.0]).collect();
        let b: Vec<f64> = (0..6 * 2).flat_map(|_| [0.0, 2.0, 1.0]).collect();
        let p =
            Profile { hue_sat: vec![(2856.0, HsvMap::new(&[6, 2, 1], &a, 0).unwrap()), (6504.0, HsvMap::new(&[6, 2, 1], &b, 0).unwrap())], look_table: None };
        let at = |t: f64| p.hue_sat_at(t).unwrap().lookup(0.0, 0.5, 0.5)[1];
        assert!((at(2000.0) - 1.0).abs() < 1e-6 && (at(9000.0) - 2.0).abs() < 1e-6);
        let mid = at(4000.0);
        assert!(mid > 1.0 && mid < 2.0, "{mid}");
    }

    /// Maps of different shapes can't be interpolated: the white's nearer illuminant wins,
    /// rather than always the first.
    #[test]
    fn differently_shaped_maps_take_the_nearer_illuminant() {
        let a: Vec<f64> = (0..6 * 2).flat_map(|_| [0.0, 1.0, 1.0]).collect();
        let b: Vec<f64> = (0..4 * 2).flat_map(|_| [0.0, 2.0, 1.0]).collect();
        let p =
            Profile { hue_sat: vec![(2856.0, HsvMap::new(&[6, 2, 1], &a, 0).unwrap()), (6504.0, HsvMap::new(&[4, 2, 1], &b, 0).unwrap())], look_table: None };
        let at = |t: f64| p.hue_sat_at(t).unwrap().lookup(0.0, 0.5, 0.5)[1];
        assert!((at(2000.0) - 1.0).abs() < 1e-6 && (at(3000.0) - 1.0).abs() < 1e-6);
        assert!((at(6000.0) - 2.0).abs() < 1e-6 && (at(9000.0) - 2.0).abs() < 1e-6);
    }

    #[test]
    fn tone_curve_is_sampled_and_validated() {
        let c = tone_curve(&[0.0, 0.0, 0.5, 0.75, 1.0, 1.0]).unwrap();
        assert_eq!(c.first(), Some(&[0.0, 0.0]));
        assert!(c.last().is_some_and(|p| (p[0] - 1.0).abs() < 1e-6 && (p[1] - 1.0).abs() < 1e-6));
        let mid = c.iter().find(|p| (p[0] - 0.5).abs() < 1e-3).unwrap();
        assert!((mid[1] - 0.75).abs() < 1e-3, "{mid:?}");
        assert!(c.windows(2).all(|w| w[1][0] > w[0][0]));
        assert!(tone_curve(&[0.0, 0.0, 0.5]).is_none());
        assert!(tone_curve(&[0.0, 0.0, 0.5, 0.2, 0.4, 0.3, 1.0, 1.0]).is_none());
        assert!(tone_curve(&[0.0, 0.0, f64::NAN, 0.5, 1.0, 1.0]).is_none());
        assert!(tone_curve(&[0.0, 0.0, 2.0, 0.5]).is_none());
    }
}
