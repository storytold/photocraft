//! The Art History Brush's refusals use the reference application's alert texts (#12638): a
//! hidden target, Lock All or Lock Image Pixels, a group, a target layer the source state lacks,
//! another canvas size and no source. Each adds no history state and changes no pixel, and the
//! History Brush keeps its own texts.

use serde_json::{Value, json};

use super::{CANVAS_SIZE, GROUP, LACKS_LAYER, LOCKED, NO_SOURCE};
use crate::Session;
use crate::hidden_target::{ART_HISTORY_HIDDEN, HIDDEN};

/// An RGB document with a red "Layer 1" above the white Background, the layer active. History
/// rows: 0 Open, 1 New Layer, 2 Fill.
fn session() -> (Session, u64) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 48, "height": 32, "background": "white"})).unwrap();
    let id = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    (s, id)
}

fn stroke(cmd: &str) -> Value {
    let mut p = json!({"points": [[8, 16], [40, 16]], "size": 12, "hardness": 100});
    if cmd == "paint.artHistoryBrush" {
        p["style"] = json!("tightMedium");
    }
    p
}

/// Runs `cmd` with `extra` params, expects a refusal and returns its text, checking that the
/// document and its history are unchanged.
fn refused(s: &mut Session, cmd: &str, extra: Value) -> String {
    let mut p = stroke(cmd);
    if let Some(map) = extra.as_object() {
        for (k, v) in map {
            p[k] = v.clone();
        }
    }
    let (doc, steps) = (s.active().unwrap().doc.clone(), s.active().unwrap().history.entries().len());
    let e = s.execute(cmd, p).unwrap_err().to_string();
    let st = s.active().unwrap();
    assert_eq!(st.history.entries().len(), steps, "{cmd}: no history state for {e}");
    for l in &doc.layers {
        assert_eq!(st.doc.layer(l.id).and_then(|n| n.surface()), l.surface(), "{cmd}: no pixel changes for {e}");
    }
    e
}

