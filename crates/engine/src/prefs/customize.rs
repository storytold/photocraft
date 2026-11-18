use std::collections::BTreeMap;

use craft::keymap::import::kys;
use craft::keymap::import::{PlannedBinding, SkipReason};
use craft::keymap::{ForeignSet, ImportOptions, NoneDefined, Platform, Scope};

use super::cmds::bad;
use super::files::{file_target, read_text_file, rename_path, writable, write_files_atomically};
use super::shortcuts::{BindKind, alternate_keys, bindable, bindable_items, conflicts, default_alternates, is_tool, normalize_shortcut, reserved_shortcut};
use super::{MenuCustomization, Preferences, Result, Session, Value, json};
use crate::keymap_defs;

/// The `shortcuts` / `alternateShortcuts` changes a `.kys` import makes to `prefs` (`null` restores
/// a command's defaults), and the reply's `import` report. A key the system reserves is left out
/// and reported as `reserved`, not a failed import.
fn plan_kys(prefs: &Preferences, foreign: &ForeignSet, platform: Platform) -> (serde_json::Map<String, Value>, serde_json::Map<String, Value>, Value) {
    let keymap = keymap_defs::keymap(prefs, platform);
    // A row without a key leaves its command alone, as imports always did here.
    let options = ImportOptions { none_defined: NoneDefined::Keep, ..ImportOptions::default() };
    let plan = craft::keymap::plan(foreign, &keymap, &options);
    let mut applied = keymap.clone();
    let report = plan.apply(&mut applied);
    let (set_before, alternates_before) = keymap_defs::overrides_to_prefs(keymap.overrides());
    let (set_after, alternates_after) = keymap_defs::overrides_to_prefs(applied.overrides());

    let is_tool_row = |item: &str| foreign.entries.iter().any(|e| e.is_tool && e.item() == item);
    let text = |b: &PlannedBinding| normalize_shortcut(&b.binding.text(&Scope::app()));
    let (mut set, mut tools, mut alternate_keys, mut alternates) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new(), 0usize);
    for planned in &plan.commands {
        let keys: Vec<String> = planned.bindings.iter().filter_map(&text).collect();
        let Some(main) = keys.first().cloned() else { continue };
        let id = planned.command.id.clone();
        if crate::shell_commands::tool_default(&id).is_some() {
            tools.insert(id, main);
        } else {
            alternates += keys.len() - 1;
            alternate_keys.insert(id.clone(), keys[1..].to_vec());
            set.insert(id, main);
        }
    }
    let names = |rows: Vec<&str>, tool: bool| rows.into_iter().filter(|item| is_tool_row(item) == tool).map(str::to_owned).collect::<Vec<_>>();
    let unmatched: Vec<&str> = report.unmatched.iter().map(|u| u.item.as_str()).collect();
    let unreadable: Vec<&str> = report.unreadable.iter().map(|u| u.item.as_str()).collect();
    let reserved: Vec<String> =
        report.skipped.iter().filter(|s| s.reason == SkipReason::Reserved).map(|s| format!("{}: {}", s.command.id, s.shortcut)).collect();
    let import = json!({
        "name": foreign.name,
        "imported": set.len(),
        "set": set,
        "unknown": names(unmatched.clone(), false),
        "unreadable": names(unreadable, false),
        "alternates": alternates,
        "alternateKeys": alternate_keys,
        "toolKeys": foreign.entries.iter().filter(|e| e.is_tool && !e.keys.is_empty()).count(),
        "tools": tools,
        "unknownTools": names(unmatched, true),
        "reserved": reserved,
    });
    (changes(&set_before, &set_after), changes(&alternates_before, &alternates_after), import)
}

/// The platform a call's keys are read and written for: its `platform` (`mac`, `windows` or
/// `linux`, which the UI sends from its runtime's platform, since the build target reads Linux in
/// every browser), else the build target's.
fn platform_of(p: &Value, cmd: &str) -> Result<Platform> {
    match p.get("platform").and_then(Value::as_str) {
        None => Ok(Platform::current()),
        Some("mac") => Ok(Platform::Mac),
        Some("windows") => Ok(Platform::Windows),
        Some("linux") => Ok(Platform::Linux),
        Some(other) => Err(bad(cmd, format!("unknown platform `{other}` (mac|windows|linux)"))),
    }
}

