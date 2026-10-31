//! Named plug-in parameter sets, persisted with the existing dialog preferences.

use serde_json::{Map, Value, json};

use crate::commands::CommandSpec;
use crate::presets::{always, bad, req_str};
use crate::{Result, Session};

/// Validates the plug-in id before deriving its isolated preference key; installation is not required.
fn key(cmd: &str, p: &Value) -> Result<String> {
    let id = req_str(p, "id", cmd)?;
    if !photocraft_plugins::manifest::valid_id(id) {
        return Err(bad(cmd, "invalid plug-in id"));
    }
    Ok(format!("plugin.presets.{id}"))
}

/// Borrows the trimmed preset name, rejecting empty, control-containing or overlong names.
fn name<'a>(cmd: &str, p: &'a Value) -> Result<&'a str> {
    let name = req_str(p, "name", cmd)?.trim();
    if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
        return Err(bad(cmd, "name must be 1–64 printable characters"));
    }
    Ok(name)
}

/// Returns this plug-in id's saved preset object, or an empty object when no usable entry exists.
fn list(s: &mut Session, p: &Value) -> Result<Value> {
    let key = key("plugin.preset.list", p)?;
    Ok(s.prefs().dialogs.get(&key).filter(|v| v.is_object()).cloned().unwrap_or(json!({})))
}

/// Stores manifest-resolved parameters, excluding Skin Smooth photo samples.
/// Existing names are replaced; a plug-in may keep at most 64 distinct presets.
fn save(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "plugin.preset.save";
    let key = key(CMD, p)?;
    let name = name(CMD, p)?;
    let plugin = photocraft_plugins::registry::get(req_str(p, "id", CMD)?).ok_or_else(|| bad(CMD, "plug-in is not installed"))?;
    let values = p.get("params").filter(|v| v.is_object()).ok_or_else(|| bad(CMD, "params must be an object"))?;
    let mut resolved = plugin.manifest().resolve_params(values).map_err(|e| bad(CMD, e.to_string()))?;
    // Photo-specific samples belong to the filter session, never a named preset.
    if plugin.id() == "local.skinsmooth.skin-smooth" {
        resolved.remove("positiveColors");
        resolved.remove("negativeColors");
    }
    let mut presets = s.prefs().dialogs.get(&key).and_then(Value::as_object).cloned().unwrap_or_default();
    if presets.len() >= 64 && !presets.contains_key(name) {
        return Err(bad(CMD, "at most 64 presets per plug-in"));
    }
    presets.insert(name.into(), Value::Object(resolved));
    s.edit_prefs(|prefs| {
        prefs.dialogs.insert(key, Value::Object(presets));
    });
    Ok(json!({"name": name}))
}

/// Resolves a saved preset against the currently installed manifest, rejecting stale invalid values.
fn load(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "plugin.preset.load";
    let key = key(CMD, p)?;
    let name = name(CMD, p)?;
    let values = s.prefs().dialogs.get(&key).and_then(|v| v.get(name)).ok_or_else(|| bad(CMD, "preset not found"))?;
    let plugin = photocraft_plugins::registry::get(req_str(p, "id", CMD)?).ok_or_else(|| bad(CMD, "plug-in is not installed"))?;
    // Revalidate against the currently installed version; schema changes cannot inject bad values.
    let resolved = plugin.manifest().resolve_params(values).map_err(|e| bad(CMD, e.to_string()))?;
    Ok(Value::Object(resolved))
}

/// Deletes an existing named preset without changing document pixels or history.
fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "plugin.preset.remove";
    let key = key(CMD, p)?;
    let name = name(CMD, p)?;
    let mut presets: Map<String, Value> = s.prefs().dialogs.get(&key).and_then(Value::as_object).cloned().unwrap_or_default();
    if presets.remove(name).is_none() {
        return Err(bad(CMD, "preset not found"));
    }
    s.edit_prefs(|prefs| {
        prefs.dialogs.insert(key, Value::Object(presets));
    });
    Ok(json!({"removed": name}))
}

/// Exposes preset persistence through engine commands shared by the UI and automation.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "plugin.preset.list",
            label: "List Plug-in Presets",
            menu: &[],
            shortcut: None,
            params: r#"{"id":text}"#,
            enabled: always,
            run: list,
            journal: false,
        },
        CommandSpec {
            id: "plugin.preset.save",
            label: "Save Plug-in Preset",
            menu: &[],
            shortcut: None,
            params: r#"{"id":text,"name":text,"params":json}"#,
            enabled: always,
            run: save,
            journal: false,
        },
        CommandSpec {
            id: "plugin.preset.load",
            label: "Load Plug-in Preset",
            menu: &[],
            shortcut: None,
            params: r#"{"id":text,"name":text}"#,
            enabled: always,
            run: load,
            journal: false,
        },
        CommandSpec {
            id: "plugin.preset.remove",
            label: "Delete Plug-in Preset",
            menu: &[],
            shortcut: None,
            params: r#"{"id":text,"name":text}"#,
            enabled: always,
            run: remove,
            journal: false,
        },
    ]
}
