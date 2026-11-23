//! File › Export › Export As… (and Quick Export as PNG): format, quality, transparency and scale,
//! with a preview and an estimated file size. The estimate encodes a small proxy and scales by
//! pixel count, so the dialog stays instant on 36 MP documents.

use std::path::PathBuf;
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Mutex;

use craft::behavior::PixelSize;
use craft::cms::Profile;
use craft::ui::contract::SaveKey;
use craft::ui::dialog::{DialogId, DialogResult};
use craft::ui::patterns::form_dialogs::{
    ExportAsResult, ExportFormat, ExportFormatDefaults, ExportMetadata, ExportOptions, ExportPreview, ExportProfile, ExportSize as DialogSize, HostedExportAs,
    PreviewRequest, PreviewState, Resample, ScaleRow, SizeEstimate,
};
use craft::ui::patterns::presets::{GroupPath, NamedPresetEvent, PresetLibrary};
use craft::ui::runtime::requests::PathKind;
use photocraft_doc::{ColorMode, Document, LayerId};
use serde_json::{Map, Value, json};

use photocraft_engine::export_preset_cmds::{DEFAULT_RESAMPLE, export_resample};

use crate::{ExportSettings, PhotocraftApp};

const FORMATS: [(&str, &str); 7] = [("png", "PNG"), ("jpg", "JPG"), ("webp", "WebP"), ("tif", "TIFF"), ("tga", "TGA"), ("ico", "ICO"), ("jxl", "JPEG XL")];

/// The command id Export As records its automation step under (#7207) and replays from.
pub(crate) const COMMAND: &str = "file.export.exportAs";

/// Where the dialog keeps the settings it was last used with (the SAVED tier): it opens with them
/// and `ExportAsResult::remember` writes them after a successful export.
const REMEMBER: SaveKey = SaveKey::new("photocraft.file.export.exportAs");

/// The dialog's own settings: the form of the recorded step (#7207), of a saved export preset and of
/// the pipeline's input below. [`options_of`] and [`settings_from`] convert it to and from the
/// dialog's [`ExportOptions`].
const SETTINGS: [&str; 14] = [
    "format",
    "quality",
    "lossless",
    "transparency",
    "scale",
    "resample",
    "metadata",
    "tgaBits",
    "sizing",
    "width",
    "height",
    "lockAspect",
    "profile",
    "embedProfile",
];

fn settings_of(f: &Map<String, Value>) -> Map<String, Value> {
    SETTINGS.iter().filter_map(|k| f.get(*k).map(|v| ((*k).to_string(), v.clone()))).collect()
}

/// Whether `v` is a value the dialog's `key` control can hold: what a remembered setting is
/// seeded from and what a recorded Export As step replays with (#7207).
fn valid_setting(key: &str, v: &Value) -> bool {
    let num = |lo: f64, hi: f64| v.as_f64().is_some_and(|x| x.is_finite() && (lo..=hi).contains(&x));
    let one_of = |options: &[&str]| v.as_str().is_some_and(|s| options.contains(&s));
    match key {
        "format" => v.as_str().is_some_and(|s| FORMATS.iter().any(|(k, _)| *k == s)),
        "quality" => num(1.0, 100.0),
        "scale" => num(1.0, 1000.0),
        "width" | "height" => num(1.0, MAX_PX),
        "lossless" | "transparency" | "lockAspect" | "embedProfile" => v.is_boolean(),
        "resample" => v.as_str().is_some_and(|s| export_resample(s).is_ok()),
        "metadata" => one_of(&["none", "exif", "all"]),
        "tgaBits" => one_of(&["auto", "24", "32"]),
        "sizing" => one_of(&["scale", "pixels"]),
        "profile" => one_of(&["document", "srgb"]),
        _ => false,
    }
}

/// The dialog's [`ExportOptions`] for the settings map `f`. A key that is missing or not a value
/// [`valid_setting`] accepts keeps the dialog's default, so a damaged saved preset opens as the
/// default does.
pub(crate) fn options_of(f: &Map<String, Value>) -> ExportOptions {
    let f: Map<String, Value> = f.iter().filter(|(k, v)| valid_setting(k, v)).map(|(k, v)| (k.clone(), v.clone())).collect();
    let mut o = ExportOptions::default();
    if let Some(format) = f.get("format").and_then(Value::as_str) {
        o.format = format.to_owned();
    }
    if let Some(quality) = f.get("quality").and_then(Value::as_f64) {
        o.quality = quality.round().clamp(1.0, 100.0) as u8;
    }
    if let Some(v) = f.get("lossless").and_then(Value::as_bool) {
        o.lossless = v;
    }
    if let Some(v) = f.get("transparency").and_then(Value::as_bool) {
        o.transparency = v;
    }
    if let Some(v) = f.get("lockAspect").and_then(Value::as_bool) {
        o.lock_aspect = v;
    }
    if let Some(v) = f.get("embedProfile").and_then(Value::as_bool) {
        o.embed_profile = v;
    }
    o.metadata = match s(&f, "metadata").as_str() {
        "exif" => ExportMetadata::Exif,
        "all" => ExportMetadata::All,
        _ => ExportMetadata::None,
    };
    o.profile = match s(&f, "profile").as_str() {
        "srgb" => ExportProfile::Srgb,
        "document" => ExportProfile::Document,
        _ => o.profile,
    };
    o.bits = match s(&f, "tgaBits").as_str() {
        "24" => 24,
        "32" => 32,
        _ => 0,
    };
    if let Some(method) = f.get("resample").and_then(Value::as_str) {
        o.resample = match method {
            "bicubic" => Resample::Bicubic,
            "bicubicSmoother" => Resample::BicubicSmoother,
            "bicubicSharper" => Resample::BicubicSharper,
            "bilinear" => Resample::Bilinear,
            "nearest" => Resample::NearestNeighbor,
            "preserveDetails" => Resample::PreserveDetails,
            _ => Resample::Automatic,
        };
    }
    let pixels = f.get("width").and_then(Value::as_f64).zip(f.get("height").and_then(Value::as_f64));
    if let (true, Some((w, h))) = (s(&f, "sizing") == "pixels", pixels) {
        o.size = DialogSize::pixels(w.round() as u32, h.round() as u32);
    } else if let Some(scale) = f.get("scale").and_then(Value::as_f64) {
        o.size = DialogSize::Scale(scale);
    }
    o
}

/// The settings map (the [`SETTINGS`] keys) for `options` on a `source`-sized document: the form
/// of the recorded step, of a saved preset and of the export pipeline's input. `width` and
/// `height` are the output size in both sizing modes.
pub(crate) fn settings_from(options: &ExportOptions, source: PixelSize) -> Map<String, Value> {
    let size = options.output_size(source);
    let (sizing, scale) = match options.size {
        DialogSize::Pixels { width, .. } => ("pixels", (f64::from(width) / f64::from(source.width.max(1)) * 100.0).clamp(1.0, 1000.0)),
        DialogSize::Scale(scale) => ("scale", scale),
        _ => ("scale", 100.0),
    };
    let resample = match options.resample {
        Resample::Automatic => "bicubicAutomatic",
        Resample::Bicubic => "bicubic",
        Resample::BicubicSmoother => "bicubicSmoother",
        Resample::BicubicSharper => "bicubicSharper",
        Resample::Bilinear => "bilinear",
        Resample::NearestNeighbor => "nearest",
        Resample::PreserveDetails => "preserveDetails",
        // The dialog offers only `RESAMPLE_CHOICES`; a method outside it cannot be chosen.
        _ => DEFAULT_RESAMPLE,
    };
    let metadata = match options.metadata {
        ExportMetadata::Exif => "exif",
        ExportMetadata::All => "all",
        _ => "none",
    };
    let bits = match options.bits {
        24 => "24",
        32 => "32",
        _ => "auto",
    };
    let mut f = Map::new();
    f.insert("format".into(), json!(options.format));
    f.insert("quality".into(), json!(options.quality));
    f.insert("lossless".into(), json!(options.lossless));
    f.insert("transparency".into(), json!(options.transparency));
    f.insert("scale".into(), json!(scale));
    f.insert("resample".into(), json!(resample));
    f.insert("metadata".into(), json!(metadata));
    f.insert("tgaBits".into(), json!(bits));
    f.insert("sizing".into(), json!(sizing));
    f.insert("width".into(), json!(size.width));
    f.insert("height".into(), json!(size.height));
    f.insert("lockAspect".into(), json!(options.lock_aspect));
    f.insert("profile".into(), json!(if options.profile == ExportProfile::Srgb { "srgb" } else { "document" }));
    f.insert("embedProfile".into(), json!(options.embed_profile));
    f
}

