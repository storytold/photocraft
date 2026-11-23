//! Photoshop's Pattern Fill dialog (#2908): Layer › New Fill Layer › Pattern… and double-clicking a
//! Pattern Fill layer's thumbnail open it. The dialog is a hosted [`PatternFillDialog`]; this module
//! keeps what the app adds to it: the target it edits, the values it opened with, the live canvas
//! preview, and the canvas drag that moves the pattern. OK runs one engine command
//! (`layer.newFillLayer.pattern` for a new layer, `layer.fill.pattern.set` for an existing one), so it
//! is one History step; Cancel changes nothing.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use craft::draw::ui::pattern::{PatternFillDialog, PatternFillOptions, PatternSwatch};
use craft::ui::dialog::{DialogId, DialogResult};
use photocraft_doc::{DocId, Document, Fill, LayerContent, LayerId};
use photocraft_engine::pattern_fill_cmds as cmds;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::document_display::CanvasHost;

/// What the engine accepts for a phase coordinate (`pattern_fill_cmds`).
const MAX_PHASE: f64 = 1.0e7;

/// The app's Pattern Fill state: the open counter and the session of the open dialog.
#[derive(Default)]
pub(crate) struct State {
    opens: u64,
    pub(crate) session: Option<Session>,
}

/// What the dialog edits: an existing Pattern Fill layer, or a new fill inserted above a layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Edit(LayerId),
    /// `insert` is the active layer when the dialog opened (`None` with none, which the command allows).
    Create {
        insert: Option<LayerId>,
    },
}

/// The open dialog's context. The hosted dialog's id is fixed, so `serial` tells one opening from the next.
pub(crate) struct Session {
    serial: u64,
    document: DocId,
    target: Target,
    opened: PatternFillOptions,
    preview: Option<Preview>,
    /// The swatches' textures. The picker paints them by id and doesn't ask the texture cache again, so the dialog
    /// owns them until it closes: a cached texture would be freed once the cache is full (#16252).
    _swatch_textures: Vec<egui::TextureHandle>,
}

impl Session {
    #[cfg(test)]
    /// The layer an edit changes; `None` for a new fill.
    pub(crate) fn layer(&self) -> Option<LayerId> {
        match self.target {
            Target::Edit(layer) => Some(layer),
            Target::Create { .. } => None,
        }
    }
}

/// The cached canvas preview of one set of values.
pub(crate) struct Preview {
    document: DocId,
    revision: u64,
    serial: u64,
    params: Value,
    shown: Arc<Document>,
    key: u64,
}

/// Opens the dialog over `options` and records its session.
fn start(app: &mut PhotocraftApp, idx: usize, document: DocId, target: Target, options: PatternFillOptions) -> DialogId {
    app.pattern_fill.opens += 1;
    let serial = app.pattern_fill.opens;
    let ctx = app.ctx().clone();
    let swatch_textures: Vec<egui::TextureHandle> =
        app.session.patterns.items.iter().map(|p| crate::panels::preset_browser::pattern_texture_handle(&ctx, p)).collect();
    let swatches: Vec<PatternSwatch> = app
        .session
        .patterns
        .items
        .iter()
        .zip(&swatch_textures)
        .map(|(p, texture)| PatternSwatch::new(p.id.clone(), p.display_name()).with_picture(texture.id()))
        .collect();
    let dialog = PatternFillDialog::new(options.clone(), swatches).canvas(CanvasHost::primary(app, idx).canvas_id());
    let id = app.framework.dialogs.open(dialog);
    app.pattern_fill.session = Some(Session { serial, document, target, opened: options, preview: None, _swatch_textures: swatch_textures });
    id
}

/// Edit the active Pattern Fill layer (its thumbnail, Layer › Layer Content Options…).
pub(crate) fn open(app: &mut PhotocraftApp) -> Option<DialogId> {
    let idx = app.session.active_index()?;
    let st = app.session.active()?;
    let layer = st.active_layer?;
    let LayerContent::Fill(Fill::Pattern { scale, id, angle, link, phase, .. }) = &st.doc.layer(layer)?.content else { return None };
    let mut options = PatternFillOptions::bitmap(id.clone());
    options.scale = f64::from(*scale) * 100.0;
    options.angle = f64::from(*angle);
    options.link = *link;
    options.phase = [f64::from(phase.0), f64::from(phase.1)];
    let document = st.doc.id;
    Some(start(app, idx, document, Target::Edit(layer), options))
}

