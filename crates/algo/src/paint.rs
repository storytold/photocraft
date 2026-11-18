//! Paint helpers: bucket fill and the gradient tool.

use photocraft_color::blend::{self, BlendMode};
use photocraft_geom::Rect;
use photocraft_raster::{Surface, from_rgba_into};
use serde::{Deserialize, Serialize};

/// How a fill lays down its colour: blended with a mode, or Clear, which erases what it covers
/// (the Paint Bucket's Mode › Clear).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillMode {
    Blend(BlendMode),
    Clear,
}

/// Gradient tool shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum GradientShape {
    #[default]
    Linear,
    Radial,
    Angle,
    Reflected,
    Diamond,
}

/// Gradient tool parameter `t` for a drag from `from` to `to`.
pub fn tool_gradient_t(shape: GradientShape, from: (f32, f32), to: (f32, f32), x: f32, y: f32) -> f32 {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let len = (dx * dx + dy * dy).sqrt().max(1e-6);
    let (ux, uy) = (dx / len, dy / len);
    let (px, py) = (x - from.0, y - from.1);
    let along = px * ux + py * uy;
    let across = -px * uy + py * ux;
    let t = match shape {
        GradientShape::Linear => along / len,
        GradientShape::Radial => (px * px + py * py).sqrt() / len,
        GradientShape::Reflected => along.abs() / len,
        GradientShape::Diamond => (along.abs() + across.abs()) / len,
        GradientShape::Angle => (across.atan2(along) / std::f32::consts::TAU).rem_euclid(1.0),
    };
    t.clamp(0.0, 1.0)
}

/// Samples RGBA stops (sorted by position) at `t`.
pub fn sample_stops(stops: &[(f32, [f32; 4])], t: f32) -> [f32; 4] {
    match stops {
        [] => [0.0; 4],
        [s] => s.1,
        _ => {
            if t <= stops[0].0 {
                return stops[0].1;
            }
            for w in stops.windows(2) {
                if t <= w[1].0 {
                    let k = if w[1].0 > w[0].0 { (t - w[0].0) / (w[1].0 - w[0].0) } else { 0.0 };
                    return std::array::from_fn(|i| w[0].1[i] + (w[1].1[i] - w[0].1[i]) * k);
                }
            }
            stops[stops.len() - 1].1
        }
    }
}

/// Composites `color(i)` over `area` of the surface with per-pixel coverage. Where the coverage is
/// zero, `rim(x, y)` (when given) is the strength of a bucket's transparent-seed fringe there: the
/// pixel takes the fill at coverage `rim × (1 − α)`, its own transparency (#11354 review; see
/// [`bucket_fill_inks`]). Lock Transparent Pixels and Clear ignore it.
#[allow(clippy::too_many_arguments)]
fn composite_area(
    s: &mut Surface,
    area: Rect,
    mode: FillMode,
    coverage: impl Fn(i32, i32) -> f32 + Sync,
    color: impl Fn(i32, i32) -> [f32; 4] + Sync,
    lock_transparency: bool,
    inks: Option<[f32; 4]>,
    rim: Option<&(dyn Fn(i32, i32) -> f32 + Sync)>,
) {
    // Composite tile by tile (in parallel on native); tiles with zero coverage are left untouched,
    // so their copy-on-write storage (and GPU residency) survives.
    let fmt = s.format();
    let n = fmt.channels();
    let tiles: Vec<Rect> = area.tiles().map(|tc| tc.rect().intersect(&area)).filter(|r| !r.is_empty()).collect();
    let src: &Surface = s;
    let work = |r: &Rect| -> Option<(Rect, Vec<f32>)> {
        let w = r.width() as usize;
        let mut any = false;
        let mut region: Vec<f32> = Vec::new();
        let mut enc = [0.0f32; 8];
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let mut k = coverage(x, y);
                let on_rim = match rim {
                    Some(rim) if k <= 0.0 && !lock_transparency && matches!(mode, FillMode::Blend(_)) => rim(x, y),
                    _ => 0.0,
                };
                if k <= 0.0 && on_rim <= 0.0 {
                    continue;
                }
                if !any {
                    region = src.read_region(*r);
                    any = true;
                }
                let i = ((y - r.y0) as usize * w + (x - r.x0) as usize) * n;
                let px = &mut region[i..i + n];
                let d = photocraft_raster::to_rgba(&fmt, px);
                if k <= 0.0 {
                    // A rim pixel: only its transparency takes the fill, so opaque art stays as it is.
                    k = on_rim * (1.0 - d[3].clamp(0.0, 1.0));
                    if k.is_nan() || k <= 0.0 {
                        continue;
                    }
                }
                let o = match mode {
                    FillMode::Blend(m) if lock_transparency => blend::composite_atop(m, d, color(x, y), k),
                    FillMode::Blend(m) => blend::composite(m, d, color(x, y), k),
                    // Locked transparency keeps the alpha, so Clear leaves the pixel as it is.
                    FillMode::Clear if lock_transparency => d,
                    FillMode::Clear => [d[0], d[1], d[2], d[3] * (1.0 - k.min(1.0))],
                };
                from_rgba_into(&fmt, o, &mut enc);
                // A colour picked as CMYK inks replaces fully covered pixels of a CMYK target as it is.
                if let Some(i) = inks
                    && fmt.mode == photocraft_color::ColorMode::Cmyk
                    && mode == FillMode::Blend(BlendMode::Normal)
                    && k >= 1.0
                {
                    enc[..4].copy_from_slice(&i);
                }
                px.copy_from_slice(&enc[..n]);
            }
        }
        any.then_some((*r, region))
    };
    #[cfg(not(target_arch = "wasm32"))]
    let done: Vec<(Rect, Vec<f32>)> = {
        use rayon::prelude::*;
        tiles.par_iter().filter_map(work).collect()
    };
    #[cfg(target_arch = "wasm32")]
    let done: Vec<(Rect, Vec<f32>)> = tiles.iter().filter_map(work).collect();
    for (r, region) in done {
        s.write_region(r, &region);
    }
}

/// Gradient tool over `area` (the selection bounds or canvas).
#[allow(clippy::too_many_arguments)]
pub fn paint_gradient(
    s: &mut Surface,
    area: Rect,
    from: (f32, f32),
    to: (f32, f32),
    shape: GradientShape,
    stops: &[(f32, [f32; 4])],
    reverse: bool,
    opacity: f32,
    blend_mode: BlendMode,
    dither: bool,
    selection: Option<&Surface>,
    lock_transparency: bool,
) {
    composite_area(
        s,
        area,
        FillMode::Blend(blend_mode),
        |x, y| opacity * selection.map_or(1.0, |m| m.sample_channel(x, y, 0)),
        |x, y| {
            // Sampled one pixel up and left of the pixel centre, where the reference application samples (#12018).
            let t = tool_gradient_t(shape, from, to, x as f32 - 0.5, y as f32 - 0.5);
            let mut c = sample_stops(stops, if reverse { 1.0 - t } else { t });
            if dither {
                // One quantisation step of monochromatic noise breaks 8-bit banding without speckle.
                let n = (photocraft_color::dither_noise(x, y) - 0.5) / 255.0;
                for ch in c.iter_mut().take(3) {
                    *ch = (*ch + n).clamp(0.0, 1.0);
                }
            }
            c
        },
        lock_transparency,
        None,
        None,
    );
}

