use craft::behavior::CanvasView;

use super::display::{GPU_OUTPUT, LiveProposal, canvas_display, display_doc_live, texture_key};
use super::filter_preview::filter_channel_preview;
use super::navigator::navigator_waits_for_drag;
use super::strokes::live_stroke;
use super::{Document, PhotocraftApp};
use photocraft_doc::LayerContent;
use photocraft_geom::Rect as DRect;

/// Largest texture side we upload; bigger pictures are uploaded downsampled.
pub const MAX_TEXTURE: u32 = 4096;

/// The document area a filter preview of `preview` size covers, for a `doc` sized document
/// previewed at proxy factor `k`: the document's bounds when it is the document's size (reduced by
/// `k`), else its own size (times `k`) centred on the document's centre, as the GPU canvas centres
/// a size-changing preview (Image › Rotation).
pub(super) fn preview_bounds(doc: photocraft_geom::Size, preview: photocraft_geom::Size, k: u32) -> DRect {
    let k = k.max(1);
    let full = DRect::from_size(doc);
    if doc.width.div_ceil(k) == preview.width && doc.height.div_ceil(k) == preview.height {
        return full;
    }
    let side = |p: u32, d: u32| {
        let covered = i64::from(p) * i64::from(k);
        let x0 = (i64::from(d) - covered).div_euclid(2);
        let clamp = |v: i64| v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        (clamp(x0), clamp(x0 + covered))
    };
    let (x0, x1) = side(preview.width, doc.width);
    let (y0, y1) = side(preview.height, doc.height);
    DRect::new(x0, y0, x1, y1)
}

/// What a canvas shows of document `idx` on `output` (`None`: the main window's display) this pass, before any
/// texture: the committed document or its live preview, and the display's colour management. The CPU canvas cache and
/// the raster sources start from it.
pub(crate) struct CanvasPicture {
    /// The document's revision.
    pub revision: u64,
    /// The document's damage span start and its last damage, grown by its layer effects' reach.
    pub last_damage: (u64, Option<DRect>),
    pub id: photocraft_doc::DocId,
    /// The document or the preview shown, and its preview key (raw: the display key is not folded in).
    pub doc: std::sync::Arc<Document>,
    pub preview_key: u64,
    /// Where it lies in the document: the document's bounds, or a filter preview's own area.
    pub bounds: DRect,
    /// Its pixels are not the document's: a preview reduced for a zoomed-out view, or one that changes the canvas size.
    pub reduced: bool,
    pub display: Option<std::sync::Arc<photocraft_engine::display_color::CanvasDisplay>>,
    pub display_key: u64,
}

/// What a [`CanvasPicture`] shows while an adjustment dialog previews a large document as a GPU proxy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProxyPreview {
    /// The committed document: drawn here, the preview would cost a full-size composite per change.
    Document,
    /// The live preview, as the channel views draw it: an opaque view over the document would hide the live change
    /// (#2078).
    Preview,
}

