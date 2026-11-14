//! #14172: Edit > Check Spelling is enabled in any open document, also one without type layers,
//! where it finds nothing and adds no history state; with no document open it stays greyed.
//! `type.insertText` keeps needing an active type layer.

use serde_json::json;

use crate::Session;

pub(super) const CMD: &str = "edit.checkSpelling";

/// A white document with only its Background, as File > New makes one.
pub(super) fn blank() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 500, "background": "white"})).unwrap();
    s
}

pub(super) fn history(s: &Session) -> Vec<String> {
    s.active().unwrap().history.entries()
}

#[test]
fn greyed_without_a_document() {
    let s = Session::new();
    assert!(!s.is_enabled(CMD));
    assert_eq!(s.disabled_reason(CMD).as_deref(), Some("no document open"));
}

#[test]
fn enabled_in_a_document_without_type_layers() {
    let mut s = blank();
    assert!(s.is_enabled(CMD), "{:?}", s.disabled_reason(CMD));
    s.execute("layer.new.layer", json!({"name": "Empty"})).unwrap();
    assert!(s.is_enabled(CMD), "an empty pixel layer on top keeps it enabled");
    assert!(!s.is_enabled("type.insertText"), "Insert Glyph still needs an active type layer");
}

#[test]
fn listing_a_document_without_type_layers_finds_nothing_and_adds_no_history() {
    let mut s = blank();
    s.execute("layer.new.layer", json!({"name": "Empty"})).unwrap();
    let before = history(&s);
    for params in [json!({"action": "list"}), json!({})] {
        assert_eq!(s.execute(CMD, params.clone()).unwrap(), json!({"misspellings": [], "count": 0}), "{params}");
    }
    assert_eq!(history(&s), before);
}

#[test]
fn enabled_with_type_layers_as_before() {
    let mut s = blank();
    s.execute("type.create", json!({"x": 10, "y": 50, "text": "Teh end", "size": 12})).unwrap();
    assert!(s.is_enabled(CMD));
    assert!(s.is_enabled("type.insertText"));
    assert_eq!(s.execute(CMD, json!({"action": "list"})).unwrap()["count"], 1);
}

/// Lock All on the only type layer: greyed on main, now enabled, and it lists nothing since a
/// fully locked layer is not checked (its prompt is #14175).
#[test]
fn enabled_with_only_a_lock_all_type_layer_and_lists_nothing() {
    let mut s = blank();
    let id = s.execute("type.create", json!({"x": 10, "y": 50, "text": "Teh end", "size": 12})).unwrap()["layer"].clone();
    s.execute("layer.setProps", json!({"layer": id, "locked": true})).unwrap();
    assert!(s.is_enabled(CMD), "{:?}", s.disabled_reason(CMD));
    assert_eq!(s.execute(CMD, json!({"action": "list"})).unwrap()["count"], 0);
}
