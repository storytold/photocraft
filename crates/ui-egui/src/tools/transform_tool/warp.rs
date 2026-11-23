use craft::behavior::EditEvents;
use craft::draw::ui::transform::TransformEdit;
use craft::imaging::ui::warp::{CylinderValues, WarpEdit, WarpMesh, WarpSplit};

use super::begin::warps_type;
use super::{BezierMesh, PhotocraftApp, Warp, WarpStyle, json};
use crate::state::SessionWarp;

/// Gives the session `warp` as its new warp (one `TransformEdit::Warp`), keeping whether it is on
/// a type layer. The one place the warp glue writes the session's warp.
pub(super) fn set_warp(app: &mut PhotocraftApp, warp: Warp) {
    let now = app.nudge_bundle.now;
    let on_type = app.ui.transform.as_ref().and_then(|t| t.session.warp()).map_or_else(|| warps_type(app), |w| w.on_type);
    let mut warp = SessionWarp::new(warp, on_type);
    warp.placed = app.transform_preview.as_ref().and_then(|pv| pv.placed_warp);
    if let Some(t) = app.ui.transform.as_mut() {
        t.session.apply(TransformEdit::Warp(warp), now);
    }
}

/// How far along a grid line, in screen pixels, a pull of the surface reaches before it moves the line's own
/// anchors: the reference's Default-grid pull (`BezierMesh::pull_at`).
const LINE_REACH_PX: f64 = 23.0;

/// How close to a Perspective or Distort placement's horizon the pointer may take a warp point, as a share of the
/// homogeneous weight at the warp's centre ([`doc_to_contents`]).
const HORIZON_MARGIN: f64 = 1e-3;
/// How far from its centre (in the contents' space) a pointer at or past the horizon reaches. The reference's files
/// hold 6e8 to 1.3e9, but through its capped perspective the picture no longer changes from about 1e7 on, and a
/// pull moves a point up to about five times as far, well within what a warp under a perspective may store.
const HORIZON_REACH: f64 = 1e8;

