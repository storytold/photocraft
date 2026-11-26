//! A recorded save step with its dialog on (#15945). A person's playback shows the step's own
//! dialogs and waits for them: Save As and Save a Copy open the save dialog on the recorded file,
//! then the chosen format's options on the recorded answers, as the menu commands do; Export As
//! opens on its recorded settings. A file written continues the action after the step. Cancel in
//! any of those dialogs writes nothing and stops the action, as Cancel in a filter's step dialog
//! does. Agents, scripts and a step with its dialog off keep the silent replay
//! (`PhotocraftApp::replay_save`).

use std::path::Path;

use serde_json::{Value, json};

use photocraft_doc::DocId;
use photocraft_engine::actions_cmds::Action;

use super::runner;
use crate::{FormatChoices, PhotocraftApp};

/// A paused playback's `dialog` while it waits at a save step's dialogs.
pub(crate) const SAVE_DIALOG: &str = "__saveDialog";

/// Why a waiting save step writes nothing once another document is active.
fn document_changed() -> String {
    tl!("the active document changed while the save step's dialog was open").to_string()
}

/// The playback waiting at a save step's dialogs.
#[derive(Debug)]
pub(crate) struct Waiting {
    /// Its continuation in [`runner::PausedPlaybacks`].
    pub(super) token: u64,
    /// The step's command and recorded params: the file chosen in its dialog is judged as this
    /// step writing there ([`admit`]).
    id: String,
    params: Value,
    /// The journal's length when the dialogs last let a write through ([`admit_step`]). Only a step
    /// journaled from there on can be the dialog's own.
    admitted: Option<usize>,
    /// The file the step's save dialog returned. Only a journaled step that wrote it is the step's
    /// save, and a format's options writing anything else are another save's, which this step
    /// neither judges nor counts ([`admit_answered`]).
    chosen: Option<String>,
    /// Set when Export As has written its files: several Scale All sizes journal no step.
    exported: bool,
    /// Set when the step's own write was refused (another document active, or a gate): its reason
    /// stays in the status bar when the action ends.
    refused: bool,
    /// The document the step saves. Nothing is written once another one is active ([`admit_step`]),
    /// and the action goes on only while it is still the active one.
    doc: Option<DocId>,
    /// Whether background jobs were on before the step's write. They are switched off when a write
    /// is let through, as for the step's silent replay, so the save is written by the time its step
    /// is journaled and a failed write journals nothing; while the dialogs only wait for the person,
    /// they stay as they were.
    background: Option<bool>,
}

/// Whether step `id` shows dialogs of its own when its dialog toggle is on.
pub(super) fn asks(id: &str) -> bool {
    matches!(id, "file.saveAs" | "file.saveACopy" | crate::export_dialog::COMMAND)
}

