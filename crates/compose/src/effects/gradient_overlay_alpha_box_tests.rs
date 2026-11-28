//! #14910 - BUG-LAYER-STYLE-GRADIENT-OVERLAY-ALIGNMENT-BOX-SEMI-TRANSPARENT.
//!
//! With Align with Layer on, the reference application lays a Gradient Overlay out over the
//! bounding box of the layer's pixels that are at least half opaque (alpha 128 of 255), not of
//! every pixel with any alpha. Measured on two 8-bit RGB documents with a white Background and one
//! Gradient Overlay: black to white, Normal 100 %, Linear, angle 0, scale 100, no offset. Values
//! are the red channel of the composite. Not measured, so they keep the box of every
//! non-transparent pixel: a layer with no half-opaque pixel, other layer kinds, 16 and 32 bit.

use super::*;
use crate::{relayout_damage, render};
use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::{Document, Effect, FxCommon, Gradient, GradientStyle, Layer};
use photocraft_geom::{Rect, Size};

const ORANGE: [f32; 3] = [1.0, 128.0 / 255.0, 0.0];

/// `photocraft_io::bake_gradient` of black to white, Classic at Smoothness 100 (compose cannot
/// depend on io): `-u³ + 1.5u² + 0.5u` sampled at 97 even stops.
fn classic_smooth() -> Vec<(f32, Color)> {
    (0..=96)
        .map(|k| {
            let u = k as f32 / 96.0;
            let v = (-u * u * u + 1.5 * u * u + 0.5 * u).clamp(0.0, 1.0);
            (u, Color::rgb(v, v, v))
        })
        .collect()
}

/// Black to white, Classic at Smoothness 0: a straight ramp.
fn classic_linear() -> Vec<(f32, Color)> {
    vec![(0.0, Color::rgb(0.0, 0.0, 0.0)), (1.0, Color::rgb(1.0, 1.0, 1.0))]
}

fn overlay(stops: Vec<(f32, Color)>) -> Effect {
    let gradient =
        Gradient { stops, opacity_stops: Vec::new(), style: GradientStyle::Linear, angle: 0.0, scale: 1.0, reverse: false, align: true, offset: (0.0, 0.0) };
    Effect::GradientOverlay { common: FxCommon::new(BlendMode::Normal, 1.0), gradient, dither: false, interpolation: None }
}

/// A `w` x `h` document with a white Background and a pixel layer of orange rects, each at its
/// own opacity (0..=1), carrying `effect`.
fn doc(depth: SampleType, w: u32, h: u32, rects: &[(Rect, f32)], effect: Effect) -> Document {
    let mut d = Document::with_background("t", Size::new(w, h), ColorMode::Rgb, depth, Color::WHITE);
    let mut l = Layer::raster("Squares", d.pixel_format());
    if let Some(s) = l.surface_mut() {
        for (r, a) in rects {
            s.fill_rect(*r, &[ORANGE[0], ORANGE[1], ORANGE[2], *a]);
        }
    }
    l.effects.items = vec![effect];
    d.layers.push(l);
    d
}

/// Test document 1: 240 x 120, squares 40 x 40 at 100 / 50 / 25 % at x 20 / 100 / 180, y 40..79,
/// Classic at Smoothness 100.
fn doc1(depth: SampleType) -> Document {
    let sq = |x: i32| Rect::new(x, 40, x + 40, 80);
    doc(depth, 240, 120, &[(sq(20), 1.0), (sq(100), 0.5), (sq(180), 0.25)], overlay(classic_smooth()))
}

/// Test document 2: 240 x 120, an opaque square at x 20..59 and one filled at `n` % at x 180..219
/// (y 40..79), Classic at Smoothness 0.
fn doc2(depth: SampleType, n: f32) -> Document {
    let sq = |x: i32| Rect::new(x, 40, x + 40, 80);
    doc(depth, 240, 120, &[(sq(20), 1.0), (sq(180), n / 100.0)], overlay(classic_linear()))
}

/// Red channel (0..=255) of the composite at `xs` on row `y`.
fn reds<const N: usize>(d: &Document, y: i32, xs: [i32; N]) -> [u8; N] {
    let row = render(d, Rect::new(0, y, d.bounds().x1, y + 1));
    xs.map(|x| row.px.get(x as usize).map_or(0, |p| (p[0] * 255.0).round().clamp(0.0, 255.0) as u8))
}

