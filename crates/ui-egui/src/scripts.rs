//! JavaScript scripts (`.js`, `.jsx`): a run through the platform's script service, its dialogs,
//! ScriptUI windows and file pickers, and File › Scripts › Scripts Manager.
//!
//! The platform starts runs ([`Services::scripts`](crate::Services)): on the desktop each run has
//! its own script host process, in the browser a dedicated Worker (on a page that isn't
//! cross-origin isolated the browser service refuses to start one). Either way the run is pumped
//! once a frame ([`tick`]); the session commands it asks for run there on the app's session, and
//! what it needs from the user (an alert, a ScriptUI dialog, a file picker) is shown here
//! ([`show`]) and answered back. One run at a time.

use std::collections::BTreeMap;
use std::time::Duration;

use egui::{Id, RichText};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use craft::host::{PickRequest, Picked};
use craft::ui::contract::Show as _;

use crate::file_dialog::last_used_dir;

/// What the platform needs to start a run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptStart {
    /// Shown to the user and in the script's error messages.
    pub name: String,
    pub source: String,
    /// `app.name` for this run (the first tag of a listed script).
    pub app_name: Option<String>,
}

/// A dialog a run waits at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptDialog {
    Alert,
    Confirm,
    Prompt,
    /// A ScriptUI dialog: `{"id", "window"}`; answered with the user's next event.
    Window,
    /// `{"prompt", "filter", "multiple"}`: a path, a list of paths, or `null`.
    OpenFile,
    /// `{"prompt", "filter"}`: a path or `null`.
    SaveFile,
    /// `{"prompt"}`: a path or `null`.
    SelectFolder,
}

impl ScriptDialog {
    fn picker(self) -> bool {
        matches!(self, Self::OpenFile | Self::SaveFile | Self::SelectFolder)
    }
}

/// What a run needs from the interface.
#[derive(Clone, Debug, PartialEq)]
pub enum ScriptEvent {
    /// A line the script wrote.
    Output(String),
    /// Show this dialog and [`ScriptRun::answer`] it.
    Dialog { kind: ScriptDialog, args: Value },
    /// Show or update (`Some`, its controls) or hide (`None`) a window that does not wait.
    Window { id: u32, window: Option<Value> },
    /// The run ended; `Err` holds the message for the user.
    Finished(Result<(), String>),
}

/// A run in progress, as the platform provides it.
pub trait ScriptRun {
    /// Advance the run for about `budget` without waiting on the user: run the session commands
    /// it asks for through `commands`, and return what the interface must show. Stops early at
    /// a dialog or the end of the run.
    fn pump(&mut self, commands: &mut dyn ScriptCommands, budget: Duration) -> Vec<ScriptEvent>;
    /// Answer the dialog the run waits at.
    ///
    /// # Errors
    /// The run is gone or was not waiting.
    fn answer(&mut self, answer: Value) -> Result<(), String>;
    /// Stop the run; it reports [`ScriptEvent::Finished`] on a later pump.
    fn cancel(&mut self);
}

/// Why a run can't start without a script service.
pub(crate) const NO_SCRIPTS: &str = "this build of PhotoCraft can't run JavaScript scripts";

/// Where a run's commands go: the app's own command path ([`PhotocraftApp::run`]), with its
/// authorization and handling, never a bare session.
pub trait ScriptCommands {
    /// Run session command `id`. A command naming its document ([`ON_DOCUMENT`]) runs on that
    /// document, made active in the same call.
    ///
    /// # Errors
    /// The command's error, for the script.
    fn execute(&mut self, id: &str, params: Value) -> Result<Value, String>;
}

/// Tests only: a command that replaces the params of the journal's last entry, as typing or a
/// brush gesture folds into it.
#[cfg(test)]
pub(crate) const TEST_FOLD: &str = "test.foldIntoLastEntry";

/// The parameter a script command names its document by: the document's serial. The same
/// name as `photocraft_script_js::commands::ON_DOCUMENT` (the desktop app checks they match).
pub const ON_DOCUMENT: &str = "onDocument";

/// What one run may do through the app. The script's own time is bounded by its host; this
/// bounds the work it asks the app for, whose memory and time are the app's.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct RunBudget {
    commands: u64,
    layers: u32,
    documents: u32,
    app_time: Duration,
}

/// Commands one run may run.
const MAX_RUN_COMMANDS: u64 = 1 << 20;
/// Layers one run may make.
const MAX_RUN_LAYERS: u32 = 10_000;
/// Documents one run may make.
const MAX_RUN_DOCUMENTS: u32 = 64;
/// Time one run may take of the app for its commands.
const MAX_RUN_APP_TIME: Duration = Duration::from_secs(30 * 60);

impl RunBudget {
    /// Whether the run may still have the app run a command at all: commands and app time left.
    fn open(&self) -> Result<(), String> {
        if self.commands >= MAX_RUN_COMMANDS {
            return Err(format!("a script can run at most {MAX_RUN_COMMANDS} commands"));
        }
        if self.app_time >= MAX_RUN_APP_TIME {
            return Err("the script has used all the time PhotoCraft gives a script's commands".into());
        }
        Ok(())
    }

    /// Charge command `id`, which makes `copies` layers when it duplicates them (a group with
    /// everything in it, or a whole document), or say why the run may not run it.
    fn charge(&mut self, id: &str, params: &Value, copies: u32) -> Result<(), String> {
        self.open()?;
        let layers = match id {
            "layer.new.layer" | "layer.new.group" | "type.create" => 1,
            "layer.duplicate" => copies.max(1),
            // A move with `copy` duplicates the layer where it goes.
            "layer.moveTo" if copying(params) => copies.max(1),
            "image.duplicate" => copies,
            _ => 0,
        };
        let document = matches!(id, "file.new" | "file.open" | "image.duplicate");
        if layers > 0 && self.layers.saturating_add(layers) > MAX_RUN_LAYERS {
            return Err(format!("a script can make at most {MAX_RUN_LAYERS} layers"));
        }
        if document && self.documents >= MAX_RUN_DOCUMENTS {
            return Err(format!("a script can make at most {MAX_RUN_DOCUMENTS} documents"));
        }
        self.commands = self.commands.saturating_add(1);
        self.layers = self.layers.saturating_add(layers);
        self.documents = self.documents.saturating_add(u32::from(document));
        Ok(())
    }
}

/// Whether `layer.moveTo` params copy the layer rather than move it.
fn copying(params: &Value) -> bool {
    params.get("copy").and_then(Value::as_bool) == Some(true)
}

