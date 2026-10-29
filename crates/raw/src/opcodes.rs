//! DNG opcode lists (DNG 1.7, chapter 7 "Opcode List Processing").
//!
//! An opcode list is big-endian whatever the file's byte order: a count, then
//! per opcode its ID, DNG version, flags (bit 0 = optional), parameter size
//! and parameters. OpcodeList2's GainMap (ID 9), the lens-shading correction
//! phones write, and MapPolynomial (ID 8), the tone mapping lossy DNGs carry
//! per plane, are applied; other opcodes are reported.

use crate::par;
use crate::sensor::{BlackLevels, Rect};

/// One GainMap opcode: a grid of gains, bilinearly interpolated, applied to
/// the pixels of an area (in active-area coordinates) at a row / column pitch.
#[derive(Debug, Clone, PartialEq)]
pub struct GainMap {
    pub top: usize,
    pub left: usize,
    pub bottom: usize,
    pub right: usize,
    pub plane: usize,
    pub planes: usize,
    pub row_pitch: usize,
    pub col_pitch: usize,
    pub points_v: usize,
    pub points_h: usize,
    /// Spacing and origin of the map points, in image-relative coordinates (0..1).
    pub spacing_v: f64,
    pub spacing_h: f64,
    pub origin_v: f64,
    pub origin_h: f64,
    pub map_planes: usize,
    /// `points_v × points_h × map_planes` gains.
    pub gains: Vec<f32>,
}

impl GainMap {
    /// Gains along the map's horizontal points for image-relative row `rv`
    /// and map plane `mp` (vertical interpolation done).
    pub fn row(&self, rv: f64, mp: usize) -> Vec<f32> {
        let mp = mp.min(self.map_planes.saturating_sub(1));
        let fv = ((rv - self.origin_v) / self.spacing_v).clamp(0.0, (self.points_v - 1) as f64);
        let i0 = (fv.floor() as usize).min(self.points_v - 1);
        let i1 = (i0 + 1).min(self.points_v - 1);
        let t = (fv - i0 as f64) as f32;
        (0..self.points_h)
            .map(|j| {
                let a = self.gains.get((i0 * self.points_h + j) * self.map_planes + mp).copied().unwrap_or(1.0);
                let b = self.gains.get((i1 * self.points_h + j) * self.map_planes + mp).copied().unwrap_or(1.0);
                a + (b - a) * t
            })
            .collect()
    }

    /// Interpolates a [`GainMap::row`] at image-relative column `rh`.
    #[inline]
    pub fn at(&self, row: &[f32], rh: f64) -> f32 {
        let fh = ((rh - self.origin_h) / self.spacing_h).clamp(0.0, (self.points_h - 1) as f64);
        let j0 = (fh.floor() as usize).min(self.points_h - 1);
        let j1 = (j0 + 1).min(self.points_h - 1);
        let t = (fh - j0 as f64) as f32;
        let (a, b) = (row.get(j0).copied().unwrap_or(1.0), row.get(j1).copied().unwrap_or(1.0));
        a + (b - a) * t
    }

    /// Bilinearly interpolates the gain at image-relative row `rv` and column `rh` for map
    /// plane `mp`, touching only the four surrounding grid points. This avoids building the
    /// whole `points_h`-wide row per image row, which made development work proportional to
    /// image height times horizontal map size even when each output row needs few values.
    #[inline]
    pub fn at_rv(&self, rv: f64, mp: usize, rh: f64) -> f32 {
        let mp = mp.min(self.map_planes.saturating_sub(1));
        let fv = ((rv - self.origin_v) / self.spacing_v).clamp(0.0, (self.points_v - 1) as f64);
        let i0 = (fv.floor() as usize).min(self.points_v - 1);
        let i1 = (i0 + 1).min(self.points_v - 1);
        let tv = (fv - i0 as f64) as f32;
        let fh = ((rh - self.origin_h) / self.spacing_h).clamp(0.0, (self.points_h - 1) as f64);
        let j0 = (fh.floor() as usize).min(self.points_h - 1);
        let j1 = (j0 + 1).min(self.points_h - 1);
        let th = (fh - j0 as f64) as f32;
        let g = |i: usize, j: usize| self.gains.get((i * self.points_h + j) * self.map_planes + mp).copied().unwrap_or(1.0);
        let top = g(i0, j0) + (g(i0, j1) - g(i0, j0)) * th;
        let bot = g(i1, j0) + (g(i1, j1) - g(i1, j0)) * th;
        top + (bot - top) * tv
    }

