//! The native Estimate Depth flow: estimate at once when the tool is ready, else walk the person
//! through the shared setup. See the parent module.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

use craft_ai_client::{
    Adopted, CheckedNative, FolderDesk, MissingModel, ModelDownloader, NativeSetupPlan, SetupLink, SetupProgress, SetupSession, SetupTicket, ToolDesk,
};
use craft_ai_core::AiError;
use craft_ai_egui::{NativeSetupAction, PopupAction, missing_model_popup, native_model_setup};
use craft_ai_features::{Feature, builtin};
use photocraft_doc::DocId;
use photocraft_engine::depth_cmds::{AiHost, TOOL};
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// What the setup window is doing, as `ui.inspect` reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Reading what the tool needs.
    Loading,
    /// Showing the missing-model popup.
    Missing,
    /// Building the setup plan.
    Planning,
    /// Showing the setup screen (the person reviews, then downloads or checks files).
    Setup,
    /// The setup could not continue; the message is shown.
    Failed,
}

impl Phase {
    fn name(self) -> &'static str {
        match self {
            Phase::Loading => "loading",
            Phase::Missing => "missing",
            Phase::Planning => "planning",
            Phase::Setup => "setup",
            Phase::Failed => "failed",
        }
    }
}

/// A folder the person picked, and what it is for.
enum Folder {
    /// Nothing changes; the plan is just read again.
    Keep,
    /// The folder the suite suggests becomes the download folder.
    Suggested,
    /// The download folder.
    Destination(PathBuf),
    /// A folder that already holds model files, linked for discovery.
    Existing(PathBuf),
}

/// An answer from a worker thread.
enum Msg {
    Popup(Result<Option<MissingModel>, String>),
    Plan(Result<NativeSetupPlan, String>),
    Progress(SetupTicket, usize, SetupProgress),
    Checked(SetupTicket, Result<CheckedNative, AiError>),
    Chosen(Result<(), String>),
    Adopted(Result<Adopted, String>),
    /// A worker ended without an answer.
    Failed(String),
}

enum Stage {
    Loading,
    Missing(Box<MissingModel>),
    Planning,
    Setup(Box<SetupSession>),
    Failed,
}

/// What one Estimate Depth asks for: the channel to write and the quality, fixed when it is chosen.
#[derive(Clone, Debug, PartialEq)]
struct Ask {
    channel: String,
    quality: &'static str,
}

impl Ask {
    /// The brush's current Estimate Depth settings.
    fn now(app: &PhotocraftApp) -> Self {
        let surface = &app.session.tools.brush.surface;
        Self { channel: surface.depth_channel.clone(), quality: if surface.depth_high_quality { "high" } else { "fast" } }
    }
}

/// The setup window. Bound to the document that started the estimate.
struct Window {
    /// Which setup this is. A new one is made by [`start`] and by Back, so an answer (a picked
    /// folder) meant for an earlier setup is told apart from this one's.
    id: u64,
    doc: DocId,
    /// The channel and quality asked for when Estimate Depth was chosen, used when setup ends.
    ask: Ask,
    host: AiHost,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    stage: Stage,
    /// A worker is running that the window waits for (not the download, which the session tracks).
    working: bool,
    /// The first frame starts the first worker (it needs the egui context).
    started: bool,
    /// The provider of the setup option the person took (`native:<tool>:<provider>`).
    provider: Option<String>,
    error: Option<String>,
}

impl Window {
    fn phase(&self) -> Phase {
        match self.stage {
            Stage::Loading => Phase::Loading,
            Stage::Missing(_) => Phase::Missing,
            Stage::Planning => Phase::Planning,
            Stage::Setup(_) => Phase::Setup,
            Stage::Failed => Phase::Failed,
        }
    }
}

/// What the app keeps of Estimate Depth's setup. Not serde: it holds workers and shared-layer
/// state; `ui.inspect` shows [`inspect`].
#[derive(Default)]
pub struct DepthAi {
    window: Option<Window>,
    /// The id the next setup gets.
    next_id: u64,
    /// A folder the person picked, waiting for the next frame to start its plan worker (the file
    /// dialog's answer arrives without an egui context).
    pending: Option<Folder>,
}

