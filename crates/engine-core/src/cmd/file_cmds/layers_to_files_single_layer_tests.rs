use super::*;

const COMMAND: &str = "file.export.layersToFiles";
const MESSAGE: &str = "Layers to Files needs a document with multiple layers.";

#[test]
fn layers_to_files_refuses_a_background_only_document_without_writing() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 50, "height": 40, "background": "white"})).unwrap();
    s.edit("Use an imported Background name", |doc, _| {
        doc.background_mut().unwrap().name = "Hintergrund".into();
        Ok(())
    })
    .unwrap();
    let dir = tempfile::tempdir().unwrap();

    let error = s.execute(COMMAND, json!({"dir": dir.path()})).unwrap_err();
    assert_eq!(error.to_string(), MESSAGE);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn layers_to_files_refuses_a_background_only_document_before_streaming() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 50, "height": 40, "background": "white"})).unwrap();

    let mut writes = 0;
    let error = export_layers_report_with(&s, &json!({"visibleOnly": false}), |_, _| {
        writes += 1;
        Ok(())
    })
    .unwrap_err();
    assert_eq!(error.to_string(), MESSAGE);
    assert_eq!(writes, 0);
}

#[test]
fn layers_to_files_still_exports_a_single_normal_layer_named_background() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 50, "height": 40, "background": "transparent"})).unwrap();
    s.execute("layer.setProps", json!({"name": "Background"})).unwrap();
    let dir = tempfile::tempdir().unwrap();

    let result = s.execute(COMMAND, json!({"dir": dir.path(), "prefix": "doc"})).unwrap();
    assert_eq!(result["files"].as_array().unwrap().len(), 1);
    assert!(dir.path().join("doc_0000_Background.png").is_file());
}
