//! File › Send to Telegram: post the saved or exported file to a chat the user configured.
//!
//! The bot token never goes through preferences or the control channel. It lives in a separate
//! settings file the desktop app reads and writes (`Services::load_telegram` /
//! `save_telegram`). Copy and `prefs.get` therefore cannot see it. Sending is a shell command:
//! the engine has no network, and plug-ins cannot open one.

use serde_json::{Value, json};

use crate::PhotocraftApp;

mod protocol;

#[cfg(not(target_arch = "wasm32"))]
use protocol::{read_error, send_to_chats};

use protocol::{append_chat, caption, chat_selected, file_name, file_secs, png_preview_for, read_file, remember_sent_files, send_report};
pub use protocol::{
    chats_from_updates, files_from_updates, first_message_id, media_group, message_link, multipart, parse_chats, redact, sent_files_from_reply,
    sent_message_id, utc_stamp, validate_chat, validate_token, with_token, without_token,
};

/// Telegram Bot API `sendDocument` limit for a bot.
pub const MAX_BYTES: u64 = 50 * 1024 * 1024;

/// Which write just finished, so each automatic send can be switched on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteKind {
    Save,
    SaveAs,
    /// File › Save a Copy. The document's own path stays unchanged.
    SaveCopy,
    Export,
}

/// What the user stored for sending. Empty token or chat means "not set up yet".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    pub bot_token: String,
    pub chat_id: String,
    /// After File › Save. Off until the user opts in.
    pub send_on_save: bool,
    /// After File › Save As. Off until the user opts in.
    pub send_on_save_as: bool,
    /// After an export. Off until the user opts in.
    pub send_on_export: bool,
    /// After File › Save a Copy. Off until the user opts in.
    pub send_on_save_copy: bool,
    /// When the file sent is a PSD, also send a PNG of the same document.
    pub also_png: bool,
    /// Pin the sent message. Off until the user opts in. Needs the bot to be an admin.
    pub pin_message: bool,
    /// Files this app sent, newest first, so they can be opened again. The bot token is not here.
    pub sent_files: Vec<Value>,
}

impl Settings {
    fn wants(&self, kind: WriteKind) -> bool {
        match kind {
            WriteKind::Save => self.send_on_save,
            WriteKind::SaveAs => self.send_on_save_as,
            WriteKind::SaveCopy => self.send_on_save_copy,
            WriteKind::Export => self.send_on_export,
        }
    }
}

impl Settings {
    pub fn ready(&self) -> bool {
        validate_token(&self.bot_token).is_ok() && parse_chats(&self.chat_id).is_ok()
    }
}

/// Parse the settings file. A missing or broken file is "not set up", never an error.
pub fn settings_from_json(text: &str) -> Settings {
    let Ok(v) = serde_json::from_str::<Value>(text) else { return Settings::default() };
    Settings {
        bot_token: v.get("botToken").and_then(Value::as_str).unwrap_or("").trim().to_string(),
        chat_id: v.get("chatId").and_then(Value::as_str).unwrap_or("").trim().to_string(),
        send_on_save: v.get("sendOnSave").and_then(Value::as_bool).unwrap_or(false),
        // Older files had one switch for Save As and Export together.
        send_on_save_as: v.get("sendOnSaveAs").and_then(Value::as_bool).unwrap_or_else(|| v.get("sendAfterExport").and_then(Value::as_bool).unwrap_or(false)),
        send_on_export: v.get("sendOnExport").and_then(Value::as_bool).unwrap_or_else(|| v.get("sendAfterExport").and_then(Value::as_bool).unwrap_or(false)),
        send_on_save_copy: v.get("sendOnSaveCopy").and_then(Value::as_bool).unwrap_or(false),
        also_png: v.get("alsoPng").and_then(Value::as_bool).unwrap_or(false),
        pin_message: v.get("pinMessage").and_then(Value::as_bool).unwrap_or(false),
        sent_files: v.get("sentFiles").and_then(Value::as_array).cloned().unwrap_or_default(),
    }
}

