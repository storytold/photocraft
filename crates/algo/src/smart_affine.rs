//! A smart object under an affine transform (rotation, skew, scale with either), drawn as the
//! reference application draws one. Measured byte for byte at 8 bits against 44 of its renders
//! (rotations from 5° to 80°, skews, rotations with scale from 70 % to 120 %, skews with shrinks
//! from 30 % to 75 %), and every one matches:
//!
//! 1. **Bleed** as for scaling ([`crate::smart_scale`]).
//! 2. **Quarter turns first:** past 45° the source is turned by whole quarter turns (an exact
//!    pixel shuffle), so the passes see a rotation within ±45°.
//! 3. **Two passes:** pass 1 resamples each source row onto the document columns
//!    (`x' = a x + c y + e`), pass 2 each document column onto the document rows. Plain Bicubic
//!    (Keys −0.75, unstretched), 14-bit weights, colour straight with the edges repeated, alpha
//!    transparent beyond; each pass rounds to the sample levels.
//! 4. **Positions in fixed point:** each line's shift and scaled length are rounded to 1/256 of a
//!    document pixel. A pass that keeps its scale shifts each line by whole 1/128 phases (the
//!    floor). Otherwise a line walks its pixels from the first one of the clip box: the start
//!    rounded up and the step truncated, both to 2⁻³² of a source sample, the phase the floor to
//!    1/128.
//! 5. **Strong shrinks:** a pass shrinking below 0.7 first averages its lines down to
//!    `round(n·scale/0.7)` samples, each the plain mean of the samples whose centres it covers.
//! 6. **Clip:** columns between the corners' rounded extremes; rows between the rounded column
//!    tops at the first and last column and the bottoms below them.
//!
//! Mirror flips (a negative determinant) are not handled here.

use photocraft_geom::Rect;
use photocraft_raster::Surface;

use crate::smart_scale::{Depth, Image, MAX_SAMPLES, ONE, bleed, keys};

/// Below this scale a pass first averages its lines down (rule 5).
const SHRINK_LIMIT: f64 = 0.7;
/// Fractional bits of a phase (1/128 sample) in the line walk: 32 bits of a sample in all.
const WALK_BITS: u32 = 25;

