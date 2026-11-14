#![allow(clippy::unwrap_used, clippy::panic)]

use super::*;
use crate::jobs::{Started, historical_xcf_fixture as fixture};
use photocraft_doc::{HistoricalXcfSampleOrder, SmartObject};

fn session() -> Session {
    let mut s = Session::new();
    s.color.settings.ask_on_missing = false;
    s.color.settings.ask_on_mismatch = false;
    s.execute("file.new", json!({"width":1,"height":1,"depth":16})).unwrap();
    s
}

fn object(s: &Session, id: LayerId) -> &SmartObject {
    smart(&s.active().unwrap().doc, id).unwrap()
}

fn roundtrip(s: &Session) -> Session {
    let bytes = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap();
    let doc = photocraft_format::load_from_bytes(&bytes).unwrap();
    let mut reopened = Session::new();
    reopened.add_document(doc, None);
    reopened
}

fn assert_child_pixels(s: &Session) {
    let doc = &s.active().unwrap().doc;
    let LayerContent::Raster(surface) = &doc.layers[0].content else { panic!("decoded source pixel layer") };
    let mut pixel = [0.; 4];
    surface.read_pixel(0, 0, &mut pixel);
    assert_eq!(pixel, [0x1234 as f32 / 65535., 0x4567 as f32 / 65535., 0x89ab as f32 / 65535., 1.]);
}

#[test]
fn historical_smart_placements_native_reopen_edit_and_save_contents_keep_source_identity() {
    let dir = tempfile::tempdir().unwrap();
    for (little, order, interpretation) in
        [(true, "little-endian", HistoricalXcfSampleOrder::LittleEndian), (false, "big-endian", HistoricalXcfSampleOrder::BigEndian)]
    {
        let bytes = fixture::historical_pixel(little);
        let path = dir.path().join(format!("source-{order}.xcf"));
        std::fs::write(&path, &bytes).unwrap();
        for command in ["file.placeEmbedded", "file.placeLinked", "file.openAsSmartObject"] {
            let mut s = session();
            let reply = s.execute(command, json!({"path":path,"historicalXcfSampleOrder":order})).unwrap();
            let id = LayerId(reply["layer"].as_u64().unwrap());
            let contents_id = object(&s, id).contents_id;
            assert_eq!(object(&s, id).historical_xcf_sample_order, Some(interpretation), "{command}");
            let mut reopened = roundtrip(&s);
            assert_eq!(object(&reopened, id).contents_id, contents_id);
            assert_eq!(object(&reopened, id).historical_xcf_sample_order, Some(interpretation));
            assert_eq!(contents_of(&reopened.active().unwrap().doc, id).unwrap().1.as_slice(), bytes);
            let parent_id = reopened.active().unwrap().doc.id;
            reopened.execute("layer.smartObjects.editContents", json!({"layer":id.0})).unwrap();
            assert_ne!(reopened.active().unwrap().doc.id, parent_id);
            assert!(reopened.active().unwrap().path.is_none());
            assert_child_pixels(&reopened);
            reopened.execute("layer.renameLayer", json!({"name":"Edited source"})).unwrap();
            reopened.execute("layer.smartObjects.saveContents", json!({})).unwrap();
            assert!(!reopened.active().unwrap().is_dirty());
            assert!(reopened.set_active(0));
            let sm = object(&reopened, id);
            assert_eq!(sm.contents_id, contents_id);
            assert_eq!(sm.historical_xcf_sample_order, None, "native replacement no longer contains historical samples");
            let SmartSource::Embedded { file_name, bytes: native } = &sm.source else { panic!("edited contents are embedded") };
            assert!(file_name.ends_with(".pcraft"));
            assert_ne!(native.as_slice(), bytes);
            let mut saved_again = roundtrip(&reopened);
            saved_again.execute("layer.smartObjects.editContents", json!({"layer":id.0})).unwrap();
            assert_child_pixels(&saved_again);
            assert_eq!(saved_again.active().unwrap().doc.layers[0].name, "Edited source");
            assert_eq!(std::fs::read(&path).unwrap(), bytes, "original source is never rewritten");
        }
    }
}

