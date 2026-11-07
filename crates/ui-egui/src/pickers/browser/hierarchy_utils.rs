//! Structural hierarchy relationships, sibling ordering, and move planning.
//!
//! This module is deliberately independent of pointer coordinates and egui geometry. Dragging
//! resolves a pointer into a sibling/container destination; these helpers interpret the hierarchy.

use std::collections::{BTreeMap, BTreeSet};

use crate::pickers::browser::identity::PickerNodeIdentity;
use crate::pickers::browser::path_identity_model::{ContainerId, DropPosition, HierarchyNode, PickerBrowserHierarchy, PickerId};

/// Relationship in a name-keyed parent projection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GroupRelation {
    Same,
    Descendant,
    Unrelated,
}

pub(crate) fn group_relation(parents: &BTreeMap<String, String>, candidate: &str, ancestor: &str) -> GroupRelation {
    if candidate == ancestor {
        return GroupRelation::Same;
    }
    let mut current = candidate;
    let mut visited = BTreeSet::new();
    while let Some(parent) = parents.get(current) {
        if parent == ancestor {
            return GroupRelation::Descendant;
        }
        if !visited.insert(parent.as_str()) {
            break;
        }
        current = parent;
    }
    GroupRelation::Unrelated
}

pub(crate) fn is_same_or_descendant(parents: &BTreeMap<String, String>, candidate: &str, ancestor: &str) -> bool {
    group_relation(parents, candidate, ancestor) != GroupRelation::Unrelated
}

/// Path-qualified structural result used by the Brushes compatibility helper. Browser hit areas
/// no longer produce this type; the canonical drag path carries `DropPosition` directly.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum HierarchyDrop {
    MoveGroup { group: String, group_path: Vec<String>, before: Option<String> },
    MoveInto { group: String, group_path: Vec<String>, into: Option<(String, Vec<String>)>, before: Option<String> },
}

/// Resolve the legacy Brushes API's before/into choice using only group paths, with no pointer or
/// egui geometry. Runtime browser dragging uses object-owned `DropPosition` instead.
pub(crate) fn hierarchy_group_drop(src_group: &str, src_path: &[String], target_group: &str, target_path: &[String], edge: bool) -> Option<HierarchyDrop> {
    if src_group == target_group && target_path.starts_with(src_path) {
        return None;
    }
    let group = src_group.to_owned();
    let group_path = src_path.to_vec();
    if !edge {
        let parent = src_path.split_last().map(|(_, parent)| parent);
        if src_group == target_group && parent == Some(target_path) {
            return None;
        }
        return Some(HierarchyDrop::MoveInto { group, group_path, into: Some((target_group.to_owned(), target_path.to_vec())), before: None });
    }
    match target_path.split_last() {
        None if src_path.is_empty() => Some(HierarchyDrop::MoveGroup { group, group_path, before: Some(target_group.to_owned()) }),
        None => Some(HierarchyDrop::MoveInto { group, group_path, into: None, before: Some(target_group.to_owned()) }),
        Some((leaf, parent)) => {
            let siblings = src_group == target_group && src_path.split_last().map(|(_, source_parent)| source_parent) == Some(parent);
            if siblings {
                Some(HierarchyDrop::MoveGroup { group, group_path, before: Some(leaf.clone()) })
            } else {
                Some(HierarchyDrop::MoveInto { group, group_path, into: Some((target_group.to_owned(), parent.to_vec())), before: Some(leaf.clone()) })
            }
        }
    }
}

/// Selected roots in a safe insertion order for a shared before/after/prepend anchor.
pub(crate) fn hierarchy_move_sources(selection: &[PickerId], position: &DropPosition) -> Vec<PickerId> {
    if matches!(position, DropPosition::After { .. } | DropPosition::PrependTo { .. }) { selection.iter().rev().cloned().collect() } else { selection.to_vec() }
}

