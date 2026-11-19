//! Duplicating several layers adds one history step named "Duplicate Layer", as for one layer
//! (#14611), in place and into another document.

use crate::Session;
use photocraft_doc::LayerId;
use serde_json::json;

/// A 32 x 32 document with the Background and three empty pixel layers "A", "B" and "C".
fn session() -> (Session, [LayerId; 3]) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 32, "height": 32, "background": "white"})).unwrap();
    let ids = ["A", "B", "C"].map(|name| {
        let id = s.execute("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap();
        LayerId(id)
    });
    (s, ids)
}

fn steps(s: &Session) -> Vec<String> {
    s.active().unwrap().history.entries()
}

fn assert_one_duplicate_layer_step(before: &[String], after: &[String]) {
    assert_eq!(after.len(), before.len() + 1, "exactly one step: {after:?}");
    assert_eq!(after.last().map(String::as_str), Some("Duplicate Layer"), "{after:?}");
}

#[test]
fn duplicating_several_selected_layers_is_one_duplicate_layer_step() {
    let (mut s, [a, _, c]) = session();
    // Two non-adjacent layers, as measured.
    s.active_mut().unwrap().selected_layers = vec![a, c];
    let before = steps(&s);
    s.execute("layer.duplicate", json!({})).unwrap();
    assert_one_duplicate_layer_step(&before, &steps(&s));
}

#[test]
fn duplicating_one_layer_or_a_group_stays_one_duplicate_layer_step() {
    let (mut s, [a, b, _]) = session();
    let before = steps(&s);
    s.execute("layer.duplicate", json!({"layer": a.0})).unwrap();
    assert_one_duplicate_layer_step(&before, &steps(&s));

    s.active_mut().unwrap().selected_layers = vec![b];
    s.execute("layer.groupLayers", json!({})).unwrap();
    let group = s.active().unwrap().active_layer.unwrap();
    assert!(s.active().unwrap().doc.layer(group).unwrap().children().is_some(), "a group is active");
    let before = steps(&s);
    s.execute("layer.duplicate", json!({"layer": group.0})).unwrap();
    assert_one_duplicate_layer_step(&before, &steps(&s));
}

#[test]
fn copying_several_layers_into_another_document_is_one_duplicate_layer_step() {
    let (mut s, [a, b, _]) = session();
    s.execute("file.new", json!({"width": 32, "height": 32, "background": "white"})).unwrap();
    let dest = s.documents().len() - 1;
    let before = s.documents()[dest].history.entries();
    s.set_active(0);
    s.execute("layer.copyToDocument", json!({"document": dest, "layers": [a.0, b.0]})).unwrap();
    assert_one_duplicate_layer_step(&before, &s.documents()[dest].history.entries());
}
