//! Select › Select and Mask…: the app's side of the framework's Select and Mask workspace.
//!
//! The workspace itself (tools, history, view, Properties panel, keys, Enter and Esc) is craft-imaging's
//! `SelectMaskWorkspace`, hosted on the app's dialog host as a `HostedWorkspace` with a held OK. This file gives it
//! the document: a [`PcSelectMaskSource`] with owned snapshots of the pixels, the preview job (the edits applied on
//! a reduced copy of the document, then the refined selection shown in the view), and the answers to what the
//! workspace reports (the colour picker, OK).
//!
//! A finished preview reaches the display through the source: the job writes the view and the ants into cells the
//! source owns, and the framework asks for the pass that reads them again.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use craft::behavior::PixelSize;
use craft::host::queue::Cancel;
use craft::imaging::trace::{MaskRect, MaskSource};
use craft::imaging::ui::mask_view::MaskViewRecipe;
use craft::imaging::ui::select_mask::{
    OutputTo, SelectMaskCommand, SelectMaskEvent, SelectMaskLimits, SelectMaskOpen, SelectMaskPreviewRequest, SelectMaskSettings, SelectMaskSource,
    SelectMaskWorkspace,
};
use craft::imaging::ui::wide_angle::PreviewDone;
use craft::ui::canvas::RasterSource;
use craft::ui::dialog::{DialogId, HostedWorkspace};
use craft::ui::runtime::memory::SaveKey;
use craft::ui::runtime::pending::Job as PendingJob;
use photocraft_doc::{Document, LayerId};
use photocraft_engine::jobs::JobCtx;
use photocraft_engine::select_mask_edits::{self, MAX_EDIT_POINTS, MAX_EDITS};
use photocraft_engine::smartselect_cmds::{MAX_STROKE_POINTS, MAX_STROKES};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::display_image;
use crate::dialogs::shared_picture::SharedPicture;
use crate::outline::SurfaceMask;

/// Longest side of the reduced copy the preview runs on (px), unless High Quality Preview.
const PROXY_SIDE: usize = 1600;
const CMD: &str = crate::select_mask_view::COMMAND;
/// Where Remember Settings keeps the settings (the framework's SAVED tier).
const REMEMBERED: SaveKey = SaveKey::new("photocraft.select.refineEdge.workspace");

type Hosted = HostedWorkspace<SelectMaskWorkspace>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The selection after the first `edits` (document coordinates) on the copy reduced by `k`.
#[derive(Clone)]
struct BaseCache {
    k: u32,
    /// The document revision the base was computed from.
    revision: u64,
    edits: Vec<Value>,
    base: Option<Surface>,
}

/// What the next preview reuses: the base and the reduced copy it was made on.
#[derive(Clone)]
struct Cache {
    base: BaseCache,
    proxy: Arc<Document>,
}

/// The ants surface, its own revision and the size of the reduced copy it covers, behind the lock the preview job and
/// the overlay share.
type AntsCell = Mutex<Option<(Arc<Surface>, u64, [u32; 2])>>;

/// The refined selection the Marching Ants view outlines, owned so the framework's overlay can read it through
/// [`MaskSource`] while the preview job replaces it.
pub(crate) struct SharedMask {
    /// The ants surface, its own revision and the size of the reduced copy it covers.
    cell: AntsCell,
    /// Document pixels per mask pixel.
    k: AtomicU32,
    next_revision: AtomicU64,
}

impl SharedMask {
    fn new(k: u32) -> Self {
        SharedMask { cell: Mutex::new(None), k: AtomicU32::new(k), next_revision: AtomicU64::new(1) }
    }

    fn set(&self, surface: Option<Surface>, size: [u32; 2], k: u32) {
        let revision = self.next_revision.fetch_add(1, Ordering::Relaxed);
        *lock(&self.cell) = surface.map(|s| (Arc::new(s), revision, size));
        self.k.store(k, Ordering::Release);
    }

    fn reduction(&self) -> u32 {
        self.k.load(Ordering::Acquire)
    }

