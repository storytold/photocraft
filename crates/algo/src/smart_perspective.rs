//! A smart object under a one-way perspective (its corners moved so that only one direction
//! converges: Perspective dragged sideways or up and down, with any scale or skew), drawn as the
//! reference application draws one. Measured byte for byte at 8 bits against its renders
//! (horizontal and vertical perspectives growing and shrinking, both edges, impulse probes at
//! every phase), and every one matches:
//!
//! 1. **Bleed** as for scaling ([`crate::smart_scale`]).
//! 2. **The projective direction first:** pass 1 runs along the direction in which the map is
//!    projective (rows when the vanishing point is to a side, else columns, by transposing), pass 2
//!    along the other, where every line is a straight walk.
//! 3. **Pass 1:** each document pixel samples the source line at the exact inverse of its centre,
//!    rounded to the nearest 1/128 sample. Within half a sample of the line's ends the position is
//!    held at the end sample's centre; further out the pixel is transparent. Bicubic Automatic for
//!    the whole quad: Sharper (Keys −1, stretch 1.05) when the quad is smaller than the contents,
//!    else Smoother (Keys −0.625, stretch 1.15); 14-bit weights.
//! 4. **Pass 2:** each document column is the affine walk ([`crate::smart_affine`]) with that
//!    column's own top and length, plain Bicubic, no averaging.
//! 5. **Clip:** the corners' rounded bounding box.
//!
//! **Both directions converging** (Perspective dragged sideways *and* up or down, no scale, skew or
//! rotation) is drawn in two such stages, measured byte for byte on four impulse probes and two
//! renders. In coordinates centred on the contents' centre the map is `x ↦ x / (1 + g·x + h·y)`,
//! which is the horizontal one-way perspective `g` followed by the vertical one `h` (they
//! commute). The one with the larger coefficient goes first, into an intermediate image that is
//! stretched so the second stage never magnifies along its walk: along the second stage's walk
//! axis by the second stage's largest walk magnification (`1 / (1 + h·y)` at the far row of the
//! first quad), along its projective axis by the ratio of the final quad's extent to the first
//! quad's. The intermediate's far edge along the walk axis lands on the whole pixel below the
//! final quad's edge; along the projective axis it lands exactly on the final quad's edge. The
//! second stage then draws the intermediate with the same one-way rules. The two kernels compose
//! into the wide, two-humped weights the reference shows, and the first kernel's phase-dependent
//! centroid, averaged by the second, into its 1/20-pixel ripples.
//!
//! Quads with an affine part as well (scale, skew or rotation with a two-way perspective) are not
//! handled here.

use photocraft_geom::Rect;
use photocraft_raster::Surface;

use crate::smart_affine::{r256, round_half_up, transpose, walk, weight_table};
use crate::smart_scale::{Depth, Image, MAX_SAMPLES, ONE, bleed, keys};

/// Taps of the projective pass: `floor(u) − 3 ..= floor(u) + 3` (Smoother reaches 2.3 samples).
const TAPS: usize = 7;
/// Below this a perspective coefficient counts as zero.
const FLAT: f64 = 1e-9;

/// 14-bit weights of the [`TAPS`] taps for each of the 128 phases, for Keys `a` stretched by `st`.
fn table(a: f64, st: f64) -> Vec<[i64; TAPS]> {
    (0..128)
        .map(|ph| {
            let u = f64::from(ph) / 128.0;
            let w: Vec<f64> = (0..TAPS).map(|t| keys((u + 3.0 - t as f64) / st, a)).collect();
            let total: f64 = w.iter().sum();
            let mut q = [0i64; TAPS];
            if total.abs() < 1e-12 {
                return q;
            }
            for (qq, ww) in q.iter_mut().zip(&w) {
                *qq = (ww / total * ONE as f64).trunc() as i64;
            }
            let mut big = 0;
            for (i, v) in w.iter().enumerate() {
                if w.get(big).is_some_and(|b| v > b) {
                    big = i;
                }
            }
            let others: i64 = q.iter().enumerate().filter(|&(i, _)| i != big).map(|(_, v)| v).sum();
            if let Some(b) = q.get_mut(big) {
                *b = ONE - others;
            }
            q
        })
        .collect()
}

