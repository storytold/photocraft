//! Curves points handed to a command as params (#14444), measured against the reference
//! application: they are sorted by input, two points with the same input keep the later one, and
//! 2 to 16 points are accepted, and fractions are truncated to whole levels. Points the engine wrote itself - `curve_arr` at hundredths of a
//! level - keep their params round trip.

use super::*;

/// One curve of a Curves param set, as parsed: `points` (the composite) or a channel key.
fn curve(params: &Value, key: &str) -> Vec<CurvePoint> {
    let Adjustment::Curves { master, per_channel, .. } =
        from_command_params("curves", params, None, ColorMode::Rgb).unwrap_or_else(|e| panic!("{params}: {e}"))
    else {
        panic!("expected a Curves adjustment from {params}")
    };
    match key {
        "points" => master,
        "red" => per_channel[0].clone(),
        other => panic!("no curve for `{other}` in the test params"),
    }
}

/// The 256 outputs of `pts` over a 0..255 input ramp, as the renderer samples a point curve.
fn ramp(pts: &[CurvePoint]) -> Vec<f32> {
    photocraft_compose::adjust::curve_samples::<CURVE_MAP_LEN>(pts)
}

/// The ramp a Curves param set renders.
fn param_ramp(params: &Value) -> Vec<f32> {
    ramp(&curve(params, "points"))
}

/// How many of the two param sets' 256 ramp outputs differ by a quarter of a level or more.
fn ramps_off(a: &Value, b: &Value) -> usize {
    let (a, b) = (param_ramp(a), param_ramp(b));
    a.iter().zip(b.iter()).filter(|pair| (*pair.0 - *pair.1).abs() > 0.25 / 255.0).count()
}

#[test]
fn equal_input_points_keep_the_later_one() {
    let kept = json!({"points": [[0, 0], [128, 200], [255, 255]]});
    let doubled = json!({"points": [[0, 0], [128, 100], [128, 200], [255, 255]]});
    assert_eq!(ramps_off(&doubled, &kept), 0, "128,100 then 128,200 gives a single point at 128,200");
    assert_eq!(curve(&doubled, "points"), curve(&kept, "points"), "the point given last wins");
    assert_eq!(param_ramp(&doubled), param_ramp(&kept), "and the whole ramp follows it");
    // Given the other way round the other point wins; the order of the array does not matter.
    let swapped = json!({"points": [[0, 0], [128, 200], [128, 100], [255, 255]]});
    assert_eq!(curve(&swapped, "points"), curve(&json!({"points": [[0, 0], [128, 100], [255, 255]]}), "points"));
    let unsorted = json!({"points": [[255, 255], [128, 100], [128, 200], [0, 0]]});
    assert_eq!(curve(&unsorted, "points"), curve(&kept, "points"), "sorted by input, then the later duplicate wins");
    // A channel curve follows the same rule.
    let channel = json!({"red": [[0, 0], [128, 100], [128, 200], [255, 255]]});
    assert_eq!(
        curve(&channel, "red"),
        curve(&json!({"red": [[0, 0], [128, 200], [255, 255]]}), "red"),
        "a channel keeps the later of its two points at level 128 too"
    );
    // Two points closer than half a level collapse the same way: the later of them stays.
    let close = json!({"points": [[0, 0], [128.2, 100], [128.4, 140], [255, 255]]});
    assert_eq!(curve(&close, "points").len(), 3, "closer than half a level is still one point");
    assert!(!curve(&close, "points").iter().any(|p| (p.output * 255.0 - 100.0).abs() < 1e-3), "the later of them wins");
}

#[test]
fn params_take_16_points_and_refuse_17() {
    let at = |n: usize, step: usize| json!({"points": (0..n).map(|k| [k * step, k * step]).collect::<Vec<_>>()});
    assert!(from_command_params("curves", &at(16, 16), None, ColorMode::Rgb).is_ok(), "16 points work");
    for (n, step) in [(17, 14), (18, 14), (19, 13), (20, 13), (64, 4)] {
        let e = from_command_params("curves", &at(n, step), None, ColorMode::Rgb).unwrap_err().to_string();
        assert!(e.contains("2..=16") && !e.contains("19"), "{n} points are refused by a message naming 16: {e}");
    }
    // A curve read from a file or placed by the engine's own edits keeps the wider record limit.
    assert_eq!(MAX_CURVE_POINTS, 19);
    assert_eq!(curve(&json!({"red": (0..16).map(|k| [k * 16, k * 16]).collect::<Vec<_>>()}), "red").len(), 16);
}

