//! #12120: the Patch tool refuses an empty area, a locked layer, a hidden layer and an Indexed Color image with the
//! reference's alert texts, changing nothing and adding no History state. Content-Aware on an empty layer keeps
//! today's behaviour, and a normal patch on a layer with pixels still runs.

use std::sync::Arc;

use super::*;

/// A 128 × 64 RGB document: a white Background, a layer "Paint" filled with a pattern, and an empty layer "Empty"
/// on top, which is active. A 20 × 20 selection at 16,22.
fn session() -> (Session, LayerId, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 128, "height": 64, "background": "#ffffff"})).unwrap();
    let paint = LayerId(s.execute("layer.new.layer", json!({"name": "Paint"})).unwrap()["layer"].as_u64().unwrap());
    for (x, c) in [(0, "#3366cc"), (40, "#cc6633"), (80, "#33cc66")] {
        s.execute("select.rect", json!({"x": x, "y": 0, "width": 40, "height": 64})).unwrap();
        s.execute("edit.fill", json!({"contents": "color", "color": c})).unwrap();
    }
    let empty = LayerId(s.execute("layer.new.layer", json!({"name": "Empty"})).unwrap()["layer"].as_u64().unwrap());
    s.execute("select.rect", json!({"x": 16, "y": 22, "width": 20, "height": 20})).unwrap();
    (s, paint, empty)
}

fn patch(s: &mut Session) -> std::result::Result<Value, String> {
    s.execute("paint.patch", json!({"offset": [50, 0]})).map_err(|e| e.to_string())
}

/// The document and its History length, to show a refusal changed neither.
fn state(s: &Session) -> (Arc<Document>, usize) {
    let st = s.active().unwrap();
    (Arc::clone(&st.doc), st.history.past_len())
}

fn refused_with(s: &mut Session, text: &str) {
    let (doc, steps) = state(s);
    assert_eq!(patch(s).unwrap_err(), text);
    let (after, after_steps) = state(s);
    assert!(Arc::ptr_eq(&doc, &after), "nothing changed");
    assert_eq!(after_steps, steps, "no History state");
}

#[test]
fn an_empty_area_is_refused() {
    let (mut s, _, _) = session();
    refused_with(&mut s, super::EMPTY_AREA);
}

#[test]
fn a_locked_layer_is_refused() {
    let (mut s, paint, _) = session();
    s.execute("layer.select", json!({"layer": paint.0})).unwrap();
    s.execute("layer.setProps", json!({"layer": paint.0, "locks": {"pixels": true}})).unwrap();
    refused_with(&mut s, super::LOCKED);
}

#[test]
fn a_hidden_layer_is_refused() {
    let (mut s, paint, _) = session();
    s.execute("layer.select", json!({"layer": paint.0})).unwrap();
    s.execute("layer.setProps", json!({"layer": paint.0, "visible": false})).unwrap();
    refused_with(&mut s, crate::hidden_target::PATCH_HIDDEN);
}

#[test]
fn an_indexed_color_image_is_refused() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 128, "height": 64, "background": "#808080"})).unwrap();
    s.execute("image.mode.indexedColor", json!({"palette": "exact", "forced": "none", "dither": "none"})).unwrap();
    s.execute("select.rect", json!({"x": 16, "y": 22, "width": 20, "height": 20})).unwrap();
    refused_with(&mut s, super::INDEXED);
}

#[test]
fn a_layer_with_pixels_still_patches_and_content_aware_on_an_empty_layer_runs() {
    let (mut s, paint, _) = session();
    let steps = state(&s).1;
    s.execute("paint.patch", json!({"offset": [50, 0], "contentAware": true})).unwrap();
    assert_eq!(state(&s).1, steps + 1, "Content-Aware on the empty layer adds its state, as today");
    s.execute("layer.select", json!({"layer": paint.0})).unwrap();
    let steps = state(&s).1;
    patch(&mut s).unwrap();
    assert_eq!(state(&s).1, steps + 1, "a normal patch on pixels runs");
}

/// A mask or channel target keeps today's checks on an Indexed Color image: it is not the Indexed alert's business
/// (CodeRabbit on #16178).
#[test]
fn a_mask_target_on_an_indexed_image_does_not_get_the_indexed_alert() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 128, "height": 64, "background": "#808080"})).unwrap();
    s.execute("image.mode.indexedColor", json!({"palette": "exact", "forced": "none", "dither": "none"})).unwrap();
    s.execute("select.rect", json!({"x": 16, "y": 22, "width": 20, "height": 20})).unwrap();
    let e = s.execute("paint.patch", json!({"offset": [50, 0], "target": "mask"})).unwrap_err().to_string();
    assert_ne!(e, super::INDEXED, "{e}");
}
