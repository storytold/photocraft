//! The Color Mixer (HSL) panel: eight hue bands × Hue, Saturation and Luminance.
//!
//! Measured as a black box on Photoshop 2026's Camera Raw Filter (Camera Raw 18.6); no Adobe
//! code or data was used. A 33³ RGB cube went through the filter with each slider of each band
//! at ±25, ±50, ±75 and ±100, with every band at once at the same values, and with mixes. What
//! came out:
//!
//! - Each slider is a fixed function of the pixel's colour, and it acts in HSV of linear
//!   ProPhoto RGB (the hue/saturation/value space of the public DNG spec's hue/sat maps): the
//!   hue shift depends on the input hue alone (99 % of its variance), and a table over hue ×
//!   saturation × value explains the saturation and value changes (95–99 %).
//! - The bands do not add their effects. Per hue, the sliders of one kind combine into one
//!   effective value `u(h) = Σ w_b(h) · v_b` (band weights that sum to 1 at every hue, peaking
//!   at about 0°, 30°, 55°, 95°, 160°, 230°, 275° and 330° of ProPhoto hue), and the effect is
//!   that of every band at `u`. With all eight Luminance bands at −100, adding the single-band
//!   effects missed by ΔE76 9.0; the effective value misses by 0.5.
//! - The three kinds act one after the other: Luminance, then Saturation, then Hue, each looked
//!   up at the input's hue but at the saturation and value the previous kind left. On 96 measured
//!   pairs of kinds in one band (Blues, Oranges) this order missed by ΔE76 0.83, adding the
//!   three changes by 1.13 and the other orders by 1.06–2.43 (Luminance up desaturates, and
//!   Saturation then has more room).
//!
//! So the data is, per kind, the effect with every band at −100, −75 … 100 (a table of hue
//! shift in degrees, saturation change and log-value change on 72 hues × 9 saturations × 9
//! values, interpolated trilinearly with hue wrapping round) and the band weights at the 72
//! hues: `data/camera_raw_hsl.bin`, written by the measurement tools (`export_hsl.py`).
//!
//! On the cube (mean ΔE76; the old hat-function model → this): single sliders 3.7 → 0.3,
//! mixes of one kind 0.1–0.5, pairs of kinds in one band 0.8, all three kinds in one band
//! about 1.2.

use std::sync::OnceLock;

use super::Encoding;
use super::hsv_table::{Grid, KNOTS, Reader, Steps, at_value};

/// The tables' nodes: hues every 5°, 9 saturations, 9 values.
pub(crate) const GRID: Grid = Grid { hues: 72, sats: 9, vals: 9 };
/// Hue nodes.
pub(crate) const HUES: usize = GRID.hues;
/// Saturation nodes over 0 … 1.
pub(crate) const SATS: usize = GRID.sats;
/// Value nodes over 0 … 1.
pub(crate) const VALS: usize = GRID.vals;
/// Nodes in one table.
pub(crate) const NODES: usize = GRID.nodes();
/// Tables in a [`Mixer`]: Luminance, Saturation, Hue, in the order they apply.
pub(crate) const KINDS: usize = 3;

static DATA: &[u8] = include_bytes!("../../data/camera_raw_hsl.bin");

/// The measured model: per kind (hue, saturation, luminance) the band weights at every hue node
/// and the tables at [`KNOTS`].
struct Model {
    /// `[kind][band][hue node]`.
    weights: Vec<[Vec<f32>; 8]>,
    /// `[kind][knot]`, [`NODES`] entries each: hue shift (degrees), saturation change, log-value
    /// change.
    tables: Vec<Vec<Vec<[f32; 3]>>>,
}

