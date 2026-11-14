//! #14822: Layer > New > Layer from Background makes the converted layer **visible**. A Background
//! hidden with `layer.setProps {"visible":false}` stays hidden today, because the shared
//! `unlock_background` only renames the layer and clears its transparency and position locks.
//!
//! The change belongs in `layer_from_background`, not in `unlock_background` /
//! `background_to_layer`: whether the implicit conversions (moving, erasing, transforming the
//! Background, adding a mask) also make a hidden Background visible is *not measured* in the issue,
//! so those callers keep today's behaviour and the last test below pins it.

use photocraft_doc::LayerId;
use serde_json::{Value, json};

use crate::Session;

fn doc(s: &Session) -> &photocraft_doc::Document {
    &s.active().unwrap().doc
}

/// A white Background, hidden, with one more layer above it: a Background that is the only layer
/// cannot be hidden at all.
fn hidden_background() -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 12})).unwrap();
    let bg = doc(&s).layers[0].id;
    s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
    s.execute("layer.setProps", json!({"layer": bg.0, "visible": false})).unwrap();
    let l = doc(&s).layer(bg).unwrap();
    assert!(doc(&s).is_background(bg) && !l.visible, "the Background is hidden and still the Background");
    (s, bg)
}

#[test]
fn a_hidden_background_becomes_a_visible_layer() {
    let (mut s, bg) = hidden_background();
    let v: Value = s.execute("layer.new.layerFromBackground", json!({})).unwrap();
    assert_eq!(v["layer"], bg.0);
    assert!(!doc(&s).is_background(bg), "the Background is a normal layer now");
    let l = doc(&s).layer(bg).unwrap();
    assert!(l.visible, "and the converted layer is visible");
    assert!(!l.locks.transparency && !l.locks.position, "unlocked, as before");
}

#[test]
fn one_undo_step_brings_back_the_hidden_background() {
    let (mut s, bg) = hidden_background();
    s.execute("layer.new.layerFromBackground", json!({})).unwrap();
    assert!(doc(&s).layer(bg).unwrap().visible, "visible after the command");
    assert!(s.undo(), "one step back");
    let l = doc(&s).layer(bg).unwrap();
    assert!(doc(&s).is_background(bg), "the Background is back");
    assert!(!l.visible, "hidden, as it was");
    assert!(l.locks.transparency && l.locks.position, "locked, as it was");
    assert_eq!(l.name, "Background", "with its old name");
    assert!(s.redo(), "and redo converts it again");
    assert!(!doc(&s).is_background(bg) && doc(&s).layer(bg).unwrap().visible, "as a visible layer");
}

#[test]
fn a_visible_background_still_converts_to_a_visible_layer() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 12})).unwrap();
    let bg = doc(&s).layers[0].id;
    s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
    assert!(doc(&s).layer(bg).unwrap().visible, "the Background starts visible");
    s.execute("layer.new.layerFromBackground", json!({})).unwrap();
    assert!(!doc(&s).is_background(bg) && doc(&s).layer(bg).unwrap().visible, "and converts to a visible layer");
}

#[test]
fn an_implicit_conversion_keeps_the_backgrounds_visibility() {
    // Not measured in #14822, so these callers of `background_to_layer` keep today's behaviour:
    // an edit the Background cannot take converts it and leaves it hidden.
    for (key, value) in [("opacity", json!(0.5)), ("fill", json!(0.5)), ("blend", json!("Multiply"))] {
        let (mut s, bg) = hidden_background();
        s.execute("layer.setProps", json!({"layer": bg.0, key: value})).unwrap();
        assert!(!doc(&s).is_background(bg), "{key}: the Background became a normal layer");
        assert!(!doc(&s).layer(bg).unwrap().visible, "{key}: it stayed hidden as today");
    }
}
