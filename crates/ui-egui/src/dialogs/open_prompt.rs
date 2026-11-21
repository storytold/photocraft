//! Interactive opening confirmations: damaged PSD data and Color Settings questions.
//! A decoded file is held back while its standard, agent-accessible dialog asks what to do.
//! Damage confirmation comes first; Continue admits the file normally once any profile question
//! is answered, with no persistent save restriction. Cancelling damage discards just that file;
//! cancelling a profile question retains its existing behaviour of cancelling the whole queue.
//! Files that finish reading while a prompt is up wait their turn.

use std::sync::Arc;

use craft::color_ui::ui::{ProfileAction, ProfileMismatchDialog, ProfileMode, ProfileQuestion};
use craft::ui::dialog::{DialogResult, PromptAnswer, PromptChoice, StandardPrompt};
use craft::ui::runtime::env::Label;
use photocraft_doc::{ColorMode, Document};
use serde_json::{Value, json};

use super::alerts;
use crate::PhotocraftApp;

/// Tags the damaged-file prompt: the front of `held_opens` is the damaged file.
pub(crate) struct DamageTag;

/// Tags a high-memory prompt with the preflight it asks about, so an answer that arrives after the
/// queue moved on is not applied to another file.
pub(crate) struct MemoryTag {
    /// The `MemoryPrompt::id` the prompt was opened for.
    pub(crate) id: u64,
}

/// A file read and decoded, waiting to open.
pub struct Held {
    pub doc: Document,
    /// The source is read-only (an Affinity file): Save doesn't write back to it.
    pub read_only: bool,
    /// Ask whether to continue before adding a damaged PSD or showing its profile prompt.
    /// Discarded after the answer: it does not become a document or save restriction.
    pub damage_pending: bool,
    pub name: String,
    pub path: Option<String>,
    pub warnings: Vec<String>,
    /// The file's bytes, for Camera Raw to develop from when there is no path.
    pub bytes: Option<Arc<Vec<u8>>>,
    /// The tab position a reopened file keeps.
    pub slot: Option<usize>,
    /// The file's place in a set opened together, when it is one of several
    /// ([`crate::jobs_ui::OpenOrder`]): kept while this file waits, so its tab still lands in the
    /// set's order.
    pub order: Option<crate::jobs_ui::OpenOrder>,
    /// The profile question ([`photocraft_engine::color_cmds::ColorState::open_question`]), or
    /// `None` to open without asking.
    pub question: Option<Value>,
    /// The Color Settings policy was already applied (a background open added it before it had
    /// to wait); otherwise opening applies it.
    pub policy_applied: bool,
    /// Held by Open as Smart Object: the document holds the smart object layer already, and is
    /// admitted by [`photocraft_engine::file_cmds::admit_as_smart_object`].
    pub smart_object: bool,
}

/// Item kind in the arrival-ordered open prompt queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PromptKind {
    Memory,
    Open,
}

/// An un-decoded PSD file whose preflight estimate requires user approval.
#[derive(Clone)]
pub struct MemoryPrompt {
    pub id: u64,
    pub name: String,
    pub path: Option<String>,
    pub slot: Option<usize>,
    pub order: Option<crate::jobs_ui::OpenOrder>,
    pub estimated: u64,
    /// The capacity measured when the estimate was made; `None` when it could not be measured.
    pub available: Option<u64>,
    /// The engine could not measure available memory for this estimate. No number is shown.
    pub capacity_unknown: bool,
    /// The estimate is above the unknown-capacity warning threshold (8 GiB), so the prompt also
    /// says the process or tab may run out of memory. Approval stays possible.
    pub uncertain_capacity_warning: bool,
}

/// Whether a file that just finished reading has to wait: one earlier in the queue does.
pub fn waiting(app: &PhotocraftApp) -> bool {
    !app.held_opens.is_empty() || !app.held_memory_prompts.is_empty()
}

