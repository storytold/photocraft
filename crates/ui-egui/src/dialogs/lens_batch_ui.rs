//! File › Automate › Lens Correction…, laid out like the reference application's dialog (#13981):
//! the groups Source Files, Destination Folder, Lens Correction Profile and Correction Options,
//! top to bottom, the last in two columns.
//!
//! It is the `__form` dialog of `file.automate.lensCorrection` with its own body: the fields are
//! the command's params, so OK, `ui.dialog.set` and `ui.dialog.confirm` work as for every form.

use serde_json::{Map, Value, json};

pub(crate) const COMMAND: &str = photocraft_engine::lens_cmds::LENS_BATCH;

const FORMATS: [(&str, &str); 4] = [("jpg", "JPG"), ("png", "PNG"), ("psd", "PSD"), ("tiff", "TIFF")];
const PROFILES: [(&str, &str); 3] = [("auto", "Auto"), ("generic", "Generic"), ("none", "None")];
const EDGES: [(&str, &str); 4] = [("edgeExtension", "Edge Extension"), ("transparency", "Transparency"), ("black", "Black Color"), ("white", "White Color")];

/// The values each dropdown offers (the form's `__choices`, for `ui.dialog.set` and inspection).
pub(crate) fn choices() -> Value {
    let values = |options: &[(&'static str, &str)]| options.iter().map(|(v, _)| *v).collect::<Vec<_>>();
    json!({"format": values(&FORMATS), "profile": values(&PROFILES), "edge": values(&EDGES)})
}

/// What a field the form lacks, or holds with another type (`ui.dialog.set` with `null`), becomes:
/// the dialog's defaults, with an empty destination. `input` is the Source Files list's own.
fn fallback() -> Value {
    json!({"output": "", "format": "psd", "profile": "auto", "correctDistortion": true, "correctVignette": false, "correctCA": false, "autoScale": true, "edge": "transparency"})
}

/// Whether `f` is the batch Lens Correction form.
pub(crate) fn owns(f: &Map<String, Value>) -> bool {
    f.contains_key("__form") && f.get("__command").and_then(Value::as_str) == Some(COMMAND)
}

/// Why OK is greyed, as in the reference application (#13986): no source file, or no profile
/// (Match Best Profile off and no profile file: today `profile` `none`, or what the engine reads
/// as none). The corrections do not count.
fn missing(f: &Map<String, Value>) -> Option<&'static str> {
    if !super::lens_batch_sources::has_sources(f) {
        Some("Lens Correction needs at least one source file")
    } else if f.get("profile").is_some_and(|p| matches!(p.as_str().unwrap_or("none"), "none" | "")) {
        Some("Lens Correction needs a lens profile: auto or generic")
    } else {
        None
    }
}

pub(crate) fn can_confirm(f: &Map<String, Value>) -> bool {
    !owns(f) || missing(f).is_none()
}

/// `ui.dialog.confirm` refuses what the greyed OK does.
pub(super) fn preflight(ui: &crate::state::UiState, id: u64) -> Result<(), String> {
    match ui.dialogs.iter().find(|d| d.id == id).filter(|d| owns(&d.fields)).and_then(|d| missing(&d.fields)) {
        Some(why) => Err(why.into()),
        None => Ok(()),
    }
}

/// The dialog body: one block per group. A missing or mistyped field is set to its [`fallback`]
/// first, so the dialog shows what OK sends.
pub(crate) fn body(app: &mut crate::PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>, id: u64) {
    if let Value::Object(fallback) = fallback() {
        for (key, value) in fallback {
            if f.get(&key).is_none_or(|v| std::mem::discriminant(v) != std::mem::discriminant(&value)) {
                f.insert(key, value);
            }
        }
    }
    let labels: Vec<&str> = ["Use:", "File Type:", "Profile:"].iter().map(|l| tl!(l)).collect();
    let w = crate::widgets::label_column_width(ui, &labels, 80.0, 160.0);
    heading(ui, tl!("Source Files"));
    super::lens_batch_sources::group(app, ui, f, id, w);
    destination_folder(ui, f, w);
    profile(ui, f, w);
    correction_options(ui, f);
}

/// Destination Folder: the file type and the folder the results are written to.
fn destination_folder(ui: &mut egui::Ui, f: &mut Map<String, Value>, w: f32) {
    heading(ui, tl!("Destination Folder"));
    row(ui, w, tl!("File Type:"), |ui| choice(ui, f, "format", tl!("File Type"), &FORMATS));
    row(ui, w, "", |ui| folder(ui, f, "output", tl!("Output"), tl!("Choose…")));
}

/// Lens Correction Profile: today's profile choice.
fn profile(ui: &mut egui::Ui, f: &mut Map<String, Value>, w: f32) {
    heading(ui, tl!("Lens Correction Profile"));
    row(ui, w, tl!("Profile:"), |ui| choice(ui, f, "profile", tl!("Profile"), &PROFILES));
}

/// Correction Options: the corrections on the left, scaling and the edge fill on the right.
fn correction_options(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    heading(ui, tl!("Correction Options"));
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            check(ui, f, "correctDistortion", tl!("Geometric Distortion"));
            check(ui, f, "correctCA", tl!("Chromatic Aberration"));
            check(ui, f, "correctVignette", tl!("Vignette"));
        });
        ui.add_space(32.0);
        ui.vertical(|ui| {
            check(ui, f, "autoScale", tl!("Auto Scale Image"));
            ui.horizontal(|ui| {
                ui.label(tl!("Edge:"));
                choice(ui, f, "edge", tl!("Edge"), &EDGES);
            });
        });
    });
}

/// A folder field `key` and its picker button.
fn folder(ui: &mut egui::Ui, f: &mut Map<String, Value>, key: &str, name: &str, button: &str) {
    let mut path = text(f, key);
    if ui.add(egui::TextEdit::singleline(&mut path).desired_width(220.0)).accessible_name(name).changed() {
        f.insert(key.into(), json!(path));
    }
    if crate::widgets::secondary_button(ui, button, 0.0).clicked() {
        f.insert(crate::dialogs::BROWSE_FIELD.into(), json!(key));
    }
}

fn check(ui: &mut egui::Ui, f: &mut Map<String, Value>, key: &str, label: &str) {
    let mut on = f.get(key).and_then(Value::as_bool).unwrap_or_default();
    if crate::widgets::checkbox(ui, &mut on, label).changed() {
        f.insert(key.into(), json!(on));
    }
}

/// A dropdown of the text field `key`'s values.
fn choice(ui: &mut egui::Ui, f: &mut Map<String, Value>, key: &str, name: &str, options: &[(&str, &str)]) {
    let mut current = text(f, key);
    let options: Vec<(String, &str)> = options.iter().map(|(v, label)| (v.to_string(), *label)).collect();
    if crate::widgets::dropdown(ui, &format!("lens-batch-{key}"), name, &mut current, &options, 140.0) {
        f.insert(key.into(), json!(current));
    }
}

fn text(f: &Map<String, Value>, key: &str) -> String {
    f.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn heading(ui: &mut egui::Ui, title: &str) {
    ui.add_space(8.0);
    crate::widgets::section_label(ui, title);
    crate::widgets::hairline(ui);
}

/// A row of a group: `label` in the label column, then the controls.
fn row(ui: &mut egui::Ui, w: f32, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        crate::widgets::column_label(ui, label, w);
        add(ui);
    });
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
