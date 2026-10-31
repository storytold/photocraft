//! Asset-neutral picker browser model, rendering, and interaction routing.
//!
//! Consumers project their domain assets into `GroupView` and `ItemView`, provide preview painters,
//! and translate `BrowserAction` into domain commands. This module owns the common browser presentation only.

use std::collections::BTreeSet;

use egui::{Align2, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, pos2, vec2};

use crate::pickers::asset_browser_groups::{ROW_INDENT, group_open, row_indent};
use crate::pickers::asset_browser_hierarchy::HierarchyGroup;
use crate::pickers::asset_browser_interaction::{
    AssetDrag, ContextMenuAction, HeaderDropCapabilities, HierarchyDrop, asset_browser_header_drag_source_with_id,
    asset_browser_header_target_with_capabilities,
    asset_browser_take_release,
    asset_browser_item_drag_source_with_id, asset_browser_item_gesture, asset_browser_item_hierarchy_target, asset_browser_item_target,
    context_menu_target, draw_group_context_menu, draw_item_context_menu, group_append_hit, group_header_drop_hit, header_drop_before, hierarchy_group_drop,
    card_drop_hit, card_drop_hits, group_gap_drop_hits, group_gap_upper_anchor, item_drop_after, item_row_edge_targets, paint_group_insertion_line, root_append_hit,
};
use crate::pickers::asset_browser_items::{item_info as asset_browser_item_info, list_row as asset_browser_list_row};
use crate::pickers::picker_state::escape_hierarchy_key_part;
use crate::pickers::picker_state::{HierarchyOrderEntry, ItemRenameState, PresetUi};
use crate::pickers::picker_browser_hierarchy::{HierarchyIdentity, commit_hierarchy_move};
use crate::pickers::picker_browser_model::{ContainerId, HierarchyNode, NodeId, PickerBrowserHierarchy};
use crate::pickers::picker_browser_dragging::{DragSession, DropCommitResult, DropHitRegion, DropMoveIntent, PointerRegion, arbitrate_hit_index, paint_candidate};
use crate::theme::Tokens;

pub(crate) const MIN_THUMB: f32 = 20.0;
const GROUP_HEADER_HEIGHT: f32 = 30.0;
const GROUP_DRAG_GHOST_OPACITY: f32 = 0.4;
const HIT_DEBUG_LABEL_HEIGHT: f32 = 9.0;

fn hit_debug_label_position(hit: &DropHitRegion) -> Pos2 {
    let label_y = if hit.hit_rect.height() <= 6.1 {
        match hit.region {
            // The upper label must not be placed above the gap: that part can be clipped by
            // the preceding group's row. Stack both labels below the gap instead.
            PointerRegion::TrailingEdge => hit.hit_rect.bottom() + 3.0,
            PointerRegion::LeadingEdge => hit.hit_rect.bottom() + HIT_DEBUG_LABEL_HEIGHT,
            _ => hit.hit_rect.top() + 2.0,
        }
    } else {
        hit.hit_rect.top() + 2.0
    };
    pos2(hit.hit_rect.left() + 3.0, label_y)
}

/// Asset-neutral item identity and label consumed by the shared picker browser.
pub(crate) struct ItemView {
    /// Stable consumer-defined key used for selection, emitted events, and persisted sibling
    /// ordering. Keys must be unique within their containing group.
    pub(crate) key: String,
    /// Display label shown in the item card or list row.
    pub(crate) name: String,
    /// Optional nested-group path beneath the containing root group.
    pub(crate) group_path: Vec<String>,
}

/// Asset-neutral group and its ordered items, ready for rendering by the shared browser.
pub(crate) struct GroupView {
    /// Group label and stable identity used for collapse state and group actions. Names are unique within a picker.
    pub(crate) name: String,
    /// Items in the order they should appear within this group.
    pub(crate) items: Vec<ItemView>,
}

/// Test helper for the shared browser's root-group/nested-group/item query contract.
#[cfg(test)]
pub(crate) fn filter_groups(groups: &[GroupView], query: &str) -> Vec<GroupView> {
    let query = query.trim().to_lowercase();
    groups
        .iter()
        .filter_map(|group| {
            let group_matches = group_matches_query(group, &query);
            let items: Vec<ItemView> = group
                .items
                .iter()
                .filter(|item| item_visible_for_query(item, &query, group_matches))
                .map(|item| ItemView { key: item.key.clone(), name: item.name.clone(), group_path: item.group_path.clone() })
                .collect();
            group_visible_for_query(&query, group_matches, !items.is_empty()).then(|| GroupView { name: group.name.clone(), items })
        })
        .collect()
}

fn group_matches_query(group: &GroupView, query: &str) -> bool {
    !query.is_empty() && group.name.to_lowercase().contains(query)
}

fn item_matches_query(item: &ItemView, query: &str) -> bool {
    !query.is_empty() && (item.name.to_lowercase().contains(query) || item.group_path.iter().any(|part| part.to_lowercase().contains(query)))
}

fn group_visible_for_query(query: &str, group_matches: bool, has_matching_items: bool) -> bool {
    query.is_empty() || group_matches || has_matching_items
}

fn item_visible_for_query(item: &ItemView, query: &str, group_matches: bool) -> bool {
    query.is_empty() || group_matches || item_matches_query(item, query)
}

struct GroupRenderSegment {
    group_index: usize,
    depth: usize,
    show_header: bool,
    folder_path: Vec<String>,
    show_folder_header: bool,
    item_indices: Vec<usize>,
}

/// Flatten the shared sibling sequences into render segments. Cards and child groups are walked in
/// their canonical order; adjacent cards are batched for grid layout, and every group is emitted at
/// exactly the position represented by its `Group` token.
fn build_hierarchy_projection(
    groups: &[GroupView],
    display: &[(usize, usize)],
    parents: &std::collections::BTreeMap<String, String>,
    orders: &std::collections::BTreeMap<String, Vec<HierarchyOrderEntry>>,
) -> Option<(Vec<GroupRenderSegment>, PickerBrowserHierarchy, std::collections::BTreeMap<NodeId, HierarchyIdentity>)> {
    fn group_id(name: &str) -> NodeId {
        NodeId::group(name)
    }

    fn item_id(group: &str, key: &str) -> NodeId {
        NodeId::item(group, key)
    }

    fn insert_path_item(
        nodes: &mut Vec<HierarchyNode>,
        root_group: &str,
        parent_path: &[String],
        path: &[String],
        item: HierarchyNode,
        identities: &mut std::collections::BTreeMap<NodeId, HierarchyIdentity>,
    ) {
        let Some((name, rest)) = path.split_first() else {
            nodes.push(item);
            return;
        };
        let full_path: Vec<String> = parent_path.iter().cloned().chain(std::iter::once(name.clone())).collect();
        let id = NodeId::group_path(std::iter::once(root_group.to_owned()).chain(parent_path.iter().cloned()), name.clone());
        let index = nodes.iter().position(|node| node.id() == &id).unwrap_or_else(|| {
            nodes.push(HierarchyNode::group(id.clone(), Vec::new()));
            identities.insert(id.clone(), HierarchyIdentity::Folder { group: root_group.to_owned(), path: full_path.clone() });
            nodes.len() - 1
        });
        if let Some(HierarchyNode::Group { children, .. }) = nodes.get_mut(index) {
            insert_path_item(children, root_group, &full_path, rest, item, identities);
        }
    }

    fn build_group(
        group_index: usize,
        depth: usize,
        groups: &[GroupView],
        display: &[(usize, usize)],
        parents: &std::collections::BTreeMap<String, String>,
        orders: &std::collections::BTreeMap<String, Vec<HierarchyOrderEntry>>,
        visited: &mut BTreeSet<usize>,
        item_indices: &mut std::collections::BTreeMap<NodeId, (usize, usize)>,
        identities: &mut std::collections::BTreeMap<NodeId, HierarchyIdentity>,
        lifted_items: &BTreeSet<(String, String)>,
    ) -> Option<HierarchyNode> {
        let group = groups.get(group_index)?;
        if !visited.insert(group_index) {
            return None;
        }
        let children: Vec<(usize, usize)> = display
            .iter()
            .filter_map(|&(index, child_depth)| {
                let child = groups.get(index)?;
                (child_depth == depth + 1 && parents.get(&child.name).is_some_and(|parent| parent == &group.name)).then_some((index, child_depth))
            })
            .collect();
        let child_names: std::collections::BTreeMap<&str, (usize, usize)> = children
            .iter()
            .filter_map(|&(index, child_depth)| groups.get(index).map(|child| (child.name.as_str(), (index, child_depth))))
            .collect();
        let mut entries = orders.get(&group.name).cloned().unwrap_or_default();
        if !orders.contains_key(&group.name) {
            entries.extend(group.items.iter().filter(|item| !lifted_items.contains(&(group.name.clone(), item.key.clone()))).map(|item| HierarchyOrderEntry::Item(item.key.clone())));
            entries.extend(children.iter().filter_map(|(index, _)| groups.get(*index).map(|child| HierarchyOrderEntry::Group(child.name.clone()))));
        }
        // Saved sequences can be stale or partial. Keep the same deterministic append behavior
        // as the old projection while ensuring every live adapter node is represented once.
        for item in group.items.iter().filter(|item| !lifted_items.contains(&(group.name.clone(), item.key.clone()))) {
            let entry = HierarchyOrderEntry::Item(item.key.clone());
            if !entries.contains(&entry) {
                entries.push(entry);
            }
        }
        for (index, _) in &children {
            if let Some(child) = groups.get(*index) {
                let entry = HierarchyOrderEntry::Group(child.name.clone());
                if !entries.contains(&entry) {
                    entries.push(entry);
                }
            }
        }
        let item_by_key: std::collections::BTreeMap<&str, usize> = group.items.iter().enumerate().filter(|(_, item)| !lifted_items.contains(&(group.name.clone(), item.key.clone()))).map(|(index, item)| (item.key.as_str(), index)).collect();
        let mut nodes = Vec::new();
        for entry in entries {
            match entry {
                HierarchyOrderEntry::Item(key) => {
                    if let Some(&index) = item_by_key.get(key.as_str()) {
                        let item = &group.items[index];
                        let folder_path = &item.group_path[..item.group_path.len().min(crate::pickers::asset_browser_hierarchy::MAX_HIERARCHY_DEPTH)];
                        let id = if folder_path.is_empty() {
                            item_id(&group.name, &key)
                        } else {
                            NodeId::item_in_path(std::iter::once(group.name.clone()).chain(folder_path.iter().cloned()), key.clone())
                        };
                        item_indices.insert(id.clone(), (group_index, index));
                        if folder_path.is_empty() {
                            identities.insert(id.clone(), HierarchyIdentity::Item { group: group.name.clone(), key });
                            nodes.push(HierarchyNode::item(id));
                        } else {
                            identities.insert(id.clone(), HierarchyIdentity::FolderItem { group: group.name.clone(), path: folder_path.to_vec(), key });
                            insert_path_item(&mut nodes, &group.name, &[], folder_path, HierarchyNode::item(id), identities);
                        }
                    }
                }
                HierarchyOrderEntry::Group(name) => {
                    if let Some((index, child_depth)) = child_names.get(name.as_str()).copied()
                        && let Some(child) = build_group(index, child_depth, groups, display, parents, orders, visited, item_indices, identities, lifted_items)
                    {
                        nodes.push(child);
                    }
                }
                // Root-only entries are never valid in a group's child sequence.
                HierarchyOrderEntry::RootItem { .. } => {}
            }
        }
        let id = group_id(&group.name);
        identities.insert(id.clone(), HierarchyIdentity::Group(group.name.clone()));
        Some(HierarchyNode::group(id, nodes))
    }

    fn flatten_group(
        node: &HierarchyNode,
        depth: usize,
        root_group_index: usize,
        folder_path: &[String],
        item_indices: &std::collections::BTreeMap<NodeId, (usize, usize)>,
        group_indices: &std::collections::BTreeMap<NodeId, usize>,
        out: &mut Vec<GroupRenderSegment>,
    ) {
        let group_index = group_indices.get(node.id()).copied().unwrap_or(root_group_index);
        let mut run = Vec::new();
        let mut first = true;
        for child in node.children() {
            match child {
                HierarchyNode::Item(id) => {
                    if let Some((_, item_index)) = item_indices.get(id) {
                        run.push(*item_index);
                    }
                }
                HierarchyNode::Group { .. } => {
                    if !run.is_empty() || first {
                        out.push(GroupRenderSegment {
                            group_index,
                            depth,
                            show_header: folder_path.is_empty() && first,
                            folder_path: folder_path.to_vec(),
                            show_folder_header: !folder_path.is_empty() && first,
                            item_indices: std::mem::take(&mut run),
                        });
                        first = false;
                    }
                    match child.id() {
                        NodeId::GroupPath(path) => {
                            flatten_group(child, depth, root_group_index, path.get(1..).unwrap_or_default(), item_indices, group_indices, out);
                        }
                        _ => {
                            let child_index = group_indices.get(child.id()).copied().unwrap_or(root_group_index);
                            flatten_group(child, depth + 1, child_index, &[], item_indices, group_indices, out);
                        }
                    }
                }
            }
        }
        if !run.is_empty() || first {
            out.push(GroupRenderSegment {
                group_index,
                depth,
                show_header: folder_path.is_empty() && first,
                folder_path: folder_path.to_vec(),
                show_folder_header: !folder_path.is_empty() && first,
                item_indices: run,
            });
        }
    }

    let lifted_items: BTreeSet<(String, String)> = orders.get("").into_iter().flatten().filter_map(|entry| match entry {
        HierarchyOrderEntry::RootItem { group, key } => Some((group.clone(), key.clone())),
        _ => None,
    }).collect();
    let mut visited = BTreeSet::new();
    let mut item_indices = std::collections::BTreeMap::new();
    let mut identities = std::collections::BTreeMap::new();
    let mut roots: Vec<HierarchyNode> = display
        .iter()
        .filter_map(|&(index, depth)| (depth == 0).then_some(index))
        .filter_map(|index| build_group(index, 0, groups, display, parents, orders, &mut visited, &mut item_indices, &mut identities, &lifted_items))
        .collect();
    let root_entries = orders.get("").cloned().unwrap_or_default();
    let group_by_name: std::collections::BTreeMap<&str, usize> = groups.iter().enumerate().map(|(i, g)| (g.name.as_str(), i)).collect();
    let root_group_display_order: Vec<String> = roots.iter().filter_map(|node| match identities.get(node.id()) { Some(HierarchyIdentity::Group(name)) => Some(name.clone()), _ => None }).collect();
    let mut root_groups: std::collections::BTreeMap<String, HierarchyNode> = roots.drain(..).filter_map(|node| {
        let HierarchyIdentity::Group(name) = identities.get(node.id())? else { return None };
        Some((name.clone(), node))
    }).collect();
    let mut ordered_roots = Vec::new();
    for entry in root_entries {
        match entry {
            HierarchyOrderEntry::Group(name) => if let Some(node) = root_groups.remove(&name) { ordered_roots.push(node); },
            // Ordinary Item entries belong to a group sequence; malformed/root legacy values are ignored.
            HierarchyOrderEntry::Item(_) => {}
            HierarchyOrderEntry::RootItem { group: group_name, key } => {
                let Some(&group_index) = group_by_name.get(group_name.as_str()) else { continue };
                let Some(item_index) = groups[group_index].items.iter().position(|item| item.key == key) else { continue };
                let id = item_id(&group_name, &key);
                item_indices.insert(id.clone(), (group_index, item_index));
                identities.insert(id.clone(), HierarchyIdentity::Item { group: group_name.clone(), key: key.clone() });
                ordered_roots.push(HierarchyNode::item(id));
            }
        }
    }
    for name in root_group_display_order {
        if let Some(node) = root_groups.remove(&name) { ordered_roots.push(node); }
    }
    roots = ordered_roots;
    // Adapter group names are stable unique identities; item IDs are typed group/key pairs, so
    // duplicate card keys across groups remain distinct without delimiter encoding.
    let Ok(hierarchy) = PickerBrowserHierarchy::new(roots) else { return None };
    let group_indices: std::collections::BTreeMap<NodeId, usize> = groups.iter().enumerate().map(|(index, group)| (group_id(&group.name), index)).collect();
    let mut out = Vec::new();
    for root in hierarchy.roots() {
        if let HierarchyNode::Item(id) = root {
            if let Some((group_index, item_index)) = item_indices.get(id) {
                out.push(GroupRenderSegment { group_index: *group_index, depth: 0, show_header: false, folder_path: Vec::new(), show_folder_header: false, item_indices: vec![*item_index] });
            }
        } else {
            let root_index = group_indices.get(root.id()).copied().unwrap_or_default();
            flatten_group(root, 0, root_index, &[], &item_indices, &group_indices, &mut out);
        }
    }
    Some((out, hierarchy, identities))
}

