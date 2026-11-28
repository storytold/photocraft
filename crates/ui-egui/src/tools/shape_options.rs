//! Shape toolbar edits share the engine commands used by the Properties panel.

use craft::behavior::{EditEvent, Mixed};
use craft::color_ui::ui::{Paint, PaintChip};
use craft::draw::stroke::StrokeStyle;
use craft::draw::ui::RADIUS_RANGE;
use craft::draw::ui::shape::{ShapeAppearance, ShapeBar, ShapeChange, ShapePaint, ShapeToolSettings, ShapeWords};
use craft::draw::ui::shape_drag::ShapeKind;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _, bind_tracked};
use craft::ui::dialog::{Alert, HostedAlert};
use craft::ui::display::Divider;
use craft::ui::form::Field;
use craft::ui::runtime::env::{Env, Label};
use photocraft_doc::{Fill, LayerContent};
use photocraft_engine::presets::PresetsExt;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::shape_gradient_ui::{self as shape_gradient, ShapeGradients};
use crate::shape_paint_ui::{ChipLibrary, PaintChange, decorate_chip, gradient_fill, paint_of, paint_param, rgba, srgba};
use crate::shape_stroke_ui::StrokeOptions;
use crate::state::Tool;
use crate::vector_ui::enabled_stroke_width;

fn solid(color: [f32; 4]) -> photocraft_doc::Fill {
    photocraft_doc::Fill::Solid(photocraft_doc::Color::rgba(color[0], color[1], color[2], color[3]))
}

/// The shape layer the options bar edits.
struct SelectedShape {
    doc: photocraft_doc::DocId,
    layer: photocraft_doc::LayerId,
    fill: Option<photocraft_doc::Fill>,
    stroke: Option<photocraft_doc::ShapeStroke>,
    live: Option<photocraft_doc::LiveShape>,
}

fn selected_shape(session: &photocraft_engine::Session) -> Option<SelectedShape> {
    let st = session.active()?;
    let layer = st.active_layer?;
    match &st.doc.layer(layer)?.content {
        LayerContent::Shape(sh) => Some(SelectedShape { doc: st.doc.id, layer, fill: sh.fill.clone(), stroke: sh.stroke.clone(), live: sh.live.clone() }),
        _ => None,
    }
}

/// The geometry the bar shows: the tool's, or for Path and Direct Selection the selected live shape's.
/// `None` for a tool that has no shape options.
fn kind_of(tool: Tool, target: Option<&SelectedShape>) -> Option<ShapeKind> {
    Some(match tool {
        Tool::Rectangle => ShapeKind::Rectangle,
        Tool::EllipseShape => ShapeKind::Ellipse,
        Tool::Triangle => ShapeKind::Triangle,
        Tool::Polygon => ShapeKind::Polygon,
        Tool::Star => ShapeKind::Star,
        Tool::Line => ShapeKind::Line,
        Tool::CustomShape => ShapeKind::Custom,
        Tool::PathSelection | Tool::DirectSelection => match target.and_then(|s| s.live.as_ref()) {
            Some(photocraft_doc::LiveShape::Rect { .. }) => ShapeKind::Rectangle,
            Some(photocraft_doc::LiveShape::Ellipse { .. }) => ShapeKind::Ellipse,
            _ => ShapeKind::Custom,
        },
        _ => return None,
    })
}

/// The value an event proposes: a running gesture's, a committed or one-step value, or the value a
/// cancelled gesture restores.
fn proposal(event: &EditEvent<ShapeChange>) -> Option<&ShapeChange> {
    match event {
        EditEvent::Update { value, .. } => Some(value),
        EditEvent::Commit { after, .. } | EditEvent::Set { after, .. } => Some(after),
        EditEvent::Cancel { before: Some(Mixed::Same(value)), .. } => Some(value),
        _ => None,
    }
}

/// The history key part of a change: edits of one field in one gesture coalesce into one step.
fn field_of(change: &ShapeChange) -> &'static str {
    match change {
        ShapeChange::Fill(_) | ShapeChange::FillOn(_) => "fill",
        ShapeChange::Stroke { .. } => "stroke",
        ShapeChange::StrokeWidth(_) => "stroke-w",
        ShapeChange::StrokeStyle { .. } => "stroke-style",
        ShapeChange::Radii(_) | ShapeChange::CornerRadius(_) => "radius",
        ShapeChange::Sides(_) => "sides",
        ShapeChange::Weight(_) => "weight",
        _ => "other",
    }
}

