//! Move tool drags shown live (#128): while the pointer drags, the canvas shows the document with
//! the moving layers already at the pointer, exactly as `layer.translate` will leave them, and
//! recomposites only where they were and where they are. Releasing commits one `layer.translate`
//! (one history step) whose damage rect refreshes the same area.
//!
//! With a selection, the Move tool drags the selected pixels instead (#578): they float
//! (`select.float`, the canvas shows the piece at the pointer) until ↩, Deselect or any other
//! command drops them, as one "Move" history step.

use std::sync::Arc;

use photocraft_doc::{DocId, Document, LayerId};
use photocraft_engine::channel_cmds::ChannelTarget;
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::{Tool, TransformSession};

/// Does the Move tool drag (and do the arrow keys nudge) the selected pixels of the active layer
/// rather than the whole layer, with `tool` in effect? With a selection, as in Photoshop; a layer
/// whose pixels can't float (type, shape, Smart Object, group, position-locked), a hidden layer or
/// several selected layers still move whole, as without one. The canvas cursor asks before the
/// press, and a pointer event passes `canvas::event_tool`, which also sees a ⌘ sent with the event
/// (#2768).
pub(crate) fn moves_selected_pixels_with(app: &PhotocraftApp, tool: Tool) -> bool {
    let Some(st) = app.session.active() else { return false };
    tool == Tool::Move
        && app.ui.transform.is_none()
        && st.doc.selection.is_some()
        && st.selected_layers().len() <= 1
        && st.active_layer.and_then(|id| st.doc.layer(id)).is_some_and(|l| l.visible)
        && app.session.is_enabled("select.float")
}

/// Floats the selected pixels and moves them by (dx, dy) (`select.float`): cuts them the first
/// time, or with `copy` (⌥) floats a duplicate and leaves them; `copy` on a piece already floating
/// drops it where it is and floats a copy of it. False when it failed (reported in the status bar).
pub(crate) fn float_selected(app: &mut PhotocraftApp, copy: bool, dx: f64, dy: f64) -> bool {
    let floating = app.session.active().is_some_and(|st| photocraft_engine::float_cmds::floating(st).is_some());
    let dropped = if copy && floating { app.run("select.drop", json!({})) } else { Ok(json!(null)) };
    match dropped.and_then(|_| app.run("select.float", json!({"dx": dx, "dy": dy, "copy": copy}))) {
        Ok(_) => {
            crate::snap_ui::follow_cut(app);
            true
        }
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
            false
        }
    }
}

/// Preview keys of move drags: `BASE + n`, one per offset shown (see `canvas::display_doc`).
const BASE: u64 = 1 << 39;

pub(crate) struct MovePreview {
    doc: DocId,
    revision: u64,
    /// Layers that move ([`photocraft_engine::layer_multi_cmds::move_targets`]).
    ids: Vec<LayerId>,
    /// `ids` and every layer inside them: the layers shown at the drag's offset.
    moving: Vec<LayerId>,
    /// What moving them can change at offset (0, 0), not clipped to the canvas: a layer larger
    /// than the canvas brings its pixels from beyond the edge into view (`None` = anything).
    bounds: Option<Rect>,
    canvas: Rect,
    /// Offsets shown so far, by preview key (`BASE + index`).
    offsets: Vec<(i32, i32)>,
    /// The document at the latest offset.
    shown: Option<Arc<Document>>,
    /// Shows a floating selection (`select.float`): offsets are the piece's, from where it was cut.
    floating: bool,
}

/// An immutable, reduced source for a Move drag on one colour channel. The per-frame offset
/// changes only the affine shader's uniforms; the full-resolution edit still goes through
/// `layer.translate` on release. `gpu: None` caches an unsupported source for this snapshot.
pub(crate) struct MoveGpuPreview {
    doc: DocId,
    /// Eye changes bump the session revision without changing native pixels. Match the
    /// document snapshot so those changes only rerun the shader, never upload the source.
    snapshot: std::sync::Weak<Document>,
    layer: LayerId,
    channel: usize,
    background: [f32; 4],
    factor: u32,
    bounds: Rect,
    gpu: Option<crate::channel_transform_gpu::Preview>,
    shown: bool,
}

