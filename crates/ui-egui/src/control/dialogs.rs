//! The app's `ui.dialog.*` control surface and the `{ui:{…}}` requests on a hosted workspace.
//!
//! Every dialog is on the framework's `DialogHost`; a request names it by the id `ui.inspect` lists, or `"top"`
//! (`craft::automation::dialogs::dialog_id`). Production's field and choice names map to the framework's field ids
//! through [`FIELD_KEYS`], in one place for every dialog type.

use craft::automation::dialogs::dialog_id;
use craft::ui::dialog::DialogId;
use serde_json::{Value, json};

use super::params::{err, ok};
use super::{ControlRequest, Reply};
use crate::PhotocraftApp;

use super::dispatch::wait_param;

/// One opener: reads its own params and returns the opened id or the refusal text.
type Opener = fn(&mut PhotocraftApp, &egui::Context, &Value) -> Result<DialogId, String>;

/// The `ui.dialog.open` kinds. Production also accepts the capitalised spellings (`NewDocument`, `About`,
/// `ColorPicker`, `LayerStyle`, `Command`); [`open`] lowercases the first letter before the lookup.
pub(crate) const KINDS: &[(&str, Opener)] = &[
    ("newDocument", |app, _, _| Ok(crate::new_doc_ui::open(app))),
    ("about", open_about),
    ("colorPicker", open_color_picker),
    ("layerStyle", |app, _, p| crate::layer_style::open(app, p.get("effect").and_then(Value::as_str)).ok_or_else(|| "no active layer".to_string())),
    ("newCropPreset", |app, _, _| {
        crate::crop_presets::open_new(app).ok_or_else(|| "nothing to save: set W and H (W x H x Resolution) or a ratio first".to_string())
    }),
    ("deleteCropPreset", |app, _, _| crate::crop_presets::open_delete(app).ok_or_else(|| "no crop presets are saved".to_string())),
    ("command", |app, _, p| {
        let Some(cmd) = p.get("command").and_then(Value::as_str) else { return Err("command dialogs need `command`".into()) };
        let label = photocraft_engine::commands::find(cmd).map(|c| c.label).unwrap_or(cmd);
        crate::dialogs::open_command_dialog(app, cmd, label)
    }),
    ("newSnapshot", |app, _, _| crate::snapshot_dialog::open(app).ok_or_else(|| "no document is open".to_string())),
    ("newBrushPreset", |app, _, _| Ok(crate::brush_preset_dialog::open(app, None))),
    ("channelOptions", |app, _, p| {
        let opened = match p.get("channel") {
            Some(channel) => crate::channel_options_ui::open(app, channel),
            None => crate::channel_options_ui::open_new_channel(app),
        };
        opened.ok_or_else(|| "no channel to show options for".to_string())
    }),
    ("mergeChannels", |app, _, _| crate::dialogs::merge_channels_ui::open(app).ok_or_else(|| "Merge Channels needs open documents".to_string())),
    ("formatOptions", |app, ctx, p| {
        if p.get("format").and_then(Value::as_str).is_none() || p.get("path").and_then(Value::as_str).is_none() {
            return Err("formatOptions needs `format` and `path`".into());
        }
        crate::dialogs::format_prompts::open(app, ctx, p)
    }),
];

fn open_about(app: &mut PhotocraftApp, _ctx: &egui::Context, p: &Value) -> Result<DialogId, String> {
    use craft::ui::patterns::form_dialogs::AboutTab;
    let tab = match p.get("tab").and_then(Value::as_str) {
        None | Some("about") => AboutTab::About,
        Some("contributors") => AboutTab::Contributors,
        Some("models") => AboutTab::Models,
        Some(other) => return Err(format!("unknown About tab `{other}` (about, contributors, models)")),
    };
    Ok(crate::credits::open_about(app, tab))
}

