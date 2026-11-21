//! A recorded save step with its dialog on (#15945). A person's playback shows the step's own
//! dialogs and waits for them: Save As and Save a Copy open the save dialog on the recorded file,
//! then the chosen format's options as the menu commands do; Export As opens on its recorded
//! settings. A file written continues the action after the step. Cancel in any of those dialogs
//! writes nothing and stops the action, as Cancel in a filter's step dialog does. Agents, scripts
//! and a step with its dialog off keep the silent replay (`PhotocraftApp::replay_save`).

use serde_json::{Value, json};

use photocraft_doc::DocId;
use photocraft_engine::actions_cmds::{self, Action};

use super::runner;
use crate::PhotocraftApp;

/// A paused playback's `dialog` while it waits at a save step's dialogs.
pub(crate) const SAVE_DIALOG: &str = "__saveDialog";

/// The playback waiting at a save step's dialogs.
#[derive(Debug)]
pub(crate) struct Waiting {
    /// Its continuation in [`runner::PausedPlaybacks`].
    pub(super) token: u64,
    /// The step's command and recorded params: the file chosen in its dialog is judged as this
    /// step writing there ([`admit`]).
    id: String,
    params: Value,
    /// The journal's length when the dialogs opened: each save written since journals its step.
    journal: usize,
    /// Set when Export As has written its files: several Scale All sizes journal no step.
    exported: bool,
    /// The document the step saves. The action goes on only while it is still the active one.
    doc: Option<DocId>,
    /// Whether background jobs were on. They are off while the dialogs are open, as for the
    /// step's silent replay, so a save is written by the time its step is journaled and a failed
    /// write journals nothing.
    background: bool,
    /// The status bar when the dialogs opened: a failure reported since stays there.
    status: String,
}

/// Whether step `id` shows dialogs of its own when its dialog toggle is on.
pub(super) fn asks(id: &str) -> bool {
    matches!(id, "file.saveAs" | "file.saveACopy" | crate::export_dialog::COMMAND)
}

/// Opens the dialogs of step `i` of `action` (command `id`, recorded `params`) and parks the
/// playback until they end ([`tick`]).
pub(super) fn pause(app: &mut PhotocraftApp, action: &Action, i: usize, id: &str, params: &Value) -> Result<(), String> {
    if app.paused_playbacks.save.is_some() {
        return Err("another action is waiting at a save dialog".into());
    }
    let token = app.paused_playbacks.pause(action.name.clone(), i + 1, app.session.authorize);
    app.paused_playbacks.save = Some(Waiting {
        token,
        id: id.to_owned(),
        params: params.clone(),
        journal: app.session.journal.len(),
        exported: false,
        doc: app.session.active().map(|d| d.doc.id),
        background: std::mem::replace(&mut app.background_jobs, false),
        status: app.ui.status.clone(),
    });
    let opened = match id {
        // Save As renames the document: like a filter's step dialog, it binds the action's history
        // before it writes (`runner::prepare_history`), so the rest of the action plays on.
        "file.saveAs" => app
            .save_as_dialog(params, move |app, path| {
                admit(app, path)?;
                runner::prepare_history(app, token)
            })
            .map(drop),
        // A copy and an export leave the document as it is.
        "file.saveACopy" => params
            .get("path")
            .and_then(Value::as_str)
            .filter(|p| !p.is_empty())
            .ok_or_else(|| "Save a Copy needs a `path`".to_string())
            .and_then(|suggested| crate::view_cmds::save_a_copy_from(app, suggested, admit))
            .map(drop),
        _ => crate::export_dialog::open_recorded(app, params).map(drop),
    };
    if let Err(error) = opened {
        if let Some(waiting) = app.paused_playbacks.save.take() {
            app.background_jobs = waiting.background;
        }
        app.paused_playbacks.paused.remove(&token);
        return Err(error);
    }
    Ok(())
}

/// The file chosen in a waiting save step's dialog, judged as the step writing there: by the gate
/// the playback paused under and the current one, as an edited step dialog's values are. `Ok`
/// when no save step is waiting (a save by hand).
pub(crate) fn admit(app: &mut PhotocraftApp, path: &str) -> Result<(), String> {
    let Some(waiting) = app.paused_playbacks.save.as_ref() else { return Ok(()) };
    let gate = app.paused_playbacks.paused.get(&waiting.token).and_then(|p| p.gate);
    let mut params = waiting.params.clone();
    if let Some(object) = params.as_object_mut() {
        object.remove("folder");
        object.remove("extension");
        object.insert("path".into(), json!(path));
    }
    runner::authorize_owned(app, &waiting.id, &params, gate, false)
}

/// Export As wrote its files: a waiting Export As step continues.
pub(crate) fn exported(app: &mut PhotocraftApp) {
    if let Some(waiting) = app.paused_playbacks.save.as_mut() {
        waiting.exported = true;
    }
}

/// Whether the dialogs a save step opened are still up: its save dialog, a format's options, the
/// Export As dialog, or the write they started.
fn open(app: &PhotocraftApp) -> bool {
    app.file_dialog_open() || app.save_options_open() || app.export_open.is_some() || app.saving()
}

/// Every frame: once the dialogs of the waiting save step are gone, the action continues after the
/// step when they wrote a file on the document it was playing on, and stops there otherwise.
pub(crate) fn tick(app: &mut PhotocraftApp) {
    if app.paused_playbacks.save.is_none() || open(app) {
        return;
    }
    let Some(waiting) = app.paused_playbacks.save.take() else { return };
    app.background_jobs = waiting.background;
    let saved =
        waiting.exported || app.session.journal.get(waiting.journal..).is_some_and(|steps| steps.iter().any(|(id, _)| actions_cmds::shell_save_command(id)));
    if !saved {
        // A failed write keeps its own error in the status bar; Cancel says the action stopped.
        if app.ui.status_error && app.ui.status != waiting.status {
            runner::abandoned(app, waiting.token);
        } else {
            runner::stopped(app, waiting.token);
        }
        return;
    }
    if waiting.doc.is_some() && waiting.doc != app.session.active().map(|d| d.doc.id) {
        let step = app.paused_playbacks.paused.get(&waiting.token).map_or(0, |p| p.from.saturating_sub(1));
        runner::abandoned(app, waiting.token);
        let error = "the active document changed while the save step's dialog was open";
        runner::report_play(app, &json!({"failed": {"step": step, "id": waiting.id, "error": error}}));
        return;
    }
    // `resume` reports a failure of the rest in the status bar.
    let _ = runner::resume(app, waiting.token);
}
