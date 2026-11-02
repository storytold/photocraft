//! Window › Generate (ShotoPhopEX): the docked prompt box, ArtCraft-style. One prompt shaped by
//! the chosen template's capabilities (`generate.models`): a reference deck when the template
//! takes reference images, aspect and size pickers for text to image, a count, the Enhance wand,
//! and a Generate button that shows the time estimate (`generate_cmds::estimate_seconds`, the
//! formula the CLI and scripts use) and turns into Cancel while the job runs. A segmented switch
//! picks Image · Fill · Edit · Expand; below it the form reads top to bottom in sections
//! (References, Prompt, Settings) with one full-width Generate at the end. When the dock squeezes
//! the group it shows one row: mode, prompt, Generate.
//!
//! The task bar under a selection (`generative_bar`) is this panel's compact form: both read and
//! write [`GeneratePanel`]'s prompt, per-mode template and count, so they never disagree. Every
//! run is an engine command (`generate.image`, `.fill`, `.edit`, `.expand`), so the Generations
//! library records it and automation can do the same through `ui.set {generatePanel}` and
//! `ui.inspect.generatePanel`.

use std::collections::BTreeMap;

use egui::{Align2, FontId, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use photocraft_engine::jobs::{JobEvent, JobId, JobOutcome};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::icons;
use crate::theme::Tokens;
use crate::widgets;

pub const MODES: [&str; 4] = ["image", "fill", "edit", "expand"];
/// Sizes the image page offers: the longer side in pixels (the template's limits apply).
pub const SIZES: [(&str, u32); 3] = [("1K", 1024), ("1.5K", 1536), ("2K", 2048)];
/// The prompt's length limit (the engine's `MAX_PROMPT_CHARS`).
pub const MAX_PROMPT: usize = 4000;
/// Below this content height the group shows its one-row compact form.
const COMPACT_BELOW: f32 = 150.0;
/// Reference thumbnails in the deck.
const THUMB: f32 = 48.0;

/// One template as the panel needs it (from `generate.models`, without contacting the server).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub task: String,
    pub pages: Vec<String>,
    pub refs_max: u32,
    pub refs_min: u32,
    pub aspects: Vec<String>,
    pub size_min: u32,
    pub size_max: u32,
    pub step: u32,
    pub counts_max: u32,
    pub allowed: bool,
    pub family: String,
    pub description: String,
    /// A research-only licence.
    pub research: bool,
    /// The server has every file (None until the background probe answers).
    pub installed: Option<bool>,
}

/// The Generate panel's state (`UiState::generate`), shared with the task bar.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneratePanel {
    /// `image`, `fill`, `edit` or `expand`.
    pub mode: String,
    /// A mode was picked (the person, a menu or a script) since the canvas last changed what it
    /// asks for; until then the panel shows the one that fits the canvas ([`context_mode`]).
    #[serde(skip)]
    pub mode_chosen: bool,
    /// The mode the canvas asked for when the panel last looked: a change (a selection made or
    /// dropped, the last document closed or one opened) puts the panel back on the fitting mode.
    #[serde(skip)]
    pub context: Option<&'static str>,
    /// Image mode: `document` (a new document) or `layer` (a layer over the canvas).
    pub into: String,
    pub prompt: String,
    pub negative: String,
    /// The negative prompt field is open (it is behind a disclosure).
    pub show_negative: bool,
    /// Reference images as `references` specs: `{"layer"}`, `{"path"}`, `{"job", "index"}`.
    pub references: Vec<Value>,
    /// The template per mode; empty = Auto / the preference's default.
    pub template: BTreeMap<String, String>,
    /// Image mode: `"1:1"`, `"3:2"`, … or `"custom"`.
    pub aspect: String,
    /// Image mode: `1K`, `1.5K` or `2K` (the longer side).
    pub size: String,
    /// Image mode with a custom aspect: the size itself.
    pub width: u32,
    pub height: u32,
    /// Results per run.
    pub count: u8,
    /// Rewrite the prompt first; None = Preferences › Enhance prompts.
    pub enhance: Option<bool>,
    /// Expand mode: pixels to add on the left, top, right and bottom.
    pub expand: [u32; 4],
    /// Fill mode: widen the selection by this many pixels before the request (`grow`), so a
    /// bulkier replacement has room.
    pub grow: u32,
    /// The job the panel started (its button shows Cancel and the percent meanwhile).
    #[serde(skip)]
    pub job: Option<u64>,
    /// The `generate.enhancePrompt` job of the wand.
    #[serde(skip)]
    pub enhance_job: Option<u64>,
    /// The prompt takes the keyboard focus next frame.
    #[serde(skip)]
    pub focus: bool,
    /// Templates, loaded once.
    #[serde(skip)]
    pub catalog: Vec<ModelInfo>,
    /// Drawn in its compact form last frame.
    #[serde(skip)]
    pub compact: bool,
    /// The frame it was last drawn in (the task bar steps aside meanwhile in `auto` mode).
    #[serde(skip)]
    pub drawn_frame: Option<u64>,
    /// Where files dropped onto the panel become references (last frame's rect).
    #[serde(skip)]
    pub drop_rect: Option<egui::Rect>,
    /// "Clear all" with references attached waits for a confirmation.
    #[serde(skip)]
    pub confirm_clear: bool,
    /// The prompt is open in its large editor.
    #[serde(skip)]
    pub editor: bool,
    /// The server was asked which templates it has (the picker's `not installed` badges).
    #[serde(skip)]
    pub probed: bool,
    /// LoRA adapters by file name: switched on, and the strength (the LoRAs rows; a run sends
    /// the ones on for the template's family as `loras`).
    pub loras: BTreeMap<String, (bool, f32)>,
    /// `generate.loras` for a template: (template, runs on the native engine, its files).
    #[serde(skip)]
    pub lora_list: Option<(String, bool, Vec<String>)>,
    /// Fill mode on a template whose sampler takes it: how much of the selection is redrawn
    /// (`denoise`; 1 replaces it).
    pub denoise: f32,
}

impl Default for GeneratePanel {
    fn default() -> Self {
        Self {
            mode: "image".into(),
            mode_chosen: false,
            context: None,
            into: "document".into(),
            prompt: String::new(),
            negative: String::new(),
            show_negative: false,
            references: Vec::new(),
            template: BTreeMap::new(),
            aspect: "1:1".into(),
            size: "1K".into(),
            width: 1024,
            height: 1024,
            count: 1,
            enhance: None,
            expand: [0, 0, 256, 0],
            grow: 0,
            job: None,
            enhance_job: None,
            focus: false,
            catalog: Vec::new(),
            compact: false,
            drawn_frame: None,
            drop_rect: None,
            confirm_clear: false,
            editor: false,
            probed: false,
            loras: BTreeMap::new(),
            lora_list: None,
            denoise: 1.0,
        }
    }
}

/// Ask the engine which LoRAs fit `template` (the files' headers in the models folder, no
/// server), once per template.
pub fn refresh_loras(app: &mut PhotocraftApp, template: &str) {
    if app.ui.generate.lora_list.as_ref().is_some_and(|(t, _, _)| t == template) {
        return;
    }
    let v = app.session.execute("generate.loras", json!({"template": template})).unwrap_or(Value::Null);
    let files = v["loras"].as_array().map(|a| a.iter().filter_map(|l| l["file"].as_str().map(str::to_string)).collect()).unwrap_or_default();
    app.ui.generate.lora_list = Some((template.to_string(), v["native"].as_bool().unwrap_or(false), files));
}

/// The `loras` a run of `mode` sends: the switched-on files of the template's list, when the
/// template runs on the native engine (the route that applies them).
fn chosen_loras(app: &PhotocraftApp, mode: &str) -> Vec<Value> {
    let p = &app.ui.generate;
    match &p.lora_list {
        Some((t, true, files)) if matches!(mode, "image" | "edit" | "fill") && *t == effective_template(app, mode) => {
            files.iter().filter_map(|f| p.loras.get(f).filter(|(on, _)| *on).map(|(_, s)| json!({"file": f, "strength": s}))).collect()
        }
        _ => Vec::new(),
    }
}

/// The engine command a mode runs.
pub fn command_of(mode: &str) -> &'static str {
    match mode {
        "fill" => "generate.fill",
        "edit" => "generate.edit",
        "expand" => "generate.expand",
        _ => "generate.image",
    }
}

fn mode_label(mode: &str) -> &'static str {
    match mode {
        "fill" => tl!("Fill"),
        "edit" => tl!("Edit"),
        "expand" => tl!("Expand"),
        _ => tl!("Image"),
    }
}

fn hint(mode: &str) -> &'static str {
    match mode {
        "fill" => tl!("Describe what to generate in the selection…"),
        "edit" => tl!("Describe the change…"),
        "expand" => tl!("Describe what continues the scene…"),
        _ => tl!("Describe what you want in the image…"),
    }
}

