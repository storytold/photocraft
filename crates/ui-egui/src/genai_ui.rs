//! Generative AI in the shell (#41): Generative AI Settings (provider, model and API key), the
//! Edit › Generative Fill prompt, and installing the configured provider on the session.
//!
//! Off by default: until the user picks a provider and saves a key, no provider exists, the
//! Generative Fill menu item is hidden and nothing is ever contacted. The provider and model live
//! in preferences (shell-owned `dialogs` entry); the key lives only in the OS credential store
//! ([`crate::GenAiServices`]) and in process memory. The key is typed into a buffer outside the
//! dialog's fields, so `ui.inspect` and the control channel never see it.

use photocraft_doc::{Layer, LayerId};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;

/// Opens Generative AI Settings.
pub const SETTINGS: &str = "ui.generativeAiSettings";
const FILL: &str = photocraft_engine::genai_cmds::FILL;
const SELECT_VARIATION: &str = photocraft_engine::genai_cmds::SELECT_VARIATION;
const MORE: &str = photocraft_engine::genai_cmds::MORE;
/// Long side of a variation thumbnail, in texels.
const THUMB: u32 = 96;
const SETTINGS_MARK: &str = "__genaiSettings";
const PROMPT_MARK: &str = "__genaiPrompt";
/// `prefs.dialogs` entry holding `{"provider", "model"}`.
const PREF: &str = "ui.generativeAiSettings";
const KEY_PAGE: &str = "https://aistudio.google.com/apikey";

/// Shell state that must not be serialised or inspected.
#[derive(Default)]
pub struct Runtime {
    /// What is typed in the settings dialog's key field. Cleared when the dialog closes.
    key_input: String,
    /// Variation thumbnails for the Properties panel, by variation layer: (pixels fingerprint,
    /// texture).
    thumbs: std::collections::HashMap<LayerId, (u64, egui::TextureHandle)>,
}

/// Providers the settings offer: (id, name). Only providers the app can build appear.
fn providers() -> Vec<(String, &'static str)> {
    vec![(String::new(), tl!("None")), (photocraft_genai::gemini::ID.into(), photocraft_genai::gemini::NAME)]
}

fn saved(app: &PhotocraftApp) -> (String, String) {
    let v = app.session.prefs().dialogs.get(PREF);
    let get = |k: &str| v.and_then(|v| v.get(k)).and_then(Value::as_str).unwrap_or("").to_owned();
    (get("provider"), get("model"))
}

/// Build the saved provider and install it on the session (at launch and after Settings). Reads
/// the credential store only when a provider is configured; never touches the network.
pub fn restore(app: &mut PhotocraftApp) -> Result<(), String> {
    let (provider, model) = saved(app);
    if provider.is_empty() {
        app.session.set_image_provider(None);
        return Ok(());
    }
    let Some(services) = app.services.genai.as_ref() else {
        return Ok(());
    };
    let key = (services.load_key)(&provider)?.and_then(|k| photocraft_genai::ApiKey::new(&k));
    let Some(key) = key else {
        app.session.set_image_provider(None);
        return Err(crate::i18n::fmt(tl!("No API key is saved for {provider}."), &[("provider", &provider)]));
    };
    let built = (services.provider)(&provider, &model, key)?;
    app.session.set_image_provider(Some(built));
    Ok(())
}

/// Menu, palette and shortcut entry points. Clicking Generative Fill asks for a prompt; with
/// `prompt` already in the params (scripted use) it runs directly.
pub fn menu(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    match id {
        SETTINGS => Some(Ok(json!({"dialog": open_settings(app)}))),
        FILL if params.get("prompt").is_none() => {
            if app.session.image_provider().is_none() {
                return Some(Ok(json!({"dialog": open_settings(app)})));
            }
            if let Some(why) = app.session.disabled_reason(FILL) {
                return Some(Err(why));
            }
            Some(Ok(json!({"dialog": open_prompt(app)})))
        }
        _ => None,
    }
}

