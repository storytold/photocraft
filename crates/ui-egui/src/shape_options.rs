//! Shape toolbar edits share the engine commands used by the Properties panel.

use egui::Sense;
use photocraft_doc::{Fill, LayerContent, LayerId, LiveShape, ShapeLayer};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::shape_stroke_ui::StrokeOptions;
use crate::state::Tool;
use crate::theme::Tokens;

const MAX_SIZE: f32 = 300_000.0;

pub(crate) fn fill_color(app: &PhotocraftApp) -> [f32; 4] {
    app.ui.tool_options.shape_fill_color.unwrap_or(app.session.tools.foreground)
}

pub(crate) fn stroke_color(app: &PhotocraftApp) -> [f32; 4] {
    app.ui.tool_options.shape_stroke_color.unwrap_or(app.session.tools.background)
}

fn solid(color: [f32; 4]) -> Fill {
    Fill::Solid(photocraft_doc::Color::rgba(color[0], color[1], color[2], color[3]))
}

pub(crate) fn paint(app: &PhotocraftApp) -> (Option<Fill>, Option<photocraft_doc::ShapeStroke>) {
    if app.ui.tool_options.shape.mode == "shape"
        && let Some((_, sh)) = target(app)
    {
        return (sh.fill, sh.stroke);
    }
    let o = &app.ui.tool_options;
    (o.shape_fill.then(|| default_fill(app)), (o.stroke_width > 0.0).then(|| o.shape_stroke.stroke(o.stroke_width, default_stroke(app))))
}

fn default_fill(app: &PhotocraftApp) -> Fill {
    app.ui.tool_options.shape_fill_paint.clone().unwrap_or_else(|| solid(fill_color(app)))
}

fn default_stroke(app: &PhotocraftApp) -> Fill {
    app.ui.tool_options.shape_stroke_paint.clone().unwrap_or_else(|| solid(stroke_color(app)))
}

fn target(app: &PhotocraftApp) -> Option<(LayerId, ShapeLayer)> {
    if app.ui.mask_target || app.ui.vector_mask_target {
        return None;
    }
    let st = app.session.active()?;
    let id = st.active_layer?;
    match &st.doc.layer(id)?.content {
        LayerContent::Shape(sh) => Some((id, sh.clone())),
        _ => None,
    }
}

/// Geometry shown in the toolbar also seeds the next shape of the same tool.
pub(crate) fn drawing_live(app: &PhotocraftApp) -> Option<LiveShape> {
    match app.ui.tool_options.shape.mode.as_str() {
        "shape" if !app.ui.mask_target && !app.ui.vector_mask_target => {
            let st = app.session.active()?;
            match &st.doc.layer(st.active_layer?)?.content {
                LayerContent::Shape(sh) => sh.live.clone(),
                _ => None,
            }
        }
        "path" => crate::shape_modes::path_shape(app)?.live,
        _ => None,
    }
}

fn bounds(sh: &ShapeLayer) -> Option<[f64; 4]> {
    match &sh.live {
        Some(LiveShape::Rect { rect, .. } | LiveShape::Ellipse { rect } | LiveShape::Polygon { rect, .. }) => Some(*rect),
        Some(LiveShape::Line { from, to, .. }) => Some([from[0].min(to[0]), from[1].min(to[1]), (to[0] - from[0]).abs(), (to[1] - from[1]).abs()]),
        _ => sh.path.control_bounds().map(|(x0, y0, x1, y1)| [x0, y0, x1 - x0, y1 - y0]),
    }
    .filter(|r| r.iter().all(|v| v.is_finite()))
}

enum Change {
    Fill(Option<Fill>),
    Stroke(Option<Fill>),
    Width(f32),
    Style(StrokeOptions),
    Size { height: bool, value: f32, linked: bool },
    Radius(f32),
    Sides(u32),
    StarRatio(f64),
    SmoothIndents(bool),
    Weight(f32),
}

