//! Shared projection of adapter-provided picker views into the canonical hierarchy.
//!
//! This module owns normalized view data, common tree assembly, structural-folder synthesis,
//! mixed sibling ordering and rendering-segment flattening. Adapters supply all stable identity
//! tokens and reverse metadata; the projector never derives either from view names or paths.

use std::collections::BTreeSet;

use super::super::state::HierarchyOrderEntry;
use super::identity::{PickerNodeIdentity, PickerNodeRole, PickerProjectionKeys, ProjectionNodeRef};
use super::path_identity_model::{HierarchyNode, PickerBrowserHierarchy, PickerId, PickerIdRegistry};

/// Asset-neutral item projection supplied by a picker adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ItemView {
    /// Stable adapter-defined item key used by ordering and domain actions.
    pub(crate) key: String,
    /// Display label shown by the browser.
    pub(crate) name: String,
    /// Structural folder path to synthesize beneath the owning asset group.
    pub(crate) group_path: Vec<String>,
}

/// Asset-neutral group projection supplied by a picker adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GroupView {
    /// Stable adapter-defined group key and display label.
    pub(crate) name: String,
    /// Backing-only group whose cards participate in the root mixed sequence.
    pub(crate) root_only: bool,
    pub(crate) items: Vec<ItemView>,
}

/// Flattened rendering segment for one consecutive run of cards in a canonical sibling sequence.
pub(crate) struct GroupRenderSegment {
    pub(crate) group_index: usize,
    pub(crate) depth: usize,
    pub(crate) root_items: bool,
    pub(crate) show_header: bool,
    pub(crate) folder_path: Vec<String>,
    pub(crate) show_folder_header: bool,
    /// Source group and item index for each card. Root card runs can span backing groups.
    pub(crate) item_indices: Vec<(usize, usize)>,
}

fn projection_id(registry: &mut PickerIdRegistry, keys: &PickerProjectionKeys, node: ProjectionNodeRef) -> Option<PickerId> {
    Some(registry.id_for(keys.key(&node)?.clone()))
}