/// Pass 1 (rule 3): row `j` of `img` onto the document columns `x0 .. x0 + count`, the map along
/// the row being `x ↦ (h0 x + h1 y + h2) / (h6 x + 1)` at `y = j + ½`.
fn projective(img: &Image, h: &[f64; 9], x0: i64, count: usize, depth: Depth, table: &[[i64; TAPS]]) -> Option<Image> {
    let (n, nc) = (img.w, img.n);
    let alpha = nc - 1;
    let last = n as i64 - 1;
    let nf = n as f64;
    let mut px = vec![0.0f32; count.checked_mul(img.h)?.checked_mul(nc)?];
    crate::par::chunks_mut(&mut px, count * nc, true, |j, row| {
        let Some(line) = img.px.get(j * n * nc..(j + 1) * n * nc) else { return };
        let y = j as f64 + 0.5;
        let mut acc = vec![0.0f64; nc];
        for (k, out) in row.chunks_exact_mut(nc).enumerate() {
            let xc = (x0 + k as i64) as f64 + 0.5;
            let den = h[0] - xc * h[6];
            let pos = (xc * (h[7] * y + 1.0) - h[1] * y - h[2]) / den - 0.5;
            if !(den > 0.0 && pos.is_finite() && pos >= -0.5 && pos < nf - 0.5) {
                continue; // transparent
            }
            let ph = (pos.clamp(0.0, last as f64) * 128.0 + 0.5).floor() as i64;
            let base = ph.div_euclid(128) - 3;
            let Some(q) = table.get(ph.rem_euclid(128) as usize) else { continue };
            acc.iter_mut().for_each(|v| *v = 0.0);
            for (t, &qq) in q.iter().enumerate() {
                if qq == 0 {
                    continue;
                }
                let i = base + t as i64;
                let inside = (0..=last).contains(&i);
                let at = i.clamp(0, last.max(0)) as usize * nc;
                let Some(sample) = line.get(at..at + nc) else { continue };
                for (ch, v) in sample.iter().enumerate() {
                    if ch == alpha && !inside {
                        continue;
                    }
                    if let Some(a) = acc.get_mut(ch) {
                        *a += f64::from(*v) * qq as f64;
                    }
                }
            }
            for (ch, v) in out.iter_mut().enumerate() {
                *v = depth.settle(acc.get(ch).copied().unwrap_or(0.0), ch == alpha);
            }
        }
    });
    Some(Image { w: count, h: img.h, n: nc, px })
}

/// The same map with x and y swapped (contents and document both transposed).
fn transposed(h: &[f64; 9]) -> [f64; 9] {
    [h[4], h[3], h[5], h[1], h[0], h[2], h[7], h[6], h[8]]
}

/// Draws `img` through `h` (h6 ≠ 0, h7 = 0, h8 = 1): the result and its top-left corner.
fn draw(img: &Image, h: &[f64; 9], depth: Depth) -> Option<(Image, i64, i64)> {
    let (wf, hf) = (img.w as f64, img.h as f64);
    let map = |x: f64, y: f64| {
        let w = h[6] * x + h[7] * y + 1.0;
        ((h[0] * x + h[1] * y + h[2]) / w, (h[3] * x + h[4] * y + h[5]) / w, w)
    };
    let corners = [map(0.0, 0.0), map(wf, 0.0), map(wf, hf), map(0.0, hf)];
    if corners.iter().any(|&(x, y, w)| !(w > 0.0 && x.is_finite() && y.is_finite())) {
        return None;
    }
    let lo = |f: fn(&(f64, f64, f64)) -> f64| corners.iter().map(f).fold(f64::INFINITY, f64::min);
    let hi = |f: fn(&(f64, f64, f64)) -> f64| corners.iter().map(f).fold(f64::NEG_INFINITY, f64::max);
    let (x0, x1) = (round_half_up(lo(|c| c.0)), round_half_up(hi(|c| c.0)));
    let (y0, y1) = (round_half_up(lo(|c| c.1)), round_half_up(hi(|c| c.1)));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let (nx, ny) = ((x1 - x0) as usize, (y1 - y0) as usize);
    if nx.max(img.w).checked_mul(ny.max(img.h))?.checked_mul(img.n)? > MAX_SAMPLES {
        return None;
    }
    // Rule 3's Automatic: the quad's area against the contents'.
    let area = 0.5
        * (0..4)
            .map(|i| {
                let (p, q) = (corners[i], corners[(i + 1) % 4]);
                p.0 * q.1 - q.0 * p.1
            })
            .sum::<f64>()
            .abs();
    let (a, st) = if area < wf * hf { (-1.0, 1.05) } else { (-0.625, 1.15) };
    let inter = projective(img, h, x0, nx, depth, &table(a, st))?;
    // Pass 2: column X samples source x = p y + q; along it y ↦ (A y + B) / Dd (h7 = 0).
    let mut dist0 = Vec::with_capacity(nx);
    let mut lengths = Vec::with_capacity(nx);
    for i in 0..nx {
        let xc = (x0 + i as i64) as f64 + 0.5;
        let den = h[0] - xc * h[6];
        let (p, q) = (-h[1] / den, (xc - h[2]) / den);
        let (aa, bb, dd) = (h[3] * p + h[4], h[3] * q + h[5], h[6] * q + 1.0);
        if !(den > 0.0 && dd > 0.0 && aa > 0.0) {
            return None;
        }
        dist0.push(y0 * 256 - r256(bb / dd));
        lengths.push(r256(hf * aa / dd));
    }
    let out = transpose(&walk(&transpose(&inter), &dist0, ny, &lengths, depth, &weight_table())?);
    Some((out, x0, y0))
}

