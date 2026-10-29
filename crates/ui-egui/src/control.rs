//! Programmatic control of the running app, for agents, tests and (soon) MCP.
//!
//! Transport-agnostic: a transport thread (TCP in the desktop app, a channel in tests) sends
//! [`ControlRequest`]s; the UI thread handles them between frames and replies with JSON.
//!
//! Methods:
//! - `engine.execute {command, params}`: run any engine or UI command by id
//! - `engine.commands`: list commands with enablement
//! - `ui.inspect`: full UI state (tool, panels, views, dialogs, windows, window size); the menu
//!   tree is `ui.menu.list`
//! - `ui.set {tool?, panels?, dock?, dockTabs?, dockWidth?, colorPanel?, maskTarget?, vectorMaskTarget?, filterMaskTarget?, selectionMode?, zoom?, center?, rotation?, fit?, theme?, brushSection?, brushTab?, brushesView?, brushPicker?, brushPickerName?, brushPickerStroke?, brushPickerTip?, brushPickerScale?, brushSize?}`:
//!   change UI state; any other field is an error ([`UI_SET_FIELDS`])
//! - `ui.dialog.open {kind, fields?}` (kinds: newDocument, about, layerStyle {effect?}, colorPicker {target: foreground|background}, command {command}) / `ui.dialog.set {dialog, field, value}` / `ui.dialog.confirm {dialog, wait?}` / `ui.dialog.cancel {dialog}`
//! - `ui.dialog.apply {dialog}`: commit Preferences changes without closing the dialog
//! - `ui.window.open {document?}` / `ui.window.close {window}`: extra document windows
//! - `ui.pointer {events: [{kind: down|move|up, x, y, pressure?, tiltX?, tiltY?, rotation?}], tool?, modifiers?, button?}`: drive the active tool in document coordinates (`button: "secondary"` opens the tool's canvas context menu (the pasteboard colour menu outside the image) or Brush Preset picker, or erases with Preferences › Tools › Right-click with painting tools = erase)
//! - `ui.click {x, y, button?, count?}` / `ui.move {x, y}`: synthetic pointer input in screen points
//!   (`count` at most [`MAX_CLICKS`])
//! - `ui.key {key, command?, shift?, alt?, ctrl?}` / `ui.type {text}`: synthetic keyboard input
//! - The synthetic input methods reject unknown fields, wrong types, unknown enum values and
//!   out-of-range numbers before acting, naming the parameter (#2449)
//! - `ui.resize {width, height}`: resize the main window
//! - `ui.gpu.simulateLoss {error?, destroy?}`: act as if the wgpu device was lost (or, with
//!   `error: true`, reported an error): the app switches to the CPU renderer for the rest of the
//!   session, as on a real loss. For testing the fallback; returns whether a GPU canvas was active.
//!   `destroy: true` destroys the window's real device instead, as a driver reset does: the app
//!   saves its unsaved documents for recovery and closes (#3438); returns `destroyed`
//! - `ui.screenshot {path?, focus?}`: capture the main window (PNG). Raises the window first (default)
//!   because occluded macOS windows stop rendering
//! - `ui.focus`: bring the main window to the front
//! - `app.open {path}` / `app.save {path}`: relative file I/O under the automation roots; reply with `warnings`
//! - `app.quit`
//! - `jobs.list` / `jobs.cancel {job?}`: background jobs (#210) with progress; cancel one (or all).
//!   `engine.execute`, `ui.menu.invoke` and `ui.dialog.confirm` wait for a command that runs as a
//!   job unless `wait: false` (then the reply is `{job, pending: true}`)
//! - `health`: liveness counters and ages, answered by the transport without the frame loop

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::{DialogKind, Tool};

mod dispatch;
mod inspect;
mod params;

use dispatch::dispatch;
pub use inspect::{inspect, save_screenshot, save_screenshot_rgba};
#[cfg(test)]
use params::screen_point;
pub use params::{MAX_CLICKS, UI_SET_FIELDS};
use params::{err, ok};

pub type ControlResponse = Value;

