use super::options::solid_paint;
use super::overlay::path_lines;
use super::{Affine, Color32, PhotocraftApp, Tokens, Tool, ToolOptions, Value, ViewXform, json, paint_param};

pub fn is_shape_tool(t: Tool) -> bool {
    matches!(t, Tool::Rectangle | Tool::EllipseShape | Tool::Triangle | Tool::Polygon | Tool::Line | Tool::CustomShape)
}

fn rgb32(c: [f32; 4]) -> Color32 {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(b(c[0]), b(c[1]), b(c[2]))
}

pub(super) fn hex(c: [f32; 4]) -> String {
    let c = rgb32(c);
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

pub(super) fn stroke_param(app: &PhotocraftApp) -> Value {
    let o = &app.ui.tool_options;
    if o.stroke_width <= 0.0 {
        return Value::Null;
    }
    let mut p = o.shape_stroke.params();
    p["width"] = json!(o.stroke_width);
    p["color"] = tool_paint_param(app, true);
    p
}

/// The paint the next shape's fill (`stroke` false) or stroke takes: the options bar's gradient,
/// else the foreground / background colour.
fn tool_paint(app: &PhotocraftApp, stroke: bool) -> photocraft_doc::Fill {
    let (gradient, color) = tool_paint_source(app, stroke);
    gradient.clone().unwrap_or_else(|| solid_paint(color))
}

/// [`tool_paint`] as `shape.create` takes it.
fn tool_paint_param(app: &PhotocraftApp, stroke: bool) -> Value {
    let (gradient, color) = tool_paint_source(app, stroke);
    gradient.as_ref().map_or_else(|| json!(color), paint_param)
}

fn tool_paint_source(app: &PhotocraftApp, stroke: bool) -> (&Option<photocraft_doc::Fill>, [f32; 4]) {
    let o = &app.ui.tool_options;
    if stroke { (&o.shape_stroke_gradient, app.session.tools.background) } else { (&o.shape_fill_gradient, app.session.tools.foreground) }
}

/// A Line-tool drag snapped as Photoshop's Align Edges (on by default) does: the ends on whole
/// pixels, and a horizontal or vertical line's weight on whole pixels and its sides on pixel edges,
/// the low side moved down to the edge at or before it (a 1 px line drawn at y 23 fills row 22, as
/// in Photoshop), so a 1 px line fills one pixel row instead of half of two (#9486). Diagonal lines
/// keep their exact sides, and a ⇧-constrained 45° one its angle. None when the ends meet.
pub(super) fn align_line_edges(a: [f64; 2], b: [f64; 2], weight: f64, keep_angle: bool) -> Option<([f64; 2], [f64; 2], f64)> {
    let d = [b[0] - a[0], b[1] - a[1]];
    let (mut a, mut b) = (a.map(f64::round), b.map(f64::round));
    // A ⇧-constrained 45° line (`keep_angle`) keeps its angle: one whole-pixel step along both
    // axes, not each end rounded on its own (which can turn a short diagonal into a 1 × 2 step).
    if keep_angle && (d[0].abs() - d[1].abs()).abs() <= 1e-9 * d[0].abs().max(1.0) {
        let step = d[0].abs().round();
        b = [a[0] + step.copysign(d[0]), a[1] + step.copysign(d[1])];
    }
    if a == b {
        return None;
    }
    let mut weight = weight;
    // The axis across a horizontal (1) or vertical (0) line.
    let across = if a[1] == b[1] {
        Some(1)
    } else if a[0] == b[0] {
        Some(0)
    } else {
        None
    };
    if let Some(i) = across {
        // Whole pixels across: the weight snaps to the nearest pixel (so 1.99 and 2.01 both fill
        // 2, not 1 and 3), then the low side moves down to the edge at or before it.
        weight = weight.round().max(1.0);
        let lo = (a[i] - weight / 2.0).floor();
        (a[i], b[i]) = (lo + weight / 2.0, lo + weight / 2.0);
    }
    Some((a, b, weight))
}

/// Geometry of a Shape-tool drag as `shape.create` params, without fill and stroke (None when too
/// small): ⇧ constrains proportions, ⌥ draws from the centre. Custom shapes give their box as a `rect`.
pub(super) fn shape_geometry(o: &ToolOptions, tool: Tool, start: [f64; 2], end: [f64; 2], mods: egui::Modifiers) -> Option<Value> {
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
        let (from, to, weight) = align_line_edges(start, [start[0] + dx, start[1] + dy], f64::from(o.line_weight.max(1.0)), mods.shift)?;
        json!({"kind": "line", "from": from, "to": to, "weight": weight})
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

/// The box `[x0, y0, x1, y1]` a Shape-tool drag draws (a line's: its ends'), which snaps while
/// Space repositions it (`snap_ui`). None when too small to draw.
pub fn shape_bounds(o: &ToolOptions, tool: Tool, start: [f64; 2], end: [f64; 2], mods: egui::Modifiers) -> Option<[f64; 4]> {
    let g = shape_geometry(o, tool, start, end, mods)?;
    if tool == Tool::Line {
        let a: [f64; 2] = serde_json::from_value(g["from"].clone()).ok()?;
        let b: [f64; 2] = serde_json::from_value(g["to"].clone()).ok()?;
        return Some([a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])]);
    }
    let [x, y, w, h]: [f64; 4] = serde_json::from_value(g["rect"].clone()).ok()?;
    Some([x, y, x + w, y + h])
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

pub(super) fn fill_param(app: &PhotocraftApp) -> Value {
    if app.ui.tool_options.shape_fill { tool_paint_param(app, false) } else { Value::Null }
}

/// Create a shape layer from `shape.create` geometry with the options bar's fill and stroke: the
/// end of a drag, and the Create Rectangle / Ellipse / … dialogs (`shape_dialog`).
pub fn create_shape(app: &mut PhotocraftApp, mut geometry: Value) -> Result<Value, String> {
    geometry["fill"] = fill_param(app);
    geometry["stroke"] = stroke_param(app);
    app.run("shape.create", geometry)
}

/// The preview and command both use the full float tool colours, including transparent paint.
pub(super) fn shape_preview(app: &PhotocraftApp, tool: Tool, start: [f64; 2], end: [f64; 2], mods: egui::Modifiers) -> Option<photocraft_doc::ShapeLayer> {
    let o = &app.ui.tool_options;
    let geometry = shape_geometry(o, tool, start, end, mods)?;
    let path = if tool == Tool::CustomShape {
        let rect = geometry.get("rect").and_then(|r| serde_json::from_value::<[f64; 4]>(r.clone()).ok())?;
        let groups = photocraft_engine::presets::shapes::all_groups(&app.session);
        let shape = groups.iter().flat_map(|g| &g.items).find(|s| s.name == app.ui.presets_ui.shape())?;
        photocraft_engine::presets::shapes::fit(&shape.path, rect, mods.shift)
    } else {
        photocraft_engine::vector_cmds::shape_path(&geometry).ok()?
    };
    Some(photocraft_doc::ShapeLayer {
        path,
        fill: o.shape_fill.then(|| tool_paint(app, false)),
        stroke: (o.stroke_width > 0.0).then(|| o.shape_stroke.stroke(o.stroke_width, tool_paint(app, true))),
        ..Default::default()
    })
}

/// Shape-tool drag preview using the same path and vector stroke geometry as the commit.
pub fn draw_shape_preview(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform, tool: Tool, start: [f64; 2], end: [f64; 2], mods: egui::Modifiers) {
    let Some(mut shape) = shape_preview(app, tool, start, end, mods) else { return };
    let path = shape.path.clone();
    let origin = xf.to_screen(0.0, 0.0);
    let x = xf.to_screen(1.0, 0.0) - origin;
    let y = xf.to_screen(0.0, 1.0) - origin;
    let transform = Affine { m: [x.x as f64, x.y as f64, y.x as f64, y.y as f64, origin.x as f64, origin.y as f64] };
    shape.path = shape.path.transform(&transform);
    if let Some(stroke) = &mut shape.stroke {
        stroke.width *= xf.zoom;
    }
    // This lightweight screen overlay retains the existing display-colour behaviour; document
    // rendering still goes through its format/profile path when the shape is committed.
    crate::shape_stroke_ui::paint_shape_preview(painter, &shape);
    let accent = Tokens::get(painter.ctx()).accent;
    for (pts, _) in path_lines(&path, &|q| xf.to_screen(q[0] as f32, q[1] as f32)) {
        // The accent path over a dark halo: visible on any pixels (#172).
        painter.extend(crate::tool_feedback::contrast_path(pts, true, accent));
    }
}
