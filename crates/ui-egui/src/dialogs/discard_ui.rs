//! Save / Don't Save / Cancel prompt for actions that would throw away unsaved changes: closing
//! documents, Revert, and leaving the app (File › Exit or the window's close button).
//!
//! egui is immediate-mode, so the action is parked in [`Prompt`] while the modal is up and re-run
//! once every affected document has been answered. Cancel at any point drops it. Documents are
//! tracked by id, not tab index, so closing one elsewhere while the prompt is up can't retarget it.

use craft::quit::{CloseAnswer, CloseScope};
use craft::ui::dialog::{DialogResult, UnsavedAnswer, UnsavedChangesDialog};
use craft::ui::shell::embed::CloseDecision;
use photocraft_doc::DocId;
use serde_json::{Value, json};

use crate::PhotocraftApp;

const EXIT: &str = "file.exit";

/// An action waiting on the user, and the dirty documents still to ask about.
pub struct Prompt {
    id: String,
    params: Value,
    /// The document the command was aimed at (`document` param, else the active one), if it has one.
    target: Option<DocId>,
    docs: Vec<DocId>,
    /// The document the open question was opened for. A prompt that replaced this one (quitting
    /// over a pending close) starts with `None`, so the question still open is recognised as the
    /// old prompt's and closed.
    asked: Option<DocId>,
    /// The acknowledged save waits for an editor pass before continuing Close or Quit.
    provider_save: Option<ProviderSave>,
    /// The answer given to every document still to ask about, once "Apply to All" was used.
    auto: Option<Answer>,
    /// The document `auto` answered last: if it is asked about again, its save didn't go through.
    auto_answered: Option<DocId>,
    /// `auto` came from a control client (`saveAll`): its Saves are judged as automation.
    auto_control: bool,
}

struct ProviderSave {
    doc: DocId,
    revision: u64,
    connection: String,
    durable: bool,
    had_input: bool,
}

fn provider_close_waits(app: &PhotocraftApp) -> bool {
    app.discard.as_ref().is_some_and(|p| p.provider_save.is_some())
}

fn index_of(app: &PhotocraftApp, id: DocId) -> Option<usize> {
    app.session.documents().iter().position(|d| d.doc.id == id)
}

/// Which documents a command discards, relative to its target (the `document` param, else the active one).
#[derive(Clone, Copy)]
enum Reach {
    Target,
    AllButTarget,
    All,
}

/// Every command that can throw away unsaved changes. A new one only needs a row here.
const DISCARDING: &[(&str, Reach)] = &[
    ("file.close", Reach::Target),
    ("file.revert", Reach::Target),
    ("file.closeOthers", Reach::AllButTarget),
    ("file.closeAll", Reach::All),
    (EXIT, Reach::All),
];

fn reach(id: &str) -> Option<Reach> {
    DISCARDING.iter().find(|(c, _)| *c == id).map(|(_, r)| *r)
}

/// The command's target document and the unsaved documents it would discard (empty for commands
/// that discard nothing).
fn discarded(app: &PhotocraftApp, id: &str, params: &Value) -> (Option<DocId>, Vec<DocId>) {
    let docs = app.session.documents();
    let target = params.get("document").and_then(Value::as_u64).map(|i| i as usize).or(app.session.active_index());
    let affected: Vec<usize> = match reach(id) {
        Some(Reach::Target) => target.into_iter().collect(),
        Some(Reach::AllButTarget) => (0..docs.len()).filter(|&i| Some(i) != target).collect(),
        Some(Reach::All) => (0..docs.len()).collect(),
        None => Vec::new(),
    };
    let dirty = affected
        .into_iter()
        .filter_map(|i| docs.get(i))
        .filter(|d| {
            d.is_dirty()
                || app.services.file_provider.as_ref().is_some_and(|provider| provider.pending_for(d.doc.id) || provider.prepared_for(d.doc.id))
                || app.discard.as_ref().and_then(|p| p.provider_save.as_ref()).is_some_and(|save| save.doc == d.doc.id)
        })
        .map(|d| d.doc.id)
        .collect();
    (target.and_then(|i| docs.get(i)).map(|d| d.doc.id), dirty)
}

