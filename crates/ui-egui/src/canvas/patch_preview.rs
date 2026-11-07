//! Patch previews recompose only the affected rectangle. In Affinity mode the canvas keeps the
//! last completed preview while a worker computes the latest placement. The final command uses
//! the exact solve.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

use photocraft_doc::{DocId, Document, LayerId};
use photocraft_geom::Rect;
use serde_json::Value;
#[cfg(test)]
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// Preview keys: `BASE + n`, one per placement shown (see `canvas::display_doc`).
const BASE: u64 = 1 << 38;
/// Cells of the preview's coarse solve: a 128² solve takes about 5 ms (`bench_retouch`), whatever
/// the selection's size.
const MAX_CELLS: u64 = 128 * 128;

/// Affinity's moving preview uses a smaller colour-fit grid. The texture remains sharp.
const AFFINITY_CELLS: u64 = 64 * 64;

type Rendered = Option<(Arc<Document>, Rect)>;

/// While the user moves a patch on a simple raster, draw its sample as a textured quad.
const SETTLE_SECONDS: f64 = 0.10;
const MOTION_TEXTURE_SIDE: u32 = 160;
const MOTION_EDGE_SAMPLES: usize = 48;

type Tone = [[i16; 3]; 4];

struct EdgeSample {
    uv: [f32; 2],
    rgb: [f32; 3],
}

pub(crate) struct PatchMotion {
    key: (DocId, u64, LayerId, u64),
    texture: egui::TextureHandle,
    sample: photocraft_compose::Buffer,
    edges: Vec<EdgeSample>,
    tone: Tone,
    params: Value,
    quad: [[f64; 2]; 4],
    last_move: f64,
    moving: bool,
}

/// Return the placed Patch box, or the quad at the current hover position before placement.
fn motion_quad(app: &mut PhotocraftApp, bounds: Rect) -> Option<[[f64; 2]; 4]> {
    if let Some(t) = &app.ui.cam_transform {
        return t.patch.as_ref().map(|_| t.frame.quad);
    }
    let offset = hover_offset(app)?;
    let place = crate::cam_transform::patch_placement(app, offset);
    Some(place.quad([bounds.x0, bounds.y0, bounds.x1, bounds.y1].map(f64::from)))
}

/// Sample the layer through the selection mask into a bounded-size preview texture.
fn motion_texture(layer: &photocraft_raster::Surface, sel: &photocraft_raster::Surface, bounds: Rect) -> photocraft_compose::Buffer {
    let step = bounds.width().max(bounds.height()).div_ceil(MOTION_TEXTURE_SIDE).max(1);
    let (w, h) = (bounds.width().div_ceil(step), bounds.height().div_ceil(step));
    let mut buf = photocraft_compose::Buffer::transparent(Rect::new(0, 0, w as i32, h as i32));
    if step == 1 {
        layer.read_rgba_into(bounds, &mut buf.px);
    }
    for (y, row) in buf.px.chunks_exact_mut(w as usize).enumerate() {
        let sy = bounds.y0 + (((2 * y as u32 + 1) as u64 * u64::from(bounds.height())) / (2 * u64::from(h))) as i32;
        for (x, pixel) in row.iter_mut().enumerate() {
            let sx = bounds.x0 + (((2 * x as u32 + 1) as u64 * u64::from(bounds.width())) / (2 * u64::from(w))) as i32;
            if step > 1 {
                *pixel = layer.rgba(sx, sy);
            }
            pixel[3] *= sel.sample_channel(sx, sy, 0).clamp(0.0, 1.0);
        }
    }
    buf
}

/// Cache a small set of source samples just outside the mask. The full solver also fits its
/// colour correction to the difference across the boundary.
fn motion_edges(sample: &photocraft_compose::Buffer, surface: &photocraft_raster::Surface, bounds: Rect, canvas: Rect) -> Vec<EdgeSample> {
    let (w, h) = (sample.rect.width() as usize, sample.rect.height() as usize);
    let inside = |x: isize, y: isize| {
        x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && sample.px.get(y as usize * w + x as usize).is_some_and(|px| px[3] >= 0.5)
    };
    let mut boundary = Vec::new();
    for (i, px) in sample.px.iter().enumerate() {
        if px[3] < 0.5 {
            continue;
        }
        let (x, y) = ((i % w) as isize, (i / w) as isize);
        for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            if !inside(x + dx, y + dy) {
                boundary.push([(x + dx) as f32 + 0.5, (y + dy) as f32 + 0.5]);
            }
        }
    }
    let step = boundary.len().div_ceil(MOTION_EDGE_SAMPLES).max(1);
    boundary
        .into_iter()
        .step_by(step)
        .filter_map(|[x, y]| {
            let uv = [x / w as f32, y / h as f32];
            let x = bounds.x0 + (uv[0] * bounds.width() as f32).floor() as i32;
            let y = bounds.y0 + (uv[1] * bounds.height() as f32).floor() as i32;
            if !canvas.contains(x, y) {
                return None;
            }
            let [r, g, b, a] = surface.rgba(x, y);
            (a >= 0.5).then_some(EdgeSample { uv, rgb: [r, g, b] })
        })
        .collect()
}