/// [`ScriptCommands`] through the app, as the run's own starter: a run started by automation
/// keeps automation's gate for every command it runs.
struct AppCommands<'a> {
    app: &'a mut PhotocraftApp,
    automation: bool,
    /// Started by automation under an agent policy: with that policy gone (the agent link
    /// stopped), its commands are refused rather than run without one.
    gated: bool,
    budget: &'a mut RunBudget,
}

impl AppCommands<'_> {
    /// The layers command `id` copies in the document with serial `target` (or the active one):
    /// `layer.duplicate`'s layer and all it holds (and a copying `layer.moveTo`'s), every layer
    /// for `image.duplicate` (one when merged).
    fn copies(&self, id: &str, target: Option<u64>, params: &Value) -> u32 {
        fn count(l: &photocraft_doc::Layer) -> u32 {
            1u32.saturating_add(l.children().map_or(0, |ch| ch.iter().map(count).fold(0, u32::saturating_add)))
        }
        let session = &self.app.session;
        let doc = match target {
            Some(serial) => session.documents().iter().map(|d| &**d).find(|d| d.serial == serial),
            None => session.active(),
        };
        match id {
            // A move that copies nothing makes no layers: nothing to count.
            "layer.moveTo" if !copying(params) => 0,
            "layer.duplicate" | "layer.moveTo" => {
                let layer = params["layer"].as_u64().map(photocraft_doc::LayerId);
                doc.zip(layer).and_then(|(d, id)| d.doc.layer(id)).map_or(1, count)
            }
            // A merged duplicate is one layer.
            "image.duplicate" if params["mergedOnly"].as_bool() == Some(true) => 1,
            "image.duplicate" => doc.map_or(0, |d| d.doc.layers.iter().map(count).fold(0, u32::saturating_add)),
            _ => 0,
        }
    }
}

impl ScriptCommands for AppCommands<'_> {
    fn execute(&mut self, id: &str, mut params: Value) -> Result<Value, String> {
        #[cfg(test)]
        if id == TEST_FOLD {
            if let Some(last) = self.app.session.journal.last_mut() {
                last.1 = params;
            }
            return Ok(Value::Null);
        }
        if self.gated && self.app.services.automation_command.is_none() && self.app.services.automation_authorize.is_none() {
            return Err("the agent link that started this script has stopped".into());
        }
        let target = params.as_object_mut().and_then(|m| m.remove(ON_DOCUMENT)).and_then(|v| v.as_u64());
        // Counting the copies walks the layers: checked first, and its time is the app's too.
        self.budget.open()?;
        let started = web_time::Instant::now();
        let copies = self.copies(id, target, &params);
        self.budget.app_time += started.elapsed();
        self.budget.charge(id, &params, copies)?;
        let started = web_time::Instant::now();
        let was = std::mem::replace(&mut self.app.automation_input, self.automation);
        let result = (|| {
            if let Some(serial) = target
                && !self.app.session.active().is_some_and(|d| d.serial == serial)
            {
                if !self.app.session.documents().iter().any(|d| d.serial == serial) {
                    return Err("the document is no longer open".to_owned());
                }
                // Refused for another reason (automation's policy): that reason, not a closed document.
                self.app.run("document.activate", json!({ "serial": serial }))?;
            }
            self.app.run(id, params)
        })();
        self.app.automation_input = was;
        self.budget.app_time += started.elapsed();
        result
    }
}

/// Starts runs: the platform's script service.
pub type StartScriptFn = Box<dyn FnMut(ScriptStart) -> Result<Box<dyn ScriptRun>, String>>;

/// The waiting marker of an action playback paused at a script step (see `actions`).
pub(crate) const PLAYBACK_WAIT: &str = "script";
/// How long one frame pumps a run.
const PUMP_BUDGET: Duration = Duration::from_millis(8);
/// Output lines the Scripts Manager keeps.
const LOG_LINES: usize = 2000;
/// Characters of one output line the Scripts Manager shows.
const LOG_LINE_CHARS: usize = 4000;
/// A run shows its Stop window after this long without a dialog (ms).
const STOP_AFTER_MS: f64 = 500.0;

/// The Scripts Manager's view state (the list lives in the preferences).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ManagerUi {
    pub open: bool,
    pub selected: Option<usize>,
    /// The tags field of the selected script while it is being typed in.
    #[serde(skip)]
    pub tags: Option<String>,
}

/// Runs and their output (not saved).
#[derive(Default)]
pub(crate) struct Runtime {
    run: Option<Active>,
    /// The id of the last run started: a late file-picker answer finds its run by it.
    last_id: u64,
    /// Output of recent runs, oldest first.
    pub(crate) log: Vec<String>,
    /// The run is taken out to pump its commands through the app: it is still running.
    pumping: bool,
}

struct Active {
    id: u64,
    run: Box<dyn ScriptRun>,
    name: String,
    dialog: Option<Open>,
    /// Windows shown without waiting: palettes, and a dialog whose event the run is handling.
    windows: BTreeMap<u32, Value>,
    /// The paused action playback to resume when the run ends.
    resume: Option<u64>,
    /// Started by automation input (MCP, the control channel): its commands keep that gate.
    automation: bool,
    /// Started by automation while an agent policy was installed (see [`AppCommands::gated`]).
    gated: bool,
    /// What it has asked of the app so far.
    budget: RunBudget,
    started_ms: f64,
    /// When a dialog became busy with its event (ms): its Stop shows only once that takes a while.
    busy_since_ms: Option<f64>,
}

struct Open {
    kind: ScriptDialog,
    args: Value,
    /// A prompt's text.
    text: String,
    /// ScriptUI values the user changed since the window was sent, by control id.
    edits: BTreeMap<u32, Value>,
    /// A picker's file dialog was asked for.
    asked: bool,
}

impl Runtime {
    /// Whether a script runs.
    pub(crate) fn running(&self) -> bool {
        self.run.is_some() || self.pumping
    }

    /// The paused playback the current run resumes.
    pub(crate) fn resume_token(&self) -> Option<u64> {
        self.run.as_ref().and_then(|a| a.resume)
    }

    fn log_line(&mut self, line: &str) {
        let line: String = line.chars().take(LOG_LINE_CHARS).collect();
        self.log.push(line);
        if self.log.len() > LOG_LINES {
            let extra = self.log.len() - LOG_LINES;
            self.log.drain(..extra);
        }
    }
}

// ---------- starting ----------

