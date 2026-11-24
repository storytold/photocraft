use super::dispatch::no_params;
use super::{PhotocraftApp, Value, json};
use crate::dialogs::image_processor_ui;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Outcome as _, Props as _, Show as _};
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::Label;
use craft::ui::runtime::requests::PathKind;

#[path = "artboard_export_options.rs"]
mod artboard_export_options;
pub(crate) use artboard_export_options::nest as nest_artboard_export_options;
#[path = "artboard_name_options.rs"]
mod artboard_name_options;
pub(crate) use artboard_name_options::strip as strip_artboard_name_options;

// ---------- dialogs and pickers in front of engine commands ----------

/// The command ids whose dialog is a hosted parameter form built by [`params_for`] (Open EXR Parts included).
/// The dialogs area's `PARAM_FAMILIES` row owns exactly these.
const FORM_COMMANDS: [&str; 21] = [
    "file.openExrParts",
    "layer.videoLayers.interpretFootage",
    "file.fileInfo",
    "file.automate.fitImage",
    "file.automate.conditionalModeChange",
    "file.export.layersToFiles",
    "file.export.layerCompsToFiles",
    "file.export.artboardsToFiles",
    "file.export.artboardsToPdf",
    "file.export.dataSetsAsFiles",
    "layer.new.artboard",
    "file.export.colorLookupTables",
    "file.automate.photomerge",
    "file.automate.mergeToHdrPro",
    "file.automate.lensCorrection",
    "file.scripts.imageProcessor",
    "file.automate.batch",
    "select.saveSelection",
    "select.loadSelection",
    "image.applyImage",
    "image.calculations",
];

/// Whether `key` is the command id of a form this file opens.
pub(crate) fn is_form(key: &str) -> bool {
    FORM_COMMANDS.contains(&key)
}

/// The reply of a command that opened a dialog.
fn opened(id: DialogId) -> Value {
    json!({"dialog": id.value().to_string()})
}

/// Opens the hosted parameter form of command `id`. `fixed` holds values the command needs that no row edits.
fn open_form(app: &mut PhotocraftApp, id: &str, title: &str, params: Vec<Param>, fields: DialogFields, fixed: Value) -> DialogId {
    let title = crate::dialogs::filter_dialog::title_of(title);
    let key = id.to_owned();
    let image_processor = id == image_processor_ui::COMMAND;
    if image_processor {
        image_processor_ui::settings_opened(app.ctx());
    }
    crate::dialogs::params::open_param(app, id, fields, fixed, move |fields: &mut DialogFields| {
        derive(&key, fields);
        let dialog = ParamDialog::command(key.clone(), title.clone(), params.clone()).values(Bind::bound(fields));
        if image_processor { image_processor_ui::settings_footer(dialog) } else { dialog }
    })
}

/// Brings a form's shell keys up to date with the values they follow, before each pass, OK and automation write (the
/// form's builder runs at each): a row takes one `visible_when` condition, so a row shown by two values is shown by a
/// shell key that combines them.
fn derive(id: &str, fields: &mut DialogFields) {
    if id == artboard_export_options::COMMAND {
        artboard_export_options::derive(fields);
    }
    if id == artboard_name_options::COMMAND {
        artboard_name_options::derive(fields);
    }
}

/// Choice options from `(id, label)` pairs whose labels are `tl!` literals at the call site.
fn opts(list: &[(&'static str, &'static str)]) -> Vec<(String, Label)> {
    list.iter().map(|(id, label)| ((*id).to_owned(), Label::from(*label))).collect()
}

/// Options named by the user's own text (action and set names): shown as they are.
fn named(list: &[String]) -> Vec<(String, Label)> {
    list.iter().map(|n| (n.clone(), Label::from(n.clone()))).collect()
}

/// Channel and layer options: the engine's fixed ids read as English words, every other name is data.
fn channels(list: &[String]) -> Vec<(String, Label)> {
    let special = |id: &str| -> Option<Label> {
        Some(Label::from(match id {
            "new" => tl!("New"),
            "none" => tl!("None"),
            "merged" => tl!("Merged"),
            "transparency" => tl!("Transparency"),
            "mask" => tl!("Mask"),
            "selection" => tl!("Selection"),
            _ => return None,
        }))
    };
    list.iter().map(|n| (n.clone(), special(n).unwrap_or_else(|| Label::from(n.clone())))).collect()
}

/// A choice row with `default` selected.
fn choice(key: &str, label: &'static str, options: Vec<(String, Label)>, default: &str) -> Param {
    Param::new(key, label, ParamKind::choice(options, Some(default.to_owned())))
}

/// The file formats the exports to files write.
fn export_formats() -> Vec<(String, Label)> {
    opts(&[("png", tl!("PNG")), ("jpg", tl!("JPEG")), ("psd", tl!("PSD")), ("tiff", tl!("TIFF")), ("webp", tl!("WebP")), ("bmp", tl!("BMP"))])
}

/// The reference application's File Type list of Artboards to Files, in its order (#13635). Format names, not
/// translated.
fn artboard_formats() -> Vec<(String, Label)> {
    opts(&[("bmp", "BMP"), ("jpg", "JPEG"), ("pdf", "PDF"), ("psd", "PSD"), ("tga", "Targa"), ("tiff", "TIFF"), ("png8", "PNG-8"), ("png24", "PNG-24")])
}

/// The blending modes of the channel dialogs.
fn blend_modes() -> Vec<(String, Label)> {
    opts(&[
        ("multiply", tl!("Multiply")),
        ("screen", tl!("Screen")),
        ("normal", tl!("Normal")),
        ("overlay", tl!("Overlay")),
        ("softLight", tl!("Soft Light")),
        ("hardLight", tl!("Hard Light")),
        ("darken", tl!("Darken")),
        ("lighten", tl!("Lighten")),
        ("colorDodge", tl!("Color Dodge")),
        ("colorBurn", tl!("Color Burn")),
        ("linearBurn", tl!("Linear Burn")),
        ("linearDodge", tl!("Linear Dodge (Add)")),
        ("difference", tl!("Difference")),
        ("exclusion", tl!("Exclusion")),
        ("add", tl!("Add")),
        ("subtract", tl!("Subtract")),
    ])
}

