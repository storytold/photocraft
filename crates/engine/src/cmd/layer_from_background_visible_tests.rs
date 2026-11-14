//! Layer > New > Layer from Background makes a hidden Background a visible layer (#14822), undo
//! brings the hidden Background back in one step, and the implicit conversions
//! (`background_to_layer`) keep the layer's visibility as before.

use crate::Session;
use photocraft_doc::{Layer, LayerId};
use serde_json::json;

/// A 32 x 32 document with a white Background and one pixel layer above it; returns the
/// Background's id.
fn session() -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 32, "height": 32, "background": "white"})).unwrap();
    let bg = s.active().unwrap().doc.background().unwrap().id;
    s.execute("layer.new.layer", json!({})).unwrap();
    (s, bg)
}

fn layer(s: &Session, id: LayerId) -> &Layer {
    s.active().unwrap().doc.layer(id).unwrap()
}

fn hide(s: &mut Session, id: LayerId) {
    s.execute("layer.setProps", json!({"layer": id.0, "visible": false})).unwrap();
    assert!(!layer(s, id).visible, "the Background is hidden");
}

#[test]
fn a_hidden_background_becomes_a_visible_layer() {
    let (mut s, bg) = session();
    hide(&mut s, bg);
    let out = s.execute("layer.new.layerFromBackground", json!({})).unwrap();
    assert_eq!(out["layer"].as_u64(), Some(bg.0), "the same layer is converted");
    let l = layer(&s, bg);
    assert!(l.visible, "the converted layer is visible");
    assert!(s.active().unwrap().doc.background().is_none(), "no Background is left");
}

#[test]
fn undo_restores_the_hidden_background_in_one_step() {
    let (mut s, bg) = session();
    hide(&mut s, bg);
    let steps = s.active().unwrap().history.past_len();
    let (name, locks) = (layer(&s, bg).name.clone(), layer(&s, bg).locks);
    s.execute("layer.new.layerFromBackground", json!({})).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "one history state");
    s.execute("history.undo", json!({})).unwrap();
    let l = layer(&s, bg);
    assert!(!l.visible, "the Background is hidden again");
    assert_eq!(l.name, name, "its name is back");
    assert_eq!(l.locks, locks, "its locks are back");
    assert_eq!(s.active().unwrap().doc.background().map(|l| l.id), Some(bg), "it is the Background again");
}

#[test]
fn a_visible_background_still_converts_to_a_visible_layer() {
    let (mut s, bg) = session();
    s.execute("layer.new.layerFromBackground", json!({})).unwrap();
    assert!(layer(&s, bg).visible);
}

/// Implicit conversions (moving, erasing, transforming the Background, adding a mask) were not
/// measured, so they keep the layer's visibility.
#[test]
fn implicit_conversions_keep_a_hidden_background_hidden() {
    let (mut s, bg) = session();
    hide(&mut s, bg);
    s.edit("implicit", |doc, _| {
        crate::extra_cmds::background_to_layer(doc, bg);
        Ok(())
    })
    .unwrap();
    assert!(s.active().unwrap().doc.background().is_none(), "the Background was converted");
    assert!(!layer(&s, bg).visible, "it stays hidden");
}
