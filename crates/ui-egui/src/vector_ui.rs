//! Vector tools: Shape tools (U), Pen (P), Path Selection (A), path overlays and the Paths panel.
//! Direct Selection (A) lives in `direct_select`.
//! All edits go through the engine's `shape.*` / `path.*` commands.

use egui::{Color32, Pos2, Rect, Sense, Stroke, pos2, vec2};
use photocraft_doc::vector::Path;
use photocraft_doc::{Document, LayerContent};
use photocraft_geom::Affine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::state::{Tool, ToolOptions};
use crate::theme::Tokens;

/// An open path the Pen is continuing: commit replaces this subpath instead of writing a new path.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PenResume {
    /// `"work"` or `"layer"`.
    pub name: String,
    pub layer: Option<u64>,
    pub subpath: usize,
    /// Anchors already on the path. Undo will not drop below this.
    pub kept: usize,
}

/// Pen tool path under construction: knots as [anchor, in, out] (document px).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PenPath {
    pub knots: Vec<[[f64; 2]; 3]>,
    /// Indices of cusp knots whose incoming and outgoing handles are not linked.
    #[serde(default)]
    pub unlinked: Vec<usize>,
    /// Set while the next commit continues an existing open subpath.
    #[serde(default)]
    pub resume: Option<PenResume>,
    #[serde(skip)]
    pub dragging: bool,
    /// Re-dragging the final anchor changes its outgoing control only; the incoming curve stays put.
    #[serde(skip)]
    pub adjusting_last: bool,
}

pub fn is_shape_tool(t: Tool) -> bool {
    matches!(t, Tool::Rectangle | Tool::EllipseShape | Tool::Triangle | Tool::Polygon | Tool::Line | Tool::CustomShape)
}

fn rgb32(c: [f32; 4]) -> Color32 {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(b(c[0]), b(c[1]), b(c[2]))
}

fn hex(c: [f32; 4]) -> String {
    let c = rgb32(c);
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

fn stroke_param(app: &PhotocraftApp) -> Value {
    let o = &app.ui.tool_options;
    if o.stroke_width <= 0.0 {
        return Value::Null;
    }
    let mut p = o.shape_stroke.params();
    p["width"] = json!(o.stroke_width);
    p["color"] = json!(hex(app.session.tools.background));
    p
}

/// Geometry of a Shape-tool drag as `shape.create` params, without fill and stroke (None when too
/// small): ⇧ constrains proportions, ⌥ draws from the centre. Custom shapes give their box as a `rect`.
fn shape_geometry(o: &ToolOptions, tool: Tool, start: [f64; 2], end: [f64; 2], mods: egui::Modifiers) -> Option<Value> {
    Some(if tool == Tool::Line {
        let (mut dx, mut dy) = (end[0] - start[0], end[1] - start[1]);
        if mods.shift {
            // Snap to 45°.
            let a = (dy.atan2(dx) / std::f64::consts::FRAC_PI_4).round() * std::f64::consts::FRAC_PI_4;
            let len = dx.hypot(dy);
            (dx, dy) = (len * a.cos(), len * a.sin());
        }
        if dx.hypot(dy) < 1.0 {
            return None;
        }
        json!({"kind": "line", "from": start, "to": [start[0] + dx, start[1] + dy], "weight": o.line_weight.max(1.0)})
    } else {
        let (mut w, mut h) = (end[0] - start[0], end[1] - start[1]);
        if mods.shift {
            let m = w.abs().max(h.abs());
            (w, h) = (m * w.signum(), m * h.signum());
        }
        let (x0, y0) = if mods.alt { (start[0] - w, start[1] - h) } else { (start[0], start[1]) };
        let (w, h) = if mods.alt { (w * 2.0, h * 2.0) } else { (w, h) };
        let rect = [x0.min(x0 + w).round(), y0.min(y0 + h).round(), w.abs().round(), h.abs().round()];
        if rect[2] < 1.0 || rect[3] < 1.0 {
            return None;
        }
        match tool {
            Tool::Rectangle if o.corner_radius > 0.0 => json!({"kind": "roundedRect", "rect": rect, "radii": vec![o.corner_radius; 4]}),
            Tool::Rectangle | Tool::CustomShape => json!({"kind": "rect", "rect": rect}),
            Tool::EllipseShape => json!({"kind": "ellipse", "rect": rect}),
            Tool::Triangle => json!({"kind": "polygon", "rect": rect, "sides": 3}),
            _ => json!({"kind": "polygon", "rect": rect, "sides": o.polygon_sides.max(3)}),
        }
    })
}

/// Finish a Shape-tool drag: ⇧ constrains proportions, ⌥ draws from the centre.
pub fn finish_shape(app: &mut PhotocraftApp, tool: Tool, start: [f64; 2], end: [f64; 2], mods: egui::Modifiers) {
    let Some(mut p) = shape_geometry(&app.ui.tool_options, tool, start, end, mods) else { return };
    if tool == Tool::CustomShape {
        // ⇧ keeps the shape's proportions (the rect is already squared).
        if let Ok(rect) = serde_json::from_value(p["rect"].take()) {
            let (fill, stroke) = (fill_param(app), stroke_param(app));
            crate::preset_panels::finish_custom_shape(app, rect, mods.shift, fill, stroke);
        }
        return;
    }
    if let Err(e) = create_shape(app, p) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

fn fill_param(app: &PhotocraftApp) -> Value {
    if app.ui.tool_options.shape_fill { json!(hex(app.session.tools.foreground)) } else { Value::Null }
}

/// Create a shape layer from `shape.create` geometry with the options bar's fill and stroke: the
/// end of a drag, and the Create Rectangle / Ellipse / … dialogs (`shape_dialog`).
pub fn create_shape(app: &mut PhotocraftApp, mut geometry: Value) -> Result<Value, String> {
    geometry["fill"] = fill_param(app);
    geometry["stroke"] = stroke_param(app);
    app.run("shape.create", geometry)
}

/// Shape-tool drag preview using the same path and vector stroke geometry as the commit.
pub fn draw_shape_preview(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform, tool: Tool, start: [f64; 2], end: [f64; 2], mods: egui::Modifiers) {
    let o = &app.ui.tool_options;
    let Some(geometry) = shape_geometry(o, tool, start, end, mods) else { return };
    let path = if tool == Tool::CustomShape {
        let Some(rect) = geometry.get("rect").and_then(|r| serde_json::from_value::<[f64; 4]>(r.clone()).ok()) else { return };
        let groups = photocraft_engine::presets::shapes::all_groups(&app.session);
        let Some(shape) = groups.iter().flat_map(|g| &g.items).find(|s| s.name == app.ui.presets_ui.shape()) else { return };
        photocraft_engine::presets::shapes::fit(&shape.path, rect, mods.shift)
    } else {
        let Ok(path) = photocraft_engine::vector_cmds::shape_path(&geometry) else { return };
        path
    };
    // ponytail: preview colours skip the canvas's colour management; the commit renders them exactly.
    let fill = if o.shape_fill { rgb32(app.session.tools.foreground) } else { Color32::TRANSPARENT };
    {
        let origin = xf.to_screen(0.0, 0.0);
        let x = xf.to_screen(1.0, 0.0) - origin;
        let y = xf.to_screen(0.0, 1.0) - origin;
        let transform = Affine { m: [x.x as f64, x.y as f64, y.x as f64, y.y as f64, origin.x as f64, origin.y as f64] };
        let c = app.session.tools.background;
        let shape = photocraft_doc::ShapeLayer {
            path: path.transform(&transform),
            fill: (fill != Color32::TRANSPARENT).then(|| {
                photocraft_doc::Fill::Solid(photocraft_doc::Color::rgba(fill.r() as f32 / 255.0, fill.g() as f32 / 255.0, fill.b() as f32 / 255.0, 1.0))
            }),
            stroke: (o.stroke_width > 0.0)
                .then(|| o.shape_stroke.stroke(o.stroke_width * xf.zoom, photocraft_doc::Fill::Solid(photocraft_doc::Color::rgba(c[0], c[1], c[2], c[3])))),
            ..Default::default()
        };
        crate::shape_stroke_ui::paint_shape_preview(painter, &shape);
    }
    let accent = Tokens::get(painter.ctx()).accent;
    for (pts, _) in path_lines(&path, &|q| xf.to_screen(q[0] as f32, q[1] as f32)) {
        // The accent path over a dark halo: visible on any pixels (#172).
        painter.extend(crate::tool_feedback::contrast_path(pts, true, accent));
    }
}

// ---------------------------------------------------------------------------------------------
// Pen

fn pen_to_json(pen: &PenPath, closed: bool) -> Value {
    json!({"subpaths": [{"closed": closed, "knots": pen.knots.iter().enumerate().map(|(i, k)| json!({"anchor": k[0], "in": k[1], "out": k[2], "smooth": !pen.unlinked.contains(&i) && (k[1] != k[0] || k[2] != k[0])})).collect::<Vec<_>>()}]})
}

/// ⇧ locks a Pen point onto the nearest 45° line through `from` (document space).
/// A pointer left or right of the last anchor stays on that horizontal, at the pointer's
/// position along the line; the same rule picks vertical and the diagonals.
pub(crate) fn shift_locked_point(from: [f64; 2], point: [f64; 2], shift: bool) -> [f64; 2] {
    if shift { crate::stroke_constraint::snap45(from, point) } else { point }
}

/// Pen press: close on the first anchor, reshape the last anchor's outgoing handle,
/// or add a new anchor (dragging a new anchor pulls symmetrical handles).
/// ⇧ constrains a new anchor to 45° from the previous one. Closing and re-clicking the
/// last anchor still use the raw pointer, so ⇧ does not slide those hits off the anchor.
pub fn pen_down(app: &mut PhotocraftApp, x: f64, y: f64, shift: bool) {
    let tol = 6.0 / app.point_zoom().max(0.01) as f64;
    let pen = app.ui.pen.get_or_insert_with(PenPath::default);
    if let Some(first) = pen.knots.first().map(|k| k[0])
        && pen.knots.len() >= 2
        && (first[0] - x).hypot(first[1] - y) < tol
    {
        pen_commit(app, true);
        return;
    }
    // Clicking the last anchor breaks its outgoing handle without throwing away the incoming
    // curve. Dragging from that anchor then sets the outgoing handle independently (#1482).
    // This must precede adding a point, or a re-click creates a zero-length segment.
    if let Some((i, last)) = pen.knots.len().checked_sub(1).zip(pen.knots.last_mut())
        && (last[0][0] - x).hypot(last[0][1] - y) < tol
    {
        last[2] = last[0];
        if !pen.unlinked.contains(&i) {
            pen.unlinked.push(i);
        }
        pen.dragging = true;
        pen.adjusting_last = true;
        return;
    }
    let [x, y] = pen.knots.last().map(|k| shift_locked_point(k[0], [x, y], shift)).unwrap_or([x, y]);
    pen.knots.push([[x, y]; 3]);
    pen.dragging = true;
    pen.adjusting_last = false;
}

pub fn pen_move(app: &mut PhotocraftApp, x: f64, y: f64, shift: bool) {
    if let Some(pen) = app.ui.pen.as_mut()
        && pen.dragging
    {
        let adjusting_last = pen.adjusting_last;
        let last_i = pen.knots.len().saturating_sub(1);
        let unlinked = pen.unlinked.contains(&last_i);
        let mut mark_corner = false;
        if let Some(k) = pen.knots.last_mut() {
            let a = k[0];
            let [x, y] = shift_locked_point(a, [x, y], shift);
            // A smooth endpoint keeps its handles collinear. A corner (or a cusp made by
            // re-clicking the last anchor) only moves the outgoing handle, so the previous
            // segment stays put.
            let was_smooth = !unlinked && (k[1] != a || k[2] != a);
            k[2] = [x, y];
            if !adjusting_last || was_smooth {
                k[1] = [2.0 * a[0] - x, 2.0 * a[1] - y];
            } else {
                mark_corner = true;
            }
        }
        if mark_corner && !pen.unlinked.contains(&last_i) {
            pen.unlinked.push(last_i);
        }
    }
}

/// Undo the most recently placed Pen anchor while its path is still in progress.
/// These knots are editor-only gesture state, not document history until Enter or
/// clicking the first anchor commits the path. Therefore Cmd/Ctrl+Z must consume one
/// knot rather than undoing an unrelated, already committed document operation.
pub fn pen_undo_last_point(app: &mut PhotocraftApp) -> bool {
    let Some(pen) = app.ui.pen.as_mut() else { return false };
    if pen.resume.as_ref().is_some_and(|r| pen.knots.len() <= r.kept) {
        return false;
    }
    if pen.knots.pop().is_none() {
        return false;
    }
    pen.dragging = false;
    if pen.knots.is_empty() {
        app.ui.pen = None;
    }
    true
}

pub fn pen_up(app: &mut PhotocraftApp) {
    if let Some(pen) = app.ui.pen.as_mut() {
        pen.dragging = false;
        pen.adjusting_last = false;
    }
}

/// Paths the idle Pen can continue or add a point to, front first: the active shape or targeted
/// vector mask, then the work path.
fn placed_paths(app: &PhotocraftApp) -> Vec<(String, Option<u64>, Path)> {
    let layer = targeted_vector_mask(app).or_else(|| active_shape_path(app));
    let work = app.session.active().and_then(|st| st.doc.work_path.clone());
    layer.map(|(id, p)| ("layer".into(), Some(id), p)).into_iter().chain(work.map(|p| ("work".into(), None, p))).collect()
}

fn placed_path(app: &PhotocraftApp, name: &str, layer: Option<u64>) -> Option<Path> {
    placed_paths(app).into_iter().find(|(n, id, _)| n == name && *id == layer).map(|(_, _, p)| p)
}

/// Knots of an existing subpath, as the Pen stores them. Continuing from the first anchor
/// reverses the subpath so new points extend from that end.
fn load_knots(knots: &[photocraft_doc::Knot], from_start: bool) -> (Vec<[[f64; 2]; 3]>, Vec<usize>) {
    let mut ks = knots.to_vec();
    if from_start && ks.len() > 1 {
        for k in &mut ks {
            std::mem::swap(&mut k.in_ctrl, &mut k.out_ctrl);
        }
        ks.reverse();
    }
    let mut unlinked = Vec::new();
    let triples = ks
        .iter()
        .enumerate()
        .map(|(i, k)| {
            let on = |p: photocraft_geom::Point| (p.x - k.anchor.x).abs() < 1e-9 && (p.y - k.anchor.y).abs() < 1e-9;
            if !k.smooth && !(on(k.in_ctrl) && on(k.out_ctrl)) {
                unlinked.push(i);
            }
            [[k.anchor.x, k.anchor.y], [k.in_ctrl.x, k.in_ctrl.y], [k.out_ctrl.x, k.out_ctrl.y]]
        })
        .collect();
    (triples, unlinked)
}

fn run_path_edit(app: &mut PhotocraftApp, id: &str, mut params: Value, name: &str, layer: Option<u64>) -> Result<(), String> {
    if let (Some(obj), Some(id_layer)) = (params.as_object_mut(), layer) {
        obj.insert("name".into(), json!(name));
        obj.insert("layer".into(), json!(id_layer));
    } else if let Some(obj) = params.as_object_mut() {
        obj.insert("name".into(), json!(name));
    }
    app.run(id, params).map(|_| ())
}

/// Idle Pen on a path that is already placed: click an open end to continue it, click a segment
/// to add an anchor there, click any other anchor to remove it. False when the click missed
/// every placed path, so the caller starts a new one.
pub fn pen_edit_existing(app: &mut PhotocraftApp, x: f64, y: f64) -> bool {
    if app.ui.pen.is_some() || !x.is_finite() || !y.is_finite() {
        return false;
    }
    let tol = 6.0 / f64::from(app.point_zoom().max(0.01));
    let found = placed_paths(app)
        .into_iter()
        .find_map(|(name, layer, path)| photocraft_vector::edit::hit(&path, photocraft_geom::Point::new(x, y), tol, &[]).map(|h| (name, layer, path, h)));
    let Some((name, layer, path, hit)) = found else { return false };
    match hit {
        photocraft_vector::edit::Hit::Anchor([s, k]) if photocraft_vector::edit::is_endpoint(&path, [s, k]) => {
            let Some(sp) = path.subpaths.get(s) else { return false };
            let (knots, unlinked) = load_knots(&sp.knots, k == 0 && sp.knots.len() > 1);
            let kept = knots.len();
            app.ui.pen = Some(PenPath { knots, unlinked, resume: Some(PenResume { name, layer, subpath: s, kept }), dragging: true, adjusting_last: true });
            true
        }
        photocraft_vector::edit::Hit::Anchor([s, k]) => {
            if let Err(e) = run_path_edit(app, "path.deleteAnchor", json!({"subpath": s, "knot": k}), &name, layer) {
                app.ui.status = e;
                app.ui.status_error = true;
            }
            true
        }
        photocraft_vector::edit::Hit::Segment([s, k], t) => {
            if let Err(e) = run_path_edit(app, "path.addAnchor", json!({"subpath": s, "knot": k, "t": t}), &name, layer) {
                app.ui.status = e;
                app.ui.status_error = true;
            }
            true
        }
        photocraft_vector::edit::Hit::Handle(..) => true,
    }
}

/// The mark drawn beside the pointer while a path tool hovers something a press would edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathCursor {
    /// Open end: continue the path. First anchor of a path being drawn: close it.
    Continue,
    /// An anchor that is not an end: delete it.
    Remove,
    /// A segment: insert an anchor.
    Add,
    /// A handle, a convert-point press, or the last anchor of a path being drawn: curve it.
    Curve,
    /// An anchor (or a whole path) that a press would move.
    Move,
}

impl PathCursor {
    /// Icon name for [`crate::icons::cursor_badge`].
    pub fn icon(self) -> &'static str {
        match self {
            PathCursor::Continue => "circle",
            PathCursor::Remove => "minus",
            PathCursor::Add => "plus",
            PathCursor::Curve => "pen-curve",
            PathCursor::Move => "move",
        }
    }
}

