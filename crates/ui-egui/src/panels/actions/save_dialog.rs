//! A recorded save step with its dialog on (#15945). A person's playback shows the step's own
//! dialogs and waits for them: Save As and Save a Copy open the save dialog on the recorded file,
//! then the chosen format's options as the menu commands do; Export As opens on its recorded
//! settings. A file written continues the action after the step. Cancel in any of those dialogs
//! writes nothing and stops the action, as Cancel in a filter's step dialog does. Agents, scripts
//! and a step with its dialog off keep the silent replay (`PhotocraftApp::replay_save`).

use serde_json::Value;

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
    /// The journal's length when the dialogs opened: each save written since journals its step.
    journal: usize,
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
    let journal = app.session.journal.len();
    let token = app.paused_playbacks.pause(action.name.clone(), i + 1, app.session.authorize);
    let opened = match id {
        // Save As renames the document: like a filter's step dialog, it binds the action's history
        // before it writes (`runner::prepare_history`), so the rest of the action plays on.
        "file.saveAs" => app.save_as_dialog(params, move |app| runner::prepare_history(app, token)).map(drop),
        // A copy and an export leave the document as it is.
        "file.saveACopy" => params
            .get("path")
            .and_then(Value::as_str)
            .filter(|p| !p.is_empty())
            .ok_or_else(|| "Save a Copy needs a `path`".to_string())
            .and_then(|suggested| crate::view_cmds::save_a_copy_from(app, suggested))
            .map(drop),
        _ => crate::export_dialog::open_recorded(app, params).map(drop),
    };
    if let Err(error) = opened {
        app.paused_playbacks.paused.remove(&token);
        return Err(error);
    }
    app.paused_playbacks.save = Some(Waiting { token, journal });
    Ok(())
}

/// Whether the dialogs a save step opened are still up: its save dialog, a format's options, the
/// Export As dialog, or the write they started.
fn open(app: &PhotocraftApp) -> bool {
    app.file_dialog_open() || app.save_options_open() || app.export_open.is_some() || app.saving()
}

/// Every frame: once the dialogs of the waiting save step are gone, the action continues after the
/// step when they wrote a file, and stops there when they were cancelled.
pub(crate) fn tick(app: &mut PhotocraftApp) {
    if app.paused_playbacks.save.is_none() || open(app) {
        return;
    }
    let Some(Waiting { token, journal }) = app.paused_playbacks.save.take() else { return };
    let saved = app.session.journal.get(journal..).is_some_and(|steps| steps.iter().any(|(id, _)| actions_cmds::shell_save_command(id)));
    if saved {
        // `resume` reports a failure of the rest in the status bar.
        let _ = runner::resume(app, token);
    } else {
        runner::stopped(app, token);
    }
}
