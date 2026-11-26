use super::*;
use photocraft_doc::import_conversion::{ConversionEntry, ConversionReason, ConversionReport, NativeTargetRef, RetainedImportData, SourceItemRef};
use photocraft_doc::{ColorMode, Document, Layer, LayerId, SampleType};
use photocraft_geom::Size;

fn session_with_report(count: usize) -> (Session, LayerId) {
    let mut doc = Document::new("Imported", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    let layer = Layer::raster("Current owner", doc.pixel_format());
    let id = layer.id;
    doc.layers.push(layer);
    let dispositions = [D::NativeEquivalent, D::NativeApproximation, D::RetainedUnapplied, D::SkippedNonessential];
    let entries = (0..count)
        .map(|ordinal| ConversionEntry {
            feature_code: format!("fixture.feature.{ordinal}"),
            source: SourceItemRef { kind: "layer".into(), ordinal: ordinal as u32, name: "Original owner".into(), ..Default::default() },
            targets: vec![NativeTargetRef { layer_id: Some(id), label: "Historical owner".into(), ..Default::default() }],
            disposition: dispositions[ordinal % dispositions.len()],
            reason: ConversionReason::NativePolicy,
            source_description: "Import-time description".into(),
            native_description: "Converted native owner".into(),
            attachments: vec![0],
        })
        .collect();
    doc.metadata.import_conversion = Some(Arc::new(ImportConversion::Report(ConversionReport {
        source_format: "xcf".into(),
        source_version: Some(11),
        policy_version: 1,
        entries,
        attachments: vec![RetainedImportData {
            owner: SourceItemRef { kind: "layer".into(), ..Default::default() },
            kind: "optional configuration".into(),
            encoding: "opaque".into(),
            bytes: Arc::new(b"DO_NOT_EXPOSE_RETAINED_BYTES".to_vec()),
        }],
    })));
    let mut session = crate::session();
    session.add_document(doc, None);
    (session, id)
}

#[test]
fn import_report_pages_are_bounded_and_preserve_order_and_summary() {
    let (mut s, _) = session_with_report(252);
    let default = s.execute(CMD, json!({})).unwrap();
    assert_eq!(default["total"], 252);
    assert_eq!(default["entries"].as_array().unwrap().len(), 100);
    assert_eq!(default["nextOffset"], 100);
    assert_eq!(default["entries"][0]["featureCode"], "fixture.feature.0");
    assert_eq!(default["entries"][99]["source"]["ordinal"], 99);
    let summary = &default["summary"];
    assert_eq!(summary["schemaVersion"], 1);
    assert_eq!(summary["sourceFormat"], "xcf");
    assert_eq!(summary["sourceVersion"], 11);
    assert_eq!(summary["policyVersion"], 1);
    assert_eq!(summary["attachmentCount"], 1);
    for disposition in ["native_equivalent", "native_approximation", "retained_unapplied", "skipped_nonessential"] {
        assert_eq!(summary["dispositionCounts"][disposition], 63);
    }
    let first = s.execute(CMD, json!({"limit":250})).unwrap();
    assert_eq!(first["entries"].as_array().unwrap().len(), 250);
    assert_eq!(first["nextOffset"], 250);
    let last = s.execute(CMD, json!({"offset":250,"limit":250})).unwrap();
    assert_eq!(last["entries"].as_array().unwrap().len(), 2);
    assert_eq!(last["entries"][0]["featureCode"], "fixture.feature.250");
    assert_eq!(last["entries"][1]["featureCode"], "fixture.feature.251");
    assert!(last["nextOffset"].is_null());
    assert_eq!(s.execute("document.inspect", json!({})).unwrap()["importConversion"], *summary);
}

#[test]
fn import_report_empty_and_far_out_pages_do_not_wrap() {
    for count in [0, 3] {
        let (mut s, _) = session_with_report(count);
        for offset in [count as u64, u64::MAX] {
            let page = s.execute(CMD, json!({"offset":offset,"limit":1})).unwrap();
            assert_eq!(page["offset"], offset);
            assert_eq!(page["total"], count);
            assert_eq!(page["entries"], json!([]));
            assert!(page["nextOffset"].is_null());
        }
    }
}

#[test]
fn invalid_report_pagination_is_refused_without_editing() {
    let (mut s, _) = session_with_report(3);
    let before = s.active().unwrap().clone();
    for params in [
        json!({"limit":0}),
        json!({"limit":251}),
        json!({"limit":-1}),
        json!({"limit":1.5}),
        json!({"limit":true}),
        json!({"limit":"1"}),
        json!({"limit":null}),
        json!({"offset":-1}),
        json!({"offset":1.5}),
        json!({"offset":false}),
        json!({"offset":"1"}),
        json!({"offset":null}),
    ] {
        let error = s.execute(CMD, params.clone()).unwrap_err();
        assert!(matches!(error, EngineError::BadParams { .. }), "{params}: {error}");
    }
    for params in [Value::Null, json!([]), json!(false)] {
        assert!(page(&mut s, &params).is_err());
    }
    assert!(Arc::ptr_eq(&before.doc, &s.active().unwrap().doc));
    assert_eq!(before.history.past_len(), s.active().unwrap().history.past_len());
}

#[test]
fn report_queries_expose_no_payload_and_change_no_document_history_or_identity() {
    let (mut s, _) = session_with_report(4);
    s.execute("layer.new.layer", json!({"name":"Undo fixture"})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.active().unwrap().history.can_redo());
    s.active_mut().unwrap().source_read_only = true;
    let before = s.active().unwrap().clone();
    let original = before.doc.metadata.import_conversion.as_ref().unwrap();
    assert!(!crate::registry::find(CMD).unwrap().journal);
    for params in [json!({}), json!({"offset":1,"limit":2}), json!({"offset":u64::MAX})] {
        let page = s.execute(CMD, params).unwrap();
        let serialized = page.to_string();
        assert!(!serialized.contains("DO_NOT_EXPOSE_RETAINED_BYTES"));
        assert!(!serialized.contains("optional configuration"));
        assert!(!serialized.contains("\"bytes\""));
        for entry in page["entries"].as_array().unwrap() {
            assert_eq!(entry["attachments"], json!([0]));
        }
    }
    let after = s.active().unwrap();
    assert!(Arc::ptr_eq(&before.doc, &after.doc));
    assert!(Arc::ptr_eq(original, after.doc.metadata.import_conversion.as_ref().unwrap()));
    assert_eq!(after.doc.id, before.doc.id);
    assert_eq!(after.active_layer, before.active_layer);
    assert_eq!(after.revision, before.revision);
    assert_eq!(after.saved_revision, before.saved_revision);
    assert_eq!(after.history.past_len(), before.history.past_len());
    assert_eq!(after.history.can_redo(), before.history.can_redo());
    assert_eq!(after.path, before.path);
    assert_eq!(after.source_read_only, before.source_read_only);
}

#[test]
fn targets_remain_historical_when_the_current_layer_is_renamed_or_deleted() {
    let (mut s, id) = session_with_report(1);
    s.execute("layer.setProps", json!({"layer":id.0,"name":"Renamed owner"})).unwrap();
    let page = s.execute(CMD, json!({})).unwrap();
    assert_eq!(page["entries"][0]["source"]["name"], "Original owner");
    assert_eq!(page["entries"][0]["targets"][0]["label"], "Historical owner");
    assert_eq!(page["entries"][0]["targets"][0]["currentName"], "Renamed owner");
    assert_eq!(page["entries"][0]["targets"][0]["present"], true);
    assert_eq!(page["entries"][0]["targets"][0]["historical"], true);
    s.execute("layer.new.layer", json!({"name":"Surviving owner"})).unwrap();
    s.execute("layer.delete", json!({"layer":id.0})).unwrap();
    let page = s.execute(CMD, json!({})).unwrap();
    assert_eq!(page["entries"][0]["targets"][0]["layerId"], id.0);
    assert_eq!(page["entries"][0]["targets"][0]["present"], false);
    assert!(page["entries"][0]["targets"][0]["currentName"].is_null());
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.execute(CMD, json!({})).unwrap()["entries"][0]["targets"][0]["present"], true);
}

