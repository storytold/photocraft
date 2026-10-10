use super::*;
use photocraft_doc::{Color, ColorMode, SampleType, Size};
use serde_json::json;

fn session_at(x: i32, y: i32) -> Session {
    let mut doc = Document::with_background("compact", Size::new(32, 32), ColorMode::Rgb, SampleType::U8, Color::TRANSPARENT);
    let layer = doc.layers.first_mut().unwrap();
    layer.locks = Default::default();
    layer.surface_mut().unwrap().write_pixel(x, y, &[0.2, 0.4, 0.6, 1.0]);
    let mut session = Session::new();
    session.add_document(doc, None);
    session.active_mut().unwrap().history.max_states = 2_000;
    session
}

fn same_document(actual: &Document, expected: &Document) {
    // Allocation of all-default tiles is storage layout, not an image/history difference.
    let normalized = |doc: &Document| {
        let mut doc = doc.clone();
        for layer in &mut doc.layers {
            if let Some(surface) = layer.surface_mut() {
                surface.prune();
            }
        }
        doc
    };
    assert!(normalized(actual) == normalized(expected), "restored pixels or document metadata differ");
}

fn compare_all_states(mut session: Session, moves: &[(i32, i32)]) {
    let mut expected = vec![session.active().unwrap().doc.clone()];
    let id = session.active().unwrap().doc.layers.first().unwrap().id;
    for &(dx, dy) in moves {
        let mut next = expected.last().unwrap().as_ref().clone();
        crate::layer_multi_cmds::move_layers(&mut next, &[(id, dx, dy)]).unwrap();
        expected.push(Arc::new(next));
        session.execute("layer.translate", json!({"dx":dx,"dy":dy})).unwrap();
        assert_eq!(session.active().unwrap().doc.as_ref(), expected.last().unwrap().as_ref());
    }
    assert_eq!(session.active().unwrap().history.past_len(), moves.len());
    for state in expected.iter().rev().skip(1) {
        assert!(session.try_undo().unwrap());
        same_document(session.active().unwrap().doc.as_ref(), state.as_ref());
    }
    for state in expected.iter().skip(1) {
        assert!(session.try_redo().unwrap());
        same_document(session.active().unwrap().doc.as_ref(), state.as_ref());
    }
}

#[test]
fn outward_and_back_across_negative_tile_boundaries_replays_every_state() {
    compare_all_states(session_at(-257, -1), &[(-1, -1), (-255, -255), (1, 1), (255, 255), (257, 257), (-257, -257)]);
}

#[test]
fn thousand_monotonic_nudges_replay_exactly() {
    compare_all_states(session_at(-513, -257), &vec![(1, 0); 1_000]);
}

#[test]
fn source_near_coordinate_limit_returns_error_without_mutation_or_panic() {
    let mut session = session_at(i32::MAX - 1_024, 0);
    let before = session.active().unwrap().doc.clone();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| session.execute("layer.translate", json!({"dx":2_048}))));
    assert!(result.is_ok(), "the command must validate before translating tiles");
    assert!(result.unwrap().is_err());
    assert!(Arc::ptr_eq(&session.active().unwrap().doc, &before));
    assert!(session.compact_move.is_none());
    assert!(!session.active().unwrap().history.can_undo());
}

#[test]
fn another_edit_starts_a_new_source_and_coalesced_move_keeps_final_state() {
    let mut session = session_at(-1, 0);
    session.execute("layer.translate", json!({"dx":1})).unwrap();
    session
        .edit("Rename", |doc, _| {
            doc.name = "renamed".into();
            Ok(())
        })
        .unwrap();
    let renamed = session.active().unwrap().doc.clone();
    session.execute("layer.translate", json!({"dx":1,"coalesce":"drag"})).unwrap();
    session.execute("layer.translate", json!({"dx":1,"coalesce":"drag"})).unwrap();
    let final_doc = session.active().unwrap().doc.clone();
    let source = session.compact_move.as_ref().unwrap().source.upgrade().unwrap();
    assert!(Arc::ptr_eq(&source, &renamed));
    assert_eq!(session.active().unwrap().history.past_len(), 3);
    assert!(session.try_undo().unwrap());
    assert_eq!(session.active().unwrap().doc.as_ref(), renamed.as_ref());
    assert!(session.try_redo().unwrap());
    assert_eq!(session.active().unwrap().doc.as_ref(), final_doc.as_ref());
}

#[test]
fn purge_and_close_release_compact_source_backing() {
    let mut session = session_at(0, 0);
    let source = Arc::downgrade(&session.active().unwrap().doc);
    session.execute("layer.translate", json!({"dx":1})).unwrap();
    assert!(source.upgrade().is_some());
    session.active_mut().unwrap().history.clear();
    assert!(source.upgrade().is_none(), "Run must retain only weak source/current handles");
    session.execute("layer.translate", json!({"dx":1})).unwrap();
    let next_source = session.compact_move.as_ref().unwrap().source.clone();
    assert!(next_source.upgrade().is_some());
    drop(session.close(0));
    assert!(next_source.upgrade().is_none());
}

#[test]
fn coalesced_non_move_edit_invalidates_previous_replay() {
    let mut session = session_at(0, 0);
    let original = session.active().unwrap().doc.clone();
    session.execute("layer.translate", json!({"dx": 1, "dy": 0, "coalesce": "drag"})).unwrap();
    session.execute("layer.renameLayer", json!({"name": "later edit", "coalesce": "drag"})).unwrap();
    let expected = session.active().unwrap().doc.clone();
    assert_eq!(session.active().unwrap().history.past_len(), 1);
    assert!(session.undo());
    same_document(&session.active().unwrap().doc, &original);
    assert!(session.redo());
    same_document(&session.active().unwrap().doc, &expected);
}

#[test]
fn non_history_metadata_edit_cannot_reuse_stale_current_replay() {
    let mut session = session_at(0, 0);
    session.execute("layer.translate", json!({"dx": 1, "dy": 0})).unwrap();
    let layer = session.active().unwrap().active_layer.unwrap();
    session.execute("image.variables.define", json!({"defs": [{"name": "Visible", "layer": layer.0, "type": "visibility"}]})).unwrap();
    let expected = session.active().unwrap().doc.clone();
    assert_eq!(expected.variables.defs.len(), 1);
    assert!(session.undo());
    assert!(session.redo());
    same_document(&session.active().unwrap().doc, &expected);
}
