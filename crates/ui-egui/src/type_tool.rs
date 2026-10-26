//! Type tool: click to add point text, drag to add paragraph text, edit inline with a caret.
//!
//! Every edit is an engine `type.*` command carrying the session's `coalesce` key, so a whole
//! typing session is one "Edit Type" history step and automation sees exactly what the user does.
//! Offsets in [`TextEdit`] are character indices (the engine's unit); the layout works in bytes.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::{Arc, PoisonError};

use egui::ime_context::{CommittedText, ImeContextTracker, ImeMutationStamp, ImeObservation, ImeTarget};
use egui::{Color32, Pos2, Stroke};
use photocraft_doc::{Document, LayerContent, LayerId, TextLayer};
use photocraft_geom::{Affine, Point};
use photocraft_text::TextLayout;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::state::{BoxResize, TextEdit};

pub const PLACEHOLDER: &str = "Lorem Ipsum";

/// Per-editor IME owner/generation bookkeeping for the custom Type tool.
///
/// The tracker is shared by every viewport and keeps the context generation alive when this
/// editor is replaced. The overlay records the pass's canvas mapping and the pass end recomputes
/// the final geometry from the observed model, so an event-free or discarded pass never replays
/// stale text.
#[derive(Debug, Default)]
pub(crate) struct ImeState {
    tracker: ImeContextTracker,
    frame: Option<ImeFrame>,
    /// The canvas mapping of the current pass, so the final descriptor geometry is recomputed
    /// from the observed model rather than from an earlier point in the pass.
    view: Option<ImeView>,
    /// The last descriptor identity the platform actually received (a discarded pass did not
    /// receive it, so its transition must still be reported by the final pass).
    emitted: Option<ImeEmitted>,
    /// Ask the platform to cancel the native composition (owner/session transition or a stale
    /// pre-edit). Stays set until a non-discarded descriptor carries it.
    interrupt: bool,
    /// Incoming composition provenance survives idle/discarded passes and an empty preedit.
    /// A stale owner stays fenced until the exact host reset completion arrives.
    incoming: Option<IncomingComposition>,
    /// The composition text was invalidated; this pass must report `Unavailable` rather than
    /// presenting the orphaned composition as committed text.
    stale: bool,
    /// The exact composition written into the layer, used to check the stored range before any
    /// replacement, deletion, underline or surrounding-text report.
    preedit: Option<ImePreedit>,
    /// A local fence is released only by its exact host completion, never by terminal text.
    resets: HashMap<egui::ViewportId, ResetPhase>,
    context: Option<egui::Context>,
    needs_request: bool,
    /// One exclusive event owner per actual UI pass; logic-only ticks never replay input.
    raw_pass: HashMap<egui::ViewportId, u64>,
    custom_pass: HashMap<egui::ViewportId, (u64, Option<ResetContext>)>,
    last_custom: HashMap<egui::ViewportId, ResetContext>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ResetContext(egui::Id, u64, egui::IMEPurpose);

impl From<&egui::output::IMEOutput> for ResetContext {
    fn from(ime: &egui::output::IMEOutput) -> Self {
        Self(ime.owner, ime.context_generation, ime.purpose)
    }
}

#[derive(Clone, Copy, Debug)]
enum ResetPhase {
    NeedsRequest,
    Awaiting { id: egui::ImeResetId, destination: IncomingSelection, context: Option<ResetContext> },
    Suspended,
    AllocationFailed { destination: IncomingSelection, context: Option<ResetContext> },
}

#[derive(Clone, Copy, Debug)]
struct ImeFrame {
    viewport: egui::ViewportId,
    frame_nr: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ImeEmitted {
    target: ImeTarget,
    generation: u64,
}

#[derive(Clone, Copy, Debug)]
struct ImeView {
    viewport: egui::ViewportId,
    pass_nr: u64,
    xf: ViewXform,
}

/// The composition text and the edit session that wrote it. A range that still lies inside the
/// text is not enough: an application edit can shift it onto committed characters.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ImePreedit {
    layer: u64,
    session: String,
    text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct IncomingSelection {
    target: ImeTarget,
    caret: usize,
    anchor: usize,
    revision: u64,
}

#[derive(Clone, Copy, Debug)]
enum IncomingComposition {
    Active(IncomingSelection),
    TerminalPending(IncomingSelection),
    Stale,
}

/// A bounded committed snapshot captured from the type layer.
///
/// The snapshot owns at most `MAX_IME_SURROUNDING_TEXT_BYTES`; the layer's whole text is only
/// borrowed while it is built. The current pre-edit lives in the layer so it lays out like
/// typed text, and is removed from this value rather than reported as committed text.
struct ImeCommitted {
    snapshot: egui::output::IMESurroundingText,
}

impl ImeCommitted {
    fn stamp(&self) -> ImeMutationStamp {
        ImeMutationStamp::from_surrounding_text(self.snapshot.clone())
    }
}

impl ImeState {
    /// Remember the frame used by this pass's accepted IME receipts.
    fn begin_frame(&mut self, ctx: &egui::Context) {
        let viewport = ctx.viewport_id();
        self.frame = Some(ImeFrame { viewport, frame_nr: ctx.cumulative_frame_nr_for(viewport) });
    }

    /// Record the canvas mapping of this pass. `sync_ime` recomputes both rectangles from the
    /// model after every panel has run.
    fn note_view(&mut self, viewport: egui::ViewportId, pass_nr: u64, xf: &ViewXform) {
        self.view = Some(ImeView { viewport, pass_nr, xf: *xf });
    }

    /// Request the native composition reset; incoming ownership is tracked separately.
    fn request_interrupt(&mut self) {
        self.interrupt = true;
        if self.incoming.is_some() || !self.resets.is_empty() {
            self.needs_request = true;
            // Context-less commands must retire every old id synchronously.
            for phase in self.resets.values_mut() {
                *phase = ResetPhase::NeedsRequest;
            }
        }
    }

    /// Forget a composition whose range no longer describes the recorded text.
    fn invalidate_preedit(&mut self) {
        self.preedit = None;
        self.stale = true;
        self.incoming = Some(IncomingComposition::Stale);
        self.request_interrupt();
    }

    fn note_preedit(&mut self, ed: &TextEdit, text: &str) {
        self.preedit = Some(ImePreedit { layer: ed.layer, session: ed.session.clone(), text: text.to_owned() });
    }

    fn clear_preedit(&mut self) {
        self.preedit = None;
    }
}

/// The immutable owner of the Type tool's IME context.
///
/// The document, layer and viewport identify the target; the edit session is the replacement
/// epoch, so committing and editing the same layer again cannot reuse the old generation.
fn ime_target(app: &PhotocraftApp, ed: &TextEdit, viewport: egui::ViewportId) -> Option<ImeTarget> {
    let document = ed.document.or_else(|| active_doc_id(app))?;
    // A Type Mask session is a distinct owner: its hidden working layer can have the same id as
    // a regular type layer in another tool session. The session epoch replaces the context when
    // an owner is recreated.
    let owner = egui::Id::unique(("photocraft-type-ime", document, ed.layer, ed.mask, viewport.0.value()));
    let epoch = egui::Id::unique(("photocraft-type-ime-session", &ed.session)).value().max(1);
    Some(ImeTarget::new(owner, epoch))
}

fn incoming_selection(app: &PhotocraftApp, viewport: egui::ViewportId) -> Option<IncomingSelection> {
    let ed = app.ui.text_edit.as_ref()?;
    Some(IncomingSelection { target: ime_target(app, ed, viewport)?, caret: ed.caret, anchor: ed.anchor, revision: app.session.active()?.revision })
}

/// Settle the already-visible preview where it was typed, never replay it at a moved caret.
fn interrupt_incoming(app: &mut PhotocraftApp) {
    if !matches!(app.type_ime.incoming, Some(IncomingComposition::Active(_) | IncomingComposition::TerminalPending(_))) {
        return;
    }
    app.type_ime.incoming = Some(IncomingComposition::Stale);
    if let Some(ed) = &mut app.ui.text_edit {
        ed.preedit = None;
    }
    app.type_ime.clear_preedit();
    app.type_ime.request_interrupt();
}

fn interrupt_action(app: &mut PhotocraftApp, viewport: egui::ViewportId) {
    let guarded = reset_guarded(app, viewport);
    interrupt_incoming(app);
    if guarded {
        app.type_ime.request_interrupt();
    }
}

fn sync_incoming(app: &mut PhotocraftApp, viewport: egui::ViewportId) {
    let destination = incoming_selection(app, viewport);
    if app.type_ime.resets.get(&viewport).is_some_and(|phase| match phase {
        ResetPhase::Awaiting { destination: old, .. } | ResetPhase::AllocationFailed { destination: old, .. } => Some(*old) != destination,
        _ => false,
    }) {
        app.type_ime.request_interrupt();
    }
    let expected = match app.type_ime.incoming {
        Some(IncomingComposition::Active(expected) | IncomingComposition::TerminalPending(expected)) => expected,
        _ => return,
    };
    let lost_preview = matches!(app.type_ime.incoming, Some(IncomingComposition::Active(_))) && app.ui.text_edit.as_ref().is_none_or(|ed| ed.preedit.is_none());
    if incoming_selection(app, viewport) != Some(expected) || lost_preview {
        // An external edit can move an in-bounds preview range onto committed characters.
        // Validate it before interruption clears the metadata needed to detect that mismatch.
        // A valid preview still settles normally; never inspect another document's layer here.
        if let Some(ed) = app.ui.text_edit.clone()
            && ed.preedit.is_some()
            && ime_target(app, &ed, viewport) == Some(expected.target)
            && let Some(active) = app.session.active_index()
            && edit_home(app, &ed) == Some(active)
            && let Some(text) = current_text(app, LayerId(ed.layer))
            && reconcile_preedit(app, &ed, &text)
        {
            return;
        }
        interrupt_incoming(app);
    }
}

/// Accepted IME mutations update provenance without canceling their own Delete→Commit chain.
fn remember_incoming(app: &mut PhotocraftApp, preedit_empty: Option<bool>) {
    let viewport = app.type_ime.frame.map_or(egui::ViewportId::ROOT, |frame| frame.viewport);
    let Some(expected) = incoming_selection(app, viewport) else { return };
    let next = match (preedit_empty, app.type_ime.incoming) {
        (Some(false), _) | (None, Some(IncomingComposition::Active(_))) => IncomingComposition::Active(expected),
        (Some(true), _) | (None, Some(IncomingComposition::TerminalPending(_))) => IncomingComposition::TerminalPending(expected),
        _ => return,
    };
    app.type_ime.incoming = Some(next);
}

/// A host may recreate its Context while retaining the application. Old pass counters and
/// output keys are not evidence of publication in that new Context.
fn bind_reset_context(app: &mut PhotocraftApp, ctx: &egui::Context) {
    // `on_begin_pass` callbacks registered during this pass cannot mark the pass already
    // running. Reinstall here so a replacement Context is ready for its next pass, and rely on
    // the host's creator/`setup_context` having installed before the very first `run_ui`.
    install_reset_hooks(ctx);
    if app.type_ime.context.as_ref().is_some_and(|previous| previous != ctx) {
        interrupt_incoming(app);
        for phase in app.type_ime.resets.values_mut() {
            *phase = ResetPhase::Suspended;
        }
        app.type_ime.needs_request = false;
        app.type_ime.raw_pass.clear();
        app.type_ime.custom_pass.clear();
        app.type_ime.last_custom.clear();
        app.type_ime.view = None;
        app.type_ime.frame = None;
        app.type_ime.tracker.observe(ctx.viewport_id(), None);
    }
    app.type_ime.context = Some(ctx.clone());
}

fn ui_mode_id(viewport: egui::ViewportId) -> egui::Id {
    egui::Id::unique(("photocraft-type-reset-ui", viewport))
}

/// Install before the first UI pass on every Context, including a replacement. These callbacks
/// retain neither the app nor its Context. Calling this from inside a running pass only affects
/// subsequent passes: egui's begin-pass callback list is already being iterated.
pub(crate) fn install_reset_hooks(ctx: &egui::Context) {
    let installed = egui::Id::unique("photocraft-type-reset-hooks");
    if ctx.data(|data| data.get_temp::<bool>(installed)).unwrap_or(false) {
        return;
    }
    ctx.data_mut(|data| data.insert_temp(installed, true));
    ctx.on_begin_pass(
        "photocraft type reset UI mode",
        Arc::new(|ui| {
            let key = ui_mode_id(ui.ctx().viewport_id());
            ui.ctx().data_mut(|data| data.insert_temp(key, true));
        }),
    );
    ctx.on_end_pass(
        "photocraft type reset UI mode end",
        Arc::new(|ui| {
            let key = ui_mode_id(ui.ctx().viewport_id());
            ui.ctx().data_mut(|data| data.insert_temp(key, false));
        }),
    );
}

pub(crate) fn in_ui_pass(ctx: &egui::Context) -> bool {
    let key = ui_mode_id(ctx.viewport_id());
    ctx.data(|data| data.get_temp::<bool>(key)).unwrap_or(false)
}

fn reset_guarded(app: &PhotocraftApp, viewport: egui::ViewportId) -> bool {
    app.type_ime.needs_request || app.type_ime.resets.contains_key(&viewport) || matches!(app.type_ime.incoming, Some(IncomingComposition::Stale))
}

fn reset_route_eligible(app: &PhotocraftApp, ctx: &egui::Context) -> bool {
    // The actual app draws only after `logic_frame` marks fonts ready; `ui_frame` early-returns
    // on the first frame. Direct handler tests that have not run the first logic frame use the
    // pre-styled app state and are not an actual UI route.
    (app.fonts_ready || !app.styled)
        && app.ui.text_edit.is_some()
        && app.ui.dialogs.is_empty()
        && !crate::screen_picker::busy(ctx)
        && ctx.input(|i| i.focused)
        && !ctx.text_edit_focused()
        && !ctx.egui_wants_keyboard_input()
}

/// Whether this input can be attributed to a visible custom descriptor right now. Completion
/// arrives before the final descriptor exists, so use the last canvas mapping and the current
/// model: requiring this pass's final output here would deadlock the ordered suffix.
fn reset_geometry_available(app: &mut PhotocraftApp, viewport: egui::ViewportId) -> bool {
    let Some(ed) = app.ui.text_edit.clone() else { return false };
    let Some(view) = app.type_ime.view.filter(|view| view.viewport == viewport) else { return false };
    ime_geometry(app, &ed, view.xf).is_some()
}

fn suspend_reset(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let viewport = ctx.viewport_id();
    if reset_guarded(app, viewport) {
        app.type_ime.resets.insert(viewport, ResetPhase::Suspended);
        app.type_ime.needs_request = false;
    }
    app.type_ime.last_custom.remove(&viewport);
}

fn allocate_reset(app: &mut PhotocraftApp, ctx: &egui::Context, destination: IncomingSelection, context: Option<ResetContext>) {
    let viewport = ctx.viewport_id();
    let phase = match ctx.request_ime_reset(viewport) {
        Some(id) => ResetPhase::Awaiting { id, destination, context },
        None => ResetPhase::AllocationFailed { destination, context },
    };
    app.type_ime.resets.insert(viewport, phase);
    app.type_ime.needs_request = false;
}

/// Reconcile before the next raw element, not merely at descriptor publication.
fn prepare_reset(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let viewport = ctx.viewport_id();
    sync_incoming(app, viewport);
    if app.type_ime.needs_request {
        app.type_ime.resets.insert(viewport, ResetPhase::NeedsRequest);
        app.type_ime.needs_request = false;
    }
    // Allocation belongs to the true UI tail: a stock field drawn later in this pass
    // must never inherit a speculative custom-editor reset request.
}

fn complete_reset(app: &mut PhotocraftApp, ctx: &egui::Context, request_id: egui::ImeResetId) {
    let viewport = ctx.viewport_id();
    let Some(ResetPhase::Awaiting { id, destination, context }) = app.type_ime.resets.get(&viewport).copied() else { return };
    if request_id != id
        || request_id.viewport_id() != viewport
        || !reset_route_eligible(app, ctx)
        || incoming_selection(app, viewport) != Some(destination)
        || context != app.type_ime.last_custom.get(&viewport).copied()
    {
        return;
    }
    if !reset_geometry_available(app, viewport) {
        // Retire the old id and keep the following preedit blocked before it can reach the
        // editor. The UI tail may still rearm once this pass produces real custom output.
        suspend_reset(app, ctx);
        return;
    }
    app.type_ime.resets.remove(&viewport);
    app.type_ime.incoming = None;
    app.type_ime.preedit = None;
    app.type_ime.stale = false;
    app.type_ime.interrupt = false;
    if let Some(ed) = app.ui.text_edit.as_mut() {
        ed.preedit = None;
    }
}

fn claim_raw_pass(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    let viewport = ctx.viewport_id();
    let pass = ctx.cumulative_pass_nr_for(viewport);
    app.type_ime.raw_pass.insert(viewport, pass) != Some(pass)
}

/// The exclusive route when no custom editor may mutate. Do not consume stock text/keys.
pub(crate) fn controls_only(app: &mut PhotocraftApp, ctx: &egui::Context) {
    bind_reset_context(app, ctx);
    if !claim_raw_pass(app, ctx) {
        return;
    }
    if !reset_route_eligible(app, ctx) {
        interrupt_incoming(app);
    }
    suspend_reset(app, ctx); // Retire an old id before visiting even the first marker.
    for event in ctx.input(|i| i.events.clone()) {
        if let egui::Event::ImeResetCompleted { request_id } = event {
            complete_reset(app, ctx, request_id);
        }
    }
}

/// True outer UI tail, including discarded/early-return passes and host-side mutations.
pub(crate) fn finish_reset_pass(app: &mut PhotocraftApp, ctx: &egui::Context) {
    bind_reset_context(app, ctx);
    let viewport = ctx.viewport_id();
    if app.type_ime.raw_pass.get(&viewport).copied() != Some(ctx.cumulative_pass_nr_for(viewport)) {
        controls_only(app, ctx);
    }
    sync_incoming(app, viewport);
    let produced = app.type_ime.custom_pass.get(&viewport).copied().filter(|(pass, _)| *pass == ctx.cumulative_pass_nr_for(viewport)).and_then(|(_, key)| key);
    let actual = ctx.output(|o| o.ime.as_ref().map(ResetContext::from));
    let Some(context) = produced.filter(|key| Some(*key) == actual && reset_route_eligible(app, ctx)) else {
        suspend_reset(app, ctx);
        return;
    };
    // Never allocate a request for output egui will throw away. Keep any existing guard and its
    // queued R1 blocked; the final pass either publishes this context and reuses it or retires
    // the old id through the ineligible branch above.
    if ctx.will_discard() {
        return;
    }
    app.type_ime.last_custom.insert(viewport, context);
    if !reset_guarded(app, viewport) {
        return;
    }
    let Some(destination) = incoming_selection(app, viewport) else {
        suspend_reset(app, ctx);
        return;
    };
    let unchanged = app.type_ime.resets.get(&viewport).is_some_and(|phase| match phase {
        ResetPhase::Awaiting { destination: old, context: old_context, .. } | ResetPhase::AllocationFailed { destination: old, context: old_context } => {
            *old == destination && *old_context == Some(context) && !app.type_ime.needs_request
        }
        _ => false,
    });
    if !unchanged {
        allocate_reset(app, ctx, destination, Some(context));
    }
}

pub(crate) fn finish_reset_logic(app: &mut PhotocraftApp, ctx: &egui::Context) {
    bind_reset_context(app, ctx);
    if !in_ui_pass(ctx) {
        sync_incoming(app, ctx.viewport_id());
        suspend_reset(app, ctx);
    }
}

/// UTF-8 byte offsets of the caret, anchor and pre-edit, found in one traversal.
///
/// The pre-edit is in characters in [`TextEdit`]; surround-text offsets are UTF-8 bytes. A range
/// that runs past the text is an explicit unavailable state, not a reason to report the
/// composition as committed.
struct ImeOffsets {
    cursor: usize,
    anchor: usize,
    preedit: Option<Range<usize>>,
}

fn ime_offsets(text: &str, ed: &TextEdit) -> Option<ImeOffsets> {
    let (start, end) = match ed.preedit {
        Some((start, len)) => (start, start.checked_add(len)?),
        None => (0, 0),
    };
    let wanted = [ed.caret, ed.anchor, start, end];
    let mut bytes = [usize::MAX; 4];
    let mut chars = 0usize;
    for (index, (byte, _)) in text.char_indices().enumerate() {
        for (slot, want) in bytes.iter_mut().zip(wanted) {
            if *slot == usize::MAX && want == index {
                *slot = byte;
            }
        }
        chars = index.saturating_add(1);
    }
    // An index at (or past) the end clamps to the end, as before; a pre-edit past the end is a
    // stale composition and makes the surrounding text unavailable.
    for (slot, (index, want)) in bytes.iter_mut().zip(wanted.into_iter().enumerate()) {
        if *slot == usize::MAX {
            // The caret and anchor clamp after undo/redo; a pre-edit beyond the text is stale.
            if index >= 2 && want > chars {
                return None;
            }
            *slot = text.len();
        }
    }
    let preedit = match ed.preedit {
        Some(_) if end > chars => return None,
        Some(_) => Some(bytes[2]..bytes[3]),
        None => None,
    };
    Some(ImeOffsets { cursor: bytes[0], anchor: bytes[1], preedit })
}

/// Whether the stored character range still contains exactly the composition that wrote it.
fn ime_preedit_is_current(app: &PhotocraftApp, text: &str, ed: &TextEdit, offsets: &ImeOffsets) -> bool {
    match (ed.preedit, offsets.preedit.as_ref()) {
        (None, _) => true,
        (Some(_), Some(range)) => {
            app.type_ime.preedit.as_ref().is_some_and(|p| p.layer == ed.layer && p.session == ed.session && text.get(range.clone()) == Some(p.text.as_str()))
        }
        (Some(_), None) => false,
    }
}

/// Check the stored pre-edit against the live text, invalidating it (and the native composition)
/// when an application edit moved or replaced it. Returns true when it was stale.
fn reconcile_preedit(app: &mut PhotocraftApp, ed: &TextEdit, text: &str) -> bool {
    if ed.preedit.is_none() {
        return false;
    }
    let current = ime_offsets(text, ed).is_some_and(|offset| ime_preedit_is_current(app, text, ed, &offset));
    if current {
        return false;
    }
    app.type_ime.invalidate_preedit();
    if let Some(e) = app.ui.text_edit.as_mut() {
        e.preedit = None;
    }
    true
}

/// A borrowed observation for the tracker. It copies only the bounded window.
fn committed_text<'a>(app: &PhotocraftApp, text: &'a str, ed: &TextEdit) -> Option<CommittedText<'a>> {
    let offsets = ime_offsets(text, ed)?;
    if !ime_preedit_is_current(app, text, ed, &offsets) {
        return None;
    }
    let committed = CommittedText::new(text, offsets.cursor, offsets.anchor);
    Some(match offsets.preedit {
        Some(preedit) => committed.with_preedit(preedit),
        None => committed,
    })
}

/// The bounded committed snapshot of the edited layer, or `None` when it cannot be exposed.
fn ime_committed(app: &PhotocraftApp, ed: &TextEdit) -> Option<ImeCommitted> {
    let text = text_layer(&app.session.active()?.doc, LayerId(ed.layer))?.text.as_str();
    let offsets = ime_offsets(text, ed)?;
    if !ime_preedit_is_current(app, text, ed, &offsets) {
        return None;
    }
    Some(ImeCommitted { snapshot: egui::output::IMESurroundingText::from_text(text, offsets.cursor, offsets.anchor, offsets.preedit) })
}

/// Report an accepted IME commit/deletion. Call only after the engine accepted the mutation and
/// from the snapshots immediately around it.
fn note_ime_apply(app: &mut PhotocraftApp, ed: &TextEdit, before: &ImeCommitted, after: &ImeCommitted) -> bool {
    let Some(frame) = app.type_ime.frame else { return false };
    let Some(target) = ime_target(app, ed, frame.viewport) else { return false };
    app.type_ime.tracker.note_input_method_apply(frame.viewport, frame.frame_nr, target, before.stamp(), after.stamp())
}

/// Report an application-origin edit (typing, paste, cut, deletion, undo or a toolbar action).
fn note_ime_application_change(app: &mut PhotocraftApp, ed: &TextEdit) -> bool {
    let Some(frame) = app.type_ime.frame else { return false };
    let Some(target) = ime_target(app, ed, frame.viewport) else { return false };
    app.type_ime.tracker.note_application_change(frame.viewport, frame.frame_nr, target)
}

/// Snapshot after the engine mutation and describe the current pass.
///
/// Run once per UI pass, after the editor has drawn its caret, so the descriptor reflects this
/// pass's final model state. With no visible focused editor the context is ended; a moved focus
/// therefore cannot leave the previous owner's surrounding text latched.
pub(crate) fn sync_ime(app: &mut PhotocraftApp, ctx: &egui::Context) {
    bind_reset_context(app, ctx);
    let viewport = ctx.viewport_id();
    let frame_nr = ctx.cumulative_frame_nr_for(viewport);
    let pass_nr = ctx.cumulative_pass_nr_for(viewport);
    app.type_ime.custom_pass.insert(viewport, (pass_nr, None));
    sync_incoming(app, viewport);
    let Some(mut ed) = app.ui.text_edit.clone() else {
        app.type_ime.view = None;
        app.type_ime.tracker.observe(viewport, None);
        return;
    };
    // An application edit (control channel, undo, a menu command) can move the stored composition
    // range onto committed characters. Check the actual composition text before reporting it.
    if ed.preedit.is_some()
        && let Some(text) = current_text(app, LayerId(ed.layer))
        && reconcile_preedit(app, &ed, &text)
        && let Some(current) = app.ui.text_edit.clone()
    {
        ed = current;
    }
    let view = app.type_ime.view.filter(|view| view.viewport == viewport && view.pass_nr == pass_nr);
    let visible = view.is_some() && app.session.active_index().is_some() && edit_home(app, &ed) == app.session.active_index();
    let focused = visible && reset_route_eligible(app, ctx);
    if !focused {
        interrupt_incoming(app);
        if let Some(current) = &app.ui.text_edit {
            ed = current.clone();
        }
    }
    let Some(target) = ime_target(app, &ed, viewport) else {
        app.type_ime.tracker.observe(viewport, None);
        return;
    };
    // The platform only resets the native composition from `should_interrupt_composition`; on
    // X11, macOS, Windows and the web a changed owner/generation alone leaves it alive. Latch the
    // last *delivered* target so a discarded pass does not lose the transition.
    if app.type_ime.emitted.is_some_and(|last| last.target != target) {
        app.type_ime.interrupt = true;
    }
    // Final geometry from this pass's model and canvas mapping: a later panel can move the caret
    // after the overlay drew it, and `rect` is the edited layer's extent, not the caret.
    let geometry = if focused { view.and_then(|view| ime_geometry(app, &ed, view.xf)) } else { None };
    // Borrow the layer's logical text for this observation; only the bounded window is copied.
    let committed = if focused && !app.type_ime.stale {
        app.session.active().and_then(|state| text_layer(&state.doc, LayerId(ed.layer))).and_then(|layer| committed_text(app, &layer.text, &ed))
    } else {
        None
    };
    let observation = ImeObservation::new(frame_nr, pass_nr, target).focused(focused).purpose(egui::IMEPurpose::Normal).committed(committed);
    let update = app.type_ime.tracker.observe(viewport, Some(observation));
    // A generation change (focus/refocus, a pass gap, purpose or replacement) also needs the
    // native reset; the tracker reports it only after observing.
    let mut interrupt = app.type_ime.interrupt;
    if let Some(update) = &update
        && app.type_ime.emitted.is_some_and(|last| last.target != target || last.generation != update.context_generation)
    {
        app.type_ime.interrupt = true;
        interrupt = true;
    }
    let mut sent = false;
    let mut sent_generation = None;
    if let (Some(update), Some((rect, cursor_rect))) = (update, geometry) {
        let generation = update.context_generation;
        // Owner and generation accompany even an Unavailable payload. The cause comes from the
        // tracker's accepted-receipt chain, never from the event that happened to be last.
        ctx.output_mut(|out| {
            out.ime = Some(egui::output::IMEOutput {
                owner: update.owner,
                context_generation: update.context_generation,
                purpose: update.purpose,
                rect,
                cursor_rect,
                should_interrupt_composition: interrupt,
                change_cause: update.change_cause,
                surrounding_text: update.surrounding_text,
                reset_request: None,
            });
        });
        let key = ctx.output(|out| out.ime.as_ref().map(ResetContext::from));
        app.type_ime.custom_pass.insert(viewport, (pass_nr, key));
        sent = true;
        sent_generation = Some(generation);
    }
    if sent && !ctx.will_discard() {
        if let Some(generation) = sent_generation {
            app.type_ime.emitted = Some(ImeEmitted { target, generation });
        }
        app.type_ime.interrupt = false;
        app.type_ime.stale = false;
    }
}

/// The final caret and layer rectangles for an IME descriptor, in screen points.
fn ime_geometry(app: &mut PhotocraftApp, ed: &TextEdit, xf: ViewXform) -> Option<(egui::Rect, egui::Rect)> {
    let id = LayerId(ed.layer);
    let (l, aff, text) = layout(app, id)?;
    let shape = app.session.active().and_then(|s| text_layer(&s.doc, id).map(|t| t.shape.clone()));
    let bend = shape.as_ref().and_then(photocraft_text::warp::Warp::on_path);
    let scr_t = |x: f32, y: f32| -> Pos2 {
        let p = aff.apply(Point::new(f64::from(x), f64::from(y)));
        xf.to_screen(p.x as f32, p.y as f32)
    };
    let scr = |x: f32, y: f32| -> Pos2 {
        let (x, y) = l.to_text(x, y);
        match &bend {
            Some(w) => {
                let (x, y) = w.apply(f64::from(x), f64::from(y));
                scr_t(x as f32, y as f32)
            }
            None => scr_t(x, y),
        }
    };
    let (x, top, bot) = l.caret(byte_of(&text, ed.caret));
    let (x, top, bot) = if l.lines.is_empty() { (0.0, -(12.0 * l.px_per_pt.max(1.0)), 3.0) } else { (x, top, bot) };
    let cursor = egui::Rect::from_two_pos(scr(x, top), scr(x, bot)).expand(1.0);
    let doc_bounds =
        app.session.active().and_then(|s| text_layer(&s.doc, id)).and_then(|t| t.cache.as_ref()).map(|c| c.content_bounds()).filter(|r| !r.is_empty());
    let rect = doc_bounds.map(|r| xf.doc_rect(r)).or_else(|| {
        l.bounds().map(|b| {
            let corners = [scr_t(b[0], b[1]), scr_t(b[2], b[1]), scr_t(b[2], b[3]), scr_t(b[0], b[3])];
            let mut min = corners[0];
            let mut max = corners[0];
            for p in corners {
                min = min.min(p);
                max = max.max(p);
            }
            egui::Rect::from_min_max(min, max)
        })
    });
    Some((rect.unwrap_or(cursor).expand(1.0), cursor))
}

fn text_layer(doc: &Document, id: LayerId) -> Option<&TextLayer> {
    match &doc.layer(id)?.content {
        LayerContent::Text(t) => Some(t),
        _ => None,
    }
}

fn byte_of(text: &str, ci: usize) -> usize {
    photocraft_text::byte_index(text, ci)
}

/// Layout of a type layer (cached per document revision) and its text → document transform.
pub fn layout(app: &mut PhotocraftApp, id: LayerId) -> Option<(Arc<TextLayout>, Affine, String)> {
    let dragged = crate::type_transform::current_transform(app, id);
    // Borrows the document in place (no clone): the canvas hover hit-tests type every frame.
    let st = app.session.active()?;
    let t = text_layer(&st.doc, id)?;
    let transform = dragged.unwrap_or(t.transform);
    let key = (st.doc.id.0, st.revision, id.0);
    if let Some((k, l)) = &app.type_layout
        && *k == key
    {
        return Some((l.clone(), transform, t.text.clone()));
    }
    let l = Arc::new(photocraft_text::shared().lock().ok()?.layout(t, st.doc.resolution_dpi));
    app.type_layout = Some((key, l.clone()));
    Some((l, transform, t.text.clone()))
}

fn to_text(aff: &Affine, x: f64, y: f64) -> (f32, f32) {
    let p = aff.inverse().unwrap_or(Affine::IDENTITY).apply(Point::new(x, y));
    (p.x as f32, p.y as f32)
}

/// Topmost shown type layer (it and every group around it visible, #4320) whose laid-out text
/// contains the document point. The press and the canvas hover (`hover_path_target`) share it, so
/// the pointer never promises a path click that the press would turn into editing this layer.
pub(crate) fn hit_layer(app: &mut PhotocraftApp, x: f64, y: f64) -> Option<LayerId> {
    let slop = 6.0 / app.point_zoom().max(0.01);
    // A Type Mask session's text is a hidden layer; it is still the text being edited.
    let editing = app.ui.text_edit.as_ref().filter(|ed| ed.mask).map(|ed| LayerId(ed.layer));
    let mut ids: Vec<LayerId> = {
        let doc = &app.session.active()?.doc;
        doc.walk()
            .into_iter()
            .filter(|(path, _, l)| matches!(l.content, LayerContent::Text(_)) && (editing == Some(l.id) || doc.shown_at(path)))
            .map(|(_, _, l)| l.id)
            .collect()
    };
    ids.reverse(); // walk() is bottom-up; hit the topmost first
    ids.into_iter().find(|id| {
        // The pixels shown count too: a PSD's type keeps Photoshop's rendering until edited,
        // which can sit off our layout when fonts are substituted (#123).
        let shown = app.session.active().and_then(|st| text_layer(&st.doc, *id)).and_then(|t| t.cache.as_ref()).map(|c| c.content_bounds()).is_some_and(|r| {
            let s = f64::from(slop);
            !r.is_empty() && x >= f64::from(r.x0) - s && x <= f64::from(r.x1) + s && y >= f64::from(r.y0) - s && y <= f64::from(r.y1) + s
        });
        // The hover calls this every frame: skip the layout when the pixels already answer.
        if shown {
            return true;
        }
        let Some((l, aff, _)) = layout(app, *id) else { return false };
        let (tx, ty) = to_text(&aff, x, y);
        photocraft_text::text_point_inside(&l, tx, ty, slop)
    })
}

