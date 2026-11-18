//! Background jobs in the shell (#210): long commands and file opens run on worker threads (see
//! `photocraft_engine::jobs`) while the window keeps drawing at full frame rate.
//!
//! - [`run`] starts a job-capable command in the background (when [`PhotocraftApp::background_jobs`]
//!   is on: the desktop app; tests and the web run inline as before).
//! - [`tick`] (every frame) applies finished jobs, reports failures, answers control requests
//!   waiting on a job, finishes background opens and keeps frames coming while anything runs.
//! - [`status_progress`] shows the status-bar progress with a Cancel button; [`dialog`] the blocking
//!   progress overlay for jobs that lock the active document (filters, Content-Aware Fill, …),
//!   shown after a short delay so quick jobs don't flash it. Esc cancels.
//! - Files opening in the background get a document tab at once ([`OpenTab`]); its canvas area
//!   shows the progress, and closing the tab cancels the open.
//! - Saves encode and write on a worker ([`start_save`], #2017); the document is locked
//!   meanwhile, and the save is recorded when the job ends. With Preferences › File Handling ›
//!   Save in Background on (the default) you keep working while the file is written, the progress
//!   in the document's tab and the status bar; off, the save runs in the foreground (#6207): the
//!   modal progress dialog shows at once and no shortcut or tool input reaches the app until the
//!   file is complete ([`foreground_save`]).
//!
//! The canvas keeps showing the pre-job state: workers never touch the session's documents.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use craft::ui::bars::StatusProgress;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _, Sizing};
use craft::ui::dialog::{BlockingOverlay, PromptAnswer, PromptChoice, StandardPrompt};
use craft::ui::display::{Picture, Progress, Shade, Text};
use craft::ui::layout::RowAlign;
use craft::ui::layout_components::Row;
use craft::ui::primitives::textures::Revision;
use craft::ui::runtime::overlay::CloseReason;
use craft::ui::shell::OpeningCard;
use craft::ui::style::TypeRole;
use photocraft_doc::DocId;
use photocraft_engine::jobs::{CancelRefused, JobCtx, JobEvent, JobId, JobInfo, JobOutcome, OPEN_JOB, OpenSource, Started};
use serde_json::{Value, json};

use crate::{ExportSettings, PhotocraftApp, SaveFileFn, notices};

/// Delay before the blocking progress overlay appears (quick jobs finish without it).
pub const DIALOG_DELAY_MS: f64 = 250.0;

/// A file opening in the background: shown as a document tab with progress.
#[derive(Clone, Debug)]
pub struct OpenTab {
    pub job: JobId,
    /// Display name (the file name).
    pub name: String,
    /// Where it was read from (File › Save writes back there; added to Open Recent).
    pub path: Option<String>,
    /// The tab position it goes to when it opens (a file dropped on the tabs); `None`: the end.
    pub slot: Option<usize>,
    /// Its place in a set of files opened together, when it is one of several (see [`OpenOrder`]).
    pub order: Option<OpenOrder>,
}

/// A file's place in a set opened together (File › Open with several files, paths on the command
/// line, the OS's "open these documents"): the set's tabs end up in its order whatever order the
/// background opens finish in (#3248), small files first otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenOrder {
    /// The set, from [`begin_batch`].
    pub batch: u64,
    /// The file's position in the set.
    pub seq: usize,
}

/// A set of files opened together: where its tabs start and which members have opened.
#[derive(Clone, Debug)]
pub(crate) struct OpenBatch {
    id: u64,
    /// The tab position of the set's first document: after the documents and opens there when it
    /// started.
    base: usize,
    /// The `seq` of every member whose document is in place.
    opened: Vec<usize>,
}

/// A file being placed in the background (File › Place, or a file dropped on the canvas): Free
/// Transform starts on its layer when the job lands, and the files dropped after it follow.
#[derive(Clone, Debug)]
pub struct PlaceJob {
    pub job: JobId,
    /// Display name (the file name).
    pub name: String,
    /// The document it lands in: Free Transform starts only while that one is in view.
    pub doc: DocId,
}

/// The job command of a background save.
pub const SAVE_JOB: &str = "file.save";

/// A save worker's state, shared between the worker and the UI. Whether it may still be
/// cancelled is its job's ([`JobCtx::commit`]), so every cancel path agrees.
#[derive(Debug, Default)]
pub struct SaveState {
    finished: AtomicBool,
}

impl SaveState {
    /// The worker has returned (or unwound): nothing of this save can still touch the file.
    pub fn finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }
}

/// Marks the worker finished when the work closure is dropped, which also covers a panic.
struct WorkerDone(Arc<SaveState>);

impl Drop for WorkerDone {
    fn drop(&mut self) {
        self.0.finished.store(true, Ordering::Release);
    }
}

/// What a save worker gets (see [`crate::Services::save_file`]): the job's progress and
/// cancellation, and [`commit`](Self::commit), the point after which the save can no longer
/// be cancelled.
pub struct SaveCtl {
    pub job: JobCtx,
}

impl SaveCtl {
    pub fn progress(&self, fraction: f32, message: &str) {
        self.job.progress(fraction, message);
    }

    /// `Err` once the save was cancelled: stop, write nothing.
    pub fn check(&self) -> Result<(), String> {
        if self.job.cancelled() { Err(SAVE_CANCELLED.into()) } else { Ok(()) }
    }

    /// Call once, right before replacing the file. `Ok`: the save can no longer be cancelled
    /// (Cancel, Esc, `jobs.cancel`, cancelling every job and closing the document all leave it
    /// running) and must run to the end; `Err`: it was cancelled first and nothing may be
    /// written.
    pub fn commit(&self) -> Result<(), String> {
        self.job.commit().map_err(|_| SAVE_CANCELLED.into())
    }
}

/// The error a cancelled save worker returns.
const SAVE_CANCELLED: &str = "cancelled";

