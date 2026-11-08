//! Generative AI in the shell (#41): the Edit › Generative Fill prompt, the consent prompt, the
//! options bar's Generative Fill… button, the Properties panel's variations, fills in progress
//! (`pending`) and the Generative AI section of Preferences › Integrations (`setup`).
//!
//! The model, the connections and the keys belong to the Craft suite's shared AI layer, reached
//! through the session's [`photocraft_engine::genai::GenerativeAi`] (desktop only). Whether the
//! tool can run is asked each time it is about to be used ([`photocraft_engine::Session::
//! generative_status`]); when it can't, the tool is greyed out, the layer's reason is shown as it
//! is, and Preferences › Integrations is where to fix it. Nothing is ever sent without the
//! person's agreement for a destination off this computer (`consent`).

use photocraft_doc::{Layer, LayerId};
use photocraft_engine::genai::{AiBilling, AiStatus};
use photocraft_engine::genai_cmds::{ColourMatch, Framing, VariationNote};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;

const FILL: &str = photocraft_engine::genai_cmds::FILL;
const SELECT_VARIATION: &str = photocraft_engine::genai_cmds::SELECT_VARIATION;
const MORE: &str = photocraft_engine::genai_cmds::MORE;
const SAVE: &str = photocraft_engine::genai_cmds::SAVE_DIAGNOSTICS;
/// Long side of a variation thumbnail, in texels.
const THUMB: u32 = 96;
/// Most separate areas of a selection the prompt dialog offers to fill one by one (each is its
/// own Generative Fill, sent on its own).
const MAX_SEPARATE: usize = 16;
const PROMPT_MARK: &str = "__genaiPrompt";
/// The Preferences section the Generative AI setup is in.
pub const SETUP_SECTION: &str = "integrations";
const NO_AI: &str = photocraft_engine::genai_cmds::NO_AI;

mod consent;
mod pending;
mod setup;

pub use consent::CONSENT_MARK;
pub use pending::{is_generative, layer_rows, layer_rows_unplaced, on_job_event, paint_canvas as paint_pending, status as status_fills};
pub use setup::prefs_section;

/// Shell state that must not be serialised or inspected.
#[derive(Default)]
pub struct Runtime {
    /// Variation thumbnails for the Properties panel, by variation layer: (pixels fingerprint,
    /// texture).
    thumbs: std::collections::HashMap<LayerId, (u64, egui::TextureHandle)>,
    /// Fills in progress, as drawn ([`pending`]).
    pending: pending::Pending,
    /// Preferences › Integrations › Generative AI ([`setup`]); holds what is typed in the key
    /// field, outside the dialog's fields so `ui.inspect` never sees it.
    setup: setup::State,
}

/// Menu, palette and shortcut entry points. Clicking Generative Fill asks for a prompt; with
/// `prompt` already in the params (scripted use) it runs directly.
pub fn menu(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    match id {
        FILL if params.get("prompt").is_none() => {
            // Asked now, not at start-up: the person may have changed their choice meanwhile.
            if let Some(why) = app.session.disabled_reason(FILL) {
                return Some(Err(why));
            }
            Some(Ok(json!({"dialog": open_prompt(app)})))
        }
        SAVE if params.get("folder").is_none() => Some(save_diagnostics(app, params)),
        _ => None,
    }
}

/// What Save Fill Diagnostics does, for its tooltips.
fn diagnostics_tip() -> &'static str {
    tl!("Writes what this fill sent and got back (the pictures, the mask, the prompt and a summary) to a folder you choose. Nothing is saved unless you ask.")
}