    fn holds_mask(&self) -> bool {
        lock(&self.cell).is_some()
    }

    /// Runs `f` over a borrowing adapter of the held surface, or returns `empty` when there is none. No guard escapes.
    fn with<T>(&self, empty: T, f: impl FnOnce(&SurfaceMask<'_>) -> T) -> T {
        match &*lock(&self.cell) {
            Some((surface, revision, size)) => f(&SurfaceMask::new(surface, *revision, *size)),
            None => empty,
        }
    }
}

impl MaskSource for SharedMask {
    fn size(&self) -> [u32; 2] {
        self.with([0, 0], |m| m.size())
    }

    fn revision(&self) -> u64 {
        self.with(0, |m| m.revision())
    }

    fn content_bounds(&self) -> MaskRect {
        self.with(MaskRect::default(), |m| m.content_bounds())
    }

    fn read_row(&self, y: i32, x0: i32, step: u32, out: &mut [f32]) {
        match &*lock(&self.cell) {
            Some((surface, revision, size)) => SurfaceMask::new(surface, *revision, *size).read_row(y, x0, step, out),
            None => out.fill(0.0),
        }
    }
}

/// What the preview jobs of one source write.
struct Shared {
    view: Arc<SharedPicture>,
    ants: SharedMask,
    cache: Mutex<Option<Cache>>,
    /// Reduced-copy pixels at least half selected in the last preview.
    selected: AtomicUsize,
    /// The error the last preview returned; cleared by the next one that lands.
    error: Mutex<Option<String>>,
    /// The request the last job was started for.
    last_request: Mutex<Option<SelectMaskPreviewRequest>>,
}

/// What the workspace reads from the document: snapshots owned by the source (M-9), rebuilt when the document
/// revision moves.
pub(crate) struct PcSelectMaskSource {
    doc: Arc<Document>,
    revision: u64,
    layer: Option<LayerId>,
    target_mask: bool,
    /// Document pixels per reduced-copy pixel without High Quality Preview.
    k: u32,
    display: Option<Arc<photocraft_engine::display_color::CanvasDisplay>>,
    image: Arc<SharedPicture>,
    shared: Arc<Shared>,
}

impl PcSelectMaskSource {
    fn new(
        doc: Arc<Document>,
        revision: u64,
        layer: Option<LayerId>,
        target_mask: bool,
        display: Option<Arc<photocraft_engine::display_color::CanvasDisplay>>,
    ) -> Self {
        let size = [doc.size.width, doc.size.height];
        let k = u32::try_from((size[0].max(size[1]) as usize).div_ceil(PROXY_SIDE).max(1)).unwrap_or(u32::MAX);
        let pixels = PixelSize::new(size[0], size[1]);
        let image = SharedPicture::new(pixels);
        image.replace(display_image(display.as_deref(), &photocraft_compose::flatten(&doc)), 1);
        let shared = Arc::new(Shared {
            view: SharedPicture::new(pixels),
            ants: SharedMask::new(k),
            cache: Mutex::new(None),
            selected: AtomicUsize::new(0),
            error: Mutex::new(None),
            last_request: Mutex::new(None),
        });
        PcSelectMaskSource { doc, revision, layer, target_mask, k, display, image, shared }
    }

    /// The source over the active document as it is now.
    fn of_active(app: &PhotocraftApp, layer: Option<LayerId>, target_mask: bool) -> Option<Self> {
        let st = app.session.active()?;
        let display = crate::canvas::canvas_display(app, &st.doc, None).0;
        Some(Self::new(Arc::clone(&st.doc), st.revision, layer, target_mask, display))
    }

