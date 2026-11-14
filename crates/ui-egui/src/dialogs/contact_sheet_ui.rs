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

/// The top of the size and font size fields.
const FIELD_MAX: f64 = 300_000.0;
/// Resolution in pixels/inch.
const RES_MIN: f64 = 1.0;
const RES_MAX: f64 = 10_000.0;
/// The top of the Columns and Rows fields.
const GRID_MAX: f64 = 100.0;

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

/// The dialog's defaults: what a first open and Reset… show.
pub(crate) fn defaults() -> Value {
    json!({"input": "", "units": "inches", "width": 8.0, "height": 10.0, "resolution": 300.0, "mode": "rgb", "depth": 8, "columns": 5, "rows": 6, "placeAcrossFirst": true, "autoSpacing": true, "rotateForBestFit": false, "caption": true, "font": photocraft_text::fonts::DEFAULT_FAMILY, "fontSize": 12.0, "flatten": false, "includeSubfolders": true, "groupByFolder": false})
}

/// The fields the dialog opens with: `defaults` under the values of the last OK (#13695), kept in
/// the preferences so they survive a restart. Each stored value the dialog cannot show gives its
/// default (see [`settings`]).
pub(crate) fn opening(app: &PhotocraftApp, mut defaults: Value) -> Value {
    let saved = app.session.prefs().dialogs.get(COMMAND).and_then(Value::as_object).map(settings);
    if let (Some(saved), Some(fields)) = (saved, defaults.as_object_mut()) {
        fields.extend(saved);
    }
    defaults
}

/// OK ran the command: keep the dialog's values for the next open. The Files list (#13686) is not
/// among them, and Cancel never reaches here, so the next open shows the last OK'd run.
pub(crate) fn remember(app: &mut PhotocraftApp, f: &Map<String, Value>) {
    let saved = settings(f);
    app.session.prefs.edit(|p| p.dialogs.insert(COMMAND.into(), Value::Object(saved)));
}

/// The values of `f` the dialog keeps: each one its control can show, whole numbers as integers.
/// A value it cannot show (a hand-edited preferences file, a value set over the control channel)
/// and a key without a control are left out, so the dialog shows its default there. Units, Width
/// and Height are kept only all three together, since a width means nothing without its units.
fn settings(f: &Map<String, Value>) -> Map<String, Value> {
    let within = |v: &Value, lo: f64, hi: f64| v.as_f64().filter(|n| (lo..=hi).contains(n)).map(|n| json!(n));
    let whole = |v: &Value, hi: f64| v.as_f64().filter(|n| n.fract() == 0.0 && (0.0..=hi).contains(n)).map(|n| json!(n as u64));
    let one_of = |v: &Value, ids: &[&str]| v.as_str().filter(|s| ids.contains(s)).map(|s| json!(s));
    let mut kept = Map::new();
    for (key, v) in f {
        let v = match key.as_str() {
            "input" | "font" => v.is_string().then(|| v.clone()),
            RES_UNIT => one_of(v, &RES_UNITS.map(|(id, _)| id)),
            "mode" => one_of(v, &MODES.map(|(id, _)| id)),
            "depth" => whole(v, f64::MAX).filter(|d| DEPTHS.iter().any(|(id, _)| d.as_u64() == Some(*id))),
            "fontSize" => within(v, 0.0, FIELD_MAX),
            "resolution" => within(v, RES_MIN, RES_MAX),
            "columns" | "rows" => whole(v, GRID_MAX),
            "includeSubfolders" | "groupByFolder" | "placeAcrossFirst" | "autoSpacing" | "rotateForBestFit" | "caption" | "flatten" => {
                v.is_boolean().then(|| v.clone())
            }
            _ => None,
        };
        if let Some(v) = v {
            kept.insert(key.clone(), v);
        }
    }
    let size = (
        f.get("units").and_then(|v| one_of(v, &UNITS.map(|(id, _)| id))),
        f.get("width").and_then(|v| within(v, 0.0, FIELD_MAX)),
        f.get("height").and_then(|v| within(v, 0.0, FIELD_MAX)),
    );
    if let (Some(units), Some(width), Some(height)) = size {
        kept.extend([("units".into(), units), ("width".into(), width), ("height".into(), height)]);
    }
    kept
}

/// The dialog body: one block per group, then Load…, Save… and Reset… (#13698).
pub(crate) fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>, id: u64) {
    let labels = ["Input:", "Units:", "Width:", "Height:", "Resolution:", "Mode:", "Bit Depth:", "Place:", "Columns:", "Rows:", "Font:", "Font size:"];
    let labels: Vec<&str> = labels.iter().map(|l| tl!(l)).collect();
    let w = crate::widgets::label_column_width(ui, &labels, 90.0, 160.0);
    source_images(ui, f, w);
    document(ui, f, w);
    thumbnails(ui, f, w);
    caption(ui, f, w);
    settings_buttons(app, ui, f, id);
}