/// Hover cursor for the Pen, Direct Selection and Path Selection. `None` over empty canvas.
pub fn path_cursor(app: &PhotocraftApp, p: [f64; 2], mods: egui::Modifiers) -> Option<PathCursor> {
    if !p[0].is_finite() || !p[1].is_finite() {
        return None;
    }
    let tool = app.ui.tool;
    let pen = tool == Tool::Pen;
    let direct = tool == Tool::DirectSelection || (pen && mods.command);
    let selecting = tool == Tool::PathSelection;
    if !pen && !direct && !selecting {
        return None;
    }
    let tol = 6.0 / f64::from(app.point_zoom().max(0.01));
    if direct || selecting {
        if let Some(hit) = crate::direct_select::cursor_hit(app, p) {
            return Some(match hit {
                photocraft_vector::edit::Hit::Handle(..) => PathCursor::Curve,
                photocraft_vector::edit::Hit::Anchor(_) => PathCursor::Move,
                photocraft_vector::edit::Hit::Segment(..) if selecting && !direct => PathCursor::Move,
                photocraft_vector::edit::Hit::Segment(..) => PathCursor::Curve,
            });
        }
        if pen && mods.command {
            return pen_anchor_at(app.ui.pen.as_ref()?, p, tol).map(|_| PathCursor::Move);
        }
        return None;
    }
    if let Some(pen_path) = app.ui.pen.as_ref() {
        let i = pen_anchor_at(pen_path, p, tol)?;
        let n = pen_path.knots.len();
        if i + 1 == n {
            return Some(PathCursor::Curve);
        }
        if i == 0 && n >= 2 {
            return Some(PathCursor::Continue);
        }
        return None;
    }
    let found = placed_paths(app)
        .into_iter()
        .find_map(|(_, _, path)| photocraft_vector::edit::hit(&path, photocraft_geom::Point::new(p[0], p[1]), tol, &[]).map(|h| (path, h)));
    let Some((path, hit)) = found else { return None };
    let convert = mods.alt && !mods.command;
    match hit {
        photocraft_vector::edit::Hit::Anchor(_) if convert => Some(PathCursor::Curve),
        photocraft_vector::edit::Hit::Anchor(k) if photocraft_vector::edit::is_endpoint(&path, k) => Some(PathCursor::Continue),
        photocraft_vector::edit::Hit::Anchor(_) => Some(PathCursor::Remove),
        photocraft_vector::edit::Hit::Segment(..) if convert => None,
        photocraft_vector::edit::Hit::Segment(..) => Some(PathCursor::Add),
        photocraft_vector::edit::Hit::Handle(..) => None,
    }
}

/// Index of the in-progress anchor nearest `p`, within `tol`.
fn pen_anchor_at(pen: &PenPath, p: [f64; 2], tol: f64) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (i, k) in pen.knots.iter().enumerate() {
        let d = (k[0][0] - p[0]).hypot(k[0][1] - p[1]);
        if d <= tol && best.is_none_or(|(_, bd)| d < bd) {
            best = Some((i, d));
        }
    }
    best.map(|(i, _)| i)
}

fn near_point(p: photocraft_geom::Point, q: [f64; 2]) -> bool {
    (p.x - q[0]).abs() < 1e-6 && (p.y - q[1]).abs() < 1e-6
}

/// True when continuing did not move a handle or add a point, so commit must not record a step.
/// Continuing from the first anchor reverses the stored order; that alone is not a change.
fn resume_unchanged(sp: &photocraft_doc::Subpath, pen: &PenPath, closed: bool) -> bool {
    if sp.closed != closed || sp.knots.len() != pen.knots.len() || sp.knots.is_empty() {
        return false;
    }
    let from_start = sp.knots.len() > 1
        && pen.knots.last().is_some_and(|k| near_point(sp.knots[0].anchor, k[0]))
        && pen.knots.first().is_some_and(|k| sp.knots.last().is_some_and(|end| near_point(end.anchor, k[0])));
    let (loaded, unlinked) = load_knots(&sp.knots, from_start);
    pen.knots == loaded && pen.unlinked == unlinked
}

fn commit_resumed(app: &mut PhotocraftApp, pen: PenPath, closed: bool) {
    let Some(resume) = pen.resume.clone() else { return };
    if placed_path(app, &resume.name, resume.layer).and_then(|p| p.subpaths.get(resume.subpath).cloned()).is_some_and(|sp| resume_unchanged(&sp, &pen, closed))
    {
        return;
    }
    let knots = pen_to_json(&pen, closed)["subpaths"][0]["knots"].clone();
    if let Err(e) = run_path_edit(app, "path.replaceSubpath", json!({"subpath": resume.subpath, "closed": closed, "knots": knots}), &resume.name, resume.layer)
    {
        app.ui.pen = Some(pen);
        app.ui.status = e;
        app.ui.status_error = true;
    } else if resume.name == "work" {
        app.ui.selected_path = Some("work".into());
    }
}

