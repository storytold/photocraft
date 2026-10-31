//! Adversarial stress test suite for photocraft-compose & craft-blitter integration.
//!
//! Rigorously verifies:
//! 1. Extreme rects (zero-size, negative, inverted, giant, out-of-bounds).
//! 2. Out-of-range opacity and fill dynamics (negative, > 1.0, NaN, Inf).
//! 3. Compositing over transparent and translucent backdrops across all blend modes.
//! 4. Clipping masks over fully transparent, translucent, and stacked bases (8-bit and 32-bit float).
//! 5. Memory safety, absence of undefined behavior, and zero panics under adversarial inputs.

use photocraft_color::{BlendMode, Color, ColorMode, PixelFormat, SampleType};
use photocraft_compose::psblend::{self, to_blitter_blend_mode};
use photocraft_compose::{MAX_RENDER_PIXELS, render, render_tiled};
use photocraft_doc::{Document, Layer};
use photocraft_geom::{Rect, Size};

const ALL_BLEND_MODES: [BlendMode; 34] = [
    BlendMode::PassThrough,
    BlendMode::Normal,
    BlendMode::Dissolve,
    BlendMode::Darken,
    BlendMode::Multiply,
    BlendMode::ColorBurn,
    BlendMode::LinearBurn,
    BlendMode::DarkerColor,
    BlendMode::Lighten,
    BlendMode::Screen,
    BlendMode::ColorDodge,
    BlendMode::LinearDodge,
    BlendMode::LighterColor,
    BlendMode::Overlay,
    BlendMode::SoftLight,
    BlendMode::HardLight,
    BlendMode::VividLight,
    BlendMode::LinearLight,
    BlendMode::PinLight,
    BlendMode::HardMix,
    BlendMode::Difference,
    BlendMode::Exclusion,
    BlendMode::Subtract,
    BlendMode::Divide,
    BlendMode::Hue,
    BlendMode::Saturation,
    BlendMode::Color,
    BlendMode::Luminosity,
    BlendMode::Reflect,
    BlendMode::Glow,
    BlendMode::Negation,
    BlendMode::Xor,
    BlendMode::PaintNetColorBurn,
    BlendMode::PaintNetColorDodge,
];

fn solid_layer(name: &str, rect: Rect, rgba: [f32; 4], mode: BlendMode, opacity: f32, fill: f32) -> Layer {
    let mut l = Layer::raster(name, PixelFormat::RGBA8);
    l.blend = mode;
    l.opacity = opacity;
    l.fill_opacity = fill;
    l.surface_mut().unwrap().fill_rect(rect, &rgba);
    l
}

fn solid_layer_f32(name: &str, rect: Rect, rgba: [f32; 4], mode: BlendMode, opacity: f32, fill: f32) -> Layer {
    let mut l = Layer::raster(name, PixelFormat::RGBA32F);
    l.blend = mode;
    l.opacity = opacity;
    l.fill_opacity = fill;
    l.surface_mut().unwrap().fill_rect(rect, &rgba);
    l
}

fn empty_doc(w: u32, h: u32) -> Document {
    Document::new("test", Size::new(w, h), ColorMode::Rgb, SampleType::U8)
}

fn empty_doc_f32(w: u32, h: u32) -> Document {
    Document::new("test", Size::new(w, h), ColorMode::Rgb, SampleType::F32)
}

fn white_doc(w: u32, h: u32) -> Document {
    Document::with_background("test", Size::new(w, h), ColorMode::Rgb, SampleType::U8, Color::WHITE)
}

// =========================================================================
// 1. Extreme & Zero-Size Rects
// =========================================================================

#[test]
fn zero_size_rects_and_empty_bounds_never_panic() {
    let mut doc = empty_doc(16, 16);
    doc.layers.push(solid_layer("base", Rect::new(0, 0, 16, 16), [0.5, 0.5, 0.5, 1.0], BlendMode::Normal, 1.0, 1.0));

    let zero_rects = [
        Rect::new(0, 0, 0, 0),
        Rect::new(5, 5, 5, 5),
        Rect::new(0, 0, 10, 0),
        Rect::new(0, 0, 0, 10),
        Rect::new(10, 10, 5, 5),       // inverted
        Rect::new(-10, -10, -5, -5),   // negative
        Rect::new(100, 100, 200, 200), // outside canvas
    ];

    for r in zero_rects {
        let b = render(&doc, r);
        if r.is_empty() {
            assert!(b.px.is_empty(), "empty rect {r:?} should yield empty px buffer");
        }
        for tile in [1, 2, 7, 256] {
            let bt = render_tiled(&doc, r, tile);
            assert_eq!(b.px, bt.px, "tiled vs direct mismatch on rect {r:?}");
        }
    }
}

