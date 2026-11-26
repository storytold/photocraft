//! JavaScript scripts (`.js`, `.jsx`): a run through the platform's script service, and its
//! dialogs.
//!
//! The platform starts runs ([`Services::scripts`](crate::Services)): on the desktop each run has
//! its own script host process. The run is pumped once a frame ([`tick`]); the session commands
//! it asks for run there through the app, and what it needs from the user (an alert, a
//! confirmation, a prompt) opens as one of the app's own dialogs, so the dialog gate, the keyboard
//! and automation (`ui.dialog.*`) treat it as any other, and is answered back. One run at a time.

use std::time::Duration;

use craft::ui::contract::Bind;
use craft::ui::dialog::{DialogId, DialogResult, PromptChoice, StandardPrompt};
use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::sections::{DialogRole, FooterButton};
use egui::Id;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::dialogs::params::{self, ParamFamily};
use craft::host::{PickRequest, Picked};

use crate::file_dialog::last_used_dir;

/// What the platform needs to start a run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptStart {
    /// Shown to the user and in the script's error messages.
    pub name: String,
    pub source: String,
    /// `app.name` for this run; `None` for the default.
    pub app_name: Option<String>,
}

/// A dialog a run waits at. Its arguments are the script's, as text: `[message, title]` for
/// an alert, `[message, noAsDefault, title]` for a confirmation, `[message, default, title]` for
/// a prompt, each but the message optional. A file picker's are `{"prompt", "filter",
/// "multiple"}` (only those its kind takes), and its answer is a path, a list of paths, or `null`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptDialog {
    Alert,
    Confirm,
    Prompt,
    /// `File.openDialog`: one file, or several with `multiple`.
    OpenFile,
    /// `File.saveDialog`.
    SaveFile,
    /// `Folder.selectDialog`.
    SelectFolder,
}

impl ScriptDialog {
    /// Whether it is a file picker: the app's file dialog, not one on the dialog stack.
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
pub(crate) const NO_SCRIPTS: &str = "This build of PhotoCraft can't run JavaScript scripts.";

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

/// The parameter a script command names its document by: the document's serial. The same
/// name as `photocraft_script_js::commands::ON_DOCUMENT` (the desktop app checks they match).
pub const ON_DOCUMENT: &str = "onDocument";

/// What one run may do through the app. The script's own time is bounded by its host, which
/// also caps how many commands it asks for; this bounds the work it asks the app for, whose
/// memory and time are the app's.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct RunBudget {
    layers: u32,
    documents: u32,
    app_time: Duration,
}

/// Layers one run may make.
const MAX_RUN_LAYERS: u32 = 10_000;
/// Documents one run may make.
const MAX_RUN_DOCUMENTS: u32 = 64;
/// Time one run may take of the app for its commands.
const MAX_RUN_APP_TIME: Duration = Duration::from_secs(30 * 60);

impl RunBudget {
    /// Whether the run may still have the app run command `id` at all: app time left. Deleting
    /// a layer is let through without it, so a script's cleanup of a copy it couldn't place
    /// (`duplicate` of a group into itself) never leaves the copy behind; it makes nothing.
    fn open(&self, id: &str) -> Result<(), String> {
        if self.app_time >= MAX_RUN_APP_TIME && id != "layer.delete" {
            return Err(tl!("The script has used all the time PhotoCraft gives a script's commands.").into());
        }
        Ok(())
    }

    /// Charge command `id`, which makes `copies` layers when it duplicates them (a group with
    /// everything in it, or a whole document), or say why the run may not run it.
    fn charge(&mut self, id: &str, params: &Value, copies: u32) -> Result<(), String> {
        self.open(id)?;
        let layers = match id {
            "layer.new.layer" | "layer.new.group" | "type.create" => 1,
            "layer.duplicate" => copies.max(1),
            // A move with `copy` duplicates the layer where it goes.
            "layer.moveTo" if copying(params) => copies.max(1),
            "image.duplicate" => copies,
            _ => 0,
        };
        let document = matches!(id, "file.new" | "image.duplicate");
        if layers > 0 && self.layers.saturating_add(layers) > MAX_RUN_LAYERS {
            return Err(crate::i18n::fmt(tl!("A script can make at most {count} layers."), &[("count", &MAX_RUN_LAYERS.to_string())]));
        }
        if document && self.documents >= MAX_RUN_DOCUMENTS {
            return Err(crate::i18n::fmt(tl!("A script can make at most {count} documents."), &[("count", &MAX_RUN_DOCUMENTS.to_string())]));
        }
        self.layers = self.layers.saturating_add(layers);
        self.documents = self.documents.saturating_add(u32::from(document));
        Ok(())
    }
}