/// What the open Export As dialog needs besides the dialog itself: which layers it exports, the
/// source size its settings are for, the preview in progress and the texture it shows. The dialog
/// has one id, so the app keeps one of these.
pub(crate) struct ExportOpen {
    /// The dialog's id, which the tests read to drive it.
    #[cfg(test)]
    pub(crate) dialog: DialogId,
    /// The layers Layer › Export As exports; empty for the whole document, several for one file each.
    pub(crate) layers: Vec<LayerId>,
    /// The size of the source the dialog fits its size to: the document, or the first layer.
    source: PixelSize,
    /// The preview's texture; the dialog's state holds only its id, so it lives as long as the dialog.
    #[allow(dead_code)] // held for its drop: the texture is freed with the dialog's record
    texture: Option<egui::TextureHandle>,
    /// The save dialog the dialog asked for is open (until it answers or is cancelled).
    picking: bool,
    /// The newest preview the dialog asked for that is not fully answered yet.
    work: Option<PreviewWork>,
    /// The newest request made, with the working profile and the scratch read-error generation it was made under: a
    /// change of the working profile alters the preview of an untagged document without changing the settings, and a
    /// tile that fails to read and recovers alters the pixels without a new revision (`crate::scratch_reads`).
    last: Option<(u64, ExportOptions, Option<String>, u64)>,
}

/// One preview request being answered: the proxy preview first, then the exact size estimate.
struct PreviewWork {
    revision: u64,
    settings: Map<String, Value>,
    signature: String,
    key: egui::Id,
    doc: Arc<Document>,
    profile: Arc<Profile>,
    /// The answer already given (the proxy preview), which the exact estimate then completes.
    answered: Option<ExportPreview>,
}

/// The export formats the dialog offers: the framework's list and JPEG XL, which has no quality.
fn dialog_formats() -> Vec<ExportFormat> {
    let mut formats = ExportFormat::standard();
    formats.push(ExportFormat::new("jxl", "JPEG XL").transparency(true));
    formats
}

/// The resample methods the engine's Image Size implements, Preserve Details included (#13737), in the order of the
/// engine's `RESAMPLE`.
const RESAMPLE_CHOICES: [Resample; 7] = [
    Resample::Bicubic,
    Resample::Automatic,
    Resample::BicubicSharper,
    Resample::BicubicSmoother,
    Resample::Bilinear,
    Resample::NearestNeighbor,
    Resample::PreserveDetails,
];

/// The saved export presets as the dialog's library: the engine's list in its order, each preset's
/// settings read with [`options_of`].
fn preset_library(app: &PhotocraftApp) -> PresetLibrary<ExportOptions> {
    let mut library = PresetLibrary::new();
    for preset in &app.session.presets.export_presets {
        if let Ok(Value::Object(settings)) = serde_json::to_value(&preset.settings) {
            // A name the library refuses (a duplicate ignoring case) is one the engine would not have kept.
            let _ = library.add(&GroupPath::root(), &preset.name, options_of(&settings));
        }
    }
    library
}

/// The file name the dialog suggests, without extension: the document's name.
fn stem_of(name: &str) -> String {
    name.rsplit_once('.').map_or(name, |(a, _)| a).to_string()
}

/// File › Export › Export As…
pub fn open(app: &mut PhotocraftApp) -> Result<DialogId, String> {
    let st = app.session.active().ok_or("no document")?;
    let (stem, size) = (stem_of(&st.doc.name), st.doc.size);
    open_sized(app, stem, None, PixelSize::new(size.width, size.height), Vec::new())
}

/// The dialog fitted to a `source`-sized document or layer. Fitting the document first would
/// replace a remembered pixel size the document can't take with the percent, even when the layer
/// can take it.
fn open_sized(app: &mut PhotocraftApp, stem: String, title: Option<String>, source: PixelSize, layers: Vec<LayerId>) -> Result<DialogId, String> {
    let prefs = app.session.prefs().export.clone();
    let mut options = ExportOptions::default();
    options.quality = u8::try_from(prefs.jpeg_quality.clamp(1, 100)).unwrap_or(85);
    options.lossless = prefs.webp_lossless;
    let mut defaults = ExportFormatDefaults::default();
    defaults.jpeg_quality = u8::try_from(prefs.jpeg_quality.clamp(1, 100)).unwrap_or(85);
    defaults.webp_quality = u8::try_from(prefs.webp_quality.clamp(1, 100)).unwrap_or(85);
    defaults.webp_lossless = prefs.webp_lossless;
    // One file per exportable layer of a selection: a single one is named after its layer, several go into a folder
    // (the dialog asks for it, #15028, #14808).
    let docs = layer_documents(app, &layers);
    let stem = match docs.as_slice() {
        [one] => layer_file_stem(&one.name),
        _ => stem,
    };
    let mut dialog = HostedExportAs::new(stem, source, options)
        .files(docs.len().max(1))
        .formats(dialog_formats())
        .defaults(defaults)
        .methods(RESAMPLE_CHOICES.to_vec())
        .presets(preset_library(app))
        .remember(REMEMBER)
        // The browser build's file dialog answers no folder, so Scale All exports one size at a time there.
        .folder_export(!cfg!(target_arch = "wasm32"));
    if let Some(title) = title {
        dialog = dialog.title(title);
    }
    let id = app.framework.dialogs.open(dialog);
    app.export_open = Some(ExportOpen {
        #[cfg(test)]
        dialog: id,
        layers,
        source,
        texture: None,
        picking: false,
        work: None,
        last: None,
    });
    Ok(id)
}

/// Layer › Export As…: the same dialog for just the active layer (trimmed to its pixels).
pub fn open_layer(app: &mut PhotocraftApp, layer: LayerId) -> Result<DialogId, String> {
    let st = app.session.active().ok_or("no document")?;
    let ldoc = photocraft_engine::layer_menu_cmds::layer_document(&st.doc, layer).map_err(|e| e.to_string())?;
    // The size fields follow the layer's trimmed size, so the dialog shows and edits the size
    // the layer exports at.
    let template = "Export As: {name}";
    let title = crate::i18n::fmt(tl!(template), &[("name", &ldoc.name)]);
    open_sized(app, stem_of(&ldoc.name), Some(title), PixelSize::new(ldoc.size.width, ldoc.size.height), vec![layer])
}

/// Layer › Export As… with several layers (artboards, say) selected: the same dialog, then one
/// file per layer with its settings (#3629). The dialog opens on the first layer with pixels and
/// its preview shows that layer: an empty, adjustment or off-canvas layer no longer stops it
/// (#14806), while any other failure (an unknown id) is the answer.
pub fn open_layers(app: &mut PhotocraftApp, layers: &[LayerId]) -> Result<DialogId, String> {
    match layers {
        [] => return Err("no selected layers".into()),
        [one] => return open_layer(app, *one),
        _ => {}
    }
    let st = app.session.active().ok_or("no document")?;
    let ldoc = exportable_layers(&st.doc, layers)?.swap_remove(0);
    // The batch is named after the document, as Quick Export names a selection's files.
    let stem = export_name(&stem_of(&st.doc.name));
    let template = "Export As: {n} layers";
    let title = crate::i18n::fmt(tl!(template), &[("n", &layers.len().to_string())]);
    open_sized(app, stem, Some(title), PixelSize::new(ldoc.size.width, ldoc.size.height), layers.to_vec())
}

