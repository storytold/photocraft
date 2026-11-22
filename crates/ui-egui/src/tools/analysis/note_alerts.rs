//! The Note tool's alerts: "Delete this note?" (#11788) and "Are you sure you want to delete all
//! notes?" (#11791), as `StandardPrompt`s tagged with the question they ask.

use craft::ui::dialog::{PromptAnswer, PromptChoice, StandardPrompt};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::Tool;

/// The "Don't show again" choice of the delete question, kept with the other dialog preferences.
pub(crate) const DELETE_NOTE_PREF: &str = "notes.delete.confirmation";

/// Which question a prompt asks. `document` is the id of the document the question was asked in, so
/// an answer after the document changed deletes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NoteQuestion {
    /// Delete note `index` of `document`.
    Delete {
        /// The document's id.
        document: u64,
        /// The note's index.
        index: usize,
    },
    /// Delete every note of `document`.
    ClearAll {
        /// The document's id.
        document: u64,
    },
}

fn report(app: &mut PhotocraftApp, message: String) {
    app.ui.status = message;
    app.ui.status_error = true;
}

/// The note index Delete and Backspace act on: the Note tool is active, a note of the active document
/// is selected and no text is being edited. The tool host adds the focus and modifier rules.
pub(crate) fn deletable(app: &PhotocraftApp) -> Option<usize> {
    let index = app.ui.analysis.note_selected?;
    let in_range = app.session.active().is_some_and(|s| index < s.doc.notes.len());
    (app.ui.tool == Tool::Note && app.ui.text_edit.is_none() && in_range).then_some(index)
}

/// The Note tool's Clear All: asks first; OK deletes every note in one "Delete all Notes" step.
/// Nothing happens without notes in the active document.
pub(crate) fn ask_clear_notes(app: &mut PhotocraftApp) {
    let Some(document) = app.session.active().filter(|s| !s.doc.notes.is_empty()).map(|s| s.doc.id.0) else { return };
    app.framework.dialogs.open(
        StandardPrompt::custom(tl!("Are you sure you want to delete all notes?"))
            .title(tl!("Clear All"))
            .button(PromptChoice::Proceed, tl!("OK"))
            .button(PromptChoice::Cancel, tl!("Cancel"))
            .tag(NoteQuestion::ClearAll { document }),
    );
}

/// Deletes note `index` of the active document after the alert "Delete this note?", or at once when
/// "Don't show again" was ticked before. Nothing is selected afterwards.
pub(crate) fn ask_delete_note(app: &mut PhotocraftApp, index: usize) {
    let skip = app.session.prefs().dialogs.get(DELETE_NOTE_PREF).and_then(|v| v.get("skip")).and_then(Value::as_bool) == Some(true);
    let Some(document) = app.session.active().map(|s| s.doc.id.0) else { return };
    if skip {
        match app.run("notes.delete", json!({"index": index})) {
            Ok(_) => app.ui.analysis.note_selected = None,
            Err(e) => report(app, e),
        }
        return;
    }
    app.framework.dialogs.open(
        StandardPrompt::custom(tl!("Delete this note?"))
            .title(tl!("Delete Note"))
            .button(PromptChoice::Proceed, tl!("Yes"))
            .button(PromptChoice::Cancel, tl!("No"))
            .dont_show_again()
            .tag(NoteQuestion::Delete { document, index }),
    );
}

/// The `PROMPTS` entry: acts on the answer of a note question and says whether the prompt was one.
pub(crate) fn answer(app: &mut PhotocraftApp, _ctx: &egui::Context, answer: &mut PromptAnswer) -> bool {
    let Some(question) = answer.take_tag::<NoteQuestion>() else { return false };
    if !matches!(answer.choice, PromptChoice::Proceed) {
        return true;
    }
    let done = match question {
        NoteQuestion::Delete { document, index } => delete_confirmed(app, document, index, answer.skip),
        NoteQuestion::ClearAll { document } => clear_confirmed(app, document),
    };
    if let Err(message) = done {
        report(app, message);
    }
    true
}

/// OK of the delete question: the note goes (one "Delete Note" step) and "Don't show again" is kept.
fn delete_confirmed(app: &mut PhotocraftApp, document: u64, index: usize, skip: bool) -> Result<(), String> {
    let current = app.session.active().map(|s| (s.doc.id.0, s.doc.notes.len()));
    if current.is_none_or(|(id, len)| id != document || index >= len) {
        return Err(tl!("The document or its notes changed. Please delete the note again.").into());
    }
    app.run("notes.delete", json!({"index": index}))?;
    app.ui.analysis.note_selected = None;
    if skip {
        app.session.edit_prefs(|prefs| {
            prefs.dialogs.insert(DELETE_NOTE_PREF.into(), json!({"skip": true}));
        });
        if let Err(error) = crate::prefs_ui::save_preferences(app) {
            report(app, crate::i18n::fmt(tl!("Note deleted, but the preference could not be saved: {error}"), &[("error", &error)]));
        }
    }
    Ok(())
}

/// OK of Clear All: every note goes in one step, nothing stays selected, and the options bar shows the
/// tool defaults again (a selected note had put its author and colour there).
fn clear_confirmed(app: &mut PhotocraftApp, document: u64) -> Result<(), String> {
    if app.session.active().map(|s| s.doc.id.0) != Some(document) {
        return Err(tl!("The document changed. Please choose Clear All again.").into());
    }
    app.run("notes.delete", json!({"all": true}))?;
    let defaults = super::AnalysisUi::default();
    let a = &mut app.ui.analysis;
    (a.note_selected, a.note_author, a.note_color) = (None, defaults.note_author, defaults.note_color);
    Ok(())
}

#[cfg(test)]
#[path = "../note_delete_key_tests.rs"]
mod note_delete_key_tests;

#[cfg(test)]
#[path = "../note_clear_all_tests.rs"]
mod note_clear_all_tests;