/// Start a run. `resume` is the paused action playback to continue when it ends.
pub(crate) fn start(app: &mut PhotocraftApp, start: ScriptStart, resume: Option<u64>) -> Result<Value, String> {
    if app.scripts.running() {
        return Err("a script is already running".into());
    }
    let Some(service) = app.services.scripts.as_mut() else { return Err(NO_SCRIPTS.into()) };
    let name = start.name.clone();
    let run = service(start)?;
    app.scripts.log_line(&format!("— {name}"));
    app.scripts.last_id += 1;
    let id = app.scripts.last_id;
    let automation = app.automation_input;
    let gated = automation && (app.services.automation_command.is_some() || app.services.automation_authorize.is_some());
    app.scripts.run = Some(Active {
        id,
        run,
        name: name.clone(),
        dialog: None,
        windows: BTreeMap::new(),
        resume,
        automation,
        gated,
        budget: RunBudget::default(),
        started_ms: crate::gpu_canvas::now_ms(),
        busy_since_ms: None,
    });
    let template = "Running script {name}…";
    app.ui.status = crate::i18n::fmt(tl!(template), &[("name", &name)]);
    app.ui.status_error = false;
    Ok(json!({ "script": name, "running": true }))
}

/// Resolve `params` through `file.scripts.run` (which records the run in an action being
/// recorded) and start it with `app.name` = `app_name`. A run that does not start is not
/// recorded, and neither is a script text too large to keep in an action.
fn resolve_and_start(app: &mut PhotocraftApp, params: Value, app_name: Option<String>, resume: Option<u64>) -> Result<Value, String> {
    if app.scripts.running() {
        return Err("a script is already running".into());
    }
    if app.services.scripts.is_none() {
        return Err(NO_SCRIPTS.into());
    }
    let journal = app.session.journal.len();
    let large = params.get("source").and_then(Value::as_str).is_some_and(|t| t.len() > photocraft_engine::script_list_cmds::MAX_STORED_SOURCE);
    let r = app.session.execute("file.scripts.run", params).map_err(|e| e.to_string())?;
    if large && app.session.journal.len() > journal {
        app.session.journal.truncate(journal);
        app.scripts.log_line("(this script's text is too large to keep in an action: it is not recorded)");
    }
    let text = |k: &str| r.get(k).and_then(Value::as_str).map(str::to_owned);
    let started = match (text("name"), text("source")) {
        (Some(name), Some(source)) => start(app, ScriptStart { name, source, app_name }, resume),
        _ => Err("the script could not be read".into()),
    };
    if started.is_err() {
        app.session.journal.truncate(journal);
    }
    started
}

/// Run listed script `index`, with `app.name` from its first tag. It is recorded by its file's
/// path, or by its text, never by its place in the list or with its tags: an action recorded
/// from it plays the same script anywhere.
pub(crate) fn run_listed(app: &mut PhotocraftApp, index: usize) -> Result<Value, String> {
    let Some(entry) = app.session.prefs().scripts.entries.get(index).cloned() else { return Err(format!("no script {index} in the list")) };
    let params = match (&entry.path, &entry.source) {
        (Some(path), _) => json!({ "path": path }),
        (None, Some(source)) => json!({ "name": entry.name, "source": source }),
        (None, None) => return Err("the listed script has neither a file nor a text".into()),
    };
    resolve_and_start(app, params, entry.tags.first().cloned(), None)
}

/// An action step `file.scripts.run`: run it, and continue the playback (`token`) when it ends.
pub(crate) fn run_step(app: &mut PhotocraftApp, params: &Value, token: u64) -> Result<Value, String> {
    resolve_and_start(app, params.clone(), None, Some(token))
}

/// Whether a script can run here: without a script service a run fails with [`NO_SCRIPTS`].
pub(crate) fn can_run(app: &PhotocraftApp) -> bool {
    app.services.scripts.is_some()
}

/// Shell handling of the script commands. `None` when the engine handles `id` as is.
pub(crate) fn menu(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    match id {
        "file.scripts.manager" => {
            app.ui.scripts.open = true;
            Some(app.session.execute(id, params.clone()).map_err(|e| e.to_string()))
        }
        // Without a script service no script can run: say so rather than let the engine only
        // resolve the script and report success.
        "file.scripts.run" if !can_run(app) => Some(Err(NO_SCRIPTS.into())),
        "file.scripts.run" => Some(match params.get("index").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()) {
            // A run from the list, as the Scripts Manager runs it.
            Some(i) if params.as_object().is_some_and(|m| m.len() == 1) => run_listed(app, i),
            _ => resolve_and_start(app, params.clone(), None, None),
        }),
        "file.scripts.browse" if can_run(app) => {
            let path = params.get("path").and_then(Value::as_str);
            match path {
                Some(path) if is_js(path) => Some(resolve_and_start(app, json!({ "path": path }), None, None)),
                Some(_) => None,
                None if params.as_object().is_none_or(|m| m.is_empty()) => Some(browse(app)),
                None => None,
            }
        }
        _ => None,
    }
}

/// Whether a file name is a JavaScript script (`.js`, `.jsx`).
fn is_js(name: &str) -> bool {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    file.rsplit_once('.').filter(|(base, _)| !base.is_empty()).is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("js") || ext.eq_ignore_ascii_case("jsx"))
}

/// File › Scripts › Browse…: a JavaScript file runs here, anything else goes to the engine's
/// action scripts as before.
fn browse(app: &mut PhotocraftApp) -> Result<Value, String> {
    let request = in_last_dir(app, PickRequest::open("script").filter("Scripts", &["jsx", "js", "jsxbin", "psjs", "ccjs", "json", "txt", "pcdroplet"]));
    app.ask_file(request, |app, picked| match picked {
        Picked::Paths(paths) => {
            let Some(path) = paths.into_iter().next() else { return Err(crate::file_dialog::CANCELLED.into()) };
            let path = path.to_string_lossy().into_owned();
            if is_js(&path) {
                resolve_and_start(app, json!({ "path": path }), None, None)
            } else {
                // An action script: the engine reads and runs it.
                app.run("file.scripts.browse", json!({ "path": path }))
            }
        }
        Picked::Contents(files) => {
            let Some((name, bytes)) = files.into_iter().next() else { return Err(crate::file_dialog::CANCELLED.into()) };
            if is_js(&name) {
                let source = String::from_utf8(bytes).map_err(|_| format!("{name} is not UTF-8 text"))?;
                resolve_and_start(app, json!({ "name": name, "source": source }), None, None)
            } else {
                app.run("file.scripts.browse", json!({ "name": name, "script": String::from_utf8_lossy(&bytes) }))
            }
        }
        // Save answers: an Open picker gives neither.
        Picked::Saved { .. } | Picked::Cancelled | Picked::Unavailable(_) | Picked::Failed(_) | Picked::Downloaded(_) => {
            Err("the file dialog gave an unexpected answer".into())
        }
    })
}

