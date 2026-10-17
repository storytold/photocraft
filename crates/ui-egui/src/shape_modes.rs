//! Shape-tool modes, path actions and view-only path display options.
use crate::{PhotocraftApp, state::Tool};
use photocraft_doc::{LayerContent, Path, ShapeLayer};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ShapeToolOptions {
    pub mode: String,
    pub blend: photocraft_color::BlendMode,
    pub opacity: f32,
    pub anti_alias: bool,
    pub path_width: f32,
    pub path_color: Option<[u8; 3]>,
    pub show_path: bool,
    pub from_center: bool,
    pub constrain: bool,
    pub size_mode: String,
    pub size_width: f64,
    pub size_height: f64,
    pub align_edges: bool,
    #[serde(skip)]
    pub path_geometry: Option<(u64, Value, Path)>,
}
impl Default for ShapeToolOptions {
    fn default() -> Self {
        Self {
            mode: "shape".into(),
            blend: photocraft_color::BlendMode::Normal,
            opacity: 100.0,
            anti_alias: true,
            path_width: 1.0,
            path_color: None,
            show_path: true,
            from_center: false,
            constrain: false,
            size_mode: "unconstrained".into(),
            size_width: 100.0,
            size_height: 100.0,
            align_edges: true,
            path_geometry: None,
        }
    }
}
impl ShapeToolOptions {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if !matches!(self.mode.as_str(), "shape" | "path" | "pixels") {
            return Err("shapeOptions.mode must be shape, path or pixels".into());
        }
        if !matches!(self.size_mode.as_str(), "unconstrained" | "square" | "fixed" | "proportional") {
            return Err("Invalid shapeOptions.sizeMode".into());
        }
        if !(0.0..=100.0).contains(&self.opacity)
            || !(0.5..=10.0).contains(&self.path_width)
            || ![self.size_width, self.size_height].iter().all(|v| (1.0..=300000.0).contains(v))
        {
            return Err("Shape options contain invalid numeric values".into());
        }
        Ok(())
    }
}

pub(crate) fn path_shape(app: &PhotocraftApp) -> Option<ShapeLayer> {
    let st = app.session.active()?;
    let name = app.ui.selected_path.as_deref().filter(|n| *n != "layer").unwrap_or("work");
    let path = if name == "work" { st.doc.work_path.as_ref()? } else { &st.doc.paths.iter().find(|p| p.name == name)?.path };
    let live = app.ui.tool_options.shape.path_geometry.as_ref().and_then(|(doc, geometry, original)| {
        if *doc == st.doc.id.0 && name == "work" && original == path { photocraft_engine::vector_cmds::shape_live(geometry).ok() } else { None }
    });
    Some(ShapeLayer { path: path.clone(), live, ..Default::default() })
}

pub(crate) fn edit_path(app: &mut PhotocraftApp, patch: Value) -> Result<(), String> {
    let shape = path_shape(app).ok_or("No path to edit")?;
    let name = app.ui.selected_path.as_deref().filter(|n| *n != "layer").unwrap_or("work").to_string();
    let mut geometry = app
        .ui
        .tool_options
        .shape
        .path_geometry
        .as_ref()
        .filter(|(doc, _, original)| shape.live.is_some() && original == &shape.path && app.session.active().is_some_and(|st| st.doc.id.0 == *doc))
        .map(|(_, p, _)| p.clone());
    let path = if patch.get("transform").is_some() {
        let m: [f64; 6] = serde_json::from_value(patch["transform"].clone()).map_err(|e| e.to_string())?;
        geometry = None;
        shape.path.transform(&photocraft_geom::Affine { m })
    } else {
        let p = geometry.as_mut().ok_or("This path has no live shape parameters")?;
        if let Some(patch) = patch.as_object() {
            for (k, v) in patch {
                p[k] = v.clone();
            }
        }
        photocraft_engine::vector_cmds::shape_path(p).map_err(|e| e.to_string())?
    };
    photocraft_vector::flatten::validate_shape(&path, None).map_err(String::from)?;
    let mut params = json!({"name": name, "path": photocraft_engine::vector_cmds::path_json(&path)});
    if let Some(key) = patch.get("coalesce") {
        params["coalesce"] = key.clone();
    }
    app.run("path.set", params)?;
    app.ui.tool_options.shape.path_geometry = geometry.and_then(|p| app.session.active().map(|st| (st.doc.id.0, p, path)));
    Ok(())
}

pub(crate) fn create(app: &mut PhotocraftApp, mut geometry: Value) -> Result<Value, String> {
    app.ui.tool_options.shape.validate()?;
    match app.ui.tool_options.shape.mode.as_str() {
        "path" => {
            let path = if geometry["kind"] == "path" {
                photocraft_engine::vector_cmds::parse_path(&geometry["path"]).map_err(|e| e.to_string())?
            } else {
                photocraft_engine::vector_cmds::shape_path(&geometry).map_err(|e| e.to_string())?
            };
            photocraft_vector::flatten::validate_shape(&path, None).map_err(String::from)?;
            let params = json!({"name": "work", "path": photocraft_engine::vector_cmds::path_json(&path)});
            let result = app.run("path.set", params)?;
            let original = app.session.active().and_then(|st| st.doc.work_path.clone());
            app.ui.tool_options.shape.path_geometry = original.and_then(|p| app.session.active().map(|st| (st.doc.id.0, geometry, p)));
            app.ui.selected_path = Some("work".into());
            Ok(result)
        }
        "pixels" => {
            geometry["mode"] = json!(app.ui.tool_options.shape.blend);
            geometry["opacity"] = json!(app.ui.tool_options.shape.opacity);
            geometry["antiAlias"] = json!(app.ui.tool_options.shape.anti_alias);
            geometry["color"] = json!(app.session.tools.foreground);
            geometry["target"] = crate::canvas::paint_target(app);
            app.run("shape.drawPixels", geometry)
        }
        _ => app.run("shape.create", geometry),
    }
}

