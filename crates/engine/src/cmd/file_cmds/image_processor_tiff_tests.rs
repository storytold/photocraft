//! #13898: Image Processor's TIFF results are `.tif` files that keep their layers, with the
//! composite on white and no alpha channel; explicit TIFF options and other formats keep today's.
use serde_json::json;

use crate::Session;

fn dir(name: &str) -> tempfile::TempDir {
    tempfile::Builder::new().prefix(&format!("photocraft-ip-tiff-{name}-")).tempdir().unwrap()
}

/// A source folder holding `a-rgb8.psd` (400 × 200, a "Red" layer over a white Background) and
/// `t-alpha.png` (120 × 80, green in the left half, the right half transparent).
fn sources() -> tempfile::TempDir {
    let src = dir("src");
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 200, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Red"})).unwrap();
    s.execute("select.rect", json!({"x": 20, "y": 20, "width": 100, "height": 60})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("file.saveACopy", json!({"path": src.path().join("a-rgb8.psd").to_string_lossy()})).unwrap();
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 120, "height": 80, "background": "transparent"})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 60, "height": 80})).unwrap();
    s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("file.saveACopy", json!({"path": src.path().join("t-alpha.png").to_string_lossy()})).unwrap();
    src
}

fn names(out: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(out).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

fn layers(path: &std::path::Path) -> Vec<String> {
    let bytes = std::fs::read(path).unwrap();
    let doc = photocraft_io::import(&path.to_string_lossy(), &bytes).unwrap().document;
    doc.walk().into_iter().map(|(_, _, l)| l.name.clone()).collect()
}

fn alpha_and_right_pixel(path: &std::path::Path) -> (bool, [u8; 4]) {
    let img = photocraft_codecs::decode(&std::fs::read(path).unwrap()).unwrap();
    let alpha = img.layout().has_alpha();
    let rgba = img.to_rgba8();
    let i = (40 * 120 + 100) * 4;
    (alpha, [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]])
}

#[test]
fn tiff_results_are_tif_files_with_their_layers_on_white() {
    let src = sources();
    let out = dir("out");
    let mut s = Session::new();
    let r = s
        .execute("file.scripts.imageProcessor", json!({"input": src.path().to_string_lossy(), "output": out.path().to_string_lossy(), "format": "tiff"}))
        .unwrap();
    assert!(r["errors"].as_array().is_none_or(Vec::is_empty), "{r}");
    assert_eq!(names(out.path()), ["a-rgb8.tif", "t-alpha.tif"], ".tif, not .tiff");
    let l = layers(&out.path().join("a-rgb8.tif"));
    assert!(l.contains(&"Red".to_string()) && l.contains(&"Background".to_string()), "layers kept: {l:?}");
    let (alpha, right) = alpha_and_right_pixel(&out.path().join("t-alpha.tif"));
    assert!(!alpha, "no alpha channel");
    assert_eq!(right, [255, 255, 255, 255], "the transparent half composited on white");
}

#[test]
fn the_three_formats_run_writes_its_tiff_on_white_too() {
    let src = sources();
    let out = dir("out3");
    let mut s = Session::new();
    s.execute("file.scripts.imageProcessor", json!({"input": src.path().to_string_lossy(), "output": out.path().to_string_lossy(), "tiff": {}})).unwrap();
    let (alpha, right) = alpha_and_right_pixel(&out.path().join("t-alpha.tif"));
    assert!(!alpha);
    assert_eq!(right, [255, 255, 255, 255]);
    let l = layers(&out.path().join("a-rgb8.tif"));
    assert!(l.contains(&"Red".to_string()), "{l:?}");
}

#[test]
fn explicit_tiff_options_and_other_formats_keep_todays_behaviour() {
    let src = sources();
    let mut s = Session::new();
    let out = dir("flat");
    s.execute(
        "file.scripts.imageProcessor",
        json!({"input": src.path().to_string_lossy(), "output": out.path().to_string_lossy(), "format": "tif", "tiffLayers": false, "tiffTransparency": true}),
    )
    .unwrap();
    assert_eq!(layers(&out.path().join("a-rgb8.tif")).len(), 1, "flat when asked");
    assert!(alpha_and_right_pixel(&out.path().join("t-alpha.tif")).0, "transparency kept when asked");
    let out = dir("png");
    s.execute("file.scripts.imageProcessor", json!({"input": src.path().to_string_lossy(), "output": out.path().to_string_lossy(), "format": "png"})).unwrap();
    assert_eq!(names(out.path()), ["a-rgb8.png", "t-alpha.png"]);
    assert!(alpha_and_right_pixel(&out.path().join("t-alpha.png")).0, "a PNG keeps its transparency");
}
