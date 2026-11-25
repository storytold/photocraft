//! Alerts and plain prompts on the dialog host: the one place that opens a notice or a
//! `StandardPrompt`, the "Don't show again" preference those prompts share, and the readers that tell
//! which family's prompt is open.
//!
//! The host keeps one `StandardPrompt` open at a time, so every family opens through
//! [`open_prompt`] (a second open is refused, not replaced) and tags its prompt with its own type.
//! [`prompt_tagged`] and [`prompt_tag`] find the open prompt by that tag.

use craft::ui::dialog::{Alert, DialogId, HostedAlert, PromptAnswer, PromptChoice, StackDialog, StandardPrompt};
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// The tag of an alert opened by [`alert_once`]: the dialog preference its "Don't show again" box sets.
pub(crate) struct AlertOnce {
    pref: &'static str,
}

/// The title every alert of the app carries.
const TITLE: &str = "PhotoCraft";

/// Shows `alert` on the host. One alert is open at a time, and the same message opened again while it
/// is up adds nothing: the open alert's id comes back.
pub fn show(app: &mut PhotocraftApp, alert: Alert<'static>) -> DialogId {
    let dialog = HostedAlert::new(alert);
    let message = |dialog: &HostedAlert| dialog.json_fields().and_then(|fields| fields.get("message").and_then(Value::as_str).map(str::to_owned));
    let host = &mut app.framework.dialogs;
    if let Some(open) = host.get::<HostedAlert>()
        && message(open) == message(&dialog)
    {
        return open.id();
    }
    host.open(dialog)
}

/// An alert titled "PhotoCraft" with `message` and OK; see [`show`] for the one-per-message rule.
pub fn alert(app: &mut PhotocraftApp, message: impl Into<String>) -> DialogId {
    show(app, Alert::new(TITLE, message.into()))
}

/// `message` with OK and "Don't show again", as the reference shows for warnings such as the
/// Content-Aware Move tool's "No pixels are selected.". `None` once the box was ticked for `pref`.
///
/// Escape answers OK (the prompt has no Cancel), so a ticked box is kept on OK and on Escape.
pub fn alert_once(app: &mut PhotocraftApp, message: impl Into<String>, pref: &'static str) -> Option<DialogId> {
    if dismissed(app, pref) {
        return None;
    }
    let prompt =
        StandardPrompt::custom(message.into()).title(TITLE).button(PromptChoice::Proceed, tl!("OK").to_owned()).dont_show_again().tag(AlertOnce { pref });
    open_prompt(app, prompt).ok().or_else(|| prompt_tagged::<AlertOnce>(app))
}

/// Whether "Don't show again" was ticked on the question remembered as `pref`.
pub fn dismissed(app: &PhotocraftApp, pref: &str) -> bool {
    app.session.prefs().dialogs.get(pref).and_then(|v| v.get("skip")).and_then(Value::as_bool) == Some(true)
}

/// Keeps "Don't show again" for the question remembered as `pref`, with the other dialog preferences.
pub fn remember_skip(app: &mut PhotocraftApp, pref: &str) {
    remember_skip_or(app, pref, tl!("The preference could not be saved: {error}"));
}

/// [`remember_skip`] with `failure` (a template with `{error}`) as the status text when the preference
/// could not be saved.
pub(crate) fn remember_skip_or(app: &mut PhotocraftApp, pref: &str, failure: &str) {
    app.session.edit_prefs(|prefs| {
        prefs.dialogs.insert(pref.into(), json!({"skip": true}));
    });
    if let Err(error) = crate::prefs_ui::save_preferences(app) {
        app.ui.status = crate::i18n::fmt(failure, &[("error", &error)]);
        app.ui.status_error = true;
    }
}

/// Reports a refused answer in the status bar: a prompt's answer has no reply to carry the error.
pub(crate) fn report_error(app: &mut PhotocraftApp, error: impl Into<String>) {
    app.ui.status = error.into();
    app.ui.status_error = true;
}

/// Reports the `Err` of a prompt's answer; the value of an `Ok` is dropped.
pub(crate) fn report(app: &mut PhotocraftApp, result: Result<Value, String>) {
    if let Err(error) = result {
        report_error(app, error);
    }
}

/// The `PROMPTS` entry of [`alert_once`]: stores a ticked box. `false` when the answer is another
/// family's.
pub(crate) fn answer(app: &mut PhotocraftApp, _ctx: &egui::Context, answer: &mut PromptAnswer) -> bool {
    let Some(AlertOnce { pref }) = answer.take_tag::<AlertOnce>() else { return false };
    if answer.skip {
        remember_skip(app, pref);
    }
    true
}

/// Opens `prompt` unless a `StandardPrompt` is already open. `Err` carries the open prompt's id.
pub(crate) fn open_prompt(app: &mut PhotocraftApp, prompt: StandardPrompt) -> Result<DialogId, DialogId> {
    let host = &mut app.framework.dialogs;
    match host.get::<StandardPrompt>().map(StackDialog::id) {
        Some(open) => Err(open),
        None => Ok(host.open(prompt)),
    }
}

/// Takes the answer of a closed alert: it carries nothing, and the host keeps one plain result per dialog type.
pub(crate) fn take(app: &mut PhotocraftApp, _ctx: &egui::Context) {
    let _ = app.framework.dialogs.take::<HostedAlert>();
}

/// The ids of the alerts open now: the plain alert and the prompt of [`alert_once`]. For tests.
#[cfg(test)]
pub(crate) fn alerts(app: &PhotocraftApp) -> Vec<DialogId> {
    let host = &app.framework.dialogs;
    host.get::<HostedAlert>().map(StackDialog::id).into_iter().chain(prompt_tagged::<AlertOnce>(app)).collect()
}

/// The open `StandardPrompt` when its tag is a `T`.
pub(crate) fn prompt_tagged<T: std::any::Any>(app: &PhotocraftApp) -> Option<DialogId> {
    app.framework.dialogs.get::<StandardPrompt>().filter(|prompt| prompt.tag_as::<T>().is_some()).map(StackDialog::id)
}

/// The tag of the open `StandardPrompt` when it is a `T`.
pub(crate) fn prompt_tag<T: std::any::Any>(app: &PhotocraftApp) -> Option<&T> {
    app.framework.dialogs.get::<StandardPrompt>()?.tag_as::<T>()
}
