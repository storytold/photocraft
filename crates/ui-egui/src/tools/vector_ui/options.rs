use craft::behavior::EditEvent;
use craft::behavior::geometry::OutlineDraft;
use craft::behavior::kurbo::Rect;
use craft::color_ui::ui::Paint;
use craft::draw::ui::pen::PathOptionsPopover;
use craft::draw::ui::shape::{self as fw, Arrowheads, ArrowheadsPopover, ShapePanel, StrokeCells, VectorMode, VectorModeSelect};
use craft::theme::Srgba;
use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _, SideLabel, bind_tracked, clicked};
use craft::ui::display::{Divider, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::runtime::env::{Label, UnitKind};
use craft::ui::text::NumberField;
use photocraft_engine::presets::PresetsExt;

use super::path_menus::{path_alignment_menu, path_arrangement_menu, path_operations_menu};
use super::path_ops::{active_path_name, indexed_document};
use super::shapes::is_shape_tool;
use super::{LayerContent, PhotocraftApp, Tool, Value, copy_arrow_to_tool_defaults, json};
use crate::history_keys;

// ---------------------------------------------------------------------------------------------
// Options bars

/// The popover flags of the bar, kept by the framework per viewer.
#[derive(Clone, Copy, Default)]
struct BarOpen {
    path_options: bool,
    arrows: bool,
    rect_gear: bool,
}

/// A label from a translated string.
fn label(text: &str) -> Label {
    Label::from(text.to_string())
}

/// The document's pixels per inch, which sizes lengths typed in inches or points.
fn document_ppi(app: &PhotocraftApp) -> f64 {
    app.session.active().map_or(72.0, |st| f64::from(st.doc.resolution_dpi))
}

/// A line of muted help text in the bar.
fn hint(ui: &mut egui::Ui, text: &str) {
    Text::new(label(text)).show(ui);
}

const PEN_MODES: [(VectorMode, bool); 3] = [(VectorMode::Shape, true), (VectorMode::Path, true), (VectorMode::Pixels, false)];
const OTHER_PEN_MODES: [(VectorMode, bool); 2] = [(VectorMode::Path, true), (VectorMode::Shape, true)];
const SHAPE_TOOL_MODES: [(VectorMode, bool); 2] = [(VectorMode::Shape, true), (VectorMode::Path, true)];
const SHAPE_ONLY: [(VectorMode, bool); 1] = [(VectorMode::Shape, true)];

/// Options bar for vector tools; false for other tools.
pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool) -> bool {
    if !(is_shape_tool(tool)
        || matches!(
            tool,
            Tool::Pen | Tool::FreeformPen | Tool::CurvaturePen | Tool::AddAnchorPoint | Tool::DeleteAnchorPoint | Tool::PathSelection | Tool::DirectSelection
        ))
    {
        return false;
    }
    Row::new().show(ui, |ui, cx| {
        // PLAN: assumed the bar's popover flags live in the body's shown-memory slot (`BodyCx::memory.shown`); the contract lists no public accessor.
        let open = cx.memory.shown::<BarOpen>(cx.id);
        bar(app, ui, tool, open);
    });
    true
}

