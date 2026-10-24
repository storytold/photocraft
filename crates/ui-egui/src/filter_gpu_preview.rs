//! Gaussian Blur's live preview on the GPU (#5772): while the dialog is open, the target layer's
//! content is a GPU layer operator (`photocraft_gpu::GaussianBlur`) and the canvas composites the
//! committed document with it, only where the view (and the dialog's preview window) shows, under
//! a session-tagged preview key (`preview_session`). A slider change re-runs the blur on the GPU;
//! nothing is read back or flattened on the CPU.
//!
//! Anything this path can't preview exactly stays on the CPU preview (`canvas::ensure_filter_preview`),
//! unchanged: see [`ineligible`]. If the GPU fails to draw it (out of memory, a device fault), the
//! session switches to the CPU preview for the rest of the dialog ([`failed`]): the layer is never
//! shown unfiltered under the preview's key.
//!
//! After OK the operator stays until the background job that applies the filter lands, so the
//! canvas never flashes the unfiltered image ([`confirmed`]).

use std::sync::Arc;

use photocraft_doc::{DocId, Document, LayerContent, LayerId};
use photocraft_geom::Rect;
use photocraft_gpu::{GaussianBlur, LayerOperator, SeparableBlur};
use serde_json::Value;

use crate::PhotocraftApp;
use crate::preview_session::{PreviewSession, hash};

/// Tag of the filter preview keys (never another preview's).
const TAG: u64 = 1 << 57;
/// Bits of a key naming the session, between the settings hash and [`TAG`].
const SESSION_BITS: u32 = 17;
/// The one filter this path previews so far.
pub const COMMAND: &str = "filter.blur.gaussianBlur";

/// One filter dialog's GPU preview session.
pub struct FilterGpuPreview {
    s: PreviewSession,
    dialog: u64,
    target: LayerId,
    /// The selection, shared by every operator of the session (its identity keys the GPU's copy).
    selection: Option<Arc<photocraft_raster::Surface>>,
    /// The target's and the selection's content bounds, measured when the session started.
    measured: Measured,
    /// The settings hash shown and its operator; `None` when the dialog's Preview is off.
    shown: Option<(u64, LayerOperator)>,
    /// The GPU couldn't draw it: the dialog previews on the CPU from now on.
    failed: bool,
    /// Failed only because the GPU canvas couldn't draw the document (a smaller canvas budget, a
    /// compositor lost or without operators): back on the GPU once it can again (Retry GPU).
    unfit: bool,
    /// After a failure, the preview key the canvas texture still shows, held on screen until the
    /// CPU preview arrives (rather than the unfiltered document in between).
    held: Option<u64>,
    /// OK applied the filter as a background job: the preview stays until it lands.
    committing: bool,
    /// The document part the dialog's preview window shows (it joins the damage). Set while the
    /// dialog draws, with the app borrowed shared.
    window: std::cell::Cell<Option<Rect>>,
}

/// The open filter dialog this path could preview: (dialog id, params, Preview on).
fn dialog(app: &PhotocraftApp) -> Option<(u64, Value, bool)> {
    let d = app.ui.dialogs.iter().find(|d| d.fields.contains_key("__filter"))?;
    if d.fields.get("__command").and_then(Value::as_str) != Some(COMMAND) || d.fields.contains_key("__smartFilter") || crate::select_mask_view::owns(&d.fields)
    {
        return None;
    }
    let preview = d.fields.get("__preview").and_then(Value::as_bool) == Some(true);
    Some((d.id, crate::filter_dialog::params_of(&d.fields), preview))
}

