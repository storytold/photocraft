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

fn active_smart(s: &Session) -> Option<&SmartObject> {
    let state = s.active()?;
    match &state.doc.layer(state.active_layer?)?.content {
        LayerContent::Smart(smart) => Some(smart),
        _ => None,
    }
}

#[test]
fn scripted_place_open_and_relink_ignore_unused_sample_order_for_modern_xcf() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("modern.xcf");
    let historical = dir.path().join("historical.xcf");
    std::fs::write(&historical, fixture::historical_pixel(true)).unwrap();
    let options = json!({"path":path,"historicalXcfSampleOrder":"little-endian"});
    for command in ["file.placeEmbedded", "file.placeLinked", "file.openAsSmartObject", "layer.smartObjects.relinkToFile"] {
        std::fs::write(&path, fixture::modern_pixel(0x1234)).unwrap();
        let mut s = session();
        if command == "layer.smartObjects.relinkToFile" {
            s.execute("file.placeLinked", json!({"path":historical,"historicalXcfSampleOrder":"little-endian"})).unwrap();
            assert!(active_smart(&s).unwrap().historical_xcf_link_digest.is_some());
        }
        s.execute(command, options.clone()).unwrap();
        let smart = active_smart(&s).unwrap();
        assert!(smart.historical_xcf_sample_order.is_none(), "{command}");
        assert!(smart.historical_xcf_link_digest.is_none(), "{command}");
        assert!((smart.cache.as_ref().unwrap().pixel(0, 0)[0] - f32::from(0x1234u16) / 65535.).abs() <= 1. / 65535.);
        if matches!(command, "file.placeLinked" | "layer.smartObjects.relinkToFile") {
            std::fs::write(&path, fixture::modern_pixel(0x4321)).unwrap();
            s.execute("layer.smartObjects.updateModifiedContent", json!({})).unwrap();
            assert!((active_smart(&s).unwrap().cache.as_ref().unwrap().pixel(0, 0)[0] - f32::from(0x4321u16) / 65535.).abs() <= 1. / 65535.);
        }
    }
}

#[test]
fn historical_place_resume_keeps_the_frame_and_original_file_bytes() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("frame.xcf");
    let original = fixture::historical_pixel(true);
    std::fs::write(&path, &original).unwrap();

    let mut s = session();
    let frame = s.execute("layer.new.frame", json!({"x":0,"y":0,"width":1,"height":1})).unwrap()["layer"].as_u64().unwrap();
    let original_doc = s.active().unwrap().doc.clone();
    let question = s.execute("file.placeEmbedded", json!({"path":path,"frame":frame,"askHistoricalXcfSampleOrder":true})).unwrap();
    let id = question["heldHistoricalXcfPlace"].as_u64().unwrap();
    assert!(Arc::ptr_eq(&s.active().unwrap().doc, &original_doc));
    std::fs::write(&path, fixture::historical_pixel(false)).unwrap();

    let crate::jobs::Started::Done(result) = s.resume_historical_xcf_open(id, photocraft_io::HistoricalXcfSampleOrder::LittleEndian, false).unwrap() else {
        panic!("inline frame placement");
    };
    assert_eq!(result["frame"], frame);
    let children = s.active().unwrap().doc.layer(LayerId(frame)).unwrap().children().unwrap();
    let LayerContent::Smart(smart) = &children[0].content else { panic!("embedded frame content") };
    let SmartSource::Embedded { bytes, .. } = &smart.source else { panic!("original embedded source") };
    assert_eq!(bytes.as_slice(), original);
    assert_eq!(smart.historical_xcf_sample_order, Some(photocraft_io::HistoricalXcfSampleOrder::LittleEndian));
    assert!(!s.holds_historical_xcf_open(id));
}

#[test]
fn a_wrapped_canvas_drop_probes_only_the_header_until_confirmation() {
    use std::io::Write;

    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&fixture::historical_pixel(true)).unwrap();
    let mut gzip = encoder.finish().unwrap();
    gzip.truncate(gzip.len() - 4); // The header is available but the full envelope cannot pass.
    let mut s = session();
    let before = s.active().unwrap().doc.clone();
    let crate::jobs::Started::Job(job) = s.start_place_bytes("source.xcf.gz", gzip, None, json!({"askHistoricalXcfSampleOrder":true})).unwrap() else {
        panic!("the wrapped drop is probed on a worker")
    };
    let reply = s.wait_job(job).unwrap();
    let held = reply["heldHistoricalXcfPlace"].as_u64().unwrap();
    assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
    assert!(s.resume_historical_xcf_open(held, photocraft_io::HistoricalXcfSampleOrder::LittleEndian, false).is_err());
    assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
}

