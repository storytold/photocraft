//! Artboards to Files' Export Options (#13636): a checkbox below File Type, unticked by default,
//! that shows an Options group with the chosen file type's settings. Each type keeps its own
//! settings (shell state in [`OPTS`]); the command gets the flat params of the shown type only
//! while the checkbox is on, so an export without it writes what it wrote before. Both are
//! remembered after a successful run, never after a failed one.

use super::{Map, PhotocraftApp, Value, json};

pub(super) const COMMAND: &str = "file.export.artboardsToFiles";
/// The checkbox (a field, so it shows in `__order`; the command ignores it).
pub(super) const CHECKBOX: &str = "exportOptions";
/// Every type's settings, `{type: {param: value}}`: shell state, never a command parameter.
pub(super) const OPTS: &str = "__exportOpts";
/// The params the group sends; the form renders none of them itself.
pub(super) const PARAMS: [&str; 9] =
    ["iccProfile", "bmpBits", "tgaBits", "pngInterlaced", "transparency", "tiffCompression", "maximizeCompatibility", "preserveArtboard", "quality"];

/// The settings key of a File Type value (`jpg` and `jpeg` share one, as `tif` and `tiff` do).
fn kind(format: &str) -> Option<&'static str> {
    Some(match format.to_ascii_lowercase().as_str() {
        "psd" | "psb" => "psd",
        "bmp" => "bmp",
        "jpg" | "jpeg" => "jpg",
        "tif" | "tiff" => "tiff",
        "tga" => "tga",
        "png" => "png",
        _ => return None,
    })
}

/// The reference application's defaults for each type.
fn defaults() -> Value {
    json!({
        "psd": {"maximizeCompatibility": true, "preserveArtboard": true, "iccProfile": true},
        "bmp": {"bmpBits": 24, "iccProfile": true},
        "tga": {"tgaBits": 24, "iccProfile": true},
        "jpg": {"quality": 8.0, "iccProfile": true},
        "tiff": {"tiffCompression": "none", "quality": 8.0, "iccProfile": true},
        "png": {"transparency": true, "pngInterlaced": false, "iccProfile": true},
    })
}

/// Whether `v` is a valid value for `key` (a hand-edited preferences file may hold anything).
fn valid(key: &str, v: &Value) -> bool {
    match key {
        "bmpBits" | "tgaBits" => v.as_u64().is_some_and(|b| [16, 24, 32].contains(&b)),
        "tiffCompression" => v.as_str().is_some_and(|c| ["none", "lzw", "zip", "jpeg"].contains(&c)),
        "quality" => v.as_f64().is_some_and(|q| (0.0..=12.0).contains(&q)),
        _ => v.is_boolean(),
    }
}

/// The open dialog's fields: the checkbox and every type's settings, the defaults overlaid with
/// what the last successful run remembered.
pub(super) fn open_fields(app: &PhotocraftApp) -> (Value, Value) {
    let mut opts = defaults();
    let saved = app.session.prefs().dialogs.get(COMMAND).cloned().unwrap_or(Value::Null);
    if let (Some(all), Some(remembered)) = (opts.as_object_mut(), saved.get("options").and_then(Value::as_object)) {
        for (kind, settings) in all.iter_mut() {
            let (Some(settings), Some(from)) = (settings.as_object_mut(), remembered.get(kind).and_then(Value::as_object)) else { continue };
            for (key, v) in settings.iter_mut() {
                if let Some(r) = from.get(key).filter(|r| valid(key, r)) {
                    *v = r.clone();
                }
            }
        }
    }
    (json!(saved.get(CHECKBOX).and_then(Value::as_bool).unwrap_or(false)), opts)
}

/// Inserts the group's fields into a new dialog's `fields`.
pub(super) fn insert_fields(app: &PhotocraftApp, fields: &mut Value) {
    let (on, opts) = open_fields(app);
    if let Some(f) = fields.as_object_mut() {
        f.insert(CHECKBOX.into(), on);
        f.insert(OPTS.into(), opts);
        for p in PARAMS {
            f.insert(p.into(), Value::Null);
        }
    }
    sync_params(fields.as_object_mut());
}

/// Remembers the checkbox and every type's settings after a successful run of the dialog.
pub(crate) fn remember(app: &mut PhotocraftApp, command: &str, fields: &Map<String, Value>) {
    if command != COMMAND || !fields.contains_key(OPTS) {
        return;
    }
    let saved = json!({CHECKBOX: fields.get(CHECKBOX).and_then(Value::as_bool).unwrap_or(false), "options": fields.get(OPTS).cloned().unwrap_or(Value::Null)});
    app.session.prefs.edit(|prefs| prefs.dialogs.insert(COMMAND.into(), saved));
}

