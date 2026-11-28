//! Commands that render the whole canvas, or merge layers into an area that large, refuse a
//! composite too large to hold (a new layer past the budget for one, flat working buffers past
//! what Image Size resamples within, or past the compositor's per-render limit for Indexed
//! Color) instead of building one of hundreds of gigabytes (the allocation aborted the
//! app). The document is left as it was, and a merge of small layers on the same canvas still
//! works.

use photocraft_doc::LayerId;
use photocraft_engine::Session;
use serde_json::{Value, json};

/// The refusal names the composite's size.
const FLATTEN: &str = "of working memory for a 300000 x 300000 px area";
const MERGE: &str = "Merging the layers would";

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
fn whole_canvas_renders_refuse_a_canvas_past_the_budget() {
    for cmd in ["layer.flattenImage", "layer.stampVisible", "layer.stampDown", "edit.copyMerged"] {
        refused(&mut huge("white").0, cmd, json!({}), FLATTEN);
    }
    // Stamp Visible into an active empty pixel layer, which takes the stamp itself.
    let (mut s, _) = huge("white");
    s.execute("layer.new.layer", json!({})).unwrap();
    refused(&mut s, "layer.stampVisible", json!({}), FLATTEN);
    // Stamp Down with several layers selected stamps only those, as a new layer.
    let (mut s, a) = huge("transparent");
    let b = LayerId(s.execute("shape.create", json!({"kind": "rect", "rect": [30, 30, 40, 40], "fill": "#3060c0"})).unwrap()["layer"].as_u64().unwrap());
    select(&mut s, &[a, b]);
    refused(&mut s, "layer.stampDown", json!({}), "Stamp Down would need");
    // A layer made into the Background is rendered over the background colour.
    refused(&mut huge("transparent").0, "layer.new.backgroundFromLayer", json!({}), FLATTEN);
    // Copy of a group copies what it shows.
    let (mut s, _) = huge("white");
    s.execute("layer.groupLayers", json!({})).unwrap();
    refused(&mut s, "edit.copy", json!({}), FLATTEN);
}

#[test]
fn merges_refuse_an_area_past_the_budget() {
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

/// Within the compositor's limit but past the working-memory budget at four floats a pixel,
/// Indexed Color is refused before it flattens, and the document is as it was.
#[test]
fn indexed_color_refuses_a_flatten_past_the_memory_budget() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 30000, "height": 30000, "resolution": 72, "background": "white"})).unwrap();
    refused(&mut s, "image.mode.indexedColor", json!({"palette": "exact", "forced": "none", "dither": "none", "transparency": false}), "working memory");
}

/// Indexed Color flattens the whole canvas: past the compositor's limit it is refused (the
/// empty flatten used to reach a canvas-sized write and panic). Match Color's statistics of a
/// source and printing read the whole canvas too, and are refused past the budget.
#[test]
fn indexed_color_match_color_and_print_refuse_the_largest_canvas() {
    let (mut s, _) = huge("white");
    refused(&mut s, "image.mode.indexedColor", json!({"palette": "exact", "forced": "none", "dither": "none", "transparency": false}), "too large to flatten");
    // The huge document as Match Color's source, applied to a small one.
    s.execute("file.new", json!({"width": 8, "height": 8, "resolution": 72})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#808080"})).unwrap();
    refused(&mut s, "image.adjustments.matchColor", json!({"source": 0}), "reading the image's colours would need");
    let (mut s, _) = huge("white");
    let out = tempfile::tempdir().unwrap();
    let out = out.path().join("print.pdf");
    refused(&mut s, "file.print", json!({"output": out.to_string_lossy(), "scaleToFit": true}), "printing would need");
    assert!(!out.exists());
}

/// Two layers far apart on a small canvas: merging them would render an area past the
/// compositor's per-render limit, though within the memory budget. It renders in bands the
/// area's full width, and a band past that limit renders as nothing, so the merge would lose
/// pixels: it is refused, and the layers stay as they were.
#[test]
fn a_merge_of_layers_far_apart_is_refused_rather_than_rendered_empty() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 300, "resolution": 72, "background": "transparent"})).unwrap();
    let mut ids = Vec::new();
    for (x, dx) in [(0, -2_300_000), (40, 2_300_000)] {
        let id = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
        s.execute("select.rect", json!({"x": x, "y": 0, "width": 10, "height": 300})).unwrap();
        s.execute("edit.fill", json!({"color": "#e07020"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("layer.translate", json!({"layer": id.0, "dx": dx, "dy": 0})).unwrap();
        let doc = &s.active().unwrap().doc;
        assert!(!photocraft_engine::transform_cmds::transform_bounds(doc, doc.layer(id).unwrap()).is_empty(), "the layer holds its pixels out there");
        ids.push(id);
    }
    select(&mut s, &ids);
    refused(&mut s, "layer.mergeLayers", json!({}), "in bands past the limit of 1073741824 pixels");
    s.execute("layer.select", json!({"layer": ids[1].0})).unwrap();
    refused(&mut s, "layer.mergeDown", json!({"layer": ids[1].0}), "in bands past the limit of 1073741824 pixels");
}
