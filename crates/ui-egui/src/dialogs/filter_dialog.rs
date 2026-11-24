//! Filter and command parameter dialogs, generated from engine command parameter specs, with live
//! on-canvas preview and an in-dialog preview window (#3769).
//!
//! The dialogs are `HostedParamDialog`s opened through `dialogs::params::open_param`: the framework
//! draws the rows from [`params`], keeps the values, remembers them and answers OK as a
//! `ParamResult`, which [`run_command`] runs.
//!
//! The preview runs the *same engine command* on the downsampled proxy document (pixel-sized
//! parameters scaled by the proxy factor), so what you preview is what you get.

use photocraft_engine::color_cmds::ColorExt;
use std::sync::Arc;

use craft::behavior::NumberCodec;
use craft::host::{PickRequest, Picked};
use craft::ui::contract::{Bind, Show as _};
use craft::ui::dialog::{DialogId, dialog_id};
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind, ParamLayout, ParamResult};
use craft::ui::runtime::env::{Env, Label, UnitKind};
use craft::ui::runtime::requests::PathKind;
use craft::ui::text::NumberField;
use photocraft_doc::Document;
use photocraft_engine::color_cmds::ColorSettings;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;

mod color_settings_presets;
mod rows;
pub(crate) use color_settings_presets::reconcile as reconcile_color_settings;
pub(crate) use rows::{REPLACE_COLOR, replace_color_sample};

const MATCH_COLOR: &str = "image.adjustments.matchColor";
const SCALE_EFFECTS: &str = "layer.layerStyle.scaleEffects";
#[cfg(test)]
pub(crate) const MATCH_COLOR_DESTINATION: &str = "__matchColorDestination";
/// The key of the document id Lens Blur captures at open (see [`validate_lens_blur_document`]).
const LENS_BLUR_DOCUMENT: &str = "lensBlurDocument";

/// The decimals of the Rotate Canvas angle: it keeps hundredths of a degree, so the ruler's
/// straightening angle reaches the dialog as 33.69 and not 33.7 (#12035).
const ARBITRARY_ANGLE_DECIMALS: u32 = 2;

/// Parameter keys measured in pixels (scaled for proxy previews).
fn is_pixel_param(key: &str) -> bool {
    matches!(
        key,
        "radius"
            | "distance"
            | "cellSize"
            | "horizontal"
            | "vertical"
            | "height"
            | "wavelengthMin"
            | "wavelengthMax"
            | "amplitudeMin"
            | "amplitudeMax"
            | "maxRadius"
            | "size"
            | "blur"
            | "speed"
            | "shadowRadius"
            | "highlightRadius"
    )
}

/// [`is_pixel_param`] for `key` of `command`. Some commands use those names for relative values:
/// Tree's and Picture Frame's `size` (a fraction of the canvas, an ornament scale) and Lens
/// Correction's perspective `horizontal` / `vertical` (-100..100), and Lighting Effects' light
/// `radius` (a fraction of the canvas). Scaled by the proxy factor,
/// their preview drew a quarter-size tree and a quarter of the perspective (#2063).
fn is_pixel_param_of(command: &str, key: &str) -> bool {
    is_pixel_param(key)
        && !matches!(
            (command, key),
            ("filter.render.tree" | "filter.render.pictureFrame", "size")
                | ("filter.lensCorrection", "horizontal" | "vertical")
                | ("filter.render.lightingEffects", "radius")
        )
}

/// Filters whose features have a fixed size in document pixels that no dialog parameter scales:
/// per-pixel noise and dots (Add Noise, Diffuse, Mezzotint, Fibers, and Dust & Scratches, whose
/// specks a proxy averages away — so the preview would show nothing removed at all, #3265 — while
/// its radius stays in document pixels), pixel kernels and scan lines
/// (Custom, Reduce Noise, De-Interlace, Trace Contour's one-pixel lines), fixed lengths and
/// widths (Ripple, Oil Paint, Wind's streaks, Extrude's depth, Flame's length, width and
/// interval) and Tree, whose branches are built from 6 px segments. On a reduced proxy those
/// features come out k times too large (or a different tree grows), so these preview at full
/// resolution: what the preview shows is what OK applies, as in Photoshop (#2063).
///
/// Sharpen, Sharpen More, Sharpen Edges, Find Edges, Emboss and Despeckle use fixed one-pixel
/// kernels with no size parameter, so a proxy changes their look: edges average away, making
/// the sharpening and edge detection invisible. These must preview at full resolution (#3929).
///
/// Content-Aware Fill searches for matching patches, and the matches found on a reduced copy
/// differ from those found at full size, so its preview would not show what OK writes (#6530).
const FULL_RESOLUTION_PREVIEW: &[&str] = &[
    "filter.distort.ripple",
    "filter.stylize.diffuse",
    "filter.stylize.oilPaint",
    "filter.render.flame",
    "filter.render.fibers",
    "filter.pixelate.mezzotint",
    "filter.noise.addNoise",
    "filter.noise.dustAndScratches",
    "filter.noise.reduceNoise",
    "filter.other.custom",
    "filter.video.deInterlace",
    "filter.stylize.traceContour",
    "filter.stylize.wind",
    "filter.stylize.extrude",
    "filter.render.tree",
    "filter.sharpen.sharpen",
    "filter.sharpen.sharpenMore",
    "filter.sharpen.sharpenEdges",
    "filter.stylize.findEdges",
    "filter.stylize.emboss",
    "filter.noise.despeckle",
    "edit.contentAwareFill",
];

/// Whether `command` with `params` previews at full resolution: [`FULL_RESOLUTION_PREVIEW`], every
/// Filter Gallery filter (its strokes, grain and textures are sized in pixels; the settings are
/// abstract levels, not lengths) and the dithered colour-mode conversions (Indexed Color, Bitmap).
fn full_resolution_preview(command: &str, params: &Value) -> bool {
    let choice = |key: &str| params.get(key).and_then(Value::as_str);
    FULL_RESOLUTION_PREVIEW.contains(&command)
        || command.starts_with("filter.gallery.")
        || (command == "image.mode.indexedColor" && choice("dither") != Some("none"))
        || (command == "image.mode.bitmap" && !matches!(choice("method"), Some("threshold" | "threshold50")))
}

/// Proxy factor for a live preview of `command` with `params`, given the document's reduced factor
/// `k`: 1 for [`full_resolution_preview`] commands, otherwise the largest factor up to `k` at which
/// no pixel-sized parameter, divided by it, drops below the command's minimum. A clamped value
/// would change the feature size: Pointillize's 5 px cells at k = 4 became the 3 px minimum, i.e.
/// 12 px cells in the preview (#2063).
pub fn preview_factor(command: &str, params: &Value, k: u32) -> u32 {
    if k <= 1 || full_resolution_preview(command, params) {
        return 1;
    }
    // A plug-in's pixel lengths are the params its manifest marks `"unit": "px"` (#7524).
    if command == photocraft_engine::plugin_cmds::RUN {
        return photocraft_engine::plugin_cmds::preview_factor(params, k);
    }
    let Some(spec) = photocraft_engine::commands::find(command) else { return k };
    let k = Param::parse_list(spec.params).into_iter().fold(k, |k, p| match p.kind {
        ParamKind::Range { min, .. } if min > 0.0 && is_pixel_param_of(command, &p.key) => match params.get(&p.key).and_then(Value::as_f64) {
            Some(v) if v.is_finite() => k.min((v / min).floor().clamp(1.0, f64::from(k)) as u32),
            _ => k,
        },
        _ => k,
    });
    // Minimum and Maximum measure the radius in whole pixels (Squareness rounds it, Roundness
    // floors it, and 0 is the identity), so at k = 4 a typed 1.5 px became 0.375 = 0 proxy px and
    // the preview showed the untouched document while OK applied a real effect (#10474). Keep at
    // least one proxy pixel of radius, and the largest k whose whole-pixel proxy radius, scaled
    // back, is within 1 px of the one OK applies: Squareness at 6 px and k = 4 rounds 1.5 to 2 proxy
    // px (8 px), and Roundness's disc reaches floor(1.5) = 1 proxy px along an axis (4 px).
    // A large radius still previews on a proxy: at full size Roundness costs O(radius) per pixel.
    if matches!(command, "filter.other.minimum" | "filter.other.maximum") {
        let Some(r) = params.get("radius").and_then(Value::as_f64).filter(|r| r.is_finite()) else { return k };
        let r = r.clamp(0.2, 500.0);
        let whole: fn(f64) -> f64 = if params.get("preserve").and_then(Value::as_str) == Some("roundness") { f64::floor } else { f64::round };
        let fits = |d: u32| {
            let d = f64::from(d);
            d <= r.floor() && (whole(r / d) * d - whole(r)).abs() <= 1.0
        };
        return (1..=k).rev().find(|&d| fits(d)).unwrap_or(1);
    }
    // Mosaic rounds its cells to whole pixels: 10 px cells at k = 4 became 3 proxy px = 12 px.
    // Divide the cell size evenly instead.
    match params.get("cellSize").and_then(Value::as_f64) {
        Some(v) if command == "filter.pixelate.mosaic" && v.is_finite() && v >= 1.0 => {
            let cell = v.round().min(f64::from(u32::MAX)) as u32;
            (1..=k).rev().find(|d| cell.is_multiple_of(*d)).unwrap_or(1)
        }
        _ => k,
    }
}

