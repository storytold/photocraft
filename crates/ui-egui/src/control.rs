//! PhotoCraft's control methods, for agents, tests and (soon) MCP: the method table that
//! craft-automation's control channel runs.
//!
//! A transport thread (TCP in the desktop app, a channel in tests) sends [`ControlRequest`]s;
//! between frames the UI thread hands each to [`handle`], which answers with a [`Reply`] that the
//! channel settles (`app/control.rs`). Desktop only, as craft-automation's channel is.
//!
//! Methods:
//! - `engine.execute {command, params}`: run any engine or UI command by id
//! - `engine.commands`: list commands with enablement
//! - `ui.inspect`: full UI state (tool, panels, views, dialogs, windows, window size, `rects`);
//!   `dialogs` lists the framework `DialogHost`'s dialogs in their one stack order (top last);
//!   `rects` maps each canvas, hosted dialog and the task bar's id to its last on-screen rect;
//!   the menu tree is `ui.menu.list`
//! - `ui.set {tool?, panels?, dock?, dockTabs?, dockWidth?, colorPanel?, maskTarget?, vectorMaskTarget?, filterMaskTarget?, selectionMode?, zoom?, center?, rotation?, fit?, theme?, brushSection?, brushTab?, brushesView?, brushPicker?, brushPickerName?, brushPickerStroke?, brushPickerTip?, brushPickerScale?, brushSize?, statusInfo?, marqueeStyle?, marqueeSize?, actionsPanel?, presetBrowsers?, toolbarColumns?, recentView?}`:
//!   change UI state; any other field is an error ([`UI_SET_FIELDS`])
//! - `ui.dialog.open {kind, fields?}` / `ui.dialog.set {dialog, field, value}` / `ui.dialog.confirm {dialog, wait?}` / `ui.dialog.cancel {dialog}`:
//!   `dialog` is an id as `ui.inspect` lists it, or `"top"` for the top hosted dialog
//! - `ui.dialog.apply {dialog}`: commit Preferences changes without closing the dialog
//! - `ui.notice.post {title, lines?, tone?}`: show a notice; replies `{id}`
//! - `ui.window.open {document?}` / `ui.window.close {window}`: extra document windows
//! - `ui.pointer {events: [{kind: down|move|up, x, y, pressure?, tiltX?, tiltY?, rotation?}], tool?, modifiers?, button?}`: drive the active tool in document coordinates (the open full-window workspace's tool while one is open). The events are queued for the canvas, which takes them on its next pass; the reply does not wait for them (`button: "secondary"` opens the tool's canvas context menu (the pasteboard colour menu outside the image) or Brush Preset picker, or erases with Preferences › Tools › Right-click with painting tools = erase)
//! - `ui.click {x, y, coordinates?, button?, count?, hold?, modifiers?}` / `ui.move {x, y, coordinates?, rest?, modifiers?}` /
//!   `ui.wheel {x, y, coordinates?, dx?, dy?, modifiers?}` / `ui.release {button?, key?, x?, y?, coordinates?}`: synthetic
//!   pointer input in screen points, or document points with `coordinates: "document"` (the main canvas maps them);
//!   `hold` leaves the last press down until `ui.release`, `rest` waits for a hover to rest before the reply
//!   (`count` at most [`MAX_CLICKS`], `x` and `y` within ±[`MAX_SCREEN_POINT`]; craft-automation's `synthetic`)
//! - `ui.key {key, hold?, command?, shift?, alt?, ctrl?}` / `ui.type {text}`: synthetic keyboard input
//! - `ui.capsLock {on}`: hold Caps Lock on or off for the canvas (`null` returns to the keyboard's state)
//! - The synthetic input methods reject unknown fields, wrong types, unknown enum values and
//!   out-of-range numbers before acting, naming the parameter (#2449)
//! - `ui.resize {width, height}`: resize the main window
//! - `ui.gpu.simulateLoss {error?, destroy?}`: act as if the wgpu device was lost (or, with
//!   `error: true`, reported an error): the app switches to the CPU renderer for the rest of the
//!   session, as on a real loss. For testing the fallback; returns whether a GPU canvas was active.
//!   `destroy: true` destroys the window's real device instead, as a driver reset does: the app
//!   saves its unsaved documents for recovery and closes (#3438); returns `destroyed`
//! - `ui.screenshot {path?, focus?, window?}`: capture the main window, or the document window `window` (PNG).
//!   Raises the window first (default) because occluded macOS windows stop rendering
//! - `ui.focus`: bring the main window to the front
//! - `app.open {path}` / `app.save {path}`: relative file I/O under the automation roots; reply with `warnings`
//! - `app.quit`
//! - `jobs.list` / `jobs.cancel {job?}`: background jobs (#210) with progress; cancel one (or all).
//!   `engine.execute`, `ui.menu.invoke` and `ui.dialog.confirm` wait for a command that runs as a
//!   job unless `wait: false` (then the reply is `{job, pending: true}`)