/// Opens `held` now, or queues it behind the file whose prompt is up.
pub fn arrive(app: &mut PhotocraftApp, held: Held) {
    app.held_opens.push_back(held);
    app.prompt_order.push_back(PromptKind::Open);
    advance(app);
}

/// Whether any open prompt (damage, profile or high-memory) is currently on screen.
pub fn has_open_dialog(app: &PhotocraftApp) -> bool {
    app.framework.dialogs.is_open::<ProfileMismatchDialog>()
        || alerts::prompt_tagged::<DamageTag>(app).is_some()
        || alerts::prompt_tagged::<MemoryTag>(app).is_some()
}

/// The result pump (`dialogs::TAKERS`): closes a damage or memory prompt whose file left the queue
/// (a cancelled profile question drops every queued file), applies the profile prompt's result,
/// then lets the next queued prompt open once the previous one is gone. Each pass drains only
/// results already stored, so a second pass in one frame finds nothing to apply twice.
pub(crate) fn take(app: &mut PhotocraftApp, ctx: &egui::Context) {
    close_stale(app, ctx);
    match app.framework.dialogs.take::<ProfileMismatchDialog>() {
        Some(DialogResult::Ok(profile)) => {
            let action = engine_action(profile.action);
            // A failed open is reported by `open` itself (`open_failed`).
            let _ = if profile.remember { answer_remembering(app, action) } else { answer(app, Some(action)) };
        }
        Some(_) => {
            let _ = answer(app, None);
        }
        None => {}
    }
    advance(app);
}

/// Closes the damage or memory prompt that no longer matches the front of its queue.
fn close_stale(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let damaged = app.held_opens.front().is_some_and(|h| h.damage_pending);
    if !damaged && let Some(id) = alerts::prompt_tagged::<DamageTag>(app) {
        app.framework.dialogs.cancel_by_id(ctx, id);
    }
    let front = app.held_memory_prompts.front().map(|m| m.id);
    if let Some(id) = alerts::prompt_tagged::<MemoryTag>(app)
        && alerts::prompt_tag::<MemoryTag>(app).map(|tag| tag.id) != front
    {
        app.framework.dialogs.cancel_by_id(ctx, id);
    }
}

/// The answer of a damage or memory prompt (`dialogs::PROMPTS`): Proceed opens the file, Cancel
/// drops that file only. `false` for another family's prompt.
pub(crate) fn answer_prompt(app: &mut PhotocraftApp, _ctx: &egui::Context, answer: &mut PromptAnswer) -> bool {
    let accept = answer.choice == PromptChoice::Proceed;
    if answer.take_tag::<DamageTag>().is_some() {
        if let Err(e) = answer_damage(app, accept) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        return true;
    }
    if let Some(MemoryTag { id }) = answer.take_tag::<MemoryTag>() {
        // `answer_memory` reports its own failures in the status bar.
        if app.held_memory_prompts.front().map(|m| m.id) == Some(id) {
            let _ = answer_memory(app, accept);
        }
        return true;
    }
    false
}

/// Opens queued files in order until one needs its prompt, which is shown if it isn't yet.
pub fn advance(app: &mut PhotocraftApp) {
    // Reconcile legacy callers that populated held queues directly.
    if app.prompt_order.is_empty() {
        app.prompt_order.extend(std::iter::repeat_n(PromptKind::Open, app.held_opens.len()));
        app.prompt_order.extend(std::iter::repeat_n(PromptKind::Memory, app.held_memory_prompts.len()));
    }
    if has_open_dialog(app) {
        return;
    }
    loop {
        match app.prompt_order.front() {
            None => return,
            Some(PromptKind::Memory) => {
                if let Some(front) = app.held_memory_prompts.front().cloned() {
                    show_memory(app, &front);
                    return;
                } else {
                    app.prompt_order.pop_front();
                }
            }
            Some(PromptKind::Open) => {
                let Some(front) = app.held_opens.front() else {
                    app.prompt_order.pop_front();
                    continue;
                };
                if front.damage_pending {
                    show_damage(app);
                    return;
                }
                if front.question.is_some() {
                    show(app);
                    return;
                }
                let Some(held) = app.held_opens.pop_front() else {
                    app.prompt_order.pop_front();
                    continue;
                };
                app.prompt_order.pop_front();
                let _ = open(app, held, None);
                if has_open_dialog(app) {
                    return;
                }
            }
        }
    }
}

