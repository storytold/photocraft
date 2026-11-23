//! #14907 - BUG-LAYER-STYLE-GRADIENT-OVERLAY-CENTRE-HALF-PIXEL.
//!
//! Measured on the issue's test document: 400 x 300 px, RGB 8 bit, white Background, an orange
//! (255, 128, 0) pixel layer covering x 100..299, y 75..224, with one Gradient Overlay: black to
//! white, Normal 100 %, angle 90, scale 100, Align with Layer on, no offset, Method Classic at
//! Smoothness 100 unless a case says otherwise. Values are the red channel of the composite, on
//! row y 150 and column x 200. The reference application centres Radial, Angle and Diamond on
//! pixel (200, 150) (its top-left corner) and scales a Linear overlay about its whole-pixel
//! midpoint; Linear and Reflected at Scale 100 already matched and must not move.

use super::*;
use crate::render;
use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::{Document, Effect, Fill, FxCommon, FxPaint, Gradient, GradientStyle, Layer, LayerContent, StrokeFx, StrokePosition};
use photocraft_geom::{Rect, Size};

const ROW_XS: [i32; 12] = [100, 101, 125, 150, 175, 199, 200, 225, 250, 275, 298, 299];
const COL_YS: [i32; 12] = [75, 76, 94, 112, 131, 149, 150, 168, 187, 205, 223, 224];

/// `photocraft_io::bake_gradient` of black to white, Classic at Smoothness 100 (compose cannot
/// depend on io): the Catmull-Rom curve with half-slope end tangents, `-u³ + 1.5u² + 0.5u`,
/// sampled at 97 even stops.
fn classic_black_to_white() -> Vec<(f32, Color)> {
    (0..=96)
        .map(|k| {
            let u = k as f32 / 96.0;
            let v = (-u * u * u + 1.5 * u * u + 0.5 * u).clamp(0.0, 1.0);
            (u, Color::rgb(v, v, v))
        })
        .collect()
}

fn gradient(style: GradientStyle, angle: f32, scale: f32, offset: (f32, f32)) -> Gradient {
    Gradient { stops: classic_black_to_white(), opacity_stops: Vec::new(), style, angle, scale, reverse: false, align: true, offset }
}

fn shape_doc(depth: SampleType, effect: Effect) -> Document {
    shape_doc_in(depth, Rect::new(100, 75, 300, 225), effect)
}

/// [`shape_doc`] with the orange rectangle covering `shape`.
fn shape_doc_in(depth: SampleType, shape: Rect, effect: Effect) -> Document {
    let mut d = Document::with_background("t", Size::new(400, 300), ColorMode::Rgb, depth, Color::WHITE);
    let mut l = Layer::raster("Shape", d.pixel_format());
    if let Some(s) = l.surface_mut() {
        s.fill_rect(shape, &[1.0, 128.0 / 255.0, 0.0, 1.0]);
    }
    l.effects.items = vec![effect];
    d.layers.push(l);
    d
}

fn overlay(g: Gradient) -> Effect {
    Effect::GradientOverlay { common: FxCommon::new(BlendMode::Normal, 1.0), gradient: g, dither: false, interpolation: None }
}

/// Red channel (0..=255) of the composite at the issue's row and column samples.
fn samples(d: &Document) -> ([u8; 12], [u8; 12]) {
    let row = render(d, Rect::new(0, 150, 400, 151));
    let col = render(d, Rect::new(200, 0, 201, 300));
    let r = |p: [f32; 4]| (p[0] * 255.0).round().clamp(0.0, 255.0) as u8;
    let at = |px: &[[f32; 4]], i: i32| px.get(i as usize).copied().map_or(0, r);
    (ROW_XS.map(|x| at(&row.px, x)), COL_YS.map(|y| at(&col.px, y)))
}

fn assert_close(what: &str, got: [u8; 12], want: [u8; 12]) {
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        assert!(g.abs_diff(w) <= 2, "{what} sample {i}: got {got:?}, reference {want:?}");
    }
}

fn check(style: GradientStyle, scale: f32, offset: (f32, f32), row: Option<[u8; 12]>, col: Option<[u8; 12]>) {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let (r, c) = samples(&shape_doc(depth, overlay(gradient(style, 90.0, scale, offset))));
        if let Some(want) = row {
            assert_close(&format!("{style:?} scale {scale} offset {offset:?} {depth:?} row"), r, want);
        }
        if let Some(want) = col {
            assert_close(&format!("{style:?} scale {scale} offset {offset:?} {depth:?} column"), c, want);
        }
    }
}