/// 14-bit Bicubic weights of the six taps `floor(u) − 2 ..= floor(u) + 3` for each of the 128
/// phases of `u`.
pub(crate) fn weight_table() -> Vec<[i64; 6]> {
    (0..128)
        .map(|ph| {
            let u = f64::from(ph) / 128.0;
            let w: Vec<f64> = (0..6).map(|t| keys(u + 2.0 - f64::from(t), -0.75)).collect();
            let total: f64 = w.iter().sum();
            let mut q = [0i64; 6];
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

/// Rounds to whole 1/256 px, halves up.
pub(crate) fn r256(v: f64) -> i64 {
    (v * 256.0 + 0.5).floor() as i64
}

pub(crate) fn round_half_up(v: f64) -> i64 {
    (v + 0.5).floor() as i64
}

/// The phase indices (1/128 sample) of `count` consecutive pixels of a line whose first pixel lies
/// `dist0` (1/256 px) from the line's origin; `length` (1/256 px) covers `n` samples (rule 4).
fn line_phases(dist0: i64, count: usize, length: i64, n: usize) -> Vec<i64> {
    if length <= 0 {
        return Vec::new();
    }
    if length == n as i64 * 256 {
        return (0..count as i64).map(|k| ((dist0 + 256 * k) * 128).div_euclid(256)).collect();
    }
    let num = n as i128 * 128 * 256;
    let len = i128::from(length);
    let step = (num << WALK_BITS) / len;
    let exact = (i128::from(dist0) * num) << WALK_BITS;
    let den = len * 256;
    let start = -(-exact).div_euclid(den);
    (0..count as i128).map(|k| (start + k * step).div_euclid(1 << WALK_BITS) as i64).collect()
}

/// Rows of `img` (its lines) averaged down to `n1` samples each (rule 5).
fn reduce(img: &Image, n1: usize, depth: Depth) -> Image {
    let (w, h, n) = (img.w, img.h, img.n);
    let node: Vec<usize> = (0..w).map(|r| (((r + 1) * n1).saturating_sub(1) / w).min(n1.saturating_sub(1))).collect();
    let mut px = vec![0.0f32; n1 * h * n];
    crate::par::chunks_mut(&mut px, n1 * n, true, |y, row| {
        let mut sums = vec![0.0f64; n1 * n];
        let mut counts = vec![0usize; n1];
        for (r, &j) in node.iter().enumerate() {
            if let Some(c) = counts.get_mut(j) {
                *c += 1;
            }
            for ch in 0..n {
                if let Some(s) = sums.get_mut(j * n + ch) {
                    *s += f64::from(img.at(r, y, ch));
                }
            }
        }
        for (j, out) in row.chunks_exact_mut(n).enumerate() {
            let c = counts.get(j).copied().unwrap_or(0);
            for (ch, v) in out.iter_mut().enumerate() {
                *v = depth.mean(sums.get(j * n + ch).copied().unwrap_or(0.0), c);
            }
        }
    });
    Image { w: n1, h, n, px }
}

/// One pass along the rows of `img`: row `y` produces `count` pixels, the first lying `dist0[y]`
/// (1/256 px) from that row's origin, the row's `img.w` samples spanning `length` (1/256 px).
fn pass(img: &Image, dist0: &[i64], count: usize, length: i64, depth: Depth, table: &[[i64; 6]]) -> Option<Image> {
    let scale = length as f64 / (img.w as f64 * 256.0);
    if scale < SHRINK_LIMIT {
        let n1 = round_half_up(img.w as f64 * scale / SHRINK_LIMIT).max(1) as usize;
        return walk(&reduce(img, n1, depth), dist0, count, &vec![length; img.h], depth, table);
    }
    walk(img, dist0, count, &vec![length; img.h], depth, table)
}

/// The walk of rule 4 along the rows of `img`, no averaging: row `y` produces `count` pixels, the
/// first lying `dist0[y]` (1/256 px) from that row's origin, its `img.w` samples spanning
/// `lengths[y]` (1/256 px).
pub(crate) fn walk(img: &Image, dist0: &[i64], count: usize, lengths: &[i64], depth: Depth, table: &[[i64; 6]]) -> Option<Image> {
    let (n, nc) = (img.w, img.n);
    let alpha = nc - 1;
    let last = n as i64 - 1;
    let mut px = vec![0.0f32; count.checked_mul(img.h)?.checked_mul(nc)?];
    crate::par::chunks_mut(&mut px, count * nc, true, |y, row| {
        let (Some(&d0), Some(&length)) = (dist0.get(y), lengths.get(y)) else { return };
        let Some(line) = img.px.get(y * n * nc..(y + 1) * n * nc) else { return };
        let phases = line_phases(d0, count, length, n);
        let mut acc = vec![0.0f64; nc];
        for (out, ph) in row.chunks_exact_mut(nc).zip(phases) {
            let base = ph.div_euclid(128) - 2;
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

pub(crate) fn transpose(img: &Image) -> Image {
    let (w, h, n) = (img.w, img.h, img.n);
    let mut px = vec![0.0f32; w * h * n];
    // Output row x is input column x.
    crate::par::chunks_mut(&mut px, h * n, true, |x, row| {
        for (y, out) in row.chunks_exact_mut(n).enumerate() {
            if let Some(src) = img.px.get((y * w + x) * n..(y * w + x + 1) * n) {
                out.copy_from_slice(src);
            }
        }
    });
    Image { w: h, h: w, n, px }
}

/// A quarter turn clockwise (row `i` of the result is column `i` read bottom to top).
fn turn(img: &Image) -> Image {
    let (w, h, n) = (img.w, img.h, img.n);
    let mut px = vec![0.0f32; w * h * n];
    crate::par::chunks_mut(&mut px, h * n, true, |i, row| {
        for (j, out) in row.chunks_exact_mut(n).enumerate() {
            let y = h - 1 - j;
            if let Some(src) = img.px.get((y * w + i) * n..(y * w + i + 1) * n) {
                out.copy_from_slice(src);
            }
        }
    });
    Image { w: h, h: w, n, px }
}

/// The pixels of `src` inside `bounds` (its contents' canvas; `src` has alpha) drawn through the
/// affine `m = [a, b, c, d, e, f]` (contents → document: `x' = a x + c y + e`,
/// `y' = b x + d y + f`, with `x, y` measured from `bounds`' corner). `None` for a mirror flip, a
/// degenerate map, or a size past [`MAX_SAMPLES`].
pub fn render(src: &Surface, bounds: Rect, m: [f64; 6]) -> Option<Surface> {
    render_with(src, bounds, m, true, true)
}

/// [`render`], bleeding the contents or (for a perspective cascade's intermediate, which keeps
/// its computed colours under transparency) not.
/// `clear`: a final result, whose fully transparent pixels lose their colour ([`crate::smart_scale::clear_hidden`]);
/// not a cascade's intermediate, whose colours its next stage reads.
pub(crate) fn render_with(src: &Surface, bounds: Rect, m: [f64; 6], bleed_contents: bool, clear: bool) -> Option<Surface> {
    let fmt = src.format();
    let nc = src.channels();
    if !fmt.alpha || nc == 0 || bounds.is_empty() || m.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let [a0, b0, c0, d0, e0, f0] = m;
    if a0 * d0 - b0 * c0 <= 1e-12 {
        return None;
    }
    let depth = Depth::of(fmt.sample);
    let unit = depth.levels.unwrap_or(1.0);
    let (w, h) = (bounds.width() as usize, bounds.height() as usize);
    let mut px = src.read_region(bounds);
    if depth.levels.is_some() {
        crate::par::chunks_mut(&mut px, 1 << 16, true, |_, part| {
            for v in part {
                *v = (v.clamp(0.0, 1.0) * unit).round();
            }
        });
    }
    let mut img = Image { w, h, n: nc, px };
    if bleed_contents {
        bleed(&mut img);
    }
    // Rule 2: quarter turns, and the map from the turned image.
    let ang = b0.atan2(a0).to_degrees();
    let turns = ((ang + 45.0) / 90.0).floor().rem_euclid(4.0) as usize;
    let (mut l, mut t) = ([[a0, c0], [b0, d0]], [e0, f0]);
    for _ in 0..turns {
        img = turn(&img);
        // p' = R p + (H_old, 0) with R = [[0, -1], [1, 0]]: the map from the turned image is
        // L R⁻¹ (p' − (H_old, 0)), R⁻¹ = [[0, 1], [-1, 0]].
        let ri = [[0.0, 1.0], [-1.0, 0.0]];
        let lr = [
            [l[0][0] * ri[0][0] + l[0][1] * ri[1][0], l[0][0] * ri[0][1] + l[0][1] * ri[1][1]],
            [l[1][0] * ri[0][0] + l[1][1] * ri[1][0], l[1][0] * ri[0][1] + l[1][1] * ri[1][1]],
        ];
        let hold = img.w as f64; // the old height is the turned width
        t = [t[0] - lr[0][0] * hold, t[1] - lr[1][0] * hold];
        l = lr;
    }
    let (a, c, b, d, e, f) = (l[0][0], l[0][1], l[1][0], l[1][1], t[0], t[1]);
    if a <= 0.0 {
        return None;
    }
    let k = d - b * c / a;
    if k <= 0.0 {
        return None;
    }
    let (w, h) = (img.w, img.h);
    let (wf, hf) = (w as f64, h as f64);
    let xs = [e, a * wf + e, c * hf + e, a * wf + c * hf + e];
    let x0 = round_half_up(xs.iter().copied().fold(f64::INFINITY, f64::min));
    let x1 = round_half_up(xs.iter().copied().fold(f64::NEG_INFINITY, f64::max));
    if x1 <= x0 {
        return None;
    }
    let nx = (x1 - x0) as usize;
    let top = |x: i64| f + b * (x as f64 + 0.5 - e) / a;
    let (t0, t1) = (top(x0), top(x1 - 1));
    let y0 = round_half_up(t0.min(t1));
    let y1 = round_half_up(t0.max(t1) + hf * k);
    if y1 <= y0 {
        return None;
    }
    let ny = (y1 - y0) as usize;
    let biggest = nx.max(w).checked_mul(ny.max(h))?.checked_mul(nc)?;
    if biggest > MAX_SAMPLES {
        return None;
    }
    let table = weight_table();
    // Pass 1: each source row onto the document columns x0..x1.
    let dist1: Vec<i64> = (0..h).map(|j| x0 * 256 - r256(e + c * (j as f64 + 0.5))).collect();
    let inter = pass(&img, &dist1, nx, r256(wf * a), depth, &table)?;
    // Pass 2: each document column onto the rows y0..y1.
    let cols = transpose(&inter);
    let dist2: Vec<i64> = (0..nx).map(|i| y0 * 256 - r256(top(x0 + i as i64))).collect();
    let out = transpose(&pass(&cols, &dist2, ny, r256(hf * k), depth, &table)?);
    let mut surface = Surface::new(fmt);
    let mut data = out.px;
    crate::par::chunks_mut(&mut data, 1 << 16, true, |_, part| {
        for v in part {
            *v /= unit;
        }
    });
    if clear {
        crate::smart_scale::clear_hidden(&mut data, &fmt);
    }
    surface.write_region_par(Rect::new(x0 as i32, y0 as i32, x1 as i32, y1 as i32), &data);
    Some(surface)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_that_keeps_its_scale_moves_by_whole_phases() {
        // Shift by 0.25 px (64/256): phase 32, then 128 per pixel.
        assert_eq!(line_phases(64, 3, 128 * 256, 128), vec![32, 160, 288]);
    }

    #[test]
    fn a_scaling_line_lands_just_below_exact_boundaries_once_it_has_walked() {
        // 128 samples over 96 px: 4/3 sample per px, not a binary fraction. Starting exactly on a
        // phase, the first pixel keeps it (the start is rounded up); later exact ones fall below.
        let ph = line_phases(0, 4, 96 * 256, 128);
        assert_eq!(ph[0], 0);
        assert_eq!(ph[3], 512 - 1, "three steps of 4/3 = 4 samples exactly, truncated just below");
    }

    #[test]
    fn weights_sum_to_one() {
        for q in weight_table() {
            assert_eq!(q.iter().sum::<i64>(), ONE);
        }
    }

    #[test]
    fn strong_shrinks_average_the_covered_samples() {
        // 128 samples to 91 nodes (50 % / 0.7): ((r + 1) 91 − 1) div 128 groups 1 or 2 samples.
        let img = Image { w: 128, h: 1, n: 1, px: (0..128).map(|i| i as f32).collect() };
        let r = reduce(&img, 91, Depth { levels: Some(255.0) });
        assert_eq!(r.w, 91);
        // Sample 0 goes to node 0 ((1·91 − 1) div 128 = 0), sample 1 to node 1 (181 div 128).
        assert_eq!(r.at(0, 0, 0), 0.0);
        assert_eq!(r.at(1, 0, 0), 1.0);
        assert!(r.px.iter().all(|v| v.fract() == 0.0), "rounded to whole levels");
    }

    /// A rotated render leaves no colour under alpha 0: the bled colour of the transparent
    /// surround is cleared, so the layer's content ends where it is visible.
    #[test]
    fn a_rotation_leaves_no_colour_in_transparent_pixels() {
        use photocraft_color::{ColorMode, PixelFormat, SampleType};
        let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
        s.fill_rect(Rect::new(0, 0, 40, 30), &[1.0, 0.0, 0.0, 1.0]);
        let (sin, cos) = 30f64.to_radians().sin_cos();
        let out = render(&s, Rect::new(0, 0, 40, 30), [cos, sin, -sin, cos, 20.0, 5.0]).expect("rendered");
        let b = out.content_bounds();
        let px = out.read_region(b);
        let hidden = px.chunks(4).filter(|p| p[3] == 0.0 && p[..3].iter().any(|v| *v != 0.0)).count();
        assert_eq!(hidden, 0, "colour under alpha 0");
        assert!(px.chunks(4).any(|p| p[3] > 0.99), "the picture is drawn");
    }

    /// The probe source: 128 x 128, each row one grey from a 32-bit LCG (seed 777), opaque.
    fn bands() -> Surface {
        use photocraft_color::{ColorMode, PixelFormat, SampleType};
        let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
        let mut seed: u32 = 777;
        let mut data = Vec::with_capacity(128 * 128 * 4);
        for _ in 0..128 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let v = (seed >> 24) as f32 / 255.0;
            for _ in 0..128 {
                data.extend_from_slice(&[v, v, v, 1.0]);
            }
        }
        s.write_region(Rect::new(0, 0, 128, 128), &data);
        s
    }

    fn grey(s: &Surface, x: i32, y: i32) -> u8 {
        (s.pixel(x, y)[0] * 255.0).round() as u8
    }

    /// Byte for byte the reference application's render of the bands rotated 30° about the
    /// document centre (160, 160), source at (96, 96): document row 160, columns 112..136.
    #[test]
    fn a_rotated_smart_object_matches_the_reference_byte_for_byte() {
        let (sn, cs) = 30f64.to_radians().sin_cos();
        let (cx, cy) = (160.0, 160.0);
        let e = cx - (cs * cx - sn * cy) + (cs * 96.0 - sn * 96.0);
        let f = cy - (sn * cx + cs * cy) + (sn * 96.0 + cs * 96.0);
        let out = render(&bands(), Rect::new(0, 0, 128, 128), [cs, sn, -sn, cs, e, f]).unwrap();
        let want = [173, 252, 204, 54, 47, 188, 255, 255, 178, 73, 6, 14, 92, 206, 191, 51, 0, 50, 55, 8, 56, 168, 199, 131];
        let got: Vec<u8> = (112..136).map(|x| grey(&out, x, 160)).collect();
        assert_eq!(got, want);
    }

    /// The bands skewed 10° vertically and shrunk to 50 % (below the strong-shrink limit), with
    /// the reference application's stored corners: document column 160, rows 140..164.
    #[test]
    fn a_strongly_shrunk_skewed_smart_object_matches_the_reference_byte_for_byte() {
        let (top_l, top_r, bot_l) = (116.715073234658, 139.284926765342, 180.715073234658);
        let m = [1.0, (top_r - top_l) / 128.0, 0.0, (bot_l - top_l) / 128.0, 96.0, top_l];
        let out = render(&bands(), Rect::new(0, 0, 128, 128), m).unwrap();
        let want = [147, 147, 176, 200, 214, 164, 94, 112, 98, 99, 81, 129, 212, 141, 18, 206, 104, 79, 222, 232, 65, 229, 222, 18];
        let got: Vec<u8> = (140..164).map(|y| grey(&out, 160, y)).collect();
        assert_eq!(got, want);
    }
}
