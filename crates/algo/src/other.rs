//! Minimum, Maximum, Offset.

use photocraft_geom::Rect;

use crate::image::{Edge, Image};
use crate::{Ctx, Preserve, UndefinedAreas};

/// The largest radius Minimum / Maximum runs at (the dialog's limit), so params that bypass the
/// engine's clamp cannot stall a tile.
const MAX_RADIUS: f32 = 500.0;

/// Minimum or Maximum with the dialog's Preserve option: a square (`min_max`) or a disc
/// (`min_max_round`). A fractional radius blends from the exact result at the radius toward the
/// one with the next larger structuring element ([`square_step`], [`disc_step`], [`blend`]), so
/// the result changes continuously with the radius and every integer radius stays exact.
pub(crate) fn min_max_preserve(src: &Image, out: Rect, radius: f32, max: bool, preserve: Preserve, alpha: bool) -> Vec<f32> {
    let r = if radius.is_finite() { radius.clamp(0.0, MAX_RADIUS) } else { 0.0 };
    let n = src.ch.max(1);
    match preserve {
        Preserve::Squareness => {
            let (k, f) = square_step(r);
            if f <= 0.0 {
                return min_max(src, out, k as f32, max);
            }
            // The next square is this one grown by a 3×3 square, so one more small pass gives it.
            let grown_rect = out.inflate(1);
            let grown = min_max(src, grown_rect, k as f32, max);
            let (gw, gh) = (grown_rect.width() as usize, grown_rect.height() as usize);
            let mut lo = Vec::with_capacity(grown.len());
            for row in grown.chunks_exact(gw.max(1) * n).skip(1).take(gh.saturating_sub(2)) {
                lo.extend_from_slice(row.get(n..row.len().saturating_sub(n)).unwrap_or_default());
            }
            let hi = extreme_3x3(&grown, gw, gh, n, max);
            blend_results(&mut lo, &hi, f, n, alpha);
            lo
        }
        Preserve::Roundness => {
            let mut lo = min_max_round(src, out, r, max);
            let step = disc_step(r);
            if step.f <= 0.0 {
                return lo;
            }
            let mut hi = lo.clone();
            fold_shell(src, out, &step.shell, max, &mut hi);
            blend_results(&mut lo, &hi, step.f, n, alpha);
            lo
        }
    }
}

/// Squareness at `radius`: the square of half-width `k`, and how far (`f` in `[0, 1)`) the
/// result moves toward the square of half-width `k + 1`.
pub(crate) fn square_step(radius: f32) -> (u32, f32) {
    let r = if radius.is_finite() { radius.clamp(0.0, MAX_RADIUS) } else { 0.0 };
    let k = r.floor();
    (k as u32, r - k)
}

/// Roundness at a radius: the hard disc `dx² + dy² ≤ n_lo` the radius selects, the next larger
/// disc `dx² + dy² ≤ n_hi`, the offsets that disc adds (`shell`: every point with
/// `dx² + dy² = n_hi`), and how far (`f`, linear in the radius from `√n_lo` to `√n_hi`) the
/// result moves toward it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DiscStep {
    pub n_lo: u64,
    pub n_hi: u64,
    pub shell: Vec<(i32, i32)>,
    pub f: f32,
}

pub(crate) fn disc_step(radius: f32) -> DiscStep {
    let r = if radius.is_finite() { radius.clamp(0.0, MAX_RADIUS) } else { 0.0 };
    let r2 = f64::from(r) * f64::from(r);
    let ri = r.floor() as u64;
    // Each row's next point out, and the centre of the first row the disc doesn't reach.
    let (mut n_lo, mut n_hi) = (0, (ri + 1) * (ri + 1));
    for dy in 0..=ri {
        let h = half_width(r2, dy);
        n_lo = n_lo.max(dy * dy + h * h);
        n_hi = n_hi.min(dy * dy + (h + 1) * (h + 1));
    }
    let mut shell = Vec::new();
    for dy in -(ri as i64 + 1)..=ri as i64 + 1 {
        let Some(rest) = n_hi.checked_sub(dy.unsigned_abs().pow(2)) else { continue };
        let h = isqrt(rest);
        if h * h == rest {
            shell.push((h as i32, dy as i32));
            if h > 0 {
                shell.push((-(h as i32), dy as i32));
            }
        }
    }
    let (lo, hi) = ((n_lo as f64).sqrt(), (n_hi as f64).sqrt());
    let f = if hi > lo { (f64::from(r) - lo) / (hi - lo) } else { 0.0 };
    let f = if f.is_nan() { 0.0 } else { f.clamp(0.0, 1.0) as f32 };
    DiscStep { n_lo, n_hi, shell, f }
}

