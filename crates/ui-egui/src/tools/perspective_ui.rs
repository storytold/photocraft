//! Edit › Perspective Warp on the canvas.
//!
//! **Layout** mode: drag on the image to draw a plane, drag its corners onto the image's perspective lines; a
//! corner dropped near another plane's corner snaps to it, which links the planes along that edge. **Warp** mode:
//! drag corners (linked corners move together) and the image follows; the options bar straightens near-vertical /
//! near-horizontal edges. The framework's `PerspectiveWarp` takes the pointer and keys in the canvas body,
//! `PerspectiveBar` is the options bar, and `PerspectiveSession` holds the planes, the mode and the session's undo
//! steps. ↩ / Commit runs `edit.perspectiveWarp` with the planes. The preview is the layer texture on a grid pushed
//! through the same piecewise-homography map the engine uses.
//!
//! Every change of the session, from the canvas, the options bar or the control channel, is made on a copy and kept
//! only if the engine's map accepts the planes it leads to ([`accept`]); a refusal is an error, never a silent skip.

use std::sync::Arc;

use craft::behavior::EditEvent;
use craft::behavior::kurbo::{Point, Rect as DocRect};
use craft::imaging::ui::transform_preview::{PreviewPixels, PreviewTextures};
use craft::imaging::ui::warp::perspective::{PerspectiveBar, PerspectiveEdit, PerspectiveSession, PerspectiveWarp, Plane, PlaneMode, Straighten};
use craft::imaging::ui::{PreviewMap, PreviewQuality, TransformPreview};
use craft::ui::contract::{Bind, Show as _};
use photocraft_algo::perspective::{PerspectiveMap, Plane as AlgoPlane, Straighten as AlgoStraighten};
use photocraft_doc::{DocId, Document, LayerId};
use photocraft_geom::Rect;
use serde_json::{Value, json};

use crate::PhotocraftApp;

fn point(v: [f64; 2]) -> Point {
    Point::new(v[0], v[1])
}

fn xy(p: Point) -> [f64; 2] {
    [p.x, p.y]
}

/// The engine's planes for the framework's: `source` is `src`, `target` is `dst`, corners clockwise from the top left
/// in both.
fn to_algo(planes: &[Plane]) -> Vec<AlgoPlane> {
    planes.iter().map(|p| AlgoPlane { src: p.source.map(xy), dst: p.target.map(xy) }).collect()
}

fn from_algo(planes: &[AlgoPlane]) -> Vec<Plane> {
    planes.iter().map(|p| Plane::new(p.src.map(point), p.dst.map(point))).collect()
}

/// The framework's straightening for the engine's: the engine's `Auto` is Both.
fn straighten_of(how: AlgoStraighten) -> Straighten {
    match how {
        AlgoStraighten::Horizontal => Straighten::Horizontal,
        AlgoStraighten::Vertical => Straighten::Vertical,
        AlgoStraighten::Auto => Straighten::Both,
    }
}

/// An open Perspective Warp.
pub struct PerspSession {
    /// Names the session's textures.
    pub key: u64,
    /// Document this session belongs to (#5115).
    pub document: DocId,
    /// The layer being warped.
    pub layer: LayerId,
    /// Command to commit with (`edit.perspectiveWarp` or `layer.smartObjects.perspectiveWarp`).
    command: String,
    /// The document without the layer: what the canvas shows under the preview.
    pub preview_doc: Arc<Document>,
    /// The layer's opacity times its fill.
    opacity: f32,
    /// The layer's content bounds: the preview's source frame.
    bounds: Rect,
    /// The planes, the mode and the undo steps.
    pub warp: PerspectiveSession,
    textures: PreviewTextures,
}

impl PerspSession {
    /// The session's state for `ui.inspect` and the control channel.
    pub fn describe(&self) -> Value {
        let mode = if self.warp.mode() == PlaneMode::Warp { "warp" } else { "layout" };
        let planes = to_algo(self.warp.planes());
        let linked = photocraft_algo::perspective::linked_edge_count(&planes);
        json!({"document": self.document.0, "layer": self.layer.0, "mode": mode, "planes": planes, "linkedEdges": linked})
    }
}

fn doc_rect(b: Rect) -> DocRect {
    DocRect::new(f64::from(b.x0), f64::from(b.y0), f64::from(b.x1), f64::from(b.y1))
}

