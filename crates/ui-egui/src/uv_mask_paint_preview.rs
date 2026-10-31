//! Transient opacity and spatial Fill previews for mask Paint layers. Fill display samples a
//! bounded low-resolution proxy off-thread; the full-quality document changes only on commit.
use std::{
    sync::Arc,
    sync::atomic::{AtomicU64, Ordering},
    sync::{Condvar, Mutex},
    time::{Duration, Instant},
};

use photocraft_doc::{DocId, Document, LayerId, mask_paint::Stack};
use serde_json::json;

use crate::PhotocraftApp;

const KEY_BASE: u64 = 1 << 42;
const FILL_PROXY_MAX_SIDE: u32 = 256;
const FILL_SAMPLE_INTERVAL: Duration = Duration::from_millis(50);
const FILL_WORKER_POLL_INTERVAL: Duration = Duration::from_millis(16);
static NEXT_PREVIEW_GENERATION: AtomicU64 = AtomicU64::new(1);

struct FillMaskJob {
    generation: u64,
    doc: DocId,
    revision: u64,
    layer: LayerId,
    index: usize,
    key: u64,
    token: u64,
    canvas: photocraft_geom::Rect,
    size: [u32; 2],
    stack: Stack,
}

#[derive(Clone)]
pub(crate) struct FillMaskProxy {
    generation: u64,
    pub(crate) doc: DocId,
    pub(crate) revision: u64,
    pub(crate) layer: LayerId,
    pub(crate) index: usize,
    pub(crate) key: u64,
    pub(crate) size: [u32; 2],
    pub(crate) values: Arc<Vec<f32>>,
}

#[derive(Default)]
struct FillMaskWorkerState {
    job: Option<FillMaskJob>,
    result: Option<FillMaskProxy>,
    running: bool,
    stopping: bool,
}

struct FillMaskWorker {
    state: Arc<(Mutex<FillMaskWorkerState>, Condvar)>,
    cancel_epoch: Arc<AtomicU64>,
}

impl FillMaskWorker {
    fn new() -> Option<Self> {
        let state = Arc::new((Mutex::new(FillMaskWorkerState::default()), Condvar::new()));
        let thread_state = state.clone();
        let cancel_epoch = Arc::new(AtomicU64::new(0));
        let thread_cancel_epoch = cancel_epoch.clone();
        std::thread::Builder::new()
            .name("fill-mask-sample".into())
            .spawn(move || {
                loop {
                    let job = {
                        let (lock, wake) = &*thread_state;
                        let mut state = lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                        while state.job.is_none() && !state.stopping {
                            state = wake.wait(state).unwrap_or_else(std::sync::PoisonError::into_inner);
                        }
                        if state.stopping {
                            return;
                        }
                        let Some(job) = state.job.take() else { continue };
                        state.running = true;
                        job
                    };
                    let result = sample_fill_mask(job, &thread_cancel_epoch);
                    let (lock, _) = &*thread_state;
                    let mut state = lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    if !state.stopping {
                        if let Some(result) = result {
                            state.result = Some(result);
                        }
                        state.running = false;
                    }
                }
            })
            .ok()?;
        Some(Self { state, cancel_epoch })
    }

    fn submit(&self, mut job: FillMaskJob) {
        let (lock, wake) = &*self.state;
        let mut state = lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !state.stopping {
            // Keep one running sample and replace any queued sample with the latest pointer state.
            job.token = self.cancel_epoch.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
            state.job = Some(job);
            wake.notify_one();
        }
    }

    fn take_result(&self) -> Option<FillMaskProxy> {
        self.state.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).result.take()
    }

    fn is_busy(&self) -> bool {
        let state = self.state.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        state.running || state.job.is_some()
    }
}

impl Drop for FillMaskWorker {
    fn drop(&mut self) {
        self.cancel_epoch.fetch_add(1, Ordering::Relaxed);
        let (lock, wake) = &*self.state;
        let mut state = lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        state.stopping = true;
        state.job = None;
        wake.notify_one();
    }
}

#[derive(Default)]
struct FillSampleSchedule {
    last_sampled_at: Option<Instant>,
    latest_key: Option<u64>,
}