/// The largest `h` with `h² + dy² ≤ r2`. `r2` is a squared f32, exact in f64, so the test is exact.
fn half_width(r2: f64, dy: u64) -> u64 {
    let rest = r2 - (dy * dy) as f64;
    if rest.is_nan() || rest < 0.0 {
        return 0;
    }
    let mut h = rest.sqrt() as u64;
    while ((h + 1) * (h + 1)) as f64 <= rest {
        h += 1;
    }
    while h > 0 && (h * h) as f64 > rest {
        h -= 1;
    }
    h
}

fn isqrt(v: u64) -> u64 {
    let mut h = (v as f64).sqrt() as u64;
    while (h + 1) * (h + 1) <= v {
        h += 1;
    }
    while h > 0 && h * h > v {
        h -= 1;
    }
    h
}

/// The half-widths of a disc of `radius` (Preserve: Roundness), one per row `dy` in
/// `-floor(radius) ..= floor(radius)` (empty for a radius under 1): the largest `h` with
/// `h² + dy² ≤ radius²`, tested exactly. Shared with the GPU's layer operators.
pub(crate) fn disc_half_widths(radius: f32) -> Vec<u32> {
    let r = radius.max(0.0);
    let ri = if r.is_finite() { r.floor().min(100_000.0) as i64 } else { 0 };
    if ri == 0 {
        return Vec::new();
    }
    let r2 = f64::from(r) * f64::from(r);
    (-ri..=ri).map(|dy| half_width(r2, dy.unsigned_abs()) as u32).collect()
}

/// Folds the source at each `shell` offset into `res` (the result over `out`), one shifted source
/// row at a time. Samples outside the source are 0, as in [`Image::get`].
fn fold_shell(src: &Image, out: Rect, shell: &[(i32, i32)], max: bool, res: &mut [f32]) {
    let n = src.ch;
    let stride = out.width() as usize * n;
    if stride == 0 {
        return;
    }
    let pick = |a: f32, b: f32| if max { a.max(b) } else { a.min(b) };
    for &(dx, dy) in shell {
        let (sx0, sx1) = (out.x0.saturating_add(dx), out.x1.saturating_add(dx));
        let (x0, x1) = (sx0.max(src.rect.x0), sx1.min(src.rect.x1));
        for (y, dst) in (out.y0..out.y1).zip(res.chunks_exact_mut(stride)) {
            let sy = y.saturating_add(dy);
            let (lead, inside) = if x0 < x1 && sy >= src.rect.y0 && sy < src.rect.y1 {
                ((x0 - sx0) as usize * n, src.row(Rect::new(x0, sy, x1, sy + 1)))
            } else {
                (stride, &[][..])
            };
            for (i, d) in dst.iter_mut().enumerate() {
                let v = i.checked_sub(lead).and_then(|j| inside.get(j)).copied().unwrap_or(0.0);
                *d = pick(*d, v);
            }
        }
    }
}

/// The 3×3 extreme of a `w`×`h` image of `n` channels, over its inner `(w - 2)`×`(h - 2)` pixels.
fn extreme_3x3(v: &[f32], w: usize, h: usize, n: usize, max: bool) -> Vec<f32> {
    let pick = |a: f32, b: f32| if max { a.max(b) } else { a.min(b) };
    let (ow, oh) = (w.saturating_sub(2), h.saturating_sub(2));
    let at = |x: usize, y: usize, c: usize| v.get((y * w + x) * n + c).copied().unwrap_or(0.0);
    let mut rows = Vec::with_capacity(ow * h * n);
    for y in 0..h {
        for x in 0..ow {
            for c in 0..n {
                rows.push(pick(pick(at(x, y, c), at(x + 1, y, c)), at(x + 2, y, c)));
            }
        }
    }
    let row = |x: usize, y: usize, c: usize| rows.get((y * ow + x) * n + c).copied().unwrap_or(0.0);
    let mut res = Vec::with_capacity(ow * oh * n);
    for y in 0..oh {
        for x in 0..ow {
            for c in 0..n {
                res.push(pick(pick(row(x, y, c), row(x, y + 1, c)), row(x, y + 2, c)));
            }
        }
    }
    res
}