/// Whether `layer.moveTo` params copy the layer rather than move it.
fn copying(params: &Value) -> bool {
    params.get("copy").and_then(Value::as_bool) == Some(true)
}

/// The engine's command gate a run was started under, if any.
type Gate = Option<crate::services::AutomationAuthorizeFn>;

/// [`ScriptCommands`] through the app, as the run's own starter: a run started by automation
/// keeps automation's gates (the app's and the engine's) for every command it runs. Either way its
/// commands are programmatic, as `engine.execute`'s are: they answer now and open no dialog.
struct AppCommands<'a> {
    app: &'a mut PhotocraftApp,
    automation: bool,
    gate: Gate,
    /// What the run's commands have taken of the app so far.
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
        let target = params.as_object_mut().and_then(|m| m.remove(ON_DOCUMENT)).and_then(|v| v.as_u64());
        // Counting the copies walks the layers: checked first, and its time is the app's too.
        self.budget.open(id)?;
        let started = web_time::Instant::now();
        let copies = self.copies(id, target, &params);
        self.budget.app_time += started.elapsed();
        self.budget.charge(id, &params, copies)?;
        let started = web_time::Instant::now();
        let was = std::mem::replace(&mut self.app.automation_input, self.automation);
        let was_programmatic = std::mem::replace(&mut self.app.programmatic_playback, true);
        let was_gate = std::mem::replace(&mut self.app.session.authorize, self.gate);
        // A command that panics leaves the app's input provenance as it was, too: the panic goes
        // on to the run's guard once it is put back.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if let Some(serial) = target
                && !self.app.session.active().is_some_and(|d| d.serial == serial)
            {
                if !self.app.session.documents().iter().any(|d| d.serial == serial) {
                    return Err(tl!("The document is no longer open.").to_owned());
                }
                // Refused for another reason (automation's policy): that reason, not a closed document.
                self.app.run("document.activate", json!({ "serial": serial }))?;
            }
            self.app.run(id, params)
        }));
        self.app.automation_input = was;
        self.app.programmatic_playback = was_programmatic;
        self.app.session.authorize = was_gate;
        let result = match result {
            Ok(result) => result,
            Err(panic) => std::panic::resume_unwind(panic),
        };
        self.budget.app_time += started.elapsed();
        result
    }
}

/// Starts runs: the platform's script service.
pub type StartScriptFn = Box<dyn FnMut(ScriptStart) -> Result<Box<dyn ScriptRun>, String>>;

/// How long one frame pumps a run.
const PUMP_BUDGET: Duration = Duration::from_millis(8);
/// Output lines kept.
const LOG_LINES: usize = 2000;
/// Characters of one output line kept.
const LOG_LINE_CHARS: usize = 4000;
/// A run shows its Stop window after this long without a dialog (ms).
const STOP_AFTER_MS: f64 = 500.0;

/// Runs and their output (not saved).
#[derive(Default)]
pub(crate) struct Runtime {
    run: Option<Active>,
    /// Output of recent runs, oldest first.
    pub(crate) log: std::collections::VecDeque<String>,
    /// The run is taken out to pump its commands through the app: it is still running.
    pumping: bool,
    /// The Script Output window is shown: from a run's first line until the user closes it.
    output_open: bool,
    /// Numbers the dialogs, so each one's answer is kept apart from every other prompt's.
    dialogs: u64,
}

struct Active {
    run: Box<dyn ScriptRun>,
    name: String,
    /// The dialog the run waits at, on the app's dialog stack.
    dialog: Option<Open>,
    /// Started by automation input (MCP, the control channel): its commands keep that gate.
    automation: bool,
    /// The engine's gate it was started under (the control channel's, an agent's): its commands
    /// run under it.
    gate: Gate,
    /// What its commands have taken of the app.
    budget: RunBudget,
    started_ms: f64,
    /// It has written a line: the output window opened for it.
    wrote: bool,
}

