//! Frozen scalar rasterizer used only as a pixel oracle for the computed-tip fast path and the
//! sampled-tip lane kernel.
use super::*;
use crate::tile::GrayTile;
use crate::{soft_exp, tip_reach, tip_shape};
use photocraft_geom::Point;

impl BrushContext {
    fn rasterize_reference(&self, d: &Dab, dual: bool, rect: Rect, out: &mut Vec<f32>, mut depth: Option<&mut Vec<f32>>) {
        let (w, h) = (rect.width() as usize, rect.height() as usize);
        out.clear();
        out.resize(w * h, 0.0);
        if let Some(dp) = depth.as_deref_mut() {
            dp.clear();
            dp.resize(w * h, 0.0);
        }
        let b = &self.brush;
        let (hardness, mips) = if dual { (b.dual_brush.hardness, self.dual_tip.as_deref()) } else { (b.hardness, self.tip.as_deref()) };
        let aliased = b.aliased && !dual;
        let wet = b.wet_edges && !dual && depth.is_none();
        let noise = b.noise && !dual;
        let tex_tip = !dual && b.texture.enabled && b.texture.each_tip && self.texture.is_some();
        let (cx, cy) = if aliased { grid_center(d.center.x, d.center.y, 2.0 * d.radius) } else { (d.center.x as f32, d.center.y as f32) };
        let (sn, cs) = d.angle.sin_cos();
        let (fx, fy) = (if d.flip_x { -1.0 } else { 1.0 }, if d.flip_y { -1.0 } else { 1.0 });
        // Brush Projection: stretch the sampling coordinates along the tilt direction, which
        // foreshortens the tip there by `proj_scale`.
        let proj = (!dual && d.proj_scale < 0.999).then(|| {
            let (ps, pc) = d.proj_angle.sin_cos();
            (pc, ps, 1.0 / d.proj_scale.max(0.05) - 1.0)
        });
        let r = d.radius;
        let ro = d.roundness.max(0.5 / r).min(1.0);
        let rm = (r * ro).max(0.5);
        let reach = if aliased || mips.is_some() { rm } else { crate::tip_reach(rm, hardness).max(rm) };
        let reach2 = (reach + 0.5) * (reach + 0.5);
        // Sampled-tip mapping.
        let (level, inv_scale, tw, th) = match mips {
            Some(m) => {
                let (w0, h0, _) = &m.levels[0];
                let big = *w0.max(h0) as f32;
                (m.level_for(2.0 * r * ro.max(0.25)), big / (2.0 * r), *w0 as f32, *h0 as f32)
            }
            None => (0, 1.0, 1.0, 1.0),
        };
        // A pixel offset from the dab centre in tip space: to y-up, project, rotate by -angle, flip.
        let to_tip = |dx: f32, dy: f32| {
            let (mut ux, mut uy) = (dx, -dy);
            if let Some((ax, ay, k)) = proj {
                let t = (ux * ax + uy * ay) * k;
                ux += t * ax;
                uy += t * ay;
            }
            ((ux * cs + uy * sn) * fx, (-ux * sn + uy * cs) * fy)
        };
        // A tip a few pixels across takes each pixel's covered area (a 4×4 grid inside it):
        // sampling only the pixel centre made a tiny dab's total coverage swing with its sub-pixel
        // position, and thin lines beaded.
        let small = mips.is_none() && !aliased && rm < SMALL_TIP_RADIUS;
        let small_reach2 = (reach + 0.75) * (reach + 0.75);
        for yy in 0..h {
            let y = rect.y0 + yy as i32;
            let dy = y as f32 + 0.5 - cy;
            for xx in 0..w {
                let x = rect.x0 + xx as i32;
                let dx = x as f32 + 0.5 - cx;
                let (u, v) = to_tip(dx, dy);
                let (mut val, rn) = match mips {
                    // Block: the whole-pixel square, hard (the Eraser's Block mode).
                    _ if aliased && b.square => {
                        if dx.abs() >= r || dy.abs() >= r {
                            continue;
                        }
                        (1.0, 0.0)
                    }
                    None if small => {
                        let d2 = (u * ro).powi(2) + v * v;
                        if d2 >= small_reach2 {
                            continue;
                        }
                        let mut sum = 0.0;
                        for sy in SUBPIXEL {
                            for sx in SUBPIXEL {
                                let (su, sv) = to_tip(dx + sx, dy + sy);
                                let sd = ((su * ro).powi(2) + sv * sv).sqrt();
                                sum += tip_falloff(sd, rm, hardness);
                            }
                        }
                        (sum / (SUBPIXEL.len() * SUBPIXEL.len()) as f32, d2.sqrt() / rm)
                    }
                    None => {
                        let d2 = (u * ro).powi(2) + v * v;
                        if d2 >= reach2 {
                            continue;
                        }
                        let dd = d2.sqrt();
                        let val = if aliased { if dd <= rm { 1.0 } else { 0.0 } } else { dab_coverage(dd, rm, hardness) };
                        (val, dd / rm)
                    }
                    Some(m) => {
                        let (px, py) = (u * inv_scale + tw / 2.0, -(v / ro) * inv_scale + th / 2.0);
                        if px < -1.0 || py < -1.0 || px > tw + 1.0 || py > th + 1.0 {
                            continue;
                        }
                        let s = m.sample_clamped(level, px / tw, py / th);
                        let val = if aliased { if s >= 0.5 { 1.0 } else { 0.0 } } else { s };
                        (val, 1.0 - s)
                    }
                };
                if val <= 0.0 {
                    continue;
                }
                if noise && val < 1.0 {
                    let n = hash2(x, y, NOISE_SALT);
                    let hard = if n < val { 1.0 } else { 0.0 };
                    val += (hard - val) * 0.7;
                }
                if tex_tip {
                    val = mask_combine(b.texture.mode, val, self.texture_at(x, y), d.depth);
                }
                if wet {
                    val *= wet_factor(rn);
                }
                if let Some(slot) = depth.as_deref_mut().and_then(|dp| dp.get_mut(yy * w + xx)) {
                    *slot = (1.0 - rn).clamp(0.0, 1.0);
                }
                out[yy * w + xx] = val * d.alpha;
            }
        }
    }
}

