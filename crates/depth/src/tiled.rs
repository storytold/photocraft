//! High-quality depth: the whole image is estimated once (the global pass), then again in
//! overlapping tiles that each get the network's full resolution. Every tile is fitted to the
//! global map by least squares (the network's output is only relative: scale and offset are free)
//! and the aligned tiles are blended with raised-cosine feathers. The global pass keeps the
//! large-scale ordering right; the tiles add detail the 518 px pass cannot resolve.

use crate::{DepthError, DepthMap, MODEL_SIZE, check_dims, postprocess};

/// Most tiles a call may use (the engine caps working resolution accordingly).
pub const MAX_TILES: u32 = 16;
/// Fraction of a tile that overlaps its neighbour.
const OVERLAP: f64 = 0.25;
/// Allowed range of a tile's fitted scale: outside it the tile is too flat to trust.
const SCALE_RANGE: (f32, f32) = (0.02, 50.0);
/// Below this weighted variance a tile has no usable structure to fit (scale falls back to 1).
const MIN_VARIANCE: f64 = 1e-8;

/// A pixel rectangle `[x0, x1) x [y0, y1)` of the working image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileRect {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

impl TileRect {
    pub fn width(&self) -> u32 {
        self.x1.saturating_sub(self.x0)
    }

    pub fn height(&self) -> u32 {
        self.y1.saturating_sub(self.y0)
    }
}

/// Tile count and length along one axis of `len` pixels when each tile should be about `target`
/// long and at most `max_n` tiles fit: `(n, tile_len)`.
fn axis_plan(len: u32, target: u32, max_n: u32) -> (u32, u32) {
    if len <= target || max_n <= 1 {
        return (1, len);
    }
    let stride = f64::from(target) * (1.0 - OVERLAP);
    let want = ((f64::from(len) - f64::from(target)) / stride).ceil() as u32 + 1;
    let n = want.clamp(1, max_n);
    if n == 1 {
        return (1, len);
    }
    // n tiles of length t overlapping by OVERLAP cover len: t * (1 + (n - 1) * (1 - OVERLAP)) = len.
    let t = (f64::from(len) / (1.0 + f64::from(n - 1) * (1.0 - OVERLAP))).ceil() as u32;
    (n, t.clamp(1, len))
}

/// The grid of overlapping tiles for a `w` x `h` image: about [`MODEL_SIZE`] px each, 25 % overlap,
/// at most `max_tiles` (clamped to 1..=[`MAX_TILES`]). One tile (the whole image) means no tiling.
pub fn plan_tiles(w: u32, h: u32, max_tiles: u32) -> Vec<TileRect> {
    let max_tiles = max_tiles.clamp(1, MAX_TILES);
    let (mut nx, mut tw) = axis_plan(w, MODEL_SIZE, max_tiles);
    let (mut ny, mut th) = axis_plan(h, MODEL_SIZE, max_tiles);
    // Too many: coarsen the axis with more tiles until the grid fits.
    while nx * ny > max_tiles {
        if nx >= ny {
            (nx, tw) = axis_plan(w, MODEL_SIZE, nx - 1);
        } else {
            (ny, th) = axis_plan(h, MODEL_SIZE, ny - 1);
        }
    }
    let starts = |len: u32, n: u32, t: u32| -> Vec<u32> {
        if n <= 1 {
            return vec![0];
        }
        let span = f64::from(len.saturating_sub(t));
        (0..n).map(|i| (span * f64::from(i) / f64::from(n - 1)).round() as u32).collect()
    };
    let (xs, ys) = (starts(w, nx, tw), starts(h, ny, th));
    let mut out = Vec::with_capacity(xs.len() * ys.len());
    for &y0 in &ys {
        for &x0 in &xs {
            out.push(TileRect { x0, y0, x1: (x0 + tw).min(w), y1: (y0 + th).min(h) });
        }
    }
    out
}

/// How many tiles [`plan_tiles`] uses (1 = the global pass only).
pub fn tile_count(w: u32, h: u32, max_tiles: u32) -> u32 {
    plan_tiles(w, h, max_tiles).len() as u32
}

