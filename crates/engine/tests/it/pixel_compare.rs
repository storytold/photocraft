//! Independent contract tests for read-only comparisons with History snapshots.
use std::sync::Arc;

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, LayerMask, Slice, SliceOrigin};
use photocraft_engine::{EngineError, Session, inspect};
use photocraft_geom::{Rect, Size};
use photocraft_raster::Surface;
use serde_json::{Value, json};

const COMMAND: &str = "document.comparePixels";

/// Creates one opaque raster without exports or preview quantization.
fn session(mode: ColorMode, depth: SampleType, alpha: bool, size: Size) -> Session {
    let format = PixelFormat::new(mode, depth, alpha);
    let mut fill = vec![0.0; format.channels()];
    if alpha {
        *fill.last_mut().unwrap() = 1.0;
    }
    let mut pixels = Surface::new(format);
    pixels.fill_rect(Rect::from_size(size), &fill);
    let mut doc = Document::new("Comparison fixture", size, mode, depth);
    doc.layers.push(Layer::new("Pixels", LayerContent::Raster(pixels)));
    let mut session = Session::new();
    session.add_document(doc, None);
    session
}

/// Stores the current rendered baseline through the existing History API.
fn snapshot(session: &mut Session) -> u64 {
    session.execute("history.newSnapshot", json!({})).unwrap()["snapshot"].as_u64().unwrap()
}

/// Applies controlled native-sample edits as one ordinary undoable operation.
fn write_pixels(session: &mut Session, pixels: &[(i32, i32, Vec<f32>)]) {
    session
        .edit("Fixture pixels", |doc, active| {
            let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            for (x, y, color) in pixels {
                surface.write_pixel(*x, *y, color);
            }
            Ok(())
        })
        .unwrap();
}

/// Expresses half-open pixel bounds through the public command's size fields.
fn region(x: i32, y: i32, width: u32, height: u32) -> Value {
    json!({"x": x, "y": y, "width": width, "height": height})
}

/// Runs the opt-in query without selecting or restoring its reference snapshot.
fn compare(session: &mut Session, snapshot: u64, regions: Vec<Value>, tolerance: Option<f64>) -> Value {
    let mut params = json!({"snapshot": snapshot, "regions": regions});
    if let Some(tolerance) = tolerance {
        params["tolerance"] = json!(tolerance);
    }
    session.execute(COMMAND, params).unwrap()
}

/// Captures observable editing state, including the action-recording journal.
fn state(session: &Session) -> Value {
    let active = session.active().unwrap();
    json!({
        "document": inspect::document(active), "session": inspect::session(session),
        "journal": session.journal, "savedRevision": active.saved_revision,
        "viewRevision": active.view_revision, "damage": active.last_damage, "coalesce": active.coalesce,
    })
}

#[test]
fn exact_counts_include_alpha_and_maximum_is_independent_of_strict_tolerance() {
    let mut session = session(ColorMode::Rgb, SampleType::F32, true, Size::new(4, 3));
    let baseline = snapshot(&mut session);
    write_pixels(
        &mut session,
        &[
            (1, 0, vec![0.125, 0.0, 0.0, 1.0]),
            (2, 0, vec![0.25, 0.375, 0.0, 1.0]),
            (0, 1, vec![0.0, 0.0, 0.0, 0.5]),
            (3, 0, vec![0.0, 0.0, 1.0, 1.0]),
            (2, 2, vec![0.0, 0.0, 1.0, 1.0]),
        ],
    );
    let regions = vec![region(0, 0, 3, 2), region(0, 1, 1, 1)];
    let result = compare(&mut session, baseline, regions.clone(), None);
    assert_eq!(result["snapshot"], baseline);
    assert_eq!(result["space"], "composited-rgba-f32");
    assert_eq!(result["tolerance"].as_f64(), Some(0.0));
    assert_eq!(
        result["regions"],
        json!([
            {"rect": [0, 0, 3, 2], "comparedPixels": 6, "changedPixels": 3, "maxChannelDifference": 0.5},
            {"rect": [0, 1, 1, 2], "comparedPixels": 1, "changedPixels": 1, "maxChannelDifference": 0.5},
        ])
    );
    for (tolerance, changed) in [(0.125, 2), (0.375, 1), (0.5, 0), (1e300, 0)] {
        let result = compare(&mut session, baseline, regions.clone(), Some(tolerance));
        assert_eq!(result["regions"][0]["changedPixels"], changed, "tolerance {tolerance}");
        assert_eq!(result["regions"][0]["maxChannelDifference"].as_f64(), Some(0.5));
    }
}

