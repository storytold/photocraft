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
    /// The answer given to every document still to ask about, once "Apply to All" was used.
    auto: Option<Answer>,
    /// The document `auto` answered last: if it is asked about again, its save didn't go through.
    auto_answered: Option<DocId>,
    /// `auto` came from a control client (`saveAll`): its Saves are judged as automation.
    auto_control: bool,
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
    let dirty = affected.into_iter().filter_map(|i| docs.get(i)).filter(|d| d.is_dirty()).map(|d| d.doc.id).collect();
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
    let prompt = Prompt { id: id.to_string(), params: params.clone(), target, docs, asked: None, auto: None, auto_answered: None, auto_control: false };
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
        !self.allow_close && self.session.documents().iter().any(|d| d.is_dirty())
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
            .filter(|d| d.is_dirty() && (matches!(reach(&id), Some(Reach::All)) || Some(d.doc.id) != target))
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
        Ok(_) => {
            !app.save_options_open()
                && !app.saving()
                && index_of(app, doc).is_some_and(|i| app.services.write_unconfirmed || !app.session.documents()[i].is_dirty())
        }
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
        && app.discard.as_ref().and_then(|p| p.docs.first()) == Some(&doc)
        && index_of(app, doc).is_some_and(|i| app.services.write_unconfirmed || !app.session.documents()[i].is_dirty())
    {
        advance(app, ctx);
    }
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
    if app.file_dialog_open() || app.save_options_open() || app.saving() {
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
    if !app.framework.dialogs.is_open::<UnsavedChangesDialog>() {
        return;
    }
    let front = app.discard.as_ref().and_then(|p| p.docs.first().copied());
    let asked = app.discard.as_ref().and_then(|p| p.asked);
    if front.is_none() || front != asked {
        app.framework.dialogs.cancel::<UnsavedChangesDialog>(ctx);
        let _ = app.framework.dialogs.take::<UnsavedChangesDialog>();
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

/// The command the prompt holds back and its params, for the automation policy to see before a
/// control client answers it.
pub fn pending_command(app: &PhotocraftApp) -> Option<(String, Value)> {
    app.discard.as_ref().map(|p| (p.id.clone(), p.params.clone()))
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
