//! The live Gradient tool on a layer mask (#10703). Photoshop's Gradient tool previews a gradient
//! drawn on a mask and keeps its start and end handles on the canvas, so where it starts and ends
//! can still change before it is painted into the mask's pixels. Here a drag previews the gradient
//! in the mask; afterwards the start and end handles move it, its line drags it whole and a drag
//! elsewhere draws it again. ↩, another tool or layer, or a command that edits the document paints
//! it (`paint.gradient`, one history step); Esc drops it, and so does an edit made under it (undo).

use std::sync::Arc;

use egui::{Color32, Stroke};
use photocraft_doc::{DocId, Document, LayerId};
use serde_json::{Value, json};

use super::{GRAB, Grab, dist, line, paint_params, project};
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, ViewXform};
use crate::state::Tool;

/// A gradient drawn on a mask and not painted into it yet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Pending {
    doc: DocId,
    layer: LayerId,
    /// The document's revision when it was drawn: an edit under it (an undo) drops it.
    revision: u64,
    from: [f32; 2],
    to: [f32; 2],
}

/// A drag drawing a new mask gradient (`grab` None) or moving the pending one's handles or line.
#[derive(Clone, Copy, Debug)]
struct Drag {
    /// The document and layer the press came on: a release on another (a tab or layer switched
    /// mid-drag) draws nothing.
    doc: DocId,
    layer: LayerId,
    grab: Option<Grab>,
    down: [f32; 2],
    /// The line when the press came, for dragging it whole.
    start: ([f32; 2], [f32; 2]),
    from: [f32; 2],
    to: [f32; 2],
}

/// The live mask gradient's state (not saved).
#[derive(Default)]
pub(crate) struct MaskGradient {
    /// At most one per document: a gradient drawn in another document waits there, it is not
    /// replaced.
    pending: Vec<Pending>,
    drag: Option<Drag>,
    /// Cached preview document by key.
    preview: Option<(u64, Arc<Document>)>,
}

impl MaskGradient {
    /// Whether a drag on the mask gradient is under way.
    pub(crate) fn dragging(&self) -> bool {
        self.drag.is_some()
    }
}

/// Commands before which a pending mask gradient is painted, as Photoshop applies it before
/// anything else changes the document: saving, editing pixels, layers, the selection, channels.
/// Zooming and panning keep it.
const COMMITTING: [&str; 10] = ["file.", "edit.", "image.", "layer.", "select.", "filter.", "channel.", "paint.", "type.", "path."];

/// Whether the Gradient tool works on a layer mask live: not Classic, the mask targeted, and no
/// gradient fill layer selected (selecting one targets its mask, and that is edited as a fill).
pub(crate) fn active(app: &PhotocraftApp) -> bool {
    app.active_tool() == Tool::Gradient
        && !app.ui.tool_options.gradient_classic
        && crate::canvas::paint_target(app).as_str() == Some("mask")
        && super::active_gradient(app).is_none()
}

/// The active document and layer, and the document's revision.
fn target(app: &PhotocraftApp) -> Option<(DocId, LayerId, u64)> {
    let st = app.session.active()?;
    Some((st.doc.id, st.active_layer?, st.revision))
}

/// The active document's pending gradient, whatever layer it is on.
fn pending_in_active(app: &PhotocraftApp) -> Option<Pending> {
    let doc = app.session.active()?.doc.id;
    app.gradient.mask.pending.iter().find(|q| q.doc == doc).copied()
}

/// The pending gradient, when it belongs to the active document and layer.
fn pending_here(app: &PhotocraftApp) -> Option<Pending> {
    let (_, layer, _) = target(app)?;
    pending_in_active(app).filter(|q| q.layer == layer)
}

/// Takes the active document's pending gradient out of the list.
fn take_active(app: &mut PhotocraftApp) -> Option<Pending> {
    let doc = app.session.active()?.doc.id;
    let i = app.gradient.mask.pending.iter().position(|q| q.doc == doc)?;
    Some(app.gradient.mask.pending.remove(i))
}

/// The line shown: the one being dragged, else the pending one.
fn shown(app: &PhotocraftApp) -> Option<([f32; 2], [f32; 2])> {
    match app.gradient.mask.drag {
        Some(d) => Some((d.from, d.to)),
        None => pending_here(app).map(|q| (q.from, q.to)),
    }
}

/// What `p` grabs on the line `from`→`to`, `tol` document pixels away at most.
fn hit(from: [f32; 2], to: [f32; 2], p: [f32; 2], tol: f32) -> Option<Grab> {
    if dist(p, to) <= tol {
        return Some(Grab::End);
    }
    if dist(p, from) <= tol {
        return Some(Grab::Start);
    }
    let (u, d) = project(from, to, p);
    (d <= tol).then_some(Grab::Line(u))
}

