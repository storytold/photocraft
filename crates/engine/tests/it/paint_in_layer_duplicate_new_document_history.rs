//! Tests that run paint commands, moved from the engine-core `layer_duplicate_new_document_history` tests: 1 test(s).

use serde_json::json;

use photocraft_engine::{DocState, Session};

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
fn duplicate_to_a_new_document_undoes_one_layer_at_each_depth() {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 30, "depth": depth})).unwrap();
        let layer = s.execute("layer.new.layer", json!({"name": "Paint"})).unwrap()["layer"].clone();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        let source = s.active().unwrap().clone();

        let r = s.execute("layer.duplicate", json!({"document": "new", "layer": layer, "name": "Ink"})).unwrap();
        assert_eq!(r["document"], 1);
        assert_eq!(s.active_index(), Some(1));
        let dst = s.active().unwrap();
        assert_eq!(dst.doc.layers.len(), 1);
        assert_eq!(dst.doc.layers[0].name, "Ink");
        assert_eq!(dst.snapshots().len(), 1);
        assert_eq!(dst.snapshots()[0].name, dst.doc.name);
        assert_eq!(*dst.snapshots()[0].doc, *dst.doc);
        check_duplicate_history(&mut s, &source);
    }
}