/// `request`, starting in the folder of the most recent file when there is one.
fn in_last_dir(app: &PhotocraftApp, request: PickRequest) -> PickRequest {
    match last_used_dir(&app.ui.recent_files) {
        Some(dir) => request.directory(dir),
        None => request,
    }
}

/// Stop the current run.
pub(crate) fn cancel(app: &mut PhotocraftApp) {
    if let Some(a) = app.scripts.run.as_mut() {
        a.run.cancel();
        // A dialog it waited at is gone with it.
        a.dialog = None;
    }
}

// ---------- running ----------

/// Pump the run once a frame: commands, output and window updates, until it waits or ends.
pub(crate) fn tick(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(active) = app.scripts.run.as_mut() else { return };
    // A run waiting at a dialog sends nothing, but it may still end (its host was killed), so
    // it is pumped all the same.
    // Not for a playback that stopped meanwhile: the pump below ends that run.
    let owned = active.resume.is_none_or(|token| app.paused_playbacks.holds(token));
    if let Some(open) = active.dialog.as_mut()
        && open.kind.picker()
        && !open.asked
        && owned
    {
        open.asked = true;
        let (id, kind, args) = (active.id, open.kind, open.args.clone());
        ask_picker(app, id, kind, &args);
    }
    let journal = app.session.journal.len();
    // A script command may also fold into the last journal entry (typing, a brush gesture).
    let recording = app.session.recording.target.is_some();
    let last = if recording { app.session.journal.last().cloned() } else { None };
    // The run is taken out while it pumps, so its commands can go through the app.
    let kept = last.clone();
    let pump = move |app: &mut PhotocraftApp| {
        let Some(mut active) = app.scripts.run.take() else { return Vec::new() };
        let (automation, gated) = (active.automation, active.gated);
        app.scripts.pumping = true;
        let pumped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            active.run.pump(&mut AppCommands { app: &mut *app, automation, gated, budget: &mut active.budget }, PUMP_BUDGET)
        }));
        app.scripts.pumping = false;
        let events = match pumped {
            Ok(events) => events,
            Err(panic) => {
                // The app's guard reports the panic; the run ends with it, and scripts can still
                // run afterwards. The commands it ran stay done, but out of an action being
                // recorded, as when it ends normally.
                active.run.cancel();
                if recording && app.session.recording.target.is_some() {
                    app.session.journal.truncate(journal);
                    if let (Some(last), Some(entry)) = (kept, app.session.journal.last_mut()) {
                        *entry = last;
                    }
                }
                std::panic::resume_unwind(panic);
            }
        };
        // Nothing a command did may start another run meanwhile (`start` refuses while one
        // pumps); one that did anyway is stopped rather than lost under this one.
        if let Some(mut other) = app.scripts.run.replace(active) {
            other.run.cancel();
        }
        events
    };
    // A playback's script step edits inside the paused action's history step.
    let Some(resume) = app.scripts.run.as_ref().map(|a| a.resume) else { return };
    let events = match resume {
        Some(token) => match crate::actions::within_paused(app, token, pump) {
            Ok(events) => events,
            Err(e) => {
                if let Some(a) = app.scripts.run.as_mut() {
                    a.run.cancel();
                    // A stopped playback has nothing left to continue or report to.
                    if !app.paused_playbacks.holds(token) {
                        a.resume = None;
                    }
                }
                finish(app, Err(e));
                return;
            }
        },
        None => pump(app),
    };
    // The commands a script runs are its own: an action recorded meanwhile holds the script
    // step, which runs them again when it plays, not each of them.
    if recording && app.session.recording.target.is_some() {
        app.session.journal.truncate(journal);
        if let (Some(last), Some(entry)) = (last, app.session.journal.last_mut()) {
            *entry = last;
        }
    }
    for event in events {
        match event {
            ScriptEvent::Output(line) => app.scripts.log_line(&line),
            ScriptEvent::Window { id, window } => {
                if let Some(a) = app.scripts.run.as_mut() {
                    match window {
                        Some(w) => a.windows.insert(id, w),
                        None => a.windows.remove(&id),
                    };
                }
            }
            ScriptEvent::Dialog { kind, args } => {
                if let Some(a) = app.scripts.run.as_mut() {
                    if kind == ScriptDialog::Window
                        && let Some(id) = args.get("id").and_then(Value::as_u64).and_then(|i| u32::try_from(i).ok())
                    {
                        // The window waits again: no longer shown as busy.
                        a.windows.remove(&id);
                    }
                    let text = match kind {
                        ScriptDialog::Prompt => args.get(1).and_then(Value::as_str).unwrap_or_default().to_owned(),
                        _ => String::new(),
                    };
                    a.dialog = Some(Open { kind, args, text, edits: BTreeMap::new(), asked: false });
                }
            }
            ScriptEvent::Finished(result) => {
                finish(app, result);
                return;
            }
        }
    }
    if let Some(a) = app.scripts.run.as_ref() {
        if a.dialog.is_none() {
            ctx.request_repaint();
        } else {
            // A waiting run sends nothing: keep frames coming to notice one whose host ended.
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
}

fn finish(app: &mut PhotocraftApp, result: Result<(), String>) {
    let Some(active) = app.scripts.run.take() else { return };
    match &result {
        Ok(()) => {
            let template = "Ran script {name}";
            app.ui.status = crate::i18n::fmt(tl!(template), &[("name", &active.name)]);
            app.ui.status_error = false;
        }
        Err(message) => {
            app.scripts.log_line(message);
            let first = message.lines().next().unwrap_or_default();
            let template = "Script {name}: {error}";
            app.ui.status = crate::i18n::fmt(tl!(template), &[("name", &active.name), ("error", first)]);
            app.ui.status_error = true;
        }
    }
    if let Some(token) = active.resume {
        // The playback goes on as the run was started, not as the input of whatever pass the
        // run happened to end in: an unrelated injected pass must not put an action a person
        // played under automation's gate, nor a person's pass lift it from one automation played.
        let was = std::mem::replace(&mut app.automation_input, active.automation);
        crate::actions::script_step_finished(app, token, &active.name, result);
        app.automation_input = was;
    }
}

/// Answer the dialog the run waits at.
fn answer(app: &mut PhotocraftApp, value: Value) {
    let Some(a) = app.scripts.run.as_mut() else { return };
    let open = a.dialog.take();
    // A ScriptUI dialog stays on screen, busy, while the script handles the event.
    if let Some(open) = open
        && open.kind == ScriptDialog::Window
        && let (Some(id), Some(window)) = (open.args.get("id").and_then(Value::as_u64).and_then(|i| u32::try_from(i).ok()), open.args.get("window"))
    {
        a.windows.insert(id, window.clone());
    }
    if let Err(e) = a.run.answer(value) {
        a.run.cancel();
        app.scripts.log_line(&e);
    }
}

/// The extensions of a ScriptUI file filter (`"Images:*.png;*.jpg"`, `"*.txt"`); `None` for
/// every file.
fn filter_extensions(filter: Option<&str>) -> Option<Vec<String>> {
    let filter = filter?;
    let mut out: Vec<String> = Vec::new();
    for part in filter.split([',', ';', ':', ' ']) {
        let Some(ext) = part.trim().strip_prefix("*.") else { continue };
        if ext.is_empty() || ext == "*" {
            return None;
        }
        if ext.chars().all(|c| c.is_ascii_alphanumeric()) && !out.iter().any(|e| e.eq_ignore_ascii_case(ext)) {
            out.push(ext.to_ascii_lowercase());
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Whether run `id` still waits at a file picker.
fn waits_at_picker(app: &PhotocraftApp, id: u64) -> bool {
    app.scripts.run.as_ref().is_some_and(|a| a.id == id && a.dialog.as_ref().is_some_and(|d| d.kind.picker()))
}

/// The file picker a script's `kind` of picker asks for, with its `filter` (Open and Save File)
/// as the file types offered: every file without one.
fn picker_request(kind: ScriptDialog, args: &Value) -> PickRequest {
    let multiple = args.get("multiple").and_then(Value::as_bool).unwrap_or(false);
    let request = match kind {
        ScriptDialog::OpenFile if multiple => PickRequest::open_many("script"),
        ScriptDialog::OpenFile => PickRequest::open("script"),
        ScriptDialog::SaveFile => PickRequest::save("script").file_name("Untitled"),
        _ => return PickRequest::folder("script"),
    };
    match filter_extensions(args.get("filter").and_then(Value::as_str)) {
        Some(exts) => request.filter("Files", &exts.iter().map(String::as_str).collect::<Vec<_>>()),
        None => request,
    }
}

fn ask_picker(app: &mut PhotocraftApp, id: u64, kind: ScriptDialog, args: &Value) {
    let multiple = args.get("multiple").and_then(Value::as_bool).unwrap_or(false);
    let request = picker_request(kind, args);
    let request = if kind == ScriptDialog::SaveFile { request } else { in_last_dir(app, request) };
    let asked = app.ask_file_answer(request, move |app, chosen| {
        // The run was stopped meanwhile (and maybe another started): the answer is nobody's.
        if !waits_at_picker(app, id) {
            return Ok(Value::Null);
        }
        let text = |p: std::path::PathBuf| p.to_string_lossy().into_owned();
        let value = match chosen {
            Picked::Paths(paths) if multiple => json!(paths.into_iter().map(text).collect::<Vec<_>>()),
            Picked::Paths(paths) => json!(paths.into_iter().next().map(text)),
            // Save File's answer: the path chosen, the extension policy applied.
            Picked::Saved { path, .. } => json!(text(path)),
            // The web gives a file's name and contents, not a path: the script gets the name.
            Picked::Contents(files) if multiple => json!(files.into_iter().map(|(n, _)| n).collect::<Vec<_>>()),
            Picked::Contents(files) => json!(files.into_iter().next().map(|(n, _)| n)),
            Picked::Unavailable(why) | Picked::Failed(why) => {
                app.ui.status = why;
                app.ui.status_error = true;
                Value::Null
            }
            Picked::Cancelled | Picked::Downloaded(_) => Value::Null,
        };
        answer(app, value);
        Ok(Value::Null)
    });
    if asked.is_err() {
        // No file dialog here, or another is open: the script sees a cancelled picker.
        answer(app, Value::Null);
    }
}

// ---------- showing ----------

/// Draw the run's dialogs and windows, and the Scripts Manager.
pub(crate) fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    manager_window(app, ctx);
    let Some(active) = app.scripts.run.as_mut() else { return };
    // Palettes, and a dialog busy with its event: shown, not interactive.
    let mut palettes: Vec<(u32, Value)> = active.windows.iter().map(|(k, v)| (*k, v.clone())).collect();
    let waiting_dialog: Vec<(u32, Value)> = palettes.iter().filter(|(_, w)| w["style"] == "dialog").cloned().collect();
    palettes.retain(|(_, w)| w["style"] != "dialog");
    for (id, w) in &palettes {
        let title = w["text"].as_str().unwrap_or_default().to_owned();
        egui::Window::new(if title.is_empty() { active.name.clone() } else { title })
            .id(Id::unique(("script-palette", *id)))
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                let mut st = Draw { window: *id, edits: &mut BTreeMap::new(), event: None, live: false, multiline_focus: false };
                node(ui, w, &mut st, 0, &[]);
            });
    }
    // A busy dialog is modal, so the way to stop the run is inside it.
    let now = crate::gpu_canvas::now_ms();
    active.busy_since_ms = if waiting_dialog.is_empty() { None } else { Some(active.busy_since_ms.unwrap_or(now)) };
    let offer_stop = active.busy_since_ms.is_some_and(|t| now - t >= STOP_AFTER_MS);
    let mut stop = false;
    for (id, w) in &waiting_dialog {
        egui::Modal::new(Id::unique(("script-dialog", *id))).show(ctx, |ui| {
            window_title(ui, w);
            let mut st = Draw { window: *id, edits: &mut BTreeMap::new(), event: None, live: false, multiline_focus: false };
            node(ui, w, &mut st, 0, &[]);
            if offer_stop {
                ui.separator();
                ui.horizontal(|ui| {
                    ui.spinner();
                    stop |= ui.button(tl!("Stop")).clicked();
                });
            }
        });
    }
    if stop {
        cancel(app);
        return;
    }
    let reply = match active.dialog.as_mut() {
        Some(open) if !open.kind.picker() => dialog(ctx, open),
        Some(_) => None,
        None => {
            if waiting_dialog.is_empty() {
                stop_window(ctx, active);
            }
            None
        }
    };
    if let Some(reply) = reply {
        answer(app, reply);
    }
}

fn window_title(ui: &mut egui::Ui, w: &Value) {
    if let Some(t) = w["text"].as_str().filter(|t| !t.is_empty()) {
        ui.heading(t);
        ui.add_space(4.0);
    }
}

/// A small window to stop a run that is busy without a dialog.
fn stop_window(ctx: &egui::Context, active: &mut Active) {
    if crate::gpu_canvas::now_ms() - active.started_ms < STOP_AFTER_MS {
        return;
    }
    let mut stop = false;
    egui::Window::new(tl!("Script"))
        .id(Id::unique("script-running"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -40.0))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.spinner();
                let template = "Running {name}…";
                ui.label(crate::i18n::fmt(tl!(template), &[("name", &active.name)]));
                stop = ui.button(tl!("Stop")).clicked();
            });
        });
    if stop {
        active.run.cancel();
    }
}