#[cfg(test)]
fn group_render_segments(
    groups: &[GroupView],
    display: &[(usize, usize)],
    parents: &std::collections::BTreeMap<String, String>,
    orders: &std::collections::BTreeMap<String, Vec<HierarchyOrderEntry>>,
) -> Vec<GroupRenderSegment> {
    build_hierarchy_projection(groups, display, parents, orders).map(|(segments, _, _)| segments).unwrap_or_default()
}

fn item_occurrence_position(groups: &[GroupView], key: &str) -> Option<(usize, usize)> {
    groups
        .iter()
        .enumerate()
        .find_map(|(group_index, group)| group.items.iter().position(|item| item.key.as_str() == key).map(|item_index| (group_index, item_index)))
}

fn move_item_sibling_order(
    st: &mut PresetUi,
    panel: &str,
    orders: &std::collections::BTreeMap<String, Vec<HierarchyOrderEntry>>,
    source_group: &str,
    target_group: &str,
    key: &str,
    before: Option<&HierarchyOrderEntry>,
) {
    let moving = HierarchyOrderEntry::Item(key.to_string());
    if source_group != target_group {
        if let Some(entries) = orders.get(source_group) {
            let remaining = entries.iter().filter(|entry| *entry != &moving).cloned().collect();
            st.set_hierarchy_order(panel, source_group, remaining);
        }
    }
    let entries = orders.get(target_group).cloned().unwrap_or_default();
    st.move_hierarchy_entry(panel, target_group, entries, moving, before);
}

/// Pick the last eligible target, matching the browser's existing reverse-order drop arbitration.
fn best_drop_target_index<T>(targets: &[T], mut eligible: impl FnMut(&T) -> bool) -> Option<usize> {
    targets.iter().rposition(&mut eligible)
}

/// An item target outranks the indentation-column target when both hit regions overlap. The
/// indentation column must not consume egui's one-shot release payload before that arbitration.
fn best_indent_drop_target_index<T>(targets: &[T], item_target_wins: bool, eligible: impl FnMut(&T) -> bool) -> Option<usize> {
    (!item_target_wins).then(|| best_drop_target_index(targets, eligible)).flatten()
}

/// Find the next hierarchy sibling at the destination level after a card. A following direct
/// preset is a stable item anchor; a following child group is a sibling-group anchor. This keeps
/// a drop immediately after a card ahead of a nested group that happens to follow it in storage.
fn item_hierarchy_insertion_anchor(
    items: &[ItemView],
    index: usize,
    group_path: &[String],
    after: bool,
    moving_group_path: Option<&[String]>,
) -> (Option<String>, Option<String>) {
    if !after {
        return (None, items.get(index).map(|item| item.key.clone()));
    }
    // A group is temporarily still present in the projected item list while the drop is
    // resolved. Do not choose that same group as its own `before` anchor: the engine removes
    // the moving subtree before validating destination siblings, so such an anchor is stale.
    let next = items
        .iter()
        .skip(index.saturating_add(1))
        .find(|item| item.group_path.starts_with(group_path) && !moving_group_path.is_some_and(|moving| item.group_path.starts_with(moving)));
    match next {
        Some(item) if item.group_path == group_path => (None, Some(item.key.clone())),
        Some(item) => (item.group_path.get(group_path.len()).cloned(), None),
        None => (None, None),
    }
}

/// Generic browser interactions; each asset consumer translates these into its own commands.
#[derive(Clone)]
pub(crate) enum BrowserAction {
    /// Select an item without activating or dismissing its picker.
    Select(String),
    /// Select one item occurrence, qualified by its owning root group.
    SelectItem { group: String, group_path: Vec<String>, key: String },
    /// Activate an item, typically on a double-click.
    Activate(String),
    /// Activate one item occurrence, qualified by its owning root group.
    ActivateItem { group: String, group_path: Vec<String>, key: String },
    /// Drop an item onto a canvas position in screen coordinates.
    Drop(String, Pos2),
    /// Rename an item from its stable key to a new display name.
    Rename(String, String),
    RenameItem { group: String, group_path: Vec<String>, key: String, text: String },
    /// Delete the item identified by its stable key.
    Delete(String),
    DeleteItem { group: String, group_path: Vec<String>, key: String },
    /// Move an item into the named group.
    Move(String, String),
    /// Move an item into a group at a specific final insertion index.
    MoveItem(String, String, Option<usize>),
    /// Rename a group from its current name to a new name.
    RenameGroup(String, String),
    /// Delete a group and its contents.
    DeleteGroup(String),
    /// Rename a nested group identified by its owning root group and full path.
    RenameNestedGroup { group: String, group_path: Vec<String>, text: String },
    /// Delete a nested group and its contents, identified by its owning root group and full path.
    DeleteNestedGroup { group: String, group_path: Vec<String> },
    /// Request creation of a new group.
    NewGroup,
    /// Request creation of a new item.
    New,
    /// Move an item into a nested group; an empty path means directly into the root group.
    /// `before` identifies the destination sibling to insert before; `None` appends.
    MoveItemToGroup { key: String, group: String, group_path: Vec<String>, before: Option<String> },
    /// Reorder a root or nested group among siblings. An empty `group_path` identifies the root group;
    /// `before=None` appends it after its siblings.
    MoveGroup { group: String, group_path: Vec<String>, before: Option<String> },
    /// Move a root or nested group into a destination hierarchy. `into=None` promotes it to a
    /// top-level group; an empty destination path means directly inside that group. `before=None`
    /// appends within the destination.
    MoveInto { group: String, group_path: Vec<String>, into: Option<(String, Vec<String>)>, before: Option<String>, before_item: Option<String> },
}

fn hierarchy_action(drop: HierarchyDrop) -> BrowserAction {
    match drop {
        HierarchyDrop::MoveGroup { group, group_path, before } => BrowserAction::MoveGroup { group, group_path, before },
        HierarchyDrop::MoveInto { group, group_path, into, before } => BrowserAction::MoveInto { group, group_path, into, before, before_item: None },
    }
}

fn source_node_for_drag(drag: &AssetDrag, panel: &str, identities: &std::collections::BTreeMap<NodeId, HierarchyIdentity>) -> Option<NodeId> {
    if let AssetDrag::CanonicalGroup { scope, node_id } = drag {
        return (scope == panel && matches!(identities.get(node_id), Some(HierarchyIdentity::Group(_) | HierarchyIdentity::Folder { .. })))
            .then(|| node_id.clone());
    }
    if let AssetDrag::Preset { scope, key, node_id } = drag {
        return node_id
            .as_ref()
            .filter(|node_id| {
                scope == panel && matches!(identities.get(*node_id), Some(HierarchyIdentity::Item { key: identity_key, .. } | HierarchyIdentity::FolderItem { key: identity_key, .. }) if identity_key == key)
            })
            .cloned();
    }
    identities.iter().find_map(|(id, identity)| {
        let matches = match (drag, identity) {
            (AssetDrag::Group { scope, name, .. }, HierarchyIdentity::Group(group)) => scope == panel && name == group,
            _ => false,
        };
        matches.then(|| id.clone())
    })
}

fn item_node_for_identity(group: &str, group_path: &[String], key: &str, identities: &std::collections::BTreeMap<NodeId, HierarchyIdentity>) -> Option<NodeId> {
    identities.iter().find_map(|(id, identity)| {
        let matches = matches!(identity,
            HierarchyIdentity::Item { group: item_group, key: item_key } if group_path.is_empty() && item_group == group && item_key == key
        ) || matches!(identity,
            HierarchyIdentity::FolderItem { group: item_group, path, key: item_key } if !group_path.is_empty() && item_group == group && path == group_path && item_key == key
        );
        matches.then(|| id.clone())
    })
}

fn group_is_descendant(parents: &std::collections::BTreeMap<String, String>, source: &str, target: &str) -> bool {
    is_group_descendant(parents, source, target)
}

fn is_group_descendant(parents: &std::collections::BTreeMap<String, String>, source: &str, target: &str) -> bool {
    let mut current = target;
    let mut seen = BTreeSet::new();
    while seen.insert(current.to_string()) {
        let Some(parent) = parents.get(current) else { return false };
        if parent == source {
            return true;
        }
        current = parent;
    }
    false
}

#[cfg(test)]
mod group_drop_tests {
    use super::is_group_descendant;
    use std::collections::BTreeMap;

    #[test]
    fn rejects_dropping_a_group_into_any_descendant() {
        let parents = BTreeMap::from([
            ("Child".to_string(), "Parent".to_string()),
            ("Grandchild".to_string(), "Child".to_string()),
            ("Other".to_string(), "Elsewhere".to_string()),
        ]);

        assert!(is_group_descendant(&parents, "Parent", "Child"));
        assert!(is_group_descendant(&parents, "Parent", "Grandchild"));
        assert!(!is_group_descendant(&parents, "Parent", "Other"));
        assert!(!is_group_descendant(&parents, "Parent", "Parent"));
    }
}

use crate::pickers::asset_browser_groups::{header as asset_browser_header, inset_row_rect, paint as paint_asset_browser_header};

/// Browser-specific layout and presentation, supplied by each asset consumer.
/// `flush` lays the grid out as Photoshop's Swatches panel: the cells touch, square-cornered and
/// without a border of their own, so no screen is spent on gaps (#5103); gradients, patterns and
/// the other preset grids keep their [`GRID_GAP`] and card borders.
#[derive(Clone, Copy)]
pub(crate) struct BrowserOptions {
    /// Canvas hit region used to distinguish drops onto the canvas from drops into the browser.
    pub(crate) canvas: Rect,
    /// Use touching, square-cornered grid cells rather than spaced preset cards.
    pub(crate) flush: bool,
    /// Centre a partially filled grid row in the available item area.
    pub(crate) center_grid: bool,
    /// Left inset for grid items before nested-group indentation is added.
    pub(crate) grid_indent: f32,
    /// Start every group expanded rather than only the first group.
    pub(crate) all_groups_open: bool,
    /// Use nested menu submenus for context actions inside a parent popup.
    pub(crate) compact_context_menus: bool,
    /// Optional fixed grid-cell dimensions for consumers with custom preview geometry.
    pub(crate) item_size: Option<egui::Vec2>,
    /// Optional allocated list-row height. Defaults to 24px when not set.
    pub(crate) list_item_height: Option<f32>,
    /// Emit group moves as domain actions instead of applying transient browser ordering.
    pub(crate) domain_hierarchy_actions: bool,
}

/// Features that can be enabled independently for a particular browser consumer.
#[derive(Clone, Copy)]
pub(crate) struct BrowserCapabilities {
    /// Whether presets can be dragged to the canvas or other browser targets.
    pub(crate) item_drag: bool,
    /// Whether a dragged preset can be dropped onto a group header.
    pub(crate) item_drop: bool,
    /// Whether group headers can be reordered by dragging.
    pub(crate) group_drag: bool,
    /// Whether clicking an item emits a selection event.
    pub(crate) selection: bool,
    /// Whether activating an item (for example, double-clicking) emits an activation event.
    pub(crate) activation: bool,
    /// Use the canonical hierarchy resolver as the sole drag-target and release owner.
    /// Keep this off when an adapter's domain hierarchy cannot be losslessly projected into
    /// `PickerBrowserHierarchy`; those consumers retain the legacy path until the projection and
    /// domain commit contract cover every folder/group operation they expose.
    pub(crate) canonical_hierarchy_drag: bool,
}