/// What a several-layer export says when no selected layer has pixels, as in the reference
/// application.
const NO_VALID_ASSET: &str = "No valid asset to export. Please resolve any warnings and try again.";

/// The export documents of the selected `ids` that have pixels on the canvas, in order. Empty,
/// adjustment and off-canvas layers are skipped (#14806); any other failure is still an error,
/// and so is a selection with nothing left to export.
fn exportable_layers(doc: &Document, ids: &[LayerId]) -> Result<Vec<Document>, String> {
    let mut docs = Vec::new();
    for id in ids {
        match photocraft_engine::layer_menu_cmds::layer_document(doc, *id) {
            Ok(d) => docs.push(d),
            Err(e) if photocraft_engine::layer_menu_cmds::is_empty_layer(&e) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    if docs.is_empty() {
        return Err(tl!(NO_VALID_ASSET).into());
    }
    Ok(docs)
}

#[cfg(test)]
mod layer_empty_skip_tests;
#[cfg(test)]
mod layer_folder_export_tests;

/// Layer › Quick Export as PNG.
pub fn quick_export_layer_png(app: &mut PhotocraftApp, layer: LayerId) -> Result<Value, String> {
    let mut f = Map::new();
    f.insert("format".into(), json!("png"));
    f.insert("transparency".into(), json!(true));
    f.insert("scale".into(), json!(100));
    f.insert("__layer".into(), json!(layer.0));
    pick_and_export(app, f)
}

/// Export each selected layer separately, with one destination prompt for the batch.
/// The chosen path names the first file; subsequent files have numbered sibling names.
pub fn quick_export_selected_layers_png(app: &mut PhotocraftApp) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document")?;
    let layers = st.selected_layers();
    if layers.len() == 1 {
        return quick_export_layer_png(app, layers[0]);
    }
    if layers.is_empty() {
        return Err("no selected layers".into());
    }
    // Capture all sources before showing the dialog: changing selection or document while it
    // is open cannot change which layers get exported.
    let docs = exportable_layers(&st.doc, layers)?;
    let suggested = format!("{}.png", export_name(&st.doc.name));
    app.pick_export(&suggested, move |app, path| {
        if !path.to_ascii_lowercase().ends_with(".png") {
            return Err("Quick Export as PNG writes .png files".into());
        }
        let paths = batch_paths(&path, &docs);
        let mut exported = Vec::new();
        for (doc, path) in docs.iter().zip(paths) {
            crate::app::ensure_memory_to_save(doc, &path)?;
            let export = app.services.export.as_ref().ok_or("no exporter configured")?;
            let (bytes, warnings) = export(doc, &path, &ExportSettings { xmp_all: false, ..Default::default() })?;
            let write = app.services.write.as_mut().ok_or("no writer configured")?;
            write(&path, &bytes)?;
            crate::notices::io_warnings(app, &format!("Exported {}", crate::file_open::display_name(&path)), &warnings);
            exported.push(json!({"path": path, "bytes": bytes.len(), "warnings": warnings}));
        }
        app.ui.status = format!("Exported {} PNG files", exported.len());
        app.ui.status_error = false;
        Ok(json!({"files": exported}))
    })
}

/// The first 80 characters of `name`, each control character and each character a file name
/// can't hold (`< > : " / \ | ? *`) turned into `_`.
fn file_name_chars(name: &str) -> String {
    name.chars().take(80).map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect()
}

fn export_name(name: &str) -> String {
    let name = file_name_chars(name);
    let name = name.trim().trim_matches('.');
    if name.is_empty() { "layer".into() } else { name.into() }
}

/// A single layer's export name: like [`export_name`], but boundary spaces and dots stay as the
/// name holds them; only a name that is empty or nothing but dots and whitespace falls back to
/// "layer".
fn layer_export_name(name: &str) -> String {
    let name = file_name_chars(name);
    if name.chars().all(|c| c.is_whitespace() || c == '.') { "layer".into() } else { name }
}

/// The files a multi-layer export writes: the chosen path, then numbered siblings named after
/// each layer, in the chosen path's format.
fn batch_paths(path: &str, docs: &[Document]) -> Vec<String> {
    let first = std::path::Path::new(path);
    let parent = first.parent().unwrap_or_else(|| std::path::Path::new(""));
    let stem = first.file_stem().and_then(|s| s.to_str()).unwrap_or("layers");
    let ext = first.extension().and_then(|e| e.to_str()).unwrap_or("png");
    docs.iter()
        .enumerate()
        .map(|(i, doc)| {
            if i == 0 {
                return path.to_string();
            }
            let name = export_name(&doc.name);
            let candidate = parent.join(format!("{stem}-{}-{name}.{ext}", i + 1));
            // The first file's overwrite is handled by the Save dialog; never overwrite sibling
            // files silently on desktop. Downloads on the web are named by the browser.
            #[cfg(not(target_arch = "wasm32"))]
            let candidate = {
                let mut candidate = candidate;
                let mut suffix = 1u64;
                // A name any entry occupies is taken, a symlink (even a dangling one) included: the writer
                // follows links, so one planted here would redirect the export.
                while candidate.symlink_metadata().is_ok() {
                    candidate = parent.join(format!("{stem}-{}-{name}-{suffix}.{ext}", i + 1));
                    suffix = suffix.saturating_add(1);
                }
                candidate
            };
            candidate.to_string_lossy().into_owned()
        })
        .collect()
}

/// The document a dialog exports: the whole image, or one layer (`__layer`).
fn source_document_from(doc: Arc<Document>, f: &Map<String, Value>) -> Result<Arc<Document>, String> {
    match f.get("__layer").and_then(Value::as_u64) {
        Some(id) => photocraft_engine::layer_menu_cmds::layer_document(&doc, photocraft_doc::LayerId(id)).map(Arc::new).map_err(|e| e.to_string()),
        None => Ok(doc),
    }
}

fn source_document(app: &PhotocraftApp, f: &Map<String, Value>) -> Result<Arc<Document>, String> {
    source_document_from(app.session.active().ok_or("no document")?.doc.clone(), f)
}

fn effective_source_profile(app: &PhotocraftApp, doc: &Document) -> Arc<Profile> {
    doc.icc_profile
        .as_ref()
        .and_then(|bytes| photocraft_engine::color_cmds::profile_from_bytes(bytes).ok())
        .filter(|profile| profile.color_space == photocraft_engine::color_cmds::mode_space(doc.mode))
        .unwrap_or_else(|| app.session.color.working(doc.mode))
}

fn s(f: &Map<String, Value>, k: &str) -> String {
    f.get(k).and_then(Value::as_str).unwrap_or_default().to_string()
}
fn n(f: &Map<String, Value>, k: &str, d: f64) -> f64 {
    f.get(k).and_then(Value::as_f64).unwrap_or(d)
}

/// Image Size's method for the exported copy (Export As › Resample). A missing choice is
/// Photoshop's default, Bicubic Automatic; an unknown one is an error, never a silent Bicubic.
/// The preview proxy (`max_side`) resamples bilinearly to stay fast, except for Nearest
/// Neighbor, whose hard pixel edges are the point of choosing it and cost nothing extra.
fn resample_method(f: &Map<String, Value>, max_side: Option<u32>) -> Result<&'static str, String> {
    let chosen = match f.get("resample") {
        None | Some(Value::Null) => DEFAULT_RESAMPLE,
        Some(v) => export_resample(v.as_str().ok_or("resample must be a string")?)?,
    };
    Ok(if max_side.is_some() && chosen != "nearest" { "bilinear" } else { chosen })
}

/// Whether the dialog asks for Nearest Neighbor (the preview then magnifies without smoothing).
fn nearest(f: &Map<String, Value>) -> bool {
    f.get("resample").and_then(Value::as_str) == Some("nearest")
}