fn settings_json(s: &Settings) -> String {
    serde_json::to_string(&json!({
        "botToken": s.bot_token,
        "chatId": s.chat_id,
        "sendOnSave": s.send_on_save,
        "sendOnSaveAs": s.send_on_save_as,
        "sendOnExport": s.send_on_export,
        "sendOnSaveCopy": s.send_on_save_copy,
        "alsoPng": s.also_png,
        "pinMessage": s.pin_message,
        "sentFiles": s.sent_files,
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

/// Official BotFather walkthrough. Chat ids come from `getUpdates` (the Find chats button).
pub const BOT_TUTORIAL: &str = "https://core.telegram.org/bots/tutorial";

/// File › Send to Telegram…. Opens the setup dialog when the token or chat is missing.
/// A document that has not been saved is flattened to a temporary PNG and sent from a
/// background thread, so the window stays usable while Telegram answers.
pub fn command(app: &mut PhotocraftApp) -> Result<Value, String> {
    let pending = app.session.active().and_then(|d| d.path.clone());
    Ok(json!({"dialog": open_dialog(app, pending)}))
}

/// Save, Save As and Export call this after the file is on disk. Each kind sends only when
/// its own switch is on. A missing setup opens the dialog and leaves the saved file alone.
pub fn after_written(app: &mut PhotocraftApp, path: &str, kind: WriteKind) {
    let source = app.session.active().map(|st| st.doc.clone());
    after_written_for(app, path, kind, source.as_deref());
}

pub(crate) fn after_written_for(app: &mut PhotocraftApp, path: &str, kind: WriteKind, source: Option<&photocraft_doc::Document>) {
    if !app.telegram.wants(kind) {
        return;
    }
    if !app.telegram.ready() {
        if !dialog_open(app) {
            let id = open_dialog(app, Some(path.to_string()));
            if let Some(dialog) = app.ui.dialog_mut(id) {
                if let Some(source) = source {
                    dialog.fields.insert("__document".into(), json!(source.id.0));
                } else {
                    dialog.fields.remove("__document");
                    dialog.fields.insert("__missingSource".into(), json!(true));
                }
            }
        }
        return;
    }
    let chats = match parse_chats(&app.telegram.chat_id) {
        Ok(chats) => chats,
        Err(error) => {
            app.ui.status = error;
            app.ui.status_error = true;
            return;
        }
    };
    let preview = match png_preview_for(app, path, source) {
        Ok(preview) => preview,
        Err(error) => {
            app.ui.status = error;
            app.ui.status_error = true;
            return;
        }
    };
    if let Err(error) = start_send(app, path.to_string(), false, chats, preview) {
        app.ui.status = error;
        app.ui.status_error = true;
    }
}

/// Apply a finished background send. Keeps requesting frames until Telegram answers.
pub fn poll(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(job) = app.telegram_job.as_ref() else { return };
    let finished = match job.rx.try_recv() {
        Ok(result) => Some(result),
        Err(std::sync::mpsc::TryRecvError::Empty) => {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err("The Telegram send stopped before it finished.".to_string())),
    };
    let Some(job) = app.telegram_job.take() else { return };
    if job.temporary {
        remove_temp_path(&job.path);
    }
    if let Some(preview) = &job.preview {
        remove_temp_path(preview);
    }
    match finished {
        Some(Ok(body)) => {
            if let Some(path) = body.strip_prefix("OPEN\t") {
                let opened = std::fs::read(path).ok();
                let name = file_name(path);
                remove_temp_path(path);
                if let Some(bytes) = opened {
                    if let Err(error) = app.open_bytes(&name, &bytes) {
                        app.ui.status = error;
                        app.ui.status_error = true;
                    } else {
                        app.ui.status = format!("Opened {name}.");
                        app.ui.status_error = false;
                    }
                } else {
                    app.ui.status = format!("Could not read {name}.");
                    app.ui.status_error = true;
                }
                return;
            }
            if let Some(json) = body.strip_prefix("CHATS\t") {
                if let Some(dialog) = app.ui.dialogs.iter_mut().find(|dialog| owns(&dialog.fields)) {
                    match serde_json::from_str::<Value>(json) {
                        Ok(chats) => {
                            dialog.fields.insert("found".into(), chats);
                            dialog.fields.remove("findError");
                            app.ui.status = "Found chats.".to_string();
                            app.ui.status_error = false;
                        }
                        Err(_) => {
                            dialog.fields.insert("findError".into(), json!("Telegram did not return chats."));
                        }
                    }
                }
                return;
            }
            if let Some(json) = body.strip_prefix("INBOX\t") {
                if let Some(dialog) = app.ui.dialogs.iter_mut().find(|dialog| owns(&dialog.fields)) {
                    match serde_json::from_str::<Value>(json) {
                        Ok(files) => {
                            dialog.fields.insert("inbox".into(), files);
                            dialog.fields.remove("findError");
                            app.ui.status = "Loaded files.".to_string();
                            app.ui.status_error = false;
                        }
                        Err(_) => {
                            dialog.fields.insert("findError".into(), json!("Telegram did not return files."));
                        }
                    }
                }
                return;
            }
            let report = send_report(&body);
            let url = report.urls.first().cloned();
            let links = if report.urls.is_empty() { tl!("Telegram did not return a link for this chat.").to_string() } else { report.urls.join("\n") };
            let mut status = crate::i18n::fmt(tl!("Sent {name}. {url}"), &[("name", &job.name), ("url", &links)]);
            if report.pin_failed {
                let note = tl!("Could not pin the message. The file was still sent.").to_string();
                status = format!("{status}\n{note}");
            }
            remember_sent_files(app, &report.files);
            app.ui.status = status.clone();
            app.ui.status_error = false;
            show_result(app, status, url);
        }
        Some(Err(error)) => {
            let message = redact(&error, &job.token);
            app.ui.status = message.clone();
            app.ui.status_error = true;
            show_result(app, message, None);
        }
        None => {}
    }
}

pub fn owns(fields: &serde_json::Map<String, Value>) -> bool {
    fields.get("__telegram").and_then(Value::as_bool) == Some(true)
}

pub fn result_owns(fields: &serde_json::Map<String, Value>) -> bool {
    fields.get("__telegram_result").and_then(Value::as_bool) == Some(true)
}

pub fn result_body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &serde_json::Map<String, Value>) {
    ui.set_min_width(420.0);
    if let Some(message) = f.get("message").and_then(Value::as_str) {
        ui.label(message);
        ui.add_space(8.0);
    }
    let Some(url) = f.get("url").and_then(Value::as_str) else { return };
    ui.hyperlink(url);
    ui.add_space(4.0);
    if crate::widgets::secondary_button(ui, tl!("Open the Telegram message"), 220.0).clicked() {
        crate::links::open(app, ui.ctx(), url);
    }
}

fn show_result(app: &mut PhotocraftApp, message: String, url: Option<String>) {
    let mut fields = serde_json::Map::new();
    fields.insert("__telegram_result".into(), json!(true));
    fields.insert("__label".into(), json!("Telegram"));
    fields.insert("message".into(), json!(message));
    if let Some(url) = url {
        fields.insert("url".into(), json!(url));
    }
    app.ui.open_dialog(crate::state::DialogKind::Command, fields);
}

fn dialog_open(app: &PhotocraftApp) -> bool {
    app.ui.dialogs.iter().any(|d| owns(&d.fields))
}

fn open_dialog(app: &mut PhotocraftApp, pending: Option<String>) -> u64 {
    let mut fields = serde_json::Map::new();
    fields.insert("__telegram".into(), json!(true));
    if let Some(st) = app.session.active() {
        fields.insert("__document".into(), json!(st.doc.id.0));
    }
    fields.insert("__label".into(), json!("Send to Telegram"));
    app.telegram_token_draft.clone_from(&app.telegram.bot_token);
    fields.insert("tokenSet".into(), json!(!app.telegram_token_draft.is_empty()));
    fields.insert("chat".into(), json!(app.telegram.chat_id));
    fields.insert("onSave".into(), json!(app.telegram.send_on_save));
    fields.insert("onSaveAs".into(), json!(app.telegram.send_on_save_as));
    fields.insert("onExport".into(), json!(app.telegram.send_on_export));
    fields.insert("onSaveCopy".into(), json!(app.telegram.send_on_save_copy));
    fields.insert("alsoPng".into(), json!(app.telegram.also_png));
    fields.insert("pin".into(), json!(app.telegram.pin_message));
    let saved = pending.as_ref().is_some_and(|path| std::fs::metadata(path).is_ok_and(|meta| meta.is_file()))
        || app.session.active().and_then(|d| d.path.as_deref()).is_some_and(|path| std::fs::metadata(path).is_ok_and(|meta| meta.is_file()));
    fields.insert("format".into(), json!(if saved { "file" } else { "psd" }));
    if let Some(path) = pending {
        fields.insert("pending".into(), json!(path));
    }
    app.ui.open_dialog(crate::state::DialogKind::Command, fields)
}

pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut serde_json::Map<String, Value>) {
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.set_min_width(440.0);
    ui.label(egui::RichText::new(tl!("Sending needs an internet connection.")).color(t.text_dim));
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(tl!(
            "Create a bot with BotFather, then send that bot any message. Use Find chats to fill the id, or paste a number or @channelname."
        ))
        .color(t.text_dim),
    );
    ui.add_space(4.0);
    if crate::widgets::secondary_button(ui, tl!("Open the official bot guide"), 220.0).clicked() {
        crate::links::open(app, ui.ctx(), BOT_TUTORIAL);
    }
    ui.add_space(8.0);
    ui.label(tl!("Bot token"));
    let mut token = app.telegram_token_draft.clone();
    if ui.add(egui::TextEdit::singleline(&mut token).password(true).desired_width(ui.available_width())).changed() {
        app.telegram_token_draft.clone_from(&token);
        f.insert("tokenSet".into(), json!(!token.is_empty()));
    }
    ui.label(egui::RichText::new(tl!("The bot token is kept in the system keychain, not in PhotoCraft's settings file.")).color(t.text_dim));
    ui.add_space(4.0);
    if crate::widgets::secondary_button(ui, tl!("Find chats for this token"), 220.0).clicked() {
        if let Err(error) = start_find_chats(app, &token) {
            f.insert("findError".into(), json!(error));
        } else {
            f.remove("findError");
        }
    }
    if let Some(error) = f.get("findError").and_then(Value::as_str) {
        ui.label(egui::RichText::new(error).color(t.warning));
    }
    if let Some(chats) = f.get("found").and_then(Value::as_array).cloned() {
        if chats.is_empty() {
            ui.label(tl!("No chats yet. Send your bot a message, then try again."));
        }
        for chat in chats {
            let id = chat.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            let label = chat.get("label").and_then(Value::as_str).unwrap_or(&id).to_string();
            if id.is_empty() {
                continue;
            }
            if ui.selectable_label(chat_selected(f.get("chat").and_then(Value::as_str).unwrap_or(""), &id), label).clicked() {
                f.insert("chat".into(), json!(append_chat(f.get("chat").and_then(Value::as_str).unwrap_or(""), &id)));
                ui.ctx().copy_text(id);
                app.ui.status = "Copied the chat id.".to_string();
                app.ui.status_error = false;
            }
        }
    }
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(tl!("Files sent from PhotoCraft, and the ten newest files sent to this bot. Click one to open it in PhotoCraft."))
            .color(t.text_dim),
    );
    ui.add_space(4.0);
    if crate::widgets::secondary_button(ui, tl!("Get files"), 160.0).clicked() {
        if let Err(error) = start_fetch_inbox(app, &token) {
            f.insert("findError".into(), json!(error));
        } else {
            f.remove("findError");
        }
    }
    if let Some(files) = f.get("inbox").and_then(Value::as_array).cloned() {
        if files.is_empty() {
            ui.label(tl!("No files yet. Send one from here, or send the bot a file, then try again."));
        }
        for file in files {
            let id = file.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            let name = file.get("name").and_then(Value::as_str).unwrap_or("file").to_string();
            let label = file.get("label").and_then(Value::as_str).unwrap_or(&name).to_string();
            if id.is_empty() {
                continue;
            }
            if ui.selectable_label(false, label).clicked()
                && let Err(error) = start_download(app, &token, &id, &name)
            {
                app.ui.status = error;
                app.ui.status_error = true;
            }
        }
    }
    ui.add_space(8.0);
    ui.label(tl!("Chats or channels. Several are allowed, separated by commas."));
    let mut chat = f.get("chat").and_then(Value::as_str).unwrap_or("").to_string();
    if ui.add(egui::TextEdit::singleline(&mut chat).desired_width(ui.available_width())).changed() {
        f.insert("chat".into(), json!(chat));
    }
    ui.add_space(8.0);
    ui.label(tl!("Send as"));
    let format = f.get("format").and_then(Value::as_str).unwrap_or("psd").to_string();
    ui.horizontal(|ui| {
        for (id, label) in [("file", tl!("Saved file")), ("psd", "PSD"), ("png", "PNG")] {
            if ui.selectable_label(format == id, label).clicked() {
                f.insert("format".into(), json!(id));
            }
        }
    });
    ui.label(
        egui::RichText::new(tl!("Saved file sends the copy on disk. PSD and PNG are a new copy of the open document, including changes you have not saved."))
            .color(t.text_dim),
    );
    let mut also_png = f.get("alsoPng").and_then(Value::as_bool).unwrap_or(false);
    if crate::widgets::checkbox(ui, &mut also_png, tl!("Also send a PNG preview with a PSD")).changed() {
        f.insert("alsoPng".into(), json!(also_png));
    }
    let mut pin = f.get("pin").and_then(Value::as_bool).unwrap_or(false);
    if crate::widgets::checkbox(ui, &mut pin, tl!("Pin the message")).changed() {
        f.insert("pin".into(), json!(pin));
    }
    ui.label(egui::RichText::new(tl!("Pinning needs the bot to be an admin. If it is not, the file is still sent.")).color(t.text_dim));
    ui.add_space(8.0);
    crate::widgets::section_label(ui, "Send automatically");
    ui.add_space(4.0);
    for (key, label) in [
        ("onSave", tl!("Also send when I Save")),
        ("onSaveAs", tl!("Also send when I Save As")),
        ("onSaveCopy", tl!("Also send when I Save a Copy")),
        ("onExport", tl!("Also send when I Export")),
    ] {
        let mut on = f.get(key).and_then(Value::as_bool).unwrap_or(false);
        if crate::widgets::checkbox(ui, &mut on, label).changed() {
            f.insert(key.into(), json!(on));
        }
    }
}

