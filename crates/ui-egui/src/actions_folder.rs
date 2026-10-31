//! The optional Actions discovery folder; filesystem scans stay off the UI thread.

use crate::PhotocraftApp;
use serde_json::{Map, Value, json};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

#[derive(Default)]
pub(crate) struct Runtime {
    folder: String,
    available: Option<String>,
    pub(crate) files: Vec<String>,
    pending: Option<Receiver<Option<Scan>>>,
    next: Option<Instant>,
}

struct Scan {
    directory: String,
    files: Vec<String>,
}

fn scan(folder: &str) -> Option<Scan> {
    let directory = std::path::absolute(folder).ok()?;
    let entries = std::fs::read_dir(&directory).ok()?;
    let mut files = Vec::new();
    for entry in entries.take(2048).flatten() {
        let path = entry.path();
        if !path.extension().and_then(|s| s.to_str()).is_some_and(|s| s.eq_ignore_ascii_case("atn")) {
            continue;
        }
        let Ok(metadata) = std::fs::metadata(&path) else { continue };
        if metadata.is_file()
            && let Some(path) = path.to_str()
        {
            files.push(path.to_string());
        }
    }
    files.sort_by(|a, b| crate::file_open::natural_cmp(a, b));
    files.truncate(256);
    Some(Scan { directory: directory.to_str()?.to_string(), files })
}

fn configured(app: &PhotocraftApp) -> &str {
    let folder = app.session.prefs().file_handling.actions_folder.as_str();
    if folder.trim().is_empty() { "" } else { folder }
}

pub(crate) fn directory(app: &PhotocraftApp) -> Option<String> {
    let rt = &app.prefs_rt.actions_folder;
    if rt.folder == configured(app) { rt.available.clone() } else { None }
}

pub(crate) fn files(app: &PhotocraftApp) -> &[String] {
    if directory(app).is_some() { &app.prefs_rt.actions_folder.files } else { &[] }
}

pub(crate) fn tick(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let folder = configured(app).to_string();
    let rt = &mut app.prefs_rt.actions_folder;
    if rt.folder != folder {
        rt.folder = folder.clone();
        rt.available = None;
        rt.files.clear();
        rt.pending = None;
        rt.next = None;
    }
    if folder.is_empty() {
        return;
    }
    if let Some(rx) = &rt.pending {
        match rx.try_recv() {
            Ok(result) => {
                rt.pending = None;
                rt.available = result.as_ref().map(|s| s.directory.clone());
                rt.files = result.map(|s| s.files).unwrap_or_default();
            }
            Err(TryRecvError::Disconnected) => rt.pending = None,
            Err(TryRecvError::Empty) => return,
        }
    }
    let now = Instant::now();
    if rt.pending.is_none() && rt.next.is_none_or(|next| now >= next) {
        rt.next = Some(now + Duration::from_secs(30));
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        match std::thread::Builder::new().name("actions-folder".into()).spawn(move || {
            let _ = tx.send(scan(&folder));
            ctx.request_repaint();
        }) {
            Ok(_) => rt.pending = Some(rx),
            Err(err) => log::warn!("Could not scan the Actions folder: {err}"),
        }
    }
    ctx.request_repaint_after(Duration::from_secs(30));
}

const REQUEST: &str = "prefs.actionsFolder.request";

pub(crate) fn field(ui: &mut egui::Ui, obj: &mut Map<String, Value>, value: &str) {
    ui.label(tl!("Actions Folder"));
    let mut value = value.to_string();
    ui.vertical(|ui| {
        ui.add(egui::TextEdit::singleline(&mut value).desired_width(260.0));
        ui.horizontal(|ui| {
            if ui.button(tl!("Browse…")).clicked() {
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique(REQUEST), true));
            }
            if ui.add_enabled(!value.trim().is_empty(), egui::Button::new(tl!("Open Location"))).clicked() {
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique(REQUEST), false));
            }
        });
    });
    obj.insert("actionsFolder".into(), json!(value));
}

pub(crate) fn preferences(app: &mut PhotocraftApp, ui: &egui::Ui, fields: &Map<String, Value>) {
    let Some(browse) = ui.ctx().data_mut(|d| d.remove_temp::<bool>(egui::Id::unique(REQUEST))) else { return };
    let folder = fields.get("values").and_then(|v| v.get("fileHandling")).and_then(|v| v.get("actionsFolder")).and_then(Value::as_str).unwrap_or("").to_string();
    let result = if browse {
        let Some(id) = fields.get("__prefsDialogId").and_then(Value::as_u64) else { return };
        app.ask_file(crate::file_dialog::FileDialogRequest::Folder { initial_dir: (!folder.is_empty()).then_some(folder) }, move |app, answer| {
            let crate::file_dialog::FileDialogAnswer::FolderTo(path) = answer else { return Err("expected a folder".into()) };
            if let Some(dialog) = app.ui.dialog_mut(id) {
                let values = dialog.fields.get_mut("values").and_then(Value::as_object_mut).ok_or("missing preferences")?;
                let handling = values.get_mut("fileHandling").and_then(Value::as_object_mut).ok_or("missing file handling preferences")?;
                handling.insert("actionsFolder".into(), json!(path));
            }
            Ok(Value::Null)
        })
    } else {
        app.services.open_url.as_ref().ok_or_else(|| "no system file opener configured".to_string()).and_then(|open| open(&folder)).map(|()| Value::Null)
    };
    if let Err(err) = result {
        app.ui.status = err;
        app.ui.status_error = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_folder_discovers_sets_and_missing_folders_fall_back() {
        let dir = std::env::temp_dir().join(format!(
            "photocraft-actions-folder-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        for name in ["Set 10.atn", "Set 2.ATN", "readme.txt"] {
            std::fs::write(dir.join(name), b"test").unwrap();
        }
        std::fs::create_dir(dir.join("not-a-file.atn")).unwrap();
        let result = scan(dir.to_str().unwrap()).unwrap();
        assert_eq!(result.files.iter().map(|p| crate::file_open::display_name(p)).collect::<Vec<_>>(), ["Set 2.ATN", "Set 10.atn"]);
        assert!(scan(dir.join("missing").to_str().unwrap()).is_none());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.prefs.edit(|p| p.file_handling.actions_folder = dir.to_string_lossy().to_string());
        let rt = &mut app.prefs_rt.actions_folder;
        rt.folder = dir.to_string_lossy().to_string();
        rt.available = Some(result.directory.clone());
        rt.files = result.files;
        assert_eq!(directory(&app).as_deref(), dir.to_str());
        app.session.prefs.edit(|p| p.file_handling.actions_folder = "missing".into());
        assert!(directory(&app).is_none());
        assert!(files(&app).is_empty());
        for name in ["Set 10.atn", "Set 2.ATN", "readme.txt"] {
            std::fs::remove_file(dir.join(name)).unwrap();
        }
        std::fs::remove_dir(dir.join("not-a-file.atn")).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
