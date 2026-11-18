//! Window › Workspace over the framework's `WorkspaceSet`, Window › Modifier Keys, and the shell dialogs of View:
//! Save Pixel Aspect Ratio, Show Extras Options and 32-bit Preview Options.
//!
//! The set lives at `app.workspaces` and is written to `Preferences::workspace_set` by [`store`] after every change
//! of it; what a workspace captures beside its layout (shortcuts, menus, toolbar) is in
//! `Preferences::workspace_extras` by workspace id. [`ShellUi`] is the serialisable shell state of `UiState`.

use craft::behavior::dock::{NamedWorkspace, WorkspaceChrome, WorkspaceError, WorkspaceKeeps, WorkspaceSet};
use craft::keymap::{CommandRef, Mods};
use craft::ui::contract::{Bind, CommandSet as _};
use craft::ui::dialog::DialogResult;
use craft::ui::docking::{WorkspaceAsk, WorkspaceCommands, run_workspace, workspace_states};
use craft::ui::patterns::dock::{HostedDeleteWorkspace, HostedNewWorkspace, NewWorkspace, WorkspaceSwitcher};
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamDialog, ParamKind, ParamLayout, ParamResult};
use craft::ui::runtime::commands::CommandState;
use craft::ui::value_drag::SliderScale;
use photocraft_engine::prefs::Preferences;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::panels::panel_docking;

/// A View › Pixel Aspect Ratio › Custom entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CustomPar {
    pub name: String,
    pub ratio: f32,
}

/// The shell's own state. Keys of earlier versions (`modifier_keys`, `sticky_*`, `dialog`) are ignored on load.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShellUi {
    /// The modifiers the Modifier Keys panel holds down; `frame_input` passes them as `FrameInput::latched`.
    pub latched: Mods,
    /// Custom pixel aspect ratios (the active one is `ViewOptions::pixel_aspect` = `custom:<ratio>:<name>`).
    pub custom_pars: Vec<CustomPar>,
}

/// The workspaces of the app.
#[derive(Clone, Debug, Default)]
pub struct Workspaces {
    /// Built-in and saved workspaces, the current one, the lock.
    pub set: WorkspaceSet<String>,
}

/// The built-in workspaces as (id, English name), in menu order (the catalog's Window › Workspace rows).
const BUILTIN: [(&str, &str); 6] = [
    ("essentials", "Essentials"),
    ("photography", "Photography"),
    ("painting", "Painting"),
    ("pixelArt", "Pixel Art"),
    ("graphicAndWeb", "Graphic and Web"),
    ("motion", "Motion"),
];

/// The preference keys a workspace can capture beside its layout.
const EXTRAS: [&str; 3] = ["shortcuts", "menus", "toolbar"];

/// The dialog keys of the shell's parameter dialogs: the ids of the commands they answer.
const CUSTOM_PAR: &str = "view.pixelAspectRatio.custom";
const PREVIEW32: &str = "view.thirtyTwoBitPreviewOptions";
const EXTRAS_OPTIONS: &str = "view.show.showExtrasOptions";

const IDS: [&str; 5] = ["window.panel.modifierKeys", "window.workspace.select", CUSTOM_PAR, EXTRAS_OPTIONS, PREVIEW32];

/// Menu ids handled here (live menu items).
pub fn handles(id: &str) -> bool {
    IDS.contains(&id) || WorkspaceCommands::from_ref(&CommandRef::new(id)).is_some()
}

/// The check mark of a menu id of this module.
pub fn checked(app: &PhotocraftApp, id: &str) -> Option<bool> {
    match id {
        "window.panel.modifierKeys" => Some(panel_docking::is_shown(app, "modifierKeys")),
        CUSTOM_PAR => Some(app.ui.view.pixel_aspect.starts_with("custom:")),
        _ => None,
    }
}

/// Whether a menu id of this module is enabled; `None` when it is not ours.
pub fn is_enabled(app: &PhotocraftApp, id: &str) -> Option<bool> {
    Some(match id {
        PREVIEW32 => app.session.is_enabled(id),
        i if handles(i) => true,
        _ => return None,
    })
}