#[test]
fn a_hidden_target_layer() {
    let (mut s, id) = session();
    s.execute("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
    assert_eq!(refused(&mut s, "paint.artHistoryBrush", json!({})), ART_HISTORY_HIDDEN);
    assert_eq!(refused(&mut s, "paint.historyBrush", json!({})), HIDDEN);
}

#[test]
fn lock_all_or_lock_image_pixels() {
    for key in ["all", "pixels"] {
        let (mut s, id) = session();
        s.execute("layer.setProps", json!({"layer": id, "locks": {key: true}})).unwrap();
        assert_eq!(refused(&mut s, "paint.artHistoryBrush", json!({})), LOCKED, "{key}");
        // Row 1 holds Layer 1, so the History Brush gets as far as the lock.
        let e = refused(&mut s, "paint.historyBrush", json!({"state": 1}));
        assert!(e.contains("Could not complete your request because the layer \"Layer 1\" is locked"), "{key}: {e}");
    }
}

/// Lock Transparent Pixels was not measured: it still paints, as before.
#[test]
fn lock_transparent_pixels_still_paints() {
    let (mut s, id) = session();
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    s.execute("layer.setProps", json!({"layer": id, "locks": {"transparency": true}})).unwrap();
    s.execute("paint.artHistoryBrush", json!({"points": [[8, 16], [40, 16]], "size": 12, "hardness": 100, "state": 2})).unwrap();
}

#[test]
fn a_group_is_active() {
    let (mut s, _) = session();
    s.execute("layer.groupLayers", json!({})).unwrap();
    let e = refused(&mut s, "paint.artHistoryBrush", json!({}));
    assert!(e.contains(GROUP), "{e}");
    assert!(!s.is_enabled("paint.artHistoryBrush"));
    assert_eq!(s.disabled_reason("paint.artHistoryBrush").as_deref(), Some(GROUP));
    let e = refused(&mut s, "paint.historyBrush", json!({}));
    assert!(e.contains("not a pixel layer") && !e.contains("art history"), "{e}");
}

/// Row 1 (New Layer) holds the Background and Layer 1; a second layer added after it is missing.
#[test]
fn the_source_state_lacks_the_target_layer() {
    let (mut s, _) = session();
    s.execute("layer.new.layer", json!({})).unwrap();
    assert_eq!(refused(&mut s, "paint.artHistoryBrush", json!({"state": 1})), LACKS_LAYER);
    // The History panel's source too.
    s.execute("history.setSource", json!({"state": 1})).unwrap();
    assert_eq!(refused(&mut s, "paint.artHistoryBrush", json!({})), LACKS_LAYER);
    let e = refused(&mut s, "paint.historyBrush", json!({}));
    assert!(e.contains("the target did not exist"), "{e}");
}

#[test]
fn the_canvas_size_changed() {
    let (mut s, _) = session();
    s.execute("image.canvasSize", json!({"width": 60, "height": 32, "anchor": "topLeft"})).unwrap();
    assert_eq!(refused(&mut s, "paint.artHistoryBrush", json!({})), CANVAS_SIZE);
    assert_eq!(refused(&mut s, "paint.artHistoryBrush", json!({"state": 1})), CANVAS_SIZE);
    let e = refused(&mut s, "paint.historyBrush", json!({}));
    assert!(e.contains("does not match that of the History Brush source (48×32)"), "{e}");
}

#[test]
fn no_source_is_set() {
    let (mut s, _) = session();
    let snap = s.execute("history.newSnapshot", json!({})).unwrap()["snapshot"].clone();
    s.execute("history.setSource", json!({"snapshot": snap})).unwrap();
    s.execute("history.deleteSnapshot", json!({"snapshot": snap})).unwrap();
    assert_eq!(refused(&mut s, "paint.artHistoryBrush", json!({})), NO_SOURCE);
    let e = refused(&mut s, "paint.historyBrush", json!({}));
    assert!(e.contains("the History Brush source was deleted"), "{e}");
}

/// No source and another canvas size: the missing source is reported, as before.
#[test]
fn no_source_comes_before_the_canvas_size() {
    let (mut s, _) = session();
    let snap = s.execute("history.newSnapshot", json!({})).unwrap()["snapshot"].clone();
    s.execute("history.setSource", json!({"snapshot": snap})).unwrap();
    s.execute("history.deleteSnapshot", json!({"snapshot": snap})).unwrap();
    s.execute("image.canvasSize", json!({"width": 60, "height": 32, "anchor": "topLeft"})).unwrap();
    assert_eq!(refused(&mut s, "paint.artHistoryBrush", json!({})), NO_SOURCE);
}

/// The checks don't stop a stroke that may paint: row 2 is the red layer, now blue.
#[test]
fn a_valid_stroke_still_paints() {
    let (mut s, _) = session();
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    let steps = s.active().unwrap().history.entries().len();
    s.execute("paint.artHistoryBrush", json!({"points": [[8, 16], [40, 16]], "size": 12, "hardness": 100, "state": 2})).unwrap();
    assert_eq!(s.active().unwrap().history.entries().len(), steps + 1);
}

/// A group with a mask passes the precondition (for its mask), but its pixels are still refused.
#[test]
fn a_group_with_a_mask_and_its_pixels_targeted() {
    let (mut s, _) = session();
    s.execute("layer.groupLayers", json!({})).unwrap();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    assert_eq!(refused(&mut s, "paint.artHistoryBrush", json!({"target": "pixels"})), GROUP);
}

/// A mask target was not measured: a hidden layer, no source and another canvas size keep the
/// History Brush's texts.
#[test]
fn a_mask_target_keeps_todays_texts() {
    let (mut s, id) = session();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    let mask = json!({"target": "mask"});
    s.execute("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
    assert_eq!(refused(&mut s, "paint.artHistoryBrush", mask.clone()), HIDDEN);
    s.execute("layer.setProps", json!({"layer": id, "visible": true})).unwrap();
    s.execute("image.canvasSize", json!({"width": 60, "height": 32, "anchor": "topLeft"})).unwrap();
    let e = refused(&mut s, "paint.artHistoryBrush", mask.clone());
    assert!(e.contains("does not match that of the History Brush source"), "{e}");
    let snap = s.execute("history.newSnapshot", json!({})).unwrap()["snapshot"].clone();
    s.execute("history.setSource", json!({"snapshot": snap})).unwrap();
    s.execute("history.deleteSnapshot", json!({"snapshot": snap})).unwrap();
    let e = refused(&mut s, "paint.artHistoryBrush", mask);
    assert!(e.contains("the History Brush source was deleted"), "{e}");
}
