//! Generative AI in the shell (#41): the Edit › Generative Fill prompt, the consent prompt, the
//! options bar's Generative Fill… button, the Properties panel's variations, fills in progress
//! (`pending`) and the Generative AI setup (`setup`).
//!
//! The model, the connections and the keys belong to the Craft suite's shared AI layer, reached
//! through the session's [`photocraft_engine::genai::GenerativeAi`] (desktop only). Whether the
//! tool can run is asked each time it is about to be used ([`photocraft_engine::Session::
//! generative_status`]); when it can't, the tool is greyed out, the layer's reason is shown as it
//! is, and the setup is where to fix it. Nothing is ever sent without the person's agreement for a
//! destination off this computer (`consent`).
//!
//! The prompt, the consent prompt and the setup are parameter dialogs on the framework's host
//! ([`FAMILY`]): their keys are [`FILL`], [`consent::KEY`] and [`setup::KEY`].

use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Props as _, Show as _, Sizing, clicked};
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::Label;
use craft::ui::runtime::memory::{MemoryStage, SaveKey, set_saved_value};
use craft::ui::sections::{DialogRole, FooterButton};
use photocraft_doc::{Layer, LayerId};
use photocraft_engine::genai::{AiBilling, AiStatus};
use photocraft_engine::genai_cmds::{ColourMatch, Framing, VariationNote};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::dialogs::params::{self, ParamFamily};

const FILL: &str = photocraft_engine::genai_cmds::FILL;
const SELECT_VARIATION: &str = photocraft_engine::genai_cmds::SELECT_VARIATION;
const MORE: &str = photocraft_engine::genai_cmds::MORE;
const SAVE: &str = photocraft_engine::genai_cmds::SAVE_DIAGNOSTICS;
/// Long side of a variation thumbnail, in texels.
const THUMB: u32 = 96;
/// Most separate areas of a selection the prompt dialog offers to fill one by one (each is its
/// own Generative Fill, sent on its own).
const MAX_SEPARATE: usize = 16;
const NO_AI: &str = photocraft_engine::genai_cmds::NO_AI;
/// Where the last prompt is remembered: the dialog's own memory keeps no free text.
const LAST_PROMPT: &str = "photocraft.genai.prompt";
/// The prompt dialog's footer button that opens the setup.
const SET_UP: &str = "genai.setUp";

mod consent;
mod pending;
mod setup;

pub use pending::{is_generative, on_job_event, paint_canvas as paint_pending};

/// The prompt, consent and setup dialogs' family on the dialog host.
pub(crate) const FAMILY: ParamFamily = ParamFamily { footer: Some(footer), cancel: Some(cancel), ..ParamFamily::new(owns, run) };

/// Shell state that must not be serialised or inspected.
#[derive(Default)]
pub struct Runtime {
    /// Variation thumbnails for the Properties panel, by generative layer and variation: (pixels
    /// fingerprint, texture).
    thumbs: std::collections::HashMap<(LayerId, usize), (u64, egui::TextureHandle)>,
    /// Fills in progress, as drawn ([`pending`]).
    pending: pending::Pending,
    /// The setup dialog's state ([`setup`]); holds what is typed in the key field, outside the
    /// dialog's fields so `ui.inspect` never sees it.
    setup: setup::State,
    /// What the open consent prompt lists ([`consent`]).
    consent: consent::Shared,
}

/// Menu, palette and shortcut entry points. Clicking Generative Fill asks for a prompt; with
/// `prompt` already in the params (scripted use) it runs directly.
pub fn menu(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    match id {
        FILL if params.get("prompt").is_none() => {
            if !offered(app) {
                return Some(Err(NO_AI.into()));
            }
            Some(Ok(json!({"dialog": open_prompt(app).value().to_string()})))
        }
        SAVE if params.get("folder").is_none() => Some(save_diagnostics(app, params)),
        _ => None,
    }
}