fn refused(error: WorkspaceError) -> String {
    format!("{error:?}")
}

// ---------------------------------------------------------------------------------------------------------------
// The workspace set.
// ---------------------------------------------------------------------------------------------------------------

/// Builds `app.workspaces` from the preferences: the built-ins from code, the saved part from
/// `workspace_set`. Saves of the earlier format (`workspaces`, `workspace_locked`, the current name in
/// `panel_layout`) move into the set once and are cleared.
pub fn load(app: &mut PhotocraftApp) {
    let prefs = app.session.prefs().clone();
    let mut set = WorkspaceSet::new(
        BUILTIN.iter().map(|&(id, english)| NamedWorkspace::new(id, tl!(english).to_string(), crate::panels::registry::preset(english))).collect(),
    );
    let stored = (!prefs.workspace_set.is_null()).then(|| serde_json::from_value::<WorkspaceSet<String>>(prefs.workspace_set.clone()).ok()).flatten();
    match stored {
        Some(stored) => {
            set.saved = stored.saved;
            set.locked = stored.locked;
            set.edited = stored.edited;
            if set.find(&stored.current).is_some() {
                set.current = stored.current;
            }
            app.workspaces = Workspaces { set };
        }
        None => {
            let moved = migrate(app, &prefs, &mut set);
            app.workspaces = Workspaces { set };
            if moved {
                store(app);
            }
        }
    }
}

/// The one-time move of the earlier format into `set`; true when something moved.
fn migrate(app: &mut PhotocraftApp, prefs: &Preferences, set: &mut WorkspaceSet<String>) -> bool {
    let current = prefs.panel_layout.get("workspace").and_then(Value::as_str).map(str::to_owned);
    if prefs.workspaces.is_empty() && !prefs.workspace_locked && current.is_none() {
        return false;
    }
    let mut extras = std::collections::BTreeMap::new();
    for (name, entry) in &prefs.workspaces {
        // The old entry's layout loads through the dock's own one-time move, into the live workspace and out again.
        let live = app.ui.docking.clone();
        panel_docking::load(app, entry);
        let layout = std::mem::replace(&mut app.ui.docking, live);
        let has = |key: &str| entry.get(key).is_some();
        let keeps = WorkspaceKeeps::new(has("shortcuts"), has("menus"), has("toolbar"));
        let Ok(id) = set.save(name, layout, WorkspaceChrome::default(), keeps) else { continue };
        let captured: Map<String, Value> = EXTRAS.iter().filter_map(|key| entry.get(*key).map(|value| ((*key).to_owned(), value.clone()))).collect();
        if !captured.is_empty() {
            extras.insert(id, Value::Object(captured));
        }
    }
    set.locked = prefs.workspace_locked;
    set.current =
        current.and_then(|name| set.builtin.iter().chain(&set.saved).find(|w| w.name == name).map(|w| w.id.clone())).unwrap_or_else(|| BUILTIN[0].0.to_owned());
    app.session.prefs.edit(|p| {
        p.workspace_extras.extend(extras);
        p.workspaces.clear();
        p.workspace_locked = false;
    });
    true
}

/// Writes the set to the preferences. The one writer of `Preferences::workspace_set`: every change of the set ends
/// here.
fn store(app: &mut PhotocraftApp) {
    let value = serde_json::to_value(&app.workspaces.set).unwrap_or_default();
    app.session.prefs.edit(|p| {
        p.workspace_set = value;
    });
}

/// What `keeps` names, read from the preferences.
fn captured(prefs: &Preferences, keeps: WorkspaceKeeps) -> Map<String, Value> {
    let mut out = Map::new();
    if keeps.shortcuts {
        out.insert("shortcuts".into(), json!(prefs.shortcuts));
    }
    if keeps.menus {
        out.insert("menus".into(), serde_json::to_value(&prefs.menus).unwrap_or_default());
    }
    if keeps.toolbar {
        out.insert("toolbar".into(), serde_json::to_value(&prefs.toolbar).unwrap_or_default());
    }
    out
}

