//! Confirm clearing the action library against the revision the user reviewed.
use serde_json::{Map, Value, json};

use crate::{PhotocraftApp, state::DialogKind};

const COMMAND: &str = "actions.clearAll";
const MARK: &str = "__clearActions";
/// The library revision the warning applies to; control clients cannot replace it.
pub(crate) const REVISION: &str = "__clearActionsRevision";

/// Whether this is the fixed Clear All confirmation, rather than an arbitrary command form.
pub(crate) fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// Open without changing definitions; the actual command is authorized before prompting.
pub(super) fn open(app: &mut PhotocraftApp) -> Result<Value, String> {
    let params = json!({"confirm": true});
    let gate = app.session.authorize.or(if app.automation_input { app.services.automation_authorize } else { None });
    if let Some(gate) = gate {
        gate(COMMAND, &params).map_err(|e| e.to_string())?;
    }
    if app.automation_input
        && let Some(authorize) = app.services.automation_command.as_ref()
    {
        authorize(COMMAND, &params)?;
    }
    let fields = json!({MARK: true, REVISION: app.session.actions.rev, "__command": COMMAND, "__label": "Clear All Actions"});
    let dialog = app.ui.open_dialog(DialogKind::Command, fields.as_object().cloned().unwrap_or_default());
    Ok(json!({"dialog": dialog}))
}

/// The warning is fixed; automation cannot change the meaning of the requested deletion.
pub(crate) fn body(ui: &mut egui::Ui) {
    ui.label(tl!("Clear all actions? This cannot be undone."));
}

/// Reject a changed library before dispatch, then reset the panel selection after success.
pub(crate) fn confirm(app: &mut PhotocraftApp, fields: &Map<String, Value>) -> Result<Value, String> {
    if fields.get("__command").and_then(Value::as_str) != Some(COMMAND) {
        return Err(tl!("Invalid Clear All Actions command").into());
    }
    if fields.get(REVISION).and_then(Value::as_u64) != Some(app.session.actions.rev) {
        return Err(tl!("The actions changed. Request Clear All Actions again.").into());
    }
    let result = app.run(COMMAND, json!({"confirm": true}))?;
    app.ui.actions.selected = None;
    app.ui.actions.selected_step = None;
    app.ui.actions.expanded.clear();
    app.ui.actions.renaming = None;
    app.ui.actions.reveal_recording = None;
    Ok(result)
}