impl FillSampleSchedule {
    fn accept(&mut self, now: Instant, key: u64) -> bool {
        let due = self.last_sampled_at.is_none_or(|at| now.saturating_duration_since(at) >= FILL_SAMPLE_INTERVAL);
        if due && self.latest_key != Some(key) {
            self.last_sampled_at = Some(now);
            self.latest_key = Some(key);
            true
        } else {
            false
        }
    }

    fn until_next(&self, now: Instant) -> Option<Duration> {
        let at = self.last_sampled_at?;
        Some(FILL_SAMPLE_INTERVAL.saturating_sub(now.saturating_duration_since(at)))
    }
}

#[derive(Default)]
struct FillMaskSampler {
    worker: Option<FillMaskWorker>,
    schedule: FillSampleSchedule,
    ready: Option<FillMaskProxy>,
}

pub(crate) struct Preview {
    generation: u64,
    doc: DocId,
    revision: u64,
    layer: LayerId,
    index: usize,
    opacity: f32,
    bounds: Option<[i32; 4]>,
    rotation: Option<f32>,
    shown: Option<(u64, Arc<Document>)>,
    mask_sampler: FillMaskSampler,
    materialized: Option<(u64, Arc<Document>)>,
}

#[derive(Clone, Copy)]
pub(crate) struct Commit {
    pub doc: DocId,
    pub revision: u64,
    pub layer: LayerId,
    pub index: usize,
    pub opacity: f32,
    pub bounds: Option<[i32; 4]>,
    pub rotation: Option<f32>,
}
impl Commit {
    pub(crate) fn params(self) -> serde_json::Value {
        let mut params = json!({"layer":self.layer.0,"index":self.index});
        if let Some(opacity) = self.opacity.is_finite().then_some(self.opacity) {
            params["opacity"] = json!(opacity);
        }
        if let Some(bounds) = self.bounds {
            params["bounds"] = json!(bounds);
        }
        if let Some(rotation) = self.rotation {
            params["rotation"] = json!(rotation);
        }
        params
    }
}

pub(crate) fn set(app: &mut PhotocraftApp, layer: LayerId, index: usize, opacity: f32) {
    if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
        return;
    }

    let Some((doc, revision, _)) = active_opacity(app, layer, index) else { return };
    let matches_active = app.uv_mask_paint_preview.as_ref().is_some_and(|p| (p.doc, p.revision, p.layer, p.index) == (doc, revision, layer, index));
    if app.uv_mask_paint_preview.is_some() && !matches_active {
        let Some(previous) = pending(app) else { return };
        if let Err(error) = app.run("uv.maskPaint.modify", previous.params()) {
            app.ui.status = error;
            app.ui.status_error = true;
            return;
        }
    }

    // Committing another preview can advance this document's revision; re-read both revision
    // and canonical value before assigning the next transient value.
    let Some((doc, revision, canonical)) = active_opacity(app, layer, index) else { return };
    if let Some(p) = app.uv_mask_paint_preview.as_mut()
        && (p.doc, p.revision, p.layer, p.index) == (doc, revision, layer, index)
    {
        if opacity.to_bits() == canonical.to_bits() {
            app.uv_mask_paint_preview = None;
        } else if p.opacity.to_bits() != opacity.to_bits() {
            p.opacity = opacity;
            p.bounds = None;
            p.rotation = None;
            p.shown = None;
            p.mask_sampler = FillMaskSampler::default();
            p.materialized = None;
        }
    } else if app.uv_mask_paint_preview.is_none() && opacity.to_bits() != canonical.to_bits() {
        app.uv_mask_paint_preview = Some(Preview {
            generation: NEXT_PREVIEW_GENERATION.fetch_add(1, Ordering::Relaxed),
            doc,
            revision,
            layer,
            index,
            opacity,
            bounds: None,
            rotation: None,
            shown: None,
            mask_sampler: FillMaskSampler::default(),
            materialized: None,
        });
    }
}