/// The document as exported: scaled (engine Image Size, with the dialog's Resample method) and
/// flattened over white when transparency is off or the format has no alpha.
fn export_document(doc: &Document, f: &Map<String, Value>, max_side: Option<u32>, source_profile: &Profile) -> Result<Document, String> {
    let resample = resample_method(f, max_side)?;
    let srgb = s(f, "profile") == "srgb";
    let mut copy = doc.clone();
    if srgb {
        copy.icc_profile = Some(source_profile.to_bytes());
    }
    let mut session = photocraft_engine::Session::new();
    session.add_document(copy, None);
    if srgb && (doc.mode != ColorMode::Rgb || !source_profile.same_bytes(craft::cms::Builtin::Srgb.profile())) {
        session.execute("edit.convertToProfile", json!({"profile": "srgb", "intent": "perceptual"})).map_err(|e| e.to_string())?;
    }
    let (w, h) = export_pixels(f64::from(doc.size.width), f64::from(doc.size.height), f, max_side)?;
    if s_fmt(f) == "ico" && (w > 256.0 || h > 256.0) && max_side.is_none() {
        return Err("ICO export is limited to 256 × 256 pixels; reduce the export scale".into());
    }
    if (w - doc.size.width as f64).abs() >= 1.0 || (h - doc.size.height as f64).abs() >= 1.0 {
        session.execute("image.imageSize", json!({"width": w, "height": h, "resample": resample})).map_err(|e| e.to_string())?;
    }
    let fmt = s_fmt(f);
    if !f.get("transparency").and_then(Value::as_bool).unwrap_or(true) || fmt == "jpg" {
        session.execute("layer.flattenImage", json!({})).map_err(|e| e.to_string())?;
    }
    session.active().map(|d| (*d.doc).clone()).ok_or_else(|| "export failed".into())
}

/// The exported pixel size: each side scaled and rounded on its own, never below 1 px. The
/// dialog's size label and `export_document` share it so the label names the file's real size.
fn scaled_size(w: f64, h: f64, scale: f64) -> (f64, f64) {
    ((w * scale).round().max(1.0), (h * scale).round().max(1.0))
}

/// Largest side Export As will ask Image Size for. The same cap `image.imageSize` enforces.
pub(crate) const MAX_PX: f64 = 300_000.0;

fn finite_px(v: f64) -> Result<f64, String> {
    if !v.is_finite() || !(1.0..=MAX_PX).contains(&v) {
        return Err(format!("export size must be within 1..={MAX_PX} pixels"));
    }
    Ok(v.round())
}

/// The pixels `export_document` resizes to. Explicit width and height win; otherwise the percent
/// scale does, and a preview limit shrinks the long side before rounding (the same reduction the
/// scale path used before pixel fields existed).
fn export_pixels(src_w: f64, src_h: f64, f: &Map<String, Value>, max_side: Option<u32>) -> Result<(f64, f64), String> {
    // Exact pixels apply only in pixel mode. Scale mode keeps the percent, so each layer in a
    // multi-layer export is scaled from its own size rather than forced to the preview's pixels.
    let explicit = if s(f, "sizing") == "pixels" { f.get("width").and_then(Value::as_f64).zip(f.get("height").and_then(Value::as_f64)) } else { None };
    let (mut w, mut h) = if let Some((w, h)) = explicit {
        (finite_px(w)?, finite_px(h)?)
    } else {
        let mut scale = n(f, "scale", 100.0) / 100.0;
        if !scale.is_finite() || scale <= 0.0 {
            return Err("export scale must be a positive number".into());
        }
        if let Some(m) = max_side {
            let long = src_w.max(src_h) * scale;
            if long > f64::from(m) {
                scale *= f64::from(m) / long;
            }
        }
        return Ok(scaled_size(src_w, src_h, scale));
    };
    if let Some(m) = max_side {
        let long = w.max(h);
        if long > f64::from(m) && long > 0.0 {
            (w, h) = scaled_size(w, h, f64::from(m) / long);
        }
    }
    Ok((w, h))
}

fn s_fmt(f: &Map<String, Value>) -> String {
    let v = s(f, "format");
    if v.is_empty() { "png".into() } else { v }
}

fn lossless(f: &Map<String, Value>) -> bool {
    f.get("lossless").and_then(Value::as_bool).unwrap_or(false)
}

/// TGA bits per pixel: `"24"` or `"32"`; `"auto"` (the default) is 32 with an alpha channel or
/// transparency, else 24, as Export As wrote before it offered the choice.
fn tga_bits(f: &Map<String, Value>) -> Option<photocraft_io::TgaBits> {
    s(f, "tgaBits").parse().ok().and_then(photocraft_io::TgaBits::from_bits)
}

fn settings(f: &Map<String, Value>) -> ExportSettings {
    let fmt = s_fmt(f);
    let quality = n(f, "quality", 85.0).clamp(1.0, 100.0) as u8;
    let metadata = s(f, "metadata");
    ExportSettings {
        jpeg_quality: (fmt == "jpg").then_some(quality),
        // Photoshop's 1-7 Export As presets measured 4:2:0 at 1-4 and 4:4:4 at 5-7.
        // An exact mapping to 1-100 is unestablished; reuse PhotoCraft's Save for Web cutoff.
        jpeg_chroma_subsampling: fmt == "jpg" && quality <= 50,
        webp_lossless: fmt != "webp" || lossless(f),
        webp_quality: (fmt == "webp" && !lossless(f)).then_some(quality),
        xmp_all: metadata == "all",
        tga_bits: if fmt == "tga" { tga_bits(f) } else { None },
        embed_icc: f.get("embedProfile").and_then(Value::as_bool).unwrap_or(true),
        // Metadata: None drops EXIF and resolution as well as XMP. A missing key keeps them,
        // which is what Export As wrote before the choice existed.
        embed_metadata: metadata != "none",
        // Export As's JPGs are progressive (SOF2) in the reference application, measured at its
        // default quality; the scan layout is one setting for the whole dialog.
        jpeg_format: photocraft_codecs::JpegFormat::Progressive,
        ..Default::default()
    }
}

/// The preview and size estimate share a single reduced document. Resizing every layer twice is
/// especially expensive for layered PSDs, even when the canvas itself is small.
fn preview_proxy(doc: &Document, f: &Map<String, Value>, source_profile: &Profile) -> Result<Document, String> {
    export_document(doc, f, Some(if s_fmt(f) == "ico" { 256 } else { 512 }), source_profile)
}

fn scaled_estimate(bytes: usize, doc: &Document, proxy: &Document, f: &Map<String, Value>) -> u64 {
    let full = match (f.get("width").and_then(Value::as_f64), f.get("height").and_then(Value::as_f64)) {
        (Some(w), Some(h)) if w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0 => w * h,
        _ => {
            let scale = n(f, "scale", 100.0) / 100.0;
            doc.size.width as f64 * scale * doc.size.height as f64 * scale
        }
    };
    let small = (proxy.size.width as f64 * proxy.size.height as f64).max(1.0);
    (bytes as f64 * full / small) as u64
}

/// Estimated size (bytes) from the already prepared proxy encode, scaled by pixel count.
fn estimate(app: &PhotocraftApp, doc: &Document, proxy: &Document, f: &Map<String, Value>) -> Option<u64> {
    let export = app.services.export.as_ref()?;
    let (bytes, _) = export(proxy, &format!("estimate.{}", s_fmt(f)), &settings(f)).ok()?;
    Some(scaled_estimate(bytes.len(), doc, proxy, f))
}

