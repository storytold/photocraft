//! File › Submit to DeviantArt…. Uploads to Sta.sh, DeviantArt's drafts, not the public gallery.
//!
//! The refresh token stays in the system keychain. `deviantart.json` keeps the app id, title,
//! tags and the auto-submit switches. The web build has no submit.

use serde_json::{Value, json};

use crate::PhotocraftApp;

mod dialog;
mod errors;
mod extras;
mod format;
mod management_ui;
mod pending;
mod publish_ui;
#[cfg(test)]
mod surface_tests;
#[cfg(test)]
mod tests;

pub use extras::gallery_rows;
use extras::{remember_stash, stash_rows_from};
pub use format::{clean_tags, item_id, multipart, stash_url};

const BASE_SCOPES: &str = "stash";

/// Loopback the user must whitelist on their DeviantArt app. A fixed port so the redirect URI
/// does not change between logins.
pub const REDIRECT_URI: &str = "http://127.0.0.1:43821/callback";

pub const DEVELOPER_PAGE: &str = "https://www.deviantart.com/developers/";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteKind {
    Save,
    SaveAs,
    SaveCopy,
    Export,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub client_id: String,
    pub refresh_token: String,
    pub scopes: String,
    pub title: String,
    pub tags: String,
    pub send_on_save: bool,
    pub send_on_save_as: bool,
    pub send_on_export: bool,
    pub send_on_save_copy: bool,
    pub last_item_id: String,
    /// Sta.sh uploads from this app, newest first. DeviantArt no longer offers a list endpoint.
    pub recent: Vec<RecentItem>,
}

/// One Sta.sh draft this app uploaded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecentItem {
    pub item_id: i64,
    pub title: String,
    pub url: String,
}

impl Settings {
    fn ready(&self) -> bool {
        !self.client_id.is_empty() && !self.refresh_token.is_empty()
    }

    fn wants(&self, kind: WriteKind) -> bool {
        match kind {
            WriteKind::Save => self.send_on_save,
            WriteKind::SaveAs => self.send_on_save_as,
            WriteKind::SaveCopy => self.send_on_save_copy,
            WriteKind::Export => self.send_on_export,
        }
    }
}

