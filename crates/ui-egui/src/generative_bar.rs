//! The generative task bar (comfy-photocraft): Photoshop's contextual bar under a selection,
//! reduced to what the fork does today. A prompt, the template picker, the number of variations,
//! Generate; progress with Cancel while the job runs. The variations of a run are compared in the
//! Variations strip at the canvas edge ([`crate::variations_strip`]).
//!
//! The bar is data in [`crate::state::UiState`] (`ui.inspect` shows it, `ui.set` drives it) and
//! it only ever runs engine commands (`generate.fill`, `generate.edit`), so the work, the
//! undo history and the journal are the ones a menu would produce. Edit › Generative Fill… and the
//! selection context menu open it (the generated dialog stays for a hidden bar); it also appears
//! by itself when a selection is made, as Preferences › AI Integrations › Show generative bar says:
//! `auto` (while the Generate panel is not on screen), `always` or `never`.
//!
//! ShotoPhopEX: the bar is the Generate panel's compact form. Its prompt, template (per mode) and
//! count are the panel's ([`crate::generate_panel::GeneratePanel`]), so the two never disagree.

use egui::{Rect, RichText, Sense, Stroke, pos2, vec2};
use photocraft_engine::jobs::{JobEvent, JobId, JobOutcome};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;

pub const COMMAND: &str = "generate.fill";
/// The bar's other mode: the whole picture by instruction, masked to the selection.
pub const EDIT_COMMAND: &str = "generate.edit";
const ENHANCE_COMMAND: &str = "generate.enhancePrompt";
const MODE_FILL: &str = "fill";
const MODE_EDIT: &str = "edit";
const AREA_ID: &str = "generative-bar";
/// Gap between the selection and the bar, and the bar's distance from the canvas edges.
const GAP: f32 = 12.0;
const EDGE: f32 = 8.0;

/// One template the picker offers (from `generate.models`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TemplateChoice {
    pub id: String,
    pub name: String,
    /// Research-licence templates are listed but disabled until the preference allows them.
    pub allowed: bool,
    /// Whether the server has every model file the template needs (None until the background
    /// probe answers, or when no server answers); the picker marks the missing ones.
    #[serde(default)]
    pub installed: Option<bool>,
    /// `fill` or `edit`: which mode of the bar lists it.
    #[serde(default)]
    pub task: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GenerativeBar {
    /// Shown under the selection (× hides it until the next selection is made).
    pub open: bool,
    /// `fill` (regenerate the selection from the prompt) or `edit` (change the whole picture
    /// by instruction, shown inside the selection).
    pub mode: String,
    /// The `generate.fill` job the bar started and shows progress for.
    pub job: Option<u64>,
    /// Templates for the picker, fetched when the bar opens.
    pub templates: Vec<TemplateChoice>,
    /// The prompt field takes the keyboard focus on the next frame (opened from a menu).
    #[serde(skip)]
    pub focus: bool,
    /// Whether the active document had a selection last frame (the bar opens when one appears).
    #[serde(skip)]
    had_selection: bool,
    /// The job whose progress the bar drew this frame (the modal progress dialog stays away).
    #[serde(skip)]
    showing_job: Option<u64>,
    /// The background `generate.models` probe filling in [`TemplateChoice::installed`].
    #[serde(skip)]
    probe_job: Option<u64>,
    /// The `generate.enhancePrompt` job the Enhance button started; its answer replaces the prompt.
    #[serde(skip)]
    enhance_job: Option<u64>,
}

impl Default for GenerativeBar {
    fn default() -> Self {
        Self {
            open: false,
            mode: MODE_FILL.to_string(),
            job: None,
            templates: Vec::new(),
            focus: false,
            had_selection: false,
            showing_job: None,
            probe_job: None,
            enhance_job: None,
        }
    }
}

/// Is the bar drawing job `id`'s progress (so the modal progress dialog is not needed)? Its
/// Enhance job too: the bar says "Enhancing…" by the prompt while it runs.
pub fn shows_job(app: &PhotocraftApp, id: JobId) -> bool {
    app.ui.generative_bar.showing_job == Some(id.0) || app.ui.generative_bar.enhance_job == Some(id.0)
}

