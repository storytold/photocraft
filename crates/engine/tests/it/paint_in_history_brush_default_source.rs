//! #12845 (EPIC-HISTORY-BRUSH #12618): the History Brush's default source is the open snapshot, so
//! a click paints the pixels the document had when it opened, even after the history filled up and
//! the opening state fell off. The opened pixels are read from the open snapshot itself, so the
//! comparison is against the state captured at open, not a hand-written colour.

use photocraft_engine::Session;
use serde_json::{Value, json};

/// A white 8×8 document.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "depth": 16, "background": "white"})).unwrap();
    s
}

fn rgba(s: &Session, x: i32, y: i32) -> [f32; 4] {
    let st = s.active().unwrap();
    st.doc.layers[0].surface().unwrap().rgba(x, y)
}

/// The pixel (3, 3) of the document as opened, as the History panel holds it (snapshot 0).
fn opened_pixel(s: &Session) -> [f32; 4] {
    s.active().unwrap().snapshots()[0].doc.layers[0].surface().unwrap().rgba(3, 3)
}

fn close(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01)
}

fn paint(s: &mut Session, extra: Value) -> photocraft_engine::Result<Value> {
    let mut p = json!({"points": [[3, 3]], "size": 40, "hardness": 100});
    if let (Some(p), Some(e)) = (p.as_object_mut(), extra.as_object()) {
        p.extend(e.clone());
    }
    s.execute("paint.historyBrush", p)
}

#[test]
fn default_source_reports_the_open_snapshot() {
    let s = session();
    let name = s.active().unwrap().doc.name.clone();
    assert_eq!(photocraft_engine::inspect::document(s.active().unwrap())["historySource"], json!({"snapshot": 0, "name": name}));
}

#[test]
fn the_default_brush_paints_the_pixels_the_document_opened_with() {
    let mut s = session();
    let at_open = opened_pixel(&s);
    s.execute("paint.stroke", json!({"points": [[0, 3], [7, 3]], "size": 8, "hardness": 100, "color": "#ff0000"})).unwrap();
    assert!(rgba(&s, 3, 3)[0] > 0.9 && rgba(&s, 3, 3)[1] < 0.1, "the stroke painted red");
    paint(&mut s, json!({})).unwrap();
    assert!(close(rgba(&s, 3, 3), at_open), "the default source restores the opened pixel: {:?} vs {at_open:?}", rgba(&s, 3, 3));
}

#[test]
fn a_default_brush_click_still_paints_the_opened_image_after_the_history_fills_up() {
    let mut s = session();
    let at_open = opened_pixel(&s);
    s.active_mut().unwrap().history.max_states = 2;
    for colour in ["#ff0000", "#00ff00", "#0000ff", "#ff00ff"] {
        s.execute("paint.stroke", json!({"points": [[0, 3], [7, 3]], "size": 8, "hardness": 100, "color": colour})).unwrap();
    }
    let st = s.active().unwrap();
    assert!(st.history.entries()[0] != "New", "the opening state fell off row 0: {:?}", st.history.entries());
    paint(&mut s, json!({})).unwrap();
    assert!(close(rgba(&s, 3, 3), at_open), "a click still restores the opened image: {:?} vs {at_open:?}", rgba(&s, 3, 3));
}

#[test]
fn deleting_the_open_snapshot_leaves_the_default_without_a_source() {
    let mut s = session();
    s.execute("history.deleteSnapshot", json!({"snapshot": 0})).unwrap();
    assert!(photocraft_engine::inspect::document(s.active().unwrap())["historySource"].is_null());
    let before = rgba(&s, 3, 3);
    let e = paint(&mut s, json!({})).unwrap_err().to_string();
    assert!(e.contains("source was deleted"), "{e}");
    assert_eq!(rgba(&s, 3, 3), before, "nothing painted");
    // Setting a source makes it paint again: a snapshot of the red document.
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let snap = s.execute("history.newSnapshot", json!({"name": "Red"})).unwrap()["snapshot"].clone();
    s.execute("history.setSource", json!({"snapshot": snap})).unwrap();
    paint(&mut s, json!({})).unwrap();
    assert!(rgba(&s, 3, 3)[0] > 0.9, "{:?}", rgba(&s, 3, 3));
}