fn assert_close<const N: usize>(what: &str, got: [u8; N], want: [u8; N]) {
    for (g, w) in got.iter().zip(want) {
        assert!(g.abs_diff(w) <= 2, "{what}: got {got:?}, want {want:?}");
    }
}

#[test]
fn document_1_leaves_the_quarter_opaque_square_out_of_the_box() {
    // Reference: 31 / 239 / 255 (the box is the 100 % and 50 % squares, x 20..139). Main gave
    // 16 / 191 / 251 (the box reached the 25 % square).
    assert_close("8 bit", reds(&doc1(SampleType::U8), 60, [40, 120, 200]), [31, 239, 255]);
}

#[test]
fn document_2_box_holds_only_squares_at_least_half_opaque() {
    for n in [1.0, 10.0, 49.0] {
        // Box = the opaque square only.
        assert_close(&format!("N = {n}"), reds(&doc2(SampleType::U8, n), 60, [20, 40, 59]), [0, 128, 249]);
    }
    for n in [50.0, 51.0] {
        // A 50 % fill stores alpha 128, inside the box: both squares.
        assert_close(&format!("N = {n}"), reds(&doc2(SampleType::U8, n), 60, [20, 40, 59]), [0, 26, 50]);
    }
}

#[test]
fn deep_documents_keep_the_box_of_every_visible_pixel() {
    // 16 and 32 bit are not measured: they keep the box of every non-transparent pixel.
    for depth in [SampleType::U16, SampleType::F32] {
        assert_close(&format!("document 1 {depth:?}"), reds(&doc1(depth), 60, [40, 120, 200]), [16, 191, 251]);
        assert_close(&format!("document 2 {depth:?}"), reds(&doc2(depth, 10.0), 60, [20, 40, 59]), [0, 26, 50]);
    }
}

#[test]
fn a_layer_with_no_half_opaque_pixel_keeps_the_box_of_every_visible_pixel() {
    // Squares at 49 % (x 20..59) and 25 % (x 180..219): no pixel is half opaque, so the gradient
    // spans both squares, x 20..219, as before.
    let sq = |x: i32| Rect::new(x, 40, x + 40, 80);
    let d = doc(SampleType::U8, 240, 120, &[(sq(20), 0.49), (sq(180), 0.25)], overlay(classic_linear()));
    let a = 125.0 / 255.0;
    let over_white = |g: f32| (g * a + 255.0 * (1.0 - a)).round() as u8;
    let want = [20, 40, 59].map(|x| over_white((x - 20) as f32 / 200.0 * 255.0));
    assert_close("49 % and 25 %", reds(&d, 60, [20, 40, 59]), want);
}

#[test]
fn opaque_layers_keep_their_pixels() {
    // An opaque layer's half-opaque box is its pixel box: the ramp spans x 100..299 at every depth.
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let d = doc(depth, 400, 300, &[(Rect::new(100, 75, 300, 225), 1.0)], overlay(classic_linear()));
        let xs = [100, 101, 150, 199, 200, 250, 299];
        let want = xs.map(|x| ((x - 100) as f32 / 200.0 * 255.0).round() as u8);
        let got = reds(&d, 150, xs);
        assert!(got.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= 1), "{depth:?}: got {got:?}, want {want:?}");
    }
}

#[test]
fn painting_that_moves_the_half_opaque_box_lays_the_gradient_out_again() {
    // The 10 % square becoming 60 % grows the box to both squares: the whole layer repaints, not
    // only the changed square. Main kept the box (both squares were already non-transparent).
    let before = doc2(SampleType::U8, 10.0);
    // The same document with the faint square repainted at `n` %.
    let repaint = |n: f32| {
        let mut d = before.clone();
        if let Some(s) = d.layers.last_mut().and_then(Layer::surface_mut) {
            s.fill_rect(Rect::new(180, 40, 220, 80), &[ORANGE[0], ORANGE[1], ORANGE[2], n / 100.0]);
        }
        d
    };
    let after = repaint(60.0);
    let damage = relayout_damage(&before, &after);
    assert!(damage.contains_rect(&Rect::new(20, 40, 60, 80)), "damage {damage:?} misses the opaque square");
    // A change that leaves the box where it is (10 % to 20 %) repaints only what it changed.
    assert_eq!(relayout_damage(&before, &repaint(20.0)), Rect::EMPTY);
    // And the re-laid gradient is the one the new box gives.
    assert_close("after", reds(&after, 60, [20, 40, 59]), [0, 26, 50]);
}

