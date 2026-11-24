//! Tests that run paint commands, moved from the engine-core `analysis_cmds` tests: 1 test(s).

use photocraft_doc::Document;
use photocraft_engine::Session;
use serde_json::json;
use std::sync::Arc;

fn session(w: u32, h: u32, depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "depth": depth, "background": "white"})).unwrap();
    s
}

fn d(s: &Session) -> Arc<Document> {
    s.active().unwrap().doc.clone()
}

#[test]
fn straighten_layer_and_image() {
    // A layer: rotated about the canvas centre, canvas unchanged.
    let mut s = session(80, 60, 8);
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 10, "y": 28, "width": 60, "height": 4})).unwrap();
    s.execute("edit.fill", json!({"color": "#000000"})).unwrap_or_default();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("image.analysis.rulerTool", json!({"start": [10, 30], "end": [70, 25]})).unwrap();
    let before = s.active().unwrap().history.past_len();
    let r = s.execute("image.analysis.straightenLayer", json!({})).unwrap();
    assert_eq!(r["cropped"], false);
    assert!((r["angle"].as_f64().unwrap() - 4.7636).abs() < 1e-3, "{r}");
    assert_eq!(s.active().unwrap().history.past_len(), before + 1, "one history step");
    assert_eq!(d(&s).size.width, 80);
    assert!(d(&s).measurement.ruler.is_none());
    // The Background used to rotate the canvas and crop it to the image; #12033 greys that state
    // out, so the same crop now happens on a layer and only when it is asked for.
    let mut s = session(80, 60, 16);
    s.execute("image.analysis.rulerTool", json!({"start": [0, 0], "end": [40, 40]})).unwrap();
    assert!(!s.is_enabled("image.analysis.straightenLayer"), "the Background is greyed out (#12033)");
    s.execute("layer.new.layer", json!({})).unwrap();
    let r = s.execute("image.analysis.straightenLayer", json!({"crop": true})).unwrap();
    assert_eq!(r["cropped"], true);
    assert_eq!(r["angle"].as_f64().unwrap().abs(), 45.0);
    let nd = d(&s);
    assert!(nd.size.width < 80 && nd.size.height < 60);
    assert!(s.undo());
    assert_eq!(d(&s).size.width, 80);
}