/// Finish the pen path: a work path (Path mode) or a new shape layer (Shape mode).
/// A path continued from an existing open end is written back into that subpath.
pub fn pen_commit(app: &mut PhotocraftApp, closed: bool) {
    let Some(pen) = app.ui.pen.take() else { return };
    if pen.knots.len() < 2 {
        return;
    }
    if pen.resume.is_some() {
        commit_resumed(app, pen, closed);
        return;
    }
    let path = pen_to_json(&pen, closed);
    let r = if app.ui.tool_options.vector_mode == "shape" {
        let fill = if closed && app.ui.tool_options.shape_fill { json!(hex(app.session.tools.foreground)) } else { Value::Null };
        let stroke = if closed {
            stroke_param(app)
        } else {
            let mut stroke = app.ui.tool_options.shape_stroke.params();
            stroke["width"] = json!(app.ui.tool_options.stroke_width.max(1.0));
            stroke["color"] = json!(hex(app.session.tools.foreground));
            stroke
        };
        app.run("shape.create", json!({"kind": "path", "path": path, "fill": fill, "stroke": stroke}))
    } else if let Some((id, existing)) = targeted_vector_mask(app) {
        // A targeted vector mask takes the new subpath (#196), as in Photoshop.
        let mut p = photocraft_engine::vector_cmds::path_json(&existing);
        if let (Some(subs), Some(new)) = (p.get_mut("subpaths").and_then(Value::as_array_mut), path.get("subpaths").and_then(Value::as_array)) {
            subs.extend(new.iter().cloned());
        }
        app.run("layer.vectorMask.edit", json!({"layer": id, "path": p}))
    } else {
        // The drawn work path is selected in the Paths panel, as in Photoshop.
        app.run("path.set", json!({"name": "work", "path": path})).inspect(|_| app.ui.selected_path = Some("work".into()))
    };
    if let Err(e) = r {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// The path that path commands act on without a name: the one selected in the Paths panel, else
/// the work path, else the selected layer's shape path or vector mask; `None` when there's none.
pub fn active_path_name(app: &PhotocraftApp) -> Option<String> {
    let st = app.session.active()?;
    let rows = path_rows(&st.doc, st.active_layer);
    let key = |r: &PathEntry| match r.kind {
        PathRow::Work => "work".to_string(),
        PathRow::Layer => "layer".to_string(),
        PathRow::Saved => r.name.clone(),
    };
    let selected = app.ui.selected_path.as_deref().filter(|s| rows.iter().any(|r| key(r) == *s)).map(str::to_string);
    selected.or_else(|| rows.iter().find(|r| r.kind == PathRow::Work).map(key)).or_else(|| rows.iter().find(|r| r.kind == PathRow::Layer).map(key))
}

/// The commands that create a fill or adjustment layer and take a `"path"` as its vector mask.
pub fn takes_path_mask(id: &str) -> bool {
    id.starts_with("layer.newFillLayer.") || id.starts_with("layer.newAdjustmentLayer.")
}

/// New fill and adjustment layers take the path selected in the Paths panel as their vector mask,
/// as in Photoshop (#1419): the command gets it as `"path"` unless the caller set one (`null`
/// opts out). Only an explicitly selected work or saved path counts; deselect it by clicking the
/// panel's empty area.
pub fn with_active_path(app: &PhotocraftApp, id: &str, params: Value) -> Value {
    if !takes_path_mask(id) || params.get("path").is_some() {
        return params;
    }
    let Some(st) = app.session.active() else { return params };
    let Some(sel) = app.ui.selected_path.as_deref() else { return params };
    let listed = path_rows(&st.doc, st.active_layer).iter().any(|r| match r.kind {
        PathRow::Work => sel == "work",
        PathRow::Saved => r.name == sel,
        PathRow::Layer => false,
    });
    if !listed {
        return params;
    }
    match params {
        Value::Object(mut m) => {
            m.insert("path".into(), json!(sel));
            Value::Object(m)
        }
        _ => json!({ "path": sel }),
    }
}

/// ⌘↩ / Ctrl+Enter (#306): load a path as a selection, like Photoshop with a Pen or Path
/// Selection tool or a path selected in the Paths panel. A path being drawn with the Pen is
/// finished first and loads; otherwise [`active_path_name`]. `params` may set feather, mode…
pub fn path_to_selection(app: &mut PhotocraftApp, params: Value) -> Result<Value, String> {
    if app.ui.pen.as_ref().is_some_and(|p| p.knots.len() >= 2) {
        // The finished path becomes the work path, or a shape layer's path or a vector mask.
        let on_layer = app.ui.tool_options.vector_mode == "shape" || targeted_vector_mask(app).is_some();
        pen_commit(app, false);
        app.ui.selected_path = Some(if on_layer { "layer" } else { "work" }.into());
    }
    let name = active_path_name(app)
        .ok_or_else(|| tl!("No path to make a selection from: draw one with the Pen tool or select one in the Paths panel.").to_string())?;
    let mut p = params.as_object().cloned().unwrap_or_default();
    p.insert("name".into(), json!(name));
    app.run("path.toSelection", Value::Object(p))
}

// ---------------------------------------------------------------------------------------------
// Path Selection

/// What Path Selection edits.
enum PathTarget {
    Shape(u64),
    /// The active layer's vector mask, when its Layers thumbnail is targeted (#196).
    VectorMask(u64),
    Work,
}

/// The targeted vector mask of the active layer, if the Layers panel targets it.
pub(crate) fn targeted_vector_mask(app: &PhotocraftApp) -> Option<(u64, Path)> {
    let st = app.session.active()?;
    let l = st.active_layer.and_then(|id| st.doc.layer(id))?;
    (app.ui.vector_mask_target && !matches!(l.content, LayerContent::Shape(_))).then_some(())?;
    l.vector_mask.as_ref().map(|m| (l.id.0, m.path.clone()))
}

/// The active shape layer's path, if the active layer is a shape layer.
pub(crate) fn active_shape_path(app: &PhotocraftApp) -> Option<(u64, Path)> {
    let st = app.session.active()?;
    let l = st.active_layer.and_then(|id| st.doc.layer(id))?;
    match &l.content {
        LayerContent::Shape(sh) => Some((l.id.0, sh.path.clone())),
        _ => None,
    }
}

/// The path Path Selection edits: the targeted vector mask, the active shape layer's path, else
/// the work path.
fn target_path(app: &PhotocraftApp) -> Option<(PathTarget, Path)> {
    if let Some((id, p)) = targeted_vector_mask(app) {
        return Some((PathTarget::VectorMask(id), p));
    }
    if let Some((id, p)) = active_shape_path(app) {
        return Some((PathTarget::Shape(id), p));
    }
    app.session.active()?.doc.work_path.clone().map(|p| (PathTarget::Work, p))
}

/// Edit › Free Transform Path (⌘T with Path Selection or Direct Selection): the path the
/// box transforms (the one Path Selection edits) and `path.transform`'s params that name it.
pub(crate) fn free_transform_path(app: &PhotocraftApp) -> Option<(Value, Path)> {
    if !matches!(app.ui.tool, Tool::PathSelection | Tool::DirectSelection) {
        return None;
    }
    let (target, path) = target_path(app)?;
    path.control_bounds()?;
    let params = match target {
        PathTarget::Shape(id) | PathTarget::VectorMask(id) => json!({"name": "layer", "layer": id}),
        PathTarget::Work => json!({"name": "work"}),
    };
    Some((params, path))
}

pub fn path_selection_finish(app: &mut PhotocraftApp, start: [f64; 2], end: [f64; 2]) {
    let (dx, dy) = (end[0] - start[0], end[1] - start[1]);
    if dx.abs() + dy.abs() < 0.5 {
        return;
    }
    let Some((target, path)) = target_path(app) else { return };
    let _ = match target {
        PathTarget::Shape(id) => app.run("shape.edit", json!({"layer": id, "move": [dx.round(), dy.round()]})),
        PathTarget::VectorMask(id) => {
            let moved = path.transform(&Affine::translate(dx.round(), dy.round()));
            app.run("layer.vectorMask.edit", json!({"layer": id, "path": photocraft_engine::vector_cmds::path_json(&moved)}))
        }
        PathTarget::Work => {
            let moved = path.transform(&Affine::translate(dx.round(), dy.round()));
            app.run("path.set", json!({"name": "work", "path": photocraft_engine::vector_cmds::path_json(&moved)}))
        }
    };
}

// ---------------------------------------------------------------------------------------------
// Drawing

fn bezier(p0: [f64; 2], c0: [f64; 2], c1: [f64; 2], p1: [f64; 2], t: f64) -> [f64; 2] {
    let u = 1.0 - t;
    let f = |a: f64, b: f64, c: f64, d: f64| u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d;
    [f(p0[0], c0[0], c1[0], p1[0]), f(p0[1], c0[1], c1[1], p1[1])]
}

/// Screen polylines for a path (one per subpath).
pub(crate) fn path_lines(path: &Path, xf: &dyn Fn([f64; 2]) -> Pos2) -> Vec<(Vec<Pos2>, bool)> {
    path.subpaths
        .iter()
        .map(|s| {
            let n = s.knots.len();
            let mut pts = Vec::new();
            let segs = if s.closed { n } else { n.saturating_sub(1) };
            for i in 0..segs {
                let (a, b) = (&s.knots[i], &s.knots[(i + 1) % n]);
                for k in 0..=16 {
                    let q =
                        bezier([a.anchor.x, a.anchor.y], [a.out_ctrl.x, a.out_ctrl.y], [b.in_ctrl.x, b.in_ctrl.y], [b.anchor.x, b.anchor.y], k as f64 / 16.0);
                    pts.push(xf(q));
                }
            }
            if pts.is_empty()
                && let Some(k) = s.knots.first()
            {
                pts.push(xf([k.anchor.x, k.anchor.y]));
            }
            (pts, s.closed)
        })
        .collect()
}

/// A path's outline in the accent colour.
pub(crate) fn draw_outline(painter: &egui::Painter, p: &Path, to_scr: &dyn Fn([f64; 2]) -> Pos2, accent: Color32) {
    for (pts, closed) in path_lines(p, to_scr) {
        if closed {
            painter.add(egui::Shape::closed_line(pts, Stroke::new(1.0, accent)));
        } else {
            painter.add(egui::Shape::line(pts, Stroke::new(1.0, accent)));
        }
    }
}

/// An anchor point's square: filled with the accent when selected, else hollow (white).
pub(crate) fn draw_anchor(painter: &egui::Painter, c: Pos2, selected: bool, accent: Color32) {
    let r = Rect::from_center_size(c, vec2(6.0, 6.0));
    painter.rect_filled(r, 0.0, if selected { accent } else { Color32::WHITE });
    painter.rect_stroke(r, 0.0, Stroke::new(1.0, accent), egui::StrokeKind::Inside);
}

/// A direction handle: the line from its anchor and the round handle end.
pub(crate) fn draw_handle(painter: &egui::Painter, anchor: Pos2, handle: Pos2, accent: Color32) {
    painter.line_segment([anchor, handle], Stroke::new(1.0, accent));
    painter.circle_filled(handle, 3.0, accent);
}

/// Work path / active shape path outlines, anchors, and the pen path in progress.
pub fn draw_overlay(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform, doc: &Document) {
    let tool = app.ui.tool;
    let vector_tool = matches!(tool, Tool::Pen | Tool::PathSelection) || is_shape_tool(tool);
    let accent = Tokens::get(painter.ctx()).accent;
    let to_scr = |q: [f64; 2]| xf.to_screen(q[0] as f32, q[1] as f32);
    let draw_path = |p: &Path, anchors: bool, name: &str, layer: Option<u64>| {
        let hidden = app.ui.pen.as_ref().and_then(|pen| pen.resume.as_ref()).filter(|r| r.name == name && r.layer == layer).map(|r| r.subpath);
        let owned;
        let p = if let Some(i) = hidden {
            owned = {
                let mut q = p.clone();
                if let Some(sp) = q.subpaths.get_mut(i) {
                    sp.knots.clear();
                }
                q
            };
            &owned
        } else {
            p
        };
        draw_outline(painter, p, &to_scr, accent);
        if anchors {
            for k in p.subpaths.iter().flat_map(|s| &s.knots) {
                draw_anchor(painter, to_scr([k.anchor.x, k.anchor.y]), false, accent);
            }
        }
    };
    // View › Show › Target Path (under Extras) hides the paths; the Pen's path in progress stays.
    // Free Transform Path draws the path through its box instead (transform_tool).
    let paths = app.ui.view.shows(app.ui.view.show.target_path) && !app.ui.transform.as_ref().is_some_and(|t| t.path.is_some());
    if crate::direct_select::shows(app, painter.ctx().input(|i| i.modifiers)) {
        // Direct Selection (or the Pen with ⌘/Ctrl held) draws the paths it edits (#790).
        crate::direct_select::draw_overlay(app, painter, &to_scr, accent, paths);
    } else if paths {
        if vector_tool {
            // The Pen shows anchors too: an inserted point on a straight side does not move the outline.
            let anchors = matches!(tool, Tool::Pen | Tool::PathSelection);
            if let Some(wp) = &doc.work_path {
                draw_path(wp, anchors, "work", None);
            }
            let st = app.session.active();
            if let Some(l) = st.and_then(|s| s.active_layer.and_then(|id| s.doc.layer(id)))
                && let LayerContent::Shape(sh) = &l.content
            {
                draw_path(&sh.path, anchors, "layer", Some(l.id.0));
            }
        }
        // A targeted vector mask shows its path with any tool (#196).
        if let Some((id, p)) = targeted_vector_mask(app) {
            draw_path(&p, matches!(tool, Tool::Pen | Tool::PathSelection), "layer", Some(id));
        }
    }
    // Pen path in progress, with handles of the last knot and a rubber band to the pointer.
    if let Some(pen) = &app.ui.pen {
        let mut pts = Vec::new();
        for w in pen.knots.windows(2) {
            for k in 0..=16 {
                pts.push(to_scr(bezier(w[0][0], w[0][2], w[1][1], w[1][0], k as f64 / 16.0)));
            }
        }
        painter.add(egui::Shape::line(pts, Stroke::new(1.5, accent)));
        if let (Some(last), Some(h)) = (pen.knots.last(), app.hover_doc)
            && !pen.dragging
        {
            let shift = painter.ctx().input(|i| i.modifiers.shift);
            let end = shift_locked_point(last[0], h, shift);
            painter.add(egui::Shape::dashed_line(&[to_scr(last[0]), to_scr(end)], Stroke::new(1.0, accent), 4.0, 3.0));
        }
        for (i, k) in pen.knots.iter().enumerate() {
            let c = to_scr(k[0]);
            let last = i + 1 == pen.knots.len();
            if last && k[2] != k[0] {
                for hnd in [k[1], k[2]] {
                    draw_handle(painter, c, to_scr(hnd), accent);
                }
            }
            draw_anchor(painter, c, last, accent);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Options bars

/// Options bar for vector tools; false for other tools.
pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool) -> bool {
    if !(is_shape_tool(tool) || matches!(tool, Tool::Pen | Tool::PathSelection | Tool::DirectSelection)) {
        return false;
    }
    let t = Tokens::get(ui.ctx());
    let lbl = |ui: &mut egui::Ui, s: &str| {
        ui.label(egui::RichText::new(s).color(t.text_dim).size(12.0));
    };
    let o = &mut app.ui.tool_options;
    if tool == Tool::PathSelection {
        lbl(ui, if app.ui.vector_mask_target { "Drag to move the targeted vector mask" } else { tl!("Drag to move the active shape's path or the Work Path") });
        return true;
    }
    if tool == Tool::DirectSelection {
        let (shift, alt) = (crate::shortcuts::pretty("Shift"), crate::shortcuts::pretty("Alt"));
        lbl(
            ui,
            &crate::i18n::fmt(
                tl!("Click: select anchor · Drag: move anchor, handle or segment · {shift}-click: add · {alt}-click: whole subpath"),
                &[("shift", &shift), ("alt", &alt)],
            ),
        );
        return true;
    }
    if tool == Tool::Pen {
        let opts = [("path".to_string(), tl!("Path")), ("shape".to_string(), tl!("Shape"))];
        crate::widgets::dropdown(ui, "pen-mode", &mut o.vector_mode, &opts, 80.0);
        crate::widgets::vline(ui, 22.0);
        if o.vector_mode == "shape" {
            crate::widgets::value_field(ui, &mut o.stroke_width, 0.0..=288.0, "px", 58.0);
            if let Some(style) = crate::shape_stroke_ui::button(ui, "pen-stroke", &o.shape_stroke, &mut app.ui.stroke_editor, false, true, None) {
                o.shape_stroke = style;
            }
        }
        lbl(
            ui,
            &crate::i18n::fmt(
                tl!("Click: corner · Drag: smooth · {shift}: 45° · Click end: continue · Click segment: add · {key} finish · Esc cancel"),
                &[("shift", &crate::shortcuts::pretty("Shift")), ("key", &crate::shortcuts::pretty("Enter"))],
            ),
        );
        return true;
    }
    let mut mode = "shape".to_string();
    crate::widgets::dropdown(ui, "shape-mode", &mut mode, &[("shape".to_string(), tl!("Shape"))], 80.0);
    crate::widgets::vline(ui, 22.0);
    lbl(ui, tl!("Fill:"));
    crate::widgets::checkbox(ui, &mut o.shape_fill, "");
    let (r, _) = ui.allocate_exact_size(vec2(22.0, 16.0), Sense::hover());
    ui.painter().rect_filled(r, 2.0, rgb32(app.session.tools.foreground));
    ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, t.field_border), egui::StrokeKind::Outside);
    lbl(ui, tl!("Stroke:"));
    let bg = app.session.tools.background;
    let stroke_color = swatch(ui, Some(&photocraft_doc::Fill::Solid(photocraft_doc::Color::rgba(bg[0], bg[1], bg[2], bg[3]))), tl!("Stroke color"));
    crate::widgets::value_field(ui, &mut o.stroke_width, 0.0..=288.0, "px", 58.0);
    if let Some(style) = crate::shape_stroke_ui::button(ui, "tool-stroke", &o.shape_stroke, &mut app.ui.stroke_editor, true, tool != Tool::EllipseShape, None) {
        o.shape_stroke = style;
    }
    match tool {
        Tool::Rectangle => {
            crate::widgets::vline(ui, 22.0);
            lbl(ui, tl!("Radius:"));
            crate::widgets::value_field(ui, &mut o.corner_radius, 0.0..=10000.0, "px", 62.0);
        }
        Tool::Polygon => {
            crate::widgets::vline(ui, 22.0);
            lbl(ui, tl!("Sides:"));
            let mut s = o.polygon_sides as f32;
            if crate::widgets::value_field(ui, &mut s, 3.0..=100.0, "", 48.0).changed() {
                o.polygon_sides = s.round().clamp(3.0, 100.0) as u32;
            }
        }
        Tool::Line => {
            crate::widgets::vline(ui, 22.0);
            lbl(ui, tl!("Weight:"));
            crate::widgets::value_field(ui, &mut o.line_weight, 1.0..=1000.0, "px", 58.0);
        }
        Tool::CustomShape => {
            crate::widgets::vline(ui, 22.0);
            crate::preset_panels::shape_picker(app, ui);
        }
        _ => {}
    }
    if let Some(color) = stroke_color {
        if color == "none" {
            app.ui.tool_options.stroke_width = 0.0;
        } else if let Err(e) = app.run("tools.setColors", json!({"background": color})) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
    true
}

// ---------------------------------------------------------------------------------------------
// Properties (shape layers)

fn color_of(f: &photocraft_doc::Fill) -> Option<Color32> {
    match f {
        photocraft_doc::Fill::Solid(c) => {
            let v = c.to_rgba8();
            Some(Color32::from_rgb(v[0], v[1], v[2]))
        }
        _ => None,
    }
}

/// A colour swatch that opens a picker; returns the new `#rrggbb` when changed.
fn swatch(ui: &mut egui::Ui, fill: Option<&photocraft_doc::Fill>, tip: &str) -> Option<String> {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(26.0, 18.0), Sense::click());
    let current = fill.and_then(color_of);
    match (current, fill) {
        (Some(c), _) => {
            ui.painter().rect_filled(r, 2.0, c);
        }
        (None, Some(photocraft_doc::Fill::Gradient { stops, .. })) if !stops.is_empty() => {
            // Gradient fills preview as a left-to-right ramp through their stops.
            let rgb = |c: &photocraft_doc::Color| {
                let v = c.to_rgba8();
                Color32::from_rgb(v[0], v[1], v[2])
            };
            let mut mesh = egui::Mesh::default();
            let mut ramp: Vec<(f32, Color32)> = stops.iter().map(|(p, c)| (p.clamp(0.0, 1.0), rgb(c))).collect();
            ramp.insert(0, (0.0, ramp[0].1));
            ramp.push((1.0, ramp[ramp.len() - 1].1));
            for (i, (p, c)) in ramp.iter().enumerate() {
                let x = r.left() + r.width() * p;
                mesh.colored_vertex(egui::pos2(x, r.top()), *c);
                mesh.colored_vertex(egui::pos2(x, r.bottom()), *c);
                if i > 0 {
                    let k = (i as u32) * 2;
                    mesh.add_triangle(k - 2, k - 1, k);
                    mesh.add_triangle(k - 1, k, k + 1);
                }
            }
            ui.painter().add(mesh);
        }
        (None, Some(_)) => {
            // Pattern fill: a neutral checker hint.
            crate::widgets::checker(ui.painter(), r, 4.0);
        }
        (None, None) => {
            // "No colour": white with a red slash, like Photoshop.
            ui.painter().rect_filled(r, 2.0, Color32::WHITE);
            ui.painter().line_segment([r.left_bottom(), r.right_top()], Stroke::new(1.5, Color32::from_rgb(220, 40, 40)));
        }
    };
    ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, t.field_border), egui::StrokeKind::Outside);
    let resp = resp.on_hover_text(tip);
    let screen_id = resp.id.with("screen-color");
    let mut out = crate::screen_picker::take(ui.ctx(), screen_id).map(crate::color_picker_ui::hex);
    crate::widgets::swatch_popup(&resp).show(|ui| {
        let mut c = current.unwrap_or(Color32::BLACK);
        if egui::color_picker::color_picker_color32(ui, &mut c, egui::color_picker::Alpha::Opaque) {
            out = Some(format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b()));
        }
        if let Some(rgb) = crate::screen_picker::button(ui, screen_id) {
            out = Some(crate::color_picker_ui::hex(rgb));
        }
        if fill.is_some() && ui.button(tl!("No Color")).clicked() {
            out = Some("none".into());
        }
    });
    out
}