#[derive(Clone, Copy)]
struct Open {
    kind: ScriptDialog,
    /// Its place on the dialog stack; `None` for a file picker.
    id: Option<DialogId>,
    /// Numbers it among every script dialog: a picker's late answer names the one it is for.
    token: u64,
}

impl Runtime {
    /// Whether a script runs.
    pub(crate) fn running(&self) -> bool {
        self.run.is_some() || self.pumping
    }

    /// Keep `text`, one log row per line it holds: the output window lays out rows of one line.
    fn log_line(&mut self, text: &str) {
        for line in text.lines() {
            let mut chars = line.chars();
            let mut line: String = chars.by_ref().take(LOG_LINE_CHARS).collect();
            if chars.next().is_some() {
                line.push('…');
            }
            // The oldest line goes as each new one comes once the log is full: no shifting.
            if self.log.len() >= LOG_LINES {
                self.log.pop_front();
            }
            self.log.push_back(line);
        }
    }
}

// ---------- starting ----------

/// Start a run.
pub(crate) fn start(app: &mut PhotocraftApp, start: ScriptStart) -> Result<Value, String> {
    if app.scripts.running() {
        return Err(tl!(ALREADY_RUNNING).into());
    }
    let Some(service) = app.services.scripts.as_mut() else { return Err(tl!(NO_SCRIPTS).into()) };
    let name = start.name.clone();
    let run = service(start)?;
    app.scripts.log_line(&format!("— {name}"));
    // A control request runs under the engine's gate without marking its input as automation's:
    // a run it starts is automation's all the same, and keeps the app's command policy too.
    let automation = app.automation_input || app.session.authorize.is_some();
    let gate = app.session.authorize.or(if automation { app.services.automation_authorize } else { None });
    app.scripts.run = Some(Active {
        run,
        name: name.clone(),
        dialog: None,
        automation,
        gate,
        budget: RunBudget::default(),
        started_ms: crate::gpu_canvas::now_ms(),
        wrote: false,
    });
    let template = "Running script {name}…";
    app.ui.status = crate::i18n::fmt(tl!(template), &[("name", &name)]);
    app.ui.status_error = false;
    Ok(json!({ "script": name, "running": true }))
}

/// Why a second run can't start.
const ALREADY_RUNNING: &str = "A script is already running.";

/// Read the script `params` name (`{"path"}`, or `{"name", "source"}`) and start it.
fn resolve_and_start(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    if app.scripts.running() {
        return Err(tl!(ALREADY_RUNNING).into());
    }
    if app.services.scripts.is_none() {
        return Err(tl!(NO_SCRIPTS).into());
    }
    let (name, source) = photocraft_engine::script_cmds::resolve(params).map_err(|e| e.to_string())?;
    start(app, ScriptStart { name, source, app_name: None })
}

/// Whether a script can run here: without a script service a run fails with [`NO_SCRIPTS`].
pub(crate) fn can_run(app: &PhotocraftApp) -> bool {
    app.services.scripts.is_some()
}

/// `file.scripts.run`, or `file.scripts.browse` of a JavaScript file, however it is reached (a
/// menu, a key, `engine.execute`, a nested command), after the app has authorized it: the engine
/// alone can't run a script. `None` for anything else, which the engine runs.
pub(crate) fn intercept(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if !matches!(id, "file.scripts.run" | "file.scripts.browse") {
        return None;
    }
    // The engine's gate (the control channel's, an agent's) judges it, as it would the command.
    if let Some(gate) = app.session.authorize
        && let Err(e) = gate(id, params)
    {
        return Some(Err(e.to_string()));
    }
    match id {
        "file.scripts.run" => Some(resolve_and_start(app, params)),
        // A JavaScript file Browse names runs as a script here (or is refused with its reason when
        // it is one PhotoCraft can't run); without a script service the engine refuses it.
        "file.scripts.browse" if can_run(app) => {
            let path = params.get("path").and_then(Value::as_str).filter(|p| is_js(p))?;
            // It runs a script: judged as `file.scripts.run` (the app's policy, the engine's
            // gate), as the picker's choice is.
            Some(app.run_command("file.scripts.run", json!({ "path": path })))
        }
        _ => None,
    }
}