/// With Automatically Create First Snapshot off there is no open snapshot: the default keeps
/// today's behaviour and paints the oldest state held.
#[test]
fn without_the_open_snapshot_the_default_still_paints_the_oldest_state() {
    let mut s = Session::new();
    s.execute("history.options", json!({"automaticallyCreateFirstSnapshot": false})).unwrap();
    s.execute("file.new", json!({"width": 8, "height": 8, "depth": 16, "background": "white"})).unwrap();
    // Tidy-up only: preferences belong to the session, so this does not reach other tests; it
    // leaves the option as the default it started from.
    let restore = |s: &mut Session| s.execute("history.options", json!({"automaticallyCreateFirstSnapshot": true})).unwrap();
    assert!(s.active().unwrap().snapshots().is_empty());
    let opened = rgba(&s, 3, 3);
    s.execute("paint.stroke", json!({"points": [[0, 3], [7, 3]], "size": 8, "hardness": 100, "color": "#ff0000"})).unwrap();
    paint(&mut s, json!({})).unwrap();
    assert!(close(rgba(&s, 3, 3), opened), "the oldest state (the opening state) is the source");
    assert_eq!(photocraft_engine::inspect::document(s.active().unwrap())["historySource"], json!({"state": 0, "label": "New"}));
    restore(&mut s);
}

/// The Art History Brush shares the document's one source, so its default is the open snapshot too.
/// The history has filled up here, so the oldest state held would paint a colour instead. (The
/// brush itself is 8-bit only, so this document is 8-bit.)
#[test]
fn the_art_history_brush_default_source_is_the_open_snapshot() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "depth": 8, "background": "white"})).unwrap();
    let at_open = opened_pixel(&s);
    s.active_mut().unwrap().history.max_states = 2;
    for colour in ["#ff0000", "#00ff00", "#0000ff"] {
        s.execute("paint.stroke", json!({"points": [[0, 3], [7, 3]], "size": 8, "hardness": 100, "color": colour})).unwrap();
    }
    assert!(!close(rgba(&s, 3, 3), at_open), "the canvas holds the last stroke");
    s.execute("paint.artHistoryBrush", json!({"points": [[3, 3]], "size": 40, "hardness": 100})).unwrap();
    assert!(close(rgba(&s, 3, 3), at_open), "the open snapshot, not the trimmed row 0: {:?}", rgba(&s, 3, 3));
}

/// Fill > History reads the same source: with the history full its default fills the opened image
/// back, where the oldest state held would fill a colour.
#[test]
fn fill_from_history_uses_the_open_snapshot_after_the_history_fills_up() {
    let mut s = session();
    let at_open = opened_pixel(&s);
    s.active_mut().unwrap().history.max_states = 2;
    for colour in ["#ff0000", "#00ff00", "#0000ff"] {
        s.execute("edit.fill", json!({"color": colour})).unwrap();
    }
    let st = s.active().unwrap();
    assert!(st.history.entries().first().is_some_and(|label| label == "Fill"), "row 0 is a fill state: {:?}", st.history.entries());
    s.execute("edit.fill", json!({"contents": "history"})).unwrap();
    assert!(close(rgba(&s, 3, 3), at_open), "the opened image comes back: {:?}", rgba(&s, 3, 3));
}

/// `history.newSnapshot` with `replaceExisting` drops the open snapshot by name, and snapshot ids
/// are never reused: the default is left empty and the brush refuses, as after a delete (#12845).
#[test]
fn a_replacement_snapshot_named_after_the_document_leaves_the_brush_without_a_source() {
    let mut s = session();
    let name = s.active().unwrap().doc.name.clone();
    s.execute("paint.stroke", json!({"points": [[0, 3], [7, 3]], "size": 8, "hardness": 100, "color": "#ff0000"})).unwrap();
    let fresh = s.execute("history.newSnapshot", json!({"name": name, "replaceExisting": true})).unwrap()["snapshot"].as_u64().unwrap();
    assert_ne!(fresh, 0, "a fresh id, so the new row is not the open snapshot");
    assert!(photocraft_engine::inspect::document(s.active().unwrap())["historySource"].is_null());
    let before = rgba(&s, 3, 3);
    let e = paint(&mut s, json!({})).unwrap_err().to_string();
    assert!(e.contains("source was deleted"), "{e}");
    assert_eq!(rgba(&s, 3, 3), before, "nothing painted");
    // Picking that snapshot as the source paints again.
    s.execute("history.setSource", json!({"snapshot": fresh})).unwrap();
    paint(&mut s, json!({})).unwrap();
    assert!(close(rgba(&s, 3, 3), before), "the picked snapshot holds the red stroke: {:?}", rgba(&s, 3, 3));
}