/// Restores the captured shortcuts, menus and toolbar of workspace `id`, as far as its keeps name them.
fn restore_extras(app: &mut PhotocraftApp, id: &str) {
    let Some(keeps) = app.workspaces.set.find(id).map(|w| w.keeps) else { return };
    let Some(extras) = app.session.prefs().workspace_extras.get(id).cloned() else { return };
    let take = |key: &str, on: bool| on.then(|| extras.get(key).cloned()).flatten();
    let (shortcuts, menus, toolbar) = (take("shortcuts", keeps.shortcuts), take("menus", keeps.menus), take("toolbar", keeps.toolbar));
    app.session.prefs.edit(|p| {
        if let Some(v) = shortcuts.and_then(|v| serde_json::from_value(v).ok()) {
            p.shortcuts = v;
        }
        if let Some(v) = menus.and_then(|v| serde_json::from_value(v).ok()) {
            p.menus = v;
        }
        if let Some(v) = toolbar.and_then(|v| serde_json::from_value(v).ok()) {
            p.toolbar = v;
        }
    });
}

/// Makes workspace `id` current: its layout replaces the live one, its captured extras are restored, the dock's
/// layout is remembered. The host's chrome flags (menu bar, rail) stay as the host has them.
pub fn select(app: &mut PhotocraftApp, id: &str) -> Result<(), String> {
    app.workspaces.set.select(id, &mut app.ui.docking).map_err(refused)?;
    restore_extras(app, id);
    crate::dock::persist(app);
    store(app);
    Ok(())
}

/// Saves the live layout under `name` and makes it current; no extras are captured. Returns its id.
pub fn save(app: &mut PhotocraftApp, name: &str) -> Result<String, String> {
    save_keeping(app, name, WorkspaceKeeps::new(false, false, false))
}

fn save_keeping(app: &mut PhotocraftApp, name: &str, keeps: WorkspaceKeeps) -> Result<String, String> {
    let live = app.ui.docking.clone();
    let id = app.workspaces.set.save(name, live, WorkspaceChrome::default(), keeps).map_err(refused)?;
    let extras = captured(app.session.prefs(), keeps);
    app.session.prefs.edit(|p| {
        if extras.is_empty() {
            p.workspace_extras.remove(&id);
        } else {
            p.workspace_extras.insert(id.clone(), Value::Object(extras));
        }
    });
    store(app);
    Ok(id)
}

/// Deletes the saved workspace named `name` (any case).
pub fn delete(app: &mut PhotocraftApp, name: &str) -> Result<(), String> {
    let wanted = name.trim().to_lowercase();
    let id = app
        .workspaces
        .set
        .saved
        .iter()
        .find(|w| w.name.to_lowercase() == wanted)
        .map(|w| w.id.clone())
        .ok_or_else(|| format!("no saved workspace \"{name}\""))?;
    delete_id(app, &id)
}

fn delete_id(app: &mut PhotocraftApp, id: &str) -> Result<(), String> {
    app.workspaces.set.delete(id).map_err(refused)?;
    app.session.prefs.edit(|p| {
        p.workspace_extras.remove(id);
    });
    store(app);
    Ok(())
}

/// Runs a workspace command. New and Delete without a name open their hosted dialog and return its id.
pub fn run(app: &mut PhotocraftApp, _ctx: &egui::Context, command: WorkspaceCommands) -> Result<Value, String> {
    let ask = run_workspace(&mut app.workspaces.set, &mut app.ui.docking, command);
    match ask {
        Some(WorkspaceAsk::Name) => {
            let dialog = HostedNewWorkspace::new(&app.workspaces.set).offered(WorkspaceKeeps::new(true, true, true));
            let id = app.framework.dialogs.open(dialog);
            return Ok(json!({"dialog": id.value().to_string()}));
        }
        Some(WorkspaceAsk::Which) => {
            let id = app.framework.dialogs.open(HostedDeleteWorkspace::new(&app.workspaces.set));
            return Ok(json!({"dialog": id.value().to_string()}));
        }
        Some(_) | None => {}
    }
    if command == WorkspaceCommands::Reset {
        crate::dock::persist(app);
    }
    store(app);
    Ok(if command == WorkspaceCommands::Lock { json!({"locked": app.workspaces.set.locked}) } else { json!({"workspace": app.workspaces.set.current}) })
}

