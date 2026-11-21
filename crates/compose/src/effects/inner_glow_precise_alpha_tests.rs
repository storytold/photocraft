use super::{Contour, Effect, FxPaint, Glow, GlowSource, GlowTechnique};
use photocraft_color::{BlendMode, Color, ColorMode, SampleType};
use photocraft_doc::{Document, FxCommon, Layer};
use photocraft_geom::{Rect, Size};

fn glow(technique: GlowTechnique) -> Glow {
    Glow {
        common: FxCommon::new(BlendMode::Normal, 1.0),
        paint: FxPaint::Color(Color::BLACK),
        technique,
        spread: 0.0,
        size: 5.0,
        contour: Contour::Linear,
        anti_alias: false,
        range: 0.5,
        jitter: 0.0,
        noise: 0.0,
        source: GlowSource::Edge,
    }
}

fn squares(depth: SampleType, effect: Glow) -> crate::Buffer {
    let mut doc = Document::with_background("Inner Glow", Size::new(240, 120), ColorMode::Rgb, depth, Color::WHITE);
    let mut layer = Layer::raster("Squares", doc.pixel_format());
    let surface = layer.surface_mut().unwrap();
    for (x, alpha) in [(20, 1.0), (100, 0.5), (180, 0.25)] {
        surface.fill_rect(Rect::new(x, 40, x + 40, 80), &[1.0, 128.0 / 255.0, 0.0, alpha]);
    }
    layer.effects.items.push(Effect::InnerGlow(effect));
    doc.layers.push(layer);
    crate::flatten(&doc)
}

fn rgb(image: &crate::Buffer, x: i32, y: i32) -> [u8; 3] {
    let p = image.get(x, y);
    std::array::from_fn(|i| (p[i] * 255.0).round() as u8)
}

fn agrees(actual: [u8; 3], expected: [u8; 3], tolerance: u8) -> bool {
    actual.into_iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= tolerance)
}

#[test]
fn precise_inner_glow_fills_each_translucent_square_evenly() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let image = squares(depth, glow(GlowTechnique::Precise));
        for (x, expected) in [(120, [138, 133, 127]), (200, [194, 193, 191])] {
            let first = rgb(&image, x, 40);
            let row: Vec<_> = (40..46).map(|y| rgb(&image, x, y)).collect();
            let tolerance = if depth == SampleType::U8 { 0 } else { 3 };
            assert!(agrees(first, expected, tolerance), "{depth:?} square at x={x}: {row:?}");
            assert!((40..80).all(|y| rgb(&image, x, y) == first), "{depth:?} square at x={x} has an inward fade: {row:?}");
            for y in [40, 60, 79] {
                for x in x - 20..x + 20 {
                    assert_eq!(rgb(&image, x, y), first, "{depth:?} point ({x}, {y})");
                }
            }
        }
    }
}

#[test]
fn inner_glow_keeps_the_opaque_profile_and_softer_controls() {
    let precise = squares(SampleType::U8, glow(GlowTechnique::Precise));
    let expected = [[43, 22, 0], [86, 43, 0], [128, 64, 0], [170, 85, 0], [213, 107, 0], [255, 128, 0]];
    for (y, expected) in (40..46).zip(expected) {
        let actual = rgb(&precise, 40, y);
        assert!(agrees(actual, expected, 1), "opaque profile at y={y}: {actual:?}");
    }
    let softer = squares(SampleType::U8, glow(GlowTechnique::Softer));
    for (x, expected) in [
        (40, [[51, 26, 0], [133, 67, 0], [194, 97, 0], [235, 118, 0], [255, 128, 0], [255, 128, 0]]),
        (120, [[127, 127, 127], [127, 127, 127], [127, 127, 127], [127, 127, 127], [128, 127, 127], [128, 127, 127]]),
        (200, [[191, 191, 191]; 6]),
    ] {
        for (y, expected) in (40..46).zip(expected) {
            assert_eq!(rgb(&softer, x, y), expected, "Softer at ({x}, {y})");
        }
    }
}

#[test]
fn other_precise_settings_keep_their_profiles() {
    let precise = glow(GlowTechnique::Precise);
    for effect in [
        Glow { size: 6.0, ..precise.clone() },
        Glow { spread: 0.25, ..precise.clone() },
        Glow { range: 0.65, ..precise.clone() },
        Glow { anti_alias: true, ..precise.clone() },
    ] {
        let image = squares(SampleType::U8, effect);
        assert_eq!(rgb(&image, 120, 60), [255, 191, 127]);
        assert_eq!(rgb(&image, 200, 60), [255, 223, 191]);
    }
    let center = squares(SampleType::U8, Glow { source: GlowSource::Center, ..precise });
    assert_eq!(rgb(&center, 120, 60), [127, 127, 127]);
    assert_eq!(rgb(&center, 200, 60), [191, 191, 191]);
}

#[test]
fn precise_inner_glow_preserves_transitions_and_isolated_pixels() {
    let mut shape = super::Map::new(9, 9, 0.0);
    shape.v[0] = 0.5;
    for row in shape.v.chunks_exact_mut(shape.w).skip(2) {
        row[3] = 0.25;
        row[4] = 0.5;
        row[5..].fill(1.0);
    }
    let precise = glow(GlowTechnique::Precise);
    let original = super::glow_map(&shape, &Glow { anti_alias: true, ..precise.clone() }, true, (0, 0));
    let map = super::glow_map(&shape, &precise, true, (0, 0));
    assert_eq!(map.v, original.v);
}
