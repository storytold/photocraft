//! Gaussian Blur's live preview on the GPU (#5772): while the dialog is open, the target layer's
//! content is a GPU layer operator (`photocraft_gpu::GaussianBlur`) and the canvas composites the
//! committed document with it, only where the view (and the dialog's preview window) shows, under
//! a session-tagged preview key (`preview_session`). A slider change re-runs the blur on the GPU;
//! nothing is read back or flattened on the CPU.
//!
//! Zoomed out, the preview runs on a reduced proxy of the document (Phase 2): the CPU preview's
//! factor `k`, its document and layer ids flipped (`adjust_preview::proxy_base`), the blur's
//! lengths scaled by `k` as the CPU preview scales them, composited into the proxy's own texture
//! and drawn scaled by `k` ([`gpu_proxy`]).
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
use photocraft_gpu::{GaussianBlur, LayerOperator, OperatorTarget, SeparableBlur};
use serde_json::Value;

use crate::PhotocraftApp;
use crate::preview_session::{PreviewSession, hash};

/// Tag of the filter preview keys (never another preview's).
const TAG: u64 = 1 << 57;
/// Bits of a key naming the session, between the settings hash and [`TAG`].
const SESSION_BITS: u32 = 17;
/// The one filter this path previews so far.
pub const COMMAND: &str = "filter.blur.gaussianBlur";

/// Unsharp Mask: the same blur, combined with the source (`photocraft_gpu::Combine::Unsharp`).
pub const UNSHARP: &str = "filter.sharpen.unsharpMask";

/// High Pass: the same blur, as `0.5 + source - blurred` (`photocraft_gpu::Combine::HighPass`).
pub const HIGH_PASS: &str = "filter.other.highPass";

/// Box Blur: a uniform kernel or one box pass.
pub const BOX_BLUR: &str = "filter.blur.boxBlur";

/// The filters this path previews.
pub const COMMANDS: [&str; 4] = [COMMAND, UNSHARP, HIGH_PASS, BOX_BLUR];

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
    /// After a failure zoomed out: the proxy's texture (its last drawn preview) is held on screen
    /// the same way.
    held_proxy: bool,
    /// OK applied the filter as a background job: the preview stays until it lands.
    committing: bool,
    /// The document part the dialog's preview window shows (it joins the damage). Set while the
    /// dialog draws, with the app borrowed shared.
    window: std::cell::Cell<Option<Rect>>,
    /// Zoomed out: the reduced proxy the preview runs on instead.
    proxy: Option<FilterProxy>,
    /// The Navigator's picture of the preview ([`navigator_doc`]).
    nav: Option<NavigatorProxy>,
}

/// The Navigator's picture while the filter previews on the GPU: the CPU preview's own filter
/// run on a proxy of the Navigator's size (its operator lives on the main canvas, whose texture
/// is only drawn where the view is).
struct NavigatorProxy {
    k: u32,
    base: Arc<Document>,
    /// The settings hash shown and the filtered proxy.
    shown: Option<(u64, Arc<Document>)>,
}

/// A zoomed-out session's proxy of the document: reduced by `k`, with its own ids.
struct FilterProxy {
    k: u32,
    base: Arc<Document>,
    target: LayerId,
    selection: Option<Arc<photocraft_raster::Surface>>,
    measured: Measured,
    /// The settings hash shown and its operator (on `base`).
    shown: Option<(u64, LayerOperator)>,
    /// The settings hash last composited into the proxy's texture.
    drawn: Option<u64>,
    /// Where the proxy's previews change it (proxy pixels).
    region: Rect,
}

/// The open filter dialog this path could preview: (dialog id, its command, params, Preview on).
fn dialog(app: &PhotocraftApp) -> Option<(u64, &'static str, Value, bool)> {
    let d = app.ui.dialogs.iter().find(|d| d.fields.contains_key("__filter"))?;
    let command = d.fields.get("__command").and_then(Value::as_str).and_then(|c| COMMANDS.into_iter().find(|k| *k == c))?;
    if d.fields.contains_key("__smartFilter") || crate::select_mask_view::owns(&d.fields) {
        return None;
    }
    let preview = d.fields.get("__preview").and_then(Value::as_bool) == Some(true);
    Some((d.id, command, crate::filter_dialog::params_of(&d.fields), preview))
}

