use craft::behavior::CanvasView;
use craft::behavior::kurbo::{Point, Rect};
use craft::draw::geometry::shape::ShapeGeometry;
use craft::draw::ui::shape_drag::{BoxConstraints, ShapeKind};
use craft::keymap::Mods;

use super::options::draws_path;
use super::{Color32, PhotocraftApp, Tool, Value, json, paint_param};
use crate::state::ToolOptions;

pub fn is_shape_tool(t: Tool) -> bool {
    matches!(t, Tool::Rectangle | Tool::EllipseShape | Tool::Triangle | Tool::Polygon | Tool::Star | Tool::Line | Tool::CustomShape)
}

/// The framework's shape kind of a Shape tool. The Rectangle tool is `Rectangle` whatever its corner
/// radius: `create_geometry` rounds from the geometry's radii.
pub fn kind_of(tool: Tool) -> ShapeKind {
    match tool {
        Tool::EllipseShape => ShapeKind::Ellipse,
        Tool::Triangle => ShapeKind::Triangle,
        Tool::Polygon => ShapeKind::Polygon,
        Tool::Star => ShapeKind::Star,
        Tool::Line => ShapeKind::Line,
        Tool::CustomShape => ShapeKind::Custom,
        _ => ShapeKind::Rectangle,
    }
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

/// An open Pen shape's stroke: the options bar's stroke when one is set, else a 1 px stroke in the
/// foreground colour, so the open path stays visible.
pub(super) fn open_pen_stroke_param(app: &PhotocraftApp) -> Value {
    let stroke = stroke_param(app);
    if !stroke.is_null() {
        return stroke;
    }
    let mut fallback = app.ui.tool_options.shape_stroke.params();
    fallback["width"] = json!(1.0);
    fallback["color"] = solid_paint_param(tool_solid_color(app, false));
    fallback
}

/// The paint the next shape's fill (`stroke` false) or stroke takes: the options bar's gradient,
/// else the foreground / background colour.
pub(super) fn tool_paint(app: &PhotocraftApp, stroke: bool) -> photocraft_doc::Fill {
    tool_gradient(app, stroke).clone().unwrap_or_else(|| photocraft_doc::Fill::Solid(tool_solid_color(app, stroke)))
}

/// [`tool_paint`] as `shape.create` takes it.
fn tool_paint_param(app: &PhotocraftApp, stroke: bool) -> Value {
    tool_gradient(app, stroke).as_ref().map_or_else(|| solid_paint_param(tool_solid_color(app, stroke)), paint_param)
}

fn tool_gradient(app: &PhotocraftApp, stroke: bool) -> &Option<photocraft_doc::Fill> {
    let options = &app.ui.tool_options;
    if stroke { &options.shape_stroke_gradient } else { &options.shape_fill_gradient }
}

fn solid_paint_param(color: photocraft_color::Color) -> Value {
    if color.mode == photocraft_color::ColorMode::Cmyk { json!(color) } else { json!([color.c[0], color.c[1], color.c[2], color.alpha]) }
}

fn tool_solid_color(app: &PhotocraftApp, stroke: bool) -> photocraft_color::Color {
    use photocraft_color::{Color, ColorMode};
    let [r, g, b, alpha] = if stroke { app.session.tools.background } else { app.session.tools.foreground };
    if app.session.active().is_some_and(|st| st.doc.mode == ColorMode::Cmyk)
        && let Some(c) = app.session.tools.picked_role_cmyk(usize::from(stroke))
    {
        return Color { mode: ColorMode::Cmyk, c, alpha };
    }
    Color::rgba(r, g, b, alpha)
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

/// A box on whole pixels (position and size rounded on their own) unless `whole` is off, `None`
/// under 1 px a side. The one rounding rule of every box shape, so the preview and the commit draw
/// the same path.
fn whole_box(rect: &Rect, whole: bool) -> Option<Rect> {
    let r = rect.abs();
    let (x0, y0, w, h) = if whole { (r.x0.round(), r.y0.round(), r.width().round(), r.height().round()) } else { (r.x0, r.y0, r.width(), r.height()) };
    (w >= 1.0 && h >= 1.0).then(|| Rect::new(x0, y0, x0 + w, y0 + h))
}

/// Whether a drag of `kind` lands on whole pixels. The Triangle (#13370), Rectangle (#13346) and
/// Polygon (#13326) drags keep their fractions with Align Edges off in Shape mode; the other Shape
/// tools' drags (the framework's drag lands them on pixels), and a greyed checkbox, still snap.
pub(super) fn snaps_to_pixels(app: &PhotocraftApp, kind: ShapeKind) -> bool {
    let o = &app.ui.tool_options;
    !(matches!(kind, ShapeKind::Triangle | ShapeKind::Rectangle | ShapeKind::Polygon) && o.shape_mode == super::VectorMode::Shape && !o.align_edges)
}

/// `geometry` as a drag commits it: boxes on whole pixels, a line aligned to pixel edges
/// ([`align_line_edges`]; `keep_angle` for a ⇧-constrained one). `whole` off keeps a box's
/// fractions ([`snaps_to_pixels`]). `None` when too small to draw.
/// The one place the rounding lives: `shape_outline` and `finish_drag` both call it.
fn snapped(geometry: &ShapeGeometry, keep_angle: bool, whole: bool) -> Option<ShapeGeometry> {
    Some(match geometry {
        ShapeGeometry::Rect { rect, radii } => ShapeGeometry::Rect { rect: whole_box(rect, whole)?, radii: *radii },
        ShapeGeometry::Ellipse { rect } => ShapeGeometry::Ellipse { rect: whole_box(rect, whole)? },
        ShapeGeometry::Triangle { rect } => ShapeGeometry::Triangle { rect: whole_box(rect, whole)? },
        ShapeGeometry::Polygon { rect, sides, star_ratio } => ShapeGeometry::Polygon { rect: whole_box(rect, whole)?, sides: *sides, star_ratio: *star_ratio },
        ShapeGeometry::Custom { rect, keep_proportions } => ShapeGeometry::Custom { rect: whole_box(rect, whole)?, keep_proportions: *keep_proportions },
        ShapeGeometry::Line { from, to, weight } => {
            if from.distance(*to) < 1.0 {
                return None;
            }
            let (a, b, weight) = align_line_edges([from.x, from.y], [to.x, to.y], weight.max(1.0), keep_angle)?;
            ShapeGeometry::Line { from: Point::new(a[0], a[1]), to: Point::new(b[0], b[1]), weight }
        }
        other => other.clone(),
    })
}

/// The Rectangle tool's gear as the framework's [`BoxConstraints`]: the stored constraint name (serde's names), the
/// fixed and ratio pairs (a 0 is an empty field) and From Center. The one mapping the tool's drag
/// (`host::configure`) and the gear in the options bar share.
pub(super) fn constraints_of(o: &ToolOptions) -> BoxConstraints {
    let field = |v: f32| (v != 0.0).then_some(f64::from(v));
    let mut c = BoxConstraints::default();
    c.constraint = serde_json::from_value(json!(o.rect_constraint)).unwrap_or_default();
    c.fixed = (field(o.rect_fixed_width), field(o.rect_fixed_height));
    c.ratio = (field(o.rect_ratio_width), field(o.rect_ratio_height));
    c.from_center = o.rect_from_center;
    c
}

/// Stores the gear's `constraints` into the options: the inverse of [`constraints_of`].
pub(super) fn store_constraints(o: &mut ToolOptions, constraints: &BoxConstraints) {
    let field = |v: Option<f64>| v.map_or(0.0, |v| v as f32);
    o.rect_constraint = match serde_json::to_value(constraints.constraint) {
        Ok(Value::String(name)) => name,
        _ => "unconstrained".into(),
    };
    o.rect_fixed_width = field(constraints.fixed.0);
    o.rect_fixed_height = field(constraints.fixed.1);
    o.rect_ratio_width = field(constraints.ratio.0);
    o.rect_ratio_height = field(constraints.ratio.1);
    o.rect_from_center = constraints.from_center;
}

fn box_of(rect: &Rect) -> [f64; 4] {
    let r = rect.abs();
    [r.x0, r.y0, r.width(), r.height()]
}

/// `shape.create` params of `geometry` without fill and stroke: the one builder of the command's
/// geometry. The arrowheads come from the tool options, which `ShapeGeometry` does not carry.
fn geometry_params(app: &PhotocraftApp, geometry: &ShapeGeometry, kind: ShapeKind) -> Result<Value, String> {
    let o = &app.ui.tool_options;
    Ok(match geometry {
        ShapeGeometry::Rect { rect, radii } if radii.iter().any(|r| *r > 0.0) => json!({"kind": "roundedRect", "rect": box_of(rect), "radii": radii}),
        ShapeGeometry::Rect { rect, .. } => json!({"kind": "rect", "rect": box_of(rect)}),
        ShapeGeometry::Ellipse { rect } => json!({"kind": "ellipse", "rect": box_of(rect)}),
        // The Triangle tool's triangle fills its box (#13350).
        ShapeGeometry::Triangle { rect } => json!({"kind": "triangle", "rect": box_of(rect)}),
        ShapeGeometry::Polygon { rect, sides, star_ratio } => {
            let mut p = json!({"kind": if kind == ShapeKind::Star { "star" } else { "polygon" }, "rect": box_of(rect), "sides": sides});
            if kind == ShapeKind::Star || *star_ratio < 1.0 {
                p["starRatio"] = json!(star_ratio);
            }
            p
        }
        ShapeGeometry::Line { from, to, weight } => json!({"kind": "line", "from": [from.x, from.y], "to": [to.x, to.y], "weight": weight,
            "arrowStart": o.arrow_start, "arrowEnd": o.arrow_end,
            "arrowWidth": o.arrow_width, "arrowLength": o.arrow_length,
            // Settings saved with the old 0..100 range draw at the nearest value (#13258).
            "arrowConcavity": if o.arrow_concavity.is_finite() { o.arrow_concavity.round().clamp(-50.0, 50.0) } else { 0.0 }}),
        // PhotoCraft's drag rules keep `radial_polygons` off, so no tool sends a radial polygon and `shape.create` has no kind for it
        ShapeGeometry::RadialPolygon { .. } => return Err(tl!("This shape can't be created").into()),
        ShapeGeometry::Custom { .. } => return Err(tl!("A custom shape is placed from its preset").into()),
    })
}

/// `p` (a drag's `shape.create` geometry of `kind`) with the Polygon tool's own corner radius, which
/// rounds every vertex of the next polygon it drags (#13302). The Create Polygon dialog keeps its own.
fn with_tool_radius(app: &PhotocraftApp, kind: ShapeKind, mut p: Value) -> Value {
    let r = app.ui.tool_options.polygon_corner_radius;
    if kind == ShapeKind::Polygon && r > 0.0 {
        p["cornerRadius"] = json!(r);
    }
    p
}

/// `shape.create` params with the options bar's fill and stroke.
fn create_params(app: &PhotocraftApp, geometry: &ShapeGeometry, kind: ShapeKind) -> Result<Value, String> {
    let mut p = geometry_params(app, geometry, kind)?;
    p["useAlignedRendering"] = json!(app.ui.tool_options.align_edges);
    p["fill"] = fill_param(app);
    p["stroke"] = stroke_param(app);
    Ok(p)
}

fn report(app: &mut PhotocraftApp, e: String) {
    app.ui.status = e;
    app.ui.status_error = true;
}

/// Finish a Shape-tool drag with the geometry the framework tool committed (⇧ and ⌥ are already in
/// it): Path mode adds the outline to the Work Path, Custom Shape places the preset, the others
/// make a shape layer. A geometry too small to draw changes nothing.
pub fn finish_drag(app: &mut PhotocraftApp, kind: ShapeKind, geometry: &ShapeGeometry, mods: Mods) {
    let Some(geometry) = snapped(geometry, mods.shift, snaps_to_pixels(app, kind)) else { return };
    if draws_path(&app.ui.tool_options) {
        // Path mode: the outline joins the work path (one more subpath), and no layer is made.
        let Some(outline) = outline_of(app, kind, &geometry) else { return };
        // Appended here rather than with `path.set`'s `op`, which would turn a custom shape's
        // subtracted subpaths (holes) into combined ones; the shape goes in as one component.
        // A new work path takes the outline's fill rule.
        let mut path = app
            .session
            .active()
            .and_then(|st| st.doc.work_path.clone())
            .unwrap_or_else(|| photocraft_doc::vector::Path { fill_rule: outline.fill_rule, ..Default::default() });
        let outline = as_one_component(&outline, path.fill_rule).unwrap_or(outline);
        let first_new = path.subpaths.len();
        path.subpaths.extend(outline.subpaths);
        // The first component of a path always combines; the later ones take the drag's operation.
        if first_new > 0
            && let Some(op) = drag_path_op(app)
            && let Some(sp) = path.subpaths.get_mut(first_new)
        {
            sp.op = path_op_of(op);
        }
        let path = photocraft_engine::vector_cmds::path_json(&path);
        match app.run("path.set", json!({"name": "work", "path": path, "historyName": tool_history_name(kind)})) {
            Ok(_) => app.ui.selected_path = Some("work".into()),
            Err(e) => report(app, e),
        }
        return;
    }
    if let ShapeGeometry::Custom { rect, keep_proportions } = &geometry {
        // With an operation other than New Layer the shape joins the active shape layer as one
        // component, its holes joined to it as in Path mode.
        if let Some(op) = drag_path_op(app)
            && let Some(id) = super::options::selected_shape(&app.session)
            && let Some(outline) = outline_of(app, kind, &geometry)
        {
            // `shape.create` gives every component the operation, so a shape of several components
            // that can't be made one (its parts intersect or exclude) would change: refuse it.
            let outline = match as_one_component(&outline, outline.fill_rule) {
                Some(one) => one,
                None if outline.subpaths.len() <= 1 => outline,
                None => {
                    report(app, tl!("This custom shape can't be added to a shape layer with a path operation: choose New Layer").into());
                    return;
                }
            };
            let path = photocraft_engine::vector_cmds::path_json(&outline);
            if let Err(e) =
                app.run("shape.create", json!({"kind": "path", "path": path, "addTo": id.0, "op": op, "useAlignedRendering": app.ui.tool_options.align_edges}))
            {
                report(app, e);
            }
            return;
        }
        if let Ok(rect) = serde_json::from_value(json!(box_of(rect))) {
            let (fill, stroke) = (fill_param(app), stroke_param(app));
            crate::preset_panels::finish_custom_shape(app, rect, *keep_proportions, fill, stroke);
        }
        return;
    }
    let mut p = match create_params(app, &geometry, kind) {
        Ok(p) => with_tool_radius(app, kind, p),
        Err(e) => return report(app, e),
    };
    p["historyName"] = json!(tool_history_name(kind));
    // With an operation other than New Layer the drag joins the active shape layer.
    if let Some(op) = drag_path_op(app)
        && let Some(id) = super::options::selected_shape(&app.session)
    {
        p["addTo"] = json!(id.0);
        p["op"] = json!(op);
    }
    if let Err(e) = app.run("shape.create", p) {
        report(app, e);
    }
}

/// The operation the next Shape-tool drag is added with (the options bar's Path operations);
/// None for a new shape layer. The one place press modifiers can override it (#13227).
fn drag_path_op(app: &PhotocraftApp) -> Option<&'static str> {
    super::pen::shape_path_op(&app.ui.tool_options)
}

fn path_op_of(op: &str) -> photocraft_doc::vector::PathOp {
    use photocraft_doc::vector::PathOp;
    match op {
        "subtract" => PathOp::Subtract,
        "intersect" => PathOp::Intersect,
        "exclude" => PathOp::Exclude,
        _ => PathOp::Combine,
    }
}

/// The tool's History label for a new shape layer or a Work Path edit, when defined.
pub(crate) fn tool_history_name(kind: ShapeKind) -> Option<&'static str> {
    match kind {
        ShapeKind::Line => Some("Line Tool"),
        ShapeKind::Triangle => Some("Triangle Tool"),
        ShapeKind::Ellipse => Some("Ellipse Tool"),
        _ => None,
    }
}