/// Use the existing native-component shader for a Move drag when the captured RGB stack has
/// a faithful GPU representation. Unsupported stacks, profiles and selections use the CPU.
pub(crate) fn channel_texture(app: &mut PhotocraftApp, idx: usize, output: Option<u32>) -> Option<egui::TextureId> {
    if app.session.active_index() != Some(idx) {
        return None;
    }
    let Some(offset) = drag_offset(app) else {
        app.move_gpu_preview = None;
        return None;
    };
    // A single GPU texture is shared by document windows. A window on another output can
    // require a different display encoding; let its CPU preview own that conversion.
    if output != app.session.color.main_display {
        return None;
    }
    if let Some(p) = app.move_gpu_preview.as_mut() {
        p.shown = false;
    }
    if offset == (0, 0) || app.ui.transform.is_some() || app.session.prefs().interface.show_bounding_box_when_dragging_layer || app.layer_mask_targeted() {
        return None;
    }
    let st = app.session.documents().get(idx)?;
    let ChannelTarget::Color(channel) = st.channel_view.target else { return None };
    let layer = st.active_layer?;
    if st.doc.selection.is_some() || photocraft_engine::float_cmds::floating(st).is_some() {
        return None;
    }
    let (doc, view) = (st.doc.clone(), st.channel_view.clone());
    let background = app.session.tools.background;
    let factor = if photocraft_compose::proxy::proxy_faithful(&doc) { crate::proxy::preview_factor(&doc, crate::proxy::reduced_previews(app)) } else { 1 };
    let (display, _) = crate::canvas::canvas_display(app, &doc, output);
    let fast_view = display.as_ref().is_none_or(|d| d.transform.is_none())
        && view.shown_layer_mask(&doc).is_none()
        && (0..doc.channels.len()).all(|i| !view.alpha_shown(i))
        && (doc.quick_mask.is_none() || view.quick_mask_hidden);
    if !fast_view {
        return None;
    }
    let gpu = app.gpu.as_ref().filter(|g| g.health().is_ok())?;
    let fresh = app.move_gpu_preview.as_ref().is_some_and(|p| {
        p.doc == doc.id
            && p.snapshot.ptr_eq(&Arc::downgrade(&doc))
            && p.layer == layer
            && p.channel == channel
            && p.background == background
            && p.factor == factor
    });
    if !fresh {
        let native_bounds = doc.layer(layer)?.surface()?.content_bounds();
        // The shader uploads the document only. An off-canvas source can move pixels in from
        // beyond that upload, so retain the CPU oracle for it.
        let (bounds, preview) = if native_bounds.is_empty() || !doc.bounds().contains_rect(&native_bounds) {
            (Rect::EMPTY, None)
        } else {
            let proxy = crate::proxy::proxy_document(&doc, factor);
            let bounds = proxy.layer(layer).and_then(|l| l.surface()).map_or(Rect::EMPTY, photocraft_raster::Surface::content_bounds);
            let preview = crate::channel_transform_gpu::Preview::new(gpu, &proxy, Some(layer), background);
            (bounds, preview)
        };
        app.move_gpu_preview =
            Some(MoveGpuPreview { doc: doc.id, snapshot: Arc::downgrade(&doc), layer, channel, background, factor, bounds, gpu: preview, shown: false });
    }
    let p = app.move_gpu_preview.as_mut()?;
    let rect = [p.bounds.x0, p.bounds.y0, p.bounds.x1, p.bounds.y1].map(|v| f64::from(v) * f64::from(factor));
    let quad =
        [[rect[0], rect[1]], [rect[2], rect[1]], [rect[2], rect[3]], [rect[0], rect[3]]].map(|[x, y]| [x + f64::from(offset.0), y + f64::from(offset.1)]);
    let t = TransformSession {
        session: 0,
        layer: layer.0,
        rect,
        quad,
        pivot: [(rect[0] + rect[2]) / 2.0, (rect[1] + rect[3]) / 2.0],
        interpolation: if offset.0.rem_euclid(factor as i32) == 0 && offset.1.rem_euclid(factor as i32) == 0 { "nearest" } else { "bilinear" }.into(),
        warp: None,
        selection: false,
        target: Some(json!({"color": channel})),
        path: None,
        made: None,
        mode: Default::default(),
        content_aware: None,
        skew_v: 0.0,
        layers: Vec::new(),
    };
    let texture = p.gpu.as_mut().and_then(|g| {
        g.texture_move(&t, &view, app.session.prefs().interface.show_channels_in_color, display.as_ref().is_some_and(|d| d.encode_srgb), factor, p.bounds)
    });
    p.shown = texture.is_some();
    if texture.is_some() {
        // A prior CPU frame (e.g. the Navigator's first frame) must not be composited too.
        app.move_preview = None;
    }
    texture
}