    /// The job's inputs for `request`.
    fn job(&self, request: &SelectMaskPreviewRequest) -> Job {
        let k = if request.high_quality { 1 } else { self.k };
        let cmd = request.session.command(request.sample_all_layers, self.target_mask);
        let mut params = command_params(&cmd, self.target_mask);
        // The edits are applied to the selection before the refinement; the refinement does not apply them again.
        if let Some(o) = params.as_object_mut() {
            o.remove("edits");
        }
        Job {
            doc: Arc::clone(&self.doc),
            revision: self.revision,
            layer: self.layer,
            k,
            target_mask: self.target_mask,
            edits: request.session.edits.iter().filter_map(|e| serde_json::to_value(e).ok()).collect(),
            params,
            recipe: request.recipe.clone(),
            cache: lock(&self.shared.cache).clone(),
        }
    }
}

impl SelectMaskSource for PcSelectMaskSource {
    fn view(&self) -> &dyn RasterSource {
        &*self.shared.view
    }

    fn image(&self) -> &dyn RasterSource {
        &*self.image
    }

    fn ants(&self) -> Option<(&dyn MaskSource, u32)> {
        self.shared.ants.holds_mask().then(|| (&self.shared.ants as &dyn MaskSource, self.shared.ants.reduction()))
    }

    fn preview_job(&self, request: &SelectMaskPreviewRequest) -> PendingJob<PreviewDone, String> {
        *lock(&self.shared.last_request) = Some(request.clone());
        let job = self.job(request);
        let display = self.display.clone();
        let shared = Arc::clone(&self.shared);
        PendingJob::new(move |_, cancel| {
            let started = Instant::now();
            let (k, ants_wanted) = (job.k.max(1), job.recipe.ants);
            match compute(job, cancel) {
                Ok(done) => {
                    let size = [done.shown.size.width, done.shown.size.height];
                    let selection = done.shown.selection.clone();
                    shared.selected.store(selected_pixels(selection.as_ref(), done.shown.bounds()), Ordering::Release);
                    // Only the Marching Ants view keeps the refined selection; switching to it runs a new preview.
                    shared.ants.set(selection.filter(|_| ants_wanted), size, k);
                    *lock(&shared.cache) = Some(done.cache);
                    shared.view.replace(display_image(display.as_deref(), &photocraft_compose::flatten(&done.shown)), k);
                    *lock(&shared.error) = None;
                    Ok(PreviewDone::new(started.elapsed()))
                }
                Err(e) => {
                    if e != CANCELLED {
                        *lock(&shared.error) = Some(e.clone());
                    }
                    Err(e)
                }
            }
        })
    }
}

/// The error a cancelled preview returns; it is not a failure to show.
const CANCELLED: &str = "cancelled";

/// What a preview computation needs (moved to a worker thread).
struct Job {
    doc: Arc<Document>,
    revision: u64,
    layer: Option<LayerId>,
    k: u32,
    target_mask: bool,
    /// The edits of the request's session (with Real-Time Refinement, the one being drawn is already in it).
    edits: Vec<Value>,
    /// The refinement params with `refineStrokes`, without `edits`.
    params: Value,
    recipe: MaskViewRecipe,
    cache: Option<Cache>,
}

/// A computed preview, before it is written to the source.
struct Computed {
    shown: Document,
    cache: Cache,
}

/// The `select.refineEdge` params of `cmd`: the refinement values, `sampleAllLayers`, `edits`, `refineStrokes` and,
/// for a layer mask, `target`. The preview job and OK use the same function.
fn command_params(cmd: &SelectMaskCommand, target_mask: bool) -> Value {
    let mut params = serde_json::to_value(cmd).unwrap_or(Value::Null);
    if target_mask && let Some(o) = params.as_object_mut() {
        o.insert("target".into(), json!("mask"));
    }
    params
}

/// The facts of the open workspace for the control surface (merged with the workspace's own fields; a refused OK's
/// message is the workspace's own, shown through `ok_failed`).
pub(crate) fn describe(app: &PhotocraftApp) -> Option<Value> {
    let shared = &source(app)?.shared;
    Some(json!({
        "proxyScale": shared.ants.reduction(),
        "selectedProxyPixels": shared.selected.load(Ordering::Acquire),
        "previewReady": shared.view.is_ready(),
        "previewError": *lock(&shared.error),
    }))
}

/// The source of the open workspace: the app keeps its own handle beside the dialog's.
pub(crate) fn source(app: &PhotocraftApp) -> Option<&PcSelectMaskSource> {
    app.select_mask_target.as_ref().map(|(.., source)| &**source)
}

/// Opens the workspace on the selection (or the active layer's mask).
pub fn open(app: &mut PhotocraftApp) -> Result<DialogId, String> {
    // Opening again would drop the open workspace's work: edit it with `ui`, or cancel it.
    if app.framework.dialogs.is_open::<Hosted>() {
        return Err("Select and Mask is already open".into());
    }
    if let Some(Err(why)) = photocraft_engine::commands::find(CMD).map(|c| (c.enabled)(&app.session)) {
        return Err(format!("Select and Mask needs a selection or a layer mask ({why})"));
    }
    // A targeted layer mask is what is refined (Photoshop opens on it from the mask's thumbnail).
    let target_mask = app.layer_mask_targeted();
    let st = app.session.active().ok_or("no document")?;
    let document = st.doc.id;
    let layer = st.active_layer.filter(|id| st.doc.layer(*id).is_some()).ok_or("Select and Mask needs an active layer")?;
    let size = [st.doc.size.width, st.doc.size.height];
    let subject = st.doc.name.clone();
    let mask_only = st.doc.selection.is_none() && st.doc.layer(layer).is_some_and(|l| l.mask.is_some());
    let source = Arc::new(PcSelectMaskSource::of_active(app, Some(layer), target_mask).ok_or("no document")?);
    // The stored settings replace these only when their Remember Settings flag is on.
    let mut settings = SelectMaskSettings::default();
    if mask_only || target_mask {
        settings.refine.output = OutputTo::LayerMask;
    }
    let limits = SelectMaskLimits::new(MAX_EDITS, MAX_EDIT_POINTS, MAX_STROKES, MAX_STROKE_POINTS);
    // PLAN: the navigation, transparency-grid and cursor preferences have no app accessor in this tree; the workspace
    // opens with the framework's defaults for them.
    let open = SelectMaskOpen::new(size, subject, limits).settings(settings).target_mask(target_mask).remember(REMEMBERED);
    let hosted = HostedWorkspace::new(SelectMaskWorkspace::new(open), Arc::clone(&source) as Arc<dyn SelectMaskSource>).held_ok();
    let dialog = app.framework.dialogs.open(hosted);
    app.select_mask_target = Some((dialog, document, layer, source));
    Ok(dialog)
}

/// Why the open workspace's work no longer applies: the active document is not the one it was opened on, or the active
/// layer is not the one it was opened on (another layer selected by a shortcut or the control channel).
/// `select.refineEdge` works on the active layer of the active document, so its edits must not run there.
pub(crate) fn stale(app: &PhotocraftApp) -> Option<&'static str> {
    let (_, document, opened_on, _) = app.select_mask_target.as_ref()?;
    let Some(st) = app.session.active().filter(|st| st.doc.id == *document) else {
        return Some(tl!("Select and Mask was closed: its document is no longer the active one"));
    };
    let layer = st.active_layer.filter(|id| st.doc.layer(*id).is_some());
    (layer != Some(*opened_on)).then_some(tl!("Select and Mask was closed: another layer was selected"))
}