/// File › Scripts › Browse… with no file named: its picker. `None` when the engine handles it.
pub(crate) fn menu(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    (id == "file.scripts.browse" && can_run(app) && params.as_object().is_none_or(|m| m.is_empty())).then(|| browse(app))
}

/// Whether a file name is a JavaScript script: one PhotoCraft runs (`.js`, `.jsx`), or one it
/// refuses with its reason (`.jsxbin`, `.psjs`, `.ccjs`).
fn is_js(name: &str) -> bool {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    file.rsplit_once('.')
        .filter(|(base, _)| !base.is_empty())
        .is_some_and(|(_, ext)| ["js", "jsx", "jsxbin", "psjs", "ccjs"].iter().any(|js| ext.eq_ignore_ascii_case(js)))
}

/// File › Scripts › Browse…: a JavaScript file runs here, anything else goes to the engine's
/// action scripts as before.
fn browse(app: &mut PhotocraftApp) -> Result<Value, String> {
    let request = in_last_dir(app, PickRequest::open("script").filter(tl!("Scripts"), &["jsx", "js", "jsxbin", "psjs", "ccjs", "json", "txt", "pcdroplet"]));
    // The pick is answered in a later frame: what it runs keeps the gates of the request that
    // opened the picker, and gains those of the frame that answers it (an agent may answer a
    // picker a person opened).
    let opened = (app.automation_input, app.session.authorize.or(if app.automation_input { app.services.automation_authorize } else { None }));
    app.ask_file(request, move |app, picked| {
        let automation = opened.0 || app.automation_input;
        let gate = opened.1.or(app.session.authorize).or(if app.automation_input { app.services.automation_authorize } else { None });
        let was = (std::mem::replace(&mut app.automation_input, automation), std::mem::replace(&mut app.session.authorize, gate));
        // Put back on a panic too: a later frame must not keep this pick's gates.
        let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_picked(app, picked)));
        (app.automation_input, app.session.authorize) = was;
        ran.unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}

/// Run the file Browse picked: a JavaScript file as a script, anything else as an action script.
fn run_picked(app: &mut PhotocraftApp, picked: Picked) -> Result<Value, String> {
    match picked {
        Picked::Paths(paths) => {
            let Some(path) = paths.into_iter().next() else { return Err(crate::file_dialog::CANCELLED.into()) };
            let path = path.to_string_lossy().into_owned();
            if is_js(&path) {
                app.run("file.scripts.run", json!({ "path": path }))
            } else {
                // An action script: the engine reads and runs it.
                let name = path.rsplit(['/', '\\']).next().unwrap_or(&path).to_owned();
                let ran = app.run("file.scripts.browse", json!({ "path": path }));
                ran_action_script(app, &name, ran)
            }
        }
        Picked::Contents(files) => {
            let Some((name, bytes)) = files.into_iter().next() else { return Err(crate::file_dialog::CANCELLED.into()) };
            if is_js(&name) {
                // A script PhotoCraft can't run says so before its text is read.
                photocraft_engine::script_cmds::runnable("file.scripts.run", &name).map_err(|e| e.to_string())?;
                let template = "{name} is not UTF-8 text.";
                let source = String::from_utf8(bytes).map_err(|_| crate::i18n::fmt(tl!(template), &[("name", &name)]))?;
                app.run("file.scripts.run", json!({ "name": name, "source": source }))
            } else {
                let ran = app.run("file.scripts.browse", json!({ "name": name, "script": String::from_utf8_lossy(&bytes) }));
                ran_action_script(app, &name, ran)
            }
        }
        // Save answers: an Open picker gives neither.
        Picked::Saved { .. } | Picked::Cancelled | Picked::Unavailable(_) | Picked::Failed(_) | Picked::Downloaded(_) => {
            Err(tl!("The file dialog gave an unexpected answer.").into())
        }
    }
}

