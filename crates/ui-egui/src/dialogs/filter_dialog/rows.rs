//! The filter dialogs' rows that need the app: what the schema parser cannot build from a spec string.
//!
//! [`custom_rows`] turns the parsed rows of one command into the rows the dialog shows: Indexed Color's matte and
//! colour count, Lens Blur's depth sources, the installed profiles, Color Lookup's LUT picker, Match Color's
//! source document and layer, Replace Color's droppers and Result, Radial Blur's centre and the Blur Gallery's
//! blur row.
//!
//! These rows keep values under keys no command parameter has (`matteColor`, `sampleTool`, `resultPicked`,
//! `lutSize`, `lutError`) and Lens Blur's `channel:N` sources and Match Color's textual
//! `source`; the family's `command_params` (dialogs-w24) maps them back and leaves the private ones out.

use std::path::PathBuf;
use std::sync::Arc;

use craft::behavior::{CursorBadge, EditEvent, IconId, Mixed};
use craft::cms::lutfile::write_cube;
use craft::cms::{ColorSpace, TransformOptions};
use craft::color_ui::ui::{LutChoice, LutPicker, LutPickerOutput};
use craft::imaging::ui::color_range::{ColorSamples, SampleSign, SampleTool};
use craft::keymap::Mods;
use craft::theme::Srgba;
use craft::ui::buttons::ToggleButton;
use craft::ui::choice::Swatch;
use craft::ui::contract::{Bind, Entry, Outcome as _, Props as _, Show as _};
use craft::ui::dialog::{DialogId, StackDialog as _};
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamKind};
use craft::ui::pickers::Select;
use craft::ui::primitives::IconSpec;
use craft::ui::runtime::env::Label;
use craft::ui::runtime::requests::EyedropperRequest;
use photocraft_color::ColorMode;
use photocraft_doc::Document;
use photocraft_engine::installed_profiles::InstalledProfile;
use serde_json::{Value, json};

use crate::PhotocraftApp;

const INDEXED_COLOR: &str = "image.mode.indexedColor";
pub(crate) const LENS_BLUR: &str = "filter.blur.lensBlur";
const MATCH_COLOR: &str = "image.adjustments.matchColor";
const COLOR_LOOKUP: &str = "image.adjustments.colorLookup";
pub(crate) const REPLACE_COLOR: &str = "image.adjustments.replaceColor";
const RADIAL_BLUR: &str = "filter.blur.radialBlur";

/// Where Indexed Color's custom matte colour is kept; `matte` is `custom` while it applies.
pub(crate) const MATTE_COLOR: &str = "matteColor";
/// The Replace Color dropper armed: `sample`, `add`, `subtract`, or empty when none is.
pub(crate) const SAMPLE_TOOL: &str = "sampleTool";
/// The colour the Result swatch's Color Picker answers into; it is there only from the answer to the next pass.
const RESULT_PICKED: &str = "resultPicked";
/// The size and name of the LUT Load 3D LUT… embedded, and why one could not be read.
const LUT_SIZE: &str = "lutSize";
const LUT_ERROR: &str = "lutError";

/// Builds the rows `command`'s dialog shows from the rows `params` the spec parser made.
pub(crate) fn custom_rows(app: &mut PhotocraftApp, command: &str, params: &mut Vec<Param>) {
    match command {
        INDEXED_COLOR => indexed_color(app, params),
        LENS_BLUR => lens_blur(app, params),
        MATCH_COLOR => match_color(app, params),
        COLOR_LOOKUP => color_lookup(app, params),
        REPLACE_COLOR => replace_color(app, params),
        RADIAL_BLUR => radial_blur(params),
        _ if crate::blur_pins::handles(command) => {
            if let Some((kind, bounds, start)) = crate::blur_pins::initial(app, command) {
                params.push(crate::blur_pins::blur_row(kind, bounds, start));
            }
        }
        _ => {}
    }
    installed_profiles(command, params);
}

/// The row `key` of `params`, mutably.
fn row<'a>(params: &'a mut [Param], key: &str) -> Option<&'a mut Param> {
    params.iter_mut().find(|p| p.key == key)
}