#[test]
fn gigantic_rect_does_not_abort_or_panic() {
    let doc = empty_doc(10, 10);
    // Exceeds MAX_RENDER_PIXELS (2^30)
    let giant = Rect::new(0, 0, 200_000, 200_000);
    assert!(u64::from(giant.width()) * u64::from(giant.height()) > MAX_RENDER_PIXELS);
    let b = render(&doc, giant);
    assert!(b.px.is_empty(), "giant rect must be refused gracefully with empty buffer");
}

#[test]
fn odd_and_single_pixel_rects_match_across_tile_sizes() {
    let mut doc = empty_doc(33, 17);
    doc.layers.push(solid_layer("layer1", Rect::new(2, 3, 20, 15), [0.8, 0.2, 0.4, 0.9], BlendMode::Multiply, 0.8, 0.7));
    doc.layers.push(solid_layer("layer2", Rect::new(10, 5, 30, 16), [0.1, 0.7, 0.3, 0.6], BlendMode::Screen, 0.9, 0.5));

    let single_px = Rect::new(12, 8, 13, 9);
    let direct_single = render(&doc, single_px);
    assert_eq!(direct_single.px.len(), 1);

    for tile in [1, 3, 5, 16, 256] {
        let tiled_single = render_tiled(&doc, single_px, tile);
        assert_eq!(direct_single.px, tiled_single.px);
    }

    let whole = doc.bounds();
    let direct_whole = render(&doc, whole);
    for tile in [1, 3, 7, 16, 64, 256] {
        let tiled_whole = render_tiled(&doc, whole, tile);
        assert_eq!(direct_whole.px, tiled_whole.px, "tile size {tile} mismatch");
    }
}

// =========================================================================
// 2. Out-of-Range Opacity & Fill (Negative, > 1.0, NaN, Inf)
// =========================================================================

#[test]
fn out_of_range_and_nan_opacity_and_fill_never_panic() {
    let test_values = [0.0f32, -0.001, -1.0, -1000.0, 1.0, 1.001, 2.5, 1e5, f32::NAN, f32::INFINITY, f32::NEG_INFINITY];

    let backdrop = [0.4f32, 0.5, 0.6, 0.7];
    let source = [0.8f32, 0.2, 0.1, 0.9];

    for &mode in &ALL_BLEND_MODES {
        for &op in &test_values {
            // Test direct psblend scalar functions
            let _ = psblend::composite(mode, backdrop, source, op);
            let _ = psblend::composite_gamma(mode, backdrop, source, op, 1.45);

            for &fill in &test_values {
                let _ = psblend::composite_fill(mode, backdrop, source, op, fill, 1.0);
                let _ = psblend::composite_fill(mode, backdrop, source, op, fill, 1.45);
            }
        }
    }
}