/// Park `id` behind a prompt if it would discard unsaved work. Returns whether it did.
pub fn intercept(app: &mut PhotocraftApp, id: &str, params: &Value) -> bool {
    let (target, docs) = discarded(app, id, params);
    if docs.is_empty() {
        return false;
    }
    // Once the window's GPU device is lost nothing can be drawn (`gpu_status::check_window`), so
    // the prompt would never show. While saving for recovery keeps failing, quitting (the close
    // button, File › Exit) must still go through rather than leave a blank window that can't be
    // closed.
    if id == EXIT && app.window_lost {
        log::warn!("quitting a window that lost its GPU device before its unsaved work could be saved for recovery");
        return false;
    }
    let prompt = Prompt {
        id: id.to_string(),
        params: params.clone(),
        target,
        docs,
        asked: None,
        provider_save: None,
        auto: None,
        auto_answered: None,
        auto_control: false,
    };
    match &app.discard {
        None => ask(app, prompt),
        // Quitting overrides whatever is pending: it covers every document, so nothing is lost.
        Some(open) if id == EXIT && open.id != EXIT => ask(app, prompt),
        // Repeated quit requests (the X pressed again) keep the prompt, and the answers so far.
        Some(open) if open.id == id => {}
        Some(_) => {
            app.ui.status = tl!("Answer the unsaved-changes prompt first").into();
            app.ui.status_error = true;
        }
    }
    true
}

/// Puts up `prompt`. One over several documents shows the document it asks about first; each
/// later one is shown as it comes up (see [`advance`]).
fn ask(app: &mut PhotocraftApp, prompt: Prompt) {
    if !matches!(reach(&prompt.id), Some(Reach::Target)) {
        show_tab(app, prompt.docs.first().copied());
    }
    app.discard = Some(prompt);
}

/// Makes `doc` the active tab, if it is still open.
fn show_tab(app: &mut PhotocraftApp, doc: Option<DocId>) {
    if let Some(i) = doc.and_then(|d| index_of(app, d)) {
        app.session.set_active(i);
    }
}

impl PhotocraftApp {
    /// Whether closing now would lose work: a document has unsaved changes and closing wasn't
    /// confirmed. The web build has the browser ask before leaving the page while this holds (#1380).
    pub fn has_unsaved_work(&self) -> bool {
        !self.allow_close
            && (self.session.documents().iter().any(|d| d.is_dirty())
                || self.services.file_provider.as_ref().is_some_and(|provider| provider.busy())
                || provider_close_waits(self))
    }
}

/// The close decision for the craft runtime's close path (`embed::logic`), asked when the window
/// was asked to close: hold it while there is unsaved work. Every way of leaving (File › Exit /
/// Quit, the close button, the OS's quit event, `app.quit`) ends up here as a close request. On
/// `Close` the runtime leaves through the platform's own quit where there is one (macOS).
pub fn close_decision(app: &mut PhotocraftApp) -> CloseDecision {
    if !app.allow_close && intercept(app, EXIT, &Value::Null) {
        return CloseDecision::Hold;
    }
    // A save or print worker must finish its external side effect before the process exits;
    // `jobs_ui` repeats the close after completion and keeps errors visible.
    if app.saving() || app.printing() {
        app.jobs.close_after_saves = true;
        return CloseDecision::Hold;
    }
    CloseDecision::Close
}

