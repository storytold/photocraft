//! Delete Layer protects explicit Lock All without changing Background or last-layer parity.

use photocraft_doc::LayerId;
use photocraft_engine::Session;
use serde_json::{Value, json};
use std::sync::Arc;

fn document(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4, "height": 4, "depth": depth})).unwrap();
    s
}

fn new_layer(s: &mut Session, name: &str) -> LayerId {
    LayerId(s.execute("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap())
}

fn group(s: &mut Session, id: LayerId) -> LayerId {
    LayerId(s.execute("layer.groupLayers", json!({"layer": id.0})).unwrap()["layer"].as_u64().unwrap())
}

fn select(s: &mut Session, ids: &[LayerId]) {
    for (i, id) in ids.iter().enumerate() {
        s.execute("layer.select", json!({"layer": id.0, "mode": if i == 0 { "replace" } else { "add" }})).unwrap();
    }
}

fn refused(s: &mut Session, params: Value, reason: &str) {
    let st = s.active().unwrap();
    let doc = st.doc.clone();
    let before = (st.revision, st.saved_revision, st.active_layer, st.selected_layers.clone(), st.layer_anchor, st.history.past_len(), st.history.can_redo());
    let error = s.execute("layer.delete", params).unwrap_err().to_string();
    assert!(error.contains(reason), "{error}");
    let st = s.active().unwrap();
    assert!(Arc::ptr_eq(&doc, &st.doc), "a refused deletion changed the document");
    assert_eq!(
        (st.revision, st.saved_revision, st.active_layer, st.selected_layers.clone(), st.layer_anchor, st.history.past_len(), st.history.can_redo()),
        before,
        "a refused deletion changed targeting or history"
    );
}

#[test]
fn background_partial_locks_allow_selected_and_explicit_deletion() {
    for depth in [8, 16, 32] {
        for explicit in [false, true] {
            let mut s = document(depth);
            let background = s.active().unwrap().active_layer.unwrap();
            let locks = s.active().unwrap().doc.layer(background).unwrap().locks;
            assert!(locks.transparency && locks.position && !locks.all);
            let survivor = new_layer(&mut s, "Survivor");
            if !explicit {
                select(&mut s, &[background]);
            }
            let history = s.active().unwrap().history.past_len();
            s.execute("layer.delete", if explicit { json!({"layer": background.0}) } else { json!({}) }).unwrap();
            let st = s.active().unwrap();
            assert!(st.doc.layer(background).is_none());
            assert_eq!(st.active_layer, Some(survivor));
            assert_eq!(st.selected_layers(), vec![survivor]);
            assert_eq!(st.history.past_len(), history + 1);
            s.undo();
            assert_eq!(s.active().unwrap().doc.layer(background).unwrap().locks, locks);
            assert_eq!(s.active().unwrap().active_layer, Some(if explicit { survivor } else { background }));
            s.redo();
            assert!(s.active().unwrap().doc.layer(background).is_none());
            assert_eq!(s.active().unwrap().active_layer, Some(survivor));
        }
    }
}

#[test]
fn background_can_be_part_of_a_multi_delete_with_a_survivor() {
    let mut s = document(8);
    let background = s.active().unwrap().active_layer.unwrap();
    let ordinary = new_layer(&mut s, "Ordinary");
    let survivor = new_layer(&mut s, "Survivor");
    select(&mut s, &[background, ordinary]);
    let history = s.active().unwrap().history.past_len();
    s.execute("layer.delete", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.layer_count(), 1);
    assert_eq!(s.active().unwrap().active_layer, Some(survivor));
    assert_eq!(s.active().unwrap().history.past_len(), history + 1);
    s.undo();
    assert_eq!(s.active().unwrap().doc.layer_count(), 3);
    assert_eq!(s.active().unwrap().selected_layers(), vec![background, ordinary]);
    s.redo();
    assert_eq!(s.active().unwrap().doc.layer_count(), 1);
}

#[test]
fn partial_locks_on_ordinary_layers_allow_deletion() {
    for lock in ["transparency", "pixels", "position"] {
        let mut s = document(8);
        let ordinary = new_layer(&mut s, "Partially locked");
        s.execute("layer.setProps", json!({"locks": {lock: true}})).unwrap();
        s.execute("layer.delete", json!({})).unwrap();
        assert!(s.active().unwrap().doc.layer(ordinary).is_none(), "{lock}");
    }
}

#[test]
fn lock_all_refuses_selected_and_explicit_layer_deletion() {
    for depth in [8, 16, 32] {
        for explicit in [false, true] {
            let mut s = document(depth);
            let locked = new_layer(&mut s, "Protected");
            s.execute("layer.setProps", json!({"locked": true})).unwrap();
            refused(&mut s, if explicit { json!({"layer": locked.0}) } else { json!({}) }, "\"Protected\" is locked");
            s.undo();
            assert!(!s.active().unwrap().doc.layer(locked).unwrap().locks.all, "the refused delete added no undo step");
        }
    }
}

#[test]
fn mixed_locked_selection_is_refused_before_removing_any_layer() {
    for locked_first in [false, true] {
        let mut s = document(8);
        let bottom = new_layer(&mut s, "Bottom");
        let top = new_layer(&mut s, "Top");
        let locked = if locked_first { bottom } else { top };
        select(&mut s, &[top, bottom]);
        s.execute("layer.setProps", json!({"layer": locked.0, "locks": {"all": true}})).unwrap();
        refused(&mut s, json!({}), "is locked");
        assert!(s.active().unwrap().doc.layer(bottom).is_some() && s.active().unwrap().doc.layer(top).is_some());
        s.undo();
        assert!(!s.active().unwrap().doc.layer(locked).unwrap().locks.all);
    }
}

#[test]
fn group_lock_all_protects_the_group_and_its_children() {
    let mut s = document(8);
    let child = new_layer(&mut s, "Child");
    let parent = group(&mut s, child);
    s.execute("layer.setProps", json!({"locked": true})).unwrap();
    refused(&mut s, json!({}), "is locked");
    select(&mut s, &[child]);
    refused(&mut s, json!({}), "\"Child\" is locked");
    refused(&mut s, json!({"layer": parent.0}), "is locked");
    assert!(s.active().unwrap().doc.layer(child).is_some());
}

#[test]
fn a_locked_descendant_protects_an_unlocked_group_and_mixed_selection() {
    for lock_group in [false, true] {
        let mut s = document(8);
        let child = new_layer(&mut s, "Protected child");
        let inner = group(&mut s, child);
        let outer = group(&mut s, inner);
        let other = new_layer(&mut s, "Other");
        s.execute("layer.setProps", json!({"layer": if lock_group { inner.0 } else { child.0 }, "locks": {"all": true}})).unwrap();
        select(&mut s, &[child]);
        refused(&mut s, json!({"layer": outer.0}), "is locked");
        select(&mut s, &[outer]);
        refused(&mut s, json!({}), "is locked");
        select(&mut s, &[other, outer, child]);
        refused(&mut s, json!({}), "is locked");
        assert!(s.active().unwrap().doc.layer(other).is_some());
    }
}

#[test]
fn deleting_a_group_retargets_its_active_child_and_round_trips_history() {
    for multi in [false, true] {
        let mut s = document(8);
        let survivor = s.active().unwrap().active_layer.unwrap();
        let child = new_layer(&mut s, "Child");
        let parent = group(&mut s, child);
        let targets = [parent, child];
        select(&mut s, if multi { &targets } else { std::slice::from_ref(&child) });
        let before = s.active().unwrap().selected_layers();
        let history = s.active().unwrap().history.past_len();
        s.execute("layer.delete", if multi { json!({}) } else { json!({"layer": parent.0}) }).unwrap();
        let st = s.active().unwrap();
        assert!(st.doc.layer(parent).is_none() && st.doc.layer(child).is_none());
        assert_eq!(st.active_layer, Some(survivor));
        assert_eq!(st.selected_layers(), vec![survivor]);
        assert_eq!(st.history.past_len(), history + 1);
        s.undo();
        assert!(s.active().unwrap().doc.layer(parent).is_some() && s.active().unwrap().doc.layer(child).is_some());
        assert_eq!(s.active().unwrap().active_layer, Some(child));
        assert_eq!(s.active().unwrap().selected_layers(), before);
        s.redo();
        assert_eq!(s.active().unwrap().active_layer, Some(survivor));
    }
}

#[test]
fn deleting_every_selected_layer_still_refuses_an_empty_document() {
    for depth in [8, 16, 32] {
        let mut s = document(depth);
        let background = s.active().unwrap().active_layer.unwrap();
        let other = new_layer(&mut s, "Other");
        select(&mut s, &[background, other]);
        refused(&mut s, json!({}), "a document must keep at least one layer");
    }
}