/// Commands outside `filter.*` that get the schema dialog *with* live preview.
pub const PREVIEWED: &[&str] = &[
    "image.adjustments.colorLookup",
    "image.adjustments.shadowsHighlights",
    "image.adjustments.replaceColor",
    MATCH_COLOR,
    "image.adjustments.hdrToning",
    "image.mode.indexedColor",
    "image.mode.bitmap",
    "image.mode.duotone",
    "layer.matting.defringe",
    "layer.matting.colorDecontaminate",
    "layer.layerStyle.scaleEffects",
    "type.warpText",
    // The preview shows the image through the chosen profile, as Photoshop's Preview does.
    "edit.assignProfile",
    "edit.convertToProfile",
    // Previewed from the session's history (`canvas::filter_preview`), at full size (#8230).
    "edit.fade",
];

/// Whether the dialog of `command` offers the Preview box and a live preview.
pub(crate) fn previews(command: &str) -> bool {
    command.starts_with("filter.") || PREVIEWED.contains(&command)
}

/// Whether `command` opens a schema dialog.
pub fn has_dialog(command: &str) -> bool {
    // The Filter Gallery has its own full-window dialog (gallery_ui).
    if command == "filter.filterGallery" {
        return false;
    }
    (command.starts_with("filter.")
        || command.starts_with("select.modify.")
        || PREVIEWED.contains(&command)
        || matches!(
            command,
            "image.trim"
                // Rotate Canvas has no preview, as in the reference application (#14462).
                | "image.rotation.arbitrary"
                | "view.proofSetup"
                | "layer.layerStyle.globalLight"
                | "image.mode.colorTable"
                | "edit.definePattern"
                | "select.convertToShape"
                | "layer.new.artboardFromLayers"
        ))
        && photocraft_engine::commands::find(command).is_some_and(|c| !Param::parse_list(c.params).is_empty())
}

/// Whether a result of the dialog `key` is this family's: any engine command is run with the
/// dialog's values. The family is listed after every family with its own handler.
pub(crate) fn owns(key: &str) -> bool {
    photocraft_engine::commands::find(key).is_some()
}

/// The byte offsets of the `:doc` markers of `spec`: parameters that name an open document.
fn document_marks(spec: &str) -> Vec<usize> {
    spec.match_indices(":doc").map(|(at, _)| at).filter(|at| !spec[at + 4..].starts_with(|c: char| c.is_alphanumeric())).collect()
}

/// The keys of `spec`'s document parameters (one of the open documents, stored as its index).
fn document_keys(spec: &str) -> Vec<String> {
    document_marks(spec)
        .into_iter()
        .filter_map(|at| {
            let head = spec[..at].trim_end().strip_suffix('"')?;
            let start = head.rfind('"')?;
            Some(head[start + 1..].to_owned())
        })
        .collect()
}

/// `spec` with each document parameter as free text, so the framework's parser keeps its row, its
/// place and its label.
fn documents_as_text(spec: &str) -> String {
    let mut out = String::with_capacity(spec.len());
    let mut last = 0;
    for at in document_marks(spec) {
        out.push_str(&spec[last..at]);
        out.push_str(":text");
        last = at + 4;
    }
    out.push_str(&spec[last..]);
    out
}

/// Whether a remembered value of `key` would override what the open document supplies: those rows
/// opt out of remembering so the document's value wins.
fn document_derived(command: &str, key: &str) -> bool {
    matches!(
        (command, key),
        ("image.rotation.arbitrary", "angle" | "direction")
            | ("layer.layerStyle.globalLight", "angle" | "altitude")
            | ("edit.definePattern", "name")
            | ("layer.new.artboardFromLayers", "name" | "width" | "height")
    ) || command == "type.warpText"
        || command == "view.proofSetup"
}

/// The rows of `command`'s dialog for the parameter notation `spec`: the framework's parse of it,
/// then the app's rows (documents, angle dials, units, the rows only the open document can fill).
pub(crate) fn params(app: &mut PhotocraftApp, command: &str, spec: &str) -> Vec<Param> {
    let documents = document_keys(spec);
    let names: Vec<String> = app.session.documents().iter().map(|d| d.doc.name.clone()).collect();
    let allowed = (command == "edit.assignProfile").then(|| {
        let mut listed = Map::new();
        assign_profile_defaults(app, &mut listed);
        listed.get("__profiles").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect::<Vec<_>>()
    });
    let installed = photocraft_engine::installed_profiles::installed();
    let mut list: Vec<Param> = Param::parse_list(&documents_as_text(spec))
        .into_iter()
        // Optional parameters the generic dialog can't express keep their defaults.
        .filter(|p| {
            !(matches!(
                command,
                "edit.fade" | "edit.findAndReplaceText" | "edit.defineBrushPreset" | "edit.defineCustomShape" | "edit.autoAlignLayers" | "edit.autoBlendLayers"
            ) && matches!(p.key.as_str(), "reference" | "path"))
        })
        .map(|mut p| {
            // Edit › Fade's Mode is the blend list with the modes the document can't fade with greyed (#13852).
            if command == super::fade_modes::COMMAND && p.key == "mode" {
                return super::fade_modes::param(app, p.label);
            }
            if documents.contains(&p.key) {
                let mut options = vec![("-1".to_owned(), Label::from(tl!("None").to_string()))];
                options.extend(names.iter().enumerate().map(|(i, name)| (i.to_string(), Label::from(name.clone()))));
                return Param::new(p.key, p.label, ParamKind::choice(options, Some("-1".into()))).not_remembered();
            }
            if uses_angle_dial(command, &p.key)
                && let ParamKind::Range { min, max, default, .. } = p.kind
            {
                // Global Light's angle runs -360 to 360 in whole degrees; every other dial is one turn.
                let kind = if command == "layer.layerStyle.globalLight" { ParamKind::angle_in(min, max, default, 0) } else { ParamKind::angle(default) };
                p = Param::new(p.key, p.label, kind);
            }
            if command == SCALE_EFFECTS && p.key == "scale" {
                return scale_effects_row(p);
            }
            if let ParamKind::Range { max, .. } = p.kind {
                if is_pixel_param_of(command, &p.key) {
                    p = p.unit(UnitKind::Pixels);
                } else if p.key == "angle" {
                    p = p.unit(UnitKind::Degrees);
                } else if p.key == "amount" && max <= 500.0 {
                    p = p.unit(UnitKind::Percent);
                }
            }
            if let ParamKind::Choice { options, .. } = &mut p.kind {
                // The profile menus also list the profiles installed in the system, by description.
                options.extend(rows::installed_choices(command, &p.key, &installed).into_iter().map(|(path, description)| (path, Label::from(description))));
                // Assign Profile: only what the document's colour mode can take (#4951).
                if p.key == "profile"
                    && let Some(allowed) = &allowed
                {
                    options.retain(|(id, _)| allowed.contains(id));
                }
            }
            // Never retain document ids, free-form paths or raw JSON in the remembered values.
            if matches!(p.kind, ParamKind::Text { .. }) {
                p = p.not_remembered();
            }
            if document_derived(command, &p.key) {
                p = p.not_remembered();
            }
            // Global Light's Altitude takes whole degrees, as its Angle does (#14980).
            if command == "layer.layerStyle.globalLight" && p.key == "altitude" {
                p = p.decimals(0);
            }
            if command == "image.rotation.arbitrary" && p.key == "angle" {
                p = p.decimals(ARBITRARY_ANGLE_DECIMALS);
            }
            if p.key == "seed" && matches!(p.kind, ParamKind::Int { .. }) {
                p = p.randomize();
            }
            // Install Plug-in…: pick the .wasm file instead of typing its path (#7531).
            if command == "plugin.install" && p.key == "path" {
                p = p.browse(PathKind::open_file(&["wasm"]));
            }
            p
        })
        .collect();
    if command == crate::bitmap_dialog::COMMAND {
        list = crate::bitmap_dialog::params(app);
    }
    if command == "image.mode.colorTable" {
        list.extend(crate::color_table_ui::rows(app));
    }
    // Rotate Canvas: one row of its own, without the Interpolation row (#14462).
    if command == super::rotate_canvas_ui::COMMAND {
        list = super::rotate_canvas_ui::params();
    }
    rows::custom_rows(app, command, &mut list);
    if command == color_settings_presets::COMMAND {
        // The Settings menu: Custom, a greyed group label and the named setups in groups (#14229).
        if let Some(p) = list.iter_mut().find(|p| p.key == "preset") {
            *p = color_settings_presets::row(Label::from(tl!("Settings")));
        }
        // The monitor line under the rows: the cell holds the text each frame refreshes, which the build closure cannot read from the app.
        let note = crate::monitor_status::note_cell(app);
        list.push(craft::ui::patterns::form_dialogs::Param::custom("monitorNote", craft::ui::runtime::env::Label::Empty, move |ui, _, _| {
            let text = note.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone();
            ui.label(text)
        }));
    }
    list
}

