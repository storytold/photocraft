//! Filter › Vanishing Point on the hosted workspace.
//!
//! The editor, its history and its bar are the framework's [`VanishingPointWorkspace`]. This module is the app's side
//! of it: the perspective scene over `photocraft_algo::vanishing` ([`PcVanishingScene`]), the pictures the workspace
//! shows ([`PcVanishingSource`]), the clone strokes painted onto a copy of the layer ([`Preview`]), and the answer to
//! OK, which runs `filter.vanishingPoint` with the workspace's command so the whole session is one history step.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Instant;

use craft::behavior::geometry::quad::Homography;
use craft::behavior::kurbo::Point;
use craft::behavior::{EditEvent, PixelSize};
use craft::imaging::ui::vanishing_point::{
    PlaneCheck, PlaneEdge, StampLimits, StampStroke, TornPlane, VanishingPlane, VanishingPointCommand, VanishingPointEvent, VanishingPointOpen,
    VanishingPointSession, VanishingPointSettings, VanishingPointSource, VanishingPointWorkspace, VanishingPreviewRequest, VanishingScene,
};
use craft::imaging::ui::wide_angle::PreviewDone;
use craft::theme::variant::Tone;
use craft::ui::canvas::{PixelRect, RasterSource};
use craft::ui::dialog::{DialogId, HostedWorkspace};
use craft::ui::primitives::Revision;
use craft::ui::runtime::pending::Job;
use egui::{Color32, ColorImage};
use photocraft_algo::vanishing::{Edge, PlaneCheck as AlgoCheck, Scene, VpPlane, clone_dabs_from, clone_source};
use photocraft_doc::{DocId, Document, Layer, LayerContent, LayerId};
use photocraft_engine::vp_cmds::{MAX_CLONE_DABS, MAX_CLONE_STROKES, VP, copy_changed, over, paste_onto, visible_image};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::distort_ui::surface_image;

#[cfg(test)]
mod tests;

/// A picture the UI thread edits while the workspace's canvas reads it: the raster behind the paste and the stamp preview.
struct SharedPicture {
    inner: Mutex<Picture>,
}

/// The pixels, their revision and the one region the edits since `changed.0` touched.
struct Picture {
    image: ColorImage,
    revision: u64,
    changed: Option<(u64, PixelRect)>,
}

impl SharedPicture {
    fn new(image: ColorImage) -> Self {
        SharedPicture { inner: Mutex::new(Picture { image, revision: 0, changed: None }) }
    }

    /// Swaps the whole picture; a reader then reads everything.
    fn replace(&self, image: ColorImage) {
        let mut p = locked(&self.inner);
        p.image = image;
        p.revision += 1;
        p.changed = None;
    }

    /// Lets `edit` change the pixels inside `area`; a reader at the revision before the first edit since the last
    /// replace reads only the union of the edited areas.
    fn edit(&self, area: PixelRect, edit: impl FnOnce(&mut ColorImage)) {
        let mut p = locked(&self.inner);
        edit(&mut p.image);
        p.changed = Some(match p.changed {
            Some((from, r)) => (from, PixelRect::new(r.x0.min(area.x0), r.y0.min(area.y0), r.x1.max(area.x1), r.y1.max(area.y1))),
            None => (p.revision, area),
        });
        p.revision += 1;
    }
}

impl RasterSource for SharedPicture {
    fn size(&self) -> PixelSize {
        let [w, h] = locked(&self.inner).image.size;
        PixelSize::new(u32::try_from(w).unwrap_or(u32::MAX), u32::try_from(h).unwrap_or(u32::MAX))
    }

    fn revision(&self) -> Revision {
        Revision::new(locked(&self.inner).revision)
    }

    fn damage(&self, since: Revision) -> Option<PixelRect> {
        let p = locked(&self.inner);
        if since == Revision::new(p.revision) {
            return Some(PixelRect::new(0, 0, 0, 0));
        }
        match p.changed {
            Some((from, area)) if Revision::new(from) == since => Some(area),
            _ => None,
        }
    }

    fn read(&self, region: PixelRect, factor: u32, out: &mut ColorImage) {
        let p = locked(&self.inner);
        let factor = i64::from(factor.max(1));
        let [w, h] = p.image.size;
        let [out_w, out_h] = out.size;
        for oy in 0..out_h {
            for ox in 0..out_w {
                let sx = i64::from(region.x0) + ox as i64 * factor;
                let sy = i64::from(region.y0) + oy as i64 * factor;
                let px = match (usize::try_from(sx), usize::try_from(sy)) {
                    (Ok(x), Ok(y)) if x < w && y < h => p.image.pixels.get(y * w + x).copied().unwrap_or(Color32::TRANSPARENT),
                    _ => Color32::TRANSPARENT,
                };
                if let Some(dst) = out.pixels.get_mut(oy * out_w + ox) {
                    *dst = px;
                }
            }
        }
    }
}

