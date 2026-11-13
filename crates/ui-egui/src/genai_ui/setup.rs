//! Preferences › Integrations › Generative AI: PhotoCraft's small setup over the Craft suite's
//! shared AI layer, until the Craft launcher has its own Set up AI window. It shows whether
//! Generative Fill can run (the layer's reason as it is when it can't), lets the person pick the
//! model it uses (the suite's choice, as the launcher's), lists the saved connections, and
//! connects OpenAI, Black Forest Labs or Google Gemini with a key, or a ComfyUI server by its address.
//!
//! Connections, keys and the choice are the shared layer's, not PhotoCraft's preferences: a key is
//! typed into a buffer outside the dialog's fields (so `ui.inspect` and the control channel never
//! see it), handed to the layer once, and kept in the OS keychain by it.

use photocraft_engine::genai::{AiFailure, AiStatus, ConnectionProblem, ServiceKind, SetupView};

use crate::PhotocraftApp;

/// How long a read of the shared settings is shown before it is read again, in seconds.
const REFRESH_S: f64 = 10.0;

/// What the setup keeps between frames.
#[derive(Default)]
pub struct State {
    kind: Option<ServiceKind>,
    /// What is typed in the key field. Cleared once it has been handed over.
    pub(super) key: String,
    address: String,
    /// A connection being checked and saved off the interface thread.
    #[cfg(not(target_arch = "wasm32"))]
    busy: Option<std::sync::mpsc::Receiver<Result<(), AiFailure>>>,
    /// The last outcome to show: (text, is an error).
    pub(super) message: Option<(String, bool)>,
    /// The shared settings as last read, when, and the tool's status then.
    view: Option<(f64, AiStatus, Result<SetupView, String>)>,
    /// The last read is out of date (something here changed the settings): read them again.
    stale: bool,
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

fn heading(ui: &mut egui::Ui, text: &str) {
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.add_space(8.0);
    ui.label(egui::RichText::new(text).font(crate::theme::semibold(12.5)).color(t.text));
    ui.add_space(2.0);
}

/// The shared settings, read again after [`REFRESH_S`], once something here changed them, or as
/// soon as the tool's status (`status`, refreshed about every second) differs from when they were
/// read: a model chosen or a connection removed in another Craft app shows in the rows as soon as
/// in the status line, without reading the settings (and the keychain) every second. The read
/// runs on a worker: until it answers the last read shows, or nothing before the first.
fn view(app: &mut PhotocraftApp, ctx: &egui::Context, now: f64, status: &AiStatus) -> Option<Result<SetupView, String>> {
    finish_read(app, ctx, now);
    let s = &app.genai.setup;
    let current = s.view.as_ref().is_some_and(|(at, seen, _)| !s.stale && now - at < REFRESH_S && seen == status);
    if !current {
        start_read(app, ctx, now, status);
    }
    app.genai.setup.view.as_ref().map(|(_, _, v)| v.clone())
}

/// Read the shared settings on a worker, unless a read is under way.
#[cfg(not(target_arch = "wasm32"))]
fn start_read(app: &mut PhotocraftApp, ctx: &egui::Context, now: f64, status: &AiStatus) {
    if app.genai.setup.reading.is_some() {
        return;
    }
    app.genai.setup.stale = false;
    let Some(ai) = app.session.generative_ai().cloned() else {
        app.genai.setup.view = Some((now, status.clone(), Err(super::NO_AI.to_owned())));
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
        Err(e) => app.genai.setup.view = Some((now, status.clone(), Err(e.to_string()))),
    }
}

/// A read of the shared settings under way: done?
#[cfg(not(target_arch = "wasm32"))]
fn finish_read(app: &mut PhotocraftApp, ctx: &egui::Context, now: f64) {
    let Some((status, rx)) = &app.genai.setup.reading else { return };
    match rx.try_recv() {
        Ok(v) => {
            app.genai.setup.view = Some((now, status.clone(), v));
            app.genai.setup.reading = None;
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
    app.genai.setup.view = Some((now, status.clone(), v));
}

#[cfg(target_arch = "wasm32")]
fn finish_read(_: &mut PhotocraftApp, _: &egui::Context, _: f64) {}

/// A choice or removal here was made: read the settings again, and show its error or clear the
/// last one.
pub(super) fn changed(app: &mut PhotocraftApp, r: Result<(), AiFailure>) {
    app.genai.setup.stale = true;
    app.genai.setup.message = match r {
        Ok(()) => None,
        Err(e) => Some((e.message, true)),
    };
}

/// The section's rows, drawn under Preferences › Integrations.
pub fn prefs_section(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if !super::offered(app) {
        return;
    }
    let t = crate::theme::Tokens::get(ui.ctx());
    let now = ui.input(|i| i.time);
    poll(app, ui.ctx());
    heading(ui, tl!("Generative AI"));
    let status = app.session.generative_status();
    super::status_lines(ui, &status);
    super::faint(
        ui,
        tl!(
            "Connections and the model you choose are shared with the other Craft apps. Keys are kept in this computer's keychain, never in PhotoCraft's settings."
        ),
    );
    let setup = match view(app, ui.ctx(), now, &status) {
        Some(Ok(v)) => v,
        Some(Err(e)) => {
            ui.add(egui::Label::new(egui::RichText::new(e).color(t.danger).size(12.0)).wrap());
            return;
        }
        // The first read hasn't answered yet.
        None => return,
    };

    heading(ui, tl!("Model for Generative Fill"));
    if setup.models.is_empty() {
        super::faint(ui, tl!("No connected model can do Generative Fill yet. Connect a service below."));
    }
    let mut choose = None;
    for m in &setup.models {
        let r = ui.radio(m.selected, &m.label);
        let r = if m.description.is_empty() { r } else { r.on_hover_text(&m.description) };
        if r.clicked() && !m.selected {
            choose = Some((m.connection.clone(), m.model.clone()));
        }
        if !m.description.is_empty() {
            ui.indent(("genai-model", &m.connection, &m.model), |ui| super::faint(ui, &m.description));
        }
    }
    if let Some((c, m)) = choose
        && let Some(ai) = app.session.generative_ai().cloned()
    {
        changed(app, ai.choose(&c, &m));
    }

    if !setup.connections.is_empty() {
        heading(ui, tl!("Connections"));
    }
    let mut remove = None;
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
                if crate::widgets::secondary_button(ui, tl!("Remove"), 0.0).clicked() {
                    remove = Some(c.id.clone());
                }
            });
        });
    }
    if let Some(id) = remove
        && let Some(ai) = app.session.generative_ai().cloned()
    {
        changed(app, ai.disconnect(&id));
    }

    heading(ui, tl!("Connect a service"));
    connect_row(app, ui);
    super::faint(ui, tl!("Connecting asks the service for its models with your key. Nothing from your documents is sent."));
    if let Some((text, error)) = &app.genai.setup.message {
        ui.add(egui::Label::new(egui::RichText::new(text).color(if *error { t.danger } else { t.text_dim }).size(12.0)).wrap());
    }
}

