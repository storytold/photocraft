//! Measured corrections for Temperature, Tint and Whites down on rendered pixels (Vibrance and
//! Saturation are measured tables of their own, see [`presence`](super::presence)).
//!
//! Measured as a black box on Photoshop 2026's Camera Raw Filter (Camera Raw 18.6); no Adobe
//! code or data was used. A 33³ RGB cube went through the filter at every 5 of each slider:
//! one fixed function of the pixel's colour, the same in every image (applying the cube's
//! result to 17 photographs and renders matched Photoshop's own result on them to a mean ΔE76
//! of 0.4–1.5). The analytic models ([`white_balance`](super::white_balance), the Whites curve
//! in [`whites_blacks`](super::whites_blacks)) follow the
//! greys but miss how coloured pixels move, so each slider carries the difference, Photoshop's
//! cube minus the analytic model's, as a correction on a 17³ grid of sRGB-encoded colours,
//! interpolated trilinearly. Along the slider the corrections are low-rank: a few grid vectors
//! (8 bits each with a scale) and their weights at every 5, interpolated between (0 is no
//! correction). The tables are `data/camera_raw_colour.bin`, written by the measurement tools
//! (`export_cube.py`); when an analytic model changes they must be measured again.
//!
//! On 17 images (mean ΔE76, analytic model alone → with the correction): Temperature
//! −100 6.6 → 1.0, −50 4.8 → 0.7, +50 3.4 → 0.5, +100 4.7 → 0.6; Tint ±100 2.8–5.3 → 0.7;
//! Whites −50 0.9 → 0.2, −100 1.4 → 0.3 (also a fixed function of
//! the colour: the cube's own result gave 0.2–0.3).

use std::sync::OnceLock;

use super::Encoding;

/// Grid nodes per channel.
pub(crate) const N: usize = 17;
/// Values in one grid: RGB per node.
const CELLS: usize = N * N * N * 3;

static DATA: &[u8] = include_bytes!("../../data/camera_raw_colour.bin");

/// The sliders with a measured correction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Slider {
    Temperature = 0,
    Tint = 1,
    /// Whites down only (Whites up adapts to the image, so it has no fixed correction).
    Whites = 4,
}

/// One slider's correction on one side of 0.
struct Table {
    slider: u8,
    negative: bool,
    /// Slider magnitudes the weights were measured at, ascending (5, 10, … 100).
    at: Vec<f32>,
    /// Per magnitude, one weight per basis grid.
    weights: Vec<Vec<f32>>,
    /// Basis grids, `CELLS` values each.
    basis: Vec<Vec<f32>>,
}

/// Reads `b` (see `export_cube.py` in the measurement tools): `PCCL`, version 1, the grid size,
/// the table count, then per table: slider, side (−1 / 1), basis count `k`, magnitude count `m`,
/// `m` magnitudes (u8), `m × k` weights (f32), `k` scales (f32), `k × CELLS` basis values (i8).
fn parse(b: &[u8]) -> Option<Vec<Table>> {
    let mut at = 0usize;
    let mut take = |n: usize| -> Option<&[u8]> {
        let s = b.get(at..at.checked_add(n)?)?;
        at += n;
        Some(s)
    };
    let f32s = |s: &[u8]| -> Vec<f32> { s.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect() };
    if take(4)? != b"PCCL" {
        return None;
    }
    let head = take(3)?;
    let (version, n, count) = (head[0], head[1] as usize, head[2]);
    if version != 1 || n != N {
        return None;
    }
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let h = take(4)?;
        let (slider, side, k, m) = (h[0], h[1] as i8, h[2] as usize, h[3] as usize);
        if k == 0 || m == 0 || k > 16 {
            return None;
        }
        let at_v: Vec<f32> = take(m)?.iter().map(|v| f32::from(*v)).collect();
        if at_v.windows(2).any(|w| w[1] <= w[0]) {
            return None;
        }
        let w = f32s(take(m.checked_mul(k)?.checked_mul(4)?)?);
        let scale = f32s(take(k.checked_mul(4)?)?);
        let raw = take(k.checked_mul(CELLS)?)?;
        if w.iter().chain(&scale).any(|v| !v.is_finite()) {
            return None;
        }
        let basis = raw.as_chunks::<CELLS>().0.iter().zip(&scale).map(|(g, s)| g.iter().map(|v| f32::from(*v as i8) * s).collect()).collect();
        out.push(Table { slider, negative: side < 0, at: at_v, weights: w.chunks_exact(k).map(<[f32]>::to_vec).collect(), basis });
    }
    Some(out)
}