/// Edit › Generative Fill… (and the selection context menu) with no params: open the bar instead
/// of the generated dialog. With params, or with the bar turned off in Preferences, the usual
/// path runs (the dialog from a menu, the engine from automation).
pub fn menu(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let pref = app.session.prefs().integrations.show_generative_bar.clone();
    if id != COMMAND || !params.as_object().is_none_or(|o| o.is_empty()) || pref == "never" {
        return None;
    }
    // The Generate panel on screen takes the request in its Fill mode instead.
    if pref == "auto" && crate::generate_panel::visible(app) {
        if let Some(Err(why)) = photocraft_engine::commands::find(COMMAND).map(|c| (c.enabled)(&app.session)) {
            return Some(Err(why));
        }
        crate::generate_panel::focus_mode(app, MODE_FILL);
        app.ui.generative_bar.mode = MODE_FILL.to_string();
        return Some(Ok(json!({"generatePanel": true})));
    }
    Some(open(app))
}

/// Whether the bar shows by itself when a selection appears (the preference; `auto` = while
/// the Generate panel is not on screen).
fn wanted(app: &PhotocraftApp) -> bool {
    match app.session.prefs().integrations.show_generative_bar.as_str() {
        "never" => false,
        "always" => true,
        _ => !crate::generate_panel::visible(app),
    }
}

/// Open the bar under the current selection with the keyboard in its prompt field.
pub fn open(app: &mut PhotocraftApp) -> Result<Value, String> {
    if let Some(Err(why)) = photocraft_engine::commands::find(COMMAND).map(|c| (c.enabled)(&app.session)) {
        return Err(why);
    }
    load_templates(app);
    let bar = &mut app.ui.generative_bar;
    bar.open = true;
    bar.focus = true;
    bar.had_selection = true;
    Ok(json!({"generativeBar": true}))
}

