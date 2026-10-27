use super::*;
use crate::jobs::JobCtx;
use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType};

fn scan(sample: SampleType, layout: ChannelLayout) -> Acquisition {
    let mut image = Image::new(3, 2, layout, sample).unwrap();
    if layout == ChannelLayout::Rgb {
        image.icc = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes().as_ref().clone());
    }
    image.meta.dpi = Some((150.0, 150.0));
    Acquisition {
        report: json!({"available": true, "devices": [{"id": "v500", "name": "EPSON V500"}], "acquired": true, "cancelled": false}),
        image: Some(Scan {
            bytes: photocraft_codecs::encode(&image, Format::Tiff, &EncodeOptions::default()).unwrap(),
            extension: "tif".into(),
            dpi: Some(300.0),
        }),
    }
}

#[test]
fn discovery_is_the_safe_default_and_params_are_validated_before_driver_calls() {
    assert!(request(&json!({})).unwrap().action == Action::List);
    assert!(request(&json!({"action": "acquire"})).unwrap().show_ui);
    let literal = "scanner's \"name\" $HOME $(exit) `exit` \\ ü";
    let r = request(&json!({"action": "acquire", "deviceId": literal, "showUI": false, "dpi": 300})).unwrap();
    assert_eq!(r.device_id.as_deref(), Some(literal));
    assert_eq!(serde_json::to_value(r).unwrap()["deviceId"], literal);
    for p in [
        Value::Null,
        json!([]),
        json!({"action": "unknown"}),
        json!({"action": "acquire", "showUI": false}),
        json!({"showUI": "false"}),
        json!({"deviceId": "  "}),
        json!({"deviceId": "a\u{0}b"}),
        json!({"deviceId": "x".repeat(4097)}),
        json!({"dpi": 0}),
        json!({"dpi": -1}),
        json!({"dpi": 300.5}),
        json!({"dpi": 12801}),
        json!({"dpi": u64::MAX}),
        json!({"mode": "rgb"}),
    ] {
        let mut s = Session::new();
        assert!(matches!(s.execute(COMMAND, p), Err(EngineError::BadParams { .. })));
        assert!(s.documents().is_empty());
    }
}

#[test]
fn scanner_import_preserves_depth_model_profile_and_uses_actual_dpi() {
    for sample in [SampleType::U8, SampleType::U16] {
        for layout in [ChannelLayout::Rgb, ChannelLayout::Gray] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 8, "height": 9})).unwrap();
            let previous = s.active().unwrap().doc.id;
            let result = run_with(&mut s, request(&json!({"action": "acquire"})).unwrap(), move |_, _| Ok(scan(sample, layout))).unwrap();
            assert_eq!(result["document"], 1);
            assert_eq!(result["docId"], s.active().unwrap().doc.id.0);
            assert_eq!(result["acquired"], true);
            assert_eq!(s.documents()[0].doc.id, previous);
            assert_eq!(s.documents().len(), 2);
            let st = s.active().unwrap();
            assert!(st.path.is_none(), "temporary scan files must never become save destinations");
            assert!(st.is_dirty(), "closing a scan must offer to save it");
            assert_eq!(st.doc.name, "Scan");
            assert_eq!((st.doc.size.width, st.doc.size.height), (3, 2));
            assert_eq!(st.doc.depth.bits(), if sample == SampleType::U16 { 16 } else { 8 });
            assert_eq!(st.doc.mode, if layout == ChannelLayout::Gray { photocraft_color::ColorMode::Grayscale } else { photocraft_color::ColorMode::Rgb });
            assert_eq!(st.doc.resolution_dpi, 300.0);
            if layout == ChannelLayout::Rgb {
                assert_eq!(result["color"]["action"], "kept");
                assert!(st.doc.icc_profile.is_some());
            }
        }
    }
}

