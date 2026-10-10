//! Delete Layer may leave an empty document. Single-layer and group deletion must reset the
//! layer target and remain undoable, including calls with an explicit layer parameter.

use photocraft_engine::Session;
use serde_json::json;

fn one_layer_doc() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 10, "height": 5, "background": "transparent"})).unwrap();
    s
}

#[test]
fn deleting_the_only_layer_is_undoable() {
    let mut s = one_layer_doc();
    let id = s.active().unwrap().doc.layers[0].id;
    s.execute("layer.delete", json!({"layer": id.0})).unwrap();
    let st = s.active().unwrap();
    assert!(st.doc.layers.is_empty());
    assert_eq!(st.active_layer, None);
    assert!(st.selected_layers().is_empty());
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc.layers.len(), 1);
    assert_eq!(s.active().unwrap().active_layer, Some(id));
}

#[test]
fn deleting_the_only_layer_without_a_param_is_undoable() {
    let mut s = one_layer_doc();
    s.execute("layer.delete", json!({})).unwrap();
    assert!(s.active().unwrap().doc.layers.is_empty());
    assert_eq!(s.active().unwrap().active_layer, None);
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc.layers.len(), 1);
}

#[test]
fn deleting_a_group_that_holds_every_layer_is_undoable() {
    let mut s = one_layer_doc();
    let r = s.execute("layer.groupLayers", json!({})).unwrap();
    let group = r["layer"].as_u64().unwrap_or_else(|| s.active().unwrap().active_layer.unwrap().0);
    assert_eq!(s.active().unwrap().doc.layers.len(), 1, "the group should be the only top-level layer");
    s.execute("layer.delete", json!({"layer": group})).unwrap();
    assert!(s.active().unwrap().doc.layers.is_empty());
    assert_eq!(s.active().unwrap().active_layer, None);
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc.layers.len(), 1);
    assert_eq!(s.active().unwrap().active_layer.unwrap().0, group);
    assert_eq!(s.active().unwrap().doc.layer_count(), 2, "undo restores the group's child too");
}

#[test]
fn deleting_one_of_two_layers_still_works() {
    let mut s = one_layer_doc();
    s.execute("layer.new.layer", json!({})).unwrap();
    let top = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.delete", json!({"layer": top.0})).unwrap();
    let st = s.active().unwrap();
    assert_eq!(st.doc.layers.len(), 1);
    assert!(st.doc.layer(top).is_none());
    assert_eq!(st.active_layer, Some(st.doc.layers[0].id));
}
