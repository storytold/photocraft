//! The shape layers' canvas controls: the bounding box with eight handles that scales the selected
//! shapes, and the corner radius widgets of live rectangles, polygons and stars.
//!
//! With Path Selection or a shape tool active and shape layers selected, the union of their path
//! bounds shows as a `TransformBox` (no rotate zone, no reference point, no inside move). A drag
//! scales every selected shape with one `edit.transform`, which keeps live shapes live, as one
//! History step named "Transform Path"; no Free Transform session opens. The `CornerWidgets` come
//! after the box in the canvas body, so a widget wins a press where both reach.
//!
//! Neither control keeps a drag in PhotoCraft: the box's id salt carries the document, its revision
//! and the tool, so a switch, an Undo or a tool change makes a new control and the library cancels
//! the press of the one that is gone.

use craft::behavior::EditEvent;
use craft::behavior::geometry::quad::{BoxAllow, BoxRules, BoxState, ScaleRule, TransformMode};
use craft::behavior::kurbo::{BezPath, Point, Rect as DocRect};
use craft::draw::ui::corner::{CornerKind, CornerPoint, CornerTarget, CornerWidgets};
use craft::draw::ui::transform::{LineLook, RotateZone, TransformBox};
use craft::ui::canvas::ToolHost;
use craft::ui::canvas_controls::SelectionOutline;
use craft::ui::contract::{Bind, Props, Show};
use photocraft_doc::vector::{LiveShape, Path};
use photocraft_doc::{DocId, LayerContent, LayerId};
use photocraft_geom::Affine;
use photocraft_vector::shapes;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::CanvasHost;
use crate::state::Tool;

#[cfg(test)]
mod box_tests;
#[cfg(test)]
mod corner_tests;

/// The scale a drag can reach, as `path.transform`'s.
const MIN_SCALE: f64 = 0.01;
const MAX_SCALE: f64 = 100.0;

/// The controls' names in the accessibility tree and in the History step of a drag.
const BOX_LABEL: &str = "Shape bounds";
const CORNERS_LABEL: &str = "Corner radius";
const BOX_OUTLINE_LABEL: &str = "Scaled shape outlines";
const CORNERS_OUTLINE_LABEL: &str = "Rounded shape outlines";

/// Shows the box and the corner widgets over the canvas of `ui`, and the outlines the running drag
/// proposes. The box comes first so a widget wins a press where both reach.
pub(crate) fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let rect = ui.max_rect();
    show_box(app, ui, rect);
    show_widgets(app, ui, rect);
}

/// Whether the box or the corner widgets hold a press now. The framework's read covers every canvas
/// control of the active document's canvas; only these two and the Direct Selection controls exist
/// on a shape.
pub(crate) fn editing(app: &PhotocraftApp) -> bool {
    let Some(idx) = app.session.active_index() else { return false };
    ToolHost::controls_pressing(app.ctx(), CanvasHost::primary(app, idx).canvas_id())
}

/// The salt that ends a control's press when the document, an edit under it or the tool changes.
fn salt(app: &PhotocraftApp) -> Option<(DocId, u64, Tool)> {
    app.session.active().map(|st| (st.doc.id, st.revision, app.ui.tool))
}

// ---------------------------------------------------------------------------------------------
// The scale box

/// The tools that show the box, while View › Extras and View › Show › Target Path show paths
/// (hidden handles take no press).
fn box_shows(app: &PhotocraftApp) -> bool {
    app.ui.transform.is_none()
        && app.ui.view.shows(app.ui.view.show.target_path)
        && matches!(app.ui.tool, Tool::PathSelection | Tool::Rectangle | Tool::EllipseShape | Tool::Triangle | Tool::Polygon | Tool::Star | Tool::Line)
}

/// The selected shape layers and the box round their paths `[x0, y0, x1, y1]`. `None` unless at least
/// one layer is selected and all are shape layers. A box with no extent along one axis counts only
/// when `flat_ok` (a level or upright live line).
fn selection(app: &PhotocraftApp, flat_ok: bool) -> Option<(Vec<u64>, [f64; 4])> {
    let st = app.session.active()?;
    let ids = st.selected_layers();
    if ids.is_empty() {
        return None;
    }
    let mut layers = Vec::with_capacity(ids.len());
    let mut rect: Option<[f64; 4]> = None;
    for id in ids {
        let l = st.doc.layer(id)?;
        let LayerContent::Shape(sh) = &l.content else { return None };
        let (x0, y0, x1, y1) = sh.path.control_bounds()?;
        rect = Some(rect.map_or([x0, y0, x1, y1], |r| [r[0].min(x0), r[1].min(y0), r[2].max(x1), r[3].max(y1)]));
        layers.push(l.id.0);
    }
    let r = rect.filter(|r| {
        r.iter().all(|v| v.is_finite()) && if flat_ok { r[2] >= r[0] && r[3] >= r[1] && (r[2] > r[0] || r[3] > r[1]) } else { r[2] > r[0] && r[3] > r[1] }
    })?;
    Some((layers, r))
}