/// A save running in the background: what to record once the file is written.
#[derive(Clone, Debug)]
pub struct SaveJob {
    pub job: JobId,
    pub doc: DocId,
    /// The document's revision when the save started (the job locks it against edits).
    pub revision: u64,
    pub path: String,
    /// A copy leaves the document's path and unsaved state alone.
    pub copy: bool,
    /// An automation save records without firing user-configured hooks.
    pub automation: bool,
    pub state: Arc<SaveState>,
    /// The job has ended (cancelled, say) but its worker may still be running: kept until it
    /// returns, so quitting and another save to the same file wait for it.
    pub ended: bool,
    /// Started with Save in Background off: the app waits for it under the blocking progress overlay
    /// (see [`foreground_save`]).
    pub foreground: bool,
}

/// The shell's job bookkeeping.
#[derive(Default)]
pub struct JobsUi {
    /// Background opens, in tab order.
    pub opens: Vec<OpenTab>,
    /// Sets of files opened together that still have members opening (see [`OpenOrder`]).
    pub(crate) batches: Vec<OpenBatch>,
    /// The id of the next set.
    next_batch: u64,
    /// Background saves.
    pub saves: Vec<SaveJob>,
    /// The window was asked to close while a save was writing: close it once none is left.
    pub close_after_saves: bool,
    /// The open tab shown in the canvas area instead of the active document.
    pub focus: Option<JobId>,
    /// The job the latest [`run`] started (the control channel waits on it).
    pub(crate) last_started: Option<JobId>,
    /// The file being placed in the background, if any. One at a time: files dropped meanwhile
    /// wait in the app's drop queue.
    pub place: Option<PlaceJob>,
}

/// Run command `id`: in the background when background jobs are on and the command is a job
/// (the result is then `{"job": id, "pending": true}` and the edit lands when [`tick`] applies
/// it), else synchronously like [`photocraft_engine::Session::execute`].
pub fn run(app: &mut PhotocraftApp, id: &str, params: Value) -> Result<Value, String> {
    // A place is also a job where there are no threads (the web build): the session steps it a
    // slice per frame, and every other command stays synchronous there.
    let place = matches!(id, "file.placeEmbedded" | "file.placeLinked") && place_in_background(app);
    if !app.background_jobs && !place {
        return app.session.execute(id, params).map_err(|e| e.to_string());
    }
    match app.session.start(id, params).map_err(|e| e.to_string())? {
        Started::Done(v) => Ok(v),
        Started::Job(job) => {
            app.jobs.last_started = Some(job);
            // The status bar's progress readout names the job; clear any old message.
            app.ui.status.clear();
            app.ui.status_error = false;
            Ok(json!({"job": job.0, "pending": true}))
        }
    }
}

/// Is a place a job? With worker threads (`background_jobs`), or where the session has none and
/// steps the place itself a slice per frame (`cooperative_jobs`, the web build). Every other
/// command goes by `background_jobs` alone: threads are what save, print and the rest need.
pub(crate) fn place_in_background(app: &PhotocraftApp) -> bool {
    app.background_jobs || app.session.cooperative_jobs()
}

/// Place a file's bytes in the background (File › Place for a file with no path to read, a dropped
/// file read by the browser): like [`run`] for `file.placeEmbedded`, the result is
/// `{"job": id, "pending": true}` and the layer lands when [`tick`] applies the job. `linked`
/// makes it Place Linked to that path. Hand the result to [`track_place`].
pub(crate) fn start_place_bytes(app: &mut PhotocraftApp, name: &str, bytes: Vec<u8>, linked: Option<String>, params: &Value) -> Result<Value, String> {
    if app.jobs.place.is_some() {
        return Err(tl!("A file is still being placed; try again when it has finished.").into());
    }
    match app.session.start_place_bytes(name, bytes, linked, params.clone()).map_err(|e| e.to_string())? {
        Started::Done(v) => Ok(v),
        Started::Job(job) => {
            app.jobs.last_started = Some(job);
            // The status bar's progress readout names the job; clear any old message.
            app.ui.status.clear();
            app.ui.status_error = false;
            Ok(json!({"job": job.0, "pending": true}))
        }
    }
}

/// Follow the place that `started` (the result of [`run`] or [`start_place_bytes`] for the file
/// `name`) began in the background: when it lands, Free Transform starts on its layer and the
/// next dropped file is placed (see `on_event`). False when it already ran (nothing to follow).
pub(crate) fn track_place(app: &mut PhotocraftApp, name: &str, started: &Value) -> bool {
    let job = started.get("pending").and_then(Value::as_bool).filter(|pending| *pending).and_then(|_| started.get("job")?.as_u64());
    let (Some(job), Some(doc)) = (job, app.session.active().map(|st| st.doc.id)) else { return false };
    app.jobs.place = Some(PlaceJob { job: JobId(job), name: crate::file_open::display_name(name), doc });
    true
}

/// A background place ended: start Free Transform on the layer it made, or report why not, then
/// go on with the files dropped meanwhile.
fn finish_place(app: &mut PhotocraftApp, ctx: &egui::Context, place: PlaceJob, outcome: JobOutcome) {
    app.sync_views();
    match outcome {
        JobOutcome::Done(result) => {
            notices::io_result_warnings(app, &result, &place.name);
            // The tab can change while the file is placed: the box opens only on the document it
            // was placed in, and never over another one. An image placed into a frame has none.
            if app.ui.transform.is_none()
                && result.get("frame").and_then(Value::as_u64).is_none()
                && app.session.active().is_some_and(|st| st.doc.id == place.doc)
                && let Err(e) = crate::transform_tool::begin_placed(app, ctx)
            {
                app.ui.status = e;
            }
        }
        JobOutcome::Failed(err) => app.place_failed(&place.name, &err),
        JobOutcome::Cancelled if app.session.documents().iter().any(|st| st.doc.id == place.doc) => {
            app.ui.status = crate::i18n::fmt(tl!("Cancelled placing {name}"), &[("name", &place.name)]);
            app.ui.status_error = false;
        }
        JobOutcome::Cancelled => {}
    }
    app.place_next_dropped(ctx);
}