/// Estimate four corner colour offsets from paired source and destination samples along the mask boundary.
fn motion_tone(edges: &[EdgeSample], quad: [[f64; 2]; 4], surface: &photocraft_raster::Surface, canvas: Rect) -> Tone {
    let mut sums = [[0.0_f32; 3]; 4];
    let mut weights = [0.0_f32; 4];
    for edge in edges {
        let [u, v] = edge.uv;
        let at = |axis: usize| (quad[0][axis] + (quad[1][axis] - quad[0][axis]) * f64::from(u) + (quad[3][axis] - quad[0][axis]) * f64::from(v)).floor() as i32;
        let (x, y) = (at(0), at(1));
        if !canvas.contains(x, y) {
            continue;
        }
        let [r, g, b, alpha] = surface.rgba(x, y);
        if alpha < 0.5 {
            continue;
        }
        let [u, v] = [u.clamp(0.0, 1.0), v.clamp(0.0, 1.0)];
        let weights_at = [(1.0 - u) * (1.0 - v), u * (1.0 - v), u * v, (1.0 - u) * v];
        for ((sum, weight), k) in sums.iter_mut().zip(&mut weights).zip(weights_at) {
            *weight += k;
            for (c, (dst, src)) in sum.iter_mut().zip([r, g, b].into_iter().zip(edge.rgb)) {
                *c += (dst - src) * k;
            }
        }
    }
    std::array::from_fn(|i| sums[i].map(|c| (c * 255.0 / weights[i].max(0.001)).round() as i16))
}

/// Interpolate the corner offsets across the texture, then convert it to display colours.
fn motion_image(sample: &photocraft_compose::Buffer, tone: Tone, display: Option<&photocraft_engine::display_color::CanvasDisplay>) -> egui::ColorImage {
    if tone == [[0; 3]; 4] {
        return super::display::display_image(display, sample);
    }
    let tone = tone.map(|corner| corner.map(|c| f32::from(c) / 255.0));
    let (w, h) = (sample.rect.width() as usize, sample.rect.height() as f32);
    let mut toned = sample.clone();
    for (y, row) in toned.px.chunks_exact_mut(w).enumerate() {
        let v = (y as f32 + 0.5) / h;
        let left: [f32; 3] = std::array::from_fn(|c| tone[0][c] + (tone[3][c] - tone[0][c]) * v);
        let dx: [f32; 3] = std::array::from_fn(|c| (tone[1][c] + (tone[2][c] - tone[1][c]) * v - left[c]) / w as f32);
        for (x, pixel) in row.iter_mut().enumerate() {
            for (c, value) in pixel.iter_mut().take(3).enumerate() {
                *value = (*value + left[c] + dx[c] * (x as f32 + 0.5)).clamp(0.0, 1.0);
            }
        }
    }
    super::display::display_image(display, &toned)
}

