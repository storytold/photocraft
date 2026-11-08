//! Shareable Window › Workspace files (`.pcraft-workspace`).
//!
//! A file is JSON with `"photocraftWorkspace": 1`. Two shapes are accepted:
//!
//! - A preset names the panels to show (`navigator`, `color`, `layers`, `history`,
//!   `properties`, `character`, `timeline`) and may name a theme id. This is the shape to
//!   commit when sharing a layout.
//! - A snapshot is the layout New Workspace stores (`panels`, `dock`, `docking`, …) plus
//!   `name` and an optional `theme`. Export writes this shape.
//!
//! Files under `crates/ui-egui/workspaces/` ship with the app. Register each one in
//! [`BUNDLED`] (one line). A bundled name is reserved: importing it applies the file and
//! does not add a second copy to the saved workspaces. Give a custom file its own name to
//! keep it. A preferences entry that already uses a bundled name stays in the menu as
//! "{name} (saved)" and does not replace the sample. Built-in preset names (Essentials,
//! Photography, …) are rejected. Reset and those presets do not change the theme; a file
//! changes it only when it has `theme`.

use std::sync::OnceLock;

use serde_json::{Map, Value, json};

use super::items::MenuItem;
use super::workspace_ui::PRESETS;
use crate::PhotocraftApp;
use crate::theme::ThemeKind;

/// Menu ids of the bundled samples: `window.workspace.bundled.<index>`.
pub(crate) const PREFIX: &str = "window.workspace.bundled.";

/// One mebibyte. Larger files are refused before they are parsed.
const MAX_BYTES: usize = 1024 * 1024;

const PANEL_KEYS: [&str; 7] = ["navigator", "color", "layers", "history", "properties", "character", "timeline"];

/// `(filename, contents)`. Add a line when adding a file under `workspaces/`.
const BUNDLED: &[(&str, &str)] = &[
    ("cs4.pcraft-workspace", include_str!("../../workspaces/cs4.pcraft-workspace")),
    ("cs5.pcraft-workspace", include_str!("../../workspaces/cs5.pcraft-workspace")),
];

struct WorkspaceFile {
    name: String,
    theme: Option<String>,
    body: Value,
    snapshot: bool,
}

fn bundled() -> &'static [WorkspaceFile] {
    static FILES: OnceLock<Vec<WorkspaceFile>> = OnceLock::new();
    FILES.get_or_init(|| BUNDLED.iter().filter_map(|(_, text)| parse(text.as_bytes()).ok()).collect())
}

pub(crate) fn bundled_names() -> Vec<String> {
    bundled().iter().map(|file| file.name.clone()).collect()
}

pub(crate) fn is_bundled(name: &str) -> bool {
    bundled().iter().any(|file| file.name == name)
}

fn bundled_by_name(name: &str) -> Option<&'static WorkspaceFile> {
    bundled().iter().find(|file| file.name == name)
}

pub(crate) fn bundled_name(id: &str) -> Option<String> {
    let index = id.strip_prefix(PREFIX)?.parse::<usize>().ok()?;
    bundled().get(index).map(|file| file.name.clone())
}

/// The theme id on a command result, when it names a real theme.
pub(crate) fn theme_in(value: &Value) -> Option<ThemeKind> {
    let id = value.get("theme")?.as_str()?;
    ThemeKind::ALL.into_iter().find(|kind| kind.id() == id)
}

/// Apply the bundled file for the current workspace name. Returns whether one applied.
pub(crate) fn apply_current_bundle(app: &mut PhotocraftApp) -> bool {
    let Some(file) = bundled_by_name(&app.ui.workspace) else { return false };
    apply_file(app, file);
    true
}