/// Draw the dialog the run waits at; `Some` with the answer once the user is done.
fn dialog(ctx: &egui::Context, open: &mut Open) -> Option<Value> {
    let arg = |i: usize| open.args.get(i).and_then(Value::as_str).unwrap_or_default().to_owned();
    // The runtime takes a plain Escape out of egui's input: read it where it is, once.
    let esc = crate::window_keys::take_escape(ctx);
    let enter = ctx.input(|i| i.key_pressed(egui::Key::Enter));
    match open.kind {
        ScriptDialog::Alert => {
            let mut done = false;
            egui::Modal::new(Id::unique("script-alert")).show(ctx, |ui| {
                let title = arg(1);
                ui.heading(if title.is_empty() { tl!("Script Alert").to_string() } else { title });
                ui.add_space(4.0);
                ui.label(arg(0));
                ui.add_space(8.0);
                done = ui.button(tl!("OK")).clicked();
            });
            (done || esc || enter).then_some(Value::Null)
        }
        ScriptDialog::Confirm => {
            let mut chosen = None;
            let no_default = open.args.get(1).and_then(Value::as_str).is_some_and(|v| v == "true");
            egui::Modal::new(Id::unique("script-confirm")).show(ctx, |ui| {
                let title = arg(2);
                ui.heading(if title.is_empty() { tl!("Script Alert").to_string() } else { title });
                ui.add_space(4.0);
                ui.label(arg(0));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(tl!("Yes")).clicked() {
                        chosen = Some(true);
                    }
                    if ui.button(tl!("No")).clicked() {
                        chosen = Some(false);
                    }
                });
            });
            if esc {
                chosen = Some(false);
            } else if enter && chosen.is_none() {
                // Enter on a focused button clicks it; the default is for Enter elsewhere.
                chosen = Some(!no_default);
            }
            chosen.map(Value::Bool)
        }
        ScriptDialog::Prompt => {
            let mut chosen = None;
            egui::Modal::new(Id::unique("script-prompt")).show(ctx, |ui| {
                let title = arg(2);
                ui.heading(if title.is_empty() { tl!("Script Prompt").to_string() } else { title });
                ui.add_space(4.0);
                // The field is named by the prompt's text, for a screen reader.
                let label = ui.label(arg(0));
                let r = ui.add(egui::TextEdit::singleline(&mut open.text).desired_width(320.0)).labelled_by(label.id);
                if !r.has_focus() && !r.lost_focus() && ui.memory(|m| m.focused().is_none()) {
                    r.request_focus();
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(tl!("OK")).clicked() {
                        chosen = Some(Value::String(open.text.clone()));
                    }
                    if ui.button(tl!("Cancel")).clicked() {
                        chosen = Some(Value::Null);
                    }
                });
            });
            if esc {
                chosen = Some(Value::Null);
            } else if enter && chosen.is_none() {
                // Enter on a focused button clicks it (Cancel stays Cancel); elsewhere it is OK.
                chosen = Some(Value::String(open.text.clone()));
            }
            chosen
        }
        ScriptDialog::Window => {
            let id = open.args.get("id").and_then(Value::as_u64).and_then(|i| u32::try_from(i).ok())?;
            let tree = open.args.get("window").cloned().unwrap_or(Value::Null);
            let mut st = Draw { window: id, edits: &mut open.edits, event: None, live: true, multiline_focus: false };
            egui::Modal::new(Id::unique(("script-dialog", id))).show(ctx, |ui| {
                window_title(ui, &tree);
                node(ui, &tree, &mut st, 0, &[]);
            });
            let mut event = st.event.take();
            let multiline = st.multiline_focus;
            if event.is_none() && esc {
                event = Some(match find_flag(&tree, "cancel", 0) {
                    Some(c) => ("click", c),
                    None => ("close", id),
                });
            }
            if event.is_none()
                && enter
                && !multiline
                && let Some(d) = find_flag(&tree, "default", 0)
            {
                event = Some(("click", d));
            }
            let (name, target) = event?;
            let values: Vec<Value> = open.edits.iter().map(|(k, v)| json!({ "id": k, "value": v })).collect();
            Some(json!({ "event": name, "id": target, "values": values }))
        }
        ScriptDialog::OpenFile | ScriptDialog::SaveFile | ScriptDialog::SelectFolder => None,
    }
}