/// Replaces the kind of the row `key`, passing the old one.
fn retype(params: &mut [Param], key: &str, kind: impl FnOnce(&ParamKind) -> ParamKind) {
    if let Some(p) = row(params, key) {
        p.kind = kind(&p.kind);
    }
}

// --- Indexed Color ---------------------------------------------------------------------------------------------

/// The matte menu's entries: the named mattes, then `custom` for the colour row after it.
fn matte_options() -> Vec<(String, Label)> {
    [
        ("none", Label::from(tl!("None"))),
        ("foreground", Label::from(tl!("Foreground Color"))),
        ("background", Label::from(tl!("Background Color"))),
        ("white", Label::from(tl!("White"))),
        ("black", Label::from(tl!("Black"))),
        ("gray", Label::from(tl!("50% Gray"))),
        ("netscapeGray", Label::from(tl!("Netscape Gray"))),
        ("custom", Label::from(tl!("Other…"))),
    ]
    .into_iter()
    .map(|(id, label)| (id.to_owned(), label))
    .collect()
}

fn indexed_color(app: &PhotocraftApp, params: &mut Vec<Param>) {
    retype(params, "colors", |old| {
        let default = match old {
            ParamKind::Range { default, .. } => default.round().clamp(2.0, 256.0) as i64,
            _ => 256,
        };
        ParamKind::int(Some(2), Some(256), default)
    });
    let Some(at) = params.iter().position(|p| p.key == "matte") else { return };
    retype(params, "matte", |old| {
        let default = match old {
            ParamKind::Choice { default, .. } => default.clone(),
            _ => None,
        };
        ParamKind::choice(matte_options(), default)
    });
    // The picker opens on the document-encoded foreground, as the named matte it replaces would.
    let seed = indexed_matte_seed(&indexed_matte_defaults(app), "foreground").unwrap_or([1.0; 3]);
    let matte_color = Param::new(MATTE_COLOR, Label::from(tl!("Matte Color")), ParamKind::color(Some(seed))).visible_when("matte", "custom");
    params.insert(at + 1, matte_color);
}

/// Resolve a tool colour into the encoded samples used by Indexed Color's matte.
///
/// The picker and command fields carry document RGB coordinates. Tool swatches are
/// working-RGB coordinates, so only the named tool choices need this conversion here.
fn indexed_matte_document_color(app: &PhotocraftApp, doc: &Document, color: [f32; 4]) -> Result<[f32; 3], String> {
    let destination = match photocraft_engine::color_cmds::mode_space(doc.mode) {
        ColorSpace::Rgb | ColorSpace::Gray => photocraft_engine::color_cmds::document_profile(doc),
        _ => photocraft_engine::color_cmds::composite_profile(doc),
    };
    let source = app.session.color.working(ColorMode::Rgb);
    let options = TransformOptions { intent: app.session.color.settings.intent(), bpc: app.session.color.settings.bpc, ..Default::default() };
    let transform = craft::cms::transform::cached(&source, &destination, options).map_err(|e| e.to_string())?;
    let mut output = [0.0; 16];
    transform.eval(&color[..3], &mut output).map_err(|e| e.to_string())?;
    let rgb = if destination.color_space == ColorSpace::Gray { [output[0]; 3] } else { [output[0], output[1], output[2]] };
    if rgb.iter().any(|v| !v.is_finite()) {
        return Err("cannot convert matte colour to finite document samples".into());
    }
    Ok(rgb.map(|v| v.clamp(0.0, 1.0)))
}

/// The tool colours as the active document encodes them; `None` where the document cannot hold the colour.
pub(crate) struct MatteColors {
    foreground: Option<[f32; 3]>,
    background: Option<[f32; 3]>,
}

/// The foreground and background in the active document's encoding.
pub(crate) fn indexed_matte_defaults(app: &PhotocraftApp) -> MatteColors {
    let Some(doc) = app.session.active().map(|state| state.doc.as_ref()) else { return MatteColors { foreground: None, background: None } };
    let convert = |color| indexed_matte_document_color(app, doc, color).ok();
    MatteColors { foreground: convert(app.session.tools.foreground), background: convert(app.session.tools.background) }
}