    /// `true` if the map covers active-area pixel (`ax`, `ay`).
    #[inline]
    pub fn covers(&self, ax: usize, ay: usize) -> bool {
        ay >= self.top
            && ay < self.bottom
            && ax >= self.left
            && ax < self.right
            && (ay - self.top).is_multiple_of(self.row_pitch)
            && (ax - self.left).is_multiple_of(self.col_pitch)
    }
}

/// Opcode names (DNG 1.7 table) for warnings.
fn name(id: u32) -> String {
    match id {
        1 => "WarpRectilinear".into(),
        2 => "WarpFisheye".into(),
        3 => "FixVignetteRadial".into(),
        4 => "FixBadPixelsConstant".into(),
        5 => "FixBadPixelsList".into(),
        6 => "TrimBounds".into(),
        7 => "MapTable".into(),
        8 => "MapPolynomial".into(),
        9 => "GainMap".into(),
        10 => "DeltaPerRow".into(),
        11 => "DeltaPerColumn".into(),
        12 => "ScalePerRow".into(),
        13 => "ScalePerColumn".into(),
        14 => "WarpRectilinear2".into(),
        id => format!("opcode {id}"),
    }
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    let s: [u8; 4] = b.get(at..at.checked_add(4)?)?.try_into().ok()?;
    Some(u32::from_be_bytes(s))
}

fn be64f(b: &[u8], at: usize) -> Option<f64> {
    let s: [u8; 8] = b.get(at..at.checked_add(8)?)?.try_into().ok()?;
    Some(f64::from_be_bytes(s)).filter(|v| v.is_finite())
}

fn gain_map(p: &[u8]) -> Option<GainMap> {
    let u = |i: usize| be32(p, 4 * i).map(|v| v as usize);
    let (top, left, bottom, right) = (u(0)?, u(1)?, u(2)?, u(3)?);
    let (plane, planes, row_pitch, col_pitch) = (u(4)?, u(5)?, u(6)?, u(7)?);
    let (points_v, points_h) = (u(8)?, u(9)?);
    let (spacing_v, spacing_h, origin_v, origin_h) = (be64f(p, 40)?, be64f(p, 48)?, be64f(p, 56)?, be64f(p, 64)?);
    let map_planes = u(18)?;
    let n = points_v.checked_mul(points_h)?.checked_mul(map_planes)?;
    if n == 0 || n > 1 << 20 || row_pitch == 0 || col_pitch == 0 || spacing_v <= 0.0 || spacing_h <= 0.0 || bottom <= top || right <= left {
        return None;
    }
    let gains = (0..n).map(|i| be32(p, 76 + 4 * i).map(f32::from_bits).filter(|g| g.is_finite())).collect::<Option<Vec<f32>>>()?;
    Some(GainMap {
        top,
        left,
        bottom,
        right,
        plane,
        planes,
        row_pitch,
        col_pitch,
        points_v,
        points_h,
        spacing_v,
        spacing_h,
        origin_v,
        origin_h,
        map_planes,
        gains,
    })
}

/// One MapPolynomial opcode: `out = Σ cᵢ · inⁱ` on the linear reference values (0..1, after the
/// black and white levels) of an area, in active-area coordinates, and a plane range, at a row /
/// column pitch. Input and output are clipped to 0..1.
#[derive(Debug, Clone, PartialEq)]
pub struct MapPolynomial {
    pub top: usize,
    pub left: usize,
    pub bottom: usize,
    pub right: usize,
    pub plane: usize,
    pub planes: usize,
    pub row_pitch: usize,
    pub col_pitch: usize,
    /// `c₀ … c_degree` (degree at most 8).
    pub coefficients: Vec<f64>,
}

