//! #14596: a full Image › Duplicate copy starts with the copy of the source's active layer active
//! (and selected), also inside a group, as the reference application does; a merged copy's
//! active layer is its single layer.

use photocraft_doc::{Document, LayerId};
use serde_json::{Value, json};

use crate::Session;

fn id(v: Value) -> LayerId {
    LayerId(v["layer"].as_u64().expect("the command names its layer"))
}

/// The issue's test document: Background, "Top", "Hidden" (hidden) and the group "Grp" holding
/// the layer "Inner" and the Levels adjustment "Levels 1" above it. Returns the session and the
/// source's "Levels 1", "Top" and "Grp" ids.
fn source() -> (Session, LayerId, LayerId, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"name": "dupspec", "width": 40, "height": 30, "background": "white"})).unwrap();
    let top = id(s.execute("layer.new.layer", json!({"name": "Top"})).unwrap());
    s.execute("layer.new.layer", json!({"name": "Hidden"})).unwrap();
    s.execute("layer.setProps", json!({"visible": false})).unwrap();
    let inner = id(s.execute("layer.new.layer", json!({"name": "Inner"})).unwrap());
    s.execute("layer.newAdjustmentLayer.levels", json!({"name": "Levels 1"})).unwrap();
    let levels = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.select", json!({"layer": inner.0})).unwrap();
    s.execute("layer.select", json!({"layer": levels.0, "mode": "add"})).unwrap();
    let grp = id(s.execute("layer.new.groupFromLayers", json!({"name": "Grp"})).unwrap());
    s.execute("layer.select", json!({"layer": levels.0})).unwrap();
    assert_eq!(s.active().unwrap().active_layer, Some(levels), "Levels 1 is active in the source");
    (s, levels, top, grp)
}

/// The name of `id` in `doc` and the name of the group holding it (`None` at the top level).
fn place(doc: &Document, id: LayerId) -> (String, Option<String>) {
    let walk = doc.walk();
    let (path, _, layer) = walk.iter().find(|(_, _, l)| l.id == id).expect("the layer is in the document");
    let parent = (path.len() > 1).then(|| walk.iter().find(|(p, _, _)| p.as_slice() == &path[..path.len() - 1]).map(|(_, _, l)| l.name.clone())).flatten();
    (layer.name.clone(), parent)
}

#[test]
fn a_full_copy_starts_on_the_copy_of_the_source_active_layer_inside_its_group() {
    let (mut s, levels, _, _) = source();
    s.execute("image.duplicate", json!({})).unwrap();
    assert_eq!(s.active_index(), Some(1), "the copy is active");
    let st = s.active().unwrap();
    let active = st.active_layer.expect("the copy has an active layer");
    assert_ne!(active, levels, "the copy's layers have their own ids");
    assert_eq!(place(&st.doc, active), ("Levels 1".to_string(), Some("Grp".to_string())));
    assert_eq!(st.selected_layers(), vec![active], "the active layer is the only selected one");
    // The source keeps its own target.
    assert_eq!(s.documents()[0].active_layer, Some(levels));
}

#[test]
fn a_full_copy_of_a_top_level_active_layer_starts_on_its_copy() {
    let (mut s, _, top, _) = source();
    s.execute("layer.select", json!({"layer": top.0})).unwrap();
    s.execute("image.duplicate", json!({})).unwrap();
    let st = s.active().unwrap();
    let active = st.active_layer.expect("the copy has an active layer");
    assert_eq!(place(&st.doc, active), ("Top".to_string(), None));
}

#[test]
fn a_merged_copy_starts_on_its_single_layer() {
    let (mut s, _, _, _) = source();
    s.execute("image.duplicate", json!({"mergedOnly": true})).unwrap();
    let st = s.active().unwrap();
    assert_eq!(st.doc.layers.len(), 1);
    assert_eq!(st.active_layer, Some(st.doc.layers[0].id));
}

#[test]
fn the_first_snapshot_of_the_copy_targets_the_same_layer() {
    let (mut s, _, _, _) = source();
    s.execute("image.duplicate", json!({})).unwrap();
    let st = s.active().unwrap();
    let active = st.active_layer;
    assert_eq!(active.map(|id| place(&st.doc, id).0).as_deref(), Some("Levels 1"), "the copy starts on Levels 1");
    let first = st.snapshots.first().expect("the copy has its first snapshot");
    assert_eq!(first.layers.active, active, "going back to the first snapshot keeps the active layer");
    // An edit in the copy, then undo, comes back to the same target.
    s.execute("layer.new.layer", json!({"name": "Later"})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().active_layer, active, "undo restores the copied active layer");
}

#[test]
fn several_selected_layers_leave_the_copy_on_its_top_layer() {
    // Not measured (#14596), so today's behaviour: the copy starts on its top layer alone.
    let (mut s, levels, top, _) = source();
    s.execute("layer.select", json!({"layer": top.0})).unwrap();
    s.execute("layer.select", json!({"layer": levels.0, "mode": "add"})).unwrap();
    let st = s.active().unwrap();
    assert_eq!((st.active_layer, st.selected_layers().len()), (Some(levels), 2), "Top and Levels 1 selected, Levels 1 active");
    s.execute("image.duplicate", json!({})).unwrap();
    let st = s.active().unwrap();
    let active = st.active_layer.expect("the copy has an active layer");
    assert_eq!(place(&st.doc, active), ("Grp".to_string(), None), "the copy's top layer");
    assert_eq!(st.selected_layers(), vec![active]);
}
