use std::sync::Arc;

use super::*;

const CMD: &str = "image.calculations";

fn doc_of(s: &Session) -> Arc<Document> {
    s.active().unwrap().doc.clone()
}

fn new_doc() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 20, "background": "white"})).unwrap();
    s
}

#[test]
fn no_document_disables_calculations() {
    let mut s = Session::new();
    assert!(!s.is_enabled(CMD));
    let err = s.execute(CMD, json!({})).unwrap_err();
    assert!(matches!(&err, EngineError::Disabled(id, reason) if id == CMD && !reason.is_empty()), "got {err}");
}

#[test]
fn group_layer_disables_calculations_and_refuses_to_run() {
    let mut s = new_doc();
    s.execute("layer.new.layer", json!({"name": "Base"})).unwrap();
    s.execute("layer.groupLayers", json!({"name": "G"})).unwrap();

    let st = s.active().unwrap();
    let active = st.active_layer.and_then(|id| st.doc.layer(id)).unwrap();
    assert_eq!(active.name, "G");
    assert!(matches!(active.content, LayerContent::Group(_)));

    let before = doc_of(&s);
    let history = s.active().unwrap().history.past_len();
    let channels_before = before.channels.len();

    assert!(!s.is_enabled(CMD), "image.calculations must be disabled when a layer group is active");
    let err = s.execute(CMD, json!({"source1": {"channel": "composite"}, "source2": {"channel": "composite"}})).unwrap_err();
    assert!(matches!(&err, EngineError::Disabled(id, reason) if id == CMD && !reason.is_empty()), "expected EngineError::Disabled with reason, got {err}");
    assert!(Arc::ptr_eq(&before, &doc_of(&s)), "document must be unchanged");
    assert_eq!(doc_of(&s).channels.len(), channels_before);
    assert_eq!(s.active().unwrap().history.past_len(), history);
}

#[test]
fn type_smart_hidden_and_locked_layers_keep_calculations_enabled() {
    // Type layer
    let mut s = new_doc();
    s.execute("type.create", json!({"x": 4, "y": 12, "text": "Hi", "size": 10})).unwrap();
    assert!(s.is_enabled(CMD), "type layer keeps image.calculations enabled");
    s.execute(CMD, json!({"source1": {"channel": "composite"}, "source2": {"channel": "composite"}})).unwrap();
    assert_eq!(doc_of(&s).channels.len(), 1);

    // Smart object
    let mut s = new_doc();
    s.execute("layer.new.layer", json!({"name": "Smart"})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    assert!(s.is_enabled(CMD), "smart object keeps image.calculations enabled");
    s.execute(CMD, json!({"source1": {"channel": "composite"}, "source2": {"channel": "composite"}})).unwrap();
    assert_eq!(doc_of(&s).channels.len(), 1);

    // Hidden layer
    let mut s = new_doc();
    let hidden_id = s.execute("layer.new.layer", json!({"name": "Hidden"})).unwrap()["layer"].as_u64().unwrap();
    std::sync::Arc::make_mut(&mut s.active_mut().unwrap().doc).layer_mut(LayerId(hidden_id)).unwrap().visible = false;
    assert!(s.is_enabled(CMD), "hidden layer keeps image.calculations enabled");
    s.execute(CMD, json!({"source1": {"channel": "composite"}, "source2": {"channel": "composite"}})).unwrap();
    assert_eq!(doc_of(&s).channels.len(), 1);

    // Fully locked layer
    let mut s = new_doc();
    s.execute("layer.new.layer", json!({"name": "Locked"})).unwrap();
    s.execute("layer.lockLayers", json!({"all": true})).unwrap();
    assert!(s.is_enabled(CMD), "fully locked layer keeps image.calculations enabled");
    s.execute(CMD, json!({"source1": {"channel": "composite"}, "source2": {"channel": "composite"}})).unwrap();
    assert_eq!(doc_of(&s).channels.len(), 1);
}