/// An action script's result, said in the status bar when it ran.
fn ran_action_script(app: &mut PhotocraftApp, name: &str, result: Result<Value, String>) -> Result<Value, String> {
    if result.is_ok() {
        let template = "Ran script {name}";
        app.ui.status = crate::i18n::fmt(tl!(template), &[("name", name)]);
    }
    result
}

/// `request`, starting in the folder of the most recent file when there is one.
fn in_last_dir(app: &PhotocraftApp, request: PickRequest) -> PickRequest {
    match last_used_dir(&app.ui.recent_files) {
        Some(dir) => request.directory(dir),
        None => request,
    }
}

// ---------- running ----------

/// Pump the run once a frame: commands and output, until it waits or ends.
pub(crate) fn tick(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.scripts.run.is_none() {
        return;
    }
    if held(app) {
        // Its next command waits, as a click would: the host waits for the answer meanwhile.
        ctx.request_repaint_after(WAITING_REPAINT);
        return;
    }
    // A run waiting at its own dialog sends nothing, but it may still end (its host was killed),
    // so it is pumped all the same. The run is taken out while it pumps, so its commands can go
    // through the app.
    let Some(mut active) = app.scripts.run.take() else { return };
    let (automation, gate) = (active.automation, active.gate);
    app.scripts.pumping = true;
    let pumped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        active.run.pump(&mut AppCommands { app: &mut *app, automation, gate, budget: &mut active.budget }, PUMP_BUDGET)
    }));
    app.scripts.pumping = false;
    let events = match pumped {
        Ok(events) => events,
        Err(panic) => {
            // The app's guard reports the panic; the run ends with it, and scripts can still
            // run afterwards.
            active.run.cancel();
            std::panic::resume_unwind(panic);
        }
    };
    // Nothing a command did may start another run meanwhile (`start` refuses while one pumps);
    // one that did anyway is stopped rather than lost under this one.
    if let Some(mut other) = app.scripts.run.replace(active) {
        other.run.cancel();
    }
    for event in events {
        match event {
            ScriptEvent::Output(line) => {
                app.scripts.log_line(&line);
                // The window opens at a run's first line; closed after that, it stays closed.
                if let Some(a) = app.scripts.run.as_mut()
                    && !std::mem::replace(&mut a.wrote, true)
                {
                    app.scripts.output_open = true;
                }
            }
            ScriptEvent::Dialog { kind, args } => open_dialog(app, kind, &args),
            ScriptEvent::Finished(result) => {
                finish(app, ctx, result);
                return;
            }
        }
    }
    if let Some(a) = app.scripts.run.as_ref() {
        // A busy run is pumped again soon; a waiting one sends nothing, but frames keep coming
        // to notice one whose host ended.
        ctx.request_repaint_after(if a.dialog.is_none() { BUSY_REPAINT } else { WAITING_REPAINT });
    }
}

/// How soon a busy run is pumped again.
const BUSY_REPAINT: Duration = Duration::from_millis(16);
/// How soon a waiting run is looked at again.
const WAITING_REPAINT: Duration = Duration::from_millis(250);

/// Whether the run's commands wait: the app is in a dialog other than the run's own, or another
/// modal state (the shortcut recorder, a full-window workspace, a pending crop), where a click
/// would run nothing either.
fn held(app: &PhotocraftApp) -> bool {
    let own = app.scripts.run.as_ref().is_some_and(|a| a.dialog.is_some());
    !own && (app.dialog_open()
        || crate::prefs_ui::shortcuts_open(app)
        || app.smart_svg.modal_open()
        || crate::distort_ui::modal_workspace(app).is_some()
        || crate::crop_ui::pending(app))
}

fn finish(app: &mut PhotocraftApp, ctx: &egui::Context, result: Result<(), String>) {
    let Some(active) = app.scripts.run.take() else { return };
    // A run that ended at its dialog takes the dialog with it.
    // A picker's late answer finds no run, and is nobody's.
    if let Some(Open { kind, id: Some(id), .. }) = active.dialog {
        app.framework.dialogs.cancel_by_id(ctx, id);
        // An alert's or a confirmation's result is kept by its instance, and nothing else takes
        // it; a prompt's goes to its family, which finds no run to answer.
        if kind != ScriptDialog::Prompt {
            let _ = app.framework.dialogs.take_by_id::<StandardPrompt>(id);
        }
    }
    match &result {
        Ok(()) => {
            let template = "Ran script {name}";
            app.ui.status = crate::i18n::fmt(tl!(template), &[("name", &active.name)]);
            app.ui.status_error = false;
        }
        Err(message) => {
            app.scripts.log_line(message);
            app.scripts.output_open = true;
            let first = message.lines().next().unwrap_or_default();
            let template = "Script {name}: {error}";
            app.ui.status = crate::i18n::fmt(tl!(template), &[("name", &active.name), ("error", first)]);
            app.ui.status_error = true;
        }
    }
}

