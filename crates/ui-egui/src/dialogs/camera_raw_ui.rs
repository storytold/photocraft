//! Filter › Camera Raw Filter…: the app's side of the framework's RAW Develop workspace
//! (`craft::imaging::ui::raw_develop`).
//!
//! The workspace keeps the settings, their history, the scope's state and the color samplers; this module gives it the
//! develop. [`PcRawSource`] develops a CPU proxy of the layer (≤ 900 px, `pixel_scale` keeps pixel radii true to the
//! full image) on the runtime's job queue, on the GPU where the settings allow it (`camera_raw_gpu`), and refines the
//! visible part at full resolution. A develop writes its picture and its counts into the source's result slot; the
//! canvas reads the picture, so a landing needs no event and no new source.
//!
//! OK runs `filter.cameraRaw` with the non-default settings, so the result is one history step (or a smart filter on a
//! smart object) and exactly replayable. [`take`] answers the workspace's events and finish.
//!
//! Control channel: `ui.menu.invoke {"id":"filter.cameraRaw","params":{"ui":{"set":{…},
//! "commit":true | "cancel":true}}}`; the reply describes the dialog ([`describe`]). `{"ui":{"saveSettings":true}}`
//! and `{"ui":{"loadSettings":true}}` run Save Settings… / Load Settings… (Camera Raw `.xmp` settings files, see
//! [`photocraft_io::camera_raw_xmp`]) through the file dialog.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use craft::behavior::PixelSize;
use craft::imaging::raw::Demosaic as FrameworkDemosaic;
use craft::imaging::scopes::ClippingMask;
use craft::imaging::ui::raw_develop::{RawDevelopEvent, RawDevelopResult, RawDevelopSource, RawDevelopWorkspace, RawFinish, RawPreviewRequest};
use craft::imaging::ui::scopes::{ScopeFrame, ScopeReadout};
use craft::imaging::ui::wide_angle::PreviewDone;
use craft::ui::canvas::{PixelRect, RasterSource};
use craft::ui::dialog::{DialogResult, FullWindowWorkspace, HostedWorkspace, dialog_id};
use craft::ui::primitives::Revision;
use craft::ui::runtime::env::Label;
use craft::ui::runtime::pending::Job;
use egui::ColorImage;
use photocraft_algo::camera_raw::{CameraRaw, DevelopInputs};
use photocraft_algo::histogram::RgbHistogram;
use photocraft_doc::LayerId;
use photocraft_geom::Rect;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::camera_raw_gpu::CameraRawGpu;
use crate::dialogs::camera_raw_detail_ui::DetailPreview;
use crate::dialogs::camera_raw_scope_ui as scope_ui;
use crate::dialogs::shared_picture::SharedPicture;

mod convert;
mod filmstrip;
mod menu;
mod open;
pub(crate) mod proxy;
mod sidecar;

pub use filmstrip::HeldRaw;
pub use menu::menu;
pub use open::{
    REDEVELOP_JOB, RawOpen, Redevelop, on_redevelop_event, open, open_smart_filter, opened_without_dialog, queue_open_dialog, wants_open_dialog,
    without_develop_note,
};
use open::{Target, non_defaults};
use proxy::{ProxyBase, ProxyJob};
pub use sidecar::sidecar_path;

const PROXY_SIDE: usize = 900;
/// Smallest run of proxy pixels one rayon task takes in the per-pixel passes: below it the
/// split costs more than the work.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const PARALLEL_MIN_PIXELS: usize = 16 * 1024;

/// The hosted workspace this module opens.
pub(crate) type Hosted = HostedWorkspace<RawDevelopWorkspace>;

/// What a develop leaves for the canvas and the readers: its counts, timings and settings. A newer develop replaces it.
pub(crate) struct Developed {
    /// The request's revision.
    pub(crate) revision: u64,
    /// The settings it developed.
    pub(crate) settings: CameraRaw,
    /// The developed proxy (straight RGBA).
    pub(crate) processed: Vec<[f32; 4]>,
    /// The developed proxy's histogram.
    pub(crate) after_histogram: RgbHistogram,
    /// Milliseconds of the whole job, of the develop alone and of the histogram.
    pub(crate) render_ms: f64,
    pub(crate) develop_ms: f64,
    pub(crate) histogram_ms: f64,
    /// Where the develop ran: "gpu" or "cpu".
    pub(crate) backend: &'static str,
    /// The scope's counts and clipping mask over the developed pixels.
    pub(crate) scope: crate::dialogs::camera_raw_scope_ui::ScopeData,
}