/// True when the layer's pixels are what our engine draws for it. A PSD's type layer keeps
/// Photoshop's pixels until it is edited, and with substituted fonts or a different line layout
/// they sit somewhere else than the glyphs the caret and selection are placed on.
pub(crate) fn shows_own_layout(doc: &Document, t: &TextLayer) -> bool {
    let Some(cache) = &t.cache else { return false };
    let mut eng = photocraft_text::shared().lock().unwrap_or_else(PoisonError::into_inner);
    let ours = eng.render(t, doc.resolution_dpi, doc.pixel_format()).1.surface.content_bounds();
    let have = cache.content_bounds();
    [(ours.x0, have.x0), (ours.y0, have.y0), (ours.x1, have.x1), (ours.y1, have.y1)].iter().all(|(a, b)| (a - b).abs() <= 1)
}

/// Start editing an existing type layer. Like Photoshop, editing shows the text as the type
/// engine lays it out, so a PSD layer is re-rendered first (inside the edit session's history
/// step, so Cancel brings Photoshop's pixels back).
fn begin_edit(app: &mut PhotocraftApp, id: LayerId, key: &str) -> Result<(), String> {
    let st = app.session.active().ok_or("no document open")?;
    let doc = st.doc.clone();
    if let Some(t) = text_layer(&doc, id)
        && !shows_own_layout(&doc, t)
    {
        app.run("type.edit", json!({"layer": id.0, "coalesce": key}))?;
    }
    Ok(())
}

/// Edit the active type layer from its thumbnail, selecting all text like Photoshop. Reuse the
/// current session when already editing it, so Cancel and undo still cover the whole edit.
pub fn edit_active(app: &mut PhotocraftApp) -> Result<(), String> {
    let st = app.session.active().ok_or("no document open")?;
    let id = st.active_layer.ok_or("no active layer")?;
    let n = text_layer(&st.doc, id).ok_or("active layer is not a type layer")?.text.chars().count();
    if app.ui.text_edit.as_ref().is_none_or(|ed| ed.layer != id.0) {
        commit(app);
        let key = session_key(app);
        begin_edit(app, id, &key)?;
        app.ui.text_edit = Some(TextEdit {
            layer: id.0,
            document: active_doc_id(app),
            caret: n,
            anchor: 0,
            session: key,
            created: false,
            mask: false,
            dragging: false,
            resize: None,
            preedit: None,
        });
    } else {
        let mut cancelled = false;
        if let Some(ed) = app.ui.text_edit.as_mut() {
            ed.anchor = 0;
            ed.caret = n;
            // Re-entering the same session moves the caret away from the composition: cancel it
            // rather than reusing a range that no longer describes the text.
            cancelled = ed.preedit.take().is_some();
        }
        if cancelled {
            app.type_ime.clear_preedit();
            interrupt_incoming(app);
            app.type_ime.request_interrupt();
        }
    }
    let vertical = app.session.active().and_then(|st| text_layer(&st.doc, id)).is_some_and(|t| t.orientation == photocraft_doc::text::Orientation::Vertical);
    app.ui.text_edit_return_tool = None;
    app.ui.tool = if vertical { crate::state::Tool::VerticalType } else { crate::state::Tool::Type };
    app.ui.mask_target = false;
    app.ui.vector_mask_target = false;
    Ok(())
}

/// Double-click with the Move tool (Photoshop CC 2019+): edit the type layer under document
/// point (`x`, `y`) with the caret on the glyph there, switching to the Type tool. False when no
/// type is there.
pub fn edit_at(app: &mut PhotocraftApp, x: f64, y: f64) -> bool {
    let Some(id) = hit_layer(app, x, y) else { return false };
    let previous_tool = app.ui.tool;
    commit(app);
    if app.session.select_layer(id).is_err() {
        return false;
    }
    let key = session_key(app);
    if begin_edit(app, id, &key).is_err() {
        return false;
    }
    let off = hit_offset(app, id, x, y);
    app.ui.text_edit = Some(TextEdit {
        layer: id.0,
        document: active_doc_id(app),
        caret: off,
        anchor: off,
        session: key,
        created: false,
        mask: false,
        dragging: false,
        resize: None,
        preedit: None,
    });
    let vertical = app.session.active().and_then(|st| text_layer(&st.doc, id)).is_some_and(|t| t.orientation == photocraft_doc::text::Orientation::Vertical);
    if !previous_tool.is_type() {
        app.ui.text_edit_return_tool = Some(previous_tool);
    }
    app.ui.tool = if vertical { crate::state::Tool::VerticalType } else { crate::state::Tool::Type };
    app.ui.mask_target = false;
    app.ui.vector_mask_target = false;
    true
}

fn hit_offset(app: &mut PhotocraftApp, id: LayerId, x: f64, y: f64) -> usize {
    offset_at(app, id, x, y)
}

/// The character offset of the caret position nearest document point (`x`, `y`) in layer `id`.
pub(crate) fn offset_at(app: &mut PhotocraftApp, id: LayerId, x: f64, y: f64) -> usize {
    let Some((l, aff, text)) = layout(app, id) else { return 0 };
    let (tx, ty) = to_text(&aff, x, y);
    // Type on a path: the point's place along the path is the position in the (single) line.
    let shape = app.session.active().and_then(|s| text_layer(&s.doc, id).map(|t| t.shape.clone()));
    let (tx, ty) = match shape {
        Some(photocraft_doc::text::TextShape::Path { path, start, flip }) => {
            photocraft_text::warp::arc_length_at(&path, flip, f64::from(tx), f64::from(ty)).map_or((tx, ty), |s| ((s - f64::from(start)) as f32, -1.0))
        }
        _ => (tx, ty),
    };
    photocraft_text::hit_char(&l, &text, tx, ty).0
}

fn hex(c: [f32; 4]) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(c[0]), b(c[1]), b(c[2]))
}

/// Smallest paragraph box a handle drag leaves, in text-space px.
const MIN_BOX: f32 = 4.0;

/// Which box edges handle `i` moves: (left, right, top, bottom). The order matches the overlay.
fn handle_sides(i: u8) -> (bool, bool, bool, bool) {
    match i {
        0 => (true, false, true, false),
        1 => (false, true, true, false),
        2 => (false, true, false, true),
        3 => (true, false, false, true),
        4 => (false, false, true, false),
        5 => (false, true, false, false),
        6 => (false, false, false, true),
        _ => (true, false, false, false),
    }
}

pub(crate) fn box_shape(app: &PhotocraftApp, id: LayerId) -> Option<(f32, f32, f32, f32)> {
    match app.session.active().and_then(|s| text_layer(&s.doc, id).map(|t| t.shape.clone()))? {
        photocraft_doc::text::TextShape::Box { x, y, width, height } => Some((x, y, width, height)),
        _ => None,
    }
}

/// The paragraph-box handle under the document point, if the layer is paragraph text.
pub(crate) fn box_handle_at(app: &mut PhotocraftApp, id: LayerId, x: f64, y: f64) -> Option<u8> {
    let (bx, by, w, h) = box_shape(app, id)?;
    let (_, aff, _) = layout(app, id)?;
    let (r, b) = (bx + w, by + h);
    let (mx, my) = (bx + w / 2.0, by + h / 2.0);
    let spots = [(bx, by), (r, by), (r, b), (bx, b), (mx, by), (r, my), (mx, b), (bx, my)];
    let tol = f64::from(6.0 / app.point_zoom().max(0.01));
    spots
        .iter()
        .position(|&(sx, sy)| {
            let p = aff.apply(Point::new(f64::from(sx), f64::from(sy)));
            (p.x - x).abs() <= tol && (p.y - y).abs() <= tol
        })
        .map(|i| i as u8)
}

/// Drag the handle of `r` (the box and transform snapshotted at the press) to the document
/// point: the dragged edges follow it, the others stay put. With ⌥ the opposite edges mirror
/// the drag about the centre the box started with, so the block grows symmetrically instead of
/// from one side — pressing or releasing ⌥ mid-drag switches that on and off as if it had been
/// held from the start.
fn resize_box(app: &mut PhotocraftApp, ed: &TextEdit, r: BoxResize, x: f64, y: f64, alt: bool) {
    let (px, py) = to_text(&r.aff, x, y);
    let (l, rw, t, b) = handle_sides(r.handle);
    let (bx, by, w, h) = (r.shape[0], r.shape[1], r.shape[2], r.shape[3]);
    let (mut x0, mut y0, mut x1, mut y1) = (bx, by, bx + w, by + h);
    // The centre the ⌥ mirror turns about: the box as it stood at the press (every move
    // recomputes from that snapshot, so the centre holds for the whole drag).
    let (cx, cy) = (bx + w / 2.0, by + h / 2.0);
    if l {
        x0 = px.min(x1 - MIN_BOX);
    }
    if rw {
        x1 = px.max(x0 + MIN_BOX);
    }
    if t {
        y0 = py.min(y1 - MIN_BOX);
    }
    if b {
        y1 = py.max(y0 + MIN_BOX);
    }
    if alt {
        // The mirror keeps the centre fixed: the dragged edge clamps at MIN_BOX/2 from it, and
        // the opposite edge follows, so the block never inverts below the minimum.
        if l {
            x0 = px.min(cx - MIN_BOX / 2.0);
            x1 = 2.0 * cx - x0;
        }
        if rw {
            x1 = px.max(cx + MIN_BOX / 2.0);
            x0 = 2.0 * cx - x1;
        }
        if t {
            y0 = py.min(cy - MIN_BOX / 2.0);
            y1 = 2.0 * cy - y0;
        }
        if b {
            y1 = py.max(cy + MIN_BOX / 2.0);
            y0 = 2.0 * cy - y1;
        }
    }
    // type.edit places the box's top-left at the given document point.
    let o = r.aff.apply(Point::new(f64::from(x0), f64::from(y0)));
    let _ = app.run("type.edit", json!({"layer": ed.layer, "box": [o.x, o.y, x1 - x0, y1 - y0], "coalesce": ed.session}));
}

/// True when document point (`x`, `y`) lies inside the paragraph box of type layer `id` (false
/// for point text).
pub(crate) fn inside_box(app: &mut PhotocraftApp, id: LayerId, x: f64, y: f64) -> bool {
    let Some((bx, by, w, h)) = box_shape(app, id) else { return false };
    let Some((_, aff, _)) = layout(app, id) else { return false };
    let (tx, ty) = to_text(&aff, x, y);
    tx >= bx && tx <= bx + w && ty >= by && ty <= by + h
}

/// Pointer down with the Type tool. Returns true when the press was consumed (no box drag).
pub fn pointer_down(app: &mut PhotocraftApp, x: f64, y: f64, mods: egui::Modifiers) -> bool {
    if let Some(ed) = app.ui.text_edit.clone() {
        let id = LayerId(ed.layer);
        if let Some(i) = box_handle_at(app, id, x, y) {
            // Snapshot the box and transform now: every move recomputes from them, so ⌥ held
            // or released mid-drag mirrors about the centre the drag started with.
            let snap = box_shape(app, id).zip(layout(app, id)).map(|((bx, by, w, h), (_, aff, _))| BoxResize { handle: i, shape: [bx, by, w, h], aff });
            if let (Some(snap), Some(e)) = (snap, app.ui.text_edit.as_mut()) {
                e.resize = Some(snap);
            }
            return true;
        }
        // Photoshop: anywhere inside the paragraph box places the caret, even off the glyphs
        // (left of a line's start, below the last line); only a click outside it commits.
        if hit_layer(app, x, y) == Some(id) || inside_box(app, id, x, y) {
            let off = hit_offset(app, id, x, y);
            if let Some(e) = app.ui.text_edit.as_mut() {
                e.caret = off;
                if !mods.shift {
                    e.anchor = off;
                }
                e.dragging = true;
            }
            return true;
        }
        // ⇧/⌥ on that click combine a Type Mask with the selection.
        commit_with(app, mods);
        return true; // Photoshop: a click outside commits without starting new text
    }
    // The Type Mask tools always start new text: existing type layers are just pixels to them;
    // so does a ⇧-click with the Type tool, as in Photoshop, to start new text over old.
    if app.ui.tool.is_type_mask() || mods.shift {
        return false;
    }
    if let Some(id) = hit_layer(app, x, y) {
        let _ = app.session.select_layer(id);
        let key = session_key(app);
        if begin_edit(app, id, &key).is_err() {
            return true;
        }
        let off = hit_offset(app, id, x, y);
        app.ui.text_edit = Some(TextEdit {
            layer: id.0,
            document: active_doc_id(app),
            caret: off,
            anchor: off,
            session: key,
            created: false,
            mask: false,
            dragging: true,
            resize: None,
            preedit: None,
        });
        app.ui.text_edit_return_tool = None;
        return true;
    }
    false
}

pub fn pointer_move(app: &mut PhotocraftApp, x: f64, y: f64, mods: egui::Modifiers) {
    let Some(ed) = app.ui.text_edit.clone() else { return };
    if let Some(r) = ed.resize {
        resize_box(app, &ed, r, x, y, mods.alt);
        return;
    }
    if ed.dragging {
        let off = hit_offset(app, LayerId(ed.layer), x, y);
        if let Some(e) = app.ui.text_edit.as_mut() {
            e.caret = off;
        }
    }
}

/// What a Type tool click on a path does, like Photoshop: inside a closed path, area type;
/// on a path's outline, type on the path from that point.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PathClick {
    Area { shape: Option<u64> },
    OnPath { shape: Option<u64>, subpath: usize, start: f64 },
}

/// Arc length (px) along `sub` from its first anchor to segment `seg` at curve parameter `t`.
fn arc_length_to(sub: &photocraft_doc::Subpath, seg: usize, t: f64) -> f64 {
    const PIECES: usize = 32;
    let eval = photocraft_vector::edit::eval;
    let length = |s: &[Point; 4], upto: f64| {
        (1..=PIECES)
            .fold((0.0, eval(s, 0.0)), |(len, prev), j| {
                let q = eval(s, upto * j as f64 / PIECES as f64);
                (len + (q.x - prev.x).hypot(q.y - prev.y), q)
            })
            .0
    };
    let segs = sub.segments();
    let whole: f64 = segs.iter().take(seg).map(|s| length(s, 1.0)).sum();
    whole + segs.get(seg).map_or(0.0, |s| length(s, t.clamp(0.0, 1.0)))
}

/// The path a Type tool click at document `(x, y)` would use: on the outline (within 6 screen px)
/// of the active shape layer's path, else of the work path, type on that path; inside a closed
/// one, area type. The work path's inside counts only while the canvas draws it (selected in the
/// Paths panel, #6551): a hidden Work Path left after Load as Selection must not turn a plain
/// click into area type. None while View › Show › Target Path (or Extras) hides paths.
pub(crate) fn path_target(app: &PhotocraftApp, x: f64, y: f64) -> Option<PathClick> {
    use photocraft_vector::edit::{Hit, hit};
    if !app.ui.view.shows(app.ui.view.show.target_path) {
        return None;
    }
    let shape_path = crate::vector_ui::active_shape_path(app);
    let work = app.session.active().and_then(|s| s.doc.work_path.clone());
    let work_shown = app.ui.selected_path.as_deref() == Some("work");
    // The canvas hover runs this every frame and `hit` samples every segment, so a path with more
    // anchors than this is not a click target (the click and the pointer agree: plain point
    // text). 4096 anchors is far beyond hand-drawn paths and shapes yet keeps a frame cheap, and
    // it is the most `type.fillPath` accepts, so every area click target can be filled.
    const MAX_TARGET_KNOTS: usize = photocraft_text::area::MAX_AREA_KNOTS;
    let small = |path: &photocraft_doc::vector::Path| path.subpaths.iter().map(|s| s.knots.len()).fold(0usize, usize::saturating_add) <= MAX_TARGET_KNOTS;
    let tol = 6.0 / app.point_zoom().max(0.01) as f64;
    // Only the horizontal Type tool starts type on a path: vertical type on a path is not laid
    // out yet.
    let vertical = app.ui.tool.is_vertical_type();
    let outline = |path: &photocraft_doc::vector::Path, shape: Option<u64>| {
        let on_path = |sp: usize, k: usize, t: f64| {
            // Type on a path needs a segment to run along (`type.attachToPath` rejects one anchor).
            let sub = path.subpaths.get(sp).filter(|s| !vertical && s.knots.len() >= 2)?;
            Some(PathClick::OnPath { shape, subpath: sp, start: arc_length_to(sub, k, t) })
        };
        match hit(path, Point::new(x, y), tol, &[]) {
            Some(Hit::Segment([sp, k], t)) => on_path(sp, k, t),
            Some(Hit::Anchor([sp, k])) => on_path(sp, k, 0.0),
            _ => None,
        }
    };
    let inside = |path: &photocraft_doc::vector::Path| photocraft_text::area::AreaShape::new(path, false).is_some_and(|a| a.contains(x as f32, y as f32));
    if let Some((id, p)) = shape_path.as_ref().filter(|(_, p)| small(p)) {
        if let Some(c) = outline(p, Some(*id)) {
            return Some(c);
        }
        if inside(p) {
            return Some(PathClick::Area { shape: Some(*id) });
        }
    }
    let w = work.as_ref().filter(|w| small(w))?;
    if let Some(c) = outline(w, None) {
        return Some(c);
    }
    (work_shown && inside(w)).then_some(PathClick::Area { shape: None })
}

/// What the Type tool's pointer over document `(x, y)` shows (#218): the path click a press there
/// would make, unless the press edits the type layer under it instead (`pointer_down`: an
/// unshifted press on shown type), where Photoshop keeps the plain I-beam. The type hit is only
/// tested over a path, so a hover elsewhere lays out nothing. Nothing while a type edit is open
/// (a click then commits rather than starting new type). While the button is held it is what the
/// release makes (`pointer_up`): nothing once the drag is a text box, else the path click at the
/// press.
pub(crate) fn hover_path_target(app: &mut PhotocraftApp, x: f64, y: f64, shift: bool) -> Option<PathClick> {
    if app.ui.text_edit.is_some() {
        return None;
    }
    if let Some(start) = app.drag.as_ref().map(|d| d.start) {
        // The press already passed over editing type (`pointer_down`), so no type hit test.
        return if makes_box(app, start, [x, y]) { None } else { path_target(app, start[0], start[1]) };
    }
    let target = path_target(app, x, y)?;
    (shift || hit_layer(app, x, y).is_none()).then_some(target)
}

/// Whether a press at `start` released at `end` drags out a paragraph box rather than clicking
/// (at least 4 screen pixels both ways).
fn makes_box(app: &PhotocraftApp, start: [f64; 2], end: [f64; 2]) -> bool {
    let (w, h) = ((end[0] - start[0]).abs(), (end[1] - start[1]).abs());
    let min = 4.0 / app.point_zoom().max(0.01) as f64;
    w >= min && h >= min
}

/// Pointer up. `rect` is the dragged box (document px) when no edit session consumed the press.
pub fn pointer_up(app: &mut PhotocraftApp, start: [f64; 2], end: [f64; 2]) {
    if let Some(e) = app.ui.text_edit.as_mut() {
        e.dragging = false;
        e.resize = None;
        app.ui.caret_blink_time = app.last_frame_time;
        return;
    }
    let (w, h) = ((end[0] - start[0]).abs(), (end[1] - start[1]).abs());
    let min = 4.0 / app.point_zoom().max(0.01) as f64;
    let o = app.ui.tool_options.clone();
    let mask = app.ui.tool.is_type_mask();
    // A click (not a drag) with the Type tool on a path: area type inside a closed path, type
    // on the path on its outline. The Type Mask tools ignore paths.
    let target = if !mask && !makes_box(app, start, end) { path_target(app, start[0], start[1]) } else { None };
    // Preferences ▸ Type: "Fill new type layers with placeholder text" (on by default). The Type
    // Mask tools start empty, as Photoshop's do.
    let text = if !mask && app.session.prefs().type_.fill_new_type_layers_with_placeholder { PLACEHOLDER } else { "" };
    let mut p = default_style_params(app);
    p["text"] = json!(text);
    p["orientation"] = json!(if app.ui.tool.is_vertical_type() { "vertical" } else { "horizontal" });
    if w >= min && h >= min {
        p["box"] = json!([start[0].min(end[0]).round(), start[1].min(end[1]).round(), w.round(), h.round()]);
    } else {
        p["x"] = json!(start[0].round());
        p["y"] = json!(start[1].round());
    }
    let key = session_key(app);
    p["coalesce"] = json!(key);
    if let Ok(v) = app.run("type.create", p)
        && let Some(id) = v.get("layer").and_then(serde_json::Value::as_u64)
    {
        // A failed follow-up leaves point text; say why instead of passing over it.
        let follow_up = match &target {
            Some(PathClick::Area { shape }) => app.run("type.fillPath", json!({"layer": id, "shape": shape, "coalesce": key})),
            Some(PathClick::OnPath { shape, subpath, start }) => {
                app.run("type.attachToPath", json!({"layer": id, "shape": shape, "subpath": subpath, "start": start, "coalesce": key}))
            }
            None => Ok(serde_json::Value::Null),
        };
        if let Err(e) = follow_up {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        if o.type_aa != "sharp" {
            let _ = app.run("type.edit", json!({"layer": id, "antialias": o.type_aa, "coalesce": key}));
        }
        // A Type Mask's text is a hidden working layer: the canvas shows the mask overlay
        // instead, and committing turns it into the selection.
        if mask {
            let _ = app.run("layer.setProps", json!({"layer": id, "visible": false, "coalesce": key}));
        }
        // Like Photoshop: the placeholder is selected, so typing replaces it.
        let n = text.chars().count();
        app.ui.caret_blink_time = app.last_frame_time;
        app.ui.text_edit = Some(TextEdit {
            layer: id,
            document: active_doc_id(app),
            caret: n,
            anchor: 0,
            session: key,
            created: true,
            mask,
            dragging: false,
            resize: None,
            preedit: None,
        });
        app.ui.text_edit_return_tool = None;
    }
}

fn session_key(app: &mut PhotocraftApp) -> String {
    format!("type-{}", app.ui.alloc_id())
}

fn active_doc_id(app: &PhotocraftApp) -> Option<u64> {
    app.session.active().map(|st| st.doc.id.0)
}

fn current_text(app: &PhotocraftApp, id: LayerId) -> Option<String> {
    Some(text_layer(&app.session.active()?.doc, id)?.text.clone())
}

/// Character count without cloning the layer's text.
fn text_char_count(app: &PhotocraftApp, id: LayerId) -> Option<usize> {
    Some(text_layer(&app.session.active()?.doc, id)?.text.chars().count())
}

/// The selected characters, clamped after undo/redo may have shortened the text.
pub(crate) fn selected_range(app: &PhotocraftApp) -> Option<[usize; 2]> {
    let ed = app.ui.text_edit.as_ref()?;
    let n = text_layer(&app.session.active()?.doc, LayerId(ed.layer))?.text.chars().count();
    let (a, b) = (ed.caret.min(ed.anchor).min(n), ed.caret.max(ed.anchor).min(n));
    (a < b).then_some([a, b])
}

/// Copy the selected characters to the system clipboard. Returns the copied range. Shared by the
/// clipboard Copy event and Edit > Copy during a type edit.
pub(crate) fn copy_selection(app: &PhotocraftApp, ctx: &egui::Context) -> Option<[usize; 2]> {
    let [a, b] = selected_range(app)?;
    let ed = app.ui.text_edit.as_ref()?;
    let text = text_layer(&app.session.active()?.doc, LayerId(ed.layer))?;
    ctx.copy_text(text.text.chars().skip(a).take(b - a).collect());
    Some([a, b])
}

/// Edit > Paste during a type edit: ask the window for the clipboard text, which arrives as a
/// Paste event next frame and replaces the selection there (like ⌘V).
pub(crate) fn request_paste(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
}

/// Select > All during a type edit selects every character (like ⌘A).
pub(crate) fn select_all(app: &mut PhotocraftApp) {
    let Some(n) = app.ui.text_edit.as_ref().and_then(|ed| current_text(app, LayerId(ed.layer))).map(|t| t.chars().count()) else { return };
    if let Some(e) = app.ui.text_edit.as_mut() {
        (e.anchor, e.caret) = (0, n);
    }
}

/// Shared by the clipboard Cut event and Edit > Cut during a type edit.
pub(crate) fn cut_selection(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some([a, b]) = copy_selection(app, ctx) else { return };
    if let Some(ed) = app.ui.text_edit.as_mut() {
        ed.anchor = a;
        ed.caret = b;
    }
    insert(app, "");
}

/// Replace the selection with `s`.
fn insert(app: &mut PhotocraftApp, s: &str) {
    let Some(ed) = app.ui.text_edit.clone() else { return };
    let (a, b) = (ed.caret.min(ed.anchor), ed.caret.max(ed.anchor));
    let s = s.replace("\r\n", "\n").replace('\r', "\n");
    if a == b && s.is_empty() {
        return;
    }
    crate::type_transform::finish(app);
    let applied = app.run("type.edit", json!({"layer": ed.layer, "replace": {"start": a, "end": b, "text": s}, "coalesce": ed.session})).is_ok();
    if applied {
        let mut cancelled = false;
        if let Some(e) = app.ui.text_edit.as_mut() {
            e.caret = a + s.chars().count();
            e.anchor = e.caret;
            // An application edit invalidates a live composition: its recorded range no longer
            // describes the text, and its pending native commit must not be applied here.
            cancelled = e.preedit.take().is_some();
        }
        if cancelled {
            app.type_ime.clear_preedit();
            interrupt_incoming(app);
            app.type_ime.request_interrupt();
        }
        // Ordinary typing, paste, cut and deletion are application-origin, never IME receipts.
        note_ime_application_change(app, &ed);
    }
}

/// Typed text (not a paste): with Preferences ▸ Type ▸ Use Smart Quotes, straight quotes become
/// typographic (Photoshop does not rewrite pasted text).
fn insert_typed(app: &mut PhotocraftApp, s: &str) {
    if !app.session.prefs().type_.smart_quotes {
        return insert(app, s);
    }
    let prev = app.ui.text_edit.clone().and_then(|ed| {
        let at = ed.caret.min(ed.anchor);
        let text = current_text(app, LayerId(ed.layer))?;
        (at > 0).then(|| text.chars().nth(at - 1)).flatten()
    });
    insert(app, &smart_quotes(s, prev));
}

/// Straight quotes become “ ” and ‘ ’: opening after whitespace, a bracket or a dash (or at the
/// start of the text), closing next to a letter or another quote. `prev` is the character before
/// the caret.
fn smart_quotes(s: &str, mut prev: Option<char>) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let c = match c {
            '"' => {
                if opens_quote(prev) {
                    '\u{201c}'
                } else {
                    '\u{201d}'
                }
            }
            '\'' => {
                if opens_quote(prev) {
                    '\u{2018}'
                } else {
                    '\u{2019}'
                }
            }
            _ => c,
        };
        out.push(c);
        prev = Some(c);
    }
    out
}

fn opens_quote(prev: Option<char>) -> bool {
    prev.is_none_or(|p| p.is_whitespace() || matches!(p, '(' | '[' | '{' | '-' | '\u{2013}' | '\u{2014}'))
}

/// Alt+←/→: kern the pair before the caret by `by` (1/1000 em). No pair (caret at a text or line
/// edge): nothing happens, like Photoshop. Not coalesced: one history step per press.
fn kern_pair(app: &mut PhotocraftApp, id: LayerId, caret: usize, by: f32) {
    let Some(text) = current_text(app, id) else { return };
    let before = caret.checked_sub(1).and_then(|i| text.chars().nth(i));
    let after = text.chars().nth(caret);
    if before.is_none_or(|c| c == '\n') || after.is_none_or(|c| c == '\n') {
        return;
    }
    crate::type_transform::finish(app);
    let _ = app.run("type.edit", json!({"layer": id.0, "kernPair": {"at": caret, "by": by}}));
}

/// A character style the type editor's ⇧⌘ keys toggle (Photoshop: ⇧⌘K All Caps, ⇧⌘U
/// Underline, ⇧⌘B Faux Bold, ⇧⌘I Faux Italic, ⇧⌘/ Strikethrough).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CharToggle {
    AllCaps,
    Underline,
    FauxBold,
    FauxItalic,
    Strikethrough,
}

impl CharToggle {
    fn is_on(self, st: &photocraft_doc::text::CharStyle) -> bool {
        match self {
            Self::AllCaps => st.caps == photocraft_doc::text::Caps::AllCaps,
            Self::Underline => st.underline,
            Self::FauxBold => st.faux_bold,
            Self::FauxItalic => st.faux_italic,
            Self::Strikethrough => st.strikethrough,
        }
    }

    fn param(self, on: bool) -> (&'static str, serde_json::Value) {
        match self {
            Self::AllCaps => ("caps", json!(if on { "all" } else { "normal" })),
            Self::Underline => ("underline", json!(on)),
            Self::FauxBold => ("fauxBold", json!(on)),
            Self::FauxItalic => ("fauxItalic", json!(on)),
            Self::Strikethrough => ("strikethrough", json!(on)),
        }
    }
}

/// The byte range of characters `a..b` of `text`.
fn byte_range(text: &str, a: usize, b: usize) -> (usize, usize) {
    let at = |c: usize| text.char_indices().nth(c).map_or(text.len(), |(i, _)| i);
    (at(a.min(b)), at(a.max(b)))
}

/// The character styles of the selection, or of the typing style when the text is empty. Empty
/// when nothing is selected in non-empty text.
fn selected_styles(t: &TextLayer, a: usize, b: usize) -> Vec<photocraft_doc::text::CharStyle> {
    let runs = t.char_runs();
    if t.text.is_empty() {
        return runs.into_iter().take(1).map(|r| r.style).collect();
    }
    let (ba, bb) = byte_range(&t.text, a, b);
    let mut at = 0usize;
    let mut out = Vec::new();
    for r in runs {
        let (rs, re) = (at, at.saturating_add(r.len));
        at = re;
        if rs < bb && re > ba {
            out.push(r.style);
        }
    }
    out
}

/// The character range the type editor's formatting keys act on: the selection, or the empty
/// text's typing style. `None` for a collapsed caret in non-empty text: Photoshop would style the
/// next characters typed, which the document model can't hold apart from the text, so the keys
/// do nothing there (rather than restyle a word the user didn't select).
fn format_range(app: &PhotocraftApp, ed: &TextEdit) -> Option<(usize, usize)> {
    let t = text_layer(&app.session.active()?.doc, LayerId(ed.layer))?;
    let n = t.text.chars().count();
    let (a, b) = (ed.caret.min(ed.anchor).min(n), ed.caret.max(ed.anchor).min(n));
    (a < b || n == 0).then_some((a, b))
}