#[test]
fn historical_byte_placements_store_the_explicit_source_interpretation() {
    for (little, order, interpretation) in
        [(true, "little-endian", HistoricalXcfSampleOrder::LittleEndian), (false, "big-endian", HistoricalXcfSampleOrder::BigEndian)]
    {
        for linked in [None, Some("not-read-during-byte-placement.xcf".into())] {
            let mut s = session();
            let bytes = fixture::historical_pixel(little);
            let started = s.start_place_bytes("source.xcf", bytes.clone(), linked, json!({"historicalXcfSampleOrder":order})).unwrap();
            let reply = match started {
                Started::Done(reply) => reply,
                Started::Job(id) => s.wait_job(id).unwrap(),
            };
            let id = LayerId(reply["layer"].as_u64().unwrap());
            assert_eq!(object(&s, id).historical_xcf_sample_order, Some(interpretation));
            if let SmartSource::Embedded { bytes: retained, .. } = &object(&s, id).source {
                assert_eq!(retained.as_slice(), bytes);
            }
        }
    }
}

#[test]
fn historical_relink_missing_old_file_preserves_order_and_storage_conversions_do_too() {
    let dir = tempfile::tempdir().unwrap();
    for (little, order, interpretation) in
        [(true, "little-endian", HistoricalXcfSampleOrder::LittleEndian), (false, "big-endian", HistoricalXcfSampleOrder::BigEndian)]
    {
        let old = dir.path().join(format!("old-{order}.xcf"));
        let relocated = dir.path().join(format!("relocated-{order}.xcf"));
        let linked_again = dir.path().join(format!("embedded-{order}.xcf"));
        let bytes = fixture::historical_pixel(little);
        std::fs::write(&old, &bytes).unwrap();
        let mut s = session();
        let reply = s.execute("file.placeLinked", json!({"path":old,"historicalXcfSampleOrder":order})).unwrap();
        let id = LayerId(reply["layer"].as_u64().unwrap());
        std::fs::rename(&old, &relocated).unwrap();
        s.execute("layer.smartObjects.relinkToFile", json!({"layer":id.0,"path":relocated})).unwrap();
        assert_eq!(object(&s, id).historical_xcf_sample_order, Some(interpretation));
        let before = s.active().unwrap().doc.clone();
        assert!(s.execute("layer.smartObjects.relinkToFile", json!({"layer":id.0,"path":relocated,"historicalXcfSampleOrder":null})).is_err());
        assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc), "clearing an order must not guess how to decode historical samples");
        s.execute("layer.smartObjects.convertToEmbedded", json!({"layer":id.0})).unwrap();
        assert_eq!(object(&s, id).historical_xcf_sample_order, Some(interpretation));
        s.execute("layer.smartObjects.convertToLinked", json!({"layer":id.0,"path":linked_again})).unwrap();
        assert_eq!(object(&s, id).historical_xcf_sample_order, Some(interpretation));
        assert_eq!(std::fs::read(&linked_again).unwrap(), bytes);
        s.execute("layer.smartObjects.editContents", json!({"layer":id.0})).unwrap();
        assert_child_pixels(&s);
        assert_eq!(std::fs::read(&relocated).unwrap(), bytes);
    }
}

#[test]
fn historical_relink_explicit_order_replaces_the_old_interpretation() {
    let dir = tempfile::tempdir().unwrap();
    let little = dir.path().join("little.xcf");
    let big = dir.path().join("big.xcf");
    std::fs::write(&little, fixture::historical_pixel(true)).unwrap();
    std::fs::write(&big, fixture::historical_pixel(false)).unwrap();
    let mut s = session();
    let reply = s.execute("file.placeLinked", json!({"path":little,"historicalXcfSampleOrder":"little-endian"})).unwrap();
    let id = LayerId(reply["layer"].as_u64().unwrap());
    s.execute("layer.smartObjects.relinkToFile", json!({"layer":id.0,"path":big,"historicalXcfSampleOrder":"big-endian"})).unwrap();
    assert_eq!(object(&s, id).historical_xcf_sample_order, Some(HistoricalXcfSampleOrder::BigEndian));
    s.execute("layer.smartObjects.editContents", json!({"layer":id.0})).unwrap();
    assert_child_pixels(&s);
}