impl Default for BrowserCapabilities {
    fn default() -> Self {
        Self { item_drag: true, item_drop: true, group_drag: true, selection: true, activation: true, canonical_hierarchy_drag: false }
    }
}

/// Optional consumer-provided painter for a group row: UI, row rectangle, group, open, hovered, opacity.
pub(crate) type PaintGroup = dyn FnMut(&egui::Ui, Rect, &GroupView, bool, bool, f32);
pub(crate) type PaintListItem<'a> = dyn FnMut(&egui::Ui, Rect, usize, usize, bool) + 'a;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ItemPaintState {
    Item { selected: bool },
    DragGhost,
}
pub(crate) type PaintItem<'a> = dyn FnMut(&egui::Ui, Rect, usize, usize, ItemPaintState) + 'a;

/// Consumer adapter for the generic browser. Each asset library projects its own identity and
/// labels into `GroupView`/`ItemView`, paints its own preview, and handles returned `BrowserAction` values with
/// its own commands. The browser never knows about gradient, pattern, swatch, style, or brush data.
pub(crate) struct BrowserAdapter<'a> {
    /// Stable key for this consumer's selection, sizing, and group-collapse state.
    pub(crate) panel: &'a str,
    /// Consumer-projected groups and ordered items to display.
    pub(crate) groups: &'a [GroupView],
    /// Grid geometry and consumer-specific layout options.
    pub(crate) options: BrowserOptions,
    /// Toggleable interactions supported by this consumer.
    pub(crate) capabilities: BrowserCapabilities,
    /// Consumers may replace the standard item menu or omit it entirely. The browser only owns
    /// the hit target; menu contents and emitted generic requests live behind this adapter seam.
    pub(crate) item_context_menu: Option<ItemContextMenu>,
    /// Consumer-specific group menu policy, attached to the shared group hit target.
    pub(crate) group_context_menu: Option<GroupContextMenu>,
    /// Optional context-menu policy for nested group headers. Receives the owning root group and full group path.
    /// `None` intentionally exposes no nested-group rename/delete menu for adapters without a
    /// domain command implementation; only Brushes currently supplies this specialised policy.
    pub(crate) nested_group_context_menu: Option<NestedGroupContextMenu>,
    /// Authoritative adapter commit for a validated canonical move. The callback receives the
    /// exact source identity and destination position, and owns all domain persistence (including
    /// path-aware folder/item moves). Returning an error rejects the move and is shown to the user.
    /// When absent, the shared browser uses its legacy hierarchy persistence for Group/Item trees.
    pub(crate) canonical_hierarchy_move: Option<&'a mut dyn FnMut(&DropMoveIntent) -> Result<(), String>>,
    /// Domain commit for simple flat item moves. Receives the source group/key and the exact
    /// destination group/final insertion index computed by the canonical hierarchy resolver.
    pub(crate) hierarchy_item_move: Option<&'a mut dyn FnMut(&str, &str, &str, usize) -> Result<(), String>>,
    /// Optional override for group-row visuals; the default is a group row with disclosure.
    pub(crate) paint_group: Option<&'a mut PaintGroup>,
    /// Consumer-specific item painter. The state distinguishes regular selection styling from a drag ghost.
    pub(crate) paint_item: &'a mut PaintItem<'a>,
    /// Optional specialised list-row painter. It paints the full row; shared code retains all hit targets.
    pub(crate) paint_list_item: Option<&'a mut PaintListItem<'a>>,
}

/// Consumer callback that appends generic events for actions offered on an item.
pub(crate) type ItemContextMenu = fn(&mut egui::Ui, &mut PresetUi, &str, &ItemView, &GroupView, &[String], &mut Vec<BrowserAction>);
/// Consumer callback that appends generic events for actions offered on a group header.
pub(crate) type GroupContextMenu = fn(&mut egui::Ui, &mut PresetUi, &str, &GroupView, &mut Vec<BrowserAction>);
/// Consumer callback for rename/delete operations on a nested group, with its full path.
pub(crate) type NestedGroupContextMenu = fn(&mut egui::Ui, &mut PresetUi, &str, &GroupView, &[String], &mut Vec<BrowserAction>);

/// Default menu policy for the standard preset panels. Keeping this separate from the browser
/// lets specialised consumers supply their own operations without forking the shared hit targets.
pub(crate) fn standard_item_context_menu(
    ui: &mut egui::Ui,
    st: &mut PresetUi,
    panel: &str,
    item: &ItemView,
    group: &GroupView,
    group_names: &[String],
    ev: &mut Vec<BrowserAction>,
) {
    match draw_item_context_menu(ui, tl!("Rename…"), tl!("Delete"), Some((tl!("Move to"), group_names, &group.name))) {
        Some(ContextMenuAction::Rename) => st.item_renaming = Some(ItemRenameState { panel: panel.to_string(), group: group.name.clone(), group_path: item.group_path.clone(), key: item.key.clone(), text: item.name.clone() }),
        Some(ContextMenuAction::Delete) => ev.push(BrowserAction::DeleteItem { group: group.name.clone(), group_path: item.group_path.clone(), key: item.key.clone() }),
        Some(ContextMenuAction::MoveTo(group)) => ev.push(BrowserAction::Move(item.key.clone(), group)),
        None => {}
    }
}

/// Apply the shared rename/delete policy for a standard browser group context menu.
pub(crate) fn standard_group_context_menu(ui: &mut egui::Ui, st: &mut PresetUi, panel: &str, group: &GroupView, ev: &mut Vec<BrowserAction>) {
    match draw_group_context_menu(ui, tl!("Rename Group…"), tl!("Delete Group")) {
        Some(ContextMenuAction::Rename) => st.renaming = Some((panel.to_string(), format!("group:{}", group.name), group.name.clone())),
        Some(ContextMenuAction::Delete) => ev.push(BrowserAction::DeleteGroup(group.name.clone())),
        _ => {}
    }
}

/// Show context actions for compact-picker targets at the pointer position.
fn compact_context_menu_target(ui: &mut egui::Ui, resp: egui::Response, menu: impl FnOnce(&mut egui::Ui)) {
    // Compact pickers live inside an egui menu popup. A separate context popup is treated as
    // outside that parent and dismisses it. Use the same nested-SubMenu mechanism as the gear.
    let submenu_id = egui::containers::menu::SubMenu::id_from_widget_id(resp.id);
    // Do not set `open_item` before calling `SubMenu::show`: MenuState validates that item
    // against the previous frame's visible submenu, so a newly requested submenu gets cleared.
    // Showing it on the right-click frame lets SubMenu register itself normally and persist.
    let right_clicked = resp.secondary_clicked();
    let open = egui::containers::menu::MenuState::from_ui(ui, |state, _| state.open_item == Some(submenu_id));
    if right_clicked || open {
        egui::containers::menu::SubMenu::new().show(ui, &resp, menu);
    }
}

/// Space between preset cards, and the grid's inset from the panel's left edge.
const GRID_GAP: f32 = 0.0;
type GroupRelease = (AssetDrag, String, Vec<String>, bool);

