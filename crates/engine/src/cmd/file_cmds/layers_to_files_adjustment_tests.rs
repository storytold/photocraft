//! Layers to Files writes a file for an adjustment layer, like an empty layer (#13966).

use super::*;

fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("photocraft-l2f-adjustment-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().into_owned()
}

/// A 100 x 80 document: `Background`, then `Empty`, then a Levels adjustment layer named `Levels 1`.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 100, "height": 80, "depth": 8})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Empty"})).unwrap();
    s.execute("layer.newAdjustmentLayer.levels", json!({})).unwrap();
    s.execute("layer.setProps", json!({"name": "Levels 1"})).unwrap();
    s
}

fn decode_file(dir: &str, name: &str) -> photocraft_codecs::Image {
    photocraft_codecs::decode(&std::fs::read(std::path::Path::new(dir).join(name)).unwrap()).unwrap()
}

#[test]
fn png_with_trim_writes_a_full_canvas_transparent_file_for_the_adjustment() {
    let dir = tmp("png");
    let mut s = session();
    let r = s.execute("file.export.layersToFiles", json!({"dir": dir, "prefix": "doc", "trim": true})).unwrap();
    assert_eq!(r["skipped"], json!([]));
    let files: Vec<&str> = r["files"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    let names: Vec<&str> = files.iter().map(|f| f.rsplit(['/', '\\']).next().unwrap()).collect();
    assert_eq!(names, ["doc_0000_Levels 1.png", "doc_0001_Empty.png", "doc_0002_Background.png"]);
    let img = decode_file(&dir, "doc_0000_Levels 1.png");
    assert_eq!((img.width(), img.height()), (100, 80));
    let rgba = img.to_rgba8();
    assert!(rgba.chunks(4).all(|p| p[3] == 0), "every pixel is fully transparent");
    let empty = decode_file(&dir, "doc_0001_Empty.png");
    assert_eq!((empty.width(), empty.height()), (100, 80));
}

#[test]
fn jpeg_writes_a_white_full_canvas_image_for_the_adjustment() {
    let dir = tmp("jpeg");
    let mut s = session();
    let r = s.execute("file.export.layersToFiles", json!({"dir": dir, "prefix": "doc", "format": "jpg"})).unwrap();
    assert_eq!(r["skipped"], json!([]));
    let img = decode_file(&dir, "doc_0000_Levels 1.jpg");
    assert_eq!((img.width(), img.height()), (100, 80));
    let rgba = img.to_rgba8();
    assert!(rgba.chunks(4).all(|p| p[..3].iter().all(|&c| c >= 253)), "white");
    let empty = decode_file(&dir, "doc_0001_Empty.jpg").to_rgba8();
    assert_eq!(rgba, empty, "the same as the empty layer");
}

#[test]
fn a_hidden_adjustment_is_skipped_by_visible_only() {
    let dir = tmp("hidden");
    let mut s = session();
    let adjustment = s.active().unwrap().doc.layers.last().unwrap().id.0;
    s.execute("layer.setProps", json!({"visible": false})).unwrap();
    let r = s.execute("file.export.layersToFiles", json!({"dir": dir, "prefix": "doc"})).unwrap();
    assert_eq!(r["skipped"], json!([{"layer": adjustment, "name": "Levels 1", "reason": "hidden by visibleOnly"}]));
    assert_eq!(r["files"].as_array().unwrap().len(), 2);
    let all = s.execute("file.export.layersToFiles", json!({"dir": dir, "prefix": "all", "visibleOnly": false})).unwrap();
    assert_eq!(all["skipped"], json!([]));
    assert_eq!(all["files"].as_array().unwrap().len(), 3);
}
