//! #11791: the Note tool's Clear All asks "Are you sure you want to delete all notes?" first. OK
//! deletes every note in one "Delete all Notes" step that one undo restores; Cancel changes
//! nothing; `notes.delete {"all": true}` itself never asks.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::*;

/// A 200 × 100 document with three notes, the second one selected, and the Note tool.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    for (x, author) in [(10, "A"), (60, "B"), (110, "C")] {
        app.run("notes.add", json!({"x": x, "y": 20, "author": author})).unwrap();
    }
    app.ui.tool = Tool::Note;
    app.ui.analysis.note_selected = Some(1);
    app
}

fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1200.0, 80.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::options_bar(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(4);
    h
}

fn notes(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.notes.len()
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

/// The open Clear All alert's id.
fn alert(app: &PhotocraftApp) -> Option<u64> {
    app.ui.dialogs.iter().find(|d| owns_clear_notes(&d.fields)).map(|d| d.id)
}

/// Click the options bar's Clear All and return the alert it opened.
fn clear_all(h: &mut Harness<'static, PhotocraftApp>) -> u64 {
    h.get_by_label("Clear All").click();
    h.run_steps(3);
    alert(h.state()).expect("Clear All opens the alert")
}

#[test]
fn clear_all_asks_first_and_cancel_changes_nothing() {
    let mut h = harness(app());
    let before = steps(h.state());
    let id = clear_all(&mut h);
    assert_eq!(notes(h.state()), 3, "nothing is deleted before the answer");
    let ctx = egui::Context::default();
    let mut texts = Vec::new();
    let mut o = ctx.run_ui(egui::RawInput::default(), clear_notes_body);
    o.textures_delta.clear();
    for c in o.shapes {
        if let egui::Shape::Text(t) = c.shape {
            texts.push(t.galley.text().to_owned());
        }
    }
    assert!(texts.iter().any(|t| t == "Are you sure you want to delete all notes?"), "{texts:?}");
    crate::dialogs::cancel(h.state_mut(), id).unwrap();
    assert_eq!(notes(h.state()), 3);
    assert_eq!(steps(h.state()), before, "no history step");
    assert_eq!(h.state().ui.analysis.note_selected, Some(1), "the selection stays");
}

#[test]
fn ok_deletes_every_note_in_one_step_that_undo_restores() {
    let mut h = harness(app());
    let before = steps(h.state());
    let id = clear_all(&mut h);
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    h.run_steps(3);
    let app = h.state();
    assert_eq!(notes(app), 0);
    assert_eq!(steps(app), before + 1, "one history step");
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Delete all Notes"));
    assert_eq!(app.ui.analysis.note_selected, None, "nothing selected after OK");
    assert!(h.get_by_label("Clear All").accesskit_node().is_disabled(), "Clear All greys without notes");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(notes(h.state()), 3, "one undo brings every note back");
}

#[test]
fn the_command_itself_never_asks_and_names_its_step_too() {
    let mut app = app();
    app.run("notes.delete", json!({"all": true})).unwrap();
    assert_eq!(notes(&app), 0);
    assert!(alert(&app).is_none(), "no alert from the command");
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Delete all Notes"));
}

#[test]
fn clear_all_stays_grey_without_notes() {
    let mut app = app();
    app.run("notes.delete", json!({"all": true})).unwrap();
    ask_clear_notes(&mut app);
    assert!(alert(&app).is_none(), "nothing to ask about");
    let h = harness(app);
    assert!(h.get_by_label("Clear All").accesskit_node().is_disabled());
}

#[test]
fn a_second_click_while_the_alert_is_open_opens_no_second_alert() {
    let mut h = harness(app());
    clear_all(&mut h);
    ask_clear_notes(h.state_mut());
    assert_eq!(h.state().ui.dialogs.iter().filter(|d| owns_clear_notes(&d.fields)).count(), 1);
}