/// Paint bucket: fills the region similar to the seed pixel (on the layer's
/// own pixels) within `area`, respecting the selection.
#[allow(clippy::too_many_arguments)]
pub fn bucket_fill(
    s: &mut Surface,
    area: Rect,
    seed: (i32, i32),
    tolerance: f32,
    contiguous: bool,
    anti_alias: bool,
    soft_edge: bool,
    color: [f32; 4],
    opacity: f32,
    selection: Option<&Surface>,
    lock_transparency: bool,
) -> bool {
    bucket_fill_src(s, area, seed, tolerance, contiguous, anti_alias, soft_edge, opacity, selection, lock_transparency, |_, _| color)
}

/// How far (in pixels) the soft edge of a bucket fill reaches past the pixels that match the seed.
const FRINGE: i32 = 3;

/// The region a Paint Bucket click fills: the pixels similar to the seed by the bucket's match rule
/// ([`crate::selection::wand_region_bucket`]), with the wand-style edge (`anti_alias` smooths just
/// the fill's boundary) or, with `soft_edge`, a soft edge that reaches past it.
///
/// A line's anti-aliased edge is a ramp from the sheet's colour to the line's, and its outer
/// pixels are too far from the seed colour to match, so a plain flood fill leaves them as a halo
/// between the fill and the line. With `soft_edge` (`anti_alias` then plays no part), every non-matching pixel within [`FRINGE`]
/// pixels of the region is covered by how much of it is the seed colour rather than the
/// strongest different colour on its ramp: `1 - d / d_max`, where `d` is its distance from the
/// seed colour and `d_max` the distance at the top of the ramp (the line). A ramp climbs in
/// strictly growing steps from the region to the line, so a half-line pixel is half filled, while
/// the line itself, a flat patch of another colour, a mid-tone line two pixels wide and the far
/// side of a thin line (whose ramp leads away from the region) are not, and a hard edge stays hard. Matching pixels keep full coverage (the Magic Wand's own edge blend would under-fill
/// them). Pixels of a similar colour that are not connected to the seed (`contiguous`) are never
/// covered.
pub fn bucket_region(
    img: &[[u8; 4]],
    area: Rect,
    seed: (i32, i32),
    tolerance: f32,
    contiguous: bool,
    anti_alias: bool,
    soft_edge: bool,
) -> Option<crate::selection::Region> {
    if !soft_edge {
        return crate::selection::wand_region_bucket(img, area, seed, tolerance, contiguous, anti_alias);
    }
    let core = crate::selection::wand_region_bucket(img, area, seed, tolerance, contiguous, false)?;
    let w = area.width() as usize;
    let tol = crate::selection::tolerance_steps(tolerance);
    let at = |x: i32, y: i32| -> Option<[u8; 4]> {
        if !area.contains(x, y) {
            return None;
        }
        img.get((y - area.y0) as usize * w + (x - area.x0) as usize).copied()
    };
    let target = at(seed.0, seed.1)?;
    // Distance from the seed colour. Against a transparent seed only opacity counts: the colour
    // under zero alpha is meaningless.
    // The bucket's own match rule (shared with `wand_region_bucket`), so a pixel it left out for
    // being a different colour is never mistaken for a disconnected part of the seed's region.
    let similar =
        |p: [u8; 4]| crate::selection::similar_by_rule(crate::selection::MatchRule::Bucket, target, tol, p, crate::selection::ColorDistance::LargestChannel);
    let dist = |p: [u8; 4]| -> i32 {
        if target[3] == 0 { i32::from(p[3]) } else { (0..4).map(|c| (i32::from(p[c]) - i32::from(target[c])).abs()).max().unwrap_or(0) }
    };
    let b = core.bbox;
    let grown = Rect::new((b.x0 - FRINGE).max(area.x0), (b.y0 - FRINGE).max(area.y0), (b.x1 + FRINGE).min(area.x1), (b.y1 + FRINGE).min(area.y1));
    let (gw, gh) = (grown.width() as usize, grown.height() as usize);
    let bw = b.width() as usize;
    let mut mask = vec![0u8; gw.checked_mul(gh)?];
    let idx = |x: i32, y: i32| (y - grown.y0) as usize * gw + (x - grown.x0) as usize;
    for y in b.y0..b.y1 {
        let src = (y - b.y0) as usize * bw;
        if let (Some(s), Some(d)) = (core.mask.get(src..src + bw), mask.get_mut(idx(b.x0, y)..idx(b.x0, y) + bw)) {
            d.copy_from_slice(s);
        }
    }
    let matched = |x: i32, y: i32| core.at(x, y) > 0.0;
    // Candidates: the unmatched pixels within FRINGE of the region's edge. The region's edge is
    // found row by row, then spread FRINGE pixels sideways and then up and down, each pass over
    // independent rows.
    let bh = (b.y1 - b.y0) as usize;
    let mut edge = vec![false; bw.checked_mul(bh)?];
    for_each_row(&mut edge, bw, |ry, row| {
        let y = b.y0 + ry as i32;
        for (rx, e) in row.iter_mut().enumerate() {
            let x = b.x0 + rx as i32;
            *e = matched(x, y) && ![(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)].iter().all(|&(nx, ny)| matched(nx, ny) || !area.contains(nx, ny));
        }
    });
    let mut spread = vec![false; gw.checked_mul(bh)?];
    for_each_row(&mut spread, gw, |ry, row| {
        for (gx, s) in row.iter_mut().enumerate() {
            let x = grown.x0 + gx as i32;
            *s = ((x - FRINGE).max(b.x0)..(x + FRINGE + 1).min(b.x1)).any(|nx| edge.get(ry * bw + (nx - b.x0) as usize).copied().unwrap_or(false));
        }
    });
    let mut candidate = vec![false; mask.len()];
    for_each_row(&mut candidate, gw, |gy, row| {
        let y = grown.y0 + gy as i32;
        let rows = (y - FRINGE).max(b.y0)..(y + FRINGE + 1).min(b.y1);
        for (gx, c) in row.iter_mut().enumerate() {
            *c = rows.clone().any(|ny| spread.get((ny - b.y0) as usize * gw + gx).copied().unwrap_or(false));
        }
    });
    const AROUND: [(i32, i32); 8] = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];
    let touches_region = |c: (i32, i32)| AROUND.iter().any(|&(dx, dy)| matched(c.0 + dx, c.1 + dy));
    // One step along the ramp: the neighbour farthest from the seed colour (`outward`) or the
    // one closest to it, provided it is strictly farther or closer than where we stand.
    let climb = |(pos, cur): ((i32, i32), i32), outward: bool| -> Option<((i32, i32), i32)> {
        let mut best: Option<((i32, i32), i32)> = None;
        for (dx, dy) in AROUND {
            let n = (pos.0 + dx, pos.1 + dy);
            let Some(q) = at(n.0, n.1) else { continue };
            let nd = dist(q);
            let ok = if outward { nd > cur } else { nd < cur };
            if ok && best.is_none_or(|(_, bd)| if outward { nd > bd } else { nd < bd }) {
                best = Some((n, nd));
            }
        }
        best
    };
    // Each candidate reads only the image and the matched region, and writes its own mask byte,
    // so the rows are covered in parallel.
    for_each_row(&mut mask, gw, |gy, row| {
        let y = grown.y0 + gy as i32;
        for (gx, m) in row.iter_mut().enumerate() {
            let x = grown.x0 + gx as i32;
            if !candidate.get(gy * gw + gx).copied().unwrap_or(false) || matched(x, y) {
                continue;
            }
            let Some(p) = at(x, y) else { continue };
            // Similar but not connected to the seed: another region, not an edge.
            if similar(p) {
                continue;
            }
            let d = dist(p);
            // Only a ramp is an edge: pixels ever more different from the seed colour, from the
            // region out to the line. Walk in to the region (each step strictly closer to the
            // seed colour) and out to the line's peak (strictly farther). A flat patch of another
            // colour, a lone mid-tone line or the far side of a thin line has no such ramp, so a
            // darker pixel nearby cannot cover it.
            let (mut cur, mut cd) = ((x, y), d);
            let mut reaches = touches_region(cur);
            for _ in 0..FRINGE {
                if reaches {
                    break;
                }
                let Some((n, nd)) = climb((cur, cd), false) else { break };
                (cur, cd) = (n, nd);
                reaches = touches_region(cur);
            }
            if !reaches {
                continue;
            }
            let (mut peak, mut pd) = ((x, y), d);
            for _ in 0..FRINGE {
                let Some((n, nd)) = climb((peak, pd), true) else { break };
                (peak, pd) = (n, nd);
            }
            let d_max = pd;
            if d_max <= d || d_max <= 0 {
                continue;
            }
            let coverage = 1.0 - d as f32 / d_max as f32;
            if coverage >= 0.04 {
                *m = (coverage * 255.0).round() as u8;
            }
        }
    });
    Some(crate::selection::Region { bbox: grown, mask })
}