fn message(e: AiError) -> String {
    e.message
}

fn feature() -> Result<Feature, String> {
    builtin().into_iter().find(|f| f.id == TOOL).ok_or_else(|| "Estimate Depth is not part of the AI tools yet".to_owned())
}

/// Run `work` on a worker thread and send its message back, waking the window. A worker that
/// panics or cannot start answers [`Msg::Failed`].
fn spawn(ctx: &egui::Context, tx: &Sender<Msg>, work: impl FnOnce() -> Msg + Send + 'static) -> Result<(), String> {
    let (ctx, tx) = (ctx.clone(), tx.clone());
    std::thread::Builder::new()
        .name("photocraft-depth-setup".into())
        .spawn(move || {
            let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
                .unwrap_or_else(|_| Msg::Failed("Setting up Estimate Depth stopped unexpectedly".into()));
            // Nothing is waiting any more when the window was closed meanwhile.
            let _ = tx.send(answer);
            ctx.request_repaint();
        })
        .map(|_| ())
        .map_err(|e| format!("could not start the setup: {e}"))
}

fn load_popup(host: &AiHost) -> Result<Option<MissingModel>, String> {
    let desk = ToolDesk::new(host.client()?, host.shared().clone(), None);
    let role = feature()?.role().ok_or("the tool id is not valid")?;
    desk.missing_model(&role, &builtin()).map_err(message)
}

fn prepare(host: &AiHost, provider: &str, folder: Folder) -> Result<NativeSetupPlan, String> {
    let client = host.client()?;
    let desk = FolderDesk::new(host.shared().clone());
    match folder {
        Folder::Keep => {}
        Folder::Suggested => {
            desk.ensure_default().map_err(message)?;
        }
        Folder::Destination(path) => {
            let added = desk.add_new(&path, None).map_err(message)?;
            desk.set_default(&added.folder.id).map_err(message)?;
        }
        Folder::Existing(path) => {
            desk.add(&path, None).map_err(message)?;
        }
    }
    desk.prepare_native_setup(&feature()?, provider, &client).map_err(message)
}

/// The provider named by a setup option id `native:<tool>:<provider>`.
fn native_provider(option: &str) -> Option<String> {
    let mut parts = option.splitn(3, ':');
    (parts.next() == Some("native") && parts.next() == Some(TOOL)).then(|| parts.next().map(str::to_owned)).flatten()
}

/// The document the app shows, if any.
fn active_doc(app: &PhotocraftApp) -> Option<DocId> {
    app.session.active().map(|d| d.doc.id)
}

/// Why the Estimate Depth button is off, for its tooltip, or `None` while it can be used.
pub fn unavailable_reason(app: &PhotocraftApp) -> Option<String> {
    app.session.depth.ai.status_reason()
}

/// A small summary for `ui.inspect`: `null` while no setup window is open.
pub fn inspect(app: &PhotocraftApp) -> Value {
    match &app.depth_ai.window {
        None => Value::Null,
        Some(w) => json!({"phase": w.phase().name(), "busy": w.working || matches!(&w.stage, Stage::Setup(s) if s.busy()), "error": w.error}),
    }
}

/// Run `image.estimateDepth` as `ask` says, as a background job when jobs are on.
fn estimate(app: &mut PhotocraftApp, ask: &Ask) {
    if let Err(e) = app.run("image.estimateDepth", json!({"channel": ask.channel, "quality": ask.quality})) {
        match app.depth_ai.window.as_mut() {
            Some(w) => {
                w.error = Some(e);
                w.stage = Stage::Failed;
            }
            None => {
                app.ui.status = e;
                app.ui.status_error = true;
            }
        }
    }
}

