//! Tests that run paint commands, moved from the engine-core `dense_budget` tests: 1 test(s).

use serde_json::json;

use photocraft_engine::Session;

#[test]
fn face_detection_on_a_huge_empty_canvas_is_refused() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 300_000, "height": 300_000, "background": "transparent"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let e = s.execute("image.detectFaces", json!({"sampleAllLayers": false})).unwrap_err().to_string();
    assert!(e.contains("working memory"), "{e}");
}