/// The pixels of `src` inside `bounds` (its contents' canvas; `src` has alpha) drawn through the
/// homography `h` (row-major 3×3, contents → document, `x, y` measured from `bounds`' corner).
/// `None` when the map isn't a one-way perspective (affine, or converging both ways), folds over,
/// or the size passes [`MAX_SAMPLES`].
pub fn render(src: &Surface, bounds: Rect, h: [f64; 9]) -> Option<Surface> {
    let fmt = src.format();
    let nc = src.channels();
    if !fmt.alpha || nc == 0 || bounds.is_empty() || h.iter().any(|v| !v.is_finite()) || h[8].abs() < FLAT {
        return None;
    }
    let h = h.map(|v| v / h[8]);
    // One-way: converging along rows only with each document column a straight walk (h7 = h1 =
    // 0), or the same along columns (h6 = h3 = 0).
    let flat = |v: f64| v.abs() <= FLAT;
    let side = !flat(h[6]) && flat(h[7]) && flat(h[1] / h[0].abs().max(FLAT));
    let down = !flat(h[7]) && flat(h[6]) && flat(h[3] / h[4].abs().max(FLAT));
    if !flat(h[6]) && !flat(h[7]) {
        return two_way(src, bounds, &h);
    }
    if !(side || down) {
        return None;
    }
    let depth = Depth::of(fmt.sample);
    let unit = depth.levels.unwrap_or(1.0);
    let (w, hgt) = (bounds.width() as usize, bounds.height() as usize);
    w.checked_mul(hgt)?.checked_mul(nc)?.le(&MAX_SAMPLES).then_some(())?;
    let mut px = src.read_region(bounds);
    if depth.levels.is_some() {
        crate::par::chunks_mut(&mut px, 1 << 16, true, |_, part| {
            for v in part {
                *v = (v.clamp(0.0, 1.0) * unit).round();
            }
        });
    }
    let mut img = Image { w, h: hgt, n: nc, px };
    bleed(&mut img);
    let (out, x0, y0) = if side {
        draw(&img, &h, depth)?
    } else {
        let (out, y0, x0) = draw(&transpose(&img), &transposed(&h), depth)?;
        (transpose(&out), x0, y0)
    };
    let mut surface = Surface::new(fmt);
    let mut data = out.px;
    crate::par::chunks_mut(&mut data, 1 << 16, true, |_, part| {
        for v in part {
            *v /= unit;
        }
    });
    let rect = Rect::new(i32::try_from(x0).ok()?, i32::try_from(y0).ok()?, i32::try_from(x0 + out.w as i64).ok()?, i32::try_from(y0 + out.h as i64).ok()?);
    surface.write_region_par(rect, &data);
    Some(surface)
}