struct PreviewPixels {
    estimate: Option<u64>,
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

fn preview_pixels(proxy: &Document, estimate: Option<u64>) -> PreviewPixels {
    let img = photocraft_compose::thumbnail(proxy, 360);
    PreviewPixels { estimate, width: img.width as usize, height: img.height as usize, rgba: img.pixels }
}

fn preview_sync(app: &PhotocraftApp, doc: &Document, f: &Map<String, Value>, source_profile: &Profile) -> Result<PreviewPixels, String> {
    let proxy = preview_proxy(doc, f, source_profile)?;
    Ok(preview_pixels(&proxy, estimate(app, doc, &proxy, f)))
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
struct PendingPreview {
    key: egui::Id,
    signature: String,
    result: Arc<Mutex<Option<Result<PreviewPixels, String>>>>,
}

#[cfg(not(target_arch = "wasm32"))]
fn background_preview_needed(doc: &Document) -> bool {
    doc.walk().len() > 32 || doc.size.area() > 16_000_000
}

/// Native app: keep the UI responsive while a many-layer document is reduced and encoded.
/// A single pending job per document also avoids starting a worker for every slider tick.
#[cfg(not(target_arch = "wasm32"))]
fn poll_or_start_preview(
    ctx: &egui::Context,
    key: egui::Id,
    signature: &str,
    doc: Arc<Document>,
    fields: &Map<String, Value>,
    source_profile: Arc<Profile>,
) -> Option<Result<PreviewPixels, String>> {
    let pending_key = egui::Id::unique(("export-preview-pending", doc.id.0));
    let pending: Option<PendingPreview> = ctx.data(|d| d.get_temp(pending_key));
    if let Some(pending) = pending {
        let result = pending.result.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()?;
        ctx.data_mut(|d| d.remove::<PendingPreview>(pending_key));
        if pending.key == key && pending.signature == signature {
            return Some(result);
        }
    }

    let result = Arc::new(Mutex::new(None));
    let shared = result.clone();
    let repaint = ctx.clone();
    let fields = fields.clone();
    let started = std::thread::Builder::new().name("export-preview".into()).spawn(move || {
        let prepared = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let source = source_document_from(doc.clone(), &fields).unwrap_or(doc);
            let proxy = preview_proxy(&source, &fields, &source_profile)?;
            let estimate = estimate_native(&source, &proxy, &fields);
            Ok::<_, String>(preview_pixels(&proxy, estimate))
        }))
        .unwrap_or_else(|_| Err("preview worker panicked".into()));
        *shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(prepared);
        repaint.request_repaint();
    });
    match started {
        Ok(_) => {
            ctx.data_mut(|d| d.insert_temp(pending_key, PendingPreview { key, signature: signature.to_string(), result }));
            None
        }
        Err(error) => Some(Err(format!("could not start preview worker: {error}"))),
    }
}

/// The native exporter uses this same IO encoder. Keep the entire estimate off the UI thread.
#[cfg(not(target_arch = "wasm32"))]
fn estimate_native(doc: &Document, proxy: &Document, f: &Map<String, Value>) -> Option<u64> {
    let bytes = photocraft_io::export(proxy, &format!("estimate.{}", s_fmt(f)), &settings(f).io_options()).ok()?.bytes;
    Some(scaled_estimate(bytes.len(), doc, proxy, f))
}

/// Exports up to this many pixels get an exact size: the full image encoded as Export will
/// write it, as Photoshop's Export As reports (#6595). Larger ones keep the proxy estimate, which
/// reads low for detailed images (a downscaled proxy compresses better).
#[cfg(not(target_arch = "wasm32"))]
const EXACT_ESTIMATE_PX: f64 = 40_000_000.0;

/// The exported file's size in bytes, from encoding the full export, or `None` when it is too
/// large to encode for an estimate or can't be encoded.
#[cfg(not(target_arch = "wasm32"))]
fn exact_estimate(doc: &Document, f: &Map<String, Value>, source_profile: &Profile) -> Option<u64> {
    // The size `export_document` will make: exact pixels only in pixel mode, else the scale.
    let (w, h) = export_pixels(f64::from(doc.size.width), f64::from(doc.size.height), f, None).ok()?;
    let px = w * h;
    if !px.is_finite() || px > EXACT_ESTIMATE_PX {
        return None;
    }
    let full = export_document(doc, f, None, source_profile).ok()?;
    let bytes = photocraft_io::export(&full, &format!("estimate.{}", s_fmt(f)), &settings(f).io_options()).ok()?.bytes;
    u64::try_from(bytes.len()).ok()
}

/// The exact size for `signature`, worked out off the UI thread: `None` until it is ready (the
/// proxy estimate shows meanwhile), then `Some(None)` when there is no exact size.
#[cfg(not(target_arch = "wasm32"))]
fn poll_or_start_exact_estimate(
    ctx: &egui::Context,
    key: egui::Id,
    signature: &str,
    doc: Arc<Document>,
    fields: &Map<String, Value>,
    source_profile: Arc<Profile>,
) -> Option<Option<u64>> {
    type Done = Arc<Mutex<Option<Option<u64>>>>;
    let done_key = key.with("exact-estimate");
    if let Some((sig, size)) = ctx.data(|d| d.get_temp::<(String, Option<u64>)>(done_key))
        && sig == signature
    {
        return Some(size);
    }
    let pending_key = key.with("exact-estimate-pending");
    if let Some((sig, result)) = ctx.data(|d| d.get_temp::<(String, Done)>(pending_key)) {
        if sig == signature {
            let size = result.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()?;
            ctx.data_mut(|d| {
                d.remove::<(String, Done)>(pending_key);
                d.insert_temp(done_key, (sig, size));
            });
            return Some(size);
        }
        // Settings changed meanwhile: that worker's answer is for other settings.
        if result.lock().unwrap_or_else(std::sync::PoisonError::into_inner).is_none() {
            return None;
        }
    }
    let result: Done = Arc::new(Mutex::new(None));
    let shared = result.clone();
    let repaint = ctx.clone();
    let fields = fields.clone();
    let started = std::thread::Builder::new().name("export-estimate".into()).spawn(move || {
        let size = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let source = source_document_from(doc.clone(), &fields).unwrap_or(doc);
            exact_estimate(&source, &fields, &source_profile)
        }))
        .unwrap_or(None);
        *shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(size);
        repaint.request_repaint();
    });
    if started.is_ok() {
        ctx.data_mut(|d| d.insert_temp(pending_key, (signature.to_string(), result)));
        None
    } else {
        Some(None)
    }
}

/// The dialog's driver, a `dialogs::TAKERS` row: answers the preview and save requests the dialog
/// filed, stores its preset edits, and exports when it finishes with a path. Each request is taken
/// once, so a second pass in a frame answers none of them again.
pub(crate) fn take(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(mut open) = app.export_open.take() else { return };
    if drive(app, ctx, &mut open) {
        app.export_open = Some(open);
    }
}