/// Open a file in the background: a tab appears at once and shows the progress; the document
/// replaces it when decoded. `path` is remembered for File › Save and Open Recent. `slot` (a file
/// dropped on the tabs) and `order` (its place in a set opened together) survive an inline
/// completion, a memory prompt and the open itself, so the tab lands where it belongs either way.
pub fn start_open(
    app: &mut PhotocraftApp,
    name: &str,
    path: Option<String>,
    source: OpenSource,
    slot: Option<usize>,
    order: Option<OpenOrder>,
) -> Result<(), String> {
    let name = &app.open_name(name);
    match app.session.start_open_for_user_with_background(name, source, app.background_jobs).map_err(|e| e.to_string())? {
        // Inline (wasm, or background jobs off): finish now, like a background open that ended at
        // once. A preflight keeps the slot and set order in its memory prompt.
        Started::Done(v) => finish_open(app, name, path.as_deref(), &v, slot, order),
        Started::Job(job) => {
            track_open(app, OpenTab { job, name: name.to_string(), path, slot, order });
            Ok(())
        }
    }
}

/// A file opening in the background gets its tab: focused, with the status line naming it.
fn track_open(app: &mut PhotocraftApp, tab: OpenTab) {
    app.ui.status = crate::i18n::fmt(tl!("Opening {name}…"), &[("name", &tab.name)]);
    app.jobs.focus = Some(tab.job);
    app.jobs.opens.push(tab);
    app.ui.chrome.home = None;
    app.ui.status_error = false;
}

/// File › Open as Smart Object… for one file: `file.openAsSmartObject` as a tracked open job, so
/// the result goes through File › Open's completion ([`finish_open`]: the profile prompt, the
/// Camera Raw open dialog for a raw, the import warnings). The document is untitled, so
/// Open Recent is untouched. Inline (background jobs off, or the web) it finishes at once.
pub(crate) fn start_open_as_smart_object(app: &mut PhotocraftApp, path: &str, order: Option<OpenOrder>) -> Result<(), String> {
    let name = crate::file_open::display_name(path);
    // `confirmDamage`: a damaged PSD waits for the user's answer, as in File › Open.
    let params = json!({"path": path, "confirmDamage": true});
    let started = if app.background_jobs {
        app.session.start("file.openAsSmartObject", params).map_err(|e| e.to_string())?
    } else {
        Started::Done(app.session.execute("file.openAsSmartObject", params).map_err(|e| e.to_string())?)
    };
    match started {
        Started::Done(v) => finish_open(app, &name, None, &v, None, order),
        Started::Job(job) => {
            track_open(app, OpenTab { job, name, path: None, slot: None, order });
            Ok(())
        }
    }
}

/// Start a set of files opened together: the `batch` of their [`OpenOrder`]s. Its tabs follow the
/// documents and opens already there. Call [`end_batch`] once every member has been started.
pub(crate) fn begin_batch(app: &mut PhotocraftApp) -> u64 {
    let id = app.jobs.next_batch;
    app.jobs.next_batch = app.jobs.next_batch.wrapping_add(1);
    let base = app.session.documents().len() + app.jobs.opens.len();
    app.jobs.batches.push(OpenBatch { id, base, opened: Vec::new() });
    id
}

/// A member of a set opened at once (not in the background): its document is in place at the
/// end, so later members go after it.
#[cfg(test)]
pub(crate) fn opened_in_order(jobs: &mut JobsUi, order: OpenOrder) {
    if let Some(batch) = jobs.batches.iter_mut().find(|b| b.id == order.batch) {
        batch.opened.push(order.seq);
    }
}

/// Every member of the set has been started: forget it unless some are still opening, waiting on
/// their prompt (memory, profile or damage), or waiting for a prompt to clear.
pub(crate) fn end_batch(app: &mut PhotocraftApp) {
    let waiting: Vec<u64> =
        app.held_memory_prompts.iter().filter_map(|m| m.order.map(|o| o.batch)).chain(app.held_opens.iter().filter_map(|h| h.order.map(|o| o.batch))).collect();
    let JobsUi { opens, batches, .. } = &mut app.jobs;
    batches.retain(|b| opens.iter().any(|o| o.order.is_some_and(|x| x.batch == b.id)) || waiting.contains(&b.id));
}

/// The tab position for a member of a set that just opened: after the members before it that
/// have opened, so the set reads in order whichever order the opens finish in.
fn batch_slot(jobs: &mut JobsUi, order: OpenOrder) -> Option<usize> {
    let batch = jobs.batches.iter_mut().find(|b| b.id == order.batch)?;
    let slot = batch.base + batch.opened.iter().filter(|s| **s < order.seq).count();
    batch.opened.push(order.seq);
    Some(slot)
}

/// Where a finished open puts its tab: an explicit slot (a file dropped on the tabs) wins;
/// otherwise its place in a set opened together, if that set is still tracked.
pub(crate) fn tab_slot(app: &mut PhotocraftApp, slot: Option<usize>, order: Option<OpenOrder>) -> Option<usize> {
    let in_set = order.and_then(|o| batch_slot(&mut app.jobs, o));
    slot.or(in_set)
}

/// A background open's result (`{document, warnings, color}`) for `tab`, already taken out of
/// [`JobsUi::opens`]: the document goes to the tab's slot, or to its place in its set. A memory
/// preflight keeps both in its prompt.
pub(crate) fn finish_open_tab(app: &mut PhotocraftApp, tab: &OpenTab, v: &Value) -> Result<(), String> {
    let r = finish_open(app, &tab.name, tab.path.as_deref(), v, tab.slot, tab.order);
    end_batch(app);
    r
}

