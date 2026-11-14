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
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

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
    float_selected_as(app, copy, dx, dy, None)
}

/// The start of a Move-tool drag of the selected pixels. An ⌥-drag copy is one "Duplicate" step,
/// as in Photoshop (#11383); a plain drag is "Move".
pub(crate) fn float_selected_for_drag(app: &mut PhotocraftApp, copy: bool) -> bool {
    float_selected_as(app, copy, 0.0, 0.0, copy.then(|| tl!("Duplicate")))
}

fn float_selected_as(app: &mut PhotocraftApp, copy: bool, dx: f64, dy: f64, step_name: Option<&str>) -> bool {
    let floating = app.session.active().is_some_and(|st| photocraft_engine::float_cmds::floating(st).is_some());
    let dropped = if copy && floating { app.run("select.drop", json!({})) } else { Ok(json!(null)) };
    match dropped.and_then(|_| app.run("select.float", json!({"dx": dx, "dy": dy, "copy": copy, "stepName": step_name}))) {
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

/// Most pixel memory the layers of a live Move drag may hold. Every frame of the drag shifts
/// copies of them by a pixel offset that is rarely a whole tile, so a multi-gigabyte layer would
/// copy gigabytes per pointer event on the UI thread and could run the machine out of memory
/// (#6114, #6163). Past it the drag shows the outline and arrow, as with Preferences › Interface ›
/// Show bounding box when dragging layer, and the release moves the layers once.
const LIVE_LIMIT: u64 = 512 << 20;

/// Pixel memory a shift of `s` by a pixel offset that isn't a whole tile writes, as
/// `Surface::translated` builds it: each target tile reads a 2 × 2 block of source tiles (one for
/// every block holding a stored tile, so isolated tiles each straddle four), and a block inside the
/// content whose four tiles are all stored is built once per distinct set of tiles. The inside of
/// a solid fill (one shared tile, `fill_rect`, PSD import) therefore costs one tile however large
/// it is; only its edge is written. Counts tile identities, never pixels: cheap at drag start.
fn shift_bytes(s: &photocraft_raster::Surface) -> u64 {
    let b = photocraft_compose::bounds::content_bounds(s);
    if b.is_empty() {
        return 0;
    }
    let stored: std::collections::HashMap<(i32, i32), usize> =
        s.tiles().filter(|(c, _)| !c.rect().intersect(&b).is_empty()).map(|(c, t)| ((c.tx, c.ty), Arc::as_ptr(t) as usize)).collect();
    let blocks: std::collections::HashSet<(i32, i32)> =
        stored.keys().flat_map(|&(x, y)| [(x.saturating_sub(1), y.saturating_sub(1)), (x, y.saturating_sub(1)), (x.saturating_sub(1), y), (x, y)]).collect();
    let mut built = std::collections::HashSet::new();
    let mut written = 0u64;
    for (x, y) in blocks {
        let (x1, y1) = (x.saturating_add(1), y.saturating_add(1));
        let corners = [(x, y), (x1, y), (x, y1), (x1, y1)].map(|c| stored.get(&c).copied());
        let inside = b.contains_rect(&photocraft_geom::TileCoord::new(x, y).rect().union(&photocraft_geom::TileCoord::new(x1, y1).rect()));
        let shared = match corners {
            [Some(p0), Some(p1), Some(p2), Some(p3)] if inside => !built.insert([p0, p1, p2, p3]),
            _ => false,
        };
        if !shared {
            written += 1;
        }
    }
    // Blocks are not trimmed to the moved content; the targets `translated` builds are, so they
    // never exceed the tiles the content can reach either.
    let t = photocraft_geom::TILE_SIZE as u64;
    let reach = (u64::from(b.width()).div_ceil(t) + 1).saturating_mul(u64::from(b.height()).div_ceil(t) + 1);
    written.min(reach).saturating_mul(t * t).saturating_mul(s.format().bytes_per_pixel() as u64)
}

/// Whether the pixel memory a live drag frame writes for the move targets `ids` of `doc` exceeds
/// `limit`, counting what `moved_target` shifts: a target whose unlinked mask is dragged alone
/// (`mask_target`) copies that mask only and its descendants stay put; any other target shifts its
/// content and linked mask (and a smart object's filter mask), and every layer inside it likewise (a group's contents move whole).
fn exceeds(doc: &Document, ids: &[LayerId], limit: u64, mask_target: bool) -> bool {
    let mut total = 0u64;
    for id in ids {
        let Some(root) = doc.layer(*id) else { continue };
        let moving = if mask_target && photocraft_engine::layer_multi_cmds::moves_mask_alone(root) {
            total = total.saturating_add(root.mask.as_ref().map_or(0, |m| shift_bytes(&m.surface)));
            Vec::new()
        } else {
            with_descendants(doc, &[*id])
        };
        for l in moving.iter().filter_map(|d| doc.layer(*d)) {
            let content = l.surface().map_or(0, shift_bytes);
            let mask = l.mask.as_ref().filter(|m| m.linked).map_or(0, |m| shift_bytes(&m.surface));
            // `shift_smart` shifts a smart object's filter mask too, linked or not.
            let filter_mask = match &l.content {
                photocraft_doc::LayerContent::Smart(sm) => sm.filter_mask.as_ref().map_or(0, |m| shift_bytes(&m.surface)),
                _ => 0,
            };
            total = total.saturating_add(content).saturating_add(mask).saturating_add(filter_mask);
        }
        if total > limit {
            return true;
        }
    }
    false
}

/// Preview keys of move drags: `BASE + n`, one per offset shown (see `canvas::display_doc`).
/// [`PATTERN_BIT`] marks a wrapped preview so a mid-drag Pattern Preview toggle cannot reuse
/// the other mode's cached pixels (`is_preview_key` still holds: `BASE + bit + n < BASE << 1`).
const BASE: u64 = 1 << 39;
const PATTERN_BIT: u64 = 1 << 20;

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
    /// Offsets shown so far, by preview key (`BASE + index`, with [`PATTERN_BIT`] when wrapping).
    offsets: Vec<(i32, i32)>,
    /// The document at the latest offset.
    shown: Option<Arc<Document>>,
    /// Shows a floating selection (`select.float`): offsets are the piece's, from where it was cut.
    floating: bool,
    /// The moving layers hold more pixels than [`LIVE_LIMIT`]: they aren't copied for every frame
    /// of the drag, which shows its outline and arrow alone (#6114, #6163).
    too_big: bool,
    /// A drag too costly to show live ([`costly_to_redraw`], [`LIVE_LIMIT`]): a reduced picture
    /// of the moving layers follows the pointer instead, over the canvas with them hidden.
    snapshot: Option<Snapshot>,
    /// View › Pattern Preview: pixels wrap, so a partial damage rect would leave the far edge stale.
    pattern: bool,
}

/// The reduced picture of the moving layers a costly drag shows at the pointer ([`draw_snapshot`]).
pub(crate) struct Snapshot {
    /// The document area it shows, at offset (0, 0).
    rect: Rect,
    /// The render, until it arrives from its thread.
    rx: Option<std::sync::mpsc::Receiver<photocraft_compose::Buffer>>,
    /// The render once arrived, as composited: each display converts its own texture from it.
    picture: Option<photocraft_compose::Buffer>,
    /// Its textures, by display (`CanvasDisplay::key`; 0 unconverted): two views on monitors
    /// with different profiles each show it in their own colours, and a profile changed
    /// mid-drag converts it again.
    textures: Vec<(u64, egui::TextureHandle)>,
}

/// Displays a snapshot keeps textures for at once (the oldest is dropped past that).
const SNAPSHOT_DISPLAYS: usize = 2;

/// Longest side of a drag snapshot: enough for the zooms at which a costly drag happens.
const SNAPSHOT_SIDE: u32 = 2048;

/// Starts rendering the layers `moving` of `doc` (and nothing else) over `rect`, reduced, on a
/// thread. `None` (the drag shows its outline and arrow) when the area is empty, the thread can't
/// start, another drag's render is still running ([`SnapshotSlot`]), or on the web, which has no
/// thread to render it on without stopping the pointer.
fn start_snapshot(doc: &Document, moving: &[LayerId], rect: Rect) -> Option<Snapshot> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (doc, moving, rect);
        None
    }
    #[cfg(not(target_arch = "wasm32"))]
    start_snapshot_thread(doc, moving, rect)
}