/// Answer the dialog the run waits at, when `id` is that dialog.
fn answer(app: &mut PhotocraftApp, id: DialogId, value: Value) {
    reply(app, |open| open.id == Some(id), value);
}

/// Answer the dialog the run waits at, when `is` says it is the one answered.
fn reply(app: &mut PhotocraftApp, is: impl Fn(&Open) -> bool, value: Value) {
    let Some(a) = app.scripts.run.as_mut() else { return };
    if a.dialog.as_ref().is_none_or(|open| !is(open)) {
        return;
    }
    a.dialog = None;
    if let Err(e) = a.run.answer(value) {
        a.run.cancel();
        app.scripts.log_line(&e);
    }
}

// ---------- dialogs ----------

/// The parameter dialog of a prompt.
const PROMPT_KEY: &str = "script.prompt";

/// The family of the prompt's parameter dialog: OK answers its text, Cancel `null`.
pub(crate) const PROMPT_FAMILY: ParamFamily =
    ParamFamily { cancel: Some(prompt_cancelled), footer: Some(prompt_footer), ..ParamFamily::new(owns_prompt, prompt_answered) };

/// Every script dialog's way to end the run: a script that asks again and again can't keep the
/// app waiting at its dialogs, where the Stop window isn't shown.
const STOP_SCRIPT: &str = "Stop Script";
/// The prompt's footer action for [`STOP_SCRIPT`].
const STOP_ACTION: &str = "stopScript";

/// A prompt's Stop Script: the run ends, and its dialog closes without an answer.
fn prompt_footer(app: &mut PhotocraftApp, ctx: &egui::Context, id: DialogId, action: &str) {
    if action == STOP_ACTION {
        stop_at(app, id);
        app.framework.dialogs.cancel_by_id(ctx, id);
    }
}

/// End the run waiting at dialog `id`, without answering it.
fn stop_at(app: &mut PhotocraftApp, id: DialogId) {
    if let Some(a) = app.scripts.run.as_mut()
        && a.dialog.is_some_and(|open| open.id == Some(id))
    {
        a.dialog = None;
        a.run.cancel();
    }
}

fn owns_prompt(key: &str) -> bool {
    key == PROMPT_KEY
}

fn prompt_answered(app: &mut PhotocraftApp, _ctx: &egui::Context, result: ParamResult) {
    let text = result.fields.text("text").unwrap_or_default().to_owned();
    answer(app, result.id, Value::String(text));
}

fn prompt_cancelled(app: &mut PhotocraftApp, _ctx: &egui::Context, id: DialogId) {
    answer(app, id, Value::Null);
}

/// A script dialog's title: always that a script opened it, and which, so a script can't make
/// its dialog look like the app's own. The script's title, if it gave one, follows.
fn title(script: &str, title: &str) -> String {
    if title.is_empty() {
        let template = "Script: {name}";
        crate::i18n::fmt(tl!(template), &[("name", script)])
    } else {
        let template = "Script: {name} – {title}";
        crate::i18n::fmt(tl!(template), &[("name", script), ("title", title)])
    }
}

