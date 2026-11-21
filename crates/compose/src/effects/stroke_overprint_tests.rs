//! Stroke Overprint (#15000), measured on the document of the issue: 400 x 300 px, RGB 8 bit,
//! Background grey (128,128,128), a pixel layer "Shape" holding an orange (255,128,0) rectangle
//! over x 100..299, y 75..224, with a blue (0,0,255) stroke at Normal unless stated. Levels are
//! read from the flattened image in column x 200; the issue's numbers hold within 1 level (it
//! rounds a half level up or down).
//!
//! Overprint off (the default): the band replaces the layer's own pixels and the stroke's own
//! mode and opacity work on what lies UNDER the layer. Overprint on: the stroke blends over the
//! layer's own pixels.

use photocraft_color::{BlendMode, Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::vector::{Path, ShapeLayer, Subpath};
use photocraft_doc::{Effect, Fill, FxCommon, FxPaint, Gradient, Layer, LayerContent, LayerMask, StrokeFx, StrokePosition};
use photocraft_geom::{Rect, Size};

use crate::{Buffer, Document, render};

const GREY: f32 = 128.0 / 255.0;
const ORANGE: [f32; 3] = [1.0, GREY, 0.0];
const BLUE: [f32; 3] = [0.0, 0.0, 1.0];
const SIZE: Size = Size::new(400, 300);
/// The issue's band, in the layer's own pixels, one row in from its top edge.
const INSIDE_BAND: std::ops::RangeInclusive<i32> = 75..=79;

fn stroke(size: f32, position: StrokePosition, blend: BlendMode, opacity: f32, overprint: bool, paint: FxPaint) -> Effect {
    Effect::Stroke(StrokeFx { common: FxCommon::new(blend, opacity), size, position, paint, overprint })
}

fn blue_paint() -> FxPaint {
    FxPaint::Color(Color::rgb(BLUE[0], BLUE[1], BLUE[2]))
}

/// The issue's document: the grey Background and the orange pixel layer, with `strokes` as its style.
fn measured(strokes: Vec<Effect>) -> Document {
    let mut d = Document::with_background("overprint", SIZE, ColorMode::Rgb, SampleType::U8, Color::rgb(GREY, GREY, GREY));
    let mut pixels = photocraft_raster::Surface::new(PixelFormat::RGBA8);
    pixels.fill_rect(Rect::new(100, 75, 300, 225), &[ORANGE[0], ORANGE[1], ORANGE[2], 1.0]);
    let mut layer = Layer::new("Shape", LayerContent::Raster(pixels));
    layer.effects.items = strokes;
    d.layers.push(layer);
    d
}

/// The flattened document. A 1 x 1 render would build the stroke's distance fields over a window
/// with no shape edge in it, so the whole canvas goes through the compositor once per case.
fn flatten(d: &Document) -> Buffer {
    render(d, d.bounds())
}

fn levels(b: &Buffer, x: i32, y: i32) -> [f32; 3] {
    let p = b.get(x, y);
    [p[0] * 255.0, p[1] * 255.0, p[2] * 255.0]
}

/// One of the issue's measured levels, within 1.
fn assert_px(b: &Buffer, x: i32, y: i32, want: [f32; 3]) {
    let got = levels(b, x, y);
    for c in 0..3 {
        assert!((got[c] - want[c]).abs() <= 1.0, "({x}, {y}): {got:?}, channel {c} is {}, measured {}", got[c], want[c]);
    }
}

fn assert_band(b: &Buffer, ys: std::ops::RangeInclusive<i32>, want: [f32; 3]) {
    for y in ys {
        assert_px(b, 200, y, want);
    }
}

/// Done when: Inside 10 at 50 % opacity with Overprint on gives 127.64.128 in the band: half blue
/// over the layer's own orange, not over the grey beneath the layer.
#[test]
fn inside_stroke_with_overprint_blends_over_the_layer_s_own_pixels() {
    let d = measured(vec![stroke(10.0, StrokePosition::Inside, BlendMode::Normal, 0.5, true, blue_paint())]);
    let b = flatten(&d);
    assert_band(&b, INSIDE_BAND, [127.0, 64.0, 128.0]);
    // Where the layer has no pixels of its own, the stroke paints over the Background.
    assert_px(&b, 200, 60, [GREY * 255.0; 3]);
}

/// Done when: Inside 10 Multiply without Overprint gives 0.0.128 in the band: blue multiplied
/// with the grey under the layer, not with the layer's own orange (which gives 0.0.0).
#[test]
fn inside_stroke_without_overprint_blends_with_what_lies_under_the_layer() {
    let d = measured(vec![stroke(10.0, StrokePosition::Inside, BlendMode::Multiply, 1.0, false, blue_paint())]);
    let b = flatten(&d);
    assert_band(&b, INSIDE_BAND, [0.0, 0.0, 128.0]);
    // Deep inside the layer, past the band, its own pixels are untouched.
    assert_px(&b, 200, 150, ORANGE.map(|v| v * 255.0));
}

/// Done when: Inside 10 Multiply with Overprint gives 0.0.0, blue multiplied with the layer's own
/// orange. Guard: the compositor already painted that before the flag was read.
#[test]
fn inside_multiply_stroke_with_overprint_keeps_the_layer_s_own_pixels() {
    let d = measured(vec![stroke(10.0, StrokePosition::Inside, BlendMode::Multiply, 1.0, true, blue_paint())]);
    let b = flatten(&d);
    assert_band(&b, INSIDE_BAND, [0.0, 0.0, 0.0]);
}

/// Done when: Inside 10 at 50 % opacity without Overprint gives 64.64.192 in the band, half blue
/// over the grey under the layer. Guard: a Normal stroke already replaced the layer under its
/// band (#6735), and Overprint off keeps that for every mode.
#[test]
fn inside_normal_stroke_without_overprint_still_replaces_the_layer() {
    let d = measured(vec![stroke(10.0, StrokePosition::Inside, BlendMode::Normal, 0.5, false, blue_paint())]);
    let b = flatten(&d);
    assert_band(&b, INSIDE_BAND, [64.0, 64.0, 192.0]);
    assert_px(&b, 200, 150, ORANGE.map(|v| v * 255.0));
}

/// Done when: Center 10 at 50 % opacity with Overprint gives 64.64.192 on the half of the band
/// over the Background (y 70..74) and 127.64.128 on the half over the layer (y 75..79).
#[test]
fn center_stroke_with_overprint_paints_its_two_halves_differently() {
    let d = measured(vec![stroke(10.0, StrokePosition::Center, BlendMode::Normal, 0.5, true, blue_paint())]);
    let b = flatten(&d);
    assert_band(&b, 70..=74, [64.0, 64.0, 192.0]);
    assert_band(&b, INSIDE_BAND, [127.0, 64.0, 128.0]);
}

/// Done when: an Outside stroke gives the same pixels with and without Overprint, in both modes
/// the table measured; outside the layer's own pixels the flag makes no difference.
#[test]
fn outside_strokes_ignore_overprint() {
    for (blend, opacity, want) in [(BlendMode::Normal, 0.5, [64.0, 64.0, 192.0]), (BlendMode::Multiply, 1.0, [0.0, 0.0, 128.0])] {
        let off = measured(vec![stroke(10.0, StrokePosition::Outside, blend, opacity, false, blue_paint())]);
        let on = measured(vec![stroke(10.0, StrokePosition::Outside, blend, opacity, true, blue_paint())]);
        // The two documents differ only in the flag: a copy of `off` with the flag set equals
        // `on`, and `off` itself is rendered with Overprint off.
        let mut flipped = off.clone();
        if let Effect::Stroke(s) = &mut flipped.layers[1].effects.items[0] {
            s.overprint = true;
        }
        assert_eq!(on.layers[1].effects.items, flipped.layers[1].effects.items);
        assert!(matches!(&off.layers[1].effects.items[0], Effect::Stroke(s) if !s.overprint));
        let (before, after) = (flatten(&off), flatten(&on));
        assert_band(&before, 65..=74, want);
        assert_band(&after, 65..=74, want);
        // The layer's own pixels below the band stay untouched.
        assert_px(&before, 200, 150, ORANGE.map(|v| v * 255.0));
        assert_eq!(before.px, after.px, "{blend:?} outside stroke: Overprint changed a pixel");
    }
}

/// Not measured (the issue's list): gradient paint, a partly transparent layer, Fill below 100 %,
/// a layer that is not pixels, and a CMYK or Lab document keep today's behaviour, where the
/// checkbox changes nothing. The levels are the ones `upstream/main` 5bdbcc118 produced.
#[test]
fn unmeasured_configurations_keep_ignoring_overprint() {
    let inside = |overprint: bool, blend: BlendMode, opacity: f32, paint: FxPaint| stroke(10.0, StrokePosition::Inside, blend, opacity, overprint, paint);
    let gradient = FxPaint::Gradient(Gradient { stops: vec![(0.0, Color::BLACK), (1.0, Color::WHITE)], angle: 90.0, ..Gradient::default() });
    // A gradient paint at 50 % opacity, once through today's replace path (Normal) and once
    // through its blend path (Multiply).
    for (what, paint, blend, want) in [
        ("gradient Normal", gradient.clone(), BlendMode::Normal, [189.8, 189.8, 189.8]),
        ("gradient Multiply", gradient, BlendMode::Multiply, [253.3, 127.1, 0.0]),
    ] {
        let off = measured(vec![inside(false, blend, 0.5, paint.clone())]);
        let on = measured(vec![inside(true, blend, 0.5, paint.clone())]);
        let (before, after) = (flatten(&off), flatten(&on));
        assert_px(&before, 200, 77, want);
        assert_eq!(before.px, after.px, "{what}: Overprint changed a stroke the issue did not measure");
    }
    // A partly transparent pixel layer and a Fill of 50 % both keep today's pixels.
    for (what, fill, alpha, want) in [("semi-transparent pixels", 1.0, 0.5, [63.7, 63.7, 63.7]), ("Fill 50 %", 0.5, 1.0, [0.0, 0.0, 64.0])] {
        let build = |overprint: bool| {
            let mut d = measured(vec![inside(overprint, BlendMode::Multiply, 1.0, blue_paint())]);
            let layer = &mut d.layers[1];
            layer.fill_opacity = fill;
            layer.surface_mut().unwrap().fill_rect(Rect::new(100, 75, 300, 225), &[ORANGE[0], ORANGE[1], ORANGE[2], alpha]);
            d
        };
        let (before, after) = (flatten(&build(false)), flatten(&build(true)));
        assert_px(&before, 200, 77, want);
        assert_eq!(before.px, after.px, "{what}: Overprint changed a stroke the issue did not measure");
    }
    // A shape layer: the measured document holds pixels, so the flag stays ignored here.
    let shape_doc = |overprint: bool| {
        let mut d = Document::with_background("overprint", SIZE, ColorMode::Rgb, SampleType::U8, Color::rgb(GREY, GREY, GREY));
        let path = Path::new(vec![Subpath::polygon(&[(100.0, 75.0), (300.0, 75.0), (300.0, 225.0), (100.0, 225.0)])]);
        let mut sh = ShapeLayer { path, fill: Some(Fill::Solid(Color::rgb(ORANGE[0], ORANGE[1], ORANGE[2]))), ..ShapeLayer::default() };
        sh.cache = Some(photocraft_vector::render_shape(&sh, PixelFormat::RGBA8, Rect::new(0, 0, 400, 300)));
        let mut layer = Layer::new("Shape", LayerContent::Shape(sh));
        layer.effects.items = vec![inside(overprint, BlendMode::Multiply, 1.0, blue_paint())];
        d.layers.push(layer);
        d
    };
    let (before, after) = (flatten(&shape_doc(false)), flatten(&shape_doc(true)));
    assert_px(&before, 200, 77, [0.0, 0.0, 0.0]);
    assert_eq!(before.px, after.px, "a shape layer: Overprint changed a pixel the issue did not measure");
    // CMYK and Lab documents composite their inks and stored values: unmeasured, so the flag is
    // ignored there too.
    for (mode, want) in [(ColorMode::Cmyk, [0.0, 0.0, 18.7]), (ColorMode::Lab, [0.0, 0.0, 255.0])] {
        let channels = if mode == ColorMode::Cmyk { 5 } else { 4 };
        let build = |overprint: bool| {
            let mut d = Document::with_background("overprint", Size::new(64, 48), mode, SampleType::U8, Color::rgb(GREY, GREY, GREY));
            let mut pixels = photocraft_raster::Surface::new(PixelFormat::new(mode, SampleType::U8, true));
            let ink: Vec<f32> = match channels {
                5 => vec![0.0, 0.7, 1.0, 0.0, 1.0],
                _ => vec![ORANGE[0], ORANGE[1], ORANGE[2], 1.0],
            };
            pixels.fill_rect(Rect::new(16, 12, 48, 36), &ink);
            let mut layer = Layer::new("Shape", LayerContent::Raster(pixels));
            layer.effects.items = vec![inside(overprint, BlendMode::Multiply, 1.0, blue_paint())];
            d.layers.push(layer);
            d
        };
        let (before, after) = (flatten(&build(false)), flatten(&build(true)));
        assert_px(&before, 18, 14, want);
        assert_eq!(before.px, after.px, "{mode:?} document: Overprint changed a pixel the issue did not measure");
    }
}

/// Not measured: half-transparent layer pixels with Transparency Shapes Layer off. There the
/// effect shape is the whole layer, so the shape alone does not tell an opaque layer pixel from
/// a half-transparent one; the pixel's own alpha does. Those pixels keep today's path, where
/// the flag changes nothing (a Multiply stroke blends over the layer's own pixels).
#[test]
fn half_transparent_pixels_without_transparency_shapes_layer_ignore_overprint() {
    let build = |overprint: bool| {
        let mut d = measured(vec![stroke(10.0, StrokePosition::Inside, BlendMode::Multiply, 1.0, overprint, blue_paint())]);
        let layer = &mut d.layers[1];
        layer.advanced.transparency_shapes = false;
        layer.surface_mut().unwrap().fill_rect(Rect::new(100, 75, 300, 225), &[ORANGE[0], ORANGE[1], ORANGE[2], 0.5]);
        // A layer mask showing the rectangle: with Transparency Shapes Layer off the effects
        // take it as the layer's shape, so the band lies on the half-transparent pixels.
        let mut m = photocraft_raster::Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false));
        m.fill_rect(Rect::new(100, 75, 300, 225), &photocraft_raster::from_rgba(&m.format(), [1.0; 4]));
        layer.mask = Some(LayerMask { surface: m, enabled: true, linked: true, density: 1.0, feather: 0.0 });
        d
    };
    let (before, after) = (flatten(&build(false)), flatten(&build(true)));
    // Today's path: blue multiplied over the half-transparent orange in the band, not blue
    // multiplied with the grey under the layer (0.0.128).
    assert_px(&before, 200, 77, [0.0, 0.0, 63.7]);
    assert_eq!(before.px, after.px, "Transparency Shapes Layer off: Overprint changed a half-transparent pixel");
}

