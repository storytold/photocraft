//! The path dialogs of the Pen's canvas menu, the Paths panel and the Pen options bar: Make
//! Selection, Stroke Path, Fill Path, Clipping Path, Transform Path, Define Custom Shape and Make
//! Work Path. Each is a `ParamDialog` opened through `dialogs::params::open_param`, which gives it
//! the remembered-values key `photocraft.<command>`. Only Stroke Path's Simulate Pressure is
//! remembered (#10527); every other row opens at its default.

use craft::ui::contract::Bind;
use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::Label;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::{PathMenuSource, PathSelection};

/// Immutable target of one opening, separate from editable/restorable form values.
pub(crate) struct Target {
    path: PathSelection,
    source: PathMenuSource,
    layer: Option<photocraft_doc::LayerId>,
}

/// The commands this family's dialogs run.
const COMMANDS: [&str; 7] =
    ["edit.defineCustomShape", "path.toSelection", "path.fill", "path.stroke", "path.clippingPath.set", "path.transform", "select.toWorkPath"];

/// The brush-based tools Stroke Path can stroke with, as `path.stroke`'s `tool` takes them.
const STROKE_TOOLS: [(&str, &str); 17] = [
    ("brush", "Brush"),
    ("pencil", "Pencil"),
    ("eraser", "Eraser"),
    ("backgroundEraser", "Background Eraser"),
    ("cloneStamp", "Clone Stamp"),
    ("patternStamp", "Pattern Stamp"),
    ("healingBrush", "Healing Brush"),
    ("spotHealingBrush", "Spot Healing Brush"),
    ("historyBrush", "History Brush"),
    ("blur", "Blur"),
    ("sharpen", "Sharpen"),
    ("smudge", "Smudge"),
    ("dodge", "Dodge"),
    ("burn", "Burn"),
    ("sponge", "Sponge"),
    ("colorReplacement", "Color Replacement"),
    ("mixerBrush", "Mixer Brush"),
];

/// The Make Selection operations, in Photoshop's order: (`path.toSelection` mode, label).
const MAKE_SELECTION_OPERATIONS: [(&str, &str); 4] =
    [("replace", "New Selection"), ("add", "Add to Selection"), ("subtract", "Subtract from Selection"), ("intersect", "Intersect with Selection")];

/// Does the dialog of `key` belong to this family?
pub(crate) fn owns(key: &str) -> bool {
    COMMANDS.contains(&key)
}

/// Recorded steps carry their own target parameters and playback lifecycle.
pub(crate) fn action_editor(app: &PhotocraftApp, id: craft::ui::dialog::DialogId) -> bool {
    crate::actions::is_editor(app, id) || crate::actions::token_of(app, id).is_some()
}

/// A row that opens at its default: the dialog does not remember it.
fn plain(key: &str, label: &str, kind: ParamKind) -> Param {
    Param::new(key, tl!(label).to_string(), kind).not_remembered()
}

fn number(key: &str, label: &str, range: (f64, f64), default: f64, decimals: u32) -> Param {
    plain(key, label, ParamKind::number(Some(range.0), Some(range.1), default, 1.0, decimals))
}

fn define_custom_shape(_: &PhotocraftApp) -> Vec<Param> {
    vec![plain("name", "Name:", ParamKind::text(""))]
}

/// Photoshop's Make Selection dialog: Rendering (Feather Radius, Anti-aliased) and Operation.
/// Without a selection there is nothing to combine with, so only New Selection is available.
fn make_selection_params(app: &PhotocraftApp) -> Vec<Param> {
    let has_selection = app.session.active().is_some_and(|st| st.doc.selection.is_some());
    let operations = MAKE_SELECTION_OPERATIONS.iter().map(|(key, label)| (key.to_string(), tl!(label).to_string().into())).collect();
    let mut mode = plain("mode", "Operation", ParamKind::radio(operations, Some("replace".into())));
    if !has_selection {
        mode = mode.disabled_options(["add", "subtract", "intersect"]);
    }
    vec![
        Param::section(tl!("Rendering").to_string()),
        number("feather", "Feather Radius (pixels):", (0.0, 250.0), 0.0, 1),
        plain("antiAlias", "Anti-aliased", ParamKind::bool(true)),
        Param::section(tl!("Operation").to_string()),
        mode,
    ]
}