/// Whether a cancelled gesture's restore value is already what the bar shows: no `Update` of the gesture
/// reached the document, so a restore would add an empty history step and a cancelled pick must not run.
fn already_shown(change: &ShapeChange, appearance: &ShapeAppearance, settings: &ShapeToolSettings) -> bool {
    match change {
        ShapeChange::Fill(p) => appearance.fill == *p,
        ShapeChange::Stroke { paint, .. } => appearance.stroke == *paint,
        ShapeChange::StrokeWidth(w) => appearance.stroke_width == *w,
        ShapeChange::StrokeStyle { style, .. } => appearance.stroke_style == *style,
        ShapeChange::CornerRadius(r) => settings.corner_radius == *r,
        ShapeChange::Sides(n) => settings.sides == *n,
        ShapeChange::Weight(w) => settings.line_weight == *w,
        _ => false,
    }
}

/// The paint change a chip's paint stands for; a gradient is [`gradient_fill`] over `gradient`, the chip's
/// [`ChipLibrary::gradient`].
fn paint_change(paint: &craft::color_ui::ui::Paint, gradient: &Fill) -> Option<PaintChange> {
    match paint {
        craft::color_ui::ui::Paint::None => Some(PaintChange::None),
        craft::color_ui::ui::Paint::Solid(c) => Some(PaintChange::Color(rgba(*c))),
        craft::color_ui::ui::Paint::Gradient(preview) => Some(PaintChange::Gradient(gradient_fill(preview, gradient))),
        _ => None,
    }
}

/// Applies a paint change to the tool defaults; false when the engine refused it.
fn apply_tool_paint(app: &mut PhotocraftApp, stroke: bool, change: &PaintChange) -> bool {
    match set_tool_paint(app, stroke, change.clone()) {
        Ok(()) => true,
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
            false
        }
    }
}

/// The `shape.edit` parameters for a paint change of the fill or the stroke (`width` the stroke width in force).
fn paint_edit(stroke: bool, change: &PaintChange, width: f32) -> Value {
    match (stroke, change) {
        (false, PaintChange::None) => json!({"fill": null}),
        (false, PaintChange::Color(c)) => json!({"fill": c}),
        (false, PaintChange::Gradient(g)) => json!({"fill": paint_param(g)}),
        (true, PaintChange::None) => json!({"stroke": null}),
        (true, PaintChange::Color(c)) => json!({"stroke": {"color": c, "width": enabled_stroke_width(width)}}),
        (true, PaintChange::Gradient(g)) => json!({"stroke": {"color": paint_param(g), "width": enabled_stroke_width(width)}}),
    }
}

/// The Shape tools' options. `paint` shows Fill and Stroke: off in Path mode, where a drag draws
/// only an outline, but the tool's geometry (radius, sides, weight, custom shape) stays.
///
/// A pass applies only its own edits: the values are read from the app at the start of each pass
/// and `ShapeBar` reports an edit only in the pass that read the input, so a second pass in the
/// same frame sees the stored result and writes nothing.
/// The bar's number fields: the next shape's, or the selected rectangle's own radius when the bar edits it.
fn tool_settings(o: &crate::state::ToolOptions, kind: ShapeKind, live: Option<&photocraft_doc::LiveShape>, rect_target: bool) -> ShapeToolSettings {
    let mut settings = ShapeToolSettings::default();
    settings.fill_on = o.shape_fill;
    settings.corner_radius = match live {
        Some(photocraft_doc::LiveShape::Rect { radii, .. }) if rect_target => radii[0],
        _ => f64::from(o.corner_radius),
    };
    // The Star tool's points are its own, apart from the Polygon tool's sides (#16867).
    settings.sides = if kind == ShapeKind::Star { o.star_points } else { o.polygon_sides };
    settings.line_weight = f64::from(o.line_weight);
    settings.stroke_width = f64::from(o.stroke_width);
    settings
}