/// Build or update the moving preview for an eligible raster target.
/// Reuse the sampled pixels while the document, layer, and display transform stay the same.
fn motion_state(app: &mut PhotocraftApp, ctx: &egui::Context, idx: usize, output: Option<u32>) -> Option<PatchMotion> {
    if app.session.active_index() != Some(idx)
        || !crate::retouch_ui::affinity_patch(app)
        || app.active_tool() != Tool::Patch
        || app.ui.tool_options.patch_mode != "destination"
        || app.ui.view.pattern_preview
        || !app.ui.dialogs.is_empty()
    {
        return None;
    }
    let bounds = crate::retouch_ui::patch_bounds(app)?;
    let quad = motion_quad(app, bounds)?;
    let params = wanted(app)?;
    if params["target"] != "pixels" {
        return None;
    }
    let st = app.session.active()?;
    let doc = &st.doc;
    let id = st.active_layer?;
    let layer = doc.layers.iter().rfind(|layer| layer.visible)?;
    let channels = &st.channel_view;
    if doc.mode != photocraft_doc::ColorMode::Rgb
        || doc.depth != photocraft_doc::SampleType::U8
        || channels.target != photocraft_engine::channel_cmds::ChannelTarget::Composite
        || channels.layer_mask.is_some()
        || channels.color_hidden.contains(&true)
        || channels.alpha_visible.contains(&true)
        || layer.id != id
        || !layer.visible
        || layer.opacity != 1.0
        || layer.fill_opacity != 1.0
        || layer.blend != photocraft_doc::BlendMode::Normal
        || layer.mask.is_some()
        || layer.vector_mask.is_some()
        || layer.frame.is_some()
        || layer.video.is_some()
        || layer.clipped
        || layer.excluded_channels != 0
        || layer.blend_if != Default::default()
        || layer.advanced != Default::default()
        || layer.locks.all
        || layer.locks.pixels
        || (layer.locks.transparency && !doc.is_background(id))
        || photocraft_compose::effects::has_effects(layer)
    {
        return None;
    }
    let surface = layer.surface()?;
    let sel = doc.selection.as_ref()?;
    let (display, display_key) = super::display::canvas_display(app, doc, output);
    let key = (doc.id, st.revision, id, display_key);
    let time = ctx.input(|i| i.time);
    if let Some(mut m) = app.patch_motion.take().filter(|m| m.key == key) {
        if m.quad != quad || m.params != params {
            m.last_move = time;
            m.quad = quad;
            m.params = params;
            let tone = motion_tone(&m.edges, quad, surface, doc.bounds());
            if m.tone != tone {
                m.texture.set(motion_image(&m.sample, tone, display.as_deref()), egui::TextureOptions::LINEAR);
                m.tone = tone;
            }
        }
        m.moving = time - m.last_move < SETTLE_SECONDS;
        return Some(m);
    }
    let sample = motion_texture(surface, sel, bounds);
    let edges = motion_edges(&sample, surface, bounds, doc.bounds());
    let tone = motion_tone(&edges, quad, surface, doc.bounds());
    let image = motion_image(&sample, tone, display.as_deref());
    let texture = ctx.load_texture("affinity patch sample", image, egui::TextureOptions::LINEAR);
    Some(PatchMotion { key, texture, sample, edges, tone, params, quad, last_move: time, moving: true })
}

/// Refresh the motion overlay and schedule the frame that starts healing after the pointer settles.
pub(super) fn update_motion(app: &mut PhotocraftApp, ctx: &egui::Context, idx: usize, output: Option<u32>) {
    let start = crate::gpu_canvas::now_ms();
    app.patch_motion = motion_state(app, ctx, idx, output);
    if app.patch_motion.is_some() {
        app.perf.span("patch motion", crate::gpu_canvas::now_ms() - start);
    }
    if let Some(m) = app.patch_motion.as_ref().filter(|m| m.moving) {
        let time = ctx.input(|i| i.time);
        let remaining = (m.last_move + SETTLE_SECONDS - time).clamp(0.0, SETTLE_SECONDS);
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(remaining));
    }
}

