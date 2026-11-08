//! Save As must store the new filename in the native manifest, not just the in-session tab name.

use photocraft_automation::{Headless, files};
use serde_json::json;

#[test]
fn native_save_as_reopens_with_the_written_filename() {
    let dir = std::env::temp_dir().join(format!(
        "pc-native-save-name-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let first = dir.join("FirstTest.pcraft");
    let second = dir.join("SecondTest.pcraft");
    let mut headless = Headless::trusted_local();
    headless.handle("doc.new", json!({"width": 8, "height": 8, "name": "Old Title"})).unwrap();
    headless.handle("doc.save", json!({"path": first.to_str().unwrap()})).unwrap();
    assert_eq!(files::open(&first).unwrap().document.name, "FirstTest.pcraft");
    headless.handle("doc.save", json!({"path": second.to_str().unwrap()})).unwrap();
    assert_eq!(headless.session.active().unwrap().doc.name, "SecondTest.pcraft");
    assert_eq!(files::open(&second).unwrap().document.name, "SecondTest.pcraft");
    // Save back in place and retain the correct title after another edit.
    headless.handle("engine.execute", json!({"command":"image.adjustments.invert", "params":{}})).unwrap();
    headless.handle("doc.save", json!({})).unwrap();
    assert_eq!(files::open(&second).unwrap().document.name, "SecondTest.pcraft");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_encoded_native_bundle_uses_requested_name_without_mutating_source() {
    let mut headless = Headless::trusted_local();
    headless.handle("doc.new", json!({"width": 8, "height": 8, "name": "Original"})).unwrap();
    let doc = &headless.session.active().unwrap().doc;
    let (bytes, warnings) = files::save_bytes(doc, "exports/Renamed.pcraft", None, &Default::default()).unwrap();
    assert!(warnings.is_empty());
    let reopened = files::open_bytes("Renamed.pcraft", &bytes).unwrap().document;
    assert_eq!(reopened.name, "Renamed.pcraft");
    assert_eq!(doc.name, "Original", "a copy should not retarget the open document");
}

#[test]
fn a_timeline_the_load_drops_is_reported_with_the_open() {
    let mut headless = Headless::trusted_local();
    headless.handle("doc.new", json!({"width": 8, "height": 8, "name": "Lossy"})).unwrap();
    let mut doc = (*headless.session.active().unwrap().doc).clone();
    // An unscoped id far ahead of the counter makes the loader remap, dropping the timeline.
    doc.layers[0].id = photocraft_doc::LayerId((1 << 52) + 50_000_000);
    let base = std::sync::Arc::new(doc.clone());
    doc.design_timeline = Some(std::sync::Arc::new(photocraft_doc::design::DesignTimeline { base, steps: Vec::new(), marker: None }));
    let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
    let opened = files::open_bytes("Lossy.pcraft", &bytes).unwrap();
    assert!(opened.document.design_timeline.is_none());
    assert!(opened.warnings.iter().any(|w| w.contains("design timeline")), "{:?}", opened.warnings);
}
