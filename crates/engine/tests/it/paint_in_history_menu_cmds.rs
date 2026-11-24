//! Tests that run paint commands, moved from the engine-core `history_menu_cmds` tests: 6 test(s).

use photocraft_engine::Session;
use serde_json::json;

use std::sync::Arc;

/// A document with three steps after Open: entries Open, New Layer, Fill, Fill.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 20, "height": 10})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
    s
}

fn entries(s: &Session) -> Vec<String> {
    s.active().unwrap().history.entries()
}

#[test]
fn step_backward_and_forward_move_one_state() {
    let mut s = session();
    let all = entries(&s);
    let last = s.active().unwrap().doc.clone();
    assert!(!s.is_enabled("history.stepForward"), "no later state yet");
    assert!(s.is_enabled("history.stepBackward"));
    let r = s.execute("history.stepBackward", json!({})).unwrap();
    assert_eq!((r["states"].as_u64(), r["current"].as_u64()), (Some(all.len() as u64), Some(all.len() as u64 - 2)));
    assert_eq!(r["label"], json!(all[all.len() - 2]));
    assert_eq!(entries(&s), all[..all.len() - 1].to_vec());
    assert!(s.is_enabled("history.stepForward"));
    s.execute("history.stepBackward", json!({})).unwrap();
    s.execute("history.stepForward", json!({})).unwrap();
    let r = s.execute("history.stepForward", json!({})).unwrap();
    assert_eq!(r["current"].as_u64(), Some(all.len() as u64 - 1));
    assert_eq!(entries(&s), all);
    assert!(Arc::ptr_eq(&s.active().unwrap().doc, &last), "back at the last state");
    assert!(s.execute("history.stepForward", json!({})).is_err(), "nothing to step forward to");
}

#[test]
fn step_backward_stops_at_the_oldest_state() {
    let mut s = session();
    while s.is_enabled("history.stepBackward") {
        s.execute("history.stepBackward", json!({})).unwrap();
    }
    assert_eq!(entries(&s), ["New"]);
    assert!(s.execute("history.stepBackward", json!({})).is_err());
    assert_eq!(entries(&s), ["New"], "a refused step changes nothing");
}

#[test]
fn clear_history_keeps_the_image_and_leaves_one_state() {
    let mut s = session();
    s.execute("history.stepBackward", json!({})).unwrap();
    let all = entries(&s);
    let doc = s.active().unwrap().doc.clone();
    let revision = s.active().unwrap().revision;
    let r = s.execute("history.clear", json!({})).unwrap();
    assert_eq!((r["states"].as_u64(), r["current"].as_u64()), (Some(1), Some(0)));
    let st = s.active().unwrap();
    assert_eq!(st.history.entries(), [all.last().unwrap().clone()], "the current state is the only one");
    assert!(!st.history.can_redo(), "redo states are cleared too");
    assert!(Arc::ptr_eq(&st.doc, &doc) && st.revision == revision, "the image doesn't change");
    assert!(!s.is_enabled("history.clear"), "nothing left to clear");
    assert!(s.execute("history.clear", json!({})).is_err());
    assert!(!s.is_enabled("history.stepForward") && !s.is_enabled("history.deleteState"));
    assert!(!s.is_enabled("edit.fade"));
}

#[test]
fn undo_after_clear_history_brings_the_states_back() {
    let mut s = session();
    s.execute("history.stepBackward", json!({})).unwrap();
    let all = entries(&s);
    let redo: Vec<String> = s.active().unwrap().history.redo_labels().map(str::to_string).collect();
    let doc = s.active().unwrap().doc.clone();
    s.execute("history.clear", json!({})).unwrap();
    assert!(s.is_enabled("edit.undo") && s.is_enabled("history.stepBackward"), "Clear History is undoable");
    s.execute("edit.undo", json!({})).unwrap();
    let st = s.active().unwrap();
    assert_eq!(st.history.entries(), all);
    assert_eq!(st.history.redo_labels().map(str::to_string).collect::<Vec<_>>(), redo);
    assert!(Arc::ptr_eq(&st.doc, &doc), "undoing it doesn't change the image either");
    // A step after Clear History forgets the cleared states.
    s.execute("history.clear", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    assert_eq!(entries(&s).len(), 2);
    s.execute("edit.undo", json!({})).unwrap();
    assert!(!s.is_enabled("edit.undo"), "the cleared states are gone");
}

#[test]
fn clear_history_and_its_undo_keep_a_saved_document_clean() {
    let mut s = session();
    let st = s.active_mut().unwrap();
    st.saved_revision = st.revision;
    s.execute("history.clear", json!({})).unwrap();
    assert!(!s.active().unwrap().is_dirty(), "Clear History is no edit");
    let revision = s.active().unwrap().revision;
    assert!(s.execute("edit.undo", json!({})).unwrap().as_bool().unwrap());
    let st = s.active().unwrap();
    assert!(!st.is_dirty(), "nor is undoing it");
    assert!(st.revision > revision, "the panels still see the list change");
    // A modified document stays modified.
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    s.execute("history.clear", json!({})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.active().unwrap().is_dirty());
}

#[test]
fn clear_history_leaves_other_documents_alone() {
    let mut s = session();
    let first = entries(&s);
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    assert_eq!(s.active_index(), Some(1));
    s.execute("history.clear", json!({})).unwrap();
    assert_eq!(entries(&s).len(), 1);
    let other = &s.documents()[0];
    assert_eq!(other.history.entries(), first, "the other document keeps its states");
    assert!(!other.history.is_cleared() && other.history.can_undo());
    assert!(s.set_active(0));
    assert!(s.is_enabled("history.clear") && s.is_enabled("history.stepBackward"));
    s.execute("history.stepBackward", json!({})).unwrap();
    assert_eq!(entries(&s).len(), first.len() - 1);
}
