use std::sync::Arc;

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::import_conversion::{
    ConversionDisposition as D, ConversionEntry, ConversionReason as R, ConversionReport, ImportConversion, NativeTargetRef, RetainedImportData, SourceItemRef,
};
use photocraft_doc::{Document, Layer};
use photocraft_engine::Session;
use photocraft_geom::Size;
use serde_json::{Value, json};

fn session() -> Session {
    let mut doc = Document::new("Native converted", Size::new(3, 2), ColorMode::Rgb, SampleType::U8);
    doc.layers.push(Layer::raster("Native one", doc.pixel_format()));
    doc.layers.push(Layer::raster("Native two", doc.pixel_format()));
    let entries = (0..3)
        .map(|ordinal| ConversionEntry {
            feature_code: "xcf.test_mapping".into(),
            source: SourceItemRef { kind: "layer".into(), ordinal, name: "Duplicate".into(), ..Default::default() },
            targets: doc.layers.iter().map(|l| NativeTargetRef { layer_id: Some(l.id), ..Default::default() }).collect(),
            disposition: if ordinal == 0 { D::NativeEquivalent } else { D::NativeApproximation },
            reason: R::NativePolicy,
            source_description: "Source layer".into(),
            native_description: "Native layers".into(),
            attachments: vec![0],
        })
        .collect();
    doc.metadata.import_conversion = Some(ImportConversion::Report(ConversionReport {
        source_format: "xcf".into(),
        source_version: Some(11),
        policy_version: 1,
        entries,
        attachments: vec![RetainedImportData {
            owner: SourceItemRef { kind: "layer".into(), ..Default::default() },
            kind: "optional config".into(),
            encoding: "opaque".into(),
            bytes: Arc::new(b"private binary C:\\unpublished\\resource.asset".to_vec()),
        }],
    }));
    let mut s = Session::new();
    s.add_document(doc, None);
    s
}

#[test]
fn report_absence_is_not_a_fidelity_claim_and_no_document_fails_normally() {
    let mut s = Session::new();
    assert!(s.execute("document.importReport", json!({})).is_err());
    s.execute("file.new", json!({"width": 2, "height": 2})).unwrap();
    assert_eq!(s.execute("document.inspect", json!({})).unwrap()["importConversion"], Value::Null);
    let page = s.execute("document.importReport", json!({})).unwrap();
    assert_eq!(page["total"], 0);
    assert_eq!(page["entries"], json!([]));
    assert_eq!(page["nextOffset"], Value::Null);
}

#[test]
fn inspection_reports_live_source_protection_independently_of_historical_conversion() {
    let mut s = session();
    assert_eq!(s.execute("document.inspect", json!({})).unwrap()["sourceReadOnly"], false);
    assert_eq!(photocraft_engine::inspect::session(&s)["documents"][0]["sourceReadOnly"], false);
    s.active_mut().unwrap().source_read_only = true;
    assert_eq!(s.execute("document.inspect", json!({})).unwrap()["sourceReadOnly"], true);
    assert_eq!(photocraft_engine::inspect::session(&s)["documents"][0]["sourceReadOnly"], true);
    // A fresh native reopen carries the historical report, not the live grant
    // or source protection state of the session that imported it.
    let bytes = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap();
    let mut reopened = Session::new();
    reopened.color.settings.ask_on_missing = false;
    photocraft_engine::file_cmds::open_bytes_as(&mut reopened, "native.pcraft", &bytes, None, None).unwrap();
    assert_eq!(reopened.execute("document.inspect", json!({})).unwrap()["sourceReadOnly"], false);
    assert!(reopened.execute("document.inspect", json!({})).unwrap()["importConversion"].is_object());
}

