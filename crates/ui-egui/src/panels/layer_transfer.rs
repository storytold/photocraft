//! Dragging layers to another document (#589), as in Photoshop: drag layers from the Layers panel,
//! or with the Move tool from the canvas, onto another document's tab. The tab shows its document
//! while the pointer is over it (spring-loaded), and releasing over that document's canvas copies
//! the layers there with `layer.copyToDocument` (one history step in the destination).
//!
//! Where the copies land: a Move-tool drag puts the pixel that was grabbed under the pointer, and
//! a Layers panel drag centres the layers on it. With ⇧, or when released on the tab itself, they
//! keep their place when both documents have the same pixel size and are centred otherwise
//! (Photoshop's ⇧-drag). With Window › Arrange tiling several documents, releasing over another
//! document's tile drops there.
//!
//! A Move drag with a pixel selection copies only those pixels into a new layer.

use photocraft_doc::{DocId, LayerId};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// The layers being dragged and where from.
#[derive(Clone, Debug)]
struct Transfer {
    source: DocId,
    layers: Vec<u64>,
    /// The document point a Move-tool drag grabbed (`None` for a Layers panel drag).
    grab: Option<[f64; 2]>,
    /// What the ghost by the pointer says.
    label: String,
    selection_only: bool,
    /// The layer's mask is the content that travels (`"target": "mask"`), not its pixels.
    mask: bool,
    /// The viewport the transfer was armed in: only that viewport completes it, so a document
    /// window's `finish` cannot clear a transfer the main window is still dragging.
    viewport: egui::ViewportId,
}

fn key() -> egui::Id {
    egui::Id::unique("pc-layer-transfer")
}

fn get(ctx: &egui::Context) -> Option<Transfer> {
    ctx.data(|d| d.get_temp::<Transfer>(key()))
}

fn clear(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove::<Transfer>(key()));
}

/// The layers `id` stands for: the selection when it is one of the selected layers, else itself.
fn begin(app: &PhotocraftApp, ctx: &egui::Context, id: LayerId, grab: Option<[f64; 2]>) -> bool {
    let Some(st) = app.session.active() else { return false };
    let layers = if st.is_layer_selected(id) { st.selected_layers() } else { vec![id] };
    let label = match layers.as_slice() {
        [one] => st.doc.layer(*one).map_or_else(String::new, |l| l.name.clone()),
        many => crate::i18n::trn(crate::i18n::current(), many.len() as u64, "{n} layer", "{n} layers"),
    };
    // Several selected layers move whole; the engine's selected-pixel copy intentionally accepts
    // exactly one raster layer.
    let selection_only = grab.is_some() && layers.len() == 1 && st.doc.selection.is_some();
    // A mask target travels as the mask, not as the layer's pixels: the mask's own selection
    // is what the pointer grabbed.
    let mask = selection_only && st.floating.as_ref().is_some_and(|f| f.parts.is_mask()) || selection_only && app.layer_mask_targeted();
    let t = Transfer { source: st.doc.id, layers: layers.iter().map(|l| l.0).collect(), grab, label, selection_only, mask, viewport: ctx.viewport_id() };
    ctx.data_mut(|d| d.insert_temp(key(), t));
    true
}

/// A Layers panel row drag began on `layer`: it can be dropped on another document.
pub fn begin_from_panel(app: &PhotocraftApp, ctx: &egui::Context, layer: LayerId) {
    begin(app, ctx, layer, None);
}