/// The service, its key (or address) and Connect.
fn connect_row(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let s = &mut app.genai.setup;
    let mut kind = s.kind.unwrap_or(ServiceKind::OpenAi);
    let busy = connecting(s);
    let mut go = false;
    ui.horizontal(|ui| {
        let options: Vec<(ServiceKind, &str)> = ServiceKind::ALL.iter().map(|k| (*k, kind_name(*k))).collect();
        if crate::widgets::dropdown(ui, "genai-service", tl!("Connect a service"), &mut kind, &options, 150.0) {
            s.kind = Some(kind);
        }
        let r = if kind.needs_key() {
            ui.add(egui::TextEdit::singleline(&mut s.key).password(true).hint_text(tl!("API key")).desired_width(220.0))
        } else {
            ui.add(egui::TextEdit::singleline(&mut s.address).hint_text("http://127.0.0.1:8188").desired_width(220.0))
        };
        crate::field_tab::register(ui.ctx(), r.id);
        let ready = !kind.needs_key() || !s.key.trim().is_empty();
        let label = if busy { tl!("Checking…") } else { tl!("Connect") };
        go = ui.add_enabled_ui(ready && !busy, |ui| crate::widgets::secondary_button(ui, label, 0.0)).inner.clicked();
    });
    if go {
        let value = if kind.needs_key() { std::mem::take(&mut s.key) } else { s.address.trim().to_owned() };
        start_connect(app, kind, value);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn connecting(s: &State) -> bool {
    s.busy.is_some()
}

#[cfg(target_arch = "wasm32")]
fn connecting(_: &State) -> bool {
    false
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
            app.genai.setup.message = Some((tl!("Checking…").to_owned(), false));
        }
        Err(e) => app.genai.setup.message = Some((e.to_string(), true)),
    }
}

#[cfg(target_arch = "wasm32")]
fn start_connect(app: &mut PhotocraftApp, _: ServiceKind, _: String) {
    app.genai.setup.message = Some((super::NO_AI.to_owned(), true));
}

/// A connection that was being checked: done?
#[cfg(not(target_arch = "wasm32"))]
pub fn poll(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(rx) = &app.genai.setup.busy else { return };
    match rx.try_recv() {
        Ok(r) => {
            app.genai.setup.busy = None;
            app.genai.setup.stale = true;
            app.genai.setup.message = Some(match r {
                Ok(()) => (tl!("Connected.").to_owned(), false),
                Err(e) => (e.message, true),
            });
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
        Err(std::sync::mpsc::TryRecvError::Disconnected) => app.genai.setup.busy = None,
    }
}

#[cfg(target_arch = "wasm32")]
pub fn poll(_: &mut PhotocraftApp, _: &egui::Context) {}