/// Not measured: more than one inside stroke on a layer. A lower red Normal stroke with Overprint
/// off and an upper blue Multiply stroke with Overprint on keep today's pixels: the upper stroke
/// multiplies the lower one (0.0.0), it does not land on a knocked-out layer and show plain blue.
/// Swapping the two flags changes nothing either.
#[test]
fn stacked_inside_strokes_keep_today_s_pixels() {
    let red = FxPaint::Color(Color::rgb(1.0, 0.0, 0.0));
    let build = |upper_overprint: bool, lower_overprint: bool| {
        measured(vec![
            stroke(10.0, StrokePosition::Inside, BlendMode::Multiply, 1.0, upper_overprint, blue_paint()),
            stroke(10.0, StrokePosition::Inside, BlendMode::Normal, 1.0, lower_overprint, red.clone()),
        ])
    };
    let (asked, swapped) = (flatten(&build(true, false)), flatten(&build(false, true)));
    assert_band(&asked, INSIDE_BAND, [0.0, 0.0, 0.0]);
    assert_eq!(asked.px, swapped.px, "stacked inside strokes: Overprint changed a pixel the issue did not measure");
    // Past the band the layer's own pixels are untouched.
    assert_px(&asked, 200, 150, ORANGE.map(|v| v * 255.0));
}

