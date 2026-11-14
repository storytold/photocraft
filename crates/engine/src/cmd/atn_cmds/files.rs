use super::convert::{UniqueNames, clip_name, export, import_with_encoding};
use super::{Action, DEFAULT_SET, EngineError, Result, Session, Value, bad, json};

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
    let mut imp = import_with_encoding(&bytes, encoding).map_err(|e| bad(cmd, e))?;
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
    if replace {
        s.actions.list.retain(|a| a.set.as_deref() != Some(imp.set.as_str()));
    }
    let mut taken = UniqueNames::new(s.actions.list.iter().map(|a| a.name.clone()));
    let mut names = Vec::with_capacity(imp.actions.len());
    // The file's name of each action that had to be renamed, and its new name.
    let mut renamed: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut steps = 0usize;
    let mut added = Vec::with_capacity(imp.actions.len());
    for mut a in imp.actions {
        let name = taken.claim(&a.name);
        if name != a.name {
            renamed.entry(a.name.clone()).or_insert_with(|| name.clone());
        }
        a.name = name.clone();
        steps = steps.saturating_add(a.steps.len());
        names.push(name);
        added.push(a);
    }
    // Play calls qualified with another action set, as `(calling action, child, set)`, checked
    // once the file is in.
    let mut cross_set_calls = Vec::new();
    // Play, Conditional, Batch and Create Droplet steps call actions by name: follow the renames in
    // the actions just loaded, each target looked up once by the name the file gave it.
    for a in &mut added {
        let caller = a.name.clone();
        for (id, params) in &mut a.steps {
            // A qualified Batch or Create Droplet reference belongs to the file's original set, not
            // to a foreign set which happens to have the destination override's name.
            let by_set = matches!(id.as_str(), "file.automate.batch" | "file.automate.createDroplet");
            if by_set && params.get("set").and_then(Value::as_str) == Some(original_set.as_str()) {
                params["set"] = json!(imp.set);
            } else if by_set && params.get("set").is_some_and(|set| !set.is_null()) {
                continue;
            }
            if id == "actions.play" {
                // A qualifier the importer kept for a child its own set lacks names this import's
                // set, which a `set` override renamed.
                if params.get("set").and_then(Value::as_str) == Some(original_set.as_str()) {
                    params["set"] = json!(imp.set);
                }
                if let Some((child, set)) = bind_play_target(params, &renamed) {
                    cross_set_calls.push((caller.clone(), child, set));
                }
                continue;
            }
            let keys: &[&str] = match id.as_str() {
                "actions.conditional" => &["then", "else"],
                "file.automate.batch" | "file.automate.createDroplet" => &["action"],
                _ => &[],
            };
            for key in keys {
                if let Some(to) = params.get(*key).and_then(Value::as_str).and_then(|n| renamed.get(n)) {
                    params[*key] = json!(to);
                }
            }
        }
    }
    // Actions an automation client loads keep its gate when the user plays them.
    let automation = s.authorize.is_some();
    for a in &mut added {
        a.automation = automation;
    }
    s.actions.list.extend(added);
    // A cross-set call the session cannot answer is reported here, so it is heard of when the file
    // loads rather than only when playback refuses to guess (#11523). The two reasons read
    // differently: a set the session has not loaded asks for that set, while a loaded set without
    // that child (or holding two of its renamed forms) asks about the call. A `HashSet` beside the
    // ordered list keeps the deduplication linear when a big library repeats one lost target.
    let mut seen: std::collections::HashSet<String> = imp.warnings.iter().cloned().collect();
    let mut lost_targets = Vec::new();
    let holds_set = |wanted: &str| s.actions.list.iter().any(|a| a.set.as_deref() == Some(wanted)) || s.actions.sets.iter().any(|set| set == wanted);
    for (caller, child, set) in cross_set_calls {
        if crate::actions_cmds::qualified_target(&s.actions.list, &child, &set).is_some() {
            continue;
        }
        let warning = if holds_set(&set) {
            format!("`{caller}` plays `{child}`, which set `{set}` does not hold; playback will report that action rather than guess another set's")
        } else {
            format!("`{caller}` plays `{child}` in set `{set}`, which is not loaded; load that set before playing it")
        };
        if seen.insert(warning.clone()) {
            lost_targets.push(warning);
        }
    }
    imp.warnings.extend(lost_targets);
    // A set saved with no actions (Photoshop saves an empty set) is an empty folder, like one New
    // Set made.
    if !s.actions.list.iter().any(|a| a.set.as_deref() == Some(imp.set.as_str())) && !s.actions.sets.contains(&imp.set) {
        s.actions.sets.push(imp.set.clone());
    }
    let keys: Vec<(String, String)> = imp.function_keys.into_iter().map(|(n, k)| (renamed.get(&n).cloned().unwrap_or(n), k)).collect();
    if !keys.is_empty() {
        s.edit_prefs(|prefs| {
            for (name, key) in &keys {
                // A function key belongs to one action.
                prefs.shortcuts.retain(|k, v| !(k.starts_with(crate::actions_cmds::PLAY_SHORTCUT_PREFIX) && v == key));
                prefs.shortcuts.insert(format!("{}{name}", crate::actions_cmds::PLAY_SHORTCUT_PREFIX), key.clone());
            }
        });
    }
    s.actions.touch();
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

/// An imported Play step that names an action set other than the file's own, as `(child, set)`.
///
/// Photoshop records a Play Action target as a child name *and* the action set holding it, and
/// `convert_action` already dropped a qualifier naming the file's own set — so what is left points
/// across sets and must survive the load: `actions.play` resolves it against that set and its
/// child, so a same-named child of a third set can no longer be played instead (#11523). A call by
/// name alone follows this load's renames of the child, as it always did (#6806).
/// `Some((child, set))` asks the caller to check that the session holds that child of that set.
fn bind_play_target(params: &mut Value, renamed: &std::collections::HashMap<String, String>) -> Option<(String, String)> {
    let object = params.as_object_mut()?;
    let set = match object.get("set") {
        Some(Value::String(set)) if !set.is_empty() => Some(set.clone()),
        Some(Value::Null) | Some(Value::String(_)) | None => None,
        Some(_) => {
            // A qualifier that is not a set name carries no target: keep the plain call.
            object.remove("set");
            None
        }
    };
    let recorded = object.get("action").and_then(Value::as_str).unwrap_or_default().to_string();
    if let Some(set) = set {
        return (!recorded.is_empty()).then_some((recorded, set));
    }
    if let Some(to) = renamed.get(&recorded) {
        object.insert("action".into(), Value::String(to.clone()));
    }
    None
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
        let idx = crate::actions_cmds::resolve(&s.actions.list, p, cmd)?;
        let a = &s.actions.list[idx];
        (set_param.or_else(|| a.set.clone()).unwrap_or_else(|| a.name.clone()), vec![a])
    } else if let Some(set) = set_param {
        let list: Vec<&Action> = s.actions.list.iter().filter(|a| a.set.as_deref() == Some(set.as_str())).collect();
        // An empty set New Set made is written with no actions, as Photoshop saves one.
        if list.is_empty() && !s.actions.sets.contains(&set) {
            return Err(bad(cmd, format!("no actions in a set named `{set}`")));
        }
        (set, list)
    } else {
        if s.actions.list.is_empty() {
            return Err(bad(cmd, "there are no actions to save"));
        }
        (DEFAULT_SET.to_string(), s.actions.list.iter().collect())
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
