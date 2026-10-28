//! Resolve browser-only missing fonts through the existing font installer.
//! Downloads use Google's open font repository, never a guessed third-party URL.
use photocraft_ui_egui::{PhotocraftApp, served_fonts};
use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};
use wasm_bindgen::JsCast as _;

const DATABASE: &str = "photocraft.fonts";
const ROOT: &str = "https://raw.githubusercontent.com/google/fonts/main";
const MAX_FONT: usize = 16 * 1024 * 1024;
type Files = Vec<(String, Vec<u8>)>;
type Reply = (String, Result<Option<(Files, Option<String>)>, String>);

#[derive(Default)]
pub struct Fonts {
    replies: Rc<RefCell<Vec<Reply>>>,
    pending: Vec<String>,
    messages: HashMap<String, String>,
    missing: Vec<String>,
    buttons: Vec<serde_json::Value>,
    next_scan: f64,
    hydration: Option<Files>,
}

impl Fonts {
    pub async fn load() -> Self {
        let mut fonts = Self::default();
        let loaded = async {
            let db = crate::indexed_presets::open(DATABASE, js_sys::Date::now() + 10_000.0).await?;
            let result = crate::indexed_presets::read_fonts(&db).await;
            db.close();
            result
        }
        .await;
        match loaded {
            Ok(files) => fonts.hydration = Some(files),
            Err(error) => {
                fonts.messages.insert(String::new(), format!("Retained fonts unavailable: {error}"));
            }
        }
        fonts
    }

    pub fn tick(&mut self, app: &mut PhotocraftApp, ctx: &egui::Context) {
        if let Some(files) = self.hydration.take() {
            served_fonts::install(app, files);
        }
        for (family, result) in self.replies.take() {
            self.pending.retain(|name| name != &family);
            match result {
                Ok(Some((files, warning))) => {
                    let delivered = files.into_iter().map(|(_, bytes)| (family.clone(), bytes)).collect();
                    served_fonts::install(app, delivered);
                    let message = warning.unwrap_or_else(|| format!("{family} is ready for this and future sessions."));
                    app.ui.status = message.clone();
                    app.ui.status_error = false;
                    // Remove only font warnings which no longer apply; keep other import warnings.
                    let prefix = format!("font '{family}' is not installed;");
                    app.ui.notices.retain_mut(|notice| {
                        let had_warning = notice.lines.iter().any(|line| line.starts_with(&prefix));
                        notice.lines.retain(|line| !line.starts_with(&prefix));
                        !had_warning || !notice.lines.is_empty()
                    });
                    photocraft_ui_egui::notices::post(app, message.clone(), Vec::new(), false, None);
                    self.messages.insert(family, message);
                }
                Ok(None) => {}
                Err(error) => {
                    self.messages.insert(family, error);
                }
            }
            self.next_scan = 0.0;
        }
        let now = js_sys::Date::now();
        if now >= self.next_scan {
            self.missing = app.session.active().map(|doc| photocraft_engine::type_extra_cmds::missing_fonts(&doc.doc)).unwrap_or_default();
            self.next_scan = now + 500.0;
        }
        // Also discover a just-opened document when there are no animation frames.
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }

    pub fn ui(&mut self, app: &PhotocraftApp, ui: &mut egui::Ui) {
        self.buttons.clear();
        if self.missing.is_empty() {
            return;
        }
        egui::Panel::top("browser_missing_fonts").show(ui, |ui| {
            let tokens = photocraft_ui_egui::theme::Tokens::for_kind(app.ui.theme);
            ui.colored_label(tokens.warning, "Missing fonts — download an open font or import your TTF/OTF.");
            ui.hyperlink_to("Google Fonts catalogue and licences", "https://fonts.google.com/");
            for family in self.missing.clone().into_iter().take(8) {
                let busy = self.pending.contains(&family);
                ui.horizontal_wrapped(|ui| {
                    ui.label(&family);
                    let download = ui.add_enabled(self.pending.is_empty(), egui::Button::new("Download from Google Fonts"));
                    let import = ui.add_enabled(self.pending.is_empty(), egui::Button::new("Import TTF/OTF…"));
                    let point = |r: &egui::Response| [r.rect.center().x, r.rect.center().y];
                    self.buttons.push(serde_json::json!({"family": family, "download": point(&download), "import": point(&import)}));
                    if download.clicked() {
                        self.begin(family.clone(), false, ui.ctx().clone());
                    }
                    if import.clicked() {
                        self.begin(family.clone(), true, ui.ctx().clone());
                    }
                    if busy {
                        ui.spinner();
                        ui.label("Loading font…");
                    }
                });
                if let Some(message) = self.messages.get(&family) {
                    ui.label(message);
                }
            }
            if let Some(message) = self.messages.get("") {
                ui.label(message);
            }
        });
    }