pub(crate) fn set_fill(app: &mut PhotocraftApp, layer: LayerId, index: usize, bounds: [i32; 4], rotation: f32) {
    if !rotation.is_finite() || bounds[0] >= bounds[2] || bounds[1] >= bounds[3] {
        return;
    }
    let Some((doc, revision)) = active_fill(app, layer, index) else { return };
    if app.uv_mask_paint_preview.as_ref().is_some_and(|p| (p.doc, p.revision, p.layer, p.index) != (doc, revision, layer, index)) {
        let Some(previous) = pending(app) else { return };
        if let Err(error) = app.run("uv.maskPaint.modify", previous.params()) {
            app.ui.status = error;
            app.ui.status_error = true;
            return;
        }
    }
    let Some((doc, revision)) = active_fill(app, layer, index) else { return };
    let current =
        app.session.active().and_then(|st| st.doc.layer(layer)).and_then(|l| l.mask.as_ref()).and_then(|m| m.paint.as_ref()).and_then(|s| s.layers.get(index));
    let Some(effect) = current else { return };
    let canonical_opacity = effect.opacity;
    let canonical_bounds = effect.fill_bounds.unwrap_or_else(|| {
        app.session.active().filter(|st| st.doc.id == doc).map_or([0, 0, 0, 0], |st| [0, 0, st.doc.size.width as i32, st.doc.size.height as i32])
    });
    let canonical_rotation = match &effect.kind {
        photocraft_doc::mask_paint::EffectKind::Fill { rotation, .. } => *rotation,
        _ => return,
    };
    if bounds == canonical_bounds && rotation.to_bits() == canonical_rotation.to_bits() {
        app.uv_mask_paint_preview = None;
    } else if let Some(p) = app.uv_mask_paint_preview.as_mut()
        && (p.doc, p.revision, p.layer, p.index) == (doc, revision, layer, index)
    {
        if p.bounds == Some(bounds) && p.rotation.is_some_and(|current| current.to_bits() == rotation.to_bits()) {
            return;
        }
        if p.bounds.is_none() || p.rotation.is_none() {
            p.mask_sampler = FillMaskSampler::default();
        }
        if !p.opacity.is_finite() {
            p.opacity = canonical_opacity;
        }
        p.bounds = Some(bounds);
        p.rotation = Some(rotation);
        p.materialized = None;
    } else if app.uv_mask_paint_preview.is_none() {
        app.uv_mask_paint_preview = Some(Preview {
            generation: NEXT_PREVIEW_GENERATION.fetch_add(1, Ordering::Relaxed),
            doc,
            revision,
            layer,
            index,
            opacity: canonical_opacity,
            bounds: Some(bounds),
            rotation: Some(rotation),
            shown: None,
            mask_sampler: FillMaskSampler::default(),
            materialized: None,
        });
    }
}

fn active_fill(app: &PhotocraftApp, layer: LayerId, index: usize) -> Option<(DocId, u64)> {
    let st = app.session.active()?;
    if st.doc.effective_locks(layer).all {
        return None;
    }
    let effect = st.doc.layer(layer)?.mask.as_ref()?.paint.as_ref()?.layers.get(index)?;
    matches!(&effect.kind, photocraft_doc::mask_paint::EffectKind::Fill { .. }).then_some((st.doc.id, st.revision))
}

fn active_opacity(app: &PhotocraftApp, layer: LayerId, index: usize) -> Option<(DocId, u64, f32)> {
    let st = app.session.active()?;
    let target = st.doc.layer(layer)?;
    if st.doc.effective_locks(layer).all {
        return None;
    }
    let opacity = target.mask.as_ref()?.paint.as_ref()?.layers.get(index)?.opacity;
    Some((st.doc.id, st.revision, opacity))
}

pub(crate) fn value(app: &PhotocraftApp, layer: LayerId, index: usize, fallback: f32) -> f32 {
    app.uv_mask_paint_preview
        .as_ref()
        .filter(|p| {
            p.opacity.is_finite()
                && p.layer == layer
                && p.index == index
                && app.session.active().is_some_and(|st| st.doc.id == p.doc && st.revision == p.revision)
        })
        .map_or(fallback, |p| p.opacity)
}

