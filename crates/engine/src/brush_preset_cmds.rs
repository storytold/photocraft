//! Organising brush presets (the Brushes panel's drag and drop and context menus): rename, move a
//! preset within or between groups and folders, move, rename and delete whole groups and folders.
//!
//! Groups are the panel's top-level folders ([`BrushPreset::group`]); a group can hold nested
//! folders ([`BrushPreset::folder`], a path inside the group, e.g. the folders of an imported
//! `.abr` file). The group commands take an optional `folder` path to act on a nested folder
//! instead of the whole group, and `moveGroup` can move a group or folder `into` another folder
//! or group, or out to the top level (dragging a folder onto a folder in Photoshop's panel). A
//! folders are normally reconstructed from preset paths; explicitly-created empty groups and
//! folders are stored in the preset index so they survive a restart too.
//!
//! The library order is the panel order: groups appear in order of their first preset, and a
//! group's folders and presets in order of their first preset too. Every
//! change syncs to the preset store ([`crate::preset_store`]), whose index keeps the group order
//! and the order of every preset, built-ins included. A built-in that is renamed or moved to
//! another group becomes the user's preset (built-ins are regenerated with their own group).

use photocraft_paint::BrushPreset;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

impl crate::presets::Named for BrushPreset {
    fn name(&self) -> &str {
        &self.name
    }
    fn set_name(&mut self, name: String) {
        self.name = name;
    }
}

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}
fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

/// Longest preset or group name accepted.
pub const MAX_NAME: usize = 255;
/// Deepest folder path accepted inside a group (matches the `.abr` reader's limit).
pub const MAX_FOLDER_DEPTH: usize = photocraft_psd::abr::MAX_FOLDER_DEPTH;

fn text(p: &Value, k: &str, cmd: &str) -> Result<String> {
    let v = p.get(k).and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(cmd, format!("missing `{k}`")))?;
    if v.chars().count() > MAX_NAME {
        return Err(bad(cmd, format!("`{k}` is longer than {MAX_NAME} characters")));
    }
    Ok(v.to_string())
}

/// A group name param: may be empty (the ungrouped presets), never absurdly long.
fn group_param(p: &Value, k: &str, cmd: &str) -> Result<Option<String>> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(g)) if g.chars().count() <= MAX_NAME => Ok(Some(g.trim().to_string())),
        Some(_) => Err(bad(cmd, format!("`{k}` must be a group name (string, at most {MAX_NAME} characters)"))),
    }
}

/// A folder path param (`["Outer", "Inner"]`, inside the group): `None` when absent or null.
fn folder_param(p: &Value, cmd: &str) -> Result<Option<Vec<String>>> {
    let err = || bad(cmd, format!("`folder` must be a list of at most {MAX_FOLDER_DEPTH} folder names (non-empty strings of at most {MAX_NAME} characters)"));
    match p.get("folder") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(a)) if a.len() <= MAX_FOLDER_DEPTH => a
            .iter()
            .map(|v| v.as_str().map(str::trim).filter(|n| !n.is_empty() && n.chars().count() <= MAX_NAME).map(str::to_string).ok_or_else(err))
            .collect::<Result<Vec<_>>>()
            .map(Some),
        Some(_) => Err(err()),
    }
}

/// Is `x` in folder `f` of group `g`, directly or below it?
fn in_folder(x: &BrushPreset, g: &str, f: &[String]) -> bool {
    x.group == g && x.folder.starts_with(f)
}

/// The subfolders directly inside folder `parent` of group `g`, in panel order.
pub fn subfolder_order(presets: &[BrushPreset], g: &str, parent: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for x in presets.iter().filter(|x| in_folder(x, g, parent)) {
        if let Some(n) = x.folder.get(parent.len())
            && !out.contains(n)
        {
            out.push(n.clone());
        }
    }
    out
}

fn index_of(presets: &[BrushPreset], name: &str) -> Option<usize> {
    presets.iter().position(|x| x.name.eq_ignore_ascii_case(name))
}

fn find(s: &Session, p: &Value, k: &str, cmd: &str) -> Result<usize> {
    let name = text(p, k, cmd)?;
    index_of(&s.tools.presets, &name).ok_or_else(|| bad(cmd, format!("no brush preset named `{name}`")))
}

/// Groups in panel order (first appearance).
pub fn group_order(presets: &[BrushPreset]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in presets {
        if !out.contains(&p.group) {
            out.push(p.group.clone());
        }
    }
    out
}

fn session_group_order(s: &Session) -> Vec<String> {
    let all = group_order(&s.tools.presets);
    let builtin: Vec<String> = all.iter().filter(|group| s.tools.presets.iter().any(|preset| preset.builtin && preset.group == **group)).cloned().collect();
    let derived: Vec<String> =
        all.into_iter().filter(|group| !group.is_empty() && s.tools.presets.iter().any(|preset| !preset.builtin && preset.group == *group)).collect();
    let complete = derived.iter().all(|group| s.tools.preset_hierarchy.groups.contains(group));
    let mut out = builtin;
    out.extend(if complete { s.tools.preset_hierarchy.groups.clone() } else { derived });
    for group in &s.tools.preset_hierarchy.groups {
        if !out.contains(group) {
            out.push(group.clone());
        }
    }
    out
}

fn session_root_order(s: &Session) -> Vec<crate::BrushPresetRootEntry> {
    use crate::BrushPresetRootEntry as Root;

    let mut derived = Vec::new();
    let mut seen_groups = std::collections::BTreeSet::new();
    for preset in &s.tools.presets {
        if preset.group.is_empty() && preset.folder.is_empty() {
            let entry = Root::Preset(preset.name.clone());
            if !derived.contains(&entry) {
                derived.push(entry);
            }
        } else if !preset.group.is_empty() && seen_groups.insert(preset.group.clone()) {
            derived.push(Root::Group(preset.group.clone()));
        }
    }
    for group in session_group_order(s) {
        if seen_groups.insert(group.clone()) {
            derived.push(Root::Group(group));
        }
    }

    let stored: Vec<_> = s
        .tools
        .preset_hierarchy
        .root_order
        .iter()
        .filter(|entry| match entry {
            Root::Group(group) => has_group(s, group),
            Root::Preset(name) => s.tools.presets.iter().any(|preset| preset.group.is_empty() && preset.folder.is_empty() && preset.name == *name),
        })
        .cloned()
        .collect();
    if stored.is_empty() {
        derived
    } else {
        let mut out = stored;
        for entry in derived {
            if !out.contains(&entry) {
                out.push(entry);
            }
        }
        out
    }
}

fn set_root_order(s: &mut Session, order: Vec<crate::BrushPresetRootEntry>) {
    s.tools.preset_hierarchy.root_order = order;
}

fn insert_root_entry(order: &mut Vec<crate::BrushPresetRootEntry>, entry: crate::BrushPresetRootEntry, index: Option<usize>) {
    order.retain(|candidate| candidate != &entry);
    let at = index.unwrap_or(order.len()).min(order.len());
    order.insert(at, entry);
}

fn root_anchor_index(order: &[crate::BrushPresetRootEntry], start: usize, presets: &[BrushPreset]) -> Option<usize> {
    order.iter().skip(start).find_map(|entry| match entry {
        crate::BrushPresetRootEntry::Group(group) => presets.iter().position(|preset| preset.group == *group),
        crate::BrushPresetRootEntry::Preset(name) => {
            presets.iter().position(|preset| preset.group.is_empty() && preset.folder.is_empty() && preset.name == *name)
        }
    })
}

fn session_subfolder_order(s: &Session, group: &str, parent: &[String]) -> Vec<String> {
    let derived: Vec<String> = subfolder_order(&s.tools.presets, group, parent)
        .into_iter()
        .filter(|name| s.tools.presets.iter().any(|preset| !preset.builtin && preset.group == group && preset.folder.get(parent.len()) == Some(name)))
        .collect();
    let stored: Vec<String> = s
        .tools
        .preset_hierarchy
        .folders
        .iter()
        .filter_map(|folder| {
            (folder.group == group && folder.path.len() == parent.len() + 1 && folder.path.starts_with(parent)).then(|| folder.path.last().cloned()).flatten()
        })
        .fold(Vec::new(), |mut out, name| {
            if !out.contains(&name) {
                out.push(name);
            }
            out
        });
    let complete = derived.iter().all(|name| stored.contains(name));
    let mut out = if complete { stored.clone() } else { derived };
    for name in stored {
        if !out.iter().any(|existing| existing == &name) {
            out.push(name);
        }
    }
    out
}

/// Position a child folder against direct presets in its parent. Populated folders start at
/// their first descendant preset; empty folders use their persisted `before_item` anchor.
fn subfolder_boundary(s: &Session, group: &str, parent: &[String], name: &str) -> usize {
    let mut path = parent.to_vec();
    path.push(name.to_owned());
    if let Some(first_descendant) = s.tools.presets.iter().position(|preset| preset.group == group && preset.folder.starts_with(&path)) {
        return first_descendant;
    }
    let anchor =
        s.tools.preset_hierarchy.folders.iter().find(|folder| folder.group == group && folder.path == path).and_then(|folder| folder.before_item.as_ref());
    anchor
        .and_then(|name| s.tools.presets.iter().position(|preset| preset.group == group && preset.folder == parent && preset.name.eq_ignore_ascii_case(name)))
        .unwrap_or(usize::MAX)
}

fn ensure_group_order_metadata(s: &mut Session) {
    let derived: Vec<String> = group_order(&s.tools.presets)
        .into_iter()
        .filter(|group| !group.is_empty() && s.tools.presets.iter().any(|preset| !preset.builtin && preset.group == *group))
        .collect();
    if derived.iter().all(|group| s.tools.preset_hierarchy.groups.contains(group)) {
        return;
    }
    let mut groups = derived;
    for group in std::mem::take(&mut s.tools.preset_hierarchy.groups) {
        if !groups.contains(&group) {
            groups.push(group);
        }
    }
    s.tools.preset_hierarchy.groups = groups;
}

fn ensure_folder_order_metadata(s: &mut Session, group: &str) {
    let mut derived = Vec::new();
    for preset in s.tools.presets.iter().filter(|preset| preset.group == group) {
        for depth in 1..=preset.folder.len().min(MAX_FOLDER_DEPTH) {
            let folder = crate::BrushPresetFolder { group: group.to_string(), path: preset.folder[..depth].to_vec(), before_item: None };
            if !derived.contains(&folder) {
                derived.push(folder);
            }
        }
    }
    let existing: Vec<_> = s.tools.preset_hierarchy.folders.iter().filter(|folder| folder.group == group).cloned().collect();
    if derived.iter().all(|folder| existing.contains(folder)) {
        return;
    }
    let mut ordered = derived;
    for folder in existing {
        if !ordered.contains(&folder) {
            ordered.push(folder);
        }
    }
    s.tools.preset_hierarchy.folders.retain(|folder| folder.group != group);
    s.tools.preset_hierarchy.folders.extend(ordered);
}

fn has_group(s: &Session, group: &str) -> bool {
    s.tools.presets.iter().any(|p| p.group == group) || s.tools.preset_hierarchy.groups.iter().any(|g| g == group)
}

fn group_name_conflicts(s: &Session, current: &str, candidate: &str) -> bool {
    s.tools.presets.iter().any(|p| p.group.eq_ignore_ascii_case(candidate) && !p.group.eq_ignore_ascii_case(current))
        || s.tools.preset_hierarchy.groups.iter().any(|g| g.eq_ignore_ascii_case(candidate) && !g.eq_ignore_ascii_case(current))
}

fn has_folder(s: &Session, group: &str, path: &[String]) -> bool {
    has_group(s, group)
        && (path.is_empty()
            || s.tools.presets.iter().any(|p| in_folder(p, group, path))
            || s.tools.preset_hierarchy.folders.iter().any(|f| f.group == group && f.path == path))
}