/// Runs the workspace command `command` and says what it did; `None` when it is not a workspace command. New and
/// Delete with a `name` param act on it at once (the control protocol's form); without one they ask.
pub(crate) fn run_command(app: &mut PhotocraftApp, ctx: &egui::Context, command: &CommandRef) -> Option<Result<Value, String>> {
    let parsed = WorkspaceCommands::from_ref(command)?;
    let name = command.params.get("name").and_then(Value::as_str).map(str::to_owned);
    let flag = |key: &str| command.params.get(key).and_then(Value::as_bool).unwrap_or(false);
    Some(match (parsed, name) {
        (WorkspaceCommands::New, Some(name)) => {
            let keeps = WorkspaceKeeps::new(flag("keyboardShortcuts"), flag("menus"), flag("toolbar"));
            save_keeping(app, &name, keeps).map(|id| json!({"workspace": id}))
        }
        (WorkspaceCommands::Delete, Some(name)) => delete(app, &name).map(|()| json!({"deleted": name})),
        (parsed, _) => run(app, ctx, parsed),
    })
}

/// The `run` of the workspace command source: a fired `WorkspaceCommands` row or key. A refusal shows in the status.
pub(crate) fn run_fired(app: &mut PhotocraftApp, ctx: &egui::Context, command: &CommandRef) -> bool {
    let Some(result) = run_command(app, ctx, command) else { return false };
    if let Err(message) = result {
        app.ui.status = message;
        app.ui.status_error = true;
    }
    true
}

/// A `TAKERS` row: the answers of the hosted New and Delete Workspace dialogs.
pub fn take(app: &mut PhotocraftApp, _ctx: &egui::Context) {
    while let Some(result) = app.framework.dialogs.take::<HostedNewWorkspace<String>>() {
        if let DialogResult::Ok(NewWorkspace { name, keeps, .. }) = result {
            // `confirm` refused what `check_name` refuses, so a refusal here has nothing to reopen.
            let _ = save_keeping(app, &name, keeps);
        }
    }
    while let Some(result) = app.framework.dialogs.take::<HostedDeleteWorkspace<String>>() {
        if let DialogResult::Ok(id) = result {
            let _ = delete_id(app, &id);
        }
    }
}

/// The workspace switcher of the title bar.
pub fn switcher(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let out = WorkspaceSwitcher::new(&app.workspaces.set).show(ui);
    if let Some(id) = out.switched
        && let Err(message) = select(app, &id)
    {
        app.ui.status = message;
        app.ui.status_error = true;
    }
}

/// The state of each workspace command, for the command source.
pub fn states(app: &PhotocraftApp) -> Vec<(CommandRef, CommandState)> {
    workspace_states(&app.workspaces.set).into_iter().map(|(command, state)| (CommandRef::from(command), state)).collect()
}

// ---------------------------------------------------------------------------------------------------------------
// Menu commands.
// ---------------------------------------------------------------------------------------------------------------

fn no_params(p: &Value) -> bool {
    p.as_object().is_none_or(|o| o.is_empty())
}

/// Whether `key` is the key of one of the shell's parameter dialogs, for the `PARAM_FAMILIES` row.
pub(crate) fn owns_param(key: &str) -> bool {
    [CUSTOM_PAR, PREVIEW32, EXTRAS_OPTIONS].contains(&key)
}

/// A shell parameter dialog's OK: runs its command with the fields; an error goes to the status line.
pub(crate) fn run_param(app: &mut PhotocraftApp, ctx: &egui::Context, result: ParamResult) {
    let params = serde_json::to_value(&result.fields).unwrap_or_default();
    if let Err(message) = crate::menus::invoke(app, ctx, &result.key, params) {
        app.ui.status = message;
        app.ui.status_error = true;
    }
}

