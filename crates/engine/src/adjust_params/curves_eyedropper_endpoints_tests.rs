//! #14377: the Curves black and white eyedroppers each move only their own end point.
use super::*;
use photocraft_color::{PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_geom::{Rect, Size};
use photocraft_raster::Surface;

const BLACK: f32 = 32.0 / 255.0;
const WHITE: f32 = 208.0 / 255.0;

/// The issue's strips: the 17 grays 0, 16, ..., 240, 255, then the six colour strips.
fn strips() -> Vec<[u8; 3]> {
    let mut v: Vec<[u8; 3]> = (0..16).map(|k| [k * 16; 3]).collect();
    v.push([255; 3]);
    v.extend([[200, 50, 50], [50, 200, 50], [50, 50, 200], [255, 128, 0], [30, 60, 90], [128, 128, 128]]);
    v
}

/// One opaque 8-bit RGB pixel per strip, on a single layer.
fn session() -> crate::Session {
    let strips = strips();
    let mut doc = Document::new("Curves eyedroppers", Size::new(strips.len() as u32, 1), ColorMode::Rgb, SampleType::U8);
    let mut pixels = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
    for (x, c) in strips.iter().enumerate() {
        let px = [f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0, 1.0];
        pixels.fill_rect(Rect::from_xywh(x as i32, 0, 1, 1), &px);
    }
    let layer = Layer::new("Strips", LayerContent::Raster(pixels));
    let id = layer.id;
    doc.layers.push(layer);
    let mut s = crate::Session::new();
    s.add_document(doc, None);
    s.select_layer(id).unwrap();
    s
}

/// The 8-bit colour of every strip after `image.adjustments.curves` with `params`.
fn applied(params: Value) -> Vec<[u8; 3]> {
    let mut s = session();
    s.execute("image.adjustments.curves", params).unwrap();
    let d = s.active().unwrap();
    let layer = d.doc.layer(d.active_layer.unwrap()).unwrap();
    let LayerContent::Raster(surface) = &layer.content else { panic!("expected pixels") };
    (0..strips().len() as i32)
        .map(|x| {
            let px = surface.pixel(x, 0);
            std::array::from_fn(|c| (px[c] * 255.0).round() as u8)
        })
        .collect()
}

/// Two picks in a row, as the dialog chains them: the second pick starts from the first's params.
fn picks(base: Value, picks: [(CurvesEyedropper, f32); 2]) -> Adjustment {
    picks.into_iter().fold(from_params("curves", &base, None, ColorMode::Rgb).unwrap(), |adjustment, (picker, tone)| {
        curves_eyedropper(&to_params(&adjustment), picker, [tone; 3], ColorMode::Rgb).unwrap()
    })
}

fn channels(a: &Adjustment) -> (&[CurvePoint], &[Vec<CurvePoint>; 3]) {
    match a {
        Adjustment::Curves { master, per_channel, .. } => (master, per_channel),
        _ => panic!("expected curves"),
    }
}

fn close(point: &CurvePoint, input: f32, output: f32) -> bool {
    (point.input - input).abs() < 1e-4 && (point.output - output).abs() < 1e-4
}

/// The measured result: every channel follows `(v - 32) * 255 / 176`.
fn assert_measured(adjustment: &Adjustment, case: &str) {
    let out = applied(to_params(adjustment));
    let by_value = |v: u8| out[strips().iter().position(|c| *c == [v; 3]).unwrap()];
    for (gray, expected) in [(0, 0), (32, 0), (80, 70), (112, 116), (128, 139), (160, 185), (208, 255), (255, 255)] {
        assert_eq!(by_value(gray), [expected; 3], "{case}: gray {gray}");
    }
    let colours = [
        ([200, 50, 50], [243, 26, 26]),
        ([50, 200, 50], [26, 243, 26]),
        ([50, 50, 200], [26, 26, 243]),
        ([255, 128, 0], [255, 139, 0]),
        ([30, 60, 90], [0, 41, 84]),
    ];
    for (input, expected) in colours {
        let at = strips().iter().position(|c| *c == input).unwrap();
        assert_eq!(out[at], expected, "{case}: {input:?}");
    }
}

#[test]
fn black_then_white_gives_the_measured_values() {
    assert_measured(&picks(json!({}), [(CurvesEyedropper::Black, BLACK), (CurvesEyedropper::White, WHITE)]), "black then white");
}

#[test]
fn white_then_black_gives_the_same_values() {
    assert_measured(&picks(json!({}), [(CurvesEyedropper::White, WHITE), (CurvesEyedropper::Black, BLACK)]), "white then black");
}

#[test]
fn each_pick_moves_only_its_own_end_point() {
    for order in [[(CurvesEyedropper::Black, BLACK), (CurvesEyedropper::White, WHITE)], [(CurvesEyedropper::White, WHITE), (CurvesEyedropper::Black, BLACK)]] {
        let adjustment = picks(json!({}), order);
        let (master, per_channel) = channels(&adjustment);
        for curve in per_channel {
            assert_eq!(curve.len(), 2, "{order:?}: {curve:?}");
            assert!(close(&curve[0], BLACK, 0.0), "{order:?}: black end point {curve:?}");
            assert!(close(&curve[1], WHITE, 1.0), "{order:?}: white end point {curve:?}");
        }
        assert!(master.len() == 2 && close(&master[0], 0.0, 0.0) && close(&master[1], 1.0, 1.0), "the composite stays straight: {master:?}");
    }

    // An end point that was edited by hand keeps its output too.
    let base = json!({"red": [[10, 40], [255, 255]], "green": [[10, 40], [255, 255]], "blue": [[10, 40], [255, 255]]});
    let white = curves_eyedropper(&base, CurvesEyedropper::White, [WHITE; 3], ColorMode::Rgb).unwrap();
    for curve in channels(&white).1 {
        assert!(close(&curve[0], 10.0 / 255.0, 40.0 / 255.0), "{curve:?}");
        assert!(close(&curve[1], WHITE, 1.0), "{curve:?}");
    }
}

/// A first pick on a curve with inner points scales them as before: inputs times the sample for
/// white, into `sample..1` for black.
#[test]
fn a_first_pick_scales_inner_points_as_before() {
    let base = json!({
        "red": [[0, 0], [100, 130], [255, 255]],
        "green": [[0, 0], [110, 125], [255, 255]],
        "blue": [[0, 0], [90, 120], [255, 255]]
    });
    let inner = [(100.0, 130.0), (110.0, 125.0), (90.0, 120.0)].map(|(i, o)| (i / 255.0, o / 255.0));

    let white = curves_eyedropper(&base, CurvesEyedropper::White, [WHITE; 3], ColorMode::Rgb).unwrap();
    for (curve, (input, output)) in channels(&white).1.iter().zip(inner) {
        assert!(close(&curve[0], 0.0, 0.0), "{curve:?}");
        assert!(close(&curve[1], input * WHITE, output), "{curve:?}");
        assert!(close(&curve[2], WHITE, 1.0), "{curve:?}");
    }

    let black = curves_eyedropper(&base, CurvesEyedropper::Black, [BLACK; 3], ColorMode::Rgb).unwrap();
    for (curve, (input, output)) in channels(&black).1.iter().zip(inner) {
        assert!(close(&curve[0], BLACK, 0.0), "{curve:?}");
        assert!(close(&curve[1], BLACK + input * (1.0 - BLACK), output), "{curve:?}");
        assert!(close(&curve[2], 1.0, 1.0), "{curve:?}");
    }
}

/// After the other end point was moved, inner points are remapped between the kept end point and
/// the sample (scaling them by the sample would give 54.6 here, and could push one past the black
/// end point).
#[test]
fn inner_points_are_remapped_between_the_end_points() {
    let base = json!({"red": [[0, 0], [40, 60], [255, 255]], "green": [[0, 0], [40, 60], [255, 255]], "blue": [[0, 0], [40, 60], [255, 255]]});
    let adjustment = picks(base, [(CurvesEyedropper::Black, BLACK), (CurvesEyedropper::White, WHITE)]);
    // After the black pick the inner point sits at 32 + 40 × 223/255, kept to hundredths of a level.
    let after_black = ((32.0 + 40.0 * 223.0 / 255.0) * 100.0_f32).round() / 100.0;
    let inner = (32.0 + (after_black - 32.0) * 176.0 / 223.0) / 255.0;
    for curve in channels(&adjustment).1 {
        assert_eq!(curve.len(), 3, "{curve:?}");
        assert!(close(&curve[0], BLACK, 0.0) && close(&curve[2], WHITE, 1.0), "{curve:?}");
        assert!(close(&curve[1], inner, 60.0 / 255.0), "the inner point lands at 59.6 and keeps its output: {curve:?}");
    }
}

/// A second pick with the same eyedropper replaces the first: inner points end where a single
/// pick on the second sample puts them, not scaled twice.
#[test]
fn a_repeat_pick_does_not_compound() {
    let base = json!({"red": [[0, 0], [100, 130], [255, 255]], "green": [[0, 0], [100, 130], [255, 255]], "blue": [[0, 0], [100, 130], [255, 255]]});
    let second = 200.0 / 255.0;
    let adjustment = picks(base, [(CurvesEyedropper::White, WHITE), (CurvesEyedropper::White, second)]);
    for curve in channels(&adjustment).1 {
        assert_eq!(curve.len(), 3, "{curve:?}");
        assert!(close(&curve[0], 0.0, 0.0) && close(&curve[2], second, 1.0), "{curve:?}");
        assert!(close(&curve[1], 100.0 / 255.0 * second, 130.0 / 255.0), "{curve:?}");
    }
}

/// A white pick at or below the black end point (or the reverse) would cross the end points, so
/// it keeps scaling every point toward the sample.
#[test]
fn a_pick_past_the_other_end_point_scales_every_point() {
    let black = 160.0 / 255.0;
    let white = 120.0 / 255.0;
    let adjustment = picks(json!({}), [(CurvesEyedropper::Black, black), (CurvesEyedropper::White, white)]);
    for curve in channels(&adjustment).1 {
        assert_eq!(curve.len(), 2, "{curve:?}");
        assert!(close(&curve[0], black * white, 0.0), "{curve:?}");
        assert!(close(&curve[1], white, 1.0), "{curve:?}");
    }

    let adjustment = picks(json!({}), [(CurvesEyedropper::White, white), (CurvesEyedropper::Black, black)]);
    for curve in channels(&adjustment).1 {
        assert_eq!(curve.len(), 2, "{curve:?}");
        assert!(close(&curve[0], black, 0.0), "{curve:?}");
        assert!(close(&curve[1], black + white * (1.0 - black), 1.0), "{curve:?}");
    }
}

/// Grayscale picks edit the composite curve the same way.
#[test]
fn grayscale_picks_move_only_their_own_end_point() {
    let black = curves_eyedropper(&json!({}), CurvesEyedropper::Black, [BLACK; 3], ColorMode::Grayscale).unwrap();
    let both = curves_eyedropper(&to_params(&black), CurvesEyedropper::White, [WHITE; 3], ColorMode::Grayscale).unwrap();
    let (master, _) = channels(&both);
    assert!(master.len() == 2 && close(&master[0], BLACK, 0.0) && close(&master[1], WHITE, 1.0), "{master:?}");
}