pub(crate) fn gpu_showing(app: &PhotocraftApp, idx: usize) -> bool {
    app.session.active_index() == Some(idx) && drag_offset(app).is_some() && app.move_gpu_preview.as_ref().is_some_and(|p| p.shown)
}

/// Closing/switching the document or cancelling a drag can bypass the normal pointer release.
pub(crate) fn cancel_stale_gpu(app: &mut PhotocraftApp) {
    if app.move_gpu_preview.as_ref().is_some_and(|p| drag_offset(app).is_none() || app.session.active().is_none_or(|st| st.doc.id != p.doc)) {
        app.move_gpu_preview = None;
    }
}

impl MovePreview {
    fn key(&self) -> u64 {
        BASE + self.offsets.len() as u64
    }
    fn offset_of(&self, key: u64) -> Option<(i32, i32)> {
        if key == 0 {
            return Some((0, 0));
        }
        let i = usize::try_from(key.checked_sub(BASE + 1)?).ok()?;
        self.offsets.get(i).copied()
    }
    fn area(&self, d: (i32, i32)) -> Option<Rect> {
        let b = self.bounds?;
        Some(if b.is_empty() { b } else { b.translate(d.0, d.1).inflate(1).intersect(&self.canvas) })
    }
}

/// The whole-pixel offset of the current Move drag on document `idx`, if one is under way.
fn drag_offset(app: &PhotocraftApp) -> Option<(i32, i32)> {
    let d = app.drag.as_ref().filter(|d| d.tool == Tool::Move)?;
    let end = d.points.last().map_or(d.start, |p| [p[0], p[1]]);
    let dx = (end[0] - d.start[0]).round().clamp(-1e7, 1e7) as i32;
    let dy = (end[1] - d.start[1]).round().clamp(-1e7, 1e7) as i32;
    Some((dx, dy))
}

/// The ΔX/ΔY values shown beside the pointer while the Move tool drags layers or selected pixels,
/// as in Photoshop (#5727): the whole pixels the release will move them. Free Transform shows its
/// own (`transform_tool::live_readout`). `None` until the pointer moves.
pub(crate) fn offset_readout(app: &PhotocraftApp) -> Option<[String; 2]> {
    app.drag.as_ref().filter(|d| d.points.len() > 1)?;
    let (dx, dy) = drag_offset(app)?;
    Some([dx, dy].map(|v| format!("{v} px")))
}