/// Document point `p` in a smart object's contents, through the inverse `back` of its placement `to_doc` (the
/// contents' warp box `bounds`). Past the placement's horizon there is no such point: the inverse would put it behind
/// the picture and a drag there tore the warp apart. The reference sends such a pointer towards the vanishing point,
/// [`HORIZON_REACH`] out along the way the placement recedes, keeping where the inverse puts it across that way; and
/// near the horizon no farther than that from the centre.
fn doc_to_contents(to_doc: &photocraft_algo::transform::Homography, back: &photocraft_algo::transform::Homography, bounds: [f64; 4], p: [f64; 2]) -> [f64; 2] {
    let map = |q: [f64; 2]| {
        let (x, y) = back.apply(q[0], q[1]);
        [x, y]
    };
    let m = to_doc.0;
    // Only a Perspective or Distort placement has a horizon; an affine one maps as it is.
    if m[6] == 0.0 && m[7] == 0.0 {
        return map(p);
    }
    let centre = [(bounds[0] + bounds[2]) / 2.0, (bounds[1] + bounds[3]) / 2.0];
    let c = {
        let (x, y) = to_doc.apply(centre[0], centre[1]);
        [x, y]
    };
    let w_at = |q: [f64; 2]| back.0[6] * q[0] + back.0[7] * q[1] + back.0[8];
    let wc = w_at(c);
    if !(wc.abs() > 1e-300 && wc.is_finite()) {
        return map(p);
    }
    if w_at(p) / wc < HORIZON_MARGIN {
        let side = (m[6] * centre[0] + m[7] * centre[1] + m[8]).signum();
        let (dx, dy) = (m[6] * side, m[7] * side);
        let n = dx.hypot(dy);
        if !(n > 0.0 && n.is_finite()) {
            return centre;
        }
        let (ux, uy) = (dx / n, dy / n);
        let q = map(p);
        let (vx, vy) = (q[0] - centre[0], q[1] - centre[1]);
        let along = vx * ux + vy * uy;
        let (px, py) = if along.is_finite() { (vx - along * ux, vy - along * uy) } else { (0.0, 0.0) };
        let (px, py) = if px.is_finite() && py.is_finite() { (px, py) } else { (0.0, 0.0) };
        return [centre[0] + px + ux * HORIZON_REACH, centre[1] + py + uy * HORIZON_REACH];
    }
    let far = |q: [f64; 2]| (q[0] - centre[0]).hypot(q[1] - centre[1]) > HORIZON_REACH;
    let q = map(p);
    if !far(q) {
        return q;
    }
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    for _ in 0..60 {
        let mid = (lo + hi) / 2.0;
        if far(map([c[0] + mid * (p[0] - c[0]), c[1] + mid * (p[1] - c[1])])) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    map([c[0] + lo * (p[0] - c[0]), c[1] + lo * (p[1] - c[1])])
}

/// Where a press lies on the warp grid's surface, as the reference finds it: the mesh parameters of the nearest
/// surface point (`BezierMesh::param_at`), and only within `unit` of the press (the document size of a screen pixel),
/// so a press off the surface pulls nothing. Under a smart object's placement `to_doc` the press is taken into the
/// contents and found on the contents' mesh, as [`grid_rule`] applies the pull there.
pub(super) fn grid_surface(mesh: &WarpMesh, unit: f64, to_doc: Option<[f64; 9]>) -> impl Fn(craft::behavior::kurbo::Point) -> Option<(f64, f64)> {
    let doc = bezier_of(mesh);
    let placed = to_doc.and_then(|m| {
        let h = photocraft_algo::transform::Homography(m);
        Some((h.inverse()?, h))
    });
    let contents = placed.map(|(back, _)| {
        doc.map_points(|p| {
            let (x, y) = back.apply(p[0], p[1]);
            [x, y]
        })
    });
    // Past the perspective cap the picture (and the grid, [`shown_mesh`]) is drawn through the capped map: the press is
    // found where that map puts the contents.
    let capped = to_doc.zip(contents.as_ref()).and_then(|(m, c)| capped_view(&m, c));
    move |p: craft::behavior::kurbo::Point| {
        let (mesh, at, unit) = match (&contents, &placed) {
            (Some(contents), Some((back, to_doc))) => {
                let ((x, y), scale) = match &capped {
                    Some((fwd, inv)) => {
                        let (x, y) = inv(p.x, p.y);
                        let (a, b) = (fwd(x, y), fwd(x + 1.0, y));
                        ((x, y), (b.0 - a.0).hypot(b.1 - a.1))
                    }
                    None => {
                        let (x, y) = back.apply(p.x, p.y);
                        ((x, y), to_doc.local_scale(x, y))
                    }
                };
                (contents, [x, y], if scale.is_finite() && scale > 1e-9 { unit / scale } else { unit })
            }
            _ => (&doc, [p.x, p.y], unit),
        };
        let (s, t) = mesh.param_at(at);
        let q = mesh.eval(s, t);
        ((q[0] - at[0]).hypot(q[1] - at[1]) < unit).then_some((s, t))
    }
}

/// How close to a patch's edge, as a share of the patch, a split line may lie before it counts as on the edge (the
/// warp grid's own rule).
const SPLIT_EDGE: f64 = 0.01;

/// The lines split tool `tool` inserts in the one-patch Default grid at its parameters `(s, t)`: the tool's lines
/// that lie off the patch's edges, as the reference cuts it (its thirds are drawn handles, not lines). `None` when
/// every line lies on an edge.
pub(super) fn one_patch_split(tool: WarpSplit, s: f64, t: f64) -> Option<WarpSplit> {
    let free = |c: f64| (SPLIT_EDGE..=1.0 - SPLIT_EDGE).contains(&c);
    match (tool != WarpSplit::Horizontal && free(s), tool != WarpSplit::Vertical && free(t)) {
        (true, true) => Some(WarpSplit::Cross),
        (true, false) => Some(WarpSplit::Vertical),
        (false, true) => Some(WarpSplit::Horizontal),
        (false, false) => None,
    }
}

/// One edit of the warp grid as [`rule_mesh`] applies it (the grid's own edits can only be read, not made, here).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum RuleEdit {
    /// Control point `index` dragged by `by` (document units).
    Point { index: usize, by: [f64; 2] },
    /// The surface pulled at the mesh parameters `(s, t)` by `by`.
    Surface { s: f64, t: f64, by: [f64; 2] },
    /// A split of a one-patch mesh at its parameters `(s, t)`.
    SplitOnePatch { split: WarpSplit, s: f64, t: f64 },
}

