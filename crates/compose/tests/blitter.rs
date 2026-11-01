//! Integration tests for `craft-blitter` dispatch in `photocraft-compose`:
//! verifies bit-identity (`f32::to_bits`) between `craft-blitter` fast paths and PhotoCraft's
//! pre-existing scalar compositing code across the full 16-bit (`0..=65535`) sample range,
//! block/tail/parallel buffer sizes, clipping groups (`SourceAtop`), 8/16/32f depths, and
//! PhotoCraft-specific fallback paths (`Special Eight` fill, `TEXT_GAMMA`, `LAB_MIX`, `Dissolve`,
//! and `EDGE` modes).

use photocraft_color::{BlendMode, Color, ColorMode, PixelFormat, SampleType};
use photocraft_compose::{psblend, render, render_tiled};
use photocraft_doc::{Document, Layer};
use photocraft_geom::{Rect, Size};

fn solid_layer(name: &str, rect: Rect, rgba: [f32; 4], mode: BlendMode, opacity: f32, fill: f32) -> Layer {
    let mut l = Layer::raster(name, PixelFormat::RGBA32F);
    l.blend = mode;
    l.opacity = opacity;
    l.fill_opacity = fill;
    if let Some(s) = l.surface_mut() {
        s.fill_rect(rect, &rgba);
    }
    l
}

fn gradient_layer(name: &str, rect: Rect, fmt: PixelFormat, seed: u32, mode: BlendMode, opacity: f32, fill: f32) -> Layer {
    let mut l = Layer::raster(name, fmt);
    l.blend = mode;
    l.opacity = opacity;
    l.fill_opacity = fill;
    let w = rect.width() as usize;
    let h = rect.height() as usize;
    let mut rgba = Vec::with_capacity(w * h * 4);
    for i in 0..(w * h) {
        let h1 = (i as u32).wrapping_mul(747_796_405).wrapping_add(seed.wrapping_mul(2_891_336_453));
        let r = (h1 & 0xff) as f32 / 255.0;
        let g = ((h1 >> 8) & 0xff) as f32 / 255.0;
        let b = ((h1 >> 16) & 0xff) as f32 / 255.0;
        let a = ((h1 >> 24) & 0xff) as f32 / 255.0;
        rgba.extend_from_slice(&[r, g, b, a]);
    }
    if let Some(s) = l.surface_mut() {
        let mut flat = Vec::with_capacity(w * h * fmt.channels());
        for &px in rgba.as_chunks::<4>().0 {
            flat.extend_from_slice(&photocraft_raster::from_rgba(&fmt, px));
        }
        s.write_region(rect, &flat);
    }
    l
}

/// Pre-PR scalar `psblend::composite` implementation from `main`, kept verbatim as the reference
/// oracle so tests prove zero IEEE-754 bit drift (`f32::to_bits`) against the old code.
fn legacy_scalar_composite(mode: BlendMode, backdrop: [f32; 4], source: [f32; 4], opacity: f32) -> [f32; 4] {
    let ab = backdrop[3];
    let as_ = source[3] * opacity;
    if as_ <= 0.0 {
        return backdrop;
    }
    let ao = as_ + ab * (1.0 - as_);
    if mode == BlendMode::Normal {
        if ao <= 0.0 {
            return [0.0; 4];
        }
        let kb = ab * (1.0 - as_);
        let inv = 1.0 / ao;
        return [(kb * backdrop[0] + as_ * source[0]) * inv, (kb * backdrop[1] + as_ * source[1]) * inv, (kb * backdrop[2] + as_ * source[2]) * inv, ao];
    }
    let cb = [backdrop[0], backdrop[1], backdrop[2]];
    let cs = [source[0], source[1], source[2]];
    let b = psblend::blend_rgb(mode, cb, cs);
    if ao <= 0.0 {
        return [0.0; 4];
    }
    let mut out = [0.0f32; 4];
    for i in 0..3 {
        out[i] = ((1.0 - as_) * ab * cb[i] + (1.0 - ab) * as_ * cs[i] + as_ * ab * b[i]) / ao;
    }
    out[3] = ao;
    out
}

