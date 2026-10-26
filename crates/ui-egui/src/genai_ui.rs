//! Generative AI in the shell (#41): Generative AI Settings (provider, model and API key), the
//! Edit › Generative Fill prompt (Fill or Edit, with optional reference images), and installing
//! the configured provider on the session.
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
/// Most separate areas of a selection the prompt dialog offers to fill one by one (each is its
/// own Generative Fill, billed on its own).
const MAX_SEPARATE: usize = 16;
/// Long side of a reference image's preview in the prompt dialog, in texels.
const REFERENCE_THUMB: u32 = 64;
const SETTINGS_MARK: &str = "__genaiSettings";
const PROMPT_MARK: &str = "__genaiPrompt";
/// `prefs.dialogs` entry holding `{"provider", "model"}`.
const PREF: &str = "ui.generativeAiSettings";
const KEY_PAGE: &str = "https://aistudio.google.com/apikey";

mod pending;

pub use pending::{is_generative, layer_rows, layer_rows_unplaced, on_job_event, paint_canvas as paint_pending, status as status_fills};

/// Shell state that must not be serialised or inspected.
#[derive(Default)]
pub struct Runtime {
    /// What is typed in the settings dialog's key field. Cleared when the dialog closes.
    key_input: String,
    /// Variation thumbnails for the Properties panel, by variation layer: (pixels fingerprint,
    /// texture).
    thumbs: std::collections::HashMap<LayerId, (u64, egui::TextureHandle)>,
    /// The reference images of the last Generative Fill (`references` entries), offered again the
    /// next time the prompt opens. Kept for this session only.
    references: Vec<Value>,
    /// Previews of reference images, by entry ([`reference_key`]).
    previews: std::collections::HashMap<String, Preview>,
    /// Fills in progress, as drawn ([`pending`]).
    pending: pending::Pending,
}

/// A reference image's preview: decoded when it was added, uploaded on first draw.
enum Preview {
    Image(egui::ColorImage),
    Texture(egui::TextureHandle),
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
    let last = app.session.prefs().dialogs.get(FILL).cloned().unwrap_or(Value::Null);
    let get = |k: &str| last.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    let mode = if get("mode") == "edit" { "edit" } else { "fill" };
    let separate = last.get("separate").and_then(Value::as_bool).unwrap_or(true);
    let areas = photocraft_engine::genai_cmds::selection_areas(&app.session).len();
    let references: Vec<Value> = app.genai.references.iter().filter(|r| reference_name(app, r).is_some()).cloned().collect();
    // Previews of references no longer offered go; an open document's is made afresh.
    app.genai.previews.retain(|key, _| references.iter().any(|r| reference_key(r) == *key && r.get("path").is_some()));
    let mut f = Map::new();
    f.insert(PROMPT_MARK.into(), json!(true));
    f.insert("__label".into(), json!("Generative Fill"));
    f.insert("prompt".into(), json!(get("prompt")));
    f.insert("mode".into(), json!(mode));
    f.insert("references".into(), Value::Array(references));
    f.insert("separate".into(), json!(separate));
    f.insert("__areas".into(), json!(areas));
    app.ui.open_dialog(DialogKind::Command, f)
}

/// What the configured provider can do in the prompt dialog: (edit in place, most references).
fn prompt_capabilities(app: &PhotocraftApp) -> (bool, usize) {
    let caps = app.session.image_provider().map(|p| p.capabilities()).unwrap_or_default();
    (caps.edit_in_place, caps.max_references.min(photocraft_genai::MAX_REFERENCES))
}

/// A `references` entry's identity, for its preview.
fn reference_key(r: &Value) -> String {
    r.to_string()
}

/// What a `references` entry is called: its file's name, or its open document's (`None` once that
/// document is closed).
fn reference_name(app: &PhotocraftApp, r: &Value) -> Option<String> {
    if let Some(path) = r.get("path").and_then(Value::as_str) {
        let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
        return Some(name.to_owned());
    }
    let id = r.get("document")?.as_u64()?;
    app.session.documents().iter().find(|d| d.doc.id.0 == id).map(|d| d.doc.name.clone())
}

/// The preview of a reference image, at most [`REFERENCE_THUMB`] texels on its long side.
fn preview_image(doc: &photocraft_doc::Document) -> Result<egui::ColorImage, String> {
    let (w, h, rgb) = photocraft_engine::genai_cmds::reference_pixels(doc, REFERENCE_THUMB).map_err(|e| e.to_string())?;
    Ok(egui::ColorImage::from_rgb([w as usize, h as usize], &rgb))
}

fn references_of(f: &Map<String, Value>) -> Vec<Value> {
    f.get("references").and_then(Value::as_array).cloned().unwrap_or_default()
}

