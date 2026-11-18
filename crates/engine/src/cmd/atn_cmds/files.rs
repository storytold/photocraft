use super::convert::{ReplacementSet, clip_name, export, import_first_with_keys};
use super::{Action, DEFAULT_SET, EngineError, Result, Session, Value, bad, json};
use crate::actions_cmds::ActionsFile;

mod replacement;
pub(super) use replacement::replace;
pub use replacement::validate as validate_replacement;

/// The name the file gave an action that `merge_file` renamed to stay unique, to the name it got.
fn renamed_to<'a>(renames: &'a [(String, String)], name: &str) -> Option<&'a String> {
    renames.iter().find(|(from, _)| from == name).map(|(_, to)| to)
}

/// `actions.load`: add the actions of an `.atn` file (`path` or `data`) to the list, retargeting
/// the set's own action references once and preserving foreign qualifications even when the
/// caller changes the imported set's name or action names collide. A set an automation client
/// loads is marked as automation's, so it keeps the gate when the user plays it.
pub(super) fn load(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "actions.load";
    if s.actions.recording.is_some() || s.actions.playing > 0 {
        return Err(bad(cmd, "stop recording or playing before loading actions"));
    }
    let encoding = match p.get("encoding") {
        None | Some(Value::Null) => None,
        Some(Value::String(e)) if e == "auto" => None,
        Some(Value::String(e)) if !e.is_empty() => Some(e.as_str()),
        Some(_) => return Err(bad(cmd, "encoding must be \"auto\" or a label such as shift_jis, gbk, big5, euc-kr, windows-1252, macintosh")),
    };
    let (bytes, _) = crate::preset_import_cmds::file_bytes(p, cmd, ".atn file")?;
    let ReplacementSet { imported: mut imp, keys: function_keys } = import_first_with_keys(&bytes, encoding).map_err(|e| bad(cmd, e))?;
    let original_set = imp.set.clone();
    match p.get("set") {
        None | Some(Value::Null) => {}
        Some(Value::String(n)) if !n.trim().is_empty() => {
            // Held to the importer's name limit: the set name is copied into every action.
            imp.set = clip_name(n.trim()).trim_end().to_string();
            for a in &mut imp.actions {
                a.set = Some(imp.set.clone());
            }
        }
        Some(_) => return Err(bad(cmd, "`set` must be a non-empty string")),
    }
    let replace = match p.get("replace") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(bad(cmd, "`replace` must be true or false")),
    };
    let steps = imp.actions.iter().fold(0usize, |total, a| total.saturating_add(a.steps.len()));
    // The file names its set even when it holds no action (Photoshop saves an empty set): the list
    // keeps it as an empty folder, like one New Set made, and `replace` drops the set's actions.
    if replace {
        // The function keys of the actions it replaces go with them.
        let replaced: Vec<String> = s.actions.list.actions().iter().filter(|a| a.set.as_deref() == Some(imp.set.as_str())).map(|a| a.name.clone()).collect();
        crate::actions_cmds::remove_shortcuts(s, replaced.iter().map(String::as_str));
    }
    let file = ActionsFile::new(imp.actions, vec![imp.set.clone()]);
    // Play, Conditional, Batch and Create Droplet steps call actions by name: follow the renames in
    // the actions just loaded, each target looked up once by the name the file gave it.
    let merged = s.actions.list.merge_file(file, replace, |step, renames| {
        let (id, params) = (step.command.as_str(), &mut step.params);
        // A qualified Batch or Create Droplet reference belongs to the file's original set, not
        // to a foreign set which happens to have the destination override's name.
        let by_set = matches!(id, "file.automate.batch" | "file.automate.createDroplet");
        if by_set && params.get("set").and_then(Value::as_str) == Some(original_set.as_str()) {
            params["set"] = json!(imp.set);
        } else if by_set && params.get("set").is_some_and(|set| !set.is_null()) {
            return;
        }
        let keys: &[&str] = match id {
            "actions.play" => &["action"],
            "actions.conditional" => &["then", "else"],
            "file.automate.batch" | "file.automate.createDroplet" => &["action"],
            _ => &[],
        };
        for key in keys {
            if let Some(to) = params.get(*key).and_then(Value::as_str).and_then(|n| renamed_to(renames, n)) {
                params[*key] = json!(to);
            }
        }
    });
    // Actions an automation client loads keep its gate when the user plays them.
    if s.authorize.is_some() {
        for &index in &merged.added {
            s.actions.list.set_automation(index, true).map_err(|e| crate::actions_cmds::list_error(cmd, e))?;
        }
    }
    let names: Vec<String> = merged.added.iter().map(|&index| s.actions.list.actions()[index].name.clone()).collect();
    // Keys go by the file's action order, so two actions the file gives one name keep their own.
    let keys: Vec<(String, String)> = function_keys.into_iter().filter_map(|(index, key)| Some((names.get(index)?.clone(), key))).collect();
    if !keys.is_empty() {
        s.edit_prefs(|prefs| {
            for (name, key) in &keys {
                // A function key belongs to one action.
                prefs.shortcuts.retain(|k, v| !(k.starts_with(crate::actions_cmds::PLAY_SHORTCUT_PREFIX) && v == key));
                prefs.shortcuts.insert(format!("{}{name}", crate::actions_cmds::PLAY_SHORTCUT_PREFIX), key.clone());
            }
        });
    }
    Ok(json!({
        "set": imp.set,
        "actions": names,
        "steps": steps,
        "unsupported": imp.unsupported,
        "functionKeys": keys.iter().map(|(n, k)| json!({"action": n, "key": k})).collect::<Vec<_>>(),
        "warnings": imp.warnings,
        "encoding": imp.encoding.as_deref().unwrap_or("UTF-8"),
    }))
}