/// The document to show while a Move drag is under way on document `idx`: the moving layers at
/// the pointer. `None` without a drag (or when the layers can't move: locked, say; the drag then
/// shows its arrow and the release reports why).
pub(crate) fn display_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    if app.session.active_index() == Some(idx)
        && let Some(st) = app.session.documents().get(idx)
        && photocraft_engine::float_cmds::floating(st).is_some()
    {
        return floating_doc(app, idx);
    }
    // Preferences › Interface › Show bounding box when dragging layer: outline and arrow only.
    if app.session.active_index() != Some(idx) || app.session.prefs().interface.show_bounding_box_when_dragging_layer {
        return None;
    }
    let Some(offset) = drag_offset(app) else {
        app.move_preview = None;
        return None;
    };
    let mask_target = app.layer_mask_targeted();
    let background = app.session.tools.background;
    let st = app.session.documents().get(idx)?;
    let (doc_id, revision, doc) = (st.doc.id, st.revision, st.doc.clone());
    // One colour channel targeted (#6068): the drag moves that channel of the active layer, as
    // `layer.translate` will on release, and the layer itself stays.
    let channel = match st.channel_view.target {
        photocraft_engine::channel_cmds::ChannelTarget::Color(k) if !mask_target => st.active_layer.map(|id| (id, k)),
        _ => None,
    };
    let fresh = app.move_preview.as_ref().is_some_and(|p| p.doc == doc_id && p.revision == revision);
    if !fresh {
        let ids = match channel {
            Some((id, _)) => vec![id],
            None => photocraft_engine::layer_multi_cmds::move_targets(&doc, &st.selected_layers()),
        };
        if ids.is_empty() {
            return None;
        }
        let all = Rect::new(i32::MIN / 2, i32::MIN / 2, i32::MAX / 2, i32::MAX / 2);
        let bounds = ids.iter().try_fold(Rect::EMPTY, |acc, id| Some(acc.union(&photocraft_compose::change_bounds(doc.layer(*id)?, all)?)));
        let canvas = doc.bounds();
        // A channel move leaves the layer (and its Layer Edges outline) where it is.
        let moving = if channel.is_some() { Vec::new() } else { with_descendants(&doc, &ids) };
        let bounds = bounds.map(|b| moving.iter().filter_map(|id| shape_reach(doc.layer(*id)?, canvas)).fold(b, |acc, r| acc.union(&r)));
        app.move_preview = Some(MovePreview { doc: doc_id, revision, ids, moving, bounds, canvas, offsets: Vec::new(), shown: None, floating: false });
    }
    let p = app.move_preview.as_mut()?;
    if offset == (0, 0) && p.offsets.is_empty() {
        return None;
    }
    if p.offsets.last() != Some(&offset) {
        let t0 = crate::gpu_canvas::now_ms();
        let moved = match channel {
            Some((id, k)) => {
                let mut d = (*doc).clone();
                photocraft_engine::channel_cmds::move_color_channel(&mut d, id, k, offset.0, offset.1, background).map(|()| d)
            }
            None => photocraft_engine::layer_multi_cmds::moved_target(&doc, &p.ids, offset.0, offset.1, mask_target),
        };
        match moved {
            Ok(d) => {
                // Duotone documents display through their inks.
                let d = photocraft_engine::mode_cmds::display_document(&d).unwrap_or(d);
                p.shown = Some(Arc::new(d));
                p.offsets.push(offset);
            }
            Err(_) => {
                p.shown = None;
                return None;
            }
        }
        app.perf.span("move preview", crate::gpu_canvas::now_ms() - t0);
    }
    let p = app.move_preview.as_ref()?;
    Some((p.shown.clone()?, p.key()))
}

/// Where shape layer `l` paints, effects included, not cut at the canvas as its rendered pixels
/// are: a drag brings the rest of it into view (#3946). `None` for other layers.
fn shape_reach(l: &photocraft_doc::Layer, canvas: Rect) -> Option<Rect> {
    let photocraft_doc::LayerContent::Shape(sh) = &l.content else { return None };
    let b = photocraft_vector::CompiledShape::new(sh, photocraft_vector::DEFAULT_TOLERANCE, canvas).bounds()?;
    let m = if photocraft_compose::effects::has_effects(l) { photocraft_compose::effects::margin(l) } else { 0 };
    Some(b.inflate(m))
}

/// `ids` and every layer inside them (groups move their contents).
fn with_descendants(doc: &Document, ids: &[LayerId]) -> Vec<LayerId> {
    let walk = doc.walk();
    let roots: Vec<_> = walk.iter().filter(|(_, _, l)| ids.contains(&l.id)).map(|(p, _, _)| p.as_slice()).collect();
    walk.iter().filter(|(p, _, _)| roots.iter().any(|r| p.starts_with(r))).map(|(_, _, l)| l.id).collect()
}

/// Where the canvas shows layer `id` of document `doc` at `revision` relative to where it is: the
/// offset of the latest Move drag frame drawn ([`display_doc`]) when the layer moves with the drag,
/// so its Layer Edges outline moves in lockstep with its pixels (#2413). `None` when it shows
/// where it is.
pub(crate) fn shown_layer_offset(app: &PhotocraftApp, doc: DocId, revision: u64, id: LayerId) -> Option<(i32, i32)> {
    let p = app.move_preview.as_ref().filter(|p| p.doc == doc && p.revision == revision && !p.floating && p.shown.is_some())?;
    if !p.moving.contains(&id) {
        return None;
    }
    p.offsets.last().copied()
}

/// A floating selection's offset plus a drag of it in progress (`canvas::selection_drag_delta`).
pub(crate) fn floating_offset(app: &PhotocraftApp) -> Option<(i32, i32)> {
    let f = photocraft_engine::float_cmds::floating(app.session.active()?)?;
    let d = crate::canvas::selection_drag_delta(app).unwrap_or((0, 0));
    Some((f.offset.0 + d.0, f.offset.1 + d.1))
}