fn tables() -> &'static [Table] {
    static T: OnceLock<Vec<Table>> = OnceLock::new();
    T.get_or_init(|| parse(DATA).unwrap_or_default())
}

/// A slider's correction at one value: RGB deltas (sRGB-encoded) on the 17³ grid.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Correction(Vec<[f32; 3]>);

/// The correction for `slider` at `v` (−100 … 100), `None` at 0 or when the tables are missing.
pub(crate) fn correction(slider: Slider, v: f32) -> Option<Correction> {
    if !v.is_finite() || v == 0.0 {
        return None;
    }
    let v = v.clamp(-100.0, 100.0);
    let t = tables().iter().find(|t| t.slider == slider as u8 && t.negative == (v < 0.0))?;
    let a = v.abs();
    // Weights at `a`: between the neighbouring measured magnitudes, from 0 below the first.
    let i = t.at.iter().position(|x| *x >= a).unwrap_or(t.at.len() - 1);
    let (lo, hi, f) = if i == 0 { (None, 0, (a / t.at[0]).min(1.0)) } else { (Some(i - 1), i, ((a - t.at[i - 1]) / (t.at[i] - t.at[i - 1])).clamp(0.0, 1.0)) };
    let k = t.basis.len();
    let w: Vec<f32> = (0..k)
        .map(|j| {
            let a = lo.and_then(|l| t.weights.get(l)).and_then(|r| r.get(j)).copied().unwrap_or(0.0);
            let b = t.weights.get(hi).and_then(|r| r.get(j)).copied().unwrap_or(0.0);
            a + (b - a) * f
        })
        .collect();
    let mut grid = vec![[0.0f32; 3]; N * N * N];
    for (wj, b) in w.iter().zip(&t.basis) {
        for (g, d) in grid.iter_mut().zip(b.as_chunks::<3>().0) {
            g[0] += wj * d[0];
            g[1] += wj * d[1];
            g[2] += wj * d[2];
        }
    }
    Some(Correction(grid))
}

impl Correction {
    /// The grid: RGB deltas at node `(r · 17 + g) · 17 + b`, nodes `i / 16` per channel.
    pub(crate) fn grid(&self) -> &[[f32; 3]] {
        &self.0
    }

    /// The correction at sRGB-encoded `c` (clamped to 0 … 1), trilinear between the nodes.
    fn at(&self, c: [f32; 3]) -> [f32; 3] {
        let m = (N - 1) as f32;
        let mut i = [0usize; 3];
        let mut t = [0.0f32; 3];
        for k in 0..3 {
            let f = if c[k].is_finite() { c[k].clamp(0.0, 1.0) * m } else { 0.0 };
            i[k] = (f as usize).min(N - 2);
            t[k] = f - i[k] as f32;
        }
        let mut out = [0.0f32; 3];
        for corner in 0..8usize {
            let (dr, dg, db) = (corner >> 2 & 1, corner >> 1 & 1, corner & 1);
            let w = (if dr == 1 { t[0] } else { 1.0 - t[0] }) * (if dg == 1 { t[1] } else { 1.0 - t[1] }) * (if db == 1 { t[2] } else { 1.0 - t[2] });
            let idx = ((i[0] + dr) * N + i[1] + dg) * N + i[2] + db;
            if let Some(d) = self.0.get(idx) {
                for k in 0..3 {
                    out[k] += w * d[k];
                }
            }
        }
        out
    }

