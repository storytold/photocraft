//! Filter › Adaptive Wide Angle…: the framework's Adaptive Wide Angle workspace on the app's
//! dialog host. Dragging on the image draws a constraint (the scene line between the two points,
//! curved by the lens model); Shift on release makes it horizontal or vertical; a right-click on a
//! constraint opens its menu. The workspace owns the layout, the constraints and their history;
//! this module gives it the layer's proxy (≤ 800 px), the lens geometry, the solve-and-warp
//! preview job, and what OK runs: `filter.adaptiveWideAngle` with the constraints in document
//! pixels. Automation drives the open workspace through `control::dialogs::workspace_ui`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use craft::behavior::kurbo::Point;
use craft::imaging::ui::wide_angle::{PreviewDone, WideAngleGeometry, WideAngleOpen, WideAngleSource, WideAngleWorkspace};
use craft::imaging::wide_angle::WideAngleSession;
use craft::ui::canvas::RasterSource;
use craft::ui::dialog::HostedWorkspace;
use craft::ui::dialog::{DialogId, DialogResult};
use craft::ui::runtime::pending::Job;
use photocraft_algo::transform::Interp;
use photocraft_algo::wideangle::{self, Camera, Constraint, WideAngle, WideMesh};
use photocraft_doc::{DocId, LayerId};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::gallery_ui::SharedPicture;

const PROXY_SIDE: usize = 800;
const KEY: &str = "filter.adaptiveWideAngle";

/// The hosted Adaptive Wide Angle dialog.
pub(crate) type Hosted = HostedWorkspace<WideAngleWorkspace>;

/// The last solved warp and the session it solves, shared by the preview job and the geometry.
struct Solved {
    session: WideAngle,
    mesh: Arc<WideMesh>,
}

type SolvedSlot = Arc<Mutex<Option<Solved>>>;

/// The session scaled onto the proxy: constraints move from document pixels to proxy pixels.
fn proxy_params(session: &WideAngle, frame: Rect, scale: f64) -> WideAngle {
    let mut p = session.clone();
    let (fx, fy) = (frame.x0 as f64, frame.y0 as f64);
    p.constraints = session
        .constraints
        .iter()
        .map(|c| Constraint::new([(c.a[0] - fx) * scale, (c.a[1] - fy) * scale], [(c.b[0] - fx) * scale, (c.b[1] - fy) * scale], c.orientation))
        .collect();
    p
}

/// The corrected proxy for `params` (already in proxy pixels), warped by `mesh` or by a fresh
/// solve; the mesh comes back so the geometry can map through it.
fn preview_surface(proxy: &Surface, params: &WideAngle, pf: Rect, target: &Value, mesh: Option<Arc<WideMesh>>) -> Result<(Surface, Arc<WideMesh>), String> {
    let mesh = mesh.unwrap_or_else(|| Arc::new(wideangle::solve(params, pf)));
    let mut out = photocraft_algo::warp::warp_triangles(proxy, pf, &mesh.verts, &mesh.triangles(), Interp::Bilinear);
    if let Some(colors) =
        photocraft_engine::color_channels::explicit(&json!({"target": target}), proxy.format().mode.color_channels()).map_err(|e| e.to_string())?
    {
        photocraft_engine::color_channels::restrict_surface_in(proxy, &mut out, colors, pf).map_err(|e| e.to_string())?;
    }
    Ok((out, mesh))
}

/// The lens geometry the constraint overlays read: arcs on the source and the solved warp's map.
pub(crate) struct PcGeometry {
    /// Canvas frame (document px) and the proxy scale (proxy px per document px).
    frame: Rect,
    scale: f64,
    proxy_rect: Rect,
    solved: SolvedSlot,
}

impl WideAngleGeometry for PcGeometry {
    fn arc(&self, session: &WideAngleSession, a: [f64; 2], b: [f64; 2], segments: usize) -> Vec<Point> {
        // PLAN: assumed `Point::new(x: f64, y: f64)`.
        Camera::new(session, self.frame).arc(a, b, segments).into_iter().map(|q| Point::new(q[0], q[1])).collect()
    }

    fn corrected(&self, session: &WideAngleSession, at: Point) -> Option<Point> {
        let solved = self.solved.lock().ok()?;
        let s = solved.as_ref().filter(|s| s.session == *session)?;
        let (fx, fy, k) = (self.frame.x0 as f64, self.frame.y0 as f64, self.scale);
        // PLAN: assumed `Point` has public `x` and `y`.
        let pq = s.mesh.map(self.proxy_rect, [(at.x - fx) * k, (at.y - fy) * k]);
        Some(Point::new(pq[0] / k + fx, pq[1] / k + fy))
    }
}