/// A Move-tool drag of whole layers (not a selection or a floating piece) under way.
fn move_drag(app: &PhotocraftApp) -> Option<[f64; 2]> {
    if let Some(d) = app.drag.as_ref().filter(|d| d.tool == Tool::Move && d.sel_move != Some(false)) {
        let st = app.session.active()?;
        let plain =
            app.ui.transform.is_none() && app.guide_drag.is_none() && ((st.doc.selection.is_none() && st.floating.is_none()) || d.sel_move == Some(true));
        return plain.then_some(d.start);
    }
    // A press that found nothing movable starts a marquee; when that marquee grabbed a layer
    // (a locked layer or the Background), the marquee can still become a transfer of it.
    // Likewise, a blocked Move press (Auto-Select off) recorded its start point for transfer.
    let st = app.session.active()?;
    if app.move_blocked
        && let Some(start) = app.move_blocked_start
        && app.ui.transform.is_none()
        && app.guide_drag.is_none()
        && st.floating.is_none()
        && (app.move_blocked_layer.is_some() || (st.doc.selection.is_some() && st.active_layer.is_some()))
    {
        return Some(start);
    }
    if st.doc.selection.is_some() || st.floating.is_some() {
        return None;
    }
    app.layer_box.as_ref().filter(|b| b.grabbed_layer.is_some()).map(|b| b.start)
}

/// The pointer, while layers are dragged (or a Move drag could become a transfer): tabs only
/// hit-test it then.
pub fn pointer_if_armed(app: &PhotocraftApp, ctx: &egui::Context) -> Option<egui::Pos2> {
    if get(ctx).is_none() && move_drag(app).is_none() {
        return None;
    }
    ctx.input(|i| i.pointer.latest_pos())
}

/// The pointer is over document tab `index` while layers are dragged: a Move drag becomes a
/// transfer (the source layers stay where they were), the tab's document is shown, and a release
/// right here drops the layers into it.
fn arm_move_transfer(app: &mut PhotocraftApp, ctx: &egui::Context, index: usize) -> bool {
    if get(ctx).is_none() {
        let Some(grab) = move_drag(app) else { return false };
        // A marquee that grabbed a layer transfers it even when no layer is active.
        let active = app.session.active().and_then(|st| st.active_layer);
        if app.session.active_index() == Some(index) {
            return false;
        }
        // A marquee that grabbed a layer or a blocked Move press has no drag object and no
        // floating piece to take back: the grabbed or hit layer is what transfers.
        if app.drag.is_none() {
            if app.move_viewport.is_some_and(|v| v != ctx.viewport_id()) {
                return false;
            }
            let grabbed = if let Some(b) = app.layer_box.as_ref() {
                let Some(g) = b.grabbed_layer else { return false };
                app.layer_box = None;
                app.move_viewport = None;
                g
            } else if app.move_blocked {
                let Some(hit) = app.move_blocked_layer.or_else(|| active.filter(|_| app.session.active().is_some_and(|st| st.doc.selection.is_some()))) else {
                    return false;
                };
                app.clear_move_blocked();
                hit
            } else {
                return false;
            };
            return begin(app, ctx, grabbed, Some(grab));
        }
        let Some(previous) = app.drag.as_ref().map(|d| d.transfer_floating_before.clone()) else { return false };
        // An Option-drag of an existing float first drops it and floats a copy. Take that
        // compound start back before restoring the captured float, so document, history and
        // floating revision remain one consistent source state.
        if previous.as_ref().is_some_and(|before| app.session.active().is_some_and(|st| st.revision != before.revision)) {
            // The drag's own float is gone or stale: another edit (an opacity key, say) came
            // after the drag started, and a rollback would take that edit back instead. The
            // drag stays a move in its own document.
            if app.session.active().and_then(photocraft_engine::float_cmds::floating).is_none() {
                return false;
            }
            // Nothing is taken from the drag before this can fail (a background job holds the
            // history): it goes on as it was, with its ⇧ lock and offset (`move_mods`).
            if !app.session.rollback() {
                return false;
            }
        }
        // The Move drag ends without moving anything; an ⌥-drag's copy is taken back.
        app.drag = None;
        app.move_preview = None;
        crate::move_mods::abandon(app);
        // The rollback raised the revision; the restored document is the one the float was
        // over, so it floats over the new revision (else `floating()` drops it as stale).
        if let Some(st) = app.session.active_mut() {
            st.floating = previous.map(|f| photocraft_engine::float_cmds::Floating { revision: st.revision, ..f });
        }
        let Some(active) = app.session.active().and_then(|st| st.active_layer).or(active) else { return false };
        if !begin(app, ctx, active, Some(grab)) {
            return false;
        }
    }
    true
}