/// Save the active document to `path` in the background: `save` encodes and writes it on a
/// worker while the window keeps drawing and shows the progress dialog. The document is locked
/// until the job ends, which [`tick`] records like a synchronous save. `Ok(None)`: started;
/// `Ok(Some(..))`: it ran inline (wasm) and is already recorded.
pub fn start_save(
    app: &mut PhotocraftApp,
    path: String,
    settings: ExportSettings,
    copy: bool,
    save: SaveFileFn,
    automation: bool,
) -> Result<Option<(String, Vec<String>)>, String> {
    let st = app.session.active().ok_or("no document")?;
    if let Some(j) = app.session.job_on(st.doc.id) {
        // The engine's wording for a command refused while a job runs.
        return Err(format!("“{}” is still running on this document; wait for it to finish or cancel it (Esc)", j.label));
    }
    crate::app::ensure_memory_to_save(&st.doc, &path)?;
    let name = crate::file_open::display_name(&path);
    // One writer per file: an older snapshot must not land after a newer one.
    if app.jobs.saves.iter().any(|s| s.path == path) {
        return Err(crate::i18n::fmt(tl!("A save to {name} is still running; try again when it finishes"), &[("name", &name)]));
    }
    let (snapshot, doc, revision) = (Arc::clone(&st.doc), st.doc.id, st.revision);
    let label = crate::i18n::fmt(tl!("Saving {name}"), &[("name", &name)]);
    let target = path.clone();
    let result_path = path.clone();
    let state = Arc::new(SaveState::default());
    let worker = Arc::clone(&state);
    let started = app.session.start_job(
        SAVE_JOB,
        json!({"path": path}),
        &label,
        true,
        move |ctx| {
            let _done = WorkerDone(worker);
            let ctl = SaveCtl { job: ctx.clone() };
            save(&snapshot, &target, &settings, &ctl)
                .map_err(|e| if ctl.check().is_err() { photocraft_engine::EngineError::Cancelled } else { photocraft_engine::EngineError::Other(e) })
        },
        move |_, warnings: Vec<String>| Ok(json!({"path": result_path, "warnings": warnings})),
    );
    match started.map_err(|e| e.to_string())? {
        Started::Done(v) => {
            let warnings = warnings_of(&v);
            if automation {
                app.saved_automation(doc, revision, &path, &warnings, copy);
            } else {
                app.saved(doc, revision, &path, &warnings, copy);
            }
            Ok(Some((path, warnings)))
        }
        Started::Job(job) => {
            // Preferences › File Handling › Save in Background, read when the save starts.
            let foreground = !app.session.prefs().file_handling.save_in_background;
            app.jobs.saves.push(SaveJob { job, doc, revision, path, copy, automation, state, ended: false, foreground });
            // Control and MCP requests wait on it, as for other jobs.
            app.jobs.last_started = Some(job);
            // The status bar's progress readout names the save; clear any old message.
            app.ui.status.clear();
            app.ui.status_error = false;
            Ok(None)
        }
    }
}