#[test]
fn craft_blitter_slice_adversarial_values_never_panic() {
    let test_values = [0.0f32, -0.5, -10.0, 1.5, 100.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY];

    let mut dst = vec![[0.3f32, 0.4, 0.5, 0.6]; 16];
    let src = vec![[0.7f32, 0.2, 0.1, 0.8]; 16];

    for &mode in &ALL_BLEND_MODES {
        let bmode = to_blitter_blend_mode(mode);
        for &op in &test_values {
            for &fill in &test_values {
                for &gamma in &[1.0f32, 1.45, 2.2, -1.0, f32::NAN] {
                    for hdr in [false, true] {
                        for clipping in [false, true] {
                            for alpha_mode in [craft_blitter::AlphaMode::Straight, craft_blitter::AlphaMode::Premultiplied] {
                                craft_blitter::blit_rgba32f_opts(&mut dst, &src, bmode, op, fill, gamma, hdr, alpha_mode, clipping);
                            }
                        }
                    }
                }
            }
        }
    }

    // Finite fixture validation with hand-derived expected pixels
    let mut finite_dst = vec![[0.2f32, 0.4, 0.6, 1.0]; 4];
    let finite_src = vec![[0.8f32, 0.2, 0.4, 1.0]; 4];

    // Normal at 50% opacity: 0.2*0.5 + 0.8*0.5 = 0.5; 0.4*0.5 + 0.2*0.5 = 0.3; 0.6*0.5 + 0.4*0.5 = 0.5
    craft_blitter::blit_rgba32f_opts(
        &mut finite_dst,
        &finite_src,
        craft_blitter::BlendMode::Normal,
        0.5,
        1.0,
        1.0,
        false,
        craft_blitter::AlphaMode::Straight,
        false,
    );
    for p in &finite_dst {
        assert!((p[0] - 0.5).abs() < 1e-5);
        assert!((p[1] - 0.3).abs() < 1e-5);
        assert!((p[2] - 0.5).abs() < 1e-5);
        assert!((p[3] - 1.0).abs() < 1e-5);
    }

    // Multiply at 100% opacity on 0.5 grey: 0.5 * 0.8 = 0.40, 0.3 * 0.2 = 0.06, 0.5 * 0.4 = 0.20
    craft_blitter::blit_rgba32f_opts(
        &mut finite_dst,
        &finite_src,
        craft_blitter::BlendMode::Multiply,
        1.0,
        1.0,
        1.0,
        false,
        craft_blitter::AlphaMode::Straight,
        false,
    );
    for p in &finite_dst {
        assert!((p[0] - 0.5 * 0.8).abs() < 1e-5);
        assert!((p[1] - 0.3 * 0.2).abs() < 1e-5);
        assert!((p[2] - 0.5 * 0.4).abs() < 1e-5);
    }

    // Clipping isolation: destination alpha must be preserved exactly
    let mut clipped_dst = vec![[0.5f32, 0.5, 0.5, 0.35]; 4];
    craft_blitter::blit_rgba32f_opts(
        &mut clipped_dst,
        &finite_src,
        craft_blitter::BlendMode::Normal,
        1.0,
        1.0,
        1.0,
        false,
        craft_blitter::AlphaMode::Straight,
        true,
    );
    for p in &clipped_dst {
        assert!((p[3] - 0.35).abs() < 1e-6, "Clipping mask failed to preserve base alpha");
    }

    // Premultiplied alpha mode: Normal mode blending
    let mut pm_dst = vec![[0.2f32, 0.2, 0.2, 0.5]; 4];
    let pm_src = vec![[0.4f32, 0.4, 0.4, 0.5]; 4];
    craft_blitter::blit_rgba32f_opts(
        &mut pm_dst,
        &pm_src,
        craft_blitter::BlendMode::Normal,
        1.0,
        1.0,
        1.0,
        false,
        craft_blitter::AlphaMode::Premultiplied,
        false,
    );
    for p in &pm_dst {
        // Co = Cs + Cb*(1 - As) = 0.4 + 0.2*0.5 = 0.5
        // Ao = As + Ab*(1 - As) = 0.5 + 0.5*0.5 = 0.75
        assert!((p[0] - 0.5).abs() < 1e-5);
        assert!((p[3] - 0.75).abs() < 1e-5);
    }
}