fn compare(ctx: &BrushContext, dab: &Dab, dual: bool, rect: Rect, with_depth: bool) {
    // Reused, dirty buffers also check clearing and resizing.
    let (mut actual, mut expected) = (vec![0.75; 19], vec![0.75; 19]);
    let (mut actual_depth, mut expected_depth) = (vec![0.25; 23], vec![0.25; 23]);
    ctx.rasterize_into(dab, dual, rect, &mut actual, with_depth.then_some(&mut actual_depth));
    ctx.rasterize_reference(dab, dual, rect, &mut expected, with_depth.then_some(&mut expected_depth));
    for (label, a, b) in [("coverage", actual, expected), ("depth", actual_depth, expected_depth)] {
        assert_eq!(a.len(), b.len());
        let mismatch = a.iter().zip(&b).enumerate().find(|(_, (a, b))| a.to_bits() != b.to_bits());
        assert!(mismatch.is_none(), "{label} mismatch {mismatch:?}; {dab:?}; dual={dual}, rect={rect:?}");
    }
}

#[test]
fn computed_tips_match_scalar_bits() {
    for aliased in [false, true] {
        for hardness in [-0.5, 0.0, 0.1, 0.5, 0.9999, 1.0, 1.5] {
            let ctx = BrushContext::new(&BrushSettings { aliased, hardness, ..Default::default() });
            for radius in [0.5, 1.0, 2.99, 3.0, 3.01, 15.0, 31.5] {
                for roundness in [0.0, 0.1, 0.4, 1.0] {
                    for alpha in [-0.5, -0.0, 0.0, 0.37, 1.0] {
                        let mut d = Dab::round(Point::new(-31.25, 63.75), radius, alpha);
                        d.roundness = roundness;
                        d.flip_x = true;
                        d.flip_y = true;
                        compare(&ctx, &d, false, ctx.dab_rect(&d, false), false);
                    }
                }
            }
        }
    }
}