/// The warp grid's rule, as measured on the reference application ([`rule_mesh`]); the grid's other edits are its
/// own. `unit` is the document size of a screen pixel. `to_doc` is the map from a smart object's contents to the
/// document when Warp mode edits its warp there ([`super::OpenPreview::placed_warp`]): the rule then runs in the
/// contents' own space, where the reference applies it, and its result is carried back.
pub(super) fn grid_rule(unit: f64, to_doc: Option<[f64; 9]>, armed: Option<WarpSplit>) -> impl Fn(&WarpMesh, &WarpEdit) -> WarpMesh {
    let placed = to_doc.and_then(|m| {
        let h = photocraft_algo::transform::Homography(m);
        Some((h, h.inverse()?))
    });
    move |mesh: &WarpMesh, edit: &WarpEdit| {
        let ours = match *edit {
            WarpEdit::Point { index, by, .. } => Some(RuleEdit::Point { index, by: [by.x, by.y] }),
            WarpEdit::Surface { s, t, by, .. } => Some(RuleEdit::Surface { s, t, by: [by.x, by.y] }),
            // The grid placed the cut on the thirds it would have made real: the same place on the one patch.
            WarpEdit::Split { split, column, row, u, v, thirds: true, .. } if mesh.columns() == 1 && mesh.rows() == 1 => {
                let (s, t) = ((column as f64 + u) / 3.0, (row as f64 + v) / 3.0);
                // The grid chose which lines to insert on the thirds it would have made real, so a cut on a third
                // dropped that line. On the one patch only its own edges are lines: an armed tool keeps its lines.
                let split = match armed {
                    Some(tool) => one_patch_split(tool, s, t).unwrap_or(split),
                    None => split,
                };
                Some(RuleEdit::SplitOnePatch { split, s, t })
            }
            _ => None,
        };
        let applied = match (ours, &placed) {
            (Some(ours @ (RuleEdit::Point { .. } | RuleEdit::Surface { .. })), Some((to_doc, back))) => in_contents(mesh, ours, unit, to_doc, back),
            // A split where the press lies in the contents: under a projective placement the carried mesh's
            // parameters at a document point are not the contents' there.
            (Some(RuleEdit::SplitOnePatch { split, s, t }), Some((_, back))) => {
                let doc = bezier_of(mesh);
                let at = doc.eval(s, t);
                let (x, y) = back.apply(at[0], at[1]);
                let (s, t) = doc
                    .map_points(|p| {
                        let (x, y) = back.apply(p[0], p[1]);
                        [x, y]
                    })
                    .param_at([x, y]);
                (s.is_finite() && t.is_finite()).then(|| mesh.split(split, 0, 0, s.clamp(0.0, 1.0), t.clamp(0.0, 1.0)))
            }
            (Some(ours), _) => rule_mesh(mesh, ours, unit),
            (None, _) => None,
        };
        applied.unwrap_or_else(|| mesh.edited(edit))
    }
}

/// The grids Warp mode has shown or proposed for a smart object under a projective placement, each with the map and
/// the stored-style mesh it stands for: shared by the drawing ([`shown_mesh`]), the warp grid's rule, which runs while
/// the grid is drawn, and [`grid_edit`], which lands them. The session holds the stored-style mesh (the contents'
/// control points carried through the plain map); the grid shows and is pulled in the picture's own space, which past
/// the render's perspective cap ([`photocraft_algo::smart_perspective::capped_map`]) is elsewhere.
#[derive(Clone, Default)]
pub(crate) struct ShownGrids(std::sync::Arc<std::sync::Mutex<Vec<Shown>>>);

/// A shown grid's points, the map it was carried through and the stored-style mesh's points.
type Shown = (Vec<[f64; 2]>, [f64; 9], Vec<[f64; 2]>);

fn points_of(mesh: &WarpMesh) -> Vec<[f64; 2]> {
    mesh.points.iter().map(|p| [p.x, p.y]).collect()
}

fn same(a: &[[f64; 2]], mesh: &WarpMesh) -> bool {
    a.len() == mesh.points.len() && a.iter().zip(&mesh.points).all(|(a, b)| a[0] == b.x && a[1] == b.y)
}

impl ShownGrids {
    /// The most kept: the grid is drawn every pass and its rule runs a few times a pass, and only the newest land.
    const KEPT: usize = 16;

    fn remember(&self, shown: &WarpMesh, placed: [f64; 9], stored: &WarpMesh) {
        let mut v = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let points = points_of(shown);
        v.retain(|(p, ..)| *p != points);
        v.push((points, placed, points_of(stored)));
        let extra = v.len().saturating_sub(Self::KEPT);
        v.drain(..extra);
    }

    /// The map and the stored-style mesh the shown grid `mesh` stands for, if it was shown or proposed.
    fn stored_of(&self, mesh: &WarpMesh) -> Option<([f64; 9], WarpMesh)> {
        let v = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (_, placed, stored) = v.iter().rev().find(|(p, ..)| same(p, mesh))?;
        Some((*placed, WarpMesh::new(mesh.us.clone(), mesh.vs.clone(), stored.iter().map(|p| craft::behavior::kurbo::Point::new(p[0], p[1])).collect())?))
    }