pub(crate) fn fill_transform(app: &PhotocraftApp, layer: LayerId, index: usize, bounds: [i32; 4], rotation: f32) -> ([i32; 4], f32) {
    app.uv_mask_paint_preview
        .as_ref()
        .filter(|p| {
            p.bounds.is_some() && p.layer == layer && p.index == index && app.session.active().is_some_and(|st| st.doc.id == p.doc && st.revision == p.revision)
        })
        .map_or((bounds, rotation), |p| (p.bounds.unwrap_or(bounds), p.rotation.unwrap_or(rotation)))
}

pub(crate) fn fill_preview_pending(app: &PhotocraftApp, layer: LayerId, index: usize) -> bool {
    let Some(state) = app.session.active() else { return false };
    let Some(preview) =
        app.uv_mask_paint_preview.as_ref().filter(|p| p.doc == state.doc.id && p.revision == state.revision && p.layer == layer && p.index == index)
    else {
        return false;
    };
    if preview.bounds.is_none() {
        return false;
    }
    let explicit_gray =
        state.channel_view.layer_mask.is_some_and(|view| view.layer == layer && view.mode == photocraft_engine::mask_view_cmds::MaskViewMode::Gray);
    let targeted_gray = state.channel_view.target == photocraft_engine::channel_cmds::ChannelTarget::LayerMask && state.active_layer == Some(layer);
    if explicit_gray || targeted_gray {
        return false;
    }
    let key = preview_key(preview.revision, preview.layer, preview.index, preview.opacity, preview.bounds, preview.rotation);
    preview.mask_sampler.ready.as_ref().is_none_or(|shown| shown.key != key)
}

pub(crate) fn current_fill_key(app: &PhotocraftApp, idx: usize) -> Option<u64> {
    let state = app.session.documents().get(idx)?;
    let p =
        app.uv_mask_paint_preview.as_ref().filter(|p| p.doc == state.doc.id && p.revision == state.revision && p.bounds.is_some() && p.rotation.is_some())?;
    Some(preview_key(p.revision, p.layer, p.index, p.opacity, p.bounds, p.rotation))
}

/// The last low-resolution Fill result accepted for this preview gesture. Its key describes
/// the parameters actually sampled, which can intentionally lag the current pointer state.
pub(crate) fn sampled_fill_proxy(app: &PhotocraftApp, idx: usize) -> Option<FillMaskProxy> {
    if app.session.active_index() != Some(idx) {
        return None;
    }
    let state = app.session.documents().get(idx)?;
    let preview =
        app.uv_mask_paint_preview.as_ref().filter(|p| p.doc == state.doc.id && p.revision == state.revision && p.bounds.is_some() && p.rotation.is_some())?;
    let proxy = preview.mask_sampler.ready.as_ref()?;
    (proxy.generation == preview.generation
        && proxy.doc == preview.doc
        && proxy.revision == preview.revision
        && proxy.layer == preview.layer
        && proxy.index == preview.index)
        .then(|| proxy.clone())
}

