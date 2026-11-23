//! Tests that run paint commands, moved from the engine-core `history_source_cmds` tests: 10 test(s).

use photocraft_engine::{EngineError, Session};
use serde_json::{Value, json};

/// A white 8×8 document filled red, then green: rows Open, Fill (red), Fill (green).
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "depth": 16, "background": "white"})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
    s
}

fn rgb(s: &Session) -> [f32; 3] {
    let st = s.active().unwrap();
    let px = st.doc.layers[0].surface().unwrap().rgba(4, 4);
    [px[0], px[1], px[2]]
}

fn brush(s: &mut Session, extra: Value) {
    let mut p = json!({"points": [[4, 4]], "size": 40, "hardness": 100});
    if let (Some(p), Some(e)) = (p.as_object_mut(), extra.as_object()) {
        p.extend(e.clone());
    }
    s.execute("paint.historyBrush", p).unwrap();
}

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01)
}

#[test]
fn default_source_is_the_oldest_state() {
    let mut s = session();
    assert_eq!(photocraft_engine::inspect::document(s.active().unwrap())["historySource"], json!({"state": 0, "label": "New"}));
    brush(&mut s, json!({}));
    assert!(close(rgb(&s), [1.0, 1.0, 1.0]), "{:?}", rgb(&s));
}

#[test]
fn the_history_brush_and_fill_paint_from_the_set_source() {
    let mut s = session();
    let r = s.execute("history.setSource", json!({"state": 1})).unwrap();
    assert_eq!(r, json!({"state": 1, "label": "Fill"}));
    assert_eq!(photocraft_engine::inspect::document(s.active().unwrap())["historySource"]["state"], 1);
    brush(&mut s, json!({}));
    assert!(close(rgb(&s), [1.0, 0.0, 0.0]), "the red state: {:?}", rgb(&s));
    // An explicit `state` still wins.
    brush(&mut s, json!({"state": 0}));
    assert!(close(rgb(&s), [1.0, 1.0, 1.0]), "{:?}", rgb(&s));
    s.execute("edit.fill", json!({"contents": "history"})).unwrap();
    assert!(close(rgb(&s), [1.0, 0.0, 0.0]), "Fill › History uses the source too: {:?}", rgb(&s));
    // Setting the source is not a step.
    assert_eq!(s.active().unwrap().history.entries().len(), 6);
}

/// The common Photoshop workflow: apply, step back, set the source on the greyed state, paint.
#[test]
fn a_redo_state_can_be_the_source_until_a_new_step_discards_it() {
    let mut s = session();
    assert!(s.undo());
    s.execute("history.setSource", json!({"state": 2})).unwrap();
    brush(&mut s, json!({}));
    assert!(close(rgb(&s), [0.0, 1.0, 0.0]), "the greyed green state: {:?}", rgb(&s));
    // Painting was a new step, which discarded the green state: no source until one is set.
    assert!(photocraft_engine::inspect::document(s.active().unwrap())["historySource"].is_null());
    let before = rgb(&s);
    assert!(s.execute("paint.historyBrush", json!({"points": [[4, 4]], "size": 40})).is_err());
    assert!(s.execute("edit.fill", json!({"contents": "history"})).is_err());
    assert!(close(rgb(&s), before), "nothing painted");
    brush(&mut s, json!({"state": 0}));
    assert!(close(rgb(&s), [1.0, 1.0, 1.0]), "an explicit state still paints: {:?}", rgb(&s));
    s.execute("history.setSource", json!({"state": 0})).unwrap();
    brush(&mut s, json!({}));
}

#[test]
fn the_source_follows_its_state_as_older_states_fall_off() {
    let mut s = session();
    s.active_mut().unwrap().history.max_states = 2;
    s.execute("history.setSource", json!({"state": 1})).unwrap();
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    // Open fell off: the red state is row 0 now and still the source.
    let st = s.active().unwrap();
    assert_eq!((st.history.entries().len(), st.history.brush_source()), (3, Some(0)));
    // It would fall off next: it is kept, as in Photoshop, and paints red.
    s.execute("edit.fill", json!({"color": "#ffffff"})).unwrap();
    brush(&mut s, json!({}));
    let st = s.active().unwrap();
    assert_eq!((st.history.entries()[0].as_str(), st.history.brush_source()), ("Fill", Some(0)));
    assert!(close(rgb(&s), [1.0, 0.0, 0.0]), "{:?}", rgb(&s));
}

