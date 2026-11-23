//! Explicit embedded producer only; ordinary compatibility and linked workflows stay unchanged.
use super::*;
use photocraft_doc::HistoricalXcfSampleOrder as Order;
use photocraft_geom::Rect;
use std::io::Write;

fn fixture(little: bool) -> Vec<u8> {
    let mut bytes = crate::jobs::historical_fixture::historical_rgba16();
    if little {
        let start = bytes.len() - 8;
        for word in bytes[start..].chunks_exact_mut(2) {
            word.swap(0, 1);
        }
    }
    bytes
}
fn session() -> Session {
    let mut session = crate::session();
    session.color_mut().settings.ask_on_missing = false;
    session.color_mut().settings.ask_on_mismatch = false;
    session
}
fn object(doc: &Document) -> &SmartObject {
    let LayerContent::Smart(object) = &doc.layers[0].content else { panic!("embedded Smart Object") };
    object
}
fn exact(doc: &Document, bytes: &[u8], order: Option<Order>) {
    let object = object(doc);
    assert_eq!(object.historical_xcf_sample_order, order);
    assert_eq!(object.historical_xcf_link_digest.is_some(), matches!(object.source, SmartSource::Linked { .. }) && order.is_some());
    let (_, retained) = crate::smart_cmds::source_bytes(&doc.metadata, &object.source).unwrap();
    assert_eq!(retained.as_slice(), bytes);
    crate::smart_cmds::purge_source_cache();
    crate::smart_cmds::purge_stack_cache();
    let pixels = crate::smart_cmds::render(doc, object).unwrap().unwrap().read_region(Rect::new(0, 0, 1, 1));
    let expected = [0x1234u16, 0x2345, 0x3456, 0xffff].map(|v| f32::from(v) / 65535.);
    for (actual, expected) in pixels.iter().zip(expected) {
        assert!((actual - expected).abs() <= 1. / 65535., "{actual} versus {expected}");
    }
}

#[test]
fn explicit_open_as_smart_object_keeps_order_through_job_cold_render_native_and_psd() {
    let _lock = crate::edit_menu_cmds::PURGE_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = tempfile::tempdir().unwrap();
    for (wire, order, little) in [("little-endian", Order::LittleEndian, true), ("big-endian", Order::BigEndian, false)] {
        let bytes = fixture(little);
        let path = directory.path().join(format!("{wire}.xcf"));
        std::fs::write(&path, &bytes).unwrap();
        for mode in 0..3 {
            let mut session = session();
            session.set_cooperative_jobs(mode == 2);
            let params = json!({"path":path,"historicalXcfSampleOrder":wire});
            let result = if mode == 0 {
                session.execute(CMD, params).unwrap()
            } else {
                match session.start(CMD, params).unwrap() {
                    crate::jobs::Started::Done(result) => result,
                    crate::jobs::Started::Job(id) => session.wait_job(id).unwrap(),
                }
            };
            assert!(result["docId"].is_u64(), "{result}");
            let doc = &session.active().unwrap().doc;
            exact(doc, &bytes, Some(order));
            let native = photocraft_format::save_to_bytes(doc, &Default::default()).unwrap();
            exact(&photocraft_format::load_from_bytes(&native).unwrap(), &bytes, Some(order));
            let psd = photocraft_io::export(doc, "roundtrip.psd", &Default::default()).unwrap();
            assert!(!psd.warnings.iter().any(|warning| warning.contains("written as pixels")), "{:?}", psd.warnings);
            exact(&photocraft_io::import("roundtrip.psd", &psd.bytes).unwrap().document, &bytes, Some(order));
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }
}

#[test]
fn prepared_wrapped_source_does_not_reopen_replaced_path_and_cancel_is_atomic() {
    let _lock = crate::edit_menu_cmds::PURGE_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("retained.xcf.gz");
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&fixture(true)).unwrap();
    let bytes = gzip.finish().unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let options = photocraft_io::ImportOptions {
        historical_xcf_order: photocraft_io::HistoricalXcfOrder::Explicit(photocraft_io::HistoricalXcfSampleOrder::LittleEndian),
        ..Default::default()
    };
    let prepared = prepare(path.to_str().unwrap(), "retained.xcf.gz", options, false, &crate::jobs::JobCtx::new()).unwrap();
    let replacement = fixture(false);
    std::fs::write(&path, &replacement).unwrap();
    let mut session = session();
    apply(&mut session, path.to_str().unwrap(), "retained.xcf.gz", prepared).unwrap();
    exact(&session.active().unwrap().doc, &bytes, Some(Order::LittleEndian));
    assert_eq!(std::fs::read(&path).unwrap(), replacement);
    let before = session.active().unwrap().doc.clone();
    let cancelled = crate::jobs::JobCtx::new();
    cancelled.cancel();
    assert!(matches!(prepare(path.to_str().unwrap(), "retained.xcf", options, false, &cancelled), Err(crate::EngineError::Cancelled)));
    assert!(Arc::ptr_eq(&session.active().unwrap().doc, &before));
}

#[test]
fn absent_historical_choice_and_png_keep_compatibility_and_invalid_choices_fail_before_io() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("compatible.xcf");
    let bytes = fixture(false);
    std::fs::write(&path, &bytes).unwrap();
    let mut compatible = session();
    compatible.execute(CMD, json!({"path":path})).unwrap();
    let compatible_object = object(&compatible.active().unwrap().doc);
    assert_eq!(compatible_object.historical_xcf_sample_order, None);
    assert_eq!(
        compatible_object.cache.as_ref().unwrap().read_region(Rect::new(0, 0, 1, 1)),
        [0x1234u16, 0x2345, 0x3456, 0xffff].map(|v| f32::from(v) / 65535.)
    );
    let image = photocraft_codecs::Image::from_u8(1, 1, photocraft_codecs::ChannelLayout::Rgba, vec![13, 47, 91, 255]).unwrap();
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let path = directory.path().join("ordinary.png");
    std::fs::write(&path, png).unwrap();
    for choice in [Value::Null, json!("little-endian"), json!("big-endian")] {
        let mut ordinary = session();
        ordinary.execute(CMD, json!({"path":path,"historicalXcfSampleOrder":choice})).unwrap();
        assert_eq!(object(&ordinary.active().unwrap().doc).historical_xcf_sample_order, None);
        assert_eq!(object(&ordinary.active().unwrap().doc).historical_xcf_link_digest, None);
    }
    for choice in [json!(true), json!(23), json!("native"), json!("LittleEndian")] {
        let mut rejected = session();
        assert!(matches!(
            rejected.execute(CMD, json!({"path":directory.path().join("missing.xcf"),"historicalXcfSampleOrder":choice})),
            Err(crate::EngineError::BadParams { .. })
        ));
        assert!(rejected.documents().is_empty());
    }
}