/// Draw only the reusable library region. It owns groups/items, selection, drag/drop and item/group
/// context targets, but not the containing panel's optional controls or footer.
///
/// `allocated_height` is reserved by the caller or picker shell; the browser owns only the
/// scrolling library region, not search, recents, panel menus, footer controls, or command dispatch.
pub(crate) fn browser(ui: &mut egui::Ui, st: &mut PresetUi, adapter: &mut BrowserAdapter<'_>, allocated_height: f32) -> Vec<BrowserAction> {
    let t = Tokens::get(ui.ctx());
    let panel = adapter.panel;
    let query = st.search.get(panel).map(|query| query.trim().to_lowercase()).unwrap_or_default();
    let groups = adapter.groups;
    let BrowserOptions {
        canvas,
        flush,
        center_grid,
        grid_indent,
        all_groups_open,
        compact_context_menus,
        item_size,
        list_item_height,
        domain_hierarchy_actions,
    } = adapter.options;
    let capabilities = adapter.capabilities;
    // Canonical hierarchy drag/drop is opt-in: each adapter enables it only after its projected
    // tree and domain commit callback can represent every hierarchy operation it exposes.
    let canonical_hierarchy_drag = capabilities.canonical_hierarchy_drag;
    let item_context_menu = adapter.item_context_menu;
    let group_context_menu = adapter.group_context_menu;
    let nested_group_context_menu = adapter.nested_group_context_menu;
    let canonical_hierarchy_move = &mut adapter.canonical_hierarchy_move;
    let hierarchy_item_move = &mut adapter.hierarchy_item_move;
    let paint_group = &mut adapter.paint_group;
    let paint_item = &mut adapter.paint_item;
    let paint_list_item = &mut adapter.paint_list_item;
    let max_h = allocated_height;
    let mut ev = Vec::new();
    let size = *st.sizes.get(panel).unwrap_or(&40.0);
    let list = size <= MIN_THUMB + 0.5;
    let rename_row_top = ui.cursor().top();
    let item_rename = st.item_renaming.clone().filter(|rename| rename.panel == panel);
    let nested_group_rename = st.group_renaming.clone().filter(|rename| rename.panel == panel);
    let has_rename_row = item_rename.is_some() || nested_group_rename.is_some() || st.renaming.as_ref().is_some_and(|r| r.0 == panel);
    // Rename field (Photoshop opens a small dialog; inline keeps the panel modeless).
    if let Some(mut rename) = item_rename {
        if st.renaming.as_ref().is_some_and(|r| r.0 == panel) { st.renaming = None; }
        let mut text = rename.text.clone();
        let mut done = None;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Name:")).color(t.text_dim).size(11.5));
            let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(ui.available_width() - 4.0));
            if !r.has_focus() && !r.lost_focus() { r.request_focus(); }
            if r.lost_focus() { done = Some(ui.input(|i| i.key_pressed(egui::Key::Enter))); }
        });
        rename.text = text.clone();
        st.item_renaming = Some(rename.clone());
        if let Some(ok) = done {
            st.item_renaming = None;
            if ok && !text.trim().is_empty() { ev.push(BrowserAction::RenameItem { group: rename.group, group_path: rename.group_path, key: rename.key, text: text.trim().to_string() }); }
        }
    } else if let Some(mut rename) = nested_group_rename {
        // The typed nested-group target is authoritative while this editor is active. Remove any stale
        // legacy target for the same panel so it cannot reappear after commit or cancellation.
        if st.renaming.as_ref().is_some_and(|r| r.0 == panel) {
            st.renaming = None;
        }
        let mut text = rename.text.clone();
        let mut done = None;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Name:")).color(t.text_dim).size(11.5));
            let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(ui.available_width() - 4.0));
            if !r.has_focus() && !r.lost_focus() {
                r.request_focus();
            }
            if r.lost_focus() {
                done = Some(ui.input(|i| i.key_pressed(egui::Key::Enter)));
            }
        });
        rename.text = text.clone();
        st.group_renaming = Some(rename.clone());
        if let Some(ok) = done {
            st.group_renaming = None;
            if ok && !text.trim().is_empty() {
                ev.push(BrowserAction::RenameNestedGroup { group: rename.group, group_path: rename.group_path, text: text.trim().to_string() });
            }
        }
    } else if let Some((p, key, mut text)) = st.renaming.clone().filter(|r| r.0 == panel) {
        // Starting a regular item/group rename supersedes any pending nested-group target for this panel.
        if st.group_renaming.as_ref().is_some_and(|rename| rename.panel == panel) {
            st.group_renaming = None;
        }
        let mut done = None;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Name:")).color(t.text_dim).size(11.5));
            let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(ui.available_width() - 4.0));
            if !r.has_focus() && !r.lost_focus() {
                r.request_focus();
            }
            if r.lost_focus() {
                done = Some(ui.input(|i| i.key_pressed(egui::Key::Enter)));
            }
        });
        st.renaming = Some((p, key.clone(), text.clone()));
        if let Some(ok) = done {
            st.renaming = None;
            if ok && !text.trim().is_empty() {
                let text = text.trim().to_string();
                match key.strip_prefix("group:") {
                    Some(g) => ev.push(BrowserAction::RenameGroup(g.to_string(), text)),
                    None => ev.push(BrowserAction::Rename(key, text)),
                }
            }
        }
    }
    // The rename row is part of the browser's allocated height, not extra height above it. The
    // scroll area's minimum must exclude the row (and its layout spacing), or a short dock can
    // push the shell-owned footer below the panel.
    let rename_row_height = if has_rename_row { (ui.cursor().top() - rename_row_top).max(0.0) } else { 0.0 };
    let scroll_height = (max_h - rename_row_height).max(0.0);
    let mut headers: Vec<(usize, Rect, String)> = Vec::new();
    let mut indent_drop_targets: Vec<(String, egui::Response)> = Vec::new();
    let mut indent_release: Option<(String, String)> = None;
    let mut group_content_areas: Vec<(String, Rect)> = Vec::new();
    let mut gap_headers: Vec<(NodeId, Rect, usize, bool)> = Vec::new();
    let mut expanded_groups = BTreeSet::new();
    let mut dropped: Option<(String, Pos2)> = None;
    let mut browser_item_drop = false;
    // Group drag (#3658): the order the groups are drawn in, and what the pointer is doing this
    // frame (the insertion line and the drop are resolved once every header is known).
    let mut drag: Option<(usize, Pos2)> = None;
    let mut released: Option<(usize, Pos2)> = None;
    let mut group_releases: Vec<GroupRelease> = Vec::new();
    let canonical_drop_hits = std::cell::RefCell::new(Vec::<(DropHitRegion, egui::Response)>::new());
    let non_root_hierarchy_target_hovered = std::cell::Cell::new(false);
    let group_names: Vec<String> = groups.iter().map(|g| g.name.clone()).collect();
    // Project the canonical mixed sibling sequences into the transient validated browser tree.
    let display = if domain_hierarchy_actions {
        groups.iter().enumerate().map(|(index, _)| (index, 0)).collect()
    } else {
        st.nested_display_order(panel, &group_names)
    };
    let parents = if domain_hierarchy_actions { std::collections::BTreeMap::new() } else { st.group_parents(panel) };
    let mut hierarchy_orders = std::collections::BTreeMap::new();
    let root_entries = if domain_hierarchy_actions { Vec::new() } else { st.hierarchy_order.get(panel).and_then(|orders| orders.get("")).cloned().unwrap_or_default() };
    let lifted_items: BTreeSet<(String, String)> = root_entries.iter().filter_map(|entry| match entry {
        HierarchyOrderEntry::RootItem { group, key } => Some((group.clone(), key.clone())),
        _ => None,
    }).collect();
    for group in groups {
        let child_groups: Vec<String> = display
            .iter()
            .filter_map(|(child_index, _)| {
                let child = groups.get(*child_index)?;
                (parents.get(&child.name).is_some_and(|parent| parent == &group.name)).then(|| child.name.clone())
            })
            .collect();
        let item_keys: Vec<String> = group.items.iter().map(|item| item.key.clone()).filter(|key| !lifted_items.contains(&(group.name.clone(), key.clone()))).collect();
        let entries = if domain_hierarchy_actions {
            group.items.iter().map(|item| HierarchyOrderEntry::Item(item.key.clone())).collect()
        } else {
            st.ordered_hierarchy_entries(panel, &group.name, &item_keys, &child_groups)
        };
        hierarchy_orders.insert(group.name.clone(), entries);
    }
    hierarchy_orders.insert("".into(), root_entries);
    let Some((render_segments, mut hierarchy, hierarchy_identities)) = build_hierarchy_projection(groups, &display, &parents, &hierarchy_orders) else {
        return ev;
    };
    // The browser is the flexible middle region of a picker panel. Keep it at the allocated
    // height even when the library is short; otherwise the footer floats up and each consumer's
    // panel geometry depends on how many presets happen to be installed.
    let scroll = egui::ScrollArea::vertical()
        .id_salt(("preset-scroll", panel))
        .max_height(scroll_height)
        .min_scrolled_height(scroll_height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
            // Keep drag targets reachable when a preset or hierarchy node is dragged beyond the
            // current viewport. This behavior belongs to the shared scrollable browser, not to a
            // particular asset consumer.
            let dragging = egui::DragAndDrop::has_payload_of_type::<AssetDrag>(ui.ctx()) && ui.input(|i| i.pointer.primary_down());
            let delta = crate::panels::layer_drag_edge_scroll(ui.ctx().pointer_interact_pos(), ui.clip_rect(), dragging, ui.input(|i| i.stable_dt));
            if delta != 0.0 {
                ui.scroll_with_delta(vec2(0.0, delta));
                ui.ctx().request_repaint();
            }
            let mut hidden_groups = BTreeSet::new();
            let mut hidden_folders = BTreeSet::<(String, Vec<String>)>::new();
            // Resolve hierarchy-on-card candidates across every visible group segment so a
            // group drag can show only one insertion line in the entire shared browser.
            let mut item_hierarchy_targets: Vec<(egui::Response, Rect, bool, bool, usize, String, Vec<String>)> = Vec::new();
            for segment in &render_segments {
                let gi = segment.group_index;
                let depth = segment.depth;
                let Some(g) = groups.get(gi) else { continue };
                let group_matches = group_matches_query(g, &query);
                let has_matching_items = g.items.iter().any(|item| item_matches_query(item, &query));
                if !group_visible_for_query(&query, group_matches, has_matching_items) {
                    continue;
                }
                let parent = parents.get(&g.name);
                if segment.show_header && parent.is_some_and(|p| hidden_groups.contains(p)) {
                    hidden_groups.insert(g.name.clone());
                    continue;
                }
                if hidden_groups.contains(&g.name) && !segment.show_header {
                    continue;
                }
                if !segment.folder_path.is_empty()
                    && (1..=segment.folder_path.len()).any(|length| hidden_folders.contains(&(g.name.clone(), segment.folder_path[..length].to_vec())))
                {
                    continue;
                }
                // A non-empty query should reveal matching items even when their group was
                // collapsed before the search began.
                let open = !segment.show_header || !query.is_empty() || group_open(st, panel, gi, &g.name, all_groups_open);
                if !open {
                    hidden_groups.insert(g.name.clone());
                } else {
                    expanded_groups.insert(g.name.clone());
                }
                if segment.show_header {
                // Reserve six real pixels between group rows: two from normal row spacing and
                // four explicit pixels for reliable insertion hit targets.
                if !gap_headers.is_empty() {
                    ui.add_space(4.0);
                }
                let (hr, hresp) = if let Some(paint) = paint_group.as_deref_mut() {
                    let (allocated_rect, allocated) = ui.allocate_exact_size(vec2(ui.available_width(), GROUP_HEADER_HEIGHT), Sense::hover());
                    let r = Rect::from_min_max(pos2(ui.clip_rect().left(), allocated_rect.top()), pos2(allocated_rect.right(), allocated_rect.bottom()));
                    let response = ui.interact(inset_row_rect(r, depth), allocated.id, Sense::click_and_drag());
                    paint(ui, r, g, open, response.hovered() || response.dragged(), 1.0);
                    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
                    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::CollapsingHeader, true, &g.name));
                    (r, response)
                } else {
                    asset_browser_header(ui, &g.name, open, depth, None, GROUP_HEADER_HEIGHT)
                };
                if capabilities.group_drag && hresp.is_pointer_button_down_on() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                } else if capabilities.group_drag && hresp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                let header_rect = inset_row_rect(hr, depth);
                gap_headers.push((NodeId::group(&g.name), header_rect, depth, open));
                let header_released = if canonical_hierarchy_drag {
                    if capabilities.group_drag {
                        asset_browser_header_drag_source_with_id(&hresp, panel, NodeId::group(&g.name));
                    }
                    let hit_index = canonical_drop_hits.borrow().len();
                    canonical_drop_hits.borrow_mut().push((group_header_drop_hit(header_rect, Pos2::ZERO, NodeId::group(&g.name), depth, hit_index), hresp.clone()));
                    None
                } else {
                    asset_browser_header_target_with_capabilities(
                        ui,
                        &hresp,
                        header_rect,
                        panel,
                        &g.name,
                        &[],
                        HeaderDropCapabilities {
                            item_drop: capabilities.item_drop,
                            group_drag: capabilities.group_drag,
                            reject_group_drop: hresp.dnd_hover_payload::<AssetDrag>().is_some_and(|payload| {
                                matches!(payload.as_ref(), AssetDrag::Group { scope, name, path } if scope == panel && path.is_empty() && group_is_descendant(&parents, name, &g.name))
                            }),
                        },
                    )
                };
                if hresp.clicked() {
                    let k = format!("{panel}/{}", escape_hierarchy_key_part(&g.name));
                    if !st.toggled.remove(&k) {
                        st.toggled.insert(k);
                    }
                }
                if let Some(menu) = group_context_menu {
                    if compact_context_menus {
                        compact_context_menu_target(ui, hresp.clone(), |ui| menu(ui, st, panel, g, &mut ev));
                    } else {
                        context_menu_target(hresp.clone(), |ui| menu(ui, st, panel, g, &mut ev));
                    }
                }
                if let Some(dragged) = header_released {
                    match dragged {
                        AssetDrag::Preset { scope, key, .. } if scope.as_str() == panel => {
                            ev.push(BrowserAction::MoveItem(key.clone(), g.name.clone(), Some(0)));
                            browser_item_drop = true;
                            if !domain_hierarchy_actions
                                && let Some((source_index, _)) = item_occurrence_position(groups, &key)
                                && let Some(source_group) = groups.get(source_index)
                            {
                                move_item_sibling_order(st, panel, &hierarchy_orders, &source_group.name, &g.name, &key, hierarchy_orders.get(&g.name).and_then(|entries| entries.first()));
                            }
                        }
                        // Top-level group source headers are handled once by the final release
                        // resolver below, which also knows the between-header insertion order.
                        AssetDrag::Group { scope, path, .. } if scope.as_str() == panel && path.is_empty() && domain_hierarchy_actions => {}
                        AssetDrag::Group { scope, name, path } if scope.as_str() == panel && domain_hierarchy_actions => {
                            let edge = ui.ctx().pointer_interact_pos().is_some_and(|p| header_drop_before(hr, p));
                            if let Some(action) = hierarchy_group_drop(&name, &path, &g.name, &[], edge) {
                                ev.push(hierarchy_action(action));
                            }
                        }
                        _ => {}
                    }
                }
                if capabilities.group_drag
                    && hresp.dragged()
                    && let Some(p) = ui.ctx().pointer_interact_pos()
                {
                    drag = Some((gi, p));
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                    // Reuse the actual header painter on a tooltip layer so the drag ghost stays
                    // visually identical to the painted row, including consumer-specific styling.
                    let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::new(("preset-group-drag", panel)));
                    let gr = Rect::from_center_size(p, vec2(hr.width(), hr.height()));
                    let ghost_ui = ui.new_child(egui::UiBuilder::new().layer_id(layer).max_rect(gr));
                    if let Some(paint) = paint_group.as_deref_mut() {
                        paint(&ghost_ui, gr, g, open, false, GROUP_DRAG_GHOST_OPACITY);
                    } else {
                        crate::pickers::asset_browser_groups::paint_with_opacity(&ghost_ui, gr, open, &g.name, depth, None, GROUP_DRAG_GHOST_OPACITY);
                    }
                }
                if capabilities.group_drag
                    && !canonical_hierarchy_drag
                    && hresp.drag_stopped()
                    && let Some(p) = ui.ctx().pointer_interact_pos()
                {
                    released = Some((gi, p));
                }
                headers.push((gi, inset_row_rect(hr, depth), g.name.clone()));
                }
                // Keep each group's assets directly beneath its header. The previous separate
                // content pass placed every swatch after every group header, so expanding a group
                // could not reveal its assets in the expected place.
                let folder_group = HierarchyGroup {
                    path: segment.folder_path.clone(),
                    children: Vec::new(),
                    count: g.items.iter().filter(|item| item.group_path.starts_with(&segment.folder_path)).count(),
                };
                let folder_open = segment.folder_path.is_empty() || {
                    let path = segment.folder_path.iter().map(|part| escape_hierarchy_key_part(part)).collect::<Vec<_>>().join("/");
                    !st.toggled.contains(&format!("{panel}/{}/{}", escape_hierarchy_key_part(&g.name), path))
                };
                if open {
                    let mut group_toggles = Vec::new();
                    {
                        let segment_items: Vec<&ItemView> = segment
                            .item_indices
                            .iter()
                            .filter_map(|index| g.items.get(*index))
                            .filter(|item| item_visible_for_query(item, &query, group_matches))
                            .collect();
                        let st_cell = std::cell::RefCell::new(&mut *st);
                        let ev_cell = std::cell::RefCell::new(&mut ev);
                        let mut draw_group = |ui: &mut egui::Ui, group: &HierarchyGroup<'_, ItemView>, group_open: bool, group_depth: usize| {
                            if !gap_headers.is_empty() {
                                ui.add_space(4.0);
                            }
                            let (allocated_rect, allocated) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
                            let fr = Rect::from_min_max(pos2(ui.clip_rect().left(), allocated_rect.top()), pos2(allocated_rect.right(), allocated_rect.bottom()));
                            let row_depth = group_depth + 1 + depth;
                            let fresp = ui.interact(inset_row_rect(fr, row_depth), allocated.id, Sense::click_and_drag());
                            if fresp.hovered()
                                && fresp.dnd_hover_payload::<AssetDrag>().is_some_and(|payload| matches!(payload.as_ref(), AssetDrag::Group { scope, .. } if scope == panel))
                            {
                                non_root_hierarchy_target_hovered.set(true);
                            }
                            fresp.widget_info(|| {
                                egui::WidgetInfo::labeled(egui::WidgetType::CollapsingHeader, true, group.path.last().map(String::as_str).unwrap_or(""))
                            });
                            paint_asset_browser_header(
                                ui,
                                fr,
                                group_open,
                                group.path.last().map(String::as_str).unwrap_or(""),
                                row_depth,
                                fresp.hovered(),
                                Some(group.count),
                            );
                            let name = group.path.last().map(String::as_str).unwrap_or("");
                            let node_id = NodeId::group_path(
                                std::iter::once(g.name.clone()).chain(group.path.iter().take(group.path.len().saturating_sub(1)).cloned()),
                                name.to_owned(),
                            );
                            gap_headers.push((node_id.clone(), inset_row_rect(fr, row_depth), row_depth, group_open));
                            if canonical_hierarchy_drag {
                                if capabilities.group_drag {
                                    asset_browser_header_drag_source_with_id(&fresp, panel, node_id.clone());
                                }
                                let hit_index = canonical_drop_hits.borrow().len();
                                canonical_drop_hits.borrow_mut().push((
                                    group_header_drop_hit(inset_row_rect(fr, row_depth), Pos2::ZERO, node_id, row_depth, hit_index),
                                    fresp.clone(),
                                ));
                            } else if !canonical_hierarchy_drag
                                && let Some(dragged) = asset_browser_header_target_with_capabilities(
                                ui,
                                &fresp,
                                inset_row_rect(fr, row_depth),
                                panel,
                                &g.name,
                                &group.path,
                                HeaderDropCapabilities {
                                    item_drop: capabilities.item_drop && domain_hierarchy_actions,
                                    // Nested path groups can only accept group drops when the
                                    // consumer has a domain adapter for those paths. Never consume
                                    // a release that the transient root-group model cannot apply.
                                    group_drag: capabilities.group_drag && domain_hierarchy_actions,
                                    reject_group_drop: fresp.dnd_hover_payload::<AssetDrag>().is_some_and(|payload| {
                                        matches!(payload.as_ref(), AssetDrag::Group { scope, name, path } if scope == panel && path.is_empty() && group_is_descendant(&parents, name, &g.name))
                                    }),
                                },
                            ) {
                                let edge = ui.ctx().pointer_interact_pos().is_some_and(|p| header_drop_before(fr, p));
                                group_releases.push((dragged, g.name.clone(), group.path.clone(), edge));
                            }
                            if fresp.clicked() {
                                let path = group.path.iter().map(|part| escape_hierarchy_key_part(part)).collect::<Vec<_>>().join("/");
                                group_toggles.push(format!("{panel}/{}/{}", escape_hierarchy_key_part(&g.name), path));
                            }
                            if let Some(menu) = nested_group_context_menu {
                                if compact_context_menus {
                                    compact_context_menu_target(ui, fresp.clone(), |ui| {
                                menu(ui, &mut st_cell.borrow_mut(), panel, g, &group.path, &mut ev_cell.borrow_mut())
                                    });
                                } else {
                                    context_menu_target(fresp.clone(), |ui| {
                                menu(ui, &mut st_cell.borrow_mut(), panel, g, &group.path, &mut ev_cell.borrow_mut())
                                    });
                                }
                            }
                        };
                        let mut draw_items = |ui: &mut egui::Ui, items: &[&ItemView], group_path: &[String]| {
                            let group_depth = group_path.len();
                            let w = ui.available_width();
                            // Cards and hierarchy rows share the same 20px-per-level visual
                            // coordinate. Grid-only padding is accounted for inside that amount,
                            // rather than being added on top of the hierarchy indentation.
                            let visual_indent = row_indent(depth + group_depth + 1);
                            let content_indent = if list { visual_indent } else { (visual_indent - grid_indent).max(0.0) };
                            let pad = content_indent + (if list { 0.0 } else { grid_indent });
                            let gap = if list || flush { 0.0 } else { GRID_GAP };
                            let (cell, cols) = if list {
                                let row_h = list_item_height.unwrap_or(24.0).max(1.0);
                                (vec2((w - content_indent).max(0.0), row_h), 1)
                            } else {
                                let cell = item_size.unwrap_or(vec2(size, size));
                                let cols = ((w - pad + gap) / (cell.x + gap)).floor().max(1.0) as usize;
                                (cell, cols)
                            };
                            let rows = items.len().div_ceil(cols);
                            let grid_width = cols as f32 * cell.x + cols.saturating_sub(1) as f32 * gap;
                            let center_offset = if !list && center_grid { ((w - pad - grid_width).max(0.0)) * 0.5 } else { 0.0 };
                            // Reserve the exact grid height: the extra two pixels here used to
                            // accumulate with row spacing and leave uneven vertical gaps.
                            let grid_height = if rows == 0 { 0.0 } else { rows as f32 * cell.y + rows.saturating_sub(1) as f32 * gap };
                            let (area, _) = ui.allocate_exact_size(vec2(w, grid_height), Sense::hover());
                            if grid_height > 0.0 {
                                group_content_areas.push((g.name.clone(), area));
                            }
                            let mut insertion_markers = Vec::new();
                            for (run_i, it) in items.iter().enumerate() {
                                let ii = g.items.iter().position(|original| std::ptr::eq(original, *it)).unwrap_or(run_i);
                                let (cx, cy) = ((run_i % cols) as f32, (run_i / cols) as f32);
                                let r = Rect::from_min_size(area.min + vec2(pad + center_offset + cx * (cell.x + gap), cy * (cell.y + gap)), cell);
                                // A one-pixel inset on every card yields a two-pixel visible gap
                                // between neighbours while retaining the same gap at group edges.
                                let r = if !list && !flush { r.shrink(1.0) } else { r };
                                let next_rect = (run_i + 1 < items.len()).then(|| {
                                    let next_i = run_i + 1;
                                    let (next_x, next_y) = ((next_i % cols) as f32, (next_i / cols) as f32);
                                    let next =
                                        Rect::from_min_size(area.min + vec2(pad + center_offset + next_x * (cell.x + gap), next_y * (cell.y + gap)), cell);
                                    if !list && !flush { next.shrink(1.0) } else { next }
                                });
                                let resp = ui.interact(r, ui.id().with((panel, gi, ii)), Sense::click_and_drag());
                                let is_sel = st_cell.borrow().is_selected_item(panel, &g.name, &it.group_path, &it.key);
                                if list {
                                    if let Some(paint) = paint_list_item.as_deref_mut() {
                                        asset_browser_item_info(&resp, &it.name, is_sel);
                                        paint(ui, r, gi, ii, is_sel);
                                        if !is_sel && resp.hovered() {
                                            ui.painter().rect_filled(r, 2.0, t.hover.gamma_multiply(0.5));
                                        }
                                    } else {
                                        asset_browser_list_row(ui, &resp, r, &it.name, is_sel);
                                        let tr = Rect::from_min_size(r.min + vec2(24.0, 2.0), vec2(20.0, 20.0));
                                        paint_item(ui, tr, gi, ii, ItemPaintState::Item { selected: is_sel });
                                        ui.painter().text(
                                            pos2(tr.right() + 8.0, r.center().y),
                                            Align2::LEFT_CENTER,
                                            &it.name,
                                            FontId::proportional(11.5),
                                            t.text,
                                        );
                                    }
                                } else {
                                    asset_browser_item_info(&resp, &it.name, is_sel);
                                    paint_item(ui, r, gi, ii, ItemPaintState::Item { selected: is_sel });
                                    // Only selected cards are outlined; centring the stroke on the inset
                                    // card edge leaves one pixel of it on either side.
                                    if is_sel {
                                        ui.painter().rect_stroke(r, CornerRadius::ZERO, Stroke::new(2.0, t.accent), egui::StrokeKind::Middle);
                                    }
                                }
                                let hierarchy_accepts = resp.dnd_hover_payload::<AssetDrag>().is_some_and(|payload| match payload.as_ref() {
                                    AssetDrag::Group { scope, name, path } if scope == panel => {
                                        if path.is_empty() {
                                            domain_hierarchy_actions
                                                .then_some(!group_is_descendant(&parents, name, &g.name) && name != &g.name)
                                                .unwrap_or(true)
                                        } else {
                                            domain_hierarchy_actions && !(name == &g.name && it.group_path.starts_with(path))
                                        }
                                    }
                                    _ => false,
                                });
                                let hierarchy_drag = resp.dnd_hover_payload::<AssetDrag>().is_some_and(|payload| {
                                    matches!(payload.as_ref(), AssetDrag::Group { scope, .. } if scope == panel)
                                });
                                if hierarchy_drag && !hierarchy_accepts {
                                    // Invalid/self/unsupported card targets must not fall through
                                    // and reinterpret the same release as a root-group reorder.
                                    non_root_hierarchy_target_hovered.set(true);
                                }
                                if capabilities.group_drag {
                                    item_hierarchy_targets.push((resp.clone(), r, !list, hierarchy_accepts, ii, g.name.clone(), it.group_path.clone()));
                                }
                                if canonical_hierarchy_drag {
                                    if capabilities.item_drag {
                                        if let Some(source_id) = item_node_for_identity(&g.name, &it.group_path, &it.key, &hierarchy_identities) {
                                            asset_browser_item_drag_source_with_id(&resp, panel, &it.key, source_id);
                                        }
                                    }
                                    if let Some(target_id) = item_node_for_identity(&g.name, &it.group_path, &it.key, &hierarchy_identities) {
                                        let hit_index = canonical_drop_hits.borrow().len();
                                        for hit in card_drop_hits(r, !list, target_id, depth + it.group_path.len() + 1, hit_index) {
                                            canonical_drop_hits.borrow_mut().push((hit, resp.clone()));
                                        }
                                    }
                                }
                                let mut released_drag = if capabilities.item_drag && !hierarchy_drag && !canonical_hierarchy_drag {
                                    asset_browser_item_target(ui, &resp, r, next_rect, &mut insertion_markers, panel, &it.key, !list)
                                } else {
                                    None
                                };
                                // On grid rows, only the unused space after the last card is an
                                // additional drop target. Prepend stays within the card bounds.
                                if capabilities.item_drag && !list && (canonical_hierarchy_drag || released_drag.is_none()) {
                                    let last_in_row = run_i + 1 == items.len() || (run_i + 1) % cols == 0;
                                    let right_edge = item_row_edge_targets(area, r, last_in_row);
                                    let pointer = ui.ctx().pointer_interact_pos();
                                    if canonical_hierarchy_drag {
                                        if let Some(edge_rect) = right_edge {
                                            let edge_response = ui.interact(edge_rect, ui.id().with((panel, gi, ii, "right")), Sense::hover());
                                            if let Some(target_id) = item_node_for_identity(&g.name, &it.group_path, &it.key, &hierarchy_identities) {
                                                let hit_index = canonical_drop_hits.borrow().len();
                                                let mut hit = card_drop_hit(r, Pos2::new(r.right(), r.center().y), true, target_id, depth + it.group_path.len() + 1, hit_index);
                                                hit.hit_rect = edge_rect;
                                                canonical_drop_hits.borrow_mut().push((hit, edge_response));
                                            }
                                        }
                                    }
                                    let edge_target = right_edge.filter(|rect| pointer.is_some_and(|pointer| rect.contains(pointer))).map(|rect| ("right", rect));
                                    if let Some((side, edge_rect)) = edge_target {
                                        let edge_response = ui.interact(edge_rect, ui.id().with((panel, gi, ii, side)), Sense::hover());
                                        let edge_hierarchy_accepts = edge_response.dnd_hover_payload::<AssetDrag>().is_some_and(|payload| match payload.as_ref() {
                                            AssetDrag::Group { scope, name, path } if scope == panel => {
                                                if path.is_empty() {
                                                    domain_hierarchy_actions
                                                        .then_some(!group_is_descendant(&parents, name, &g.name) && name != &g.name)
                                                        .unwrap_or(true)
                                                } else {
                                                    domain_hierarchy_actions && !(name == &g.name && it.group_path.starts_with(path))
                                                }
                                            }
                                            _ => false,
                                        });
                                        let edge_hierarchy_drag = edge_response.dnd_hover_payload::<AssetDrag>().is_some_and(|payload| {
                                            matches!(payload.as_ref(), AssetDrag::Group { scope, .. } if scope == panel)
                                        });
                                        if capabilities.group_drag {
                                            item_hierarchy_targets.push((edge_response.clone(), r, true, edge_hierarchy_accepts, ii, g.name.clone(), it.group_path.clone()));
                                        }
                                        if !edge_hierarchy_drag && !canonical_hierarchy_drag {
                                            released_drag = asset_browser_item_target(ui, &edge_response, r, next_rect, &mut insertion_markers, panel, &it.key, true);
                                        }
                                    }
                                }
                                if let Some(AssetDrag::Preset { scope, key, .. }) = released_drag
                                    && scope.as_str() == panel
                                    && key.as_str() != it.key.as_str()
                                {
                                    browser_item_drop = true;
                                    let after = ui.ctx().pointer_interact_pos().is_some_and(|p| item_drop_after(r, p, !list));
                                    let mut index = ii + usize::from(after);
                                    if let Some((source_group, source_index)) = item_occurrence_position(groups, &key)
                                        && source_group == gi
                                        && source_index < index
                                    {
                                        index = index.saturating_sub(1);
                                    }
                                    if it.group_path.is_empty() {
                                        if !domain_hierarchy_actions
                                            && let Some((source_group_index, _)) = item_occurrence_position(groups, &key)
                                            && let Some(source_group) = groups.get(source_group_index)
                                        {
                                            let entries = hierarchy_orders.get(&g.name).cloned().unwrap_or_default();
                                            let target = HierarchyOrderEntry::Item(it.key.clone());
                                            let before = entries
                                                .iter()
                                                .position(|entry| entry == &target)
                                                .and_then(|position| if after { entries.get(position + 1).cloned() } else { entries.get(position).cloned() });
                                            move_item_sibling_order(&mut **st_cell.borrow_mut(), panel, &hierarchy_orders, &source_group.name, &g.name, &key, before.as_ref());
                                        }
                                        ev_cell.borrow_mut().push(BrowserAction::MoveItem(key, g.name.clone(), Some(index)));
                                    } else {
                                        let before = if after {
                                            g.items
                                                .iter()
                                                .skip(ii + 1)
                                                .find(|candidate| candidate.group_path == it.group_path && candidate.key != key)
                                                .map(|candidate| candidate.key.clone())
                                        } else {
                                            Some(it.key.clone())
                                        };
                                        ev_cell.borrow_mut().push(BrowserAction::MoveItemToGroup {
                                            key,
                                            group: g.name.clone(),
                                            group_path: it.group_path.clone(),
                                            before,
                                        });
                                    }
                                }
                                let hover = if it.group_path.is_empty() { it.name.clone() } else { format!("{} / {}", it.group_path.join(" / "), it.name) };
                                let resp = resp.on_hover_text(hover);
                                let (selected, activated) = asset_browser_item_gesture(&resp, capabilities.selection, capabilities.activation);
                                if activated {
                                    ev_cell.borrow_mut().push(BrowserAction::ActivateItem { group: g.name.clone(), group_path: it.group_path.clone(), key: it.key.clone() });
                                } else if selected {
                                    ev_cell.borrow_mut().push(BrowserAction::SelectItem { group: g.name.clone(), group_path: it.group_path.clone(), key: it.key.clone() });
                                }
                                if capabilities.item_drag
                                    && resp.dragged()
                                    && let Some(pp) = ui.ctx().pointer_interact_pos()
                                {
                                    // Keep the source card's size and preview, floating above the
                                    // browser content while the shared hit targets continue working.
                                    let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::new(("preset-drag", panel)));
                                    let grab_offset = ui
                                        .input(|input| input.pointer.press_origin())
                                        .filter(|origin| r.contains(*origin))
                                        .map(|origin| origin - r.min)
                                        .unwrap_or_else(|| r.size() * 0.5);
                                    let ghost_rect = Rect::from_min_size(pp - grab_offset, r.size());
                                    // Extend the child clip by half the selection stroke so the outer
                                    // half is not clipped at the card's bounds.
                                    let mut ghost_ui = ui.new_child(egui::UiBuilder::new().layer_id(layer).max_rect(ghost_rect.expand(1.0)));
                                    ghost_ui.set_opacity(0.4);
                                    paint_item(&ghost_ui, ghost_rect, gi, ii, ItemPaintState::DragGhost);
                                    ghost_ui.painter().rect_stroke(ghost_rect, CornerRadius::ZERO, Stroke::new(2.0, t.accent), egui::StrokeKind::Middle);
                                }
                                if capabilities.item_drag
                                    && resp.drag_stopped()
                                    && let Some(pp) = ui.ctx().pointer_interact_pos()
                                    && !r.contains(pp)
                                    && canvas.contains(pp)
                                {
                                    dropped = Some((it.key.clone(), pp));
                                }
                                if let Some(menu) = item_context_menu {
                                    if compact_context_menus {
                                        compact_context_menu_target(ui, resp.clone(), |ui| {
                                            menu(ui, &mut st_cell.borrow_mut(), panel, it, g, &group_names, &mut ev_cell.borrow_mut())
                                        });
                                    } else {
                                        context_menu_target(resp.clone(), |ui| {
                                            menu(ui, &mut st_cell.borrow_mut(), panel, it, g, &group_names, &mut ev_cell.borrow_mut())
                                        });
                                    }
                                }
                            }
                            for (marker_rect, after, grid) in insertion_markers {
                                crate::widgets::drop_line(ui, marker_rect, after, grid, &t);
                            }
                        };
                        if segment.show_folder_header {
                            draw_group(ui, &folder_group, folder_open, segment.folder_path.len().saturating_sub(1));
                            if folder_open {
                                hidden_folders.remove(&(g.name.clone(), segment.folder_path.clone()));
                            } else {
                                hidden_folders.insert((g.name.clone(), segment.folder_path.clone()));
                            }
                        }
                        if folder_open {
                            draw_items(ui, &segment_items, &segment.folder_path);
                        }
                    }
                    for (dragged, target_group, target_group_path, edge) in group_releases.drain(..) {
                        match dragged {
                            AssetDrag::Preset { scope, key, .. } if scope == panel => {
                                let before = groups
                                    .iter()
                                    .find(|group| group.name == target_group)
                                    .and_then(|group| group.items.iter().find(|item| item.group_path == target_group_path))
                                    .map(|item| item.key.clone());
                                ev.push(BrowserAction::MoveItemToGroup { key, group: target_group, group_path: target_group_path, before });
                                browser_item_drop = true;
                            }
                            AssetDrag::Group { scope, name, path } if scope == panel && domain_hierarchy_actions => {
                                if let Some(action) = hierarchy_group_drop(&name, &path, &target_group, &target_group_path, edge) {
                                    ev.push(hierarchy_action(action));
                                }
                            }
                            _ => {}
                        }
                    }
                    for key in group_toggles {
                        if !st.toggled.remove(&key) {
                            st.toggled.insert(key);
                        }
                    }
                }
            }
            // Match the existing indentation-column arbitration: the last eligible
            // (topmost/deepest) hit target wins both its preview and its release.
            let best_item_hierarchy_target = best_drop_target_index(&item_hierarchy_targets, |(response, _, _, accepts, _, _, _)| {
                    *accepts
                        && response.dnd_hover_payload::<AssetDrag>().is_some_and(|payload| {
                            matches!(payload.as_ref(), AssetDrag::Group { scope, .. } if scope == panel)
                        })
                });
            // The indentation column overlaps the leading edge of nested cards. Treat these as
            // competing destinations, not independent drop zones: when a card wins, its vertical
            // insertion marker and release must suppress the column's horizontal append marker.
            let item_hierarchy_target_wins = best_item_hierarchy_target.is_some();
            if item_hierarchy_target_wins {
                indent_release = None;
            }
            if !canonical_hierarchy_drag
                && let Some((response, rect, grid, _, item_index, target_name, target_group_path)) =
                best_item_hierarchy_target.and_then(|index| item_hierarchy_targets.get(index))
                && let Some((dragged, after)) = asset_browser_item_hierarchy_target(ui, response, *rect, *grid, true)
                && let Some(target_group) = groups.iter().find(|group| group.name == *target_name)
            {
                browser_item_drop = true;
                let moving_group_path = match &dragged {
                    AssetDrag::Group { name, path, .. } if *name == *target_name && !path.is_empty() => Some(path.clone()),
                    _ => None,
                };
                match dragged {
                    AssetDrag::Group { scope, name, path } if scope == panel && domain_hierarchy_actions => {
                        let (before, before_item) =
                            item_hierarchy_insertion_anchor(&target_group.items, *item_index, target_group_path, after, moving_group_path.as_deref());
                        ev.push(BrowserAction::MoveInto {
                            group: name,
                            group_path: path,
                            into: Some((target_name.clone(), target_group_path.clone())),
                            before,
                            before_item,
                        });
                        // The nested card/header target has consumed this release. Do not let the
                        // source root header also resolve a between-root-groups reorder.
                        released = None;
                    }
                    AssetDrag::Group { scope, name, path } if scope == panel && path.is_empty() && !domain_hierarchy_actions => {
                        if let Some(target_item) = target_group.items.get(*item_index).map(|item| item.key.clone()) {
                            let entries = hierarchy_orders.get(target_name).cloned().unwrap_or_default();
                            let target_entry = HierarchyOrderEntry::Item(target_item);
                            let before = entries
                                .iter()
                                .position(|entry| entry == &target_entry)
                                .and_then(|position| if after { entries.get(position + 1).cloned() } else { entries.get(position).cloned() });
                            if let Some(old_parent) = parents.get(&name).filter(|parent| *parent != target_name) {
                                st.remove_hierarchy_entry(panel, old_parent, &HierarchyOrderEntry::Group(name.clone()));
                            }
                            st.nest_group(panel, &group_names, &name, Some(target_name));
                            st.move_hierarchy_entry(panel, target_name, entries, HierarchyOrderEntry::Group(name), before.as_ref());
                            released = None;
                        }
                    }
                    _ => {}
                }
            }
            // Each expanded group owns a drop area spanning its full header width. Its vertical
            // span covers the group's complete visible subtree and ends before the next sibling.
            if capabilities.group_drag {
                for (_, header, group_name) in &headers {
                    if !expanded_groups.contains(group_name) {
                        continue;
                    }
                    let bottom = headers
                        .iter()
                        .filter(|(_, _, name)| name == group_name || group_is_descendant(&parents, group_name, name))
                        .map(|(_, rect, _)| rect.bottom())
                        .chain(
                            group_content_areas
                                .iter()
                                .filter(|(name, _)| name == group_name || group_is_descendant(&parents, group_name, name))
                                .map(|(_, rect)| rect.bottom()),
                        )
                        .max_by(f32::total_cmp);
                    if let Some(bottom) = bottom {
                        let column = Rect::from_min_max(pos2(header.left(), header.bottom()), pos2(header.left() + ROW_INDENT, bottom));
                        if column.is_positive() {
                            let response = ui.interact(
                                column,
                                ui.id().with(("group-indent-column", panel, group_name)),
                                Sense::hover(),
                            );
                            if canonical_hierarchy_drag {
                                let append_indicator = Rect::from_min_max(
                                    pos2(header.left(), bottom - 2.0),
                                    pos2(header.right(), bottom + 2.0),
                                );
                                let depth = display.iter().find_map(|(index, depth)| {
                                    groups.get(*index).is_some_and(|group| group.name == *group_name).then_some(*depth)
                                }).unwrap_or(0);
                                let hit_index = canonical_drop_hits.borrow().len();
                                let hit = group_append_hit(column, append_indicator, NodeId::group(group_name), depth, hit_index);
                                canonical_drop_hits.borrow_mut().push((hit, response.clone()));
                            }
                            indent_drop_targets.push((group_name.clone(), response));
                        }
                    }
                }
            }
            // Each adjacent visible group pair has an actual six-pixel gap. The upper half inserts
            // after the last card in the upper group's own sibling sequence; when that sequence has
            // no cards, it falls back to placing after the group header itself.
            if canonical_hierarchy_drag {
                for pair in gap_headers.windows(2) {
                    let (upper_id, _upper_rect, upper_depth, upper_open) = &pair[0];
                    let (lower_id, lower_rect, lower_depth, _) = &pair[1];
                    let gap_rect = Rect::from_min_max(
                        pos2(lower_rect.left(), lower_rect.top() - 6.0),
                        pos2(lower_rect.right(), lower_rect.top()),
                    );
                    // The layout inserts exactly six pixels before every visible header after
                    // the first. Do not infer the gap from the upper group's content bounds: a
                    // mixed sibling sequence can render cards from that group after a child
                    // header, and nested descendants can extend beyond the upper group's own
                    // card area. The six pixels immediately preceding the lower header are the
                    // authoritative gap regardless of that content ordering.
                    if gap_rect.is_positive() {
                        let response = ui.interact(
                            gap_rect,
                            ui.id().with(("group-gap", panel, upper_id, lower_id)),
                            Sense::hover(),
                        );
                        let (upper_anchor, upper_anchor_depth) = if *upper_open {
                            group_gap_upper_anchor(&hierarchy, upper_id)
                        } else {
                            (upper_id.clone(), *upper_depth)
                        };
                        let mut hits = group_gap_drop_hits(
                            gap_rect,
                            upper_anchor,
                            lower_id.clone(),
                            upper_anchor_depth.max(*upper_depth),
                            *lower_depth,
                            canonical_drop_hits.borrow().len(),
                        );
                        // The gap immediately before a visible child group is also the parent's
                        // append target. This remains valid when the dragged group is already a
                        // child of that parent: AppendTo reorders it to the end of the same list.
                        if hierarchy.parent_of(lower_id) == Some(ContainerId::Group(upper_id.clone())) {
                            hits[0].target = None;
                            hits[0].container = Some(ContainerId::Group(upper_id.clone()));
                            hits[0].region = PointerRegion::ContainerEnd;
                        }
                        for hit in hits {
                            canonical_drop_hits.borrow_mut().push((hit, response.clone()));
                        }
                    }
                }
            }
            // `dnd_release_payload` takes the one global egui payload. Resolve the card target
            // first, then allow the indentation column to consume it only when no card target
            // won and the pointer is actually over an eligible column. Calling it while merely
            // registering each column lets an overlapping column steal a card release.
            let indent_target = best_indent_drop_target_index(&indent_drop_targets, item_hierarchy_target_wins, |(parent, response)| {
                    if canonical_hierarchy_drag {
                        ui.ctx().pointer_interact_pos().is_some_and(|pointer| response.rect.contains(pointer))
                    } else {
                        response.dnd_hover_payload::<AssetDrag>().is_some_and(|payload| {
                            matches!(payload.as_ref(), AssetDrag::Group { scope, name, path } if scope == panel && path.is_empty() && name != parent && !group_is_descendant(&parents, name, parent))
                        })
                    }
                }).and_then(|index| indent_drop_targets.get(index));
            if indent_target.is_none()
                && indent_drop_targets.iter().any(|(_, response)| {
                    response.dnd_hover_payload::<AssetDrag>().is_some_and(|payload| matches!(payload.as_ref(), AssetDrag::Group { scope, .. } if scope == panel))
                })
            {
                non_root_hierarchy_target_hovered.set(true);
            }
            if !canonical_hierarchy_drag
                && let Some((parent, response)) = indent_target
                && let Some(AssetDrag::Group { scope, name, path }) = response.dnd_release_payload::<AssetDrag>().map(|payload| payload.as_ref().clone())
                && scope == panel
                && path.is_empty()
                && name != *parent
            {
                indent_release = Some((name, parent.clone()));
            }
            // Keep the legacy append marker below the parent's complete visible subtree. The
            // canonical path instead highlights the full receiving group because group-body
            // drops now insert at its first child position.
            let indent_target_hovered = indent_target.is_some();
            if indent_target_hovered {
                if let Some((parent, _)) = indent_target {
                    let end = headers
                        .iter()
                        .filter(|(_, _, name)| name == parent || group_is_descendant(&parents, parent, name))
                        .map(|(_, rect, _)| *rect)
                        .chain(
                            group_content_areas
                                .iter()
                                .filter(|(name, _)| name == parent || group_is_descendant(&parents, parent, name))
                                .map(|(_, rect)| *rect),
                        )
                        .max_by(|a, b| a.bottom().total_cmp(&b.bottom()));
                    if let (Some(anchor), Some((_, header, _))) = (
                        end,
                        headers.iter().find(|(_, _, name)| name == parent),
                    ) {
                        // Keep the user-facing append marker aligned to the destination header,
                        // not to the narrower indentation hit target or full-width content area.
                        let marker_anchor = Rect::from_min_max(
                            pos2(header.left(), anchor.bottom() - header.height()),
                            pos2(header.right(), anchor.bottom()),
                        );
                        if !canonical_hierarchy_drag {
                            paint_group_insertion_line(ui, marker_anchor, false, t.accent);
                        }
                    }
                }
            }
            // The regular group drag marker handles between-header reordering.
            if !canonical_hierarchy_drag && !indent_target_hovered && !item_hierarchy_target_wins && let Some((from, p)) = drag {
                // Header targets already paint their feedback through the shared interaction
                // helper. Only draw the between-groups insertion marker when the pointer is in a
                // gap/outside a target header.
                if !headers.iter().any(|(i, r, _)| *i != from && r.contains(p)) {
                    if let Some(from_name) = groups.get(from).map(|group| group.name.as_str()) {
                        let before = group_drop_before(&headers, from, from_name, p, &parents);
                        // The source header resolves to a no-op on release; never show a marker
                        // suggesting that dropping it there would move it.
                        if before != Some(from) {
                            let source_parent = parents.get(from_name);
                            let source_rect = headers.iter().find(|(index, _, _)| *index == from).map(|(_, rect, _)| *rect);
                            let promote_to_root = source_parent.is_some() && source_rect.is_some_and(|rect| p.x < rect.left() + ROW_INDENT * 0.5);
                            let destination_parent = if promote_to_root { None } else { source_parent };
                            let anchor = match before {
                                Some(i) => headers.iter().find(|(x, _, _)| *x == i).map(|(_, r, _)| *r),
                                None => headers.iter().rev().find(|(_, _, name)| parents.get(name).map(String::as_str) == destination_parent.map(String::as_str)).map(|(_, r, _)| *r),
                            };
                            if let Some(r) = anchor {
                                paint_group_insertion_line(ui, r, before.is_some(), t.accent);
                            }
                        }
                    }
                }
            }
            if canonical_hierarchy_drag {
                let (root_end_rect, root_end_response) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
                let indicator = Rect::from_min_max(
                    pos2(root_end_rect.left(), root_end_rect.top() - 2.0),
                    pos2(root_end_rect.right(), root_end_rect.top() + 2.0),
                );
                let hit_index = canonical_drop_hits.borrow().len();
                canonical_drop_hits.borrow_mut().push((root_append_hit(root_end_rect, indicator, hit_index), root_end_response));
            }
        });
    let mut hierarchy_drop_feedback = None;
    if canonical_hierarchy_drag {
        if let Some(pointer) = ui.ctx().pointer_interact_pos() {
            let hits: Vec<DropHitRegion> = canonical_drop_hits.borrow().iter().map(|(hit, _)| hit.clone()).collect();
            if let Some(index) = arbitrate_hit_index(pointer, &hits)
                && canonical_drop_hits.borrow().get(index).is_some()
            {
                // The canonical hit rectangles are arbitrated independently of egui's topmost
                // hovered response. A valid geometric hit can therefore belong to a response
                // that egui does not consider hovered (notably later root-group headers).
                // Read the active drag from the global payload and use the winning hit above as
                // the sole authority for the destination.
                let hovered = egui::DragAndDrop::payload::<AssetDrag>(ui.ctx()).map(|payload| payload.as_ref().clone());
                if let Some(dragged) = hovered.as_ref()
                && let Some(source) = source_node_for_drag(dragged, panel, &hierarchy_identities)
                {
                    let mut session = DragSession::new(source.clone());
                    if let Some(candidate) = session.resolve(pointer, &hits[index], &hierarchy).cloned() {
                        paint_candidate(ui, &candidate, t.accent);
                        // Resolve and validate the exact candidate before taking egui's global
                        // one-shot payload. The taken source must still identify this same node.
                        if let Some(released) = asset_browser_take_release(ui.ctx()) {
                            if source_node_for_drag(&released, panel, &hierarchy_identities).as_ref() == Some(&source) {
                                let result = session.commit_with(|intent| {
                                    if let Some(commit) = canonical_hierarchy_move.as_deref_mut() {
                                        return commit(intent);
                                    }
                                    if let Some(HierarchyIdentity::Item { group, key }) = hierarchy_identities.get(&intent.source) {
                                        let resolved = hierarchy.resolve_move(&intent.source, &intent.position).map_err(|error| format!("Could not resolve item move: {error:?}"))?;
                                        let ContainerId::Group(destination_id) = resolved.destination() else {
                                            return Err("This asset library cannot move items outside a named group.".to_owned());
                                        };
                                        let Some(HierarchyIdentity::Group(destination_group)) = hierarchy_identities.get(destination_id) else {
                                            return Err("The destination group is no longer available.".to_owned());
                                        };
                                        let Some(commit) = hierarchy_item_move.as_deref_mut() else {
                                            return Err("This asset adapter does not support authoritative item moves.".to_owned());
                                        };
                                        let children = hierarchy.children_of(resolved.destination()).ok_or_else(|| "The destination group is no longer available.".to_owned())?;
                                        let same_container = hierarchy.parent_of(&intent.source).as_ref() == Some(resolved.destination());
                                        let domain_index = children
                                            .iter()
                                            .filter(|node| !same_container || node.id() != &intent.source)
                                            .take(resolved.insertion_index())
                                            .filter(|node| matches!(node, HierarchyNode::Item(_)))
                                            .count();
                                        commit(group, key, destination_group, domain_index)?;
                                    }
                                    let legacy_persistable = hierarchy_identities.values().all(|identity| {
                                        matches!(identity, HierarchyIdentity::Item { .. } | HierarchyIdentity::Group(_))
                                    });
                                    if !legacy_persistable {
                                        return Err("This asset adapter does not support path-aware hierarchy moves.".to_owned());
                                    }
                                    commit_hierarchy_move(st, panel, &mut hierarchy, &intent.source, &intent.position, &hierarchy_identities)
                                        .map_err(|error| format!("Could not persist hierarchy move: {error:?}"))
                                });
                                hierarchy_drop_feedback = match result {
                                    DropCommitResult::Committed | DropCommitResult::NoCandidate => None,
                                    DropCommitResult::Rejected(reason) => Some(format!("Move rejected: {reason:?}")),
                                    DropCommitResult::Failed(error) => Some(error),
                                };
                            } else {
                                hierarchy_drop_feedback = Some("Move rejected: the dragged source changed before release.".to_owned());
                            }
                        }
                    }
                }
            }
        }
    }
    if let Some(message) = hierarchy_drop_feedback {
        ui.colored_label(ui.visuals().error_fg_color, message);
    }
    // A nested group header is a distinct target family from the root-group rows. If the pointer
    // is over one, the root resolver must not reinterpret that same release as a root reorder.
    if non_root_hierarchy_target_hovered.get() {
        released = None;
    }
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("preset-browser-rect", panel)), scroll.inner_rect));
    // Apply the drop now every header rect is known; the next frame draws the new order.
    if !canonical_hierarchy_drag && let Some((from_name, parent)) = indent_release {
        if !group_is_descendant(&parents, &from_name, &parent) {
            if let Some(old_parent) = parents.get(&from_name).filter(|old_parent| *old_parent != &parent) {
                st.remove_hierarchy_entry(panel, old_parent, &HierarchyOrderEntry::Group(from_name.clone()));
            }
            st.nest_group(panel, &group_names, &from_name, Some(&parent));
            let entries = hierarchy_orders.get(&parent).cloned().unwrap_or_default();
            st.move_hierarchy_entry(panel, &parent, entries, HierarchyOrderEntry::Group(from_name), None);
        }
    } else if !canonical_hierarchy_drag && let Some((from, p)) = released {
        let from_name = groups.get(from).map(|g| g.name.clone());
        if let Some(from_name) = from_name {
            if let Some((_, rect, target)) = headers.iter().find(|(i, rect, _)| *i != from && rect.contains(p)) {
                let edge = header_drop_before(*rect, p);
                if domain_hierarchy_actions && !group_is_descendant(&parents, &from_name, target) {
                    if let Some(action) = hierarchy_group_drop(&from_name, &[], target, &[], edge) {
                        ev.push(hierarchy_action(action));
                    }
                } else if !domain_hierarchy_actions {
                    let destination_parent = if edge { parents.get(target).cloned() } else { Some(target.clone()) };
                    let creates_cycle =
                        destination_parent.as_deref().is_some_and(|parent| parent == from_name || group_is_descendant(&parents, &from_name, parent));
                    if !creates_cycle {
                        if let Some(old_parent) = parents.get(&from_name).filter(|old_parent| Some(*old_parent) != destination_parent.as_ref()) {
                            st.remove_hierarchy_entry(panel, old_parent, &HierarchyOrderEntry::Group(from_name.clone()));
                        }
                        st.nest_group(panel, &group_names, &from_name, destination_parent.as_deref());
                        if let Some(parent) = destination_parent {
                            let entries = hierarchy_orders.get(&parent).cloned().unwrap_or_default();
                            let before = edge.then(|| HierarchyOrderEntry::Group(target.clone()));
                            st.move_hierarchy_entry(panel, &parent, entries, HierarchyOrderEntry::Group(from_name), before.as_ref());
                        } else {
                            st.move_group(panel, &group_names, &from_name, Some(target));
                        }
                    }
                }
            } else {
                let before = group_drop_before(&headers, from, &from_name, p, &parents);
                if before != Some(from) {
                    let before_name = before.and_then(|i| groups.get(i).map(|g| g.name.clone()));
                    if domain_hierarchy_actions {
                        ev.push(BrowserAction::MoveGroup { group: from_name, group_path: Vec::new(), before: before_name });
                    } else {
                        let source_parent = parents.get(&from_name).cloned();
                        let source_rect = headers.iter().find(|(index, _, _)| *index == from).map(|(_, rect, _)| *rect);
                        let promote_to_root = source_parent.is_some() && source_rect.is_some_and(|rect| p.x < rect.left() + ROW_INDENT * 0.5);
                        if let Some(parent) = source_parent.clone().filter(|_| !promote_to_root) {
                            let entries = hierarchy_orders.get(&parent).cloned().unwrap_or_default();
                            let before = before_name.as_ref().map(|name| HierarchyOrderEntry::Group(name.clone()));
                            st.move_hierarchy_entry(panel, &parent, entries, HierarchyOrderEntry::Group(from_name), before.as_ref());
                        } else {
                            if let Some(old_parent) = source_parent.as_deref() {
                                st.remove_hierarchy_entry(panel, old_parent, &HierarchyOrderEntry::Group(from_name.clone()));
                            }
                            st.nest_group(panel, &group_names, &from_name, None);
                            st.move_group(panel, &group_names, &from_name, before_name.as_deref());
                        }
                    }
                }
            }
        }
    }
    let recorded: Vec<(String, Rect)> = headers.iter().map(|(_, r, n)| (n.clone(), *r)).collect();
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("preset-group-headers", panel)), recorded));
    if let Some((key, pos)) = dropped.filter(|_| !browser_item_drop) {
        ev.push(BrowserAction::Drop(key, pos));
    }
    ev
}