#[test]
fn native_modes_and_depths_are_compared_in_the_declared_composite_space() {
    for mode in [ColorMode::Grayscale, ColorMode::Rgb, ColorMode::Cmyk] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut session = session(mode, depth, true, Size::new(2, 2));
            let baseline = snapshot(&mut session);
            let reference = session.active().unwrap().doc.clone();
            let mut color = vec![0.25; mode.color_channels() + 1];
            color[0] = 0.75;
            *color.last_mut().unwrap() = 0.5;
            write_pixels(&mut session, &[(0, 0, color)]);
            // A one-pixel compositor oracle checks mode conversion without
            // duplicating region traversal or depending on PNG/native byte values.
            let pixel = Rect::new(0, 0, 1, 1);
            let before = photocraft_compose::render(&reference, pixel).get(0, 0);
            let after = photocraft_compose::render(&session.active().unwrap().doc, pixel).get(0, 0);
            let expected = before.into_iter().zip(after).map(|(a, b)| (f64::from(a) - f64::from(b)).abs()).fold(0.0f64, f64::max);
            assert!(expected > 0.0, "{mode:?}/{depth:?}: fixture must change its composite");
            let result = compare(&mut session, baseline, vec![region(0, 0, 2, 2)], None);
            assert_eq!(result["regions"][0]["comparedPixels"], 4);
            assert_eq!(result["regions"][0]["changedPixels"], 1);
            assert_eq!(result["regions"][0]["maxChannelDifference"].as_f64(), Some(expected), "{mode:?}/{depth:?}");
        }
    }
}

#[test]
fn fractional_float_and_hdr_changes_survive_without_preview_clamping() {
    let mut session = session(ColorMode::Rgb, SampleType::F32, false, Size::new(2, 1));
    let baseline = snapshot(&mut session);
    write_pixels(&mut session, &[(0, 0, vec![4.5, 0.0, 0.0]), (1, 0, vec![1.0 / 65536.0, 0.0, 0.0])]);
    let result = compare(&mut session, baseline, vec![region(0, 0, 2, 1)], None);
    assert_eq!(result["regions"][0]["changedPixels"], 2);
    assert_eq!(result["regions"][0]["maxChannelDifference"].as_f64(), Some(4.5));
    let result = compare(&mut session, baseline, vec![region(1, 0, 1, 1)], Some(1.0 / 65536.0));
    assert_eq!(result["regions"][0]["changedPixels"], 0);
    assert_eq!(result["regions"][0]["maxChannelDifference"].as_f64(), Some(1.0 / 65536.0));
}

#[test]
fn noop_queries_use_canvas_regions_and_ignore_invisible_native_rgb() {
    let mut session = session(ColorMode::Rgb, SampleType::F32, true, Size::new(3, 2));
    write_pixels(&mut session, &[(0, 0, vec![0.0; 4])]);
    let baseline = snapshot(&mut session);
    let noop = compare(&mut session, baseline, vec![region(0, 0, 3, 2)], None);
    assert_eq!(noop["regions"][0]["changedPixels"], 0);
    assert_eq!(noop["regions"][0]["comparedPixels"], 6);
    write_pixels(&mut session, &[(0, 0, vec![0.75, 0.25, 0.5, 0.0])]);
    let result = compare(&mut session, baseline, vec![region(0, 0, 3, 2)], None);
    assert_eq!(result["regions"][0]["changedPixels"], 0);
    assert_eq!(result["regions"][0]["maxChannelDifference"].as_f64(), Some(0.0));
}