/// [`start_snapshot`] off the web.
#[cfg(not(target_arch = "wasm32"))]
fn start_snapshot_thread(doc: &Document, moving: &[LayerId], rect: Rect) -> Option<Snapshot> {
    if rect.is_empty() {
        return None;
    }
    let slot = SnapshotSlot::take()?;
    // The moving layers and the groups holding them stay visible; every other layer is hidden.
    let walk = doc.walk();
    let paths: Vec<_> = walk.iter().filter(|(_, _, l)| moving.contains(&l.id)).map(|(p, _, _)| p.clone()).collect();
    let keep = |path: &[usize]| paths.iter().any(|m| m.starts_with(path) || path.starts_with(m));
    let hide: Vec<LayerId> = walk.iter().filter(|(p, _, _)| !keep(p)).map(|(_, _, l)| l.id).collect();
    let mut only = doc.clone();
    for id in hide {
        if let Some(l) = only.layer_mut(id) {
            l.visible = false;
        }
    }
    let longest = rect.width().max(rect.height()).max(1);
    let k = longest.div_ceil(SNAPSHOT_SIDE).max(1);
    let (w, h) = (rect.width().div_ceil(k).max(1), rect.height().div_ceil(k).max(1));
    let (tx, rx) = std::sync::mpsc::channel();
    let started = std::thread::Builder::new().name("photocraft-drag-snapshot".into()).spawn(move || {
        // On a small pool of its own: in the shared one its tiles queued ahead of the redraw
        // thread's uploads, and the canvas blocks drawn meanwhile took 100–260 ms each.
        let picture = match snapshot_pool() {
            Some(pool) => pool.install(|| photocraft_compose::render_reduced_rect(&only, rect, w, h)),
            None => photocraft_compose::render_reduced_rect(&only, rect, w, h),
        };
        // Freed once rendered, whether or not its drag still wants it.
        drop(slot);
        let _ = tx.send(picture);
    });
    started.ok()?;
    Some(Snapshot { rect, rx: Some(rx), picture: None, textures: Vec::new() })
}