/// The cells of the bar for `tool`.
fn bar(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool, open: &mut BarOpen) {
    let target = selected_shape(&app.session);
    // Direct Selection shares the Path Selection bar (#13219).
    if matches!(tool, Tool::PathSelection | Tool::DirectSelection) && !crate::direct_select::active_paths(app).is_empty() {
        path_operations_menu(app, ui);
        Divider::vertical().show(ui);
    }
    if matches!(tool, Tool::PathSelection | Tool::DirectSelection) && target.is_none() {
        if tool == Tool::PathSelection {
            hint(ui, if app.ui.vector_mask_target { "Drag to move the targeted vector mask" } else { tl!("Drag to move the selected path") });
        }
        return;
    }
    if tool == Tool::AddAnchorPoint {
        hint(ui, tl!("No options for the Add Anchor Point Tool."));
        return;
    }
    if tool == Tool::DeleteAnchorPoint {
        hint(ui, tl!("No options for the Delete Anchor Point Tool."));
        return;
    }
    if matches!(tool, Tool::Pen | Tool::FreeformPen | Tool::CurvaturePen) {
        pen_bar(app, ui, tool, open);
        return;
    }
    if is_shape_tool(tool) {
        // Shape draws a shape layer, Path adds the outline to the work path (#9453).
        VectorModeSelect::new(bind_tracked!(app.ui.tool_options => shape_mode), &SHAPE_TOOL_MODES).show(ui);
        if draws_path(&app.ui.tool_options) {
            Divider::vertical().show(ui);
            make_buttons(app, ui);
        }
    } else {
        // Path Selection and Direct Selection edit the selected shape layer: the Shape tools'
        // Path mode isn't theirs to show or change.
        let mut mode = VectorMode::Shape;
        VectorModeSelect::new(Bind::bound(&mut mode), &SHAPE_ONLY).show(ui);
    }
    Divider::vertical().show(ui);
    // The Shape tools' Path mode hides Fill and Stroke (an outline has no paint); the tool's
    // geometry, the Line's arrowheads included, stays.
    let paint = !(is_shape_tool(tool) && draws_path(&app.ui.tool_options));
    crate::shape_options::options_bar(app, ui, tool, paint);
    if is_shape_tool(tool) {
        // Path operations, alignment and arrangement, in Shape and Path mode.
        Divider::vertical().show(ui);
        path_operations_menu(app, ui);
        path_alignment_menu(app, ui);
        path_arrangement_menu(app, ui);
    }
    if tool == Tool::Rectangle {
        // The gear: what a drag is kept to (square, fixed size, proportional) and From Center.
        Divider::vertical().show(ui);
        let ppi = document_ppi(app);
        let gear = super::shapes::constraints_of(&app.ui.tool_options);
        let out = fw::BoxConstraintsPopover::new(Bind::given(gear), Bind::bound(&mut open.rect_gear), ppi).show(ui);
        if let Some(picked) = out.edits.iter().rev().find_map(EditEvent::value) {
            super::shapes::store_constraints(&mut app.ui.tool_options, picked);
        }
        if let Some(alert) = out.out_of_range {
            crate::option_cells::range_alert(app, ui, &alert);
        }
    }
    if tool == Tool::Line {
        Divider::vertical().show(ui);
        let ppi = document_ppi(app);
        let o = &mut app.ui.tool_options;
        let mut arrows = Arrowheads::new();
        arrows.start = o.arrow_start;
        arrows.end = o.arrow_end;
        arrows.width = f64::from(o.arrow_width);
        arrows.length = f64::from(o.arrow_length);
        arrows.concavity = f64::from(o.arrow_concavity);
        let out = ArrowheadsPopover::new(Bind::given(arrows), Bind::bound(&mut open.arrows), ppi).show(ui);
        // A Concavity typed past -50..50 % raises the reference's alert (#13258).
        let alert = out.out_of_range;
        if let Some(a) = out.edits.iter().rev().find_map(EditEvent::value) {
            o.arrow_start = a.start;
            o.arrow_end = a.end;
            o.arrow_width = a.width as f32;
            o.arrow_length = a.length as f32;
            o.arrow_concavity = a.concavity as f32;
        }
        if let Some(alert) = alert {
            crate::option_cells::range_alert(app, ui, &alert);
        }
    }
    // Every shape tool's one Align Edges, at the bar's right end after the gear (#13346, #13370,
    // #13326, #16869 for the Star tool). Only a shape layer has pixel edges to align: greyed in Path
    // mode (and in a pixels mode).
    if is_shape_tool(tool) {
        Divider::vertical().show(ui);
        super::alignment::shape_tool_option(app, ui);
    }
}