pub(crate) fn insert_menu_items(app: &PhotocraftApp, items: &mut Vec<MenuItem>) {
    let names = bundled_names();
    if names.is_empty() {
        return;
    }
    let after = items
        .iter()
        .rposition(|item| item.id.starts_with(super::workspace_ui::SAVED))
        .or_else(|| items.iter().position(|item| item.id == "window.workspace.motion"));
    let Some(after) = after else { return };
    let path = vec!["Window".to_string(), "Workspace".to_string()];
    let mut extra = Vec::with_capacity(names.len() + 1);
    extra.push(MenuItem { id: "---".into(), label: "---".into(), path: path.clone(), shortcut: None, enabled: false, checked: None, color: None });
    for (index, name) in names.into_iter().enumerate() {
        let id = format!("{PREFIX}{index}");
        let checked = Some(app.ui.workspace == name);
        extra.push(MenuItem { id, label: name, path: path.clone(), shortcut: None, enabled: true, checked, color: None });
    }
    for (offset, item) in extra.into_iter().enumerate() {
        items.insert(after + 1 + offset, item);
    }
}

pub(crate) fn import(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    if let Some(path) = params.get("path").and_then(Value::as_str).filter(|path| !path.is_empty()) {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = path;
            return Err("open a workspace file from the file dialog on the web".into());
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            return import_path(app, path);
        }
    }
    app.pick_file_bytes_filtered(&["pcraft-workspace", "json"], |app, _name, bytes| import_bytes(app, &bytes))
}

pub(crate) fn export(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    let name = export_name(app, params)?;
    let bytes = encode(&capture(app, &name))?;
    if let Some(path) = params.get("path").and_then(Value::as_str).filter(|path| !path.is_empty()) {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (path, bytes);
            return Err("save a workspace file from the file dialog on the web".into());
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            write_export_path(app, path, &bytes)?;
            return Ok(json!({"path": path, "workspace": name}));
        }
    }
    let suggested = format!("{name}.pcraft-workspace");
    app.pick_save(&suggested, move |app, path| {
        write_bytes(app, &path, &bytes)?;
        Ok(json!({"path": path, "workspace": name}))
    })
}

fn export_name(app: &PhotocraftApp, params: &Value) -> Result<String, String> {
    if let Some(name) = params.get("name").and_then(Value::as_str) {
        return shareable_name(name);
    }
    let current = app.ui.workspace.trim();
    if shareable_name(current).is_ok() {
        return Ok(current.to_string());
    }
    Ok("My Workspace".to_string())
}

fn shareable_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    validate_name(name)?;
    if PRESETS.contains(&name) {
        return Err(format!("\"{name}\" is a built-in workspace"));
    }
    Ok(name.to_string())
}

fn validate_name(name: &str) -> Result<(), String> {
    let count = name.chars().count();
    if !(1..=80).contains(&count) {
        return Err("a workspace name must be 1 to 80 characters".into());
    }
    if name.chars().any(char::is_control) {
        return Err("a workspace name cannot contain control characters".into());
    }
    Ok(())
}

fn known_theme(id: &str) -> Result<(), String> {
    if ThemeKind::ALL.iter().any(|kind| kind.id() == id) { Ok(()) } else { Err(format!("unknown theme `{id}`")) }
}

fn parse(bytes: &[u8]) -> Result<WorkspaceFile, String> {
    if bytes.len() > MAX_BYTES {
        return Err("the workspace file is too large".into());
    }
    let body: Value = serde_json::from_slice(bytes).map_err(|_| "not a PhotoCraft workspace file".to_string())?;
    let obj = body.as_object().ok_or("not a PhotoCraft workspace file")?;
    if obj.get("photocraftWorkspace").and_then(Value::as_u64) != Some(1) {
        return Err("this workspace file needs photocraftWorkspace version 1".into());
    }
    let name = obj.get("name").and_then(Value::as_str).map(str::trim).unwrap_or("");
    let name = shareable_name(name)?;
    let theme = match obj.get("theme") {
        None | Some(Value::Null) => None,
        Some(Value::String(id)) => {
            known_theme(id)?;
            Some(id.clone())
        }
        Some(_) => return Err("theme must be a theme id".into()),
    };
    let snapshot = obj.contains_key("dock") || obj.contains_key("docking");
    if !snapshot {
        let panels = obj.get("panels").and_then(Value::as_object).ok_or("a workspace file needs panels")?;
        for key in PANEL_KEYS {
            match panels.get(key) {
                Some(Value::Bool(_)) => {}
                Some(_) => return Err(format!("panels.{key} must be true or false")),
                None => return Err(format!("panels.{key} is missing")),
            }
        }
    }
    Ok(WorkspaceFile { name, theme, body, snapshot })
}