/// One pass of the app's side of the workspace (a `TAKERS` entry): closes a stale workspace, answers what the
/// workspace reported (the colour picker), follows the document's revision, and runs a held OK.
pub(crate) fn take(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(id) = app.select_mask_target.as_ref().map(|t| t.0) else { return };
    if !app.framework.dialogs.is_open::<Hosted>() {
        // Finished: OK ran its command when it was held, so the result carries nothing more to do.
        let _ = app.framework.dialogs.take::<Hosted>();
        app.select_mask_target = None;
        return;
    }
    if let Some(why) = stale(app) {
        app.framework.dialogs.cancel_by_id(ctx, id);
        let _ = app.framework.dialogs.take::<Hosted>();
        app.select_mask_target = None;
        app.ui.status = why.into();
        app.ui.status_error = true;
        return;
    }
    let events = app.framework.dialogs.get_mut_by_id::<Hosted>(id).map(Hosted::take_events).unwrap_or_default();
    for event in events {
        if let SelectMaskEvent::PickColor(request) = event {
            crate::color_picker_ui::open_for_request(app, id, request);
        }
    }
    if let Some((key, color)) = crate::color_picker_ui::answer_for(app, id)
        && let Some(hosted) = app.framework.dialogs.get_mut_by_id::<Hosted>(id)
    {
        hosted.workspace_mut().set_color(&key, color);
    }
    follow_revision(app, id);
    run_held_ok(app, id);
}