fn fill_params(app: &PhotocraftApp) -> Vec<Param> {
    let fg = app.session.tools.foreground;
    vec![
        plain("color", "Color:", ParamKind::color(Some([fg[0], fg[1], fg[2]]))),
        number("opacity", "Opacity:", (0.0, 100.0), 100.0, 0),
        number("feather", "Feather Radius (pixels):", (0.0, 250.0), 0.0, 1),
        plain("antiAlias", "Anti-aliased", ParamKind::bool(true)),
    ]
}

/// Photoshop's Stroke Path dialog: the brush-based tool to stroke with (painting with that tool's
/// current options) and Simulate Pressure, the one remembered row (#2978, #10527).
fn stroke_params(_: &PhotocraftApp) -> Vec<Param> {
    let tools = STROKE_TOOLS.iter().map(|(key, label)| (key.to_string(), tl!(label).to_string().into())).collect();
    vec![
        plain("tool", "Tool:", ParamKind::choice(tools, Some("brush".into()))),
        Param::new("simulatePressure", tl!("Simulate Pressure").to_string(), ParamKind::bool(false)),
    ]
}

fn clipping_params(_: &PhotocraftApp) -> Vec<Param> {
    vec![number("flatness", "Flatness:", (0.0, 100.0), 0.0, 1)]
}

fn transform_params(_: &PhotocraftApp) -> Vec<Param> {
    vec![
        number("translateX", "Horizontal:", (-30000.0, 30000.0), 0.0, 2),
        number("translateY", "Vertical:", (-30000.0, 30000.0), 0.0, 2),
        number("scaleX", "Scale Horizontal:", (0.01, 100.0), 1.0, 2),
        number("scaleY", "Scale Vertical:", (0.01, 100.0), 1.0, 2),
        number("angle", "Angle:", (-360.0, 360.0), 0.0, 2),
    ]
}

/// Photoshop's Make Work Path… asks for the tolerance first.
fn work_path_params(_: &PhotocraftApp) -> Vec<Param> {
    vec![number("tolerance", "Tolerance:", (0.5, 10.0), 2.0, 1)]
}

/// Typed rows shared by direct path dialogs and recorded-action editors.
fn rows_builder(command: &str) -> Option<fn(&PhotocraftApp) -> Vec<Param>> {
    Some(match command {
        "edit.defineCustomShape" => define_custom_shape,
        "path.toSelection" => make_selection_params,
        "path.fill" => fill_params,
        "path.stroke" => stroke_params,
        "path.clippingPath.set" => clipping_params,
        "path.transform" => transform_params,
        "select.toWorkPath" => work_path_params,
        _ => return None,
    })
}

// Affine entries are editable directly: skew, reflections and singular matrices cannot
// all be expressed by the ordinary rotation and scale controls.
const MATRIX_ROWS: [(&str, &str); 6] =
    [("__xx", "X from X:"), ("__yx", "Y from X:"), ("__xy", "X from Y:"), ("__yy", "Y from Y:"), ("translateX", "Horizontal:"), ("translateY", "Vertical:")];

fn matrix_rows(matrix: &Value, fields: &mut DialogFields) -> Result<Vec<Param>, String> {
    let entries = matrix.as_array().filter(|a| a.len() == MATRIX_ROWS.len()).ok_or("Expected six affine matrix entries")?;
    let mut rows = Vec::new();
    for ((key, label), entry) in MATRIX_ROWS.iter().copied().zip(entries) {
        let value = entry.as_f64().filter(|v| v.abs() <= 1_000_000.0).ok_or("Invalid affine matrix entry")?;
        fields.set(key, value);
        rows.push(number(key, label, (-1_000_000.0, 1_000_000.0), value, 8));
    }
    Ok(rows)
}