/// Why document `idx`'s `target` can't preview on the GPU (the CPU preview then runs), or `None`.
pub fn ineligible(app: &PhotocraftApp, idx: usize, params: &Value) -> Option<&'static str> {
    let Some(gpu) = app.gpu.as_ref() else { return Some("no GPU canvas") };
    let Some(st) = app.session.documents().get(idx) else { return Some("no document") };
    let doc = &st.doc;
    if !matches!(doc.mode, photocraft_color::ColorMode::Rgb | photocraft_color::ColorMode::Grayscale) {
        return Some("not an RGB or Gray document");
    }
    if st.channel_view.target != photocraft_engine::channel_cmds::ChannelTarget::Composite || doc.quick_mask.is_some() {
        return Some("filtering a channel");
    }
    if !app.with_mask_target(COMMAND, Value::Null).is_null() {
        return Some("filtering a layer mask");
    }
    let Some(target) = st.active_layer else { return Some("no active layer") };
    let Some(layer) = doc.layer(target) else { return Some("no active layer") };
    if !matches!(layer.content, LayerContent::Raster(_)) {
        return Some("not a pixel layer");
    }
    if photocraft_compose::effects::has_effects(layer) || under_effects(&doc.layers, target, 0) {
        return Some("the layer or a group around it has effects");
    }
    let locks = doc.effective_locks(target);
    if locks.pixels || locks.all {
        return Some("the layer is locked");
    }
    // Zoomed out the preview runs on a reduced proxy (a later phase); at 100 % every pixel.
    if crate::canvas::filter_preview_factor(app, doc, COMMAND, params) != 1 {
        return Some("zoomed out");
    }
    if !gpu_view(app, idx) {
        return Some("the view is mirrored or pixel-aspect corrected");
    }
    if !gpu.supports(doc) || !gpu.operators_supported() {
        return Some("the GPU compositor can't draw it");
    }
    None
}

/// Whether `target` sits inside a group with layer effects (bounded depth).
fn under_effects(layers: &[photocraft_doc::Layer], target: LayerId, depth: usize) -> bool {
    depth < 256
        && layers.iter().any(|l| match l.children() {
            Some(c) => {
                let holds = |c: &[photocraft_doc::Layer]| doc_holds(c, target, depth + 1);
                (photocraft_compose::effects::has_effects(l) && holds(c)) || under_effects(c, target, depth + 1)
            }
            None => false,
        })
}

fn doc_holds(layers: &[photocraft_doc::Layer], target: LayerId, depth: usize) -> bool {
    depth < 256 && layers.iter().any(|l| l.id == target || l.children().is_some_and(|c| doc_holds(c, target, depth + 1)))
}

/// Whether document `idx`'s view draws on the GPU canvas: a mirrored or pixel-aspect-corrected
/// view draws through the CPU canvas (`gpu_canvas::gpu_view_ok`), which can't show operators.
fn gpu_view(app: &PhotocraftApp, idx: usize) -> bool {
    let rotation = app.ui.views.get(idx).map_or(0.0, |v| v.rotation);
    crate::gpu_canvas::gpu_view_ok(app.ui.view.flip_horizontal, rotation, app.ui.view.display_aspect())
}

/// The content bounds of a session's target layer and selection: they don't change while the
/// dialog is open, so they are measured once rather than on every slider step.
#[derive(Clone, Copy, Debug)]
struct Measured {
    content: Rect,
    selection: Option<Rect>,
}

impl Measured {
    fn of(doc: &Document, target: LayerId) -> Option<Self> {
        let content = doc.layer(target)?.surface()?.content_bounds();
        Some(Measured { content, selection: doc.selection.as_ref().map(photocraft_raster::Surface::content_bounds) })
    }
}

/// The blur for `params` on `target` of `doc`, exactly as the engine applies it
/// (`photocraft_engine::filters::run_filter`): its output area, edge extent, selection and Lock
/// Transparency. Also returns the area it changes.
fn blur_for(
    doc: &Document,
    target: LayerId,
    params: &Value,
    selection: Option<Arc<photocraft_raster::Surface>>,
    measured: Measured,
) -> Option<(GaussianBlur, Rect)> {
    let fp = photocraft_engine::filters::params_for(COMMAND, params)?;
    let Measured { content, selection: sel_bounds } = measured;
    let bounds = sel_bounds.filter(|b| !b.is_empty()).unwrap_or(doc.bounds());
    let area = photocraft_algo::output_area(&fp, content, bounds, sel_bounds);
    let extent = photocraft_algo::separable::edge_extent(doc.bounds(), content);
    let blur = match photocraft_algo::separable::Separable::of(&fp)? {
        photocraft_algo::separable::Separable::Kernel(k) => SeparableBlur::Kernel(k),
        photocraft_algo::separable::Separable::Boxes(b) => SeparableBlur::Boxes(b),
    };
    let keep_alpha = doc.effective_locks(target).transparency;
    let op = GaussianBlur::new(blur, extent, area, selection, keep_alpha, doc.depth)?;
    Some((op, area.intersect(&doc.bounds())))
}