/// The command the open dialog previews, or Gaussian Blur's.
fn command(app: &PhotocraftApp) -> &'static str {
    dialog(app).map_or(COMMAND, |(_, c, ..)| c)
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
    let Some(target) = st.active_layer else { return Some("no active layer") };
    let Some(layer) = doc.layer(target) else { return Some("no active layer") };
    if targets(app) == OperatorTarget::Mask {
        // The operator replaces the pixel mask the compositor samples: an enabled one, not
        // combined with vector masks first, and not shown on its own (the ⌥-click view).
        if st.channel_view.layer_mask.is_some() {
            return Some("the layer mask is viewed on its own");
        }
        if !layer.mask.as_ref().is_some_and(|m| m.enabled) || photocraft_compose::masks::combined_mask(layer, doc.bounds()).is_some() {
            return Some("the layer mask is disabled, feathered or combined with a vector mask");
        }
    } else {
        if !matches!(layer.content, LayerContent::Raster(_)) {
            return Some("not a pixel layer");
        }
        // The engine filters a targeted mask whatever the layer's locks.
        let locks = doc.effective_locks(target);
        if locks.pixels || locks.all {
            return Some("the layer is locked");
        }
    }
    if photocraft_compose::effects::has_effects(layer) || under_effects(&doc.layers, target, 0) {
        return Some("the layer or a group around it has effects");
    }
    // Zoomed out the preview runs on a reduced proxy (a later phase); at 100 % every pixel.
    if crate::canvas::filter_preview_factor(app, doc, command(app), params) > 1 && !photocraft_compose::proxy::proxy_faithful(doc) {
        return Some("zoomed out, and a proxy can't show this document");
    }
    if !gpu_view(app, idx) {
        return Some("the view is mirrored or pixel-aspect corrected");
    }
    // A canvas past the texture budget draws on the CPU, which can't show operators.
    if !gpu.fits_canvas(doc) || !gpu.supports(doc) || !gpu.operators_supported() {
        return Some("the GPU compositor can't draw it");
    }
    None
}

/// What the filter edits: the active layer's mask when the Layers panel (or an adjustment layer)
/// targets it, as `PhotocraftApp::with_mask_target` routes the command; else its pixels.
fn targets(app: &PhotocraftApp) -> OperatorTarget {
    if app.with_mask_target(command(app), Value::Null).is_null() { OperatorTarget::Content } else { OperatorTarget::Mask }
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
    /// The filter (one of [`COMMANDS`]).
    command: &'static str,
    /// What the filter edits (the layer's pixels or its mask).
    on: OperatorTarget,
    /// The filter's content bounds: the layer's pixels, or for a mask the canvas and the mask's
    /// pixels (`photocraft_engine::filters::run_filter`).
    content: Rect,
    selection: Option<Rect>,
    /// The depth the result rounds to: the document's, or the mask's own.
    depth: photocraft_color::SampleType,
}