#[test]
fn smart_object_open_decode_observes_cancellation_inside_wrapped_import() {
    use std::io::Write as _;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&fixture::historical_pixel(true)).unwrap();
    let bytes = encoder.finish().unwrap();
    let options =
        photocraft_io::ImportOptions { historical_xcf_sample_order: Some(photocraft_io::HistoricalXcfSampleOrder::LittleEndian), ..Default::default() };
    assert!(open::import_result_for_open("old.xcf.gz", &bytes, &options, &photocraft_raster::Interrupt::NONE).is_ok());

    let checks = AtomicUsize::new(0);
    let cancel = || checks.fetch_add(1, Ordering::Relaxed) >= 2;
    let interrupt = photocraft_raster::Interrupt::cancel_only(&cancel);
    let result = open::import_result_for_open("old.xcf.gz", &bytes, &options, &interrupt);
    assert!(matches!(result, Err(EngineError::Cancelled)));
    assert!(checks.load(Ordering::Relaxed) >= 3);
}

#[test]
fn folder_batch_and_image_processor_use_explicit_historical_xcf_order() {
    let folder = tempfile::tempdir().unwrap();
    let source = folder.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("big.xcf"), fixture::historical_pixel(false)).unwrap();
    for (id, name, mut params) in [
        ("file.automate.batch", "batch", json!({"format":"png","steps":[{"command":"layer.new.layer","params":{}}]})),
        ("file.automate.batch", "quiet", json!({"format":"png","suppressWarnings":true,"steps":[{"command":"layer.new.layer","params":{}}]})),
        ("file.scripts.imageProcessor", "simple", json!({"format":"png"})),
        ("file.scripts.imageProcessor", "multiple", json!({"tiff":{"enabled":true}})),
    ] {
        let mut s = session();
        let output = folder.path().join(name);
        params["input"] = json!(source);
        params["output"] = json!(output);
        let missing = s.execute(id, params.clone()).unwrap();
        assert!(missing["files"].as_array().unwrap().is_empty(), "{missing:?}");
        assert!(!missing["errors"].as_array().unwrap().is_empty(), "{missing:?}");
        params["historicalXcfSampleOrder"] = json!("native");
        assert!(s.execute(id, params.clone()).is_err());
        params["historicalXcfSampleOrder"] = json!("big-endian");
        let result = s.execute(id, params).unwrap();
        assert!(result["errors"].as_array().unwrap().is_empty(), "{result:?}");
        if name == "batch" {
            let notice = &result["notices"][0];
            assert!(notice["file"].as_str().unwrap().ends_with("big.xcf"));
            assert!(notice["warnings"].as_array().unwrap().iter().any(|warning| warning.as_str().unwrap().contains("explicit big-endian choice")));
        }
        let written = result["files"][0].as_str().unwrap();
        assert!(std::path::Path::new(written).is_file(), "{result:?}");
    }
}

#[test]
fn historical_sample_order_reaches_stack_statistics_contact_sheet_and_droplets() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("historical.xcf");
    std::fs::write(&path, fixture::historical_pixel(false)).unwrap();
    let mut s = session();
    let order = "big-endian";
    for command in ["file.scripts.loadFilesIntoStack", "file.scripts.statistics", "file.automate.contactSheetII"] {
        let params = json!({"input":[path],"historicalXcfSampleOrder":order,"mode":if command == "file.scripts.statistics" {"mean"} else {"rgb"},
            "units":"pixels","width":16,"height":16,"depth":16,"columns":1,"rows":1,"caption":false});
        let result = s.execute(command, params).unwrap();
        assert_eq!(result["layers"].as_u64().or_else(|| result["images"].as_u64()), Some(1), "{command}: {result}");
        let doc = s.active().unwrap().doc();
        let imported = doc.layers.last().unwrap().surface().unwrap();
        let bounds = imported.content_bounds();
        let actual = imported.pixel(bounds.x0, bounds.y0);
        assert!((actual[0] - f32::from(0x1234u16) / 65535.).abs() <= 1. / 65535., "{command}: {actual:?}");
    }
    let droplet = dir.path().join("historical.pcdroplet");
    let output = dir.path().join("result");
    s.execute(
        "file.automate.createDroplet",
        json!({"path":droplet,"format":"pcraft","shim":false,"overrideOpen":true,
            "steps":[["file.openAs",{"path":"recorded.xcf"}]],"historicalXcfSampleOrder":order}),
    )
    .unwrap();
    let result = s.execute("file.automate.runDroplet", json!({"droplet":droplet,"input":[path],"output":output})).unwrap();
    assert_eq!(result["errors"], json!([]), "{result}");
    assert!(result["notices"][0]["warnings"].as_array().unwrap().iter().any(|note| note.as_str().unwrap().contains("explicit big-endian choice")));
    let written = std::fs::read(result["files"][0].as_str().unwrap()).unwrap();
    let doc = photocraft_format::load_from_bytes(&written).unwrap();
    assert_eq!(doc.layers[0].surface().unwrap().pixel(0, 0)[0], f32::from(0x1234u16) / 65535.);
}

