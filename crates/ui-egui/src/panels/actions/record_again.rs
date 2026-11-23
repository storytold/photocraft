//! One editor owns one original step; previews and confirmation retain its authority.
use craft::ui::dialog::DialogId;
use photocraft_engine::actions_cmds::ActionsExt;
use photocraft_engine::{
    actions_edit_cmds,
    doc::{DocId, LayerId},
};
use serde_json::{Value, json};

use crate::PhotocraftApp;

const COMMAND: &str = "actions.recordAgain";

/// Selection, document and authority are private rather than editable dialog parameters.
pub(crate) struct Held {
    dialog: DialogId,
    selection: Value,
    command: String,
    document: Option<(DocId, u64, Option<LayerId>)>,
    gate: Option<super::Gate>,
    automated: bool,
}

/// Whether this dialog id still owns a deferred panel operation.
pub(crate) fn owns_dialog(app: &PhotocraftApp, dialog: DialogId) -> bool {
    app.held_record_again.as_ref().is_some_and(|held| held.dialog == dialog)
}

/// Apply captured and current permissions to the actual operation and final settings.
fn authorize(app: &PhotocraftApp, held: &Held, id: &str, params: &Value) -> Result<(), String> {
    super::authorize_owned(app, id, params, held.gate, held.automated)
}

/// Refuse stale selection or a different document before preview or confirmation can edit it.
fn unchanged(app: &PhotocraftApp, held: &Held) -> Result<(), String> {
    super::suspended_guard(app, COMMAND)?;
    if held.selection.get("revision").and_then(Value::as_u64) != Some(app.session.actions().list.revision())
        || app.session.recording.target.is_some()
        || app.session.recording.playing > 0
    {
        return Err(tl!("The actions changed. Request Record Again again.").into());
    }
    if app.session.active().map(|s| (s.doc.id, s.revision, s.active_layer)) != held.document {
        return Err(tl!("The document changed. Request Record Again again.").into());
    }
    Ok(())
}

/// Open the existing command editor and seed its controls from step `step` of `action`'s recorded values.
pub(super) fn open(app: &mut PhotocraftApp, action: usize, step: usize) -> Result<Value, String> {
    // The editor's open state is the signal: an editor closed without a result no longer holds anything.
    if app.held_record_again.as_ref().is_some_and(|held| app.framework.dialogs.stack().contains(held.dialog)) {
        return Err(tl!("Confirm or cancel the pending Record Again editor first.").into());
    }
    app.held_record_again = None;
    let selection = json!({"action": action, "step": step, "revision": app.session.actions().list.revision()});
    let target = actions_edit_cmds::record_again_target(&app.session, &selection).map_err(|e| e.to_string())?;
    let gate = app.session.authorize.or(if app.automation_input { app.services.automation_authorize } else { None });
    let mut held = Held {
        dialog: DialogId::NULL,
        selection,
        command: target.command,
        document: app.session.active().map(|s| (s.doc.id, s.revision, s.active_layer)),
        gate,
        automated: app.automation_input || gate.is_some(),
    };
    unchanged(app, &held)?;
    authorize(app, &held, COMMAND, &held.selection)?;
    authorize(app, &held, &held.command, &target.params)?;
    let resolved = photocraft_engine::actions_cmds::playback_params(&app.session, &held.command, &target.params).map_err(|e| e.to_string())?;
    app.session.precheck(&held.command, &resolved).map_err(|e| e.to_string())?;
    let dialog = super::open_recorded_editor(app, &held.command, &target.params)?;
    // The editor files its OK for `check` instead of closing, so a refusal reaches the caller and the editor stays.
    super::runner::hold_editor_ok(app, dialog);
    held.dialog = dialog;
    app.held_record_again = Some(held);
    Ok(json!({"dialog": dialog.value()}))
}

/// Guard both actual preview routes before parsing LUT files or evaluating a command.
pub(crate) fn authorize_preview(app: &PhotocraftApp, dialog: DialogId, command: &str, params: &Value) -> Result<(), String> {
    let Some(held) = app.held_record_again.as_ref().filter(|h| h.dialog == dialog) else { return Ok(()) };
    unchanged(app, held)?;
    if command != held.command {
        return Err(tl!("The Record Again command changed.").into());
    }
    authorize(app, held, command, params)
}

/// Judge the editor's answer: the step and document are unchanged, the command is the recorded one, and the original
/// caller's permissions allow the operation and its final settings. Returns the request that records the answer.
fn judge(app: &PhotocraftApp, held: &Held, command: &str, params: &Value) -> Result<Value, String> {
    unchanged(app, held)?;
    if command != held.command {
        return Err(tl!("The Record Again command changed.").into());
    }
    let mut request = held.selection.clone();
    request["params"] = params.clone();
    authorize(app, held, COMMAND, &request)?;
    authorize(app, held, &held.command, params)?;
    Ok(request)
}

/// The held OK of the editor `dialog`: whether [`confirm`] would take its answer now. A refusal keeps the editor and its
/// step.
pub(crate) fn check(app: &PhotocraftApp, dialog: DialogId, command: &str, params: &Value) -> Result<(), String> {
    let held = app.held_record_again.as_ref().filter(|held| held.dialog == dialog).ok_or(tl!("No recorded step belongs to this editor."))?;
    judge(app, held, command, params).map(drop)
}

/// Apply through the registered engine command, retaining flags and replacing only after success. `command` and
/// `params` are the editor's answer, from the one conversion of its values into command params.
pub(crate) fn confirm(app: &mut PhotocraftApp, dialog: DialogId, command: &str, params: Value) -> Result<Value, String> {
    if !owns_dialog(app, dialog) {
        return Err(tl!("No recorded step belongs to this editor.").into());
    }
    let held = app.held_record_again.take().ok_or(tl!("No step is waiting."))?;
    let request = judge(app, &held, command, &params)?;
    let saved = app.session.authorize;
    // Different policy functions must not replace the original caller's nested-command gate.
    if let (Some(old), Some(current)) = (held.gate, saved)
        && !std::ptr::fn_addr_eq(old, current)
    {
        return Err(tl!("The command permissions changed. Request Record Again again.").into());
    }
    app.session.authorize = held.gate.or(saved);
    let outcome = app.run(COMMAND, request);
    app.session.authorize = saved;
    app.filter_preview = None;
    outcome
}

/// A different dialog cannot discard this editor's selection or authority.
pub(crate) fn cancel(app: &mut PhotocraftApp, dialog: DialogId) {
    if owns_dialog(app, dialog) {
        app.held_record_again = None;
    }
}