/// The colour picker kind: a text style's colour, a Layer Style effect's colour, or the foreground or background.
fn open_color_picker(app: &mut PhotocraftApp, _ctx: &egui::Context, p: &Value) -> Result<DialogId, String> {
    let s = |key: &str| p.get(key).and_then(Value::as_str);
    if let Some(style) = p.get("textStyle") {
        if ["parent", "effect", "field", "target"].iter().any(|key| p.get(*key).is_some()) {
            return Err("textStyle cannot be combined with another picker target".into());
        }
        let Some(style) = style.as_object().filter(|s| s.keys().all(|k| k == "kind" || k == "id")) else {
            return Err("textStyle needs kind and id only".into());
        };
        let Some(kind) = style.get("kind").cloned().and_then(|v| serde_json::from_value::<crate::type_panels_ui::color_picker::Kind>(v).ok()) else {
            return Err("textStyle kind must be character or paragraph".into());
        };
        let Some(id) = style.get("id").and_then(Value::as_u64).and_then(|id| u32::try_from(id).ok()) else {
            return Err("textStyle id must be a u32 integer".into());
        };
        // PLAN: assumed `type_panels_ui::color_picker::open(app, kind, id) -> Result<DialogId, String>` (panels-w42).
        return crate::type_panels_ui::color_picker::open(app, kind, id);
    }
    if p.get("parent").is_some() {
        // The parent is the dialog id `ui.inspect` lists (a string of its number).
        let parent = p.get("parent").and_then(|id| id.as_u64().or_else(|| id.as_str().and_then(|id| id.parse().ok())));
        let (Some(parent), Some(effect), Some(field)) = (parent, s("effect"), s("field")) else {
            return Err("need the Layer Style dialog's id as `parent` and string `effect` and `field`".into());
        };
        let stop = match p.get("stop") {
            None | Some(Value::Null) => None,
            Some(v) => match v.as_u64().and_then(|i| usize::try_from(i).ok()) {
                Some(i) => Some(i),
                None => return Err("`stop` must be a non-negative integer".into()),
            },
        };
        // PLAN: assumed `layer_style::color_picker::open(..) -> Result<DialogId, String>` (dialogs-w35).
        return crate::layer_style::color_picker::open(app, parent, effect, field, stop);
    }
    if p.get("effect").is_some() || p.get("field").is_some() {
        return Err("Layer Style color pickers need `parent`".into());
    }
    let target = if s("target") == Some("background") { "background" } else { "foreground" };
    // PLAN: assumed `color_picker_ui::open(app, target: &str) -> DialogId` (dialogs-w37).
    Ok(crate::color_picker_ui::open(app, target))
}

// ---- the dialog and its fields ----

fn host(app: &PhotocraftApp) -> &craft::ui::dialog::DialogHost {
    &app.framework.dialogs
}

fn fields_of(app: &PhotocraftApp, id: DialogId) -> Value {
    host(app).open_dialogs().into_iter().find(|d| d.id == id).and_then(|d| d.fields).unwrap_or(Value::Null)
}

fn key_of(app: &PhotocraftApp, id: DialogId) -> Option<String> {
    host(app).open_dialogs().into_iter().find(|d| d.id == id).and_then(|d| d.key)
}