/// Run `f(row index, row)` over the `width`-wide rows of `buf`, in parallel where threads exist.
fn for_each_row<T: Send>(buf: &mut [T], width: usize, f: impl Fn(usize, &mut [T]) + Sync + Send) {
    if width == 0 {
        return;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        buf.par_chunks_mut(width).enumerate().for_each(|(i, row)| f(i, row));
    }
    #[cfg(target_arch = "wasm32")]
    buf.chunks_mut(width).enumerate().for_each(|(i, row)| f(i, row));
}

/// Like [`bucket_fill`] but the fill colour comes from `src(x, y)` — e.g. a pattern sampled at the
/// canvas position — so the Paint Bucket can fill with a pattern as well as a solid colour.
#[allow(clippy::too_many_arguments)]
pub fn bucket_fill_src(
    s: &mut Surface,
    area: Rect,
    seed: (i32, i32),
    tolerance: f32,
    contiguous: bool,
    anti_alias: bool,
    soft_edge: bool,
    opacity: f32,
    selection: Option<&Surface>,
    lock_transparency: bool,
    src: impl Fn(i32, i32) -> [f32; 4] + Sync,
) -> bool {
    bucket_fill_inks(
        s,
        area,
        seed,
        tolerance,
        contiguous,
        anti_alias,
        soft_edge,
        opacity,
        selection,
        lock_transparency,
        src,
        None,
        FillMode::Blend(BlendMode::Normal),
    )
}

/// [`bucket_fill_src`] where `inks` (CMYK, 0..1) are what a fully covered pixel of a CMYK target
/// gets instead of `src`'s sRGB stand-in (a colour that was picked as CMYK), laid down with `mode`
/// (the options bar's Mode).
#[allow(clippy::too_many_arguments)]
pub fn bucket_fill_inks(
    s: &mut Surface,
    area: Rect,
    seed: (i32, i32),
    tolerance: f32,
    contiguous: bool,
    anti_alias: bool,
    soft_edge: bool,
    opacity: f32,
    selection: Option<&Surface>,
    lock_transparency: bool,
    src: impl Fn(i32, i32) -> [f32; 4] + Sync,
    inks: Option<[f32; 4]>,
    mode: FillMode,
) -> bool {
    if !area.contains(seed.0, seed.1) {
        return false;
    }
    let img = crate::selection::rgba8_image(s, area);
    let Some(region) = bucket_region(&img, area, seed, tolerance, contiguous, anti_alias, soft_edge) else { return false };
    // Clicked on transparency with Anti-alias on: the wand's fringe gives the partly transparent
    // pixels around the flood no coverage (their alpha is measured against the seed's 0), but the
    // reference application fills each pixel 8-adjacent to the flood at coverage `1 − α`, so the
    // soft rim a cleared selection leaves rises partly (α 187 → 205, 56 → 211, 3 → 252; the
    // reference gives 204, 211, 253) and a black edge pixel of alpha 40 filled green becomes
    // (0, 248, 0, 221) (#11311, #12528, #11354 review). Pixels further out stay as they are; with
    // Anti-alias off nothing outside the flood changes. The soft edge already covers that rim at
    // `1 − α` (its ramp is the alpha), so the rule is not applied on top of it.
    let at_seed = usize::try_from(seed.1 - area.y0)
        .ok()
        .and_then(|y| y.checked_mul(area.width() as usize))
        .and_then(|row| row.checked_add(usize::try_from(seed.0 - area.x0).ok()?));
    let empty_seed = anti_alias && !soft_edge && at_seed.and_then(|i| img.get(i)).is_some_and(|p| p[3] == 0);
    drop(img);
    fill_region(s, &region, opacity, selection, lock_transparency, src, inks, mode, empty_seed);
    true
}

/// Paints a bucket region sampled before the edit, so its boundary can come from the visible
/// composite while only `s` receives the colour or pattern. Selection coverage still clips it.
pub fn bucket_fill_region_src(
    s: &mut Surface,
    region: &crate::selection::Region,
    opacity: f32,
    selection: Option<&Surface>,
    lock_transparency: bool,
    src: impl Fn(i32, i32) -> [f32; 4] + Sync,
) {
    bucket_fill_region_inks(s, region, opacity, selection, lock_transparency, src, None, FillMode::Blend(BlendMode::Normal));
}

/// [`bucket_fill_region_src`] with the CMYK `inks` and the `mode` of [`bucket_fill_inks`].
#[allow(clippy::too_many_arguments)]
pub fn bucket_fill_region_inks(
    s: &mut Surface,
    region: &crate::selection::Region,
    opacity: f32,
    selection: Option<&Surface>,
    lock_transparency: bool,
    src: impl Fn(i32, i32) -> [f32; 4] + Sync,
    inks: Option<[f32; 4]>,
    mode: FillMode,
) {
    fill_region(s, region, opacity, selection, lock_transparency, src, inks, mode, false);
}