/// The colour a matte choice or `#rrggbb` stands for, in the document's encoding.
pub(crate) fn indexed_matte_seed(colors: &MatteColors, value: &str) -> Option<[f32; 3]> {
    match value {
        "foreground" => colors.foreground,
        "background" => colors.background,
        "black" => Some([0.0; 3]),
        "gray" => Some([128.0 / 255.0; 3]),
        "netscapeGray" => Some([192.0 / 255.0; 3]),
        "none" | "white" => Some([1.0; 3]),
        _ => Srgba::from_hex(value).map(Srgba::to_rgb_array),
    }
}

// --- Lens Blur -------------------------------------------------------------------------------------------------

/// The active document's channel names, in index order.
fn channel_names(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().map(|d| d.doc.channels.iter().map(|c| c.name.clone()).collect()).unwrap_or_default()
}

/// Lens Blur's Source menu entry for the alpha channel at `index`. By index, not name: two
/// channels may share a name, and each must be choosable.
pub(crate) fn channel_choice(index: usize) -> String {
    format!("channel:{index}")
}

/// The channel index a Source menu entry names.
pub(crate) fn channel_of_choice(id: &str) -> Option<usize> {
    id.strip_prefix("channel:")?.parse().ok()
}

/// The alpha channel `v` names (an index, or a name: the first channel so named), as the
/// engine reads `depthChannel`.
pub(crate) fn channel_index(v: &Value, names: &[String]) -> Option<usize> {
    match v {
        Value::Number(n) => n.as_u64().and_then(|i| usize::try_from(i).ok()).filter(|i| *i < names.len()),
        Value::String(name) => names.iter().position(|n| n == name),
        _ => None,
    }
}

/// Lens Blur's Depth Map Source lists the document's alpha channels after None, Transparency
/// and Layer Mask, as in Photoshop (#7400).
fn lens_blur(app: &PhotocraftApp, params: &mut [Param]) {
    let names = channel_names(app);
    if let Some(depth_map) = row(params, "depthMap") {
        if let ParamKind::Choice { options, default, .. } = &mut depth_map.kind {
            options.retain(|(id, _)| id != "channel");
            options.extend(names.iter().enumerate().map(|(i, name)| (channel_choice(i), Label::from(name.clone()))));
            // A source that names a channel of one document is never carried to another, so the menu opens as None.
            *default = Some("none".into());
        }
        *depth_map = depth_map.clone().not_remembered();
    }
}

/// Opens Lens Blur's values on the menu's own ids: a `channel` source with its `depthChannel` (an index or a
/// name) becomes `channel:N`; one without a channel this document has (references aren't remembered) is None.
pub(crate) fn lens_blur_channels(app: &PhotocraftApp, fields: &mut DialogFields) {
    let names = channel_names(app);
    let depth = fields.remove("depthChannel").and_then(|v| serde_json::to_value(v).ok());
    let source = fields.text("depthMap").map(str::to_owned);
    let known = source.as_deref().and_then(channel_of_choice).filter(|i| *i < names.len());
    let chosen = match source.as_deref() {
        Some("channel") => depth.as_ref().and_then(|v| channel_index(v, &names)),
        _ => known,
    };
    if let Some(i) = chosen {
        fields.set("depthMap", channel_choice(i));
    } else if source.as_deref().is_some_and(|s| s == "channel" || channel_of_choice(s).is_some()) {
        fields.set("depthMap", "none");
    }
}

// --- Installed profiles ----------------------------------------------------------------------------------------

/// Schema dialogs whose `profile` menu lists the installed profiles after the built-ins.
const PROFILE_DIALOGS: &[&str] = &["edit.assignProfile", "edit.convertToProfile", "view.proofSetup"];

/// Which installed profiles a menu of `cmd` lists after its built-ins, if it lists any: every
/// destination profile for the `profile` menus of [`PROFILE_DIALOGS`], and for Color Settings'
/// working spaces those of that space, as Photoshop lists them (#6323). An RGB working space is a
/// display or colour-space profile, not a scanner's or a printer's.
fn installed_filter(cmd: &str, key: &str) -> Option<fn(&InstalledProfile) -> bool> {
    use craft::cms::ProfileClass;
    match (cmd, key) {
        (c, "profile") if PROFILE_DIALOGS.contains(&c) => Some(|_| true),
        ("edit.colorSettings", "workingRgb") => {
            Some(|i| i.color_space == ColorSpace::Rgb && matches!(i.class, ProfileClass::Display | ProfileClass::ColorSpace))
        }
        ("edit.colorSettings", "workingCmyk") => Some(|i| i.color_space == ColorSpace::Cmyk),
        ("edit.colorSettings", "workingGray") => Some(|i| i.color_space == ColorSpace::Gray),
        _ => None,
    }
}

