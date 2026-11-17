//! Edit › Perspective Warp on the canvas.
//!
//! **Layout** mode: drag on the image to draw a plane, drag its corners onto the image's perspective lines; a
//! corner dropped near another plane's corner snaps to it, which links the planes along that edge. **Warp** mode:
//! drag corners (linked corners move together) and the image follows; the options bar straightens near-vertical /
//! near-horizontal edges. The framework's `PerspectiveWarp` takes the pointer and keys in the canvas body,
//! `PerspectiveBar` is the options bar, and `PerspectiveSession` holds the planes, the mode and the session's undo
//! steps. ↩ / Commit runs `edit.perspectiveWarp` with the planes. The preview is the layer texture on a grid pushed
//! through the same piecewise-homography map the engine uses.

use std::sync::Arc;

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
        json!({"document": self.document.0, "layer": self.layer.0, "mode": mode, "planes": to_algo(self.warp.planes())})
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
    let textures = PreviewTextures::new(format!("persp-{key}"), PreviewPixels::new(image, [1.0, 1.0]));
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

/// Commits the session: one `edit.perspectiveWarp` step. No planes, or planes that all map onto themselves, run no
/// command.
pub fn commit(app: &mut PhotocraftApp) {
    let Some(s) = app.distort.perspective.take() else { return };
    let Some(planes) = s.warp.result() else { return };
    let _ = app.run(&s.command, json!({"planes": to_algo(planes), "layer": s.layer.0}));
}

/// Control channel: `edit.perspectiveWarp {"ui": {...}}` while the mode is active.
pub fn control(app: &mut PhotocraftApp, ui: &Value) -> Result<Value, String> {
    if ui.get("commit").and_then(Value::as_bool) == Some(true) {
        commit(app);
        return Ok(json!({"committed": true}));
    }
    if ui.get("cancel").and_then(Value::as_bool) == Some(true) {
        app.distort.perspective = None;
        return Ok(json!({"cancelled": true}));
    }
    let s = app.distort.perspective.as_mut().ok_or(tl!("Perspective Warp is not active"))?;
    if let Some(m) = ui.get("mode").and_then(Value::as_str) {
        s.warp.apply(PerspectiveEdit::Mode(if m == "warp" { PlaneMode::Warp } else { PlaneMode::Layout }));
    }
    if let Some(p) = ui.get("planes") {
        let planes: Vec<AlgoPlane> = serde_json::from_value(p.clone()).map_err(|e| format!("bad planes: {e}"))?;
        // The session ends a corner drag it holds indices of (#952).
        s.warp.replace(from_algo(&planes));
    }
    if let Some(st) = ui.get("straighten").and_then(Value::as_str) {
        let how = AlgoStraighten::parse(st).ok_or("straighten: horizontal|vertical|auto")?;
        s.warp.apply(PerspectiveEdit::Straighten(straighten_of(how)));
    }
    Ok(s.describe())
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
    if let Some(s) = app.distort.perspective.as_mut() {
        for edit in output.edits() {
            s.warp.apply(edit);
        }
    }
    if output.apply {
        commit(app);
    } else if output.discard {
        app.distort.perspective = None;
    }
}

/// The options bar: the Layout and Warp tabs, the plane count and Remove Planes or the straighten buttons, commit and
/// cancel.
pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let Some(s) = app.distort.perspective.as_mut() else { return };
    let output = PerspectiveBar::new(&s.warp).show(ui);
    for edit in output.edits {
        s.warp.apply(edit);
    }
    if output.apply {
        commit(app);
    } else if output.discard {
        app.distort.perspective = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn perspective_app() -> PhotocraftApp {
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
        let s = app.distort.perspective.as_mut().unwrap();
        s.warp.apply(PerspectiveEdit::replace(&before, after, "Move Corner"));
        let planes = s.describe()["planes"].clone();
        assert_eq!(planes[0]["dst"][1], json!([100.0, 5.0]));
        assert_eq!(planes[1]["dst"][0], json!([100.0, 5.0]));
        assert_eq!(planes[0]["src"][1], json!([100.0, 20.0]), "warp mode keeps the layout");
        assert_eq!(planes[1]["src"][3], json!([100.0, 100.0]), "corners run clockwise from the top left");
        let past = past_len(&app);
        commit(&mut app);
        assert!(app.distort.perspective.is_none());
        assert_eq!(past_len(&app), past + 1);
    }

    #[test]
    fn planes_that_map_onto_themselves_commit_nothing() {
        let mut app = perspective_app();
        control(&mut app, &json!({"planes": TWO_PLANES()})).unwrap();
        let past = past_len(&app);
        commit(&mut app);
        assert!(app.distort.perspective.is_none());
        assert_eq!(past_len(&app), past);
    }
}