/// [`bucket_fill_region_inks`]; with `rim`, the pixels 8-adjacent to the region's fully covered
/// pixels that the region itself leaves uncovered take the fill at coverage `1 − α` (see
/// [`composite_area`]). Fully covered pixels and pixels further out are filled as without it.
#[allow(clippy::too_many_arguments)]
fn fill_region(
    s: &mut Surface,
    region: &crate::selection::Region,
    opacity: f32,
    selection: Option<&Surface>,
    lock_transparency: bool,
    src: impl Fn(i32, i32) -> [f32; 4] + Sync,
    inks: Option<[f32; 4]>,
    mode: FillMode,
    rim: bool,
) {
    // Composite only over the filled region's box (with Anti-alias on it already holds the
    // one-pixel ring around the flood).
    let b = region.bbox;
    let strength = |x: i32, y: i32| opacity * selection.map_or(1.0, |m| m.sample_channel(x, y, 0));
    // The ring is only ever read for an unlocked Blend fill (see composite_area), and it is
    // empty when the region covers every pixel, so do not build it in those cases: on a large
    // canvas it is a bool per pixel plus a full scan of the mask.
    let use_rim = rim && !lock_transparency && matches!(mode, FillMode::Blend(_)) && region.mask.contains(&0);
    let ring = if use_rim { rim_ring(region) } else { Vec::new() };
    let on_ring = |x: i32, y: i32| {
        let at = usize::try_from(y - b.y0).ok().zip(usize::try_from(x - b.x0).ok()).filter(|_| b.contains(x, y));
        let hit = at.and_then(|(yy, xx)| yy.checked_mul(b.width() as usize)?.checked_add(xx)).and_then(|i| ring.get(i));
        if hit == Some(&true) { strength(x, y) } else { 0.0 }
    };
    let rim: Option<&(dyn Fn(i32, i32) -> f32 + Sync)> = if use_rim { Some(&on_ring) } else { None };
    composite_area(s, b, mode, |x, y| region.at(x, y) * strength(x, y), src, lock_transparency, inks, rim);
}

/// Over `region.bbox` (row-major): the pixels the region leaves uncovered that touch one of its
/// fully covered pixels, diagonals included.
fn rim_ring(region: &crate::selection::Region) -> Vec<bool> {
    let (w, h) = (region.bbox.width() as usize, region.bbox.height() as usize);
    let full = |x: usize, y: usize| y.checked_mul(w).and_then(|r| r.checked_add(x)).and_then(|i| region.mask.get(i)) == Some(&255);
    let n = w.saturating_mul(h).min(region.mask.len());
    (0..n)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            region.mask.get(i) == Some(&0)
                && (y.saturating_sub(1)..=(y + 1).min(h - 1)).any(|ny| (x.saturating_sub(1)..=(x + 1).min(w - 1)).any(|nx| (nx, ny) != (x, y) && full(nx, ny)))
        })
        .collect()
}