/// Opens the parameter dialog `key` over `fields`; the reply names its id.
fn open_form(app: &mut PhotocraftApp, key: &'static str, title: &'static str, fields: DialogFields, params: Vec<Param>) -> Result<Value, String> {
    // PLAN: assumed dialogs-w2 `params::open_param(app, key, fields, captured: Value, build) -> DialogId`. A held OK
    // with a name check for Save Pixel Aspect Ratio needs a `ParamFamily::check` row (dialogs-w19); `custom_par`
    // refuses an empty name and the status line shows it.
    let id = crate::dialogs::params::open_param(app, key, fields, Value::Null, move |fields| {
        ParamDialog::new(key, tl!(title), params.clone()).values(Bind::bound(fields)).layout(ParamLayout::Stacked)
    });
    Ok(json!({"dialog": id.value().to_string()}))
}

fn open_custom_par(app: &mut PhotocraftApp) -> Result<Value, String> {
    let fields = DialogFields::from_pairs([("name", FieldValue::Text(String::new())), ("ratio", FieldValue::Number(1.0))]);
    let params =
        vec![Param::new("name", tl!("Name:"), ParamKind::text("")), Param::new("ratio", tl!("Factor:"), ParamKind::range(0.1, 10.0, 1.0, SliderScale::Linear))];
    open_form(app, CUSTOM_PAR, "Save Pixel Aspect Ratio", fields, params)
}

fn open_extras(app: &mut PhotocraftApp) -> Result<Value, String> {
    let show = serde_json::to_value(app.ui.view.show).unwrap_or_default();
    let mut fields = DialogFields::new();
    let mut params = Vec::new();
    for (key, label) in extras_options() {
        let on = show.get(key).and_then(Value::as_bool).unwrap_or(false);
        fields.set(key, FieldValue::Bool(on));
        params.push(Param::new(key, label, ParamKind::bool(on)));
    }
    open_form(app, EXTRAS_OPTIONS, "Show Extras Options", fields, params)
}

fn open_preview32(app: &mut PhotocraftApp) -> Option<Result<Value, String>> {
    let d = app.session.active()?;
    let h = app.session.color.hdr.get(&d.doc.id).copied().unwrap_or_default();
    let method = if h.highlight_compression { "highlightCompression" } else { "exposureGamma" };
    let fields = DialogFields::from_pairs([
        ("method", FieldValue::Text(method.into())),
        ("exposure", FieldValue::Number(f64::from(h.exposure))),
        ("gamma", FieldValue::Number(f64::from(h.gamma))),
    ]);
    let methods =
        vec![("exposureGamma".to_owned(), tl!("Exposure and Gamma").into()), ("highlightCompression".to_owned(), tl!("Highlight Compression").into())];
    let params = vec![
        Param::new("method", tl!("Mode"), ParamKind::choice(methods, Some("exposureGamma".into()))),
        Param::new("exposure", tl!("Exposure:"), ParamKind::range(-20.0, 20.0, 0.0, SliderScale::Linear)).enabled_when("method", "exposureGamma"),
        Param::new("gamma", tl!("Gamma:"), ParamKind::range(0.1, 9.99, 1.0, SliderScale::Linear)).enabled_when("method", "exposureGamma"),
    ];
    Some(open_form(app, PREVIEW32, "32-bit Preview Options", fields, params))
}