#[test]
fn a_snapshot_after_an_existing_warp_is_the_baseline_for_later_edits() {
    let mut session = session(ColorMode::Rgb, SampleType::U8, true, Size::new(64, 32));
    session
        .edit("Marker", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(12, 10, 14, 22), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    let original = session.active().unwrap().doc.clone();
    session.execute("filter.liquify", json!({"strokes": [{"tool": "forwardWarp", "size": 20, "points": [[12, 16], [18, 16]]}]})).unwrap();
    let warped = session.active().unwrap().doc.clone();
    let protected = Rect::new(0, 0, 32, 32);
    assert!(
        photocraft_compose::render(&original, protected).px != photocraft_compose::render(&warped, protected).px,
        "the first warp must already affect the protected region"
    );
    let baseline = snapshot(&mut session);
    write_pixels(&mut session, &[(60, 4, vec![0.0, 0.0, 1.0, 1.0])]);
    let result = compare(&mut session, baseline, vec![region(0, 0, 32, 32), region(0, 0, 64, 32)], None);
    assert_eq!(result["regions"][0]["changedPixels"], 0);
    assert_eq!(result["regions"][0]["maxChannelDifference"].as_f64(), Some(0.0));
    assert_eq!(result["regions"][1]["changedPixels"], 1);
}

#[test]
fn successful_and_rejected_queries_preserve_redo_journal_and_document_state() {
    let mut session = session(ColorMode::Rgb, SampleType::F32, true, Size::new(4, 3));
    let baseline = snapshot(&mut session);
    write_pixels(&mut session, &[(0, 0, vec![1.0, 0.0, 0.0, 1.0])]);
    assert!(session.undo());
    assert!(session.active().unwrap().history.can_redo());
    let active = session.active_mut().unwrap();
    let layer = active.active_layer;
    Arc::make_mut(&mut active.doc).slices.list.push(Slice { id: 1, origin: SliceOrigin::Layer, layer, rect: Rect::new(0, 0, 1, 1), ..Default::default() });
    let before = state(&session);
    let document = session.active().unwrap().doc.clone();
    let slices = document.slices.clone();
    for (params, valid) in
        [(json!({"snapshot": baseline, "regions": [region(0, 0, 4, 3)]}), true), (json!({"snapshot": baseline, "regions": [region(-1, 0, 1, 1)]}), false)]
    {
        assert!(matches!((session.execute(COMMAND, params), valid), (Ok(_), true) | (Err(EngineError::BadParams { .. }), false)));
        assert_eq!(state(&session), before);
        assert!(Arc::ptr_eq(&document, &session.active().unwrap().doc));
        assert_eq!(session.active().unwrap().doc.slices, slices, "inspection must not refresh stale layer-based slices");
    }
    assert!(session.redo(), "the query must not clear the existing redo branch");
    assert_eq!(photocraft_compose::render(&session.active().unwrap().doc, Rect::new(0, 0, 1, 1)).get(0, 0), [1.0, 0.0, 0.0, 1.0]);
    session.add_document(Document::new("Other", Size::new(4, 3), ColorMode::Rgb, SampleType::F32), None);
    let before = state(&session);
    assert!(
        matches!(session.execute(COMMAND, json!({"snapshot": baseline, "regions": [region(0, 0, 1, 1)]})), Err(EngineError::BadParams { .. })),
        "snapshot IDs belong to the active document"
    );
    assert_eq!(state(&session), before);
}

#[test]
fn malformed_parameters_and_stale_snapshot_ids_are_rejected_without_mutation() {
    let mut session = session(ColorMode::Rgb, SampleType::F32, true, Size::new(4, 3));
    let deleted = snapshot(&mut session);
    let baseline = snapshot(&mut session);
    session.execute("history.deleteSnapshot", json!({"snapshot": deleted})).unwrap();
    let valid = json!({"snapshot": baseline, "regions": [region(0, 0, 1, 1)]});
    let mut invalid = vec![json!({}), json!({"snapshot": baseline}), json!({"regions": [region(0, 0, 1, 1)]})];
    for id in [json!(deleted), json!(0), json!(-1), json!(1.5), json!("1"), json!(u64::MAX)] {
        let mut params = valid.clone();
        params["snapshot"] = id;
        invalid.push(params);
    }
    for regions in [
        json!([]),
        json!(null),
        json!({}),
        json!(vec![region(0, 0, 1, 1); 33]),
        json!([region(-1, 0, 1, 1)]),
        json!([region(0, -1, 1, 1)]),
        json!([region(4, 0, 1, 1)]),
        json!([region(0, 3, 1, 1)]),
        json!([region(0, 0, 0, 1)]),
        json!([region(0, 0, 1, 0)]),
        json!([region(i32::MAX, 0, 1, 1)]),
        json!([region(0, i32::MAX, 1, 1)]),
        json!([{"x": 0.5, "y": 0, "width": 1, "height": 1}]),
        json!([{"x": 0, "y": 0, "width": -1, "height": 1}]),
        json!([{"x": 0, "y": 0, "width": 4294967296u64, "height": 1}]),
    ] {
        let mut params = valid.clone();
        params["regions"] = regions;
        invalid.push(params);
    }
    for tolerance in [json!(-0.1), json!(null), json!("0"), json!(false)] {
        let mut params = valid.clone();
        params["tolerance"] = tolerance;
        invalid.push(params);
    }
    let before = state(&session);
    for params in invalid {
        assert!(matches!(session.execute(COMMAND, params.clone()), Err(EngineError::BadParams { .. })), "expected invalid parameters for {params}");
        assert_eq!(state(&session), before, "rejected {params} mutated editing state");
    }
    assert_eq!(session.execute(COMMAND, valid).unwrap()["regions"][0]["changedPixels"], 0, "a remaining snapshot ID must not be mistaken for its array index");
    assert!(Session::new().execute(COMMAND, json!({"snapshot": 1, "regions": [region(0, 0, 1, 1)]})).is_err());
}

#[test]
fn region_limits_count_overlaps_and_reject_incompatible_snapshot_metadata() {
    let mut large = session(ColorMode::Rgb, SampleType::U8, false, Size::new(1024, 1024));
    let baseline = snapshot(&mut large);
    let accepted = compare(&mut large, baseline, vec![region(0, 0, 1024, 512); 2], None);
    assert_eq!(accepted["regions"][0]["comparedPixels"], 524288);
    assert_eq!(accepted["regions"][1]["comparedPixels"], 524288);
    assert!(
        matches!(large.execute(COMMAND, json!({"snapshot": baseline, "regions": vec![region(0, 0, 1024, 513); 2]})), Err(EngineError::BadParams { .. })),
        "overlapping ROI areas still count against the sum budget"
    );
    assert_eq!(compare(&mut large, baseline, vec![region(0, 0, 1, 1); 32], None)["regions"].as_array().unwrap().len(), 32);
    for mismatch in ["bounds", "mode", "depth", "profile"] {
        let mut session = session(ColorMode::Rgb, SampleType::F32, true, Size::new(4, 3));
        let baseline = snapshot(&mut session);
        match mismatch {
            "mode" => {
                session.execute("image.mode.grayscale", json!({})).unwrap();
            }
            "depth" => {
                session.execute("image.mode.bits16", json!({})).unwrap();
            }
            _ => {
                session
                    .edit("Metadata mismatch", |doc, _| {
                        if mismatch == "bounds" {
                            doc.size = Size::new(5, 3);
                        } else {
                            doc.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
                        }
                        Ok(())
                    })
                    .unwrap();
            }
        }
        let before = state(&session);
        assert!(
            matches!(session.execute(COMMAND, json!({"snapshot": baseline, "regions": [region(0, 0, 1, 1)]})), Err(EngineError::BadParams { .. })),
            "expected invalid parameters for incompatible {mismatch}"
        );
        assert_eq!(state(&session), before);
    }
}

#[test]
fn queries_preserve_pending_and_stale_floating_selections_and_measure_only_committed_pixels() {
    let mut session = session(ColorMode::Rgb, SampleType::F32, true, Size::new(8, 4));
    write_pixels(&mut session, &[(0, 0, vec![1.0, 0.0, 0.0, 1.0]), (1, 0, vec![1.0, 0.0, 0.0, 1.0])]);
    let baseline = snapshot(&mut session);
    session.execute("select.rect", region(0, 0, 2, 1)).unwrap();
    write_pixels(&mut session, &[(6, 3, vec![0.0, 0.0, 1.0, 1.0])]);
    assert!(session.undo());
    session.execute("select.float", json!({"dx": 3, "dy": 1})).unwrap();
    let displayed = photocraft_engine::float_cmds::displayed(session.active().unwrap(), (0, 0)).unwrap();
    let canvas = Rect::new(0, 0, 8, 4);
    assert!(
        photocraft_compose::render(&displayed, canvas).px != photocraft_compose::render(&session.active().unwrap().doc, canvas).px,
        "the pending preview must differ from committed pixels"
    );
    for stale in [false, true] {
        if stale {
            session.active_mut().unwrap().floating.as_mut().unwrap().revision -= 1;
        }
        // Direct fields only: running document.inspect would itself drop the float.
        let before = state(&session);
        let document = session.active().unwrap().doc.clone();
        let pending = session.active().unwrap().floating.clone().unwrap();
        assert!(session.active().unwrap().history.can_redo());
        for valid in [true, false] {
            let roi = if valid { region(0, 0, 8, 4) } else { region(-1, 0, 1, 1) };
            let result = session.execute(COMMAND, json!({"snapshot": baseline, "regions": [roi]}));
            assert!(matches!((&result, valid), (Ok(_), true) | (Err(EngineError::BadParams { .. }), false)));
            if let Ok(result) = result {
                assert_eq!(result["regions"][0]["changedPixels"], 0, "floating previews are outside the committed-composite comparison");
            }
            assert_eq!(state(&session), before, "stale={stale}, valid={valid}");
            let active = session.active().unwrap();
            assert!(Arc::ptr_eq(&active.doc, &document));
            let after = active.floating.as_ref().expect("comparison must retain the pending piece");
            assert_eq!((after.layer, after.offset, after.revision), (pending.layer, pending.offset, pending.revision));
            assert!(Arc::ptr_eq(&after.parts, &pending.parts));
        }
    }
}

#[test]
fn an_off_origin_region_crossing_a_tile_boundary_matches_a_full_masked_composite() {
    let mut session = session(ColorMode::Rgb, SampleType::F32, true, Size::new(520, 6));
    session
        .edit("Feathered mask", |doc, active| {
            let mut mask = LayerMask::reveal_all();
            mask.surface.fill_rect(Rect::new(254, 0, 259, 6), &[0.0]);
            mask.feather = 1.5;
            doc.layer_mut(active.unwrap()).unwrap().mask = Some(mask);
            Ok(())
        })
        .unwrap();
    let baseline = snapshot(&mut session);
    let reference = session.active().unwrap().doc.clone();
    write_pixels(&mut session, &[(255, 2, vec![0.5, 0.0, 0.0, 1.0]), (256, 3, vec![0.0, 0.25, 0.0, 0.75])]);
    let roi = Rect::new(253, 1, 260, 5);
    let before = photocraft_compose::render(&reference, roi);
    let after = photocraft_compose::render(&session.active().unwrap().doc, roi);
    let mut changed = 0;
    let mut maximum = 0.0f64;
    for y in roi.y0..roi.y1 {
        for x in roi.x0..roi.x1 {
            let deltas = before.get(x, y).into_iter().zip(after.get(x, y)).map(|(a, b)| (f64::from(a) - f64::from(b)).abs()).collect::<Vec<_>>();
            changed += usize::from(deltas.iter().any(|&delta| delta > 0.0));
            maximum = deltas.into_iter().fold(maximum, f64::max);
        }
    }
    assert_eq!(changed, 2, "both edits must remain visible through the feathered mask");
    let result = compare(&mut session, baseline, vec![region(253, 1, 7, 4)], None);
    assert_eq!(result["regions"][0]["comparedPixels"], 28);
    assert_eq!(result["regions"][0]["changedPixels"], changed);
    assert_eq!(result["regions"][0]["maxChannelDifference"].as_f64(), Some(maximum));
}

#[test]
fn universal_command_parameters_leave_comparison_semantics_and_state_unchanged() {
    let mut session = session(ColorMode::Rgb, SampleType::F32, true, Size::new(2, 1));
    let baseline = snapshot(&mut session);
    write_pixels(&mut session, &[(0, 0, vec![0.125, 0.0, 0.0, 1.0])]);
    let plain = json!({"snapshot": baseline, "regions": [region(0, 0, 2, 1)]});
    let expected = session.execute(COMMAND, plain.clone()).unwrap();
    assert_eq!(expected["regions"][0]["changedPixels"], 1);
    assert_eq!(expected["regions"][0]["maxChannelDifference"].as_f64(), Some(0.125));
    let before = state(&session);
    let document = session.active().unwrap().doc.clone();
    let mut universal = plain.clone();
    universal["coalesce"] = json!("qa-check");
    universal["target"] = json!("mask");
    assert_eq!(session.execute(COMMAND, universal).unwrap(), expected, "universal parameters must not retarget the composite comparison");
    assert_eq!(state(&session), before);
    assert!(Arc::ptr_eq(&document, &session.active().unwrap().doc));
    for unknown_region_field in [false, true] {
        let mut invalid = plain.clone();
        if unknown_region_field {
            invalid["regions"][0]["unknownRegionField"] = json!(true);
        } else {
            invalid["tolernace"] = json!(0.1);
        }
        assert!(matches!(session.execute(COMMAND, invalid), Err(EngineError::BadParams { .. })), "only documented universal keys may be ignored");
        assert_eq!(state(&session), before);
        assert!(Arc::ptr_eq(&document, &session.active().unwrap().doc));
    }
}