/// Shows the Embedded Profile Mismatch (or missing profile) prompt for the first queued file.
fn show(app: &mut PhotocraftApp) {
    let Some(front) = app.held_opens.front() else { return };
    let Some(q) = &front.question else { return };
    let mode = match front.doc.mode {
        ColorMode::Cmyk => ProfileMode::Cmyk,
        ColorMode::Grayscale => ProfileMode::Gray,
        _ => ProfileMode::Rgb,
    };
    let text = |key: &str| q.get(key).and_then(Value::as_str).unwrap_or("?").to_owned();
    let missing = q.get("missing").and_then(Value::as_bool) == Some(true);
    let (question, action) = if missing {
        // Only "leave as is" is a policy for a missing profile; the radios offer no other policy action.
        (ProfileQuestion::missing(text("working")), ProfileAction::LeaveAsIs)
    } else {
        let action = match q.get("action").and_then(Value::as_str) {
            Some("convert") => ProfileAction::ConvertToWorking,
            Some("discard") => ProfileAction::Discard,
            _ => ProfileAction::UseEmbedded,
        };
        (ProfileQuestion::mismatch(text("embedded"), text("working")), action)
    };
    let dialog = ProfileMismatchDialog::new(front.name.clone(), mode, question).action(action).rememberable(policy_key(front.doc.mode).is_some());
    // Refused only when the question is already open; it keeps its answers.
    let _ = app.framework.dialogs.open_unless_open(dialog);
}

/// The engine's name for a profile answer (`ColorState::resolve_open`).
fn engine_action(action: ProfileAction) -> &'static str {
    match action {
        ProfileAction::UseEmbedded | ProfileAction::LeaveAsIs => "preserve",
        ProfileAction::ConvertToWorking => "convert",
        ProfileAction::Discard => "discard",
        ProfileAction::AssignWorking => "assignWorking",
        // `ProfileAction` is non-exhaustive: an action added later keeps the file as it is.
        _ => "preserve",
    }
}

/// The Color Settings policy that answers this mode's files when Ask On Mismatch is off.
fn policy_key(mode: ColorMode) -> Option<&'static str> {
    match photocraft_engine::color_cmds::mode_space(mode) {
        craft::cms::ColorSpace::Rgb => Some("policyRgb"),
        craft::cms::ColorSpace::Cmyk => Some("policyCmyk"),
        craft::cms::ColorSpace::Gray => Some("policyGray"),
        _ => None,
    }
}

/// The prompt's answer: `Some(action)` (OK) opens the first queued file with it and moves on to
/// the next; `None` (Cancel) drops every queued file and stops the reads still running.
pub fn answer(app: &mut PhotocraftApp, action: Option<&str>) -> Result<Value, String> {
    if app.held_opens.front().is_some_and(|h| h.damage_pending) {
        return Err("answer the damaged PSD confirmation first".into());
    }
    let Some(action) = action else {
        let dropped = app.held_opens.len() + app.held_memory_prompts.len();
        let name = app.held_opens.front().map(|h| h.name.clone()).or_else(|| app.held_memory_prompts.front().map(|m| m.name.clone())).unwrap_or_default();
        for m in app.held_memory_prompts.drain(..) {
            app.session.cancel_pending_preflight(m.id);
        }
        app.session.clear_pending_preflights();
        app.held_opens.clear();
        app.prompt_order.clear();
        for job in app.jobs.opens.iter().map(|o| o.job).collect::<Vec<_>>() {
            app.session.cancel_job(job);
        }
        app.jobs.opens.clear();
        app.jobs.batches.clear();
        app.ui.status = crate::i18n::fmt(tl!("Canceled opening {name}"), &[("name", &name)]);
        app.ui.status_error = false;
        return Ok(json!({"cancelled": dropped}));
    };
    let Some(held) = app.held_opens.pop_front() else { return Err("no file is waiting to open".into()) };
    if let Some(pos) = app.prompt_order.iter().position(|p| *p == PromptKind::Open) {
        app.prompt_order.remove(pos);
    }
    let r = open(app, held, Some(action));
    advance(app);
    crate::jobs_ui::end_batch(app);
    r
}