/// One drag snapshot renders at a time: a drag released before its picture arrived leaves the
/// render running to its end, and repeated short drags would otherwise stack renders of the whole
/// area nobody will see. A drag starting meanwhile shows its outline and arrow. Per UI thread
/// (each test runs its own).
#[cfg(not(target_arch = "wasm32"))]
pub(super) struct SnapshotSlot(Arc<std::sync::atomic::AtomicBool>);

#[cfg(not(target_arch = "wasm32"))]
impl SnapshotSlot {
    /// The slot, unless a render holds it.
    pub(super) fn take() -> Option<Self> {
        thread_local! {
            static BUSY: Arc<std::sync::atomic::AtomicBool> = Arc::new(std::sync::atomic::AtomicBool::new(false));
        }
        BUSY.with(|busy| (!busy.swap(true, std::sync::atomic::Ordering::AcqRel)).then(|| SnapshotSlot(Arc::clone(busy))))
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for SnapshotSlot {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}

/// The thread pool drag snapshots render on: half the cores, so the redraw thread's work on the
/// shared pool goes on meanwhile. `None` if it couldn't be built (the shared pool is used).
#[cfg(not(target_arch = "wasm32"))]
fn snapshot_pool() -> Option<&'static rayon::ThreadPool> {
    static POOL: std::sync::OnceLock<Option<rayon::ThreadPool>> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        let n = std::thread::available_parallelism().map_or(2, |n| (n.get() / 2).max(1));
        rayon::ThreadPoolBuilder::new().num_threads(n).thread_name(|i| format!("photocraft-snapshot-{i}")).build().ok()
    })
    .as_ref()
}

