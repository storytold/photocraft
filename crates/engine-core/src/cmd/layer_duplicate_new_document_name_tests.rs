//! #14608: Duplicate Layer… with Document = New and an empty Name makes an "Untitled-N"
//! document, as the reference application does, not "<source> copy".

use serde_json::json;

use crate::Session;

fn doc_name(s: &Session, i: usize) -> String {
    s.documents().get(i).map(|d| d.doc.name.clone()).unwrap_or_default()
}

/// A one-layer document named `name`, with its only layer active.
fn source(name: &str) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"name": name, "width": 40, "height": 30, "background": "white"})).unwrap();
    s
}

#[test]
fn an_empty_name_makes_the_next_untitled_document() {
    let mut s = source("oracle");
    s.execute("layer.duplicate", json!({"document": "new"})).unwrap();
    assert_eq!(doc_name(&s, 0), "oracle", "the source keeps its name");
    assert_eq!(doc_name(&s, 1), "Untitled-1");
}

#[test]
fn the_counter_skips_untitled_documents_already_open() {
    let mut s = source("oracle");
    // An Untitled-1 is open beside the source; the copy takes the next number.
    s.execute("file.new", json!({"name": "Untitled-1", "width": 8, "height": 8})).unwrap();
    let oracle = s.documents().iter().position(|d| d.doc.name == "oracle").unwrap();
    s.set_active(oracle);
    s.execute("layer.duplicate", json!({"document": "new"})).unwrap();
    assert_eq!(doc_name(&s, s.active_index().unwrap()), "Untitled-2");
}

#[test]
fn a_typed_name_is_used_as_given() {
    let mut s = source("oracle");
    s.execute("layer.duplicate", json!({"document": "new", "documentName": "MyNew"})).unwrap();
    assert_eq!(doc_name(&s, 1), "MyNew");
}

#[test]
fn an_empty_typed_name_falls_back_to_untitled() {
    let mut s = source("oracle");
    s.execute("layer.duplicate", json!({"document": "new", "documentName": ""})).unwrap();
    assert_eq!(doc_name(&s, 1), "Untitled-1");
}