/// The Pen, Freeform Pen and Curvature Pen bar.
fn pen_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool, open: &mut BarOpen) {
    // Photoshop's Pen lists Shape, Path and a greyed Pixels; the other pens keep theirs.
    let offered: &[(VectorMode, bool)] = if tool == Tool::Pen { &PEN_MODES } else { &OTHER_PEN_MODES };
    let mode_before = app.ui.tool_options.vector_mode;
    VectorModeSelect::new(bind_tracked!(app.ui.tool_options => vector_mode), offered).show(ui);
    // #12910 measured: in an Indexed Color document the Pen lists Shape and keeps the entry enabled,
    // but picking it leaves the tool in Path mode. Only a pick through this dropdown counts: a Shape
    // mode that arrived another way (a tool preset, `penMode`) is not measured and keeps today's
    // behaviour, and the other pens keep theirs.
    if tool == Tool::Pen && mode_before == VectorMode::Path && app.ui.tool_options.vector_mode == VectorMode::Shape && indexed_document(app) {
        app.ui.tool_options.vector_mode = VectorMode::Path;
    }
    Divider::vertical().show(ui);
    if app.ui.tool_options.vector_mode == VectorMode::Shape {
        // #12907: the row leads with the Fill and Stroke swatches, the shape tools' chips with
        // their popups; a pick sets the tool defaults the next Pen shape is painted with.
        crate::shape_options::pen_paint_swatches(app, ui);
        let ppi = document_ppi(app);
        let o = &app.ui.tool_options;
        let look = fw::ShapeAppearance::new(Paint::None, Paint::None, f64::from(o.stroke_width)).stroke_style((&o.shape_stroke).into());
        let out = StrokeCells::new(&look, ppi).corners(true).stroke_open(Bind::bound(&mut app.ui.shape_stroke_open)).show(ui);
        for event in &out.events {
            match event.value() {
                Some(fw::ShapeChange::StrokeWidth(w)) => app.ui.tool_options.stroke_width = *w as f32,
                Some(fw::ShapeChange::StrokeStyle { style, width, .. }) => {
                    app.ui.tool_options.shape_stroke = crate::shape_stroke_ui::StrokeOptions::from_style(style);
                    if let Some(w) = width {
                        app.ui.tool_options.stroke_width = *w as f32;
                    }
                }
                _ => {}
            }
        }
        if let Some(alert) = out.out_of_range {
            crate::option_cells::range_alert(app, ui, &alert);
        }
        // W and H (#12907): 0 px with no shape and the live size of the shape being drawn. What
        // typing into them does was not measured, so it keeps today's behaviour — there was no
        // field and no effect: these fields apply no edit (the Shape tools have no size fields
        // yet either, #2496).
        let (w, h) = pen_shape_size(app, tool);
        // The Field shows the "W:" / "H:" caption, as the reference's row prints them; in a bar a
        // Field already takes its content width. The fields are views: their output applies nothing,
        // so their label offers no scrub handle either (scrub(false), unlike the editing fields).
        let _ = Field::new(label(tl!("W:")))
            .id_salt("pen-w")
            .side(SideLabel::new().scrub(false))
            .show(ui, |ui, _| NumberField::new(label(tl!("W:")), Bind::given(Some(w))).unit_kind(UnitKind::Pixels).show(ui));
        let _ = Field::new(label(tl!("H:")))
            .id_salt("pen-h")
            .side(SideLabel::new().scrub(false))
            .show(ui, |ui, _| NumberField::new(label(tl!("H:")), Bind::given(Some(h))).unit_kind(UnitKind::Pixels).show(ui));
    }
    // Align Edges, enabled in Shape mode only (greyed in Path mode).
    super::alignment::pen_option(app, ui);
    Divider::vertical().show(ui);
    path_operations_menu(app, ui);
    if tool == Tool::Pen {
        Divider::vertical().show(ui);
        // The gear's popover replaces the Rubber Band item (tools-03 D3).
        // PLAN: `PathOptionsOutput::pick_color` (key PATH_OPTIONS_COLOR_KEY) is answered by the owner of the colour picker dialog.
        let _ = PathOptionsPopover::new(label(tl!("Set additional pen and path options")), bind_tracked!(app.ui.tool_options => path_options))
            .show(ui, &mut open.path_options);
        Checkbox::new(bind_tracked!(app.ui.tool_options => pen_auto_add_delete)).label(label(tl!("Auto Add/Delete"))).show(ui);
    }
    let enter = crate::shortcuts::pretty("Enter");
    match tool {
        Tool::FreeformPen => {
            Divider::vertical().show(ui);
            // In a `Field`: a bare control's label only names it, it is not drawn (#16338).
            let picked = Field::new(label(tl!("Curve Fit:")))
                .show(ui, |ui, _| {
                    let field = NumberField::new(label(tl!("Curve Fit:")), Bind::given(Some(f64::from(app.ui.tool_options.curve_fit))));
                    field.range(0.5..=10.0).unit_kind(UnitKind::Pixels).show(ui).value().copied()
                })
                .inner
                .flatten();
            if let Some(Some(v)) = picked {
                app.ui.tool_options.curve_fit = v as f32;
            }
            let alt = crate::shortcuts::pretty("Alt");
            hint(ui, &crate::i18n::fmt(tl!("Drag: draw freehand · {alt}-click: straight segment · Release on the start point: close"), &[("alt", &alt)]));
        }
        Tool::CurvaturePen => hint(
            ui,
            &crate::i18n::fmt(
                tl!("Click: smooth point · Double-click: corner · Drag a point: move · Click first point: close · {key} finish"),
                &[("key", &enter)],
            ),
        ),
        _ => hint(
            ui,
            &crate::i18n::fmt(
                tl!("Click: corner · Drag: smooth · {alt}-drag: break handle · Click or drag first point: close · {key} finish · Esc cancel"),
                &[("alt", &crate::shortcuts::pretty("Alt")), ("key", &enter)],
            ),
        ),
    }
    if app.ui.tool_options.vector_mode == VectorMode::Path {
        Divider::vertical().show(ui);
        make_buttons(app, ui);
    }
}