/// One `type.setStyle` step on the edited layer's `range`, kept apart from the typing session's
/// coalesced edit so Undo takes back one key press at a time (as with kerning).
fn set_style(app: &mut PhotocraftApp, ed: &TextEdit, range: (usize, usize), mut p: serde_json::Value) {
    crate::type_transform::finish(app);
    p["layer"] = json!(ed.layer);
    p["range"] = json!([range.0, range.1]);
    if let Err(e) = app.run("type.setStyle", p) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// ⇧⌘K/U/B/I/: flips a character style over the selection: off when every selected character
/// already has it, else on.
pub(crate) fn toggle_char_style(app: &mut PhotocraftApp, which: CharToggle) {
    let Some(ed) = app.ui.text_edit.clone() else { return };
    let Some(range) = format_range(app, &ed) else { return };
    let Some(styles) = app.session.active().and_then(|s| text_layer(&s.doc, LayerId(ed.layer))).map(|t| selected_styles(t, range.0, range.1)) else {
        return;
    };
    let on = !styles.iter().all(|st| which.is_on(st));
    let (key, value) = which.param(on);
    set_style(app, &ed, range, json!({ key: value }));
}

/// ⌥↑/↓ leading, ⇧⌥↑/↓ baseline shift and ⌥←/→ tracking: steps every selected character from its
/// own value (`key` is `leadingBy`, `baselineShiftBy` or `trackingBy`).
pub(crate) fn step_char_metric(app: &mut PhotocraftApp, key: &str, by: f32) {
    let Some(ed) = app.ui.text_edit.clone() else { return };
    let Some(range) = format_range(app, &ed) else { return };
    // Leading steps in the points the Character panel shows for a scaled layer.
    set_style(app, &ed, range, json!({ key: by, "metricsAsShown": key == "leadingBy" }));
}

/// ⌥⇧⌘H: flips Hyphenate for the paragraphs the selection (or the caret) is in: off when all of
/// them hyphenate, else on.
pub(crate) fn toggle_hyphenate(app: &mut PhotocraftApp) {
    let Some(ed) = app.ui.text_edit.clone() else { return };
    let Some(t) = app.session.active().and_then(|s| text_layer(&s.doc, LayerId(ed.layer))) else { return };
    let n = t.text.chars().count();
    let (a, b) = (ed.caret.min(ed.anchor).min(n), ed.caret.max(ed.anchor).min(n));
    let (ba, bb) = byte_range(&t.text, a, b);
    let len = t.text.len();
    let mut at = 0usize;
    let mut all = true;
    let mut any = false;
    for r in t.paragraph_runs() {
        let (ps, pe) = (at, at.saturating_add(r.len));
        at = pe;
        let touches = if ba == bb { ps <= ba && (ba < pe || pe >= len) } else { ps < bb && pe > ba };
        if touches {
            any = true;
            all &= r.style.hyphenate;
        }
    }
    let on = !(any && all);
    set_style(app, &ed, (a, b), json!({ "hyphenate": on }));
}

/// IME composition. The preedit text is written into the layer (so it lays out and reflows like
/// typed text) and replaced by every update; `commit` makes the result final.
fn ime_update(app: &mut PhotocraftApp, s: &str, commit: bool) {
    let Some(ed) = app.ui.text_edit.clone() else { return };
    // Validate the composition only while one exists: a plain preedit/commit otherwise needs no
    // copy of the whole layer.
    let n = if ed.preedit.is_some() {
        let Some(text) = current_text(app, LayerId(ed.layer)) else { return };
        // A composition range that no longer holds the recorded composition belongs to an edit
        // that was replaced under it: never use it to delete or replace text.
        if reconcile_preedit(app, &ed, &text) {
            return;
        }
        text.chars().count()
    } else {
        let Some(n) = text_char_count(app, LayerId(ed.layer)) else { return };
        n
    };
    let s = s.replace("\r\n", "\n").replace('\r', "\n");
    // An empty preedit or commit while nothing is being composed changes nothing. Wayland input
    // methods send one on every state change (the caret rectangle moving as a selection grows),
    // and it used to replace the selection, deleting the text as it was selected (#1381).
    if s.is_empty() && ed.preedit.is_none() {
        return;
    }
    let (start, end) = match ed.preedit {
        Some((p, l)) if p.saturating_add(l) <= n => (p, p + l),
        _ => (ed.caret.min(ed.anchor).min(n), ed.caret.max(ed.anchor).min(n)),
    };
    if start == end && s.is_empty() {
        if let Some(e) = app.ui.text_edit.as_mut() {
            e.preedit = None;
        }
        app.type_ime.clear_preedit();
        return;
    }
    // Capture the committed text *before* the engine runs. The pre-edit is excluded from this
    // and from the after snapshot; only an accepted commit earns an IME receipt.
    let before = if commit { ime_committed(app, &ed) } else { None };
    let len = s.chars().count();
    crate::type_transform::finish(app);
    if app.run("type.edit", json!({"layer": ed.layer, "replace": {"start": start, "end": end, "text": s}, "coalesce": ed.session})).is_err() {
        return;
    }
    let mut after_ed = ed.clone();
    after_ed.caret = start + len;
    after_ed.anchor = after_ed.caret;
    after_ed.preedit = if commit || len == 0 { None } else { Some((start, len)) };
    if let Some(e) = app.ui.text_edit.as_mut() {
        *e = after_ed.clone();
    }
    if after_ed.preedit.is_some() {
        app.type_ime.note_preedit(&after_ed, &s);
    } else {
        app.type_ime.clear_preedit();
    }
    if commit {
        app.type_ime.incoming = None;
    } else {
        remember_incoming(app, Some(s.is_empty()));
    }
    if commit
        && let Some(before) = before
        && let Some(after) = ime_committed(app, &after_ed)
    {
        // A no-op commit (equal snapshots) is rejected by the tracker rather than fabricated.
        note_ime_apply(app, &after_ed, &before, &after);
    }
}

/// The active range for an IME delete-surrounding batch: the pre-edit when composing, otherwise
/// the sorted selection. Returns `(start, end, length)` in character indices.
fn ime_active_range(app: &PhotocraftApp, ed: &TextEdit) -> Option<(usize, usize, usize)> {
    let n = text_char_count(app, LayerId(ed.layer))?;
    if let Some((start, len)) = ed.preedit {
        let end = start.checked_add(len)?;
        return (end <= n).then_some((start, end, n));
    }
    let a = ed.caret.min(n);
    let b = ed.anchor.min(n);
    Some((a.min(b), a.max(b), n))
}

/// Delete characters around the selection/pre-edit, as one coalesced type edit.
fn ime_delete_chars(app: &mut PhotocraftApp, before_chars: usize, after_chars: usize) -> bool {
    let Some(ed) = app.ui.text_edit.clone() else { return false };
    if ed.preedit.is_some() {
        let Some(text) = current_text(app, LayerId(ed.layer)) else { return false };
        if reconcile_preedit(app, &ed, &text) {
            return false;
        }
    }
    let Some((start, end, n)) = ime_active_range(app, &ed) else { return false };
    let from = start.saturating_sub(before_chars);
    let to = end.saturating_add(after_chars).min(n);
    ime_delete_around(app, ed, from, start, end, to)
}

/// `DeleteSurroundingBytes` counts UTF-8 bytes from the active range's edges. A byte count that
/// ends inside a character leaves that character, as the stock TextEdit does.
fn ime_delete_bytes(app: &mut PhotocraftApp, before_bytes: usize, after_bytes: usize) -> bool {
    let Some(ed) = app.ui.text_edit.clone() else { return false };
    let Some(text) = current_text(app, LayerId(ed.layer)) else { return false };
    if reconcile_preedit(app, &ed, &text) {
        return false;
    }
    let Some((start, end, n)) = ime_active_range(app, &ed) else { return false };
    let start_byte = byte_of(&text, start);
    let end_byte = byte_of(&text, end);
    let from_byte = start_byte.saturating_sub(before_bytes);
    let before_chars = text.get(..start_byte).map_or(0, |before| before.char_indices().rev().take_while(|(index, _)| *index >= from_byte).count());
    let after_chars =
        text.get(end_byte..).map_or(0, |after| after.char_indices().take_while(|(index, character)| index + character.len_utf8() <= after_bytes).count());
    let from = start.saturating_sub(before_chars);
    let to = end.saturating_add(after_chars).min(n);
    ime_delete_around(app, ed, from, start, end, to)
}

/// Apply one accepted IME deletion and record its receipt from the snapshots around it.
///
/// The active selection/pre-edit itself stays; only the requested runs before `start` and after
/// `end` are removed. They are two engine mutations with the same coalescing key (one history
/// step), and one receipt covers the whole batch.
fn ime_delete_around(app: &mut PhotocraftApp, ed: TextEdit, from: usize, start: usize, end: usize, to: usize) -> bool {
    if from >= start && end >= to {
        return false;
    }
    let before = ime_committed(app, &ed);
    crate::type_transform::finish(app);
    // Delete the trailing run first so the leading run's character indices stay valid.
    if end < to && app.run("type.edit", json!({"layer": ed.layer, "replace": {"start": end, "end": to, "text": ""}, "coalesce": ed.session})).is_err() {
        return false;
    }
    if from < start && app.run("type.edit", json!({"layer": ed.layer, "replace": {"start": from, "end": start, "text": ""}, "coalesce": ed.session})).is_err() {
        return false;
    }
    let shift = start - from;
    let mut after_ed = ed.clone();
    after_ed.caret = ed.caret.saturating_sub(shift);
    after_ed.anchor = ed.anchor.saturating_sub(shift);
    if let Some((preedit_start, preedit_len)) = ed.preedit {
        after_ed.preedit = Some((preedit_start.saturating_sub(shift), preedit_len));
    }
    if let Some(e) = app.ui.text_edit.as_mut() {
        *e = after_ed.clone();
    }
    if let (Some(before), Some(after)) = (before, ime_committed(app, &after_ed)) {
        note_ime_apply(app, &after_ed, &before, &after);
    }
    remember_incoming(app, None);
    true
}

/// Character index of the previous / next word boundary (shared with `type.navigate`).
fn word_boundary(text: &str, from: usize, forward: bool) -> usize {
    photocraft_text::word_boundary(text, from, forward)
}

/// Most clicks in a run that select more: 2 a word, 3 a line, 4 a paragraph, 5 all text. The
/// next quick click starts over with a caret.
const MAX_CLICKS: u32 = 5;

/// Count of this click in a run of clicks at one spot. `egui_count` is egui's own count for the
/// click (1 single, 2 double, 3 triple; it never goes past 3), which already applies the
/// double-click delay and distance; `prev` is our count for the previous click (0: none). A slow
/// or distant click (egui says 1) starts a new run; after five clicks the run starts over.
pub(crate) fn next_click_count(prev: u32, egui_count: u32) -> u32 {
    match (egui_count, prev) {
        (0 | 1, _) => 1,
        (n, 0) => n.min(MAX_CLICKS),
        (_, p) => p % MAX_CLICKS + 1,
    }
}

/// The selection (anchor, caret) a run of `count` clicks makes at character `caret`: 1 a caret,
/// 2 the word under it, 3 its visual line as laid out (a column in vertical type), 4 its
/// paragraph up to the hard return, 5 all text. `line` is the laid-out line the click hit; it
/// decides at a soft wrap, where one caret offset ends a line and starts the next. Without a
/// layout a line is its paragraph. Offsets past the text are clamped, so an empty layer or a
/// stale caret never panics.
pub(crate) fn click_selection(layout: Option<&TextLayout>, text: &str, caret: usize, line: Option<usize>, count: u32) -> (usize, usize) {
    let n = text.chars().count();
    let c = caret.min(n);
    match count {
        2 => {
            let chars: Vec<char> = text.chars().collect();
            let word = |i: usize| chars.get(i).is_some_and(|ch| ch.is_alphanumeric());
            let (mut a, mut b) = (c, c);
            while a > 0 && word(a - 1) {
                a -= 1;
            }
            while word(b) {
                b += 1;
            }
            (a, b)
        }
        3 => {
            let Some(l) = layout else { return paragraph_of(text, c) };
            let ln = line.and_then(|i| l.lines.get(i)).or_else(|| l.lines.get(photocraft_text::line_index(l, byte_of(text, c))));
            ln.map_or((c, c), |ln| (photocraft_text::char_index(text, ln.range.start), photocraft_text::char_index(text, ln.range.end)))
        }
        4 => paragraph_of(text, c),
        5 => (0, n),
        _ => (c, c),
    }
}

/// Characters of the paragraph holding character `c`, without its hard return.
fn paragraph_of(text: &str, c: usize) -> (usize, usize) {
    let b = byte_of(text, c);
    let paras = photocraft_text::layout::split_paragraphs(text);
    let Some(r) = paras.iter().find(|r| b >= r.start && b < r.end).or_else(|| paras.last()) else { return (c, c) };
    let body = text.get(r.clone()).unwrap_or("").trim_end_matches(['\r', '\n']);
    let start = photocraft_text::char_index(text, r.start);
    (start, start + body.chars().count())
}

/// A click while editing type at document point (`x`, `y`), after it placed the caret there.
/// `egui_count` is egui's count for the click (see [`next_click_count`]). Like Photoshop, two
/// clicks select a word, three the line, four the paragraph and five all text.
pub fn multi_click(app: &mut PhotocraftApp, egui_count: u32, x: f64, y: f64) {
    let Some(ed) = app.ui.text_edit.clone() else {
        app.type_clicks = None;
        return;
    };
    let prev = app.type_clicks.filter(|(layer, _)| *layer == ed.layer).map_or(0, |(_, n)| n);
    let count = next_click_count(prev, egui_count);
    app.type_clicks = Some((ed.layer, count));
    if count < 2 {
        return;
    }
    let id = LayerId(ed.layer);
    let Some(text) = current_text(app, id) else { return };
    let laid = layout(app, id);
    let (caret, line) = match &laid {
        Some((l, aff, t)) if *t == text && x.is_finite() && y.is_finite() => {
            let (tx, ty) = to_text(aff, x, y);
            let (ci, line) = photocraft_text::hit_char(l, t, tx, ty);
            (ci, Some(line))
        }
        _ => (ed.caret, None),
    };
    let (a, b) = click_selection(laid.as_ref().map(|(l, _, _)| l.as_ref()), &text, caret, line, count);
    if let Some(e) = app.ui.text_edit.as_mut() {
        (e.anchor, e.caret) = (a, b);
    }
}

/// Caret on the neighbouring line (±1; a column in vertical type), keeping the position
/// along the line. Works in line space, so it serves both orientations.
fn line_step(app: &mut PhotocraftApp, id: LayerId, caret: usize, dir: i32) -> usize {
    let Some((l, _, text)) = layout(app, id) else { return caret };
    let x = l.caret(byte_of(&text, caret)).0;
    photocraft_text::line_step(&l, &text, caret, x, dir)
}

fn is_vertical(app: &PhotocraftApp, id: LayerId) -> bool {
    app.session.active().and_then(|s| text_layer(&s.doc, id)).is_some_and(|t| t.orientation == photocraft_doc::text::Orientation::Vertical)
}

/// Arrow keys follow the text flow: in vertical type ↑/↓ move along the column (previous/next
/// character) and ←/→ move to the next/previous column (columns advance right to left). Returns
/// the horizontal-type key with the same meaning.
pub(crate) fn flow_key(key: egui::Key, vertical: bool) -> egui::Key {
    use egui::Key;
    if !vertical {
        return key;
    }
    match key {
        Key::ArrowUp => Key::ArrowLeft,
        Key::ArrowDown => Key::ArrowRight,
        Key::ArrowLeft => Key::ArrowDown,
        Key::ArrowRight => Key::ArrowUp,
        k => k,
    }
}

/// Line start / end for the caret's line.
fn line_edge(app: &mut PhotocraftApp, id: LayerId, caret: usize, end: bool) -> usize {
    let Some((l, _, text)) = layout(app, id) else { return caret };
    photocraft_text::line_edge(&l, &text, caret, end)
}

/// Keyboard input while editing. Returns true when a type edit session is active (single-key
/// tool shortcuts must then be skipped). Handled events are removed from the frame's input.
pub fn handle_keys(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    bind_reset_context(app, ctx);
    // A receipt is only valid for the frame that produced it; never reuse last frame's.
    app.type_ime.frame = None;
    end_if_left(app);
    sync_incoming(app, ctx.viewport_id());
    if !ctx.input(|i| i.focused) || ctx.text_edit_focused() || ctx.egui_wants_keyboard_input() {
        interrupt_incoming(app);
    }
    crate::type_transform::cancel_stale(app);
    if !ctx.input(|i| i.focused) {
        crate::type_transform::cancel_drag(app);
    }
    let Some(ed) = app.ui.text_edit.clone() else {
        controls_only(app, ctx);
        return false;
    };
    // The font picker (and other focused fields) owns keyboard input while it is open.
    if !reset_route_eligible(app, ctx) {
        controls_only(app, ctx);
        return true;
    }
    let id = LayerId(ed.layer);
    let Some(n) = text_char_count(app, id) else {
        app.ui.text_edit = None;
        app.ui.text_edit_return_tool = None;
        app.type_ime.clear_preedit();
        controls_only(app, ctx);
        return false;
    };
    // Undo/redo can shorten the text under us.
    if let Some(e) = app.ui.text_edit.as_mut() {
        e.caret = e.caret.min(n);
        e.anchor = e.anchor.min(n);
    }
    if !claim_raw_pass(app, ctx) {
        return true;
    }
    app.type_ime.begin_frame(ctx);
    prepare_reset(app, ctx);
    // A target swap (session replacement, Type Mask, another layer) is known from the last
    // descriptor the platform received. Cancel the native composition before any event in this
    // pass can be applied to the new target.
    if ime_target(app, &ed, ctx.viewport_id()).is_some_and(|target| app.type_ime.emitted.is_some_and(|last| last.target != target)) {
        app.type_ime.interrupt = true;
    }
    let time = ctx.input(|i| i.time);
    let elapsed = (time - app.ui.caret_blink_time).max(0.0);
    let period = 0.530;
    let remaining = period - (elapsed % period);
    ctx.request_repaint_after(std::time::Duration::from_secs_f64(remaining.clamp(0.010, period)));
    let events = ctx.input(|i| i.events.clone());
    let vertical_flow = is_vertical(app, id);
    let mut handled = vec![false; events.len()];
    let mut primary_down = events
        .iter()
        .find_map(|event| match event {
            egui::Event::PointerButton { button: egui::PointerButton::Primary, pressed, .. } => Some(!pressed),
            _ => None,
        })
        .unwrap_or_else(|| ctx.input(|input| input.pointer.primary_down()));
    let mut custom_mutations = true;
    let mut deferred_custom = false;
    for (k, ev) in events.iter().enumerate() {
        if custom_mutations && !reset_route_eligible(app, ctx) {
            custom_mutations = false;
            suspend_reset(app, ctx);
        }
        if !custom_mutations {
            if let egui::Event::ImeResetCompleted { request_id } = ev {
                complete_reset(app, ctx, *request_id);
                if let Some(slot) = handled.get_mut(k) {
                    *slot = true;
                }
            } else if deferred_custom
                && matches!(ev, egui::Event::Text(_) | egui::Event::Paste(_) | egui::Event::Cut | egui::Event::Ime(_))
                && let Some(slot) = handled.get_mut(k)
            {
                *slot = true;
            }
            continue;
        }
        prepare_reset(app, ctx);
        let Some(ed) = app.ui.text_edit.clone() else { continue };
        let (a, b) = (ed.caret.min(ed.anchor), ed.caret.max(ed.anchor));
        // Only key handling needs the text (word/line edges); pointer, clipboard and IME events
        // do not clone the whole layer.
        let is_key = matches!(ev, egui::Event::Key { .. });
        let text = if is_key { current_text(app, id).unwrap_or_default() } else { String::new() };
        let n = if is_key { text.chars().count() } else { 0 };
        let set = |app: &mut PhotocraftApp, caret: usize, extend: bool| {
            if let Some(e) = app.ui.text_edit.as_mut() {
                e.caret = caret.min(n);
                if !extend {
                    e.anchor = e.caret;
                }
            }
        };
        handled[k] = true;
        match ev {
            egui::Event::Text(s) => insert_typed(app, s),
            egui::Event::Paste(s) => insert(app, s),
            egui::Event::ImeResetCompleted { request_id } => complete_reset(app, ctx, *request_id),
            egui::Event::Ime(ime_event) => {
                // Untagged terminal payload cannot acknowledge a host reset. Only the exact
                // current completion arm above may open admission for the following element.
                if reset_guarded(app, ctx.viewport_id()) {
                    continue;
                }
                match ime_event {
                    // A bare line break from the IME is the Enter key, which the key arm handles.
                    egui::ImeEvent::Preedit { text: s, .. } | egui::ImeEvent::Commit(s) if s == "\n" || s == "\r" => handled[k] = false,
                    egui::ImeEvent::Preedit { text, .. } => ime_update(app, text, false),
                    egui::ImeEvent::Commit(s) => {
                        ime_update(app, s, true);
                        app.type_ime.incoming = None;
                    }
                    egui::ImeEvent::DeleteSurrounding { before_chars, after_chars } => {
                        ime_delete_chars(app, *before_chars, *after_chars);
                    }
                    egui::ImeEvent::DeleteSurroundingBytes { before_bytes, after_bytes } => {
                        ime_delete_bytes(app, *before_bytes, *after_bytes);
                    }
                    _ => {}
                }
            }
            egui::Event::PointerButton { button: egui::PointerButton::Primary, pressed, .. } => {
                primary_down = *pressed;
                if *pressed {
                    // The canvas destination handler runs after this pass; never apply a
                    // same-batch Text/Paste/Cut/IME suffix at the old caret just because no
                    // composition or reset guard existed yet.
                    interrupt_action(app, ctx.viewport_id());
                    custom_mutations = false;
                    deferred_custom = true;
                    suspend_reset(app, ctx);
                }
                handled[k] = false;
            }
            egui::Event::PointerMoved(_) if primary_down => {
                interrupt_action(app, ctx.viewport_id());
                custom_mutations = false;
                deferred_custom = true;
                suspend_reset(app, ctx);
                handled[k] = false;
            }
            egui::Event::Copy => {
                copy_selection(app, ctx);
            }
            egui::Event::Cut => cut_selection(app, ctx),
            egui::Event::Key { key, pressed: true, modifiers: m, .. } => {
                use egui::Key;
                let key = &flow_key(*key, vertical_flow);
                match key {
                    // Photoshop's type formatting keys act on the selected characters and win
                    // over the global ⇧⌘K/U/B/I (Color Settings, Desaturate, Auto Color, Inverse)
                    // while typing. Each press is one history step.
                    Key::K | Key::U | Key::B | Key::I | Key::Slash | Key::Questionmark if m.command && m.shift && !m.alt => {
                        let which = match key {
                            Key::K => CharToggle::AllCaps,
                            Key::U => CharToggle::Underline,
                            Key::B => CharToggle::FauxBold,
                            Key::I => CharToggle::FauxItalic,
                            _ => CharToggle::Strikethrough,
                        };
                        toggle_char_style(app, which);
                    }
                    Key::H if m.command && m.shift && m.alt => toggle_hyphenate(app),
                    // ⌥↑/↓ leading -/+2 pt, ⇧⌥↑/↓ baseline shift +/-2 pt (10 with ⌘/Ctrl).
                    Key::ArrowUp | Key::ArrowDown if m.alt => {
                        let step = if m.command { 10.0 } else { 2.0 };
                        let up = *key == Key::ArrowUp;
                        if m.shift {
                            step_char_metric(app, "baselineShiftBy", if up { step } else { -step });
                        } else {
                            step_char_metric(app, "leadingBy", if up { -step } else { step });
                        }
                    }
                    // ⌥←/→ over a selection tracks it by -/+20 (100 with ⌘/Ctrl).
                    Key::ArrowLeft | Key::ArrowRight if m.alt && !m.shift && a != b => {
                        let step = if m.command { 100.0 } else { 20.0 };
                        step_char_metric(app, "trackingBy", if *key == Key::ArrowRight { step } else { -step });
                    }
                    Key::Backspace | Key::Delete => {
                        if a == b {
                            let fwd = *key == Key::Delete;
                            let to = match (fwd, m.alt, m.command) {
                                (false, _, true) => line_edge(app, id, a, false),
                                (false, true, _) => word_boundary(&text, a, false),
                                (false, _, _) => a.saturating_sub(1),
                                (true, true, _) => word_boundary(&text, a, true),
                                (true, _, _) => (a + 1).min(n),
                            };
                            if to != a {
                                if let Some(e) = app.ui.text_edit.as_mut() {
                                    e.anchor = to;
                                }
                                insert(app, "");
                            }
                        } else {
                            insert(app, "");
                        }
                    }
                    // Photoshop: Alt/Option+←/→ at a collapsed caret kerns the pair before it by
                    // 20/1000 em (100 with ⌘/Ctrl), one history step per press.
                    Key::ArrowLeft | Key::ArrowRight if m.alt && !m.shift && a == b => {
                        let step = if m.command { 100.0 } else { 20.0 };
                        kern_pair(app, id, ed.caret, if *key == Key::ArrowRight { step } else { -step });
                    }
                    Key::ArrowLeft | Key::ArrowRight => {
                        let fwd = *key == Key::ArrowRight;
                        // Word movement: ⌘/Ctrl (Photoshop's; Alt is kerning), and Alt+Shift
                        // extends the selection by words; Home/End go to the line edges.
                        let to = if m.command || m.alt {
                            word_boundary(&text, ed.caret, fwd)
                        } else if a != b && !m.shift {
                            if fwd { b } else { a }
                        } else if fwd {
                            ed.caret + 1
                        } else {
                            ed.caret.saturating_sub(1)
                        };
                        set(app, to, m.shift);
                    }
                    Key::ArrowUp | Key::ArrowDown => {
                        let to = if m.command {
                            if *key == Key::ArrowUp { 0 } else { n }
                        } else {
                            line_step(app, id, ed.caret, if *key == Key::ArrowUp { -1 } else { 1 })
                        };
                        set(app, to, m.shift);
                    }
                    Key::Home | Key::End => {
                        let to = line_edge(app, id, ed.caret, *key == Key::End);
                        set(app, to, m.shift);
                    }
                    Key::Enter if m.command => commit_with(app, *m),
                    Key::Enter => insert(app, "\n"),
                    // Photoshop types a tab character; layout advances it to the next tab stop.
                    Key::Tab if !m.command && !m.alt && !m.shift => {
                        // egui queues focus navigation before this handler runs; consuming the
                        // key alone would let the menu bar take focus after the first tab (#8889).
                        ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
                        insert(app, "\t");
                    }
                    Key::Escape if crate::type_transform::active(app) => crate::type_transform::cancel_drag(app),
                    // The Type Mask tools: Escape always cancels, leaving the selection as it was.
                    Key::Escape if ed.mask => cancel(app),
                    // Preferences ▸ Type: with "Use Escape to Commit" off, Escape cancels the
                    // session (undoes it, removing a just-created layer) like the Cancel button.
                    Key::Escape if app.session.prefs().type_.use_esc_to_commit => commit(app),
                    Key::Escape => cancel(app),
                    Key::A if m.command => {
                        if let Some(e) = app.ui.text_edit.as_mut() {
                            e.anchor = 0;
                            e.caret = n;
                        }
                    }
                    // Photoshop: while editing type, ⌘/Ctrl+T shows or hides the Character panel
                    // (Free Transform of the layer being typed into is not what it means here).
                    Key::T if m.command && !m.shift && !m.alt => {
                        let _ = crate::menus::invoke(app, ctx, "window.panel.character", json!({}));
                    }
                    // Other command shortcuts (⌘Z, ⌘S, …) pass through to the menus.
                    _ if m.command => {
                        if *key == Key::Z {
                            crate::type_transform::cancel_drag(app);
                        }
                        if reset_guarded(app, ctx.viewport_id()) || app.type_ime.incoming.is_some() {
                            interrupt_action(app, ctx.viewport_id());
                            custom_mutations = false;
                            deferred_custom = true;
                            suspend_reset(app, ctx);
                        }
                        handled[k] = false;
                    }
                    _ => {}
                }
            }
            egui::Event::Key { pressed: false, modifiers: m, .. } => handled[k] = !m.command,
            _ => handled[k] = false,
        }
    }
    prepare_reset(app, ctx);
    if handled.iter().any(|h| *h) {
        app.ui.caret_blink_time = ctx.input(|i| i.time);
        let mut k = 0;
        ctx.input_mut(|i| {
            i.events.retain(|_| {
                let keep = !handled.get(k).copied().unwrap_or(false);
                k += 1;
                keep
            })
        });
    }
    app.ui.text_edit.is_some()
}

/// End the editing session. A new layer left empty is deleted; a new layer is named after its text.
pub fn commit(app: &mut PhotocraftApp) {
    commit_with(app, egui::Modifiers::NONE);
}

/// [`commit`] with the modifiers held while committing: a Type Mask adds its glyphs to the
/// selection with ⇧, subtracts them with ⌥ and intersects with ⇧⌥ (Photoshop); with none it
/// replaces the selection.
pub fn commit_with(app: &mut PhotocraftApp, mods: egui::Modifiers) {
    let current = app.session.active_index();
    let Some(home) = app.ui.text_edit.as_ref().map(|ed| edit_home(app, ed)) else {
        return commit_here(app, mods);
    };
    match home {
        Some(home) if Some(home) != current => {
            // An edit left in a background document commits there, in its own history (#6483).
            app.session.set_active(home);
            commit_here(app, mods);
            if let Some(i) = current {
                app.session.set_active(i);
            }
        }
        Some(_) => commit_here(app, mods),
        // Its document closed: there is nothing to commit into, and the layer id may name a
        // layer of another document.
        None => {
            crate::type_transform::reset(app);
            app.ui.text_edit = None;
            app.ui.text_edit_return_tool = None;
            app.type_ime.clear_preedit();
        }
    }
}

/// Index of the document an edit belongs to; `None` once it is closed.
fn edit_home(app: &PhotocraftApp, ed: &TextEdit) -> Option<usize> {
    match ed.document {
        Some(d) => app.session.documents().iter().position(|st| st.doc.id.0 == d),
        None => app.session.active_index(),
    }
}

/// [`commit_with`] in the active document, which is the edit's own.
fn commit_here(app: &mut PhotocraftApp, mods: egui::Modifiers) {
    if app.ui.tool.is_type() {
        crate::type_transform::finish(app);
    }
    crate::type_transform::reset(app);
    let Some(ed) = app.ui.text_edit.take() else { return };
    app.type_ime.clear_preedit();
    if let Some(tool) = app.ui.text_edit_return_tool.take()
        && app.ui.tool.is_type()
    {
        app.ui.tool = tool;
    }
    let Some(text) = current_text(app, LayerId(ed.layer)) else { return };
    if ed.mask {
        // Type Mask: the text's glyphs become (or combine with) the selection and the working
        // layer goes, all in the session's one history step. No text: nothing changes.
        let operation = crate::tool_feedback::selection_mode(crate::state::Tool::HorizontalTypeMask, 0, mods);
        if !text.trim().is_empty()
            && let Err(e) = app.run("type.createSelection", json!({"layer": ed.layer, "removeLayer": true, "operation": operation, "coalesce": ed.session}))
        {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        if current_text(app, LayerId(ed.layer)).is_some() {
            undo_session(app, &ed);
        }
        return;
    }
    if text.trim().is_empty() && ed.created {
        let _ = app.run("layer.delete", json!({"layer": ed.layer}));
    } else if ed.created {
        let name = photocraft_engine::type_cmds::layer_name(&text);
        let _ = app.run("type.edit", json!({"layer": ed.layer, "name": name, "coalesce": ed.session}));
    }
}

/// Commit the edit once its layer is no longer the one worked on (#6483): another layer became
/// active (a Layers panel menu, the canvas layer picker, automation) or another document did (a
/// tab, File ▸ New/Open). Photoshop commits then; left open, the session took single-key
/// shortcuts as typing and typed into whichever document was active.
pub fn end_if_left(app: &mut PhotocraftApp) {
    let Some(ed) = app.ui.text_edit.as_ref() else { return };
    let layer = LayerId(ed.layer);
    if edit_home(app, ed).is_none_or(|h| Some(h) != app.session.active_index()) {
        return commit(app);
    }
    let Some(st) = app.session.active() else { return };
    if st.active_layer == Some(layer) {
        return;
    }
    // The selection just made stands: ending an empty Type Mask undoes its session, and undo
    // restores the selection from before it.
    let chosen = (st.active_layer, st.selected_layers.clone(), st.layer_anchor);
    commit(app);
    let moved = app.session.active().is_some_and(|st| (st.active_layer, &st.selected_layers, st.layer_anchor) != (chosen.0, &chosen.1, chosen.2));
    if moved {
        let _ = app.session.restore_layer_selection(chosen.0, &chosen.1, chosen.2);
    }
}

/// Selection highlight, caret and text frame over the canvas.
pub fn draw_overlay(app: &mut PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    let Some(ed) = app.ui.text_edit.clone() else { return };
    // Until it commits, an edit in a document no longer active draws nothing over this one.
    if edit_home(app, &ed) != app.session.active_index() {
        return;
    }
    if ed.dragging {
        app.ui.caret_blink_time = painter.ctx().input(|i| i.time);
    }
    let id = LayerId(ed.layer);
    let Some((l, aff, text)) = layout(app, id) else { return };
    if ed.mask {
        draw_mask_overlay(app, painter, xf, id);
    }
    let t = crate::theme::Tokens::get(painter.ctx());
    // Text space → screen.
    let scr_t = |x: f32, y: f32| -> Pos2 {
        let p = aff.apply(Point::new(x as f64, y as f64));
        xf.to_screen(p.x as f32, p.y as f32)
    };
    let shape = app.session.active().and_then(|s| text_layer(&s.doc, id).map(|t| t.shape.clone()));
    // Type on a path: the caret, selection and underline follow the curve the glyphs sit on.
    let bend = shape.as_ref().and_then(photocraft_text::warp::Warp::on_path);
    // Line space (lines, clusters, carets) → screen: vertical type turns it 90° clockwise.
    let scr = |x: f32, y: f32| -> Pos2 {
        let (x, y) = l.to_text(x, y);
        match &bend {
            Some(w) => {
                let (x, y) = w.apply(f64::from(x), f64::from(y));
                scr_t(x as f32, y as f32)
            }
            None => scr_t(x, y),
        }
    };
    // A line-space run from `x0` to `x1` at height `y`: sampled, so it bends with path type.
    let run = |x0: f32, x1: f32, y: f32| -> Vec<Pos2> {
        let n = if bend.is_some() { ((x1 - x0) / 4.0).ceil().clamp(1.0, 512.0) as usize } else { 1 };
        (0..=n).map(|i| scr(x0 + (x1 - x0) * i as f32 / n as f32, y)).collect()
    };
    // The IME descriptor is emitted once at pass end by `sync_ime`, from this pass's canvas
    // mapping and the model as it stands after every panel has run.
    let pass_nr = painter.ctx().cumulative_pass_nr_for(painter.ctx().viewport_id());
    app.type_ime.note_view(painter.ctx().viewport_id(), pass_nr, xf);
    // Frame: paragraph text shows its box with handles; point text an underline per line.
    let mods = crate::workspace_ui::sticky_mods(app, painter.ctx().input(|i| i.modifiers));
    let has_transform = crate::type_transform::draw(app, painter, xf, mods);
    let frame = Stroke::new(1.0, t.accent);
    match shape {
        Some(photocraft_doc::text::TextShape::Box { x, y, width, height }) => {
            if !has_transform {
                let c = [scr_t(x, y), scr_t(x + width, y), scr_t(x + width, y + height), scr_t(x, y + height)];
                painter.add(egui::Shape::closed_line(c.to_vec(), frame));
                let mids = [c[0].lerp(c[1], 0.5), c[1].lerp(c[2], 0.5), c[2].lerp(c[3], 0.5), c[3].lerp(c[0], 0.5)];
                for p in c.iter().chain(mids.iter()) {
                    let r = egui::Rect::from_center_size(*p, egui::vec2(7.0, 7.0));
                    painter.rect_filled(r, 0.0, Color32::WHITE);
                    painter.rect_stroke(r, 0.0, frame, egui::StrokeKind::Inside);
                }
            }
        }
        // Area type: the outline of the shape it wraps inside.
        Some(photocraft_doc::text::TextShape::Area { path }) => {
            if !has_transform {
                for poly in photocraft_vector::flatten::flatten_path(&path, 0.5) {
                    let pts: Vec<Pos2> = poly.pts.iter().map(|&(x, y)| scr_t(x as f32, y as f32)).collect();
                    if pts.len() >= 2 {
                        painter.add(egui::Shape::closed_line(pts, frame));
                    }
                }
            }
        }
        _ => {
            for ln in &l.lines {
                // Horizontal: just under the baseline; vertical: the column's centre line.
                let y = if l.vertical { ln.baseline } else { ln.baseline + ln.descent * 0.25 };
                painter.add(egui::Shape::line(run(ln.x0.min(0.0), ln.x1.max(ln.x0 + 1.0), y), Stroke::new(1.0, t.accent.gamma_multiply(0.8))));
            }
        }
    }
    // Uncommitted IME text is underlined, but only while the stored range still holds exactly
    // the composition: an application edit may have shifted it onto committed characters.
    if let Some(offsets) = ime_offsets(&text, &ed)
        && ime_preedit_is_current(app, &text, &ed, &offsets)
        && let Some((a, b)) = offsets.preedit.map(|range| (range.start, range.end))
    {
        for (li, ln) in l.lines.iter().enumerate() {
            let xs: Vec<(f32, f32)> =
                l.clusters.iter().filter(|c| c.line == li && c.range.start >= a && c.range.end <= b).map(|c| (c.x, c.x + c.advance)).collect();
            let (x0, x1) = xs.iter().fold((f32::MAX, f32::MIN), |(lo, hi), (p, q)| (lo.min(*p), hi.max(*q)));
            if x0 < x1 {
                let y = ln.baseline + ln.descent * 0.5;
                painter.add(egui::Shape::line(run(x0, x1, y), Stroke::new(1.5, t.accent)));
            }
        }
    }
    // Selection: per line, the clusters inside [a, b).
    let (a, b) = (byte_of(&text, ed.caret.min(ed.anchor)), byte_of(&text, ed.caret.max(ed.anchor)));
    if a < b {
        let fill = Color32::from_rgba_unmultiplied(t.accent.r(), t.accent.g(), t.accent.b(), 110);
        for (li, ln) in l.lines.iter().enumerate() {
            let xs: Vec<(f32, f32)> =
                l.clusters.iter().filter(|c| c.line == li && c.range.start >= a && c.range.end <= b).map(|c| (c.x, c.x + c.advance)).collect();
            let (mut x0, mut x1) = xs.iter().fold((f32::MAX, f32::MIN), |(lo, hi), (p, q)| (lo.min(*p), hi.max(*q)));
            // A selected line break shows as a small sliver past the line end.
            if b > ln.range.end && a <= ln.range.end {
                x1 = x1.max(ln.x1 + (ln.ascent + ln.descent) * 0.25);
                x0 = x0.min(ln.x1);
            }
            let (top, bot) = (ln.baseline - ln.ascent, ln.baseline + ln.descent);
            if bend.is_some() {
                // On a path each glyph turns on its own, so each gets its own highlight.
                // Glyphs past the end of the path are not drawn, so neither is their highlight.
                let shown = |p: f32, q: f32| bend.as_ref().is_some_and(|w| w.place_glyph(f64::from(p + q) / 2.0) != photocraft_text::warp::Placement::Hidden);
                for (p, q) in xs.into_iter().filter(|&(p, q)| shown(p, q)) {
                    painter.add(egui::Shape::convex_polygon(vec![scr(p, top), scr(q, top), scr(q, bot), scr(p, bot)], fill, Stroke::NONE));
                }
            } else if x0 < x1 {
                painter.add(egui::Shape::convex_polygon(vec![scr(x0, top), scr(x1, top), scr(x1, bot), scr(x0, bot)], fill, Stroke::NONE));
            }
        }
    } else {
        // Blinking caret (Photoshop's ~0.53 s rhythm), solid while dragging or immediately after interaction.
        let time = painter.ctx().input(|i| i.time);
        let elapsed = (time - app.ui.caret_blink_time).max(0.0);
        if ed.dragging || (elapsed * 1000.0 / 530.0) as i64 % 2 == 0 {
            let (x, top, bot) = l.caret(a);
            let (x, top, bot) = if l.lines.is_empty() { (0.0, -(12.0 * l.px_per_pt.max(1.0)), 3.0) } else { (x, top, bot) };
            for stroke in CARET_STROKES {
                painter.line_segment([scr(x, top), scr(x, bot)], stroke);
            }
        }
    }
}

/// The text caret, drawn in order: a white halo, then a solid black line inside it, so it reads on
/// any background (#6085). The black line used to be a 0.75 pt hairline inside a white one, which
/// on a white background left only a faint grey line.
const CARET_STROKES: [Stroke; 2] = [Stroke { width: 3.0, color: Color32::WHITE }, Stroke { width: 1.0, color: Color32::BLACK }];

/// The Type Mask tools while typing: Quick Mask's colour over the canvas with the glyphs clear,
/// as Photoshop shows the selection-to-be. The glyphs come from the hidden working layer's pixels.
fn draw_mask_overlay(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform, id: LayerId) {
    let Some(st) = app.session.active() else { return };
    let canvas = st.doc.bounds();
    let opts = app.session.quick_mask_options;
    let rgb = opts.color.to_rgb();
    let tint = |a: f32| {
        let a = (a * opts.opacity).clamp(0.0, 1.0);
        let q = |v: f32| (v.clamp(0.0, 1.0) * a * 255.0 + 0.5) as u8;
        Color32::from_rgba_premultiplied(q(rgb[0]), q(rgb[1]), q(rgb[2]), (a * 255.0 + 0.5) as u8)
    };
    let corners = |r: photocraft_geom::Rect| -> Vec<Pos2> {
        let (x0, y0, x1, y1) = (r.x0 as f32, r.y0 as f32, r.x1 as f32, r.y1 as f32);
        vec![xf.to_screen(x0, y0), xf.to_screen(x1, y0), xf.to_screen(x1, y1), xf.to_screen(x0, y1)]
    };
    let surface = text_layer(&st.doc, id).and_then(|t| t.cache.as_ref());
    let area = surface.map(|s| s.content_bounds().intersect(&canvas)).unwrap_or(photocraft_geom::Rect::EMPTY);
    // One unfeathered mesh, so the bands and the text's box meet without seams.
    let mut bands = egui::Mesh::default();
    let mut band = |x0: i32, y0: i32, x1: i32, y1: i32| {
        let r = photocraft_geom::Rect { x0, y0, x1, y1 };
        if !r.is_empty() {
            let base = bands.vertices.len() as u32;
            for p in corners(r) {
                bands.colored_vertex(p, tint(1.0));
            }
            bands.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    };
    let Some(surface) = surface.filter(|_| !area.is_empty()) else {
        band(canvas.x0, canvas.y0, canvas.x1, canvas.y1);
        painter.add(egui::Shape::mesh(bands));
        return;
    };
    band(canvas.x0, canvas.y0, canvas.x1, area.y0);
    band(canvas.x0, area.y1, canvas.x1, canvas.y1);
    band(canvas.x0, area.y0, area.x0, area.y1);
    band(area.x1, area.y0, canvas.x1, area.y1);
    painter.add(egui::Shape::mesh(bands));
    // The text's box: a texture of the tint where there is no glyph, cached per revision.
    let ctx = painter.ctx();
    let key = (st.doc.id.0, st.revision, id.0);
    let cache = egui::Id::unique("pc-type-mask-overlay");
    let tex = ctx.data(|d| d.get_temp::<((u64, u64, u64), egui::TextureHandle)>(cache)).filter(|(k, _)| *k == key).map(|(_, t)| t);
    let tex = tex.unwrap_or_else(|| {
        let max = (ctx.input(|i| i.max_texture_side) as u32).clamp(256, 4096);
        let factor = area.width().max(area.height()).div_ceil(max).max(1);
        let (w, h) = (area.width().div_ceil(factor) as usize, area.height().div_ceil(factor) as usize);
        let mut row = vec![[0.0f32; 4]; area.width() as usize];
        let mut px = Vec::with_capacity(w * h);
        for y in (area.y0..area.y1).step_by(factor as usize) {
            surface.read_rgba_into(photocraft_geom::Rect { x0: area.x0, y0: y, x1: area.x1, y1: y + 1 }, &mut row);
            px.extend(row.iter().step_by(factor as usize).take(w).map(|p| tint(1.0 - p[3])));
        }
        px.resize(w * h, tint(1.0));
        let img = egui::ColorImage::new([w, h], px);
        let t = ctx.load_texture("type-mask-overlay", img, egui::TextureOptions::LINEAR);
        ctx.data_mut(|d| d.insert_temp(cache, (key, t.clone())));
        t
    });
    let c = corners(area);
    let mut mesh = egui::Mesh::with_texture(tex.id());
    for (p, uv) in c.iter().zip([egui::pos2(0.0, 0.0), egui::pos2(1.0, 0.0), egui::pos2(1.0, 1.0), egui::pos2(0.0, 1.0)]) {
        mesh.vertices.push(egui::epaint::Vertex { pos: *p, uv, color: Color32::WHITE });
    }
    mesh.indices.extend([0, 1, 2, 0, 2, 3]);
    painter.add(egui::Shape::mesh(mesh));
}

/// Font families: bundled, system, and those the host serves (`photocraft_text::served`, fetched
/// when picked). Cached until the shared engine registers fonts or the served catalog changes.
pub fn families() -> Arc<Vec<String>> {
    type Cached = Option<((u64, u64), Arc<Vec<String>>)>;
    static FAMILIES: std::sync::Mutex<Cached> = std::sync::Mutex::new(None);
    let key = (photocraft_text::fonts::shared_generation(), photocraft_text::served::generation());
    let mut cached = FAMILIES.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((k, v)) = cached.as_ref()
        && *k == key
    {
        return v.clone();
    }
    let mut v = photocraft_text::shared().lock().map(|mut e| e.fonts.families()).unwrap_or_default();
    for f in photocraft_text::served::families() {
        if !v.contains(&f) {
            v.push(f);
        }
    }
    v.sort_by_key(|s| s.to_lowercase());
    let v = Arc::new(v);
    *cached = Some((key, v.clone()));
    v
}

/// Where [`sync_font_names`] leaves the font menus' localized labels for this frame.
fn font_labels_id() -> egui::Id {
    egui::Id::unique("photocraft-font-menu-localized-names")
}

/// Preferences ▸ Type ▸ Show Font Names in English: when off, font menus list a family that has a
/// localized (native-language) name by that name, as Photoshop does ("MS Gothic" reads
/// "ＭＳ ゴシック"). Called once a frame; the menus read the labels from the context.
pub fn sync_font_names(ctx: &egui::Context, show_in_english: bool) {
    let labels = (!show_in_english).then(localized_families);
    ctx.data_mut(|d| {
        if let Some(labels) = labels {
            d.insert_temp(font_labels_id(), labels);
        } else {
            d.remove::<Arc<HashMap<String, String>>>(font_labels_id());
        }
    });
}

/// The fonts' localized family names, keyed by family. Cached until the shared engine registers fonts.
fn localized_families() -> Arc<HashMap<String, String>> {
    type Cached = Option<(u64, Arc<HashMap<String, String>>)>;
    static LOCALIZED: std::sync::Mutex<Cached> = std::sync::Mutex::new(None);
    let key = photocraft_text::fonts::shared_generation();
    let mut cached = LOCALIZED.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((k, v)) = cached.as_ref()
        && *k == key
    {
        return v.clone();
    }
    let v = Arc::new(photocraft_text::shared().lock().map(|mut e| e.fonts.localized_family_names()).unwrap_or_default());
    *cached = Some((key, v.clone()));
    v
}

/// The font menu's rows: each family with the label it shows (its localized name when `labels`
/// has one, else the family), matching `query` (lowercase) by either name. With localized labels
/// the rows sort by label, so the menu stays alphabetical as shown. As in Photoshop, recently
/// used families also head the menu in their own group, most recent first (Preferences ▸ Type ▸
/// Recent Fonts), and keep their alphabetical place below it; a query shows each match once.
/// Returns the rows and how many of them form the recent group.
fn font_menu_rows<'a>(
    families: &'a [String],
    labels: Option<&'a HashMap<String, String>>,
    query: &str,
    recents: &[String],
) -> (Vec<(&'a String, &'a str)>, usize) {
    let label = |f: &'a String| labels.and_then(|l| l.get(f)).map_or(f.as_str(), String::as_str);
    let mut rows: Vec<(&String, &str)> = families
        .iter()
        .map(|f| (f, label(f)))
        .filter(|(f, l)| query.is_empty() || f.to_lowercase().contains(query) || l.to_lowercase().contains(query))
        .collect();
    if labels.is_some_and(|l| !l.is_empty()) {
        rows.sort_by_cached_key(|(_, l)| l.to_lowercase());
    }
    if !query.is_empty() {
        return (rows, 0);
    }
    let recent: Vec<(&String, &str)> = recents.iter().filter_map(|r| rows.iter().find(|(f, _)| *f == r).copied()).collect();
    let n = recent.len();
    (recent.into_iter().chain(rows).collect(), n)
}

fn weight_name(w: f32) -> &'static str {
    match w.round() as i32 {
        ..=150 => "Thin",
        151..=250 => "ExtraLight",
        251..=350 => "Light",
        351..=450 => "Regular",
        451..=550 => "Medium",
        551..=650 => "SemiBold",
        651..=750 => "Bold",
        751..=850 => "ExtraBold",
        _ => "Black",
    }
}

/// Style names available for a family: each face's actual OpenType subfamily (including numeric
/// and nonstandard ones), plus the standard weights a variable face's `wght` axis covers.
pub fn styles(family: &str) -> Vec<String> {
    let faces = photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner()).fonts.faces(family);
    let v = style_names(&faces);
    if v.is_empty() { vec!["Regular".into()] } else { v }
}