/// The control server's per-request reply deadline.
pub const REPLY_DEADLINE: std::time::Duration = std::time::Duration::from_secs(60);

/// The error the transport thread returns when a dispatched command gave no reply within
/// [`REPLY_DEADLINE`]. It warns that the command may still apply: re-sending blindly could apply
/// an edit twice (#3662). Built from the deadline constant, so the text cannot drift from it.
pub fn timeout_error() -> String {
    format!(
        "timeout: no reply in {}s — the command may still be running and may still apply, or the app was not drawing frames (display asleep, window fully occluded, or a long operation blocked the frame loop)",
        REPLY_DEADLINE.as_secs()
    )
}

/// The reply a request gets when its [`REPLY_DEADLINE`] passes.
pub fn timeout_reply() -> Value {
    json!({"ok": false, "error": timeout_error()})
}

/// The error for a request still queued when its deadline passed, from the UI thread or the
/// transport (see [`Dispatch`]). It was dropped before dispatch, so re-sending it is safe (#3662). Same `timeout` prefix as
/// [`timeout_error`], which machines match on.
pub fn expired_error() -> String {
    format!(
        "timeout: not run — the request was still queued when its {}s deadline passed, so it was dropped before dispatch and is safe to re-send; the app was not drawing frames (display asleep, window fully occluded, or a long operation blocked the frame loop)",
        REPLY_DEADLINE.as_secs()
    )
}

pub struct ControlRequest {
    pub method: String,
    pub params: Value,
    pub reply: Sender<ControlResponse>,
    /// Requests still queued at this instant are rejected; already dispatched work continues.
    pub deadline: Option<std::time::Instant>,
    /// Shared with the transport, so that when it stops waiting it knows whether the command ran.
    pub dispatch: Dispatch,
}

impl ControlRequest {
    pub fn new(method: impl Into<String>, params: Value) -> (Self, std::sync::mpsc::Receiver<ControlResponse>) {
        let (tx, rx) = std::sync::mpsc::channel();
        (Self { method: method.into(), params, reply: tx, deadline: None, dispatch: Dispatch::default() }, rx)
    }
}

/// Whether a request was dispatched, settled once between the UI thread (about to run it) and
/// the transport (giving up on its reply). The transport's clock runs out no later than the UI
/// thread's, so without this it could only ever say the command may still apply (#3662).
#[derive(Clone, Default)]
pub struct Dispatch(std::sync::Arc<std::sync::atomic::AtomicU8>);

const QUEUED: u8 = 0;
const DISPATCHED: u8 = 1;
const ABANDONED: u8 = 2;

impl Dispatch {
    /// The UI thread claims the request to run it: false once the transport gave up on it. A
    /// request the UI thread rejects as expired stays queued, which `give_up` reads as not run.
    pub(crate) fn start(&self) -> bool {
        self.settle(DISPATCHED) == DISPATCHED
    }

    /// The transport stops waiting: the error to reply with, [`expired_error`] when the command
    /// can no longer run and [`timeout_error`] when it was dispatched and may still apply.
    pub fn give_up(&self) -> String {
        if self.settle(ABANDONED) == DISPATCHED { timeout_error() } else { expired_error() }
    }

    /// Moves a queued request to `to`; returns the state it ends in.
    fn settle(&self, to: u8) -> u8 {
        use std::sync::atomic::Ordering;
        self.0.compare_exchange(QUEUED, to, Ordering::AcqRel, Ordering::Acquire).map_or_else(|state| state, |_| to)
    }
}

/// How long the transport waits after requesting a repaint before deciding the frame loop
/// did not service it: a wedged compositor sends no frame callbacks at all.
pub const STALL_CHECK: std::time::Duration = std::time::Duration::from_secs(1);

/// The error a read-only getter fails fast with when the frame loop is wedged and no
/// snapshot of its reply exists yet. Built from [`STALL_CHECK`], so the text cannot drift.
pub fn not_drawing_frames_error() -> String {
    format!(
        "not drawing frames: no frame update ran within {}s of the request (display asleep, window fully occluded, or a blocked frame loop); call `health` for liveness and retry when frames resume",
        STALL_CHECK.as_secs()
    )
}