pub(super) fn fill_param(app: &PhotocraftApp) -> Value {
    if app.ui.tool_options.shape_fill { tool_paint_param(app, false) } else { Value::Null }
}

/// Create a shape layer from `geometry` with the options bar's fill and stroke, as one History
/// step named `history` (the commands' own name without one): the Create Rectangle / Ellipse / …
/// dialogs' OK (`shape_dialog`). The geometry is used as given, with no pixel rounding.
pub fn create_geometry(app: &mut PhotocraftApp, geometry: &ShapeGeometry, kind: ShapeKind, history: Option<&str>) -> Result<Value, String> {
    let mut p = create_params(app, geometry, kind)?;
    if let Some(name) = history {
        p["historyName"] = json!(name);
    }
    app.run("shape.create", p)
}

/// A custom shape as one shape component, the way Photoshop adds it to a path: the subpaths
/// after the first join it, holes wound against it so that `rule` cuts them, so its holes cut
/// the shape only and not the work path under it. `None` for a single subpath, and when joined
/// it would cover a different area (an intersect or exclude subpath, or a cut reaching past
/// the shape, like the Crescent Moon's).
fn as_one_component(outline: &photocraft_doc::vector::Path, rule: photocraft_doc::vector::FillRule) -> Option<photocraft_doc::vector::Path> {
    use photocraft_doc::vector::{Path, PathOp, Subpath};
    // Clockwise on screen (y down) or not, from the flattened outline's signed area.
    let clockwise = |s: &Subpath| {
        let pts = photocraft_vector::flatten_subpath(s, 0.25).pts;
        let twice_area: f64 = pts.iter().zip(pts.iter().cycle().skip(1)).map(|(p, q)| p.0 * q.1 - q.0 * p.1).sum();
        twice_area > 0.0
    };
    let (first, rest) = outline.subpaths.split_first()?;
    if rest.is_empty() {
        return None;
    }
    let outer = clockwise(first);
    let mut joined = Path { fill_rule: rule, ..outline.clone() };
    for s in &mut joined.subpaths[1..] {
        let want = match s.op {
            PathOp::Join => continue,
            PathOp::Combine => outer,
            PathOp::Subtract => !outer,
            PathOp::Intersect | PathOp::Exclude => return None,
        };
        if clockwise(s) != want {
            s.knots.reverse();
            for k in &mut s.knots {
                std::mem::swap(&mut k.in_ctrl, &mut k.out_ctrl);
            }
        }
        s.op = PathOp::Join;
    }
    // Compare the two areas at a small fixed size.
    const SIDE: f64 = 128.0;
    let rect = photocraft_geom::Rect::from_xywh(0, 0, SIDE as u32, SIDE as u32);
    let cover = |p: &Path| photocraft_vector::path_coverage(&photocraft_engine::presets::shapes::fit(p, [0.0, 0.0, SIDE, SIDE], true), rect);
    let same = cover(outline).iter().zip(cover(&joined)).all(|(a, b)| (a - b).abs() <= 0.5);
    same.then_some(joined)
}