/// Scale Layer Effects' row, as in Photoshop (#14996): a whole-percent field whose arrow opens a linear slider over
/// the range, where the schema's range row would draw an always-shown logarithmic slider for 1..1000.
fn scale_effects_row(p: Param) -> Param {
    let ParamKind::Range { min, max, default, .. } = p.kind else { return p };
    let label = p.label;
    Param::custom("scale", label.clone(), move |ui, fields, _| {
        let value = fields.number("scale").unwrap_or(default);
        let codec = NumberCodec::new(Env::of(ui).units().reader(None), 0);
        let shown = NumberField::new(label.clone(), Bind::given(Some(value)))
            .codec(codec)
            .range(min..=max)
            .step(1.0)
            .commit_decimals(0)
            .unit("%")
            .popup_slider(min..=max)
            .show(ui);
        if let Some(v) = shown.edit.value().copied().flatten() {
            fields.set("scale", v.round());
        }
        shown.edit.response
    })
    .opens_with(default)
}

/// The values `command`'s dialog opens with before the remembered ones: the open document's, the
/// session's and the captured editing target. Each is the dialog's own opening state, so the Alt
/// reset restores them too (through [`reset_values`]).
pub(crate) fn fields(app: &PhotocraftApp, command: &str, params: &[Param]) -> DialogFields {
    let mut fields = DialogFields::new();
    overlay(params, &command_defaults(app, command, params), &mut fields);
    // Replace Color starts from the foreground colour.
    if command == rows::REPLACE_COLOR {
        let fg = app.session.tools.foreground;
        fields.set("color", FieldValue::Color([fg[0], fg[1], fg[2]]));
    }
    if let Some(target) = captured_target(app, command, false) {
        fields.set("target", FieldValue::Json(target));
    }
    if command == rows::LENS_BLUR {
        rows::lens_blur_channels(app, &mut fields);
    }
    if command == crate::bitmap_dialog::COMMAND {
        crate::bitmap_dialog::defaults(app, &mut fields);
    }
    if command == super::rotate_canvas_ui::COMMAND {
        super::rotate_canvas_ui::open_values(&mut fields);
    }
    fields
}

/// Command-specific open defaults (ruler straighten, warp from layer, …), as JSON by parameter key.
fn command_defaults(app: &PhotocraftApp, command: &str, params: &[Param]) -> Map<String, Value> {
    let mut map = Map::new();
    if command == "image.rotation.arbitrary" {
        straighten_defaults(app, &mut map);
        // #14465: the reference dialog offers no interpolation choice, and the dialog has no row
        // for it (#14462): it sends none, so the command resamples as Preferences › General ›
        // Image Interpolation says on every run.
    }
    if command == "type.warpText" {
        warp_defaults(app, &mut map);
    }
    // Photoshop's Pattern Name dialog starts from the name the pattern would get anyway.
    if command == "edit.definePattern" {
        map.insert("name".into(), json!(photocraft_engine::pattern_cmds::default_name(&app.session)));
    }
    // The reference application's Artboard from Layers opens at the next Artboard N and the
    // selected layers' size every time, never a remembered size; past the field's 30000 px
    // range, 0 keeps the bounds.
    if command == "layer.new.artboardFromLayers"
        && let Some((name, [w, h])) = photocraft_engine::artboard_cmds::from_layers_defaults(&app.session)
    {
        let fits = w <= 30_000 && h <= 30_000;
        map.insert("name".into(), json!(name));
        map.insert("width".into(), json!(if fits { w } else { 0 }));
        map.insert("height".into(), json!(if fits { h } else { 0 }));
    }
    if command == "layer.layerStyle.globalLight" {
        global_light_defaults(app, &mut map);
    }
    if command == "view.proofSetup" {
        proof_setup_defaults(app, &mut map);
    }
    if command == "edit.colorSettings"
        && let Ok(Value::Object(current)) = serde_json::to_value(&app.session.color().settings)
    {
        map.extend(current.into_iter().filter(|(key, _)| params.iter().any(|p| p.key == *key)));
    }
    map
}

/// The editing target the dialog captures when it opens, as the command's `target` parameter: the
/// targeted filter mask or layer mask, else the selected colour channels. A spec dialog (a plug-in,
/// a frame) takes a colour target only for several channels.
fn captured_target(app: &PhotocraftApp, command: &str, spec_dialog: bool) -> Option<Value> {
    let follows = photocraft_engine::channel_cmds::follows_target(command);
    if app.filter_mask_targeted() && photocraft_engine::channel_cmds::follows_filter_mask(command) && (follows || spec_dialog) {
        Some(json!("filterMask"))
    } else if app.layer_mask_targeted() && follows {
        Some(json!("mask"))
    } else if follows || spec_dialog {
        photocraft_engine::color_channels::selected(&app.session).filter(|c| !spec_dialog || c.len() > 1).map(|c| c.params())
    } else {
        None
    }
}

/// `value` as the field of `param`, when the row can hold it.
fn field_for(param: &Param, value: &Value) -> Option<FieldValue> {
    let field = match &param.kind {
        ParamKind::Color { .. } => FieldValue::Color(crate::color_picker_ui::parse_hex(value.as_str()?)?),
        ParamKind::Bool { .. } => FieldValue::Bool(value.as_bool()?),
        ParamKind::Int { .. } => FieldValue::Int(value.as_i64()?),
        ParamKind::Range { .. } | ParamKind::Number { .. } | ParamKind::Angle { .. } | ParamKind::OptionalNumber { .. } => FieldValue::Number(value.as_f64()?),
        ParamKind::Choice { .. } | ParamKind::Radio { .. } | ParamKind::Text { .. } | ParamKind::Pattern { .. } => {
            FieldValue::Text(value.as_str().map(str::to_owned).or_else(|| value.as_i64().map(|i| i.to_string()))?)
        }
        ParamKind::Grid { .. } => FieldValue::Grid(value.as_array()?.iter().map(Value::as_i64).collect::<Option<Vec<_>>>()?),
        ParamKind::Point { .. } => {
            let point = value.as_array()?;
            FieldValue::Point([point.first()?.as_f64()?, point.get(1)?.as_f64()?])
        }
        _ => FieldValue::Json(value.clone()),
    };
    param.accepts(&param.key, &field).then_some(field)
}

/// Writes the JSON `values` into `fields`: a key with a row as that row's field, any other key
/// (a plug-in id, the formatting a warp edits) as it is, so it reaches the command.
fn overlay(params: &[Param], values: &Map<String, Value>, fields: &mut DialogFields) {
    for (key, value) in values {
        let field = match params.iter().find(|p| p.key == *key) {
            Some(param) => field_for(param, value),
            None => Some(FieldValue::Json(value.clone())),
        };
        if let Some(field) = field {
            fields.set(key.clone(), field);
        }
    }
}

/// The values Alt+Cancel restores: the schema defaults, then the command's own defaults, never the
/// last-used values.
fn reset_values(params: &[Param], opening: &DialogFields) -> DialogFields {
    let mut reset = DialogFields::new();
    for p in params {
        if let Some(value) = p.kind.default_value() {
            reset.set(p.key.clone(), value);
        }
    }
    for (key, value) in opening.iter() {
        reset.set(key, value.clone());
    }
    reset
}

/// The dialog's title: the command's label without its ellipsis.
pub(crate) fn title_of(label: &str) -> Label {
    Label::from(crate::i18n::t(label).trim_end_matches('…').to_string())
}

/// The title of `command`'s dialog: [`title_of`] its label, or the dialog's own name where the reference application
/// gives it one (Arbitrary opens "Rotate Canvas", #14462).
fn dialog_title(command: &str, label: &str) -> Label {
    if command == super::rotate_canvas_ui::COMMAND { super::rotate_canvas_ui::title() } else { title_of(label) }
}

/// The build closure of a command dialog over its values, over the active document's canvas: its droppers, the Blur
/// Gallery's pins and the preview pane's locator take presses there.
fn builder(
    app: &PhotocraftApp,
    command: &str,
    title: Label,
    params: Vec<Param>,
    reset: DialogFields,
    preview: bool,
) -> impl for<'b> Fn(&'b mut DialogFields) -> ParamDialog<'b> + Send + 'static {
    let key = command.to_owned();
    let canvas = app.session.active_index().map(|idx| crate::canvas::document_display::CanvasHost::primary(app, idx).canvas_id());
    move |fields: &mut DialogFields| {
        let mut dialog = ParamDialog::command(key.clone(), title.clone(), params.clone())
            .values(Bind::bound(fields))
            .reset_to(reset.clone())
            .layout(ParamLayout::Stacked)
            .previews(preview)
            // Alt keeps Rotate Canvas's Cancel a Cancel, as in the reference application (#14462).
            .alt_resets(key != super::rotate_canvas_ui::COMMAND)
            .alert_title(tl!("PhotoCraft"));
        if let Some(canvas) = canvas {
            dialog = dialog.canvas(canvas);
        }
        if key == "image.mode.colorTable" {
            dialog = dialog
                .extra_buttons(vec![("load".to_owned(), Label::from(tl!("Load…").to_string())), ("save".to_owned(), Label::from(tl!("Save…").to_string()))]);
        }
        dialog
    }
}

/// Opens `command`'s schema dialog using the current settings and editing target. The dialog's key
/// is the command; Match Color captures its destination document and layer.
pub fn open(app: &mut PhotocraftApp, command: &str) -> Result<DialogId, String> {
    open_as(app, command, previews(command))
}