#[cfg(not(target_arch = "wasm32"))]
fn import_path(app: &mut PhotocraftApp, path: &str) -> Result<Value, String> {
    let bytes = read_workspace_bytes(app, path)?;
    import_bytes(app, &bytes)
}

/// A path on a control request goes through the authorized roots. The file dialog keeps
/// [`write_bytes`].
#[cfg(not(target_arch = "wasm32"))]
fn read_workspace_bytes(app: &mut PhotocraftApp, path: &str) -> Result<Vec<u8>, String> {
    let automated = app.automation_input || app.services.automation_read.is_some();
    if automated {
        let read = app.services.automation_read.as_mut().ok_or_else(|| "automation read authority is not configured".to_string())?;
        let (_name, bytes) = read(path)?;
        if bytes.len() > MAX_BYTES {
            return Err("the workspace file is too large".into());
        }
        return Ok(bytes);
    }
    let meta = std::fs::metadata(path).map_err(|err| err.to_string())?;
    if meta.len() > MAX_BYTES as u64 {
        return Err("the workspace file is too large".into());
    }
    std::fs::read(path).map_err(|err| err.to_string())
}

#[cfg(not(target_arch = "wasm32"))]
fn write_export_path(app: &mut PhotocraftApp, path: &str, bytes: &[u8]) -> Result<(), String> {
    let automated = app.automation_input || app.services.automation_write.is_some();
    if automated {
        let write = app.services.automation_write.as_mut().ok_or_else(|| "automation write authority is not configured".to_string())?;
        return write(path, bytes);
    }
    write_bytes(app, path, bytes)
}

fn import_bytes(app: &mut PhotocraftApp, bytes: &[u8]) -> Result<Value, String> {
    let file = parse(bytes)?;
    let theme = file.theme.clone();
    let name = file.name.clone();
    apply_file(app, &file);
    if !is_bundled(&name) {
        let stored = stored_body(app, &file);
        app.session.prefs.edit(|prefs| {
            prefs.workspaces.insert(name.clone(), stored);
        });
    }
    Ok(reply(&name, theme.as_deref()))
}

fn stored_body(app: &PhotocraftApp, file: &WorkspaceFile) -> Value {
    if file.snapshot {
        return file.body.clone();
    }
    let mut shot = capture(app, &file.name);
    if let Some(obj) = shot.as_object_mut() {
        match &file.theme {
            Some(theme) => {
                obj.insert("theme".into(), json!(theme));
            }
            None => {
                obj.remove("theme");
            }
        }
    }
    shot
}

fn apply_file(app: &mut PhotocraftApp, file: &WorkspaceFile) {
    app.ui.workspace = file.name.clone();
    if file.snapshot {
        app.ui.dock = Default::default();
        crate::dock::apply(app, &file.body);
        return;
    }
    reset_shell(app);
    let panels = file.body.get("panels").and_then(Value::as_object);
    let flag = |key: &str| panels.and_then(|panels| panels.get(key)).and_then(Value::as_bool).unwrap_or(false);
    let ui_panels = &mut app.ui.panels;
    ui_panels.navigator = flag("navigator");
    ui_panels.color = flag("color");
    ui_panels.layers = flag("layers");
    ui_panels.history = flag("history");
    ui_panels.properties = flag("properties");
    ui_panels.character = flag("character");
    app.ui.timeline.open = flag("timeline");
    if !app.ui.timeline.open {
        app.ui.timeline.playing = false;
    }
}

