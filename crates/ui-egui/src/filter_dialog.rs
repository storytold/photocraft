//! Filter dialogs generated from engine command parameter specs, with live on-canvas preview.
//!
//! The preview runs the *same engine command* on the downsampled proxy document (pixel-sized
//! parameters scaled by the proxy factor), so what you preview is what you get.

use std::sync::Arc;

use photocraft_doc::Document;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Range {
        min: f32,
        max: f32,
        default: f32,
    },
    Choice(Vec<String>),
    Bool(bool),
    Int {
        default: i64,
    },
    /// Free text (e.g. a file path).
    Text,
    /// One of the open documents (stored as its index).
    Document,
    /// A row-major grid of integers (`int[25]` = 5×5).
    Grid(usize),
    /// Structured JSON (pins, curve points…): settable through the command, not shown in the dialog.
    Json,
    /// A `"#rrggbb"` colour (`color`, or `color=#rrggbb` with a default), picked with a swatch.
    /// Without a default the param stays unset until a colour is picked.
    Color(Option<[f32; 3]>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub key: String,
    pub kind: Kind,
}

/// Parse the registry's parameter notation, e.g.
/// `{"radius":0.1..1000=1,"method":"spin|zoom","monochromatic":bool,"seed":u32=0,"horizontal":px=0}`.
pub fn parse_spec(spec: &str) -> Vec<Param> {
    let inner = object_body(spec);
    let mut out = Vec::new();
    // Split on commas that start a new `"key":` (not inside strings or brackets).
    let mut parts: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_str = false;
    let mut depth = 0i32;
    for ch in inner.chars() {
        if ch == '"' {
            in_str = !in_str;
        }
        if !in_str {
            match ch {
                '[' | '{' => depth += 1,
                ']' | '}' => depth -= 1,
                _ => {}
            }
        }
        if ch == ',' && !in_str && depth == 0 {
            parts.push(std::mem::take(&mut cur));
        } else {
            cur.push(ch);
        }
    }
    if !cur.trim().is_empty() {
        parts.push(cur);
    }
    for part in parts {
        let Some((k, v)) = part.split_once(':') else { continue };
        let key = k.trim().trim_matches('"').to_string();
        if key.is_empty() || key == "layer" {
            continue;
        }
        let v = v.trim();
        let kind = if let Some(choices) = v.strip_prefix('"') {
            let body = choices.split('"').next().unwrap_or("");
            Kind::Choice(body.split('|').map(str::to_string).collect())
        } else if let Some(rest) = v.strip_prefix("bool") {
            Kind::Bool(rest.trim_start_matches('=').trim() == "true")
        } else if v == "text" {
            Kind::Text
        } else if v == "color" {
            Kind::Color(None)
        } else if let Some(default) = v.strip_prefix("color=") {
            Kind::Color(crate::color_picker_ui::parse_hex(default))
        } else if v == "doc" {
            Kind::Document
        } else if v == "json" || v.starts_with("layer id") || v.starts_with('[') || v.starts_with('{') {
            Kind::Json
        } else if let Some(n) = v.strip_prefix("int[").and_then(|r| r.strip_suffix(']')).and_then(|n| n.parse().ok()) {
            Kind::Grid(n)
        } else if let Some((range, default)) = v.split_once('=').map(|(a, b)| (a, b.trim())).or(Some((v, ""))) {
            if let Some((lo, hi)) = range.split_once("..") {
                let min = lo.trim().parse().unwrap_or(0.0);
                let max = hi.trim().parse().unwrap_or(100.0);
                let default = default.parse().unwrap_or(min);
                Kind::Range { min, max, default }
            } else {
                Kind::Int { default: default.parse().unwrap_or(0) }
            }
        } else {
            continue;
        };
        out.push(Param { key, kind });
    }
    out
}