/// [`styles`] of these faces, lightest first. A variable face reports only its default instance
/// (Montserrat's is Thin), yet the layout draws any weight its `wght` axis covers, so the standard
/// weights in that range are listed too.
fn style_names(faces: &[photocraft_text::FaceInfo]) -> Vec<String> {
    let label = |weight: f32, italic: bool| {
        let name = match (weight_name(weight), italic) {
            ("Regular", true) => "Italic".to_string(),
            (w, true) => format!("{w} Italic"),
            (w, false) => w.to_string(),
        };
        (weight.round() as i32, italic, name)
    };
    let mut v: Vec<(i32, bool, String)> = faces.iter().map(|f| (f.weight.round() as i32, f.italic, f.style.clone())).collect();
    for f in faces {
        if let Some((_, min, _, max)) = f.axes.iter().find(|a| a.0 == "wght") {
            v.extend((100..=900).step_by(100).map(|w| w as f32).filter(|w| (*min..=*max).contains(w)).map(|w| label(w, f.italic)));
        }
    }
    v.sort();
    let mut names: Vec<String> = Vec::new();
    for (_, _, name) in v {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// A variable-font axis: (tag, min, default, max), as [`photocraft_text::FaceInfo::axes`].
pub(crate) type Axis = (String, f32, f32, f32);

/// The axes the Character section shows as sliders (#2985): those of the faces matching the
/// style's italic, else of any face. Optical size has none: it follows the type size.
pub(crate) fn slider_axes(faces: &[photocraft_text::FaceInfo], italic: bool) -> Vec<Axis> {
    let pick = |want: Option<bool>| {
        let mut v: Vec<Axis> = Vec::new();
        for f in faces.iter().filter(|f| want.is_none_or(|w| f.italic == w)) {
            for a in &f.axes {
                let usable = a.0 != "opsz" && a.1.is_finite() && a.3.is_finite() && a.1 < a.3;
                if usable && !v.iter().any(|b| b.0 == a.0) {
                    v.push(a.clone());
                }
            }
        }
        v
    };
    let v = pick(Some(italic));
    if v.is_empty() { pick(None) } else { v }
}

/// The value an axis slider shows: the style's own value for the axis, else what the layout
/// draws (the style's weight on `wght`, the axis default otherwise), within the axis range.
pub(crate) fn axis_value(c: &photocraft_doc::text::CharStyle, (tag, min, default, max): &Axis) -> f32 {
    let v = c.variations.iter().find(|v| &v.axis == tag).map(|v| v.value).unwrap_or_else(|| match tag.as_str() {
        "wght" => f32::from(c.weight),
        "ital" if c.italic => *max,
        _ => *default,
    });
    let d = default.clamp(*min, *max);
    if v.is_finite() { v.clamp(*min, *max) } else { d }
}

/// Slider label for an axis: the registered axes by name, others by their tag (as the font
/// names them in its `fvar` table, e.g. `GRAD`).
fn axis_label(tag: &str) -> String {
    match tag {
        "wght" => tl!("Weight").to_string(),
        "wdth" => tl!("Width").to_string(),
        "slnt" => tl!("Slant").to_string(),
        "ital" => tl!("Italic").to_string(),
        _ => tag.to_string(),
    }
}

/// One slider row per variable-font axis of the style's font, below the font style (#2985).
fn axis_sliders(app: &mut PhotocraftApp, ui: &mut egui::Ui, c: &photocraft_doc::text::CharStyle) {
    // Drawn every frame: no sliders until the background font scan is done (#3021).
    let Some(engine) = photocraft_text::try_shared() else { return };
    let axes = {
        let faces = engine.lock().unwrap_or_else(PoisonError::into_inner).fonts.faces(&c.font_family);
        slider_axes(&faces, c.italic)
    };
    let t = crate::theme::Tokens::get(ui.ctx());
    for axis in &axes {
        let (min, max) = (axis.1, axis.3);
        let mut v = axis_value(c, axis);
        let mut changed = false;
        ui.push_id(("type-axis", &axis.0), |ui| {
            ui.horizontal(|ui| {
                let axis_name = axis_label(&axis.0);
                ui.label(egui::RichText::new(&axis_name).color(t.text_dim));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    changed |= crate::widgets::value_field(ui, &mut v, min..=max, "", 74.0).accessible_name(axis_name).changed();
                });
            });
            changed |= crate::widgets::slider(ui, &mut v, min..=max, None).changed();
        });
        ui.add_space(crate::theme::ROW_GAP);
        if changed {
            // Whole units on wide axes (weight 100–900), hundredths on narrow ones (ital 0–1).
            let v = if max - min >= 10.0 { v.round() } else { (v * 100.0).round() / 100.0 };
            let mut m = serde_json::Map::new();
            m.insert(axis.0.clone(), json!(v));
            apply(app, ui.ctx(), json!({ "variations": m }));
        }
    }
}

/// PSD text can identify a face only by its PostScript name. Show its real subfamily too.
pub(crate) fn selected_style(style: &photocraft_doc::text::CharStyle) -> String {
    // Drawn every frame: while the font scan runs, the stored style name stands in.
    if let Some(ps) = &style.postscript_name
        && let Some(engine) = photocraft_text::try_shared()
    {
        let faces = engine.lock().unwrap_or_else(|e| e.into_inner()).fonts.faces(&style.font_family);
        if let Some(face) = faces.into_iter().find(|f| f.postscript_name.as_ref() == Some(ps)) {
            return face.style;
        }
    }
    if style.font_style.is_empty() { if style.italic { "Italic".into() } else { "Regular".into() } } else { style.font_style.clone() }
}

/// Localized style label for the dropdown. The raw string stays the engine's `fontStyle` key;
/// the words (weights and "Italic") are translated in the `fontWeight` context, where "Light"
/// means a weight — unlike the Camera Raw "Light" section.
pub fn style_label(style: &str) -> String {
    style.split(' ').map(|w| crate::i18n::tr_ctx(crate::i18n::current(), "fontWeight", w)).collect::<Vec<_>>().join(" ")
}

/// Starts scanning the installed fonts on a background thread (desktop start-up), so choosing
/// the Type tool doesn't stall the UI while they are read (#3021). Does nothing on the web.
pub fn warm_fonts() {
    photocraft_text::warm_shared();
}

/// Whether the font database is ready. The installed fonts are scanned on a background thread
/// from launch (`photocraft_text::warm_shared`); until that finishes, font menus draw this
/// disabled "Loading fonts…" box `width` wide instead of blocking the UI, and a repaint picks the
/// menus up once the scan is done.
pub(crate) fn fonts_ready(ui: &mut egui::Ui, width: f32) -> bool {
    if photocraft_text::try_shared().is_some() {
        return true;
    }
    let label = tl!("Loading fonts…").to_string();
    ui.add_enabled_ui(false, |ui| crate::widgets::dropdown(ui, "fonts-loading", tl!("Font"), &mut label.clone(), &[(label.clone(), label.as_str())], width));
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    false
}

/// Searchable font-family combo box.
fn font_picker(ui: &mut egui::Ui, current: &mut String, width: f32, preview: photocraft_engine::prefs::FontPreview, recents: &[String]) -> bool {
    family_picker_in(ui, "type-font", current, width, &families(), preview, recents)
}

/// Maximum height of the font menu.
const FONT_MENU_HEIGHT: f32 = 460.0;

/// [`font_picker`] over a given family list (tests pass their own).
#[cfg(test)]
fn font_picker_in(
    ui: &mut egui::Ui,
    current: &mut String,
    width: f32,
    families: &[String],
    preview: photocraft_engine::prefs::FontPreview,
    recents: &[String],
) -> bool {
    family_picker_in(ui, "type-font", current, width, families, preview, recents)
}

/// Shared by the Type options, Character, style and Glyphs panels. Selection changes still
/// flow through each caller's existing engine command.
pub(crate) fn family_picker_in(
    ui: &mut egui::Ui,
    salt: &str,
    current: &mut String,
    width: f32,
    families: &[String],
    preview: photocraft_engine::prefs::FontPreview,
    recents: &[String],
) -> bool {
    let mut changed = false;
    // A font picked from the open menu (click or Enter): the field keeps the focus afterwards.
    let mut picked = false;
    let search_id = ui.scope_id().with(("font-search", salt));
    let focus_id = search_id.with("focus");
    let recents_id = search_id.with("recents");
    let row_id = search_id.with("row");
    // The list opens on the press, so a press-drag-release picks a font too (#2735).
    let (press, close) = crate::press_menu::PressCombo::before(ui, salt);
    let labels: Option<Arc<HashMap<String, String>>> = ui.ctx().data(|d| d.get_temp(font_labels_id()));
    let labels = labels.as_deref();
    let shown = labels.and_then(|l| l.get(current.as_str())).unwrap_or(current).clone();
    let combo = egui::ComboBox::from_id_salt(salt)
        .selected_text(shown)
        .width(width)
        .height(FONT_MENU_HEIGHT)
        .icon(crate::widgets::chevron_icon)
        // Clicks inside the menu (the search field, the scroll bar) must not close it (#1369);
        // picking a font closes it explicitly below.
        .close_behavior(if matches!(close, egui::PopupCloseBehavior::IgnoreClicks) { close } else { egui::PopupCloseBehavior::CloseOnClickOutside });

    let mut response = combo.show_ui(ui, |ui| {
        let height_id = search_id.with("height");
        // Each opening focuses the search, including a reopen on the very next pass. Keep
        // asking through the invisible sizing pass, until the field receives focus.
        let mut focus = ui.data(|d| d.get_temp(focus_id)).unwrap_or(true);
        let mut q: String = ui.data(|d| d.get_temp(search_id)).unwrap_or_default();
        // Consume these before TextEdit and the canvas can interpret them as caret movement.
        let down = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown));
        let up = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp));
        let enter = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        if focus {
            ui.memory_mut(|m| m.request_focus(search_id.with("input")));
        }
        let r = ui.add(egui::TextEdit::singleline(&mut q).id(search_id.with("input")).hint_text(tl!("Search fonts")).desired_width(200.0));
        if focus {
            // The opening click can release after several focused frames. Keep the request
            // pending through that release, which can hand focus back to the combo button.
            focus = !r.has_focus() || ui.input(|i| i.pointer.primary_down() || i.pointer.primary_released());
        }
        ui.data_mut(|d| {
            d.insert_temp(focus_id, focus);
            d.insert_temp(search_id, q.clone());
        });
        let ql = q.to_lowercase();
        // The recent group stays as it was when the menu opened: each arrow step applies (and so
        // records) a family, and moving it to the top would send the next step back up.
        let opening = ui.data(|d| d.get_temp::<Vec<String>>(recents_id).is_none());
        let open_recents: Vec<String> = ui.data_mut(|d| d.get_temp_mut_or_insert_with(recents_id, || recents.to_vec()).clone());
        let (rows, n_recent) = font_menu_rows(families, labels, &ql, &open_recents);
        // A recent family is listed twice. The current family's row is the one the last arrow
        // selected while it still shows that family, else its first copy: as in Photoshop, the
        // one in the recent group when it has one (#9160), and the arrows walk on from there.
        let mut selected = ui
            .data(|d| d.get_temp::<usize>(row_id))
            .filter(|&i| rows.get(i).is_some_and(|(f, _)| *f == current))
            .or_else(|| rows.iter().position(|(f, _)| *f == current));
        // The menu opens on the current family, as Photoshop's does: with the recent group at the
        // top of the view when the marked row fits below it, else the marked row itself, at the
        // top of the view when it is not a recent one. Applied on the first laid-out pass (the
        // popup's first pass only measures), whether or not a row matches, so a missing font or
        // a filtered list never scrolls the menu later.
        let reveal_id = search_id.with("reveal");
        if opening {
            ui.data_mut(|d| d.insert_temp(reveal_id, true));
        }
        let revealing = ui.data(|d| d.get_temp::<bool>(reveal_id)).unwrap_or(false) && !ui.is_sizing_pass();
        if revealing {
            ui.data_mut(|d| d.remove::<bool>(reveal_id));
        }
        let reveal = if revealing { selected } else { None };
        let reveal_from = reveal.map(|i| if i < n_recent { 0 } else { i });
        let mut stepped = None;
        if down || up {
            let next = match selected {
                Some(i) if down => i.saturating_add(1).min(rows.len().saturating_sub(1)),
                Some(i) => i.saturating_sub(1),
                None if down => 0,
                None => rows.len().saturating_sub(1),
            };
            if let Some((f, _)) = rows.get(next) {
                changed = **f != *current;
                current.clone_from(f);
                // A served family not fetched yet: start now, as a click does.
                photocraft_text::served::request(f);
                stepped = Some(next);
                selected = Some(next);
                ui.data_mut(|d| d.insert_temp(row_id, next));
            }
        }
        let accept = enter && rows.iter().any(|(f, _)| *f == current);
        picked |= accept;
        if accept {
            ui.data_mut(|d| d.remove::<String>(search_id));
            ui.close();
        }
        let preview = crate::font_preview::sample(preview);
        let row_h = preview.map_or(28.0, |(_, [_, h])| (h + 4.0).max(28.0));

        // Wrap the font rows in a ScrollArea so only the list scrolls: the search field above
        // stays fixed at the top of the popup.
        let mut hovered: Option<String> = None;
        let reveal = if stepped.is_some() { None } else { reveal };
        let mut reveal_top: Option<egui::Rect> = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (i, (f, label)) in rows.into_iter().enumerate() {
                if i == n_recent && n_recent > 0 {
                    ui.separator();
                }
                // Highlighted by its fill alone, as Photoshop's font menu is. A selectable button lays
                // out its frame's stroke only once hovered or selected, so with the theme's stroke the
                // row under the pointer (and the selected one) grew and pushed the rows below down:
                // the list jumped as the pointer moved (#7406). One row only, though a recent
                // family is listed twice.
                let mut button = egui::Button::selectable(selected == Some(i), label).truncate().stroke(egui::Stroke::NONE);
                if let Some((size, [_, h])) = preview {
                    button = button.right_text(egui::Atom::custom(
                        egui::IdSalt::new(("font-sample", f)),
                        egui::vec2(crate::font_preview::slot_width(ui.ctx(), f, size), h),
                    ));
                }
                let response = ui.add_sized([330.0, row_h], button);
                if stepped == Some(i) {
                    response.scroll_to_me(Some(egui::Align::Center));
                } else if reveal.is_some() {
                    if reveal_from == Some(i) {
                        reveal_top = Some(response.rect);
                    }
                    if reveal == Some(i) {
                        // Jump, not glide: the menu opens with its current family in view. Arrow
                        // steps above keep the animation. A recent row too far below the top of
                        // the group (large previews, many recents) is brought up to the bottom.
                        let top = reveal_top.unwrap_or(response.rect);
                        let (rect, align) = if response.rect.bottom() - top.top() <= ui.clip_rect().height() {
                            (top, egui::Align::Min)
                        } else {
                            (response.rect, egui::Align::Max)
                        };
                        ui.scroll_to_rect_animation(rect, Some(align), egui::style::ScrollAnimation::none());
                    }
                }
                if let Some((size, _)) = preview
                    && ui.is_rect_visible(response.rect)
                {
                    crate::font_preview::paint(ui, f, response.rect, size);
                }
                if response.hovered() {
                    hovered = Some(f.clone());
                }
                if press.chosen(ui, &response) {
                    *current = f.clone();
                    // A served family not fetched yet: start now, before any text needs it.
                    photocraft_text::served::request(f);
                    changed = true;
                    picked = true;
                    ui.data_mut(|d| d.remove::<String>(search_id));
                    hovered = None;
                    ui.close();
                }
            }
        });
        // egui measures a popup only when it opens and never lets it grow back, so a filtered
        // list would leave the menu short after the query is cleared (#1369). Keep the unfiltered
        // height while searching. Measured after the rows: before them `min_rect` only holds the
        // search field, and the cache would shrink the menu to it.
        if ql.is_empty() {
            let h = ui.min_rect().height();
            ui.data_mut(|d| d.insert_temp(height_id, h));
        } else if let Some(h) = ui.data(|d| d.get_temp::<f32>(height_id)) {
            ui.set_min_height(h.min(FONT_MENU_HEIGHT));
        }
        if accept { None } else { hovered }
    });
    response.response = response.response.accessible_name(tl!("Font"));
    press.after(&response.response);
    if !egui::ComboBox::is_open(ui.ctx(), response.response.id) {
        ui.data_mut(|d| {
            d.remove::<bool>(focus_id);
            d.remove::<Vec<String>>(recents_id);
            d.remove::<usize>(row_id);
            d.remove::<bool>(search_id.with("reveal"));
        });
    }
    let hovered_font = response.inner.unwrap_or_default();

    // As in Photoshop, the field keeps the keyboard focus once a font is picked, and ↑ / ↓ on the
    // focused, closed field step to the previous / next family, applying each (#7407). The lock
    // filter keeps egui from using those arrows to move the focus to another widget.
    let field = response.response.id;
    if picked {
        ui.memory_mut(|m| m.request_focus(field));
    }
    let open = egui::ComboBox::is_open(ui.ctx(), field);
    if !open && ui.is_enabled() && ui.memory(|m| m.has_focus(field)) {
        ui.memory_mut(|m| m.set_focus_lock_filter(field, egui::EventFilter { vertical_arrows: true, ..Default::default() }));
        let down = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown));
        let up = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp));
        if down || up {
            // The lock filter only holds from the frame after the field got the focus: an arrow
            // right after a pick was already read by egui as a focus move, applied at the end of
            // the frame. Cancel it, so the arrow only steps the font.
            ui.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
        }
        // The wheel's step: a negative notch is the next family. Steps use a stable order (no
        // recent group), so recording the picked family cannot make the next step jump back.
        let notches = f32::from(u8::from(up)) - f32::from(u8::from(down));
        let (rows, _) = font_menu_rows(families, labels, "", &[]);
        if let Some(i) = rows.iter().position(|(f, _)| *f == current)
            && let Some(n) = wheel_family_index(i, rows.len(), notches)
            && let Some((f, _)) = rows.get(n)
            && **f != *current
        {
            current.clone_from(f);
            photocraft_text::served::request(f);
            changed = true;
        }
    }

    // The wheel over the closed button steps through the families, a notch each, like Photoshop's
    // options-bar font menu: no need to open the list (#3350).
    if !egui::ComboBox::is_open(ui.ctx(), response.response.id) && ui.is_enabled() {
        let hovered = response.response.hovered();
        if hovered {
            ui.input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
        }
        let notches = crate::widgets::wheel_notches_of(ui, response.response.id.with("family-wheel"), hovered, 1.0);
        // The wheel steps in a stable order: a recent group holding the family it just picked
        // would change the order between notches and can jump back to the previous family.
        // The open menu still shows the recent families first.
        let (rows, _) = font_menu_rows(families, labels, "", &[]);
        if let Some(i) = rows.iter().position(|(f, _)| *f == current)
            && let Some(n) = wheel_family_index(i, rows.len(), notches)
            && let Some((f, _)) = rows.get(n)
            && **f != *current
        {
            current.clone_from(f);
            photocraft_text::served::request(f);
            changed = true;
        }
    }

    if salt == "type-font" {
        let hover_id = egui::Id::unique("type-font-hover");
        if let Some(f) = hovered_font {
            ui.ctx().data_mut(|d| d.insert_temp(hover_id, f));
        } else if egui::ComboBox::is_open(ui.ctx(), response.response.id) {
            // Only the open menu clears it: the options bar and the Character section both draw
            // this picker, and the closed one must not wipe the other's hover (#5152).
            ui.ctx().data_mut(|d| d.remove::<String>(hover_id));
        }
    }

    changed
}

/// The row `notches` wheel notches from row `index` of `len` (down, negative, is the next one),
/// stopping at the ends; none when the wheel did not move.
fn wheel_family_index(index: usize, len: usize, notches: f32) -> Option<usize> {
    let steps = notches.abs().min(len as f32) as usize;
    if steps == 0 || len == 0 {
        return None;
    }
    let last = len - 1;
    Some(if notches < 0.0 { index.saturating_add(steps).min(last) } else { index.saturating_sub(steps) })
}

/// Representative for the displayed properties: the edit layer, else a selected type layer.
pub(crate) fn target(app: &PhotocraftApp) -> Option<(u64, Option<[usize; 2]>)> {
    if let Some(ed) = &app.ui.text_edit {
        let (a, b) = (ed.caret.min(ed.anchor), ed.caret.max(ed.anchor));
        return Some((ed.layer, (a < b).then_some([a, b])));
    }
    let st = app.session.active()?;
    let id = st
        .active_layer
        .filter(|id| text_layer(&st.doc, *id).is_some())
        .or_else(|| st.selected_layers().into_iter().find(|id| text_layer(&st.doc, *id).is_some()))?;
    Some((id.0, None))
}

/// The type layer whose properties the Type controls and dialogs show (see [`target`]).
pub(crate) fn target_text(app: &PhotocraftApp) -> Option<&TextLayer> {
    let (id, _) = target(app)?;
    text_layer(&target_doc(app)?.doc, LayerId(id))
}

