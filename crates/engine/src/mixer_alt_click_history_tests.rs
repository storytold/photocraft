//! A Mixer Brush ⌥-click load (`brush.mixer.sample`) adds one "Mixer Brush" history state and
//! leaves the pixels as they are, as the reference's does (#11954); Load Brush and Clean Brush add
//! none, and a failed sample adds none and returns an error.

use crate::Session;
use serde_json::{Value, json};

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 32, "height": 48})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": [0.86, 0.12, 0.12, 1.0]})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("tools.setColors", json!({"foreground": "#000000"})).unwrap();
    s
}

fn steps(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

fn pixels(s: &mut Session) -> Vec<Value> {
    (0..48)
        .step_by(4)
        .flat_map(|y| (0..64).step_by(4).map(move |x| (x, y)))
        .map(|(x, y)| s.execute("document.pixel", json!({"x": x, "y": y})).unwrap())
        .collect()
}

fn sample(s: &mut Session, x: f64, y: f64) -> crate::Result<Value> {
    s.execute("brush.mixer.sample", json!({"points": [[x, y]], "size": 12}))
}

#[test]
fn an_alt_click_load_adds_one_mixer_brush_state_and_keeps_the_pixels() {
    let mut s = session();
    let (before, px) = (steps(&s), pixels(&mut s));
    sample(&mut s, 16.0, 24.0).unwrap();
    assert!(s.tools.mixer.spatial.is_some(), "the brush is loaded");
    assert_eq!(steps(&s), before + 1);
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Mixer Brush"));
    assert_eq!(pixels(&mut s), px, "the pixels are unchanged");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(steps(&s), before, "Undo removes the state");
    assert_eq!(pixels(&mut s), px);
}

#[test]
fn load_brush_and_clean_brush_add_no_state() {
    let mut s = session();
    let before = steps(&s);
    s.execute("brush.mixer.load", json!({})).unwrap();
    s.execute("brush.mixer.clean", json!({})).unwrap();
    assert_eq!(steps(&s), before);
}

#[test]
fn a_failed_sample_adds_no_state() {
    // No document.
    assert!(Session::new().execute("brush.mixer.sample", json!({"points": [[4, 4]]})).is_err());
    let mut s = session();
    let before = steps(&s);
    // Bad params, a footprint too large, and a transparent area (nothing to load).
    assert!(s.execute("brush.mixer.sample", json!({"points": [[4, 4], [8, 8]]})).is_err());
    assert!(s.execute("brush.mixer.sample", json!({"points": [[4, 4]], "size": 2000})).is_err());
    assert!(sample(&mut s, 50.0, 24.0).is_err());
    assert_eq!(steps(&s), before);
}

/// A load while a moved selection floats comes after the move in History: the move is dropped
/// as its own state first, so the first Undo removes the "Mixer Brush" state and keeps the moved
/// pixels, and the second undoes the move (CodeRabbit review on mixer_live.rs:455).
#[test]
fn a_load_over_a_floating_selection_is_undone_before_the_move() {
    let mut s = session();
    let before = steps(&s);
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 32, "height": 48})).unwrap();
    s.execute("select.float", json!({"dx": 8, "dy": 0})).unwrap();
    sample(&mut s, 16.0, 24.0).unwrap();
    let st = s.active().unwrap();
    assert_eq!(st.history.past_len(), before + 3, "the selection, the move and the load");
    assert_eq!(st.history.undo_label(), Some("Mixer Brush"));
    let moved = pixels(&mut s);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(steps(&s), before + 2, "Undo removes the load's state first");
    assert_eq!(pixels(&mut s), moved, "the moved pixels stay");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(steps(&s), before + 1, "the next Undo removes the move");
}
