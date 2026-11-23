use super::*;
use crate::color_cmds::ColorExt;
use std::sync::Arc;
fn bytes(little: bool) -> Vec<u8> {
    let mut b = crate::jobs::historical_fixture::historical_rgba16();
    if little {
        let at = b.len() - 8;
        for w in b[at..].chunks_exact_mut(2) {
            w.swap(0, 1)
        }
    }
    b
}
fn session(path: &std::path::Path) -> Session {
    std::fs::write(path, bytes(true)).unwrap();
    let mut s = crate::session();
    s.color_mut().settings.ask_on_missing = false;
    s.color_mut().settings.ask_on_mismatch = false;
    s.execute("file.openAsSmartObject", json!({"path":path,"historicalXcfSampleOrder":"little-endian"})).unwrap();
    s
}
fn id(s: &Session) -> LayerId {
    s.active().unwrap().doc.layers[0].id
}
fn object(s: &Session, id: LayerId) -> &photocraft_doc::SmartObject {
    smart(&s.active().unwrap().doc, id).unwrap()
}
fn relink(s: &mut Session, path: &std::path::Path, choice: Option<&str>) -> Result<Value> {
    let mut p = json!({"path":path,"layer":id(s).0});
    if let Some(choice) = choice {
        p["historicalXcfSampleOrder"] = json!(choice)
    }
    s.execute(RELINK, p)
}

#[test]
fn same_bytes_inherit_order_different_bytes_require_new_choice_and_modern_sources_clear_it() {
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first.xcf");
    let mut s = session(&first);
    let layer = id(&s);
    relink(&mut s, &first, None).unwrap();
    assert_eq!(object(&s, layer).historical_xcf_sample_order, Some(Order::LittleEndian));
    let different = directory.path().join("different.xcf");
    std::fs::write(&different, bytes(false)).unwrap();
    let before = s.active().unwrap().doc.clone();
    assert!(relink(&mut s, &different, None).unwrap_err().to_string().contains("sample byte order"));
    assert!(Arc::ptr_eq(&s.active().unwrap().doc, &before));
    relink(&mut s, &different, Some("big-endian")).unwrap();
    assert_eq!(object(&s, layer).historical_xcf_sample_order, Some(Order::BigEndian));
    let image = photocraft_codecs::Image::from_u8(1, 1, photocraft_codecs::ChannelLayout::Rgba, vec![17, 39, 71, 255]).unwrap();
    let png = directory.path().join("source.png");
    std::fs::write(&png, photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap()).unwrap();
    relink(&mut s, &png, Some("little-endian")).unwrap();
    assert_eq!(object(&s, layer).historical_xcf_sample_order, None);
    assert_eq!(object(&s, layer).historical_xcf_link_digest, None);
}
#[test]
fn changed_links_refuse_storage_and_decode_commands_without_document_or_file_mutation() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.xcf");
    let mut s = session(&source);
    relink(&mut s, &source, None).unwrap();
    let layer = id(&s);
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.past_len();
    std::fs::write(&source, bytes(false)).unwrap();
    for command in [
        "layer.smartObjects.updateModifiedContent",
        "layer.smartObjects.updateAllModifiedContent",
        "layer.smartObjects.convertToEmbedded",
        "layer.smartObjects.embedAllLinked",
        "layer.smartObjects.editContents",
        "layer.smartObjects.convertToLayers",
        "layer.smartObjects.newSmartObjectViaCopy",
        "layer.smartObjects.exportContents",
        "layer.smartObjects.convertToLinked",
    ] {
        let output = directory.path().join(format!("{command}.pcraft"));
        let error = s.execute(command, json!({"layer":layer.0,"path":output})).unwrap_err();
        assert!(matches!(error, EngineError::HistoricalXcfLinkChanged(_)), "{command}: {error}");
        assert!(Arc::ptr_eq(&s.active().unwrap().doc, &before), "{command}");
        assert_eq!(s.active().unwrap().history.past_len(), history);
        assert!(!output.exists());
    }
    assert_eq!(std::fs::read(source).unwrap(), bytes(false));
}
#[test]
fn relink_reads_one_verified_snapshot_for_every_shared_instance_and_cancellation_publishes_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.xcf");
    let mut s = session(&path);
    let first = id(&s);
    s.execute("layer.duplicate", json!({"layer":first.0})).unwrap();
    let source = s.active().unwrap().doc.clone();
    let ctx = JobCtx::new();
    let expected = photocraft_io::linked::historical_xcf_digest(&bytes(true));
    let change = read_change(&source, first, path.to_string_lossy().into_owned(), None, Some(expected), &ctx).unwrap();
    std::fs::write(&path, bytes(false)).unwrap();
    let prepared = prepare_change(&source, first, &change, &ctx).unwrap();
    for (_, _, layer) in prepared.document.walk() {
        if let LayerContent::Smart(sm) = &layer.content {
            assert_eq!(sm.historical_xcf_link_digest, Some(expected));
            let actual = sm.cache.as_ref().unwrap().read_region(photocraft_geom::Rect::new(0, 0, 1, 1));
            assert!((actual[0] - f32::from(0x1234u16) / 65535.).abs() <= 1. / 65535.);
        }
    }
    assert!(matches!(
        read_change(&source, first, path.to_string_lossy().into_owned(), Some(Some(Order::BigEndian)), Some(expected), &ctx),
        Err(EngineError::SourceChanged { .. })
    ));
    let before = s.active().unwrap().doc.clone();
    let cancelled = JobCtx::new();
    cancelled.cancel();
    assert!(matches!(prepare_change(&source, first, &change, &cancelled), Err(EngineError::Cancelled)));
    assert!(Arc::ptr_eq(&s.active().unwrap().doc, &before));
    // Even the old warm cache cannot decode the current path under the captured old order.
    publish(&mut s, first, "Relink to File", prepared).unwrap();
    assert!(matches!(crate::smart_cmds::render(&s.active().unwrap().doc, object(&s, first)), Err(EngineError::HistoricalXcfLinkChanged(_))));
}
#[test]
fn storage_conversion_preserves_order_and_exact_encoded_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.xcf");
    let mut s = session(&source);
    let layer = id(&s);
    let target = directory.path().join("linked.xcf");
    s.execute("layer.smartObjects.convertToLinked", json!({"layer":layer.0,"path":target})).unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), bytes(true));
    assert!(object(&s, layer).historical_xcf_link_digest.is_some());
    s.execute("layer.smartObjects.convertToEmbedded", json!({"layer":layer.0})).unwrap();
    assert_eq!(object(&s, layer).historical_xcf_sample_order, Some(Order::LittleEndian));
    assert_eq!(object(&s, layer).historical_xcf_link_digest, None);
    assert_eq!(contents_of(&s.active().unwrap().doc, layer).unwrap().1.as_slice(), bytes(true));
}
#[test]
fn malformed_relink_controls_fail_before_missing_path_io() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.xcf");
    let mut s = session(&source);
    let before = s.active().unwrap().doc.clone();
    for params in [
        json!({"path":directory.path().join("absent.xcf"),"historicalXcfSampleOrder":"native"}),
        json!({"path":directory.path().join("absent.xcf"),"expectedSourceDigest":[1,2,3]}),
    ] {
        assert!(matches!(s.execute(RELINK, params), Err(EngineError::BadParams { .. })));
    }
    assert!(Arc::ptr_eq(&s.active().unwrap().doc, &before));
}