/// The document changed under the workspace: a fresh source, so the preview runs again and nothing of the old
/// pixels is shown or reused (`set_source` moves the revision the preview is keyed on).
fn follow_revision(app: &mut PhotocraftApp, id: DialogId) {
    let Some((document, layer, target_mask, seen)) = source(app).map(|s| (s.doc.id, s.layer, s.target_mask, s.revision)) else { return };
    let Some(revision) = app.session.active().filter(|st| st.doc.id == document).map(|st| st.revision) else { return };
    if revision == seen {
        return;
    }
    let Some(source) = PcSelectMaskSource::of_active(app, layer, target_mask).map(Arc::new) else { return };
    if let Some(hosted) = app.framework.dialogs.get_mut_by_id::<Hosted>(id) {
        hosted.set_source(Arc::clone(&source) as Arc<dyn SelectMaskSource>);
    }
    if let Some(target) = app.select_mask_target.as_mut() {
        target.3 = source;
    }
}

/// OK: runs `select.refineEdge`. A failure leaves the workspace open with the error in its panel, so nothing painted
/// is lost.
fn run_held_ok(app: &mut PhotocraftApp, id: DialogId) {
    let Some(done) = app.framework.dialogs.get_mut_by_id::<Hosted>(id).and_then(Hosted::take_ok) else { return };
    let target_mask = source(app).is_some_and(|s| s.target_mask);
    let answer = app.run(CMD, command_params(&done.command, target_mask));
    if let Err(e) = &answer {
        app.ui.status = e.clone();
        app.ui.status_error = true;
    }
    if let Some(hosted) = app.framework.dialogs.get_mut_by_id::<Hosted>(id) {
        hosted.answer_ok(answer.map(|_| ()).map_err(Into::into));
    }
}

/// `e` (document coordinates) on the copy reduced by `k`: points, sizes and rectangles scale by
/// `1 / k`.
fn scale_edit(e: &Value, k: u32) -> Value {
    let k = f64::from(k.max(1));
    let mut e = e.clone();
    let Some(o) = e.as_object_mut() else { return e };
    let scale = |v: &mut Value| {
        if let Some(n) = v.as_f64() {
            *v = json!(n / k);
        }
    };
    if let Some(pts) = o.get_mut("points").and_then(Value::as_array_mut) {
        for p in pts.iter_mut().filter_map(Value::as_array_mut) {
            p.iter_mut().for_each(scale);
        }
    }
    for key in ["point", "rect"] {
        if let Some(a) = o.get_mut(key).and_then(Value::as_array_mut) {
            a.iter_mut().for_each(scale);
        }
    }
    if let Some(size) = o.get("size").and_then(Value::as_f64) {
        o.insert("size".into(), json!((size / k).max(1.0)));
    }
    e
}

/// Count selected pixels without allocating a canvas-sized mask. Missing tiles use the surface's
/// default value, including an inverted selection whose default is selected.
fn selected_pixels(selection: Option<&Surface>, area: Rect) -> usize {
    let Some(surface) = selection else { return 0 };
    let default_selected = surface.default_pixel().first().is_some_and(|v| *v >= 0.5);
    let area_pixels = (area.width() as usize).saturating_mul(area.height() as usize);
    let mut count = if default_selected { area_pixels } else { 0 };
    for (coord, _) in surface.tiles() {
        let tile_area = coord.rect().intersect(&area);
        if tile_area.is_empty() {
            continue;
        }
        let pixels = (tile_area.width() as usize).saturating_mul(tile_area.height() as usize);
        if default_selected {
            count = count.saturating_sub(pixels);
        }
        let values = photocraft_algo::selection::mask_from_surface(Some(surface), tile_area);
        count = count.saturating_add(values.iter().filter(|v| **v >= 0.5).count());
    }
    count
}