/// OK with "Remember this choice": `action` becomes the Color Settings policy for the file's
/// mode and Ask On Mismatch turns off, so files read from now on open without asking. Files
/// already waiting with a mismatch in the same mode were asked before that and get the same
/// answer; any other question still asks.
pub fn answer_remembering(app: &mut PhotocraftApp, action: &str) -> Result<Value, String> {
    if app.held_opens.front().is_some_and(|h| h.damage_pending) {
        return Err("answer the damaged PSD confirmation first".into());
    }
    let Some(key) = app.held_opens.front().and_then(|h| policy_key(h.doc.mode)) else { return answer(app, Some(action)) };
    let policy = match action {
        "convert" => "convert",
        "discard" => "off",
        _ => "preserve",
    };
    app.run("edit.colorSettings", json!({key: policy, "askOnMismatch": false}))?;
    let same = |h: &Held| {
        !h.damage_pending && policy_key(h.doc.mode) == Some(key) && h.question.as_ref().is_some_and(|q| q.get("missing").and_then(Value::as_bool) != Some(true))
    };
    let Some(held) = app.held_opens.pop_front() else { return Err("no file is waiting to open".into()) };
    if let Some(pos) = app.prompt_order.iter().position(|p| *p == PromptKind::Open) {
        app.prompt_order.remove(pos);
    }
    let r = open(app, held, Some(action));
    while app.prompt_order.front() == Some(&PromptKind::Open) && app.held_opens.front().is_some_and(same) {
        if let Some(held) = app.held_opens.pop_front() {
            app.prompt_order.pop_front();
            // A failure is shown to the user (`open_failed`); the next file still opens.
            let _ = open(app, held, Some(action));
        }
    }
    advance(app);
    crate::jobs_ui::end_batch(app);
    r
}

/// Shows the framework's damaged-file prompt for the first damaged PSD waiting in the open queue,
/// with its decoding warnings. Another family's prompt being open leaves the file waiting: the
/// next `take` pass tries again.
fn show_damage(app: &mut PhotocraftApp) {
    let Some(front) = app.held_opens.front().filter(|h| h.damage_pending) else { return };
    let prompt = StandardPrompt::damaged_file(front.name.clone())
        .paragraph(tl!("If you continue, saving over the original may permanently discard unreadable data. Save a copy to keep the original."))
        .lines(front.warnings.iter().cloned().map(Label::from))
        .tag(DamageTag);
    let _ = alerts::open_prompt(app, prompt);
}

/// Continue or discard this file only, then move the queue forward. Accepting does not
/// mark the document read-only: Save, Export, scripts and web downloads work normally.
pub(crate) fn answer_damage(app: &mut PhotocraftApp, accept: bool) -> Result<Value, String> {
    let Some(front) = app.held_opens.front_mut().filter(|h| h.damage_pending) else {
        return Err("no damaged PSD is waiting for confirmation".into());
    };
    let name = front.name.clone();
    if accept {
        front.damage_pending = false;
    } else {
        app.held_opens.pop_front();
        if let Some(pos) = app.prompt_order.iter().position(|p| *p == PromptKind::Open) {
            app.prompt_order.remove(pos);
        }
        app.ui.status = crate::i18n::fmt(tl!("Canceled opening {name}"), &[("name", &name)]);
        app.ui.status_error = false;
    }
    advance(app);
    crate::jobs_ui::end_batch(app);
    Ok(json!({"name": name, "continued": accept}))
}