/// Layer › New Fill Layer › Pattern…: the dialog in front of creating the layer, starting from
/// the Patterns panel's pattern at 100 %, 0° and linked, as in Photoshop.
pub(crate) fn open_new(app: &mut PhotocraftApp) -> Result<DialogId, String> {
    if let Some(Err(why)) = photocraft_engine::commands::find(cmds::CREATE).map(|c| (c.enabled)(&app.session)) {
        return Err(why);
    }
    let idx = app.session.active_index().ok_or("no document open")?;
    let st = app.session.active().ok_or("no document open")?;
    let (document, insert) = (st.doc.id, st.active_layer);
    let pattern = photocraft_engine::pattern_cmds::resolve_id(&app.session, "").ok_or("no patterns")?;
    // The command inserts above the active layer. It is frozen here so a later layer change cannot
    // silently move the insertion point.
    Ok(start(app, idx, document, Target::Create { insert }, PatternFillOptions::bitmap(pattern)))
}

/// The command and its params: the target plus the values. An edit sends only the keys that differ from
/// the opened ones, so an imported scale or angle outside the dialog's range does not make an unrelated
/// phase edit fail the strict `SET`; a new layer sends them all.
fn command(app: &PhotocraftApp, session: &Session, options: &PatternFillOptions) -> Option<(&'static str, Value)> {
    let (cmd, mut params) = match session.target {
        Target::Edit(layer) => (cmds::SET, json!({"layer": layer.0, "document": session.document.0})),
        Target::Create { .. } => (cmds::CREATE, json!({})),
    };
    let create = cmd == cmds::CREATE;
    let o = &session.opened;
    if create || options.pattern != o.pattern {
        params["pattern"] = json!(options.pattern);
    }
    if create || options.scale != o.scale {
        params["scale"] = json!(options.scale);
    }
    if create || options.angle != o.angle {
        params["angle"] = json!(options.angle);
    }
    if create || options.link != o.link {
        params["link"] = json!(options.link);
    }
    if create || options.phase != o.phase {
        params["phase"] = json!([options.phase[0], options.phase[1]]);
    }
    // A new fill layer is masked by the path selected in the Paths panel, as when it runs.
    let params = if create { crate::vector_ui::with_active_path(app, cmd, params) } else { params };
    Some((cmd, params))
}

/// An edit dialog whose values are still the layer's.
fn unchanged(session: &Session, options: &PatternFillOptions) -> bool {
    matches!(session.target, Target::Edit(_)) && *options == session.opened
}

/// The document the dialog targets is still the active one.
fn target_is_active(app: &PhotocraftApp, session: &Session) -> bool {
    app.session.active().is_some_and(|st| st.doc.id == session.document)
}

/// A new fill inserts above the active layer frozen when the dialog opened; an edit has no insertion.
fn insertion_unchanged(app: &PhotocraftApp, session: &Session) -> bool {
    match session.target {
        Target::Edit(_) => true,
        Target::Create { insert } => app.session.active().is_some_and(|st| st.active_layer == insert),
    }
}

/// The dialog's target document and insertion layer are still the ones it opened on.
fn context_ok(app: &PhotocraftApp, session: &Session) -> bool {
    target_is_active(app, session) && insertion_unchanged(app, session)
}

/// What a linked canvas drag moves: the Pattern Fill layer being edited, or the layer a new fill
/// would be inserted above. A gesture keeps the target it started on, so a changed target (another
/// layer, another pattern, unlinking) ends it instead of moving something else.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DragTarget {
    document: DocId,
    kind: Target,
    pattern: String,
    layer: Option<LayerId>,
}

/// The target a canvas drag moves, when the open dialog is a linked Pattern Fill that can move one:
/// the document is active, the phase is within what the engine accepts, and (for an edit) the layer
/// still exists, is a linked Pattern Fill and is not pixel-locked, directly or through a group. A new
/// fill may be inserted above a locked layer, as the command allows, but its insertion layer must
/// still be the one the dialog opened on.
pub(crate) fn drag_target(app: &PhotocraftApp) -> Option<DragTarget> {
    let session = app.pattern_fill.session.as_ref()?;
    let options = app.framework.dialogs.get::<PatternFillDialog>()?.current();
    if !context_ok(app, session) || !options.phase.iter().all(|v| v.is_finite() && v.abs() <= MAX_PHASE) || !options.link {
        return None;
    }
    let st = app.session.active()?;
    let layer = match session.target {
        Target::Edit(id) => {
            let LayerContent::Fill(Fill::Pattern { .. }) = &st.doc.layer(id)?.content else { return None };
            let locks = st.doc.effective_locks(id);
            if locks.pixels || locks.all {
                return None;
            }
            Some(id)
        }
        Target::Create { .. } => st.active_layer,
    };
    Some(DragTarget { document: session.document, kind: session.target, pattern: options.pattern.clone(), layer })
}

/// A held pattern drag, kept by the canvas tool between press and release. The phase is `base` plus the
/// pointer's travel since `from`, so repeated moves neither accumulate nor drift and a drag back
/// restores the phase exactly.
pub(crate) struct PhaseDrag {
    target: DragTarget,
    base: [f64; 2],
    from: [f64; 2],
    shown: [f64; 2],
    last: [f64; 2],
}