#[test]
fn deleting_the_source_state_leaves_no_source() {
    let mut s = session();
    s.execute("history.setSource", json!({"state": 2})).unwrap();
    s.execute("history.deleteState", json!({"state": 2})).unwrap();
    assert!(photocraft_engine::inspect::document(s.active().unwrap())["historySource"].is_null());
    assert!(s.execute("paint.historyBrush", json!({"points": [[4, 4]], "size": 40})).is_err());
    s.execute("history.setSource", json!({"state": 1})).unwrap();
    brush(&mut s, json!({}));
    assert!(close(rgb(&s), [1.0, 0.0, 0.0]), "{:?}", rgb(&s));
}

/// Edit › Purge › Histories clears the states: the source goes back to the default instead of
/// leaving every document without one (#3536 review).
#[test]
fn purging_histories_resets_a_state_source() {
    let mut s = session();
    s.execute("history.setSource", json!({"state": 1})).unwrap();
    s.execute("edit.purge.histories", json!({})).unwrap();
    assert_eq!(photocraft_engine::inspect::document(s.active().unwrap())["historySource"], json!({"state": 0, "label": "Fill"}));
    brush(&mut s, json!({}));
    assert!(close(rgb(&s), [0.0, 1.0, 0.0]), "the only state left: {:?}", rgb(&s));
    s.execute("edit.fill", json!({"contents": "history"})).unwrap();
}

/// After Canvas Size (or Crop, Image Size) the old states have another size: painting from one
/// would shift the pixels, so the History Brush and Fill › History refuse, as Photoshop does.
#[test]
fn a_source_of_another_canvas_size_is_refused() {
    let mut s = session();
    s.execute("image.canvasSize", json!({"width": 12, "height": 8, "anchor": "topLeft"})).unwrap();
    for cmd in ["paint.historyBrush", "edit.fill"] {
        for state in [json!(null), json!(0), json!(2)] {
            let p = if cmd == "edit.fill" { json!({"contents": "history", "state": state}) } else { json!({"points": [[4, 4]], "size": 40, "state": state}) };
            let e = s.execute(cmd, p.clone()).unwrap_err().to_string();
            assert!(e.contains("canvas size"), "{cmd} {p}: {e}");
        }
    }
    assert_eq!(s.active().unwrap().history.entries().len(), 4, "nothing painted");
    // The state after the resize is the same size: it paints.
    s.execute("history.setSource", json!({"state": 3})).unwrap();
    brush(&mut s, json!({}));
    s.execute("edit.fill", json!({"contents": "history"})).unwrap();
    // A snapshot of the old size is refused the same way.
    let snap = s.execute("history.newSnapshot", json!({})).unwrap()["snapshot"].clone();
    s.execute("image.canvasSize", json!({"width": 8, "height": 8, "anchor": "topLeft"})).unwrap();
    s.execute("history.setSource", json!({"snapshot": snap})).unwrap();
    assert!(s.execute("paint.historyBrush", json!({"points": [[4, 4]], "size": 40})).is_err());
}

/// `state` is a History panel row for the History Brush and Fill too: the greyed redo rows
/// count, as in `history.setSource` and `historySource`.
#[test]
fn the_brush_and_fill_take_redo_rows() {
    let mut s = session();
    assert!(s.undo());
    brush(&mut s, json!({"state": 2}));
    assert!(close(rgb(&s), [0.0, 1.0, 0.0]), "the greyed green state: {:?}", rgb(&s));
    let mut s = session();
    assert!(s.undo());
    s.execute("edit.fill", json!({"contents": "history", "state": 2})).unwrap();
    assert!(close(rgb(&s), [0.0, 1.0, 0.0]), "{:?}", rgb(&s));
    assert!(s.execute("edit.fill", json!({"contents": "history", "state": 4})).is_err(), "no such row");
}