/// With the Line tool, the active layer's live line when it is the whole selection: its ends and, with an
/// arrowhead, the corners of the box aligned with the segment.
fn live_line(app: &PhotocraftApp) -> Option<LineLook> {
    if app.ui.tool != Tool::Line {
        return None;
    }
    let st = app.session.active()?;
    let id = st.active_layer?;
    if st.selected_layers() != [id] {
        return None;
    }
    let LayerContent::Shape(sh) = &st.doc.layer(id)?.content else { return None };
    let (from, to, arrow) = match sh.live.as_ref()? {
        LiveShape::Line { from, to, .. } => (*from, *to, false),
        LiveShape::ArrowLine { from, to, start, end, .. } => (*from, *to, *start || *end),
        _ => return None,
    };
    let len = (to[0] - from[0]).hypot(to[1] - from[1]);
    if !(from.iter().chain(&to).all(|v| v.is_finite()) && len > 0.0) {
        return None;
    }
    let rotated = arrow.then(|| {
        // The path's control points in the segment's frame: `u` along it, `n` across it.
        let (u, n) = ([(to[0] - from[0]) / len, (to[1] - from[1]) / len], [(from[1] - to[1]) / len, (to[0] - from[0]) / len]);
        let at = |a: f64, b: f64| Point::new(from[0] + u[0] * a + n[0] * b, from[1] + u[1] * a + n[1] * b);
        let mut r = [0.0, 0.0, len, 0.0];
        for p in sh.path.subpaths.iter().flat_map(|s| &s.knots).flat_map(|k| [k.anchor, k.in_ctrl, k.out_ctrl]) {
            let (dx, dy) = (p.x - from[0], p.y - from[1]);
            let (a, b) = (dx * u[0] + dy * u[1], dx * n[0] + dy * n[1]);
            r = [r[0].min(a), r[1].min(b), r[2].max(a), r[3].max(b)];
        }
        [at(r[0], r[1]), at(r[2], r[1]), at(r[2], r[3]), at(r[0], r[3])]
    });
    Some(LineLook::new(Point::new(from[0], from[1]), Point::new(to[0], to[1])).with_outline(rotated))
}

/// The bounds of `state`'s four corners.
fn bounds_of(state: &BoxState) -> [f64; 4] {
    let xs = state.quad.map(|p| p.x);
    let ys = state.quad.map(|p| p.y);
    [
        xs.into_iter().fold(f64::INFINITY, f64::min),
        ys.into_iter().fold(f64::INFINITY, f64::min),
        xs.into_iter().fold(f64::NEG_INFINITY, f64::max),
        ys.into_iter().fold(f64::NEG_INFINITY, f64::max),
    ]
}

/// The scale per axis that takes the box `start` to `now`, and the point it scales about. The scale is
/// kept inside what `path.transform` reaches. The point is the one the map leaves in place: the
/// opposite handle for a plain drag, the reference point for ⌥. On an axis that did not scale it is
/// the start box's centre. `None` when nothing scaled or a bound is not finite.
fn box_scale(start: [f64; 4], now: [f64; 4]) -> Option<([f64; 2], [f64; 2])> {
    let axis = |a0: f64, a1: f64, b0: f64, b1: f64| {
        // A box with no extent on this axis (a level or upright line) has nothing to scale there.
        let s = if a1 > a0 { (b1 - b0) / (a1 - a0) } else { 1.0 };
        if !s.is_finite() {
            return None;
        }
        let about = if (s - 1.0).abs() < 1e-9 { (a0 + a1) / 2.0 } else { (b0 - s * a0) / (1.0 - s) };
        Some((s, about))
    };
    let (sx, fx) = axis(start[0], start[2], now[0], now[2])?;
    let (sy, fy) = axis(start[1], start[3], now[1], now[3])?;
    if (sx - 1.0).abs() < 1e-9 && (sy - 1.0).abs() < 1e-9 {
        return None;
    }
    Some(([sx.clamp(MIN_SCALE, MAX_SCALE), sy.clamp(MIN_SCALE, MAX_SCALE)], [fx, fy]))
}

