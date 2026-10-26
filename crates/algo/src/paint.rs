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
    tool_gradient_pos(shape, from, to, x, y).0
}

/// [`tool_gradient_t`] together with the pixel's distance from the gradient's centre (pixels)
/// and the square pixel's full-width projection across the drag's direction (|ux| + |uy|, 1 for
/// an axis-aligned drag): the sweep of an Angle gradient (like a Radial's rings) is centred on
/// `from`, and its seam runs along the drag.
fn tool_gradient_pos(shape: GradientShape, from: (f32, f32), to: (f32, f32), x: f32, y: f32) -> (f32, f32, f32) {
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
    (t.clamp(0.0, 1.0), (px * px + py * py).sqrt(), ux.abs() + uy.abs())
}

/// [`tool_gradient_t`] with an Angle gradient's 0/1 wrap seam anti-aliased as the compositor's
/// `effects::gradient_seam` (the engine's live-gradient parity tests keep the two in step): the
/// ramp position `t`, the position past the seam the pixel also covers (`t` itself when `weight`
/// is 0) and `weight`, the fraction of the pixel beyond the seam. The band is half the pixel's
/// extent across the seam (its square projection on the seam's normal), as a fraction of the
/// sweep at the pixel's distance `r` from the centre.
pub fn tool_gradient_seam(shape: GradientShape, from: (f32, f32), to: (f32, f32), x: f32, y: f32) -> (f32, f32, f32) {
    let (t, r, across) = tool_gradient_pos(shape, from, to, x, y);
    // Capped at half the ramp, for pixels at the sweep's centre where the arc is unbounded.
    let half = (0.5 * across / (std::f32::consts::TAU * r.max(1e-6))).min(0.5);
    let d = t.min(1.0 - t);
    let (wrap, weight) = if shape == GradientShape::Angle && d < half { (if t >= 0.5 { t - 1.0 } else { t + 1.0 }, 0.5 - 0.5 * (d / half)) } else { (t, 0.0) };
    (t, wrap, weight)
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
            let (t, wrap, seam) = tool_gradient_seam(shape, from, to, x as f32 + 0.5, y as f32 + 0.5);
            let (t, wrap) = if reverse { (1.0 - t, 1.0 - wrap) } else { (t, wrap) };
            let mut c = sample_stops(stops, t);
            if seam > 0.0 {
                // The seam of an Angle gradient's sweep is anti-aliased like the compositor's
                // (the engine's live-gradient parity tests keep the two in step): the weight is
                // area coverage, so the two sides mix premultiplied by alpha and convert back
                // to straight (effects::seam_mix).
                let past = sample_stops(stops, wrap);
                let pm = |mut v: [f32; 4]| {
                    let alpha = v[3];
                    v[..3].iter_mut().for_each(|ch| *ch *= alpha);
                    v
                };
                let (a, b) = (pm(c), pm(past));
                let mut m = std::array::from_fn(|i| a[i] + (b[i] - a[i]) * seam);
                if m[3] > 0.0 {
                    let alpha = m[3];
                    m[..3].iter_mut().for_each(|ch| *ch /= alpha);
                }
                c = m;
            }
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
    fn angle_gradient_seam_straddle_fades_over_half_a_pixels_arc() {
        // A drag along +x: the sweep starts on the +x ray from (16, 16), so the seam lies on it.
        let seam = |x, y| tool_gradient_seam(GradientShape::Angle, (16.0, 16.0), (26.0, 16.0), x, y);
        // Just past the seam the pixel covers a little of the ramp before it (its wrapped
        // position), by the fraction of itself beyond the seam.
        let (t, wrap, w) = seam(20.0, 16.1);
        assert!(t > 0.0 && t < 0.01 && wrap > 1.0 && w > 0.0 && w < 0.5, "{t} {wrap} {w}");
        // The other side of the seam straddles by the same weight, wrapped the mirrored way.
        let (bt, bwrap, bw) = seam(20.0, 15.9);
        assert!((bt - (1.0 - t)).abs() < 1e-6 && (bwrap + t).abs() < 1e-6 && (bw - w).abs() < 1e-6, "{bt} {bwrap} {bw}");
        // Centred on the seam the two sides weigh the same; an arc away it does not straddle.
        let (t, wrap, w) = seam(20.0, 16.0);
        assert!(t.abs() < 1e-6 && (wrap - 1.0).abs() < 1e-6 && (w - 0.5).abs() < 1e-6, "{t} {wrap} {w}");
        let (_, _, w) = seam(20.0, 17.0);
        assert_eq!(w, 0.0);
        // Other styles have no seam: their ramp ends stay hard.
        let (t, wrap, w) = tool_gradient_seam(GradientShape::Linear, (16.0, 16.0), (26.0, 16.0), 15.5, 16.0);
        assert_eq!(w, 0.0);
        assert!((wrap - tool_gradient_t(GradientShape::Linear, (16.0, 16.0), (26.0, 16.0), 15.5, 16.0)).abs() < 1e-6, "{t} {wrap}");
    }

    /// The painted gradient anti-aliases its seam like the compositor's Angle fills (#4933, which
    /// the engine's live-gradient parity tests pin): pixels on the wrap mix the ramp's ends, and
    /// every other pixel stays exactly on the plain ramp.
    #[test]
    fn angle_gradient_paint_seam_is_anti_aliased() {
        let stops = &[(0.0, [0.0, 0.0, 0.0, 1.0]), (1.0, [1.0, 1.0, 1.0, 1.0])];
        let mut s = Surface::new(PixelFormat::RGBA8);
        // Centre the sweep at (8, 7.5): the seam (the +x ray) runs through row 7's pixel centres.
        paint_gradient(
            &mut s,
            Rect::new(0, 0, 17, 16),
            (8.0, 7.5),
            (18.0, 7.5),
            GradientShape::Angle,
            stops,
            false,
            1.0,
            BlendMode::Normal,
            false,
            None,
            false,
        );
        let at = |x: i32, y: i32| s.pixel(x, y)[0];
        for x in 8..17 {
            // On the seam the two sides weigh the same (before the fix: the start value, 0).
            assert!((at(x, 7) - 0.5).abs() < 0.51 / 255.0, "seam pixel ({x},7): {}", at(x, 7));
            // No adjacent pair measures the full ramp (before the fix: up to 0.96 across it).
            let step = (at(x, 6) - at(x, 7)).abs().max((at(x, 7) - at(x, 8)).abs());
            assert!(step < 0.55, "step at ({x},7): {step}");
        }
        // Away from the seam the paint is unchanged: exactly the ramp at the pixel's position.
        for y in 0..16 {
            for x in 0..17 {
                if y == 7 && x >= 8 {
                    continue;
                }
                let t = tool_gradient_t(GradientShape::Angle, (8.0, 7.5), (18.0, 7.5), x as f32 + 0.5, y as f32 + 0.5);
                assert!((at(x, y) - t).abs() < 0.51 / 255.0, "({x},{y}) left the plain ramp: {} vs {t}", at(x, y));
            }
        }
    }

    #[test]
    fn angle_gradient_seam_band_follows_the_pixels_diagonal_projection() {
        // A square pixel reaches |ux| + |uy| = √2 half-pixels across a 45° drag's seam, so
        // centres up to 0.707 px clear of it still straddle; an axis-aligned drag's band stays
        // half a pixel. As the compositor's gradient_seam (the parity tests keep them in step).
        let (s, c) = (std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2);
        // Centres `h` px off the 45° seam through (16, 16), 50 px along it.
        let at = |h: f32| {
            let (x, y) = (16.0 + 50.0 * c + h * s, 16.0 - 50.0 * c + h * s);
            tool_gradient_seam(GradientShape::Angle, (16.0, 16.0), (16.0 + c, 16.0 - c), x, y).2
        };
        assert!((at(0.0) - 0.5).abs() < 1e-4, "{}", at(0.0));
        assert!(at(0.6) > 0.0 && at(0.6) < 0.5, "{}", at(0.6));
        assert_eq!(at(0.8), 0.0);
        // The axis-aligned drag keeps its half-pixel band.
        let axis = |h: f32| tool_gradient_seam(GradientShape::Angle, (16.0, 16.0), (26.0, 16.0), 66.0, 16.0 + h).2;
        assert!(axis(0.4) > 0.0, "{}", axis(0.4));
        assert_eq!(axis(0.6), 0.0);
    }

    /// A seam pixel between the ramp's transparent and opaque sides stores the opaque colour at
    /// half alpha (the seam weight is area coverage, mixed premultiplied), so it composites to
    /// half its colour — straight mixing would store a half-intensity colour compositing to a
    /// quarter.
    #[test]
    fn angle_gradient_paint_seam_mixes_premultiplied() {
        let stops = &[(0.0, [0.0, 0.0, 0.0, 0.0]), (1.0, [1.0, 1.0, 1.0, 1.0])];
        let mut s = Surface::new(PixelFormat::RGBA8);
        paint_gradient(
            &mut s,
            Rect::new(0, 0, 17, 16),
            (8.0, 7.5),
            (18.0, 7.5),
            GradientShape::Angle,
            stops,
            false,
            1.0,
            BlendMode::Normal,
            false,
            None,
            false,
        );
        for x in 8..17 {
            let p = s.pixel(x, 7);
            assert!((p[3] - 0.5).abs() < 0.51 / 255.0 && p[0] > 0.99, "seam pixel ({x},7): {p:?}");
        }
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