pub fn validate(app: &PhotocraftApp, f: &serde_json::Map<String, Value>) -> Result<(), String> {
    validate_token(app.telegram_token_draft.trim())?;
    parse_chats(f.get("chat").and_then(Value::as_str).unwrap_or("").trim())?;
    Ok(())
}

pub fn confirm(app: &mut PhotocraftApp, f: &serde_json::Map<String, Value>) -> Result<Value, String> {
    validate(app, f)?;
    let source = if f.get("__missingSource").and_then(Value::as_bool) == Some(true) {
        None
    } else if let Some(id) = f.get("__document").and_then(Value::as_u64) {
        app.session.documents().iter().find(|st| st.doc.id.0 == id).map(|st| st.doc.clone())
    } else {
        app.session.active().map(|st| st.doc.clone())
    };
    let token = app.telegram_token_draft.trim().to_string();
    let chat = f.get("chat").and_then(Value::as_str).unwrap_or("").trim().to_string();
    validate_token(&token)?;
    let chats = parse_chats(&chat)?;
    let format = f.get("format").and_then(Value::as_str).unwrap_or("psd").to_string();
    let next = Settings {
        bot_token: token,
        chat_id: chat,
        send_on_save: f.get("onSave").and_then(Value::as_bool).unwrap_or(false),
        send_on_save_as: f.get("onSaveAs").and_then(Value::as_bool).unwrap_or(false),
        send_on_export: f.get("onExport").and_then(Value::as_bool).unwrap_or(false),
        send_on_save_copy: f.get("onSaveCopy").and_then(Value::as_bool).unwrap_or(false),
        also_png: f.get("alsoPng").and_then(Value::as_bool).unwrap_or(false),
        pin_message: f.get("pin").and_then(Value::as_bool).unwrap_or(false),
        sent_files: app.telegram.sent_files.clone(),
    };
    let text = settings_json(&next);
    let pending = f.get("pending").and_then(Value::as_str).map(str::to_string);
    let Some(save) = app.services.save_telegram.as_mut() else {
        return Err("Telegram sending needs the desktop app.".to_string());
    };
    save(&text)?;
    app.telegram = next;
    app.telegram_token_draft.clear();
    if app.session.active().is_none() && pending.is_none() {
        app.ui.status = "Telegram is set up. Save or export a file, then send it.".to_string();
        app.ui.status_error = false;
        return Ok(Value::Null);
    }
    let (path, temporary) = resolve_payload(app, &format, pending, source.as_deref())?;
    let preview = png_preview_for(app, &path, source.as_deref())?;
    let name = file_name(&path);
    start_send(app, path, temporary, chats, preview)?;
    Ok(json!({"sending": true, "name": name}))
}