/// Load the template list once (offline: `generate.models` without the server; the panel is
/// on screen from startup and must not contact the server by itself).
pub fn load_catalog(app: &mut PhotocraftApp) {
    if !app.ui.generate.catalog.is_empty() {
        return;
    }
    let Ok(v) = app.session.execute("generate.models", json!({"probe": false})) else { return };
    let num = |t: &Value, a: &str, b: &str| t[a][b].as_u64().unwrap_or(0) as u32;
    app.ui.generate.catalog = v["templates"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|t| {
                    Some(ModelInfo {
                        id: t["id"].as_str()?.to_string(),
                        name: t["name"].as_str().unwrap_or_default().to_string(),
                        task: t["task"].as_str().unwrap_or_default().to_string(),
                        pages: t["pages"].as_array().map(|p| p.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default(),
                        refs_max: num(t, "references", "max"),
                        refs_min: num(t, "references", "min"),
                        aspects: t["sizes"]["aspects"]
                            .as_array()
                            .map(|p| p.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                            .unwrap_or_default(),
                        size_min: num(t, "sizes", "min"),
                        size_max: num(t, "sizes", "max"),
                        step: num(t, "sizes", "step"),
                        counts_max: num(t, "counts", "max").max(1),
                        allowed: t["allowed"].as_bool().unwrap_or(false),
                        family: t["family"].as_str().unwrap_or_default().to_string(),
                        description: t["description"].as_str().unwrap_or_default().to_string(),
                        research: t["license"] == "research",
                        installed: None,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
}

/// Ask the server which templates it has, once: when a model picker opens (the task bar asks
/// again each time it opens). The background probe fills in both lists.
pub fn probe_once(app: &mut PhotocraftApp) {
    if !app.ui.generate.probed {
        crate::generative_bar::probe_installed(app);
    }
}

/// The model picker's rows for `page` (`fill`, `image`, … as templates list them), with the
/// time estimate at the panel's current area and count when `eta` (the panel and the bar).
pub fn picker_items(app: &PhotocraftApp, page: &str, eta: bool) -> Vec<crate::model_picker::Item> {
    let research_ok = app.session.prefs().integrations.allow_research_models;
    let area = if eta { work_area(app, page) } else { None };
    let count = u32::from(app.ui.generate.count.max(1));
    let refs = if matches!(page, "image" | "edit" | "fill") { app.ui.generate.references.len() as u32 } else { 0 };
    app.ui
        .generate
        .catalog
        .iter()
        .filter(|m| m.pages.iter().any(|p| p == page))
        .map(|m| crate::model_picker::Item {
            id: m.id.clone(),
            name: m.name.clone(),
            family: m.family.clone(),
            description: m.description.clone(),
            research: m.research,
            allowed: m.allowed || research_ok,
            installed: m.installed,
            eta: area.and_then(|(w, h)| photocraft_engine::generate_cmds::estimate_seconds(&m.id, w, h, count, refs)),
        })
        .collect()
}

fn info<'a>(app: &'a PhotocraftApp, id: &str) -> Option<&'a ModelInfo> {
    app.ui.generate.catalog.iter().find(|m| m.id == id)
}

/// The template the panel's choice names for `mode` (validated against the catalogue), if any.
pub fn chosen_template(app: &PhotocraftApp, mode: &str) -> Option<String> {
    let id = app.ui.generate.template.get(mode).map(|s| s.trim()).filter(|s| !s.is_empty() && *s != photocraft_engine::generate_cmds::AUTO_TEMPLATE)?;
    info(app, id).filter(|m| m.pages.iter().any(|p| p == mode)).map(|m| m.id.clone())
}

/// The template a run of `mode` will most likely use: the choice, else the preference's
/// default, else what `auto` picks first; image mode with references and a default that takes
/// none goes to the reference template (as the engine does).
pub fn effective_template(app: &PhotocraftApp, mode: &str) -> String {
    chosen_template(app, mode).unwrap_or_else(|| default_template(app, mode))
}

/// The template a run of `mode` without a choice most likely uses (what the picker's Auto row
/// stands for).
pub fn default_template(app: &PhotocraftApp, mode: &str) -> String {
    use photocraft_engine::generate_cmds as g;
    let prefs = &app.session.prefs().integrations;
    match mode {
        "image" => {
            let t = if prefs.default_image_template.trim().is_empty() {
                g::DEFAULT_IMAGE_TEMPLATE.to_string()
            } else {
                prefs.default_image_template.trim().to_string()
            };
            let takes_refs = info(app, &t).is_some_and(|m| m.refs_max > 0);
            if !app.ui.generate.references.is_empty() && !takes_refs { g::image_ref_template(&app.session).to_string() } else { t }
        }
        "fill" => {
            let t = prefs.default_fill_template.trim();
            let q = g::quality(&app.session);
            let t = if t.is_empty() || t == g::AUTO_TEMPLATE { g::likely_auto_in(&app.session, "generate.fill", q).to_string() } else { t.to_string() };
            // References and a default that takes none: the fill-from-reference template.
            let takes_refs = info(app, &t).is_some_and(|m| m.refs_max > 0);
            if !app.ui.generate.references.is_empty() && !takes_refs { g::fill_ref_template(q).to_string() } else { t }
        }
        m => g::likely_auto_in(&app.session, command_of(m), g::quality(&app.session)).to_string(),
    }
}

/// The name the picker shows for [`default_template`], for its Auto row.
pub fn auto_name(app: &PhotocraftApp, mode: &str) -> Option<String> {
    let id = default_template(app, mode);
    let items = picker_items(app, mode, false);
    let names = crate::model_picker::display_names(&items);
    items.iter().zip(names).find(|(i, _)| i.id == id).map(|(_, n)| n).or_else(|| info(app, &id).map(|m| crate::model_picker::short_name(&m.name).to_string()))
}

/// Image mode's output size: the aspect at the size preset (or the custom size), on the
/// template's grid and within its limits.
pub fn image_size(app: &PhotocraftApp) -> (u32, u32) {
    let p = &app.ui.generate;
    let m = info(app, &effective_template(app, "image")).cloned().unwrap_or_default();
    let (lo, hi, step) = (m.size_min.max(64), if m.size_max == 0 { 2048 } else { m.size_max }, m.step.max(16));
    let snap = |v: u32| (v / step * step).clamp(lo, hi.max(lo));
    if p.aspect == "custom" {
        return (snap(p.width), snap(p.height));
    }
    let long = SIZES.iter().find(|(k, _)| *k == p.size).map_or(1024, |(_, v)| *v);
    let (aw, ah) = p
        .aspect
        .split_once(':')
        .and_then(|(a, b)| Some((a.trim().parse::<f64>().ok()?, b.trim().parse::<f64>().ok()?)))
        .filter(|(a, b)| *a > 0.0 && *b > 0.0)
        .unwrap_or((1.0, 1.0));
    let (w, h) = if aw >= ah { (f64::from(long), f64::from(long) * ah / aw) } else { (f64::from(long) * aw / ah, f64::from(long)) };
    (snap(w.round() as u32), snap(h.round() as u32))
}

/// The area a run of `mode` works on (for the estimate): the image size, the selection with
/// the fill's margin, the picture, or the picture grown by the pads. None when it can't run.
fn work_area(app: &PhotocraftApp, mode: &str) -> Option<(u32, u32)> {
    let d = app.session.active();
    match mode {
        "image" => Some(image_size(app)),
        "fill" => {
            let b = d?.doc.selection.as_ref()?.content_bounds();
            let m = (f64::from(b.width().max(b.height())) * 0.25) as u32;
            (!b.is_empty()).then(|| (b.width() + 2 * m, b.height() + 2 * m))
        }
        "edit" => d.map(|d| (d.doc.size.width, d.doc.size.height)),
        "expand" => {
            let e = app.ui.generate.expand;
            d.map(|d| (d.doc.size.width + e[0] + e[2], d.doc.size.height + e[1] + e[3]))
        }
        _ => None,
    }
}

/// Seconds the next run should take (warm server), for the Generate button.
pub fn estimate(app: &PhotocraftApp, mode: &str) -> Option<f32> {
    let (w, h) = work_area(app, mode)?;
    let count = if mode == "expand" { 1 } else { u32::from(app.ui.generate.count.max(1)) };
    let refs = if matches!(mode, "image" | "edit" | "fill") { app.ui.generate.references.len() as u32 } else { 0 };
    photocraft_engine::generate_cmds::estimate_seconds(&effective_template(app, mode), w, h, count, refs)
}

/// "~24 s", "~2 min".
pub fn eta_label(secs: f32) -> String {
    if !secs.is_finite() || secs <= 0.0 {
        return String::new();
    }
    if secs < 90.0 { format!("~{:.0} s", secs.max(1.0)) } else { format!("~{:.0} min", (secs / 60.0).round().max(2.0)) }
}

/// Why `mode` can't run now (shown under the strip), if it can't.
fn blocked(app: &PhotocraftApp, mode: &str) -> Option<&'static str> {
    let d = app.session.active();
    match mode {
        "fill" if d.is_none_or(|d| d.doc.selection.is_none()) => Some(tl!("Fill needs a selection")),
        "edit" | "expand" if d.is_none() => Some(tl!("Open a document first")),
        _ => None,
    }
}

/// The command and params a Generate of the current state runs.
pub fn request(app: &PhotocraftApp) -> Result<(&'static str, Value), String> {
    let p = &app.ui.generate;
    let mode = p.mode.as_str();
    let prompt = p.prompt.trim();
    if prompt.is_empty() && mode != "expand" {
        return Err(tl!("Type what to generate first").to_string());
    }
    if let Some(why) = blocked(app, mode) {
        return Err(why.to_string());
    }
    let mut q = json!({"prompt": prompt});
    if let Some(t) = chosen_template(app, mode) {
        q["template"] = json!(t);
    }
    if let Some(e) = p.enhance {
        q["enhance"] = json!(e);
    }
    if p.show_negative && !p.negative.trim().is_empty() && mode != "expand" {
        q["negative"] = json!(p.negative.trim());
    }
    let count = p.count.clamp(1, 4);
    match mode {
        "image" => {
            let (w, h) = image_size(app);
            q["width"] = json!(w);
            q["height"] = json!(h);
            q["variations"] = json!(count);
            q["target"] = json!(if p.into == "layer" && app.session.active().is_some() { "layer" } else { "document" });
        }
        "fill" => {
            q["variations"] = json!(count);
            if p.grow > 0 {
                q["grow"] = json!(p.grow.min(photocraft_engine::generate_cmds::MAX_GROW));
            }
            if p.denoise < 1.0 && photocraft_engine::generate_cmds::takes_denoise(&effective_template(app, mode)) {
                q["denoise"] = json!((f64::from(p.denoise) * 100.0).round() / 100.0);
            }
        }
        "edit" => q["variations"] = json!(count),
        _ => {
            let e = p.expand;
            if e.iter().all(|v| *v == 0) {
                return Err(tl!("Give the expand at least one side").to_string());
            }
            q = json!({"prompt": prompt, "left": e[0], "top": e[1], "right": e[2], "bottom": e[3]});
            if let Some(t) = chosen_template(app, mode) {
                q["template"] = json!(t);
            }
        }
    }
    if matches!(mode, "image" | "edit" | "fill") && !p.references.is_empty() {
        q["references"] = Value::Array(p.references.clone());
    }
    let loras = chosen_loras(app, mode);
    if !loras.is_empty() {
        q["loras"] = Value::Array(loras);
    }
    Ok((command_of(mode), q))
}

/// Generate: run the command for the current state (a background job in the desktop app).
pub fn generate(app: &mut PhotocraftApp) -> Result<Value, String> {
    // The LoRA list for this template, also when the panel ran in its compact form (it loads
    // as the rows draw).
    let template = effective_template(app, &app.ui.generate.mode.clone());
    refresh_loras(app, &template);
    let (cmd, q) = request(app).inspect_err(|e| {
        app.ui.status = e.clone();
        app.ui.status_error = true;
    })?;
    match app.run(cmd, q) {
        Ok(v) => {
            app.ui.generate.job = v.get("pending").and_then(Value::as_bool).filter(|p| *p).and_then(|_| v.get("job").and_then(Value::as_u64));
            Ok(v)
        }
        Err(e) => {
            crate::notices::error(app, format!("{}: {e}", mode_label(&app.ui.generate.mode)));
            Err(e)
        }
    }
}

/// The wand: rewrite the prompt with the vision-language model (`generate.enhancePrompt`).
pub fn enhance(app: &mut PhotocraftApp) -> Result<Value, String> {
    let prompt = app.ui.generate.prompt.trim().to_string();
    if prompt.is_empty() || app.ui.generate.enhance_job.is_some() {
        return Err(tl!("Type a prompt first").to_string());
    }
    let task = match app.ui.generate.mode.as_str() {
        "fill" => "fill",
        "edit" => "edit",
        _ => "image",
    };
    match app.run("generate.enhancePrompt", json!({"prompt": prompt, "task": task})) {
        Ok(v) => {
            if v.get("pending").and_then(Value::as_bool) == Some(true) {
                app.ui.generate.enhance_job = v.get("job").and_then(Value::as_u64);
            } else {
                apply_enhanced(app, &v);
            }
            Ok(v)
        }
        Err(e) => {
            crate::notices::error(app, format!("Enhance Prompt: {e}"));
            Err(e)
        }
    }
}

fn apply_enhanced(app: &mut PhotocraftApp, v: &Value) {
    match (v["enhanced"].as_bool(), v["prompt"].as_str()) {
        (Some(true), Some(p)) if !p.trim().is_empty() => app.ui.generate.prompt = p.to_string(),
        _ => {
            app.ui.status = tl!("The model could not improve this prompt").to_string();
            app.ui.status_error = false;
        }
    }
}

/// A job ended: the panel's own run frees the button; the wand's answer replaces the prompt.
pub fn on_event(app: &mut PhotocraftApp, e: &JobEvent) {
    if app.ui.generate.job == Some(e.id.0) {
        app.ui.generate.job = None;
    }
    if app.ui.generate.enhance_job == Some(e.id.0) {
        app.ui.generate.enhance_job = None;
        match &e.outcome {
            JobOutcome::Done(v) => apply_enhanced(app, v),
            JobOutcome::Failed(msg) => crate::notices::error(app, format!("Enhance Prompt: {msg}")),
            JobOutcome::Cancelled => {}
        }
    }
}

/// Is the panel on screen (shown and not collapsed)? The task bar steps aside then (`auto`).
pub fn visible(app: &PhotocraftApp) -> bool {
    app.ui.panels.generate && !app.ui.dock.is_collapsed(crate::dock::Group::Generate) && app.ui.generate.drawn_frame.is_some_and(|f| app.frame <= f + 2)
}

/// Switch the panel to `mode` and put the keyboard in its prompt (Edit › Generative Fill… while
/// the panel is on screen, the bar's mode switch).
pub fn focus_mode(app: &mut PhotocraftApp, mode: &str) {
    if MODES.contains(&mode) {
        app.ui.generate.mode = mode.to_string();
        app.ui.generate.mode_chosen = true;
    }
    app.ui.generate.focus = true;
}

/// The panel shows the mode that fits the canvas: Image with no document, Fill over a selection,
/// Edit otherwise. A mode picked by hand holds until the canvas asks for another one, so a
/// selection made after picking Image still opens Fill. (On the Image page an instruction for
/// the open picture, "close her eyes", made a new picture instead of changing that one.)
fn context_mode(app: &mut PhotocraftApp) {
    let fits = match app.session.active() {
        None => "image",
        Some(d) if d.doc.selection.as_ref().is_some_and(|s| !s.content_bounds().is_empty()) => "fill",
        Some(_) => "edit",
    };
    let p = &mut app.ui.generate;
    let changed = p.context.is_some_and(|c| c != fits);
    p.context = Some(fits);
    if changed {
        p.mode_chosen = false;
    }
    if !p.mode_chosen {
        p.mode = fits.to_string();
    }
}

/// How many references `mode` takes now: the chosen template's limit; with Auto in image or
/// fill mode, the reference template's (the engine switches to it when references come).
pub fn refs_limit(app: &PhotocraftApp, mode: &str) -> u32 {
    let max_of = |id: &str| info(app, id).map_or(0, |m| m.refs_max);
    match mode {
        "edit" => max_of(&effective_template(app, "edit")),
        "image" => match chosen_template(app, "image") {
            Some(t) => max_of(&t),
            None => max_of(&effective_template(app, "image")).max(max_of(photocraft_engine::generate_cmds::image_ref_template(&app.session))),
        },
        "fill" => match chosen_template(app, "fill") {
            Some(t) => max_of(&t),
            None => {
                let q = photocraft_engine::generate_cmds::quality(&app.session);
                max_of(&effective_template(app, "fill")).max(max_of(photocraft_engine::generate_cmds::fill_ref_template(q)))
            }
        },
        _ => 0,
    }
}

/// Add a reference spec, within the template's limit (the status bar says when it is full).
pub fn add_reference(app: &mut PhotocraftApp, spec: Value) {
    let mode = app.ui.generate.mode.clone();
    let max = refs_limit(app, &mode);
    if app.ui.generate.references.len() >= max as usize {
        app.ui.status = crate::i18n::fmt(tl!("This model takes at most {n} reference images"), &[("n", &max.to_string())]);
        app.ui.status_error = true;
        return;
    }
    if !app.ui.generate.references.contains(&spec) {
        app.ui.generate.references.push(spec);
    }
}

/// Files dropped onto the panel: each one (with a path) becomes a reference.
pub fn add_dropped(app: &mut PhotocraftApp, files: Vec<egui::DroppedFileHandle>) {
    for f in files {
        let path = f.path();
        if path.is_absolute() {
            add_reference(app, json!({"path": path.to_string_lossy()}));
        } else {
            app.ui.status = tl!("Only files with a path can be references here").to_string();
            app.ui.status_error = true;
        }
    }
}

/// The dock group's body.
pub fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    load_catalog(app);
    context_mode(app);
    app.ui.generate.drawn_frame = Some(app.frame);
    let t = Tokens::get(ui.ctx());
    let mut action: Option<Action> = None;
    let compact = ui.available_height() < COMPACT_BELOW;
    app.ui.generate.compact = compact;
    let top = ui.cursor().min;
    if compact {
        compact_row(app, ui, &t, &mut action);
    } else {
        egui::ScrollArea::vertical().id_salt("generate-panel").auto_shrink([false, true]).show(ui, |ui| full(app, ui, &t, &mut action));
    }
    app.ui.generate.drop_rect = Some(Rect::from_min_max(top, ui.min_rect().max.max(top + vec2(ui.available_width(), 0.0))));
    editor(app, ui.ctx(), &mut action);
    confirm_clear(app, ui.ctx(), &mut action);
    if let Some(a) = action {
        apply(app, a);
    }
}

#[derive(Clone, Debug)]
enum Action {
    /// Preferences › AI Integrations, where the Models folder is set.
    OpenModelsFolder,
    Generate,
    Cancel(JobId),
    Enhance,
    Mode(String),
    AddRef(Value),
    RemoveRef(usize),
    MoveRef(usize, isize),
    PickFile,
    ClearAll,
    ClearAllNow,
    OpenEditor(bool),
}

fn apply(app: &mut PhotocraftApp, a: Action) {
    match a {
        Action::Generate => {
            let _ = generate(app);
        }
        Action::Cancel(j) => {
            crate::jobs_ui::cancel(app, j);
        }
        Action::Enhance => {
            let _ = enhance(app);
        }
        Action::Mode(m) => {
            app.ui.generate.mode = m.clone();
            app.ui.generate.mode_chosen = true;
            // The bar follows for the modes it has.
            if m == "fill" || m == "edit" {
                app.ui.generative_bar.mode = m;
            }
        }
        Action::AddRef(spec) => add_reference(app, spec),
        Action::RemoveRef(i) => {
            if i < app.ui.generate.references.len() {
                app.ui.generate.references.remove(i);
            }
        }
        Action::MoveRef(i, d) => {
            let refs = &mut app.ui.generate.references;
            let j = i as isize + d;
            if i < refs.len() && j >= 0 && (j as usize) < refs.len() {
                refs.swap(i, j as usize);
            }
        }
        Action::PickFile => {
            // The dialog answers on a later frame (`file_dialog`); on the desktop the name is
            // the file's path, which the worker reads itself.
            let r = app.pick_file_bytes(|app, name, _bytes| {
                add_reference(app, json!({"path": name}));
                Ok(Value::Null)
            });
            if let Err(e) = r
                && e != crate::file_dialog::CANCELLED
            {
                crate::notices::error(app, e);
            }
        }
        Action::ClearAll => {
            if app.ui.generate.references.is_empty() {
                app.ui.generate.prompt.clear();
            } else {
                app.ui.generate.confirm_clear = true;
            }
        }
        Action::ClearAllNow => {
            app.ui.generate.prompt.clear();
            app.ui.generate.negative.clear();
            app.ui.generate.references.clear();
            app.ui.generate.confirm_clear = false;
        }
        Action::OpenEditor(on) => app.ui.generate.editor = on,
        Action::OpenModelsFolder => {
            crate::prefs_ui::open_preferences(app, "integrations");
        }
    }
}

/// The running job the button follows: the panel's own, or a generative job on this document.
/// Is the panel showing job `id` (its Generate button became Cancel with the percent), so the
/// modal progress dialog is not needed?
pub fn shows_job(app: &PhotocraftApp, id: JobId) -> bool {
    app.ui.generate.job == Some(id.0) && visible(app)
}

fn running_job(app: &PhotocraftApp) -> Option<photocraft_engine::jobs::JobInfo> {
    app.ui.generate.job.map(JobId).and_then(|j| app.session.job(j))
}

fn generate_button(app: &PhotocraftApp, ui: &mut egui::Ui, action: &mut Option<Action>, width: f32) {
    if let Some(j) = running_job(app) {
        let progress = j.progress.clamp(0.0, 1.0);
        let label = format!("{} · {:.0}%", tl!("Cancel"), progress * 100.0);
        let r = widgets::secondary_button(ui, &label, width);
        if r.clicked() {
            *action = Some(Action::Cancel(j.id));
        }
        // The full form: a progress line under the button.
        if width > 0.0 {
            let t = Tokens::get(ui.ctx());
            let (bar, _) = ui.allocate_exact_size(vec2(r.rect.width(), 3.0), Sense::hover());
            ui.painter().rect_filled(bar, 1.5, t.field_border);
            let done = Rect::from_min_size(bar.min, vec2(bar.width() * progress, bar.height()));
            ui.painter().rect_filled(done, 1.5, t.accent);
        }
        return;
    }
    let mode = app.ui.generate.mode.clone();
    // A narrow one-line form keeps the prompt usable: the estimate moves to the tooltip.
    let narrow = width == 0.0 && ui.available_width() < 240.0;
    let eta = estimate(app, &mode).map(eta_label).filter(|s| !s.is_empty() && !narrow);
    let label = match eta {
        Some(e) => format!("{} · {e}", tl!("Generate")),
        None => tl!("Generate").to_string(),
    };
    let can = request(app).is_ok();
    let r = ui.add_enabled_ui(can, |ui| widgets::primary_button(ui, &label, width)).inner;
    if r.clicked() {
        *action = Some(Action::Generate);
    }
    if !can && let Err(why) = request(app) {
        r.on_disabled_hover_text(why);
    }
}

/// Enter (or Ctrl+Enter) in a focused prompt generates: Ctrl+Enter always, plain Enter when
/// `enter` (the one-line form always; the full form when the preference says so).
fn submitted(ui: &egui::Ui, app: &PhotocraftApp, field: egui::Id, enter: bool) -> bool {
    let focused = ui.ctx().memory(|m| m.has_focus(field));
    focused
        && request(app).is_ok()
        && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter) || (enter && i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)))
}