/// Moves each sample of `lo` toward `hi` by `f`. With alpha the colour blends premultiplied, so a
/// colour hidden under zero alpha doesn't show in the half-transparent result.
fn blend_results(lo: &mut [f32], hi: &[f32], f: f32, n: usize, alpha: bool) {
    let alpha = alpha && n >= 2;
    let cc = if alpha { n - 1 } else { n };
    for (l, h) in lo.chunks_exact_mut(n.max(1)).zip(hi.chunks_exact(n.max(1))) {
        let (a_lo, a_hi) = (l.get(cc).copied().unwrap_or(1.0), h.get(cc).copied().unwrap_or(1.0));
        // The premultiplied blend is a straight blend with the weight shifted toward the more
        // opaque end.
        let (wl, wh) = (a_lo * (1.0 - f), a_hi * f);
        let t = if alpha && wl >= 0.0 && wh >= 0.0 && (wl + wh) > 0.0 && (wl + wh).is_finite() { wh / (wl + wh) } else { f };
        for (a, b) in l.iter_mut().zip(h).take(cc) {
            *a = blend(*a, *b, t);
        }
        if let (true, Some(a)) = (alpha, l.get_mut(cc)) {
            *a = blend(a_lo, a_hi, f);
        }
    }
}

/// `lo` moved toward `hi` by `f` in `[0, 1]`, never past either. An infinite end (HDR) snaps over
/// at `f = 0.5` rather than giving NaN.
pub(crate) fn blend(lo: f32, hi: f32, f: f32) -> f32 {
    if lo == hi {
        return lo;
    }
    if !lo.is_finite() || !hi.is_finite() {
        return if f >= 0.5 { hi } else { lo };
    }
    let d = hi - lo;
    let v = if d.is_finite() { lo + f * d } else { lo * (1.0 - f) + hi * f };
    v.max(lo.min(hi)).min(lo.max(hi))
}