/// The reference application's centre is one black pixel at (`cx`, 150), lighter on every side;
/// the 2-level tolerance of `check` would also pass a centre between two pixels.
fn centre_is_darkest(style: GradientStyle, offset: (f32, f32), cx: i32) {
    let d = shape_doc(SampleType::U8, overlay(gradient(style, 90.0, 1.0, offset)));
    let red = |x: i32, y: i32| (render(&d, Rect::new(x, y, x + 1, y + 1)).px[0][0] * 255.0).round() as u8;
    let centre = red(cx, 150);
    assert_eq!(centre, 0, "{style:?} {offset:?}: centre ({cx}, 150)");
    for (x, y) in [(cx - 1, 150), (cx + 1, 150), (cx, 149), (cx, 151)] {
        assert!(red(x, y) > centre, "{style:?} {offset:?}: ({x}, {y}) is as dark as the centre");
    }
}

#[test]
fn radial_overlay_centres_on_the_middle_pixel() {
    centre_is_darkest(GradientStyle::Radial, (0.0, 0.0), 200);
    check(
        GradientStyle::Radial,
        1.0,
        (0.0, 0.0),
        Some([255, 255, 255, 179, 76, 2, 0, 76, 179, 255, 255, 255]),
        Some([255, 253, 202, 130, 53, 2, 0, 49, 125, 199, 251, 253]),
    );
}

#[test]
fn angle_overlay_centres_on_the_middle_pixel() {
    check(
        GradientStyle::Angle,
        1.0,
        (0.0, 0.0),
        Some([203, 203, 203, 203, 203, 203, 52, 52, 52, 52, 52, 52]),
        Some([0, 0, 0, 0, 0, 0, 52, 128, 128, 128, 128, 128]),
    );
}

#[test]
fn diamond_overlay_centres_on_the_middle_pixel() {
    centre_is_darkest(GradientStyle::Diamond, (0.0, 0.0), 200);
    check(
        GradientStyle::Diamond,
        1.0,
        (0.0, 0.0),
        Some([255, 255, 255, 182, 77, 2, 0, 77, 182, 255, 255, 255]),
        Some([255, 255, 205, 132, 54, 2, 0, 50, 128, 201, 253, 255]),
    );
}

#[test]
fn scaled_linear_overlay_centres_on_the_middle_pixel() {
    check(GradientStyle::Linear, 0.5, (0.0, 0.0), None, Some([255, 255, 255, 255, 205, 132, 128, 54, 0, 0, 0, 0]));
    check(GradientStyle::Linear, 0.1, (0.0, 0.0), None, Some([255, 255, 255, 255, 255, 150, 128, 0, 0, 0, 0, 0]));
}

#[test]
fn offset_radial_overlay_centres_on_its_pixel() {
    centre_is_darkest(GradientStyle::Radial, (0.25, 0.0), 250);
    check(GradientStyle::Radial, 1.0, (0.25, 0.0), Some([255, 255, 255, 255, 255, 183, 179, 76, 0, 76, 171, 175]), None);
}

/// Linear and Reflected at Scale 100 matched the reference before the fix: the overlay layout
/// and parameter are exactly the shared ones there, at every angle and offset.
#[test]
fn linear_and_reflected_at_full_scale_are_unchanged() {
    let frames = [Rect::new(100, 75, 300, 225), Rect::new(-3, 7, 30, 18), Rect::new(0, 0, 601, 61)];
    for style in [GradientStyle::Linear, GradientStyle::Reflected] {
        for angle in [0.0, 30.0, 45.0, 87.0, 90.0, 135.0, -60.0, 180.0] {
            for offset in [(0.0, 0.0), (0.25, 0.0), (-0.1, 0.3)] {
                for frame in frames {
                    let shared = crate::fill_layout::gradient_layout(style, angle, 1.0, offset, frame);
                    let ours = crate::fill_layout::overlay_gradient_layout(style, angle, 1.0, offset, frame);
                    assert_eq!(shared, ours, "{style:?} {angle} {offset:?} {frame:?}");
                    let (a, s, o) = shared;
                    for (x, y) in [(frame.x0, frame.y0), ((frame.x0 + frame.x1) / 2, (frame.y0 + frame.y1) / 2), (frame.x1 - 1, frame.y1 - 1)] {
                        let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                        let today = gradient_t(style, a, s, false, o, frame, px, py);
                        let now = overlay_gradient_t(style, a, s, false, o, frame, px, py);
                        assert_eq!(today.to_bits(), now.to_bits(), "{style:?} {angle} {offset:?} {frame:?} ({x},{y})");
                    }
                }
            }
        }
    }
}

/// Reflected at other scales is not measured: it keeps today's layout.
#[test]
fn scaled_reflected_overlay_keeps_todays_layout() {
    let frame = Rect::new(100, 75, 300, 225);
    for scale in [0.1, 0.5, 1.5] {
        let shared = crate::fill_layout::gradient_layout(GradientStyle::Reflected, 90.0, scale, (0.0, 0.0), frame);
        assert_eq!(shared, crate::fill_layout::overlay_gradient_layout(GradientStyle::Reflected, 90.0, scale, (0.0, 0.0), frame));
    }
}