/// Handles a layer transfer dragged over document tab `index`, switching the active document
/// and completing the drop when the primary button is released.
pub fn over_tab(app: &mut PhotocraftApp, ctx: &egui::Context, index: usize) {
    if !arm_move_transfer(app, ctx, index) {
        return;
    }
    if app.session.active_index() != Some(index) {
        app.session.set_active(index);
        app.jobs.focus = None;
        // The Layers panel now lists the other document: its rows don't take the drag.
        ctx.data_mut(|d| d.remove::<u64>(egui::Id::unique("layer-drag")));
    }
    if ctx.input(|i| i.pointer.primary_released()) {
        drop(app, ctx, index, None);
    }
}

/// Every frame after the document area: a release over another document's canvas drops the
/// layers there; while the button is held a ghost follows the pointer.
pub fn finish(app: &mut PhotocraftApp, ctx: &egui::Context) {
    // The primary button ends the gesture: a secondary release during a drag leaves it held.
    let (primary_released, primary_down) = ctx.input(|i| (i.pointer.primary_released(), i.pointer.primary_down()));
    // A blocked Move press is cleaned up by the window that owns it, so another window's
    // pointer state cannot clear the source window's gesture.
    if (!primary_down || primary_released) && get(ctx).is_none() && app.move_viewport.is_none_or(|v| v == ctx.viewport_id()) {
        let show_lock = app.move_blocked && app.move_blocked_moved;
        app.clear_move_blocked();
        if show_lock {
            crate::move_lock::prompt(app);
        }
        if let Some(b) = app.layer_box.as_ref()
            && b.grabbed_layer.is_some()
        {
            app.layer_box = None;
        }
    }
    let Some(t) = get(ctx) else { return };
    // Only the viewport that armed the transfer completes it: a document window's pointer state
    // must not clear a transfer the main window is still dragging.
    if t.viewport != ctx.viewport_id() {
        return;
    }
    let (pos, released, down) = ctx.input(|i| (i.pointer.latest_pos(), i.pointer.primary_released(), i.pointer.primary_down()));
    let target = pos.and_then(|p| target_at(app, p));
    if !released {
        if !down {
            clear(ctx);
        } else if let Some(p) = pos
            && app.session.active().is_some_and(|st| st.doc.id != t.source)
        {
            ghost(ctx, p, &t.label);
            if target.is_some() {
                ctx.set_cursor_icon(egui::CursorIcon::Copy);
            }
        }
        return;
    }
    if let Some((dest, at)) = target {
        drop(app, ctx, dest, at);
    }
    clear(ctx);
}

/// Is a cross-document transfer currently armed? The tool events have no context of their
/// own, so the frame's stored context answers it.
pub fn is_transfer_in_progress(app: &PhotocraftApp) -> bool {
    app.frame_tool_ctx.as_ref().is_some_and(|ctx| get(ctx).is_some())
}

/// Whether a transfer is armed in `ctx` (any viewport).
#[cfg(test)]
pub fn is_transfer_armed(ctx: &egui::Context) -> bool {
    get(ctx).is_some()
}

/// Run before canvases process release: a tiled destination must take over the
/// drag before the source can commit it as an ordinary move.
pub(crate) fn prepare(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if get(ctx).is_some() {
        return;
    }
    // Do not consume a source drag merely because it passes over another tile. On release this
    // runs before canvas input, so a destination can take ownership without the source committing;
    // returning to the source still releases as the original move.
    if !ctx.input(|i| i.pointer.primary_released()) {
        return;
    }
    let Some(pos) = pointer_if_armed(app, ctx) else { return };
    let Some((dest, _)) = target_at(app, pos) else { return };
    if app.session.active_index() != Some(dest) && arm_move_transfer(app, ctx, dest) {
        app.session.set_active(dest);
        app.jobs.focus = None;
    }
}