pub(super) fn save(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "actions.save";
    crate::actions_cmds::flush_recording(s)?;
    let skip = match p.get("skipUnsupported") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(bad(cmd, "`skipUnsupported` must be true or false")),
    };
    let set_param = match p.get("set") {
        None | Some(Value::Null) => None,
        // A set is found by its exact name, as `actions.record` finds it: one loaded from a
        // Photoshop file or an older actions.json may have surrounding spaces. Others are trimmed.
        Some(Value::String(n)) if crate::actions_edit_cmds::set_names(s).contains(n) => Some(n.clone()),
        Some(Value::String(n)) if !n.trim().is_empty() => Some(n.trim().to_string()),
        Some(_) => return Err(bad(cmd, "`set` must be a non-empty string")),
    };
    let (set, actions): (String, Vec<&Action>) = if p.get("action").is_some() {
        let idx = crate::actions_cmds::resolve(s.actions.list.actions(), p, cmd)?;
        let a = &s.actions.list.actions()[idx];
        (set_param.or_else(|| a.set.clone()).unwrap_or_else(|| a.name.clone()), vec![a])
    } else if let Some(set) = set_param {
        let list: Vec<&Action> = s.actions.list.actions().iter().filter(|a| a.set.as_deref() == Some(set.as_str())).collect();
        // An empty set New Set made is written with no actions, as Photoshop saves one.
        if list.is_empty() && !s.actions.list.sets().contains(&set.as_str()) {
            return Err(bad(cmd, format!("no actions in a set named `{set}`")));
        }
        (set, list)
    } else {
        if s.actions.list.actions().is_empty() {
            return Err(bad(cmd, "there are no actions to save"));
        }
        (DEFAULT_SET.to_string(), s.actions.list.actions().iter().collect())
    };
    let names: Vec<String> = actions.iter().map(|a| a.name.clone()).collect();
    let keys: Vec<(String, String)> =
        s.prefs().shortcuts.iter().filter_map(|(k, v)| k.strip_prefix(crate::actions_cmds::PLAY_SHORTCUT_PREFIX).map(|n| (n.to_string(), v.clone()))).collect();
    let (bytes, skipped) = export(&set, &actions, &keys, skip).map_err(|e| bad(cmd, e))?;
    let steps: usize = actions.iter().map(|a| a.steps.len()).sum::<usize>().saturating_sub(skipped.len());
    let mut out = json!({"set": set, "actions": names, "steps": steps, "bytes": bytes.len(), "skipped": skipped});
    match p.get("path") {
        None | Some(Value::Null) => {
            out["data"] = json!(photocraft_paint::tile::b64_encode(&bytes));
        }
        Some(Value::String(path)) if !path.trim().is_empty() => {
            write_file(path, &bytes).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
            out["path"] = json!(path);
        }
        Some(_) => return Err(bad(cmd, "`path` must be a non-empty string")),
    }
    Ok(out)
}

#[cfg(not(target_arch = "wasm32"))]
fn write_file(path: &str, bytes: &[u8]) -> std::io::Result<()> {
    // Atomic replacement (temporary file + rename): a failed write keeps the previous ATN (#7290).
    photocraft_format::atomic_write(std::path::Path::new(path), bytes)
}

#[cfg(target_arch = "wasm32")]
fn write_file(path: &str, _bytes: &[u8]) -> std::io::Result<()> {
    Err(std::io::Error::other(format!("cannot write paths on the web ({path}); omit `path` to get the bytes back as `data`")))
}
