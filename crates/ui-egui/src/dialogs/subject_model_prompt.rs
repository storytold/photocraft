//! The one-time offer of the subject model: Remove Background and Select Subject ask before their
//! first use, because the download is 367 MB and has terms a person agrees to. Download runs the
//! command when the model is in; Use Built-in Method (the default button, so Enter never agrees to
//! terms) runs it without the model, and with "Don't show again" ticked it does not ask again.

use craft::ui::dialog::{PromptAnswer, PromptChoice, StandardPrompt};
use photocraft_engine::subject_model::ModelStatus;
use photocraft_engine::subject_model_cmds::INSTALL_CMD;
use serde_json::{Value, json};

use super::alerts;
use crate::PhotocraftApp;

/// Its "Don't show again", under `Preferences::dialogs`.
pub const PREF: &str = "select.subjectModel.offer";

/// The command waiting for the model's download.
pub struct Pending {
    command: String,
    params: Value,
    target: Target,
    /// The download's job: only its own end goes on with the command, or drops it.
    job: u64,
}

/// What a command was asked on: the document and its active layer then.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Target {
    document: u64,
    layer: Option<u64>,
}

/// What the offer was asked for: `command` with `params`, on `target`.
pub(crate) struct SubjectModelAsk {
    command: String,
    params: Value,
    target: Target,
}

/// The active document and layer now.
fn target(app: &PhotocraftApp) -> Option<Target> {
    let d = app.session.active()?;
    Some(Target { document: d.doc.id.0, layer: d.active_layer.map(|id| id.0) })
}

/// The message when the document or layer changed under the offer.
fn changed() -> String {
    tl!("The document or layer changed. Please run the command again.").into()
}

/// Opens the offer before Remove Background or Select Subject when the model can be downloaded
/// and the person has not been asked to stop seeing it. `None` lets the command run as asked.
pub fn intercept(app: &mut PhotocraftApp, command: &str, params: &Value) -> Option<Result<Value, String>> {
    if app.automation_input
        || !matches!(command, "layer.removeBackground" | "select.subject")
        || params.get("method").is_some()
        || alerts::dismissed(app, PREF)
        || !crate::menus::is_enabled(app, command)
    {
        return None;
    }
    let ModelStatus::Offer(offer) = app.session.subject_model()?.status() else { return None };
    let asked_on = target(app)?;
    let size = (offer.download_bytes / 1_000_000).to_string();
    let message = crate::i18n::fmt(
        tl!(
            "{model} finds subjects, hair and fur much better than the built-in method. It runs on this computer, so your pictures stay there. It needs a one-time download of {size} MB."
        ),
        &[("model", &offer.model), ("size", &size)],
    );
    let mut prompt = StandardPrompt::custom(message).title(tl!("Subject Model").to_owned());
    if !offer.folder.is_empty() {
        prompt = prompt.paragraph(crate::i18n::fmt(tl!("It will be saved in {folder}."), &[("folder", &offer.folder)]));
    }
    let prompt = prompt
        .details(tl!("Terms").to_owned(), offer.terms)
        .button(PromptChoice::Alternate, tl!("Download").to_owned())
        .button(PromptChoice::Proceed, tl!("Use Built-in Method").to_owned())
        .button(PromptChoice::Cancel, tl!("Cancel").to_owned())
        .dont_show_again()
        .tag(SubjectModelAsk { command: command.into(), params: params.clone(), target: asked_on });
    // An `Err` is a question already open (this one or another family's): the command waits for it.
    Some(Ok(match alerts::open_prompt(app, prompt) {
        Ok(id) => json!({"dialog": id, "confirmationPending": true}),
        Err(_) => json!({"confirmationPending": true}),
    }))
}

/// The `PROMPTS` entry: Download starts the download the command waits for, Use Built-in Method
/// runs it without the model (and keeps a ticked "Don't show again"), Cancel runs nothing.
/// `false` for another family's answer.
pub(crate) fn answer(app: &mut PhotocraftApp, _ctx: &egui::Context, answer: &mut PromptAnswer) -> bool {
    let Some(ask) = answer.take_tag::<SubjectModelAsk>() else { return false };
    let result = match answer.choice {
        PromptChoice::Proceed => built_in(app, ask, answer.skip),
        PromptChoice::Alternate => download(app, ask),
        _ => return true,
    };
    if let Err(error) = result {
        alerts::report_error(app, error);
    }
    true
}

/// Runs the asked command with the built-in method.
fn built_in(app: &mut PhotocraftApp, ask: SubjectModelAsk, skip: bool) -> Result<Value, String> {
    // A background open can make another document active while the prompt is open.
    if target(app) != Some(ask.target) {
        return Err(changed());
    }
    if skip {
        alerts::remember_skip(app, PREF);
    }
    let mut params = ask.params;
    if let Some(map) = params.as_object_mut() {
        map.insert("method".into(), json!("classic"));
    }
    app.run_command(&ask.command, params)
}

/// Starts the download the asked command waits for, or runs it when the model came in at once.
fn download(app: &mut PhotocraftApp, ask: SubjectModelAsk) -> Result<Value, String> {
    if target(app) != Some(ask.target) {
        return Err(changed());
    }
    let started = app.run_command(INSTALL_CMD, json!({"acceptTerms": true}))?;
    if started.get("pending").and_then(Value::as_bool) == Some(true) {
        if let Some(job) = started.get("job").and_then(Value::as_u64) {
            app.jobs.subject_model_pending = Some(Pending { command: ask.command, params: ask.params, target: ask.target, job });
        }
        return Ok(started);
    }
    app.run_command(&ask.command, ask.params)
}

/// The download ended: the command it waited for goes on, on the same document and layer. `true`
/// when that failed and the error is reported, so the download's own message must not replace it.
pub fn on_job_event(app: &mut PhotocraftApp, e: &photocraft_engine::jobs::JobEvent) -> bool {
    if e.command != INSTALL_CMD || app.jobs.subject_model_pending.as_ref().is_none_or(|p| p.job != e.id.0) {
        return false;
    }
    // A failed or cancelled download drops the command with it.
    let Some(pending) = app.jobs.subject_model_pending.take() else { return false };
    if !matches!(e.outcome, photocraft_engine::jobs::JobOutcome::Done(_)) {
        return false;
    }
    let result = if target(app) == Some(pending.target) { app.run_command(&pending.command, pending.params) } else { Err(changed()) };
    match result {
        Ok(_) => false,
        Err(error) => {
            crate::notices::error(app, error);
            true
        }
    }
}

#[cfg(test)]
mod tests;