/// [`CanvasPicture`] of document `idx` on `output`, showing `proxy` during a proxy adjustment preview; `None` when
/// no such document is open. `live` is the gradient a tool proposes this pass: the picture shows it as the GPU canvas
/// does.
pub(crate) fn canvas_picture(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    idx: usize,
    output: Option<u32>,
    proxy: ProxyPreview,
    live: LiveProposal<'_>,
) -> Option<CanvasPicture> {
    let (revision, last_damage, id, committed) = {
        let st = app.session.documents().get(idx)?;
        let reach = |r: DRect| if r.is_empty() { r } else { r.inflate(effect_reach(&st.doc.layers)) };
        (st.revision, (st.damage_from(), st.last_damage.map(reach)), st.doc.id, st.doc.bounds())
    };
    let (mut doc, mut preview_key) = display_doc_live(app, idx, live);
    // A filter dialog's live preview: the GPU canvas draws it from its own texture, but the CPU canvas (no usable
    // GPU, a flipped or pixel-aspect-corrected view, a GPU fallback) and the raster sources draw its result, as the
    // channel views draw it. A preview reduced for a zoomed-out view (a proxy, factor `k`) covers the whole document
    // at its own size. One whose dimensions change otherwise (Image › Rotation) covers its own area, centred on the
    // document's, as the GPU canvas draws it (`preview_bounds`).
    let mut bounds = committed;
    let mut reduced = false;
    if let Some((preview, key)) = filter_channel_preview(app, idx, ctx) {
        let k = app.filter_preview.as_ref().map_or(1, |p| p.key.k.max(1));
        bounds = preview_bounds(committed.size(), preview.size, k);
        (doc, preview_key, reduced) = (preview, key, k > 1);
    }
    let reduced = reduced || bounds != committed;
    // An adjustment dialog's preview of a large document is taken by the GPU path as a proxy (`ProxyPreview`). The
    // display key depends on the document, so this comes before it.
    if proxy == ProxyPreview::Document
        && crate::adjust_preview::shown_key(app) == Some(preview_key)
        && let Some(st) = app.session.documents().get(idx)
        && crate::proxy::preview_factor(&st.doc, crate::proxy::reduced_previews(app)) > 1
    {
        (doc, preview_key) = (st.doc.clone(), 0);
    }
    let (display, display_key) = canvas_display(app, &doc, output);
    Some(CanvasPicture { revision, last_damage, id, doc, preview_key, bounds, reduced, display, display_key })
}