/// Starts Perspective Warp on the active layer.
pub fn begin(app: &mut PhotocraftApp, _ctx: &egui::Context, command: &str) -> Result<(), String> {
    if let Some(why) = app.session.disabled_reason(command) {
        return Err(why);
    }
    if app.ui.transform.is_some() {
        return Err("finish Free Transform first".into());
    }
    let (layer, surface, _) = crate::distort_ui::active_pixels(app)?;
    let st = app.session.active().ok_or("no document")?;
    let l = st.doc.layer(layer).ok_or("no layer")?;
    let bounds = surface.content_bounds();
    if bounds.is_empty() {
        return Err("Could not use Perspective Warp: the layer is empty".into());
    }
    let opacity = l.opacity * l.fill_opacity;
    let preview_doc = crate::distort_ui::without_layer(&st.doc, layer);
    let image = crate::distort_ui::surface_image(&surface, bounds, 2048);
    let document = st.doc.id;
    let key = app.ui.alloc_id();
    let textures = PreviewTextures::new(&mut app.texture_serials, format!("persp-{key}"), PreviewPixels::new(image, [1.0, 1.0]));
    app.distort.perspective = Some(PerspSession {
        key,
        document,
        layer,
        command: command.to_string(),
        preview_doc,
        opacity,
        bounds,
        warp: PerspectiveSession::new(doc_rect(bounds)),
        textures,
    });
    app.ui.status = tl!("Perspective Warp: draw planes along the image's perspective, then switch to Warp").into();
    Ok(())
}

/// Whether `planes` can be shown and committed: none, or planes the engine's map accepts (finite corners, convex
/// quads, after linked corners are unified). The engine refuses the rest, so the session never holds them.
fn check(planes: &[Plane]) -> Result<(), String> {
    if planes.iter().flat_map(|p| p.source.iter().chain(&p.target)).any(|c| !c.x.is_finite() || !c.y.is_finite()) {
        return Err(tl!("Perspective Warp: plane corners must be finite").to_string());
    }
    if !planes.is_empty() && PerspectiveMap::new(&to_algo(planes)).is_none() {
        return Err(tl!("Perspective Warp: that would make a plane degenerate or not convex").to_string());
    }
    Ok(())
}

/// Applies `edit` to `session` only if the planes it leads to are valid. The edit runs on a copy, so a refusal leaves
/// the planes, the mode, the undo steps and the generation as they were. A refused edit the session would silently
/// skip (a mode or straighten change during a drag, Warp with no planes) is an error too.
///
/// A refused `Update` keeps the last accepted planes. A refused final `Commit` still ends the drag, on the last
/// accepted planes and with its own id, so no drag is left running.
fn accept(session: &mut PerspectiveSession, edit: PerspectiveEdit) -> Result<(), String> {
    match &edit {
        PerspectiveEdit::Mode(_) | PerspectiveEdit::Straighten(_) if session.dragging() => {
            return Err(tl!("Perspective Warp: finish the corner drag first").to_string());
        }
        PerspectiveEdit::Mode(PlaneMode::Warp) if session.planes().is_empty() => {
            return Err(tl!("Perspective Warp: add a plane before switching to Warp").to_string());
        }
        PerspectiveEdit::Straighten(_) if session.planes().is_empty() => {
            return Err(tl!("Perspective Warp: there are no planes to straighten").to_string());
        }
        PerspectiveEdit::Planes(EditEvent::Set { .. }) if session.dragging() => {
            return Err(tl!("Perspective Warp: finish the corner drag first").to_string());
        }
        _ => {}
    }
    let mut trial = session.clone();
    trial.apply(edit.clone());
    match check(trial.planes()) {
        Ok(()) => {
            *session = trial;
            Ok(())
        }
        Err(why) => {
            if let PerspectiveEdit::Planes(mut event) = edit
                && let EditEvent::Commit { after, .. } = &mut event
                && session.dragging()
            {
                *after = session.planes().to_vec();
                session.apply(PerspectiveEdit::Planes(event));
            }
            Err(why)
        }
    }
}

/// The open session, if it belongs to the active document.
fn current(app: &PhotocraftApp) -> Result<&PerspSession, String> {
    let s = app.distort.perspective.as_ref().ok_or_else(|| tl!("Perspective Warp is not active").to_string())?;
    if app.session.active().map(|st| st.doc.id) != Some(s.document) {
        return Err(tl!("Perspective Warp belongs to another document").to_string());
    }
    Ok(s)
}

/// One edit of the open session, checked as [`accept`] does; the session is left alone when it is refused or belongs
/// to another document.
pub(crate) fn session_apply(app: &mut PhotocraftApp, edit: PerspectiveEdit) -> Result<(), String> {
    current(app)?;
    match app.distort.perspective.as_mut() {
        Some(s) => accept(&mut s.warp, edit),
        None => Err(tl!("Perspective Warp is not active").to_string()),
    }
}