fn compact_row(app: &mut PhotocraftApp, ui: &mut egui::Ui, _t: &Tokens, action: &mut Option<Action>) {
    let field_id = egui::Id::unique("generate-prompt-compact");
    if submitted(ui, app, field_id, true) {
        *action = Some(Action::Generate);
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let mut mode = app.ui.generate.mode.clone();
        let options: Vec<(String, &str)> = MODES.iter().map(|m| (m.to_string(), mode_label(m))).collect();
        if widgets::dropdown(ui, "generate-mode-compact", tl!("Mode"), &mut mode, &options, 70.0) {
            *action = Some(Action::Mode(mode.clone()));
        }
        // The button at the right edge first; the prompt takes what is left.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            generate_button(app, ui, action, 0.0);
            model_pill(app, ui, &mode, 30.0);
            let w = ui.available_width().max(40.0);
            let field = egui::TextEdit::singleline(&mut app.ui.generate.prompt).hint_text(hint(&mode)).desired_width(w).id(field_id);
            let resp = ui.add(field);
            if std::mem::take(&mut app.ui.generate.focus) {
                resp.request_focus();
            }
        });
    });
}

/// Height of the panel's controls (segmented controls, the model pill).
const ROW_H: f32 = 26.0;
/// The settings' label column.
const LABEL_W: f32 = 64.0;
/// Space between the panel's sections.
const SECTION_GAP: f32 = 12.0;