/// The Estimate Depth… button: estimate at once when the tool is ready, else open the setup window.
pub fn start(app: &mut PhotocraftApp) {
    let Some(doc) = active_doc(app) else {
        app.ui.status = tl!("No document open").into();
        app.ui.status_error = true;
        return;
    };
    // A window already open keeps its setup (its download, its choices): replacing it would drop
    // a running download without stopping it. A failed one has nothing running and is replaced.
    if let Some(w) = &app.depth_ai.window {
        if !matches!(w.stage, Stage::Failed) {
            return;
        }
        cancel(app);
    }
    let ask = Ask::now(app);
    if app.session.depth.ai.status_reason_now().is_none() {
        estimate(app, &ask);
        return;
    }
    let host = match app.session.depth.ai.host() {
        Ok(h) => h,
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
            return;
        }
    };
    let (tx, rx) = mpsc::channel();
    let id = new_setup_id(app);
    app.depth_ai.window = Some(Window { id, doc, ask, host, tx, rx, stage: Stage::Loading, working: false, started: false, provider: None, error: None });
}

/// A fresh setup identity.
fn new_setup_id(app: &mut PhotocraftApp) -> u64 {
    app.depth_ai.next_id = app.depth_ai.next_id.wrapping_add(1);
    app.depth_ai.next_id
}

/// Close the window when the document it was started for is no longer open: its setup could only
/// end in an estimate for a document that is gone.
pub fn cancel_stale(app: &mut PhotocraftApp) {
    let gone = app.depth_ai.window.as_ref().is_some_and(|w| !app.session.documents().iter().any(|d| d.doc.id == w.doc));
    if gone {
        cancel(app);
    }
}

/// Close the window and stop its download. A folder picked for it and not yet used is dropped.
pub fn cancel(app: &mut PhotocraftApp) {
    app.depth_ai.pending = None;
    if let Some(mut w) = app.depth_ai.window.take()
        && let Stage::Setup(session) = &mut w.stage
    {
        session.cancel();
    }
}

/// Hand a worker's answer to the window. Returns what to estimate when it should run now.
fn apply(app: &mut PhotocraftApp, msg: Msg) -> Option<Ask> {
    let current = active_doc(app);
    let w = app.depth_ai.window.as_mut()?;
    match msg {
        Msg::Failed(e) => {
            w.working = false;
            w.error = Some(e);
            if !matches!(w.stage, Stage::Setup(_)) {
                w.stage = Stage::Failed;
            }
        }
        Msg::Popup(r) => {
            w.working = false;
            match r {
                // The tool became ready meanwhile (another app set it up).
                Ok(None) => {
                    app.session.depth.ai.forget_status();
                    return finish(app, current);
                }
                Ok(Some(p)) => w.stage = Stage::Missing(Box::new(p)),
                Err(e) => {
                    w.error = Some(e);
                    w.stage = Stage::Failed;
                }
            }
        }
        Msg::Plan(r) => {
            w.working = false;
            match (r, &mut w.stage) {
                (Ok(plan), Stage::Setup(session)) => {
                    // A plan for the same target only refreshes what is shown.
                    session.review(plan);
                    w.error = None;
                }
                (Ok(plan), stage) => {
                    *stage = Stage::Setup(Box::new(SetupSession::new(plan)));
                    w.error = None;
                }
                (Err(e), stage) => {
                    w.error = Some(e);
                    if !matches!(stage, Stage::Setup(_)) {
                        *stage = Stage::Failed;
                    }
                }
            }
        }
        Msg::Progress(ticket, file, progress) => {
            if let Stage::Setup(session) = &mut w.stage {
                session.report_progress(ticket, file, progress);
            }
        }
        Msg::Checked(ticket, r) => {
            if let Stage::Setup(session) = &mut w.stage {
                session.complete(ticket, r);
            }
        }
        Msg::Chosen(r) => {
            w.working = false;
            match r {
                Ok(()) => {
                    app.session.depth.ai.forget_status();
                    return finish(app, current);
                }
                Err(e) => w.error = Some(e),
            }
        }
        Msg::Adopted(r) => {
            w.working = false;
            match r {
                Ok(adopted) => {
                    app.session.depth.ai.forget_status();
                    if let Some((connection, model)) = adopted.kept.filter(|_| !adopted.bound) {
                        w.error = Some(format!(
                            "Another model ({} / {}) is already chosen for Estimate Depth, so the new files were not switched on. Choose it in Set up AI in the ArtCraft Launcher.",
                            connection.as_str(),
                            model.as_str()
                        ));
                        return None;
                    }
                    return finish(app, current);
                }
                Err(e) => w.error = Some(e),
            }
        }
    }
    None
}