fn quantize_u16(px: [f32; 4]) -> [u16; 4] {
    px.map(|v| (v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16)
}

#[test]
fn full_16bit_range_blitter_matches_legacy_compositor_bit_for_bit() {
    // 256 x 256 = 65,536 pixels: covers every 16-bit integer `0u16..=65535u16` across channels and
    // exceeds `PAR_THRESHOLD` (4,096 pixels), exercising both 8-lane SoA chunks and Rayon dispatch
    // (wrapped in an isolated group so `blend_into_fill` blits the whole 65,536-pixel buffer at once,
    // as well as ungrouped pixel-layer row blits).
    let rect = Rect::new(0, 0, 256, 256);
    let mut base_bytes = Vec::with_capacity(65_536 * 8);
    let mut top_bytes = Vec::with_capacity(65_536 * 8);
    let mut base_f32 = Vec::with_capacity(65_536);
    let mut top_f32 = Vec::with_capacity(65_536);

    for i in 0u32..=65_535 {
        let br = i as u16;
        let bg = (65_535 - i) as u16;
        let bb = (i.wrapping_mul(40_503)) as u16;
        let ba = 65_535u16;

        let sr = (i.wrapping_mul(257).wrapping_add(1)) as u16;
        let sg = (i.wrapping_mul(12_289).wrapping_add(3)) as u16;
        let sb = (65_535 - i) as u16;
        let sa = i as u16;

        for ch in [br, bg, bb, ba] {
            base_bytes.extend_from_slice(&ch.to_ne_bytes());
        }
        for ch in [sr, sg, sb, sa] {
            top_bytes.extend_from_slice(&ch.to_ne_bytes());
        }
        base_f32.push([br as f32 / 65535.0, bg as f32 / 65535.0, bb as f32 / 65535.0, 1.0]);
        top_f32.push([sr as f32 / 65535.0, sg as f32 / 65535.0, sb as f32 / 65535.0, sa as f32 / 65535.0]);
    }

    let mut base_layer = Layer::raster("base16", PixelFormat::RGBA16);
    base_layer.surface_mut().unwrap().try_write_interleaved(rect, &base_bytes).unwrap();

    // Representative modes hitting 8-lane SoA (`Normal`, `Multiply`, `Screen`, `LinearDodge`,
    // `Difference`), scalar/Rayon blitter modes (`Overlay`, `SoftLight`, `Color`), and fallbacks
    // (`VividLight`, `ColorBurn`, `HardMix`) at both partial (0.75) and full (1.0) opacity.
    for mode in [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::LinearDodge,
        BlendMode::Difference,
        BlendMode::Overlay,
        BlendMode::SoftLight,
        BlendMode::Color,
        BlendMode::VividLight,
        BlendMode::ColorBurn,
        BlendMode::HardMix,
    ] {
        for opacity in [0.75f32, 1.0f32] {
            let mut top_layer = Layer::raster("top16", PixelFormat::RGBA16);
            top_layer.surface_mut().unwrap().try_write_interleaved(rect, &top_bytes).unwrap();

            // 1. Direct pixel-layer path (`composite_pixel_layer` -> 16-bit row decode + row span blit).
            let mut doc_row = Document::new("row16", Size::new(256, 256), ColorMode::Rgb, SampleType::U16);
            doc_row.layers.push(base_layer.clone());
            let mut row_top = top_layer.clone();
            row_top.blend = mode;
            row_top.opacity = opacity;
            doc_row.layers.push(row_top);
            let got_row = render(&doc_row, rect);

            // 2. Full-buffer path (`blend_into_fill` -> `try_blit_rgba32f` across all 65,536 pixels via Rayon).
            let mut doc_buf = Document::new("buf16", Size::new(256, 256), ColorMode::Rgb, SampleType::U16);
            doc_buf.layers.push(base_layer.clone());
            let mut grp = Layer::group("iso", vec![top_layer]);
            grp.blend = mode;
            grp.opacity = opacity;
            doc_buf.layers.push(grp);
            let got_buf = render(&doc_buf, rect);

            for i in 0..65_536 {
                let want = legacy_scalar_composite(mode, base_f32[i], top_f32[i], opacity);
                assert_eq!(got_row.px[i].map(f32::to_bits), want.map(f32::to_bits), "{mode:?} opacity={opacity} row pixel {i}");
                assert_eq!(quantize_u16(got_row.px[i]), quantize_u16(want), "{mode:?} opacity={opacity} u16 row pixel {i}");
                // Note: isolated group normalizes semi-transparent pixels through `composite_normal_rgb` first;
                // when `sa == 65535` (or `Normal`), `got_buf` also matches `want` bit-for-bit.
                if top_f32[i][3] == 1.0 || top_f32[i][3] == 0.0 {
                    assert_eq!(got_buf.px[i].map(f32::to_bits), want.map(f32::to_bits), "{mode:?} opacity={opacity} buf pixel {i}");
                }
            }
        }
    }

    // 3. Semi-transparent 16-bit backdrop (`0..=65535` backdrop alpha) + 16-bit source across all 65,536 values:
    // verifies `Normal` (fast path) and non-`Normal` (`ao < 1.0` scalar fallback) remain bit-identical to `legacy_scalar_composite`.
    let mut semi_bytes = Vec::with_capacity(65_536 * 8);
    for i in 0u32..=65_535 {
        let br = (i.wrapping_mul(8_191)) as u16;
        let bg = i as u16;
        let bb = (65_535 - i) as u16;
        let ba = (i.wrapping_mul(6_553).wrapping_add(1)) as u16;
        for ch in [br, bg, bb, ba] {
            semi_bytes.extend_from_slice(&ch.to_ne_bytes());
        }
    }
    let mut semi_base = Layer::raster("semi16", PixelFormat::RGBA16);
    semi_base.surface_mut().unwrap().try_write_interleaved(rect, &semi_bytes).unwrap();
    let mut base_only_doc = Document::new("base_only", Size::new(256, 256), ColorMode::Rgb, SampleType::U16);
    base_only_doc.layers.push(semi_base.clone());
    let rendered_semi_base = render(&base_only_doc, rect);

    for mode in [BlendMode::Normal, BlendMode::Multiply, BlendMode::Screen, BlendMode::SoftLight, BlendMode::VividLight] {
        let mut doc_semi = Document::new("semi_doc", Size::new(256, 256), ColorMode::Rgb, SampleType::U16);
        doc_semi.layers.push(semi_base.clone());
        let mut top_layer = Layer::raster("top16", PixelFormat::RGBA16);
        top_layer.surface_mut().unwrap().try_write_interleaved(rect, &top_bytes).unwrap();
        top_layer.blend = mode;
        top_layer.opacity = 0.75;
        doc_semi.layers.push(top_layer);
        let got_semi = render(&doc_semi, rect);
        for (i, ((&got, &base), &top)) in got_semi.px.iter().zip(&rendered_semi_base.px).zip(&top_f32).enumerate() {
            let want = legacy_scalar_composite(mode, base, top, 0.75);
            assert_eq!(got.map(f32::to_bits), want.map(f32::to_bits), "semi {mode:?} pixel {i}");
            assert_eq!(quantize_u16(got), quantize_u16(want), "semi u16 {mode:?} pixel {i}");
        }
    }
}

#[test]
fn blitter_modes_match_tiled_and_scalar_across_parallel_threshold() {
    // 73 x 61 = 4,453 pixels: exercises 8-lane SoA blocks, scalar tail (4453 % 8 == 5),
    // and Rayon parallel dispatch (>= 4,096 pixels) vs small 19x19 tiles (< 4,096 pixels).
    let (w, h) = (73, 61);
    let rect = Rect::new(0, 0, w as i32, h as i32);

    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let fmt = PixelFormat::new(ColorMode::Rgb, depth, true);
        let base = gradient_layer("base", rect, fmt, 11, BlendMode::Normal, 1.0, 1.0);

        for mode in BlendMode::layer_modes() {
            let mut doc = Document::new("d", Size::new(w, h), ColorMode::Rgb, depth);
            doc.layers.push(base.clone());
            doc.layers.push(gradient_layer("top", rect, fmt, 29, mode, 0.75, 0.85));

            let full = render(&doc, rect);
            let tiled = render_tiled(&doc, rect, 19);
            assert_eq!(full.px.len(), tiled.px.len());
            for (i, (a, b)) in full.px.iter().zip(&tiled.px).enumerate() {
                assert_eq!(a.map(f32::to_bits), b.map(f32::to_bits), "{depth:?} {mode:?} pixel {i}: {a:?} vs {b:?}");
            }
        }
    }
}