/// The workspace as the dialog host keeps it.
type Hosted = HostedWorkspace<VanishingPointWorkspace>;

/// The app's side of an open Vanishing Point: what OK needs and what the tests read.
pub(crate) struct VpApp {
    /// The document the workspace was opened on.
    pub(crate) document: DocId,
    /// The layer it was opened on.
    pub(crate) layer: LayerId,
    /// The workspace's dialog.
    pub(crate) dialog: DialogId,
    /// What the workspace shows.
    pub(crate) source: Arc<PcVanishingSource>,
}

/// The clipboard image the paste lands as, with its content bounds.
struct Clip {
    surface: Surface,
    rect: Rect,
}

/// What the preview paints over: the document as the workspace opened, its layer and the clipboard.
struct Inputs {
    base: Arc<Document>,
    layer: LayerId,
    canvas: Rect,
    clip: Option<Clip>,
}

/// The bar's choices that change what the strokes paint on: New Layer and the paste (plane, top-left, width).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct PaintContext {
    new_layer: bool,
    paste: Option<(usize, [f64; 2], f64)>,
}

impl PaintContext {
    /// The context the workspace's OK would answer with, so the preview paints what OK will.
    fn of(workspace: &VanishingPointWorkspace, has_paste: bool) -> Self {
        let command = workspace.command(has_paste);
        PaintContext { new_layer: command.new_layer, paste: command.paste.first().map(|p| (p.plane, p.at, p.width)) }
    }
}

/// What a painted preview depends on besides the strokes: any change repaints it from scratch.
#[derive(Clone, PartialEq)]
struct Key {
    planes: Vec<VanishingPlane>,
    context: PaintContext,
}

/// The clone strokes painted onto the target layer so far.
struct StampState {
    key: Key,
    /// On a new layer: the visible image the strokes sample, with the strokes so far.
    work: Option<Surface>,
    /// The sampled image before the current stroke, retained across batches.
    source: Option<(usize, Surface)>,
    /// The target layer's pixels as painted so far.
    out: Surface,
    /// The strokes painted, the last one possibly in part.
    painted: Vec<StampStroke>,
}

/// Whether `strokes` continue what `painted` holds: the same strokes, the last of `painted` possibly shorter.
fn extends(painted: &[StampStroke], strokes: &[StampStroke]) -> bool {
    let Some((last, before)) = painted.split_last() else { return true };
    if strokes.len() < painted.len() || before != &strokes[..before.len()] {
        return false;
    }
    let next = &strokes[before.len()];
    next.source == last.source
        && next.size == last.size
        && next.hardness == last.hardness
        && next.opacity == last.opacity
        && next.points.starts_with(&last.points)
}

fn union(a: Rect, b: Rect) -> Rect {
    if a.is_empty() {
        b
    } else if b.is_empty() {
        a
    } else {
        a.union(&b)
    }
}

fn locked<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn pixel_rect(r: Rect) -> PixelRect {
    PixelRect::new(r.x0, r.y0, r.x1, r.y1)
}

/// The clone strokes painted onto a copy of the layer, shared between the UI thread (batches as the strokes come) and
/// the preview job (from scratch after an undo or a plane change). One function paints both.
struct Preview {
    inputs: Inputs,
    scene: PcVanishingScene,
    state: Mutex<Option<StampState>>,
    /// The bar's choices as the app last saw them; the job paints with them.
    latest: Mutex<PaintContext>,
    picture: OnceLock<SharedPicture>,
    shown: AtomicBool,
}

impl Preview {
    fn hide(&self, state: &mut Option<StampState>) -> bool {
        self.shown.store(false, Ordering::Release);
        state.take().is_some()
    }

    /// Paints `session`'s strokes with the context the app last saw.
    fn paint_latest(&self, session: &VanishingPointSession) -> bool {
        let context = *locked(&self.latest);
        self.paint(session, context)
    }