/// Moves on to the next document, or runs the parked action once none are left.
fn advance(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(p) = app.discard.as_mut() else { return };
    let answered = (!p.docs.is_empty()).then(|| p.docs.remove(0));
    let (next, one_by_one) = (p.docs.first().copied(), !matches!(reach(&p.id), Some(Reach::Target)));
    // Close All, Close Others and quitting close each document once it is answered, as Photoshop
    // does, and show the next one asked about: otherwise the tab just answered stays up and it
    // looks like it is being asked about again (#2547). Cancel then keeps only the unanswered ones.
    if one_by_one {
        if let Some(i) = answered.and_then(|d| index_of(app, d))
            && let Err(e) = crate::menus::invoke_unguarded(app, ctx, "file.close", json!({"document": i}))
        {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        show_tab(app, next);
    }
    if next.is_some() {
        return;
    }
    // A save or close hook may have dirtied a document that was clean when the prompt opened (or
    // made a new one): ask about those before the final close, which discards without checking.
    if one_by_one && let Some(p) = app.discard.as_ref() {
        let (id, target) = (p.id.clone(), p.target);
        let rest: Vec<DocId> = app
            .session
            .documents()
            .iter()
            .filter(|d| {
                (d.is_dirty() || app.services.file_provider.as_ref().is_some_and(|provider| provider.pending_for(d.doc.id) || provider.prepared_for(d.doc.id)))
                    && (matches!(reach(&id), Some(Reach::All)) || Some(d.doc.id) != target)
            })
            .map(|d| d.doc.id)
            .collect();
        if !rest.is_empty() {
            show_tab(app, rest.first().copied());
            if let Some(p) = app.discard.as_mut() {
                p.docs = rest;
                // Apply to All covered the documents asked about; these are asked about afresh.
                (p.auto, p.auto_answered, p.auto_control) = (None, None, false);
            }
            return;
        }
    }
    let Some(Prompt { id, mut params, target, .. }) = app.discard.take() else { return };
    if id == EXIT {
        app.allow_close = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        return;
    }
    if let Some(target) = target {
        // The tab may have moved since the command was issued; aim it at the same document.
        params = match index_of(app, target) {
            Some(i) => json!({"document": i}),
            // Close All may have closed its target above already; it still closes the rest.
            None if matches!(reach(&id), Some(Reach::All)) => json!({}),
            None => return,
        };
    }
    if let Err(e) = crate::menus::invoke_unguarded(app, ctx, &id, params) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// Saves `doc`; true once it is saved. A document without a file to write back to asks where in
/// the save dialog: the prompt then hides until it is answered, and moves on only if it saved.
fn save(app: &mut PhotocraftApp, ctx: &egui::Context, doc: DocId) -> bool {
    let Some(i) = index_of(app, doc) else { return false };
    app.session.set_active(i);
    let saved = crate::menus::invoke_unguarded(app, ctx, "file.save", json!({}));
    if saved.is_ok() && app.file_dialog_open() {
        let ctx = ctx.clone();
        app.after_file_dialog(move |app, saved| {
            let saved = saved.map_err(couldnt_save)?;
            saved_document(app, &ctx, doc);
            Ok(saved)
        });
        return false;
    }
    match saved {
        // A Save Document script may have edited the document after it was written: still dirty.
        // A write the platform can't confirm (a web download) leaves it dirty too, and counts.
        Ok(_) => !app.save_options_open() && !app.saving() && saved_enough_to_close(app, doc),
        // Backing out of the file dialog is the user's choice, not an error.
        Err(e) if e == crate::file_dialog::CANCELLED => false,
        Err(e) => {
            app.ui.status = couldnt_save(e);
            app.ui.status_error = true;
            false
        }
    }
}

/// A completed save may release the close prompt. Choosing a path only starts a layered TIFF
/// save; TIFF Options calls this after the write, and `jobs_ui` when a background save ends.
/// Copies and failed writes leave the doc dirty; so does any write when it is unconfirmed.
pub(crate) fn saved_document(app: &mut PhotocraftApp, ctx: &egui::Context, doc: DocId) {
    // Still the prompt that asked (quitting may have replaced it meanwhile).
    if !app.save_options_open()
        && !provider_close_waits(app)
        && !app.services.file_provider.as_ref().is_some_and(|provider| provider.pending_for(doc))
        && app.discard.as_ref().and_then(|p| p.docs.first()) == Some(&doc)
        && saved_enough_to_close(app, doc)
    {
        advance(app, ctx);
    }
}

/// A provider ACK is applied in logic, before the frame's raw input is processed. Keep the
/// checkpoint, but defer only its close/quit continuation until a complete UI pass has settled.
pub(crate) fn provider_saved_document(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    host: &crate::file_providers::Host,
    doc: DocId,
    revision: u64,
    durable: bool,
) {
    let Some(connection) = host.connection_id() else { return };
    let Some(prompt) = app.discard.as_mut().filter(|p| p.docs.first() == Some(&doc)) else { return };
    let had_input = ctx.input(|i| !i.events.is_empty() || !i.raw.dropped_files.is_empty());
    prompt.provider_save = Some(ProviderSave { doc, revision, connection, durable, had_input });
    ctx.request_repaint();
}

/// The end of a real editor pass, never a hidden logic-only tick. A later revision cancels
/// automatic closing; pending input or an uncommitted tool/field gets time to settle first.
pub(crate) fn finish_provider_save(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if ctx.will_discard() {
        return;
    }
    let Some(mut save) = app.discard.as_mut().and_then(|p| p.provider_save.take()) else { return };
    // The answer belongs to the question before this save, even if the save's revision is no
    // longer current. Retire it before inspecting the shared stack: our own question must not
    // keep the ACK's continuation waiting, and no queued answer may act on a later revision.
    suspend_question(app, ctx);
    let same_connection = app.services.file_provider.as_ref().and_then(|host| host.connection_id()).as_deref() == Some(save.connection.as_str());
    let same_revision = app.session.documents().iter().any(|state| state.doc.id == save.doc && state.revision == save.revision);
    // A second tab may have overwritten the resource after this ACK. A retired provider
    // checkpoint must never fall through to the browser-download close exception.
    let checkpoint = !save.durable || app.services.file_provider.as_ref().is_some_and(|host| host.acknowledged_at(save.doc, save.revision));
    if !same_connection || !same_revision || !checkpoint || !saved_enough_to_close(app, save.doc) {
        ctx.request_repaint(); // bring the unanswered prompt back after this pass
        return;
    }
    // Type-tool and egui text fields may consume their events. Remember input observed when
    // the ACK arrived, so consuming it cannot make this very pass look idle.
    let input = save.had_input || ctx.input(|i| !i.events.is_empty() || !i.raw.dropped_files.is_empty());
    let held = ctx.input(|i| i.pointer.any_down() || !i.keys_down.is_empty());
    let synthetic = !craft::ui::runtime::windows::WindowHost::of(ctx).injected_idle();
    if input || held || synthetic || provider_ui_work(app, ctx, save.doc) {
        save.had_input = false;
        if let Some(prompt) = app.discard.as_mut() {
            prompt.provider_save = Some(save);
        }
        // Exactly one follow-up pass for handled input. Held gestures/drafts wake through
        // their own events; an idle editor with a parked field does not become a poll loop.
        if input {
            ctx.request_repaint();
        }
        return;
    }
    saved_document(app, ctx, save.doc);
}

pub(crate) fn provider_ui_work(app: &PhotocraftApp, ctx: &egui::Context, doc: DocId) -> bool {
    let active = app.session.active().map(|state| state.doc.id) == Some(doc);
    // Marked text now belongs to the shared canvas editor, rather than TextEdit. Keep the
    // document's open editing session until its editor commits or cancels it.
    let type_edit = app.ui.text_edit.as_ref().is_some_and(|edit| edit.document.map_or(active, |id| id == doc.0));
    type_edit
        || app.ui.type_transform.as_ref().is_some_and(|edit| edit.document == doc.0)
        || app.ui.cam_transform.as_ref().is_some_and(|edit| edit.document == doc.0)
        || (app.ui.pen.is_some() && app.ui.pen_doc == Some(doc))
        || app.dialog_open()
        || (active
            && (ctx.text_edit_focused()
                || app.ui.transform.is_some()
                || crate::canvas::tools::in_gesture(app)
                || app.canvas_tools.selection.outline_open()
                || app.live_stroke.is_some()
                || app.live_adjust.is_some()
                || crate::gradient_ui::editing(app)
                || crate::crop_ui::pending(app)
                || crate::perspective_crop_ui::pending(app)
                || app.smart_svg.modal_open()))
}

/// A local browser download keeps its existing close behaviour. A document bound to an
/// external provider must be clean at the acknowledged revision; a later edit still asks.
fn saved_enough_to_close(app: &PhotocraftApp, doc: DocId) -> bool {
    let Some(state) = app.session.documents().iter().find(|state| state.doc.id == doc) else { return false };
    !state.is_dirty() || (app.services.write_unconfirmed && !app.services.file_provider.as_ref().is_some_and(|provider| provider.bound(doc)))
}

/// A save failure as reported ("cancelled" stays as it is: it isn't reported).
fn couldnt_save(e: String) -> String {
    if e == crate::file_dialog::CANCELLED { e } else { format!("Couldn't save: {e}") }
}

/// The result pump (`dialogs::TAKERS`) of the unsaved-changes (or Revert) question: opens it for
/// the document at the front of the pending prompt and acts on its answer; a refused or failed
/// answer is reported in the status bar and the question comes back.
///
/// A pass applies only the result stored for it: the host hands each result out once, so a second
/// pass in one frame finds none. Each pass answers at most one document by Apply to All.
pub(crate) fn take(app: &mut PhotocraftApp, ctx: &egui::Context) {
    // Save can ask for a path and then TIFF or Targa Options, and then write in the background.
    // Wait for all three; cancellation or a failed save brings us back. The question is closed
    // meanwhile: the answer that started the save closed it.
    if app.file_dialog_open() || app.save_options_open() || app.saving() || provider_close_waits(app) {
        // A provider ACK or a save from another entry point can start while a question is open.
        // Retire that question and any queued answer before waiting, so it cannot answer a
        // later revision after the save or its deferred close continuation finishes.
        suspend_question(app, ctx);
        return;
    }
    close_stale_question(app, ctx);
    let Some(p) = &app.discard else { return };
    let Some(&doc) = p.docs.first() else { return };
    let scope = scope_of(&p.id);
    let Some(name) = index_of(app, doc).map(|i| app.session.documents()[i].doc.name.clone()) else {
        // Closed by something else while the prompt was up: nothing left to ask about.
        advance(app, ctx);
        return;
    };
    // "Apply to All" answers the rest one document per frame, as Photoshop does. A document that
    // comes back (its save was cancelled or failed) is asked about normally again, so a save that
    // can't succeed never repeats on its own.
    // An Apply to All a control client chose keeps judging each Save as that client's.
    match app.discard.as_ref().and_then(|p| p.auto.map(|a| (a, p.auto_answered == Some(doc), p.auto_control))) {
        Some((answer, false, from_control)) => {
            if let Some(p) = app.discard.as_mut() {
                p.auto_answered = Some(doc);
            }
            // A refused Save leaves this document first: the next frame asks about it normally.
            if let Err(e) = apply(app, ctx, doc, answer, from_control) {
                app.ui.status = e;
                app.ui.status_error = true;
            }
            ctx.request_repaint();
            return;
        }
        Some((_, true, _)) => {
            if let Some(p) = app.discard.as_mut() {
                p.auto = None;
                p.auto_control = false;
            }
        }
        None => {}
    }
    // The answer of the open question. The host reports `Cancel` only for a question it closed from
    // outside (a control client's cancel): the client wants the prompt dropped like Esc would.
    let result = match app.framework.dialogs.take::<UnsavedChangesDialog>() {
        Some(DialogResult::Ok(given)) => Some((answer_choice(given), given.apply_to_all)),
        Some(_) => Some((Answer::Cancel, false)),
        None => None,
    };
    if let Some((answer, apply_to_all)) = result {
        // A control client's answer arrives with `automation_input` set: its Saves are judged as
        // automation, and so are the later ones of an Apply to All it chose.
        let from_control = app.automation_input;
        // The answer replaces an Apply to All left behind (a save that failed at once).
        if let Some(p) = app.discard.as_mut() {
            (p.auto, p.auto_answered, p.auto_control) = (None, None, false);
        }
        if apply_to_all {
            apply_to_rest(app, doc, answer, from_control);
        }
        if let Err(e) = apply(app, ctx, doc, answer, from_control) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        return;
    }
    ensure_question(app, doc, scope, name);
}

fn ensure_question(app: &mut PhotocraftApp, doc: DocId, scope: CloseScope, name: String) {
    if !app.framework.dialogs.is_open::<UnsavedChangesDialog>() {
        let remaining = app.discard.as_ref().map_or(1, |p| p.docs.len());
        // Apply to All is offered only while several documents are left, and never for a revert:
        // the dialog decides from `remaining` and the scope.
        app.framework.dialogs.open(UnsavedChangesDialog::new(scope, name).remaining(remaining));
        if let Some(p) = app.discard.as_mut() {
            p.asked = Some(doc);
        }
    }
}

/// What the question's wording and Cancel cover: Revert for `file.revert`, Quit for `file.exit`,
/// else Close.
fn scope_of(id: &str) -> CloseScope {
    match id {
        "file.revert" => CloseScope::Revert,
        EXIT => CloseScope::Quit,
        _ => CloseScope::Close,
    }
}

/// Closes the open question when it no longer belongs to the pending prompt: the prompt is gone,
/// quitting replaced it, or its front document moved on. The result the host keeps for it is
/// dropped here so it is not taken as an answer.
fn close_stale_question(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let front = app.discard.as_ref().and_then(|p| p.docs.first().copied());
    let asked = app.discard.as_ref().and_then(|p| p.asked);
    if front.is_none() || front != asked {
        suspend_question(app, ctx);
    }
}

fn suspend_question(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.framework.dialogs.is_open::<UnsavedChangesDialog>() {
        app.framework.dialogs.cancel::<UnsavedChangesDialog>(ctx);
    }
    let _ = app.framework.dialogs.take::<UnsavedChangesDialog>();
    if let Some(prompt) = app.discard.as_mut() {
        prompt.asked = None;
    }
}

/// Makes a Save or Don't Save answer about `doc` the answer for every document still to ask about.
/// `from_control` says a control client gave it, so each later Save is judged as automation too.
fn apply_to_rest(app: &mut PhotocraftApp, doc: DocId, answer: Answer, from_control: bool) {
    if !matches!(answer, Answer::Save | Answer::Discard) {
        return;
    }
    if let Some(p) = app.discard.as_mut() {
        p.auto = Some(answer);
        p.auto_answered = Some(doc);
        p.auto_control = from_control;
    }
}

/// Acts on the answer about `doc`, the document the prompt is asking about. `from_control` says
/// a control client gave it (`ui.dialog.confirm`). Save is refused when automation may not run
/// File › Save (see [`authorize_save`]); the prompt stays up.
fn apply(app: &mut PhotocraftApp, ctx: &egui::Context, doc: DocId, answer: Answer, from_control: bool) -> Result<(), String> {
    match answer {
        Answer::Cancel => app.discard = None,
        Answer::Discard => advance(app, ctx),
        Answer::Save => {
            authorize_save(app, from_control)?;
            if save(app, ctx, doc) {
                advance(app, ctx);
            }
        }
    }
    Ok(())
}

/// Save writes the document back over its own file, as File › Save does, so automation (a
/// control client's answer, a control request's gate or synthetic input) is judged for
/// `file.save` before it runs: a client allowed to close a document can't use the prompt to
/// write a file its policy keeps it from. A control answer counts as automation even when its
/// services install no session gate (only the command callback). The installed gate (or the
/// services' policy) and the command callback both judge it, and with neither available the
/// Save is refused rather than run unjudged. A control client's `ui.dialog.confirm` calls it with
/// `from_control` before it confirms a Save, so a refusal leaves the question open.
pub(crate) fn authorize_save(app: &PhotocraftApp, from_control: bool) -> Result<(), String> {
    if !(from_control || app.automation_input || app.session.authorize.is_some()) {
        return Ok(());
    }
    let params = json!({});
    let gate = app.session.authorize.or(app.services.automation_authorize);
    if let Some(gate) = gate {
        gate("file.save", &params).map_err(|e| e.to_string())?;
    }
    match app.services.automation_command.as_ref() {
        Some(authorize) => authorize("file.save", &params),
        None if gate.is_some() => Ok(()),
        None => Err("automation command `file.save` is refused: no automation policy is available to judge it".into()),
    }
}

/// The id the control channel knows the prompt by: `ui.inspect` lists it under this id, and `ui.dialog.confirm` and
/// `ui.dialog.cancel` answer it by it (docs/control-protocol.md, #4920, #15544).
pub const CONTROL_ID: &str = "discard";

/// The host's id of the open question, for a control request naming [`CONTROL_ID`].
pub(crate) fn host_id(app: &PhotocraftApp) -> Option<craft::ui::dialog::DialogId> {
    app.framework.dialogs.get::<UnsavedChangesDialog>().map(craft::ui::dialog::StackDialog::id)
}

/// Lists the open question in `dialogs` (the host's `ui.inspect` entries) under [`CONTROL_ID`], its fields adding the
/// command it holds back, the document it asks about, how many are left to answer, the choices `ui.dialog.confirm`
/// takes and the default one.
pub(crate) fn describe(app: &PhotocraftApp, dialogs: &mut Value) {
    let Some(key) = host_id(app).map(|id| id.value().to_string()) else { return };
    let Some(entry) = dialogs.as_array_mut().and_then(|list| list.iter_mut().find(|d| d["id"] == key.as_str())) else { return };
    entry["id"] = json!(CONTROL_ID);
    let Some(p) = app.discard.as_ref() else { return };
    let name = p.docs.first().and_then(|&doc| index_of(app, doc)).and_then(|i| app.session.documents().get(i)).map(|d| d.doc.name.clone());
    let reverts = p.id == "file.revert";
    let choices: &[&str] = match (reverts, p.docs.len() > 1) {
        (true, _) => &["dontSave", "cancel"],
        (false, false) => &["save", "dontSave", "cancel"],
        (false, true) => &["save", "saveAll", "dontSave", "dontSaveAll", "cancel"],
    };
    let described = json!({
        "command": p.id,
        "document": name,
        "remaining": p.docs.len(),
        "choices": choices,
        "default": if reverts { "dontSave" } else { "save" },
    });
    match entry.get_mut("fields").and_then(Value::as_object_mut) {
        Some(fields) => fields.extend(described.as_object().cloned().unwrap_or_default()),
        None => entry["fields"] = described,
    }
}

/// The command the prompt holds back and its params, for the automation policy to see before a
/// control client answers it.
pub fn pending_command(app: &PhotocraftApp) -> Option<(String, Value)> {
    app.discard.as_ref().map(|p| (p.id.clone(), p.params.clone()))
}

/// Native protocol fixtures answer the real shared question through the current control
/// surface, including its command policy, choice mapping and result pump.
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) fn answer_control(app: &mut PhotocraftApp, ctx: &egui::Context, choice: &str) -> Result<Value, String> {
    let Some((doc, scope)) = app.discard.as_ref().and_then(|p| p.docs.first().map(|doc| (*doc, scope_of(&p.id)))) else {
        return Err("no unsaved-changes prompt is open".into());
    };
    // Do not reopen a suspended question, consume an Apply to All, or let an answer bypass the
    // save/ACK's ownership. In particular Don't Save cannot cancel an acknowledged save's wait.
    if app.file_dialog_open() || app.save_options_open() || app.saving() || provider_close_waits(app) {
        return Err("the unsaved-changes prompt is waiting for a save to finish".into());
    }
    let Some(name) = index_of(app, doc).map(|i| app.session.documents()[i].doc.name.clone()) else {
        return Err("the unsaved-changes document is no longer open".into());
    };
    close_stale_question(app, ctx);
    ensure_question(app, doc, scope, name);
    let id = app
        .framework
        .dialogs
        .stack()
        .ids()
        .find(|id| app.framework.dialogs.get_by_id::<UnsavedChangesDialog>(*id).is_some())
        .ok_or("no unsaved-changes question is open")?;
    let (request, _reply) =
        crate::control::ControlRequest::new("ui.dialog.confirm", json!({"dialog": id.value().to_string(), "choice": choice, "wait": false}));
    match crate::control::handle(app, ctx, &request) {
        crate::control::Reply::Done(reply) if reply["ok"] == true => Ok(reply["result"].clone()),
        crate::control::Reply::Done(reply) => Err(reply["error"].as_str().unwrap_or("the question refused the answer").to_owned()),
        _ => Err("the question did not answer immediately".into()),
    }
}

/// What the question's answer means for the pending prompt.
#[derive(Clone, Copy)]
enum Answer {
    Save,
    Discard,
    Cancel,
}

/// The prompt's answer for a button of the framework question (Revert answers Don't Save).
fn answer_choice(given: UnsavedAnswer) -> Answer {
    match given.answer {
        CloseAnswer::Save => Answer::Save,
        CloseAnswer::DontSave => Answer::Discard,
        CloseAnswer::Cancel => Answer::Cancel,
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod provider_question_tests {
    use craft::ui::dialog::UnsavedChangesDialog;
    use craft::ui::patterns::form_dialogs::{ExportOptionsKind, HostedFormatOptions};
    use serde_json::json;

    use super::{finish_provider_save, intercept, provider_saved_document, take};
    use crate::file_providers::Host;
    use crate::{PhotocraftApp, Services};

    fn app() -> (Box<PhotocraftApp>, Host) {
        let provider = Host::default();
        provider.set_session("close-provider-fixture-0001".into()).unwrap();
        provider.connect("files".into(), "Stored documents".into(), "account".into()).unwrap();
        let mut app = PhotocraftApp::new(
            photocraft_engine::Session::new(),
            Services { file_provider: Some(provider.clone()), write_unconfirmed: true, ..Default::default() },
        );
        app.run("file.new", json!({"width": 2, "height": 2})).unwrap();
        let document = app.session.active_mut().unwrap();
        document.revision = 7;
        document.saved_revision = 2;
        (app, provider)
    }

    #[test]
    fn an_ack_continuation_retires_its_own_question_without_waiting_on_it() {
        let (mut app, provider) = app();
        let ctx = app.ctx().clone();
        let doc = app.session.active().unwrap().doc.id;
        assert!(intercept(&mut app, "file.close", &json!({})));
        take(&mut app, &ctx);
        assert!(app.framework.dialogs.is_open::<UnsavedChangesDialog>());
        provider_saved_document(&mut app, &ctx, &provider, doc, 7, false);
        assert!(app.discard.as_ref().unwrap().provider_save.is_some());
        assert_eq!(app.session.documents().len(), 1);
        finish_provider_save(&mut app, &ctx);
        assert!(!app.framework.dialogs.is_open::<UnsavedChangesDialog>());
        assert!(app.session.documents().is_empty());
        assert!(app.discard.is_none());
    }

    #[test]
    fn an_ack_continuation_preserves_and_waits_for_another_shared_dialog() {
        let (mut app, provider) = app();
        let ctx = app.ctx().clone();
        let doc = app.session.active().unwrap().doc.id;
        assert!(intercept(&mut app, "file.close", &json!({})));
        take(&mut app, &ctx);
        app.framework.dialogs.open(HostedFormatOptions::new(&ctx, ExportOptionsKind::Dds));
        provider_saved_document(&mut app, &ctx, &provider, doc, 7, false);
        finish_provider_save(&mut app, &ctx);
        assert!(!app.framework.dialogs.is_open::<UnsavedChangesDialog>());
        assert!(app.framework.dialogs.is_open::<HostedFormatOptions>());
        assert!(app.discard.as_ref().unwrap().provider_save.is_some());
        assert_eq!(app.session.documents().len(), 1);
        app.framework.dialogs.cancel::<HostedFormatOptions>(&ctx);
        finish_provider_save(&mut app, &ctx);
        assert!(app.session.documents().is_empty());
        assert!(app.discard.is_none());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn control_answers_cannot_reopen_a_question_while_its_ack_waits() {
        let (mut app, provider) = app();
        let ctx = app.ctx().clone();
        let doc = app.session.active().unwrap().doc.id;
        assert!(intercept(&mut app, "file.close", &json!({})));
        provider_saved_document(&mut app, &ctx, &provider, doc, 7, false);
        for choice in ["save", "dontSave", "saveAll"] {
            assert!(super::answer_control(&mut app, &ctx, choice).is_err());
            assert!(!app.framework.dialogs.is_open::<UnsavedChangesDialog>());
            assert!(app.discard.as_ref().unwrap().provider_save.is_some());
            assert_eq!(app.session.documents().len(), 1);
        }
    }

    #[test]
    fn an_answer_queued_during_an_ack_cannot_discard_a_later_revision() {
        let (mut app, provider) = app();
        let ctx = app.ctx().clone();
        let doc = app.session.active().unwrap().doc.id;
        assert!(intercept(&mut app, "file.close", &json!({})));
        take(&mut app, &ctx);
        let id = app.framework.dialogs.open_dialogs().last().unwrap().id;
        provider_saved_document(&mut app, &ctx, &provider, doc, 7, false);
        app.framework.dialogs.set_json_field(&ctx, id, "choice", json!("dontSave")).unwrap();
        assert!(app.framework.dialogs.confirm_by_id(&ctx, id));
        take(&mut app, &ctx);
        app.session.active_mut().unwrap().revision = 8;
        finish_provider_save(&mut app, &ctx);
        take(&mut app, &ctx);
        assert_eq!(app.session.documents().len(), 1);
        assert_eq!(app.session.active().unwrap().revision, 8);
        assert!(app.discard.as_ref().unwrap().provider_save.is_none());
        assert_eq!(app.discard.as_ref().unwrap().asked, Some(doc));
        assert!(app.framework.dialogs.is_open::<UnsavedChangesDialog>());
    }
}