fn warnings_of(v: &Value) -> Vec<String> {
    v.get("warnings").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

/// A background save ended: record it, or report why not. The close prompt waiting on it moves
/// on, or asks again.
fn finish_save(app: &mut PhotocraftApp, ctx: &egui::Context, save: SaveJob, label: &str, outcome: JobOutcome) {
    match outcome {
        JobOutcome::Done(v) => {
            if save.automation {
                app.saved_automation(save.doc, save.revision, &save.path, &warnings_of(&v), save.copy);
            } else {
                app.saved(save.doc, save.revision, &save.path, &warnings_of(&v), save.copy);
            }
            if !save.copy {
                crate::discard_ui::saved_document(app, ctx, save.doc);
            }
        }
        JobOutcome::Failed(err) => {
            // Don't close the window over the error.
            app.jobs.close_after_saves = false;
            notices::error(app, crate::i18n::fmt(tl!("Save failed: {error}"), &[("error", &err)]));
        }
        JobOutcome::Cancelled => {
            app.jobs.close_after_saves = false;
            app.ui.status = crate::i18n::fmt(tl!("Canceled {label}"), &[("label", label)]);
            app.ui.status_error = false;
        }
    }
    close_after_saves(app, ctx);
}

/// Forget saves whose job ended and whose worker has returned; repeat a held window close once
/// no save worker is left.
fn close_after_saves(app: &mut PhotocraftApp, ctx: &egui::Context) {
    app.jobs.saves.retain(|s| !(s.ended && s.state.finished()));
    if !app.saving() && !app.printing() && std::mem::take(&mut app.jobs.close_after_saves) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

/// Per frame: apply finished jobs and react to them; keep frames coming while jobs run; Esc
/// cancels the job in view.
pub fn tick(app: &mut PhotocraftApp, ctx: &egui::Context) {
    for e in app.session.poll_jobs() {
        on_event(app, ctx, e);
    }
    // A focused open tab that ended (or was cancelled elsewhere) gives way to the documents.
    if app.jobs.focus.is_some_and(|j| !app.jobs.opens.iter().any(|o| o.job == j)) {
        app.jobs.focus = app.jobs.opens.last().map(|o| o.job);
    }
    close_after_saves(app, ctx);
    if app.saving() || app.printing() {
        // Keep checking on a cancelled save's worker until it returns.
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
    if !keep_polling(app, ctx) {
        return;
    }
    // Once per frame: Escape is taken from the frame's input, so a second pass finds none. While the Batch question is
    // open Escape is its prompt's (it answers Stop), so it is left in the input.
    if let Some(job) = job_in_view(app)
        && !craft::ui::runtime::focus::focused_rec(ctx).is_some_and(|r| r.stop.text)
        && !(job.command == "file.automate.batch" && app.session.batch_interrupt_waiting(job.id))
        && crate::window_keys::take_escape(ctx)
    {
        if job.command == "file.automate.batch" {
            app.session.request_batch_interrupt_job(job.id);
        } else {
            cancel(app, job.id);
        }
    }
}

/// Keep the event loop awake while a background document job exists.
///
/// Jobs can be started by a menu or dialog while the current frame is being drawn, after
/// [`tick`] has already run. Calling this from the shell after drawing avoids leaving such a job
/// unapplied on an otherwise idle event loop (notably Wayland): its document would remain locked,
/// making Undo and Redo appear unavailable.
pub fn keep_polling(app: &PhotocraftApp, ctx: &egui::Context) -> bool {
    if !app.session.has_jobs() {
        return false;
    }
    // Animate progress and poll for results at display rate; the UI thread only draws.
    ctx.request_repaint_after(std::time::Duration::from_millis(16));
    true
}

/// The save the app is waiting on, while it runs: one started with Preferences › File Handling ›
/// Save in Background off (#6207). Like Photoshop's foreground save it blocks the app: the modal
/// progress dialog shows at once ([`dialog`]) and no shortcut or tool reaches the documents
/// until the file is written; anything after it (a close prompt, quitting) waits for it.
pub fn foreground_save(app: &PhotocraftApp) -> Option<JobInfo> {
    app.jobs.saves.iter().filter(|s| s.foreground && !s.ended).find_map(|s| app.session.job(s.job))
}

/// The job the user is looking at: a foreground save, the open tab in view, the active
/// document's job, or (for the status bar) the newest running job.
pub fn job_in_view(app: &PhotocraftApp) -> Option<JobInfo> {
    if let Some(j) = foreground_save(app) {
        return Some(j);
    }
    if let Some(j) = app.jobs.focus.and_then(|j| app.session.job(j)) {
        return Some(j);
    }
    app.session.active_job().or_else(|| app.session.jobs().pop())
}

/// Cancel job `id` (the status bar's ×, the dialog's Cancel, Esc, closing an opening tab).
pub fn cancel(app: &mut PhotocraftApp, id: JobId) -> bool {
    try_cancel(app, id).is_ok()
}

/// [`cancel`], saying why a job was left running.
pub fn try_cancel(app: &mut PhotocraftApp, id: JobId) -> Result<(), CancelRefused> {
    let label = app.session.job(id).map(|j| j.label);
    let r = app.session.try_cancel_job(id);
    // A save that has started replacing the file runs to the end: cancelling it now would leave
    // the file written but the document marked unsaved.
    if r == Err(CancelRefused::Committing) {
        app.ui.status = crate::i18n::fmt(tl!("{label} is writing the file and can no longer be canceled"), &[("label", label.as_deref().unwrap_or_default())]);
        app.ui.status_error = false;
    }
    if r.is_ok() {
        // The event (and its status text) arrives with the next poll; say so now as well.
        if let Some(l) = label {
            app.ui.status = crate::i18n::fmt(tl!("Canceled {label}"), &[("label", &l)]);
            app.ui.status_error = false;
        }
    }
    r
}

/// Handles one finished job: optional completion beep, waiting control replies, and the
/// command-specific open, save, transform and recovery paths.
fn on_event(app: &mut PhotocraftApp, ctx: &egui::Context, e: JobEvent) {
    let cancelled_batch =
        e.command == "file.automate.batch" && matches!(&e.outcome, JobOutcome::Done(result) if result.get("cancelled").and_then(Value::as_bool) == Some(true));
    // Preferences ▸ General ▸ Beep When Done (off by default): a finished operation beeps, like
    // Photoshop's cue. Cancelling is the user's own action and stays silent.
    if matches!(e.outcome, JobOutcome::Done(_))
        && !cancelled_batch
        && app.session.prefs().general.beep_when_done
        && let Some(beep) = app.services.beep
    {
        beep();
    }
    // Control requests waiting on this job; a cancelled batch reports the files it finished.
    #[cfg(not(target_arch = "wasm32"))]
    {
        use craft::automation::control::result_reply;
        let reply = match &e.outcome {
            JobOutcome::Done(v) if cancelled_batch => json!({"ok": false, "error": "cancelled", "result": v}),
            JobOutcome::Done(v) => result_reply(Ok(v.clone())),
            JobOutcome::Failed(err) => result_reply(Err(err.clone())),
            JobOutcome::Cancelled => result_reply(Err("cancelled".into())),
        };
        app.reply_job(e.id, &reply);
    }
    // A Free Transform commit's session ends with its job (then reported like any job).
    crate::transform_tool::on_job_event(app, &e);
    if crate::prefs_ui::on_recovery_event(app, &e) {
        return;
    }
    // Camera Raw's open-time re-develop and its final step.
    if crate::camera_raw_ui::on_redevelop_event(app, &e) {
        return;
    }
    if app.jobs.place.as_ref().is_some_and(|p| p.job == e.id)
        && let Some(place) = app.jobs.place.take()
    {
        finish_place(app, ctx, place, e.outcome);
        return;
    }
    if e.command == SAVE_JOB
        && let Some(i) = app.jobs.saves.iter().position(|s| s.job == e.id)
    {
        // A cancelled save's worker may still be running: keep it counted until it returns.
        let save = if app.jobs.saves[i].state.finished() {
            app.jobs.saves.remove(i)
        } else {
            app.jobs.saves[i].ended = true;
            app.jobs.saves[i].clone()
        };
        finish_save(app, ctx, save, &e.label, e.outcome);
        return;
    }
    if e.command == OPEN_JOB || app.jobs.opens.iter().any(|o| o.job == e.id) {
        let Some(i) = app.jobs.opens.iter().position(|o| o.job == e.id) else { return };
        let tab = app.jobs.opens.remove(i);
        match e.outcome {
            JobOutcome::Done(v) => {
                if let Err(err) = finish_open_tab(app, &tab, &v) {
                    app.open_failed(&tab.name, &err);
                }
            }
            JobOutcome::Failed(err) => app.open_failed(&tab.name, &err),
            JobOutcome::Cancelled => {
                app.ui.status = crate::i18n::fmt(tl!("Canceled opening {name}"), &[("name", &tab.name)]);
                app.ui.status_error = false;
            }
        }
        end_batch(app);
        return;
    }
    match e.outcome {
        JobOutcome::Done(v) if e.command == "brush.presets.importAbr" => {
            let n = v.get("count").and_then(Value::as_u64).unwrap_or(0);
            let group = v.get("group").and_then(Value::as_str).unwrap_or_default();
            app.ui.status = crate::i18n::fmt(
                &crate::i18n::trn(crate::i18n::current(), n, "Imported {n} brush from {group}", "Imported {n} brushes from {group}"),
                &[("group", group)],
            );
            app.ui.status_error = false;
            // The import's result is in the Brushes panel.
            let _ = crate::panels::panel_docking::show_panel(app, "brushes", Some(true));
        }
        JobOutcome::Done(ref result) if matches!(e.command.as_str(), "file.print" | "file.printOneCopy") => {
            app.sync_views();
            crate::file_ui::print_status(app, result);
        }
        JobOutcome::Done(_) if cancelled_batch => {
            app.ui.status = crate::i18n::fmt(tl!("Canceled {label}"), &[("label", &e.label)]);
            app.ui.status_error = false;
        }
        JobOutcome::Done(_) => {
            app.sync_views();
            app.ui.status = e.label;
            app.ui.status_error = false;
        }
        JobOutcome::Failed(err) => {
            if matches!(e.command.as_str(), "file.print" | "file.printOneCopy") {
                app.jobs.close_after_saves = false;
            }
            app.sync_views();
            notices::error(app, format!("{}: {err}", e.label));
        }
        JobOutcome::Cancelled => {
            app.ui.status = crate::i18n::fmt(tl!("Canceled {label}"), &[("label", &e.label)]);
            app.ui.status_error = false;
        }
    }
}

/// What [`crate::PhotocraftApp::open_bytes`] does after decoding, for a background open's result
/// (`{document, docId, warnings, color}` or `{held, …}`). `slot` and `order` are the tab position
/// and set place the file keeps: through a memory prompt, a profile/damage prompt and the final
/// document, so an inline completion lands where a background tab would.
fn finish_open(app: &mut PhotocraftApp, name: &str, path: Option<&str>, v: &Value, slot: Option<usize>, order: Option<OpenOrder>) -> Result<(), String> {
    if let Some(preflight_id) = v.get("pendingPreflight").and_then(Value::as_u64) {
        let estimated = v.get("estimated").and_then(Value::as_u64).unwrap_or(0);
        let available = v.get("availableRam").and_then(Value::as_u64);
        let capacity_unknown = v.get("capacityUnknown").and_then(Value::as_bool).unwrap_or(available.is_none());
        let uncertain_capacity_warning = v.get("uncertainCapacityWarning").and_then(Value::as_bool).unwrap_or(false);
        crate::open_prompt::arrive_memory(
            app,
            preflight_id,
            name.to_string(),
            path.map(str::to_string),
            slot,
            order,
            estimated,
            available,
            capacity_unknown,
            uncertain_capacity_warning,
        );
        return Ok(());
    }
    let warnings: Vec<String> =
        v.get("warnings").and_then(Value::as_array).map(|a| a.iter().filter_map(|w| w.as_str().map(str::to_string)).collect()).unwrap_or_default();
    let held = |doc, read_only, question, policy_applied| crate::open_prompt::Held {
        doc,
        read_only,
        damage_pending: v.get("damaged").and_then(Value::as_bool).unwrap_or(false),
        name: name.to_string(),
        path: path.map(str::to_string),
        warnings: warnings.clone(),
        bytes: None,
        slot,
        order,
        question,
        policy_applied,
        smart_object: v.get("smartObject").and_then(Value::as_bool).unwrap_or(false),
    };
    // Held back for the profile prompt (Ask When Opening).
    if let Some(id) = v.get("held").and_then(Value::as_u64) {
        let (doc, read_only) = app.session.take_held_open(id).ok_or("the open job's document is gone")?;
        crate::open_prompt::arrive(app, held(doc, read_only, v.get("color").filter(|q| !q.is_null()).cloned(), false));
        return Ok(());
    }
    // poll_jobs applies all ready jobs before the shell handles their events. An earlier
    // handler may have moved or closed tabs, so the apply-time index is no longer authority.
    let document = v.get("docId").and_then(Value::as_u64).map(DocId).ok_or("the open job returned no document identity")?;
    let index = app.session.documents().iter().position(|st| st.doc.id == document).ok_or("the open job's document is missing")?;
    // An earlier file is waiting on its prompt: this document waits after it, in order.
    if crate::open_prompt::waiting(app)
        && let Some(st) = app.session.close(index)
    {
        let read_only = st.source_read_only;
        let doc = Arc::try_unwrap(st.doc).unwrap_or_else(|d| (*d).clone());
        crate::open_prompt::arrive(app, held(doc, read_only, None, true));
        return Ok(());
    }
    if let Some(slot) = tab_slot(app, slot, order) {
        let _ = app.run("document.move", json!({"document": index, "to": slot}));
    }
    after_open(app, document, name, path, &warnings, None)
}

/// What every open does for its admitted document: remember where it came from, say
/// so, show the import warnings (a raw opens in Camera Raw first, developed from `path` or
/// `bytes`) and fire the "Open Document" script events.
pub(crate) fn after_open(
    app: &mut PhotocraftApp,
    document: DocId,
    name: &str,
    path: Option<&str>,
    warnings: &[String],
    bytes: Option<&Arc<Vec<u8>>>,
) -> Result<(), String> {
    let index = app.session.documents().iter().position(|st| st.doc.id == document).ok_or("the opened document is missing")?;
    app.session.set_active(index);
    if let Some(p) = path {
        app.opened_from(p);
    }
    app.sync_views();
    // Fonts the document's type uses that aren't installed join the import warnings (#4559).
    let warnings = crate::file_open::with_missing_fonts(app, warnings.to_vec());
    let warnings = warnings.as_slice();
    let opened = crate::i18n::fmt(tl!("Opened {name}"), &[("name", name)]);
    app.ui.status = opened.clone();
    app.ui.status_error = false;
    // A raw opens in Camera Raw first; the dialog says how it was developed.
    if crate::camera_raw_ui::wants_open_dialog(app, warnings) {
        crate::camera_raw_ui::queue_open_dialog(app, document, name, path, bytes, warnings)?;
        notices::io_warnings(app, &opened, &crate::camera_raw_ui::without_develop_note(warnings));
    } else {
        notices::io_warnings(app, &opened, warnings);
        // A raw that opened without the dialog is developed by its sidecar / Camera Raw Defaults.
        if let Some(index) = app.session.active_index() {
            crate::camera_raw_ui::opened_without_dialog(app, index, name, path, warnings);
        }
    }
    // Script events bound to "Open Document".
    photocraft_engine::automate_cmds::document_opened(&mut app.session);
    app.sync_views();
    Ok(())
}

// ------------------------------------------------------------------------------------- drawing

/// Progress that has been reported, or `None` (indeterminate) while it's still 0.
fn shown_fraction(j: &JobInfo) -> Option<f32> {
    (j.progress > 0.0).then_some(j.progress)
}

fn percent(j: &JobInfo) -> String {
    // Nothing yet reported: the bar is indeterminate, so no number.
    if j.progress > 0.0 { whole_percent(j.progress) } else { String::new() }
}

/// "45%": rounded down, so a job reads 100% only once it's done.
fn whole_percent(fraction: f32) -> String {
    format!("{:.0}%", (fraction.clamp(0.0, 1.0) * 100.0).floor())
}

/// A save's progress as Photoshop words it in the status bar and the tab title: "Saving 45%"
/// (#3389).
fn saving_percent(j: &JobInfo) -> String {
    crate::i18n::fmt(tl!("Saving {percent}"), &[("percent", &whole_percent(j.progress))])
}

/// "Saving 45%" while a save of document `doc` runs, for its tab title; `None` otherwise.
pub fn save_progress(app: &PhotocraftApp, doc: DocId) -> Option<String> {
    let save = app.jobs.saves.iter().find(|s| s.doc == doc && !s.ended)?;
    app.session.job(save.job).map(|j| saving_percent(&j))
}

/// The status bar's job readout (the bar's trailing group): the job's label, a progress bar, the percentage and a
/// cancel button, "Saving" for a save. A print's cancel is shown refused: the system print queue owns it. Draws
/// nothing when no job runs.
pub fn status_progress(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let Some(j) = job_in_view(app) else { return };
    if j.command == "file.automate.batch" {
        return;
    }
    let saving = app.jobs.saves.iter().any(|s| s.job == j.id);
    let more = app.session.jobs().len().saturating_sub(1);
    // A save's percentage is the component's own.
    let name = if saving { tl!("Saving").to_owned() } else { tl!(&j.label).to_owned() };
    let label = if more > 0 { format!("{name} (+{more})") } else { name };
    let mut progress = StatusProgress::new(label, shown_fraction(&j)).cancel(tl!("Cancel (Esc)").to_owned());
    // A print has already reached the spool worker; cancellation belongs to the system queue.
    if matches!(j.command.as_str(), "file.print" | "file.printOneCopy") {
        progress = progress.cancel_refused(tl!("Manage submitted jobs in the system print queue").to_owned());
    }
    if progress.show(ui).cancelled {
        cancel(app, j.id);
    }
}

/// Show the folder Batch worker's immutable current document in the canvas area. The worker
/// publishes snapshots rather than adding them to `Session`, so no edit or Save can target one.
pub(crate) fn batch_preview(app: &mut PhotocraftApp, ui: &mut egui::Ui) -> bool {
    let current = app
        .session
        .jobs()
        .into_iter()
        .filter(|job| job.command == "file.automate.batch")
        .find_map(|job| app.session.batch_preview(job.id).map(|preview| (job.id, preview)));
    let Some((id, preview)) = current else { return false };
    let name = std::path::Path::new(&preview.path).file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| preview.path.clone());
    Row::new().show(ui, |ui, _| {
        Text::new(name.clone()).role(TypeRole::BodyStrong).show(ui);
        Text::new(tl!("Batch preview").to_owned()).shade(Shade::Faint).show(ui);
    });
    // The store keeps the texture under the job and the snapshot's generation, so it decodes once per input. The
    // thumbnail is flattened onto white, as the canvas shows a transparent document.
    let image = || {
        let thumb = photocraft_compose::thumbnail(&preview.document, 1024);
        let flat: Vec<u8> = thumb
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|px| {
                let alpha = u32::from(px[3]);
                let over_white = |c: u8| ((u32::from(c) * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
                [over_white(px[0]), over_white(px[1]), over_white(px[2]), 255]
            })
            .collect();
        egui::ColorImage::from_rgba_unmultiplied([thumb.width as usize, thumb.height as usize], &flat)
    };
    Picture::new(name, id.0, &image).revision(Revision::new(preview.generation)).sizing(Sizing::Fill).show(ui);
    true
}

/// The modal progress window for a job locking the active document (Photoshop shows one for
/// long filters): title, status, bar, percentage and elapsed time, Cancel. Appears after
/// [`DIALOG_DELAY_MS`]; Esc cancels (see [`tick`]). A save shows it only when it runs in the
/// foreground (Save in Background off), and then at once, so nothing can be clicked meanwhile; a
/// background save shows its progress in the tab and the status bar instead.
pub fn dialog(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if let Some(job) = app.session.jobs().into_iter().find(|job| job.command == "file.automate.batch" && app.session.batch_interrupt_waiting(job.id)) {
        ask_batch_interrupt(app, job.id);
        return;
    }
    let j = match foreground_save(app) {
        Some(j) => j,
        None => {
            let Some(j) = app.session.active_job() else { return };
            // A save shows its progress in the tab and the status bar; so does a place, which
            // leaves the view free to zoom and pan while the file is decoded.
            // PLAN: the screen-mode and status-bar visibility readers panels-w19 gives are not in the tree; the base reads stay.
            let place = place_uses_status_bar(
                &j.command,
                !app.ui.view.hides_chrome() && app.ui.panels.status_bar && job_in_view(app).is_some_and(|shown| shown.id == j.id),
            );
            if j.command == "file.automate.batch" || j.elapsed_ms < DIALOG_DELAY_MS || place || app.jobs.saves.iter().any(|s| s.job == j.id) {
                return;
            }
            j
        }
    };
    // Shown every pass the job lasts: the open state is this pass's, so a Cancel (or Escape) reads as a close and a
    // later job opens a fresh card.
    let mut open = true;
    let percent_text = percent(&j);
    let out = BlockingOverlay::new(tl!(&j.label).to_owned()).opened(Bind::bound(&mut open)).card(true).cancel(tl!("Cancel").to_owned()).show(ui, |ui, _| {
        Text::new(tl!(&j.label).to_owned()).role(TypeRole::BodyStrong).show(ui);
        let message = if j.message.is_empty() || j.message == j.label { tl!("Working…").to_owned() } else { j.message.clone() };
        Text::new(message).shade(Shade::Muted).show(ui);
        let bar = match shown_fraction(&j) {
            Some(fraction) => Progress::new(fraction),
            None => Progress::indeterminate(),
        };
        bar.label(tl!(&j.label).to_owned()).sizing(Sizing::FillWidth).show(ui);
        Row::new().align(RowAlign::SpaceBetween).fill_width().show(ui, |ui, _| {
            Text::new(percent_text.clone()).shade(Shade::Faint).show(ui);
            Text::new(elapsed(j.elapsed_ms)).shade(Shade::Faint).show(ui);
        });
        Text::new(tl!("Esc to cancel").to_owned()).shade(Shade::Faint).show(ui);
    });
    // A click outside the card does not cancel the work. A second pass in a frame reports no close (the overlay
    // applies only its own input), and `cancel` is idempotent besides.
    if matches!(out.closed, Some(CloseReason::Requested | CloseReason::Escape)) {
        cancel(app, j.id);
    }
}

/// Tag of the Batch question: the job it decides.
struct BatchJob(JobId);

/// Photoshop's question after Esc interrupts one input. The worker is parked before its save;
/// Continue advances the same run to the next input, and Stop returns its partial result. Escape answers Stop.
/// Opened once per question: while it is open, later passes leave it alone.
fn ask_batch_interrupt(app: &mut PhotocraftApp, id: JobId) {
    let prompt = StandardPrompt::custom(tl!("A command was canceled or stopped during batch processing. Continue with the next file or stop?").to_owned())
        .title(tl!("Batch").to_owned())
        .button(PromptChoice::Proceed, tl!("Continue").to_owned())
        .button(PromptChoice::Cancel, tl!("Stop").to_owned())
        .tag(BatchJob(id));
    let _ = app.framework.dialogs.open_unless_open(prompt);
}

/// The answer to the Batch question (a `PROMPTS` row): Continue goes on to the next input, anything else stops the
/// run. `false` for another prompt's answer.
pub(crate) fn answer(app: &mut PhotocraftApp, _ctx: &egui::Context, answer: &mut PromptAnswer) -> bool {
    let Some(BatchJob(id)) = answer.take_tag::<BatchJob>() else { return false };
    app.session.decide_batch_interrupt_job(id, answer.choice == PromptChoice::Proceed);
    true
}

fn elapsed(ms: f64) -> String {
    let s = (ms / 1000.0).max(0.0);
    if s < 60.0 { format!("{s:.1} s") } else { format!("{}:{:02}", (s / 60.0).floor() as u64, (s % 60.0).floor() as u64) }
}

/// The canvas area of an opening tab: the file name, the job's message, a progress bar and Cancel, centred in the
/// area. The card's layout (one line for the name, Cancel inside) is `OpeningCard`'s.
pub fn open_card(app: &mut PhotocraftApp, ui: &mut egui::Ui, job: JobId) {
    let Some(tab) = app.jobs.opens.iter().find(|o| o.job == job) else { return };
    let Some(j) = app.session.job(job) else { return };
    let message = if j.message.is_empty() { tl!("Reading…").to_owned() } else { format!("{}…", j.message) };
    let out = OpeningCard::new(tab.name.clone()).message(message).fraction(shown_fraction(&j)).cancel(tl!("Cancel")).show(ui);
    if out.cancelled {
        cancel(app, job);
    }
}

/// Open tabs for the tab strips: (job, title, progress); `None` shows an indeterminate tab until the job reports.
pub fn open_tabs(app: &PhotocraftApp) -> Vec<(JobId, String, Option<f32>)> {
    app.jobs.opens.iter().map(|o| (o.job, o.name.clone(), app.session.job(o.job).and_then(|j| shown_fraction(&j)))).collect()
}

/// Jobs for `ui.inspect`.
pub fn inspect(app: &PhotocraftApp) -> Value {
    json!({
        "running": app.session.jobs(),
        "opening": app.jobs.opens.iter().map(|o| json!({"job": o.job.0, "name": o.name, "path": o.path})).collect::<Vec<_>>(),
        "saving": app.jobs.saves.iter().map(|s| json!({"job": s.job.0, "path": s.path})).collect::<Vec<_>>(),
        "placing": app.jobs.place.as_ref().map(|p| json!({"job": p.job.0, "name": p.name})),
        "focus": app.jobs.focus.map(|j| j.0),
        "background": app.background_jobs,
    })
}

/// Bytes shared with a background open without copying them again.
pub fn bytes(b: &[u8]) -> OpenSource {
    OpenSource::Bytes(Arc::new(b.to_vec()))
}

#[cfg(test)]
mod tests;

/// Placements keep the canvas accessible when progress and Cancel are visible in the status bar.
fn place_uses_status_bar(command: &str, status_bar_visible: bool) -> bool {
    status_bar_visible && matches!(command, "file.placeEmbedded" | "file.placeLinked")
}