/// One corner of the live rectangle, preserving the other three radii.
/// The engine takes the full [top-left, top-right, bottom-right, bottom-left] array.
fn corner_radii_patch(radii: [f64; 4], corner: usize, radius: f32) -> Value {
    let mut next = radii;
    if let Some(r) = next.get_mut(corner) {
        *r = f64::from(radius.max(0.0));
    }
    json!({"radii": next})
}

/// Properties panel for a shape layer: Appearance (fill, stroke) and live shape geometry.
pub fn shape_properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, id: photocraft_doc::LayerId) {
    if app.session.active().is_some_and(|st| {
        let locks = st.doc.effective_locks(id);
        locks.all || locks.pixels
    }) {
        ui.disable();
    }
    let Some(sh) = app.session.active().and_then(|s| s.doc.layer(id)).and_then(|l| match &l.content {
        LayerContent::Shape(sh) => Some(sh.clone()),
        _ => None,
    }) else {
        return;
    };
    let t = Tokens::get(ui.ctx());
    let mut edit: Option<Value> = None;
    let doc_id = app.session.active().map(|st| st.doc.id);
    let key = |k: &str| format!("shape-{doc_id:?}-{}-{k}", id.0);
    // The shared collapsible section headers (#155).
    if crate::props_layout::section(ui, "appearance", tl!("Appearance")) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Fill")).color(t.text_dim).size(12.0));
            if let Some(c) = swatch(ui, sh.fill.as_ref(), tl!("Set shape fill type")) {
                edit = Some(if c == "none" { json!({"fill": null}) } else { json!({"fill": c, "coalesce": key("fill")}) });
            }
            ui.add_space(12.0);
            ui.label(egui::RichText::new(tl!("Stroke")).color(t.text_dim).size(12.0));
            if let Some(c) = swatch(ui, sh.stroke.as_ref().map(|s| &s.paint), tl!("Set shape stroke type")) {
                edit = Some(if c == "none" {
                    json!({"stroke": null})
                } else {
                    json!({"stroke": {"color": c, "width": sh.stroke.as_ref().map_or(3.0, |s| s.width)}, "coalesce": key("stroke")})
                });
            }
            let mut w = sh.stroke.as_ref().map_or(0.0, |s| s.width);
            if crate::widgets::value_field(ui, &mut w, 0.0..=288.0, "px", 60.0).changed() {
                edit = Some(if w <= 0.0 { json!({"stroke": null}) } else { json!({"stroke": {"width": w}, "coalesce": key("stroke-w")}) });
            }
        });
        if let Some(stroke) = &sh.stroke {
            let style = crate::shape_stroke_ui::StrokeOptions::from(stroke);
            if let Some(next) = crate::shape_stroke_ui::button(
                ui,
                &key("style"),
                &style,
                &mut app.ui.stroke_editor,
                sh.path.subpaths.iter().all(|s| s.closed),
                !matches!(sh.live, Some(photocraft_doc::LiveShape::Ellipse { .. })),
                app.session.active().map(|st| (st.doc.id, id)),
            ) {
                edit = Some(json!({"stroke": next.params()}));
            }
        }
    }
    if let Some(live) = &sh.live
        && crate::props_layout::section(ui, "liveShape", tl!("Shape"))
    {
        let num = |ui: &mut egui::Ui, label: &str, v: &mut f32, range: std::ops::RangeInclusive<f32>, unit: &str| -> bool {
            ui.label(egui::RichText::new(tl!(&label)).color(t.text_dim).size(12.0));
            crate::widgets::value_field(ui, v, range, unit, 64.0).changed()
        };
        match live {
            photocraft_doc::vector::LiveShape::Rect { rect, .. }
            | photocraft_doc::vector::LiveShape::Ellipse { rect }
            | photocraft_doc::vector::LiveShape::Polygon { rect, .. } => {
                ui.horizontal(|ui| {
                    let (mut w, mut h) = (rect[2] as f32, rect[3] as f32);
                    let cw = num(ui, "W", &mut w, 1.0..=300000.0, "px");
                    let ch = num(ui, "H", &mut h, 1.0..=300000.0, "px");
                    if cw || ch {
                        edit = Some(json!({"rect": [rect[0], rect[1], w.round(), h.round()], "coalesce": key("wh")}));
                    }
                });
            }
            photocraft_doc::vector::LiveShape::Line { .. } => {}
        }
        ui.vertical(|ui| match live {
            photocraft_doc::vector::LiveShape::Rect { radii, .. } => {
                // Keep the uniform workflow available after independent corner edits too.
                ui.horizontal(|ui| {
                    let mut all = radii[0] as f32;
                    if num(ui, tl!("Corner radius"), &mut all, 0.0..=100000.0, "px") {
                        edit = Some(json!({"radii": all, "coalesce": key("radius-all")}));
                    }
                });
                // Separate rows also fit the narrow Properties dock. The labels use the
                // localized corner names, rather than untranslated abbreviations.
                for (index, label) in [(0, tl!("Top Left")), (1, tl!("Top Right")), (3, tl!("Bottom Left")), (2, tl!("Bottom Right"))] {
                    ui.horizontal(|ui| {
                        let mut radius = radii[index] as f32;
                        if num(ui, label, &mut radius, 0.0..=100000.0, "px") {
                            let mut patch = corner_radii_patch(*radii, index, radius);
                            patch["coalesce"] = json!(key(&format!("radius-{index}")));
                            edit = Some(patch);
                        }
                    });
                }
            }
            photocraft_doc::vector::LiveShape::Polygon { sides, star_ratio, .. } => {
                ui.horizontal(|ui| {
                    let mut n = *sides as f32;
                    if num(ui, tl!("Sides"), &mut n, 3.0..=100.0, "") {
                        edit = Some(json!({"sides": n.round() as u32, "coalesce": key("sides")}));
                    }
                    let mut sr = (*star_ratio * 100.0) as f32;
                    if num(ui, tl!("Star ratio"), &mut sr, 1.0..=100.0, "%") {
                        edit = Some(json!({"starRatio": sr as f64 / 100.0, "coalesce": key("star")}));
                    }
                })
                .inner
            }
            photocraft_doc::vector::LiveShape::Line { weight, .. } => {
                ui.horizontal(|ui| {
                    let mut w = *weight as f32;
                    if num(ui, tl!("Weight"), &mut w, 1.0..=10000.0, "px") {
                        edit = Some(json!({"weight": w, "coalesce": key("weight")}));
                    }
                })
                .inner
            }
            _ => {}
        });
    }
    if let Some(mut p) = edit {
        p["layer"] = json!(id.0);
        if let Err(e) = app.run("shape.edit", p) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Paths panel

fn thumb(ui: &egui::Ui, r: Rect, path: &Path, doc: &Document) {
    let t = Tokens::get(ui.ctx());
    // At the document's aspect ratio, like the Channels thumbnails.
    let r = crate::channels_panel::fit_thumb(r, doc.size.width, doc.size.height).0;
    ui.painter().rect_filled(r, 0.0, Color32::from_gray(if t.pro { 222 } else { 240 }));
    let (w, h) = (doc.size.width.max(1) as f64, doc.size.height.max(1) as f64);
    let s = (r.width() as f64 / w).min(r.height() as f64 / h);
    let off = r.center() - vec2((w * s) as f32 / 2.0, (h * s) as f32 / 2.0);
    let map = |q: [f64; 2]| pos2(off.x + (q[0] * s) as f32, off.y + (q[1] * s) as f32);
    for (pts, closed) in path_lines(path, &map) {
        if closed && pts.len() >= 3 {
            ui.painter().add(egui::Shape::closed_line(pts, Stroke::new(1.0, Color32::from_gray(40))));
        } else {
            ui.painter().add(egui::Shape::line(pts, Stroke::new(1.0, Color32::from_gray(40))));
        }
    }
}

/// What a Paths panel row is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathRow {
    Saved,
    Work,
    /// The selected layer's shape path or vector mask (temporary, italic).
    Layer,
}