/// The full form, top to bottom: the mode switch, what blocks the mode (a callout), the
/// references, the prompt composer (its own toolbar: Enhance, the large editor, the counter), the
/// negative prompt behind a disclosure, the settings as labelled rows, LoRAs, and one full-width
/// Generate with the estimate (Cancel and a progress line while it runs).
fn full(app: &mut PhotocraftApp, ui: &mut egui::Ui, t: &Tokens, action: &mut Option<Action>) {
    let mode = app.ui.generate.mode.clone();
    ui.spacing_mut().item_spacing.y = 6.0;
    // First run: no Models folder yet (nothing to migrate from an old ComfyUI install).
    if app.session.prefs().integrations.models_root().is_empty() {
        callout(ui, t, "folder-open", tl!("Choose the folder that holds your model weights to use the generative tools"), t.accent);
        let w = ui.available_width();
        if widgets::secondary_button(ui, tl!("Set Models Folder…"), w).clicked() {
            *action = Some(Action::OpenModelsFolder);
        }
        ui.add_space(SECTION_GAP);
    }
    mode_switch(ui, &mode, action);
    if let Some(why) = blocked(app, &mode) {
        callout(ui, t, "info", why, t.warning);
    }
    ui.add_space(SECTION_GAP);

    // References: their own section above the prompt, when the template takes them.
    let template = effective_template(app, &mode);
    let refs_max = refs_limit(app, &mode);
    let n_refs = app.ui.generate.references.len();
    let has_refs = matches!(mode.as_str(), "image" | "edit" | "fill") && n_refs > 0;
    if refs_max > 0 || has_refs {
        let color = if n_refs as u32 > refs_max { t.danger } else { t.text_faint };
        section(ui, t, tl!("References"), Some((format!("{n_refs} / {refs_max}"), color)));
        deck(app, ui, t, refs_max, action);
        ui.add_space(SECTION_GAP);
    }

    section(ui, t, tl!("Prompt"), None);
    composer(app, ui, t, &mode, action);
    if mode != "expand" {
        let open = app.ui.generate.show_negative;
        if disclosure(ui, t, tl!("Negative prompt"), open).clicked() {
            app.ui.generate.show_negative = !open;
        }
        if app.ui.generate.show_negative {
            ui.add(
                egui::TextEdit::singleline(&mut app.ui.generate.negative)
                    .hint_text(tl!("What to avoid…"))
                    .desired_width(f32::INFINITY)
                    .id_salt("generate-negative"),
            );
        }
    }
    ui.add_space(SECTION_GAP);

    section(ui, t, tl!("Settings"), None);
    settings(app, ui, t, &mode, &template);
    lora_rows(app, ui, &mode, &template);
    ui.add_space(SECTION_GAP);

    // The one primary action, full width; Clear all and the shortcut under it.
    let w = ui.available_width();
    generate_button(app, ui, action, w);
    ui.horizontal(|ui| {
        if ui.link(RichText::new(tl!("Clear all")).small().color(t.text_dim)).clicked() {
            *action = Some(Action::ClearAll);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let key = if cfg!(target_os = "macos") { "Cmd+Enter" } else { "Ctrl+Enter" };
            let hint = if app.session.prefs().integrations.enter_to_generate { tl!("Enter to generate") } else { key };
            ui.label(RichText::new(hint).small().color(t.text_faint));
        });
    });
}