/// Run a shell command, or open its dialog. `None` when `id` isn't ours.
pub fn menu(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id == PREVIEW32 && no_params(params) {
        return open_preview32(app);
    }
    if !handles(id) {
        return None;
    }
    Some(match id {
        "window.panel.modifierKeys" => {
            let latched = &mut app.ui.shell.latched;
            for (key, slot) in [("shift", &mut latched.shift), ("command", &mut latched.cmd), ("alt", &mut latched.alt)] {
                if let Some(v) = params.get(key).and_then(Value::as_bool) {
                    *slot = v;
                }
            }
            let latched = app.ui.shell.latched;
            let shown = panel_docking::show_panel(app, "modifierKeys", params.get("show").and_then(Value::as_bool));
            shown.map(|visible| json!({"visible": visible, "shift": latched.shift, "command": latched.cmd, "alt": latched.alt}))
        }
        "window.workspace.select" => {
            let name = params.get("name").and_then(Value::as_str).ok_or("pass `name`");
            name.map_err(String::from).and_then(|name| {
                let id = app.workspaces.set.builtin.iter().chain(&app.workspaces.set.saved).find(|w| w.name == name || w.id == name).map(|w| w.id.clone());
                let id = id.ok_or_else(|| format!("no workspace \"{name}\""))?;
                select(app, &id)?;
                Ok(json!({"workspace": name}))
            })
        }
        CUSTOM_PAR if no_params(params) => open_custom_par(app),
        CUSTOM_PAR => custom_par(app, params),
        EXTRAS_OPTIONS if no_params(params) => open_extras(app),
        EXTRAS_OPTIONS => {
            let mut show = serde_json::to_value(app.ui.view.show).unwrap_or_default();
            if let (Some(o), Some(p)) = (show.as_object_mut(), params.as_object()) {
                for (k, v) in p {
                    if !o.contains_key(k) || !v.is_boolean() {
                        return Some(Err(format!("unknown Show item `{k}` ({})", o.keys().cloned().collect::<Vec<_>>().join("|"))));
                    }
                    o.insert(k.clone(), v.clone());
                }
            }
            app.ui.view.show = serde_json::from_value(show.clone()).map_err(|e| e.to_string()).ok()?;
            Ok(show)
        }
        _ => return None,
    })
}

/// The saved ratio a `{name, ratio}` request names, or why it names none.
fn parse_custom_par(p: &Value) -> Result<CustomPar, String> {
    let name = p.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()).ok_or("pass `name`")?.to_string();
    let ratio = p.get("ratio").and_then(Value::as_f64).ok_or("pass `ratio`")? as f32;
    if !(0.1..=10.0).contains(&ratio) {
        return Err("the ratio must be between 0.100 and 10.000".into());
    }
    Ok(CustomPar { name, ratio })
}

/// Judges a shell parameter dialog's OK before it closes: the Custom Pixel Aspect Ratio needs a name and a ratio in range.
pub(crate) fn check_param(_app: &mut PhotocraftApp, result: &ParamResult) -> Result<(), craft::ui::runtime::env::Label> {
    if result.key == CUSTOM_PAR {
        parse_custom_par(&serde_json::to_value(&result.fields).unwrap_or_default()).map_err(craft::ui::runtime::env::Label::from)?;
    }
    Ok(())
}

/// Custom Pixel Aspect Ratio: `{name, ratio}` adds (or updates) and selects; `{delete: name}`
/// removes; `{select: name}` selects a saved one.
fn custom_par(app: &mut PhotocraftApp, p: &Value) -> Result<Value, String> {
    let s = &mut app.ui.shell;
    if let Some(n) = p.get("delete").and_then(Value::as_str) {
        let before = s.custom_pars.len();
        s.custom_pars.retain(|c| c.name != n);
        if s.custom_pars.len() == before {
            return Err(format!("no custom pixel aspect ratio \"{n}\""));
        }
        if app.ui.view.pixel_aspect.ends_with(&format!(":{n}")) {
            app.ui.view.pixel_aspect = "square".into();
        }
        return Ok(json!({"deleted": n}));
    }
    let c = if let Some(n) = p.get("select").and_then(Value::as_str) {
        s.custom_pars.iter().find(|c| c.name == n).cloned().ok_or_else(|| format!("no custom pixel aspect ratio \"{n}\""))?
    } else {
        let c = parse_custom_par(p)?;
        match s.custom_pars.iter_mut().find(|x| x.name == c.name) {
            Some(x) => *x = c.clone(),
            None => s.custom_pars.push(c.clone()),
        }
        c
    };
    app.ui.view.pixel_aspect = format!("custom:{}:{}", c.ratio, c.name);
    Ok(json!({"pixelAspectRatio": c.ratio, "name": c.name}))
}

