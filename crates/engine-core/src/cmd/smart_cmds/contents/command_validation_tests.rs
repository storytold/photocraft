use super::*;
use photocraft_doc::SmartObject;

fn object(session: &Session, id: LayerId) -> &SmartObject {
    smart(session.active().unwrap().doc(), id).unwrap()
}

fn assert_refused_without_changes(session: &mut Session, command: &str, params: Value, output: &std::path::Path) {
    let before = session.active().unwrap().doc().clone();
    let revision = session.active().unwrap().revision;
    let undo = session.active().unwrap().history.past_len();
    let journal = session.journal.clone();
    let error = session.execute(command, params).unwrap_err();
    assert!(
        matches!(error, EngineError::BadParams { ref cmd, ref msg }
            if cmd == command && msg.starts_with("unknown parameter `historicalXcfSampleOrder`")),
        "{command}: {error}"
    );
    assert!(Arc::ptr_eq(&before, session.active().unwrap().doc()));
    assert_eq!(session.active().unwrap().revision, revision);
    assert_eq!(session.active().unwrap().history.past_len(), undo);
    assert_eq!(session.journal, journal);
    assert!(!output.exists(), "a refused conversion must not write its destination");
}

#[test]
fn storage_conversions_reject_unrelated_order_before_changing_contents_or_writing_files() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.png");
    let output = directory.path().join("converted.png");
    let image = photocraft_doc::Document::with_background(
        "Original",
        photocraft_geom::Size::new(1, 1),
        photocraft_color::ColorMode::Rgb,
        photocraft_color::SampleType::U8,
        photocraft_color::Color::rgb(0.2, 0.6, 0.9),
    );
    let bytes = photocraft_io::export(&image, "png", &Default::default()).unwrap().bytes;
    std::fs::write(&source, &bytes).unwrap();
    let mut session = crate::session();
    session.execute("file.new", json!({"width":4,"height":4})).unwrap();
    let reply = session.execute("file.placeLinked", json!({"path":source,"fit":false,"scale":100})).unwrap();
    let id = LayerId(reply["layer"].as_u64().unwrap());
    assert!(matches!(&object(&session, id).source, SmartSource::Linked { path } if path == source.to_str().unwrap()));

    // Even a well-spelled order belongs to source decoding, not storage-only
    // conversions. Null and mistyped values must not silently bypass the schema.
    for order in [json!("little-endian"), json!("big-endian"), Value::Null, json!(false)] {
        assert_refused_without_changes(&mut session, "layer.smartObjects.convertToEmbedded", json!({"layer":id.0,"historicalXcfSampleOrder":order}), &output);
    }
    session.execute("layer.smartObjects.convertToEmbedded", json!({"layer":id.0})).unwrap();
    let SmartSource::Embedded { file_name, bytes: embedded } = &object(&session, id).source else { panic!("expected an embedded source") };
    assert_eq!(file_name, "source.png");
    assert_eq!(embedded.as_slice(), bytes);

    for order in [json!("little-endian"), json!("big-endian"), Value::Null, json!(false)] {
        assert_refused_without_changes(
            &mut session,
            "layer.smartObjects.convertToLinked",
            json!({"layer":id.0,"path":output,"historicalXcfSampleOrder":order}),
            &output,
        );
    }
    session.execute("layer.smartObjects.convertToLinked", json!({"layer":id.0,"path":output})).unwrap();
    assert!(matches!(&object(&session, id).source, SmartSource::Linked { path } if path == output.to_str().unwrap()));
    assert_eq!(std::fs::read(&output).unwrap(), bytes);
    assert_eq!(std::fs::read(&source).unwrap(), bytes);
}