/// The document shown while a selection floats: the cut piece at its offset, over the cut-out
/// layer. Only the piece's area changes between offsets, so each move costs the piece, not the
/// layer.
fn floating_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    let offset = floating_offset(app)?;
    let st = app.session.documents().get(idx)?;
    let f = photocraft_engine::float_cmds::floating(st)?;
    let (doc_id, revision) = (st.doc.id, st.revision);
    let fresh = app.move_preview.as_ref().is_some_and(|p| p.doc == doc_id && p.revision == revision && p.floating);
    if !fresh {
        let bounds = st.doc.selection.as_ref().map(|s| s.content_bounds());
        let canvas = st.doc.bounds();
        app.move_preview = Some(MovePreview {
            doc: doc_id,
            revision,
            ids: vec![f.layer],
            moving: Vec::new(),
            bounds,
            canvas,
            offsets: Vec::new(),
            shown: None,
            floating: true,
        });
    }
    if offset == (0, 0) && app.move_preview.as_ref().is_none_or(|p| p.offsets.is_empty()) {
        return None;
    }
    let st = app.session.documents().get(idx)?;
    let shown = app.move_preview.as_ref()?.offsets.last() != Some(&offset);
    if shown {
        let t0 = crate::gpu_canvas::now_ms();
        let d = photocraft_engine::float_cmds::displayed(st, crate::canvas::selection_drag_delta(app).unwrap_or((0, 0)))?;
        let d = photocraft_engine::mode_cmds::display_document(&d).unwrap_or(d);
        let p = app.move_preview.as_mut()?;
        p.shown = Some(Arc::new(d));
        p.offsets.push(offset);
        app.perf.span("floating preview", crate::gpu_canvas::now_ms() - t0);
    }
    let p = app.move_preview.as_ref()?;
    Some((p.shown.clone()?, p.key()))
}

/// What changed between preview (or document) key `seen` and `now` of the current Move drag on
/// document `doc` at `revision`: where the moving layers were in both. `None` when either key
/// isn't one of this drag's (the canvas then recomposites everything).
pub(crate) fn damage(app: &PhotocraftApp, doc: DocId, revision: u64, seen: u64, now: u64) -> Option<Rect> {
    let p = app.move_preview.as_ref().filter(|p| p.doc == doc && p.revision == revision)?;
    if seen == now || (seen < BASE && seen != 0) || (now < BASE && now != 0) {
        return None;
    }
    let (a, b) = (p.area(p.offset_of(seen)?)?, p.area(p.offset_of(now)?)?);
    Some(if a.is_empty() {
        b
    } else if b.is_empty() {
        a
    } else {
        a.union(&b)
    })
}

/// Whether the canvas shows a Move drag's layers at the pointer.
pub(crate) fn showing(app: &PhotocraftApp) -> bool {
    app.move_preview.as_ref().is_some_and(|p| p.shown.is_some()) || app.session.active_index().is_some_and(|idx| gpu_showing(app, idx))
}

/// Whether `key` is a Move drag preview key.
pub(crate) fn is_preview_key(key: u64) -> bool {
    key > BASE && key < BASE << 1
}

/// Does a Move drag of whole layers take the selection along? With several layers selected, as
/// in Photoshop (#7538): their selected pixels can't float, so the layers move and the marching
/// ants follow them.
pub(crate) fn carries_selection(app: &PhotocraftApp) -> bool {
    app.session.active().is_some_and(|st| st.doc.selection.is_some() && st.selected_layers().len() > 1)
}

/// Release: one `layer.translate` by the drag's offset. The canvas already shows the result, so
/// its caches are told it showed the document: the command's damage rect refreshes that area.
pub(crate) fn finish(app: &mut PhotocraftApp, dx: f64, dy: f64) {
    // Within the range the preview and the ΔX/ΔY readout show (`drag_offset`), so the release
    // commits the offset they showed even at the lowest zoom of a huge document.
    let (dx, dy) = (dx.clamp(-1e7, 1e7), dy.clamp(-1e7, 1e7));
    app.move_gpu_preview = None;
    // Only when the canvas shows this very offset (else it recomposites everything once).
    let at = (dx as i32, dy as i32);
    let shown = app.move_preview.take().filter(|p| p.shown.is_some() && p.offsets.last() == Some(&at));
    if dx == 0.0 && dy == 0.0 {
        return;
    }
    if app.run("layer.translate", json!({"dx": dx, "dy": dy, "selection": carries_selection(app)})).is_err() {
        return;
    }
    let Some(p) = shown else { return };
    crate::canvas::shown_as_document(app, p.doc, is_preview_key);
}

#[cfg(test)]
mod tests;