/// The layer as the workspace shows it: the proxy, the corrected picture the preview job writes
/// and the lens geometry.
pub(crate) struct PcWideAngleSource {
    proxy: Arc<Surface>,
    target: Value,
    original: SharedPicture,
    corrected: SharedPicture,
    ready: Arc<AtomicBool>,
    geometry: PcGeometry,
}

impl PcWideAngleSource {
    fn new(proxy: Surface, frame: Rect, k: usize, size: (usize, usize), target: Value) -> Self {
        let (pw, ph) = size;
        let picture_size = craft::behavior::PixelSize::new(pw as u32, ph as u32);
        let original = SharedPicture::new(picture_size);
        original.replace(crate::gallery_ui::image(&proxy, pw, ph), 1, [0, 0]);
        PcWideAngleSource {
            proxy: Arc::new(proxy),
            target,
            original,
            corrected: SharedPicture::new(picture_size),
            ready: Arc::new(AtomicBool::new(false)),
            geometry: PcGeometry { frame, scale: 1.0 / k as f64, proxy_rect: Rect::new(0, 0, pw as i32, ph as i32), solved: Arc::default() },
        }
    }

    /// What the preview job runs on, cloned into it.
    fn state(&self) -> PreviewState {
        PreviewState {
            proxy: self.proxy.clone(),
            target: self.target.clone(),
            picture: self.corrected.clone(),
            ready: self.ready.clone(),
            solved: self.geometry.solved.clone(),
            frame: self.geometry.frame,
            scale: self.geometry.scale,
            pf: self.geometry.proxy_rect,
        }
    }

    /// The warp of the last finished preview job.
    #[cfg(test)]
    fn mesh(&self) -> Option<Arc<WideMesh>> {
        self.geometry.solved.lock().ok()?.as_ref().map(|s| s.mesh.clone())
    }
}

impl WideAngleSource for PcWideAngleSource {
    fn geometry(&self) -> &dyn WideAngleGeometry {
        &self.geometry
    }

    fn source(&self) -> &dyn RasterSource {
        &self.original
    }

    fn corrected(&self) -> Option<&dyn RasterSource> {
        self.ready.load(Ordering::Acquire).then_some(&self.corrected as &dyn RasterSource)
    }

    /// Solves (unless the last solve was for this very session) and warps the proxy, then writes the
    /// corrected picture. The mesh stays in the geometry for the overlays.
    fn preview_job(&self, session: &WideAngleSession) -> Job<PreviewDone, String> {
        let (state, session) = (self.state(), session.clone());
        Job::new(move |_progress, _cancel| state.run(&session))
    }
}

/// What the preview job needs of the source.
#[derive(Clone)]
struct PreviewState {
    proxy: Arc<Surface>,
    target: Value,
    picture: SharedPicture,
    ready: Arc<AtomicBool>,
    solved: SolvedSlot,
    frame: Rect,
    scale: f64,
    pf: Rect,
}

impl PreviewState {
    /// The job's body: reuses the last solve when it was for `session`, warps the proxy, writes the
    /// corrected picture and keeps the mesh for the overlays.
    fn run(&self, session: &WideAngle) -> Result<PreviewDone, String> {
        let t0 = crate::gpu_canvas::now_ms();
        let reuse = self.solved.lock().ok().and_then(|g| g.as_ref().filter(|s| s.session == *session).map(|s| s.mesh.clone()));
        let (out, mesh) = preview_surface(&self.proxy, &proxy_params(session, self.frame, self.scale), self.pf, &self.target, reuse)?;
        self.picture.replace(crate::gallery_ui::image(&out, self.pf.width() as usize, self.pf.height() as usize), 1, [0, 0]);
        if let Ok(mut slot) = self.solved.lock() {
            *slot = Some(Solved { session: session.clone(), mesh });
        }
        self.ready.store(true, Ordering::Release);
        Ok(PreviewDone::new(Duration::from_secs_f64(((crate::gpu_canvas::now_ms() - t0) / 1000.0).max(0.0))))
    }
}

/// What the app keeps of the open Adaptive Wide Angle: the dialog's id and what OK needs to run.
pub(crate) struct WideAngleApp {
    id: DialogId,
    document: DocId,
    layer: LayerId,
    target: Value,
    #[cfg(test)]
    source: Arc<PcWideAngleSource>,
}

/// The open workspace's fields for automation, with the layer it edits.
pub(crate) fn describe(app: &PhotocraftApp) -> Option<Value> {
    let w = app.wide_angle.as_ref()?;
    let mut v: Value = crate::distort_ui::workspace_fields(app, KEY).unwrap_or_else(|| json!({}));
    if let Some(o) = v.as_object_mut() {
        o.insert("document".into(), json!(w.document.0));
        o.insert("layer".into(), json!(w.layer.0));
    }
    Some(v)
}

