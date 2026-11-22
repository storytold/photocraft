//! The path dialogs of the Pen's canvas menu, the Paths panel and the Pen options bar: Make
//! Selection, Stroke Path, Fill Path, Clipping Path, Transform Path, Define Custom Shape and Make
//! Work Path. Each is a `ParamDialog` opened through `dialogs::params::open_param`, which gives it
//! the remembered-values key `photocraft.<command>`. Only Stroke Path's Simulate Pressure is
//! remembered (#10527); every other row opens at its default.

use craft::ui::contract::Bind;
use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::Label;
use serde_json::{Value, json};

use super::pen_pending;
use crate::PhotocraftApp;

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

/// Opens the dialog of `command` for the path `name` (the Paths panel menu, Alt-click on its Stroke
/// button, the Pen's canvas menu). `name` is unused by Make Work Path, which acts on the selection.
pub(crate) fn open(app: &mut PhotocraftApp, command: &str, name: &str) {
    let build: fn(&PhotocraftApp) -> Vec<Param> = match command {
        "edit.defineCustomShape" => define_custom_shape,
        "path.toSelection" => make_selection_params,
        "path.fill" => fill_params,
        "path.stroke" => stroke_params,
        "path.clippingPath.set" => clipping_params,
        "path.transform" => transform_params,
        "select.toWorkPath" => work_path_params,
        _ => return,
    };
    let mut fields = DialogFields::new();
    match command {
        "select.toWorkPath" => {}
        // The shape's name is typed; the path it is defined from rides along.
        "edit.defineCustomShape" => fields.set("path", name),
        _ => fields.set("name", name),
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
    match result.key.as_str() {
        "path.toSelection" => {
            let name = result.fields.text("name").unwrap_or_default().to_string();
            crate::vector_ui::deselect_loaded_path(app, &name);
        }
        "path.stroke" => result.remember(ctx),
        _ => {}
    }
}

/// OK held in a dialog of this family: runs the command, so a refusal keeps the dialog open with
/// its message and leaves the previous step unrenamed. A new selection's step is named here, as
/// it belongs to the step the command added.
pub(crate) fn check(app: &mut PhotocraftApp, result: &ParamResult) -> Result<(), Label> {
    let Some(params) = command_params(app, result) else { return Ok(()) };
    let revision = app.session.active().map(|st| st.revision);
    app.run(result.key.as_str(), params.clone()).map_err(Label::from)?;
    if result.key == "path.toSelection" {
        name_make_selection_step(app, &params, revision);
    }
    Ok(())
}

/// The engine parameters a dialog's confirmed values make; `None` for a key the family does not own.
fn command_params(app: &PhotocraftApp, result: &ParamResult) -> Option<Value> {
    let key = result.key.as_str();
    let f = &result.fields;
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
        _ => return None,
    };
    Some(params)
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
    let pending = pen_pending(app);
    // Where `pen_commit` puts the path being drawn: a shape layer or the targeted vector mask,
    // else the Paths panel path it adds to (a selected saved path, or the Work Path), which it
    // selects so it is the active path.
    let pending_layer =
        pending && (app.ui.tool_options.vector_mode == craft::draw::ui::VectorMode::Shape || crate::vector_ui::targeted_vector_mask(app).is_some());
    if pending && !crate::vector_ui::pen_commit(app, false) {
        return Err(app.ui.status.clone());
    }
    let name = if pending_layer { Some("layer".to_string()) } else { crate::vector_ui::active_path_name(app) }
        .ok_or_else(|| "No path is available for this action".to_string())?;
    open(app, "path.toSelection", &name);
    Ok(())
}