/// Is the Edit › Generative Fill item shown? Only once a provider can run it (#41: no
/// "coming soon" items).
pub fn menu_visible(app: &PhotocraftApp) -> bool {
    app.session.image_provider().is_some_and(|p| p.capabilities().edit)
}

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(SETTINGS_MARK) || fields.contains_key(PROMPT_MARK)
}

pub fn ok_label(fields: &Map<String, Value>) -> Option<&'static str> {
    if fields.contains_key(PROMPT_MARK) { Some(tl!("Generate")) } else { None }
}

pub fn open_settings(app: &mut PhotocraftApp) -> u64 {
    if let Some(d) = app.ui.dialogs.iter().find(|d| d.fields.contains_key(SETTINGS_MARK)) {
        return d.id;
    }
    app.genai.key_input.clear();
    let (mut provider, mut model) = saved(app);
    if provider.is_empty() {
        // Opening the settings is the way to turn it on: start on the one provider there is.
        provider = photocraft_genai::gemini::ID.into();
    }
    if model.is_empty() {
        model = photocraft_genai::gemini::DEFAULT_MODEL.into();
    }
    let has_key = app.services.genai.as_ref().and_then(|s| (s.load_key)(&provider).ok().flatten()).is_some();
    let mut f = Map::new();
    f.insert(SETTINGS_MARK.into(), json!(true));
    f.insert("__label".into(), json!("Generative AI Settings"));
    f.insert("provider".into(), json!(provider));
    f.insert("model".into(), json!(model));
    f.insert("hasKey".into(), json!(has_key));
    app.ui.open_dialog(DialogKind::Command, f)
}

fn open_prompt(app: &mut PhotocraftApp) -> u64 {
    if let Some(d) = app.ui.dialogs.iter().find(|d| d.fields.contains_key(PROMPT_MARK)) {
        return d.id;
    }
    let last = app.session.prefs().dialogs.get(FILL).and_then(|v| v.get("prompt")).and_then(Value::as_str).unwrap_or("").to_owned();
    let mut f = Map::new();
    f.insert(PROMPT_MARK.into(), json!(true));
    f.insert("__label".into(), json!("Generative Fill"));
    f.insert("prompt".into(), json!(last));
    app.ui.open_dialog(DialogKind::Command, f)
}

pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    if f.contains_key(PROMPT_MARK) {
        prompt_body(app, ui, f);
    } else {
        settings_body(app, ui, f);
    }
}

fn faint(ui: &mut egui::Ui, text: &str) {
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.add(egui::Label::new(egui::RichText::new(text).color(t.text_faint).size(12.0)).wrap());
}

fn prompt_body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let mut prompt = f.get("prompt").and_then(Value::as_str).unwrap_or("").to_owned();
    ui.label(tl!("Describe what to generate in the selection, or leave it empty to continue the image through it."));
    ui.add_space(6.0);
    let r = ui
        .add(egui::TextEdit::singleline(&mut prompt).hint_text(tl!("Prompt (optional)")).desired_width(f32::INFINITY).char_limit(photocraft_genai::MAX_PROMPT));
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
    ui.add_space(6.0);
    if let Some(info) = app.session.image_provider().map(|p| p.info()) {
        faint(
            ui,
            &crate::i18n::fmt(
                tl!("The selected area, the image around it and the prompt are sent to {name} ({model})."),
                &[("name", info.name), ("model", &info.model)],
            ),
        );
    }
}