/// Gradient fills and gradient strokes share `gradient_t` and keep sampling the pixel centre: a
/// Radial one stays symmetric about the frame's centre line.
#[test]
fn gradient_fills_and_strokes_keep_centre_sampling() {
    let frame = Rect::new(100, 75, 300, 225);
    for style in [GradientStyle::Radial, GradientStyle::Diamond] {
        let t = |x: f32, y: f32| gradient_t(style, 90.0, 1.0, false, (0.0, 0.0), frame, x, y);
        assert_eq!(t(199.5, 150.5), t(200.5, 150.5), "{style:?}");
        assert_eq!(t(200.5, 149.5), t(200.5, 150.5), "{style:?}");
    }

    // A Radial fill layer over the whole canvas: pixels 199 and 200 mirror each other.
    let mut d = Document::with_background("t", Size::new(400, 300), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let fill = Fill::gradient(vec![(0.0, Color::BLACK), (1.0, Color::WHITE)], 90.0, 1.0, GradientStyle::Radial, false);
    d.layers.push(Layer::new("fill", LayerContent::Fill(fill)));
    let row = render(&d, Rect::new(199, 150, 201, 151));
    assert_eq!(row.px[0], row.px[1]);

    // A Radial stroke (Inside, 3 px) at Scale 200: its left and right edges mirror each other.
    let mut g = gradient(GradientStyle::Radial, 90.0, 2.0, (0.0, 0.0));
    g.stops = vec![(0.0, Color::BLACK), (1.0, Color::WHITE)];
    let stroke = Effect::Stroke(StrokeFx {
        common: FxCommon::new(BlendMode::Normal, 1.0),
        size: 3.0,
        position: StrokePosition::Inside,
        paint: FxPaint::Gradient(g),
        overprint: false,
    });
    let d = shape_doc(SampleType::U8, stroke);
    let row = render(&d, Rect::new(100, 150, 300, 151));
    assert_eq!(row.px[0], row.px[199]);
    assert_eq!(row.px[1], row.px[198]);
}

/// Layers with an odd width or height were not measured (#14907): their overlays keep today's
/// layout and pixel-centre sampling. On such a layer today's centre already falls on a pixel.
#[test]
fn odd_sized_overlays_keep_todays_sampling_and_layout() {
    let styles = [GradientStyle::Linear, GradientStyle::Radial, GradientStyle::Angle, GradientStyle::Reflected, GradientStyle::Diamond];
    for frame in [Rect::new(100, 75, 301, 226), Rect::new(100, 75, 301, 225), Rect::new(100, 75, 300, 226), Rect::new(-3, 7, 30, 18)] {
        for style in styles {
            for angle in [0.0, 45.0, 90.0, 135.0] {
                for scale in [1.0, 0.5, 0.1] {
                    for offset in [(0.0, 0.0), (0.25, 0.0)] {
                        let what = format!("{style:?} {angle} scale {scale} {offset:?} {frame:?}");
                        let shared = crate::fill_layout::gradient_layout(style, angle, scale, offset, frame);
                        assert_eq!(shared, crate::fill_layout::overlay_gradient_layout(style, angle, scale, offset, frame), "{what}");
                        let (a, s, o) = shared;
                        for (x, y) in [(frame.x0, frame.y0), ((frame.x0 + frame.x1) / 2, (frame.y0 + frame.y1) / 2), (frame.x1 - 1, frame.y1 - 1)] {
                            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                            let today = gradient_t(style, a, s, false, o, frame, px, py);
                            let now = overlay_gradient_t(style, a, s, false, o, frame, px, py);
                            assert_eq!(today.to_bits(), now.to_bits(), "{what} ({x},{y})");
                        }
                    }
                }
            }
        }
    }

    // A 201 x 151 Radial overlay: its middle pixel (200, 150) samples t = 0 and its neighbours
    // mirror each other, as today.
    let d = shape_doc_in(SampleType::U8, Rect::new(100, 75, 301, 226), overlay(gradient(GradientStyle::Radial, 90.0, 1.0, (0.0, 0.0))));
    let red = |x: i32, y: i32| (render(&d, Rect::new(x, y, x + 1, y + 1)).px[0][0] * 255.0).round() as u8;
    assert_eq!(red(200, 150), 0, "201 x 151 Radial: middle pixel");
    assert_eq!(red(199, 150), red(201, 150), "201 x 151 Radial: row mirrors about the middle pixel");
    assert_eq!(red(200, 149), red(200, 151), "201 x 151 Radial: column mirrors about the middle pixel");
}

#[test]
fn overlay_layout_and_parameter_survive_hostile_values() {
    let frame = Rect::new(0, 0, 1, 1);
    for v in [f32::NAN, f32::INFINITY, -f32::INFINITY, 0.0, -1.0, 1e30] {
        for style in [GradientStyle::Linear, GradientStyle::Radial, GradientStyle::Angle, GradientStyle::Reflected, GradientStyle::Diamond] {
            let _ = crate::fill_layout::overlay_gradient_layout(style, v, v, (v, v), frame);
            let t = overlay_gradient_t(style, v, v, false, (v, v), frame, v, v);
            assert!(t.is_nan() || (0.0..=1.0).contains(&t));
        }
    }
}