/// A send whose request is still on the network. Not UI state: it holds the token only so an
/// error can be redacted.
pub struct Job {
    rx: std::sync::mpsc::Receiver<Result<String, String>>,
    path: String,
    temporary: bool,
    /// A PNG preview written only for this send. Always removed when the send ends.
    preview: Option<String>,
    name: String,
    token: String,
}

/// Arm Telegram for a save that may still be writing in the background. A finished write sends
/// now; a pending one sends when the worker reports the file is on disk.
pub fn arm_after_save(app: &mut PhotocraftApp, path: &str, kind: WriteKind, result: &Value) {
    if result.get("pending").and_then(Value::as_bool) == Some(true) {
        if let Some(save) = app.jobs.saves.iter_mut().rev().find(|save| save.path == path && !save.ended) {
            save.notify_telegram = Some(kind);
        }
        return;
    }
    after_written(app, path, kind);
}

fn forget_unsent(temporary: bool, path: &str, preview: &Option<String>) {
    if temporary {
        remove_temp_path(path);
    }
    if let Some(preview) = preview {
        remove_temp_path(preview);
    }
}

fn start_send(app: &mut PhotocraftApp, path: String, temporary: bool, chats: Vec<String>, preview: Option<String>) -> Result<(), String> {
    if let Err(error) = authorize_network(app, "send") {
        forget_unsent(temporary, &path, &preview);
        return Err(error);
    }
    if app.telegram_job.is_some() {
        if temporary {
            remove_temp_path(&path);
        }
        if let Some(preview) = &preview {
            remove_temp_path(preview);
        }
        return Err("A Telegram send is already running.".to_string());
    }
    let Some(send) = app.services.telegram_send.clone() else {
        forget_unsent(temporary, &path, &preview);
        return Err("Telegram sending needs the desktop app.".to_string());
    };
    let (name, bytes) = match read_file(&path) {
        Ok(read) => read,
        Err(error) => {
            forget_unsent(temporary, &path, &preview);
            return Err(error);
        }
    };
    let preview_file = match preview.as_deref() {
        Some(preview_path) => {
            let (preview_name, preview_bytes) = match read_file(preview_path) {
                Ok(read) => read,
                Err(error) => {
                    forget_unsent(temporary, &path, &preview);
                    return Err(error);
                }
            };
            let preview_caption = caption(preview_path, &preview_name, preview_bytes.len() as u64, file_secs(preview_path));
            Some((preview_name, preview_bytes, preview_caption))
        }
        None => None,
    };
    let caption = caption(&path, &name, bytes.len() as u64, file_secs(&path));
    let token = app.telegram.bot_token.clone();
    let pin = app.telegram.pin_message;
    let (tx, _rx) = std::sync::mpsc::channel::<Result<String, String>>();
    let _report_token = token.clone();
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (send, bytes, caption, chats, _report_token, tx, preview_file, pin);
        forget_unsent(temporary, &path, &preview);
        Err("Telegram sending needs the desktop app.".to_string())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::Builder::new()
            .name("pc-telegram".into())
            .spawn({
                let name = name.clone();
                move || {
                    let result = send_to_chats(&send, &_report_token, &chats, &caption, &name, &bytes, preview_file.as_ref(), pin);
                    let _ = tx.send(result);
                }
            })
            .map_err(|error| {
                forget_unsent(temporary, &path, &preview);
                error.to_string()
            })?;
        app.ui.status = format!("Sending {name} to Telegram…");
        app.ui.status_error = false;
        app.telegram_job = Some(Job { rx: _rx, path, temporary, preview, name, token });
        Ok(())
    }
}

