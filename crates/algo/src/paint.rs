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

/// `o`, the pixel `d` with `c` composited at coverage `k`, with what is left of `d`'s transparency
/// filled by `c` up to coverage `full` (≥ `k`) as well, behind the colour already there: the
/// transparent part of the pixel takes the fill at `full` while its colour keeps the softer `k`.
fn fill_transparency(o: [f32; 4], d: [f32; 4], c: [f32; 4], k: f32, full: f32) -> [f32; 4] {
    let add = ((full - k).max(0.0) * c[3].clamp(0.0, 1.0) * (1.0 - d[3]).max(0.0)).min((1.0 - o[3]).max(0.0));
    let a = o[3] + add;
    if add.is_nan() || add <= 0.0 || a <= 0.0 {
        return o;
    }
    let mix = |i: usize| (o[i] * o[3] + c[i] * add) / a;
    [mix(0), mix(1), mix(2), a]
}

/// Composites `color(i)` over `area` of the surface with per-pixel coverage. With `transparency`,
/// a covered pixel's transparency takes the fill at that coverage instead (see
/// [`fill_transparency`]); Lock Transparent Pixels and Clear ignore it.
#[allow(clippy::too_many_arguments)]
fn composite_area(
    s: &mut Surface,
    area: Rect,
    mode: FillMode,
    coverage: impl Fn(i32, i32) -> f32 + Sync,
    color: impl Fn(i32, i32) -> [f32; 4] + Sync,
    lock_transparency: bool,
    inks: Option<[f32; 4]>,
    transparency: Option<&(dyn Fn(i32, i32) -> f32 + Sync)>,
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
                    FillMode::Blend(m) => {
                        let c = color(x, y);
                        let o = blend::composite(m, d, c, k);
                        transparency.map_or(o, |full| fill_transparency(o, d, c, k, full(x, y)))
                    }
                    // Locked transparency keeps the alpha, so Clear leaves the pixel as it is.
                    FillMode::Clear if lock_transparency => d,
                    FillMode::Clear => [d[0], d[1], d[2], d[3] * (1.0 - k.min(1.0))],
                };
                from_rgba_into(&fmt, o, &mut enc);
                // A colour picked as CMYK inks replaces fully covered pixels of a CMYK target as it
                // is — and pixels the fill made fully opaque out of transparency, whose colour is the
                // fill alone even though their coverage is partial (the edge of a bucket fill on
                // transparency, #11354 review). A pixel that held colour keeps its blended encoding.
                if let Some(i) = inks
                    && fmt.mode == photocraft_color::ColorMode::Cmyk
                    && mode == FillMode::Blend(BlendMode::Normal)
                    && (k >= 1.0 || (d[3] <= 0.0 && o[3] >= 1.0))
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
    let Some(region) = crate::selection::wand_region(&img, area, seed, tolerance, contiguous, anti_alias) else { return false };
    // Clicked on transparency: the flood fills empty pixels, and the empty part of the pixels
    // along its anti-aliased edge (the soft rim a cleared selection leaves) is that same empty
    // area, so it is filled in full rather than left as a semi-transparent ring (#11311).
    let at_seed = usize::try_from(seed.1 - area.y0)
        .ok()
        .and_then(|y| y.checked_mul(area.width() as usize))
        .and_then(|row| row.checked_add(usize::try_from(seed.0 - area.x0).ok()?));
    let empty_seed = at_seed.and_then(|i| img.get(i)).is_some_and(|p| p[3] == 0);
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

/// [`bucket_fill_region_inks`]; with `fill_empty`, the transparent part of every pixel the region
/// touches takes the fill at the full opacity and selection coverage, while the region's
/// anti-aliasing still softens the colour over what is already there. Opaque pixels come out
/// exactly as without it.
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
    fill_empty: bool,
) {
    // Composite only over the filled region's box.
    let b = region.bbox;
    let strength = |x: i32, y: i32| opacity * selection.map_or(1.0, |m| m.sample_channel(x, y, 0));
    let empty: Option<&(dyn Fn(i32, i32) -> f32 + Sync)> = if fill_empty { Some(&strength) } else { None };
    composite_area(s, b, mode, |x, y| region.at(x, y) * strength(x, y), src, lock_transparency, inks, empty);
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
                assert!(before.rgba(3, 0)[3] < opacity * 0.97, "{fmt:?}: the old edge was semi-transparent: {:?}", before.rgba(3, 0));

                assert!(bucket_fill_inks(&mut s, a, (1, 0), 0.0, true, true, opacity, None, false, white, None, FillMode::Blend(BlendMode::Normal)));
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
        assert!(bucket_fill_inks(&mut s, a, (1, 0), 0.0, true, true, 1.0, None, false, src, Some(inks), FillMode::Blend(BlendMode::Normal)));
        assert!(is_inks(&raw(&s, 1)), "fully covered: {:?}", raw(&s, 1));
        assert!(is_inks(&raw(&s, 3)), "the made-opaque edge takes the picked inks: {:?}", raw(&s, 3));

        // The edge pixel holds white at half alpha instead: the fill raises its alpha,
        // but the colour is a blend, so the picked inks do not replace it.
        let mut s = Surface::new(PixelFormat::CMYKA8);
        s.fill_rect(Rect::new(3, 0, 4, 1), &[0.0, 0.0, 0.0, 0.0, 0.5]);
        s.fill_rect(Rect::new(4, 0, 8, 1), &[0.0, 0.0, 0.0, 0.0, 1.0]);
        assert!(bucket_fill_inks(&mut s, a, (1, 0), 0.0, true, true, 1.0, None, false, src, Some(inks), FillMode::Blend(BlendMode::Normal)));
        let edge = s.rgba(3, 0);
        assert!(edge[3] > 0.5, "the fill raised the partial pixel's alpha: {edge:?}");
        assert!(!is_inks(&raw(&s, 3)), "a blended destination keeps its own encoding: {:?}", raw(&s, 3));
    }

    /// The rule is for a click on transparency only: from an opaque seed, a semi-transparent
    /// neighbour along the edge keeps its own transparency (it belongs to another area).
    #[test]
    fn bucket_on_opaque_pixels_keeps_neighbouring_transparency() {
        let a = Rect::new(0, 0, 8, 1);
        let mut s = Surface::new(PixelFormat::RGBA8);
        s.fill_rect(Rect::new(0, 0, 4, 1), &[1.0, 1.0, 1.0, 0.5]);
        s.fill_rect(Rect::new(4, 0, 8, 1), &[1.0, 1.0, 1.0, 1.0]);
        assert!(bucket_fill_inks(&mut s, a, (6, 0), 0.0, true, true, 1.0, None, false, |_, _| [1.0; 4], None, FillMode::Blend(BlendMode::Normal)));
        let edge = s.rgba(3, 0)[3];
        assert!(edge > 0.5 && edge < 0.6, "only the anti-aliased coverage lands: {edge}");
        assert!((s.rgba(1, 0)[3] - 0.5).abs() < 0.01, "outside the region");
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
