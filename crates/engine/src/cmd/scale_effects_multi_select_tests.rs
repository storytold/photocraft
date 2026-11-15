//! Scale Effects with several layers selected, measured against the reference application
//! (#14993).
//!
//! The reference greys Layer > Layer Style > Scale Effects... while two or more layers are
//! selected, and never scales a multi-selection (neither all of them nor just the active one).
//! One selected layer with effects, a pixel layer or a group, keeps it enabled; a layer without
//! effects, or no document, keeps it greyed. A call that names its `layer` (scripts, agents) is
//! not measured and keeps today's behaviour.

use photocraft_doc::{Effect, LayerId};

use super::*;

const CMD: &str = "layer.layerStyle.scaleEffects";

/// A white document with the pixel layers "Plain" (no effects) and, above it, "Shape" with a
/// Drop Shadow of distance 10 and size 4; "Shape" is active and the only selected layer.
fn plain_and_shape() -> (Session, LayerId, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Plain"})).unwrap();
    let plain = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.new.layer", json!({"name": "Shape"})).unwrap();
    let shape = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.layerStyle.dropShadow", json!({"distance": 10, "size": 4})).unwrap();
    (s, plain, shape)
}

/// The Drop Shadow's (distance, size) on layer `id`.
fn shadow(s: &Session, id: LayerId) -> (f32, f32) {
    let l = s.active().unwrap().doc.layer(id).unwrap();
    match l.effects.items.first() {
        Some(Effect::DropShadow(sh)) => (sh.distance, sh.size),
        e => panic!("no drop shadow: {e:?}"),
    }
}

#[test]
fn two_selected_layers_grey_scale_effects_out() {
    let (mut s, plain, shape) = plain_and_shape();
    s.execute("layer.select", json!({"layer": plain.0, "mode": "add"})).unwrap();
    s.execute("layer.select", json!({"layer": shape.0, "mode": "add"})).unwrap();
    assert_eq!(s.active().unwrap().selected_layers().len(), 2);
    assert!(!s.is_enabled(CMD), "Scale Effects must be greyed with two layers selected");
}

#[test]
fn a_run_without_a_layer_on_two_selected_layers_scales_nothing() {
    let (mut s, plain, shape) = plain_and_shape();
    s.execute("layer.select", json!({"layer": plain.0, "mode": "add"})).unwrap();
    s.execute("layer.select", json!({"layer": shape.0, "mode": "add"})).unwrap();
    let undo_before = s.active().unwrap().history.entries();
    assert!(s.execute(CMD, json!({"scale": 200})).is_err());
    assert_eq!(shadow(&s, shape), (10.0, 4.0), "the active layer's effects must stay as they were");
    assert_eq!(s.active().unwrap().history.entries(), undo_before, "no history state");
}

#[test]
fn one_selected_layer_with_effects_stays_enabled_and_scales() {
    let (mut s, _, shape) = plain_and_shape();
    assert!(s.is_enabled(CMD));
    s.execute(CMD, json!({"scale": 200})).unwrap();
    assert_eq!(shadow(&s, shape), (20.0, 8.0));
}

#[test]
fn a_selected_group_with_effects_stays_enabled() {
    let (mut s, _, _) = plain_and_shape();
    s.execute("layer.new.group", json!({"name": "Group"})).unwrap();
    let group = s.active().unwrap().active_layer.unwrap();
    assert!(!s.is_enabled(CMD), "a group without effects is greyed");
    s.execute("layer.layerStyle.dropShadow", json!({"distance": 10, "size": 4})).unwrap();
    assert!(s.is_enabled(CMD));
    s.execute(CMD, json!({"scale": 50})).unwrap();
    assert_eq!(shadow(&s, group), (5.0, 2.0));
}

#[test]
fn no_effects_or_no_document_stay_greyed() {
    let mut s = Session::new();
    assert!(!s.is_enabled(CMD), "no document");
    let (mut s2, plain, _) = plain_and_shape();
    s2.execute("layer.select", json!({"layer": plain.0})).unwrap();
    assert!(!s2.is_enabled(CMD), "a single layer without effects");
    assert!(s.execute(CMD, json!({})).is_err());
}

#[test]
fn a_call_naming_its_layer_keeps_todays_behaviour() {
    let (mut s, plain, shape) = plain_and_shape();
    s.execute("layer.select", json!({"layer": plain.0, "mode": "add"})).unwrap();
    s.execute("layer.select", json!({"layer": shape.0, "mode": "add"})).unwrap();
    s.execute(CMD, json!({"scale": 200, "layer": shape.0})).unwrap();
    assert_eq!(shadow(&s, shape), (20.0, 8.0));
}