/// The inside of the spec's leading `{…}` object. Notes after it, like `→ {…}` results or
/// `(per-range [c,m,y,k] arrays: …)`, are documentation: read as parameters, their commas split
/// off junk fields and glued the note onto the last parameter (Selective Color's `"reds":json`
/// became a number field, and its OK then failed).
fn object_body(spec: &str) -> &str {
    let s = spec.trim();
    let Some(body) = s.strip_prefix('{') else { return s };
    let (mut depth, mut in_str) = (0i32, false);
    for (i, ch) in body.char_indices() {
        match ch {
            '"' => in_str = !in_str,
            '[' | '{' if !in_str => depth += 1,
            ']' | '}' if !in_str => {
                if depth == 0 {
                    return body.get(..i).unwrap_or(body);
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    body.trim_end_matches('}')
}

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
/// Correction's perspective `horizontal` / `vertical` (-100..100). Scaled by the proxy factor,
/// their preview drew a quarter-size tree and a quarter of the perspective (#2063).
fn is_pixel_param_of(command: &str, key: &str) -> bool {
    is_pixel_param(key)
        && !matches!((command, key), ("filter.render.tree" | "filter.render.pictureFrame", "size") | ("filter.lensCorrection", "horizontal" | "vertical"))
}

/// Filters whose features have a fixed size in document pixels that no dialog parameter scales:
/// per-pixel noise and dots (Add Noise, Diffuse, Mezzotint, Fibers), pixel kernels and scan lines
/// (Custom, Reduce Noise, De-Interlace, Trace Contour's one-pixel lines), fixed lengths and
/// widths (Ripple, Oil Paint, Wind's streaks, Extrude's depth, Flame's length, width and
/// interval) and Tree, whose branches are built from 6 px segments. On a reduced proxy those
/// features come out k times too large (or a different tree grows), so these preview at full
/// resolution: what the preview shows is what OK applies, as in Photoshop (#2063).
const FULL_RESOLUTION_PREVIEW: &[&str] = &[
    "filter.distort.ripple",
    "filter.stylize.diffuse",
    "filter.stylize.oilPaint",
    "filter.render.flame",
    "filter.render.fibers",
    "filter.pixelate.mezzotint",
    "filter.noise.addNoise",
    "filter.noise.reduceNoise",
    "filter.other.custom",
    "filter.video.deInterlace",
    "filter.stylize.traceContour",
    "filter.stylize.wind",
    "filter.stylize.extrude",
    "filter.render.tree",
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
    let Some(spec) = photocraft_engine::commands::find(command) else { return k };
    let k = parse_spec(spec.params).into_iter().fold(k, |k, p| match p.kind {
        Kind::Range { min, .. } if min > 0.0 && is_pixel_param_of(command, &p.key) => match params.get(&p.key).and_then(Value::as_f64) {
            Some(v) if v.is_finite() => k.min((v / f64::from(min)).floor().clamp(1.0, f64::from(k)) as u32),
            _ => k,
        },
        _ => k,
    });
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
    "image.adjustments.selectiveColor",
    "image.adjustments.colorLookup",
    "image.adjustments.shadowsHighlights",
    "image.adjustments.replaceColor",
    "image.adjustments.matchColor",
    "image.adjustments.hdrToning",
    "image.rotation.arbitrary",
    "image.mode.indexedColor",
    "image.mode.bitmap",
    "image.mode.duotone",
    "layer.matting.defringe",
    "layer.matting.colorDecontaminate",
    "layer.layerStyle.scaleEffects",
    "type.warpText",
];

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
                | "view.newGuide"
                | "select.refineEdge"
                | "edit.assignProfile"
                | "edit.convertToProfile"
                | "view.proofSetup"
                | "layer.layerStyle.globalLight"
                | "image.mode.colorTable"
                | "edit.definePattern"
        ))
        && photocraft_engine::commands::find(command).is_some_and(|c| !parse_spec(c.params).is_empty())
}

/// Keep only portable, schema-valid choices. Never retain document ids, free-form
/// paths, raw JSON, or values that a newer command version no longer accepts.
fn rememberable(kind: &Kind, value: &Value) -> bool {
    match kind {
        Kind::Range { min, max, .. } => value.as_f64().is_some_and(|n| n.is_finite() && n >= f64::from(*min) && n <= f64::from(*max)),
        Kind::Choice(choices) => value.as_str().is_some_and(|v| choices.iter().any(|choice| choice == v)),
        Kind::Bool(_) => value.is_boolean(),
        Kind::Int { .. } => value.as_i64().is_some(),
        _ => false,
    }
}

fn restore_remembered(app: &PhotocraftApp, command: &str, spec: &str, fields: &mut Map<String, Value>) {
    let Some(Value::Object(saved)) = app.session.prefs().dialogs.get(command) else { return };
    for p in parse_spec(spec) {
        if let Some(v) = saved.get(&p.key).filter(|v| rememberable(&p.kind, v)) {
            fields.insert(p.key, v.clone());
        }
    }
}

/// Remember successful built-in schema dialogs; failed or cancelled dialogs do not persist.
pub(crate) fn remember(app: &mut PhotocraftApp, command: &str, fields: &Map<String, Value>) {
    if !fields.contains_key("__filter") || fields.contains_key("__spec") {
        return;
    }
    let Some(spec) = photocraft_engine::commands::find(command) else { return };
    let saved: Map<String, Value> =
        parse_spec(spec.params).into_iter().filter_map(|p| fields.get(&p.key).filter(|v| rememberable(&p.kind, v)).map(|v| (p.key, v.clone()))).collect();
    if !saved.is_empty() {
        app.session.prefs.edit(|prefs| prefs.dialogs.insert(command.into(), Value::Object(saved)));
    }
}

pub fn open(app: &mut PhotocraftApp, command: &str) -> Option<u64> {
    let spec = photocraft_engine::commands::find(command)?;
    let mut fields = Map::new();
    fields.insert("__command".into(), json!(command));
    fields.insert("__label".into(), json!(spec.label));
    fields.insert("__filter".into(), json!(true));
    if command.starts_with("filter.") || PREVIEWED.contains(&command) {
        fields.insert("__preview".into(), json!(true));
    }
    for p in parse_spec(spec.params) {
        let v = match &p.kind {
            Kind::Range { default, .. } if command == "image.mode.indexedColor" && p.key == "colors" => json!(default.round().clamp(2.0, 256.0) as u32),
            Kind::Range { default, .. } => json!(default),
            Kind::Choice(c) => json!(c.first().cloned().unwrap_or_default()),
            Kind::Bool(default) => json!(default),
            Kind::Int { default } => json!(default),
            Kind::Text => json!(""),
            Kind::Document => json!(-1),
            // Identity kernel: 1 in the centre.
            Kind::Grid(n) => json!((0..*n).map(|i| i64::from(i == *n / 2)).collect::<Vec<_>>()),
            Kind::Color(Some(rgb)) => json!(crate::color_picker_ui::hex(*rgb)),
            Kind::Color(None) => continue,
            // Colour inputs come from the current swatches, so the proxy preview matches the result.
            Kind::Json if p.key == "foreground" => json!(app.session.tools.foreground),
            Kind::Json if p.key == "background" => json!(app.session.tools.background),
            Kind::Json => continue,
        };
        fields.insert(p.key, v);
    }
    restore_remembered(app, command, spec.params, &mut fields);
    if command == "image.rotation.arbitrary" {
        straighten_defaults(app, &mut fields);
    }
    if command == "type.warpText" {
        warp_defaults(app, &mut fields);
    }
    // Photoshop's Pattern Name dialog starts from the name the pattern would get anyway.
    if command == "edit.definePattern" {
        fields.insert("name".into(), json!(photocraft_engine::pattern_cmds::default_name(&app.session)));
    }
    if parse_spec(spec.params).iter().any(|p| p.kind == Kind::Document) {
        // The document picker lists every open document (params refer to them by index).
        let names: Vec<String> = app.session.documents().iter().map(|d| d.doc.name.clone()).collect();
        fields.insert("__docs".into(), json!(names));
    }
    let id = app.ui.open_dialog(crate::state::DialogKind::Command, fields);
    Some(id)
}

/// Arbitrary rotation starts at the angle that straightens the ruler line, when there is one.
fn straighten_defaults(app: &PhotocraftApp, fields: &mut Map<String, Value>) {
    let Some(r) = app.session.active().and_then(|d| d.doc.measurement.ruler) else { return };
    let rot = photocraft_engine::analysis_cmds::straighten_angle(&r);
    // A ruler read from a damaged file could hold non-finite ends: keep the 0° default then.
    if !rot.is_finite() {
        return;
    }
    fields.insert("angle".into(), json!(rot.abs()));
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

/// A filter dialog with live preview for `command` whose parameters follow `spec` (registry
/// notation) instead of the command's own; `fixed` params (e.g. a plug-in id) are passed through.
pub fn open_with_spec(app: &mut PhotocraftApp, command: &str, label: &str, spec: &str, fixed: Map<String, Value>) -> u64 {
    let mut fields = fixed;
    fields.insert("__command".into(), json!(command));
    fields.insert("__label".into(), json!(label));
    fields.insert("__filter".into(), json!(true));
    fields.insert("__preview".into(), json!(true));
    fields.insert("__spec".into(), json!(spec));
    for p in parse_spec(spec) {
        let v = match &p.kind {
            Kind::Range { default, .. } => json!(default),
            Kind::Choice(c) => json!(c.first().cloned().unwrap_or_default()),
            Kind::Bool(default) => json!(default),
            Kind::Int { default } => json!(default),
            Kind::Color(Some(rgb)) => json!(crate::color_picker_ui::hex(*rgb)),
            _ => continue,
        };
        fields.insert(p.key, v);
    }
    app.ui.open_dialog(crate::state::DialogKind::Command, fields)
}

pub(crate) fn label(key: &str) -> String {
    tl!(&source_label(key)).to_owned()
}

pub(crate) fn source_label(key: &str) -> String {
    // camelCase → "Camel Case"
    let mut s = String::new();
    for (i, ch) in key.chars().enumerate() {
        if i == 0 {
            s.extend(ch.to_uppercase());
        } else if ch.is_uppercase() {
            s.push(' ');
            s.push(ch);
        } else {
            s.push(ch);
        }
    }
    s
}

fn choice_label(v: &str) -> String {
    label(v)
}

fn uses_logarithmic_slider(min: f32, max: f32) -> bool {
    min > 0.0 && max / min > 500.0
}

/// Schema dialogs whose `profile` menu lists the installed profiles after the built-ins.
const PROFILE_DIALOGS: &[&str] = &["edit.assignProfile", "edit.convertToProfile", "view.proofSetup"];

/// Dialog body for filter commands.
pub fn body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let cmd = f.get("__command").and_then(Value::as_str).unwrap_or_default().to_string();
    // Plug-in dialogs carry their own spec (from the plug-in's manifest).
    let spec = match f.get("__spec").and_then(Value::as_str) {
        Some(s) => s.to_string(),
        None => match photocraft_engine::commands::find(&cmd) {
            Some(c) => c.params.to_string(),
            None => return,
        },
    };
    for p in parse_spec(&spec) {
        match p.kind {
            Kind::Range { min, max, default } => {
                let mut v = f.get(&p.key).and_then(Value::as_f64).unwrap_or(default as f64) as f32;
                if cmd == "image.mode.indexedColor" && p.key == "colors" {
                    let mut count = v.round().clamp(2.0, 256.0) as u32;
                    crate::widgets::color_count_row(ui, &label(&p.key), &mut count);
                    f.insert(p.key, json!(count));
                    continue;
                }
                let unit = if is_pixel_param_of(&cmd, &p.key) {
                    "px"
                } else if p.key == "angle" {
                    "°"
                } else if p.key == "amount" && max <= 500.0 {
                    "%"
                } else {
                    ""
                };
                if uses_logarithmic_slider(min, max) {
                    // Keep one numeric field for direct entry and one log-scaled slider for
                    // wide ranges. slider_row would add a second, linear slider (#146).
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(label(&p.key)).color(t.text_dim));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            crate::widgets::value_field(ui, &mut v, min..=max, unit, 74.0);
                        });
                    });
                    let mut lv = v.clamp(min, max).ln();
                    if crate::widgets::slider(ui, &mut lv, min.ln()..=max.ln(), None).changed() {
                        v = lv.exp();
                    }
                    ui.add_space(4.0);
                } else {
                    crate::widgets::slider_row(ui, &label(&p.key), &mut v, min..=max, unit, None);
                }
                // Small ranges (0–1 thresholds and centres) keep the two decimals their field shows.
                let scale = if max - min <= 10.0 { 100.0 } else { 10.0 };
                f.insert(p.key, json!((v * scale).round() / scale));
            }
            Kind::Choice(options) => {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(label(&p.key)).color(t.text_dim));
                    let mut cur = f.get(&p.key).and_then(Value::as_str).unwrap_or(&options[0]).to_string();
                    let labels: Vec<String> = options.iter().map(|o| choice_label(o)).collect();
                    let mut opts: Vec<(String, &str)> = options.iter().cloned().zip(labels.iter().map(String::as_str)).collect();
                    // Like Photoshop, the profile menus also list every profile installed in
                    // the system (printer and paper profiles), by description.
                    let installed = if p.key == "profile" && PROFILE_DIALOGS.contains(&cmd.as_str()) {
                        photocraft_engine::installed_profiles::installed().into_iter().filter(|i| i.is_destination()).collect()
                    } else {
                        Vec::new()
                    };
                    opts.extend(installed.iter().map(|i| (i.path.clone(), i.description.as_str())));
                    // A profile given by path that isn't listed (typed by an agent, or spelled with
                    // different case) still shows, by its file name, instead of a blank menu.
                    let stem = std::path::Path::new(&cur).file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if p.key == "profile" && PROFILE_DIALOGS.contains(&cmd.as_str()) && !opts.iter().any(|(v, _)| *v == cur) {
                        opts.push((cur.clone(), stem.as_str()));
                    }
                    crate::widgets::dropdown(ui, &format!("flt-{cmd}-{}", p.key), &mut cur, &opts, 170.0);
                    f.insert(p.key.clone(), json!(cur));
                });
            }
            Kind::Bool(default) => {
                let mut b = f.get(&p.key).and_then(Value::as_bool).unwrap_or(default);
                crate::widgets::checkbox(ui, &mut b, &label(&p.key));
                f.insert(p.key, json!(b));
            }
            Kind::Text => {
                let mut v = f.get(&p.key).and_then(Value::as_str).unwrap_or_default().to_string();
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(label(&p.key)).color(t.text_dim));
                    ui.add(egui::TextEdit::singleline(&mut v).desired_width(200.0));
                });
                if v.is_empty() {
                    f.remove(&p.key);
                } else {
                    f.insert(p.key, json!(v));
                }
            }
            Kind::Document => {
                let names: Vec<String> =
                    f.get("__docs").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
                let cur = f.get(&p.key).and_then(Value::as_i64).unwrap_or(-1);
                let mut sel = cur.to_string();
                let mut opts: Vec<(String, &str)> = vec![("-1".to_string(), tl!("None"))];
                opts.extend(names.iter().enumerate().map(|(i, n)| (i.to_string(), n.as_str())));
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(label(&p.key)).color(t.text_dim));
                    crate::widgets::dropdown(ui, &format!("flt-{cmd}-{}", p.key), &mut sel, &opts, 170.0);
                });
                match sel.parse::<i64>() {
                    Ok(i) if i >= 0 => f.insert(p.key, json!(i)),
                    _ => f.insert(p.key, json!(-1)),
                };
            }
            Kind::Grid(n) => {
                let side = (n as f32).sqrt().round().max(1.0) as usize;
                let mut vals: Vec<f32> =
                    f.get(&p.key).and_then(Value::as_array).map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect()).unwrap_or_default();
                vals.resize(n, 0.0);
                ui.label(egui::RichText::new(label(&p.key)).color(t.text_dim));
                egui::Grid::new(format!("flt-grid-{cmd}-{}", p.key)).spacing([4.0, 4.0]).show(ui, |ui| {
                    for (i, v) in vals.iter_mut().enumerate() {
                        crate::widgets::value_field(ui, v, -999.0..=999.0, "", 44.0);
                        if (i + 1) % side == 0 {
                            ui.end_row();
                        }
                    }
                });
                f.insert(p.key, json!(vals.iter().map(|v| v.round() as i64).collect::<Vec<_>>()));
            }
            Kind::Json => {}
            Kind::Color(default) => {
                let set = f.get(&p.key).and_then(Value::as_str).and_then(crate::color_picker_ui::parse_hex);
                let rgb = set.or(default).unwrap_or([0.0; 3]);
                let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                let bytes = rgb.map(q);
                let name = label(&p.key);
                let mut clicked = false;
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&name).color(t.text_dim));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // PhotoCraft's Color Picker, as everywhere else (#2144).
                        clicked = crate::widgets::color_swatch_button(ui, egui::Color32::from_rgb(bytes[0], bytes[1], bytes[2]), &name).clicked();
                        ui.label(egui::RichText::new(crate::color_picker_ui::hex(rgb)).color(t.text_dim).monospace());
                    });
                });
                if set.is_none() && default.is_some() {
                    f.insert(p.key.clone(), json!(crate::color_picker_ui::hex(rgb)));
                }
                if clicked {
                    if !f.contains_key(&p.key) {
                        f.insert(p.key.clone(), json!(crate::color_picker_ui::hex(rgb)));
                    }
                    crate::color_picker_ui::request_field(f, &p.key, &crate::color_picker_ui::title_for(&name));
                }
            }
            Kind::Int { default } => {
                let mut v = f.get(&p.key).and_then(Value::as_f64).unwrap_or(default as f64) as f32;
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(label(&p.key)).color(t.text_dim));
                    crate::widgets::value_field(ui, &mut v, -30000.0..=30000.0, "", 80.0);
                });
                f.insert(p.key, json!(v.round() as i64));
            }
        }
    }
    // Read-only context the dialog opener supplies (e.g. the monitor profile in use).
    if let Some(note) = f.get("__note").and_then(Value::as_str) {
        ui.add_space(4.0);
        ui.add(egui::Label::new(egui::RichText::new(note).color(t.text_dim)).wrap());
    }
    if let Some(mut preview) = f.get("__preview").and_then(Value::as_bool) {
        ui.add_space(4.0);
        crate::widgets::checkbox(ui, &mut preview, tl!("Preview"));
        f.insert("__preview".into(), json!(preview));
    }
}