    fn begin(&mut self, family: String, import: bool, ctx: egui::Context) {
        if !self.pending.is_empty() {
            return;
        }
        self.pending.push(family.clone());
        self.messages.remove(&family);
        let replies = self.replies.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let result = async {
                let files = if import {
                    let Some(file) = rfd::AsyncFileDialog::new().add_filter("Fonts", &["ttf", "otf", "ttc", "otc"]).pick_file().await else { return Ok(None) };
                    vec![(file.file_name(), file.read().await)]
                } else {
                    download(&family).await?
                };
                validate(&family, &files)?;
                let retained = async {
                    let db = crate::indexed_presets::open(DATABASE, js_sys::Date::now() + 10_000.0).await?;
                    let result = async {
                        let existing = crate::indexed_presets::read_fonts(&db).await?;
                        let count = existing.len().saturating_add(files.len());
                        let total = existing.iter().map(|(_, bytes)| bytes.len()).chain(files.iter().map(|(_, bytes)| bytes.len())).sum::<usize>();
                        if count > 128 || total > 64 * 1024 * 1024 {
                            return Err("The retained font collection has reached its limit".to_string());
                        }
                        let batch = files.iter().map(|(name, bytes)| (format!("{family}/{name}"), Some(Arc::<[u8]>::from(bytes.as_slice())))).collect();
                        crate::indexed_presets::write(&db, &batch).await
                    }
                    .await;
                    db.close();
                    result
                }
                .await;
                let warning = retained.err().map(|error| format!("{family} is ready in this tab only; browser retention failed: {error}"));
                Ok(Some((files, warning)))
            }
            .await;
            replies.borrow_mut().push((family, result));
            ctx.request_repaint();
        });
    }
}

fn validate(family: &str, files: &Files) -> Result<(), String> {
    if family.is_empty() || family.len() > 200 || family.contains('/') || files.iter().any(|(name, _)| name.len() > 255) {
        return Err("This font identity is too long or unsupported.".into());
    }
    let mut fonts = photocraft_text::FontDb::new();
    for (_, bytes) in files {
        if bytes.is_empty() || bytes.len() > MAX_FONT {
            return Err("Font must be between 1 byte and 16 MiB.".into());
        }
        let names = fonts.register_font_data(bytes.clone());
        if !names.iter().any(|name| name.eq_ignore_ascii_case(family)) {
            return Err(format!("This font does not contain {family}. Import the matching TTF/OTF."));
        }
    }
    Ok(())
}

async fn download(family: &str) -> Result<Files, String> {
    if family.len() > 200 || !family.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '-') {
        return Err("This family is unavailable from the open catalogue. Import its TTF/OTF.".into());
    }
    let slug = family.to_ascii_lowercase().replace(' ', "");
    for license in ["ofl", "apache", "ufl"] {
        let root = format!("{ROOT}/{license}/{slug}");
        let metadata = match fetch(&format!("{root}/METADATA.pb"), 256 * 1024).await {
            Ok(Some(bytes)) => String::from_utf8(bytes).map_err(|_| "Unreadable font catalogue entry")?,
            Ok(None) => continue,
            Err(error) => return Err(format!("Download failed: {error}. Retry or import a TTF/OTF.")),
        };
        let field =
            |line: &str, key: &str| line.trim().strip_prefix(key).and_then(|s| s.trim().strip_prefix('"')).and_then(|s| s.strip_suffix('"')).map(str::to_owned);
        if !metadata.lines().find_map(|line| field(line, "name:")).is_some_and(|name| name.eq_ignore_ascii_case(family)) {
            continue;
        }
        let names: Vec<String> = metadata.lines().filter_map(|line| field(line, "filename:")).collect();
        if names.is_empty() || names.len() > 32 {
            return Err("No supported font files in this catalogue entry. Import TTF/OTF.".into());
        }
        let mut files = Vec::new();
        let mut total = 0usize;
        for name in names {
            if name.contains('/') || name.contains('\\') || name.contains("..") || !(name.ends_with(".ttf") || name.ends_with(".otf")) {
                return Err("Unsupported catalogue font file. Import TTF/OTF.".into());
            }
            let path = js_sys::encode_uri_component(&name).as_string().ok_or("Invalid font filename")?;
            let bytes = fetch(&format!("{root}/{path}"), MAX_FONT).await?.ok_or("Font download is unavailable. Retry or import TTF/OTF.")?;
            total = total.saturating_add(bytes.len());
            if total > 64 * 1024 * 1024 {
                return Err("Font family exceeds the 64 MiB limit.".into());
            }
            files.push((name, bytes));
        }
        return Ok(files);
    }
    Err("Not available from Google Fonts. Import the matching licensed TTF/OTF instead.".into())
}

async fn fetch(url: &str, max: usize) -> Result<Option<Vec<u8>>, String> {
    let window = web_sys::window().ok_or("Browser unavailable")?;
    let abort = web_sys::AbortController::new().map_err(|_| "Cannot start font request")?;
    let init = web_sys::RequestInit::new();
    init.set_signal(Some(&abort.signal()));
    let deadline = js_sys::Date::now() + 10_000.0;
    let result = async {
        let response = crate::indexed_presets::before(&window.fetch_with_str_and_init(url, &init), deadline)
            .await?
            .dyn_into::<web_sys::Response>()
            .map_err(|_| "Invalid font response")?;
        if response.status() == 404 {
            return Ok(None);
        }
        if !response.ok() {
            return Err(format!("Font service returned HTTP {}", response.status()));
        }
        let body = crate::indexed_presets::before(&response.array_buffer().map_err(|_| "Cannot read font response")?, deadline).await?;
        let data = js_sys::Uint8Array::new(&body);
        if data.length() as usize > max {
            return Err("Font response exceeds its limit".into());
        }
        Ok(Some(data.to_vec()))
    }
    .await;
    abort.abort();
    result
}