/// The box over the selected shapes, the scale a drag commits, and the outlines it proposes.
fn show_box(app: &mut PhotocraftApp, ui: &mut egui::Ui, rect: egui::Rect) {
    if !box_shows(app) {
        return;
    }
    let line = live_line(app);
    let (Some((layers, bounds)), Some(salt)) = (selection(app, line.is_some()), salt(app)) else { return };
    let start = DocRect::new(bounds[0], bounds[1], bounds[2], bounds[3]);
    let given = BoxState::from_rect(start);
    // Free mode with rotating, skewing and distorting refused: a corner scales by the drag's projection on
    // the diagonal and an edge one way (`ScaleRule::Shape`). The outside is the tool's, so the rotate zone is
    // the band that a refused rotation leaves undeclared.
    let rules = BoxRules::in_mode(TransformMode::Free).with_allow(BoxAllow::new(false, false, false)).with_scale(ScaleRule::Shape);
    // The Ellipse tool marks the box's centre; a live line shows its ends and midpoint instead of the frame.
    let mut transform_box = TransformBox::new(BOX_LABEL, Bind::given(given), start, 0.0)
        .rules(rules)
        .rotate_zone(RotateZone::Band)
        .reference_point(false)
        .inside_moves(false)
        .centre_mark(app.ui.tool == Tool::EllipseShape)
        .id_salt((BOX_LABEL, salt));
    if let Some(line) = line {
        transform_box = transform_box.line(line);
    }
    let out = transform_box.show_at(ui, rect);
    // Only this pass's events count: a second pass of the frame sees its own. The box has no session to apply
    // (`apply` and `discard` are ignored), and a key nudge moves nothing here because the box is re-given from
    // the document.
    if let Some(EditEvent::Commit { after, .. } | EditEvent::Set { after, .. }) = out.edit.events.ended() {
        commit_box(app, &layers, bounds, after);
        return;
    }
    let proposed = out.edit.shown.as_ref().and_then(|shown| shown.same()).map(|shown| **shown).filter(|shown| *shown != given);
    if let Some((scale, about)) = proposed.and_then(|p| box_scale(bounds, bounds_of(&p))) {
        let map = Affine { m: [scale[0], 0.0, 0.0, scale[1], about[0] * (1.0 - scale[0]), about[1] * (1.0 - scale[1])] };
        let paths = layers_paths(app, &layers, |path| path.transform(&map));
        SelectionOutline::solid(BOX_OUTLINE_LABEL, &paths).show_at(ui, rect);
    }
}

/// `edit.transform` for the box dragged from `bounds` to `after`: one step for every selected layer. A drag
/// that scaled nothing runs nothing.
fn commit_box(app: &mut PhotocraftApp, layers: &[u64], bounds: [f64; 4], after: &BoxState) {
    let Some((scale, reference)) = box_scale(bounds, bounds_of(after)) else { return };
    let params = json!({
        "layers": layers,
        "rect": bounds,
        "scaleX": scale[0] * 100.0,
        "scaleY": scale[1] * 100.0,
        "reference": reference,
        // Named by the command: with background jobs on, the step lands after `run` returns, so a rename
        // here would rename the step before it.
        "stepName": photocraft_engine::transform_cmds::TRANSFORM_PATH,
    });
    run_or_report(app, "edit.transform", params);
}

/// The outlines of `layers` (shape layers) after `map`, for `SelectionOutline`.
fn layers_paths(app: &PhotocraftApp, layers: &[u64], map: impl Fn(&Path) -> Path) -> Vec<BezPath> {
    let Some(st) = app.session.active() else { return Vec::new() };
    layers
        .iter()
        .filter_map(|id| match &st.doc.layer(LayerId(*id))?.content {
            LayerContent::Shape(sh) => Some(bez_path(&map(&sh.path))),
            _ => None,
        })
        .collect()
}