/// The document [`target`]'s layer is in: a text edit's own, which can stay in a background
/// document where the same layer id may name another layer, else the active one.
fn target_doc(app: &PhotocraftApp) -> Option<&photocraft_engine::DocState> {
    match &app.ui.text_edit {
        Some(ed) => app.session.documents().get(edit_home(app, ed)?),
        None => app.session.active(),
    }
}

/// Snapshot the mutation scope separately from the representative used to display properties.
/// An inline edit remains on its layer/range; layer selection formats every selected type layer.
pub(crate) fn formatting_params(app: &PhotocraftApp) -> Option<serde_json::Value> {
    if let Some(ed) = &app.ui.text_edit {
        let mut params = json!({"layer": ed.layer, "coalesce": ed.session});
        let (a, b) = (ed.caret.min(ed.anchor), ed.caret.max(ed.anchor));
        if a < b {
            params["range"] = json!([a, b]);
        }
        return Some(params);
    }
    let st = app.session.active()?;
    let layers: Vec<u64> = st.selected_layers().into_iter().filter(|id| text_layer(&st.doc, *id).is_some()).map(|id| id.0).collect();
    (!layers.is_empty()).then(|| json!({"layers": layers}))
}

/// Scale of the target layer's transform. Like Photoshop, sizes show and edit as the layer
/// appears: a 12 pt layer scaled 200% (common in PSDs) reads 24 pt.
fn shown_scale(app: &PhotocraftApp) -> f32 {
    let Some((id, _)) = target(app) else { return 1.0 };
    let Some(t) = app.session.active().and_then(|st| text_layer(&st.doc, LayerId(id))) else { return 1.0 };
    let m = t.transform.m;
    let k = (m[0] * m[3] - m[1] * m[2]).abs().sqrt() as f32;
    if k.is_finite() && k > 1e-3 { k } else { 1.0 }
}

/// The display unit for type metrics (Preferences ▸ Units & Rulers ▸ Type): the multiplier from
/// the model's points to the unit, and its label. Pixels follow the document's resolution.
pub(crate) fn type_unit(app: &PhotocraftApp) -> (f32, &'static str) {
    let dpi = app.session.active().map_or(72.0, |d| d.doc.resolution_dpi.max(1.0));
    match app.session.prefs().units_and_rulers.type_units {
        photocraft_engine::prefs::TypeUnit::Points => (1.0, "pt"),
        photocraft_engine::prefs::TypeUnit::Pixels => (dpi / 72.0, "px"),
        photocraft_engine::prefs::TypeUnit::Millimeters => (25.4 / 72.0, "mm"),
    }
}

/// Coalesce key while a field is being scrubbed: every step of one drag shares one history step
/// (Photoshop's single step per scrub, #124). Unique per press, so two drags are two steps.
fn drag_key(ctx: &egui::Context) -> Option<String> {
    let id = ctx.dragged_id()?;
    let t = ctx.input(|i| i.pointer.press_start_time())?;
    Some(format!("type-drag-{}-{}", id.value(), t.to_bits()))
}

/// The styles a new Type-tool layer will receive. Use the same command parameters for the
/// panels and creation so defaults also survive action recording/replay. The older options-bar
/// fields remain authoritative, and text colour continues to follow the foreground colour.
fn default_style_params(app: &PhotocraftApp) -> serde_json::Value {
    let o = &app.ui.tool_options;
    let mut p = json!({});
    // Settings are editable JSON: only recognized formatting keys may reach type.create.
    // In particular a stray `text`, `box`, or `coalesce` key must not become a create parameter.
    let (mut c, mut para) = (photocraft_doc::text::CharStyle::default(), photocraft_doc::text::ParagraphStyle::default());
    for (key, value) in &o.type_style_defaults {
        // These belong to the existing options-bar fields. Apart from being redundant,
        // parsing a saved font key here would wait for the background font scan.
        if matches!(key.as_str(), "font" | "family" | "fontStyle" | "weight" | "italic" | "postscriptName") {
            continue;
        }
        let one = json!({ (key): value });
        if !finite_style_value(value)
            || photocraft_engine::type_cmds::check_size_tracking(&one).is_err()
            || photocraft_engine::type_cmds::check_kerning(&one).is_err()
        {
            continue;
        }
        if photocraft_engine::type_cmds::apply_char_props(&mut c, &one) | photocraft_engine::type_cmds::apply_para_props(&mut para, &one) {
            p[key] = value.clone();
        }
    }
    p["font"] = json!(o.type_font);
    p["fontStyle"] = json!(o.type_style);
    p["size"] = json!(o.type_size);
    p["align"] = json!(o.type_align);
    p["direction"] = json!(o.type_direction);
    p["color"] = json!(hex(app.session.tools.foreground));
    p
}

/// Style numbers become f32 in the engine. JSON itself permits finite f64 values too large
/// for that conversion; a hand-edited setting must not inject infinity into a new layer.
/// Style collections (features, axes, colour) contain only scalar values, so no recursion.
fn finite_style_value(value: &serde_json::Value) -> bool {
    let scalar = |v: &serde_json::Value| match v {
        serde_json::Value::Number(n) => n.as_f64().is_some_and(|n| (n as f32).is_finite()),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => false,
        _ => true,
    };
    match value {
        serde_json::Value::Array(v) => v.iter().all(scalar),
        serde_json::Value::Object(v) => v.values().all(scalar),
        v => scalar(v),
    }
}

fn default_styles(app: &PhotocraftApp) -> (photocraft_doc::text::CharStyle, photocraft_doc::text::ParagraphStyle) {
    let (mut c, mut para) = app.session.type_defaults.clone().unwrap_or_default();
    let mut p = default_style_params(app);
    // The saved panels can appear on the very first frame. Style parsing resolves the named
    // font through shared(), which waits for the font scan; display the saved names meanwhile,
    // and let fonts_ready/axis_sliders expose the font controls when try_shared is ready.
    if photocraft_text::try_shared().is_none() {
        c.font_family = app.ui.tool_options.type_font.clone();
        c.font_style = app.ui.tool_options.type_style.clone();
        c.postscript_name = None;
        if let Some(p) = p.as_object_mut() {
            p.remove("font");
            p.remove("fontStyle");
        }
    }
    photocraft_engine::type_cmds::apply_char_props(&mut c, &p);
    photocraft_engine::type_cmds::apply_para_props(&mut para, &p);
    (c, para)
}

/// Change tool defaults without creating a document edit. The engine owns the style schema;
/// keep its property names rather than maintaining another style parser in the shell.
fn apply_defaults(app: &mut PhotocraftApp, props: &serde_json::Value) {
    let Some(props) = props.as_object() else { return };
    let o = &mut app.ui.tool_options;
    for (key, value) in props {
        match key.as_str() {
            "font" => {
                if let Some(v) = value.as_str() {
                    o.type_font = v.into();
                }
            }
            "fontStyle" => {
                if let Some(v) = value.as_str() {
                    o.type_style = v.into();
                    // A newly chosen named style replaces a weight-axis setting, as it does
                    // in apply_char_props. Other axes retain their independently chosen values.
                    if let Some(axes) = o.type_style_defaults.get_mut("variations").and_then(serde_json::Value::as_object_mut) {
                        axes.remove("wght");
                    }
                }
            }
            "size" => {
                if let Some(v) = value.as_f64() {
                    o.type_size = v as f32;
                }
            }
            "align" => {
                if let Some(v) = value.as_str() {
                    o.type_align = v.into();
                }
            }
            "direction" => {
                if let Some(v) = value.as_str() {
                    o.type_direction = v.into();
                }
            }
            "metricsAsShown" => {} // Defaults have no layer transform.
            "features" | "variations" => {
                if let Some(patch) = value.as_object() {
                    let old = o.type_style_defaults.entry(key.clone()).or_insert_with(|| json!({}));
                    if let Some(old) = old.as_object_mut() {
                        old.extend(patch.clone());
                    } else {
                        *old = value.clone();
                    }
                }
            }
            _ => {
                o.type_style_defaults.insert(key.clone(), value.clone());
            }
        }
    }
}

/// Apply only the supplied properties to the text-edit scope or selected type layers; without
/// a type target, prepare the defaults for the next layer instead.
fn apply(app: &mut PhotocraftApp, ctx: &egui::Context, props: serde_json::Value) {
    let Some(mut p) = formatting_params(app) else {
        apply_defaults(app, &props);
        return;
    };
    if let Some(props) = props.as_object() {
        for (key, value) in props {
            p[key] = value.clone();
        }
    }
    if let Some(key) = app.ui.text_edit.as_ref().map(|ed| ed.session.clone()).or_else(|| drag_key(ctx)) {
        p["coalesce"] = json!(key);
    }
    let _ = app.run("type.setStyle", p);
}

/// `props` aimed at the type target: its layer, the selected characters while typing, and the
/// typing session's undo step. `None` without a type target.
pub(crate) fn with_target(app: &PhotocraftApp, mut props: serde_json::Value) -> Option<serde_json::Value> {
    let (layer, range) = target(app)?;
    props["layer"] = json!(layer);
    if let Some(r) = range {
        props["range"] = json!(r);
    }
    if let Some(key) = app.ui.text_edit.as_ref().map(|ed| ed.session.clone()) {
        props["coalesce"] = json!(key);
    }
    Some(props)
}

/// Open the shared Color Picker with a snapshot of the text target. Sampling or cancelling
/// the dialog never changes the text selection, tool colours, or document history.
pub fn open_color_picker(app: &mut PhotocraftApp, rgb: [f32; 3]) -> u64 {
    if let Some(params) = formatting_params(app) {
        crate::color_picker_ui::open_for_command(app, "Color Picker (Text Color)", rgb, "type.setStyle", params)
    } else {
        crate::color_picker_ui::open(app, "foreground")
    }
}

/// While characters are selected in a type layer, a new foreground colour (Color and Swatches
/// panels, the Color Picker) recolours them, in the editing session's history step.
pub fn foreground_changed(app: &mut PhotocraftApp) {
    let Some((layer, Some(range))) = target(app) else { return };
    let Some(ed) = app.ui.text_edit.as_ref() else { return };
    let p = json!({"layer": layer, "range": range, "color": hex(app.session.tools.foreground), "coalesce": ed.session});
    let _ = app.run("type.setStyle", p);
}

/// Photoshop's Type options bar.
pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = crate::theme::Tokens::get(ui.ctx());
    // Show the target layer's (first-run) style, else the tool defaults.
    let shown = target(app).and_then(|(id, _)| {
        let st = app.session.active()?;
        let tl = text_layer(&st.doc, LayerId(id))?;
        let run = tl.char_runs().into_iter().next().map(|r| r.style);
        // The size at the selection (else the first run), as the layer shows it (#124).
        let size = styles_at(app).map_or(tl.size_pt, |(c, _)| c.size_pt) * shown_scale(app);
        Some((tl.font_family.clone(), run.as_ref().map(selected_style).unwrap_or_default(), size, styles_at(app).map_or(tl.color, |(c, _)| c.color)))
    });
    let o = app.ui.tool_options.clone();
    let (mut fam, mut style, size) = match &shown {
        Some((f, s, z, _)) => (f.clone(), if s.is_empty() { "Regular".into() } else { s.clone() }, *z),
        None => (o.type_font.clone(), o.type_style.clone(), o.type_size),
    };
    if crate::icons::button(ui, "text-cursor", 24.0, false, tl!("Toggle text orientation")).clicked()
        && let Some((layer, _)) = target(app)
        && let Some(params) = formatting_params(app)
    {
        let to = if is_vertical(app, LayerId(layer)) { "horizontal" } else { "vertical" };
        if let Err(e) = app.run(&format!("type.orientation.{to}"), params) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
    // Font family and style menus (one placeholder in their place while the fonts load).
    if fonts_ready(ui, 170.0 + ui.spacing().item_spacing.x + 110.0) {
        if font_picker(ui, &mut fam, 170.0, app.session.prefs().type_.font_preview, &app.ui.recent_fonts) {
            app.note_recent_font(&fam);
            app.ui.tool_options.type_font = fam.clone();
            let st = styles(&fam);
            style = if st.contains(&style) { style } else { st.first().cloned().unwrap_or_else(|| "Regular".into()) };
            app.ui.tool_options.type_style = style.clone();
            apply(app, ui.ctx(), json!({"font": fam}));
        }
        let opts: Vec<(String, String)> = styles(&fam).into_iter().map(|s| (s.clone(), style_label(&s))).collect();
        let opts_ref: Vec<(String, &str)> = opts.iter().map(|(a, b)| (a.clone(), b.as_str())).collect();
        if crate::widgets::dropdown(ui, "type-style", tl!("Style"), &mut style, &opts_ref, 110.0) {
            app.ui.tool_options.type_style = style.clone();
            apply(app, ui.ctx(), json!({"fontStyle": style}));
        }
    }
    let (r, icon) = ui.allocate_exact_size(egui::vec2(18.0, 22.0), egui::Sense::hover());
    crate::icons::paint(ui, r, "type", 13.0, t.icon);
    icon.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, tl!("Font size")));
    // Photoshop's "T" scrubs the size field beside it, like the Character panel's icons (#6437).
    crate::widgets::scrub_option_icon(ui, &icon, tl!("Font size"));
    let (unit, label) = type_unit(app);
    let mut display = size * unit;
    if crate::widgets::value_field(ui, &mut display, unit..=1296.0 * unit, label, 66.0).changed() {
        let size = display / unit;
        app.ui.tool_options.type_size = size;
        apply(app, ui.ctx(), json!({"size": size, "metricsAsShown": true}));
    }
    let mut aa = target(app)
        .and_then(|(id, _)| app.session.active().and_then(|st| text_layer(&st.doc, LayerId(id))))
        .map(|t| photocraft_engine::type_extra_cmds::aa_name(t.antialias).to_string())
        .unwrap_or_else(|| o.type_aa.clone());
    let aa_opts = [
        ("none".to_string(), tl!("None")),
        ("sharp".to_string(), tl!("Sharp")),
        ("crisp".to_string(), tl!("Crisp")),
        ("strong".to_string(), tl!("Strong")),
        ("smooth".to_string(), tl!("Smooth")),
    ];
    if crate::widgets::dropdown(ui, "type-aa", tl!("Anti-aliasing method"), &mut aa, &aa_opts, 80.0) {
        app.ui.tool_options.type_aa = aa.clone();
        if let Some(params) = formatting_params(app) {
            let _ = app.run(&format!("type.antiAlias.{aa}"), params);
        }
    }
    crate::widgets::vline(ui, 22.0);
    ui.spacing_mut().item_spacing.x = 2.0;
    // Alignment and direction show the target paragraph's, as Photoshop's bar follows the text
    // being edited or the selected type layer; the tool defaults otherwise. A justified paragraph
    // lights none of the three.
    let para = styles_at(app).map(|(_, p)| p);
    let shown_align = match &para {
        Some(p) => match p.align {
            photocraft_doc::text::TextAlign::Left => "left",
            photocraft_doc::text::TextAlign::Center => "center",
            photocraft_doc::text::TextAlign::Right => "right",
            _ => "",
        },
        None => o.type_align.as_str(),
    };
    for (align, icon, tip) in
        [("left", "align-left", tl!("Left align text")), ("center", "align-center", tl!("Center text")), ("right", "align-right", tl!("Right align text"))]
    {
        if crate::icons::button(ui, icon, 24.0, shown_align == align, tip).clicked() {
            app.ui.tool_options.type_align = align.into();
            apply(app, ui.ctx(), json!({"align": align}));
        }
    }
    ui.spacing_mut().item_spacing.x = 8.0;
    crate::widgets::vline(ui, 22.0);
    ui.spacing_mut().item_spacing.x = 2.0;
    for (key, tip, d) in direction_items() {
        let on = para.as_ref().map_or(o.type_direction == key, |p| p.direction == d);
        if direction_button(ui, d, on, tip).clicked() {
            app.ui.tool_options.type_direction = key.into();
            apply(app, ui.ctx(), json!({"direction": key}));
        }
    }
    ui.spacing_mut().item_spacing.x = 8.0;
    crate::widgets::vline(ui, 22.0);
    // Text colour swatch with a picker popup.
    let c = shown
        .as_ref()
        .map(|s| s.3)
        .map(|c| {
            let v = c.to_rgb();
            [v[0], v[1], v[2], 1.0]
        })
        .unwrap_or(app.session.tools.foreground);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(28.0, 18.0), egui::Sense::click());
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    ui.painter().rect_filled(rect, 2.0, Color32::from_rgb(q(c[0]), q(c[1]), q(c[2])));
    ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, t.field_border), egui::StrokeKind::Outside);
    let resp = resp.on_hover_text(tl!("Set the text color"));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Set the text color")));
    if resp.clicked() {
        open_color_picker(app, [c[0], c[1], c[2]]);
    }
    // Create warped text, after the swatch as in Photoshop (#5911): the Type › Warp Text… dialog.
    let can_warp = crate::menus::is_enabled(app, "type.warpText");
    if ui.add_enabled_ui(can_warp, |ui| crate::icons::button(ui, "text-warp", 24.0, false, tl!("Create warped text"))).inner.clicked()
        && let Err(e) = crate::menus::invoke(app, &ui.ctx().clone(), "type.warpText", json!({}))
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    if app.ui.text_edit.is_some() {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(8.0);
            if crate::icons::button(
                ui,
                "check",
                24.0,
                false,
                &crate::i18n::fmt(tl!("Commit any current edits ({key})"), &[("key", &crate::shortcuts::pretty("Cmd+Enter"))]),
            )
            .clicked()
            {
                commit_with(app, ui.input(|i| i.modifiers));
            }
            if crate::icons::button(ui, "ban", 24.0, false, tl!("Cancel any current edits (Esc)")).clicked() {
                cancel(app);
            }
        });
    }
}

/// Character and paragraph style at the target (selection start, else the layer's first run).
/// The paragraph at a collapsed caret is the caret's, as Photoshop's options bar and Paragraph
/// panel show it.
pub(crate) fn styles_at(app: &PhotocraftApp) -> Option<(photocraft_doc::text::CharStyle, photocraft_doc::text::ParagraphStyle)> {
    let (layer, range) = target(app)?;
    let t = text_layer(&target_doc(app)?.doc, LayerId(layer))?;
    let at = range.map_or(0, |r| byte_of(&t.text, r[0]));
    let para_at = match (range, &app.ui.text_edit) {
        (None, Some(ed)) => byte_of(&t.text, ed.caret),
        _ => at,
    };
    let pick_at = |at: usize, lens: Vec<usize>| -> usize {
        let mut acc = 0;
        for (i, len) in lens.iter().enumerate() {
            acc += len;
            if at < acc {
                return i;
            }
        }
        lens.len().saturating_sub(1)
    };
    let runs = t.char_runs();
    let paras = t.paragraph_runs();
    let c = runs.get(pick_at(at, runs.iter().map(|r| r.len).collect())).map(|r| r.style.clone())?;
    let p = paras.get(pick_at(para_at, paras.iter().map(|r| r.len).collect())).map(|r| r.style.clone()).unwrap_or_default();
    Some((c, p))
}

/// Kerning shown for the target: at a collapsed caret, the pair before it (Photoshop's
/// Character panel); else the style at the selection.
fn kerning_at(app: &PhotocraftApp) -> Option<(photocraft_doc::text::Kerning, f32)> {
    let ed = app.ui.text_edit.as_ref().filter(|e| e.caret == e.anchor && e.caret > 0)?;
    let st = app.session.active()?;
    let t = text_layer(&st.doc, LayerId(ed.layer))?;
    let at = byte_of(&t.text, ed.caret - 1);
    let mut end = 0;
    t.char_runs()
        .into_iter()
        .find(|r| {
            end += r.len;
            at < end
        })
        .map(|r| (r.style.kerning, r.style.kern))
}

/// The Character panel's kerning text: "Metrics", "Optical" or the manual value.
pub(crate) fn kerning_label((mode, kern): (photocraft_doc::text::Kerning, f32)) -> String {
    use photocraft_doc::text::Kerning;
    match mode {
        _ if kern != 0.0 && kern.is_finite() => format!("{}", kern.round() as i64),
        Kerning::Metrics => "Metrics".into(),
        Kerning::Optical => "Optical".into(),
        Kerning::Off => "0".into(),
    }
}

/// Parses a typed kerning value: Metrics / Optical (any case) or a number in 1/1000 em
/// (Photoshop's range, -1000..10000). `None` for anything else.
pub(crate) fn parse_kerning(s: &str) -> Option<serde_json::Value> {
    let s = s.trim();
    match s.to_ascii_lowercase().as_str() {
        "metrics" => Some(json!("metrics")),
        "optical" => Some(json!("optical")),
        _ => crate::numeric_expression::parse(s).filter(|v| v.is_finite() && (-1000.0..=10000.0).contains(v)).map(|v| json!(v.round())),
    }
}

/// Photoshop's editable kerning combo: type a value or Metrics/Optical, or pick a preset.
fn kerning_field(ui: &mut egui::Ui, shown: &str, width: f32) -> Option<serde_json::Value> {
    let id = ui.make_persistent_id("props-kern-text");
    let mut buf: String = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| shown.to_string());
    let mut out = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let resp = ui.add(egui::TextEdit::singleline(&mut buf).desired_width((width - 40.0).max(24.0)).id(id.with("edit"))).accessible_name(tl!("Kerning"));
        if resp.has_focus() {
            ui.data_mut(|d| d.insert_temp(id, buf.clone()));
        } else {
            ui.data_mut(|d| d.remove::<String>(id));
        }
        if resp.lost_focus() && buf.trim() != shown {
            out = parse_kerning(&buf);
        }
        let presets = ["Metrics", "Optical", "0", "-100", "-75", "-50", "-25", "-10", "-5", "5", "10", "25", "50", "75", "100", "200"];
        let (press, close) = crate::press_menu::PressCombo::before(ui, "props-kern-presets");
        let combo = egui::ComboBox::from_id_salt("props-kern-presets")
            .selected_text("")
            .width(16.0)
            .height(420.0)
            .icon(crate::widgets::chevron_icon)
            .close_behavior(close)
            .show_ui(ui, |ui| {
                for p in presets {
                    let item = ui.selectable_label(p == shown, p);
                    if press.chosen(ui, &item) {
                        out = parse_kerning(p);
                    }
                }
            });
        press.after(&combo.response);
        let _ = combo.response.accessible_name(tl!("Kerning"));
    });
    out
}

/// Applies a kerning value: at a collapsed caret to the pair before it, else to the selection
/// (or the whole layer).
fn apply_kerning(app: &mut PhotocraftApp, ctx: &egui::Context, v: serde_json::Value) {
    match app.ui.text_edit.clone().filter(|e| e.caret == e.anchor) {
        Some(ed) if ed.caret > 0 => {
            let _ = app.run("type.edit", json!({"layer": ed.layer, "range": [ed.caret - 1, ed.caret], "kerning": v}));
        }
        Some(_) => {}
        None => apply(app, ctx, json!({ "kerning": v })),
    }
}

fn icon_label(ui: &mut egui::Ui, icon: &str, tip: &str) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(egui::vec2(crate::props_layout::LABEL_W, 22.0), egui::Sense::hover());
    crate::icons::paint(ui, r, icon, 12.0, t.text_dim);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, ui.is_enabled(), tl!(tip)));
    // Photoshop's Character panel icons scrub the field beside them.
    crate::widgets::scrub_label(ui, &resp.on_hover_text(tl!(tip)));
}

/// Labelled numeric field (Photoshop's icon + value pairs). Returns the new value when edited.
fn num_field(ui: &mut egui::Ui, label: &str, tip: &str, v: f32, range: std::ops::RangeInclusive<f32>, unit: &str, width: f32) -> Option<f32> {
    let t = crate::theme::Tokens::get(ui.ctx());
    let mut v = v;
    let mut out = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let (r, resp) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, label, crate::theme::semibold(10.0), t.text_dim);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, ui.is_enabled(), tl!(tip)));
        crate::widgets::scrub_label(ui, &resp.on_hover_text(tl!(tip)));
        let response = crate::widgets::value_field(ui, &mut v, range, unit, width);
        response.widget_info(|| {
            let mut info = egui::WidgetInfo::drag_value(ui.is_enabled(), f64::from(v));
            info.label = Some(tl!(tip).to_owned());
            info
        });
        if response.changed() {
            out = Some(v);
        }
    });
    out
}

/// Photoshop's paragraph alignment glyphs: four text lines whose widths/offsets show the mode
/// (justify variants run full width, with the last line placed left/centre/right).
fn align_glyph(p: &egui::Painter, r: egui::Rect, a: photocraft_doc::text::TextAlign, c: Color32) {
    use photocraft_doc::text::TextAlign as A;
    let widths = [1.0, 0.62, 1.0, 0.62];
    for (i, w) in widths.iter().enumerate() {
        let y = r.top() + i as f32 * r.height() / 3.0;
        let full = r.width();
        let (len, x0) = match a {
            A::Left => (full * w, r.left()),
            A::Center => (full * w, r.center().x - full * w / 2.0),
            A::Right => (full * w, r.right() - full * w),
            A::JustifyAll => (full, r.left()),
            _ if i < 3 => (full, r.left()),
            A::JustifyLeft => (full * 0.55, r.left()),
            A::JustifyCenter => (full * 0.55, r.center().x - full * 0.275),
            _ => (full * 0.55, r.right() - full * 0.55),
        };
        p.line_segment([egui::pos2(x0, y), egui::pos2(x0 + len, y)], Stroke::new(1.5, c));
    }
}

/// Paragraph direction choices for the Paragraph panel and the options bar (TYPE-218-2): the
/// `direction` value `type.setStyle` takes, the tooltip, and the direction it sets.
fn direction_items() -> [(&'static str, &'static str, photocraft_doc::text::TextDirection); 3] {
    use photocraft_doc::text::TextDirection as D;
    [
        ("auto", tl!("Automatic paragraph direction"), D::Auto),
        ("ltr", tl!("Left-to-right paragraph direction"), D::Ltr),
        ("rtl", tl!("Right-to-left paragraph direction"), D::Rtl),
    ]
}

/// A pilcrow over an arrow pointing the way the paragraph runs; Auto points both ways.
fn direction_glyph(p: &egui::Painter, r: egui::Rect, d: photocraft_doc::text::TextDirection, c: Color32) {
    use photocraft_doc::text::TextDirection as D;
    let g = p.layout_no_wrap("¶".to_string(), egui::FontId::proportional(11.0), c);
    p.galley(egui::pos2(r.center().x - g.size().x / 2.0, r.top() - 2.0), g, c);
    let (y, s) = (r.bottom() - 1.5, Stroke::new(1.3, c));
    p.line_segment([egui::pos2(r.left(), y), egui::pos2(r.right(), y)], s);
    let head = |x: f32, towards: f32| {
        for dy in [-3.0, 3.0] {
            p.line_segment([egui::pos2(x, y), egui::pos2(x - towards * 3.0, y + dy)], s);
        }
    };
    if d != D::Rtl {
        head(r.right(), 1.0);
    }
    if d != D::Ltr {
        head(r.left(), -1.0);
    }
}

/// The options bar's 24 px square button for one paragraph direction, styled like its icon buttons.
fn direction_button(ui: &mut egui::Ui, d: photocraft_doc::text::TextDirection, selected: bool, tip: &str) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::click());
    let tint = crate::icons::button_chrome(ui, r, selected, resp.hovered());
    direction_glyph(ui.painter(), egui::Rect::from_center_size(r.center(), egui::vec2(14.0, 16.0)), d, tint);
    let tip = tip.to_string();
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, &tip));
    resp.on_hover_text(tip)
}

/// A toggle button (faux styles, paragraph alignment) of width `w`, painted by `paint`.
fn toggle_cell(ui: &mut egui::Ui, w: f32, on: bool, tip: &str, paint: impl FnOnce(&egui::Ui, egui::Rect, Color32)) -> bool {
    let t = crate::theme::Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(egui::vec2(w, 24.0), egui::Sense::click());
    let bg = if on {
        t.accent_soft
    } else if resp.hovered() {
        t.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(r, t.radius_sm, bg);
    if on {
        ui.painter().rect_stroke(r, t.radius_sm, Stroke::new(1.0, t.accent_border), egui::StrokeKind::Inside);
    }
    let enabled = ui.is_enabled();
    paint(
        ui,
        r,
        if !enabled {
            t.text_faint
        } else if on {
            t.text
        } else {
            t.text_dim
        },
    );
    let tip = tip.to_string();
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, enabled, on, &tip));
    resp.on_hover_text(tl!(&tip)).clicked()
}

/// The Character panel's OpenType buttons, in Photoshop's order: (feature tag, command id,
/// tooltip). The tooltips are the Type › OpenType menu labels.
pub(crate) const OPENTYPE_BUTTONS: [(&str, &str, &str); 8] = [
    ("liga", "type.openType.standardLigatures", "Standard Ligatures"),
    ("calt", "type.openType.contextualAlternates", "Contextual Alternates"),
    ("dlig", "type.openType.discretionaryLigatures", "Discretionary Ligatures"),
    ("swsh", "type.openType.swash", "Swash"),
    ("salt", "type.openType.stylisticAlternates", "Stylistic Alternates"),
    ("titl", "type.openType.titlingAlternates", "Titling Alternates"),
    ("ordn", "type.openType.ordinals", "Ordinals"),
    ("frac", "type.openType.fractions", "Fractions"),
];

/// Photoshop's OpenType button row: each shows pressed while its feature is on at the type
/// target (the selection start, else the layer) and toggles it through the Type › OpenType
/// command, on the same scope as the other Character controls.
fn opentype_row(app: &mut PhotocraftApp, ui: &mut egui::Ui, c: &photocraft_doc::text::CharStyle, full: f32) {
    let cw = cell_width(full, OPENTYPE_BUTTONS.len());
    let mut clicked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (tag, id, tip) in OPENTYPE_BUTTONS {
            let on = photocraft_engine::type_extra_cmds::feature_on(c, tag);
            if toggle_cell(ui, cw, on, tip, |ui, r, col| opentype_glyph(ui.painter(), r, tag, col)) {
                clicked = Some((id, on));
            }
        }
    });
    if let Some((id, on)) = clicked {
        toggle_opentype(app, id, on);
    }
}

/// Runs an OpenType toggle command on the Character panel's scope: the edited layer's selected
/// characters (or its whole text) while typing, else every selected type layer. The new state
/// is explicit, so it follows the button the user saw rather than the layer's first run.
pub(crate) fn toggle_opentype(app: &mut PhotocraftApp, id: &str, shown_on: bool) {
    let Some(mut p) = formatting_params(app) else {
        if let Some((tag, _, _)) = OPENTYPE_BUTTONS.iter().find(|(_, command, _)| *command == id) {
            let props = match *tag {
                "liga" => json!({"ligatures": !shown_on}),
                "dlig" => json!({"discretionaryLigatures": !shown_on}),
                _ => json!({"features": { (*tag): !shown_on }}),
            };
            apply_defaults(app, &props);
        }
        return;
    };
    p["on"] = json!(!shown_on);
    let _ = app.run(id, p);
}

/// Small glyph labels for the OpenType buttons (drawn here, no Adobe artwork): fi, st, ct with
/// a ligature arc, a swash A, aa, T, 1st and ½.
fn opentype_glyph(p: &egui::Painter, r: egui::Rect, tag: &str, col: Color32) {
    let font = egui::FontId::proportional(13.0);
    let centred = |s: &str, font: egui::FontId| {
        let g = p.layout_no_wrap(s.to_string(), font, col);
        let pos = r.center() - g.size() / 2.0;
        let gr = egui::Rect::from_min_size(pos, g.size());
        p.galley(pos, g, col);
        gr
    };
    match tag {
        "liga" => {
            centred("fi", font);
        }
        "calt" => {
            centred("st", font);
        }
        "dlig" => {
            let gr = centred("ct", font);
            // The ligature's connecting arc from the c over to the t.
            let a = egui::pos2(gr.left() + gr.width() * 0.3, gr.top() + gr.height() * 0.42);
            let b = egui::pos2(gr.right() - gr.width() * 0.2, gr.top() + 4.0);
            let mid = egui::pos2((a.x + b.x) / 2.0, gr.top() + 1.5);
            p.line_segment([a, mid], Stroke::new(1.0, col));
            p.line_segment([mid, b], Stroke::new(1.0, col));
        }
        "swsh" => {
            let gr = centred("A", crate::theme::semibold(13.0));
            // A swash tail sweeping out under the letter.
            let y = gr.bottom() - 3.0;
            p.line_segment([egui::pos2(gr.left() - 3.0, y + 1.5), egui::pos2(gr.right() + 2.0, y - 0.5)], Stroke::new(1.0, col));
        }
        "salt" => {
            centred("aa", font);
        }
        "titl" => {
            centred("T", crate::theme::semibold(13.0));
        }
        "ordn" => {
            let one = p.layout_no_wrap("1".into(), font, col);
            let st = p.layout_no_wrap("st".into(), egui::FontId::proportional(9.0), col);
            let left = r.center().x - (one.size().x + st.size().x) / 2.0;
            let top = r.center().y - one.size().y / 2.0;
            let st_x = left + one.size().x;
            p.galley(egui::pos2(left, top), one, col);
            p.galley(egui::pos2(st_x, top - 1.0), st, col);
        }
        _ => {
            centred("½", font);
        }
    }
}

/// Width of each of `n` equal cells filling `avail` with 2 pt gaps.
fn cell_width(avail: f32, n: usize) -> f32 {
    let n = n.max(1) as f32;
    ((avail - (n - 1.0) * 2.0) / n).floor().max(20.0)
}

/// Properties panel sections for a type layer: Character, Paragraph and Type Options (#155).
pub fn type_properties(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    type_sections(app, ui, true, true, false);
    type_options(app, ui);
}

/// Window › Character / Paragraph (#150): the same controls as Properties, in their own dock
/// group. Without a type target they edit the defaults for the next Type-tool layer.
pub fn character_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui, paragraph: bool) {
    type_sections(app, ui, !paragraph, paragraph, true);
}

