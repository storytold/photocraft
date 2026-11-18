//! #11788: with the Note tool, ⌫/Delete on a selected note asks "Delete this note?" and deletes
//! the note, never the layer.

use egui_kittest::Harness;
use serde_json::json;

use super::*;
use crate::lifecycle_tests::app_harness;

/// The whole app with a second layer and one note, the Note tool on and the note selected.
fn harness() -> Harness<'static, PhotocraftApp> {
    let (mut h, _transport) = app_harness(crate::Services::default(), 1);
    let app = h.state_mut();
    app.run("layer.new.layer", json!({"name": "Ink"})).unwrap();
    app.run("notes.add", json!({"x": 10, "y": 12, "text": "fix this"})).unwrap();
    app.ui.tool = Tool::Note;
    app.ui.analysis.note_selected = Some(0);
    h.run_steps(3);
    h
}

fn layers(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.layers.len()
}

fn notes(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.notes.len()
}

fn prompt(app: &PhotocraftApp) -> Option<egui::Id> {
    crate::dialogs::alerts::prompt_tagged::<NoteQuestion>(app)
}

/// The app on its own context, driven by the canvas test driver: a key reaches the canvas through the window host, and
/// the Delete the Note tool claimed is run by the key route in the pass after its claim.
fn driven() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 60})).unwrap();
    app.run("layer.new.layer", json!({"name": "Ink"})).unwrap();
    app.run("notes.add", json!({"x": 10, "y": 12, "text": "fix this"})).unwrap();
    app.ui.tool = Tool::Note;
    app.sync_views();
    crate::canvas::testing::frame(&mut app);
    // A click on the note selects it and focuses the canvas, where a Delete reaches the tool.
    let at = [10.5, 13.0];
    crate::canvas::testing::move_to(&mut app, at, egui::Modifiers::NONE);
    crate::canvas::testing::press(&mut app, at, egui::Modifiers::NONE);
    crate::canvas::testing::release(&mut app, at, egui::Modifiers::NONE);
    assert_eq!(app.ui.analysis.note_selected, Some(0), "the click selected the note");
    crate::canvas::testing::frame(&mut app);
    app
}

#[test]
fn delete_with_a_note_selected_asks_and_deletes_the_note_not_the_layer() {
    for key in [egui::Key::Delete, egui::Key::Backspace] {
        let mut app = driven();
        let steps = app.session.active().unwrap().history.past_len();
        crate::canvas::testing::key(&mut app, key, egui::Modifiers::NONE);
        let id = prompt(&app).unwrap_or_else(|| panic!("{key:?}: the alert"));
        assert_eq!((layers(&app), notes(&app)), (2, 1), "{key:?}: nothing changes before the answer");
        let ctx = app.ctx().clone();
        crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
        assert_eq!((layers(&app), notes(&app)), (2, 0), "{key:?}: the note goes, the layers stay");
        let st = app.session.active().unwrap();
        assert_eq!(st.history.past_len(), steps + 1);
        assert_eq!(st.history.undo_label(), Some("Delete Note"));
        assert_eq!(app.ui.analysis.note_selected, None, "nothing selected after");
    }
}

#[test]
fn dont_show_again_skips_the_alert_next_time() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    let app = h.state_mut();
    app.run("notes.add", json!({"x": 30, "y": 30})).unwrap();
    ask_delete_note(app, 0);
    let id = prompt(app).expect("the alert");
    app.framework.dialogs.set_json_field(&ctx, id, "skip", json!(true)).unwrap();
    crate::dialogs::confirm(app, &ctx, id).unwrap();
    assert_eq!(notes(app), 1);
    // The next ask deletes at once.
    ask_delete_note(app, 0);
    assert!(prompt(app).is_none());
    assert_eq!((layers(app), notes(app)), (2, 0));
}

#[test]
fn without_a_selected_note_delete_keeps_its_usual_route() {
    let mut h = harness();
    h.state_mut().ui.analysis.note_selected = None;
    h.key_press(egui::Key::Delete);
    h.run_steps(3);
    assert!(prompt(h.state()).is_none());
    assert_eq!(notes(h.state()), 1, "the note stays");
    // Another tool with a note still selected: the note isn't asked about.
    let mut other = harness();
    other.state_mut().ui.tool = Tool::Move;
    other.key_press(egui::Key::Delete);
    other.run_steps(3);
    assert!(prompt(other.state()).is_none());
    assert_eq!(notes(other.state()), 1);
}
