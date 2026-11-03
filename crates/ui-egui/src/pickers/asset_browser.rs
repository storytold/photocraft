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
    asset_browser_header_target_with_capabilities, asset_browser_item_drag_source_with_id, asset_browser_item_gesture, asset_browser_item_hierarchy_target,
    asset_browser_item_target, asset_browser_take_release, card_drop_hit, card_drop_hits, card_leading_marker_rect, card_trailing_marker_rect,
    context_menu_target, draw_group_context_menu, draw_item_context_menu, group_append_hit, group_gap_drop_hits, group_gap_upper_anchor, group_header_drop_hit,
    header_drop_before, hierarchy_group_drop, item_drop_after, item_row_edge_targets, paint_group_insertion_line, root_append_hit, root_prepend_hit,
};
use crate::pickers::asset_browser_items::{item_info as asset_browser_item_info, list_row as asset_browser_list_row};
use crate::pickers::picker_browser_dragging::{
    DragSession, DropCommitResult, DropHitRegion, DropMoveIntent, PointerRegion, arbitrate_hit_index, hierarchy_move_sources, paint_candidate,
};
use crate::pickers::picker_browser_hierarchy::{HierarchyIdentity, commit_hierarchy_move};
use crate::pickers::picker_browser_model::{ContainerId, DropPosition, HierarchyNode, NodeId, PickerBrowserHierarchy};
use crate::pickers::picker_state::escape_hierarchy_key_part;
use crate::pickers::picker_state::{HierarchyOrderEntry, ItemRenameState, PresetUi};
use crate::theme::Tokens;

pub(crate) const MIN_THUMB: f32 = 20.0;
// 27px of header content plus a one-pixel border on each vertical edge.
const GROUP_HEADER_HEIGHT: f32 = 29.0;
const GROUP_LAYOUT_GAP: f32 = 0.0;
// The append line is split into two exact three-pixel targets around the group boundary.
const GROUP_GAP_DROP_HEIGHT: f32 = 6.0;
const GROUP_DRAG_GHOST_OPACITY: f32 = 0.4;
const HIT_DEBUG_LABEL_HEIGHT: f32 = 9.0;

#[derive(Clone, Debug, PartialEq)]
enum BrowserDragGhostSource {
    Group(NodeId),
    Item { group: usize, item: usize },
}

#[cfg(test)]
mod item_batch_tests {
    use super::{BrowserAction, GroupView, ItemView, advance_item_rename, begin_item_rename, delete_item_actions, item_context_targets, move_item_actions};
    use crate::pickers::picker_state::PresetUi;

    #[test]
    fn item_context_actions_target_the_selected_cards_and_keep_single_card_fallback() {
        let clicked_group = GroupView { name: "A".into(), root_only: false, items: Vec::new() };
        let clicked = ItemView { key: "one".into(), name: "One".into(), group_path: Vec::new() };
        let selected = vec![
            ("A".to_owned(), clicked.clone()),
            ("B".to_owned(), ItemView { key: "two".into(), name: "Two".into(), group_path: vec!["Folder".into()] }),
            ("C".to_owned(), ItemView { key: "three".into(), name: "Three".into(), group_path: Vec::new() }),
        ];

        let targets = item_context_targets(&clicked, &clicked_group, &selected);
        assert_eq!(targets.len(), 3);
        assert!(matches!(delete_item_actions(&targets).as_slice(), [
            BrowserAction::DeleteItem { group: a, key: one, .. },
            BrowserAction::DeleteItem { group: b, key: two, group_path, .. },
            BrowserAction::DeleteItem { group: c, key: three, .. },
        ] if a == "A" && one == "one" && b == "B" && two == "two" && group_path == &["Folder".to_owned()] && c == "C" && three == "three"));

        let moves = move_item_actions(&targets, "B");
        assert!(matches!(moves.as_slice(), [BrowserAction::Move(one, target), BrowserAction::Move(three, target_again)]
            if one == "one" && three == "three" && target == "B" && target_again == "B"));

        let unselected = ItemView { key: "outside".into(), name: "Outside".into(), group_path: Vec::new() };
        let fallback = item_context_targets(&unselected, &clicked_group, &selected);
        assert_eq!(fallback.len(), 1);
        assert_eq!(fallback[0].1.key, "outside");
    }

    #[test]
    fn multi_rename_processes_one_selected_item_after_another() {
        let targets = vec![
            ("A".into(), ItemView { key: "first".into(), name: "First".into(), group_path: Vec::new() }),
            ("B".into(), ItemView { key: "second".into(), name: "Second".into(), group_path: vec!["Folder".into()] }),
            ("C".into(), ItemView { key: "third".into(), name: "Third".into(), group_path: Vec::new() }),
        ];
        let mut state = PresetUi::default();
        begin_item_rename(&mut state, "panel", &targets);
        assert_eq!(state.item_renaming.as_ref().map(|rename| rename.key.as_str()), Some("first"));
        assert_eq!(state.item_rename_queue.iter().map(|rename| rename.key.as_str()).collect::<Vec<_>>(), ["second", "third"]);

        state.item_renaming = None; // the first inline editor was committed or cancelled
        advance_item_rename(&mut state, "panel");
        assert_eq!(
            state.item_renaming.as_ref().map(|rename| (rename.key.as_str(), rename.group.as_str(), rename.group_path.as_slice())),
            Some(("second", "B", &["Folder".to_owned()][..]))
        );

        state.item_renaming = None;
        advance_item_rename(&mut state, "panel");
        assert_eq!(state.item_renaming.as_ref().map(|rename| rename.key.as_str()), Some("third"));
        assert!(state.item_rename_queue.is_empty());
    }
}

#[derive(Clone, Debug, PartialEq)]
struct BrowserDragGhost {
    source: BrowserDragGhostSource,
    origin: Rect,
    grab_offset: egui::Vec2,
    press_origin: Option<Pos2>,
}

/// Keep a dragged card/header pressed against the asset-browser viewport edges while the pointer is inside
/// it. Outside the shell, follow the pointer using the original pickup offset.
fn browser_drag_ghost_rect(
    ui: &egui::Ui,
    panel: &str,
    source: BrowserDragGhostSource,
    response_dragged: bool,
    source_rect: Rect,
    picker_bounds: Rect,
) -> Option<Rect> {
    let ctx = ui.ctx();
    let id = egui::Id::new(("preset-browser-drag-ghost", panel));
    let has_panel_payload = egui::DragAndDrop::payload::<AssetDrag>(ctx).is_some_and(|payload| match payload.as_ref() {
        AssetDrag::Preset { scope, .. } | AssetDrag::Group { scope, .. } | AssetDrag::CanonicalGroup { scope, .. } => scope == panel,
    });
    if !has_panel_payload {
        ctx.data_mut(|data| data.remove::<BrowserDragGhost>(id));
        return None;
    }
    if response_dragged && ctx.pointer_interact_pos().is_some() {
        let press_origin = ctx.input(|input| input.pointer.press_origin());
        let already_captured = ctx.data(|data| {
            data.get_temp::<BrowserDragGhost>(id).is_some_and(|ghost| ghost.source == source && (press_origin.is_none() || ghost.press_origin == press_origin))
        });
        if !already_captured {
            let grab_offset =
                press_origin.filter(|origin| source_rect.contains(*origin)).map(|origin| origin - source_rect.min).unwrap_or_else(|| source_rect.size() * 0.5);
            ctx.data_mut(|data| data.insert_temp(id, BrowserDragGhost { source: source.clone(), origin: source_rect, grab_offset, press_origin }));
        }
    }
    let ghost = ctx.data(|data| data.get_temp::<BrowserDragGhost>(id))?;
    if ghost.source != source {
        return None;
    }
    Some(browser_drag_ghost_position(ctx.input(|input| input.pointer.hover_pos()), ghost.origin, ghost.grab_offset, picker_bounds))
}

fn browser_drag_ghost_position(pointer: Option<Pos2>, origin: Rect, grab_offset: egui::Vec2, picker_bounds: Rect) -> Rect {
    let Some(pointer) = pointer else { return origin };
    let pointer_rect = Rect::from_min_size(pointer - grab_offset, origin.size());
    if picker_bounds.contains(pointer) { clamp_drag_ghost(pointer_rect, picker_bounds) } else { pointer_rect }
}

fn clamp_drag_ghost(rect: Rect, bounds: Rect) -> Rect {
    let max_x = (bounds.right() - rect.width()).max(bounds.left());
    let max_y = (bounds.bottom() - rect.height()).max(bounds.top());
    let min = pos2(rect.left().clamp(bounds.left(), max_x), rect.top().clamp(bounds.top(), max_y));
    Rect::from_min_size(min, rect.size())
}

