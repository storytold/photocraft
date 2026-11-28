//! #12845 (EPIC-HISTORY-BRUSH #12618): the History Brush's default source is the open snapshot,
//! the document as opened, not the oldest state held. `historySource` therefore reports
//! `{snapshot, name}` with the document's name on a new or opened document, the default survives
//! the history filling up, deleting the open snapshot leaves no source, and with History Options ›
//! Automatically Create First Snapshot off the default stays the oldest state.

use serde_json::{Value, json};

use crate::Session;

/// The active document's History Brush source, as `historySource` in `doc_inspect` reports it.
fn source(s: &Session) -> Value {
    crate::inspect::document(s.active().unwrap())["historySource"].clone()
}

/// The name of the active document's first snapshot row.
fn opened_name(s: &Session) -> String {
    s.active().unwrap().snapshots()[0].name.clone()
}

#[test]
fn the_default_source_is_the_open_snapshot_named_after_the_document() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    assert_eq!(source(&s), json!({"snapshot": 0, "name": opened_name(&s)}));
    assert_eq!(s.active().unwrap().doc.name, opened_name(&s), "the open snapshot is named after the document");
    // Edits move the oldest state's label but not the default.
    s.execute("layer.new.layer", json!({})).unwrap();
    assert_eq!(source(&s), json!({"snapshot": 0, "name": opened_name(&s)}));
}

#[test]
fn the_default_open_snapshot_survives_a_full_history() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let expected = source(&s);
    s.active_mut().unwrap().history.max_states = 2;
    for _ in 0..6 {
        s.execute("layer.new.layer", json!({})).unwrap();
    }
    let st = s.active().unwrap();
    assert_eq!(st.history.past_len(), 2, "the history filled up and older states fell off");
    assert!(st.history.entries().first().is_some_and(|label| label != "New"), "the opening state is gone from row 0: {:?}", st.history.entries());
    assert_eq!(source(&s), expected, "the default still names the document as opened");
    assert_eq!(st.snapshots().len(), 1, "the open snapshot is not a state and is never trimmed");
}

#[test]
fn deleting_the_open_snapshot_leaves_no_source() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("history.deleteSnapshot", json!({"snapshot": 0})).unwrap();
    assert!(source(&s).is_null(), "no source until one is set");
    // An explicit state source still works after that.
    s.execute("history.setSource", json!({"state": 0})).unwrap();
    assert_eq!(source(&s)["state"], 0);
}

#[test]
fn the_default_follows_automatically_create_first_snapshot() {
    // Each session starts from the default and carries its own preferences: the option is set on
    // the session that then opens the document.
    for first in [false, true] {
        let mut s = crate::session();
        s.execute("history.options", json!({"automaticallyCreateFirstSnapshot": first})).unwrap();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        if first {
            assert_eq!(source(&s), json!({"snapshot": 0, "name": opened_name(&s)}), "the open snapshot");
        } else {
            // Not measured: no open snapshot exists, so today's default (the oldest state) stays.
            assert_eq!(source(&s), json!({"state": 0, "label": "New"}), "the oldest state held");
        }
    }
}

/// A state set in the panel is still the source once the open snapshot is gone: the default does
/// not overwrite an explicit choice.
#[test]
fn an_explicit_state_source_survives_deleting_the_open_snapshot() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("history.setSource", json!({"state": 1})).unwrap();
    assert_eq!(source(&s), json!({"state": 1, "label": "New Layer"}));
    s.execute("history.deleteSnapshot", json!({"snapshot": 0})).unwrap();
    assert_eq!(source(&s), json!({"state": 1, "label": "New Layer"}), "explicit sources are untouched");
}

/// With Automatically Create First Snapshot off, deleting states down to one and setting a state
/// source keep working exactly as today (the oldest-state default).
#[test]
fn without_an_open_snapshot_the_oldest_state_default_remains() {
    let mut s = crate::session();
    s.execute("history.options", json!({"automaticallyCreateFirstSnapshot": false})).unwrap();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    // The current state is a redo row now; the default is still row 0.
    assert_eq!(source(&s), json!({"state": 0, "label": "New"}));
    s.execute("history.setSource", json!({"state": 1})).unwrap();
    assert_eq!(source(&s)["label"], "New Layer");
}

/// `history.newSnapshot` with `replaceExisting` drops a snapshot of that name, so taking the
/// document's own name removes the open snapshot. Snapshot ids are never reused, so the new row is
/// not the open snapshot, and the default source is gone with it (#12845).
#[test]
fn a_replacement_named_after_the_document_takes_the_default_source_away() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let name = s.active().unwrap().doc.name.clone();
    s.execute("layer.new.layer", json!({})).unwrap();
    let fresh = s.execute("history.newSnapshot", json!({"name": name, "replaceExisting": true})).unwrap()["snapshot"].as_u64().unwrap();
    assert_ne!(fresh, 0, "the new row takes a fresh id");
    assert!(source(&s).is_null(), "no open snapshot, so no default source");
    // A snapshot or a state picked by hand still works.
    s.execute("history.setSource", json!({"snapshot": fresh})).unwrap();
    assert_eq!(source(&s)["snapshot"], json!(fresh));
    s.execute("history.setSource", json!({"state": 0})).unwrap();
    assert_eq!(source(&s)["state"], 0);
}