/// [`open`] with the Preview box whether or not `command`'s dialog has one. Rotate Canvas has none (#14462), but the
/// canvas still places a preview that changes the document's size (#1436); its tests reach that path through here.
#[doc(hidden)]
pub fn open_previewing(app: &mut PhotocraftApp, command: &str) -> Result<DialogId, String> {
    open_as(app, command, true)
}

fn open_as(app: &mut PhotocraftApp, command: &str, preview: bool) -> Result<DialogId, String> {
    let spec = photocraft_engine::commands::find(command).ok_or_else(|| format!("{command} has no dialog"))?;
    let params = params(app, command, spec.params);
    let fields = fields(app, command, &params);
    let captured = if command == MATCH_COLOR {
        json!(app.session.active().map(|st| (st.doc.id, st.active_layer)))
    } else if command == rows::LENS_BLUR {
        // The Depth Map Source lists this document's alpha channels by index.
        json!({ LENS_BLUR_DOCUMENT: app.session.active().map(|st| st.doc.id) })
    } else {
        Value::Null
    };
    let reset = reset_values(&params, &fields);
    // Photoshop titles the Scale Effects… dialog with the longer name (#14996).
    let title = if command == SCALE_EFFECTS { Label::from(tl!("Scale Layer Effects")) } else { dialog_title(command, spec.label) };
    let build = builder(app, command, title, params, reset, preview);
    let id = crate::dialogs::params::open_param(app, command, fields, captured, build);
    if command == MATCH_COLOR {
        reconcile_match_color(app);
    }
    Ok(id)
}

/// A filter dialog for `command` whose parameters follow `spec` (registry notation) instead of the
/// command's own; `fixed` params (e.g. a plug-in id) are passed through. It remembers nothing.
pub fn open_with_spec(app: &mut PhotocraftApp, command: &str, label: &str, spec: &str, fixed: Map<String, Value>) -> DialogId {
    let params = params(app, command, spec);
    let mut fields = DialogFields::new();
    overlay(&params, &fixed, &mut fields);
    if !fields.contains("target")
        && let Some(target) = captured_target(app, command, true)
    {
        fields.set("target", FieldValue::Json(target));
    }
    let reset = reset_values(&params, &fields);
    let build = builder(app, command, title_of(label), params, reset, previews(command));
    // PLAN: assumed `open_param_unsaved(app, key, fields, captured, build) -> DialogId` (dialogs-w19): `open_param` without the remember key.
    crate::dialogs::params::open_param_unsaved(app, command, fields, Value::Null, build)
}

/// Opens `command`'s dialog to edit a smart filter: the schema defaults overlaid by the `recorded`
/// parameters (a key the filter never recorded keeps its default), with the smart-filter `target`
/// `{layer, index, guard}` as the dialog's captured value. It remembers nothing.
pub fn open_smart_edit(app: &mut PhotocraftApp, command: &str, recorded: &Map<String, Value>, target: Value) -> Result<DialogId, String> {
    let spec = photocraft_engine::commands::find(command).ok_or_else(|| format!("{command} has no dialog"))?;
    let params = params(app, command, spec.params);
    let base = fields(app, command, &params);
    let reset = reset_values(&params, &base);
    let mut opening = base;
    overlay(&params, recorded, &mut opening);
    let build = builder(app, command, dialog_title(command, spec.label), params, reset, previews(command));
    // PLAN: assumed `open_param_unsaved`, as in `open_with_spec`.
    Ok(crate::dialogs::params::open_param_unsaved(app, command, opening, target, build))
}

/// The command parameters of the dialog values `fields`: the plain map, colours as `#rrggbb`,
/// document choices as indices, and Radial Blur's centre as `centerX` / `centerY`. The preview and
/// OK both build their parameters here, so they cannot differ.
pub(crate) fn command_params(command: &str, fields: &DialogFields) -> Value {
    let mut map = match serde_json::to_value(fields) {
        Ok(Value::Object(map)) => map,
        _ => Map::new(),
    };
    for (key, value) in fields.iter() {
        if let FieldValue::Color(rgb) = value {
            map.insert(key.to_owned(), json!(crate::color_picker_ui::hex(*rgb)));
        }
    }
    if let Some(spec) = photocraft_engine::commands::find(command) {
        for key in document_keys(spec.params) {
            if let Some(index) = map.get(&key).and_then(Value::as_str).and_then(|text| text.parse::<i64>().ok()) {
                map.insert(key, json!(index));
            }
        }
    }
    // The exports to files take the JPEG quality only for a JPEG (a quality would make a WebP lossy).
    if matches!(command, "file.export.layersToFiles" | "file.export.layerCompsToFiles") && map.get("format").and_then(Value::as_str) != Some("jpg") {
        map.remove("quality");
    }
    // Artboards to Files sends its Name Options only while Include Artboard Name is ticked (#13637), and the chosen
    // file type's Export Options only while that box is ticked (#13636).
    if command == "file.export.artboardsToFiles" {
        crate::menus::view_cmds::strip_artboard_name_options(&mut map);
        crate::menus::view_cmds::nest_artboard_export_options(&mut map);
    }
    // An emptied embedded LUT is no data: a stored empty `data` would be read over the chosen `lut`.
    if command == "image.adjustments.colorLookup" && map.get("data").and_then(Value::as_str).is_some_and(str::is_empty) {
        map.remove("data");
    }
    if command == "image.mode.indexedColor" {
        // The matte colour row belongs to the "custom" choice, which the command takes as the colour itself.
        let color = map.remove(rows::MATTE_COLOR);
        if map.get("matte").and_then(Value::as_str) == Some("custom")
            && let Some(color) = color
        {
            map.insert("matte".into(), color);
        }
    }
    // Lens Blur's Source menu names an alpha channel `channel:N` (by index: two channels may share a name); the command
    // takes the `channel` source and the channel's index.
    if command == rows::LENS_BLUR
        && let Some(index) = map.get("depthMap").and_then(Value::as_str).and_then(rows::channel_of_choice)
    {
        map.insert("depthMap".into(), json!("channel"));
        map.insert("depthChannel".into(), json!(index));
    }
    if command == "filter.blur.radialBlur"
        && let Some([x, y]) = fields.point("center")
    {
        // PLAN: assumed the centre Point (dialogs-w25) is in the engine's fractions of the image.
        map.remove("center");
        map.insert("centerX".into(), json!(x));
        map.insert("centerY".into(), json!(y));
    }
    if command == crate::dialogs::image_processor_ui::COMMAND {
        crate::dialogs::image_processor_ui::nest(&mut map);
    }
    if command == super::rotate_canvas_ui::COMMAND {
        super::rotate_canvas_ui::command_params(&mut map);
    }
    // After dialog keys have been reshaped into command keys (Image Processor's jpeg/width
    // becomes jpeg.width, and so on). Doing this earlier dropped those rows.
    // Drop display-only and other keys a closed schema does not take (Bitmap's Input row is
    // `inputResolution`; dispatch would reject it and OK would leave the document unchanged).
    if let Some(spec) = photocraft_engine::commands::find(command)
        && let Some(schema) = photocraft_engine::param_schema::of(spec)
        && !schema.open()
        && !photocraft_engine::is_incomplete(command)
    {
        const KEEP: &[&str] = &["coalesce", "target", "layer", "channel", "targetChannel", "document"];
        map.retain(|key, _| key.starts_with('_') || KEEP.contains(&key.as_str()) || schema.has_key(key));
    }
    params_of(&map)
}

/// The dialog values as the plain JSON map.
fn fields_map(fields: &DialogFields) -> Map<String, Value> {
    match serde_json::to_value(fields) {
        Ok(Value::Object(map)) => map,
        _ => Map::new(),
    }
}

/// Match Color preview and OK may only edit the document and layer that opened the dialog:
/// `destination` is what the dialog captured, `fields` the plain map of its values.
pub(crate) fn validate_match_color_destination(app: &PhotocraftApp, command: &str, destination: &Value, fields: &Map<String, Value>) -> Result<(), String> {
    if command != MATCH_COLOR {
        return Ok(());
    }
    let st = app.session.active().ok_or("the Match Color destination is no longer open")?;
    if *destination != json!((st.doc.id, st.active_layer))
        || st.active_layer.is_some_and(|id| st.doc.layer(id).is_none())
        || fields.get("layer").is_some_and(|layer| st.active_layer.is_none_or(|id| layer.as_u64() != Some(id.0)))
    {
        return Err("the Match Color destination changed while its dialog was open".into());
    }
    let params = app.with_mask_target(MATCH_COLOR, params_of(fields));
    if let Some(reason) = app.session.disabled_reason_with(MATCH_COLOR, &params) {
        return Err(reason);
    }
    Ok(())
}

/// Lens Blur's Depth Map Source names an alpha channel by its index in the document the dialog opened on (`captured`):
/// if another document became active since (the control channel can switch it under the dialog), that index would pick
/// a channel of the other document, so preview and OK refuse it. The other sources do not name a channel.
/// `depth_map` is the `depthMap` param as [`command_params`] makes it.
pub(crate) fn validate_lens_blur_document(app: &PhotocraftApp, command: &str, captured: &Value, depth_map: Option<&str>) -> Result<(), String> {
    if command != rows::LENS_BLUR || depth_map != Some("channel") {
        return Ok(());
    }
    // Only the dialog `open` made captures its document; a Smart Filter edit captures its own target and is checked there.
    let Some(opened) = captured.get(LENS_BLUR_DOCUMENT) else { return Ok(()) };
    let opened = opened.as_u64();
    let active = app.session.active().map(|st| st.doc.id.0);
    if opened.is_some() && opened == active { Ok(()) } else { Err("the Lens Blur depth map channel belongs to another document".into()) }
}

