//! #11793: the Notes panel with no note selected shows an empty, inactive text box with the
//! placeholder "Click a note to view or edit", a blank counter and greyed arrows and trash,
//! and selects no note by itself; the author line is empty for a note without author; the
//! counter reads "N / M"; a note made with a click puts the caret in its text.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::*;

/// A 300 × 200 document with two notes (the second without author) and the Notes panel open,
/// nothing selected; the harness draws the analysis windows.
fn harness() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    app.run("notes.add", json!({"x": 20, "y": 20, "author": "Ann", "text": "first"})).unwrap();
    app.run("notes.add", json!({"x": 120, "y": 80, "author": "", "text": "second"})).unwrap();
    app.ui.analysis.notes = true;
    app.ui.analysis.note_selected = None;
    let mut h = Harness::builder().with_size(vec2(1400.0, 900.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            super::windows(app, ui.ctx());
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h
}

fn disabled(h: &Harness<'static, PhotocraftApp>, label: &str) -> bool {
    h.get_by_label(label).accesskit_node().is_disabled()
}

#[test]
fn the_counter_reads_n_of_m_with_a_slash() {
    assert_eq!(note_counter(Some(0), 4), "1 / 4");
    assert_eq!(note_counter(Some(3), 4), "4 / 4");
    assert_eq!(note_counter(None, 4), "");
    assert_eq!(note_counter(Some(5), 4), "", "a stale selection reads as none");
}

#[test]
fn no_selection_stays_empty_with_greyed_controls() {
    let h = harness();
    assert_eq!(h.state().ui.analysis.note_selected, None, "note 1 isn't selected by itself");
    assert!(h.query_by_label("1 / 2").is_none() && h.query_by_label("2 / 2").is_none(), "blank counter");
    for label in ["Previous note", "Next note", "Delete note"] {
        assert!(disabled(&h, label), "{label} is greyed out");
    }
    assert!(h.query_by_label("first").is_none(), "no note's text is shown");
}

#[test]
fn a_selected_note_shows_its_counter_and_an_empty_author_line() {
    let mut h = harness();
    h.state_mut().ui.analysis.note_selected = Some(1);
    h.run_steps(3);
    assert!(h.query_by_label("2 / 2").is_some());
    assert!(h.query_by_label("No author").is_none(), "an empty author stays empty");
    assert!(!disabled(&h, "Next note") && !disabled(&h, "Delete note"));
    h.state_mut().ui.analysis.note_selected = Some(0);
    h.run_steps(3);
    assert!(h.query_by_label("1 / 2").is_some());
    assert!(h.query_by_label("Ann").is_some(), "the stored author");
}

#[test]
fn a_note_made_with_a_click_takes_the_keyboard() {
    let mut h = harness();
    h.state_mut().ui.tool = Tool::Note;
    crate::canvas::tool_event(h.state_mut(), ToolEvent::Down { x: 200.0, y: 150.0, pressure: 1.0 }, egui::Modifiers::NONE);
    crate::canvas::tool_event(h.state_mut(), ToolEvent::Up { x: 200.0, y: 150.0 }, egui::Modifiers::NONE);
    h.run_steps(3);
    assert_eq!(h.state().ui.analysis.note_selected, Some(2));
    assert!(h.ctx.memory(|m| m.focused().is_some()), "the note's text box has the keyboard");
    h.event(egui::Event::Text("hello".into()));
    h.run_steps(3);
    assert_eq!(h.state().session.active().unwrap().doc.notes[2].text, "hello", "typing goes into the new note");
}