/// The `installed` profiles a menu of `cmd` lists (see [`installed_filter`]), by path and description.
pub(super) fn installed_choices(cmd: &str, key: &str, installed: &[InstalledProfile]) -> Vec<(String, String)> {
    let Some(fits) = installed_filter(cmd, key) else { return Vec::new() };
    installed.iter().filter(|i| i.is_destination() && fits(i)).map(|i| (i.path.clone(), i.description.clone())).collect()
}

/// Like Photoshop, the profile menus also list the profiles installed in the system (printer and paper
/// profiles, press conditions), by description. Assign Profile's restriction to what the document's colour
/// mode can take (#4951) is dialogs-w24's `assign_profile_defaults`.
fn installed_profiles(command: &str, params: &mut [Param]) {
    if !params.iter().any(|p| installed_filter(command, &p.key).is_some()) {
        return;
    }
    let installed = photocraft_engine::installed_profiles::installed();
    for p in params.iter_mut() {
        let choices = installed_choices(command, &p.key, &installed);
        if let ParamKind::Choice { options, .. } = &mut p.kind {
            options.extend(choices.into_iter().map(|(path, description)| (path, Label::from(description))));
        }
    }
}

// --- Color Lookup ----------------------------------------------------------------------------------------------

/// The picker's choice for the values the row holds: a LUT loaded from a file, a folder file, or a built-in look.
fn lut_choice(fields: &DialogFields) -> LutChoice {
    if let Some(name) = fields.text("fileName").filter(|_| fields.text("data").is_some_and(|d| !d.is_empty())) {
        let size = fields.int(LUT_SIZE).and_then(|n| usize::try_from(n).ok()).unwrap_or(0);
        return LutChoice::loaded(name.strip_suffix(".cube").unwrap_or(name), size);
    }
    if let Some(file) = fields.text("file").filter(|f| !f.is_empty()) {
        return LutChoice::Folder(PathBuf::from(file));
    }
    let look = fields.text("lut").unwrap_or("none");
    photocraft_engine::adjust_cmds::LOOKS.iter().find(|(known, _)| *known == look).map_or(LutChoice::None, |(known, _)| LutChoice::Builtin(known))
}

/// Writes a picked choice as the command's `lut` / `file` parameters; the loaded LUT already held is no pick.
fn set_lut_choice(fields: &mut DialogFields, choice: &LutChoice) {
    let clear = |fields: &mut DialogFields, keys: &[&str]| keys.iter().for_each(|key| drop(fields.remove(key)));
    fields.remove(LUT_ERROR);
    match choice {
        LutChoice::None => {
            fields.set("lut", "none");
            clear(fields, &["file", "data", "fileName", LUT_SIZE]);
        }
        LutChoice::Builtin(id) => {
            fields.set("lut", *id);
            clear(fields, &["file", "data", "fileName", LUT_SIZE]);
        }
        LutChoice::Folder(path) => {
            fields.set("lut", "none");
            fields.set("file", path.to_string_lossy().into_owned());
            clear(fields, &["data", "fileName", LUT_SIZE]);
        }
        _ => {}
    }
}

