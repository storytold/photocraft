mod fit;
mod script;

use photocraft_doc::{LayerContent, LayerId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;

const CLEAN_LAYER: &str = "TypeR Clean";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Style {
    pub name: String,
    pub prefixes: String,
    pub font: String,
    pub font_style: String,
    pub size: f32,
    pub leading: f32,
    pub tracking: f32,
    pub color: [u8; 3],
    pub all_caps: bool,
    pub faux_bold: bool,
    pub faux_italic: bool,
    pub stroke: f32,
    pub stroke_color: [u8; 3],
}

impl Default for Style {
    fn default() -> Self {
        Style {
            name: "Normal".into(),
            prefixes: String::new(),
            font: String::new(),
            font_style: String::new(),
            size: 24.0,
            leading: 0.0,
            tracking: 0.0,
            color: [0, 0, 0],
            all_caps: false,
            faux_bold: false,
            faux_italic: false,
            stroke: 0.0,
            stroke_color: [255, 255, 255],
        }
    }
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

impl Style {
    fn params(&self, scale: f32, color: [u8; 3]) -> Value {
        let mut p = json!({
            "size": (self.size * scale).clamp(0.1, 1296.0),
            "tracking": self.tracking,
            "color": hex(color),
            "caps": if self.all_caps { "all" } else { "normal" },
            "fauxBold": self.faux_bold,
            "fauxItalic": self.faux_italic,
            "leading": if self.leading > 0.0 { json!(self.leading * scale) } else { json!("auto") },
        });
        if !self.font.is_empty() {
            p["font"] = json!(self.font);
            if !self.font_style.is_empty() {
                p["fontStyle"] = json!(self.font_style);
            }
        }
        p
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub auto_clean: bool,
    pub paste_on_click: bool,
    pub min_scale: f32,
    pub hyphenate: bool,
    pub padding: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { auto_clean: false, paste_on_click: true, min_scale: 0.75, hyphenate: true, padding: 6.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Data {
    pub script: String,
    pub current: usize,
    pub styles: Vec<Style>,
    pub default_style: usize,
    pub settings: Settings,
}

impl Default for Data {
    fn default() -> Self {
        Data { script: String::new(), current: 0, styles: vec![Style::default()], default_style: 0, settings: Settings::default() }
    }
}

#[derive(Default)]
pub struct TypeR {
    pub open: bool,
    data: Data,
    loaded: bool,
    saved: Option<Data>,
    tab: usize,
    editing_script: bool,
    edited_style: usize,
    fonts: Vec<String>,
    font_filter: String,
    status: String,
    seen_doc: usize,
    seen_selection: Option<photocraft_geom::Rect>,
    armed: bool,
}

impl TypeR {
    fn lines(&self) -> Vec<script::Line> {
        let prefixes: Vec<(String, usize)> = self
            .data
            .styles
            .iter()
            .enumerate()
            .flat_map(|(i, s)| s.prefixes.split(',').map(str::trim).filter(|p| !p.is_empty()).map(move |p| (p.to_string(), i)).collect::<Vec<_>>())
            .collect();
        script::parse(&self.data.script, &prefixes)
    }

    fn current(&self, lines: &[script::Line]) -> Option<usize> {
        let from = self.data.current.min(lines.len().saturating_sub(1));
        (from..lines.len()).chain((0..from).rev()).find(|&i| !lines[i].ignore)
    }

    fn step(&mut self, lines: &[script::Line], forward: bool) {
        let Some(cur) = self.current(lines) else { return };
        let next = if forward { (cur + 1..lines.len()).find(|&i| !lines[i].ignore) } else { (0..cur).rev().find(|&i| !lines[i].ignore) };
        if let Some(n) = next {
            self.data.current = n;
        }
    }
}

fn load(app: &mut PhotocraftApp) {
    if app.typer.loaded {
        return;
    }
    app.typer.loaded = true;
    let mut data: Data = serde_json::from_value(app.session.prefs().typer.clone()).unwrap_or_default();
    if data.styles.is_empty() {
        data.styles.push(Style::default());
    }
    app.typer.saved = Some(data.clone());
    app.typer.data = data;
}

fn persist(app: &mut PhotocraftApp) {
    if app.typer.saved.as_ref() == Some(&app.typer.data) {
        return;
    }
    if let Ok(v) = serde_json::to_value(&app.typer.data) {
        app.session.prefs.edit(|p| p.typer = v);
        app.typer.saved = Some(app.typer.data.clone());
    }
}

pub const MENU_ID: &str = "window.panel.typer";

pub fn menu(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    app.typer.open = params.get("show").and_then(Value::as_bool).unwrap_or(!app.typer.open);
    Ok(json!({ "visible": app.typer.open }))
}

fn find_layer(layers: &[photocraft_doc::Layer], name: &str) -> Option<LayerId> {
    layers.iter().rev().find(|l| l.name == name && matches!(l.content, LayerContent::Raster(_))).map(|l| l.id)
}

fn paste(app: &mut PhotocraftApp) -> Result<String, String> {
    let lines = app.typer.lines();
    let Some(cur) = app.typer.current(&lines) else { return Err("Aucune ligne à coller dans le script.".into()) };
    let line = lines[cur].clone();
    let data = app.typer.data.clone();
    let style = line.style.or(Some(data.default_style)).and_then(|i| data.styles.get(i)).cloned().unwrap_or_default();
    let settings = &data.settings;

    let st = app.session.active().ok_or("Aucun document ouvert.")?;
    let doc = st.doc.clone();
    let sel = doc.selection.as_ref().ok_or("Sélectionne une bulle (baguette magique, rectangle ou lasso).")?;
    let bounds = sel.content_bounds();
    if bounds.is_empty() || bounds.width() * bounds.height() < 200 {
        return Err("La sélection est trop petite.".into());
    }
    let (w, h) = (bounds.width() as usize, bounds.height() as usize);
    let n = sel.channels().max(1);
    let cov: Vec<f32> = sel.read_region(bounds).chunks(n).map(|c| c[0]).collect();
    let shape = fit::Shape::from_mask(&cov, w, h, bounds.x0, bounds.y0).ok_or("La sélection est vide.")?;

    let composite = photocraft_compose::render(&doc, bounds);
    let picked: Vec<[f32; 3]> = cov.iter().zip(&composite.px).filter(|(c, _)| **c >= 0.5).map(|(_, p)| [p[0], p[1], p[2]]).step_by(7).collect();
    let mean = picked.iter().fold([0.0f32; 3], |a, p| [a[0] + p[0], a[1] + p[1], a[2] + p[2]]).map(|v| v / picked.len().max(1) as f32);
    let luma = 0.2126 * mean[0] + 0.7152 * mean[1] + 0.0722 * mean[2];
    let close = picked.iter().filter(|p| (0..3).all(|i| (p[i] - mean[i]).abs() < 0.12)).count();
    let solid = close as f32 >= 0.9 * picked.len() as f32;
    let dark = luma < 0.35;
    let (color, stroke) = if dark { ([255, 255, 255], 0.0) } else { (style.color, style.stroke) };

    let dpi = doc.resolution_dpi;
    let px_per_pt = dpi / 72.0;
    let mut layer = photocraft_engine::cmd::type_cmds::new_text_layer(&app.session, &style.params(1.0, color), "type.create").map_err(|e| e.to_string())?;
    let mut lay = |text: &str| {
        layer.text = text.to_string();
        layer.runs.truncate(1);
        layer.paragraphs.truncate(1);
        layer.runs[0].len = text.len();
        layer.paragraphs[0].len = text.len();
        layer.sync_summary();
        photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner()).layout(&layer, dpi)
    };
    let two = lay("Hg\nHg");
    let metrics = match two.lines.as_slice() {
        [a, b, ..] => fit::Metrics { ascent: a.ascent, descent: a.descent, pitch: b.baseline - a.baseline },
        _ => fit::Metrics { ascent: style.size * px_per_pt * 0.8, descent: style.size * px_per_pt * 0.2, pitch: style.size * px_per_pt * 1.2 },
    };
    let text = if style.all_caps { line.text.to_uppercase() } else { line.text.clone() };
    let mut measure = |s: &str| lay(s).lines.first().map_or(0.0, |l| l.x1 - l.x0);
    let pad = settings.padding.max(0.0) + stroke;
    let plan = fit::solve(&shape.eroded(pad), &text, metrics, &mut measure, settings.min_scale, settings.hyphenate);

    let key = format!("TypeR Paste {}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis()));
    let k = || json!(key);
    let run = |app: &mut PhotocraftApp, id: &str, mut p: Value| {
        p["coalesce"] = k();
        app.run(id, p)
    };

    let white = luma > 0.85;
    let cleaned = settings.auto_clean && solid && (white || luma < 0.15);
    if cleaned {
        match find_layer(&doc.layers, CLEAN_LAYER) {
            Some(id) => {
                run(app, "layer.select", json!({ "layer": id.0 }))?;
            }
            None => {
                if let Some(top) = doc.layers.iter().rev().find(|l| matches!(l.content, LayerContent::Raster(_))) {
                    run(app, "layer.select", json!({ "layer": top.id.0 }))?;
                }
                run(app, "layer.new.layer", json!({ "name": CLEAN_LAYER }))?;
            }
        }
        let points: Vec<[f32; 2]> = shape.polygon();
        run(app, "select.lasso", json!({ "points": points, "antiAlias": false, "polygonal": true }))?;
        let fill = if white { [255u8, 255, 255] } else { [0, 0, 0] };
        run(app, "edit.fill", json!({ "contents": "color", "color": hex(fill) }))?;
    }

    let name: String = line.text.chars().take(100).collect();
    let (layer_id, target, message) = match &plan {
        Some(plan) => {
            let mut p = style.params(plan.scale, color);
            p["text"] = json!(plan.lines.join("\n"));
            p["x"] = json!(plan.center.0.round());
            p["y"] = json!(plan.first_baseline.round());
            p["align"] = json!("center");
            p["name"] = json!(name);
            let created = run(app, "type.create", p)?;
            let msg = if plan.scale < 0.999 { format!("Collé à {:.0} % de la taille du style.", plan.scale * 100.0) } else { "Collé.".into() };
            let msg = if plan.hyphenated { format!("{msg} Avec une césure.") } else { msg };
            (created["layer"].as_u64(), plan.center, msg)
        }
        None => {
            let width = (bounds.width() as f32 * 0.9).max(10.0);
            let mut p = style.params(settings.min_scale, color);
            p["text"] = json!(text);
            p["box"] = json!([bounds.x0 as f32 + (bounds.width() as f32 - width) / 2.0, bounds.y0, width, bounds.height()]);
            p["align"] = json!("center");
            p["name"] = json!(name);
            let created = run(app, "type.create", p)?;
            let c = shape.centroid().unwrap_or(((bounds.x0 + bounds.x1) as f32 / 2.0, (bounds.y0 + bounds.y1) as f32 / 2.0));
            (created["layer"].as_u64(), c, "Le texte ne tient pas dans la bulle : collage classique.".into())
        }
    };
    let layer_id = layer_id.ok_or("type.create n'a pas renvoyé de calque")?;
    if let Some(b) = run(app, "type.info", json!({ "layer": layer_id }))?["bounds"].as_array().filter(|b| b.len() == 4) {
        let v: Vec<f64> = b.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect();
        let (dx, dy) = ((f64::from(target.0) - (v[0] + v[2]) / 2.0).round(), (f64::from(target.1) - (v[1] + v[3]) / 2.0).round());
        if dx != 0.0 || dy != 0.0 {
            run(app, "type.edit", json!({ "layer": layer_id, "move": [dx, dy] }))?;
        }
    }
    if stroke > 0.0 {
        run(app, "layer.layerStyle.stroke", json!({ "layer": layer_id, "size": stroke, "position": "outside", "color": hex(style.stroke_color) }))?;
    }
    if app.session.active().is_some_and(|st| st.doc.selection.is_some()) {
        run(app, "select.deselect", json!({}))?;
    }
    app.typer.step(&lines, true);
    Ok(if cleaned { format!("{message} Bulle nettoyée.") } else { message })
}

fn do_paste(app: &mut PhotocraftApp) {
    app.typer.status = match paste(app) {
        Ok(m) => m,
        Err(e) => format!("⚠ {e}"),
    };
    app.typer.seen_selection = None;
}

fn watch_selection(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let canvas = app.last_canvas_rect;
    if ctx.input(|i| i.pointer.any_pressed() && i.pointer.press_origin().is_some_and(|p| canvas.contains(p))) {
        app.typer.armed = true;
    }
    if ctx.input(|i| i.pointer.any_down()) || app.ui.text_edit.is_some() {
        return;
    }
    let Some(st) = app.session.active() else { return };
    let ptr = std::sync::Arc::as_ptr(&st.doc) as usize;
    if ptr == app.typer.seen_doc {
        return;
    }
    let first = app.typer.seen_doc == 0;
    app.typer.seen_doc = ptr;
    let now = st.doc.selection.as_ref().map(|s| s.content_bounds()).filter(|b| !b.is_empty());
    let changed = now.is_some() && now != app.typer.seen_selection;
    app.typer.seen_selection = now;
    let armed = std::mem::take(&mut app.typer.armed);
    if changed && armed && !first && app.typer.data.settings.paste_on_click {
        do_paste(app);
    }
}

fn style_from_active_layer(app: &mut PhotocraftApp) -> Result<Style, String> {
    let info = app.run("type.info", json!({}))?;
    let s = &info["runs"][0]["style"];
    let f = |k: &str| s[k].as_f64().map(|v| v as f32);
    let c = |v: &Value| -> [u8; 3] {
        let ch = v["c"].as_array().cloned().unwrap_or_default();
        [0, 1, 2].map(|i| (ch.get(i).and_then(Value::as_f64).unwrap_or(0.0) * 255.0).round().clamp(0.0, 255.0) as u8)
    };
    Ok(Style {
        font: s["font_family"].as_str().unwrap_or_default().into(),
        font_style: s["font_style"].as_str().unwrap_or_default().into(),
        size: f("size_pt").unwrap_or(24.0),
        leading: f("leading_pt").unwrap_or(0.0),
        tracking: f("tracking").unwrap_or(0.0),
        color: c(&s["color"]),
        all_caps: s["caps"].as_str() == Some("AllCaps"),
        faux_bold: s["faux_bold"].as_bool().unwrap_or(false),
        faux_italic: s["faux_italic"].as_bool().unwrap_or(false),
        ..Style::default()
    })
}

pub fn window(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.typer.open {
        return;
    }
    load(app);
    watch_selection(app, ctx);
    let canvas = app.last_canvas_rect;
    let mut open = true;
    let mut action: Option<fn(&mut PhotocraftApp)> = None;
    egui::Window::new("TypeR")
        .id(egui::Id::unique("typer-window"))
        .open(&mut open)
        .default_pos(egui::pos2((canvas.right() - 340.0).max(canvas.left() + 8.0), canvas.top() + 40.0))
        .default_width(320.0)
        .resizable(true)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (i, name) in ["Script", "Styles", "Réglages"].iter().enumerate() {
                    if crate::widgets::pill_tab(ui, name, app.typer.tab == i).clicked() {
                        app.typer.tab = i;
                    }
                }
            });
            ui.separator();
            match app.typer.tab {
                0 => action = script_tab(app, ui),
                1 => styles_tab(app, ui),
                _ => settings_tab(&mut app.typer.data.settings, ui),
            }
            if !app.typer.status.is_empty() {
                ui.separator();
                ui.label(egui::RichText::new(&app.typer.status).small());
            }
        });
    app.typer.open = open;
    if let Some(f) = action {
        f(app);
    }
    persist(app);
}

fn script_tab(app: &mut PhotocraftApp, ui: &mut egui::Ui) -> Option<fn(&mut PhotocraftApp)> {
    let mut action: Option<fn(&mut PhotocraftApp)> = None;
    let t = &mut app.typer;
    ui.horizontal(|ui| {
        ui.toggle_value(&mut t.editing_script, "✏ Modifier le script");
        ui.checkbox(&mut t.data.settings.paste_on_click, "Coller au clic");
    });
    if t.editing_script {
        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            ui.add(egui::TextEdit::multiline(&mut t.data.script).desired_rows(16).desired_width(f32::INFINITY).hint_text("Page 1\nREG: Une réplique.\n## Une note de traduction"));
        });
        return None;
    }
    let lines = t.lines();
    let cur = t.current(&lines);
    let mut clicked = None;
    egui::ScrollArea::vertical().max_height(320.0).auto_shrink([false, true]).show(ui, |ui| {
        let mut number = 0;
        for (i, l) in lines.iter().enumerate() {
            if let Some(p) = l.page {
                ui.label(egui::RichText::new(format!("— Page {p} —")).weak().small());
                continue;
            }
            if l.ignore {
                continue;
            }
            number += 1;
            let style = l.style.and_then(|s| t.data.styles.get(s)).map(|s| format!("  [{}]", s.name)).unwrap_or_default();
            let r = ui.selectable_label(cur == Some(i), format!("{number}. {}{style}", l.text));
            if r.clicked() {
                clicked = Some(i);
            }
        }
        if lines.iter().all(|l| l.ignore) {
            ui.weak("Colle ton script avec « Modifier le script ».");
        }
    });
    if let Some(i) = clicked {
        t.data.current = i;
    }
    ui.horizontal(|ui| {
        if ui.button("◀").on_hover_text("Ligne précédente").clicked() {
            t.step(&lines, false);
        }
        if ui.button("▶").on_hover_text("Ligne suivante").clicked() {
            t.step(&lines, true);
        }
        if ui.button("Coller dans la sélection").clicked() {
            action = Some(do_paste);
        }
    });
    action
}

