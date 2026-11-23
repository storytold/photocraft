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