/// Counts the develop jobs that are queued or running; a job that is dropped unrun counts down too.
struct Running(Arc<AtomicUsize>);

impl Running {
    fn start(count: &Arc<AtomicUsize>) -> Self {
        count.fetch_add(1, Ordering::Relaxed);
        Running(count.clone())
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

/// The after picture: the proxy's develop, replaced by the full-resolution refinement where that is ready.
struct AfterRaster {
    picture: Arc<SharedPicture>,
    detail: Arc<Mutex<DetailPreview>>,
    /// The document's display encoding, for the refinement's pixels.
    display: Arc<photocraft_engine::display_color::CanvasDisplay>,
}

impl AfterRaster {
    fn detail(&self) -> MutexGuard<'_, DetailPreview> {
        self.detail.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl RasterSource for AfterRaster {
    fn size(&self) -> PixelSize {
        RasterSource::size(&*self.picture)
    }

    fn revision(&self) -> Revision {
        Revision::of(&(RasterSource::revision(&*self.picture), self.detail().ready))
    }

    fn damage(&self, since: Revision) -> Option<PixelRect> {
        // The refinement replaces whole regions without a damage record.
        if self.detail().ready { None } else { RasterSource::damage(&*self.picture, since) }
    }

    fn read(&self, region: PixelRect, factor: u32, out: &mut ColorImage) {
        if self.detail().read(region, factor, &self.display, out) {
            return;
        }
        RasterSource::read(&*self.picture, region, factor, out);
    }
}

/// What [`PcRawSource::new`] is built from.
pub(crate) struct SourceInit {
    pub(crate) proxy: Vec<[f32; 4]>,
    pub(crate) selection: Option<Vec<f32>>,
    /// Proxy size, the previewed area's size and the side of the blocks the proxy averaged.
    pub(crate) proxy_size: (usize, usize),
    pub(crate) full_size: (usize, usize),
    pub(crate) block: usize,
    pub(crate) float: bool,
    pub(crate) display: Arc<photocraft_engine::display_color::CanvasDisplay>,
    pub(crate) gpu: Option<CameraRawGpu>,
    pub(crate) encoding_gamma: f32,
    pub(crate) lab_transform: Arc<craft::cms::Transform>,
    pub(crate) scope_transform: Arc<craft::cms::Transform>,
    pub(crate) detail: DetailPreview,
    pub(crate) ctx: egui::Context,
}

#[cfg(test)]
type BeforePreviewPublish = Arc<dyn Fn(u64, &CameraRaw) + Send + Sync>;

/// PhotoCraft's develop for the RAW Develop workspace: the proxy, the selection coverage, the GPU handle and the
/// result slot the develop jobs fill. Nothing in it is kept across passes beyond those results.
pub(crate) struct PcRawSource {
    /// Proxy pixels (straight RGBA) and size.
    pub(crate) proxy: Arc<Vec<[f32; 4]>>,
    pub(crate) selection: Option<Arc<Vec<f32>>>,
    pub(crate) pw: usize,
    pub(crate) ph: usize,
    /// The previewed area's size, which the pictures cover.
    pub(crate) full_w: usize,
    pub(crate) full_h: usize,
    float: bool,
    /// The open-time dialog's camera tone curve: Shadows / Highlights act on the raw data there, in linear light before
    /// this curve, and the preview undoes it to show the same.
    raw_curve: Mutex<Option<Arc<Vec<[f32; 2]>>>>,
    display: Arc<photocraft_engine::display_color::CanvasDisplay>,
    pub(crate) gpu: Option<CameraRawGpu>,
    /// Names [`Self::proxy`] for the GPU pass's upload cache.
    source_key: u64,
    /// The develop's inputs besides the settings: the proxy's scale and the document's transfer curve.
    pub(crate) inputs: DevelopInputs,
    pub(crate) lab_transform: Arc<craft::cms::Transform>,
    pub(crate) scope_transform: Arc<craft::cms::Transform>,
    pub(crate) before_histogram: RgbHistogram,
    pub(crate) detail: Arc<Mutex<DetailPreview>>,
    before: Arc<SharedPicture>,
    after: AfterRaster,
    /// The last develop's result; the jobs fill it before they return.
    developed: Arc<Mutex<Option<Arc<Developed>>>>,
    running: Arc<AtomicUsize>,
    /// Develops started so far: the order of the results.
    issued: AtomicU64,
    /// Tests can hold one real job after its cancellation check, before the revision guard.
    #[cfg(test)]
    before_preview_publish: Mutex<Option<BeforePreviewPublish>>,
    /// The side of the blocks the proxy averaged: the pictures' reduction.
    block: u32,
    /// Wakes the UI when a refinement lands.
    ctx: egui::Context,
}

impl PcRawSource {
    /// Whether the open-time dialog knows how the raw was developed (its camera tone curve).
    #[cfg(test)]
    pub(crate) fn has_raw_curve(&self) -> bool {
        self.raw_curve.lock().unwrap_or_else(std::sync::PoisonError::into_inner).is_some()
    }

    /// A source over `init`; its pictures are empty until the first develop lands.
    pub(crate) fn new(init: SourceInit) -> Self {
        let SourceInit { proxy, selection, proxy_size, full_size, block, float, display, gpu, encoding_gamma, lab_transform, scope_transform, detail, ctx } =
            init;
        let size = PixelSize::new(full_size.0 as u32, full_size.1 as u32);
        // Before (Y) is the undeveloped proxy, shown through the document's display.
        let before = SharedPicture::new(size);
        before.replace(image(&proxy, proxy_size.0, proxy_size.1, &display), block as u32);
        let detail = Arc::new(Mutex::new(detail));
        let picture = SharedPicture::new(size);
        let after = AfterRaster { picture, detail: detail.clone(), display: display.clone() };
        PcRawSource {
            before_histogram: RgbHistogram::from_rgba(&proxy),
            proxy: Arc::new(proxy),
            selection: selection.map(Arc::new),
            pw: proxy_size.0,
            ph: proxy_size.1,
            full_w: full_size.0,
            full_h: full_size.1,
            float,
            raw_curve: Mutex::new(None),
            display,
            gpu,
            source_key: crate::camera_raw_gpu::source_key(),
            inputs: DevelopInputs { seed: 0, pixel_scale: (proxy_size.0 as f32 / full_size.0.max(1) as f32).min(1.0), encoding_gamma },
            lab_transform,
            scope_transform,
            before,
            after,
            detail,
            developed: Arc::default(),
            running: Arc::default(),
            issued: AtomicU64::new(0),
            #[cfg(test)]
            before_preview_publish: Mutex::new(None),
            block: block as u32,
            ctx,
        }
    }

    /// Gives the open-time dialog's camera tone curve to the proxy develop and the refinement.
    pub(crate) fn set_raw_curve(&self, curve: Option<Arc<Vec<[f32; 2]>>>) {
        self.detail.lock().unwrap_or_else(std::sync::PoisonError::into_inner).raw_curve = curve.clone();
        *self.raw_curve.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = curve;
    }

    /// The last develop's result, once one has landed.
    pub(crate) fn developed(&self) -> Option<Arc<Developed>> {
        self.developed.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }

    /// Whether a develop job is queued or running.
    pub(crate) fn pending(&self) -> bool {
        self.running.load(Ordering::Relaxed) > 0
    }

    /// The GPU's or the CPU's name for the last develop.
    pub(crate) fn develop_backend(&self) -> &'static str {
        self.developed().map_or("cpu", |d| d.backend)
    }

    /// What one develop of the proxy reads besides the settings, owned so it runs off the UI thread.
    pub(crate) fn proxy_base(&self) -> ProxyBase {
        ProxyBase {
            proxy: self.proxy.clone(),
            selection: self.selection.clone(),
            raw_curve: self.raw_curve.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone(),
            float: self.float,
            size: (self.pw, self.ph),
            full_w: self.full_w,
            display: self.display.clone(),
            gpu: self.gpu.clone(),
            source_key: self.source_key,
            inputs: self.inputs,
        }
    }

    /// One develop of the proxy with `settings`.
    fn proxy_job(&self, settings: &CameraRaw) -> ProxyJob {
        ProxyJob { base: self.proxy_base(), params: settings.clone() }
    }

    /// Whether the view shows more pixels than the proxy has, so the visible part is worth refining.
    fn needs_detail(&self, request: &RawPreviewRequest) -> bool {
        request.zoom > self.pw as f64 / self.full_w.max(1) as f64
    }
}

impl PcRawSource {
    /// `at`, a point in the previewed area's pixels (the workspace's image pixels: its hover and sampler points), as
    /// fractions of that area, which the proxy pictures cover.
    fn fraction(&self, at: [f64; 2]) -> [f64; 2] {
        [at[0] / self.full_w.max(1) as f64, at[1] / self.full_h.max(1) as f64]
    }
}

impl RawDevelopSource for PcRawSource {
    fn after(&self) -> &dyn RasterSource {
        &self.after
    }

    fn before(&self) -> &dyn RasterSource {
        &*self.before
    }

    fn develop(&self, request: &RawPreviewRequest) -> Job<PreviewDone, String> {
        if self.needs_detail(request) {
            self.detail.lock().unwrap_or_else(std::sync::PoisonError::into_inner).request(&self.ctx, request.revision, &request.settings, &self.inputs);
        }
        // The request's revision only counts source changes; the develops of one source are told apart by this order, so
        // an older job that finishes after a newer one does not replace its result.
        let order = self.issued.fetch_add(1, Ordering::Relaxed) + 1;
        let job = self.proxy_job(&request.settings);
        let settings = request.settings.clone();
        let picture = self.after.picture.clone();
        let developed = self.developed.clone();
        let proxy = self.proxy.clone();
        let selection = self.selection.clone();
        let scope_transform = self.scope_transform.clone();
        let before_histogram = self.before_histogram.clone();
        let (pw, ph) = (self.pw, self.ph);
        let block = self.block;
        #[cfg(test)]
        let before_preview_publish = self.before_preview_publish.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        let running = Running::start(&self.running);
        Job::new(move |_progress, cancel| {
            let _running = running;
            let frame = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job.run())).map_err(|_| "RAW Develop preview failed".to_string())?;
            if cancel.is_cancelled() {
                return Err("RAW Develop preview cancelled".to_string());
            }
            let scope = crate::dialogs::camera_raw_scope_ui::ScopeData::new(crate::dialogs::camera_raw_scope_ui::ScopeInputs {
                revision: order,
                size: [pw, ph],
                proxy: &proxy,
                developed: &frame.px,
                selection: selection.as_deref().map(Vec::as_slice),
                before_histogram: &before_histogram,
                after_histogram: &frame.histogram,
                scope_transform: &scope_transform,
            });
            let timing = Duration::from_secs_f64(frame.ms.max(0.0) / 1000.0);
            let label = if frame.backend == "gpu" { "GPU" } else { "CPU" };
            let result = Developed {
                revision: order,
                settings,
                processed: frame.px,
                after_histogram: frame.histogram,
                render_ms: frame.ms,
                develop_ms: frame.develop_ms,
                histogram_ms: frame.histogram_ms,
                backend: frame.backend,
                scope,
            };
            #[cfg(test)]
            if let Some(before_publish) = before_preview_publish {
                before_publish(order, &result.settings);
            }
            // A job for older settings that finishes after a newer one must not replace its result.
            let mut slot = developed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if slot.as_ref().is_none_or(|current| current.revision <= result.revision) {
                picture.replace(frame.image, block);
                *slot = Some(Arc::new(result));
            }
            Ok(PreviewDone::new(timing).backend(label))
        })
    }