impl MapPolynomial {
    /// The polynomial at `x`, both clipped to 0..1.
    #[inline]
    pub fn eval(&self, x: f64) -> f64 {
        let x = x.clamp(0.0, 1.0);
        let y = self.coefficients.iter().rev().fold(0.0, |acc, c| acc * x + c);
        if y.is_finite() { y.clamp(0.0, 1.0) } else { 0.0 }
    }

    /// `true` if the opcode covers active-area pixel (`ax`, `ay`) in sample `s`.
    #[inline]
    fn covers(&self, ax: usize, ay: usize, s: usize) -> bool {
        s >= self.plane
            && s - self.plane < self.planes
            && ay >= self.top
            && ay < self.bottom
            && ax >= self.left
            && ax < self.right
            && (ay - self.top).is_multiple_of(self.row_pitch)
            && (ax - self.left).is_multiple_of(self.col_pitch)
    }
}

fn map_polynomial(p: &[u8]) -> Option<MapPolynomial> {
    let u = |i: usize| be32(p, 4 * i).map(|v| v as usize);
    let (top, left, bottom, right) = (u(0)?, u(1)?, u(2)?, u(3)?);
    let (plane, planes, row_pitch, col_pitch, degree) = (u(4)?, u(5)?, u(6)?, u(7)?, u(8)?);
    if degree > 8 || planes == 0 || row_pitch == 0 || col_pitch == 0 || bottom <= top || right <= left {
        return None;
    }
    let coefficients = (0..=degree).map(|i| be64f(p, 36 + 8 * i)).collect::<Option<Vec<f64>>>()?;
    Some(MapPolynomial { top, left, bottom, right, plane, planes, row_pitch, col_pitch, coefficients })
}

pub(crate) struct PolynomialScale<'a> {
    pub black: &'a BlackLevels,
    pub white: [f32; 3],
    pub bits: u32,
    pub max_alloc: u64,
}

// Cache the complete integer domain, composing each opcode in its original
// order with the same rounding after every step. This is exact, not a sampled
// approximation of the polynomial. Spatially varying levels/coverage keep the
// scalar oracle below; hostile input cannot allocate an unbounded table.
fn polynomial_lookup(data: &mut [u16], width: usize, samples: usize, active: Rect, polys: &[MapPolynomial], scale: &PolynomialScale<'_>) -> bool {
    if samples > 3
        || scale.black.rows > 1
        || scale.black.cols > 1
        || scale.black.delta_h.iter().chain(&scale.black.delta_v).any(|v| *v != 0.0)
        || polys.iter().any(|p| p.top != 0 || p.left != 0 || p.bottom < active.height || p.right < active.width || p.row_pitch != 1 || p.col_pitch != 1)
    {
        return false;
    }
    let bins = 1usize << scale.bits.min(16);
    let bytes = (bins as u64).saturating_mul(samples as u64).saturating_mul(2);
    if bytes > scale.max_alloc || data.len() < bins.saturating_mul(samples).saturating_mul(2) {
        return false;
    }
    let mut tables = Vec::with_capacity(samples);
    for channel in 0..samples {
        let black = scale.black.at(0, 0, channel, samples);
        let range = scale.white[channel.min(2)] - black;
        if !black.is_finite() || !range.is_finite() || range < 1.0 {
            return false;
        }
        let table = (0..bins)
            .map(|value| {
                let mut v = value as u16;
                for p in polys.iter().filter(|p| p.covers(0, 0, channel)) {
                    let n = (f64::from(v) - f64::from(black)) / f64::from(range);
                    v = (f64::from(black) + p.eval(n) * f64::from(range)).round().clamp(0.0, 65535.0) as u16;
                }
                v
            })
            .collect::<Vec<_>>();
        tables.push(table);
    }
    let row_len = width.saturating_mul(samples);
    let band_rows = par::band_rows(row_len);
    par::chunks_mut(data, row_len.saturating_mul(band_rows), |band, part| {
        for (local, row) in part.chunks_mut(row_len).enumerate() {
            let y = band.saturating_mul(band_rows).saturating_add(local);
            if y < active.y || y - active.y >= active.height {
                continue;
            }
            let start = active.x.saturating_mul(samples);
            let end = active.x.saturating_add(active.width).saturating_mul(samples);
            let Some(row) = row.get_mut(start..end) else { continue };
            for pixel in row.chunks_exact_mut(samples) {
                for (channel, (v, table)) in pixel.iter_mut().zip(&tables).enumerate() {
                    if let Some(&mapped) = table.get(usize::from(*v)) {
                        *v = mapped;
                    } else {
                        let black = scale.black.at(0, 0, channel, samples);
                        let range = scale.white[channel.min(2)] - black;
                        for p in polys.iter().filter(|p| p.covers(0, 0, channel)) {
                            let n = (f64::from(*v) - f64::from(black)) / f64::from(range);
                            *v = (f64::from(black) + p.eval(n) * f64::from(range)).round().clamp(0.0, 65535.0) as u16;
                        }
                    }
                }
            }
        }
    });
    true
}