/// Encode the edited coefficients once for preview, authorization, recording and playback.
/// Keep the original JSON number for untouched entries, including integer-valued coefficients.
pub(crate) fn encode_recorded_matrix(map: &mut serde_json::Map<String, Value>) {
    if !map.contains_key("__xx") {
        return;
    }
    if let Some(entries) = map.get("matrix").and_then(Value::as_array) {
        let mut edited = entries.clone();
        for ((key, _), entry) in MATRIX_ROWS.iter().zip(&mut edited) {
            if let Some(value) = map.get(*key).filter(|v| v.as_f64() != entry.as_f64()) {
                *entry = value.clone();
            }
        }
        map.insert("matrix".into(), Value::Array(edited));
        map.remove("translateX");
        map.remove("translateY");
    }
}

/// Action editors use these same typed rows, seeded from the recorded command rather than
/// the current panel selection. Preserve non-row parameters (for example a transform matrix).
pub(crate) fn open_recorded(app: &mut PhotocraftApp, command: &str, values: &Value) -> Result<craft::ui::dialog::DialogId, String> {
    let build = rows_builder(command).ok_or("Unknown path dialog")?;
    let mut fields = DialogFields::new();
    if let Some(values) = values.as_object() {
        for (key, value) in values {
            fields.set(key.clone(), craft::ui::patterns::form_dialogs::FieldValue::Json(value.clone()));
        }
    }
    let rows = if command == "path.transform"
        && let Some(matrix) = values.get("matrix")
    {
        matrix_rows(matrix, &mut fields)?
    } else if command == "path.transform" && ["quad", "warp", "style", "mesh"].iter().any(|key| values.get(*key).is_some()) {
        // The engine applies the recorded distortion before scalar transform parameters.
        // Leave the geometry in the hosted fields for preview, confirmation and replay.
        vec![Param::read_only_text("__transformNote", Label::Empty, tl!("OK applies the recorded transform.")).full_width()]
    } else {
        build(app)
    };
    app.framework.path_dialog_targets.remove(command);
    if let Some(index) = values.get("pathIndex").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()) {
        let st = app.session.active().ok_or("No active document")?;
        let target = Target { path: PathSelection::Saved { index }, source: PathMenuSource::capture(st), layer: st.active_layer };
        app.framework.path_dialog_targets.insert(command.to_owned(), target);
    }
    let key = command.to_owned();
    let title = title(command);
    Ok(crate::dialogs::params::open_param(app, command, fields, Value::Null, move |fields: &mut DialogFields| {
        ParamDialog::command(key.clone(), tl!(title).to_string(), rows.clone()).values(Bind::bound(fields))
    }))
}

/// Opens a direct path dialog for an immutable target. Make Work Path uses the selection instead.
pub(crate) fn open(app: &mut PhotocraftApp, command: &str, name: impl Into<PathSelection>) {
    let name = name.into();
    let Some(build) = rows_builder(command) else { return };
    let mut fields = DialogFields::new();
    if command != "select.toWorkPath" {
        let Some(st) = app.session.active() else { return };
        let target = Target { path: name.clone(), source: PathMenuSource::capture(st), layer: st.active_layer };
        app.framework.path_dialog_targets.insert(command.to_owned(), target);
        // Fields remain readable to automation; confirmation uses the immutable runtime target.
        match &name {
            PathSelection::Saved { index } => fields.set("pathIndex", i64::try_from(*index).unwrap_or(i64::MAX)),
            PathSelection::Name(name) => fields.set(if command == "edit.defineCustomShape" { "path" } else { "name" }, name.clone()),
        }
    }
    let rows = build(app);
    let key = command.to_owned();
    let title = title(command);
    crate::dialogs::params::open_param(app, command, fields, Value::Null, move |fields: &mut DialogFields| {
        ParamDialog::command(key.clone(), tl!(title).to_string(), rows.clone()).values(Bind::bound(fields))
    });
}

