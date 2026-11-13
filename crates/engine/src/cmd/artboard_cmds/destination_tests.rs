use super::*;

const EXPORT: &str = "file.export.artboardsToFiles";

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 32, "height": 16})).unwrap();
    for (name, x) in [("Red", 0), ("Blue", 16)] {
        s.execute("layer.new.artboard", json!({"name": name, "rect": [x, 0, 16, 16]})).unwrap();
    }
    s
}

#[test]
fn empty_destination_asks_for_a_folder() {
    let mut s = session();
    for params in [json!({"dir": ""}), json!({"output": ""}), json!({})] {
        assert_eq!(s.execute(EXPORT, params).unwrap_err().to_string(), "Choose a destination folder.");
    }
}

#[test]
fn missing_destination_names_the_path_without_creating_anything() {
    let mut s = session();
    let parent = tempfile::tempdir().unwrap();
    let missing = parent.path().join("missing").join("nested");
    for params in [json!({"dir": missing}), json!({"output": missing})] {
        let err = s.execute(EXPORT, params).unwrap_err().to_string();
        assert_eq!(err, format!("Destination does not exist: {}", missing.display()));
        assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 0);
    }
}

#[test]
fn successful_export_preserves_the_document_selection_and_history() {
    let mut s = session();
    s.execute("select.rect", json!({"x": 17, "y": 1, "width": 4, "height": 6})).unwrap();
    let before = s.active().unwrap().clone();
    assert!(before.doc.selection.is_some());
    assert!(!before.selected_layers().is_empty());
    let dir = tempfile::tempdir().unwrap();

    let result = s.execute(EXPORT, json!({"output": dir.path(), "prefix": "export"})).unwrap();

    assert_eq!(result["files"].as_array().unwrap().len(), 2);
    assert!(dir.path().join("export_0000_Blue.png").is_file());
    assert!(dir.path().join("export_0001_Red.png").is_file());
    let after = s.active().unwrap();
    assert!(std::sync::Arc::ptr_eq(&before.doc, &after.doc), "the source, including its pixel selection, is unchanged");
    assert_eq!(after.active_layer, before.active_layer);
    assert_eq!(after.selected_layers(), before.selected_layers());
    assert_eq!(after.revision, before.revision);
    assert_eq!(after.history.entries(), before.history.entries());
    assert!(after.history.redo_labels().eq(before.history.redo_labels()));
}