fn styles_tab(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if app.typer.fonts.is_empty()
        && let Ok(v) = app.run("type.fonts", json!({}))
    {
        app.typer.fonts = v["families"].as_array().map(|a| a.iter().filter_map(|f| f.as_str().map(str::to_string)).collect()).unwrap_or_default();
    }
    let mut from_layer = false;
    {
        let t = &mut app.typer;
        let d = &mut t.data;
        ui.horizontal_wrapped(|ui| {
            for i in 0..d.styles.len() {
                let label = if i == d.default_style { format!("★ {}", d.styles[i].name) } else { d.styles[i].name.clone() };
                if ui.selectable_label(t.edited_style == i, label).clicked() {
                    t.edited_style = i;
                }
            }
            if ui.button("+").on_hover_text("Nouveau style").clicked() {
                d.styles.push(Style { name: format!("Style {}", d.styles.len() + 1), ..d.styles.get(t.edited_style).cloned().unwrap_or_default() });
                t.edited_style = d.styles.len() - 1;
            }
        });
        t.edited_style = t.edited_style.min(d.styles.len() - 1);
        let i = t.edited_style;
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Depuis le calque actif").on_hover_text("Copie la police, la taille et la couleur du calque de texte actif").clicked() {
                from_layer = true;
            }
            if ui.add_enabled(d.default_style != i, egui::Button::new("★ Par défaut")).clicked() {
                d.default_style = i;
            }
            if ui.add_enabled(d.styles.len() > 1, egui::Button::new("Supprimer")).clicked() {
                d.styles.remove(i);
                d.default_style = if d.default_style > i { d.default_style - 1 } else { d.default_style.min(d.styles.len() - 1) };
                t.edited_style = i.min(d.styles.len() - 1);
                return;
            }
        });
        let s = &mut d.styles[t.edited_style];
        egui::Grid::new("typer-style").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            ui.label("Nom");
            ui.text_edit_singleline(&mut s.name);
            ui.end_row();
            ui.label("Préfixes");
            ui.add(egui::TextEdit::singleline(&mut s.prefixes).hint_text("REG:, R:"));
            ui.end_row();
            ui.label("Police");
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut s.font).desired_width(140.0).hint_text("par défaut"));
                egui::ComboBox::from_id_salt("typer-font").selected_text("").width(24.0).show_ui(ui, |ui| {
                    ui.text_edit_singleline(&mut t.font_filter);
                    let filter = t.font_filter.to_lowercase();
                    for f in t.fonts.iter().filter(|f| f.to_lowercase().contains(&filter)).take(300) {
                        if ui.selectable_label(*f == s.font, f).clicked() {
                            s.font = f.clone();
                            s.font_style.clear();
                        }
                    }
                });
            });
            ui.end_row();
            ui.label("Graisse");
            ui.add(egui::TextEdit::singleline(&mut s.font_style).hint_text("Regular, Bold…"));
            ui.end_row();
            ui.label("Taille (pt)");
            ui.add(egui::DragValue::new(&mut s.size).range(1.0..=1296.0).speed(0.2));
            ui.end_row();
            ui.label("Interligne (pt)");
            ui.add(egui::DragValue::new(&mut s.leading).range(0.0..=2000.0).speed(0.2).custom_formatter(|v, _| if v <= 0.0 { "auto".into() } else { format!("{v:.1}") }));
            ui.end_row();
            ui.label("Approche");
            ui.add(egui::DragValue::new(&mut s.tracking).range(-1000.0..=10000.0));
            ui.end_row();
            ui.label("Couleur");
            ui.color_edit_button_srgb(&mut s.color);
            ui.end_row();
            ui.label("Contour (px)");
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut s.stroke).range(0.0..=250.0).speed(0.1));
                ui.color_edit_button_srgb(&mut s.stroke_color);
            });
            ui.end_row();
            ui.label("Options");
            ui.horizontal(|ui| {
                ui.checkbox(&mut s.all_caps, "Capitales");
                ui.checkbox(&mut s.faux_bold, "Gras");
                ui.checkbox(&mut s.faux_italic, "Italique");
            });
            ui.end_row();
        });
    }
    if from_layer {
        match style_from_active_layer(app) {
            Ok(new) => {
                let s = &mut app.typer.data.styles[app.typer.edited_style];
                *s = Style { name: s.name.clone(), prefixes: s.prefixes.clone(), stroke: s.stroke, stroke_color: s.stroke_color, ..new };
                app.typer.status = "Style copié depuis le calque actif.".into();
            }
            Err(e) => app.typer.status = format!("⚠ Sélectionne un calque de texte ({e})."),
        }
    }
}