pub(crate) fn hierarchy_drop_item_destination(
    source: &PickerId,
    position: &DropPosition,
    hierarchy: &PickerBrowserHierarchy,
    identities: &BTreeMap<PickerId, PickerNodeIdentity>,
) -> Option<(String, Vec<String>)> {
    let container = match position {
        DropPosition::Before { target } | DropPosition::After { target } => hierarchy.parent_of(target)?,
        DropPosition::PrependTo { group } => ContainerId::Group(group.clone()),
        DropPosition::AppendTo { container } => container.clone(),
    };
    match container {
        ContainerId::Group(group_id) => match identities.get(&group_id)? {
            PickerNodeIdentity::Group(group) => Some((group.clone(), Vec::new())),
            PickerNodeIdentity::Folder { group, path } => Some((group.clone(), path.clone())),
            _ => None,
        },
        ContainerId::Root => match identities.get(source)? {
            PickerNodeIdentity::Item { group, .. }
            | PickerNodeIdentity::FolderItem { group, .. }
            | PickerNodeIdentity::Group(group)
            | PickerNodeIdentity::Folder { group, .. } => Some((group.clone(), Vec::new())),
        },
    }
}

/// Resolve item-domain indices while simulating hierarchy moves in their actual commit order.
pub(crate) fn plan_hierarchy_item_moves(
    hierarchy: &PickerBrowserHierarchy,
    sources: &[PickerId],
    position: &DropPosition,
    identities: &BTreeMap<PickerId, PickerNodeIdentity>,
    root_item_group: Option<&str>,
) -> Result<Vec<(String, String, String, usize)>, String> {
    let mut planned = hierarchy.clone();
    let mut item_moves = Vec::new();
    for source in hierarchy_move_sources(sources, position) {
        if let Some(PickerNodeIdentity::Item { group, key }) = identities.get(&source) {
            let resolved = planned.resolve_move(&source, position).map_err(|error| format!("Could not resolve item move: {error:?}"))?;
            let destination_group = match resolved.destination() {
                ContainerId::Group(id) => match identities.get(id) {
                    Some(PickerNodeIdentity::Group(group)) => Some(group.as_str()),
                    _ => return Err("The destination group is no longer available.".to_owned()),
                },
                ContainerId::Root => root_item_group,
            };
            if let Some(destination_group) = destination_group {
                let children = planned.children_of(resolved.destination()).ok_or_else(|| "The destination is no longer available.".to_owned())?;
                let same_container = planned.parent_of(&source).as_ref() == Some(resolved.destination());
                let domain_index = children
                    .iter()
                    .filter(|node| !same_container || node.id() != &source)
                    .take(resolved.insertion_index())
                    .filter(|node| matches!(node, HierarchyNode::Item(_)))
                    .count();
                item_moves.push((group.clone(), key.clone(), destination_group.to_owned(), domain_index));
            }
        }
        planned.move_node(&source, position).map_err(|error| format!("Could not plan selected hierarchy move: {error:?}"))?;
    }
    Ok(item_moves)
}

#[cfg(test)]
mod tests {
    use super::{GroupRelation, group_relation};
    use std::collections::BTreeMap;

    #[test]
    fn parent_map_queries_distinguish_identity_ancestry_and_unrelated_branches() {
        let parents = BTreeMap::from([
            ("child".to_owned(), "parent".to_owned()),
            ("grandchild".to_owned(), "child".to_owned()),
            ("sibling".to_owned(), "parent".to_owned()),
        ]);

        assert_eq!(group_relation(&parents, "parent", "parent"), GroupRelation::Same);
        assert_eq!(group_relation(&parents, "grandchild", "parent"), GroupRelation::Descendant);
        assert_eq!(group_relation(&parents, "sibling", "child"), GroupRelation::Unrelated);
    }

    #[test]
    fn malformed_parent_cycles_terminate() {
        let parents = BTreeMap::from([("a".to_owned(), "b".to_owned()), ("b".to_owned(), "a".to_owned())]);
        assert_eq!(group_relation(&parents, "a", "missing"), GroupRelation::Unrelated);
    }
}