/// A document path as the framework's outline takes it.
fn bez_path(path: &Path) -> BezPath {
    let mut out = BezPath::new();
    let point = |p: photocraft_geom::Point| Point::new(p.x, p.y);
    for sub in &path.subpaths {
        let Some(first) = sub.knots.first() else { continue };
        out.move_to(point(first.anchor));
        let n = sub.knots.len();
        let segments = if sub.closed { n } else { n - 1 };
        for i in 0..segments {
            let (a, b) = (&sub.knots[i], &sub.knots[(i + 1) % n]);
            out.curve_to(point(a.out_ctrl), point(b.in_ctrl), point(b.anchor));
        }
        if sub.closed {
            out.close_path();
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// The corner radius widgets

/// Do the widgets show with the current tool?
fn corners_show(app: &PhotocraftApp) -> bool {
    app.ui.transform.is_none()
        && match app.ui.tool {
            Tool::PathSelection | Tool::Rectangle | Tool::Triangle | Tool::Polygon | Tool::Star => true,
            Tool::Move => app.ui.tool_options.move_show_transform,
            _ => false,
        }
}

/// `[x, y, w, h]` with a positive size, or `None` for an empty or non-finite one.
fn normal_rect([x, y, w, h]: [f64; 4]) -> Option<[f64; 4]> {
    let (x, w) = if w < 0.0 { (x + w, -w) } else { (x, w) };
    let (y, h) = if h < 0.0 { (y + h, -h) } else { (y, h) };
    (w > 0.0 && h > 0.0 && [x, y, w, h].iter().all(|v| v.is_finite())).then_some([x, y, w, h])
}

/// A shape layer's live shape as a widget target, if it has corners: a rectangle's four, each with its own
/// radius; a polygon's or star's outer points, or a triangle's three, sharing one.
fn corner_target(layer: u64, live: &LiveShape) -> Option<CornerTarget> {
    let at = |p: [f64; 2]| Point::new(p[0], p[1]);
    match *live {
        LiveShape::Rect { rect, radii } => {
            let [x, y, w, h] = normal_rect(rect)?;
            if !radii.iter().all(|r| r.is_finite()) {
                return None;
            }
            // Clockwise from the top left, as `radii` is.
            let v = [[x, y], [x + w, y], [x + w, y + h], [x, y + h]];
            let corners = (0..4).map(|i| CornerPoint::new(at(v[i]), at(v[(i + 3) % 4]), at(v[(i + 1) % 4]), radii[i], CornerKind::Round)).collect();
            Some(CornerTarget::new(layer, corners, true))
        }
        LiveShape::Polygon { rect, sides, star_ratio, corner_radius } => {
            let [x, y, w, h] = normal_rect(rect)?;
            if !(star_ratio.is_finite() && corner_radius.is_finite()) {
                return None;
            }
            let sharp = shapes::polygon(x, y, w, h, sides, star_ratio);
            let vertices: Vec<[f64; 2]> = sharp.subpaths.first()?.knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect();
            let n = vertices.len();
            // A star's inner points stay sharp, so they have no widget.
            let corners = (0..n)
                .filter(|&i| shapes::polygon_rounds_vertex(star_ratio, i))
                .map(|i| CornerPoint::new(at(vertices[i]), at(vertices[(i + n - 1) % n]), at(vertices[(i + 1) % n]), corner_radius.max(0.0), CornerKind::Round))
                .collect();
            Some(CornerTarget::new(layer, corners, false))
        }
        LiveShape::Triangle { rect, corner_radius } => {
            let [x, y, w, h] = normal_rect(rect)?;
            if !corner_radius.is_finite() {
                return None;
            }
            let sharp = shapes::triangle(x, y, w, h);
            let vertices: Vec<[f64; 2]> = sharp.subpaths.first()?.knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect();
            let n = vertices.len();
            let corners = (0..n)
                .map(|i| CornerPoint::new(at(vertices[i]), at(vertices[(i + n - 1) % n]), at(vertices[(i + 1) % n]), corner_radius.max(0.0), CornerKind::Round))
                .collect();
            Some(CornerTarget::new(layer, corners, false))
        }
        LiveShape::Ellipse { .. } | LiveShape::Line { .. } | LiveShape::ArrowLine { .. } => None,
    }
}

/// The selected layers that are live shapes with corners, front first: the widget of the topmost shape wins a
/// press where two reach it. Empty when the widgets do not show.
pub(crate) fn corner_targets(app: &PhotocraftApp) -> Vec<CornerTarget> {
    if !corners_show(app) {
        return Vec::new();
    }
    let Some(st) = app.session.active() else { return Vec::new() };
    let mut out: Vec<CornerTarget> = st
        .selected_layers()
        .into_iter()
        .filter_map(|id| {
            let l = st.doc.layer(id)?;
            let LayerContent::Shape(sh) = &l.content else { return None };
            corner_target(l.id.0, sh.live.as_ref()?)
        })
        .collect();
    // `selected_layers` lists bottom to top.
    out.reverse();
    out
}

/// The live shape `live` with the radii `radii` (one per widget corner), as a document path.
fn rounded_path(live: &LiveShape, radii: &[f64]) -> Option<Path> {
    match *live {
        LiveShape::Rect { rect, .. } => {
            let [x, y, w, h] = normal_rect(rect)?;
            let at = |i: usize| radii.get(i).copied().unwrap_or(0.0);
            Some(shapes::rounded_rect(x, y, w, h, [at(0), at(1), at(2), at(3)]))
        }
        LiveShape::Polygon { rect, sides, star_ratio, .. } => {
            let corner_radius = radii.first().copied().unwrap_or(0.0);
            Some(shapes::live_path(&LiveShape::Polygon { rect, sides, star_ratio, corner_radius }))
        }
        LiveShape::Triangle { rect, .. } => Some(shapes::live_path(&LiveShape::Triangle { rect, corner_radius: radii.first().copied().unwrap_or(0.0) })),
        LiveShape::Ellipse { .. } | LiveShape::Line { .. } | LiveShape::ArrowLine { .. } => None,
    }
}

/// The widgets of the selected live shapes, the radii a drag commits, and the outlines it proposes.
fn show_widgets(app: &mut PhotocraftApp, ui: &mut egui::Ui, rect: egui::Rect) {
    let targets = corner_targets(app);
    let Some(salt) = salt(app) else { return };
    if targets.is_empty() {
        return;
    }
    let out = CornerWidgets::new(CORNERS_LABEL, Bind::given(targets.clone())).id_salt((CORNERS_LABEL, salt)).show_at(ui, rect);
    // Only this pass's events count: a second pass of the frame sees its own, so a commit is run once.
    let key = match out.edit.events.ended() {
        Some(EditEvent::Commit { id, .. }) => Some(format!("corner-radius-{}", id.0)),
        Some(EditEvent::Set { .. }) => Some(format!("corner-radius-{}", app.ui.alloc_id())),
        _ => None,
    };
    if let Some(key) = key {
        commit_corners(app, &targets, out.change.as_ref(), &key);
        return;
    }
    let Some(shown) = out.edit.shown.as_ref().and_then(|shown| shown.same()) else { return };
    let Some(st) = app.session.active() else { return };
    let paths: Vec<BezPath> = shown
        .iter()
        .zip(&targets)
        .filter_map(|(now, was)| {
            let radii: Vec<f64> = now.corners.iter().map(|c| c.radius).collect();
            let before: Vec<f64> = was.corners.iter().map(|c| c.radius).collect();
            if radii == before {
                return None;
            }
            let LayerContent::Shape(sh) = &st.doc.layer(LayerId(now.key))?.content else { return None };
            rounded_path(sh.live.as_ref()?, &radii).map(|p| bez_path(&p))
        })
        .collect();
    if !paths.is_empty() {
        SelectionOutline::solid(CORNERS_OUTLINE_LABEL, &paths).show_at(ui, rect);
    }
}

/// One `shape.edit` per changed target, all under `key` so they are one History step: `radii` for a
/// rectangle, `cornerRadius` for a polygon or a triangle. A click without movement reports no change and runs nothing.
fn commit_corners(app: &mut PhotocraftApp, targets: &[CornerTarget], change: Option<&craft::draw::ui::corner::CornerEdit>, key: &str) {
    let Some(change) = change else { return };
    for edit in &change.targets {
        let Some(target) = targets.iter().find(|t| t.key == edit.key) else { continue };
        let radii = &edit.radii;
        let params = if target.individual {
            json!({"layer": edit.key, "radii": radii, "coalesce": key})
        } else {
            json!({"layer": edit.key, "cornerRadius": radii.first().copied().unwrap_or(0.0), "coalesce": key})
        };
        if !run_or_report(app, "shape.edit", params) {
            break;
        }
    }
}

/// Runs `command`; on an error the status line shows it. Whether it succeeded.
fn run_or_report(app: &mut PhotocraftApp, command: &str, params: serde_json::Value) -> bool {
    match app.run(command, params) {
        Ok(_) => true,
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
            false
        }
    }
}