/// Each frame, before the dialog host runs: the setup's worker answers and the consent prompt's
/// list.
pub(crate) fn feed(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !offered(app) {
        return;
    }
    setup::feed(app, ctx);
    consent::feed(app);
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

/// Is `key` one of this module's dialogs?
pub(crate) fn owns(key: &str) -> bool {
    key == FILL || key == consent::KEY || key == setup::KEY
}

/// OK on one of this module's dialogs.
fn run(app: &mut PhotocraftApp, ctx: &egui::Context, result: ParamResult) {
    let outcome = match result.key.as_str() {
        consent::KEY => consent::confirm(app, result.fields.text("term")),
        setup::KEY => Ok(Value::Null),
        _ => {
            // The prompt and the separate-areas choice are kept for the next open, whatever the fill does.
            result.remember(ctx);
            set_saved_value(ctx, &SaveKey::named(LAST_PROMPT), Some(result.fields.text("prompt").unwrap_or("").trim().to_owned()));
            confirm_prompt(app, &result.fields)
        }
    };
    if let Err(e) = outcome {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// A footer button of the prompt dialog: Set Up Generative AI….
fn footer(app: &mut PhotocraftApp, _ctx: &egui::Context, _id: DialogId, button: &str) {
    if button == SET_UP {
        open_setup(app);
    }
}

/// The dialog closed without OK: Don't Send on the consent prompt discards what it asked about.
fn cancel(app: &mut PhotocraftApp, _ctx: &egui::Context, id: DialogId) {
    if consent::is(app, id) {
        consent::cancelled(app);
    }
}

/// Where the chosen model runs and what that means for what is sent, in lines.
fn status_text(status: &AiStatus) -> Vec<String> {
    match status {
        AiStatus::Ready { model, connection, local, .. } => vec![
            crate::i18n::fmt(tl!("Generative Fill uses {model} ({connection})."), &[("model", model), ("connection", connection)]),
            if *local { tl!("It runs on this computer: nothing leaves it.") } else { tl!("It runs online: PhotoCraft asks before anything is sent.") }
                .to_owned(),
        ],
        // The shared layer's sentence, as it is.
        AiStatus::Unavailable { reason } => vec![reason.clone()],
    }
}

/// A wrapped line of faint text.
fn faint(ui: &mut egui::Ui, text: &str) {
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.add(egui::Label::new(egui::RichText::new(text).color(t.text_faint).size(12.0)).wrap());
}

/// Open the Generative AI setup (or bring the open one forward).
pub fn open_setup(app: &mut PhotocraftApp) -> DialogId {
    setup::open(app)
}

/// The open hosted parameter dialog with `key`, if any.
pub(crate) fn open_dialog(app: &PhotocraftApp, key: &str) -> Option<DialogId> {
    use craft::ui::dialog::StackDialog as _;
    let dialogs = &app.framework.dialogs;
    dialogs.stack().ids().find(|id| dialogs.get_by_id::<craft::ui::patterns::form_dialogs::HostedParamDialog>(*id).and_then(|d| d.key()) == Some(key))
}

/// Open the prompt dialog (or return the open one). The prompt and the separate-areas choice are
/// remembered by the dialog.
fn open_prompt(app: &mut PhotocraftApp) -> DialogId {
    if let Some(id) = open_dialog(app, FILL) {
        return id;
    }
    let status = app.session.generative_status();
    let areas = photocraft_engine::genai_cmds::selection_areas(&app.session).len();
    let ready = status.is_ready();
    let why = match &status {
        AiStatus::Unavailable { reason } => Some(reason.clone()),
        AiStatus::Ready { .. } => None,
    };
    let lines = status_text(&status);
    let last: String = MemoryStage::peek_saved::<Option<String>>(app.ctx(), &SaveKey::named(LAST_PROMPT)).unwrap_or_default();
    params::open_param(app, FILL, DialogFields::new(), json!({"areas": areas}), move |fields: &mut DialogFields| {
        let mut rows = Vec::new();
        if ready {
            rows.push(
                Param::new("prompt", tl!("Prompt:"), ParamKind::text(last.clone())).max_chars(photocraft_engine::genai_cmds::MAX_PROMPT).select_on_focus(),
            );
            if (2..=MAX_SEPARATE).contains(&areas) {
                let label = crate::i18n::fmt(tl!("Fill each of the {n} separate areas on its own"), &[("n", &areas.to_string())]);
                rows.push(Param::new("separate", label, ParamKind::bool(true)));
            }
        }
        let n = request_count(areas, fields.bool("separate").unwrap_or(true));
        let cost = cost_text(&status, n);
        let lines = lines.clone();
        let empty = fields.text("prompt").is_none_or(|p| p.trim().is_empty());
        let mut dialog = ParamDialog::command(FILL, tl!("Generative Fill"), rows)
            .values(Bind::bound(fields))
            .ok_label(Label::from(crate::i18n::fmt(tl!("Generate {n}"), &[("n", &n.to_string())])))
            .ok_enabled(ready)
            .below(move |ui, _| {
                if ready {
                    faint(
                        ui,
                        if empty {
                            tl!("Empty: removes what is selected and continues the image through it.")
                        } else {
                            tl!("Describe what to generate in the selection.")
                        },
                    );
                    if areas > MAX_SEPARATE {
                        faint(
                            ui,
                            &crate::i18n::fmt(
                                tl!("The selection has {n} separate areas, too many to fill one by one (at most {max}): it is filled as one."),
                                &[("n", &areas.to_string()), ("max", &MAX_SEPARATE.to_string())],
                            ),
                        );
                    }
                    ui.add_space(6.0);
                    for line in &cost {
                        ui.add(egui::Label::new(line.as_str()).wrap());
                    }
                }
                for line in &lines {
                    faint(ui, line);
                }
            });
        if let Some(why) = &why {
            dialog = dialog
                .ok_reason(Label::from(why.clone()))
                .extra_buttons(vec![(SET_UP.to_owned(), FooterButton::new(tl!("Set Up Generative AI…"), DialogRole::Other))]);
        }
        dialog
    })
}

/// How many variations, each its own request, Generate makes: three, for each of `areas`
/// separate areas when they are filled on their own.
pub(crate) fn request_count(areas: usize, separate: bool) -> u64 {
    let per = photocraft_engine::genai_cmds::DEFAULT_VARIATIONS as u64;
    if separate && (2..=MAX_SEPARATE).contains(&areas) { per * areas as u64 } else { per }
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
    let mut lines = vec![crate::i18n::fmt(&what, &[("connection", connection), ("n", &n.to_string())])];
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

/// Generate: run the fill (one per separate area when asked).
fn confirm_prompt(app: &mut PhotocraftApp, f: &DialogFields) -> Result<Value, String> {
    let prompt = f.text("prompt").unwrap_or("").trim().to_owned();
    let separate = f.bool("separate").unwrap_or(true);
    let areas = params::captured(app, FILL).get("areas").and_then(Value::as_u64).unwrap_or(0) as usize;
    let fill = json!({"prompt": prompt});
    let consent_ids = |app: &PhotocraftApp| -> Vec<u64> { app.session.generative_fills().iter().filter(|f| f.state == "consent").map(|f| f.id).collect() };
    // An area that wasn't sent for a reason other than waiting for consent: its error is kept,
    // whatever the other areas did.
    let mut failed: Option<String> = None;
    let r = if separate && (2..=MAX_SEPARATE).contains(&areas) {
        // One fill per area, side by side.
        let mut last = Ok(Value::Null);
        for k in 0..areas {
            let mut p = fill.clone();
            p["area"] = json!(k);
            let before = consent_ids(app);
            last = app.run(FILL, p);
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
        app.run(FILL, fill)
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
    if !clicked(&Button::new(tl!("Generative Fill…")).enabled(enabled).tooltip(hover).show(ui)) {
        return;
    }
    if why_not.is_none() {
        let _ = crate::menus::invoke(app, &ui.ctx().clone(), FILL, json!({}));
    } else {
        open_setup(app);
    }
}

/// A fingerprint of a surface's pixels, for the thumbnails' cache.
fn surface_fingerprint(s: &photocraft_raster::Surface) -> u64 {
    use std::hash::{Hash as _, Hasher as _};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.generation().hash(&mut h);
    let b = s.content_bounds();
    (b.x0, b.y0, b.x1, b.y1).hash(&mut h);
    h.finish()
}

/// The mask-shaped crop of a variation: its pixels inside the mask's bounds, faded where the mask
/// hides them.
fn variation_image(s: &photocraft_raster::Surface, mask: Option<&photocraft_doc::LayerMask>) -> Option<egui::ColorImage> {
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

/// Variation `index` of generative layer `layer`'s thumbnail texture, made again only when its
/// pixels or the mask changed.
fn variation_thumb(app: &mut PhotocraftApp, ctx: &egui::Context, layer: &Layer, index: usize) -> Option<egui::TextureId> {
    let pixels = layer.generative.as_ref()?.variations.get(index)?;
    let mask = layer.mask.as_ref();
    let rev = surface_fingerprint(pixels) ^ mask.map_or(0, |m| surface_fingerprint(&m.surface));
    let key = (layer.id, index);
    if let Some((r, tex)) = app.genai.thumbs.get(&key)
        && *r == rev
    {
        return Some(tex.id());
    }
    let img = variation_image(pixels, mask)?;
    let tex = ctx.load_texture(format!("genai-variation-{}-{index}", layer.id.0), img, egui::TextureOptions::LINEAR);
    let id = tex.id();
    app.genai.thumbs.insert(key, (rev, tex));
    Some(id)
}

/// Properties › Variations, for the active generative layer, as Photoshop's Properties panel
/// shows one: a thumbnail per variation (click one and the layer shows it, one undo step), the
/// prompt, and Generate, which asks for three more with the same picture, mask and prompt.
/// Compact, since the dock can be short.
pub fn variations_section(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let Some(id) = app.session.generative_layer() else { return };
    let Some(layer) = app.session.active().and_then(|st| st.doc.layer(id).cloned()) else { return };
    let Some(generative) = layer.generative.as_deref() else { return };
    let t = crate::theme::Tokens::get(ui.ctx());
    if let Some(st) = app.session.active() {
        app.genai.thumbs.retain(|(l, i), _| st.doc.layer(*l).and_then(|l| l.generative.as_ref()).is_some_and(|g| *i < g.variations.len()));
    }
    let enabled = app.session.is_enabled(MORE);
    // Why it can't, as the shared layer says it; else what it does.
    let tip = app.session.disabled_reason(MORE).unwrap_or_else(|| tl!("Sends the same area and prompt again for three more variations.").to_owned());
    ui.add_space(2.0);
    ui.label(egui::RichText::new(tl!("Variations")).font(crate::theme::semibold(12.5)).color(t.text));
    ui.add_space(4.0);
    let ctx = ui.ctx().clone();
    let height = 48.0;
    let mut chosen = None;
    egui::ScrollArea::horizontal().id_salt("genai-variations").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for i in 0..generative.variations.len() {
                let tex = variation_thumb(app, &ctx, &layer, i);
                let size = app.genai.thumbs.get(&(id, i)).map_or(egui::vec2(1.0, 1.0), |(_, h)| h.size_vec2());
                let width = (height * size.x / size.y.max(1.0)).clamp(36.0, 96.0);
                let (r, resp) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
                ui.painter().rect_filled(r, t.radius_sm, t.field);
                if let Some(tex) = tex {
                    let fit = ((width - 4.0) / size.x.max(1.0)).min((height - 4.0) / size.y.max(1.0));
                    let img = egui::Rect::from_center_size(r.center(), size * fit);
                    ui.painter().image(tex, img, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                }
                let shown = i == generative.selected;
                let (w, c) = if shown {
                    (2.0, t.accent)
                } else if resp.hovered() {
                    (1.0, t.accent_border)
                } else {
                    (1.0, t.field_border)
                };
                ui.painter().rect_stroke(r, t.radius_sm, egui::Stroke::new(w, c), egui::StrokeKind::Inside);
                let name = crate::i18n::fmt(tl!("Variation {n}"), &[("n", &(i + 1).to_string())]);
                resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, shown, &name));
                if resp.on_hover_text(&name).clicked() && !shown {
                    chosen = Some(i);
                }
            }
        });
    });
    // How the variation showing was placed, when it wasn't plainly (colour matching skipped, the
    // service's picture framed differently).
    if let Some(note) = app.session.generative_variation_note(id, generative.selected) {
        for line in note_lines(note) {
            ui.add_space(2.0);
            faint(ui, &line);
        }
    }
    ui.add_space(6.0);
    ui.label(egui::RichText::new(tl!("Prompt")).font(crate::theme::semibold(12.0)).color(t.text_dim));
    let prompt = if generative.prompt.is_empty() { tl!("No prompt: continues the image") } else { generative.prompt.as_str() };
    ui.add(egui::Label::new(egui::RichText::new(prompt).color(t.text)).wrap());
    ui.add_space(4.0);
    let more = clicked(&Button::new(tl!("Generate")).enabled(enabled).tooltip(tip).sizing(Sizing::FillWidth).show(ui));
    if offers_diagnostics(app, id) && clicked(&Button::new(tl!("Save Fill Diagnostics…")).tooltip(diagnostics_tip()).sizing(Sizing::FillWidth).show(ui)) {
        let ctx = ui.ctx().clone();
        if let Err(e) = crate::menus::invoke(app, &ctx, SAVE, json!({"layer": id.0})) {
            app.ui.status = e;
        }
    }
    if let Some(i) = chosen
        && let Err(e) = app.run(SELECT_VARIATION, json!({"layer": id.0, "index": i}))
    {
        app.ui.status = e;
    }
    if more {
        match app.run(MORE, json!({"layer": id.0, "count": photocraft_engine::genai_cmds::DEFAULT_VARIATIONS})) {
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
