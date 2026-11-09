//! Operation-specific interpretation of the browser's canonical selected targets.
//!
//! Selection resolution preserves exactly what the user selected. Each operation decides what
//! those targets mean: rename edits each selected header/card, while delete recursively removes
//! selected containers and suppresses descendants already covered by them.

use super::asset_browser::BrowserAction;
use super::selection::{BrowserSelection, SelectionTarget};

pub(crate) fn rename_selection(selection: &BrowserSelection) -> Option<BrowserAction> {
    if selection.targets.is_empty() {
        return None;
    }
    let mut targets = selection.targets.clone();
    targets.sort_by_key(|target| match target {
        SelectionTarget::Item { .. } => (0usize, 0usize),
        SelectionTarget::Folder { path, .. } => (1, usize::MAX - path.len()),
        SelectionTarget::Group(_) => (2, 0),
    });
    Some(BrowserAction::BeginRenameSelection { targets })
}

pub(crate) fn delete_selection(selection: &BrowserSelection) -> Vec<BrowserAction> {
    delete_targets_actions(&selection.targets)
}

pub(crate) fn delete_targets_actions(targets: &[SelectionTarget]) -> Vec<BrowserAction> {
    delete_targets(targets).iter().map(delete_action).collect()
}

pub(crate) fn delete_targets(targets: &[SelectionTarget]) -> Vec<SelectionTarget> {
    targets.iter().filter(|target| !targets.iter().any(|ancestor| ancestor != *target && covers(ancestor, target))).cloned().collect()
}

fn covers(ancestor: &SelectionTarget, target: &SelectionTarget) -> bool {
    match (ancestor, target) {
        (SelectionTarget::Group(group), SelectionTarget::Folder { group: child_group, .. })
        | (SelectionTarget::Group(group), SelectionTarget::Item { group: child_group, .. }) => group == child_group,
        (SelectionTarget::Folder { group, path }, SelectionTarget::Folder { group: child_group, path: child_path })
        | (SelectionTarget::Folder { group, path }, SelectionTarget::Item { group: child_group, path: child_path, .. }) => {
            group == child_group && child_path.starts_with(path) && child_path.len() >= path.len()
        }
        _ => false,
    }
}

fn delete_action(target: &SelectionTarget) -> BrowserAction {
    match target {
        SelectionTarget::Group(group) => BrowserAction::DeleteGroup(group.clone()),
        SelectionTarget::Folder { group, path } => BrowserAction::DeleteNestedGroup { group: group.clone(), group_path: path.clone() },
        SelectionTarget::Item { group, path, key } => BrowserAction::DeleteItem { group: group.clone(), group_path: path.clone(), key: key.clone() },
    }
}

pub(crate) fn export_selection(selection: &BrowserSelection) -> Vec<String> {
    selection.cards.iter().map(|(_, key)| key.clone()).collect()
}
