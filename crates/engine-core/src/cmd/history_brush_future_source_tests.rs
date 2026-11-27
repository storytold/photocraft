//! #12844 (EPIC-HISTORY-BRUSH #12618): a History Brush source that is a later (redo) state stays
//! in the list when a new step discards the later states. It stays the source, so further clicks
//! paint from it again, it stays the last row (the panel draws it dimmed), and `historySource` keeps
//! reporting it. It is not a state the document can step forward to: Redo skips it, and Undo of the
//! step gives back the state that step replaced, as before. Choosing another source, deleting
//! states and Edit › Purge › Histories let the kept state go as they do today.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::DocState;
use crate::Session;

/// The active document's History Brush source, as `historySource` in `doc_inspect` reports it.
fn source(s: &Session) -> Value {
    crate::inspect::document(s.active().unwrap())["historySource"].clone()
}

/// New, then two steps, then one Undo: the newest step's state is now the later (redo) row, the row
/// after the current one.
fn stepped_back() -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    s
}

/// The last row of the History panel list: the farthest later state, listed after every other row
/// (#12844 keeps a source state a step discarded there).
fn last_row(st: &DocState) -> usize {
    st.history.past_len() + st.history.redo_labels().count()
}

/// The label History panel row `row` carries: the states, then the current one, then the later ones.
fn row_label(st: &DocState, row: usize) -> String {
    st.history.entries().into_iter().chain(st.history.redo_labels().map(str::to_string)).nth(row).expect("the row is listed")
}

/// Make the later (redo) state the source and return its [`photocraft_ops::HistoryState::id`] and
/// the document it holds.
fn set_later_source(s: &mut Session) -> (u64, Arc<photocraft_doc::Document>) {
    let st = s.active().unwrap();
    let row = last_row(st);
    let id = st.history.row_id(row).expect("the later state has a row");
    let doc = st.history.row_doc(row, &st.doc).expect("the later state is held");
    s.execute("history.setSource", json!({"state": row})).unwrap();
    (id, doc)
}

/// A new step discards the later states, except the source: it is still the source, still the last
/// row, and the row still holds the very document it held before the step.
#[test]
fn the_source_state_survives_the_step_that_discards_the_later_states() {
    let mut s = stepped_back();
    let (id, kept_doc) = set_later_source(&mut s);
    let label = row_label(s.active().unwrap(), last_row(s.active().unwrap()));
    s.execute("layer.new.layer", json!({})).unwrap();
    let st = s.active().unwrap();
    let last = last_row(st);
    assert_eq!(st.history.row_of(id), Some(last), "the source state is still held, as the last row");
    assert_eq!(source(&s), json!({"state": last, "label": label}), "and still the source");
    let now = st.history.row_doc(last, &st.doc).expect("the source row");
    assert!(Arc::ptr_eq(&now, &kept_doc), "the same state, not a copy of a later one");
}

/// Further steps keep it: every one of them discards the states after the current one, and the
/// source stays the last row and the same document, so a click keeps painting those same pixels.
#[test]
fn further_steps_keep_the_source_state_and_its_pixels() {
    let mut s = stepped_back();
    let (id, kept_doc) = set_later_source(&mut s);
    for _ in 0..3 {
        s.execute("layer.new.layer", json!({})).unwrap();
        let st = s.active().unwrap();
        let last = last_row(st);
        assert_eq!(st.history.row_of(id), Some(last), "still the last row after another step");
        assert_eq!(source(&s)["state"], json!(last), "and still the source");
        let now = st.history.row_doc(last, &st.doc).expect("the source row");
        assert!(Arc::ptr_eq(&now, &kept_doc), "the same pixels every time");
    }
    assert_eq!(s.active().unwrap().history.redo_labels().count(), 1, "and only that one state is kept");
}

/// The kept state is not a state the document can step forward to: `edit.redo` refuses and leaves
/// the document where it is.
#[test]
fn the_kept_source_is_not_a_state_the_document_can_step_forward_to() {
    let mut s = stepped_back();
    let (id, _) = set_later_source(&mut s);
    s.execute("layer.new.layer", json!({})).unwrap();
    let before = s.active().unwrap().doc.clone();
    assert!(!s.active().unwrap().history.can_redo(), "the kept state is not a redo state");
    assert_eq!(crate::inspect::document(s.active().unwrap())["canRedo"], json!(false));
    let e = s.execute("edit.redo", json!({})).unwrap_err().to_string();
    assert!(e.contains("nothing to redo"), "{e}");
    let st = s.active().unwrap();
    assert!(Arc::ptr_eq(&st.doc, &before), "the document did not move");
    assert_eq!(st.history.row_of(id), Some(last_row(st)), "and the source is still the source");
}