/// The edits the canvas or the options bar reported, each through [`session_apply`]; a refusal is shown in the status
/// bar.
fn apply_reported(app: &mut PhotocraftApp, edits: Vec<PerspectiveEdit>) {
    for edit in edits {
        if let Err(error) = session_apply(app, edit) {
            crate::dialogs::alerts::report_error(app, error);
        }
    }
}

/// Commits `warp`'s planes as one `edit.perspectiveWarp` step on the document the session belongs to. The planes are
/// checked before anything else, so an invalid state is never taken for an identity. No planes, or planes that all
/// map onto themselves, run no command and close the session (`committed: false`). If the engine refuses, the
/// session and its preview stay open.
fn commit_planes(app: &mut PhotocraftApp, warp: &PerspectiveSession) -> Result<Value, String> {
    let s = current(app)?;
    check(warp.planes())?;
    let Some(planes) = warp.result() else {
        let reason = if warp.planes().is_empty() { "noPlanes" } else { "identity" };
        app.distort.perspective = None;
        return Ok(json!({"committed": false, "reason": reason}));
    };
    let (command, params) = (s.command.clone(), json!({"planes": to_algo(planes), "layer": s.layer.0}));
    app.run(&command, params)?;
    app.distort.perspective = None;
    Ok(json!({"committed": true}))
}

/// Commits the open session.
pub fn commit(app: &mut PhotocraftApp) -> Result<Value, String> {
    let warp = current(app)?.warp.clone();
    commit_planes(app, &warp)
}

/// Enter or the options bar's commit: the same commit, a refusal shown in the status bar.
fn commit_from_ui(app: &mut PhotocraftApp) {
    if let Err(error) = commit(app) {
        crate::dialogs::alerts::report_error(app, error);
    }
}

/// Control channel: `edit.perspectiveWarp {"ui": {...}}` while the mode is active. Every field is read and the
/// result checked on a copy of the session before the session changes, so a refused request changes nothing. The
/// fields apply in the order planes, mode, straighten, linkEdges, then commit.
pub fn control(app: &mut PhotocraftApp, ui: &Value) -> Result<Value, String> {
    let field = |key: &str| ui.get(key).filter(|v| !v.is_null());
    let flag = |key: &str| -> Result<bool, String> { field(key).map_or(Ok(false), |v| v.as_bool().ok_or_else(|| format!("{key}: true|false"))) };
    let (commit_requested, cancel_requested) = (flag("commit")?, flag("cancel")?);
    let mode = field("mode")
        .map(|v| match v.as_str() {
            Some("warp") => Ok(PlaneMode::Warp),
            Some("layout") => Ok(PlaneMode::Layout),
            _ => Err("mode: warp|layout".to_string()),
        })
        .transpose()?;
    let planes = field("planes").map(|p| serde_json::from_value::<Vec<AlgoPlane>>(p.clone()).map_err(|e| format!("bad planes: {e}"))).transpose()?;
    if let Some(planes) = &planes {
        check(&from_algo(planes))?;
    }
    let straighten = field("straighten")
        .map(|v| v.as_str().and_then(AlgoStraighten::parse).ok_or_else(|| "straighten: horizontal|vertical|auto".to_string()))
        .transpose()?;
    // Keep the headless edge-link operation without reintroducing a Link Edges button.
    // None = omitted; Some(None) = false; Some(Some(tolerance)) = requested.
    let link_edges: Option<Option<f64>> = field("linkEdges")
        .map(|value| {
            let tolerance = match value {
                Value::Bool(false) => return Ok(None),
                Value::Bool(true) => 8.0,
                Value::Object(obj) if obj.len() == 1 => obj.get("tolerance").and_then(Value::as_f64).ok_or("linkEdges tolerance must be a number")?,
                _ => return Err("linkEdges: true|false|{tolerance:number}"),
            };
            if !tolerance.is_finite() || !(0.0..=32.0).contains(&tolerance) {
                return Err("linkEdges tolerance must be finite and within 0..32 document pixels");
            }
            Ok(Some(tolerance))
        })
        .transpose()
        .map_err(str::to_string)?;
    let edits = mode.is_some() || planes.is_some() || straighten.is_some() || link_edges.is_some();
    if commit_requested && cancel_requested {
        return Err("commit and cancel cannot be combined".into());
    }
    if cancel_requested {
        if edits {
            return Err("cancel cannot be combined with mode, planes, straighten or linkEdges".into());
        }
        current(app)?;
        app.distort.perspective = None;
        return Ok(json!({"cancelled": true}));
    }
    let mut trial = current(app)?.warp.clone();
    if let Some(planes) = planes {
        // Keep the original sources: PerspectiveMap groups and averages them once. Pre-unifying here can
        // move one component's mean within linking distance of an unrelated component on the next map build.
        // The session ends a corner drag it holds indices of (#952) and bumps its generation.
        trial.replace(from_algo(&planes));
        check(trial.planes())?;
    }
    if let Some(mode) = mode {
        accept(&mut trial, PerspectiveEdit::Mode(mode))?;
    }
    if let Some(how) = straighten {
        accept(&mut trial, PerspectiveEdit::Straighten(straighten_of(how)))?;
    }
    let mut welded_edges = None;
    if let Some(Some(tolerance)) = link_edges {
        if trial.mode() != PlaneMode::Layout {
            return Err("linkEdges requires Layout mode".into());
        }
        if trial.dragging() {
            return Err("finish the corner drag before linking edges".into());
        }
        let mut planes = to_algo(trial.planes());
        let welded = photocraft_algo::perspective::weld_adjacent_edges(&mut planes, tolerance).map_err(str::to_string)?;
        if welded > 0 {
            check(&from_algo(&planes))?;
            trial.replace(from_algo(&planes));
            check(trial.planes())?;
        }
        welded_edges = Some(welded);
    }
    if commit_requested {
        let mut result = commit_planes(app, &trial)?;
        if let Some(welded) = welded_edges {
            result["weldedEdges"] = json!(welded);
        }
        return Ok(result);
    }
    let Some(s) = app.distort.perspective.as_mut() else { return Err(tl!("Perspective Warp is not active").to_string()) };
    s.warp = trial;
    let mut result = s.describe();
    if let Some(welded) = welded_edges {
        result["weldedEdges"] = json!(welded);
    }
    Ok(result)
}