fn resize(sh: &ShapeLayer, height: bool, value: f32, linked: bool) -> Result<Value, String> {
    if !value.is_finite() || !(1.0..=MAX_SIZE).contains(&value) {
        return Err("Shape dimensions must be between 1 and 300000 pixels".into());
    }
    let [x, y, w, h] = bounds(sh).ok_or("The shape has no finite bounds")?;
    let old = if height { h } else { w };
    if old <= 0.0 || (linked && (w <= 0.0 || h <= 0.0)) {
        return Err("Cannot resize a zero-length shape dimension".into());
    }
    let scale = f64::from(value) / old;
    // Keep the entered dimension exact; derive only its linked companion.
    let value = f64::from(value);
    let (nw, nh) = if height { (if linked { w * value / h } else { w }, value) } else { (value, if linked { h * value / w } else { h }) };
    if !scale.is_finite() || !nw.is_finite() || !nh.is_finite() {
        return Err("The resized shape dimensions are not finite".into());
    }
    if linked && (!(1.0..=f64::from(MAX_SIZE)).contains(&nw) || !(1.0..=f64::from(MAX_SIZE)).contains(&nh)) {
        return Err("Linked shape dimensions must be between 1 and 300000 pixels".into());
    }
    Ok(match sh.live {
        Some(LiveShape::Rect { .. } | LiveShape::Ellipse { .. } | LiveShape::Polygon { .. }) => json!({"rect": [x, y, nw, nh]}),
        Some(LiveShape::Line { from, to, .. }) => {
            let point =
                |p: [f64; 2]| [if !height || linked { x + (p[0] - x) * scale } else { p[0] }, if height || linked { y + (p[1] - y) * scale } else { p[1] }];
            json!({"from": point(from), "to": point(to)})
        }
        // Custom paths keep their path geometry instead of becoming rectangles.
        _ => {
            let (sx, sy) = (if !height || linked { scale } else { 1.0 }, if height || linked { scale } else { 1.0 });
            let transform = [sx, 0.0, 0.0, sy, x * (1.0 - sx), y * (1.0 - sy)];
            if transform.iter().any(|v| !v.is_finite()) {
                return Err("The resized shape transform is not finite".into());
            }
            json!({"transform": transform})
        }
    })
}

/// Change an explicit toolbar target, then update defaults only after a successful edit.
fn apply(app: &mut PhotocraftApp, id: Option<LayerId>, change: Change, coalesce: Option<String>) -> Result<(), String> {
    if let Some(id) = id
        && matches!(change, Change::Size { .. })
        && app.session.active().is_some_and(|st| st.doc.effective_locks(id).position)
    {
        return Err("Shape position is locked".into());
    }
    let path_shape = (app.ui.tool_options.shape.mode == "path").then(|| crate::shape_modes::path_shape(app)).flatten();
    let shape = id
        .and_then(|id| app.session.active()?.doc.layer(id))
        .and_then(|l| match &l.content {
            LayerContent::Shape(sh) => Some(sh),
            _ => None,
        })
        .or(path_shape.as_ref());
    let default_width = app.ui.tool_options.stroke_width;
    let stroke_width = shape.and_then(|sh| sh.stroke.as_ref()).map_or(if default_width > 0.0 { default_width } else { 3.0 }, |s| s.width);
    let style_width = crate::vector_ui::enabled_stroke_width(if stroke_width > 0.0 { stroke_width } else { default_width });
    if id.is_some() || path_shape.is_some() {
        let sh = shape.ok_or("The toolbar target is no longer a shape layer")?;
        let mut patch = match &change {
            Change::Fill(fill) => json!({"fill": photocraft_engine::vector_cmds::shape_paint_params(fill.as_ref(), None)["fill"].clone()}),
            Change::Stroke(None) => json!({"stroke": null}),
            Change::Stroke(Some(fill)) => {
                let mut stroke = if sh.stroke.is_none() { app.ui.tool_options.shape_stroke.params() } else { json!({}) };
                stroke["fill"] = photocraft_engine::vector_cmds::shape_paint_params(Some(fill), None)["fill"].clone();
                stroke["width"] = json!(crate::vector_ui::enabled_stroke_width(stroke_width));
                json!({"stroke": stroke})
            }
            Change::Width(w) if *w <= 0.0 => json!({"stroke": null}),
            Change::Width(w) => {
                let mut stroke = json!({"width": w});
                if sh.stroke.is_none() {
                    stroke["fill"] = photocraft_engine::vector_cmds::shape_paint_params(Some(&default_stroke(app)), None)["fill"].clone();
                    // A new stroke also takes the current tool's cap, join and dash settings.
                    if let Some(style) = app.ui.tool_options.shape_stroke.params().as_object() {
                        for (key, value) in style {
                            stroke[key] = value.clone();
                        }
                    }
                }
                json!({"stroke": stroke})
            }
            Change::Style(style) => {
                let mut stroke = style.params();
                if sh.stroke.as_ref().is_none_or(|stroke| stroke.width <= 0.0) {
                    stroke["width"] = json!(style_width);
                }
                if sh.stroke.is_none() {
                    stroke["fill"] = photocraft_engine::vector_cmds::shape_paint_params(Some(&default_stroke(app)), None)["fill"].clone();
                }
                json!({"stroke": stroke})
            }
            Change::Size { height, value, linked } => resize(sh, *height, *value, *linked)?,
            Change::Radius(r) => {
                if matches!(sh.live, Some(LiveShape::Polygon { .. })) {
                    json!({"cornerRadius": r})
                } else {
                    json!({"radii": r})
                }
            }
            Change::Sides(n) => json!({"sides": n}),
            Change::StarRatio(r) => json!({"starRatio": r}),
            Change::SmoothIndents(smooth) => json!({"smoothIndents": smooth}),
            Change::Weight(w) => json!({"weight": w}),
        };
        if let Some(key) = coalesce {
            patch["coalesce"] = json!(format!("shape-toolbar-{:?}-{id:?}-{key}", app.session.active().map(|st| st.doc.id)));
        }
        if let Some(id) = id {
            patch["layer"] = json!(id.0);
            app.run("shape.edit", patch)?;
        } else {
            crate::shape_modes::edit_path(app, patch)?;
        }
    }
    let o = &mut app.ui.tool_options;
    match change {
        Change::Fill(fill) => {
            o.shape_fill = fill.is_some();
            if let Some(Fill::Solid(color)) = &fill {
                let [r, g, b] = color.to_rgb();
                o.shape_fill_color = Some([r, g, b, color.alpha]);
            }
            if fill.is_some() {
                o.shape_fill_paint = fill;
            }
        }
        Change::Stroke(fill) => {
            o.stroke_width = if fill.is_some() { crate::vector_ui::enabled_stroke_width(stroke_width) } else { 0.0 };
            if let Some(Fill::Solid(color)) = &fill {
                let [r, g, b] = color.to_rgb();
                o.shape_stroke_color = Some([r, g, b, color.alpha]);
            }
            if fill.is_some() {
                o.shape_stroke_paint = fill;
            }
        }
        Change::Width(w) => o.stroke_width = w,
        Change::Style(style) => {
            o.shape_stroke = style;
            o.stroke_width = style_width;
        }
        Change::Radius(r) => o.corner_radius = r,
        Change::Sides(n) => o.polygon_sides = n,
        Change::StarRatio(r) => o.shape.star_ratio = r,
        Change::SmoothIndents(smooth) => o.shape.smooth_indents = smooth,
        Change::Weight(w) => o.line_weight = w,
        Change::Size { .. } => {}
    }
    Ok(())
}