#[test]
fn document_render_with_adversarial_layer_properties_never_panics() {
    // 1. Finite fixture verification with hand-derived expected pixels and full render vs render_tiled equivalence
    {
        let mut doc = empty_doc(8, 8);
        doc.layers.push(solid_layer("base", Rect::new(0, 0, 8, 8), [0.2, 0.4, 0.6, 1.0], BlendMode::Normal, 1.0, 1.0));
        doc.layers.push(solid_layer("top", Rect::new(0, 0, 8, 8), [0.8, 0.2, 0.4, 1.0], BlendMode::Normal, 0.5, 1.0));

        let buf = render(&doc, doc.bounds());
        let tiled = render_tiled(&doc, doc.bounds(), 4);

        assert_eq!(buf.px.len(), 64);
        assert_eq!(tiled.px.len(), 64);

        // Expected: 0.2 * 0.5 + 0.8 * 0.5 = 0.5, 0.4 * 0.5 + 0.2 * 0.5 = 0.3, 0.6 * 0.5 + 0.4 * 0.5 = 0.5, alpha = 1.0
        let expected = [0.5f32, 0.3, 0.5, 1.0];
        for (i, (&b_px, &t_px)) in buf.px.iter().zip(&tiled.px).enumerate() {
            for c in 0..4 {
                assert!((b_px[c] - expected[c]).abs() < 1e-5, "pixel {i} channel {c} mismatch in render: {} != {}", b_px[c], expected[c]);
                assert!((t_px[c] - expected[c]).abs() < 1e-5, "pixel {i} channel {c} mismatch in render_tiled: {} != {}", t_px[c], expected[c]);
            }
        }
    }

    // 2. Adversarial non-finite property sweep: compare render and render_tiled with NaN-aware comparison
    let adversarial_props = [
        (-1.0f32, 1.0f32),
        (2.5, 0.5),
        (1.0, -0.5),
        (0.5, 3.0),
        (f32::NAN, 1.0),
        (1.0, f32::NAN),
        (f32::NAN, f32::NAN),
        (f32::INFINITY, 1.0),
        (1.0, f32::NEG_INFINITY),
    ];

    for (op, fill) in adversarial_props {
        for &mode in &ALL_BLEND_MODES {
            let mut doc = empty_doc(8, 8);
            doc.layers.push(solid_layer("base", Rect::new(0, 0, 8, 8), [0.2, 0.4, 0.6, 0.8], BlendMode::Normal, 1.0, 1.0));
            doc.layers.push(solid_layer("top", Rect::new(0, 0, 8, 8), [0.9, 0.1, 0.3, 0.7], mode, op, fill));

            let buf = render(&doc, doc.bounds());
            let tiled = render_tiled(&doc, doc.bounds(), 4);
            assert_eq!(buf.px.len(), 64);
            assert_eq!(tiled.px.len(), 64);

            for (&b_px, &t_px) in buf.px.iter().zip(&tiled.px) {
                for c in 0..4 {
                    let bc = b_px[c];
                    let tc = t_px[c];
                    assert!(
                        (bc.is_nan() && tc.is_nan()) || (bc - tc).abs() < 1e-5,
                        "render vs render_tiled mismatch for mode {mode:?}, op {op}, fill {fill}: {bc} vs {tc}"
                    );
                }
            }
        }
    }
}

// =========================================================================
// 3. Transparent & Translucent Backdrops
// =========================================================================

#[test]
fn transparent_backdrop_compositing_correctness() {
    let transparent = [0.0f32, 0.0, 0.0, 0.0];
    let source = [0.6f32, 0.4, 0.2, 0.8];

    for &mode in &ALL_BLEND_MODES {
        let res = psblend::composite(mode, transparent, source, 1.0);
        // Over fully transparent backdrop, source color is preserved and alpha is as_
        assert!((res[3] - 0.8).abs() < 1e-5, "{mode:?} alpha failed: {res:?}");
        for c in 0..3 {
            assert!((res[c] - source[c]).abs() < 1e-4, "{mode:?} color channel {c} failed: {res:?}");
        }

        // At opacity 0.5
        let res_half = psblend::composite(mode, transparent, source, 0.5);
        assert!((res_half[3] - 0.4).abs() < 1e-5, "{mode:?} half alpha failed: {res_half:?}");
        for c in 0..3 {
            assert!((res_half[c] - source[c]).abs() < 1e-4, "{mode:?} half color channel {c} failed: {res_half:?}");
        }

        // At opacity 0.0 or source alpha 0.0 -> result must be fully transparent
        let res_zero = psblend::composite(mode, transparent, source, 0.0);
        assert_eq!(res_zero, transparent, "{mode:?} zero opacity failed");

        let res_src_trans = psblend::composite(mode, transparent, [0.5, 0.5, 0.5, 0.0], 1.0);
        assert_eq!(res_src_trans, transparent, "{mode:?} zero src alpha failed");
    }
}

