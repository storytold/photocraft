//! File › Automate › Lens Correction…, laid out like the reference application's dialog (#13981):
//! the sections Source Files, Destination Folder, Lens Correction Profile and Correction Options,
//! top to bottom, with the reference captions and the Edge entries in the reference order.
//!
//! The rows are the command's params, so OK, `ui.dialog.set` and `ui.dialog.confirm` work as for every form. Source
//! Files (#13979) is the framework's files row: Use (Files or Folder), the list, Browse…, Remove and Add Open Files;
//! [`command_params`] turns it into the command's `input`, the listed paths in list order or the folder.

use std::path::{Path, PathBuf};

use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamKind, ParamResult};
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

/// Whether Source Files listed a file when [`derive`] last looked.
const HAD_SOURCES: &str = "hadSources";

/// The rows of the dialog. The destination starts empty (#13982). They open with the reference defaults (#13980): Chromatic
/// Aberration and Vignette off, File Type PSD; the command's own defaults for scripts and the CLI are unchanged. Correction Options lists the corrections, then scaling and
/// the edge fill, as the reference's left and right columns read.
pub(crate) fn rows(app: &PhotocraftApp) -> Vec<Param> {
    vec![
        Param::section(tl!("Source Files")),
        Param::files("input", tl!("Source Files")).folder_mode(true).open_files(saved_documents(app)),
        Param::section(tl!("Destination Folder")),
        Param::new("format", tl!("File Type"), ParamKind::choice(opts(&FORMATS), Some("psd".into()))),
        Param::new("output", tl!("Output"), ParamKind::text("")).browse(PathKind::Folder),
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

/// Before each pass, OK and automation write: the first file added to an empty Source Files list (Browse…, Folder,
/// Add Open Files, a list set through `ui.dialog.set`) sets an empty destination to `results` in that file's folder, as
/// in the reference dialog, so an OK right after a script set the list sends it. A destination already set (Choose…,
/// typed, or set through `ui.dialog.set`) is kept.
pub(crate) fn derive(fields: &mut DialogFields) {
    let first = fields.list("input").and_then(|list| {
        list.iter().find_map(|v| match v {
            FieldValue::Text(path) => Some(PathBuf::from(path)),
            _ => None,
        })
    });
    if fields.bool(HAD_SOURCES) != Some(true)
        && fields.text("output").is_none_or(str::is_empty)
        && let Some(results) = first.as_deref().and_then(Path::parent).map(|dir| dir.join("results"))
    {
        fields.set("output", results.to_string_lossy().into_owned());
    }
    if fields.bool(HAD_SOURCES) != Some(first.is_some()) {
        fields.set(HAD_SOURCES, first.is_some());
    }
}

/// Why OK is greyed, as in the reference application (#13986): no source file, or no profile (Match Best Profile off
/// and no profile file: today `profile` `none`). The corrections do not count.
pub(crate) fn missing(fields: &DialogFields) -> Option<&'static str> {
    let listed = fields.list("input").is_some_and(|list| !list.is_empty());
    let folder = fields.bool("input.useFolder") == Some(true) && fields.text("input.folder").is_some_and(|dir| !dir.is_empty());
    if !listed && !folder {
        Some(tl!("Lens Correction needs at least one source file"))
    } else if matches!(fields.text("profile").unwrap_or("none"), "none" | "") {
        Some(tl!("Lens Correction needs a lens profile: auto or generic"))
    } else {
        None
    }
}

/// The `ParamFamily::check` of the dialog: `ui.dialog.confirm` refuses what the greyed OK does.
pub(crate) fn check(_app: &mut PhotocraftApp, result: &ParamResult) -> Result<(), Label> {
    missing(&result.fields).map_or(Ok(()), |why| Err(Label::from(why)))
}

/// Source Files as the command's `input`: the folder in Folder mode, else the listed paths in list order.
pub(crate) fn command_params(map: &mut Map<String, Value>) {
    map.remove(HAD_SOURCES);
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

#[cfg(test)]
#[path = "lens_batch_source_tests.rs"]
mod lens_batch_source_tests;

#[cfg(test)]
#[path = "lens_batch_destination_tests.rs"]
mod lens_batch_destination_tests;