/// What the bar shows after the framework's cells for one tool: the Polygon's own corner radius.
/// (Align Edges, every shape tool's, sits at the bar's right end: `vector_ui::alignment`.)
fn tool_extras(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool) {
    if tool == Tool::Polygon {
        // The tool's own corner radius for the next polygon, after Sides (#13302); the active shape is not touched.
        // A value typed past the Rectangle Radius's range takes the closest one and says so.
        Divider::vertical().show(ui);
        let out = crate::option_cells::number(tl!("Radius"), bind_tracked!(app.ui.tool_options => polygon_corner_radius), RADIUS_RANGE).unit("px").show(ui);
        if let Some(alert) = out.out_of_range {
            crate::option_cells::range_alert(app, ui, &alert);
        }
    }
}

/// The points a Star tool's star may have: whole numbers 3 to 20, as the reference's options bar (#16867).
const STAR_POINTS: std::ops::RangeInclusive<f64> = 3.0..=20.0;

/// The star points a Sides change of `n` sets (`None`: keep them), and whether it was a value past 20 the user is
/// told about (the reference's "An integer between 3 and 20 is required" alert). The framework's Sides cell holds
/// 3 to 100 and reports values past that itself; this covers 21 to 100 until its Star cell holds 3 to 20.
/// - A proposal past 20 while the gesture runs (`committed` false) keeps the points: a clamped value here would end
///   the field's gesture before it commits.
/// - The committed value is clamped. `shown` is the value the field showed: a committed 21 from 20 is an arrow step
///   past the end, which stops there without an alert, as the field's own ends do.
fn star_points(n: u32, shown: u32, committed: bool) -> (Option<u32>, bool) {
    let (low, high) = (*STAR_POINTS.start() as u32, *STAR_POINTS.end() as u32);
    if n <= high {
        return (Some(n.max(low)), false);
    }
    if !committed {
        return (None, false);
    }
    let stepped_past_end = n == high + 1 && shown == high;
    (Some(high), !stepped_past_end)
}

/// Stores the Star tool's points from a Sides change of `n` ([`star_points`]); true when it was a value past 20 the
/// user is told about.
fn set_star_points(app: &mut PhotocraftApp, n: u32, shown: u32, committed: bool) -> bool {
    let (points, alert) = star_points(n, shown, committed);
    if let Some(points) = points {
        app.ui.tool_options.star_points = points;
    }
    alert
}

/// The range alert of the bar's cell, with a star's count past the shared Sides range (3 to 100) naming the star's own.
fn star_range(kind: ShapeKind, mut range: craft::ui::text::RangeAlert) -> craft::ui::text::RangeAlert {
    if kind == ShapeKind::Star {
        range.range = STAR_POINTS;
        range.kept = range.kept.clamp(*STAR_POINTS.start(), *STAR_POINTS.end());
    }
    range
}

/// The alert of a star points value typed past 20: the text fields' whole-number range sentence over 3 to 20.
fn star_points_range_alert(title: String, env: &Env) -> Alert<'static> {
    let words = env.words::<craft::ui::text::TextWords>();
    let units = env.units();
    let (low, high) = (units.number(*STAR_POINTS.start(), 0..=0), units.number(*STAR_POINTS.end(), 0..=0));
    Alert::new(title, words.range_integer.fill(&[("low", low.as_str()), ("high", high.as_str())]))
}

/// The title of the alert for a number typed outside its range.
fn range_title(kind: ShapeKind) -> String {
    match kind {
        ShapeKind::Polygon | ShapeKind::Star => tl!("Sides"),
        ShapeKind::Line => tl!("Weight"),
        _ => tl!("Radius"),
    }
    .to_owned()
}

/// The paint a chip's Color Picker request edits, with the colour it starts from when the layer has none.
fn paint_dialog_target<C>(which: ShapePaint, fg: C, bg: C) -> Option<(crate::shape_fill_ui::Paint, C)> {
    match which {
        ShapePaint::Fill => Some((crate::shape_fill_ui::Paint::Fill, fg)),
        ShapePaint::Stroke => Some((crate::shape_fill_ui::Paint::Stroke, bg)),
        _ => None,
    }
}

