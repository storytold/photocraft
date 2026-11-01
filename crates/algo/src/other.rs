//! Minimum, Maximum, Offset.

use photocraft_geom::Rect;

use crate::image::{Edge, Image};
use crate::{Ctx, Preserve, UndefinedAreas};

/// Minimum or Maximum with the dialog's Preserve option: a square (`min_max`) or a disc
/// (`min_max_round`).
pub(crate) fn min_max_preserve(src: &Image, out: Rect, radius: f32, max: bool, preserve: Preserve) -> Vec<f32> {
    match preserve {
        Preserve::Squareness => min_max(src, out, radius, max),
        Preserve::Roundness => min_max_round(src, out, radius, max),
    }
}

/// Minimum or Maximum over a disc (Preserve: Roundness): every offset with
/// `dx² + dy² ≤ radius²`, so corners and straight edges move by the same distance. Each row of
/// the disc is a horizontal window, taken with the van Herk / Gil-Werman running extreme
/// (constant work per pixel whatever the window), so the cost is O(radius) per pixel.
///
/// Disc rows of the same half-width (the two halves of the disc, and runs of rows near its
/// middle) need the same horizontal extremes of a source row, so each source row's extreme is
/// worked out once per half-width and kept while a later disc row still needs it. The rows are
/// then combined per pixel in the same order as one pass per disc row (top to bottom, the running
/// result first), so the output is identical, signed zeros and NaNs included.
pub(crate) fn min_max_round(src: &Image, out: Rect, radius: f32, max: bool) -> Vec<f32> {
    let n = src.ch;
    let r = radius.max(0.0);
    let ri = r.floor() as i32;
    if ri == 0 {
        return src.crop(out);
    }
    let (ow, oh) = (out.width() as usize, out.height() as usize);
    let pick = |a: f32, b: f32| if max { a.max(b) } else { a.min(b) };
    let sentinel = if max { f32::MIN } else { f32::MAX };
    // Half-width of the disc on each row (the epsilon keeps exact squares, e.g. r 5, dy 3 → 4).
    let hws: Vec<usize> = (-ri..=ri).map(|dy| ((r * r - (dy * dy) as f32).max(0.0).sqrt() + 1e-4).floor() as usize).collect();
    let (ru, rows) = (ri as usize, oh + 2 * ri as usize);
    // The last disc row (index into `hws`) that uses each half-width.
    let mut last = vec![0usize; ru + 1];
    for (j, &hw) in hws.iter().enumerate() {
        last[hw] = j;
    }
    // The source rows a half-width's extremes are needed for: window rows `j..j + oh` for each of
    // its disc rows `j`.
    let mut span = vec![(usize::MAX, 0usize); ru + 1];
    for (j, &hw) in hws.iter().enumerate() {
        span[hw] = (span[hw].0.min(j), span[hw].1.max(j + oh));
    }
    let pw = ow + 2 * ru;
    // The window's samples of one channel, `rows × pw` from (out.x0 - r, out.y0 - r); outside the
    // source they read as 0, as `Image::get` does.
    let win = Rect::new(out.x0 - ri, out.y0 - ri, out.x1 + ri, out.y1 + ri);
    let inside = win.intersect(&src.rect);
    let mut plane = vec![0.0f32; rows * pw];
    let mut acc = vec![sentinel; ow * oh];
    let mut cache: Vec<Option<Vec<f32>>> = vec![None; ru + 1];
    let mut scratch = Extremes::default();
    let mut res = vec![0.0f32; ow * oh * n];
    for c in 0..n {
        plane.fill(0.0);
        if !inside.is_empty() {
            for y in inside.y0..inside.y1 {
                let o = (y - win.y0) as usize * pw + (inside.x0 - win.x0) as usize;
                let row = src.row(Rect::new(inside.x0, y, inside.x1, y + 1));
                for (d, px) in plane[o..o + inside.width() as usize].iter_mut().zip(row.chunks_exact(n)) {
                    *d = px[c];
                }
            }
        }
        acc.fill(sentinel);
        for (j, &hw) in hws.iter().enumerate() {
            // This half-width's extremes for the source rows it needs (window rows `y0..y1`).
            let (y0, y1) = span[hw];
            let ext = cache[hw].get_or_insert_with(|| {
                let mut e = vec![0.0f32; (y1 - y0) * ow];
                for (y, dst) in (y0..y1).zip(e.chunks_exact_mut(ow)) {
                    let line = &plane[y * pw + ru - hw..y * pw + ru + ow + hw];
                    running_extreme_with(line, 2 * hw + 1, max, &mut scratch);
                    dst.copy_from_slice(&scratch.out);
                }
                e
            });
            for (a, e) in acc.chunks_exact_mut(ow).zip(ext[(j - y0) * ow..].chunks_exact(ow)) {
                for (a, &v) in a.iter_mut().zip(e) {
                    *a = pick(*a, v);
                }
            }
            if last[hw] == j {
                cache[hw] = None;
            }
        }
        for (o, &v) in res.iter_mut().skip(c).step_by(n).zip(&acc) {
            *o = v;
        }
    }
    res
}