    /// Paints what the picture does not show yet: the strokes after the ones painted, or everything when the planes,
    /// the paste or New Layer changed or the strokes are not a continuation. True when the picture changed.
    fn paint(&self, session: &VanishingPointSession, context: PaintContext) -> bool {
        let mut guard = locked(&self.state);
        if session.strokes.is_empty() {
            return self.hide(&mut guard);
        }
        let Some(sc) = self.scene.algo(&session.planes) else { return self.hide(&mut guard) };
        let canvas = self.inputs.canvas;
        let key = Key { planes: session.planes.clone(), context };
        let stale = guard.as_ref().is_none_or(|s| s.key != key || !extends(&s.painted, &session.strokes));
        let mut area = Rect::EMPTY;
        if stale {
            let Some(layer) = self.inputs.base.layer(self.inputs.layer).and_then(Layer::surface) else { return self.hide(&mut guard) };
            let mut out = if context.new_layer { Surface::new(layer.format()) } else { layer.clone() };
            // OK pastes first and the strokes then paint over (and sample) the pasted pixels.
            if let (Some((i, at, width)), Some(c)) = (context.paste, self.inputs.clip.as_ref())
                && session.planes.get(i).is_some_and(|p| self.scene.check_plane(p) != PlaneCheck::Invalid)
            {
                let _ = paste_onto(&sc, &mut out, i, &c.surface, c.rect, at, width);
            }
            let work = context.new_layer.then(|| {
                let mut w = visible_image(&self.inputs.base);
                over(&mut w, &out);
                w
            });
            *guard = Some(StampState { key, work, source: None, out, painted: Vec::new() });
        }
        let Some(st) = guard.as_mut() else { return false };
        let first = st.painted.len().saturating_sub(1);
        let skip_first = st.painted.last().map_or(0, |c| c.points.len());
        for (si, c) in session.strokes.iter().enumerate().skip(first) {
            let skip = if si == first { skip_first } else { 0 };
            let (Some(anchor), Some(points)) = (c.points.first(), c.points.get(skip..)) else { continue };
            if points.is_empty() {
                continue;
            }
            if st.source.as_ref().is_none_or(|(stroke, _)| *stroke != si) {
                st.source = Some((si, st.work.as_ref().unwrap_or(&st.out).clone()));
            }
            let Some((_, source)) = st.source.as_ref() else { continue };
            let opacity = c.opacity as f32;
            let changed = match st.work.as_mut() {
                Some(work) => {
                    let before = work.clone();
                    let (_, a) = clone_dabs_from(&sc, work, source, c.source, *anchor, points, c.size, c.hardness, opacity, Some(canvas));
                    // As OK does: only the canvas (the dabs' boxes can span far off it).
                    let a = a.intersect(&canvas);
                    copy_changed(&mut st.out, &before, work, a);
                    a
                }
                None => clone_dabs_from(&sc, &mut st.out, source, c.source, *anchor, points, c.size, c.hardness, opacity, Some(canvas)).1,
            };
            area = union(area, changed);
        }
        for (si, c) in session.strokes.iter().enumerate().skip(first) {
            match st.painted.get_mut(si) {
                Some(done) => {
                    let have = done.points.len();
                    done.points.extend_from_slice(&c.points[have..]);
                }
                None => st.painted.push(c.clone()),
            }
        }
        let shown = st.work.as_ref().unwrap_or(&st.out);
        let changed = if stale {
            let image = surface_image(shown, canvas, usize::MAX);
            match self.picture.get() {
                Some(picture) => picture.replace(image),
                None => {
                    let _ = self.picture.set(SharedPicture::new(image));
                }
            }
            true
        } else if let (false, Some(picture)) = (area.is_empty(), self.picture.get()) {
            let sub = surface_image(shown, area, usize::MAX);
            let at = [(area.x0 - canvas.x0) as usize, (area.y0 - canvas.y0) as usize];
            picture.edit(pixel_rect(area), move |image| {
                let width = image.size[0];
                for (row, pixels) in sub.pixels.chunks(sub.size[0]).enumerate() {
                    let start = (at[1] + row) * width + at[0];
                    image.pixels[start..start + pixels.len()].copy_from_slice(pixels);
                }
            });
            true
        } else {
            false
        };
        self.shown.store(true, Ordering::Release);
        changed
    }
}

/// What the workspace reads from the app: the scene, the layer, the strokes painted on a copy of it and the clipboard.
pub(crate) struct PcVanishingSource {
    scene: PcVanishingScene,
    image: SharedPicture,
    paste: Option<SharedPicture>,
    preview: Arc<Preview>,
}