/// Bring the session up to date for document `idx` (the active one) and return it.
fn update(app: &mut PhotocraftApp, idx: usize) -> Option<&FilterGpuPreview> {
    let st = app.session.documents().get(idx)?;
    let (doc_id, revision) = (st.doc.id, st.revision);
    // After OK: kept while the job applying it runs on the same revision.
    // (Only for the session's own document: views of others ask too.)
    if let Some(p) = app.filter_gpu_preview.as_mut()
        && p.committing
        && p.s.doc == doc_id
    {
        if p.s.revision == revision && app.session.job_on(doc_id).is_some() {
            return app.filter_gpu_preview.as_ref();
        }
        p.committing = false;
        p.shown = None;
        clear_operators(app, doc_id);
        return None;
    }
    // Another document's OK: its session stays as it is while the job applies (this document
    // previews on the CPU meanwhile), and is retired here once it has landed.
    if let Some(p) = app.filter_gpu_preview.as_ref().filter(|p| p.committing) {
        let (doc, revision) = (p.s.doc, p.s.revision);
        let applying = app.session.documents().iter().any(|st| st.doc.id == doc && st.revision == revision) && app.session.job_on(doc).is_some();
        if applying {
            return None;
        }
        if let Some(p) = app.filter_gpu_preview.as_mut() {
            p.committing = false;
            p.shown = None;
        }
        clear_operators(app, doc);
    }
    if app.session.active_index() != Some(idx) {
        return None;
    }
    let Some((dialog_id, params, preview)) = dialog(app) else {
        // Keep the closed dialog's session for the canvas's switch back, but show nothing.
        hide(app);
        return None;
    };
    let target = st.active_layer?;
    let session = hash(&[&doc_id.0.to_le_bytes(), &revision.to_le_bytes(), &target.0.to_le_bytes(), &dialog_id.to_le_bytes()]);
    if app.filter_gpu_preview.as_ref().is_none_or(|p| p.s.session != session || p.s.doc != doc_id) {
        if let Some(reason) = ineligible(app, idx, &params) {
            log::debug!("filter preview on the CPU: {reason}");
            if let Some(p) = app.filter_gpu_preview.take() {
                clear_operators(app, p.s.doc);
            }
            return None;
        }
        hide(app);
        let st = app.session.documents().get(idx)?;
        let selection = st.doc.selection.clone().map(Arc::new);
        let measured = Measured::of(&st.doc, target)?;
        let (_, region) = blur_for(&st.doc, target, &params, selection.clone(), measured)?;
        app.filter_gpu_preview = Some(FilterGpuPreview {
            s: PreviewSession::new(TAG, SESSION_BITS, doc_id, revision, session, region),
            dialog: dialog_id,
            target,
            selection,
            measured,
            shown: None,
            failed: false,
            unfit: false,
            held: None,
            committing: false,
            window: std::cell::Cell::new(None),
        });
    }
    // The GPU canvas can stop drawing this document mid-session (a smaller canvas budget, a
    // compositor that failed or was lost): it couldn't show the operator any more, so the CPU
    // preview takes over, as after a failed refresh.
    let drawable = app.session.documents().get(idx).is_some_and(|st| app.gpu.as_ref().is_some_and(|g| g.fits_canvas(&st.doc) && g.operators_supported()));
    // Its operator is cleared, as after a failed refresh, so no canvas draws it again unasked; the
    // GPU canvas shows nothing of this document meanwhile, so there is no texture to hold.
    let (fell, back) = app.filter_gpu_preview.as_ref().filter(|p| !p.committing).map_or((false, false), |p| (!drawable && !p.failed, drawable && p.unfit));
    if fell && let Some(p) = app.filter_gpu_preview.as_mut() {
        log::debug!("filter preview on the CPU: the GPU canvas can't draw it any more");
        p.failed = true;
        p.unfit = true;
        p.held = None;
        hide(app);
    } else if back && let Some(p) = app.filter_gpu_preview.as_mut() {
        log::debug!("filter preview back on the GPU: the GPU canvas can draw it again");
        p.failed = false;
        p.unfit = false;
    }
    let p = app.filter_gpu_preview.as_ref()?;
    if p.failed {
        // Hold the last GPU preview until the CPU's is ready.
        let ready = app.filter_preview.as_ref().is_some_and(|c| c.key.dialog == p.dialog && c.result.is_some());
        if ready && let Some(p) = app.filter_gpu_preview.as_mut() {
            p.held = None;
        }
        return app.filter_gpu_preview.as_ref();
    }
    // Still zoomed in (the view's zoom picks the factor) on a GPU view: else the CPU preview.
    if crate::canvas::filter_preview_factor(app, &app.session.documents().get(idx)?.doc, COMMAND, &params) != 1 || !gpu_view(app, idx) {
        hide(app);
        return None;
    }
    let h = hash(&[params.to_string().as_bytes()]);
    if !preview {
        hide(app);
        return app.filter_gpu_preview.as_ref();
    }
    if p.shown.as_ref().is_none_or(|(sh, _)| *sh != h) {
        let st = app.session.documents().get(idx)?;
        let Some((blur, region)) = blur_for(&st.doc, p.target, &params, p.selection.clone(), p.measured) else {
            // Settings the GPU can't run (a reach past its limit): the CPU preview for them.
            hide(app);
            return None;
        };
        let op = LayerOperator { layer: p.target, program: Arc::new(blur) };
        if let Some(gpu) = app.gpu.as_ref() {
            gpu.set_operators(doc_id, vec![op.clone()]);
        }
        let p = app.filter_gpu_preview.as_mut()?;
        // A larger radius changes more: the switch recomposites the larger area.
        p.s.region = p.s.region.union(&region);
        p.shown = Some((h, op));
    }
    app.filter_gpu_preview.as_ref()
}

