//! File › Save asks before replacing the document's own file when another app changed it after
//! the document was opened from it or last saved to it (#6596). Replace writes the document over
//! it anyway; Cancel leaves the file and the document as they are, and File › Save As keeps both.
//!
//! The prompt is a `StandardPrompt` on the dialog host: `ui.dialog.confirm` replaces the file and
//! `ui.dialog.cancel` keeps it. Recorded actions and automation saves don't ask.

use craft::ui::dialog::{PromptAnswer, PromptChoice, StandardPrompt};
use photocraft_doc::DocId;
use serde_json::{Value, json};

use super::alerts;
use crate::PhotocraftApp;

/// The save the prompt holds back: `doc` to `path`, with the format answers a script gave. `flat` is
/// a flat file confirmed in the Save dialog.
pub(crate) struct HeldSave {
    doc: DocId,
    pub(crate) path: String,
    flat: bool,
    choices: crate::FormatChoices,
}

/// Opens the prompt instead of saving `doc` to its own file at `path` when another app changed
/// that file; `None` saves as usual. `flat` is a flat file confirmed in the Save dialog, which
/// is saved (and recorded) the way [`PhotocraftApp::save_dialog_flow`] does it.
pub(crate) fn intercept(app: &mut PhotocraftApp, doc: DocId, path: &str, flat: bool, choices: crate::FormatChoices) -> Option<Result<Value, String>> {
    if !app.changed_on_disk(doc, path) {
        return None;
    }
    let prompt = StandardPrompt::changed_on_disk(crate::file_open::display_name(path)).tag(HeldSave { doc, path: path.to_owned(), flat, choices });
    Some(Ok(match alerts::open_prompt(app, prompt) {
        Ok(id) => json!({"dialog": id.value(), "confirmationPending": true}),
        Err(_) => json!({"confirmationPending": true}),
    }))
}

/// The `PROMPTS` entry: Replace runs the save the prompt held back, Cancel leaves the file. `false`
/// for another family's answer.
pub(crate) fn answer(app: &mut PhotocraftApp, _ctx: &egui::Context, answer: &mut PromptAnswer) -> bool {
    let Some(held) = answer.take_tag::<HeldSave>() else { return false };
    if answer.choice == PromptChoice::Proceed {
        let result = replace(app, held);
        alerts::report(app, result);
    }
    true
}

/// Replace: the save the prompt held back.
fn replace(app: &mut PhotocraftApp, held: HeldSave) -> Result<Value, String> {
    let HeldSave { doc, path, flat, choices } = held;
    app.with_document(doc, |app| {
        if flat {
            let from = app.session.journal.len();
            let r = app.save_to(path.clone(), choices)?;
            app.journal_as_plain_save(&path, from);
            Ok(r)
        } else if app.session.active().is_some_and(|st| crate::saves_flat_in_place(&st.doc, &path)) {
            // Its own flat file: a JPEG, TIFF, Targa or DDS texture asks its format's options next (#2223).
            app.save_flat_in_place(path, choices)
        } else if app.session.active().is_some_and(|st| photocraft_engine::file_cmds::holds_at(&st.doc, &path)) {
            app.save_in_place(path, crate::FormatChoices::default())
        } else {
            // The document changed while the prompt was open and its file can't keep it any more:
            // Save As, never a write over the original with less (#5733).
            app.save_dialog_flow(None, choices, true)
        }
    })
}