/// The outline of an already snapped geometry, as a path (a custom shape fitted to its box).
fn outline_of(app: &PhotocraftApp, kind: ShapeKind, geometry: &ShapeGeometry) -> Option<photocraft_doc::vector::Path> {
    if let ShapeGeometry::Custom { rect, keep_proportions } = geometry {
        let groups = photocraft_engine::presets::shapes::all_groups(&app.session);
        let shape = groups.iter().flat_map(|g| &g.items).find(|s| s.name == app.ui.presets_ui.shape())?;
        Some(photocraft_engine::presets::shapes::fit(&shape.path, box_of(rect), *keep_proportions))
    } else {
        photocraft_engine::vector_cmds::shape_path(&with_tool_radius(app, kind, geometry_params(app, geometry, kind).ok()?)).ok()
    }
}

/// The outline a Shape-tool drag draws for `geometry`, as the commit makes it.
fn shape_outline(app: &PhotocraftApp, kind: ShapeKind, geometry: &ShapeGeometry, keep_angle: bool) -> Option<photocraft_doc::vector::Path> {
    outline_of(app, kind, &snapped(geometry, keep_angle, snaps_to_pixels(app, kind))?)
}

/// The preview and command both use the full float tool colours, including transparent paint.
/// In Path mode only the outline shows, as the drag draws no layer.
fn preview_layer(app: &PhotocraftApp, kind: ShapeKind, geometry: &ShapeGeometry, keep_angle: bool) -> Option<photocraft_doc::ShapeLayer> {
    let o = &app.ui.tool_options;
    let path = shape_outline(app, kind, geometry, keep_angle)?;
    let paint = !draws_path(o);
    Some(photocraft_doc::ShapeLayer {
        path,
        use_aligned_rendering: paint && o.align_edges,
        fill: (paint && o.shape_fill).then(|| tool_paint(app, false)),
        stroke: (paint && o.stroke_width > 0.0).then(|| o.shape_stroke.stroke(o.stroke_width, tool_paint(app, true))),
        ..Default::default()
    })
}

