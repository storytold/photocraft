//! #14597: a duplicate starts with its own history baseline and must be saved before closing.

use serde_json::{Value, json};
use std::sync::Arc;

use crate::Session;

fn layered_source(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "name": "oracle", "background": "white", "depth": depth})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Paint"})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s
}

fn open_png(s: &mut Session) {
    let img = photocraft_codecs::Image::from_u8(8, 8, photocraft_codecs::ChannelLayout::Rgb, vec![128; 8 * 8 * 3]).unwrap();
    let bytes = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    crate::file_cmds::open_bytes_as(s, "oracle.png", &bytes, None, Some("oracle.png".into())).unwrap();
}

fn history(s: &Session) -> Value {
    crate::inspect::document(s.active().unwrap())
}

#[test]
fn layered_and_merged_copies_have_one_duplicate_state_under_their_named_snapshot() {
    for depth in [8, 16, 32] {
        for merged in [false, true] {
            for name in [None, Some("My Dup")] {
                let mut s = layered_source(depth);
                let mut params = json!({"mergedOnly": merged});
                if let Some(name) = name {
                    params["name"] = json!(name);
                }
                s.execute("image.duplicate", params).unwrap();
                let copy = history(&s);
                assert_eq!(copy["history"], json!(["Duplicate Image"]), "depth={depth}, merged={merged}, name={name:?}");
                assert_eq!(copy["snapshots"], json!([{"id": 0, "name": name.unwrap_or("oracle copy")}]));
                assert_eq!(copy["canUndo"], false);
                assert_eq!(copy["canRedo"], false);
                assert_eq!(copy["historySource"], json!({"state": 0, "label": "Duplicate Image"}));
                assert_eq!(s.active().unwrap().doc.layers.len(), if merged { 1 } else { 2 });
            }
        }
    }
}

#[test]
fn unsaved_copies_refuse_ask_but_allow_explicit_discard() {
    for depth in [8, 16, 32] {
        for merged in [false, true] {
            let mut s = layered_source(depth);
            let source = s.active().unwrap().doc.clone();
            s.execute("image.duplicate", json!({"mergedOnly": merged})).unwrap();
            let copy = s.active().unwrap();
            let id = copy.doc.id;
            assert!(copy.is_dirty(), "depth={depth}, merged={merged}");
            assert!(copy.path.is_none());
            assert_eq!(crate::inspect::session(&s)["documents"][1]["dirty"], true);
            let err = s.execute("file.close", json!({"save": "ask"})).unwrap_err();
            assert!(matches!(err, crate::EngineError::BadParams { .. }), "{err}");
            assert!(err.to_string().contains("unsaved changes"), "{err}");
            assert_eq!(s.documents().len(), 2);
            assert_eq!(s.active().unwrap().doc.id, id);
            assert!(s.active().unwrap().is_dirty());
            s.execute("file.close", json!({"save": "discard"})).unwrap();
            assert_eq!(s.documents().len(), 1);
            assert!(Arc::ptr_eq(&s.active().unwrap().doc, &source));
        }
    }
}

#[test]
fn duplicating_leaves_clean_and_dirty_sources_and_their_history_unchanged() {
    for dirty in [false, true] {
        for merged in [false, true] {
            let mut s = Session::new();
            open_png(&mut s);
            if dirty {
                s.execute("layer.new.layer", json!({})).unwrap();
            }
            let source = s.active().unwrap();
            assert_eq!(source.is_dirty(), dirty);
            let before = crate::inspect::document(source);
            let doc = source.doc.clone();
            let revisions = (source.revision, source.saved_revision);
            s.execute("image.duplicate", json!({"mergedOnly": merged})).unwrap();
            let source = &s.documents()[0];
            assert_eq!(crate::inspect::document(source), before);
            assert!(Arc::ptr_eq(&source.doc, &doc));
            assert_eq!((source.revision, source.saved_revision), revisions);
            assert_eq!(source.is_dirty(), dirty);
        }
    }
}

#[test]
fn new_and_opened_documents_keep_their_clean_initial_states_next_to_a_duplicate() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    open_png(&mut s);
    s.execute("image.duplicate", json!({})).unwrap();
    for (source, label) in s.documents()[..2].iter().zip(["New", "Open"]) {
        assert_eq!(source.history.entries(), [label]);
        assert!(!source.is_dirty());
    }
    assert_eq!(s.documents()[1].path.as_deref(), Some("oracle.png"));
}

#[test]
fn editing_undoing_and_redoing_a_copy_preserves_its_unsaved_duplicate_baseline() {
    for merged in [false, true] {
        let mut s = layered_source(8);
        s.execute("image.duplicate", json!({"mergedOnly": merged})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        assert_eq!(history(&s)["history"], json!(["Duplicate Image", "New Layer"]));
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(history(&s)["history"], json!(["Duplicate Image"]));
        assert!(s.active().unwrap().is_dirty());
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(history(&s)["history"], json!(["Duplicate Image", "New Layer"]));
        assert!(s.active().unwrap().is_dirty());
    }
}

#[test]
fn saving_a_duplicate_clears_its_dirty_mark_and_allows_ask_to_close() {
    let dir = tempfile::tempdir().unwrap();
    for merged in [false, true] {
        let path = dir.path().join(format!("copy-{merged}.pcraft"));
        let mut s = layered_source(8);
        s.execute("image.duplicate", json!({"mergedOnly": merged})).unwrap();
        assert!(s.active().unwrap().is_dirty());
        s.execute("file.saveAs", json!({"path": path})).unwrap();
        assert!(path.is_file());
        assert!(!s.active().unwrap().is_dirty());
        assert_eq!(s.active().unwrap().path.as_deref(), path.to_str());
        assert_eq!(history(&s)["history"], json!(["Duplicate Image"]));
        s.execute("file.close", json!({"save": "ask"})).unwrap();
        assert_eq!(s.documents().len(), 1);
    }
}
