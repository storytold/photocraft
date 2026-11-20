//! File › Automate › Lens Correction…, laid out like the reference application's dialog (#13981):
//! the sections Source Files, Destination Folder, Lens Correction Profile and Correction Options,
//! top to bottom, with the reference captions and the Edge entries in the reference order.
//!
//! The rows are the command's params, so OK, `ui.dialog.set` and `ui.dialog.confirm` work as for every form.

use craft::ui::patterns::form_dialogs::{Param, ParamKind};
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

/// The rows of the dialog, reading the images from `dir`. They open with the reference defaults (#13980): Chromatic
/// Aberration and Vignette off, File Type PSD; the command's own defaults for scripts and the CLI are unchanged. Correction Options lists the corrections, then scaling and
/// the edge fill, as the reference's left and right columns read.
pub(crate) fn rows(dir: String) -> Vec<Param> {
    vec![
        Param::section(tl!("Source Files")),
        folder("input", tl!("Input"), dir.clone()),
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

#[cfg(test)]
#[path = "lens_batch_layout_tests.rs"]
mod lens_batch_layout_tests;

#[cfg(test)]
#[path = "lens_batch_defaults_tests.rs"]
mod lens_batch_defaults_tests;
