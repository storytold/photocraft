use super::convert::{export_groups, import_with_encoding};
use super::sidecar;
use super::{Action, DEFAULT_SET, EngineError, Result, Session, Value, bad, json};

/// A set rename moves its own references; foreign scopes retain their meaning.
fn retarget_scopes(action: &mut Action, from: &str, to: &str) {
    crate::actions_cmds::visit_calls(&mut action.steps, &mut |id, params, _| {
        let keys: &[&str] = match id {
            "actions.play" | "file.automate.batch" | "file.automate.createDroplet" => &["set", "actionSet"],
            "actions.conditional" => &["set", "thenSet", "elseSet"],
            _ => &[],
        };
        for key in keys {
            if params.get(*key).and_then(Value::as_str) == Some(from) {
                params[*key] = json!(to);
            }
        }
    });
}

/// Retained callers keep their old targets when replacement removes preceding rows.
fn retain_calls(list: &mut [Action], original: &[Action], targets: &std::collections::HashSet<String>) {
    let mut next = 0usize;
    let indices: Vec<_> = original
        .iter()
        .map(|action| {
            if targets.contains(super::logical_set_name(action.set.as_deref())) {
                None
            } else {
                let index = next;
                next += 1;
                Some(index)
            }
        })
        .collect();
    for action in list {
        crate::actions_cmds::visit_calls(&mut action.steps, &mut |id, params, _| {
            if !matches!(id, "actions.play" | "file.automate.batch" | "file.automate.createDroplet") || !params.get("action").is_some_and(Value::is_number) {
                return;
            }
            if let Ok(index) = crate::actions_cmds::resolve(original, params, "actions.load") {
                match indices.get(index).copied().flatten() {
                    Some(next) if next != index => params["action"] = json!(next),
                    None => params["action"] = Value::Null,
                    _ => {}
                }
            }
        });
    }
}

