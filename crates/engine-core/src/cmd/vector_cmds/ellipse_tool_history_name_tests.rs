use photocraft_doc::LiveShape;
use serde_json::json;

use crate::Session;

const LABEL: &str = "Ellipse Tool";

fn document() -> Session {
    let mut session = crate::session();
    session.execute("file.new", json!({"width": 160, "height": 120})).unwrap();
    session
}

#[test]
fn ellipse_shape_records_one_undoable_tool_step() {
    let mut session = document();
    let before = session.active().unwrap().doc.clone();
    let steps = session.active().unwrap().history.past_len();
    session.execute("shape.create", json!({"kind": "ellipse", "rect": [15, 20, 45, 35], "historyName": LABEL})).unwrap();
    let state = session.active().unwrap();
    assert_eq!(state.history.past_len(), steps + 1);
    assert_eq!(state.history.undo_label(), Some(LABEL));
    assert_eq!(state.doc.layers.last().unwrap().name, "Ellipse 1");
    let with_ellipse = state.doc.clone();
    assert!(session.undo());
    assert_eq!(session.active().unwrap().doc, before);
    assert!(session.redo());
    assert_eq!(session.active().unwrap().doc, with_ellipse);
}

#[test]
fn each_ellipse_work_path_edit_records_an_independent_tool_step() {
    let mut session = document();
    let state = session.active().unwrap();
    let steps = state.history.past_len();
    let layers = state.doc.layers.len();
    let mut outline = photocraft_doc::vector::Path::default();
    for (i, x) in [15.0, 80.0].into_iter().enumerate() {
        let ellipse = photocraft_vector::shapes::live_path(&LiveShape::Ellipse { rect: [x, 20.0, 45.0, 35.0] });
        outline.subpaths.extend(ellipse.subpaths);
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
fn scripting_defaults_and_shape_additions_keep_their_labels() {
    let mut session = document();
    let response = session.execute("shape.create", json!({"kind": "ellipse", "rect": [15, 20, 45, 35]})).unwrap();
    let layer = &response["layer"];
    assert_eq!(session.active().unwrap().history.undo_label(), Some("New Shape Layer"));
    session.execute("shape.create", json!({"kind": "ellipse", "rect": [30, 30, 40, 40], "addTo": layer, "historyName": LABEL})).unwrap();
    assert_eq!(session.active().unwrap().history.undo_label(), Some("Add Shape"));

    let ellipse = photocraft_vector::shapes::live_path(&LiveShape::Ellipse { rect: [15.0, 20.0, 45.0, 35.0] });
    session.execute("path.set", json!({"name": "work", "path": super::path_json(&ellipse)})).unwrap();
    assert_eq!(session.active().unwrap().history.undo_label(), Some("Work Path"));
}

#[test]
fn invalid_history_labels_leave_the_document_and_history_intact() {
    let mut session = document();
    let before = session.active().unwrap().doc.clone();
    let steps = session.active().unwrap().history.past_len();
    let ellipse = photocraft_vector::shapes::live_path(&LiveShape::Ellipse { rect: [15.0, 20.0, 45.0, 35.0] });
    for invalid in [json!("Wrong Tool"), json!(42), json!("Ellipse tool")] {
        assert!(session.execute("shape.create", json!({"kind": "ellipse", "rect": [15, 20, 45, 35], "historyName": invalid})).is_err());
        assert!(session.execute("path.set", json!({"name": "work", "path": super::path_json(&ellipse), "historyName": invalid})).is_err());
    }
    assert_eq!(session.active().unwrap().doc, before);
    assert_eq!(session.active().unwrap().history.past_len(), steps);
}
