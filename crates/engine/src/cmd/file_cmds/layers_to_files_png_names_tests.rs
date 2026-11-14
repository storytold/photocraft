//! Layers to Files: PNG file names turn spaces in the prefix and the layer name into dashes; the
//! other formats keep them (#13963).

use super::*;

fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("photocraft-l2f-png-names-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().into_owned()
}

/// Background, `Blue Half`, and a hidden layer `Hid den` on top.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4, "height": 4, "depth": 8})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Blue Half"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Hid den"})).unwrap();
    s.execute("layer.setProps", json!({"visible": false})).unwrap();
    s
}

fn export(test: &str, format: &str) -> serde_json::Value {
    let dir = tmp(&format!("{test}-{format}"));
    let mut s = session();
    s.execute("file.export.layersToFiles", json!({"dir": dir, "prefix": "p 1", "format": format})).unwrap()
}

fn file_names(result: &serde_json::Value) -> Vec<String> {
    result["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap().rsplit(['/', '\\']).next().unwrap().to_string()).collect()
}

#[test]
fn png_names_use_dashes_for_spaces() {
    let result = export("dashes", "png");
    assert_eq!(file_names(&result), vec!["p-1_0000_Blue-Half.png".to_string(), "p-1_0001_Background.png".to_string()]);
}

#[test]
fn other_formats_keep_spaces() {
    for format in ["psd", "jpg", "tif"] {
        let result = export("keep", format);
        assert_eq!(file_names(&result), vec![format!("p 1_0000_Blue Half.{format}"), format!("p 1_0001_Background.{format}")], "{format}");
    }
}

#[test]
fn skipped_layers_keep_the_original_name() {
    for format in ["png", "psd"] {
        let result = export("skipped", format);
        let skipped = result["skipped"].as_array().unwrap();
        assert_eq!(skipped.len(), 1, "{format}");
        assert_eq!(skipped[0]["name"], json!("Hid den"), "{format}");
        assert_eq!(skipped[0]["reason"], json!("hidden by visibleOnly"), "{format}");
    }
}