impl PcVanishingSource {
    fn new(base: Arc<Document>, layer: LayerId, surface: &Surface, canvas: Rect, clip: Option<Clip>) -> Self {
        let scene = PcVanishingScene::new(canvas);
        let paste = clip.as_ref().map(|c| SharedPicture::new(surface_image(&c.surface, c.rect, 1024)));
        let preview = Arc::new(Preview {
            inputs: Inputs { base, layer, canvas, clip },
            scene,
            state: Mutex::new(None),
            latest: Mutex::new(PaintContext::default()),
            picture: OnceLock::new(),
            shown: AtomicBool::new(false),
        });
        PcVanishingSource { scene, image: SharedPicture::new(surface_image(surface, canvas, usize::MAX)), paste, preview }
    }

    /// Whether the clipboard holds an image the paste can use.
    fn has_paste(&self) -> bool {
        self.preview.inputs.clip.is_some()
    }
}

impl VanishingPointSource for PcVanishingSource {
    fn scene(&self) -> &dyn VanishingScene {
        &self.scene
    }

    fn image(&self) -> &dyn RasterSource {
        &self.image
    }

    fn stamp_preview(&self) -> Option<&dyn RasterSource> {
        if !self.preview.shown.load(Ordering::Acquire) {
            return None;
        }
        self.preview.picture.get().map(|p| p as &dyn RasterSource)
    }

    fn paste(&self) -> Option<&dyn RasterSource> {
        self.paste.as_ref().map(|p| p as &dyn RasterSource)
    }

    fn preview_job(&self, request: &VanishingPreviewRequest) -> Job<PreviewDone, String> {
        let preview = self.preview.clone();
        let session = request.session.clone();
        Job::new(move |_, cancel| {
            let start = Instant::now();
            if cancel.is_cancelled() {
                return Err("cancelled".to_string());
            }
            preview.paint_latest(&session);
            Ok(PreviewDone::new(start.elapsed()).backend("CPU"))
        })
    }
}

/// The perspective geometry of `photocraft_algo::vanishing`, for the framework's editor.
#[derive(Clone, Copy)]
pub(crate) struct PcVanishingScene {
    center: [f64; 2],
    /// The focal length when the first plane gives none: the canvas's longer side.
    fallback: f64,
}

fn algo_plane(p: &VanishingPlane) -> VpPlane {
    VpPlane { corners: p.corners }
}

fn plane_of(p: VpPlane) -> VanishingPlane {
    VanishingPlane::new(p.corners)
}

/// The engine's edge; `None` for an edge the engine does not know (the enum is non-exhaustive).
fn algo_edge(e: PlaneEdge) -> Option<Edge> {
    match e {
        PlaneEdge::Top => Some(Edge::Top),
        PlaneEdge::Right => Some(Edge::Right),
        PlaneEdge::Bottom => Some(Edge::Bottom),
        PlaneEdge::Left => Some(Edge::Left),
        _ => None,
    }
}

impl PcVanishingScene {
    fn new(canvas: Rect) -> Self {
        PcVanishingScene {
            center: [f64::from(canvas.x0 + canvas.x1) / 2.0, f64::from(canvas.y0 + canvas.y1) / 2.0],
            fallback: f64::from(canvas.width().max(canvas.height())),
        }
    }

    /// The planes with the shared camera the engine derives from the first one.
    fn algo(&self, planes: &[VanishingPlane]) -> Option<Scene> {
        let mut scene = Scene::new(algo_plane(planes.first()?), self.center, None, self.fallback);
        scene.planes.extend(planes.iter().skip(1).map(algo_plane));
        Some(scene)
    }

    fn check_plane(&self, plane: &VanishingPlane) -> PlaneCheck {
        match algo_plane(plane).check(self.center) {
            AlgoCheck::Valid => PlaneCheck::Valid,
            AlgoCheck::Questionable => PlaneCheck::Questionable,
            AlgoCheck::Invalid => PlaneCheck::Invalid,
        }
    }

    /// Plane `i`'s frame in the algo scene, when its metric sides are usable.
    fn algo_frame(sc: &Scene, i: usize) -> Option<(photocraft_algo::transform::Homography, f64, f64)> {
        if i >= sc.planes.len() {
            return None;
        }
        let (h, lu, lv) = sc.frame(i)?;
        (lu.is_finite() && lv.is_finite() && lu > 1e-12 && lv > 1e-12).then_some((h, lu, lv))
    }
}