/// The sibling group (by library index) a header dragged to `y` lands before (`None` = last).
/// Root and nested groups use the same sibling-level positioning rule.
fn group_drop_before(
    headers: &[(usize, Rect, String)],
    from: usize,
    from_name: &str,
    pointer: Pos2,
    parents: &std::collections::BTreeMap<String, String>,
) -> Option<usize> {
    let source_parent = parents.get(from_name);
    let source_rect = headers.iter().find(|(index, _, _)| *index == from).map(|(_, rect, _)| *rect);
    let promote_to_root = source_parent.is_some() && source_rect.is_some_and(|rect| pointer.x < rect.left() + ROW_INDENT * 0.5);
    let destination_parent = if promote_to_root { None } else { source_parent };
    let siblings: Vec<(usize, Rect, &str)> =
        headers.iter().filter(|(_, _, name)| parents.get(name) == destination_parent).map(|(index, rect, name)| (*index, *rect, name.as_str())).collect();
    let hit = siblings.iter().find(|(_, rect, _)| pointer.y < rect.center().y).map(|(index, _, _)| *index);
    match hit {
        Some(i) if i == from => Some(from),
        Some(i) => {
            let after_self = siblings
                .iter()
                .position(|(index, _, _)| *index == from)
                .zip(siblings.iter().position(|(index, _, _)| *index == i))
                .is_some_and(|(a, b)| b == a + 1);
            if after_self { Some(from) } else { Some(i) }
        }
        None => None,
    }
}