/// Draw the textured quad clipped to the canvas until the matching healed preview can replace it.
pub(super) fn draw_motion(app: &PhotocraftApp, painter: &egui::Painter, xf: &super::ViewXform, doc: DocId) {
    let Some(m) = app.patch_motion.as_ref().filter(|m| m.key.0 == doc) else { return };
    let healed = app
        .patch_preview
        .as_ref()
        .is_some_and(|p| (p.doc, p.revision, p.layer) == (m.key.0, m.key.1, Some(m.key.2)) && p.shown.is_some() && p.params.as_ref() == Some(&m.params));
    if !m.moving && healed {
        return;
    }
    let Some(size) = app.session.active().filter(|st| st.doc.id == doc).map(|st| st.doc.size) else { return };
    let mut mesh = egui::Mesh::with_texture(m.texture.id());
    for ([x, y], [u, v]) in m.quad.into_iter().zip([[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]) {
        mesh.vertices.push(egui::epaint::Vertex { pos: egui::pos2(x as f32, y as f32), uv: egui::pos2(u, v), color: egui::Color32::WHITE });
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    let mesh = crate::transform_tool::clip_to_canvas(mesh, [size.width as f32, size.height as f32], |p| xf.to_screen(p.x, p.y));
    if !mesh.indices.is_empty() {
        painter.add(egui::Shape::mesh(mesh));
    }
}

pub(crate) struct PatchWorker {
    doc: DocId,
    revision: u64,
    layer: Option<LayerId>,
    params: Value,
    result: Receiver<Rendered>,
}

pub(crate) struct PatchPreview {
    doc: DocId,
    revision: u64,
    params: Option<Value>,
    layer: Option<LayerId>,
    affinity: bool,
    serial: u64,
    /// Recent damage rectangles, oldest first, indexed by preview key.
    damage: VecDeque<(u64, Rect)>,
    shown: Option<Arc<Document>>,
}

impl PatchPreview {
    /// Store the rendered document and retain recent damage bounds for incremental canvas refreshes.
    fn publish(&mut self, params: Value, layer: Option<LayerId>, result: Rendered) {
        self.params = Some(params);
        self.layer = layer;
        self.shown = result.map(|(doc, area)| {
            self.serial += 1;
            self.damage.push_back((BASE + self.serial, area));
            if self.damage.len() > 64 {
                self.damage.pop_front();
            }
            doc
        });
    }
}

/// Convert a rendered preview to the display document while preserving its damage rectangle.
fn display_result(doc: Document, damage: Rect) -> (Arc<Document>, Rect) {
    (Arc::new(photocraft_engine::mode_cmds::display_document(&doc).unwrap_or(doc)), damage)
}

/// Compare Patch direction and paint target independently of placement.
fn same_target(a: &Value, b: &Value) -> bool {
    a["mode"] == b["mode"] && a["target"] == b["target"]
}

/// Keep one native job in flight, even across document changes. The next job always takes the
/// latest pointer position, so input cannot accumulate a backlog.
fn background(app: &mut PhotocraftApp, params: &Value, min_step: u32) {
    let Some(p) = app.patch_preview.as_mut() else { return };
    let (doc, revision, layer) = (p.doc, p.revision, p.layer);
    if let Some(worker) = &app.patch_worker {
        match worker.result.try_recv() {
            Ok(result) => {
                let Some(worker) = app.patch_worker.take() else { return };
                if (worker.doc, worker.revision, worker.layer) == (doc, revision, layer)
                    && same_target(&worker.params, params)
                    && (worker.params == *params || (result.is_some() && p.params.as_ref() != Some(params)))
                {
                    p.publish(worker.params, layer, result);
                }
            }
            Err(TryRecvError::Disconnected) => {
                app.patch_worker = None;
            }
            Err(TryRecvError::Empty) => return,
        }
    }
    if p.params.as_ref() == Some(params) {
        return;
    }
    let job = match photocraft_engine::retouch_cmds::prepare_patch_preview(&app.session, params, AFFINITY_CELLS, min_step) {
        Ok(job) => job,
        Err(_) => {
            p.publish(params.clone(), layer, None);
            return;
        }
    };
    let (tx, result) = std::sync::mpsc::channel();
    if std::thread::Builder::new()
        .name("patch-preview".into())
        .spawn(move || {
            let result = job.run().ok().map(|(d, area)| display_result(d, area));
            let _ = tx.send(result);
        })
        .is_ok()
    {
        app.patch_worker = Some(PatchWorker { doc, revision, layer, params: params.clone(), result });
    } else {
        p.publish(params.clone(), layer, None);
    }
}

/// Keep drawing until the worker publishes a result, including after the pointer stops moving.
pub(crate) fn wake(app: &PhotocraftApp, ctx: &egui::Context) {
    if app.patch_worker.is_some() && crate::retouch_ui::affinity_patch(app) {
        ctx.request_repaint_after(std::time::Duration::from_millis(10));
    }
}

/// The offset of the current Patch drag, if one is dragging the patch (not drawing a lasso).
fn drag_offset(app: &mut PhotocraftApp) -> Option<[i32; 2]> {
    // Content-Aware patches are background jobs: the canvas shows the result when it is done.
    if app.ui.tool_options.patch_content_aware {
        return None;
    }
    let d = app.drag.as_ref().filter(|d| d.tool == Tool::Patch)?;
    let (start, mods) = (d.start, d.modifiers);
    let end = d.points.last().map_or(start, |p| [p[0], p[1]]);
    if !crate::retouch_ui::patch_drags_selection(app, start, mods) {
        return None;
    }
    Some(crate::retouch_ui::patch_offset(app, start, end))
}

/// Return the cursor-following offset while Affinity Patch has no active drag or placed box.
pub(crate) fn hover_offset(app: &mut PhotocraftApp) -> Option<[i32; 2]> {
    if !crate::retouch_ui::affinity_patch(app) || app.drag.is_some() || crate::cam_transform::active(app) {
        return None;
    }
    let at = app.patch_hover?;
    crate::retouch_ui::affinity_offset(app, at)
}

/// Build preview parameters from the current Patch box, hover position, or classic drag.
fn wanted(app: &mut PhotocraftApp) -> Option<Value> {
    if crate::retouch_ui::affinity_patch(app) {
        let place = if let Some(t) = app.ui.cam_transform.as_ref().filter(|t| t.patch.is_some()) {
            crate::cam_transform::placement(t)?
        } else {
            let offset = hover_offset(app)?;
            crate::cam_transform::patch_placement(app, offset)
        };
        if place.offset == [0, 0] && place.is_plain_move() {
            return None;
        }
        let mut params = crate::retouch_ui::patch_params(app, place.offset);
        crate::cam_transform::transform_params(&mut params, &place);
        Some(params)
    } else {
        let offset = drag_offset(app)?;
        (offset != [0, 0]).then(|| crate::retouch_ui::patch_params(app, offset.map(i64::from)))
    }
}

/// Map the original selection outline to the cursor-following Patch quad.
pub(super) fn hover_map(app: &mut PhotocraftApp) -> Option<photocraft_algo::transform::Homography> {
    let offset = hover_offset(app)?;
    let place = crate::cam_transform::patch_placement(app, offset);
    if offset == [0, 0] && place.is_plain_move() {
        return None;
    }
    let b = crate::retouch_ui::patch_bounds(app)?;
    let rect = [b.x0, b.y0, b.x1, b.y1].map(f64::from);
    photocraft_algo::transform::Homography::rect_to_quad(rect, place.quad(rect))
}

/// The document to show with the current preview, if its placement is valid.
pub(crate) fn display_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    if app.session.active_index() != Some(idx) {
        return None;
    }
    crate::cam_transform::cancel_stale(app);
    let st = app.session.documents().get(idx)?;
    let (doc, revision, layer) = (st.doc.id, st.revision, st.active_layer);
    if app.patch_preview.as_ref().is_some_and(|p| p.doc != doc || p.revision != revision) {
        app.patch_preview = None;
    }
    let Some(params) = wanted(app) else {
        if let Some(p) = &mut app.patch_preview {
            p.params = None;
            p.shown = None;
        }
        // Drain completed work while the pointer is out of range, without publishing it.
        if app.patch_worker.as_ref().is_some_and(|w| !matches!(w.result.try_recv(), Err(TryRecvError::Empty))) {
            app.patch_worker = None;
        }
        return None;
    };
    let affinity = crate::retouch_ui::affinity_patch(app);
    let motion = app.patch_motion.as_ref().filter(|m| affinity && (m.key.0, m.key.1, Some(m.key.2)) == (doc, revision, layer) && m.params == params);
    if motion.is_some_and(|m| m.moving) {
        return None;
    }
    let motion_overlay = motion.is_some();
    let p =
        app.patch_preview.get_or_insert_with(|| PatchPreview { doc, revision, params: None, layer, affinity, serial: 0, damage: VecDeque::new(), shown: None });
    if p.layer != layer || p.affinity != affinity || p.params.as_ref().is_some_and(|prior| !same_target(prior, &params)) {
        p.params = None;
        p.shown = None;
    }
    p.layer = layer;
    p.affinity = affinity;
    let min_step = (1.0 / app.point_zoom().max(1e-3)).floor().clamp(1.0, 64.0) as u32;
    if affinity && !cfg!(target_arch = "wasm32") {
        background(app, &params, min_step);
    } else if app.patch_preview.as_ref()?.params.as_ref() != Some(&params) {
        let t0 = crate::gpu_canvas::now_ms();
        let (max_cells, min_step) = if affinity { (AFFINITY_CELLS, min_step) } else { (MAX_CELLS, min_step) };
        let result = photocraft_engine::retouch_cmds::patch_preview(&app.session, &params, max_cells, min_step).ok().map(|(d, area)| display_result(d, area));
        app.patch_preview.as_mut()?.publish(params.clone(), layer, result);
        app.perf.span("patch preview", crate::gpu_canvas::now_ms() - t0);
    }
    let p = app.patch_preview.as_ref()?;
    if motion_overlay && p.params.as_ref() != Some(&params) {
        return None;
    }
    Some((p.shown.clone()?, BASE + p.serial))
}

/// What changed between preview (or document) key `seen` and `now` of the current Patch drag on
/// document `doc` at `revision`: the areas both healed. `None` when either key isn't one of this
/// preview's recent keys (the canvas then recomposites everything).
pub(crate) fn damage(app: &PhotocraftApp, doc: DocId, revision: u64, seen: u64, now: u64) -> Option<Rect> {
    let p = app.patch_preview.as_ref().filter(|p| p.doc == doc && p.revision == revision)?;
    let ours = |k: u64| k == 0 || is_preview_key(k);
    if seen == now || !ours(seen) || !ours(now) {
        return None;
    }
    let area = |k: u64| if k == 0 { Some(Rect::EMPTY) } else { p.damage.iter().find(|(key, _)| *key == k).map(|(_, area)| *area) };
    let (a, b) = (area(seen)?, area(now)?);
    Some(if a.is_empty() {
        b
    } else if b.is_empty() {
        a
    } else {
        a.union(&b)
    })
}

/// Whether the canvas shows a Patch drag's healed preview.
pub(crate) fn showing(app: &PhotocraftApp) -> bool {
    app.patch_preview.as_ref().is_some_and(|p| p.shown.is_some())
}

/// Whether `key` is a Patch drag preview key.
pub(crate) fn is_preview_key(key: u64) -> bool {
    key > BASE && key < BASE << 1
}

/// Release: after `paint.patch` succeeded. If the canvas showed these same parameters, its
/// caches count the preview as the document, so the commit's damage rect (the same healed area,
/// now solved exactly) refreshes only that area instead of everything.
pub(crate) fn committed(app: &mut PhotocraftApp, preview: Option<PatchPreview>, params: &Value) {
    let layer = app.session.active().and_then(|st| st.active_layer);
    let Some(p) = preview.filter(|p| p.shown.is_some() && p.params.as_ref() == Some(params) && p.layer == layer) else {
        return;
    };
    crate::canvas::shown_as_document(app, p.doc, is_preview_key);
}

/// Take the preview off the app (at release, before the commit changes the document).
pub(crate) fn take(app: &mut PhotocraftApp) -> Option<PatchPreview> {
    app.patch_preview.take()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::{ToolEvent, tool_event};

    fn red_at(d: &Document, x: i32, y: i32) -> bool {
        d.layers[0].surface().unwrap().rgba(x, y)[1] < 0.5
    }

    /// Drive the preview until the native worker publishes a result, with a bounded wait.
    fn wait_for_preview(app: &mut PhotocraftApp) -> Arc<Document> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let shown = display_doc(app, 0);
            if app.patch_worker.is_none() {
                return shown.expect("the worker produced the current preview").0;
            }
            assert!(std::time::Instant::now() < deadline, "patch preview timed out");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    /// Exercise the full Affinity placement lifecycle, including repeated placement and Undo.
    #[test]
    fn affinity_patch_previews_places_transforms_and_applies_on_outside_click() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 144, "height": 96})).unwrap();
        app.run("paint.pencil", json!({"points": [[22, 38], [34, 38]], "size": 4, "color": "#ff0000"})).unwrap();
        app.run("select.rect", json!({"x": 12, "y": 22, "width": 32, "height": 32})).unwrap();
        app.ui.tool = Tool::Patch;
        app.ui.tool_options.patch_affinity = true;
        app.ui.tool_options.patch_mode = "destination".into();
        app.ui.views[0].zoom = 1.0;
        let doc = |app: &PhotocraftApp| app.session.documents()[0].doc.clone();
        let steps = app.session.documents()[0].history.past_len();
        let m = egui::Modifiers::NONE;
        let click = |app: &mut PhotocraftApp, x, y| {
            tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, m);
            tool_event(app, ToolEvent::Up { x, y }, m);
        };
        let drag = |app: &mut PhotocraftApp, [x, y]: [f64; 2], [end_x, end_y]: [f64; 2]| {
            tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, m);
            tool_event(app, ToolEvent::Move { x: end_x, y: end_y, pressure: 1.0 }, m);
            tool_event(app, ToolEvent::Up { x: end_x, y: end_y }, m);
        };

        tool_event(&mut app, ToolEvent::Move { x: 74.0, y: 38.0, pressure: 1.0 }, m);
        display_doc(&mut app, 0);
        assert!(app.patch_worker.is_some());
        tool_event(&mut app, ToolEvent::Move { x: 88.0, y: 38.0, pressure: 1.0 }, m);
        let shown = wait_for_preview(&mut app);
        assert!(red_at(&shown, 88, 38) && !red_at(&shown, 74, 38), "the preview catches up to the latest pointer");

        assert!(!red_at(&doc(&app), 88, 38));

        click(&mut app, 88.0, 38.0);
        assert!(crate::cam_transform::active(&app));
        assert_eq!(app.session.documents()[0].history.past_len(), steps);
        // Once placed, hovering elsewhere does not move the patch.
        tool_event(&mut app, ToolEvent::Move { x: 130.0, y: 80.0, pressure: 1.0 }, m);
        assert!(red_at(&wait_for_preview(&mut app), 88, 38));

        drag(&mut app, [88.0, 38.0], [96.0, 38.0]);
        tool_event(&mut app, ToolEvent::Down { x: 123.0, y: 63.0, pressure: 1.0 }, m);
        tool_event(&mut app, ToolEvent::Move { x: 71.0, y: 65.0, pressure: 1.0 }, m);
        tool_event(&mut app, ToolEvent::Move { x: 123.0, y: 63.0, pressure: 1.0 }, m);
        tool_event(&mut app, ToolEvent::Up { x: 123.0, y: 63.0 }, m);
        assert!(crate::cam_transform::active(&app), "a rotation drag back to its start does not apply the patch");
        assert_eq!(app.session.documents()[0].history.past_len(), steps);
        drag(&mut app, [123.0, 63.0], [71.0, 65.0]);
        let place = crate::cam_transform::placement(app.ui.cam_transform.as_ref().unwrap()).unwrap();
        assert_eq!(place.offset, [68, 0]);
        assert!((place.rotate - 90.0).abs() < 0.001, "{place:?}");
        crate::cam_transform::set_patch_transform(&mut app, [125.0; 2], 90.0);
        let place = crate::cam_transform::placement(app.ui.cam_transform.as_ref().unwrap()).unwrap();
        assert_eq!((place.offset, place.scale), ([68, 0], [125.0; 2]));
        let shown = wait_for_preview(&mut app);
        assert!(red_at(&shown, 96, 34) && !red_at(&shown, 101, 38));
        assert!(!red_at(&doc(&app), 96, 34));

        click(&mut app, 135.0, 80.0);
        assert!(!crate::cam_transform::active(&app) && !app.ui.status_error, "{}", app.ui.status);
        assert!(red_at(&doc(&app), 96, 34) && red_at(&doc(&app), 28, 38));
        assert!(doc(&app).selection.is_some(), "the source remains selected for another placement");
        assert_eq!(app.session.documents()[0].history.past_len(), steps + 1);
        let next = wanted(&mut app).expect("the applied patch follows the cursor again");
        assert_eq!(next["rotate"], json!(90.0));
        click(&mut app, 62.0, 69.0);
        let second = crate::cam_transform::placement(app.ui.cam_transform.as_ref().unwrap()).unwrap();
        assert_eq!(second.rotate, place.rotate);
        assert_eq!(second.scale, place.scale);
        click(&mut app, 140.0, 92.0);
        assert!(red_at(&doc(&app), 62, 65) && red_at(&doc(&app), 96, 34));
        assert_eq!(app.session.documents()[0].history.past_len(), steps + 2);
        app.run("edit.undo", json!({})).unwrap();
        assert!(doc(&app).selection.is_some() && red_at(&doc(&app), 96, 34) && !red_at(&doc(&app), 62, 65));
    }

    /// Check that the transformed sample can follow the cursor closer to the canvas edge.
    #[test]
    fn affinity_source_clamps_the_transformed_sample_at_the_canvas_edge() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 144, "height": 96})).unwrap();
        app.run("select.rect", json!({"x": 12, "y": 22, "width": 40, "height": 20})).unwrap();
        app.ui.tool_options.patch_mode = "source".into();
        app.ui.tool_options.patch_scale = [50.0; 2];
        app.ui.tool_options.patch_rotation = 90.0;

        assert_eq!(crate::retouch_ui::affinity_offset(&mut app, [7.0, 32.0]), Some([-25, 0]));
        assert_eq!(crate::retouch_ui::affinity_offset(&mut app, [0.0, 32.0]), Some([-27, 0]));
    }

    /// Check that pointer motion reuses the sample texture and delays the solve until it settles.
    #[test]
    fn moving_destination_patch_reuses_one_texture_and_defers_healing_until_rest() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 144, "height": 96})).unwrap();
        app.run("select.rect", json!({"x": 92, "y": 0, "width": 52, "height": 96})).unwrap();
        app.run("edit.fill", json!({"color": "#404040"})).unwrap();
        app.run("select.rect", json!({"x": 12, "y": 22, "width": 32, "height": 32})).unwrap();
        app.ui.tool = Tool::Patch;
        app.ui.tool_options.patch_affinity = true;
        app.ui.tool_options.patch_mode = "destination".into();
        let ctx = egui::Context::default();
        let mut frame = |time, x| {
            app.patch_hover = Some([x, 38.0]);
            let mut output = ctx.run_ui(egui::RawInput { time: Some(time), ..Default::default() }, |ui| {
                update_motion(&mut app, ui.ctx(), 0, None);
            });
            output.textures_delta.clear();
            let preview = display_doc(&mut app, 0);
            let motion = app.patch_motion.as_ref().expect("the normal raster layer has a motion preview");
            (motion.texture.id(), motion.quad, motion.moving, preview.is_some(), app.patch_worker.is_some(), motion.tone)
        };
        let first = frame(0.0, 74.0);
        let second = frame(0.02, 112.0);
        assert_eq!(first.0, second.0, "moving the quad reuses the uploaded sample");
        assert_ne!(first.1, second.1);
        assert!(second.5[0][0] < first.5[0][0] - 100, "the patch darkens as it moves onto the grey destination");
        assert_eq!((second.2, second.3, second.4), (true, false, false));
        let edge = frame(0.04, 143.0);
        assert_eq!(edge.0, first.0, "the same texture survives crossing the canvas edge");
        assert_eq!((edge.1[0][0] + edge.1[2][0]) / 2.0, 143.0);
        assert!(edge.1[2][0] > 144.0 && edge.2 && !edge.4);
        let settled = frame(0.2, 143.0);
        assert!(!settled.2 && settled.4, "settling starts the healed preview worker even at the edge");
        wait_for_preview(&mut app);
    }

    /// Check that classic drag shows a temporary heal and only commits it on release.
    #[test]
    fn dragging_the_patch_previews_the_heal_and_release_commits_it() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 100, "height": 60})).unwrap();
        app.run("paint.pencil", json!({"points": [[68, 30], [74, 30]], "size": 6, "color": "#ff0000"})).unwrap();
        app.run("select.lasso", json!({"points": [[60, 20], [84, 20], [84, 40], [60, 40]]})).unwrap();
        app.ui.tool = Tool::Patch;
        let m = egui::Modifiers::NONE;
        let doc = |app: &PhotocraftApp| app.session.documents()[0].doc.clone();
        let (id, rev) = (doc(&app).id, app.session.documents()[0].revision);

        // Nothing to preview before the patch moves, or while a lasso is drawn.
        tool_event(&mut app, ToolEvent::Down { x: 70.0, y: 30.0, pressure: 1.0 }, m);
        assert!(display_doc(&mut app, 0).is_none());
        tool_event(&mut app, ToolEvent::Move { x: 50.0, y: 30.0, pressure: 1.0 }, m);
        let (shown, k1) = display_doc(&mut app, 0).expect("a preview while dragging");
        assert!(is_preview_key(k1) && showing(&app));
        assert!(!red_at(&shown, 71, 30), "the preview shows the blemish healed");
        assert!(red_at(&doc(&app), 71, 30), "the document is untouched until release");
        let a = damage(&app, id, rev, 0, k1).unwrap();
        assert!(a.contains(71, 30) && !a.contains(20, 30), "only the healed area recomposites: {a:?}");
        // Same offset: the same preview, no new solve.
        assert_eq!(display_doc(&mut app, 0).unwrap().1, k1);
        tool_event(&mut app, ToolEvent::Move { x: 30.0, y: 30.0, pressure: 1.0 }, m);
        let k2 = display_doc(&mut app, 0).unwrap().1;
        assert!(k2 != k1 && damage(&app, id, rev, k1, k2).is_some_and(|r| r.contains(71, 30)));
        assert!(damage(&app, id, rev, k1, 12345).is_none(), "foreign keys recomposite everything");

        tool_event(&mut app, ToolEvent::Up { x: 30.0, y: 30.0 }, m);
        assert!(!app.ui.status_error, "{}", app.ui.status);
        assert!(!red_at(&doc(&app), 71, 30), "release commits the patch");
        assert!(display_doc(&mut app, 0).is_none() && !showing(&app));

        // A drag starting outside the selection draws a lasso: no preview.
        tool_event(&mut app, ToolEvent::Down { x: 5.0, y: 5.0, pressure: 1.0 }, m);
        tool_event(&mut app, ToolEvent::Move { x: 30.0, y: 10.0, pressure: 1.0 }, m);
        assert!(display_doc(&mut app, 0).is_none());
    }
}
