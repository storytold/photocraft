//! Closing a document's own OS window closes the document, as in Photoshop.

use serde_json::json;

use super::{close_document_of, closes_document};
use crate::PhotocraftApp;
use crate::state::DocWindow;

/// An app with two documents; `a.psd` (index 0) floats in its own window.
fn app_with_floating() -> (PhotocraftApp, DocWindow) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 48, "name": "a"})).unwrap();
    app.run("file.new", json!({"width": 64, "height": 48, "name": "b"})).unwrap();
    app.sync_views();
    let id = crate::view_cmds::float_window(&mut app, 0, 0, true);
    let window = app.ui.windows.iter().find(|w| w.id == id).cloned().unwrap();
    (app, window)
}

fn names(app: &PhotocraftApp) -> Vec<String> {
    app.session.documents().iter().map(|d| d.doc.name.clone()).collect()
}

#[test]
fn closing_a_floating_documents_window_closes_the_document() {
    let (mut app, window) = app_with_floating();
    assert!(closes_document(&app, &window));
    let first = names(&app)[0].clone();
    close_document_of(&mut app, &egui::Context::default(), &window);
    assert_eq!(app.session.documents().len(), 1, "the document is closed, not put back in a tab");
    assert!(!names(&app).contains(&first));
    app.sync_views();
    assert!(app.ui.windows.is_empty(), "its window goes with it");
}

#[test]
fn unsaved_changes_ask_first_and_keep_the_window() {
    let (mut app, window) = app_with_floating();
    app.session.set_active(0);
    app.run("layer.new.layer", json!({})).unwrap();
    assert!(app.session.documents()[0].is_dirty());
    close_document_of(&mut app, &egui::Context::default(), &window);
    assert!(app.discard.is_some(), "Save / Don't Save / Cancel is asked");
    assert_eq!(app.session.documents().len(), 2, "nothing is closed before the answer");
    assert!(app.ui.windows.iter().any(|w| w.id == window.id && w.open), "the window stays while the question is up");
    // A second click on the window's × while the question is up asks nothing new.
    close_document_of(&mut app, &egui::Context::default(), &window);
    assert_eq!(app.session.documents().len(), 2);
}

#[test]
fn closing_a_second_view_only_closes_that_window() {
    let (mut app, floating) = app_with_floating();
    // New Window for Document: a second view of a document that keeps its tab.
    let view = crate::view_cmds::float_window(&mut app, 1, 0, false);
    let view = app.ui.windows.iter().find(|w| w.id == view).cloned().unwrap();
    assert!(!closes_document(&app, &view), "the document keeps its tab");
    // A floating document also shown in a second window: closing one leaves the other.
    let other = crate::view_cmds::float_window(&mut app, 0, 0, false);
    assert!(!closes_document(&app, &floating), "another window still shows the document");
    app.ui.windows.retain(|w| w.id != other);
    assert!(closes_document(&app, &floating));
}
