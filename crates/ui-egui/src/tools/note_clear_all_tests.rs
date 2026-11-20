//! #11791: the Note tool's Clear All asks first. OK deletes every note in one "Delete all Notes"
//! step that one undo restores; a stale document deletes nothing.

use serde_json::json;

use super::*;

/// A 200 × 100 document with three notes, the second one selected, and the Note tool.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    for (x, author) in [(10, "A"), (60, "B"), (110, "C")] {
        app.run("notes.add", json!({"x": x, "y": 20, "author": author})).unwrap();
    }
    app.ui.tool = Tool::Note;
    app.ui.analysis.note_selected = Some(1);
    app
}

fn notes(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.notes.len()
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn prompt(app: &PhotocraftApp) -> Option<egui::Id> {
    crate::dialogs::alerts::prompt_tagged::<NoteQuestion>(app)
}

#[test]
fn ok_deletes_every_note_in_one_step_that_undo_restores() {
    let mut app = app();
    app.run("notes.set", json!({"index": 1, "color": "#3366cc"})).unwrap();
    app.ui.analysis.note_author = "B".into();
    let ctx = app.ctx().clone();
    let before = steps(&app);
    ask_clear_notes(&mut app);
    let id = prompt(&app).expect("Clear All opens the alert");
    assert_eq!(notes(&app), 3, "nothing is deleted before the answer");
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    assert_eq!(notes(&app), 0);
    assert_eq!(steps(&app), before + 1, "one history step");
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Delete all Notes"));
    // A fresh app holds the tool defaults: `app()` changes only `note_selected`.
    let fresh = self::app();
    let defaults = &fresh.ui.analysis;
    let a = &app.ui.analysis;
    assert_eq!(a.note_selected, None, "nothing selected after OK");
    assert_eq!((a.note_author.as_str(), a.note_color), (defaults.note_author.as_str(), defaults.note_color), "the tool defaults after OK");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(notes(&app), 3, "one undo brings every note back");
}

#[test]
fn ok_after_the_document_changed_deletes_nothing() {
    let mut app = app();
    let ctx = app.ctx().clone();
    ask_clear_notes(&mut app);
    let id = prompt(&app).expect("the alert");
    app.run("file.new", json!({"width": 50, "height": 50})).unwrap();
    app.run("notes.add", json!({"x": 5, "y": 5})).unwrap();
    let past: Vec<usize> = app.session.documents().iter().map(|d| d.history.past_len()).collect();
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    assert!(app.ui.status_error && !app.ui.status.is_empty(), "the status line reports the stale question");
    let docs = app.session.documents();
    assert_eq!(docs.iter().map(|d| d.doc.notes.len()).collect::<Vec<_>>(), [3, 1], "no note deleted in either document");
    assert_eq!(docs.iter().map(|d| d.history.past_len()).collect::<Vec<_>>(), past, "no history step in either");
}
