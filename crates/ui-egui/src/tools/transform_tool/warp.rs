use craft::behavior::EditEvents;
use craft::draw::ui::transform::TransformEdit;
use craft::imaging::ui::warp::{CylinderValues, WarpMesh};

use super::begin::warps_type;
use super::{BezierMesh, PhotocraftApp, Warp, WarpStyle, json};
use crate::state::SessionWarp;

/// Keeps the widget's point index tied to the knot layout it was picked on.
#[derive(Default)]
pub(super) struct GridSelection {
    knots: Option<(Vec<f64>, Vec<f64>)>,
    /// Gives the grid fresh widget memory when the old point indices no longer apply.
    pub(super) generation: u64,
    pub(super) picked: Option<usize>,
}

impl GridSelection {
    fn layout(warp: &SessionWarp) -> Option<(&[f64], &[f64])> {
        (!warp.on_type && warp.warp.style != WarpStyle::Cylinder).then_some((warp.mesh.us.as_slice(), warp.mesh.vs.as_slice()))
    }

    fn knots(&self) -> Option<(&[f64], &[f64])> {
        self.knots.as_ref().map(|(us, vs)| (us.as_slice(), vs.as_slice()))
    }

    pub(super) fn sync(&mut self, warp: Option<&SessionWarp>) {
        let layout = warp.and_then(Self::layout);
        if self.knots() != layout {
            self.knots = layout.map(|(us, vs)| (us.to_vec(), vs.to_vec()));
            self.generation += 1;
            self.picked = None;
        }
    }

    fn picked(&self, warp: &SessionWarp) -> Option<usize> {
        // A menu or control request can arrive before the next canvas pass.
        if self.knots() != Self::layout(warp) {
            return None;
        }
        self.picked
    }
}

/// Gives the session `warp` as its new warp (one `TransformEdit::Warp`), keeping whether it is on
/// a type layer. The one place the warp glue writes the session's warp.
pub(super) fn set_warp(app: &mut PhotocraftApp, warp: Warp) {
    let now = app.nudge_bundle.now;
    let on_type = app.ui.transform.as_ref().and_then(|t| t.session.warp()).map_or_else(|| warps_type(app), |w| w.on_type);
    if let Some(t) = app.ui.transform.as_mut() {
        t.session.apply(TransformEdit::Warp(SessionWarp::new(warp, on_type)), now);
        if let Some(pv) = app.transform_preview.as_mut() {
            pv.warp_grid.sync(t.session.warp());
        }
    }
}

/// The app's mesh of the framework's: the same knots and control points.
fn bezier_of(mesh: &WarpMesh) -> BezierMesh {
    BezierMesh { us: mesh.us.clone(), vs: mesh.vs.clone(), points: mesh.points.iter().map(|p| [p.x, p.y]).collect() }
}

/// Applies the warp grid's edits of this pass: every value it reports becomes a Custom warp over
/// the box (a preset warp's first edit turns it Custom), and a cancelled drag puts back the mesh it
/// began on. `split` is whether the pass placed a split line
/// (`matches!(out.edit, Some(WarpEdit::Split { .. }))`): the armed Split tool is one-shot, so it
/// disarms. A pass with no events changes nothing, and a second pass of a frame has none, so it
/// applies nothing twice.
pub(crate) fn grid_edit(app: &mut PhotocraftApp, events: &EditEvents<WarpMesh>, split: bool) {
    let Some(b) = app.ui.transform.as_ref().and_then(|t| t.session.warp()).map(|w| w.warp.bounds) else { return };
    let mut applied = false;
    for event in events.iter() {
        // PLAN: the sheet says a `Cancel` keeps the session's warp, but `TransformSession::apply(Warp(..))` stores each Update, so the drag's start is restored from the event's `before` (as a Custom warp over the same mesh).
        let Some(mesh) = event.landed() else { continue };
        set_warp(app, Warp::custom(bezier_of(mesh), b));
        applied = true;
    }
    if split
        && applied
        && let Some(pv) = app.transform_preview.as_mut()
    {
        pv.split = None;
    }
}

/// Applies the cylinder's edits of this pass: every value it reports becomes the cylinder warp
/// over the box, and a cancelled drag puts back the values it began with. A pass with no events
/// changes nothing. A set of values that describe no cylinder is refused and the warp stays.
pub(crate) fn cylinder_edit(app: &mut PhotocraftApp, events: &EditEvents<CylinderValues>) {
    let Some(b) = app.ui.transform.as_ref().and_then(|t| t.session.warp()).map(|w| w.warp.bounds) else { return };
    for event in events.iter() {
        let Some(values) = event.landed() else { continue };
        if let Some(warp) = Warp::cylinder(&values.to_array(), b) {
            set_warp(app, warp);
        }
    }
}

/// The seven values of `w` when it is a cylinder.
pub(crate) fn cylinder_values(w: &Warp) -> Option<CylinderValues> {
    if w.style != WarpStyle::Cylinder {
        return None;
    }
    <[f64; 7]>::try_from(w.values.as_slice()).ok().map(CylinderValues::from_array)
}

/// Runs a warp-edit command against the mesh in the open Warp session and writes the mesh back.
pub fn edit_session_warp(app: &mut PhotocraftApp, id: &str, params: &serde_json::Value) -> Result<(), String> {
    let Some(w) = app.ui.transform.as_ref().and_then(|t| t.session.warp()).map(|w| w.warp.clone()) else { return Err("not warping".into()) };
    if warps_type(app) {
        return Err("Warp on type takes only the warp styles".into());
    }
    let mut p = if params.is_object() { params.clone() } else { json!({}) };
    if p.get("warp").is_none() {
        p["warp"] = serde_json::to_value(&w).map_err(|e| e.to_string())?;
    }
    let r = app.session.execute(id, p).map_err(|e| e.to_string())?;
    let raw = r.get("warp").cloned().ok_or_else(|| "warp command returned no mesh".to_string())?;
    let nw: Warp = serde_json::from_value(raw).map_err(|e| e.to_string())?;
    set_warp(app, nw);
    Ok(())
}

/// Applies a split-warp command to the mesh being edited. Remove can use the grid's picked point
/// when no explicit `at` is supplied.
// PLAN: sheet note 8 says a split without `at` arms `pv.split`; the base splits at the box centre and the menu's callers rely on it, and tools-w27 does not name the command-to-`WarpSplit` function arming would need, so this keeps the base rule.
pub fn split(app: &mut PhotocraftApp, id: &str, at: Option<[f64; 2]>) -> Result<(), String> {
    let mut p = json!({});
    if let Some(a) = at {
        p["at"] = json!(a);
    } else if id == "edit.transform.removeWarpSplit"
        && let (Some(pv), Some(warp)) = (&app.transform_preview, app.ui.transform.as_ref().and_then(|t| t.session.warp()))
        && let Some(picked) = pv.warp_grid.picked(warp)
    {
        p["_pickedAnchor"] = json!(picked);
    }
    edit_session_warp(app, id, &p)
}