/// Reads `b` (see `export_hsl.py` in the measurement tools): `PCHS`, version 1, the node counts
/// (hues, saturations, values) and the knot count as u32; the weights as f32 `[3][8][hues]`;
/// then per kind and knot a table ([`Reader::table`]).
fn parse(b: &[u8]) -> Option<Model> {
    let mut r = Reader::new(b);
    if r.take(4)? != b"PCHS" {
        return None;
    }
    if r.u32s(5)? != [1, HUES as u32, SATS as u32, VALS as u32, KNOTS.len() as u32] {
        return None;
    }
    let mut weights = Vec::with_capacity(3);
    for _ in 0..3 {
        let mut kind: [Vec<f32>; 8] = Default::default();
        for band in &mut kind {
            *band = r.f32s(HUES)?;
        }
        weights.push(kind);
    }
    let mut tables = Vec::with_capacity(3);
    for _ in 0..3 {
        tables.push(KNOTS.iter().map(|_| r.table(NODES)).collect::<Option<Vec<_>>>()?);
    }
    r.done().then_some(Model { weights, tables })
}

fn model() -> Option<&'static Model> {
    static M: OnceLock<Option<Model>> = OnceLock::new();
    M.get_or_init(|| parse(DATA)).as_ref()
}

/// The Color Mixer's change at one setting: three tables (Luminance, Saturation, Hue, in the
/// order they apply, see [`Steps`]). A kind whose sliders are all 0 has a table of zeros.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Mixer(Steps);

impl Mixer {
    /// The three tables one after the other, [`KINDS`] × [`NODES`] entries (for the GPU).
    pub(crate) fn nodes(&self) -> &[[f32; 3]] {
        self.0.nodes()
    }

    /// See [`Steps::apply_encoded`].
    pub(crate) fn apply_encoded(&self, c: [f32; 3], enc: Encoding) -> [f32; 3] {
        self.0.apply_encoded(c, enc)
    }

    /// `c` (linear ProPhoto RGB) through the Color Mixer.
    #[cfg(test)]
    fn apply(&self, c: [f32; 3]) -> [f32; 3] {
        self.0.apply(c)
    }

    #[cfg(test)]
    fn lookup(&self, kind: usize, h: f32, s: f32, v: f32) -> [f32; 3] {
        self.0.lookup(kind, h, s, v)
    }
}