#[cfg(test)]
mod browser_drag_ghost_tests {
    use super::{Rect, browser_drag_ghost_position, clamp_drag_ghost, pos2, vec2};

    #[test]
    fn pointer_leaving_the_window_returns_the_ghost_to_its_exact_pickup_bounds() {
        let origin = Rect::from_min_size(pos2(12.0, 18.0), vec2(80.0, 40.0));
        let picker = Rect::from_min_size(pos2(20.0, 30.0), vec2(200.0, 120.0));
        assert_eq!(browser_drag_ghost_position(None, origin, vec2(15.0, 10.0), picker), origin);
    }

    #[test]
    fn pointer_outside_the_picker_shell_follows_cursor_with_original_pickup_offset() {
        let origin = Rect::from_min_size(pos2(30.0, 45.0), vec2(80.0, 40.0));
        let picker = Rect::from_min_size(pos2(20.0, 30.0), vec2(200.0, 120.0));
        let expected = Rect::from_min_size(pos2(85.0, 160.0), origin.size());
        assert_eq!(browser_drag_ghost_position(Some(pos2(100.0, 170.0)), origin, vec2(15.0, 10.0), picker), expected);
    }

    #[test]
    fn ghost_stays_fully_inside_browser_window_when_pointer_reaches_its_edges() {
        let origin = Rect::from_min_size(pos2(30.0, 45.0), vec2(80.0, 40.0));
        let picker = Rect::from_min_size(pos2(20.0, 30.0), vec2(200.0, 120.0));
        let expected = Rect::from_min_size(pos2(140.0, 110.0), vec2(80.0, 40.0));
        assert_eq!(browser_drag_ghost_position(Some(pos2(215.0, 145.0)), origin, vec2(15.0, 10.0), picker), expected);
    }

    #[test]
    fn dragged_cards_and_groups_press_against_each_window_edge() {
        let window = Rect::from_min_size(pos2(20.0, 30.0), vec2(200.0, 120.0));
        let ghost = Rect::from_min_size(pos2(-15.0, 130.0), vec2(80.0, 40.0));
        assert_eq!(clamp_drag_ghost(ghost, window), Rect::from_min_size(pos2(20.0, 110.0), vec2(80.0, 40.0)));
    }