fn move_hierarchy_prefix(s: &mut Session, old_group: &str, old_path: &[String], new_group: &str, new_path: &[String]) {
    for folder in &mut s.tools.preset_hierarchy.folders {
        if folder.group == old_group && folder.path.starts_with(old_path) {
            let tail = folder.path[old_path.len()..].to_vec();
            folder.group = new_group.to_string();
            folder.path = new_path.iter().cloned().chain(tail).collect();
        }
    }
    if old_path.is_empty() && old_group != new_group {
        s.tools.preset_hierarchy.groups.retain(|group| group != old_group);
    }
    if !new_path.is_empty() {
        for depth in 1..=new_path.len() {
            let path = new_path[..depth].to_vec();
            if !s.tools.preset_hierarchy.folders.iter().any(|folder| folder.group == new_group && folder.path == path) {
                s.tools.preset_hierarchy.folders.push(crate::BrushPresetFolder { group: new_group.to_string(), path, before_item: None });
            }
        }
    }
}

fn reorder_folder_metadata(s: &mut Session, group: &str, parent: &[String], order: &[String]) {
    let mut buckets: std::collections::BTreeMap<String, Vec<crate::BrushPresetFolder>> = std::collections::BTreeMap::new();
    let mut rest = Vec::new();
    for folder in std::mem::take(&mut s.tools.preset_hierarchy.folders) {
        if folder.group == group && folder.path.len() > parent.len() && folder.path.starts_with(parent) {
            if let Some(name) = folder.path.get(parent.len()) {
                buckets.entry(name.clone()).or_default().push(folder);
            }
        } else {
            rest.push(folder);
        }
    }
    for name in order {
        if let Some(mut subtree) = buckets.remove(name) {
            rest.append(&mut subtree);
        }
    }
    for (_, mut subtree) in buckets {
        rest.append(&mut subtree);
    }
    s.tools.preset_hierarchy.folders = rest;
}

fn position_in_group(presets: &[BrushPreset], i: usize) -> usize {
    presets.get(i).map_or(0, |pr| presets[..i].iter().filter(|x| x.group == pr.group).count())
}

fn rename(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.presets.rename";
    let i = find(s, p, "name", cmd)?;
    let new = text(p, "newName", cmd)?;
    rename_at(s, i, &new, cmd)?;
    Ok(json!({ "name": new }))
}

/// Rename preset `i` (shared by Brush Settings and Edit › Presets › Preset Manager): refuses a
/// name another preset has (case-insensitively), makes the preset the user's and keeps the
/// current preset pointing at it.
pub(crate) fn rename_at(s: &mut Session, i: usize, new: &str, cmd: &str) -> Result<()> {
    if index_of(&s.tools.presets, new).is_some_and(|j| j != i) {
        return Err(bad(cmd, format!("a brush preset named `{new}` already exists")));
    }
    let mut root_order = session_root_order(s);
    let (old_name, was_current, is_root) = {
        let preset = s.tools.presets.get(i).ok_or_else(|| bad(cmd, "no such preset"))?;
        (
            preset.name.clone(),
            s.tools.current_preset.as_ref().is_some_and(|current| current.eq_ignore_ascii_case(&preset.name)),
            preset.group.is_empty() && preset.folder.is_empty(),
        )
    };
    if old_name != new {
        let preset = s.tools.presets.get_mut(i).ok_or_else(|| bad(cmd, "no such preset"))?;
        preset.name = new.to_string();
        preset.builtin = false;
        if is_root {
            for entry in &mut root_order {
                if *entry == crate::BrushPresetRootEntry::Preset(old_name.clone()) {
                    *entry = crate::BrushPresetRootEntry::Preset(new.to_string());
                }
            }
            set_root_order(s, root_order);
        }
        if was_current {
            s.tools.current_preset = Some(new.to_string());
            s.tools.brush_changed();
        }
        s.brush_presets_changed();
    }
    Ok(())
}

fn move_preset(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.presets.move";
    let mut root_order = session_root_order(s);
    let i = p
        .get("presetId")
        .and_then(Value::as_str)
        .map(|id| s.tools.presets.iter().position(|preset| preset.id == id).ok_or_else(|| bad(cmd, "no brush preset with that ID")))
        .unwrap_or_else(|| find(s, p, "name", cmd))?;
    let before = match p.get("beforeId").or_else(|| p.get("before")) {
        None | Some(Value::Null) => None,
        Some(_) => {
            let b = if p.get("beforeId").and_then(Value::as_str).is_some() {
                let id = p["beforeId"].as_str().unwrap_or_default();
                s.tools.presets.iter().position(|preset| preset.id == id).ok_or_else(|| bad(cmd, "no brush preset with that before ID"))?
            } else {
                find(s, p, "before", cmd)?
            };
            if b == i {
                return Err(bad(cmd, "`before` names the preset being moved"));
            }
            Some(b)
        }
    };
    let index = match p.get("index") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().ok_or_else(|| bad(cmd, "`index` must be a non-negative integer"))?),
    };
    let root_index = match p.get("rootIndex") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().ok_or_else(|| bad(cmd, "`rootIndex` must be a non-negative integer"))? as usize),
    };
    let folder = folder_param(p, cmd)?;
    let group = group_param(p, "group", cmd)?;
    let lib = &mut s.tools.presets;
    let (Some(moving), before_pr) = (lib.get(i), before.and_then(|b| lib.get(b))) else {
        return Err(bad(cmd, "no such preset"));
    };
    let source_was_root = moving.group.is_empty() && moving.folder.is_empty();
    let source_root_position = root_order.iter().position(|entry| *entry == crate::BrushPresetRootEntry::Preset(moving.name.clone()));
    let mut root_siblings = root_order.clone();
    if source_was_root {
        root_siblings.retain(|entry| *entry != crate::BrushPresetRootEntry::Preset(moving.name.clone()));
    }
    let target = match (&group, before_pr) {
        (Some(g), _) => g.clone(),
        (None, Some(b)) => b.group.clone(),
        (None, None) => moving.group.clone(),
    };
    // The folder: as given, else `before`'s, else its own when it stays in its group.
    let target_folder = match (folder, before_pr) {
        (Some(f), _) => f,
        (None, Some(b)) => b.folder.clone(),
        (None, None) if moving.group == target => moving.folder.clone(),
        (None, None) => Vec::new(),
    };
    if let Some(b) = before_pr {
        if b.group != target {
            return Err(bad(cmd, format!("`before` is not in group `{target}`")));
        }
        if b.folder != target_folder {
            return Err(bad(cmd, "`before` is not in that folder"));
        }
    }
    if root_index.is_some() && (target != "" || !target_folder.is_empty() || before_pr.is_some()) {
        return Err(bad(cmd, "`rootIndex` is only valid when moving a preset to the root, without `before`"));
    }
    let before_name = before_pr.map(|b| b.name.clone());
    let source_group = moving.group.clone();
    let source_folder = moving.folder.clone();
    let source_name = moving.name.clone();
    let leaves_source_parent = source_group != target || source_folder != target_folder;
    let mut pr = lib.remove(i);
    if leaves_source_parent {
        // Empty sibling folders persist their position as `before_item`. If that card leaves
        // the parent (e.g. it is moved into a newly-created folder), retarget those folders to
        // the next remaining direct card. Repeated moves naturally walk across a multi-selection
        // until the anchor points at the first unselected card.
        let next_anchor =
            lib.iter().skip(i.min(lib.len())).find(|preset| preset.group == source_group && preset.folder == source_folder).map(|preset| preset.name.clone());
        for folder in &mut s.tools.preset_hierarchy.folders {
            if folder.group == source_group
                && folder.path.len() == source_folder.len() + 1
                && folder.path.starts_with(&source_folder)
                && folder.before_item.as_deref().is_some_and(|anchor| anchor.eq_ignore_ascii_case(&source_name))
            {
                folder.before_item = next_anchor.clone();
            }
        }
    }
    if pr.group != target || pr.folder != target_folder {
        pr.group = target.clone();
        pr.folder = target_folder.clone();
        // A built-in moved elsewhere is the user's now (built-ins regenerate in their own group).
        pr.builtin = false;
    }
    let members: Vec<usize> = lib.iter().enumerate().filter(|(_, x)| x.group == target).map(|(k, _)| k).collect();
    let root_flat_pos = root_index.and_then(|index| root_anchor_index(&root_siblings, index, lib));
    // Appended: after the folder's last preset (or the group's, for a new folder).
    let end = lib.iter().rposition(|x| in_folder(x, &target, &target_folder)).or(members.last().copied()).map_or(lib.len(), |k| k + 1);
    let pos = match (before_name.clone(), index) {
        (Some(n), _) => index_of(lib, &n).unwrap_or(lib.len()),
        (None, Some(_)) if root_index.is_some() => root_flat_pos.unwrap_or(lib.len()),
        (None, Some(n)) => match members.get(n.min(usize::MAX as u64) as usize) {
            Some(&k) => k,
            None => members.last().map_or(lib.len(), |k| k + 1),
        },
        (None, None) if root_index.is_some() => root_flat_pos.unwrap_or(lib.len()),
        (None, None) => end,
    };
    let pos = pos.min(lib.len());
    let name = pr.name.clone();
    lib.insert(pos, pr);
    if source_was_root {
        root_order.retain(|entry| *entry != crate::BrushPresetRootEntry::Preset(name.clone()));
    }
    if target.is_empty() && target_folder.is_empty() {
        let insert_at = root_index
            .or_else(|| {
                before_name.as_ref().and_then(|before| root_order.iter().position(|entry| *entry == crate::BrushPresetRootEntry::Preset(before.clone())))
            })
            .or_else(|| {
                index.map(|index| {
                    root_order
                        .iter()
                        .enumerate()
                        .filter(|(_, entry)| matches!(entry, crate::BrushPresetRootEntry::Preset(_)))
                        .nth(index.min(usize::MAX as u64) as usize)
                        .map_or(root_order.len(), |(position, _)| position)
                })
            });
        insert_root_entry(&mut root_order, crate::BrushPresetRootEntry::Preset(name.clone()), insert_at);
    } else if !target.is_empty() && !root_order.contains(&crate::BrushPresetRootEntry::Group(target.clone())) {
        insert_root_entry(&mut root_order, crate::BrushPresetRootEntry::Group(target.clone()), source_root_position);
    }
    let at = position_in_group(lib, pos);
    set_root_order(s, root_order);
    s.brush_presets_changed();
    Ok(json!({ "name": name, "group": target, "folder": target_folder, "index": at }))
}

fn group_block(s: &Session, g: &str, cmd: &str) -> Result<()> {
    if has_group(s, g) { Ok(()) } else { Err(bad(cmd, format!("no brush preset group named `{g}`"))) }
}

/// The group and (possibly empty) folder path a group command acts on; both must exist.
fn target_folder(s: &Session, p: &Value, cmd: &str) -> Result<(String, Vec<String>)> {
    let g = group_param(p, "group", cmd)?.ok_or_else(|| bad(cmd, "missing `group`"))?;
    group_block(s, &g, cmd)?;
    let f = folder_param(p, cmd)?.unwrap_or_default();
    if !has_folder(s, &g, &f) {
        return Err(bad(cmd, format!("no folder `{}` in brush preset group `{g}`", f.join("/"))));
    }
    Ok((g, f))
}

