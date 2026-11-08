//! #12181: Content-Aware Move refuses an empty selection, a locked or hidden layer, and an
//! Indexed Color document with Photoshop's alert text. Nothing is painted and no History step
//! is added. Sample All Layers still writes into an empty layer.

use std::sync::Arc;

use serde_json::json;

use super::{REFUSAL_EMPTY, REFUSAL_HIDDEN, REFUSAL_INDEXED, REFUSAL_LOCKED};
use crate::Session;

fn doc() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 80, "height": 48, "background": "white"})).unwrap();
    s
}

fn steps(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

fn rgba(s: &Session, x: i32, y: i32) -> [f32; 4] {
    let st = s.active().unwrap();
    let id = st.active_layer.unwrap();
    st.doc.layer(id).unwrap().surface().unwrap().rgba(x, y)
}

/// The command returns `text` exactly, and the document (pixels and selection) and History stay.
fn assert_refused(s: &mut Session, text: &str) {
    let (doc, n) = (s.active().unwrap().doc.clone(), steps(s));
    let err = s.execute("paint.contentAwareMove", json!({"offset": [24, 0]})).unwrap_err().to_string();
    assert_eq!(err, text);
    assert!(Arc::ptr_eq(&doc, &s.active().unwrap().doc), "pixels and selection unchanged");
    assert_eq!(steps(s), n, "no history step");
}

fn select(s: &mut Session) {
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
}

#[test]
fn an_empty_layer_is_refused_unless_sample_all_layers_is_on() {
    let mut s = doc();
    s.execute("paint.pencil", json!({"points": [[16, 16]], "size": 10, "color": "#202020"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    select(&mut s);
    assert_refused(&mut s, REFUSAL_EMPTY);
    assert!(rgba(&s, 40, 16)[3] < 0.01, "the empty layer stays empty");
    // Sample All Layers reads the background and writes the move into the empty layer.
    let n = steps(&s);
    s.execute("paint.contentAwareMove", json!({"offset": [24, 0], "sampleAllLayers": true})).unwrap();
    assert_eq!(steps(&s), n + 1, "the successful move is one history step");
    assert!(rgba(&s, 40, 16)[3] > 0.5, "the moved content lands on the empty layer: {:?}", rgba(&s, 40, 16));
}

#[test]
fn a_layer_with_pixels_locked_is_refused() {
    let mut s = doc();
    s.execute("paint.pencil", json!({"points": [[16, 16]], "size": 10, "color": "#3366cc"})).unwrap();
    select(&mut s);
    s.execute("layer.setProps", json!({"locks": {"pixels": true}})).unwrap();
    assert_refused(&mut s, REFUSAL_LOCKED);
}

#[test]
fn a_hidden_target_including_the_background_is_refused() {
    let mut s = doc();
    select(&mut s);
    s.execute("layer.setProps", json!({"visible": false})).unwrap();
    assert_refused(&mut s, REFUSAL_HIDDEN);

    let mut s = doc();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("paint.pencil", json!({"points": [[16, 16]], "size": 10, "color": "#cc3333"})).unwrap();
    select(&mut s);
    s.execute("layer.setProps", json!({"visible": false})).unwrap();
    assert_refused(&mut s, REFUSAL_HIDDEN);
}

#[test]
fn an_indexed_color_document_is_refused() {
    let mut s = doc();
    s.execute("image.mode.indexedColor", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.mode, photocraft_doc::ColorMode::Indexed);
    select(&mut s);
    assert_refused(&mut s, REFUSAL_INDEXED);
}

#[test]
fn a_visible_unlocked_layer_with_pixels_still_moves() {
    let mut s = doc();
    select(&mut s);
    let n = steps(&s);
    s.execute("paint.contentAwareMove", json!({"offset": [24, 0]})).unwrap();
    assert_eq!(steps(&s), n + 1);
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Content-Aware Move"));
}

#[test]
fn other_errors_keep_their_own_text() {
    let mut s = doc();
    select(&mut s);
    for (p, snippet) in [(json!({"offset": [0, 0]}), "drag the selection"), (json!({"offset": [400, 0]}), "inside the canvas"), (json!({}), "missing `offset`")]
    {
        let err = s.execute("paint.contentAwareMove", p).unwrap_err().to_string();
        assert!(err.contains(snippet), "{err}");
        assert_ne!(err, REFUSAL_EMPTY);
        assert_ne!(err, REFUSAL_LOCKED);
        assert_ne!(err, REFUSAL_HIDDEN);
        assert_ne!(err, REFUSAL_INDEXED);
    }
}