/// Add a reference image to the open prompt dialog (if it is still open and has room).
fn add_reference(app: &mut PhotocraftApp, entry: Value, preview: Option<egui::ColorImage>) {
    let (_, max) = prompt_capabilities(app);
    let Some(d) = app.ui.dialogs.iter_mut().find(|d| d.fields.contains_key(PROMPT_MARK)) else { return };
    let mut refs = references_of(&d.fields);
    if refs.len() >= max || refs.contains(&entry) {
        return;
    }
    if let Some(img) = preview {
        app.genai.previews.insert(reference_key(&entry), Preview::Image(img));
    }
    refs.push(entry);
    d.fields.insert("references".into(), Value::Array(refs));
}

/// Image File…: pick a file, check it opens (so a bad one is reported now, not at Generate) and
/// add it by path; the engine reads it again when the fill runs.
fn pick_reference_file(app: &mut PhotocraftApp) -> Result<Value, String> {
    app.pick_file_bytes(|app, path, bytes| {
        let doc = photocraft_engine::genai_cmds::reference_document(&path, &bytes).map_err(|e| e.to_string())?;
        let preview = preview_image(&doc).ok();
        add_reference(app, json!({"path": path}), preview);
        Ok(Value::Null)
    })
}

/// The texture of a reference's preview, making it (for an open document) or uploading it on
/// first use.
fn preview_texture(app: &mut PhotocraftApp, ctx: &egui::Context, r: &Value) -> Option<(egui::TextureId, egui::Vec2)> {
    let key = reference_key(r);
    if !app.genai.previews.contains_key(&key) {
        let id = r.get("document").and_then(Value::as_u64)?;
        let doc = app.session.documents().iter().find(|d| d.doc.id.0 == id).map(|d| d.doc.clone())?;
        app.genai.previews.insert(key.clone(), Preview::Image(preview_image(&doc).ok()?));
    }
    let preview = app.genai.previews.get_mut(&key)?;
    if let Preview::Image(img) = preview {
        let tex = ctx.load_texture(format!("genai-reference-{key}"), std::mem::take(img), egui::TextureOptions::LINEAR);
        *preview = Preview::Texture(tex);
    }
    match preview {
        Preview::Texture(tex) => Some((tex.id(), tex.size_vec2())),
        Preview::Image(_) => None,
    }
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

/// Fill / Edit, as tabs: Fill blurs the selection out and generates into it (or removes what was
/// there), Edit keeps what is there and changes what the prompt says.
fn mode_tabs(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let edit = f.get("mode").and_then(Value::as_str) == Some("edit");
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (mode, label, tip, on) in [
            ("fill", tl!("Fill selection"), tl!("Generate new content in the selection, or remove what is there"), !edit),
            ("edit", tl!("Edit selection"), tl!("Keep what is in the selection and change only what you ask for"), edit),
        ] {
            let r = crate::widgets::pill_tab(ui, label, on);
            r.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, label));
            if r.on_hover_text(tip).clicked() {
                f.insert("mode".into(), json!(mode));
            }
        }
    });
}

/// Reference images: a preview per image with a remove button, and Add Reference (an image file
/// or an open document).
fn references_section(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>, max: usize) {
    let t = crate::theme::Tokens::get(ui.ctx());
    // An open document that was closed since it was added is dropped.
    let mut refs: Vec<Value> = references_of(f).into_iter().filter(|r| reference_name(app, r).is_some()).collect();
    ui.label(egui::RichText::new(tl!("Reference images")).font(crate::theme::semibold(12.5)).color(t.text));
    faint(ui, tl!("Optional: an object or product to show, or a design, colour or look to match."));
    ui.add_space(4.0);
    let ctx = ui.ctx().clone();
    let mut remove = None;
    let mut pick_file = false;
    let mut add_doc = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
        for (i, r) in refs.iter().enumerate() {
            let name = reference_name(app, r).unwrap_or_default();
            let tex = preview_texture(app, &ctx, r);
            egui::Frame::new().fill(t.field).corner_radius(t.radius_sm).inner_margin(egui::Margin::same(3)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(36.0, 36.0), egui::Sense::hover());
                    if let Some((tex, size)) = tex {
                        let fit = (rect.width() / size.x.max(1.0)).min(rect.height() / size.y.max(1.0));
                        let img = egui::Rect::from_center_size(rect.center(), size * fit);
                        ui.painter().image(tex, img, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                    }
                    let short: String = if name.chars().count() > 22 { format!("{}…", name.chars().take(21).collect::<String>()) } else { name.clone() };
                    ui.label(egui::RichText::new(short).size(12.0).color(t.text)).on_hover_text(&name);
                    if crate::icons::button(ui, "x", 20.0, false, tl!("Remove")).clicked() {
                        remove = Some(i);
                    }
                });
            });
        }
        if refs.len() < max {
            let add = crate::widgets::secondary_button(ui, tl!("Add Reference…"), 0.0);
            egui::Popup::menu(&add).show(|ui| {
                crate::widgets::style_spectrum_popup_menu(ui);
                ui.set_min_width(200.0);
                if ui.button(tl!("Image File…")).clicked() {
                    pick_file = true;
                    ui.close();
                }
                let docs: Vec<(u64, String)> = app.session.documents().iter().map(|d| (d.doc.id.0, d.doc.name.clone())).collect();
                if !docs.is_empty() {
                    ui.separator();
                    ui.label(egui::RichText::new(tl!("Open Documents")).size(11.5).color(t.text_faint));
                    for (id, name) in docs {
                        let entry = json!({"document": id});
                        if ui.add_enabled(!refs.contains(&entry), egui::Button::new(name)).clicked() {
                            add_doc = Some(entry);
                            ui.close();
                        }
                    }
                }
            });
        }
    });
    if let Some(i) = remove
        && i < refs.len()
    {
        refs.remove(i);
    }
    if let Some(entry) = add_doc
        && refs.len() < max
        && !refs.contains(&entry)
    {
        refs.push(entry);
    }
    if references_of(f) != refs {
        f.insert("references".into(), Value::Array(refs));
    }
    if pick_file && let Err(e) = pick_reference_file(app) {
        app.ui.status = e;
    }
}