/// Register the session's shown operator again: after Retry GPU, the new canvas has none (they
/// lived in the old one's resources), and with unchanged settings `update` wouldn't set them.
pub(crate) fn rebind(app: &mut PhotocraftApp) {
    // A failed session's operators are gone and stay so: one that failed only because the canvas
    // couldn't draw it builds them again in `update` once the new canvas can.
    let Some(p) = app.filter_gpu_preview.as_ref().filter(|p| !p.failed) else { return };
    if let (Some(gpu), Some((_, op))) = (app.gpu.as_ref(), p.shown.as_ref()) {
        gpu.set_operators(p.s.doc, vec![op.clone()]);
    }
}

/// Show nothing for the session (its operators cleared on its own document), keeping it for the
/// canvas's switch back.
fn hide(app: &mut PhotocraftApp) {
    let Some(p) = app.filter_gpu_preview.as_mut() else { return };
    if p.shown.take().is_some() {
        let doc = p.s.doc;
        clear_operators(app, doc);
    }
}

fn clear_operators(app: &PhotocraftApp, doc: DocId) {
    if let Some(gpu) = app.gpu.as_ref() {
        gpu.set_operators(doc, Vec::new());
    }
}

/// The document the canvas shows for document `idx` while the filter previews on the GPU: the
/// committed document under the preview's key (its layer operator does the filtering).
pub fn display_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    let key = {
        let p = update(app, idx)?;
        match (p.failed, p.shown.as_ref()) {
            (true, _) => p.held?,
            (false, Some((h, _))) => p.s.key(*h),
            (false, None) => return None,
        }
    };
    Some((app.session.documents().get(idx)?.doc.clone(), key))
}

/// Whether the open filter dialog previews on the GPU (so the CPU preview must stand aside).
pub fn on_gpu(app: &mut PhotocraftApp, idx: usize) -> bool {
    update(app, idx).is_some_and(|p| !p.failed && (p.shown.is_some() || p.committing))
}