/// Open the dialog the run now waits at, as one of the app's dialogs. Its arguments are the
/// script's (see [`ScriptDialog`]).
fn open_dialog(app: &mut PhotocraftApp, kind: ScriptDialog, args: &Value) {
    let Some(name) = app.scripts.run.as_ref().map(|a| a.name.clone()) else { return };
    let arg = |i: usize| args.get(i).and_then(Value::as_str).unwrap_or_default().to_owned();
    app.scripts.dialogs += 1;
    let token = app.scripts.dialogs;
    if kind.picker() {
        if let Some(a) = app.scripts.run.as_mut() {
            a.dialog = Some(Open { kind, id: None, token });
        }
        return ask_picker(app, token, kind, args);
    }
    let instance = ("script-dialog", token);
    let id = match kind {
        ScriptDialog::Alert => {
            // Escape is OK, as before Stop Script was offered: stopping is asked for by name.
            let prompt = StandardPrompt::custom(arg(0))
                .title(title(&name, &arg(1)))
                .button(PromptChoice::Proceed, tl!("OK"))
                .button(PromptChoice::Alternate, tl!(STOP_SCRIPT))
                .alternate_role(DialogRole::Other)
                .escape(PromptChoice::Proceed);
            app.framework.dialogs.open(prompt.instance(instance))
        }
        ScriptDialog::Confirm => {
            let prompt = StandardPrompt::custom(arg(0))
                .title(title(&name, &arg(2)))
                .button(PromptChoice::Proceed, tl!("Yes"))
                .button(PromptChoice::Cancel, tl!("No"))
                .button(PromptChoice::Alternate, tl!(STOP_SCRIPT))
                // Apart from the answers, as on the prompt: it is no answer.
                .alternate_role(DialogRole::Other);
            // `noAsDefault`: Enter answers No.
            let prompt = if arg(1) == "true" { prompt.default_choice(PromptChoice::Cancel) } else { prompt };
            app.framework.dialogs.open(prompt.instance(instance))
        }
        ScriptDialog::Prompt => {
            let (message, text, heading) = (arg(0), arg(1), title(&name, &arg(2)));
            params::open_param_unsaved(app, PROMPT_KEY, DialogFields::new(), Value::Null, move |fields: &mut DialogFields| {
                let rows = vec![Param::new("text", message.clone(), ParamKind::text(text.clone()))];
                ParamDialog::new(PROMPT_KEY, heading.clone(), rows)
                    .values(Bind::bound(fields))
                    .extra_buttons(vec![(STOP_ACTION.to_owned(), FooterButton::new(tl!(STOP_SCRIPT).to_string(), DialogRole::Other))])
            })
        }
        ScriptDialog::OpenFile | ScriptDialog::SaveFile | ScriptDialog::SelectFolder => return,
    };
    if let Some(a) = app.scripts.run.as_mut() {
        a.dialog = Some(Open { kind, id: Some(id), token });
    }
}

/// The extensions a script's `filter` names (`"*.jpg;*.png"`, `"Images:*.jpg,*.tif"`), or `None`
/// for every file: no filter, one that names none, or one that allows any (`*.*`).
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

/// The file picker a script's `kind` of picker asks for, with its `filter` (Open and Save File)
/// as the file types offered: every file without one. The script's prompt is the picker's
/// title, where the platform shows one.
fn picker_request(kind: ScriptDialog, args: &Value) -> PickRequest {
    let multiple = args.get("multiple").and_then(Value::as_bool).unwrap_or(false);
    let request = match kind {
        ScriptDialog::OpenFile if multiple => PickRequest::open_many("script"),
        ScriptDialog::OpenFile => PickRequest::open("script"),
        ScriptDialog::SaveFile => PickRequest::save("script").file_name(tl!("Untitled")),
        _ => PickRequest::folder("script"),
    };
    let request = match args.get("prompt").and_then(Value::as_str).filter(|p| !p.is_empty()) {
        Some(prompt) => request.title(prompt),
        None => request,
    };
    if !matches!(kind, ScriptDialog::OpenFile | ScriptDialog::SaveFile) {
        return request;
    }
    match filter_extensions(args.get("filter").and_then(Value::as_str)) {
        Some(exts) => request.filter(tl!("Files"), &exts.iter().map(String::as_str).collect::<Vec<_>>()),
        None => request,
    }
}