/// One option of a [`segmented`] control.
struct Seg<'a, T> {
    value: T,
    label: &'a str,
    icon: Option<&'static str>,
    tip: Option<&'a str>,
}

impl<'a, T> Seg<'a, T> {
    fn new(value: T, label: &'a str) -> Self {
        Self { value, label, icon: None, tip: None }
    }
    fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }
    fn tip(mut self, tip: &'a str) -> Self {
        self.tip = Some(tip);
        self
    }
}

/// A segmented control `width` wide (0: the available width): equal segments in one track, the
/// chosen one raised. A segment too narrow for its icon and label shows the icon alone (the label
/// moves to the tooltip). Returns the option clicked, when it is not the current one.
fn segmented<T: PartialEq + Clone>(ui: &mut egui::Ui, current: &T, options: &[Seg<'_, T>], width: f32) -> Option<T> {
    let t = Tokens::get(ui.ctx());
    let width = if width > 0.0 { width } else { ui.available_width() };
    let (track, track_resp) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::hover());
    let r = t.radius_sm;
    ui.painter().rect_filled(track, r, t.field);
    ui.painter().rect_stroke(track, r, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
    let n = options.len().max(1) as f32;
    let seg_w = (track.width() - 4.0) / n;
    let enabled = ui.is_enabled();
    let mut picked = None;
    for (i, o) in options.iter().enumerate() {
        let rect = Rect::from_min_size(pos2(track.left() + 2.0 + seg_w * i as f32, track.top() + 2.0), vec2(seg_w, ROW_H - 4.0));
        let resp = ui.interact(rect, track_resp.id.with(i), Sense::click());
        let selected = o.value == *current;
        if selected {
            widgets::surface(ui, rect, t.hover, true);
            if !t.bevel {
                ui.painter().rect_stroke(rect, (r - 1.0).max(0.0), Stroke::new(1.0, t.field_border), StrokeKind::Inside);
            }
        } else if resp.hovered() && enabled {
            ui.painter().rect_filled(rect, (r - 1.0).max(0.0), t.hover.gamma_multiply(0.5));
        } else if i > 0 && options.get(i - 1).is_none_or(|p| p.value != *current) {
            // A divider between two unselected segments.
            let x = rect.left();
            ui.painter().line_segment([pos2(x, rect.top() + 5.0), pos2(x, rect.bottom() - 5.0)], Stroke::new(1.0, t.separator));
        }
        if resp.has_focus() {
            ui.painter().rect_stroke(rect, r, Stroke::new(2.0, t.accent), StrokeKind::Inside);
        }
        let color = if !enabled {
            t.text_faint
        } else if selected {
            t.text
        } else {
            t.text_dim
        };
        let font = crate::theme::medium(12.0);
        let galley = ui.painter().layout_no_wrap(o.label.to_string(), font, color);
        let icon_w = if o.icon.is_some() { 18.0 } else { 0.0 };
        let fits = icon_w + galley.size().x + 8.0 <= rect.width();
        let show_label = fits || o.icon.is_none();
        let content_w = if show_label { icon_w + galley.size().x } else { 14.0 };
        let mut x = rect.center().x - content_w / 2.0;
        if let Some(icon) = o.icon {
            let ir = Rect::from_min_size(pos2(x, rect.center().y - 7.0), vec2(14.0, 14.0));
            icons::paint(ui, ir, icon, 14.0, color);
            x += icon_w;
        }
        if show_label {
            ui.painter().with_clip_rect(rect.shrink(2.0)).galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, color);
        }
        let label = o.label.to_string();
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, enabled, selected, &label));
        let resp = match (o.tip, show_label) {
            (Some(tip), true) => resp.on_hover_text(tip),
            (Some(tip), false) => resp.on_hover_text(format!("{}\n{tip}", o.label)),
            (None, false) => resp.on_hover_text(o.label),
            (None, true) => resp,
        };
        if resp.clicked() && enabled && !selected {
            picked = Some(o.value.clone());
        }
    }
    picked
}

/// Image · Fill · Edit · Expand, with the Generate hub's icons.
fn mode_switch(ui: &mut egui::Ui, mode: &str, action: &mut Option<Action>) {
    let icon = |m: &str| match m {
        "fill" => "square-dashed",
        "edit" => "wand-sparkles",
        "expand" => "scaling",
        _ => "image",
    };
    let options: Vec<Seg<'_, String>> = MODES.iter().map(|m| Seg::new(m.to_string(), mode_label(m)).icon(icon(m))).collect();
    if let Some(m) = segmented(ui, &mode.to_string(), &options, 0.0) {
        *action = Some(Action::Mode(m));
    }
}

/// A section's title in small capitals, with an optional note at the right (a count).
fn section(ui: &mut egui::Ui, t: &Tokens, title: &str, right: Option<(String, egui::Color32)>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title.to_uppercase()).font(crate::theme::semibold(10.5)).color(t.text_faint));
        if let Some((note, color)) = right {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(note).small().color(color));
            });
        }
    });
}

/// A tinted note with an icon (what blocks the mode, the first-run Models folder).
fn callout(ui: &mut egui::Ui, t: &Tokens, icon: &str, text: &str, color: egui::Color32) {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.12))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.45)))
        .corner_radius(t.radius_sm)
        .inner_margin(egui::Margin { left: 8, right: 8, top: 6, bottom: 6 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (r, _) = ui.allocate_exact_size(vec2(14.0, 16.0), Sense::hover());
                icons::paint(ui, r, icon, 14.0, color);
                ui.add(egui::Label::new(RichText::new(text).small().color(t.text)).wrap());
            });
        });
}

/// A chevron and a label that open and close what follows (the negative prompt).
fn disclosure(ui: &mut egui::Ui, t: &Tokens, label: &str, open: bool) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(12.0), t.text_dim);
    let (rect, resp) = ui.allocate_exact_size(vec2(18.0 + galley.size().x, 20.0), Sense::click());
    let color = if resp.hovered() { t.text } else { t.text_dim };
    let chevron = Rect::from_min_size(pos2(rect.left(), rect.center().y - 7.0), vec2(14.0, 14.0));
    icons::paint(ui, chevron, if open { "chevron-down" } else { "chevron-right" }, 12.0, color);
    ui.painter().galley_with_override_text_color(pos2(rect.left() + 18.0, rect.center().y - galley.size().y / 2.0), galley, color);
    if resp.has_focus() {
        ui.painter().rect_stroke(rect.expand(2.0), t.radius_sm, Stroke::new(2.0, t.accent), StrokeKind::Outside);
    }
    let enabled = ui.is_enabled();
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, enabled, open, label));
    resp
}

/// The prompt in one framed box (the accent frame while it has the keyboard) with its toolbar
/// inside: Enhance, the large editor, and the counter. Grows from three to eight rows.
fn composer(app: &mut PhotocraftApp, ui: &mut egui::Ui, t: &Tokens, mode: &str, action: &mut Option<Action>) {
    let rows = app.ui.generate.prompt.lines().count().clamp(3, 8);
    let enter_generates = app.session.prefs().integrations.enter_to_generate;
    let field_id = egui::Id::unique("generate-prompt");
    let submit = submitted(ui, app, field_id, enter_generates);
    let focused = ui.ctx().memory(|m| m.has_focus(field_id));
    let border = if focused { t.accent } else { t.field_border };
    egui::Frame::new()
        .fill(t.field)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(t.radius_sm)
        .inner_margin(egui::Margin { left: 8, right: 4, top: 6, bottom: 2 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let field = egui::TextEdit::multiline(&mut app.ui.generate.prompt)
                .hint_text(hint(mode))
                .desired_rows(rows)
                .desired_width(f32::INFINITY)
                .char_limit(MAX_PROMPT * 2)
                .frame(egui::Frame::NONE)
                .id(field_id);
            let resp = ui.add(field);
            if std::mem::take(&mut app.ui.generate.focus) {
                resp.request_focus();
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let enhancing = app.ui.generate.enhance_job.is_some();
                let can = !enhancing && !app.ui.generate.prompt.trim().is_empty();
                let tip = if can || enhancing { tl!("Enhance prompt") } else { tl!("Type a prompt first") };
                if icons::button(ui, "wand-sparkles", 24.0, enhancing, tip).clicked() && can {
                    *action = Some(Action::Enhance);
                }
                if icons::button(ui, "maximize-2", 24.0, false, tl!("Open the prompt in a larger editor")).clicked() {
                    *action = Some(Action::OpenEditor(true));
                }
                if enhancing {
                    ui.label(RichText::new(tl!("Enhancing…")).small().color(t.accent));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(4.0);
                    let n = app.ui.generate.prompt.chars().count();
                    let color = if n > MAX_PROMPT { t.danger } else { t.text_faint };
                    ui.label(RichText::new(format!("{n} / {MAX_PROMPT}")).small().color(color));
                });
            });
        });
    if submit {
        *action = Some(Action::Generate);
    }
}