/// Undo of the step gives back the state that step replaced, as it does today: Redo lands on that
/// state and never on the kept source.
#[test]
fn undo_of_the_step_gives_back_the_state_it_replaced() {
    let mut s = stepped_back();
    let (id, _) = set_later_source(&mut s);
    let replaced = s.active().unwrap().doc.clone();
    s.execute("layer.new.layer", json!({})).unwrap();
    let made = s.active().unwrap().doc.clone();
    s.execute("edit.undo", json!({})).unwrap();
    let st = s.active().unwrap();
    assert!(Arc::ptr_eq(&st.doc, &replaced), "the state the step replaced is current again");
    assert!(st.history.can_redo(), "the undo is redoable");
    assert_eq!(st.history.row_of(id), Some(last_row(st)), "the kept source stays the last row");
    s.execute("edit.redo", json!({})).unwrap();
    let st = s.active().unwrap();
    assert!(Arc::ptr_eq(&st.doc, &made), "Redo goes back to the step, not to the kept source");
    assert_eq!(st.history.row_of(id), Some(last_row(st)), "the source is still the source after Redo");
    assert_eq!(source(&s)["state"], json!(last_row(s.active().unwrap())));
}

/// Another state source replaces the kept one: the state that was only kept as a source goes, so
/// the panel does not carry an orphan row, and the new source works as before.
#[test]
fn another_state_source_lets_the_kept_state_go() {
    let mut s = stepped_back();
    let (id, _) = set_later_source(&mut s);
    s.execute("layer.new.layer", json!({})).unwrap();
    let listed = s.active().unwrap().history.entries().len();
    s.execute("history.setSource", json!({"state": 0})).unwrap();
    let st = s.active().unwrap();
    assert_eq!(source(&s)["state"], json!(0), "the new source");
    assert_eq!(st.history.row_of(id), None, "the state kept only as a source is gone");
    assert_eq!(st.history.redo_labels().count(), 0, "and the list has no later row left");
    assert!(!st.history.can_redo());
    assert_eq!(st.history.entries().len(), listed, "the steps themselves are untouched");
}

/// A snapshot source leaves no state source, so the kept state goes with it.
#[test]
fn a_snapshot_source_lets_the_kept_state_go() {
    let mut s = stepped_back();
    let snapshot = s.execute("history.newSnapshot", json!({})).unwrap()["snapshot"].as_u64().unwrap();
    let (id, _) = set_later_source(&mut s);
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("history.setSource", json!({"snapshot": snapshot})).unwrap();
    let st = s.active().unwrap();
    let name = st.snapshots().iter().find(|snap| snap.id == snapshot).map(|snap| snap.name.clone());
    assert_eq!(source(&s), json!({"snapshot": snapshot, "name": name}), "the snapshot is the source");
    assert_eq!(st.history.row_of(id), None, "no state source, so nothing is kept past its time");
    assert_eq!(st.history.redo_labels().count(), 0);
}

/// Edit › Purge › Histories still resets a state source to the default (#3536), kept state included.
#[test]
fn purging_histories_still_resets_a_state_source() {
    let mut s = stepped_back();
    let (id, _) = set_later_source(&mut s);
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.purge.histories", json!({})).unwrap();
    let st = s.active().unwrap();
    assert_eq!(st.history.redo_labels().count(), 0, "the purge leaves no later state");
    assert_eq!(st.history.row_of(id), None, "the kept state went with them");
    assert_eq!(source(&s)["state"], json!(0), "the source is the default again");
}

/// History panel › Delete still takes the state it names and everything after it, the kept source
/// row included; a source the delete removed leaves no source, as before.
#[test]
fn deleting_a_state_still_takes_everything_after_it() {
    let mut s = stepped_back();
    let (id, _) = set_later_source(&mut s);
    s.execute("layer.new.layer", json!({})).unwrap();
    let past = s.active().unwrap().history.past_len();
    s.execute("history.deleteState", json!({"state": past - 1})).unwrap();
    let st = s.active().unwrap();
    assert_eq!(st.history.row_of(id), None, "the deleted rows are gone, kept source included");
    assert_eq!(st.history.redo_labels().count(), 0, "nothing after them is left to redo");
    assert!(source(&s).is_null(), "a source the delete removed leaves no source");
}

/// The issue does not measure what clicking the kept row does, so it behaves like a click on any
/// other later row: it steps there. Stepping back from it must leave Redo working as usual, which
/// means the state stops being a kept one once it is the current state (#12844).
#[test]
fn stepping_to_the_kept_state_and_back_leaves_redo_working() {
    let mut s = stepped_back();
    let (id, _) = set_later_source(&mut s);
    s.execute("layer.new.layer", json!({})).unwrap();
    let kept_row = last_row(s.active().unwrap());
    s.execute("history.selectState", json!({"state": kept_row})).unwrap();
    let st = s.active().unwrap();
    assert_eq!(st.history.row_of(id), Some(st.history.past_len()), "the click made the kept state the current one");
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.active().unwrap().history.can_redo(), "the state that Undo left behind is redoable");
    s.execute("edit.redo", json!({})).unwrap();
    let st = s.active().unwrap();
    assert_eq!(st.history.row_of(id), Some(st.history.past_len()), "Redo comes back to it");
}

/// Without a source set, a new step still discards every later state and leaves nothing to redo,
/// exactly as it does today.
#[test]
fn a_step_with_no_source_set_still_discards_the_later_states() {
    let mut s = stepped_back();
    s.execute("layer.new.layer", json!({})).unwrap();
    let st = s.active().unwrap();
    assert_eq!(st.history.redo_labels().count(), 0, "no row is kept without a source");
    assert!(!st.history.can_redo());
    assert_eq!(st.history.entries().len(), st.history.past_len() + 1, "the list is the plain one");
}
