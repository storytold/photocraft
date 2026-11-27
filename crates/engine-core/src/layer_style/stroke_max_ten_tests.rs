//! #15140: `layer.layerStyle.stroke {"add": true}` stops at ten strokes, like the Layer Style
//! dialog. Editing a stroke still works on a layer with ten or more, and a layer that already
//! holds more (from a file or a script) keeps them all.

use super::*;
use serde_json::json;

fn session() -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s
}

fn effects(s: &Session) -> Vec<Effect> {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().effects.items.clone()
}

fn strokes(s: &Session) -> Vec<f32> {
    effects(s)
        .iter()
        .filter_map(|e| match e {
            Effect::Stroke(st) => Some(st.size),
            _ => None,
        })
        .collect()
}

fn add_strokes(s: &mut Session, n: usize) {
    for i in 0..n {
        s.execute("layer.layerStyle.stroke", json!({"size": i + 1, "add": true})).unwrap();
    }
}

#[test]
fn adding_an_eleventh_stroke_is_refused_and_leaves_the_layer_unchanged() {
    let mut s = session();
    add_strokes(&mut s, MAX_STROKES);
    assert_eq!(strokes(&s).len(), 10);
    let before = effects(&s);
    let doc = std::sync::Arc::clone(&s.active().unwrap().doc);

    let err = s.execute("layer.layerStyle.stroke", json!({"size": 11, "add": true}));
    assert!(err.is_err(), "an 11th stroke must be refused: {err:?}");
    assert_eq!(effects(&s), before);
    assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &doc), "a refused add leaves the document as it was");
}

#[test]
fn other_effects_can_still_be_added_next_to_ten_strokes() {
    let mut s = session();
    add_strokes(&mut s, MAX_STROKES);
    s.execute("layer.layerStyle.dropShadow", json!({"add": true})).unwrap();
    assert_eq!(effects(&s).len(), 11);
}

#[test]
fn editing_a_stroke_still_works_with_ten_strokes() {
    let mut s = session();
    add_strokes(&mut s, MAX_STROKES);
    s.execute("layer.layerStyle.stroke", json!({"size": 42})).unwrap();
    let sizes = strokes(&s);
    assert_eq!(sizes.len(), 10);
    assert_eq!(sizes.first(), Some(&42.0));
}

#[test]
fn a_layer_with_more_than_ten_strokes_keeps_them_and_can_still_edit_one() {
    let mut s = session();
    // A file or a script can carry more than ten strokes; they all stay.
    let id = s.active().unwrap().active_layer.unwrap();
    s.edit("Load", |doc, _| {
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let one = effect_from_params("stroke", &json!({"size": 1})).unwrap();
        l.effects.items = vec![one; 12];
        Ok(())
    })
    .unwrap();
    assert_eq!(strokes(&s).len(), 12);

    s.execute("layer.layerStyle.stroke", json!({"size": 9})).unwrap();
    let sizes = strokes(&s);
    assert_eq!(sizes.len(), 12);
    assert_eq!(sizes.first(), Some(&9.0));
    assert!(s.execute("layer.layerStyle.stroke", json!({"size": 3, "add": true})).is_err());
    assert_eq!(strokes(&s).len(), 12);
}
