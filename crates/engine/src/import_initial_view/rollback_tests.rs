use super::*;
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::Document;
use photocraft_geom::Size;
use photocraft_io::{NativeImportTarget, NativeInitialView};
use serde_json::json;

#[test]
fn invalid_callback_returning_an_old_or_absent_index_removes_every_appended_tab() {
    for active in [None, Some(0), Some(1)] {
        for appended in [1, 2] {
            for returned in [0, usize::MAX] {
                let mut session = Session::new();
                for _ in 0..3 {
                    session.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
                    session.execute("layer.layerMask.revealAll", json!({})).unwrap();
                    let layer = session.active().unwrap().active_layer.unwrap();
                    session
                        .apply_import_initial_view(Some(NativeInitialView {
                            selected_layers: vec![layer],
                            active_layer: Some(layer),
                            target: NativeImportTarget::LayerMask,
                            ..Default::default()
                        }))
                        .unwrap();
                }
                session.active = active;
                session.note_active_change();
                session.sync_tools_mask_target();
                let before = session.documents().to_vec();
                let opening_order = session.open_document_order.clone();
                let palette = (session.tools.mask_targeted, session.tools.foreground, session.tools.background);
                let result = session.admit_import_with_initial_view(true, Some(NativeInitialView::default()), |session| {
                    for ordinal in 0..appended {
                        let mut document = Document::new(format!("Unaccepted {ordinal}"), Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
                        document.id = before[0].doc.id; // A persisted collision must not compromise older tab identity.
                        session.add_document(document, Some(format!("unaccepted-{ordinal}.pcraft")));
                        session.sync_tools_mask_target();
                    }
                    (returned, ())
                });
                assert!(result.unwrap_err().to_string().contains("import admission"));
                assert_eq!(session.documents().len(), before.len());
                assert_eq!(session.active_index(), active);
                assert_eq!(session.open_document_order, opening_order);
                assert_eq!((session.tools.mask_targeted, session.tools.foreground, session.tools.background), palette);
                for (state, original) in session.documents().iter().zip(before) {
                    assert!(Arc::ptr_eq(&state.doc, &original.doc));
                    assert_eq!(state.path, original.path);
                    assert_eq!(state.source_read_only, original.source_read_only);
                    assert_eq!(state.active_layer, original.active_layer);
                    assert_eq!(state.selected_layers, original.selected_layers);
                    assert_eq!(state.channel_view, original.channel_view);
                    assert_eq!(state.color_samplers, original.color_samplers);
                    assert_eq!(
                        (state.revision, state.saved_revision, state.history.past_len()),
                        (original.revision, original.saved_revision, original.history.past_len())
                    );
                }
            }
        }
    }
}

#[test]
fn invalid_multiple_admission_callback_returning_a_new_index_still_removes_the_entire_suffix() {
    let mut session = Session::new();
    session.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
    let original = session.active().unwrap().doc.clone();
    let result = session.admit_import_with_initial_view(true, None, |session| {
        let first = session.add_document(Document::new("First", Size::new(4, 4), ColorMode::Rgb, SampleType::U8), None);
        session.add_document(Document::new("Second", Size::new(4, 4), ColorMode::Rgb, SampleType::U8), None);
        (first, ())
    });
    assert!(result.unwrap_err().to_string().contains("import admission"));
    assert_eq!(session.documents().len(), 1);
    assert_eq!(session.active_index(), Some(0));
    assert!(Arc::ptr_eq(&session.active().unwrap().doc, &original));
}
