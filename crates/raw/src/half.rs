//! A reduced-size preview of the development, for the editor's first paint.
//!
//! [`develop_half`] skips the demosaic: every 2×2 block of a Bayer mosaic becomes one output pixel
//! (the red sample, the mean of the two greens, the blue sample), every 3×3 block of an X-Trans
//! mosaic likewise, and LinearRaw data is box-filtered the same way. Linearization, black and
//! white levels, white balance (with the clip), lens-shading gain maps, the colour matrix and the
//! gamma 1.8 encode are fused into one pass, so the cost is a few reads of the sensor and no
//! intermediate frame. The result has the type, colour space and orientation of
//! [`develop_sensor`](crate::develop_sensor)'s, at half (X-Trans: one third) of the size, so a
//! viewer can show it in place of the full image and swap the full one in later.
//!
//! The preview is approximate by design: the Tone sliders and the camera's black point are not
//! applied (a warning says so), and there is no demosaic interpolation, so it is softer than the
//! full image by half a pixel of the preview at most.

use crate::color::{self, Mat3};
use crate::demosaic::Pattern;
use crate::develop::{DevelopOptions, Developed, GammaLut, RawInfo, Scale, gain_row, orient, resolve_neutral};
use crate::error::{RawError, Result};
use crate::sensor::{Cfa, Sensor};
use crate::tone::Tone;
use crate::{RawFormat, par};

