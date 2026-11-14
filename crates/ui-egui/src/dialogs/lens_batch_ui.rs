//! File › Automate › Lens Correction…, laid out like the reference application's dialog (#13981):
//! the sections Source Files, Destination Folder, Lens Correction Profile and Correction Options,
//! top to bottom, with the reference captions and the Edge entries in the reference order.
//!
//! The rows are the command's params, so OK, `ui.dialog.set` and `ui.dialog.confirm` work as for every form. Source
//! Files (#13979) is the framework's files row: Use (Files or Folder), the list, Browse…, Remove and Add Open Files;
//! [`command_params`] turns it into the command's `input`, the listed paths in list order or the folder.

use std::path::{Path, PathBuf};

use craft::ui::patterns::form_dialogs::{Param, ParamKind};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use craft::ui::runtime::env::Label;
use craft::ui::runtime::requests::PathKind;

pub(crate) const COMMAND: &str = photocraft_engine::lens_cmds::LENS_BATCH;

const FORMATS: [(&str, &str); 4] = [("jpg", "JPG"), ("png", "PNG"), ("psd", "PSD"), ("tiff", "TIFF")];
const PROFILES: [(&str, &str); 3] = [("auto", "Auto"), ("generic", "Generic"), ("none", "None")];
const EDGES: [(&str, &str); 4] = [("edgeExtension", "Edge Extension"), ("transparency", "Transparency"), ("black", "Black Color"), ("white", "White Color")];

/// Choice options whose labels are `tl!` literals here.
fn opts(list: &[(&'static str, &'static str)]) -> Vec<(String, Label)> {
    list.iter().map(|(id, label)| ((*id).to_owned(), Label::from(tl!(label)))).collect()
}

/// A folder field with its Browse button.
fn folder(key: &str, label: &'static str, default: String) -> Param {
    Param::new(key, label, ParamKind::text(default)).browse(PathKind::Folder)
}

/// The rows of the dialog, writing the results to `<dir>/corrected`. They open with the reference defaults (#13980): Chromatic
/// Aberration and Vignette off, File Type PSD; the command's own defaults for scripts and the CLI are unchanged. Correction Options lists the corrections, then scaling and
/// the edge fill, as the reference's left and right columns read.
pub(crate) fn rows(app: &PhotocraftApp, dir: String) -> Vec<Param> {
    vec![
        Param::section(tl!("Source Files")),
        Param::files("input", tl!("Source Files")).folder_mode(true).open_files(saved_documents(app)),
        Param::section(tl!("Destination Folder")),
        Param::new("format", tl!("File Type"), ParamKind::choice(opts(&FORMATS), Some("psd".into()))),
        folder("output", tl!("Output"), format!("{dir}/corrected")),
        Param::section(tl!("Lens Correction Profile")),
        Param::new("profile", tl!("Profile"), ParamKind::choice(opts(&PROFILES), Some("auto".into()))),
        Param::section(tl!("Correction Options")),
        Param::new("correctDistortion", tl!("Geometric Distortion"), ParamKind::bool(true)),
        Param::new("correctCA", tl!("Chromatic Aberration"), ParamKind::bool(false)),
        Param::new("correctVignette", tl!("Vignette"), ParamKind::bool(false)),
        Param::new("autoScale", tl!("Auto Scale Image"), ParamKind::bool(true)),
        Param::new("edge", tl!("Edge"), ParamKind::choice(opts(&EDGES), Some("transparency".into()))),
    ]
}

/// The open documents Add Open Files lists: those saved to a file and unchanged since, in tab order. A modified or
/// never-saved one has nothing on disk to correct.
fn saved_documents(app: &PhotocraftApp) -> Vec<PathBuf> {
    app.session.documents().iter().filter(|d| !d.is_dirty()).filter_map(|d| d.path.clone()).map(PathBuf::from).collect()
}

/// The images of `folder` the batch reads, for Source Files in Folder mode.
pub(crate) fn listing(_app: &PhotocraftApp, folder: &Path) -> Vec<PathBuf> {
    photocraft_engine::file_cmds::list_images(&folder.to_string_lossy()).unwrap_or_default().into_iter().map(PathBuf::from).collect()
}

/// Source Files as the command's `input`: the folder in Folder mode, else the listed paths in list order.
pub(crate) fn command_params(map: &mut Map<String, Value>) {
    let use_folder = map.remove("input.useFolder").and_then(|v| v.as_bool()).unwrap_or(false);
    let folder = map.remove("input.folder");
    if use_folder && let Some(folder) = folder.filter(|f| f.as_str().is_some_and(|f| !f.is_empty())) {
        map.insert("input".into(), folder);
    } else if !map.get("input").is_some_and(Value::is_array) {
        map.insert("input".into(), json!([]));
    }
}

#[cfg(test)]
#[path = "lens_batch_layout_tests.rs"]
mod lens_batch_layout_tests;

#[cfg(test)]
#[path = "lens_batch_defaults_tests.rs"]
mod lens_batch_defaults_tests;