/// Opens the dialogs of step `i` of `action` (command `id`, recorded `params`) and parks the
/// playback until they end ([`tick`]).
pub(super) fn pause(app: &mut PhotocraftApp, action: &Action, i: usize, id: &str, params: &Value) -> Result<(), String> {
    if app.paused_playbacks.save.is_some() {
        return Err(tl!("another action is waiting at a save dialog").into());
    }
    let token = app.paused_playbacks.pause(action.name.clone(), i + 1, app.session.authorize);
    app.paused_playbacks.save = Some(Waiting {
        token,
        id: id.to_owned(),
        params: params.clone(),
        admitted: None,
        chosen: None,
        exported: false,
        refused: false,
        doc: app.session.active().map(|d| d.doc.id),
        background: None,
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
        "file.saveACopy" => match params.get("path").and_then(Value::as_str).filter(|p| !p.is_empty()) {
            Some(recorded) => {
                // A recorded answer the replay would refuse just isn't a starting value here.
                let seed = FormatChoices::from_step(params, id).unwrap_or_default();
                crate::view_cmds::save_a_copy_from(app, &crate::file_dialog::in_existing_folder(recorded), seed, admit).map(drop)
            }
            None => Err(tl!("Save a Copy needs a `path`").into()),
        },
        _ => crate::export_dialog::open_recorded(app, params).map(drop),
    };
    if let Err(error) = opened {
        app.paused_playbacks.save = None;
        app.paused_playbacks.paused.remove(&token);
        return Err(error);
    }
    Ok(())
}

/// The file a waiting Save As or Save a Copy step's own save dialog returned: kept as the step's
/// file, then [`admit_step`] for the recorded step writing at `path`. Only the step's own picker
/// calls this.
pub(crate) fn admit(app: &mut PhotocraftApp, path: &str) -> Result<(), String> {
    let Some(waiting) = app.paused_playbacks.save.as_mut() else { return Ok(()) };
    waiting.chosen = Some(path.to_owned());
    admit_step(app, recorded_at(&waiting_params(app), path))
}

/// A format's options about to write `path` (`write_answered_save`): [`admit`]'s check again
/// when it is the waiting step's own file. Any other save's options (another request made while
/// the step's save dialog was open) are neither judged nor counted as the step's.
pub(crate) fn admit_answered(app: &mut PhotocraftApp, path: &str) -> Result<(), String> {
    let Some(waiting) = app.paused_playbacks.save.as_ref() else { return Ok(()) };
    if waiting.chosen.as_deref() != Some(path) {
        return Ok(());
    }
    admit_step(app, recorded_at(&waiting_params(app), path))
}

/// Export As about to write `params` (its settings and file): [`admit_step`] when the waiting step
/// is an Export As, whose dialog this is. `Ok` otherwise.
pub(crate) fn admit_export(app: &mut PhotocraftApp, params: Value) -> Result<(), String> {
    match app.paused_playbacks.save.as_ref() {
        Some(waiting) if waiting.id == crate::export_dialog::COMMAND => admit_step(app, params),
        _ => Ok(()),
    }
}

/// The waiting step's recorded params (`Null` when none is waiting).
fn waiting_params(app: &PhotocraftApp) -> Value {
    app.paused_playbacks.save.as_ref().map_or(Value::Null, |w| w.params.clone())
}

/// The recorded step `params` writing at `path`.
fn recorded_at(params: &Value, path: &str) -> Value {
    let mut params = params.clone();
    if let Some(object) = params.as_object_mut() {
        object.remove("folder");
        object.remove("extension");
        object.insert("path".into(), json!(path));
    }
    params
}

/// The write a waiting save step's dialog is about to make, as the step with `params`: refused
/// once another document is active, then judged by the gate the playback paused under and the
/// current one, as an edited step dialog's values are. Called right before each write the step's
/// own dialogs make ([`admit`], [`admit_answered`], [`admit_export`]). A write it lets through runs
/// with background jobs off, and an error reported from then on is that write's.
fn admit_step(app: &mut PhotocraftApp, params: Value) -> Result<(), String> {
    let Some(waiting) = app.paused_playbacks.save.as_ref() else { return Ok(()) };
    let (id, token, doc) = (waiting.id.clone(), waiting.token, waiting.doc);
    let verdict = if doc.is_some() && doc != app.session.active().map(|d| d.doc.id) {
        Err(document_changed())
    } else {
        let gate = app.paused_playbacks.paused.get(&token).and_then(|p| p.gate);
        runner::authorize_owned(app, &id, &params, gate, false)
    };
    let (journal, background) = (app.session.journal.len(), app.background_jobs);
    let Some(waiting) = app.paused_playbacks.save.as_mut() else { return verdict };
    if verdict.is_err() {
        waiting.refused = true;
        return verdict;
    }
    waiting.admitted = Some(journal);
    waiting.background.get_or_insert(background);
    app.background_jobs = false;
    app.ui.status_error = false;
    verdict
}

/// Export As wrote its files: a waiting Export As step continues. Another export, finished while a
/// Save As or Save a Copy step waits, is not that step's save.
pub(crate) fn exported(app: &mut PhotocraftApp) {
    if let Some(waiting) = app.paused_playbacks.save.as_mut().filter(|w| w.id == crate::export_dialog::COMMAND) {
        waiting.exported = true;
    }
}

/// Whether journaled step `params` wrote `file`: its `path`, or for a Save As that kept the
/// document's own name, its `folder` and `extension` (`PhotocraftApp::save_as_record`).
fn wrote(params: &Value, file: &str) -> bool {
    if let Some(path) = params.get("path").and_then(Value::as_str) {
        return path == file;
    }
    let file = Path::new(file);
    let folder = params.get("folder").and_then(Value::as_str);
    let extension = params.get("extension").and_then(Value::as_str);
    folder.is_some_and(|folder| file.parent() == Some(Path::new(folder))) && extension.is_some_and(|e| file.extension().is_some_and(|x| x == e))
}

/// Whether the dialogs a save step opened are still up: its save dialog, a format's options, the
/// Export As dialog, or the write they started.
fn open(app: &PhotocraftApp) -> bool {
    app.file_dialog_open() || app.save_options_open() || app.export_open.is_some() || app.saving()
}

/// Every frame: once the dialogs of the waiting save step are gone, the action continues after the
/// step when they wrote its file on the document it was playing on, and stops there otherwise.
pub(crate) fn tick(app: &mut PhotocraftApp) {
    if app.paused_playbacks.save.is_none() || open(app) {
        return;
    }
    let Some(waiting) = app.paused_playbacks.save.take() else { return };
    if let Some(background) = waiting.background {
        app.background_jobs = background;
    }
    let saved = if waiting.id == crate::export_dialog::COMMAND {
        waiting.exported
    } else {
        let own = |(id, params): &(String, Value)| *id == waiting.id && waiting.chosen.as_deref().is_some_and(|file| wrote(params, file));
        waiting.admitted.and_then(|from| app.session.journal.get(from..)).is_some_and(|steps| steps.iter().any(own))
    };
    if !saved {
        // The step's own refused or failed write keeps its reason in the status bar; Cancel, and
        // anything else that happened meanwhile, says the action stopped.
        if waiting.refused || (waiting.admitted.is_some() && app.ui.status_error) {
            runner::abandoned(app, waiting.token);
        } else {
            runner::stopped(app, waiting.token);
        }
        return;
    }
    if waiting.doc.is_some() && waiting.doc != app.session.active().map(|d| d.doc.id) {
        let step = app.paused_playbacks.paused.get(&waiting.token).map_or(0, |p| p.from.saturating_sub(1));
        runner::abandoned(app, waiting.token);
        runner::report_play(app, &json!({"failed": {"step": step, "id": waiting.id, "error": document_changed()}}));
        return;
    }
    // `resume` reports a failure of the rest in the status bar.
    let _ = runner::resume(app, waiting.token);
}