/// The title of the dialog of `command`: the name of its menu row.
fn title(command: &str) -> &'static str {
    match command {
        "edit.defineCustomShape" => "Define Custom Shape",
        "path.toSelection" => "Make Selection",
        "path.fill" => "Fill Path",
        "path.stroke" => "Stroke Path",
        "path.clippingPath.set" => "Clipping Path",
        "path.transform" => "Transform Path",
        _ => "Make Work Path",
    }
}

/// OK in a dialog of this family: runs the command with the confirmed values. A combining Make
/// Selection mode without a selection runs `replace`. OK in Make Selection deselects and hides the
/// path, as ⌘↩ does (#6079); a new selection is the History step Selection Change, as in Photoshop
/// (#12917). Simulate Pressure is remembered only after the stroke ran. [`check`] ran the command
/// and named the step; this runs what follows a passed check.
pub(crate) fn run(app: &mut PhotocraftApp, ctx: &egui::Context, result: ParamResult) {
    if action_editor(app, result.id) {
        crate::filter_dialog::run(app, ctx, result);
        return;
    }
    match result.key.as_str() {
        "path.toSelection" => {
            if let Some(target) = app.framework.path_dialog_targets.get(&result.key).map(|t| t.path.clone()) {
                crate::vector_ui::deselect_loaded_path(app, target);
            }
        }
        "path.stroke" => result.remember(ctx),
        _ => {}
    }
}

/// OK held in a dialog of this family: runs the command, so a refusal keeps the dialog open with
/// its message and leaves the previous step unrenamed. A new selection's step is named here, as
/// it belongs to the step the command added.
pub(crate) fn check(app: &mut PhotocraftApp, result: &ParamResult) -> Result<(), Label> {
    if action_editor(app, result.id) {
        check_source(app, &result.key).map_err(Label::from)?;
        return crate::filter_dialog::check(app, result);
    }
    let params = command_params(app, &result.key, &result.fields).map_err(Label::from)?;
    let revision = app.session.active().map(|st| st.revision);
    app.run(result.key.as_str(), params.clone()).map_err(Label::from)?;
    if result.key == "path.toSelection" {
        name_make_selection_step(app, &params, revision);
    }
    Ok(())
}

/// A paused action's numeric target is positional too. Refuse both preview and OK after
/// the source changes, before a shifted index can edit another path.
pub(crate) fn check_source(app: &PhotocraftApp, key: &str) -> Result<(), String> {
    if let Some(target) = app.framework.path_dialog_targets.get(key)
        && !matches!(target.path.name(), Some("work" | "layer"))
        && app.session.active().map(PathMenuSource::capture).as_ref() != Some(&target.source)
    {
        return Err("Path dialog target is stale; reopen the dialog".into());
    }
    Ok(())
}