#[test]
fn circular_tips_ignore_finite_rotation() {
    for aliased in [false, true] {
        for hardness in [0.0, 0.5, 1.0] {
            let ctx = BrushContext::new(&BrushSettings { aliased, hardness, ..Default::default() });
            for radius in [3.0, 15.25, 150.0] {
                for roundness in [1.0, 1.5] {
                    let mut d = Dab::round(Point::new(-31.25, 63.75), radius, 0.37);
                    d.roundness = roundness;
                    let rect = ctx.dab_rect(&d, false);
                    let expected: Vec<_> = [false, true]
                        .into_iter()
                        .map(|dual| {
                            let mut coverage = Vec::new();
                            ctx.rasterize_reference(&d, dual, rect, &mut coverage, None);
                            coverage
                        })
                        .collect();
                    for angle in [-2.3, 0.7, std::f32::consts::FRAC_PI_2, std::f32::consts::PI] {
                        for (flip_x, flip_y) in [(false, false), (true, false), (false, true), (true, true)] {
                            d.angle = angle;
                            d.flip_x = flip_x;
                            d.flip_y = flip_y;
                            for (dual, expected) in [false, true].into_iter().zip(&expected) {
                                let mut actual = vec![0.75; 19];
                                ctx.rasterize(&d, dual, rect, &mut actual);
                                assert_eq!(actual.len(), expected.len());
                                let mismatch = actual.iter().zip(expected).enumerate().find(|(_, (a, b))| a.to_bits() != b.to_bits());
                                assert!(
                                    mismatch.is_none(),
                                    "circle changed with rotation: {mismatch:?}; {d:?}, aliased={aliased}, hardness={hardness}, dual={dual}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn rotated_ellipses_and_projected_circles_match_scalar_bits() {
    for aliased in [false, true] {
        let ctx = BrushContext::new(&BrushSettings { aliased, hardness: 0.5, ..Default::default() });
        for (roundness, proj_scale) in [(0.4, 1.0), (1.0, 0.6)] {
            for angle in [0.0, 0.7, -2.3] {
                let mut d = Dab::round(Point::new(-31.25, 63.75), 15.25, 0.37);
                d.roundness = roundness;
                d.angle = angle;
                d.proj_scale = proj_scale;
                d.proj_angle = 0.3;
                compare(&ctx, &d, false, ctx.dab_rect(&d, false), false);
            }
        }
    }
}

#[test]
fn large_corner_dabs_keep_off_canvas_coverage() {
    for (hardness, aliased) in [(0.0, false), (1.0, false), (1.0, true)] {
        let ctx = BrushContext::new(&BrushSettings { hardness, aliased, ..Default::default() });
        let d = Dab::round(Point::new(-1050.25, -1050.75), 1500.0, 0.73);
        let rect = ctx.dab_rect(&d, false);
        compare(&ctx, &d, false, rect, false);
        let mut coverage = Vec::new();
        ctx.rasterize(&d, false, rect, &mut coverage);
        // The layer is an infinite plane: paint outside the visible document must survive.
        assert!(coverage.iter().enumerate().any(|(i, v)| *v > 0.0 && rect.x0 + ((i % rect.width() as usize) as i32) < 0));
        compare(&ctx, &d, false, Rect::new(0, 0, 32, 32), false);
        compare(&ctx, &d, false, Rect::new(0, 0, 0, 0), false);
    }
}

#[test]
fn feature_and_exceptional_dabs_match_scalar_bits() {
    let tip = GrayTile::from_fn(8, 5, |x, y| ((x + y) % 7) as f32 / 6.0);
    for feature in 0..7 {
        let mut b = BrushSettings { hardness: 0.25, ..Default::default() };
        match feature {
            0 => b.noise = true,
            1 => b.wet_edges = true,
            2 => {
                b.aliased = true;
                b.square = true;
            }
            3 => b.tip = TipShape::Sampled(tip.clone()),
            4 => {
                b.texture.enabled = true;
                b.texture.each_tip = true;
                b.texture.pattern = Pattern::Tile(tip.clone());
            }
            5 => b.dual_brush.tip = TipShape::Sampled(tip.clone()),
            _ => {}
        }
        let ctx = BrushContext::new(&b);
        for dual in [false, true] {
            for with_depth in [false, true] {
                for (angle, proj_scale) in [(0.0, 1.0), (0.7, 1.0), (0.0, 0.6)] {
                    let mut d = Dab::round(Point::new(63.5, -0.25), 15.0, 0.47);
                    d.angle = angle;
                    d.roundness = 0.4;
                    d.proj_scale = proj_scale;
                    compare(&ctx, &d, dual, ctx.dab_rect(&d, dual), with_depth);
                }
            }
        }
    }
    let ctx = BrushContext::new(&BrushSettings::default());
    for value in [f32::NAN, f32::from_bits(0x7f80_0123), f32::INFINITY, f32::NEG_INFINITY] {
        for field in 0..4 {
            let mut d = Dab::round(Point::new(0.0, 0.0), 15.0, 1.0);
            match field {
                0 => d.alpha = value,
                1 => d.angle = value,
                2 => d.radius = value,
                _ => d.roundness = value,
            }
            compare(&ctx, &d, false, Rect::new(-4, -4, 4, 4), false);
        }
        let ctx = BrushContext::new(&BrushSettings { hardness: value, ..Default::default() });
        compare(&ctx, &Dab::round(Point::new(0.0, 0.0), 15.0, 1.0), false, Rect::new(-16, -16, 16, 16), false);
    }
}

#[test]
fn computed_tips_match_at_pixel_and_float_boundaries() {
    for aliased in [false, true] {
        for center in [Point::new(0.0, -0.0), Point::new(0.5, 0.5), Point::new(255.9999, -256.0001), Point::new(16_777_216.25, -16_777_216.75)] {
            for radius in [3.0, 3.5, 15.25, 113.75] {
                for hardness in [0.0, 0.6, 1.0] {
                    let ctx = BrushContext::new(&BrushSettings { hardness, aliased, ..Default::default() });
                    let mut d = Dab::round(center, radius, 0.137);
                    d.roundness = 0.375;
                    compare(&ctx, &d, false, ctx.dab_rect(&d, false), false);
                }
            }
        }
    }
}

#[test]
fn cached_profiles_match_scalar_for_small_and_transformed_dabs() {
    for hardness in [-0.5, 0.0, 0.035, 0.37, 0.95, 1.0, 1.5, f32::NAN, f32::INFINITY] {
        let ctx = BrushContext::new(&BrushSettings { hardness, ..Default::default() });
        for radius in [0.5, 1.25, 2.99, 3.0, 7.5, 100.0] {
            for dual in [false, true] {
                for with_depth in [false, true] {
                    for angle in [0.0, 0.7] {
                        let mut d = Dab::round(Point::new(4.25, -3.75), radius, 0.73);
                        d.angle = angle;
                        d.roundness = 0.6;
                        compare(&ctx, &d, dual, ctx.dab_rect(&d, dual), with_depth);
                    }
                }
            }
        }
    }
}

// Keep the pre-cache scalar coverage equations independent of TipProfile.
fn tip_falloff(d: f32, radius: f32, hardness: f32) -> f32 {
    let (r0, s) = tip_shape(radius, hardness);
    if d <= r0 {
        return 1.0;
    }
    let x = (d - r0) / s;
    soft_exp(x * x)
}

fn dab_coverage(d: f32, radius: f32, hardness: f32) -> f32 {
    // NaN distances paint nothing.
    if d.is_nan() || d >= tip_reach(radius, hardness) + 0.5 {
        return 0.0;
    }
    tip_falloff(d, radius, hardness)
}

#[test]
fn cached_profile_preserves_public_coverage_at_boundaries() {
    for radius in [-1.0, 0.0, 0.5, 3.0, 100.0, f32::NAN, f32::INFINITY] {
        for hardness in [-1.0, 0.0, 0.035, 0.37, 1.0, 2.0, f32::NAN, f32::INFINITY] {
            let profile = TipProfile::new(radius, hardness);
            let (core, shoulder) = tip_shape(radius, hardness);
            for d in [-1.0, -0.0, core, core + shoulder, profile.reach(), profile.reach() + 0.5, f32::INFINITY, f32::NEG_INFINITY, f32::NAN] {
                let expected = dab_coverage(d, radius, hardness);
                assert_eq!(profile.coverage(d).to_bits(), expected.to_bits());
                assert_eq!(crate::dab_coverage(d, radius, hardness).to_bits(), expected.to_bits());
                let expected = tip_falloff(d, radius, hardness);
                let actual = crate::tip_falloff(d, radius, hardness);
                assert!(actual.to_bits() == expected.to_bits() || (actual.is_nan() && expected.is_nan()));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Sampled tips: the lane kernel (`SampledTip`) against the frozen scalar loop
// ---------------------------------------------------------------------------

/// [`compare`] for sampled tips: the same bits, except that a NaN only has to stay NaN (the lane
/// kernel may pass on another NaN payload).
fn compare_sampled(ctx: &BrushContext, dab: &Dab, dual: bool, rect: Rect, with_depth: bool) {
    let (mut actual, mut expected) = (vec![0.75; 19], vec![0.75; 19]);
    let (mut actual_depth, mut expected_depth) = (vec![0.25; 23], vec![0.25; 23]);
    ctx.rasterize_into(dab, dual, rect, &mut actual, with_depth.then_some(&mut actual_depth));
    ctx.rasterize_reference(dab, dual, rect, &mut expected, with_depth.then_some(&mut expected_depth));
    for (label, a, b) in [("coverage", actual, expected), ("depth", actual_depth, expected_depth)] {
        assert_eq!(a.len(), b.len(), "{label}");
        let mismatch = a.iter().zip(&b).enumerate().find(|(_, (a, b))| a.to_bits() != b.to_bits() && !(a.is_nan() && b.is_nan()));
        assert!(mismatch.is_none(), "{label} mismatch {mismatch:?}; {dab:?}; dual={dual}, rect={rect:?}");
    }
}

/// A sampled tip with structure everywhere: a grain of hashed values, zeros, exact halves (the
/// aliased cut) and ones, so a misplaced texel or a swapped lerp changes the coverage.
fn grain(w: u32, h: u32) -> GrayTile {
    GrayTile::from_fn(w, h, |x, y| match (x.wrapping_mul(2_654_435_761) ^ y.wrapping_mul(40_503)) % 7 {
        0 => 0.0,
        1 => 1.0,
        2 => 0.5,
        k => k as f32 / 7.0,
    })
}

fn sampled(tip: &GrayTile) -> BrushSettings {
    BrushSettings { tip: TipShape::Sampled(tip.clone()), ..Default::default() }
}

/// Tip bitmaps from a single texel to non-square and odd sizes, drawn from far below to far above
/// their resolution (every mip level, magnified and minified), rotated, flipped, squashed and
/// projected, aliased or not, at any flow: every coverage value has the scalar loop's bits.
#[test]
fn sampled_tips_match_scalar_bits() {
    for (w, h) in [(1, 1), (2, 1), (1, 7), (8, 5), (33, 64), (257, 129)] {
        let tip = grain(w, h);
        for aliased in [false, true] {
            let ctx = BrushContext::new(&BrushSettings { aliased, ..sampled(&tip) });
            for radius in [0.25, 0.5, 1.0, 2.9, 7.5, 40.25, 150.0] {
                for (angle, flip) in [(0.0, false), (0.3, true), (std::f32::consts::FRAC_PI_2, false), (-2.1, true)] {
                    for (roundness, proj_scale) in [(1.0, 1.0), (0.4, 1.0), (1.0, 0.6), (0.01, 0.05)] {
                        let mut d = Dab::round(Point::new(17.3, -9.6), radius, 0.73);
                        (d.angle, d.flip_x, d.flip_y, d.roundness, d.proj_scale, d.proj_angle) = (angle, flip, !flip, roundness, proj_scale, 0.9);
                        compare_sampled(&ctx, &d, false, ctx.dab_rect(&d, false), false);
                    }
                }
            }
            // Flow and its signs: zero, negative zero, negative, full.
            for alpha in [0.0, -0.0, -0.5, 1.0] {
                let d = Dab::round(Point::new(3.5, 3.5), 9.0, alpha);
                compare_sampled(&ctx, &d, false, ctx.dab_rect(&d, false), false);
            }
        }
    }
}

/// Rows of every width (whole blocks, tails, both, a single pixel), windows cutting through the
/// dab on any side, and dab centres on pixel, block and tile boundaries and far from the origin.
#[test]
fn sampled_tip_rows_windows_and_boundaries_match_scalar_bits() {
    let tip = grain(40, 24);
    let ctx = BrushContext::new(&sampled(&tip));
    for center in [Point::new(0.0, 0.0), Point::new(0.5, -0.5), Point::new(63.999, 64.0), Point::new(-255.5, 256.25), Point::new(16_777_216.25, -16_777_216.75)]
    {
        let mut d = Dab::round(center, 12.0, 0.6);
        d.angle = 0.5;
        compare_sampled(&ctx, &d, false, ctx.dab_rect(&d, false), false);
        let (cx, cy) = (center.x as i32, center.y as i32);
        for width in [1, 2, 7, 8, 9, 15, 16, 17, 31] {
            for (ox, oy) in [(-20, -3), (-4, 0), (5, 2)] {
                compare_sampled(&ctx, &d, false, Rect::new(cx + ox, cy + oy, cx + ox + width, cy + oy + 3), false);
            }
        }
        compare_sampled(&ctx, &d, false, Rect::new(cx, cy, cx, cy + 4), false);
    }
}

/// The largest brush (a 5000 px sampled tip covers ~50 MP): windows at its centre, its rim and
/// past its corner.
#[test]
fn huge_sampled_dabs_match_scalar_bits() {
    let tip = grain(64, 48);
    for aliased in [false, true] {
        let ctx = BrushContext::new(&BrushSettings { aliased, ..sampled(&tip) });
        let mut d = Dab::round(Point::new(-1000.25, 2000.75), crate::MAX_BRUSH_SIZE / 2.0, 0.81);
        d.angle = 0.2;
        for (x, y) in [(-1040, 1980), (-1000 + 2480, 2000), (-1000 - 3600, 2000 - 3600), (-1000 + 1700, 2000 + 1700)] {
            compare_sampled(&ctx, &d, false, Rect::new(x, y, x + 77, y + 33), false);
        }
    }
}

/// Noise, per-tip Texture, Wet Edges (with and without the stroke's depth) and the Dual Brush's
/// sampled tip take the kernel's per-pixel path: the scalar loop's bits too.
#[test]
fn sampled_tip_features_and_dual_tips_match_scalar_bits() {
    let tip = grain(30, 22);
    let pattern = grain(9, 13);
    for feature in 0..5 {
        let mut b = BrushSettings { hardness: 0.25, ..sampled(&tip) };
        match feature {
            0 => b.noise = true,
            1 => b.wet_edges = true,
            2 => {
                b.texture = crate::brush::Texture {
                    enabled: true,
                    each_tip: true,
                    mode: MaskMode::Multiply,
                    depth: 0.8,
                    scale: 0.7,
                    pattern: Pattern::Tile(pattern.clone()),
                    ..Default::default()
                };
            }
            3 => {
                b.noise = true;
                b.wet_edges = true;
                b.aliased = true;
            }
            _ => {
                b.dual_brush.enabled = true;
                b.dual_brush.tip = TipShape::Sampled(pattern.clone());
            }
        }
        let ctx = BrushContext::new(&b);
        for dual in [false, true] {
            for with_depth in [false, true] {
                for (angle, proj_scale, radius) in [(0.0, 1.0, 15.0), (0.7, 1.0, 4.5), (0.0, 0.6, 31.0), (-1.3, 0.3, 60.0)] {
                    let mut d = Dab::round(Point::new(63.5, -0.25), radius, 0.47);
                    (d.angle, d.roundness, d.proj_scale, d.depth) = (angle, 0.4, proj_scale, 0.6);
                    compare_sampled(&ctx, &d, dual, ctx.dab_rect(&d, dual), with_depth);
                }
            }
        }
    }
}

/// NaN and infinite dab fields (from a direct caller; the generator never makes them) paint
/// what the scalar loop paints, and never panic.
#[test]
fn exceptional_sampled_dabs_match_scalar_bits() {
    let tip = grain(16, 16);
    let ctx = BrushContext::new(&sampled(&tip));
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, f32::MAX, -f32::MAX, f32::MIN_POSITIVE, 0.0] {
        for field in 0..6 {
            let mut d = Dab::round(Point::new(4.0, 4.0), 6.0, 1.0);
            match field {
                0 => d.alpha = value,
                1 => d.angle = value,
                2 => d.radius = value,
                3 => d.roundness = value,
                4 => d.proj_scale = value,
                _ => d.proj_angle = value,
            }
            compare_sampled(&ctx, &d, false, Rect::new(-8, -8, 17, 16), false);
            compare_sampled(&ctx, &d, false, Rect::new(-8, -8, 17, 16), true);
        }
    }
    for c in [Point::new(f64::NAN, 3.0), Point::new(3.0, f64::INFINITY), Point::new(1e30, -1e30)] {
        compare_sampled(&ctx, &Dab::round(c, 6.0, 1.0), false, Rect::new(-8, -8, 17, 16), false);
    }
    // A negative radius (and roundness) turns the tip mapping's scale negative: the column
    // bound must still hold, near the origin and far out, where f32 rounds the pixel grid.
    for (x, y) in [(4.0, 4.0), (16_777_216.25, -16_777_216.75), (-3.0e7, 1.2e7)] {
        for radius in [-0.25, -6.0, -150.0] {
            for (roundness, angle) in [(1.0, 0.0), (0.3, 0.0), (-0.4, 0.0), (1.0, 0.4), (-0.4, 0.4)] {
                let mut d = Dab::round(Point::new(x, y), radius, 0.9);
                (d.roundness, d.angle) = (roundness, angle);
                let (cx, cy) = (x as i32, y as i32);
                for with_depth in [false, true] {
                    compare_sampled(&ctx, &d, false, Rect::new(cx - 20, cy - 9, cx + 21, cy + 10), with_depth);
                }
            }
        }
    }
}

/// A whole stroke rendered with the frozen scalar rasterizer: the renderer's own dab generation,
/// accumulation and composite, with each dab rasterised by `rasterize_reference`.
fn reference_stroke(brush: &BrushSettings, pts: &[StrokePoint], pre: &Surface, sel: Option<&Surface>, lock: bool) -> (Rect, Surface) {
    let mut r = StrokeRenderer::new(brush, Some(pre.format()), 1.0);
    let (mut dabs, mut duals) = (Vec::new(), Vec::new());
    r.generator.push(pts, &mut dabs, &mut duals);
    r.generator.finish(&mut dabs, &mut duals);
    let wet = r.ctx.brush.wet_edges;
    let (mut buf, mut depth, mut native) = (Vec::new(), Vec::new(), [0.0f32; 8]);
    for d in &dabs {
        let rect = r.ctx.dab_rect(d, false);
        r.ctx.rasterize_reference(d, false, rect, &mut buf, wet.then_some(&mut depth));
        let col = match (r.per_dab_color, r.fmt) {
            (true, Some(f)) => {
                from_rgba_into(&f, d.color, &mut native);
                Some(&native[..f.mode.color_channels()])
            }
            _ => None,
        };
        r.cov.accumulate(rect, &buf, d.opacity, col, wet.then_some(&depth[..]));
        r.cov.bounds = r.cov.bounds.union(&rect);
    }
    if let Some(dm) = r.dual.as_mut() {
        for d in &duals {
            let rect = r.ctx.dab_rect(d, true);
            r.ctx.rasterize_reference(d, true, rect, &mut buf, None);
            dm.accumulate(rect, &buf, 1.0, None, None);
        }
    }
    let mut out = pre.clone();
    let dmg = r.composite(pre, &mut out, sel, lock, true);
    (dmg, out)
}

fn assert_same_surface(got: &Surface, want: &Surface, r: Rect, what: &str) {
    let (a, b) = (got.read_region(r), want.read_region(r));
    let mismatch = a.iter().zip(&b).position(|(a, b)| a.to_bits() != b.to_bits());
    assert!(mismatch.is_none(), "{what}: first differing sample {mismatch:?} of {r:?}");
}

/// Golden strokes: a sampled-tip stroke across coverage and layer tile boundaries and off the
/// canvas edge, rendered by the renderer and by the frozen scalar rasterizer, gives the same
/// pixels at 8, 16 and 32 bits in RGB, Gray and CMYK, in every blend mode, at opacity and flow
/// 0 and 1, erasing, with Lock Transparency and a selection, and at the smallest spacing.
#[test]
fn sampled_strokes_match_the_scalar_rasterizer_pixel_for_pixel() {
    use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType};
    let tip = grain(48, 40);
    // From off the canvas (negative coordinates) across the 64² coverage and 256² layer tiles.
    let pts: Vec<StrokePoint> =
        (0..24).map(|i| StrokePoint::new(-30.0 + i as f64 * 14.0, 250.0 + (i as f64 * 0.7).sin() * 20.0, 0.4 + 0.025 * i as f32)).collect();
    let base = BrushSettings {
        size: 46.0,
        spacing: 0.3,
        seed: 3,
        color: [0.8, 0.2, 0.1, 1.0],
        shape_dynamics: crate::brush::ShapeDynamics {
            enabled: true,
            angle: crate::brush::Dynamic::jitter(1.0),
            size: crate::brush::Dynamic::jitter(0.4),
            ..Default::default()
        },
        ..sampled(&tip)
    };
    let mut sel = Surface::new(PixelFormat::GRAY8);
    sel.fill_rect(Rect::new(0, 200, 180, 300), &[1.0]);
    sel.fill_rect(Rect::new(180, 200, 260, 300), &[0.4]);
    let mut cases: Vec<(String, BrushSettings, bool, bool)> = Vec::new();
    for mode in BlendMode::layer_modes() {
        cases.push((format!("{mode:?}"), BrushSettings { mode, ..base.clone() }, false, false));
    }
    for (o, f) in [(0.0, 1.0), (1.0, 0.0), (1.0, 1.0), (0.35, 0.6)] {
        cases.push((format!("opacity {o} flow {f}"), BrushSettings { opacity: o, flow: f, ..base.clone() }, false, false));
    }
    cases.push(("erase".into(), BrushSettings { erase: true, ..base.clone() }, false, false));
    cases.push(("lock transparency".into(), base.clone(), true, false));
    cases.push(("selection".into(), base.clone(), false, true));
    cases.push(("spacing 1 %".into(), BrushSettings { spacing: 0.01, ..base.clone() }, false, false));
    cases.push(("aliased".into(), BrushSettings { aliased: true, ..base.clone() }, false, false));
    cases.push(("wet edges and noise".into(), BrushSettings { wet_edges: true, noise: true, ..base.clone() }, false, false));
    cases.push((
        "per-dab colour".into(),
        BrushSettings { color_dynamics: crate::brush::ColorDynamics { enabled: true, hue_jitter: 0.5, ..Default::default() }, ..base.clone() },
        false,
        false,
    ));
    for (mode, sample) in [
        (ColorMode::Rgb, SampleType::U8),
        (ColorMode::Rgb, SampleType::U16),
        (ColorMode::Rgb, SampleType::F32),
        (ColorMode::Grayscale, SampleType::U8),
        (ColorMode::Cmyk, SampleType::U16),
    ] {
        let fmt = PixelFormat::new(mode, sample, true);
        let mut pre = Surface::new(fmt);
        // Half the canvas painted, half transparent: Lock Transparency and the eraser have both.
        for (x, c) in [(0, [0.2, 0.5, 0.9, 1.0]), (150, [0.9, 0.9, 0.3, 0.6])] {
            pre.fill_rect(Rect::new(x, 180, x + 150, 320), &photocraft_raster::from_rgba(&fmt, c));
        }
        for (what, b, lock, with_sel) in &cases {
            // The pixel modes run on RGB; the others are checked once there.
            if mode == ColorMode::Rgb || what.starts_with("opacity") || what == "Normal" || what == "erase" {
                let selection = with_sel.then_some(&sel);
                let (want_dmg, want) = reference_stroke(b, &pts, &pre, selection, *lock);
                let mut got = pre.clone();
                let dmg = render_stroke(&mut got, b, &pts, selection, *lock, 1.0);
                assert_eq!(dmg, want_dmg, "{what} {fmt:?}");
                assert_same_surface(&got, &want, dmg.union(&Rect::new(-64, 160, 360, 340)), &format!("{what} {fmt:?}"));
            }
        }
    }
}

/// Chunked live rendering of a sampled stroke (the canvas's `push` + `composite` per frame) ends
/// on the scalar rasterizer's pixels too.
#[test]
fn live_sampled_strokes_match_the_scalar_rasterizer() {
    use photocraft_color::PixelFormat;
    let tip = grain(64, 64);
    let b = BrushSettings {
        size: 90.0,
        spacing: 0.1,
        seed: 11,
        smoothing: crate::brush::Smoothing { enabled: true, amount: 0.4, ..Default::default() },
        shape_dynamics: crate::brush::ShapeDynamics { enabled: true, angle: crate::brush::Dynamic::jitter(1.0), ..Default::default() },
        ..sampled(&tip)
    };
    let pts: Vec<StrokePoint> =
        (0..40).map(|i| StrokePoint { time: i as f64 * 8.0, ..StrokePoint::new(30.0 + i as f64 * 9.0, 120.0 + (i as f64 * 0.4).cos() * 60.0, 1.0) }).collect();
    let fmt = PixelFormat::RGBA8;
    let mut pre = Surface::new(fmt);
    pre.fill_rect(Rect::new(0, 0, 400, 260), &[0.9, 0.9, 0.9, 1.0]);
    let (want_dmg, want) = reference_stroke(&b, &pts, &pre, None, false);
    let mut live = pre.clone();
    let mut r = StrokeRenderer::new(&b, Some(fmt), 1.0);
    for c in pts.chunks(3) {
        r.push(c);
        r.composite(&pre, &mut live, None, false, false);
    }
    r.finish();
    r.composite(&pre, &mut live, None, false, false);
    assert_eq!(r.bounds(), want_dmg);
    assert_same_surface(&live, &want, want_dmg, "live sampled stroke");
}

/// A small deterministic generator for the randomised checks below (xorshift64*).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next()
    }
    /// `10^e` for `e` uniform in `lo..hi`, with a random sign.
    fn magnitude(&mut self, lo: f64, hi: f64) -> f64 {
        let v = 10f64.powf(self.range(lo, hi));
        if self.next() < 0.5 { -v } else { v }
    }
}

/// The row bound the kernel skips columns by ([`SampledTip::columns`]) holds every pixel whose
/// f32 window test passes, for mappings far beyond what brushes produce: centres to ±3·10⁷
/// (where f32 can't hold the pixel grid), any rotation and flip, projection up to the strongest,
/// scales from 1/1000 to 10⁴ texels per pixel, squash to 1/10⁴ and tips up to 5000 texels; on
/// one row in four the scale is negative, and on one in eight the squash too (a direct caller's
/// negative radius and roundness).
#[test]
fn sampled_tip_column_bound_holds_every_pixel_in_the_window() {
    let mut rng = Rng(0x5eed_1234_abcd_ef01);
    let data = [0.5f32; 4];
    let (mut rows, mut bounded, mut hits) = (0, 0, 0);
    for n in 0..30_000 {
        // A third of the rows far out, where f32 rounds the pixel grid itself.
        let far = n % 3 == 0;
        let angle = rng.range(-4.0, 4.0) as f32;
        let (sn, cs) = angle.sin_cos();
        let pa = rng.range(-4.0, 4.0) as f32;
        let proj = rng.next() < 0.5;
        let k = if proj { rng.range(0.0, 19.0) as f32 } else { 0.0 };
        // The signs come from `n`, so the random draws stay those of the positive mappings.
        let (inv_sign, ro_sign) = (if n % 4 == 1 { -1.0 } else { 1.0 }, if n % 8 == 5 { -1.0 } else { 1.0 });
        let tip = SampledTip {
            cx: if far { rng.magnitude(6.5, 7.5) } else { rng.magnitude(-1.0, 6.0) } as f32,
            sn,
            cs,
            flip_x: if rng.next() < 0.5 { -1.0 } else { 1.0 },
            flip_y: if rng.next() < 0.5 { -1.0 } else { 1.0 },
            proj: (pa.cos(), pa.sin(), k),
            inv_scale: inv_sign * 10f64.powf(if far { rng.range(-1.0, 1.0) } else { rng.range(-3.0, 4.0) }) as f32,
            tw: rng.range(1.0, 5000.0).round() as f32,
            th: rng.range(1.0, 5000.0).round() as f32,
            ro: ro_sign * 10f64.powf(if far { rng.range(-0.5, 0.0) } else { rng.range(-4.0, 0.0) }) as f32,
            lw: 2,
            lh: 2,
            data: &data,
        };
        // A row through a point of tip space mostly near the window's edges (inverting the
        // mapping in f64), as long as a brush's rectangle can be.
        let edge = |rng: &mut Rng, top: f32| match (rng.next() * 3.0) as u32 {
            0 => -1.0 + rng.range(-0.5, 0.5),
            1 => f64::from(top) + 1.0 + rng.range(-0.5, 0.5),
            _ => rng.range(-2.0, f64::from(top) + 2.0),
        };
        let (tx, ty) = (edge(&mut rng, tip.tw), edge(&mut rng, tip.th));
        let inv = f64::from(tip.inv_scale);
        let u = (tx - f64::from(tip.tw) / 2.0) / inv * f64::from(tip.flip_x);
        let v = -(ty - f64::from(tip.th) / 2.0) * f64::from(tip.ro) / inv * f64::from(tip.flip_y);
        let (s, c) = (f64::from(tip.sn), f64::from(tip.cs));
        let (mut ux, mut uy) = (u * c - v * s, u * s + v * c);
        let (ax, ay, k) = if proj { (f64::from(tip.proj.0), f64::from(tip.proj.1), f64::from(tip.proj.2)) } else { (0.0, 0.0, 0.0) };
        let back = (ux * ax + uy * ay) * k / (1.0 + k);
        (ux, uy) = (ux - back * ax, uy - back * ay);
        let dy = (-uy).clamp(-1e9, 1e9) as f32;
        let len = rng.range(1.0, 600.0) as i32;
        let start = (f64::from(tip.cx) + ux - f64::from(len) * rng.next()).clamp(-2e9, 2e9) as i32;
        let (x0, x1) = (start, start.saturating_add(len));
        rows += 1;
        let Some((first, last)) = tip.columns(x0, x1, dy, proj) else { continue };
        bounded += 1;
        let mut x = x0;
        while x < x1 {
            let (_, inside) = if proj { tip.block::<true>(x, dy) } else { tip.block::<false>(x, dy) };
            for (i, &inside) in inside.iter().enumerate() {
                let px = i64::from(x) + i as i64;
                if inside && px < i64::from(x1) {
                    hits += 1;
                    assert!((first..=last).contains(&px), "column {px} passes the window test outside [{first}, {last}] (row {x0}..{x1}, dy {dy})");
                }
            }
            x = x.saturating_add(LANES as i32);
        }
    }
    // The check exercised the bound: most rows got one, and many pixels were inside windows.
    assert!(bounded > rows / 2 && hits > 100_000, "rows {rows}, bounded {bounded}, inside pixels {hits}");
}

/// Random sampled dabs (tips, sizes, poses, flows, centres near and far, windows of every
/// shape) through the kernel and the scalar loop: the same bits every time.
#[test]
fn random_sampled_dabs_match_scalar_bits() {
    let mut rng = Rng(0x0dab_5eed_0bad_cafe);
    let tips: Vec<GrayTile> = [(1, 1), (3, 2), (17, 31), (64, 64), (120, 45)].iter().map(|&(w, h)| grain(w, h)).collect();
    for n in 0..1500 {
        let tip = &tips[n % tips.len()];
        let b = BrushSettings { aliased: rng.next() < 0.2, noise: rng.next() < 0.1, wet_edges: rng.next() < 0.1, ..sampled(tip) };
        let ctx = BrushContext::new(&b);
        let far = rng.next() < 0.15;
        let center = Point::new(rng.magnitude(0.0, if far { 7.3 } else { 3.0 }), rng.magnitude(0.0, if far { 7.3 } else { 3.0 }));
        let mut d = Dab::round(center, rng.magnitude(-0.7, 2.3).abs() as f32, rng.range(-0.2, 1.0) as f32);
        d.angle = rng.range(-7.0, 7.0) as f32;
        d.roundness = 10f64.powf(rng.range(-2.5, 0.0)) as f32;
        d.flip_x = rng.next() < 0.5;
        d.flip_y = rng.next() < 0.5;
        if rng.next() < 0.4 {
            d.proj_scale = rng.range(0.05, 1.0) as f32;
            d.proj_angle = rng.range(-4.0, 4.0) as f32;
        }
        let full = ctx.dab_rect(&d, false);
        // The whole dab when it is small, else a window somewhere over it.
        let rect = if (full.width() as u64) * (full.height() as u64) <= 6000 {
            full
        } else {
            let (w, h) = (rng.range(1.0, 90.0) as i32, rng.range(1.0, 40.0) as i32);
            let x = full.x0 + (rng.next() * f64::from(full.width().saturating_sub(w as u32))) as i32;
            let y = full.y0 + (rng.next() * f64::from(full.height().saturating_sub(h as u32))) as i32;
            Rect::new(x, y, x + w, y + h)
        };
        compare_sampled(&ctx, &d, false, rect, rng.next() < 0.2);
    }
}