fn dialog_phase(app: &PhotocraftApp) -> Option<[f64; 2]> {
    app.framework.dialogs.get::<PatternFillDialog>().map(|d| d.current().phase)
}

/// A press at document point `at` starts a drag from the phase the dialog shows; `None` when the dialog
/// has no draggable target.
pub(crate) fn press(app: &PhotocraftApp, at: [f64; 2]) -> Option<PhaseDrag> {
    if !at.iter().all(|v| v.is_finite()) {
        return None;
    }
    let target = drag_target(app)?;
    let shown = dialog_phase(app)?;
    Some(PhaseDrag { target, base: shown, from: at, shown, last: at })
}

/// The pointer is at document point `at`. Returns `false` when the gesture is over (the target changed;
/// nothing is written). A phase the dialog shows other than the one this drag wrote (Snap to Origin, an
/// automation edit) is kept and the drag continues from it. The phase comes from the dialog and the drag's
/// own state only, so a second pass in one frame writes the same value again.
pub(crate) fn drag(app: &mut PhotocraftApp, gesture: &mut PhaseDrag, at: [f64; 2]) -> bool {
    if !at.iter().all(|v| v.is_finite()) || drag_target(app).as_ref() != Some(&gesture.target) {
        return false;
    }
    let Some(current) = dialog_phase(app) else { return false };
    if current != gesture.shown {
        gesture.base = current;
        gesture.from = gesture.last;
    }
    let phase = [0, 1].map(|i| (gesture.base[i] + (at[i] - gesture.from[i])).clamp(-MAX_PHASE, MAX_PHASE));
    if phase != current
        && let Some(dialog) = app.framework.dialogs.get_mut::<PatternFillDialog>()
    {
        dialog.set_phase(phase);
    }
    gesture.shown = phase;
    gesture.last = at;
    true
}

/// The button came up at `at`: the motion since the last drag applies, then the gesture is over.
pub(crate) fn release(app: &mut PhotocraftApp, mut gesture: PhaseDrag, at: [f64; 2]) {
    drag(app, &mut gesture, at);
}

/// The host's result of the Pattern Fill dialog: Cancel drops the session, OK commits it.
pub(crate) fn take(app: &mut PhotocraftApp, _ctx: &egui::Context) {
    let Some(result) = app.framework.dialogs.take::<PatternFillDialog>() else { return };
    let session = app.pattern_fill.session.take();
    if let DialogResult::Ok(options) = result
        && let Some(session) = session
        && let Err(why) = commit(app, &session, &options)
    {
        app.ui.status = why;
        app.ui.status_error = true;
    }
}

/// OK: one engine command. A stale document or insertion layer never receives the edit.
fn commit(app: &mut PhotocraftApp, session: &Session, options: &PatternFillOptions) -> Result<(), String> {
    if !target_is_active(app, session) {
        return Err("the target document is no longer active".into());
    }
    if !insertion_unchanged(app, session) {
        return Err("the layer the new fill would go above changed; reopen Pattern Fill".into());
    }
    if unchanged(session, options) {
        return Ok(());
    }
    let (cmd, params) = command(app, session, options).ok_or("invalid Pattern Fill dialog")?;
    app.run(cmd, params).map(|_| ())
}

/// The canvas preview while the dialog is open: the document as OK would leave it.
pub(crate) fn display_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    let options = app.framework.dialogs.get::<PatternFillDialog>()?.current().clone();
    let session = app.pattern_fill.session.as_ref()?;
    let st = app.session.documents().get(idx)?;
    if !context_ok(app, session) || app.session.active_index() != Some(idx) || unchanged(session, &options) {
        return None;
    }
    let (cmd, params) = command(app, session, &options)?;
    let (document, revision, serial) = (st.doc.id, st.revision, session.serial);
    if let Some(p) = &session.preview
        && p.document == document
        && p.revision == revision
        && p.serial == serial
        && p.params == params
    {
        return Some((p.shown.clone(), p.key));
    }
    // Out-of-range values typed by automation show the committed document; OK reports them.
    let doc = cmds::preview(&app.session, cmd, &params).ok()?;
    // The serial keeps a reopened dialog at the same revision from reusing an old preview's pixels in the canvas caches.
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    (document.0, revision, serial, params.to_string()).hash(&mut hash);
    let key = (1 << 47) | (hash.finish() & ((1 << 36) - 1));
    let shown = Arc::new(photocraft_engine::mode_cmds::display_document(&doc).unwrap_or(doc));
    app.pattern_fill.session.as_mut()?.preview = Some(Preview { document, revision, serial, params, shown: shown.clone(), key });
    Some((shown, key))
}

#[cfg(test)]
mod tests;
