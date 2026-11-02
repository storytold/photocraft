//! Confirm clearing the action library against the revision the user reviewed.
use serde_json::{Map, Value, json};

use crate::{PhotocraftApp, state::DialogKind};

const COMMAND: &str = "actions.clearAll";
const MARK: &str = "__clearActions";
/// The library revision the warning applies to; control clients cannot replace it.
pub(crate) const REVISION: &str = "__clearActionsRevision";

/// Only the real warning owns the reviewed library and its caller's authority.
pub(crate) struct Held {
    dialog: u64,
    revision: u64,
    gate: Option<super::Gate>,
    automated: bool,
}

/// Whether this is the fixed Clear All confirmation, rather than an arbitrary command form.
pub(crate) fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// Open without changing definitions; the actual command is authorized before prompting.
pub(super) fn open(app: &mut PhotocraftApp) -> Result<Value, String> {
    super::suspended_guard(app, COMMAND)?;
    if app.held_actions_clear.is_some() {
        return Err(tl!("Confirm or cancel the pending Clear All warning first.").into());
    }
    if app.session.actions.recording.is_some() || app.session.actions.playing > 0 {
        return Err(tl!("Stop recording or playback before clearing actions.").into());
    }
    let params = json!({"confirm": true});
    let gate = app.session.authorize.or(if app.automation_input { app.services.automation_authorize } else { None });
    let automated = app.automation_input || gate.is_some();
    super::authorize_owned(app, COMMAND, &params, gate, automated)?;
    let fields = json!({MARK: true, REVISION: app.session.actions.rev, "__command": COMMAND, "__label": "Clear All Actions"});
    let dialog = app.ui.open_dialog(DialogKind::Command, fields.as_object().cloned().unwrap_or_default());
    app.held_actions_clear = Some(Held { dialog, revision: app.session.actions.rev, gate, automated });
    Ok(json!({"dialog": dialog}))
}

/// The warning is fixed; automation cannot change the meaning of the requested deletion.
pub(crate) fn body(ui: &mut egui::Ui) {
    ui.label(tl!("Clear all actions? This cannot be undone."));
}

/// Reject a changed library before dispatch, then reset the panel selection after success.
pub(crate) fn confirm(app: &mut PhotocraftApp, dialog: u64, fields: &Map<String, Value>) -> Result<Value, String> {
    if app.held_actions_clear.as_ref().is_none_or(|h| h.dialog != dialog) {
        return Err(tl!("No Clear All warning belongs to this dialog.").into());
    }
    let held = app.held_actions_clear.take().ok_or(tl!("No Clear All warning is waiting."))?;
    if fields.get("__command").and_then(Value::as_str) != Some(COMMAND) {
        return Err(tl!("Invalid Clear All Actions command").into());
    }
    if held.revision != app.session.actions.rev || fields.get(REVISION).and_then(Value::as_u64) != Some(held.revision) {
        return Err(tl!("The actions changed. Request Clear All Actions again.").into());
    }
    let params = json!({"confirm":true});
    super::authorize_owned(app, COMMAND, &params, held.gate, held.automated)?;
    let result = app.run(COMMAND, params)?;
    app.ui.actions.selected = None;
    app.ui.actions.selected_set = None;
    app.ui.actions.selected_step = None;
    app.ui.actions.expanded.clear();
    app.ui.actions.renaming = None;
    app.ui.actions.reveal_recording = None;
    Ok(result)
}

/// An unrelated Cancel cannot discard this warning's retained authority.
pub(crate) fn cancel(app: &mut PhotocraftApp, dialog: u64) {
    if app.held_actions_clear.as_ref().is_some_and(|h| h.dialog == dialog) {
        app.held_actions_clear = None;
    }
}