/// The Character and/or Paragraph sections, with the shared collapsible headers and fields that
/// fill the panel width (#155).
fn type_sections(app: &mut PhotocraftApp, ui: &mut egui::Ui, character: bool, paragraph: bool, show_defaults: bool) {
    use crate::props_layout::{COL_GAP, LABEL_GAP, LABEL_W, field_width, section};
    use crate::theme::ROW_GAP;
    let Some((c, para)) = styles_at(app).or_else(|| show_defaults.then(|| default_styles(app))) else { return };
    // Preferences ▸ Units & Rulers ▸ Type, as in the options bar: the model keeps points.
    let (u, unit) = type_unit(app);
    let t = crate::theme::Tokens::get(ui.ctx());
    // Row helper: `ui.horizontal` with the column gap, then the shared row gap.
    let row = |ui: &mut egui::Ui, add: &mut dyn FnMut(&mut egui::Ui)| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = COL_GAP;
            add(ui)
        });
        ui.add_space(ROW_GAP);
    };
    if character && section(ui, "character", tl!("Character")) {
        let full = ui.available_width();
        let w = field_width(full, 2, LABEL_W);
        let mut fam = c.font_family.clone();
        let mut ready = true;
        row(ui, &mut |ui| {
            ready = fonts_ready(ui, full);
            if ready && font_picker(ui, &mut fam, full, app.session.prefs().type_.font_preview, &app.ui.recent_fonts) {
                app.note_recent_font(&fam);
                app.ui.tool_options.type_font = fam.clone();
                apply(app, ui.ctx(), json!({"font": fam}));
            }
        });
        row(ui, &mut |ui| {
            if !ready {
                return;
            }
            let mut style = selected_style(&c);
            let opts: Vec<(String, String)> = styles(&fam).into_iter().map(|s| (s.clone(), style_label(&s))).collect();
            let opts_ref: Vec<(String, &str)> = opts.iter().map(|(a, b)| (a.clone(), b.as_str())).collect();
            if crate::widgets::dropdown(ui, "props-type-style", tl!("Style"), &mut style, &opts_ref, full) {
                apply(app, ui.ctx(), json!({"fontStyle": style}));
            }
        });
        axis_sliders(app, ui, &c);
        row(ui, &mut |ui| {
            let k = shown_scale(app);
            // Hover tips stay English until wired through `tl!` in a multi-locale change
            // (`every_tl_literal_is_translated` requires every complete catalog).
            if let Some(v) = num_field(ui, "tT", "Font size", c.size_pt * k * u, 0.1 * u..=1296.0 * u, unit, w) {
                app.ui.tool_options.type_size = v / u;
                apply(app, ui.ctx(), json!({"size": v / u, "metricsAsShown": true}));
            }
            let lead = c.leading_pt.unwrap_or(c.size_pt * para.auto_leading.max(0.01)) * k;
            if let Some(v) = num_field(ui, "A↕", "Leading (set to the font size × auto-leading when Auto)", lead * u, 0.1 * u..=5000.0 * u, unit, w) {
                apply(app, ui.ctx(), json!({"leading": v / u, "metricsAsShown": true}));
            }
        });
        row(ui, &mut |ui| {
            let shown = kerning_label(kerning_at(app).unwrap_or((c.kerning, c.kern)));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = LABEL_GAP;
                icon_label(ui, "text-cursor", "Kerning: Metrics, Optical or a value in 1/1000 em (Alt+←/→ at the caret)");
                if let Some(v) = kerning_field(ui, &shown, w) {
                    apply_kerning(app, ui.ctx(), v);
                }
            });
            if let Some(v) = num_field(ui, "VA", "Tracking (1/1000 em)", c.tracking, -1000.0..=10000.0, "", w) {
                apply(app, ui.ctx(), json!({"tracking": v}));
            }
        });
        row(ui, &mut |ui| {
            if let Some(v) = num_field(ui, "↕T", "Vertical scale", c.vertical_scale * 100.0, 0.0..=1000.0, "%", w) {
                apply(app, ui.ctx(), json!({"verticalScale": v}));
            }
            if let Some(v) = num_field(ui, "↔T", "Horizontal scale", c.horizontal_scale * 100.0, 0.0..=1000.0, "%", w) {
                apply(app, ui.ctx(), json!({"horizontalScale": v}));
            }
        });
        row(ui, &mut |ui| {
            if let Some(v) = num_field(ui, "Aª", "Baseline shift", c.baseline_shift_pt * u, -1296.0 * u..=1296.0 * u, unit, w) {
                apply(app, ui.ctx(), json!({"baselineShift": v / u}));
            }
            // Colour: label + a swatch filling the rest of the second column.
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = LABEL_GAP;
                let g = ui.painter().layout_no_wrap(tl!("Color").into(), egui::FontId::proportional(12.0), t.text_dim);
                let lw = g.size().x.max(LABEL_W);
                let (lr, _) = ui.allocate_exact_size(egui::vec2(lw, 22.0), egui::Sense::hover());
                ui.painter().galley(egui::pos2(lr.left(), lr.center().y - g.size().y / 2.0), g, t.text_dim);
                let rgb = c.color.to_rgb();
                let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                let sw = (LABEL_W + LABEL_GAP + w - lw - LABEL_GAP).max(24.0);
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(sw, 22.0), egui::Sense::click());
                ui.painter().rect_filled(rect, t.radius_sm, Color32::from_rgb(q(rgb[0]), q(rgb[1]), q(rgb[2])));
                ui.painter().rect_stroke(rect, t.radius_sm, Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
                let resp = resp.on_hover_text(tl!("Text color"));
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Text color")));
                if resp.clicked() {
                    open_color_picker(app, rgb);
                }
            });
        });
        // Faux styles: T (bold)  T (italic)  TT  Tᴛ  T̲  T̶, sharing the row evenly.
        let caps = c.caps;
        // Tip strings are English keys; `toggle_cell` runs them through `tl!`.
        let toggles: [(&str, &str, bool, serde_json::Value); 6] = [
            ("T", "Faux Bold", c.faux_bold, json!({"fauxBold": !c.faux_bold})),
            ("T", "Faux Italic", c.faux_italic, json!({"fauxItalic": !c.faux_italic})),
            (
                "TT",
                "All Caps",
                caps == photocraft_doc::text::Caps::AllCaps,
                json!({"caps": if caps == photocraft_doc::text::Caps::AllCaps { "normal" } else { "allCaps" }}),
            ),
            (
                "Tᴛ",
                "Small Caps",
                caps == photocraft_doc::text::Caps::SmallCaps,
                json!({"caps": if caps == photocraft_doc::text::Caps::SmallCaps { "normal" } else { "smallCaps" }}),
            ),
            ("T", "Underline", c.underline, json!({"underline": !c.underline})),
            ("T", "Strikethrough", c.strikethrough, json!({"strikethrough": !c.strikethrough})),
        ];
        let cw = cell_width(full, toggles.len());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (i, (glyph, tip, on, props)) in toggles.into_iter().enumerate() {
                let clicked = toggle_cell(ui, cw, on, tip, |ui, r, col| {
                    let font = if i == 0 { crate::theme::semibold(13.0) } else { egui::FontId::proportional(13.0) };
                    let g = ui.painter().layout_no_wrap(glyph.to_string(), font, col);
                    let pos = r.center() - g.size() / 2.0;
                    let gr = egui::Rect::from_min_size(pos, g.size());
                    if i == 1 {
                        // Faux italic: a slanted T drawn as strokes (no italic face is bundled).
                        let (top, bot, cx) = (gr.top() + 3.0, gr.bottom() - 3.0, gr.center().x);
                        let slant = (bot - top) * 0.25;
                        ui.painter().line_segment([egui::pos2(cx - 4.0 + slant / 2.0, top), egui::pos2(cx + 4.0 + slant / 2.0, top)], Stroke::new(1.3, col));
                        ui.painter().line_segment([egui::pos2(cx + slant / 2.0, top), egui::pos2(cx - slant / 2.0, bot)], Stroke::new(1.3, col));
                    } else {
                        ui.painter().galley(pos, g, col);
                    }
                    if i == 4 {
                        ui.painter().line_segment([egui::pos2(gr.left(), gr.bottom() - 2.0), egui::pos2(gr.right(), gr.bottom() - 2.0)], Stroke::new(1.0, col));
                    }
                    if i == 5 {
                        ui.painter()
                            .line_segment([egui::pos2(gr.left() - 1.0, gr.center().y), egui::pos2(gr.right() + 1.0, gr.center().y)], Stroke::new(1.0, col));
                    }
                });
                if clicked {
                    apply(app, ui.ctx(), props);
                }
            }
        });
        ui.add_space(ROW_GAP);
        opentype_row(app, ui, &c, full);
        ui.add_space(ROW_GAP);
    }
    if paragraph && section(ui, "paragraph", tl!("Paragraph")) {
        use photocraft_doc::text::TextAlign as A;
        let full = ui.available_width();
        let w = field_width(full, 2, LABEL_W);
        let items = [
            ("left", tl!("Left align text"), A::Left),
            ("center", tl!("Center text"), A::Center),
            ("right", tl!("Right align text"), A::Right),
            ("justifyLeft", tl!("Justify last left"), A::JustifyLeft),
            ("justifyCenter", tl!("Justify last centered"), A::JustifyCenter),
            ("justifyRight", tl!("Justify last right"), A::JustifyRight),
            ("justifyAll", tl!("Justify all"), A::JustifyAll),
        ];
        let cw = cell_width(full, items.len());
        // Justification needs a paragraph box or area type: on point text Photoshop greys the four
        // justify buttons (#7713).
        // The panel formats the layer being typed in, else every selected type layer.
        let point = match formatting_params(app) {
            Some(p) => p.get("layers").and_then(serde_json::Value::as_array).map_or_else(
                || p.get("layer").and_then(serde_json::Value::as_u64).into_iter().collect::<Vec<_>>(),
                |ids| ids.iter().filter_map(serde_json::Value::as_u64).collect(),
            ),
            None => Vec::new(),
        }
        .into_iter()
        .any(|layer| {
            let shape = app.session.active().and_then(|s| text_layer(&s.doc, LayerId(layer)).map(|t| t.shape.clone()));
            !matches!(shape, Some(photocraft_doc::text::TextShape::Box { .. } | photocraft_doc::text::TextShape::Area { .. }))
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (key, tip, a) in items {
                let justify = !matches!(a, A::Left | A::Center | A::Right);
                let clicked = ui
                    .add_enabled_ui(!(point && justify), |ui| {
                        toggle_cell(ui, cw, para.align == a, tip, |ui, r, col| {
                            // The glyph keeps its size; the cell grows around it.
                            let g = egui::Rect::from_center_size(r.center(), egui::vec2(12.0, 10.0));
                            let col = if !ui.is_enabled() || para.align == a { col } else { t.icon };
                            align_glyph(ui.painter(), g, a, col);
                        })
                    })
                    .inner;
                if clicked {
                    apply(app, ui.ctx(), json!({"align": key}));
                }
            }
        });
        ui.add_space(ROW_GAP);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (key, tip, d) in direction_items() {
                let clicked = toggle_cell(ui, cw, para.direction == d, tip, |ui, r, col| {
                    let g = egui::Rect::from_center_size(r.center(), egui::vec2(14.0, 16.0));
                    direction_glyph(ui.painter(), g, d, col);
                });
                if clicked {
                    apply(app, ui.ctx(), json!({"direction": key}));
                }
            }
        });
        ui.add_space(ROW_GAP);
        row(ui, &mut |ui| {
            if let Some(v) = num_field(ui, "→|", "Indent left margin", para.start_indent_pt * u, -1296.0 * u..=1296.0 * u, unit, w) {
                apply(app, ui.ctx(), json!({"startIndent": v / u}));
            }
            if let Some(v) = num_field(ui, "|←", "Indent right margin", para.end_indent_pt * u, -1296.0 * u..=1296.0 * u, unit, w) {
                apply(app, ui.ctx(), json!({"endIndent": v / u}));
            }
        });
        row(ui, &mut |ui| {
            if let Some(v) = num_field(ui, "¶→", "Indent first line", para.first_line_indent_pt * u, -1296.0 * u..=1296.0 * u, unit, w) {
                apply(app, ui.ctx(), json!({"firstLineIndent": v / u}));
            }
        });
        row(ui, &mut |ui| {
            if let Some(v) = num_field(ui, "↑¶", "Add space before paragraph", para.space_before_pt * u, 0.0 * u..=1296.0 * u, unit, w) {
                apply(app, ui.ctx(), json!({"spaceBefore": v / u}));
            }
            if let Some(v) = num_field(ui, "¶↓", "Add space after paragraph", para.space_after_pt * u, 0.0 * u..=1296.0 * u, unit, w) {
                apply(app, ui.ctx(), json!({"spaceAfter": v / u}));
            }
        });
        // Hyphenate is stored (⌥⇧⌘H / PSD AutoHyphenate) but layout never breaks words (#6710).
        ui.add_space(ROW_GAP);
    }
}

/// Type Options: anti-aliasing and orientation of the whole layer (the Type menu's commands).
fn type_options(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    use crate::props_layout::{COL_GAP, LABEL_GAP};
    let Some((layer, _)) = target(app) else { return };
    let Some((aa, vertical)) = app
        .session
        .active()
        .and_then(|st| text_layer(&st.doc, LayerId(layer)))
        .map(|t| (photocraft_engine::type_extra_cmds::aa_name(t.antialias).to_string(), t.orientation == photocraft_doc::text::Orientation::Vertical))
    else {
        return;
    };
    if !crate::props_layout::section(ui, "typeOptions", "Type Options") {
        return;
    }
    let full = ui.available_width();
    let mut run: Option<String> = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = COL_GAP;
        // Anti-aliasing dropdown (aₐ glyph label), then the two orientation toggles.
        let orient_w = 2.0 * 28.0 + 2.0;
        let dd_w = (full - crate::props_layout::LABEL_W - LABEL_GAP - COL_GAP - orient_w).max(60.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = LABEL_GAP;
            let t = crate::theme::Tokens::get(ui.ctx());
            let (r, resp) = ui.allocate_exact_size(egui::vec2(crate::props_layout::LABEL_W, 22.0), egui::Sense::hover());
            ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, "aa", crate::theme::semibold(11.0), t.text_dim);
            resp.on_hover_text(tl!("Anti-aliasing method"));
            let mut cur = aa.clone();
            let opts: Vec<(String, &str)> = ["none", "sharp", "crisp", "strong", "smooth", "windowsLcd", "windows"]
                .into_iter()
                .filter_map(|k| photocraft_engine::commands::find(&format!("type.antiAlias.{k}")).map(|c| (k.to_string(), c.label)))
                .collect();
            if crate::widgets::dropdown(ui, "props-type-aa", tl!("Anti-aliasing method"), &mut cur, &opts, dd_w) {
                run = Some(format!("type.antiAlias.{cur}"));
            }
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (key, tip, on) in [("horizontal", "Horizontal type", !vertical), ("vertical", "Vertical type", vertical)] {
                let clicked = toggle_cell(ui, 28.0, on, tip, |ui, r, col| {
                    let g = egui::Rect::from_center_size(r.center(), egui::vec2(12.0, 12.0));
                    let s = Stroke::new(1.3, col);
                    if key == "horizontal" {
                        ui.painter().line_segment([g.left_top(), g.right_top()], s);
                        ui.painter().line_segment([g.center_top(), g.center_bottom()], s);
                    } else {
                        ui.painter().line_segment([g.left_top(), g.left_bottom()], s);
                        ui.painter().line_segment([g.left_center(), g.right_center()], s);
                    }
                });
                if clicked && !on {
                    run = Some(format!("type.orientation.{key}"));
                }
            }
        });
    });
    ui.add_space(crate::theme::ROW_GAP);
    if let Some(id) = run
        && let Some(params) = formatting_params(app)
        && let Err(e) = app.run(&id, params)
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// Cancel the editing session: undo it back to where it started (removes a new layer).
pub fn cancel(app: &mut PhotocraftApp) {
    crate::type_transform::reset(app);
    let Some(ed) = app.ui.text_edit.take() else { return };
    app.type_ime.clear_preedit();
    if let Some(tool) = app.ui.text_edit_return_tool.take()
        && app.ui.tool.is_type()
    {
        app.ui.tool = tool;
    }
    undo_session(app, &ed);
}

/// Undo the session's history step, if it is still the latest.
fn undo_session(app: &mut PhotocraftApp, ed: &TextEdit) {
    let coalesced = app.session.active().is_some_and(|s| s.coalesce.as_deref() == Some(ed.session.as_str()));
    if coalesced {
        let _ = app.run("edit.undo", json!({}));
    }
}

#[cfg(test)]
#[path = "type_tool_tests.rs"]
mod canvas_tests;

#[cfg(test)]
#[path = "type_font_picker_tests.rs"]
mod font_picker_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 400, "height": 200})).unwrap();
        app.sync_views();
        app.ui.tool = crate::state::Tool::Type;
        app
    }

    /// Preferences ▸ Type ▸ Recent Fonts caps the recorded list; a re-pick moves to the front and
    /// 0 keeps no list.
    #[test]
    fn note_recent_font_caps_and_de_duplicates() {
        let mut app = app();
        for f in ["A", "B", "C"] {
            app.note_recent_font(f);
        }
        assert_eq!(app.ui.recent_fonts, ["C", "B", "A"]);
        app.run("prefs.set", json!({"path": "type.recentFonts", "value": 2})).unwrap();
        app.note_recent_font("D");
        assert_eq!(app.ui.recent_fonts, ["D", "C"], "the cap trims the tail");
        app.note_recent_font("C");
        assert_eq!(app.ui.recent_fonts, ["C", "D"], "a re-pick moves to the front");
        app.run("prefs.set", json!({"path": "type.recentFonts", "value": 0})).unwrap();
        app.note_recent_font("E");
        assert!(app.ui.recent_fonts.is_empty(), "0 keeps no list");
    }

    /// A smaller Recent Fonts cap (or 0) applies to the existing list at once, without waiting
    /// for the next pick (CodeRabbit review of #3559).
    #[test]
    fn lowering_the_recent_fonts_cap_applies_at_once() {
        let mut app = app();
        for f in ["A", "B", "C"] {
            app.note_recent_font(f);
        }
        app.run("prefs.set", json!({"path": "type.recentFonts", "value": 2})).unwrap();
        app.sync_recent_fonts();
        assert_eq!(app.ui.recent_fonts, ["C", "B"], "the smaller cap trims the tail without a pick");
        app.run("prefs.set", json!({"path": "type.recentFonts", "value": 0})).unwrap();
        app.sync_recent_fonts();
        assert!(app.ui.recent_fonts.is_empty(), "0 clears the list");
    }

    fn layer_text(app: &PhotocraftApp) -> String {
        let ed = app.ui.text_edit.as_ref().unwrap();
        current_text(app, LayerId(ed.layer)).unwrap()
    }

    /// A committed type layer reading `s` at the same spot as the others (stacked variants, #4320).
    fn stacked_variant(app: &mut PhotocraftApp, s: &str) -> LayerId {
        app.ui.text_edit = None;
        pointer_up(app, [50.0, 100.0], [50.0, 100.0]);
        insert(app, s);
        commit(app);
        app.session.active().unwrap().active_layer.unwrap()
    }

    fn hide(app: &mut PhotocraftApp, id: LayerId) {
        app.run("layer.setProps", json!({"layer": id.0, "visible": false})).unwrap();
    }

    /// Wraps `id` in a new group and returns the group.
    fn group_of(app: &mut PhotocraftApp, id: LayerId) -> LayerId {
        app.run("layer.select", json!({"layer": id.0, "mode": "replace"})).unwrap();
        app.run("layer.groupLayers", json!({})).unwrap();
        let doc = &app.session.active().unwrap().doc;
        let path = doc.path_of(id).unwrap();
        doc.layer_at(&path[..path.len() - 1]).unwrap().id
    }

    /// Three type variants at one spot, bottom to top: A, B, C.
    fn variants() -> (PhotocraftApp, [LayerId; 3]) {
        let mut app = app();
        let ids = ["Variant A", "Variant B", "Variant C"].map(|s| stacked_variant(&mut app, s));
        (app, ids)
    }

    const ON_TEXT: (f64, f64) = (60.0, 95.0);

    /// #4320: variants hidden through their groups above the shown one are not picked.
    #[test]
    fn type_click_skips_text_in_hidden_groups_above_the_shown_variant() {
        let (mut app, [a, b, c]) = variants();
        for id in [b, c] {
            let g = group_of(&mut app, id);
            hide(&mut app, g);
        }
        assert_eq!(hit_layer(&mut app, ON_TEXT.0, ON_TEXT.1), Some(a));
        assert!(pointer_down(&mut app, ON_TEXT.0, ON_TEXT.1, egui::Modifiers::NONE));
        assert_eq!(app.ui.text_edit.as_ref().map(|e| e.layer), Some(a.0), "the click edits the shown variant");
    }

    #[test]
    fn type_click_skips_text_in_a_nested_hidden_group() {
        let (mut app, [a, b, c]) = variants();
        hide(&mut app, b);
        let inner = group_of(&mut app, c);
        let outer = group_of(&mut app, inner);
        hide(&mut app, outer);
        assert!(app.session.active().unwrap().doc.layer(c).unwrap().visible, "only the outer group is hidden");
        assert_eq!(hit_layer(&mut app, ON_TEXT.0, ON_TEXT.1), Some(a));
    }

    #[test]
    fn type_click_skips_own_hidden_variants_above_and_below() {
        let (mut app, [a, b, c]) = variants();
        hide(&mut app, b);
        hide(&mut app, c);
        assert_eq!(hit_layer(&mut app, ON_TEXT.0, ON_TEXT.1), Some(a), "hidden variants above");
        let (mut app, [a, b, c]) = variants();
        hide(&mut app, a);
        hide(&mut app, b);
        assert_eq!(hit_layer(&mut app, ON_TEXT.0, ON_TEXT.1), Some(c), "hidden variants below");
    }

    /// The Move tool's double-click on type (#1132) shares the pick.
    #[test]
    fn move_double_click_edits_the_shown_variant_not_one_in_a_hidden_group() {
        let (mut app, [a, b, c]) = variants();
        for id in [b, c] {
            let g = group_of(&mut app, id);
            hide(&mut app, g);
        }
        app.ui.tool = crate::state::Tool::Move;
        assert!(edit_at(&mut app, ON_TEXT.0, ON_TEXT.1));
        assert_eq!(app.ui.text_edit.as_ref().map(|e| e.layer), Some(a.0));
    }

    /// Photoshop: a click on existing type edits it; a ⇧-click there starts a new type layer.
    #[test]
    fn shift_click_on_existing_type_starts_new_text() {
        let mut app = app();
        pointer_up(&mut app, [50.0, 100.0], [50.0, 100.0]);
        insert(&mut app, "Existing words");
        commit(&mut app);
        let existing = app.session.active().unwrap().doc.layers.last().unwrap().id;
        let (x, y) = (60.0, 95.0);
        assert_eq!(hit_layer(&mut app, x, y), Some(existing), "the click lands on the existing text");
        assert!(!pointer_down(&mut app, x, y, egui::Modifiers::SHIFT), "⇧: not consumed, so a new type layer starts");
        assert!(app.ui.text_edit.is_none());
        assert!(pointer_down(&mut app, x, y, egui::Modifiers::NONE), "a plain click edits the existing text");
        assert_eq!(app.ui.text_edit.as_ref().map(|e| e.layer), Some(existing.0));
    }

    #[test]
    fn click_creates_placeholder_selected_and_typing_replaces_it() {
        let mut app = app();
        assert!(!pointer_down(&mut app, 50.0, 100.0, egui::Modifiers::NONE));
        pointer_up(&mut app, [50.0, 100.0], [50.0, 100.0]);
        let ed = app.ui.text_edit.clone().unwrap();
        assert_eq!((ed.anchor, ed.caret), (0, PLACEHOLDER.chars().count()));
        insert(&mut app, "Héllo");
        insert(&mut app, " world");
        assert_eq!(layer_text(&app), "Héllo world");
        // Creating and typing share the session key: one step after the initial snapshot.
        let steps = app.session.active().unwrap().history.entries();
        assert_eq!(steps, ["Open", "New Type Layer"]);
        commit(&mut app);
        let doc = &app.session.active().unwrap().doc;
        assert_eq!(doc.layers.last().unwrap().name, "Héllo world");
        assert!(app.ui.text_edit.is_none());
    }

    /// TYPE-218-1: a Type tool click on the work path starts type on the path from the point
    /// clicked, in one history step with the placeholder selected; a click off it is point text.
    #[test]
    fn click_on_a_path_starts_type_on_the_path() {
        use photocraft_doc::text::TextShape;
        let mut app = app();
        app.session
            .edit("path", |doc, _| {
                doc.work_path = Some(photocraft_doc::vector::Path::new(vec![photocraft_doc::vector::Subpath::polyline(&[(20.0, 100.0), (380.0, 100.0)])]));
                Ok(())
            })
            .unwrap();
        assert!(matches!(path_target(&app, 200.0, 101.0), Some(PathClick::OnPath { .. })));
        assert!(path_target(&app, 200.0, 160.0).is_none(), "off the path");
        let before = app.session.active().unwrap().history.entries().len();
        assert!(!pointer_down(&mut app, 200.0, 101.0, egui::Modifiers::NONE));
        pointer_up(&mut app, [200.0, 101.0], [200.0, 101.0]);
        let ed = app.ui.text_edit.clone().unwrap();
        assert_eq!((ed.anchor, ed.caret), (0, PLACEHOLDER.chars().count()), "the placeholder is selected");
        let shape = text_layer(&app.session.active().unwrap().doc, LayerId(ed.layer)).unwrap().shape.clone();
        assert!(matches!(shape, TextShape::Path { start, flip: false, .. } if (start - 180.0).abs() < 0.5), "{shape:?}");
        assert_eq!(app.session.active().unwrap().history.entries().len(), before + 1, "one step for the click");
        insert(&mut app, "On the line");
        assert_eq!(layer_text(&app), "On the line");
        commit(&mut app);
        // With the vertical tool a click does not start path type.
        app.ui.tool = crate::state::Tool::VerticalType;
        assert!(!matches!(path_target(&app, 200.0, 101.0), Some(PathClick::OnPath { .. })));
        app.ui.tool = crate::state::Tool::Type;
        pointer_up(&mut app, [200.0, 160.0], [200.0, 160.0]);
        let id = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
        assert_eq!(text_layer(&app.session.active().unwrap().doc, id).unwrap().shape, TextShape::Point);
    }

    /// A click in type on a path puts the caret at the glyph there along the curve, not where the
    /// straight line would have it.
    #[test]
    fn caret_hits_type_on_a_path_along_the_curve() {
        let mut app = app();
        app.session
            .edit("path", |doc, _| {
                doc.work_path = Some(photocraft_doc::vector::Path::new(vec![photocraft_doc::vector::Subpath::polyline(&[(300.0, 10.0), (300.0, 195.0)])]));
                Ok(())
            })
            .unwrap();
        pointer_up(&mut app, [300.0, 20.0], [300.0, 20.0]);
        insert(&mut app, "ABCDEFGH");
        let id = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
        assert!(matches!(text_layer(&app.session.active().unwrap().doc, id).unwrap().shape, photocraft_doc::text::TextShape::Path { .. }));
        // The type runs down the path: near its start the caret is at the beginning, far down at the end.
        assert_eq!(offset_at(&mut app, id, 300.0, 18.0), 0);
        // (The straight line would put both of these at the start: they are left of its first glyph.)
        let (mid, far) = (offset_at(&mut app, id, 296.0, 60.0), offset_at(&mut app, id, 296.0, 190.0));
        assert!(0 < mid && mid < far, "{mid} {far}");
    }

    /// Preferences ▸ Units & Rulers ▸ Type switches the display unit of type metrics: points by
    /// default, pixels by the document's resolution, or millimetres.
    #[test]
    fn type_unit_follows_the_units_and_rulers_preference() {
        let mut app = app();
        assert_eq!(type_unit(&app), (1.0, "pt"));
        app.run("prefs.set", json!({"path": "unitsAndRulers.typeUnits", "value": "pixels"})).unwrap();
        assert_eq!(type_unit(&app), (1.0, "px"), "a 72 ppi document: 1 pt is 1 px");
        app.run("image.imageSize", json!({"resolution": 144, "resample": "none"})).unwrap();
        assert_eq!(type_unit(&app), (2.0, "px"), "144 ppi: a point is two pixels");
        app.run("prefs.set", json!({"path": "unitsAndRulers.typeUnits", "value": "mm"})).unwrap();
        assert!((type_unit(&app).0 - 25.4 / 72.0).abs() < 1e-6, "millimetres ignore the resolution");
    }

    /// Preferences ▸ Type ▸ Fill new type layers with placeholder text: off, a click creates an
    /// empty layer (nothing selected) and committing without typing removes it again.
    #[test]
    fn placeholder_preference_controls_new_type_layer_text() {
        let mut app = app();
        app.run("prefs.set", json!({"path": "type.fillNewTypeLayersWithPlaceholder", "value": false})).unwrap();
        pointer_up(&mut app, [50.0, 100.0], [50.0, 100.0]);
        let ed = app.ui.text_edit.clone().unwrap();
        assert_eq!((ed.anchor, ed.caret), (0, 0), "nothing to select");
        assert_eq!(layer_text(&app), "");
        assert_eq!(app.session.active().unwrap().doc.layers.len(), 2, "the layer was created");
        commit(&mut app);
        assert_eq!(app.session.active().unwrap().doc.layers.len(), 1, "an empty type layer is removed on commit");
        assert!(app.ui.text_edit.is_none());
    }

    /// Pure smart-quote mapping: opening after whitespace, a bracket or a dash, closing next to a
    /// letter or another quote.
    #[test]
    fn smart_quotes_are_typographic() {
        assert_eq!(smart_quotes("\"Hello\"", None), "\u{201c}Hello\u{201d}");
        assert_eq!(smart_quotes("don't", None), "don\u{2019}t");
        assert_eq!(smart_quotes("(\"x\")", None), "(\u{201c}x\u{201d})");
        assert_eq!(smart_quotes("--'x'", None), "--\u{2018}x\u{2019}", "a dash stays a dash, the quote opens after it");
        assert_eq!(smart_quotes("\"\"", None), "\u{201c}\u{201d}");
    }

    /// Preferences ▸ Type ▸ Use Smart Quotes switches typed quotes (typed text only, not paste).
    #[test]
    fn smart_quotes_preference_controls_typed_quotes() {
        for (smart, expected) in [(true, "hi \u{201c}you\u{201d}"), (false, "hi \"you\"")] {
            let mut app = app();
            app.run("prefs.set", json!({"path": "type.smartQuotes", "value": smart})).unwrap();
            pointer_up(&mut app, [50.0, 100.0], [50.0, 100.0]);
            insert_typed(&mut app, "hi \"you\"");
            assert_eq!(layer_text(&app), expected, "smartQuotes={smart}");
        }
    }

    #[test]
    fn a_name_from_text_after_blank_lines_keeps_following_the_text() {
        let mut app = app();
        pointer_up(&mut app, [50.0, 100.0], [50.0, 100.0]);
        insert(&mut app, "\n\nTitle");
        let id = app.ui.text_edit.as_ref().unwrap().layer;
        commit(&mut app);
        let name = |app: &PhotocraftApp| app.session.active().unwrap().doc.layers.last().unwrap().name.clone();
        assert_eq!(name(&app), "Title");
        // Edited later, outside the session that created it (#483).
        app.run("type.edit", json!({"layer": id, "text": "\nSubtitle"})).unwrap();
        assert_eq!(name(&app), "Subtitle");
    }

    #[test]
    fn dragging_a_box_handle_resizes_the_paragraph_box() {
        let mut app = app();
        pointer_up(&mut app, [10.0, 10.0], [110.0, 60.0]);
        let id = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
        let steps = app.session.active().unwrap().history.entries().len();
        // Top-left corner: the box keeps its bottom-right corner.
        assert!(pointer_down(&mut app, 10.0, 10.0, egui::Modifiers::NONE));
        assert_eq!(app.ui.text_edit.as_ref().unwrap().resize.map(|r| r.handle), Some(0));
        pointer_move(&mut app, 20.0, 25.0, egui::Modifiers::NONE);
        pointer_move(&mut app, 30.0, 30.0, egui::Modifiers::NONE);
        pointer_up(&mut app, [10.0, 10.0], [30.0, 30.0]);
        assert_eq!(box_shape(&app, id), Some((0.0, 0.0, 80.0, 30.0)));
        let aff = layout(&mut app, id).unwrap().1;
        assert_eq!((aff.m[4], aff.m[5]), (30.0, 30.0));
        // The whole drag is one history step, and the edit session survives it.
        assert_eq!(app.session.active().unwrap().history.entries().len(), steps);
        assert!(app.ui.text_edit.is_some());
        // Right edge: only the width changes, and never below the minimum.
        assert!(pointer_down(&mut app, 110.0, 45.0, egui::Modifiers::NONE));
        pointer_move(&mut app, 0.0, 99.0, egui::Modifiers::NONE);
        pointer_up(&mut app, [110.0, 45.0], [0.0, 99.0]);
        assert_eq!(box_shape(&app, id), Some((0.0, 0.0, MIN_BOX, 30.0)));
        // ⌥ mirrors the drag about the centre: the left edge follows the right one out.
        assert!(pointer_down(&mut app, 34.0, 45.0, egui::Modifiers::ALT));
        pointer_move(&mut app, 70.0, 45.0, egui::Modifiers::ALT);
        pointer_up(&mut app, [34.0, 45.0], [70.0, 45.0]);
        assert_eq!(box_shape(&app, id), Some((0.0, 0.0, MIN_BOX + 72.0, 30.0)));
        let aff = layout(&mut app, id).unwrap().1;
        // Text space −36..40 (centre 2) through the translation: the doc centre stays at 32.
        assert_eq!((aff.m[4], aff.m[5]), (-6.0, 30.0));
    }

    #[test]
    fn ime_preedit_is_replaced_by_each_update_and_commit_finalises() {
        let mut app = app();
        pointer_up(&mut app, [50.0, 100.0], [50.0, 100.0]);
        // Each preedit update replaces the previous one (and the placeholder selection first).
        ime_update(&mut app, "に", false);
        assert_eq!(layer_text(&app), "に");
        ime_update(&mut app, "にほん", false);
        ime_update(&mut app, "にほんご", false);
        assert_eq!(layer_text(&app), "にほんご");
        assert_eq!(app.ui.text_edit.as_ref().unwrap().preedit, Some((0, 4)));
        ime_update(&mut app, "日本語", true);
        assert_eq!(layer_text(&app), "日本語");
        let ed = app.ui.text_edit.clone().unwrap();
        assert_eq!((ed.caret, ed.anchor, ed.preedit), (3, 3, None));
        // The next composition starts after the committed text; cancelling removes it again.
        ime_update(&mut app, "あ", false);
        assert_eq!(layer_text(&app), "日本語あ");
        ime_update(&mut app, "", false);
        assert_eq!(layer_text(&app), "日本語");
        assert_eq!(app.ui.text_edit.as_ref().unwrap().preedit, None);
        // A stale preedit range (text shortened by undo) must not replace committed text; the
        // composition is invalidated and its pending commit is dropped.
        app.ui.text_edit.as_mut().unwrap().preedit = Some((10, 5));
        ime_update(&mut app, "x", true);
        assert_eq!(layer_text(&app), "日本語");
        assert_eq!(app.ui.text_edit.as_ref().unwrap().preedit, None);
    }

    #[test]
    fn empty_new_layer_is_deleted_on_commit() {
        let mut app = app();
        pointer_up(&mut app, [10.0, 50.0], [10.0, 50.0]);
        let n = app.session.active().unwrap().doc.layers.len();
        insert(&mut app, "");
        if let Some(e) = app.ui.text_edit.as_mut() {
            e.anchor = 0;
        }
        insert(&mut app, "");
        commit(&mut app);
        assert_eq!(app.session.active().unwrap().doc.layers.len(), n - 1);
    }

    #[test]
    fn word_boundaries() {
        assert_eq!(word_boundary("hello big world", 0, true), 5);
        assert_eq!(word_boundary("hello big world", 7, false), 6);
        assert_eq!(word_boundary("hello big world", 15, false), 10);
    }
    #[test]
    fn arrow_keys_reset_caret_blink_phase() {
        let mut app = app();
        pointer_up(&mut app, [10.0, 50.0], [10.0, 50.0]);
        insert(&mut app, "Hello");
        app.ui.caret_blink_time = 0.0;
        let ctx = egui::Context::default();

        let has_caret = |app: &mut PhotocraftApp, ctx: &egui::Context, time: f64| -> bool {
            let mut out = ctx.run_ui(egui::RawInput { time: Some(time), ..Default::default() }, |ui| {
                let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 400.0));
                let painter = ui.ctx().layer_painter(egui::LayerId::background()).with_clip_rect(rect);
                let xf = ViewXform { rect, zoom: 1.0, center: [100.0, 100.0], flip: false, rotation: 0.0, aspect: 1.0 };
                draw_overlay(app, &painter, &xf);
            });
            out.textures_delta.clear();
            out.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::LineSegment { stroke, .. } if CARET_STROKES.contains(stroke)))
        };

        // Before resetting, the caret is in the off phase at t = 10.5 s (10500 / 530 = 19, odd phase)
        assert!(!has_caret(&mut app, &ctx, 10.5), "caret should be off at t = 10.5 with phase 0.0");

        ctx.begin_pass(egui::RawInput {
            time: Some(10.5),
            events: vec![egui::Event::Key { key: egui::Key::ArrowRight, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }],
            ..Default::default()
        });
        handle_keys(&mut app, &ctx);
        assert_eq!(app.ui.caret_blink_time, 10.5);

        // draw_overlay paints the solid caret in the new phase
        assert!(has_caret(&mut app, &ctx, 10.5), "caret should be painted right after arrow key");
    }

    #[test]
    fn pointer_up_click_resets_caret_blink_phase() {
        let mut app = app();
        pointer_up(&mut app, [10.0, 50.0], [10.0, 50.0]);
        insert(&mut app, "Hello");
        app.ui.caret_blink_time = 0.0;
        app.last_frame_time = 12.3;
        pointer_up(&mut app, [10.0, 50.0], [10.0, 50.0]);
        assert_eq!(app.ui.caret_blink_time, 12.3);
    }

    #[test]
    fn double_click_from_move_tool_restores_move_tool_on_commit_and_cancel() {
        let (mut app, _) = variants();
        app.ui.tool = crate::state::Tool::Move;
        assert!(edit_at(&mut app, ON_TEXT.0, ON_TEXT.1));
        assert_eq!(app.ui.tool, crate::state::Tool::Type);
        assert_eq!(app.ui.text_edit_return_tool, Some(crate::state::Tool::Move));
        commit(&mut app);
        assert_eq!(app.ui.tool, crate::state::Tool::Move, "commit restores the Move tool");
        assert_eq!(app.ui.text_edit_return_tool, None);

        // Also test cancel
        app.ui.tool = crate::state::Tool::Move;
        assert!(edit_at(&mut app, ON_TEXT.0, ON_TEXT.1));
        assert_eq!(app.ui.tool, crate::state::Tool::Type);
        cancel(&mut app);
        assert_eq!(app.ui.tool, crate::state::Tool::Move, "cancel restores the Move tool");
        assert_eq!(app.ui.text_edit_return_tool, None);

        // Switching tools mid-edit commits without overriding the newly chosen tool
        app.ui.tool = crate::state::Tool::Move;
        assert!(edit_at(&mut app, ON_TEXT.0, ON_TEXT.1));
        assert_eq!(app.ui.tool, crate::state::Tool::Type);
        assert_eq!(app.ui.text_edit_return_tool, Some(crate::state::Tool::Move));
        app.ui.tool = crate::state::Tool::Brush;
        commit(&mut app);
        assert_eq!(app.ui.tool, crate::state::Tool::Brush, "switching tools keeps the chosen tool");
        assert_eq!(app.ui.text_edit_return_tool, None);

        // Layers-thumbnail edit stays on Type tool (matches Photoshop)
        app.ui.tool = crate::state::Tool::Move;
        edit_active(&mut app).unwrap();
        assert_eq!(app.ui.tool, crate::state::Tool::Type);
        assert_eq!(app.ui.text_edit_return_tool, None);
        commit(&mut app);
        assert_eq!(app.ui.tool, crate::state::Tool::Type, "Layers-thumbnail edit stays on Type tool");
    }
}