impl VanishingScene for PcVanishingScene {
    fn normalized(&self, plane: VanishingPlane) -> VanishingPlane {
        plane_of(algo_plane(&plane).normalized())
    }

    fn resolve(&self, planes: &[VanishingPlane], torn: &[Option<TornPlane>]) -> Vec<VanishingPlane> {
        let Some(first) = planes.first() else { return Vec::new() };
        let mut sc = Scene::new(algo_plane(first), self.center, None, self.fallback);
        for (i, plane) in planes.iter().enumerate().skip(1) {
            match torn.get(i).and_then(Option::as_ref).filter(|t| t.from < i) {
                Some(t) => {
                    if algo_edge(t.edge).and_then(|e| sc.tear_off(t.from, e, t.angle, t.depth)).is_none() {
                        sc.planes.push(VpPlane { corners: [[0.0; 2]; 4] });
                    }
                }
                None => sc.planes.push(algo_plane(plane)),
            }
        }
        sc.planes.into_iter().map(plane_of).collect()
    }

    fn check(&self, planes: &[VanishingPlane], index: usize) -> PlaneCheck {
        planes.get(index).map_or(PlaneCheck::Invalid, |p| self.check_plane(p))
    }

    fn plane_at(&self, planes: &[VanishingPlane], at: [f64; 2]) -> Option<usize> {
        self.algo(planes)?.plane_at(at)
    }

    fn frame(&self, planes: &[VanishingPlane], index: usize) -> Option<(Homography, f64, f64)> {
        let (h, lu, lv) = Self::algo_frame(&self.algo(planes)?, index)?;
        let m = h.0;
        Some((Homography::from_rows([[m[0], m[1], m[2]], [m[3], m[4], m[5]], [m[6], m[7], m[8]]]), lu, lv))
    }

    fn stamp_ring(&self, planes: &[VanishingPlane], at: [f64; 2], diameter: f64) -> Option<Vec<Point>> {
        let sc = self.algo(planes)?;
        let i = sc.plane_at(at)?;
        let (h, lu, lv) = Self::algo_frame(&sc, i)?;
        let inv = h.inverse()?;
        let metric = |q: [f64; 2]| {
            let (u, v) = inv.apply(q[0], q[1]);
            [u * lu, v * lv]
        };
        let (m, probe) = (metric(at), metric([at[0] + diameter / 2.0, at[1]]));
        let r = (probe[0] - m[0]).hypot(probe[1] - m[1]);
        if !r.is_finite() || r <= 0.0 {
            return None;
        }
        let ring: Vec<Point> = (0..48)
            .map(|k| {
                let a = f64::from(k) / 48.0 * std::f64::consts::TAU;
                let (x, y) = h.apply((m[0] + r * a.cos()) / lu, (m[1] + r * a.sin()) / lv);
                Point::new(x, y)
            })
            .collect();
        ring.iter().all(|q| q.x.is_finite() && q.y.is_finite()).then_some(ring)
    }

    fn clone_source(&self, planes: &[VanishingPlane], source: [f64; 2], anchor: [f64; 2], at: [f64; 2]) -> Option<[f64; 2]> {
        clone_source(&self.algo(planes)?, source, anchor, at)
    }
}

/// Starts Vanishing Point on the active pixel layer.
pub fn begin(app: &mut PhotocraftApp, _ctx: &egui::Context) -> Result<(), String> {
    if app.ui.transform.is_some() {
        return Err("finish Free Transform first".into());
    }
    let st = app.session.active().ok_or("no document")?;
    let layer = st.active_layer.ok_or("no active layer")?;
    let (name, surface) = match st.doc.layer(layer) {
        Some(l) => match &l.content {
            LayerContent::Raster(s) => (l.name.clone(), s.clone()),
            other => return Err(format!("Vanishing Point needs a pixel layer (active layer is {} {} layer)", other.article(), other.kind_name())),
        },
        None => return Err("no active layer".into()),
    };
    let canvas = st.doc.bounds();
    let (base, document) = (st.doc.clone(), st.doc.id);
    let clip = app.session.clipboard.as_ref().and_then(|c| {
        let r = c.surface.content_bounds();
        (!r.is_empty()).then(|| Clip { surface: c.surface.clone(), rect: r })
    });
    let source = Arc::new(PcVanishingSource::new(base, layer, &surface, canvas, clip));
    let mut settings = VanishingPointSettings::default();
    settings.paste = source.has_paste();

    let open = VanishingPointOpen::new([canvas.width(), canvas.height()], name, StampLimits::new(MAX_CLONE_STROKES, MAX_CLONE_DABS))
        .settings(settings)
        .navigation(crate::canvas::document_display::navigation(app))
        .checkerboard(crate::canvas::document_display::checkerboard(app).unwrap_or_default());
    let dynamic: Arc<dyn VanishingPointSource> = source.clone();
    let dialog = app.framework.dialogs.open(HostedWorkspace::new(VanishingPointWorkspace::new(open), dynamic).held_ok());
    app.vanishing = Some(VpApp { document, layer, dialog, source });
    app.ui.status = tl!("Vanishing Point: click four corners to create a plane").into();
    Ok(())
}