/// Pointer input on a mask (see [`active`]). Returns true when consumed.
pub(crate) fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers) -> bool {
    // ⌘ is the Move tool, as for the live gradient on pixels; a drag under way keeps its events.
    if app.gradient.mask.drag.is_none() && crate::canvas::event_tool(app, mods) != Tool::Gradient {
        return false;
    }
    let zoom = app.point_zoom().max(0.01);
    match ev {
        ToolEvent::Down { x, y, .. } => {
            let Some((doc, layer, _)) = target(app) else { return true };
            let p = [x as f32, y as f32];
            let pending = pending_here(app);
            let grab = pending.and_then(|q| hit(q.from, q.to, p, GRAB / zoom));
            app.gradient.mask.drag = Some(match (grab, pending) {
                (Some(grab), Some(q)) => Drag { doc, layer, grab: Some(grab), down: p, start: (q.from, q.to), from: q.from, to: q.to },
                _ => Drag { doc, layer, grab: None, down: p, start: (p, p), from: p, to: p },
            });
        }
        ToolEvent::Move { x, y, .. } => drag_to(app, [x as f32, y as f32], mods),
        ToolEvent::Up { x, y } => {
            drag_to(app, [x as f32, y as f32], mods);
            release(app);
        }
    }
    true
}

fn drag_to(app: &mut PhotocraftApp, p: [f32; 2], mods: egui::Modifiers) {
    let Some(d) = app.gradient.mask.drag.as_mut() else { return };
    // ⇧: 45° angles, as for every gradient drag.
    let snap = |from: [f32; 2], to: [f32; 2]| -> [f32; 2] {
        if !mods.shift {
            return to;
        }
        let c = crate::stroke_constraint::snap45([f64::from(from[0]), f64::from(from[1])], [f64::from(to[0]), f64::from(to[1])]);
        [c[0] as f32, c[1] as f32]
    };
    match d.grab {
        None | Some(Grab::End) => d.to = snap(d.from, p),
        Some(Grab::Start) => d.from = snap(d.to, p),
        Some(_) => {
            let (dx, dy) = (p[0] - d.down[0], p[1] - d.down[1]);
            d.from = [d.start.0[0] + dx, d.start.0[1] + dy];
            d.to = [d.start.1[0] + dx, d.start.1[1] + dy];
        }
    }
}

fn release(app: &mut PhotocraftApp) {
    let Some(d) = app.gradient.mask.drag.take() else { return };
    let Some((doc, layer, revision)) = target(app) else { return };
    // A tab or layer switched mid-drag: the release is not on what the press drew on.
    if (doc, layer) != (d.doc, d.layer) {
        return;
    }
    let zoom = app.point_zoom().max(0.01);
    // A click draws nothing and leaves a pending gradient as it is. A new line is as long as the
    // preview needs to show it ([`display_doc`]), so nothing is painted that was never shown.
    let moved = match d.grab {
        None => dist(d.from, d.to) * zoom >= 2.0 && dist(d.from, d.to) >= MIN_SPAN,
        Some(_) => d.from != d.start.0 || d.to != d.start.1,
    };
    if moved {
        let q = Pending { doc, layer, revision, from: d.from, to: d.to };
        let list = &mut app.gradient.mask.pending;
        match list.iter_mut().find(|p| p.doc == doc) {
            Some(slot) => *slot = q,
            None => list.push(q),
        }
    }
}

/// The shortest line, in document pixels, a mask gradient is previewed and painted for.
const MIN_SPAN: f32 = 1.0;

/// `paint.gradient`'s params for line `from`→`to` on the mask of `layer`.
fn params(app: &PhotocraftApp, layer: LayerId, from: [f32; 2], to: [f32; 2]) -> Value {
    let mut p = paint_params(app, [f64::from(from[0]), f64::from(from[1])], [f64::from(to[0]), f64::from(to[1])]);
    p["layer"] = json!(layer.0);
    p["target"] = json!("mask");
    p
}

/// Paints the active document's pending gradient into its mask (one history step); one pending
/// in another document waits there. A failure is reported in the status bar, drops it and is
/// returned.
pub(crate) fn commit(app: &mut PhotocraftApp) -> Result<(), String> {
    app.gradient.mask.drag = None;
    let Some(q) = take_active(app) else { return Ok(()) };
    let p = params(app, q.layer, q.from, q.to);
    app.run("paint.gradient", p).map(|_| ()).inspect_err(|e| {
        app.ui.status = e.clone();
        app.ui.status_error = true;
    })
}

