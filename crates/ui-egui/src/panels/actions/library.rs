//! Clear All Actions, Reset Actions… and Replace Actions…: the question before each whole-list
//! command, and Edit › Undo of the last one.
//!
//! A panel menu event asks with the framework's [`LibraryPrompt`] on the dialog host, tagged with
//! what OK runs ([`Held`]); the answer reaches [`answer`] through the dialog host's prompt rows.
//! OK runs only while the list is at the revision it was asked at, and the command must pass both
//! the authorization it was asked under and the current one.

use craft::automation::ui::actions::{ActionsWords, LibraryPrompt};
use craft::host::{PickRequest, Picked};
use craft::ui::dialog::{PromptAnswer, PromptChoice};
use craft::ui::runtime::env::Env;
use craft::ui::runtime::words::CommonWords;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::dialogs::alerts;

const CLEAR_ALL: &str = "actions.clearAll";
const RESET: &str = "actions.reset";
const REPLACE: &str = "actions.replace";
const UNDO: &str = "actions.undoLibrary";

/// The largest Actions file Replace reads: its bytes travel base64-encoded in the command's
/// params, which stay well under the engine's import limit.
const MAX_PICKED_BYTES: u64 = photocraft_engine::preset_import_cmds::MAX_FILE_BYTES / 7 * 3;

/// When a question was asked and under which authorization.
#[derive(Clone, Copy)]
struct Asked {
    revision: u64,
    gate: Option<super::Gate>,
    automated: bool,
}

impl Asked {
    fn now(app: &PhotocraftApp) -> Self {
        let gate = app.session.authorize.or(if app.automation_input { app.services.automation_authorize } else { None });
        Asked { revision: app.session.actions.list.revision(), gate, automated: app.automation_input || gate.is_some() }
    }

    /// The asking authorization and the current one both allow `command` with `params`.
    fn authorize(&self, app: &PhotocraftApp, command: &str, params: &Value) -> Result<(), String> {
        super::authorize_owned(app, command, params, self.gate, self.automated)
    }
}

/// What OK of a library question runs: the prompt's tag.
struct Held {
    prompt: LibraryPrompt,
    asked: Asked,
    /// Replace: the chosen file's path (checked again on OK) and the command's params with the
    /// bytes read when the file was chosen, so OK replaces with exactly what the question counted.
    path: Option<String>,
    data: Option<Value>,
}

fn command(prompt: &LibraryPrompt) -> &'static str {
    match prompt {
        LibraryPrompt::ClearAll => CLEAR_ALL,
        LibraryPrompt::Reset => RESET,
        _ => REPLACE,
    }
}

/// Refuses a library change while the library is still loading, or a recording or playback holds
/// one of its actions (a Step by Step wait, a paused menu item editor or Check Spelling).
pub(crate) fn idle(app: &PhotocraftApp, command: &str) -> Result<(), String> {
    super::suspended_guard(app, command)?;
    if app.services.preset_store.is_some() {
        return Err(tl!("Wait for the Actions library to load.").into());
    }
    let paused = &app.paused_playbacks.paused;
    let editor = paused.values().any(|p| p.menu_editor.is_some());
    let spelling = app.ui.type_panels.spell.as_ref().and_then(|spell| spell.resume).is_some_and(|token| paused.contains_key(&token));
    if app.session.actions.recording.is_some() || app.session.actions.playing > 0 || super::playback::pending(app) || editor || spelling {
        return Err(tl!("Stop recording or playback before changing the Actions library.").into());
    }
    Ok(())
}

/// Clear All Actions: asks first.
pub(super) fn clear_all(app: &mut PhotocraftApp) -> Result<Value, String> {
    idle(app, CLEAR_ALL)?;
    let asked = Asked::now(app);
    asked.authorize(app, CLEAR_ALL, &json!({"confirm": true}))?;
    ask(app, Held { prompt: LibraryPrompt::ClearAll, asked, path: None, data: None })
}

/// Reset Actions…: asks whether the starting sets replace the list (OK) or join it (Append). The
/// question opens when either answer is allowed; the chosen one is checked again on the answer.
pub(super) fn reset(app: &mut PhotocraftApp) -> Result<Value, String> {
    idle(app, RESET)?;
    let asked = Asked::now(app);
    let allowed = |mode: &str| asked.authorize(app, RESET, &json!({"confirm": true, "mode": mode}));
    if let (Err(error), Err(_)) = (allowed("replace"), allowed("append")) {
        return Err(error);
    }
    ask(app, Held { prompt: LibraryPrompt::Reset, asked, path: None, data: None })
}

