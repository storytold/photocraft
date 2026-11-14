//! Layer from Background on a hidden Background (#14822): the reference application makes the new
//! layer visible, and one undo brings the hidden Background back. Implicit conversions (an edit the
//! Background can't take) keep its visibility.

use super::*;

/// White Background (hidden when `hidden`) plus one normal layer above it.
fn session(hidden: bool) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let bg = background(&s).id;
    s.execute("layer.setProps", json!({"layer": bg.0, "visible": !hidden})).unwrap();
    s
}

fn background(s: &Session) -> &Layer {
    &s.active().unwrap().doc.layers[0]
}

#[test]
fn converting_a_hidden_background_makes_the_layer_visible() {
    let mut s = session(true);
    assert!(!background(&s).visible);
    let h = s.active().unwrap().history.past_len();
    s.execute("layer.new.layerFromBackground", json!({})).unwrap();
    let l = background(&s);
    assert_eq!(l.name, "Layer 0");
    assert!(l.visible, "the converted layer is visible");
    assert_eq!(s.active().unwrap().history.past_len(), h + 1, "one History step");

    assert!(s.undo());
    let l = background(&s);
    assert_eq!(l.name, "Background");
    assert!(!l.visible, "undo brings back the hidden Background");
    assert!(l.locks.transparency && l.locks.position, "with its locks");
}

#[test]
fn converting_a_visible_background_keeps_it_visible() {
    let mut s = session(false);
    s.execute("layer.new.layerFromBackground", json!({})).unwrap();
    assert!(background(&s).visible);
}

#[test]
fn an_implicit_conversion_keeps_a_hidden_background_hidden() {
    let mut s = session(true);
    let bg = background(&s).id;
    // A layer mask is something the Background can't take, so it becomes "Layer 0" first.
    s.execute("layer.layerMask.revealAll", json!({"layer": bg.0})).unwrap();
    let l = background(&s);
    assert_eq!(l.name, "Layer 0");
    assert!(!l.visible);
}