#[test]
fn historical_converted_export_and_convert_to_layers_decode_the_stored_order() {
    let dir = tempfile::tempdir().unwrap();
    for (little, order) in [(true, "little-endian"), (false, "big-endian")] {
        let path = dir.path().join(format!("source-{order}.xcf"));
        let native = dir.path().join(format!("export-{order}.pcraft"));
        let bytes = fixture::historical_pixel(little);
        std::fs::write(&path, &bytes).unwrap();
        let mut s = session();
        let reply = s.execute("file.placeEmbedded", json!({"path":path,"historicalXcfSampleOrder":order})).unwrap();
        let id = LayerId(reply["layer"].as_u64().unwrap());
        let result = s.execute("layer.smartObjects.exportContents", json!({"layer":id.0,"path":native})).unwrap();
        assert_eq!(result["converted"], true);
        let exported = photocraft_format::load_from_bytes(&std::fs::read(&native).unwrap()).unwrap();
        let mut exported_session = Session::new();
        exported_session.add_document(exported.clone(), None);
        assert_child_pixels(&exported_session);
        // Match profiles so this also checks exact decoded samples rather than an unrelated
        // profile conversion when unpacking into the destination.
        s.edit("Match source profile", |doc, _| {
            doc.icc_profile = exported.icc_profile.clone();
            Ok(())
        })
        .unwrap();
        s.execute("layer.smartObjects.convertToLayers", json!({"layer":id.0})).unwrap();
        let layer = s.active().unwrap().doc.layer(s.active().unwrap().active_layer.unwrap()).unwrap();
        let LayerContent::Raster(surface) = &layer.content else { panic!("single source layer unpacked") };
        let mut pixel = [0.; 4];
        surface.read_pixel(0, 0, &mut pixel);
        assert_eq!(pixel, [0x1234 as f32 / 65535., 0x4567 as f32 / 65535., 0x89ab as f32 / 65535., 1.]);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn fresh_replacement_requires_its_own_order_and_native_replacement_clears_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("little.xcf");
    std::fs::write(&path, fixture::historical_pixel(true)).unwrap();
    let mut s = session();
    let reply = s.execute("file.placeEmbedded", json!({"path":path,"historicalXcfSampleOrder":"little-endian"})).unwrap();
    let id = LayerId(reply["layer"].as_u64().unwrap());
    let before = s.active().unwrap().doc.clone();
    assert!(replace_contents_from_bytes(&mut s, &json!({"layer":id.0}), "big.xcf", fixture::historical_pixel(false)).is_err());
    assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc), "a fresh source cannot inherit the old order");
    replace_contents_from_bytes(&mut s, &json!({"layer":id.0,"historicalXcfSampleOrder":"big-endian"}), "big.xcf", fixture::historical_pixel(false)).unwrap();
    assert_eq!(object(&s, id).historical_xcf_sample_order, Some(HistoricalXcfSampleOrder::BigEndian));
    s.execute("layer.smartObjects.editContents", json!({"layer":id.0})).unwrap();
    assert_child_pixels(&s);
    let native = encode_source(&s.active().unwrap().doc).unwrap();
    assert!(s.set_active(0));
    replace_contents_bytes(&mut s, id, "replacement.pcraft", native).unwrap();
    assert_eq!(object(&s, id).historical_xcf_sample_order, None);
    s.execute("layer.smartObjects.editContents", json!({"layer":id.0})).unwrap();
    assert_child_pixels(&s);
}