/// Every pointer drag and focus session gets its own history group.
pub(crate) fn edit_key(ctx: &egui::Context) -> Option<String> {
    let focused = ctx.memory(|m| m.focused());
    let focus_key = egui::Id::new("shape-toolbar-focus");
    let (previous, started) = ctx.data(|d| d.get_temp::<(Option<egui::Id>, u64)>(focus_key)).unwrap_or_default();
    let started = if previous == focused { started } else { ctx.cumulative_pass_nr() };
    ctx.data_mut(|d| d.insert_temp(focus_key, (focused, started)));
    if let Some(id) = ctx.dragged_id()
        && let Some(time) = ctx.input(|i| i.pointer.press_start_time())
    {
        return Some(format!("drag-{}-{}", id.value(), time.to_bits()));
    }
    focused.map(|id| format!("focus-{}-{started}", id.value()))
}

fn dimension(ui: &mut egui::Ui, height: bool, value: &mut f32) -> bool {
    let t = Tokens::get(ui.ctx());
    let label = if height { tl!("H") } else { tl!("W") };
    let response = ui
        .add(egui::Label::new(egui::RichText::new(format!("{label}:")).color(t.text_dim).size(12.0)).sense(Sense::drag()))
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
        .on_hover_text(if height { tl!("Height") } else { tl!("Width") });
    let start_key = response.id.with("scrub-start");
    if response.drag_started() {
        ui.data_mut(|d| d.insert_temp(start_key, *value));
    }
    let mut changed = false;
    if response.dragged()
        && let (Some(start), Some(origin), Some(pos)) =
            (ui.data(|d| d.get_temp::<f32>(start_key)), ui.input(|i| i.pointer.press_origin()), response.interact_pointer_pos())
    {
        let next = (start + (pos.x - origin.x) * 0.5).clamp(1.0, MAX_SIZE);
        changed = next != *value;
        *value = next;
    }
    // Disabled zero-length axes and imported subpixel sizes must not clamp on every draw.
    let field = crate::widgets::value_field(ui, value, 0.0..=MAX_SIZE.max(*value), "px", 62.0);
    ui.is_enabled() && (field.changed() || changed)
}

