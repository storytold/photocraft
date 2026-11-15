//! Black & White treatment and its mixer (the Color Mixer's "B&W" mode), measured.
//!
//! Measured as a black box on Photoshop 2026's Camera Raw Filter (Camera Raw 18.6); no Adobe
//! code or data was used. Every 8-bit RGB colour (a 4096 × 4096 image) went through the filter
//! in black and white, by default and with each mixer band at ±25, ±50, ±75 and ±100. The grey
//! is a fixed function of the colour; it is not the colour's luminance (a plain desaturation
//! misses by ΔE76 8.7), so it is modelled as the luminance (linear ProPhoto) times a measured
//! factor that depends on hue, saturation and value in HSV of linear ProPhoto RGB: a table on
//! 36 × 13 × 13 nodes of the factor's log. Each mixer band adds its own measured change to
//! that log, interpolated between the measured values.
//!
//! On the colours of 17 photographs (mean ΔE76): the default 0.11, one band at ±50/±100 about
//! 0.13; mixes of several bands 0.25–0.8, and every band moved together 1.1–1.3 (neighbouring
//! bands interact a little). The tables are `data/camera_raw_bw.bin`, written by the
//! measurement tools (`export_bw.py`).

use std::sync::OnceLock;

use super::Encoding;
use super::hsv_table::{Grid, KNOTS, Reader, Steps, at_value};

/// The tables' nodes.
const GRID: Grid = Grid { hues: 36, sats: 13, vals: 13 };
/// Linear ProPhoto RGB to luminance (Y of its D50 primaries).
const Y_PROPHOTO: [f32; 3] = [0.288_040_2, 0.711_874_1, 0.000_085_7];

static DATA: &[u8] = include_bytes!("../../data/camera_raw_bw.bin");

/// The default table, then per band its tables at [`KNOTS`] (changes to the default).
struct Model {
    default: Vec<[f32; 3]>,
    bands: Vec<Vec<Vec<[f32; 3]>>>,
}

/// One table of `n` log factors: an f32 scale, then `n` little-endian i16, kept in the third
/// channel of [`hsv_table`](super::hsv_table)'s triples.
fn table(r: &mut Reader<'_>, n: usize) -> Option<Vec<[f32; 3]>> {
    let scale = r.f32s(1)?.first().copied()?;
    let raw = r.take(n.checked_mul(2)?)?;
    Some(raw.as_chunks::<2>().0.iter().map(|b| [0.0, 0.0, f32::from(i16::from_le_bytes(*b)) * scale]).collect())
}

/// Reads `b` (see `export_bw.py` in the measurement tools): `PCBW`, version 1, the node counts
/// (hues, saturations, values) and the knot count as u32; the default table, then per band (red,
/// orange, yellow, green, aqua, blue, purple, magenta) its tables at [`KNOTS`].
fn parse(b: &[u8]) -> Option<Model> {
    let mut r = Reader::new(b);
    if r.take(4)? != b"PCBW" {
        return None;
    }
    if r.u32s(5)? != [1, GRID.hues as u32, GRID.sats as u32, GRID.vals as u32, KNOTS.len() as u32] {
        return None;
    }
    let n = GRID.nodes();
    let default = table(&mut r, n)?;
    let mut bands = Vec::with_capacity(8);
    for _ in 0..8 {
        bands.push(KNOTS.iter().map(|_| table(&mut r, n)).collect::<Option<Vec<_>>>()?);
    }
    r.done().then_some(Model { default, bands })
}

fn model() -> Option<&'static Model> {
    static M: OnceLock<Option<Model>> = OnceLock::new();
    M.get_or_init(|| parse(DATA)).as_ref()
}

/// The black-and-white conversion at one mixer setting.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Mono(Steps);

impl Mono {
    /// `c` in the document's encoding `enc` (and its primaries, see
    /// [`exposure::primaries`](super::exposure::primaries)) as a grey; a colour within 0 … 1
    /// stays within.
    pub(crate) fn apply_encoded(&self, c: [f32; 3], enc: Encoding) -> [f32; 3] {
        let lin = c.map(|v| enc.decode(v));
        let x = super::exposure::primaries(enc).map_or(lin, |(to, _)| super::exposure::mat(&to, lin));
        let y = (Y_PROPHOTO[0] * x[0] + Y_PROPHOTO[1] * x[1] + Y_PROPHOTO[2] * x[2]).max(0.0);
        let [h, s, v] = super::rgb_to_hsv(x.map(|q| q.max(0.0)));
        let k = self.0.lookup(0, h, s, v)[2].clamp(-20.0, 20.0).exp();
        let g = y * k;
        if !g.is_finite() {
            return c;
        }
        let inside = c.iter().all(|v| (0.0..=1.0).contains(v));
        let e = enc.encode(g);
        [if inside { e.clamp(0.0, 1.0) } else { e }; 3]
    }
}

/// The black-and-white conversion with mixer `mix` (−100 … 100 per band: red, orange, yellow,
/// green, aqua, blue, purple, magenta); `None` when the tables are missing.
pub(crate) fn mono(mix: &[f32; 8]) -> Option<Mono> {
    let m = model()?;
    let mut t = m.default.clone();
    for (u, tables) in mix.iter().zip(&m.bands) {
        if !u.is_finite() || *u == 0.0 {
            continue;
        }
        for (node, o) in t.iter_mut().enumerate() {
            o[2] += at_value(tables, *u, node)[2];
        }
    }
    Steps::new(GRID, t).map(Mono)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tables_parse() {
        assert!(model().is_some(), "camera_raw_bw.bin parses");
        assert!(parse(&DATA[..DATA.len() - 1]).is_none(), "truncated data is rejected");
        assert!(parse(b"PCBW").is_none());
    }

    #[test]
    fn colours_become_greys_as_photoshop_makes_them() {
        let enc = Encoding::of(0.0);
        let m = mono(&[0.0; 8]).expect("tables");
        for c in [[0.8, 0.2, 0.1], [0.1, 0.5, 0.9], [0.5, 0.5, 0.5], [0.0; 3], [1.0; 3]] {
            let g = m.apply_encoded(c, enc);
            assert!(g[0] == g[1] && g[1] == g[2] && (0.0..=1.0).contains(&g[0]), "{c:?} → {g:?}");
        }
        // A grey stays close to itself; black stays black.
        let g = m.apply_encoded([0.5; 3], enc)[0];
        assert!((g - 0.5).abs() < 0.05, "{g}");
        assert_eq!(m.apply_encoded([0.0; 3], enc), [0.0; 3]);
    }

    #[test]
    fn a_mixer_band_brightens_its_colours() {
        let enc = Encoding::of(0.0);
        let red = [0.8, 0.15, 0.1];
        let base = mono(&[0.0; 8]).expect("tables").apply_encoded(red, enc)[0];
        let mut mix = [0.0; 8];
        mix[0] = 100.0;
        let up = mono(&mix).expect("tables").apply_encoded(red, enc)[0];
        mix[0] = -100.0;
        let down = mono(&mix).expect("tables").apply_encoded(red, enc)[0];
        assert!(up > base + 0.05 && down < base - 0.05, "{down} {base} {up}");
        // Hostile values are safe.
        let odd = mono(&[f32::NAN, 1e9, -1e9, f32::INFINITY, 0.0, 0.0, 0.0, 0.0]).expect("tables");
        for c in [[f32::NAN, 0.2, 0.1], [-1.0, 2.0, 0.5], [1e6, 0.0, 0.0]] {
            let _ = odd.apply_encoded(c, Encoding::of(1.8));
        }
    }
}