/// Scratch buffers of [`running_extreme_with`].
#[derive(Default)]
struct Extremes {
    pre: Vec<f32>,
    suf: Vec<f32>,
    out: Vec<f32>,
}

/// [`running_extreme`] into `s.out`, reusing `s`'s buffers (the same values).
fn running_extreme_with(v: &[f32], k: usize, max: bool, s: &mut Extremes) {
    let Extremes { pre, suf, out } = s;
    out.clear();
    let n = v.len();
    if k <= 1 {
        out.extend_from_slice(v);
        return;
    }
    if n < k {
        return;
    }
    let pick = |a: f32, b: f32| if max { a.max(b) } else { a.min(b) };
    pre.clear();
    pre.extend_from_slice(v);
    suf.clear();
    suf.extend_from_slice(v);
    // Within each block of `k` (no division per element): prefix extremes forwards, suffix
    // extremes backwards, the same picks in the same order as testing `i % k` per element.
    for (p, b) in pre.chunks_mut(k).zip(v.chunks(k)) {
        for i in 1..p.len() {
            p[i] = pick(p[i - 1], b[i]);
        }
    }
    for (q, b) in suf.chunks_mut(k).zip(v.chunks(k)) {
        for i in (0..q.len().saturating_sub(1)).rev() {
            q[i] = pick(q[i + 1], b[i]);
        }
    }
    out.extend((0..=n - k).map(|i| pick(suf[i], pre[i + k - 1])));
}

/// `out[i]` = extreme of `v[i..i + k]` for every full window (van Herk / Gil-Werman: prefix
/// extremes within blocks of `k` plus suffix extremes within blocks, one lookup of each).
pub(crate) fn running_extreme(v: &[f32], k: usize, max: bool, out: &mut Vec<f32>) {
    out.clear();
    let n = v.len();
    if k <= 1 {
        out.extend_from_slice(v);
        return;
    }
    if n < k {
        return;
    }
    let pick = |a: f32, b: f32| if max { a.max(b) } else { a.min(b) };
    let mut pre = v.to_vec();
    let mut suf = v.to_vec();
    // Within each block of `k` (no division per element): prefix extremes forwards, suffix
    // extremes backwards, the same picks in the same order as testing `i % k` per element.
    for (p, b) in pre.chunks_mut(k).zip(v.chunks(k)) {
        for i in 1..p.len() {
            p[i] = pick(p[i - 1], b[i]);
        }
    }
    for (q, b) in suf.chunks_mut(k).zip(v.chunks(k)) {
        for i in (0..q.len().saturating_sub(1)).rev() {
            q[i] = pick(q[i + 1], b[i]);
        }
    }
    out.extend((0..=n - k).map(|i| pick(suf[i], pre[i + k - 1])));
}

/// Minimum (spreads dark/transparent areas) or Maximum over a square. Separable (rows, then
/// columns), each pass a [`running_extreme`], so the cost per pixel does not grow with the radius.
pub(crate) fn min_max(src: &Image, out: Rect, radius: f32, max: bool) -> Vec<f32> {
    let n = src.ch;
    let r = radius.max(0.0).round() as i32;
    if r == 0 {
        return src.crop(out);
    }
    let k = (2 * r + 1) as usize;
    // Rows: the extreme over x - r..=x + r, for every row the column pass needs.
    let win = Rect::new(out.x0, out.y0 - r, out.x1, out.y1 + r);
    let (ww, wh) = (win.width() as usize, win.height() as usize);
    let mut tmp = vec![0.0f32; ww * wh * n];
    let (mut line, mut ext) = (Vec::new(), Vec::new());
    for y in win.y0..win.y1 {
        let o = (y - win.y0) as usize * ww * n;
        for c in 0..n {
            line.clear();
            line.extend((win.x0 - r..win.x1 + r).map(|x| src.get(x, y, c)));
            running_extreme(&line, k, max, &mut ext);
            for (x, v) in ext.iter().enumerate() {
                tmp[o + x * n + c] = *v;
            }
        }
    }
    // Columns: the extreme over y - r..=y + r of the row results.
    let (ow, oh) = (out.width() as usize, out.height() as usize);
    let mut res = vec![0.0f32; ow * oh * n];
    for x in 0..ow {
        for c in 0..n {
            line.clear();
            line.extend((0..wh).map(|y| tmp[(y * ww + x) * n + c]));
            running_extreme(&line, k, max, &mut ext);
            for (y, v) in ext.iter().enumerate() {
                res[(y * ow + x) * n + c] = *v;
            }
        }
    }
    res
}