/// The GPU couldn't draw document `doc`'s preview (`gpu_canvas::REFRESH_OPERATOR_FAILED`): preview
/// on the CPU for the rest of the dialog. `texture` is the preview key the canvas texture still
/// shows (the failed refresh drew nothing): held on screen until the CPU preview arrives.
pub fn failed(app: &mut PhotocraftApp, doc: DocId, texture: Option<u64>) {
    if let Some(p) = app.filter_gpu_preview.as_mut().filter(|p| p.s.doc == doc) {
        log::warn!("filter preview falls back to the CPU: the GPU couldn't draw it");
        p.held = texture.filter(|k| p.s.owns(*k));
        p.failed = true;
        p.unfit = false;
        p.shown = None;
        p.committing = false;
    }
    clear_operators(app, doc);
}

/// OK was pressed: `running` when the filter is applying as a background job (the preview stays
/// until it lands), else the session ends.
pub fn confirmed(app: &mut PhotocraftApp, running: bool) {
    let Some(p) = app.filter_gpu_preview.as_mut() else { return };
    if running && p.shown.is_some() && !p.failed {
        p.committing = true;
    } else {
        let doc = p.s.doc;
        p.shown = None;
        clear_operators(app, doc);
    }
}

/// See [`PreviewSession::switch_region`].
pub fn switch_region(app: &PhotocraftApp, doc: DocId, revision: u64, from: u64, to: u64) -> Option<Rect> {
    app.filter_gpu_preview.as_ref()?.s.switch_region(doc, revision, from, to)
}

/// See [`PreviewSession::switch_damage`]: what the view and the dialog's preview window show.
pub fn switch_damage(app: &mut PhotocraftApp, to: u64, region: Rect, visible: Rect) -> Rect {
    match app.filter_gpu_preview.as_mut() {
        Some(p) => {
            let shown = p.window.get().map_or(visible, |w| visible.union(&w));
            p.s.switch_damage(to, region, shown)
        }
        None => region,
    }
}

/// See [`PreviewSession::uncovered`]: the view and the dialog's preview window.
pub fn uncovered(app: &mut PhotocraftApp, doc: DocId, key: u64, visible: Rect) -> Option<Rect> {
    // A held preview can't be drawn any more (its operator is gone): never redraw under its key.
    let p = app.filter_gpu_preview.as_mut().filter(|p| !p.failed)?;
    let shown = p.window.get().map_or(visible, |w| visible.union(&w));
    p.s.uncovered(doc, key, shown)
}

/// The canvas's GPU view of the document the dialog `dialog` previews on the GPU, for its preview
/// window to draw from the same texture, after noting `rect` (the document part the window shows)
/// so the canvas composites the preview there too. `None` while the GPU path isn't showing.
pub fn window_view(app: &PhotocraftApp, dialog: u64, rect: Rect) -> Option<crate::gpu_canvas::ViewParams> {
    let p = app.filter_gpu_preview.as_ref().filter(|p| p.dialog == dialog && p.shown.is_some() && !p.failed)?;
    p.window.set(Some(rect));
    let params = *app.canvas_gpu.get(&p.s.doc)?;
    let st = app.session.documents().iter().find(|st| st.doc.id == p.s.doc)?;
    ([st.doc.size.width, st.doc.size.height] == params.doc_size).then_some(params)
}

/// The preview key the canvas texture of document `doc` shows while it previews (the window draws
/// from that texture only then).
pub fn shown_key(app: &PhotocraftApp, doc: DocId) -> Option<u64> {
    let p = app.filter_gpu_preview.as_ref().filter(|p| p.s.doc == doc && !p.failed)?;
    p.shown.as_ref().map(|(h, _)| p.s.key(*h))
}

/// Drop the session of a closed document.
pub(crate) fn retain_documents(app: &mut PhotocraftApp) {
    if app.filter_gpu_preview.as_ref().is_some_and(|p| !app.session.documents().iter().any(|st| st.doc.id == p.s.doc)) {
        app.filter_gpu_preview = None;
    }
}

#[cfg(test)]
#[path = "filter_gpu_preview_tests.rs"]
mod tests;