/// Which dialog a [`FieldKey`] row applies to. `StackDialog::key` is `None` for the prompts, so those match by type
/// or tag.
#[derive(Clone, Copy)]
pub(crate) enum Which {
    /// The dialog's `OpenDialog.key`.
    Key(&'static str),
    /// A test on the app and the dialog id.
    Is(fn(&PhotocraftApp, DialogId) -> bool),
}

/// Builds the framework field pairs from the dialog's current `json_fields` and a production value.
type FieldBuilder = fn(current: &Value, value: &Value) -> Vec<(String, Value)>;

/// One production field name and how it reaches the framework's field.
pub(crate) struct FieldKey {
    /// The dialog the row applies to.
    pub which: Which,
    /// The production field name.
    pub from: &'static str,
    /// The framework field id; unused when `with` is set.
    pub to: &'static str,
    /// Production value to framework value, for a string value.
    pub values: &'static [(&'static str, &'static str)],
    /// Builds the pairs to set from the dialog's current `json_fields` and the value, for a field that is not a
    /// one-to-one rename.
    pub with: Option<FieldBuilder>,
}

fn is_prompt<T: std::any::Any>(app: &PhotocraftApp, id: DialogId) -> bool {
    crate::dialogs::alerts::prompt_tagged::<T>(app) == Some(id)
}

fn is_unsaved(app: &PhotocraftApp, id: DialogId) -> bool {
    host(app).get_by_id::<craft::ui::dialog::UnsavedChangesDialog>(id).is_some()
}

/// Whether confirming dialog `id` answers the unsaved-changes prompt with Save.
pub(crate) fn is_save_answer(app: &PhotocraftApp, id: DialogId) -> bool {
    is_unsaved(app, id) && fields_of(app, id)["choice"] == "save"
}

fn is_profile(app: &PhotocraftApp, id: DialogId) -> bool {
    host(app).get_by_id::<craft::color_ui::ui::ProfileMismatchDialog>(id).is_some()
}

fn is_color_picker(app: &PhotocraftApp, id: DialogId) -> bool {
    // PLAN: assumed facade path of `ColorPickerDialog`.
    host(app).get_by_id::<craft::color_ui::ui::ColorPickerDialog>(id).is_some()
}

/// Export As's `scale` (percent) is the dialog's `size`, as the scale variant of its serde form.
fn export_scale(_: &Value, v: &Value) -> Vec<(String, Value)> {
    vec![pair("size", json!({"Scale": v}))]
}

/// The mask-trash prompt of a vector mask (the layer-mask prompt has the same tag type, other answers).
fn is_vector_trash(app: &PhotocraftApp, id: DialogId) -> bool {
    host(app).get_by_id::<craft::ui::dialog::StandardPrompt>(id).is_some_and(|prompt| matches!(prompt.kind(), craft::ui::dialog::PromptKind::TrashVectorMask))
}

const CANCEL: (&str, &str) = ("cancel", "cancel");

macro_rules! prompt {
    ($tag:ty, $values:expr) => {
        FieldKey { which: Which::Is(is_prompt::<$tag>), from: "choice", to: "choice", values: $values, with: None }
    };
}

fn pair(field: &str, value: Value) -> (String, Value) {
    (field.to_string(), value)
}

fn save_all(_: &Value, _: &Value) -> Vec<(String, Value)> {
    vec![pair("applyToAll", json!(true)), pair("choice", json!("save"))]
}

fn dont_save_all(_: &Value, _: &Value) -> Vec<(String, Value)> {
    vec![pair("applyToAll", json!(true)), pair("choice", json!("dontSave"))]
}

/// A blend label such as "Multiply" to the Blend row's id; a string `parse_blend` refuses passes through so the
/// row refuses it.
fn blend_id(value: &Value) -> Value {
    value.as_str().and_then(photocraft_engine::commands::parse_blend).map(|mode| json!(mode.key().to_string())).unwrap_or_else(|| value.clone())
}

// PLAN: the framework field ids `mode` (Fill, Stroke) and `blend` (Smart Filter) are assumed (dialogs-w22 by request).
fn mode_row(_: &Value, v: &Value) -> Vec<(String, Value)> {
    vec![pair("mode", blend_id(v))]
}

fn blend_row(_: &Value, v: &Value) -> Vec<(String, Value)> {
    vec![pair("blend", blend_id(v))]
}

fn merge_documents(_: &Value, v: &Value) -> Vec<(String, Value)> {
    v.as_array().map(|ids| ids.iter().take(4).enumerate().map(|(i, id)| (format!("channel{}", i + 1), id.clone())).collect()).unwrap_or_default()
}

fn center_with(current: &Value, v: &Value, axis: usize) -> Vec<(String, Value)> {
    let mut center = current.get("center").and_then(|c| serde_json::from_value::<[f64; 2]>(c.clone()).ok()).unwrap_or([50.0, 50.0]);
    center[axis] = v.as_f64().unwrap_or(center[axis]);
    vec![pair("center", json!(center))]
}

fn center_x(current: &Value, v: &Value) -> Vec<(String, Value)> {
    center_with(current, v, 0)
}

fn center_y(current: &Value, v: &Value) -> Vec<(String, Value)> {
    center_with(current, v, 1)
}

fn slice_background(_: &Value, v: &Value) -> Vec<(String, Value)> {
    vec![pair("background", if v.as_str() == Some("none") { Value::Null } else { v.clone() })]
}

fn dropped(_: &Value, _: &Value) -> Vec<(String, Value)> {
    Vec::new()
}

fn liquify_backdrop(_: &Value, v: &Value) -> Vec<(String, Value)> {
    let backdrop = if v.as_str() == Some("all") { json!("allLayers") } else { json!({"layer": v}) };
    vec![pair("backdrop", backdrop)]
}

/// Production names to framework field ids, for every dialog type; the first row whose dialog and `from` match wins.
pub(crate) const FIELD_KEYS: &[FieldKey] = &[
    FieldKey { which: Which::Is(is_vector_trash), from: "choice", to: "choice", values: &[("delete", "proceed"), CANCEL], with: None },
    prompt!(crate::dialogs::mask_trash_prompt::MaskTrash, &[("apply", "proceed"), ("delete", "alternate"), CANCEL]),
    prompt!(crate::dialogs::rasterize_prompt::Rasterize, &[("rasterize", "proceed"), ("smartObject", "alternate"), CANCEL]),
    prompt!(crate::dialogs::preset_group_prompt::PresetClash, &[("new", "proceed"), ("replace", "alternate"), CANCEL]),
    prompt!(crate::dialogs::live_shape_prompt::LiveShapeAsk, &[("yes", "proceed"), ("no", "cancel"), CANCEL]),
    prompt!(crate::dialogs::delete_layer_prompt::PendingDelete, &[("delete", "proceed"), CANCEL]),
    prompt!(crate::dialogs::open_prompt::DamageTag, &[("continue", "proceed"), ("open", "proceed"), CANCEL]),
    prompt!(crate::dialogs::open_prompt::MemoryTag, &[("continue", "proceed"), ("open", "proceed"), CANCEL]),
    prompt!(crate::dialogs::disk_change_prompt::HeldSave, &[CANCEL]),
    FieldKey { which: Which::Is(is_unsaved), from: "choice", to: "choice", values: &[], with: None },
    FieldKey { which: Which::Is(is_unsaved), from: "choice", to: "choice", values: &[("saveAll", "save")], with: Some(save_all) },
    FieldKey { which: Which::Is(is_unsaved), from: "choice", to: "choice", values: &[("dontSaveAll", "dontSave")], with: Some(dont_save_all) },
    FieldKey {
        which: Which::Is(is_profile),
        from: "action",
        to: "action",
        values: &[("preserve", "useEmbedded"), ("convert", "convertToWorking"), ("discard", "discard")],
        with: None,
    },
    FieldKey { which: Which::Is(is_color_picker), from: "webOnly", to: "webColors", values: &[], with: None },
    FieldKey { which: Which::Key("edit.fill"), from: "mode", to: "mode", values: &[], with: Some(mode_row) },
    FieldKey { which: Which::Key("edit.stroke"), from: "mode", to: "mode", values: &[], with: Some(mode_row) },
    FieldKey { which: Which::Key("layer.smartFilter.blendingOptions"), from: "blend", to: "blend", values: &[], with: Some(blend_row) },
    FieldKey { which: Which::Key("channel.mergeChannels"), from: "documents", to: "", values: &[], with: Some(merge_documents) },
    FieldKey { which: Which::Key("filter.blur.radialBlur"), from: "centerX", to: "center", values: &[], with: Some(center_x) },
    FieldKey { which: Which::Key("filter.blur.radialBlur"), from: "centerY", to: "center", values: &[], with: Some(center_y) },
    FieldKey { which: Which::Key("slice.set"), from: "kind", to: "kind", values: &[("table", "htmlText")], with: None },
    FieldKey { which: Which::Key("slice.set"), from: "background", to: "background", values: &[], with: Some(slice_background) },
    FieldKey { which: Which::Key("slice.divide"), from: "horizontal", to: "rows", values: &[], with: None },
    FieldKey { which: Which::Key("slice.divide"), from: "vertical", to: "columns", values: &[], with: None },
    FieldKey { which: Which::Key("slice.divide"), from: "number", to: "", values: &[], with: Some(dropped) },
    FieldKey { which: Which::Key("file.exportAs"), from: "scale", to: "", values: &[], with: Some(export_scale) },
    FieldKey { which: Which::Key("filter.liquify"), from: "backdropUse", to: "backdrop", values: &[], with: Some(liquify_backdrop) },
];

/// What `ui.dialog.set` sets for a production `field` and `value`: the rows' pairs, or the field unchanged (a
/// parameter dialog's keys are the engine's).
pub(crate) fn map_request(app: &PhotocraftApp, id: DialogId, field: &str, value: Value) -> Vec<(String, Value)> {
    let key = key_of(app, id);
    let current = fields_of(app, id);
    for row in FIELD_KEYS {
        let hit = match row.which {
            Which::Key(k) => key.as_deref() == Some(k),
            Which::Is(is) => is(app, id),
        };
        // A row with `with` and a plain row for one prompt (the unsaved prompt) are told apart by the value.
        let fits = row.with.is_none() || row.values.is_empty() || value.as_str().is_some_and(|v| row.values.iter().any(|(from, _)| *from == v));
        let plain_shadowed = row.with.is_none()
            && row.values.is_empty()
            && FIELD_KEYS.iter().any(|r| {
                r.from == field
                    && r.with.is_some()
                    && matches!((r.which, row.which), (Which::Is(a), Which::Is(b)) if a as usize == b as usize)
                    && value.as_str().is_some_and(|v| r.values.iter().any(|(from, _)| *from == v))
            });
        if !hit || row.from != field || !fits || plain_shadowed {
            continue;
        }
        if let Some(with) = row.with {
            return with(&current, &value);
        }
        let mapped = value.as_str().and_then(|v| row.values.iter().find(|(from, _)| *from == v)).map_or(value, |(_, to)| json!(to));
        return vec![(row.to.to_string(), mapped)];
    }
    vec![(field.to_string(), hex_as_color(&current, field, value))]
}

/// A `#rrggbb` string sent to a colour row, as the three 0..=1 components the row keeps.
fn hex_as_color(current: &Value, field: &str, value: Value) -> Value {
    let is_color_row = current.get(field).and_then(Value::as_array).is_some_and(|c| c.len() == 3 && c.iter().all(Value::is_number));
    match value.as_str().filter(|_| is_color_row).and_then(craft::theme::Srgba::from_hex) {
        Some(c) => json!([c.r, c.g, c.b]),
        None => value,
    }
}

/// The command a hosted dialog's confirm is judged as, with its params; `None` when it runs no command.
fn command_of(app: &PhotocraftApp, id: DialogId, choice: &str) -> Option<(String, Value)> {
    if is_unsaved(app, id) {
        return if choice == "cancel" { None } else { crate::discard_ui::pending_command(app) };
    }
    let key = key_of(app, id)?;
    let fields = fields_of(app, id);
    match key.as_str() {
        "workspace.new" => Some(("window.workspace.newWorkspace".into(), fields)),
        "workspace.delete" => Some(("window.workspace.deleteWorkspace".into(), json!({"name": fields.get("name")}))),
        _ => hosted_command(app, id),
    }
}

/// The command a hosted parameter dialog stands for, with its values: the dialog's key, when the key is a command (an
/// engine command or a shell one such as `file.save`). Automation is judged for it when the dialog is confirmed.
pub(crate) fn hosted_command(app: &PhotocraftApp, id: DialogId) -> Option<(String, Value)> {
    let key = key_of(app, id)?;
    (host(app).get_by_id::<craft::ui::patterns::form_dialogs::HostedParamDialog>(id).is_some() && crate::menus::is_live(&key))
        .then(|| (key, fields_of(app, id)))
}

// ---- ui.dialog.* ----

/// Answers `ui.dialog.{open, set, confirm, cancel, apply}`; `None` for any other method.
pub(crate) fn method(app: &mut PhotocraftApp, ctx: &egui::Context, req: &ControlRequest) -> Option<Reply> {
    let p = &req.params;
    if req.method == "ui.dialog.open" {
        return Some(open(app, ctx, p));
    }
    if !matches!(req.method.as_str(), "ui.dialog.set" | "ui.dialog.confirm" | "ui.dialog.cancel" | "ui.dialog.apply") {
        return None;
    }
    let id = match dialog_id(host(app), p) {
        Ok(id) => id,
        Err(e) => return Some(err(e)),
    };
    Some(match req.method.as_str() {
        "ui.dialog.set" => set(app, ctx, id, p),
        "ui.dialog.confirm" => confirm(app, ctx, id, p),
        "ui.dialog.cancel" => Reply::from_result(crate::dialogs::cancel(app, ctx, id).map(|()| Value::Null)),
        _ => apply(app, id),
    })
}

fn open(app: &mut PhotocraftApp, ctx: &egui::Context, p: &Value) -> Reply {
    let kind = p.get("kind").and_then(Value::as_str).unwrap_or("");
    let mut chars = kind.chars();
    let normal: String = chars.next().map(|c| c.to_lowercase().chain(chars).collect()).unwrap_or_default();
    let Some((_, opener)) = KINDS.iter().find(|(name, _)| *name == normal) else { return err(format!("unknown dialog kind `{kind}`")) };
    match opener(app, ctx, p) {
        Ok(id) => ok(json!({"dialog": id.value().to_string()})),
        Err(e) => err(e),
    }
}

fn set(app: &mut PhotocraftApp, ctx: &egui::Context, id: DialogId, p: &Value) -> Reply {
    let Some(field) = p.get("field").and_then(Value::as_str) else { return err("need `dialog` and `field`") };
    if host(app).open_dialogs().iter().any(|d| d.parent == Some(id)) {
        return err("confirm or cancel the color picker first");
    }
    let value = p.get("value").cloned().unwrap_or(Value::Null);
    // An adjustment dialog's address (`target`, `layer`, `document`) is not one of its fields: it retargets the dialog.
    if matches!(field, "target" | "layer" | "document") && app.adjust_open.as_ref().is_some_and(|open| open.id == id) {
        return if crate::adjust_dialog::apply_params(app, id, &json!({ field: value })) {
            ok(Value::Null)
        } else {
            err(format!("the dialog refused `{field}`"))
        };
    }
    // An adjustment dialog lists the command's parameters beside the host's own fields (Preview, the picker, the shown
    // channel, the clipping display): a set of a parameter is an edit of the parameters, read into the dialog's value
    // as the engine reads them.
    if !matches!(field, "preview" | "picker" | "channel" | "clipping")
        && let Some(mut params) = app.adjust_open.as_ref().filter(|open| open.id == id).and_then(|_| crate::adjust_dialog::open_params(app))
        && params.get(field).is_some()
    {
        params[field] = value;
        if !crate::adjust_dialog::apply_params(app, id, &params) {
            return err(format!("the dialog refused `{field}`"));
        }
        crate::dialogs::take_results(app, ctx);
        return ok(Value::Null);
    }
    for (field, value) in map_request(app, id, field, value) {
        if let Err(e) = app.framework.dialogs.set_json_field(ctx, id, &field, value) {
            return err(e);
        }
    }
    crate::dialogs::take_results(app, ctx);
    ok(Value::Null)
}

fn confirm(app: &mut PhotocraftApp, ctx: &egui::Context, id: DialogId, p: &Value) -> Reply {
    let choice = p.get("choice").and_then(Value::as_str);
    if let Some(choice) = choice {
        for (field, value) in map_request(app, id, "choice", json!(choice)) {
            if let Err(e) = app.framework.dialogs.set_json_field(ctx, id, &field, value) {
                return err(e);
            }
        }
    }
    // The unsaved-changes prompt answers Save when no choice is named.
    let judged = choice.unwrap_or("save");
    // A Save writes the document back, so it is judged as `file.save` before the question closes.
    if is_save_answer(app, id)
        && let Err(error) = crate::discard_ui::authorize_save(app, true)
    {
        return err(error);
    }
    if let Some((command, params)) = command_of(app, id, judged)
        && let Some(authorize) = app.services.automation_command.as_ref()
        && let Err(error) = authorize(&command, &params)
    {
        return err(error);
    }
    // The unsaved-changes prompt answers with what is left to answer: whether another prompt or a file dialog is open.
    let unsaved = is_unsaved(app, id);
    super::run_waiting(app, wait_param(p), |app| {
        crate::dialogs::confirm(app, ctx, id)?;
        Ok(if unsaved { json!({"open": app.discard.is_some(), "fileDialog": app.file_dialog_open()}) } else { Value::Null })
    })
}

fn apply(app: &mut PhotocraftApp, id: DialogId) -> Reply {
    if key_of(app, id).as_deref() == Some("preferences") {
        return Reply::from_result(crate::prefs_ui::apply_window(app));
    }
    err("`ui.dialog.apply` is for Preferences; use `ui.dialog.confirm`")
}

// ---- hosted workspaces ----

/// A hosted workspace's `describe(app)`.
type DescribeFn = fn(&PhotocraftApp) -> Option<Value>;

/// The `describe(app)` of each hosted workspace, by its dialog key (L-21). Facts that live in app data stay in the
/// module's adapter; this table only merges them into the workspace's `json_fields`.
const DESCRIBE: &[(&str, DescribeFn)] = &[
    ("select.selectAndMask", crate::select_mask_ui::describe),
    ("edit.contentAwareFill", crate::caf_ui::describe),
    ("filter.liquify", crate::liquify_ui::describe),
    ("filter.filterGallery", crate::gallery_ui::describe),
    (photocraft_engine::lens_cmds::WIDE, crate::wide_angle_ui::describe),
    ("filter.vanishingPoint", crate::vp_ui::describe),
    ("filter.cameraRaw", |app| Some(crate::camera_raw_ui::describe(app))),
];

fn workspace_reply(app: &PhotocraftApp, key: &str, id: DialogId) -> Value {
    let mut reply = fields_of(app, id);
    if !reply.is_object() {
        reply = json!({});
    }
    let facts = DESCRIBE.iter().find(|(k, _)| *k == key).and_then(|(_, describe)| describe(app));
    if let (Some(map), Some(Value::Object(facts))) = (reply.as_object_mut(), facts) {
        map.extend(facts);
    }
    reply
}

/// Runs the preview jobs of the open workspace to completion, then one pass of results, so the request's reply
/// shows the finished preview.
fn wait_preview(app: &mut PhotocraftApp, ctx: &egui::Context) {
    /// How long a request waits for the jobs on the framework's lanes.
    const WAIT: std::time::Duration = std::time::Duration::from_secs(30);
    craft::ui::runtime::pending::finish_jobs(ctx, WAIT);
    crate::dialogs::take_results(app, ctx);
}

fn apply_field(app: &mut PhotocraftApp, ctx: &egui::Context, id: DialogId, field: &str, value: &Value) -> Result<(), String> {
    for (field, value) in map_request(app, id, field, value.clone()) {
        app.framework.dialogs.set_json_field(ctx, id, &field, value)?;
    }
    Ok(())
}

/// A `{ui:{…}}` request on the open hosted workspace `key`: the keys apply in order with `commit` and `cancel` last (a
/// JSON object keeps no order of its own), each key is applied or refused on its own (the error names every refused key; the others stay
/// applied), and the reply is the workspace's fields with its `describe` facts.
pub(crate) fn workspace_ui(app: &mut PhotocraftApp, ctx: &egui::Context, key: &str, ui: &Value) -> Result<Value, String> {
    let Some(id) = host(app).open_dialogs().into_iter().find(|d| d.key.as_deref() == Some(key)).map(|d| d.id) else {
        return Err(format!("`{key}` is not open"));
    };
    let Some(request) = ui.as_object() else { return Err("`ui` must be an object".into()) };
    let mut committed = None;
    // Each key is applied or refused on its own: a refused key leaves its own value, the keys after it still apply and
    // the reply names every refusal.
    let mut refusals = Vec::new();
    let (ends, edits): (Vec<_>, Vec<_>) = request.iter().partition(|(name, _)| matches!(name.as_str(), "commit" | "cancel"));
    for (name, value) in edits.into_iter().chain(ends) {
        let step = match name.as_str() {
            "commit" if value.as_bool() != Some(false) => {
                committed = Some(workspace_reply(app, key, id));
                crate::dialogs::confirm(app, ctx, id)
            }
            "cancel" if value.as_bool() != Some(false) => crate::dialogs::cancel(app, ctx, id),
            "waitPreview" => {
                wait_preview(app, ctx);
                Ok(())
            }
            "set" if value.is_object() => {
                for (field, value) in value.as_object().into_iter().flatten() {
                    if let Err(e) = apply_field(app, ctx, id, field, value) {
                        refusals.push(format!("`{field}`: {e}"));
                    }
                }
                Ok(())
            }
            "commit" | "cancel" => Ok(()),
            field => apply_field(app, ctx, id, field, value),
        };
        if let Err(e) = step {
            refusals.push(format!("`{name}`: {e}"));
        }
    }
    crate::dialogs::take_results(app, ctx);
    if !refusals.is_empty() {
        return Err(refusals.join("; "));
    }
    Ok(match committed {
        Some(result) => json!({"committed": true, "result": result}),
        None => workspace_reply(app, key, id),
    })
}
