//! Measured colour changes as tables over hue × saturation × value in HSV of linear ProPhoto RGB
//! (the hue/saturation/value space of the public DNG spec's hue/sat maps), shared by the Color
//! Mixer ([`color_mixer`](super::color_mixer)) and Vibrance and Saturation
//! ([`presence`](super::presence)).
//!
//! A table holds, per node, the hue shift in degrees, the saturation change and the log-value
//! change. Several tables apply one after the other ([`Steps`]): each is looked up trilinearly
//! (hue wrapping round, saturation and value clamped to the grid) at the input's hue and at the
//! saturation and value the previous one left.

use super::Encoding;

/// A table's nodes: hues (every 360° / `hues`, wrapping round), saturations and values (each
/// evenly over 0 … 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Grid {
    pub hues: usize,
    pub sats: usize,
    pub vals: usize,
}

impl Grid {
    /// Nodes in one table (hue-major, then saturation, then value).
    pub(crate) const fn nodes(self) -> usize {
        self.hues * self.sats * self.vals
    }
}

/// Saturations at or below this are greys (matrix rounding), which the tables leave alone.
const GREY: f32 = 1e-4;

/// Slider values with a measured table, on the 0 … ±100 scale (0 is no change).
pub(crate) const KNOTS: [f32; 8] = [-100.0, -75.0, -50.0, -25.0, 25.0, 50.0, 75.0, 100.0];

/// Node `node` of a slider's change at `u` (−100 … 100) from its tables at [`KNOTS`]: linear
/// between the neighbouring knots, from no change at 0.
pub(crate) fn at_value(tables: &[Vec<[f32; 3]>], u: f32, node: usize) -> [f32; 3] {
    if !u.is_finite() || u == 0.0 {
        return [0.0; 3];
    }
    let f = (u.clamp(-100.0, 100.0) + 100.0) / 25.0;
    let i = (f as usize).min(7);
    let t = f - i as f32;
    // Knot `j` of −100, −75 … 0 … 100; 0 has no table.
    let table = |j: usize| match j {
        0..4 => Some(j),
        4 => None,
        _ => Some(j - 1),
    };
    let at = |k: Option<usize>| k.and_then(|k| tables.get(k)).and_then(|t| t.get(node)).copied().unwrap_or([0.0; 3]);
    let (a, b) = (at(table(i)), at(table(i + 1)));
    [0, 1, 2].map(|c| a[c] + (b[c] - a[c]) * t)
}

/// Reads tables written by the measurement tools: per table three f32 scales (little endian),
/// then `nodes × 3` i8 values, each times its channel's scale.
pub(crate) struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(b: &'a [u8]) -> Self {
        Reader { b, at: 0 }
    }

    pub(crate) fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(s)
    }

    /// `n` little-endian u32s.
    pub(crate) fn u32s(&mut self, n: usize) -> Option<Vec<u32>> {
        Some(self.take(n.checked_mul(4)?)?.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect())
    }

    /// `n` finite little-endian f32s.
    pub(crate) fn f32s(&mut self, n: usize) -> Option<Vec<f32>> {
        let v: Vec<f32> = self.take(n.checked_mul(4)?)?.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect();
        v.iter().all(|x| x.is_finite()).then_some(v)
    }

    /// One table of `nodes` entries: three scales, then i8 triples.
    pub(crate) fn table(&mut self, nodes: usize) -> Option<Vec<[f32; 3]>> {
        let scale = self.f32s(3)?;
        let raw = self.take(nodes.checked_mul(3)?)?;
        Some(raw.as_chunks::<3>().0.iter().map(|n| [0, 1, 2].map(|c| f32::from(n[c] as i8) * scale.get(c).copied().unwrap_or(0.0))).collect())
    }

    /// Whether every byte was read.
    pub(crate) fn done(&self) -> bool {
        self.at == self.b.len()
    }
}

/// Tables on one [`Grid`] that apply one after the other.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Steps {
    grid: Grid,
    /// The tables one after the other, `grid.nodes()` entries each.
    tables: Vec<[f32; 3]>,
}

impl Steps {
    /// `tables` must hold a whole number of `grid` tables.
    pub(crate) fn new(grid: Grid, tables: Vec<[f32; 3]>) -> Option<Self> {
        let n = grid.nodes();
        (grid.hues > 0 && grid.sats > 1 && grid.vals > 1 && !tables.is_empty() && tables.len().is_multiple_of(n)).then_some(Steps { grid, tables })
    }

    /// The tables one after the other (for the GPU).
    pub(crate) fn nodes(&self) -> &[[f32; 3]] {
        &self.tables
    }

    /// `c` in the document's encoding `enc` (and its primaries, see
    /// [`exposure::primaries`](super::exposure::primaries)) through the tables; greys and colours
    /// the tables leave alone stay exactly as they are, and a colour within 0 … 1 stays within.
    pub(crate) fn apply_encoded(&self, c: [f32; 3], enc: Encoding) -> [f32; 3] {
        let pp = super::exposure::primaries(enc);
        let lin = c.map(|v| enc.decode(v));
        let x = pp.as_ref().map_or(lin, |(to, _)| super::exposure::mat(to, lin));
        let y = self.apply(x);
        if y == x {
            return c;
        }
        let back = pp.as_ref().map_or(y, |(_, from)| super::exposure::mat(from, y));
        let inside = c.iter().all(|v| (0.0..=1.0).contains(v));
        back.map(|v| {
            let e = enc.encode(v);
            if inside { e.clamp(0.0, 1.0) } else { e }
        })
    }