#[test]
fn translucent_backdrop_porter_duff_alpha_math() {
    let backdrop = [0.2f32, 0.4, 0.6, 0.5];
    let source = [0.8f32, 0.3, 0.1, 0.4];

    // Formula: ao = as + ab * (1 - as)
    // as = 0.4 * 0.5 = 0.2
    // ab = 0.5
    // ao = 0.2 + 0.5 * (1 - 0.2) = 0.2 + 0.4 = 0.6
    let expected_ao = 0.6f32;

    for &mode in &ALL_BLEND_MODES {
        let res = psblend::composite(mode, backdrop, source, 0.5);
        assert!((res[3] - expected_ao).abs() < 1e-5, "{mode:?} ao {}: expected {}", res[3], expected_ao);
    }
}

// =========================================================================
// 4. Clipping Masks Under Extreme Conditions
// =========================================================================

#[test]
fn clipping_mask_over_fully_transparent_base_is_completely_invisible() {
    // Document with NO background (pure transparent)
    let mut doc = empty_doc(8, 8);

    // Base layer with 0.0 alpha
    let base = solid_layer("base", Rect::new(0, 0, 8, 8), [0.0, 0.0, 0.0, 0.0], BlendMode::Normal, 1.0, 1.0);
    doc.layers.push(base);

    // Highly visible clipped layer: bright red, 100% opacity
    let mut clip = solid_layer("clip", Rect::new(0, 0, 8, 8), [1.0, 0.0, 0.0, 1.0], BlendMode::Normal, 1.0, 1.0);
    clip.clipped = true;
    doc.layers.push(clip);

    let buf = render(&doc, doc.bounds());
    for (idx, p) in buf.px.iter().enumerate() {
        assert_eq!(*p, [0.0; 4], "pixel {idx} should be completely transparent, got {p:?}");
    }
}

#[test]
fn clipping_mask_over_translucent_base_strictly_preserves_base_alpha_8bit() {
    // Document with transparent background (8-bit)
    let mut doc = empty_doc(8, 8);

    // Base layer: blue with alpha 0.42 (quantizes to 107/255 = 0.41960785)
    let base_alpha = 0.42f32;
    let expected_quantized_alpha = (base_alpha * 255.0).round() / 255.0;
    let base = solid_layer("base", Rect::new(0, 0, 8, 8), [0.0, 0.0, 1.0, base_alpha], BlendMode::Normal, 1.0, 1.0);
    doc.layers.push(base);

    // Clipped layer: green with full opacity
    let mut clip = solid_layer("clip", Rect::new(0, 0, 8, 8), [0.0, 1.0, 0.0, 1.0], BlendMode::Normal, 1.0, 1.0);
    clip.clipped = true;
    doc.layers.push(clip);

    let buf = render(&doc, doc.bounds());
    for (idx, p) in buf.px.iter().enumerate() {
        assert!(
            (p[3] - expected_quantized_alpha).abs() < 1e-6,
            "pixel {idx} alpha must remain exactly expected_quantized_alpha ({expected_quantized_alpha}), got {}",
            p[3]
        );
        // Green should have replaced blue in RGB because Normal blend over opaque base would produce green
        assert!((p[1] - 1.0).abs() < 1e-4, "pixel {idx} green channel should be 1.0, got {}", p[1]);
        assert!(p[2] < 1e-4, "pixel {idx} blue channel should be 0.0, got {}", p[2]);
    }
}

#[test]
fn clipping_mask_over_translucent_base_strictly_preserves_base_alpha_f32() {
    // Document with transparent background (32-bit float HDR)
    let mut doc = empty_doc_f32(8, 8);

    // Base layer in RGBA32F: unquantized float alpha
    let base_alpha = 0.424242f32;
    let base = solid_layer_f32("base", Rect::new(0, 0, 8, 8), [0.0, 0.0, 1.0, base_alpha], BlendMode::Normal, 1.0, 1.0);
    doc.layers.push(base);

    // Clipped layer: green with full opacity
    let mut clip = solid_layer_f32("clip", Rect::new(0, 0, 8, 8), [0.0, 1.0, 0.0, 1.0], BlendMode::Normal, 1.0, 1.0);
    clip.clipped = true;
    doc.layers.push(clip);

    let buf = render(&doc, doc.bounds());
    for (idx, p) in buf.px.iter().enumerate() {
        assert!((p[3] - base_alpha).abs() < 1e-6, "pixel {idx} f32 alpha must remain bit-exact base_alpha ({base_alpha}), got {}", p[3]);
        assert!((p[1] - 1.0).abs() < 1e-6, "pixel {idx} green channel should be 1.0, got {}", p[1]);
        assert!(p[2] < 1e-6, "pixel {idx} blue channel should be 0.0, got {}", p[2]);
    }
}

