use super::*;
use std::sync::Arc;

const COMMAND: &str = "edit.findAndReplaceText";

#[test]
fn find_replace_without_type_is_enabled_and_leaves_the_document_unchanged() {
    for depth in [8, 16, 32] {
        let mut s = crate::session();
        s.execute("file.new", json!({"width": 8, "height": 8, "depth": depth})).unwrap();
        let before = s.active().unwrap().doc.clone();
        let revision = s.active().unwrap().revision;
        let history = s.active().unwrap().history.entries();
        assert!(s.is_enabled(COMMAND), "{depth}-bit document without type");
        for action in ["changeAll", "find", "change", "changeFind"] {
            let reply = s.execute(COMMAND, json!({"find": "word", "replace": "other", "action": action})).unwrap();
            match action {
                "changeAll" => assert_eq!(reply, json!({"count": 0, "layers": 0})),
                "change" => assert_eq!(reply, json!({"changed": null})),
                _ => assert_eq!(reply, json!({"found": null, "changed": null})),
            }
            let st = s.active().unwrap();
            assert!(Arc::ptr_eq(&before, &st.doc));
            assert_eq!(st.revision, revision);
            assert_eq!(st.history.entries(), history);
        }
        assert!(s.execute(COMMAND, json!({})).is_err(), "the command still requires a search term");
    }
}

#[test]
fn find_replace_needs_a_document_and_preserves_locked_type_refusal() {
    let mut s = crate::session();
    assert!(!s.is_enabled(COMMAND));
    assert!(s.execute(COMMAND, json!({"find": "word"})).is_err());
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let layer = s.execute("type.create", json!({"text": "word", "size": 8})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.setProps", json!({"layer": layer, "locked": true})).unwrap();
    assert!(!s.is_enabled(COMMAND));
    let before = s.active().unwrap().doc.clone();
    assert!(s.execute(COMMAND, json!({"find": "word", "replace": "other"})).is_err());
    assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));

    s.execute("layer.setProps", json!({"layer": layer, "locked": false})).unwrap();
    assert!(s.is_enabled(COMMAND));
    assert_eq!(s.execute(COMMAND, json!({"find": "word", "replace": "other"})).unwrap(), json!({"count": 1, "layers": 1}));
    assert!(s.undo());
    let content = &s.active().unwrap().doc.layer(LayerId(layer)).unwrap().content;
    assert!(matches!(content, LayerContent::Text(text) if text.text == "word"));
}

#[test]
fn find_replace_empty_document_without_an_active_layer_is_a_noop() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let st = s.active_mut().unwrap();
    Arc::make_mut(st.doc_mut()).layers.clear();
    st.set_active_layer(None);
    st.selection_mut().clear();
    assert!(s.is_enabled(COMMAND));
    assert_eq!(s.execute(COMMAND, json!({"find": "word", "allLayers": false})).unwrap(), json!({"count": 0, "layers": 0}));
}