/// What changed since a canvas cache showed (`revision`, `preview key`) `seen`, when only a
/// rectangle did: the document's damage since revision `damage_from` (one revision, or a whole
/// command's span: [`photocraft_engine::DocState::damage_from`]), or the live stroke's dabs since
/// then. Cached keys have the colour display's key folded in (`^ display_key`); `now`'s is still raw.
pub(crate) fn damage_since(
    app: &PhotocraftApp,
    idx: usize,
    seen: (u64, u64),
    now: (u64, u64),
    display_key: u64,
    (damage_from, last_damage): (u64, Option<DRect>),
) -> Option<DRect> {
    // A brush stroke committed with the pixels its live preview already showed: only the steps
    // this cache hadn't shown yet (usually nothing), not the stroke's whole bounding box.
    if seen.0 + 1 == now.0
        && let Some(st) = app.session.documents().get(idx)
        && let Some(c) = app.committed_stroke.as_ref().filter(|c| c.doc == st.doc.id && c.revision == now.0)
        && let Some(r) = c.since(seen.1 ^ display_key)
    {
        let r = if c.relayout.is_empty() {
            r
        } else if r.is_empty() {
            c.relayout
        } else {
            r.union(&c.relayout)
        };
        return Some(if r.is_empty() { r } else { r.inflate(effect_reach(&st.doc.layers)) });
    }
    // `seen` is within the span the document's damage covers.
    let in_span = seen.0 >= damage_from && seen.0 < now.0;
    if seen.1 == now.1 ^ display_key && in_span {
        return last_damage;
    }
    // A canvas can miss several revisions between passes. Use their accumulated engine damage
    // only when the preview and display key stayed the same; an unknown span forces a full read.
    // Each recorded rectangle already includes the effect reach at the time it was changed.
    if seen.1 == now.1 ^ display_key
        && seen.0 < now.0
        && let Some(st) = app.session.documents().get(idx)
        && let Some(r) = st.damage_since(seen.0)
    {
        return Some(r);
    }
    // Opening, canceling or committing a Free Transform box: the moving layer and the layers
    // drawn over the box, not the whole document (a gigapixel canvas took over a second).
    if let Some(st) = app.session.documents().get(idx)
        && let Some(r) = crate::transform_tool::damage(app, st.doc.id, (seen.0, seen.1 ^ display_key), now, last_damage)
    {
        return Some(if r.is_empty() { r } else { r.inflate(effect_reach(&st.doc.layers)) });
    }
    if seen.0 == now.0
        && let Some(st) = app.session.documents().get(idx)
        && let Some(r) = crate::type_transform::damage(app, st.doc.id, now.0, seen.1 ^ display_key, now.1)
            .or_else(|| crate::type_path_placement::damage(app, st.doc.id, now.0, seen.1 ^ display_key, now.1))
    {
        return Some(if r.is_empty() { r } else { r.inflate(effect_reach(&st.doc.layers)) });
    }
    // Between an adjustment dialog's previews (and the document): the target's area.
    if seen.0 == now.0
        && let Some(st) = app.session.documents().get(idx)
        && let Some(r) = crate::adjust_preview::switch_region(app, st.doc.id, now.0, seen.1 ^ display_key, now.1)
    {
        return Some(r);
    }
    // Between a Properties-panel adjustment-layer drag's previews (and the document): the
    // layer's area. Without this every parameter value recomposited the whole document (#6839).
    if seen.0 == now.0
        && let Some(st) = app.session.documents().get(idx)
        && let Some(r) = crate::adjust_preview::live_switch_region(app, st.doc.id, now.0, seen.1 ^ display_key, now.1)
    {
        return Some(r);
    }
    // Between a Move drag's offsets: where the moving layers were and are. Revisions that changed
    // no pixels since `seen` (Auto-Select's pick at the press) count as the same document.
    if (seen.0 == now.0 || (in_span && last_damage.is_some_and(|r| r.is_empty())))
        && let Some(st) = app.session.documents().get(idx)
        && let Some(r) = crate::move_ui::damage(app, st.doc.id, now.0, seen.1 ^ display_key, now.1)
    {
        return Some(if r.is_empty() { r } else { r.inflate(effect_reach(&st.doc.layers)) });
    }
    // Between a Patch drag's previews: the areas they healed.
    if seen.0 == now.0
        && let Some(st) = app.session.documents().get(idx)
        && let Some(r) = crate::patch_preview::damage(app, st.doc.id, now.0, seen.1 ^ display_key, now.1)
    {
        return Some(if r.is_empty() { r } else { r.inflate(effect_reach(&st.doc.layers)) });
    }
    // Between hovered blend modes (and the document): where the layer's blending shows.
    if seen.0 == now.0
        && let Some(st) = app.session.documents().get(idx)
        && let Some(r) = crate::blend_preview::damage(app, st.doc.id, now.0, seen.1 ^ display_key, now.1)
    {
        return Some(if r.is_empty() { r } else { r.inflate(effect_reach(&st.doc.layers)) });
    }
    // Between colours under a text or stop Color Picker (and the document): the target layers' bounds.
    if seen.0 == now.0
        && let Some(st) = app.session.documents().get(idx)
        && let Some(r) = crate::color_picker_preview::damage(app, st.doc.id, now.0, seen.1 ^ display_key, now.1)
    {
        return Some(if r.is_empty() { r } else { r.inflate(effect_reach(&st.doc.layers)) });
    }
    // Between hovered fonts (and the document): the text layer's old/new ink bounds.
    if seen.0 == now.0
        && let Some(st) = app.session.documents().get(idx)
        && let Some(r) = crate::type_font_preview::damage(app, st.doc.id, now.0, seen.1 ^ display_key, now.1)
    {
        return Some(if r.is_empty() { r } else { r.inflate(effect_reach(&st.doc.layers)) });
    }
    // Between a dragged paragraph box's renders (and the document): the layer's old/new ink bounds.
    if seen.0 == now.0
        && let Some(st) = app.session.documents().get(idx)
        && let Some(r) = crate::type_box_preview::damage(app, st.doc.id, now.0, seen.1 ^ display_key, now.1)
    {
        return Some(if r.is_empty() { r } else { r.inflate(effect_reach(&st.doc.layers)) });
    }
    let l = live_stroke(app, idx).filter(|l| seen.0 == now.0 && l.display_key() == now.1)?;
    let r = l.since(seen.1 ^ display_key)?;
    Some(if r.is_empty() { r } else { r.inflate(effect_reach(&l.doc().layers)) })
}

