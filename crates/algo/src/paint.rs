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

/// Composites `color(i)` over `area` of the surface with per-pixel coverage.
fn composite_area(
    s: &mut Surface,
    area: Rect,
    mode: FillMode,
    coverage: impl Fn(i32, i32) -> f32 + Sync,
    color: impl Fn(i32, i32) -> [f32; 4] + Sync,
    lock_transparency: bool,
    inks: Option<[f32; 4]>,
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
                let k = coverage(x, y);
                if k <= 0.0 {
                    continue;
                }
                if !any {
                    region = src.read_region(*r);
                    any = true;
                }
                let i = ((y - r.y0) as usize * w + (x - r.x0) as usize) * n;
                let px = &mut region[i..i + n];
                let d = photocraft_raster::to_rgba(&fmt, px);
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
            let t = tool_gradient_t(shape, from, to, x as f32 + 0.5, y as f32 + 0.5);
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
    color: [f32; 4],
    opacity: f32,
    selection: Option<&Surface>,
    lock_transparency: bool,
) -> bool {
    bucket_fill_src(s, area, seed, tolerance, contiguous, anti_alias, opacity, selection, lock_transparency, |_, _| color)
}

/// How far (in pixels) the soft edge of a bucket fill reaches past the pixels that match the seed.
const FRINGE: i32 = 3;

