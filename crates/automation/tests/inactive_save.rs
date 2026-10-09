//! Regression for #1350: saving an inactive .pcraft file must not select it.

use photocraft_automation::Headless;
use serde_json::json;

#[test]
fn indexed_native_save_preserves_active_document_and_future_command_target() {
    let dir = std::env::temp_dir().join(format!(
        "pc-inactive-save-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("first.pcraft");
    let mut headless = Headless::trusted_local();

    headless.handle("doc.new", json!({"width": 2, "height": 2, "background": "white", "name": "first"})).unwrap();
    headless.handle("doc.new", json!({"width": 2, "height": 2, "background": "white", "name": "second"})).unwrap();
    assert_eq!(headless.session.active_index(), Some(1));
    let first_revision = headless.session.documents()[0].revision;
    let second_revision = headless.session.documents()[1].revision;

    headless.handle("doc.save", json!({"index": 0, "path": path.to_str().unwrap()})).unwrap();
    assert!(path.is_file(), "inactive document was saved");
    assert_eq!(headless.session.active_index(), Some(1), "save must not select document 0");
    let first = &headless.session.documents()[0];
    assert_eq!(first.path.as_deref(), path.to_str());
    assert_eq!(first.revision, first.saved_revision, "saved document is clean");

    headless.handle("engine.execute", json!({"command": "image.adjustments.invert", "params": {}})).unwrap();
    assert_eq!(headless.session.active_index(), Some(1));
    assert_eq!(headless.session.documents()[0].revision, first_revision);
    assert!(headless.session.documents()[1].revision > second_revision);
    drop(headless);
    std::fs::remove_dir_all(dir).unwrap();
}