#[test]
fn points_the_engine_wrote_keep_their_round_trip() {
    // `curve_arr` writes hundredths of a 0..255 level, and an eyedropper places its point at a
    // sampled level. Reading that back must give the same params again, and points closer than one
    // level apart must not be merged on the way.
    for picker in [CurvesEyedropper::Black, CurvesEyedropper::White] {
        for sample in [0.2_f32, 0.6, 0.494 / 255.0, 0.496 / 255.0, 1.0 - 0.494 / 255.0, 1.0 - 0.496 / 255.0] {
            let a = curves_eyedropper(&json!({}), picker, [sample; 3], ColorMode::Rgb).unwrap();
            let params = to_params(&a);
            let back = from_params("curves", &params, None, ColorMode::Rgb).unwrap_or_else(|e| panic!("{picker:?} at {sample}: {params}: {e}"));
            assert_eq!(to_params(&back), params, "{picker:?} at level {sample} survives to_params/from_params");
        }
    }
    // A neutral-gray anchor sits at a sampled level between other points; the curve it wrote reads
    // back as it was written.
    let a = curves_eyedropper(&json!({"red": [[0, 0], [80, 90], [255, 255]]}), CurvesEyedropper::NeutralGray, [0.88, 0.76, 0.63], ColorMode::Rgb).unwrap();
    let params = to_params(&a);
    assert_eq!(to_params(&from_params("curves", &params, None, ColorMode::Rgb).unwrap()), params, "a gray point keeps its anchor");
    // Two points a fraction of a level apart - only the engine places those - keep their positions.
    let tight = json!({"points": [[0, 0], [100.25, 100.5], [100.85, 120.25], [255, 255]]});
    let a = from_params("curves", &tight, None, ColorMode::Rgb).unwrap();
    assert!(matches!(&a, Adjustment::Curves { master, .. } if master.len() == 4), "a point closer than a level is not merged");
    assert_eq!(to_params(&from_params("curves", &to_params(&a), None, ColorMode::Rgb).unwrap()), to_params(&a));
}

#[test]
fn command_params_truncate_fractions_to_whole_levels() {
    let whole = json!({"points": [[0, 0], [127, 160], [255, 255]]});
    let fractional = json!({"points": [[0, 0], [127.5, 160.4], [255, 255]]});
    assert_eq!(ramps_off(&fractional, &whole), 0, "127.5,160.4 acts as 127,160");
    assert_eq!(param_ramp(&fractional), param_ramp(&whole), "the whole 256-level ramp is the same");
    // A channel curve truncates too, and so do two points that land on one level.
    let channel = json!({"red": [[0, 0], [127.9, 160.9], [255, 255]]});
    assert_eq!(curve(&channel, "red"), curve(&json!({"red": [[0, 0], [127, 160], [255, 255]]}), "red"));
    let same_level = json!({"points": [[0, 0], [127.2, 100.7], [127.9, 160.4], [255, 255]]});
    assert_eq!(curve(&same_level, "points"), curve(&whole, "points"), "both become input 127, the later one wins");
    // Points given as whole numbers are untouched.
    assert_eq!(curve(&whole, "points").len(), 3);
}

