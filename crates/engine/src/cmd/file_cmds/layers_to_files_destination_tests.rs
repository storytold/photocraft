//! #13969: Layers to Files refuses an empty destination and one that doesn't exist, before
//! writing anything or creating a folder; an existing one exports as before.

use super::*;

const EXPORT: &str = "file.export.layersToFiles";

/// A document with the layers `Top` and `Background`.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Top"})).unwrap();
    s
}

#[test]
fn an_empty_destination_asks_for_one() {
    let mut s = session();
    for params in [json!({}), json!({"dir": ""}), json!({"dir": "   "}), json!({"output": ""}), json!({"dir": null}), json!({"dir": 7})] {
        let e = s.execute(EXPORT, params.clone()).unwrap_err();
        assert_eq!(e.to_string(), "Please specify destination.", "{params}");
    }
}

#[test]
fn a_missing_destination_is_named_and_never_created() {
    let mut s = session();
    let parent = tempfile::tempdir().unwrap();
    let missing = parent.path().join("nope").join("x");
    let shown = missing.to_string_lossy().into_owned();
    for key in ["dir", "output"] {
        let e = s.execute(EXPORT, json!({key: shown})).unwrap_err();
        assert_eq!(e.to_string(), format!("Destination does not exist: {shown}"), "{key}");
    }
    assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 0, "no folder was created");
}

#[test]
fn an_existing_destination_exports_as_before() {
    let mut s = session();
    let dir = tempfile::tempdir().unwrap();
    let r = s.execute(EXPORT, json!({"dir": dir.path().to_string_lossy(), "prefix": "doc"})).unwrap();
    let files: Vec<&str> = r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap()).collect();
    assert_eq!(files.len(), 2);
    assert!(files[0].ends_with("doc_0000_Top.png"), "{files:?}");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
}
