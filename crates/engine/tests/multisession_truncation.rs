use photocraft_engine::Session;
use photocraft_engine::events::poll;
use serde_json::json;

/// Critere H4/C2 : un calque de 300 caracteres multioctets (`é`) donne un `document.changed`
/// dont `elements[].name` est coupe a exactement 256 caracteres, avec au plus 256 elements.
#[test]
fn ms_trunc_long_name_from_command_is_published_truncated() {
    let long = "é".repeat(300);

    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4, "height": 4, "background": "#804020"})).expect("file.new works");
    s.execute("layer.new.layer", json!({"name": long, "client": "ag-1"})).expect("layer.new.layer works");

    let batch = poll(&s.events, &json!({"since": 0, "events": ["document.changed"]})).expect("poll works");
    let ev = batch
        .events
        .iter()
        .rev()
        .find(|e| e.event == "document.changed" && e.data["command"] == json!("layer.new.layer"))
        .expect("the command event with a 300-character name must be published, not dropped");
    let elements = ev.data["elements"].as_array().expect("elements lists the touched layers");
    assert!(!elements.is_empty(), "the created layer must be listed in elements");
    assert!(elements.len() <= 256, "at most 256 elements, got {}", elements.len());
    for el in elements {
        let name = el["name"].as_str().expect("an element name is a string");
        assert!(name.chars().count() <= 256, "element name must be at most 256 characters, got {}", name.chars().count());
    }
    assert!(
        elements.iter().any(|el| el["name"].as_str().is_some_and(|n| n.chars().count() == 256)),
        "the 300-character layer name must be cut to exactly 256 characters in elements"
    );
}