/// The value a W or H field's event proposes, as [`proposal`] reads the bar's events.
fn number_proposal(event: &EditEvent<Option<f64>>) -> Option<f64> {
    match event {
        EditEvent::Update { value, .. } => *value,
        EditEvent::Commit { after, .. } | EditEvent::Set { after, .. } => *after,
        EditEvent::Cancel { before: Some(Mixed::Same(value)), .. } => *value,
        _ => None,
    }
}

/// The Line tool's W, link and H fields (#13250; nothing for another tool): the active line's length and thickness in its own
/// rotated frame, 0 px with no live line. A typed W or H resizes the line along or across itself
/// (`shape.edit {transform}`, one "Scale Paths" step per gesture); the tool's own Weight and
/// arrowhead settings for the next line stay as they are. What the link does was not measured: it
/// shows off and changes nothing.
fn line_size_fields(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool, target: Option<&SelectedShape>) {
    use craft::ui::contract::{ShowContainer as _, SideLabel};
    use craft::ui::form::Field;
    use craft::ui::runtime::env::{Label, UnitKind};
    use craft::ui::text::NumberField;

    if tool != Tool::Line {
        return;
    }
    let live = target.and_then(|s| s.live.as_ref().map(|l| (s.doc, s.layer, l.clone())));
    let size = live.as_ref().and_then(|(_, _, l)| photocraft_engine::vector_cmds::line_frame_size(l));
    let (w, h) = size.unwrap_or((0.0, 0.0));
    let field = |ui: &mut egui::Ui, key: &str, caption: &str, value: f64| {
        let caption = Label::from(caption.to_owned());
        Field::new(caption.clone())
            .id_salt(key)
            .side(SideLabel::new().scrub(true))
            .show(ui, |ui, _| {
                // Two decimals, as the reference shows a line's length (223.61 px); a whole number shows none.
                let codec = craft::behavior::NumberCodec::new(Env::of(ui).units().reader(Some(UnitKind::Pixels)), 2);
                NumberField::new(caption.clone(), Bind::given(Some(value))).unit_kind(UnitKind::Pixels).codec(codec).scrub_by_label().show(ui).edit.events
            })
            .inner
            .unwrap_or_default()
    };
    Divider::vertical().show(ui);
    let w_events = field(ui, "line-w", tl!("W:"), w);
    let _ = craft::ui::text::link_toggle(Label::from(tl!("Maintain aspect ratio").to_owned()), Bind::given(false)).show(ui);
    let h_events = field(ui, "line-h", tl!("H:"), h);
    let (Some((doc, layer, live)), Some((w, h))) = (live, size) else { return };
    for (axis, events) in [("w", w_events), ("h", h_events)] {
        for event in &events {
            let Some(value) = number_proposal(event) else { continue };
            let (length, thickness) = if axis == "w" { (value, h) } else { (w, value) };
            // A cleared, zero, negative or non-finite value is not a size: no edit.
            let Some(m) = photocraft_engine::vector_cmds::line_frame_scale(&live, length, thickness) else { continue };
            if m == photocraft_geom::Affine::IDENTITY {
                continue;
            }
            let key = crate::history_keys::coalesce(&format!("line-size-{doc:?}-{}-{axis}", layer.0), event);
            if let Err(e) = app.run("shape.edit", json!({"layer": layer.0, "transform": m.m, "coalesce": key})) {
                app.ui.status = e;
                app.ui.status_error = true;
            }
            // The first value applied changes the line: later events of this pass would scale from stale sizes.
            return;
        }
    }
}

