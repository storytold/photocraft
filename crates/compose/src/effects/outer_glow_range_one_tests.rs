//! #14958: the Outer Glow Range 1 % profile on the measured document.
//!
//! Test document: 400 x 300 px, 72 ppi, RGB 8 bit, white Background, pixel layer "Shape" with an
//! orange (255,128,0) rectangle at x 100..299, y 75..224. Outer Glow black, Normal 100 %, Softer,
//! Spread 0, Size 10, Contour Linear, Anti-aliased off, Noise 0. Profiles list the R channel of
//! the flattened image from the layer edge outwards (255 = no glow, 0 = full black glow); the
//! reference application gives `0 x8, 39, 255` at Range 1 % on all four edges (within 3 levels).
//! Range 25/50/100 keep today's profiles (the measured values, within 4 levels). #15871 measured
//! Range 2 and Sizes 20 and 5 at Range 1 on the same document (within 3 levels). The cases still
//! *not measured* (Precise at Range 1, Range 1 with Spread, a custom contour at Range 1, the
//! Inner Glow Center source at Range 1) pin today's output. The inner test mirrors #14929's
//! measured Inner Glow Range-1 profile `0 x8, 19, 128` as a cross-check; it does not claim that
//! issue's item.

use super::{Contour, Effect, FxPaint, Glow, GlowSource, GlowTechnique, Rect};
use photocraft_color::{BlendMode, Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, FxCommon, Layer};
use photocraft_geom::Size;

/// The measured glow with `range` (0..=1): Size 10, Spread 0, Softer, Edge, Linear contour.
fn glow(range: f32) -> Glow {
    Glow {
        common: FxCommon::new(BlendMode::Normal, 1.0),
        paint: FxPaint::Color(Color::BLACK),
        technique: GlowTechnique::Softer,
        spread: 0.0,
        size: 10.0,
        contour: Contour::Linear,
        anti_alias: false,
        range,
        jitter: 0.0,
        noise: 0.0,
        source: GlowSource::Edge,
    }
}

/// The measured document with one glow effect on the orange rectangle; `inner` for an Inner Glow.
fn measured_doc(g: Glow, inner: bool) -> Document {
    let mut d = Document::with_background("range one", Size::new(400, 300), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut l = Layer::raster("Shape", PixelFormat::RGBA8);
    l.surface_mut().unwrap().fill_rect(Rect::new(100, 75, 300, 225), &[1.0, 128.0 / 255.0, 0.0, 1.0]);
    l.effects.items = vec![if inner { Effect::InnerGlow(g) } else { Effect::OuterGlow(g) }];
    d.layers.push(l);
    d
}

/// The pixel `d` (1-based) out from edge `e` (0 top, 1 left, 2 bottom, 3 right) of the rectangle.
fn out_point(e: usize, d: usize) -> (i32, i32) {
    let d = d as i32;
    match e {
        0 => (200, 75 - d),
        1 => (100 - d, 150),
        2 => (200, 224 + d),
        _ => (299 + d, 150),
    }
}

/// The pixel `d` (1-based) in from edge `e`, for the inner-glow checks.
fn in_point(e: usize, d: usize) -> (i32, i32) {
    let d = d as i32;
    match e {
        0 => (200, 74 + d),
        1 => (99 + d, 150),
        2 => (200, 225 - d),
        _ => (300 - d, 150),
    }
}

/// The flattened 8-bit level of channel `ch` at `(x, y)` in the already-rendered buffer `b`.
fn level(b: &crate::Buffer, x: i32, y: i32, ch: usize) -> i32 {
    let w = b.rect.width() as usize;
    (255.0 * b.px[(y - b.rect.y0) as usize * w + (x - b.rect.x0) as usize][ch]).round() as i32
}

/// Compares the `want.len()` pixels from each of the four edges along channel `ch` (inwards for
/// inner glows) against `want`, level by level, within `tol`.
fn assert_all_edges(doc: &Document, ch: usize, inward: bool, want: &[i32], tol: i32, what: &str) {
    let b = crate::render(doc, doc.bounds());
    for e in 0..4 {
        for (d, &want) in want.iter().enumerate() {
            let (x, y) = if inward { in_point(e, d + 1) } else { out_point(e, d + 1) };
            let got = level(&b, x, y, ch);
            assert!(got - want <= tol && want - got <= tol, "{what}, edge {e}, {} px from the edge: expected {want}, got {got} (want {want:?})", d + 1);
        }
    }
}

/// #14958 Done when 1: Range 1 % at Size 10 gives eight solid pixels, then 39, then no glow on
/// all four edges, within 3 levels. Today (main) the glow reaches one pixel further with a much
/// lighter last pixel: nine solid, then 191.
#[test]
fn outer_glow_range_one_gives_eight_solid_then_39_on_every_edge() {
    assert_all_edges(&measured_doc(glow(0.01), false), 0, false, &[0, 0, 0, 0, 0, 0, 0, 0, 39, 255, 255, 255], 3, "Range 1 %");
}

/// #14958 Done when 2 (guard, passes before and after): Range 25 keeps today's profile, matching
/// the measured `0 0 0 42 103 153 194 225 245 255` within 4 levels.
#[test]
fn outer_glow_range_25_keeps_the_measured_profile() {
    assert_all_edges(&measured_doc(glow(0.25), false), 0, false, &[0, 0, 0, 42, 103, 153, 194, 225, 245, 255, 255, 255], 4, "Range 25 %");
}

/// #14958 Done when 2 (guard): Range 50 keeps today's measured `26 72 113 148 179 204 225 240
/// 250 255` within 4 levels.
#[test]
fn outer_glow_range_50_keeps_the_measured_profile() {
    assert_all_edges(&measured_doc(glow(0.5), false), 0, false, &[26, 72, 113, 148, 179, 204, 225, 240, 250, 255, 255, 255], 4, "Range 50 %");
}

/// #14958 Done when 2 (guard): Range 100 keeps today's measured `140 163 184 201 217 229 240
/// 247 252 255` within 4 levels.
#[test]
fn outer_glow_range_100_keeps_the_measured_profile() {
    assert_all_edges(&measured_doc(glow(1.0), false), 0, false, &[140, 163, 184, 201, 217, 229, 240, 247, 252, 255, 255, 255], 4, "Range 100 %");
}

/// The first pixel inside the edge stays the layer colour: the glow never covers the layer
/// itself (guard, passes before and after).
#[test]
fn outer_glow_range_one_never_covers_the_layer_itself() {
    let d = measured_doc(glow(0.01), false);
    let b = crate::render(&d, d.bounds());
    let inside = [level(&b, 200, 75, 0), level(&b, 200, 75, 1), level(&b, 200, 75, 2)];
    assert_eq!(inside, [255, 128, 0], "(200, 75) stays the layer colour 255.128.0");
}

/// Cross-check of the shared cause with #14929 (no claim on that item): its measured Inner Glow
/// Range-1 profile is the mirror image `0 x8, 19, 128` on the G channel of the same document.
/// Today the glow reaches one pixel further inwards (nine solid, then 96).
#[test]
fn inner_glow_range_one_mirrors_the_measured_profile() {
    assert_all_edges(&measured_doc(glow(0.01), true), 1, true, &[0, 0, 0, 0, 0, 0, 0, 0, 19, 128, 128, 128], 3, "inner Range 1 %");
}

/// Not measured, keeps today's behaviour: a Precise glow at Range 1 % (its coverage stays above
/// the range down to the last pixel of its falloff).
#[test]
fn outer_glow_range_one_precise_keeps_todays_profile() {
    let g = Glow { technique: GlowTechnique::Precise, ..glow(0.01) };
    assert_all_edges(&measured_doc(g, false), 0, false, &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 255, 255], 1, "precise Range 1 %");
}