fn mul(a: &[f64; 9], b: &[f64; 9]) -> [f64; 9] {
    let mut m = [0.0; 9];
    for r in 0..3 {
        for c in 0..3 {
            m[r * 3 + c] = (0..3).map(|k| a[r * 3 + k] * b[k * 3 + c]).sum();
        }
    }
    m
}

fn inverse(m: &[f64; 9]) -> Option<[f64; 9]> {
    let [a, b, c, d, e, f, g, h, i] = *m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if !det.is_finite() || det.abs() < 1e-300 {
        return None;
    }
    let inv = [e * i - f * h, c * h - b * i, b * f - c * e, f * g - d * i, a * i - c * g, c * d - a * f, d * h - e * g, b * g - a * h, a * e - b * d];
    Some(inv.map(|v| v / det))
}

fn translation(x: f64, y: f64) -> [f64; 9] {
    [1.0, 0.0, x, 0.0, 1.0, y, 0.0, 0.0, 1.0]
}

/// The map's image of the box `w × h`: the four corners, or `None` past the horizon.
fn corners(m: &[f64; 9], w: f64, h: f64) -> Option<[(f64, f64); 4]> {
    let mut out = [(0.0, 0.0); 4];
    for (o, (x, y)) in out.iter_mut().zip([(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]) {
        let d = m[6] * x + m[7] * y + m[8];
        if !d.is_finite() || d <= 0.0 {
            return None;
        }
        *o = ((m[0] * x + m[1] * y + m[2]) / d, (m[3] * x + m[4] * y + m[5]) / d);
    }
    out.iter().all(|c| c.0.is_finite() && c.1.is_finite()).then_some(out)
}

/// `[x0, y0, x1, y1]` around the corners.
fn extent(c: &[(f64, f64); 4]) -> [f64; 4] {
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for &(x, y) in c {
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
    }
    [x0, y0, x1, y1]
}

/// The smallest rectangle holding every pixel of `s` with any coverage.
fn alpha_bounds(s: &Surface) -> Option<Rect> {
    let r = s.content_bounds();
    if r.is_empty() {
        return None;
    }
    let nc = s.channels();
    let px = s.read_region(r);
    let w = r.width() as usize;
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
    for (i, p) in px.chunks_exact(nc).enumerate() {
        if p.get(nc - 1).is_some_and(|a| *a > 0.0) {
            let (x, y) = (i % w, i / w);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
        }
    }
    (x1 > 0).then(|| Rect::new(r.x0 + x0 as i32, r.y0 + y0 as i32, r.x0 + x1 as i32, r.y0 + y1 as i32))
}

/// A perspective converging both ways, with no affine part: two one-way stages (see the module
/// docs). `h` is normalised and measured from `bounds`' corner.
fn two_way(src: &Surface, bounds: Rect, h: &[f64; 9]) -> Option<Surface> {
    let flat = |v: f64| v.abs() <= FLAT;
    let (w, hgt) = (f64::from(bounds.width()), f64::from(bounds.height()));
    // In coordinates centred on the contents' centre and on its image: x ↦ (A x + t) / (1 + p·x).
    let (cx, cy) = (w / 2.0, hgt / 2.0);
    let dc = h[6] * cx + h[7] * cy + h[8];
    if !dc.is_finite() || dc <= FLAT {
        return None;
    }
    let (tx, ty) = ((h[0] * cx + h[1] * cy + h[2]) / dc, (h[3] * cx + h[4] * cy + h[5]) / dc);
    let hc = mul(&mul(&translation(-tx, -ty), h), &translation(cx, cy));
    if hc[8].abs() < FLAT {
        return None;
    }
    let hc = hc.map(|v| v / hc[8]);
    let (g, p) = (hc[6], hc[7]);
    if !(flat(hc[0] - 1.0) && flat(hc[1]) && flat(hc[3]) && flat(hc[4] - 1.0) && flat(hc[2]) && flat(hc[5])) {
        return None; // an affine part as well: not measured yet
    }
    let to_doc = translation(tx, ty);
    let to_centre = translation(-w / 2.0, -hgt / 2.0);
    // The larger coefficient goes first; a tie (within float noise of corners read from a file) goes
    // horizontal.
    let hfirst = g.abs() >= p.abs() * (1.0 - 1e-3);
    let first: [f64; 9] = if hfirst { [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, g, 0.0, 1.0] } else { [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, p, 1.0] };
    let m1 = mul(&mul(&to_doc, &first), &to_centre);
    let q1 = corners(&m1, w, hgt)?;
    let fb = extent(&q1);
    let b = extent(&corners(h, w, hgt)?);
    // The stretch: the second stage's largest walk magnification along its walk axis, the extent
    // ratio along its projective axis; placed with the far edge on the pixel below the final edge
    // along the walk axis and exactly on it along the other.
    let snap = |edges: [f64; 2], targets: [f64; 2]| {
        let k = usize::from((edges[1] - targets[1]).abs() < (edges[0] - targets[0]).abs());
        targets[k].floor() - edges[k]
    };
    let s = if hfirst {
        let sx = q1.iter().map(|c| 1.0 / (1.0 + p * (c.1 - ty))).fold(f64::NEG_INFINITY, f64::max);
        let sy = (b[3] - b[1]) / (fb[3] - fb[1]);
        let shift = snap([tx + sx * (fb[0] - tx), tx + sx * (fb[2] - tx)], [b[0], b[2]]);
        [sx, 0.0, tx - sx * tx + shift, 0.0, sy, b[1] - sy * fb[1], 0.0, 0.0, 1.0]
    } else {
        let sy = q1.iter().map(|c| 1.0 / (1.0 + g * (c.0 - tx))).fold(f64::NEG_INFINITY, f64::max);
        let sx = (b[2] - b[0]) / (fb[2] - fb[0]);
        let shift = snap([ty + sy * (fb[1] - ty), ty + sy * (fb[3] - ty)], [b[1], b[3]]);
        [sx, 0.0, b[0] - sx * fb[0], 0.0, sy, ty - sy * ty + shift, 0.0, 0.0, 1.0]
    };
    if s.iter().any(|v| !v.is_finite()) || s[0] <= 0.0 || s[4] <= 0.0 {
        return None;
    }
    let m1s = mul(&s, &m1);
    let inter = render(src, bounds, m1s)?;
    let ib = alpha_bounds(&inter)?;
    let m2 = mul(&mul(h, &inverse(&m1s)?), &translation(f64::from(ib.x0), f64::from(ib.y0)));
    render(&inter, ib, m2)
}

/// `h` (row-major, in the contents' own coordinates) measured instead from the point `(x, y)`, the
/// corner of the box [`render`] takes.
pub fn from_corner(h: [f64; 9], x: f64, y: f64) -> [f64; 9] {
    let [h0, h1, h2, h3, h4, h5, h6, h7, h8] = h;
    [h0, h1, h0 * x + h1 * y + h2, h3, h4, h3 * x + h4 * y + h5, h6, h7, h6 * x + h7 * y + h8]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projective_weights_sum_to_one() {
        for (a, st) in [(-1.0, 1.05), (-0.625, 1.15)] {
            for q in table(a, st) {
                assert_eq!(q.iter().sum::<i64>(), ONE);
            }
        }
    }

    #[test]
    fn affine_and_two_way_maps_are_left_to_other_paths() {
        use photocraft_color::{ColorMode, PixelFormat, SampleType};
        let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
        s.write_region(Rect::new(0, 0, 4, 4), &[1.0; 64]);
        let r = Rect::new(0, 0, 4, 4);
        assert!(render(&s, r, [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]).is_none());
        // Converging both ways about a point other than the contents' centre (an affine part): not drawn.
        assert!(render(&s, r, [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.01, 0.01, 1.0]).is_none());
        // The same about the centre: the two-stage path.
        let about_centre = mul(&mul(&translation(10.0, 10.0), &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.01, 0.005, 1.0]), &translation(-2.0, -2.0));
        assert!(render(&s, r, about_centre).is_some());
        assert!(render(&s, r, [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.01, 0.0, 1.0]).is_some());
        assert!(render(&s, r, [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.01, 1.0]).is_some());
        assert!(render(&s, r, [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, f64::NAN, 0.0, 1.0]).is_none());
        // A one-way perspective turned afterwards: the columns are no longer straight walks.
        assert!(render(&s, r, [0.94, -0.34, 0.0, 0.34, 0.94, 0.0, 0.01, 0.0, 1.0]).is_none());
        assert!(render(&s, r, [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, -1.0, 0.0, 1.0]).is_none());
    }

    /// Renders the cases listed in `$PERSP_CASES` (`name source.rgba h0 .. h8` per line, 128 px
    /// 8-bit sources) into `$PERSP_OUT/<name>.rust.rgba` (320 px, straight 8-bit) for comparison
    /// with the reference's renders.
    #[test]
    #[ignore]
    fn dump_reference_cases() {
        use photocraft_color::{ColorMode, PixelFormat, SampleType};
        let (Ok(cases), Ok(out)) = (std::env::var("PERSP_CASES"), std::env::var("PERSP_OUT")) else { return };
        for line in std::fs::read_to_string(cases).expect("cases").lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            let bytes = std::fs::read(f[1]).expect("source");
            let h: Vec<f64> = f[2..11].iter().map(|v| v.parse().expect("number")).collect();
            let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
            let data: Vec<f32> = bytes.iter().map(|&b| f32::from(b) / 255.0).collect();
            s.write_region(Rect::new(0, 0, 128, 128), &data);
            let h: [f64; 9] = h.try_into().expect("nine");
            let r = render(&s, Rect::new(0, 0, 128, 128), h);
            let px = r.map(|r| r.read_region(Rect::new(0, 0, 320, 320))).unwrap_or_else(|| vec![0.0; 320 * 320 * 4]);
            let b: Vec<u8> = px.iter().map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8).collect();
            std::fs::write(format!("{out}/{}.rust.rgba", f[0]), b).expect("write");
        }
    }

    /// General-quad experiments: each line of `$GEN_CASES` is `name source.rgba depth mode ...`
    /// (128 px 8-bit RGBA source, rendered at `depth` 8 or 16): `direct h0..h8` (the pixel-layer
    /// sampler on the source) or `scaled nw nh ox oy g0..g8` (the smart-object scaling to
    /// `nw x nh` at `(ox, oy)`, then the pixel-layer sampler through `g`). Writes
    /// `$GEN_OUT/<name>.f32` (320 x 320 RGBA, 0..1).
    #[test]
    #[ignore]
    fn dump_general_cases() {
        use crate::transform::{Homography, Interp, warp_surface};
        use photocraft_color::{ColorMode, PixelFormat, SampleType};
        let (Ok(cases), Ok(out)) = (std::env::var("GEN_CASES"), std::env::var("GEN_OUT")) else { return };
        for line in std::fs::read_to_string(cases).expect("cases").lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            let bytes = std::fs::read(f[1]).expect("source");
            let sample = if f[2] == "16" { SampleType::U16 } else { SampleType::U8 };
            let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, sample, true));
            let data: Vec<f32> = bytes.iter().map(|&b| f32::from(b) / 255.0).collect();
            s.write_region(Rect::new(0, 0, 128, 128), &data);
            let nums: Vec<f64> = f[4..].iter().map(|v| v.parse().expect("number")).collect();
            let r = match f[3] {
                "direct" => {
                    let h: [f64; 9] = nums[..9].try_into().expect("nine");
                    warp_surface(&s, Rect::new(0, 0, 128, 128), &Homography(h), Interp::Bicubic).expect("warp")
                }
                _ => {
                    let (nw, nh, ox, oy) = (nums[0] as u32, nums[1] as u32, nums[2] as i32, nums[3] as i32);
                    let inter = crate::smart_scale::scale(&s, Rect::new(0, 0, 128, 128), nw, nh, (ox, oy)).expect("scale");
                    let g: [f64; 9] = nums[4..13].try_into().expect("nine");
                    let ib = Rect::new(ox, oy, ox + nw as i32, oy + nh as i32);
                    warp_surface(&inter, ib, &Homography(g), Interp::Bicubic).expect("warp")
                }
            };
            let px = r.read_region(Rect::new(0, 0, 320, 320));
            let b: Vec<u8> = px.iter().flat_map(|v| v.to_le_bytes()).collect();
            std::fs::write(format!("{out}/{}.f32", f[0]), b).expect("write");
        }
    }
}