pub(crate) fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool) {
    let mode = app.ui.tool_options.shape.mode.clone();
    let shown = if mode == "shape" { target(app) } else { None };
    let path_shape = if mode == "path" { crate::shape_modes::path_shape(app) } else { None };
    let id = shown.as_ref().map(|(id, _)| *id);
    let sh = shown.as_ref().map(|(_, sh)| sh).or(path_shape.as_ref());
    let locked = id.is_some_and(|id| {
        app.session.active().is_some_and(|st| {
            let locks = st.doc.effective_locks(id);
            locks.all || locks.pixels
        })
    });
    let position_locked = id.is_some_and(|id| app.session.active().is_some_and(|st| st.doc.effective_locks(id).position));
    let fill = sh.map_or_else(|| app.ui.tool_options.shape_fill.then(|| default_fill(app)), |sh| sh.fill.clone());
    let stroke = sh.and_then(|sh| sh.stroke.as_ref());
    let paint = if sh.is_some() { stroke.map(|s| s.paint.clone()) } else { (app.ui.tool_options.stroke_width > 0.0).then(|| default_stroke(app)) };
    let mut width = if sh.is_some() { stroke.map_or(0.0, |s| s.width) } else { app.ui.tool_options.stroke_width };
    let style = stroke.map_or_else(|| app.ui.tool_options.shape_stroke.clone(), StrokeOptions::from);
    let t = Tokens::get(ui.ctx());
    let label = |ui: &mut egui::Ui, text: &str| {
        ui.label(egui::RichText::new(text).color(t.text_dim).size(12.0));
    };
    let mut change = None;
    let mut discrete_paint = false;
    let mut stroke_coalesce = None;
    let doc_id = app.session.active().map(|st| st.doc.id);
    let path_name = (mode == "path").then(|| app.ui.selected_path.as_deref().filter(|n| *n != "layer").unwrap_or("work").to_string());
    ui.push_id(("shape-toolbar", doc_id, id, path_name), |ui| {
        ui.add_enabled_ui(!locked, |ui| {
            if mode == "shape" {
                label(ui, tl!("Fill:"));
                if let Some(c) = crate::shape_paint_ui::swatch(app, ui, fill.as_ref(), fill_color(app), tl!("Fill color")) {
                    discrete_paint = c.discrete;
                    change = Some(Change::Fill(c.paint));
                }
                label(ui, tl!("Stroke:"));
                if let Some(c) = crate::shape_paint_ui::swatch(app, ui, paint.as_ref(), stroke_color(app), tl!("Stroke color")) {
                    discrete_paint = c.discrete;
                    change = Some(Change::Stroke(c.paint));
                }
                let max_width = 288.0_f32.max(width);
                if crate::widgets::value_field(ui, &mut width, 0.0..=max_width, "px", 58.0).changed() && ui.is_enabled() {
                    change = Some(Change::Width(width));
                }
                let closed = sh.is_none_or(|sh| sh.path.subpaths.iter().all(|s| s.closed));
                let corners = sh.map_or(tool != Tool::EllipseShape, |sh| !matches!(sh.live, Some(LiveShape::Ellipse { .. })));
                let key = id.map_or_else(|| "tool-stroke".to_string(), |_| format!("shape-toolbar-{doc_id:?}-{id:?}-stroke"));
                if let Some(next) = crate::shape_stroke_ui::button(ui, &key, &style, &mut app.ui.stroke_editor, closed, corners, doc_id.zip(id), width > 0.0) {
                    stroke_coalesce = Some(next.coalesce);
                    change = Some(Change::Style(next.options));
                }
                crate::widgets::vline(ui, 22.0);
            }
            if mode != "pixels" {
                let rect = sh.and_then(bounds);
                let mut linked = app.ui.tool_options.shape_link_dimensions;
                ui.add_enabled_ui(rect.is_some_and(|r| r[2] > 0.0 && r[3] > 0.0), |ui| {
                    if crate::icons::button(ui, "link", 24.0, linked, tl!("Link width and height")).clicked() {
                        linked = !linked;
                    }
                });
                app.ui.tool_options.shape_link_dimensions = linked;
                for height in [false, true] {
                    let mut value = rect.map_or(0.0, |r| if height { r[3] } else { r[2] }) as f32;
                    ui.add_enabled_ui(!position_locked && value.is_finite() && value > 0.0, |ui| {
                        if dimension(ui, height, &mut value) {
                            change = Some(Change::Size { height, value, linked });
                        }
                    });
                }
            }
            match tool {
                Tool::Rectangle | Tool::Triangle | Tool::Polygon => {
                    let live_radius = sh.and_then(|sh| match sh.live {
                        Some(LiveShape::Rect { radii, .. }) if tool == Tool::Rectangle => Some(radii[0] as f32),
                        Some(LiveShape::Polygon { corner_radius, .. }) if tool != Tool::Rectangle => Some(corner_radius as f32),
                        _ => None,
                    });
                    ui.add_enabled_ui(sh.is_none() || live_radius.is_some(), |ui| {
                        label(ui, tl!("Radius:"));
                        let mut radius = live_radius.unwrap_or(app.ui.tool_options.corner_radius);
                        let max_radius = 10000.0_f32.max(radius);
                        if crate::widgets::value_field(ui, &mut radius, 0.0..=max_radius, "px", 62.0).changed() && ui.is_enabled() {
                            change = Some(Change::Radius(radius));
                        }
                    });
                }
                _ => {}
            }
            match tool {
                Tool::Polygon => {
                    let live_sides = sh.and_then(|sh| match sh.live {
                        Some(LiveShape::Polygon { sides, .. }) => Some(sides),
                        _ => None,
                    });
                    ui.add_enabled_ui(sh.is_none() || live_sides.is_some(), |ui| {
                        label(ui, tl!("Sides:"));
                        let mut sides = live_sides.unwrap_or(app.ui.tool_options.polygon_sides) as f32;
                        if crate::widgets::value_field(ui, &mut sides, 3.0..=100.0, "", 48.0).changed() && ui.is_enabled() {
                            change = Some(Change::Sides(sides.round().clamp(3.0, 100.0) as u32));
                        }
                        let (ratio, smooth) = sh
                            .and_then(|sh| match sh.live {
                                Some(LiveShape::Polygon { star_ratio, smooth_indents, .. }) => Some((star_ratio, smooth_indents)),
                                _ => None,
                            })
                            .unwrap_or((app.ui.tool_options.shape.star_ratio, app.ui.tool_options.shape.smooth_indents));
                        let mut percent = (ratio * 100.0) as f32;
                        label(ui, tl!("Star Ratio:"));
                        if crate::widgets::value_field(ui, &mut percent, 1.0..=100.0, "%", 58.0).changed() && ui.is_enabled() {
                            change = Some(Change::StarRatio(f64::from(percent) / 100.0));
                        }
                        let mut smooth = smooth;
                        if ui.checkbox(&mut smooth, tl!("Smooth Indents")).changed() {
                            change = Some(Change::SmoothIndents(smooth));
                        }
                    });
                }
                Tool::Line => {
                    let live_weight = sh.and_then(|sh| match sh.live {
                        Some(LiveShape::Line { weight, .. }) => Some(weight as f32),
                        _ => None,
                    });
                    ui.add_enabled_ui(sh.is_none() || live_weight.is_some(), |ui| {
                        label(ui, tl!("Weight:"));
                        let mut weight = live_weight.unwrap_or(app.ui.tool_options.line_weight);
                        let (min_weight, max_weight) = (1.0_f32.min(weight), 1000.0_f32.max(weight));
                        if crate::widgets::value_field(ui, &mut weight, min_weight..=max_weight, "px", 58.0).changed() && ui.is_enabled() {
                            change = Some(Change::Weight(weight));
                        }
                    });
                }
                Tool::CustomShape => crate::preset_panels::shape_picker(app, ui),
                _ => {}
            }
        });
    });
    let key = if discrete_paint { None } else { stroke_coalesce.or_else(|| edit_key(ui.ctx())) };
    if let Some(change) = change
        && let Err(error) = apply(app, id, change, key)
    {
        app.ui.status = error;
        app.ui.status_error = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    fn app(depth: u32) -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200, "depth": depth})).unwrap();
        app.run("tools.setColors", json!({"foreground": "#ff0000", "background": "#0000ff"})).unwrap();
        app.ui.tool = Tool::Rectangle;
        app
    }

    fn rectangle(app: &mut PhotocraftApp) -> LayerId {
        LayerId(app.run("shape.create", json!({"kind": "rect", "rect": [10, 20, 100, 50], "fill": "#aa2233"})).unwrap()["layer"].as_u64().unwrap())
    }

    fn shape(app: &PhotocraftApp, id: LayerId) -> &ShapeLayer {
        match &app.session.active().unwrap().doc.layer(id).unwrap().content {
            LayerContent::Shape(sh) => sh,
            _ => panic!("expected a shape layer"),
        }
    }

    #[test]
    fn colors_edit_the_active_shape_and_new_defaults_without_changing_tool_colors() {
        for depth in [8, 16, 32] {
            let mut app = app(depth);
            let id = rectangle(&mut app);
            let original = shape(&app, id).fill.clone();
            let colors = (app.session.tools.foreground, app.session.tools.background);
            apply(&mut app, Some(id), Change::Fill(Some(solid([34.0 / 255.0, 102.0 / 255.0, 170.0 / 255.0, 1.0]))), None).unwrap();
            let fill = shape(&app, id).fill.clone();
            assert_ne!(fill, original);
            assert!(shape(&app, id).stroke.is_none());
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(shape(&app, id).fill, original);
            app.run("edit.redo", json!({})).unwrap();
            assert_eq!(shape(&app, id).fill, fill);
            apply(&mut app, Some(id), Change::Stroke(Some(solid([68.0 / 255.0, 170.0 / 255.0, 102.0 / 255.0, 1.0]))), None).unwrap();
            let stroke = shape(&app, id).stroke.clone();
            assert_eq!(shape(&app, id).fill, fill);
            assert_eq!(stroke.as_ref().unwrap().width, 3.0);
            let created = crate::vector_ui::create_shape(&mut app, json!({"kind": "ellipse", "rect": [120, 20, 70, 50]})).unwrap();
            let next = shape(&app, LayerId(created["layer"].as_u64().unwrap()));
            assert_eq!(next.fill, fill);
            assert_eq!(next.stroke, stroke);
            assert_eq!((app.session.tools.foreground, app.session.tools.background), colors);
        }
    }

    #[test]
    fn dimensions_keep_live_geometry_and_each_drag_has_one_undo_step() {
        let mut app = app(16);
        let id = rectangle(&mut app);
        app.run("shape.edit", json!({"layer": id.0, "radii": [1, 2, 3, 4]})).unwrap();
        for value in [110.0, 120.0] {
            apply(&mut app, Some(id), Change::Size { height: false, value, linked: true }, Some("drag-a".into())).unwrap();
        }
        assert_eq!(bounds(shape(&app, id)), Some([10.0, 20.0, 120.0, 60.0]));
        let Some(LiveShape::Rect { radii, .. }) = shape(&app, id).live else { panic!("live rectangle") };
        assert_eq!(radii, [1.0, 2.0, 3.0, 4.0]);
        apply(&mut app, Some(id), Change::Size { height: true, value: 80.0, linked: true }, Some("drag-b".into())).unwrap();
        assert_eq!(bounds(shape(&app, id)), Some([10.0, 20.0, 160.0, 80.0]));
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(bounds(shape(&app, id)), Some([10.0, 20.0, 120.0, 60.0]));
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(bounds(shape(&app, id)), Some([10.0, 20.0, 100.0, 50.0]));
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(bounds(shape(&app, id)), Some([10.0, 20.0, 120.0, 60.0]));
    }

    #[test]
    fn custom_paths_and_lines_resize_without_replacing_their_geometry() {
        let mut app = app(8);
        let id = rectangle(&mut app);
        let path = photocraft_engine::vector_cmds::path_json(&shape(&app, id).path);
        app.run("shape.edit", json!({"layer": id.0, "path": path})).unwrap();
        assert!(shape(&app, id).live.is_none());
        apply(&mut app, Some(id), Change::Size { height: false, value: 200.0, linked: false }, None).unwrap();
        assert_eq!(bounds(shape(&app, id)), Some([10.0, 20.0, 200.0, 50.0]));
        assert!(shape(&app, id).live.is_none());
        let id = LayerId(app.run("shape.create", json!({"kind": "line", "from": [10, 20], "to": [110, 20], "weight": 4})).unwrap()["layer"].as_u64().unwrap());
        apply(&mut app, Some(id), Change::Size { height: false, value: 150.0, linked: false }, None).unwrap();
        assert_eq!(shape(&app, id).live, Some(LiveShape::Line { from: [10.0, 20.0], to: [160.0, 20.0], weight: 4.0, arrowheads: None }));
        assert!(apply(&mut app, Some(id), Change::Size { height: true, value: 50.0, linked: false }, None).is_err());
    }

    #[test]
    fn edits_respect_locks_mask_targets_and_invalid_dimensions() {
        let mut app = app(32);
        let id = rectangle(&mut app);
        let before = shape(&app, id).clone();
        let defaults = app.ui.tool_options.shape_fill_color;
        app.run("layer.setProps", json!({"layer": id.0, "locks": {"pixels": true}})).unwrap();
        assert!(apply(&mut app, Some(id), Change::Fill(Some(solid([1.0; 4]))), None).is_err());
        assert_eq!(shape(&app, id), &before);
        assert_eq!(app.ui.tool_options.shape_fill_color, defaults);
        app.run("layer.setProps", json!({"layer": id.0, "locks": {"pixels": false, "position": true}})).unwrap();
        assert!(apply(&mut app, Some(id), Change::Size { height: false, value: 120.0, linked: false }, None).is_err());
        app.run("layer.setProps", json!({"layer": id.0, "locks": {"position": false}})).unwrap();
        for value in [0.0, -1.0, f32::NAN, f32::INFINITY, 300001.0] {
            assert!(apply(&mut app, Some(id), Change::Size { height: false, value, linked: true }, None).is_err());
            assert_eq!(shape(&app, id), &before);
        }
        app.ui.mask_target = true;
        assert!(target(&app).is_none());
        app.ui.mask_target = false;
        app.ui.vector_mask_target = true;
        assert!(target(&app).is_none());
    }

    #[test]
    fn stroke_width_and_style_preserve_paint_and_defaults_enable_a_new_stroke() {
        let mut app = app(8);
        let id = rectangle(&mut app);
        app.ui.tool_options.shape_stroke.dashes = vec![3.0, 2.0];
        apply(&mut app, Some(id), Change::Width(5.0), None).unwrap();
        let stroke = shape(&app, id).stroke.clone().unwrap();
        assert_eq!(stroke.paint, solid(app.session.tools.background));
        assert_eq!(stroke.dashes, [3.0, 2.0]);
        let mut style = StrokeOptions::from(&stroke);
        style.dashes = vec![1.0, 4.0];
        apply(&mut app, Some(id), Change::Style(style), None).unwrap();
        assert_eq!(shape(&app, id).stroke.as_ref().unwrap().paint, stroke.paint);
        assert_eq!(shape(&app, id).stroke.as_ref().unwrap().width, stroke.width);
    }

    #[test]
    fn deliberate_stroke_style_enables_zero_width_and_preserves_paint_with_undo() {
        for (default_width, expected_width) in [(0.0, 3.0), (10.0, 10.0)] {
            let mut app = app(8);
            let raster = app.session.active().unwrap().active_layer.unwrap();
            let id = rectangle(&mut app);
            app.run("shape.edit", json!({"layer": id.0, "stroke": {"color": [0.125, 0.375, 0.625, 0.5], "width": 0}})).unwrap();
            let original = shape(&app, id).clone();
            let mut style = StrokeOptions::from(original.stroke.as_ref().unwrap());
            style.dashes = vec![1.0, 3.0];
            app.ui.tool_options.stroke_width = default_width;
            let history = app.session.active().unwrap().history.past_len();
            apply(&mut app, Some(id), Change::Style(style), None).unwrap();
            let stroke = shape(&app, id).stroke.as_ref().unwrap();
            assert_eq!(stroke.width, expected_width);
            assert_eq!(stroke.paint, original.stroke.as_ref().unwrap().paint);
            assert_eq!(stroke.dashes, [1.0, 3.0]);
            assert_eq!(app.ui.tool_options.stroke_width, expected_width);
            assert_eq!(app.session.active().unwrap().history.past_len(), history + 1);
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(shape(&app, id), &original);
            app.run("edit.redo", json!({})).unwrap();
            assert_eq!(shape(&app, id).stroke.as_ref().unwrap().width, expected_width);
            app.run("layer.select", json!({"layer": raster.0})).unwrap();
            let next = crate::vector_ui::create_shape(&mut app, json!({"kind": "ellipse", "rect": [120, 20, 70, 50]})).unwrap();
            let stroke = shape(&app, LayerId(next["layer"].as_u64().unwrap())).stroke.as_ref().unwrap();
            assert_eq!(stroke.width, expected_width, "a new shape inherits the activated width after changing selection");
            assert_eq!(stroke.dashes, [1.0, 3.0]);
        }
    }

    fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 600.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    crate::i18n::with_language(crate::i18n::lang_from_tag("en").unwrap(), || {
                        ui.horizontal(|ui| crate::vector_ui::options_bar(app, ui, app.ui.tool));
                    });
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        h.run_steps(4);
        h
    }

    #[test]
    fn toolbar_fill_and_stroke_buttons_edit_the_selected_shape() {
        let mut app = app(8);
        let id = rectangle(&mut app);
        app.run("shape.edit", json!({"layer": id.0, "stroke": {"color": "#114488", "width": 4}})).unwrap();
        let original = shape(&app, id).clone();
        let mut h = harness(app);
        h.get_by_label("Fill color").click();
        h.run_steps(3);
        h.get_by_label("No Color").click();
        h.run_steps(3);
        assert!(shape(h.state(), id).fill.is_none());
        assert_eq!(shape(h.state(), id).stroke, original.stroke);
        h.state_mut().run("edit.undo", json!({})).unwrap();
        h.run_steps(3);
        h.get_by_label("Stroke color").click();
        h.run_steps(3);
        h.get_by_label("No Color").click();
        h.run_steps(3);
        assert!(shape(h.state(), id).stroke.is_none());
        assert_eq!(shape(h.state(), id).fill, original.fill);
    }

    #[test]
    fn dragging_width_label_updates_the_shape_and_undoes_in_one_step() {
        let mut app = app(8);
        let id = rectangle(&mut app);
        app.ui.tool_options.shape_link_dimensions = true;
        let mut h = harness(app);
        let label = h.query_all(egui_kittest::kittest::by().label("W:").include_labels()).next().unwrap().rect().center();
        h.hover_at(label);
        h.step();
        h.event(egui::Event::PointerButton { pos: label, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.step();
        for x in [10.0, 20.0, 40.0] {
            h.event(egui::Event::PointerMoved(label + egui::vec2(x, 0.0)));
            h.step();
        }
        h.event(egui::Event::PointerButton {
            pos: label + egui::vec2(40.0, 0.0),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_steps(3);
        assert_eq!(bounds(shape(h.state(), id)), Some([10.0, 20.0, 120.0, 60.0]));
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(bounds(shape(h.state(), id)), Some([10.0, 20.0, 100.0, 50.0]));
    }

    #[test]
    fn drawing_the_toolbar_does_not_clamp_large_strokes_or_zero_length_line_axes() {
        for params in [
            json!({"kind": "rect", "rect": [10, 20, 20, 10], "stroke": {"width": 500, "color": "#114488"}}),
            json!({"kind": "line", "from": [10, 20], "to": [110, 20], "weight": 4}),
        ] {
            let mut app = app(8);
            let is_line = params["kind"] == "line";
            let id = LayerId(app.run("shape.create", params).unwrap()["layer"].as_u64().unwrap());
            if is_line {
                app.ui.tool = Tool::Line;
            }
            let before = shape(&app, id).clone();
            let revision = app.session.active().unwrap().revision;
            let mut h = harness(app);
            h.run_steps(6);
            assert_eq!(shape(h.state(), id), &before);
            assert_eq!(h.state().session.active().unwrap().revision, revision);
            assert!(!h.state().ui.status_error);
        }
    }

    #[test]
    fn old_tool_settings_keep_their_color_behavior() {
        let mut saved = serde_json::to_value(crate::state::ToolOptions::default()).unwrap();
        for key in ["shape_fill_color", "shape_stroke_color", "shape_link_dimensions"] {
            saved.as_object_mut().unwrap().remove(key);
        }
        let old: crate::state::ToolOptions = serde_json::from_value(saved).unwrap();
        assert!(old.shape_fill_color.is_none() && old.shape_stroke_color.is_none());
        assert!(!old.shape_link_dimensions);
    }

    #[test]
    fn star_toolbar_edits_live_geometry_without_changing_paint_and_undoes() {
        let mut a = app(16);
        a.ui.tool = Tool::Polygon;
        let id = LayerId(
            a.run("shape.create", json!({"kind":"polygon","rect":[20,20,100,100],"sides":5,"starRatio":0.5,"fill":"#aa2233"})).unwrap()["layer"]
                .as_u64()
                .unwrap(),
        );
        let original = shape(&a, id).clone();
        apply(&mut a, Some(id), Change::StarRatio(0.4), None).unwrap();
        assert!(matches!(shape(&a, id).live, Some(LiveShape::Polygon { star_ratio: 0.4, .. })));
        let sharp = shape(&a, id).path.clone();
        let mut h = harness(a);
        h.get_by_label("Smooth Indents").click();
        h.run_steps(3);
        assert!(matches!(shape(h.state(), id).live, Some(LiveShape::Polygon { smooth_indents: true, .. })));
        assert_ne!(shape(h.state(), id).path, sharp);
        assert_eq!(shape(h.state(), id).fill, original.fill);
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(shape(h.state(), id).path, sharp);
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(shape(h.state(), id), &original);
    }
}