/// The fill templates, without contacting the server (this runs on the UI thread).
fn load_templates(app: &mut PhotocraftApp) {
    let Ok(v) = app.session.execute("generate.models", json!({"probe": false})) else { return };
    let templates = v["templates"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|t| t["task"] == MODE_FILL || t["task"] == MODE_EDIT)
                .filter_map(|t| {
                    Some(TemplateChoice {
                        id: t["id"].as_str()?.to_string(),
                        name: t["name"].as_str().unwrap_or_default().to_string(),
                        allowed: t["allowed"].as_bool().unwrap_or(false),
                        installed: None,
                        task: t["task"].as_str().unwrap_or_default().to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    app.ui.generative_bar.templates = templates;
    // The picker's rows come from the Generate panel's catalogue (offline); opening the bar asks
    // the server which of them it has.
    crate::generate_panel::load_catalog(app);
    probe_installed(app);
}

/// Ask the server which templates it has the files for: on a worker (the desktop app) so the
/// bar and the panel never wait for the server; inline (tests, web) the answer lands at once.
/// No server configured: the badges stay unknown. The answer marks both the bar's list and the
/// Generate panel's catalogue (the model picker's `not installed`).
pub(crate) fn probe_installed(app: &mut PhotocraftApp) {
    if app.ui.generative_bar.probe_job.is_none() && app.session.prefs().integrations.native_configured() {
        app.ui.generate.probed = true;
        match app.run("generate.models", json!({"probe": true, "async": true})) {
            Ok(v) if v.get("pending").and_then(Value::as_bool) == Some(true) => app.ui.generative_bar.probe_job = v.get("job").and_then(Value::as_u64),
            Ok(v) => apply_installed(app, &v),
            Err(_) => {}
        }
    }
}

/// Mark each picker template installed or not from a `generate.models` answer (a template is
/// installed when every one of its model files is; unknown stays unknown).
fn apply_installed(app: &mut PhotocraftApp, v: &Value) {
    let Some(list) = v["templates"].as_array() else { return };
    for t in list {
        let Some(id) = t["id"].as_str() else { continue };
        let slots: Vec<Option<bool>> = t["models"].as_array().map(|a| a.iter().map(|m| m["installed"].as_bool()).collect()).unwrap_or_default();
        let installed = if slots.iter().any(Option::is_none) { None } else { Some(slots.iter().all(|s| *s == Some(true))) };
        if let Some(c) = app.ui.generative_bar.templates.iter_mut().find(|c| c.id == id) {
            c.installed = installed;
        }
        if let Some(m) = app.ui.generate.catalog.iter_mut().find(|m| m.id == id) {
            m.installed = installed;
        }
    }
}

/// The template id the next run uses: the bar's choice or the preference's default (`auto` =
/// the engine picks the fastest permissive tier the server has).
fn current_template(app: &PhotocraftApp) -> String {
    let bar = &app.ui.generative_bar;
    let chosen = app.ui.generate.template.get(&bar.mode).map_or("", |s| s.trim());
    // A chosen template counts only for the mode it belongs to.
    if !chosen.is_empty() && bar.templates.iter().any(|c| c.id == chosen && c.task == bar.mode) {
        return chosen.to_string();
    }
    if bar.mode == MODE_EDIT {
        return photocraft_engine::generate_cmds::AUTO_TEMPLATE.to_string();
    }
    let pref = app.session.prefs().integrations.default_fill_template.trim();
    if pref.is_empty() { photocraft_engine::generate_cmds::AUTO_TEMPLATE.to_string() } else { pref.to_string() }
}

/// The command the bar's mode runs.
fn mode_command(app: &PhotocraftApp) -> &'static str {
    if app.ui.generative_bar.mode == MODE_EDIT { EDIT_COMMAND } else { COMMAND }
}

/// Does a Fill prompt read like an instruction to change what is already there ("make this
/// hyper realistic", "turn it into a painting")? Fill regenerates the selection from scratch,
/// so such prompts belong to Edit; the bar says so.
pub fn looks_like_an_edit(prompt: &str) -> bool {
    let lower = prompt.trim().to_ascii_lowercase();
    let mut words = lower.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty());
    let Some(first) = words.next() else { return false };
    let verbs = ["make", "turn", "convert", "transform", "change", "redraw", "repaint", "restyle", "stylize", "stylise", "render"];
    verbs.contains(&first) && words.any(|w| w == "this" || w == "it" || w == "these" || w == "them")
}

/// Run the bar's command (`generate.fill`, or `generate.edit` in Edit mode) with its prompt,
/// template and variations. In the desktop app it starts a background job the bar then
/// follows; inline (tests, web) the results land at once. Either way the run's variations show
/// in the Variations strip.
pub fn generate(app: &mut PhotocraftApp) -> Result<Value, String> {
    let prompt = app.ui.generate.prompt.trim().to_string();
    if prompt.is_empty() {
        return Err("type what to generate first".into());
    }
    let variations = app.ui.generate.count.clamp(1, 4);
    let template = current_template(app);
    let command = mode_command(app);
    let params = json!({"prompt": prompt, "variations": variations, "template": template});
    match app.run(command, params) {
        Ok(v) => {
            if v.get("pending").and_then(Value::as_bool) == Some(true) {
                app.ui.generative_bar.job = v.get("job").and_then(Value::as_u64);
            }
            Ok(v)
        }
        Err(e) => {
            crate::notices::error(app, format!("{}: {e}", if command == EDIT_COMMAND { "Generative Edit" } else { "Generative Fill" }));
            Err(e)
        }
    }
}

/// The Enhance button: `generate.enhancePrompt` rewrites the bar's prompt with the
/// vision-language model, the picture (and in Fill mode the selection's place) in view, and
/// the answer replaces the prompt for the user to read and run. A background job in the desktop
/// app; inline (tests, web) the answer lands at once.
pub fn enhance(app: &mut PhotocraftApp) -> Result<Value, String> {
    let prompt = app.ui.generate.prompt.trim().to_string();
    if prompt.is_empty() {
        return Err("type a prompt first".into());
    }
    if app.ui.generative_bar.enhance_job.is_some() {
        return Err("a prompt is being enhanced already".into());
    }
    let task = if app.ui.generative_bar.mode == MODE_EDIT { "edit" } else { "fill" };
    match app.run(ENHANCE_COMMAND, json!({"prompt": prompt, "task": task})) {
        Ok(v) => {
            if v.get("pending").and_then(Value::as_bool) == Some(true) {
                app.ui.generative_bar.enhance_job = v.get("job").and_then(Value::as_u64);
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

/// A `generate.enhancePrompt` answer: the rewritten prompt replaces the bar's, or the status
/// bar says the model had nothing better.
fn apply_enhanced(app: &mut PhotocraftApp, v: &Value) {
    match (v["enhanced"].as_bool(), v["prompt"].as_str()) {
        (Some(true), Some(p)) if !p.trim().is_empty() => app.ui.generate.prompt = p.to_string(),
        _ => {
            app.ui.status = tl!("The model could not improve this prompt").to_string();
            app.ui.status_error = false;
        }
    }
}

/// A background job ended: the bar's probe, its Enhance answer, or its own run (whose results the
/// Variations strip shows).
pub fn on_event(app: &mut PhotocraftApp, e: &JobEvent) {
    if app.ui.generative_bar.probe_job == Some(e.id.0) {
        app.ui.generative_bar.probe_job = None;
        if let JobOutcome::Done(v) = &e.outcome {
            apply_installed(app, v);
        }
        return;
    }
    if app.ui.generative_bar.enhance_job == Some(e.id.0) {
        app.ui.generative_bar.enhance_job = None;
        match &e.outcome {
            JobOutcome::Done(v) => apply_enhanced(app, v),
            JobOutcome::Failed(msg) => crate::notices::error(app, format!("Enhance Prompt: {msg}")),
            JobOutcome::Cancelled => {}
        }
        return;
    }
    if app.ui.generative_bar.job == Some(e.id.0) {
        app.ui.generative_bar.job = None;
    }
}

/// `ui.inspect.generativeBar`: the bar's own state plus the prompt, template and variations it
/// shares with the Generate panel (the fields scripts read before the panel existed).
pub fn inspect(app: &PhotocraftApp) -> Value {
    let mut v = serde_json::to_value(&app.ui.generative_bar).unwrap_or_else(|_| json!({}));
    if let Some(o) = v.as_object_mut() {
        let mode = app.ui.generative_bar.mode.clone();
        o.insert("prompt".into(), json!(app.ui.generate.prompt));
        o.insert("template".into(), json!(app.ui.generate.template.get(&mode).cloned().unwrap_or_default()));
        o.insert("variations".into(), json!(app.ui.generate.count));
    }
    v
}

/// `ui.set` fields: `generativeBar` (open), `generativeMode` (`fill` | `edit`),
/// `generativePrompt`, `generativeTemplate`, `generativeVariations` (1..=4).
pub fn set(app: &mut PhotocraftApp, p: &Value) -> Result<(), String> {
    // The prompt, template and count are the Generate panel's (aliases kept for scripts).
    if let Some(v) = p.get("generativeMode") {
        match v.as_str() {
            Some(m) if m == MODE_FILL || m == MODE_EDIT => {
                app.ui.generative_bar.mode = m.to_string();
                app.ui.generate.mode = m.to_string();
                app.ui.generate.mode_chosen = true;
            }
            _ => return Err("generativeMode must be \"fill\" or \"edit\"".into()),
        }
    }
    if let Some(v) = p.get("generativeVariations") {
        match v.as_u64() {
            Some(n) if (1..=4).contains(&n) => app.ui.generate.count = n as u8,
            _ => return Err("generativeVariations must be 1 to 4".into()),
        }
    }
    if let Some(v) = p.get("generativePrompt") {
        let s = v.as_str().ok_or("generativePrompt must be a string")?;
        if s.chars().count() > 4000 {
            return Err("generativePrompt is longer than 4000 characters".into());
        }
        app.ui.generate.prompt = s.to_string();
    }
    if let Some(v) = p.get("generativeTemplate") {
        let s = v.as_str().ok_or("generativeTemplate must be a template id (see generate.models) or empty for the default")?;
        if s.len() > 200 {
            return Err("generativeTemplate is too long".into());
        }
        let mode = app.ui.generative_bar.mode.clone();
        app.ui.generate.template.insert(mode, s.to_string());
    }
    if let Some(v) = p.get("generativeBar") {
        match v.as_bool() {
            Some(true) => {
                open(app)?;
            }
            Some(false) => app.ui.generative_bar.open = false,
            None => return Err("generativeBar must be true or false".into()),
        }
    }
    Ok(())
}

/// Per frame: open the bar when a selection appears, close it when the selection goes, draw it.
pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.session.active_index().is_none() {
        app.ui.generative_bar.had_selection = false;
        return;
    }
    let sel_bounds = app.session.active().and_then(|d| d.doc.selection.as_ref()).map(|s| s.content_bounds()).filter(|b| !b.is_empty());
    let has_selection = sel_bounds.is_some();
    let wanted = wanted(app);
    app.ui.generative_bar.showing_job = None;
    {
        let bar = &mut app.ui.generative_bar;
        if has_selection && !bar.had_selection && wanted {
            bar.open = true;
        }
        if !has_selection {
            bar.open = false;
            bar.focus = false;
        }
        bar.had_selection = has_selection;
    }
    let Some(bounds) = sel_bounds else { return };
    let busy = app.ui.transform.is_some() || app.ui.text_edit.is_some() || app.jobs.focus.is_some() || !app.ui.dialogs.is_empty();
    if !app.ui.generative_bar.open || busy {
        return;
    }
    // Opened by the selection itself: the picker still needs its template list (offline, cheap).
    if app.ui.generative_bar.templates.is_empty() {
        load_templates(app);
    }
    let Some(xf) = crate::canvas::ViewXform::active(app) else { return };
    let canvas = crate::rulers::content_rect(app, app.last_canvas_rect);
    if canvas.width() < 200.0 || canvas.height() < 80.0 {
        return;
    }
    let sel = xf.doc_rect(bounds);
    let id = egui::Id::unique(AREA_ID);
    let size = ctx.memory(|m| m.area_rect(id)).map_or(vec2(560.0, 44.0), |r| r.size());
    let pos = place(sel, size, canvas);
    let t = Tokens::get(ctx);
    let mut action: Option<Action> = None;
    // The bar's own job, or any fill running on this document (started by a script or the dialog).
    let job = app
        .ui
        .generative_bar
        .job
        .map(JobId)
        .and_then(|j| app.session.job(j))
        .or_else(|| app.session.active_job().filter(|j| j.command == COMMAND || j.command == EDIT_COMMAND));
    app.ui.generative_bar.showing_job = job.as_ref().map(|j| j.id.0);
    let picker = crate::generate_panel::picker_items(app, &app.ui.generative_bar.mode.clone(), true);
    let group = app.session.prefs().integrations.group_models_by_family;
    let auto = crate::generate_panel::auto_name(app, &app.ui.generative_bar.mode.clone());
    egui::Area::new(id).order(egui::Order::Foreground).fixed_pos(pos).constrain(false).show(ctx, |ui| {
        egui::Frame::NONE
            .fill(t.card)
            .stroke(Stroke::new(1.0, t.card_border))
            .corner_radius(t.radius_lg)
            .shadow(ui.style().visuals.popup_shadow)
            .inner_margin(egui::Margin::symmetric(10, 7))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.horizontal(|ui| {
                    let (ir, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                    crate::icons::paint(ui, ir, "sparkles", 16.0, t.accent);
                    match job {
                        Some(j) => running(ui, &j, &t, &mut action),
                        None => idle(ui, app, (&picker, auto.as_deref(), group), &t, &mut action),
                    }
                    let (xr, xresp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                    if xresp.hovered() {
                        ui.painter().rect_filled(xr, t.radius_sm, t.hover);
                    }
                    crate::icons::paint(ui, xr, "x", 11.0, if xresp.hovered() { t.text } else { t.text_dim });
                    if xresp.on_hover_text(tl!("Close")).clicked() {
                        action = Some(Action::Close);
                    }
                });
            });
    });
    match action {
        Some(Action::Generate) => {
            let _ = generate(app);
        }
        Some(Action::Enhance) => {
            let _ = enhance(app);
        }
        Some(Action::Cancel(j)) => {
            crate::jobs_ui::cancel(app, j);
        }
        Some(Action::Close) => app.ui.generative_bar.open = false,
        None => {}
    }
}

enum Action {
    Generate,
    Enhance,
    Cancel(JobId),
    Close,
}

/// Where the bar goes: centred under the selection, above it when there is no room below, and
/// always inside the canvas area.
fn place(sel: Rect, size: egui::Vec2, canvas: Rect) -> egui::Pos2 {
    let x = (sel.center().x - size.x / 2.0).clamp(canvas.left() + EDGE, (canvas.right() - size.x - EDGE).max(canvas.left() + EDGE));
    let below = sel.bottom() + GAP;
    let above = sel.top() - GAP - size.y;
    let y = if below + size.y <= canvas.bottom() - EDGE {
        below.max(canvas.top() + EDGE)
    } else if above >= canvas.top() + EDGE {
        above
    } else {
        canvas.bottom() - EDGE - size.y
    };
    pos2(x.round(), y.round())
}

fn running(ui: &mut egui::Ui, j: &photocraft_engine::jobs::JobInfo, t: &Tokens, action: &mut Option<Action>) {
    let msg = if j.message.is_empty() || j.message == j.label { tl!("Generating…").to_string() } else { j.message.clone() };
    ui.label(RichText::new(msg).color(t.text_dim));
    let (br, _) = ui.allocate_exact_size(vec2(180.0, 6.0), Sense::hover());
    crate::jobs_ui::bar(ui, br, (j.progress > 0.0).then_some(j.progress), t);
    let pct = if j.progress > 0.0 { format!("{:.0}%", j.progress * 100.0) } else { String::new() };
    ui.label(RichText::new(pct).color(t.text_faint).size(11.5).monospace());
    if crate::widgets::secondary_button(ui, tl!("Cancel"), 72.0).clicked() {
        *action = Some(Action::Cancel(j.id));
    }
}

fn idle(
    ui: &mut egui::Ui,
    app: &mut PhotocraftApp,
    (picker, auto, group): (&[crate::model_picker::Item], Option<&str>, bool),
    t: &Tokens,
    action: &mut Option<Action>,
) {
    let (bar, shared) = (&mut app.ui.generative_bar, &mut app.ui.generate);
    // Fill (regenerate the selection) or Edit (change the picture by instruction, shown
    // inside the selection). The Generate panel follows.
    let mut mode = if bar.mode == MODE_EDIT { MODE_EDIT.to_string() } else { MODE_FILL.to_string() };
    if crate::widgets::dropdown(ui, "generative-mode", tl!("Mode"), &mut mode, &[(MODE_FILL.to_string(), "Fill"), (MODE_EDIT.to_string(), "Edit")], 64.0) {
        bar.mode = mode.clone();
        shared.mode = mode.clone();
        shared.mode_chosen = true;
    }
    // The prompt; Enter generates.
    let hint = if mode == MODE_EDIT { tl!("Describe the change…") } else { tl!("Describe what to generate…") };
    let field = egui::TextEdit::singleline(&mut shared.prompt).hint_text(hint).desired_width(240.0).id_salt("generative-prompt");
    let resp = ui.add(field);
    if bar.focus {
        resp.request_focus();
        bar.focus = false;
    }
    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    // Enhance: the vision-language model rewrites the prompt with the picture in view (names
    // what is there instead of "this", adds the detail the image model wants).
    let enhancing = bar.enhance_job.is_some();
    let can_enhance = !enhancing && !shared.prompt.trim().is_empty();
    let (er, eresp) = ui.allocate_exact_size(vec2(20.0, 20.0), if can_enhance { Sense::click() } else { Sense::hover() });
    if can_enhance && eresp.hovered() {
        ui.painter().rect_filled(er, t.radius_sm, t.hover);
    }
    crate::icons::paint(
        ui,
        er,
        "wand-sparkles",
        13.0,
        if enhancing {
            t.accent
        } else if can_enhance {
            t.text
        } else {
            t.text_faint
        },
    );
    if eresp.on_hover_text(tl!("Enhance prompt")).clicked() && can_enhance {
        *action = Some(Action::Enhance);
    }
    if enhancing {
        ui.label(RichText::new(tl!("Enhancing…")).color(t.text_dim).size(11.5));
    } else if mode == MODE_FILL && looks_like_an_edit(&shared.prompt) {
        ui.label(RichText::new(tl!("Changing what is there? Switch to Edit")).color(t.text_dim).size(11.5));
    }
    // The model picker: Auto (the fastest permissive tier the server has), then the mode's
    // templates with their estimates, research ones greyed until the preference allows them,
    // missing files marked. The same picker as the Generate panel's.
    let mut current = shared.template.get(&mode).cloned().filter(|c| c != photocraft_engine::generate_cmds::AUTO_TEMPLATE).unwrap_or_default();
    if crate::model_picker::pill(ui, "generative-bar", picker, &mut current, auto, group, 170.0) {
        shared.template.insert(mode.clone(), current);
    }
    // Variations.
    ui.label(RichText::new(tl!("Variations")).color(t.text_dim));
    let mut n = shared.count.clamp(1, 4);
    crate::widgets::dropdown(ui, "generative-variations", tl!("Variations"), &mut n, &[(1u8, "1"), (2, "2"), (3, "3"), (4, "4")], 44.0);
    shared.count = n;
    let can_run = !shared.prompt.trim().is_empty();
    if ui.add_enabled_ui(can_run, |ui| crate::widgets::primary_button(ui, tl!("Generate"), 88.0)).inner.clicked() || (enter && can_run) {
        *action = Some(Action::Generate);
    }
}

#[cfg(test)]
#[path = "generative_bar_tests.rs"]
mod tests;