    /// The grid last shown or proposed for the stored-style mesh `stored` under `placed`.
    fn shown_of(&self, stored: &WarpMesh, placed: [f64; 9]) -> Option<WarpMesh> {
        let v = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (shown, ..) = v.iter().rev().find(|(_, m, s)| *m == placed && same(s, stored))?;
        WarpMesh::new(stored.us.clone(), stored.vs.clone(), shown.iter().map(|p| craft::behavior::kurbo::Point::new(p[0], p[1])).collect())
    }
}

/// The render's capped map of `to_doc` over the contents' control points `contents`, both ways, when the placement
/// converges past the cap there; `None` when the plain map is drawn.
#[allow(clippy::type_complexity)]
fn capped_view(to_doc: &[f64; 9], contents: &BezierMesh) -> Option<(Box<dyn Fn(f64, f64) -> (f64, f64)>, Box<dyn Fn(f64, f64) -> (f64, f64)>)> {
    let frame = contents.control_bounds();
    let fwd = photocraft_algo::smart_perspective::capped_map(to_doc, frame)?;
    let inv = photocraft_algo::smart_perspective::capped_inverse(to_doc, frame)?;
    Some((Box::new(fwd), Box::new(inv)))
}

/// The grid Warp mode shows for the session's stored-style mesh `stored` under the placement `to_doc`: its contents'
/// control points carried through the map the render draws the picture with, the capped one past the reference's
/// perspective cap (measured on the reference's own state after a long pull: its grid's side points lie where the
/// capped map puts its stored points, within 2 px; the plain map put the grid's corners hundreds of pixels inside the
/// picture). The same grid every pass for the same state, so a drag's value stays what it proposed.
pub(super) fn shown_mesh(stored: &WarpMesh, to_doc: Option<[f64; 9]>, shown: &ShownGrids) -> WarpMesh {
    let Some(m) = to_doc else { return stored.clone() };
    if let Some(mesh) = shown.shown_of(stored, m) {
        return mesh;
    }
    let Some(back) = photocraft_algo::transform::Homography(m).inverse() else { return stored.clone() };
    let contents = bezier_of(stored).map_points(|p| {
        let (x, y) = back.apply(p[0], p[1]);
        [x, y]
    });
    let Some((fwd, _)) = capped_view(&m, &contents) else { return stored.clone() };
    let doc = contents.map_points(|p| {
        let (x, y) = fwd(p[0], p[1]);
        [x, y]
    });
    let Some(mesh) = WarpMesh::new(doc.us, doc.vs, doc.points.iter().map(|p| craft::behavior::kurbo::Point::new(p[0], p[1])).collect()) else {
        return stored.clone();
    };
    if !mesh.points.iter().all(|p| p.x.is_finite() && p.y.is_finite()) {
        return stored.clone();
    }
    shown.remember(&mesh, m, stored);
    mesh
}

/// `rule` under a smart object's projective placement `to_doc` (its contents' box `bounds`): the grid it is given is
/// the one shown ([`shown_mesh`]), the rule runs on the stored-style mesh it stands for, and each pull's proposal is
/// given way at the horizon and shown as drawn ([`squeeze_proposal`]). The grid shows and lands exactly what it
/// proposed: had the host changed the proposal afterwards, the framework would see the warp changed under the drag
/// and end it (a long pull stuck until the button was released).
pub(super) fn squeezing_rule(
    rule: impl Fn(&WarpMesh, &WarpEdit) -> WarpMesh,
    to_doc: Option<[f64; 9]>,
    bounds: [f64; 4],
    squeezes: ShownGrids,
) -> impl Fn(&WarpMesh, &WarpEdit) -> WarpMesh {
    move |mesh: &WarpMesh, edit: &WarpEdit| {
        let stored = squeezes.stored_of(mesh).map(|(_, s)| s);
        let held = stored.as_ref().unwrap_or(mesh);
        let out = rule(held, edit);
        match to_doc {
            Some(m) if matches!(edit, WarpEdit::Point { .. } | WarpEdit::Surface { .. }) => squeeze_proposal(out, m, bounds, &squeezes),
            _ => out,
        }
    }
}