/// The held-OK check: Match Color's destination and Lens Blur's depth channel document captured at open still hold.
pub(crate) fn check(app: &mut PhotocraftApp, result: &ParamResult) -> Result<(), Label> {
    let destination = crate::dialogs::params::captured(app, &result.key);
    let fields = fields_map(&result.fields);
    validate_match_color_destination(app, &result.key, &destination, &fields).map_err(Label::from)?;
    let params = command_params(&result.key, &result.fields);
    validate_lens_blur_document(app, &result.key, &destination, params.get("depthMap").and_then(Value::as_str)).map_err(Label::from)?;
    validate_match_color_source_layer(app, &result.key, &fields).map_err(Label::from)
}

/// The chosen source must be an open document, and an explicit source layer must be in it: a closed or invalid source
/// or a missing layer refuses OK instead of falling back to another document or to Merged.
fn validate_match_color_source_layer(app: &PhotocraftApp, command: &str, fields: &Map<String, Value>) -> Result<(), String> {
    if command != MATCH_COLOR {
        return Ok(());
    }
    let source = fields.get("source").and_then(|source| source.as_u64().or_else(|| source.as_str()?.parse().ok())).and_then(|i| usize::try_from(i).ok());
    let source = match source {
        Some(index) => Some(app.session.documents().get(index).ok_or("the Match Color source document is not open")?),
        None => None,
    };
    let Some(layer) = fields.get("sourceLayer").filter(|layer| !layer.is_null()) else { return Ok(()) };
    let Some(source) = source else { return Ok(()) };
    let listed =
        source.doc.walk().into_iter().any(|(_, _, l)| layer.as_u64() == Some(l.id.0) || layer.as_i64().is_some_and(|v| i64::try_from(l.id.0) == Ok(v)));
    if listed { Ok(()) } else { Err("the Match Color source layer is not in the source document".into()) }
}

/// A filter dialog was cancelled: its preview goes, and the playing action it paused ends.
pub(crate) fn cancel(app: &mut PhotocraftApp, _ctx: &egui::Context, dialog: DialogId) {
    app.filter_preview = None;
    if let Some(token) = crate::actions::token_of(app, dialog) {
        crate::actions::stopped(app, token);
    }
}

/// The document index Match Color's `source` row holds (a choice id, so text; a number from a caller).
fn source_index(fields: &Map<String, Value>) -> Option<usize> {
    let source = fields.get("source")?;
    source.as_u64().or_else(|| source.as_str()?.parse().ok()).and_then(|i| usize::try_from(i).ok())
}

/// Runs [`refresh_match_color_sources`] on the open Match Color dialog and writes back what it changed: the source
/// index after a document closed, a source layer that was deleted, and the identities it compares the next frame with.
pub(crate) fn reconcile_match_color(app: &mut PhotocraftApp) {
    let id = dialog_id(MATCH_COLOR);
    let Some(dialog) = app.framework.dialogs.get_by_id::<HostedParamDialog>(id) else { return };
    let mut fields = fields_map(dialog.fields());
    fields.insert("__command".into(), json!(MATCH_COLOR));
    let before = fields.clone();
    refresh_match_color_sources(&app.session, &mut fields);
    if fields == before {
        return;
    }
    let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedParamDialog>(id) else { return };
    for (key, value) in &fields {
        if before.get(key) != Some(value) {
            match value.as_str().filter(|_| key == "source") {
                Some(text) => dialog.set_field(key.clone(), FieldValue::Text(text.to_owned())),
                None => dialog.set_field(key.clone(), FieldValue::Json(value.clone())),
            }
        }
    }
    for key in before.keys().filter(|key| !fields.contains_key(*key)) {
        dialog.clear_field(key);
    }
}

/// Keep Match Color's source choices live without reinterpreting a shifted document index.
/// Rebuild layer lists only when a document changes; a closed source remains visibly unavailable.
pub(crate) fn refresh_match_color_sources(session: &photocraft_engine::Session, fields: &mut Map<String, Value>) {
    if fields.get("__command").and_then(Value::as_str) != Some(MATCH_COLOR) {
        return;
    }
    let documents = session.documents();
    let state = json!(documents.iter().map(|st| (st.doc.id.0, st.revision, &st.doc.name)).collect::<Vec<_>>());
    if fields.get("__sourceState") == Some(&state) {
        return;
    }
    let old_index = source_index(fields);
    let selected = old_index.and_then(|i| fields.get("__sourceDocuments")?.as_array()?.get(i)?.as_u64());
    let mut ids: Vec<_> = documents.iter().map(|st| st.doc.id.0).collect();
    let mut names: Vec<_> = documents.iter().map(|st| st.doc.name.clone()).collect();
    let mut layers: Vec<Value> = documents
        .iter()
        .map(|st| json!(st.doc.walk().into_iter().rev().map(|(_, _, layer)| json!({"id": layer.id, "name": layer.name})).collect::<Vec<_>>()))
        .collect();
    if let Some(id) = selected {
        let index = ids.iter().position(|current| *current == id).unwrap_or_else(|| {
            let index = ids.len();
            // Keep the missing identity at an invalid engine index, rather than sampling its successor.
            ids.push(id);
            names.push(tl!("Closed document").into());
            layers.push(Value::Null);
            index
        });
        fields.insert("source".into(), json!(index.to_string()));
    }
    let source = source_index(fields);
    if let Some(layer) = fields.get("sourceLayer").and_then(Value::as_u64) {
        let contains_layer = |choices: Option<&Value>| {
            choices.and_then(Value::as_array).is_some_and(|choices| choices.iter().any(|choice| choice.get("id").and_then(Value::as_u64) == Some(layer)))
        };
        // A deleted choice can reset to Merged. An explicitly unavailable reference must
        // survive refresh and reach the engine, which rejects it.
        let previous = old_index.and_then(|i| fields.get("__sourceLayers")?.as_array()?.get(i));
        if contains_layer(previous) && !contains_layer(source.and_then(|i| layers.get(i))) {
            fields.remove("sourceLayer");
        }
    }
    fields.insert("__docs".into(), json!(names));
    fields.insert("__sourceDocuments".into(), json!(ids));
    fields.insert("__sourceLayers".into(), json!(layers));
    fields.insert("__sourceState".into(), state);
}

/// Assign Profile offers only the profiles the document's colour mode can take (#4951): the
/// command refuses one of another colour space. `__profiles` lists them for the `profile` menu,
/// and a remembered choice that doesn't fit this document falls back to the working space.
fn assign_profile_defaults(app: &PhotocraftApp, fields: &mut Map<String, Value>) {
    let Some(st) = app.session.active() else { return };
    let Some(spec) = photocraft_engine::commands::find("edit.assignProfile") else { return };
    let space = photocraft_engine::color_cmds::mode_space(st.doc.mode);
    let fits = |name: &str| name == "none" || app.session.color().resolve(name, Some(&st.doc), None).is_ok_and(|p| p.color_space == space);
    let builtin: Vec<String> = Param::parse_list(spec.params)
        .into_iter()
        .filter(|p| p.key == "profile")
        .flat_map(|p| match p.kind {
            ParamKind::Choice { options, .. } => options.into_iter().map(|(id, _)| id).collect(),
            _ => Vec::new(),
        })
        .collect();
    let installed: Vec<_> = photocraft_engine::installed_profiles::installed().into_iter().filter(|i| i.is_destination()).collect();
    let allowed: Vec<String> =
        builtin.iter().filter(|name| fits(name)).cloned().chain(installed.iter().filter(|i| i.color_space == space).map(|i| i.path.clone())).collect();
    // A path nobody lists (typed by an agent) is left alone: the command decides about it.
    let listed = |cur: &str| builtin.iter().any(|name| name == cur) || installed.iter().any(|i| i.path == cur);
    if fields.get("profile").and_then(Value::as_str).is_some_and(|cur| listed(cur) && !allowed.iter().any(|name| name == cur)) {
        fields.insert("profile".into(), json!("working"));
    }
    fields.insert("__profiles".into(), json!(allowed));
}

/// Arbitrary rotation starts at the angle that straightens the ruler line, when there is one.
fn straighten_defaults(app: &PhotocraftApp, fields: &mut Map<String, Value>) {
    let Some(r) = app.session.active().and_then(|d| d.doc.measurement.ruler) else { return };
    let rot = photocraft_engine::analysis_cmds::straighten_angle(&r);
    // A ruler read from a damaged file could hold non-finite ends: keep the 0° default then.
    if !rot.is_finite() {
        return;
    }
    // Rounded here, in f64, to the field's two decimals: `ui.dialog.open` then `ui.dialog.confirm`
    // can run with no frame drawn between them, and only a drawn frame rounds (#13067).
    let scale = 10f64.powi(ARBITRARY_ANGLE_DECIMALS.min(6) as i32);
    fields.insert("angle".into(), json!((rot.abs() * scale).round() / scale));
    fields.insert("direction".into(), json!(if rot < 0.0 { "ccw" } else { "cw" }));
}

