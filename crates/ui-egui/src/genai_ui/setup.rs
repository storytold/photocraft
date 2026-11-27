//! Set Up Generative AI: PhotoCraft's small setup over the Craft suite's shared AI layer, until the
//! Craft launcher has its own Set up AI window. It shows whether Generative Fill can run (the
//! layer's reason as it is when it can't), lets the person pick the model it uses (the suite's
//! choice, as the launcher's), lists the saved connections, and connects OpenAI, Black Forest Labs
//! or Google Gemini with a key, or a ComfyUI server by its address.
//!
//! Connections, keys and the choice are the shared layer's, not PhotoCraft's preferences. The
//! dialog draws from a [`Shared`] state the app fills before each pass ([`feed`]) and reads its
//! requests back from: a key is typed into that state, never into the dialog's fields (so
//! `ui.inspect` and the control channel never see it), handed to the layer once, and kept in the
//! OS keychain by it.

use std::sync::{Arc, Mutex};

use craft::ui::contract::Bind;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamDialog};
use craft::ui::runtime::env::Label;
use photocraft_engine::genai::{AiFailure, AiStatus, ConnectionProblem, ServiceKind, SetupView};
use serde_json::Value;

use crate::PhotocraftApp;
use crate::dialogs::params;

/// The setup dialog's key.
pub const KEY: &str = "genai.setup";

/// How long a read of the shared settings is shown before it is read again, in seconds.
const REFRESH_S: f64 = 10.0;

/// What the setup's rows ask the app for.
#[derive(Clone, Debug, PartialEq)]
enum Request {
    /// Use this connection's model for Generative Fill.
    Choose { connection: String, model: String },
    /// Forget this connection.
    Remove(String),
    /// Check and save a connection to this service, with its key or address.
    Connect(ServiceKind, String),
}

/// What the dialog draws and what it asks, shared between the app and the dialog's rows.
#[derive(Default)]
pub struct View {
    /// The tool's status lines.
    status: Vec<String>,
    /// The shared settings as last read; `None` before the first read answers.
    setup: Option<Result<SetupView, String>>,
    kind: Option<ServiceKind>,
    /// What is typed in the key field. Cleared once it has been handed over.
    key: String,
    address: String,
    /// A connection is being checked.
    busy: bool,
    /// The last outcome to show: (text, is an error).
    message: Option<(String, bool)>,
    requests: Vec<Request>,
}

/// The setup's [`View`], shared with the dialog's rows.
pub type Shared = Arc<Mutex<View>>;

/// What the app keeps for the setup between frames.
#[derive(Default)]
pub struct State {
    shared: Shared,
    /// When the shared settings were last read, and the tool's status then.
    read_at: Option<(f64, AiStatus)>,
    /// The last read is out of date (something here changed the settings): read them again.
    stale: bool,
    /// A connection being checked and saved off the interface thread.
    #[cfg(not(target_arch = "wasm32"))]
    busy: Option<std::sync::mpsc::Receiver<Result<(), AiFailure>>>,
    /// The settings being read off the interface thread (they include the keychain), with the
    /// tool's status when the read started.
    #[cfg(not(target_arch = "wasm32"))]
    reading: Option<(AiStatus, std::sync::mpsc::Receiver<Result<SetupView, String>>)>,
}

/// A service's name as setup lists it.
fn kind_name(k: ServiceKind) -> &'static str {
    match k {
        ServiceKind::OpenAi => "OpenAI",
        ServiceKind::BlackForestLabs => "Black Forest Labs",
        ServiceKind::Gemini => "Google Gemini",
        ServiceKind::ComfyUi => "ComfyUI",
    }
}

/// What is wrong with a connection, in words.
fn problem_text(p: ConnectionProblem) -> &'static str {
    match p {
        ConnectionProblem::NoKey => tl!("No key saved"),
        ConnectionProblem::KeyUnreadable => tl!("The key can't be read"),
        ConnectionProblem::SwitchedOff => tl!("Switched off"),
    }
}