/// Save Fill Diagnostics…: ask for a folder, then write what the fill (`fill`, `layer` or the
/// active generative layer) sent and got back into a new folder there. Desktop only.
fn save_diagnostics(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    if !offered(app) {
        return Err(NO_AI.into());
    }
    let mut target = params.as_object().cloned().unwrap_or_default();
    if !target.contains_key("fill") && !target.contains_key("layer") {
        let layer = app.session.generative_layer().ok_or_else(|| tl!("Select a generative layer first.").to_owned())?;
        target.insert("layer".into(), json!(layer.0));
    }
    app.ask_folder(move |app, dir| {
        target.insert("folder".into(), json!(dir));
        let r = app.run(SAVE, Value::Object(target));
        if let Ok(v) = &r {
            let folder = v.get("folder").and_then(Value::as_str).unwrap_or(&dir).to_owned();
            app.ui.status = crate::i18n::fmt(tl!("Saved Generative Fill diagnostics to {folder}"), &[("folder", &folder)]);
            app.ui.status_error = false;
        }
        r
    })
}

/// Does this app offer Save Fill Diagnostics for `layer` (desktop, and a capture is kept)?
pub fn offers_diagnostics(app: &PhotocraftApp, layer: LayerId) -> bool {
    offered(app) && app.session.generative_has_diagnostics(layer)
}

/// Does this app offer Generative AI at all (the desktop app; not the web build)?
pub fn offered(app: &PhotocraftApp) -> bool {
    app.services.genai.is_some()
}

/// Is the Edit › Generative Fill item listed? Where generative AI can't exist (the web build) it
/// isn't; on the desktop it is, greyed out while the shared layer says it can't run.
pub fn menu_visible(app: &PhotocraftApp) -> bool {
    offered(app)
}

/// Is this the prompt dialog or the consent prompt?
pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(PROMPT_MARK) || fields.contains_key(CONSENT_MARK)
}

/// The default button's label: Generate with the number of variations it makes, or Send.
pub fn ok_label(fields: &Map<String, Value>) -> Option<std::borrow::Cow<'static, str>> {
    if fields.contains_key(PROMPT_MARK) {
        Some(crate::i18n::fmt(tl!("Generate {n}"), &[("n", &request_count(fields).to_string())]).into())
    } else if fields.contains_key(CONSENT_MARK) {
        Some(tl!("Send").into())
    } else {
        None
    }
}

/// The consent prompt's Cancel reads Don't Send.
pub fn cancel_label(fields: &Map<String, Value>) -> Option<&'static str> {
    fields.contains_key(CONSENT_MARK).then(|| tl!("Don't Send"))
}

/// The dialog closed without OK: Don't Send on the consent prompt discards what it asked about.
pub fn cancelled(app: &mut PhotocraftApp, fields: &Map<String, Value>) {
    if fields.contains_key(CONSENT_MARK) {
        consent::cancelled(app, fields);
    }
}

/// OK is greyed while the tool can't run (the prompt shows why).
pub fn can_confirm(app: &PhotocraftApp, fields: &Map<String, Value>) -> bool {
    !fields.contains_key(PROMPT_MARK) || app.session.generative_status().is_ready()
}

/// Open the prompt dialog (or return the open one), with the last prompt and separate-areas choice.
fn open_prompt(app: &mut PhotocraftApp) -> u64 {
    if let Some(d) = app.ui.dialogs.iter().find(|d| d.fields.contains_key(PROMPT_MARK)) {
        return d.id;
    }
    let last = app.session.prefs().dialogs.get(FILL).cloned().unwrap_or(Value::Null);
    let prompt = last.get("prompt").and_then(Value::as_str).unwrap_or("").to_owned();
    let separate = last.get("separate").and_then(Value::as_bool).unwrap_or(true);
    let areas = photocraft_engine::genai_cmds::selection_areas(&app.session).len();
    let mut f = Map::new();
    f.insert(PROMPT_MARK.into(), json!(true));
    // Automation confirming this dialog is judged as the command (and refused).
    f.insert("__command".into(), json!(FILL));
    f.insert("__label".into(), json!("Generative Fill"));
    f.insert("prompt".into(), json!(prompt));
    f.insert("separate".into(), json!(separate));
    f.insert("__areas".into(), json!(areas));
    app.ui.open_dialog(DialogKind::Command, f)
}

