//! Shape toolbar edits share the engine commands used by the Properties panel.

use photocraft_doc::{Fill, LayerContent, LayerId, ShapeLayer};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::shape_stroke_ui::StrokeOptions;
use crate::state::Tool;
use crate::theme::Tokens;

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
    if let Some((_, sh)) = target(app) {
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

enum Change {
    Fill(Option<Fill>),
    Stroke(Option<Fill>),
    Width(f32),
    Style(StrokeOptions),
}

/// Change an explicit toolbar target, then update defaults only after a successful edit.
fn apply(app: &mut PhotocraftApp, id: Option<LayerId>, change: Change, coalesce: Option<String>) -> Result<(), String> {
    let shape = id.and_then(|id| app.session.active()?.doc.layer(id)).and_then(|l| match &l.content {
        LayerContent::Shape(sh) => Some(sh),
        _ => None,
    });
    let default_width = app.ui.tool_options.stroke_width;
    let stroke_width = shape.and_then(|sh| sh.stroke.as_ref()).map_or(if default_width > 0.0 { default_width } else { 3.0 }, |s| s.width);
    let style_width = crate::vector_ui::enabled_stroke_width(if stroke_width > 0.0 { stroke_width } else { default_width });
    if let Some(id) = id {
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
        };
        if let Some(key) = coalesce {
            patch["coalesce"] = json!(format!("shape-toolbar-{:?}-{id:?}-{key}", app.session.active().map(|st| st.doc.id)));
        }
        patch["layer"] = json!(id.0);
        app.run("shape.edit", patch)?;
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

pub(crate) fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool) {
    let shown = target(app);
    let id = shown.as_ref().map(|(id, _)| *id);
    let sh = shown.as_ref().map(|(_, sh)| sh);
    let locked = id.is_some_and(|id| {
        app.session.active().is_some_and(|st| {
            let locks = st.doc.effective_locks(id);
            locks.all || locks.pixels
        })
    });
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
    // Geometry fields keep their previous behaviour: they set the next shape's default and edit
    // the selected live shape.
    let mut edit: Option<Value> = None;
    let doc_id = app.session.active().map(|st| st.doc.id);
    let key = |k: &str| doc_id.zip(id).map(|(doc, layer)| format!("shape-options-{doc:?}-{}-{k}", layer.0));
    ui.push_id(("shape-toolbar", doc_id, id), |ui| {
        ui.add_enabled_ui(!locked, |ui| {
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
            let corners = sh.map_or(tool != Tool::EllipseShape, |sh| !matches!(sh.live, Some(photocraft_doc::LiveShape::Ellipse { .. })));
            let stroke_key = id.map_or_else(|| "tool-stroke".to_string(), |_| format!("shape-toolbar-{doc_id:?}-{id:?}-stroke"));
            if let Some(next) = crate::shape_stroke_ui::button(ui, &stroke_key, &style, &mut app.ui.stroke_editor, closed, corners, doc_id.zip(id), width > 0.0)
            {
                stroke_coalesce = Some(next.coalesce);
                change = Some(Change::Style(next.options));
            }
            let o = &mut app.ui.tool_options;
            match tool {
                Tool::Rectangle => {
                    crate::widgets::vline(ui, 22.0);
                    label(ui, tl!("Radius:"));
                    let mut r = match sh.and_then(|s| s.live.as_ref()) {
                        Some(photocraft_doc::LiveShape::Rect { radii, .. }) => radii[0] as f32,
                        _ => o.corner_radius,
                    };
                    if crate::widgets::value_field(ui, &mut r, 0.0..=10000.0, "px", 62.0).changed() {
                        o.corner_radius = r;
                        let rf = r as f64;
                        edit = Some(json!({"radii": [rf, rf, rf, rf], "coalesce": key("radius")}));
                    }
                }
                Tool::Polygon => {
                    crate::widgets::vline(ui, 22.0);
                    label(ui, tl!("Sides:"));
                    let mut s = o.polygon_sides as f32;
                    if crate::widgets::value_field(ui, &mut s, 3.0..=100.0, "", 48.0).changed() {
                        o.polygon_sides = s.round().clamp(3.0, 100.0) as u32;
                    }
                }
                Tool::Line => {
                    crate::widgets::vline(ui, 22.0);
                    label(ui, tl!("Weight:"));
                    crate::widgets::value_field(ui, &mut o.line_weight, 1.0..=1000.0, "px", 58.0);
                }
                Tool::CustomShape => {
                    crate::widgets::vline(ui, 22.0);
                    crate::preset_panels::shape_picker(app, ui);
                }
                _ if matches!(tool, Tool::PathSelection | Tool::DirectSelection) => {
                    if let Some(photocraft_doc::LiveShape::Rect { radii, .. }) = sh.and_then(|s| s.live.as_ref()) {
                        crate::widgets::vline(ui, 22.0);
                        label(ui, tl!("Radius:"));
                        let mut r = radii[0] as f32;
                        if crate::widgets::value_field(ui, &mut r, 0.0..=10000.0, "px", 62.0).changed() {
                            o.corner_radius = r;
                            let rf = r as f64;
                            edit = Some(json!({"radii": [rf, rf, rf, rf], "coalesce": key("radius")}));
                        }
                    }
                }
                _ => {}
            }
        });
    });
    let coalesce = if discrete_paint { None } else { stroke_coalesce.or_else(|| edit_key(ui.ctx())) };
    if let Some(change) = change
        && let Err(error) = apply(app, id, change, coalesce)
    {
        app.ui.status = error;
        app.ui.status_error = true;
    }
    if let (Some(mut p), Some(layer)) = (edit, id) {
        p["layer"] = json!(layer.0);
        if let Err(e) = app.run("shape.edit", p) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
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
    fn edits_respect_locks_and_mask_targets() {
        let mut app = app(32);
        let id = rectangle(&mut app);
        let before = shape(&app, id).clone();
        let defaults = app.ui.tool_options.shape_fill_color;
        app.run("layer.setProps", json!({"layer": id.0, "locks": {"pixels": true}})).unwrap();
        assert!(apply(&mut app, Some(id), Change::Fill(Some(solid([1.0; 4]))), None).is_err());
        assert_eq!(shape(&app, id), &before);
        assert_eq!(app.ui.tool_options.shape_fill_color, defaults);
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
    fn drawing_the_toolbar_does_not_clamp_large_strokes() {
        let mut app = app(8);
        let id = LayerId(
            app.run("shape.create", json!({"kind": "rect", "rect": [10, 20, 20, 10], "stroke": {"width": 500, "color": "#114488"}})).unwrap()["layer"]
                .as_u64()
                .unwrap(),
        );
        let before = shape(&app, id).clone();
        let revision = app.session.active().unwrap().revision;
        let mut h = harness(app);
        h.run_steps(6);
        assert_eq!(shape(h.state(), id), &before);
        assert_eq!(h.state().session.active().unwrap().revision, revision);
        assert!(!h.state().ui.status_error);
    }

    #[test]
    fn old_tool_settings_keep_their_color_behavior() {
        let mut saved = serde_json::to_value(crate::state::ToolOptions::default()).unwrap();
        for key in ["shape_fill_color", "shape_stroke_color"] {
            saved.as_object_mut().unwrap().remove(key);
        }
        let old: crate::state::ToolOptions = serde_json::from_value(saved).unwrap();
        assert!(old.shape_fill_color.is_none() && old.shape_stroke_color.is_none());
    }
}
