//! Gallery, download, publish/edit and avatar host plumbing. Submission uses the engine.
#[cfg(not(target_arch = "wasm32"))]
use super::{Job, file_name, stem};
use super::{JobKind, PhotocraftApp, RecentItem, Settings, Value, authorize_action, json, remove_temp_path, start_connect, stash_url};

pub fn gallery_rows(text: &str) -> Vec<Value> {
    let mut out = Vec::new();
    let Ok(value) = serde_json::from_str::<Value>(text) else { return out };
    let Some(results) = value.get("results").and_then(Value::as_array) else { return out };
    for item in results {
        let Some(id) = item.get("deviationid").and_then(Value::as_str) else { continue };
        if id.is_empty() {
            continue;
        }
        let title = item.get("title").and_then(Value::as_str).unwrap_or(id);
        out.push(json!({"id": id, "title": title}));
    }
    out
}

pub(super) fn scopes_for_operation(operation: &str) -> &'static str {
    match operation {
        "gallery" => "browse user",
        "download" => "browse",
        "publish" | "edit" => "publish",
        "avatar" => "browse user.manage",
        _ => "",
    }
}

pub(super) fn start_call(app: &mut PhotocraftApp, request: &str) -> Result<(), String> {
    let parsed: Value = serde_json::from_str(request).unwrap_or_else(|_| json!({}));
    let op = parsed.get("op").and_then(Value::as_str).unwrap_or("");
    let (_kind, path, temporary) = match op {
        "publish" => (JobKind::Publish, String::new(), false),
        "edit" => (JobKind::Edit, String::new(), false),
        "gallery" => (JobKind::Gallery, String::new(), false),
        "download" => (JobKind::Download, String::new(), false),
        "avatar" => {
            let path = parsed.get("path").and_then(Value::as_str).unwrap_or("").to_string();
            (JobKind::Avatar, path, true)
        }
        _ => (JobKind::Submit, String::new(), false),
    };
    let discard_temp = |path: &str| {
        if temporary {
            remove_temp_path(path);
        }
    };
    if let Err(error) = authorize_action(app, op) {
        discard_temp(&path);
        return Err(error);
    }
    if app.deviantart_job.is_some() {
        discard_temp(&path);
        return Err(tl!("A DeviantArt submit is already running.").to_string());
    }
    let scopes = scopes_for_operation(op);
    if !scopes.is_empty() && !scopes.split_whitespace().all(|scope| app.deviantart.scopes.split_whitespace().any(|granted| granted == scope)) {
        discard_temp(&path);
        return start_connect(app, scopes);
    }
    let Some(call) = app.services.deviantart_call.clone() else {
        discard_temp(&path);
        return Err(tl!("DeviantArt submitting needs the desktop app.").to_string());
    };
    let refresh = app.deviantart.refresh_token.clone();
    let client_id = app.deviantart.client_id.clone();
    let request = request.to_string();
    let (tx, _rx) = std::sync::mpsc::channel::<Result<String, String>>();
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (call, refresh, client_id, request, tx);
        discard_temp(&path);
        Err(tl!("DeviantArt submitting needs the desktop app.").to_string())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::Builder::new()
            .name("pc-da-call".into())
            .spawn(move || {
                let _ = tx.send(call(&refresh, &client_id, &request));
            })
            .map_err(|error| {
                discard_temp(&path);
                error.to_string()
            })?;
        app.deviantart_job = Some(Job { rx: _rx, kind: _kind, path, temporary, name: "DeviantArt".into() });
        Ok(())
    }
}

pub(super) fn avatar_file(app: &mut PhotocraftApp, format: &str) -> Result<String, String> {
    authorize_action(app, "avatar")?;
    let extension = if format == "jpg" { "jpg" } else { "png" };
    export_avatar_temp(app, extension)
}

pub(super) fn remember_stash(app: &mut PhotocraftApp, item_id: i64, title: &str, url: &str) {
    if item_id <= 0 {
        return;
    }
    app.deviantart.recent.retain(|item| item.item_id != item_id);
    app.deviantart.recent.insert(0, RecentItem { item_id, title: if title.is_empty() { "Sta.sh".into() } else { title.into() }, url: url.to_string() });
    app.deviantart.recent.truncate(10);
}