    /// `base` (the analytic model's result for the pixel `input`, both in encoding `enc`) with
    /// the correction looked up at `input` added, in sRGB-encoded values. A result is kept to
    /// 0 … 1 when the input was, like Photoshop's.
    pub(crate) fn apply(&self, input: [f32; 3], base: [f32; 3], enc: Encoding) -> [f32; 3] {
        let to = |v: f32| if enc.0 > 0.0 { super::linear_to_srgb(enc.decode(v)) } else { v };
        let from = |v: f32| if enc.0 > 0.0 { enc.encode(super::srgb_to_linear(v)) } else { v };
        let inside = input.iter().all(|v| (0.0..=1.0).contains(v));
        let mut d = self.at(input.map(to));
        // A grey result stays grey (Saturation −100, Vibrance on a grey): the measured correction
        // there is a change of level only, and its 8-bit storage noise would tint it.
        if base.iter().fold(f32::MIN, |a, v| a.max(*v)) - base.iter().fold(f32::MAX, |a, v| a.min(*v)) < 1e-5 {
            let m = (d[0] + d[1] + d[2]) / 3.0;
            d = [m; 3];
        }
        std::array::from_fn(|k| {
            let s = to(base[k]);
            let s = if inside { (s.clamp(0.0, 1.0) + d[k]).clamp(0.0, 1.0) } else { s + d[k] };
            from(s)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_load_and_cover_every_slider_and_side() {
        let t = tables();
        assert_eq!(t.len(), 5);
        assert!(correction(Slider::Whites, -60.0).is_some() && correction(Slider::Whites, 60.0).is_none());
        for s in [Slider::Temperature, Slider::Tint] {
            for v in [-100.0, -37.0, 3.0, 100.0] {
                assert!(correction(s, v).is_some(), "{s:?} {v}");
            }
        }
    }

    #[test]
    fn zero_and_hostile_values_are_safe() {
        assert!(correction(Slider::Temperature, 0.0).is_none());
        assert!(correction(Slider::Tint, f32::NAN).is_none());
        assert_eq!(correction(Slider::Tint, 400.0), correction(Slider::Tint, 100.0));
        let c = correction(Slider::Temperature, -100.0).unwrap();
        for q in [[f32::NAN, 0.5, 0.5], [2.0, -1.0, f32::INFINITY], [0.0; 3], [1.0; 3]] {
            let o = c.apply(q, q, Encoding(0.0));
            assert!(o.iter().all(|v| v.is_finite() || q.iter().any(|x| !x.is_finite())), "{q:?} {o:?}");
        }
    }

    #[test]
    fn corrections_shrink_toward_zero_and_stay_in_range() {
        // A small value corrects less than a large one; results stay within 0 … 1.
        let mid = [0.6, 0.4, 0.3];
        let small = correction(Slider::Temperature, -5.0).unwrap().at(mid);
        let large = correction(Slider::Temperature, -100.0).unwrap().at(mid);
        let n = |d: [f32; 3]| d.iter().map(|v| v * v).sum::<f32>();
        assert!(n(small) < n(large), "{small:?} {large:?}");
        let c = correction(Slider::Tint, 100.0).unwrap();
        for q in [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.9, 0.9, 0.1]] {
            assert!(c.apply(q, q, Encoding(0.0)).iter().all(|v| (0.0..=1.0).contains(v)));
        }
        // A grey stays grey.
        let g = correction(Slider::Temperature, -100.0).unwrap().apply([0.7, 0.2, 0.4], [0.35; 3], Encoding(1.8));
        assert!((g[0] - g[1]).abs() < 1e-6 && (g[1] - g[2]).abs() < 1e-6, "{g:?}");
    }

    #[test]
    fn malformed_tables_are_rejected() {
        assert!(parse(b"").is_none());
        assert!(parse(b"PCCL\x01\x11\x01").is_none());
        assert!(parse(&DATA[..DATA.len() / 2]).is_none());
        let mut bad = DATA.to_vec();
        bad[5] = 9;
        assert!(parse(&bad).is_none());
    }
}