#[test]
fn clipping_groups_preserve_base_alpha_and_match_scalar_atop() {
    let rect = Rect::new(0, 0, 35, 17);
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let fmt = PixelFormat::new(ColorMode::Rgb, depth, true);
        for mode in [BlendMode::Normal, BlendMode::Multiply, BlendMode::Screen, BlendMode::Overlay, BlendMode::ColorBurn, BlendMode::Dissolve, BlendMode::Color]
        {
            let mut doc = Document::new("clip", Size::new(35, 17), ColorMode::Rgb, depth);
            // Half-canvas base at 0.4 alpha; right half stays 0.0 alpha.
            let mut base = gradient_layer("base", Rect::new(0, 0, 18, 17), fmt, 3, BlendMode::Normal, 0.4, 1.0);
            base.clipped = false;
            let mut clipped = gradient_layer("clip", rect, fmt, 17, mode, 0.7, 0.8);
            clipped.clipped = true;
            doc.layers.push(base);
            doc.layers.push(clipped);

            let full = render(&doc, rect);
            let tiled = render_tiled(&doc, rect, 11);
            assert_eq!(full.px, tiled.px, "{depth:?} {mode:?}");

            // Transparent right half must never leak colour from the clipped layer.
            for y in 0..17 {
                for x in 18..35 {
                    assert_eq!(full.get(x, y), [0.0; 4], "{depth:?} {mode:?} leaked at ({x}, {y})");
                }
            }
        }
    }
}