fn resolve_payload(
    app: &mut PhotocraftApp,
    format: &str,
    pending: Option<String>,
    source: Option<&photocraft_doc::Document>,
) -> Result<(String, bool), String> {
    match format {
        "png" => export_temp_for(app, source.ok_or("source document is no longer open")?, "png").map(|path| (path, true)),
        "psd" => export_temp_for(app, source.ok_or("source document is no longer open")?, "psd").map(|path| (path, true)),
        _ => saved_file(app, pending, source).ok_or_else(|| "There is no saved file to send. Choose PSD or PNG.".to_string()),
    }
}

fn saved_file(app: &PhotocraftApp, pending: Option<String>, source: Option<&photocraft_doc::Document>) -> Option<(String, bool)> {
    let path = pending.filter(|path| std::fs::metadata(path).is_ok_and(|meta| meta.is_file())).or_else(|| {
        source
            .and_then(|source| app.session.documents().iter().find(|st| st.doc.id == source.id))
            .and_then(|st| st.path.clone())
            .filter(|path| std::fs::metadata(path).is_ok_and(|meta| meta.is_file()))
    })?;
    Some((path, false))
}

fn export_temp_for(app: &mut PhotocraftApp, doc: &photocraft_doc::Document, extension: &str) -> Result<String, String> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (app, doc, extension);
        return Err("Telegram sending needs the desktop app.".to_string());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let export = app.services.export.as_ref().ok_or_else(|| "Telegram sending needs the desktop app.".to_string())?;
        let stem = file_name(&doc.name);
        let temp_dir = tempfile::Builder::new().prefix("photocraft-telegram-").tempdir().map_err(|error| error.to_string())?;
        let path = temp_dir.path().join(format!("{stem}.{extension}"));
        let path_str = path.display().to_string();
        let settings = crate::ExportSettings { xmp_all: false, tiff_layers: false, ..crate::ExportSettings::default() };
        let (bytes, _) = export(doc, &path_str, &settings).map_err(|error| format!("Could not prepare a file to send: {error}"))?;
        if bytes.len() as u64 > MAX_BYTES {
            let _ = std::fs::remove_file(&path);
            return Err("That file is larger than Telegram accepts (50 MB).".to_string());
        }
        std::fs::write(&path, &bytes).map_err(|error| read_error(&path_str, &error.to_string()))?;
        let _ = temp_dir.keep();
        Ok(path_str)
    }
}