pub(crate) fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool, paint: bool) {
    // Path mode edits only the next outline: an active shape layer is not the target.
    let target = if paint { selected_shape(&app.session) } else { None };
    let Some(kind) = kind_of(tool, target.as_ref()) else { return };
    // Fill and Stroke edit the selected shape layer too (#3347), and still set the next shape's.
    let (fg, bg) = (app.session.tools.foreground, app.session.tools.background);
    let o = &app.ui.tool_options;
    let (fill, stroke) = match &target {
        Some(s) => (s.fill.clone(), s.stroke.clone()),
        None => {
            let fill = o.shape_fill_gradient.clone().unwrap_or_else(|| solid(fg));
            let stroke = o.shape_stroke_gradient.clone().unwrap_or_else(|| solid(bg));
            (o.shape_fill.then_some(fill), (o.stroke_width > 0.0).then(|| o.shape_stroke.stroke(o.stroke_width, stroke)))
        }
    };
    let width = stroke.as_ref().map_or(0.0, |s| s.width);
    let has_stroke = stroke.is_some();
    // The Rectangle tool's Radius is the next rectangle's (#13338); the active shape is edited only from the
    // selection tools, whose Radius is its own.
    let rect_target = matches!(tool, Tool::PathSelection | Tool::DirectSelection)
        && matches!(target.as_ref().and_then(|s| s.live.as_ref()), Some(photocraft_doc::LiveShape::Rect { .. }));
    let settings = tool_settings(o, kind, target.as_ref().and_then(|s| s.live.as_ref()), rect_target);
    let style = stroke.as_ref().map_or_else(|| o.shape_stroke.clone(), StrokeOptions::from);
    let locked = target.as_ref().is_some_and(|s| {
        app.session.active().is_some_and(|st| {
            let locks = st.doc.effective_locks(s.layer);
            locks.all || locks.pixels
        })
    });
    let appearance = craft::draw::ui::shape::ShapeAppearance::new(paint_of(fill.as_ref()), paint_of(stroke.as_ref().map(|s| &s.paint)), f64::from(width))
        .stroke_style(StrokeStyle::from(&style))
        .locked(locked);
    let ppi = app.session.active().map_or(72.0, |st| f64::from(st.doc.resolution_dpi));
    let sampled = [ShapePaint::Fill, ShapePaint::Stroke].into_iter().find_map(|p| crate::screen_picker::take(app, ("shape-bar", p)).map(|c| (p, c)));
    let recent_colors: Vec<craft::theme::Srgba> =
        crate::paint_picker::recent_colors(&app.session).into_iter().map(|c| srgba([c[0], c[1], c[2], 1.0])).collect();
    let groups = ShapeGradients::groups_of(&app.session.presets().gradients, fg, bg);
    let gradients = ShapeGradients { groups, current: &app.session.presets().gradient, recent: &mut app.ui.recent_gradients, fg, bg };
    let current = [fill.clone(), stroke.as_ref().map(|s| s.paint.clone())];
    let mut library =
        ChipLibrary::new(&recent_colors, &app.session.presets().swatches, &mut app.ui.paint_folders, gradients, &mut app.ui.shape_gradient_state, current);
    let out = ShapeBar::new(kind, &settings, &appearance, ppi)
        .paint(paint)
        .fallbacks(srgba(fg), srgba(bg))
        .sampled(sampled)
        .stroke_open(Bind::bound(&mut app.ui.shape_stroke_open))
        .chips(|which, chip, chip_ui| decorate_chip(chip, which, &mut library, chip_ui))
        .show(ui);
    let (fill_gradient, stroke_gradient) = (library.gradient(ShapePaint::Fill), library.gradient(ShapePaint::Stroke));
    line_size_fields(app, ui, tool, target.as_ref());
    tool_extras(app, ui, tool);

    let mut recent: Option<[f32; 3]> = None;
    let mut remembered: Vec<Fill> = Vec::new();
    let mut star_points_alert = false;
    for event in &out.events {
        let Some(change) = proposal(event) else { continue };
        if matches!(event, EditEvent::Cancel { .. }) && already_shown(change, &appearance, &settings) {
            continue;
        }
        let committed = !matches!(event, EditEvent::Update { .. } | EditEvent::Cancel { .. });
        let mut edit: Option<Value> = None;
        let mut applied = true;
        let mut picked: Option<(bool, PaintChange)> = None;
        match change {
            ShapeChange::Fill(p) => {
                if let Some(pc) = paint_change(p, &fill_gradient) {
                    applied = apply_tool_paint(app, false, &pc);
                    edit = Some(paint_edit(false, &pc, width));
                    picked = Some((false, pc));
                }
            }
            ShapeChange::Stroke { paint: p, .. } => {
                if let Some(pc) = paint_change(p, &stroke_gradient) {
                    applied = apply_tool_paint(app, true, &pc);
                    edit = Some(paint_edit(true, &pc, width));
                    picked = Some((true, pc));
                }
            }
            ShapeChange::FillOn(on) => {
                app.ui.tool_options.shape_fill = *on;
                let o = &app.ui.tool_options;
                edit = Some(json!({"fill": if *on { o.shape_fill_gradient.as_ref().map_or_else(|| json!(fg), paint_param) } else { Value::Null }}));
            }
            ShapeChange::StrokeWidth(w) => {
                let w = *w as f32;
                app.ui.tool_options.stroke_width = w;
                // A 0 px "change" (an arrow step clamped at the minimum) must not add a stroke.
                if has_stroke || w > 0.0 {
                    let mut p = json!({"width": w});
                    if !has_stroke {
                        p["color"] = app.ui.tool_options.shape_stroke_gradient.as_ref().map_or_else(|| json!(bg), paint_param);
                    }
                    edit = Some(json!({"stroke": p}));
                }
            }
            ShapeChange::StrokeStyle { style, width: enabled, .. } => {
                let options = StrokeOptions::from_style(style);
                let mut p = options.params();
                if let Some(w) = enabled {
                    p["width"] = json!(w);
                }
                if !has_stroke {
                    p["color"] = app.ui.tool_options.shape_stroke_gradient.as_ref().map_or_else(|| json!(bg), paint_param);
                }
                app.ui.tool_options.shape_stroke = options;
                app.ui.tool_options.stroke_width = enabled_stroke_width(app.ui.tool_options.stroke_width);
                edit = Some(json!({"stroke": p}));
            }
            ShapeChange::CornerRadius(r) => {
                let r = *r as f32;
                app.ui.tool_options.corner_radius = r;
                if rect_target {
                    let rf = f64::from(r);
                    edit = Some(json!({"radii": [rf, rf, rf, rf]}));
                }
            }
            ShapeChange::Radii(r) => {
                app.ui.tool_options.corner_radius = r[0] as f32;
                if rect_target {
                    edit = Some(json!({"radii": r}));
                }
            }
            ShapeChange::Sides(n) if kind == ShapeKind::Star => star_points_alert |= set_star_points(app, *n, settings.sides, committed),
            ShapeChange::Sides(n) => app.ui.tool_options.polygon_sides = *n,
            ShapeChange::Weight(w) => app.ui.tool_options.line_weight = *w as f32,
            _ => {}
        }
        if let (Some(mut p), Some(s)) = (edit, &target) {
            p["layer"] = json!(s.layer.0);
            let key = crate::history_keys::coalesce(&format!("shape-options-{:?}-{}-{}", s.doc, s.layer.0, field_of(change)), event);
            p["coalesce"] = json!(key);
            if let Err(e) = app.run("shape.edit", p) {
                app.ui.status = e;
                app.ui.status_error = true;
                applied = false;
            }
        }
        // A swatch picked in a Fill/Stroke popover joins the Recently Used Colors once it was applied.
        if applied && committed {
            match picked {
                Some((_, PaintChange::Color(c))) => recent = Some([c[0], c[1], c[2]]),
                Some((_, PaintChange::Gradient(g))) => remembered.push(g),
                _ => {}
            }
        }
    }
    if let Some(rgb) = recent {
        crate::paint_picker::remember(app, rgb);
    }
    for g in &remembered {
        shape_gradient::remember_recent(&mut app.ui.recent_gradients, g);
    }
    if let Some((which, _request)) = out.pick_color {
        // The dialog starts from the layer's own paint, so the chip's request is not needed.
        let Some((which, fallback)) = paint_dialog_target(which, fg, bg) else { return };
        crate::shape_fill_ui::open_paint(app, which, target.as_ref().map(|s| s.layer), true, fallback);
    }
    if let Some((which, gesture)) = out.screen_pick {
        crate::screen_picker::hold(app, ui.ctx(), ("shape-bar", which), gesture);
    }
    let alert = match out.out_of_range {
        Some(range) => Some(Alert::range(range_title(kind), &star_range(kind, range), &Env::of(ui))),
        None => star_points_alert.then(|| star_points_range_alert(range_title(kind), &Env::of(ui))),
    };
    if let Some(alert) = alert {
        // An `Err` is the alert that was not opened because one is already up.
        let _ = app.framework.dialogs.open_unless_open(HostedAlert::new(alert));
    }
    if tool == Tool::CustomShape {
        crate::preset_panels::shape_picker(app, ui);
    }
}

