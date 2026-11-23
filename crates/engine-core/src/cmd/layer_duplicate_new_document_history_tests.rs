use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::LayerId;
use serde_json::json;

use crate::{DocState, Session};

fn check_duplicate_history(s: &mut Session, source: &DocState) {
    let dst = s.active().unwrap();
    assert!(dst.history.can_undo(), "the duplicate is undoable in the destination");
    assert_eq!(dst.history.entries(), ["Open", "Duplicate Layer"]);
    let with_copies = dst.doc.clone();
    let selected = dst.selected_layers().to_vec();
    let active = dst.active_layer;
    let mut empty = (*with_copies).clone();
    empty.layers.clear();

    assert_eq!(s.execute("edit.undo", json!({})).unwrap(), true);
    assert_eq!(s.active_index(), Some(1));
    let dst = s.active().unwrap();
    assert_eq!(*dst.doc, empty, "undo removes the copies and retains document properties");
    assert_eq!(dst.active_layer, None);
    assert!(dst.selected_layers().is_empty());
    assert!(!dst.history.can_undo());
    assert_eq!(*s.documents()[0].doc, *source.doc);
    assert_eq!(s.documents()[0].history.entries(), source.history.entries());

    assert_eq!(s.execute("edit.redo", json!({})).unwrap(), true);
    let dst = s.active().unwrap();
    assert_eq!(s.active_index(), Some(1));
    assert_eq!(*dst.doc, *with_copies);
    assert_eq!(dst.selected_layers(), selected);
    assert_eq!(dst.active_layer, active);
    assert_eq!(dst.history.entries(), ["Open", "Duplicate Layer"]);
    assert_eq!(*s.documents()[0].doc, *source.doc);
    assert_eq!(s.documents()[0].history.entries(), source.history.entries());
    assert_eq!(s.documents()[0].selected_layers(), source.selected_layers());
}

#[test]
fn duplicate_to_a_new_document_undoes_all_selected_layers_in_one_step() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Paint"})).unwrap();
    let background = s.active().unwrap().doc.layers[0].id.0;
    s.execute("layer.select", json!({"layer": background, "mode": "add"})).unwrap();
    let source = s.active().unwrap().clone();
    assert_eq!(source.selected_layers().len(), 2);

    s.execute("layer.duplicate", json!({"document": "new"})).unwrap();
    let dst = s.active().unwrap();
    let names: Vec<_> = dst.doc.layers.iter().map(|layer| layer.name.as_str()).collect();
    assert_eq!(names, ["Background", "Paint"]);
    assert_eq!(dst.selected_layers().len(), 2);
    assert_eq!(dst.snapshots().len(), 1);
    check_duplicate_history(&mut s, &source);
}

#[test]
fn duplicate_to_a_new_document_undoes_a_group_without_a_first_snapshot() {
    let mut s = crate::session();
    s.execute("history.options", json!({"automaticallyCreateFirstSnapshot": false})).unwrap();
    s.execute("file.new", json!({"width": 40, "height": 30, "mode": "gray", "depth": 16, "resolution": 300, "profile": "gray-gamma-2.2"})).unwrap();
    let child = s.execute("layer.new.layer", json!({"name": "Child"})).unwrap()["layer"].clone();
    let group = s.execute("layer.groupLayers", json!({"name": "Folder", "layer": child})).unwrap();
    let source = s.active().unwrap().clone();
    let r = s.execute("layer.duplicate", json!({"document": "new", "layer": group["layer"], "documentName": "Grayscale copy"})).unwrap();
    let dst = s.active().unwrap();
    assert!(dst.snapshots().is_empty());
    assert_eq!(dst.doc.mode, ColorMode::Grayscale);
    assert_eq!(dst.doc.depth, SampleType::U16);
    assert_eq!(dst.doc.resolution_dpi, 300.0);
    assert!(dst.doc.icc_profile.is_some());
    assert_eq!(dst.doc.icc_profile, source.doc.icc_profile);
    assert_eq!(dst.doc.layers.len(), 1);
    let copied = dst.doc.layer(LayerId(r["layer"].as_u64().unwrap())).unwrap();
    assert_eq!(copied.name, "Folder");
    let children = copied.children().unwrap();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].name, "Child");
    check_duplicate_history(&mut s, &source);
    assert!(s.active().unwrap().snapshots().is_empty());
}