/// The entries of `after` that differ from `before`, and `null` for the ones `after` lacks.
fn changes<V: serde::Serialize + PartialEq>(before: &BTreeMap<String, V>, after: &BTreeMap<String, V>) -> serde_json::Map<String, Value> {
    let mut out = serde_json::Map::new();
    for (id, value) in after {
        if before.get(id) != Some(value) {
            out.insert(id.clone(), json!(value));
        }
    }
    for id in before.keys().filter(|id| !after.contains_key(*id)) {
        out.insert(id.clone(), Value::Null);
    }
    out
}

/// Edit › Keyboard Shortcuts (engine part): list, change and reset application shortcuts.
pub(super) fn keyboard_shortcuts(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "edit.keyboardShortcuts";
    let platform = platform_of(p, cmd)?;
    // Every change lands at once, after all of it validated: a bad key leaves the set as it was.
    let mut next = s.prefs().clone();
    // A set loads first, so the call's other changes apply on top of it.
    let mut changed = crate::shortcut_sets::load(&mut next, p).map_err(|e| bad(cmd, e))?;
    if let Some(r) = p.get("reset") {
        match r {
            Value::Bool(true) => {
                next.shortcuts.clear();
                next.alternate_shortcuts.clear();
                changed = true;
            }
            Value::Array(ids) => {
                for id in ids.iter().filter_map(Value::as_str) {
                    next.shortcuts.remove(id);
                    next.alternate_shortcuts.remove(id);
                }
                changed = true;
            }
            _ => {}
        }
    }
    // A Photoshop `.kys` set (`importKys`, its XML text) becomes `set` and `alternates` entries,
    // matched by craft-keymap's `plan` over `keymap_defs::keymap`; a key equal to the command's
    // default restores the default, and keys given in `set` / `alternates` as well win over the
    // file's.
    let imported;
    let mut import = None;
    let p = match p.get("importKys") {
        Some(v) => {
            // The file's text, or {"path": …} to read it.
            let read;
            let xml = match v {
                Value::String(s) => s.as_str(),
                Value::Object(o) => {
                    let path =
                        o.get("path").and_then(Value::as_str).filter(|s| !s.trim().is_empty()).ok_or_else(|| bad(cmd, "`importKys.path` is a .kys file"))?;
                    read = read_text_file(path, kys::MAX_BYTES).map_err(|e| bad(cmd, format!("{path}: {e}")))?;
                    read.as_str()
                }
                _ => return Err(bad(cmd, "`importKys` is the text of a .kys file, or {\"path\": …}")),
            };
            let foreign = kys::read(xml).map_err(|e| bad(cmd, e.to_string()))?;
            let (mut merged, mut alternates, report) = plan_kys(&next, &foreign, platform);
            merged.extend(p.get("set").and_then(Value::as_object).cloned().unwrap_or_default());
            alternates.extend(p.get("alternates").and_then(Value::as_object).cloned().unwrap_or_default());
            import = Some(report);
            let mut with_import = p.clone();
            with_import["set"] = Value::Object(merged);
            with_import["alternates"] = Value::Object(alternates);
            imported = with_import;
            &imported
        }
        None => p,
    };
    let empty = serde_json::Map::new();
    let set = match p.get("set") {
        Some(Value::Object(m)) => m,
        _ => &empty,
    };
    let alts = match p.get("alternates") {
        None | Some(Value::Null) => &empty,
        Some(v) => v.as_object().ok_or_else(|| bad(cmd, "`alternates` maps command ids to lists of shortcuts"))?,
    };
    changed |= apply_shortcut_changes(&mut next, p, set, alts, cmd)?;
    // Then the sets are saved, deleted or switched, from the bindings as they now stand.
    changed |= crate::shortcut_sets::store(&mut next, p).map_err(|e| bad(cmd, e))?;
    // The reply and the exports read the bindings as they will be; they are committed only once
    // the exports are written, so a bad export target changes nothing.
    let prefs = &next;
    let filter = p.get("filter").and_then(Value::as_str).map(str::to_ascii_lowercase);
    let list: Vec<Value> = bindable_items()
        .filter(|c| filter.as_ref().is_none_or(|f| c.0.to_ascii_lowercase().contains(f) || c.1.to_ascii_lowercase().contains(f)))
        .filter(|c| {
            p.get("list").and_then(Value::as_bool).unwrap_or(false)
                || filter.is_some()
                || prefs.shortcuts.contains_key(c.0)
                || prefs.alternate_shortcuts.contains_key(c.0)
        })
        .map(|(id, label, menu, def, kind)| {
            let main = prefs.shortcut(id, def);
            let defaults: Vec<&str> = default_alternates(id).collect();
            let mut v = json!({"id": id, "label": label, "menu": menu, "default": def, "shortcut": main, "alternates": prefs.alternates(id, main)});
            if !defaults.is_empty() {
                v["defaultAlternates"] = json!(defaults);
            }
            match kind {
                BindKind::Hold => v["hold"] = json!(true),
                BindKind::Tool => v["tool"] = json!(true),
                BindKind::Command => {}
            }
            v
        })
        .collect();
    let mut bindings: Vec<(&str, String)> = Vec::new();
    for (id, def) in bindable() {
        let main = prefs.shortcut(id, def);
        bindings.extend(main.map(|m| (id, m.to_string())));
        bindings.extend(prefs.alternates(id, main).into_iter().map(|k| (id, k)));
    }
    // Tools sharing a key are a cycling group, not a conflict.
    let conflicts: Vec<Value> = conflicts(bindings.iter().map(|(id, k)| (*id, k.as_str())))
        .into_iter()
        .filter(|(_, ids)| !ids.iter().all(|id| is_tool(id)))
        .map(|(sc, ids)| json!({"shortcut": sc, "commands": ids}))
        .collect();
    let mut out = json!({"overrides": prefs.shortcuts, "alternateOverrides": prefs.alternate_shortcuts, "commands": list, "conflicts": conflicts});
    if let (Some(o), Value::Object(sets)) = (out.as_object_mut(), crate::shortcut_sets::describe(prefs)) {
        o.extend(sets);
    }
    // The bindings as a `.kys` file, or Photoshop's Summarize page: every bindable item (the
    // engine's commands, the shell's commands and the tools).
    let set_name = if prefs.shortcut_set.is_empty() { crate::shortcut_sets::DEFAULTS_NAME } else { prefs.shortcut_set.as_str() };
    // Both targets are resolved and their texts made before anything is written, and a path that
    // can't be written is refused before either file is: a bad export changes nothing.
    let kys_target = file_target(cmd, p.get("exportKys"))?;
    let html_target = file_target(cmd, p.get("summarize"))?;
    let mut files: Vec<(&str, String, String)> = Vec::new();
    if let Some(target) = kys_target {
        let text = kys::write(set_name, &crate::keymap_defs::keymap(prefs, platform));
        match target {
            Some(path) => files.push(("kysPath", path, text)),
            None => out["kys"] = json!(text),
        }
    }
    if let Some(target) = html_target {
        let v = p.get("summarize").cloned().unwrap_or(Value::Null);
        let mac = v.get("mac").and_then(Value::as_bool).unwrap_or(cfg!(target_os = "macos"));
        let mut sections: Vec<crate::shortcut_sets::Section> = Vec::new();
        for crate::shortcut_sets::Bound { label, menu, keys, .. } in crate::shortcut_sets::bound(prefs) {
            match sections.iter_mut().find(|(m, _)| *m == menu) {
                Some((_, rows)) => rows.push((label, keys)),
                None => sections.push((menu, vec![(label, keys)])),
            }
        }
        let html = crate::shortcut_sets::summarize(set_name, &sections, mac);
        match target {
            Some(path) => files.push(("htmlPath", path, html)),
            None => out["html"] = json!(html),
        }
    }
    for (_, path, _) in &files {
        writable(path).map_err(|e| bad(cmd, format!("{path}: {e}")))?;
    }
    let list: Vec<(&str, &str)> = files.iter().map(|(_, path, text)| (path.as_str(), text.as_str())).collect();
    write_files_atomically(&list, &rename_path).map_err(|e| bad(cmd, e))?;
    for (key, path, _) in &files {
        out[*key] = json!(path);
    }
    if changed {
        s.edit_prefs(|p| *p = next);
    }
    if let Some(import) = import {
        out["import"] = import;
    }
    Ok(out)
}