#[test]
fn scanner_profile_question_holds_unmodified_pixels_until_the_answer() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 2, "height": 2})).unwrap();
    let previous = s.active().unwrap().doc.id;
    s.execute("edit.colorSettings", json!({"askOnMissing": true})).unwrap();
    let prepared = prepare(scan(SampleType::U16, ChannelLayout::Gray), &JobCtx::new()).unwrap();
    let result = apply(&mut s, prepared).unwrap();
    assert_eq!(result["acquired"], true);
    assert_eq!(result["color"]["missing"], true);
    assert!(result.get("document").is_none());
    assert_eq!(s.documents().len(), 1);
    assert_eq!(s.active().unwrap().doc.id, previous);
    let (scan, read_only) = s.take_held_open(result["held"].as_u64().unwrap()).unwrap();
    assert!(!read_only);
    assert!(scan.icc_profile.is_none());
    assert_eq!(scan.resolution_dpi, 300.0);
    assert_eq!(scan.depth.bits(), 16);
}

#[test]
fn cancellation_discovery_and_driver_errors_leave_the_document_unchanged() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 9})).unwrap();
    let before = s.active().unwrap().doc.id;
    for cancelled in [false, true] {
        let result = run_with(&mut s, request(&json!({})).unwrap(), move |_, _| {
            Ok(Acquisition { report: json!({"available": true, "devices": [], "acquired": false, "cancelled": cancelled}), image: None })
        })
        .unwrap();
        assert_eq!(result["cancelled"], cancelled);
        assert!(result.get("document").is_none());
    }
    assert!(run_with(&mut s, request(&json!({})).unwrap(), |_, _| Err(EngineError::Other("scanner busy".into()))).is_err());
    let mut corrupt = scan(SampleType::U8, ChannelLayout::Rgb);
    corrupt.image.as_mut().unwrap().bytes = vec![0, 1, 2];
    assert!(run_with(&mut s, request(&json!({})).unwrap(), move |_, _| Ok(corrupt)).is_err());
    assert_eq!(s.documents().len(), 1);
    assert_eq!(s.active().unwrap().doc.id, before);
}

#[test]
fn cancelled_work_is_never_applied() {
    let mut s = Session::new();
    let ctx = JobCtx::new();
    ctx.cancel();
    s.set_inline_job_ctx(Some(ctx));
    assert!(matches!(
        run_with(&mut s, request(&json!({"action": "acquire"})).unwrap(), |_, _| Ok(scan(SampleType::U16, ChannelLayout::Rgb))),
        Err(EngineError::Cancelled)
    ));
    assert!(s.documents().is_empty());
}

#[test]
fn invalid_driver_dpi_does_not_replace_image_metadata() {
    for dpi in [None, Some(0.0), Some(f32::NAN), Some(f32::INFINITY), Some(12801.0)] {
        let mut acquired = scan(SampleType::U8, ChannelLayout::Rgb);
        acquired.image.as_mut().unwrap().dpi = dpi;
        let prepared = prepare(acquired, &JobCtx::new()).unwrap();
        assert_eq!(prepared.imported.unwrap().document.resolution_dpi, 150.0);
    }
}

#[cfg(target_os = "windows")]
#[test]
#[ignore = "requires the Windows WIA service, but never scans or opens dialogs"]
fn windows_device_discovery_smoke() {
    let mut s = Session::new();
    let result = s.execute(COMMAND, json!({})).unwrap();
    assert_eq!(result["available"], true);
    assert!(result["devices"].is_array());
    assert_eq!(result["acquired"], false);
    assert!(s.documents().is_empty());
    // An impossible explicit ID verifies the bridge error path without connecting hardware.
    assert!(s.execute(COMMAND, json!({"action": "acquire", "deviceId": "photocraft-test-nonexistent-device", "showUI": false})).is_err());
    assert!(s.documents().is_empty());
}

#[cfg(not(target_os = "windows"))]
#[test]
fn other_platforms_report_unavailability_and_reject_acquisition() {
    let mut s = Session::new();
    let result = s.execute(COMMAND, json!({})).unwrap();
    assert_eq!(result["available"], false);
    assert_eq!(result["devices"], json!([]));
    assert!(s.execute(COMMAND, json!({"action": "acquire"})).is_err());
    assert!(s.documents().is_empty());
}
