//! Vibrance and Saturation (the Presence group), measured.
//!
//! Measured as a black box on Photoshop 2026's Camera Raw Filter (Camera Raw 18.6); no Adobe
//! code or data was used. Every 8-bit RGB colour (a 4096 × 4096 image) went through the filter
//! at ±25, ±50, ±75 and ±100 of each slider and at 16 mixes of the two. Both are one fixed
//! function of the colour: pixels of the same colour came out identical within a picture,
//! across 17 photographs and on the synthetic image (spread 0.000 ΔE76).
//!
//! Each slider value is a table over hue × saturation × value in HSV of linear ProPhoto RGB
//! ([`hsv_table`](super::hsv_table); 48 hues × 21 saturations × 21 values, fitted by least
//! squares to 300 000 random colours of the full result), interpolated between the measured
//! values. Mixed, Saturation applies first and Vibrance then acts on its result (mean ΔE76 on
//! the 16 measured mixes: 1.25; adding the two changes 4.2, Vibrance first 3.2). Vibrance down
//! with Saturation up stays furthest (2–5): Saturation pushes colours past what an sRGB picture
//! holds, where Vibrance was never measured.
//!
//! On the colours of 17 photographs, weighted as they occur (mean ΔE76; the previous fitted
//! model with its 17³ correction → these tables): see `docs/gaps.md` G14.

use std::sync::OnceLock;

use super::hsv_table::{Grid, KNOTS, Reader, Steps, at_value};

/// The tables' nodes.
pub(crate) const GRID: Grid = Grid { hues: 48, sats: 21, vals: 21 };
/// Tables in a [`Presence`]: Saturation, Vibrance, in the order they apply.
pub(crate) const KINDS: usize = 2;

static DATA: &[u8] = include_bytes!("../../data/camera_raw_presence.bin");

/// Saturation's and Vibrance's tables, each at [`KNOTS`].
type Tables = [Vec<Vec<[f32; 3]>>; 2];

/// Reads `b` (see `export_presence.py` in the measurement tools): `PCPR`, version 1, the node
/// counts (hues, saturations, values) and the knot count as u32; then Saturation's and
/// Vibrance's tables at [`KNOTS`] ([`Reader::table`]).
fn parse(b: &[u8]) -> Option<Tables> {
    let mut r = Reader::new(b);
    if r.take(4)? != b"PCPR" {
        return None;
    }
    if r.u32s(5)? != [1, GRID.hues as u32, GRID.sats as u32, GRID.vals as u32, KNOTS.len() as u32] {
        return None;
    }
    let mut kind = || KNOTS.iter().map(|_| r.table(GRID.nodes())).collect::<Option<Vec<_>>>();
    let (sat, vib) = (kind()?, kind()?);
    r.done().then_some([sat, vib])
}

fn model() -> Option<&'static Tables> {
    static M: OnceLock<Option<Tables>> = OnceLock::new();
    M.get_or_init(|| parse(DATA)).as_ref()
}

/// Vibrance and Saturation at one setting: two tables, Saturation's then Vibrance's (see
/// [`Steps`]); a slider at 0 has a table of zeros.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Presence(Steps);

impl Presence {
    /// The two tables one after the other (for the GPU).
    pub(crate) fn nodes(&self) -> &[[f32; 3]] {
        self.0.nodes()
    }

    /// See [`Steps::apply_encoded`].
    pub(crate) fn apply_encoded(&self, c: [f32; 3], enc: super::Encoding) -> [f32; 3] {
        self.0.apply_encoded(c, enc)
    }
}

/// Vibrance and Saturation at `vibrance` and `saturation` (−100 … 100), `None` when both are 0
/// (or not finite) or the tables are missing.
pub(crate) fn presence(vibrance: f32, saturation: f32) -> Option<Presence> {
    let on = |v: f32| v.is_finite() && v != 0.0;
    if !on(vibrance) && !on(saturation) {
        return None;
    }
    let [sat, vib] = model()?;
    let n = GRID.nodes();
    let mut out = vec![[0.0f32; 3]; KINDS * n];
    for (k, (tables, u)) in [(sat, saturation), (vib, vibrance)].into_iter().enumerate() {
        if !on(u) {
            continue;
        }
        for node in 0..n {
            if let Some(o) = out.get_mut(k * n + node) {
                *o = at_value(tables, u, node);
            }
        }
    }
    Steps::new(GRID, out).map(Presence)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera_raw::{Encoding, rgb_to_hsv};

    #[test]
    fn the_tables_parse() {
        assert!(model().is_some(), "camera_raw_presence.bin parses");
        assert!(parse(&DATA[..DATA.len() - 1]).is_none(), "truncated data is rejected");
        assert!(parse(b"PCPR").is_none());
        assert!(presence(0.0, 0.0).is_none() && presence(f32::NAN, 0.0).is_none());
    }

    #[test]
    fn greys_stay_and_the_sliders_move_saturation() {
        let enc = Encoding::of(0.0);
        let p = presence(100.0, 100.0).expect("a change");
        for g in [0.0, 0.25, 0.6, 1.0] {
            assert_eq!(p.apply_encoded([g; 3], enc), [g; 3]);
        }
        let dull = [0.5, 0.42, 0.38];
        let sat = |c: [f32; 3]| rgb_to_hsv(c)[1];
        let up = presence(100.0, 0.0).expect("vibrance").apply_encoded(dull, enc);
        let down = presence(-100.0, 0.0).expect("vibrance").apply_encoded(dull, enc);
        assert!(sat(up) > sat(dull) + 0.05 && sat(down) < sat(dull), "{up:?} {down:?}");
        let grey = presence(0.0, -100.0).expect("saturation").apply_encoded([0.8, 0.3, 0.2], enc);
        assert!(sat(grey) < 0.05, "Saturation −100 greys: {grey:?}");
    }

    #[test]
    fn odd_inputs_are_safe() {
        let p = presence(1e9, -f32::INFINITY).expect("clamped");
        let enc = Encoding::of(1.8);
        for c in [[f32::NAN, 0.2, 0.1], [-1.0, 2.0, 0.5], [1e6, 0.0, 0.0], [f32::INFINITY, 0.0, 0.0]] {
            let _ = p.apply_encoded(c, enc);
        }
        let inside = p.apply_encoded([0.9, 0.1, 0.05], Encoding::of(0.0));
        assert!(inside.iter().all(|v| (0.0..=1.0).contains(v)), "{inside:?}");
    }
}