/// Show the app's file dialog for picker `token`, and answer the run with what the user chose.
fn ask_picker(app: &mut PhotocraftApp, token: u64, kind: ScriptDialog, args: &Value) {
    let multiple = args.get("multiple").and_then(Value::as_bool).unwrap_or(false);
    let request = picker_request(kind, args);
    let request = if kind == ScriptDialog::SaveFile { request } else { in_last_dir(app, request) };
    let asked = app.ask_file_answer(request, move |app, chosen| {
        let text = |p: std::path::PathBuf| p.to_string_lossy().into_owned();
        let value = match chosen {
            Picked::Paths(paths) if multiple => json!(paths.into_iter().map(text).collect::<Vec<_>>()),
            Picked::Paths(paths) => json!(paths.into_iter().next().map(text)),
            // Save File's answer: the file chosen (on the web, its name).
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
        // The run was stopped meanwhile (and maybe another started): then the answer is nobody's.
        reply(app, |open| open.token == token, value);
        Ok(Value::Null)
    });
    if asked.is_err() {
        // No file dialog here, or another is open: the script sees a cancelled picker.
        reply(app, |open| open.token == token, Value::Null);
    }
}

/// The dialog the run waits at, if any.
pub(crate) fn waits_at(app: &PhotocraftApp) -> Option<DialogId> {
    app.scripts.run.as_ref().and_then(|a| a.dialog).and_then(|open| open.id)
}

/// A `TAKERS` row: the answer of an alert or a confirmation the run waits at (a prompt's comes
/// through [`PROMPT_FAMILY`]). A dialog closed from outside answers as Escape would.
pub(crate) fn take(app: &mut PhotocraftApp, _ctx: &egui::Context) {
    let Some(Open { kind, id: Some(id), .. }) = app.scripts.run.as_ref().and_then(|a| a.dialog) else { return };
    if kind == ScriptDialog::Prompt {
        return;
    }
    let yes = match app.framework.dialogs.take_by_id::<StandardPrompt>(id) {
        Some(DialogResult::Ok(answer)) if answer.choice == PromptChoice::Alternate => return stop_at(app, id),
        Some(DialogResult::Ok(answer)) => answer.choice == PromptChoice::Proceed,
        Some(_) => false,
        None if app.framework.dialogs.stack().ids().any(|d| d == id) => return,
        None => false,
    };
    let value = if kind == ScriptDialog::Confirm { Value::Bool(yes) } else { Value::Null };
    answer(app, id, value);
}

// ---------- drawing ----------

/// Draw what scripts wrote, and a way to stop a run that is busy without a dialog.
pub(crate) fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    output_window(app, ctx);
    if let Some(active) = app.scripts.run.as_mut()
        && active.dialog.is_none()
    {
        stop_window(ctx, active);
    }
}

/// The lines scripts wrote (`$.writeln`) and how their runs ended, newest last.
fn output_window(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.scripts.output_open {
        return;
    }
    let env = craft::ui::runtime::env::Env::of_root(ctx);
    let tokens = env.tokens.tokens();
    let mut open = true;
    let mut clear = false;
    egui::Window::new(tl!("Script Output")).id(Id::unique("script-output")).open(&mut open).default_width(tokens.size.popup_min * 2.0).show(ctx, |ui| {
        let row = ui.text_style_height(&egui::TextStyle::Body);
        // Only the rows in view are laid out: the log holds up to `LOG_LINES` lines.
        egui::ScrollArea::both().stick_to_bottom(true).auto_shrink([false, true]).max_height(row * 16.0).show_rows(
            ui,
            row,
            app.scripts.log.len(),
            |ui, rows| {
                for line in app.scripts.log.range(rows) {
                    // One row a line, scrolled sideways when it is long.
                    ui.add(egui::Label::new(line).extend());
                }
            },
        );
        ui.separator();
        clear = ui.button(tl!("Clear")).clicked();
    });
    if clear {
        app.scripts.log.clear();
    }
    app.scripts.output_open = open;
}

/// A small window to stop a run that is busy without a dialog.
fn stop_window(ctx: &egui::Context, active: &mut Active) {
    if crate::gpu_canvas::now_ms() - active.started_ms < STOP_AFTER_MS {
        return;
    }
    let env = craft::ui::runtime::env::Env::of_root(ctx);
    let tokens = env.tokens.tokens();
    let inset = tokens.space.inset.screen;
    let mut stop = false;
    egui::Window::new(tl!("Script"))
        .id(Id::unique("script-running"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-inset, -(inset + tokens.size.bar.status)))
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

#[cfg(test)]
mod tests;