/// Not measured, keeps today's behaviour: Range 1 % with Spread 20 % (dilate 2, tent 8: solid
/// through 9 px, then 155, then no glow).
#[test]
fn outer_glow_range_one_with_spread_keeps_todays_profile() {
    let g = Glow { spread: 0.2, ..glow(0.01) };
    assert_all_edges(&measured_doc(g, false), 0, false, &[0, 0, 0, 0, 0, 0, 0, 0, 0, 155, 255, 255], 1, "spread 20 % at Range 1 %");
}

/// #15871 Done when 1: Size 20 at Range 1 % gives seventeen solid pixels, then 93, 201, then no
/// glow, within 3 levels. Before, the glow reached one pixel further: 48, 175, 239.
#[test]
fn outer_glow_range_one_at_size_20_gives_the_measured_profile() {
    let g = Glow { size: 20.0, ..glow(0.01) };
    assert_all_edges(&measured_doc(g, false), 0, false, &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 93, 201, 255, 255], 3, "size 20 at Range 1 %");
}

/// #15871 Done when 1: Size 5 at Range 1 % gives four solid pixels, then no glow.
#[test]
fn outer_glow_range_one_at_size_5_gives_the_measured_profile() {
    let g = Glow { size: 5.0, ..glow(0.01) };
    assert_all_edges(&measured_doc(g, false), 0, false, &[0, 0, 0, 0, 255, 255], 3, "size 5 at Range 1 %");
}

/// #15871 Done when 1: Range 2 % at Size 10 gives eight solid pixels, then 125, then no glow,
/// within 3 levels. Before, it gave 96, 223.
#[test]
fn outer_glow_range_two_gives_the_measured_profile() {
    assert_all_edges(&measured_doc(glow(0.02), false), 0, false, &[0, 0, 0, 0, 0, 0, 0, 0, 125, 255, 255, 255], 3, "Range 2 %");
}

/// Not measured, keeps today's behaviour: a custom (flat) contour at Range 1 % keeps the
/// #11361 reach-capped transfer (solid through 10 px, then no glow).
#[test]
fn outer_glow_range_one_custom_contour_keeps_todays_profile() {
    let g = Glow {
        contour: Contour::Custom {
            name: "flat".into(),
            points: [(0.0, 1.0), (1.0, 1.0)].iter().map(|&(input, output)| photocraft_doc::adjust::CurvePoint { input, output }).collect(),
            corners: vec![],
        },
        ..glow(0.01)
    };
    assert_all_edges(&measured_doc(g, false), 0, false, &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 255, 255], 1, "flat contour at Range 1 %");
}

/// Not measured, keeps today's behaviour: an Inner Glow from the Center at Range 1 % (no glow
/// for the first nine pixels in, one dim step at 10 px, full towards the middle).
#[test]
fn inner_glow_range_one_center_source_keeps_todays_profile() {
    let g = Glow { source: GlowSource::Center, ..glow(0.01) };
    assert_all_edges(&measured_doc(g, true), 1, true, &[128, 128, 128, 128, 128, 128, 128, 128, 128, 32, 0, 0], 1, "inner center at Range 1 %");
}
