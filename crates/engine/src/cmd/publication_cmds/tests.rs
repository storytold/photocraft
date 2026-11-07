use serde_json::json;

use super::*;

fn session(w: u32, h: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "background": "white"})).unwrap();
    s
}

/// A folder removed when it goes out of scope, a failed assertion included.
fn tmp() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
fn writes_tiff_and_png_at_300_dpi_without_touching_the_document() {
    let mut s = session(1200, 800);
    let tmp_dir = tmp();
    let dir = tmp_dir.path();
    let before = photocraft_doc::Document::clone(&s.active().unwrap().doc);
    let r = s.execute(CMD, json!({"folder": dir.to_string_lossy(), "name": "fig1"})).unwrap();
    assert_eq!((r["width"].as_u64(), r["height"].as_u64(), r["dpi"].as_f64()), (Some(1200), Some(800), Some(300.0)), "{r}");
    assert_eq!(r["widthMm"].as_f64(), Some(101.6));
    for (ext, fmt) in [("tif", "tiff"), ("png", "png")] {
        let path = dir.join(format!("fig1.{ext}"));
        let bytes = std::fs::read(&path).unwrap();
        let d = photocraft_io::import(&path.to_string_lossy(), &bytes).unwrap().document;
        assert_eq!((d.size.width, d.size.height), (1200, 800), "{fmt}");
        assert!((d.resolution_dpi - 300.0).abs() < 0.5, "{fmt} keeps 300 dpi: {}", d.resolution_dpi);
    }
    assert_eq!(*s.active().unwrap().doc, before, "the document is unchanged");
}

#[test]
fn a_column_width_resamples_to_that_print_size() {
    let mut s = session(1000, 500);
    let tmp_dir = tmp();
    let dir = tmp_dir.path();
    let r = s.execute(CMD, json!({"folder": dir.to_string_lossy(), "name": "fig", "formats": ["png"], "width": "double", "dpi": 600})).unwrap();
    // 175 mm at 600 dpi = 4134 px; the height keeps the aspect ratio.
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(4134), Some(2067)), "{r}");
    assert_eq!(r["files"].as_array().map(Vec::len), Some(1));
    let only_tiff = s.execute(CMD, json!({"folder": dir.to_string_lossy(), "name": "fig2", "tiff": true, "png": false})).unwrap();
    assert_eq!(only_tiff["files"][0]["format"], "tiff", "{only_tiff}");
    assert_eq!(only_tiff["files"].as_array().map(Vec::len), Some(1));
    assert!(r["warnings"].as_array().is_some_and(|w| w.iter().any(|m| m.as_str().is_some_and(|m| m.contains("enlarged")))), "{r}");
}

#[test]
fn bad_params_are_errors_and_write_nothing() {
    let mut s = session(100, 100);
    let tmp_dir = tmp();
    let dir = tmp_dir.path();
    let folder = dir.to_string_lossy().to_string();
    for p in [
        json!({}),
        json!({"folder": folder, "dpi": 10}),
        json!({"folder": folder, "dpi": "300"}),
        json!({"folder": folder, "formats": []}),
        json!({"folder": folder, "formats": ["jpeg"]}),
        json!({"folder": folder, "formats": "tiff"}),
        json!({"folder": folder, "width": "triple"}),
        json!({"folder": folder, "width": "custom"}),
        json!({"folder": folder, "width": "custom", "widthMm": 1e9}),
        json!({"folder": folder, "name": "../escape"}),
        json!({"folder": folder, "tiff": false, "png": false}),
        json!({"folder": folder, "tiff": "yes"}),
    ] {
        assert!(s.execute(CMD, p.clone()).is_err(), "{p}");
    }
    assert_eq!(std::fs::read_dir(dir).unwrap().count(), 0);
    assert!(!s.prefs().dialogs.contains_key(CMD), "a rejected export remembers nothing");
    assert!(Session::new().execute(CMD, json!({"folder": "/tmp"})).is_err());
}