/// Bilinear resize of one plane (pixel centres aligned).
pub fn resample_plane(src: &[f32], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<f32> {
    if (sw, sh) == (dw, dh) {
        return src.to_vec();
    }
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 || src.len() < sw * sh {
        return vec![0.0; dw.saturating_mul(dh)];
    }
    let at = |x: usize, y: usize| src.get(y * sw + x).copied().unwrap_or(0.0);
    let (fx, fy) = (sw as f32 / dw as f32, sh as f32 / dh as f32);
    let xs: Vec<(usize, usize, f32)> = (0..dw)
        .map(|x| {
            let u = ((x as f32 + 0.5) * fx - 0.5).clamp(0.0, (sw - 1) as f32);
            let x0 = u.floor() as usize;
            (x0, (x0 + 1).min(sw - 1), u - x0 as f32)
        })
        .collect();
    let mut out = Vec::with_capacity(dw * dh);
    for y in 0..dh {
        let v = ((y as f32 + 0.5) * fy - 0.5).clamp(0.0, (sh - 1) as f32);
        let y0 = v.floor() as usize;
        let y1 = (y0 + 1).min(sh - 1);
        let wy = v - y0 as f32;
        out.extend(xs.iter().map(|&(x0, x1, wx)| {
            let a = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * wx;
            let b = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * wx;
            a + (b - a) * wy
        }));
    }
    out
}

/// Raised-cosine weight along one axis for a tile `[a, b)` of an image `len` long: 1 in the
/// interior, ramping to ~0 at the tile's edges that face a neighbour, and 1 right up to the
/// image border (nothing overlaps there).
fn axis_weights(a: u32, b: u32, len: u32) -> Vec<f32> {
    let ramp = (f64::from(b - a) * OVERLAP).round().max(1.0);
    (a..b)
        .map(|i| {
            let p = f64::from(i) + 0.5;
            let left = if a > 0 { p - f64::from(a) } else { f64::INFINITY };
            let right = if b < len { f64::from(b) - p } else { f64::INFINITY };
            let d = left.min(right);
            if d >= ramp { 1.0 } else { (0.5 - 0.5 * (std::f64::consts::PI * d / ramp).cos()) as f32 }
        })
        .collect()
}

/// Weighted least squares `a * t + b ~ g` (weights `w`): `(a, b)`. A tile with no structure keeps
/// scale 1 and only shifts to the global mean; the scale is kept positive and bounded.
fn fit_affine(t: &[f32], g: &[f32], w: &[f32]) -> (f32, f32) {
    let (mut sw, mut st, mut sg, mut stt, mut stg) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for ((&t, &g), &w) in t.iter().zip(g).zip(w) {
        let (t, g, w) = (f64::from(t), f64::from(g), f64::from(w));
        sw += w;
        st += w * t;
        sg += w * g;
        stt += w * t * t;
        stg += w * t * g;
    }
    if sw <= 0.0 {
        return (1.0, 0.0);
    }
    let (mt, mg) = (st / sw, sg / sw);
    let var = stt / sw - mt * mt;
    let cov = stg / sw - mt * mg;
    let a = if var > MIN_VARIANCE { (cov / var) as f32 } else { 1.0 };
    let a = if a.is_finite() { a.clamp(SCALE_RANGE.0, SCALE_RANGE.1) } else { 1.0 };
    let b = (mg - f64::from(a) * mt) as f32;
    (a, if b.is_finite() { b } else { 0.0 })
}

/// Blend per-tile depth maps into the global map's frame. `global` is the whole working image;
/// each tile is `(rect, map)` with `map` exactly `rect`-sized. Every tile is fitted to the global
/// map by weighted least squares, blended with separable raised-cosine feathers, and pixels no
/// tile covers keep the global value. The result is min-max normalised to 0..1.
pub fn merge_tiles(global: &DepthMap, tiles: &[(TileRect, DepthMap)]) -> Result<DepthMap, DepthError> {
    let (w, h) = (global.width, global.height);
    check_dims(w, h)?;
    let (wu, hu) = (w as usize, h as usize);
    if global.data.len() != wu * hu || global.data.iter().any(|v| !v.is_finite()) {
        return Err(DepthError::Invalid("global depth map is invalid".into()));
    }
    let mut num = vec![0.0f32; wu * hu];
    let mut den = vec![0.0f32; wu * hu];
    for (r, map) in tiles {
        let (tw, th) = (r.width() as usize, r.height() as usize);
        if tw == 0
            || th == 0
            || r.x1 > w
            || r.y1 > h
            || map.width as usize != tw
            || map.height as usize != th
            || map.data.len() != tw * th
            || map.data.iter().any(|v| !v.is_finite())
        {
            return Err(DepthError::Invalid("tile depth map does not match its rectangle".into()));
        }
        let (wx, wy) = (axis_weights(r.x0, r.x1, w), axis_weights(r.y0, r.y1, h));
        let mut g = Vec::with_capacity(tw * th);
        let mut wt = Vec::with_capacity(tw * th);
        for (j, &wyv) in wy.iter().enumerate() {
            let row = (r.y0 as usize + j) * wu + r.x0 as usize;
            g.extend(global.data.get(row..row + tw).unwrap_or(&[]));
            wt.extend(wx.iter().map(|&wxv| wxv * wyv));
        }
        let (a, b) = fit_affine(&map.data, &g, &wt);
        for j in 0..th {
            let row = (r.y0 as usize + j) * wu + r.x0 as usize;
            for i in 0..tw {
                let k = j * tw + i;
                let (Some(&t), Some(&wv)) = (map.data.get(k), wt.get(k)) else { continue };
                if let (Some(n), Some(d)) = (num.get_mut(row + i), den.get_mut(row + i)) {
                    *n += wv * (a * t + b);
                    *d += wv;
                }
            }
        }
    }
    let merged: Vec<f32> = num.iter().zip(&den).zip(&global.data).map(|((&n, &d), &g)| if d > 1e-6 { n / d } else { g }).collect();
    postprocess(&merged, w, h)
}

/// An estimator's output must be a well-formed, finite map.
fn check_map(m: &DepthMap) -> Result<(), DepthError> {
    if m.width == 0 || m.height == 0 || m.data.len() != m.width as usize * m.height as usize || m.data.iter().any(|v| !v.is_finite()) {
        return Err(DepthError::Backend("the depth estimator returned an invalid map".into()));
    }
    Ok(())
}

/// Crop `rect` from interleaved RGB of width `w`.
fn crop_rgb(rgb: &[f32], w: u32, rect: &TileRect) -> Vec<f32> {
    let (wu, tw) = (w as usize, rect.width() as usize);
    let mut out = Vec::with_capacity(tw * rect.height() as usize * 3);
    for y in rect.y0..rect.y1 {
        let start = (y as usize * wu + rect.x0 as usize) * 3;
        out.extend(rgb.get(start..start + tw * 3).unwrap_or(&[]));
    }
    out
}

/// A per-image estimator: interleaved RGB, width, height to a depth map of any size.
pub type TileEstimator<'a> = dyn Fn(&[f32], u32, u32) -> Result<DepthMap, DepthError> + 'a;

/// The tiled pipeline over any per-image estimator (`estimate(rgb, w, h)` returns a map of any
/// size, larger = nearer). `rgb` is interleaved 0..1 at the working resolution; the result is at
/// that resolution. `interrupt` gets the fraction done before each estimate (one global pass plus
/// one per tile) and returns true to cancel.
pub fn estimate_tiled_with(
    estimate: &TileEstimator<'_>,
    rgb: &[f32],
    w: u32,
    h: u32,
    max_tiles: u32,
    interrupt: &dyn Fn(f32) -> bool,
) -> Result<DepthMap, DepthError> {
    check_dims(w, h)?;
    if rgb.len() != w as usize * h as usize * 3 {
        return Err(DepthError::Invalid(format!("expected {} values, got {}", w as usize * h as usize * 3, rgb.len())));
    }
    let tiles = plan_tiles(w, h, max_tiles);
    let units = (tiles.len() + 1) as f32;
    if interrupt(0.0) {
        return Err(DepthError::Cancelled);
    }
    let g = estimate(rgb, w, h)?;
    check_map(&g)?;
    let global = DepthMap { width: w, height: h, data: resample_plane(&g.data, g.width as usize, g.height as usize, w as usize, h as usize) };
    let global = postprocess(&global.data, w, h)?;
    if tiles.len() <= 1 {
        return Ok(global);
    }
    let mut done = Vec::with_capacity(tiles.len());
    for (i, r) in tiles.iter().enumerate() {
        if interrupt((i + 1) as f32 / units) {
            return Err(DepthError::Cancelled);
        }
        let (tw, th) = (r.width(), r.height());
        let m = estimate(&crop_rgb(rgb, w, r), tw, th)?;
        check_map(&m)?;
        let data = resample_plane(&m.data, m.width as usize, m.height as usize, tw as usize, th as usize);
        done.push((*r, DepthMap { width: tw, height: th, data }));
    }
    merge_tiles(&global, &done)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(w: u32, h: u32) -> DepthMap {
        DepthMap { width: w, height: h, data: (0..w * h).map(|i| (i % w) as f32 / (w - 1) as f32).collect() }
    }

    fn crop(m: &DepthMap, r: TileRect) -> DepthMap {
        let mut data = Vec::new();
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                data.push(m.data[(y * m.width + x) as usize]);
            }
        }
        DepthMap { width: r.width(), height: r.height(), data }
    }

    #[test]
    fn plans_cover_the_image_within_the_cap() {
        for (w, h, cap) in [
            (100, 80, 16),
            (518, 518, 16),
            (1600, 1200, 16),
            (2072, 1554, 16),
            (2072, 1554, 4),
            (5000, 300, 16),
            (300, 5000, 6),
            (1, 1, 16),
            (2000, 2000, 1),
            (3000, 3000, 99),
        ] {
            let t = plan_tiles(w, h, cap);
            assert!(!t.is_empty() && t.len() as u32 <= cap.clamp(1, MAX_TILES), "{w}x{h} cap {cap}: {}", t.len());
            let mut cov = vec![false; (w * h) as usize];
            for r in &t {
                assert!(r.x1 <= w && r.y1 <= h && r.width() > 0 && r.height() > 0);
                for y in r.y0..r.y1 {
                    for x in r.x0..r.x1 {
                        cov[(y * w + x) as usize] = true;
                    }
                }
            }
            assert!(cov.iter().all(|&c| c), "{w}x{h} cap {cap} not covered");
        }
        assert_eq!(tile_count(400, 300, 16), 1);
        assert!(tile_count(1600, 1200, 16) > 1);
    }

    #[test]
    fn affine_tiles_merge_back_to_the_global_map() {
        let (w, h) = (120u32, 90u32);
        let global = ramp(w, h);
        let rects = plan_tiles_for_test(w, h);
        // Each tile is a different affine transform of the global map.
        let tiles: Vec<(TileRect, DepthMap)> = rects
            .iter()
            .enumerate()
            .map(|(i, &r)| {
                let (a, b) = (0.5 + 0.4 * i as f32, 0.1 * i as f32 - 0.3);
                let mut m = crop(&global, r);
                m.data.iter_mut().for_each(|v| *v = (*v - b) / a);
                (r, m)
            })
            .collect();
        let out = merge_tiles(&global, &tiles).unwrap();
        for (o, g) in out.data.iter().zip(&global.data) {
            assert!((o - g).abs() < 1e-3, "{o} vs {g}");
        }
    }

    fn plan_tiles_for_test(w: u32, h: u32) -> Vec<TileRect> {
        // Four overlapping tiles on a 2 x 2 grid, 25 % overlap.
        let (tw, th) = (w * 5 / 8 + 1, h * 5 / 8 + 1);
        let (sx, sy) = (w - tw, h - th);
        vec![
            TileRect { x0: 0, y0: 0, x1: tw, y1: th },
            TileRect { x0: sx, y0: 0, x1: w, y1: th },
            TileRect { x0: 0, y0: sy, x1: tw, y1: h },
            TileRect { x0: sx, y0: sy, x1: w, y1: h },
        ]
    }

    #[test]
    fn overlaps_have_no_seams() {
        let (w, h) = (120u32, 90u32);
        let global = ramp(w, h);
        let rects = plan_tiles_for_test(w, h);
        // Tiles disagree with each other: different affine maps plus a different mild curvature.
        let tiles: Vec<(TileRect, DepthMap)> = rects
            .iter()
            .enumerate()
            .map(|(i, &r)| {
                let mut m = crop(&global, r);
                let bend = 0.04 * (i as f32 - 1.5);
                m.data.iter_mut().for_each(|v| *v = (*v + bend * *v * (1.0 - *v)) * (0.6 + 0.2 * i as f32) + 0.1 * i as f32);
                (r, m)
            })
            .collect();
        let out = merge_tiles(&global, &tiles).unwrap();
        let slope = 1.0 / (w - 1) as f32;
        for y in 0..h as usize {
            for x in 1..w as usize {
                let d = (out.data[y * w as usize + x] - out.data[y * w as usize + x - 1]).abs();
                assert!(d < slope * 3.0, "step {d} at ({x},{y})");
            }
        }
        for y in 1..h as usize {
            for x in 0..w as usize {
                let d = (out.data[y * w as usize + x] - out.data[(y - 1) * w as usize + x]).abs();
                assert!(d < slope * 1.5, "vertical step {d} at ({x},{y})");
            }
        }
    }

    #[test]
    fn detail_only_in_tiles_survives() {
        let (w, h) = (120u32, 90u32);
        let global = ramp(w, h);
        let rects = plan_tiles_for_test(w, h);
        let (bx, by) = (30usize, 20usize);
        let tiles: Vec<(TileRect, DepthMap)> = rects
            .iter()
            .map(|&r| {
                let mut m = crop(&global, r);
                // A narrow bump (detail the global pass lacks) in the first tile only.
                if r.x0 == 0 && r.y0 == 0 {
                    m.data[by * m.width as usize + bx] += 0.2;
                }
                (r, m)
            })
            .collect();
        let out = merge_tiles(&global, &tiles).unwrap();
        let i = by * w as usize + bx;
        let around = (out.data[i - 1] + out.data[i + 1]) / 2.0;
        assert!(out.data[i] - around > 0.1, "bump lost: {} vs {around}", out.data[i]);
    }

    #[test]
    fn degenerate_and_invalid_input() {
        let global = ramp(40, 30);
        // A constant tile has no structure: it shifts to the global mean and adds nothing.
        let r = TileRect { x0: 0, y0: 0, x1: 20, y1: 30 };
        let flat = DepthMap { width: 20, height: 30, data: vec![0.7; 600] };
        let out = merge_tiles(&global, &[(r, flat)]).unwrap();
        assert!(out.data.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
        // No tiles: the global map.
        assert_eq!(merge_tiles(&global, &[]).unwrap().data, global.data);
        // Mismatched or out-of-bounds tiles are errors.
        let bad = DepthMap { width: 3, height: 3, data: vec![0.0; 9] };
        assert!(merge_tiles(&global, &[(r, bad)]).is_err());
        let oob = TileRect { x0: 30, y0: 0, x1: 50, y1: 30 };
        assert!(merge_tiles(&global, &[(oob, DepthMap { width: 20, height: 30, data: vec![0.0; 600] })]).is_err());
        assert!(merge_tiles(&DepthMap { width: 0, height: 0, data: vec![] }, &[]).is_err());
    }

    #[test]
    fn tiled_pipeline_with_a_fake_estimator() {
        let (w, h) = (1100u32, 800u32);
        let rgb = vec![0.5f32; (w * h * 3) as usize];
        // The fake sees a different resolution per call: depth is the horizontal position.
        let est = |_: &[f32], tw: u32, th: u32| -> Result<DepthMap, DepthError> { Ok(ramp(tw.min(64), th.min(48))) };
        let calls = std::cell::Cell::new(0u32);
        let last = std::cell::Cell::new(-1.0f32);
        let m = estimate_tiled_with(&est, &rgb, w, h, 16, &|f| {
            calls.set(calls.get() + 1);
            assert!(f >= last.get());
            last.set(f);
            false
        })
        .unwrap();
        assert_eq!((m.width, m.height), (w, h));
        assert_eq!(calls.get(), tile_count(w, h, 16) + 1);
        assert!(m.data.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
        // Cancel stops between tiles.
        let n = std::cell::Cell::new(0);
        let r = estimate_tiled_with(&est, &rgb, w, h, 16, &|_| {
            n.set(n.get() + 1);
            n.get() > 2
        });
        assert_eq!(r, Err(DepthError::Cancelled));
        assert!(estimate_tiled_with(&est, &rgb[..10], w, h, 16, &|_| false).is_err());
        let broken = |_: &[f32], _: u32, _: u32| -> Result<DepthMap, DepthError> { Ok(DepthMap { width: 4, height: 4, data: vec![0.5; 3] }) };
        assert!(estimate_tiled_with(&broken, &rgb, w, h, 16, &|_| false).is_err());
    }
}