fn remove_temp_path(path: &str) {
    let path = std::path::Path::new(path);
    let _ = std::fs::remove_file(path);
    if let Some(parent) =
        path.parent().filter(|parent| parent.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with("photocraft-telegram-")))
    {
        let _ = std::fs::remove_dir(parent);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn inbox_rows(sent_files: &[Value], body: &str) -> Vec<Value> {
    let mut rows = sent_files.to_vec();
    let mut added = 0;
    for row in files_from_updates(body) {
        let id = row.get("id").and_then(Value::as_str).unwrap_or("");
        if id.is_empty() || rows.iter().any(|have| have.get("id").and_then(Value::as_str) == Some(id)) {
            continue;
        }
        rows.push(row);
        added += 1;
        if added == 10 {
            break;
        }
    }
    rows
}

// Direct dialog buttons also run under synthetic input; confirmation is not their only entry.
fn authorize_network(app: &PhotocraftApp, action: &str) -> Result<(), String> {
    let params = json!({"action": action});
    if (app.automation_input || app.session.authorize.is_some())
        && let Some(authorize) = app.services.automation_command.as_ref()
    {
        authorize("file.sendTelegram", &params)?;
    }
    if let Some(authorize) = app.session.authorize {
        authorize("file.sendTelegram", &params).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn start_download(app: &mut PhotocraftApp, token: &str, file_id: &str, name: &str) -> Result<(), String> {
    authorize_network(app, "download")?;
    if app.telegram_job.is_some() {
        return Err("A Telegram send is already running.".to_string());
    }
    let Some(download) = app.services.telegram_download.clone() else {
        return Err("Telegram sending needs the desktop app.".to_string());
    };
    let token = token.to_string();
    let file_id = file_id.to_string();
    let name = file_name(name);
    let (tx, _rx) = std::sync::mpsc::channel::<Result<String, String>>();
    let _token_for_job = token.clone();
    let _name_for_status = name.clone();
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (download, token, file_id, name, tx);
        Err("Telegram sending needs the desktop app.".to_string())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::Builder::new()
            .name("pc-tg-get".into())
            .spawn(move || {
                let result = match download(&token, &file_id) {
                    Ok(bytes) => (|| {
                        let temp_dir = tempfile::Builder::new().prefix("photocraft-telegram-in-").tempdir().map_err(|error| error.to_string())?;
                        let path = temp_dir.path().join(&name);
                        std::fs::write(&path, &bytes).map_err(|error| error.to_string())?;
                        let _ = temp_dir.keep();
                        Ok(format!("OPEN\t{}", path.display()))
                    })(),
                    Err(error) => Err(redact(&error, &token)),
                };
                let _ = tx.send(result);
            })
            .map_err(|error| error.to_string())?;
        app.ui.status = format!("Opening {_name_for_status}…");
        app.ui.status_error = false;
        app.telegram_job = Some(Job { rx: _rx, path: String::new(), temporary: false, preview: None, name: _name_for_status, token: _token_for_job });
        Ok(())
    }
}

fn start_find_chats(app: &mut PhotocraftApp, token: &str) -> Result<(), String> {
    authorize_network(app, "findChats")?;
    if app.telegram_job.is_some() {
        return Err("A Telegram send is already running.".to_string());
    }
    validate_token(token)?;
    let Some(get) = app.services.telegram_get.clone() else {
        return Err("Telegram sending needs the desktop app.".to_string());
    };
    let token = token.to_string();
    let (tx, _rx) = std::sync::mpsc::channel::<Result<String, String>>();
    let _token_for_job = token.clone();
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (get, token, tx);
        Err("Telegram sending needs the desktop app.".to_string())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::Builder::new()
            .name("pc-tg-chats".into())
            .spawn(move || {
                let url = format!("https://api.telegram.org/bot{token}/getUpdates");
                let result = get(&url)
                    .map_err(|error| redact(&error, &token))
                    .map(|body| format!("CHATS\t{}", serde_json::to_string(&chats_from_updates(&body)).unwrap_or_else(|_| "[]".into())));
                let _ = tx.send(result);
            })
            .map_err(|error| error.to_string())?;
        app.ui.status = "Looking for chats…".to_string();
        app.ui.status_error = false;
        app.telegram_job = Some(Job { rx: _rx, path: String::new(), temporary: false, preview: None, name: String::new(), token: _token_for_job });
        Ok(())
    }
}

fn start_fetch_inbox(app: &mut PhotocraftApp, token: &str) -> Result<(), String> {
    authorize_network(app, "getFiles")?;
    if app.telegram_job.is_some() {
        return Err("A Telegram send is already running.".to_string());
    }
    validate_token(token)?;
    let Some(get) = app.services.telegram_get.clone() else {
        return Err("Telegram sending needs the desktop app.".to_string());
    };
    let token = token.to_string();
    let sent = app.telegram.sent_files.clone();
    let (tx, _rx) = std::sync::mpsc::channel::<Result<String, String>>();
    let _token_for_job = token.clone();
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (get, token, sent, tx);
        Err("Telegram sending needs the desktop app.".to_string())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::Builder::new()
            .name("pc-tg-inbox".into())
            .spawn(move || {
                let url = format!("https://api.telegram.org/bot{token}/getUpdates");
                let result = get(&url).map_err(|error| redact(&error, &token)).map(|body| {
                    let rows = inbox_rows(&sent, &body);
                    format!("INBOX\t{}", serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into()))
                });
                let _ = tx.send(result);
            })
            .map_err(|error| error.to_string())?;
        app.ui.status = "Loading files…".to_string();
        app.ui.status_error = false;
        app.telegram_job = Some(Job { rx: _rx, path: String::new(), temporary: false, preview: None, name: String::new(), token: _token_for_job });
        Ok(())
    }
}

#[cfg(test)]
mod tests;