/// The enabled, visible button marked `flag` (`"default"` or `"cancel"`).
fn find_flag(n: &Value, flag: &str, depth: usize) -> Option<u32> {
    if depth > MAX_DRAW_DEPTH || n["visible"] == false || n["enabled"] == false {
        return None;
    }
    if n[flag] == true {
        return n["id"].as_u64().and_then(|i| u32::try_from(i).ok());
    }
    n["children"].as_array()?.iter().find_map(|c| find_flag(c, flag, depth + 1))
}

/// Deepest nesting drawn (the script side allows 16).
const MAX_DRAW_DEPTH: usize = 32;

struct Draw<'a> {
    window: u32,
    edits: &'a mut BTreeMap<u32, Value>,
    /// The user's event this frame: (`click` | `change`, control).
    event: Option<(&'static str, u32)>,
    /// Interactive (a dialog waiting for the user), or only shown.
    live: bool,
    multiline_focus: bool,
}

impl Draw<'_> {
    /// The pass's event: the first, except that a click outranks a `change` before it (an edit
    /// field loses focus to the button clicked, in the same pass; the edit travels with the
    /// click's values).
    fn fire(&mut self, name: &'static str, id: u32) {
        if self.live && (self.event.is_none() || (name == "click" && self.event.is_some_and(|(n, _)| n == "change"))) {
            self.event = Some((name, id));
        }
    }
}

fn width_of(n: &Value) -> Option<f32> {
    n["width"].as_f64().map(|w| w.clamp(8.0, 2000.0) as f32)
}

