//! #14251: Edit › Content-Aware Fill is unavailable on a hidden layer and in Indexed Color. With
//! Lock All on the target it stays available, and running it fails with the reference's text,
//! changing nothing. 16-bit, Grayscale and RGB documents keep filling; Delete and Fill Selection
//! keeps today's rules.

use serde_json::{Value, json};

use crate::Session;

const CAF: &str = "edit.contentAwareFill";
const LOCKED: &str = "Could not complete the command because the layer is locked.";

/// A 48 × 32 document of `new` with a striped pixel layer "Paint" and a selection over its middle.
fn session(new: Value) -> Session {
    let mut s = Session::new();
    s.execute("file.new", new).unwrap();
    s.execute("layer.new.layer", json!({"name": "Paint"})).unwrap();
    for x in (0..48).step_by(8) {
        s.execute("select.rect", json!({"x": x, "y": 0, "width": 4, "height": 32})).unwrap();
        s.execute("edit.fill", json!({"color": "#3a6ea5"})).unwrap();
    }
    s.execute("select.rect", json!({"x": 18, "y": 10, "width": 10, "height": 10})).unwrap();
    s
}

fn rgb() -> Session {
    session(json!({"width": 48, "height": 32, "background": "white"}))
}

/// The History length and the active layer's pixels over the selection, to tell that nothing changed.
fn state(s: &Session) -> (usize, Vec<[f32; 4]>) {
    let st = s.active().unwrap();
    let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
    let px = layer.surface().map(|sf| (10..20).flat_map(|y| (18..28).map(move |x| sf.rgba(x, y))).collect()).unwrap_or_default();
    (st.history.past_len(), px)
}

#[test]
fn a_hidden_layer_greys_it_and_running_it_changes_nothing() {
    let mut s = rgb();
    s.execute("layer.setProps", json!({"layer": "Paint", "visible": false})).unwrap();
    assert!(!s.is_enabled(CAF));
    let before = state(&s);
    assert!(s.execute(CAF, json!({})).is_err());
    assert_eq!(state(&s), before);
}

#[test]
fn an_indexed_color_document_greys_it() {
    let mut s = rgb();
    s.execute("layer.flattenImage", json!({})).unwrap();
    s.execute("image.mode.indexedColor", json!({"palette": "exact", "forced": "none", "dither": "none", "transparency": false})).unwrap();
    s.execute("select.rect", json!({"x": 18, "y": 10, "width": 10, "height": 10})).unwrap();
    assert!(!s.is_enabled(CAF));
}

#[test]
fn lock_all_keeps_it_available_and_running_it_says_the_layer_is_locked() {
    let mut s = rgb();
    s.execute("layer.lockLayers", json!({"all": true})).unwrap();
    assert!(s.is_enabled(CAF), "the menu item stays enabled");
    let before = state(&s);
    let err = s.execute(CAF, json!({})).unwrap_err();
    assert_eq!(err.to_string(), LOCKED);
    assert_eq!(state(&s), before, "no History state, the pixels unchanged");
}

#[test]
fn sixteen_bit_grayscale_and_rgb_documents_still_fill() {
    for new in [
        json!({"width": 48, "height": 32, "background": "white"}),
        json!({"width": 48, "height": 32, "background": "white", "depth": 16}),
        json!({"width": 48, "height": 32, "background": "white", "mode": "grayscale"}),
    ] {
        let mut s = session(new.clone());
        assert!(s.is_enabled(CAF), "{new}");
        let before = state(&s).0;
        s.execute(CAF, json!({})).unwrap();
        assert_eq!(state(&s).0, before + 1, "{new}: one History state");
    }
}

#[test]
fn delete_and_fill_selection_keeps_todays_rules() {
    let mut s = rgb();
    s.execute("layer.setProps", json!({"layer": "Paint", "visible": false})).unwrap();
    assert!(s.is_enabled("edit.deleteAndFillSelection"), "a hidden layer: as today");
    s.execute("layer.setProps", json!({"layer": "Paint", "visible": true})).unwrap();
    s.execute("layer.lockLayers", json!({"all": true})).unwrap();
    assert!(!s.is_enabled("edit.deleteAndFillSelection"), "Lock All: as today");
}