fn apply_shortcut_changes(
    next: &mut Preferences,
    p: &Value,
    set: &serde_json::Map<String, Value>,
    alts: &serde_json::Map<String, Value>,
    cmd: &str,
) -> Result<bool> {
    if set.is_empty() && alts.is_empty() {
        return Ok(false);
    }
    let allow_unknown = p.get("allowUnknown").and_then(Value::as_bool).unwrap_or(false);
    // Reserved system keys can't be bound, by hand or from a file.
    let reserved = |v: &Value| -> Option<String> {
        let keys = match v {
            Value::String(s) => vec![s.clone()],
            other => alternate_keys(other).unwrap_or_default(),
        };
        keys.into_iter().find(|k| reserved_shortcut(k))
    };
    if let Some((id, k)) = set.iter().chain(alts.iter()).find_map(|(id, v)| reserved(v).map(|k| (id, k))) {
        return Err(bad(cmd, format!("`{k}` (for `{id}`) is reserved by the system and can't be assigned")));
    }
    for (id, v) in set {
        if !bindable().any(|(b, _)| b == id) && !allow_unknown {
            return Err(bad(cmd, format!("unknown command `{id}`")));
        }
        next.set(&format!("shortcuts.{id}"), v.clone()).map_err(|e| bad(cmd, e))?;
    }
    for (id, v) in alts {
        if !bindable().any(|(b, _)| b == id) && default_alternates(id).next().is_none() && !allow_unknown {
            return Err(bad(cmd, format!("unknown command `{id}`")));
        }
        next.set(&format!("alternateShortcuts.{id}"), v.clone()).map_err(|e| bad(cmd, e))?;
    }
    // Like Photoshop, a shortcut moved to a new command is taken away from the old one,
    // whether the old one held it as its main key or as an alternate (#3665).
    if p.get("removeConflicts").and_then(Value::as_bool).unwrap_or(true) {
        let assigned: Vec<(&str, String)> = set
            .iter()
            .filter_map(|(id, v)| Some((id.as_str(), v.as_str().and_then(normalize_shortcut)?)))
            .chain(alts.iter().flat_map(|(id, v)| alternate_keys(v).unwrap_or_default().into_iter().map(move |k| (id.as_str(), k))))
            .collect();
        for (id, sc) in assigned {
            for (c, def) in bindable() {
                // Tools may share a key: they cycle with Shift, as Photoshop's groups do.
                if is_tool(id) && is_tool(c) {
                    continue;
                }
                // Never strip a command that this same call is assigning:
                // two entries for one key are a clash inside the call, which
                // the returned conflicts list reports (#719). Stripping
                // each other here unbound both silently.
                if c == id || set.contains_key(c) || alts.contains_key(c) {
                    continue;
                }
                let main = next.shortcut(c, def).map(str::to_string);
                if main.as_deref().and_then(normalize_shortcut).as_deref() == Some(sc.as_str()) {
                    next.shortcuts.insert(c.to_string(), String::new());
                }
                let held = next.alternates(c, main.as_deref());
                if held.contains(&sc) {
                    next.alternate_shortcuts.insert(c.to_string(), held.into_iter().filter(|k| *k != sc).collect());
                }
            }
        }
    }
    Ok(true)
}