/// Polls the single coalescing sampler and submits at most one latest-state job every 50 ms.
/// A caller should request another repaint when the returned duration is present.
pub(crate) fn fill_mask_proxy_at(app: &mut PhotocraftApp, idx: usize, now: Instant) -> (Option<FillMaskProxy>, Option<Duration>) {
    invalidate_sampler_for_active_context(app);
    if app.session.active_index() != Some(idx) {
        return (None, None);
    }
    let Some(state) = app.session.documents().get(idx) else { return (None, None) };
    let (doc, revision) = (state.doc.clone(), state.revision);
    let Some(preview) = app.uv_mask_paint_preview.as_mut().filter(|p| p.doc == doc.id && p.revision == revision && p.bounds.is_some() && p.rotation.is_some())
    else {
        return (None, None);
    };
    let valid_target = doc
        .layer(preview.layer)
        .and_then(|layer| layer.mask.as_ref())
        .and_then(|mask| mask.paint.as_ref())
        .and_then(|stack| stack.layers.get(preview.index))
        .is_some_and(|effect| matches!(effect.kind, photocraft_doc::mask_paint::EffectKind::Fill { .. }));
    if !valid_target {
        return (None, None);
    }
    let key = preview_key(preview.revision, preview.layer, preview.index, preview.opacity, preview.bounds, preview.rotation);
    if let Some(worker) = preview.mask_sampler.worker.as_ref()
        && let Some(result) = worker.take_result()
        && result.generation == preview.generation
        && result.doc == preview.doc
        && result.revision == preview.revision
        && result.layer == preview.layer
        && result.index == preview.index
    {
        preview.mask_sampler.ready = Some(result);
    }

    let due = preview.mask_sampler.schedule.accept(now, key);
    if due {
        if preview.mask_sampler.worker.is_none() {
            preview.mask_sampler.worker = FillMaskWorker::new();
        }
        let Some(worker) = preview.mask_sampler.worker.as_ref() else {
            preview.mask_sampler.schedule.latest_key = None;
            preview.mask_sampler.schedule.last_sampled_at = None;
            return (preview.mask_sampler.ready.clone(), Some(FILL_WORKER_POLL_INTERVAL));
        };
        let Some(mut stack) = doc.layer(preview.layer).and_then(|layer| layer.mask.as_ref()).and_then(|mask| mask.paint.as_ref()).cloned() else {
            return (preview.mask_sampler.ready.clone(), None);
        };
        let Some(target) = stack.layers.get_mut(preview.index) else { return (preview.mask_sampler.ready.clone(), None) };
        target.opacity = preview.opacity;
        target.fill_bounds = preview.bounds;
        if let photocraft_doc::mask_paint::EffectKind::Fill { rotation, .. } = &mut target.kind {
            *rotation = preview.rotation.unwrap_or(*rotation);
        }
        let canvas = doc.bounds();
        let largest = canvas.width().max(canvas.height()).max(1);
        let scale = largest.div_ceil(FILL_PROXY_MAX_SIDE).max(1);
        let size = [canvas.width().div_ceil(scale).max(1), canvas.height().div_ceil(scale).max(1)];
        worker.submit(FillMaskJob {
            generation: preview.generation,
            doc: doc.id,
            revision,
            layer: preview.layer,
            index: preview.index,
            key,
            token: 0,
            canvas,
            size,
            stack,
        });
    }
    let busy = preview.mask_sampler.worker.as_ref().is_some_and(FillMaskWorker::is_busy);
    let until_sample = preview.mask_sampler.schedule.until_next(now);
    let repaint_after = if busy || due {
        Some(FILL_WORKER_POLL_INTERVAL)
    } else if preview.mask_sampler.schedule.latest_key != Some(key) {
        until_sample
    } else {
        None
    };
    (preview.mask_sampler.ready.clone(), repaint_after)
}

#[cfg(test)]
pub(crate) fn hold_fill_sample_schedule(app: &mut PhotocraftApp, idx: usize, now: Instant) {
    let raw_key = current_fill_key(app, idx);
    if let (Some(raw_key), Some(preview)) = (raw_key, app.uv_mask_paint_preview.as_mut()) {
        preview.mask_sampler.schedule.last_sampled_at = Some(now);
        preview.mask_sampler.schedule.latest_key = Some(raw_key);
    }
}

pub(crate) fn fill_mask_proxy(app: &mut PhotocraftApp, idx: usize, ctx: &egui::Context) -> Option<FillMaskProxy> {
    let (proxy, repaint_after) = fill_mask_proxy_at(app, idx, Instant::now());
    if let Some(delay) = repaint_after {
        ctx.request_repaint_after(delay);
    }
    proxy
}