/// Runs `run` with an engine `JobCtx` that is cancelled when the framework's `cancel` is: the engine's refinement
/// checks the `JobCtx`, the framework job its `Cancel`.
#[cfg(not(target_arch = "wasm32"))]
fn with_engine_cancel<T: Send>(cancel: &Cancel, run: impl FnOnce(&JobCtx) -> T + Send) -> T {
    let ctx = JobCtx::new();
    let done = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            while !done.load(Ordering::Acquire) {
                if cancel.is_cancelled() {
                    ctx.cancel();
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        });
        let out = run(&ctx);
        done.store(true, Ordering::Release);
        out
    })
}

/// Without threads the framework's `Cancel` is checked before the refinement and after it.
#[cfg(target_arch = "wasm32")]
fn with_engine_cancel<T>(_cancel: &Cancel, run: impl FnOnce(&JobCtx) -> T) -> T {
    run(&JobCtx::new())
}

/// The preview of `job`: the edits applied (from where the cache left off), the edge refined and the result shown as
/// the recipe says.
fn compute(job: Job, cancel: &Cancel) -> Result<Computed, String> {
    if cancel.is_cancelled() {
        return Err(CANCELLED.into());
    }
    let k = job.k.max(1);
    let proxy = match job.cache.as_ref().filter(|c| c.base.k == k && c.base.revision == job.revision) {
        Some(c) => Arc::clone(&c.proxy),
        None => Arc::new(crate::proxy::proxy_document(&job.doc, k)),
    };
    let mut doc = (*proxy).clone();
    let layer = job.layer.filter(|id| doc.layer(*id).is_some());
    let mut params = job.params;
    let start = || {
        let mask = layer.and_then(|id| doc.layer(id)).and_then(|l| l.mask.as_ref()).map(|m| m.surface.clone());
        if job.target_mask { mask.or_else(|| doc.selection.clone()) } else { doc.selection.clone().or(mask) }
    };
    let scaled = |list: &[Value]| -> Result<Vec<select_mask_edits::SelectionEdit>, String> {
        let list: Vec<Value> = list.iter().map(|e| scale_edit(e, k)).collect();
        select_mask_edits::parse(&json!({ "edits": list })).map_err(|e| e.to_string())
    };
    // With no edits yet, the selection Select and Mask starts from is where the first one starts.
    let mut base_cache = BaseCache { k, revision: job.revision, edits: Vec::new(), base: start() };
    if !job.edits.is_empty() {
        // The selection after the edits already applied, when they still lead the list.
        let (from, done) = match job.cache.map(|c| c.base).filter(|c| c.k == k && c.revision == job.revision && job.edits.starts_with(&c.edits)) {
            Some(c) => (c.base, c.edits.len()),
            None => (start(), 0),
        };
        let rest = scaled(job.edits.get(done..).unwrap_or_default())?;
        let base = select_mask_edits::apply(&doc, layer, from, &rest);
        base_cache = BaseCache { k, revision: job.revision, edits: job.edits.clone(), base: base.clone() };
        // Nothing selected is an empty selection, not "use the layer mask".
        doc.selection = Some(base.unwrap_or_else(|| Surface::new(photocraft_color::PixelFormat::GRAY8)));
        if let Some(o) = params.as_object_mut() {
            o.remove("target");
        }
    }
    let shown = with_engine_cancel(cancel, |ctx| crate::select_mask_view::preview_proxy(doc, layer, &params, &job.recipe, k, Some(ctx)));
    if cancel.is_cancelled() {
        return Err(CANCELLED.into());
    }
    let shown = shown.ok_or("the preview failed")?;
    Ok(Computed { shown, cache: Cache { base: base_cache, proxy } })
}

#[cfg(test)]
mod tests;