#[cfg(test)]
/// The shape layer the drag of `geometry` would make, for the drag's preview and its tests.
pub(super) fn shape_preview(app: &PhotocraftApp, kind: ShapeKind, geometry: &ShapeGeometry) -> Option<photocraft_doc::ShapeLayer> {
    preview_layer(app, kind, geometry, false)
}

/// `path` with every point mapped through `view` to screen points.
fn path_on_screen(path: &photocraft_doc::vector::Path, view: &CanvasView) -> photocraft_doc::vector::Path {
    let map = |p: &photocraft_geom::Point| {
        let s = view.to_screen(Point::new(p.x, p.y));
        photocraft_geom::Point::new(f64::from(s.x), f64::from(s.y))
    };
    let mut out = path.clone();
    for k in out.subpaths.iter_mut().flat_map(|s| s.knots.iter_mut()) {
        let (a, i, o) = (map(&k.anchor), map(&k.in_ctrl), map(&k.out_ctrl));
        (k.anchor, k.in_ctrl, k.out_ctrl) = (a, i, o);
    }
    out
}

/// Paints the fill and stroke of the Shape-tool drag in progress, with the path and the stroke of
/// the commit. The accent outline over its halo is the framework tool's own paint.
pub fn show_preview(app: &PhotocraftApp, ui: &egui::Ui, at: egui::Rect, view: &CanvasView) {
    let Some((kind, geometry)) = super::host::shape_preview(app) else { return };
    let shift = craft::ui::runtime::pointer::frame(ui.ctx()).mods.shift;
    let Some(mut shape) = preview_layer(app, kind, geometry, shift) else { return };
    // Align Edges fits the shape to document pixels, so it is applied here, in document space:
    // the preview then shows the committed layer's geometry at any zoom.
    if let Some(aligned) = photocraft_vector::align_edges::aligned_shape(&shape, photocraft_vector::DEFAULT_TOLERANCE) {
        shape = aligned;
    }
    shape.use_aligned_rendering = false;
    shape.path = path_on_screen(&shape.path, view);
    if let Some(stroke) = &mut shape.stroke {
        // `view.zoom` is screen points per document unit.
        stroke.width *= view.zoom as f32;
    }
    // This lightweight screen overlay retains the existing display-colour behaviour; document
    // rendering still goes through its format/profile path when the shape is committed.
    crate::shape_stroke_ui::paint_shape_preview(&ui.painter().with_clip_rect(at), &shape);
}