/// Group header rects drawn last frame (name → rect), in display order; for tests and automation.
#[cfg(test)]
pub fn last_group_headers(ctx: &egui::Context, panel: &str) -> Vec<(String, Rect)> {
    ctx.data(|d| d.get_temp(egui::Id::new(("preset-group-headers", panel)))).unwrap_or_default()
}

/// The allocated viewport from the last frame; useful to verify the shared browser's layout.
#[cfg(test)]
pub(crate) fn last_browser_rect(ctx: &egui::Context, panel: &str) -> Option<Rect> {
    ctx.data(|d| d.get_temp(egui::Id::new(("preset-browser-rect", panel))))
}

#[cfg(test)]
#[path = "asset_browser_ui_tests.rs"]
mod ui_tests;

#[cfg(test)]
mod filter_tests {
    use super::{
        BrowserAction, BrowserAdapter, BrowserCapabilities, BrowserOptions, GroupView, HierarchyOrderEntry, ItemPaintState, ItemView, best_drop_target_index,
        best_indent_drop_target_index, browser, filter_groups, group_render_segments, hit_debug_label_position, item_hierarchy_insertion_anchor, item_occurrence_position,
        HIT_DEBUG_LABEL_HEIGHT,
    };
    use crate::pickers::picker_state::PresetUi;
    use egui_kittest::kittest::Queryable;

