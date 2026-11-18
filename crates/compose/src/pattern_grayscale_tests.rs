//! #14975: in a Grayscale document a colour pattern paints the greys of the document's grey
//! profile, as measured in the reference application (blue 69, green 221, red 129 of 255), not the
//! darker luma-formula greys (29 / 150 / 76). RGB, Lab, 16-bit and 32-bit documents keep the
//! pattern's own colours.

use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Effect, FxCommon, Layer, Pattern};
use photocraft_geom::{Rect, Size};

use super::pattern_rgba;
use crate::render;

/// The issue's test pattern: 20 x 20 px, quadrants red (top left), green (top right), blue
/// (bottom left) and black (bottom right).
fn quadrants() -> Pattern {
    let mut s = photocraft_raster::Surface::new(PixelFormat::RGBA8);
    s.fill_rect(Rect::new(0, 0, 10, 10), &[1.0, 0.0, 0.0, 1.0]);
    s.fill_rect(Rect::new(10, 0, 20, 10), &[0.0, 1.0, 0.0, 1.0]);
    s.fill_rect(Rect::new(0, 10, 10, 20), &[0.0, 0.0, 1.0, 1.0]);
    s.fill_rect(Rect::new(10, 10, 20, 20), &[0.0, 0.0, 0.0, 1.0]);
    Pattern::new("quadrants", s, 20, 20)
}

/// The measured greys (of 255) of the red, green and blue cells.
const MEASURED: [(&str, (i32, i32), f32); 3] = [("red", (5, 5), 129.0), ("green", (15, 5), 221.0), ("blue", (5, 15), 69.0)];

/// A 20 x 20 document of `mode` and `depth` with a white Background and a layer covering it that
/// carries the quadrants as a Normal 100 % Pattern Overlay.
fn overlay_doc(mode: ColorMode, depth: SampleType) -> Document {
    let mut d = Document::with_background("t", Size::new(20, 20), mode, depth, Color::WHITE);
    let pat = quadrants();
    let fmt = d.pixel_format();
    let mut l = Layer::raster("overlay", fmt);
    // An opaque mid grey, encoded in the document's own channels.
    l.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 20, 20), &photocraft_raster::from_rgba(&fmt, [0.5, 0.5, 0.5, 1.0]));
    l.effects.enabled = true;
    l.effects.items.push(Effect::PatternOverlay {
        common: FxCommon::new(photocraft_color::BlendMode::Normal, 1.0),
        name: pat.name.clone(),
        id: pat.id.clone(),
        scale: 1.0,
        angle: 0.0,
        link: true,
        phase: (0.0, 0.0),
    });
    d.patterns.push(pat);
    d.layers.push(l);
    d
}

fn at(d: &Document, (x, y): (i32, i32)) -> [f32; 4] {
    render(d, Rect::from_xywh(x, y, 1, 1)).px[0]
}

#[test]
fn a_colour_pattern_converts_to_the_grey_profiles_greys() {
    let px = pattern_rgba(&quadrants(), ColorMode::Grayscale);
    let mut wrong = Vec::new();
    for (cell, (x, y), want) in MEASURED {
        let got = px[(y * 20 + x) as usize];
        if (got[0] * 255.0 - want).abs() > 2.0 || got[0] != got[1] || got[1] != got[2] || got[3] != 1.0 {
            wrong.push(format!("{cell}: {:?} (grey {:.1}), want {want}", got, got[0] * 255.0));
        }
    }
    // Black stays black.
    let black = px[15 * 20 + 15];
    if black[0] * 255.0 > 0.5 {
        wrong.push(format!("black: {black:?}"));
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[test]
fn a_pattern_overlay_in_a_grayscale_document_paints_the_measured_greys() {
    // A document created in Grayscale, and one whose mode was set to Grayscale (the state Image ›
    // Mode › Grayscale leaves): the overlay is drawn from the same conversion.
    let mut wrong = Vec::new();
    for depth in [SampleType::U8, SampleType::U16] {
        let d = overlay_doc(ColorMode::Grayscale, depth);
        for (cell, xy, want) in MEASURED {
            let got = at(&d, xy);
            if (got[0] * 255.0 - want).abs() > 2.0 {
                wrong.push(format!("{depth:?} {cell}: grey {:.1}, want {want}", got[0] * 255.0));
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[test]
fn rgb_lab_16_and_32_bit_documents_keep_the_patterns_colours() {
    let cells = [((5, 5), [1.0, 0.0, 0.0]), ((15, 5), [0.0, 1.0, 0.0]), ((5, 15), [0.0, 0.0, 1.0]), ((15, 15), [0.0, 0.0, 0.0])];
    for mode in [ColorMode::Rgb, ColorMode::Lab] {
        assert_eq!(pattern_rgba(&quadrants(), mode), pattern_rgba(&quadrants(), ColorMode::Rgb), "{mode:?}: the pattern's own colours");
    }
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let d = overlay_doc(ColorMode::Rgb, depth);
        for (xy, rgb) in cells {
            let got = at(&d, xy);
            assert!(got.iter().zip(rgb).all(|(g, w)| (g - w).abs() < 0.01), "RGB {depth:?} at {xy:?}: {got:?}, want {rgb:?}");
        }
    }
}