pub(crate) fn edge_of(u: UndefinedAreas) -> Edge {
    match u {
        UndefinedAreas::Wrap => Edge::Wrap,
        UndefinedAreas::Repeat => Edge::Repeat,
        UndefinedAreas::Transparent => Edge::Transparent,
    }
}

/// Shifts the bounds' content by whole pixels.
pub(crate) fn offset(src: &Image, out: Rect, ctx: &Ctx, dx: i32, dy: i32, undefined: UndefinedAreas) -> Vec<f32> {
    let n = src.ch;
    // Subtraction can leave i32 even for valid command parameters. Apply the edge
    // policy before narrowing: saturating first would change the wrapped pixel.
    let coordinate = |position: i32, distance: i32, start: i32, end: i32| -> Option<i32> {
        if start >= end {
            return None;
        }
        let p = i64::from(position) - i64::from(distance);
        let (start, end) = (i64::from(start), i64::from(end));
        let mapped = match undefined {
            UndefinedAreas::Wrap => start + (p - start).rem_euclid(end - start),
            UndefinedAreas::Repeat => p.clamp(start, end - 1),
            UndefinedAreas::Transparent if p >= start && p < end => p,
            UndefinedAreas::Transparent => return None,
        };
        i32::try_from(mapped).ok()
    };
    let mut res = Vec::with_capacity(out.width() as usize * out.height() as usize * n);
    for y in out.y0..out.y1 {
        let sy = coordinate(y, dy, ctx.bounds.y0, ctx.bounds.y1);
        for x in out.x0..out.x1 {
            let sx = coordinate(x, dx, ctx.bounds.x0, ctx.bounds.x1);
            for c in 0..n {
                res.push(match (sx, sy) {
                    (Some(sx), Some(sy)) => src.get(sx, sy, c),
                    _ => 0.0,
                });
            }
        }
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The square window scanned directly, the reference for [`min_max`].
    fn min_max_direct(src: &Image, out: Rect, r: i32, max: bool) -> Vec<f32> {
        let mut res = Vec::new();
        for y in out.y0..out.y1 {
            for x in out.x0..out.x1 {
                for c in 0..src.ch {
                    let mut m = if max { f32::MIN } else { f32::MAX };
                    for yy in y - r..=y + r {
                        for xx in x - r..=x + r {
                            let v = src.get(xx, yy, c);
                            m = if max { m.max(v) } else { m.min(v) };
                        }
                    }
                    res.push(m);
                }
            }
        }
        res
    }

    /// The previous disc implementation (one pass per disc row), the reference for the shared
    /// half-width extremes of [`min_max_round`].
    fn min_max_round_reference(src: &Image, out: Rect, radius: f32, max: bool) -> Vec<f32> {
        let n = src.ch;
        let r = radius.max(0.0);
        let ri = r.floor() as i32;
        if ri == 0 {
            return src.crop(out);
        }
        let w = out.width() as usize;
        let pick = |a: f32, b: f32| if max { a.max(b) } else { a.min(b) };
        let mut res = vec![if max { f32::MIN } else { f32::MAX }; w * out.height() as usize * n];
        let (mut row, mut ext) = (Vec::new(), Vec::new());
        for dy in -ri..=ri {
            // Half-width of the disc on this row (the epsilon keeps exact squares, e.g. r 5, dy 3 → 4).
            let hw = ((r * r - (dy * dy) as f32).max(0.0).sqrt() + 1e-4).floor() as i32;
            for y in out.y0..out.y1 {
                let o = (y - out.y0) as usize * w * n;
                for c in 0..n {
                    row.clear();
                    row.extend((out.x0 - hw..out.x1 + hw).map(|x| src.get(x, y + dy, c)));
                    running_extreme(&row, (2 * hw + 1) as usize, max, &mut ext);
                    for (x, v) in ext.iter().enumerate() {
                        let i = o + x * n + c;
                        res[i] = pick(res[i], *v);
                    }
                }
            }
        }
        res
    }

    /// The disc's shared extremes give exactly the previous result: integer and fractional radii,
    /// windows past the source, outputs wider than the source, ties, signed zeros, values outside
    /// 0..1 and NaNs, minimum and maximum.
    #[test]
    fn round_min_max_matches_one_pass_per_disc_row() {
        let src_rect = Rect::new(-12, -9, 37, 30);
        let mut img = Image::new(src_rect, 3);
        let mut s = 0x9e37_79b9u32;
        for (i, v) in img.data.iter_mut().enumerate() {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            *v = match i % 5 {
                0 => [0.0, -0.0, 1.5, -2.0, f32::NAN][(s % 5) as usize],
                1 => (s % 4) as f32 / 3.0,
                _ => (s % 1000) as f32 / 999.0,
            };
        }
        for out in [Rect::new(0, 0, 25, 21), Rect::new(-14, -3, 3, 40), Rect::new(-30, -20, 50, 45)] {
            for r in [0.5, 1.0, 1.6, 2.0, 3.0, 5.0, 7.3, 12.0, 25.0] {
                for max in [false, true] {
                    let bits = |v: Vec<f32>| v.into_iter().map(f32::to_bits).collect::<Vec<_>>();
                    assert_eq!(bits(min_max_round(&img, out, r, max)), bits(min_max_round_reference(&img, out, r, max)), "r {r} max {max} out {out:?}");
                }
            }
        }
    }

    #[test]
    fn square_min_max_matches_the_direct_window_exactly() {
        let src_rect = Rect::new(-12, -9, 37, 30);
        let mut img = Image::new(src_rect, 4);
        let mut s = 0x2545_f491u32;
        for v in img.data.iter_mut() {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            *v = (s % 1000) as f32 / 999.0;
        }
        // Includes windows wider than the output and output reaching past the source.
        for out in [Rect::new(0, 0, 25, 21), Rect::new(-14, -3, 3, 40)] {
            for r in [1, 2, 3, 7, 20] {
                for max in [false, true] {
                    let fast = min_max(&img, out, r as f32, max);
                    assert_eq!(fast, min_max_direct(&img, out, r, max), "r {r} max {max} out {out:?}");
                }
            }
        }
    }
}

#[cfg(test)]
mod offset_overflow_tests {
    use super::*;

    #[test]
    fn offset_limits_match_equivalent_small_offsets_for_every_edge_policy() {
        for bounds in [Rect::new(0, 0, 3, 5), Rect::new(-7, 11, -4, 16)] {
            let mut src = Image::new(bounds, 4);
            // Include signed/HDR values and varying alpha; offset must only move samples.
            for (i, sample) in src.data.iter_mut().enumerate() {
                *sample = i as f32 * 0.125 - 2.0;
            }
            let ctx = Ctx { bounds, mode: photocraft_color::ColorMode::Rgb, alpha: true };
            for policy in [UndefinedAreas::Wrap, UndefinedAreas::Repeat, UndefinedAreas::Transparent] {
                for dx in [i32::MIN, i32::MAX, -8, 0, 7] {
                    for dy in [i32::MIN, i32::MAX, -9, 0, 6] {
                        let reduce = |d: i32, size: i32| match policy {
                            UndefinedAreas::Wrap => d.rem_euclid(size),
                            _ => d.clamp(-100, 100),
                        };
                        let (rx, ry) = (reduce(dx, 3), reduce(dy, 5));
                        let mut expected = Vec::new();
                        for y in bounds.y0..bounds.y1 {
                            for x in bounds.x0..bounds.x1 {
                                for c in 0..4 {
                                    expected.push(src.get_edge(x - rx, y - ry, c, edge_of(policy), bounds));
                                }
                            }
                        }
                        assert_eq!(offset(&src, bounds, &ctx, dx, dy, policy), expected, "{bounds:?} {policy:?} {dx} {dy}");
                    }
                }
            }
        }
    }

    #[test]
    fn offset_wrap_handles_bounds_at_signed_coordinate_limits() {
        for start in [i32::MIN, i32::MAX - 3] {
            let bounds = Rect::new(start, start, start + 3, start + 3);
            let mut src = Image::new(bounds, 1);
            src.data = (0..9).map(|i| i as f32).collect();
            let ctx = Ctx { bounds, mode: photocraft_color::ColorMode::Grayscale, alpha: false };
            assert_eq!(offset(&src, bounds, &ctx, i32::MIN, i32::MIN, UndefinedAreas::Wrap), vec![8.0, 6.0, 7.0, 2.0, 0.0, 1.0, 5.0, 3.0, 4.0]);
        }
    }

    #[test]
    fn offset_wrap_accepts_minimum_signed_distance() {
        let bounds = Rect::new(0, 0, 3, 1);
        let mut src = Image::new(bounds, 1);
        src.data = vec![0.1, 0.5, 0.9];
        let ctx = Ctx { bounds, mode: photocraft_color::ColorMode::Grayscale, alpha: false };
        let got = offset(&src, bounds, &ctx, i32::MIN, 0, UndefinedAreas::Wrap);
        assert_eq!(got, vec![0.9, 0.1, 0.5]);
    }
}
