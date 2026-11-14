//! The Source Files group of File › Automate › Lens Correction… (#13979): Use (Files or Folder),
//! the list of the images to correct by file name, Browse…, Remove and Add Open Files.
//!
//! The list is the command's `input`: the full paths, in list order, so OK sends what is listed.

use egui::{RichText, vec2};
use serde_json::{Map, Value, json};

use crate::file_dialog::{FileDialogAnswer, FileDialogRequest};
use crate::{PhotocraftApp, state::DialogKind, theme::Tokens, widgets};

/// `"files"` or `"folder"`: what Browse… picks. Not a command param.
const USE: &str = "__use";
/// The selected row's path: a path, unlike an index, still names the same row (or none) after the
/// list changes under it.
const SELECTED: &str = "__selected";
/// The folder in `input` that could not be read, so it is not read again on every frame.
const UNREADABLE: &str = "__unreadable";

pub(crate) const UNSAVED: &str = "Documents must be saved before they can be corrected";

/// The listed paths.
fn list(f: &Map<String, Value>) -> Vec<String> {
    f.get("input").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

/// Every change of the list goes through here; the selection does not survive it.
fn set_list(f: &mut Map<String, Value>, list: Vec<String>) {
    f.insert("input".into(), json!(list));
    f.remove(SELECTED);
    f.remove(UNREADABLE);
}

/// Appends `paths` that are not listed yet.
fn append(f: &mut Map<String, Value>, paths: impl IntoIterator<Item = String>) {
    let mut listed = list(f);
    for path in paths {
        if !listed.contains(&path) {
            listed.push(path);
        }
    }
    set_list(f, listed);
}

/// The images the batch would read from `dir`: its image files by name, no sub-folders.
fn images(dir: &str) -> Result<Vec<String>, String> {
    photocraft_engine::file_cmds::list_images(dir).map_err(|e| e.to_string())
}

/// `input` as the list: a folder (a step recorded before the list, `ui.dialog.set`) lists its
/// images in Folder mode, anything else that is not a list becomes an empty one. A folder that
/// cannot be read stays as it is, so OK hands it to the command and gets the command's error.
fn normalize(app: &mut PhotocraftApp, f: &mut Map<String, Value>) {
    match f.get("input") {
        Some(Value::Array(_)) => {}
        Some(Value::String(dir)) if !dir.is_empty() => {
            if f.get(UNREADABLE) == f.get("input") {
                return;
            }
            let dir = dir.clone();
            f.insert(USE.into(), json!("folder"));
            match images(&dir) {
                Ok(found) => set_list(f, found),
                Err(e) => {
                    app.ui.status = e;
                    f.insert(UNREADABLE.into(), json!(dir));
                }
            }
        }
        _ => set_list(f, Vec::new()),
    }
}

fn folder_mode(f: &Map<String, Value>) -> bool {
    f.get(USE).and_then(Value::as_str) == Some("folder")
}

fn file_name(path: &str) -> &str {
    std::path::Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or(path)
}

/// The group's controls below its title, in dialog `id`; `w` is the dialog's label column.
pub(super) fn group(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>, id: u64, w: f32) {
    normalize(app, f);
    ui.label(tl!("Choose files for batch lens correction."));
    ui.horizontal(|ui| {
        widgets::column_label(ui, tl!("Use:"), w);
        let mut mode = if folder_mode(f) { "folder" } else { "files" }.to_string();
        let options = [("files".to_string(), tl!("Files")), ("folder".to_string(), tl!("Folder"))];
        if widgets::dropdown(ui, "lens-batch-use", tl!("Use"), &mut mode, &options, 120.0) {
            f.insert(USE.into(), json!(mode));
        }
    });
    let listed = list(f);
    let selected = f.get(SELECTED).and_then(Value::as_str).and_then(|p| listed.iter().position(|l| l == p));
    ui.horizontal_top(|ui| {
        if let Some(path) = rows(ui, &listed, selected).and_then(|i| listed.get(i)) {
            f.insert(SELECTED.into(), json!(path));
        }
        ui.vertical(|ui| {
            let bw = widgets::button_width(ui, &[tl!("Browse…"), tl!("Remove"), tl!("Add Open Files")]);
            if widgets::secondary_button(ui, tl!("Browse…"), bw).clicked() {
                browse(app, id, folder_mode(f));
            }
            if ui.add_enabled_ui(selected.is_some(), |ui| widgets::secondary_button(ui, tl!("Remove"), bw)).inner.clicked()
                && let Some(i) = selected
            {
                let mut rest = listed.clone();
                rest.remove(i);
                set_list(f, rest);
            }
            let any_open = !app.session.documents().is_empty();
            if ui.add_enabled_ui(any_open, |ui| widgets::secondary_button(ui, tl!("Add Open Files"), bw)).inner.clicked() {
                add_open_files(app, f);
            }
        });
    });
}

/// The list box: one row per path, by file name. Returns the row clicked.
fn rows(ui: &mut egui::Ui, listed: &[String], selected: Option<usize>) -> Option<usize> {
    let t = Tokens::get(ui.ctx());
    let mut clicked = None;
    egui::Frame::NONE.fill(t.field).stroke(egui::Stroke::new(1.0, t.field_border)).corner_radius(t.radius_sm).inner_margin(4.0).show(ui, |ui| {
        ui.set_min_size(vec2(300.0, 120.0));
        ui.set_max_width(300.0);
        egui::ScrollArea::vertical().id_salt("lens-batch-sources").max_height(120.0).auto_shrink([false, false]).show(ui, |ui| {
            // The rows' own top-down layout: the group around the list is a horizontal one.
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                for (i, path) in listed.iter().enumerate() {
                    let row = ui.add(egui::Button::selectable(selected == Some(i), RichText::new(file_name(path))));
                    if row.clicked() {
                        clicked = Some(i);
                    }
                }
            });
        });
    });
    clicked
}

/// Browse…: files to append (Files) or the folder whose images replace the list (Folder), into
/// dialog `id` once the file dialog answers.
fn browse(app: &mut PhotocraftApp, id: u64, folder: bool) {
    let r = if folder {
        app.ask_folder(move |app, dir| {
            let found = images(&dir)?;
            set_list(&mut app.ui.dialog_mut(id).ok_or("the dialog was closed")?.fields, found);
            Ok(Value::Null)
        })
    } else {
        let initial_dir = crate::file_dialog::last_used_dir(&app.ui.recent_files);
        app.ask_file(FileDialogRequest::Open { multiple: true, initial_dir, extensions: None }, move |app, answer| {
            let FileDialogAnswer::Paths(paths) = answer else { return Err("Lens Correction reads files from disk".into()) };
            append(&mut app.ui.dialog_mut(id).ok_or("the dialog was closed")?.fields, paths);
            Ok(Value::Null)
        })
    };
    if let Err(e) = r.map(|_| ()).or_else(|e| if e == crate::file_dialog::CANCELLED { Ok(()) } else { Err(e) }) {
        app.ui.status = e;
    }
}

/// Add Open Files: the open documents saved to a file and unchanged since, in tab order. A
/// modified or never-saved one is left out with the reference application's alert.
fn add_open_files(app: &mut PhotocraftApp, f: &mut Map<String, Value>) {
    let docs = app.session.documents();
    let saved: Vec<String> = docs.iter().filter(|d| !d.is_dirty()).filter_map(|d| d.path.clone()).collect();
    let unsaved = saved.len() < docs.len();
    append(f, saved);
    if unsaved {
        app.ui.open_dialog(DialogKind::Error, json!({"message": tl!(UNSAVED)}).as_object().cloned().unwrap_or_default());
    }
}