    fn group(name: &str, items: Vec<ItemView>) -> GroupView {
        GroupView { name: name.into(), items }
    }

    fn item(key: &str, name: &str, group_path: &[&str]) -> ItemView {
        ItemView { key: key.into(), name: name.into(), group_path: group_path.iter().map(|part| (*part).into()).collect() }
    }

    #[test]
    fn filtering_keeps_identity_order_and_matches_root_or_nested_group_names() {
        let groups = vec![
            group("Warm", vec![item("sunset-id", "Sunset", &["Favorites", "Sky"]), item("ember-id", "Ember", &[])]),
            group("Cool", vec![item("ocean-id", "Ocean", &[])]),
        ];

        let by_nested_group = filter_groups(&groups, "sky");
        assert_eq!(by_nested_group.len(), 1);
        assert_eq!(by_nested_group[0].items.iter().map(|item| item.key.as_str()).collect::<Vec<_>>(), ["sunset-id"]);

        let by_item = filter_groups(&groups, "ember");
        assert_eq!(by_item.len(), 1);
        assert_eq!(by_item[0].items.iter().map(|item| item.key.as_str()).collect::<Vec<_>>(), ["ember-id"]);

        let by_group = filter_groups(&groups, "WARM");
        assert_eq!(by_group[0].items.iter().map(|item| item.key.as_str()).collect::<Vec<_>>(), ["sunset-id", "ember-id"]);

        let all = filter_groups(&groups, "  ");
        assert_eq!(all.iter().map(|group| group.name.as_str()).collect::<Vec<_>>(), ["Warm", "Cool"]);
    }

    #[test]
    fn occurrence_lookup_distinguishes_items_with_the_same_display_name() {
        let groups = vec![group("Brushes", vec![item("brush-occurrence-a", "Soft Round", &[]), item("brush-occurrence-b", "Soft Round", &[])])];

        assert_eq!(item_occurrence_position(&groups, "brush-occurrence-a"), Some((0, 0)));
        assert_eq!(item_occurrence_position(&groups, "brush-occurrence-b"), Some((0, 1)));
    }

    #[test]
    fn best_drop_target_is_the_last_eligible_candidate() {
        let candidates = [false, true, false, true, false];

        assert_eq!(best_drop_target_index(&candidates, |eligible| *eligible), Some(3));
        assert_eq!(best_drop_target_index(&[false, false], |eligible| *eligible), None);
    }

