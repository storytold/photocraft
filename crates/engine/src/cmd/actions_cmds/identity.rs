//! Persisted action occurrences are independent of name, list order, and folder position.
use std::collections::{BTreeMap, HashSet};

use super::{Action, PLAY_SHORTCUT_PREFIX, bad};
use crate::{Result, Session};

const OCCURRENCE_PREFIX: &str = "actions.playById:";
const COMMAND: &str = "action shortcut";

pub fn is_action_key(key: &str) -> bool {
    key.starts_with(PLAY_SHORTCUT_PREFIX) || key.starts_with(OCCURRENCE_PREFIX)
}

pub fn occurrence_key(id: &str) -> String {
    format!("{OCCURRENCE_PREFIX}{id}")
}

pub(crate) fn resolve_action_id(list: &[Action], id: &str) -> Option<usize> {
    if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return None;
    }
    let mut matches = list.iter().enumerate().filter(|(_, action)| action.action_id.as_deref() == Some(id));
    let (index, _) = matches.next()?;
    matches.next().is_none().then_some(index)
}

/// Never let a duplicated on-disk ID become valid just because one of its rows is removed.
pub(crate) fn invalidate_duplicate_ids(list: &mut [Action]) {
    let mut seen = HashSet::new();
    let mut duplicates = HashSet::new();
    for id in list.iter().filter_map(|action| action.action_id.as_ref()) {
        if !seen.insert(id) {
            duplicates.insert(id.clone());
        }
    }
    for action in list {
        if action.action_id.as_ref().is_some_and(|id| duplicates.contains(id)) {
            action.action_id = Some(String::new());
        }
    }
}

pub fn shortcut_row(list: &[Action], key: &str) -> Option<usize> {
    if let Some(id) = key.strip_prefix(OCCURRENCE_PREFIX) {
        return resolve_action_id(list, id);
    }
    let name = key.strip_prefix(PLAY_SHORTCUT_PREFIX)?;
    list.iter().position(|action| action.name == name)
}

/// Prepare an ID without changing the library, so a denied shortcut edit leaves it untouched.
pub fn prepare_action_id(list: &[Action], index: usize) -> Result<String> {
    let action = list.get(index).ok_or_else(|| bad(COMMAND, "action no longer exists"))?;
    if let Some(id) = &action.action_id {
        return if resolve_action_id(list, id) == Some(index) { Ok(id.clone()) } else { Err(bad(COMMAND, "invalid or duplicate action ID")) };
    }
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|error| bad(COMMAND, format!("could not create an action ID: {error}")))?;
    let id = format!("{:032x}", u128::from_be_bytes(bytes));
    if list.iter().any(|a| a.action_id.as_deref() == Some(id.as_str())) {
        return Err(bad(COMMAND, "duplicate generated action ID"));
    }
    Ok(id)
}

/// Complete a successful shortcut edit. The shortcut command only edits preferences.
pub fn commit_action_id(s: &mut Session, index: usize, id: String) {
    if s.actions.list[index].action_id.is_none() {
        s.actions.list[index].action_id = Some(id);
        s.actions.touch();
        s.sync_preset_store();
    }
}

/// A valid explicit key takes precedence over a legacy key for the same row.
pub fn action_shortcut(s: &Session, index: usize) -> Option<&str> {
    let list = &s.actions.list;
    let action = list.get(index)?;
    let keys = &s.prefs().shortcuts;
    if let Some(id) = action.action_id.as_deref().filter(|id| resolve_action_id(list, id) == Some(index))
        && let Some(key) = keys.get(&occurrence_key(id))
    {
        return Some(key.as_str());
    }
    let legacy = format!("{PLAY_SHORTCUT_PREFIX}{}", action.name);
    (shortcut_row(list, &legacy) == Some(index)).then(|| keys.get(&legacy)).flatten().map(String::as_str)
}

/// Move a legacy binding to its already resolved row before an edit changes name lookup.
pub(crate) fn pin_shortcut(
    list: &mut [Action],
    index: usize,
    shortcuts: &mut BTreeMap<String, String>,
    alternates: &mut BTreeMap<String, Vec<String>>,
) -> Result<()> {
    let legacy = format!("{PLAY_SHORTCUT_PREFIX}{}", list[index].name);
    if shortcut_row(list, &legacy) != Some(index) || !(shortcuts.contains_key(&legacy) || alternates.contains_key(&legacy)) {
        return Ok(());
    }
    let id = prepare_action_id(list, index)?;
    let key = occurrence_key(&id);
    list[index].action_id = Some(id);
    if let Some(binding) = shortcuts.remove(&legacy) {
        shortcuts.entry(key.clone()).or_insert(binding);
    }
    if let Some(bindings) = alternates.remove(&legacy) {
        alternates.entry(key).or_insert(bindings);
    }
    Ok(())
}

/// Remove only the bindings owned by the deleted rows, before the indices or first match change.
pub(crate) fn remove_shortcuts(s: &mut Session, remove: impl Fn(usize, &Action) -> bool) {
    let list = &s.actions.list;
    let mut keys = s
        .prefs()
        .shortcuts
        .keys()
        .chain(s.prefs().alternate_shortcuts.keys())
        .filter(|key| shortcut_row(list, key).is_some_and(|row| remove(row, &list[row])))
        .cloned()
        .peekable();
    if keys.peek().is_none() {
        return;
    }
    let keys: Vec<_> = keys.collect();
    s.edit_prefs(|prefs| {
        for key in keys {
            prefs.shortcuts.remove(&key);
            prefs.alternate_shortcuts.remove(&key);
        }
    });
}
