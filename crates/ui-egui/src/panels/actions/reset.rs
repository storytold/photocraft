//! Reset offers explicit append/replace consent while default contents remain a placeholder.
use crate::{PhotocraftApp, state::DialogKind};
use serde_json::{Map, Value, json};
const COMMAND: &str = "actions.reset";
const MARK: &str = "__resetActions";

/// The actual warning and reviewed library are private, not editable dialog fields.
pub(crate) struct Held {
    dialog: u64,
    revision: u64,
    gate: Option<super::Gate>,
    automated: bool,
}

/// Identify the fixed reset workflow through the protected route.
pub(crate) fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// Reset must not discard a live or suspended recording.
fn idle(app: &PhotocraftApp) -> Result<(), String> {
    super::suspended_guard(app, COMMAND)?;
    if app.session.actions.recording.is_some() || app.session.actions.playing > 0 {
        return Err(tl!("Stop recording or playback before resetting actions.").into());
    }
    Ok(())
}

/// Capture this library and caller, keeping append as the initial choice.
pub(super) fn open(app: &mut PhotocraftApp) -> Result<Value, String> {
    idle(app)?;
    if app.held_actions_reset.is_some() {
        return Err(tl!("Confirm or cancel the pending Actions reset first.").into());
    }
    let gate = app.session.authorize.or(if app.automation_input { app.services.automation_authorize } else { None });
    let automated = app.automation_input || gate.is_some();
    super::authorize_owned(app, COMMAND, &json!({"mode":"append","confirm":true}), gate, automated)?;
    let fields = json!({MARK:true,"__command":COMMAND,"__label":"Reset Actions","mode":"append"});
    let dialog = app.ui.open_dialog(DialogKind::Command, fields.as_object().cloned().unwrap_or_default());
    app.held_actions_reset = Some(Held { dialog, revision: app.session.actions.rev, gate, automated });
    Ok(json!({"dialog":dialog}))
}

/// Clearly label the temporary content and offer append, replace or the standard Cancel.
pub(crate) fn body(ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    ui.label(tl!("Default Actions contents are not available yet. This restores an empty placeholder set."));
    let mut mode = fields.get("mode").and_then(Value::as_str).unwrap_or("append").to_string();
    ui.radio_value(&mut mode, "append".into(), tl!("Append"));
    ui.radio_value(&mut mode, "replace".into(), tl!("Replace"));
    if mode == "replace" {
        ui.label(tl!("This removes the current action library and cannot be undone."));
    }
    fields.insert("mode".into(), json!(mode));
}

/// Check actual consent and retained permissions before the registered command commits.
pub(crate) fn confirm(app: &mut PhotocraftApp, dialog: u64, fields: &Map<String, Value>) -> Result<Value, String> {
    if app.held_actions_reset.as_ref().is_none_or(|h| h.dialog != dialog) {
        return Err(tl!("No Actions reset belongs to this dialog.").into());
    }
    let held = app.held_actions_reset.take().ok_or(tl!("No Actions reset is waiting."))?;
    idle(app)?;
    if held.revision != app.session.actions.rev || fields.get("__command").and_then(Value::as_str) != Some(COMMAND) {
        return Err(tl!("The actions or reset command changed. Request Reset Actions again.").into());
    }
    let params = json!({"mode":fields.get("mode"),"confirm":true});
    super::authorize_owned(app, COMMAND, &params, held.gate, held.automated)?;
    let saved = app.session.authorize;
    if let (Some(old), Some(current)) = (held.gate, saved)
        && !std::ptr::fn_addr_eq(old, current)
    {
        return Err(tl!("The command permissions changed. Request Reset Actions again.").into());
    }
    app.session.authorize = held.gate.or(saved);
    let result = app.run(COMMAND, params);
    app.session.authorize = saved;
    let result = result?;
    app.ui.actions.selected = None;
    app.ui.actions.selected_step = None;
    app.ui.actions.selected_set = result.get("set").and_then(Value::as_str).map(str::to_string);
    app.ui.actions.expanded.resize(app.session.actions.list.len(), false);
    app.ui.actions.renaming = None;
    app.ui.actions.reveal_recording = None;
    if let Some(set) = &app.ui.actions.selected_set {
        app.ui.actions.collapsed_sets.retain(|s| s != set);
    }
    Ok(result)
}

/// Another dialog cannot discard this warning's consent.
pub(crate) fn cancel(app: &mut PhotocraftApp, dialog: u64) {
    if app.held_actions_reset.as_ref().is_some_and(|h| h.dialog == dialog) {
        app.held_actions_reset = None;
    }
}
