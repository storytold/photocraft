//! #13244 and #13328: a Line tool drag records one "Line Tool" step when creating a shape
//! layer, adding to that layer, or drawing a path. Scripts retain the default History names.

use serde_json::{Value, json};

use super::*;

fn session() -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 800, "height": 600})).unwrap();
    s
}

fn steps(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

fn last_step(s: &Session) -> Option<&str> {
    s.active().unwrap().history.undo_label()
}

fn line(history_name: Option<&str>) -> Value {
    json!({"kind": "line", "from": [400, 300], "to": [600, 400], "weight": 6, "historyName": history_name})
}

/// The closed outline a Path-mode Line drag adds to the Work Path.
fn outline(x: f64) -> Value {
    json!({"subpaths": [{"knots": [[x, 300.0], [x + 200.0, 300.0], [x + 200.0, 306.0], [x, 306.0]]}]})
}

#[test]
fn a_shape_mode_line_is_one_step_named_line_tool() {
    let mut s = session();
    let n = steps(&s);
    s.execute("shape.create", line(Some("Line Tool"))).unwrap();
    assert_eq!(steps(&s), n + 1);
    assert_eq!(last_step(&s), Some("Line Tool"));
}

#[test]
fn each_path_mode_line_is_one_step_named_line_tool() {
    let mut s = session();
    for x in [100.0, 400.0] {
        let n = steps(&s);
        s.execute("path.set", json!({"name": "work", "path": outline(x), "historyName": "Line Tool"})).unwrap();
        assert_eq!(steps(&s), n + 1);
        assert_eq!(last_step(&s), Some("Line Tool"));
    }
}

#[test]
fn without_the_tool_the_commands_keep_their_own_names() {
    let mut s = session();
    s.execute("shape.create", line(None)).unwrap();
    assert_eq!(last_step(&s), Some("New Shape Layer"));
    s.execute("path.set", json!({"name": "work", "path": outline(100.0)})).unwrap();
    assert_eq!(last_step(&s), Some("Work Path"));
}

#[test]
fn an_unknown_history_name_is_refused_without_a_step() {
    let mut s = session();
    let n = steps(&s);
    assert!(s.execute("shape.create", line(Some("Unknown Tool"))).is_err());
    assert!(s.execute("path.set", json!({"name": "work", "path": outline(100.0), "historyName": 7})).is_err());
    assert_eq!(steps(&s), n);
}

#[test]
fn adding_a_line_to_a_shape_uses_the_tool_history_name() {
    let mut s = session();
    let id = s.execute("shape.create", line(Some("Line Tool"))).unwrap()["layer"].as_u64().unwrap();
    let before = s.active().unwrap().doc.clone();
    let n = steps(&s);
    // Shift, Alt and Shift+Alt send these three operations to shape.create.
    for op in ["combine", "subtract", "intersect"] {
        let mut params = line(Some("Line Tool"));
        params["addTo"] = json!(id);
        params["op"] = json!(op);
        params["from"] = json!([450, 320]);
        let result = s.execute("shape.create", params).unwrap();
        assert_eq!(result["layer"], id, "{op}");
        assert_eq!(result["path"]["subpaths"][1]["op"], op, "{op}");
        assert_eq!(steps(&s), n + 1, "{op}");
        assert_eq!(last_step(&s), Some("Line Tool"), "{op}");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, *before, "{op}");
        assert_eq!(steps(&s), n, "{op}");
    }
}

#[test]
fn a_scripted_shape_add_keeps_its_default_history_name() {
    let mut s = session();
    let id = s.execute("shape.create", line(None)).unwrap()["layer"].as_u64().unwrap();
    let n = steps(&s);
    let mut params = line(None);
    params["addTo"] = json!(id);
    params.as_object_mut().unwrap().remove("historyName");
    s.execute("shape.create", params).unwrap();
    assert_eq!(steps(&s), n + 1);
    assert_eq!(last_step(&s), Some("Add Shape"));
}

#[test]
fn a_bad_label_for_a_shape_add_preserves_the_document_and_history() {
    let mut s = session();
    let id = s.execute("shape.create", line(None)).unwrap()["layer"].as_u64().unwrap();
    let n = steps(&s);
    let before = s.active().unwrap().doc.clone();
    let mut params = line(Some("Unknown Tool"));
    params["addTo"] = json!(id);
    assert!(s.execute("shape.create", params).is_err());
    assert_eq!(steps(&s), n);
    assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &before));
}