#[test]
fn serialized_curves_keep_17_to_19_points_and_hundredths_through_from_params() {
    // A curve a file carries may have up to 19 points and an eyedropper anchors at hundredths of a
    // level; `from_params` is the inverse of `to_params` for both, only a command's own params are
    // held to 16 points and whole levels.
    for n in [17usize, 19] {
        let pts: Vec<[f64; 2]> = (0..n).map(|k| [k as f64 * 13.25, k as f64 * 13.25]).collect();
        let params = json!({"points": pts});
        let a = from_params("curves", &params, None, ColorMode::Rgb).unwrap();
        assert!(matches!(&a, Adjustment::Curves { master, .. } if master.len() == n), "{n} points survive from_params");
        assert_eq!(from_params("curves", &to_params(&a), None, ColorMode::Rgb).unwrap(), a, "{n} points round-trip");
        assert!(from_command_params("curves", &params, None, ColorMode::Rgb).is_err(), "but {n} points handed to a command are refused");
    }
    let anchored = json!({"points": [[0, 0], [43.35, 33.2], [255, 255]]});
    let a = from_params("curves", &anchored, None, ColorMode::Rgb).unwrap();
    let back = to_params(&a);
    let level = |k: usize, i: usize| back["points"][k][i].as_f64().unwrap_or(f64::NAN);
    assert!((level(1, 0) - 43.35).abs() < 1e-3 && (level(1, 1) - 33.2).abs() < 1e-3, "hundredths survive: {back}");
}

/// A curve a file carries with 17 to 19 points, on an adjustment layer: the Properties panel's
/// eyedropper and a command editing another curve of it read it back through `to_params` and
/// `from_params`, which keep the file's limit; only points a command is given are held to 16.
#[test]
fn an_imported_19_point_curve_survives_the_eyedropper_and_an_edit() {
    let pts: Vec<[f64; 2]> = (0..19).map(|k| [k as f64 * 14.0, (k as f64 * 14.0 + 6.0).min(255.0)]).collect();
    let imported = from_params("curves", &json!({"points": pts}), None, ColorMode::Rgb).unwrap();
    assert!(matches!(&imported, Adjustment::Curves { master, .. } if master.len() == 19));
    let mut s = crate::Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("edit.fill", json!({"color": [0.6, 0.5, 0.4]})).unwrap();
    s.execute("layer.newAdjustmentLayer.curves", json!({})).unwrap();
    let place = |s: &mut crate::Session, adjustment: &Adjustment| {
        s.edit("Import", |doc, _| {
            let id = doc.layers.last().map(|l| l.id).ok_or(crate::EngineError::Other("no layer".into()))?;
            let l = doc.layer_mut(id).ok_or(crate::EngineError::NoLayer(id))?;
            l.content = photocraft_doc::LayerContent::Adjustment(adjustment.clone());
            Ok(())
        })
        .unwrap();
    };
    let master_len = |s: &crate::Session| match s.active().unwrap().doc.layers.last().map(|l| l.content.clone()) {
        Some(photocraft_doc::LayerContent::Adjustment(Adjustment::Curves { master, .. })) => master.len(),
        other => panic!("expected a Curves layer, got {other:?}"),
    };
    for point in ["black", "white"] {
        place(&mut s, &imported);
        s.execute("layer.setAdjustment", json!({"eyedropper": {"point": point, "at": [2, 2]}})).unwrap_or_else(|e| panic!("{point} on 19 points: {e}"));
        assert_eq!(master_len(&s), 19, "the {point} point replaces an end of the 19-point curve");
    }
    // Editing another curve of the layer merges over the 19-point base without touching it.
    place(&mut s, &imported);
    s.execute("layer.setAdjustment", json!({"red": [[0, 0], [100, 120], [255, 255]]})).unwrap();
    assert_eq!(master_len(&s), 19, "the composite curve keeps its 19 points");
    // An eyedropper with another key of the same call reads that key over the 19-point base too.
    place(&mut s, &imported);
    s.execute("layer.setAdjustment", json!({"red": [[0, 0], [100, 120], [255, 255]], "eyedropper": {"point": "white", "at": [2, 2]}})).unwrap();
    assert_eq!(master_len(&s), 19);
    // A command's own points stay held to 16, on a layer with that base as well.
    let seventeen: Vec<[usize; 2]> = (0..17).map(|k| [k * 15, k * 15]).collect();
    place(&mut s, &imported);
    assert!(s.execute("layer.setAdjustment", json!({"points": seventeen})).is_err(), "17 points handed to a command are refused");
    // The same call that also asks for an eyedropper reads its own points as a command's.
    place(&mut s, &imported);
    let with_pick = json!({"points": seventeen, "eyedropper": {"point": "white", "at": [2, 2]}});
    assert!(s.execute("layer.setAdjustment", with_pick).is_err(), "17 points handed to a command are refused beside an eyedropper");
}