/// Draw the prompt dialog or the consent prompt.
pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    if f.contains_key(CONSENT_MARK) {
        consent::body(app, ui, f);
    } else {
        prompt_body(app, ui, f);
    }
}

fn faint(ui: &mut egui::Ui, text: &str) {
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.add(egui::Label::new(egui::RichText::new(text).color(t.text_faint).size(12.0)).wrap());
}

/// Where the chosen model runs and what that means for what is sent, in a line.
fn status_lines(ui: &mut egui::Ui, status: &AiStatus) {
    match status {
        AiStatus::Ready { model, connection, local, .. } => {
            faint(ui, &crate::i18n::fmt(tl!("Generative Fill uses {model} ({connection})."), &[("model", model), ("connection", connection)]));
            faint(
                ui,
                if *local { tl!("It runs on this computer: nothing leaves it.") } else { tl!("It runs online: PhotoCraft asks before anything is sent.") },
            );
        }
        // The shared layer's sentence, as it is.
        AiStatus::Unavailable { reason } => {
            let t = crate::theme::Tokens::get(ui.ctx());
            ui.add(egui::Label::new(egui::RichText::new(reason).color(t.text).size(12.5)).wrap());
        }
    }
}

/// Open Preferences › Integrations, where Generative AI is set up.
pub fn open_setup(app: &mut PhotocraftApp) -> u64 {
    crate::prefs_ui::open_preferences(app, SETUP_SECTION)
}

/// The prompt dialog: the prompt, separate areas, what Generate will do and which model; setup instead when the tool can't run.
fn prompt_body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let status = app.session.generative_status();
    if !status.is_ready() {
        status_lines(ui, &status);
        ui.add_space(8.0);
        if crate::widgets::secondary_button(ui, tl!("Set Up Generative AI…"), 0.0).clicked() {
            open_setup(app);
        }
        return;
    }
    let mut prompt = f.get("prompt").and_then(Value::as_str).unwrap_or("").to_owned();
    ui.label(tl!("Describe what to generate in the selection, or leave it empty to continue the image through it."));
    ui.add_space(6.0);
    let r = ui.add(
        egui::TextEdit::singleline(&mut prompt)
            .hint_text(tl!("Prompt (optional)"))
            .desired_width(f32::INFINITY)
            .char_limit(photocraft_engine::genai_cmds::MAX_PROMPT),
    );
    crate::field_tab::register(ui.ctx(), r.id);
    if !f.contains_key("__focused") {
        f.insert("__focused".into(), json!(true));
        crate::field_tab::focus(ui.ctx(), r.id);
    }
    if r.gained_focus() {
        crate::field_tab::select_all(ui.ctx(), r.id, &prompt);
    }
    if r.changed() {
        f.insert("prompt".into(), json!(prompt));
    }
    if prompt.trim().is_empty() {
        ui.add_space(2.0);
        faint(ui, tl!("Empty: removes what is selected and continues the image through it."));
    }
    areas_section(ui, f);
    ui.add_space(8.0);
    cost_lines(ui, &status, request_count(f));
    ui.add_space(2.0);
    status_lines(ui, &status);
}

/// How many variations, each its own request, Generate makes from the prompt dialog's fields:
/// three, for each separate area when they are filled on their own.
pub(crate) fn request_count(f: &Map<String, Value>) -> u64 {
    let per = photocraft_engine::genai_cmds::DEFAULT_VARIATIONS as u64;
    let areas = f.get("__areas").and_then(Value::as_u64).unwrap_or(0);
    let separate = f.get("separate").and_then(Value::as_bool).unwrap_or(true);
    if separate && (2..=MAX_SEPARATE as u64).contains(&areas) { per * areas } else { per }
}