/// Setup is done: estimate when the person is still on the document that asked, else say so and
/// leave nothing running.
fn finish(app: &mut PhotocraftApp, current: Option<DocId>) -> Option<Ask> {
    let w = app.depth_ai.window.as_mut()?;
    if let Some(reason) = app.session.depth.ai.status_reason_now() {
        w.error = Some(reason);
        w.stage = Stage::Failed;
        return None;
    }
    if current != Some(w.doc) {
        w.error = Some(
            tl!("Estimate Depth is set up. The document it was started for is no longer the active one: switch back to it and choose Estimate Depth again.")
                .into(),
        );
        w.stage = Stage::Failed;
        return None;
    }
    app.depth_ai.window.take().map(|w| w.ask)
}

/// Ask for a folder, then rebuild the plan with it. `existing` is true for a folder that already
/// holds model files (linked for discovery), false for the folder downloads go to.
fn pick_folder(app: &mut PhotocraftApp, existing: bool) {
    // The file dialog answers later: only the setup that asked takes the answer.
    let Some(id) = app.depth_ai.window.as_ref().map(|w| w.id) else { return };
    let asked = app.ask_folder(move |app, path| {
        folder_answered(app, id, existing, path);
        Ok(Value::Null)
    });
    if let (Err(e), Some(w)) = (asked, app.depth_ai.window.as_mut()) {
        w.error = Some(e);
    }
}

/// A folder was picked for the setup `id`. It is kept only while that setup is the open one; after
/// Back or a close (and a new start) it belongs to nobody.
fn folder_answered(app: &mut PhotocraftApp, id: u64, existing: bool, path: String) {
    if app.depth_ai.window.as_ref().is_some_and(|w| w.id == id) {
        let path = PathBuf::from(path);
        app.depth_ai.pending = Some(if existing { Folder::Existing(path) } else { Folder::Destination(path) });
    }
}