/// The read-only getters whose last reply is kept for serving while the frame loop is
/// wedged: the methods an agent probes state with. `document.inspect` is keyed per
/// document (the `document` param inside `params`, which is what `engine.execute`
/// dispatches), since each document has its own reply; no param means the active document.
pub fn snapshot_key(method: &str, params: &Value) -> Option<String> {
    match method {
        "ui.inspect" => Some("ui.inspect".into()),
        "engine.commands" => Some("engine.commands".into()),
        "jobs.list" => Some("jobs.list".into()),
        "engine.execute" => {
            // `dispatch` accepts the command id as `command` or `id`; both spellings key alike.
            let command = params.get("command").or_else(|| params.get("id")).and_then(Value::as_str)?;
            match command {
                "document.inspect" => {
                    let index = params.get("params").and_then(|p| p.get("document"));
                    Some(match index {
                        Some(index) => format!("engine.execute:document.inspect:{}", index_key(index)),
                        None => "engine.execute:document.inspect".into(),
                    })
                }
                "session.inspect" => Some("engine.execute:session.inspect".into()),
                _ => None,
            }
        }
        _ => None,
    }
}

/// A `document` index rendered into a snapshot key.
fn index_key(value: &Value) -> String {
    if let Some(n) = value.as_u64() {
        n.to_string()
    } else if let Some(n) = value.as_i64() {
        n.to_string()
    } else if let Some(n) = value.as_f64() {
        n.to_string()
    } else {
        value.to_string()
    }
}

/// Liveness counters and the last snapshot-getter replies, shared between the UI thread
/// (which updates them) and the transport threads (which read them while the frame loop
/// is wedged). `health` is answered from these alone, so it works even when
/// no frame ever runs.
#[derive(Default)]
pub struct ControlWatch {
    logic_count: AtomicU64,
    last_logic_ms: AtomicU64,
    drawn_count: AtomicU64,
    last_drawn_ms: AtomicU64,
    snapshots: Mutex<HashMap<String, (u64, Arc<Value>)>>,
}

impl ControlWatch {
    pub fn new() -> Self {
        Self::default()
    }

    /// Stamp a `logic` pass (runs even for hidden windows when a repaint was requested).
    pub fn note_logic(&self) {
        self.note(&self.logic_count, &self.last_logic_ms);
    }

    /// Stamp a rendered frame; stalls while the window is hidden or occluded.
    pub fn note_drawn(&self) {
        self.note(&self.drawn_count, &self.last_drawn_ms);
    }

    fn note(&self, count: &AtomicU64, at: &AtomicU64) {
        count.fetch_add(1, Ordering::Relaxed);
        at.store(crate::gpu_canvas::now_ms().max(0.0) as u64, Ordering::Relaxed);
    }

    pub fn logic_count(&self) -> u64 {
        self.logic_count.load(Ordering::Relaxed)
    }

    pub fn drawn_count(&self) -> u64 {
        self.drawn_count.load(Ordering::Relaxed)
    }

    /// Remember a snapshot getter's reply, so a wedged frame loop can still serve it.
    pub fn note_snapshot(&self, key: &str, reply: &Value) {
        let value = Arc::new(reply.clone());
        let mut map = self.snapshots.lock().unwrap_or_else(|e| e.into_inner());
        map.insert(key.to_string(), ((crate::gpu_canvas::now_ms().max(0.0)) as u64, value));
    }

    /// The last snapshot-getter reply and its age in milliseconds.
    pub fn snapshot_with_age(&self, key: &str) -> Option<(u64, Arc<Value>)> {
        let map = self.snapshots.lock().unwrap_or_else(|e| e.into_inner());
        let (built, value) = map.get(key)?;
        let now = (crate::gpu_canvas::now_ms().max(0.0)) as u64;
        Some((now.saturating_sub(*built), Arc::clone(value)))
    }

