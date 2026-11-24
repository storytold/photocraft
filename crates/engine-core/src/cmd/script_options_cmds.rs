//! Custom options scripts keep between runs (`app.putCustomOptions` / `getCustomOptions` /
//! `eraseCustomOptions` in a script).
//!
//! A persistent value lives in the preferences and is there in later sessions; one that isn't
//! lasts until the app quits. Values are JSON objects (a script's descriptor, as the script
//! globals write it), bounded in size and count: on the web all preferences are one browser
//! storage item.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::presets::always;
use crate::{EngineError, Result, Session};

/// Most keys kept, persistent and not together.
pub const MAX_KEYS: usize = 256;
/// Longest key, in bytes.
pub const MAX_KEY_BYTES: usize = 1024;
/// Largest value, serialized.
pub const MAX_VALUE_BYTES: usize = 64 << 10;
/// Largest total of the persistent values, serialized.
pub const MAX_PERSISTENT_BYTES: usize = 256 << 10;

/// The persistent options, in the preferences. Those for this session only live in the
/// session (`Session::script_options`), so rebuilding the preferences can't drop them.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CustomOptions {
    pub persistent: BTreeMap<String, Value>,
}

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn key<'a>(cmd: &str, p: &'a Value) -> Result<&'a str> {
    match p.get("key").and_then(Value::as_str) {
        Some(k) if !k.is_empty() && k.len() <= MAX_KEY_BYTES => Ok(k),
        _ => Err(bad(cmd, format!("`key` must be text of 1 to {MAX_KEY_BYTES} bytes"))),
    }
}

fn size(v: &Value) -> usize {
    serde_json::to_vec(v).map_or(usize::MAX, |b| b.len())
}

/// `scripts.customOptions.get`: the value kept under `key`, or null.
fn get(s: &mut Session, p: &Value) -> Result<Value> {
    let k = key("scripts.customOptions.get", p)?;
    Ok(s.script_options.get(k).or_else(|| s.prefs().scripts.custom_options.persistent.get(k)).cloned().unwrap_or(Value::Null))
}

/// `scripts.customOptions.put`: keep `value` under `key`, replacing what was there.
fn put(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "scripts.customOptions.put";
    let k = key(CMD, p)?.to_owned();
    let value = p.get("value").filter(|v| v.is_object()).cloned().ok_or_else(|| bad(CMD, "`value` must be an object"))?;
    let persistent = p.get("persistent").and_then(Value::as_bool).unwrap_or(true);
    let bytes = size(&value);
    if bytes > MAX_VALUE_BYTES {
        return Err(bad(CMD, format!("a value can be at most {} KiB", MAX_VALUE_BYTES >> 10)));
    }
    let o = &s.prefs().scripts.custom_options;
    let known = o.persistent.contains_key(&k) || s.script_options.contains_key(&k);
    if !known && o.persistent.len() + s.script_options.len() >= MAX_KEYS {
        return Err(bad(CMD, format!("at most {MAX_KEYS} custom options can be kept; erase some first")));
    }
    if persistent {
        let others: usize = o.persistent.iter().filter(|(key, _)| **key != k).map(|(_, v)| size(v)).sum();
        if others + bytes > MAX_PERSISTENT_BYTES {
            return Err(bad(CMD, format!("the kept custom options can take at most {} KiB in all", MAX_PERSISTENT_BYTES >> 10)));
        }
    }
    s.script_options.remove(&k);
    if persistent {
        s.edit_prefs(|pr| {
            pr.scripts.custom_options.persistent.insert(k, value);
        });
    } else {
        if s.prefs().scripts.custom_options.persistent.contains_key(&k) {
            s.edit_prefs(|pr| {
                pr.scripts.custom_options.persistent.remove(&k);
            });
        }
        s.script_options.insert(k, value);
    }
    Ok(Value::Null)
}

/// `scripts.customOptions.erase`: forget `key`.
fn erase(s: &mut Session, p: &Value) -> Result<Value> {
    let k = key("scripts.customOptions.erase", p)?.to_owned();
    let kept = s.prefs().scripts.custom_options.persistent.contains_key(&k);
    if kept {
        s.edit_prefs(|pr| {
            pr.scripts.custom_options.persistent.remove(&k);
        });
    }
    let existed = s.script_options.remove(&k).is_some() || kept;
    Ok(json!({ "erased": existed }))
}