/// Runs one step of [`take`] on `open`; `false` once the dialog is closed.
fn drive(app: &mut PhotocraftApp, ctx: &egui::Context, open: &mut ExportOpen) -> bool {
    match app.framework.dialogs.take::<HostedExportAs>() {
        Some(DialogResult::Ok(result)) => {
            if let Err(error) = finish(app, ctx, open, result) {
                app.ui.status = error;
                app.ui.status_error = true;
            }
            return false;
        }
        Some(_) => return false,
        None => {}
    }
    let Some(dialog) = app.framework.dialogs.get_mut::<HostedExportAs>() else { return false };
    let wanted = dialog.wanted_preview();
    let events = dialog.take_preset_events();
    let pick = dialog.pick_path();
    // A created preset's value is in the library under its name.
    let saved: Vec<(String, ExportOptions)> = events
        .iter()
        .filter_map(|event| match event {
            NamedPresetEvent::Create { name, .. } => {
                dialog.saved_presets().items().into_iter().find(|item| &item.name == name).map(|item| (name.clone(), item.value.clone()))
            }
            _ => None,
        })
        .collect();
    for event in events {
        store_preset(app, open.source, event, &saved);
    }
    if let Some(request) = pick {
        let key = request.key.clone();
        let suggested = request.file_name.clone().unwrap_or_default();
        let answer = move |app: &mut PhotocraftApp, path: String| -> Result<Value, String> {
            if let Some(open) = app.export_open.as_mut() {
                open.picking = false;
            }
            if let Some(dialog) = app.framework.dialogs.get_mut::<HostedExportAs>() {
                dialog.answer_path(key, PathBuf::from(path));
            }
            Ok(json!({}))
        };
        // The dialog's request says whether the answer is a folder (several Scale All sizes, several layers' files on
        // the desktop) or a file named after the document or its one layer.
        // PLAN: `pick_export` remembers the chosen folder when the answer is taken, before the export it leads to has run, so a failed export still moves where the next one starts.
        let asked = if matches!(request.kind, PathKind::Folder) { app.ask_folder(answer) } else { app.pick_export(&suggested, answer) };
        match asked {
            Ok(_) => open.picking = true,
            Err(error) => {
                if let Some(dialog) = app.framework.dialogs.get_mut::<HostedExportAs>() {
                    dialog.cancel_requests();
                }
                if error != crate::file_dialog::CANCELLED {
                    app.ui.status = error;
                    app.ui.status_error = true;
                }
            }
        }
    } else if open.picking && !app.file_dialog_open() {
        // The save dialog ended without an answer: the dialog's Export works again.
        open.picking = false;
        if let Some(dialog) = app.framework.dialogs.get_mut::<HostedExportAs>() {
            dialog.cancel_requests();
        }
    }
    // The settings did not change but the working profile or the scratch read-error generation did: ask again for the
    // same revision.
    let wanted = wanted.or_else(|| {
        let (revision, options, hash, scratch) = open.last.as_ref()?;
        (working_profile_hash(app) != *hash || crate::scratch_reads::generation() != *scratch).then(|| PreviewRequest::new(*revision, options.clone()))
    });
    if let Some(request) = wanted {
        open.last = Some((request.revision, request.options.clone(), working_profile_hash(app), crate::scratch_reads::generation()));
        open.work = start_preview(app, open, request);
    }
    answer_preview(app, ctx, open);
    true
}

/// The documents a Layer › Export As of several `layers` writes, one per layer with pixels (#14806); empty for one
/// layer or the whole document.
fn layer_documents(app: &PhotocraftApp, layers: &[LayerId]) -> Vec<Document> {
    if layers.len() < 2 {
        return Vec::new();
    }
    app.session.active().and_then(|st| exportable_layers(&st.doc, layers).ok()).unwrap_or_default()
}

/// The working profile an untagged active document is previewed under; `None` for a tagged one,
/// whose own profile decides. Tagged documents skip the profile lookup on every frame.
fn working_profile_hash(app: &PhotocraftApp) -> Option<String> {
    let doc = &app.session.active()?.doc;
    doc.icc_profile.is_none().then(|| app.session.color.working(doc.mode).content_hash().to_string())
}

/// One preset edit of the dialog, stored through the engine's commands. `created` holds the values
/// of the presets the dialog made this pass.
fn store_preset(app: &mut PhotocraftApp, source: PixelSize, event: NamedPresetEvent<String>, created: &[(String, ExportOptions)]) {
    let (id, params) = match event {
        NamedPresetEvent::Create { name, .. } => {
            let Some((_, options)) = created.iter().find(|(made, _)| *made == name) else { return };
            ("file.export.presets.save", json!({"name": name, "settings": settings_from(options, source)}))
        }
        NamedPresetEvent::Delete(name) => ("file.export.presets.delete", json!({"name": name})),
        NamedPresetEvent::Rename { key, text, .. } => ("file.export.presets.rename", json!({"name": key, "newName": text})),
        _ => return,
    };
    if let Err(error) = app.run(id, params) {
        app.ui.status = error;
        app.ui.status_error = true;
    }
}

/// The work for a new preview request: the document and profile it is made from, captured now.
fn start_preview(app: &PhotocraftApp, open: &ExportOpen, request: PreviewRequest<ExportOptions>) -> Option<PreviewWork> {
    let st = app.session.active()?;
    let doc = st.doc.clone();
    let mut settings = settings_from(&request.options, open.source);
    // The preview of several layers shows the first.
    if let Some(layer) = open.layers.first() {
        settings.insert("__layer".into(), json!(layer.0));
    }
    let profile = effective_source_profile(app, &doc);
    let signature = format!("{}:{}", Value::Object(settings.clone()), profile.content_hash());
    let layer = open.layers.first().map(|layer| layer.0);
    let key = egui::Id::unique(("export-preview", doc.id.0, st.revision, layer, crate::scratch_reads::generation()));
    Some(PreviewWork { revision: request.revision, settings, signature, key, doc, profile, answered: None })
}

/// Gives the dialog the preview of the work in progress when it is ready, then the exact size
/// estimate when that is. The work ends when the last of them is given.
fn answer_preview(app: &mut PhotocraftApp, ctx: &egui::Context, open: &mut ExportOpen) {
    let Some(mut work) = open.work.take() else { return };
    let mut give = None;
    if work.answered.is_none() {
        #[cfg(not(target_arch = "wasm32"))]
        let result = if background_preview_needed(&work.doc) {
            poll_or_start_preview(ctx, work.key, &work.signature, work.doc.clone(), &work.settings, work.profile.clone())
        } else {
            Some(sync_preview(app, &work))
        };
        #[cfg(target_arch = "wasm32")]
        let result = Some(sync_preview(app, &work));
        if let Some(result) = result {
            let preview = preview_of(ctx, open, &work, result);
            work.answered = Some(preview.clone());
            give = Some(preview);
        }
    }
    let mut done = false;
    if let Some(answered) = &work.answered {
        #[cfg(not(target_arch = "wasm32"))]
        match poll_or_start_exact_estimate(ctx, work.key, &work.signature, work.doc.clone(), &work.settings, work.profile.clone()) {
            Some(Some(bytes)) => {
                give = Some(answered.clone().estimate(SizeEstimate::Exact(bytes)));
                done = true;
            }
            Some(None) => done = true,
            None => {}
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = answered;
            done = true;
        }
    }
    if let (Some(preview), Some(dialog)) = (give, app.framework.dialogs.get_mut::<HostedExportAs>()) {
        dialog.set_preview(preview);
    }
    if !done {
        open.work = Some(work);
    }
}

/// The preview of `work` made on this thread, for a document small enough not to stall the frame.
fn sync_preview(app: &PhotocraftApp, work: &PreviewWork) -> Result<PreviewPixels, String> {
    let source = source_document_from(work.doc.clone(), &work.settings).unwrap_or_else(|_| work.doc.clone());
    preview_sync(app, &source, &work.settings, &work.profile)
}

/// The dialog's answer for `work`: the picture as a texture the dialog's state names (kept in
/// `open`), with the proxy's size estimate, or the failure. A small export is magnified to fill the
/// preview, and with Nearest Neighbor that keeps hard pixel edges too.
fn preview_of(ctx: &egui::Context, open: &mut ExportOpen, work: &PreviewWork, result: Result<PreviewPixels, String>) -> ExportPreview {
    match result {
        Ok(pixels) => {
            let filter = if nearest(&work.settings) { egui::TextureOptions::NEAREST } else { egui::TextureOptions::LINEAR };
            let image = egui::ColorImage::from_rgba_unmultiplied([pixels.width, pixels.height], &pixels.rgba);
            let texture = ctx.load_texture("export-preview", image, filter);
            let state = PreviewState::Ready(egui::load::SizedTexture::from_handle(&texture));
            open.texture = Some(texture);
            let preview = ExportPreview::new(work.revision, state);
            match pixels.estimate {
                Some(bytes) => preview.estimate(SizeEstimate::Proxy(bytes)),
                None => preview,
            }
        }
        Err(error) => {
            log::warn!("Export As preview failed: {error}");
            ExportPreview::new(work.revision, PreviewState::Failed(error.into()))
        }
    }
}