/// Not measured: an inside stroke at 0 % opacity. It keeps today's pixels, where it paints
/// nothing: the layer's own orange stays in the band with Overprint off and on, in Normal (today's
/// replace path skips a stroke pixel with no alpha) and in Multiply (it blends at 0 %). It does not
/// knock the layer out and show the grey under it.
#[test]
fn zero_opacity_inside_stroke_keeps_the_layer_s_own_pixels() {
    for blend in [BlendMode::Normal, BlendMode::Multiply] {
        let off = flatten(&measured(vec![stroke(10.0, StrokePosition::Inside, blend, 0.0, false, blue_paint())]));
        let on = flatten(&measured(vec![stroke(10.0, StrokePosition::Inside, blend, 0.0, true, blue_paint())]));
        assert_band(&off, INSIDE_BAND, ORANGE.map(|v| v * 255.0));
        assert_eq!(off.px, on.px, "{blend:?} at 0 %: Overprint changed a pixel the issue did not measure");
    }
}

/// Not measured (found in the review of #15249): a layer whose own blend mode is not Normal. The
/// issue's table measured the default Normal layer. With Blend Interior Effects as Group on, the
/// knocked-out band of an Overprint-off stroke would lose the layer's Multiply at the band where
/// today's path keeps it multiplied, and ticking the flag would change pixels on a configuration
/// the issue did not measure. The flag stays ignored there, and today's pixels are kept, with and
/// without Blend Interior Effects as Group. The levels are the ones `upstream/main` 856f865cf
/// produced.
#[test]
fn inside_stroke_on_a_non_normal_layer_blend_keeps_today_s_pixels() {
    for (blend_interior, want) in [(false, [64.0, 64.0, 191.5]), (true, [64.0, 64.0, 128.0])] {
        let build = |overprint: bool| {
            let mut d = measured(vec![stroke(10.0, StrokePosition::Inside, BlendMode::Normal, 0.5, overprint, blue_paint())]);
            let layer = &mut d.layers[1];
            layer.blend = BlendMode::Multiply;
            layer.advanced.blend_interior = blend_interior;
            d
        };
        let (off, on) = (flatten(&build(false)), flatten(&build(true)));
        assert_band(&off, INSIDE_BAND, want);
        assert_eq!(off.px, on.px, "Multiply layer, blend_interior {blend_interior}: Overprint changed a pixel the issue did not measure");
    }
}