/// Warp Text edits the type layers it opened on (the selected ones, or the one being edited) and
/// starts from the warp the shown layer has, or None, never from remembered values (#218).
fn warp_defaults(app: &PhotocraftApp, fields: &mut Map<String, Value>) {
    if let Some(Value::Object(targets)) = crate::type_tool::formatting_params(app) {
        fields.extend(targets);
    }
    let warp = crate::type_tool::target_text(app).and_then(|t| t.warp.clone());
    let w = warp.unwrap_or(photocraft_doc::text::TextWarp {
        style: "warpNone".into(),
        value: 50.0,
        horizontal_distortion: 0.0,
        vertical_distortion: 0.0,
        horizontal: true,
    });
    // A style without a short id (from a PSD) shows as None, with the layer's values.
    fields.insert("style".into(), json!(photocraft_text::warp::short_style(&w.style).unwrap_or("none")));
    fields.insert("bend".into(), json!(w.value));
    fields.insert("horizontalDistortion".into(), json!(w.horizontal_distortion));
    fields.insert("verticalDistortion".into(), json!(w.vertical_distortion));
    fields.insert("orientation".into(), json!(if w.horizontal { "horizontal" } else { "vertical" }));
}

/// Global Light reflects the active document's current shared lighting, never remembered values.
fn global_light_defaults(app: &PhotocraftApp, fields: &mut Map<String, Value>) {
    let Some(st) = app.session.active() else { return };
    fields.insert("angle".into(), json!(st.doc.global_light.angle));
    fields.insert("altitude".into(), json!(st.doc.global_light.altitude));
}

/// Proof Setup reflects the active document's current proofing condition, never remembered values.
fn proof_setup_defaults(app: &PhotocraftApp, fields: &mut Map<String, Value>) {
    let Some(st) = app.session.active() else { return };
    let pv = app.session.color().proof(st.doc.id);
    fields.insert("profile".into(), json!(pv.setup.name));
    fields.insert("intent".into(), json!(pv.setup.intent.id()));
    fields.insert("bpc".into(), json!(pv.setup.bpc));
    fields.insert("simulatePaper".into(), json!(pv.setup.simulate_paper));
}

/// Commands whose `angle` is a direction in document angles (counter-clockwise from three
/// o'clock): a light or a blur direction, edited on a dial as in Photoshop's Motion Blur, Emboss
/// and Global Light (#3152). Rotation amounts (Twirl, Rotate Arbitrary, Crop, Lens Correction,
/// the Blur Gallery shapes) and angles with another zero (Flame's points up) keep the slider.
const ANGLE_DIAL: &[&str] = &[
    "filter.blur.motionBlur",
    "filter.stylize.emboss",
    "filter.stylize.oilPaint",
    "filter.render.lightingEffects",
    "filter.render.relight",
    "layer.layerStyle.globalLight",
];

fn uses_angle_dial(command: &str, key: &str) -> bool {
    key == "angle" && ANGLE_DIAL.contains(&command)
}

/// What OK on an edited smart filter runs (`smart_ui::confirm_command`).
const SMART_FILTER_SET_PARAMS: &str = "layer.smartFilter.setParams";

/// User-facing params (strip the private `__` keys).
pub fn params_of(f: &Map<String, Value>) -> Value {
    // A document picker left at "None" (-1) means "not given".
    let unset = |k: &str, v: &Value| k == "mapDocument" && v.as_i64() == Some(-1);
    Value::Object(f.iter().filter(|(k, v)| !k.starts_with("__") && !unset(k, v)).map(|(k, v)| (k.clone(), v.clone())).collect())
}

/// Runs the confirmed dialog of `command`: the command with the dialog's parameters, then settles
/// the filter preview and the playing action, and remembers the values on success.
pub(crate) fn run_command(app: &mut PhotocraftApp, ctx: &egui::Context, command: &str, result: ParamResult) -> Result<Value, String> {
    let dialog = result.id;
    if crate::actions::record_again::owns_dialog(app, dialog) {
        return crate::actions::record_again::confirm(app, dialog, &result.key, command_params(&result.key, &result.fields));
    }
    // PLAN: `token_of`, `abandoned`, `resume`, `run_step_dialog` and `authorize_panel_preview` are assumed re-exported at `crate::actions` (panels-w32).
    let token = crate::actions::token_of(app, dialog);
    let params = command_params(command, &result.fields);
    let captured = crate::dialogs::params::captured(app, command);
    if let Err(error) = crate::actions::authorize_panel_preview(app, dialog, command, &params) {
        app.filter_preview = None;
        abandon(app, token);
        return Err(error);
    }
    if needs_displace_map(command, &params) {
        app.filter_preview = None;
        let outcome = ask_displace_map(app, command.to_owned(), result);
        if outcome.is_err() {
            abandon(app, token);
        } else {
            // Cancel and an unavailable picker finish before the Displace callback runs. Observe
            // the whole file-dialog result, including those exits.
            app.after_file_dialog(move |app, outcome| {
                if outcome.is_err() {
                    abandon(app, token);
                }
                outcome
            });
        }
        return outcome;
    }
    finish(app, ctx, command, params, &captured, &result, token)
}

/// The `ParamFamily::run` of the generic filter family: runs the confirmed dialog under its own key
/// and reports a failure through the notices.
pub(crate) fn run(app: &mut PhotocraftApp, ctx: &egui::Context, result: ParamResult) {
    let command = result.key.clone();
    if let Err(error) = run_command(app, ctx, &command, result) {
        crate::notices::error(app, error);
    }
}

/// Lets the playing action drop the step of a dialog that did not run.
fn abandon(app: &mut PhotocraftApp, token: Option<u64>) {
    if let Some(token) = token {
        crate::actions::abandoned(app, token);
    }
}

/// The tail of [`run_command`], shared with the Displace map answer: the smart-filter edit, the
/// command, its preview and the action's next step.
fn finish(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    command: &str,
    params: Value,
    captured: &Value,
    result: &ParamResult,
    token: Option<u64>,
) -> Result<Value, String> {
    let (cmd, params) = match crate::smart_ui::confirm_command(app, captured, command.to_owned(), params) {
        Ok(edit) => edit,
        Err(error) => {
            app.filter_preview = None;
            abandon(app, token);
            return Err(error);
        }
    };
    let outcome = crate::actions::run_step_dialog(app, result.id, &cmd, params.clone());
    if let Ok(r) = &outcome {
        report_file_errors(app, command, r);
    }
    // A filter that runs as a background job keeps its preview on screen until the job
    // lands (`canvas::committed_filter_preview`) instead of flashing the unfiltered image.
    let running =
        app.session.active().is_some_and(|st| app.filter_preview.as_ref().is_some_and(|p| p.key.doc == st.doc.id) && app.session.job_on(st.doc.id).is_some());
    match app.filter_preview.as_mut() {
        Some(p) if outcome.is_ok() && running => p.committing = true,
        _ => app.filter_preview = None,
    }
    if outcome.is_ok() && cmd == "view.newGuideLayout" {
        app.ui.view.guide_layout = params;
    }
    if outcome.is_ok() {
        result.remember(ctx);
        // A step dialog of a playing action: the action continues after it.
        if let Some(token) = token {
            crate::actions::resume(app, token)?;
        }
    } else {
        abandon(app, token);
    }
    outcome
}

/// Batch and the other folder runs (Image Processor, …) go on past a file that fails and list it
/// in their result's `errors`; the status bar and a notice say which files failed and why, so a run
/// that fails on every file isn't silent (#10682). Each file is named by its whole path: files of
/// the same name in different subfolders are told apart.
fn report_file_errors(app: &mut PhotocraftApp, command: &str, r: &Value) {
    let Some(errors) = r.get("errors").and_then(Value::as_array).filter(|e| !e.is_empty()) else { return };
    let lines: Vec<String> = errors
        .iter()
        .map(|e| {
            let file = e.get("file").and_then(Value::as_str).unwrap_or_default();
            format!("{file}: {}", e.get("error").and_then(Value::as_str).unwrap_or_default())
        })
        .collect();
    let label = photocraft_engine::commands::find(command).map_or("Command", |c| c.label);
    let what = crate::i18n::t(label).trim_end_matches('…').to_string();
    crate::notices::file_failures(app, &what, &lines);
}

/// Filter › Distort › Displace confirmed with no map: no open document picked and no path typed
/// (#5806). Photoshop asks for the displacement map file after OK; so does PhotoCraft.
pub(crate) fn needs_displace_map(command: &str, params: &Value) -> bool {
    command == "filter.distort.displace"
        && params.get("mapPath").and_then(Value::as_str).is_none_or(|p| p.trim().is_empty())
        && params.get("mapDocument").and_then(Value::as_i64).is_none_or(|i| i < 0)
}