#[test]
fn duplicate_names_one_to_many_targets_and_stale_references_page_without_mutation() {
    let mut s = session();
    let summary = s.execute("document.inspect", json!({})).unwrap()["importConversion"].clone();
    assert_eq!(summary["entryCount"], 3);
    assert_eq!(summary["attachmentCount"], 1);
    assert_eq!(summary["dispositionCounts"]["native_equivalent"], 1);
    assert_eq!(summary["dispositionCounts"]["native_approximation"], 2);
    let state = s.active().unwrap();
    let before = (state.doc.clone(), state.revision, state.is_dirty(), state.history.entries());
    let first = s.execute("document.importReport", json!({"limit": 2})).unwrap();
    assert_eq!(first["nextOffset"], 2);
    assert_eq!(first["entries"].as_array().unwrap().len(), 2);
    assert_eq!(first["entries"][0]["source"]["name"], first["entries"][1]["source"]["name"]);
    assert_ne!(first["entries"][0]["source"]["ordinal"], first["entries"][1]["source"]["ordinal"]);
    assert_eq!(first["entries"][0]["targets"].as_array().unwrap().len(), 2);
    assert_eq!(first, s.execute("document.importReport", json!({"limit": 2})).unwrap());
    let last = s.execute("document.importReport", json!({"offset": 2, "limit": 2})).unwrap();
    assert_eq!(last["entries"].as_array().unwrap().len(), 1);
    assert_eq!(last["nextOffset"], Value::Null);
    assert!(!first.to_string().contains("unpublished"));
    assert!(!first.to_string().contains("private binary"));
    let state = s.active().unwrap();
    assert_eq!((state.doc.clone(), state.revision, state.is_dirty(), state.history.entries()), before);
    s.execute("layer.delete", json!({})).unwrap();
    let stale = s.execute("document.importReport", json!({})).unwrap();
    assert!(stale["entries"][0]["targets"].as_array().unwrap().iter().any(|t| t["present"] == false && t["historical"] == true));
    assert_eq!(s.execute("document.inspect", json!({})).unwrap()["importConversion"], summary);
}

#[test]
fn report_paging_validates_types_and_bounds_without_overflow() {
    // `offset` must be a nonnegative integer; `limit` is 1-250, default 100.
    let mut s = session();
    for params in [
        json!(null),
        json!([]),
        json!("x"),
        json!({"offset": -1}),
        json!({"offset": 1.5}),
        json!({"offset": "1"}),
        json!({"offset": null}),
        json!({"limit": 0}),
        json!({"limit": 251}),
        json!({"limit": -1}),
        json!({"limit": true}),
        json!({"limit": null}),
        json!({"limit": "2"}),
    ] {
        assert!(s.execute("document.importReport", params.clone()).is_err(), "{params}");
    }
    for offset in [3, 100, u64::MAX] {
        let page = s.execute("document.importReport", json!({"offset": offset})).unwrap();
        assert_eq!(page["offset"], offset);
        assert_eq!(page["entries"], json!([]));
        assert_eq!(page["nextOffset"], Value::Null);
    }
    assert_eq!(s.execute("document.importReport", json!({"limit": 250})).unwrap()["entries"].as_array().unwrap().len(), 3);
}

#[test]
fn future_opaque_reports_are_summarized_without_parsing_or_exposing_their_bytes() {
    let mut doc = Document::new("Future", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    doc.metadata.import_conversion = Some(ImportConversion::Opaque {
        schema_version: 2,
        record: Arc::new(b"not parsed private record".to_vec()),
        attachments: vec![Arc::new(vec![1, 2, 3])],
    });
    let mut s = Session::new();
    s.add_document(doc, None);
    let summary = s.execute("document.inspect", json!({})).unwrap()["importConversion"].clone();
    assert_eq!(summary["schemaVersion"], 2);
    assert_eq!(summary["entryCount"], Value::Null);
    let page = s.execute("document.importReport", json!({})).unwrap();
    assert_eq!(page["opaque"], true);
    assert_eq!(page["entries"], json!([]));
    assert!(!page.to_string().contains("private"));
}

#[test]
fn mandatory_xcf_report_overflow_fails_before_publishing_a_document() {
    // Six references to one small record with a long escaped name exceed the independent
    // encoded-report allowance, while copied source names remain below their old 32 MiB cap.
    let mut bytes = b"gimp xcf v011\0".to_vec();
    for value in [1u32, 1, 0, 150, 0, 0] {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    let pointer_start = bytes.len();
    bytes.resize(pointer_start + 8 * 8, 0); // Six layers, layer terminator, channel terminator.
    let record = bytes.len() as u64;
    for pointer in bytes[pointer_start..].as_chunks_mut::<8>().0.iter_mut().take(6) {
        pointer.copy_from_slice(&record.to_be_bytes());
    }
    for value in [1u32, 1, 1, 1 << 20] {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    bytes.resize(bytes.len() + (1 << 20) - 1, 1);
    bytes.push(0);
    bytes.extend_from_slice(&[0; 24]); // End properties and absent hierarchy/mask.
    let mut s = session();
    let before = s.active().unwrap().doc.clone();
    let revision = s.active().unwrap().revision;
    let error = photocraft_engine::file_cmds::open_bytes_as(&mut s, "report-limit.xcf", &bytes, None, None).unwrap_err();
    assert!(error.to_string().contains("conversion report"), "{error}");
    assert_eq!(s.documents().len(), 1);
    assert!(Arc::ptr_eq(&s.active().unwrap().doc, &before));
    assert_eq!(s.active().unwrap().revision, revision);
}