/// Opens a queued file, applying `action` (the prompt's answer) or else the Color Settings
/// policy, then does what any open does afterwards.
fn open(app: &mut PhotocraftApp, held: Held, action: Option<&str>) -> Result<Value, String> {
    if held.damage_pending {
        return Err("damaged PSD cannot be opened before confirmation".into());
    }
    let Held { mut doc, read_only, damage_pending: _, name, path, warnings, bytes, slot, order, question, policy_applied, smart_object } = held;
    let index = if question.is_some() || policy_applied {
        if let Some(action) = action
            && let Err(e) = app.session.color.resolve_open(&mut doc, action)
        {
            app.open_failed(&name, &e.to_string());
            return Err(e.to_string());
        }
        if smart_object { photocraft_engine::file_cmds::admit_as_smart_object(&mut app.session, doc) } else { app.session.add_document(doc, None) }
    } else if smart_object {
        // A damage-only hold has no colour question: apply the policy as the engine's open does,
        // then admit it as the smart object document it is (name, history label, snapshot).
        let ignore_exif = app.session.prefs().file_handling.ignore_exif_profile_tag;
        app.session.color.open_policy(&mut doc, ignore_exif);
        photocraft_engine::file_cmds::admit_as_smart_object(&mut app.session, doc)
    } else {
        app.session.open_document(doc, None).0
    };
    // add_document may remap a persisted ID already owned by another open document.
    let document = app.session.documents().get(index).map(|st| st.doc.id).ok_or("the opened document is missing")?;
    if let Some(st) = app.session.active_mut() {
        st.source_read_only = read_only;
    }
    let slot = crate::jobs_ui::tab_slot(app, slot, order);
    let index = match slot {
        Some(slot) => app
            .run("document.move", json!({"document": index, "to": slot}))
            .ok()
            .and_then(|m| m.get("document").and_then(Value::as_u64))
            .map_or(index, |i| i as usize),
        None => index,
    };
    app.session.set_active(index);
    crate::jobs_ui::after_open(app, document, &name, path.as_deref(), &warnings, bytes.as_ref())?;
    Ok(json!({"document": index, "action": action}))
}

/// Queues a high-memory prompt in arrival order and shows it when it reaches the front.
#[allow(clippy::too_many_arguments)]
pub fn arrive_memory(
    app: &mut PhotocraftApp,
    id: u64,
    name: String,
    path: Option<String>,
    slot: Option<usize>,
    order: Option<crate::jobs_ui::OpenOrder>,
    estimated: u64,
    available: Option<u64>,
    capacity_unknown: bool,
    uncertain_capacity_warning: bool,
) {
    app.held_memory_prompts.push_back(MemoryPrompt { id, name, path, slot, order, estimated, available, capacity_unknown, uncertain_capacity_warning });
    app.prompt_order.push_back(PromptKind::Memory);
    advance_memory(app);
}

/// Advances the shared open-prompt queue; memory prompts take their turn with damage and profile
/// prompts in arrival order.
pub fn advance_memory(app: &mut PhotocraftApp) {
    advance(app);
}

/// Opens the confirmation prompt for a queued high-memory preflight: Open or Cancel. Another
/// family's prompt being open leaves it waiting: the next `take` pass tries again.
fn show_memory(app: &mut PhotocraftApp, prompt: &MemoryPrompt) {
    let shown = StandardPrompt::custom(memory_message(prompt))
        .title(tl!("High Memory Photoshop Document"))
        .button(PromptChoice::Proceed, tl!("Open"))
        .button(PromptChoice::Cancel, tl!("Cancel"))
        .tag(MemoryTag { id: prompt.id });
    let _ = alerts::open_prompt(app, shown);
}

