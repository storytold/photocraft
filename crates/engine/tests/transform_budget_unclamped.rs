//! #6430: integration tests use the production pixel budget, not the unit tests' reduced one.
use photocraft_engine::Session;
use photocraft_geom::Rect;
use serde_json::{Value, json};

fn painted() -> Session {
    let mut session = Session::new();
    session.execute("file.new", json!({"width": 24, "height": 16})).unwrap();
    session.execute("layer.new.layer", json!({})).unwrap();
    session
        .edit("paint", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(10, 2, 11, 10), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    session
}

fn assert_refused_without_changes(session: &mut Session, command: &str, params: Value) {
    let state = session.active().unwrap();
    let (document, revision, history) = (state.doc.clone(), state.revision, state.history.past_len());
    let error = session.execute(command, params).unwrap_err().to_string();
    assert!(error.contains("limit"), "{error}");
    let state = session.active().unwrap();
    assert!(state.doc == document, "refusal changed the document");
    assert_eq!((state.revision, state.history.past_len()), (revision, history));
}

#[test]
fn extreme_layer_scales_are_refused_before_a_clipped_strip_is_warped() {
    let mut session = painted();
    session.execute("filter.render.clouds", json!({})).unwrap();
    for scale in [1e9, 1e30, 1e308, -1e308] {
        for axis in ["scaleX", "scaleY"] {
            assert_refused_without_changes(&mut session, "edit.transform", json!({(axis): scale}));
        }
    }
}

#[test]
fn extreme_selection_scales_are_refused_without_losing_the_selection() {
    let mut session = painted();
    session.execute("select.rect", json!({"x": 10, "y": 2, "width": 1, "height": 8})).unwrap();
    for scale in [1e30, 1e308] {
        for axis in ["scaleX", "scaleY"] {
            assert_refused_without_changes(&mut session, "select.transformSelection", json!({(axis): scale}));
        }
    }
}