/// The value a chip event proposes: like [`proposal`], for the paint a `PaintChip` reports.
fn paint_proposal(event: &EditEvent<Paint>) -> Option<&Paint> {
    match event {
        EditEvent::Update { value, .. } => Some(value),
        EditEvent::Commit { after, .. } | EditEvent::Set { after, .. } => Some(after),
        EditEvent::Cancel { before: Some(Mixed::Same(value)), .. } => Some(value),
        _ => None,
    }
}

/// The Pen family's Shape-mode Fill and Stroke swatches (#12907): the shape tools' chips with the
/// same popups (Recently Used Colors, the Swatches list, the Gradient tab and Color Picker…),
/// under the reference's "Fill:" and "Stroke:" labels. A pick writes the tool's shape fill and
/// stroke defaults — the paints `vector_ui::shapes` gives the next Pen shape — and runs no
/// document command: as in the Shape tools' bar with no shape layer selected, only the tool
/// changes.
pub(crate) fn pen_paint_swatches(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let (fg, bg) = (app.session.tools.foreground, app.session.tools.background);
    let o = &app.ui.tool_options;
    // The same tool-default paints the Shape tools' bar shows when no shape layer is selected.
    let fill = o.shape_fill_gradient.clone().unwrap_or_else(|| solid(fg));
    let stroke_paint = o.shape_stroke_gradient.clone().unwrap_or_else(|| solid(bg));
    let (fill, stroke) = (o.shape_fill.then_some(fill), (o.stroke_width > 0.0).then(|| o.shape_stroke.stroke(o.stroke_width, stroke_paint)));
    let width = stroke.as_ref().map_or(0.0, |s| f64::from(s.width));
    let appearance = ShapeAppearance::new(paint_of(fill.as_ref()), paint_of(stroke.as_ref().map(|s| &s.paint)), width).stroke_style((&o.shape_stroke).into());
    let sampled = [ShapePaint::Fill, ShapePaint::Stroke].into_iter().find_map(|p| crate::screen_picker::take(app, ("pen-bar", p)).map(|c| (p, c)));
    let recent_colors: Vec<craft::theme::Srgba> =
        crate::paint_picker::recent_colors(&app.session).into_iter().map(|c| srgba([c[0], c[1], c[2], 1.0])).collect();
    let groups = ShapeGradients::groups_of(&app.session.presets().gradients, fg, bg);
    let gradients = ShapeGradients { groups, current: &app.session.presets().gradient, recent: &mut app.ui.recent_gradients, fg, bg };
    let current = [fill.clone(), stroke.as_ref().map(|s| s.paint.clone())];
    let mut library =
        ChipLibrary::new(&recent_colors, &app.session.presets().swatches, &mut app.ui.paint_folders, gradients, &mut app.ui.shape_gradient_state, current);
    let words = Env::of(ui).words::<ShapeWords>();
    // The Field's label is the control's accessible name, so the swatches answer to the
    // reference's "Fill:" and "Stroke:"; inside, the chips keep the shape tools' names and tips.
    let fill_out = Field::new(Label::from(tl!("Fill:").to_owned())).id_salt("pen-fill").show(ui, |ui, _| {
        let mut chip = PaintChip::new(words.fill_color.clone(), Bind::given(appearance.fill.clone()), srgba(fg)).tooltip(words.set_fill_type.clone());
        if let Some((_, color)) = sampled.filter(|(target, _)| *target == ShapePaint::Fill) {
            chip = chip.sampled(color);
        }
        decorate_chip(chip, ShapePaint::Fill, &mut library, ui)
    });
    let stroke_out = Field::new(Label::from(tl!("Stroke:").to_owned())).id_salt("pen-stroke").show(ui, |ui, _| {
        let mut chip = PaintChip::new(words.stroke_color.clone(), Bind::given(appearance.stroke.clone()), srgba(bg)).tooltip(words.set_stroke_type.clone());
        if let Some((_, color)) = sampled.filter(|(target, _)| *target == ShapePaint::Stroke) {
            chip = chip.sampled(color);
        }
        decorate_chip(chip, ShapePaint::Stroke, &mut library, ui)
    });
    let (fill_gradient, stroke_gradient) = (library.gradient(ShapePaint::Fill), library.gradient(ShapePaint::Stroke));
    let mut recent: Option<[f32; 3]> = None;
    let mut remembered: Vec<Fill> = Vec::new();
    for (is_stroke, gradient, out) in [(false, fill_gradient, fill_out.inner), (true, stroke_gradient, stroke_out.inner)] {
        let Some(out) = out else { continue };
        let which = if is_stroke { ShapePaint::Stroke } else { ShapePaint::Fill };
        let shown = if is_stroke { &appearance.stroke } else { &appearance.fill };
        for event in &out.edit.events {
            let Some(paint) = paint_proposal(event) else { continue };
            // A cancelled gesture restoring what the chip already shows reached no edit: run nothing.
            if matches!(event, EditEvent::Cancel { .. }) && shown == paint {
                continue;
            }
            let Some(change) = paint_change(paint, &gradient) else { continue };
            let applied = apply_tool_paint(app, is_stroke, &change);
            if applied && !matches!(event, EditEvent::Update { .. } | EditEvent::Cancel { .. }) {
                match &change {
                    PaintChange::Color(c) => recent = Some([c[0], c[1], c[2]]),
                    PaintChange::Gradient(g) => remembered.push(g.clone()),
                    PaintChange::None => {}
                }
            }
        }
        if let Some(gesture) = out.screen_pick {
            crate::screen_picker::hold(app, ui.ctx(), ("pen-bar", which), gesture);
        }
        if out.pick_color.is_some() {
            // The full Color Picker starts from the tool's paint; no layer is edited.
            if let Some((which, fallback)) = paint_dialog_target(which, fg, bg) {
                crate::shape_fill_ui::open_paint(app, which, None, true, fallback);
            }
        }
    }
    if let Some(rgb) = recent {
        crate::paint_picker::remember(app, rgb);
    }
    for g in &remembered {
        shape_gradient::remember_recent(&mut app.ui.recent_gradients, g);
    }
}