/// A pull's stored-style result `out` (carried through the placement `to_doc`, the contents' box `bounds`) as the grid
/// proposes it: when the pull takes the warp towards the viewer past the placement's horizon, the placement gives way
/// ([`horizon_squeeze`]); the proposal is the result shown through that map ([`shown_mesh`]), which `squeezes`
/// remembers with the map and the stored-style mesh for [`grid_edit`].
pub(super) fn squeeze_proposal(out: WarpMesh, to_doc: [f64; 9], bounds: [f64; 4], squeezes: &ShownGrids) -> WarpMesh {
    let to_doc = photocraft_algo::transform::Homography(to_doc);
    let Some(back) = to_doc.inverse() else { return out };
    let contents = bezier_of(&out).map_points(|p| {
        let (x, y) = back.apply(p[0], p[1]);
        [x, y]
    });
    let given = match horizon_squeeze(to_doc.0, &contents.points, bounds) {
        Some(squeeze) => to_doc.mul(&photocraft_algo::transform::Homography(squeeze)),
        None => to_doc,
    };
    if !given.0.iter().all(|v| v.is_finite()) {
        return out;
    }
    let doc = contents.map_points(|p| {
        let (x, y) = given.apply(p[0], p[1]);
        [x, y]
    });
    let Some(stored) = WarpMesh::new(doc.us, doc.vs, doc.points.iter().map(|p| craft::behavior::kurbo::Point::new(p[0], p[1])).collect()) else {
        return out;
    };
    let shown = shown_mesh(&stored, Some(given.0), squeezes);
    // Plain and unsqueezed, the grid is the stored-style mesh itself; remembered all the same, so it lands with its map.
    squeezes.remember(&shown, given.0, &stored);
    shown
}

/// [`rule_mesh`] in a smart object's contents: the mesh, the press and the pointer's move taken back through
/// `back`, the rule applied there (with the reach in contents units at the press) and the mesh carried to the
/// document through `to_doc`.
pub(super) fn in_contents(
    mesh: &WarpMesh,
    edit: RuleEdit,
    unit: f64,
    to_doc: &photocraft_algo::transform::Homography,
    back: &photocraft_algo::transform::Homography,
) -> Option<WarpMesh> {
    let map = |h: &photocraft_algo::transform::Homography, p: [f64; 2]| {
        let (x, y) = h.apply(p[0], p[1]);
        [x, y]
    };
    let doc = bezier_of(mesh);
    let contents = doc.map_points(|p| map(back, p));
    // The press in the document, and where the pointer is now, both taken into the contents. A surface press is the
    // contents' point at its parameters (where [`grid_surface`] found the pointer) taken through the placement itself:
    // under a perspective the carried mesh's point there lies elsewhere, and a pull from it moved too little. The pull
    // works through the plain map, held before its horizon ([`doc_to_contents`]), also where the grid is drawn through
    // the capped one ([`shown_mesh`]): through the capped map's inverse a sideways pull after a long one took points
    // behind the plain horizon, and the grid flipped through infinity (the sequence check, seed 967 of 4,000).
    let (press, by) = match edit {
        RuleEdit::Point { index, by } => (doc.points.get(index).copied()?, by),
        RuleEdit::Surface { s, t, by } => (map(to_doc, contents.eval(s, t)), by),
        RuleEdit::SplitOnePatch { .. } => return None,
    };
    let bounds = contents.control_bounds();
    // A surface pull presses [`PULL_PRESS`] from the pointer in the document, at the contents' point found there; off
    // the surface it keeps the pointer's own.
    let (press, edit) = match edit {
        RuleEdit::Surface { by, .. } => {
            let q = [press[0] + PULL_PRESS[0], press[1] + PULL_PRESS[1]];
            match on_surface(&contents, doc_to_contents(to_doc, back, bounds, q)) {
                Some((s, t)) => (q, RuleEdit::Surface { s, t, by }),
                None => (press, edit),
            }
        }
        _ => (press, edit),
    };
    let (from, to) = (doc_to_contents(to_doc, back, bounds, press), doc_to_contents(to_doc, back, bounds, [press[0] + by[0], press[1] + by[1]]));
    let by = [to[0] - from[0], to[1] - from[1]];
    let scale = to_doc.local_scale(from[0], from[1]);
    let unit = if scale.is_finite() && scale > 1e-9 { unit / scale } else { unit };
    let mut moved = contents.clone();
    match edit {
        RuleEdit::Point { index, .. } => moved.drag_point(&contents.points, index, by),
        RuleEdit::Surface { s, t, .. } => moved.pull_at(&contents.points, s, t, from, by, LINE_REACH_PX * unit),
        RuleEdit::SplitOnePatch { .. } => return None,
    }
    let placed = moved.map_points(|p| map(to_doc, p));
    WarpMesh::new(placed.us, placed.vs, placed.points.iter().map(|p| craft::behavior::kurbo::Point::new(p[0], p[1])).collect())
}