    /// The `health` result: factual counters and ages only, answerable from a transport
    /// thread. An idle-but-healthy app legitimately shows stale ages; a counter at
    /// zero reports its age as null, since no pass has ever run.
    pub fn health(&self) -> Value {
        let now = (crate::gpu_canvas::now_ms().max(0.0)) as u64;
        let ago = |count: u64, last: u64| if count == 0 { Value::Null } else { json!(now.saturating_sub(last)) };
        json!({
            "lastLogicAgoMs": ago(self.logic_count(), self.last_logic_ms.load(Ordering::Relaxed)),
            "lastDrawnAgoMs": ago(self.drawn_count(), self.last_drawn_ms.load(Ordering::Relaxed)),
            "logicCount": self.logic_count.load(Ordering::Relaxed),
            "drawnCount": self.drawn_count.load(Ordering::Relaxed),
        })
    }
}

/// What a transport thread does for a request whose [`STALL_CHECK`] elapsed without a
/// reply: the frame loop either serviced the repaint (alive, maybe slow) or never ran
/// (wedged: serve the getter's snapshot, or fail fast).
pub enum Stalled {
    /// The frame loop ran after the request; wait for its reply as before.
    KeepWaiting,
    /// Serve the getter's last reply, stamped with its age; the frame thread's eventual
    /// reply lands on a dropped receiver.
    Snapshot { age_ms: u64, value: Arc<Value> },
    /// The frame loop is wedged and no snapshot of this getter exists yet.
    NotDrawingFrames,
}

pub fn stalled_action(watch: &ControlWatch, method: &str, params: &Value, logic_before: u64) -> Stalled {
    if watch.logic_count() != logic_before {
        return Stalled::KeepWaiting;
    }
    match snapshot_key(method, params) {
        Some(key) => match watch.snapshot_with_age(&key) {
            Some((age_ms, value)) => Stalled::Snapshot { age_ms, value },
            None => Stalled::NotDrawingFrames,
        },
        None => Stalled::KeepWaiting,
    }
}

pub enum Outcome {
    Done(Value),
    Screenshot {
        token: u64,
        path: Option<String>,
    },
    /// Synthetic input queued: reply once the app has processed all of it (so a following
    /// `ui.inspect`/`engine.execute` observes the effect).
    AfterInput,
    /// A command started a background job and the caller waits for it: reply with its result
    /// once it has been applied (or with its error / "cancelled").
    AfterJob(photocraft_engine::jobs::JobId),
}

fn wrap(r: Result<Value, String>) -> Outcome {
    match r {
        Ok(v) => ok(v),
        Err(e) => err(e),
    }
}

/// Run a command the way automation does (script events off). Long commands may run as
/// background jobs: by default the reply waits for the job's result (backward compatible); with
/// `wait` false it is `{job, pending: true}` at once.
fn run_waiting(app: &mut PhotocraftApp, wait: bool, run: impl FnOnce(&mut PhotocraftApp) -> Result<Value, String>) -> Outcome {
    let events_enabled = app.session.prefs().script_events.enabled;
    if events_enabled {
        app.session.edit_prefs(|prefs| prefs.script_events.enabled = false);
    }
    app.jobs.last_started = None;
    let result = run(app);
    if events_enabled {
        app.session.edit_prefs(|prefs| prefs.script_events.enabled = true);
    }
    match (result, app.jobs.last_started.take()) {
        (Ok(_), Some(job)) if wait => Outcome::AfterJob(job),
        (result, _) => wrap(result),
    }
}

/// The `dialog` id of the shell's own dialog (`workspace_ui`) in `ui.inspect` and `ui.dialog.*`.
const SHELL_DIALOG: &str = "shell";

pub fn handle(app: &mut PhotocraftApp, ctx: &egui::Context, req: &ControlRequest) -> Outcome {
    let saved = app.session.authorize;
    if let Some(gate) = app.services.automation_authorize {
        app.session.authorize = Some(gate);
    }
    let outcome = dispatch(app, ctx, req);
    app.session.authorize = saved;
    outcome
}

#[cfg(test)]
mod tests;