impl Measured {
    fn of(doc: &Document, target: LayerId, command: &'static str, on: OperatorTarget) -> Option<Self> {
        let layer = doc.layer(target)?;
        let (content, depth) = match on {
            OperatorTarget::Content => (layer.surface()?.content_bounds(), doc.depth),
            OperatorTarget::Mask => {
                let m = &layer.mask.as_ref()?.surface;
                (doc.bounds().union(&m.content_bounds()), m.format().sample)
            }
        };
        Some(Measured { command, on, content, selection: doc.selection.as_ref().map(photocraft_raster::Surface::content_bounds), depth })
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
    let Measured { command, on, content, selection: sel_bounds, depth } = measured;
    let fp = photocraft_engine::filters::params_for(command, params)?;
    let bounds = sel_bounds.filter(|b| !b.is_empty()).unwrap_or(doc.bounds());
    let area = photocraft_algo::output_area(&fp, content, bounds, sel_bounds);
    let extent = photocraft_algo::separable::edge_extent(doc.bounds(), content);
    let blur = match photocraft_algo::separable::Separable::of(&fp)? {
        photocraft_algo::separable::Separable::Kernel(k) => SeparableBlur::Kernel(k),
        photocraft_algo::separable::Separable::Boxes(b) => SeparableBlur::Boxes(b),
    };
    // Lock Transparency keeps a layer's alpha; a mask has none.
    let keep_alpha = on == OperatorTarget::Content && doc.effective_locks(target).transparency;
    let op = GaussianBlur::new(blur, extent, area, selection, keep_alpha, depth)?;
    // Unsharp Mask combines the blur with the source.
    let op = match fp {
        photocraft_algo::FilterParams::UnsharpMask { amount, threshold, .. } => op.unsharp(amount, threshold)?,
        photocraft_algo::FilterParams::HighPass { .. } => op.high_pass(),
        _ => op,
    };
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
        hide_proxy(app);
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
        hide_proxy(app);
    }
    if app.session.active_index() != Some(idx) {
        return None;
    }
    let Some((dialog_id, command, params, preview)) = dialog(app) else {
        // Keep the closed dialog's session for the canvas's switch back, but show nothing.
        hide(app);
        return None;
    };
    let target = app.session.documents().get(idx)?.active_layer?;
    let on = [u8::from(targets(app) == OperatorTarget::Mask)];
    let session = hash(&[&doc_id.0.to_le_bytes(), &revision.to_le_bytes(), &target.0.to_le_bytes(), &dialog_id.to_le_bytes(), &on, command.as_bytes()]);
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
        let measured = Measured::of(&st.doc, target, command, targets(app))?;
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
            held_proxy: false,
            committing: false,
            window: std::cell::Cell::new(None),
            proxy: None,
            nav: None,
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
            p.held_proxy = false;
        }
        return app.filter_gpu_preview.as_ref();
    }
    // On a GPU view only: else the CPU preview.
    if !gpu_view(app, idx) {
        hide(app);
        return None;
    }
    let h = hash(&[params.to_string().as_bytes()]);
    if !preview {
        hide(app);
        return app.filter_gpu_preview.as_ref();
    }
    // Zoomed out (the view's zoom picks the factor): the reduced proxy previews.
    let k = crate::canvas::filter_preview_factor(app, &app.session.documents().get(idx)?.doc, command, &params);
    if k > 1 {
        hide_main(app);
        // Checked here too: the session may have started at 100 %, where it didn't matter.
        if !photocraft_compose::proxy::proxy_faithful(&app.session.documents().get(idx)?.doc) {
            hide_proxy(app);
            return None;
        }
        return update_proxy(app, idx, k, &params, h);
    }
    hide_proxy(app);
    let p = app.filter_gpu_preview.as_ref()?;
    if p.shown.as_ref().is_none_or(|(sh, _)| *sh != h) {
        let st = app.session.documents().get(idx)?;
        let Some((blur, region)) = blur_for(&st.doc, p.target, &params, p.selection.clone(), p.measured) else {
            // Settings the GPU can't run (a reach past its limit): the CPU preview for them.
            hide(app);
            return None;
        };
        let op = LayerOperator { layer: p.target, target: p.measured.on, program: Arc::new(blur) };
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

/// Register the session's shown operators again: after Retry GPU, the new canvas has none (they
/// lived in the old one's resources), and with unchanged settings `update` wouldn't set them. The
/// zoomed-out proxy's texture is drawn again too.
pub(crate) fn rebind(app: &mut PhotocraftApp) {
    // A failed session's operators are gone and stay so: one that failed only because the canvas
    // couldn't draw it builds them again in `update` once the new canvas can.
    let Some(gpu) = app.gpu.clone() else { return };
    let Some(p) = app.filter_gpu_preview.as_mut().filter(|p| !p.failed) else { return };
    if let Some((_, op)) = p.shown.as_ref() {
        gpu.set_operators(p.s.doc, vec![op.clone()]);
    }
    if let Some(g) = p.proxy.as_mut() {
        if let Some((_, op)) = g.shown.as_ref() {
            gpu.set_operators(g.base.id, vec![op.clone()]);
        }
        g.drawn = None;
    }
}

/// Show nothing for the session (its operators cleared), keeping it for the canvas's switch back.
fn hide(app: &mut PhotocraftApp) {
    hide_main(app);
    hide_proxy(app);
}

/// [`hide`] the full-size preview only (its operators cleared on the session's own document).
fn hide_main(app: &mut PhotocraftApp) {
    let Some(p) = app.filter_gpu_preview.as_mut() else { return };
    if p.shown.take().is_some() {
        let doc = p.s.doc;
        clear_operators(app, doc);
    }
}

/// [`hide`] the zoomed-out proxy's preview only (its operators cleared on the proxy).
fn hide_proxy(app: &mut PhotocraftApp) {
    let Some(g) = app.filter_gpu_preview.as_mut().and_then(|p| p.proxy.as_mut()) else { return };
    if g.shown.take().is_some() {
        let doc = g.base.id;
        clear_operators(app, doc);
    }
}

/// The zoomed-out preview at factor `k` for settings `params` (hash `h`): the session's proxy,
/// made once per factor, with the blur's lengths scaled by `k` on its copy of the target, exactly
/// as the CPU preview runs it on its proxy.
fn update_proxy<'a>(app: &'a mut PhotocraftApp, idx: usize, k: u32, params: &Value, h: u64) -> Option<&'a FilterGpuPreview> {
    let p = app.filter_gpu_preview.as_ref()?;
    let (session_target, stale) = (p.target, p.proxy.as_ref().is_none_or(|g| g.k != k));
    if stale {
        hide_proxy(app);
        let st = app.session.documents().get(idx)?;
        let gpu = app.gpu.as_ref()?;
        let base = crate::adjust_preview::proxy_base(&st.doc, k);
        let target = crate::adjust_preview::proxy_layer(session_target);
        if !gpu.supports(&base) {
            return None;
        }
        let m = app.filter_gpu_preview.as_ref()?.measured;
        let measured = Measured::of(&base, target, m.command, m.on)?;
        let selection = base.selection.clone().map(Arc::new);
        let region = Rect::EMPTY;
        let g = FilterProxy { k, base: Arc::new(base), target, selection, measured, shown: None, drawn: None, region };
        app.filter_gpu_preview.as_mut()?.proxy = Some(g);
    }
    let g = app.filter_gpu_preview.as_ref()?.proxy.as_ref()?;
    if g.shown.as_ref().is_none_or(|(sh, _)| *sh != h) {
        let mut scaled = params.clone();
        crate::filter_dialog::scale_preview_params(g.measured.command, &mut scaled, k);
        let Some((blur, region)) = blur_for(&g.base, g.target, &scaled, g.selection.clone(), g.measured) else {
            // A reach past the GPU's limit: the CPU preview for these settings.
            hide_proxy(app);
            return None;
        };
        let op = LayerOperator { layer: g.target, target: g.measured.on, program: Arc::new(blur) };
        if let Some(gpu) = app.gpu.as_ref() {
            gpu.set_operators(g.base.id, vec![op.clone()]);
        }
        let g = app.filter_gpu_preview.as_mut()?.proxy.as_mut()?;
        g.region = g.region.union(&region);
        g.shown = Some((h, op));
    }
    app.filter_gpu_preview.as_ref()
}