pub use craft::automation::{ControlRequest, ControlResponse, REPLY_DEADLINE, Reply};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::Tool;

pub(crate) mod dialogs;
mod dispatch;
mod inspect;
mod params;

pub use craft::automation::synthetic::{MAX_CLICKS, MAX_SCREEN_POINT};
use dispatch::dispatch;
pub use inspect::inspect;
pub use params::UI_SET_FIELDS;
#[cfg(test)]
use params::screen_point;

/// Run a command the way automation does (script events off). Long commands may run as
/// background jobs: by default the reply waits for the job's result (backward compatible); with
/// `wait` false it is `{job, pending: true}` at once.
fn run_waiting(app: &mut PhotocraftApp, wait: bool, run: impl FnOnce(&mut PhotocraftApp) -> Result<Value, String>) -> Reply {
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
        (Ok(_), Some(job)) if wait => Reply::AfterJob(job.0),
        // A dialog's confirm has no value of its own: the job it started is its reply.
        (Ok(Value::Null), Some(job)) => Reply::from_result(Ok(json!({"job": job.0, "pending": true}))),
        (result, _) => Reply::from_result(result),
    }
}

/// The methods that only hand the window input (it reaches a region as a hand's does) or only read: every other method
/// may change what a settling region shows, so it counts as a command run (`PhotocraftApp::commands_run`).
const INPUT_OR_READ: &[&str] = &[
    "engine.commands",
    "jobs.list",
    "ui.capsLock",
    "ui.click",
    "ui.inspect",
    "ui.key",
    "ui.menu.list",
    "ui.move",
    "ui.pointer",
    "ui.release",
    "ui.screenshot",
    "ui.type",
    "ui.wheel",
];

pub fn handle(app: &mut PhotocraftApp, ctx: &egui::Context, req: &ControlRequest) -> Reply {
    // A view command run by id (`engine.execute`, `ui.menu.invoke`) changes only what the view's regions hold in their
    // versions.
    let command = match req.method.as_str() {
        "engine.execute" => req.params.get("command"),
        "ui.menu.invoke" => req.params.get("id"),
        _ => None,
    };
    let view_only = command.and_then(serde_json::Value::as_str).is_some_and(|id| crate::panels::region_versions::changes_only_the_view(app, id));
    if !INPUT_OR_READ.contains(&req.method.as_str()) && !view_only {
        app.commands_run += 1;
        ctx.request_repaint();
    }
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

pub(super) const METHODS: &[&str] = &[
    "app.open",
    "app.quit",
    "app.save",
    "engine.commands",
    "engine.execute",
    "jobs.cancel",
    "jobs.list",
    "ui.capsLock",
    "ui.click",
    "ui.context.choose",
    "ui.dialog.apply",
    "ui.dialog.cancel",
    "ui.dialog.confirm",
    "ui.dialog.open",
    "ui.dialog.set",
    "ui.focus",
    "ui.gpu.simulateLoss",
    "ui.inspect",
    "ui.key",
    "ui.menu.invoke",
    "ui.menu.list",
    "ui.move",
    "ui.notice.post",
    "ui.pointer",
    "ui.release",
    "ui.resize",
    "ui.screenshot",
    "ui.set",
    "ui.type",
    "ui.wheel",
    "ui.window.close",
    "ui.window.open",
];