/// What Generate is about to do, before it is pressed: how many variations, as how many requests
/// to which connection, and how the connection says they are paid for (never a price: PhotoCraft
/// doesn't know one).
pub(crate) fn cost_text(status: &AiStatus, n: u64) -> Vec<String> {
    let AiStatus::Ready { connection, local, billing, .. } = status else { return Vec::new() };
    let lang = crate::i18n::current();
    let what = if *local {
        crate::i18n::trn(lang, n, "Makes {n} variation on this computer, with {connection}.", "Makes {n} variations on this computer, with {connection}.")
    } else {
        crate::i18n::trn(lang, n, "Makes {n} variation: {n} request to {connection}.", "Makes {n} variations: {n} separate requests to {connection}.")
    };
    let mut lines = vec![crate::i18n::fmt(&what, &[("connection", connection)])];
    let paid = match (billing, *local) {
        (AiBilling::Metered, _) => Some(tl!("Each request is billed by the service.")),
        (AiBilling::Subscription, _) => Some(tl!("Each request uses your subscription allowance.")),
        (AiBilling::Free, false) => Some(tl!("The connection is set up as free of charge.")),
        (AiBilling::Unknown, false) => Some(tl!("Check the service's own terms for what each request costs.")),
        (AiBilling::Free | AiBilling::Unknown, true) => None,
    };
    lines.extend(paid.map(str::to_owned));
    lines
}

/// Draw [`cost_text`].
fn cost_lines(ui: &mut egui::Ui, status: &AiStatus, n: u64) {
    let t = crate::theme::Tokens::get(ui.ctx());
    for line in cost_text(status, n) {
        ui.add(egui::Label::new(egui::RichText::new(line).color(t.text).size(12.5)).wrap());
    }
}

/// The selection's separate areas (several objects selected together): fill each on its own, side
/// by side, each in its own layer, or all of it as one.
fn areas_section(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let n = f.get("__areas").and_then(Value::as_u64).unwrap_or(0);
    if n < 2 {
        return;
    }
    ui.add_space(8.0);
    if n as usize > MAX_SEPARATE {
        faint(
            ui,
            &crate::i18n::fmt(
                tl!("The selection has {n} separate areas, too many to fill one by one (at most {max}): it is filled as one."),
                &[("n", &n.to_string()), ("max", &MAX_SEPARATE.to_string())],
            ),
        );
        return;
    }
    let mut separate = f.get("separate").and_then(Value::as_bool).unwrap_or(true);
    let label = crate::i18n::trn(crate::i18n::current(), n, "Fill each of the {n} separate areas on its own", "Fill each of the {n} separate areas on its own");
    if ui.checkbox(&mut separate, label).changed() {
        f.insert("separate".into(), json!(separate));
    }
    if separate {
        faint(ui, tl!("Each area becomes its own Generative Fill and layer, generated side by side; each sends its own requests."));
    }
}

