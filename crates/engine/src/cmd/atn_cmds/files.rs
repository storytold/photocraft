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
        // So do the renames the set's earlier load recorded (#11523).
        s.actions.load_renames.retain(|(set, _), _| set != &imp.set);
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
        if id == "actions.play" {
            // A qualifier the importer kept for a child its own set lacks names this import's
            // set, which a `set` override renamed.
            if params.get("set").and_then(Value::as_str) == Some(original_set.as_str()) {
                params["set"] = json!(imp.set);
            }
            bind_play_target(params, renames);
            return;
        }
        let keys: &[&str] = match id {
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
    // This load is what renamed these children, so it is the one place that knows: record each so a
    // later load can bind a call to the name the child carries now (#11523), and follow it for the
    // calls already in the library that named a child of this set by the name the file gave it.
    let mut renamed = std::collections::HashMap::new();
    for (from, to) in &merged.renames {
        renamed.entry(from.clone()).or_insert_with(|| to.clone());
    }
    for (from, to) in &renamed {
        s.actions.load_renames.insert((imp.set.clone(), from.clone()), to.clone());
    }
    retarget_renamed_calls(&mut s.actions.list, &merged.added, &imp.set, &renamed);
    let lost_targets = bind_cross_set_calls(&mut s.actions, &merged.added, &imp.warnings);
    imp.warnings.extend(lost_targets);
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

/// The Play calls qualified with another action set that the load just added (the steps of the
/// actions at `added`), bound to the child they name, and a warning for each one the session cannot
/// answer, so it is heard of when the file loads rather than only when playback refuses to guess
/// (#11523). The two reasons read differently: a set the session has not loaded asks for that set,
/// while a loaded set without that child asks about the call. A `HashSet` beside the ordered list
/// keeps the deduplication linear when a big library repeats one lost target; `known` are the
/// warnings the import already has.
fn bind_cross_set_calls(actions: &mut crate::actions_cmds::ActionState, added: &[usize], known: &[String]) -> Vec<String> {
    let mut calls = Vec::new();
    for &at in added {
        let Some(a) = actions.list.action(at) else { continue };
        for (step_at, step) in a.steps.iter().enumerate() {
            let (Some(child), Some(set)) = (step.params.get("action").and_then(Value::as_str), step.params.get("set").and_then(Value::as_str)) else {
                continue;
            };
            if step.command == "actions.play" && !child.is_empty() && !set.is_empty() {
                calls.push((at, step_at, a.name.clone(), child.to_owned(), set.to_owned()));
            }
        }
    }
    let mut seen: std::collections::HashSet<String> = known.iter().cloned().collect();
    let mut lost_targets = Vec::new();
    for (at, step_at, caller, child, set) in calls {
        if crate::actions_cmds::qualified_target(actions.list.actions(), &child, &set).is_some() {
            continue;
        }
        // A child an earlier load renamed is named by what that load recorded, never by a guess
        // from the spelling of a name.
        if let Some(loaded) = actions.load_renames.get(&(set.clone(), child.clone())).cloned()
            && crate::actions_cmds::qualified_target(actions.list.actions(), &loaded, &set).is_some()
        {
            set_play_target(&mut actions.list, at, step_at, &loaded);
            continue;
        }
        let holds_set = actions.list.actions().iter().any(|a| a.set.as_deref() == Some(set.as_str())) || actions.list.sets().contains(&set.as_str());
        let warning = if holds_set {
            format!("`{caller}` plays `{child}`, which set `{set}` does not hold; playback will report that action rather than guess another set's")
        } else {
            format!("`{caller}` plays `{child}` in set `{set}`, which is not loaded; load that set before playing it")
        };
        if seen.insert(warning.clone()) {
            lost_targets.push(warning);
        }
    }
    lost_targets
}

/// Calls already in the library (every action but the ones at `added`) that name a child of `set`
/// by the name its file gave it, which this load renamed (`renamed`: file name to loaded name),
/// follow the rename. A child of that name the set still holds keeps its calls (#11523).
fn retarget_renamed_calls(list: &mut crate::actions_cmds::ActionList, added: &[usize], set: &str, renamed: &std::collections::HashMap<String, String>) {
    if renamed.is_empty() {
        return;
    }
    let holds = |list: &crate::actions_cmds::ActionList, name: &str| list.actions().iter().any(|a| a.set.as_deref() == Some(set) && a.name == name);
    let mut retargeted = Vec::new();
    for (at, a) in list.actions().iter().enumerate() {
        if added.contains(&at) {
            continue;
        }
        for (step_at, step) in a.steps.iter().enumerate() {
            if step.command != "actions.play" || step.params.get("set").and_then(Value::as_str) != Some(set) {
                continue;
            }
            let Some(to) = step.params.get("action").and_then(Value::as_str).filter(|name| !holds(list, name)).and_then(|name| renamed.get(name)) else {
                continue;
            };
            retargeted.push((at, step_at, to.clone()));
        }
    }
    for (at, step_at, to) in retargeted {
        set_play_target(list, at, step_at, &to);
    }
}

/// Point the Play step `step_at` of action `at` at the action named `to`.
fn set_play_target(list: &mut crate::actions_cmds::ActionList, at: usize, step_at: usize, to: &str) {
    let Some(mut step) = list.action(at).and_then(|a| a.steps.get(step_at)).cloned() else { return };
    step.params["action"] = json!(to);
    // The indices come from the list just read, so the step is there.
    let _ = list.replace_step(at, step_at, step);
}

/// An imported Play step that names an action set other than the file's own keeps its target.
///
/// Photoshop records a Play Action target as a child name *and* the action set holding it, and
/// `convert_action` already dropped a qualifier naming the file's own set, so what is left points
/// across sets and must survive the load: `actions.play` resolves it against that set and its
/// child, so the action of that name in another set can no longer be played instead (#11523).
/// [`bind_cross_set_calls`] checks it once the file is in. A call by name alone follows this load's
/// renames of the child, as it always did (#6806).
fn bind_play_target(params: &mut Value, renames: &[(String, String)]) {
    let Some(object) = params.as_object_mut() else { return };
    match object.get("set") {
        Some(Value::String(set)) if !set.is_empty() => return,
        Some(Value::Null) | Some(Value::String(_)) | None => {}
        Some(_) => {
            // A qualifier that is not a set name carries no target: keep the plain call.
            object.remove("set");
        }
    }
    if let Some(to) = object.get("action").and_then(Value::as_str).and_then(|name| renamed_to(renames, name)).cloned() {
        object.insert("action".into(), Value::String(to));
    }
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