#[test]
fn absent_and_future_reports_are_distinct_without_interpreting_opaque_bytes() {
    let mut s = crate::session();
    assert!(s.execute(CMD, json!({})).is_err());
    s.add_document(Document::new("Plain", Size::new(1, 1), ColorMode::Rgb, SampleType::U8), None);
    let empty = s.execute(CMD, json!({})).unwrap();
    assert!(empty["summary"].is_null());
    assert_eq!(empty["entries"], json!([]));
    assert_eq!(empty["opaque"], false);
    assert!(s.execute("document.inspect", json!({})).unwrap()["importConversion"].is_null());
    let mut future = Document::new("Future", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    future.metadata.import_conversion = Some(Arc::new(ImportConversion::Opaque {
        schema_version: 2,
        record: Arc::new(b"DO_NOT_PARSE_FUTURE_RECORD".to_vec()),
        attachments: vec![Arc::new(b"DO_NOT_EXPOSE_FUTURE_BYTES".to_vec())],
    }));
    s.add_document(future, None);
    let page = s.execute(CMD, json!({})).unwrap();
    assert_eq!(page["opaque"], true);
    assert_eq!(page["summary"]["schemaVersion"], 2);
    assert_eq!(page["summary"]["attachmentCount"], 1);
    assert!(page["summary"]["entryCount"].is_null());
    assert_eq!(page["entries"], json!([]));
    assert!(!page.to_string().contains("DO_NOT_"));
}

#[test]
fn invalid_known_report_is_rejected_before_serializing_entries() {
    let (mut s, _) = session_with_report(1);
    s.edit("Malformed fixture", |doc, _| {
        let conversion = Arc::make_mut(doc.metadata.import_conversion.as_mut().unwrap());
        let ImportConversion::Report(report) = conversion else { panic!("known report") };
        report.entries[0].attachments = vec![1];
        Ok(())
    })
    .unwrap();
    let before = s.active().unwrap().doc.clone();
    assert!(s.execute(CMD, json!({})).unwrap_err().to_string().contains("attachment reference"));
    assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
}

#[test]
fn report_query_survives_native_save_and_reopen_without_source_decoder_dependencies() {
    let (mut s, _) = session_with_report(4);
    let original = s.execute(CMD, json!({})).unwrap();
    let bytes = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap();
    let doc = photocraft_format::load_from_bytes(&bytes).unwrap();
    let mut reopened = crate::session();
    reopened.add_document(doc, None);
    assert_eq!(reopened.execute(CMD, json!({})).unwrap(), original);
}