/// Document `doc`'s canvas caches showed a preview that the edit just committed reproduces
/// (`was_preview` tells its keys): count it as the document itself, so the commit's damage rect
/// refreshes only that area instead of everything.
pub(crate) fn shown_as_document(app: &mut PhotocraftApp, doc: photocraft_doc::DocId, was_preview: impl Fn(u64) -> bool) {
    let Some(d) = app.session.documents().iter().find(|st| st.doc.id == doc).map(|st| st.doc.clone()) else { return };
    // Every display's cache of the document: the GPU state folds in the texture key, CPU
    // textures their display's key.
    let outputs: Vec<u32> = app.canvases.keys().filter(|k| k.0 == doc).map(|k| k.1).collect();
    for out in outputs {
        let (display, _) = canvas_display(app, &d, (out != 0 && out != GPU_OUTPUT).then_some(out));
        let gpu_key = texture_key(display.as_deref(), &d);
        let Some(c) = app.canvases.get_mut(&(doc, out)) else { continue };
        if was_preview(c.preview_key ^ gpu_key) {
            c.preview_key = gpu_key;
        }
    }
    // The raster sources' pictures of the document, keyed like the CPU textures.
    let outputs: Vec<u32> = app.raster_logs.keys().filter(|k| k.0 == doc).map(|k| k.1).collect();
    for out in outputs {
        let (_, key) = canvas_display(app, &d, (out != 0).then_some(out));
        if let Some(log) = app.raster_logs.get_mut(&(doc, out)) {
            log.shown_as_document(key, &was_preview);
        }
    }
}

/// How far beyond an edit's damage rect the composite can change: layer effects (shadows, glows,
/// strokes, …) on the edited layer and on the groups around it reach that far.
pub(crate) fn effect_reach(layers: &[photocraft_doc::Layer]) -> i32 {
    layers
        .iter()
        .filter(|l| l.visible)
        .map(|l| {
            let own = if photocraft_compose::effects::has_effects(l) { photocraft_compose::effects::margin(l) } else { 0 };
            let inner = match &l.content {
                LayerContent::Group(g) => effect_reach(&g.children),
                _ => 0,
            };
            own + inner
        })
        .max()
        .unwrap_or(0)
}

