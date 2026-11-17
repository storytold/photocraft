//! Closing a tab preserves document identity and restores the most recently activated survivor.
use photocraft_doc::DocId;
use photocraft_engine::Session;
use serde_json::json;

fn session_with_documents(count: usize) -> (Session, Vec<DocId>) {
    let mut session = Session::new();
    let mut ids = Vec::new();
    for i in 0..count {
        session.execute("file.new", json!({"name": format!("Document {i}"), "width": 8, "height": 8})).unwrap();
        ids.push(session.active().unwrap().doc.id);
    }
    (session, ids)
}

fn document_ids(session: &Session) -> Vec<DocId> {
    session.documents().iter().map(|state| state.doc.id).collect()
}

#[test]
fn closing_a_background_document_keeps_the_active_document() {
    for (active, closing) in [(0, 1), (0, 2), (1, 0), (1, 2), (2, 0), (2, 1)] {
        let (mut session, mut ids) = session_with_documents(3);
        assert!(session.set_active(active));
        let active_id = ids[active];
        ids.remove(closing);

        session.execute("file.close", json!({"document": closing})).unwrap();

        assert_eq!(session.active().unwrap().doc.id, active_id, "active {active}, closing {closing}");
        assert_eq!(session.active_index(), ids.iter().position(|id| *id == active_id));
        assert_eq!(document_ids(&session), ids);
    }
}

#[test]
fn closing_the_active_document_selects_the_most_recent_document() {
    for (closing, next) in [(0, 2), (1, 2), (2, 1)] {
        let (mut session, mut ids) = session_with_documents(3);
        assert!(session.set_active(closing));
        let next_id = ids[next];
        ids.remove(closing);

        session.execute("file.close", json!({})).unwrap();

        assert_eq!(session.active().unwrap().doc.id, next_id);
        assert_eq!(session.active_index(), ids.iter().position(|id| *id == next_id));
        assert_eq!(document_ids(&session), ids);
    }
}

/// Observed in Photoshop 2020: open four tabs, activate the first then fourth, close the
/// fourth; the first returns, rather than the neighbouring third tab.
#[test]
fn closing_uses_activation_order_even_after_tab_reordering() {
    for moved in [None, Some((0, 3)), Some((3, 0)), Some((1, 3))] {
        let (mut session, ids) = session_with_documents(4);
        assert!(session.set_active(0));
        assert!(session.set_active(3));
        if let Some((from, to)) = moved {
            session.move_document(from, to).unwrap();
        }
        session.execute("file.close", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc.id, ids[0], "{moved:?}");
        // Tab moves and Close must not turn activation order into recorded opening order.
        session.execute("document.activate", json!({"document":{"index":2}})).unwrap();
        assert_eq!(session.active().unwrap().doc.id, ids[1]);
    }
}

#[test]
fn invalid_activation_and_background_close_do_not_reorder_recent_documents() {
    let (mut session, ids) = session_with_documents(4);
    assert!(session.set_active(0));
    assert!(session.set_active(3));
    assert!(!session.set_active(usize::MAX));
    session.execute("file.close", json!({"document":1})).unwrap();
    assert_eq!(session.active().unwrap().doc.id, ids[3]);
    session.execute("file.close", json!({})).unwrap();
    assert_eq!(session.active().unwrap().doc.id, ids[0]);
    session.execute("file.close", json!({})).unwrap();
    assert_eq!(session.active().unwrap().doc.id, ids[2], "closed entries are removed from activation order");
}

#[test]
fn activation_order_uses_ids_when_document_names_match_and_after_reopening() {
    let (mut session, ids) = session_with_documents(3);
    for index in 0..3 {
        session.set_active(index);
        session
            .edit("Name", |doc, _| {
                doc.name = "Same".into();
                Ok(())
            })
            .unwrap();
    }
    session.set_active(0);
    session.set_active(2);
    let source = session.documents()[0].doc.as_ref().clone();
    session.execute("file.close", json!({})).unwrap();
    assert_eq!(session.active().unwrap().doc.id, ids[0]);
    session.execute("file.closeAll", json!({})).unwrap();
    session.add_document(source.clone(), None);
    let first = session.active().unwrap().doc.id;
    session.add_document(source, None);
    let second = session.active().unwrap().doc.id;
    assert_ne!(first, second, "the saved identity is remapped while its first copy is open");
    session.execute("file.close", json!({})).unwrap();
    assert_eq!(session.active().unwrap().doc.id, first);
}