#[cfg(test)]
#[path = "paint_gradient_sample_tests.rs"]
mod paint_gradient_sample_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;

    #[test]
    fn gradient_t_shapes() {
        let (f, t) = ((0.0, 0.0), (10.0, 0.0));
        assert_eq!(tool_gradient_t(GradientShape::Linear, f, t, 5.0, 3.0), 0.5);
        assert_eq!(tool_gradient_t(GradientShape::Linear, f, t, -5.0, 0.0), 0.0);
        assert_eq!(tool_gradient_t(GradientShape::Radial, f, t, 0.0, 5.0), 0.5);
        assert_eq!(tool_gradient_t(GradientShape::Reflected, f, t, -5.0, 0.0), 0.5);
        assert_eq!(tool_gradient_t(GradientShape::Diamond, f, t, 2.5, 2.5), 0.5);
        assert!((tool_gradient_t(GradientShape::Angle, f, t, 0.0, 5.0) - 0.25).abs() < 1e-5);
    }

    #[test]
    fn gradient_paints_ramp_within_selection() {
        let mut s = Surface::new(PixelFormat::RGBA8);
        let a = Rect::new(0, 0, 11, 2);
        let mut sel = Surface::new(PixelFormat::GRAY8);
        sel.fill_rect(Rect::new(0, 0, 11, 1), &[1.0]);
        paint_gradient(
            &mut s,
            a,
            (-0.5, 0.0),
            (9.5, 0.0),
            GradientShape::Linear,
            &[(0.0, [0.0, 0.0, 0.0, 1.0]), (1.0, [1.0, 1.0, 1.0, 1.0])],
            false,
            1.0,
            BlendMode::Normal,
            false,
            Some(&sel),
            false,
        );
        assert!(s.pixel(0, 0)[0] < 0.01 && s.pixel(10, 0)[0] > 0.99);
        assert!((s.pixel(5, 0)[0] - 0.5).abs() < 0.01);
        assert_eq!(s.pixel(5, 1)[3], 0.0, "outside the selection");
    }

    #[test]
    fn locked_gradient_blends_source_atop_on_semi_transparent_pixels() {
        // Locked half-transparent white, flat black gradient at 50%: colour halfway, alpha kept (#5783).
        for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
            for mode in [BlendMode::Normal, BlendMode::Multiply] {
                let mut s = Surface::new(fmt);
                let a = Rect::new(0, 0, 4, 1);
                s.fill_rect(a, &[1.0, 1.0, 1.0, 0.5]);
                let black = [0.0, 0.0, 0.0, 1.0];
                paint_gradient(&mut s, a, (0.0, 0.0), (4.0, 0.0), GradientShape::Linear, &[(0.0, black), (1.0, black)], false, 0.5, mode, false, None, true);
                let p = s.rgba(2, 0);
                assert!((p[0] - 0.5).abs() < 0.01 && (p[3] - 0.5).abs() < 0.01, "{fmt:?} {mode:?} {p:?}");
            }
        }
    }

    #[test]
    fn bucket_fills_contiguous_region() {
        let mut s = Surface::new(PixelFormat::RGBA8);
        let a = Rect::new(0, 0, 10, 10);
        s.fill_rect(a, &[1.0, 1.0, 1.0, 1.0]);
        s.fill_rect(Rect::new(5, 0, 6, 10), &[0.0, 0.0, 0.0, 1.0]);
        assert!(bucket_fill(&mut s, a, (1, 1), 10.0, true, false, false, [1.0, 0.0, 0.0, 1.0], 1.0, None, false));
        assert_eq!(s.pixel(2, 2), vec![1.0, 0.0, 0.0, 1.0]);
        assert_eq!(s.pixel(8, 2), vec![1.0, 1.0, 1.0, 1.0]);
        assert!(!bucket_fill(&mut s, a, (50, 1), 10.0, true, false, false, [1.0, 0.0, 0.0, 1.0], 1.0, None, false));
    }

    #[test]
    fn bucket_region_uses_document_coordinates_and_multiplies_coverage() {
        let mut s = Surface::new(PixelFormat::RGBA8);
        let region = crate::selection::Region { bbox: Rect::new(-3, 4, 0, 5), mask: vec![255, 128, 255] };
        let mut selection = Surface::new(PixelFormat::GRAY8);
        selection.fill_rect(Rect::new(-3, 4, -1, 5), &[0.5]);
        bucket_fill_region_src(&mut s, &region, 0.5, Some(&selection), false, |_, _| [1.0, 0.0, 0.0, 1.0]);
        assert!((s.rgba(-3, 4)[3] - 0.25).abs() < 0.005);
        assert!((s.rgba(-2, 4)[3] - 0.125).abs() < 0.005);
        assert_eq!(s.rgba(-1, 4), [0.0; 4], "outside the selection");
        assert_eq!(s.rgba(-4, 4), [0.0; 4], "outside the region");
    }

    /// A white sheet with a black line at x = 10 whose edges are anti-aliased: columns 8 and 12
    /// are 25 % line, 9 and 11 are 50 % and 75 % line (a soft three-pixel ramp each side).
    fn soft_line_sheet() -> (Surface, Rect) {
        let a = Rect::new(0, 0, 20, 6);
        let mut s = Surface::new(PixelFormat::RGBA8);
        s.fill_rect(a, &[1.0, 1.0, 1.0, 1.0]);
        for (x, line) in [(8, 0.25), (9, 0.5), (11, 0.5), (12, 0.25), (10, 1.0), (7, 0.1), (13, 0.1)] {
            s.fill_rect(Rect::new(x, 0, x + 1, 6), &[1.0 - line, 1.0 - line, 1.0 - line, 1.0]);
        }
        (s, a)
    }

    #[test]
    fn bucket_covers_the_soft_edge_of_a_line_instead_of_leaving_a_halo() {
        let (mut s, a) = soft_line_sheet();
        let img = crate::selection::rgba8_image(&s, a);
        let region = bucket_region(&img, a, (2, 2), 32.0, true, false, true).unwrap();
        // The flat white is filled completely, right up to the ramp (no under-covered edge pixel).
        assert_eq!(region.at(6, 2), 1.0, "flat white next to the ramp");
        assert_eq!(region.at(2, 2), 1.0);
        // The ramp is covered in proportion to how white it is, so nothing is left as a halo:
        // coverage = 1 - line strength.
        assert_eq!(region.at(7, 2), 1.0, "within tolerance, so filled completely");
        for (x, line) in [(8, 0.25), (9, 0.5)] {
            let want = 1.0 - line;
            assert!((region.at(x, 2) - want).abs() < 0.08, "x = {x}: {} vs {want}", region.at(x, 2));
        }
        // The line itself and everything past it stay untouched.
        assert_eq!(region.at(10, 2), 0.0);
        assert_eq!(region.at(15, 2), 0.0, "the other side of the line is another region");
        // Painted: the ramp pixels move toward the fill colour instead of staying light grey.
        bucket_fill(&mut s, a, (2, 2), 32.0, true, false, true, [1.0, 0.0, 0.0, 1.0], 1.0, None, false);
        assert!(s.pixel(8, 2)[1] < 0.5, "the soft pixel next to the line is mostly filled: {:?}", s.pixel(8, 2));
        assert_eq!(s.pixel(10, 2)[0..3], [0.0, 0.0, 0.0], "the line itself is not painted");
    }

    #[test]
    fn bucket_edges_stay_crisp_without_a_soft_edge_and_without_anti_alias() {
        // A hard black line: the white next to it is filled fully and the line not at all.
        let a = Rect::new(0, 0, 12, 4);
        let mut s = Surface::new(PixelFormat::RGBA8);
        s.fill_rect(a, &[1.0, 1.0, 1.0, 1.0]);
        s.fill_rect(Rect::new(6, 0, 7, 4), &[0.0, 0.0, 0.0, 1.0]);
        let img = crate::selection::rgba8_image(&s, a);
        let hard = bucket_region(&img, a, (1, 1), 32.0, true, false, true).unwrap();
        assert_eq!((hard.at(5, 1), hard.at(6, 1), hard.at(7, 1)), (1.0, 0.0, 0.0));
        // Anti-alias off: exactly the pixels within tolerance.
        let (s, a) = soft_line_sheet();
        let img = crate::selection::rgba8_image(&s, a);
        let off = bucket_region(&img, a, (2, 2), 32.0, true, false, false).unwrap();
        assert_eq!((off.at(6, 2), off.at(8, 2), off.at(9, 2)), (1.0, 0.0, 0.0));
    }

    #[test]
    fn bucket_fills_up_to_line_art_drawn_on_a_transparent_layer() {
        // Anti-aliased strokes on a transparent layer: alpha ramps 0 → 255 across the stroke.
        let a = Rect::new(0, 0, 20, 4);
        let mut s = Surface::new(PixelFormat::RGBA8);
        for (x, alpha) in [(9, 0.2), (10, 0.6), (11, 1.0), (12, 0.6), (13, 0.2)] {
            s.fill_rect(Rect::new(x, 0, x + 1, 4), &[0.0, 0.0, 0.0, alpha]);
        }
        let img = crate::selection::rgba8_image(&s, a);
        let region = bucket_region(&img, a, (2, 2), 32.0, true, false, true).unwrap();
        assert_eq!(region.at(2, 2), 1.0);
        assert_eq!(region.at(11, 2), 0.0, "the stroke core");
        // A stroke edge of alpha 0.6 gets 40 % fill: the fill meets the line without a gap.
        assert!((region.at(10, 2) - 0.4).abs() < 0.1, "{}", region.at(10, 2));
        assert!((region.at(9, 2) - 0.8).abs() < 0.1, "{}", region.at(9, 2));
        assert_eq!(region.at(15, 2), 0.0, "beyond the stroke is a separate region");
    }

    /// Without the soft edge the region is exactly the Magic Wand's, whether or not Anti-alias is on
    /// (so existing fills, pattern fills included, are unchanged); the soft edge is its own option.
    #[test]
    fn bucket_without_a_soft_edge_is_the_magic_wands_region() {
        let (s, a) = soft_line_sheet();
        let img = crate::selection::rgba8_image(&s, a);
        for (contiguous, anti_alias) in [(true, false), (true, true), (false, false), (false, true)] {
            let wand = crate::selection::wand_region(&img, a, (2, 2), 32.0, contiguous, anti_alias).unwrap();
            let bucket = bucket_region(&img, a, (2, 2), 32.0, contiguous, anti_alias, false).unwrap();
            assert_eq!((bucket.bbox, &bucket.mask), (wand.bbox, &wand.mask), "contiguous {contiguous}, anti-alias {anti_alias}");
        }
        // And the soft edge reaches past what the Magic Wand takes, with or without Anti-alias.
        let soft = bucket_region(&img, a, (2, 2), 32.0, true, false, true).unwrap();
        let wand = crate::selection::wand_region(&img, a, (2, 2), 32.0, true, true).unwrap();
        assert!(soft.at(9, 2) > wand.at(9, 2) + 0.2, "soft {} vs wand {}", soft.at(9, 2), wand.at(9, 2));
        let with_aa = bucket_region(&img, a, (2, 2), 32.0, true, true, true).unwrap();
        assert_eq!(soft.mask, with_aa.mask, "Anti-alias does not change the soft edge");
    }

    /// The soft edge decides what is "similar" exactly as the Magic Wand does, tolerance and the
    /// transparent-seed rule included, so a pixel the wand takes is never given a partial edge.
    #[test]
    fn the_soft_edge_agrees_with_the_magic_wand_on_what_is_similar() {
        let a = Rect::new(0, 0, 8, 3);
        let mut img = vec![[0u8; 4]; 24];
        for y in 0..3 {
            for (x, px) in [(4usize, [9u8, 200, 30, 0]), (5, [255, 0, 0, 255]), (6, [255, 0, 0, 255]), (7, [255, 0, 0, 255])] {
                img[y * 8 + x] = px;
            }
        }
        // A transparent seed: invisible pixels count as similar whatever RGB they hold.
        let wand = crate::selection::wand_region(&img, a, (0, 1), 8.0, true, false).unwrap();
        let bucket = bucket_region(&img, a, (0, 1), 8.0, true, false, true).unwrap();
        assert_eq!(wand.at(4, 1), 1.0);
        assert_eq!(bucket.at(4, 1), 1.0, "a transparent pixel with other RGB is still part of the region");
        // Tolerance is the same whole-step rule, NaN and negative included.
        for tol in [0.0, 7.9999, 8.0, -3.0, f32::NAN] {
            assert_eq!(crate::selection::tolerance_steps(tol), (tol + 1e-3).floor().max(0.0) as i32);
        }
        assert!(crate::selection::similar_to_seed([10, 10, 10, 255], 8, [18, 2, 10, 255], crate::selection::ColorDistance::LargestChannel));
        assert!(!crate::selection::similar_to_seed([10, 10, 10, 255], 8, [19, 10, 10, 255], crate::selection::ColorDistance::LargestChannel));
        assert!(crate::selection::similar_to_seed([0, 0, 0, 0], 0, [9, 9, 9, 0], crate::selection::ColorDistance::LargestChannel));
        assert!(!crate::selection::similar_to_seed([0, 0, 0, 255], 0, [0, 0, 0, 254], crate::selection::ColorDistance::LargestChannel));
    }

    /// Greyscale rows, three tall, from one brightness per column.
    fn grey_sheet(cols: &[u8]) -> (Vec<[u8; 4]>, Rect) {
        let a = Rect::new(0, 0, cols.len() as i32, 3);
        (cols.iter().cycle().take(cols.len() * 3).map(|&v| [v, v, v, 255]).collect(), a)
    }

    #[test]
    fn bucket_fringe_ignores_flat_patches_and_lines_that_are_not_a_ramp() {
        let covered = |cols: &[u8]| {
            let (img, a) = grey_sheet(cols);
            let r = bucket_region(&img, a, (0, 1), 8.0, true, false, true).unwrap();
            (0..cols.len() as i32).map(|x| r.at(x, 1)).collect::<Vec<_>>()
        };
        // A two-colour boundary stays hard.
        assert_eq!(covered(&[255, 255, 0, 0]), [1.0, 1.0, 0.0, 0.0]);
        // A flat grey patch beside the seed with a black line within 3 px is another region.
        let flat = covered(&[255, 255, 128, 128, 128, 0, 0]);
        assert_eq!(&flat[2..4], [0.0, 0.0], "{flat:?}");
        // A light line two pixels wide beside a dark outline is not painted either.
        let light = covered(&[255, 255, 200, 200, 0, 0]);
        assert_eq!(&light[2..4], [0.0, 0.0], "{light:?}");
        // A real ramp still is, in proportion.
        let ramp = covered(&[255, 255, 192, 128, 64, 0, 0]);
        assert!((ramp[2] - 0.75).abs() < 0.05 && (ramp[3] - 0.5).abs() < 0.05 && ramp[5] == 0.0, "{ramp:?}");
    }

    #[test]
    fn bucket_fringe_stops_at_the_far_side_of_a_thin_line() {
        // seed | ramp | 1 px line | ramp | other region, all one tolerance apart.
        let (img, a) = grey_sheet(&[255, 255, 128, 0, 128, 255, 255]);
        let r = bucket_region(&img, a, (0, 1), 8.0, true, false, true).unwrap();
        assert!((r.at(2, 1) - 0.5).abs() < 0.05);
        assert_eq!((r.at(3, 1), r.at(4, 1), r.at(5, 1), r.at(6, 1)), (0.0, 0.0, 0.0, 0.0), "the other side of the line");
    }

    #[test]
    fn bucket_fringe_covers_a_faint_coloured_stroke_pixel_next_to_a_transparent_seed() {
        // Transparent sheet, a red stroke whose edge pixel is only 8 % opaque.
        let a = Rect::new(0, 0, 6, 3);
        let mut img = vec![[0u8; 4]; 18];
        for y in 0..3 {
            for (x, alpha) in [(3usize, 20u8), (4, 128), (5, 255)] {
                img[y * 6 + x] = [255, 0, 0, alpha];
            }
        }
        let r = bucket_region(&img, a, (0, 1), 8.0, true, false, true).unwrap();
        assert!((r.at(3, 1) - (1.0 - 20.0 / 255.0)).abs() < 0.05, "{}", r.at(3, 1));
        assert!((r.at(4, 1) - 0.5).abs() < 0.05);
        assert_eq!(r.at(5, 1), 0.0);
    }

    /// Release timing on a 24 MP sheet (`cargo test -p photocraft-algo --release -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn bucket_region_timing_on_24_megapixels() {
        use std::time::Instant;
        let (w, h) = (6000i32, 4000i32);
        let a = Rect::new(0, 0, w, h);
        // White sheet crossed by soft grey lines every 40 px, plus noise-like texture.
        let img: Vec<[u8; 4]> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let line = [0.0, 0.25, 0.5, 1.0, 0.5, 0.25][(x % 40).min(5) as usize] * f32::from(x % 40 < 6) + if y % 53 < 2 { 0.5 } else { 0.0 };
                let v = (255.0 * (1.0 - line.min(1.0))) as u8;
                let t = ((x * 7 + y * 13) % 5) as u8;
                [v.saturating_sub(t), v.saturating_sub(t), v.saturating_sub(t), 255]
            })
            .collect();
        for (contiguous, soft) in [(true, false), (true, true), (false, false), (false, true)] {
            let t = Instant::now();
            let r = bucket_region(&img, a, (5, 5), 32.0, contiguous, false, soft).unwrap();
            println!("bucket_region contiguous={contiguous} soft_edge={soft}: {:?} ({} bytes)", t.elapsed(), r.mask.len());
        }
        for contiguous in [true, false] {
            let t = Instant::now();
            let r = crate::selection::wand_region(&img, a, (5, 5), 32.0, contiguous, true).unwrap();
            println!("wand_region (previous bucket path) contiguous={contiguous} aa: {:?} ({} bytes)", t.elapsed(), r.mask.len());
        }
    }

    #[test]
    fn bucket_region_edge_cases_do_not_panic() {
        let a = Rect::new(0, 0, 3, 3);
        let img = vec![[255u8; 4]; 9];
        // Seed outside the area, an area larger than the data, a one-pixel area, global mode.
        assert!(bucket_region(&img, a, (5, 5), 32.0, true, false, true).is_none());
        assert!(bucket_region(&img[..4], a, (1, 1), 32.0, true, false, true).is_none());
        let one = bucket_region(&img[..1], Rect::new(0, 0, 1, 1), (0, 0), 32.0, true, false, true).unwrap();
        assert_eq!(one.at(0, 0), 1.0);
        let all = bucket_region(&img, a, (1, 1), 0.0, false, false, true).unwrap();
        assert!((0..3).all(|y| (0..3).all(|x| all.at(x, y) == 1.0)));
        // NaN and absurd tolerances.
        assert!(bucket_region(&img, a, (1, 1), f32::NAN, true, false, true).is_some());
        assert!(bucket_region(&img, a, (1, 1), 1e30, true, false, true).is_some());
    }

    /// #11311: flooding transparency with anti-aliasing fills the empty part of the region's edge
    /// pixels at the fill's opacity, so the edge is no semi-transparent ring; opaque pixels the
    /// anti-aliasing reaches blend exactly as before.
    #[test]
    fn bucket_on_transparency_fills_the_edge_pixels_in_full() {
        let a = Rect::new(0, 0, 8, 1);
        let white = |_, _| [1.0, 1.0, 1.0, 1.0];
        for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
            for (opaque, opacity) in [([1.0, 1.0, 1.0, 1.0], 1.0), ([1.0, 1.0, 1.0, 1.0], 0.5), ([1.0, 0.0, 0.0, 1.0], 1.0)] {
                let mut s = Surface::new(fmt);
                s.fill_rect(Rect::new(4, 0, 8, 1), &opaque);
                // The same region filled without the rule: what the bucket used to do.
                let mut before = s.clone();
                let img = crate::selection::rgba8_image(&before, a);
                let region = crate::selection::wand_region(&img, a, (1, 0), 0.0, true, true).unwrap();
                bucket_fill_region_inks(&mut before, &region, opacity, None, false, white, None, FillMode::Blend(BlendMode::Normal));

                assert!(bucket_fill_inks(&mut s, a, (1, 0), 0.0, true, true, false, opacity, None, false, white, None, FillMode::Blend(BlendMode::Normal)));
                for x in 0..4 {
                    let p = s.rgba(x, 0);
                    assert!(p[..3].iter().all(|c| (c - 1.0).abs() < 1e-3) && (p[3] - opacity).abs() < 3e-3, "{fmt:?} {opaque:?} at {x}: {p:?}");
                }
                for x in 4..8 {
                    assert_eq!(s.rgba(x, 0), before.rgba(x, 0), "{fmt:?} {opaque:?} at {x}: opaque pixels blend as before");
                }
                assert!(s.rgba(4, 0)[3] > 0.999);
            }
        }
    }

    /// #11354 review: on a CMYK target, the edge pixels a transparent-seed bucket makes
    /// opaque take the picked inks like the fully covered interior — their colour is the
    /// fill alone. A partially transparent destination the fill touches keeps the blended
    /// colour it already had, so its inks stay the conversion of that blend.
    #[test]
    fn bucket_on_transparency_writes_the_picked_cmyk_inks_at_the_edge() {
        let a = Rect::new(0, 0, 8, 1);
        let inks = [1.0f32, 0.0, 0.0, 0.0];
        let rgb = photocraft_color::convert::cmyk_to_rgb(inks).map(|c| c.clamp(0.0, 1.0));
        let src = |_, _| [rgb[0], rgb[1], rgb[2], 1.0];
        let raw = |s: &Surface, x: i32| -> Vec<f32> { s.read_region(Rect::new(x, 0, x + 1, 1)).into_iter().take(4).collect() };
        let is_inks = |got: &[f32]| got.iter().zip(inks).all(|(a, b)| (a - b).abs() < 0.02);

        // Transparent run against opaque white: x=3 is the anti-aliased edge (coverage < 1).
        let mut s = Surface::new(PixelFormat::CMYKA8);
        s.fill_rect(Rect::new(4, 0, 8, 1), &[0.0, 0.0, 0.0, 0.0, 1.0]);
        assert!(bucket_fill_inks(&mut s, a, (1, 0), 0.0, true, true, false, 1.0, None, false, src, Some(inks), FillMode::Blend(BlendMode::Normal)));
        assert!(is_inks(&raw(&s, 1)), "fully covered: {:?}", raw(&s, 1));
        assert!(is_inks(&raw(&s, 3)), "the made-opaque edge takes the picked inks: {:?}", raw(&s, 3));

        // The edge pixel holds white at half alpha instead: the fill raises its alpha,
        // but the colour is a blend, so the picked inks do not replace it.
        let mut s = Surface::new(PixelFormat::CMYKA8);
        s.fill_rect(Rect::new(3, 0, 4, 1), &[0.0, 0.0, 0.0, 0.0, 0.5]);
        s.fill_rect(Rect::new(4, 0, 8, 1), &[0.0, 0.0, 0.0, 0.0, 1.0]);
        assert!(bucket_fill_inks(&mut s, a, (1, 0), 0.0, true, true, false, 1.0, None, false, src, Some(inks), FillMode::Blend(BlendMode::Normal)));
        let edge = s.rgba(3, 0);
        assert!(edge[3] > 0.5, "the fill raised the partial pixel's alpha: {edge:?}");
        assert!(!is_inks(&raw(&s, 3)), "a blended destination keeps its own encoding: {:?}", raw(&s, 3));
    }

    /// #11354 review: the soft rim a Clear leaves rises only partly. The reference application
    /// takes rim alphas 187, 56 and 3 to 204, 211 and 253; the rule here, `α + (1 − α)²`, gives
    /// 205, 211 and 252, each within one level of it (fit in PR #11354's review round). The rim pixel
    /// sits on the bucket's anti-aliased fringe; the hole beside it fills in full and the opaque
    /// art behind it is untouched by the rule.
    #[test]
    fn bucket_on_transparency_raises_the_cleared_rim_partly() {
        let a = Rect::new(0, 0, 8, 1);
        let white = |_, _| [1.0, 1.0, 1.0, 1.0];
        let level = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as i32;
        // (rim alpha after the Clear, the reference application's result, ours)
        for (rim, reference, ours) in [(187, 204, 205), (56, 211, 211), (3, 253, 252)] {
            for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
                let mut s = Surface::new(fmt);
                // The cleared hole is [0, 0, 0, 0] (Clear zeroes its RGB), the rim straight white.
                s.fill_rect(Rect::new(4, 0, 5, 1), &[1.0, 1.0, 1.0, rim as f32 / 255.0]);
                s.fill_rect(Rect::new(5, 0, 8, 1), &[1.0, 1.0, 1.0, 1.0]);
                let before = s.clone();
                assert!(bucket_fill_inks(&mut s, a, (1, 0), 32.0, true, true, false, 1.0, None, false, white, None, FillMode::Blend(BlendMode::Normal)));
                let p = s.rgba(4, 0);
                assert_eq!(level(p[3]), ours, "{fmt:?}: rim {rim} -> {p:?}");
                assert!((ours - reference).abs() <= 1, "rim {rim}: ours {ours}, reference {reference}");
                assert!(p[..3].iter().all(|c| (c - 1.0).abs() < 1e-3), "{fmt:?}: the rim stays white: {p:?}");
                for x in 0..4 {
                    assert_eq!(level(s.rgba(x, 0)[3]), 255, "{fmt:?} rim {rim}: the hole and its edge fill in full at {x}");
                }
                for x in 5..8 {
                    assert_eq!(s.rgba(x, 0), before.rgba(x, 0), "{fmt:?} rim {rim}: opaque art at {x}");
                }
            }
        }
    }

    /// The rule is for a click on transparency only: from an opaque seed the fill is exactly the
    /// wand region's own fill, fringe included.
    #[test]
    fn bucket_on_opaque_pixels_keeps_neighbouring_transparency() {
        let a = Rect::new(0, 0, 8, 1);
        let mut s = Surface::new(PixelFormat::RGBA8);
        s.fill_rect(Rect::new(0, 0, 4, 1), &[1.0, 1.0, 1.0, 0.5]);
        s.fill_rect(Rect::new(4, 0, 8, 1), &[1.0, 1.0, 1.0, 1.0]);
        let mut plain = s.clone();
        let img = crate::selection::rgba8_image(&plain, a);
        let region = crate::selection::wand_region(&img, a, (6, 0), 0.0, true, true).unwrap();
        bucket_fill_region_inks(&mut plain, &region, 1.0, None, false, |_, _| [1.0; 4], None, FillMode::Blend(BlendMode::Normal));
        assert!(bucket_fill_inks(&mut s, a, (6, 0), 0.0, true, true, false, 1.0, None, false, |_, _| [1.0; 4], None, FillMode::Blend(BlendMode::Normal)));
        for x in 0..8 {
            assert_eq!(s.rgba(x, 0), plain.rgba(x, 0), "at {x}");
        }
        assert!((s.rgba(1, 0)[3] - 0.5).abs() < 0.01, "outside the region");
    }

    /// #12528 (reference application, transparent seed, Tolerance 32, Anti-alias on): a black
    /// edge pixel bordering the filled area takes the green fill at coverage `1 − α`, so its
    /// colour as well as its alpha follow the reference within two levels; with Anti-alias off it
    /// stays as it was.
    #[test]
    fn bucket_on_transparency_blends_into_the_edge_at_its_transparency() {
        let a = Rect::new(0, 0, 8, 1);
        let green = |_, _| [0.0, 1.0, 0.0, 1.0];
        let level = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as i32;
        // (edge alpha, the reference application's result)
        for (alpha, reference) in [(40, [0, 248, 0, 221]), (112, [0, 190, 0, 192]), (197, [0, 70, 0, 210]), (253, [0, 2, 0, 253])] {
            for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
                for anti_alias in [true, false] {
                    let mut s = Surface::new(fmt);
                    s.fill_rect(Rect::new(4, 0, 5, 1), &[0.0, 0.0, 0.0, alpha as f32 / 255.0]);
                    s.fill_rect(Rect::new(5, 0, 8, 1), &[0.0, 0.0, 0.0, 1.0]);
                    let before = s.clone();
                    assert!(bucket_fill_inks(
                        &mut s,
                        a,
                        (1, 0),
                        32.0,
                        true,
                        anti_alias,
                        false,
                        1.0,
                        None,
                        false,
                        green,
                        None,
                        FillMode::Blend(BlendMode::Normal)
                    ));
                    let got = s.rgba(4, 0).map(level);
                    if anti_alias {
                        assert!(got.iter().zip(reference).all(|(g, r)| (g - r).abs() <= 2), "{fmt:?} alpha {alpha}: {got:?}, reference {reference:?}");
                    } else {
                        assert_eq!(s.rgba(4, 0), before.rgba(4, 0), "{fmt:?} alpha {alpha}: Anti-alias off leaves the edge");
                    }
                    for x in 5..8 {
                        assert_eq!(s.rgba(x, 0), before.rgba(x, 0), "{fmt:?} alpha {alpha}: opaque art at {x}");
                    }
                }
            }
        }
    }

    /// #11354 review (measured in the reference application with a 4 px feathered rim): only the
    /// one-pixel ring 8-adjacent to the flood rises, diagonals included; pixels further out keep
    /// their alpha exactly.
    #[test]
    fn bucket_on_transparency_raises_only_the_ring_next_to_the_flood() {
        let a = Rect::new(0, 0, 7, 7);
        let mut s = Surface::new(PixelFormat::RGBA8);
        s.fill_rect(a, &[1.0, 1.0, 1.0, 100.0 / 255.0]);
        s.fill_rect(Rect::new(3, 3, 4, 4), &[0.0; 4]);
        let before = s.clone();
        assert!(bucket_fill_inks(&mut s, a, (3, 3), 32.0, true, true, false, 1.0, None, false, |_, _| [1.0; 4], None, FillMode::Blend(BlendMode::Normal)));
        for y in 0..7 {
            for x in 0..7 {
                let (p, was) = (s.rgba(x, y), before.rgba(x, y));
                match ((x - 3).abs(), (y - 3).abs()) {
                    (0, 0) => assert!(p[3] > 0.999, "the flood at ({x},{y}): {p:?}"),
                    (dx, dy) if dx <= 1 && dy <= 1 => assert!(p[3] > was[3] + 0.1, "the ring at ({x},{y}) rises: {p:?}"),
                    _ => assert_eq!(p, was, "beyond the ring at ({x},{y})"),
                }
            }
        }
    }

    /// The Paint Bucket's Mode: Multiply darkens what's there, Clear erases it (half at 50 %).
    #[test]
    fn bucket_mode_blends_or_clears() {
        let a = Rect::new(0, 0, 4, 1);
        let grey = [0.5, 0.5, 0.5, 1.0];
        for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
            let mut s = Surface::new(fmt);
            s.fill_rect(a, &grey);
            let red = |_, _| [1.0, 0.0, 0.0, 1.0];
            assert!(bucket_fill_inks(&mut s, a, (0, 0), 0.0, true, false, false, 1.0, None, false, red, None, FillMode::Blend(BlendMode::Multiply)));
            let px = s.rgba(1, 0);
            assert!((px[0] - 0.5).abs() < 0.01 && px[1] < 0.01 && (px[3] - 1.0).abs() < 1e-3, "{fmt:?} multiply: {px:?}");
            assert!(bucket_fill_inks(&mut s, a, (0, 0), 0.0, true, false, false, 0.5, None, false, red, None, FillMode::Clear));
            assert!((s.rgba(1, 0)[3] - 0.5).abs() < 0.01, "{fmt:?} clear at 50 %");
            // Locked transparency keeps the alpha, so Clear does nothing.
            assert!(bucket_fill_inks(&mut s, a, (0, 0), 0.0, true, false, false, 1.0, None, true, red, None, FillMode::Clear));
            assert!((s.rgba(1, 0)[3] - 0.5).abs() < 0.01, "{fmt:?} locked");
        }
    }
}