/// The Show Extras Options… checkboxes as (field, label). The fields are the serialized
/// [`crate::view_cmds::Show`] names, the keys the dialog is seeded with and its command accepts.
fn extras_options() -> [(&'static str, &'static str); 13] {
    [
        ("layer_edges", tl!("Layer Edges")),
        ("selection_edges", tl!("Selection Edges")),
        ("target_path", tl!("Target Path")),
        ("notes", tl!("Notes")),
        ("pixel_grid", tl!("Pixel Grid")),
        ("slices", tl!("Slices")),
        ("count", tl!("Count")),
        ("smart_guides", tl!("Smart Guides")),
        ("brush_preview", tl!("Brush Preview")),
        ("mesh", tl!("Mesh")),
        ("edit_pins", tl!("Edit Pins")),
        ("canvas_guides", tl!("Canvas Guides")),
        ("artboard_guides", tl!("Artboard Guides")),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> (PhotocraftApp, egui::Context) {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = app.ctx().clone();
        (app, ctx)
    }

    fn shown(app: &PhotocraftApp, panel: &str) -> bool {
        panel_docking::is_shown(app, panel)
    }

    #[test]
    fn saved_workspaces_preserve_host_chrome_on_select_and_reset() {
        for menu_bar in [false, true] {
            let (mut app, ctx) = app();
            panel_docking::show_panel(&mut app, "character", Some(true)).unwrap();
            panel_docking::show_panel(&mut app, "paragraph", Some(true)).unwrap();
            let id = save(&mut app, "Host layout").unwrap();
            app.ui.panels.menu_bar = menu_bar;
            select(&mut app, "painting").unwrap();
            assert!(!shown(&app, "character"));
            select(&mut app, &id).unwrap();
            assert_eq!(app.ui.panels.menu_bar, menu_bar, "select");
            assert!(shown(&app, "character") && shown(&app, "paragraph"));
            app.ui.panels.menu_bar = !menu_bar;
            panel_docking::show_panel(&mut app, "character", Some(false)).unwrap();
            run(&mut app, &ctx, WorkspaceCommands::Reset).unwrap();
            assert_eq!(app.ui.panels.menu_bar, !menu_bar, "reset");
            assert!(shown(&app, "character") && shown(&app, "paragraph"), "reset restores the saved layout");
        }
    }

    #[test]
    fn new_select_delete_workspaces() {
        let (mut app, _ctx) = app();
        panel_docking::show_panel(&mut app, "character", Some(true)).unwrap();
        app.session.prefs.edit(|p| {
            p.shortcuts.insert("filter.blur.gaussian".into(), "Cmd+Alt+G".into());
        });
        let id = save_keeping(&mut app, "Retouch", WorkspaceKeeps::new(true, false, false)).unwrap();
        assert_eq!(app.workspaces.set.current, id);
        assert!(app.session.prefs().workspace_extras.contains_key(&id), "the Capture boxes are stored");
        select(&mut app, "painting").unwrap();
        app.session.prefs.edit(|p| p.shortcuts.clear());
        select(&mut app, &id).unwrap();
        assert!(shown(&app, "character"));
        assert_eq!(app.session.prefs().shortcuts.get("filter.blur.gaussian").map(String::as_str), Some("Cmd+Alt+G"));
        // The set persists in the preferences.
        let stored: WorkspaceSet<String> = serde_json::from_value(app.session.prefs().workspace_set.clone()).unwrap();
        assert_eq!((stored.current.as_str(), stored.saved.len()), (id.as_str(), 1));
        // The current workspace cannot be deleted; another can, and its extras go with it.
        assert!(delete(&mut app, "retouch").is_err());
        select(&mut app, "essentials").unwrap();
        delete(&mut app, "RETOUCH").unwrap();
        assert!(app.workspaces.set.saved.is_empty());
        assert!(!app.session.prefs().workspace_extras.contains_key(&id));
        // An earlier save's map moves into the set once.
        app.session.prefs.edit(|p| {
            p.workspace_set = Value::Null;
            p.workspaces.insert("Old".into(), json!({"shortcuts": {"a": "b"}}));
        });
        load(&mut app);
        let old = app.workspaces.set.saved.iter().find(|w| w.name == "Old").expect("moved");
        assert!(old.keeps.shortcuts);
        assert!(app.session.prefs().workspaces.is_empty());
        let old_id = old.id.clone();
        assert!(app.session.prefs().workspace_extras.contains_key(&old_id));
    }

    #[test]
    fn shell_view_commands_take_their_params() {
        let (mut app, _ctx) = app();
        let ok = |app: &mut PhotocraftApp, id: &str, p: Value| menu(app, id, &p).expect("handled").unwrap();
        let err = |app: &mut PhotocraftApp, id: &str, p: Value| menu(app, id, &p).expect("handled").unwrap_err();

        ok(&mut app, CUSTOM_PAR, json!({"name": "Cinema", "ratio": 1.25}));
        assert_eq!(crate::view_cmds::pixel_aspect_ratio(&app.ui.view.pixel_aspect), 1.25);
        assert_eq!(checked(&app, CUSTOM_PAR), Some(true));
        err(&mut app, CUSTOM_PAR, json!({"name": "x", "ratio": 50}));
        err(&mut app, CUSTOM_PAR, json!({"name": "x", "ratio": 0.05}));
        app.ui.view.pixel_aspect = "square".into();
        ok(&mut app, CUSTOM_PAR, json!({"select": "Cinema"}));
        assert_eq!(crate::view_cmds::pixel_aspect_ratio(&app.ui.view.pixel_aspect), 1.25);
        ok(&mut app, CUSTOM_PAR, json!({"delete": "Cinema"}));
        assert_eq!(app.ui.view.pixel_aspect, "square");

        // Every Show key reaches its flag; an unknown key is refused.
        let before = serde_json::to_value(app.ui.view.show).unwrap();
        let flipped: Map<String, Value> = extras_options().iter().map(|(k, _)| ((*k).to_owned(), json!(!before[*k].as_bool().unwrap()))).collect();
        ok(&mut app, EXTRAS_OPTIONS, Value::Object(flipped));
        let after = serde_json::to_value(app.ui.view.show).unwrap();
        for (k, _) in extras_options() {
            assert_eq!(after[k].as_bool(), before[k].as_bool().map(|b| !b), "{k}");
        }
        err(&mut app, EXTRAS_OPTIONS, json!({"bogus": true}));
    }

    #[test]
    fn thirty_two_bit_preview_dialog() {
        let (mut app, ctx) = app();
        app.run("file.new", json!({"width": 20, "height": 20, "depth": 32})).unwrap();
        menu(&mut app, PREVIEW32, &json!({})).expect("handled").unwrap();
        let id = app.framework.dialogs.open_dialogs().last().expect("the dialog is open").id;
        for (field, value) in [("exposure", json!(1.5)), ("gamma", json!(1.5))] {
            app.framework.dialogs.set_json_field(&ctx, id, field, value).unwrap();
        }
        crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
        let d = app.session.active().unwrap().doc.id;
        assert_eq!((app.session.color.hdr[&d].exposure, app.session.color.hdr[&d].gamma), (1.5, 1.5));

        // Highlight Compression disables the two ranges.
        menu(&mut app, PREVIEW32, &json!({})).expect("handled").unwrap();
        let id = app.framework.dialogs.open_dialogs().last().unwrap().id;
        app.framework.dialogs.set_json_field(&ctx, id, "method", json!("highlightCompression")).unwrap();
        assert!(app.framework.dialogs.set_json_field(&ctx, id, "exposure", json!(2.0)).is_err());
        assert!(app.framework.dialogs.set_json_field(&ctx, id, "gamma", json!(2.0)).is_err());
    }
}