/// The warped pixels and the planes, in the canvas body at `at`. Each pass applies only the edits its own pointer and
/// key input produced, so a second pass in one frame applies nothing the first did; Enter commits and Escape with no
/// drag cancels.
pub fn canvas(app: &mut PhotocraftApp, ui: &mut egui::Ui, at: egui::Rect) {
    let Some(size) = app.session.active().map(|st| [st.doc.size.width, st.doc.size.height]) else { return };
    let Some(s) = app.distort.perspective.as_ref() else { return };
    // Layout leaves the picture where it is; Warp pushes it through the engine's map of the planes.
    let algo = to_algo(s.warp.planes());
    let map = (s.warp.mode() == PlaneMode::Warp).then(|| PerspectiveMap::new(&algo)).flatten();
    let through = |p: Point| match &map {
        Some(m) => {
            let (x, y) = m.map(p.x, p.y);
            Point::new(x, y)
        }
        None => p,
    };
    TransformPreview::new("Moving layer", &s.textures)
        .map(PreviewMap::function(doc_rect(s.bounds), PreviewQuality::PLANE_CELLS, &through))
        .opacity(s.opacity)
        .clip(size)
        .show_at(ui, at);
    let output = PerspectiveWarp::new("Planes", Bind::given(s.warp.planes().to_vec()), s.warp.mode(), s.warp.generation()).show_at(ui, at);
    apply_reported(app, output.edits());
    if output.apply {
        commit_from_ui(app);
    } else if output.discard {
        app.distort.perspective = None;
    }
}

/// What the options bar shows of the open session: the mode and the plane count. A plane drawn on the canvas changes
/// them without a command, so the options bar's content version holds them.
pub(crate) fn bar_key(app: &PhotocraftApp) -> Option<(PlaneMode, usize)> {
    app.distort.perspective.as_ref().map(|s| (s.warp.mode(), s.warp.planes().len()))
}

/// The options bar: the Layout and Warp tabs, the plane count and Remove Planes or the straighten buttons, commit and
/// cancel.
pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let Some(s) = app.distort.perspective.as_ref() else { return };
    let output = PerspectiveBar::new(&s.warp).show(ui);
    apply_reported(app, output.edits);
    if output.apply {
        commit_from_ui(app);
    } else if output.discard {
        app.distort.perspective = None;
    }
}

#[cfg(test)]
mod tests {
    use craft::behavior::{GestureId, Mixed};

    use super::*;