    #[test]
    fn card_drop_target_wins_over_an_overlapping_group_indent_column() {
        let columns = [false, true, true];

        assert_eq!(best_indent_drop_target_index(&columns, true, |eligible| *eligible), None);
        assert_eq!(best_indent_drop_target_index(&columns, false, |eligible| *eligible), Some(2));
    }

    #[test]
    fn card_drop_anchor_tracks_the_next_item_or_nested_group_at_the_same_level() {
        let items = vec![item("a", "A", &[]), item("b", "B", &[]), item("nested", "Nested item", &["Nested"]), item("c", "C", &[])];

        assert_eq!(item_hierarchy_insertion_anchor(&items, 1, &[], false, None), (None, Some("b".into())));
        assert_eq!(item_hierarchy_insertion_anchor(&items, 0, &[], true, None), (None, Some("b".into())));
        assert_eq!(item_hierarchy_insertion_anchor(&items, 1, &[], true, None), (Some("Nested".into()), None));
        assert_eq!(item_hierarchy_insertion_anchor(&items, 3, &[], true, None), (None, None));
    }

    #[test]
    fn card_drop_after_skips_the_group_being_moved_when_it_is_the_next_sibling() {
        let items = vec![item("a", "A", &[]), item("f1", "F1", &["F"]), item("f2", "F2", &["F"]), item("b", "B", &[])];

        // F already follows A. It cannot be its own `before` anchor after the engine
        // partitions F out of the source group; B is the stable next sibling instead.
        assert_eq!(item_hierarchy_insertion_anchor(&items, 0, &[], true, Some(&["F".into()])), (None, Some("b".into())));
    }

    #[test]
    fn card_drop_after_skips_a_moving_group_before_the_target_card() {
        let items = vec![item("f", "F", &["F"]), item("a", "A", &[]), item("b", "B", &[])];

        assert_eq!(item_hierarchy_insertion_anchor(&items, 1, &[], true, Some(&["F".into()])), (None, Some("b".into())));
    }

    #[test]
    fn upper_and_lower_gap_debug_labels_do_not_overwrite_each_other() {
        let gap = egui::Rect::from_min_max(egui::pos2(10.0, 100.0), egui::pos2(210.0, 106.0));
        let hits = crate::pickers::asset_browser_interaction::group_gap_drop_hits(
            gap,
            crate::pickers::picker_browser_model::NodeId::from("upper"),
            crate::pickers::picker_browser_model::NodeId::from("lower"),
            0,
            1,
            0,
        );
        let upper_label = hit_debug_label_position(&hits[0]);
        let lower_label = hit_debug_label_position(&hits[1]);

        assert!(upper_label.y >= gap.bottom(), "upper label starts above the visible gap: y={}", upper_label.y);
        assert!(
            upper_label.y + HIT_DEBUG_LABEL_HEIGHT <= lower_label.y,
            "upper label at y={} overlaps lower label at y={}",
            upper_label.y,
            lower_label.y,
        );
    }

    #[test]
    fn nested_card_drop_after_skips_the_moving_group_and_anchors_to_the_next_sibling_group() {
        let items = vec![
            item("a", "A", &["Parent"]),
            item("f1", "F1", &["Parent", "Moving"]),
            item("f2", "F2", &["Parent", "Moving"]),
            item("g1", "G1", &["Parent", "Next"]),
        ];

        assert_eq!(
            item_hierarchy_insertion_anchor(&items, 0, &["Parent".into()], true, Some(&["Parent".into(), "Moving".into()])),
            (Some("Next".into()), None)
        );
    }

    fn segment_summary(segments: &[super::GroupRenderSegment], groups: &[GroupView]) -> Vec<(String, bool, Vec<String>)> {
        segments
            .iter()
            .map(|segment| {
                let group = &groups[segment.group_index];
                let items = segment.item_indices.iter().filter_map(|index| group.items.get(*index)).map(|item| item.key.clone()).collect();
                (group.name.clone(), segment.show_header, items)
            })
            .collect()
    }

    #[test]
    fn unified_sibling_order_splits_card_runs_at_group_positions() {
        let groups = vec![
            group("Parent", vec![item("a", "A", &[]), item("b", "B", &[]), item("c", "C", &[])]),
            group("Before A", vec![item("x", "X", &[])]),
            group("Before C", vec![item("y", "Y", &[])]),
        ];
        // The persisted sequence is the sole source of sibling positioning, regardless of the
        // groups' library/display order.
        let display = [(0, 0), (2, 1), (1, 1)];
        let parents = [("Before A".into(), "Parent".into()), ("Before C".into(), "Parent".into())].into();
        let orders = [(
            "Parent".into(),
            vec![
                HierarchyOrderEntry::Group("Before A".into()),
                HierarchyOrderEntry::Item("a".into()),
                HierarchyOrderEntry::Item("b".into()),
                HierarchyOrderEntry::Group("Before C".into()),
                HierarchyOrderEntry::Item("c".into()),
            ],
        )]
        .into();

        let segments = group_render_segments(&groups, &display, &parents, &orders);

        assert_eq!(
            segment_summary(&segments, &groups),
            [
                ("Parent".into(), true, vec![]),
                ("Before A".into(), true, vec!["x".into()]),
                ("Parent".into(), false, vec!["a".into(), "b".into()]),
                ("Before C".into(), true, vec!["y".into()]),
                ("Parent".into(), false, vec!["c".into()]),
            ]
        );
        assert_eq!(segments.iter().filter(|segment| segment.show_header).count(), groups.len());
    }

    #[test]
    fn sibling_sequence_keeps_empty_groups_and_appends_unlisted_entries() {
        let groups =
            vec![group("Parent", vec![item("a", "A", &[])]), group("First", vec![]), group("Second", vec![]), group("Stale", vec![]), group("Append", vec![])];
        let display = [(0, 0), (1, 1), (2, 1), (3, 1), (4, 1)];
        let parents =
            [("First".into(), "Parent".into()), ("Second".into(), "Parent".into()), ("Stale".into(), "Parent".into()), ("Append".into(), "Parent".into())]
                .into();
        let orders = [(
            "Parent".into(),
            vec![HierarchyOrderEntry::Group("Second".into()), HierarchyOrderEntry::Item("a".into()), HierarchyOrderEntry::Group("First".into())],
        )]
        .into();

        let segments = group_render_segments(&groups, &display, &parents, &orders);

        assert_eq!(
            segment_summary(&segments, &groups),
            [
                ("Parent".into(), true, vec![]),
                ("Second".into(), true, vec![]),
                ("Parent".into(), false, vec!["a".into()]),
                ("First".into(), true, vec![]),
                ("Stale".into(), true, vec![]),
                ("Append".into(), true, vec![]),
            ]
        );
    }

    #[test]
    fn typed_root_items_distinguish_duplicate_keys_from_different_groups() {
        use super::build_hierarchy_projection;
        use crate::pickers::picker_browser_hierarchy::HierarchyIdentity;
        use crate::pickers::picker_browser_model::HierarchyNode;

        let groups = vec![group("First", vec![item("same", "Same", &[])]), group("Second", vec![item("same", "Same", &[])])];
        let display = [(0, 0), (1, 0)];
        let orders = [
            ("".into(), vec![
                HierarchyOrderEntry::RootItem { group: "First".into(), key: "same".into() },
                HierarchyOrderEntry::RootItem { group: "Second".into(), key: "same".into() },
            ]),
            ("First".into(), vec![HierarchyOrderEntry::Item("same".into())]),
            ("Second".into(), vec![HierarchyOrderEntry::Item("same".into())]),
        ].into();
        let (_, hierarchy, identities) = build_hierarchy_projection(&groups, &display, &Default::default(), &orders).unwrap();
        let root_items: Vec<_> = hierarchy.roots().iter().filter(|node| matches!(node, HierarchyNode::Item(_))).collect();
        assert_eq!(root_items.len(), 2);
        assert_eq!(identities[root_items[0].id()], HierarchyIdentity::Item { group: "First".into(), key: "same".into() });
        assert_eq!(identities[root_items[1].id()], HierarchyIdentity::Item { group: "Second".into(), key: "same".into() });
        assert!(hierarchy.roots().iter().all(|node| !node.is_group() || node.children().is_empty()));
    }

    #[test]
    fn unified_order_supports_empty_parent_and_nested_group_sequences() {
        let groups = vec![group("Parent", vec![]), group("Child", vec![item("b", "B", &[])]), group("Grandchild", vec![])];
        let display = [(0, 0), (1, 1), (2, 2)];
        let parents = [("Child".into(), "Parent".into()), ("Grandchild".into(), "Child".into())].into();
        let orders = [
            ("Parent".into(), vec![HierarchyOrderEntry::Group("Child".into())]),
            ("Child".into(), vec![HierarchyOrderEntry::Group("Grandchild".into()), HierarchyOrderEntry::Item("b".into())]),
        ]
        .into();

        let segments = group_render_segments(&groups, &display, &parents, &orders);

        assert_eq!(
            segment_summary(&segments, &groups),
            [("Parent".into(), true, vec![]), ("Child".into(), true, vec![]), ("Grandchild".into(), true, vec![]), ("Child".into(), false, vec!["b".into()]),]
        );
        assert_eq!(segments.iter().filter(|segment| segment.show_header).count(), groups.len());
    }

    #[test]
    fn malformed_group_parent_cycles_are_emitted_once_without_recursing_forever() {
        let groups = vec![group("A", vec![]), group("B", vec![])];
        let display = [(0, 0), (1, 0)];
        let parents = [("A".into(), "B".into()), ("B".into(), "A".into())].into();

        let segments = group_render_segments(&groups, &display, &parents, &Default::default());

        assert_eq!(segment_summary(&segments, &groups).iter().map(|(name, _, _)| name.as_str()).collect::<Vec<_>>(), ["A", "B"]);
    }

    fn nested_group_drop_harness(domain_hierarchy_actions: bool, target_nested_card: bool) -> egui_kittest::Harness<'static, (PresetUi, Vec<BrowserAction>)> {
        use egui_kittest::Harness;

        let groups = vec![group("A", vec![item("nested-card", "Nested item", &["Nested"])]), group("B", vec![item("root-card", "Root item", &[])])];
        let mut h = Harness::builder().with_size(egui::vec2(420.0, 360.0)).build_ui_state(
            move |ui, state: &mut (PresetUi, Vec<BrowserAction>)| {
                if !ui.ctx().fonts(|fonts| fonts.definitions().families.contains_key(&egui::FontFamily::Name("semibold".into()))) {
                    return;
                }
                let mut paint_item = |_: &egui::Ui, _: egui::Rect, _: usize, _: usize, _: ItemPaintState| {};
                let mut adapter = BrowserAdapter {
                    panel: "hierarchy-drop-test",
                    groups: &groups,
                    options: BrowserOptions {
                        canvas: egui::Rect::NOTHING,
                        flush: false,
                        center_grid: false,
                        grid_indent: 8.0,
                        all_groups_open: true,
                        compact_context_menus: false,
                        item_size: None,
                        list_item_height: None,
                        domain_hierarchy_actions,
                    },
                    capabilities: BrowserCapabilities { item_drag: false, item_drop: false, group_drag: true, selection: false, activation: false, canonical_hierarchy_drag: false },
                    item_context_menu: None,
                    group_context_menu: None,
                    nested_group_context_menu: None,
                    canonical_hierarchy_move: None,
                    hierarchy_item_move: None,
                    paint_group: None,
                    paint_item: &mut paint_item,
                    paint_list_item: None,
                };
                let (preset_ui, actions) = state;
                actions.extend(browser(ui, preset_ui, &mut adapter, 300.0));
            },
            (PresetUi::default(), Vec::new()),
        );
        crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
        h.run_steps(3);

        let source = super::last_group_headers(&h.ctx, "hierarchy-drop-test").into_iter().find(|(name, _)| name == "B").map(|(_, rect)| rect.center()).unwrap();
        let target = if target_nested_card { h.get_by_label("Nested item").rect() } else { h.get_by_label("Nested").rect() };
        let target = target.center();
        h.event(egui::Event::PointerMoved(source));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: source, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
        for step in 1..=6 {
            h.event(egui::Event::PointerMoved(source + (target - source) * (step as f32 / 6.0)));
            h.run_steps(1);
        }
        h.event(egui::Event::PointerButton { pos: target, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run_steps(3);
        h
    }

    #[test]
    fn root_group_drop_on_nested_header_resolves_once_in_domain_mode() {
        let h = nested_group_drop_harness(true, false);
        assert!(matches!(
            h.state().1.as_slice(),
            [BrowserAction::MoveInto { group, into: Some((target, path)), before: None, before_item: None, .. }]
                if group == "B" && target == "A" && path == &["Nested".to_string()]
        ));
        assert_eq!(h.state().1.len(), 1, "the nested header's MoveInto must suppress the source header's root MoveGroup");
    }

    #[test]
    fn unsupported_root_group_drop_on_nested_header_or_card_is_not_reinterpreted_as_root_reorder() {
        let h = nested_group_drop_harness(false, false);
        assert!(h.state().1.is_empty(), "unsupported nested header must not consume or misroute the release");
        assert_eq!(h.state().0.parent_of("hierarchy-drop-test", "B"), None);
        assert!(!h.state().0.hierarchy_order.get("hierarchy-drop-test").is_some_and(|orders| orders.contains_key("")), "an unsupported nested target must not mutate root group order");
    }

    #[test]
    fn root_group_drop_on_nested_card_updates_the_unified_sibling_sequence() {
        let h = nested_group_drop_harness(false, true);
        assert!(h.state().1.is_empty(), "the transient shared hierarchy handles the drop without a domain command");
        assert_eq!(h.state().0.parent_of("hierarchy-drop-test", "B"), Some("A"));
        assert!(
            h.state().0.hierarchy_order["hierarchy-drop-test"]["A"].contains(&HierarchyOrderEntry::Group("B".into())),
            "the moved group must occupy the same sibling sequence as the target card"
        );
    }
}
