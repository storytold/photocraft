//! A picked Actions file belongs to one confirmation and never gets reread after consent.
use serde_json::{Map, Value, json};

use crate::{PhotocraftApp, state::DialogKind};

const COMMAND: &str = "actions.replace";
const MARK: &str = "__replaceActions";

// Raw input and its base64 transport overlap during staging and decoding. Three raw bytes
// need four encoded bytes; allow at most the shared 512 MiB input cap for both together.
// This limits the picker, not Load Actions or the engine's separate per-file import cap.
const MAX_PICKED_BYTES: u64 = photocraft_engine::preset_import_cmds::MAX_FILE_BYTES / 7 * 3;

/// Frozen input and authority, kept out of editable and inspectable dialog fields.
pub(crate) struct Held {
    dialog: u64,
    revision: u64,
    params: Value,
    gate: Option<super::Gate>,
    automated: bool,
    file: String,
    count: usize,
}

/// Identify the replacement handler through the existing protected route mechanism.
pub(crate) fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// Apply both the request's authority and the current confirmation's authority.
fn authorize(app: &PhotocraftApp, params: &Value, gate: Option<super::Gate>, automated: bool) -> Result<(), String> {
    super::authorize_owned(app, COMMAND, params, gate, automated)
}

/// A file picker or warning must not discard an active or suspended recording.
fn idle(app: &PhotocraftApp) -> Result<(), String> {
    super::suspended_guard(app, COMMAND)?;
    if app.session.actions.recording.is_some() || app.session.actions.playing > 0 {
        return Err(tl!("Stop recording or playback before replacing actions.").into());
    }
    Ok(())
}

/// Pick a single file without changing Load Actions' separate bulk-import behavior.
pub(super) fn open(app: &mut PhotocraftApp) -> Result<Value, String> {
    idle(app)?;
    if app.held_actions_replace.is_some() {
        return Err(tl!("Confirm or cancel the pending Actions replacement first.").into());
    }
    let revision = app.session.actions.rev;
    let gate = app.session.authorize.or(if app.automation_input { app.services.automation_authorize } else { None });
    let automated = app.automation_input || gate.is_some();
    authorize(app, &json!({"confirm": true}), gate, automated)?;
    app.pick_file_bytes_with_limit(&["atn"], Some(MAX_PICKED_BYTES), move |app, name, bytes| {
        idle(app)?;
        if app.session.actions.rev != revision || app.held_actions_replace.is_some() {
            return Err(tl!("The actions changed. Request Replace Actions again.").into());
        }
        // Validate before asking to discard the library. The normal command parses this
        // same immutable input again on confirmation, keeping dispatch authoritative.
        let count = photocraft_engine::atn_cmds::import(&bytes)?.actions.len();
        let mut params = json!({"confirm": true});
        // Move the encoded allocation into the transport rather than serializing another String copy.
        params["data"] = Value::String(photocraft_engine::paint::tile::b64_encode(&bytes));
        drop(bytes);
        authorize(app, &params, gate, automated)?;
        let fields = json!({MARK: true, "__command": COMMAND, "__label": "Replace Actions"});
        let dialog = app.ui.open_dialog(DialogKind::Command, fields.as_object().cloned().unwrap_or_default());
        app.held_actions_replace = Some(Held { dialog, revision, params, gate, automated, file: crate::file_open::display_name(&name), count });
        Ok(json!({"dialog": dialog, "confirmationPending": true}))
    })
}

/// Show the actual held file and action count, never metadata supplied by dialog edits.
pub(crate) fn body(app: &PhotocraftApp, ui: &mut egui::Ui, dialog: u64) {
    if let Some(held) = app.held_actions_replace.as_ref().filter(|held| held.dialog == dialog) {
        let text = crate::i18n::fmt(tl!("Replace all actions with {count} actions from {file}?"), &[("count", &held.count.to_string()), ("file", &held.file)]);
        ui.label(text);
        ui.label(tl!("This removes the current action library and cannot be undone."));
    }
}

/// Only this warning may consume its input; every refusal releases that owned input.
pub(crate) fn confirm(app: &mut PhotocraftApp, dialog: u64, fields: &Map<String, Value>) -> Result<Value, String> {
    if app.held_actions_replace.as_ref().is_none_or(|held| held.dialog != dialog) {
        return Err(tl!("No Actions replacement belongs to this dialog.").into());
    }
    let held = app.held_actions_replace.take().ok_or(tl!("No Actions replacement is waiting."))?;
    if fields.get("__command").and_then(Value::as_str) != Some(COMMAND) || app.session.actions.rev != held.revision {
        return Err(tl!("The actions or replacement command changed. Request Replace Actions again.").into());
    }
    idle(app)?;
    authorize(app, &held.params, held.gate, held.automated)?;
    let replaced_sets = photocraft_engine::actions_edit_cmds::set_names(&app.session);
    let saved = app.session.authorize;
    app.session.authorize = saved.or(held.gate);
    let result = app.run(COMMAND, held.params);
    app.session.authorize = saved;
    let result = result?;
    app.ui.actions.collapsed_sets.retain(|set| !replaced_sets.contains(set));
    app.ui.actions.selected = (!app.session.actions.list.is_empty()).then_some(0);
    app.ui.actions.selected_set = if app.session.actions.list.is_empty() {
        result.get("set").and_then(Value::as_str).filter(|set| app.session.actions.sets.iter().any(|s| s == set)).map(str::to_string)
    } else {
        None
    };
    app.ui.actions.selected_step = None;
    app.ui.actions.expanded = vec![false; app.session.actions.list.len()];
    if let Some(first) = app.ui.actions.expanded.first_mut() {
        *first = true;
    }
    app.ui.actions.renaming = None;
    app.ui.actions.reveal_recording = None;
    app.ui.panels.history = true;
    Ok(result)
}

/// Cancelling another dialog cannot discard this warning's staged replacement.
pub(crate) fn cancel(app: &mut PhotocraftApp, dialog: u64) {
    if app.held_actions_replace.as_ref().is_some_and(|held| held.dialog == dialog) {
        app.held_actions_replace = None;
    }
}