/// Draw control `n`; `radios` are the ids of the radio buttons beside it.
fn node(ui: &mut egui::Ui, n: &Value, st: &mut Draw, depth: usize, radios: &[u32]) {
    if depth > MAX_DRAW_DEPTH || n["visible"] == false {
        return;
    }
    let Some(id) = n["id"].as_u64().and_then(|i| u32::try_from(i).ok()) else { return };
    let enabled = n["enabled"] != false && st.live && ui.is_enabled();
    let text = n["text"].as_str().unwrap_or_default();
    let tip = n["helpTip"].as_str().filter(|t| !t.is_empty());
    let salt = (st.window, id);
    let response = match n["type"].as_str().unwrap_or_default() {
        "window" | "group" => {
            children(ui, n, st, depth);
            None
        }
        "panel" => {
            let r = ui.group(|ui| {
                if !text.is_empty() {
                    ui.label(RichText::new(text).strong());
                }
                children(ui, n, st, depth);
            });
            Some(r.response)
        }
        "statictext" => Some(ui.label(text)),
        "button" => {
            let mut b = egui::Button::new(text);
            if let Some(w) = width_of(n) {
                b = b.min_size(egui::vec2(w, 0.0));
            }
            let r = ui.add_enabled(enabled, b);
            if r.clicked() {
                st.fire("click", id);
            }
            Some(r)
        }
        "checkbox" => {
            let mut v = st.edits.get(&id).and_then(Value::as_bool).unwrap_or(n["value"] == true);
            let r = ui.add_enabled(enabled, egui::Checkbox::new(&mut v, text));
            if r.changed() {
                st.edits.insert(id, json!(v));
                st.fire("click", id);
            }
            Some(r)
        }
        "radiobutton" => {
            let on = st.edits.get(&id).and_then(Value::as_bool).unwrap_or(n["value"] == true);
            let r = ui.add_enabled(enabled, egui::RadioButton::new(on, text));
            if r.clicked() && !on {
                for other in radios {
                    st.edits.insert(*other, json!(*other == id));
                }
                st.fire("click", id);
            }
            Some(r)
        }
        "edittext" => {
            let original = text.to_owned();
            let mut t = st.edits.get(&id).and_then(Value::as_str).map(str::to_owned).unwrap_or_else(|| original.clone());
            let multiline = n["multiline"] == true;
            let width = width_of(n).or_else(|| n["characters"].as_f64().map(|c| (c as f32 * 7.5 + 8.0).clamp(24.0, 2000.0))).unwrap_or(160.0);
            let edit = if multiline { egui::TextEdit::multiline(&mut t).desired_rows(4) } else { egui::TextEdit::singleline(&mut t) };
            let edit = edit.desired_width(width).password(n["noecho"] == true).interactive(n["readonly"] != true);
            let r = ui.add_enabled(enabled, edit);
            if r.changed() {
                st.edits.insert(id, json!(t));
            }
            if multiline && r.has_focus() {
                st.multiline_focus = true;
            }
            if r.lost_focus() && st.edits.get(&id).and_then(Value::as_str).is_some_and(|v| v != original) {
                st.fire("change", id);
            }
            Some(r)
        }
        "dropdownlist" => {
            let items = n["items"].as_array().cloned().unwrap_or_default();
            let current = st.edits.get(&id).and_then(Value::as_u64).or_else(|| n["selection"].as_u64()).and_then(|i| usize::try_from(i).ok());
            // Separators (non-text items) are not choices.
            let entries: Vec<craft::ui::contract::Entry<usize>> =
                items.iter().enumerate().filter_map(|(i, item)| item.as_str().map(|t| craft::ui::contract::Entry::new(i, t.to_owned()))).collect();
            let label = n["text"].as_str().unwrap_or_default().to_owned();
            let picked = ui
                .add_enabled_ui(enabled, |ui| {
                    craft::ui::pickers::Select::new(label, craft::ui::contract::Bind::given(current.unwrap_or(usize::MAX)), &entries).show(ui)
                })
                .inner;
            if let Some(&i) = picked.events.committed().filter(|i| Some(**i) != current) {
                st.edits.insert(id, json!(i));
                st.fire("change", id);
            }
            Some(picked.response)
        }
        "listbox" => {
            let items = n["items"].as_array().cloned().unwrap_or_default();
            let current = st.edits.get(&id).and_then(Value::as_u64).or_else(|| n["selection"].as_u64()).and_then(|i| usize::try_from(i).ok());
            let mut chosen = None;
            let height = n["height"].as_f64().map_or(120.0, |h| h.clamp(24.0, 2000.0) as f32);
            let r = ui.add_enabled_ui(enabled, |ui| {
                ui.group(|ui| {
                    if let Some(w) = width_of(n) {
                        ui.set_min_width(w);
                    }
                    egui::ScrollArea::vertical().id_salt(("script-list", salt)).max_height(height).show(ui, |ui| {
                        for (i, item) in items.iter().enumerate() {
                            match item.as_str() {
                                Some(t) => {
                                    if ui.selectable_label(current == Some(i), t).clicked() {
                                        chosen = Some(i);
                                    }
                                }
                                None => {
                                    ui.separator();
                                }
                            }
                        }
                    });
                });
            });
            if let Some(i) = chosen.filter(|i| Some(*i) != current) {
                st.edits.insert(id, json!(i));
                st.fire("change", id);
            }
            Some(r.response)
        }
        "progressbar" => {
            let (min, max) = (n["min"].as_f64().unwrap_or(0.0), n["max"].as_f64().unwrap_or(100.0));
            let v = n["value"].as_f64().unwrap_or(min);
            let span = max - min;
            let fraction = if span > 0.0 { ((v - min) / span).clamp(0.0, 1.0) } else { 0.0 };
            Some(ui.add(egui::ProgressBar::new(fraction as f32).desired_width(width_of(n).unwrap_or(200.0))))
        }
        "slider" => {
            let (min, max) = (n["min"].as_f64().unwrap_or(0.0), n["max"].as_f64().unwrap_or(100.0));
            let (min, max) = if max >= min { (min, max) } else { (max, min) };
            let mut v = st.edits.get(&id).and_then(Value::as_f64).or_else(|| n["value"].as_f64()).unwrap_or(min);
            let r = ui.add_enabled(enabled, egui::Slider::new(&mut v, min..=max));
            if r.changed() {
                st.edits.insert(id, json!(v));
            }
            if r.drag_stopped() || (r.changed() && !r.dragged()) {
                st.fire("change", id);
            }
            Some(r)
        }
        _ => None,
    };
    if let (Some(r), Some(tip)) = (response, tip) {
        r.on_hover_text(tip);
    }
}

fn children(ui: &mut egui::Ui, n: &Value, st: &mut Draw, depth: usize) {
    let Some(kids) = n["children"].as_array() else { return };
    let radios: Vec<u32> = kids.iter().filter(|k| k["type"] == "radiobutton").filter_map(|k| k["id"].as_u64()).filter_map(|i| u32::try_from(i).ok()).collect();
    // A disabled group or panel disables everything in it, as the script side counts it.
    let disabled = n["enabled"] == false;
    let draw = |ui: &mut egui::Ui, st: &mut Draw| {
        if disabled {
            ui.disable();
        }
        for k in kids {
            node(ui, k, st, depth + 1, &radios);
        }
    };
    if n["orientation"] == "row" {
        ui.horizontal(|ui| draw(ui, st));
    } else {
        ui.vertical(|ui| draw(ui, st));
    }
}