/// Replace Actions…: picks one `.atn` file, reads and counts it, then asks. The list stays as it
/// is until OK.
pub(super) fn replace(app: &mut PhotocraftApp) -> Result<Value, String> {
    idle(app, REPLACE)?;
    let asked = Asked::now(app);
    let mut request = PickRequest::open("actions").filter("Actions", &["atn"]);
    if let Some(dir) = crate::file_dialog::last_used_dir(&app.ui.recent_files) {
        request = request.directory(dir);
    }
    app.ask_file(request, move |app, picked| {
        idle(app, REPLACE)?;
        if app.session.actions.list.revision() != asked.revision {
            return Err(tl!("The actions changed. Request Replace Actions again.").into());
        }
        let (name, bytes, path) = match picked {
            Picked::Paths(paths) => {
                let path = paths.into_iter().next().ok_or(crate::file_dialog::CANCELLED)?.to_string_lossy().into_owned();
                asked.authorize(app, REPLACE, &json!({"confirm": true, "path": path}))?;
                let bytes = photocraft_format::read_file_limited(std::path::Path::new(&path), MAX_PICKED_BYTES).map_err(|error| format!("{path}: {error}"))?;
                (path.clone(), bytes, Some(path))
            }
            Picked::Contents(files) => {
                let (name, bytes) = files.into_iter().next().ok_or(crate::file_dialog::CANCELLED)?;
                (name, bytes, None)
            }
            Picked::Cancelled => return Err(crate::file_dialog::CANCELLED.into()),
            Picked::Unavailable(why) => return Err(why),
        };
        if bytes.len() as u64 > MAX_PICKED_BYTES {
            return Err(tl!("The Actions file is too large.").into());
        }
        let data = json!({"confirm": true, "data": photocraft_engine::paint::tile::b64_encode(&bytes)});
        asked.authorize(app, REPLACE, &data)?;
        let count = photocraft_engine::atn_cmds::validate_replacement(&bytes)?;
        let prompt = LibraryPrompt::replace(count, crate::file_open::display_name(&name));
        ask(app, Held { prompt, asked, path, data: Some(data) })
    })
}

/// Opens the question for `held`. Another open question is answered first.
fn ask(app: &mut PhotocraftApp, held: Held) -> Result<Value, String> {
    let env = Env::of_root(app.ctx());
    let prompt = held.prompt.prompt(&env.words::<ActionsWords>(), &env.words::<CommonWords>());
    match alerts::open_prompt(app, prompt.tag(held)) {
        Ok(id) => Ok(json!({"dialog": id.value()})),
        Err(_) => Err(tl!("Answer the open question first.").into()),
    }
}

/// The `PROMPTS` row of the library questions: OK (or Reset's Append) runs the held command and
/// reports a refusal on the status bar. `false` for another family's answer.
pub(crate) fn answer(app: &mut PhotocraftApp, _ctx: &egui::Context, answer: &mut PromptAnswer) -> bool {
    let Some(held) = answer.take_tag::<Held>() else { return false };
    let append = match answer.choice {
        PromptChoice::Proceed => false,
        PromptChoice::Alternate => true,
        _ => return true,
    };
    if let Err(error) = confirm(app, held, append) {
        alerts::report_error(app, error);
    }
    true
}

/// Runs the held command when the list is unchanged and both authorizations allow it.
fn confirm(app: &mut PhotocraftApp, held: Held, append: bool) -> Result<Value, String> {
    let command = command(&held.prompt);
    idle(app, command)?;
    if app.session.actions.list.revision() != held.asked.revision {
        return Err(tl!("The Actions library changed. Choose the command again.").into());
    }
    let params = match (&held.prompt, held.data) {
        (LibraryPrompt::Reset, _) => json!({"confirm": true, "mode": if append { "append" } else { "replace" }}),
        (_, Some(data)) => data,
        _ => json!({"confirm": true}),
    };
    if let Some(path) = &held.path {
        held.asked.authorize(app, command, &json!({"confirm": true, "path": path}))?;
    }
    held.asked.authorize(app, command, &params)?;
    let saved = app.session.authorize;
    // Different policy functions must not replace the asking caller's gate.
    if let (Some(asked), Some(current)) = (held.asked.gate, saved)
        && !std::ptr::fn_addr_eq(asked, current)
    {
        return Err(tl!("The command permissions changed. Choose the command again.").into());
    }
    app.session.authorize = held.asked.gate.or(saved);
    let result = app.run(command, params);
    app.session.authorize = saved;
    result
}

/// The Edit › Undo label while Undo reaches the last library change: no Pen point, transform or
/// crop box step and no newer document edit comes first, and nothing holds the library.
pub(crate) fn edit_undo_label(app: &PhotocraftApp) -> Option<&'static str> {
    if crate::vector_ui::pen_can_undo(app)
        || crate::transform_tool::is_enabled(app, "edit.undo").is_some()
        || crate::crop_ui::is_enabled(app, "edit.undo").is_some()
        || idle(app, UNDO).is_err()
    {
        return None;
    }
    photocraft_engine::actions_edit_cmds::undo_library_label(&app.session)
}

/// Edit › Undo of the last library change, through the command's own authorization.
pub(crate) fn undo(app: &mut PhotocraftApp) -> Result<Value, String> {
    app.run(UNDO, json!({}))
}
