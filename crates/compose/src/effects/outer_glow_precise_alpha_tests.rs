use super::{Contour, Effect, FxPaint, Glow, GlowSource, GlowTechnique};
use photocraft_color::{BlendMode, Color, ColorMode, SampleType};
use photocraft_doc::adjust::CurvePoint;
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
    let mut doc = Document::with_background("Outer Glow", Size::new(240, 120), ColorMode::Rgb, depth, Color::WHITE);
    let mut layer = Layer::raster("Squares", doc.pixel_format());
    let surface = layer.surface_mut().unwrap();
    for (x, alpha) in [(20, 1.0), (100, 0.5), (180, 0.25)] {
        surface.fill_rect(Rect::new(x, 40, x + 40, 80), &[1.0, 128.0 / 255.0, 0.0, alpha]);
    }
    layer.effects.items.push(Effect::OuterGlow(effect));
    doc.layers.push(layer);
    crate::flatten(&doc)
}

fn rgb(image: &crate::Buffer, x: i32, y: i32) -> [u8; 3] {
    let p = image.get(x, y);
    std::array::from_fn(|i| (p[i] * 255.0).round() as u8)
}

#[test]
fn precise_outer_glow_attenuates_under_translucent_squares() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let image = squares(depth, glow(GlowTechnique::Precise));
        let actual = [rgb(&image, 120, 60), rgb(&image, 200, 60)];
        let measured = [[139, 75, 11], [89, 57, 25]];
        let tolerance = if depth == SampleType::U8 { 0 } else { 3 };
        assert!(actual.iter().flatten().zip(measured.iter().flatten()).all(|(a, b)| a.abs_diff(*b) <= tolerance), "{depth:?}: {actual:?}");
    }
}

#[test]
fn precise_outer_glow_keeps_the_opaque_square_and_exterior_ramps() {
    let image = squares(SampleType::U8, glow(GlowTechnique::Precise));
    assert_eq!(rgb(&image, 40, 60), [255, 128, 0]);
    for (x, expected) in [(60, [42, 85, 128, 170, 212]), (140, [64, 106, 149, 191, 234]), (220, [74, 117, 159, 202, 244])] {
        let actual: [u8; 5] = std::array::from_fn(|i| rgb(&image, x + i as i32, 60)[0]);
        assert!(actual.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1), "exterior at x={x}: got {actual:?}, expected {expected:?}");
    }
}

#[test]
fn softer_and_other_precise_settings_keep_the_translucent_centers() {
    let precise = glow(GlowTechnique::Precise);
    let softer = squares(SampleType::U8, glow(GlowTechnique::Softer));
    assert_eq!(rgb(&softer, 120, 60), [128, 64, 0]);
    assert_eq!(rgb(&softer, 200, 60), [159, 127, 95]);
    for effect in [
        Glow { size: 6.0, ..precise.clone() },
        Glow { spread: 0.25, ..precise.clone() },
        Glow { range: 0.65, ..precise.clone() },
        Glow { anti_alias: true, ..precise.clone() },
        Glow {
            contour: Contour::Custom {
                name: "Identity".into(),
                points: vec![CurvePoint { input: 0.0, output: 0.0 }, CurvePoint { input: 1.0, output: 1.0 }],
                corners: vec![],
            },
            ..precise.clone()
        },
        Glow { common: FxCommon::new(BlendMode::Multiply, 1.0), ..precise },
    ] {
        let image = squares(SampleType::U8, effect);
        assert_eq!(rgb(&image, 120, 60), [128, 64, 0]);
        assert_eq!(rgb(&image, 200, 60), [64, 32, 0]);
    }
}

#[test]
fn precise_outer_glow_keeps_the_coverage_at_an_antialiased_edge() {
    let mut shape = super::Map::new(9, 9, 0.0);
    for row in shape.v.chunks_exact_mut(shape.w) {
        row[3] = 0.25;
        row[4] = 0.5;
        row[5..].fill(1.0);
    }
    let map = super::glow_map(&shape, &glow(GlowTechnique::Precise), false, (0, 0));
    assert_eq!(map.get(3, 4), 1.0);
    assert_eq!(map.get(4, 4), 1.0);
}

#[test]
fn precise_outer_glow_handles_empty_and_single_pixel_shapes() {
    let effect = glow(GlowTechnique::Precise);
    let mut shape = super::Map::new(9, 9, 0.0);
    let empty = super::glow_map(&shape, &effect, false, (0, 0));
    assert!(empty.v.iter().all(|&alpha| alpha == 0.0));

    shape.v[4 * 9 + 4] = 0.5;
    let single = super::glow_map(&shape, &effect, false, (0, 0));
    assert_eq!(single.get(4, 4), 1.0);
    assert!((single.get(5, 4) - 0.75).abs() < 1e-6);
}