/// Validate and normalize an external archive before committing actions, physical rows and shortcuts.
pub(super) fn load(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "actions.load";
    if s.actions.recording_active() || s.actions.playing > 0 {
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
    // Legacy imports still use the existing conversion route. Version16 additionally keeps
    // raw occurrences; normalization of the external import remains a deliberate edit.
    let provenance = crate::actions_cmds::library::decode(&bytes).ok();
    if let Some(raw) = &provenance {
        for (action, original) in imp.actions.iter_mut().zip(&raw.list) {
            crate::actions_cmds::library::bind_import(action, original);
        }
        for (index, set) in imp.sets.iter_mut().enumerate() {
            set.metadata = sidecar::empty_source(raw, index);
        }
    }
    let set_override = match p.get("set") {
        None | Some(Value::Null) => None,
        Some(Value::String(n)) if !n.trim().is_empty() => {
            if imp.sets.len() != 1 {
                return Err(bad(cmd, "set override requires exactly one real set"));
            }
            Some(super::convert::clip_name(n.trim()).trim_end().to_string())
        }
        Some(_) => return Err(bad(cmd, "`set` must be a non-empty string")),
    };
    for action in &mut imp.actions {
        action.automation |= s.authorize.is_some();
    }
    let replace = match p.get("replace") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(bad(cmd, "`replace` must be true or false")),
    };
    let targets: std::collections::HashSet<String> = imp.sets.iter().map(|set| set_override.as_ref().unwrap_or(&set.name).clone()).collect();
    let keep = |action: &&Action| !replace || !targets.contains(super::logical_set_name(action.set.as_deref()));
    let retained = s.actions.list.iter().filter(keep).count();
    if retained.checked_add(imp.actions.len()).is_none_or(|count| count > super::atn::MAX_TOTAL_ACTIONS) {
        return Err(bad(cmd, "combined action library exceeds the action limit"));
    }
    let mut total_steps = 0usize;
    for action in s.actions.list.iter().filter(keep).chain(imp.actions.iter()) {
        total_steps = total_steps
            .checked_add(action.steps.len())
            .filter(|count| *count <= super::atn::MAX_TOTAL_ITEMS)
            .ok_or_else(|| bad(cmd, "combined action library exceeds the step limit"))?;
    }
    let previous_rows: std::collections::HashSet<String> = s.actions.sets.iter().filter(|name| targets.contains(*name)).cloned().collect();
    let old_rows = s.actions.sets.iter().filter(|name| !replace || !targets.contains(*name)).count();
    let new_rows = if provenance.is_some() || imp.sets.iter().any(|set| set.metadata.is_some()) {
        imp.sets.len()
    } else {
        imp.sets
            .iter()
            .filter(|set| set.actions.is_empty() || previous_rows.contains(set_override.as_ref().unwrap_or(&set.name)))
            .map(|set| set_override.as_ref().unwrap_or(&set.name).as_str())
            .collect::<std::collections::HashSet<_>>()
            .len()
    };
    if old_rows.checked_add(new_rows).is_none_or(|count| count > super::atn::MAX_SETS) {
        return Err(bad(cmd, "too many action set rows"));
    }
    let names =
        super::normalize_import(&mut imp, s.actions.list.iter().filter(keep).map(|action| action.name.clone()), true).map_err(|error| bad(cmd, error))?;
    // Resolve against the original archive before its display set is overridden, so a
    // pre-existing foreign destination scope cannot become an imported local target.
    if let Some(target) = set_override {
        let original_set = imp.set.clone();
        let original_wire = imp.sets.first().ok_or_else(|| bad(cmd, "missing imported set"))?.wire_name.clone();
        imp.set = target;
        if let Some(set) = imp.sets.first_mut() {
            set.name = imp.set.clone();
            set.wire_name = imp.set.clone();
        }
        for action in &mut imp.actions {
            action.set = Some(imp.set.clone());
            retarget_scopes(action, &original_set, &imp.set);
            if original_wire != original_set {
                retarget_scopes(action, &original_wire, &imp.set);
            }
        }
    }
    let keys = imp.function_keys.clone();
    let steps: usize = imp.actions.iter().map(|action| action.steps.len()).sum();
    let mut added = imp.actions;
    crate::actions_cmds::shift_action_calls(&mut added, 0, retained);
    let set_names: Vec<String> = imp.sets.iter().map(|set| set.name.clone()).collect();
    let mut proposed = crate::actions_cmds::library::definition_copy(&s.actions);
    if replace {
        proposed.list.retain(|action| !targets.contains(super::logical_set_name(action.set.as_deref())));
        retain_calls(&mut proposed.list, &s.actions.list, &targets);
        sidecar::retire_sets(&mut proposed, &targets);
    }
    proposed.list.extend(added);
    for (index, set) in imp.sets.into_iter().enumerate() {
        if provenance.is_some() || set.metadata.is_some() {
            let slot = proposed.sets.len();
            sidecar::install_row(&mut proposed, &set.name, set.metadata);
            if let Some(raw) = &provenance {
                crate::actions_cmds::library::import_row(&mut proposed, raw, index, slot);
            }
        } else if (set.actions.is_empty() || previous_rows.contains(&set.name)) && !proposed.sets.contains(&set.name) {
            sidecar::install_row(&mut proposed, &set.name, None);
        }
    }
    let mut shortcuts = s.prefs().shortcuts.clone();
    for (name, key) in &keys {
        shortcuts.retain(|k, v| !(k.starts_with(crate::actions_cmds::PLAY_SHORTCUT_PREFIX) && v == key));
        shortcuts.insert(format!("{}{name}", crate::actions_cmds::PLAY_SHORTCUT_PREFIX), key.clone());
    }
    // The exact durable representation owns group, sidecar and byte limits. Refuse before
    // publishing either the actions or preferences so a rejected import leaves both intact.
    crate::actions_cmds::library::check_definition(&proposed, &shortcuts).map_err(|error| bad(cmd, error))?;
    s.actions.list = proposed.list;
    s.actions.sets = proposed.sets;
    s.actions.library = proposed.library;
    s.actions.atn_set_rows = proposed.atn_set_rows;
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
        "sets": set_names,
        "actions": names,
        "steps": steps,
        "unsupported": imp.unsupported,
        "functionKeys": keys.iter().map(|(n, k)| json!({"action": n, "key": k})).collect::<Vec<_>>(),
        "warnings": imp.warnings,
        "encoding": imp.encoding.as_deref().unwrap_or("UTF-8"),
    }))
}