/// Open the setup, or return the open one.
pub fn open(app: &mut PhotocraftApp) -> DialogId {
    if let Some(id) = super::open_dialog(app, KEY) {
        return id;
    }
    app.genai.setup.stale = true;
    let shared = Arc::clone(&app.genai.setup.shared);
    params::open_param_unsaved(app, KEY, DialogFields::new(), Value::Null, move |fields: &mut DialogFields| {
        let shared = Arc::clone(&shared);
        ParamDialog::command(KEY, tl!("Generative AI"), Vec::<Param>::new()).values(Bind::bound(fields)).ok_label(Label::from(tl!("Done"))).below(
            move |ui, _| {
                if let Ok(mut view) = shared.lock() {
                    rows(ui, &mut view);
                }
            },
        )
    })
}

/// Each frame: the workers' answers, a fresh read when due, and what the rows asked for. Only
/// while the setup is open.
pub(super) fn feed(app: &mut PhotocraftApp, ctx: &egui::Context) {
    poll(app, ctx);
    if super::open_dialog(app, KEY).is_none() {
        return;
    }
    let now = ctx.input(|i| i.time);
    let status = app.session.generative_status();
    let requests = app.genai.setup.shared.lock().map(|mut v| {
        v.status = super::status_text(&status);
        std::mem::take(&mut v.requests)
    });
    for request in requests.unwrap_or_default() {
        handle(app, request);
    }
    finish_read(app, ctx, now);
    let s = &app.genai.setup;
    let current = s.read_at.as_ref().is_some_and(|(at, seen)| !s.stale && now - at < REFRESH_S && *seen == status);
    if !current {
        start_read(app, ctx, now, &status);
    }
}

/// Run a row's request against the shared layer.
fn handle(app: &mut PhotocraftApp, request: Request) {
    let Some(ai) = app.session.generative_ai().cloned() else { return };
    match request {
        Request::Choose { connection, model } => changed(app, ai.choose(&connection, &model)),
        Request::Remove(id) => changed(app, ai.disconnect(&id)),
        Request::Connect(kind, value) => start_connect(app, kind, value),
    }
}

/// A choice or removal here was made: read the settings again, and show its error or clear the
/// last one.
fn changed(app: &mut PhotocraftApp, r: Result<(), AiFailure>) {
    app.genai.setup.stale = true;
    if let Ok(mut v) = app.genai.setup.shared.lock() {
        v.message = r.err().map(|e| (e.message, true));
    }
}

/// Keep `setup` as the last read, taken at `now` with `status`.
fn store(app: &mut PhotocraftApp, now: f64, status: AiStatus, setup: Result<SetupView, String>) {
    app.genai.setup.read_at = Some((now, status));
    if let Ok(mut v) = app.genai.setup.shared.lock() {
        v.setup = Some(setup);
    }
}