/// Runs OK's session as `filter.adaptiveWideAngle` when the workspace finished. The constraint
/// events need no action: the workspace applied and recorded them already.
pub(crate) fn take(app: &mut PhotocraftApp, _ctx: &egui::Context) {
    let Some(id) = app.wide_angle.as_ref().map(|w| w.id) else { return };

    if let Some(h) = app.framework.dialogs.get_mut_by_id::<Hosted>(id) {
        drop(h.take_events());
    }
    let Some(result) = app.framework.dialogs.take_by_id::<Hosted>(id) else { return };
    let Some(w) = app.wide_angle.take() else { return };
    let DialogResult::Ok(session) = result else { return };
    let mut v = serde_json::to_value(&session).unwrap_or(json!({}));
    v["layer"] = json!(w.layer.0);
    if let Err(e) = app.run(KEY, crate::distort_ui::with_filter_target(v, &w.target)) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

pub fn open(app: &mut PhotocraftApp, _ctx: &egui::Context) -> Result<(), String> {
    photocraft_engine::commands::find(KEY).map(|c| (c.enabled)(&app.session)).unwrap_or(Err("unknown command".into()))?;
    let (layer, surf, _) = crate::distort_ui::active_filter_target(app)?;
    let st = app.session.active().ok_or("no document")?;
    let frame = st.doc.bounds();
    let document = st.doc.id;
    let name = st.doc.layer(layer).map(|l| l.name.clone()).unwrap_or_default();
    let (w, h) = (frame.width() as usize, frame.height() as usize);
    let k = w.max(h).div_ceil(PROXY_SIDE).max(1);
    let (pw, ph) = (w.div_ceil(k), h.div_ceil(k));
    let proxy = if k == 1 {
        surf.clone()
    } else {
        photocraft_algo::resample::resize_surface(&surf, 1.0 / k as f64, 1.0 / k as f64, photocraft_algo::resample::Resample::Bilinear)
    };
    // Normalise the proxy to start at the origin.
    let mut px = Surface::new(surf.format());
    let src_r = Rect::new(
        (frame.x0 as f64 / k as f64) as i32,
        (frame.y0 as f64 / k as f64) as i32,
        (frame.x0 as f64 / k as f64) as i32 + pw as i32,
        (frame.y0 as f64 / k as f64) as i32 + ph as i32,
    );
    px.write_region(Rect::new(0, 0, pw as i32, ph as i32), &proxy.read_region(src_r));
    // EXIF focal length / crop factor, as the engine would resolve them.
    let info = st.doc.metadata.exif.as_ref().map(|e| photocraft_algo::exif::read(e));
    let params = photocraft_engine::lens_cmds::wide_params(KEY, &json!({}), info.as_ref()).unwrap_or_default();
    let target = crate::distort_ui::colour_set_target(app);
    let source = Arc::new(PcWideAngleSource::new(px, frame, k, (pw, ph), target.clone()));
    let workspace = WideAngleWorkspace::new(WideAngleOpen::new([w as u32, h as u32], name).session(params));
    let shared: Arc<dyn WideAngleSource> = source.clone();
    let id = app.framework.dialogs.open(HostedWorkspace::new(workspace, shared));
    app.wide_angle = Some(WideAngleApp {
        id,
        document,
        layer,
        target,
        #[cfg(test)]
        source,
    });
    Ok(())
}

/// Opens the workspace for the menu command; a `{ui}` request addresses the open workspace (`workspace_ui`), and is an
/// error while it is closed rather than a run with the defaults.
pub fn menu(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    #[cfg(not(target_arch = "wasm32"))]
    let request = workspace_request;
    #[cfg(target_arch = "wasm32")]
    let request = browser_workspace_request;
    menu_with_workspace_request(app, ctx, id, params, request)
}

fn menu_with_workspace_request(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    id: &str,
    params: &Value,
    request: impl FnOnce(&mut PhotocraftApp, &egui::Context, &Value) -> Option<Result<Value, String>>,
) -> Option<Result<Value, String>> {
    if id != KEY {
        return None;
    }
    if let Some(reply) = request(app, ctx, params) {
        return Some(reply);
    }
    if params.as_object().is_none_or(|o| o.is_empty()) {
        return Some(open(app, ctx).map(|_| describe(app).unwrap_or(Value::Null)));
    }
    None
}

#[cfg(not(target_arch = "wasm32"))]
fn workspace_request(app: &mut PhotocraftApp, ctx: &egui::Context, params: &Value) -> Option<Result<Value, String>> {
    let ui = params.get("ui")?;
    Some(if app.wide_angle.is_some() { crate::control::dialogs::workspace_ui(app, ctx, KEY, ui) } else { Err("Adaptive Wide Angle is not open".into()) })
}

#[cfg(any(target_arch = "wasm32", test))]
fn browser_workspace_request(_app: &mut PhotocraftApp, _ctx: &egui::Context, params: &Value) -> Option<Result<Value, String>> {
    // A workspace control must never fall through to an engine edit with default parameters.
    params.get("ui").map(|_| Err("Workspace controls are unavailable in the browser".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_workspace_controls_are_consumed_without_editing() {
        let mut app = grey_layer();
        let ctx = app.ctx().clone();
        let document = app.session.active().unwrap().doc.clone();
        let before = app.session.active().unwrap().history.past_len();
        for ui in [Value::Null, json!({}), json!({"cancel": true}), json!({"commit": true})] {
            let reply = menu_with_workspace_request(&mut app, &ctx, KEY, &json!({"ui": ui}), browser_workspace_request);
            assert!(matches!(reply, Some(Err(ref error)) if error.contains("unavailable in the browser")), "{reply:?}");
            assert!(app.wide_angle.is_none());
            assert!(Arc::ptr_eq(&document, &app.session.active().unwrap().doc));
            assert_eq!(app.session.active().unwrap().history.past_len(), before);
        }
        assert!(menu_with_workspace_request(&mut app, &ctx, KEY, &json!({"scale": 1.0}), browser_workspace_request).is_none());
        assert!(menu_with_workspace_request(&mut app, &ctx, "unrelated", &json!({"ui": {}}), browser_workspace_request).is_none());
        assert!(menu_with_workspace_request(&mut app, &ctx, KEY, &json!({}), browser_workspace_request).unwrap().is_ok());
        assert!(app.wide_angle.is_some(), "ordinary menu invocation still opens the workspace");
        let reply = menu_with_workspace_request(&mut app, &ctx, KEY, &json!({"ui": {"cancel": true}}), browser_workspace_request);
        assert!(matches!(reply, Some(Err(_))));
        assert!(app.wide_angle.is_some(), "refused controls leave an open workspace untouched");
        assert!(Arc::ptr_eq(&document, &app.session.active().unwrap().doc));
        assert_eq!(app.session.active().unwrap().history.past_len(), before);
    }

    fn ui(app: &mut PhotocraftApp, v: Value) -> Result<Value, String> {
        let ctx = app.ctx().clone();

        crate::control::dialogs::workspace_ui(app, &ctx, KEY, &v)
    }

    /// The session the open workspace holds.
    fn session_of(app: &PhotocraftApp) -> WideAngle {
        let id = app.wide_angle.as_ref().unwrap().id;
        app.framework.dialogs.get_by_id::<Hosted>(id).unwrap().workspace().session().clone()
    }

    fn source_of(app: &PhotocraftApp) -> Arc<PcWideAngleSource> {
        app.wide_angle.as_ref().unwrap().source.clone()
    }

    /// A 160 × 100 grey layer to open the workspace on.
    fn grey_layer() -> Box<PhotocraftApp> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 160, "height": 100})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#808080"})).unwrap();
        app
    }

    /// Open the workspace with one free constraint across the middle of the layer.
    fn open_with_a_constraint(app: &mut PhotocraftApp) {
        let ctx = app.ctx().clone();
        open(app, &ctx).unwrap();
        ui(app, json!({"add": {"a": [20, 50], "b": [140, 50]}})).unwrap();
    }

    #[test]
    fn native_component_wide_angle_previews_and_commit_keep_unselected_samples() {
        for mode in ["rgb", "cmyk", "lab"] {
            for depth in [8, 16, 32] {
                for alpha in [true, false] {
                    let fixture = crate::distort_ui::component_tests::app(mode, depth);
                    let mut app = if alpha { fixture } else { crate::distort_ui::component_tests::without_alpha(fixture) };
                    let ctx = app.ctx().clone();
                    open(&mut app, &ctx).unwrap();
                    ui(&mut app, json!({"set": {"model": "fisheye", "focalLength": 10.0}})).unwrap();
                    ui(&mut app, json!({"add": {"a": [8, 8], "b": [56, 16], "orientation": "horizontal"}})).unwrap();
                    let layer = app.wide_angle.as_ref().unwrap().layer;
                    let source = source_of(&app);
                    let session = session_of(&app);
                    let before = source.proxy.as_ref().clone();
                    let pf = source.geometry.proxy_rect;
                    let (preview, _) =
                        preview_surface(&before, &proxy_params(&session, source.geometry.frame, source.geometry.scale), pf, &source.target, None).unwrap();
                    source.state().run(&session).unwrap();
                    crate::distort_ui::component_tests::protected(&before, &preview, pf);
                    assert_ne!(preview.read_region(pf), before.read_region(pf));
                    app.run("channel.target", json!({"channel":{"color":1}})).unwrap();
                    ui(&mut app, json!({"commit": true})).unwrap();
                    take(&mut app, &ctx);
                    let after = app.session.active().unwrap().doc.layer(layer).unwrap().surface().unwrap();
                    crate::distort_ui::component_tests::protected(&before, after, pf);
                    assert_ne!(after.read_region(pf), before.read_region(pf));
                }
            }
        }
    }

    #[test]
    fn dialog_adds_constraints_and_commits() {
        let mut app = grey_layer();
        let ctx = app.ctx().clone();
        menu(&mut app, &ctx, KEY, &json!({})).unwrap().unwrap();
        assert!(app.wide_angle.is_some());
        ui(&mut app, json!({"set": {"model": "fisheye", "focalLength": 10.0}})).unwrap();
        ui(&mut app, json!({"add": {"a": [20, 20], "b": [140, 20], "orientation": "horizontal"}})).unwrap();
        let session = session_of(&app);
        assert_eq!(session.constraints.len(), 1);
        let source = source_of(&app);
        let mesh = wideangle::solve(&proxy_params(&session, source.geometry.frame, source.geometry.scale), source.geometry.proxy_rect);
        assert!(mesh.residual / source.geometry.scale < 3.0, "{}", mesh.residual);
        ui(&mut app, json!({"commit": true})).unwrap();
        take(&mut app, &ctx);
        let (id, p) = app.session.journal.last().cloned().unwrap();
        assert_eq!((id.as_str(), &p["model"]), (KEY, &json!("fisheye")));
        assert!(app.wide_angle.is_none());
        menu(&mut app, &ctx, KEY, &json!({})).unwrap().unwrap();
        // A constraint without `b` is refused and changes nothing.
        assert!(ui(&mut app, json!({"add": {"a": [1, 1]}})).is_err());
        assert!(session_of(&app).constraints.is_empty());
        ui(&mut app, json!({"cancel": true})).unwrap();
        take(&mut app, &ctx);
        assert!(app.wide_angle.is_none());
    }

    #[test]
    fn constraint_overlay_reuses_the_preview_mesh_on_redraw() {
        let mut app = grey_layer();
        open_with_a_constraint(&mut app);
        let source = source_of(&app);
        let session = session_of(&app);
        assert!(source.geometry().corrected(&session, Point::new(80.0, 50.0)).is_none(), "no solve is ready before the job");
        source.state().run(&session).unwrap();
        let first = source.mesh().unwrap();
        let at = |s: &PcWideAngleSource| s.geometry().corrected(&session, Point::new(80.0, 50.0)).map(|p| (p.x, p.y)).unwrap();
        let mapped = at(&source);
        // Redraws under the unchanged session read the same mesh.
        for _ in 0..3 {
            assert_eq!(at(&source), mapped);
            assert!(Arc::ptr_eq(&first, &source.mesh().unwrap()));
        }
        // A second job for the same session warps again but does not solve again.
        source.state().run(&session).unwrap();
        assert!(Arc::ptr_eq(&first, &source.mesh().unwrap()), "the solve is reused");
    }

    #[test]
    fn preview_mesh_refreshes_after_edits_and_history_steps() {
        let mut app = grey_layer();
        open_with_a_constraint(&mut app);
        let source = source_of(&app);
        let original = session_of(&app);
        source.state().run(&original).unwrap();
        let first = source.mesh().unwrap();
        ui(&mut app, json!({"set": {"scale": 120.0}})).unwrap();
        let edited = session_of(&app);
        assert_ne!(edited, original);
        assert!(source.geometry().corrected(&edited, Point::new(80.0, 50.0)).is_none(), "the old solve is not the edited session's");
        source.state().run(&edited).unwrap();
        let second = source.mesh().unwrap();
        assert_ne!(second.verts, first.verts);
        // Undo through the workspace returns the session; the next job solves it again.
        ui(&mut app, json!({"undo": true})).unwrap();
        assert_eq!(session_of(&app), original);
        source.state().run(&original).unwrap();
        let third = source.mesh().unwrap();
        assert!(!Arc::ptr_eq(&second, &third));
        assert_eq!(third.verts, first.verts);
    }
}