/// Applies `polys` in order to `data` (`width × height` pixels of `samples` interleaved u16
/// samples), in the sensor's own units: each covered sample is taken to its linear reference value
/// with `black` and `white`, mapped, and brought back.
pub(crate) fn apply_polynomials(data: &mut [u16], width: usize, samples: usize, active: Rect, polys: &[MapPolynomial], scale: PolynomialScale<'_>) {
    let (black, white) = (scale.black, scale.white);
    if polys.is_empty() || width == 0 || samples == 0 {
        return;
    }
    let Some(row_len) = width.checked_mul(samples) else { return };
    if active.x.checked_add(active.width).is_none_or(|n| n > width) || active.y.checked_add(active.height).is_none_or(|n| n > data.len() / row_len) {
        return;
    }
    if polynomial_lookup(data, width, samples, active, polys, &scale) {
        return;
    }
    par::chunks_mut(data, width * samples, |y, row| {
        let Some(ay) = y.checked_sub(active.y).filter(|ay| *ay < active.height) else { return };
        for ax in 0..active.width {
            let x = active.x + ax;
            for s in 0..samples {
                let Some(v) = row.get_mut(x * samples + s) else { return };
                let b = black.at(ax, ay, s, samples);
                let range = white[s.min(2)] - b;
                if range.is_nan() || range < 1.0 {
                    continue;
                }
                for p in polys.iter().filter(|p| p.covers(ax, ay, s)) {
                    let n = (f64::from(*v) - f64::from(b)) / f64::from(range);
                    *v = (f64::from(b) + p.eval(n) * f64::from(range)).round().clamp(0.0, 65535.0) as u16;
                }
            }
        }
    });
}

/// What an opcode list holds: the gain maps and polynomials, and the names of opcodes not applied.
#[derive(Debug, Default)]
pub(crate) struct Opcodes {
    pub gain_maps: Vec<GainMap>,
    pub polynomials: Vec<MapPolynomial>,
    pub skipped: Vec<String>,
}

/// The most MapPolynomial opcodes kept from one list. Real files carry one per plane; every
/// polynomial is checked for every sample, so a crafted list of thousands would hang the open.
/// Those past the cap are named as not applied.
const MAX_POLYNOMIALS: usize = 64;

