//! Tests that run paint commands, moved from the engine-core `history_cmds` tests: 15 test(s).

use photocraft_doc::Document;
use photocraft_engine::Session;
use serde_json::{Value, json};

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
fn history_undo_and_redo_alias_edit_undo_and_redo() {
    let mut s = session();
    let n = s.active().unwrap().history.past_len();
    s.execute("history.undo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), n - 1, "history.undo steps back like edit.undo");
    s.execute("history.redo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), n, "history.redo steps forward like edit.redo");
}

#[test]
fn delete_current_state_steps_back_and_drops_it() {
    let mut s = session();
    let before = entries(&s);
    let prev_doc = s.active().unwrap().history.state(before.len() - 2).unwrap();
    let r = s.execute("history.deleteState", json!({})).unwrap();
    assert_eq!(r["states"], json!(before.len() - 1));
    assert_eq!(entries(&s), before[..before.len() - 1].to_vec());
    assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &prev_doc), "the document is the state before");
    assert!(!s.active().unwrap().history.can_redo(), "the deleted state can't be redone");
    assert!(s.execute("edit.redo", json!({})).is_err());
}

#[test]
fn delete_an_earlier_state_drops_it_and_every_later_one() {
    let mut s = session();
    let first = s.active().unwrap().history.state(0).unwrap();
    s.execute("history.deleteState", json!({"state": 1})).unwrap();
    assert_eq!(entries(&s), vec!["New".to_string()]);
    assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &first));
    assert!(!s.active().unwrap().history.can_redo());
    // Redo states after an undo go too.
    let mut s = session();
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("history.deleteState", json!({})).unwrap();
    assert_eq!(entries(&s).len(), 2);
    assert!(!s.active().unwrap().history.can_redo());
}

#[test]
fn delete_state_refuses_the_oldest_state_and_bad_params() {
    let mut s = Session::new();
    assert!(s.execute("history.deleteState", json!({})).is_err(), "no document");
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    assert!(!s.is_enabled("history.deleteState"), "only the opening state");
    assert!(s.execute("history.deleteState", json!({})).is_err());
    let mut s = session();
    let before = entries(&s);
    for p in [json!({"state": 0}), json!({"state": -1}), json!({"state": 99}), json!({"state": "x"}), json!({"state": f64::MAX}), json!({"state": [1]})] {
        assert!(s.execute("history.deleteState", p.clone()).is_err(), "{p}");
        assert_eq!(entries(&s), before, "{p} changed nothing");
    }
}

#[test]
fn new_document_from_a_state_copies_it_into_a_fresh_document() {
    let mut s = session();
    let src = s.active().unwrap().doc.clone();
    let label = entries(&s).last().cloned().unwrap();
    let r = s.execute("history.newDocument", json!({})).unwrap();
    assert_eq!(s.documents().len(), 2);
    assert_eq!(r["document"], json!(1));
    assert_eq!(r["name"], json!(label), "named after the state");
    let new = s.active().unwrap();
    assert_eq!(new.doc.size, src.size);
    assert_eq!(new.doc.layers.len(), src.layers.len());
    assert_ne!(new.doc.id, src.id);
    assert_eq!(new.history.entries().len(), 1, "a fresh history");
    // An earlier state, with a chosen name; the source document is untouched.
    s.set_active(0);
    let before = entries(&s);
    let open = s.active().unwrap().history.state(0).unwrap();
    s.execute("history.newDocument", json!({"state": 0, "name": "Start"})).unwrap();
    let new = s.active().unwrap();
    assert_eq!(new.doc.name, "Start");
    assert_eq!(new.doc.layers.len(), open.layers.len());
    s.set_active(0);
    assert_eq!(entries(&s), before);
}

/// The top layer's pixel at (1, 1).
fn top_pixel(s: &Session) -> [f32; 4] {
    s.active().unwrap().doc.layers.last().unwrap().surface().unwrap().rgba(1, 1)
}

fn snapshots(s: &Session) -> Value {
    photocraft_engine::inspect::document(s.active().unwrap())["snapshots"].clone()
}

#[test]
fn new_snapshot_keeps_the_document_and_is_not_a_history_state() {
    let mut s = session();
    let before = entries(&s);
    let r = s.execute("history.newSnapshot", json!({})).unwrap();
    assert_eq!(r, json!({"snapshot": 1, "name": "Snapshot 1"}));
    assert_eq!(entries(&s), before, "taking a snapshot is not a step");
    let r = s.execute("history.newSnapshot", json!({"name": "Before green"})).unwrap();
    assert_eq!(r, json!({"snapshot": 2, "name": "Before green"}));
    let first = json!({"id": 0, "name": s.active().unwrap().doc.name});
    assert_eq!(snapshots(&s), json!([first, {"id": 1, "name": "Snapshot 1"}, {"id": 2, "name": "Before green"}]));
}