/// Asks for the displacement map, then runs Displace with the dialog's values and that file, as
/// OK in a dialog that had one would (smart filters included). Cancel applies nothing. In the
/// browser the picker gives contents, not a path the filter can read: the map has to be opened
/// as a document and picked as Map Document there. It displaces the document that asked, even
/// when another one was activated while the picker was open.
pub(crate) fn ask_displace_map(app: &mut PhotocraftApp, command: String, result: ParamResult) -> Result<Value, String> {
    let doc = app.active_doc_id()?;
    let token = crate::actions::token_of(app, result.id);
    let captured = crate::dialogs::params::captured(app, &command);
    let request = PickRequest::open("displacementMap").title(tl!("Choose a displacement map"));
    app.ask_file(request, move |app, picked| {
        let path = match picked {
            Picked::Paths(paths) => paths.into_iter().next().ok_or("no displacement map was chosen")?,
            Picked::Cancelled | Picked::Downloaded(_) => return Err(crate::file_dialog::CANCELLED.into()),
            Picked::Unavailable(error) | Picked::Failed(error) => return Err(error),
            Picked::Contents(_) => return Err("open the displacement map as a document, then choose it as the Map Document".into()),
        };
        let mut params = command_params(&command, &result.fields);
        if let Some(object) = params.as_object_mut() {
            object.insert("mapPath".into(), json!(path));
        }
        app.refocus(doc)?;
        let ctx = app.ctx().clone();
        finish(app, &ctx, &command, params, &captured, &result, token)
    })
}

/// Compute a preview document: run `command` with `params` on the proxy (scaled) copy of `doc`.
pub fn preview_document(doc: &Document, active: Option<photocraft_doc::LayerId>, command: &str, params: &Value, k: u32) -> Option<Document> {
    preview_document_with(doc, active, command, params, k, None)
}

/// [`preview_document`] whose filter stops early, giving `None`, once `cancel` is cancelled
/// (a preview superseded by newer dialog values).
pub fn preview_document_with(
    doc: &Document,
    active: Option<photocraft_doc::LayerId>,
    command: &str,
    params: &Value,
    k: u32,
    cancel: Option<&photocraft_engine::jobs::JobCtx>,
) -> Option<Document> {
    preview_document_for_target(
        doc,
        active,
        command,
        params,
        k,
        cancel,
        photocraft_engine::channel_cmds::ChannelTarget::Composite,
        &[],
        &ColorSettings::default(),
        None,
    )
}

/// Run the preview with the same pixel target as the final command, including individual channels.
/// `documents` are the open documents in session order: document params (Match Color's Source,
/// Displace's map) index into them. `color` is the app's Edit › Color Settings: Assign and Convert
/// to Profile's `working` is the working space chosen there, not the built-in default.
/// `tool_colors` carries the caller's working-space foreground/background values; standalone
/// previews without a session use the engine's default tool colors.
#[allow(clippy::too_many_arguments)]
pub(crate) fn preview_document_for_target(
    doc: &Document,
    active: Option<photocraft_doc::LayerId>,
    command: &str,
    params: &Value,
    k: u32,
    cancel: Option<&photocraft_engine::jobs::JobCtx>,
    target: photocraft_engine::channel_cmds::ChannelTarget,
    documents: &[Arc<Document>],
    color: &ColorSettings,
    tool_colors: Option<([f32; 4], [f32; 4])>,
) -> Option<Document> {
    // Warp Text renders the type again from the layer model (font size, position), which a
    // reduced proxy doesn't scale: it runs at full size and the result is reduced (#218).
    let (run_k, reduce_k) = if command == "type.warpText" { (1, k) } else { (k, 1) };
    let mut proxy = crate::proxy::proxy_document(doc, run_k);
    let mut p = params.clone();
    if is_smart_edit(command) {
        let layer = match params.get("layer") {
            Some(id) => photocraft_doc::LayerId(id.as_u64()?),
            None => active?,
        };
        let index = usize::try_from(params.get("index")?.as_u64()?).ok()?;
        let photocraft_doc::LayerContent::Smart(sm) = &mut proxy.layer_mut(layer)?.content else { return None };
        // Proxy_document scales the cache, placement and shared mask (#5766). Editing a filter
        // re-renders its source: scale every filter's lengths and the mask's feather as well.
        if run_k > 1 {
            if let Some(mask) = &mut sm.filter_mask {
                mask.feather /= run_k as f32;
            }
            for f in &mut sm.smart_filters {
                scale_preview_params(&f.command, &mut f.params, run_k);
            }
            if command == "layer.smartFilter.setParams" {
                let f = sm.smart_filters.get(index)?;
                scale_preview_params(&f.command, p.get_mut("params")?, run_k);
            }
        }
    } else {
        scale_preview_params(command, &mut p, run_k);
    }
    let mut s = photocraft_engine::Session::new();
    s.color_mut().settings = color.clone();
    if let Some((foreground, background)) = tool_colors {
        s.tools.foreground = foreground;
        s.tools.background = background;
    }
    s.set_inline_job_ctx(cancel.cloned());
    let doc_id = proxy.id;
    s.add_document(proxy, None);
    // Bitmap's Output resolution resamples the image: the preview keeps the canvas's size and
    // resolution, so its dots and halftone screen show at their printed size (#5559).
    if command == crate::bitmap_dialog::COMMAND
        && let Some(o) = p.as_object_mut()
    {
        o.remove("resolution");
    }
    add_source_documents(&mut s, doc_id, command, &mut p, documents, run_k);
    if let Some(id) = active {
        s.select_layer(id).ok()?;
    }
    s.active_mut()?.channel_view.target = target;
    s.execute(command, p).ok()?;
    s.active().map(|d| crate::proxy::proxy_document(&d.doc, reduce_k))
}

/// Whether `command` is a Smart Filter edit (its parameters or its blending) rather than a filter run on its own.
fn is_smart_edit(command: &str) -> bool {
    matches!(command, "layer.smartFilter.setParams" | "layer.smartFilter.blendingOptions")
}

/// Each stage constrains the proxy factor; an unscalable filter keeps full-resolution preview.
pub(crate) fn smart_preview_factor(doc: &Document, command: &str, params: &Value, k: u32) -> u32 {
    if !is_smart_edit(command) {
        return preview_factor(command, params, k);
    }
    let Some(layer) = params.get("layer").and_then(Value::as_u64).and_then(|id| doc.layer(photocraft_doc::LayerId(id))) else { return 1 };
    let photocraft_doc::LayerContent::Smart(sm) = &layer.content else { return 1 };
    let mut factor = sm.smart_filters.iter().fold(k, |k, f| preview_factor(&f.command, &f.params, k));
    if command == "layer.smartFilter.setParams"
        && let Some(index) = params.get("index").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok())
        && let Some(f) = sm.smart_filters.get(index)
        && let Some(p) = params.get("params")
    {
        factor = preview_factor(&f.command, p, factor);
    }
    factor
}

/// Scale the pixel-sized parameters of `command` down by the preview proxy factor `k`, keeping each above its minimum and negative lengths negative.
fn scale_preview_params(command: &str, params: &mut Value, k: u32) {
    if command == photocraft_engine::plugin_cmds::RUN {
        // A plug-in declares its pixel lengths (`"unit": "px"` in its manifest), and only those
        // are scaled: a 12 px shift previewed 4 times too wide at k = 4, and a unitless param that
        // happens to be called `size` or `radius` must not be divided (#7524).
        photocraft_engine::plugin_cmds::scale_for_preview(params, k);
    } else if k > 1
        && let Some(o) = params.as_object_mut()
    {
        let spec = photocraft_engine::commands::find(command).map(|c| Param::parse_list(c.params)).unwrap_or_default();
        for (key, v) in o.iter_mut() {
            if is_pixel_param_of(command, key)
                && let Some(x) = v.as_f64()
            {
                // Match the engine's minimum after scaling (e.g. Box Blur 1, Mosaic 2).
                // Previously its tolerant decoder clamped these; new calls validate first.
                let minimum = spec.iter().find(|p| p.key == key.as_str()).and_then(|p| match &p.kind {
                    ParamKind::Range { min, .. } => Some(*min),
                    _ => None,
                });
                let floor = minimum.unwrap_or(0.0).max(if key == "cellSize" { 1.0 } else { 0.1 });
                // Negative lengths (Offset's left/up shift, Displace's inverted scale) keep their
                // sign: the floor that keeps a radius positive turned them into +0.1 (#2063).
                let x = x / k as f64;
                *v = json!(if x > 0.0 { x.max(floor) } else { minimum.map_or(x, |m| x.max(m)) });
            }
        }
    }
    // Blur Gallery pins carry their blur in document pixels too.
    if command.starts_with("filter.blurGallery.") {
        crate::blur_pins::scale_for_proxy(params, k);
    }
}