/// Move folder `f` (non-empty) of group `g` among its sibling folders, as a block.
fn move_folder(s: &mut Session, p: &Value, g: String, f: Vec<String>, cmd: &str) -> Result<Value> {
    let Some((leaf, parent)) = f.split_last() else { return Err(bad(cmd, "missing `folder`")) };
    ensure_folder_order_metadata(s, &g);
    let siblings = session_subfolder_order(s, &g, parent);
    let before = match p.get("before") {
        None | Some(Value::Null) => None,
        Some(Value::String(b)) if b.eq_ignore_ascii_case(leaf) => return Err(bad(cmd, "`before` names the folder being moved")),
        Some(Value::String(b)) if siblings.iter().any(|name| name.eq_ignore_ascii_case(b)) => Some(b.clone()),
        Some(_) => return Err(bad(cmd, "`before` must name a folder next to the one being moved")),
    };
    let index = match p.get("index") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().ok_or_else(|| bad(cmd, "`index` must be a non-negative integer"))?),
    };
    let lib = std::mem::take(&mut s.tools.presets);
    // Where the block sits among the other presets (its place when nothing else is in the parent).
    let start = lib.iter().position(|x| in_folder(x, &g, &f)).unwrap_or(0);
    let (block, mut rest): (Vec<BrushPreset>, Vec<BrushPreset>) = lib.into_iter().partition(|x| in_folder(x, &g, &f));
    let sibling = |name: &str| -> Vec<String> { parent.iter().cloned().chain(std::iter::once(name.to_string())).collect() };
    let first = |name: &str, rest: &[BrushPreset]| rest.iter().position(|x| in_folder(x, &g, &sibling(name))).unwrap_or(rest.len());
    let others: Vec<&String> = siblings.iter().filter(|n| *n != leaf).collect();
    let parent_end = |rest: &[BrushPreset]| rest.iter().rposition(|x| in_folder(x, &g, parent)).map_or(start, |k| k + 1);
    let pos = match (&before, index) {
        (Some(b), _) => first(b, &rest),
        (None, Some(n)) => match others.get(n.min(usize::MAX as u64) as usize) {
            Some(name) => first(name, &rest),
            None => parent_end(&rest),
        },
        (None, None) => parent_end(&rest),
    };
    let tail = rest.split_off(pos.min(rest.len()));
    rest.extend(block);
    rest.extend(tail);
    s.tools.presets = rest;
    let mut ordered_siblings = siblings.clone();
    ordered_siblings.retain(|name| name != leaf);
    let at = before
        .as_ref()
        .and_then(|name| ordered_siblings.iter().position(|candidate| candidate == name))
        .or(index.map(|n| n.min(ordered_siblings.len() as u64) as usize))
        .unwrap_or(ordered_siblings.len());
    ordered_siblings.insert(at, leaf.clone());
    reorder_folder_metadata(s, &g, parent, &ordered_siblings);
    s.brush_presets_changed();
    let at = session_subfolder_order(s, &g, parent).iter().position(|x| x == leaf).unwrap_or(0);
    Ok(json!({ "group": g, "folder": f, "index": at }))
}

/// Where `into` puts a folder: `Some(None)` = the top level (it becomes a group), `Some(Some((group,
/// folder)))` = inside that folder of that group (`[]`: directly in the group). `None` without `into`.
type Dest = Option<(String, Vec<String>)>;

fn into_param(s: &Session, p: &Value, cmd: &str) -> Result<Option<Dest>> {
    let into = match p.get("into") {
        None | Some(Value::Null) => return Ok(None),
        Some(v @ Value::Object(_)) => v,
        Some(_) => return Err(bad(cmd, "`into` must be an object {group?, folder?}")),
    };
    let Some(g) = group_param(into, "group", cmd)? else {
        if folder_param(into, cmd)?.is_some_and(|f| !f.is_empty()) {
            return Err(bad(cmd, "`into.folder` needs `into.group`"));
        }
        return Ok(Some(None));
    };
    group_block(s, &g, cmd)?;
    let f = folder_param(into, cmd)?.unwrap_or_default();
    if !has_folder(s, &g, &f) {
        return Err(bad(cmd, format!("no folder `{}` in brush preset group `{g}`", f.join("/"))));
    }
    Ok(Some(Some((g, f))))
}

/// Move folder `f` of group `g` (the whole group when `f` is empty) into `dest`, keeping its
/// content and the order of its presets: Photoshop's drag of a folder onto another folder. It
/// lands at the end of the destination, or before the sibling folder (group at the top level)
/// named by `before`.
fn move_into(s: &mut Session, p: &Value, g: String, f: Vec<String>, dest: Dest, cmd: &str) -> Result<Value> {
    ensure_group_order_metadata(s);
    ensure_folder_order_metadata(s, &g);
    let mut root_order = session_root_order(s);
    if let Some((destination_group, _)) = &dest {
        if destination_group != &g {
            ensure_folder_order_metadata(s, destination_group);
        }
    }
    if p.get("index").is_some_and(|v| !v.is_null()) {
        return Err(bad(cmd, "`index` can't be combined with `into` (use `before`)"));
    }
    let leaf = match f.last() {
        Some(l) => l.clone(),
        None if g.is_empty() => return Err(bad(cmd, "the ungrouped presets are not a folder that can be moved")),
        None => g.clone(),
    };
    let moving = |x: &BrushPreset| in_folder(x, &g, &f);
    let rest_of = |x: &BrushPreset| x.folder.get(f.len()..).unwrap_or_default().to_vec();
    let before = match p.get("before") {
        None | Some(Value::Null) => None,
        Some(Value::String(b)) => Some(b.trim().to_string()),
        Some(_) => return Err(bad(cmd, "`before` must be a folder name")),
    };
    let before_item = match p.get("beforeItem") {
        None | Some(Value::Null) => None,
        Some(Value::String(name)) if !name.trim().is_empty() && name.chars().count() <= MAX_NAME => Some(name.trim().to_string()),
        Some(_) => return Err(bad(cmd, format!("`beforeItem` must be a non-empty preset name of at most {MAX_NAME} characters"))),
    };
    let root_index = match p.get("rootIndex") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().ok_or_else(|| bad(cmd, "`rootIndex` must be a non-negative integer"))? as usize),
    };
    if root_index.is_some() && (dest.is_some() || before.is_some() || before_item.is_some()) {
        return Err(bad(cmd, "`rootIndex` is only valid when moving a group to the root, without `before` or `beforeItem`"));
    }
    let mut root_siblings = root_order.clone();
    if f.is_empty() {
        root_siblings.retain(|entry| *entry != crate::BrushPresetRootEntry::Group(g.clone()));
    }
    if before.is_some() && before_item.is_some() {
        return Err(bad(cmd, "`before` and `beforeItem` cannot be combined"));
    }
    // The destination's own folders and groups, without the one being moved.
    let others: Vec<BrushPreset> = s.tools.presets.iter().filter(|x| !moving(x)).cloned().collect();
    let (new_group, new_prefix, siblings) = match &dest {
        None => {
            if f.is_empty() && before_item.is_none() {
                return Err(bad(cmd, "`into`: the group is already at the top level"));
            }
            (leaf.clone(), Vec::new(), session_group_order(s).into_iter().filter(|name| name != &leaf).collect())
        }
        Some((dg, df)) => {
            if *dg == g && df.starts_with(&f) {
                return Err(bad(cmd, "`into` names the folder being moved or one of its own folders"));
            }
            let mut prefix = df.clone();
            prefix.push(leaf.clone());
            let moving_from_this_parent = *dg == g && f.len() == df.len() + 1 && f.starts_with(df);
            (dg.clone(), prefix, session_subfolder_order(s, dg, df).into_iter().filter(|name| !(moving_from_this_parent && name == &leaf)).collect::<Vec<_>>())
        }
    };
    if siblings.iter().any(|name| name.eq_ignore_ascii_case(&leaf)) {
        return Err(bad(cmd, format!("a folder named `{leaf}` already exists there")));
    }
    if let Some(b) = &before
        && !siblings.contains(b)
    {
        return Err(bad(cmd, format!("`before`: no folder named `{b}` there")));
    }
    if let Some(anchor) = &before_item {
        let (dg, df) = dest.as_ref().map_or(("", &[][..]), |(group, folder)| (group.as_str(), folder.as_slice()));
        let matches = others.iter().filter(|x| x.name.eq_ignore_ascii_case(anchor) && x.group == dg && x.folder == df).count();
        if matches != 1 {
            return Err(bad(cmd, format!("`beforeItem`: expected one direct preset named `{anchor}` in the destination folder")));
        }
    }
    let deepest = s.tools.presets.iter().filter(|x| moving(x)).map(|x| new_prefix.len() + rest_of(x).len()).max().unwrap_or(0);
    if deepest > MAX_FOLDER_DEPTH {
        return Err(bad(cmd, format!("folders can be nested at most {MAX_FOLDER_DEPTH} deep")));
    }
    let lib = std::mem::take(&mut s.tools.presets);
    let (mut block, mut rest): (Vec<BrushPreset>, Vec<BrushPreset>) = lib.into_iter().partition(|x| in_folder(x, &g, &f));
    for x in &mut block {
        let tail = rest_of(x);
        x.group.clone_from(&new_group);
        x.folder = new_prefix.iter().cloned().chain(tail).collect();
        x.builtin = false;
    }
    let root_flat_pos = root_index.and_then(|index| root_anchor_index(&root_siblings, index, &rest));
    let pos = match (&dest, &before, &before_item) {
        (Some((dg, df)), None, Some(anchor)) => rest.iter().position(|x| x.name.eq_ignore_ascii_case(anchor) && x.group == *dg && x.folder == *df),
        (None, None, Some(anchor)) => rest.iter().position(|x| x.name.eq_ignore_ascii_case(anchor) && x.group.is_empty() && x.folder.is_empty()),
        (None, Some(b), None) => rest.iter().position(|x| x.group == *b),
        (None, None, None) => root_flat_pos,
        (Some((dg, df)), Some(b), None) => {
            let sib: Vec<String> = df.iter().cloned().chain(std::iter::once(b.clone())).collect();
            rest.iter().position(|x| in_folder(x, dg, &sib))
        }
        (Some((dg, df)), None, None) => rest.iter().rposition(|x| in_folder(x, dg, df)).map(|k| k + 1),
        _ => None,
    };
    let tail = rest.split_off(pos.unwrap_or(rest.len()).min(rest.len()));
    rest.extend(block);
    rest.extend(tail);
    s.tools.presets = rest;
    move_hierarchy_prefix(s, &g, &f, &new_group, &new_prefix);
    if let Some((destination_group, destination_path)) = dest.as_ref() {
        let inherited_anchor = before.as_ref().and_then(|name| {
            let mut sibling_path = destination_path.clone();
            sibling_path.push(name.clone());
            s.tools
                .preset_hierarchy
                .folders
                .iter()
                .find(|folder| folder.group == *destination_group && folder.path == sibling_path)
                .and_then(|folder| folder.before_item.clone())
        });
        if let Some(folder) = s.tools.preset_hierarchy.folders.iter_mut().find(|folder| folder.group == *destination_group && folder.path == new_prefix) {
            folder.before_item = before_item.clone().or(inherited_anchor);
        }
        let leaf = new_prefix.last().cloned().unwrap_or_default();
        let mut order = session_subfolder_order(s, destination_group, destination_path);
        order.retain(|name| name != &leaf);
        let at = before
            .as_ref()
            .and_then(|anchor| order.iter().position(|name| name == anchor))
            .or_else(|| {
                let anchor = before_item.as_ref()?;
                let anchor_index =
                    s.tools.presets.iter().position(|preset| {
                        preset.group == *destination_group && preset.folder == *destination_path && preset.name.eq_ignore_ascii_case(anchor)
                    })?;
                // Existing folders whose boundary is this card (or an earlier card) already
                // precede the card. Insert after them so a new folder occupies the card's
                // position instead of jumping ahead of empty sibling folders anchored there.
                order.iter().position(|name| {
                    let boundary = subfolder_boundary(s, destination_group, destination_path, name);
                    // A root group being placed before a card should precede folders already
                    // anchored to that card. Moving an existing folder to the card position
                    // retains those folders ahead of it, matching create-folder placement.
                    if f.is_empty() { boundary >= anchor_index } else { boundary > anchor_index }
                })
            })
            .unwrap_or(order.len());
        order.insert(at, leaf);
        reorder_folder_metadata(s, destination_group, destination_path, &order);
    }
    if dest.is_none() && !s.tools.preset_hierarchy.groups.contains(&new_group) {
        s.tools.preset_hierarchy.groups.push(new_group.clone());
    }
    if f.is_empty() && g != new_group {
        s.tools.preset_hierarchy.groups.retain(|name| name != &g);
        if !new_group.is_empty() && !s.tools.preset_hierarchy.groups.contains(&new_group) {
            s.tools.preset_hierarchy.groups.push(new_group.clone());
        }
    }
    if f.is_empty() {
        root_order.retain(|entry| *entry != crate::BrushPresetRootEntry::Group(g.clone()));
    }
    if dest.is_none() {
        let insert_at = root_index
            .or_else(|| before.as_ref().and_then(|name| root_order.iter().position(|entry| *entry == crate::BrushPresetRootEntry::Group(name.clone()))))
            .or_else(|| before_item.as_ref().and_then(|name| root_order.iter().position(|entry| *entry == crate::BrushPresetRootEntry::Preset(name.clone()))));
        insert_root_entry(&mut root_order, crate::BrushPresetRootEntry::Group(new_group.clone()), insert_at);
    }
    set_root_order(s, root_order);
    s.brush_presets_changed();
    let folder = new_prefix.get(..new_prefix.len().saturating_sub(1)).unwrap_or_default();
    let at = match &dest {
        None => session_group_order(s).iter().position(|x| *x == new_group),
        Some(_) => session_subfolder_order(s, &new_group, folder).iter().position(|x| *x == leaf),
    };
    Ok(json!({ "group": new_group, "folder": new_prefix, "index": at.unwrap_or(0) }))
}