    fn auto(&self, settings: &CameraRaw) -> CameraRaw {
        let mut next = settings.clone();
        photocraft_algo::camera_raw::auto_tone(&self.proxy, &mut next);
        next
    }

    fn scope(&self, before: bool, region: bool, at: Option<[f64; 2]>, show: &mut dyn FnMut(Option<ScopeFrame<'_>>)) {
        let Some(developed) = self.developed() else {
            show(None);
            return;
        };
        let pixels: &[[f32; 4]] = if before { &self.proxy } else { &developed.processed };
        let hover = at
            .and_then(|at| scope_ui::pixel_at(self.fraction(at), [self.pw, self.ph], pixels, |_| None))
            .and_then(|pixel| scope_ui::hover_target(pixel, &self.scope_transform));
        developed.scope.show(before, region, hover, show);
    }

    fn clipping(&self) -> Option<(Arc<ClippingMask>, Revision)> {
        self.developed().map(|developed| developed.scope.clipping())
    }

    fn sample(&self, at: [f64; 2], before: bool, lab: bool) -> Option<ScopeReadout> {
        let developed = self.developed()?;
        let pixels: &[[f32; 4]] = if before { &self.proxy } else { &developed.processed };
        let pixel = scope_ui::pixel_at(self.fraction(at), [self.pw, self.ph], pixels, |_| None)?;
        Some(scope_ui::readout(pixel, lab, &self.lab_transform))
    }

    fn has_selection(&self) -> bool {
        self.selection.is_some()
    }
}

/// App state beside the hosted workspace while RAW Develop is open.
pub(crate) struct RawSession {
    /// The layer the filter applies to.
    pub(crate) layer: LayerId,
    pub(crate) target: Target,
    /// Set for the open-time dialog ([`Target::OpenRaw`]).
    pub(crate) raw_open: Option<RawOpen>,
    /// Settings when the dialog opened (stored settings of an edited smart filter): what OK validates against.
    pub(crate) original: CameraRaw,
    /// The develop the workspace reads.
    pub(crate) source: Arc<PcRawSource>,
    /// The GPU develop, when the app may use one; the tests read it to check the pass.
    #[cfg(test)]
    pub(crate) gpu: Option<CameraRawGpu>,
}

/// The hosted workspace, while it is open.
pub(crate) fn hosted(app: &PhotocraftApp) -> Option<&Hosted> {
    app.framework.dialogs.get::<Hosted>()
}

/// The hosted workspace, for the app's answers to its events.
pub(crate) fn hosted_mut(app: &mut PhotocraftApp) -> Option<&mut Hosted> {
    app.framework.dialogs.get_mut::<Hosted>()
}

/// The settings the workspace holds now: the one copy while it is open.
pub(crate) fn settings(app: &PhotocraftApp) -> Option<&CameraRaw> {
    hosted(app).map(|h| h.workspace().session())
}

/// The demosaic the workspace shows for the raw being opened.
pub(crate) fn workspace_demosaic(hosted: &Hosted) -> Option<photocraft_io::raw::Demosaic> {
    let fields = hosted.workspace().json_fields(&**hosted.source())?;
    let value = fields.get("demosaic")?.clone();
    serde_json::from_value::<FrameworkDemosaic>(value).ok().map(from_framework)
}

/// The framework's demosaic for the app's.
pub(crate) fn to_framework(d: photocraft_io::raw::Demosaic) -> FrameworkDemosaic {
    use photocraft_io::raw::Demosaic;
    match d {
        Demosaic::Ahd => FrameworkDemosaic::Ahd,
        Demosaic::Mhc => FrameworkDemosaic::Mhc,
        Demosaic::Bilinear => FrameworkDemosaic::Bilinear,
    }
}

/// The app's demosaic for the framework's.
pub(crate) fn from_framework(d: FrameworkDemosaic) -> photocraft_io::raw::Demosaic {
    use photocraft_io::raw::Demosaic;
    match d {
        FrameworkDemosaic::Mhc => Demosaic::Mhc,
        FrameworkDemosaic::Bilinear => Demosaic::Bilinear,
        _ => Demosaic::Ahd,
    }
}

/// Params for the engine: only the settings that differ from the defaults.
pub(crate) fn command_params(settings: &CameraRaw) -> Value {
    non_defaults(settings)
}

/// `px` (`w`×`h`) as a display image. Bands are copied straight from `px` and converted in parallel (#4682), not a
/// whole copy of the proxy converted on one thread.
pub(crate) fn image(px: &[[f32; 4]], w: usize, h: usize, display: &photocraft_engine::display_color::CanvasDisplay) -> ColorImage {
    crate::dialogs::camera_raw_detail_ui::display_image(Rect::new(0, 0, w as i32, h as i32), display, |r, out| {
        let start = (r.y0.max(0) as usize).saturating_mul(w);
        if let Some(src) = px.get(start..start.saturating_add(out.len())) {
            out.copy_from_slice(src);
        }
    })
}

/// The reply that describes RAW Develop: its settings and what the develop and the scope hold. `null` while it is
/// closed.
pub(crate) fn describe(app: &PhotocraftApp) -> Value {
    let (Some(session), Some(hosted)) = (app.raw.as_ref(), hosted(app)) else { return Value::Null };
    let source = &session.source;
    let developed = source.developed();
    let fields = hosted.workspace().json_fields(&**hosted.source()).unwrap_or(Value::Null);
    let before = fields.get("before").and_then(Value::as_bool).unwrap_or(false);
    let scope = fields.get("scope").cloned().unwrap_or(Value::Null);
    let region = scope.get("selectedRegion").and_then(Value::as_bool).unwrap_or(false);
    let h = if before { Some(&source.before_histogram) } else { developed.as_ref().map(|d| &d.after_histogram) };
    let histogram = h.map(|h| {
        json!({"source": if before { "before" } else { "after" }, "approximate": true, "size": [source.pw, source.ph],
            "sampleDomain": "preview-rgb", "range": [0, 1], "red": h.channels[0].as_slice(), "green": h.channels[1].as_slice(), "blue": h.channels[2].as_slice(),
            "samples": h.samples, "transparent": h.transparent, "invalid": h.invalid, "underflow": h.underflow, "overflow": h.overflow, "shadows": h.shadows,
            "highlights": h.highlights})
    });
    let detail = source.detail.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut result = json!({
        "layer": session.layer.0,
        "params": serde_json::to_value(hosted.workspace().session()).unwrap_or(Value::Null),
        "proxy": [source.pw, source.ph],
        "renderMs": developed.as_ref().map_or(0.0, |d| d.render_ms),
        "developMs": developed.as_ref().map_or(0.0, |d| d.develop_ms),
        "developBackend": source.develop_backend(),
        "histogramMs": developed.as_ref().map_or(0.0, |d| d.histogram_ms),
        "previewRevision": developed.as_ref().map_or(0, |d| d.revision),
        "histogram": histogram,
        "smartFilter": match session.target {
            Target::SmartFilter(index) => Some(index),
            Target::NewFilter | Target::OpenRaw => None,
        },
        "openingRaw": session.raw_open.as_ref().map(|r| json!({"name": r.name, "path": r.path, "demosaic": workspace_demosaic(hosted).unwrap_or(r.demosaic).id()})),
        "sourceSize": [source.full_w, source.full_h],
        "previewPending": source.pending(),
        "previewApproximate": !detail.ready && (source.pw != source.full_w || source.ph != source.full_h),
        "detailPending": detail.pending(),
        "detailError": detail.error,
        "scope": scope,
        "hasScopeSelection": source.selection.is_some(),
    });
    drop(detail);
    // The vectorscope's counts for the Before/region pair; the workspace's own fields carry the pointer readout and the samplers' readouts.
    result["vectorscope"] = developed.as_ref().map_or(Value::Null, |d| scope_ui::vectorscope_reply(&d.scope, before, region));
    if let (Value::Object(out), Value::Object(own)) = (&mut result, fields) {
        for (key, value) in own {
            out.entry(key).or_insert(value);
        }
    }
    if let Value::Object(out) = &mut result {
        out.entry("pointerReadout").or_insert(Value::Null);
        out.entry("samplerReadouts").or_insert(json!([]));
    }
    result
}

/// The result pump (a `TAKERS` row): answers the workspace's events, files a held OK, and ends a cancelled session.
/// Each call acts on what the workspace reported since the last call, whatever number of passes the frame ran: the host keeps
/// every pass's events and a held OK is taken once. A finish or cancel leaves its reply for the menu request that
/// caused it (`app.raw_reply`).
pub(crate) fn take(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if let Some(reply) = take_with(app, ctx) {
        app.raw_reply = Some(reply);
    }
}

/// [`take`], with the reply of a finish that landed in this call: the command's result, an error, or `{"cancelled": true}`.
pub(crate) fn take_with(app: &mut PhotocraftApp, ctx: &egui::Context) -> Option<Result<Value, String>> {
    // A raw queued by an open (`queue_open_dialog`) shows its dialog on the next pass; the shelf keeps it as the
    // filmstrip's tile while it is shown.
    if app.raw.is_none()
        && !app.raw_shelf.is_empty()
        && let Err(e) = filmstrip::open_held(app, ctx, 0)
    {
        // The document stays open, developed with the defaults.
        app.raw_shelf.remove(0);
        app.ui.status = e;
        app.ui.status_error = true;
    }
    let events = hosted_mut(app).map(Hosted::take_events).unwrap_or_default();
    for event in events {
        let outcome = match event {
            RawDevelopEvent::Show(index) => filmstrip::show_raw(app, ctx, index).map(|()| Value::Null),
            RawDevelopEvent::SaveSettings => sidecar::save_settings(app),
            RawDevelopEvent::LoadSettings => sidecar::load_settings(app),
            // The develop is keyed by the settings, so an edit needs no answer here.
            _ => continue,
        };
        sidecar::report_file_error(app, outcome);
    }
    let mut reply = None;
    // PLAN: raws queued with `queue_open_dialog` after the workspace opened are not in its filmstrip: the workspace has no call to add a tile
    // (owner: dialogs-w17).
    if app.raw.as_ref().is_some_and(|s| s.target == Target::OpenRaw) {
        filmstrip::sync_pictures(app, ctx);
    }
    if let Some(result) = hosted_mut(app).and_then(Hosted::take_ok) {
        let outcome = finish(app, &result);
        if let Some(hosted) = hosted_mut(app) {
            hosted.answer_ok(outcome.as_ref().map(|_| ()).map_err(|e| Label::from(e.clone())));
        }
        reply = Some(outcome);
    }
    match app.framework.dialogs.take::<Hosted>() {
        Some(DialogResult::Cancel) => {
            open::cancel(app);
            reply.get_or_insert(Ok(json!({"cancelled": true})));
        }
        // The held OK was answered and the app acted on it.
        Some(_) => app.raw = None,
        None => {}
    }
    reply
}

/// Closes RAW Develop without acting on it: a request that opened it and was then refused.
pub(crate) fn dismiss(app: &mut PhotocraftApp, ctx: &egui::Context) {
    app.framework.dialogs.cancel_by_id(ctx, dialog_id(RawDevelopWorkspace::KEY));
    let _ = app.framework.dialogs.take::<Hosted>();
    app.raw = None;
}

/// OK of the workspace: the command (or the open) its target asks for. A failure leaves the session as it is, so the
/// workspace stays open with its settings.
fn finish(app: &mut PhotocraftApp, result: &RawDevelopResult) -> Result<Value, String> {
    let session = app.raw.as_ref().ok_or(tl!("RAW Develop isn't open"))?;
    let (layer, target, original) = (session.layer, session.target, session.original.clone());
    let raw = session.raw_open.clone();
    let settings = &result.settings;
    let reply = match (result.finish, target) {
        (RawFinish::Open | RawFinish::Done, Target::OpenRaw) => {
            let mut raw = raw.ok_or("no raw file is being opened")?;
            if let Some(demosaic) = result.demosaic {
                raw.demosaic = from_framework(demosaic);
            }
            filmstrip::keep(app, settings, raw.demosaic, false);
            if result.finish == RawFinish::Done {
                let reply = filmstrip::done(app, settings);
                // A failed write keeps the workspace and every raw open, so no edit is lost.
                if reply.get("done").and_then(Value::as_bool) == Some(false) {
                    return Err(app.ui.status.clone());
                }
                reply
            } else {
                filmstrip::open_all(app, settings, &raw, &original)?
            }
        }
        (_, Target::NewFilter) => {
            let mut p = command_params(settings);
            p["layer"] = json!(layer.0);
            app.run("filter.cameraRaw", p)?
        }
        (_, Target::SmartFilter(index)) => {
            settings.validate_changes(&original)?;
            // Every setting, not just the non-defaults: setParams merges into the stored ones, so a control reset to its
            // default must overwrite the old value.
            let params = serde_json::to_value(settings).map_err(|e| e.to_string())?;
            app.run("layer.smartFilter.setParams", json!({"layer": layer.0, "index": index, "params": params}))?
        }
        (_, Target::OpenRaw) => return Err("Ok is for filters; a raw being opened finishes with Open or Done".into()),
    };
    app.raw = None;
    Ok(reply)
}

/// `CameraRaw` with the named fields set (the type is `#[non_exhaustive]`, so no struct expression), or a copy of a base with them set.
#[cfg(test)]
macro_rules! cr {
    ($($f:ident : $v:expr),* $(,)?) => {{
        let mut s = CameraRaw::default();
        $(s.$f = $v;)*
        s
    }};
    ($base:expr => $($f:ident : $v:expr),* $(,)?) => {{
        let mut s: CameraRaw = $base;
        $(s.$f = $v;)*
        s
    }};
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod identity_tests;