/// Develops a reduced-size preview of `s`: half the crop's width and height (floored), or a third
/// for X-Trans. Fails like [`develop_sensor`](crate::develop_sensor) on a malformed sensor, and
/// with [`RawError::Unsupported`] for CFAs other than Bayer and 6×6 X-Trans or when the crop is
/// smaller than one block. `opts.demosaic`, `opts.tone` and the sensor's black point are ignored.
pub fn develop_half(s: &Sensor, opts: &DevelopOptions) -> Result<Developed> {
    let c = s.crop;
    let samples = s.width.checked_mul(s.height).and_then(|n| n.checked_mul(s.samples));
    if c.is_empty()
        || c.x.checked_add(c.width).is_none_or(|x| x > s.width)
        || c.y.checked_add(c.height).is_none_or(|y| y > s.height)
        || samples.is_none_or(|n| s.data.len() < n)
    {
        return Err(RawError::malformed("crop lies outside the sensor data"));
    }
    if s.samples != 1 && s.samples != 3 {
        return Err(RawError::unsupported(format!("{} samples per pixel", s.samples)));
    }
    if let Some(cfa) = &s.cfa
        && (s.samples != 1
            || cfa.width == 0
            || cfa.height == 0
            || cfa.width > 16
            || cfa.height > 16
            || cfa.colors.len() != cfa.width * cfa.height
            || cfa.colors.iter().any(|c| *c > 2))
    {
        return Err(RawError::malformed("invalid sensor CFA pattern"));
    }
    // Block edge: 2 for Bayer and LinearRaw, 3 for X-Trans (every 3×3 block holds all colours).
    let bayer = s.cfa.as_ref().is_none_or(Cfa::is_bayer);
    let b = match &s.cfa {
        Some(cfa) if !bayer => {
            if (cfa.width, cfa.height) != (6, 6) {
                return Err(RawError::unsupported("half-size preview of this colour filter array"));
            }
            3
        }
        _ => 2,
    };
    let (hw, hh) = (c.width / b, c.height / b);
    if hw == 0 || hh == 0 {
        return Err(RawError::unsupported("sensor crop smaller than one preview pixel"));
    }
    // The preview and its oriented copy (u16 RGB each).
    opts.limits.check(hw as u64, hh as u64, 12)?;

    let mut warnings = s.warnings.clone();
    let sc = Scale::new(s);
    let neutral = resolve_neutral(s, opts, &sc, &mut warnings)?;
    let mult = neutral.map(|v| (1.0 / v) as f32);

    // Balanced camera -> XYZ D50, as `develop_sensor` picks it.
    let mono = s.is_monochrome();
    let look = opts.camera_look.as_ref().filter(|l| l.is_usable() && !mono);
    if opts.camera_look.is_some() && look.is_none() && !mono {
        warnings.push("the supplied camera look is not usable (non-finite or singular); ignored".into());
    }
    let to_xyz: Mat3 = match (look, s.color.balanced_to_xyz_d50(neutral)) {
        (Some(l), _) => {
            warnings.push(l.note());
            l.to_xyz
        }
        (None, Some(m)) => m,
        (None, None) => {
            if s.format != RawFormat::Dng && !mono {
                warnings.push("no colour calibration for this camera; camera colours are treated as sRGB primaries".into());
            }
            color::srgb_to_xyz_d50()
        }
    };
    let gain = 2f64.powf(s.baseline_exposure + opts.exposure.clamp(-10.0, 10.0));
    let out_m = color::scale(&color::mul(&color::xyz_d50_to_prophoto(), &to_xyz), gain);
    let out_m: [[f32; 3]; 3] = out_m.map(|r| r.map(|v| v as f32));
    let grey_gain = gain as f32;
    if opts.tone != Tone::default() || s.black_point != 0.0 {
        warnings.push("half-size preview: tone sliders and the black point are not applied".into());
    }
    let lut = match look {
        Some(l) => GammaLut::with_tone(l.tone.as_ref()),
        None => GammaLut::with_curve(&s.tone_curve),
    };

    let rows = RowScaler::new(s, mult);
    // Per block phase (the CFA repeat measured in blocks): the colour of each sample of the block
    // and 1 / the number of samples of each colour.
    let pattern = s.cfa.as_ref().map(|cfa| Pattern::of(cfa, c.x, c.y));
    let phases = pattern.as_ref().map(|p| Phases::new(p, b)).transpose()?;

    // Camera RGB (balanced, 0..1) -> linear ProPhoto.
    let to_pro = |p: [f32; 3]| if mono { [grey_gain * p[0]; 3] } else { [0, 1, 2].map(|k| out_m[k][0] * p[0] + out_m[k][1] * p[1] + out_m[k][2] * p[2]) };
    let fast = BayerRows::new(s, &rows, bayer && b == 2 && s.gain_maps.is_empty() && rows.periodic);
    let mut out = vec![0u16; hw * hh * 3];
    let band = par::band_rows(hw * 3);
    par::chunks_mut(&mut out, band * hw * 3, |bi, chunk| {
        let n = hw * b;
        let planes = if phases.is_some() { 1 } else { s.samples };
        let mut lines = vec![0f32; planes * b * n];
        let mut lin_row = vec![0f32; hw * 3];
        for (r, orow) in chunk.chunks_exact_mut(hw * 3).enumerate() {
            let j = bi * band + r;
            let y0 = c.y + j * b;
            if let Some(fast) = &fast {
                fast.row(y0, hw, &mut |i, p| lin_row[i * 3..i * 3 + 3].copy_from_slice(&to_pro(p)));
                encode_row(&lut, &mut lin_row, orow);
                continue;
            }
            for p in 0..planes {
                for dy in 0..b {
                    let line = &mut lines[(p * b + dy) * n..][..n];
                    match (&s.cfa, planes) {
                        (Some(cfa), _) => rows.fill(y0 + dy, c.x, line, 0, |x, y| usize::from(cfa.color(x, y)).min(2)),
                        (None, _) => rows.fill(y0 + dy, c.x, line, p, |_, _| p),
                    }
                }
            }
            let mut put = |i: usize, p: [f32; 3]| lin_row[i * 3..i * 3 + 3].copy_from_slice(&to_pro(p));
            match &phases {
                Some(ph) if b == 2 => ph.reduce::<2>(j, hw, &lines, n, &mut put),
                Some(ph) => ph.reduce::<3>(j, hw, &lines, n, &mut put),
                None => {
                    let inv = 1.0 / (b * b) as f32;
                    for i in 0..hw {
                        let mut m = [0f32; 3];
                        for (p, mp) in m.iter_mut().enumerate().take(planes) {
                            let base = p * b * n + i * b;
                            let mut sum = 0.0;
                            for dy in 0..b {
                                sum += lines[base + dy * n..base + dy * n + b].iter().sum::<f32>();
                            }
                            *mp = sum * inv;
                        }
                        if planes == 1 {
                            m = [m[0]; 3];
                        }
                        put(i, m);
                    }
                }
            }
            encode_row(&lut, &mut lin_row, orow);
        }
    });

    let (out, ow, oh) = if opts.orient { orient(out, hw, hh, s.orientation) } else { (out, hw, hh) };
    let names = |p: &[u8]| p.iter().map(|c| ['R', 'G', 'B'][usize::from(*c).min(2)]).collect::<String>();
    let cfa_name = match (&s.cfa, &pattern) {
        (Some(cfa), _) if bayer => Some(names(&cfa.phase(c.x, c.y))),
        (_, Some(pat)) => Some(pat.colors.chunks(pat.w.max(1)).map(names).collect::<Vec<_>>().join("/")),
        _ => None,
    };
    let nmax = mult.iter().cloned().fold(f32::MAX, f32::min).max(1e-6);
    Ok(Developed {
        width: ow as u32,
        height: oh as u32,
        rgb: out,
        info: RawInfo {
            format: s.format,
            make: s.make.clone(),
            model: s.model.clone(),
            sensor_width: s.width,
            sensor_height: s.height,
            cfa: cfa_name,
            wb_multipliers: mult.map(|m| f64::from(m / nmax)),
            orientation: s.orientation,
            baseline_exposure: s.baseline_exposure,
        },
        warnings,
    })
}

