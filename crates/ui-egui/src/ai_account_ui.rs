//! Thin account controls for the existing AI edit form. Only explicit sign-in opens a browser.
use crate::PhotocraftApp;
use egui::Context;
use photocraft_engine::cmd::ai_account_cmds::{COMMAND, Snapshot, State};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountUi {
    pub status: Snapshot,
    #[serde(skip)]
    browser_requested: bool,
    #[serde(skip)]
    browser_error: bool,
    #[serde(skip)]
    refresh_requested: bool,
}

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.get("__command").and_then(Value::as_str) == Some("edit.ai.codex")
}

fn action(app: &mut PhotocraftApp, action: &str) {
    match app.run(COMMAND, json!({"action":action})) {
        Ok(value) => match serde_json::from_value(value) {
            Ok(status) => app.ui.ai_account.status = status,
            Err(_) => app.ui.ai_account.status = Snapshot { state: State::Failed, message: Some("Invalid account status".into()), plan: None },
        },
        Err(error) => app.ui.ai_account.status = Snapshot { state: State::Failed, message: Some(error.to_string()), plan: None },
    }
}

pub fn opened(app: &mut PhotocraftApp) {
    app.ui.ai_account = AccountUi::default();
    // Headless UI harnesses do not start an installed runtime implicitly. They inject a
    // deterministic account service; the actual desktop uses background_jobs.
    if app.background_jobs || app.session.ai_account.configured() {
        if app.session.ai_account.pending() {
            app.ui.ai_account.refresh_requested = true;
            action(app, "status");
        } else {
            action(app, "refresh");
        }
    }
}

fn login_url(app: &mut PhotocraftApp) -> Option<String> {
    let value = app.run(COMMAND, json!({"action":"loginUrl"})).ok();
    // Fetching the URL may consume completion too; do not leave a stale waiting UI.
    action(app, "status");
    value.and_then(|value| {
        value.get("url").and_then(Value::as_str).filter(|url| photocraft_engine::cmd::ai_account_cmds::login_url_is_safe(url)).map(str::to_owned)
    })
}

fn open_browser(app: &mut PhotocraftApp, ctx: &Context) {
    let url = login_url(app);
    if app.ui.ai_account.status.state != State::WaitingForBrowser {
        return;
    }
    if let Some(url) = url {
        // Use the same native service as PhotoCraft's Help/About links. The desktop disables
        // eframe's optional links feature, so ctx.open_url alone silently does nothing there.
        // Do not use links::open: it writes the full URL into persistent UI status/history.
        app.ui.ai_account.browser_error = if let Some(open) = &app.services.open_url {
            open(&url).is_err()
        } else {
            #[cfg(target_arch = "wasm32")]
            {
                ctx.open_url(egui::OpenUrl::new_tab(url));
                false
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let _ = ctx;
                true
            }
        };
    } else {
        app.ui.ai_account.browser_error = true;
    }
}

pub fn tick(app: &mut PhotocraftApp, ctx: &Context) {
    let pending = app.session.ai_account.pending();
    if pending || app.ui.dialogs.iter().any(|dialog| owns(&dialog.fields)) {
        action(app, "status");
    }
    // A reopened dialog can outlive the cancelled worker. Retry only after polling
    // its completion, and never start an implicit refresh after the dialog closes.
    if app.ui.ai_account.refresh_requested && !app.session.ai_account.pending() {
        app.ui.ai_account.refresh_requested = false;
        if app.ui.dialogs.iter().any(|dialog| owns(&dialog.fields)) {
            action(app, "refresh");
        }
    }
    if app.session.ai_account.pending() {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
    if app.ui.ai_account.browser_requested && app.ui.ai_account.status.state == State::WaitingForBrowser {
        app.ui.ai_account.browser_requested = false;
        open_browser(app, ctx);
    }
    if matches!(app.ui.ai_account.status.state, State::Connected | State::Failed | State::Cancelled) {
        app.ui.ai_account.browser_requested = false;
    }
}

pub fn closed(app: &mut PhotocraftApp) {
    app.ui.ai_account.refresh_requested = false;
    app.ui.ai_account.browser_requested = false;
    app.ui.ai_account.browser_error = false;
    if app.session.ai_account.pending() {
        action(app, "cancel");
    }
}

pub fn can_confirm(app: &PhotocraftApp, fields: &Map<String, Value>) -> bool {
    !owns(fields)
        || (app.ui.ai_account.status.state == State::Connected
            && !app.session.ai_account.pending()
            && fields.get("prompt").and_then(Value::as_str).is_some_and(|prompt| !prompt.trim().is_empty() && prompt.len() <= 16_000))
}

pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    tick(app, ui.ctx());
    ui.set_max_width(440.0);
    let state = app.ui.ai_account.status.state.clone();
    let text = match state {
        State::Unknown => tl!("Codex account has not been checked"),
        State::Checking => tl!("Checking Codex connection…"),
        State::WaitingForBrowser => tl!("Complete ChatGPT sign-in in your browser"),
        State::Connected => tl!("Connected to ChatGPT"),
        State::SignedOut => tl!("ChatGPT sign-in is required"),
        State::Unsupported => tl!("Codex is using another login method. Sign in with ChatGPT to use your subscription."),
        State::Cancelling => tl!("Cancelling sign-in…"),
        State::Cancelled => tl!("Sign-in cancelled"),
        State::RuntimeUnavailable => tl!("Codex was not found. Install and open the Codex desktop app, then check the connection again."),
        State::Failed => tl!("Could not verify the account. Check your connection or sign in again."),
    };
    ui.label(text);
    if state == State::Connected
        && let Some(plan) = &app.ui.ai_account.status.plan
    {
        ui.weak(format!("{}: {plan}", tl!("ChatGPT plan")));
    }
    if matches!(state, State::Failed | State::RuntimeUnavailable)
        && let Some(message) = &app.ui.ai_account.status.message
    {
        ui.collapsing(tl!("Connection details"), |ui| {
            ui.label(message);
        });
    }
    ui.horizontal_wrapped(|ui| {
        let busy = app.session.ai_account.pending();
        if !matches!(state, State::Connected | State::Checking | State::WaitingForBrowser | State::Cancelling | State::RuntimeUnavailable)
            && ui.add_enabled(!busy, egui::Button::new(tl!("Sign in with ChatGPT"))).clicked()
        {
            action(app, "login");
            app.ui.ai_account.browser_requested = app.session.ai_account.pending();
        }
        if state == State::WaitingForBrowser && ui.button(tl!("Open sign-in page")).clicked() {
            open_browser(app, ui.ctx());
        }
        if state == State::WaitingForBrowser
            && ui.button(tl!("Copy sign-in link")).clicked()
            && let Some(url) = login_url(app)
        {
            ui.ctx().copy_text(url);
        }
        if ui.add_enabled(!busy, egui::Button::new(tl!("Check connection"))).clicked() {
            action(app, "refresh");
        }
        if busy && ui.add_enabled(state != State::Cancelling, egui::Button::new(tl!("Cancel sign-in"))).clicked() {
            closed(app);
        }
    });
    if state == State::RuntimeUnavailable {
        ui.weak(tl!("Use the complete Codex installation; copying codex.exe alone may omit image-editing components."));
    }
    if app.ui.ai_account.browser_error {
        ui.label(tl!("Could not open your browser. Try again or copy the sign-in link into your browser."));
    }
    ui.weak(tl!("Sign-in is managed by the installed Codex runtime and shared with Codex. PhotoCraft does not store tokens."));
    ui.separator();
    crate::view_cmds::form_body(ui, fields);
}
