use super::convert::{export_groups, import_with_encoding};
use super::sidecar;
use super::{Action, DEFAULT_SET, EngineError, Result, Session, UniqueNames, Value, bad, json};

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
    match p.get("set") {
        None | Some(Value::Null) => {}
        Some(Value::String(n)) if !n.trim().is_empty() => {
            if imp.sets.len() != 1 {
                return Err(bad(cmd, "set override requires exactly one real set"));
            }
            imp.set = n.trim().to_string();
            if let Some(set) = imp.sets.first_mut() {
                set.name = imp.set.clone();
            }
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
    let targets: std::collections::HashSet<String> = imp.sets.iter().map(|set| set.name.clone()).collect();
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
    let old_groups = sidecar::groups(&s.actions, None).map_err(|error| bad(cmd, error))?;
    let retained_groups: Vec<_> = old_groups.iter().filter(|(name, _)| !replace || !targets.contains(*name)).collect();
    let sidecar_needed = imp.sets.iter().any(|set| set.metadata.is_some())
        || retained_groups.iter().any(|(_, selection)| {
            selection.source.is_some()
                || selection.actions.iter().any(|action| action.atn_metadata.is_some() || !action.description.is_empty() || !action.icon.is_empty())
        });
    let overhead = usize::from(sidecar_needed);
    if retained_groups.len().checked_add(imp.sets.len()).and_then(|count| count.checked_add(overhead)).is_none_or(|count| count > super::atn::MAX_SETS) {
        return Err(bad(cmd, "combined action library exceeds the set limit"));
    }
    if retained.checked_add(imp.actions.len()).and_then(|count| count.checked_add(overhead)).is_none_or(|count| count > super::atn::MAX_TOTAL_ACTIONS)
        || total_steps.checked_add(overhead).is_none_or(|count| count > super::atn::MAX_TOTAL_ITEMS)
    {
        return Err(bad(cmd, "combined action library leaves no space for its metadata sidecar"));
    }
    drop(retained_groups);
    drop(old_groups);
    let previous_rows: std::collections::HashSet<String> = s.actions.sets.iter().filter(|name| targets.contains(*name)).cloned().collect();
    let old_rows = s.actions.sets.iter().filter(|name| !replace || !targets.contains(*name)).count();
    let new_rows = if provenance.is_some() || imp.sets.iter().any(|set| set.metadata.is_some()) {
        imp.sets.len()
    } else {
        imp.sets
            .iter()
            .filter(|set| set.actions.is_empty() || previous_rows.contains(&set.name))
            .map(|set| set.name.as_str())
            .collect::<std::collections::HashSet<_>>()
            .len()
    };
    if old_rows.checked_add(new_rows).is_none_or(|count| count > super::atn::MAX_SETS) {
        return Err(bad(cmd, "too many action set rows"));
    }
    let mut set_for = vec![0usize; imp.actions.len()];
    for (index, set) in imp.sets.iter().enumerate() {
        set_for.get_mut(set.actions.clone()).ok_or_else(|| bad(cmd, "invalid imported set range"))?.fill(index);
    }
    if imp.action_keys.len() != imp.actions.len() {
        return Err(bad(cmd, "invalid imported shortcut positions"));
    }
    let mut taken = UniqueNames::new(s.actions.list.iter().filter(keep).map(|action| action.name.clone()));
    let mut names = Vec::with_capacity(imp.actions.len());
    let mut renamed: Vec<std::collections::HashMap<String, String>> = vec![Default::default(); imp.sets.len()];
    let mut global: std::collections::HashMap<String, Option<String>> = std::collections::HashMap::new();
    let mut steps = 0usize;
    let mut added = Vec::with_capacity(imp.actions.len());
    let mut keys = Vec::new();
    for (index, mut a) in imp.actions.into_iter().enumerate() {
        let name = taken.claim(&a.name);
        let group = set_for.get(index).and_then(|set| renamed.get_mut(*set)).ok_or_else(|| bad(cmd, "invalid imported action position"))?;
        group.entry(a.name.clone()).or_insert_with(|| name.clone());
        global
            .entry(a.name.clone())
            .and_modify(|entry| {
                if entry.as_ref() != Some(&name) {
                    *entry = None;
                }
            })
            .or_insert(Some(name.clone()));
        if let Some(key) = imp.action_keys.get(index).and_then(Option::as_ref) {
            keys.push((name.clone(), key.clone()));
        }
        a.name = name.clone();
        steps = steps.saturating_add(a.steps.len());
        names.push(name);
        added.push(a);
    }
    // Resolve imported calls within their physical set first; never use another set's first
    // duplicate display name for metadata or shortcuts. Ambiguous cross-set calls fail before commit.
    for (index, a) in added.iter_mut().enumerate() {
        let local = set_for.get(index).and_then(|set| renamed.get(*set)).ok_or_else(|| bad(cmd, "invalid imported action position"))?;
        for (id, params) in &mut a.steps {
            let keys: &[&str] = match id.as_str() {
                "actions.play" => &["action"],
                "actions.conditional" => &["then", "else"],
                _ => &[],
            };
            for key in keys {
                if let Some(target) = params.get(*key).and_then(Value::as_str) {
                    let to = if let Some(to) = local.get(target) {
                        Some(to)
                    } else {
                        match global.get(target) {
                            Some(Some(to)) => Some(to),
                            Some(None) => return Err(bad(cmd, "ambiguous cross-set action call; source set is required")),
                            None => None,
                        }
                    };
                    if let Some(to) = to {
                        params[*key] = json!(to);
                    }
                }
            }
        }
    }
    let set_names: Vec<String> = imp.sets.iter().map(|set| set.name.clone()).collect();
    if replace {
        s.actions.list.retain(|action| !targets.contains(super::logical_set_name(action.set.as_deref())));
        sidecar::retire_sets(&mut s.actions, &targets);
    }
    s.actions.list.extend(added);
    for (index, set) in imp.sets.into_iter().enumerate() {
        if provenance.is_some() || set.metadata.is_some() {
            let slot = s.actions.sets.len();
            sidecar::install_row(&mut s.actions, &set.name, set.metadata);
            if let Some(raw) = &provenance {
                crate::actions_cmds::library::import_row(&mut s.actions, raw, index, slot);
            }
        } else if (set.actions.is_empty() || previous_rows.contains(&set.name)) && !s.actions.sets.contains(&set.name) {
            sidecar::install_row(&mut s.actions, &set.name, None);
        }
    }
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
    // Preserve the old default single-set output name; a multi-set Save keeps each real group.
    if p.get("action").is_none() && set_param.is_none() && groups.len() == 1 {
        if let Some((name, _)) = groups.first_mut() {
            if name != super::UNGROUPED_SET {
                *name = DEFAULT_SET.to_string();
            }
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
        .map(|(name, selection)| {
            (name.as_str(), sidecar::Selection { actions: selection.actions.clone(), source: selection.source.clone(), raw: selection.raw.clone() })
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