/// A click of a Shape tool: the Create Shape dialog for the clicked kind, none in Path mode.
pub fn click(app: &mut PhotocraftApp, kind: ShapeKind, at: [f64; 2]) -> Option<craft::ui::dialog::DialogId> {
    // A Rectangle click inside the active live rectangle leaves it alone (#13340).
    if draws_path(&app.ui.tool_options) || crate::dialogs::shape_dialog::click_inside_active_rectangle(app, kind, at) {
        return None;
    }
    crate::dialogs::shape_dialog::open(app, kind, Point::new(at[0], at[1]))
}

#[cfg(test)]
mod triangle_history_ui_tests {
    use super::*;
    use crate::canvas::testing;

    fn app() -> Box<PhotocraftApp> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.ui.tool = Tool::Triangle;
        app
    }

    #[test]
    fn triangle_drags_record_tool_history_in_shape_and_path_modes() {
        for mode in [super::super::VectorMode::Shape, super::super::VectorMode::Path] {
            let mut app = app();
            app.ui.tool_options.shape_mode = mode;
            for x in [20.0, 150.0] {
                let before = app.session.active().unwrap().history.past_len();
                testing::press(&mut app, [x, 20.0], egui::Modifiers::NONE);
                testing::move_to(&mut app, [x + 100.0, 100.0], egui::Modifiers::NONE);
                testing::release(&mut app, [x + 100.0, 100.0], egui::Modifiers::NONE);
                let st = app.session.active().unwrap();
                assert_eq!(st.history.past_len(), before + 1, "{mode:?}");
                assert_eq!(st.history.undo_label(), Some("Triangle Tool"), "{mode:?}");
                if mode == super::super::VectorMode::Path {
                    assert_eq!(st.doc.work_path.as_ref().unwrap().subpaths.len(), if x == 20.0 { 1 } else { 2 });
                } else {
                    assert!(st.doc.layers.iter().any(|l| l.name == "Triangle 1"));
                }
            }
        }
    }

    #[test]
    fn triangle_dialog_confirmation_records_tool_history() {
        let mut app = app();
        let id = crate::dialogs::shape_dialog::open(&mut app, ShapeKind::Triangle, Point::new(20.0, 20.0)).unwrap();
        let before = app.session.active().unwrap().history.past_len();
        crate::dialogs::testing::confirm(&mut app, id).unwrap();
        let st = app.session.active().unwrap();
        assert_eq!(st.history.past_len(), before + 1);
        assert_eq!(st.history.undo_label(), Some("Triangle Tool"));
        assert!(st.doc.layers.iter().any(|l| l.name == "Triangle 1"));
    }
}
