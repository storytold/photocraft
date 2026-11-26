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
    // Preferences are process-wide: put the default back before this test's documents are gone.
    let restore = |s: &mut Session| s.execute("history.options", json!({"automaticallyCreateFirstSnapshot": true})).unwrap();
    assert!(s.active().unwrap().snapshots().is_empty());
    let opened = rgba(&s, 3, 3);
    s.execute("paint.stroke", json!({"points": [[0, 3], [7, 3]], "size": 8, "hardness": 100, "color": "#ff0000"})).unwrap();
    paint(&mut s, json!({})).unwrap();
    assert!(close(rgba(&s, 3, 3), opened), "the oldest state (the opening state) is the source");
    assert_eq!(photocraft_engine::inspect::document(s.active().unwrap())["historySource"], json!({"state": 0, "label": "New"}));
    restore(&mut s);
}