/// The zoomed-out preview of document `idx` for the canvas: its proxy composited into the proxy's
/// own texture (the changed area on a settings change), as (factor, texture key, size), drawn
/// scaled by the factor. `None` at 100 % (see [`display_doc`]) or when the GPU doesn't preview.
pub fn gpu_proxy(app: &mut PhotocraftApp, idx: usize) -> Option<(u32, u64, [u32; 2])> {
    let p = update(app, idx)?;
    if p.failed {
        return None;
    }
    let g = p.proxy.as_ref()?;
    let (h, _) = g.shown.as_ref()?;
    let (k, base, h, damage, stale) = (g.k, g.base.clone(), *h, g.drawn.map(|_| g.region), g.drawn != Some(*h));
    let (key, size) = (base.id.0, [base.size.width, base.size.height]);
    let gpu = app.gpu.clone()?;
    if stale || !gpu.has(key, size) {
        let doc = app.session.documents().get(idx)?.doc.clone();
        let (display, _) = crate::canvas::canvas_display(app, &doc, None);
        let r = gpu.refresh(key, &base, damage.filter(|_| gpu.has(key, size)), display.as_deref());
        if r.kind == crate::gpu_canvas::REFRESH_OPERATOR_FAILED {
            failed(app, doc.id, None);
            return None;
        }
        app.perf.record(if r.kind.starts_with("gpu") { "gpu-filter-proxy" } else { "filter-proxy" }, r.px, r.composite_ms, r.upload_ms);
        if let Some(t) = r.gpu {
            app.perf.record_gpu(t);
        }
        if let Some(g) = app.filter_gpu_preview.as_mut().and_then(|p| p.proxy.as_mut()) {
            g.drawn = Some(h);
        }
    }
    Some((k, key, size))
}

