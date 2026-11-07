//! Closing a document's own OS window closes the document, as in Photoshop.

use serde_json::json;

use super::{close_document, close_documents, closes_document};
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
    close_document(&mut app, &egui::Context::default(), window.document);
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
    close_document(&mut app, &egui::Context::default(), window.document);
    assert!(app.discard.is_some(), "Save / Don't Save / Cancel is asked");
    assert_eq!(app.session.documents().len(), 2, "nothing is closed before the answer");
    assert!(app.ui.windows.iter().any(|w| w.id == window.id && w.open), "the window stays while the question is up");
    // A second click on the window's × while the question is up asks nothing new.
    close_document(&mut app, &egui::Context::default(), window.document);
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

/// One frame of the documents' windows in which every window is asked to close. Without a window
/// system egui embeds them in the main window, so they read its close request.
fn frame_closing_every_window(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let info = egui::ViewportInfo { events: vec![egui::ViewportEvent::Close], ..Default::default() };
    let raw = egui::RawInput { viewports: std::iter::once((egui::ViewportId::ROOT, info)).collect(), ..Default::default() };
    ctx.run_ui(raw, |ui| super::extra_windows(app, ui.ctx())).textures_delta.clear();
}

/// Two windows closed in the same frame (the taskbar's Close all windows): both documents close,
/// each found by id although closing the first renumbers the second (CodeRabbit on #11665).
#[test]
fn every_window_closed_in_one_frame_closes_its_document() {
    let (mut app, _) = app_with_floating();
    crate::view_cmds::float_window(&mut app, 1, 0, true);
    assert_eq!(app.ui.windows.iter().filter(|w| w.floating).count(), 2);
    frame_closing_every_window(&mut app, &egui::Context::default());
    assert!(app.session.documents().is_empty(), "both documents are closed, not only the last request's");
    app.sync_views();
    assert!(app.ui.windows.is_empty());
}

/// A close that fails leaves the document open and says why in a notice, which stays on screen
/// in the main window (CodeRabbit on #11665).
#[test]
fn a_failed_close_is_reported_in_a_notice() {
    let (mut app, _) = app_with_floating();
    let notices = app.ui.notices.len();
    close_document(&mut app, &egui::Context::default(), 99);
    assert_eq!(app.session.documents().len(), 2, "nothing is closed");
    assert_eq!(app.ui.notices.len(), notices + 1, "a notice says why");
    assert!(app.ui.status_error);
}

/// When one of them asks about unsaved changes, the others wait with their windows open.
#[test]
fn closing_waits_while_unsaved_changes_are_asked_about() {
    let (mut app, _) = app_with_floating();
    let _second = crate::view_cmds::float_window(&mut app, 1, 0, true);
    app.session.set_active(0);
    app.run("layer.new.layer", json!({})).unwrap();
    let ids: Vec<_> = app.session.documents().iter().map(|d| d.doc.id).collect();
    close_documents(&mut app, &egui::Context::default(), ids);
    assert!(app.discard.is_some(), "the unsaved document asks first");
    assert_eq!(app.session.documents().len(), 2, "the other document waits: nothing closes unasked");
}