fn settings_tab(s: &mut Settings, ui: &mut egui::Ui) {
    ui.checkbox(&mut s.paste_on_click, "Coller au clic (baguette, rectangle ou lasso)");
    ui.checkbox(&mut s.auto_clean, "Auto clean : peindre la bulle sous le texte (calque « TypeR Clean », bulles unies blanches ou noires)");
    ui.checkbox(&mut s.hyphenate, "Autoriser une césure (français) en dernier recours");
    ui.horizontal(|ui| {
        ui.label("Taille minimale");
        let mut pct = s.min_scale * 100.0;
        if ui.add(egui::Slider::new(&mut pct, 30.0..=100.0).suffix(" %")).changed() {
            s.min_scale = pct / 100.0;
        }
    });
    ui.horizontal(|ui| {
        ui.label("Marge intérieure");
        ui.add(egui::DragValue::new(&mut s.padding).range(0.0..=200.0).suffix(" px"));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bubble() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        for (cmd, p) in [
            ("file.new", json!({"width": 400, "height": 300, "background": "white"})),
            ("edit.fill", json!({"color": "#808080"})),
            ("select.rect", json!({"x": 50, "y": 50, "width": 300, "height": 200, "ellipse": true})),
            ("edit.fill", json!({"color": "#000000"})),
            ("select.rect", json!({"x": 55, "y": 55, "width": 290, "height": 190, "ellipse": true})),
            ("edit.fill", json!({"color": "#ffffff"})),
            ("select.rect", json!({"x": 180, "y": 140, "width": 40, "height": 10})),
            ("edit.fill", json!({"color": "#000000"})),
            ("select.magicWand", json!({"x": 120, "y": 150, "tolerance": 20, "contiguous": true, "sampleAllLayers": true})),
        ] {
            app.run(cmd, p).unwrap();
        }
        app
    }

    #[test]
    fn paste_fits_the_line_in_the_bubble_cleans_it_and_advances() {
        let mut app = bubble();
        app.typer.loaded = true;
        app.typer.data.script = "Page 1\nPremière réplique, assez longue pour tenir sur plusieurs lignes dans la bulle.\nDeuxième".into();
        app.typer.data.settings.auto_clean = true;
        let msg = paste(&mut app).unwrap();
        assert!(msg.contains("nettoyée"), "{msg}");
        let st = app.session.active().unwrap();
        assert!(st.doc.selection.is_none());
        let text_id = st.doc.layers.iter().find(|l| matches!(l.content, LayerContent::Text(_))).expect("a type layer").id;
        let info = app.run("type.info", json!({"layer": text_id.0})).unwrap();
        let b: Vec<f64> = info["bounds"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
        assert!(b[0] > 55.0 && b[1] > 55.0 && b[2] < 345.0 && b[3] < 245.0, "inside the bubble: {b:?}");
        assert!(((b[0] + b[2]) / 2.0 - 200.0).abs() < 3.0 && ((b[1] + b[3]) / 2.0 - 150.0).abs() < 3.0, "centred: {b:?}");
        assert!(info["lines"].as_array().unwrap().len() >= 2, "{info}");
        let st = app.session.active().unwrap();
        let clean = st.doc.layers.iter().position(|l| l.name == CLEAN_LAYER).expect("clean layer");
        let LayerContent::Raster(s) = &st.doc.layers[clean].content else { panic!() };
        assert!(s.pixel(200, 145).iter().all(|v| *v > 0.99), "{:?}", s.pixel(200, 145));
        assert!(clean < st.doc.layers.iter().position(|l| l.id == text_id).unwrap());
        let lines = app.typer.lines();
        assert_eq!(lines[app.typer.current(&lines).unwrap()].text, "Deuxième");
        let before = app.session.active().unwrap().doc.layers.len();
        app.run("edit.undo", json!({})).unwrap();
        assert!(app.session.active().unwrap().doc.layers.len() <= before - 2);
    }

    #[test]
    fn black_bubbles_get_white_text() {
        let mut app = bubble();
        app.run("edit.fill", json!({"color": "#000000"})).unwrap();
        app.typer.loaded = true;
        app.typer.data.script = "Nuit".into();
        paste(&mut app).unwrap();
        let st = app.session.active().unwrap();
        let LayerContent::Text(t) = &st.doc.layers.iter().find(|l| matches!(l.content, LayerContent::Text(_))).unwrap().content else { panic!() };
        assert!(t.runs[0].style.color.c[0] > 0.99, "{:?}", t.runs[0].style.color);
    }
}