/// The text the high-memory prompt shows: the estimate against the available memory, the slowdown
/// warning and, where they apply, the low-memory and unknown-capacity warnings.
pub(crate) fn memory_message(prompt: &MemoryPrompt) -> String {
    let est_gb = prompt.estimated as f64 / (1u64 << 30) as f64;
    // An unmeasured capacity is never printed as a number, even if a stale measurement is
    // attached: the engine's `capacityUnknown` wins.
    let available = prompt.available.filter(|_| !prompt.capacity_unknown);
    let mut message = match available {
        Some(a) => crate::i18n::fmt(
            tl!("Opening “{name}” requires an estimated {est} GiB of memory ({avail} GiB available)."),
            &[("name", &prompt.name), ("est", &format!("{est_gb:.1}")), ("avail", &format!("{:.1}", a as f64 / (1u64 << 30) as f64))],
        ),
        None => crate::i18n::fmt(
            tl!("Opening “{name}” requires an estimated {est} GiB of memory. The available memory could not be measured."),
            &[("name", &prompt.name), ("est", &format!("{est_gb:.1}"))],
        ),
    };
    message.push_str("\n\n");
    message.push_str(tl!("Opening this document uses more memory and may slow down your computer or other running applications."));
    if available.is_some_and(|a| a.saturating_sub(prompt.estimated) < photocraft_io::memory::LOW_MEMORY_WARNING_MARGIN) {
        message.push_str("\n\n");
        message.push_str(tl!("Little memory would be left after opening this file. Close other apps first, or PhotoCraft may run out of memory."));
    }
    if prompt.uncertain_capacity_warning {
        message.push_str("\n\n");
        message.push_str(tl!(
            "The estimate is above 8 GiB and the available memory is unknown, so this process or browser tab may run out of memory. You can still try to open it."
        ));
    }
    message
}

/// Answers the front high-memory prompt: accepts and starts the approved open, or declines and
/// advances the queue. The engine command revalidates the source identity and the hard limit.
pub(crate) fn answer_memory(app: &mut PhotocraftApp, accept: bool) -> Result<Value, String> {
    let Some(front) = app.held_memory_prompts.pop_front() else {
        advance(app);
        return Err("no high-memory PSD is waiting for confirmation".into());
    };
    if let Some(pos) = app.prompt_order.iter().position(|p| *p == PromptKind::Memory) {
        app.prompt_order.remove(pos);
    }
    let id = front.id;
    let name = front.name.clone();
    let path = front.path.clone();
    let slot = front.slot;
    let order = front.order;

    let res = app.run("file.approveMemory", json!({"id": id, "approve": accept, "background": app.background_jobs})).map_err(|e| format!("{}: {e}", name));

    match res {
        Ok(r) => {
            if accept {
                if let Some(job) = r.get("job").and_then(Value::as_u64) {
                    app.jobs.opens.push(crate::jobs_ui::OpenTab { job: photocraft_engine::jobs::JobId(job), name: name.clone(), path, slot, order });
                    app.jobs.focus = Some(photocraft_engine::jobs::JobId(job));
                    app.ui.chrome.home = None;
                    app.ui.status = crate::i18n::fmt(tl!("Opening {name}…"), &[("name", &name)]);
                    app.ui.status_error = false;
                } else if let Some(result) = r.get("result") {
                    let tab = crate::jobs_ui::OpenTab { job: photocraft_engine::jobs::JobId(0), name: name.clone(), path: path.clone(), slot, order };
                    if let Err(e) = crate::jobs_ui::finish_open_tab(app, &tab, result) {
                        app.ui.status = e;
                        app.ui.status_error = true;
                    }
                }
            } else {
                app.ui.status = crate::i18n::fmt(tl!("Canceled opening {name}"), &[("name", &name)]);
                app.ui.status_error = false;
                crate::jobs_ui::end_batch(app);
            }
            advance(app);
            crate::jobs_ui::end_batch(app);
            Ok(r)
        }
        Err(err) => {
            app.ui.status = err.clone();
            app.ui.status_error = true;
            crate::jobs_ui::end_batch(app);
            advance(app);
            crate::jobs_ui::end_batch(app);
            Err(err)
        }
    }
}
