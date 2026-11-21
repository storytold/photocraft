//! File Info OK without changes adds no history step (#13820).

use super::*;
use serde_json::json;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "depth": 8})).unwrap();
    s
}

/// Every field as the dialog submits it on OK, with the current values.
fn all_fields(s: &mut Session) -> Value {
    let mut info = s.execute("file.fileInfo", json!({})).unwrap();
    info.as_object_mut().unwrap().remove("camera");
    info
}

fn revision(s: &Session) -> u64 {
    s.active().unwrap().revision
}

#[test]
fn ok_without_changes_on_a_document_without_metadata_adds_no_step() {
    let mut s = session();
    s.active_mut().unwrap().saved_revision = revision(&s);
    let fields = all_fields(&mut s);
    s.execute("file.fileInfo", fields).unwrap();
    assert!(!s.active().unwrap().is_dirty(), "a clean document stays clean");
    assert!(s.active().unwrap().doc.metadata.xmp.is_none(), "no empty packet is written");
    assert!(!s.undo(), "no File Info step");
}

#[test]
fn ok_without_changes_after_an_edit_adds_no_step() {
    let mut s = session();
    s.execute("file.fileInfo", json!({"title": "Night", "keywords": ["stars", "sky"], "copyrightStatus": "copyrighted"})).unwrap();
    let before = revision(&s);
    let fields = all_fields(&mut s);
    s.execute("file.fileInfo", fields).unwrap();
    // Keywords typed back as the dialog shows them are the same list.
    s.execute("file.fileInfo", json!({"keywords": "stars; sky"})).unwrap();
    assert_eq!(revision(&s), before);
    assert!(s.undo(), "the one File Info edit is undone");
    assert_eq!(s.execute("file.fileInfo", json!({})).unwrap()["title"], "");
}

#[test]
fn ok_with_one_change_still_adds_one_undoable_step() {
    let mut s = session();
    s.execute("file.fileInfo", json!({"title": "Night"})).unwrap();
    let mut fields = all_fields(&mut s);
    fields["author"] = json!("Vincent");
    s.execute("file.fileInfo", fields).unwrap();
    assert!(s.undo());
    let info = s.execute("file.fileInfo", json!({})).unwrap();
    assert_eq!((info["title"].as_str(), info["author"].as_str()), (Some("Night"), Some("")));
    assert!(s.redo());
    assert_eq!(s.execute("file.fileInfo", json!({})).unwrap()["author"], "Vincent");
}