#[test]
fn replace_existing_snapshot_and_clear_history() {
    let mut s = session();
    s.execute("history.newSnapshot", json!({"name": "Result"})).unwrap();
    s.execute("history.newSnapshot", json!({"name": "Other"})).unwrap();
    s.execute("history.newSnapshot", json!({"name": "Result", "replaceExisting": true})).unwrap();
    let opening = json!({"id": 0, "name": s.active().unwrap().doc.name});
    assert_eq!(snapshots(&s), json!([opening, {"id": 2, "name": "Other"}, {"id": 3, "name": "Result"}]));
    let kept_snapshots = snapshots(&s);
    let history_before = entries(&s);
    let pixel = top_pixel(&s);
    s.execute("history.clear", json!({})).unwrap();
    assert_eq!(entries(&s).len(), 1, "only the current state is left");
    assert!(s.active().unwrap().history.can_undo(), "the current Clear History remains undoable");
    assert_eq!(top_pixel(&s), pixel, "the document is unchanged");
    assert_eq!(snapshots(&s), kept_snapshots, "opening and named snapshots stay unchanged");
    // Current History menu semantics refuse a second clear; snapshots remain independent.
    assert!(s.execute("history.clear", json!({})).is_err());
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(entries(&s), history_before, "Undo restores the cleared states");
    assert_eq!(snapshots(&s), kept_snapshots, "Undo also preserves every snapshot");
    assert_eq!(top_pixel(&s), pixel);
    assert!(Session::new().execute("history.clear", json!({})).is_err(), "no document");
}

#[test]
fn select_snapshot_returns_to_it_as_one_undoable_state() {
    let mut s = session();
    s.execute("history.newSnapshot", json!({})).unwrap();
    let green = top_pixel(&s);
    let id = s.active().unwrap().doc.id;
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    let blue = top_pixel(&s);
    let before = entries(&s);
    s.execute("history.selectSnapshot", json!({"snapshot": 1})).unwrap();
    assert_eq!(top_pixel(&s), green);
    assert_eq!(entries(&s).len(), before.len() + 1, "one new state");
    assert_eq!(entries(&s).last().map(String::as_str), Some("Snapshot 1"));
    assert_eq!(s.active().unwrap().doc.id, id, "the same document");
    assert!(s.active().unwrap().active_layer.is_some());
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(top_pixel(&s), blue, "undo leaves the snapshot");
    assert_eq!(snapshots(&s).as_array().unwrap().len(), 2, "snapshots are not history");
}

#[test]
fn snapshots_from_merged_layers_or_the_current_layer() {
    let mut s = session();
    assert_eq!(s.active().unwrap().doc.layers.len(), 2);
    s.execute("history.newSnapshot", json!({"from": "mergedLayers"})).unwrap();
    s.execute("history.newSnapshot", json!({"from": "currentLayer"})).unwrap();
    s.execute("history.newSnapshot", json!({"from": "fullDocument"})).unwrap();
    s.execute("history.selectSnapshot", json!({"snapshot": 1})).unwrap();
    let doc = s.active().unwrap().doc.clone();
    assert_eq!(doc.layers.len(), 1, "merged into one layer");
    assert_eq!(doc.layers[0].surface().unwrap().rgba(1, 1), [0.0, 1.0, 0.0, 1.0], "the visible result");
    s.execute("history.selectSnapshot", json!({"snapshot": 2})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers.len(), 1, "only the active layer");
    s.execute("history.selectSnapshot", json!({"snapshot": 3})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers.len(), 2);
}

fn tile_bytes(doc: &Document) -> usize {
    doc.layers.iter().filter_map(|l| l.surface()).flat_map(|s| s.tiles().map(|(_, t)| t.len())).sum()
}

/// A Merged Layers snapshot is a flattened copy only it holds (#4530): it counts in the
/// document's memory and against the history budget, so the same edits keep one state less.
#[test]
fn merged_snapshot_counts_against_the_history_budget() {
    let (mut with, mut without) = (session(), session());
    let base = with.active().unwrap().memory_bytes();
    with.execute("history.newSnapshot", json!({"from": "mergedLayers"})).unwrap();
    let st = with.active().unwrap();
    let flat = tile_bytes(&st.snapshots().last().unwrap().doc);
    assert!(flat > 0);
    assert_eq!(st.memory_bytes(), base + flat);
    // Room for the document's tiles and four fill tiles.
    let cur = tile_bytes(&st.doc);
    let fill = tile_bytes(&Document { layers: st.doc.layers[1..].to_vec().into(), ..(*st.doc).clone() });
    assert!(flat <= fill, "the flattened tile is no bigger than a fill's");
    let budget = cur + 4 * fill;
    for s in [&mut with, &mut without] {
        s.active_mut().unwrap().history.max_bytes = budget;
        for i in 0..6 {
            s.execute("edit.fill", json!({"color": format!("#0000{:02x}", 0x40 + i * 0x20)})).unwrap();
        }
    }
    let held = |s: &Session| s.active().unwrap().history.past_len();
    assert_eq!(held(&without), 4);
    assert_eq!(held(&with), 3, "the snapshot's tile takes one state's room");
    assert!(with.active().unwrap().memory_bytes() <= budget);
}

