//! Edit › Assign Profile on a document with layers first asks whether to go on (#14235): giving the document another
//! profile can change how its layers look. A warning icon beside the text (#16683), OK (the default) and Cancel, and a
//! "Don't show again" box kept with the other dialog preferences. OK opens the Assign Profile dialog; Cancel or Escape
//! ends the command with nothing changed. A document with only a Background layer never asks, nor does
//! `edit.assignProfile` run with params (scripts, actions, agents, the control channel), which the menu arm leaves to
//! the command.

use craft::behavior::IconId;
use craft::ui::dialog::{DialogId, PromptAnswer, PromptChoice, StandardPrompt};

use super::alerts;
use crate::PhotocraftApp;

/// Its "Don't show again", under `Preferences::dialogs`.
pub const PREF: &str = "edit.assignProfile.layers";

/// PhotoCraft's own wording of the reference application's warning.
const QUESTION: &str = "Giving the document another profile can change how its layers look. Assign a profile anyway?";

/// The tag of the open question.
pub(crate) struct AssignProfileAsk;

/// Whether the menu's Assign Profile asks first: the active document has anything but a lone Background layer, and
/// "Don't show again" was not ticked.
pub fn asks(app: &PhotocraftApp) -> bool {
    let Some(st) = app.session.active() else { return false };
    let flat = st.doc.layers.len() == 1 && st.doc.background().is_some();
    !flat && !alerts::dismissed(app, PREF)
}

/// Opens the question; the id of the question already open when there is one.
pub fn open(app: &mut PhotocraftApp) -> DialogId {
    let prompt = StandardPrompt::custom(tl!(QUESTION).to_owned())
        .title(tl!("Assign Profile").to_owned())
        .icon(IconId::new("warning"))
        .button(PromptChoice::Proceed, tl!("OK").to_owned())
        .button(PromptChoice::Cancel, tl!("Cancel").to_owned())
        .dont_show_again()
        .tag(AssignProfileAsk);
    alerts::open_prompt(app, prompt).unwrap_or_else(|open| open)
}

/// The `PROMPTS` entry: OK keeps a ticked "Don't show again" and opens the Assign Profile dialog; Cancel changes
/// nothing. `false` for another family's answer.
pub(crate) fn answer(app: &mut PhotocraftApp, _ctx: &egui::Context, answer: &mut PromptAnswer) -> bool {
    if answer.take_tag::<AssignProfileAsk>().is_none() {
        return false;
    }
    if answer.choice != PromptChoice::Proceed {
        return true;
    }
    if answer.skip {
        alerts::remember_skip(app, PREF);
    }
    if let Err(error) = crate::filter_dialog::open(app, "edit.assignProfile") {
        alerts::report_error(app, error);
    }
    true
}
