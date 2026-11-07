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
//! General quads (both directions converging, as Distort makes) are not handled here.

use photocraft_geom::Rect;
use photocraft_raster::Surface;

use crate::smart_affine::{r256, round_half_up, transpose, walk, weight_table};
use crate::smart_scale::{Depth, Image, MAX_SAMPLES, ONE, bleed, keys};

/// Taps of the projective pass: `floor(u) − 3 ..= floor(u) + 3` (Smoother reaches 2.3 samples).
const TAPS: usize = 7;
/// Below this a perspective coefficient counts as zero.
const FLAT: f64 = 1e-12;

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
    let (side, down) = (h[6].abs() > FLAT, h[7].abs() > FLAT);
    if side == down {
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
        assert!(render(&s, r, [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.01, 0.01, 1.0]).is_none());
        assert!(render(&s, r, [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.01, 0.0, 1.0]).is_some());
        assert!(render(&s, r, [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.01, 1.0]).is_some());
        assert!(render(&s, r, [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, f64::NAN, 0.0, 1.0]).is_none());
        assert!(render(&s, r, [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, -1.0, 0.0, 1.0]).is_none());
    }

    /// Renders the cases listed in `$PERSP_CASES` (`name source.rgba h0 .. h8` per line, 128 px
    /// 8-bit sources) into `$PERSP_OUT/<name>.rust.rgba` (320 px, straight 8-bit) for comparison
    /// with the reference's renders.
    #[test]
    #[ignore]
    fn dump_reference_cases() {
        use photocraft_color::{ColorMode, PixelFormat, SampleType};
        let cases = std::env::var("PERSP_CASES").expect("PERSP_CASES");
        let out = std::env::var("PERSP_OUT").expect("PERSP_OUT");
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
}