/// Color Lookup's 3D LUT menu lists the LUT folder's files after the built-in looks (#9754), as the adjustment
/// layer's Properties do: one picker row over the command's `lut`, `file` and, for a LUT loaded from a file,
/// `data` and `fileName`.
fn color_lookup(app: &PhotocraftApp, params: &mut Vec<Param>) {
    let at = params.iter().position(|p| p.key == "lut").unwrap_or(params.len());
    params.retain(|p| !matches!(p.key.as_str(), "lut" | "file"));
    let folder = Some(app.session.prefs().file_handling.lut_folder.clone()).filter(|f| !f.is_empty()).map(PathBuf::from);
    // PLAN: `craft::cms` needs the umbrella's `cms` feature in crates/ui-egui/Cargo.toml (as `panels/adjust_ui.rs`).
    // The folder is read from the preferences here: `__lutFolder` cannot be a `DialogFields` key.
    let row = Param::custom("lut", Label::from(tl!("3D LUT File")), move |ui, fields, _| {
        let mut picker = LutPicker::new(Bind::given(lut_choice(fields)));
        if let Some(folder) = &folder {
            picker = picker.folder(folder.clone());
        }
        let out: LutPickerOutput = picker.show(ui);
        for event in &out.edit.events {
            if let EditEvent::Commit { after, .. } | EditEvent::Set { after, .. } = event {
                set_lut_choice(fields, after);
            }
        }
        match out.loaded {
            Some(Ok(lut)) => {
                set_lut_choice(fields, &LutChoice::None);
                fields.set("data", write_cube(&lut));
                fields.set("fileName", format!("{}.cube", lut.title));
                fields.set(LUT_SIZE, i64::try_from(lut.size).unwrap_or(0));
            }
            Some(Err(error)) => fields.set(LUT_ERROR, error.0),
            None => {}
        }
        if let Some(error) = fields.text(LUT_ERROR) {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        out.edit.response
    })
    .companions(["file", "data", "fileName"]);
    params.insert(at.min(params.len()), row);
}

// --- Match Color -----------------------------------------------------------------------------------------------

/// Match Color's source document and the source layer of it, from the documents open now. The layer row reads
/// the document `source` names, so it follows the menu.
fn match_color(app: &PhotocraftApp, params: &mut Vec<Param>) {
    let documents = app.session.documents();
    if !params.iter().any(|p| p.key == "source") {
        let mut options = vec![("-1".to_owned(), Label::from(tl!("None")))];
        options.extend(documents.iter().enumerate().map(|(i, st)| (i.to_string(), Label::from(st.doc.name.clone()))));
        params.push(Param::new("source", Label::from(tl!("Source")), ParamKind::choice(options, Some("-1".into()))));
    }
    // A layer belongs to the document that was chosen: another source returns the layer to Merged.
    if let Some(source) = row(params, "source") {
        *source = source.clone().resets(["sourceLayer"]);
    }
    let layers: Arc<Vec<Vec<(i64, String)>>> = Arc::new(
        documents
            .iter()
            .map(|st| st.doc.walk().into_iter().rev().map(|(_, _, layer)| (i64::try_from(layer.id.0).unwrap_or(-1), layer.name.clone())).collect())
            .collect(),
    );
    let label = Label::from(tl!("Source Layer"));
    let name = label.clone();
    params.retain(|p| p.key != "sourceLayer");
    let count = documents.len();
    params.push(
        Param::custom("sourceLayer", label, move |ui, fields, _| {
            let source = fields.text("source").map(str::to_owned);
            let listed = source.as_deref().and_then(|s| s.parse::<usize>().ok()).and_then(|i| layers.get(i));
            let shown = layer_ref(fields).filter(|id| listed.is_some_and(|l| l.iter().any(|(known, _)| known == id))).unwrap_or(-1);
            let mut entries = vec![Entry::new(-1i64, Label::from(tl!("Merged")))];
            entries.extend(listed.into_iter().flatten().map(|(id, layer)| Entry::new(*id, Label::from(layer.clone()))));
            let out = Select::new(name.clone(), Bind::given(shown), &entries).enabled(listed.is_some()).show(ui);
            match out.value().copied() {
                Some(-1) => drop(fields.remove("sourceLayer")),
                Some(id) => fields.set("sourceLayer", id),
                None => {}
            }
            out.response().clone()
        })
        .enabled_when_in("source", (0..count).map(|i| i.to_string())),
    );
}

/// The source layer a form holds: the row's own choice, or the id a caller set through the row's key.
fn layer_ref(fields: &DialogFields) -> Option<i64> {
    fields.int("sourceLayer")
}

// --- Replace Color ---------------------------------------------------------------------------------------------

/// Replace Color (#4143): the "From" colour starts as the foreground and is sampled from the image with the
/// canvas droppers; the Result swatch shows the colour after the shift.
fn replace_color(app: &PhotocraftApp, params: &mut Vec<Param>) {
    let fg = app.session.tools.foreground;
    retype(params, "color", |_| ParamKind::color(Some([fg[0], fg[1], fg[2]])));
    // Range only applies with Localized Color Clusters on, as in Photoshop.
    if let Some(range) = row(params, "range") {
        *range = range.clone().enabled_when("localized", true);
    }
    let at = params.iter().position(|p| p.key == "color").map_or(params.len(), |i| i + 1);
    params.insert(at, droppers_row());
    params.push(result_row());
}

/// The three droppers (#6280), as Photoshop's: Eyedropper, Add to Sample and Subtract from Sample. Clicking
/// the armed one puts it away. The dialog opens with the first armed (#6279), and Reset arms it again.
fn droppers_row() -> Param {
    Param::custom(SAMPLE_TOOL, Label::from(tl!("Eyedroppers")), |ui, fields, cx| {
        let armed = fields.text(SAMPLE_TOOL).unwrap_or_default().to_owned();
        // The body can run more than once in a pass and each run starts clean, so the request is filed on every run
        // while a dropper is armed: the host's arming is idempotent.
        if !armed.is_empty() {
            cx.arm_eyedropper(EyedropperRequest::new("color"));
        }
        let tools = [
            ("sample", Label::from(tl!("Pick Color From Image")), IconSpec::svg(IconId::new("eyedropper"))),
            ("add", Label::from(tl!("Add to Sample")), IconSpec::mark(CursorBadge::Add)),
            ("subtract", Label::from(tl!("Subtract from Sample")), IconSpec::mark(CursorBadge::Subtract)),
        ];
        let response = ui
            .horizontal(|ui| {
                for (id, label, icon) in tools {
                    let toggle = ToggleButton::new(label, Bind::given(armed == id)).icon(icon).icon_only().id_salt(id).show(ui);
                    match toggle.value().copied() {
                        Some(true) => {
                            fields.set(SAMPLE_TOOL, id);
                        }
                        Some(false) => fields.set(SAMPLE_TOOL, ""),
                        None => {}
                    }
                }
            })
            .response;
        cx.sampling(!fields.text(SAMPLE_TOOL).unwrap_or_default().is_empty());
        response
    })
    .opens_with("sample")
}

/// The sampled colour after the Hue, Saturation and Lightness shift.
fn result_color(fields: &DialogFields) -> [f32; 3] {
    use photocraft_algo::tone;
    let from = fields.color("color").unwrap_or([0.0; 3]);
    let number = |key: &str| fields.number(key).unwrap_or(0.0) as f32;
    tone::replace_color_result(from, number("hue"), number("saturation"), number("lightness")).map(|v| v.clamp(0.0, 1.0))
}

/// Picking a Result in the Color Picker sets Hue, Saturation and Lightness so the sampled colour becomes it (#6278).
/// The picker's answer is the only write of `resultPicked`, and the pass that reads it removes it.
fn apply_picked_result(fields: &mut DialogFields) {
    use photocraft_algo::tone;
    let Some(to) = fields.color(RESULT_PICKED) else { return };
    let from = fields.color("color").unwrap_or([0.0; 3]);
    let (hue, saturation, lightness) = tone::replace_color_shift(from, to);
    for (key, v) in [("hue", hue), ("saturation", saturation), ("lightness", lightness)] {
        fields.set(key, f64::from(v.round()));
    }
    fields.remove(RESULT_PICKED);
}

/// Replace Color's Result swatch (#6278): the sampled colour after the Hue, Saturation and Lightness shift.
fn result_row() -> Param {
    Param::custom("result", Label::from(tl!("Result")), |ui, fields, cx| {
        apply_picked_result(fields);
        let rgb = result_color(fields);
        let color = Srgba::from_rgb_array(rgb);
        let out = ui
            .horizontal(|ui| {
                let swatch = Swatch::new(Label::from(tl!("Result")), Mixed::Same(color)).picker(RESULT_PICKED).id_salt("result").show(ui);
                ui.monospace(color.to_hex());
                swatch
            })
            .inner;
        // The row writes nothing itself: the picker opens on the shown Result and answers into `resultPicked`.
        if out.pick_color.is_some() {
            cx.pick_color(RESULT_PICKED, color);
        }
        out.response().clone()
    })
}

/// The sample tool the dialog has armed; `None` when none is.
fn armed_tool(fields: &DialogFields) -> Option<SampleTool> {
    match fields.text(SAMPLE_TOOL)? {
        "sample" => Some(SampleTool::Sample),
        "add" => Some(SampleTool::Add),
        "subtract" => Some(SampleTool::Subtract),
        _ => None,
    }
}

/// The points a sampling key holds, as the command reads them.
fn points_at(fields: &DialogFields, key: &str) -> Vec<[f64; 2]> {
    fields.get(key).and_then(|v| serde_json::to_value(v).ok()).and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
}

/// The samples the dialog holds: `points`, `subtractPoints` and their click `order`.
fn samples_of(fields: &DialogFields) -> ColorSamples {
    let order: Vec<SampleSign> =
        fields.get("order").and_then(|v| serde_json::to_value(v).ok()).and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
    let mut samples = ColorSamples::default();
    samples.add = points_at(fields, "points");
    samples.subtract = points_at(fields, "subtractPoints");
    samples.order = order;
    samples
}

/// Sample the merged image at document point `at` into Replace Color's "From" colour.
/// Returns true when the press was consumed, even off-image or over transparency, where nothing
/// changes and the dropper stays armed (like the Curves picker). `false` when `id` is no Replace Color with a dropper armed.
/// With the Add to Sample dropper (or Shift) the press adds a sample, with Subtract from
/// Sample (or Alt) it subtracts one, as in Photoshop and Select › Color Range (#6280); the one rule is
/// [`ColorSamples::pick`].
pub(crate) fn replace_color_sample(app: &mut PhotocraftApp, id: DialogId, at: [f64; 2], mods: Mods) -> bool {
    let Some((tool, mut samples)) = app
        .framework
        .dialogs
        .get_by_id::<HostedParamDialog>(id)
        .filter(|d| d.key() == Some(REPLACE_COLOR))
        .and_then(|d| Some((armed_tool(d.fields())?, samples_of(d.fields()))))
    else {
        return false;
    };
    let Some(rgb) = crate::canvas::composite_color(app, at[0], at[1]) else { return true };
    let Some(size) = app.session.active().map(|st| [st.doc.size.width, st.doc.size.height]) else { return true };
    // A first Add to Sample, before any sample, starts from the clicked colour too: one point matches through
    // `color`, and Color Range's matcher starts its range there as well.
    let effective = tool.with_mods(mods);
    let sets_color = effective == SampleTool::Sample || (effective == SampleTool::Add && samples.add.is_empty());
    // Where it was picked: Localized Color Clusters match the colour near this point (#4457); added and
    // subtracted samples go to `points` / `subtractPoints` in click order.
    samples.pick(at, size, tool, mods);
    let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedParamDialog>(id) else { return true };
    // PLAN: the sheet names `set_json_field`; these keys have no row, which `set_json_field` refuses, so the typed `set_field` (dl.K70) writes them.
    dialog.set_field("points", FieldValue::Json(json!(samples.add)));
    dialog.set_field("subtractPoints", FieldValue::Json(json!(samples.subtract)));
    dialog.set_field("order", FieldValue::Json(json!(samples.order)));
    if sets_color {
        dialog.answer_eyedropper("color", Srgba::from_rgb_array(rgb));
    }
    true
}

// --- Radial Blur -----------------------------------------------------------------------------------------------

/// Radial Blur's centre is one point row, shown for the zoom method. `centerX` and `centerY` stay the command's
/// keys; the family splits the point back into them.
fn radial_blur(params: &mut Vec<Param>) {
    let at = params.iter().position(|p| p.key == "centerX").unwrap_or(params.len());
    params.retain(|p| !matches!(p.key.as_str(), "centerX" | "centerY"));
    // PLAN: a point row is in percent of the image (param.rs `ParamKind::Point`), the sheet's default [0.5, 0.5]
    // is a fraction: the row opens at the middle, [50, 50], and `centerX`/`centerY` are fractions (dialogs-w24 scales).
    let centre = Param::new("center", Label::from(tl!("Center")), ParamKind::point([50.0, 50.0])).visible_when("method", "zoom");
    params.insert(at.min(params.len()), centre);
}