/// User-facing params (strip the dialog's private `__` keys).
pub fn params_of(f: &Map<String, Value>) -> Value {
    // A document picker left at "None" (-1) means "not given".
    let unset = |k: &str, v: &Value| k == "mapDocument" && v.as_i64() == Some(-1);
    Value::Object(f.iter().filter(|(k, v)| !k.starts_with("__") && !unset(k, v)).map(|(k, v)| (k.clone(), v.clone())).collect())
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
    // Warp Text renders the type again from the layer model (font size, position), which a
    // reduced proxy doesn't scale: it runs at full size and the result is reduced (#218).
    let (run_k, reduce_k) = if command == "type.warpText" { (1, k) } else { (k, 1) };
    let proxy = crate::proxy::proxy_document(doc, run_k);
    let mut s = photocraft_engine::Session::new();
    s.set_inline_job_ctx(cancel.cloned());
    s.add_document(proxy, None);
    if let Some(id) = active {
        s.select_layer(id).ok()?;
    }
    let mut p = params.clone();
    if run_k > 1
        && let Some(o) = p.as_object_mut()
    {
        let spec = photocraft_engine::commands::find(command).map(|c| parse_spec(c.params)).unwrap_or_default();
        for (key, v) in o.iter_mut() {
            if is_pixel_param_of(command, key)
                && let Some(x) = v.as_f64()
            {
                // Match the engine's minimum after scaling (e.g. Box Blur 1, Mosaic 2).
                // Previously its tolerant decoder clamped these; new calls validate first.
                let minimum = spec.iter().find(|p| p.key == key.as_str()).and_then(|p| match &p.kind {
                    Kind::Range { min, .. } => Some(f64::from(*min)),
                    _ => None,
                });
                let floor = minimum.unwrap_or(0.0).max(if key == "cellSize" { 1.0 } else { 0.1 });
                // Negative lengths (Offset's left/up shift, Displace's inverted scale) keep their
                // sign: the floor that keeps a radius positive turned them into +0.1 (#2063).
                let x = x / run_k as f64;
                *v = json!(if x > 0.0 { x.max(floor) } else { minimum.map_or(x, |m| x.max(m)) });
            }
        }
    }
    s.execute(command, p).ok()?;
    s.active().map(|d| crate::proxy::proxy_document(&d.doc, reduce_k))
}

/// Cached preview state on the app.
pub struct FilterPreview {
    pub key: FilterPreviewKey,
    pub result: Option<Arc<Document>>,
    /// OK was pressed and the filter runs as a background job: the preview stays on screen until
    /// the job lands, so the canvas doesn't flash the unfiltered image in between.
    pub committing: bool,
}

