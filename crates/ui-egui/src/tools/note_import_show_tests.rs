//! #13918: File › Import › Notes… leaves the Notes panel as it was and selects no note, and a
//! successful import turns View › Show › Notes back on so the notes show on the canvas.

use std::sync::Arc;

use craft::host::{Picked, ScriptedFiles};
use serde_json::json;

use crate::PhotocraftApp;
use crate::panel_docking::{is_front, is_shown, show_panel};
use crate::tools::analysis::NOTES;

/// A document with no notes, file dialogs the test answers, and a saved document holding two notes.
fn app() -> (Box<PhotocraftApp>, Arc<ScriptedFiles>, Vec<u8>) {
    let files = Arc::new(ScriptedFiles::held());
    let services = crate::Services { files: Some(files.clone()), ..Default::default() };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    for x in [10, 60] {
        app.run("notes.add", json!({"x": x, "y": 20})).unwrap();
    }
    let source = photocraft_format::save_to_bytes(&app.session.active().unwrap().doc, &Default::default()).unwrap();
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    (app, files, source)
}

fn import(app: &mut PhotocraftApp, files: &ScriptedFiles, picked: Picked) {
    crate::menus::invoke(app, &egui::Context::default(), "file.import.notes", json!({})).unwrap();
    files.answer(picked).expect("Import Notes opens a file dialog");
    app.poll_file_dialog();
}

fn notes(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.notes.len()
}

#[test]
fn an_import_shows_the_notes_and_leaves_the_panel_alone() {
    for panel_open in [false, true] {
        let (mut app, files, source) = app();
        show_panel(&mut app, NOTES, Some(panel_open)).unwrap();
        app.ui.view.show.notes = false;
        let extras = app.ui.view.extras;
        let panel = (is_shown(&app, NOTES), is_front(&app, NOTES));
        import(&mut app, &files, Picked::Contents(vec![("notes.pcraft".into(), source)]));
        assert!(!app.ui.status_error, "{}", app.ui.status);
        assert!(notes(&app) > 0, "the notes are imported");
        assert!(app.ui.view.show.notes, "View › Show › Notes is on again");
        assert_eq!(app.ui.view.extras, extras, "the Extras switch is left alone");
        assert_eq!((is_shown(&app, NOTES), is_front(&app, NOTES)), panel, "panel open {panel_open}: it keeps its state");
        assert_eq!(app.ui.analysis.note_selected, None, "no note is selected");
    }
}

#[test]
fn a_cancelled_or_failed_import_changes_neither() {
    for picked in [Picked::Cancelled, Picked::Contents(vec![("notes.pcraft".into(), b"not a document".to_vec())])] {
        let (mut app, files, _) = app();
        show_panel(&mut app, NOTES, Some(false)).unwrap();
        app.ui.view.show.notes = false;
        import(&mut app, &files, picked);
        assert_eq!(notes(&app), 0);
        assert!(!app.ui.view.show.notes && !is_shown(&app, NOTES));
    }
}