/// A Paths panel row.
#[derive(Clone, Debug)]
pub struct PathEntry {
    pub name: String,
    pub path: Path,
    pub kind: PathRow,
}

/// The Paths panel's rows, top to bottom: saved paths, the work path, then the selected layer's
/// shape path ("<Layer> Shape Path") or vector mask ("<Layer> Vector Mask"), like Photoshop.
pub fn path_rows(doc: &Document, active: Option<photocraft_doc::LayerId>) -> Vec<PathEntry> {
    let mut rows: Vec<_> = doc.paths.iter().map(|p| PathEntry { name: p.name.clone(), path: p.path.clone(), kind: PathRow::Saved }).collect();
    if let Some(wp) = &doc.work_path {
        rows.push(PathEntry { name: "Work Path".into(), path: wp.clone(), kind: PathRow::Work });
    }
    if let Some(l) = active.and_then(|id| doc.layer(id)) {
        let layer_path = match &l.content {
            LayerContent::Shape(sh) => Some((format!("{} Shape Path", l.name), sh.path.clone())),
            _ => l.vector_mask.as_ref().map(|v| (format!("{} Vector Mask", l.name), v.path.clone())),
        };
        if let Some((name, path)) = layer_path {
            rows.push(PathEntry { name, path, kind: PathRow::Layer });
        }
    }
    rows
}

fn footer_id() -> egui::Id {
    egui::Id::new("paths-footer")
}

fn ctx_data_footer(ctx: &egui::Context, r: Rect) {
    ctx.data_mut(|d| d.insert_temp(footer_id(), r));
}

/// Where the Paths panel's footer buttons were drawn last frame (the rect around them all).
pub fn paths_footer(ctx: &egui::Context) -> Option<Rect> {
    ctx.data(|d| d.get_temp(footer_id()))
}

/// Commands offered by a Paths row's context menu. The row's path is passed explicitly so a
/// right-click acts on that row even when another path is selected in the panel.
fn path_context_actions(entry: &PathEntry, doc: &Document) -> Vec<(&'static str, &'static str, Value)> {
    let key = match entry.kind {
        PathRow::Work => "work".to_string(),
        PathRow::Layer => "layer".to_string(),
        PathRow::Saved => entry.name.clone(),
    };
    let mut actions = vec![
        ("Make Selection", "path.toSelection", json!({"name": key})),
        ("Fill Path", "path.fill", json!({"name": key})),
        ("Stroke Path", "path.stroke", json!({"name": key, "tool": "brush"})),
    ];
    if entry.kind == PathRow::Work {
        let mut n = doc.paths.len().saturating_add(1);
        while doc.paths.iter().any(|p| p.name == format!("Path {n}")) {
            n = n.saturating_add(1);
            if n == usize::MAX {
                break;
            }
        }
        actions.push(("Save Path", "path.rename", json!({"name": "work", "to": format!("Path {n}")})));
    } else {
        let mut n = 1usize;
        while doc.paths.iter().any(|p| p.name == format!("{} copy {n}", entry.name)) {
            n = n.saturating_add(1);
            if n == usize::MAX {
                break;
            }
        }
        let copy_name = format!("{} copy {n}", entry.name);
        actions.push(("Duplicate Path", "path.set", json!({"name": copy_name, "path": photocraft_engine::vector_cmds::path_json(&entry.path)})));
    }
    if entry.kind != PathRow::Layer {
        actions.push(("Delete Path", "path.delete", json!({"name": key})));
    } else if entry.name.ends_with(" Vector Mask") {
        actions.push(("Delete Vector Mask", "layer.vectorMask.delete", json!({})));
    }
    actions
}