/// GPU path: make sure document `idx` is current in the GPU canvas. Edits draw their damage;
/// unchanged lower-stack composites can be reused by the compositor inside that region.
/// Returns false if there is no GPU canvas. `live` is the gradient a tool proposes this pass.
pub(super) fn ensure_gpu(app: &mut PhotocraftApp, ctx: &egui::Context, idx: usize, visible: DRect, live: LiveProposal<'_>) -> bool {
    let Some(gpu) = app.gpu.clone() else { return false };
    let Some((revision, last_damage, id)) =
        app.session.documents().get(idx).map(|st| {
            (st.revision, (st.damage_from(), st.last_damage.map(|r| if r.is_empty() { r } else { r.inflate(effect_reach(&st.doc.layers)) })), st.doc.id)
        })
    else {
        return false;
    };
    let (doc, raw_key) = display_doc_live(app, idx, live);
    let (display, _) = canvas_display(app, &doc, None);
    // Repeated pattern tiles sample the same texture, including pixels outside the unwrapped
    // viewport. Refresh the affected document region while Pattern Preview is enabled.
    let blend_visible = if app.ui.view.pattern_preview { doc.bounds() } else { visible };
    // The texture is shared by every display: only what it stores counts, not the monitor.
    let display_key = texture_key(display.as_deref(), &doc);
    let seen = app.canvases.get(&(id, GPU_OUTPUT)).map(|c| (c.revision, c.preview_key));
    let damage_start = crate::gpu_canvas::now_ms();
    let mut damage = seen.and_then(|seen| damage_since(app, idx, seen, (revision, raw_key), display_key, last_damage));
    if let Some(region) = damage {
        damage = Some(crate::blend_preview::clip_damage(app, id, revision, raw_key, region, blend_visible));
    }
    app.perf.stage.damage_ms += crate::gpu_canvas::now_ms() - damage_start;
    // To an adjustment dialog's preview: only what the view shows now (the rest as it moves there).
    if damage.is_some()
        && let Some(r) = seen.filter(|s| s.0 == revision).and_then(|s| crate::adjust_preview::switch_region(app, id, revision, s.1 ^ display_key, raw_key))
    {
        damage = Some(crate::adjust_preview::switch_damage(app, raw_key, r, visible));
    }
    // The same for a Properties-panel drag's previews.
    if damage.is_some()
        && let Some(r) = seen.filter(|s| s.0 == revision).and_then(|s| crate::adjust_preview::live_switch_region(app, id, revision, s.1 ^ display_key, raw_key))
    {
        damage = Some(crate::adjust_preview::live_switch_damage(app, raw_key, r, visible));
    }
    let preview_key = raw_key ^ display_key;
    // The Navigator may draw this texture while it holds the plain document or a drag preview
    // that updates the full texture (not an adjustment preview composited only where the view is).
    if raw_key == 0 || navigator_waits_for_drag(app, idx) {
        app.navigator_plain.insert(id);
    } else {
        app.navigator_plain.remove(&id);
        app.navigator_gpu.remove(&id);
    }
    let size = [doc.size.width, doc.size.height];
    // A full CPU redraw running in the background: show the blocks it has finished.
    crate::background_redraw::poll(app, &gpu, id);
    let cache = app.canvases.entry((id, GPU_OUTPUT)).or_default();
    let present = cache.on_gpu && gpu.has(id.0, size);
    let current = cache.revision == revision && cache.preview_key == preview_key;
    // GPU Memory or Memory Usage changed with nothing to redraw: the new budget, and whether the
    // layers fit it, show at once (the status bar's GPU Memory Full).
    if present && app.perf.gpu_budget_allowance != budget_allowance(app) {
        gpu_budget(app, &gpu, idx, false, visible);
    }
    // Switched to a document whose canvas is already current: size the budget for it and measure
    // whether its layers fit, so GPU Memory Full shows (or clears) at once rather than at its
    // next full refresh. Once per switch: the fit then names this document.
    if present && app.perf.gpu_layer_fit.is_none_or(|f| f.doc != id.0) {
        gpu_budget(app, &gpu, idx, false, visible);
        let fit = gpu.measure_fit(&doc).unwrap_or(crate::gpu_canvas::LayerFit { doc: id.0, needed: 0, budget: 0 });
        crate::render_path::layer_fit(app, fit);
    }
    if present && current {
        // An adjustment preview composited where the view was: catch up where it moved to.
        if let Some(r) = crate::adjust_preview::uncovered(app, id, raw_key, visible) {
            let r = gpu.refresh(id.0, &doc, Some(r), display.as_deref());
            app.perf.record_refresh(&r);
        }
        if let Some(r) = crate::adjust_preview::live_uncovered(app, id, raw_key, visible) {
            let r = gpu.refresh(id.0, &doc, Some(r), display.as_deref());
            app.perf.record_refresh(&r);
        }
        // Blend previews keep offscreen pixels untouched until the view reaches them.
        // Use the normal refresh path so CPU fallback and pending redraws stay synchronized.
        damage = crate::blend_preview::uncovered(app, id, revision, raw_key, blend_visible);
        if damage.is_none() {
            return true;
        }
    }
    // Display textures for a canvas this large would not fit the GPU: the reduced CPU canvas
    // draws it (checked before the memory budget, which walks the history, every frame).
    if !gpu.fits_canvas(&doc) {
        crate::background_redraw::cancel(app, id);
        // Drawn on the CPU canvas: no layer budget applies to it.
        if app.perf.gpu_layer_fit.is_some_and(|f| f.doc == id.0) {
            app.perf.gpu_layer_fit = None;
        }
        let (bytes, budget) = gpu.canvas_fit(&doc).unwrap_or_default();
        crate::render_path::note(app, id.0, crate::render_path::Path::Cpu(crate::render_path::CpuReason::CanvasTooLarge { bytes, budget }));
        return false;
    }
    let partial = present && damage.is_some();
    let budget_start = crate::gpu_canvas::now_ms();
    gpu_budget(app, &gpu, idx, partial, visible);
    app.perf.stage.damage_ms += crate::gpu_canvas::now_ms() - budget_start;
    let defer = crate::background_redraw::enabled(app);
    let r = gpu.refresh_with(id.0, &doc, if partial { damage } else { None }, display.as_deref(), defer);
    app.perf.gpu_layer_fit = gpu.layer_fit();
    if r.kind == "lost" || r.kind == crate::gpu_canvas::REFRESH_OVER_BUDGET {
        // The device is gone (`gpu_status` then drops the GPU canvas) or the canvas is over the
        // texture budget: draw this frame on the CPU path.
        crate::background_redraw::cancel(app, id);
        if let Some(reason) = r.fallback {
            crate::render_path::note(app, id.0, crate::render_path::Path::Cpu(reason));
        }
        return false;
    }
    if r.kind == crate::gpu_canvas::REFRESH_DEFERRED {
        // Composited off the UI thread, the visible part first (#4024).
        let region = if partial { damage.unwrap_or(doc.bounds()) } else { doc.bounds() };
        crate::background_redraw::start(app, ctx, &gpu, doc.clone(), display.clone(), visible, region, r.fallback.clone(), r.gpu_attempt_ms);
    } else if partial {
        // The edit drew its damage; blocks of a running redraw still to come follow the edit.
        crate::background_redraw::restart(app, ctx, &gpu, doc.clone(), display.clone(), visible);
    } else {
        crate::background_redraw::cancel(app, id);
    }
    match &r.fallback {
        Some(reason) => crate::render_path::note(app, id.0, crate::render_path::Path::Cpu(reason.clone())),
        None if r.kind.starts_with("gpu") => crate::render_path::note(app, id.0, crate::render_path::Path::Gpu { full: r.kind == "gpu-full" }),
        None => {}
    }
    if r.kind != crate::gpu_canvas::REFRESH_DEFERRED {
        app.perf.record_refresh(&r);
    }
    if r.kind == "full" {
        crate::notices::slow_cpu_refresh(app, id.0, r.composite_ms + r.upload_ms, r.fallback.as_ref());
    }
    if r.kind.starts_with("gpu") {
        app.perf.gpu_uploads = r.uploads;
    }
    let Some(cache) = app.canvases.get_mut(&(id, GPU_OUTPUT)) else { return true };
    cache.revision = revision;
    cache.preview_key = preview_key;
    cache.on_gpu = true;
    true
}