/// Tool-bar paint changes affect the next shape; Properties edits the selected shape instead.
pub(crate) fn set_tool_paint(app: &mut PhotocraftApp, stroke: bool, change: PaintChange) -> Result<(), String> {
    set_tool_paint_with_inks(app, stroke, change, None)
}

pub(crate) fn set_tool_paint_with_inks(app: &mut PhotocraftApp, stroke: bool, change: PaintChange, inks: Option<[f32; 4]>) -> Result<(), String> {
    // A colour takes the place of a gradient; No Color keeps it for when the paint comes back.
    let o = &mut app.ui.tool_options;
    let gradient = if stroke { &mut o.shape_stroke_gradient } else { &mut o.shape_fill_gradient };
    match &change {
        PaintChange::Gradient(g) => *gradient = Some(g.clone()),
        PaintChange::Color(_) => *gradient = None,
        PaintChange::None => {}
    }
    if let PaintChange::Color(color) = change {
        let role = if stroke { "background" } else { "foreground" };
        let mut params = json!({role: color});
        if let Some(inks) = inks {
            params[format!("{role}Cmyk")] = json!(inks);
        }
        app.run("tools.setColors", params)?;
    }
    if stroke {
        app.ui.tool_options.stroke_width = if change == PaintChange::None { 0.0 } else { enabled_stroke_width(app.ui.tool_options.stroke_width) };
    } else {
        app.ui.tool_options.shape_fill = change != PaintChange::None;
    }
    Ok(())
}

#[cfg(test)]
#[path = "rectangle_radius_range_tests.rs"]
mod rectangle_radius_range_tests;

#[cfg(test)]
#[path = "rectangle_radius_target_tests.rs"]
mod rectangle_radius_target_tests;

#[cfg(test)]
#[path = "polygon_sides_alert_tests.rs"]
mod polygon_sides_alert_tests;

#[cfg(test)]
#[path = "line_wh_fields_tests.rs"]
mod line_wh_fields_tests;

#[cfg(test)]
#[path = "star_points_field_tests.rs"]
mod star_points_field_tests;

#[cfg(test)]
#[path = "polygon_corner_radius_field_tests.rs"]
mod polygon_corner_radius_field_tests;