/// One settings row: the name in a fixed column (elided when a translation is longer), then
/// the control in the rest of the width.
fn setting_row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.allocate_ui_with_layout(vec2(LABEL_W, ROW_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_size(vec2(LABEL_W, ROW_H));
            widgets::elided_label(ui, label, LABEL_W);
        });
        add(ui);
    });
}

/// The model picker of the mode (Auto, then the page's templates with their estimates); the
/// one-row form draws it as the family glyph alone. Its popover opening asks the server which
/// templates it has, once.
fn model_pill(app: &mut PhotocraftApp, ui: &mut egui::Ui, mode: &str, width: f32) {
    let group = app.session.prefs().integrations.group_models_by_family;
    let mut current = app.ui.generate.template.get(mode).cloned().unwrap_or_default();
    let items = picker_items(app, mode, true);
    let auto = auto_name(app, mode);
    if crate::model_picker::pill(ui, "generate-panel", &items, &mut current, auto.as_deref(), group, width) {
        app.ui.generate.template.insert(mode.to_string(), current);
    }
    if crate::model_picker::is_open(ui.ctx(), "generate-panel") {
        probe_once(app);
    }
}

/// Preferences › AI Integrations › Quality beside the model: what Auto optimises for (Best = the
/// full-step base templates and no 4-bit weights; Balanced = the 8-step Lightning tier; Fast =
/// the 4-step tier where one exists). A chosen template is left alone.
fn quality_pick(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let q = photocraft_engine::generate_cmds::quality(&app.session).to_string();
    let options = [
        Seg::new("best".to_string(), tl!("Best")).tip(tl!("Auto takes the full-step model and keeps the 4-bit weights off")),
        Seg::new("balanced".to_string(), tl!("Balanced")).tip(tl!("Auto takes the 8-step Lightning tier when the server has it")),
        Seg::new("fast".to_string(), tl!("Fast")).tip(tl!("Auto takes the 4-step tier where there is one")),
    ];
    if let Some(q) = segmented(ui, &q, &options, 0.0) {
        let _ = app.run("prefs.set", json!({"path": "integrations.quality", "value": q}));
    }
}

/// The settings rows: model and quality, then what the mode takes (aspect and size, the count,
/// where the picture goes, the fill's widen and denoise, the expand's sides).
fn settings(app: &mut PhotocraftApp, ui: &mut egui::Ui, t: &Tokens, mode: &str, template: &str) {
    setting_row(ui, tl!("Model"), |ui| {
        let w = ui.available_width();
        model_pill(app, ui, mode, w);
    });
    setting_row(ui, tl!("Quality"), |ui| quality_pick(app, ui));
    let m = info(app, template).cloned().unwrap_or_default();
    if mode == "image" {
        setting_row(ui, tl!("Aspect"), |ui| {
            let mut aspect = app.ui.generate.aspect.clone();
            let aspects: Vec<(String, &str)> =
                m.aspects.iter().map(|a| (a.clone(), a.as_str())).chain(std::iter::once(("custom".to_string(), tl!("Custom")))).collect();
            let w = ui.available_width();
            if widgets::dropdown(ui, "generate-aspect", tl!("Aspect Ratio"), &mut aspect, &aspects, w) {
                app.ui.generate.aspect = aspect;
            }
        });
        setting_row(ui, tl!("Size"), |ui| {
            if app.ui.generate.aspect == "custom" {
                let (lo, hi) = (m.size_min.max(64) as f32, if m.size_max == 0 { 2048.0 } else { m.size_max as f32 });
                let fw = ((ui.available_width() - 20.0) / 2.0).clamp(52.0, 90.0);
                let mut w = app.ui.generate.width as f32;
                let mut h = app.ui.generate.height as f32;
                widgets::value_field(ui, &mut w, lo..=hi, "px", fw).on_hover_text(tl!("Width"));
                ui.label(RichText::new("×").color(t.text_dim));
                widgets::value_field(ui, &mut h, lo..=hi, "px", fw).on_hover_text(tl!("Height"));
                app.ui.generate.width = w.round() as u32;
                app.ui.generate.height = h.round() as u32;
            } else {
                let options: Vec<Seg<'_, String>> =
                    SIZES.iter().filter(|(_, v)| m.size_max == 0 || *v <= m.size_max).map(|(k, _)| Seg::new(k.to_string(), k)).collect();
                if let Some(s) = segmented(ui, &app.ui.generate.size, &options, 0.0) {
                    app.ui.generate.size = s;
                }
            }
        });
        // What it will make, on the template's grid.
        let (w, h) = image_size(app);
        setting_row(ui, "", |ui| {
            ui.label(RichText::new(format!("{w} × {h} px")).small().color(t.text_faint));
        });
    }
    if mode != "expand" {
        let max = m.counts_max.clamp(1, 4) as u8;
        let n = app.ui.generate.count.clamp(1, max);
        app.ui.generate.count = n;
        if max > 1 {
            setting_row(ui, tl!("Count"), |ui| {
                let options: Vec<Seg<'_, u8>> =
                    [(1u8, "1"), (2, "2"), (3, "3"), (4, "4")].into_iter().filter(|(c, _)| *c <= max).map(|(c, l)| Seg::new(c, l)).collect();
                if let Some(c) = segmented(ui, &n, &options, 0.0) {
                    app.ui.generate.count = c;
                }
            });
        }
    }
    if mode == "image" && app.session.active().is_some() {
        setting_row(ui, tl!("Output"), |ui| {
            let into = if app.ui.generate.into == "layer" { "layer".to_string() } else { "document".to_string() };
            let options = [
                Seg::new("document".to_string(), tl!("Document")).icon("file-plus").tip(tl!("A new document")),
                Seg::new("layer".to_string(), tl!("Layer")).icon("layers").tip(tl!("A new layer over the open picture")),
            ];
            if let Some(x) = segmented(ui, &into, &options, 0.0) {
                app.ui.generate.into = x;
            }
        });
    }
    if mode == "fill" {
        // Widen the selection first (the fill's `grow`): room for a bulkier replacement.
        setting_row(ui, tl!("Widen"), |ui| {
            let max = photocraft_engine::generate_cmds::MAX_GROW as f32;
            let mut g = app.ui.generate.grow as f32;
            widgets::value_field(ui, &mut g, 0.0..=max, "px", 72.0)
                .on_hover_text(tl!("Widen the selection by this many pixels before the fill, so a bulkier replacement has room"));
            app.ui.generate.grow = g.round().clamp(0.0, max) as u32;
        });
        // How much is redrawn, where the template's sampler takes it (the 2511 fills).
        if photocraft_engine::generate_cmds::takes_denoise(template) {
            setting_row(ui, tl!("Denoise"), |ui| {
                let mut d = app.ui.generate.denoise;
                widgets::value_field_prec(ui, &mut d, 0.05..=1.0, "", 72.0, 2)
                    .on_hover_text(tl!("How much of the selection is redrawn: 1 replaces it, about 0.9 keeps the old shape"));
                app.ui.generate.denoise = d.clamp(0.05, 1.0);
            });
        }
    }
    if mode == "expand" {
        expand_sides(app, ui, t);
    }
}

/// A side of the expand, in pixels.
fn side_field(ui: &mut egui::Ui, v: &mut u32, name: &str) {
    let mut f = *v as f32;
    widgets::value_field_named(ui, name, &mut f, 0.0..=4096.0, "px", 64.0).on_hover_text(name);
    *v = f.round() as u32;
}

/// The expand's four sides around a diagram of the result: the picture (grey) inside the grown
/// canvas (accent), to scale.
fn expand_sides(app: &mut PhotocraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let mut e = app.ui.generate.expand;
    ui.label(RichText::new(tl!("Pixels to add on each side")).small().color(t.text_faint));
    ui.vertical_centered(|ui| side_field(ui, &mut e[1], tl!("Top")));
    ui.horizontal(|ui| {
        let gap = ui.spacing().item_spacing.x;
        let box_w = (ui.available_width() - 2.0 * (64.0 + gap)).max(40.0);
        side_field(ui, &mut e[0], tl!("Left"));
        let (r, _) = ui.allocate_exact_size(vec2(box_w, 52.0), Sense::hover());
        let (dw, dh) = app.session.active().map_or((1.0, 1.0), |d| (d.doc.size.width.max(1) as f32, d.doc.size.height.max(1) as f32));
        let (nw, nh) = (dw + (e[0] + e[2]) as f32, dh + (e[1] + e[3]) as f32);
        let s = ((r.width() - 4.0) / nw).min((r.height() - 4.0) / nh).max(0.0);
        let outer = Rect::from_center_size(r.center(), vec2(nw * s, nh * s));
        let inner = Rect::from_min_size(outer.min + vec2(e[0] as f32 * s, e[1] as f32 * s), vec2(dw * s, dh * s));
        ui.painter().rect_filled(outer, 2.0, t.accent.gamma_multiply(0.22));
        ui.painter().rect_stroke(outer, 2.0, Stroke::new(1.0, t.accent), StrokeKind::Inside);
        ui.painter().rect_filled(inner, 1.0, t.text_faint);
        side_field(ui, &mut e[2], tl!("Right"));
    });
    ui.vertical_centered(|ui| side_field(ui, &mut e[3], tl!("Bottom")));
    app.ui.generate.expand = e;
}

