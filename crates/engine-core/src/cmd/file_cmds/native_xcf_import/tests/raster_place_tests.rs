use super::*;
use crate::file_cmds::{PlaceInput, PlaceSteps, apply_place};
use crate::jobs::{JobCtx, StepWork};
use photocraft_doc::{LayerId, Rect};
use std::sync::Arc;

fn destination(depth: u32) -> crate::Session {
    let mut session = session();
    session.execute("file.new", json!({"width":1,"height":1,"depth":depth})).unwrap();
    session
}

fn prepare(input: PlaceInput, fine: bool, ctx: &JobCtx) -> Result<super::super::super::open::Prepared> {
    let mut steps = PlaceSteps::new(input, fine);
    for _ in 0..32 {
        if let Some(prepared) = steps.step(ctx, web_time::Instant::now())? {
            return Ok(prepared);
        }
    }
    panic!("one-pixel Place must finish its bounded stages");
}

fn pixel(session: &crate::Session, result: &Value) -> Vec<f32> {
    let layer = session.active().unwrap().doc.layer(LayerId(result["layer"].as_u64().unwrap())).unwrap();
    let LayerContent::Raster(surface) = &layer.content else { panic!("explicit raster Place must publish a raster layer") };
    surface.read_region(Rect::new(0, 0, 1, 1))
}

fn assert_close(actual: &[f32], expected: &[f32]) {
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        assert!((actual - expected).abs() < 0.000001, "{actual} versus {expected}");
    }
}

#[test]
fn raster_place_native_tones_match_direct_commands_and_private_preparation() {
    let bytes = fixture();
    // Ordinary IO remains inert; an independent direct-command sequence gives
    // the native semantic oracle (Exposure then legacy contrast), including HDR.
    let inert = photocraft_io::import("inert.xcf", &bytes).unwrap();
    let mut oracle = session();
    oracle.add_document(inert.document, None);
    oracle.execute("image.adjustments.exposure", json!({"exposure":1.,"offset":0.,"gamma":1.})).unwrap();
    oracle.execute("image.adjustments.brightnessContrast", json!({"brightness":0.,"contrast":-50.,"legacy":true})).unwrap();
    let expected = oracle.active().unwrap().doc.layers[0].surface().unwrap().read_region(Rect::new(0, 0, 1, 1));
    // The corpus-measured F32 legacy contrast pivot is 127/255, not 1/2.
    // Exposure doubles the samples before -50 contrast halves their distance from that pivot.
    assert_close(&expected, &[127.25 / 255., 191. / 255., 573.5 / 255., 1.]);
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("tones.xcf");
    std::fs::write(&path, &bytes).unwrap();
    let params = json!({"path":path,"smartObject":false,"frame":false,"fit":false});
    let mut direct = destination(32);
    let before = direct.active().unwrap().history.past_len();
    let result = direct.execute("file.placeEmbedded", params.clone()).unwrap();
    assert_close(&pixel(&direct, &result), &expected);
    assert_eq!(direct.active().unwrap().history.past_len(), before + 1);
    for fine in [false, true] {
        let mut private = destination(32);
        let input = PlaceInput::new(&private, "tones.xcf", bytes.clone(), None, &params).unwrap();
        let prepared = prepare(input, fine, &JobCtx::new()).unwrap();
        let result = apply_place(&mut private, prepared).unwrap();
        assert_close(&pixel(&private, &result), &expected);
    }
    assert_eq!(std::fs::read(path).unwrap(), bytes);
    let inert = photocraft_io::import("after-place.xcf", &bytes).unwrap();
    assert!(matches!(inert.document.layers[0].content, LayerContent::Raster(_)), "Place must not leak native import authority");
}