/// Encode the selected action or library while retaining its physical source and metadata.
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
        // Photoshop file or a legacy library may have surrounding spaces. Others are trimmed.
        Some(Value::String(n)) if crate::actions_edit_cmds::set_names(s).contains(n) => Some(n.clone()),
        Some(Value::String(n)) if !n.trim().is_empty() => Some(n.trim().to_string()),
        Some(_) => return Err(bad(cmd, "`set` must be a non-empty string")),
    };
    let mut groups = if p.get("action").is_some() {
        let idx = crate::actions_cmds::resolve(&s.actions.list, p, cmd)?;
        let a = &s.actions.list[idx];
        vec![(set_param.clone().unwrap_or_else(|| super::logical_set_name(a.set.as_deref()).to_string()), sidecar::selection(&s.actions, a))]
    } else if let Some(set) = &set_param {
        let groups = sidecar::groups(&s.actions, Some(set)).map_err(|error| bad(cmd, error))?;
        if groups.is_empty() {
            return Err(bad(cmd, format!("no actions in a set named `{set}`")));
        }
        groups.into_iter().map(|(name, selection)| (name.to_string(), selection)).collect()
    } else {
        if s.actions.list.is_empty() && s.actions.sets.is_empty() {
            return Err(bad(cmd, "there are no actions to save"));
        }
        sidecar::groups(&s.actions, None).map_err(|error| bad(cmd, error))?.into_iter().map(|(name, selection)| (name.to_string(), selection)).collect()
    };
    let mut output_actions = sidecar::portable_actions(&s.actions, &groups, skip).map_err(|error| bad(cmd, error))?;
    // Preserve the old default single-set output name and move its internal calls with it.
    if p.get("action").is_none()
        && set_param.is_none()
        && groups.len() == 1
        && let Some((name, _)) = groups.first_mut()
        && name != super::UNGROUPED_SET
    {
        let from = name.clone();
        *name = DEFAULT_SET.to_string();
        for action in output_actions.iter_mut().flatten() {
            retarget_scopes(action, &from, DEFAULT_SET);
        }
    }
    let names: Vec<String> = groups.iter().flat_map(|(_, selection)| &selection.actions).map(|action| action.name.clone()).collect();
    let set_names: Vec<String> = groups.iter().map(|(name, _)| name.clone()).collect();
    let set = set_param.or_else(|| set_names.first().cloned()).unwrap_or_else(|| DEFAULT_SET.to_string());
    let keys: Vec<(String, String)> =
        s.prefs().shortcuts.iter().filter_map(|(k, v)| k.strip_prefix(crate::actions_cmds::PLAY_SHORTCUT_PREFIX).map(|n| (n.to_string(), v.clone()))).collect();
    let steps: usize = groups.iter().flat_map(|(_, selection)| &selection.actions).map(|action| action.steps.len()).sum();
    let borrowed: Vec<_> = groups
        .iter()
        .zip(&output_actions)
        .map(|((name, selection), actions)| {
            let actions = actions.iter().collect();
            (name.as_str(), sidecar::Selection { actions, source: selection.source.clone(), raw: selection.raw.clone() })
        })
        .collect();
    let (bytes, skipped) = export_groups(&borrowed, &keys, skip).map_err(|error| bad(cmd, error))?;
    let steps = steps.saturating_sub(skipped.len());
    let mut out = json!({"set": set, "sets": set_names, "actions": names, "steps": steps, "bytes": bytes.len(), "skipped": skipped});
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