/// OK: run the fill (one per separate area when asked), or answer the consent prompt.
pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    if f.contains_key(CONSENT_MARK) {
        return consent::confirm(app, f);
    }
    let prompt = f.get("prompt").and_then(Value::as_str).unwrap_or("").trim().to_owned();
    let separate = f.get("separate").and_then(Value::as_bool).unwrap_or(true);
    let areas = f.get("__areas").and_then(Value::as_u64).unwrap_or(0) as usize;
    app.session.prefs.edit(|p| p.dialogs.insert(FILL.into(), json!({"prompt": prompt, "separate": separate})));
    let mut params = Map::new();
    params.insert("prompt".into(), json!(prompt));
    let consent_ids = |app: &PhotocraftApp| -> Vec<u64> { app.session.generative_fills().iter().filter(|f| f.state == "consent").map(|f| f.id).collect() };
    // An area that wasn't sent for a reason other than waiting for consent: its error is kept,
    // whatever the other areas did.
    let mut failed: Option<String> = None;
    let r = if separate && (2..=MAX_SEPARATE).contains(&areas) {
        // One fill per area, side by side.
        let mut last = Ok(Value::Null);
        for k in 0..areas {
            let mut p = params.clone();
            p.insert("area".into(), json!(k));
            let before = consent_ids(app);
            last = app.run(FILL, Value::Object(p));
            // One waiting for consent isn't a failure: the others are asked about with it.
            let asked = consent_ids(app).iter().any(|id| !before.contains(id));
            if let Err(e) = &last
                && !asked
            {
                failed.get_or_insert_with(|| e.clone());
                if consent_ids(app).is_empty() {
                    break;
                }
            }
        }
        last
    } else {
        app.run(FILL, Value::Object(params))
    };
    // A fill that must be agreed to first waits for the person: ask now.
    consent::ask_if_waiting(app);
    let waiting = !consent_ids(app).is_empty();
    if waiting && r.is_err() && failed.is_none() {
        // Run inline, the wait came back as the command's error: it is a question, not a failure.
        app.ui.status.clear();
        app.ui.status_error = false;
    }
    // In the background the fill took what it needs: let the selection go, so the next area can
    // be selected while it generates.
    if (r.is_ok() || waiting) && app.background_jobs {
        let _ = app.run("select.deselect", json!({}));
    }
    match failed {
        Some(e) => Err(e),
        None if waiting => Ok(Value::Null),
        None => r,
    }
}

/// The Generative Fill button for the selection tools' options bar, next to Select and Mask….
/// Shown once the tool can run (a configured, working model). Before that it would only promote
/// setup, permanent clutter in three bars for someone who doesn't use AI, so it stays hidden
/// unless Preferences › Integrations › Always show the Generative Fill… button is on; then it
/// opens the setup. Edit › Generative Fill stays the main route either way. Never in the web build.
pub fn options_bar_button(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if !offered(app) {
        return;
    }
    let why_not = match app.session.generative_status() {
        AiStatus::Ready { .. } => None,
        AiStatus::Unavailable { reason } => Some(reason),
    };
    if why_not.is_some() && !app.session.prefs().integrations.always_show_generative_fill_button {
        return;
    }
    // Not set up, the button says why and is always clickable: it opens the setup.
    let enabled = why_not.is_some() || app.session.is_enabled(FILL);
    let hover = why_not.clone().unwrap_or_else(|| tl!("Fill the selection from a prompt").to_string());
    let clicked = ui.add_enabled_ui(enabled, |ui| crate::widgets::secondary_button(ui, tl!("Generative Fill…"), 0.0).on_hover_text(hover)).inner.clicked();
    if !clicked {
        return;
    }
    if why_not.is_none() {
        let _ = crate::menus::invoke(app, &ui.ctx().clone(), FILL, json!({}));
    } else {
        open_setup(app);
    }
}

/// The selection-shaped crop of a variation: its pixels inside the group mask's bounds, faded
/// where the mask hides them.
fn variation_image(variation: &Layer, mask: Option<&photocraft_doc::LayerMask>) -> Option<egui::ColorImage> {
    let s = variation.surface()?;
    let area = mask.map_or_else(|| s.content_bounds(), |m| m.surface.content_bounds().intersect(&s.content_bounds()));
    if area.is_empty() {
        return None;
    }
    let k = f64::from(THUMB) / f64::from(area.width().max(area.height()));
    let (w, h) = (((f64::from(area.width()) * k).round() as usize).max(1), ((f64::from(area.height()) * k).round() as usize).max(1));
    let mut px = [0.0f32; 8];
    let mut m = [0.0f32; 1];
    let n = s.channels().min(px.len());
    let mut out = Vec::with_capacity(w * h);
    for ty in 0..h {
        for tx in 0..w {
            let x = area.x0 + ((tx as f64 + 0.5) / k) as i32;
            let y = area.y0 + ((ty as f64 + 0.5) / k) as i32;
            s.read_pixel(x, y, &mut px[..n]);
            let c = photocraft_raster::to_rgba(&s.format(), &px[..n]);
            let a = mask.map_or(1.0, |mask| {
                mask.surface.read_pixel(x, y, &mut m);
                m[0]
            });
            let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            out.push(egui::Color32::from_rgba_unmultiplied(q(c[0]), q(c[1]), q(c[2]), q(c[3] * a.max(0.25))));
        }
    }
    Some(egui::ColorImage::new([w, h], out))
}

