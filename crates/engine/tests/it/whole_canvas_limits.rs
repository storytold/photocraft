//! Commands that render the whole canvas, or merge layers into an area that large, refuse a
//! render past the compositor's per-render limit, as Save a Copy and Duplicate Merged already
//! did, instead of building a composite of hundreds of gigabytes (the allocation aborted the
//! app). The document is left as it was, and a merge of small layers on the same canvas still
//! works.

use photocraft_doc::LayerId;
use photocraft_engine::Session;
use serde_json::{Value, json};

const FLATTEN: &str = "300000×300000 canvas is too large to flatten";
const MERGE: &str = "the merged area (300000×300000 px) is too large to render";

/// The largest canvas `file.new` allows, with one small shape on a layer of its own (active).
fn huge(background: &str) -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 300000, "height": 300000, "resolution": 72, "background": background})).unwrap();
    let r = s.execute("shape.create", json!({"kind": "rect", "rect": [10, 10, 20, 20], "fill": "#e07020"})).unwrap();
    (s, LayerId(r["layer"].as_u64().unwrap()))
}

fn layer_count(s: &Session) -> usize {
    s.active().unwrap().doc.walk().len()
}

fn select(s: &mut Session, ids: &[LayerId]) {
    s.execute("layer.select", json!({"layer": ids[0].0})).unwrap();
    for id in &ids[1..] {
        s.execute("layer.select", json!({"layer": id.0, "mode": "add"})).unwrap();
    }
    assert_eq!(s.active().unwrap().selected_layers.len(), ids.len());
}

fn refused(s: &mut Session, cmd: &str, params: Value, want: &str) {
    let (layers, steps) = (layer_count(s), s.active().unwrap().history.past_len());
    let err = s.execute(cmd, params).expect_err(cmd).to_string();
    assert!(err.contains(want), "{cmd}: {err}");
    assert_eq!(layer_count(s), layers, "{cmd}: the layers are as they were");
    assert_eq!(s.active().unwrap().history.past_len(), steps, "{cmd}: nothing was recorded");
}

#[test]
fn whole_canvas_renders_refuse_a_canvas_past_the_render_limit() {
    for cmd in ["layer.flattenImage", "layer.stampVisible", "layer.stampDown", "edit.copyMerged"] {
        refused(&mut huge("white").0, cmd, json!({}), FLATTEN);
    }
    // Stamp Visible into an active empty pixel layer, which takes the stamp itself.
    let (mut s, _) = huge("white");
    s.execute("layer.new.layer", json!({})).unwrap();
    refused(&mut s, "layer.stampVisible", json!({}), FLATTEN);
    // Stamp Visible with several layers selected stamps only those.
    let (mut s, a) = huge("transparent");
    let b = LayerId(s.execute("shape.create", json!({"kind": "rect", "rect": [30, 30, 40, 40], "fill": "#3060c0"})).unwrap()["layer"].as_u64().unwrap());
    select(&mut s, &[a, b]);
    refused(&mut s, "layer.stampVisible", json!({}), FLATTEN);
    // A layer made into the Background is rendered over the background colour.
    refused(&mut huge("transparent").0, "layer.new.backgroundFromLayer", json!({}), FLATTEN);
    // Copy of a group copies what it shows.
    let (mut s, _) = huge("white");
    s.execute("layer.groupLayers", json!({})).unwrap();
    refused(&mut s, "edit.copy", json!({}), FLATTEN);
}

#[test]
fn merges_refuse_an_area_past_the_render_limit() {
    // The Background spans the whole canvas, so every merge that takes it does too.
    refused(&mut huge("white").0, "layer.mergeVisible", json!({}), MERGE);
    let (mut s, shape) = huge("white");
    refused(&mut s, "layer.mergeDown", json!({"layer": shape.0}), MERGE);
    let (mut s, shape) = huge("white");
    let bg = s.active().unwrap().doc.layers[0].id;
    select(&mut s, &[bg, shape]);
    refused(&mut s, "layer.mergeLayers", json!({}), MERGE);
    // A copy of the Background is an ordinary layer as large as the canvas.
    let (mut s, _) = huge("white");
    let bg = s.active().unwrap().doc.layers[0].id;
    let copy = LayerId(s.execute("layer.duplicate", json!({"layer": bg.0})).unwrap()["layer"].as_u64().unwrap());
    s.execute("layer.select", json!({"layer": copy.0})).unwrap();
    let group = LayerId(s.execute("layer.groupLayers", json!({})).unwrap()["layer"].as_u64().unwrap());
    refused(&mut s, "layer.mergeGroup", json!({"layer": group.0}), MERGE);
}

#[test]
fn small_layers_on_the_largest_canvas_still_merge() {
    let (mut s, a) = huge("transparent");
    let b = LayerId(s.execute("shape.create", json!({"kind": "rect", "rect": [30, 30, 40, 40], "fill": "#3060c0"})).unwrap()["layer"].as_u64().unwrap());
    let before = layer_count(&s);
    s.execute("layer.mergeDown", json!({"layer": b.0})).unwrap();
    assert_eq!(layer_count(&s), before - 1);
    let merged = s.active().unwrap().doc.layer(a).unwrap().surface().unwrap();
    assert!(merged.rgba(15, 15)[3] > 0.99 && merged.rgba(35, 35)[3] > 0.99, "both shapes kept");
}

#[test]
fn save_a_copy_refuses_a_canvas_past_the_render_limit_in_every_format() {
    let dir = tempfile::tempdir().unwrap();
    for ext in ["png", "jpg", "tif", "psb", "pcraft"] {
        let path = dir.path().join(format!("copy.{ext}"));
        let started = std::time::Instant::now();
        refused(&mut huge("white").0, "file.saveACopy", json!({"path": path.to_string_lossy()}), FLATTEN);
        assert!(!path.exists(), "{ext}: nothing written");
        // Refused up front, not after reading the canvas.
        assert!(started.elapsed() < std::time::Duration::from_secs(30), "{ext}: {:?}", started.elapsed());
    }
}