fn reset_shell(app: &mut PhotocraftApp) {
    app.ui.dock = Default::default();
    app.ui.docking_reveal = None;
    app.ui.docking_generation = app.ui.docking_generation.wrapping_add(1);
    app.ui.docking = None;
    app.ui.docking_kept = None;
    app.ui.docking_returns.clear();
    app.ui.docking_collapsed.clear();
    app.ui.dock_tabs = Default::default();
}

fn capture(app: &PhotocraftApp, name: &str) -> Value {
    let mut ws = json!({
        "photocraftWorkspace": 1,
        "name": name,
        "theme": app.ui.theme.id(),
        "panels": app.ui.panels,
        "dockTabs": app.ui.dock_tabs,
        "dock": app.ui.dock,
        "timelineOpen": app.ui.timeline.open,
        "docking": app.ui.docking,
        "dockingReturns": app.ui.docking_returns,
        "dockingCollapsed": app.ui.docking_collapsed,
    });
    if let Some(panels) = ws.get_mut("panels").and_then(Value::as_object_mut) {
        panels.remove("rail");
        panels.remove("menu_bar");
    }
    ws
}

fn encode(value: &Value) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|err| err.to_string())?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn write_bytes(app: &mut PhotocraftApp, path: &str, bytes: &[u8]) -> Result<(), String> {
    if let Some(write) = app.services.write.as_mut() {
        return write(path, bytes);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::write(path, bytes).map_err(|err| err.to_string())
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (path, bytes);
        Err("saving a workspace file needs the file dialog".into())
    }
}

fn reply(name: &str, theme: Option<&str>) -> Value {
    let mut value = Map::new();
    value.insert("workspace".into(), json!(name));
    if let Some(theme) = theme {
        value.insert("theme".into(), json!(theme));
    }
    Value::Object(value)
}

/// Result of selecting `name` when `theme` should be applied by the caller.
pub(crate) fn reply_for(name: &str, theme: Option<&str>) -> Value {
    reply(name, theme)
}

pub(crate) fn bundled_theme(name: &str) -> Option<Option<String>> {
    bundled_by_name(name).map(|file| file.theme.clone())
}