/// The engine parameters of a direct dialog, refusing an unknown key or stale saved target.
pub(crate) fn command_params(app: &PhotocraftApp, key: &str, f: &DialogFields) -> Result<Value, String> {
    let num = |k: &str, default: f64| f.number(k).unwrap_or(default);
    let name = f.text("name").unwrap_or_default().to_string();
    let round1 = |v: f64| (v * 10.0).round() / 10.0;
    let params = match key {
        "edit.defineCustomShape" => json!({"path": f.text("path").unwrap_or_default(), "name": name}),
        "path.toSelection" => {
            let has_selection = app.session.active().is_some_and(|st| st.doc.selection.is_some());
            let mode = f.text("mode").filter(|_| has_selection).unwrap_or("replace");
            json!({"name": name, "feather": round1(num("feather", 0.0)), "antiAlias": f.bool("antiAlias").unwrap_or(true), "mode": mode})
        }
        "path.fill" => {
            let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            let [r, g, b] = f.color("color").unwrap_or([0.0; 3]);
            json!({
                "name": name,
                "color": format!("#{:02x}{:02x}{:02x}", byte(r), byte(g), byte(b)),
                "opacity": num("opacity", 100.0),
                "feather": round1(num("feather", 0.0)),
                "antiAlias": f.bool("antiAlias").unwrap_or(true),
            })
        }
        "path.stroke" => json!({"name": name, "tool": f.text("tool").unwrap_or("brush"), "simulatePressure": f.bool("simulatePressure").unwrap_or(false)}),
        "path.clippingPath.set" => json!({"name": name, "flatness": num("flatness", 0.0)}),
        "path.transform" => json!({
            "name": name,
            "translateX": num("translateX", 0.0),
            "translateY": num("translateY", 0.0),
            "scaleX": num("scaleX", 1.0),
            "scaleY": num("scaleY", 1.0),
            "angle": num("angle", 0.0),
        }),
        "select.toWorkPath" => json!({"tolerance": num("tolerance", 2.0)}),
        _ => return Err("Unknown path dialog".into()),
    };
    if key == "select.toWorkPath" {
        return Ok(params);
    }
    let target = app.framework.path_dialog_targets.get(key).ok_or("Path dialog target is missing; reopen the dialog")?;
    let st = app.session.active().ok_or("No active document")?;
    // Only saved rows carry positional identity. Work/layer dialogs keep their existing
    // behavior when a selection changes while the dialog is open.
    if !matches!(target.path.name(), Some("work" | "layer")) && target.source != PathMenuSource::capture(st) {
        return Err("Path dialog target is stale; reopen the dialog".into());
    }
    if key == "edit.defineCustomShape" {
        let path = if target.path.name() == Some("layer") {
            target.layer.and_then(|id| st.doc.layer(id)).and_then(|l| match &l.content {
                photocraft_doc::LayerContent::Shape(shape) => Some(&shape.path),
                _ => l.vector_mask.as_ref().map(|m| &m.path),
            })
        } else {
            target.path.path(&st.doc)
        }
        .ok_or("Path no longer exists")?;
        return Ok(json!({"name": name, "path": photocraft_engine::vector_cmds::path_json(path)}));
    }
    if key == "path.clippingPath.set" {
        let index = target.path.saved_index(&st.doc).ok_or("No saved path selected")?;
        let name = &st.doc.paths.get(index).ok_or("Path no longer exists")?.name;
        if st.doc.paths.iter().filter(|p| p.name == *name).count() != 1 {
            return Err("Clipping paths require a unique name; rename the path first".into());
        }
        return Ok(json!({"name": name, "flatness": num("flatness", 0.0)}));
    }
    let mut params = target.path.with_params(params);
    if target.path.name() == Some("layer") {
        params["layer"] = json!(target.layer.ok_or("No layer selected")?.0);
    }
    Ok(params)
}

/// A new selection is the History step Selection Change. The combining operations keep the
/// engine's Make Selection (not measured). Only a step this run added is renamed: `revision` is from
/// before it.
fn name_make_selection_step(app: &mut PhotocraftApp, params: &Value, revision: Option<u64>) {
    if params.get("mode").and_then(Value::as_str).is_some_and(|m| m != "replace") {
        return;
    }
    if let Some(st) = app.session.active_mut().filter(|st| Some(st.revision) != revision) {
        st.history.set_current_label("Selection Change");
    }
}

/// The Pen options bar's Make Selection (Path mode, #5703): the canvas menu's Make Selection…
/// dialog for the path being drawn (committed first, as the menu does) or the active path.
pub(crate) fn make_selection(app: &mut PhotocraftApp) -> Result<(), String> {
    let name = super::commit_pending_pen_target(app)?
        .or_else(|| crate::vector_ui::active_path_name(app))
        .ok_or_else(|| "No path is available for this action".to_string())?;
    open(app, "path.toSelection", &name);
    Ok(())
}