pub fn paths_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else {
        ui.label(egui::RichText::new(tl!("No document")).color(t.text_faint));
        return;
    };
    let doc = st.doc.clone();
    let rows = path_rows(&doc, st.active_layer);
    let has_layer_path = rows.iter().any(|r| r.kind == PathRow::Layer);
    let mut action: Option<(&str, Value)> = None;
    // The buttons sit in a footer at the panel's bottom, like Photoshop's.
    let footer = crate::widgets::footer_height(ui) + ui.spacing().item_spacing.y;
    let fill = ui.available_height() > footer + 60.0;
    let rows_h = if fill { ui.available_height() - footer } else { f32::INFINITY };
    egui::ScrollArea::vertical().id_salt("path-rows").max_height(rows_h).min_scrolled_height(if fill { rows_h } else { 0.0 }).auto_shrink([false, !fill]).show(
        ui,
        |ui| {
            if rows.is_empty() {
                ui.label(egui::RichText::new(tl!("Draw with the Pen tool (P) or make a work path from a selection.")).color(t.text_faint).size(11.5));
            }
            for PathEntry { name, path, kind } in &rows {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
                let key = match kind {
                    PathRow::Work => "work".to_string(),
                    PathRow::Layer => "layer".to_string(),
                    PathRow::Saved => name.clone(),
                };
                // Rows are painted: name them for screen readers and UI tests.
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::SelectableLabel, true, name));
                let sel = app.ui.selected_path.as_deref() == Some(key.as_str());
                if sel {
                    ui.painter().rect_filled(r, 0.0, t.row_selected);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, 0.0, t.hover.gamma_multiply(0.5));
                }
                thumb(ui, Rect::from_min_size(pos2(r.left() + 8.0, r.top() + 4.0), vec2(28.0, 28.0)), path, &doc);
                let mut job = egui::text::LayoutJob::default();
                // Temporary paths (the work path, the selected layer's shape path or vector mask) are italic.
                let italics = *kind != PathRow::Saved;
                job.append(
                    if *kind == PathRow::Work { tl!(name) } else { name },
                    0.0,
                    egui::TextFormat { font_id: egui::FontId::proportional(12.0), color: t.text, italics, ..Default::default() },
                );
                let g = ui.painter().layout_job(job);
                ui.painter().galley(pos2(r.left() + 46.0, r.center().y - g.size().y / 2.0), g, t.text);
                if resp.clicked() {
                    app.ui.selected_path = Some(key.clone());
                }
                // Double-clicking the Work Path saves it (Photoshop's "Save Path").
                if resp.double_clicked() && *kind == PathRow::Work {
                    let n = doc.paths.len() + 1;
                    action = Some(("path.rename", json!({"name": "work", "to": format!("Path {n}")})));
                }
                resp.context_menu(|ui| {
                    crate::widgets::menu_scroll(ui, |ui| {
                        ui.set_min_width(190.0);
                        let entry = PathEntry { name: name.clone(), path: path.clone(), kind: *kind };
                        let can_paint = app.session.active().and_then(|s| s.active_layer.and_then(|id| s.doc.layer(id))).is_some_and(|l| l.surface().is_some());
                        for (label, cmd, params) in path_context_actions(&entry, &doc) {
                            let enabled = !matches!(cmd, "path.fill" | "path.stroke") || can_paint;
                            if ui.add_enabled(enabled, egui::Button::new(tl!(label))).clicked() {
                                action = Some((cmd, params));
                                ui.close();
                            }
                        }
                    });
                });
                ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, t.separator));
            }
            // Clicking the empty area deselects the path, as in Photoshop: new fill and
            // adjustment layers then take the selection as their mask instead (#1419).
            let rest = ui.available_rect_before_wrap();
            let rest = rest.intersect(Rect::from_min_size(rest.min, vec2(rest.width(), rest.height().min(4000.0))));
            if rest.is_positive() && ui.interact(rest, ui.id().with("paths-empty"), Sense::click()).clicked() {
                app.ui.selected_path = None;
            }
        },
    );
    let sel = app.ui.selected_path.clone().filter(|s| s != "layer" || has_layer_path).unwrap_or_else(|| "work".into());
    let buttons = crate::widgets::panel_footer(ui, |ui| {
        let fg = hex(app.session.tools.foreground);
        let mut n = doc.paths.len() + 1;
        while doc.paths.iter().any(|p| p.name == format!("Path {n}")) {
            n += 1;
        }
        let items: [(&str, &str, &str, Value); 7] = [
            ("paint-bucket", "Fill path with foreground color", "path.fill", json!({"name": sel, "color": fg})),
            ("circle", "Stroke path with brush", "path.stroke", json!({"name": sel, "tool": "brush"})),
            ("square-dashed", "Load path as a selection", "path.toSelection", json!({"name": sel})),
            ("spline", "Make work path from selection", "select.toWorkPath", json!({"tolerance": 2.0})),
            ("square", "Add vector mask", "layer.vectorMask.fromPath", json!({"name": sel})),
            ("square-plus", "Create new path", "path.set", json!({"name": format!("Path {n}"), "path": {"subpaths": []}})),
            ("trash", "Delete path", "path.delete", json!({"name": sel})),
        ];
        // The layer's own path: deleting it deletes the vector mask (a shape keeps its path).
        let shape = rows.iter().any(|r| r.kind == PathRow::Layer && r.name.ends_with(" Shape Path"));
        let delete_layer_path = sel == "layer" && !shape;
        let items = items.map(|(icon, tip, cmd, p)| {
            if cmd == "path.delete" && delete_layer_path { (icon, "Delete vector mask", "layer.vectorMask.delete", json!({})) } else { (icon, tip, cmd, p) }
        });
        // Laid out from the right, so the last button goes first.
        let mut buttons = Rect::NOTHING;
        for (icon, tip, cmd, p) in items.into_iter().rev() {
            let icon = if crate::icons::exists(icon) { icon } else { "square" };
            let b = crate::icons::button(ui, icon, 26.0, false, tip);
            if b.clicked() {
                action = Some((cmd, p));
            }
            buttons = buttons.union(b.rect);
        }
        buttons
    });
    ctx_data_footer(ui.ctx(), buttons);
    if let Some((cmd, p)) = action
        && let Err(e) = app.run(cmd, p)
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_shape_tools_and_open_pen_use_the_complete_stroke_defaults() {
        for tool in [Tool::Rectangle, Tool::EllipseShape, Tool::Triangle, Tool::Polygon, Tool::Line, Tool::CustomShape] {
            let mut a = app();
            a.ui.tool_options.stroke_width = 5.0;
            a.ui.tool_options.shape_stroke = crate::shape_stroke_ui::StrokeOptions {
                cap: photocraft_doc::LineCap::Square,
                join: photocraft_doc::LineJoin::Bevel,
                dashes: vec![4.0, 2.0, 1.0, 3.0],
                dash_offset: -0.5,
                opacity: 0.4,
                ..Default::default()
            };
            let history = a.session.active().unwrap().history.past_len();
            finish_shape(&mut a, tool, [20.0, 20.0], [120.0, 100.0], Default::default());
            let info = a.run("shape.info", json!({})).unwrap();
            assert_eq!(info["stroke"]["dashes"], json!([4.0, 2.0, 1.0, 3.0]));
            assert_eq!(info["stroke"]["cap"], "square");
            assert_eq!(info["stroke"]["width"], 5.0);
            assert_eq!(a.session.active().unwrap().history.past_len(), history + 1);
            a.run("edit.undo", json!({})).unwrap();
            a.run("edit.redo", json!({})).unwrap();
            assert_eq!(a.run("shape.info", json!({})).unwrap(), info);
            a.ui.tool_options.vector_mode = "shape".into();
            a.ui.pen = Some(PenPath { knots: vec![[[30.0, 40.0]; 3], [[80.0, 40.0]; 3]], ..Default::default() });
            pen_commit(&mut a, false);
            let open = a.run("shape.info", json!({})).unwrap();
            assert_eq!(open["stroke"]["dashes"], info["stroke"]["dashes"]);
            assert_eq!(open["path"]["subpaths"][0]["closed"], false);
        }
    }

    fn app() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.sync_views();
        app
    }

    #[test]
    fn paths_context_menu_uses_the_clicked_row_and_dispatches_commands() {
        let mut app = app();
        let path = json!({"subpaths": [{"closed": true, "knots": [
            {"anchor": [10, 10], "in": [10, 10], "out": [10, 10]},
            {"anchor": [80, 10], "in": [80, 10], "out": [80, 10]},
            {"anchor": [80, 80], "in": [80, 80], "out": [80, 80]}
        ]}]});
        app.run("path.set", json!({"name": "First", "path": path})).unwrap();
        app.run("path.set", json!({"name": "Second", "path": path})).unwrap();
        let doc = &app.session.active().unwrap().doc;
        let first = path_rows(doc, None).into_iter().find(|r| r.name == "First").unwrap();
        let entries = path_context_actions(&first, doc);
        assert_eq!(entries[0].1, "path.toSelection");
        assert_eq!(entries[0].2["name"], "First");
        assert!(entries.iter().all(|(_, id, _)| photocraft_engine::commands::find(id).is_some()));
        let (_, id, params) = entries.into_iter().find(|(_, id, _)| *id == "path.toSelection").unwrap();
        app.run(id, params).unwrap();
        assert!(app.session.active().unwrap().doc.selection.is_some());
        assert_eq!(app.session.journal.last().map(|(id, _)| id.as_str()), Some("path.toSelection"));

        app.run("path.set", json!({"name": "First copy 1", "path": path})).unwrap();
        let doc = &app.session.active().unwrap().doc;
        let first = path_rows(doc, None).into_iter().find(|r| r.name == "First").unwrap();
        let (_, _, duplicate) = path_context_actions(&first, doc).into_iter().find(|(_, id, _)| *id == "path.set").unwrap();
        assert_eq!(duplicate["name"], "First copy 2", "duplicate must not overwrite an existing path");
    }

    #[test]
    fn work_path_context_save_and_layer_path_deletion_rules() {
        let mut app = app();
        let path = json!({"subpaths": []});
        app.run("path.set", json!({"name": "work", "path": path})).unwrap();
        let doc = &app.session.active().unwrap().doc;
        let work = path_rows(doc, None).into_iter().find(|r| r.kind == PathRow::Work).unwrap();
        let entries = path_context_actions(&work, doc);
        assert!(entries.iter().any(|(_, id, _)| *id == "path.rename"));
        assert!(!entries.iter().any(|(_, id, _)| *id == "path.set"));
        let layer = PathEntry { kind: PathRow::Layer, ..work };
        let entries = path_context_actions(&layer, doc);
        assert!(!entries.iter().any(|(_, id, _)| *id == "path.delete"));
    }

    /// How many shapes the path overlay paints for `app` this frame.
    fn overlay_shapes(app: &PhotocraftApp, ctx: &egui::Context) -> usize {
        let doc = app.session.active().unwrap().doc.clone();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 400.0));
            let painter = ui.ctx().layer_painter(egui::LayerId::background()).with_clip_rect(rect);
            let xf = ViewXform { rect, zoom: 1.0, center: [100.0, 100.0], flip: false, rotation: 0.0 };
            draw_overlay(app, &painter, &xf, &doc);
        });
        out.textures_delta.clear();
        out.shapes.len()
    }

    #[test]
    fn show_target_path_and_extras_hide_the_path_outlines() {
        // #1119: View › Show › Target Path only flipped its checkmark; the canvas kept drawing the
        // work path and the active shape's path.
        let ctx = egui::Context::default();
        let mut app = app();
        let path = json!({"subpaths": [{"closed": true, "knots": [
            {"anchor": [10, 10], "in": [10, 10], "out": [10, 10]},
            {"anchor": [80, 10], "in": [80, 10], "out": [80, 10]},
            {"anchor": [80, 80], "in": [80, 80], "out": [80, 80]}
        ]}]});
        app.run("path.set", json!({"path": path})).unwrap();
        assert!(app.session.active().unwrap().doc.work_path.is_some());
        finish_shape(&mut app, Tool::Rectangle, [100.0, 100.0], [160.0, 150.0], egui::Modifiers::NONE);
        let toggle = |app: &mut PhotocraftApp, id: &str| crate::menus::invoke(app, &ctx, id, Value::Null).unwrap();
        for tool in [Tool::Pen, Tool::PathSelection, Tool::DirectSelection, Tool::Rectangle] {
            app.ui.tool = tool;
            assert!(app.ui.view.show.target_path && app.ui.view.extras);
            assert!(overlay_shapes(&app, &ctx) > 0, "{tool:?}: the paths are drawn by default");
            toggle(&mut app, "view.show.targetPath");
            assert!(!app.ui.view.show.target_path);
            assert_eq!(overlay_shapes(&app, &ctx), 0, "{tool:?}: Target Path off");
            toggle(&mut app, "view.show.targetPath");
            toggle(&mut app, "view.extras");
            assert!(!app.ui.view.extras);
            assert_eq!(overlay_shapes(&app, &ctx), 0, "{tool:?}: Extras off");
            toggle(&mut app, "view.extras");
            assert!(overlay_shapes(&app, &ctx) > 0, "{tool:?}: shown again");
        }
        // The Pen's path in progress is a gesture, not the target path: it stays visible.
        app.ui.tool = Tool::Pen;
        app.ui.pen = Some(PenPath { knots: vec![[[20.0, 20.0]; 3], [[60.0, 40.0]; 3]], ..Default::default() });
        toggle(&mut app, "view.show.targetPath");
        assert!(overlay_shapes(&app, &ctx) > 0, "the pen path in progress is still drawn");
    }

    #[test]
    fn shape_drag_creates_layers_with_modifiers() {
        let mut app = app();
        finish_shape(&mut app, Tool::Rectangle, [10.0, 10.0], [60.0, 30.0], egui::Modifiers::SHIFT);
        let st = app.session.active().unwrap();
        let b = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds();
        assert_eq!((b.width(), b.height()), (50, 50), "shift = square");
        finish_shape(&mut app, Tool::EllipseShape, [100.0, 100.0], [120.0, 110.0], egui::Modifiers::ALT);
        let st = app.session.active().unwrap();
        let b = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds();
        assert!(b.x0.abs_diff(80) <= 1 && b.width().abs_diff(40) <= 1, "alt = from centre: {b:?}");
        finish_shape(&mut app, Tool::Line, [10.0, 150.0], [90.0, 152.0], egui::Modifiers::SHIFT);
        assert_eq!(app.session.active().unwrap().doc.layers.len(), 4);
    }

    #[test]
    fn shift_pressed_during_a_shape_drag_constrains_it() {
        // #668: ⇧ only counted when held at the press; Photoshop constrains when it is pressed
        // mid-drag (and stops when it is released).
        use crate::canvas::{ToolEvent, tool_event};
        for (tool, sides) in [(Tool::Polygon, 6), (Tool::Rectangle, 4), (Tool::EllipseShape, 0)] {
            let mut app = app();
            app.ui.tool = tool;
            app.ui.tool_options.polygon_sides = 6;
            let none = egui::Modifiers::NONE;
            tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, none);
            tool_event(&mut app, ToolEvent::Move { x: 40.0, y: 30.0, pressure: 1.0 }, none);
            tool_event(&mut app, ToolEvent::Move { x: 70.0, y: 40.0, pressure: 1.0 }, egui::Modifiers::SHIFT);
            tool_event(&mut app, ToolEvent::Up { x: 70.0, y: 40.0 }, egui::Modifiers::SHIFT);
            let st = app.session.active().unwrap();
            let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
            assert!(matches!(l.content, LayerContent::Shape(_)), "{tool:?} ({sides} sides): a shape layer");
            let b = l.surface().unwrap().content_bounds();
            // The drag is 60 × 30; constrained, the shape's box is 60 × 60 (a hexagon is narrower
            // than its box, so the height tells).
            assert!(b.height().abs_diff(60) <= 1 && b.width() <= 61, "{tool:?}: ⇧ mid-drag keeps proportions: {b:?}");
        }
    }

    #[test]
    fn shape_preview_is_the_committed_path() {
        let mut app = app();
        app.ui.tool_options.corner_radius = 8.0;
        let preview = |app: &PhotocraftApp, tool, end| {
            shape_geometry(&app.ui.tool_options, tool, [10.0, 10.0], end, egui::Modifiers::ALT)
                .and_then(|p| photocraft_engine::vector_cmds::shape_path(&p).ok())
        };
        for tool in [Tool::Rectangle, Tool::EllipseShape, Tool::Triangle, Tool::Polygon, Tool::Line] {
            let shown = preview(&app, tool, [60.0, 30.0]).unwrap();
            finish_shape(&mut app, tool, [10.0, 10.0], [60.0, 30.0], egui::Modifiers::ALT);
            let st = app.session.active().unwrap();
            let LayerContent::Shape(sh) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("{tool:?}: no shape layer") };
            assert_eq!(sh.path, shown, "{tool:?}");
        }
        assert!(preview(&app, Tool::Rectangle, [10.2, 40.0]).is_none(), "too thin to draw");
        assert!(photocraft_engine::vector_cmds::shape_path(&json!({"kind": "nope", "rect": [0, 0, 5, 5]})).is_err());
        assert!(photocraft_engine::vector_cmds::shape_path(&json!({"kind": "rect"})).is_err());
    }

    #[test]
    fn pen_builds_and_closes_a_work_path() {
        let mut app = app();
        app.ui.tool = Tool::Pen;
        for (x, y) in [(20.0, 20.0), (120.0, 20.0)] {
            pen_down(&mut app, x, y, false);
            pen_up(&mut app);
        }
        pen_down(&mut app, 120.0, 120.0, false);
        pen_move(&mut app, 140.0, 140.0, false); // smooth knot
        pen_up(&mut app);
        pen_down(&mut app, 20.5, 20.5, false); // click the first anchor: close
        assert!(app.ui.pen.is_none());
        let wp = app.session.active().unwrap().doc.work_path.clone().unwrap();
        assert!(wp.subpaths[0].closed && wp.subpaths[0].knots.len() == 3);
        assert!(wp.subpaths[0].knots[2].smooth);
        // Path Selection moves it.
        path_selection_finish(&mut app, [50.0, 50.0], [60.0, 55.0]);
        let wp = app.session.active().unwrap().doc.work_path.clone().unwrap();
        assert_eq!((wp.subpaths[0].knots[0].anchor.x, wp.subpaths[0].knots[0].anchor.y), (30.0, 25.0));
    }

    /// ⇧ locks the next Pen anchor to the nearest 45° through the last one. A pointer to the
    /// left or right stays on that horizontal, at the pointer's position along the line.
    #[test]
    fn shift_locks_the_next_pen_point_from_the_last_anchor() {
        use crate::canvas::{ToolEvent, tool_event};
        let mut app = app();
        app.ui.tool = Tool::Pen;
        app.ui.extras.snap = false;
        let shift = egui::Modifiers::SHIFT;
        let none = egui::Modifiers::NONE;
        let click = |app: &mut PhotocraftApp, x, y, mods| {
            tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, mods);
            tool_event(app, ToolEvent::Up { x, y }, mods);
        };
        let anchor = |app: &PhotocraftApp, i: usize| app.ui.pen.as_ref().unwrap().knots[i][0];
        let near = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6;

        click(&mut app, 40.0, 50.0, none);
        // Slightly below a point to the right: lock horizontal and keep the pointer's x.
        click(&mut app, 110.0, 62.0, shift);
        assert!(near(anchor(&app, 1), [110.0, 50.0]), "right: {:?}", anchor(&app, 1));
        // Slightly above a point to the left of that anchor: lock left on the same horizontal.
        click(&mut app, 20.0, 44.0, shift);
        assert!(near(anchor(&app, 2), [20.0, 50.0]), "left: {:?}", anchor(&app, 2));
        // Mostly above the last anchor: lock vertical, keeping the pointer's y.
        click(&mut app, 28.0, 10.0, shift);
        assert!(near(anchor(&app, 3), [20.0, 10.0]), "up: {:?}", anchor(&app, 3));
        // A diagonal pointer locks to 45° and projects onto that line.
        click(&mut app, 70.0, 55.0, shift);
        let d = anchor(&app, 4);
        assert!((d[0] - 20.0 - (d[1] - 10.0)).abs() < 1e-6 && d[0] > 60.0, "45°: {d:?}");
        // Without ⇧ the pointer is the anchor.
        click(&mut app, 30.0, 80.0, none);
        assert!(near(anchor(&app, 5), [30.0, 80.0]), "free: {:?}", anchor(&app, 5));

        // ⇧ while dragging a new anchor snaps the handle the same way, and releasing it frees the drag.
        click(&mut app, 40.0, 90.0, none);
        tool_event(&mut app, ToolEvent::Down { x: 40.0, y: 120.0, pressure: 1.0 }, shift);
        tool_event(&mut app, ToolEvent::Move { x: 90.0, y: 128.0, pressure: 1.0 }, shift);
        let handle = app.ui.pen.as_ref().unwrap().knots.last().unwrap()[2];
        assert!(near(handle, [90.0, 120.0]), "handle: {handle:?}");
        tool_event(&mut app, ToolEvent::Move { x: 95.0, y: 150.0, pressure: 1.0 }, none);
        let handle = app.ui.pen.as_ref().unwrap().knots.last().unwrap()[2];
        assert!(near(handle, [95.0, 150.0]), "released: {handle:?}");
        tool_event(&mut app, ToolEvent::Up { x: 95.0, y: 150.0 }, none);

        // ⇧-click on the first anchor still closes; the lock must not move the hit off it.
        let before = app.ui.pen.as_ref().unwrap().knots.len();
        click(&mut app, 40.0, 50.0, shift);
        assert!(app.ui.pen.is_none(), "closed from {before} knots");

        // The rubber band uses the same lock as the click.
        pen_down(&mut app, 10.0, 10.0, false);
        pen_up(&mut app);
        assert!(near(shift_locked_point([10.0, 10.0], [80.0, 18.0], true), [80.0, 10.0]));
        assert!(near(shift_locked_point([10.0, 10.0], [12.0, 70.0], true), [10.0, 70.0]));
        assert!(near(shift_locked_point([10.0, 10.0], [40.0, 25.0], false), [40.0, 25.0]));
        assert!(shift_locked_point([10.0, 10.0], [f64::NAN, 4.0], true)[0].is_nan());
    }

    /// After a path is placed, the Pen continues from either open end and inserts an anchor on a
    /// segment. A click on any other anchor removes it. None of these start a replacement path.
    #[test]
    fn pen_continues_an_open_end_and_adds_or_removes_a_placed_point() {
        use crate::canvas::{ToolEvent, tool_event};
        let mut app = app();
        app.ui.tool = Tool::Pen;
        app.ui.extras.snap = false;
        let click = |app: &mut PhotocraftApp, x, y| {
            let none = egui::Modifiers::NONE;
            tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, none);
            tool_event(app, ToolEvent::Up { x, y }, none);
        };
        app.run("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": false, "knots": [[20, 30], [80, 30], [80, 90]]}]}})).unwrap();
        click(&mut app, 80.0, 90.0);
        let pen = app.ui.pen.as_ref().unwrap();
        assert_eq!(pen.knots.len(), 3);
        assert_eq!(pen.knots[2][0], [80.0, 90.0]);
        assert!(pen.resume.is_some());
        assert!(!pen_undo_last_point(&mut app), "placed anchors are not undone as if they were new");
        click(&mut app, 140.0, 90.0);
        pen_commit(&mut app, false);
        let wp = app.session.active().unwrap().doc.work_path.clone().unwrap();
        assert_eq!(wp.subpaths[0].knots.len(), 4);
        assert_eq!((wp.subpaths[0].knots[0].anchor.x, wp.subpaths[0].knots[3].anchor.x), (20.0, 140.0));

        click(&mut app, 20.0, 30.0);
        assert_eq!(app.ui.pen.as_ref().unwrap().knots.last().unwrap()[0], [20.0, 30.0]);
        click(&mut app, 20.0, 80.0);
        pen_commit(&mut app, false);
        let wp = app.session.active().unwrap().doc.work_path.clone().unwrap();
        let last = wp.subpaths[0].knots.last().unwrap().anchor;
        assert_eq!((last.x, last.y), (20.0, 80.0));
        assert!(wp.subpaths[0].knots.iter().any(|k| (k.anchor.x - 140.0).abs() < 1e-6));

        app.run("path.set", json!({"path": {"subpaths": [{"closed": false, "knots": [[10, 40], [110, 40]]}]}})).unwrap();
        click(&mut app, 60.0, 40.0);
        assert!(app.ui.pen.is_none());
        let wp = app.session.active().unwrap().doc.work_path.clone().unwrap();
        assert_eq!(wp.subpaths[0].knots.len(), 3);
        let mid = wp.subpaths[0].knots[1].anchor;
        assert!((mid.x - 60.0).abs() < 2.0 && (mid.y - 40.0).abs() < 2.0, "{mid:?}");
        click(&mut app, mid.x, mid.y);
        assert_eq!(app.session.active().unwrap().doc.work_path.as_ref().unwrap().subpaths[0].knots.len(), 2);

        app.run("path.set", json!({"path": {"subpaths": [{"closed": true, "knots": [[10, 10], [90, 10], [90, 80], [10, 80]]}]}})).unwrap();
        click(&mut app, 50.0, 10.0);
        assert_eq!(app.session.active().unwrap().doc.work_path.as_ref().unwrap().subpaths[0].knots.len(), 5);
        click(&mut app, 90.0, 80.0);
        let sp = &app.session.active().unwrap().doc.work_path.as_ref().unwrap().subpaths[0];
        assert_eq!(sp.knots.len(), 4);
        assert!(sp.closed);
    }

    #[test]
    fn pen_hover_cursor_matches_the_press() {
        let mut app = app();
        app.ui.tool = Tool::Pen;
        app.ui.extras.snap = false;
        app.run("path.set", json!({"path": {"subpaths": [{"closed": false, "knots": [[20, 40], [80, 40], [80, 100]]}]}})).unwrap();
        let at = |app: &PhotocraftApp, x, y, mods| path_cursor(app, [x, y], mods);
        let none = egui::Modifiers::NONE;
        assert_eq!(at(&app, 20.0, 40.0, none), Some(PathCursor::Continue));
        assert_eq!(at(&app, 80.0, 100.0, none), Some(PathCursor::Continue));
        assert_eq!(at(&app, 80.0, 40.0, none), Some(PathCursor::Remove));
        assert_eq!(at(&app, 50.0, 40.0, none), Some(PathCursor::Add));
        assert_eq!(at(&app, 10.0, 10.0, none), None);
        assert_eq!(at(&app, 80.0, 40.0, egui::Modifiers::ALT), Some(PathCursor::Curve));
        assert_eq!(at(&app, 50.0, 40.0, egui::Modifiers::COMMAND), Some(PathCursor::Curve));
        assert_eq!(at(&app, 80.0, 40.0, egui::Modifiers::COMMAND), Some(PathCursor::Move));
        assert_eq!(PathCursor::Continue.icon(), "circle");
        assert_eq!(PathCursor::Remove.icon(), "minus");
        assert_eq!(PathCursor::Add.icon(), "plus");
        assert_eq!(PathCursor::Curve.icon(), "pen-curve");
        assert_eq!(PathCursor::Move.icon(), "move");

        app.ui.pen = Some(PenPath { knots: vec![[[20.0, 40.0]; 3], [[80.0, 40.0]; 3], [[80.0, 100.0]; 3]], ..Default::default() });
        assert_eq!(at(&app, 20.0, 40.0, none), Some(PathCursor::Continue));
        assert_eq!(at(&app, 80.0, 100.0, none), Some(PathCursor::Curve));
        assert_eq!(at(&app, 50.0, 40.0, none), None, "a path being drawn does not insert on its own segments");

        app.ui.pen = None;
        app.ui.tool = Tool::PathSelection;
        assert_eq!(at(&app, 80.0, 40.0, none), Some(PathCursor::Move));
        assert_eq!(at(&app, 50.0, 40.0, none), Some(PathCursor::Move));
    }

    #[test]
    fn pen_point_undo_preserves_earlier_knots_and_document_history() {
        let mut app = app();
        app.ui.tool = Tool::Pen;
        app.run("shape.create", json!({"kind": "rect", "rect": [10, 10, 40, 40], "fill": "#ff0000"})).unwrap();
        let old_history = app.session.active().unwrap().history.past_len();

        pen_down(&mut app, 20.0, 20.0, false);
        pen_up(&mut app);
        pen_down(&mut app, 120.0, 20.0, false);
        pen_move(&mut app, 120.0, 60.0, false);
        pen_up(&mut app);
        pen_down(&mut app, 120.0, 120.0, false);
        pen_up(&mut app);

        let original_first_two = app.ui.pen.as_ref().unwrap().knots[..2].to_vec();
        assert!(pen_undo_last_point(&mut app));
        let pen = app.ui.pen.as_ref().unwrap();
        assert_eq!(pen.knots, original_first_two);
        assert!(!pen.dragging, "undo releases the current Pen drag");
        assert_eq!(app.session.active().unwrap().history.past_len(), old_history, "undo must not roll back previous document changes");
        assert!(app.session.active().unwrap().doc.work_path.is_none(), "path is not committed yet");

        pen_commit(&mut app, false);
        let work = app.session.active().unwrap().doc.work_path.as_ref().unwrap();
        assert_eq!(work.subpaths[0].knots.len(), 2);
        assert!(work.subpaths[0].knots[1].smooth, "retained smooth handles are unchanged");
    }

    #[test]
    fn pen_undo_of_first_anchor_cancels_empty_draft() {
        let mut app = app();
        assert!(!pen_undo_last_point(&mut app));
        pen_down(&mut app, 15.0, 17.0, false);
        pen_up(&mut app);
        assert!(pen_undo_last_point(&mut app));
        assert!(app.ui.pen.is_none());
        assert!(!pen_undo_last_point(&mut app));
        assert_eq!(app.session.active().unwrap().history.past_len(), 0);
        assert!(app.session.active().unwrap().doc.work_path.is_none());
    }

    /// #1419: Solid Color (and every new fill or adjustment layer) made while a path is selected
    /// in the Paths panel takes it as its vector mask; with the path deselected, the selection
    /// is its layer mask again.
    #[test]
    fn new_fill_layer_takes_the_selected_path_as_its_vector_mask() {
        let mut app = app();
        let layer = |app: &PhotocraftApp, r: Result<Value, String>| {
            let id = photocraft_doc::LayerId(r.unwrap()["layer"].as_u64().unwrap());
            app.session.active().unwrap().doc.layer(id).unwrap().clone()
        };
        // Drawing a work path with the Pen selects it.
        app.ui.tool = Tool::Pen;
        for (x, y) in [(20.0, 20.0), (120.0, 20.0), (120.0, 120.0)] {
            pen_down(&mut app, x, y, false);
            pen_up(&mut app);
        }
        pen_down(&mut app, 20.5, 20.5, false);
        assert_eq!(app.ui.selected_path.as_deref(), Some("work"));
        let wp = app.session.active().unwrap().doc.work_path.clone().unwrap();
        app.run("select.rect", json!({"x": 150, "y": 150, "width": 20, "height": 20})).unwrap();
        for id in ["layer.newFillLayer.solidColor", "layer.newFillLayer.gradient", "layer.newAdjustmentLayer.invert"] {
            app.ui.selected_path = Some("work".into());
            let r = app.run(id, json!({}));
            let l = layer(&app, r);
            assert_eq!(l.vector_mask.as_ref().map(|v| &v.path), Some(&wp), "{id}: the work path is the vector mask");
            assert!(l.mask.is_none(), "{id}: the path wins over the selection");
            // The new layer's vector mask is now the active path, not the work path.
            assert_eq!(app.ui.selected_path.as_deref(), Some("layer"), "{id}");
        }
        // So the next fill layer isn't masked by the work path again.
        let r = app.run("layer.newFillLayer.solidColor", json!({}));
        assert!(layer(&app, r).vector_mask.is_none());
        // A caller can opt out.
        let r = app.run("layer.newFillLayer.solidColor", json!({"path": null}));
        assert!(layer(&app, r).vector_mask.is_none());
        // A selected saved path is used.
        app.run("path.set", json!({"name": "Saved", "path": {"subpaths": [{"closed": true, "knots": [[0, 0], [9, 0], [9, 9]]}]}})).unwrap();
        app.ui.selected_path = Some("Saved".into());
        let saved = app.session.active().unwrap().doc.paths[0].path.clone();
        let r = app.run("layer.newFillLayer.solidColor", json!({}));
        assert_eq!(layer(&app, r).vector_mask.map(|v| v.path), Some(saved));
        // Deselected (or a stale name): the selection is the layer mask, no vector mask.
        for sel in [None, Some("Gone".to_string())] {
            app.ui.selected_path = sel;
            let r = app.run("layer.newFillLayer.solidColor", json!({}));
            let l = layer(&app, r);
            assert!(l.vector_mask.is_none() && l.mask.is_some());
        }
        // Make Work Path selects the new work path.
        app.run("select.toWorkPath", json!({"tolerance": 2.0})).unwrap();
        assert_eq!(app.ui.selected_path.as_deref(), Some("work"));
    }

    /// #1482: clicking an existing final Pen anchor breaks its outgoing handle without
    /// deleting the preceding curve or creating another knot; dragging changes only that handle.
    #[test]
    fn pen_last_anchor_can_be_broken_and_reshaped_while_drawing() {
        use crate::canvas::{ToolEvent, tool_event};
        let mut app = app();
        app.ui.tool = Tool::Pen;
        let down = |app: &mut PhotocraftApp, x, y| tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, egui::Modifiers::NONE);
        let move_to = |app: &mut PhotocraftApp, x, y| tool_event(app, ToolEvent::Move { x, y, pressure: 1.0 }, egui::Modifiers::NONE);
        let up = |app: &mut PhotocraftApp, x, y| tool_event(app, ToolEvent::Up { x, y }, egui::Modifiers::NONE);

        down(&mut app, 20.0, 20.0);
        up(&mut app, 20.0, 20.0);
        down(&mut app, 100.0, 20.0);
        move_to(&mut app, 120.0, 40.0);
        up(&mut app, 120.0, 40.0);
        let original = app.ui.pen.as_ref().unwrap().knots[1];
        assert_eq!(original, [[100.0, 20.0], [80.0, 0.0], [120.0, 40.0]]);

        // Bare click breaks the outgoing handle, preserving the preceding segment's tangent.
        down(&mut app, 100.0, 20.0);
        up(&mut app, 100.0, 20.0);
        let pen = app.ui.pen.as_ref().unwrap();
        assert_eq!(pen.knots.len(), 2);
        assert_eq!(pen.knots[1], [[100.0, 20.0], [80.0, 0.0], [100.0, 20.0]]);
        assert!(!pen.dragging && !pen.adjusting_last);

        // Re-dragging the same final anchor moves only its outgoing handle (a cusp).
        down(&mut app, 100.0, 20.0);
        move_to(&mut app, 110.0, 60.0);
        up(&mut app, 110.0, 60.0);
        let pen = app.ui.pen.as_ref().unwrap();
        assert_eq!(pen.knots.len(), 2);
        assert_eq!(pen.knots[1], [[100.0, 20.0], [80.0, 0.0], [110.0, 60.0]]);

        // Drawing on still adds a separate next anchor and keeps the cusp in the work path.
        down(&mut app, 150.0, 100.0);
        up(&mut app, 150.0, 100.0);
        assert_eq!(app.ui.pen.as_ref().unwrap().knots.len(), 3);
        pen_commit(&mut app, false);
        let work = app.session.active().unwrap().doc.work_path.as_ref().unwrap();
        let k = &work.subpaths[0].knots[1];
        assert_eq!([k.in_ctrl.x, k.in_ctrl.y], [80.0, 0.0]);
        assert_eq!([k.out_ctrl.x, k.out_ctrl.y], [110.0, 60.0]);
        assert!(!k.smooth, "breaking the handle must persist as an unlinked PSD/vector knot");
    }

    #[test]
    fn pen_last_anchor_hit_tolerance_tracks_zoom_and_draft_round_trips() {
        for zoom in [0.25, 1.0, 4.0] {
            let mut app = app();
            app.ui.views[0].zoom = zoom;
            pen_down(&mut app, 20.0, 20.0, false);
            pen_up(&mut app);
            pen_down(&mut app, 100.0, 100.0, false);
            pen_move(&mut app, 110.0, 120.0, false);
            pen_up(&mut app);
            let incoming = app.ui.pen.as_ref().unwrap().knots[1][1];
            pen_down(&mut app, 100.0 + 5.0 / f64::from(zoom), 100.0, false);
            pen_up(&mut app);
            let pen = app.ui.pen.as_ref().unwrap();
            assert_eq!(pen.knots.len(), 2);
            assert_eq!(pen.knots[1][1], incoming);
            assert_eq!(pen.knots[1][2], pen.knots[1][0]);
            let saved = serde_json::to_value(pen).unwrap();
            let restored: PenPath = serde_json::from_value(saved).unwrap();
            assert_eq!(restored, *pen);
            assert_eq!(pen_to_json(&restored, false)["subpaths"][0]["knots"][1]["smooth"], false);
            pen_down(&mut app, 100.0 + 7.0 / f64::from(zoom), 100.0, false);
            assert_eq!(app.ui.pen.as_ref().unwrap().knots.len(), 3);
        }
        let legacy: PenPath = serde_json::from_value(json!({"knots": [[[10, 10], [10, 10], [10, 10]]]})).unwrap();
        assert!(legacy.unlinked.is_empty());
        assert!(!legacy.dragging && !legacy.adjusting_last);
    }

    /// #1520: changing one corner through the Properties command must keep the other three
    /// independent, remain a live rectangle, and undo/redo as one geometry edit.
    #[test]
    fn live_rectangle_corner_radius_edit_preserves_other_corners_and_history() {
        let mut app = app();
        let created = app.run("shape.create", json!({"kind": "roundedRect", "rect": [10, 10, 150, 100], "radii": [5, 10, 15, 20]})).unwrap();
        let id = created["layer"].as_u64().unwrap();
        let get_radii = |app: &PhotocraftApp| {
            let st = app.session.active().unwrap();
            let shape = st.doc.layer(photocraft_doc::LayerId(id)).unwrap();
            match &shape.content {
                LayerContent::Shape(sh) => match &sh.live {
                    Some(photocraft_doc::vector::LiveShape::Rect { radii, .. }) => *radii,
                    _ => panic!("rectangle must remain a live shape"),
                },
                _ => panic!("expected shape layer"),
            }
        };
        assert_eq!(get_radii(&app), [5.0, 10.0, 15.0, 20.0]);
        let mut patch = corner_radii_patch(get_radii(&app), 1, 30.0);
        patch["layer"] = json!(id);
        app.run("shape.edit", patch).unwrap();
        assert_eq!(get_radii(&app), [5.0, 30.0, 15.0, 20.0]);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(get_radii(&app), [5.0, 10.0, 15.0, 20.0]);
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(get_radii(&app), [5.0, 30.0, 15.0, 20.0]);
        for corner in 0..4 {
            let before = get_radii(&app);
            let radius = 35.0 + corner as f32;
            let mut expected = before;
            expected[corner] = f64::from(radius);
            let mut patch = corner_radii_patch(before, corner, radius);
            patch["layer"] = json!(id);
            app.run("shape.edit", patch).unwrap();
            assert_eq!(get_radii(&app), expected);
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(get_radii(&app), before);
            app.run("edit.redo", json!({})).unwrap();
            assert_eq!(get_radii(&app), expected);
        }
        app.run("shape.edit", json!({"layer": id, "radii": 12})).unwrap();
        assert_eq!(get_radii(&app), [12.0; 4]);
    }

    /// #534: dragging in a shape's fill picker, opened from the Properties panel at the right edge
    /// of the window, keeps the picker in place (it flipped from side to side as its width
    /// followed the colour readouts, which moved it under the pointer).
    #[test]
    fn shape_colour_picker_stays_put_while_dragging() {
        use egui::{PointerButton, vec2};
        use egui_kittest::kittest::Queryable;
        let mut app = app();
        let id = app.run("shape.create", json!({"kind": "rect", "rect": [10, 10, 100, 60], "fill": "#3070c0"})).unwrap()["layer"].as_u64().unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
            move |ui, app: &mut PhotocraftApp| {
                // Fonts set up after the first frame only apply from the next one.
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                egui::Panel::right("properties").resizable(false).exact_size(285.0).show(ui, |ui| shape_properties(app, ui, photocraft_doc::LayerId(id)));
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        h.run_steps(4);
        let label = h.query_all(egui_kittest::kittest::by().label("Fill").include_labels()).next().expect("the Fill label").rect();
        // The swatch is right of its label.
        let swatch = egui::pos2(label.right() + 17.0, label.center().y);
        let press = |h: &mut egui_kittest::Harness<'_, PhotocraftApp>, pos, pressed| {
            h.event(egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.step();
        };
        h.hover_at(swatch);
        h.step();
        press(&mut h, swatch, true);
        press(&mut h, swatch, false);
        h.run_steps(3);
        let popup = |h: &egui_kittest::Harness<'_, PhotocraftApp>| {
            h.ctx.memory(|m| m.areas().visible_layer_ids().into_iter().filter(|l| l.order == egui::Order::Foreground).find_map(|l| m.area_rect(l.id)))
        };
        let opened = popup(&h).expect("the picker opened");
        let fill = |h: &egui_kittest::Harness<'_, PhotocraftApp>| {
            let st = h.state().session.active().unwrap();
            match &st.doc.layer(photocraft_doc::LayerId(id)).unwrap().content {
                LayerContent::Shape(sh) => sh.fill.clone(),
                _ => None,
            }
        };
        let before = fill(&h);
        // Drag across the picker's colour area.
        let start = opened.min + vec2(60.0, opened.height() * 0.5);
        h.hover_at(start);
        h.step();
        press(&mut h, start, true);
        for i in 1..=10 {
            h.event(egui::Event::PointerMoved(start + vec2(i as f32 * 5.0, i as f32)));
            h.step();
            let r = popup(&h).expect("the picker stays open");
            assert_eq!(r.left_top(), opened.left_top(), "step {i}: the picker moved from {opened:?} to {r:?}");
            assert!(h.ctx.content_rect().contains_rect(r), "step {i}: {r:?} runs off the window");
        }
        press(&mut h, start, false);
        assert_ne!(fill(&h), before, "the drag picked a colour");
    }
}

#[cfg(test)]
#[path = "path_selection_tests.rs"]
mod path_selection_tests;