    /// `c` (linear ProPhoto RGB) through the tables in order.
    pub(crate) fn apply(&self, c: [f32; 3]) -> [f32; 3] {
        let [h0, mut s, mut v] = super::rgb_to_hsv(c.map(|x| x.max(0.0)));
        // A grey in the document's primaries comes out with a saturation of rounding error only.
        if v <= 1e-6 || s <= GREY {
            return c;
        }
        let (mut h, mut changed) = (h0, false);
        for k in 0..self.tables.len() / self.grid.nodes() {
            let d = self.lookup(k, h0, s, v);
            if d == [0.0; 3] {
                continue;
            }
            changed = true;
            h += d[0];
            s = (s + d[1]).clamp(0.0, 1.0);
            v *= d[2].clamp(-20.0, 20.0).exp();
        }
        if !changed {
            return c;
        }
        let out = super::hsv_to_rgb(h, s, v);
        if out.iter().all(|x| x.is_finite()) { out } else { c }
    }

    /// Table `k`'s change at `h` (degrees), `s`, `v`: trilinear, hue wrapping round, saturation
    /// and value clamped to the grid.
    pub(crate) fn lookup(&self, k: usize, h: f32, s: f32, v: f32) -> [f32; 3] {
        let Grid { hues, sats, vals } = self.grid;
        let fh = (h.rem_euclid(360.0) / 360.0 * hues as f32).clamp(0.0, hues as f32);
        let fs = s.clamp(0.0, 1.0) * (sats - 1) as f32;
        let fv = v.clamp(0.0, 1.0) * (vals - 1) as f32;
        let (ih, is, iv) = ((fh as usize).min(hues - 1), (fs as usize).min(sats - 2), (fv as usize).min(vals - 2));
        let (th, ts, tv) = (fh - ih as f32, fs - is as f32, fv - iv as f32);
        let base = k * self.grid.nodes();
        let mut d = [0.0f32; 3];
        for (dh, wh) in [(0, 1.0 - th), (1, th)] {
            for (ds, ws) in [(0, 1.0 - ts), (1, ts)] {
                for (dv, wv) in [(0, 1.0 - tv), (1, tv)] {
                    let node = base + (((ih + dh) % hues) * sats + is + ds) * vals + iv + dv;
                    let Some(n) = self.tables.get(node) else { continue };
                    let w = wh * ws * wv;
                    for c in 0..3 {
                        d[c] += w * n[c];
                    }
                }
            }
        }
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const G: Grid = Grid { hues: 4, sats: 2, vals: 2 };

    #[test]
    fn steps_reject_partial_tables_and_keep_untouched_colours() {
        assert!(Steps::new(G, vec![[0.0; 3]; 15]).is_none());
        assert!(Steps::new(G, Vec::new()).is_none());
        assert!(Steps::new(Grid { hues: 4, sats: 1, vals: 2 }, vec![[0.0; 3]; 8]).is_none());
        let zero = Steps::new(G, vec![[0.0; 3]; 32]).expect("two tables");
        for c in [[0.2, 0.5, 0.1], [0.3; 3], [-0.1, 0.4, 2.0], [f32::NAN, 0.0, 0.1]] {
            let o = zero.apply(c);
            assert!(o.iter().zip(c).all(|(a, b)| a == &b || (a.is_nan() && b.is_nan())), "{o:?}");
        }
    }

    #[test]
    fn a_constant_table_shifts_hue_and_scales_value() {
        let s = Steps::new(G, vec![[120.0, 0.0, 2f32.ln()]; 16]).expect("one table");
        let [h, sat, v] = crate::camera_raw::rgb_to_hsv(s.apply([0.4, 0.1, 0.1]));
        assert!((h - 120.0).abs() < 1e-3 && (sat - 0.75).abs() < 1e-4 && (v - 0.8).abs() < 1e-4, "{h} {sat} {v}");
    }

    #[test]
    fn values_between_knots_interpolate_from_zero() {
        let tables: Vec<Vec<[f32; 3]>> = KNOTS.iter().map(|k| vec![[*k, 0.0, 0.0]]).collect();
        assert_eq!(at_value(&tables, 0.0, 0), [0.0; 3]);
        assert_eq!(at_value(&tables, 12.5, 0), [12.5, 0.0, 0.0]);
        assert_eq!(at_value(&tables, -60.0, 0), [-60.0, 0.0, 0.0]);
        assert_eq!(at_value(&tables, 400.0, 0), [100.0, 0.0, 0.0]);
        assert_eq!(at_value(&tables, f32::NAN, 0), [0.0; 3]);
        assert_eq!(at_value(&tables, 50.0, 9), [0.0; 3], "missing node");
    }

    #[test]
    fn the_reader_stops_at_the_end() {
        let mut r = Reader::new(&[1, 0, 0, 0, 0, 0, 128, 63]);
        assert_eq!(r.u32s(1), Some(vec![1]));
        assert_eq!(r.f32s(1), Some(vec![1.0]));
        assert!(r.done());
        assert_eq!(r.take(1), None);
        let nan = f32::NAN.to_le_bytes();
        assert_eq!(Reader::new(&nan).f32s(1), None);
    }
}