pub fn specs() -> Vec<CommandSpec> {
    let spec = |id, label, params, run| CommandSpec { id, label, menu: &[], shortcut: None, params, enabled: always, journal: false, run };
    vec![
        spec("scripts.customOptions.get", "Get Script Options", r#"{"key": str} → the object kept under key, or null"#, get),
        spec(
            "scripts.customOptions.put",
            "Keep Script Options",
            r#"{"key": str, "value": object (at most 64 KiB), "persistent": bool=true (false: until the app quits)} → null (at most 256 keys; 256 KiB of persistent values)"#,
            put,
        ),
        spec("scripts.customOptions.erase", "Erase Script Options", r#"{"key": str} → {erased: bool}"#, erase),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prefs::SavedPrefsExt;

    #[test]
    fn options_are_kept_replaced_and_erased() {
        let mut s = crate::session();
        assert_eq!(s.execute("scripts.customOptions.get", json!({"key": "k"})).unwrap(), Value::Null);
        s.execute("scripts.customOptions.put", json!({"key": "k", "value": {"a": 1}, "persistent": true})).unwrap();
        assert_eq!(s.execute("scripts.customOptions.get", json!({"key": "k"})).unwrap(), json!({"a": 1}));
        s.execute("scripts.customOptions.put", json!({"key": "k", "value": {"a": 2}, "persistent": false})).unwrap();
        assert_eq!(s.execute("scripts.customOptions.get", json!({"key": "k"})).unwrap(), json!({"a": 2}));
        assert!(s.prefs().scripts.custom_options.persistent.is_empty(), "moved to the session");
        assert_eq!(s.execute("scripts.customOptions.erase", json!({"key": "k"})).unwrap(), json!({"erased": true}));
        assert_eq!(s.execute("scripts.customOptions.get", json!({"key": "k"})).unwrap(), Value::Null);
    }

    #[test]
    fn session_options_outlive_a_change_to_the_preferences() {
        let mut s = crate::session();
        s.execute("scripts.customOptions.put", json!({"key": "k", "value": {"a": 1}, "persistent": false})).unwrap();
        s.execute("prefs.set", json!({"path": "unitsAndRulers.rulers", "value": "cm"})).unwrap();
        let json = s.prefs_to_json();
        s.load_prefs_json(&json).unwrap();
        assert_eq!(s.execute("scripts.customOptions.get", json!({"key": "k"})).unwrap(), json!({"a": 1}));
    }

    #[test]
    fn only_persistent_options_are_saved() {
        let mut s = crate::session();
        s.execute("scripts.customOptions.put", json!({"key": "kept", "value": {"a": 1}, "persistent": true})).unwrap();
        s.execute("scripts.customOptions.put", json!({"key": "gone", "value": {"b": 1}, "persistent": false})).unwrap();
        let saved = serde_json::to_value(&s.prefs().scripts).unwrap();
        assert_eq!(saved["customOptions"], json!({"persistent": {"kept": {"a": 1}}}));
    }

    #[test]
    fn bad_and_oversized_options_are_errors() {
        let mut s = crate::session();
        for p in [
            json!({}),
            json!({"key": ""}),
            json!({"key": "k".repeat(MAX_KEY_BYTES + 1), "value": {}}),
            json!({"key": "k", "value": [1]}),
            json!({"key": "k", "value": "text"}),
            json!({"key": "k", "value": {"a": "x".repeat(MAX_VALUE_BYTES)}}),
        ] {
            assert!(s.execute("scripts.customOptions.put", p.clone()).is_err(), "{p}");
        }
        let big = json!({"a": "x".repeat(MAX_VALUE_BYTES - 100)});
        for i in 0..MAX_PERSISTENT_BYTES / MAX_VALUE_BYTES {
            s.execute("scripts.customOptions.put", json!({"key": format!("k{i}"), "value": big})).unwrap();
        }
        assert!(s.execute("scripts.customOptions.put", json!({"key": "more", "value": big})).is_err(), "the persistent total is bounded");
        s.execute("scripts.customOptions.put", json!({"key": "more", "value": big, "persistent": false})).unwrap();
        for i in 0..MAX_KEYS {
            let _ = s.execute("scripts.customOptions.put", json!({"key": format!("n{i}"), "value": {}, "persistent": false}));
        }
        assert_eq!(s.prefs().scripts.custom_options.persistent.len() + s.script_options.len(), MAX_KEYS, "the key count is bounded");
    }
}