/// The region a Paint Bucket click fills: the pixels similar to the seed (as the Magic Wand finds
/// them) and, with `anti_alias`, a soft edge.
///
/// A line's anti-aliased edge is a ramp from the sheet's colour to the line's, and its outer
/// pixels are too far from the seed colour to match, so a plain flood fill leaves them as a halo
/// between the fill and the line. With `anti_alias`, every non-matching pixel within [`FRINGE`]
/// pixels of the region is covered by how much of it is the seed colour rather than the
/// strongest different colour on its ramp: `1 - d / d_max`, where `d` is its distance from the
/// seed colour and `d_max` the distance at the top of the ramp (the line). A ramp climbs in
/// strictly growing steps from the region to the line, so a half-line pixel is half filled, while
/// the line itself, a flat patch of another colour, a mid-tone line two pixels wide and the far
/// side of a thin line (whose ramp leads away from the region) are not, and a hard edge stays hard. Matching pixels keep full coverage (the Magic Wand's own edge blend would under-fill
/// them). Pixels of a similar colour that are not connected to the seed (`contiguous`) are never
/// covered.
pub fn bucket_region(img: &[[u8; 4]], area: Rect, seed: (i32, i32), tolerance: f32, contiguous: bool, anti_alias: bool) -> Option<crate::selection::Region> {
    let core = crate::selection::wand_region(img, area, seed, tolerance, contiguous, false)?;
    if !anti_alias {
        return Some(core);
    }
    let w = area.width() as usize;
    let tol = (tolerance + 1e-3).floor().max(0.0) as i32;
    let at = |x: i32, y: i32| -> Option<[u8; 4]> {
        if !area.contains(x, y) {
            return None;
        }
        img.get((y - area.y0) as usize * w + (x - area.x0) as usize).copied()
    };
    let target = at(seed.0, seed.1)?;
    // Distance from the seed colour. Against a transparent seed only opacity counts: the colour
    // under zero alpha is meaningless.
    // The Magic Wand's own rule (`wand_region`), so a pixel it left out for being a different
    // colour is never mistaken for a disconnected part of the seed's region.
    let similar = |p: [u8; 4]| (target[3] == 0 && p[3] == 0) || (0..4).all(|c| (i32::from(p[c]) - i32::from(target[c])).abs() <= tol);
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
    // Candidates: the unmatched pixels within FRINGE of the region's edge.
    let mut candidate = vec![false; mask.len()];
    for y in b.y0..b.y1 {
        for x in b.x0..b.x1 {
            if !matched(x, y) || [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)].iter().all(|&(nx, ny)| matched(nx, ny) || !area.contains(nx, ny)) {
                continue;
            }
            for ny in (y - FRINGE).max(grown.y0)..(y + FRINGE + 1).min(grown.y1) {
                for nx in (x - FRINGE).max(grown.x0)..(x + FRINGE + 1).min(grown.x1) {
                    if let Some(c) = candidate.get_mut(idx(nx, ny)) {
                        *c = true;
                    }
                }
            }
        }
    }
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
    for y in grown.y0..grown.y1 {
        for x in grown.x0..grown.x1 {
            if !candidate.get(idx(x, y)).copied().unwrap_or(false) || matched(x, y) {
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
            if coverage >= 0.04
                && let Some(m) = mask.get_mut(idx(x, y))
            {
                *m = (coverage * 255.0).round() as u8;
            }
        }
    }
    Some(crate::selection::Region { bbox: grown, mask })
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
    opacity: f32,
    selection: Option<&Surface>,
    lock_transparency: bool,
    src: impl Fn(i32, i32) -> [f32; 4] + Sync,
) -> bool {
    bucket_fill_inks(s, area, seed, tolerance, contiguous, anti_alias, opacity, selection, lock_transparency, src, None, FillMode::Blend(BlendMode::Normal))
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
    let Some(region) = bucket_region(&img, area, seed, tolerance, contiguous, anti_alias) else { return false };
    drop(img);
    bucket_fill_region_inks(s, &region, opacity, selection, lock_transparency, src, inks, mode);
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
    // Composite only over the filled region's box.
    let b = region.bbox;
    composite_area(s, b, mode, |x, y| region.at(x, y) * opacity * selection.map_or(1.0, |m| m.sample_channel(x, y, 0)), src, lock_transparency, inks);
}

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
            (0.5, 0.0),
            (10.5, 0.0),
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
        assert!(bucket_fill(&mut s, a, (1, 1), 10.0, true, false, [1.0, 0.0, 0.0, 1.0], 1.0, None, false));
        assert_eq!(s.pixel(2, 2), vec![1.0, 0.0, 0.0, 1.0]);
        assert_eq!(s.pixel(8, 2), vec![1.0, 1.0, 1.0, 1.0]);
        assert!(!bucket_fill(&mut s, a, (50, 1), 10.0, true, false, [1.0, 0.0, 0.0, 1.0], 1.0, None, false));
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
        let region = bucket_region(&img, a, (2, 2), 32.0, true, true).unwrap();
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
        bucket_fill(&mut s, a, (2, 2), 32.0, true, true, [1.0, 0.0, 0.0, 1.0], 1.0, None, false);
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
        let hard = bucket_region(&img, a, (1, 1), 32.0, true, true).unwrap();
        assert_eq!((hard.at(5, 1), hard.at(6, 1), hard.at(7, 1)), (1.0, 0.0, 0.0));
        // Anti-alias off: exactly the pixels within tolerance.
        let (s, a) = soft_line_sheet();
        let img = crate::selection::rgba8_image(&s, a);
        let off = bucket_region(&img, a, (2, 2), 32.0, true, false).unwrap();
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
        let region = bucket_region(&img, a, (2, 2), 32.0, true, true).unwrap();
        assert_eq!(region.at(2, 2), 1.0);
        assert_eq!(region.at(11, 2), 0.0, "the stroke core");
        // A stroke edge of alpha 0.6 gets 40 % fill: the fill meets the line without a gap.
        assert!((region.at(10, 2) - 0.4).abs() < 0.1, "{}", region.at(10, 2));
        assert!((region.at(9, 2) - 0.8).abs() < 0.1, "{}", region.at(9, 2));
        assert_eq!(region.at(15, 2), 0.0, "beyond the stroke is a separate region");
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
            let r = bucket_region(&img, a, (0, 1), 8.0, true, true).unwrap();
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
        let r = bucket_region(&img, a, (0, 1), 8.0, true, true).unwrap();
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
        let r = bucket_region(&img, a, (0, 1), 8.0, true, true).unwrap();
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
        for (contiguous, aa) in [(true, false), (true, true), (false, false), (false, true)] {
            let t = Instant::now();
            let r = bucket_region(&img, a, (5, 5), 32.0, contiguous, aa).unwrap();
            println!("bucket_region contiguous={contiguous} aa={aa}: {:?} ({} bytes)", t.elapsed(), r.mask.len());
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
        assert!(bucket_region(&img, a, (5, 5), 32.0, true, true).is_none());
        assert!(bucket_region(&img[..4], a, (1, 1), 32.0, true, true).is_none());
        let one = bucket_region(&img[..1], Rect::new(0, 0, 1, 1), (0, 0), 32.0, true, true).unwrap();
        assert_eq!(one.at(0, 0), 1.0);
        let all = bucket_region(&img, a, (1, 1), 0.0, false, true).unwrap();
        assert!((0..3).all(|y| (0..3).all(|x| all.at(x, y) == 1.0)));
        // NaN and absurd tolerances.
        assert!(bucket_region(&img, a, (1, 1), f32::NAN, true, true).is_some());
        assert!(bucket_region(&img, a, (1, 1), 1e30, true, true).is_some());
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
            assert!(bucket_fill_inks(&mut s, a, (0, 0), 0.0, true, false, 1.0, None, false, red, None, FillMode::Blend(BlendMode::Multiply)));
            let px = s.rgba(1, 0);
            assert!((px[0] - 0.5).abs() < 0.01 && px[1] < 0.01 && (px[3] - 1.0).abs() < 1e-3, "{fmt:?} multiply: {px:?}");
            assert!(bucket_fill_inks(&mut s, a, (0, 0), 0.0, true, false, 0.5, None, false, red, None, FillMode::Clear));
            assert!((s.rgba(1, 0)[3] - 0.5).abs() < 0.01, "{fmt:?} clear at 50 %");
            // Locked transparency keeps the alpha, so Clear does nothing.
            assert!(bucket_fill_inks(&mut s, a, (0, 0), 0.0, true, false, 1.0, None, true, red, None, FillMode::Clear));
            assert!((s.rgba(1, 0)[3] - 0.5).abs() < 0.01, "{fmt:?} locked");
        }
    }
}