fn sample_fill_mask(job: FillMaskJob, cancel_epoch: &AtomicU64) -> Option<FillMaskProxy> {
    if cancel_epoch.load(Ordering::Relaxed) != job.token {
        return None;
    }
    // Compose once at the proxy's resolution. Calling sample_stack_pixel per proxy texel turns
    // each Blur layer into a fresh source-resolution Gaussian neighborhood; at radius 64, even
    // the bounded 256² proxy would repeat tens of millions of taps. The compose API scales the
    // blur kernel to the proxy grid, which is sufficient for this deliberately low-resolution
    // transient preview.
    let values = photocraft_compose::mask_paint::sample_stack_resampled(&job.stack, job.canvas, job.size)?;
    if cancel_epoch.load(Ordering::Relaxed) != job.token {
        return None;
    }
    Some(FillMaskProxy {
        generation: job.generation,
        doc: job.doc,
        revision: job.revision,
        layer: job.layer,
        index: job.index,
        key: job.key,
        size: job.size,
        values: Arc::new(values.into_iter().map(|value| value.clamp(0.0, 1.0)).collect()),
    })
}

pub(crate) fn current_fill_gray_mask(app: &PhotocraftApp, idx: usize) -> bool {
    let Some(state) = app.session.documents().get(idx) else { return false };
    let Some(preview) =
        app.uv_mask_paint_preview.as_ref().filter(|p| p.doc == state.doc.id && p.revision == state.revision && p.bounds.is_some() && p.rotation.is_some())
    else {
        return false;
    };
    let explicit =
        state.channel_view.layer_mask.is_some_and(|view| view.layer == preview.layer && view.mode == photocraft_engine::mask_view_cmds::MaskViewMode::Gray);
    let targeted = state.channel_view.target == photocraft_engine::channel_cmds::ChannelTarget::LayerMask && state.active_layer == Some(preview.layer);
    (explicit || targeted) && state.doc.layer(preview.layer).is_some_and(|layer| layer.mask.is_some())
}

pub(crate) fn fill_manipulation_active(app: &PhotocraftApp, idx: usize) -> bool {
    current_fill_key(app, idx).is_some()
}

pub(crate) fn cancel_fill(app: &mut PhotocraftApp, layer: LayerId, index: usize) {
    let active_doc = app.session.active().map(|st| st.doc.id);
    if app.uv_mask_paint_preview.as_ref().is_some_and(|p| Some(p.doc) == active_doc && p.layer == layer && p.index == index && p.bounds.is_some()) {
        app.uv_mask_paint_preview = None;
    }
}

/// An immutable display snapshot. Fill gestures keep the full-resolution document canonical;
/// channel and UV views consume [`fill_mask_proxy`] separately.
pub(crate) fn display_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    invalidate_sampler_for_active_context(app);
    let st = app.session.documents().get(idx)?;
    let source = st.doc.clone();
    let revision = st.revision;
    let p = app.uv_mask_paint_preview.as_mut().filter(|p| p.doc == source.id && p.revision == revision)?;
    let key = preview_key(p.revision, p.layer, p.index, p.opacity, p.bounds, p.rotation);
    if let Some((seen, shown)) = &p.shown
        && *seen == key
    {
        return Some((shown.clone(), key));
    }
    let is_fill = source
        .layer(p.layer)
        .and_then(|layer| layer.mask.as_ref())
        .and_then(|mask| mask.paint.as_ref())
        .and_then(|stack| stack.layers.get(p.index))
        .is_some_and(|effect| matches!(effect.kind, photocraft_doc::mask_paint::EffectKind::Fill { .. }));
    if is_fill {
        // A Fill manipulator is drawn from raw parameters separately. No full-resolution compose
        // runs on the UI thread or in a per-pointer worker job.
        return Some((source, 0));
    }
    let mut doc = (*source).clone();
    let paint = doc.layer_mut(p.layer)?.mask.as_mut()?.paint.as_mut()?.layers.get_mut(p.index)?;
    if p.opacity.is_finite() {
        paint.opacity = p.opacity;
    }
    let shown = Arc::new(doc);
    p.shown = Some((key, shown.clone()));
    Some((shown, key))
}

fn invalidate_sampler_for_active_context(app: &mut PhotocraftApp) {
    let active = app.session.active().map(|state| (state.doc.id, state.revision));
    let Some(preview) = app.uv_mask_paint_preview.as_mut() else { return };
    let has_sampler_state = preview.mask_sampler.worker.is_some() || preview.mask_sampler.ready.is_some() || preview.mask_sampler.schedule.latest_key.is_some();
    if (preview.bounds.is_some() || preview.rotation.is_some()) && active != Some((preview.doc, preview.revision)) && has_sampler_state {
        preview.generation = NEXT_PREVIEW_GENERATION.fetch_add(1, Ordering::Relaxed);
        preview.mask_sampler = FillMaskSampler::default();
    }
}