/// How far from the pointer the reference presses a pull of the warp's surface, in document pixels: half a pixel up
/// and left, also under a smart object's placement. Measured on 154 Default-grid pull probes at 100 % (median error
/// 2.05 → 1.29 px, worst 7.58 → 3.85), 12 at 50 %, where it is half a document pixel, not half a screen pixel (1.59 →
/// 1.23; a whole pixel, half a screen pixel there, made it worse), and the pulls under a perspective (pixels off by
/// more than 32 levels, 189,166 → 141,476 over the catalogue). Box handles and warp points take the pointer itself.
const PULL_PRESS: [f64; 2] = [-0.5, -0.5];

/// The parameters of the surface point a pull at `(s, t)` presses: the point [`PULL_PRESS`] away from the pointer's,
/// or the pointer's own where that lies off the surface (past an edge of the grid).
pub(super) fn pressed(base: &BezierMesh, s: f64, t: f64) -> (f64, f64) {
    let p = base.eval(s, t);
    on_surface(base, [p[0] + PULL_PRESS[0], p[1] + PULL_PRESS[1]]).unwrap_or((s, t))
}

/// The parameters of `base`'s surface point at `q`, or `None` where `q` lies off the surface.
fn on_surface(base: &BezierMesh, q: [f64; 2]) -> Option<(f64, f64)> {
    let (s, t) = base.param_at(q);
    let found = base.eval(s, t);
    ((0.0..=1.0).contains(&s) && (0.0..=1.0).contains(&t) && (found[0] - q[0]).hypot(found[1] - q[1]) < 0.05).then_some((s, t))
}

/// `mesh` after `edit`, as the reference application moves it: a point drag moves an anchor with its handles and
/// the twist points beside it (`BezierMesh::drag_point`), a pull of the surface moves the points the reference moves
/// (`BezierMesh::pull_at`, with its thirds and a line reach of [`LINE_REACH_PX`] screen pixels, `unit` each), and a
/// split of the one-patch Default grid adds only its own lines: the grid's thirds stay drawn handles. `None` when
/// the result is no mesh.
pub(super) fn rule_mesh(mesh: &WarpMesh, edit: RuleEdit, unit: f64) -> Option<WarpMesh> {
    let base = bezier_of(mesh);
    let mut moved = base.clone();
    match edit {
        RuleEdit::Point { index, by } => moved.drag_point(&base.points, index, by),
        RuleEdit::Surface { s, t, by } => {
            let (s, t) = pressed(&base, s, t);
            moved.pull_at(&base.points, s, t, base.eval(s, t), by, LINE_REACH_PX * unit);
        }
        RuleEdit::SplitOnePatch { split, s, t } => return Some(mesh.split(split, 0, 0, s, t)),
    }
    WarpMesh::new(moved.us, moved.vs, moved.points.iter().map(|p| craft::behavior::kurbo::Point::new(p[0], p[1])).collect())
}

/// A warp built over the warp's own box (a style from the options bar, Reset) as the session holds it: while Warp
/// mode edits a smart object's warp in its contents ([`super::OpenPreview::placed_warp`]) that box is the contents',
/// so its points are carried into the document, where the grid and the preview read them, as a custom mesh (the
/// reference stores its presets as custom meshes too). Otherwise as it is.
pub(super) fn placed_doc_warp(app: &PhotocraftApp, warp: Warp) -> Warp {
    let Some((to_doc, _)) = app.transform_preview.as_ref().and_then(|pv| pv.placed_warp) else { return warp };
    let h = photocraft_algo::transform::Homography(to_doc);
    let mesh = warp.to_mesh(1, 1).map_points(|p| {
        let (x, y) = h.apply(p[0], p[1]);
        [x, y]
    });
    Warp::custom(mesh, warp.bounds)
}

/// How close to a Perspective or Distort placement's horizon a pulled warp may come on the viewer's side before the
/// placement gives way ([`horizon_squeeze`]): its homogeneous weight there as a share of the weight on the far side
/// (the reference's files: 0.0099 to 0.0116 over pulls from 240 to 300 px).
pub(super) const HORIZON_SQUEEZE: f64 = 0.0107;