/// Not measured (found in the review of #15249): a layer whose own blend mode is not Normal. The
/// gate checked the layer's own surface mode, not the document's, so such a layer took the
/// knockout even though the issue's CMYK rows measured a layer of CMYK pixels. The flag stays
/// ignored. The levels are the ones `upstream/main` 856f865cf produced.
#[test]
fn rgb_layer_in_a_cmyk_document_keeps_today_s_pixels() {
    for (blend, opacity, want) in [(BlendMode::Multiply, 1.0, [0.0, 0.0, 0.0]), (BlendMode::Normal, 0.5, [63.8, 64.3, 191.9])] {
        let build = |overprint: bool| {
            let mut d = Document::with_background("overprint", Size::new(64, 48), ColorMode::Cmyk, SampleType::U8, Color::rgb(GREY, GREY, GREY));
            let mut pixels = photocraft_raster::Surface::new(PixelFormat::RGBA8);
            pixels.fill_rect(Rect::new(16, 12, 48, 36), &[ORANGE[0], ORANGE[1], ORANGE[2], 1.0]);
            let mut layer = Layer::new("Shape", LayerContent::Raster(pixels));
            layer.effects.items = vec![stroke(10.0, StrokePosition::Inside, blend, opacity, overprint, blue_paint())];
            d.layers.push(layer);
            d
        };
        let (off, on) = (flatten(&build(false)), flatten(&build(true)));
        assert_px(&off, 18, 14, want);
        assert_eq!(off.px, on.px, "an RGB layer in a CMYK document, {blend:?}: Overprint changed a pixel the issue did not measure");
    }
}