/// The LoRA rows of the template's family (`generate.loras`), when the template runs on the
/// native engine: a switch and a strength per file, folded away under "LoRAs".
fn lora_rows(app: &mut PhotocraftApp, ui: &mut egui::Ui, mode: &str, template: &str) {
    if !matches!(mode, "image" | "fill" | "edit") {
        return;
    }
    refresh_loras(app, template);
    let Some((_, true, files)) = app.ui.generate.lora_list.clone() else { return };
    if files.is_empty() {
        return;
    }
    let on = files.iter().filter(|f| app.ui.generate.loras.get(*f).is_some_and(|(o, _)| *o)).count();
    let title = if on > 0 { format!("{} ({on})", tl!("LoRAs")) } else { tl!("LoRAs").to_string() };
    egui::CollapsingHeader::new(RichText::new(title).small()).id_salt("generate-loras").show(ui, |ui| {
        ui.label(RichText::new(tl!("Adapters in the models folder's loras subfolder, applied by the native engine")).small().weak());
        for f in &files {
            let entry = app.ui.generate.loras.entry(f.clone()).or_insert((false, 1.0));
            ui.horizontal(|ui| {
                ui.checkbox(&mut entry.0, f.strip_suffix(".safetensors").unwrap_or(f));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    widgets::value_field_prec(ui, &mut entry.1, 0.0..=1.5, "", 52.0, 2).on_hover_text(tl!("Strength"));
                });
            });
        }
    });
}

/// The reference deck: thumbnails with × and context menus, then an Add tile with the + menu.
fn deck(app: &mut PhotocraftApp, ui: &mut egui::Ui, t: &Tokens, max: u32, action: &mut Option<Action>) {
    let refs = app.ui.generate.references.clone();
    let body = |ui: &mut egui::Ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        for (i, spec) in refs.iter().enumerate() {
            let (rect, resp) = ui.allocate_exact_size(vec2(THUMB, THUMB), Sense::click());
            widgets::checker(ui.painter(), rect, 4.0);
            match reference_texture(app, ui.ctx(), spec) {
                Some(tex) => {
                    ui.painter().image(tex, rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
                }
                None => icons::paint(ui, rect, "image", 18.0, t.text_faint),
            }
            let over = i as u32 >= max;
            let stroke = if over {
                Stroke::new(1.5, t.danger)
            } else if resp.hovered() {
                Stroke::new(1.0, t.accent)
            } else {
                Stroke::new(1.0, t.separator)
            };
            ui.painter().rect_stroke(rect, t.radius_sm, stroke, StrokeKind::Inside);
            // The number the prompt can refer to ("image 2"), on a dark chip so it reads on any picture.
            let chip = Rect::from_min_size(rect.left_bottom() + vec2(2.0, -14.0), vec2(12.0, 12.0));
            ui.painter().rect_filled(chip, 3.0, egui::Color32::from_black_alpha(160));
            ui.painter().text(chip.center(), Align2::CENTER_CENTER, (i + 1).to_string(), FontId::proportional(9.0), egui::Color32::WHITE);
            if resp.hovered() {
                let x = Rect::from_min_size(pos2(rect.right() - 15.0, rect.top() + 2.0), vec2(13.0, 13.0));
                ui.painter().circle_filled(x.center(), 6.5, t.chrome.gamma_multiply(0.92));
                icons::paint(ui, x, "x", 9.0, t.text);
                if resp.clicked() && ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| x.contains(p)) {
                    *action = Some(Action::RemoveRef(i));
                }
            }
            let tip = describe_reference(app, spec);
            let tip = if over { format!("{tip}\n{}", tl!("More than this model takes")) } else { tip };
            resp.on_hover_text(tip).context_menu(|ui| {
                if ui.button(tl!("Move left")).clicked() {
                    *action = Some(Action::MoveRef(i, -1));
                }
                if ui.button(tl!("Move right")).clicked() {
                    *action = Some(Action::MoveRef(i, 1));
                }
                if ui.button(tl!("Remove")).clicked() {
                    *action = Some(Action::RemoveRef(i));
                }
            });
        }
        if (refs.len() as u32) < max.max(1) {
            // The Add tile: a dashed square with a plus, the size of a thumbnail.
            let (rect, plus) = ui.allocate_exact_size(vec2(THUMB, THUMB), Sense::click());
            if plus.hovered() {
                ui.painter().rect_filled(rect, t.radius_sm, t.hover);
            }
            let color = if plus.hovered() { t.text } else { t.text_dim };
            dashed_rect(ui.painter(), rect.shrink(0.5), Stroke::new(1.0, if plus.hovered() { t.accent } else { t.field_border }));
            icons::paint(ui, rect, "plus", 16.0, color);
            let tip = tl!("Add a reference image");
            plus.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
            let plus = plus.on_hover_text(tip);
            egui::Popup::menu(&plus).show(|ui| add_menu(app, ui, action));
        }
        if refs.is_empty() {
            ui.allocate_ui_with_layout(vec2(ui.available_width().max(60.0), THUMB), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(
                    egui::Label::new(RichText::new(tl!("Optional: a layer, a file or a past generation to guide the result")).small().color(t.text_faint))
                        .wrap(),
                );
            });
        }
    };
    // Empty, the hint sits beside the Add tile; with thumbnails the row wraps.
    if refs.is_empty() {
        ui.horizontal(body);
    } else {
        ui.horizontal_wrapped(body);
    }
}

/// A dashed outline (the Add tile).
fn dashed_rect(p: &egui::Painter, r: Rect, stroke: Stroke) {
    let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    for w in pts.windows(2) {
        p.extend(egui::Shape::dashed_line(&[w[0], w[1]], stroke, 4.0, 3.0));
    }
}

fn add_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui, action: &mut Option<Action>) {
    // The document's pixel layers, active one first.
    if let Some(d) = app.session.active() {
        let active = d.active_layer;
        if let Some(l) = active.and_then(|a| d.doc.layer(a)).filter(|l| l.surface().is_some())
            && ui.button(crate::i18n::fmt(tl!("Active layer ({name})"), &[("name", &l.name)])).clicked()
        {
            *action = Some(Action::AddRef(json!({"layer": l.id.0})));
        }
        let layers: Vec<(u64, String)> = d.doc.layers.iter().rev().filter(|l| l.surface().is_some()).map(|l| (l.id.0, l.name.clone())).collect();
        if !layers.is_empty() {
            ui.menu_button(tl!("Layer"), |ui| {
                for (id, name) in layers {
                    if ui.button(name).clicked() {
                        *action = Some(Action::AddRef(json!({"layer": id})));
                    }
                }
            });
        }
    }
    if app.services.file_dialog.is_some() && ui.button(tl!("File…")).clicked() {
        *action = Some(Action::PickFile);
    }
    let recent: Vec<(u64, String)> = app
        .session
        .generations
        .records()
        .iter()
        .rev()
        .filter(|r| r.result.as_ref().is_some_and(|x| !x.files.is_empty()))
        .take(12)
        .map(|r| (r.id, if r.prompt.is_empty() { r.label.clone() } else { r.prompt.chars().take(40).collect() }))
        .collect();
    if !recent.is_empty() {
        ui.menu_button(tl!("From Generations"), |ui| {
            for (id, label) in recent {
                if ui.button(label).clicked() {
                    *action = Some(Action::AddRef(json!({"job": id, "index": 0})));
                }
            }
        });
    }
}

fn describe_reference(app: &PhotocraftApp, spec: &Value) -> String {
    if let Some(id) = spec["layer"].as_u64() {
        let name = app
            .session
            .active()
            .and_then(|d| d.doc.layer(photocraft_doc::LayerId(id)))
            .map_or_else(|| tl!("a layer that is gone").to_string(), |l| l.name.clone());
        return crate::i18n::fmt(tl!("Layer: {name}"), &[("name", &name)]);
    }
    if let Some(p) = spec["path"].as_str() {
        return crate::i18n::fmt(tl!("File: {name}"), &[("name", &crate::file_open::display_name(p))]);
    }
    if let Some(id) = spec["job"].as_u64() {
        let label = app.session.generations.get(id).map_or_else(String::new, |r| r.prompt.clone());
        return crate::i18n::fmt(tl!("Generation: {name}"), &[("name", &label)]);
    }
    String::new()
}

/// A thumbnail for a reference: the layer's (from the layer cache), a generation's (from the
/// Generations panel's cache); files show an icon.
fn reference_texture(app: &mut PhotocraftApp, ctx: &egui::Context, spec: &Value) -> Option<egui::TextureId> {
    if let Some(id) = spec["layer"].as_u64() {
        let doc = app.session.active()?.doc.clone();
        let layer = doc.layer(photocraft_doc::LayerId(id))?.clone();
        return Some(app.layer_thumb(ctx, &doc, &layer).0);
    }
    if let Some(id) = spec["job"].as_u64() {
        return crate::generations_panel::thumbnail(app, ctx, id).map(|t| t.id());
    }
    None
}