/// What the dialog's Export does once its path is known: writes the document at each Scale All
/// size, or one file per layer, then remembers the settings for the next open. A failed export
/// leaves them as they were.
fn finish(app: &mut PhotocraftApp, ctx: &egui::Context, open: &ExportOpen, result: ExportAsResult) -> Result<Value, String> {
    let path = result.path.to_string_lossy().into_owned();
    let mut f = settings_from(&result.options, open.source);
    let rows = result.options.scale_rows();
    let exported = if open.layers.len() > 1 {
        export_layers(app, &f, &open.layers, &path)?
    } else {
        if let Some(layer) = open.layers.first() {
            f.insert("__layer".into(), json!(layer.0));
        }
        // With more than one output the path is the folder the files go into.
        if result.outputs.len() > 1 { export_sizes(app, &f, &rows, &path)? } else { export_to(app, &scaled_fields(&f, rows[0].size.factor()), &path)? }
    };
    result.remember(ctx);
    Ok(exported)
}

/// Exports the dialog's source (the document, or the layer `__layer`) to `path` with the settings
/// `f`. A whole-document export is kept by an action being recorded, with its destination and
/// settings (#7207); a layer's isn't, since its layer is this document's.
fn export_to(app: &mut PhotocraftApp, f: &Map<String, Value>, path: &str) -> Result<Value, String> {
    let doc = source_document(app, f)?;
    let source_profile = effective_source_profile(app, &doc);
    let r = write_export(app, &doc, f, path, &source_profile)?;
    if !f.contains_key("__layer") {
        let mut step = settings_of(f);
        step.insert("path".into(), json!(path));
        let step = Value::Object(step);
        photocraft_engine::actions_cmds::note_automation_step(&mut app.session, COMMAND, &step);
        app.session.journal.push((COMMAND.into(), step));
    }
    Ok(r)
}

/// Asks where to save, then [`export_to`]s: Quick Export as PNG's way of exporting without the dialog.
/// A layer's name may hold characters a file name can't (#14810): `a/b:c*?` suggests `a_b_c__`.
fn pick_and_export(app: &mut PhotocraftApp, f: Map<String, Value>) -> Result<Value, String> {
    let doc = source_document(app, &f)?;
    let stem = if f.contains_key("__layer") { layer_export_name(&stem_of(&doc.name)) } else { stem_of(&doc.name) };
    let suggested = format!("{stem}.{}", s_fmt(&f));
    app.pick_export(&suggested, move |app, path| export_to(app, &f, &path))
}

/// The dialog settings `f` with the export size multiplied by `factor` (a Scale All row).
fn scaled_fields(f: &Map<String, Value>, factor: f64) -> Map<String, Value> {
    let mut out = f.clone();
    if (factor - 1.0).abs() > f64::EPSILON {
        out.insert("scale".into(), json!(n(f, "scale", 100.0) * factor));
        for key in ["width", "height"] {
            if let Some(v) = f.get(key).and_then(Value::as_f64) {
                out.insert(key.into(), json!((v * factor).round().max(1.0)));
            }
        }
    }
    out
}

/// A row's suffix as part of a file name: characters a file name can't hold become `_`.
fn file_suffix(suffix: &str) -> String {
    suffix.chars().map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect()
}

/// The file names a batch writes, `<name><suffix>.<ext>` per row, or an error naming the first one two
/// rows share (suffixes that sanitize alike, or differ only in case, count: a file system may fold them, by
/// upper-casing as Windows does, so `σ` and `ς` are one). Nothing is written then.
fn batch_file_names(stem: &str, ext: &str, rows: &[ScaleRow]) -> Result<Vec<String>, String> {
    let names: Vec<String> = rows.iter().map(|row| format!("{stem}{}.{ext}", file_suffix(&row.suffix))).collect();
    for (i, name) in names.iter().enumerate() {
        if names[..i].iter().any(|other| other.to_uppercase() == name.to_uppercase() || other.to_lowercase() == name.to_lowercase()) {
            let template = "Two sizes would write the same file, {name}; give each size its own suffix";
            return Err(crate::i18n::fmt(tl!(template), &[("name", name)]));
        }
    }
    Ok(names)
}

/// Scale All with several rows (#15028): `<name><suffix>.<ext>` in the folder `dir` for each row, at the
/// row's multiple of the export size. Only a folder was chosen, so nothing already there is replaced:
/// any entry at a target path (a file, or a link the writer would follow) stops the whole batch before
/// the first write.
fn export_sizes(app: &mut PhotocraftApp, f: &Map<String, Value>, rows: &[ScaleRow], dir: &str) -> Result<Value, String> {
    let doc = source_document(app, f)?;
    let source_profile = effective_source_profile(app, &doc);
    let stem = stem_of(&doc.name);
    let stem = export_name(&if f.contains_key("__layer") { layer_export_name(&stem) } else { stem });
    let names = batch_file_names(&stem, &s_fmt(f), rows)?;
    #[cfg(not(target_arch = "wasm32"))]
    for name in &names {
        let path = std::path::Path::new(dir).join(name);
        if let Ok(meta) = path.symlink_metadata() {
            let message = if meta.file_type().is_symlink() {
                let template = "{path} is a link; nothing was written. Choose another folder or change the suffix";
                tl!(template)
            } else {
                let template = "{path} already exists; nothing was written. Choose another folder or change the suffix";
                tl!(template)
            };
            return Err(crate::i18n::fmt(message, &[("path", &path.display().to_string())]));
        }
    }
    let mut exported = Vec::new();
    for (row, name) in rows.iter().zip(&names) {
        let path = std::path::Path::new(dir).join(name).to_string_lossy().into_owned();
        exported.push(write_export(app, &doc, &scaled_fields(f, row.size.factor()), &path, &source_profile)?);
    }
    let template = "Exported {n} files";
    app.ui.status = crate::i18n::fmt(tl!(template), &[("n", &exported.len().to_string())]);
    app.ui.status_error = false;
    Ok(json!({"exported": exported}))
}

/// Exports `doc` to `path` with the dialog settings `f` (Export As, and its replayed action step).
fn write_export(app: &mut PhotocraftApp, doc: &Document, f: &Map<String, Value>, path: &str, source_profile: &Profile) -> Result<Value, String> {
    let out = export_document(doc, f, None, source_profile)?;
    crate::app::ensure_memory_to_save(&out, path)?;
    let export = app.services.export.as_ref().ok_or("no exporter configured")?;
    let (bytes, warnings) = export(&out, path, &settings(f))?;
    let write = app.services.write.as_mut().ok_or("no writer configured")?;
    write(path, &bytes)?;
    let template = "Exported {path} ({size})";
    app.ui.status = crate::i18n::fmt(tl!(template), &[("path", path), ("size", &crate::sizing::human_bytes(bytes.len() as f64))]);
    app.ui.status_error = false;
    let template = "Exported {path}";
    let what = crate::i18n::fmt(tl!(template), &[("path", &crate::file_open::display_name(path))]);
    crate::notices::io_warnings(app, &what, &warnings);
    Ok(json!({"path": path, "bytes": bytes.len(), "warnings": warnings}))
}

/// Replays a recorded Export As step (#7207): the active document exported to the recorded `path`
/// with the recorded settings, without the dialog. The remembered dialog settings are untouched.
pub(crate) fn replay(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    let path = params.get("path").and_then(Value::as_str).filter(|p| !p.is_empty()).ok_or("Export As needs a `path`")?;
    let f = params.as_object().map(settings_of).unwrap_or_default();
    if let Some(format) = f.get("format").filter(|v| !valid_setting("format", v)) {
        return Err(format!("Export As can't write the format {format}"));
    }
    // A setting out of the dialog's range or of the wrong type is an error, not quietly its
    // default: the step would otherwise write something other than what was recorded.
    if let Some((key, v)) = f.iter().find(|(k, v)| !valid_setting(k, v)) {
        return Err(format!("Export As: `{key}` can't be {v}"));
    }
    let doc = app.session.active().ok_or("no document")?.doc.clone();
    let source_profile = effective_source_profile(app, &doc);
    write_export(app, &doc, &f, path, &source_profile)
}