pub(crate) fn apply_bundled(app: &mut PhotocraftApp, name: &str) -> bool {
    let Some(file) = bundled_by_name(name) else { return false };
    apply_file(app, file);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn app() -> (PhotocraftApp, egui::Context) {
        (PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default()), egui::Context::default())
    }

    fn panels_of(navigator: bool, color: bool, layers: bool, history: bool, properties: bool, character: bool, timeline: bool) -> Value {
        json!({
            "navigator": navigator,
            "color": color,
            "layers": layers,
            "history": history,
            "properties": properties,
            "character": character,
            "timeline": timeline
        })
    }

    #[test]
    fn bundled_samples_parse_with_their_themes() {
        let names = bundled_names();
        assert_eq!(names, ["CS4", "CS5"]);
        let cs4 = bundled_by_name("CS4").expect("cs4");
        let cs5 = bundled_by_name("CS5").expect("cs5");
        assert_eq!(cs4.theme.as_deref(), Some("cs4"));
        assert_eq!(cs5.theme.as_deref(), Some("cs5"));
        assert!(!cs4.snapshot && !cs5.snapshot);
        assert_eq!(cs4.body["panels"]["navigator"], true);
        assert_eq!(cs5.body["panels"]["navigator"], false);
        assert_eq!(cs4.body["panels"]["history"], true);
        assert_eq!(cs5.body["panels"]["history"], true);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn every_workspace_file_is_registered() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("workspaces");
        let mut found = Vec::new();
        for entry in std::fs::read_dir(&dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display())) {
            let entry = entry.expect("workspace directory entry");
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(".pcraft-workspace") {
                found.push(name);
            }
        }
        found.sort();
        let mut registered: Vec<&str> = BUNDLED.iter().map(|(name, _)| *name).collect();
        registered.sort();
        assert_eq!(found, registered);
    }

    #[test]
    fn bad_files_fail_without_a_panic() {
        assert!(parse(b"").is_err());
        assert!(parse(b"{}").is_err());
        assert!(parse(br#"{"photocraftWorkspace":2,"name":"Mine","panels":{}}"#).is_err());
        let panels = panels_of(false, true, true, false, true, false, false);
        let essentials = json!({"photocraftWorkspace": 1, "name": "Essentials", "panels": panels});
        assert!(parse(&serde_json::to_vec(&essentials).unwrap()).is_err());
        let mut bad_theme =
            json!({"photocraftWorkspace": 1, "name": "Mine", "theme": "not-a-theme", "panels": panels_of(false, true, true, false, true, false, false)});
        assert!(parse(&serde_json::to_vec(&bad_theme).unwrap()).is_err());
        bad_theme["theme"] = json!("cs4");
        bad_theme["panels"]["history"] = json!("yes");
        assert!(parse(&serde_json::to_vec(&bad_theme).unwrap()).is_err());
        assert!(parse(&vec![b' '; MAX_BYTES + 1]).is_err());
        assert!(parse(br#"{"photocraftWorkspace":1,"name":"","panels":{}}"#).is_err());
    }

    #[test]
    fn selecting_cs4_and_cs5_sets_panels_and_theme_and_presets_keep_the_theme() {
        let (mut app, ctx) = app();
        let result = crate::menus::invoke(&mut app, &ctx, "window.workspace.select", json!({"name": "CS4"})).unwrap();
        assert_eq!(result["theme"], "cs4");
        assert_eq!(app.ui.theme, ThemeKind::Cs4);
        assert!(app.ui.panels.navigator && app.ui.panels.color && app.ui.panels.layers && app.ui.panels.history && app.ui.panels.properties);
        assert!(!app.ui.panels.character && !app.ui.timeline.open);
        crate::menus::invoke(&mut app, &ctx, "window.workspace.essentials", json!({})).unwrap();
        assert_eq!(app.ui.workspace, "Essentials");
        assert_eq!(app.ui.theme, ThemeKind::Cs4, "Essentials leaves the theme");
        assert!(!app.ui.panels.history);
        let result = crate::menus::invoke(&mut app, &ctx, "window.workspace.select", json!({"name": "CS5"})).unwrap();
        assert_eq!(result["theme"], "cs5");
        assert_eq!(app.ui.theme, ThemeKind::Cs5);
        assert!(!app.ui.panels.navigator && app.ui.panels.history && app.ui.panels.color && app.ui.panels.layers && app.ui.panels.properties);
        app.ui.panels.history = false;
        crate::menus::invoke(&mut app, &ctx, "window.workspace.resetWorkspace", json!({})).unwrap();
        assert!(app.ui.panels.history, "reset reapplies the CS5 layout");
        assert_eq!(app.ui.theme, ThemeKind::Cs5, "reset leaves the theme");
        crate::menus::invoke(&mut app, &ctx, "window.workspace.photography", json!({})).unwrap();
        assert_eq!(app.ui.theme, ThemeKind::Cs5);
        assert!(crate::menus::invoke(&mut app, &ctx, "window.workspace.newWorkspace", json!({"name": "CS4"})).is_err());
        assert!(crate::menus::invoke(&mut app, &ctx, "window.workspace.newWorkspace", json!({"name": "CS5"})).is_err());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn export_and_import_round_trip_and_a_bundled_name_is_not_saved() {
        let (mut app, ctx) = app();
        crate::menus::invoke(&mut app, &ctx, "window.workspace.select", json!({"name": "CS4"})).unwrap();
        app.ui.panels.navigator = false;
        let dir = std::env::temp_dir().join(format!("photocraft-ws-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mine.pcraft-workspace");
        let path_str = path.to_string_lossy().into_owned();
        crate::menus::invoke(&mut app, &ctx, "window.workspace.exportWorkspace", json!({"name": "Retouch Desk", "path": path_str})).unwrap();
        let saved = std::fs::read(&path).unwrap();
        let parsed = parse(&saved).unwrap();
        assert_eq!(parsed.name, "Retouch Desk");
        assert_eq!(parsed.theme.as_deref(), Some("cs4"));
        assert!(parsed.snapshot);
        crate::menus::invoke(&mut app, &ctx, "window.workspace.essentials", json!({})).unwrap();
        assert!(!app.ui.panels.history);
        let imported = crate::menus::invoke(&mut app, &ctx, "window.workspace.importWorkspace", json!({"path": path_str})).unwrap();
        assert_eq!(imported["workspace"], "Retouch Desk");
        assert_eq!(imported["theme"], "cs4");
        assert_eq!(app.ui.theme, ThemeKind::Cs4);
        assert!(!app.ui.panels.navigator);
        assert!(app.session.prefs().workspaces.contains_key("Retouch Desk"));
        let again = crate::menus::invoke(&mut app, &ctx, "window.workspace.essentials", json!({})).unwrap();
        assert!(again.get("theme").is_none());
        crate::menus::invoke(&mut app, &ctx, "window.workspace.select", json!({"name": "Retouch Desk"})).unwrap();
        assert_eq!(app.ui.theme, ThemeKind::Cs4);
        assert!(!app.ui.panels.navigator);

        let bundled_path = dir.join("cs4.pcraft-workspace");
        let bundled_str = bundled_path.to_string_lossy().into_owned();
        std::fs::write(&bundled_path, BUNDLED[0].1).unwrap();
        crate::menus::invoke(&mut app, &ctx, "window.workspace.importWorkspace", json!({"path": bundled_str})).unwrap();
        assert!(!app.session.prefs().workspaces.contains_key("CS4"));
        assert!(app.ui.panels.navigator);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_control_path_uses_the_automation_roots() {
        let (mut app, ctx) = app();
        app.automation_input = true;
        let missing = crate::menus::invoke(&mut app, &ctx, "window.workspace.importWorkspace", json!({"path": "mine.pcraft-workspace"})).unwrap_err();
        assert!(missing.contains("automation read authority"), "{missing}");
        let refused = crate::menus::invoke(&mut app, &ctx, "window.workspace.exportWorkspace", json!({"name": "Retouch Desk", "path": "out.pcraft-workspace"}))
            .unwrap_err();
        assert!(refused.contains("automation write authority"), "{refused}");

        let bytes = BUNDLED[0].1.as_bytes().to_vec();
        app.services.automation_read = Some(Box::new(move |path| {
            assert_eq!(path, "cs4.pcraft-workspace");
            Ok(("cs4.pcraft-workspace".into(), bytes.clone()))
        }));
        let imported = crate::menus::invoke(&mut app, &ctx, "window.workspace.importWorkspace", json!({"path": "cs4.pcraft-workspace"})).unwrap();
        assert_eq!(imported["workspace"], "CS4");
        assert_eq!(imported["theme"], "cs4");
        assert_eq!(app.ui.theme, ThemeKind::Cs4);
        assert!(!app.session.prefs().workspaces.contains_key("CS4"));

        let seen = std::sync::Arc::new(std::sync::Mutex::new(None));
        let slot = std::sync::Arc::clone(&seen);
        app.services.automation_write = Some(Box::new(move |path, bytes| {
            assert_eq!(path, "out.pcraft-workspace");
            *slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(bytes.to_vec());
            Ok(())
        }));
        crate::menus::invoke(&mut app, &ctx, "window.workspace.exportWorkspace", json!({"name": "Retouch Desk", "path": "out.pcraft-workspace"})).unwrap();
        let saved = seen.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone().expect("written");
        let parsed = parse(&saved).unwrap();
        assert_eq!(parsed.name, "Retouch Desk");
        assert_eq!(parsed.theme.as_deref(), Some("cs4"));
    }
}