/// A snapshot as the source (the column on snapshot rows): kept by snapshot id, apart from the
/// state rows; deleting it leaves no source; Purge › Histories keeps it.
#[test]
fn a_snapshot_can_be_the_source() {
    let mut s = session();
    assert!(s.undo());
    let snap = s.execute("history.newSnapshot", json!({"name": "Red"})).unwrap()["snapshot"].as_u64().unwrap();
    assert_eq!(snap, 1, "snapshot 1, while state row 1 is the red fill too: they must not mix");
    assert!(s.redo());
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    let r = s.execute("history.setSource", json!({"snapshot": snap})).unwrap();
    assert_eq!(r, json!({"snapshot": 1, "name": "Red"}));
    assert_eq!(photocraft_engine::inspect::document(s.active().unwrap())["historySource"], r);
    // No state is the source now: the default would be state 0 (white).
    assert_eq!(s.active().unwrap().history.brush_source(), Some(0));
    brush(&mut s, json!({}));
    assert!(close(rgb(&s), [1.0, 0.0, 0.0]), "the snapshot paints: {:?}", rgb(&s));
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    s.execute("edit.fill", json!({"contents": "history"})).unwrap();
    assert!(close(rgb(&s), [1.0, 0.0, 0.0]), "{:?}", rgb(&s));
    // An explicit snapshot or state still wins.
    brush(&mut s, json!({"state": 0}));
    assert!(close(rgb(&s), [1.0, 1.0, 1.0]), "{:?}", rgb(&s));
    brush(&mut s, json!({"snapshot": snap}));
    assert!(close(rgb(&s), [1.0, 0.0, 0.0]), "{:?}", rgb(&s));
    // Purging histories keeps the snapshots and so the source.
    s.execute("edit.purge.histories", json!({})).unwrap();
    assert_eq!(photocraft_engine::inspect::document(s.active().unwrap())["historySource"], r);
    // A state source replaces it.
    s.execute("history.setSource", json!({"state": 0})).unwrap();
    assert_eq!(photocraft_engine::inspect::document(s.active().unwrap())["historySource"]["state"], 0);
    s.execute("history.setSource", json!({"snapshot": snap})).unwrap();
    // Deleting the snapshot leaves no source.
    s.execute("history.deleteSnapshot", json!({"snapshot": snap})).unwrap();
    assert!(photocraft_engine::inspect::document(s.active().unwrap())["historySource"].is_null());
    assert!(s.execute("paint.historyBrush", json!({"points": [[4, 4]], "size": 40})).is_err());
    assert!(s.execute("edit.fill", json!({"contents": "history"})).is_err());
    assert!(s.execute("history.setSource", json!({"snapshot": snap})).is_err(), "it is gone");
}

#[test]
fn bad_params_and_no_document_are_errors() {
    assert!(matches!(Session::new().execute("history.setSource", json!({"state": 0})), Err(EngineError::NoDocument | EngineError::Disabled(..))));
    let mut s = session();
    for p in [
        json!({}),
        json!({"state": null}),
        json!({"state": -1}),
        json!({"state": 3}),
        json!({"state": 1e300}),
        json!({"state": 1.5}),
        json!({"state": "1"}),
        json!({"state": [1]}),
        json!({"snapshot": 99}),
        json!({"snapshot": 1}),
        json!({"snapshot": -1}),
        json!({"snapshot": "Open"}),
        json!({"snapshot": "Open", "state": 1}),
        json!({"snapshot": 1, "state": 1}),
    ] {
        assert!(s.execute("history.setSource", p.clone()).is_err(), "{p}");
    }
    assert_eq!(s.active().unwrap().history.brush_source(), Some(0), "nothing changed");
    // Snapshot 0 is the document as opened (Automatically Create First Snapshot), so it is a valid source.
    assert_eq!(s.execute("history.setSource", json!({"snapshot": 0})).unwrap()["snapshot"], 0);
}
