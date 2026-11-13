//! File › Automate › Contact Sheet II…, laid out like the reference application's dialog (#13683):
//! the groups Source Images, Document, Thumbnails and Use Filename as Caption, top to bottom.
//!
//! It is the `__form` dialog of `file.automate.contactSheetII` with its own body: the fields are
//! the command's params (`resolution` in pixels/inch, `width` and `height` in `units`), so OK,
//! `ui.dialog.set` and `ui.dialog.confirm` work as for every form. The resolution unit is view
//! state (`__resUnit`).

use photocraft_engine::prefs::Unit;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;

pub(crate) const COMMAND: &str = "file.automate.contactSheetII";

/// The reference application's alert for OK while no image folder is chosen (#13692).
pub(crate) const NO_FOLDER_ALERT: &str = "Select an Image Folder";

/// The resolution unit field: `cm` shows `resolution` per centimetre, anything else per inch.
const RES_UNIT: &str = "__resUnit";
/// Centimetres per inch.
const CM: f64 = 2.54;

const UNITS: [(&str, &str); 3] = [("inches", "inches"), ("cm", "cm"), ("pixels", "pixels")];
const RES_UNITS: [(&str, &str); 2] = [("in", "pixels/inch"), ("cm", "pixels/cm")];
const MODES: [(&str, &str); 4] = [("gray", "Grayscale"), ("rgb", "RGB Color"), ("cmyk", "CMYK Color"), ("lab", "Lab Color")];
const DEPTHS: [(u64, &str); 2] = [(8, "8-bit"), (16, "16-bit")];
const PLACES: [(bool, &str); 2] = [(true, "across first"), (false, "down first")];

/// Whether `f` is the Contact Sheet II form.
pub(crate) fn owns(f: &Map<String, Value>) -> bool {
    f.get("__command").and_then(Value::as_str) == Some(COMMAND)
}

/// Checked before the dialog closes (`dialogs::confirm_preflight`): OK without a folder shows
/// [`NO_FOLDER_ALERT`] over the dialog, which stays open with its values. A folder that holds no
/// images or is gone still reaches the command and gets its error.
pub(crate) fn preflight(app: &mut PhotocraftApp, id: u64) -> Result<(), String> {
    let Some(f) = app.ui.dialogs.iter().find(|d| d.id == id).map(|d| &d.fields).filter(|f| owns(f)) else { return Ok(()) };
    // The source as the command reads it.
    let chosen = match f.get("input").or_else(|| f.get("files")) {
        None | Some(Value::Null) => false,
        Some(Value::String(dir)) => !dir.is_empty(),
        Some(_) => true,
    };
    if chosen {
        return Ok(());
    }
    let message = tl!(NO_FOLDER_ALERT).to_owned();
    // Repeated confirms (automation) keep one alert.
    let shown = app.ui.dialogs.iter().any(|d| d.kind == DialogKind::Error && d.fields.get("message").and_then(Value::as_str) == Some(message.as_str()));
    if !shown {
        app.ui.open_dialog(DialogKind::Error, json!({ "message": message }).as_object().cloned().unwrap_or_default());
    }
    Err(message)
}

/// The dialog body: one block per group.
pub(crate) fn body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let labels = ["Input:", "Units:", "Width:", "Height:", "Resolution:", "Mode:", "Bit Depth:", "Place:", "Columns:", "Rows:", "Font:", "Font size:"];
    let labels: Vec<&str> = labels.iter().map(|l| tl!(l)).collect();
    let w = crate::widgets::label_column_width(ui, &labels, 90.0, 160.0);
    source_images(ui, f, w);
    document(ui, f, w);
    thumbnails(ui, f, w);
    caption(ui, f, w);
}

/// Source Images: the folder the images are read from, then Include Subfolders and Group Images
/// by Folder (#13687), the latter greyed while Include Subfolders is off.
fn source_images(ui: &mut egui::Ui, f: &mut Map<String, Value>, w: f32) {
    heading(ui, tl!("Source Images"));
    row(ui, w, tl!("Input:"), |ui| {
        let mut input = text(f, "input", "");
        if ui.add(egui::TextEdit::singleline(&mut input).hint_text(tl!("[Select Image Folder]")).desired_width(220.0)).accessible_name(tl!("Input")).changed() {
            f.insert("input".into(), json!(input));
        }
        if crate::widgets::secondary_button(ui, tl!("Browse…"), 0.0).clicked() {
            f.insert(crate::dialogs::BROWSE_FIELD.into(), json!("input"));
        }
    });
    checkbox_row(ui, f, w, "includeSubfolders", tl!("Include Subfolders"), true);
    let subfolders = flag(f, "includeSubfolders", true);
    row(ui, w, "", |ui| {
        ui.add_enabled_ui(subfolders, |ui| {
            let mut on = flag(f, "groupByFolder", false);
            let tip = tl!("Images in subfolders will start on a new Contact Sheet");
            let resp = crate::widgets::checkbox(ui, &mut on, tl!("Group Images by Folder")).on_hover_text(tip).on_disabled_hover_text(tip);
            if resp.changed() {
                f.insert("groupByFolder".into(), json!(on));
            }
        });
    });
}