pub(crate) fn path_display(app: &PhotocraftApp, ctx: &egui::Context) -> (egui::Color32, f32) {
    let o = &app.ui.tool_options.shape;
    let color = o.path_color.map_or(crate::theme::Tokens::get(ctx).accent, |c| egui::Color32::from_rgb(c[0], c[1], c[2]));
    (color, o.path_width.clamp(0.5, 10.0))
}

fn run(app: &mut PhotocraftApp, id: &str, p: Value) {
    if let Err(e) = app.run(id, p) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

fn menu_button(ui: &mut egui::Ui, icon: &str, label: &str) -> egui::Response {
    let r = crate::icons::button(ui, icon, 24.0, false, label);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label));
    r
}

pub(crate) fn actions(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool) {
    let mode = app.ui.tool_options.shape.mode.clone();
    if mode == "pixels" {
        ui.label(tl!("Mode:"));
        let opts: Vec<_> = photocraft_color::BlendMode::LAYER_MODES.into_iter().map(|m| (m, m.label())).collect();
        crate::widgets::dropdown(ui, "shape-pixel-blend", &mut app.ui.tool_options.shape.blend, &opts, 126.0);
        ui.label(tl!("Opacity:"));
        crate::widgets::value_field(ui, &mut app.ui.tool_options.shape.opacity, 0.0..=100.0, "%", 58.0);
        ui.checkbox(&mut app.ui.tool_options.shape.anti_alias, tl!("Anti-alias"));
    } else if mode == "path" {
        let name = app.ui.selected_path.clone().filter(|n| n != "layer").unwrap_or("work".into());
        let path = path_shape(app);
        ui.add_enabled_ui(path.is_some(), |ui| {
            if ui.button(tl!("Selection")).clicked() {
                run(app, "path.toSelection", json!({"name":name}));
            }
            let mask = app
                .session
                .active()
                .and_then(|st| st.active_layer.and_then(|id| st.doc.layer(id)))
                .is_some_and(|l| !matches!(l.content, LayerContent::Shape(_)) && !app.session.active().is_some_and(|st| st.doc.effective_locks(l.id).all));
            if ui.add_enabled(mask, egui::Button::new(tl!("Mask"))).clicked() {
                run(app, "layer.vectorMask.fromPath", json!({"name":name}));
            }
            if ui.button(tl!("Shape")).clicked()
                && let Some(path) = &path
            {
                let geometry = json!({"kind":"path","path":photocraft_engine::vector_cmds::path_json(&path.path),"fill":app.session.tools.foreground});
                match app.run("shape.create", geometry) {
                    Ok(_) => {
                        app.ui.selected_path = Some("layer".into());
                        app.ui.tool_options.shape.mode = "shape".into();
                    }
                    Err(e) => {
                        app.ui.status = e;
                        app.ui.status_error = true;
                    }
                }
            }
        });
    }
    options_menu(app, ui, tool);
    ui.checkbox(&mut app.ui.tool_options.shape.align_edges, tl!("Align Edges"));
}

fn options_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool) {
    let response = menu_button(ui, "settings", tl!("Path Options"));
    egui::Popup::menu(&response).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        let o = &mut app.ui.tool_options.shape;
        ui.checkbox(&mut o.show_path, tl!("Show Path"));
        ui.horizontal(|ui| {
            ui.label(tl!("Width:"));
            crate::widgets::value_field(ui, &mut o.path_width, 0.5..=10.0, "px", 62.0);
        });
        let mut color = o.path_color.unwrap_or_else(|| {
            let c = crate::theme::Tokens::get(ui.ctx()).accent;
            [c.r(), c.g(), c.b()]
        });
        ui.horizontal(|ui| {
            ui.label(tl!("Color:"));
            if ui.color_edit_button_srgb(&mut color).changed() {
                o.path_color = Some(color);
            }
        });
        if ui.button(tl!("Reset")).clicked() {
            o.path_color = None;
            o.path_width = 1.0;
            o.show_path = true;
        }
        ui.separator();
        ui.checkbox(&mut o.from_center, tl!("From Center"));
        ui.checkbox(&mut o.constrain, tl!("Constrain Proportions"));
        if tool != Tool::Line {
            for (mode, label) in
                [("unconstrained", tl!("Unconstrained")), ("square", tl!("Square")), ("fixed", tl!("Fixed Size")), ("proportional", tl!("Proportional"))]
            {
                ui.radio_value(&mut o.size_mode, mode.into(), label);
            }
            if matches!(o.size_mode.as_str(), "fixed" | "proportional") {
                for (label, value) in [(tl!("Width:"), &mut o.size_width), (tl!("Height:"), &mut o.size_height)] {
                    ui.horizontal(|ui| {
                        ui.label(label);
                        let mut number = *value as f32;
                        if crate::widgets::value_field(ui, &mut number, 1.0..=300000.0, "px", 70.0).changed() {
                            *value = f64::from(number);
                        }
                    });
                }
            }
        }
    });
}

#[cfg(test)]
#[path = "shape_modes_tests.rs"]
mod tests;
