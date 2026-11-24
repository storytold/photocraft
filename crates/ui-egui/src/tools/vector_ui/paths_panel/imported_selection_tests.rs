use super::*;
use photocraft_doc::vector::NamedPath;
use photocraft_geom::Size;
use photocraft_io::NativeInitialView;
use std::sync::Arc;

fn imported(names: &[&str], selected: usize) -> Box<PhotocraftApp> {
    let mut document = Document::new("Imported paths", Size::new(8, 6), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    document.paths = names.iter().map(|name| NamedPath { name: (*name).into(), ..Default::default() }).collect();
    let view = NativeInitialView {
        selected_path: Some(selected),
        selected_path_identity: Some(photocraft_io::initial_view::path_identity(&document.paths[selected])),
        ..Default::default()
    };
    let mut session = photocraft_engine::Session::new();
    let (_, warning) = session.admit_import_with_initial_view(document, Some("paths.xcf".into()), true, Some(view));
    assert!(warning.is_none());
    PhotocraftApp::new(session, crate::Services::default())
}

#[test]
fn saved_path_selection_reaches_the_panel_and_tools_once_without_an_edit() {
    let mut app = imported(&["Other", "Selected"], 1);
    let before = app.session.active().unwrap();
    let doc = before.doc.clone();
    let history = before.history.past_len();
    app.sync_views();
    assert_eq!(app.ui.selected_path.as_deref(), Some("Selected"));
    assert_eq!(super::super::selected_document_path(&app).as_deref(), Some("Selected"));
    assert_eq!(key_of_name(&doc, "Selected", None), Some(PathRef::Saved(1)));
    let state = app.session.active().unwrap();
    assert!(state.initial_path_selection.is_none() && state.initial_path_identity.is_none());
    assert!(Arc::ptr_eq(&doc, &state.doc));
    assert_eq!(state.history.past_len(), history);
    assert!(state.source_read_only);
    assert_eq!(state.protected_import_source.as_deref(), Some("paths.xcf"));
    app.ui.selected_path = Some("Other".into());
    app.sync_views();
    assert_eq!(app.ui.selected_path.as_deref(), Some("Other"));
    assert!(app.ui.notices.is_empty());
}

#[test]
fn stale_or_unaddressable_saved_path_selection_warns_once_without_selecting_another_path() {
    for names in [vec!["Duplicate", "Duplicate"], vec!["work"], vec!["layer"], vec!["Changed"]] {
        let mut app = imported(&names, names.len() - 1);
        if names == ["Changed"] {
            Arc::make_mut(&mut app.session.active_mut().unwrap().doc).paths[0].name = "Different".into();
        }
        app.sync_views();
        assert!(app.ui.selected_path.is_none());
        assert_eq!(app.ui.notices.len(), 1);
        assert!(app.ui.notices[0].lines[0].contains("saved path selection"));
        assert!(app.ui.notices[0].lines[0].len() < 256);
        assert!(app.session.active().unwrap().initial_path_selection.is_none());
        app.sync_views();
        assert_eq!(app.ui.notices.len(), 1);
    }
}

#[test]
fn pending_open_preserves_the_exact_handoff_until_final_admission() {
    let mut app = imported(&["Selected"], 0);
    let id = app.session.active().unwrap().doc.id;
    app.jobs.opens.push(crate::jobs_ui::OpenTab { job: photocraft_engine::jobs::JobId(99), name: "pending.xcf".into(), path: None, slot: None, order: None });
    app.sync_views();
    assert!(app.ui.selected_path.is_none());
    let state = app.session.active_mut().unwrap();
    let hint = state.take_fresh_import_view().unwrap();
    assert_eq!(hint.selected_path, Some(0));
    assert_eq!(hint.selected_path_identity, Some(photocraft_io::initial_view::path_identity(&state.doc.paths[0])));
    app.session.apply_import_initial_view(Some(hint)).unwrap();
    crate::jobs_ui::after_open(&mut app, id, "paths.xcf", None, &[], None).unwrap();
    assert_eq!(app.ui.selected_path.as_deref(), Some("Selected"));
    assert_eq!(app.jobs.opens.len(), 1);
    assert!(app.session.active().unwrap().initial_path_selection.is_none());
}