#[test]
fn saving_background_smart_contents_does_not_change_document_activation_order() {
    let (mut session, ids) = session_with_documents(1);
    session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    session.execute("layer.smartObjects.editContents", json!({})).unwrap();
    session.execute("layer.new.layer", json!({"name":"Changed"})).unwrap();
    for name in ["Previous", "Current"] {
        session.execute("file.new", json!({"name":name,"width":8,"height":8})).unwrap();
    }
    let previous = session.documents()[2].doc.id;
    let current = session.active().unwrap().doc.id;
    session.execute("file.close", json!({"document":1})).unwrap();
    assert_eq!(session.active().unwrap().doc.id, current);
    let parent = session.documents().iter().find(|st| st.doc.id == ids[0]).unwrap();
    let (name, bytes) = photocraft_engine::smart_cmds::contents_of(&parent.doc, parent.active_layer.unwrap()).unwrap();
    let saved = photocraft_engine::smart_cmds::decode_source(&name, &bytes).unwrap();
    assert!(saved.walk().iter().any(|(_, _, l)| l.name == "Changed"), "the child's edits still reach its parent");
    session.execute("file.close", json!({})).unwrap();
    assert_eq!(session.active().unwrap().doc.id, previous, "the internal parent save is not a document activation");
}

#[test]
fn editing_other_documents_does_not_change_activation_order() {
    for command in ["layer.duplicate", "select.saveSelection", "channel.duplicate"] {
        let (mut session, ids) = session_with_documents(4);
        session.set_active(0);
        session.set_active(3);
        session.execute("select.all", json!({})).unwrap();
        if command == "channel.duplicate" {
            session.execute("select.saveSelection", json!({"name":"Saved"})).unwrap();
        }
        let params = match command {
            "layer.duplicate" => json!({"document":{"name":"Document 1"}}),
            "channel.duplicate" => json!({"channel":"Saved","document":1}),
            _ => json!({"document":1}),
        };
        session.execute(command, params).unwrap();
        assert_eq!(session.active().unwrap().doc.id, ids[3], "{command} keeps its source active");
        session.execute("file.close", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc.id, ids[0], "{command} must not activate its hidden edit destination");
    }
}

#[test]
fn copying_and_switching_to_the_destination_records_its_activation() {
    let (mut session, ids) = session_with_documents(4);
    session.set_active(0);
    session.set_active(3);
    session.execute("layer.copyToDocument", json!({"document":1})).unwrap();
    assert_eq!(session.active().unwrap().doc.id, ids[1]);
    session.execute("file.close", json!({})).unwrap();
    assert_eq!(session.active().unwrap().doc.id, ids[3]);
}

#[test]
fn closing_the_last_document_clears_the_active_document() {
    let (mut session, _) = session_with_documents(1);
    session.execute("file.close", json!({})).unwrap();
    assert!(session.documents().is_empty());
    assert_eq!(session.active_index(), None);
    assert!(session.active().is_none());
    assert!(session.close(0).is_none());
    assert!(session.execute("file.close", json!({})).is_err());
}

#[test]
fn closing_an_invalid_document_preserves_the_session() {
    let (mut session, ids) = session_with_documents(3);
    assert!(session.set_active(1));
    for index in [3, usize::MAX] {
        assert!(session.close(index).is_none());
        assert!(session.execute("file.close", json!({"document": index})).is_err());
        assert_eq!(session.active_index(), Some(1));
        assert_eq!(session.active().unwrap().doc.id, ids[1]);
        assert_eq!(document_ids(&session), ids);
    }
}

#[test]
fn closing_other_documents_keeps_the_requested_document() {
    let (mut session, ids) = session_with_documents(3);
    session.execute("file.closeOthers", json!({"document": 1})).unwrap();
    assert_eq!(document_ids(&session), vec![ids[1]]);
    assert_eq!(session.active_index(), Some(0));
    session.execute("file.closeAll", json!({})).unwrap();
    assert!(session.documents().is_empty());
    assert_eq!(session.active_index(), None);
}
