//! Tier 5 Adversarial Test Suite for photocraft-compose.
//!
//! White-box stress testing of:
//! 1. Nested clipping groups over transparent and translucent base.
//! 2. Unlinked masks during translation and sampling.
//! 3. Zero-size damage rects, inverted rects, and out-of-bounds queries.

use photocraft_color::{BlendMode, Color, ColorMode, PixelFormat, SampleType};
use photocraft_compose::{render, render_tiled};
use photocraft_doc::{Document, Layer, LayerMask};
use photocraft_geom::{Rect, Size};
use photocraft_raster::Surface;

fn solid_layer(name: &str, rect: Rect, rgba: [f32; 4], mode: BlendMode, opacity: f32, fill: f32) -> Layer {
    let mut l = Layer::raster(name, PixelFormat::RGBA8);
    l.blend = mode;
    l.opacity = opacity;
    l.fill_opacity = fill;
    l.surface_mut().unwrap().fill_rect(rect, &rgba);
    l
}

fn empty_doc(w: u32, h: u32) -> Document {
    Document::new("test", Size::new(w, h), ColorMode::Rgb, SampleType::U8)
}

#[test]
fn test_nested_clipping_group_over_transparent_base_pure_invariance() {
    let mut doc = empty_doc(16, 16);

    // Base layer with alpha = 0.0 (transparent)
    let base = solid_layer("Base", Rect::new(0, 0, 16, 16), [0.0, 0.0, 0.0, 0.0], BlendMode::Normal, 1.0, 1.0);

    // Chained clipped layers with bright colors and various blend modes
    let mut clip1 = solid_layer("Clip1_Red", Rect::new(0, 0, 16, 16), [1.0, 0.0, 0.0, 1.0], BlendMode::Normal, 1.0, 1.0);
    clip1.clipped = true;

    let mut clip2 = solid_layer("Clip2_Green", Rect::new(0, 0, 16, 16), [0.0, 1.0, 0.0, 1.0], BlendMode::Multiply, 1.0, 1.0);
    clip2.clipped = true;

    let mut clip3 = solid_layer("Clip3_Blue", Rect::new(0, 0, 16, 16), [0.0, 0.0, 1.0, 1.0], BlendMode::Screen, 1.0, 1.0);
    clip3.clipped = true;

    let mut clip4 = solid_layer("Clip4_Vivid", Rect::new(0, 0, 16, 16), [1.0, 1.0, 0.0, 1.0], BlendMode::VividLight, 0.8, 0.5);
    clip4.clipped = true;

    // Group 1: isolated group containing a transparent base and chained clipped layers
    let g1 = Layer::group("Group1", vec![base, clip1, clip2, clip3, clip4]);
    doc.layers.push(g1);

    // Top layer clipped onto Group 1
    let mut top_clip = solid_layer("TopClip_Magenta", Rect::new(0, 0, 16, 16), [1.0, 0.0, 1.0, 1.0], BlendMode::Difference, 1.0, 1.0);
    top_clip.clipped = true;
    doc.layers.push(top_clip);

    let buf = render(&doc, doc.bounds());
    for (idx, p) in buf.px.iter().enumerate() {
        assert_eq!(*p, [0.0; 4], "Pixel {idx} leaked color over transparent base: {p:?}");
    }

    // Verify tiled rendering produces identical result
    for tile in [1, 2, 4, 16] {
        let tiled = render_tiled(&doc, doc.bounds(), tile);
        assert_eq!(tiled.px, buf.px, "Tiled mismatch at tile size {tile}");
    }
}

#[test]
fn test_nested_clipping_group_over_translucent_base_preserves_alpha() {
    let mut doc = empty_doc(8, 8);

    let base_alpha = 0.35f32;
    let expected_alpha = (base_alpha * 255.0).round() / 255.0;

    let base = solid_layer("Base", Rect::new(0, 0, 8, 8), [0.2, 0.4, 0.6, base_alpha], BlendMode::Normal, 1.0, 1.0);
    doc.layers.push(base);

    let mut clip1 = solid_layer("Clip1", Rect::new(0, 0, 8, 8), [0.8, 0.1, 0.3, 1.0], BlendMode::Overlay, 0.9, 0.8);
    clip1.clipped = true;
    doc.layers.push(clip1);

    let mut clip2 = solid_layer("Clip2", Rect::new(0, 0, 8, 8), [0.1, 0.9, 0.4, 0.8], BlendMode::Difference, 0.7, 0.6);
    clip2.clipped = true;
    doc.layers.push(clip2);

    let buf = render(&doc, doc.bounds());
    for (idx, p) in buf.px.iter().enumerate() {
        assert!((p[3] - expected_alpha).abs() < 1e-5, "Pixel {idx} alpha diverted from base: got {}, expected {}", p[3], expected_alpha);
    }
}

#[test]
fn test_unlinked_mask_stationary_during_layer_translation() {
    let mut doc = Document::with_background("test", Size::new(16, 16), ColorMode::Rgb, SampleType::U8, Color::WHITE);

    // Layer with 10x10 red square
    let content_rect = Rect::new(0, 0, 10, 10);
    let mut layer = solid_layer("Art", content_rect, [1.0, 0.0, 0.0, 1.0], BlendMode::Normal, 1.0, 1.0);

    // Mask covering 5x5 at (0, 0)
    let mask_fmt = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false);
    let mut mask_surf = Surface::new(mask_fmt);
    mask_surf.fill_rect(Rect::new(0, 0, 5, 5), &[1.0]);
    layer.mask = Some(LayerMask {
        surface: mask_surf,
        enabled: true,
        linked: false, // Unlinked!
        density: 1.0,
        feather: 0.0,
    });

    // Translate layer pixels by (5, 5) while mask is unlinked
    if let Some(surf) = layer.surface_mut() {
        *surf = surf.translated(5, 5, content_rect);
    }

    doc.layers.push(layer);

    let buf = render(&doc, doc.bounds());

    // Intersection of moved pixels [5, 15) and stationary mask [0, 5) is EMPTY!
    // Therefore all pixels should be white background
    for y in 0..16 {
        for x in 0..16 {
            let p = buf.get(x, y);
            assert!(
                (p[0] - 1.0).abs() < 1e-3 && (p[1] - 1.0).abs() < 1e-3 && (p[2] - 1.0).abs() < 1e-3,
                "Pixel ({x}, {y}) should be white background, got {p:?}"
            );
        }
    }
}

#[test]
fn test_zero_size_damage_rects_and_empty_bounds() {
    let mut doc = empty_doc(16, 16);
    doc.layers.push(solid_layer("L1", Rect::new(0, 0, 16, 16), [0.5, 0.5, 0.5, 1.0], BlendMode::Normal, 1.0, 1.0));

    let zero_and_extreme_rects = [
        Rect::new(0, 0, 0, 0),
        Rect::new(8, 8, 8, 8),
        Rect::new(0, 0, 16, 0),
        Rect::new(0, 0, 0, 16),
        Rect::new(16, 16, 8, 8),       // inverted
        Rect::new(-20, -20, -10, -10), // negative outside
        Rect::new(100, 100, 200, 200), // positive outside
    ];

    for r in zero_and_extreme_rects {
        let b = render(&doc, r);
        if r.is_empty() {
            assert!(b.px.is_empty(), "Empty rect {r:?} must produce empty buffer");
        }
        for tile in [1, 2, 7, 256] {
            let bt = render_tiled(&doc, r, tile);
            assert_eq!(b.px, bt.px, "Tiled vs direct mismatch on rect {r:?}");
        }
    }
}