/// [`overlay`] with Align with Layer off.
fn unaligned(stops: Vec<(f32, Color)>) -> Effect {
    let mut e = overlay(stops);
    if let Effect::GradientOverlay { gradient, .. } = &mut e {
        gradient.align = false;
    }
    e
}

#[test]
fn an_overlay_not_aligned_with_the_layer_keeps_the_box_of_every_visible_pixel() {
    // Only Align with Layer on was measured (#14910); Align off keeps the frame it had, which
    // spans both squares of document 2 whatever their opacity.
    let mut d = doc2(SampleType::U8, 10.0);
    if let Some(l) = d.layers.last_mut() {
        l.effects.items = vec![unaligned(classic_linear())];
    }
    assert_close("align off", reds(&d, 60, [20, 40, 59]), [0, 26, 50]);
    // Each overlay picks its own box: an unaligned overlay painted above an aligned one keeps the
    // wide box, and the aligned one below it still uses the half-opaque box.
    if let Some(l) = d.layers.last_mut() {
        l.effects.items = vec![unaligned(classic_linear()), overlay(classic_linear())];
    }
    assert_close("unaligned on top", reds(&d, 60, [20, 40, 59]), [0, 26, 50]);
    if let Some(l) = d.layers.last_mut() {
        l.effects.items = vec![overlay(classic_linear()), unaligned(classic_linear())];
    }
    assert_close("aligned on top", reds(&d, 60, [20, 40, 59]), [0, 128, 249]);
}

#[test]
fn painting_under_an_overlay_not_aligned_with_the_layer_keeps_its_damage() {
    // With Align off the half-opaque box plays no part, so moving it repaints only what changed.
    let mut before = doc2(SampleType::U8, 10.0);
    if let Some(l) = before.layers.last_mut() {
        l.effects.items = vec![unaligned(classic_linear())];
    }
    let mut after = before.clone();
    if let Some(s) = after.layers.last_mut().and_then(Layer::surface_mut) {
        s.fill_rect(Rect::new(180, 40, 220, 80), &[ORANGE[0], ORANGE[1], ORANGE[2], 0.6]);
    }
    assert_eq!(relayout_damage(&before, &after), Rect::EMPTY);
}

#[test]
fn composite_with_effects_lays_the_overlay_out_like_the_document() {
    // The public single-layer entry point gives the same box as a document render: document 2 at
    // N = 10 spans the opaque square only.
    let d = doc2(SampleType::U8, 10.0);
    let Some(layer) = d.layers.last() else { panic!("the squares layer") };
    let rect = d.bounds();
    let (w, h) = (rect.width() as usize, rect.height() as usize);
    let mut content = Buffer::filled(rect, [0.0; 4]);
    for (i, p) in content.px.iter_mut().enumerate() {
        let (x, y) = ((i % w) as i32, (i / w) as i32);
        let a = if !(40..80).contains(&y) {
            0.0
        } else if (20..60).contains(&x) {
            1.0
        } else if (180..220).contains(&x) {
            0.1
        } else {
            0.0
        };
        *p = [ORANGE[0], ORANGE[1], ORANGE[2], a];
    }
    let shape: Vec<f32> = content.px.iter().map(|p| p[3]).collect();
    assert_eq!(shape.len(), w * h);
    let maps = build_maps(layer, shape, rect, &photocraft_doc::GlobalLight::default(), &TextureCtx { rect, patterns: &[], anchor: (20.0, 40.0) });
    let mut out = Buffer::filled(rect, [1.0; 4]);
    composite_with_effects(layer, &content, &mut out, &maps, Rect::new(20, 40, 220, 80), &[]);
    let red = |x: usize| out.px.get(60 * w + x).map_or(0, |p| (p[0] * 255.0).round().clamp(0.0, 255.0) as u8);
    assert_close("composite_with_effects", [red(20), red(40), red(59)], [0, 128, 249]);
}