fn settings_body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = crate::theme::Tokens::get(ui.ctx());
    if app.services.genai.is_none() {
        faint(ui, tl!("Generative AI needs the desktop app."));
        return;
    }
    let mut provider = f.get("provider").and_then(Value::as_str).unwrap_or("").to_owned();
    let options = providers();
    let opts: Vec<(String, &str)> = options.iter().map(|(id, name)| (id.clone(), *name)).collect();
    egui::Grid::new("genai-settings").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
        ui.label(tl!("Provider"));
        if crate::widgets::dropdown(ui, "genai-provider", tl!("Provider"), &mut provider, &opts, 220.0) {
            f.insert("provider".into(), json!(provider));
        }
        ui.end_row();
        if provider == photocraft_genai::gemini::ID {
            ui.label(tl!("Model"));
            let mut model = f.get("model").and_then(Value::as_str).unwrap_or(photocraft_genai::gemini::DEFAULT_MODEL).to_owned();
            let models: Vec<(String, &str)> = photocraft_genai::gemini::MODELS.iter().map(|(id, name)| ((*id).to_owned(), *name)).collect();
            if crate::widgets::dropdown(ui, "genai-model", tl!("Model"), &mut model, &models, 220.0) {
                f.insert("model".into(), json!(model));
            }
            ui.end_row();
            ui.label(tl!("API key"));
            let has_key = f.get("hasKey").and_then(Value::as_bool) == Some(true);
            ui.horizontal(|ui| {
                let hint = if has_key { tl!("Saved (paste to replace)") } else { tl!("Paste your Gemini API key") };
                let r = ui.add(egui::TextEdit::singleline(&mut app.genai.key_input).password(true).hint_text(hint).desired_width(220.0));
                crate::field_tab::register(ui.ctx(), r.id);
                if has_key && crate::widgets::secondary_button(ui, tl!("Remove Key"), 0.0).clicked() {
                    let removed = app.services.genai.as_ref().map(|s| (s.delete_key)(&provider));
                    match removed {
                        Some(Ok(())) => {
                            f.insert("hasKey".into(), json!(false));
                            app.session.set_image_provider(None);
                        }
                        Some(Err(e)) => app.ui.status = e,
                        None => {}
                    }
                }
            });
            ui.end_row();
        }
    });
    if provider == photocraft_genai::gemini::ID {
        ui.add_space(4.0);
        if ui.link(tl!("Get a Gemini API key from Google AI Studio")).clicked() {
            crate::links::open(app, &ui.ctx().clone(), KEY_PAGE);
        }
        ui.add_space(8.0);
        crate::widgets::hairline(ui);
        ui.add_space(6.0);
        ui.label(egui::RichText::new(tl!("What is sent, and to whom")).font(crate::theme::semibold(12.5)).color(t.text));
        ui.add_space(2.0);
        faint(
            ui,
            tl!(
                "Nothing is sent until you run Generative Fill. Each time you do, PhotoCraft sends the selected area and the image around it, a black-and-white mask of the selection, and your prompt to Google (generativelanguage.googleapis.com) with your API key. Google's Gemini API terms and privacy policy apply, and its usage is billed to your key. Results carry Google's SynthID watermark."
            ),
        );
        ui.add_space(4.0);
        faint(ui, tl!("Each Generative Fill makes three variations, so it sends three requests, each billed to your key."));
        ui.add_space(4.0);
        let store = app.services.genai.as_ref().map_or("", |s| s.key_store_name);
        faint(ui, &crate::i18n::fmt(tl!("Your key is kept in {store}, never in PhotoCraft's settings files."), &[("store", tl!(store))]));
    }
}

/// OK: remember the choice, save a newly typed key, and install the provider (or remove it).
pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    if f.contains_key(PROMPT_MARK) {
        let prompt = f.get("prompt").and_then(Value::as_str).unwrap_or("").trim().to_owned();
        app.session.prefs.edit(|p| p.dialogs.insert(FILL.into(), json!({"prompt": prompt})));
        return app.run(FILL, json!({"prompt": prompt}));
    }
    let typed = std::mem::take(&mut app.genai.key_input);
    let provider = f.get("provider").and_then(Value::as_str).unwrap_or("").to_owned();
    let model = f.get("model").and_then(Value::as_str).unwrap_or("").to_owned();
    if !provider.is_empty() {
        let services = app.services.genai.as_ref().ok_or_else(|| tl!("Generative AI needs the desktop app.").to_string())?;
        if let Some(key) = photocraft_genai::ApiKey::new(&typed) {
            // Check the provider builds (model id) before the key is stored.
            (services.provider)(&provider, &model, key.clone())?;
            (services.save_key)(&provider, key.expose())?;
        }
    }
    app.session.prefs.edit(|p| p.dialogs.insert(PREF.into(), json!({"provider": provider, "model": model})));
    restore(app)?;
    Ok(json!({"provider": provider, "model": model, "ready": app.session.image_provider().is_some()}))
}

