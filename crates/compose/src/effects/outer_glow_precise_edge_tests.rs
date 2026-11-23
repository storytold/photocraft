//! #15006 - Outer Glow: a flat-contour Precise band's outermost pixel is soft on a pixel-aligned
//! edge.
//!
//! Measured on the issue's document: 60 x 60 px, transparent, a black 20 x 20 square at x/y
//! 20..39. Outer Glow Normal 100 %, Precise, Spread 0, Size 4 px, Range 50 %, custom flat contour
//! (0,255)-(255,255), Anti-aliased off. Along row 30, x 14 and 15 read alpha 0, x 16 (4 px out)
//! 213 and x 17..19 255. On the diagonal, (16, 16) reads 0, (17, 17) 161 and (18, 18) 255.

use super::{Contour, Effect, FxPaint, Glow, GlowSource, GlowTechnique};
use photocraft_color::{BlendMode, Color, ColorMode, SampleType};
use photocraft_doc::{Document, FxCommon, Layer, adjust::CurvePoint};
use photocraft_geom::{Rect, Size};

fn outer_glow(contour: Contour) -> Glow {
    Glow {
        common: FxCommon::new(BlendMode::Normal, 1.0),
        paint: FxPaint::Color(Color::BLACK),
        technique: GlowTechnique::Precise,
        spread: 0.0,
        size: 4.0,
        contour,
        anti_alias: false,
        range: 0.5,
        jitter: 0.0,
        noise: 0.0,
        source: GlowSource::Edge,
    }
}

fn flat() -> Contour {
    Contour::Custom { name: "Flat".into(), points: vec![CurvePoint { input: 0.0, output: 1.0 }, CurvePoint { input: 1.0, output: 1.0 }], corners: vec![] }
}

fn alphas(glow: Glow) -> impl Fn(i32, i32) -> u8 {
    let mut doc = Document::new("Outer glow", Size::new(60, 60), ColorMode::Rgb, SampleType::U8);
    let mut layer = Layer::raster("Square", doc.pixel_format());
    layer.surface_mut().unwrap().fill_rect(Rect::new(20, 20, 40, 40), &[0.0, 0.0, 0.0, 1.0]);
    layer.effects.items.push(Effect::OuterGlow(glow));
    doc.layers.push(layer);
    let image = crate::flatten(&doc);
    move |x, y| (image.get(x, y)[3] * 255.0).round() as u8
}

#[test]
fn the_pixel_at_size_px_out_is_soft() {
    let alpha = alphas(outer_glow(flat()));
    for (x, expected) in [(14, 0), (15, 0), (17, 255), (18, 255), (19, 255)] {
        assert_eq!(alpha(x, 30), expected, "x {x}");
    }
    let edge = alpha(16, 30);
    assert!(edge.abs_diff(213) <= 3, "x 16, 4 px out: expected 213, got {edge}");
}

#[test]
fn the_corner_diagonal_fades_the_same_way() {
    let alpha = alphas(outer_glow(flat()));
    assert_eq!(alpha(16, 16), 0);
    assert_eq!(alpha(18, 18), 255);
    let diagonal = alpha(17, 17);
    assert!(diagonal.abs_diff(161) <= 5, "(17, 17), √18 px out: expected 161, got {diagonal}");
}