/// Several layers, each exported with the dialog's settings `f`. Empty, adjustment and off-canvas
/// layers are skipped (#14806). With several files on the desktop `path` is the folder the dialog
/// asked for, and each layer is written there as `<layer name>.<ext>`, replacing a file of that
/// name (#14808). One file is written at `path`; the web downloads each file, the first under
/// `path` and the others named as Quick Export names a selection's files.
fn export_layers(app: &mut PhotocraftApp, f: &Map<String, Value>, ids: &[LayerId], path: &str) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document")?;
    let source_profile = effective_source_profile(app, &st.doc);
    if ids.is_empty() {
        return Err("no selected layers".into());
    }
    let docs = exportable_layers(&st.doc, ids)?;
    let settings = settings(f);
    let folder = docs.len() > 1 && !cfg!(target_arch = "wasm32");
    let paths: Vec<String> = if folder {
        layer_file_names(&docs, &s_fmt(f)).into_iter().map(|name| std::path::Path::new(path).join(name).to_string_lossy().into_owned()).collect()
    } else {
        batch_paths(path, &docs)
    };
    let mut exported = Vec::new();
    for (doc, path) in docs.iter().zip(paths) {
        // Only a folder was chosen, so a link where a file goes is removed rather than written through.
        if folder {
            replace_link(std::path::Path::new(&path))?;
        }
        exported.push(write_layer_file(app, doc, f, &settings, &source_profile, &path)?);
    }
    let template = "Exported {n} files";
    app.ui.status = crate::i18n::fmt(tl!(template), &[("n", &exported.len().to_string())]);
    app.ui.status_error = false;
    Ok(json!({"files": exported}))
}

/// One layer's file of a Layer › Export As: rendered with the dialog's settings and written.
fn write_layer_file(
    app: &mut PhotocraftApp,
    doc: &Document,
    f: &Map<String, Value>,
    settings: &ExportSettings,
    source_profile: &Profile,
    path: &str,
) -> Result<Value, String> {
    let out = export_document(doc, f, None, source_profile)?;
    crate::app::ensure_memory_to_save(&out, path)?;
    let export = app.services.export.as_ref().ok_or("no exporter configured")?;
    let (bytes, warnings) = export(&out, path, settings)?;
    let write = app.services.write.as_mut().ok_or("no writer configured")?;
    write(path, &bytes)?;
    let template = "Exported {path}";
    let what = crate::i18n::fmt(tl!(template), &[("path", &crate::file_open::display_name(path))]);
    crate::notices::io_warnings(app, &what, &warnings);
    Ok(json!({"path": path, "bytes": bytes.len(), "warnings": warnings}))
}

/// A layer's file name, as measured in the reference application: each character a file name
/// can't hold becomes `_` (`a/b:c*?` → `a_b_c__`), spaces are kept.
fn layer_file_stem(name: &str) -> String {
    let stem: String = name.chars().map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect();
    if stem.chars().all(|c| c.is_whitespace() || c == '.') { "layer".into() } else { stem }
}

/// The names several layers' files get, for `docs` in document order (bottom to top). Equal
/// names are numbered in Layers panel order: the upper layer keeps the name, the next ones down
/// get `-1`, `-2`, … (skipping a name another layer already has). Names are compared upper-cased and lower-cased
/// (so a final sigma `ς` and `σ`, and `K` and the Kelvin sign, are one name),
/// as a case-insensitive destination (a default Windows or macOS volume) treats `Logo` and `logo`
/// as one file, which a second write would silently replace.
fn layer_file_names(docs: &[Document], ext: &str) -> Vec<String> {
    // A name is taken when its upper-cased or its lower-cased form is: upper-casing joins `σ` and `ς`
    // (Windows folds that way), lower-casing joins `K` and the Kelvin sign `K` (which APFS treats as
    // one name). Neither folding alone catches both.
    let taken = |used: &std::collections::HashSet<String>, name: &str| used.contains(&name.to_uppercase()) || used.contains(&name.to_lowercase());
    let mut used = std::collections::HashSet::new();
    let mut names = vec![String::new(); docs.len()];
    for (i, doc) in docs.iter().enumerate().rev() {
        let stem = layer_file_stem(&doc.name);
        let mut name = format!("{stem}.{ext}");
        let mut n = 1u64;
        while taken(&used, &name) {
            name = format!("{stem}-{n}.{ext}");
            n = n.saturating_add(1);
        }
        used.insert(name.to_uppercase());
        used.insert(name.to_lowercase());
        names[i] = name;
    }
    names
}

/// Existing files are replaced without a prompt, but a link where a file goes is removed first,
/// so the write lands there and is never redirected (the writer follows links).
fn replace_link(path: &std::path::Path) -> Result<(), String> {
    #[cfg(not(target_arch = "wasm32"))]
    if path.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink()) {
        std::fs::remove_file(path).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    #[cfg(target_arch = "wasm32")]
    let _ = path;
    Ok(())
}

/// File › Export › Quick Export as PNG: the format, quality, metadata, colour space and location
/// from File › Export › Export Preferences (engine `file.export.quickExport`). On the web (no
/// file system) it falls back to a PNG download through the export service.
pub fn quick_export_png(app: &mut PhotocraftApp) -> Result<Value, String> {
    if !cfg!(target_arch = "wasm32") {
        let prefs = app.session.prefs().export.clone();
        let fmt = serde_json::to_value(prefs.quick_export_format).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| "png".into());
        let same = serde_json::to_value(prefs.quick_export_location).ok().is_some_and(|v| v == "sameFolder");
        let saved = app.session.active().is_some_and(|d| d.path.is_some());
        // Same Folder writes beside the document's file without asking, except when that would
        // replace the file itself (a document opened from photo.png): then the save dialog asks,
        // so replacing it is the user's choice (#6599).
        if same && saved && !photocraft_engine::web_cmds::quick_export_target_is_own_file(&app.session) {
            return quick_export(app, json!({}));
        }
        let st = app.session.active().ok_or("no document")?;
        let stem = st.doc.name.rsplit_once('.').map_or(st.doc.name.as_str(), |(a, _)| a);
        let (suggested, doc) = (format!("{stem}.{fmt}"), st.doc.id);
        return app.pick_export(&suggested, move |app, path| {
            app.refocus(doc)?;
            quick_export(app, json!({"path": path}))
        });
    }
    let mut f = Map::new();
    f.insert("format".into(), json!("png"));
    f.insert("transparency".into(), json!(true));
    f.insert("scale".into(), json!(100));
    pick_and_export(app, f)
}

/// Runs Quick Export with `p` (an explicit `path`, or none for the document's folder).
fn quick_export(app: &mut PhotocraftApp, p: Value) -> Result<Value, String> {
    let r = app.run("file.export.quickExport", p)?;
    let template = "Exported {path}";
    app.ui.status = crate::i18n::fmt(tl!(template), &[("path", r["path"].as_str().unwrap_or_default())]);
    Ok(r)
}

#[cfg(test)]
mod preserve_details_tests;
#[cfg(test)]
mod srgb_default_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod jpg_progressive_tests;
#[cfg(test)]
mod layer_name_sanitise_tests;

#[cfg(all(test, unix))]
mod batch_name_tests {
    use super::*;
    use photocraft_doc::{ColorMode, SampleType, Size};

    #[test]
    fn a_planted_symlink_makes_a_batch_name_taken() {
        let dir = std::env::temp_dir().join(format!("pc-batch-names-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Dangling: `exists()` follows it and reports the name free; the writer would follow it too.
        let planted = dir.join("x-2-a.png");
        let _ = std::fs::remove_file(&planted);
        std::os::unix::fs::symlink(dir.join("victim"), &planted).unwrap();
        let doc = Document::new("a", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
        let paths = batch_paths(dir.join("x.png").to_str().unwrap(), &[doc.clone(), doc]);
        assert_ne!(paths[1], planted.to_str().unwrap());
        std::fs::remove_file(&planted).unwrap();
    }
}

#[cfg(test)]
mod scale_all_tests;