/// A variation's thumbnail texture, made again only when its pixels or the mask changed.
fn variation_thumb(app: &mut PhotocraftApp, ctx: &egui::Context, variation: &Layer, mask: Option<&photocraft_doc::LayerMask>) -> Option<egui::TextureId> {
    let rev = variation.surface().map_or(0, crate::surface_fingerprint) ^ mask.map_or(0, |m| crate::surface_fingerprint(&m.surface));
    if let Some((r, tex)) = app.genai.thumbs.get(&variation.id)
        && *r == rev
    {
        return Some(tex.id());
    }
    let img = variation_image(variation, mask)?;
    let tex = ctx.load_texture(format!("genai-variation-{}", variation.id.0), img, egui::TextureOptions::LINEAR);
    let id = tex.id();
    app.genai.thumbs.insert(variation.id, (rev, tex));
    Some(id)
}

/// Properties › Variations, for the active generative layer (or one of its variations): a row of
/// thumbnails, one per variation (click one to show it), and Generate More, as Photoshop's
/// Properties panel offers for a generative layer. Compact, since the dock can be short.
pub fn variations_section(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let Some(group_id) = app.session.generative_layer() else { return };
    let Some(group) = app.session.active().and_then(|st| st.doc.layer(group_id).cloned()) else { return };
    let Some(children) = group.children() else { return };
    let t = crate::theme::Tokens::get(ui.ctx());
    if let Some(st) = app.session.active() {
        app.genai.thumbs.retain(|id, _| st.doc.layer(*id).is_some());
    }
    let prompt = app.session.generative_prompt(group_id).unwrap_or("").to_owned();
    let enabled = app.session.is_enabled(MORE);
    // Why it can't, as the shared layer says it; else what it does.
    let tip = app.session.disabled_reason(MORE).unwrap_or_else(|| tl!("Sends the same area and prompt again for three more.").to_owned());
    let mut more = false;
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        let head = ui.label(egui::RichText::new(tl!("Variations")).font(crate::theme::semibold(12.5)).color(t.text));
        head.on_hover_text(if prompt.is_empty() { tl!("No prompt: continues the image") } else { &prompt });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let r = ui.add_enabled_ui(enabled, |ui| crate::widgets::secondary_button(ui, tl!("Generate More"), 0.0)).inner;
            more = if enabled { r.on_hover_text(tip).clicked() } else { r.on_disabled_hover_text(tip).clicked() };
        });
    });
    ui.add_space(4.0);
    let ctx = ui.ctx().clone();
    let height = 48.0;
    let mut chosen = None;
    egui::ScrollArea::horizontal().id_salt("genai-variations").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (i, v) in children.iter().enumerate() {
                let tex = variation_thumb(app, &ctx, v, group.mask.as_ref());
                let size = app.genai.thumbs.get(&v.id).map_or(egui::vec2(1.0, 1.0), |(_, h)| h.size_vec2());
                let width = (height * size.x / size.y.max(1.0)).clamp(36.0, 96.0);
                let (r, resp) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
                ui.painter().rect_filled(r, t.radius_sm, t.field);
                if let Some(tex) = tex {
                    let fit = ((width - 4.0) / size.x.max(1.0)).min((height - 4.0) / size.y.max(1.0));
                    let img = egui::Rect::from_center_size(r.center(), size * fit);
                    ui.painter().image(tex, img, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                }
                let (w, c) = if v.visible {
                    (2.0, t.accent)
                } else if resp.hovered() {
                    (1.0, t.accent_border)
                } else {
                    (1.0, t.field_border)
                };
                ui.painter().rect_stroke(r, t.radius_sm, egui::Stroke::new(w, c), egui::StrokeKind::Inside);
                resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, v.visible, &v.name));
                if resp.on_hover_text(&v.name).clicked() && !v.visible {
                    chosen = Some(i);
                }
            }
        });
    });
    // How the variation showing was placed, when it wasn't plainly (colour matching skipped, the
    // service's picture framed differently).
    if let Some(note) = children.iter().find(|v| v.visible).and_then(|v| app.session.generative_variation_note(v.id)) {
        for line in note_lines(note) {
            ui.add_space(2.0);
            faint(ui, &line);
        }
    }
    if offers_diagnostics(app, group_id) && ui.link(egui::RichText::new(tl!("Save Fill Diagnostics…")).size(12.0)).on_hover_text(diagnostics_tip()).clicked()
    {
        let ctx = ui.ctx().clone();
        if let Err(e) = crate::menus::invoke(app, &ctx, SAVE, json!({"layer": group_id.0})) {
            app.ui.status = e;
        }
    }
    if let Some(i) = chosen
        && let Err(e) = app.run(SELECT_VARIATION, json!({"layer": group_id.0, "index": i}))
    {
        app.ui.status = e;
    }
    if more {
        match app.run(MORE, json!({"layer": group_id.0})) {
            Ok(_) => consent::ask_if_waiting(app),
            Err(e) => {
                consent::ask_if_waiting(app);
                if !app.session.generative_fills().iter().any(|f| f.state == "consent") {
                    app.ui.status = e;
                }
            }
        }
    }
    ui.add_space(6.0);
    crate::widgets::hairline(ui);
    ui.add_space(4.0);
}