/// Dialog closed without OK: drop whatever was typed in the key field.
pub fn cancelled(app: &mut PhotocraftApp) {
    app.genai.key_input.clear();
}

/// The Generative Fill button (and its settings arrow) for the selection tools' options bar,
/// next to Select and Mask…. The button needs a selection; with no provider set up it opens the
/// settings instead.
pub fn options_bar_button(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if app.services.genai.is_none() {
        return;
    }
    let ready = app.session.image_provider().is_some();
    let can_fill = !ready || app.session.is_enabled(FILL);
    ui.spacing_mut().item_spacing.x = 2.0;
    let tip = if ready { tl!("Fill the selection from a prompt") } else { tl!("Set up Generative Fill with your own API key") };
    let clicked = ui.add_enabled_ui(can_fill, |ui| crate::widgets::secondary_button(ui, tl!("Generative Fill…"), 0.0).on_hover_text(tip)).inner.clicked();
    if clicked {
        let _ = crate::menus::invoke(app, &ui.ctx().clone(), FILL, json!({}));
    }
    if crate::icons::button(ui, "chevron-down", 22.0, false, tl!("Generative AI Settings…")).clicked() {
        open_settings(app);
    }
    ui.spacing_mut().item_spacing.x = 8.0;
}

/// Preferences › Integrations: where Generative AI is set up.
pub fn prefs_rows(app: &PhotocraftApp, ui: &mut egui::Ui) -> bool {
    let t = crate::theme::Tokens::get(ui.ctx());
    if app.services.genai.is_none() {
        return false;
    }
    ui.label(egui::RichText::new(tl!("Generative AI")).font(crate::theme::semibold(12.5)).color(t.text));
    let status = match app.session.image_provider().map(|p| p.info()) {
        Some(info) => crate::i18n::fmt(tl!("Generative Fill uses {name} ({model})."), &[("name", info.name), ("model", &info.model)]),
        None => tl!("Off. Set up a provider with your own API key to use Generative Fill.").to_string(),
    };
    faint(ui, &status);
    ui.add_space(4.0);
    crate::widgets::secondary_button(ui, tl!("Generative AI Settings…"), 0.0).clicked()
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
    let n = s.channels();
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
    let tip = match app.session.image_provider().map(|p| p.info()) {
        Some(info) => crate::i18n::fmt(tl!("Sends the same area and prompt to {name} again for three more."), &[("name", info.name)]),
        None => app.session.disabled_reason(MORE).unwrap_or_default(),
    };
    let enabled = app.session.is_enabled(MORE);
    let mut more = false;
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        let head = ui.label(egui::RichText::new(tl!("Variations")).font(crate::theme::semibold(12.5)).color(t.text));
        head.on_hover_text(if prompt.is_empty() { tl!("No prompt: continues the image") } else { &prompt });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            more = ui.add_enabled_ui(enabled, |ui| crate::widgets::secondary_button(ui, tl!("Generate More"), 0.0).on_hover_text(tip)).inner.clicked();
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
                    let fit = ((width - 4.0) / size.x).min((height - 4.0) / size.y);
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
    if let Some(i) = chosen
        && let Err(e) = app.run(SELECT_VARIATION, json!({"layer": group_id.0, "index": i}))
    {
        app.ui.status = e;
    }
    if more && let Err(e) = app.run(MORE, json!({"layer": group_id.0})) {
        app.ui.status = e;
    }
    ui.add_space(6.0);
    crate::widgets::hairline(ui);
    ui.add_space(4.0);
}

#[cfg(test)]
#[path = "genai_ui_tests.rs"]
mod tests;