/// After the GPU failed zoomed out, the proxy's texture with its last drawn preview, as
/// [`gpu_proxy`] gives it: the canvas draws it until the CPU preview is ready (it comes after the
/// CPU preview in the canvas's chain, so that one is still requested), never the document
/// unfiltered in between.
pub fn held_proxy(app: &mut PhotocraftApp, idx: usize) -> Option<(u32, u64, [u32; 2])> {
    let p = update(app, idx)?;
    let g = p.proxy.as_ref().filter(|g| p.failed && p.held_proxy && g.drawn.is_some())?;
    Some((g.k, g.base.id.0, [g.base.size.width, g.base.size.height]))
}

/// The proxy's texture key, to keep it alive while the session previews zoomed out.
pub fn gpu_keys(app: &PhotocraftApp) -> Option<u64> {
    let p = app.filter_gpu_preview.as_ref()?;
    let g = p.proxy.as_ref()?;
    (g.shown.is_some() || (p.failed && p.held_proxy)).then_some(g.base.id.0)
}

/// What the Navigator shows of document `idx` while the filter previews on the GPU, and its key:
/// the filter run on a proxy of the Navigator's size, exactly as the CPU preview runs it on a
/// zoomed-out proxy (made once per session). `None` when the GPU isn't previewing it.
pub fn navigator_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    let st = app.session.documents().get(idx)?;
    let p = app.filter_gpu_preview.as_ref()?;
    if p.s.doc != st.doc.id {
        return None;
    }
    if p.failed {
        // While the canvas holds the GPU's last preview, so does the Navigator; then the CPU's.
        if p.held.is_none() && !p.held_proxy {
            return None;
        }
        let (h, doc) = p.nav.as_ref()?.shown.as_ref()?;
        return Some((doc.clone(), h ^ NAVIGATOR_KEY));
    }
    let showing = p.shown.is_some() || p.proxy.as_ref().is_some_and(|g| g.shown.is_some());
    if !showing {
        return None;
    }
    let (_, command, params, _) = dialog(app)?;
    let params = app.with_mask_target(command, params);
    let h = hash(&[command.as_bytes(), params.to_string().as_bytes()]);
    let longest = st.doc.size.width.max(st.doc.size.height);
    // The largest factor that keeps the proxy at least the Navigator's size.
    let k = (longest / crate::canvas::NAVIGATOR_SIDE).max(1);
    let target = p.target;
    if p.nav.as_ref().is_none_or(|n| n.k != k) {
        let base = Arc::new(crate::adjust_preview::proxy_base(&st.doc, k));
        app.filter_gpu_preview.as_mut()?.nav = Some(NavigatorProxy { k, base, shown: None });
    }
    let n = app.filter_gpu_preview.as_ref()?.nav.as_ref()?;
    if let Some((sh, doc)) = &n.shown
        && *sh == h
    {
        return Some((doc.clone(), h ^ NAVIGATOR_KEY));
    }
    let mut scaled = params;
    crate::filter_dialog::scale_preview_params(command, &mut scaled, k);
    let doc = crate::filter_dialog::preview_document(&n.base, Some(crate::adjust_preview::proxy_layer(target)), command, &scaled, 1)?;
    let doc = Arc::new(doc);
    app.filter_gpu_preview.as_mut()?.nav.as_mut()?.shown = Some((h, doc.clone()));
    Some((doc, h ^ NAVIGATOR_KEY))
}