/// The W / H the Pen Shape-mode bar shows (#12907): the size of the shape being drawn, 0 px when
/// there is none. The Pen and the Curvature Pen draw into `app.ui.pen`, measured with the crate's
/// one finite-guarded path bounds helper (`photocraft_text::area::path_bounds`, anchors and
/// handles): the measured shapes are straight (200 × 200 px), and what a handle beyond the anchors
/// or a typed size does was not measured and changes nothing today. A Freeform Pen trace is not a
/// path while the pointer is down: the tool keeps its live points in its own outline draft and fits
/// them into a path only when the pass ends, so that tool's row measures the draft's points and
/// falls back to `app.ui.pen` (the fitted path of a trace whose commit failed) once no trace is in
/// progress.
fn pen_shape_size(app: &PhotocraftApp, tool: Tool) -> (f64, f64) {
    if tool == Tool::FreeformPen
        && let Some(size) = freeform_trace_size(&app.canvas_tools.vector.freeform.draft)
    {
        return size;
    }
    let Some(pen) = app.ui.pen.as_ref().filter(|p| !p.draft.knots.is_empty()) else { return (0.0, 0.0) };
    photocraft_text::area::path_bounds(&super::pen_path(pen)).map_or((0.0, 0.0), |(x0, y0, x1, y1)| (x1 - x0, y1 - y0))
}

/// The box the Freeform Pen's live trace covers, from the points the tool paints between presses
/// (document units); `None` with no trace in progress. One point is the flat 0 x 0 the pen drafts
/// show. The draft never holds a non-finite point — a non-finite sample is dropped where it comes
/// in — so this needs no guard of its own.
fn freeform_trace_size(draft: &OutlineDraft) -> Option<(f64, f64)> {
    let first = *draft.points().first()?;
    let box_ = draft.points().iter().fold(Rect::from_points(first, first), |box_, &at| box_.union_pt(at));
    Some((box_.width(), box_.height()))
}