/// Document: the sheet's size, resolution, layers, mode and depth.
fn document(ui: &mut egui::Ui, f: &mut Map<String, Value>, w: f32) {
    heading(ui, tl!("Document"));
    let res = number(f, "resolution", 300.0);
    row(ui, w, tl!("Units:"), |ui| {
        let mut units = text(f, "units", "inches");
        let before = units.clone();
        if choice(ui, "cs2-units", tl!("Units"), &mut units, &UNITS) {
            convert_size(f, &before, &units, res);
            f.insert("units".into(), json!(units));
        }
    });
    let units = text(f, "units", "inches");
    let decimals = unit_of(&units).decimals() as u32;
    for (key, label, name, d) in [("width", tl!("Width:"), tl!("Width"), 8.0), ("height", tl!("Height:"), tl!("Height"), 10.0)] {
        row(ui, w, label, |ui| {
            let mut v = number(f, key, d) as f32;
            if crate::widgets::value_field_prec(ui, &mut v, 0.0..=300_000.0, "", 90.0, decimals).accessible_name(name).changed() {
                f.insert(key.into(), json!(exact(v)));
            }
        });
    }
    row(ui, w, tl!("Resolution:"), |ui| {
        let mut unit = text(f, RES_UNIT, "in");
        let k = if unit == "cm" { CM } else { 1.0 };
        let mut v = (res / k) as f32;
        if crate::widgets::value_field_prec(ui, &mut v, (1.0 / k as f32)..=(10_000.0 / k as f32), "", 90.0, 2).accessible_name(tl!("Resolution")).changed() {
            f.insert("resolution".into(), json!(exact(v) * k));
        }
        if choice(ui, "cs2-res-unit", tl!("Resolution Unit"), &mut unit, &RES_UNITS) {
            f.insert(RES_UNIT.into(), json!(unit));
        }
    });
    checkbox_row(ui, f, w, "flatten", tl!("Flatten All Layers"), false);
    row(ui, w, tl!("Mode:"), |ui| {
        let mut mode = text(f, "mode", "rgb");
        if choice(ui, "cs2-mode", tl!("Mode"), &mut mode, &MODES) {
            f.insert("mode".into(), json!(mode));
        }
    });
    row(ui, w, tl!("Bit Depth:"), |ui| {
        let mut depth = f.get("depth").and_then(Value::as_f64).map_or(8, |d| d as u64);
        if dropdown(ui, "cs2-depth", tl!("Bit Depth"), &mut depth, &DEPTHS) {
            f.insert("depth".into(), json!(depth));
        }
    });
}

/// Thumbnails: the order and grid of the images on a sheet.
fn thumbnails(ui: &mut egui::Ui, f: &mut Map<String, Value>, w: f32) {
    heading(ui, tl!("Thumbnails"));
    row(ui, w, tl!("Place:"), |ui| {
        let mut across = flag(f, "placeAcrossFirst", true);
        if dropdown(ui, "cs2-place", tl!("Place"), &mut across, &PLACES) {
            f.insert("placeAcrossFirst".into(), json!(across));
        }
    });
    for (key, label, name, d) in [("columns", tl!("Columns:"), tl!("Columns"), 5.0), ("rows", tl!("Rows:"), tl!("Rows"), 6.0)] {
        row(ui, w, label, |ui| {
            let mut v = number(f, key, d) as f32;
            if crate::widgets::value_field_whole(ui, &mut v, 0.0..=100.0, "", 90.0).accessible_name(name).changed() {
                f.insert(key.into(), json!(v.round() as i64));
            }
        });
    }
    checkbox_row(ui, f, w, "rotateForBestFit", tl!("Rotate For Best Fit"), false);
    checkbox_row(ui, f, w, "autoSpacing", tl!("Use Auto-Spacing"), true);
}

