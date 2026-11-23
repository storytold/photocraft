use super::{Contour, Effect, FxPaint, Glow, GlowSource, GlowTechnique};
use photocraft_color::{BlendMode, Color, ColorMode, SampleType};
use photocraft_doc::{Document, FxCommon, Layer};
use photocraft_geom::{Rect, Size};

fn inner_glow(size: f32) -> Glow {
    Glow {
        common: FxCommon::new(BlendMode::Normal, 1.0),
        paint: FxPaint::Color(Color::BLACK),
        technique: GlowTechnique::Softer,
        spread: 0.0,
        size,
        contour: Contour::Linear,
        anti_alias: false,
        range: 0.5,
        jitter: 0.0,
        noise: 0.0,
        source: GlowSource::Edge,
    }
}

fn fixture(mode: ColorMode, depth: SampleType, background: Color, fill: &[f32], glow: Glow) -> Document {
    let mut doc = Document::with_background("Inner glow", Size::new(400, 300), mode, depth, background);
    let mut layer = Layer::raster("Rectangle", doc.pixel_format());
    layer.surface_mut().unwrap().fill_rect(Rect::new(100, 75, 300, 225), fill);
    layer.effects.items.push(Effect::InnerGlow(glow));
    doc.layers.push(layer);
    doc
}

fn orange(glow: Glow) -> crate::Buffer {
    crate::flatten(&fixture(ColorMode::Rgb, SampleType::U8, Color::WHITE, &[1.0, 128.0 / 255.0, 0.0, 1.0], glow))
}

fn green(image: &crate::Buffer, x: i32, y: i32) -> u8 {
    (image.get(x, y)[1] * 255.0).round() as u8
}

#[test]
fn size_one_draws_the_measured_pixel_on_all_four_edges() {
    let image = orange(inner_glow(1.0));
    for ((x, y), (nx, ny)) in [((200, 75), (200, 76)), ((200, 224), (200, 223)), ((100, 150), (101, 150)), ((299, 150), (298, 150))] {
        let edge = green(&image, x, y);
        assert!(edge.abs_diff(64) <= 2, "edge ({x}, {y}): expected green 64, got {edge}");
        assert_eq!(green(&image, nx, ny), 128, "the glow stops after the first pixel");
    }
    assert_eq!(green(&image, 200, 150), 128);
    assert_eq!(green(&image, 200, 74), 255, "the glow stays inside the layer");
}

#[test]
fn other_sizes_keep_their_profiles() {
    for (size, profile) in [(0.0, &[128][..]), (3.0, &[43, 100, 128]), (10.0, &[13, 36, 57, 74, 90, 102, 113, 120, 125, 128]), (20.0, &[6])] {
        let image = orange(inner_glow(size));
        for (offset, &expected) in profile.iter().enumerate() {
            let actual = green(&image, 200, 75 + offset as i32);
            assert!(actual.abs_diff(expected) <= 2, "size {size}, offset {offset}: expected green {expected}, got {actual}");
        }
        assert_eq!(green(&image, 200, 75 + size as i32), 128);
    }
}

#[test]
fn other_size_one_settings_keep_their_profiles() {
    let precise = orange(Glow { technique: GlowTechnique::Precise, ..inner_glow(1.0) });
    assert_eq!(green(&precise, 200, 75), 64);
    assert_eq!(green(&precise, 200, 76), 128);

    let center = orange(Glow { source: GlowSource::Center, ..inner_glow(1.0) });
    assert_eq!(green(&center, 200, 75), 0);
    assert_eq!(green(&center, 200, 150), 0);

    let choke = orange(Glow { spread: 0.25, ..inner_glow(1.0) });
    assert_eq!(green(&choke, 200, 75), 128);
}

#[test]
fn size_one_uses_the_document_color_and_depth() {
    let gray =
        fixture(ColorMode::Grayscale, SampleType::U16, Color::gray(1.0), &[0.5, 1.0], Glow { paint: FxPaint::Color(Color::gray(0.0)), ..inner_glow(1.0) });
    let gray = crate::flatten_to_surface(&gray, gray.pixel_format(), None);
    assert!((gray.sample_channel(200, 75, 0) - 0.25).abs() < 0.0001);
    assert!((gray.sample_channel(200, 76, 0) - 0.5).abs() < 0.0001);

    let ink = |k| Color { mode: ColorMode::Cmyk, c: [0.0, 0.0, 0.0, k], alpha: 1.0 };
    let cmyk = fixture(ColorMode::Cmyk, SampleType::U16, ink(0.0), &[0.0, 0.0, 0.0, 0.25, 1.0], Glow { paint: FxPaint::Color(ink(1.0)), ..inner_glow(1.0) });
    assert!(crate::cmyk::split::supports(&cmyk));
    let cmyk = crate::flatten_to_surface(&cmyk, cmyk.pixel_format(), None);
    assert!((cmyk.sample_channel(200, 75, 3) - 0.625).abs() < 0.0001);
    assert!((cmyk.sample_channel(200, 76, 3) - 0.25).abs() < 0.0001);
    for ch in 0..3 {
        assert_eq!(cmyk.sample_channel(200, 75, ch), 0.0);
    }
}