#[test]
fn chained_clipping_masks_stack_over_translucent_base() {
    let mut doc = empty_doc_f32(8, 8);

    // Base layer: alpha = 0.5, red
    let base_alpha = 0.5f32;
    doc.layers.push(solid_layer_f32("base", Rect::new(0, 0, 8, 8), [1.0, 0.0, 0.0, base_alpha], BlendMode::Normal, 1.0, 1.0));

    // Clip 1: green, opacity 0.5
    let mut clip1 = solid_layer_f32("clip1", Rect::new(0, 0, 8, 8), [0.0, 1.0, 0.0, 1.0], BlendMode::Normal, 0.5, 1.0);
    clip1.clipped = true;
    doc.layers.push(clip1);

    // Clip 2: blue, Multiply
    let mut clip2 = solid_layer_f32("clip2", Rect::new(0, 0, 8, 8), [0.0, 0.0, 1.0, 1.0], BlendMode::Multiply, 1.0, 1.0);
    clip2.clipped = true;
    doc.layers.push(clip2);

    // Clip 3: white, Screen, opacity 0.3
    let mut clip3 = solid_layer_f32("clip3", Rect::new(0, 0, 8, 8), [1.0, 1.0, 1.0, 1.0], BlendMode::Screen, 0.3, 1.0);
    clip3.clipped = true;
    doc.layers.push(clip3);

    let buf = render(&doc, doc.bounds());
    for p in &buf.px {
        assert!((p[3] - base_alpha).abs() < 1e-6, "chained clipping stack must preserve base alpha {base_alpha}, got {}", p[3]);
    }
}

#[test]
fn clipping_masks_with_special_eight_fill_modes() {
    let special_modes = [
        BlendMode::ColorBurn,
        BlendMode::LinearBurn,
        BlendMode::ColorDodge,
        BlendMode::LinearDodge,
        BlendMode::VividLight,
        BlendMode::LinearLight,
        BlendMode::HardMix,
        BlendMode::Difference,
    ];

    for &mode in &special_modes {
        let mut doc = white_doc(8, 8);
        let base_rect = Rect::new(0, 0, 4, 8);
        doc.layers.push(solid_layer("base", base_rect, [0.3, 0.6, 0.8, 0.7], BlendMode::Normal, 1.0, 1.0));

        let mut clip = solid_layer("clip", Rect::new(0, 0, 8, 8), [0.9, 0.4, 0.2, 0.9], mode, 0.8, 0.5);
        clip.clipped = true;
        doc.layers.push(clip);

        let buf = render(&doc, doc.bounds());

        // In the unclipped area (x >= 4), only white background is visible
        for y in 0..8 {
            for x in 4..8 {
                let p = buf.get(x, y);
                assert!((p[0] - 1.0).abs() < 1e-3 && (p[1] - 1.0).abs() < 1e-3 && (p[2] - 1.0).abs() < 1e-3 && (p[3] - 1.0).abs() < 1e-3);
            }
        }
    }
}

// =========================================================================
// 5. Memory Safety & Concurrency Fuzzing
// =========================================================================

#[test]
fn multithreaded_concurrent_stress_preserves_tls_and_no_races() {
    use std::sync::Arc;
    use std::thread;

    let mut doc1 = white_doc(16, 16);
    doc1.layers.push(solid_layer("b1", Rect::new(0, 0, 16, 16), [0.2, 0.3, 0.4, 0.6], BlendMode::ColorDodge, 0.8, 0.6));
    let mut c1 = solid_layer("c1", Rect::new(0, 0, 16, 16), [0.9, 0.8, 0.7, 0.9], BlendMode::HardMix, 0.7, 0.4);
    c1.clipped = true;
    doc1.layers.push(c1);

    let doc1 = Arc::new(doc1);
    let mut handles = Vec::new();

    for _ in 0..8 {
        let d = Arc::clone(&doc1);
        handles.push(thread::spawn(move || {
            for _ in 0..50 {
                let b1 = render(&d, d.bounds());
                let b2 = render_tiled(&d, d.bounds(), 4);
                assert_eq!(b1.px, b2.px);
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }
}
