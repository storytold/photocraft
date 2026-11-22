//! Importing a brush (`.abr`) or gradient (`.grd`) file asks first when the library already has
//! a group of the file's name with presets of the user's (#10640). The group key is only the file
//! name, so it can be another author's set, or hold brushes the user edited or moved there, and
//! the preset libraries are outside document history.
//!
//! Import as New Group (the default) imports beside it as "Kit (2)"; Replace deletes the group's
//! presets first, which can't be undone; Cancel imports nothing. Without a clash the file imports
//! at once, as before.
//!
//! The prompt is a `StandardPrompt` on the dialog host and the held import travels in its tag:
//! `ui.inspect` shows it, `ui.dialog.confirm` imports as a new group, `ui.dialog.set` with
//! `"choice": "alternate"` then `ui.dialog.confirm` replaces, and `ui.dialog.cancel` imports nothing.

use craft::ui::dialog::{PromptAnswer, PromptChoice, StandardPrompt};
use serde_json::{Value, json};

use super::alerts;
use crate::PhotocraftApp;

/// An import held back while the prompt is up.
pub struct Held {
    pub cmd: &'static str,
    pub what: &'static str,
    pub stem: String,
    pub params: Value,
}

/// The tag of the prompt: the import it holds back.
pub(crate) struct PresetClash(Held);

/// How many of the user's presets a group named `group` holds in the library `cmd` imports to
/// (built-in brushes are never replaced, so they don't count).
fn clashing(app: &PhotocraftApp, cmd: &str, group: &str) -> usize {
    match cmd {
        "brush.presets.importAbr" => app.session.tools.presets.iter().filter(|x| !x.builtin && x.group == group).count(),
        "gradient.presets.importGrd" => app.session.presets.gradients.iter().filter(|g| g.name == group).map(|g| g.items.len()).sum(),
        _ => 0,
    }
}

/// Opens the prompt for `held` when its group clashes; otherwise hands `held` back to import now.
pub(crate) fn intercept(app: &mut PhotocraftApp, held: Held) -> Result<Option<Held>, String> {
    let n = clashing(app, held.cmd, &held.stem);
    if n == 0 {
        return Ok(Some(held));
    }
    let (title, message) = if held.what == "gradients" {
        let text = crate::i18n::trn(
            crate::i18n::current(),
            n as u64,
            "The gradient group “{group}” already exists ({n} gradient).",
            "The gradient group “{group}” already exists ({n} gradients).",
        );
        (tl!("Import Gradients"), text)
    } else {
        let text = crate::i18n::trn(
            crate::i18n::current(),
            n as u64,
            "The brush group “{group}” already has {n} brush of yours.",
            "The brush group “{group}” already has {n} brushes of yours.",
        );
        (tl!("Import Brushes"), text)
    };
    let group = held.stem.clone();
    let prompt = StandardPrompt::custom(crate::i18n::fmt(&message, &[("group", &group)]))
        .title(title.to_owned())
        .paragraph(crate::i18n::fmt(tl!("Import as New Group keeps “{group}” and adds the file as a new group beside it."), &[("group", &group)]))
        .paragraph(crate::i18n::fmt(
            tl!("Replace deletes everything in “{group}” first, including presets you edited or moved there. This can't be undone."),
            &[("group", &group)],
        ))
        .button(PromptChoice::Proceed, tl!("Import as New Group").to_owned())
        .button(PromptChoice::Alternate, tl!("Replace").to_owned())
        .button(PromptChoice::Cancel, tl!("Cancel").to_owned())
        .tag(PresetClash(held));
    alerts::open_prompt(app, prompt).map_err(|_| tl!("Answer the open import question first, then import this file again.").to_owned())?;
    Ok(None)
}

/// The `PROMPTS` entry: Import as New Group keeps the existing group, Replace imports into its
/// place, Cancel imports nothing. `false` for another family's answer.
pub(crate) fn answer(app: &mut PhotocraftApp, _ctx: &egui::Context, answer: &mut PromptAnswer) -> bool {
    let Some(PresetClash(held)) = answer.take_tag::<PresetClash>() else { return false };
    let replace = match answer.choice {
        PromptChoice::Proceed => false,
        PromptChoice::Alternate => true,
        _ => return true,
    };
    let Held { cmd, what, stem, mut params } = held;
    params["replace"] = json!(replace);
    if let Err(error) = crate::preset_files_ui::run_import(app, cmd, what, &stem, params) {
        alerts::report_error(app, error);
    }
    true
}

#[cfg(test)]
mod tests;