#[test]
fn persisted_missing_historical_order_never_guesses_when_editing_contents() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("historical.xcf");
    let bytes = fixture::historical_pixel(true);
    std::fs::write(&path, &bytes).unwrap();
    let mut s = session();
    let reply = s.execute("file.placeEmbedded", json!({"path":path,"historicalXcfSampleOrder":"little-endian"})).unwrap();
    let id = LayerId(reply["layer"].as_u64().unwrap());
    s.edit("Legacy source without interpretation", |doc, _| {
        smart_mut(doc, id)?.historical_xcf_sample_order = None;
        Ok(())
    })
    .unwrap();
    let mut reopened = roundtrip(&s);
    let before = reopened.active().unwrap().doc.clone();
    assert!(reopened.execute("layer.smartObjects.editContents", json!({"layer":id.0})).is_err());
    assert_eq!(reopened.documents().len(), 1);
    assert!(Arc::ptr_eq(&before, &reopened.active().unwrap().doc));
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}

#[test]
fn replacing_vector_source_clears_an_unrelated_historical_interpretation() {
    let mut s = session();
    let original = r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1" fill="red"/></svg>"#;
    let replacement = original.replace("red", "blue");
    let reply = crate::file_cmds::place_bytes(&mut s, "Artwork.svg", original.as_bytes().to_vec(), None, &json!({"smartObject":true})).unwrap();
    let id = LayerId(reply["layer"].as_u64().unwrap());
    s.edit("Old source interpretation", |doc, _| {
        smart_mut(doc, id)?.historical_xcf_sample_order = Some(HistoricalXcfSampleOrder::LittleEndian);
        Ok(())
    })
    .unwrap();
    let handoff = s.execute("layer.smartObjects.exportSvg", json!({"layer":id.0})).unwrap();
    s.execute(
        "layer.smartObjects.updateSvg",
        json!({
            "document":handoff["document"],"layer":id.0,"svg":replacement,
            "fingerprint":handoff["fingerprint"],"contentsId":handoff["contentsId"]
        }),
    )
    .unwrap();
    assert_eq!(object(&s, id).historical_xcf_sample_order, None);
}

#[test]
fn storage_conversions_ignore_unrelated_order_parameters_and_preserve_source_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.xcf");
    let destination = dir.path().join("converted.xcf");
    let bytes = fixture::historical_pixel(true);
    std::fs::write(&source, &bytes).unwrap();
    for unrelated in [json!("big-endian"), Value::Null, json!("invalid")] {
        let mut s = session();
        let reply = s.execute("file.placeLinked", json!({"path":source,"historicalXcfSampleOrder":"little-endian"})).unwrap();
        let id = LayerId(reply["layer"].as_u64().unwrap());
        s.execute("layer.smartObjects.convertToEmbedded", json!({"layer":id.0,"historicalXcfSampleOrder":unrelated})).unwrap();
        assert_eq!(object(&s, id).historical_xcf_sample_order, Some(HistoricalXcfSampleOrder::LittleEndian));
        assert_eq!(contents_of(&s.active().unwrap().doc, id).unwrap().1.as_slice(), bytes);
        s.execute("layer.smartObjects.convertToLinked", json!({"layer":id.0,"path":destination,"historicalXcfSampleOrder":unrelated})).unwrap();
        assert_eq!(object(&s, id).historical_xcf_sample_order, Some(HistoricalXcfSampleOrder::LittleEndian));
        assert_eq!(std::fs::read(&destination).unwrap(), bytes);
        s.execute("layer.smartObjects.editContents", json!({"layer":id.0})).unwrap();
        assert_child_pixels(&s);
    }
}

#[test]
fn relink_invalid_order_reports_its_command_before_attempting_file_io() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.xcf");
    std::fs::write(&source, fixture::historical_pixel(true)).unwrap();
    let mut s = session();
    let reply = s.execute("file.placeLinked", json!({"path":source,"historicalXcfSampleOrder":"little-endian"})).unwrap();
    let id = LayerId(reply["layer"].as_u64().unwrap());
    let before = s.active().unwrap().doc.clone();
    let error = s
        .execute("layer.smartObjects.relinkToFile", json!({"layer":id.0,"path":dir.path().join("missing.xcf"),"historicalXcfSampleOrder":"invalid"}))
        .unwrap_err();
    assert!(matches!(error, EngineError::BadParams { cmd, .. } if cmd == "layer.smartObjects.relinkToFile"));
    assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
}
