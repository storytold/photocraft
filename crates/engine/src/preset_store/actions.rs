//! Keep a saved reference in its original namespace when startup attaches the disk library.
use std::collections::HashSet;

use serde_json::{Value, json};

use crate::Session;
use crate::actions_cmds::{
    Action, PLAY_SHORTCUT_PREFIX, branch_id_field, conditional_branch, pin_shortcut, prepare_action_id, resolve_play_target, shortcut_row, valid_action_id,
    visit_calls,
};

/// Explicit IDs that failed in their source list must also fail after the lists are joined.
fn block_unknown_ids(list: &mut [Action]) {
    let ids: HashSet<_> = list.iter().filter_map(|action| action.action_id.as_ref().filter(|id| valid_action_id(id)).cloned()).collect();
    for action in list {
        visit_calls(&mut action.steps, &mut |command, params, _| {
            let fields: &[&str] = match command {
                "actions.play" | "file.automate.batch" => &["actionId"],
                "actions.conditional" => &["thenActionId", "elseActionId"],
                _ => &[],
            };
            for field in fields {
                if let Some(value) = params.get(*field)
                    && !value.as_str().is_some_and(|id| ids.contains(id))
                {
                    params[*field] = Value::Null;
                }
            }
        });
    }
}

pub(super) fn pin_disk_targets(s: &mut Session, disk: &mut [Action], warnings: &mut Vec<String>) {
    if s.actions.list.is_empty() || disk.is_empty() {
        return;
    }
    block_unknown_ids(&mut s.actions.list);
    block_unknown_ids(disk);
    let memory = &s.actions.list;

    // A legacy shortcut read with the saved preferences belongs to the first disk match.
    // The current Action Options path already gives a newly assigned in-memory binding an ID.
    let (mut keys, mut alternates) = (s.prefs().shortcuts.clone(), s.prefs().alternate_shortcuts.clone());
    let owners: Vec<_> = keys
        .keys()
        .chain(alternates.keys())
        .filter_map(|key| {
            let name = key.strip_prefix(PLAY_SHORTCUT_PREFIX)?;
            memory.iter().any(|action| action.name == name).then(|| shortcut_row(disk, key)).flatten().map(|row| (key.clone(), row))
        })
        .collect();
    for (key, row) in owners {
        if let Err(error) = pin_shortcut(disk, row, &mut keys, &mut alternates) {
            // The original target cannot be identified. The legacy key must not play a memory row.
            keys.remove(&key);
            alternates.remove(&key);
            warnings.push(format!("actions: {error}"));
        }
    }

    for row in 0..disk.len() {
        let mut steps = std::mem::take(&mut disk[row].steps);
        visit_calls(&mut steps, &mut |command, params, _| match command {
            "actions.play" | "file.automate.batch" if params.get("actionId").is_none() => {
                let resolve = |list: &[Action]| {
                    if command == "actions.play" { resolve_play_target(list, params, command) } else { crate::file_cmds::batch_action(list, params) }
                };
                if (params.get("action").is_some_and(Value::is_number) || resolve(memory).is_ok())
                    && let Ok(target) = resolve(disk)
                {
                    params["actionId"] = pin_target(disk, target, warnings);
                }
            }
            "actions.conditional" => {
                for branch in ["then", "else"] {
                    let field = branch_id_field(branch);
                    if params.get(field).is_some() {
                        continue;
                    }
                    if let Ok(Some(target)) = conditional_branch(params, branch)
                        && resolve_play_target(memory, &target, command).is_ok()
                        && let Ok(target) = resolve_play_target(disk, &target, command)
                    {
                        params[field] = pin_target(disk, target, warnings);
                    }
                }
            }
            _ => {}
        });
        disk[row].steps = steps;
    }
    if keys != s.prefs().shortcuts || alternates != s.prefs().alternate_shortcuts {
        s.edit_prefs(|prefs| {
            prefs.shortcuts = keys;
            prefs.alternate_shortcuts = alternates;
        });
    }
}

fn pin_target(list: &mut [Action], index: usize, warnings: &mut Vec<String>) -> Value {
    match prepare_action_id(list, index) {
        Ok(id) => {
            list[index].action_id = Some(id.clone());
            json!(id)
        }
        Err(error) => {
            warnings.push(format!("actions: {error}"));
            Value::Null
        }
    }
}