/// A smart object's warp (`points` over its contents' box `bounds`) pulled towards the viewer past its Perspective or
/// Distort placement `to_doc`'s horizon: the squeeze the placement takes first so every point stays on the picture's
/// side, as the reference does, along the way the picture recedes about the far side of the box, until the nearest
/// point is back at [`HORIZON_SQUEEZE`] of the weight there. `None` when nothing comes that near.
pub(super) fn horizon_squeeze(m: [f64; 9], points: &[[f64; 2]], bounds: [f64; 4]) -> Option<[f64; 9]> {
    if m[6] == 0.0 && m[7] == 0.0 {
        return None;
    }
    let c = [(bounds[0] + bounds[2]) / 2.0, (bounds[1] + bounds[3]) / 2.0];
    let side = (m[6] * c[0] + m[7] * c[1] + m[8]).signum();
    let (gx, gy) = (m[6] * side, m[7] * side);
    let n = gx.hypot(gy);
    if !(n > 0.0 && n.is_finite() && side != 0.0) {
        return None;
    }
    let (ux, uy) = (gx / n, gy / n);
    let a = m[8] * side;
    let s_of = |p: &[f64; 2]| p[0] * ux + p[1] * uy;
    let [x0, y0, x1, y1] = bounds;
    let box_s = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]].map(|p| s_of(&p));
    let s_far = box_s.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let box_near = box_s.iter().copied().fold(f64::INFINITY, f64::min);
    let s_near = points.iter().map(s_of).fold(f64::INFINITY, f64::min);
    let (w_far, w_near) = (a + n * s_far, a + n * s_near);
    if !(w_far > 0.0 && w_near.is_finite() && s_far > s_near && s_near < box_near) || w_near >= HORIZON_SQUEEZE * w_far {
        return None;
    }
    let k = (HORIZON_SQUEEZE - 1.0) * w_far / (n * (s_near - s_far));
    if !(k > 0.0 && k < 1.0) {
        return None;
    }
    Some([
        1.0 + (k - 1.0) * ux * ux,
        (k - 1.0) * ux * uy,
        -(k - 1.0) * s_far * ux,
        (k - 1.0) * ux * uy,
        1.0 + (k - 1.0) * uy * uy,
        -(k - 1.0) * s_far * uy,
        0.0,
        0.0,
        1.0,
    ])
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
        // A smart object's warp under its placement: the drag works through the map it began with; what the drag
        // shows and lands is that map as the pull has squeezed it.
        if matches!(event, craft::behavior::EditEvent::Begin { .. })
            && let Some(pv) = app.transform_preview.as_mut()
            && pv.drag_placed.is_none()
        {
            pv.drag_placed = pv.placed_warp;
        }
        // PLAN: the sheet says a `Cancel` keeps the session's warp, but `TransformSession::apply(Warp(..))` stores each Update, so the drag's start is restored from the event's `before` (as a Custom warp over the same mesh).
        let Some(mesh) = event.landed() else { continue };
        applied = true;
        let base = app.transform_preview.as_ref().and_then(|pv| pv.drag_placed.or(pv.placed_warp));
        let ends = matches!(event, craft::behavior::EditEvent::Commit { .. } | craft::behavior::EditEvent::Cancel { .. });
        let cancelled = matches!(event, craft::behavior::EditEvent::Cancel { .. });
        // A smart object's warp pulled towards the viewer past its placement's horizon: the placement gives way as
        // the reference's does ([`horizon_squeeze`]), all through the drag (the reference's picture passes the horizon
        // once and settles, it never flips back and forth), and the points are carried through it.
        // A grid Warp mode showed or proposed (in the picture's space past the perspective cap) lands as the stored-style
        // mesh it stands for, with its map: a proposal as proposed, a cancel as the drag began.
        let known = app.transform_preview.as_ref().and_then(|pv| pv.shown_grids.stored_of(mesh));
        let squeezed = match (known, base) {
            (Some((placed, stored)), _) => Some((placed, bezier_of(&stored))),
            (None, Some((to_doc, _))) if !cancelled => photocraft_algo::transform::Homography(to_doc).inverse().and_then(|back| {
                let contents = bezier_of(mesh).map_points(|p| {
                    let (x, y) = back.apply(p[0], p[1]);
                    [x, y]
                });
                let squeeze = horizon_squeeze(to_doc, &contents.points, b)?;
                let placed = photocraft_algo::transform::Homography(to_doc).mul(&photocraft_algo::transform::Homography(squeeze));
                placed.0.iter().all(|v| v.is_finite()).then(|| {
                    let doc = contents.map_points(|p| {
                        let (x, y) = placed.apply(p[0], p[1]);
                        [x, y]
                    });
                    (placed.0, doc)
                })
            }),
            _ => None,
        };
        match squeezed {
            Some((placed, doc)) => {
                if let Some(pv) = app.transform_preview.as_mut() {
                    // The map the drag began with stays as it was; one it gave way to is a moved placement.
                    pv.placed_warp = if base.is_some_and(|(m, _)| m == placed) { base } else { Some((placed, true)) };
                }
                set_warp(app, Warp::custom(doc, b));
            }
            None => {
                if let Some(pv) = app.transform_preview.as_mut()
                    && pv.drag_placed.is_some()
                {
                    pv.placed_warp = base;
                }
                set_warp(app, Warp::custom(bezier_of(mesh), b));
            }
        }
        if ends && let Some(pv) = app.transform_preview.as_mut() {
            pv.drag_placed = None;
        }
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