/// Flatten the shared sibling sequences into render segments. Cards and child groups are walked in
/// their canonical order; adjacent cards are batched for grid layout, and every group is emitted at
/// exactly the position represented by its `Group` token.
pub(crate) fn build_hierarchy_projection(
    groups: &[GroupView],
    display: &[(usize, usize)],
    parents: &std::collections::BTreeMap<String, String>,
    orders: &std::collections::BTreeMap<String, Vec<HierarchyOrderEntry>>,
    projection_keys: &PickerProjectionKeys,
    registry: &mut PickerIdRegistry,
) -> Option<(Vec<GroupRenderSegment>, PickerBrowserHierarchy, std::collections::BTreeMap<PickerId, PickerNodeIdentity>)> {
    fn insert_path_item(
        nodes: &mut Vec<HierarchyNode>,
        parent_path: &[String],
        path: &[String],
        item: HierarchyNode,
        identities: &mut std::collections::BTreeMap<PickerId, PickerNodeIdentity>,
        projection_keys: &PickerProjectionKeys,
        registry: &mut PickerIdRegistry,
        group_index: usize,
    ) -> Option<()> {
        let Some((name, rest)) = path.split_first() else {
            nodes.push(item);
            return Some(());
        };
        let full_path: Vec<String> = parent_path.iter().cloned().chain(std::iter::once(name.clone())).collect();
        let id = projection_id(registry, projection_keys, ProjectionNodeRef::Folder { group_index, path: full_path.clone() });
        let id = id?;
        let folder_identity = projection_keys.reverse(&ProjectionNodeRef::Folder { group_index, path: full_path.clone() })?.clone();
        let index = nodes.iter().position(|node| node.id() == &id).unwrap_or_else(|| {
            nodes.push(HierarchyNode::group(id.clone(), Vec::new()));
            identities.insert(id.clone(), folder_identity);
            nodes.len() - 1
        });
        if let Some(HierarchyNode::Group { children, .. }) = nodes.get_mut(index) {
            insert_path_item(children, &full_path, rest, item, identities, projection_keys, registry, group_index)?;
        }
        Some(())
    }

    struct ProjectionContext<'a> {
        groups: &'a [GroupView],
        display: &'a [(usize, usize)],
        parents: &'a std::collections::BTreeMap<String, String>,
        orders: &'a std::collections::BTreeMap<String, Vec<HierarchyOrderEntry>>,
        lifted_items: &'a BTreeSet<(String, String)>,
        visited: BTreeSet<usize>,
        item_indices: std::collections::BTreeMap<PickerId, (usize, usize)>,
        identities: std::collections::BTreeMap<PickerId, PickerNodeIdentity>,
        registry: &'a mut PickerIdRegistry,
        projection_keys: &'a PickerProjectionKeys,
    }

    fn build_group(group_index: usize, depth: usize, context: &mut ProjectionContext<'_>) -> Option<HierarchyNode> {
        let (group_name, group_items) = {
            let group = context.groups.get(group_index)?;
            (group.name.clone(), group.items.clone())
        };
        if !context.visited.insert(group_index) {
            return None;
        }
        let children: Vec<(usize, usize)> = context
            .display
            .iter()
            .filter_map(|&(index, child_depth)| {
                let child = context.groups.get(index)?;
                (child_depth == depth + 1 && context.parents.get(&child.name).is_some_and(|parent| parent == &group_name)).then_some((index, child_depth))
            })
            .collect();
        let child_names: std::collections::BTreeMap<String, (usize, usize)> =
            children.iter().filter_map(|&(index, child_depth)| context.groups.get(index).map(|child| (child.name.clone(), (index, child_depth)))).collect();
        let mut entries = context.orders.get(&group_name).cloned().unwrap_or_default();
        if !context.orders.contains_key(&group_name) {
            entries.extend(
                group_items
                    .iter()
                    .filter(|item| !context.lifted_items.contains(&(group_name.clone(), item.key.clone())))
                    .map(|item| HierarchyOrderEntry::Item(item.key.clone())),
            );
            entries.extend(children.iter().filter_map(|(index, _)| context.groups.get(*index).map(|child| HierarchyOrderEntry::Group(child.name.clone()))));
        }
        // Saved sequences can be stale or partial. Keep the same deterministic append behavior
        // as the old projection while ensuring every live adapter node is represented once.
        for item in group_items.iter().filter(|item| !context.lifted_items.contains(&(group_name.clone(), item.key.clone()))) {
            let entry = HierarchyOrderEntry::Item(item.key.clone());
            if !entries.contains(&entry) {
                entries.push(entry);
            }
        }
        for (index, _) in &children {
            if let Some(child) = context.groups.get(*index) {
                let entry = HierarchyOrderEntry::Group(child.name.clone());
                if !entries.contains(&entry) {
                    entries.push(entry);
                }
            }
        }
        let item_by_key: std::collections::BTreeMap<String, usize> = group_items
            .iter()
            .enumerate()
            .filter(|(_, item)| !context.lifted_items.contains(&(group_name.clone(), item.key.clone())))
            .map(|(index, item)| (item.key.clone(), index))
            .collect();
        let mut nodes = Vec::new();
        for entry in entries {
            match entry {
                HierarchyOrderEntry::Item(key) => {
                    if let Some(&index) = item_by_key.get(&key) {
                        let item = &group_items[index];
                        let folder_path = &item.group_path[..item.group_path.len().min(crate::pickers::browser::hierarchy::MAX_HIERARCHY_DEPTH)];
                        let id = projection_id(context.registry, context.projection_keys, ProjectionNodeRef::Item { group_index, item_index: index })?;
                        context.item_indices.insert(id.clone(), (group_index, index));
                        if folder_path.is_empty() {
                            context
                                .identities
                                .insert(id.clone(), context.projection_keys.reverse(&ProjectionNodeRef::Item { group_index, item_index: index })?.clone());
                            nodes.push(HierarchyNode::item(id));
                        } else {
                            context
                                .identities
                                .insert(id.clone(), context.projection_keys.reverse(&ProjectionNodeRef::Item { group_index, item_index: index })?.clone());
                            insert_path_item(
                                &mut nodes,
                                &[],
                                folder_path,
                                HierarchyNode::item(id),
                                &mut context.identities,
                                context.projection_keys,
                                context.registry,
                                group_index,
                            )?;
                        }
                    }
                }
                HierarchyOrderEntry::Group(name) => {
                    if let Some((index, child_depth)) = child_names.get(&name).copied()
                        && let Some(child) = build_group(index, child_depth, context)
                    {
                        nodes.push(child);
                    }
                }
                // Root-only entries are never valid in a group's child sequence.
                HierarchyOrderEntry::RootItem { .. } => {}
            }
        }
        let id = projection_id(context.registry, context.projection_keys, ProjectionNodeRef::Group(group_index))?;
        context.identities.insert(id.clone(), context.projection_keys.reverse(&ProjectionNodeRef::Group(group_index))?.clone());
        Some(HierarchyNode::group(id, nodes))
    }

    fn flatten_group(
        node: &HierarchyNode,
        depth: usize,
        root_group_index: usize,
        folder_path: &[String],
        item_indices: &std::collections::BTreeMap<PickerId, (usize, usize)>,
        group_indices: &std::collections::BTreeMap<PickerId, usize>,
        identities: &std::collections::BTreeMap<PickerId, PickerNodeIdentity>,
        out: &mut Vec<GroupRenderSegment>,
    ) {
        let group_index = group_indices.get(node.id()).copied().unwrap_or(root_group_index);
        let mut run = Vec::new();
        let mut first = true;
        for child in node.children() {
            match child {
                HierarchyNode::Item(id) => {
                    if let Some((source_group_index, item_index)) = item_indices.get(id) {
                        run.push((*source_group_index, *item_index));
                    }
                }
                HierarchyNode::Group { .. } => {
                    if !run.is_empty() || first {
                        out.push(GroupRenderSegment {
                            group_index,
                            depth,
                            root_items: false,
                            show_header: folder_path.is_empty() && first,
                            folder_path: folder_path.to_vec(),
                            show_folder_header: !folder_path.is_empty() && first,
                            item_indices: std::mem::take(&mut run),
                        });
                        first = false;
                    }
                    match identities.get(child.id()) {
                        Some(identity) if identity.role() == PickerNodeRole::StructuralFolder => {
                            let PickerNodeIdentity::Folder { path, .. } = identity else { return };
                            flatten_group(child, depth, root_group_index, path, item_indices, group_indices, identities, out);
                        }
                        _ => {
                            let child_index = group_indices.get(child.id()).copied().unwrap_or(root_group_index);
                            flatten_group(child, depth + 1, child_index, &[], item_indices, group_indices, identities, out);
                        }
                    }
                }
            }
        }
        if !run.is_empty() || first {
            out.push(GroupRenderSegment {
                group_index,
                depth,
                root_items: false,
                show_header: folder_path.is_empty() && first,
                folder_path: folder_path.to_vec(),
                show_folder_header: !folder_path.is_empty() && first,
                item_indices: run,
            });
        }
    }

    let lifted_items: BTreeSet<(String, String)> = orders
        .get("")
        .into_iter()
        .flatten()
        .filter_map(|entry| match entry {
            HierarchyOrderEntry::RootItem { group, key } => Some((group.clone(), key.clone())),
            _ => None,
        })
        .collect();
    let mut context = ProjectionContext {
        groups,
        display,
        parents,
        orders,
        lifted_items: &lifted_items,
        visited: BTreeSet::new(),
        item_indices: std::collections::BTreeMap::new(),
        identities: std::collections::BTreeMap::new(),
        registry,
        projection_keys,
    };
    let mut roots: Vec<HierarchyNode> = display
        .iter()
        .filter_map(|&(index, depth)| (depth == 0).then_some(index))
        .filter(|index| groups.get(*index).is_some_and(|group| !group.root_only))
        .filter_map(|index| build_group(index, 0, &mut context))
        .collect();
    let mut item_indices = context.item_indices;
    let mut identities = context.identities;
    let root_entries = orders.get("").cloned().unwrap_or_default();
    let group_by_name: std::collections::BTreeMap<&str, usize> = groups.iter().enumerate().map(|(i, g)| (g.name.as_str(), i)).collect();
    let root_group_display_order: Vec<String> = roots
        .iter()
        .filter_map(|node| match identities.get(node.id()) {
            Some(PickerNodeIdentity::Group(name)) => Some(name.clone()),
            _ => None,
        })
        .collect();
    let mut root_groups: std::collections::BTreeMap<String, HierarchyNode> = roots
        .drain(..)
        .filter_map(|node| {
            let PickerNodeIdentity::Group(name) = identities.get(node.id())? else { return None };
            Some((name.clone(), node))
        })
        .collect();
    let mut ordered_roots = Vec::new();
    for entry in root_entries {
        match entry {
            HierarchyOrderEntry::Group(name) => {
                if let Some(node) = root_groups.remove(&name) {
                    ordered_roots.push(node);
                }
            }
            // Ordinary Item entries belong to a group sequence; malformed/root legacy values are ignored.
            HierarchyOrderEntry::Item(_) => {}
            HierarchyOrderEntry::RootItem { group: group_name, key } => {
                let Some(&group_index) = group_by_name.get(group_name.as_str()) else { continue };
                let Some(item_index) = groups[group_index].items.iter().position(|item| item.key == key) else { continue };
                let id = projection_id(registry, projection_keys, ProjectionNodeRef::Item { group_index, item_index })?;
                item_indices.insert(id.clone(), (group_index, item_index));
                identities.insert(id.clone(), projection_keys.reverse(&ProjectionNodeRef::Item { group_index, item_index })?.clone());
                ordered_roots.push(HierarchyNode::item(id));
            }
        }
    }
    for name in root_group_display_order {
        if let Some(node) = root_groups.remove(&name) {
            ordered_roots.push(node);
        }
    }
    roots = ordered_roots;
    // The adapter supplied one opaque token per structural node. Duplicate domain keys remain
    // distinct whenever the adapter's identity policy says they are distinct.
    let Ok(hierarchy) = PickerBrowserHierarchy::new(roots) else { return None };
    let group_indices: std::collections::BTreeMap<PickerId, usize> = groups
        .iter()
        .enumerate()
        .filter_map(|(index, _)| projection_id(registry, projection_keys, ProjectionNodeRef::Group(index)).map(|id| (id, index)))
        .collect();
    let mut out = Vec::new();
    for root in hierarchy.roots() {
        if let HierarchyNode::Item(id) = root {
            if let Some((group_index, item_index)) = item_indices.get(id) {
                if let Some(segment) = out.last_mut().filter(|segment: &&mut GroupRenderSegment| segment.root_items) {
                    segment.item_indices.push((*group_index, *item_index));
                } else {
                    out.push(GroupRenderSegment {
                        group_index: *group_index,
                        depth: 0,
                        root_items: true,
                        show_header: false,
                        folder_path: Vec::new(),
                        show_folder_header: false,
                        item_indices: vec![(*group_index, *item_index)],
                    });
                }
            }
        } else {
            let root_index = group_indices.get(root.id()).copied().unwrap_or_default();
            flatten_group(root, 0, root_index, &[], &item_indices, &group_indices, &identities, &mut out);
        }
    }
    registry.retain_keys(&projection_keys.all_keys());
    Some((out, hierarchy, identities))
}