/// Load… and Save… read and write the dialog's values as a settings file
/// ([`settings_file`]) through the platform's file dialog; Reset… asks before restoring the
/// defaults.
fn settings_buttons(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &Map<String, Value>, id: u64) {
    ui.add_space(8.0);
    let r = ui
        .horizontal(|ui| {
            if crate::widgets::secondary_button(ui, tl!("Load…"), 0.0).clicked() {
                return load(app, id);
            }
            if crate::widgets::secondary_button(ui, tl!("Save…"), 0.0).clicked() {
                let bytes = settings_file::write(f, &settings_file::Stamp::now()).into_bytes();
                return app.pick_save(settings_file::FILE_NAME, move |app, path| {
                    let write = app.services.write.as_mut().ok_or("no writer configured")?;
                    write(&path, &bytes)?;
                    Ok(json!({"path": path}))
                });
            }
            if crate::widgets::secondary_button(ui, tl!("Reset…"), 0.0).clicked() {
                ask_reset(app, id);
            }
            Ok(Value::Null)
        })
        .inner;
    if let Err(e) = r
        && e != crate::file_dialog::CANCELLED
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// The largest settings file Load… reads: the reference's are under 1 KiB.
const SETTINGS_FILE_MAX: usize = 1 << 20;

/// Load…: the values of the chosen settings file the dialog can show go into dialog `id`; the
/// others keep the dialog's (see [`settings`]). A file that is not a settings file shows an
/// alert and changes nothing.
fn load(app: &mut PhotocraftApp, id: u64) -> Result<Value, String> {
    app.pick_file_bytes_filtered(&["xml"], move |app, name, bytes| {
        // The dialog may close while the picker is open; then there is nothing to fill.
        let Some(d) = app.ui.dialogs.iter_mut().find(|d| d.id == id) else { return Ok(Value::Null) };
        let read = if bytes.len() > SETTINGS_FILE_MAX {
            Err(tl!("too large").to_string())
        } else {
            std::str::from_utf8(&bytes).map_err(|e| e.to_string()).and_then(|xml| settings_file::read(xml, &d.fields))
        };
        match read {
            Ok(read) => d.fields.extend(settings(&read)),
            // The alert reports it; an `Err` would repeat it in the status bar.
            Err(e) => {
                let message = format!("{}: {e}", crate::file_open::display_name(&name));
                app.ui.open_dialog(DialogKind::Error, json!({ "message": message }).as_object().cloned().unwrap_or_default());
            }
        }
        Ok(Value::Null)
    })
}

/// Marks Reset…'s question; holds the id of the dialog it resets.
const RESET: &str = "__contactSheetReset";

/// Whether `f` is Reset…'s question.
pub(crate) fn owns_reset(f: &Map<String, Value>) -> bool {
    f.contains_key(RESET)
}

/// Reset… asks first, as the reference application's script alert does.
fn ask_reset(app: &mut PhotocraftApp, id: u64) {
    if app.ui.dialogs.iter().any(|d| owns_reset(&d.fields)) {
        return;
    }
    let f = json!({ RESET: id, "__label": "Script Alert" });
    app.ui.open_dialog(DialogKind::Command, f.as_object().cloned().unwrap_or_default());
}

pub(crate) fn reset_body(ui: &mut egui::Ui) {
    ui.label(tl!("Restore default settings?"));
}

/// Yes: the dialog shows its defaults, with no folder chosen and no Files list. No (Cancel)
/// changes nothing.
pub(crate) fn confirm_reset(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    let id = f.get(RESET).and_then(Value::as_u64).ok_or("no Contact Sheet II dialog to reset")?;
    let Some(d) = app.ui.dialogs.iter_mut().find(|d| d.id == id) else { return Ok(Value::Null) };
    // The dialog's own markers stay; its values, the resolution unit included, go.
    let mut fields: Map<String, Value> = d.fields.iter().filter(|(k, _)| k.starts_with("__") && *k != RES_UNIT).map(|(k, v)| (k.clone(), v.clone())).collect();
    fields.extend(defaults().as_object().cloned().unwrap_or_default());
    d.fields = fields;
    Ok(Value::Null)
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
            if crate::widgets::value_field_prec(ui, &mut v, 0.0..=FIELD_MAX as f32, "", 90.0, decimals).accessible_name(name).changed() {
                f.insert(key.into(), json!(exact(v)));
            }
        });
    }
    row(ui, w, tl!("Resolution:"), |ui| {
        let mut unit = text(f, RES_UNIT, "in");
        let k = if unit == "cm" { CM } else { 1.0 };
        let mut v = (res / k) as f32;
        if crate::widgets::value_field_prec(ui, &mut v, ((RES_MIN / k) as f32)..=((RES_MAX / k) as f32), "", 90.0, 2)
            .accessible_name(tl!("Resolution"))
            .changed()
        {
            // The f32 field's ends read back a hair outside the range in pixels/cm.
            f.insert("resolution".into(), json!((exact(v) * k).clamp(RES_MIN, RES_MAX)));
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
            if crate::widgets::value_field_whole(ui, &mut v, 0.0..=GRID_MAX as f32, "", 90.0).accessible_name(name).changed() {
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
            if crate::widgets::value_field(ui, &mut v, 0.0..=FIELD_MAX as f32, "pt", 90.0).accessible_name(tl!("Font size")).changed() {
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

#[path = "contact_sheet_settings_file.rs"]
mod settings_file;

#[cfg(test)]
#[path = "contact_sheet_layout_tests.rs"]
mod contact_sheet_layout_tests;

#[cfg(test)]
#[path = "contact_sheet_ok_validation_tests.rs"]
mod contact_sheet_ok_validation_tests;

#[cfg(test)]
#[path = "contact_sheet_sticky_tests.rs"]
mod contact_sheet_sticky_tests;