// ---------- Scripts Manager ----------

fn manager_window(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.ui.scripts.open {
        return;
    }
    let mut open = true;
    // Only what is drawn: a listed script's text stays in the preferences.
    let entries: Vec<(String, Option<String>, Vec<String>)> =
        app.session.prefs().scripts.entries.iter().map(|e| (e.name.clone(), e.path.clone(), e.tags.clone())).collect();
    let selected = app.ui.scripts.selected.filter(|i| *i < entries.len());
    let running = app.scripts.running();
    let can_run = can_run(app);
    let mut action: Option<ManagerAction> = None;
    // Leaving the tags field and clicking a button are one click: both happen.
    let mut tags_commit: Option<(usize, String)> = None;
    egui::Window::new(tl!("Scripts Manager")).id(Id::unique("scripts-manager")).open(&mut open).default_width(460.0).collapsible(false).show(ctx, |ui| {
        ui.horizontal(|ui| {
            if ui.button(tl!("Load…")).clicked() {
                action = Some(ManagerAction::Load);
            }
            if ui.add_enabled(selected.is_some(), egui::Button::new(tl!("Remove"))).clicked() {
                action = selected.map(ManagerAction::Remove);
            }
            ui.separator();
            if ui.add_enabled(selected.is_some() && !running && can_run, egui::Button::new(tl!("Run"))).clicked() {
                action = selected.map(ManagerAction::Run);
            }
            if ui.add_enabled(running, egui::Button::new(tl!("Stop"))).clicked() {
                action = Some(ManagerAction::Stop);
            }
        });
        ui.separator();
        egui::ScrollArea::vertical().id_salt("scripts-manager-list").max_height(220.0).show(ui, |ui| {
            if entries.is_empty() {
                ui.weak(tl!("No scripts yet. Load… adds .js and .jsx files."));
            }
            for (i, (name, path, tags)) in entries.iter().enumerate() {
                let label = if tags.is_empty() { name.clone() } else { format!("{name}  [{}]", tags.join(", ")) };
                let r = ui.selectable_label(selected == Some(i), label);
                let r = match path {
                    Some(p) => r.on_hover_text(p),
                    None => r,
                };
                if r.clicked() {
                    action = Some(ManagerAction::Select(i));
                }
                if r.double_clicked() && can_run && !running {
                    action = Some(ManagerAction::Run(i));
                }
            }
        });
        if let Some(i) = selected
            && let Some((_, _, tags)) = entries.get(i)
        {
            ui.separator();
            ui.horizontal(|ui| {
                let label = ui.label(tl!("Tags:"));
                let mut text = app.ui.scripts.tags.clone().unwrap_or_else(|| tags.join(", "));
                let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(280.0).hint_text(tl!("comma-separated"))).labelled_by(label.id);
                if r.changed() {
                    app.ui.scripts.tags = Some(text.clone());
                }
                if r.lost_focus() && app.ui.scripts.tags.is_some() {
                    tags_commit = Some((i, text));
                }
            });
            ui.weak(tl!("The first tag is what scripts see as app.name when run from this list."));
        }
        ui.separator();
        ui.label(RichText::new(tl!("Output")).strong());
        // Only the lines in view are laid out, however long the log.
        let row = ui.text_style_height(&egui::TextStyle::Monospace);
        egui::ScrollArea::vertical().id_salt("scripts-manager-log").max_height(160.0).stick_to_bottom(true).show_rows(
            ui,
            row,
            app.scripts.log.len(),
            |ui, rows| {
                for line in app.scripts.log.get(rows).unwrap_or_default() {
                    ui.add(egui::Label::new(RichText::new(line).monospace()).truncate());
                }
            },
        );
        if !can_run {
            ui.weak(tl!("This build of PhotoCraft can't run scripts."));
        }
    });
    if !open {
        app.ui.scripts.open = false;
    }
    if let Some((i, text)) = tags_commit {
        app.ui.scripts.tags = None;
        if let Err(e) = app.run("file.scripts.setTags", json!({ "index": i, "tags": text })) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
    let result = match action {
        None => return,
        Some(ManagerAction::Select(i)) => {
            app.ui.scripts.selected = Some(i);
            app.ui.scripts.tags = None;
            return;
        }
        Some(ManagerAction::Stop) => {
            cancel(app);
            return;
        }
        Some(ManagerAction::Load) => load_dialog(app),
        Some(ManagerAction::Remove(i)) => {
            app.ui.scripts.tags = None;
            let r = app.run("file.scripts.remove", json!({ "index": i }));
            let n = app.session.prefs().scripts.entries.len();
            app.ui.scripts.selected = (n > 0).then(|| i.min(n - 1));
            r
        }
        Some(ManagerAction::Run(i)) => run_listed(app, i),
    };
    if let Err(e) = result
        && e != crate::file_dialog::CANCELLED
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

enum ManagerAction {
    Load,
    Remove(usize),
    Run(usize),
    Stop,
    Select(usize),
}

/// The manager's Load…: add the chosen `.js` / `.jsx` files to the list.
fn load_dialog(app: &mut PhotocraftApp) -> Result<Value, String> {
    let request = in_last_dir(app, PickRequest::open_many("script").filter("Scripts", &["jsx", "js"]));
    app.ask_file(request, |app, picked| {
        // A script is UTF-8 text, kept as written: anything else is refused, not repaired.
        let text = |name: String, bytes: Vec<u8>| match String::from_utf8(bytes) {
            Ok(source) => Ok(json!({ "name": name, "source": source })),
            Err(_) => Err(format!("{name} is not UTF-8 text")),
        };
        let loads: Vec<Value> = match picked {
            Picked::Paths(paths) => paths.into_iter().map(|p| json!({ "path": p.to_string_lossy() })).collect(),
            Picked::Contents(files) => files.into_iter().map(|(name, bytes)| text(name, bytes)).collect::<Result<_, _>>()?,
            Picked::Saved { .. } | Picked::Cancelled | Picked::Unavailable(_) | Picked::Failed(_) | Picked::Downloaded(_) => {
                return Err("the file dialog gave an unexpected answer".into());
            }
        };
        let mut last = Value::Null;
        for params in loads {
            last = app.run("file.scripts.load", params)?;
            if let Some(i) = last.get("index").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()) {
                app.ui.scripts.selected = Some(i);
                app.ui.scripts.tags = None;
            }
        }
        Ok(last)
    })
}

#[cfg(test)]
mod tests;