/// Draw the window and step it: take workers' answers, draw the current screen, act on it.
pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.depth_ai.window.is_none() {
        return;
    }
    // Start the first worker.
    if let Some(w) = app.depth_ai.window.as_mut()
        && !w.started
    {
        w.started = true;
        w.working = true;
        let host = w.host.clone();
        if let Err(e) = spawn(ctx, &w.tx, move || Msg::Popup(load_popup(&host))) {
            w.working = false;
            w.error = Some(e);
            w.stage = Stage::Failed;
        }
    }
    // A folder picked on an earlier frame starts its plan worker now.
    if let Some(folder) = app.depth_ai.pending.take()
        && let Some(w) = app.depth_ai.window.as_mut()
        && let Some(provider) = w.provider.clone()
    {
        let host = w.host.clone();
        w.working = true;
        w.error = None;
        if let Err(e) = spawn(ctx, &w.tx, move || Msg::Plan(prepare(&host, &provider, folder))) {
            w.working = false;
            w.error = Some(e);
        }
    }
    // Take answers.
    let mut run_estimate = None;
    while let Some(w) = app.depth_ai.window.as_ref() {
        match w.rx.try_recv() {
            Ok(msg) => {
                if let Some(ask) = apply(app, msg) {
                    run_estimate = Some(ask);
                    break;
                }
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
        }
    }
    if let Some(ask) = run_estimate {
        estimate(app, &ask);
        return;
    }
    let Some(w) = app.depth_ai.window.as_mut() else { return };
    if w.working || matches!(&w.stage, Stage::Setup(s) if s.busy()) {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }

    /// What the person asked for this frame.
    enum Act {
        None,
        Close,
        Choose(craft_ai_core::ConnectionId, craft_ai_core::ModelId),
        SetUp(SetupLink),
        Setup(Box<NativeSetupAction>),
        Back,
    }
    let mut act = Act::None;
    let mut open = true;
    egui::Window::new(tl!("Estimate Depth")).open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO).show(
        ctx,
        |ui| {
            ui.set_max_width(420.0);
            match &mut w.stage {
                Stage::Loading | Stage::Planning => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(tl!("Working…"));
                    });
                }
                Stage::Missing(popup) => {
                    ui.add_enabled_ui(!w.working, |ui| match missing_model_popup(ui, popup) {
                        Some(PopupAction::Choose { connection, model }) => act = Act::Choose(connection, model),
                        Some(PopupAction::SetUp(link)) => act = Act::SetUp(link),
                        Some(PopupAction::NotNow) => act = Act::Close,
                        // The install offer is not drawn here.
                        Some(PopupAction::InstallComfyUi(_)) | None => {}
                    });
                }
                Stage::Setup(session) => {
                    ui.add_enabled_ui(!w.working, |ui| {
                        if let Some(action) = native_model_setup(ui, session) {
                            act = match action {
                                NativeSetupAction::Back => Act::Back,
                                other => Act::Setup(Box::new(other)),
                            };
                        }
                    });
                }
                Stage::Failed => {
                    if ui.button(tl!("Close")).clicked() {
                        act = Act::Close;
                    }
                }
            }
            if let Some(e) = &w.error {
                ui.add_space(6.0);
                ui.colored_label(crate::theme::Tokens::get(ui.ctx()).danger, e);
            }
        },
    );
    if !open {
        act = Act::Close;
    }
    match act {
        Act::None => {}
        Act::Close => cancel(app),
        Act::Back => go_back(app),
        Act::Choose(connection, model) => {
            if let Some(w) = app.depth_ai.window.as_mut() {
                w.working = true;
                w.error = None;
                let host = w.host.clone();
                let started = spawn(ctx, &w.tx, move || {
                    let run = || -> Result<(), String> {
                        let desk = ToolDesk::new(host.client()?, host.shared().clone(), None);
                        let role = feature()?.role().ok_or("the tool id is not valid")?;
                        desk.choose(&role, &connection, &model).map_err(message)
                    };
                    Msg::Chosen(run())
                });
                if let Err(e) = started {
                    w.working = false;
                    w.error = Some(e);
                }
            }
        }
        Act::SetUp(link) => {
            let provider = match &link {
                SetupLink::New { option } => native_provider(option),
                SetupLink::Fix { .. } => None,
            };
            if let Some(w) = app.depth_ai.window.as_mut() {
                match provider {
                    Some(provider) => {
                        w.provider = Some(provider.clone());
                        w.stage = Stage::Planning;
                        w.working = true;
                        w.error = None;
                        let host = w.host.clone();
                        if let Err(e) = spawn(ctx, &w.tx, move || Msg::Plan(prepare(&host, &provider, Folder::Keep))) {
                            w.working = false;
                            w.error = Some(e);
                            w.stage = Stage::Failed;
                        }
                    }
                    None => w.error = Some(tl!("Open Set up AI in the ArtCraft Launcher to set this up.").into()),
                }
            }
        }
        Act::Setup(action) => setup_action(app, ctx, *action),
    }
}

/// Back from the setup screen to the first one: stops the download and starts reading again. The
/// answers channel is replaced, so a worker still running from before (a plan, a choice, an
/// adoption) sends into a channel nobody reads and cannot move this window to a stale screen.
fn go_back(app: &mut PhotocraftApp) {
    // It restarts the setup: a folder picked for the old one is not this one's.
    app.depth_ai.pending = None;
    let id = new_setup_id(app);
    if let Some(w) = app.depth_ai.window.as_mut() {
        w.id = id;
        if let Stage::Setup(session) = &mut w.stage {
            session.cancel();
        }
        let (tx, rx) = mpsc::channel();
        w.tx = tx;
        w.rx = rx;
        w.working = false;
        w.stage = Stage::Loading;
        w.started = false;
        w.error = None;
    }
}