/// Edit › Menus: hide/show items and give them colours.
pub(super) fn menus(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = |k: &str| -> Vec<String> {
        p.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
    };
    const COLORS: [&str; 8] = ["none", "red", "orange", "yellow", "green", "blue", "violet", "gray"];
    if let Some(m) = p.get("color").and_then(Value::as_object) {
        for v in m.values() {
            if let Some(c) = v.as_str().filter(|c| !COLORS.contains(c)) {
                return Err(bad("edit.menus", format!("unknown colour `{c}` ({})", COLORS.join("|"))));
            }
        }
    }
    let (hide, show) = (ids("hide"), ids("show"));
    if let Some(id) = hide.iter().find(|id| !MenuCustomization::hideable(id)) {
        return Err(bad("edit.menus", format!("`{id}` can't be hidden")));
    }
    s.edit_prefs(|pr| {
        if p.get("reset").and_then(Value::as_bool) == Some(true) {
            pr.menus = MenuCustomization::default();
        }
        for id in hide {
            if !pr.menus.hidden.contains(&id) {
                pr.menus.hidden.push(id);
            }
        }
        pr.menus.hidden.retain(|h| !show.contains(h));
        if let Some(m) = p.get("color").and_then(Value::as_object) {
            for (id, v) in m {
                match v.as_str() {
                    Some(c) if c != "none" => {
                        pr.menus.colors.insert(id.clone(), c.to_string());
                    }
                    _ => {
                        pr.menus.colors.remove(id);
                    }
                }
            }
        }
    });
    Ok(serde_json::to_value(&s.prefs().menus).unwrap_or_default())
}