/// Use Filename as Caption: the group's title is its checkbox, which greys the font settings.
fn caption(ui: &mut egui::Ui, f: &mut Map<String, Value>, w: f32) {
    ui.add_space(8.0);
    let mut on = flag(f, "caption", true);
    if crate::widgets::checkbox(ui, &mut on, tl!("Use Filename as Caption")).changed() {
        f.insert("caption".into(), json!(on));
    }
    crate::widgets::hairline(ui);
    ui.add_enabled_ui(on, |ui| {
        row(ui, w, tl!("Font:"), |ui| {
            let mut font = text(f, "font", photocraft_text::fonts::DEFAULT_FAMILY);
            if ui.add(egui::TextEdit::singleline(&mut font).desired_width(160.0)).accessible_name(tl!("Font")).changed() {
                f.insert("font".into(), json!(font));
            }
        });
        row(ui, w, tl!("Font size:"), |ui| {
            let mut v = number(f, "fontSize", 12.0) as f32;
            if crate::widgets::value_field(ui, &mut v, 0.0..=300_000.0, "pt", 90.0).accessible_name(tl!("Font size")).changed() {
                f.insert("fontSize".into(), json!(exact(v)));
            }
        });
    });
}

/// Width and height read in `units` at `ppi` (as the command reads them), so switching the units
/// keeps the sheet's size: 8 x 10 in is 20.32 x 25.4 cm, and 2400 x 3000 px at 300 ppi.
fn convert_size(f: &mut Map<String, Value>, from: &str, to: &str, ppi: f64) {
    let (from, to) = (unit_of(from), unit_of(to));
    for (key, d) in [("width", 8.0), ("height", 10.0)] {
        let v = to.from_px(from.to_px(number(f, key, d), ppi, 0.0, 72.0), ppi, 0.0, 72.0);
        f.insert(key.into(), json!(if to == Unit::Pixels { v.round() } else { v }));
    }
}

/// A field's value as the number it shows (`20.33`, not the f32's `20.329999923706055`).
fn exact(v: f32) -> f64 {
    v.to_string().parse().unwrap_or(f64::from(v))
}

/// The unit a `units` value stands for, read as the command reads it (inches when unknown).
fn unit_of(units: &str) -> Unit {
    match units {
        "cm" => Unit::Centimeters,
        "mm" => Unit::Millimeters,
        "px" | "pixels" => Unit::Pixels,
        _ => Unit::Inches,
    }
}

fn heading(ui: &mut egui::Ui, title: &str) {
    ui.add_space(8.0);
    crate::widgets::section_label(ui, title);
    crate::widgets::hairline(ui);
}

/// A row of the group: `label` in the label column, then the controls.
fn row(ui: &mut egui::Ui, w: f32, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        crate::widgets::column_label(ui, label, w);
        add(ui);
    });
}

fn checkbox_row(ui: &mut egui::Ui, f: &mut Map<String, Value>, w: f32, key: &str, label: &str, d: bool) {
    row(ui, w, "", |ui| {
        let mut on = flag(f, key, d);
        if crate::widgets::checkbox(ui, &mut on, label).changed() {
            f.insert(key.into(), json!(on));
        }
    });
}

fn dropdown<T: PartialEq + Clone>(ui: &mut egui::Ui, id: &str, name: &str, current: &mut T, options: &[(T, &str)]) -> bool {
    crate::widgets::dropdown(ui, id, name, current, options, 140.0)
}

/// A [`dropdown`] of a text field's values.
fn choice(ui: &mut egui::Ui, id: &str, name: &str, current: &mut String, options: &[(&str, &str)]) -> bool {
    let options: Vec<(String, &str)> = options.iter().map(|(v, label)| (v.to_string(), *label)).collect();
    dropdown(ui, id, name, current, &options)
}

fn text(f: &Map<String, Value>, key: &str, d: &str) -> String {
    f.get(key).and_then(Value::as_str).unwrap_or(d).to_string()
}

fn number(f: &Map<String, Value>, key: &str, d: f64) -> f64 {
    f.get(key).and_then(Value::as_f64).unwrap_or(d)
}

fn flag(f: &Map<String, Value>, key: &str, d: bool) -> bool {
    f.get(key).and_then(Value::as_bool).unwrap_or(d)
}

#[cfg(test)]
#[path = "contact_sheet_layout_tests.rs"]
mod contact_sheet_layout_tests;

#[cfg(test)]
#[path = "contact_sheet_ok_validation_tests.rs"]
mod contact_sheet_ok_validation_tests;