/// Parses an opcode list.
pub(crate) fn parse(list: &[u8]) -> Opcodes {
    let mut out = Opcodes::default();
    let Some(count) = be32(list, 0) else { return out };
    let mut at = 4usize;
    for _ in 0..count.min(4096) {
        let (Some(id), Some(size)) = (be32(list, at), be32(list, at.saturating_add(12))) else { break };
        let start = at.saturating_add(16);
        let Some(params) = list.get(start..start.saturating_add(size as usize)) else { break };
        let applied = match id {
            9 => gain_map(params).map(|m| out.gain_maps.push(m)).is_some(),
            8 if out.polynomials.len() < MAX_POLYNOMIALS => map_polynomial(params).map(|p| out.polynomials.push(p)).is_some(),
            _ => false,
        };
        if !applied {
            let n = name(id);
            if !out.skipped.contains(&n) {
                out.skipped.push(n);
            }
        }
        at = at.saturating_add(16).saturating_add(size as usize);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_lookup_matches_general_polynomials_including_composition_and_crop() {
        for bits in [8, 16] {
            let (w, h, spp) = (257, 769, 3);
            let active = Rect::new(1, 2, w - 3, h - 4);
            let black = BlackLevels { rows: 1, cols: 1, values: vec![1.0, 2.0, 3.0], delta_h: vec![], delta_v: vec![] };
            let white = [65535.0, 62000.0, 64000.0];
            let all = MapPolynomial {
                top: 0,
                left: 0,
                bottom: active.height,
                right: active.width,
                plane: 0,
                planes: 3,
                row_pitch: 1,
                col_pitch: 1,
                coefficients: vec![0.01, 0.8, 0.2],
            };
            let mut second = all.clone();
            second.plane = 1;
            second.planes = 1;
            second.coefficients = vec![0.04, 0.95];
            let polys = vec![all, second];
            let mut got: Vec<_> = (0..w * h * spp).map(|i| (i & ((1usize << bits) - 1)) as u16).collect();
            let mut want = got.clone();
            apply_polynomials(&mut want, w, spp, active, &polys, PolynomialScale { black: &black, white, bits, max_alloc: 0 });
            assert!(polynomial_lookup(&mut got, w, spp, active, &polys, &PolynomialScale { black: &black, white, bits, max_alloc: 2 << 20 }));
            assert_eq!(got, want, "bits={bits}");
            let mut spatial = black.clone();
            spatial.delta_h = vec![0.0, 1.0];
            assert!(!polynomial_lookup(&mut got, w, spp, active, &polys, &PolynomialScale { black: &spatial, white, bits, max_alloc: 2 << 20 }));
        }
    }

    fn list(ops: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let mut b = (ops.len() as u32).to_be_bytes().to_vec();
        for (id, p) in ops {
            b.extend_from_slice(&id.to_be_bytes());
            b.extend_from_slice(&0x0103_0000u32.to_be_bytes());
            b.extend_from_slice(&1u32.to_be_bytes());
            b.extend_from_slice(&(p.len() as u32).to_be_bytes());
            b.extend_from_slice(p);
        }
        b
    }

    pub(crate) fn gain_map_params(area: [u32; 4], pitch: u32, gains: &[[f32; 2]; 2]) -> Vec<u8> {
        let mut p = Vec::new();
        for v in [area[0], area[1], area[2], area[3], 0, 1, pitch, pitch, 2, 2] {
            p.extend_from_slice(&v.to_be_bytes());
        }
        for v in [1.0f64, 1.0, 0.0, 0.0] {
            p.extend_from_slice(&v.to_be_bytes());
        }
        p.extend_from_slice(&1u32.to_be_bytes());
        for row in gains {
            for g in row {
                p.extend_from_slice(&g.to_bits().to_be_bytes());
            }
        }
        p
    }

    #[test]
    fn parses_gain_maps_and_names_others() {
        let b = list(&[(9, gain_map_params([0, 0, 10, 10], 1, &[[1.0, 2.0], [3.0, 4.0]])), (1, vec![0; 8])]);
        let Opcodes { gain_maps: maps, skipped, .. } = parse(&b);
        assert_eq!(maps.len(), 1);
        assert_eq!(skipped, vec!["WarpRectilinear".to_string()]);
        let m = &maps[0];
        // Bilinear: centre of the 2×2 grid.
        let row = m.row(0.5, 0);
        assert!((m.at(&row, 0.5) - 2.5).abs() < 1e-6);
        assert!((m.at(&m.row(0.0, 0), 0.0) - 1.0).abs() < 1e-6);
        assert!((m.at(&m.row(1.0, 0), 1.0) - 4.0).abs() < 1e-6);
        // Outside the grid clamps.
        assert!((m.at(&m.row(-3.0, 0), 9.0) - 2.0).abs() < 1e-6);
    }

    #[test]
    fn hostile_lists_do_not_panic() {
        let b = list(&[(9, gain_map_params([0, 0, 10, 10], 2, &[[1.0, 2.0], [3.0, 4.0]]))]);
        for n in 0..b.len() {
            let _ = parse(&b[..n]);
        }
        let mut huge = b.clone();
        huge[4 + 16 + 32..4 + 16 + 40].copy_from_slice(&[0xFF; 8]); // points
        assert!(parse(&huge).gain_maps.is_empty());
    }

    fn polynomial_params(pitch: u32, degree: u32, coefficients: &[f64]) -> Vec<u8> {
        let mut p = Vec::new();
        for v in [0, 0, 10, 10, 1, 2, pitch, pitch, degree] {
            p.extend_from_slice(&v.to_be_bytes());
        }
        for c in coefficients {
            p.extend_from_slice(&c.to_be_bytes());
        }
        p
    }

    #[test]
    fn parses_map_polynomials_and_refuses_bad_ones() {
        let ops = parse(&list(&[(8, polynomial_params(1, 2, &[0.1, 0.5, 0.4]))]));
        assert!(ops.skipped.is_empty());
        let p = &ops.polynomials[0];
        assert_eq!((p.plane, p.planes, p.coefficients.clone()), (1, 2, vec![0.1, 0.5, 0.4]));
        assert!((p.eval(0.5) - (0.1 + 0.25 + 0.1)).abs() < 1e-12);
        assert_eq!((p.eval(-1.0), p.eval(9.0)), (0.1, 1.0), "input and output clip to 0..1");
        assert!(p.covers(3, 4, 1) && p.covers(3, 4, 2) && !p.covers(3, 4, 0) && !p.covers(3, 4, 3));
        // A degree above 8, a zero pitch, or too few coefficients: not applied, and named.
        for bad in [polynomial_params(1, 9, &[0.0; 10]), polynomial_params(0, 1, &[0.0, 1.0]), polynomial_params(1, 2, &[0.0, 1.0])] {
            let ops = parse(&list(&[(8, bad)]));
            assert!(ops.polynomials.is_empty());
            assert_eq!(ops.skipped, vec!["MapPolynomial".to_string()]);
        }
    }

    #[test]
    fn map_polynomials_past_the_cap_are_named_not_applied() {
        let ops = parse(&list(&vec![(8, polynomial_params(1, 1, &[0.0, 1.0])); MAX_POLYNOMIALS + 1]));
        assert_eq!(ops.polynomials.len(), MAX_POLYNOMIALS);
        assert_eq!(ops.skipped, vec!["MapPolynomial".to_string()]);
    }

    #[test]
    fn at_rv_matches_row_then_at() {
        // at_rv (direct 4-corner bilinear) must equal the old row() + at() path for a range of
        // sample positions, including points outside the grid that clamp.
        let b = list(&[(9, gain_map_params([0, 0, 10, 10], 1, &[[1.0, 2.0], [3.0, 4.5]]))]);
        let maps = parse(&b).gain_maps;
        let m = &maps[0];
        for &rv in &[-1.0, 0.0, 0.17, 0.5, 0.83, 1.0, 2.0] {
            let row = m.row(rv, 0);
            for &rh in &[-1.0, 0.0, 0.1, 0.5, 0.9, 1.0, 3.0] {
                let want = m.at(&row, rh);
                let got = m.at_rv(rv, 0, rh);
                assert!((got - want).abs() < 1e-6, "rv={rv} rh={rh}: {got} vs {want}");
            }
        }
    }
}