/// The Color Mixer at `hue`, `sat`, `lum` (−100 … 100 per band, red, orange, yellow, green,
/// aqua, blue, purple, magenta), `None` when every slider is 0 or the tables are missing.
pub(crate) fn mixer(hue: &[f32; 8], sat: &[f32; 8], lum: &[f32; 8]) -> Option<Mixer> {
    let sliders = [hue, sat, lum];
    let off = |k: &[f32; 8]| k.iter().all(|v| *v == 0.0 || !v.is_finite());
    if sliders.iter().all(|k| off(k)) {
        return None;
    }
    let m = model()?;
    let mut out = vec![[0.0f32; 3]; KINDS * NODES];
    // The data is in the order hue, saturation, luminance; the mixer's in the order they apply.
    for (kind, ((values, weights), tables)) in sliders.iter().zip(&m.weights).zip(&m.tables).enumerate() {
        if off(values) {
            continue;
        }
        let base = (KINDS - 1 - kind) * NODES;
        for h in 0..HUES {
            // The kind's effective slider value at this hue.
            let u: f32 = values.iter().zip(weights).map(|(v, w)| if v.is_finite() { v.clamp(-100.0, 100.0) * w.get(h).copied().unwrap_or(0.0) } else { 0.0 }).sum();
            for node in h * SATS * VALS..(h + 1) * SATS * VALS {
                if let Some(o) = out.get_mut(base + node) {
                    *o = at_value(tables, u, node);
                }
            }
        }
    }
    Steps::new(GRID, out).map(Mixer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tables_parse() {
        let m = model().expect("camera_raw_hsl.bin parses");
        for kind in &m.weights {
            for h in 0..HUES {
                let s: f32 = kind.iter().map(|b| b[h]).sum();
                assert!((s - 1.0).abs() < 1e-3, "weights sum to 1 at hue node {h}: {s}");
            }
        }
        assert!(parse(&DATA[..DATA.len() - 1]).is_none(), "truncated data is rejected");
        assert!(parse(b"PCHS").is_none());
    }

    #[test]
    fn neutral_is_none_and_greys_stay() {
        assert!(mixer(&[0.0; 8], &[0.0; 8], &[0.0; 8]).is_none());
        assert!(mixer(&[f32::NAN; 8], &[0.0; 8], &[0.0; 8]).is_none());
        let m = mixer(&[100.0; 8], &[-100.0; 8], &[100.0; 8]).expect("a change");
        for g in [0.0, 0.2, 0.7, 1.0] {
            assert_eq!(m.apply([g; 3]), [g; 3]);
        }
    }

    #[test]
    fn bands_act_on_their_hues() {
        // Saturation of the blues to −100 greys a ProPhoto blue and leaves a red alone.
        let mut s = [0.0; 8];
        s[5] = -100.0;
        let m = mixer(&[0.0; 8], &s, &[0.0; 8]).expect("a change");
        let blue = m.apply([0.05, 0.08, 0.4]);
        let [_, sat, _] = super::super::rgb_to_hsv(blue);
        assert!(sat < 0.2, "blue greyed: {blue:?}");
        let red = [0.4, 0.06, 0.05];
        let r = m.apply(red);
        assert!(r.iter().zip(red).all(|(a, b)| (a - b).abs() < 0.01), "red kept: {r:?}");
        // Luminance of every band down darkens a colour; up brightens it.
        let c = [0.1, 0.3, 0.05];
        let down = mixer(&[0.0; 8], &[0.0; 8], &[-100.0; 8]).expect("a change").apply(c);
        let up = mixer(&[0.0; 8], &[0.0; 8], &[100.0; 8]).expect("a change").apply(c);
        let v = |c: [f32; 3]| c[0].max(c[1]).max(c[2]);
        assert!(v(down) < v(c) && v(up) > v(c), "{down:?} {up:?}");
    }

    #[test]
    fn kinds_apply_luminance_then_saturation_then_hue() {
        let (mut s, mut l) = ([0.0; 8], [0.0; 8]);
        s[5] = 100.0;
        l[5] = 100.0;
        let m = mixer(&[0.0; 8], &s, &l).expect("a change");
        let c = [0.05, 0.1, 0.5];
        let [h0, s0, v0] = super::super::rgb_to_hsv(c);
        // Luminance at the input, then Saturation at the saturation and value Luminance left.
        let dl = m.lookup(0, h0, s0, v0);
        let (s1, v1) = ((s0 + dl[1]).clamp(0.0, 1.0), v0 * dl[2].exp());
        let ds = m.lookup(1, h0, s1, v1);
        assert_ne!(ds, m.lookup(1, h0, s0, v0), "the saturation table varies with the colour");
        let want = super::super::hsv_to_rgb(h0 + dl[0] + ds[0], (s1 + ds[1]).clamp(0.0, 1.0), v1 * ds[2].exp());
        let got = m.apply(c);
        assert!(got.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-5), "{got:?} vs {want:?}");
        assert_eq!(m.lookup(2, h0, s0, v0), [0.0; 3], "no hue sliders, a zero Hue table");
    }

    #[test]
    fn small_values_are_small_and_odd_inputs_are_safe() {
        let mut h = [0.0; 8];
        h[3] = 1.0;
        let m = mixer(&h, &[0.0; 8], &[0.0; 8]).expect("a change");
        let c = [0.1, 0.4, 0.1];
        let o = m.apply(c);
        assert!(o.iter().zip(c).all(|(a, b)| (a - b).abs() < 0.02), "{o:?}");
        let big = mixer(&[1e9; 8], &[-1e9; 8], &[f32::INFINITY; 8]).expect("clamped");
        for c in [[f32::NAN, 0.2, 0.1], [-1.0, 2.0, 0.5], [1e6, 0.0, 0.0], [f32::INFINITY, 0.0, 0.0]] {
            let _ = big.apply(c);
        }
    }
}