/// Point the wgpu compositor at what the view shows, and size its memory budget from GPU Memory,
/// or Memory Usage, the document's pixels and History (charged where the GPU shares the system's
/// memory), physical memory and the GPU's own: on full refreshes (pixels counted once per
/// structural change, not per brush dab) and whenever either preference changes.
fn gpu_budget(app: &mut PhotocraftApp, gpu: &crate::gpu_canvas::GpuCanvas, idx: usize, partial: bool, visible: DRect) {
    gpu.set_focus(Some(visible));
    let allowance = budget_allowance(app);
    let changed = app.perf.gpu_budget_allowance != allowance;
    if partial && !changed && gpu.memory_budget().is_some() {
        return;
    }
    let Some(st) = app.session.documents().get(idx) else { return };
    let pixels = st.memory_bytes() as u64;
    let budget = crate::gpu_canvas::memory_budget(allowance.0, pixels, crate::gpu_canvas::physical_memory(), gpu.video_memory(), allowance.1);
    gpu.set_memory_budget(budget);
    app.perf.gpu_budget_mb = budget >> 20;
    app.perf.gpu_budget_allowance = allowance;
    if changed && let Some(fit) = gpu.measure_fit(&st.doc) {
        crate::render_path::layer_fit(app, fit);
    }
}