#[test]
fn batch_override_open_inherits_the_batch_order_and_keeps_a_recorded_choice() {
    let folder = tempfile::tempdir().unwrap();
    let source = folder.path().join("big.xcf");
    std::fs::write(&source, fixture::historical_pixel(false)).unwrap();
    let mut s = session();
    for (name, batch_order, recorded_order) in [
        ("inherits-missing", "big-endian", None),
        ("inherits-null", "big-endian", Some(Value::Null)),
        ("keeps-recorded", "little-endian", Some(json!("big-endian"))),
    ] {
        let mut open = json!({"path":"recorded.xcf"});
        if let Some(order) = recorded_order {
            open["historicalXcfSampleOrder"] = order;
        }
        let result = s
            .execute(
                "file.automate.batch",
                json!({"input":[source],"output":folder.path().join(name),"format":"png","overrideOpen":true,
                    "historicalXcfSampleOrder":batch_order,"steps":[["file.openAs",open]]}),
            )
            .unwrap();
        assert_eq!(result["errors"], json!([]), "{name}: {result}");
        let written = result["files"][0].as_str().unwrap();
        let png = std::fs::read(written).unwrap();
        let doc = photocraft_io::import(written, &png).unwrap().document;
        let pixel = doc.layers[0].surface().unwrap().pixel(0, 0);
        assert_eq!(pixel, [0x1234 as f32 / 65535., 0x4567 as f32 / 65535., 0x89ab as f32 / 65535., 1.], "{name}");
    }
}

#[test]
fn batch_edits_the_saved_xcf_layer_with_or_without_open_suppression() {
    fn words(bytes: &mut Vec<u8>, values: &[u32]) {
        bytes.extend(values.iter().flat_map(|value| value.to_be_bytes()));
    }
    fn point(bytes: &mut [u8], slot: usize, offset: usize) {
        bytes[slot..slot + 4].copy_from_slice(&(offset as u32).to_be_bytes());
    }
    let mut source = b"gimp xcf v007\0".to_vec();
    words(&mut source, &[1, 1, 0, 250, 17, 1]);
    source.push(0);
    words(&mut source, &[0, 0]);
    let pointers = source.len();
    words(&mut source, &[0, 0, 0, 0]); // Two layers, end layers, end channels.
    for (offset, name, red) in [(0, b"top\0".as_slice(), 0x0000u16), (4, b"saved\0".as_slice(), 0x1234u16)] {
        let layer = source.len();
        point(&mut source, pointers + offset, layer);
        words(&mut source, &[1, 1, 1, name.len() as u32]);
        source.extend(name);
        if offset == 4 {
            words(&mut source, &[2, 0]); // Selected source layer, below the default top layer.
        }
        words(&mut source, &[8, 4, 1, 0, 0]);
        let hierarchy = source.len() + 8;
        words(&mut source, &[hierarchy as u32, 0, 1, 1, 8, (hierarchy + 20) as u32, 0]);
        words(&mut source, &[1, 1, (hierarchy + 36) as u32, 0]);
        source.extend([red, 0, 0, 0xffff].into_iter().flat_map(u16::to_be_bytes));
    }
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("saved-layer.xcf");
    std::fs::write(&path, source).unwrap();
    let mut s = session();
    let mut baseline = None;
    for quiet in [false, true] {
        for override_open in [false, true] {
            let mut steps = vec![json!(["image.adjustments.invert", {}])];
            if override_open {
                steps.insert(0, json!(["file.openAs", {"path":"recorded.xcf"}]));
            }
            let result = s
                .execute(
                    "file.automate.batch",
                    json!({
                        "input":[path],"output":folder.path().join(format!("{quiet}-{override_open}")),"format":"pcraft",
                        "suppressWarnings":quiet,"suppressOpenOptions":quiet,"overrideOpen":override_open,"historicalXcfSampleOrder":"big-endian","steps":steps
                    }),
                )
                .unwrap();
            assert_eq!(result["errors"], json!([]), "{quiet} {override_open}: {result}");
            let file = std::fs::read(result["files"][0].as_str().unwrap()).unwrap();
            let doc = photocraft_format::load_from_bytes(&file).unwrap();
            let pixel = |name| doc.layers.iter().find(|layer| layer.name == name).unwrap().surface().unwrap().pixel(0, 0);
            assert_eq!(pixel("top"), [0., 0., 0., 1.], "{quiet} {override_open}");
            let saved = pixel("saved");
            assert_ne!(saved[0], f32::from(0x1234u16) / 65535., "{quiet} {override_open}");
            assert_eq!(&saved, baseline.get_or_insert_with(|| saved.clone()), "{quiet} {override_open}");
            assert!(
                result["notices"][0]["warnings"].as_array().unwrap().iter().any(|warning| warning.as_str().unwrap().contains("explicit big-endian choice"))
            );
        }
    }
}