/// Applies a split-warp command to the mesh being edited (at the box centre unless `at`).
// PLAN: sheet note 8 says a split without `at` arms `pv.split`; the base splits at the box centre and the menu's callers rely on it, and tools-w27 does not name the command-to-`WarpSplit` function arming would need, so this keeps the base rule.
pub fn split(app: &mut PhotocraftApp, id: &str, at: Option<[f64; 2]>) -> Result<(), String> {
    let mut p = json!({});
    if let Some(a) = at {
        p["at"] = json!(a);
    }
    edit_session_warp(app, id, &p)
}

/// Warp mode on a moved smart object (its contents shown through `placed`, [`super::OpenPreview::placed_warp`]) that has
/// no warp of its own, while the grid is still flat: the reference shows and applies that as the box's move alone, its
/// exact map, and changes the picture only once the warp is edited (from then on the control points carried through the
/// move, which spread the rows evenly). The contents' corners in the document then, else `None`.
///
/// Deliberate parity: the first warp edit therefore redistributes the picture once (the reference's own recording: a
/// 6 px pull after a Perspective moved a boundary 14 px, ours too). The reference saves placed warps in that carried
/// model, so drawing pulled warps any other way would also render PSD files differently from it.
pub(super) fn flat_placed_warp(doc: &photocraft_doc::Document, layer: u64, warp: &Warp, placed: Option<([f64; 9], bool)>) -> Option<[[f64; 2]; 4]> {
    let (m, _) = placed?;
    let photocraft_doc::LayerContent::Smart(sm) = &doc.layer(photocraft_doc::LayerId(layer))?.content else { return None };
    if sm.warp.as_ref().is_some_and(|w| !w.is_identity()) {
        return None;
    }
    let to_doc = photocraft_algo::transform::Homography(m);
    let back = to_doc.inverse()?;
    let b = warp.bounds;
    let (mesh, flat) = (warp.to_mesh(1, 1), photocraft_geom::warp::BezierMesh::identity(b, 1, 1));
    let tol = 1e-6 * ((b[2] - b[0]).abs() + (b[3] - b[1]).abs()).max(1.0);
    let unmoved = mesh.points.len() == flat.points.len()
        && mesh.points.iter().zip(&flat.points).all(|(p, q)| {
            let (x, y) = back.apply(p[0], p[1]);
            (x - q[0]).abs() <= tol && (y - q[1]).abs() <= tol
        });
    unmoved.then(|| {
        [[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]].map(|p| {
            let (x, y) = to_doc.apply(p[0], p[1]);
            [x, y]
        })
    })
}

/// A pixel layer's (or an upright smart object's) warp still flat in a box in perspective: the box's corners, else
/// `None`. The reference shows and applies that as the box's exact map until the warp is edited, as for a moved smart
/// object ([`flat_placed_warp`]); in an affine box the flat grid already is the exact map.
pub(super) fn flat_projective_in_box(warp: &Warp, rect: [f64; 4], quad: [[f64; 2]; 4]) -> Option<[[f64; 2]; 4]> {
    let h = photocraft_algo::transform::Homography::rect_to_quad(rect, quad)?;
    let projective = (h.0[6].abs() + h.0[7].abs()) > 1e-12 * h.0[8].abs().max(1e-300);
    (projective && flat_in_box(warp, rect, quad)).then_some(quad)
}

/// Whether `warp`, built over the box's frame `rect` and shown through the box at `quad` (a pixel layer's warp, or an
/// upright smart object's edited in the document), is still the flat grid: its points are the frame's thirds carried
/// through the box's map.
pub(super) fn flat_in_box(warp: &Warp, rect: [f64; 4], quad: [[f64; 2]; 4]) -> bool {
    let Some(h) = photocraft_algo::transform::Homography::rect_to_quad(rect, quad) else { return false };
    let b = warp.bounds;
    let (mesh, flat) = (warp.to_mesh(1, 1), photocraft_geom::warp::BezierMesh::identity(rect, 1, 1));
    let tol = 1e-6 * ((b[2] - b[0]).abs() + (b[3] - b[1]).abs()).max(1.0);
    mesh.points.len() == flat.points.len()
        && mesh.points.iter().zip(&flat.points).all(|(p, q)| {
            let (x, y) = h.apply(q[0], q[1]);
            (p[0] - x).abs() <= tol && (p[1] - y).abs() <= tol
        })
}