/// Read the shared settings on a worker, unless a read is under way.
#[cfg(not(target_arch = "wasm32"))]
fn start_read(app: &mut PhotocraftApp, ctx: &egui::Context, now: f64, status: &AiStatus) {
    if app.genai.setup.reading.is_some() {
        return;
    }
    app.genai.setup.stale = false;
    let Some(ai) = app.session.generative_ai().cloned() else {
        store(app, now, status.clone(), Err(super::NO_AI.to_owned()));
        return;
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let repaint = ctx.clone();
    let spawned = std::thread::Builder::new().name("photocraft-ai-setup".into()).spawn(move || {
        let _ = tx.send(ai.setup().map_err(|e| e.message));
        repaint.request_repaint();
    });
    match spawned {
        Ok(_) => app.genai.setup.reading = Some((status.clone(), rx)),
        Err(e) => store(app, now, status.clone(), Err(e.to_string())),
    }
}

/// A read of the shared settings under way: done?
#[cfg(not(target_arch = "wasm32"))]
fn finish_read(app: &mut PhotocraftApp, ctx: &egui::Context, now: f64) {
    let Some((status, rx)) = &app.genai.setup.reading else { return };
    match rx.try_recv() {
        Ok(v) => {
            let status = status.clone();
            app.genai.setup.reading = None;
            store(app, now, status, v);
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(50)),
        Err(std::sync::mpsc::TryRecvError::Disconnected) => app.genai.setup.reading = None,
    }
}

/// The web build has no shared AI layer to read: say so at once.
#[cfg(target_arch = "wasm32")]
fn start_read(app: &mut PhotocraftApp, _: &egui::Context, now: f64, status: &AiStatus) {
    app.genai.setup.stale = false;
    let v = app.session.generative_ai().map_or_else(|| Err(super::NO_AI.to_owned()), |ai| ai.setup().map_err(|e| e.message));
    store(app, now, status.clone(), v);
}

#[cfg(target_arch = "wasm32")]
fn finish_read(_: &mut PhotocraftApp, _: &egui::Context, _: f64) {}

/// A small heading.
fn heading(ui: &mut egui::Ui, text: &str) {
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.add_space(8.0);
    ui.label(egui::RichText::new(text).font(crate::theme::semibold(12.5)).color(t.text));
    ui.add_space(2.0);
}

/// The setup's rows: the status, the model, the connections and Connect a service. What they ask
/// for goes into `view.requests`.
fn rows(ui: &mut egui::Ui, view: &mut View) {
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.set_max_width(460.0);
    for line in &view.status {
        super::faint(ui, line);
    }
    super::faint(
        ui,
        tl!(
            "Connections and the model you choose are shared with the other Craft apps. Keys are kept in this computer's keychain, never in PhotoCraft's settings."
        ),
    );
    let setup = match &view.setup {
        Some(Ok(v)) => v.clone(),
        Some(Err(e)) => {
            ui.add(egui::Label::new(egui::RichText::new(e).color(t.danger).size(12.0)).wrap());
            return;
        }
        // The first read hasn't answered yet.
        None => {
            ui.add(egui::Spinner::new());
            return;
        }
    };

    heading(ui, tl!("Model for Generative Fill"));
    if setup.models.is_empty() {
        super::faint(ui, tl!("No connected model can do Generative Fill yet. Connect a service below."));
    }
    for m in &setup.models {
        let r = ui.radio(m.selected, &m.label);
        let r = if m.description.is_empty() { r } else { r.on_hover_text(&m.description) };
        if r.clicked() && !m.selected {
            view.requests.push(Request::Choose { connection: m.connection.clone(), model: m.model.clone() });
        }
        if !m.description.is_empty() {
            ui.indent(("genai-model", &m.connection, &m.model), |ui| super::faint(ui, &m.description));
        }
    }

    if !setup.connections.is_empty() {
        heading(ui, tl!("Connections"));
    }
    for c in &setup.connections {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&c.label).color(t.text));
            let place = if c.local { tl!("On this computer") } else { tl!("Online") };
            let detail = match &c.address {
                Some(a) => format!("{a} · {place}"),
                None => place.to_owned(),
            };
            ui.label(egui::RichText::new(detail).color(t.text_faint).size(12.0));
            if let Some(p) = c.problem {
                ui.label(egui::RichText::new(problem_text(p)).color(t.danger).size(12.0));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(tl!("Remove")).clicked() {
                    view.requests.push(Request::Remove(c.id.clone()));
                }
            });
        });
    }

    heading(ui, tl!("Connect a service"));
    connect_row(ui, view);
    super::faint(ui, tl!("Connecting asks the service for its models with your key. Nothing from your documents is sent."));
    if let Some((text, error)) = &view.message {
        ui.add(egui::Label::new(egui::RichText::new(text).color(if *error { t.danger } else { t.text_dim }).size(12.0)).wrap());
    }
}