fn move_group(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.presets.moveGroup";
    ensure_group_order_metadata(s);
    let mut root_order = session_root_order(s);
    let (g, f) = target_folder(s, p, cmd)?;
    if let Some(dest) = into_param(s, p, cmd)? {
        return move_into(s, p, g, f, dest, cmd);
    }
    if !f.is_empty() {
        if p.get("rootIndex").is_some_and(|value| !value.is_null()) {
            return Err(bad(cmd, "`rootIndex` is only valid for root groups"));
        }
        if p.get("beforeItem").is_some_and(|value| !value.is_null()) {
            let parent = f[..f.len() - 1].to_vec();
            return move_into(s, p, g.clone(), f, Some((g, parent)), cmd);
        }
        return move_folder(s, p, g, f, cmd);
    }
    let before = group_param(p, "before", cmd)?;
    if let Some(b) = &before {
        group_block(s, b, cmd)?;
        if *b == g {
            return Err(bad(cmd, "`before` names the group being moved"));
        }
    }
    let index = match p.get("index") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().ok_or_else(|| bad(cmd, "`index` must be a non-negative integer"))?),
    };
    let root_index = match p.get("rootIndex") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().ok_or_else(|| bad(cmd, "`rootIndex` must be a non-negative integer"))? as usize),
    };
    if root_index.is_some() && (before.is_some() || index.is_some()) {
        return Err(bad(cmd, "`rootIndex` cannot be combined with `before` or `index`"));
    }
    let mut root_siblings = root_order.clone();
    root_siblings.retain(|entry| *entry != crate::BrushPresetRootEntry::Group(g.clone()));
    let current_order = session_group_order(s);
    let lib = std::mem::take(&mut s.tools.presets);
    let (block, mut rest): (Vec<BrushPreset>, Vec<BrushPreset>) = lib.into_iter().partition(|x| x.group == g);
    let order: Vec<String> = current_order.into_iter().filter(|name| name != &g).collect();
    let first = |name: &str, rest: &[BrushPreset]| rest.iter().position(|x| x.group == name).unwrap_or(rest.len());
    let root_flat_pos = root_index.and_then(|index| root_anchor_index(&root_siblings, index, &rest));
    let pos = match (&before, index) {
        (Some(b), _) => first(b, &rest),
        (None, Some(n)) => order.get(n.min(usize::MAX as u64) as usize).map_or(rest.len(), |name| first(name, &rest)),
        (None, None) => root_flat_pos.unwrap_or(rest.len()),
    };
    let tail = rest.split_off(pos.min(rest.len()));
    rest.extend(block);
    rest.extend(tail);
    s.tools.presets = rest;
    if s.tools.preset_hierarchy.groups.contains(&g) {
        let mut groups = s.tools.preset_hierarchy.groups.clone();
        groups.retain(|name| name != &g);
        let at = before
            .as_ref()
            .and_then(|name| groups.iter().position(|candidate| candidate == name))
            .or(index.map(|n| n.min(groups.len() as u64) as usize))
            .unwrap_or(groups.len());
        groups.insert(at, g.clone());
        s.tools.preset_hierarchy.groups = groups;
    }
    let root_at = root_index
        .or_else(|| before.as_ref().and_then(|name| root_order.iter().position(|entry| *entry == crate::BrushPresetRootEntry::Group(name.clone()))))
        .or_else(|| {
            index.map(|index| {
                root_order
                    .iter()
                    .enumerate()
                    .filter(|(_, entry)| matches!(entry, crate::BrushPresetRootEntry::Group(_)))
                    .nth(index as usize)
                    .map_or(root_order.len(), |(position, _)| position)
            })
        });
    insert_root_entry(&mut root_order, crate::BrushPresetRootEntry::Group(g.clone()), root_at);
    set_root_order(s, root_order);
    s.brush_presets_changed();
    let at = group_order(&s.tools.presets).iter().position(|x| *x == g).unwrap_or(0);
    Ok(json!({ "group": g, "index": at }))
}

fn rename_group(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.presets.renameGroup";
    ensure_group_order_metadata(s);
    let mut root_order = session_root_order(s);
    let (g, f) = target_folder(s, p, cmd)?;
    let new = text(p, "newName", cmd)?;
    if let Some((leaf, parent)) = f.split_last() {
        if new != *leaf {
            if session_subfolder_order(s, &g, parent).iter().any(|name| name != leaf && name.eq_ignore_ascii_case(&new)) {
                return Err(bad(cmd, format!("a folder named `{new}` already exists there")));
            }
            let depth = parent.len();
            for x in s.tools.presets.iter_mut().filter(|x| in_folder(x, &g, &f)) {
                if let Some(n) = x.folder.get_mut(depth) {
                    n.clone_from(&new);
                }
                x.builtin = false;
            }
            for folder in &mut s.tools.preset_hierarchy.folders {
                if folder.group == g && folder.path.starts_with(&f) {
                    if let Some(part) = folder.path.get_mut(depth) {
                        *part = new.clone();
                    }
                }
            }
            s.brush_presets_changed();
        }
        let mut renamed = parent.to_vec();
        renamed.push(new);
        return Ok(json!({ "group": g, "folder": renamed }));
    }
    if new != g && group_name_conflicts(s, &g, &new) {
        return Err(bad(cmd, format!("a group named `{new}` already exists")));
    }
    if new != g {
        for x in s.tools.presets.iter_mut().filter(|x| x.group == g) {
            x.group = new.clone();
            x.builtin = false;
        }
        for folder in &mut s.tools.preset_hierarchy.folders {
            if folder.group == g {
                folder.group = new.clone();
            }
        }
        if let Some(group) = s.tools.preset_hierarchy.groups.iter_mut().find(|name| **name == g) {
            *group = new.clone();
        }
        for entry in &mut root_order {
            if *entry == crate::BrushPresetRootEntry::Group(g.clone()) {
                *entry = crate::BrushPresetRootEntry::Group(new.clone());
            }
        }
        set_root_order(s, root_order);
        s.brush_presets_changed();
    }
    Ok(json!({ "group": new }))
}

fn delete_group(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.presets.deleteGroup";
    let (g, f) = target_folder(s, p, cmd)?;
    let mut root_order = session_root_order(s);
    let before = s.tools.presets.len();
    s.tools.presets.retain(|x| !in_folder(x, &g, &f));
    if f.is_empty() {
        s.tools.preset_hierarchy.groups.retain(|name| name != &g);
        s.tools.preset_hierarchy.folders.retain(|folder| folder.group != g);
        root_order.retain(|entry| *entry != crate::BrushPresetRootEntry::Group(g.clone()));
        if !s.tools.presets.iter().any(|preset| !preset.builtin) && s.tools.preset_hierarchy.groups.is_empty() {
            root_order.clear();
        }
        set_root_order(s, root_order);
    } else {
        s.tools.preset_hierarchy.folders.retain(|folder| !(folder.group == g && folder.path.starts_with(&f)));
    }
    if s.tools.current_preset.as_ref().is_some_and(|c| !s.tools.presets.iter().any(|x| x.name.eq_ignore_ascii_case(c))) {
        s.tools.current_preset = None;
        s.tools.brush_changed();
    }
    s.brush_presets_changed();
    Ok(json!({ "deleted": before - s.tools.presets.len(), "count": s.tools.presets.len() }))
}

fn create_folder(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.presets.createFolder";
    let group = group_param(p, "group", cmd)?.ok_or_else(|| bad(cmd, "missing `group`"))?;
    group_block(s, &group, cmd)?;
    let mut path = folder_param(p, cmd)?.ok_or_else(|| bad(cmd, "missing `folder`"))?;
    if path.is_empty() {
        return Err(bad(cmd, "`folder` must name a nested folder"));
    }
    let leaf = path.pop().ok_or_else(|| bad(cmd, "missing folder name"))?;
    if path.len() >= MAX_FOLDER_DEPTH {
        return Err(bad(cmd, format!("folders can be nested at most {MAX_FOLDER_DEPTH} deep")));
    }
    ensure_folder_order_metadata(s, &group);
    if !has_folder(s, &group, &path) {
        return Err(bad(cmd, format!("no parent folder `{}` in brush preset group `{group}`", path.join("/"))));
    }
    if session_subfolder_order(s, &group, &path).iter().any(|name| name.eq_ignore_ascii_case(&leaf)) {
        return Err(bad(cmd, format!("a folder named `{leaf}` already exists there")));
    }
    path.push(leaf);
    if !s.tools.preset_hierarchy.groups.contains(&group) {
        s.tools.preset_hierarchy.groups.push(group.clone());
    }
    for depth in 1..=path.len() {
        let folder = crate::BrushPresetFolder { group: group.clone(), path: path[..depth].to_vec(), before_item: None };
        if !s.tools.preset_hierarchy.folders.contains(&folder) {
            s.tools.preset_hierarchy.folders.push(folder);
        }
    }
    s.brush_presets_changed();
    Ok(json!({ "group": group, "folder": path }))
}

fn create_group(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.presets.createGroup";
    let group = text(p, "group", cmd)?;
    if group_name_conflicts(s, "", &group) {
        return Err(bad(cmd, format!("a group named `{group}` already exists")));
    }
    s.tools.preset_hierarchy.groups.push(group.clone());
    let mut root_order = session_root_order(s);
    insert_root_entry(&mut root_order, crate::BrushPresetRootEntry::Group(group.clone()), None);
    set_root_order(s, root_order);
    s.brush_presets_changed();
    Ok(json!({ "group": group }))
}

/// Apply a small, validated set of hierarchy operations as one engine command. The brush browser
/// uses this for create-folder-and-move-selection so a failed later move cannot leave a partially
/// created group or a subset of the selection moved. Only hierarchy mutations are accepted.
fn batch(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "brush.presets.batch";
    let operations = p.get("operations").and_then(Value::as_array).ok_or_else(|| bad(CMD, "`operations` must be an array"))?;
    if operations.is_empty() || operations.len() > 256 {
        return Err(bad(CMD, "`operations` must contain 1 to 256 operations"));
    }
    let original_tools = s.tools.clone();
    let mut results = Vec::with_capacity(operations.len());
    for (index, operation) in operations.iter().enumerate() {
        let id = operation.get("id").and_then(Value::as_str).ok_or_else(|| bad(CMD, format!("operation {index} is missing `id`")));
        let params = operation.get("params").filter(|value| value.is_object()).cloned().ok_or_else(|| bad(CMD, format!("operation {index} is missing object `params`")));
        let result = match (id, params) {
            (Ok("brush.presets.createGroup"), Ok(params)) => create_group(s, &params),
            (Ok("brush.presets.createFolder"), Ok(params)) => create_folder(s, &params),
            (Ok("brush.presets.move"), Ok(params)) => move_preset(s, &params),
            (Ok("brush.presets.moveGroup"), Ok(params)) => move_group(s, &params),
            (Ok(other), _) => Err(bad(CMD, format!("operation `{other}` is not allowed in a hierarchy batch"))),
            (Err(error), _) | (_, Err(error)) => Err(error),
        };
        match result {
            Ok(value) => results.push(value),
            Err(error) => {
                s.tools = original_tools;
                return Err(bad(CMD, format!("operation {index} failed; no changes were applied: {error}")));
            }
        }
    }
    Ok(json!({ "results": results }))
}