/// What a person should know about how a variation was placed: nothing when it came back the
/// area's shape and its colours were matched.
pub(crate) fn note_lines(note: &VariationNote) -> Vec<String> {
    let mut lines = Vec::new();
    match note.colour {
        ColourMatch::Matched { .. } => {}
        ColourMatch::TooDifferent { .. } => lines.push(
            tl!("Colour matching was skipped for this variation: the picture around the selection came back too different to match it safely.").to_owned(),
        ),
        ColourMatch::TooLittleOutside => {
            lines.push(tl!("Colour matching was skipped for this variation: too little of the picture lies outside the selection to measure.").to_owned())
        }
    }
    let size = |s: [u32; 2]| format!("{}×{}", s[0], s[1]);
    match note.framing {
        Framing::Same => {}
        Framing::Centre { returned, sent } => lines.push(crate::i18n::fmt(
            tl!("The service returned a {returned} picture for a {sent} area; its centre was used."),
            &[("returned", &size(returned)), ("sent", &size(sent))],
        )),
        Framing::Scaled { returned, sent } => lines.push(crate::i18n::fmt(
            tl!("The service returned a {returned} picture for a {sent} area; it lined up with the picture around the selection once scaled."),
            &[("returned", &size(returned)), ("sent", &size(sent))],
        )),
    }
    lines
}

/// The status line's addition for a fill that landed: how many of its variations had colour
/// matching skipped, if any (`matching` is the command result's list).
pub(crate) fn skipped_note(matching: Option<&Value>) -> Option<String> {
    let n = matching?.as_array()?.iter().filter(|m| m.pointer("/colour/state").and_then(Value::as_str).is_some_and(|s| s != "matched")).count() as u64;
    (n > 0).then(|| {
        crate::i18n::trn(crate::i18n::current(), n, "Colour matching was skipped for {n} variation.", "Colour matching was skipped for {n} variations.")
    })
}

#[cfg(test)]
mod tests;