/// The service, its key (or address) and Connect.
fn connect_row(ui: &mut egui::Ui, view: &mut View) {
    let mut kind = view.kind.unwrap_or(ServiceKind::OpenAi);
    let mut go = false;
    ui.horizontal(|ui| {
        for k in ServiceKind::ALL {
            ui.radio_value(&mut kind, k, kind_name(k));
        }
    });
    ui.horizontal(|ui| {
        view.kind = Some(kind);
        if kind.needs_key() {
            ui.add(egui::TextEdit::singleline(&mut view.key).password(true).hint_text(tl!("API key")).desired_width(220.0));
        } else {
            ui.add(egui::TextEdit::singleline(&mut view.address).hint_text("http://127.0.0.1:8188").desired_width(220.0));
        }
        let ready = !kind.needs_key() || !view.key.trim().is_empty();
        let label = if view.busy { tl!("Checking…") } else { tl!("Connect") };
        go = ui.add_enabled(ready && !view.busy, egui::Button::new(label)).clicked();
    });
    if go {
        let value = if kind.needs_key() { std::mem::take(&mut view.key) } else { view.address.trim().to_owned() };
        view.requests.push(Request::Connect(kind, value));
    }
}

/// Set the setup's outcome line and busy flag.
fn report(app: &mut PhotocraftApp, busy: bool, message: Option<(String, bool)>) {
    if let Ok(mut v) = app.genai.setup.shared.lock() {
        v.busy = busy;
        v.message = message;
    }
}

/// Check and save the connection on a worker: it asks the service for its models.
#[cfg(not(target_arch = "wasm32"))]
fn start_connect(app: &mut PhotocraftApp, kind: ServiceKind, value: String) {
    let Some(ai) = app.session.generative_ai().cloned() else { return };
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("photocraft-ai-connect".into()).spawn(move || {
        let _ = tx.send(ai.connect(kind, &value));
    });
    match spawned {
        Ok(_) => {
            app.genai.setup.busy = Some(rx);
            report(app, true, Some((tl!("Checking…").to_owned(), false)));
        }
        Err(e) => report(app, false, Some((e.to_string(), true))),
    }
}

#[cfg(target_arch = "wasm32")]
fn start_connect(app: &mut PhotocraftApp, _: ServiceKind, _: String) {
    report(app, false, Some((super::NO_AI.to_owned(), true)));
}

/// A connection that was being checked: done?
#[cfg(not(target_arch = "wasm32"))]
fn poll(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(rx) = &app.genai.setup.busy else { return };
    match rx.try_recv() {
        Ok(r) => {
            app.genai.setup.busy = None;
            app.genai.setup.stale = true;
            let message = match r {
                Ok(()) => (tl!("Connected.").to_owned(), false),
                Err(e) => (e.message, true),
            };
            report(app, false, Some(message));
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
        Err(std::sync::mpsc::TryRecvError::Disconnected) => app.genai.setup.busy = None,
    }
}

#[cfg(target_arch = "wasm32")]
fn poll(_: &mut PhotocraftApp, _: &egui::Context) {}

/// Type `key` into the key field and press Connect, as the rows would. For tests.
#[cfg(test)]
pub(super) fn connect_for_test(app: &mut PhotocraftApp, kind: ServiceKind, key: &str) {
    handle(app, Request::Connect(kind, key.to_owned()));
}

/// Choose `model` of `connection`, as the rows would. For tests.
#[cfg(test)]
pub(super) fn choose_for_test(app: &mut PhotocraftApp, connection: &str, model: &str) {
    handle(app, Request::Choose { connection: connection.to_owned(), model: model.to_owned() });
}

/// The last read of the shared settings. For tests.
#[cfg(test)]
pub(super) fn read_for_test(app: &PhotocraftApp) -> Option<Result<SetupView, String>> {
    app.genai.setup.shared.lock().ok().and_then(|v| v.setup.clone())
}