#[test]
fn raster_place_keeps_captured_authority_and_denial_or_cancellation_is_atomic() {
    let bytes = fixture();
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("tones.xcf");
    std::fs::write(&path, &bytes).unwrap();
    let params = json!({"smartObject":false,"frame":false,"fit":false});
    for (gate, cancelled) in [(deny_contrast as Gate, false), (cancel_contrast as Gate, true)] {
        for fine in [false, true] {
            let mut direct = destination(32);
            direct.authorize = Some(gate);
            direct.set_cooperative_jobs(fine);
            let before = direct.active().unwrap().doc.clone();
            let error = direct.execute("file.placeEmbedded", json!({"path":path,"smartObject":false,"frame":false,"fit":false})).unwrap_err();
            if cancelled {
                assert_eq!(error, EngineError::Cancelled);
            } else {
                assert!(error.to_string().contains("not authorized"), "{error}");
            }
            assert!(Arc::ptr_eq(&direct.active().unwrap().doc, &before));
            let mut session = destination(32);
            session.authorize = Some(gate);
            let input = PlaceInput::new(&session, "tones.xcf", bytes.clone(), None, &params).unwrap();
            session.authorize = None; // later callers must not weaken the captured request
            let before = session.active().unwrap().clone();
            let count = photocraft_io::native_xcf_tones::preparation_count();
            let error = match prepare(input, fine, &JobCtx::new()) {
                Ok(_) => panic!("a refused native stage cannot produce Place output"),
                Err(error) => error,
            };
            if cancelled {
                assert_eq!(error, EngineError::Cancelled);
            } else {
                assert!(error.to_string().contains("not authorized"), "{error}");
            }
            assert!(Arc::ptr_eq(&session.active().unwrap().doc, &before.doc));
            assert_eq!(session.active().unwrap().history.past_len(), before.history.past_len());
            assert_eq!(photocraft_io::native_xcf_tones::preparation_count(), count);
        }
    }
    let session = destination(32);
    let input = PlaceInput::new(&session, "tones.xcf", bytes, None, &params).unwrap();
    let before = session.active().unwrap().doc.clone();
    let ctx = JobCtx::new();
    ctx.cancel();
    assert!(matches!(prepare(input, true, &ctx), Err(EngineError::Cancelled)));
    assert!(Arc::ptr_eq(&session.active().unwrap().doc, &before));
}

#[test]
fn raster_svg_placement_keeps_ordinary_pixels_without_native_xcf_preparation() {
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1" fill="#00ff00"/></svg>"##;
    let mut session = destination(8);
    session.authorize = Some(deny_contrast);
    let before = photocraft_io::native_xcf_tones::preparation_count();
    let placed =
        crate::file_cmds::place_bytes(&mut session, "ordinary.svg", svg.to_vec(), None, &json!({"smartObject":false,"frame":false,"fit":false})).unwrap();
    assert_eq!(pixel(&session, &placed), vec![0., 1., 0., 1.]);
    assert_eq!(photocraft_io::native_xcf_tones::preparation_count(), before);
}

#[test]
fn raster_place_preserves_explicit_historical_samples_and_refuses_missing_or_deferred_choices() {
    use crate::jobs::historical_fixture::historical_rgba16;
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("historical.xcf");
    for (order, little) in [("big-endian", false), ("little-endian", true)] {
        let mut bytes = historical_rgba16();
        if little {
            let tile = bytes.len() - 8;
            for word in bytes[tile..].as_chunks_mut::<2>().0 {
                word.swap(0, 1);
            }
        }
        std::fs::write(&path, &bytes).unwrap();
        let params = json!({"path":path,"smartObject":false,"frame":false,"fit":false,"historicalXcfSampleOrder":order});
        let expected = [0x1234u16, 0x2345, 0x3456, 0xffff].map(|value| f32::from(value) / 65535.);
        let mut session = destination(16);
        let direct = session.execute("file.placeEmbedded", params.clone()).unwrap();
        assert_eq!(pixel(&session, &direct), expected);
        let mut private = destination(16);
        let input = PlaceInput::new(&private, "historical.xcf", bytes.clone(), None, &params).unwrap();
        let prepared = prepare(input, true, &JobCtx::new()).unwrap();
        let placed = apply_place(&mut private, prepared).unwrap();
        assert_eq!(pixel(&private, &placed), expected);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        let mut missing = destination(16);
        let before = missing.active().unwrap().doc.clone();
        assert!(
            missing
                .execute("file.placeEmbedded", json!({"path":path,"smartObject":false,"frame":false}))
                .unwrap_err()
                .to_string()
                .contains("sample byte order")
        );
        assert!(Arc::ptr_eq(&before, &missing.active().unwrap().doc));
    }
    for params in [
        json!({"path":"missing.xcf","smartObject":true,"historicalXcfSampleOrder":"little-endian"}),
        json!({"path":"missing.xcf","smartObject":false,"historicalXcfSampleOrder":"native"}),
    ] {
        let mut session = destination(16);
        let before = session.active().unwrap().doc.clone();
        assert!(matches!(session.execute("file.placeEmbedded", params), Err(EngineError::BadParams { .. })));
        assert!(Arc::ptr_eq(&before, &session.active().unwrap().doc));
    }
}