/// Encodes a row of linear ProPhoto values (`lin` is clobbered): the square roots run as one
/// vectorised pass, then the table lookups.
fn encode_row(lut: &GammaLut, lin: &mut [f32], out: &mut [u16]) {
    for v in lin.iter_mut() {
        *v = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0).sqrt() };
    }
    for (o, v) in out.iter_mut().zip(lin.iter()) {
        *o = lut.encode_sqrt(*v);
    }
}

/// Colour of each sample of a block, per block phase of the CFA repeat.
struct Phases {
    /// Repeat size in blocks.
    w: usize,
    h: usize,
    /// `w * h` blocks of `b * b` colours.
    colors: Vec<u8>,
    /// `w * h` blocks of 1 / count per colour.
    inv: Vec<[f32; 3]>,
}

impl Phases {
    fn new(p: &Pattern, b: usize) -> Result<Self> {
        let (w, h) = (p.w, p.h);
        let mut colors = Vec::with_capacity(w * h * b * b);
        let mut inv = Vec::with_capacity(w * h);
        for pj in 0..h {
            for pi in 0..w {
                let mut count = [0u32; 3];
                for dy in 0..b {
                    for dx in 0..b {
                        let c = p.colors[((pj * b + dy) % h) * w + (pi * b + dx) % w].min(2);
                        colors.push(c);
                        count[usize::from(c)] += 1;
                    }
                }
                if count.contains(&0) {
                    return Err(RawError::unsupported("half-size preview: a block of the colour filter array lacks a colour"));
                }
                inv.push(count.map(|n| 1.0 / n as f32));
            }
        }
        Ok(Phases { w, h, colors, inv })
    }

    /// Averages the `B × B` blocks of output row `j` (`hw` blocks) from `lines` (`B` rows of
    /// `n` samples) per colour and hands each pixel to `put`.
    #[inline]
    fn reduce<const B: usize>(&self, j: usize, hw: usize, lines: &[f32], n: usize, put: &mut impl FnMut(usize, [f32; 3])) {
        let row = (j % self.h) * self.w;
        let mut ph = 0;
        for i in 0..hw {
            let cols = &self.colors[(row + ph) * B * B..][..B * B];
            let mut sum = [0f32; 3];
            for dy in 0..B {
                for dx in 0..B {
                    sum[usize::from(cols[dy * B + dx])] += lines[dy * n + i * B + dx];
                }
            }
            let inv = self.inv[row + ph];
            put(i, [sum[0] * inv[0], sum[1] * inv[1], sum[2] * inv[2]]);
            ph += 1;
            if ph == self.w {
                ph = 0;
            }
        }
    }
}