/// Memory Usage and GPU Memory (Preferences › Performance), in bytes.
fn budget_allowance(app: &PhotocraftApp) -> (u64, u64) {
    let performance = &app.session.prefs().performance;
    (u64::from(performance.effective_memory_usage_mb()).saturating_mul(1 << 20), u64::from(performance.gpu_memory_mb).saturating_mul(1 << 20))
}

/// The document pixels a view shows, with a margin for filtering. Uses all four canvas corners
/// so a rotated camera still covers the visible tiles.
pub(crate) fn visible_doc_rect(view: &CanvasView) -> DRect {
    let r = view.visible();
    let c = |v: f64| v.clamp(-1e9, 1e9) as i32;
    DRect::new(c(r.x0.floor()) - 2, c(r.y0.floor()) - 2, c(r.x1.ceil()) + 2, c(r.y1.ceil()) + 2)
}

/// Prioritize deferred redraw blocks visible at either the target or the animated display camera.
/// The displayed camera may still show old texture contents outside the target's visible area.
pub(super) fn redraw_focus(target: &CanvasView, displayed: &CanvasView) -> DRect {
    visible_doc_rect(target).union(&visible_doc_rect(displayed))
}

/// A CPU composite as the GPU canvas texture stores it (sRGB-encoded for linear documents).
pub(super) fn texture_buffer<'a>(
    display: Option<&photocraft_engine::display_color::CanvasDisplay>,
    buf: &'a photocraft_compose::Buffer,
) -> std::borrow::Cow<'a, photocraft_compose::Buffer> {
    match display {
        Some(d) => d.texture_buffer(buf),
        None => std::borrow::Cow::Borrowed(buf),
    }
}

pub(crate) fn retain_gpu_documents(app: &mut PhotocraftApp) {
    // Remove a closed adjustment owner's proxy before collecting its live GPU keys.
    crate::adjust_preview::retain_documents(app);
    if let Some(gpu) = &app.gpu {
        // Keep each document's texture plus its preview textures (filter preview, adjustment proxy);
        // retaining only document ids freed the previews every frame (blank canvas while previewing).
        let mut live: Vec<u64> = app
            .session
            .documents()
            .iter()
            .flat_map(|st| [st.doc.id.0, st.doc.id.0 ^ (1u64 << 61), st.doc.id.0 ^ (1u64 << 62), super::viewport_pages::stream_key(st.doc.id.0)])
            .collect();
        live.extend(crate::adjust_preview::gpu_keys(app));
        gpu.retain(&live);
    }
    crate::background_redraw::retain(app);
    // Page work for closed documents (or of a dropped GPU canvas) is cancelled.
    if app.gpu.is_none() {
        app.streams.clear();
    } else {
        let ids: Vec<_> = app.session.documents().iter().map(|st| st.doc.id).collect();
        app.streams.retain(|id, _| ids.contains(id));
    }
    // Upload markers must not outlive the resources they describe: native reopen can reuse
    // both the document ID and revision before the next frame while a preview dialog stays open.
    let documents = app.session.documents();
    if app.filter_preview.as_ref().is_some_and(|preview| !documents.iter().any(|st| st.doc.id == preview.key.doc)) {
        app.filter_preview = None;
    }
    if app.proxy_uploaded.is_some_and(|(doc, _)| !documents.iter().any(|st| st.doc.id == doc)) {
        app.proxy_uploaded = None;
        app.proxy = None;
    }
}
