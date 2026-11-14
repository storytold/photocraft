use super::*;

use crate::jobs::historical_xcf_fixture as fixture;

#[allow(clippy::unwrap_used)]
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":1,"height":1,"depth":16})).unwrap();
    s
}

#[test]
fn historical_sample_order_reaches_both_place_commands_and_open_as_smart_object() {
    let dir = tempfile::tempdir().unwrap();
    for (little, order) in [(true, "little-endian"), (false, "big-endian")] {
        let bytes = fixture::historical_pixel(little);
        let path = dir.path().join(format!("historical-{order}.xcf"));
        std::fs::write(&path, &bytes).unwrap();
        for command in ["file.placeEmbedded", "file.placeLinked", "file.openAsSmartObject"] {
            let mut s = session();
            s.execute(command, json!({"path":path,"historicalXcfSampleOrder":order})).unwrap();
            let state = s.active().unwrap();
            let layer = state.doc.layer(state.active_layer.unwrap()).unwrap();
            let LayerContent::Smart(smart) = &layer.content else { panic!("smart placement") };
            let mut pixel = [0.0; 4];
            smart.cache.as_ref().unwrap().read_pixel(0, 0, &mut pixel);
            assert_eq!(pixel, [0x1234 as f32 / 65535., 0x4567 as f32 / 65535., 0x89ab as f32 / 65535., 1.], "{command}, {order}");
            match &smart.source {
                SmartSource::Embedded { bytes: retained, .. } => assert_eq!(retained.as_slice(), bytes),
                SmartSource::Linked { path: retained } => assert_eq!(std::path::Path::new(retained), path),
            }
            assert_eq!(std::fs::read(&path).unwrap(), bytes, "source is never rewritten");
            if command == "file.openAsSmartObject" {
                assert!(state.path.is_none(), "new smart document cannot overwrite its source");
                assert_eq!(s.documents().len(), 2);
            } else {
                assert_eq!(s.documents().len(), 1);
            }
        }
    }
}

#[test]
fn absent_or_invalid_historical_place_choice_is_transactional_and_never_inferred() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("historical.xcf");
    let bytes = fixture::historical_pixel(true);
    std::fs::write(&path, &bytes).unwrap();
    for command in ["file.placeEmbedded", "file.placeLinked", "file.openAsSmartObject"] {
        for choice in [None, Some(Value::Null), Some(json!("native")), Some(json!(1)), Some(json!(true))] {
            let mut s = session();
            let before = s.active().unwrap().doc.clone();
            let history = s.active().unwrap().history.past_len();
            let mut params = json!({"path":path});
            if let Some(choice) = choice.clone() {
                params["historicalXcfSampleOrder"] = choice;
            }
            let error = s.execute(command, params).unwrap_err();
            if choice.is_some_and(|value| !value.is_null()) {
                assert!(matches!(error, EngineError::BadParams { cmd, .. } if cmd == command));
            }
            assert_eq!(s.documents().len(), 1);
            assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
            assert_eq!(s.active().unwrap().history.past_len(), history);
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
        // Invalid choices fail before attempting any file read or scheduling a job.
        let mut s = session();
        let error = s.start(command, json!({"path":dir.path().join("missing.xcf"),"historicalXcfSampleOrder":"native"})).unwrap_err();
        assert!(matches!(error, EngineError::BadParams { cmd, .. } if cmd == command));
    }
}

#[test]
fn byte_placement_options_keep_approval_and_sample_order_explicit_per_source() {
    let s = session();
    for linked in [None, Some("linked.xcf".to_string())] {
        let input = open::PlaceInput::new(
            &s,
            "old.xcf",
            fixture::historical_pixel(true),
            linked.clone(),
            &json!({"approveHighMemory":true,"historicalXcfSampleOrder":"little-endian"}),
        )
        .unwrap();
        assert!(input.opts.psd_high_memory_approved);
        assert_eq!(input.opts.psd_approved_estimate, None);
        assert_eq!(input.opts.historical_xcf_sample_order, Some(photocraft_io::HistoricalXcfSampleOrder::LittleEndian));
        let fresh = open::PlaceInput::new(&s, "next.xcf", fixture::historical_pixel(false), linked, &json!({})).unwrap();
        assert!(!fresh.opts.psd_high_memory_approved);
        assert_eq!(fresh.opts.psd_approved_estimate, None);
        assert_eq!(fresh.opts.historical_xcf_sample_order, None);
    }
}