/// Purge › Histories keeps the snapshots, so it doesn't report the tiles they share with
/// the states as freed (#4530).
#[test]
fn purging_histories_frees_only_what_no_snapshot_keeps() {
    let mut s = session();
    s.execute("history.newSnapshot", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    let st = s.active().unwrap();
    // The snapshot's green layer tile, now only in it and the last state.
    let shared = tile_bytes(&Document { layers: st.snapshots().last().unwrap().doc.layers[1..].to_vec().into(), ..(*st.doc).clone() });
    let all = st.history.unique_bytes(&st.doc);
    let r = s.execute("edit.purge.histories", json!({})).unwrap();
    assert_eq!(r["bytes"], json!(all - shared), "the green tile stays with the snapshot");
    assert_eq!(s.active().unwrap().snapshots().len(), 2, "the first snapshot and the new one");
}

#[test]
fn rename_and_delete_snapshots_keep_the_other_ids() {
    let mut s = session();
    for _ in 0..3 {
        s.execute("history.newSnapshot", json!({})).unwrap();
    }
    s.execute("history.renameSnapshot", json!({"snapshot": 2, "name": "Lesson start"})).unwrap();
    s.execute("history.deleteSnapshot", json!({"snapshot": 1})).unwrap();
    s.execute("history.deleteSnapshot", json!({"snapshot": 0})).unwrap();
    assert_eq!(snapshots(&s), json!([{"id": 2, "name": "Lesson start"}, {"id": 3, "name": "Snapshot 3"}]), "the first snapshot can go too");
    let r = s.execute("history.newSnapshot", json!({})).unwrap();
    assert_eq!(r["snapshot"], json!(4), "ids are never reused");
}

/// Photoshop's first snapshot (#2966): every document starts with a snapshot of itself as
/// opened, named after it, so the opening state can always be brought back, even after
/// History has dropped its oldest states.
#[test]
fn a_document_starts_with_a_first_snapshot_of_itself() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 20, "height": 10})).unwrap();
    let name = s.active().unwrap().doc.name.clone();
    assert_eq!(snapshots(&s), json!([{"id": 0, "name": name}]));
    let opened = top_pixel(&s);
    for _ in 0..3 {
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    }
    s.active_mut().unwrap().history.purge_last();
    s.execute("history.selectSnapshot", json!({"snapshot": 0})).unwrap();
    assert_eq!(top_pixel(&s), opened, "back to the document as opened");
    assert_eq!(entries(&s).last(), Some(&name), "a state named after the document");
}

#[test]
fn snapshot_commands_refuse_bad_params() {
    let mut s = Session::new();
    for (cmd, p) in [
        ("history.newSnapshot", json!({})),
        ("history.selectSnapshot", json!({"snapshot": 1})),
        ("history.renameSnapshot", json!({"snapshot": 1, "name": "x"})),
        ("history.deleteSnapshot", json!({"snapshot": 1})),
    ] {
        assert!(s.execute(cmd, p).is_err(), "{cmd}: no document");
    }
    let mut s = session();
    s.execute("history.newSnapshot", json!({})).unwrap();
    let (before, snaps) = (entries(&s), snapshots(&s));
    for (cmd, p) in [
        ("history.newSnapshot", json!({"from": "everything"})),
        ("history.newSnapshot", json!({"from": 3})),
        ("history.newSnapshot", json!({"name": 7})),
        ("history.selectSnapshot", json!({})),
        ("history.selectSnapshot", json!({"snapshot": 99})),
        ("history.selectSnapshot", json!({"snapshot": -1})),
        ("history.selectSnapshot", json!({"snapshot": "1"})),
        ("history.selectSnapshot", json!({"snapshot": f64::MAX})),
        ("history.renameSnapshot", json!({"snapshot": 1})),
        ("history.renameSnapshot", json!({"snapshot": 1, "name": "  "})),
        ("history.renameSnapshot", json!({"snapshot": 99, "name": "x"})),
        ("history.deleteSnapshot", json!({"snapshot": 99})),
        ("history.deleteSnapshot", json!({"snapshot": [1]})),
    ] {
        assert!(s.execute(cmd, p.clone()).is_err(), "{cmd} {p}");
        assert_eq!((entries(&s), snapshots(&s)), (before.clone(), snaps.clone()), "{cmd} {p} changed nothing");
    }
    // Current Layer needs an active layer.
    s.execute("select.deselectLayers", json!({})).unwrap();
    assert!(s.execute("history.newSnapshot", json!({"from": "currentLayer"})).is_err());
}

#[test]
fn new_document_refuses_bad_params() {
    let mut s = Session::new();
    assert!(s.execute("history.newDocument", json!({})).is_err(), "no document");
    let mut s = session();
    for p in [json!({"state": -1}), json!({"state": 99}), json!({"state": "x"}), json!({"state": f64::NAN.to_string()})] {
        assert!(s.execute("history.newDocument", p.clone()).is_err(), "{p}");
    }
    assert_eq!(s.documents().len(), 1);
}