/// Restore factory brush presets that were previously removed from the user's library.
/// Existing presets with the same name (including edited factory presets) are left untouched.
fn append_default(s: &mut Session, _p: &Value) -> Result<Value> {
    let defaults = photocraft_paint::presets::builtin();
    let mut appended = 0usize;
    let mut hierarchy_changed = false;
    for preset in defaults {
        if !s.tools.presets.iter().any(|existing| existing.name.eq_ignore_ascii_case(&preset.name)) {
            s.tools.presets.push(preset);
            appended += 1;
        }
    }
    // Factory groups must use the same persistent hierarchy representation as user-created
    // groups. Otherwise deleting their final brush makes the group disappear, unlike a normal
    // empty folder. Persist their folder paths as well so future factory folders behave identically.
    for group in photocraft_paint::presets::GROUPS {
        if !s.tools.preset_hierarchy.groups.iter().any(|existing| existing == group) {
            s.tools.preset_hierarchy.groups.push(group.to_string());
            hierarchy_changed = true;
        }
        ensure_folder_order_metadata(s, group);
    }
    let mut root_order = session_root_order(s);
    // Factory brushes are presented as four groups. Treat the command as an append operation:
    // move those groups, in factory order, after every existing root item/group.
    for group in photocraft_paint::presets::GROUPS {
        root_order.retain(|entry| *entry != crate::BrushPresetRootEntry::Group(group.to_string()));
    }
    for group in photocraft_paint::presets::GROUPS {
        if has_group(s, group) {
            root_order.push(crate::BrushPresetRootEntry::Group(group.to_string()));
        }
    }
    let root_order_changed = root_order != s.tools.preset_hierarchy.root_order;
    if appended > 0 || root_order_changed || hierarchy_changed {
        set_root_order(s, root_order);
        s.brush_presets_changed();
    }
    Ok(json!({ "appended": appended, "count": s.tools.presets.len() }))
}

macro_rules! spec {
    ($id:literal, $label:literal, $params:literal, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[], shortcut: None, params: $params, enabled: always, run: $run, journal: true }
    };
}