/// The common case, a Bayer mosaic with periodic black levels and no gain maps: each 2×2 block is
/// read, scaled and combined straight from the sensor rows, with no row buffers.
struct BayerRows<'a> {
    s: &'a Sensor,
    rows: &'a RowScaler<'a>,
    /// Positions (0..4, row-major in the block) of red, blue and the two greens.
    roles: [usize; 4],
}

impl<'a> BayerRows<'a> {
    fn new(s: &'a Sensor, rows: &'a RowScaler<'a>, usable: bool) -> Option<Self> {
        let cfa = s.cfa.as_ref().filter(|_| usable)?;
        let p = cfa.phase(s.crop.x, s.crop.y);
        let at = |c: u8| p.iter().position(|v| *v == c);
        let greens: Vec<usize> = (0..4).filter(|i| p[*i] == 1).collect();
        match (at(0), at(2), greens.as_slice()) {
            (Some(r), Some(b), [g0, g1]) => Some(BayerRows { s, rows, roles: [r, b, *g0, *g1] }),
            _ => None,
        }
    }

    /// Emits the `hw` pixels made from data rows `y0` and `y0 + 1`.
    fn row(&self, y0: usize, hw: usize, put: &mut impl FnMut(usize, [f32; 3])) {
        let (s, c) = (self.s, self.s.crop);
        let bl = &s.black;
        let mut k = [(0f32, 0f32); 4];
        for (pos, kp) in k.iter_mut().enumerate() {
            let (x, y) = (c.x + pos % 2, y0 + pos / 2);
            let black = bl.at(x.saturating_sub(s.active.x), y.saturating_sub(s.active.y), 0, 1);
            let range = s.white[0] - black;
            let gain = self.rows.mult[usize::from(s.cfa.as_ref().map_or(1, |f| f.color(x, y))).min(2)] / range;
            *kp = if range >= 1.0 && black.is_finite() && gain.is_finite() { (black, gain) } else { (0.0, 0.0) };
        }
        let start = y0 * s.width + c.x;
        let (r0, r1) = (&s.data[start..start + 2 * hw], &s.data[start + s.width..start + s.width + 2 * hw]);
        match s.linearization.as_deref() {
            None => self.blocks(r0, r1, k, hw, f32::from, put),
            Some(l) => self.blocks(r0, r1, k, hw, |raw| f32::from(l.get(usize::from(raw)).or_else(|| l.last()).copied().unwrap_or(raw)), put),
        }
    }

    #[inline(always)]
    fn blocks(&self, r0: &[u16], r1: &[u16], k: [(f32, f32); 4], hw: usize, conv: impl Fn(u16) -> f32, put: &mut impl FnMut(usize, [f32; 3])) {
        let [pr, pb, g0, g1] = self.roles;
        for (i, (a, b)) in r0.as_chunks::<2>().0.iter().zip(r1.as_chunks::<2>().0).enumerate().take(hw) {
            let raw = [a[0], a[1], b[0], b[1]];
            let v: [f32; 4] = std::array::from_fn(|n| ((conv(raw[n]) - k[n].0) * k[n].1).clamp(0.0, 1.0));
            put(i, [v[pr], (v[g0] + v[g1]) * 0.5, v[pb]]);
        }
    }
}

/// Black level, range and white balance applied to rows of raw samples: `(raw - black) * gain`,
/// clamped to 0..1 (the same per-sample clip as the full development).
struct RowScaler<'a> {
    s: &'a Sensor,
    sc: Scale<'a>,
    mult: [f32; 3],
    /// Black varies at most per 2×2 position, so black and gain repeat every `period` columns.
    periodic: bool,
    period: usize,
}

impl<'a> RowScaler<'a> {
    fn new(s: &'a Sensor, mult: [f32; 3]) -> Self {
        let bl = &s.black;
        let width = s.cfa.as_ref().map_or(1, |c| c.width);
        let periodic = bl.delta_h.is_empty() && bl.delta_v.is_empty() && matches!(bl.rows, 1 | 2) && matches!(bl.cols, 1 | 2) && width <= 16;
        let period = if width.is_multiple_of(2) { width.max(2) } else { width * 2 };
        RowScaler { s, sc: Scale::new(s), mult, periodic, period }
    }