/// The document under screen point `p` and the document point there. Arranged tiles use their
/// own view transform, so selected pixels and plain layers both land at the visible drop point.
fn target_at(app: &PhotocraftApp, p: egui::Pos2) -> Option<(usize, Option<[f64; 2]>)> {
    if !app.last_canvas_rect.contains(p) {
        return None;
    }
    match crate::canvas::arranged_cells(app, app.last_canvas_rect) {
        Some(cells) => cells.into_iter().find(|(_, r)| r.contains(p)).and_then(|(d, r)| {
            let view = app.ui.views.get(d)?;
            let ppp = app.canvas_ppp();
            let xf = crate::canvas::ViewXform {
                rect: crate::rulers::content_rect(app, r.shrink(1.0)),
                zoom: view.zoom / ppp,
                center: view.center,
                flip: app.ui.view.flip_horizontal,
                rotation: view.rotation,
                aspect: app.ui.view.display_aspect(),
            }
            .pixel_aligned_for(app, ppp);
            Some((d, Some(xf.to_doc(p))))
        }),
        None => Some((app.session.active_index()?, crate::canvas::ViewXform::active(app).map(|xf| xf.to_doc(p)))),
    }
}

/// Copy the dragged layers into document `dest`, at document point `at` (see the module docs).
fn drop(app: &mut PhotocraftApp, ctx: &egui::Context, dest: usize, at: Option<[f64; 2]>) {
    let Some(t) = get(ctx) else { return };
    clear(ctx);
    let docs = app.session.documents();
    let Some(src) = docs.iter().position(|d| d.doc.id == t.source) else { return };
    if dest == src
        && let Some(at) = at
    {
        if let Some(frame) = crate::frame_tool::frame_at(app, at)
            && let [source] = t.layers.as_slice()
        {
            let r = app.run("layer.frame.setContent", json!({"layer":frame.0,"content":source}));
            if let Err(e) = r {
                app.ui.status = e;
                app.ui.status_error = true;
            }
        }
        return;
    }
    let Some(to) = docs.get(dest).filter(|_| dest != src) else { return };
    let same_size = to.doc.size == docs[src].doc.size;
    let mut p = json!({"document": dest, "source": src, "layers": t.layers});
    if t.selection_only {
        p["selection"] = json!(true);
        p["cut"] = json!(false);
        if t.mask {
            p["target"] = json!("mask");
        }
    } else {
        p["selectionOnly"] = json!(false);
    }
    match at.filter(|_| !ctx.input(|i| i.modifiers.shift)) {
        Some(at) => match t.grab {
            Some(g) => p["offset"] = json!([(at[0] - g[0]).round(), (at[1] - g[1]).round()]),
            None => p["at"] = json!(at),
        },
        None => p["center"] = json!(!same_size),
    }
    if let Err(e) = app.run("layer.copyToDocument", p) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// The dragged layers' name next to the pointer (also the Layers panel's reorder drag).
pub fn ghost(ctx: &egui::Context, p: egui::Pos2, label: &str) {
    let t = crate::theme::Tokens::get(ctx);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("layer-transfer-ghost")));
    let g = painter.layout_no_wrap(label.to_string(), egui::FontId::proportional(12.0), t.text);
    let r = egui::Rect::from_min_size(p + egui::vec2(12.0, -10.0), g.size() + egui::vec2(16.0, 8.0));
    painter.rect_filled(r, t.radius_sm, t.card.gamma_multiply(0.95));
    painter.rect_stroke(r, t.radius_sm, egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
    painter.galley(r.min + egui::vec2(8.0, 4.0), g, t.text);
}

#[cfg(test)]
mod tests;