fn prompt_body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let (can_edit, max_references) = prompt_capabilities(app);
    let mut prompt = f.get("prompt").and_then(Value::as_str).unwrap_or("").to_owned();
    if can_edit {
        mode_tabs(ui, f);
        ui.add_space(8.0);
    }
    let edit = can_edit && f.get("mode").and_then(Value::as_str) == Some("edit");
    if edit {
        ui.label(tl!("Describe what to change in the selection: what is there is kept, except what you ask to change (a colour, a text, a material)."));
    } else {
        ui.label(tl!("Describe what to generate in the selection, or leave it empty to continue the image through it."));
    }
    ui.add_space(6.0);
    let hint = if edit { tl!("What to change") } else { tl!("Prompt (optional)") };
    let r = ui.add(egui::TextEdit::singleline(&mut prompt).hint_text(hint).desired_width(f32::INFINITY).char_limit(photocraft_genai::MAX_PROMPT));
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
    // An empty prompt always runs Fill: Edit needs something to change.
    if prompt.trim().is_empty() {
        ui.add_space(2.0);
        if references_of(f).is_empty() {
            faint(ui, tl!("Empty: removes what is selected and continues the image through it."));
        } else {
            faint(ui, tl!("Empty: places what the reference images show in the selection."));
        }
    }
    if max_references > 0 {
        ui.add_space(10.0);
        references_section(app, ui, f, max_references);
    }
    areas_section(ui, f);
    ui.add_space(8.0);
    if let Some(info) = app.session.image_provider().map(|p| p.info()) {
        faint(
            ui,
            &crate::i18n::fmt(
                tl!("The selected area, the image around it and the prompt are sent to {name} ({model})."),
                &[("name", info.name), ("model", &info.model)],
            ),
        );
        if !references_of(f).is_empty() {
            faint(ui, tl!("The reference images are sent too, at most 1024 pixels on their long side."));
        }
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
        faint(ui, tl!("In Edit mode the selected area is sent as it is. Reference images you add are sent too."));
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
        let (can_edit, max_references) = prompt_capabilities(app);
        let chosen = if can_edit && f.get("mode").and_then(Value::as_str) == Some("edit") { "edit" } else { "fill" };
        let references: Vec<Value> = references_of(f).into_iter().filter(|r| reference_name(app, r).is_some()).take(max_references).collect();
        let separate = f.get("separate").and_then(Value::as_bool).unwrap_or(true);
        let areas = f.get("__areas").and_then(Value::as_u64).unwrap_or(0) as usize;
        app.session.prefs.edit(|p| p.dialogs.insert(FILL.into(), json!({"prompt": prompt, "mode": chosen, "separate": separate})));
        app.genai.references = references.clone();
        // Edit needs something to change: with no prompt it removes, as Fill does.
        let mode = if prompt.is_empty() { "fill" } else { chosen };
        let mut params = Map::new();
        params.insert("prompt".into(), json!(prompt));
        params.insert("mode".into(), json!(mode));
        if !references.is_empty() {
            params.insert("references".into(), Value::Array(references));
        }
        let r = if separate && (2..=MAX_SEPARATE).contains(&areas) {
            // One fill per area, side by side.
            let mut last = Ok(Value::Null);
            for k in 0..areas {
                let mut p = params.clone();
                p.insert("area".into(), json!(k));
                last = app.run(FILL, Value::Object(p));
                if last.is_err() {
                    break;
                }
            }
            last
        } else {
            app.run(FILL, Value::Object(params))
        };
        // In the background the fill took what it needs: let the selection go, so the next
        // area can be selected while it generates.
        if r.is_ok() && app.background_jobs {
            let _ = app.run("select.deselect", json!({}));
        }
        return r;
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