/// The preview session holds only the proxy of the edited document (index 0), so a document param
/// pointing at another open document found nothing there: Match Color's preview with a Source
/// document failed, or matched the image to itself (#4851). Add each referenced document (as a
/// proxy at the same factor) and point the param at its index in the preview session.
fn add_source_documents(s: &mut photocraft_engine::Session, doc_id: photocraft_doc::DocId, command: &str, p: &mut Value, documents: &[Arc<Document>], k: u32) {
    let Some(spec) = photocraft_engine::commands::find(command) else { return };
    let Some(o) = p.as_object_mut() else { return };
    let mut added: Vec<(usize, usize)> = Vec::new();
    let mut remap = |v: &mut Value| {
        let Some(i) = v.as_u64().and_then(|i| usize::try_from(i).ok()) else { return };
        // An index past the open documents stays as given, so the preview fails like OK would.
        let Some(src) = documents.get(i) else { return };
        let at = if src.id == doc_id {
            0
        } else if let Some(&(_, at)) = added.iter().find(|(j, _)| *j == i) {
            at
        } else {
            let at = s.add_document(crate::proxy::proxy_document(src, k), None);
            added.push((i, at));
            at
        };
        *v = json!(at);
    };
    for key in document_keys(spec.params) {
        if let Some(v) = o.get_mut(&key) {
            remap(v);
        }
    }
    // A smart filter being edited carries its filter's params one level down: Displace's
    // `mapDocument` there names an open document too.
    if command == SMART_FILTER_SET_PARAMS
        && let Some(v) = o.get_mut("params").and_then(Value::as_object_mut).and_then(|f| f.get_mut("mapDocument"))
    {
        remap(v);
    }
    // Adding a document makes it active: the command still runs on the edited one.
    s.set_active(0);
}

/// The same document parameters the preview imports also identify its pixel dependencies.
/// Keep their revisions separate from command params and slider changes.
pub(crate) fn preview_source_revisions(session: &photocraft_engine::Session, command: &str, params: &Value) -> Vec<(photocraft_doc::DocId, u64)> {
    let Some(spec) = photocraft_engine::commands::find(command) else { return Vec::new() };
    let mut indices: Vec<_> = document_keys(spec.params).into_iter().filter_map(|key| params.get(&key)?.as_u64()).collect();
    // Smart-filter Displace has its map document one level down, as in add_source_documents.
    if command == SMART_FILTER_SET_PARAMS
        && let Some(index) = params.get("params").and_then(|p| p.get("mapDocument")).and_then(Value::as_u64)
    {
        indices.push(index);
    }
    indices
        .into_iter()
        .filter_map(|index| {
            let st = session.documents().get(usize::try_from(index).ok()?)?;
            Some((st.doc.id, st.revision))
        })
        .collect()
}

/// Cached preview state on the app.
pub struct FilterPreview {
    pub key: FilterPreviewKey,
    pub result: Option<Arc<Document>>,
    /// OK was pressed and the filter runs as a background job: the preview stays on screen until
    /// the job lands, so the canvas doesn't flash the unfiltered image in between.
    pub committing: bool,
}

/// Identity of one filter dialog's live preview: the request the preview worker computes.
///
/// `PartialEq` is the whole request: it says a worker result is exactly what the dialog is asking
/// for now. Whether a result belongs to the open preview at all is `LivePreview::owns`.
#[derive(Clone, Debug, PartialEq)]
pub struct FilterPreviewKey {
    pub doc: photocraft_doc::DocId,
    pub revision: u64,
    pub sources: Vec<(photocraft_doc::DocId, u64)>,
    pub dialog: DialogId,
    /// Which open of the dialog this is (`DialogHost::open_serial`): a dialog's id repeats at every open of its command.
    pub open: u64,
    pub active: Option<photocraft_doc::LayerId>,
    pub target: photocraft_engine::channel_cmds::ChannelTarget,
    pub command: String,
    pub params: Value,
    pub k: u32,
    pub tool_colors: ([f32; 4], [f32; 4]),
    /// The scratch read-error generation (`crate::scratch_reads`): a tile that fails to read and recovers changes
    /// the pixels without a new revision, and a preview computed during the error must not be kept.
    pub scratch: u64,
}

#[cfg(test)]
mod global_light_angle_range_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod multi_channel_dialog_tests {
    use craft::ui::patterns::form_dialogs::HostedParamDialog;

    use super::*;

    /// The command parameters of the open dialog `id`.
    fn dialog_params(app: &PhotocraftApp, id: DialogId, command: &str) -> Value {
        command_params(command, app.framework.dialogs.get_by_id::<HostedParamDialog>(id).expect("the hosted dialog").fields())
    }

    #[test]
    fn filter_mask_dialog_keeps_its_target_when_color_channels_change() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = app.ctx().clone();
        app.run("file.new", json!({"width":32,"height":24,"mode":"lab","depth":16})).unwrap();
        app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        app.run("filter.blur.gaussianBlur", json!({"radius":2})).unwrap();
        app.run("paint.stroke", json!({"points":[[8,0],[8,23]],"size":8,"hardness":1,"color":"#000000","target":"filterMask"})).unwrap();
        app.run("channel.target", json!({"colors":[0,2]})).unwrap();
        app.run("view.layerMask", json!({"mode":"gray","mask":"filter"})).unwrap();
        let command = "filter.blur.gaussianBlur";
        let id = open(&mut app, command).unwrap();
        let params = dialog_params(&app, id, command);
        assert_eq!(params["target"], "filterMask");
        app.run("channel.target", json!({"channel":"composite"})).unwrap();
        let st = app.session.active().unwrap();
        let layer = st.active_layer.unwrap();
        let preview = preview_document(&st.doc, Some(layer), command, &params, 1).unwrap();
        crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
        let mask = |doc: &Document| match &doc.layer(layer).unwrap().content {
            photocraft_doc::LayerContent::Smart(sm) => sm.filter_mask.clone(),
            _ => panic!("expected a smart object"),
        };
        assert_eq!(mask(&app.session.active().unwrap().doc), mask(&preview));
    }

    #[test]
    fn mask_view_dialogs_capture_the_mask_for_preview_and_confirmation() {
        for mode in ["rgb", "cmyk", "lab"] {
            for depth in [8, 16, 32] {
                for command in ["image.adjustments.levels", "image.adjustments.curves", "filter.blur.gaussianBlur"] {
                    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
                    let ctx = app.ctx().clone();
                    app.run("file.new", json!({"width":32,"height":24,"mode":mode,"depth":depth})).unwrap();
                    app.run("layer.layerMask.revealAll", json!({})).unwrap();
                    app.session
                        .edit("Mask edge", |doc, id| {
                            doc.layer_mut(id.unwrap()).unwrap().mask.as_mut().unwrap().surface.fill_rect(photocraft_geom::Rect::new(0, 0, 16, 24), &[0.25]);
                            Ok(())
                        })
                        .unwrap();
                    app.run("channel.target", json!({"colors":[0,2]})).unwrap();
                    app.run("view.layerMask", json!({"mode":"gray"})).unwrap();
                    let adjust = command.starts_with("image.");
                    let id = if adjust { crate::adjust_dialog::open(&mut app, command).unwrap() } else { open(&mut app, command).unwrap() };
                    let (key, value) = match command {
                        "image.adjustments.levels" => ("gamma", json!(1.4)),
                        "image.adjustments.curves" => ("points", json!([[0, 0], [100, 160], [255, 255]])),
                        _ => ("radius", json!(2)),
                    };
                    if adjust {
                        // The adjustment dialog's fields are its typed value; an engine parameter is laid over its command parameters.
                        let mut shown = crate::adjust_dialog::command_params(&app, id);
                        shown[key] = value;
                        assert!(crate::adjust_dialog::apply_params(&mut app, id, &shown), "the dialog takes {key}");
                    } else {
                        app.framework.dialogs.set_json_field(&ctx, id, key, value).unwrap();
                    }
                    let params = if adjust { crate::adjust_dialog::command_params(&app, id) } else { dialog_params(&app, id, command) };
                    assert_eq!(params["target"], json!("mask"));
                    app.run("channel.target", json!({"channel":{"color":1}})).unwrap();
                    let st = app.session.active().unwrap();
                    let layer = st.active_layer.unwrap();
                    let before = st.doc.layer(layer).unwrap().clone();
                    let preview = preview_document(&st.doc, Some(layer), command, &params, 1).unwrap();
                    let shown = preview.layer(layer).unwrap();
                    assert_eq!(shown.surface(), before.surface());
                    assert_ne!(shown.mask.as_ref().unwrap().surface, before.mask.as_ref().unwrap().surface);
                    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
                    let actual = app.session.active().unwrap().doc.layer(layer).unwrap();
                    assert_eq!(actual.surface(), before.surface());
                    assert_eq!(actual.mask.as_ref().unwrap().surface, shown.mask.as_ref().unwrap().surface);
                }
            }
        }
    }

    #[test]
    fn ordinary_filter_captures_the_set_and_preview_equals_confirmation() {
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width":32,"height":24,"depth":16})).unwrap();
        session
            .edit("edge", |doc, id| {
                doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 16, 24), &[0.2, 0.4, 0.6, 1.0]);
                Ok(())
            })
            .unwrap();
        session.execute("channel.target", json!({"colors":[0,2]})).unwrap();
        let mut app = PhotocraftApp::new(session, Default::default());
        let command = "filter.blur.gaussianBlur";
        let id = open(&mut app, command).unwrap();
        let params = dialog_params(&app, id, command);
        assert_eq!(params["target"], json!({"colors":[0,2]}));
        app.session.execute("channel.target", json!({"channel":"green"})).unwrap();
        let st = app.session.active().unwrap();
        let layer = st.active_layer.unwrap();
        let preview = preview_document(&st.doc, Some(layer), command, &params, 1).unwrap();
        app.session.execute(command, params).unwrap();
        assert_eq!(preview.layer(layer).unwrap().surface(), app.session.active().unwrap().doc.layer(layer).unwrap().surface());
    }
}
