use super::*;
use crate::smart_cmds::encode_source;
use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::{Layer, LayerContent, SmartObject};
use photocraft_geom::{Affine, Size};

fn png() -> Vec<u8> {
    let document = Document::with_background("Replacement", Size::new(2, 2), ColorMode::Rgb, SampleType::U8, Color::rgb(0., 1., 0.));
    photocraft_io::export(&document, "png", &Default::default()).unwrap().bytes
}

fn session() -> (Session, LayerId, LayerId) {
    let inner = Document::with_background("Original", Size::new(2, 2), ColorMode::Rgb, SampleType::U8, Color::rgb(1., 0., 0.));
    let object = SmartObject::new(
        SmartSource::Embedded { file_name: "Original.pcraft".into(), bytes: Arc::new(encode_source(&inner).unwrap()) },
        Affine::IDENTITY,
        Some(inner.layers[0].surface().unwrap().clone()),
    );
    let first = Layer::new("First instance", LayerContent::Smart(object.clone()));
    let first_id = first.id;
    let mut second = Layer::new("Second instance", LayerContent::Smart(object));
    let second_id = second.id;
    if let LayerContent::Smart(object) = &mut second.content {
        object.transform = Affine::translate(4., 3.);
        object.smart_filters.push(photocraft_doc::SmartFilter {
            command: "image.adjustments.invert".into(),
            params: json!({}),
            blend: photocraft_color::BlendMode::Normal,
            opacity: 1.,
            visible: true,
        });
    }
    let mut document = Document::new("Parent", Size::new(8, 8), ColorMode::Rgb, SampleType::U8);
    document.layers.extend([first, second]);
    let mut session = crate::session();
    session.add_document(document, None);
    (session, first_id, second_id)
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn picker_worker_prepares_shared_instances_before_one_captured_document_commit() {
    let (mut session, first, second) = session();
    let before = session.active().unwrap().doc().clone();
    let contents = smart(&before, first).unwrap().contents_id;
    let undo = session.active().unwrap().history.past_len();
    let bytes = png();
    // Request the second instance: the result must not accidentally name the first.
    let Started::Job(job) = start(&mut session, second, "Picked.png", bytes.clone()).unwrap() else {
        panic!("native picker replacement must register a real worker")
    };
    assert!(Arc::ptr_eq(&before, session.active().unwrap().doc()));
    let other = session.add_document(Document::new("Other", Size::new(1, 1), ColorMode::Rgb, SampleType::U8), None);
    let other_id = session.active().unwrap().doc.id;
    let result = session.wait_job(job).unwrap();
    assert_eq!(session.active().unwrap().doc.id, other_id);
    assert_eq!(result["layer"], second.0);
    assert_eq!(result["instances"], json!([first.0, second.0]));
    let parent = session.documents().iter().find(|state| state.doc.id == before.id).unwrap();
    assert_eq!(parent.history.past_len(), undo + 1);
    for id in [first, second] {
        let object = smart(parent.doc(), id).unwrap();
        assert_eq!(object.contents_id, contents);
        assert_eq!(object.transform, smart(&before, id).unwrap().transform);
        assert_eq!(object.smart_filters, smart(&before, id).unwrap().smart_filters);
        let SmartSource::Embedded { file_name, bytes: installed } = &object.source else { panic!("embedded replacement") };
        assert_eq!(file_name, "Picked.png");
        assert_eq!(installed.as_slice(), bytes);
    }
    let first_pixel = smart(parent.doc(), first).unwrap().cache.as_ref().unwrap().rgba(0, 0);
    let second_pixel = smart(parent.doc(), second).unwrap().cache.as_ref().unwrap().rgba(4, 3);
    assert!(first_pixel[1] > first_pixel[0]);
    assert!(second_pixel[0] > second_pixel[1], "each instance keeps its own filter");
    assert!(session.set_active(0));
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(session.active().unwrap().doc().layers, before.layers);
    assert!(session.set_active(other));
}

#[test]
fn cancelled_preparation_never_retargets_or_changes_any_instance() {
    let (session, first, _) = session();
    let before = session.active().unwrap().doc().clone();
    let ctx = JobCtx::new();
    ctx.cancel();
    let result = prepare(before.clone(), first, Source::Bytes("Invalid.svgz".into(), Arc::new(vec![0x1f, 0x8b])), &ctx);
    assert!(matches!(result, Err(EngineError::Cancelled)));
    assert!(Arc::ptr_eq(&before, session.active().unwrap().doc()));
}

#[test]
fn picker_replacement_checks_authority_before_registering_work() {
    let (mut session, first, _) = session();
    let before = session.active().unwrap().doc().clone();
    session.authorize = Some(|command, params| {
        assert_eq!(command, COMMAND);
        assert_eq!(params["path"], "Denied.png");
        Err(other("replacement denied"))
    });
    assert!(start(&mut session, first, "Denied.png", png()).unwrap_err().to_string().contains("replacement denied"));
    assert!(Arc::ptr_eq(&before, session.active().unwrap().doc()));
}

/// Depends on the independent strict source baseline (#16550); ordinary Open
/// may keep usable artwork with a warning, warningless replacement may not.
#[test]
fn replacement_worker_rejects_incomplete_svgz_without_changing_any_shared_source() {
    fn gzip(bytes: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="2" height="2" fill="green"/><image width="1" height="1" href="data:image/png;base64,AA=="/></svg>"#;
    let bytes = gzip(svg);
    let complete = gzip(br##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="2" height="2" fill="#00ff00"/></svg>"##);
    for name in ["Replacement.svgz", "Misnamed.bin"] {
        let (mut session, first, _) = session();
        let before = session.active().unwrap().doc().clone();
        let undo = session.active().unwrap().history.past_len();
        let result = match start(&mut session, first, name, bytes.clone()) {
            Ok(Started::Job(job)) => session.wait_job(job),
            Ok(Started::Done(value)) => Ok(value),
            Err(error) => Err(error),
        };
        assert!(result.is_err(), "{name}: partial SVG must not be installed");
        assert!(Arc::ptr_eq(&before, session.active().unwrap().doc()));
        assert_eq!(session.active().unwrap().history.past_len(), undo);
        let result = match start(&mut session, first, name, complete.clone()).unwrap() {
            Started::Job(job) => session.wait_job(job).unwrap(),
            Started::Done(value) => value,
        };
        assert_eq!(result["layer"], first.0);
        let installed = smart(session.active().unwrap().doc(), first).unwrap();
        let SmartSource::Embedded { bytes, .. } = &installed.source else { panic!("valid SVGZ source retained") };
        assert_eq!(bytes.as_slice(), complete);
        let pixel = installed.cache.as_ref().unwrap().rgba(0, 0);
        assert!(pixel[1] > pixel[0] && pixel[1] > pixel[2] && pixel[3] > 0.);
    }
}

#[test]
fn synchronous_replacement_preserves_legacy_result_and_exact_source_bytes() {
    let (mut session, first, second) = session();
    let bytes = png();
    let result = super::super::replace_contents_bytes(&mut session, second, "Sync.png", bytes.clone()).unwrap();
    assert_eq!(result["layer"], second.0);
    assert_eq!(result["instances"], json!([first.0, second.0]));
    assert!(result.get("pending").is_none());
    for id in [first, second] {
        let SmartSource::Embedded { bytes: installed, .. } = &smart(session.active().unwrap().doc(), id).unwrap().source else { panic!("embedded source") };
        assert_eq!(installed.as_slice(), bytes);
    }
}

fn historical_linked_session(path: &std::path::Path, order: photocraft_doc::HistoricalXcfSampleOrder) -> (Session, LayerId, LayerId, Vec<u8>) {
    let mut bytes = crate::jobs::historical_fixture::historical_rgba16();
    if order == photocraft_doc::HistoricalXcfSampleOrder::LittleEndian {
        let start = bytes.len() - 8;
        for word in bytes[start..].as_chunks_mut::<2>().0 {
            word.swap(0, 1);
        }
    }
    std::fs::write(path, &bytes).unwrap();
    let source = crate::smart_cmds::decode_source_with_order("Original.xcf", &bytes, Some(order)).unwrap();
    let mut object =
        SmartObject::new(SmartSource::Linked { path: path.to_str().unwrap().into() }, Affine::IDENTITY, Some(source.layers[0].surface().unwrap().clone()));
    object.historical_xcf_sample_order = Some(order);
    object.historical_xcf_link_digest = Some(*blake3::hash(&bytes).as_bytes());
    let first = Layer::new("First historical instance", LayerContent::Smart(object.clone()));
    let first_id = first.id;
    object.transform = Affine::translate(4., 3.);
    let second = Layer::new("Second historical instance", LayerContent::Smart(object));
    let second_id = second.id;
    let mut document = Document::new("Historical parent", Size::new(8, 8), ColorMode::Rgb, SampleType::U16);
    document.layers.extend([first, second]);
    let mut session = crate::session();
    session.add_document(document, None);
    (session, first_id, second_id, bytes)
}

#[test]
fn fresh_worker_replacement_clears_historical_interpretation_only_after_success_and_undo_restores_it() {
    let directory = tempfile::tempdir().unwrap();
    for order in [photocraft_doc::HistoricalXcfSampleOrder::LittleEndian, photocraft_doc::HistoricalXcfSampleOrder::BigEndian] {
        let path = directory.path().join(format!("{order}.xcf"));
        let (mut session, first, second, source) = historical_linked_session(&path, order);
        let before = session.active().unwrap().doc().clone();
        // The old source really decodes under its stored interpretation, rather
        // than testing decorative metadata on an unrelated PNG object.
        let cold = crate::smart_cmds::render(&before, smart(&before, first).unwrap()).unwrap().unwrap().rgba(0, 0);
        assert!((cold[0] - f32::from(0x1234u16) / 65535.).abs() <= 1. / 65535.);
        let refused = match start(&mut session, second, "Bad.png", vec![0, 1, 2]) {
            Ok(Started::Job(job)) => session.wait_job(job),
            Ok(Started::Done(result)) => Ok(result),
            Err(error) => Err(error),
        };
        assert!(refused.is_err());
        assert!(Arc::ptr_eq(&before, session.active().unwrap().doc()));
        let replacement = png();
        match start(&mut session, second, "Replacement.png", replacement.clone()).unwrap() {
            Started::Job(job) => {
                session.wait_job(job).unwrap();
            }
            Started::Done(_) => {}
        }
        for id in [first, second] {
            let object = smart(session.active().unwrap().doc(), id).unwrap();
            assert_eq!(object.historical_xcf_sample_order, None);
            assert_eq!(object.historical_xcf_link_digest, None);
            let SmartSource::Embedded { bytes, .. } = &object.source else { panic!("replacement embedded") };
            assert_eq!(bytes.as_slice(), replacement);
            assert_eq!(object.transform, smart(&before, id).unwrap().transform);
        }
        assert_eq!(std::fs::read(&path).unwrap(), source);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc().layers, before.layers);
    }
}
