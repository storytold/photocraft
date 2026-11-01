//! Account operations have no document edits; all I/O happens on a bounded worker.
use crate::{CommandSpec, EngineError, Result, Session};
#[cfg(not(target_arch = "wasm32"))]
pub use photocraft_gen::Error as AccountError;
#[cfg(not(target_arch = "wasm32"))]
use photocraft_gen::codex::CodexProvider;
#[cfg(not(target_arch = "wasm32"))]
pub use photocraft_gen::codex::{AccountInfo, AccountService};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, TryRecvError},
};

pub const COMMAND: &str = "ai.codex.account";

pub fn login_url_is_safe(url: &str) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        photocraft_gen::codex::valid_login_url(url)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = url;
        false
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    #[default]
    Unknown,
    Checking,
    WaitingForBrowser,
    Connected,
    SignedOut,
    Unsupported,
    Cancelling,
    Cancelled,
    RuntimeUnavailable,
    Failed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub state: State,
    pub plan: Option<String>,
    pub message: Option<String>,
}

#[cfg(not(target_arch = "wasm32"))]
enum Event {
    Url(String),
    Done(std::result::Result<AccountInfo, AccountError>),
}

#[cfg(not(target_arch = "wasm32"))]
struct Pending {
    events: Receiver<Event>,
    cancel: Arc<AtomicBool>,
    worker: std::thread::JoinHandle<()>,
}

#[derive(Default)]
pub struct Account {
    pub snapshot: Snapshot,
    #[cfg(not(target_arch = "wasm32"))]
    service: Option<Arc<dyn AccountService>>,
    #[cfg(not(target_arch = "wasm32"))]
    pending: Option<Pending>,
    // Transient and never serialized, journalled or copied into UI state.
    #[cfg(not(target_arch = "wasm32"))]
    login_url: Option<String>,
}

impl Account {
    pub fn pending(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.pending.is_some()
        }
        #[cfg(target_arch = "wasm32")]
        {
            false
        }
    }

    pub fn configured(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.service.is_some()
        }
        #[cfg(target_arch = "wasm32")]
        {
            false
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_service(&mut self, service: Arc<dyn AccountService>) -> Result<()> {
        if self.pending() {
            return Err(EngineError::Other("An account operation is already running".into()));
        }
        self.service = Some(service);
        self.snapshot = Snapshot::default();
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn start(&mut self, login: bool) -> Result<()> {
        if self.pending() {
            return Err(EngineError::Other("An account operation is already running".into()));
        }
        let service = self.service.get_or_insert_with(|| Arc::new(CodexProvider::default())).clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (tx, events) = mpsc::sync_channel(4);
        let worker = std::thread::Builder::new()
            .name("photocraft-codex-account".into())
            .spawn(move || {
                let cancelled = || worker_cancel.load(Ordering::Acquire);
                let result = if login {
                    service.login(&mut |url| tx.try_send(Event::Url(url)).map_err(|_| photocraft_gen::Error::Cancelled), &cancelled)
                } else {
                    service.check(&cancelled)
                };
                let _ = tx.try_send(Event::Done(result));
            })
            .map_err(|e| EngineError::Other(format!("Could not start account worker: {e}")))?;
        self.pending = Some(Pending { events, cancel, worker });
        self.login_url = None;
        self.snapshot = Snapshot { state: State::Checking, ..Default::default() };
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn cancel(&mut self) {
        if let Some(pending) = &self.pending {
            pending.cancel.store(true, Ordering::Release);
            self.snapshot.state = State::Cancelling;
        }
        self.login_url = None;
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn poll(&mut self) {
        // One URL and one completion normally; bound work per call even for injected services.
        for _ in 0..4 {
            let Some(pending) = &self.pending else { return };
            let event = match pending.events.try_recv() {
                Ok(event) => event,
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => Event::Done(Err(AccountError::Provider("The Codex account connection stopped. Try again.".into()))),
            };
            let cancelled = pending.cancel.load(Ordering::Acquire);
            match event {
                Event::Url(url) if !cancelled && photocraft_gen::codex::valid_login_url(&url) => {
                    self.login_url = Some(url);
                    self.snapshot.state = State::WaitingForBrowser;
                }
                Event::Url(_) => {}
                Event::Done(result) => {
                    self.pending = None;
                    self.login_url = None;
                    self.snapshot = if cancelled {
                        Snapshot { state: State::Cancelled, ..Default::default() }
                    } else {
                        match result {
                            Ok(AccountInfo::SignedOut) => Snapshot { state: State::SignedOut, ..Default::default() },
                            Ok(AccountInfo::Unsupported) => Snapshot { state: State::Unsupported, ..Default::default() },
                            Ok(AccountInfo::ChatGpt { plan }) => Snapshot { state: State::Connected, plan, message: None },
                            Err(AccountError::RuntimeUnavailable(message)) => Snapshot { state: State::RuntimeUnavailable, message: Some(message), plan: None },
                            Err(error) => Snapshot { state: State::Failed, message: Some(error.to_string()), plan: None },
                        }
                    };
                }
            }
        }
    }
}

impl Drop for Account {
    fn drop(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.cancel();
            if let Some(pending) = self.pending.take() {
                // Closing the app must let the cooperative service kill/wait its private
                // server before the process exits, otherwise its browser callback can leak.
                // Never join during ordinary UI polling or cancellation.
                drop(pending.events);
                let _ = pending.worker.join();
            }
        }
    }
}

fn enabled(_: &Session) -> std::result::Result<(), String> {
    if cfg!(target_arch = "wasm32") { Err("Codex accounts require the desktop runtime".into()) } else { Ok(()) }
}

fn run(s: &mut Session, params: &Value) -> Result<Value> {
    let action = params
        .as_object()
        .filter(|p| p.len() == 1)
        .and_then(|p| p.get("action"))
        .and_then(Value::as_str)
        .filter(|action| matches!(*action, "status" | "refresh" | "login" | "cancel" | "loginUrl"))
        .ok_or_else(|| EngineError::BadParams { cmd: COMMAND.into(), msg: "Expected {action: status|refresh|login|cancel|loginUrl}".into() })?;
    #[cfg(not(target_arch = "wasm32"))]
    {
        s.ai_account.poll();
        match action {
            "refresh" => s.ai_account.start(false)?,
            "login" => s.ai_account.start(true)?,
            "cancel" => s.ai_account.cancel(),
            "loginUrl" => return Ok(json!({"url":s.ai_account.login_url})),
            _ => {}
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = action;
    Ok(json!(s.ai_account.snapshot))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: COMMAND,
        label: "Codex Account",
        menu: &[],
        shortcut: None,
        params: r#"{"action":"status|refresh|login|cancel|loginUrl"} (explicit action; browser flow only, no API-key login)"#,
        enabled,
        run,
        journal: false,
    }]
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