/// Drops the active document's pending gradient without painting it.
pub(crate) fn cancel(app: &mut PhotocraftApp) {
    app.gradient.mask.drag = None;
    take_active(app);
    app.gradient.mask.preview = None;
}

/// Undo and Redo step the history instead of editing on top of it: painting the pending gradient
/// first would put it in the step being undone, and the undo would swallow it (#10703 review).
const STEPPING: [&str; 2] = ["edit.undo", "edit.redo"];

/// Before command `id` runs: paints the active document's pending gradient first when the
/// command edits the document. Recursion is not a worry — [`commit`] runs `paint.gradient` one
/// level down, where the caller no longer asks — so the Classic Gradient commits the pending one
/// before laying its own. A command the engine refuses (bad params, not enabled, a hidden target)
/// changes nothing: the gradient stays pending. A gradient that can't be painted refuses the
/// command, with its reason, rather than let it run over a gradient that was lost.
pub(crate) fn before_command(app: &mut PhotocraftApp, id: &str, params: &Value) -> Result<(), String> {
    if pending_in_active(app).is_none() || STEPPING.contains(&id) || !COMMITTING.iter().any(|p| id.starts_with(p)) {
        return Ok(());
    }
    if photocraft_engine::commands::find(id).is_some() {
        app.session.precheck(id, params).map_err(|e| e.to_string())?;
    }
    commit(app)
}

/// Once a frame: paints the pending gradient when the tool, the target or the layer changed, and
/// drops it when its document changed under it (an undo, a history step chosen).
pub(crate) fn tick(app: &mut PhotocraftApp) {
    // A closed document takes its pending gradient with it.
    if !app.gradient.mask.pending.is_empty() {
        let open: Vec<DocId> = app.session.documents().iter().map(|st| st.doc.id).collect();
        app.gradient.mask.pending.retain(|q| open.contains(&q.doc));
    }
    let Some(q) = pending_in_active(app) else { return };
    let Some((_, layer, revision)) = target(app) else { return };
    if revision != q.revision {
        cancel(app);
    } else if app.gradient.mask.drag.is_none() && (!active(app) || layer != q.layer) {
        // The status bar says why, if it can't be painted.
        let _ = commit(app);
    }
}

/// ↩ paints the pending gradient, Esc drops it. Returns true when a key was used.
pub(crate) fn keys(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    if pending_here(app).is_none() {
        return false;
    }
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
        // The status bar says why, if it can't be painted.
        let _ = commit(app);
        return true;
    }
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        cancel(app);
        return true;
    }
    false
}

/// The document to show while a mask gradient is drawn or pending: the mask with it painted in.
pub(crate) fn display_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    if app.session.active_index() != Some(idx) || !active(app) {
        app.gradient.mask.preview = None;
        return None;
    }
    let Some((from, to)) = shown(app).filter(|(f, t)| dist(*f, *t) >= MIN_SPAN) else {
        app.gradient.mask.preview = None;
        return None;
    };
    let (_, layer, revision) = target(app)?;
    let p = params(app, layer, from, to);
    // The colours the gradient takes from outside its params: the foreground and background, and
    // the current gradient (Gradients panel), so a change to either shows at once.
    let (tools, presets) = (&app.session.tools, &app.session.presets);
    let colours = format!("{:?}{:?}{:?}", tools.foreground, tools.background, presets.gradient);
    let key = super::preview_key(revision, &format!("mask{p}{colours}"));
    if let Some((k, d)) = &app.gradient.mask.preview
        && *k == key
    {
        return Some((d.clone(), key));
    }
    let mut shown = (*app.session.active()?.doc).clone();
    photocraft_engine::paint_cmds::gradient_preview(&app.session, &mut shown, &p).ok()?;
    let shown = Arc::new(shown);
    app.gradient.mask.preview = Some((key, shown.clone()));
    Some((shown, key))
}

/// Draws the mask gradient's line and its start and end handles over the canvas.
pub(crate) fn draw_overlay(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    if !active(app) {
        return;
    }
    let Some((from, to)) = shown(app) else { return };
    let (a, b) = (xf.to_screen(from[0], from[1]), xf.to_screen(to[0], to[1]));
    line(painter, a, b);
    for c in [a, b] {
        painter.circle_filled(c, 5.0, Color32::WHITE);
        painter.circle_stroke(c, 5.0, Stroke::new(1.5, Color32::from_black_alpha(160)));
    }
}

#[cfg(test)]
pub(crate) fn pending(app: &PhotocraftApp) -> Option<([f32; 2], [f32; 2])> {
    pending_in_active(app).map(|q| (q.from, q.to))
}