/// Whether an edit event belongs to a gesture still running: its result is painted when the gesture ends.
fn in_flight<T>(event: &EditEvent<T>) -> bool {
    matches!(event, EditEvent::Begin { .. } | EditEvent::Update { .. })
}

/// Runs OK's command: `filter.vanishingPoint` with the planes, the paste and the strokes, on the layer the workspace
/// opened on. With nothing to paste or clone there is nothing to run.
fn apply(app: &mut PhotocraftApp, layer: LayerId, command: &VanishingPointCommand) -> Result<(), String> {
    if command.paste.is_empty() && command.clone.is_empty() {
        return Ok(());
    }
    let mut params = serde_json::to_value(command).map_err(|e| e.to_string())?;
    // The layer is named only when another one became active meanwhile, so a recorded action stays portable.
    if let Some(st) = app.session.active()
        && st.active_layer != Some(layer)
        && st.doc.layer(layer).is_some()
    {
        params["layer"] = json!(layer.0);
    }
    app.run(VP, params).map(|_| ())
}

/// The workspace's pump (a `TAKERS` row): paints the strokes the workspace reported, shows its refusals, answers a held
/// OK, and drops the app's side once the workspace closed.
pub(crate) fn take(app: &mut PhotocraftApp, _ctx: &egui::Context) {
    let Some(vp) = app.vanishing.as_ref() else { return };
    let (id, layer, source) = (vp.dialog, vp.layer, vp.source.clone());
    let Some(hosted) = app.framework.dialogs.get_mut_by_id::<Hosted>(id) else {
        // Closed: OK ran when it was held, so the finished answer carries nothing more.
        let _ = app.framework.dialogs.take_by_id::<Hosted>(id);
        app.vanishing = None;
        return;
    };
    let events = hosted.take_events();
    let ok = hosted.take_ok();
    let (mut paint, mut refresh, mut refused) = (false, false, Vec::new());
    for event in events {
        match event {
            VanishingPointEvent::Refused(message) => refused.push(message),
            VanishingPointEvent::Settings(_) => (paint, refresh) = (true, true),
            VanishingPointEvent::Planes(e) if !in_flight(&e) => (paint, refresh) = (true, true),
            VanishingPointEvent::Paste(e) if !in_flight(&e) => (paint, refresh) = (true, true),
            VanishingPointEvent::Stamp(_) => paint = true,
            _ => {}
        }
    }
    if paint {
        if refresh {
            *locked(&source.preview.latest) = PaintContext::of(hosted.workspace(), source.has_paste());
        }
        source.preview.paint_latest(hosted.workspace().session());
    }
    for message in refused {
        app.ui.status = message.text.as_str().to_string();
        app.ui.status_error = message.tone == Tone::Bad;
    }
    if let Some(command) = ok {
        let answer = apply(app, layer, &command);
        if let Err(e) = &answer {
            // The planes, source and strokes stay so the user can fix the cause and retry.
            app.ui.status = e.clone();
            app.ui.status_error = true;
        }
        if let Some(hosted) = app.framework.dialogs.get_mut_by_id::<Hosted>(id) {
            hosted.answer_ok(answer.map_err(Into::into));
        }
    }
}

/// The open workspace's command fields for the automation surface, with the document and layer it works on.
pub(crate) fn describe(app: &PhotocraftApp) -> Option<Value> {
    let vp = app.vanishing.as_ref()?;
    let hosted = app.framework.dialogs.get_by_id::<Hosted>(vp.dialog)?;
    let mut fields: Value = serde_json::to_value(hosted.workspace().command(vp.source.has_paste())).ok()?;
    if let Some(object) = fields.as_object_mut() {
        object.insert("document".into(), json!(vp.document.0));
        object.insert("layer".into(), json!(vp.layer.0));
    }
    Some(fields)
}