/// Carry out what the setup screen asked for.
fn setup_action(app: &mut PhotocraftApp, ctx: &egui::Context, action: NativeSetupAction) {
    match action {
        NativeSetupAction::Back | NativeSetupAction::Cancel(_) => {}
        NativeSetupAction::FindFiles => pick_folder(app, true),
        NativeSetupAction::ChangeFolder => pick_folder(app, false),
        NativeSetupAction::UseSuggestedFolder => {
            let Some(w) = app.depth_ai.window.as_mut() else { return };
            let Some(provider) = w.provider.clone() else { return };
            w.working = true;
            w.error = None;
            let host = w.host.clone();
            if let Err(e) = spawn(ctx, &w.tx, move || Msg::Plan(prepare(&host, &provider, Folder::Suggested))) {
                w.working = false;
                w.error = Some(e);
            }
        }
        NativeSetupAction::Work(request) => {
            let Some(w) = app.depth_ai.window.as_mut() else { return };
            w.error = None;
            let (tx, repaint) = (w.tx.clone(), ctx.clone());
            let ticket = request.ticket();
            let started = spawn(ctx, &w.tx, move || {
                let downloader = ModelDownloader::new(Default::default());
                let progress_tx = tx.clone();
                let result = request.run(&downloader, None, &mut |file, progress| {
                    // The window may be gone; the download then ends at its cancel flag.
                    let _ = progress_tx.send(Msg::Progress(ticket, file, progress));
                    repaint.request_repaint();
                });
                Msg::Checked(ticket, result)
            });
            if let Err(e) = started {
                w.error = Some(e);
                if let Stage::Setup(session) = &mut w.stage {
                    session.cancel();
                }
            }
        }
        NativeSetupAction::Use(checked) => {
            let Some(w) = app.depth_ai.window.as_mut() else { return };
            w.working = true;
            w.error = None;
            let host = w.host.clone();
            let started = spawn(ctx, &w.tx, move || {
                let run = || -> Result<Adopted, String> {
                    let desk = ToolDesk::new(host.client()?, host.shared().clone(), None);
                    desk.adopt(&checked).map_err(message)
                };
                Msg::Adopted(run())
            });
            if let Err(e) = started {
                w.working = false;
                w.error = Some(e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    /// A host over an empty temporary AI store: nothing is set up, and no test reads a person's settings.
    fn empty_host(dir: &std::path::Path) -> AiHost {
        let store = craft_ai_native::shared::SharedStore::new(dir.join("connections.json"));
        let shared = store.clone();
        AiHost::new(store, move || {
            let app = craft_ai_core::AppId::new("photocraft").map_err(|e| e.to_string())?;
            Ok(craft_ai_client::AiClient::new(app, shared.clone(), Arc::new(craft_ai_native::secrets::MemoryStore::new())))
        })
    }

    fn app(dir: &std::path::Path) -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 32, "height": 24})).unwrap();
        app.session.depth.ai.set_host(empty_host(dir));
        app
    }

    #[test]
    fn setup_finishing_on_another_document_does_not_estimate() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        let started_on = active_doc(&app).unwrap();
        start(&mut app);
        assert_eq!(app.depth_ai.window.as_ref().map(|w| w.doc), Some(started_on));
        // The person switches to another document while the setup runs.
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        assert_ne!(active_doc(&app), Some(started_on));
        // The tool is ready by the time the setup ends.
        app.session.depth.ai.estimator = Some(Arc::new(|_, _, _, _| Err("not run".into())));
        assert!(apply(&mut app, Msg::Chosen(Ok(()))).is_none());
        assert_eq!(inspect(&app)["phase"], "failed");
        let error = inspect(&app)["error"].as_str().unwrap_or_default().to_owned();
        assert!(error.contains("no longer the active one"), "{error}");
        assert!(app.session.active().unwrap().doc.channels.is_empty());
        // On the document that asked, setup finishing runs the estimate and closes the window.
        let current = active_doc(&app).unwrap();
        if let Some(w) = app.depth_ai.window.as_mut() {
            w.doc = current;
            w.stage = Stage::Loading;
        }
        assert!(apply(&mut app, Msg::Chosen(Ok(()))).is_some());
        assert!(app.depth_ai.window.is_none());
    }

    /// A second Estimate Depth while a download runs keeps the window and its download.
    #[test]
    fn a_second_start_keeps_the_running_download() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        start(&mut app);
        let host = app.depth_ai.window.as_ref().map(|w| w.host.clone()).unwrap();
        let plan = prepare(&host, "candle", Folder::Destination(dir.path().join("models"))).expect("a setup plan");
        let mut session = SetupSession::new(plan);
        session.accept_licences(true);
        let _request = session.start().expect("the download starts");
        assert!(session.busy());
        if let Some(w) = app.depth_ai.window.as_mut() {
            w.stage = Stage::Setup(Box::new(session));
            w.provider = Some("candle".into());
        }
        let started_on = active_doc(&app);
        // Another document, and the button again.
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        start(&mut app);
        let w = app.depth_ai.window.as_ref().expect("the window is kept");
        assert_eq!(Some(w.doc), started_on, "still bound to the document that asked");
        assert_eq!(w.provider.as_deref(), Some("candle"));
        assert!(matches!(&w.stage, Stage::Setup(s) if s.busy()), "the download was not dropped");
        // A failed window has nothing running and is replaced.
        if let Some(w) = app.depth_ai.window.as_mut() {
            w.stage = Stage::Failed;
        }
        start(&mut app);
        assert_eq!(inspect(&app)["phase"], "loading");
    }

    /// An answer from a worker started before Back cannot reach the window.
    #[test]
    fn an_answer_from_before_back_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        start(&mut app);
        let stale = app.depth_ai.window.as_ref().map(|w| w.tx.clone()).unwrap();
        if let Some(w) = app.depth_ai.window.as_mut() {
            w.stage = Stage::Planning;
            w.working = true;
        }
        go_back(&mut app);
        let w = app.depth_ai.window.as_ref().unwrap();
        assert!(!w.working);
        assert_eq!(inspect(&app)["phase"], "loading");
        // The stale worker's answer goes nowhere, and nothing is waiting in the window's channel.
        assert!(stale.send(Msg::Failed("stale".into())).is_err());
        assert!(matches!(app.depth_ai.window.as_ref().unwrap().rx.try_recv(), Err(TryRecvError::Empty)));
    }

    /// Closing the document the setup was started for closes the window (and stops its download).
    #[test]
    fn closing_the_document_closes_the_setup_window() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        start(&mut app);
        let started_on = app.depth_ai.window.as_ref().map(|w| w.doc).unwrap();
        // The window's document is still open: nothing is cancelled.
        cancel_stale(&mut app);
        assert!(app.depth_ai.window.is_some());
        // Closed behind the app's back, so the explicit cleanup is what is tested.
        app.session.execute("file.close", json!({})).unwrap();
        assert!(!app.session.documents().iter().any(|d| d.doc.id == started_on));
        cancel_stale(&mut app);
        assert!(app.depth_ai.window.is_none());
    }

    /// A folder answered after Back, or after the window was closed and another one opened, is not
    /// taken by the new setup.
    #[test]
    fn a_folder_answer_for_an_earlier_setup_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        start(&mut app);
        let asked = app.depth_ai.window.as_ref().map(|w| w.id).unwrap();
        // The answer for the setup that asked is kept.
        folder_answered(&mut app, asked, false, "/models".into());
        assert!(matches!(&app.depth_ai.pending, Some(Folder::Destination(p)) if p == &PathBuf::from("/models")));
        app.depth_ai.pending = None;
        // After Back, the setup is a new one.
        go_back(&mut app);
        assert_ne!(app.depth_ai.window.as_ref().map(|w| w.id), Some(asked));
        folder_answered(&mut app, asked, true, "/models".into());
        assert!(app.depth_ai.pending.is_none());
        // After a close and a new start too, and a pick waiting at the close is dropped.
        let current = app.depth_ai.window.as_ref().map(|w| w.id).unwrap();
        folder_answered(&mut app, current, true, "/found".into());
        assert!(matches!(&app.depth_ai.pending, Some(Folder::Existing(_))));
        cancel(&mut app);
        assert!(app.depth_ai.pending.is_none());
        start(&mut app);
        assert_ne!(app.depth_ai.window.as_ref().map(|w| w.id), Some(current));
        folder_answered(&mut app, current, false, "/models".into());
        assert!(app.depth_ai.pending.is_none());
    }

    #[test]
    fn setup_that_leaves_the_tool_unavailable_reports_why() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        start(&mut app);
        assert!(apply(&mut app, Msg::Chosen(Ok(()))).is_none());
        assert_eq!(inspect(&app)["phase"], "failed");
        assert!(inspect(&app)["error"].is_string());
    }

    #[test]
    fn a_failed_worker_is_shown_and_stops_the_wait() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        start(&mut app);
        if let Some(w) = app.depth_ai.window.as_mut() {
            w.working = true;
        }
        assert!(apply(&mut app, Msg::Failed("boom".into())).is_none());
        assert_eq!(inspect(&app), json!({"phase": "failed", "busy": false, "error": "boom"}));
    }

    /// The model files are in place, by download or by files found on disk: both end in `adopt`.
    fn adopted() -> Msg {
        let connection = craft_ai_core::ConnectionId::new("native-estimate-depth-candle").unwrap();
        let model = craft_ai_core::ModelId::new("depth-anything-v2-small").unwrap();
        Msg::Adopted(Ok(craft_ai_client::Adopted { connection, model, bound: true, kept: None }))
    }

    /// Download completion and local-file completion on another document: nothing is estimated,
    /// and neither document gains a channel.
    #[test]
    fn model_files_arriving_while_another_document_is_active_estimate_nothing() {
        for arrival in ["download", "local files"] {
            let dir = tempfile::tempdir().unwrap();
            let mut app = app(dir.path());
            let started_on = active_doc(&app).unwrap();
            start(&mut app);
            app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
            app.session.depth.ai.estimator = Some(Arc::new(|_, _, _, _| Err("not run".into())));
            assert!(apply(&mut app, adopted()).is_none(), "{arrival}");
            assert_eq!(inspect(&app)["phase"], "failed", "{arrival}");
            assert!(app.session.active().unwrap().doc.channels.is_empty(), "{arrival}");
            // The document that asked was not touched either.
            app.run("document.activate", json!({"document": 0})).unwrap();
            assert_eq!(active_doc(&app), Some(started_on), "{arrival}");
            assert!(app.session.active().unwrap().doc.channels.is_empty(), "{arrival}");
        }
    }

    /// The channel and quality are the ones asked for when Estimate Depth was chosen, even if the
    /// brush settings change while the model files arrive.
    #[test]
    fn the_estimate_keeps_the_channel_and_quality_it_was_asked_with() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        app.session.tools.brush.surface.depth_channel = "Near".into();
        app.session.tools.brush.surface.depth_high_quality = false;
        start(&mut app);
        app.session.tools.brush.surface.depth_channel = "Other".into();
        app.session.tools.brush.surface.depth_high_quality = true;
        app.session.depth.ai.estimator = Some(Arc::new(|_, _, _, _| Err("not run".into())));
        let ask = apply(&mut app, adopted()).expect("the same document estimates");
        assert_eq!(ask, Ask { channel: "Near".into(), quality: "fast" });
        assert!(app.depth_ai.window.is_none());
    }

    #[test]
    fn the_native_provider_is_read_from_the_setup_option() {
        assert_eq!(native_provider("native:estimate-depth:candle").as_deref(), Some("candle"));
        assert_eq!(native_provider("native:select-subject:candle"), None);
        assert_eq!(native_provider("local:estimate-depth:x"), None);
        assert_eq!(native_provider("service:openai"), None);
        assert_eq!(native_provider("native:estimate-depth"), None);
    }
}