/// A choice id read as words ("colorDodge" is "Color dodge"), translated when the catalog has the sentence.
pub(crate) fn label_of(key: &str) -> String {
    // A name that already reads as text ("iPhone 14 Pro Max", "Web 1920") is not camelCase.
    if key.contains(' ') {
        return tl!(key).to_owned();
    }
    let mut out = String::new();
    for (i, c) in key.chars().enumerate() {
        if i == 0 {
            out.extend(c.to_uppercase());
        } else if c.is_uppercase() {
            out.push(' ');
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    tl!(&out).to_owned()
}

/// A folder field with its Browse button.
fn folder(key: &str, label: &'static str, default: impl Into<String>) -> Param {
    Param::new(key, label, ParamKind::text(default)).browse(PathKind::Folder)
}

/// The JPEG quality of the exports to files: sent only for a JPEG with the Custom check box on, so an untouched
/// export writes what the command writes with no quality (a quality would also make a WebP lossy).
fn export_quality() -> Param {
    Param::new("quality", tl!("Quality"), ParamKind::optional_number(0.0, 12.0, None, 10.0, 0)).enabled_when("format", "jpg")
}

/// The boards Export Selected Artboards exports: the selected top-level artboards while an artboard is the active
/// layer, else none (the checkbox is greyed and every board is exported).
fn selected_artboards(app: &PhotocraftApp) -> Vec<u64> {
    let Some(st) = app.session.active() else { return Vec::new() };
    let is_board = |id: &photocraft_doc::LayerId| st.doc.layers.iter().any(|l| l.id == *id && l.artboard().is_some());
    if !st.active_layer.as_ref().is_some_and(is_board) {
        return Vec::new();
    }
    st.selected_layers().iter().copied().filter(is_board).map(|id| id.0).collect()
}

/// Artboards to PDF's Export Selected Artboards (#13647): a checkbox over the command's own `artboards`, the ids of
/// `selected`. It opens ticked when there are any, else greyed and unticked, and it is set from the selection at every
/// open (a host-drawn row remembers nothing). Unticked, `artboards` is null and the command exports every board.
fn export_selected_artboards(selected: Vec<u64>) -> Param {
    let label = Label::from(tl!("Export Selected Artboards"));
    let ids = Value::from(selected.clone());
    let any = !selected.is_empty();
    let opening = ids.clone();
    let row = Param::custom("artboards", Label::Empty, move |ui, fields, _| {
        let ticked = any && matches!(fields.get("artboards"), Some(FieldValue::Json(Value::Array(_))));
        let out = Checkbox::new(Bind::given(ticked)).label(label.clone()).enabled(any).show(ui);
        match out.value().copied() {
            Some(true) if any => fields.set("artboards", FieldValue::Json(ids.clone())),
            // Kept as null, not removed: a missing key opens ticked again. The command takes null as every board.
            Some(false) => fields.set("artboards", FieldValue::Json(Value::Null)),
            _ => {}
        }
        out.response().clone()
    })
    // Without a row label, so the checkbox names itself, as a check box row does.
    .full_width();
    if any { row.opens_with(FieldValue::Json(opening)) } else { row }
}

/// A document's name without its extension.
fn stem(name: &str) -> String {
    name.rsplit_once('.').map_or(name, |(a, _)| a).to_owned()
}

/// File › Open EXR Parts…: with `{path}` in the parameters the picker is skipped; without, an EXR is picked first. Then
/// the parts and channel groups that become the layers of one document (a Maya/Arnold render writes one part per AOV).
fn open_exr_parts_dialog(app: &mut PhotocraftApp, params: &Value) -> Option<Result<Value, String>> {
    if let Some(path) = params.get("path").and_then(Value::as_str) {
        return Some(open_exr_parts(app, path));
    }
    Some(app.pick_file_bytes_filtered(&["exr"], |app, name, _bytes| open_exr_parts(app, &name)))
}

/// Reads the parts of the EXR at `path` and opens their form.
fn open_exr_parts(app: &mut PhotocraftApp, path: &str) -> Result<Value, String> {
    let info = app.run("file.exrParts", json!({"path": path})).map_err(|e| e.to_string())?;
    let entries = info.as_array().cloned().unwrap_or_default();
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path).to_owned();
    let mut rows = vec![
        Param::read_only_text("path", tl!("File"), file),
        choice("view", tl!("View"), opts(&[("both", tl!("Both")), ("left", tl!("Left")), ("right", tl!("Right"))]), "both"),
        Param::new("precomp", tl!("Add beauty precomp from the AOVs"), ParamKind::bool(false)),
    ];
    let mut any = false;
    for e in &entries {
        let kind = e["kind"].as_str().unwrap_or_default();
        let name = e["name"].as_str().unwrap_or_default();
        let channel_count = e["channels"].as_u64().unwrap_or(0);
        let key = format!("{kind}/{name}");
        match kind {
            "part" => {
                let on = !any && channel_count >= 3;
                any |= on;
                let label = format!("{name} ({}×{}, {channel_count} ch)", e["width"].as_u64().unwrap_or(0), e["height"].as_u64().unwrap_or(0));
                rows.push(Param::new(key, label, ParamKind::bool(on)));
            }
            "group" => rows.push(Param::new(key, format!("{name} ({channel_count} ch)"), ParamKind::bool(false))),
            _ => {}
        }
    }
    // Cryptomatte layers are information here (the picker comes separately); they ride along for the control channel.
    let crypto: Vec<Value> = entries.iter().filter(|e| e["kind"] == "cryptomatte").cloned().collect();
    let mut fixed = json!({"path": path});
    if !crypto.is_empty() {
        fixed["cryptomatte"] = Value::Array(crypto);
    }
    let id = open_form(app, "file.openExrParts", "Open EXR Parts…", rows, DialogFields::new(), fixed);
    Ok(opened(id))
}

/// The rows, and the opening values that are not row defaults, of command `id`'s form. `None` when `id` has no form or
/// its precondition (a document, a layer) is missing; the engine command then reports it.
pub(super) fn params_for(app: &mut PhotocraftApp, id: &str) -> Option<(Vec<Param>, DialogFields)> {
    let doc = app.session.active().map(|d| (d.doc.size.width, d.doc.size.height, d.doc.name.clone()));
    let dir = crate::dialogs::file_forms::default_dir(app);
    let plain = |rows: Vec<Param>| Some((rows, DialogFields::new()));
    match id {
        "layer.videoLayers.interpretFootage" => {
            let st = app.session.active()?;
            let layer = st.active_layer?;
            let fps = st.doc.layer(layer).and_then(|l| l.video.as_ref()).map_or(30.0, |v| f64::from(v.fps));
            plain(vec![Param::new("fps", tl!("Frame Rate"), ParamKind::number(Some(0.01), Some(1000.0), fps, 1.0, 2))])
        }
        "file.fileInfo" => file_info_form(app),
        "file.automate.fitImage" => {
            let (w, h, _) = doc?;
            // The values Fit Image last ran with, else the document's size (#13853). A stored value that isn't a
            // usable size is ignored. The engine keeps them in the preferences, so the rows opt out of the
            // dialog's own remembered values.
            let kept = app.session.prefs().dialogs.get(photocraft_engine::file_cmds::FIT_IMAGE).cloned().unwrap_or_default();
            let size = |key: &str| kept.get(key).and_then(Value::as_u64).filter(|v| (1..=300_000).contains(v)).and_then(|v| i64::try_from(v).ok());
            let enlarge = kept.get("dontEnlarge").and_then(Value::as_bool).unwrap_or(false);
            plain(vec![
                Param::new("width", tl!("Width"), ParamKind::int(Some(1), Some(300_000), size("width").unwrap_or(i64::from(w)))).not_remembered(),
                Param::new("height", tl!("Height"), ParamKind::int(Some(1), Some(300_000), size("height").unwrap_or(i64::from(h)))).not_remembered(),
                Param::new("dontEnlarge", tl!("Don't Enlarge"), ParamKind::bool(enlarge)).not_remembered(),
            ])
        }
        crate::dialogs::conditional_mode_change_dialog::COMMAND => plain(crate::dialogs::conditional_mode_change_dialog::rows()),
        "file.export.layersToFiles" => {
            let (_, _, name) = doc?;
            plain(vec![
                folder("dir", tl!("Destination"), dir),
                Param::new("prefix", tl!("File Name Prefix"), ParamKind::text(stem(&name))),
                Param::new("visibleOnly", tl!("Visible Layers Only"), ParamKind::bool(true)),
                choice("format", tl!("File Type"), export_formats(), "png"),
                export_quality(),
                Param::new("trim", tl!("Trim Layers"), ParamKind::bool(false)),
            ])
        }
        "file.export.layerCompsToFiles" => {
            let (_, _, name) = doc?;
            plain(vec![
                folder("dir", tl!("Destination"), dir),
                Param::new("prefix", tl!("File Name Prefix"), ParamKind::text(stem(&name))),
                Param::new("selectedOnly", tl!("Selected Layer Comp Only"), ParamKind::bool(false)),
                choice("format", tl!("File Type"), export_formats(), "png"),
                export_quality(),
            ])
        }
        "file.export.artboardsToFiles" => {
            let (_, _, name) = doc?;
            let mut rows = vec![
                folder("dir", tl!("Destination"), dir),
                Param::new("prefix", tl!("File Name Prefix"), ParamKind::text(stem(&name))),
                // Include Overlapping Areas / Artboard Content Only, right below the prefix, with no caption (#13629).
                Param::new(
                    "content",
                    "",
                    ParamKind::radio(
                        opts(&[("overlapping", tl!("Include Overlapping Areas")), ("artboard", tl!("Artboard Content Only"))]),
                        Some("overlapping".to_owned()),
                    ),
                ),
                // The reference application's list, PSD chosen when the dialog opens (#13635).
                choice("format", tl!("File Type"), artboard_formats(), "psd"),
            ];
            // Export Options and each file type's settings (#13636; the JPEG quality is one of them), then Include Artboard
            // Name and its Name Options (#13637). A section's body runs to the next section and hides with it, so both
            // checkboxes come before the Options section: after it, Include Artboard Name would show only with Export Options.
            let mut export = artboard_export_options::rows().into_iter();
            let mut name = artboard_name_options::rows(&crate::type_tool::families()).into_iter();
            rows.extend(export.next());
            rows.extend(name.next());
            rows.extend(export);
            rows.extend(name);
            plain(rows)
        }
        "file.export.artboardsToPdf" => {
            let (_, _, name) = doc?;
            plain(vec![
                Param::new("path", tl!("File"), ParamKind::text(format!("{dir}/{}.pdf", stem(&name)))),
                export_selected_artboards(selected_artboards(app)),
                // Include Overlapping Areas / Artboard Content Only, below Export Selected Artboards, with no caption (#13646).
                Param::new(
                    "content",
                    "",
                    ParamKind::radio(
                        opts(&[("overlapping", tl!("Include Overlapping Areas")), ("artboard", tl!("Artboard Content Only"))]),
                        Some("overlapping".to_owned()),
                    ),
                ),
                // Ticked on first open (#13648).
                Param::new("includeBackground", tl!("Include Background in Export"), ParamKind::bool(true)),
                Param::new("quality", tl!("Quality"), ParamKind::int(Some(0), Some(12), 10)),
                // The page mode, a radio pair with no caption, Multi-Page Document first and selected (#13649).
                Param::new(
                    "mode",
                    "",
                    ParamKind::radio(
                        opts(&[("multiPage", tl!("Multi-Page Document")), ("perArtboard", tl!("Document Per Artboard"))]),
                        Some("multiPage".to_owned()),
                    ),
                ),
                // Unticked on first open; the form remembers it as the last run left it (#13644).
                Param::new("reversePageOrder", tl!("Reverse Page Order"), ParamKind::bool(false)),
            ])
        }
        "file.export.dataSetsAsFiles" => {
            doc?;
            plain(vec![
                folder("dir", tl!("Destination"), dir),
                Param::new("naming", tl!("File Naming"), ParamKind::text("{name}")),
                choice("format", tl!("File Type"), export_formats(), "png"),
            ])
        }
        "layer.new.artboard" => {
            let (w, h, _) = doc?;
            let mut presets = vec![(String::new(), Label::from(tl!("Custom")))];
            presets.extend(photocraft_engine::artboard_cmds::PRESETS.iter().map(|p| (p.0.to_owned(), Label::from(crate::i18n::t(p.0).to_owned()))));
            plain(vec![
                Param::new("name", tl!("Name"), ParamKind::text("")),
                choice("preset", tl!("Preset"), presets, ""),
                Param::new("width", tl!("Width"), ParamKind::int(Some(1), None, i64::from(w))),
                Param::new("height", tl!("Height"), ParamKind::int(Some(1), None, i64::from(h))),
                choice(
                    "background",
                    tl!("Background Contents"),
                    opts(&[("white", tl!("White")), ("black", tl!("Black")), ("transparent", tl!("Transparent"))]),
                    "white",
                ),
            ])
        }
        "file.export.colorLookupTables" => {
            let (_, _, name) = doc?;
            let base = stem(&name);
            // A selected adjustment is a common use case, but existing no-selection exports
            // continue to bake the entire visible stack. The scope remains explicit in the UI.
            let selected_adjustments = app.session.active().is_some_and(|st| {
                let chosen = st.selected_layers();
                !chosen.is_empty()
                    && chosen.iter().all(|id| {
                        st.doc.layers.iter().any(|root| root.id == *id)
                            && st.doc.layer(*id).is_some_and(|layer| layer.visible && matches!(layer.content, photocraft_doc::LayerContent::Adjustment(_)))
                    })
            });
            let scope = if selected_adjustments { "selected" } else { "all" };
            plain(vec![
                Param::new("path", tl!("File"), ParamKind::text(format!("{dir}/{base}.cube"))),
                Param::new("size", tl!("Grid Size"), ParamKind::int(Some(2), Some(256), 33)),
                Param::new("title", tl!("Title"), ParamKind::text(base)),
                choice("scope", tl!("Scope"), opts(&[("all", tl!("All Visible Layers")), ("selected", tl!("Selected Adjustments"))]), scope),
            ])
        }
        // Photography automation (photo_cmds / lens_cmds): a folder (or the open documents).
        "file.automate.photomerge" => plain(vec![
            folder("paths", tl!("Source Folder"), dir),
            Param::new("useOpenDocuments", tl!("Use Open Documents"), ParamKind::bool(false)),
            choice(
                "layout",
                tl!("Layout"),
                opts(&[
                    ("auto", tl!("Auto")),
                    ("perspective", tl!("Perspective")),
                    ("cylindrical", tl!("Cylindrical")),
                    ("spherical", tl!("Spherical")),
                    ("collage", tl!("Collage")),
                    ("reposition", tl!("Reposition")),
                ]),
                "auto",
            ),
            Param::new("blend", tl!("Blend Images Together"), ParamKind::bool(true)),
            Param::new("vignetteRemoval", tl!("Vignette Removal"), ParamKind::bool(false)),
            Param::new("geometricCorrection", tl!("Geometric Distortion Correction"), ParamKind::bool(false)),
            Param::new("contentAwareFill", tl!("Content-Aware Fill Transparent Areas"), ParamKind::bool(false)),
        ]),
        "file.automate.mergeToHdrPro" => plain(vec![
            folder("paths", tl!("Source Folder"), dir),
            Param::new("useOpenDocuments", tl!("Use Open Documents"), ParamKind::bool(false)),
            Param::new("align", tl!("Attempt to Automatically Align Source Images"), ParamKind::bool(true)),
            Param::new("removeGhosts", tl!("Remove Ghosts"), ParamKind::bool(false)),
            choice("mode", tl!("Mode"), opts(&[("32", tl!("32 Bit")), ("16", tl!("16 Bit")), ("8", tl!("8 Bit"))]), "32"),
            choice(
                "method",
                tl!("Method"),
                opts(&[
                    ("localAdaptation", tl!("Local Adaptation")),
                    ("exposureGamma", tl!("Exposure and Gamma")),
                    ("highlightCompression", tl!("Highlight Compression")),
                    ("equalizeHistogram", tl!("Equalize Histogram")),
                ]),
                "localAdaptation",
            ),
            Param::new("radius", tl!("Radius"), ParamKind::number(Some(0.0), None, 7.0, 0.5, 1)),
            Param::new("strength", tl!("Strength"), ParamKind::number(Some(0.0), None, 0.52, 0.01, 2)),
            Param::new("gamma", tl!("Gamma"), ParamKind::number(Some(0.0), None, 1.0, 0.01, 2)),
            Param::new("exposure", tl!("Exposure"), ParamKind::number(None, None, 0.0, 0.1, 2)),
            Param::new("detail", tl!("Detail"), ParamKind::number(None, None, 30.0, 1.0, 0)),
            Param::new("saturation", tl!("Saturation"), ParamKind::number(None, None, 20.0, 1.0, 0)),
        ]),
        crate::dialogs::lens_batch_ui::COMMAND => plain(crate::dialogs::lens_batch_ui::rows(dir.clone())),
        "file.scripts.imageProcessor" => plain(vec![
            choice(
                "source",
                tl!("Source"),
                opts(&[("current", tl!("Current document")), ("open", tl!("All open documents")), ("folder", tl!("Folder"))]),
                if doc.is_some() { "current" } else { "folder" },
            ),
            folder("input", tl!("Input Folder"), dir.clone()).visible_when("source", "folder"),
            Param::new("includeSubfolders", tl!("Include All Subfolders"), ParamKind::bool(false)).visible_when("source", "folder"),
            folder("output", tl!("Output Folder"), format!("{dir}/processed")),
            Param::section(tl!("File Type")),
            // One section per format: the checkbox that enables it, then its own options and size.
            Param::new("jpeg/enabled", tl!("Save as JPEG"), ParamKind::bool(true)),
            Param::new("jpeg/quality", tl!("Quality"), ParamKind::number(Some(1.0), Some(12.0), 5.0, 1.0, 2)).enabled_when("jpeg/enabled", true),
            Param::new("jpeg/convertToSrgb", tl!("Convert Profile to sRGB"), ParamKind::bool(false)).enabled_when("jpeg/enabled", true),
            Param::new("jpeg/resize", tl!("Resize to Fit"), ParamKind::bool(false)).enabled_when("jpeg/enabled", true),
            Param::new("jpeg/width", tl!("W"), ParamKind::int(Some(1), Some(30_000), 1)).enabled_when("jpeg/resize", true),
            Param::new("jpeg/height", tl!("H"), ParamKind::int(Some(1), Some(30_000), 1)).enabled_when("jpeg/resize", true),
            Param::new("psd/enabled", tl!("Save as PSD"), ParamKind::bool(false)),
            Param::new("psd/maximizeCompatibility", tl!("Maximize Compatibility"), ParamKind::bool(true)).enabled_when("psd/enabled", true),
            Param::new("psd/resize", tl!("Resize to Fit"), ParamKind::bool(false)).enabled_when("psd/enabled", true),
            Param::new("psd/width", tl!("W"), ParamKind::int(Some(1), Some(30_000), 1)).enabled_when("psd/resize", true),
            Param::new("psd/height", tl!("H"), ParamKind::int(Some(1), Some(30_000), 1)).enabled_when("psd/resize", true),
            Param::new("tiff/enabled", tl!("Save as TIFF"), ParamKind::bool(false)),
            Param::new("tiff/lzw", tl!("LZW Compression"), ParamKind::bool(false)).enabled_when("tiff/enabled", true),
            Param::new("tiff/resize", tl!("Resize to Fit"), ParamKind::bool(false)).enabled_when("tiff/enabled", true),
            Param::new("tiff/width", tl!("W"), ParamKind::int(Some(1), Some(30_000), 1)).enabled_when("tiff/resize", true),
            Param::new("tiff/height", tl!("H"), ParamKind::int(Some(1), Some(30_000), 1)).enabled_when("tiff/resize", true),
        ]),
        "file.automate.batch" => batch_form(app, &dir),
        "select.saveSelection" | "select.loadSelection" | "image.applyImage" | "image.calculations" => channel_form(app, id),
        _ => None,
    }
}

/// File › File Info…: the document's metadata in Photoshop's order and labels, then its read-only Camera Data.
fn file_info_form(app: &mut PhotocraftApp) -> Option<(Vec<Param>, DialogFields)> {
    let info = app.session.execute("file.fileInfo", json!({})).ok()?;
    let text = |key: &str| info[key].as_str().unwrap_or_default().to_owned();
    let keywords = info["keywords"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; ")).unwrap_or_default();
    let mut rows = vec![
        Param::new("title", tl!("Document Title"), ParamKind::text("")),
        Param::new("author", tl!("Author"), ParamKind::text("")),
        Param::new("authorTitle", tl!("Author Title"), ParamKind::text("")),
        Param::new("description", tl!("Description"), ParamKind::text("")).multiline(3),
        Param::new("keywords", tl!("Keywords"), ParamKind::text("")).multiline(3),
        choice(
            "copyrightStatus",
            tl!("Copyright Status"),
            opts(&[("unknown", tl!("Unknown")), ("copyrighted", tl!("Copyrighted")), ("publicDomain", tl!("Public Domain"))]),
            "unknown",
        ),
        Param::new("copyright", tl!("Copyright Notice"), ParamKind::text("")).multiline(3),
        Param::new("copyrightUrl", tl!("Copyright Info URL"), ParamKind::text("")),
    ];
    let camera: Vec<(&'static str, String)> = photocraft_engine::file_cmds::CAMERA_FIELDS
        .iter()
        .filter_map(|(key, label)| info["camera"].get(*key).and_then(Value::as_str).map(|v| (*label, v.to_owned())))
        .collect();
    if !camera.is_empty() {
        rows.push(Param::section(tl!("Camera Data")));
        let lang = crate::i18n::current();
        for (label, value) in camera {
            rows.push(Param::read_only_text(format!("camera/{label}"), crate::i18n::tr_ctx(lang, "cameraData", label), value));
        }
    }
    let status = info["copyrightStatus"].as_str().unwrap_or("unknown").to_owned();
    let fields = DialogFields::from_pairs([
        ("title", text("title")),
        ("author", text("author")),
        ("authorTitle", text("authorTitle")),
        ("description", text("description")),
        ("keywords", keywords),
        ("copyrightStatus", status),
        ("copyright", text("copyright")),
        ("copyrightUrl", text("copyrightUrl")),
    ]);
    Some((rows, fields))
}

/// File › Automate › Batch…: the selected action and its set, and where the files come from and go.
fn batch_form(app: &PhotocraftApp, dir: &str) -> Option<(Vec<Param>, DialogFields)> {
    let action = crate::actions::selected_action(app)?;
    let (name, set) = (action.name.clone(), action.set.clone());
    let (names, sets) = crate::actions::action_form_choices(app);
    let mut rows = Vec::new();
    // The Set dropdown picks between same-named actions; an action outside any set has none.
    if !sets.is_empty() {
        rows.push(Param::new("set", tl!("Set"), ParamKind::choice(named(&sets), set)));
    }
    rows.extend([
        choice("action", tl!("Action"), named(&names), &name),
        choice("source", tl!("Source"), opts(&[("folder", tl!("Folder")), ("open", tl!("Opened Files"))]), "folder"),
        folder("input", tl!("Input Folder"), dir),
        Param::new("includeSubfolders", tl!("Include All Subfolders"), ParamKind::bool(false)),
        folder("output", tl!("Output Folder"), format!("{dir}/batch")),
        choice(
            "format",
            tl!("File Type"),
            opts(&[
                ("same", tl!("Same as Source")),
                ("png", tl!("PNG")),
                ("jpg", tl!("JPEG")),
                ("psd", tl!("PSD")),
                ("tiff", tl!("TIFF")),
                ("webp", tl!("WebP")),
            ]),
            "same",
        ),
        Param::new("naming", tl!("File Naming"), ParamKind::text("")).hint("{name}").tooltip("{name} · {serial} · {date} (UTC)"),
        Param::new("startingSerial", tl!("Starting Serial Number"), ParamKind::int(Some(1), None, 1)),
        // Off sends no quality, which keeps the command's JPEG and lossless WebP defaults.
        Param::new("quality", tl!("Quality"), ParamKind::optional_number(0.0, 12.0, None, 8.0, 0)),
        Param::new("stopOnError", tl!("Stop For Errors"), ParamKind::bool(false)),
        Param::new("errorLog", tl!("Error Log"), ParamKind::text("")),
    ]);
    Some((rows, DialogFields::new()))
}

/// The channel workflow forms (see channels_panel.rs for the channel references): Save and Load Selection, Apply
/// Image and Calculations.
fn channel_form(app: &PhotocraftApp, id: &str) -> Option<(Vec<Param>, DialogFields)> {
    let st = app.session.active()?;
    let d = &st.doc;
    let mode = d.pixel_format().mode;
    let mut chans: Vec<String> = vec![photocraft_engine::channel_cmds::composite_name(mode).to_string()];
    if mode.color_channels() > 1 {
        chans.extend(photocraft_engine::channel_cmds::color_names(mode).iter().map(|n| n.to_string()));
    }
    chans.extend(d.channels.iter().map(|c| c.name.clone()));
    let alphas: Vec<String> = std::iter::once("new".to_string()).chain(d.channels.iter().map(|c| c.name.clone())).collect();
    // The active layer's transparency, and its mask only when it has one.
    let layer = st.active_layer.and_then(|l| d.layer(l));
    let mut load = chans.clone();
    if layer.is_some() {
        load.push("transparency".into());
    }
    if layer.is_some_and(|l| l.mask.is_some()) {
        load.push("mask".into());
    }
    load.push("selection".into());
    let operation = |with_replace: bool| {
        let mut list = vec![("new", tl!("New"))];
        if with_replace {
            list.push(("replace", tl!("Replace")));
        }
        list.extend([("add", tl!("Add")), ("subtract", tl!("Subtract")), ("intersect", tl!("Intersect"))]);
        opts(&list)
    };
    let rows = match id {
        "select.saveSelection" => vec![
            choice("channel", tl!("Channel"), channels(&alphas), "new"),
            Param::new("name", tl!("Name"), ParamKind::text("")),
            choice("operation", tl!("Operation"), operation(true), "new"),
        ],
        "select.loadSelection" => {
            let first = d.channels.first().map_or_else(|| chans[0].clone(), |c| c.name.clone());
            vec![
                choice("channel", tl!("Channel"), channels(&load), &first),
                Param::new("invert", tl!("Invert"), ParamKind::bool(false)),
                choice("operation", tl!("Operation"), operation(false), "new"),
            ]
        }
        "image.applyImage" => {
            let mut masks = vec!["none".to_string()];
            masks.extend(load.iter().cloned());
            // Source layer: Merged, then the pixel layers top to bottom (the engine takes
            // the topmost layer of a name), shown by their own names.
            let mut layers = vec!["merged".to_string()];
            for (_, _, l) in d.walk().into_iter().rev() {
                if l.surface().is_some() && !layers.contains(&l.name) {
                    layers.push(l.name.clone());
                }
            }
            // Scale and Offset only affect Add and Subtract.
            vec![
                choice("sourceLayer", tl!("Layer"), channels(&layers), "merged"),
                choice("sourceChannel", tl!("Source Channel"), channels(&load), &chans[0]),
                Param::new("sourceInvert", tl!("Invert"), ParamKind::bool(false)),
                choice("blending", tl!("Blending"), blend_modes(), "multiply"),
                Param::new("opacity", tl!("Opacity"), ParamKind::number(Some(0.0), Some(100.0), 100.0, 1.0, 0)),
                Param::new("scale", tl!("Scale"), ParamKind::number(Some(0.001), Some(2.0), 1.0, 0.1, 3)),
                Param::new("offset", tl!("Offset"), ParamKind::int(Some(-255), Some(255), 0)),
                Param::new("preserveTransparency", tl!("Preserve Transparency"), ParamKind::bool(false)),
                choice("maskChannel", tl!("Mask Channel"), channels(&masks), "none"),
            ]
        }
        _ => vec![
            choice("source1Channel", tl!("Source 1 Channel"), channels(&load), &chans[0]),
            Param::new("source1Invert", tl!("Invert Source 1"), ParamKind::bool(false)),
            choice("source2Channel", tl!("Source 2 Channel"), channels(&load), &chans[0]),
            Param::new("source2Invert", tl!("Invert Source 2"), ParamKind::bool(false)),
            choice("blending", tl!("Blending"), blend_modes(), "multiply"),
            Param::new("opacity", tl!("Opacity"), ParamKind::number(Some(0.0), Some(100.0), 100.0, 1.0, 0)),
            choice(
                "result",
                tl!("Result"),
                opts(&[("newChannel", tl!("New Channel")), ("newDocument", tl!("New Document")), ("selection", tl!("Selection"))]),
                "newChannel",
            ),
        ],
    };
    Some((rows, DialogFields::new()))
}

/// Batch's OK with Source: Folder and no source folder chosen (#13669).
pub(crate) const NO_SOURCE_FOLDER: &str = "You must choose a source folder.";
/// Batch's OK with a folder destination and no destination folder chosen (#13669).
pub(crate) const NO_DESTINATION_FOLDER: &str = "You must choose a destination folder.";

/// Does a Batch folder field name a folder? No value and an empty or blank path count as nothing chosen.
fn batch_folder_chosen(path: Option<&str>) -> bool {
    path.is_some_and(|path| !path.trim().is_empty())
}

/// The first of the reference's OK checks the Batch dialog's values fail, in its order: the source folder, then the
/// destination folder. Only a folder source has either: Opened Files reads no folder, and without an output it leaves
/// the documents open.
fn batch_missing_choice(fields: &DialogFields) -> Option<&'static str> {
    if fields.text("source").is_some_and(|source| source != "folder") {
        return None;
    }
    if !batch_folder_chosen(fields.text("input")) {
        return Some(NO_SOURCE_FOLDER);
    }
    let folder_destination = fields.text("destination").is_none_or(|destination| destination == "folder");
    (folder_destination && !batch_folder_chosen(fields.text("output"))).then_some(NO_DESTINATION_FOLDER)
}

/// The `ParamFamily::check` of the Batch dialog (#13669): OK, from the button or a confirm through the control channel,
/// is refused while a folder source or its destination names no folder. As in the reference, a one-button alert
/// titled "Batch" shows the message; the dialog stays open with its values (the refusal also shows under its rows and
/// answers a control-channel confirm), and nothing runs.
pub(crate) fn batch_check(app: &mut PhotocraftApp, result: &ParamResult) -> Result<(), Label> {
    let message = match batch_missing_choice(&result.fields) {
        Some(NO_SOURCE_FOLDER) => tl!("You must choose a source folder."),
        Some(_) => tl!("You must choose a destination folder."),
        None => return Ok(()),
    };
    crate::dialogs::alerts::show(app, craft::ui::dialog::Alert::new(tl!("Batch"), message));
    Err(Label::from(message))
}

#[cfg(test)]
#[path = "batch_ok_validation_tests.rs"]
mod batch_ok_validation_tests;

/// Handle menu commands that need a shell action or a parameter dialog before execution.
pub(super) fn front(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id == "file.closeAll" || id == "file.closeOthers" {
        // Keep the remaining document's view (views are index-aligned with documents).
        let keep = app.session.active_index().and_then(|i| app.ui.views.get(i).cloned());
        let r = app.run(id, params.clone());
        if r.is_ok() {
            app.ui.views = if id == "file.closeAll" { Vec::new() } else { keep.into_iter().collect() };
            app.ui.windows.clear();
            app.sync_views();
        }
        return Some(r);
    }
    if id == "type.pasteLoremIpsum" && no_params(params) {
        // While typing, paste at the caret.
        if let Some(te) = app.ui.text_edit.clone() {
            let r = app.run(id, json!({"layer": te.layer, "at": te.caret, "coalesce": te.session}));
            if let (Ok(v), Some(te)) = (&r, app.ui.text_edit.as_mut())
                && let Some(c) = v.get("caret").and_then(Value::as_u64)
            {
                te.caret = c as usize;
                te.anchor = c as usize;
            }
            return Some(r);
        }
        return None;
    }
    // The form's own OK runs the command with every part chosen; only a lone `path` asks for the parts of that file.
    let path_only = params.as_object().is_some_and(|o| o.len() == 1 && o.contains_key("path"));
    if id == "file.openExrParts" && (no_params(params) || path_only) {
        return open_exr_parts_dialog(app, params);
    }
    if !no_params(params) {
        return None;
    }
    let label = photocraft_engine::commands::find(id).map_or(id, |c| c.label);
    match id {
        "file.openAs" => Some(app.open_dialog_file()),
        "file.openAsSmartObject" => Some(app.open_dialog_as_smart_object()),
        "file.saveACopy" => Some(save_a_copy(app)),
        "file.placeEmbedded" | "file.placeLinked" => {
            if app.ui.transform.is_some() {
                return Some(Err("Commit or cancel the transform first".into()));
            }
            let doc = match app.active_doc_id() {
                Ok(doc) => doc,
                Err(e) => return Some(Err(e)),
            };
            let linked = id == "file.placeLinked";
            let (ctx, queue_ctx) = (ctx.clone(), ctx.clone());
            let place = move |app: &mut PhotocraftApp, name: String, bytes: Vec<u8>| {
                if app.ui.transform.is_some() {
                    return Err("Commit or cancel the transform first".into());
                }
                app.refocus(doc)?;
                let linked = linked.then(|| name.clone());
                let result = app.place_bytes(&name, bytes, linked)?;
                // In the background, Free Transform starts when the layer has landed.
                if result.get("frame").and_then(Value::as_u64).is_none() && !crate::jobs_ui::track_place(app, &name, &result) {
                    crate::transform_tool::begin_placed(app, &ctx)?;
                }
                Ok(result)
            };
            if linked {
                return Some(app.pick_file_bytes(place));
            }
            // Place Embedded takes several files (#14107); one file is placed as before.
            Some(app.pick_files_bytes(move |app, files| match <[crate::file_dialog::PickedFile; 1]>::try_from(files) {
                Ok([file]) => {
                    let (name, bytes) = file.read()?;
                    place(app, name, bytes)
                }
                Err(files) => {
                    if app.ui.transform.is_some() {
                        return Err("Commit or cancel the transform first".into());
                    }
                    app.refocus(doc)?;
                    let count = files.len();
                    app.place_picked(&queue_ctx, doc, files);
                    Ok(json!({ "files": count }))
                }
            }))
        }
        "layer.smartObjects.replaceContents" => {
            let st = app.session.active()?;
            let (doc, layer) = (st.doc.id, st.active_layer?);
            Some(app.pick_file_bytes(move |app, name, bytes| {
                app.with_document(doc, |app| {
                    let r = photocraft_engine::smart_cmds::replace_contents_bytes(&mut app.session, layer, &name, bytes).map_err(|e| e.to_string());
                    app.sync_views();
                    r
                })
            }))
        }
        "layer.smartObjects.exportContents" => Some(export_contents(app)),
        "layer.smartObjects.relinkToFile" => {
            let st = app.session.active()?;
            let (doc, layer) = (st.doc.id, st.active_layer?);
            // A link needs a file on disk: the desktop picker answers its full path, the web
            // picker only a name and the bytes, which can't be linked to.
            Some(app.pick_file_bytes(move |app, path, _| {
                if !std::path::Path::new(&path).is_absolute() {
                    return Err(format!("{path}: Relink to File needs a file on disk"));
                }
                app.with_document(doc, |app| app.run("layer.smartObjects.relinkToFile", json!({"layer": layer.0, "path": path})))
            }))
        }
        "layer.smartObjects.convertToLinked" => {
            let st = app.session.active()?;
            let (doc, layer) = (st.doc.id, st.active_layer?);
            let (name, _) = match photocraft_engine::smart_cmds::contents_of(&st.doc, layer) {
                Ok(c) => c,
                Err(e) => return Some(Err(e.to_string())),
            };
            Some(app.pick_save(&name, move |app, path| {
                app.with_document(doc, |app| app.run("layer.smartObjects.convertToLinked", json!({"layer": layer.0, "path": path})))
            }))
        }
        "layer.videoLayers.newVideoLayerFromFile" | "layer.videoLayers.replaceFootage" => {
            let doc = match app.active_doc_id() {
                Ok(doc) => doc,
                Err(e) => return Some(Err(e)),
            };
            let cmd = id.to_string();
            Some(app.pick_file_bytes(move |app, path, _bytes| {
                app.with_document(doc, |app| {
                    let r = app.run(&cmd, json!({"path": path}));
                    if let Ok(v) = &r {
                        // #11113: footage salvaged from a truncated GIF or WebP must say so.
                        let what = crate::i18n::fmt(crate::i18n::t("Imported {name}"), &[("name", &crate::file_open::display_name(&path))]);
                        crate::notices::footage_warnings(app, v, &what);
                    }
                    if r.is_ok() && app.session.active().is_some_and(|d| d.doc.timeline.is_some()) {
                        let _ = crate::panel_docking::show_panel(app, "timeline", Some(true));
                    }
                    app.sync_views();
                    r
                })
            }))
        }
        "view.newGuideLayout" => Some(crate::guide_layout_ui::open(app).map(opened).ok_or_else(|| "no document".into())),
        "type.warpText" => Some(crate::dialogs::open_command_dialog(app, id, label).map(opened)),
        "file.scripts.loadFilesIntoStack" => Some(Ok(json!({"dialog": crate::stack_dialog::open(app).value().to_string()}))),
        "file.export.dataSetsAsFiles" if cfg!(target_arch = "wasm32") => Some(Err(tl!("Data Sets as Files is unavailable on the web").to_string())),
        "file.automate.batch" if crate::actions::selected_action(app).is_none() => Some(Err("record an action in the Actions panel first".into())),
        _ if is_form(id) => {
            let (rows, fields) = params_for(app, id)?;
            Some(Ok(opened(open_form(app, id, label, rows, fields, Value::Null))))
        }
        _ => None,
    }
}

/// The name Save a Copy suggests: "<name> copy.psd" for a document with a file, or with
/// Preferences › File Handling › Do not append "copy" to filename (`copy_suffix` empty) the
/// document's own name (only the extension may change), as in Photoshop (#8080). A document that
/// was never saved suggests its own name without " copy" either way (#14169).
pub(super) fn suggested_copy_name(doc_name: &str, has_file: bool, copy_suffix: &str) -> String {
    let stem = doc_name.rsplit_once('.').map_or(doc_name, |(a, _)| a);
    let suffix = if has_file { copy_suffix } else { "" };
    format!("{stem}{suffix}.psd")
}

/// File › Save a Copy: pick a name, encode with the export service, write; the document's path
/// and saved state are untouched. An action being recorded keeps it as a `file.saveACopy` step
/// with its path (#4373).
fn save_a_copy(app: &mut PhotocraftApp) -> Result<Value, String> {
    let doc = app.session.active().ok_or("no document")?.doc.clone();
    let has_file = app.session.active().is_some_and(|d| d.path.is_some());
    let suggested = suggested_copy_name(&doc.name, has_file, app.session.prefs().file_handling.copy_suffix());
    // The copy is of the document as it was when asked.
    app.pick_save(&suggested, move |app, path| {
        // An RGB Targa asks for its bits per pixel first, a DDS texture for its compression, a
        // JPEG for its JPEG Options and a TIFF for its TIFF Options; the copy is written from the
        // prompt.
        let parked = app.with_document(doc.id, |app| {
            if crate::targa_options_ui::wants_prompt(app, &path) {
                crate::targa_options_ui::park(app, path.clone(), true, "file.saveACopy")?;
                return Ok(true);
            }
            if crate::dds_options_ui::wants_prompt(&path) {
                crate::dds_options_ui::park(app, path.clone(), true, "file.saveACopy")?;
                return Ok(true);
            }
            if crate::jpeg_options_ui::wants_prompt(app, &path) {
                crate::jpeg_options_ui::park(app, path.clone(), true, "file.saveACopy")?;
                return Ok(true);
            }
            if crate::tiff_options_ui::wants_prompt(app, &path) {
                crate::tiff_options_ui::park(app, path.clone(), true, "file.saveACopy")?;
                return Ok(true);
            }
            Ok(false)
        })?;
        if parked {
            return Ok(json!({"path": path, "warnings": []}));
        }
        let r = write_copy(app, &doc, &path, crate::FormatChoices::default())?;
        let step = json!({"path": path});
        photocraft_engine::actions_cmds::note_automation_step(&mut app.session, "file.saveACopy", &step);
        app.session.journal.push(("file.saveACopy".into(), step));
        Ok(r)
    })
}

/// Writes `doc` to `path` as a copy (Save a Copy, and its replayed action step): the document's
/// path and saved state are untouched. `choices` are the Targa, DDS and JPEG answers, when there
/// are any.
pub(crate) fn write_copy(app: &mut PhotocraftApp, doc: &photocraft_doc::Document, path: &str, choices: crate::FormatChoices) -> Result<Value, String> {
    let settings = choices.apply(app.save_settings());
    crate::app::ensure_memory_to_save(doc, path)?;
    let export = app.services.export.as_ref().ok_or("no exporter configured")?;
    let (bytes, warnings) = export(doc, path, &settings)?;
    let write = app.services.write.as_mut().ok_or("no writer configured")?;
    write(path, &bytes)?;
    app.detach_others_from_file(path, doc.id);
    let template = "Saved a copy as {path}";
    app.ui.status = crate::i18n::fmt(tl!(template), &[("path", path)]);
    app.ui.status_error = false;
    let what = crate::i18n::fmt(tl!(template), &[("path", &crate::file_open::display_name(path))]);
    crate::notices::io_warnings(app, &what, &warnings);
    Ok(json!({"path": path, "warnings": warnings}))
}

/// Layer › Smart Objects › Export Contents: saves the contents file under its own name by default.
fn export_contents(app: &mut PhotocraftApp) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document")?;
    let layer = st.active_layer.ok_or("no layer selected")?;
    // The contents as they were when asked, like Save a Copy.
    let (name, bytes) = photocraft_engine::smart_cmds::contents_of(&st.doc, layer).map_err(|e| e.to_string())?;
    app.pick_save(&name.clone(), move |app, path| {
        let write = app.services.write.as_mut().ok_or("no writer configured")?;
        write(&path, &bytes)?;
        let template = "Exported contents to {path}";
        app.ui.status = crate::i18n::fmt(tl!(template), &[("path", &path)]);
        app.ui.status_error = false;
        Ok(json!({"path": path, "fileName": name, "bytes": bytes.len()}))
    })
}

#[cfg(test)]
#[path = "artboards_to_pdf_mode_tests.rs"]
mod artboards_to_pdf_mode_tests;
#[cfg(test)]
#[path = "../../artboards_to_pdf_selected_tests.rs"]
mod artboards_to_pdf_selected_tests;