    fn perspective_app() -> Box<PhotocraftApp> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width": 200, "height": 120, "depth": 8})).unwrap();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        app.session
            .edit("paint", |doc, active| {
                doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(10, 10, 190, 110), &[0.2, 0.6, 0.9, 1.0]);
                Ok(())
            })
            .unwrap();
        app.sync_views();
        let ctx = app.ctx().clone();
        crate::distort_ui::menu(&mut app, &ctx, "edit.perspectiveWarp", &json!({})).unwrap().unwrap();
        app
    }

    fn past_len(app: &PhotocraftApp) -> usize {
        app.session.active().unwrap().history.past_len()
    }

    const TWO_PLANES: fn() -> Value = || {
        let left = json!([[20.0, 20.0], [100.0, 20.0], [100.0, 100.0], [20.0, 100.0]]);
        let right = json!([[100.0, 20.0], [180.0, 20.0], [180.0, 100.0], [100.0, 100.0]]);
        json!([{"src": left, "dst": left}, {"src": right, "dst": right}])
    };

    #[test]
    fn linked_planes_warp_together_and_commit_in_clockwise_order() {
        let mut app = perspective_app();
        control(&mut app, &json!({"planes": TWO_PLANES()})).unwrap();
        control(&mut app, &json!({"mode": "warp"})).unwrap();
        let s = app.distort.perspective.as_ref().unwrap();
        assert_eq!(s.warp.mode(), PlaneMode::Warp);
        // Dragging the shared top corner moves both planes: the session edit the canvas's corner drag reports.
        let before = s.warp.planes().to_vec();
        let mut after = before.clone();
        after[0].target[1] = Point::new(100.0, 5.0);
        after[1].target[0] = Point::new(100.0, 5.0);
        session_apply(&mut app, PerspectiveEdit::replace(&before, after, "Move Corner")).unwrap();
        let planes = app.distort.perspective.as_ref().unwrap().describe()["planes"].clone();
        assert_eq!(planes[0]["dst"][1], json!([100.0, 5.0]));
        assert_eq!(planes[1]["dst"][0], json!([100.0, 5.0]));
        assert_eq!(planes[0]["src"][1], json!([100.0, 20.0]), "warp mode keeps the layout");
        assert_eq!(planes[1]["src"][3], json!([100.0, 100.0]), "corners run clockwise from the top left");
        let past = past_len(&app);
        assert_eq!(commit(&mut app).unwrap()["committed"], true);
        assert!(app.distort.perspective.is_none());
        assert_eq!(past_len(&app), past + 1, "one document step");
    }

    #[test]
    fn warp_replacement_preserves_original_components_for_preview_and_commit() {
        let mut app = perspective_app();
        control(&mut app, &json!({"planes": one_plane(), "mode": "warp"})).unwrap();
        let square = |x, y| AlgoPlane::identity([[x, y], [x + 100.0, y], [x + 100.0, y + 100.0], [x, y + 100.0]]);
        let mut original = vec![square(0.0, 0.0), square(0.5, 0.5), square(-0.01, 0.51)];
        original[2].dst = original[2].src.map(|[x, y]| [x, y + 10.0]);
        let groups = photocraft_algo::perspective::linked_corners(&original);
        assert_eq!(groups.len(), 8, "the third square is separate from the first pair");
        let past = past_len(&app);
        let reply = control(&mut app, &json!({"planes": original})).unwrap();
        assert_eq!(reply["planes"], json!(original), "the control reply retains the original constraints");
        let stored = to_algo(warp_of(&app).planes());
        assert_eq!(stored, original, "replacement must not pre-unify source components");
        assert_eq!(photocraft_algo::perspective::linked_corners(&stored), groups);
        let map = PerspectiveMap::new(&stored).unwrap();
        // These are the exact stored handles of the separate plane, with no normalization in the assertion.
        for (src, dst) in stored[2].src.iter().zip(&stored[2].dst) {
            let (x, y) = map.map(src[0], src[1]);
            assert!((x - dst[0]).hypot(y - dst[1]) < 1e-9, "stored handle {src:?} -> {dst:?}, map returned ({x}, {y})");
        }
        // The first pair shares mean handles. Do not feed these normalized sources back into the map.
        let mut normalized = original.clone();
        photocraft_algo::perspective::unify(&mut normalized);
        for plane in &normalized {
            for (src, dst) in plane.src.iter().zip(&plane.dst) {
                let (x, y) = map.map(src[0], src[1]);
                assert!((x - dst[0]).hypot(y - dst[1]) < 1e-9, "original component handle {src:?} -> {dst:?}");
            }
        }
        assert_eq!(to_algo(warp_of(&app).result().unwrap()), original, "commit receives the same sources as the preview");
        assert_eq!(past_len(&app), past, "replacing the preview does not change the document history");
        assert!(app.distort.perspective.as_mut().unwrap().warp.undo());
        assert!(app.distort.perspective.as_mut().unwrap().warp.redo());
        assert_eq!(to_algo(warp_of(&app).planes()), original, "session history retains the original components");
        assert_eq!(commit(&mut app).unwrap()["committed"], true);
        assert!(app.distort.perspective.is_none());
        assert_eq!(past_len(&app), past + 1, "the replacement commits as one document step");
    }

    #[test]
    fn planes_that_map_onto_themselves_commit_nothing() {
        let mut app = perspective_app();
        control(&mut app, &json!({"planes": TWO_PLANES()})).unwrap();
        let past = past_len(&app);
        assert_eq!(commit(&mut app).unwrap()["committed"], false);
        assert!(app.distort.perspective.is_none());
        assert_eq!(past_len(&app), past);
        let mut empty = perspective_app();
        assert_eq!(control(&mut empty, &json!({"commit": true})).unwrap()["committed"], false, "no planes: nothing committed, and it says so");
    }

    fn warp_of(app: &PhotocraftApp) -> &PerspectiveSession {
        &app.distort.perspective.as_ref().unwrap().warp
    }

    fn planes_of(app: &PhotocraftApp) -> Vec<Plane> {
        warp_of(app).planes().to_vec()
    }

    fn one_plane() -> Value {
        json!([TWO_PLANES()[0].clone()])
    }

    /// `planes` with the first plane's right corners swapped, in the layout and in the warp: a self-crossing quad.
    fn bowtie(planes: &[Plane]) -> Vec<Plane> {
        let mut crossed = planes.to_vec();
        crossed[0].source.swap(1, 2);
        crossed[0].target.swap(1, 2);
        crossed
    }

    #[test]
    fn a_refused_proposal_changes_neither_the_session_nor_its_undo_steps() {
        let mut app = perspective_app();
        control(&mut app, &json!({"planes": one_plane()})).unwrap();
        let (planes, generation) = (planes_of(&app), warp_of(&app).generation());
        let crossed = bowtie(&planes);
        let error = session_apply(&mut app, PerspectiveEdit::replace(&planes, crossed, "Move Corner")).unwrap_err();
        assert!(!error.is_empty());
        assert_eq!(planes_of(&app), planes);
        assert_eq!(warp_of(&app).generation(), generation);
        assert!(warp_of(&app).can_undo(), "the accepted plane is still one step");
        assert!(app.distort.perspective.as_mut().unwrap().warp.undo());
        assert!(planes_of(&app).is_empty(), "the refused edit left no step behind it");
        assert!(!warp_of(&app).can_undo());
    }

    #[test]
    fn a_refused_straighten_or_mode_is_an_error_and_changes_nothing() {
        let mut app = perspective_app();
        assert!(session_apply(&mut app, PerspectiveEdit::Straighten(Straighten::Both)).is_err(), "no planes to straighten");
        assert!(session_apply(&mut app, PerspectiveEdit::Mode(PlaneMode::Warp)).is_err(), "Warp needs a plane");
        assert!(control(&mut app, &json!({"mode": "warp"})).is_err());
        assert!(control(&mut app, &json!({"straighten": "auto"})).is_err());
        assert_eq!(warp_of(&app).mode(), PlaneMode::Layout);
        assert!(!warp_of(&app).can_undo());
        control(&mut app, &json!({"planes": TWO_PLANES()})).unwrap();
        let (planes, generation) = (planes_of(&app), warp_of(&app).generation());
        // Corners 0.4 apart in the layout are linked, and linking averages their targets: here that pulls the second
        // plane's top left past its top right and flips it, so the replacement is refused whole, mode included.
        let mut linked: Vec<AlgoPlane> = serde_json::from_value(TWO_PLANES()).unwrap();
        linked[1].src[0] = [100.4, 20.0];
        linked[1].src[3] = [100.4, 100.0];
        linked[1].dst = [[-100.0, 20.0], [-20.0, 20.0], [180.4, 100.0], [100.4, 100.0]];
        for plane in &linked {
            assert!(PerspectiveMap::new(std::slice::from_ref(plane)).is_some(), "each input is convex before linking");
        }
        assert!(PerspectiveMap::new(&linked).is_none(), "averaging the linked targets makes the destination invalid");
        assert!(control(&mut app, &json!({"planes": linked, "mode": "warp"})).is_err());
        assert_eq!(planes_of(&app), planes);
        assert_eq!(warp_of(&app).generation(), generation);
        assert_eq!(warp_of(&app).mode(), PlaneMode::Layout);
    }

    #[test]
    fn a_request_applies_all_or_nothing_and_planes_come_before_the_mode() {
        let mut app = perspective_app();
        let reply = control(&mut app, &json!({"planes": TWO_PLANES(), "mode": "warp"})).unwrap();
        assert_eq!(reply["mode"], "warp", "the planes of the same request are in place when Warp is asked for");
        assert_eq!(planes_of(&app).len(), 2);
        let (planes, generation, mode) = (planes_of(&app), warp_of(&app).generation(), warp_of(&app).mode());
        for bad in [
            json!({"planes": one_plane(), "straighten": "sideways"}),
            json!({"planes": one_plane(), "mode": "sideways"}),
            json!({"planes": [{"src": []}], "mode": "layout"}),
            json!({"planes": one_plane(), "mode": "layout", "commit": true, "cancel": true}),
            json!({"planes": one_plane(), "cancel": true}),
        ] {
            assert!(control(&mut app, &bad).is_err(), "{bad}");
            assert!(app.distort.perspective.is_some(), "{bad}");
            assert_eq!(planes_of(&app), planes, "{bad}");
            assert_eq!(warp_of(&app).generation(), generation, "{bad}");
            assert_eq!(warp_of(&app).mode(), mode, "{bad}");
        }
    }

    #[test]
    fn an_external_replacement_ends_a_running_drag_and_bumps_the_generation() {
        let mut app = perspective_app();
        control(&mut app, &json!({"planes": one_plane(), "mode": "warp"})).unwrap();
        let before = planes_of(&app);
        let id = GestureId::new(3);
        let begin = EditEvent::begin(id, Mixed::Same(before.clone()), "Move Corner".into(), None);
        session_apply(&mut app, PerspectiveEdit::Planes(begin)).unwrap();
        assert!(warp_of(&app).dragging());
        assert!(control(&mut app, &json!({"straighten": "auto"})).is_err(), "straighten waits for the drag");
        assert!(warp_of(&app).dragging());
        let generation = warp_of(&app).generation();
        control(&mut app, &json!({"planes": TWO_PLANES()})).unwrap();
        assert!(!warp_of(&app).dragging());
        assert!(warp_of(&app).generation() > generation);
    }

    #[test]
    fn a_refused_gesture_step_keeps_the_last_accepted_planes_and_still_ends_the_drag() {
        let mut app = perspective_app();
        control(&mut app, &json!({"planes": one_plane(), "mode": "warp"})).unwrap();
        let before = planes_of(&app);
        let mut moved = before.clone();
        moved[0].target[0] = Point::new(10.0, 10.0);
        let crossed = bowtie(&moved);
        let id = GestureId::new(7);
        let step = |app: &mut PhotocraftApp, event: EditEvent<Vec<Plane>>| session_apply(app, PerspectiveEdit::Planes(event));
        step(&mut app, EditEvent::begin(id, Mixed::Same(before.clone()), "Move Corner".into(), None)).unwrap();
        step(&mut app, EditEvent::update(id, moved.clone())).unwrap();
        step(&mut app, EditEvent::update(id, crossed.clone())).unwrap_err();
        assert_eq!(planes_of(&app), moved, "the refused update keeps the last accepted planes");
        assert!(warp_of(&app).dragging());
        step(&mut app, EditEvent::update(id, crossed.clone())).unwrap_err();
        step(&mut app, EditEvent::commit(id, Mixed::Same(before.clone()), crossed)).unwrap_err();
        assert!(!warp_of(&app).dragging(), "the release ends the drag even when its planes are refused");
        assert_eq!(planes_of(&app), moved);
        let session = &mut app.distort.perspective.as_mut().unwrap().warp;
        assert!(session.undo(), "the gesture is one undo step");
        assert_eq!(planes_of(&app), before);
    }

    #[test]
    fn a_valid_gesture_is_one_undo_step() {
        let mut app = perspective_app();
        control(&mut app, &json!({"planes": one_plane(), "mode": "warp"})).unwrap();
        let before = planes_of(&app);
        let mut moved = before.clone();
        moved[0].target[2] = Point::new(110.0, 90.0);
        let id = GestureId::new(9);
        let step = |app: &mut PhotocraftApp, event: EditEvent<Vec<Plane>>| session_apply(app, PerspectiveEdit::Planes(event));
        step(&mut app, EditEvent::begin(id, Mixed::Same(before.clone()), "Move Corner".into(), None)).unwrap();
        step(&mut app, EditEvent::update(id, moved.clone())).unwrap();
        step(&mut app, EditEvent::commit(id, Mixed::Same(before.clone()), moved.clone())).unwrap();
        assert!(!warp_of(&app).dragging());
        assert_eq!(planes_of(&app), moved);
        let session = &mut app.distort.perspective.as_mut().unwrap().warp;
        assert!(session.undo());
        assert_eq!(planes_of(&app), before);
        assert!(app.distort.perspective.as_mut().unwrap().warp.redo());
        assert_eq!(planes_of(&app), moved);
    }

    #[test]
    fn empty_and_identity_commits_explain_the_noop_without_changing_document_history() {
        let mut app = perspective_app();
        let before = past_len(&app);
        let reply = commit(&mut app).unwrap();
        assert_eq!(reply["committed"], false);
        assert_eq!(reply["reason"], "noPlanes");
        assert_eq!(past_len(&app), before);
        let mut app = perspective_app();
        let before = past_len(&app);
        control(&mut app, &json!({"planes": one_plane()})).unwrap();
        let reply = commit(&mut app).unwrap();
        assert_eq!(reply["committed"], false);
        assert_eq!(reply["reason"], "identity");
        assert_eq!(past_len(&app), before);
    }

    #[test]
    fn headless_edge_linking_is_atomic_and_reports_links_without_a_button() {
        let mut app = perspective_app();
        let a = json!([[20.0, 20.0], [90.0, 20.0], [90.0, 100.0], [20.0, 100.0]]);
        let b = json!([[94.0, 20.0], [164.0, 20.0], [164.0, 100.0], [94.0, 100.0]]);
        let initial = json!([
            {"src": a.clone(), "dst": a},
            {"src": b.clone(), "dst": b}
        ]);
        let response = control(&mut app, &json!({"planes": initial})).unwrap();
        assert_eq!(response["linkedEdges"], 0);
        let before = planes_of(&app);
        let generation = warp_of(&app).generation();
        for bad in [
            json!({"linkEdges": {"tolerance": 33.0}}),
            json!({"linkEdges": {"tolerance": -1.0}}),
            json!({"linkEdges": "yes"}),
            json!({"linkEdges": {"tolerance": "8"}}),
        ] {
            assert!(control(&mut app, &bad).is_err(), "{bad}");
            assert_eq!(planes_of(&app), before);
            assert_eq!(warp_of(&app).generation(), generation);
        }
        assert_eq!(control(&mut app, &json!({"linkEdges": false})).unwrap()["linkedEdges"], 0);
        let response = control(&mut app, &json!({"linkEdges": {"tolerance": 8.0}})).unwrap();
        assert_eq!(response["weldedEdges"], 1);
        assert_eq!(response["linkedEdges"], 1);
        let planes = planes_of(&app);
        assert_eq!(planes[0].source[1], planes[1].source[0]);
        assert_eq!(planes[0].source[2], planes[1].source[3]);
        assert_eq!(planes[0].target[1], planes[1].target[0]);
        assert_eq!(planes[0].target[2], planes[1].target[3]);
        assert!(warp_of(&app).generation() > generation);
        control(&mut app, &json!({"mode": "warp"})).unwrap();
        assert!(control(&mut app, &json!({"linkEdges": true})).is_err());
        assert_eq!(planes_of(&app), planes);
    }

    #[test]
    fn another_documents_session_is_neither_edited_nor_committed() {
        let mut app = perspective_app();
        control(&mut app, &json!({"planes": TWO_PLANES(), "mode": "warp"})).unwrap();
        let planes = planes_of(&app);
        app.session.execute("file.new", json!({"width": 50, "height": 50, "depth": 8})).unwrap();
        let past = past_len(&app);
        let document = app.session.active().unwrap().doc.clone();
        assert!(control(&mut app, &json!({"planes": one_plane()})).is_err());
        assert!(control(&mut app, &json!({"commit": true})).is_err());
        assert!(control(&mut app, &json!({"cancel": true})).is_err(), "foreign-document cancel cannot discard another document's session");
        assert!(session_apply(&mut app, PerspectiveEdit::Mode(PlaneMode::Layout)).is_err());
        assert!(commit(&mut app).is_err());
        assert!(app.distort.perspective.is_some(), "the refusal leaves the session to the stale-document check");
        assert_eq!(planes_of(&app), planes);
        assert_eq!(past_len(&app), past);
        assert!(Arc::ptr_eq(&document, &app.session.active().unwrap().doc));
    }

    #[test]
    fn a_commit_the_engine_refuses_keeps_the_session_and_its_preview() {
        let mut app = perspective_app();
        control(&mut app, &json!({"planes": TWO_PLANES(), "mode": "warp"})).unwrap();
        let before = planes_of(&app);
        let mut after = before.clone();
        after[0].target[0] = Point::new(30.0, 30.0);
        session_apply(&mut app, PerspectiveEdit::replace(&before, after, "Move Corner")).unwrap();
        let planes = planes_of(&app);
        // The layer the session warps is gone: the engine refuses the command.
        app.session.execute("layer.delete", json!({})).unwrap();
        let past = past_len(&app);
        assert!(commit(&mut app).is_err());
        assert!(control(&mut app, &json!({"commit": true})).is_err());
        assert!(app.distort.perspective.is_some());
        assert_eq!(planes_of(&app), planes);
        assert_eq!(past_len(&app), past);
        assert!(crate::distort_ui::display_doc(&app, app.session.active_index().unwrap()).is_some(), "the preview stays");
    }
}
