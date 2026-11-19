//! File › Scripts › Load Files into Stack… and Statistics: a hosted form with the Source Files row (Use: Files or
//! Folder, the list, Browse…, Remove, Add Open Files, Sort By Name), Attempt to Automatically Align Source Images and,
//! for Load Files into Stack, Create Smart Object after Loading Layers. Statistics adds Stack Mode and always
//! creates a smart object. OK runs the corresponding engine command with the list or the folder.

use std::path::{Path, PathBuf};

use craft::ui::contract::Bind;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::Label;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::dialogs::params;

/// The command id of Load Files into Stack, which is also its dialog's key.
pub const COMMAND: &str = "file.scripts.loadFilesIntoStack";
const STATISTICS: &str = "file.scripts.statistics";

/// Whether `key` is the key of one of these two dialogs.
pub(crate) fn owns(key: &str) -> bool {
    key == COMMAND || key == STATISTICS
}

/// Opens Load Files into Stack… with an empty Files list.
pub fn open(app: &mut PhotocraftApp) -> DialogId {
    open_for(app, COMMAND, tl!("Load Files into Stack…"), false)
}

/// Opens Image Statistics with an empty Files list.
pub fn open_statistics(app: &mut PhotocraftApp) -> DialogId {
    open_for(app, STATISTICS, tl!("Image Statistics"), true)
}

fn open_for(app: &mut PhotocraftApp, command: &'static str, title: &str, statistics: bool) -> DialogId {
    let mut rows = Vec::new();
    if statistics {
        let modes = photocraft_doc::StackMode::ALL.iter().map(|m| (m.id().to_string(), Label::from(m.label().to_string()))).collect();
        rows.push(Param::new("mode", tl!("Choose Stack Mode:"), ParamKind::choice(modes, Some("median".into()))));
    }
    rows.push(Param::files("files", tl!("Source Files")).folder_mode(true).open_files(open_files(app)));
    rows.push(Param::new("align", tl!("Attempt to Automatically Align Source Images"), ParamKind::bool(false)));
    if !statistics {
        rows.push(Param::new("smartObject", tl!("Create Smart Object after Loading Layers"), ParamKind::bool(false)));
    }
    let title = Label::from(title.to_string());
    params::open_param(app, command, DialogFields::new(), Value::Null, move |fields: &mut DialogFields| {
        let ready = paths(fields).is_some();
        ParamDialog::command(command, title.clone(), rows.clone()).values(Bind::bound(fields)).ok_enabled(ready)
    })
}

/// The open documents that have a file, in tab order (what Add Open Files adds).
fn open_files(app: &PhotocraftApp) -> Vec<PathBuf> {
    app.session.documents().iter().filter_map(|d| d.path.clone()).map(PathBuf::from).collect()
}

/// What OK loads: the folder in Folder mode, else the listed files. `None` while there is nothing to load, which is
/// also when OK stays disabled.
fn paths(fields: &DialogFields) -> Option<Value> {
    if fields.bool("files.useFolder").unwrap_or(false) {
        let folder = fields.text("files.folder").filter(|f| !f.is_empty())?;
        return Some(json!(folder));
    }

    let list: Vec<String> = fields
        .list("files")?
        .iter()
        .filter_map(|v| match v {
            FieldValue::Text(path) => Some(path.clone()),
            _ => None,
        })
        .collect();
    (!list.is_empty()).then(|| json!(list))
}

/// OK: loads the list (or the folder) into a new document.
pub(crate) fn run(app: &mut PhotocraftApp, _ctx: &egui::Context, result: ParamResult) {
    let Some(paths) = paths(&result.fields) else {
        app.ui.status = tl!("Add the files to load").to_string();
        app.ui.status_error = true;
        return;
    };
    let align = result.fields.bool("align").unwrap_or(false);
    let outcome = if result.key == STATISTICS {
        let mode = result.fields.text("mode").unwrap_or("median");
        app.run(STATISTICS, json!({"paths": paths, "align": align, "mode": mode}))
    } else {
        let smart = result.fields.bool("smartObject").unwrap_or(false);
        app.run(COMMAND, json!({"paths": paths, "align": align, "createSmartObject": smart}))
    };
    match outcome {
        Ok(_) => app.sync_views(),
        Err(error) => {
            app.ui.status = error;
            app.ui.status_error = true;
        }
    }
}

/// The images of `folder`, for the Files row in Folder mode: asked once per folder change.
pub(crate) fn listing(_app: &PhotocraftApp, folder: &Path) -> Vec<PathBuf> {
    photocraft_engine::file_cmds::list_images(&folder.to_string_lossy()).unwrap_or_default().into_iter().map(PathBuf::from).collect()
}

#[cfg(test)]
mod tests;
