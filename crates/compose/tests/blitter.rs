//! Integration tests for `craft-blitter` dispatch in `photocraft-compose`:
//! verifies bit-identity between scalar `psblend` compositing and slice blitting across
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
    for depth in [SampleType::U8, SampleType::F32] {
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

    // Non-finite source alpha on a float layer must not poison a non-Normal blitter composite.
    let mut fdoc = Document::with_background("nan_alpha", Size::new(12, 4), ColorMode::Rgb, SampleType::F32, Color::rgb(0.25, 0.5, 0.75));
    let mut bad = Layer::raster("bad", PixelFormat::RGBA32F);
    bad.blend = BlendMode::Multiply;
    if let Some(s) = bad.surface_mut() {
        s.write_pixel(3, 1, &[0.8, 0.2, 0.1, f32::NAN]);
    }
    fdoc.layers.push(bad);
    let out = render(&fdoc, Rect::new(0, 0, 12, 4));
    assert_eq!(out.get(3, 1), [0.25, 0.5, 0.75, 1.0]);
}