/// Match the full request before accepting a worker result, including a reopened dialog.
#[derive(Clone, Debug, PartialEq)]
pub struct FilterPreviewKey {
    pub doc: photocraft_doc::DocId,
    pub revision: u64,
    pub dialog: u64,
    pub active: Option<photocraft_doc::LayerId>,
    pub command: String,
    pub params: Value,
    pub k: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restores_only_valid_previous_filter_choices() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.prefs.edit(|prefs| {
            prefs.dialogs.insert("filter.blur.gaussianBlur".into(), json!({"radius": 11.0, "bogus": 42}));
        });
        open(&mut app, "filter.blur.gaussianBlur").unwrap();
        let fields = &app.ui.dialogs.last().unwrap().fields;
        assert_eq!(fields["radius"], json!(11.0));
        assert!(!fields.contains_key("bogus"));

        // A changed registry range, corrupt preference or stale path is never restored.
        let mut fields = Map::new();
        fields.insert("radius".into(), json!(2.0));
        restore_remembered(&app, "filter.blur.gaussianBlur", r#"{"radius":0.1..5=1}"#, &mut fields);
        assert_eq!(fields["radius"], json!(2.0));
    }

    #[test]
    fn remembering_dialogs_never_persists_paths_or_document_indices() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let mut fields = Map::new();
        fields.insert("__filter".into(), json!(true));
        fields.insert("radius".into(), json!(7.0));
        fields.insert("mapPath".into(), json!("/private/file"));
        fields.insert("document".into(), json!(5));
        remember(&mut app, "filter.blur.gaussianBlur", &fields);
        let saved = &app.session.prefs().dialogs["filter.blur.gaussianBlur"];
        assert_eq!(saved["radius"], json!(7.0));
        assert!(saved.get("mapPath").is_none());
        assert!(saved.get("document").is_none());
    }

    #[test]
    fn parses_registry_notation() {
        let p = parse_spec(r#"{"radius":0.1..1000=1,"method":"spin|zoom","monochromatic":bool,"seed":u32=0,"horizontal":px=0}"#);
        assert_eq!(p[0], Param { key: "radius".into(), kind: Kind::Range { min: 0.1, max: 1000.0, default: 1.0 } });
        assert_eq!(p[1].kind, Kind::Choice(vec!["spin".into(), "zoom".into()]));
        assert_eq!(p[2].kind, Kind::Bool(false));
        assert_eq!(p[3].kind, Kind::Int { default: 0 });
        assert_eq!(p[4].kind, Kind::Int { default: 0 });
        assert!(parse_spec("{}").is_empty());
        let p = parse_spec(r#"{"lighting":bool=true,"kernel":int[25],"pins":json,"points":[[0,0],[1,0]],"mapPath":text,"mapDocument":doc,"scale":1..9999=1}"#);
        let kinds: Vec<&Kind> = p.iter().map(|p| &p.kind).collect();
        assert_eq!(
            kinds,
            [&Kind::Bool(true), &Kind::Grid(25), &Kind::Json, &Kind::Json, &Kind::Text, &Kind::Document, &Kind::Range { min: 1.0, max: 9999.0, default: 1.0 }]
        );
    }

    /// A layer-id param has no number field: drawing the Auto-Align dialog used to write
    /// `reference: 0` back, which the engine rejects as not a selected layer (#674).
    #[test]
    fn layer_id_params_stay_out_of_the_dialog() {
        let mut f = Map::new();
        f.insert("__command".into(), json!("edit.autoAlignLayers"));
        egui::Context::default().run_ui(Default::default(), |ui| body(ui, &mut f)).textures_delta.clear();
        assert!(!params_of(&f).as_object().unwrap().contains_key("reference"), "{f:?}");
    }

    #[test]
    fn indexed_counts_are_integer_json_while_dither_amounts_keep_their_decimals() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        open(&mut app, "image.mode.indexedColor").unwrap();
        let mut defaults = app.ui.dialogs.first().unwrap().fields.clone();
        let before = params_of(&defaults)["colors"].clone();
        egui::Context::default().run_ui(Default::default(), |ui| body(ui, &mut defaults)).textures_delta.clear();
        assert_eq!(before.as_u64(), Some(256));
        assert_eq!(params_of(&defaults)["colors"], before, "opening a preview must not schedule a second job just to normalize its count");
        let mut fields = Map::new();
        fields.insert("__command".into(), json!("image.mode.indexedColor"));
        fields.insert("colors".into(), json!(17));
        fields.insert("amount".into(), json!(37.5));
        egui::Context::default().run_ui(Default::default(), |ui| body(ui, &mut fields)).textures_delta.clear();
        let params = params_of(&fields);
        assert_eq!(params["colors"].as_u64(), Some(17));
        assert_eq!(params["amount"].as_f64(), Some(37.5));
    }

    #[test]
    fn wide_positive_ranges_use_the_logarithmic_slider_path() {
        assert!(uses_logarithmic_slider(0.1, 1000.0), "Gaussian Blur radius");
        assert!(uses_logarithmic_slider(1.0, 9999.0));
        assert!(!uses_logarithmic_slider(1.0, 500.0));
        assert!(!uses_logarithmic_slider(0.0, 1000.0));
    }

    #[test]
    fn new_filters_have_dialogs_or_run_directly() {
        for id in [
            "filter.stylize.oilPaint",
            "filter.blur.lensBlur",
            "filter.blurGallery.irisBlur",
            "filter.other.custom",
            "filter.distort.displace",
            "filter.pixelate.mezzotint",
            "filter.render.lightingEffects",
            "filter.render.relight",
            "filter.other.colorToAlpha",
        ] {
            assert!(has_dialog(id), "{id}");
        }
        for id in ["filter.pixelate.facet", "filter.pixelate.fragment", "filter.video.ntscColors"] {
            assert!(!has_dialog(id), "{id}");
        }
    }

    #[test]
    fn every_filter_command_spec_parses() {
        for c in photocraft_engine::command_specs().iter().filter(|c| c.id.starts_with("filter.")) {
            let _ = parse_spec(c.params);
        }
        assert!(has_dialog("filter.blur.gaussianBlur"));
        assert!(!has_dialog("filter.stylize.findEdges"));
    }

    #[test]
    fn korean_covers_generated_filter_options_and_gallery_names() {
        let ko = crate::i18n::lang_from_tag("ko-KR").unwrap();
        let mut missing = std::collections::BTreeSet::new();
        let mut check = |s: String| {
            if !matches!(s.as_str(), "X" | "Y" | "A" | "B") && !crate::i18n::has(ko, &s) {
                missing.insert(s);
            }
        };
        for c in photocraft_engine::command_specs().iter().filter(|c| c.id.starts_with("filter.") || c.id.starts_with("image.adjustments.")) {
            for p in parse_spec(c.params) {
                if !p.key.chars().all(|c| c.is_ascii_alphanumeric()) || matches!(p.kind, Kind::Json) {
                    continue;
                }
                check(source_label(&p.key));
                if let Kind::Choice(choices) = p.kind {
                    for choice in choices {
                        // Only symbolic options are UI choices; registry docs also contain
                        // colour syntax and array notation, which are not translatable names.
                        if choice.chars().all(|c| c.is_ascii_alphanumeric()) {
                            check(source_label(&choice));
                        }
                    }
                }
            }
        }
        for f in photocraft_algo::GalleryFilter::ALL {
            check(f.name().to_string());
        }
        for cat in photocraft_algo::GALLERY_CATEGORIES {
            check(cat.to_string());
        }
        assert!(missing.is_empty(), "missing Korean dynamic labels: {missing:#?}");
    }

    #[test]
    fn preview_runs_engine_command_on_proxy() {
        let mut doc = Document::with_background(
            "p",
            photocraft_doc::Size::new(64, 64),
            photocraft_doc::ColorMode::Rgb,
            photocraft_doc::SampleType::U8,
            photocraft_doc::Color::WHITE,
        );
        let bg = doc.layers[0].id;
        doc.layers[0].surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 32, 64), &[0.0, 0.0, 0.0, 1.0]);
        let out = preview_document(&doc, Some(bg), "filter.blur.gaussianBlur", &json!({"radius": 4.0}), 1).unwrap();
        let p = out.layers[0].surface().unwrap().pixel(32, 32);
        assert!(p[0] > 0.2 && p[0] < 0.8, "edge blurred: {p:?}");
        assert_eq!(label("wavelengthMin"), "Wavelength Min");
    }

    #[test]
    fn core_filter_previews_keep_minimum_settings_after_downsampling() {
        use photocraft_doc::{Color, ColorMode, SampleType, Size};
        use photocraft_geom::Rect;

        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut doc = Document::with_background("preview", Size::new(32, 32), ColorMode::Rgb, depth, Color::WHITE);
            let layer = doc.layers[0].id;
            doc.layers[0].surface_mut().unwrap().fill_rect(Rect::new(0, 0, 16, 32), &[0.0, 0.0, 0.0, 1.0]);
            for (suffix, params) in [
                ("blur.boxBlur", json!({"radius":1})),
                ("blur.motionBlur", json!({"distance":1})),
                ("blur.surfaceBlur", json!({"radius":1})),
                ("noise.median", json!({"radius":1})),
                ("noise.dustAndScratches", json!({"radius":1})),
                ("pixelate.mosaic", json!({"cellSize":2})),
                ("stylize.emboss", json!({"height":1})),
                ("other.minimum", json!({"radius":0.2})),
                ("other.maximum", json!({"radius":0.2})),
                ("distort.wave", json!({"wavelengthMin":1,"wavelengthMax":2,"amplitudeMin":1,"amplitudeMax":1})),
            ] {
                let command = format!("filter.{suffix}");
                for k in [2, 4] {
                    let proxy = crate::proxy::proxy_document(&doc, k);
                    let expected = preview_document(&proxy, Some(layer), &command, &params, 1).unwrap();
                    let out = preview_document(&doc, Some(layer), &command, &params, k).unwrap_or_else(|| panic!("{command}: {depth:?}, proxy factor {k}"));
                    assert_eq!(
                        out.layers[0].surface().unwrap().read_region(proxy.bounds()),
                        expected.layers[0].surface().unwrap().read_region(proxy.bounds()),
                        "{command}: {depth:?}, proxy factor {k}"
                    );
                }
            }
        }
    }

    /// A serde type error from numbers the dialog sends as JSON floats (`1.0`) for an integer
    /// field ("invalid type: floating point `1.0`, expected u32"), as opposed to a legitimate
    /// refusal such as "no selection" or "needs a map".
    fn float_type_error(e: &str) -> bool {
        e.contains("invalid type: floating point")
    }

    /// #2501: every schema dialog, opened with its default fields and drawn once, runs its command
    /// (OK) and its live preview without a type error from integer fields sent as floats.
    #[test]
    fn every_dialog_default_runs_without_float_type_errors() {
        let ids: Vec<&str> = photocraft_engine::command_specs().iter().map(|c| c.id).filter(|id| has_dialog(id)).collect();
        assert!(ids.len() > 100, "{} dialogs", ids.len());
        let mut failures = Vec::new();
        for id in ids {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width": 24, "height": 24})).unwrap();
            app.run("layer.new.layer", json!({})).unwrap();
            let Some(dialog) = open(&mut app, id) else { continue };
            let mut f = app.ui.dialogs.iter().find(|d| d.id == dialog).unwrap().fields.clone();
            egui::Context::default().run_ui(Default::default(), |ui| body(ui, &mut f)).textures_delta.clear();
            let params = params_of(&f);
            let st = app.session.active().unwrap();
            let (doc, active) = ((*st.doc).clone(), st.active_layer);
            if let Err(e) = app.session.execute(id, params.clone()) {
                let e = e.to_string();
                if float_type_error(&e) {
                    failures.push(format!("{id} OK {params}: {e}"));
                }
            }
            // The preview swallows the error; rerun its params at full size to see it.
            if preview_document(&doc, active, id, &params, 1).is_none() {
                let mut s = photocraft_engine::Session::new();
                s.add_document(doc, None);
                if let Some(a) = active {
                    s.select_layer(a).unwrap();
                }
                if let Err(e) = s.execute(id, params.clone()).map_err(|e| e.to_string())
                    && float_type_error(&e)
                {
                    failures.push(format!("{id} preview {params}: {e}"));
                }
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    #[test]
    fn colour_params_parse_with_and_without_a_default() {
        let p = parse_spec(r##"{"color":color=#ffffff,"vineColor":color,"bad":color=#zz}"##);
        assert_eq!(p[0].kind, Kind::Color(Some([1.0, 1.0, 1.0])));
        assert_eq!(p[1].kind, Kind::Color(None));
        assert_eq!(p[2].kind, Kind::Color(None));
    }

    /// Color to Alpha (#1576): the dialog starts at white and the GIMP-style thresholds, drawing it
    /// keeps those params, and its live preview makes the white of a Background transparent.
    #[test]
    fn color_to_alpha_dialog_defaults_draw_and_preview() {
        const ID: &str = "filter.other.colorToAlpha";
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        let id = open(&mut app, ID).unwrap();
        let mut f = app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.clone();
        assert_eq!(f.get("__preview"), Some(&json!(true)));
        let want = json!({"color": "#ffffff", "transparencyThreshold": 0.0, "opacityThreshold": 1.0});
        assert_eq!(params_of(&f), want);
        egui::Context::default().run_ui(Default::default(), |ui| body(ui, &mut f)).textures_delta.clear();
        assert_eq!(params_of(&f), want, "drawing the dialog keeps its params");

        let mut doc = Document::with_background(
            "p",
            photocraft_doc::Size::new(32, 32),
            photocraft_doc::ColorMode::Rgb,
            photocraft_doc::SampleType::U16,
            photocraft_doc::Color::WHITE,
        );
        let bg = doc.layers[0].id;
        doc.layers[0].surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 8, 32), &[0.0, 0.0, 0.0, 1.0]);
        for k in [1, 2] {
            let out = preview_document(&doc, Some(bg), ID, &want, k).unwrap();
            let s = out.layers[0].surface().unwrap();
            assert_eq!(out.layers[0].name, "Layer 0", "k={k}");
            assert!(s.pixel(24 / k as i32, 4)[3] < 1e-3, "k={k}: white removed");
            assert_eq!(s.pixel(1, 4)[3], 1.0, "k={k}: black kept");
        }
    }

    #[test]
    fn preview_stays_inside_the_selection_at_every_proxy_factor() {
        use photocraft_doc::SampleType;
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut doc =
                Document::with_background("p", photocraft_doc::Size::new(64, 64), photocraft_doc::ColorMode::Rgb, depth, photocraft_doc::Color::gray(0.5));
            let bg = doc.layers[0].id;
            let mut sel = photocraft_raster::Surface::new(photocraft_doc::PixelFormat::GRAY8);
            sel.fill_rect(photocraft_geom::Rect::new(0, 0, 32, 64), &[1.0]);
            doc.selection = Some(sel);
            for k in [1, 2, 4] {
                let out = preview_document(&doc, Some(bg), "filter.noise.addNoise", &json!({"amount": 100.0}), k).unwrap();
                let s = out.layers[0].surface().unwrap();
                let half = 32 / k as i32;
                let changed = |x0: i32, x1: i32| (0..64 / k as i32).any(|y| (x0..x1).any(|x| s.pixel(x, y) != doc.layers[0].surface().unwrap().pixel(0, 0)));
                assert!(changed(0, half), "{depth:?} k={k}: noise inside the selection");
                assert!(!changed(half, 64 / k as i32), "{depth:?} k={k}: nothing outside the selection");
            }
        }
    }

    #[test]
    fn define_pattern_asks_for_a_name() {
        // Edit › Define Pattern… named the pattern after the document without asking ("ask", then
        // "ask 2"). Photoshop's Pattern Name dialog starts from that name and lets you change it.
        let spec = photocraft_engine::commands::find("edit.definePattern").unwrap();
        assert_eq!(parse_spec(spec.params), vec![Param { key: "name".into(), kind: Kind::Text }, Param { key: "rect".into(), kind: Kind::Json }]);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 8, "height": 8, "name": "ask.jpg"})).unwrap();
        let ctx = egui::Context::default();
        let define = |app: &mut PhotocraftApp, name: Option<&str>| {
            crate::menus::invoke(app, &ctx, "edit.definePattern", Value::Null).unwrap();
            let d = app.ui.dialogs.last().expect("Define Pattern opens a dialog").clone();
            let shown = d.fields["name"].as_str().unwrap().to_string();
            if let Some(n) = name {
                app.ui.dialog_mut(d.id).unwrap().fields.insert("name".into(), json!(n));
            }
            crate::dialogs::confirm(app, d.id).unwrap();
            shown
        };
        assert_eq!(define(&mut app, Some("Bricks")), "ask", "prefilled with the document's name");
        assert_eq!(define(&mut app, None), "ask", "Bricks didn't take it");
        assert_eq!(define(&mut app, None), "ask 2", "numbered past the library");
        let names: Vec<&str> = app.session.patterns.items.iter().map(|p| p.name.as_str()).collect();
        for n in ["Bricks", "ask", "ask 2"] {
            assert!(names.contains(&n), "{n} in {names:?}");
        }
    }

    #[test]
    fn notes_after_a_spec_are_not_parameters() {
        // A note after the object, like Selective Color's "(per-range [c,m,y,k] arrays: …)", was
        // parsed as parameters: its commas split off junk fields and it glued itself onto the last
        // one, so "reds":json became a number field the dialog showed and sent.
        let p = parse_spec(r#"{"a":0..10=1,"b":json} (notes [x,y,z] here, and more) → {"c":id}"#);
        assert_eq!(p, vec![Param { key: "a".into(), kind: Kind::Range { min: 0.0, max: 10.0, default: 1.0 } }, Param { key: "b".into(), kind: Kind::Json }]);
        // Every command with a schema dialog has plain parameter keys.
        for c in photocraft_engine::command_specs().iter().filter(|c| has_dialog(c.id)) {
            for p in parse_spec(c.params) {
                assert!(p.key.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_'), "{}: junk parameter {:?}", c.id, p.key);
            }
        }
    }

    #[test]
    fn selective_color_dialog_ok_applies() {
        // With the junk `reds: 0` the dialog's OK failed (`reds` must be an array of 4 numbers).
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        let id = open(&mut app, "image.adjustments.selectiveColor").unwrap();
        let fields = app.ui.dialogs.last().unwrap().fields.clone();
        assert!(fields.keys().all(|k| k.starts_with("__") || k.chars().all(|ch| ch.is_ascii_alphanumeric())), "{fields:?}");
        assert!(!fields.contains_key("reds"), "the per-range arrays are not dialog fields");
        app.ui.dialog_mut(id).unwrap().fields.insert("black".into(), json!(40.0));
        crate::dialogs::confirm(&mut app, id).unwrap();
        assert_eq!(app.session.journal.last().map(|j| j.0.as_str()), Some("image.adjustments.selectiveColor"));
    }

    /// A filtered test image with structure at every scale (low-frequency waves plus a ramp).
    fn waves(n: u32) -> Document {
        let mut doc = Document::with_background(
            "w",
            photocraft_doc::Size::new(n, n),
            photocraft_doc::ColorMode::Rgb,
            photocraft_doc::SampleType::U8,
            photocraft_doc::Color::WHITE,
        );
        let s = doc.layers[0].surface_mut().unwrap();
        for y in 0..n as i32 {
            for x in 0..n as i32 {
                let v = 0.5 + 0.4 * (x as f32 / 23.0).sin() * (y as f32 / 31.0).cos();
                s.fill_rect(photocraft_geom::Rect::new(x, y, x + 1, y + 1), &[v, 1.0 - v, x as f32 / n as f32, 1.0]);
            }
        }
        doc
    }

    /// Mean absolute RGB difference of two documents, compared at 1/4 scale.
    fn diff_at_quarter(a: &Document, b: &Document) -> f32 {
        let quarter = |d: &Document| photocraft_compose::flatten(&crate::proxy::proxy_document(d, (d.size.width / 64).max(1)));
        let (a, b) = (quarter(a), quarter(b));
        assert_eq!(a.px.len(), b.px.len());
        a.px.iter().zip(&b.px).map(|(p, q)| (0..3).map(|c| (p[c] - q[c]).abs()).sum::<f32>() / 3.0).sum::<f32>() / a.px.len() as f32
    }

    /// #2063: these filters drew their features in proxy pixels (noise, dots, dither, strokes,
    /// kernels, scan lines, contour lines, streaks, depths, a tree's segments), so a 1/4 preview
    /// showed them 4x larger than OK applied them. Their preview now runs at full resolution and
    /// matches the applied result.
    #[test]
    fn fixed_size_filters_preview_at_the_scale_they_apply() {
        let doc = sharp(256);
        let gray = grayscale(&doc);
        let edge = json!([0, 0, 0, 0, 0, 0, 0, -1, 0, 0, 0, -1, 4, -1, 0, 0, 0, -1, 0, 0, 0, 0, 0, 0, 0]);
        for (cmd, p) in [
            ("filter.distort.ripple", json!({"amount": 300.0, "size": "large"})),
            ("filter.stylize.diffuse", json!({"mode": "normal"})),
            ("filter.stylize.oilPaint", json!({})),
            ("filter.render.flame", json!({})),
            ("filter.render.fibers", json!({})),
            ("filter.pixelate.mezzotint", json!({"type": "mediumDots"})),
            ("filter.noise.addNoise", json!({"amount": 12.5})),
            ("filter.noise.reduceNoise", json!({})),
            ("filter.other.custom", json!({"kernel": edge})),
            ("filter.video.deInterlace", json!({})),
            ("filter.stylize.traceContour", json!({})),
            ("filter.stylize.wind", json!({"method": "blast"})),
            ("filter.stylize.extrude", json!({})),
            ("filter.render.tree", json!({"baseTreeType": 1})),
            ("filter.gallery.graphicPen", json!({})),
            ("filter.gallery.stainedGlass", json!({"cellSize": 10})),
            ("image.mode.indexedColor", json!({"colors": 16})),
            ("image.mode.bitmap", json!({"method": "diffusion"})),
        ] {
            let doc = if cmd == "image.mode.bitmap" { &gray } else { &doc };
            let bg = doc.layers[0].id;
            let applied = preview_document(doc, Some(bg), cmd, &p, 1).unwrap();
            let old = preview_document(doc, Some(bg), cmd, &p, 4).unwrap();
            assert!(diff_at_quarter(&applied, &old) > 0.005, "{cmd}: a 1/4 proxy differs from the result");
            let k = preview_factor(cmd, &p, 4);
            assert_eq!(k, 1, "{cmd}");
            let shown = preview_document(doc, Some(bg), cmd, &p, k).unwrap();
            assert!(diff_at_quarter(&applied, &shown) < 1e-4, "{cmd}: the preview is what OK applies");
        }
        // Without dithering, the colour-mode conversions keep the proxy.
        assert_eq!(preview_factor("image.mode.indexedColor", &json!({"dither": "none"}), 4), 4);
        assert_eq!(preview_factor("image.mode.bitmap", &json!({"method": "threshold"}), 4), 4);
    }

    /// #2063: proxy previews scaled what isn't a length (Tree's canvas fraction, Lens Correction's
    /// perspective), clamped negative lengths to +0.1 (Offset left/up), left Shadows/Highlights'
    /// radii in document pixels and rounded Mosaic's 10 px cells to 3 proxy px (12 px).
    #[test]
    fn proxy_previews_scale_only_lengths() {
        let doc = waves(256);
        let bg = doc.layers[0].id;
        let error = |cmd: &str, p: Value| {
            let k = preview_factor(cmd, &p, 4);
            let applied = preview_document(&doc, Some(bg), cmd, &p, 1).unwrap();
            let shown = preview_document(&doc, Some(bg), cmd, &p, k).unwrap();
            (k, mean_abs(&every_kth(&applied, k), &every_kth(&shown, 1)))
        };
        for (cmd, p) in [
            ("filter.lensCorrection", json!({"vertical": -100.0, "horizontal": 60.0})),
            ("filter.other.offset", json!({"horizontal": -40, "vertical": -24, "undefinedAreas": "wrap"})),
            (
                "image.adjustments.shadowsHighlights",
                json!({"shadowAmount": 50.0, "shadowRadius": 30.0, "highlightAmount": 50.0, "highlightRadius": 30.0, "blackClip": 0.0, "whiteClip": 0.0}),
            ),
        ] {
            let (k, diff) = error(cmd, p);
            assert_eq!(k, 4, "{cmd} keeps the proxy");
            assert!(diff < 0.01, "{cmd}: the preview shows what OK applies ({diff})");
        }
        // Mosaic's cells divide evenly: 10 px at k = 2 is 5 proxy px; 40 px keeps k = 4.
        assert_eq!(preview_factor("filter.pixelate.mosaic", &json!({"cellSize": 10.0}), 4), 2);
        assert_eq!(preview_factor("filter.pixelate.mosaic", &json!({"cellSize": 40.0}), 4), 4);
        assert_eq!(preview_factor("filter.pixelate.mosaic", &json!({"cellSize": 7.0}), 4), 1);
        assert_eq!(preview_factor("filter.pixelate.mosaic", &json!({"cellSize": "x"}), 4), 4);
        let (k, diff) = error("filter.pixelate.mosaic", json!({"cellSize": 10.0}));
        assert!(k == 2 && diff < 0.01, "Mosaic: {diff}");
        // Tree's and Picture Frame's sizes are relative, not pixels (and not shown as "px").
        assert!(!is_pixel_param_of("filter.render.tree", "size") && !is_pixel_param_of("filter.render.pictureFrame", "size"));
        assert!(is_pixel_param_of("filter.stylize.extrude", "size"));
    }

    /// #218: Warp Text renders the type again from the layer model, which a reduced proxy doesn't
    /// scale: the preview showed the text k times too large. It is the full-size result, reduced.
    #[test]
    fn warp_text_preview_matches_the_full_size_result_on_a_reduced_proxy() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 4000, "height": 3000})).unwrap();
        s.execute("type.create", json!({"x": 300, "y": 1600, "text": "Warp", "size": 400})).unwrap();
        let st = s.active().unwrap();
        let (doc, layer) = ((*st.doc).clone(), st.active_layer.unwrap());
        let p = json!({"style": "arc", "bend": 40.0});
        let k = preview_factor("type.warpText", &p, crate::proxy::preview_factor(&doc, true));
        assert!(k > 1, "the preview runs on a reduced proxy");
        let cache = |d: &Document| match &d.layer(layer).unwrap().content {
            photocraft_doc::LayerContent::Text(t) => t.cache.clone().unwrap(),
            _ => panic!("a type layer"),
        };
        let full = crate::proxy::proxy_document(&preview_document(&doc, Some(layer), "type.warpText", &p, 1).unwrap(), k);
        let shown = preview_document(&doc, Some(layer), "type.warpText", &p, k).unwrap();
        assert_eq!(shown.size, full.size);
        assert_eq!(cache(&shown).content_bounds(), cache(&full).content_bounds());
        assert_eq!(photocraft_compose::flatten(&shown).px, photocraft_compose::flatten(&full).px);
    }

    /// #2063: a pixel-sized parameter divided by the proxy factor was clamped to the command's
    /// minimum (Pointillize's 5 px cells became 3 px proxy cells = 12 px), so the preview factor
    /// drops until every pixel-sized value fits its range.
    #[test]
    fn proxy_previews_never_clamp_pixel_sized_params() {
        let pointillize = |c: f64| preview_factor("filter.pixelate.pointillize", &json!({"cellSize": c}), 4);
        assert_eq!((pointillize(5.0), pointillize(7.0), pointillize(12.0), pointillize(300.0)), (1, 2, 4, 4));
        assert_eq!(preview_factor("filter.pixelate.crystallize", &json!({"cellSize": 10.0}), 4), 3);
        assert_eq!(preview_factor("filter.pixelate.colorHalftone", &json!({"maxRadius": 8.0}), 4), 2);
        assert_eq!(preview_factor("filter.blur.gaussianBlur", &json!({"radius": 8.0}), 4), 4, "unclamped filters keep the proxy");
        // Hostile params never panic and leave the factor alone; a full-size document stays at 1.
        assert_eq!(preview_factor("filter.pixelate.pointillize", &json!({"cellSize": "x"}), 4), 4);
        assert_eq!(preview_factor("filter.pixelate.pointillize", &json!(null), 4), 4);
        assert_eq!(preview_factor("no.such.command", &json!({"cellSize": 1.0}), 4), 4);
        assert_eq!(preview_factor("filter.pixelate.pointillize", &json!({"cellSize": 5.0}), 0), 1);
        // Every previewed command, at each pixel-sized param's minimum, default and in between.
        let previewed = photocraft_engine::command_specs().iter().filter(|c| has_dialog(c.id) && (c.id.starts_with("filter.") || PREVIEWED.contains(&c.id)));
        for c in previewed {
            for p in parse_spec(c.params) {
                let Kind::Range { min, default, .. } = p.kind else { continue };
                if min <= 0.0 || !is_pixel_param_of(c.id, &p.key) {
                    continue;
                }
                for v in [f64::from(min), f64::from(default), f64::from(min) * 2.5] {
                    let k = preview_factor(c.id, &json!({ p.key.clone(): v }), 4);
                    assert!(v / f64::from(k) >= f64::from(min) - 1e-9, "{} {}={v}: k={k} clamps to {min}", c.id, p.key);
                }
            }
        }
    }

    /// Sharp structure at every scale: [`waves`] with hard squares from 2 to 32 px, a fine
    /// per-pixel texture and an interlaced band (odd rows inverted) at the bottom.
    fn sharp(n: u32) -> Document {
        let mut doc = waves(n);
        let s = doc.layers[0].surface_mut().unwrap();
        let mut x0 = 4;
        for (i, size) in [2, 3, 4, 6, 8, 12, 16, 24, 32].into_iter().enumerate() {
            for y0 in (4..n as i32 * 3 / 4 - size).step_by((size * 3) as usize) {
                let v = if (i + y0 as usize / 7).is_multiple_of(2) { 0.05 } else { 0.95 };
                s.fill_rect(photocraft_geom::Rect::new(x0, y0, x0 + size, y0 + size), &[v, v * 0.8, 1.0 - v, 1.0]);
            }
            x0 += size + 6;
        }
        for y in 0..n as i32 {
            for x in 0..n as i32 {
                let mut px = s.pixel(x, y);
                if (x * 7 + y * 13) % 5 == 0 {
                    px.iter_mut().take(3).for_each(|c| *c = (*c + 0.08).min(1.0));
                }
                if y >= n as i32 * 3 / 4 && y % 2 == 1 {
                    px[0] = 1.0 - px[0];
                }
                s.fill_rect(photocraft_geom::Rect::new(x, y, x + 1, y + 1), &px);
            }
        }
        doc
    }

    /// [`waves`] with strong per-pixel noise, the input of noise reduction.
    fn noisy(n: u32) -> Document {
        let mut doc = waves(n);
        let s = doc.layers[0].surface_mut().unwrap();
        let hash = |x: i32, y: i32| ((x as u32).wrapping_mul(73_856_093) ^ (y as u32).wrapping_mul(19_349_663)).wrapping_mul(2_654_435_761) >> 8;
        for y in 0..n as i32 {
            for x in 0..n as i32 {
                let mut px = s.pixel(x, y);
                for (c, v) in px.iter_mut().take(3).enumerate() {
                    *v = (*v + (hash(x * 3 + c as i32, y) as f32 / (1u32 << 24) as f32 - 0.5) * 0.3).clamp(0.0, 1.0);
                }
                s.fill_rect(photocraft_geom::Rect::new(x, y, x + 1, y + 1), &px);
            }
        }
        doc
    }

    /// How a preview at factor `k` compares with what OK applies (factor 1) on a document.
    #[derive(Debug)]
    struct PreviewError {
        /// Mean absolute RGB difference at the pixels the proxy samples.
        diff: f32,
        /// Feature scale of the preview shown at document size, relative to the result: 1 when
        /// the features match, 1/k when the preview draws them k times too large.
        scale: f32,
        /// How much the preview changes the image relative to how much OK does (sampled); NaN
        /// when OK barely changes it.
        change: f32,
    }

    /// Mean absolute difference of neighbouring pixels at the document's own resolution.
    fn detail(d: &Document) -> f32 {
        let b = photocraft_compose::flatten(d);
        let w = b.rect.width() as usize;
        let mut sum = 0.0;
        for (i, p) in b.px.iter().enumerate() {
            for q in [b.px.get(i + 1).filter(|_| (i + 1) % w != 0), b.px.get(i + w)].into_iter().flatten() {
                sum += (0..3).map(|c| (p[c] - q[c]).abs()).sum::<f32>() / 3.0;
            }
        }
        sum / b.px.len().max(1) as f32
    }

    /// Pixels of `d` at every `k`-th row and column: the document pixels a factor-`k` proxy
    /// samples, so a preview at `k` compares with them one to one.
    fn every_kth(d: &Document, k: u32) -> Vec<[f32; 4]> {
        let b = photocraft_compose::flatten(d);
        let w = b.rect.width() as usize;
        let k = k as usize;
        let h = b.px.len() / w.max(1);
        (0..h.div_ceil(k))
            .flat_map(|y| (0..w.div_ceil(k)).map(move |x| (x * k, y * k)))
            .map(|(x, y)| b.px.get(y * w + x).copied().unwrap_or([f32::NAN; 4]))
            .collect()
    }

    fn mean_abs(a: &[[f32; 4]], b: &[[f32; 4]]) -> f32 {
        a.iter().zip(b).map(|(p, q)| (0..3).map(|c| (p[c] - q[c]).abs()).sum::<f32>() / 3.0).sum::<f32>() / a.len().max(1) as f32
    }

    /// `None` when the command can't run on `doc` at all (e.g. Displace without a map).
    fn preview_error(doc: &Document, command: &str, p: &Value, k: u32) -> Option<PreviewError> {
        let bg = doc.layers[0].id;
        let applied = preview_document(doc, Some(bg), command, p, 1)?;
        let shown = preview_document(doc, Some(bg), command, p, k).unwrap_or_else(|| panic!("{command} {p}: the k = {k} preview failed"));
        let (a, s) = (every_kth(&applied, k), every_kth(&shown, 1));
        let (src, src_k) = (every_kth(doc, k), every_kth(&crate::proxy::proxy_document(doc, k), 1));
        let applied_change = mean_abs(&a, &src);
        Some(PreviewError {
            diff: mean_abs(&a, &s),
            scale: detail(&shown) / k as f32 / detail(&applied).max(1e-5),
            change: if applied_change > 0.01 { mean_abs(&s, &src_k) / applied_change } else { f32::NAN },
        })
    }

    /// The parameter sets to preview `command` with: the dialog's defaults, and each pixel-sized
    /// value at its minimum and at a large value (whole numbers as integers: Tree and Picture
    /// Frame read some ranges as `u32`).
    fn sweep_params(app: &mut PhotocraftApp, command: &str) -> Vec<(String, Value)> {
        let Some(id) = open(app, command) else { return Vec::new() };
        let base = params_of(&app.ui.dialog_mut(id).unwrap().fields);
        let mut out = vec![("default".to_string(), base.clone())];
        for p in parse_spec(photocraft_engine::commands::find(command).unwrap().params) {
            if !is_pixel_param_of(command, &p.key) {
                continue;
            }
            let values = match p.kind {
                Kind::Int { .. } => vec![-40.0, 40.0],
                Kind::Range { min, max, default } => [min, (default * 4.0).clamp(min, max.min(64.0))].into_iter().filter(|v| *v != default).collect(),
                _ => continue,
            };
            for v in values {
                let mut q = base.clone();
                q[p.key.as_str()] = json!(v);
                out.push((format!("{}={v}", p.key), q));
            }
        }
        for (_, p) in &mut out {
            for v in p.as_object_mut().into_iter().flat_map(|o| o.values_mut()) {
                if let Some(x) = v.as_f64().filter(|x| x.fract() == 0.0 && x.abs() < 1e9) {
                    *v = json!(x as i64);
                }
            }
        }
        out
    }

    /// Every previewed command with its parameter sets.
    fn previewed_cases() -> Vec<(&'static str, String, Value)> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ids: Vec<&'static str> = photocraft_engine::command_specs()
            .iter()
            .map(|c| c.id)
            .filter(|id| has_dialog(id) && (id.starts_with("filter.") || PREVIEWED.contains(id)))
            .collect();
        ids.into_iter().flat_map(|id| sweep_params(&mut app, id).into_iter().map(move |(label, p)| (id, label, p))).collect()
    }

    fn grayscale(d: &Document) -> Document {
        let mut s = photocraft_engine::Session::new();
        s.add_document(d.clone(), None);
        s.execute("image.mode.grayscale", json!({})).unwrap();
        s.active().map(|d| (*d.doc).clone()).unwrap()
    }

    /// Smooth, sharp and noisy test images, each in RGB and grayscale (for the modes that need it).
    fn test_images(n: u32) -> [(&'static str, Document, Document); 3] {
        let with_gray = |d: Document| (grayscale(&d), d);
        let ((ws, w), (ss, s), (ns, no)) = (with_gray(waves(n)), with_gray(sharp(n)), with_gray(noisy(n)));
        [("smooth", w, ws), ("sharp", s, ss), ("noisy", no, ns)]
    }

    fn image_for<'a>(command: &str, rgb: &'a Document, gray: &'a Document) -> &'a Document {
        if command == "image.mode.bitmap" || command == "image.mode.duotone" { gray } else { rgb }
    }

    /// Previews that only approximate the result at a proxy although their features have the
    /// right size: dots and cells placed on the proxy's pixel grid (Color Halftone, Pointillize)
    /// and tile edges resampled (Tiles). Two more only differ on a proxy as small as this test's
    /// (64 x 64 px): Picture Frame's minimum line widths, and Shadows/Highlights' 0.01 % black and
    /// white clips, which pick single pixels out of 4096 (its radii are checked in
    /// `proxy_previews_scale_only_lengths`).
    const APPROXIMATE_AT_PROXY: &[&str] = &[
        "filter.pixelate.colorHalftone",
        "filter.pixelate.pointillize",
        "filter.stylize.tiles",
        "filter.render.pictureFrame",
        "image.adjustments.shadowsHighlights",
    ];

    /// Why a preview doesn't show what OK applies, if it doesn't: a clearly different result or
    /// features at another size on the smooth image, or (on the sharp and noisy images, where a
    /// sub-pixel radius can't show on a proxy) a filter that acts more strongly in the preview,
    /// i.e. over a k times larger area.
    fn mismatch(image: &str, e: &PreviewError) -> Option<String> {
        let off = if image == "smooth" { e.diff > 0.03 || (!(0.6..=1.6).contains(&e.scale) && e.diff > 0.002) } else { e.change > 1.25 };
        off.then(|| format!("{e:?}"))
    }

    /// #2063, every previewed command: the live preview at the factor [`preview_factor`] picks
    /// for a 1/4 proxy shows what OK applies: the same features at the same size, on smooth,
    /// sharp and noisy images, at the defaults and at small and large pixel sizes.
    #[test]
    fn every_preview_shows_what_ok_applies() {
        let cases: Vec<_> = previewed_cases().into_iter().filter(|(id, _, p)| preview_factor(id, p, 4) > 1 && !APPROXIMATE_AT_PROXY.contains(id)).collect();
        // One thread per test image: about 270 filter runs at 256 x 256.
        let bad: Vec<String> = std::thread::scope(|scope| {
            let runs: Vec<_> = test_images(256)
                .into_iter()
                .map(|(image, rgb, gray)| {
                    let cases = &cases;
                    scope.spawn(move || {
                        let mut bad = Vec::new();
                        for (id, label, p) in cases {
                            let k = preview_factor(id, p, 4);
                            if let Some(why) = preview_error(image_for(id, &rgb, &gray), id, p, k).and_then(|e| mismatch(image, &e)) {
                                bad.push(format!("{id} ({label}) at k = {k} on the {image} image: {why}"));
                            }
                        }
                        bad
                    })
                })
                .collect();
            runs.into_iter().flat_map(|r| r.join().unwrap()).collect()
        });
        assert!(bad.is_empty(), "previews that differ from the result:\n{}", bad.join("\n"));
    }

    /// The table behind [`every_preview_shows_what_ok_applies`]: every previewed command at a
    /// plain 1/4 proxy and at its chosen factor, on each test image.
    /// `cargo test -p photocraft-ui-egui --lib filter_dialog::tests::sweep_all_previews -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn sweep_all_previews() {
        let images = test_images(256);
        println!("command | params | image | k | diff k=4 | scale k=4 | change k=4 | diff k | scale k | change k");
        for (id, label, p) in previewed_cases() {
            let k = preview_factor(id, &p, 4);
            for (image, rgb, gray) in &images {
                let doc = image_for(id, rgb, gray);
                match (preview_error(doc, id, &p, 4), preview_error(doc, id, &p, k)) {
                    (Some(a), Some(b)) => println!(
                        "{id} | {label} | {image} | {k} | {:.4} | {:.2} | {:.2} | {:.4} | {:.2} | {:.2}",
                        a.diff, a.scale, a.change, b.diff, b.scale, b.change
                    ),
                    _ => println!("{id} | {label} | {image} | {k} | fails"),
                }
            }
        }
    }
}