/// Path mode's "Make:" row, as in Photoshop's options bar: Selection…, Mask and Shape, from the
/// path being drawn or the active path; all greyed without one.
fn make_buttons(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    hint(ui, tl!("Make:"));
    let pending = app.ui.pen.as_ref().is_some_and(|p| p.draft.knots.len() >= 2);
    let has_path = pending || active_path_name(app).is_some();
    // A draft that finishes into the targeted vector mask has no path of its own to make a mask
    // from, as in the canvas menu's Create Vector Mask.
    let has_mask_path = has_path && !(pending && super::path_ops::targeted_vector_mask(app).is_some());
    // Make: Shape turns the path into a shape layer with the shape fill and stroke.
    let has_shape_path = pending || super::path_ops::shape_source_path(app).is_some();
    let selection = clicked(&Button::new(label(tl!("Selection…"))).enabled(has_path).show(ui));
    let mask = clicked(&Button::new(label(tl!("Mask"))).enabled(has_mask_path).show(ui));
    let shape = clicked(&Button::new(label(tl!("Shape"))).enabled(has_shape_path).show(ui));
    let made = if selection {
        crate::canvas_tool_menu::pen_dialogs::make_selection(app)
    } else if mask {
        make_mask(app)
    } else if shape {
        super::make_shape(app).map(|_| ())
    } else {
        Ok(())
    };
    if let Err(e) = made {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// Make: Mask: a vector mask on the active layer from the path being drawn (finished
/// first, as the canvas menu's Create Vector Mask does) or the active path. The command records
/// the History step Add Vector Mask and turns the Background into a normal layer in that edit.
fn make_mask(app: &mut PhotocraftApp) -> Result<(), String> {
    if app.ui.pen.as_ref().is_some_and(|p| p.draft.knots.len() >= 2) && !super::pen::pen_commit(app, false) {
        return Err(app.ui.status.clone());
    }
    let name =
        active_path_name(app).ok_or_else(|| tl!("No path to make a mask from: draw one with the Pen tool or select one in the Paths panel.").to_string())?;
    app.run("layer.vectorMask.fromPath", json!({"name": name})).map(|_| ())
}

/// Whether the Shape tools are in Path mode: a drag adds its outline to the work path.
pub(crate) fn draws_path(o: &crate::state::ToolOptions) -> bool {
    o.shape_mode == VectorMode::Path
}

/// Whether the active layer supplies the selected-shape options.
pub(super) fn selected_shape(session: &photocraft_engine::Session) -> Option<photocraft_doc::LayerId> {
    let st = session.active()?;
    let layer = st.active_layer?;
    matches!(st.doc.layer(layer)?.content, LayerContent::Shape(_)).then_some(layer)
}

// ---------------------------------------------------------------------------------------------
// Properties (shape layers)

#[cfg(test)]
pub(super) fn solid_paint(c: [f32; 4]) -> photocraft_doc::Fill {
    photocraft_doc::Fill::Solid(photocraft_doc::Color::rgba(c[0], c[1], c[2], c[3]))
}

pub(crate) fn enabled_stroke_width(width: f32) -> f32 {
    if width.is_finite() && width > 0.0 { width } else { 3.0 }
}

/// One corner of the live rectangle, preserving the other three radii.
/// The engine takes the full [top-left, top-right, bottom-right, bottom-left] array.
pub(super) fn corner_radii_patch(radii: [f64; 4], corner: usize, radius: f32) -> Value {
    let mut next = radii;
    if let Some(r) = next.get_mut(corner) {
        *r = f64::from(radius.max(0.0));
    }
    json!({"radii": next})
}

/// The framework's live shape for the layer's.
fn live_of(live: &photocraft_doc::vector::LiveShape) -> fw::LiveShape {
    use photocraft_doc::vector::LiveShape as Doc;
    match live {
        Doc::Rect { rect, radii } => fw::LiveShape::rect(rect[2], rect[3], *radii),
        Doc::Ellipse { rect } => fw::LiveShape::ellipse(rect[2], rect[3]),
        Doc::Polygon { rect, sides, star_ratio, corner_radius, .. } => fw::LiveShape::polygon(rect[2], rect[3], *corner_radius, *sides, *star_ratio),
        // One Corner radius field: a triangle has no sides or star ratio.
        Doc::Triangle { rect, corner_radius } => fw::LiveShape::triangle(rect[2], rect[3], *corner_radius),
        Doc::Line { weight, .. } => fw::LiveShape::line(*weight),
        Doc::ArrowLine { weight, start, end, width, length, concavity, .. } => {
            let mut arrows = Arrowheads::new();
            arrows.start = *start;
            arrows.end = *end;
            arrows.width = *width;
            arrows.length = *length;
            arrows.concavity = *concavity;
            fw::LiveShape::arrow_line(*weight, arrows)
        }
    }
}

/// One field's edit as a `shape.edit` patch, with the colour a solid pick adds to the Recently Used Colors and the
/// gradient a gradient pick adds to the Gradient tab's Recent row.
struct Edit {
    field: &'static str,
    patch: Value,
    recent: Option<[f32; 3]>,
    gradient: Option<photocraft_doc::Fill>,
}

/// The gradient a chip's `paint` stands for over `base`; `None` for a paint that is no gradient.
fn picked_gradient(paint: &Paint, base: &photocraft_doc::Fill) -> Option<photocraft_doc::Fill> {
    match paint {
        Paint::Gradient(preview) => Some(crate::shape_paint_ui::gradient_fill(preview, base)),
        _ => None,
    }
}

/// The `shape.edit` patch of `change` on the layer `sh`; `None` when the change is no edit. `gradients` are the fill's
/// and the stroke's [`ChipLibrary::gradient`](crate::shape_paint_ui::ChipLibrary::gradient), which a gradient paint
/// is laid over.
fn edit_of(change: &fw::ShapeChange, sh: &photocraft_doc::ShapeLayer, gradients: &[photocraft_doc::Fill; 2]) -> Option<Edit> {
    let plain = |field, patch| Some(Edit { field, patch, recent: None, gradient: None });
    let width_now = sh.stroke.as_ref().map_or(0.0, |s| s.width);
    match change {
        fw::ShapeChange::Fill(paint) => Some(Edit {
            field: "fill",
            patch: json!({"fill": crate::shape_paint_ui::fill_param(paint, &gradients[0])}),
            recent: crate::shape_paint_ui::solid_rgb(paint),
            gradient: picked_gradient(paint, &gradients[0]),
        }),
        fw::ShapeChange::Stroke { paint, width, .. } => {
            let color = crate::shape_paint_ui::fill_param(paint, &gradients[1]);
            let stroke = if color.is_null() {
                Value::Null
            } else {
                json!({"color": color, "width": width.unwrap_or_else(|| f64::from(enabled_stroke_width(width_now)))})
            };
            Some(Edit {
                field: "stroke",
                patch: json!({"stroke": stroke}),
                recent: crate::shape_paint_ui::solid_rgb(paint),
                gradient: picked_gradient(paint, &gradients[1]),
            })
        }
        fw::ShapeChange::StrokeWidth(w) => {
            // A step that clamps at 0 on a shape with no stroke is no change.
            if sh.stroke.is_none() && *w <= 0.0 {
                return None;
            }
            plain("stroke-w", json!({"stroke": {"width": w}}))
        }
        fw::ShapeChange::StrokeStyle { style, width, .. } => {
            let mut stroke = crate::shape_stroke_ui::StrokeOptions::from_style(style).params();
            if let Some(w) = width {
                stroke["width"] = json!(w);
            }
            plain("style", json!({"stroke": stroke}))
        }
        fw::ShapeChange::Size { width, height, .. } => {
            let rect = match sh.live.as_ref()? {
                photocraft_doc::vector::LiveShape::Rect { rect, .. }
                | photocraft_doc::vector::LiveShape::Ellipse { rect }
                | photocraft_doc::vector::LiveShape::Polygon { rect, .. }
                | photocraft_doc::vector::LiveShape::Triangle { rect, .. } => *rect,
                _ => return None,
            };
            plain("wh", json!({"rect": [rect[0], rect[1], width.round(), height.round()]}))
        }
        fw::ShapeChange::Radii(new) => {
            let Some(photocraft_doc::vector::LiveShape::Rect { radii, .. }) = sh.live.as_ref() else { return None };
            let mut next = *radii;
            let mut patch = json!({"radii": next});
            for (corner, radius) in new.iter().enumerate() {
                if (radius - next[corner]).abs() > f64::EPSILON {
                    patch = corner_radii_patch(next, corner, *radius as f32);
                    next[corner] = f64::from((*radius as f32).max(0.0));
                }
            }
            plain("radii", patch)
        }
        fw::ShapeChange::CornerRadius(r) => plain("corner-radius", json!({"cornerRadius": r})),
        fw::ShapeChange::Sides(n) => plain("sides", json!({"sides": n})),
        fw::ShapeChange::StarRatio(f) => plain("star", json!({"starRatio": f})),
        fw::ShapeChange::Weight(w) => plain("weight", json!({"weight": w})),
        fw::ShapeChange::Arrowheads(a) => {
            plain("arrow", json!({"arrowStart": a.start, "arrowEnd": a.end, "arrowWidth": a.width, "arrowLength": a.length, "arrowConcavity": a.concavity}))
        }
        _ => None,
    }
}

/// Properties panel for a shape layer: Appearance (fill, stroke) and live shape geometry.
///
/// Each pass reads the layer fresh and applies only the events the panel reports in that pass, so a
/// second pass in the same frame applies nothing the first already did.
pub fn shape_properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, id: photocraft_doc::LayerId) {
    let Some(st) = app.session.active() else { return };
    let doc = st.doc.id;
    let locks = st.doc.effective_locks(id);
    let locked = locks.all || locks.pixels;
    let ppi = f64::from(st.doc.resolution_dpi);
    let Some(sh) = st.doc.layer(id).and_then(|l| match &l.content {
        LayerContent::Shape(sh) => Some(sh.clone()),
        _ => None,
    }) else {
        return;
    };
    let (fg, bg) = (app.session.tools.foreground, app.session.tools.background);
    let appearance = crate::shape_paint_ui::appearance_of(&sh, locked);
    let shown = sh.live.as_ref().map_or_else(|| fw::LiveShape::line(1.0), live_of);
    let target = (doc.0, id.0);
    let was_linked = app.ui.arrow_default == Some(target);
    let mut linked = was_linked;
    let recent_colors: Vec<Srgba> = crate::paint_picker::recent_colors(&app.session).into_iter().map(|c| Srgba::new(c[0], c[1], c[2], 1.0)).collect();
    let sampled =
        [fw::ShapePaint::Fill, fw::ShapePaint::Stroke].into_iter().find_map(|p| crate::screen_picker::take(app, ("shape-properties", p)).map(|c| (p, c)));
    let groups = crate::shape_gradient_ui::ShapeGradients::groups_of(&app.session.presets().gradients, fg, bg);
    let gradients = crate::shape_gradient_ui::ShapeGradients { groups, current: &app.session.presets().gradient, recent: &mut app.ui.recent_gradients, fg, bg };
    let current = [sh.fill.clone(), sh.stroke.as_ref().map(|s| s.paint.clone())];
    let mut library = crate::shape_paint_ui::ChipLibrary::new(
        &recent_colors,
        &app.session.presets().swatches,
        &mut app.ui.paint_folders,
        gradients,
        &mut app.ui.shape_gradient_state,
        current,
    );
    let output = ShapePanel::new(label(tl!("Shape properties")), &shown, &appearance, ppi)
        .sections(true, sh.live.is_some())
        .sampled(sampled)
        .stroke_open(Bind::bound(&mut app.ui.shape_stroke_open))
        .chips(|which, chip, chip_ui| crate::shape_paint_ui::decorate_chip(chip, which, &mut library, chip_ui))
        .fallbacks(Srgba::new(fg[0], fg[1], fg[2], fg[3]), Srgba::new(bg[0], bg[1], bg[2], bg[3]))
        .arrow_default(Bind::bound(&mut linked))
        .show(ui);
    let fw::ShapePanelOutput { events, pick_color, screen_pick, out_of_range, .. } = output;
    let paint_gradients = [library.gradient(fw::ShapePaint::Fill), library.gradient(fw::ShapePaint::Stroke)];
    super::alignment::option(app, ui, Some(id), false, true);

    let mut applied = false;
    let mut recent: Option<[f32; 3]> = None;
    let mut remembered: Vec<photocraft_doc::Fill> = Vec::new();
    let prefix = |field: &str| format!("shape-{doc:?}-{}-{field}", id.0);
    for event in &events {
        let Some(change) = event.value() else { continue };
        let Some(edit) = edit_of(change, &sh, &paint_gradients) else { continue };
        let mut patch = edit.patch;
        patch["layer"] = json!(id.0);
        if let Some(key) = history_keys::coalesce(&prefix(edit.field), event) {
            patch["coalesce"] = json!(key);
        }
        match app.run("shape.edit", patch) {
            Ok(_) => {
                applied = true;
                recent = edit.recent.or(recent);
                if !matches!(event, EditEvent::Update { .. } | EditEvent::Cancel { .. }) {
                    remembered.extend(edit.gradient);
                }
            }
            Err(e) => {
                app.ui.status = e;
                app.ui.status_error = true;
            }
        }
    }
    if applied && let Some(rgb) = recent {
        // A colour picked in a Fill/Stroke popover joins the Recently Used Colors once the edit took it.
        crate::paint_picker::remember(app, rgb);
    }
    for g in &remembered {
        crate::shape_gradient_ui::remember_recent(&mut app.ui.recent_gradients, g);
    }
    if linked != was_linked {
        app.ui.arrow_default = linked.then_some(target);
        if linked {
            copy_arrow_to_tool_defaults(app, id);
        }
    } else if applied && linked {
        // Only a successful Properties edit updates new-arrow defaults, and only while the box is checked.
        copy_arrow_to_tool_defaults(app, id);
    }
    // The chip's Color Picker… opens the app's picker for that paint; it starts from the layer's paint, so the request's colour is not used.
    if let Some((which, _request)) = pick_color {
        let which = match which {
            fw::ShapePaint::Fill => crate::shape_fill_ui::Paint::Fill,
            _ => crate::shape_fill_ui::Paint::Stroke,
        };
        let _ = crate::shape_fill_ui::open_paint(app, which, Some(id), false, fg);
    }
    if let Some((which, gesture)) = screen_pick {
        crate::screen_picker::hold(app, ui.ctx(), ("shape-properties", which), gesture);
    }
    // A Concavity typed past -50..50 % raises the reference's alert (#13258).
    if let Some(alert) = out_of_range {
        crate::option_cells::range_alert(app, ui, &alert);
    }
}

#[cfg(test)]
#[path = "line_arrow_concavity_field_tests.rs"]
mod line_arrow_concavity_field_tests;

#[cfg(test)]
#[path = "pen_make_mask_tests.rs"]
mod pen_make_mask_tests;

#[cfg(test)]
#[path = "custom_shape_path_make_tests.rs"]
mod custom_shape_path_make_tests;

#[cfg(test)]
#[path = "triangle_path_make_tests.rs"]
mod triangle_path_make_tests;