/// Marks the Navigator's keys apart from the canvas's preview keys.
const NAVIGATOR_KEY: u64 = 0x6e61_7669_6761_746f;

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
    update(app, idx).is_some_and(|p| !p.failed && (p.shown.is_some() || p.committing || p.proxy.as_ref().is_some_and(|g| g.shown.is_some())))
}

/// The GPU couldn't draw document `doc`'s preview (`gpu_canvas::REFRESH_OPERATOR_FAILED`): preview
/// on the CPU for the rest of the dialog. `texture` is the preview key the canvas texture still
/// shows (the failed refresh drew nothing): held on screen until the CPU preview arrives.
pub fn failed(app: &mut PhotocraftApp, doc: DocId, texture: Option<u64>) {
    if let Some(p) = app.filter_gpu_preview.as_mut().filter(|p| p.s.doc == doc) {
        log::warn!("filter preview falls back to the CPU: the GPU couldn't draw it");
        p.held = texture.filter(|k| p.s.owns(*k));
        // Zoomed out, the proxy's texture still holds its last drawn preview.
        p.held_proxy = p.proxy.as_ref().is_some_and(|g| g.shown.is_some() && g.drawn.is_some());
        p.failed = true;
        p.unfit = false;
        p.shown = None;
        p.committing = false;
    }
    clear_operators(app, doc);
    hide_proxy(app);
}

/// OK was pressed: `running` when the filter is applying as a background job (the preview stays
/// until it lands), else the session ends.
pub fn confirmed(app: &mut PhotocraftApp, running: bool) {
    let Some(p) = app.filter_gpu_preview.as_mut() else { return };
    let showing = p.shown.is_some() || p.proxy.as_ref().is_some_and(|g| g.shown.is_some());
    if running && showing && !p.failed {
        p.committing = true;
    } else {
        hide(app);
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
pub fn window_view(app: &PhotocraftApp, dialog: u64, rect: Rect) -> Option<(crate::gpu_canvas::ViewParams, u32)> {
    let p = app.filter_gpu_preview.as_ref().filter(|p| p.dialog == dialog && !p.failed)?;
    // Zoomed out: the proxy's texture, which holds its whole preview, at its factor.
    if let Some(g) = p.proxy.as_ref().filter(|g| g.shown.is_some() && g.drawn == g.shown.as_ref().map(|(h, _)| *h)) {
        let params = *app.canvas_gpu.get(&p.s.doc)?;
        return (params.doc == g.base.id.0).then_some((params, g.k));
    }
    p.shown.as_ref()?;
    p.window.set(Some(rect));
    let params = *app.canvas_gpu.get(&p.s.doc)?;
    let st = app.session.documents().iter().find(|st| st.doc.id == p.s.doc)?;
    ([st.doc.size.width, st.doc.size.height] == params.doc_size && params.doc == p.s.doc.0).then_some((params, 1))
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