/// Full-resolution canvas rendering stays on the canonical document during spatial Fill edits.
/// The low-resolution mask proxy is a separate display path.
pub(crate) fn gpu_display_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    display_doc(app, idx)
}

/// CPU-only consumers (the UV map builder or unsupported GPU fallback) need the conventional
/// flattened cache. Reuse an already materialized Fill or build once per opacity value.
pub(crate) fn cpu_document(app: &mut PhotocraftApp, doc: Arc<Document>, key: u64) -> Arc<Document> {
    let revision = app.session.documents().iter().find(|st| st.doc.id == doc.id).map(|st| st.revision);
    let Some(p) = app.uv_mask_paint_preview.as_mut().filter(|p| p.doc == doc.id && Some(p.revision) == revision) else { return doc };
    if let Some((cached, materialized)) = &p.materialized
        && *cached == key
    {
        return materialized.clone();
    }
    let mut cpu = (*doc).clone();
    if p.bounds.is_none()
        && p.rotation.is_none()
        && let Some(mask) = cpu.layer_mut(p.layer).and_then(|layer| layer.mask.as_mut())
    {
        photocraft_compose::mask_paint::refresh_in_canvas(mask, doc.bounds());
    }
    let cpu = Arc::new(cpu);
    p.materialized = Some((key, cpu.clone()));
    cpu
}

/// Take the latest value so app.run can commit it once before another command consumes the doc.
pub(crate) fn pending(app: &PhotocraftApp) -> Option<Commit> {
    let p = app.uv_mask_paint_preview.as_ref()?;
    Some(Commit { doc: p.doc, revision: p.revision, layer: p.layer, index: p.index, opacity: p.opacity, bounds: p.bounds, rotation: p.rotation })
}
pub(crate) fn active_document(app: &PhotocraftApp) -> Option<DocId> {
    let p = app.uv_mask_paint_preview.as_ref()?;
    app.session.documents().iter().any(|st| st.doc.id == p.doc && st.revision == p.revision).then_some(p.doc)
}

pub(crate) fn clear(app: &mut PhotocraftApp, commit: Commit) {
    if app.uv_mask_paint_preview.as_ref().is_some_and(|p| (p.doc, p.revision, p.layer, p.index) == (commit.doc, commit.revision, commit.layer, commit.index)) {
        app.uv_mask_paint_preview = None;
    }
}

pub(crate) fn cancel_stale_on_escape(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(p) = app.uv_mask_paint_preview.as_ref() else { return };
    let stale = !app.session.documents().iter().any(|st| st.doc.id == p.doc && st.revision == p.revision);
    if stale && app.ui.dialogs.is_empty() && !ctx.egui_wants_keyboard_input() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        cancel(app);
        app.ui.status = "Discarded an outdated mask Paint opacity preview".into();
    }
}

pub(crate) fn cancel(app: &mut PhotocraftApp) {
    app.uv_mask_paint_preview = None;
}

fn preview_key(revision: u64, layer: LayerId, index: usize, opacity: f32, bounds: Option<[i32; 4]>, rotation: Option<f32>) -> u64 {
    let bounds_key = bounds.map_or(0, |b| b.into_iter().fold(0_u64, |key, value| key.rotate_left(13) ^ value as u32 as u64));
    let rotation_key = rotation.map_or(0, f32::to_bits) as u64;
    let opacity_key = if opacity.is_finite() { u64::from(opacity.to_bits()) } else { 0 };
    let seed = revision.wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ layer.0.rotate_left(19)
        ^ (index as u64).rotate_left(37)
        ^ opacity_key
        ^ bounds_key.rotate_left(9)
        ^ rotation_key.rotate_left(29);
    (seed & (KEY_BASE - 1)) | KEY_BASE
}

#[cfg(test)]
#[path = "uv_mask_paint_preview_tests.rs"]
mod tests;
