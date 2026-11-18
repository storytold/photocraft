//! Input-method events reach a session through the canvas editor. Each test drives a full app
//! frame with `egui::Event::Ime`; the composition rules themselves belong to the framework
//! editor.

use egui::Modifiers;
use photocraft_doc::LayerId;
use serde_json::json;

use super::commit_tests::{edit_active, ime, type_keys};
use super::mask_replace_selection_tests::click;
use super::{commit, end_if_left};
use crate::PhotocraftApp;
use crate::canvas::testing;

/// A document with a Type session on new text reading "abc", the caret at its end.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 400, "height": 200})).unwrap();
    app.sync_views();
    app.ui.tool = crate::state::Tool::Type;
    click(&mut app, 50.0, 100.0, Modifiers::NONE);
    testing::frame(&mut app);
    type_keys(&mut app, "abc");
    app
}

fn layer_text(app: &PhotocraftApp) -> String {
    let ed = app.ui.text_edit.as_ref().expect("editing");
    super::current_text(app, LayerId(ed.layer)).expect("text layer")
}

fn set_selection(app: &mut PhotocraftApp, anchor: usize, caret: usize) {
    let ed = app.ui.text_edit.as_mut().expect("editing");
    ed.anchor = anchor;
    ed.caret = caret;
}

fn preedit(text: &str) -> egui::ImeEvent {
    egui::ImeEvent::Preedit { text: text.to_owned(), active_range_chars: None }
}

fn ime_commit(text: &str) -> egui::ImeEvent {
    egui::ImeEvent::Commit(text.to_owned())
}

/// An engine edit (`type.edit`) reaches the layer outside the editor. The editor's composition
/// does not survive it, and the commit that arrives late changes nothing.
#[test]
fn external_edit_invalidates_the_preedit_and_rejects_its_late_commit() {
    let mut app = app();
    // Compose in the middle of committed text, so an edit before it moves a still-in-range (but
    // wrong) character range.
    type_keys(&mut app, "def");
    set_selection(&mut app, 3, 3);
    let mut app = app;
    ime(&mut app, preedit("に"));
    assert_eq!(layer_text(&app), "abcにdef");
    let layer = app.ui.text_edit.as_ref().unwrap().layer;
    let session = app.ui.text_edit.as_ref().unwrap().session.clone();
    app.run("type.edit", json!({"layer": layer, "replace": {"start": 0, "end": 1, "text": ""}, "coalesce": session})).unwrap();
    assert_eq!(layer_text(&app), "bcにdef");
    let mut app = app;
    testing::frame(&mut app);
    let revision = app.session.active().unwrap().revision;
    let history = app.session.active().unwrap().history.entries();
    // The commit was already on its way when the composition ended: it arrives before the platform answers the reset.
    testing::hold_ime_reset_answer(&app);
    ime(&mut app, ime_commit("に"));
    assert_eq!(layer_text(&app), "bcにdef", "the late commit edits nothing");
    assert_eq!(app.session.active().unwrap().revision, revision);
    assert_eq!(app.session.active().unwrap().history.entries(), history);
}

/// A session moved to another layer is untouched by the composition of the old one.
#[test]
fn target_change_interrupts_and_rejects_the_late_commit() {
    let mut app = app();
    ime(&mut app, preedit("に"));
    assert_eq!(layer_text(&app), "abcに");
    let created = app.run("type.create", json!({"text": "two", "x": 150, "y": 60})).unwrap();
    let second = created["layer"].as_u64().expect("new layer");
    app.session.select_layer(LayerId(second)).expect("select the second type layer");
    end_if_left(&mut app);
    edit_active(&mut app);
    set_selection(&mut app, 3, 3);
    let mut app = app;
    testing::frame(&mut app);
    assert_eq!(super::current_text(&app, LayerId(second)).unwrap(), "two", "the old composition does not touch the new session");
    let mut app = app;
    ime(&mut app, preedit("が"));
    assert_eq!(layer_text(&app), "twoが");
}

/// An input method's delete-surrounding is a deletion edit that coalesces into the session's one
/// undo step.
#[test]
fn delete_surrounding_characters_is_one_undo_step() {
    let mut app = app();
    commit(&mut app);
    edit_active(&mut app);
    set_selection(&mut app, 3, 3);
    let id = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
    ime(&mut app, egui::ImeEvent::DeleteSurrounding { before_chars: 1, after_chars: 0 });
    assert_eq!(super::current_text(&app, id).unwrap(), "ab");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(super::current_text(&app, id).unwrap(), "abc", "the delete and its session coalesce into one step");
}

/// A composition begun in one document does not edit the text of the document File › New makes.
#[test]
fn document_replacement_cannot_reuse_the_previous_documents_reset() {
    let mut app = app();
    ime(&mut app, preedit("P"));
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    app.sync_views();
    // The host runs this before handling input: a new active document ends the old edit in its
    // own document, so the next canvas click starts an editor for the new one.
    end_if_left(&mut app);
    assert!(app.ui.text_edit.is_none(), "the new document starts without the previous edit target");
    app.ui.tool = crate::state::Tool::Type;
    click(&mut app, 50.0, 100.0, Modifiers::NONE);
    testing::frame(&mut app);
    type_keys(&mut app, "new");
    let mut app = app;
    testing::frame(&mut app);
    assert_eq!(layer_text(&app), "new", "the old composition does not reach the new text");
    let mut app = app;
    ime(&mut app, preedit("が"));
    assert!(layer_text(&app).contains("が"));
}