    /// Fills `out` with the scaled `sample` of data row `y`, columns `x0..x0 + out.len()`;
    /// `channel(x, y)` is the white-balance channel of a pixel.
    fn fill(&self, y: usize, x0: usize, out: &mut [f32], sample: usize, channel: impl Fn(usize, usize) -> usize) {
        let s = self.s;
        let spp = s.samples;
        if self.periodic {
            let bl = &s.black;
            let k: Vec<(f32, f32)> = (0..self.period)
                .map(|p| {
                    let x = x0 + p;
                    let black = bl.at(x.saturating_sub(s.active.x), y.saturating_sub(s.active.y), sample, spp);
                    let range = s.white[sample.min(2)] - black;
                    let gain = self.mult[channel(x, y).min(2)] / range;
                    // Unusable levels give 0 instead of NaN (which `clamp` would pass through).
                    if range >= 1.0 && black.is_finite() && gain.is_finite() { (black, gain) } else { (0.0, 0.0) }
                })
                .collect();
            let start = (y * s.width + x0) * spp + sample;
            let src = &s.data[start..start + (out.len().saturating_sub(1)) * spp + 1];
            let lin = s.linearization.as_deref();
            for (oc, sc) in out.chunks_mut(self.period).zip(src.chunks(self.period * spp)) {
                for (idx, (o, &(black, gain))) in oc.iter_mut().zip(&k).enumerate() {
                    let raw = sc[idx * spp];
                    let v = match lin {
                        Some(l) => f32::from(l.get(usize::from(raw)).or_else(|| l.last()).copied().unwrap_or(raw)),
                        None => f32::from(raw),
                    };
                    *o = ((v - black) * gain).clamp(0.0, 1.0);
                }
            }
        } else {
            for (i, o) in out.iter_mut().enumerate() {
                let x = x0 + i;
                *o = (self.sc.value(x, y, sample) * self.mult[channel(x, y).min(2)]).clamp(0.0, 1.0);
            }
        }
        // Lens-shading gain maps, after the white-balance clip.
        if let Some(g) = gain_row(s, y, x0, out.len(), sample) {
            for (v, g) in out.iter_mut().zip(g) {
                *v = (*v * g).clamp(0.0, 1.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensor::{BlackLevels, Rect};
    use crate::{ColorInfo, Demosaic, WhiteBalance, develop_sensor};

    const BLACK: f32 = 256.0;
    const RANGE: f32 = 4000.0;

    /// A mosaic (or `samples`-per-pixel LinearRaw) sensor whose sample at (x, y) of colour `c` is
    /// `BLACK + scene(x, y)[c] * RANGE`.
    fn sensor(w: usize, h: usize, cfa: Option<Cfa>, samples: usize, scene: impl Fn(usize, usize) -> [f32; 3]) -> Sensor {
        let mut data = Vec::with_capacity(w * h * samples);
        for y in 0..h {
            for x in 0..w {
                let rgb = scene(x, y);
                for k in 0..samples {
                    let c = cfa.as_ref().map_or(k, |cfa| usize::from(cfa.color(x, y)));
                    data.push((BLACK + rgb[c] * RANGE).round() as u16);
                }
            }
        }
        Sensor {
            format: RawFormat::Dng,
            make: None,
            model: None,
            width: w,
            height: h,
            samples,
            data,
            cfa,
            linearization: None,
            black: BlackLevels::uniform(BLACK),
            white: [BLACK + RANGE; 3],
            active: Rect::new(0, 0, w, h),
            crop: Rect::new(0, 0, w, h),
            color: ColorInfo::default(),
            camera_wb: None,
            orientation: 1,
            baseline_exposure: 0.0,
            gain_maps: Vec::new(),
            tone_curve: Vec::new(),
            black_point: 0.0,
            warnings: Vec::new(),
        }
    }

    fn opts() -> DevelopOptions {
        DevelopOptions { demosaic: Demosaic::Bilinear, white_balance: WhiteBalance::Multipliers([2.0, 1.0, 1.5]), ..Default::default() }
    }

    fn xtrans() -> Cfa {
        let rows = ["GGRGGB", "GGBGGR", "BRGRBG", "GGBGGR", "GGRGGB", "RBGBRG"];
        let colors = rows
            .iter()
            .flat_map(|r| {
                r.bytes().map(|c| {
                    if c == b'R' {
                        0
                    } else if c == b'G' {
                        1
                    } else {
                        2
                    }
                })
            })
            .collect();
        Cfa { width: 6, height: 6, colors, origin_x: 0, origin_y: 0 }
    }

    /// Flat colour patches of `cell` × `cell` pixels.
    fn chart(x: usize, y: usize, cell: usize) -> [f32; 3] {
        const P: [[f32; 3]; 6] = [[0.8, 0.1, 0.1], [0.1, 0.7, 0.2], [0.05, 0.1, 0.9], [0.5, 0.5, 0.5], [0.9, 0.8, 0.1], [0.02, 0.02, 0.02]];
        P[(x / cell + 3 * (y / cell)) % 6]
    }

    /// Max code difference between the preview and the full development over the pixels at least
    /// `margin` full-size pixels away from a patch edge, plus the number of pixels compared.
    fn chart_error(cfa: Option<Cfa>, b: usize, cell: usize, margin: usize) -> (i32, usize) {
        let (w, h) = (cell * 6, cell * 4);
        let s = sensor(w, h, cfa, 1, |x, y| chart(x, y, cell));
        let half = develop_half(&s, &opts()).unwrap();
        let full = develop_sensor(&s, &opts()).unwrap();
        assert_eq!((half.width as usize, half.height as usize), (w / b, h / b));
        let (mut err, mut n) = (0, 0);
        for j in 0..h / b {
            for i in 0..w / b {
                let (x, y) = (i * b, j * b);
                let near = |v: usize| (v % cell).min(cell - 1 - (v + b - 1) % cell) < margin;
                if near(x) || near(y) {
                    continue;
                }
                for k in 0..3 {
                    let a = i32::from(half.rgb[(j * (w / b) + i) * 3 + k]);
                    let f = i32::from(full.rgb[(y * w + x) * 3 + k]);
                    err = err.max((a - f).abs());
                }
                n += 1;
            }
        }
        (err, n)
    }

    #[test]
    fn bayer_chart_matches_the_full_development_inside_patches() {
        for p in ["RGGB", "GRBG", "GBRG", "BGGR"] {
            let (err, n) = chart_error(Cfa::bayer(p), 2, 24, 3);
            assert!(n > 100);
            assert!(err <= 2, "{p}: max error {err} codes");
        }
    }

    #[test]
    fn xtrans_chart_matches_the_full_development_inside_patches() {
        let (err, n) = chart_error(Some(xtrans()), 3, 24, 4);
        assert!(n > 50);
        assert!(err <= 2, "max error {err} codes");
    }

    #[test]
    fn bayer_gradient_stays_close_to_the_full_development() {
        let (w, h) = (96, 64);
        let s = sensor(w, h, Cfa::bayer("RGGB"), 1, |x, y| [x as f32 / w as f32, 0.2 + 0.6 * y as f32 / h as f32, 1.0 - x as f32 / w as f32]);
        let half = develop_half(&s, &opts()).unwrap();
        let full = develop_sensor(&s, &opts()).unwrap();
        assert_eq!((half.width, half.height), (48, 32));
        let mut err = 0;
        for j in 1..31 {
            for i in 1..47 {
                for k in 0..3 {
                    // The full image box-filtered at the same 2×2 block (it is centred half a
                    // full pixel off the block's top left, hence the 2×2 mean).
                    let at = |dx: usize, dy: usize| i32::from(full.rgb[((2 * j + dy) * w + 2 * i + dx) * 3 + k]);
                    let mean = (at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1)) / 4;
                    err = err.max((i32::from(half.rgb[(j * 48 + i) * 3 + k]) - mean).abs());
                }
            }
        }
        // 65535 codes full scale: the preview is not demosaiced, so a gradient differs by the
        // chroma it resolves differently, about 1.1 % at most.
        assert!(err < 800, "max error {err} codes");
        assert_eq!(half.info.cfa.as_deref(), Some("RGGB"));
        assert_eq!(half.info.wb_multipliers, full.info.wb_multipliers);
    }

    #[test]
    fn sizes_and_orientations() {
        for (w, h) in [(7, 5), (8, 6), (2, 2), (3, 2), (2, 3)] {
            for o in 1..=8u16 {
                let mut s = sensor(w, h, Cfa::bayer("RGGB"), 1, |x, _| [x as f32 / 8.0; 3]);
                s.orientation = o;
                let d = develop_half(&s, &opts()).unwrap();
                let (hw, hh) = (w / 2, h / 2);
                let want = if o >= 5 { (hh, hw) } else { (hw, hh) };
                assert_eq!((d.width as usize, d.height as usize), want, "{w}x{h} orientation {o}");
                assert_eq!(d.rgb.len(), want.0 * want.1 * 3);
            }
        }
        // Same orientation handling as the full path: a left-to-right ramp, rotated.
        let mut s = sensor(8, 4, Cfa::bayer("RGGB"), 1, |x, _| [x as f32 / 8.0; 3]);
        let plain = develop_half(&s, &opts()).unwrap();
        s.orientation = 6;
        let turned = develop_half(&s, &opts()).unwrap();
        assert_eq!((turned.width, turned.height), (2, 4));
        // Output (x, y) of orientation 6 comes from source (y, h - 1 - x).
        for y in 0..4usize {
            for x in 0..2usize {
                let (sx, sy) = crate::develop::source_xy(6, 4, 2, x, y);
                assert_eq!(turned.rgb[(y * 2 + x) * 3], plain.rgb[(sy * 4 + sx) * 3]);
            }
        }
    }

    #[test]
    fn crop_origin_and_odd_crop() {
        // Crop starting on the second column/row: the pattern is seen shifted.
        let mut s = sensor(20, 20, Cfa::bayer("RGGB"), 1, |x, y| chart(x, y, 10));
        s.crop = Rect::new(1, 1, 17, 15);
        let d = develop_half(&s, &opts()).unwrap();
        assert_eq!((d.width, d.height), (8, 7));
        assert_eq!(d.info.cfa.as_deref(), Some("BGGR"));
    }

    #[test]
    fn linear_raw_and_monochrome() {
        let (w, h) = (24, 16);
        let rgb = sensor(w, h, None, 3, |x, y| chart(x, y, 8));
        let half = develop_half(&rgb, &opts()).unwrap();
        let full = develop_sensor(&rgb, &opts()).unwrap();
        assert_eq!((half.width, half.height, half.info.cfa), (12, 8, None));
        let mut err = 0;
        for j in 0..8usize {
            for i in 0..12usize {
                if [(i * 2) % 8, (j * 2) % 8].iter().any(|v| *v == 0 || *v == 7) {
                    continue;
                }
                for k in 0..3 {
                    err = err.max((i32::from(half.rgb[(j * 12 + i) * 3 + k]) - i32::from(full.rgb[(2 * j * w + 2 * i) * 3 + k])).abs());
                }
            }
        }
        assert!(err <= 2, "max error {err} codes");
        let mono = sensor(w, h, None, 1, |x, _| [x as f32 / w as f32; 3]);
        let d = develop_half(&mono, &opts()).unwrap();
        assert_eq!((d.width, d.height), (12, 8));
        assert!(d.rgb.as_chunks::<3>().0.iter().all(|p| p[0] == p[1] && p[1] == p[2]));
    }

    #[test]
    fn varying_black_levels_take_the_general_path_with_the_same_result() {
        let s = sensor(32, 32, Cfa::bayer("RGGB"), 1, |x, y| chart(x, y, 8));
        let fast = develop_half(&s, &opts()).unwrap();
        let mut slow = s.clone();
        slow.black.delta_h = vec![0.0; 32];
        let general = develop_half(&slow, &opts()).unwrap();
        let worst = fast.rgb.iter().zip(&general.rgb).map(|(a, b)| (i32::from(*a) - i32::from(*b)).abs()).max();
        assert!(worst <= Some(1), "{worst:?}");
    }

    #[test]
    fn unusable_levels_give_black_not_a_panic() {
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            for white_too in [false, true] {
                let mut s = sensor(16, 16, Cfa::bayer("RGGB"), 1, |x, y| chart(x, y, 8));
                s.black = BlackLevels::uniform(bad);
                if white_too {
                    s.white = [bad; 3];
                }
                let d = develop_half(&s, &opts()).unwrap();
                assert!(d.rgb.iter().all(|v| *v == 0), "{bad} {white_too}");
            }
        }
        let mut s = sensor(16, 16, None, 3, |x, y| chart(x, y, 8));
        s.black = BlackLevels { rows: 2, cols: 2, values: vec![f32::NAN, 0.0, 1.0, f32::NAN], delta_h: Vec::new(), delta_v: Vec::new() };
        assert!(develop_half(&s, &opts()).is_ok());
    }

    #[test]
    fn hostile_sensors_do_not_panic() {
        for (w, h) in [(1, 1), (1, 9), (9, 1), (2, 1), (3, 3), (5, 2)] {
            for cfa in [Cfa::bayer("RGGB"), Some(xtrans()), None] {
                let s = sensor(w, h, cfa, 1, |x, y| chart(x, y, 2));
                let _ = develop_half(&s, &opts());
            }
        }
        let mut s = sensor(8, 8, Cfa::bayer("RGGB"), 1, |x, y| chart(x, y, 2));
        s.crop = Rect::new(4, 4, 8, 8);
        assert!(develop_half(&s, &opts()).is_err());
        s.crop = Rect::new(usize::MAX, 0, 2, 2);
        assert!(develop_half(&s, &opts()).is_err());
        s.crop = Rect::new(0, 0, 8, 8);
        s.data.truncate(10);
        assert!(develop_half(&s, &opts()).is_err());
        let mut s = sensor(8, 8, Cfa::bayer("RGGB"), 1, |_, _| [0.5; 3]);
        s.samples = 2;
        assert!(develop_half(&s, &opts()).is_err());
        let mut s = sensor(8, 8, Cfa::bayer("RGGB"), 1, |_, _| [0.5; 3]);
        s.cfa = Some(Cfa { width: 3, height: 2, colors: vec![0, 1, 2, 1, 1, 7], origin_x: 0, origin_y: 0 });
        assert!(develop_half(&s, &opts()).is_err());
        s.cfa = Some(Cfa { width: 0, height: 0, colors: Vec::new(), origin_x: 0, origin_y: 0 });
        assert!(develop_half(&s, &opts()).is_err());
        // A non-Bayer, non-X-Trans pattern is unsupported, not wrong.
        let mut s = sensor(8, 8, Cfa::bayer("RGGB"), 1, |_, _| [0.5; 3]);
        s.cfa = Some(Cfa { width: 4, height: 2, colors: vec![0, 1, 2, 1, 1, 0, 1, 2], origin_x: 0, origin_y: 0 });
        assert!(matches!(develop_half(&s, &opts()), Err(RawError::Unsupported(_))));
    }

    #[test]
    fn working_memory_limits_apply() {
        let s = sensor(64, 64, Cfa::bayer("RGGB"), 1, |_, _| [0.5; 3]);
        let mut o = opts();
        o.limits.max_alloc = 32 * 32 * 12 - 1;
        assert!(matches!(develop_half(&s, &o), Err(RawError::LimitExceeded(_))));
        o.limits.max_alloc = 32 * 32 * 12;
        assert!(develop_half(&s, &o).is_ok());
        o.limits.max_pixels = 100;
        assert!(develop_half(&s, &o).is_err());
    }

    #[test]
    fn unapplied_tone_is_reported() {
        let s = sensor(8, 8, Cfa::bayer("RGGB"), 1, |_, _| [0.5; 3]);
        assert!(develop_half(&s, &opts()).unwrap().warnings.iter().all(|w| !w.contains("not applied")));
        let mut o = opts();
        o.tone.shadows = 30.0;
        assert!(develop_half(&s, &o).unwrap().warnings.iter().any(|w| w.contains("not applied")));
    }
}
