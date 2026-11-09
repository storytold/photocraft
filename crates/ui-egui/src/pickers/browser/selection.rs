//! Canonical browser-selection projection shared by every menu and batch operation.
//!
//! `PresetUi::browser_selection` remains the sole source of truth: it contains the selected opaque
//! picker IDs used by the renderer. This module resolves those IDs against the current hierarchy
//! into domain-facing targets. The result is a disposable view, never an independently maintained
//! selection state.

use std::collections::BTreeSet;

use super::asset_browser::{GroupView, ItemView};
use super::identity::PickerNodeIdentity;
use super::path_identity_model::{PickerBrowserHierarchy, PickerId};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct BrowserSelection {
    /// Explicitly selected card occurrences, in browser order.
    pub(crate) selected_items: Vec<(String, ItemView)>,
    /// Explicitly selected top-level groups, in browser order.
    pub(crate) groups: Vec<String>,
    /// All cards selected directly or covered by an explicitly selected container.
    pub(crate) cards: Vec<(String, String)>,
    /// Explicitly selected cards, retained as lightweight keys for older adapter consumers.
    pub(crate) selected_cards: Vec<(String, String)>,
    /// Exactly the nodes explicitly selected by the user, in browser order.
    pub(crate) targets: Vec<SelectionTarget>,
    pub(crate) contains_selected_card: bool,
    pub(crate) contains_card: bool,
    pub(crate) contains_group: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SelectionTarget {
    Group(String),
    Folder { group: String, path: Vec<String> },
    Item { group: String, path: Vec<String>, key: String },
}

/// Resolve canonical selected IDs into one ordered snapshot for all browser consumers.
pub(crate) fn resolve(
    selected: &BTreeSet<PickerId>,
    hierarchy: &PickerBrowserHierarchy,
    identities: &std::collections::BTreeMap<PickerId, PickerNodeIdentity>,
    groups: &[GroupView],
) -> BrowserSelection {
    let mut result = BrowserSelection::default();
    let mut selected_item_ids = BTreeSet::new();
    result.targets = resolve_targets(selected, hierarchy, identities);
    for target in &result.targets {
        match target {
            SelectionTarget::Group(group) => result.groups.push(group.clone()),
            SelectionTarget::Folder { .. } => {}
            SelectionTarget::Item { group, path, key } => {
                if let Some(item) =
                    groups.iter().find(|view| &view.name == group).and_then(|view| view.items.iter().find(|item| item.key == *key && item.group_path == *path))
                {
                    result.selected_items.push((group.clone(), item.clone()));
                    selected_item_ids.insert((group.clone(), key.clone(), path.clone()));
                }
            }
        }
    }
    result.selected_cards = result.selected_items.iter().map(|(group, item)| (group.clone(), item.key.clone())).collect();
    result.contains_selected_card = !result.selected_items.is_empty();
    result.contains_group = result.targets.iter().any(|target| matches!(target, SelectionTarget::Group(_) | SelectionTarget::Folder { .. }));
    result.cards = groups
        .iter()
        .flat_map(|group| {
            group.items.iter().filter_map(|item| {
                let explicitly_selected = selected_item_ids.contains(&(group.name.clone(), item.key.clone(), item.group_path.clone()));
                let covered_by_group = result.targets.iter().any(|target| match target {
                    SelectionTarget::Group(owner) => owner == &group.name,
                    SelectionTarget::Folder { group: owner, path } => owner == &group.name && item.group_path.starts_with(path),
                    SelectionTarget::Item { .. } => false,
                });
                (explicitly_selected || covered_by_group).then(|| (group.name.clone(), item.key.clone()))
            })
        })
        .collect();
    result.contains_card = !result.cards.is_empty();

    result
}

/// Resolve every explicitly selected browser node without interpreting its operation semantics.
pub(crate) fn resolve_targets(
    selected: &BTreeSet<PickerId>,
    hierarchy: &PickerBrowserHierarchy,
    identities: &std::collections::BTreeMap<PickerId, PickerNodeIdentity>,
) -> Vec<SelectionTarget> {
    fn visit(node: &super::path_identity_model::HierarchyNode, ids: &mut Vec<PickerId>) {
        ids.push(node.id().clone());
        for child in node.children() {
            visit(child, ids);
        }
    }
    let mut order = Vec::new();
    for root in hierarchy.roots() {
        visit(root, &mut order);
    }
    let mut targets = Vec::new();
    for id in order.iter().filter(|id| selected.contains(*id)) {
        match identities.get(&id) {
            Some(PickerNodeIdentity::Group(group)) => targets.push(SelectionTarget::Group(group.clone())),
            Some(PickerNodeIdentity::Folder { group, path }) => targets.push(SelectionTarget::Folder { group: group.clone(), path: path.clone() }),
            Some(PickerNodeIdentity::Item { group, key }) => targets.push(SelectionTarget::Item { group: group.clone(), path: Vec::new(), key: key.clone() }),
            Some(PickerNodeIdentity::FolderItem { group, path, key }) => {
                targets.push(SelectionTarget::Item { group: group.clone(), path: path.clone(), key: key.clone() })
            }
            None => {}
        }
    }
    targets
}
