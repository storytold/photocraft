use photocraft_automation::Headless;
use photocraft_doc::LayerContent;
use serde_json::json;

#[test]
fn scoped_pathless_save_commits_smart_contents_without_filesystem_authority() {
    let mut headless = Headless::new();
    headless.handle("doc.new", json!({"width": 8, "height": 4, "background": "#ff0000"})).unwrap();
    headless.handle("engine.execute", json!({"command": "layer.smartObjects.convertToSmartObject", "params": {}})).unwrap();
    let opened = headless.handle("engine.execute", json!({"command": "layer.smartObjects.editContents", "params": {}})).unwrap();
    let child = opened["document"].as_u64().unwrap() as usize;
    headless.handle("engine.execute", json!({"command": "image.imageRotation.90cw", "params": {}})).unwrap();
    assert!(headless.handle("engine.execute", json!({"command": "file.save", "params": {}})).is_err());
    headless.handle("doc.save", json!({})).unwrap();
    assert!(!headless.session.documents()[child].is_dirty());
    let parent = &headless.session.documents()[0].doc;
    let smart = parent
        .walk()
        .into_iter()
        .find_map(|(_, _, layer)| match &layer.content {
            LayerContent::Smart(smart) => Some(smart),
            _ => None,
        })
        .unwrap();
    let (name, bytes) = photocraft_engine::smart_cmds::source_bytes(&parent.metadata, &smart.source).unwrap();
    let committed = photocraft_engine::smart_cmds::decode_source(&name, &bytes, None).unwrap();
    assert_eq!((committed.size.width, committed.size.height), (4, 8));
    assert_eq!(photocraft_compose::flatten(&committed).px[0], [1.0, 0.0, 0.0, 1.0]);
    assert!(headless.handle("doc.save", json!({"path": "not-authorized.pcraft"})).is_err());
}
