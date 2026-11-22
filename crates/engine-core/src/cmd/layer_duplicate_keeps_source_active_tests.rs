//! Layer › Duplicate Layer… into another open document leaves the source document active, as in
//! Photoshop (#14605); dragging layers onto another document's tab still shows the destination.
use photocraft_doc::LayerId;
use serde_json::{Value, json};

use crate::Session;

/// "Source" (Background, "Ink", "Paper"; "Paper" active, a pixel selection) and "Target" (just a
/// Background); "Source" is active.
fn two_docs() -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 40, "height": 30, "name": "Source"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Ink"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Paper"})).unwrap();
    s.execute("select.rect", json!({"x": 2, "y": 3, "width": 5, "height": 4})).unwrap();
    s.execute("file.new", json!({"width": 20, "height": 10, "name": "Target"})).unwrap();
    s.set_active(0);
    s
}

/// What Duplicate Layer… must leave alone in a document: its history, active and selected layers
/// and pixel selection.
fn state(s: &Session, i: usize) -> (Vec<String>, Option<LayerId>, Vec<LayerId>, bool) {
    let st = &s.documents()[i];
    (st.history.entries(), st.active_layer, st.selected_layers().to_vec(), st.doc.selection.is_some())
}

fn history(s: &Session, i: usize) -> Vec<String> {
    s.documents()[i].history.entries()
}

#[test]
fn duplicate_layer_into_another_document_keeps_the_source_active_and_unchanged() {
    let mut s = two_docs();
    let source = state(&s, 0);
    let target = history(&s, 1);
    let r = s.execute("layer.copyToDocument", json!({"document": 1, "name": "Paper copy", "activate": false})).unwrap();
    assert_eq!(s.active_index(), Some(0), "the source stays the active document");
    assert_eq!(state(&s, 0), source, "the source's history, active layer, selected layers and selection are unchanged");
    let after = history(&s, 1);
    assert_eq!(after.len(), target.len() + 1, "{after:?}");
    assert_eq!(after.last().map(String::as_str), Some("Duplicate Layer"));
    let copy = LayerId(r["layers"][0].as_u64().unwrap());
    let dest = &s.documents()[1];
    assert_eq!(dest.active_layer, Some(copy), "the copy is the destination's active layer");
    assert_eq!(dest.selected_layers(), vec![copy]);
    assert_eq!(dest.doc.layer(copy).map(|l| l.name.as_str()), Some("Paper copy"));
}

#[test]
fn several_selected_layers_keep_the_source_selection_and_land_in_one_step() {
    let mut s = two_docs();
    let ink = s.documents()[0].doc.layers[1].id;
    s.execute("layer.select", json!({"layer": ink.0, "mode": "add"})).unwrap();
    let source = state(&s, 0);
    assert_eq!(source.2.len(), 2, "Ink and Paper are selected");
    let target = history(&s, 1).len();
    let r = s.execute("layer.copyToDocument", json!({"document": 1, "activate": false})).unwrap();
    assert_eq!(s.active_index(), Some(0));
    assert_eq!(state(&s, 0), source);
    assert_eq!(history(&s, 1).len(), target + 1);
    assert_eq!(r["layers"].as_array().map(Vec::len), Some(2));
}

#[test]
fn a_drag_without_the_param_or_with_it_on_still_shows_the_destination() {
    for extra in [json!({}), json!({"activate": true})] {
        let mut s = two_docs();
        let mut p = json!({"document": 1});
        if let (Some(p), Some(extra)) = (p.as_object_mut(), extra.as_object()) {
            p.extend(extra.clone());
        }
        let r = s.execute("layer.copyToDocument", p).unwrap();
        assert_eq!(s.active_index(), Some(1), "{extra}: the destination becomes the active document");
        assert_eq!(s.active().unwrap().active_layer, r["layers"][0].as_u64().map(LayerId));
    }
}

#[test]
fn a_non_boolean_activate_is_refused_without_changing_anything() {
    for bad in [json!("no"), json!(0), Value::Null] {
        let mut s = two_docs();
        let (source, target) = (state(&s, 0), state(&s, 1));
        assert!(s.execute("layer.copyToDocument", json!({"document": 1, "activate": bad})).is_err(), "{bad}");
        assert_eq!(s.active_index(), Some(0));
        assert_eq!((state(&s, 0), state(&s, 1)), (source, target));
    }
}

#[test]
fn a_recorded_duplicate_into_another_document_replays_keeping_the_source_active() {
    let mut s = two_docs();
    s.execute("actions.record", json!({"name": "Send"})).unwrap();
    s.execute("layer.copyToDocument", json!({"document": 1, "activate": false})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    let steps = s.execute("actions.get", json!({"action": "Send"})).unwrap()["steps"].clone();
    assert_eq!(steps, json!([["layer.copyToDocument", {"document": 1, "activate": false}]]));
    let target = history(&s, 1).len();
    let played = s.execute("actions.play", json!({"action": "Send"})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert_eq!(s.active_index(), Some(0), "playback keeps the source active too");
    assert_eq!(history(&s, 1).len(), target + 1);
}

#[test]
fn copying_from_another_source_keeps_the_document_that_was_active() {
    let mut s = two_docs();
    s.execute("file.new", json!({"width": 8, "height": 8, "name": "Third"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Dot"})).unwrap();
    s.set_active(0);
    let (active, third) = (state(&s, 0), history(&s, 2));
    let target = history(&s, 1).len();
    s.execute("layer.copyToDocument", json!({"source": 2, "document": 1, "activate": false})).unwrap();
    assert_eq!(s.active_index(), Some(0), "the document active before the copy stays active, not the source");
    assert_eq!((state(&s, 0), history(&s, 2)), (active, third));
    assert_eq!(history(&s, 1).len(), target + 1);
}
