//! #16872: drawing a star with the Star tool records one History state named "Star Tool"
//! in Shape mode, Create Star OK, and Path mode (Work Path). `shape.create` without a tool
//! name keeps "New Shape Layer", other tools keep their names, and invalid `historyName`
//! values are rejected without changing the document or history.

use photocraft_doc::LiveShape;
use serde_json::json;

use crate::Session;

const LABEL: &str = "Star Tool";

fn document() -> Session {
    let mut session = crate::session();
    session.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    session
}

fn star_path(x: f64) -> photocraft_doc::vector::Path {
    photocraft_vector::shapes::live_path(&LiveShape::Polygon { rect: [x, 20.0, 100.0, 100.0], sides: 5, star_ratio: 0.5, corner_radius: 0.0 })
}

#[test]
fn star_shape_records_one_undoable_tool_step() {
    let mut session = document();
    let before = session.active().unwrap().doc.clone();
    let steps = session.active().unwrap().history.past_len();
    session.execute("shape.create", json!({"kind": "star", "rect": [20, 20, 100, 100], "sides": 5, "starRatio": 0.5, "historyName": LABEL})).unwrap();
    let state = session.active().unwrap();
    assert_eq!(state.history.past_len(), steps + 1);
    assert_eq!(state.history.undo_label(), Some(LABEL));
    assert_eq!(state.doc.layers.last().unwrap().name, "Star 1");
    let with_star = state.doc.clone();
    assert!(session.undo());
    assert_eq!(session.active().unwrap().doc, before);
    assert!(session.redo());
    assert_eq!(session.active().unwrap().doc, with_star);
}

#[test]
fn each_star_work_path_edit_records_an_independent_tool_step() {
    let mut session = document();
    let state = session.active().unwrap();
    let steps = state.history.past_len();
    let layers = state.doc.layers.len();
    let mut outline = photocraft_doc::vector::Path::default();
    for (i, x) in [20.0, 160.0].into_iter().enumerate() {
        outline.subpaths.extend(star_path(x).subpaths);
        session.execute("path.set", json!({"name": "work", "path": super::path_json(&outline), "historyName": LABEL})).unwrap();
        let state = session.active().unwrap();
        assert_eq!(state.history.past_len(), steps + i + 1);
        assert_eq!(state.history.undo_label(), Some(LABEL));
        assert_eq!(state.doc.work_path.as_ref().unwrap().subpaths.len(), i + 1);
        assert_eq!(state.doc.layers.len(), layers);
    }
    assert!(session.undo());
    assert_eq!(session.active().unwrap().doc.work_path.as_ref().unwrap().subpaths.len(), 1);
    assert!(session.undo());
    assert!(session.active().unwrap().doc.work_path.is_none());
}

#[test]
fn defaults_and_other_tools_keep_their_history_labels() {
    let mut session = document();
    let response = session.execute("shape.create", json!({"kind": "star", "rect": [20, 20, 100, 100], "sides": 5, "starRatio": 0.5})).unwrap();
    let layer = &response["layer"];
    assert_eq!(session.active().unwrap().history.undo_label(), Some("New Shape Layer"));
    session
        .execute("shape.create", json!({"kind": "star", "rect": [40, 40, 80, 80], "sides": 5, "starRatio": 0.5, "addTo": layer, "historyName": LABEL}))
        .unwrap();
    assert_eq!(session.active().unwrap().history.undo_label(), Some("Add Shape"));

    session.execute("path.set", json!({"name": "work", "path": super::path_json(&star_path(20.0))})).unwrap();
    assert_eq!(session.active().unwrap().history.undo_label(), Some("Work Path"));

    for (kind, name) in [("line", "Line Tool"), ("triangle", "Triangle Tool"), ("ellipse", "Ellipse Tool")] {
        let p = if kind == "line" {
            json!({"kind": "line", "from": [10, 10], "to": [100, 10], "historyName": name})
        } else {
            json!({"kind": kind, "rect": [10, 10, 60, 40], "historyName": name})
        };
        session.execute("shape.create", p).unwrap();
        assert_eq!(session.active().unwrap().history.undo_label(), Some(name));
    }
}

#[test]
fn invalid_history_labels_leave_the_document_and_history_intact() {
    let mut session = document();
    let before = session.active().unwrap().doc.clone();
    let steps = session.active().unwrap().history.past_len();
    let path = super::path_json(&star_path(20.0));
    for invalid in [json!("Wrong Tool"), json!(42), json!("Star tool")] {
        assert!(
            session.execute("shape.create", json!({"kind": "star", "rect": [20, 20, 100, 100], "sides": 5, "starRatio": 0.5, "historyName": invalid})).is_err()
        );
        assert!(session.execute("path.set", json!({"name": "work", "path": path, "historyName": invalid})).is_err());
    }
    assert_eq!(session.active().unwrap().doc, before);
    assert_eq!(session.active().unwrap().history.past_len(), steps);
}