    #[test]
    fn oversized_drag_ghosts_anchor_to_the_window_origin() {
        let window = Rect::from_min_size(pos2(20.0, 30.0), vec2(80.0, 40.0));
        let ghost = Rect::from_min_size(pos2(-15.0, -15.0), vec2(100.0, 60.0));
        assert_eq!(clamp_drag_ghost(ghost, window), Rect::from_min_size(pos2(20.0, 30.0), vec2(100.0, 60.0)));
    }
}

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
#[derive(Clone)]
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
    /// Domain-only backing container for cards represented directly in the root sequence.
    /// It participates in item identity/commands but has no visible group header of its own.
    pub(crate) root_only: bool,
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
            group_visible_for_query(&query, group_matches, !items.is_empty()).then(|| GroupView { name: group.name.clone(), root_only: group.root_only, items })
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
    root_items: bool,
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
        let child_names: std::collections::BTreeMap<&str, (usize, usize)> =
            children.iter().filter_map(|&(index, child_depth)| groups.get(index).map(|child| (child.name.as_str(), (index, child_depth)))).collect();
        let mut entries = orders.get(&group.name).cloned().unwrap_or_default();
        if !orders.contains_key(&group.name) {
            entries.extend(
                group
                    .items
                    .iter()
                    .filter(|item| !lifted_items.contains(&(group.name.clone(), item.key.clone())))
                    .map(|item| HierarchyOrderEntry::Item(item.key.clone())),
            );
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
        let item_by_key: std::collections::BTreeMap<&str, usize> = group
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| !lifted_items.contains(&(group.name.clone(), item.key.clone())))
            .map(|(index, item)| (item.key.as_str(), index))
            .collect();
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
                            root_items: false,
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
    let mut visited = BTreeSet::new();
    let mut item_indices = std::collections::BTreeMap::new();
    let mut identities = std::collections::BTreeMap::new();
    let mut roots: Vec<HierarchyNode> = display
        .iter()
        .filter_map(|&(index, depth)| (depth == 0).then_some(index))
        .filter(|index| groups.get(*index).is_some_and(|group| !group.root_only))
        .filter_map(|index| build_group(index, 0, groups, display, parents, orders, &mut visited, &mut item_indices, &mut identities, &lifted_items))
        .collect();
    let root_entries = orders.get("").cloned().unwrap_or_default();
    let group_by_name: std::collections::BTreeMap<&str, usize> = groups.iter().enumerate().map(|(i, g)| (g.name.as_str(), i)).collect();
    let root_group_display_order: Vec<String> = roots
        .iter()
        .filter_map(|node| match identities.get(node.id()) {
            Some(HierarchyIdentity::Group(name)) => Some(name.clone()),
            _ => None,
        })
        .collect();
    let mut root_groups: std::collections::BTreeMap<String, HierarchyNode> = roots
        .drain(..)
        .filter_map(|node| {
            let HierarchyIdentity::Group(name) = identities.get(node.id())? else { return None };
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
                let id = item_id(&group_name, &key);
                item_indices.insert(id.clone(), (group_index, item_index));
                identities.insert(id.clone(), HierarchyIdentity::Item { group: group_name.clone(), key: key.clone() });
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
    // Adapter group names are stable unique identities; item IDs are typed group/key pairs, so
    // duplicate card keys across groups remain distinct without delimiter encoding.
    let Ok(hierarchy) = PickerBrowserHierarchy::new(roots) else { return None };
    let group_indices: std::collections::BTreeMap<NodeId, usize> = groups.iter().enumerate().map(|(index, group)| (group_id(&group.name), index)).collect();
    let mut out = Vec::new();
    for root in hierarchy.roots() {
        if let HierarchyNode::Item(id) = root {
            if let Some((group_index, item_index)) = item_indices.get(id) {
                if let Some(segment) = out.last_mut().filter(|segment: &&mut GroupRenderSegment| segment.root_items && segment.group_index == *group_index) {
                    segment.item_indices.push(*item_index);
                } else {
                    out.push(GroupRenderSegment {
                        group_index: *group_index,
                        depth: 0,
                        root_items: true,
                        show_header: false,
                        folder_path: Vec::new(),
                        show_folder_header: false,
                        item_indices: vec![*item_index],
                    });
                }
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
    SelectItem {
        group: String,
        group_path: Vec<String>,
        key: String,
    },
    /// Activate an item, typically on a double-click.
    Activate(String),
    /// Activate one item occurrence, qualified by its owning root group.
    ActivateItem {
        group: String,
        group_path: Vec<String>,
        key: String,
    },
    /// Drop an item onto a canvas position in screen coordinates.
    Drop(String, Pos2),
    /// Rename an item from its stable key to a new display name.
    Rename(String, String),
    RenameItem {
        group: String,
        group_path: Vec<String>,
        key: String,
        text: String,
    },
    /// Delete the item identified by its stable key.
    Delete(String),
    DeleteItem {
        group: String,
        group_path: Vec<String>,
        key: String,
    },
    /// Move an item into the named group.
    Move(String, String),
    /// Move an item into a group at a specific final insertion index.
    MoveItem(String, String, Option<usize>),
    /// Rename a group from its current name to a new name.
    RenameGroup(String, String),
    /// Delete a group and its contents.
    DeleteGroup(String),
    /// Rename a nested group identified by its owning root group and full path.
    RenameNestedGroup {
        group: String,
        group_path: Vec<String>,
        text: String,
    },
    /// Delete a nested group and its contents, identified by its owning root group and full path.
    DeleteNestedGroup {
        group: String,
        group_path: Vec<String>,
    },
    /// Request creation of a new group.
    NewGroup,
    /// Request creation of a new item.
    New,
    /// Move an item into a nested group; an empty path means directly into the root group.
    /// `before` identifies the destination sibling to insert before; `None` appends.
    MoveItemToGroup {
        key: String,
        group: String,
        group_path: Vec<String>,
        before: Option<String>,
    },
    /// Reorder a root or nested group among siblings. An empty `group_path` identifies the root group;
    /// `before=None` appends it after its siblings.
    MoveGroup {
        group: String,
        group_path: Vec<String>,
        before: Option<String>,
    },
    /// Move a root or nested group into a destination hierarchy. `into=None` promotes it to a
    /// top-level group; an empty destination path means directly inside that group. `before=None`
    /// appends within the destination.
    MoveInto {
        group: String,
        group_path: Vec<String>,
        into: Option<(String, Vec<String>)>,
        before: Option<String>,
        before_item: Option<String>,
    },
}

fn hierarchy_action(drop: HierarchyDrop) -> BrowserAction {
    match drop {
        HierarchyDrop::MoveGroup { group, group_path, before } => BrowserAction::MoveGroup { group, group_path, before },
        HierarchyDrop::MoveInto { group, group_path, into, before } => BrowserAction::MoveInto { group, group_path, into, before, before_item: None },
    }
}

fn source_node_for_drag(drag: &AssetDrag, panel: &str, identities: &std::collections::BTreeMap<NodeId, HierarchyIdentity>) -> Option<NodeId> {
    if let AssetDrag::CanonicalGroup { scope, node_id, .. } = drag {
        return (scope == panel && matches!(identities.get(node_id), Some(HierarchyIdentity::Group(_) | HierarchyIdentity::Folder { .. })))
            .then(|| node_id.clone());
    }
    if let AssetDrag::Preset { scope, key, node_id, .. } = drag {
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

/// Return the selected visible nodes that should move as independent hierarchy roots.
/// Selecting both a group and one of its descendants moves the group subtree only.
fn selected_drag_nodes(state: &PresetUi, panel: &str, visible_order: &[NodeId], source: &NodeId, hierarchy: &PickerBrowserHierarchy) -> Vec<NodeId> {
    let Some(selected) = state.browser_selection.get(panel).filter(|selected| selected.contains(source)) else {
        return Vec::new();
    };

    visible_order
        .iter()
        .filter(|id| selected.contains(*id))
        .filter(|id| {
            let mut parent = hierarchy.parent_of(id);
            while let Some(ContainerId::Group(parent_id)) = parent {
                if selected.contains(&parent_id) {
                    return false;
                }
                parent = hierarchy.parent_of(&parent_id);
            }
            true
        })
        .cloned()
        .collect()
}

/// Plan item-domain callback indices against the same sequence of moves that will be persisted
/// to the canonical hierarchy. The simulated tree makes indices advance correctly when several
/// selected cards are inserted into one destination container.
fn plan_hierarchy_item_moves(
    hierarchy: &PickerBrowserHierarchy,
    sources: &[NodeId],
    position: &DropPosition,
    identities: &std::collections::BTreeMap<NodeId, HierarchyIdentity>,
    root_item_group: Option<&str>,
) -> Result<Vec<(String, String, String, usize)>, String> {
    let mut planned_hierarchy = hierarchy.clone();
    let mut item_moves = Vec::new();
    for source in hierarchy_move_sources(sources, position) {
        if let Some(HierarchyIdentity::Item { group, key }) = identities.get(&source) {
            let resolved = planned_hierarchy.resolve_move(&source, position).map_err(|error| format!("Could not resolve item move: {error:?}"))?;
            let destination_group = match resolved.destination() {
                ContainerId::Group(destination_id) => {
                    let Some(HierarchyIdentity::Group(destination_group)) = identities.get(destination_id) else {
                        return Err("The destination group is no longer available.".to_owned());
                    };
                    Some(destination_group.as_str())
                }
                ContainerId::Root => root_item_group,
            };
            if let Some(destination_group) = destination_group {
                let children = planned_hierarchy.children_of(resolved.destination()).ok_or_else(|| "The destination is no longer available.".to_owned())?;
                let same_container = planned_hierarchy.parent_of(&source).as_ref() == Some(resolved.destination());
                let domain_index = children
                    .iter()
                    .filter(|node| !same_container || node.id() != &source)
                    .take(resolved.insertion_index())
                    .filter(|node| matches!(node, HierarchyNode::Item(_)))
                    .count();
                item_moves.push((group.clone(), key.clone(), destination_group.to_owned(), domain_index));
            }
        }
        planned_hierarchy.move_node(&source, position).map_err(|error| format!("Could not plan selected hierarchy move: {error:?}"))?;
    }
    Ok(item_moves)
}

#[cfg(test)]
mod selected_drag_nodes_tests {
    use std::collections::BTreeMap;

    use super::{plan_hierarchy_item_moves, selected_drag_nodes};
    use crate::pickers::picker_browser_hierarchy::HierarchyIdentity;
    use crate::pickers::picker_browser_model::{DropPosition, HierarchyNode, NodeId, PickerBrowserHierarchy};
    use crate::pickers::picker_state::PresetUi;

    #[test]
    fn selected_ancestor_subsumes_descendants_and_keeps_independent_nodes_in_visible_order() {
        let hierarchy = PickerBrowserHierarchy::new(vec![
            HierarchyNode::group("A", vec![HierarchyNode::item("a1"), HierarchyNode::group("Nested", vec![HierarchyNode::item("nested-card")])]),
            HierarchyNode::group("B", vec![HierarchyNode::item("b1")]),
        ])
        .unwrap();
        let visible = ["A", "a1", "Nested", "nested-card", "B", "b1"].map(NodeId::from);
        let mut state = PresetUi::default();
        for (index, id) in ["A", "a1", "Nested", "nested-card", "b1"].into_iter().map(NodeId::from).enumerate() {
            state.select_browser_node("panel", id, index != 0);
        }

        assert_eq!(selected_drag_nodes(&state, "panel", &visible, &NodeId::from("a1"), &hierarchy), [NodeId::from("A"), NodeId::from("b1")]);
    }

    #[test]
    fn dragging_an_unselected_node_does_not_move_the_other_selected_nodes() {
        let hierarchy = PickerBrowserHierarchy::new(vec![HierarchyNode::group("A", vec![HierarchyNode::item("a1")])]).unwrap();
        let mut state = PresetUi::default();
        state.select_browser_node("panel", NodeId::from("A"), false);

        assert!(selected_drag_nodes(&state, "panel", &[NodeId::from("A"), NodeId::from("a1")], &NodeId::from("a1"), &hierarchy).is_empty());
    }

    #[test]
    fn item_callback_indices_follow_prior_selected_moves_for_every_insertion_position() {
        let hierarchy = PickerBrowserHierarchy::new(vec![
            HierarchyNode::group(
                NodeId::group("A"),
                vec![HierarchyNode::item(NodeId::item("A", "a1")), HierarchyNode::item(NodeId::item("A", "a2")), HierarchyNode::item(NodeId::item("A", "a3"))],
            ),
            HierarchyNode::group(NodeId::group("B"), vec![HierarchyNode::item(NodeId::item("B", "b1")), HierarchyNode::item(NodeId::item("B", "b2"))]),
        ])
        .unwrap();
        let identities = BTreeMap::from([
            (NodeId::group("A"), HierarchyIdentity::Group("A".into())),
            (NodeId::item("A", "a1"), HierarchyIdentity::Item { group: "A".into(), key: "a1".into() }),
            (NodeId::item("A", "a2"), HierarchyIdentity::Item { group: "A".into(), key: "a2".into() }),
            (NodeId::item("A", "a3"), HierarchyIdentity::Item { group: "A".into(), key: "a3".into() }),
            (NodeId::group("B"), HierarchyIdentity::Group("B".into())),
            (NodeId::item("B", "b1"), HierarchyIdentity::Item { group: "B".into(), key: "b1".into() }),
            (NodeId::item("B", "b2"), HierarchyIdentity::Item { group: "B".into(), key: "b2".into() }),
        ]);
        let sources = [NodeId::item("A", "a1"), NodeId::item("A", "a2")];
        let cases = [
            (DropPosition::Before { target: NodeId::item("B", "b1") }, vec![("a1", 0), ("a2", 1)]),
            (DropPosition::After { target: NodeId::item("B", "b1") }, vec![("a2", 1), ("a1", 1)]),
            (DropPosition::PrependTo { group: NodeId::group("B") }, vec![("a2", 0), ("a1", 0)]),
            (DropPosition::AppendTo { container: crate::pickers::picker_browser_model::ContainerId::Group(NodeId::group("B")) }, vec![("a1", 2), ("a2", 3)]),
            (DropPosition::After { target: NodeId::item("A", "a3") }, vec![("a2", 2), ("a1", 1)]),
        ];

        for (position, expected) in cases {
            let actual = plan_hierarchy_item_moves(&hierarchy, &sources, &position, &identities, None).unwrap();
            assert_eq!(actual.iter().map(|(_, key, _, index)| (key.as_str(), *index)).collect::<Vec<_>>(), expected, "position={position:?}");
        }
    }
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
/// Cards use a consistent border and two-pixel inter-card spacing across picker consumers.
#[derive(Clone, Copy)]
pub(crate) struct BrowserOptions {
    /// Canvas hit region used to distinguish drops onto the canvas from drops into the browser.
    pub(crate) canvas: Rect,
    /// Legacy layout preference retained for existing consumers; card borders define spacing.
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
    /// Optional domain-owned destination for cards dropped into the implicit root sequence.
    /// Consumers without a persistent root owner keep root placement in picker layout state.
    pub(crate) root_item_group: Option<&'static str>,
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
pub(crate) type ItemContextMenu = fn(&mut egui::Ui, &mut PresetUi, &str, &ItemView, &GroupView, &[String], &[(String, ItemView)], &mut Vec<BrowserAction>);
/// Consumer callback that appends generic events for actions offered on a group header.
pub(crate) type GroupContextMenu = fn(&mut egui::Ui, &mut PresetUi, &str, &GroupView, &mut Vec<BrowserAction>);
/// Consumer callback for rename/delete operations on a nested group, with its full path.
pub(crate) type NestedGroupContextMenu = fn(&mut egui::Ui, &mut PresetUi, &str, &GroupView, &[String], &mut Vec<BrowserAction>);

/// Resolve the context-menu target set. Right-clicking one of the selected cards acts on the whole
/// selection; right-clicking an unselected card preserves the conventional single-card operation.
pub(crate) fn item_context_targets(item: &ItemView, group: &GroupView, selected: &[(String, ItemView)]) -> Vec<(String, ItemView)> {
    if selected
        .iter()
        .any(|(source_group, selected_item)| source_group == &group.name && selected_item.key == item.key && selected_item.group_path == item.group_path)
    {
        selected.to_vec()
    } else {
        vec![(group.name.clone(), item.clone())]
    }
}

pub(crate) fn delete_item_actions(targets: &[(String, ItemView)]) -> Vec<BrowserAction> {
    targets.iter().map(|(group, item)| BrowserAction::DeleteItem { group: group.clone(), group_path: item.group_path.clone(), key: item.key.clone() }).collect()
}

pub(crate) fn move_item_actions(targets: &[(String, ItemView)], destination: &str) -> Vec<BrowserAction> {
    targets.iter().filter(|(group, _)| group != destination).map(|(_, item)| BrowserAction::Move(item.key.clone(), destination.to_owned())).collect()
}

pub(crate) fn begin_item_rename(st: &mut PresetUi, panel: &str, targets: &[(String, ItemView)]) {
    let mut renames = targets.iter().map(|(group, item)| ItemRenameState {
        panel: panel.to_owned(),
        group: group.clone(),
        group_path: item.group_path.clone(),
        key: item.key.clone(),
        text: item.name.clone(),
    });
    st.item_renaming = renames.next();
    st.item_rename_queue = renames.collect();
}

fn advance_item_rename(st: &mut PresetUi, panel: &str) {
    if st.item_renaming.is_none()
        && let Some(index) = st.item_rename_queue.iter().position(|rename| rename.panel == panel)
    {
        st.item_renaming = Some(st.item_rename_queue.remove(index));
    }
}

/// Default menu policy for the standard preset panels. Keeping this separate from the browser
/// lets specialised consumers supply their own operations without forking the shared hit targets.
pub(crate) fn standard_item_context_menu(
    ui: &mut egui::Ui,
    st: &mut PresetUi,
    panel: &str,
    item: &ItemView,
    group: &GroupView,
    group_names: &[String],
    selected_items: &[(String, ItemView)],
    ev: &mut Vec<BrowserAction>,
) {
    let targets = item_context_targets(item, group, selected_items);
    let count = targets.len();
    let delete_label = if count > 1 { format!("{} ({count})", tl!("Delete")) } else { tl!("Delete").to_string() };
    let all_in_current_group = targets.iter().all(|(source_group, _)| source_group == &group.name);
    match draw_item_context_menu(ui, tl!("Rename…"), &delete_label, Some((tl!("Move to"), group_names, if all_in_current_group { &group.name } else { "" })))
    {
        Some(ContextMenuAction::Rename) => begin_item_rename(st, panel, &targets),
        Some(ContextMenuAction::Delete) => ev.extend(delete_item_actions(&targets)),
        Some(ContextMenuAction::MoveTo(destination)) => ev.extend(move_item_actions(&targets, &destination)),
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
        grid_indent: _,
        all_groups_open,
        compact_context_menus,
        item_size,
        list_item_height,
        domain_hierarchy_actions,
        root_item_group,
    } = adapter.options;
    // Kept in the options shape for existing consumers; spacing/borders are now consistent.
    let _legacy_flush_layout = flush;
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
    advance_item_rename(st, panel);
    let item_rename = st.item_renaming.clone().filter(|rename| rename.panel == panel);
    let nested_group_rename = st.group_renaming.clone().filter(|rename| rename.panel == panel);
    let has_rename_row = item_rename.is_some() || nested_group_rename.is_some() || st.renaming.as_ref().is_some_and(|r| r.0 == panel);
    // Rename field (Photoshop opens a small dialog; inline keeps the panel modeless).
    if let Some(mut rename) = item_rename {
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
        st.item_renaming = Some(rename.clone());
        if let Some(ok) = done {
            st.item_renaming = None;
            if ok && !text.trim().is_empty() {
                ev.push(BrowserAction::RenameItem { group: rename.group, group_path: rename.group_path, key: rename.key, text: text.trim().to_string() });
            }
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
    let canonical_node_bounds = std::cell::RefCell::new(std::collections::BTreeMap::<NodeId, Rect>::new());
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
    let visible_group_indices: Vec<usize> = groups.iter().enumerate().filter_map(|(index, group)| (!group.root_only).then_some(index)).collect();
    let group_names: Vec<String> = visible_group_indices.iter().filter_map(|index| groups.get(*index).map(|group| group.name.clone())).collect();
    // Project the canonical mixed sibling sequences into the transient validated browser tree.
    let display: Vec<(usize, usize)> = if domain_hierarchy_actions {
        visible_group_indices.iter().map(|index| (*index, 0)).collect()
    } else {
        st.nested_display_order(panel, &group_names)
            .into_iter()
            .filter_map(|(visible_index, depth)| visible_group_indices.get(visible_index).map(|index| (*index, depth)))
            .collect()
    };
    let parents = if domain_hierarchy_actions { std::collections::BTreeMap::new() } else { st.group_parents(panel) };
    let mut hierarchy_orders = std::collections::BTreeMap::new();
    let root_entries =
        if domain_hierarchy_actions { Vec::new() } else { st.hierarchy_order.get(panel).and_then(|orders| orders.get("")).cloned().unwrap_or_default() };
    let lifted_items: BTreeSet<(String, String)> = root_entries
        .iter()
        .filter_map(|entry| match entry {
            HierarchyOrderEntry::RootItem { group, key } => Some((group.clone(), key.clone())),
            _ => None,
        })
        .collect();
    for group in groups {
        let child_groups: Vec<String> = display
            .iter()
            .filter_map(|(child_index, _)| {
                let child = groups.get(*child_index)?;
                (parents.get(&child.name).is_some_and(|parent| parent == &group.name)).then(|| child.name.clone())
            })
            .collect();
        let item_keys: Vec<String> =
            group.items.iter().map(|item| item.key.clone()).filter(|key| !lifted_items.contains(&(group.name.clone(), key.clone()))).collect();
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
    let mut selected_items = Vec::new();
    if let Some(selection) = st.browser_selection.get(panel) {
        for group in groups {
            for item in &group.items {
                if item_node_for_identity(&group.name, &item.group_path, &item.key, &hierarchy_identities).is_some_and(|id| selection.contains(&id)) {
                    selected_items.push((group.name.clone(), item.clone()));
                }
            }
        }
    }
    // Retain the shell's horizontal clip for hierarchy hit arbitration; drag-ghost movement uses
    // the narrower ScrollArea viewport captured inside the closure below.
    let browser_parent_clip = ui.clip_rect();
    // Selection ranges follow the same depth-first sequence the browser actually exposes. A
    // collapsed group contributes its header only; descendants behind it never enter this list.
    let mut visible_selection_order = Vec::new();
    let mut selection_hidden_groups = BTreeSet::new();
    let mut selection_hidden_folders = BTreeSet::<(String, Vec<String>)>::new();
    for segment in &render_segments {
        let Some(group) = groups.get(segment.group_index) else { continue };
        let group_matches = group_matches_query(group, &query);
        let has_matching_items = group.items.iter().any(|item| item_matches_query(item, &query));
        if !group_visible_for_query(&query, group_matches, has_matching_items) {
            continue;
        }
        if segment.show_header {
            if parents.get(&group.name).is_some_and(|parent| selection_hidden_groups.contains(parent)) {
                selection_hidden_groups.insert(group.name.clone());
                continue;
            }
            let open = !query.is_empty() || group_open(st, panel, segment.group_index, &group.name, all_groups_open);
            visible_selection_order.push(NodeId::group(&group.name));
            if !open {
                selection_hidden_groups.insert(group.name.clone());
            }
        } else if selection_hidden_groups.contains(&group.name) {
            continue;
        }
        if !segment.folder_path.is_empty()
            && (1..=segment.folder_path.len()).any(|length| selection_hidden_folders.contains(&(group.name.clone(), segment.folder_path[..length].to_vec())))
        {
            continue;
        }
        let folder_open = segment.folder_path.is_empty() || {
            let path = segment.folder_path.iter().map(|part| escape_hierarchy_key_part(part)).collect::<Vec<_>>().join("/");
            !st.toggled.contains(&format!("{panel}/{}/{}", escape_hierarchy_key_part(&group.name), path))
        };
        if segment.show_folder_header {
            let name = segment.folder_path.last().cloned().unwrap_or_default();
            visible_selection_order.push(NodeId::group_path(
                std::iter::once(group.name.clone()).chain(segment.folder_path.iter().take(segment.folder_path.len().saturating_sub(1)).cloned()),
                name,
            ));
            if !folder_open {
                selection_hidden_folders.insert((group.name.clone(), segment.folder_path.clone()));
            }
        }
        if folder_open {
            for index in &segment.item_indices {
                if let Some(item) = group.items.get(*index).filter(|item| item_visible_for_query(item, &query, group_matches))
                    && let Some(id) = item_node_for_identity(&group.name, &item.group_path, &item.key, &hierarchy_identities)
                {
                    visible_selection_order.push(id);
                }
            }
        }
    }
    // The browser is the flexible middle region of a picker panel. Keep it at the allocated
    // height even when the library is short; otherwise the footer floats up and each consumer's
    // panel geometry depends on how many presets happen to be installed.
    // Foreground ghosts are clipped only to the app viewport; their movement bounds are the
    // ScrollArea viewport below (the asset browser between the shell header and footer).
    let app_window_clip = ui.ctx().content_rect();
    // Drag previews are collected while rows are laid out, then painted from the parent UI.
    // A child created inside ScrollArea inherits its painter's viewport clip, which cannot be
    // expanded to show the preview elsewhere in the picker window.
    let mut group_drag_ghosts = Vec::<(Rect, usize, bool, usize)>::new();
    let mut nested_group_drag_ghosts = Vec::<(Rect, bool, String, usize, usize)>::new();
    let mut item_drag_ghosts = Vec::<(Rect, usize, usize)>::new();
    let empty_area_clicked = std::cell::Cell::new(false);
    let scroll = egui::ScrollArea::vertical()
        .id_salt(("preset-scroll", panel))
        .max_height(scroll_height)
        .min_scrolled_height(scroll_height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let browser_bounds = ui.clip_rect();
            // Register the viewport as a background click target before its rows/cards. Later
            // child widgets take precedence, so only genuinely empty browser space clears the
            // multi-selection. The viewport excludes the shell-owned header and footer.
            let empty_area = ui.interact(browser_bounds, ui.id().with(("browser-empty-area", panel)), Sense::click());
            empty_area_clicked.set(empty_area.clicked());
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
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
            // The root is a real sibling sequence, not merely the area after the final group.
            // Give its leading edge an overlay drop target; it must not reserve vertical layout space.
            if canonical_hierarchy_drag && let Some(first) = hierarchy.roots().first() {
                let top = ui.cursor().min.y;
                let rect = Rect::from_min_max(pos2(ui.clip_rect().left(), top), pos2(ui.clip_rect().right(), top + 1.0));
                let response = ui.interact(rect, ui.id().with(("root-leading-group-drop", panel)), Sense::hover());
                let indicator = if first.is_group() {
                    // The top boundary is flush with the viewport's clip edge. Paint all four
                    // pixels inward so the shell/ScrollArea clip cannot trim the outer half.
                    Rect::from_min_max(pos2(ui.clip_rect().left(), top), pos2(ui.clip_rect().right(), top + 4.0))
                } else {
                    card_leading_marker_rect(rect)
                };
                // The leading border is the insertion boundary itself. Give it precedence over
                // the group-body hit registered immediately afterward, without adding layout space.
                canonical_drop_hits.borrow_mut().push((root_prepend_hit(rect, indicator, first.id().clone(), usize::MAX), response));
            }
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
                // Keep the visible separator compact; drag hit-testing has a larger invisible target.
                if !gap_headers.is_empty() {
                    ui.add_space((GROUP_LAYOUT_GAP - ui.spacing().item_spacing.y).max(0.0));
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
                let header_rect = inset_row_rect(hr, depth);
                let disclosure_rect = Rect::from_min_max(
                    pos2(header_rect.left(), header_rect.top()),
                    pos2((header_rect.left() + 18.0).min(header_rect.right()), header_rect.bottom()),
                );
                let disclosure = ui.interact(disclosure_rect, hresp.id.with("disclosure"), Sense::click());
                if capabilities.group_drag && hresp.is_pointer_button_down_on() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                } else if capabilities.group_drag && hresp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                gap_headers.push((NodeId::group(&g.name), header_rect, depth, open));
                canonical_node_bounds.borrow_mut().insert(NodeId::group(&g.name), header_rect);
                let header_released = if canonical_hierarchy_drag {
                    if capabilities.group_drag {
                        let source_id = NodeId::group(&g.name);
                        let selection = selected_drag_nodes(st, panel, &visible_selection_order, &source_id, &hierarchy);
                        asset_browser_header_drag_source_with_id(&hresp, panel, source_id, &selection);
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
                if hresp.clicked() && !disclosure.clicked() {
                    let modifiers = ui.input(|input| input.modifiers);
                    if capabilities.selection {
                        let id = NodeId::group(&g.name);
                        if modifiers.shift {
                            st.select_browser_range(panel, &visible_selection_order, &id);
                        } else {
                            st.select_browser_node(panel, id, modifiers.ctrl);
                        }
                    }
                }
                if disclosure.clicked() {
                    let k = format!("{panel}/{}", escape_hierarchy_key_part(&g.name));
                    if !st.toggled.remove(&k) {
                        st.toggled.insert(k);
                    }
                }
                if st.browser_node_selected(panel, &NodeId::group(&g.name)) {
                    ui.painter().rect_filled(header_rect, egui::CornerRadius::ZERO, t.row_selected.gamma_multiply(0.35));
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
                if capabilities.group_drag && hresp.dragged() && let Some(p) = ui.ctx().pointer_interact_pos() {
                    drag = Some((gi, p));
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                }
                if capabilities.group_drag
                    && let Some(gr) = browser_drag_ghost_rect(
                        ui,
                        panel,
                        BrowserDragGhostSource::Group(NodeId::group(&g.name)),
                        hresp.dragged(),
                        hr,
                        browser_bounds,
                    )
                {
                    group_drag_ghosts.push((gr, gi, open, depth));
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
                                ui.add_space((GROUP_LAYOUT_GAP - ui.spacing().item_spacing.y).max(0.0));
                            }
                            let (allocated_rect, allocated) = ui.allocate_exact_size(vec2(ui.available_width(), GROUP_HEADER_HEIGHT), Sense::hover());
                            let fr = Rect::from_min_max(pos2(ui.clip_rect().left(), allocated_rect.top()), pos2(allocated_rect.right(), allocated_rect.bottom()));
                            let row_depth = group_depth + 1 + depth;
                            let fresp = ui.interact(inset_row_rect(fr, row_depth), allocated.id, Sense::click_and_drag());
                            let nested_header_rect = inset_row_rect(fr, row_depth);
                            let disclosure_rect = Rect::from_min_max(
                                nested_header_rect.min,
                                pos2((nested_header_rect.left() + 18.0).min(nested_header_rect.right()), nested_header_rect.bottom()),
                            );
                            let disclosure = ui.interact(disclosure_rect, fresp.id.with("disclosure"), Sense::click());
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
                            if capabilities.group_drag
                                && let Some(ghost_rect) = browser_drag_ghost_rect(
                                    ui,
                                    panel,
                                    BrowserDragGhostSource::Group(node_id.clone()),
                                    fresp.dragged(),
                                    fr,
                                    browser_bounds,
                                )
                            {
                                nested_group_drag_ghosts.push((ghost_rect, group_open, name.to_owned(), row_depth, group.count));
                            }
                            if fresp.clicked() && !disclosure.clicked() {
                                let modifiers = ui.input(|input| input.modifiers);
                                if capabilities.selection {
                                    if modifiers.shift {
                                        st_cell.borrow_mut().select_browser_range(panel, &visible_selection_order, &node_id);
                                    } else {
                                        st_cell.borrow_mut().select_browser_node(panel, node_id.clone(), modifiers.ctrl);
                                    }
                                }
                            }
                            if disclosure.clicked() {
                                group_toggles.push(format!("{panel}/{}/{}", escape_hierarchy_key_part(&g.name), group.path.iter().map(|part| escape_hierarchy_key_part(part)).collect::<Vec<_>>().join("/")));
                            }
                            if st_cell.borrow().browser_node_selected(panel, &node_id) {
                                ui.painter().rect_filled(inset_row_rect(fr, row_depth), egui::CornerRadius::ZERO, t.row_selected.gamma_multiply(0.35));
                            }
                            gap_headers.push((node_id.clone(), inset_row_rect(fr, row_depth), row_depth, group_open));
                            canonical_node_bounds.borrow_mut().insert(node_id.clone(), inset_row_rect(fr, row_depth));
                            if canonical_hierarchy_drag {
                                if capabilities.group_drag {
                                    let selection = selected_drag_nodes(&st_cell.borrow(), panel, &visible_selection_order, &node_id, &hierarchy);
                                    asset_browser_header_drag_source_with_id(&fresp, panel, node_id.clone(), &selection);
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
                            let w = ui.available_width();
                            let root_level_items = segment.root_items;
                            // Cards are children of their containing group, so they take one
                            // additional twenty-pixel step beyond that group's header. Apply
                            // the same rule at every nested depth. Root-level cards are siblings
                            // of groups and retain only their small safety inset.
                            let hierarchy_indent = row_indent(depth + group_path.len() + 1);
                            let content_indent = if root_level_items { 0.0 } else { hierarchy_indent };
                            let pad = content_indent;
                            let gap = if list { 0.0 } else { GRID_GAP };
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
                            // Anchor to the same clip edge as group headers so the full hierarchy
                            // indent is applied exactly once, not on top of the area's base inset.
                            let item_origin_x = ui.clip_rect().left() + 1.0;
                            if grid_height > 0.0 {
                                group_content_areas.push((g.name.clone(), area));
                            }
                            let mut insertion_markers = Vec::new();
                            for (run_i, it) in items.iter().enumerate() {
                                let ii = g.items.iter().position(|original| std::ptr::eq(original, *it)).unwrap_or(run_i);
                                let (cx, cy) = ((run_i % cols) as f32, (run_i / cols) as f32);
                                let r = Rect::from_min_size(pos2(item_origin_x + pad + center_offset + cx * (cell.x + gap), area.min.y + cy * (cell.y + gap)), cell);
                                let next_rect = (run_i + 1 < items.len()).then(|| {
                                    let next_i = run_i + 1;
                                    let (next_x, next_y) = ((next_i % cols) as f32, (next_i / cols) as f32);
                                    let next = Rect::from_min_size(
                                        pos2(item_origin_x + pad + center_offset + next_x * (cell.x + gap), area.min.y + next_y * (cell.y + gap)),
                                        cell,
                                    );
                                    next
                                });
                                let resp = ui.interact(r, ui.id().with((panel, gi, ii)), Sense::click_and_drag());
                                let node_id = item_node_for_identity(&g.name, &it.group_path, &it.key, &hierarchy_identities);
                                let is_sel = {
                                    let state = st_cell.borrow();
                                    if state.browser_selection.contains_key(panel) {
                                        node_id.as_ref().is_some_and(|id| state.browser_node_selected(panel, id))
                                    } else {
                                        state.is_selected_item(panel, &g.name, &it.group_path, &it.key)
                                    }
                                };
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
                                    ui.painter().rect_stroke(
                                        r,
                                        CornerRadius::ZERO,
                                        Stroke::new(if is_sel { 2.0 } else { 1.0 }, if is_sel { t.accent } else { t.card_border }),
                                        // Keep the entire border inside the allocated card rect:
                                        // it consumes card dimensions and cannot overlap neighbours.
                                        egui::StrokeKind::Inside,
                                    );
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
                                            let selected_nodes = selected_drag_nodes(&st_cell.borrow(), panel, &visible_selection_order, &source_id, &hierarchy);
                                            asset_browser_item_drag_source_with_id(&resp, panel, &it.key, source_id, &selected_nodes);
                                        }
                                    }
                                    if let Some(target_id) = item_node_for_identity(&g.name, &it.group_path, &it.key, &hierarchy_identities) {
                                        canonical_node_bounds.borrow_mut().insert(target_id.clone(), r);
                                        let hit_index = canonical_drop_hits.borrow().len();
                                        // The same hit widget backs both cards and list rows. In
                                        // list mode its leading/trailing halves are top/bottom;
                                        // card_drop_hits owns that orientation and the matching
                                        // horizontal insertion preview.
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
                                // Grid rows leave horizontal whitespace after their last card,
                                // so expose that strip as an extra target. A list row already spans
                                // the available width and its lower half is the trailing target.
                                // Prepend stays within the card/row's leading half in both layouts.
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
                                if selected && let Some(id) = node_id {
                                    let modifiers = ui.input(|input| input.modifiers);
                                    if modifiers.shift {
                                        st_cell.borrow_mut().select_browser_range(panel, &visible_selection_order, &id);
                                    } else {
                                        st_cell.borrow_mut().select_browser_node(panel, id, modifiers.ctrl);
                                    }
                                }
                                if activated {
                                    ev_cell.borrow_mut().push(BrowserAction::ActivateItem { group: g.name.clone(), group_path: it.group_path.clone(), key: it.key.clone() });
                                } else if selected {
                                    ev_cell.borrow_mut().push(BrowserAction::SelectItem { group: g.name.clone(), group_path: it.group_path.clone(), key: it.key.clone() });
                                }
                                if capabilities.item_drag && resp.dragged() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                                }
                                if capabilities.item_drag
                                    && let Some(ghost_rect) = browser_drag_ghost_rect(
                                        ui,
                                        panel,
                                        BrowserDragGhostSource::Item { group: gi, item: ii },
                                        resp.dragged(),
                                        r,
                                        browser_bounds,
                                    )
                                {
                                    item_drag_ghosts.push((ghost_rect, gi, ii));
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
                                            menu(ui, &mut st_cell.borrow_mut(), panel, it, g, &group_names, &selected_items, &mut ev_cell.borrow_mut())
                                        });
                                    } else {
                                        context_menu_target(resp.clone(), |ui| {
                                            menu(ui, &mut st_cell.borrow_mut(), panel, it, g, &group_names, &selected_items, &mut ev_cell.borrow_mut())
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
            // Each adjacent visible group pair gets a six-pixel target centered on the exact
            // boundary at the lower header's top. The upper half appends within the preceding
            // group after its last direct child; the lower half inserts before the lower group.
            if canonical_hierarchy_drag {
                for pair in gap_headers.windows(2) {
                    let (upper_id, _upper_rect, _upper_depth, _upper_open) = &pair[0];
                    let (lower_id, lower_rect, lower_depth, _) = &pair[1];
                    let gap_rect = Rect::from_min_max(
                        pos2(lower_rect.left(), lower_rect.top() - GROUP_GAP_DROP_HEIGHT * 0.5),
                        pos2(lower_rect.right(), lower_rect.top() + GROUP_GAP_DROP_HEIGHT * 0.5),
                    );
                    if gap_rect.is_positive() {
                        let response = ui.interact(
                            gap_rect,
                            ui.id().with(("group-gap", panel, upper_id, lower_id)),
                            Sense::hover(),
                        );
                        let upper_anchor = group_gap_upper_anchor(&hierarchy, lower_id);
                        let upper_parent = hierarchy.parent_of(lower_id).unwrap_or(ContainerId::Root);
                        let parent_depth = match &upper_parent {
                            ContainerId::Root => 0,
                            ContainerId::Group(parent_id) => {
                                let mut depth = 0;
                                let mut ancestor = hierarchy.parent_of(parent_id).unwrap_or(ContainerId::Root);
                                while let ContainerId::Group(id) = ancestor {
                                    depth += 1;
                                    ancestor = hierarchy.parent_of(&id).unwrap_or(ContainerId::Root);
                                }
                                depth
                            }
                        };
                        let upper_bounds = upper_anchor
                            .as_ref()
                            .and_then(|(anchor, _)| canonical_node_bounds.borrow().get(anchor).copied())
                            .or_else(|| match &upper_parent {
                                ContainerId::Root => None,
                                ContainerId::Group(parent_id) => canonical_node_bounds.borrow().get(parent_id).copied(),
                            })
                            .unwrap_or(*_upper_rect);
                        let upper_target = upper_anchor.as_ref().map(|(anchor, _)| anchor.clone()).unwrap_or_else(|| upper_id.clone());
                        let lower_bounds = canonical_node_bounds.borrow().get(lower_id).copied().unwrap_or(*lower_rect);
                        let mut hits = group_gap_drop_hits(
                            gap_rect,
                            upper_target,
                            lower_id.clone(),
                            upper_bounds,
                            lower_bounds,
                            upper_anchor.as_ref().map(|(_, depth)| *depth).unwrap_or(parent_depth),
                            *lower_depth,
                            canonical_drop_hits.borrow().len(),
                        );
                        // A card's trailing edge is a vertical insertion point (the same
                        // orientation used when inserting after a card in the grid). Group
                        // anchors retain the horizontal boundary marker spanning the header.
                        if let Some((anchor, _)) = upper_anchor.as_ref()
                            && hierarchy.node(anchor).is_some_and(|node| !node.is_group())
                        {
                            hits[0].indicator_rect = card_trailing_marker_rect(upper_bounds);
                        }
                        // If the lower group is first in its sibling sequence, append to its parent.
                        // Otherwise insert after the actual preceding sibling (card or group).
                        let anchor_is_parent = upper_anchor.as_ref().is_some_and(|(anchor, _)| {
                            upper_parent == ContainerId::Group(anchor.clone())
                        });
                        if let Some((anchor, _)) = upper_anchor.filter(|_| !anchor_is_parent) {
                            hits[0].target = Some(anchor);
                            hits[0].container = None;
                            hits[0].region = PointerRegion::TrailingEdge;
                        } else {
                            hits[0].target = None;
                            hits[0].container = Some(upper_parent);
                            hits[0].region = PointerRegion::ContainerEnd;
                        }
                        for hit in hits {
                            canonical_drop_hits.borrow_mut().push((hit, response.clone()));
                        }
                    }
                }
                // Any group may be followed by a card at the same sibling level, including
                // inside arbitrarily nested parent groups. Offer a split boundary after the
                // complete preceding group subtree: append into that group, or insert after
                // the group while remaining in its parent container.
                fn collect_group_item_sibling_pairs(nodes: &[HierarchyNode], out: &mut Vec<(NodeId, NodeId)>) {
                    for pair in nodes.windows(2) {
                        if matches!(pair, [HierarchyNode::Group { .. }, HierarchyNode::Item(_)]) {
                            out.push((pair[0].id().clone(), pair[1].id().clone()));
                        }
                    }
                    for node in nodes {
                        if node.is_group() {
                            collect_group_item_sibling_pairs(node.children(), out);
                        }
                    }
                }
                let mut group_item_pairs = Vec::new();
                collect_group_item_sibling_pairs(hierarchy.roots(), &mut group_item_pairs);
                for (group_id, item_id) in group_item_pairs {
                    let Some(group_header) = canonical_node_bounds.borrow().get(&group_id).copied() else {
                        continue;
                    };
                    let Some(item_bounds) = canonical_node_bounds.borrow().get(&item_id).copied() else {
                        continue;
                    };
                    let mut subtree_nodes = Vec::new();
                    fn collect_group_nodes(node: &HierarchyNode, out: &mut Vec<NodeId>) {
                        out.push(node.id().clone());
                        for child in node.children() {
                            collect_group_nodes(child, out);
                        }
                    }
                    let Some(group_node) = hierarchy.node(&group_id) else {
                        continue;
                    };
                    collect_group_nodes(group_node, &mut subtree_nodes);
                    let subtree_bounds: Vec<(NodeId, Rect)> = subtree_nodes
                        .iter()
                        .filter_map(|id| canonical_node_bounds.borrow().get(id).copied().map(|rect| (id.clone(), rect)))
                        .collect();
                    let Some(group_bottom) = subtree_bounds.iter().map(|(_, rect)| rect.bottom()).max_by(f32::total_cmp) else {
                        continue;
                    };
                    let boundary_y = (group_bottom + item_bounds.top()) * 0.5;
                    let gap = Rect::from_min_max(
                        pos2(ui.clip_rect().left(), boundary_y - GROUP_GAP_DROP_HEIGHT * 0.5),
                        pos2(ui.clip_rect().right(), boundary_y + GROUP_GAP_DROP_HEIGHT * 0.5),
                    );
                    let response = ui.interact(gap, ui.id().with(("group-root-card-gap", panel, &group_id, &item_id)), Sense::hover());
                    let marker_y = group_bottom;
                    let append_marker = Rect::from_min_max(
                        pos2(group_header.left(), marker_y - 2.0),
                        pos2(group_header.right(), marker_y + 2.0),
                    );
                    let order = canonical_drop_hits.borrow().len();
                    canonical_drop_hits.borrow_mut().push((
                        DropHitRegion {
                            target: None,
                            container: Some(ContainerId::Group(group_id.clone())),
                            region: PointerRegion::ContainerEnd,
                            hit_rect: Rect::from_min_max(gap.min, pos2(gap.right(), gap.center().y)),
                            indicator_rect: append_marker,
                            // This boundary takes precedence over overlapping neighboring card
                            // edge regions at every hierarchy depth.
                            depth: usize::MAX,
                            order,
                        },
                        response.clone(),
                    ));
                    canonical_drop_hits.borrow_mut().push((
                        DropHitRegion {
                            target: Some(group_id),
                            container: None,
                            region: PointerRegion::TrailingEdge,
                            hit_rect: Rect::from_min_max(pos2(gap.left(), gap.center().y), gap.max),
                            // This is an after-group sibling insertion, so show the group's
                            // horizontal lower border rather than the last card's vertical edge.
                            indicator_rect: append_marker,
                            // See the append half above: this boundary wins overlapping card hits.
                            depth: usize::MAX,
                            order: order.saturating_add(1),
                        },
                        response,
                    ));
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
                // The leading target is registered before rows are laid out. Resolve a card's
                // actual edge now that its final grid bounds are available.
                if let Some(first) = hierarchy.roots().first()
                    && !first.is_group()
                    && let Some(bounds) = canonical_node_bounds.borrow().get(&first.id()).copied()
                    && let Some((hit, _)) = canonical_drop_hits
                        .borrow_mut()
                        .iter_mut()
                        .find(|(hit, _)| hit.region == PointerRegion::LeadingEdge && hit.target.as_ref() == Some(&first.id()))
                {
                    hit.indicator_rect = card_leading_marker_rect(bounds);
                }
                // The whole trailing blank area is the root append target, not just a narrow
                // sixteen-pixel strip immediately after the final node. This lets users drop in
                // the whitespace below the last group while keeping the indicator at its top edge.
                let root_end_height = (ui.clip_rect().bottom() - ui.cursor().top()).max(32.0);
                let (root_end_rect, root_end_response) = ui.allocate_exact_size(vec2(ui.available_width(), root_end_height), Sense::hover());
                let indicator = match hierarchy.roots().last() {
                    Some(last) if !last.is_group() => canonical_node_bounds
                        .borrow()
                        .get(&last.id())
                        .copied()
                        .map(card_trailing_marker_rect)
                        .unwrap_or_else(|| Rect::from_min_max(pos2(ui.clip_rect().left(), root_end_rect.top() - 2.0), pos2(ui.clip_rect().right(), root_end_rect.top() + 2.0))),
                    _ => Rect::from_min_max(pos2(ui.clip_rect().left(), root_end_rect.top() - 2.0), pos2(ui.clip_rect().right(), root_end_rect.top() + 2.0)),
                };
                let hit_index = canonical_drop_hits.borrow().len();
                let visible_root_end = root_end_rect.intersect(ui.clip_rect());
                if visible_root_end.is_positive() {
                    canonical_drop_hits.borrow_mut().push((root_append_hit(visible_root_end, indicator, hit_index), root_end_response));
                }
            }
        });
    if empty_area_clicked.get() {
        st.browser_selection.entry(panel.to_owned()).or_default().clear();
        st.browser_selection_anchor.remove(panel);
    }
    for (ghost_rect, gi, open, depth) in group_drag_ghosts {
        let Some(group) = groups.get(gi) else { continue };
        let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new(("preset-group-drag", panel)));
        let mut ghost_ui = egui::Ui::new(
            ui.ctx().clone(),
            egui::Id::new(("preset-group-drag-ui", panel)),
            egui::UiBuilder::new().layer_id(layer).max_rect(ghost_rect.expand(1.0)),
        );
        ghost_ui.set_clip_rect(app_window_clip);
        if let Some(paint) = paint_group.as_deref_mut() {
            paint(&ghost_ui, ghost_rect, group, open, false, GROUP_DRAG_GHOST_OPACITY);
        } else {
            crate::pickers::asset_browser_groups::paint_with_opacity(&ghost_ui, ghost_rect, open, &group.name, depth, None, GROUP_DRAG_GHOST_OPACITY);
        }
    }
    for (ghost_rect, open, name, depth, count) in nested_group_drag_ghosts {
        let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new(("preset-group-drag", panel)));
        let mut ghost_ui = egui::Ui::new(
            ui.ctx().clone(),
            egui::Id::new(("preset-group-drag-ui", panel)),
            egui::UiBuilder::new().layer_id(layer).max_rect(ghost_rect.expand(1.0)),
        );
        ghost_ui.set_clip_rect(app_window_clip);
        crate::pickers::asset_browser_groups::paint_with_opacity(&ghost_ui, ghost_rect, open, &name, depth, Some(count), GROUP_DRAG_GHOST_OPACITY);
    }
    for (ghost_rect, gi, ii) in item_drag_ghosts {
        let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new(("preset-drag", panel)));
        let mut ghost_ui =
            egui::Ui::new(ui.ctx().clone(), egui::Id::new(("preset-drag-ui", panel)), egui::UiBuilder::new().layer_id(layer).max_rect(ghost_rect.expand(1.0)));
        ghost_ui.set_clip_rect(app_window_clip);
        ghost_ui.set_opacity(0.4);
        paint_item(&ghost_ui, ghost_rect, gi, ii, ItemPaintState::DragGhost);
        ghost_ui.painter().rect_stroke(ghost_rect, CornerRadius::ZERO, Stroke::new(2.0, t.accent), egui::StrokeKind::Middle);
    }
    let browser_view_clip =
        Rect::from_min_max(pos2(browser_parent_clip.left(), browser_parent_clip.top()), pos2(browser_parent_clip.right(), scroll.inner_rect.bottom()))
            .intersect(browser_parent_clip);
    let mut hierarchy_drop_feedback = None;
    if canonical_hierarchy_drag {
        if let Some(pointer) = ui.ctx().pointer_interact_pos() {
            // Geometric arbitration must respect the ScrollArea viewport. Otherwise a hit region
            // clipped out of view can still win when its un-clipped rectangle overlaps another
            // visible region near the viewport edge.
            let hits: Vec<DropHitRegion> = canonical_drop_hits
                .borrow()
                .iter()
                .filter_map(|(hit, _)| {
                    let mut visible = hit.clone();
                    // Header visuals deliberately extend into the ScrollArea's left margin. The
                    // inner rect excludes that margin, so use the parent's horizontal clip with
                    // the scroll viewport's vertical bounds instead of trimming the row's left.
                    visible.hit_rect = visible.hit_rect.intersect(browser_view_clip);
                    visible.hit_rect.is_positive().then_some(visible)
                })
                .collect();
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
                    let selection = match dragged {
                        AssetDrag::Preset { selection, .. } | AssetDrag::CanonicalGroup { selection, .. } => selection.clone(),
                        _ => Vec::new(),
                    };
                    let mut session = DragSession::new(source.clone(), selection);
                    if let Some(candidate) = session.resolve(pointer, &hits[index], &hierarchy).cloned() {
                        paint_candidate(ui, &candidate, t.accent, browser_view_clip);
                        // Resolve and validate the exact candidate before taking egui's global
                        // one-shot payload. The taken source must still identify this same node.
                        if let Some(released) = asset_browser_take_release(ui.ctx()) {
                            if source_node_for_drag(&released, panel, &hierarchy_identities).as_ref() == Some(&source) {
                                let result = session.commit_with(|intent| {
                                    if let Some(commit) = canonical_hierarchy_move.as_deref_mut() {
                                        return commit(intent);
                                    }
                                    let sources = if intent.selection.is_empty() { vec![intent.source.clone()] } else { intent.selection.clone() };
                                    let hierarchy_sources = hierarchy_move_sources(&sources, &intent.position);
                                    let legacy_persistable = hierarchy_identities
                                        .values()
                                        .all(|identity| matches!(identity, HierarchyIdentity::Item { .. } | HierarchyIdentity::Group(_)));
                                    if !legacy_persistable {
                                        return Err("This asset adapter does not support path-aware hierarchy moves.".to_owned());
                                    }
                                    let item_moves = plan_hierarchy_item_moves(&hierarchy, &sources, &intent.position, &hierarchy_identities, root_item_group)?;
                                    for (group, key, destination_group, index) in item_moves {
                                        let Some(commit) = hierarchy_item_move.as_deref_mut() else {
                                            return Err("This asset adapter does not support authoritative item moves.".to_owned());
                                        };
                                        commit(&group, &key, &destination_group, index)?;
                                    }
                                    for source in &hierarchy_sources {
                                        commit_hierarchy_move(st, panel, &mut hierarchy, source, &intent.position, &hierarchy_identities)
                                            .map_err(|error| format!("Could not persist hierarchy move: {error:?}"))?;
                                    }
                                    Ok(())
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
        BrowserAction, BrowserAdapter, BrowserCapabilities, BrowserOptions, GroupView, HIT_DEBUG_LABEL_HEIGHT, HierarchyOrderEntry, ItemPaintState, ItemView,
        best_drop_target_index, best_indent_drop_target_index, browser, filter_groups, group_render_segments, hit_debug_label_position,
        item_hierarchy_insertion_anchor, item_occurrence_position,
    };
    use crate::pickers::picker_state::PresetUi;
    use egui_kittest::kittest::Queryable;

    fn group(name: &str, items: Vec<ItemView>) -> GroupView {
        GroupView { name: name.into(), root_only: false, items }
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
    fn adjacent_root_cards_share_an_unindented_grid_segment() {
        let groups = vec![GroupView { name: "Presets".into(), root_only: true, items: vec![item("first", "First", &[]), item("second", "Second", &[])] }];
        let orders = std::collections::BTreeMap::from([(
            "".into(),
            vec![
                HierarchyOrderEntry::RootItem { group: "Presets".into(), key: "first".into() },
                HierarchyOrderEntry::RootItem { group: "Presets".into(), key: "second".into() },
            ],
        )]);

        let segments = group_render_segments(&groups, &[], &Default::default(), &orders);

        assert_eq!(segments.len(), 1);
        assert!(segments[0].root_items);
        assert_eq!(segments[0].depth, 0);
        assert_eq!(segments[0].item_indices, [0, 1]);
    }

    #[test]
    fn root_card_runs_split_at_groups_and_retain_each_source_group() {
        let groups = vec![
            group("First", vec![item("first-a", "First A", &[]), item("first-b", "First B", &[])]),
            group("Middle", vec![item("middle", "Middle", &[])]),
            group("Last", vec![item("last", "Last", &[])]),
        ];
        let orders = std::collections::BTreeMap::from([(
            "".into(),
            vec![
                HierarchyOrderEntry::Group("First".into()),
                HierarchyOrderEntry::RootItem { group: "First".into(), key: "first-a".into() },
                HierarchyOrderEntry::Group("Middle".into()),
                HierarchyOrderEntry::RootItem { group: "Last".into(), key: "last".into() },
                HierarchyOrderEntry::Group("Last".into()),
                HierarchyOrderEntry::RootItem { group: "First".into(), key: "first-b".into() },
            ],
        )]);

        let segments = group_render_segments(&groups, &[(0, 0), (1, 0), (2, 0)], &Default::default(), &orders);

        assert_eq!(
            segment_summary(&segments, &groups),
            [
                ("First".into(), true, vec![]),
                ("First".into(), false, vec!["first-a".into()]),
                ("Middle".into(), true, vec!["middle".into()]),
                ("Last".into(), false, vec!["last".into()]),
                ("Last".into(), true, vec![]),
                ("First".into(), false, vec!["first-b".into()]),
            ]
        );
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
            egui::Rect::from_min_max(egui::pos2(20.0, 60.0), egui::pos2(120.0, 90.0)),
            egui::Rect::from_min_max(egui::pos2(20.0, 106.0), egui::pos2(210.0, 135.0)),
            0,
            1,
            0,
        );
        let upper_label = hit_debug_label_position(&hits[0]);
        let lower_label = hit_debug_label_position(&hits[1]);

        assert!(upper_label.y >= gap.bottom(), "upper label starts above the visible gap: y={}", upper_label.y);
        assert!(upper_label.y + HIT_DEBUG_LABEL_HEIGHT <= lower_label.y, "upper label at y={} overlaps lower label at y={}", upper_label.y, lower_label.y,);
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
            (
                "".into(),
                vec![
                    HierarchyOrderEntry::RootItem { group: "First".into(), key: "same".into() },
                    HierarchyOrderEntry::RootItem { group: "Second".into(), key: "same".into() },
                ],
            ),
            ("First".into(), vec![HierarchyOrderEntry::Item("same".into())]),
            ("Second".into(), vec![HierarchyOrderEntry::Item("same".into())]),
        ]
        .into();
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
                        root_item_group: None,
                    },
                    capabilities: BrowserCapabilities {
                        item_drag: false,
                        item_drop: false,
                        group_drag: true,
                        selection: false,
                        activation: false,
                        canonical_hierarchy_drag: false,
                    },
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
        assert!(
            !h.state().0.hierarchy_order.get("hierarchy-drop-test").is_some_and(|orders| orders.contains_key("")),
            "an unsupported nested target must not mutate root group order"
        );
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