/// Draws a costly drag's snapshot at the pointer, once its render has arrived (taking it).
pub(crate) fn draw_snapshot(app: &mut PhotocraftApp, painter: &egui::Painter, xf: &crate::canvas::ViewXform) {
    let offset = drag_offset(app);
    // Converted for the monitor the way the canvas is, so the layers keep their colours in the drag.
    let doc = app.move_preview.as_ref().and_then(|p| app.session.documents().iter().find(|st| st.doc.id == p.doc)).map(|st| st.doc.clone());
    let display = doc.and_then(|d| super::display::canvas_display(app, &d, crate::monitor_status::view_display(app, painter.ctx())).0);
    let Some(p) = app.move_preview.as_mut() else { return };
    let canvas = p.canvas;
    let Some(s) = p.snapshot.as_mut() else { return };
    if let Some(rx) = &s.rx {
        match rx.try_recv() {
            Ok(buf) => {
                let size = [buf.rect.width() as usize, buf.rect.height() as usize];
                if buf.px.len() == size[0] * size[1] {
                    s.picture = Some(buf);
                    // The next frame hides the layers under it (`display_doc`), with or without input.
                    painter.ctx().request_repaint();
                }
                s.rx = None;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => s.rx = None,
            // The render thread can't wake the UI: look again soon, even if the pointer rests.
            Err(std::sync::mpsc::TryRecvError::Empty) => painter.ctx().request_repaint_after(std::time::Duration::from_millis(50)),
        }
    }
    let key = display.as_ref().map_or(0, |d| d.key);
    if let Some(picture) = &s.picture
        && !s.textures.iter().any(|(k, _)| *k == key)
    {
        // Straight alpha, as the canvas's own textures take it.
        let image = super::display::display_image(display.as_deref(), picture);
        if s.textures.len() >= SNAPSHOT_DISPLAYS {
            drop(s.textures.remove(0));
        }
        s.textures.push((key, painter.ctx().load_texture("move-drag-snapshot", image, egui::TextureOptions::LINEAR)));
    }
    let texture = s.textures.iter().find(|(k, _)| *k == key).map(|(_, t)| t);
    let (Some(texture), Some((dx, dy))) = (texture, offset) else { return };
    let Some((r, uvs)) = snapshot_quad(s.rect.translate(dx, dy), canvas) else { return };
    let corners = [(r.x0, r.y0), (r.x1, r.y0), (r.x1, r.y1), (r.x0, r.y1)];
    let mut mesh = egui::Mesh::with_texture(texture.id());
    for ((x, y), uv) in corners.into_iter().zip(uvs) {
        mesh.vertices.push(egui::epaint::Vertex { pos: xf.to_screen(x as f32, y as f32), uv, color: egui::Color32::WHITE });
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    painter.add(egui::Shape::mesh(mesh));
}

/// The part of a snapshot drawn at `full` that lies on `canvas`, and the texture coordinates of its
/// corners (clockwise from the top left): the layers' pixels beyond the canvas stay hidden, as
/// in the live preview and the result. In document space, so rotated views clip it too.
fn snapshot_quad(full: Rect, canvas: Rect) -> Option<(Rect, [egui::Pos2; 4])> {
    let r = full.intersect(&canvas);
    if r.is_empty() || full.is_empty() {
        return None;
    }
    let (w, h) = (full.width() as f32, full.height() as f32);
    let (u0, v0) = ((r.x0 - full.x0) as f32 / w, (r.y0 - full.y0) as f32 / h);
    let (u1, v1) = ((r.x1 - full.x0) as f32 / w, (r.y1 - full.y0) as f32 / h);
    Some((r, [egui::pos2(u0, v0), egui::pos2(u1, v0), egui::pos2(u1, v1), egui::pos2(u0, v1)]))
}

impl MovePreview {
    fn key(&self) -> u64 {
        BASE + self.offsets.len() as u64 + if self.pattern { PATTERN_BIT } else { 0 }
    }
    fn offset_of(&self, key: u64) -> Option<(i32, i32)> {
        if key == 0 {
            return Some((0, 0));
        }
        if (key & PATTERN_BIT != 0) != self.pattern {
            return None;
        }
        let i = usize::try_from((key & !PATTERN_BIT).checked_sub(BASE + 1)?).ok()?;
        self.offsets.get(i).copied()
    }
    fn area(&self, d: (i32, i32)) -> Option<Rect> {
        if self.pattern {
            return Some(self.canvas);
        }
        let b = self.bounds?;
        Some(if b.is_empty() { b } else { b.translate(d.0, d.1).inflate(1).intersect(&self.canvas) })
    }
}

/// `layer.translate` params, plus `"wrap": true` while View › Pattern Preview is on.
pub(crate) fn pattern_wrap(app: &PhotocraftApp, mut args: serde_json::Value) -> serde_json::Value {
    if app.ui.view.pattern_preview
        && let Some(obj) = args.as_object_mut()
    {
        obj.insert("wrap".into(), json!(true));
    }
    args
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
    let st = app.session.documents().get(idx)?;
    let (doc_id, revision, doc) = (st.doc.id, st.revision, st.doc.clone());
    let pattern = app.ui.view.pattern_preview;
    // A toggle of Pattern Preview mid-drag drops the old keys; `damage` is then `None` and the
    // canvas recomposites, so preview pixels and the release share the live mode.
    if app.move_preview.as_ref().is_some_and(|p| p.doc == doc_id && p.revision == revision && p.pattern != pattern) {
        app.move_preview = None;
    }
    let fresh = app.move_preview.as_ref().is_some_and(|p| p.doc == doc_id && p.revision == revision);
    if !fresh {
        let ids = photocraft_engine::layer_multi_cmds::move_targets(&doc, &st.selected_layers());
        if ids.is_empty() {
            return None;
        }
        let all = Rect::new(i32::MIN / 2, i32::MIN / 2, i32::MAX / 2, i32::MAX / 2);
        // The context-free bound: it moves with the layer. `change_bounds_in` is an in-place bound
        // (the layers beneath an adjustment, or a clip base, stay put), so it can't be translated.
        let bounds = ids.iter().try_fold(Rect::EMPTY, |acc, id| Some(acc.union(&photocraft_compose::change_bounds(doc.layer(*id)?, all)?)));
        let canvas = doc.bounds();
        let moving = with_descendants(&doc, &ids);
        let bounds = bounds.map(|b| moving.iter().filter_map(|id| shape_reach(doc.layer(*id)?, canvas)).fold(b, |acc, r| acc.union(&r)));
        let too_big = exceeds(&doc, &ids, LIVE_LIMIT, mask_target) || costly_to_redraw(&doc, bounds);
        // The drag's snapshot: the layers' area as they paint it, wherever it reaches. Pattern
        // Preview tiles the canvas, which one picture can't follow: those drags keep the outline.
        let snapshot = if too_big && !mask_target && !pattern { bounds.and_then(|b| start_snapshot(&doc, &moving, b)) } else { None };
        if too_big {
            let shown = if snapshot.is_some() { "as a snapshot of the layers" } else { "as an outline" };
            log::info!("move: dragging {} layer(s) {shown} (too costly to redraw live), the release redraws once", ids.len());
        }
        app.move_preview = Some(MovePreview {
            doc: doc_id,
            revision,
            ids,
            moving,
            bounds,
            canvas,
            offsets: Vec::new(),
            shown: None,
            floating: false,
            too_big,
            snapshot,
            pattern,
        });
    }
    let p = app.move_preview.as_mut()?;
    if p.too_big {
        // Once its snapshot is ready (and the pointer has moved), the canvas shows the document
        // with the moving layers hidden, at one preview key: hiding them redraws where they were,
        // once; the snapshot follows the pointer; the release redraws where they were and are.
        let ready = p.snapshot.as_ref().is_some_and(|s| s.picture.is_some());
        if !ready || offset == (0, 0) && p.offsets.is_empty() {
            return None;
        }
        if p.shown.is_none() {
            let mut hidden = (*doc).clone();
            for id in &p.ids {
                if let Some(l) = hidden.layer_mut(*id) {
                    l.visible = false;
                }
            }
            let hidden = photocraft_engine::mode_cmds::display_document(&hidden).unwrap_or(hidden);
            p.shown = Some(Arc::new(hidden));
            // The key's offset is (0, 0): its damage is where the layers were.
            p.offsets = vec![(0, 0)];
        }
        return Some((p.shown.clone()?, p.key()));
    }
    if offset == (0, 0) && p.offsets.is_empty() {
        return None;
    }
    if p.offsets.last() != Some(&offset) {
        let t0 = crate::gpu_canvas::now_ms();
        let wrap = app.ui.view.pattern_preview.then_some(doc.bounds());
        match photocraft_engine::layer_multi_cmds::moved_wrapping(&doc, &p.ids, offset.0, offset.1, mask_target, wrap) {
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

/// Fewest layers in a document for [`costly_to_redraw`] to count it as many-layered.
const MANY_LAYERS: usize = 64;
/// Smallest area (pixels) the moving layers cover for [`costly_to_redraw`].
const LARGE_AREA_PX: u64 = 16 << 20;

/// Whether showing a drag live would redraw a large area of a many-layered document every frame:
/// every pointer move recomposites where the layers were and are through the whole stack, which
/// on a storyboard of hundreds of layers took about 100 ms a frame and seconds of redrawing after
/// it. Such a drag shows its outline and arrow instead (as Preferences › Interface › Show
/// bounding box when dragging layer does) and the release redraws once.
fn costly_to_redraw(doc: &Document, bounds: Option<Rect>) -> bool {
    let area = bounds.map_or(u64::MAX, |b| {
        let b = b.intersect(&doc.bounds());
        if b.is_empty() { 0 } else { u64::from(b.width()) * u64::from(b.height()) }
    });
    area >= LARGE_AREA_PX && doc.walk().len() >= MANY_LAYERS
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
pub(crate) fn with_descendants(doc: &Document, ids: &[LayerId]) -> Vec<LayerId> {
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
    // A snapshot drag's canvas shows the layers hidden; their outline follows the snapshot.
    if p.snapshot.is_some() {
        return drag_offset(app);
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
            too_big: false,
            snapshot: None,
            pattern: false,
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

/// A Move drag's release, which made revision `revision` of document `doc`: what the canvas
/// showed at each of the drag's preview keys, so a cache that last drew one redraws only what
/// differs from the result ([`drop_damage`]).
pub(crate) struct MoveDrop {
    doc: DocId,
    revision: u64,
    /// Where preview key `BASE + 1 + i` drew the moving layers, at `areas[i]`.
    areas: Vec<Option<Rect>>,
    /// A snapshot drag's: the key that showed the document with the layers hidden where they were,
    /// and where they landed, the only area that differs from it.
    landed: Option<(u64, Rect)>,
}

/// What a cache that last drew Move preview key `key` must redraw to show revision `revision` of
/// document `doc`, made by the drag's release: for a snapshot drag's hidden-layers key, where the
/// layers landed (widened by `reach` for their effects); for another preview, where it drew the
/// layers plus `moved`, the move's own damage (where they were and are), since the pointer may
/// have gone beyond both before the release. `None` when unknown: the caller redraws everything.
pub(crate) fn drop_damage(app: &PhotocraftApp, doc: DocId, revision: u64, key: u64, moved: Option<Rect>, reach: i32) -> Option<Rect> {
    let d = app.move_drop.as_ref().filter(|d| d.doc == doc && d.revision == revision)?;
    if let Some((k, r)) = d.landed
        && k == key
    {
        return Some(if r.is_empty() { r } else { r.inflate(reach) });
    }
    let shown = (*d.areas.get(usize::try_from(key.checked_sub(BASE + 1)?).ok()?)?)?;
    let moved = moved?;
    Some(if shown.is_empty() {
        moved
    } else if moved.is_empty() {
        shown
    } else {
        moved.union(&shown)
    })
}

/// Whether a Move drag shows (or is about to show) a snapshot of its layers rather than an arrow.
pub(crate) fn snapshot_drag(app: &PhotocraftApp) -> bool {
    app.move_preview.as_ref().is_some_and(|p| p.snapshot.is_some())
}

/// Whether the canvas shows a Move drag's layers at the pointer.
pub(crate) fn showing(app: &PhotocraftApp) -> bool {
    app.move_preview.as_ref().is_some_and(|p| p.shown.is_some())
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
    // The raw drag, including a whole number of canvases. `layer.translate` drops that only
    // when every target is raster samples folding back onto themselves.
    let at = (dx as i32, dy as i32);
    let pattern = app.ui.view.pattern_preview;
    let preview = app.move_preview.take();
    // A snapshot drag's canvas shows the layers hidden, drawn where they were: after the move it
    // differs only where they land, so that is all the drop redraws (not one rectangle around
    // both places: up to 585 MP for a 108 MP photo moved across a storyboard).
    let landed = preview.as_ref().filter(|p| p.snapshot.is_some() && p.shown.is_some() && p.offsets == [(0, 0)]).and_then(|p| Some((p.key(), p.area(at)?)));
    // Where each preview the canvas may still show drew the layers.
    // Not under Pattern Preview: its pixels wrap, so a partial redraw would leave the far edge stale.
    let drawn = preview.as_ref().filter(|p| !p.pattern && !pattern).map(|p| (p.doc, p.offsets.iter().map(|o| p.area(*o)).collect::<Vec<_>>()));
    let shown = preview.filter(|p| p.shown.is_some() && p.offsets.last() == Some(&at) && p.pattern == pattern);
    if dx == 0.0 && dy == 0.0 {
        return;
    }
    // `nest`: a layer dropped on another artboard joins it (Photoshop's auto-nesting).
    let Ok(result) =
        app.run("layer.translate", pattern_wrap(app, json!({"dx": dx, "dy": dy, "selection": carries_selection(app), "nest": app.ui.artboard_tool.auto_nest})))
    else {
        return;
    };
    if let Some((doc, areas)) = drawn
        && let Some(st) = app.session.active().filter(|st| st.doc.id == doc)
    {
        if let Some((_, rect)) = landed {
            log::info!("move: the drop redraws only where the layers landed ({:.0} MP)", rect.size().area() as f64 / 1_048_576.0);
        }
        app.move_drop = Some(MoveDrop { doc, revision: st.revision, areas, landed });
    }
    // A layer that changed board is clipped by its new board, not the old one the preview showed,
    // so the canvas recomposites instead of keeping the preview.
    let Some(p) = shown.filter(|_| result.get("nested").is_none()) else { return };
    crate::canvas::shown_as_document(app, p.doc, is_preview_key);
}

#[cfg(test)]
mod tests;