/// The large prompt editor (⤢).
fn editor(app: &mut PhotocraftApp, ctx: &egui::Context, action: &mut Option<Action>) {
    if !app.ui.generate.editor {
        return;
    }
    let mode = app.ui.generate.mode.clone();
    egui::Modal::new(egui::Id::unique("generate-prompt-editor")).show(ctx, |ui| {
        ui.set_width(560.0);
        widgets::section_label(ui, mode_label(&mode));
        ui.add(egui::TextEdit::multiline(&mut app.ui.generate.prompt).hint_text(hint(&mode)).desired_rows(14).desired_width(f32::INFINITY));
        ui.add_space(8.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::primary_button(ui, tl!("Done"), 80.0).clicked() {
                *action = Some(Action::OpenEditor(false));
            }
        });
    });
}

fn confirm_clear(app: &mut PhotocraftApp, ctx: &egui::Context, action: &mut Option<Action>) {
    if !app.ui.generate.confirm_clear {
        return;
    }
    let mut answer = None;
    egui::Modal::new(egui::Id::unique("generate-clear")).show(ctx, |ui| {
        ui.set_max_width(320.0);
        ui.label(tl!("Clear the prompt and remove the reference images?"));
        ui.add_space(10.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            answer = widgets::dialog_buttons(
                ui,
                &[
                    widgets::DialogButton::new(widgets::ButtonRole::Default, tl!("Clear"), 80.0),
                    widgets::DialogButton::new(widgets::ButtonRole::Cancel, tl!("Cancel"), 80.0),
                ],
            );
        });
    });
    match answer {
        Some(widgets::ButtonRole::Default) => *action = Some(Action::ClearAllNow),
        Some(_) => app.ui.generate.confirm_clear = false,
        None => {}
    }
}

/// `ui.inspect.generatePanel`.
pub fn inspect(app: &PhotocraftApp) -> Value {
    let p = &app.ui.generate;
    let mode = p.mode.as_str();
    let template = effective_template(app, mode);
    let request = request(app);
    json!({
        "mode": p.mode, "into": p.into, "prompt": p.prompt, "negative": p.negative, "references": p.references,
        "template": p.template, "effectiveTemplate": template, "aspect": p.aspect, "size": p.size,
        "width": p.width, "height": p.height, "imageSize": image_size(app), "count": p.count, "enhance": p.enhance,
        "expand": p.expand, "grow": p.grow, "denoise": p.denoise, "job": p.job, "compact": p.compact, "eta": estimate(app, mode), "etaLabel": estimate(app, mode).map(eta_label),
        "command": request.as_ref().ok().map(|(c, _)| *c), "params": request.as_ref().ok().map(|(_, q)| q.clone()),
        "blocked": request.err(), "referencesMax": refs_limit(app, mode),
        "loras": p.lora_list.as_ref().filter(|(t, _, _)| *t == template).map(|(_, native, files)| json!({
            "native": native,
            "files": files,
            "on": chosen_loras(app, mode),
        })),
    })
}

/// `ui.set {generatePanel: {mode, into, prompt, negative, template, aspect, size, width,
/// height, count, enhance, references, expand, grow, focus, loras, denoise}}`: each field validated before any
/// is applied; `focus: true` puts the keyboard in the prompt (then `ui.key` Enter with
/// `command` generates through the panel).
pub fn set(app: &mut PhotocraftApp, p: &Value) -> Result<(), String> {
    let Some(v) = p.get("generatePanel") else { return Ok(()) };
    let o = v.as_object().ok_or("generatePanel must be an object")?;
    let known = [
        "mode",
        "into",
        "prompt",
        "negative",
        "template",
        "aspect",
        "size",
        "width",
        "height",
        "count",
        "enhance",
        "references",
        "expand",
        "grow",
        "focus",
        "loras",
        "denoise",
    ];
    if let Some(k) = o.keys().find(|k| !known.contains(&k.as_str())) {
        return Err(format!("generatePanel has no field `{k}`"));
    }
    load_catalog(app);
    let mut next = app.ui.generate.clone();
    if let Some(m) = o.get("mode") {
        let m = m.as_str().filter(|m| MODES.contains(m)).ok_or("generatePanel.mode must be image, fill, edit or expand")?;
        next.mode = m.to_string();
        next.mode_chosen = true;
    }
    if let Some(x) = o.get("into") {
        next.into = x.as_str().filter(|x| *x == "document" || *x == "layer").ok_or("generatePanel.into must be document or layer")?.to_string();
    }
    for (key, slot) in [("prompt", &mut next.prompt), ("negative", &mut next.negative)] {
        if let Some(x) = o.get(key) {
            let s = x.as_str().ok_or(format!("generatePanel.{key} must be a string"))?;
            if s.chars().count() > MAX_PROMPT {
                return Err(format!("generatePanel.{key} is longer than {MAX_PROMPT} characters"));
            }
            *slot = s.to_string();
        }
    }
    if let Some(x) = o.get("template") {
        let s = x.as_str().ok_or("generatePanel.template must be a template id or empty for Auto")?;
        if !s.is_empty() && !app.ui.generate.catalog.iter().any(|m| m.id == s && m.pages.contains(&next.mode)) {
            return Err(format!("`{s}` is not a {} template (see generate.models)", next.mode));
        }
        next.template.insert(next.mode.clone(), s.to_string());
    }
    if let Some(x) = o.get("aspect") {
        let s = x.as_str().ok_or("generatePanel.aspect must be like \"3:2\" or \"custom\"")?;
        let ok = s == "custom" || s.split_once(':').is_some_and(|(a, b)| a.parse::<u32>().is_ok_and(|n| n > 0) && b.parse::<u32>().is_ok_and(|n| n > 0));
        if !ok {
            return Err("generatePanel.aspect must be like \"3:2\" or \"custom\"".into());
        }
        next.aspect = s.to_string();
    }
    if let Some(x) = o.get("size") {
        next.size = x.as_str().filter(|s| SIZES.iter().any(|(k, _)| k == s)).ok_or("generatePanel.size must be 1K, 1.5K or 2K")?.to_string();
    }
    for (key, slot) in [("width", &mut next.width), ("height", &mut next.height)] {
        if let Some(x) = o.get(key) {
            *slot = x.as_u64().filter(|n| (64..=4096).contains(n)).ok_or(format!("generatePanel.{key} must be 64..4096"))? as u32;
        }
    }
    if let Some(x) = o.get("count") {
        next.count = x.as_u64().filter(|n| (1..=4).contains(n)).ok_or("generatePanel.count must be 1 to 4")? as u8;
    }
    if let Some(x) = o.get("grow") {
        let max = u64::from(photocraft_engine::generate_cmds::MAX_GROW);
        next.grow = x.as_u64().filter(|n| *n <= max).ok_or(format!("generatePanel.grow must be 0 to {max} pixels"))? as u32;
    }
    if let Some(x) = o.get("enhance") {
        next.enhance = match x {
            Value::Null => None,
            Value::Bool(b) => Some(*b),
            _ => return Err("generatePanel.enhance must be true, false or null (the preference)".into()),
        };
    }
    if let Some(x) = o.get("expand") {
        let a = x.as_array().filter(|a| a.len() == 4).ok_or("generatePanel.expand must be [left, top, right, bottom]")?;
        let mut e = [0u32; 4];
        for (slot, v) in e.iter_mut().zip(a) {
            *slot = v.as_u64().filter(|n| *n <= 16_384).ok_or("generatePanel.expand values must be 0..16384")? as u32;
        }
        next.expand = e;
    }
    if let Some(x) = o.get("references") {
        let a = x.as_array().ok_or("generatePanel.references must be a list")?;
        if a.len() > 9 {
            return Err("generatePanel.references takes at most 9".into());
        }
        for r in a {
            let ok = r.as_object().is_some_and(|m| m.len() <= 2 && (r["layer"].is_u64() || r["path"].is_string() || r["job"].is_u64()));
            if !ok {
                return Err(format!("not a reference: {r} (use {{\"layer\": id}}, {{\"path\": \"…\"}} or {{\"job\": id, \"index\": i}})"));
            }
        }
        next.references = a.clone();
    }
    if let Some(x) = o.get("focus") {
        next.focus = x.as_bool().ok_or("generatePanel.focus must be true or false")?;
    }
    if let Some(x) = o.get("denoise") {
        next.denoise = x.as_f64().filter(|d| (0.05..=1.0).contains(d)).ok_or("generatePanel.denoise must be 0.05 to 1")? as f32;
    }
    if let Some(x) = o.get("loras") {
        // `{"file.safetensors": strength}`: those switched on at that strength, the rest off.
        let m = x.as_object().ok_or("generatePanel.loras must be {\"file.safetensors\": strength}")?;
        for entry in next.loras.values_mut() {
            entry.0 = false;
        }
        for (file, s) in m {
            let s = s.as_f64().filter(|s| (0.0..=1.5).contains(s)).ok_or(format!("generatePanel.loras.{file} must be a strength from 0 to 1.5"))?;
            next.loras.insert(file.clone(), (true, s as f32));
        }
    }
    if next.mode != app.ui.generate.mode && (next.mode == "fill" || next.mode == "edit") {
        app.ui.generative_bar.mode = next.mode.clone();
    }
    app.ui.generate = next;
    let template = effective_template(app, &app.ui.generate.mode.clone());
    refresh_loras(app, &template);
    Ok(())
}

#[cfg(test)]
#[path = "generate_panel_tests.rs"]
mod tests;