#[cfg(test)]
#[path = "type_commit_tests.rs"]
mod commit_tests;

#[cfg(test)]
mod ime_tests {
    use super::*;
    use egui::output::{IMEOutput, ImeChangeCause};

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        install_reset_hooks(&ctx);
        ctx
    }

    fn app() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 400, "height": 200})).unwrap();
        app.sync_views();
        app.ui.tool = crate::state::Tool::Type;
        pointer_up(&mut app, [50.0, 100.0], [50.0, 100.0]);
        insert(&mut app, "abc");
        app
    }

    fn run(app: &mut PhotocraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Option<IMEOutput> {
        let raw = egui::RawInput { focused: true, events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| {
            let rect = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0));
            let xf = ViewXform { rect, zoom: 1.0, center: [100.0, 100.0], flip: false, rotation: 0.0, aspect: 1.0 };
            handle_keys(app, ui.ctx());
            draw_overlay(app, ui.painter(), &xf);
            sync_ime(app, ui.ctx());
            finish_reset_pass(app, ui.ctx());
        });
        out.textures_delta.clear(); // This protocol harness does not paint.
        out.platform_output.ime
    }

    fn layer_text(app: &PhotocraftApp) -> String {
        let ed = app.ui.text_edit.as_ref().expect("editing");
        current_text(app, LayerId(ed.layer)).expect("text layer")
    }

    fn completed(request_id: egui::ImeResetId) -> egui::Event {
        egui::Event::ImeResetCompleted { request_id }
    }

    fn payload(ime: &IMEOutput) -> &egui::output::IMESurroundingTextPayload {
        ime.surrounding_text.available().expect("a bounded committed snapshot")
    }

    fn preedit(text: &str) -> egui::Event {
        egui::Event::Ime(egui::ImeEvent::Preedit { text: text.to_owned(), active_range_chars: None })
    }

    #[test]
    fn successive_ime_commits_are_input_method_and_keep_directional_offsets() {
        let ctx = context();
        let mut app = app();
        let baseline = run(&mut app, &ctx, Vec::new()).expect("a focused editor descriptor");
        assert_eq!(payload(&baseline).text, "abc");
        assert_eq!(baseline.change_cause, ImeChangeCause::Other);

        // The pre-edit is laid out in the layer but is not committed surrounding text.
        let composing = run(&mut app, &ctx, vec![preedit("に")]).expect("descriptor while composing");
        assert_eq!(payload(&composing).text, "abc");
        assert_eq!((payload(&composing).cursor, payload(&composing).anchor), (3, 3));

        let first = run(&mut app, &ctx, vec![egui::Event::Ime(egui::ImeEvent::Commit("日本".into()))]).expect("commit descriptor");
        assert_eq!(first.change_cause, ImeChangeCause::InputMethod);
        assert_eq!(payload(&first).text, "abc日本");

        let composing = run(&mut app, &ctx, vec![preedit("ご")]).expect("second composition");
        assert_eq!(payload(&composing).text, "abc日本");
        let second = run(&mut app, &ctx, vec![egui::Event::Ime(egui::ImeEvent::Commit("語".into()))]).expect("second commit");
        assert_eq!(second.change_cause, ImeChangeCause::InputMethod);
        assert_eq!(payload(&second).text, "abc日本語");
        assert_eq!(payload(&second).cursor, payload(&second).text.len());
    }

    #[test]
    fn reversed_selection_keeps_its_direction_in_byte_offsets() {
        let ctx = context();
        let mut app = app();
        if let Some(ed) = app.ui.text_edit.as_mut() {
            ed.caret = 1;
            ed.anchor = 3;
        }
        let selected = run(&mut app, &ctx, Vec::new()).expect("descriptor");
        assert_eq!(payload(&selected).text, "abc");
        assert_eq!((payload(&selected).cursor, payload(&selected).anchor), (1, 3));
    }

    #[test]
    fn mixed_application_and_ime_edits_in_one_frame_are_other() {
        let ctx = context();
        let mut app = app();
        run(&mut app, &ctx, Vec::new());

        // An IME commit followed by ordinary typing in the same pass: the application change
        // dominates even though the commit itself was accepted.
        let mixed = run(&mut app, &ctx, vec![egui::Event::Ime(egui::ImeEvent::Commit("い".into())), egui::Event::Text("x".into())]).expect("mixed descriptor");
        assert_eq!(payload(&mixed).text, "abcいx");
        assert_eq!(mixed.change_cause, ImeChangeCause::Other);

        // The observed mixed state is the next baseline, so a later clean IME commit is once
        // again an InputMethod change.
        let next = run(&mut app, &ctx, vec![egui::Event::Ime(egui::ImeEvent::Commit("う".into()))]).expect("next commit");
        assert_eq!(next.change_cause, ImeChangeCause::InputMethod);
        assert_eq!(payload(&next).text, "abcいxう");

        // Application text before the commit in the same frame must also prevent InputMethod.
        let mut app = self::app();
        run(&mut app, &ctx, Vec::new());
        let mixed = run(&mut app, &ctx, vec![egui::Event::Text("x".into()), egui::Event::Ime(egui::ImeEvent::Commit("い".into()))]).expect("mixed descriptor");
        assert_eq!(mixed.change_cause, ImeChangeCause::Other);
        assert_eq!(payload(&mixed).text, "abcxい");
    }

    #[test]
    fn replaced_layer_or_session_bumps_generation_without_changing_owner() {
        let ctx = context();
        let mut app = app();
        let first = run(&mut app, &ctx, Vec::new()).expect("descriptor");
        let owner = first.owner;
        let generation = first.context_generation;

        // Same document and layer id, a new edit session: the replacement epoch changes, so the
        // stale generation cannot be reused by a native IME bridge.
        app.ui.text_edit.as_mut().unwrap().session = "type-replaced".into();
        let replaced = run(&mut app, &ctx, Vec::new()).expect("replaced descriptor");
        assert_eq!(replaced.owner, owner, "the layer identity is immutable");
        assert_ne!(replaced.context_generation, generation);
        assert_eq!(replaced.change_cause, ImeChangeCause::Other);

        // Ending the editor ends the context; no descriptor is emitted for an absent owner.
        app.ui.text_edit = None;
        assert!(run(&mut app, &ctx, Vec::new()).is_none());
    }

    #[test]
    fn unavailable_editor_state_still_carries_owner_and_generation() {
        let ctx = context();
        let mut app = app();
        // A stale composition range cannot be mapped onto the layer: do not silently publish
        // the composition as committed text.
        app.ui.text_edit.as_mut().unwrap().preedit = Some((10, 5));
        let unavailable = run(&mut app, &ctx, Vec::new()).expect("descriptor with an owner");
        assert!(unavailable.surrounding_text.available().is_none());
        assert!(unavailable.context_generation > 0);
        assert_eq!(unavailable.purpose, egui::IMEPurpose::Normal);
    }

    #[test]
    fn delete_surrounding_bytes_with_preedit_excludes_the_composition() {
        let ctx = context();
        let mut app = app();
        run(&mut app, &ctx, Vec::new());
        run(&mut app, &ctx, vec![preedit("x")]);
        assert_eq!(layer_text(&app), "abcx");
        assert_eq!(app.ui.text_edit.as_ref().unwrap().preedit, Some((3, 1)));

        // `c` is the byte immediately before the pre-edit; deleting it must not delete or
        // report the composing `x`.
        let deleted = run(&mut app, &ctx, vec![egui::Event::Ime(egui::ImeEvent::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 0 })])
            .expect("deletion descriptor");
        assert_eq!(deleted.change_cause, ImeChangeCause::InputMethod);
        assert_eq!(payload(&deleted).text, "ab");
        assert_eq!((payload(&deleted).cursor, payload(&deleted).anchor), (2, 2));
        assert_eq!(layer_text(&app), "abx");
        assert_eq!(app.ui.text_edit.as_ref().unwrap().preedit, Some((2, 1)));
    }

    #[test]
    fn external_edit_invalidates_the_preedit_and_rejects_its_late_commit() {
        for bytes in [false, true] {
            for split in [false, true] {
                for discard in [false, true] {
                    let ctx = context();
                    let mut app = app();
                    run(&mut app, &ctx, Vec::new());
                    // Compose in the middle of committed text, so an application edit before it
                    // moves a still-in-range (but wrong) character range.
                    insert(&mut app, "def");
                    if let Some(ed) = app.ui.text_edit.as_mut() {
                        ed.caret = 3;
                        ed.anchor = 3;
                    }
                    let composing = run(&mut app, &ctx, vec![preedit("に")]).expect("composing descriptor");
                    assert_eq!(layer_text(&app), "abcにdef");
                    assert_eq!(payload(&composing).text, "abcdef", "the preview is not committed text");
                    let layer = app.ui.text_edit.as_ref().unwrap().layer;
                    let session = app.ui.text_edit.as_ref().unwrap().session.clone();
                    // The control channel edits the same layer without passing through `insert`.
                    app.run("type.edit", json!({"layer": layer, "replace": {"start": 0, "end": 1, "text": ""}, "coalesce": session})).unwrap();
                    assert_eq!(layer_text(&app), "bcにdef");
                    let revision = app.session.active().unwrap().revision;
                    let history = app.session.active().unwrap().history.entries();
                    let selection = app.ui.text_edit.as_ref().map(|ed| (ed.anchor, ed.caret));
                    let unchanged = |app: &PhotocraftApp| {
                        assert_eq!(layer_text(app), "bcにdef");
                        assert_eq!(app.session.active().unwrap().revision, revision);
                        assert_eq!(app.session.active().unwrap().history.entries(), history);
                        assert_eq!(app.ui.text_edit.as_ref().map(|ed| (ed.anchor, ed.caret)), selection);
                    };

                    // The old range (3,1) now covers `d`: report unavailable, cancel the
                    // composition and do not present `d` as the composition. A discarded pass
                    // must retain both latches until a descriptor is actually delivered.
                    let invalidated = if discard {
                        ctx.options_mut(|o| o.max_passes = std::num::NonZeroUsize::new(4).expect("four passes"));
                        let mut passes = Vec::new();
                        let mut out = ctx.run_ui(egui::RawInput { focused: true, ..Default::default() }, |ui| {
                            handle_keys(&mut app, ui.ctx());
                            let rect = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0));
                            let xf = ViewXform { rect, zoom: 1.0, center: [100.0, 100.0], flip: false, rotation: 0.0, aspect: 1.0 };
                            draw_overlay(&mut app, ui.painter(), &xf);
                            if passes.is_empty() {
                                ui.ctx().request_discard("invalidated preedit descriptor regression");
                            }
                            let discarded = ui.ctx().will_discard();
                            let before = (app.type_ime.stale, app.type_ime.interrupt);
                            sync_ime(&mut app, ui.ctx());
                            finish_reset_pass(&mut app, ui.ctx());
                            let unavailable = ui.ctx().output(|out| out.ime.as_ref().expect("pass descriptor").surrounding_text.available().is_none());
                            passes.push((discarded, before, (app.type_ime.stale, app.type_ime.interrupt), unavailable));
                        });
                        out.textures_delta.clear();
                        assert_eq!(passes.first(), Some(&(true, (true, true), (true, true), true)), "discard cannot consume invalidation");
                        assert_eq!(passes.last(), Some(&(false, (true, true), (false, false), true)), "the accepted pass delivers it once");
                        out.platform_output.ime.expect("descriptor")
                    } else {
                        run(&mut app, &ctx, Vec::new()).expect("descriptor")
                    };
                    assert!(invalidated.surrounding_text.available().is_none());
                    assert!(invalidated.should_interrupt_composition);
                    assert_eq!(
                        (invalidated.owner, invalidated.context_generation, invalidated.purpose),
                        (composing.owner, composing.context_generation, composing.purpose)
                    );
                    assert_eq!(invalidated.change_cause, ImeChangeCause::Other);
                    assert_eq!(app.ui.text_edit.as_ref().unwrap().preedit, None);
                    assert!(app.type_ime.preedit.is_none());
                    unchanged(&app);

                    let recovered = run(&mut app, &ctx, Vec::new()).expect("idle descriptor");
                    assert_eq!(payload(&recovered).text, "bcにdef");
                    assert_eq!((payload(&recovered).cursor, payload(&recovered).anchor), (6, 6));
                    assert!(!recovered.should_interrupt_composition, "a delivered reset is not replayed");
                    assert!(matches!(app.type_ime.incoming, Some(IncomingComposition::Stale)), "idle availability does not accept an old terminal");
                    unchanged(&app);

                    // The pending deletion and commit belonged to the cancelled composition:
                    // neither combined nor split delivery may replace `d` or grow undo history.
                    let mut events = vec![terminal_delete(bytes)];
                    if split {
                        run(&mut app, &ctx, events).expect("rejected deletion descriptor");
                        unchanged(&app);
                        for _ in 0..3 {
                            run(&mut app, &ctx, Vec::new());
                        }
                        assert!(matches!(app.type_ime.incoming, Some(IncomingComposition::Stale)));
                        events = Vec::new();
                    }
                    events.push(egui::Event::Ime(egui::ImeEvent::Commit("に".into())));
                    let late = run(&mut app, &ctx, events).expect("descriptor");
                    assert_eq!(layer_text(&app), "bcにdef");
                    assert_eq!(late.change_cause, ImeChangeCause::Other);
                    assert_eq!(payload(&late).text, "bcにdef");
                    unchanged(&app);
                    assert!(matches!(app.type_ime.incoming, Some(IncomingComposition::Stale)), "terminal input cannot acknowledge cancellation");

                    let fresh = run(&mut app, &ctx, vec![completed(invalidated.reset_request.expect("reset request")), preedit("Q")])
                        .expect("fresh preview descriptor");
                    assert_eq!(layer_text(&app), "bcにdQef");
                    assert_eq!(payload(&fresh).text, "bcにdef", "fresh preview is excluded from committed text");
                    assert!(!fresh.should_interrupt_composition);
                    let committed = run(&mut app, &ctx, vec![egui::Event::Ime(egui::ImeEvent::Commit("Q".into()))]).expect("fresh commit descriptor");
                    assert_eq!(layer_text(&app), "bcにdQef");
                    assert_eq!(payload(&committed).text, "bcにdQef");
                    assert_eq!(committed.change_cause, ImeChangeCause::InputMethod);
                    let direct = run(&mut app, &ctx, vec![egui::Event::Ime(egui::ImeEvent::Commit("R".into()))]).expect("direct commit descriptor");
                    assert_eq!(layer_text(&app), "bcにdQRef");
                    assert_eq!(payload(&direct).text, "bcにdQRef");
                    assert_eq!(direct.change_cause, ImeChangeCause::InputMethod);
                }
            }
        }
    }

    #[test]
    fn target_change_interrupts_and_rejects_the_late_commit() {
        let ctx = context();
        let mut app = app();
        run(&mut app, &ctx, Vec::new());
        run(&mut app, &ctx, vec![preedit("に")]);
        assert_eq!(layer_text(&app), "abcに");

        let created = app.run("type.create", json!({"text": "two", "x": 150, "y": 60})).unwrap();
        let second = created.get("layer").and_then(serde_json::Value::as_u64).expect("new layer");
        app.session.select_layer(LayerId(second)).expect("select the second type layer");
        app.ui.text_edit = Some(TextEdit {
            layer: second,
            document: active_doc_id(&app),
            caret: 3,
            anchor: 3,
            session: "type-second".into(),
            created: false,
            mask: false,
            dragging: false,
            resize: None,
            preedit: None,
        });
        // The platform only cancels composition on this flag (X11, macOS, Windows and the web).
        let switched = run(&mut app, &ctx, Vec::new()).expect("transition descriptor");
        assert!(switched.should_interrupt_composition);

        let late = run(&mut app, &ctx, vec![egui::Event::Ime(egui::ImeEvent::Commit("日本".into()))]).expect("descriptor");
        assert_eq!(current_text(&app, LayerId(second)).unwrap(), "two");
        assert_eq!(late.change_cause, ImeChangeCause::Other);
        run(&mut app, &ctx, vec![completed(switched.reset_request.expect("replacement layer reset")), preedit("が")]);
        assert_eq!(layer_text(&app), "twoが");
    }

    #[test]
    fn type_mask_session_has_a_distinct_owner() {
        let ctx = context();
        let mut app = app();
        let normal = run(&mut app, &ctx, Vec::new()).expect("descriptor");
        app.ui.text_edit.as_mut().unwrap().mask = true;
        let masked = run(&mut app, &ctx, Vec::new()).expect("mask descriptor");
        assert_ne!(masked.owner, normal.owner, "the Type Mask session is its own owner");
        assert_ne!(masked.context_generation, normal.context_generation);
    }

    #[test]
    fn reentering_the_same_session_cancels_the_preedit() {
        let ctx = context();
        let mut app = app();
        run(&mut app, &ctx, vec![preedit("に")]);
        assert_eq!(app.ui.text_edit.as_ref().unwrap().preedit, Some((3, 1)));
        edit_active(&mut app).unwrap();
        assert_eq!(app.ui.text_edit.as_ref().unwrap().preedit, None);
        assert!(app.type_ime.interrupt, "the native composition is cancelled");
        assert!(app.type_ime.preedit.is_none());
    }

    #[test]
    fn a_discarded_transition_pass_still_interrupts_the_next_descriptor() {
        let ctx = context();
        ctx.options_mut(|o| o.max_passes = std::num::NonZeroUsize::new(4).expect("four passes"));
        let mut app = app();
        run(&mut app, &ctx, Vec::new()).expect("baseline descriptor");
        app.ui.text_edit.as_mut().unwrap().session = "type-discarded".into();
        let mut passes = Vec::new();
        let mut out = ctx.run_ui(egui::RawInput { focused: true, ..Default::default() }, |ui| {
            handle_keys(&mut app, ui.ctx());
            let rect = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0));
            let xf = ViewXform { rect, zoom: 1.0, center: [100.0, 100.0], flip: false, rotation: 0.0, aspect: 1.0 };
            draw_overlay(&mut app, ui.painter(), &xf);
            if passes.is_empty() {
                ui.ctx().request_discard("IME transition regression");
            }
            let discarded = ui.ctx().will_discard();
            let pending_before = app.type_ime.interrupt;
            sync_ime(&mut app, ui.ctx());
            finish_reset_pass(&mut app, ui.ctx());
            passes.push((discarded, pending_before, app.type_ime.interrupt));
        });
        out.textures_delta.clear();
        assert_eq!(passes.first(), Some(&(true, true, true)), "discard retains the interrupt latch");
        assert_eq!(passes.last(), Some(&(false, true, false)), "the accepted pass delivers it once");
        assert!(out.platform_output.ime.expect("final descriptor").should_interrupt_composition);
        let next = run(&mut app, &ctx, Vec::new()).expect("next descriptor");
        assert!(!next.should_interrupt_composition, "a delivered interrupt is not replayed");
    }

    #[test]
    fn delete_surrounding_characters_is_one_undo_step() {
        let ctx = context();
        let mut app = app();
        commit(&mut app);
        edit_active(&mut app).unwrap();
        if let Some(ed) = app.ui.text_edit.as_mut() {
            ed.caret = 3;
            ed.anchor = 3;
        }
        run(&mut app, &ctx, Vec::new()).expect("baseline descriptor");
        let deleted =
            run(&mut app, &ctx, vec![egui::Event::Ime(egui::ImeEvent::DeleteSurrounding { before_chars: 1, after_chars: 0 })]).expect("deletion descriptor");
        assert_eq!(deleted.change_cause, ImeChangeCause::InputMethod);
        assert_eq!(layer_text(&app), "ab");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(layer_text(&app), "abc", "the delete and its session coalesce into one step");
    }

    #[test]
    fn an_empty_point_text_reports_an_available_empty_snapshot() {
        let ctx = context();
        let mut app = app();
        if let Some(ed) = app.ui.text_edit.as_mut() {
            ed.anchor = 0;
            ed.caret = 3;
        }
        insert(&mut app, "");
        assert_eq!(layer_text(&app), "");
        let empty = run(&mut app, &ctx, Vec::new()).expect("descriptor");
        let available = payload(&empty);
        assert_eq!(available.text, "");
        assert_eq!((available.cursor, available.anchor), (0, 0));
    }

    #[test]
    fn a_long_line_reports_a_bounded_snapshot() {
        let ctx = context();
        let mut app = app();
        if let Some(ed) = app.ui.text_edit.as_mut() {
            ed.anchor = 0;
            ed.caret = 3;
        }
        insert(&mut app, &"a".repeat(4500));
        let long = run(&mut app, &ctx, Vec::new()).expect("descriptor");
        let available = payload(&long);
        assert!(available.text.len() <= egui::output::MAX_IME_SURROUNDING_TEXT_BYTES);
        assert!(available.text.is_char_boundary(available.cursor));
        assert!(available.text.is_char_boundary(available.anchor));
    }

    #[test]
    fn the_real_app_logic_and_ui_publish_the_descriptor() {
        use eframe::App as _;
        let ctx = context();
        let mut app = app();
        let mut frame = eframe::Frame::_new_kittest();
        let mut descriptors = Vec::new();
        // The first pass registers fonts; the next passes run the real logic → ui orchestration
        // (where `handle_keys` and the final `sync_ime` live).
        for _ in 0..3 {
            let raw =
                egui::RawInput { focused: true, screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))), ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| {
                app.logic(ui.ctx(), &mut frame);
                app.ui(ui, &mut frame);
            });
            out.textures_delta.clear(); // No renderer consumes this harness output.
            descriptors.push(out.platform_output.ime);
        }
        let ime = descriptors.iter().flatten().last().expect("a descriptor from App::ui");
        assert!(ime.surrounding_text.available().is_some());
        assert!(ime.context_generation > 0);
    }

    fn terminal_delete(bytes: bool) -> egui::Event {
        egui::Event::Ime(if bytes {
            egui::ImeEvent::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 0 }
        } else {
            egui::ImeEvent::DeleteSurrounding { before_chars: 1, after_chars: 0 }
        })
    }

    fn raw_key(key: egui::Key, command: bool, shift: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                command,
                ctrl: command && !cfg!(target_os = "macos"),
                mac_cmd: command && cfg!(target_os = "macos"),
                shift,
                ..Default::default()
            },
        }
    }

    #[test]
    fn stale_owner_terminal_survives_idle_and_split_delete_commit_but_fresh_input_works() {
        for empty in [false, true] {
            for bytes in [false, true] {
                let ctx = context();
                let mut app = app();
                run(&mut app, &ctx, vec![]);
                run(&mut app, &ctx, vec![preedit("P")]);
                if empty {
                    run(&mut app, &ctx, vec![preedit("")]);
                }
                let old_layer = app.ui.text_edit.as_ref().unwrap().layer;
                let created = app.run("type.create", json!({"text": "two", "x": 150, "y": 60})).unwrap();
                let second = created["layer"].as_u64().unwrap();
                app.session.select_layer(LayerId(second)).unwrap();
                edit_active(&mut app).unwrap();
                let transition = run(&mut app, &ctx, vec![]).unwrap();
                assert!(transition.should_interrupt_composition);
                for _ in 0..5 {
                    run(&mut app, &ctx, vec![]);
                }
                let revision = app.session.active().unwrap().revision;
                let undo = app.session.active().unwrap().history.entries().len();
                let before = layer_text(&app);
                run(&mut app, &ctx, vec![terminal_delete(bytes)]);
                for _ in 0..3 {
                    run(&mut app, &ctx, vec![]);
                }
                run(&mut app, &ctx, vec![egui::Event::Ime(egui::ImeEvent::Commit("X".into()))]);
                assert_eq!(layer_text(&app), before, "an old split terminal cannot edit a replacement owner");
                assert_eq!(app.session.active().unwrap().revision, revision);
                assert_eq!(app.session.active().unwrap().history.entries().len(), undo);
                assert_eq!(current_text(&app, LayerId(old_layer)).unwrap(), if empty { "abc" } else { "abcP" });
                let selection = app.ui.text_edit.as_ref().unwrap().clone();
                let (a, b) = (selection.anchor.min(selection.caret), selection.anchor.max(selection.caret));
                let expected = before.chars().take(a).chain("Q".chars()).chain(before.chars().skip(b)).collect::<String>();
                run(
                    &mut app,
                    &ctx,
                    vec![completed(transition.reset_request.expect("reset request")), preedit("Q"), egui::Event::Ime(egui::ImeEvent::Commit("Q".into()))],
                );
                assert_eq!(layer_text(&app), expected, "the exact completion permits fresh input");
                run(&mut app, &ctx, vec![egui::Event::Ime(egui::ImeEvent::Commit("R".into()))]);
                assert!(layer_text(&app).contains("QR"), "a later direct commit remains valid");
                app.run("edit.undo", json!({})).unwrap();
                assert_eq!(current_text(&app, LayerId(second)).unwrap(), before, "the new session is undoable");
            }
        }
    }

    #[test]
    fn raw_application_selection_changes_reject_old_terminal_input_and_allow_later_fresh_input() {
        for empty in [false, true] {
            for bytes in [false, true] {
                for (key, command, shift) in [(egui::Key::A, true, false), (egui::Key::ArrowLeft, false, false), (egui::Key::Home, false, true)] {
                    let ctx = context();
                    let mut app = app();
                    run(&mut app, &ctx, vec![]);
                    run(&mut app, &ctx, vec![preedit("P")]);
                    if empty {
                        run(&mut app, &ctx, vec![preedit("")]);
                    }
                    let before = layer_text(&app);
                    let revision = app.session.active().unwrap().revision;
                    let transition = run(&mut app, &ctx, vec![raw_key(key, command, shift)]).expect("selection reset");
                    let selected = app.ui.text_edit.as_ref().unwrap().clone();
                    assert!(
                        (selected.anchor, selected.caret) != (before.chars().count(), before.chars().count()),
                        "actual raw selection key changed the caret"
                    );
                    for _ in 0..3 {
                        run(&mut app, &ctx, vec![]);
                    }
                    let undo = app.session.active().unwrap().history.entries().len();
                    run(&mut app, &ctx, vec![terminal_delete(bytes), egui::Event::Ime(egui::ImeEvent::Commit("X".into()))]);
                    assert_eq!(layer_text(&app), before);
                    assert_eq!(app.session.active().unwrap().revision, revision);
                    assert_eq!(app.session.active().unwrap().history.entries().len(), undo);
                    assert_eq!(app.ui.text_edit.as_ref().map(|e| (e.anchor, e.caret)), Some((selected.anchor, selected.caret)));
                    let (a, b) = (selected.anchor.min(selected.caret), selected.anchor.max(selected.caret));
                    let expected = before.chars().take(a).chain("Q".chars()).chain(before.chars().skip(b)).collect::<String>();
                    // The completion opens admission at its exact position in the batch.
                    run(
                        &mut app,
                        &ctx,
                        vec![completed(transition.reset_request.expect("reset request")), preedit("Q"), egui::Event::Ime(egui::ImeEvent::Commit("Q".into()))],
                    );
                    assert_eq!(layer_text(&app), expected);
                }
            }
        }
    }

    #[test]
    fn uninterrupted_live_and_empty_preedit_delete_commit_and_direct_commit_stay_valid() {
        for empty in [false, true] {
            for bytes in [false, true] {
                let ctx = context();
                let mut app = app();
                run(&mut app, &ctx, vec![]);
                run(&mut app, &ctx, vec![preedit("P")]);
                if empty {
                    run(&mut app, &ctx, vec![preedit("")]);
                }
                run(&mut app, &ctx, vec![terminal_delete(bytes)]);
                for _ in 0..3 {
                    run(&mut app, &ctx, vec![]);
                }
                run(&mut app, &ctx, vec![egui::Event::Ime(egui::ImeEvent::Commit("X".into()))]);
                assert_eq!(layer_text(&app), "abX", "accepted deletion refreshes its composing caret");
            }
        }
        let ctx = context();
        let mut app = app();
        run(&mut app, &ctx, vec![]);
        run(&mut app, &ctx, vec![raw_key(egui::Key::A, true, false), egui::Event::Ime(egui::ImeEvent::Commit("D".into()))]);
        assert_eq!(layer_text(&app), "D", "without known preedit the direct commit replaces the selection");
    }

    fn actual_app_frame(app: &mut PhotocraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
        use eframe::App as _;
        let mut frame = eframe::Frame::_new_kittest();
        let mut raw =
            egui::RawInput { focused: true, events, screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))), ..Default::default() };
        app.raw_input_hook(ctx, &mut raw);
        let mut output = ctx.run_ui(raw, |ui| {
            app.logic(ui.ctx(), &mut frame);
            app.ui(ui, &mut frame);
        });
        output.textures_delta.clear();
        output
    }

    #[test]
    fn the_real_app_pointer_interruption_rejects_idle_delayed_terminal_then_accepts_fresh_input() {
        for empty in [false, true] {
            for bytes in [false, true] {
                let ctx = context();
                let mut app = app();
                for _ in 0..4 {
                    actual_app_frame(&mut app, &ctx, vec![]);
                }
                app.ui.text_edit.as_mut().unwrap().caret = 1;
                app.ui.text_edit.as_mut().unwrap().anchor = 1;
                let point = actual_app_frame(&mut app, &ctx, vec![]).platform_output.ime.expect("real caret1 descriptor").cursor_rect.center();
                app.ui.text_edit.as_mut().unwrap().caret = 3;
                app.ui.text_edit.as_mut().unwrap().anchor = 3;
                actual_app_frame(&mut app, &ctx, vec![]);
                actual_app_frame(&mut app, &ctx, vec![preedit("P")]);
                if empty {
                    actual_app_frame(&mut app, &ctx, vec![preedit("")]);
                }
                let before = layer_text(&app);
                let button = |pressed| egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
                let press =
                    actual_app_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(point), button(true)]).platform_output.ime.expect("press descriptor");
                let release = actual_app_frame(&mut app, &ctx, vec![button(false)]).platform_output.ime.expect("release descriptor");
                let request = release.reset_request.or(press.reset_request).expect("pointer reset");
                assert_eq!(app.ui.text_edit.as_ref().unwrap().caret, 1, "actual canvas pointer moved the caret");
                for _ in 0..4 {
                    actual_app_frame(&mut app, &ctx, vec![]);
                }
                let revision = app.session.active().unwrap().revision;
                actual_app_frame(&mut app, &ctx, vec![terminal_delete(bytes), egui::Event::Ime(egui::ImeEvent::Commit("X".into()))]);
                assert_eq!(layer_text(&app), before);
                assert_eq!(app.session.active().unwrap().revision, revision);
                actual_app_frame(&mut app, &ctx, vec![completed(request), preedit("Q"), egui::Event::Ime(egui::ImeEvent::Commit("Q".into()))]);
                let expected = before.chars().take(1).chain("Q".chars()).chain(before.chars().skip(1)).collect::<String>();
                assert_eq!(layer_text(&app), expected);
            }
        }
    }

    #[test]
    fn only_a_matching_completion_allows_a_new_composition_in_the_same_raw_batch() {
        for bytes in [false, true] {
            let ctx = context();
            let mut app = app();
            run(&mut app, &ctx, vec![]);
            run(&mut app, &ctx, vec![preedit("P")]);
            let transition = run(&mut app, &ctx, vec![raw_key(egui::Key::A, true, false)]).expect("selection reset");
            run(
                &mut app,
                &ctx,
                vec![
                    terminal_delete(bytes),
                    egui::Event::Ime(egui::ImeEvent::Commit("old".into())),
                    completed(transition.reset_request.expect("reset request")),
                    preedit("Q"),
                    egui::Event::Ime(egui::ImeEvent::Commit("Q".into())),
                ],
            );
            assert_eq!(layer_text(&app), "Q", "the old terminal is rejected before the completion opens admission");
        }
    }

    #[test]
    fn an_actual_same_batch_selection_requires_completion_before_the_new_live_preedit() {
        for empty in [false, true] {
            for bytes in [false, true] {
                for terminal in [false, true] {
                    let ctx = context();
                    let mut app = app();
                    for _ in 0..4 {
                        actual_app_frame(&mut app, &ctx, vec![]);
                    }
                    actual_app_frame(&mut app, &ctx, vec![preedit("P")]);
                    if empty {
                        actual_app_frame(&mut app, &ctx, vec![preedit("")]);
                    }
                    let before = layer_text(&app);
                    let revision = app.session.active().unwrap().revision;
                    let mut events = vec![raw_key(egui::Key::A, true, false), terminal_delete(bytes)];
                    if terminal {
                        events.push(egui::Event::Ime(egui::ImeEvent::Commit("old".into())));
                    }
                    events.push(preedit("Q"));
                    let out = actual_app_frame(&mut app, &ctx, events);
                    assert_eq!(layer_text(&app), before, "all unacknowledged payloads remain stale, even after Commit");
                    assert_eq!(app.session.active().unwrap().revision, revision);
                    assert!(matches!(app.type_ime.incoming, Some(IncomingComposition::Stale)));
                    let selected = app.ui.text_edit.as_ref().unwrap();
                    assert_eq!((selected.anchor, selected.caret), (0, before.chars().count()), "the actual selection key ran");
                    let request = out.platform_output.ime.expect("interrupted descriptor").reset_request.expect("reset request");
                    actual_app_frame(&mut app, &ctx, vec![completed(request), preedit("Q")]);
                    assert_eq!(layer_text(&app), "Q", "only the new preedit replaced the selected text");
                    assert_eq!(app.session.active().unwrap().revision, revision + 1, "neither old deletion nor old commit mutated the document");
                    let editing = app.ui.text_edit.as_ref().unwrap();
                    assert_eq!((editing.anchor, editing.caret, editing.preedit), (1, 1, Some((0, 1))));
                    assert!(matches!(app.type_ime.incoming, Some(IncomingComposition::Active(_))));
                    // Keep Q live across a real idle pass before its separately delivered commit.
                    let idle = actual_app_frame(&mut app, &ctx, vec![]);
                    assert!(!idle.platform_output.ime.expect("the live preview keeps its descriptor").should_interrupt_composition);
                    actual_app_frame(&mut app, &ctx, vec![egui::Event::Ime(egui::ImeEvent::Commit("Q".into()))]);
                    assert_eq!(layer_text(&app), "Q");
                    assert!(app.ui.text_edit.as_ref().unwrap().preedit.is_none());
                    assert!(app.type_ime.incoming.is_none());
                }
            }
        }
    }

    fn pending(app: &PhotocraftApp) -> Option<egui::ImeResetId> {
        match app.type_ime.resets.get(&egui::ViewportId::ROOT) {
            Some(ResetPhase::Awaiting { id, .. }) => Some(*id),
            _ => None,
        }
    }

    fn interrupt_with_selection(app: &mut PhotocraftApp, ctx: &egui::Context) -> egui::ImeResetId {
        run(app, ctx, vec![preedit("P")]);
        run(app, ctx, vec![raw_key(egui::Key::A, true, false)]).expect("custom output").reset_request.expect("published reset")
    }

    #[test]
    fn newest_published_request_alone_opens_the_ordered_suffix() {
        let ctx = context();
        let mut app = app();
        let first = interrupt_with_selection(&mut app, &ctx);
        let second = run(&mut app, &ctx, vec![raw_key(egui::Key::ArrowLeft, false, false), completed(first), preedit("old")])
            .expect("moved selection")
            .reset_request
            .expect("replacement request");
        assert_ne!(first, second);
        let before = layer_text(&app);
        let revision = app.session.active().unwrap().revision;
        let history = app.session.active().unwrap().history.entries().len();
        run(&mut app, &ctx, vec![terminal_delete(true), completed(first), egui::Event::Ime(egui::ImeEvent::Commit("old".into()))]);
        assert_eq!(layer_text(&app), before);
        assert_eq!(app.session.active().unwrap().revision, revision);
        assert_eq!(app.session.active().unwrap().history.entries().len(), history);
        assert_eq!(pending(&app), Some(second));
        let selection = app.ui.text_edit.as_ref().map(|e| (e.anchor, e.caret));
        run(&mut app, &ctx, vec![completed(second), completed(second)]);
        assert_eq!(layer_text(&app), before, "controls alone never edit text");
        assert_eq!(app.ui.text_edit.as_ref().map(|e| (e.anchor, e.caret)), selection);
        assert_eq!(app.session.active().unwrap().revision, revision);
        assert_eq!(app.session.active().unwrap().history.entries().len(), history);
        assert_eq!(pending(&app), None);
        run(&mut app, &ctx, vec![preedit("が"), egui::Event::Ime(egui::ImeEvent::Commit("が".into()))]);
        assert!(layer_text(&app).starts_with("が"));
    }

    #[test]
    fn foreign_viewport_and_context_completions_cannot_release_root() {
        let ctx = context();
        let mut app = app();
        let root = interrupt_with_selection(&mut app, &ctx);
        let other = context();
        let viewport = egui::ViewportId::from_hash_of("photo-reset-foreign");
        let foreign_input = || {
            let rect = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0));
            let mut raw = egui::RawInput { viewport_id: viewport, screen_rect: Some(rect), focused: true, ..Default::default() };
            raw.viewports.insert(
                viewport,
                egui::ViewportInfo { inner_rect: Some(rect), native_pixels_per_point: Some(1.0), minimized: Some(false), ..Default::default() },
            );
            raw
        };
        let mut text = String::new();
        let mut output = other.run_ui(foreign_input(), |ui| {
            ui.add(egui::TextEdit::singleline(&mut text)).request_focus();
            let _ = ui.ctx().request_ime_reset(viewport);
        });
        output.textures_delta.clear();
        // Draw once more if egui has not yet reported the newly focused stock editor.
        if output.platform_output.ime.is_none() {
            output = other.run_ui(foreign_input(), |ui| {
                ui.add(egui::TextEdit::singleline(&mut text));
            });
            output.textures_delta.clear();
        }
        let foreign = output.platform_output.ime.expect("foreign stock output").reset_request.expect("foreign published request");
        assert_ne!(foreign.viewport_id(), root.viewport_id());
        let before = layer_text(&app);
        run(&mut app, &ctx, vec![completed(foreign), preedit("bad"), completed(root), preedit("が")]);
        assert_eq!(layer_text(&app), "が", "foreign marker cannot open admission before the root marker");
        assert_ne!(layer_text(&app), before);
    }

    #[test]
    fn standalone_logic_suspends_once_without_replaying_input_and_return_rearms_once() {
        use eframe::App as _;
        let ctx = context();
        let mut app = app();
        for _ in 0..4 {
            actual_app_frame(&mut app, &ctx, vec![]);
        }
        actual_app_frame(&mut app, &ctx, vec![preedit("P")]);
        let first = actual_app_frame(&mut app, &ctx, vec![raw_key(egui::Key::A, true, false)])
            .platform_output
            .ime
            .expect("custom output")
            .reset_request
            .expect("first request");
        let before = layer_text(&app);
        for _ in 0..3 {
            let mut frame = eframe::Frame::_new_kittest();
            let _ = ctx.run_logic(&egui::RawInput { focused: true, ..Default::default() }, |ctx| app.logic(ctx, &mut frame));
            assert_eq!(layer_text(&app), before, "hidden logic does not replay the previous raw selection batch");
            assert_eq!(pending(&app), None);
            assert!(matches!(app.type_ime.resets.get(&egui::ViewportId::ROOT), Some(ResetPhase::Suspended)));
        }
        let second = actual_app_frame(&mut app, &ctx, vec![completed(first), preedit("bad")])
            .platform_output
            .ime
            .expect("returned custom output")
            .reset_request
            .expect("fresh request after hidden logic");
        assert_ne!(first, second);
        assert_eq!(layer_text(&app), before);
        for _ in 0..3 {
            let idle = actual_app_frame(&mut app, &ctx, vec![]).platform_output.ime.expect("idle custom output");
            assert_eq!(idle.reset_request, None, "logic inside a full UI pass cannot suspend and rearm the stable guard");
            assert_eq!(pending(&app), Some(second));
        }
        actual_app_frame(&mut app, &ctx, vec![completed(second), preedit("が"), egui::Event::Ime(egui::ImeEvent::Commit("が".into()))]);
        assert_eq!(layer_text(&app), "が");
    }

    #[test]
    fn standalone_logic_does_not_guard_an_uninterrupted_terminal_pending_composition() {
        use eframe::App as _;
        let ctx = context();
        let mut app = app();
        for _ in 0..4 {
            actual_app_frame(&mut app, &ctx, vec![]);
        }
        actual_app_frame(&mut app, &ctx, vec![preedit("P"), preedit("")]);
        let mut frame = eframe::Frame::_new_kittest();
        let _ = ctx.run_logic(&egui::RawInput { focused: true, ..Default::default() }, |ctx| app.logic(ctx, &mut frame));
        assert!(app.type_ime.resets.is_empty());
        assert!(matches!(app.type_ime.incoming, Some(IncomingComposition::TerminalPending(_))));
        actual_app_frame(&mut app, &ctx, vec![terminal_delete(false), egui::Event::Ime(egui::ImeEvent::Commit("X".into()))]);
        assert_eq!(layer_text(&app), "abX");
    }

    #[test]
    fn fonts_not_ready_and_no_editor_early_returns_retire_the_old_request() {
        use eframe::App as _;
        for no_editor in [false, true] {
            let ctx = context();
            let mut app = app();
            for _ in 0..4 {
                actual_app_frame(&mut app, &ctx, vec![]);
            }
            actual_app_frame(&mut app, &ctx, vec![preedit("P")]);
            let first = actual_app_frame(&mut app, &ctx, vec![raw_key(egui::Key::A, true, false)])
                .platform_output
                .ime
                .expect("custom output")
                .reset_request
                .expect("first request");
            let editor = app.ui.text_edit.clone();
            let before = layer_text(&app);
            let mut frame = eframe::Frame::_new_kittest();
            let mut output = ctx.run_ui(egui::RawInput { focused: true, events: vec![completed(first), preedit("bad")], ..Default::default() }, |ui| {
                if no_editor {
                    app.ui.text_edit = None;
                } else {
                    app.fonts_ready = false;
                }
                app.ui(ui, &mut frame);
            });
            output.textures_delta.clear();
            assert!(output.platform_output.ime.is_none());
            assert_eq!(pending(&app), None);
            app.ui.text_edit = editor;
            app.fonts_ready = true;
            let second = actual_app_frame(&mut app, &ctx, vec![completed(first), preedit("bad")])
                .platform_output
                .ime
                .expect("same editor returned")
                .reset_request
                .expect("rearmed request");
            assert_ne!(first, second);
            assert_eq!(layer_text(&app), before);
            actual_app_frame(&mut app, &ctx, vec![completed(second), preedit("が")]);
            assert_eq!(layer_text(&app), "が");
        }
    }

    #[test]
    fn pointer_deferred_suffix_cannot_edit_the_old_caret_or_complete_its_old_request() {
        let ctx = context();
        let mut app = app();
        for _ in 0..4 {
            actual_app_frame(&mut app, &ctx, vec![]);
        }
        app.ui.text_edit.as_mut().unwrap().caret = 1;
        app.ui.text_edit.as_mut().unwrap().anchor = 1;
        let point = actual_app_frame(&mut app, &ctx, vec![]).platform_output.ime.expect("caret geometry").cursor_rect.center();
        app.ui.text_edit.as_mut().unwrap().caret = 3;
        app.ui.text_edit.as_mut().unwrap().anchor = 3;
        actual_app_frame(&mut app, &ctx, vec![preedit("P")]);
        let first = actual_app_frame(&mut app, &ctx, vec![raw_key(egui::Key::A, true, false)])
            .platform_output
            .ime
            .expect("selection output")
            .reset_request
            .expect("first request");
        let before = layer_text(&app);
        let revision = app.session.active().unwrap().revision;
        let event = |pressed| egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let out = actual_app_frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(point),
                event(true),
                completed(first),
                egui::Event::Text("bad".into()),
                egui::Event::Paste("bad".into()),
                egui::Event::Cut,
                preedit("bad"),
            ],
        );
        let first_after_pointer = out.platform_output.ime.expect("pointer output").reset_request;
        let out = actual_app_frame(&mut app, &ctx, vec![event(false)]);
        let second = out.platform_output.ime.expect("release output").reset_request.or(first_after_pointer).expect("actual pointer destination reset");
        assert_ne!(first, second);
        assert_eq!(layer_text(&app), before);
        assert_eq!(app.session.active().unwrap().revision, revision);
        assert_eq!(app.ui.text_edit.as_ref().unwrap().caret, 1);
        actual_app_frame(&mut app, &ctx, vec![completed(second), preedit("が")]);
        assert!(layer_text(&app).contains("が"));
    }

    #[test]
    fn completion_without_current_geometry_is_suspended_before_the_following_preedit() {
        let ctx = context();
        let mut app = app();
        let first = interrupt_with_selection(&mut app, &ctx);
        let before = layer_text(&app);
        // No canvas view for this input batch: the completion cannot be attributed to a visible
        // custom descriptor yet. This is the pre-descriptor check, not a wait on final output.
        app.type_ime.view = None;
        let output = run(&mut app, &ctx, vec![completed(first), preedit("bad")]).expect("custom output");
        let second = output.reset_request.expect("the final custom output rearms");
        assert_ne!(first, second);
        assert_eq!(layer_text(&app), before, "the preedit after a geometry-less completion stays blocked");
        assert!(matches!(app.type_ime.resets.get(&egui::ViewportId::ROOT), Some(ResetPhase::Awaiting { id, .. }) if *id == second));
        run(&mut app, &ctx, vec![completed(second), preedit("が")]);
        assert_eq!(layer_text(&app), "が");
    }

    #[test]
    fn pointer_press_without_a_guard_defers_the_same_batch_custom_suffix() {
        let ctx = context();
        let mut app = app();
        let before = layer_text(&app);
        let revision = app.session.active().unwrap().revision;
        let selection = app.ui.text_edit.as_ref().map(|ed| (ed.anchor, ed.caret));
        let press =
            egui::Event::PointerButton { pos: Pos2::new(20.0, 20.0), button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() };
        let output = run(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(Pos2::new(20.0, 20.0)),
                press,
                egui::Event::Text("bad".into()),
                egui::Event::Paste("bad".into()),
                egui::Event::Cut,
                preedit("bad"),
            ],
        )
        .expect("descriptor");
        assert_eq!(layer_text(&app), before, "the deferred canvas handler never lets a suffix edit the old caret");
        assert_eq!(app.session.active().unwrap().revision, revision);
        assert_eq!(app.ui.text_edit.as_ref().map(|ed| (ed.anchor, ed.caret)), selection);
        assert_eq!(output.reset_request, None, "no prior guard means no speculative reset yet");
    }

    #[test]
    fn synchronous_select_all_accepts_fresh_text_but_still_blocks_stale_ime() {
        let ctx = context();
        // With no guard, the synchronously selected destination is a real destination: Text is
        // an application edit, not an old IME suffix.
        let mut app = app();
        run(&mut app, &ctx, vec![raw_key(egui::Key::A, true, false), egui::Event::Text("x".into())]);
        assert_eq!(layer_text(&app), "x");

        // The same holds when Cmd+A interrupted a known composition: Text is still fresh input.
        let mut app = self::app();
        run(&mut app, &ctx, vec![preedit("P")]);
        let out = run(&mut app, &ctx, vec![raw_key(egui::Key::A, true, false), egui::Event::Text("x".into())]).expect("custom output");
        assert_eq!(layer_text(&app), "x");
        assert!(out.reset_request.is_some(), "the stale composition still needs its ordered reset");

        // An IME payload in that same batch is not fresh Text: it remains blocked until its
        // replacement request is actually completed.
        let mut app = self::app();
        run(&mut app, &ctx, vec![preedit("P")]);
        let before = layer_text(&app);
        let out = run(&mut app, &ctx, vec![raw_key(egui::Key::A, true, false), preedit("bad")]).expect("custom output");
        assert_eq!(layer_text(&app), before);
        let request = out.reset_request.expect("selection reset request");
        run(&mut app, &ctx, vec![completed(request), preedit("が")]);
        assert_eq!(layer_text(&app), "が");
    }

    struct ResetHarness {
        app: PhotocraftApp,
        stock: bool,
        picker_button: bool,
        search: String,
        search_input: Option<egui::Id>,
        search_focused: bool,
        search_height: Option<f32>,
    }

    fn reset_harness(app: PhotocraftApp) -> egui_kittest::Harness<'static, ResetHarness> {
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1200.0, 800.0))
            .with_context_setup(|ctx| {
                install_reset_hooks(ctx);
                crate::theme::install_fonts(ctx);
            })
            .build_ui_state(
                |ui, state: &mut ResetHarness| {
                    let mut frame = eframe::Frame::_new_kittest();
                    state.app.logic_frame(ui.ctx(), &mut frame);
                    state.app.ui_frame(ui, &mut frame);
                    if state.stock {
                        egui::Window::new("Reset font fixture").show(ui.ctx(), |ui| {
                            let button = ui.make_persistent_id(egui::IdSalt::new("reset-font"));
                            egui::Popup::open_id(ui.ctx(), button.with("popup"));
                            let mut current = "Family 00".to_owned();
                            family_picker_in(
                                ui,
                                "reset-font",
                                &mut current,
                                170.0,
                                &["Family 00".into(), "Family 01".into()],
                                photocraft_engine::prefs::FontPreview::Medium,
                                &[],
                            );
                            let search = ui.scope_id().with(("font-search", "reset-font"));
                            state.search = ui.data(|d| d.get_temp::<String>(search)).unwrap_or_default();
                            let input = search.with("input");
                            state.search_input = Some(input);
                            state.search_focused = ui.memory(|m| m.has_focus(input));
                            state.search_height = ui.data(|d| d.get_temp::<f32>(search.with("height")));
                        });
                    }
                    if state.picker_button {
                        egui::Window::new("Reset screen fixture").show(ui.ctx(), |ui| {
                            crate::screen_picker::button(ui, egui::Id::unique("reset-screen"));
                        });
                    }
                    state.app.finish_type_reset_pass(ui.ctx());
                },
                ResetHarness { app, stock: false, picker_button: false, search: String::new(), search_input: None, search_focused: false, search_height: None },
            );
        h.run_steps(4);
        h
    }

    fn harness_request(h: &egui_kittest::Harness<'_, ResetHarness>) -> egui::ImeResetId {
        h.output().platform_output.ime.as_ref().expect("actual harness output").reset_request.expect("published harness reset")
    }

    #[test]
    fn actual_font_search_owns_stock_input_without_a_speculative_custom_request() {
        let mut h = reset_harness(app());
        h.event(preedit("P"));
        h.step();
        h.event(raw_key(egui::Key::A, true, false));
        h.step();
        let first = harness_request(&h);
        let before = layer_text(&h.state().app);
        let custom_owner = h.output().platform_output.ime.as_ref().unwrap().owner;
        h.state_mut().stock = true;
        h.run_steps(3);
        let stock_owner = h.output().platform_output.ime.as_ref().expect("real font search descriptor").owner;
        assert_ne!(stock_owner, custom_owner);
        let search_input = h.state().search_input.expect("font search field id");
        assert!(h.state().search_focused, "the real font search field, not another stock field, owns focus");
        assert_eq!(h.ctx.memory(|m| m.focused()), Some(search_input), "the stock IME owner is the focused font search input");
        assert!(
            h.state().search_height.is_some_and(|height| height > 0.0 && height <= FONT_MENU_HEIGHT),
            "the real font list measured within its bounded popup height"
        );
        h.input_mut().events = vec![completed(first), egui::Event::Text("Family".into())];
        h.step();
        assert_eq!(h.state().search, "Family", "the actual font picker keeps its text event");
        assert_eq!(layer_text(&h.state().app), before);
        assert_eq!(pending(&h.state().app), None);
        let stock = h.output().platform_output.ime.as_ref().expect("stock output survives finalization");
        assert_eq!(stock.owner, stock_owner);
        assert_eq!(stock.reset_request, None, "an already-ineligible route cannot allocate a custom request");
        h.state_mut().stock = false;
        egui::Popup::close_all(&h.ctx);
        h.ctx.memory_mut(|m| m.stop_text_input());
        h.input_mut().events = vec![completed(first), preedit("bad")];
        h.step();
        let second = harness_request(&h);
        assert_ne!(first, second);
        assert_eq!(layer_text(&h.state().app), before);
        h.step();
        assert_eq!(h.output().platform_output.ime.as_ref().unwrap().reset_request, None);
        h.input_mut().events = vec![completed(second), preedit("が")];
        h.step();
        assert_eq!(layer_text(&h.state().app), "が");
    }

    type ImmediateViewportRecord = (egui::ViewportId, egui::ViewportId, Option<egui::Vec2>, Option<bool>);

    fn record_immediate_viewports() -> Arc<std::sync::Mutex<Vec<ImmediateViewportRecord>>> {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = seen.clone();
        egui::Context::set_immediate_viewport_renderer(move |ctx, immediate| {
            let ids = immediate.ids;
            if let Ok(mut seen) = recorded.lock() {
                seen.push((ids.this, ids.parent, immediate.builder.inner_size, immediate.builder.visible));
            }
            let mut callback = immediate.viewport_ui_cb;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    viewport_id: ids.this,
                    viewports: std::iter::once((ids.this, egui::ViewportInfo::default())).collect(),
                    screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(2.0, 2.0))),
                    ..Default::default()
                },
                |ui| callback(ui),
            );
            output.textures_delta.clear();
        });
        seen
    }

    #[test]
    fn actual_screen_picker_modal_retires_the_type_fence_and_return_rearms_it() {
        use egui_kittest::kittest::Queryable as _;
        let mut app = app();
        app.services.screen_pick = Some(Box::new(|_| {
            let (sender, receiver) = std::sync::mpsc::channel();
            sender
                .send(Ok(crate::screen_picker::Capture::Images(vec![crate::screen_picker::ScreenImage {
                    monitor: None,
                    position: Pos2::ZERO,
                    size: egui::vec2(2.0, 2.0),
                    width: 2,
                    height: 2,
                    rgba: vec![0; 16],
                    profile: None,
                }])))
                .unwrap();
            crate::screen_picker::Pending { receiver, cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)) }
        }));
        let mut h = reset_harness(app);
        h.event(preedit("P"));
        h.step();
        h.event(raw_key(egui::Key::A, true, false));
        h.step();
        let first = harness_request(&h);
        let before = layer_text(&h.state().app);
        let immediate = record_immediate_viewports();
        h.ctx.set_embed_viewports(false);
        h.state_mut().picker_button = true;
        h.run_steps(2);
        h.get_by_label("Pick screen color").click();
        h.run_steps(3);
        assert!(crate::screen_picker::busy(&h.ctx));
        assert!(crate::screen_picker::showing(&h.ctx), "the actual UI takes its screen-picker modal early return");
        let seen = immediate.lock().expect("recorded viewports");
        assert!(!seen.is_empty(), "the native immediate-viewport renderer received the picker");
        assert!(
            seen.iter().all(|(child, parent, size, visible)| *child != egui::ViewportId::ROOT
                && *parent == egui::ViewportId::ROOT
                && *size == Some(egui::vec2(2.0, 2.0))
                && *visible == Some(false)),
            "each picker viewport is a hidden 2x2 child of the root"
        );
        drop(seen);
        h.input_mut().events = vec![completed(first), preedit("bad")];
        h.step();
        assert!(h.output().platform_output.ime.is_none());
        assert_eq!(pending(&h.state().app), None);
        assert_eq!(layer_text(&h.state().app), before);
        crate::screen_picker::cancel(&h.ctx);
        h.state_mut().picker_button = false;
        h.ctx.memory_mut(|m| m.stop_text_input());
        h.input_mut().events = vec![completed(first), preedit("bad")];
        h.step();
        let second = harness_request(&h);
        assert_ne!(first, second);
        assert_eq!(layer_text(&h.state().app), before);
        h.input_mut().events = vec![completed(second), preedit("が")];
        h.step();
        assert_eq!(layer_text(&h.state().app), "が");
    }

    #[test]
    fn a_discarded_custom_output_allocates_no_new_id_and_final_absence_retires_then_rearms() {
        let ctx = context();
        ctx.options_mut(|o| o.max_passes = std::num::NonZeroUsize::new(4).unwrap());
        let mut app = app();
        // A prior queued R1 is deliberately present: the discard must preserve it, not allocate
        // a speculative replacement for output egui will throw away.
        let first = interrupt_with_selection(&mut app, &ctx);
        let mut passes = 0;
        let mut discarded_pending = None;
        let mut output = ctx.run_ui(egui::RawInput { focused: true, ..Default::default() }, |ui| {
            handle_keys(&mut app, ui.ctx());
            if passes == 0 {
                let xf = ViewXform {
                    rect: egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0)),
                    zoom: 1.0,
                    center: [100.0, 100.0],
                    flip: false,
                    rotation: 0.0,
                    aspect: 1.0,
                };
                draw_overlay(&mut app, ui.painter(), &xf);
                ui.ctx().request_discard("reset custom output disappears on final pass");
            }
            sync_ime(&mut app, ui.ctx());
            finish_reset_pass(&mut app, ui.ctx());
            if passes == 0 {
                discarded_pending = pending(&app);
            }
            passes += 1;
        });
        output.textures_delta.clear();
        assert!(passes > 1);
        assert!(output.platform_output.ime.is_none());
        assert_eq!(discarded_pending, Some(first), "discard keeps the queued R1 and allocates no NEW id");
        assert_eq!(pending(&app), None, "the final ineligible pass retires the prior R1");
        let before = layer_text(&app);
        let returned = run(&mut app, &ctx, vec![completed(first), preedit("bad")]).expect("same model returns");
        let second = returned.reset_request.expect("new published request");
        assert_ne!(first, second);
        assert_eq!(layer_text(&app), before);
        run(&mut app, &ctx, vec![completed(second), preedit("が")]);
        assert_eq!(layer_text(&app), "が");
    }

    #[test]
    fn a_recreated_context_installed_before_run_ui_rejects_the_old_completion_and_rearms() {
        use eframe::App as _;
        let old = context();
        let mut app = app();
        for _ in 0..4 {
            actual_app_frame(&mut app, &old, vec![]);
        }
        let first = interrupt_with_selection(&mut app, &old);
        let before = layer_text(&app);
        // The host replaces its Context and must run the same supported setup before `run_ui`;
        // a begin-pass hook registered later could not mark the first pass.
        let next = egui::Context::default();
        PhotocraftApp::setup_context(&next, app.ui.theme);
        let mut frame = eframe::Frame::_new_kittest();
        let mut saw_ui_pass = false;
        let mut output = next.run_ui(
            egui::RawInput {
                focused: true,
                events: vec![completed(first), preedit("bad")],
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                ..Default::default()
            },
            |ui| {
                app.logic(ui.ctx(), &mut frame);
                saw_ui_pass = in_ui_pass(ui.ctx());
                app.ui(ui, &mut frame);
            },
        );
        output.textures_delta.clear();
        assert!(saw_ui_pass, "the supported creator setup marks the first replacement Context pass");
        let second = output.platform_output.ime.expect("replacement Context custom output").reset_request.expect("replacement Context request");
        assert_ne!(first, second);
        assert_eq!(layer_text(&app), before);
        run(&mut app, &next, vec![completed(second), preedit("が")]);
        assert_eq!(layer_text(&app), "が");
    }

    #[test]
    fn type_mask_and_same_owner_session_replacement_supersede_the_full_request() {
        for mask in [false, true] {
            let ctx = context();
            let mut app = app();
            let first = interrupt_with_selection(&mut app, &ctx);
            let original = run(&mut app, &ctx, vec![]).expect("original context");
            if mask {
                app.ui.text_edit.as_mut().unwrap().mask = true;
            } else {
                app.ui.text_edit.as_mut().unwrap().session = "replacement-reset-session".into();
            }
            let before = layer_text(&app);
            let changed = run(&mut app, &ctx, vec![completed(first), preedit("bad")]).expect("replacement output");
            let second = changed.reset_request.expect("replacement request");
            assert_ne!(first, second);
            if mask {
                assert_ne!(changed.owner, original.owner);
            } else {
                assert_eq!(changed.owner, original.owner);
            }
            assert_ne!(changed.context_generation, original.context_generation);
            assert_eq!(layer_text(&app), before);
            run(&mut app, &ctx, vec![completed(first), terminal_delete(false), egui::Event::Ime(egui::ImeEvent::Commit("bad".into()))]);
            assert_eq!(layer_text(&app), before);
            run(&mut app, &ctx, vec![completed(second), preedit("が")]);
            assert_eq!(layer_text(&app), "が");
        }
    }

    #[test]
    fn document_replacement_cannot_reuse_the_previous_documents_reset() {
        let ctx = context();
        let mut app = app();
        let first = interrupt_with_selection(&mut app, &ctx);
        let old_document = active_doc_id(&app);
        app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
        app.sync_views();
        // The host runs this before handling input: a new active document ends the old edit in
        // its own document, so the next canvas click starts an editor for the new one.
        end_if_left(&mut app);
        assert!(app.ui.text_edit.is_none(), "the new document starts without the previous edit target");
        app.ui.tool = crate::state::Tool::Type;
        pointer_up(&mut app, [50.0, 100.0], [50.0, 100.0]);
        insert(&mut app, "new");
        assert_ne!(active_doc_id(&app), old_document);
        let before = layer_text(&app);
        let changed = run(&mut app, &ctx, vec![completed(first), preedit("bad")]).expect("new document output");
        let second = changed.reset_request.expect("new document reset");
        assert_ne!(first, second);
        assert_eq!(layer_text(&app), before);
        run(&mut app, &ctx, vec![completed(second), preedit("が")]);
        assert!(layer_text(&app).contains("が"));
    }
}