/// Minimum or Maximum over a disc (Preserve: Roundness): every offset with
/// `dx² + dy² ≤ radius²`, so corners and straight edges move by the same distance. Each row of
/// the disc is a horizontal window, taken with the van Herk / Gil-Werman running extreme
/// (constant work per pixel whatever the window), so the cost is O(radius) per pixel.
pub(crate) fn min_max_round(src: &Image, out: Rect, radius: f32, max: bool) -> Vec<f32> {
    let n = src.ch;
    let half_widths = disc_half_widths(radius);
    let ri = (half_widths.len() / 2) as i32;
    if ri == 0 {
        return src.crop(out);
    }
    let w = out.width() as usize;
    let pick = |a: f32, b: f32| if max { a.max(b) } else { a.min(b) };
    let mut res = vec![if max { f32::MIN } else { f32::MAX }; w * out.height() as usize * n];
    let (mut row, mut ext) = (Vec::new(), Vec::new());
    for (dy, hw) in (-ri..=ri).zip(half_widths) {
        let hw = hw as i32;
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
    for i in 1..n {
        if i % k != 0 {
            pre[i] = pick(pre[i - 1], v[i]);
        }
    }
    for i in (0..n - 1).rev() {
        if (i + 1) % k != 0 {
            suf[i] = pick(suf[i + 1], v[i]);
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

    fn noise(rect: Rect, ch: usize) -> Image {
        let mut img = Image::new(rect, ch);
        let mut s = 0x2545_f491u32;
        for v in img.data.iter_mut() {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            *v = (s % 1000) as f32 / 999.0;
        }
        img
    }

    /// Every offset with `dx² + dy² ≤ n` scanned directly, the reference for the disc.
    fn disc_direct(src: &Image, out: Rect, n: i64, max: bool) -> Vec<f32> {
        let r = (n as f64).sqrt() as i32 + 1;
        let mut res = Vec::new();
        for y in out.y0..out.y1 {
            for x in out.x0..out.x1 {
                for c in 0..src.ch {
                    let mut m = if max { f32::MIN } else { f32::MAX };
                    for dy in -r..=r {
                        for dx in -r..=r {
                            if i64::from(dx * dx + dy * dy) <= n {
                                let v = src.get(x + dx, y + dy, c);
                                m = if max { m.max(v) } else { m.min(v) };
                            }
                        }
                    }
                    res.push(m);
                }
            }
        }
        res
    }

    /// The sums of two squares just inside and just outside `r` (`n_lo ≤ r² < n_hi`), found by
    /// scanning the lattice.
    fn lattice_step(r: f32) -> (u64, u64) {
        let r2 = f64::from(r) * f64::from(r);
        let m = r as i64 + 2;
        let (mut lo, mut hi) = (0, u64::MAX);
        for dy in -m..=m {
            for dx in -m..=m {
                let s = (dx * dx + dy * dy) as u64;
                if s as f64 <= r2 {
                    lo = lo.max(s);
                } else {
                    hi = hi.min(s);
                }
            }
        }
        (lo, hi)
    }

    #[test]
    fn integer_radii_match_the_direct_square_and_disc_exactly() {
        let img = noise(Rect::new(-12, -9, 37, 30), 4);
        for out in [Rect::new(0, 0, 25, 21), Rect::new(-14, -3, 3, 40)] {
            for r in [1, 2, 3, 7, 20] {
                for max in [false, true] {
                    let square = min_max_preserve(&img, out, r as f32, max, Preserve::Squareness, true);
                    assert_eq!(square, min_max_direct(&img, out, r, max), "square r {r} max {max} out {out:?}");
                    let disc = min_max_preserve(&img, out, r as f32, max, Preserve::Roundness, true);
                    assert_eq!(disc, disc_direct(&img, out, i64::from(r * r), max), "disc r {r} max {max} out {out:?}");
                }
            }
        }
    }

    #[test]
    fn fractional_radii_blend_the_direct_windows() {
        let img = noise(Rect::new(-12, -9, 37, 30), 3);
        for out in [Rect::new(0, 0, 25, 21), Rect::new(-14, -3, 3, 40)] {
            for r in [0.2f32, 0.5, 1.5, 2.2, 2.9, 4.0, 7.3, 12.5] {
                for max in [false, true] {
                    let mix = |lo: Vec<f32>, hi: Vec<f32>, f: f32| lo.iter().zip(&hi).map(|(a, b)| blend(*a, *b, f)).collect::<Vec<_>>();
                    let k = r.floor() as i32;
                    let want = mix(min_max_direct(&img, out, k, max), min_max_direct(&img, out, k + 1, max), r - k as f32);
                    assert_eq!(min_max_preserve(&img, out, r, max, Preserve::Squareness, false), want, "square r {r} max {max} out {out:?}");
                    let (n_lo, n_hi) = lattice_step(r);
                    let f = ((f64::from(r) - (n_lo as f64).sqrt()) / ((n_hi as f64).sqrt() - (n_lo as f64).sqrt())) as f32;
                    let want = mix(disc_direct(&img, out, n_lo as i64, max), disc_direct(&img, out, n_hi as i64, max), f);
                    assert_eq!(min_max_preserve(&img, out, r, max, Preserve::Roundness, false), want, "disc r {r} max {max} out {out:?}");
                }
            }
        }
    }

    /// The exact disc test keeps the old `floor(sqrt(r² − dy²) + 1e-4)` half-widths at every
    /// integer radius: that epsilon only ever rescued exact squares.
    #[test]
    fn exact_disc_half_widths_keep_every_integer_radius() {
        for r in 1..=500 {
            let r = r as f32;
            let ri = r as i32;
            let old: Vec<u32> = (-ri..=ri).map(|dy| ((r * r - (dy * dy) as f32).max(0.0).sqrt() + 1e-4).floor() as u32).collect();
            assert_eq!(disc_half_widths(r), old, "r {r}");
        }
        assert!(disc_half_widths(0.9).is_empty());
        assert!(disc_half_widths(f32::NAN).is_empty());
        assert!(disc_half_widths(f32::INFINITY).is_empty());
    }

    #[test]
    fn disc_step_matches_a_lattice_enumeration() {
        let radii = (2..=300).map(|i| i as f32 * 0.1).chain([0.0, 1.0, 2.0f32.sqrt(), 5.0, 8.0f32.sqrt(), 499.5, 500.0]);
        for r in radii {
            let step = disc_step(r);
            let (n_lo, n_hi) = lattice_step(r);
            assert_eq!((step.n_lo, step.n_hi), (n_lo, n_hi), "r {r}");
            let m = r as i32 + 2;
            let mut want: Vec<(i32, i32)> =
                (-m..=m).flat_map(|dy| (-m..=m).map(move |dx| (dx, dy))).filter(|(dx, dy)| (dx * dx + dy * dy) as u64 == n_hi).collect();
            let mut shell = step.shell.clone();
            want.sort();
            shell.sort();
            assert_eq!(shell, want, "r {r}");
            let f = (f64::from(r) - (n_lo as f64).sqrt()) / ((n_hi as f64).sqrt() - (n_lo as f64).sqrt());
            assert!((0.0..1.0).contains(&step.f) && (f64::from(step.f) - f).abs() < 1e-6, "r {r}: f {} want {f}", step.f);
        }
        assert_eq!(disc_step(0.5), DiscStep { n_lo: 0, n_hi: 1, shell: vec![(0, -1), (1, 0), (-1, 0), (0, 1)], f: 0.5 });
        for r in [f32::NAN, f32::NEG_INFINITY, -1.0] {
            assert_eq!(disc_step(r).f, 0.0, "r {r}");
        }
    }

    /// Radii whose next ring has several lattice representations (25 = 5² + 0² = 4² + 3², …):
    /// every f32 just below and above the threshold moves each sample one way only.
    #[test]
    fn roundness_is_monotone_around_multi_representation_radii() {
        let img = noise(Rect::new(-20, -20, 21, 21), 1);
        let out = Rect::new(0, 0, 1, 1);
        for n in [25u32, 50, 65, 85, 125, 169, 325] {
            let centre = (n as f32).sqrt().to_bits();
            for max in [false, true] {
                let mut prev: Option<(f32, f32)> = None;
                for bits in centre - 3000..=centre + 3000 {
                    let r = f32::from_bits(bits);
                    let step = disc_step(r);
                    assert!((0.0..=1.0).contains(&step.f), "n {n} r {r}: f {}", step.f);
                    let v = min_max_preserve(&img, out, r, max, Preserve::Roundness, false)[0];
                    assert!(v.is_finite(), "n {n} r {r}");
                    if let Some((pr, pv)) = prev {
                        assert!(if max { v >= pv } else { v <= pv }, "n {n} max {max}: r {pr} gives {pv}, r {r} gives {v}");
                    }
                    prev = Some((r, v));
                }
            }
        }
    }

    #[test]
    fn blend_stays_between_its_ends_and_handles_non_finite() {
        assert_eq!(blend(f32::INFINITY, 3.0, 0.3), f32::INFINITY);
        assert_eq!(blend(f32::INFINITY, 3.0, 0.7), 3.0);
        assert_eq!(blend(f32::NEG_INFINITY, f32::INFINITY, 0.2), f32::NEG_INFINITY);
        assert_eq!(blend(f32::INFINITY, f32::INFINITY, 0.5), f32::INFINITY);
        assert_eq!(blend(-3e38, 3e38, 0.5), 0.0);
        assert_eq!(blend(-3e38, 3e38, 1.0), 3e38);
        let (lo, hi) = (0.1f32, 0.7f32);
        for i in 0..=1000 {
            let v = blend(lo, hi, i as f32 / 1000.0);
            assert!(v.is_finite() && (lo..=hi).contains(&v), "{i}: {v}");
        }
        assert_eq!(blend(lo, hi, 1.0), hi);
        assert!(blend(-0.0, 0.0, 0.5) == 0.0);
    }

    #[test]
    fn min_max_survives_non_finite_and_out_of_range_radii() {
        let img = noise(Rect::new(0, 0, 4, 3), 1);
        let out = img.rect;
        for preserve in [Preserve::Squareness, Preserve::Roundness] {
            for max in [false, true] {
                for r in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0] {
                    assert_eq!(min_max_preserve(&img, out, r, max, preserve, false), img.crop(out), "{preserve:?} r {r}");
                }
                let at_limit = min_max_preserve(&img, out, 500.0, max, preserve, false);
                for r in [600.0, 1e4] {
                    assert_eq!(min_max_preserve(&img, out, r, max, preserve, false), at_limit, "{preserve:?} r {r}");
                }
            }
        }
        // An empty output and a one-channel image with alpha requested.
        assert!(min_max_preserve(&img, Rect::new(2, 2, 2, 2), 1.5, true, Preserve::Squareness, true).is_empty());
        assert!(min_max_preserve(&img, Rect::new(2, 2, 2, 2), 1.5, true, Preserve::Roundness, true).is_empty());
        assert_eq!(min_max_preserve(&img, out, 0.5, true, Preserve::Roundness, true).len(), 12);
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