pub(super) fn stash_rows_from(settings: &Settings) -> Vec<Value> {
    let mut items = settings.recent.clone();
    if items.is_empty()
        && let Ok(item_id) = settings.last_item_id.parse::<i64>()
        && let Some(url) = stash_url(item_id)
    {
        let title = if settings.title.is_empty() { "Sta.sh".into() } else { settings.title.clone() };
        items.push(RecentItem { item_id, title, url });
    }
    items.into_iter().map(|item| json!({"itemId": item.item_id, "title": item.title, "url": item.url})).collect()
}

pub(super) fn gallery_section(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut serde_json::Map<String, Value>) {
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(tl!("Sta.sh drafts open their page. Published works open in PhotoCraft.")).color(t.text_dim));
    ui.add_space(4.0);
    if f.get("stash").is_none() {
        f.insert("stash".into(), json!(stash_rows_from(&app.deviantart)));
    }
    if crate::widgets::secondary_button(ui, tl!("Get files"), 160.0).clicked() {
        f.remove("filesError");
        f.remove("galleryNote");
        let stash = stash_rows_from(&app.deviantart);
        if stash.is_empty() {
            f.insert("filesError".into(), json!(tl!("No Sta.sh files yet.").to_string()));
        }
        f.insert("stash".into(), json!(stash));
        if let Err(error) = start_call(app, r#"{"op":"gallery"}"#) {
            f.insert("filesError".into(), json!(error));
        }
    }
    if let Some(error) = f.get("filesError").and_then(Value::as_str) {
        ui.label(egui::RichText::new(error).color(t.warning));
    }
    if let Some(note) = f.get("galleryNote").and_then(Value::as_str) {
        ui.label(note);
    }
    if let Some(rows) = f.get("stash").and_then(Value::as_array).cloned() {
        for row in rows {
            let item_id = row.get("itemId").and_then(Value::as_i64).unwrap_or(0);
            let label = row.get("title").and_then(Value::as_str).unwrap_or("Sta.sh").to_string();
            if item_id <= 0 {
                continue;
            }
            let page = row.get("url").and_then(Value::as_str).unwrap_or("").to_string();
            if ui.selectable_label(false, label).clicked() {
                if page.is_empty() {
                    app.ui.status = tl!("DeviantArt did not offer a download.").to_string();
                    app.ui.status_error = true;
                } else {
                    crate::links::open(app, ui.ctx(), &page);
                }
            }
        }
    }
    if let Some(rows) = f.get("gallery").and_then(Value::as_array).cloned() {
        for row in rows {
            let id = row.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            let label = row.get("title").and_then(Value::as_str).unwrap_or(&id).to_string();
            if id.is_empty() {
                continue;
            }
            if ui.selectable_label(false, label).clicked()
                && let Err(error) = start_call(app, &json!({"op":"download","deviationId": id}).to_string())
            {
                app.ui.status = error;
                app.ui.status_error = true;
            }
        }
    }
}

// Avatar-only host export. Sta.sh uploads are prepared in the engine.
fn export_avatar_temp(app: &mut PhotocraftApp, extension: &str) -> Result<String, String> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (app, extension);
        return Err(tl!("DeviantArt submitting needs the desktop app.").to_string());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let doc = app.session.active().ok_or_else(|| "no document".to_string())?.doc.clone();
        let export = app.services.export.as_ref().ok_or_else(|| tl!("DeviantArt submitting needs the desktop app.").to_string())?;
        let stem = stem(&file_name(&doc.name));
        let temp_dir = tempfile::Builder::new().prefix("photocraft-deviantart-").tempdir().map_err(|error| error.to_string())?;
        let path = temp_dir.path().join(format!("{stem}.{extension}"));
        let path_str = path.display().to_string();
        let settings = crate::ExportSettings { xmp_all: false, tiff_layers: false, ..crate::ExportSettings::default() };
        let (bytes, _) =
            export(&doc, &path_str, &settings).map_err(|error| crate::i18n::fmt(tl!("Could not prepare a file to send: {error}"), &[("error", &error)]))?;
        if bytes.len() as u64 > photocraft_engine::deviantart_cmds::MAX_BYTES {
            return Err(tl!("That file is larger than DeviantArt accepts (30 MB).").to_string());
        }
        std::fs::write(&path, &bytes).map_err(|error| error.to_string())?;
        let _ = temp_dir.keep();
        Ok(path_str)
    }
}