/// Brush preset organisation specs.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!("brush.presets.appendDefault", "Append Default Brushes", r##"{} → {appended, count}"##, append_default),
        spec!("brush.presets.createGroup", "Create Brush Group", r##"{"group":string} → {group}"##, create_group),
        spec!("brush.presets.createFolder", "Create Brush Folder", r##"{"group":string,"folder":[name] (full nested path)} → {group, folder}"##, create_folder),
        spec!("brush.presets.batch", "Brush Hierarchy Transaction", r##"{"operations":[{ "id":brush.presets.createGroup|createFolder|move|moveGroup, "params":object }]} → {results}"##, batch),
        spec!("brush.presets.rename", "Rename Brush", r##"{"name":string,"newName":string} → {name}"##, rename),
        spec!(
            "brush.presets.move",
            "Move Brush",
            r##"{"name":string,"group":string?=its group ("" = ungrouped),"folder":[name]? (nested folder names in the group, outermost first; [] = the group itself)=before's folder, else its own if it stays in its group, else [],"before":name? (preset to land before, in that folder) | "index":n? (position in the group)=end of the folder, "rootIndex":n? (mixed root sibling position when moving to the root)} → {name, group, folder, index}"##,
            move_preset
        ),
        spec!(
            "brush.presets.moveGroup",
            "Move Brush Group",
            r##"{"group":string,"folder":[name]? (move this nested folder instead),"into":{"group":string?,"folder":[name]?}? (move it inside that folder of that group, [] = directly in the group; no group = out to the top level as a group of its own; the folder keeps its content),"before":group? (or sibling folder name), "beforeItem":preset? (direct item in the destination folder; mutually exclusive with `before`) | "index":n? (group or sibling folder position, without `into`)=end, "rootIndex":n? (mixed root sibling position when moving to the root)} → {group, folder?, index}"##,
            move_group
        ),
        spec!(
            "brush.presets.renameGroup",
            "Rename Brush Group",
            r##"{"group":string,"folder":[name]? (rename this nested folder instead),"newName":string} → {group, folder?}"##,
            rename_group
        ),
        spec!(
            "brush.presets.deleteGroup",
            "Delete Brush Group",
            r##"{"group":string,"folder":[name]? (delete this nested folder instead)} → {deleted, count}"##,
            delete_group
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(s: &Session, g: &str) -> Vec<String> {
        s.tools.presets.iter().filter(|x| x.group == g).map(|x| x.name.clone()).collect()
    }

    fn with_groups() -> Session {
        let mut s = Session::new();
        for (n, g) in [("A1", "Alpha"), ("A2", "Alpha"), ("A3", "Alpha"), ("B1", "Beta"), ("B2", "Beta")] {
            s.tools.presets.push(BrushPreset {
            id: uuid::Uuid::new_v4().to_string(),
                name: n.into(),
                brush: Default::default(),
                builtin: false,
                group: g.into(),
                folder: Vec::new(),
                captures: Default::default(),
            });
        }
        s
    }

    #[test]
    fn hierarchy_batch_rolls_back_every_prior_operation_on_failure() {
        let mut s = with_groups();
        let presets_before = s.tools.presets.clone();
        let hierarchy_before = s.tools.preset_hierarchy.clone();
        let revision_before = s.tools.presets_rev;

        let result = s.execute(
            "brush.presets.batch",
            json!({
                "operations": [
                    {"id":"brush.presets.move", "params":{"name":"A1", "group":"Beta"}},
                    {"id":"brush.presets.moveGroup", "params":{"group":"Missing group", "folder":[]}}
                ]
            }),
        );

        assert!(result.is_err());
        assert_eq!(s.tools.presets, presets_before);
        assert_eq!(s.tools.preset_hierarchy, hierarchy_before);
        assert_eq!(s.tools.presets_rev, revision_before);
    }

    #[test]
    fn append_default_restores_missing_factory_presets_without_overwriting_existing_edits() {
        let mut s = Session::new();
        let defaults = photocraft_paint::presets::builtin();
        let missing = defaults.last().unwrap().name.clone();
        let edited = defaults.first().unwrap().name.clone();
        let edited_preset = s.tools.presets.iter_mut().find(|preset| preset.name == edited).unwrap();
        edited_preset.builtin = false;
        edited_preset.brush.size = 123.0;
        s.tools.presets.retain(|preset| preset.name != missing);

        let result = s.execute("brush.presets.appendDefault", json!({})).unwrap();

        assert_eq!(result["appended"], 1);
        assert!(s.tools.presets.iter().any(|preset| preset.name == missing && preset.builtin));
        let edited_preset = s.tools.presets.iter().find(|preset| preset.name == edited).unwrap();
        assert!(!edited_preset.builtin);
        assert_eq!(edited_preset.brush.size, 123.0);
    }

    #[test]
    fn append_default_is_idempotent_when_all_factory_presets_are_present() {
        let mut s = Session::new();
        let before = s.tools.presets.len();

        let result = s.execute("brush.presets.appendDefault", json!({})).unwrap();

        assert_eq!(result["appended"], 0);
        assert_eq!(s.tools.presets.len(), before);
    }

    #[test]
    fn append_default_moves_factory_groups_to_the_end_of_the_root() {
        let mut s = Session::new();
        s.tools.preset_hierarchy.groups.push("User Group".into());
        s.tools.preset_hierarchy.root_order = vec![
            crate::BrushPresetRootEntry::Group("General".into()),
            crate::BrushPresetRootEntry::Group("User Group".into()),
            crate::BrushPresetRootEntry::Group("Dry Media".into()),
        ];

        s.execute("brush.presets.appendDefault", json!({})).unwrap();

        assert_eq!(
            s.tools.preset_hierarchy.root_order,
            [
                crate::BrushPresetRootEntry::Group("User Group".into()),
                crate::BrushPresetRootEntry::Group("General".into()),
                crate::BrushPresetRootEntry::Group("Dry Media".into()),
                crate::BrushPresetRootEntry::Group("Wet Media".into()),
                crate::BrushPresetRootEntry::Group("Special Effects".into()),
            ]
        );
    }

    #[test]
    fn empty_groups_and_nested_folders_can_be_created_renamed_moved_and_deleted() {
        let mut s = Session::new();
        s.execute("brush.presets.createGroup", json!({"group": "Empty"})).unwrap();
        s.execute("brush.presets.createFolder", json!({"group": "Empty", "folder": ["Parent"]})).unwrap();
        s.execute("brush.presets.createFolder", json!({"group": "Empty", "folder": ["Parent", "Child"]})).unwrap();
        assert_eq!(session_group_order(&s).last().map(String::as_str), Some("Empty"));
        assert_eq!(session_subfolder_order(&s, "Empty", &[]), ["Parent"]);
        assert_eq!(session_subfolder_order(&s, "Empty", &["Parent".into()]), ["Child"]);

        s.execute("brush.presets.renameGroup", json!({"group": "Empty", "folder": ["Parent"], "newName": "Renamed"})).unwrap();
        assert!(has_folder(&s, "Empty", &["Renamed".into(), "Child".into()]));
        s.execute("brush.presets.createGroup", json!({"group": "Other Empty"})).unwrap();
        s.execute("brush.presets.moveGroup", json!({"group": "Other Empty", "into": {"group": "Empty", "folder": ["Renamed"]}})).unwrap();
        assert!(has_folder(&s, "Empty", &["Renamed".into(), "Other Empty".into()]));
        assert!(!has_group(&s, "Other Empty"));
        s.execute("brush.presets.moveGroup", json!({"group": "Empty", "folder": ["Renamed"], "into": {"group": "Empty"}})).unwrap();
        assert!(has_folder(&s, "Empty", &["Renamed".into()]));
        s.execute("brush.presets.deleteGroup", json!({"group": "Empty", "folder": ["Renamed"]})).unwrap();
        assert!(s.tools.preset_hierarchy.folders.is_empty());
        s.execute("brush.presets.renameGroup", json!({"group": "Empty", "newName": "Still Empty"})).unwrap();
        assert!(has_group(&s, "Still Empty"));
        s.execute("brush.presets.deleteGroup", json!({"group": "Still Empty"})).unwrap();
        assert!(s.tools.preset_hierarchy.is_empty());
    }

    #[test]
    fn empty_folders_keep_order_relative_to_populated_folders() {
        let mut s = with_groups();
        s.tools.presets.iter_mut().find(|preset| preset.name == "A1").unwrap().folder = vec!["Filled".into()];
        s.execute("brush.presets.createFolder", json!({"group": "Alpha", "folder": ["Empty"]})).unwrap();
        assert_eq!(session_subfolder_order(&s, "Alpha", &[]), ["Filled", "Empty"]);
        s.execute("brush.presets.moveGroup", json!({"group": "Alpha", "folder": ["Empty"], "index": 0})).unwrap();
        assert_eq!(session_subfolder_order(&s, "Alpha", &[]), ["Empty", "Filled"]);
        s.execute("brush.presets.moveGroup", json!({"group": "Alpha", "folder": ["Empty"], "index": 1})).unwrap();
        assert_eq!(session_subfolder_order(&s, "Alpha", &[]), ["Filled", "Empty"]);
    }

    #[test]
    fn move_within_and_between_groups() {
        let mut s = with_groups();
        let r = s.execute("brush.presets.move", json!({"name": "A3", "before": "A1"})).unwrap();
        assert_eq!(r, json!({"name": "A3", "group": "Alpha", "folder": [], "index": 0}));
        assert_eq!(names(&s, "Alpha"), ["A3", "A1", "A2"]);
        s.execute("brush.presets.move", json!({"name": "A3", "index": 99})).unwrap();
        assert_eq!(names(&s, "Alpha"), ["A1", "A2", "A3"]);
        s.execute("brush.presets.move", json!({"name": "A1", "index": 1})).unwrap();
        assert_eq!(names(&s, "Alpha"), ["A2", "A1", "A3"]);
        // Into another group, before a member (the group comes from `before`).
        s.execute("brush.presets.move", json!({"name": "A1", "before": "B2"})).unwrap();
        assert_eq!((names(&s, "Alpha"), names(&s, "Beta")), (vec!["A2".to_string(), "A3".into()], vec!["B1".to_string(), "A1".into(), "B2".into()]));
        // Into a group by name (appended), and into a new group.
        s.execute("brush.presets.move", json!({"name": "A2", "group": "Beta"})).unwrap();
        assert_eq!(names(&s, "Beta"), ["B1", "A1", "B2", "A2"]);
        s.execute("brush.presets.move", json!({"name": "A3", "group": "Gamma"})).unwrap();
        assert_eq!(group_order(&s.tools.presets).last().map(String::as_str), Some("Gamma"));
        assert!(names(&s, "Alpha").is_empty());
        // A built-in moved to another group becomes the user's.
        let b = s.tools.presets.iter().find(|x| x.builtin).unwrap().name.clone();
        s.execute("brush.presets.move", json!({"name": b, "group": "Beta", "index": 0})).unwrap();
        let moved = photocraft_paint::presets::find(&s.tools.presets, &b).unwrap();
        assert!(!moved.builtin && moved.group == "Beta");
        assert_eq!(names(&s, "Beta")[0], b);
    }

    #[test]
    fn groups_move_rename_and_delete() {
        let mut s = with_groups();
        let order = group_order(&s.tools.presets);
        let n = order.len();
        assert_eq!(&order[n - 2..], ["Alpha", "Beta"]);
        let r = s.execute("brush.presets.moveGroup", json!({"group": "Beta", "index": 0})).unwrap();
        assert_eq!(r["index"], 0);
        assert_eq!(group_order(&s.tools.presets)[0], "Beta");
        assert_eq!(names(&s, "Beta"), ["B1", "B2"], "a group moves as a block, in order");
        s.execute("brush.presets.moveGroup", json!({"group": "Alpha", "before": "Beta"})).unwrap();
        assert_eq!(&group_order(&s.tools.presets)[..2], ["Alpha", "Beta"]);
        s.execute("brush.presets.moveGroup", json!({"group": "Alpha"})).unwrap();
        assert_eq!(group_order(&s.tools.presets).last().map(String::as_str), Some("Alpha"));
        s.execute("brush.presets.renameGroup", json!({"group": "Alpha", "newName": "First"})).unwrap();
        assert_eq!(names(&s, "First"), ["A1", "A2", "A3"]);
        assert!(s.execute("brush.presets.renameGroup", json!({"group": "First", "newName": "Beta"})).is_err());
        let r = s.execute("brush.presets.deleteGroup", json!({"group": "First"})).unwrap();
        assert_eq!(r["deleted"], 3);
        assert!(names(&s, "First").is_empty());
    }

    #[test]
    fn rename_checks_names() {
        let mut s = with_groups();
        s.execute("brush.presets.rename", json!({"name": "a1", "newName": "Alpha One"})).unwrap();
        assert!(photocraft_paint::presets::find(&s.tools.presets, "Alpha One").is_some());
        assert!(s.execute("brush.presets.rename", json!({"name": "A2", "newName": "alpha one"})).is_err());
        // Case-only renames of itself are fine.
        s.execute("brush.presets.rename", json!({"name": "Alpha One", "newName": "ALPHA ONE"})).unwrap();
        let b = s.tools.presets.iter().find(|x| x.builtin).unwrap().name.clone();
        s.execute("brush.presets.rename", json!({"name": b, "newName": "My Round"})).unwrap();
        assert!(!photocraft_paint::presets::find(&s.tools.presets, "My Round").unwrap().builtin);
    }

    #[test]
    fn bad_params_fail_without_changes() {
        let mut s = with_groups();
        let before = s.tools.presets.clone();
        let long = "x".repeat(MAX_NAME + 1);
        for (id, p) in [
            ("brush.presets.move", json!({})),
            ("brush.presets.move", json!({"name": "Nope"})),
            ("brush.presets.move", json!({"name": "A1", "before": "A1"})),
            ("brush.presets.move", json!({"name": "A1", "before": "Nope"})),
            ("brush.presets.move", json!({"name": "A1", "group": "Alpha", "before": "B1"})),
            ("brush.presets.move", json!({"name": "A1", "index": -1})),
            ("brush.presets.move", json!({"name": "A1", "index": "first"})),
            ("brush.presets.move", json!({"name": "A1", "group": 7})),
            ("brush.presets.move", json!({"name": "A1", "group": long})),
            ("brush.presets.rename", json!({"name": "A1"})),
            ("brush.presets.rename", json!({"name": "A1", "newName": "  "})),
            ("brush.presets.rename", json!({"name": "A1", "newName": long})),
            ("brush.presets.moveGroup", json!({"group": "Nope"})),
            ("brush.presets.moveGroup", json!({"group": "Alpha", "before": "Alpha"})),
            ("brush.presets.moveGroup", json!({"group": "Alpha", "index": 1.5})),
            ("brush.presets.renameGroup", json!({"group": "Alpha"})),
            ("brush.presets.deleteGroup", json!({"group": null})),
            ("brush.presets.deleteGroup", json!({"group": "Nope"})),
        ] {
            assert!(s.execute(id, p.clone()).is_err(), "{id} {p}");
        }
        assert_eq!(s.tools.presets, before);
        // Huge indices clamp to the end.
        s.execute("brush.presets.move", json!({"name": "A1", "index": u64::MAX})).unwrap();
        s.execute("brush.presets.moveGroup", json!({"group": "Alpha", "index": u64::MAX})).unwrap();
    }

    /// Group "Set": S0 at its top level, folders Ink (I1, I2; subfolder Fine: F1) and Dry (D1).
    fn with_folders() -> Session {
        let mut s = Session::new();
        for (n, f) in [("S0", &[][..]), ("I1", &["Ink"][..]), ("F1", &["Ink", "Fine"][..]), ("I2", &["Ink"][..]), ("D1", &["Dry"][..])] {
            let folder = f.iter().map(|x| x.to_string()).collect();
            s.tools.presets.push(BrushPreset {
            id: uuid::Uuid::new_v4().to_string(),
                name: n.into(),
                brush: Default::default(),
                builtin: false,
                group: "Set".into(),
                folder,
                captures: Default::default(),
            });
        }
        s
    }

    fn folder_of(s: &Session, n: &str) -> Vec<String> {
        photocraft_paint::presets::find(&s.tools.presets, n).unwrap().folder.clone()
    }

    #[test]
    fn presets_move_between_folders() {
        let mut s = with_folders();
        // `before` brings its folder.
        let r = s.execute("brush.presets.move", json!({"name": "D1", "before": "I2"})).unwrap();
        assert_eq!(r["folder"], json!(["Ink"]));
        assert_eq!(folder_of(&s, "D1"), ["Ink"]);
        // An explicit folder, appended at the end of it.
        s.execute("brush.presets.move", json!({"name": "S0", "folder": ["Ink", "Fine"]})).unwrap();
        assert_eq!(folder_of(&s, "S0"), ["Ink", "Fine"]);
        let pos = |n: &str| s.tools.presets.iter().position(|x| x.name == n).unwrap();
        assert_eq!(pos("S0"), pos("F1") + 1);
        // Within its group without a folder keeps its folder; to another group without one lands
        // at that group's top level; `folder: []` is the group's top level.
        s.execute("brush.presets.move", json!({"name": "S0", "index": 0})).unwrap();
        assert_eq!(folder_of(&s, "S0"), ["Ink", "Fine"]);
        s.execute("brush.presets.move", json!({"name": "S0", "group": "Other"})).unwrap();
        assert!(folder_of(&s, "S0").is_empty());
        s.execute("brush.presets.move", json!({"name": "I1", "folder": []})).unwrap();
        assert!(folder_of(&s, "I1").is_empty());
        // `before` in another folder than the one named.
        assert!(s.execute("brush.presets.move", json!({"name": "I1", "folder": ["Dry"], "before": "I2"})).is_err());
        assert_eq!(subfolder_order(&s.tools.presets, "Set", &[]), ["Ink"]);
    }

    #[test]
    fn folders_move_rename_and_delete() {
        let mut s = with_folders();
        assert_eq!(subfolder_order(&s.tools.presets, "Set", &[]), ["Ink", "Dry"]);
        // Move Dry before Ink: the whole folder moves as a block.
        let r = s.execute("brush.presets.moveGroup", json!({"group": "Set", "folder": ["Dry"], "before": "Ink"})).unwrap();
        assert_eq!(r, json!({"group": "Set", "folder": ["Dry"], "index": 0}));
        assert_eq!(subfolder_order(&s.tools.presets, "Set", &[]), ["Dry", "Ink"]);
        s.execute("brush.presets.moveGroup", json!({"group": "Set", "folder": ["Dry"]})).unwrap();
        assert_eq!(subfolder_order(&s.tools.presets, "Set", &[]), ["Ink", "Dry"]);
        s.execute("brush.presets.moveGroup", json!({"group": "Set", "folder": ["Dry"], "index": 0})).unwrap();
        assert_eq!(subfolder_order(&s.tools.presets, "Set", &[]), ["Dry", "Ink"]);
        let ink: Vec<&str> = s.tools.presets.iter().filter(|x| x.folder.first().is_some_and(|f| f == "Ink")).map(|x| x.name.as_str()).collect();
        assert_eq!(ink, ["I1", "F1", "I2"], "presets keep their order inside a moved folder");
        // Rename a nested folder: every preset below it follows.
        let r = s.execute("brush.presets.renameGroup", json!({"group": "Set", "folder": ["Ink"], "newName": "Inks"})).unwrap();
        assert_eq!(r["folder"], json!(["Inks"]));
        assert_eq!(folder_of(&s, "F1"), ["Inks", "Fine"]);
        assert!(s.execute("brush.presets.renameGroup", json!({"group": "Set", "folder": ["Inks"], "newName": "Dry"})).is_err());
        // Renaming the group keeps the folders.
        s.execute("brush.presets.renameGroup", json!({"group": "Set", "newName": "Kit"})).unwrap();
        assert_eq!(folder_of(&s, "F1"), ["Inks", "Fine"]);
        // Delete a nested folder: only the presets in it (and below) go.
        let r = s.execute("brush.presets.deleteGroup", json!({"group": "Kit", "folder": ["Inks"]})).unwrap();
        assert_eq!(r["deleted"], 3);
        assert!(photocraft_paint::presets::find(&s.tools.presets, "S0").is_some());
        assert!(photocraft_paint::presets::find(&s.tools.presets, "D1").is_some());
    }

    #[test]
    fn bad_folder_params_fail_without_changes() {
        let mut s = with_folders();
        let before = s.tools.presets.clone();
        let long = "x".repeat(MAX_NAME + 1);
        let deep: Vec<&str> = vec!["a"; MAX_FOLDER_DEPTH + 1];
        for (id, p) in [
            ("brush.presets.move", json!({"name": "S0", "folder": "Ink"})),
            ("brush.presets.move", json!({"name": "S0", "folder": [""]})),
            ("brush.presets.move", json!({"name": "S0", "folder": [7]})),
            ("brush.presets.move", json!({"name": "S0", "folder": [long]})),
            ("brush.presets.move", json!({"name": "S0", "folder": deep})),
            ("brush.presets.moveGroup", json!({"group": "Set", "folder": ["Nope"]})),
            ("brush.presets.moveGroup", json!({"group": "Set", "folder": ["Ink"], "before": "Ink"})),
            ("brush.presets.moveGroup", json!({"group": "Set", "folder": ["Ink"], "before": "Nope"})),
            ("brush.presets.moveGroup", json!({"group": "Set", "folder": ["Ink"], "before": 3})),
            ("brush.presets.moveGroup", json!({"group": "Set", "folder": ["Ink"], "index": -2})),
            ("brush.presets.renameGroup", json!({"group": "Set", "folder": ["Ink"], "newName": ""})),
            ("brush.presets.renameGroup", json!({"group": "Set", "folder": ["Ink", "Nope"], "newName": "x"})),
            ("brush.presets.deleteGroup", json!({"group": "Set", "folder": ["Dry", "Nope"]})),
            ("brush.presets.deleteGroup", json!({"group": "Nope", "folder": ["Dry"]})),
        ] {
            assert!(s.execute(id, p.clone()).is_err(), "{id} {p}");
        }
        assert_eq!(s.tools.presets, before);
        s.execute("brush.presets.moveGroup", json!({"group": "Set", "folder": ["Ink", "Fine"], "index": u64::MAX})).unwrap();
    }

    fn paths(s: &Session) -> Vec<String> {
        s.tools.presets.iter().filter(|x| !x.builtin).map(|x| format!("{}:{}:{}", x.group, x.folder.join("/"), x.name)).collect()
    }

    /// `with_folders` plus group "Other" holding O1, and folder Box (B1) in it.
    fn with_two_groups() -> Session {
        let mut s = with_folders();
        for (n, f) in [("O1", &[][..]), ("B1", &["Box"][..])] {
            let folder = f.iter().map(|x| x.to_string()).collect();
            s.tools.presets.push(BrushPreset {
            id: uuid::Uuid::new_v4().to_string(),
                name: n.into(),
                brush: Default::default(),
                builtin: false,
                group: "Other".into(),
                folder,
                captures: Default::default(),
            });
        }
        s
    }

    #[test]
    fn folders_move_into_folders_and_groups() {
        let mut s = with_two_groups();
        // A nested folder into a sibling folder: its subtree follows, order kept.
        let r = s.execute("brush.presets.moveGroup", json!({"group": "Set", "folder": ["Dry"], "into": {"group": "Set", "folder": ["Ink", "Fine"]}})).unwrap();
        assert_eq!(r, json!({"group": "Set", "folder": ["Ink", "Fine", "Dry"], "index": 0}));
        assert_eq!(folder_of(&s, "D1"), ["Ink", "Fine", "Dry"]);
        // Into another group, before one of its folders.
        let r = s.execute("brush.presets.moveGroup", json!({"group": "Set", "folder": ["Ink"], "into": {"group": "Other"}, "before": "Box"})).unwrap();
        assert_eq!(r["folder"], json!(["Ink"]));
        assert_eq!(paths(&s), ["Set::S0", "Other::O1", "Other:Ink:I1", "Other:Ink/Fine:F1", "Other:Ink/Fine/Dry:D1", "Other:Ink:I2", "Other:Box:B1"]);
        // Out to the top level: the folder becomes a group of its own.
        let r = s.execute("brush.presets.moveGroup", json!({"group": "Other", "folder": ["Ink", "Fine"], "into": {}})).unwrap();
        assert_eq!(r["group"], "Fine");
        assert_eq!(folder_of(&s, "D1"), ["Dry"]);
        assert_eq!(photocraft_paint::presets::find(&s.tools.presets, "F1").unwrap().group, "Fine");
        // A whole group into another group's folder.
        s.execute("brush.presets.moveGroup", json!({"group": "Fine", "into": {"group": "Set"}})).unwrap();
        assert!(names(&s, "Fine").is_empty());
        assert_eq!(folder_of(&s, "D1"), ["Fine", "Dry"]);
        assert_eq!(subfolder_order(&s.tools.presets, "Set", &[]), ["Fine"]);
    }

    #[test]
    fn folder_drop_before_a_direct_preset_preserves_the_interleaved_library_order() {
        let mut s = Session::new();
        for (name, folder) in [("A", &[][..]), ("B", &[][..]), ("C", &[][..]), ("Child item", &["Child"][..])] {
            s.tools.presets.push(BrushPreset {
            id: uuid::Uuid::new_v4().to_string(),
                name: name.into(),
                brush: Default::default(),
                builtin: false,
                group: "Set".into(),
                folder: folder.iter().map(|part| (*part).to_string()).collect(),
                captures: Default::default(),
            });
        }

        s.execute("brush.presets.moveGroup", json!({"group": "Set", "folder": ["Child"], "beforeItem": "B"})).unwrap();

        let order: Vec<String> = s
            .tools
            .presets
            .iter()
            .filter(|preset| !preset.builtin)
            .map(|preset| if preset.folder.is_empty() { preset.name.clone() } else { format!("{}: {}", preset.folder.join("/"), preset.name) })
            .collect();
        assert_eq!(order, ["A", "Child: Child item", "B", "C"]);
        assert_eq!(folder_of(&s, "Child item"), ["Child"]);
    }

    #[test]
    fn moving_an_empty_folder_between_cards_persists_its_card_anchor() {
        let mut s = Session::new();
        s.tools.presets.clear();
        s.tools.preset_hierarchy = crate::BrushPresetHierarchy {
            groups: vec!["Parent".into(), "Empty".into()],
            folders: Vec::new(),
            root_order: vec![crate::BrushPresetRootEntry::Group("Parent".into()), crate::BrushPresetRootEntry::Group("Empty".into())],
        };
        for name in ["A", "B"] {
            s.tools.presets.push(BrushPreset {
            id: uuid::Uuid::new_v4().to_string(),
                name: name.into(),
                brush: Default::default(),
                builtin: false,
                group: "Parent".into(),
                folder: Vec::new(),
                captures: Default::default(),
            });
        }

        s.execute("brush.presets.moveGroup", json!({"group": "Empty", "into": {"group": "Parent"}, "beforeItem": "B"})).unwrap();

        let folder = s.tools.preset_hierarchy.folders.iter().find(|folder| folder.group == "Parent" && folder.path == ["Empty"]).unwrap();
        assert_eq!(folder.before_item.as_deref(), Some("B"));
        assert_eq!(s.tools.preset_hierarchy.groups, ["Parent"]);
    }

    #[test]
    fn moving_a_card_that_anchors_empty_folders_retargets_them_to_the_next_card() {
        let mut s = Session::new();
        s.tools.presets.clear();
        for name in ["A", "B", "C"] {
            s.tools.presets.push(BrushPreset {
            id: uuid::Uuid::new_v4().to_string(),
                name: name.into(),
                brush: Default::default(),
                builtin: false,
                group: "Parent".into(),
                folder: Vec::new(),
                captures: Default::default(),
            });
        }
        s.tools.preset_hierarchy = crate::BrushPresetHierarchy {
            groups: vec!["Parent".into()],
            folders: vec![crate::BrushPresetFolder { group: "Parent".into(), path: vec!["Empty".into()], before_item: Some("B".into()) }],
            root_order: vec![crate::BrushPresetRootEntry::Group("Parent".into())],
        };
        s.execute("brush.presets.createFolder", json!({"group":"Parent", "folder":["New"]})).unwrap();
        s.execute("brush.presets.moveGroup", json!({"group":"Parent", "folder":["New"], "into":{"group":"Parent"}, "beforeItem":"B"})).unwrap();
        assert_eq!(
            s.tools.preset_hierarchy.folders.iter().map(|folder| folder.path[0].as_str()).collect::<Vec<_>>(),
            ["Empty", "New"],
            "new folder should occupy selected card's position after existing folders already before it"
        );

        s.execute("brush.presets.move", json!({"name":"B", "group":"Parent", "folder":["New"]})).unwrap();

        assert_eq!(
            s.tools.preset_hierarchy.folders.iter().filter(|folder| folder.group == "Parent").map(|folder| folder.before_item.as_deref()).collect::<Vec<_>>(),
            [Some("C"), Some("C")]
        );
        assert_eq!(names(&s, "Parent"), ["A", "C", "B"]);
    }

    #[test]
    fn moving_a_folder_before_a_card_inserts_it_before_other_folders_anchored_there() {
        let mut s = Session::new();
        s.tools.presets.clear();
        s.tools.preset_hierarchy = crate::BrushPresetHierarchy {
            groups: vec!["Parent".into(), "First".into(), "Second".into()],
            folders: vec![crate::BrushPresetFolder { group: "Parent".into(), path: vec!["Second".into()], before_item: Some("B".into()) }],
            root_order: Vec::new(),
        };
        for name in ["A", "B", "C"] {
            s.tools.presets.push(BrushPreset {
            id: uuid::Uuid::new_v4().to_string(),
                name: name.into(),
                brush: Default::default(),
                builtin: false,
                group: "Parent".into(),
                folder: Vec::new(),
                captures: Default::default(),
            });
        }

        s.execute("brush.presets.moveGroup", json!({"group": "First", "into": {"group": "Parent"}, "beforeItem": "B"})).unwrap();

        assert_eq!(
            s.tools.preset_hierarchy.folders.iter().filter(|folder| folder.group == "Parent").map(|folder| folder.path[0].as_str()).collect::<Vec<_>>(),
            ["First", "Second"]
        );
        assert_eq!(
            s.tools.preset_hierarchy.folders.iter().find(|folder| folder.group == "Parent" && folder.path == ["First"]).unwrap().before_item.as_deref(),
            Some("B")
        );
    }

    #[test]
    fn moving_an_empty_folder_before_another_empty_folder_reorders_folder_metadata() {
        let mut s = Session::new();
        s.tools.presets.clear();
        s.tools.preset_hierarchy = crate::BrushPresetHierarchy {
            groups: vec!["Parent".into(), "First".into(), "Second".into()],
            folders: vec![crate::BrushPresetFolder { group: "Parent".into(), path: vec!["Second".into()], before_item: Some("Anchor".into()) }],
            root_order: Vec::new(),
        };
        s.tools.presets.push(BrushPreset {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Anchor".into(),
            brush: Default::default(),
            builtin: false,
            group: "Parent".into(),
            folder: Vec::new(),
            captures: Default::default(),
        });

        s.execute("brush.presets.moveGroup", json!({"group": "First", "into": {"group": "Parent"}, "before": "Second"})).unwrap();

        assert_eq!(
            s.tools.preset_hierarchy.folders.iter().filter(|folder| folder.group == "Parent").map(|folder| folder.path[0].as_str()).collect::<Vec<_>>(),
            ["First", "Second"]
        );
        assert_eq!(
            s.tools.preset_hierarchy.folders.iter().find(|folder| folder.group == "Parent" && folder.path == ["First"]).unwrap().before_item.as_deref(),
            Some("Anchor")
        );
    }

    #[test]
    fn root_group_can_be_moved_before_an_ungrouped_root_preset() {
        let mut s = Session::new();
        s.tools.presets.clear();
        s.tools.preset_hierarchy = Default::default();
        for (name, group) in [("Root A", ""), ("Root B", ""), ("Grouped A", "Set"), ("Grouped B", "Set"), ("Root C", "")] {
            s.tools.presets.push(BrushPreset {
            id: uuid::Uuid::new_v4().to_string(),
                name: name.into(),
                brush: Default::default(),
                builtin: false,
                group: group.into(),
                folder: Vec::new(),
                captures: Default::default(),
            });
        }

        s.execute("brush.presets.moveGroup", json!({"group": "Set", "into": {}, "beforeItem": "Root B"})).unwrap();

        let order: Vec<_> = s.tools.presets.iter().map(|preset| preset.name.as_str()).collect();
        assert_eq!(order, ["Root A", "Grouped A", "Grouped B", "Root B", "Root C"]);
        assert_eq!(s.tools.presets.iter().find(|preset| preset.name == "Grouped A").unwrap().group, "Set");
        assert_eq!(
            s.tools.preset_hierarchy.root_order,
            [
                crate::BrushPresetRootEntry::Preset("Root A".into()),
                crate::BrushPresetRootEntry::Group("Set".into()),
                crate::BrushPresetRootEntry::Preset("Root B".into()),
                crate::BrushPresetRootEntry::Preset("Root C".into()),
            ]
        );
    }

    #[test]
    fn empty_root_group_can_be_inserted_between_root_cards_and_at_the_start() {
        let mut s = Session::new();
        s.tools.presets.clear();
        s.tools.preset_hierarchy = crate::BrushPresetHierarchy {
            groups: vec!["Empty".into()],
            folders: Vec::new(),
            root_order: vec![
                crate::BrushPresetRootEntry::Preset("Root A".into()),
                crate::BrushPresetRootEntry::Preset("Root B".into()),
                crate::BrushPresetRootEntry::Group("Empty".into()),
            ],
        };
        for name in ["Root A", "Root B"] {
            s.tools.presets.push(BrushPreset {
            id: uuid::Uuid::new_v4().to_string(),
                name: name.into(),
                brush: Default::default(),
                builtin: false,
                group: String::new(),
                folder: Vec::new(),
                captures: Default::default(),
            });
        }

        s.execute("brush.presets.moveGroup", json!({"group": "Empty", "rootIndex": 1})).unwrap();
        assert_eq!(
            s.tools.preset_hierarchy.root_order,
            [
                crate::BrushPresetRootEntry::Preset("Root A".into()),
                crate::BrushPresetRootEntry::Group("Empty".into()),
                crate::BrushPresetRootEntry::Preset("Root B".into()),
            ]
        );

        s.execute("brush.presets.moveGroup", json!({"group": "Empty", "rootIndex": 0})).unwrap();
        assert_eq!(
            s.tools.preset_hierarchy.root_order,
            [
                crate::BrushPresetRootEntry::Group("Empty".into()),
                crate::BrushPresetRootEntry::Preset("Root A".into()),
                crate::BrushPresetRootEntry::Preset("Root B".into()),
            ]
        );
    }

    #[test]
    fn root_presets_can_be_moved_to_the_absolute_start_and_end() {
        let mut s = Session::new();
        s.tools.presets.clear();
        s.tools.preset_hierarchy = Default::default();
        for name in ["Root A", "Root B", "Root C"] {
            s.tools.presets.push(BrushPreset {
            id: uuid::Uuid::new_v4().to_string(),
                name: name.into(),
                brush: Default::default(),
                builtin: false,
                group: String::new(),
                folder: Vec::new(),
                captures: Default::default(),
            });
        }

        s.execute("brush.presets.move", json!({"name": "Root C", "group": "", "folder": [], "index": 0, "rootIndex": 0})).unwrap();
        assert_eq!(
            s.tools.preset_hierarchy.root_order,
            [
                crate::BrushPresetRootEntry::Preset("Root C".into()),
                crate::BrushPresetRootEntry::Preset("Root A".into()),
                crate::BrushPresetRootEntry::Preset("Root B".into()),
            ]
        );
        s.execute("brush.presets.move", json!({"name": "Root C", "group": "", "folder": [], "index": 2, "rootIndex": 2})).unwrap();
        assert_eq!(
            s.tools.preset_hierarchy.root_order,
            [
                crate::BrushPresetRootEntry::Preset("Root A".into()),
                crate::BrushPresetRootEntry::Preset("Root B".into()),
                crate::BrushPresetRootEntry::Preset("Root C".into()),
            ]
        );
    }

    #[test]
    fn renaming_a_root_preset_keeps_its_mixed_root_order_slot() {
        let mut s = Session::new();
        s.tools.presets.clear();
        s.tools.preset_hierarchy = crate::BrushPresetHierarchy {
            groups: vec!["Empty".into()],
            folders: Vec::new(),
            root_order: vec![
                crate::BrushPresetRootEntry::Preset("Root A".into()),
                crate::BrushPresetRootEntry::Group("Empty".into()),
                crate::BrushPresetRootEntry::Preset("Root B".into()),
            ],
        };
        for name in ["Root A", "Root B"] {
            s.tools.presets.push(BrushPreset {
            id: uuid::Uuid::new_v4().to_string(),
                name: name.into(),
                brush: Default::default(),
                builtin: false,
                group: String::new(),
                folder: Vec::new(),
                captures: Default::default(),
            });
        }

        s.execute("brush.presets.rename", json!({"name": "Root A", "newName": "Renamed"})).unwrap();
        assert_eq!(
            s.tools.preset_hierarchy.root_order,
            [
                crate::BrushPresetRootEntry::Preset("Renamed".into()),
                crate::BrushPresetRootEntry::Group("Empty".into()),
                crate::BrushPresetRootEntry::Preset("Root B".into()),
            ]
        );
    }

    #[test]
    fn populated_root_group_root_index_move_reorders_the_mixed_sequence_and_preset_block() {
        let mut s = Session::new();
        s.tools.presets.clear();
        s.tools.preset_hierarchy = Default::default();
        for (name, group) in [("Root A", ""), ("Root B", ""), ("Grouped A", "Set"), ("Grouped B", "Set")] {
            s.tools.presets.push(BrushPreset {
            id: uuid::Uuid::new_v4().to_string(),
                name: name.into(),
                brush: Default::default(),
                builtin: false,
                group: group.into(),
                folder: Vec::new(),
                captures: Default::default(),
            });
        }

        s.execute("brush.presets.moveGroup", json!({"group": "Set", "rootIndex": 1})).unwrap();

        assert_eq!(
            s.tools.preset_hierarchy.root_order,
            [
                crate::BrushPresetRootEntry::Preset("Root A".into()),
                crate::BrushPresetRootEntry::Group("Set".into()),
                crate::BrushPresetRootEntry::Preset("Root B".into()),
            ]
        );
        let names = s.tools.presets.iter().map(|preset| preset.name.as_str()).collect::<Vec<_>>();
        assert_eq!(names, ["Root A", "Grouped A", "Grouped B", "Root B"]);
    }

    #[test]
    fn folder_drop_after_the_last_direct_preset_appends_after_it() {
        let mut s = Session::new();
        for (name, folder) in [("A", &[][..]), ("B", &[][..]), ("Child item", &["Child"][..])] {
            s.tools.presets.push(BrushPreset {
            id: uuid::Uuid::new_v4().to_string(),
                name: name.into(),
                brush: Default::default(),
                builtin: false,
                group: "Set".into(),
                folder: folder.iter().map(|part| (*part).to_string()).collect(),
                captures: Default::default(),
            });
        }

        s.execute("brush.presets.moveGroup", json!({"group": "Set", "folder": ["Child"], "into": {"group": "Set"}})).unwrap();

        let order: Vec<String> = s
            .tools
            .presets
            .iter()
            .filter(|preset| !preset.builtin)
            .map(|preset| if preset.folder.is_empty() { preset.name.clone() } else { format!("{}: {}", preset.folder.join("/"), preset.name) })
            .collect();
        assert_eq!(order, ["A", "B", "Child: Child item"]);
    }

    #[test]
    fn folder_drop_before_the_next_nested_folder_preserves_the_card_gap() {
        let mut s = Session::new();
        for (name, folder) in [("A", &[][..]), ("Nested item", &["Nested"][..]), ("B", &[][..]), ("Child item", &["Child"][..])] {
            s.tools.presets.push(BrushPreset {
            id: uuid::Uuid::new_v4().to_string(),
                name: name.into(),
                brush: Default::default(),
                builtin: false,
                group: "Set".into(),
                folder: folder.iter().map(|part| (*part).to_string()).collect(),
                captures: Default::default(),
            });
        }

        s.execute("brush.presets.moveGroup", json!({"group": "Set", "folder": ["Child"], "into": {"group": "Set"}, "before": "Nested"})).unwrap();

        let order: Vec<String> = s
            .tools
            .presets
            .iter()
            .filter(|preset| !preset.builtin)
            .map(|preset| if preset.folder.is_empty() { preset.name.clone() } else { format!("{}: {}", preset.folder.join("/"), preset.name) })
            .collect();
        assert_eq!(order, ["A", "Child: Child item", "Nested: Nested item", "B"]);
    }

    #[test]
    fn moving_a_folder_into_itself_is_rejected_without_mutation() {
        let mut s = with_folders();
        let before = s.tools.presets.clone();

        let result = s.execute("brush.presets.moveGroup", json!({"group": "Set", "folder": ["Ink"], "into": {"group": "Set", "folder": ["Ink"]}}));

        assert!(result.is_err());
        assert_eq!(s.tools.presets, before);
    }

    #[test]
    fn bad_moves_into_folders_fail_without_changes() {
        let mut s = with_two_groups();
        // A chain 31 deep under Set/Deep.
        let deep: Vec<String> = std::iter::once("Deep".to_string()).chain((0..30).map(|i| format!("d{i}"))).collect();
        s.tools.presets.push(BrushPreset {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Z".into(),
            brush: Default::default(),
            builtin: false,
            group: "Set".into(),
            folder: deep.clone(),
            captures: Default::default(),
        });
        s.tools.presets.push(BrushPreset {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Y".into(),
            brush: Default::default(),
            builtin: false,
            group: "Other".into(),
            folder: vec!["Dry".into()],
            captures: Default::default(),
        });
        let before = s.tools.presets.clone();
        let deep_json = json!(deep);
        for p in [
            // Into itself or its own descendant.
            json!({"group": "Set", "folder": ["Ink"], "into": {"group": "Set", "folder": ["Ink"]}}),
            json!({"group": "Set", "folder": ["Ink"], "into": {"group": "Set", "folder": ["Ink", "Fine"]}}),
            json!({"group": "Set", "into": {"group": "Set", "folder": ["Dry"]}}),
            json!({"group": "Set", "into": {"group": "Set"}}),
            // A group already at the top level; the ungrouped presets.
            json!({"group": "Set", "into": {}}),
            // A name clash at the destination.
            json!({"group": "Set", "folder": ["Dry"], "into": {"group": "Other"}}),
            json!({"group": "Other", "folder": ["Box"], "into": {"group": "Set", "folder": ["Ink"]}, "before": "Nope"}),
            json!({"group": "Other", "folder": ["Box"], "into": {"group": "Set"}, "beforeItem": "Nope"}),
            json!({"group": "Other", "folder": ["Box"], "into": {"group": "Set"}, "before": "Ink", "beforeItem": "S0"}),
            json!({"group": "Set", "folder": ["Dry"], "into": {"group": "Set", "folder": ["Ink"]}, "index": 0}),
            // Missing destinations and bad shapes.
            json!({"group": "Set", "folder": ["Dry"], "into": {"group": "Nope"}}),
            json!({"group": "Set", "folder": ["Dry"], "into": {"group": "Other", "folder": ["Nope"]}}),
            json!({"group": "Set", "folder": ["Dry"], "into": {"folder": ["Box"]}}),
            json!({"group": "Set", "folder": ["Dry"], "into": "Other"}),
            json!({"group": "Set", "folder": ["Dry"], "into": {"group": 5}}),
            json!({"group": "Set", "folder": ["Dry"], "into": {"group": "Other"}, "before": 3}),
            // Deeper than MAX_FOLDER_DEPTH once moved (Ink/Fine is 2 deep: 2 + 31 > 32).
            json!({"group": "Set", "folder": ["Deep"], "into": {"group": "Set", "folder": ["Ink", "Fine"]}}),
            json!({"group": "Set", "folder": ["Ink"], "into": {"group": "Set", "folder": deep_json}}),
        ] {
            assert!(s.execute("brush.presets.moveGroup", p.clone()).is_err(), "{p}");
        }
        assert_eq!(s.tools.presets, before);
        // Ink/Fine into Set itself: a clash with nothing, so it moves up a level.
        s.execute("brush.presets.moveGroup", json!({"group": "Set", "folder": ["Ink", "Fine"], "into": {"group": "Set"}})).unwrap();
        assert_eq!(folder_of(&s, "F1"), ["Fine"]);
        // An ungrouped preset's group can't move.
        s.tools.presets.push(BrushPreset {
            id: uuid::Uuid::new_v4().to_string(),
            name: "U".into(),
            brush: Default::default(),
            builtin: false,
            group: String::new(),
            folder: Vec::new(),
            captures: Default::default(),
        });
        assert!(s.execute("brush.presets.moveGroup", json!({"group": "", "into": {"group": "Set"}})).is_err());
    }
}