fn interpreted_link(directory: &std::path::Path) -> (Session, LayerId, std::path::PathBuf, Vec<u8>) {
    use photocraft_doc::{ColorMode, Document, HistoricalXcfSampleOrder, Layer, SampleType};
    let bytes = crate::jobs::historical_fixture::historical_rgba16();
    let path = directory.join("historical.xcf");
    std::fs::write(&path, &bytes).unwrap();
    let mut object = SmartObject::new(SmartSource::Linked { path: path.to_str().unwrap().into() }, photocraft_geom::Affine::IDENTITY, None);
    object.historical_xcf_sample_order = Some(HistoricalXcfSampleOrder::LittleEndian);
    object.historical_xcf_link_digest = Some(*blake3::hash(&bytes).as_bytes());
    let layer = Layer::new("Interpreted linked contents", LayerContent::Smart(object));
    let id = layer.id;
    let mut doc = Document::new("Loaded native linked contents", photocraft_geom::Size::new(3, 1), ColorMode::Rgb, SampleType::U16);
    doc.layers.push(layer);
    let mut session = crate::session();
    session.add_document(doc, None);
    (session, id, path, bytes)
}

#[test]
fn changed_historical_links_refuse_contents_and_storage_commands_atomically() {
    let directory = tempfile::tempdir().unwrap();
    let (mut session, id, path, approved) = interpreted_link(directory.path());
    let mut changed = approved.clone();
    let last = changed.len() - 1;
    changed[last] ^= 1;
    std::fs::write(&path, &changed).unwrap();
    let output = directory.path().join("exported.xcf");
    let before = session.active().unwrap().clone();
    let journal = session.journal.clone();
    for command in ["editContents", "convertToLayers", "exportContents", "convertToEmbedded", "embedAllLinked", "newSmartObjectViaCopy"] {
        let params = match command {
            "exportContents" => json!({"layer":id.0,"path":output}),
            "embedAllLinked" => json!({}),
            _ => json!({"layer":id.0}),
        };
        let error = session.execute(&format!("layer.smartObjects.{command}"), params).unwrap_err();
        assert!(matches!(error, crate::EngineError::HistoricalXcfLinkChanged(_)), "{command}: {error}");
        assert!(Arc::ptr_eq(session.active().unwrap().doc(), before.doc()));
        assert_eq!((session.active().unwrap().revision, session.active().unwrap().history.past_len()), (before.revision, before.history.past_len()));
        assert_eq!(session.documents().len(), 1);
        assert_eq!(session.journal, journal);
        assert!(!output.exists());
        assert_eq!(std::fs::read(&path).unwrap(), changed);
    }
    // Convert to Linked also authenticates the old revision before its first write.
    let error = session.execute("layer.smartObjects.convertToLinked", json!({"layer":id.0,"path":output})).unwrap_err();
    assert!(matches!(error, crate::EngineError::HistoricalXcfLinkChanged(_)), "{error}");
    assert!(!output.exists());
    assert!(Arc::ptr_eq(session.active().unwrap().doc(), before.doc()));
    std::fs::write(&path, &approved).unwrap();
    session.execute("layer.smartObjects.exportContents", json!({"layer":id.0,"path":output})).unwrap();
    assert_eq!(std::fs::read(&output).unwrap(), approved, "unchanged approved bytes remain exportable verbatim");
    session.execute("layer.smartObjects.convertToEmbedded", json!({"layer":id.0})).unwrap();
    let object = object(&session, id);
    assert!(object.historical_xcf_sample_order.is_some());
    assert!(object.historical_xcf_link_digest.is_none());
    assert!(matches!(&object.source,SmartSource::Embedded {bytes,..} if bytes.as_slice()==approved));
}

#[test]
fn neighboring_source_commands_reject_invalid_order_before_missing_path_io() {
    let directory = tempfile::tempdir().unwrap();
    let (mut session, id, _, _) = interpreted_link(directory.path());
    let before = session.active().unwrap().clone();
    for command in ["layer.smartObjects.replaceContents", "layer.smartObjects.relinkToFile"] {
        let error =
            session.execute(command, json!({"layer":id.0,"path":directory.path().join("missing.xcf"),"historicalXcfSampleOrder":"invalid"})).unwrap_err();
        assert!(matches!(error,EngineError::BadParams {ref cmd,..} if cmd==command), "{error}");
        assert!(Arc::ptr_eq(session.active().unwrap().doc(), before.doc()));
        assert_eq!((session.active().unwrap().revision, session.active().unwrap().history.past_len()), (before.revision, before.history.past_len()));
    }
}