pub fn settings_from_json(text: &str) -> Settings {
    let Ok(value) = serde_json::from_str::<Value>(text) else { return Settings::default() };
    let flag = |key: &str| value.get(key).and_then(Value::as_bool).unwrap_or(false);
    let text_of = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    Settings {
        client_id: text_of("clientId"),
        refresh_token: text_of("refreshToken"),
        scopes: value
            .get("scopes")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| if text_of("refreshToken").is_empty() { String::new() } else { "stash".into() }),
        title: text_of("title"),
        tags: text_of("tags"),
        send_on_save: flag("sendOnSave"),
        send_on_save_as: flag("sendOnSaveAs"),
        send_on_export: flag("sendOnExport"),
        send_on_save_copy: flag("sendOnSaveCopy"),
        last_item_id: text_of("lastItemId"),
        recent: value
            .get("recentItems")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        let item_id = item.get("itemId").and_then(Value::as_i64)?;
                        let url = item.get("url").and_then(Value::as_str)?.to_string();
                        let title = item.get("title").and_then(Value::as_str).unwrap_or("Sta.sh").to_string();
                        Some(RecentItem { item_id, title, url })
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

pub fn settings_json(s: &Settings) -> String {
    serde_json::to_string(&json!({
        "clientId": s.client_id,
        "refreshToken": s.refresh_token,
        "scopes": s.scopes,
        "title": s.title,
        "tags": s.tags,
        "sendOnSave": s.send_on_save,
        "sendOnSaveAs": s.send_on_save_as,
        "sendOnExport": s.send_on_export,
        "sendOnSaveCopy": s.send_on_save_copy,
        "lastItemId": s.last_item_id,
        "recentItems": s.recent.iter().map(|item| json!({"itemId": item.item_id, "title": item.title, "url": item.url})).collect::<Vec<_>>(),
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

pub fn without_token(text: &str) -> Result<(String, String), String> {
    let mut value: Value = serde_json::from_str(text).unwrap_or_else(|_| json!({}));
    let token = value.get("refreshToken").and_then(Value::as_str).unwrap_or("").to_string();
    if let Some(object) = value.as_object_mut() {
        object.remove("refreshToken");
    }
    Ok((serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_string()), token))
}

pub fn with_token(text: &str, token: &str) -> String {
    let mut value: Value = serde_json::from_str(text).unwrap_or_else(|_| json!({}));
    if let Some(object) = value.as_object_mut() {
        object.insert("refreshToken".into(), json!(token));
    }
    serde_json::to_string(&value).unwrap_or_else(|_| text.to_string())
}

pub fn command(app: &mut PhotocraftApp) -> Result<Value, String> {
    if cfg!(target_arch = "wasm32") {
        return app.session.execute(photocraft_engine::deviantart_cmds::COMMAND, json!({})).map_err(|error| error.to_string());
    }
    let pending = app.session.active().and_then(|d| d.path.clone());
    Ok(json!({"dialog": open_dialog(app, pending)}))
}

pub fn after_written(app: &mut PhotocraftApp, path: &str, kind: WriteKind) {
    if !app.deviantart.wants(kind) {
        return;
    }
    if !app.deviantart.ready() {
        pending::enqueue(app, path);
        if !dialog_open(app) {
            let id = open_dialog(app, None);
            if let Some(dialog) = app.ui.dialog_mut(id) {
                dialog.fields.insert("__autoPending".into(), json!(true));
            }
        }
        return;
    }
    if app.deviantart_job.is_some() {
        pending::enqueue(app, path);
        return;
    }
    if let Err(error) = start_submit(app, path.to_string(), false) {
        app.ui.status = error;
        app.ui.status_error = true;
    }
}

pub fn arm_after_save(app: &mut PhotocraftApp, path: &str, kind: WriteKind, result: &Value) {
    if result.get("pending").and_then(Value::as_bool) == Some(true) {
        if let Some(save) = app.jobs.saves.iter_mut().rev().find(|save| save.path == path && !save.ended) {
            save.notify_deviantart = Some(kind);
        }
        return;
    }
    after_written(app, path, kind);
}

pub(crate) fn busy(app: &PhotocraftApp) -> bool {
    app.deviantart_job.as_ref().is_some_and(|job| matches!(job.kind, JobKind::Submit | JobKind::Publish | JobKind::Edit | JobKind::Avatar))
        || (app.deviantart.ready() && !app.deviantart_pending.is_empty())
}

pub fn poll(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(job) = app.deviantart_job.as_ref() else {
        pending::start_next(app, ctx);
        return;
    };
    let finished = match job.rx.try_recv() {
        Ok(result) => Some(result),
        Err(std::sync::mpsc::TryRecvError::Empty) => {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err("desktop".to_string())),
    };
    let Some(job) = app.deviantart_job.take() else { return };
    if job.temporary {
        remove_temp_path(&job.path);
    }
    match (job.kind, finished) {
        (JobKind::Connect, Some(Ok(token))) => {
            app.deviantart.refresh_token = token;
            app.deviantart.scopes = job.name.clone();
            if let Some(dialog) = app.ui.dialogs.iter_mut().find(|dialog| owns(&dialog.fields)) {
                dialog.fields.insert("connected".into(), json!(true));
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            match save_settings(app) {
                Ok(()) => {
                    app.ui.status = tl!("DeviantArt is connected.").to_string();
                    app.ui.status_error = false;
                }
                Err(error) => {
                    app.ui.status = error;
                    app.ui.status_error = true;
                }
            }
        }
        (_, Some(Ok(body))) if body.starts_with("OPEN\t") => {
            errors::remember_rotated_login(app);
            let path = body.trim_start_matches("OPEN\t");
            let opened = std::fs::read(path).ok();
            let name = file_name(path);
            remove_temp_path(path);
            if let Some(bytes) = opened {
                if let Err(error) = app.open_bytes(&name, &bytes) {
                    app.ui.status = error;
                    app.ui.status_error = true;
                } else {
                    app.ui.status = crate::i18n::fmt(tl!("Opened {name}."), &[("name", &name)]);
                    app.ui.status_error = false;
                }
            } else {
                app.ui.status = tl!("Could not open the DeviantArt download.").to_string();
                app.ui.status_error = true;
            }
        }
        (JobKind::Submit, Some(Ok(reply))) => match engine_submission_reply(app, &reply) {
            Ok(body) => {
                // DeviantArt invalidates the refresh token it just accepted and returns a new one.
                // Pick that up before saving, or the next call sends the dead token.
                errors::remember_rotated_login(app);
                let url = item_id(&body).and_then(stash_url);
                if let Some(id) = item_id(&body) {
                    app.deviantart.last_item_id = id.to_string();
                    if let Some(url) = &url {
                        remember_stash(app, id, &job.name, url);
                    }
                    if let Some(dialog) = app.ui.dialogs.iter_mut().find(|dialog| owns(&dialog.fields)) {
                        dialog.fields.insert("itemId".into(), json!(id.to_string()));
                        dialog.fields.insert("stash".into(), json!(stash_rows_from(&app.deviantart)));
                    }
                    let _ = save_settings(app);
                }
                let message = crate::i18n::fmt(tl!("Submitted {name} to Sta.sh. It stays private until you publish it."), &[("name", &job.name)]);
                app.ui.status = match &url {
                    Some(url) => format!("{message} {url}"),
                    None => message.clone(),
                };
                app.ui.status_error = false;
                show_result(app, message, url);
            }
            Err(error) => errors::report_deviantart_error(app, &error, true),
        },
        (JobKind::Publish, Some(Ok(_))) => {
            errors::remember_rotated_login(app);
            let message = tl!("Published on DeviantArt.").to_string();
            app.ui.status = message.clone();
            app.ui.status_error = false;
            show_result(app, message, None);
        }
        (JobKind::Edit, Some(Ok(_))) => {
            errors::remember_rotated_login(app);
            let message = tl!("Updated the title and tags on DeviantArt.").to_string();
            app.ui.status = message.clone();
            app.ui.status_error = false;
            show_result(app, message, None);
        }
        (JobKind::Avatar, Some(Ok(_))) => {
            errors::remember_rotated_login(app);
            let message = tl!("Set the DeviantArt profile picture.").to_string();
            app.ui.status = message.clone();
            app.ui.status_error = false;
            show_result(app, message, None);
        }
        (JobKind::Gallery, Some(Ok(body))) => {
            errors::remember_rotated_login(app);
            if let Some(dialog) = app.ui.dialogs.iter_mut().find(|dialog| owns(&dialog.fields)) {
                let rows = gallery_rows(&body);
                if rows.is_empty() {
                    dialog.fields.insert("galleryNote".into(), json!(tl!("No published works in the gallery.").to_string()));
                } else {
                    dialog.fields.remove("galleryNote");
                }
                dialog.fields.insert("gallery".into(), json!(rows));
                dialog.fields.remove("filesError");
            }
            app.ui.status_error = false;
        }
        (JobKind::Gallery, Some(Err(error))) => {
            if errors::login_was_rejected(&error) {
                app.deviantart.refresh_token.clear();
                let _ = save_settings(app);
            } else {
                errors::remember_rotated_login(app);
            }
            let message = errors::localize_error(&error);
            if let Some(dialog) = app.ui.dialogs.iter_mut().find(|dialog| owns(&dialog.fields)) {
                dialog.fields.insert("filesError".into(), json!(message.clone()));
            }
            app.ui.status = message;
            app.ui.status_error = true;
        }
        (JobKind::Download, Some(Err(error)))
        | (JobKind::Publish, Some(Err(error)))
        | (JobKind::Edit, Some(Err(error)))
        | (JobKind::Avatar, Some(Err(error))) => {
            errors::report_deviantart_error(app, &error, true);
        }
        (_, Some(Err(error))) => {
            errors::report_deviantart_error(app, &error, !matches!(job.kind, JobKind::Connect));
        }
        _ => {}
    }
    pending::start_next(app, ctx);
    crate::jobs_ui::close_after_saves(app, ctx);
}

pub fn owns(fields: &serde_json::Map<String, Value>) -> bool {
    fields.get("__deviantart").and_then(Value::as_bool) == Some(true)
}

fn dialog_open(app: &PhotocraftApp) -> bool {
    app.ui.dialogs.iter().any(|d| owns(&d.fields))
}

fn open_dialog(app: &mut PhotocraftApp, pending: Option<String>) -> u64 {
    let mut fields = serde_json::Map::new();
    fields.insert("__deviantart".into(), json!(true));
    fields.insert("__label".into(), json!("Submit to DeviantArt"));
    fields.insert("clientId".into(), json!(app.deviantart.client_id));
    fields.insert("title".into(), json!(app.deviantart.title));
    fields.insert("tags".into(), json!(app.deviantart.tags));
    fields.insert("connected".into(), json!(app.deviantart.ready()));
    fields.insert("onSave".into(), json!(app.deviantart.send_on_save));
    fields.insert("onSaveAs".into(), json!(app.deviantart.send_on_save_as));
    fields.insert("onExport".into(), json!(app.deviantart.send_on_export));
    fields.insert("onSaveCopy".into(), json!(app.deviantart.send_on_save_copy));
    let saved = pending.as_ref().is_some_and(|path| std::fs::metadata(path).is_ok_and(|meta| meta.is_file()))
        || app.session.active().and_then(|d| d.path.as_deref()).is_some_and(|path| std::fs::metadata(path).is_ok_and(|meta| meta.is_file()));
    fields.insert("format".into(), json!(if saved { "file" } else { "png" }));
    if let Some(path) = pending {
        fields.insert("pending".into(), json!(path));
    }
    app.ui.open_dialog(crate::state::DialogKind::Command, fields)
}

pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, fields: &mut serde_json::Map<String, Value>) {
    dialog::body(app, ui, fields);
}

pub fn confirm(app: &mut PhotocraftApp, f: &serde_json::Map<String, Value>) -> Result<Value, String> {
    apply_fields(app, f)?;
    save_settings(app)?;
    if f.get("__autoPending").and_then(Value::as_bool) == Some(true) {
        return Ok(Value::Null);
    }
    let format = f.get("format").and_then(Value::as_str).unwrap_or("psd");
    let pending = f.get("pending").and_then(Value::as_str).map(str::to_string);
    if app.session.active().is_none() && pending.is_none() {
        app.ui.status = tl!("DeviantArt is set up. Save or export a file, then submit it.").to_string();
        app.ui.status_error = false;
        return Ok(Value::Null);
    }
    if !app.deviantart.ready() {
        return Err(tl!("Connect DeviantArt first.").to_string());
    }
    start_prepared_submit(app, format, pending)?;
    Ok(Value::Null)
}

fn apply_fields(app: &mut PhotocraftApp, f: &serde_json::Map<String, Value>) -> Result<(), String> {
    let client_id = f.get("clientId").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if !client_id.is_empty() && !client_id.chars().all(|c| c.is_ascii_digit()) {
        return Err(tl!("The DeviantArt app id should be a number.").to_string());
    }
    app.deviantart.client_id = client_id;
    app.deviantart.title = f.get("title").and_then(Value::as_str).unwrap_or("").trim().to_string();
    app.deviantart.tags = f.get("tags").and_then(Value::as_str).unwrap_or("").trim().to_string();
    app.deviantart.send_on_save = f.get("onSave").and_then(Value::as_bool).unwrap_or(false);
    app.deviantart.send_on_save_as = f.get("onSaveAs").and_then(Value::as_bool).unwrap_or(false);
    app.deviantart.send_on_export = f.get("onExport").and_then(Value::as_bool).unwrap_or(false);
    app.deviantart.send_on_save_copy = f.get("onSaveCopy").and_then(Value::as_bool).unwrap_or(false);
    Ok(())
}

fn save_settings(app: &mut PhotocraftApp) -> Result<(), String> {
    configure_engine(app);
    let text = settings_json(&app.deviantart);
    let Some(save) = app.services.save_deviantart.as_mut() else {
        return Err(tl!("DeviantArt submitting needs the desktop app.").to_string());
    };
    save(&text)
}

#[derive(Clone, Copy)]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))] // Desktop-only workers cannot be constructed in the web build.
enum JobKind {
    Connect,
    Submit,
    Publish,
    Edit,
    Avatar,
    Gallery,
    Download,
}

pub(crate) struct Job {
    rx: std::sync::mpsc::Receiver<Result<String, String>>,
    kind: JobKind,
    path: String,
    temporary: bool,
    name: String,
}

// Screen-level synthetic input reaches dialog buttons without dialogs::confirm.
fn authorize_action(app: &PhotocraftApp, action: &str) -> Result<(), String> {
    let params = json!({"action": action});
    if (app.automation_input || app.session.authorize.is_some())
        && let Some(authorize) = app.services.automation_command.as_ref()
    {
        authorize(photocraft_engine::deviantart_cmds::COMMAND, &params)?;
    }
    if let Some(authorize) = app.session.authorize {
        authorize(photocraft_engine::deviantart_cmds::COMMAND, &params).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn start_connect(app: &mut PhotocraftApp, additional_scopes: &str) -> Result<(), String> {
    authorize_action(app, "connect")?;
    if app.deviantart_job.is_some() {
        return Err(tl!("A DeviantArt submit is already running.").to_string());
    }
    let client_id = app.deviantart.client_id.trim().to_string();
    if client_id.is_empty() || !client_id.chars().all(|c| c.is_ascii_digit()) {
        return Err(tl!("The DeviantArt app id should be a number.").to_string());
    }
    let Some(connect) = app.services.deviantart_connect.clone() else {
        return Err(tl!("DeviantArt submitting needs the desktop app.").to_string());
    };
    let scopes = requested_scopes(&app.deviantart.scopes, &format!("{BASE_SCOPES} {additional_scopes}"));
    let _scopes_for_job = scopes.clone();
    let (tx, _rx) = std::sync::mpsc::channel::<Result<String, String>>();
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (connect, client_id, tx);
        Err(tl!("DeviantArt submitting needs the desktop app.").to_string())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::Builder::new()
            .name("pc-da-login".into())
            .spawn(move || {
                let _ = tx.send(connect(&client_id, &scopes));
            })
            .map_err(|error| error.to_string())?;
        app.ui.status = tl!("Waiting for DeviantArt…").to_string();
        app.ui.status_error = false;
        app.deviantart_job = Some(Job { rx: _rx, kind: JobKind::Connect, path: String::new(), temporary: false, name: _scopes_for_job });
        Ok(())
    }
}

fn requested_scopes(granted: &str, needed: &str) -> String {
    let mut scopes = Vec::new();
    for scope in granted.split_whitespace().chain(needed.split_whitespace()) {
        if matches!(scope, "stash" | "gallery" | "browse" | "user" | "publish" | "user.manage") && !scopes.contains(&scope) {
            scopes.push(scope);
        }
    }
    scopes.join(" ")
}

/// Bind the host's authenticated transport to the engine without serializing credentials.
pub(crate) fn configure_engine(app: &mut PhotocraftApp) {
    app.session.deviantart_submit = if app.deviantart.ready() {
        app.services.deviantart_submit.clone().map(|submit| {
            let token = app.deviantart.refresh_token.clone();
            let client = app.deviantart.client_id.clone();
            std::sync::Arc::new(move |payload: &photocraft_engine::deviantart_cmds::Submission| {
                submit(&token, &client, &payload.title, &payload.comments, &payload.tags, &payload.name, &payload.bytes)
            }) as photocraft_engine::deviantart_cmds::SubmitFn
        })
    } else {
        None
    };
}

fn start_submit(app: &mut PhotocraftApp, path: String, temporary: bool) -> Result<(), String> {
    let result = start_prepared_submit(app, "saved", Some(path.clone()));
    // Engine exports stay in memory. A temporary file supplied by the host is removed
    // after failure, or by the finished job after upload.
    if temporary {
        if result.is_err() {
            remove_temp_path(&path);
        } else if let Some(job) = app.deviantart_job.as_mut() {
            job.temporary = true;
        }
    }
    result
}

fn start_prepared_submit(app: &mut PhotocraftApp, format: &str, pending: Option<String>) -> Result<(), String> {
    // The dialog's disk-copy choice is the engine's saved-path mode.
    let format = if format == "file" { "saved" } else { format };
    authorize_action(app, "submit")?;
    if app.deviantart_job.is_some() {
        return Err(tl!("A DeviantArt submit is already running.").to_string());
    }
    configure_engine(app);
    let params = submission_params(app, format, pending);
    #[cfg(target_arch = "wasm32")]
    {
        // Keep the same command id and explicit unsupported error as headless dispatch.
        app.session.execute(photocraft_engine::deviantart_cmds::COMMAND, params).map(|_| ()).map_err(|error| error.to_string())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let service = app.session.deviantart_submit.clone().ok_or_else(|| tl!("Connect DeviantArt first.").to_string())?;
        let source = app.session.active().map(|state| (state.doc.clone(), state.path.clone()));
        let path = params.get("path").and_then(Value::as_str).unwrap_or("").to_string();
        let name = if format == "saved" {
            file_name(&path)
        } else {
            format!("{}.{}", source.as_ref().map(|(doc, _)| stem(&doc.name)).unwrap_or_else(|| "PhotoCraft".into()), format)
        };
        let gate = app.session.authorize;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("pc-da-submit".into())
            .spawn(move || {
                let mut session = photocraft_engine::Session::new();
                session.deviantart_submit = Some(service);
                session.authorize = gate;
                if let Some((doc, path)) = source {
                    session.add_document((*doc).clone(), path);
                }
                let result = session
                    .execute(photocraft_engine::deviantart_cmds::COMMAND, params)
                    .map_err(|error| error.to_string())
                    .and_then(|reply| serde_json::to_string(&reply).map_err(|error| error.to_string()));
                let _ = tx.send(result);
            })
            .map_err(|error| error.to_string())?;
        app.ui.status = crate::i18n::fmt(tl!("Submitting {name} to DeviantArt…"), &[("name", &name)]);
        app.ui.status_error = false;
        app.deviantart_job = Some(Job { rx, kind: JobKind::Submit, path, temporary: false, name });
        Ok(())
    }
}

fn engine_submission_reply(app: &mut PhotocraftApp, reply: &str) -> Result<String, String> {
    let value: Value = serde_json::from_str(reply).map_err(|error| error.to_string())?;
    let body = value.get("body").and_then(Value::as_str).ok_or("invalid engine submission result")?;
    let warnings: Vec<String> = value.get("warnings").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
    crate::notices::io_warnings(app, "DeviantArt", &warnings);
    Ok(body.to_string())
}

fn submission_params(app: &PhotocraftApp, format: &str, pending: Option<String>) -> Value {
    let mut params = json!({"format":format,"title":app.deviantart.title,"tags":app.deviantart.tags});
    if format == "saved"
        && let Some(path) = pending.or_else(|| app.session.active().and_then(|state| state.path.clone()))
    {
        params["path"] = json!(path);
    }
    params
}

#[cfg(test)]
use photocraft_engine::deviantart_cmds::comments;
#[cfg(not(target_arch = "wasm32"))]
use photocraft_engine::deviantart_cmds::stem;

fn remove_temp_path(path: &str) {
    let path = std::path::Path::new(path);
    let _ = std::fs::remove_file(path);
    if let Some(parent) =
        path.parent().filter(|parent| parent.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with("photocraft-deviantart-")))
    {
        let _ = std::fs::remove_dir(parent);
    }
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or("file").to_string()
}

pub fn result_owns(fields: &serde_json::Map<String, Value>) -> bool {
    fields.get("__deviantart_result").and_then(Value::as_bool) == Some(true)
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
    if crate::widgets::secondary_button(ui, tl!("Open the Sta.sh page"), 200.0).clicked() {
        crate::links::open(app, ui.ctx(), url);
    }
}

fn show_result(app: &mut PhotocraftApp, message: String, url: Option<String>) {
    let mut fields = serde_json::Map::new();
    fields.insert("__deviantart_result".into(), json!(true));
    fields.insert("__label".into(), json!("DeviantArt"));
    fields.insert("message".into(), json!(message));
    if let Some(url) = url {
        fields.insert("url".into(), json!(url));
    }
    app.ui.open_dialog(crate::state::DialogKind::Command, fields);
}