/// Writes the flat params for the command: the shown type's settings while the checkbox is on,
/// else none. TIFF sends its quality only with JPEG compression.
fn sync_params(f: Option<&mut Map<String, Value>>) {
    let Some(f) = f else { return };
    for p in PARAMS {
        f.insert(p.into(), Value::Null);
    }
    if f.get(CHECKBOX).and_then(Value::as_bool) != Some(true) {
        return;
    }
    let Some(kind) = f.get("format").and_then(Value::as_str).and_then(kind) else { return };
    let Some(settings) = f.get(OPTS).and_then(|o| o.get(kind)).and_then(Value::as_object).cloned() else { return };
    let jpeg_tiff = settings.get("tiffCompression").and_then(Value::as_str) == Some("jpeg");
    for (key, v) in settings {
        if kind == "tiff" && key == "quality" && !jpeg_tiff {
            continue;
        }
        f.insert(key, v);
    }
}

fn get_bool(s: &Map<String, Value>, key: &str) -> bool {
    s.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn checkbox(ui: &mut egui::Ui, s: &mut Map<String, Value>, key: &str, label: &str) {
    let mut on = get_bool(s, key);
    crate::widgets::checkbox(ui, &mut on, label);
    s.insert(key.into(), json!(on));
}

/// The Quality slider and field, 0-12: a typed value past either end is clamped, fractions are
/// kept, and text that is no number leaves the value as it was.
fn quality(ui: &mut egui::Ui, s: &mut Map<String, Value>) {
    let mut q = s.get("quality").and_then(Value::as_f64).unwrap_or(8.0).clamp(0.0, 12.0) as f32;
    crate::widgets::slider_line(ui, tl!("Quality"), 70.0, &mut q, 0.0..=12.0, "");
    s.insert("quality".into(), json!(f64::from(q.clamp(0.0, 12.0))));
}

/// Depth radios for BMP and Targa.
fn depth(ui: &mut egui::Ui, s: &mut Map<String, Value>, key: &str) {
    let mut bits = s.get(key).and_then(Value::as_u64).unwrap_or(24);
    ui.horizontal(|ui| {
        ui.label(tl!("Depth:"));
        for (b, label) in [(16, tl!("16bit")), (24, tl!("24bit")), (32, tl!("32bit"))] {
            ui.radio_value(&mut bits, b, label);
        }
    });
    s.insert(key.into(), json!(bits));
}

/// The Export Options checkbox and, while it is ticked, the Options group of the chosen type.
pub(super) fn group(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let mut on = f.get(CHECKBOX).and_then(Value::as_bool).unwrap_or(false);
    let kind = f.get("format").and_then(Value::as_str).and_then(kind);
    ui.vertical(|ui| {
        crate::widgets::checkbox(ui, &mut on, tl!("Export Options"));
        let Some(kind) = kind.filter(|_| on) else { return };
        let Some(Value::Object(all)) = f.get_mut(OPTS) else { return };
        let Some(Value::Object(s)) = all.get_mut(kind) else { return };
        ui.label(tl!("Options:"));
        ui.group(|ui| {
            ui.set_min_width(300.0);
            match kind {
                "psd" => {
                    checkbox(ui, s, "maximizeCompatibility", tl!("Maximize Compatibility"));
                    checkbox(ui, s, "preserveArtboard", tl!("Preserve Artboard"));
                }
                "bmp" => depth(ui, s, "bmpBits"),
                "tga" => depth(ui, s, "tgaBits"),
                "jpg" => quality(ui, s),
                "tiff" => {
                    let mut c = s.get("tiffCompression").and_then(Value::as_str).unwrap_or("none").to_string();
                    let opts = [
                        ("none".to_string(), tl!("None")),
                        ("lzw".to_string(), tl!("LZW")),
                        ("zip".to_string(), tl!("ZIP")),
                        ("jpeg".to_string(), tl!("JPEG")),
                    ];
                    ui.horizontal(|ui| {
                        ui.label(tl!("Image Compression:"));
                        crate::widgets::dropdown(ui, "artboards-tiff-compression", tl!("Image Compression"), &mut c, &opts, 120.0);
                    });
                    let jpeg = c == "jpeg";
                    s.insert("tiffCompression".into(), json!(c));
                    ui.add_enabled_ui(jpeg, |ui| quality(ui, s));
                }
                "png" => {
                    checkbox(ui, s, "transparency", tl!("Transparency"));
                    checkbox(ui, s, "pngInterlaced", tl!("Interlaced"));
                }
                _ => {}
            }
            checkbox(ui, s, "iccProfile", tl!("Include ICC Profile"));
        });
    });
    f.insert(CHECKBOX.into(), json!(on));
    sync_params(Some(f));
}