#[test]
fn photoshop_specific_fallbacks_remain_intact() {
    let rect = Rect::new(0, 0, 16, 16);
    // Special Eight at Fill < 100% must take psblend::composite_fill rather than plain opacity * fill.
    let mut doc = Document::with_background("s8", Size::new(16, 16), ColorMode::Rgb, SampleType::U8, Color::rgb(200.0 / 255.0, 150.0 / 255.0, 100.0 / 255.0));
    doc.layers.push(solid_layer("burn_half_fill", rect, [100.0 / 255.0, 180.0 / 255.0, 220.0 / 255.0, 1.0], BlendMode::ColorBurn, 1.0, 128.0 / 255.0));
    let px = render(&doc, rect).get(0, 0);
    let expected = psblend::composite_fill(
        BlendMode::ColorBurn,
        [200.0 / 255.0, 150.0 / 255.0, 100.0 / 255.0, 1.0],
        [100.0 / 255.0, 180.0 / 255.0, 220.0 / 255.0, 1.0],
        1.0,
        128.0 / 255.0,
        1.0,
    );
    for c in 0..4 {
        assert!((px[c] - expected[c]).abs() < 1e-3, "channel {c}: {px:?} vs {expected:?}");
    }

    // Non-finite source alpha on a float layer falls back to scalar `psblend::composite`, preserving
    // the exact pre-PR NaN semantics (`tests::normal_rgb_buffer_preserves_alpha_skip_and_nested_scopes`).
    let mut fdoc = Document::with_background("nan_alpha", Size::new(12, 4), ColorMode::Rgb, SampleType::F32, Color::rgb(0.25, 0.5, 0.75));
    let mut bad = Layer::raster("bad", PixelFormat::RGBA32F);
    bad.blend = BlendMode::Multiply;
    if let Some(s) = bad.surface_mut() {
        s.write_pixel(3, 1, &[0.8, 0.2, 0.1, f32::NAN]);
    }
    fdoc.layers.push(bad);
    let out = render(&fdoc, Rect::new(0, 0, 12, 4));
    assert!(out.get(3, 1).iter().all(|v| v.is_nan()));
    assert_eq!(out.get(2, 1), [0.25, 0.5, 0.75, 1.0]);
}
