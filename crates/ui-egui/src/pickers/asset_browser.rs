//! Asset-neutral picker browser model, rendering, and interaction routing.
//!
//! Consumers project their domain assets into `GroupView` and `ItemView`, provide preview painters,
//! and translate `BrowserAction` into domain commands. This module owns the common browser presentation only.

use std::collections::BTreeSet;

use egui::{Align2, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, pos2, vec2};

use crate::pickers::asset_browser_dragging::{
    BrowserDragState, GroupRelease, GroupReleaseBatch, ItemDropCommit, ItemDropSource, RootHeaderReleaseEvent, apply_browser_item_gesture,
    apply_group_releases_and_toggles, apply_legacy_group_release, apply_root_header_release, canvas_drop_target, commit_item_drop,
    paint_group_drop_insertion_markers, register_browser_item_drop_targets, register_canonical_root_end_target, register_canonical_root_leading_target,
    register_canonical_sibling_gap_targets, register_group_indent_drop_targets, resolve_canonical_drop, resolve_group_indent_target,
    resolve_item_hierarchy_drop_target,
};
use crate::pickers::asset_browser_groups::{GROUP_HEADER_HEIGHT, group_open, nested_header as draw_nested_folder_header, row_indent};
use crate::pickers::asset_browser_hierarchy::HierarchyGroup;
use crate::pickers::asset_browser_interaction::{
    AssetDrag, BrowserDeleteContext, BrowserDragGhostSource, GroupDragGhost, HeaderDropCapabilities, HierarchyDrop, ItemDragGhost, NestedGroupDragGhost,
    asset_browser_header_drag_source_with_id, asset_browser_header_target_with_capabilities, browser_drag_ghost_rect, draw_context_menu_target,
    draw_rename_row, group_header_drop_hits, header_drop_before, item_drop_after, replace_selected_delete_actions,
};
pub(crate) use crate::pickers::asset_browser_interaction::{
    begin_item_rename, delete_item_actions, item_context_targets, move_item_actions, standard_group_context_menu, standard_item_context_menu,
};
use crate::pickers::asset_browser_items::{
    group_matches_query, group_visible_for_query, item_grid_position, item_info as asset_browser_item_info, item_matches_query, item_visible_for_query,
    list_layout_geometry, list_row as asset_browser_list_row,
};
use crate::pickers::picker_browser_dragging::DropMoveIntent;
use crate::pickers::picker_browser_identity::{PickerIdentityKey, PickerNodeIdentity, PickerProjectionKeys, ProjectionNodeRef};
use crate::pickers::picker_browser_model::{ContainerId, PickerBrowserHierarchy, PickerId, PickerIdRegistry};
use crate::pickers::picker_browser_projection::{GroupRenderSegment, build_hierarchy_projection};
pub(crate) use crate::pickers::picker_browser_projection::{GroupView, ItemView};
use crate::pickers::picker_state::escape_hierarchy_key_part;
use crate::pickers::picker_state::{HierarchyOrderEntry, PresetUi};
use crate::theme::Tokens;

pub(crate) const MIN_THUMB: f32 = 20.0;

#[cfg(test)]
use crate::pickers::asset_browser_dragging::{
    best_drop_target_index, best_indent_drop_target_index, item_hierarchy_insertion_anchor, item_occurrence_position, plan_hierarchy_item_moves,
};
#[cfg(test)]
use crate::pickers::asset_browser_interaction::{advance_item_rename, browser_drag_ghost_position, clamp_drag_ghost, selected_delete_actions};
#[cfg(test)]
use crate::pickers::picker_browser_dragging::{DropHitRegion, PointerRegion};
// 27px of header content plus a one-pixel border on each vertical edge.
const GROUP_LAYOUT_GAP: f32 = 0.0;
// The six-pixel band immediately before a group is split into two exact three-pixel targets.

/// Geometry shared by every item in one rendered grid/list segment.
#[derive(Clone, Copy, Debug)]
struct ItemGridLayout {
    content_indent: f32,
    display_mode: crate::pickers::picker_state::PickerDisplayMode,
    wrap_list: bool,
    center_grid: bool,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ItemGrid {
    pub(super) area: Rect,
    cell: egui::Vec2,
    pub(super) columns: usize,
    rows: usize,
    pad: f32,
    center_offset: f32,
    gap: f32,
    origin_x: f32,
    column_major: bool,
    pub(super) item_count: usize,
    pub(super) list: bool,
}

impl ItemGrid {
    fn cell_rect(self, index: usize) -> Rect {
        let (x, y) = item_grid_position(index, self.rows, self.columns, self.column_major);
        Rect::from_min_size(
            pos2(self.origin_x + self.pad + self.center_offset + x * (self.cell.x + self.gap), self.area.min.y + y * (self.cell.y + self.gap)),
            self.cell,
        )
    }

    fn next_cell_rect(self, index: usize) -> Option<Rect> {
        (index + 1 < self.item_count).then(|| self.cell_rect(index + 1))
    }
}

/// One visible occurrence in a projected item run. Indices identify the backing source occurrence,
/// not its position in the filtered visual run.
#[derive(Clone, Copy)]
pub(super) struct ItemRef<'a> {
    pub(super) group_index: usize,
    pub(super) item_index: usize,
    pub(super) group: &'a GroupView,
    pub(super) item: &'a ItemView,
}

/// Result of painting an item occurrence; keeps source identity, geometry and interaction together.
pub(super) struct PaintedCell<'a> {
    pub(super) item: ItemRef<'a>,
    pub(super) rect: Rect,
    pub(super) next_rect: Option<Rect>,
    pub(super) response: egui::Response,
    pub(super) node_id: Option<PickerId>,
    pub(super) depth: usize,
    pub(super) selected: bool,
}

/// One occurrence's position and hierarchy depth within a rendered item segment.
struct ItemSegmentCell<'a> {
    item: ItemRef<'a>,
    run_index: usize,
    depth: usize,
    root_level: bool,
}

/// Generic browser interactions; each asset consumer translates these into its own commands.
#[derive(Clone)]
pub(crate) enum BrowserAction {
    /// Select one item occurrence, qualified by its owning root group.
    SelectItem {
        group: String,
        group_path: Vec<String>,
        key: String,
    },
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

/// Semantic boundary between segment rendering and the picker frame's mutable browser state.
/// Actions are owned by the sink and leave the frame as one collected result; segment closures
/// never receive the caller's action vector. The context-menu escape hatch is deliberately narrow:
/// existing adapter callbacks synchronously require both `PresetUi` and an action vector.
pub(super) struct BrowserSink<'a> {
    pub(super) state: &'a mut PresetUi,
    pub(super) actions: Vec<BrowserAction>,
    pending_group_toggles: Vec<String>,
}

pub(super) struct CanonicalDropViewport {
    pub(super) parent_clip: Rect,
    pub(super) view_clip: Rect,
}

pub(super) struct CanonicalDropCallbacks<'a, 'cb> {
    pub(super) hierarchy_move: &'a mut Option<&'cb mut CanonicalHierarchyMove<'cb>>,
    pub(super) rekey_after_move: &'a mut Option<&'cb mut RekeyHierarchyAfterMove<'cb>>,
    pub(super) item_move: &'a mut Option<&'cb mut HierarchyItemMove<'cb>>,
}

pub(super) struct HierarchyCommit<'a, 'cb> {
    pub(super) state: &'a mut PresetUi,
    pub(super) picker_ids: &'a mut PickerIdRegistry,
    pub(super) root_item_group: Option<&'a str>,
    pub(super) callbacks: CanonicalDropCallbacks<'a, 'cb>,
    pub(super) viewport: CanonicalDropViewport,
}

impl<'a> BrowserSink<'a> {
    fn new(state: &'a mut PresetUi, actions: Vec<BrowserAction>) -> Self {
        Self { state, actions, pending_group_toggles: Vec::new() }
    }

    fn is_node_selected(&self, panel: &str, id: &PickerId) -> bool {
        self.state.browser_node_selected(panel, id)
    }

    fn is_group_toggled(&self, key: &str) -> bool {
        self.state.toggled.contains(key)
    }

    fn is_group_open(&self, panel: &str, group_index: usize, group_name: &str, all_groups_open: bool) -> bool {
        group_open(self.state, panel, group_index, group_name, all_groups_open)
    }

    pub(super) fn select_node(&mut self, panel: &str, id: PickerId, toggle: bool) {
        self.state.select_browser_node(panel, id, toggle);
    }

    pub(super) fn select_range(&mut self, panel: &str, order: &[PickerId], id: &PickerId) {
        self.state.select_browser_range(panel, order, id);
    }

    fn toggle_group(&mut self, key: String) {
        if !self.state.toggled.remove(&key) {
            self.state.toggled.insert(key);
        }
    }

    fn queue_group_toggle(&mut self, key: String) {
        self.pending_group_toggles.push(key);
    }

    pub(super) fn emit(&mut self, action: BrowserAction) {
        self.actions.push(action);
    }

    /// Apply the group releases and folder toggles collected while rendering a segment.
    /// Adapter context menus still have a legacy callback contract that mutates picker state and
    /// appends actions together. Keep that coupling confined to the synchronous menu invocation.
    fn with_context_menu<R>(&mut self, draw: impl FnOnce(&mut PresetUi, &mut Vec<BrowserAction>) -> R) -> R {
        draw(self.state, &mut self.actions)
    }

    fn into_actions(self) -> Vec<BrowserAction> {
        self.actions
    }
}

pub(super) fn hierarchy_action(drop: HierarchyDrop) -> BrowserAction {
    match drop {
        HierarchyDrop::MoveGroup { group, group_path, before } => BrowserAction::MoveGroup { group, group_path, before },
        HierarchyDrop::MoveInto { group, group_path, into, before } => BrowserAction::MoveInto { group, group_path, into, before, before_item: None },
    }
}

/// Keep the domain-level selected preset in sync with item moves, regardless of which drop
/// target produced the move action. Browser multi-selection is separate transient UI state.
pub(super) fn select_dropped_item(st: &mut PresetUi, panel: &str, group: &str, group_path: &[String], key: &str) {
    st.set_selected_item(panel, group, group_path, key);
}

/// Resolve the moved item's destination from the drop position, not its stable source identity.
/// Item identities deliberately retain their original group/path across hierarchy moves.
pub(super) fn selected_drag_nodes(
    state: &PresetUi,
    panel: &str,
    visible_order: &[PickerId],
    source: &PickerId,
    hierarchy: &PickerBrowserHierarchy,
) -> Vec<PickerId> {
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
pub(super) fn item_node_for_identity(
    group: &str,
    group_path: &[String],
    key: &str,
    identities: &std::collections::BTreeMap<PickerId, PickerNodeIdentity>,
) -> Option<PickerId> {
    identities.iter().find_map(|(id, identity)| {
        let matches = matches!(identity,
            PickerNodeIdentity::Item { group: item_group, key: item_key } if group_path.is_empty() && item_group == group && item_key == key
        ) || matches!(identity,
            PickerNodeIdentity::FolderItem { group: item_group, path, key: item_key } if !group_path.is_empty() && item_group == group && path == group_path && item_key == key
        );
        matches.then(|| id.clone())
    })
}

pub(super) fn picker_group_id(registry: &mut PickerIdRegistry, group_index: usize, keys: &PickerProjectionKeys) -> PickerId {
    let fallback = group_index.to_string();
    let key = keys
        .key(&ProjectionNodeRef::Group(group_index))
        .cloned()
        .unwrap_or_else(|| PickerIdentityKey::from_parts("asset-browser-group-fallback", [fallback.as_str()]));
    registry.id_for(key)
}

fn picker_folder_id(registry: &mut PickerIdRegistry, group_index: usize, keys: &PickerProjectionKeys, path: &[String]) -> PickerId {
    let node = ProjectionNodeRef::Folder { group_index, path: path.to_vec() };
    let fallback_parts = std::iter::once(group_index.to_string()).chain(path.iter().cloned()).collect::<Vec<_>>();
    let fallback = fallback_parts.iter().map(String::as_str).collect::<Vec<_>>();
    let key = keys.key(&node).cloned().unwrap_or_else(|| PickerIdentityKey::from_parts("asset-browser-folder-fallback", fallback));
    registry.id_for(key)
}

fn folder_node_for_identity(group: &str, path: &[String], identities: &std::collections::BTreeMap<PickerId, PickerNodeIdentity>) -> Option<PickerId> {
    identities.iter().find_map(|(id, identity)| {
        matches!(identity, PickerNodeIdentity::Folder { group: owner, path: candidate } if owner == group && candidate == path).then(|| id.clone())
    })
}

pub(super) fn group_is_descendant(parents: &std::collections::BTreeMap<String, String>, source: &str, target: &str) -> bool {
    is_group_descendant(parents, source, target)
}

pub(super) fn hierarchy_drop_accepts(
    drag: &AssetDrag,
    panel: &str,
    item_group: &str,
    item_path: &[String],
    parents: &std::collections::BTreeMap<String, String>,
    domain_hierarchy_actions: bool,
) -> bool {
    let AssetDrag::Group { scope, name, path } = drag else { return false };
    if scope != panel {
        return false;
    }
    if path.is_empty() {
        !domain_hierarchy_actions || (!group_is_descendant(parents, name, item_group) && name != item_group)
    } else {
        domain_hierarchy_actions && !(name == item_group && item_path.starts_with(path))
    }
}

/// All derived model state needed by one browser frame. Keeping projection, filtering and visible
/// selection order together prevents the rendering pass from rebuilding subtly different views of
/// the same hierarchy.
pub(super) struct BrowserProjection {
    pub(super) display: Vec<(usize, usize)>,
    pub(super) group_names: Vec<String>,
    pub(super) parents: std::collections::BTreeMap<String, String>,
    pub(super) hierarchy_orders: std::collections::BTreeMap<String, Vec<HierarchyOrderEntry>>,
    render_segments: Vec<GroupRenderSegment>,
    pub(super) hierarchy: PickerBrowserHierarchy,
    pub(super) hierarchy_identities: std::collections::BTreeMap<PickerId, PickerNodeIdentity>,
    selected_items: Vec<(String, ItemView)>,
    pub(super) visible_selection_order: Vec<PickerId>,
}

/// Borrowed hierarchy data shared by hierarchy interaction helpers for one browser frame.
pub(super) struct BrowserHierarchyView<'a> {
    pub(super) groups: &'a [GroupView],
    pub(super) group_names: &'a [String],
    pub(super) parents: &'a std::collections::BTreeMap<String, String>,
    pub(super) hierarchy: &'a PickerBrowserHierarchy,
    pub(super) hierarchy_identities: &'a std::collections::BTreeMap<PickerId, PickerNodeIdentity>,
    pub(super) hierarchy_orders: &'a std::collections::BTreeMap<String, Vec<HierarchyOrderEntry>>,
    pub(super) visible_selection_order: &'a [PickerId],
}

pub(super) struct ItemHierarchyDropPolicy<'a> {
    pub(super) panel: &'a str,
    pub(super) capabilities: BrowserCapabilities,
    pub(super) domain_hierarchy_actions: bool,
}

pub(super) struct ItemHierarchyDropResolution<'a> {
    pub(super) indent_release: &'a mut Option<(String, String)>,
    pub(super) browser_item_drop: &'a mut bool,
    pub(super) released: &'a mut Option<(usize, Pos2)>,
}

/// Immutable adapter configuration shared by the browser's render and interaction helpers.
/// Mutable frame state, projection data, and callbacks deliberately remain separate.
#[derive(Clone, Copy)]
pub(super) struct BrowserCtx<'a> {
    pub(super) panel: &'a str,
    pub(super) capabilities: BrowserCapabilities,
    tokens: &'a Tokens,
    pub(super) groups: &'a [GroupView],
    pub(super) projection_keys: &'a PickerProjectionKeys,
    pub(super) domain_hierarchy_actions: bool,
    context_menus: ContextMenus,
}

impl BrowserProjection {
    fn build(
        ctx: &BrowserCtx<'_>,
        state: &PresetUi,
        query: &str,
        all_groups_open: bool,
        domain_root_sequence: Option<&[HierarchyOrderEntry]>,
        picker_ids: &mut PickerIdRegistry,
    ) -> Option<Self> {
        let BrowserCtx { groups, panel, projection_keys, domain_hierarchy_actions, .. } = *ctx;
        let visible_group_indices: Vec<usize> = groups.iter().enumerate().filter_map(|(index, group)| (!group.root_only).then_some(index)).collect();
        let group_names: Vec<String> = visible_group_indices.iter().filter_map(|index| groups.get(*index).map(|group| group.name.clone())).collect();
        let display: Vec<(usize, usize)> = if domain_hierarchy_actions {
            visible_group_indices.iter().map(|index| (*index, 0)).collect()
        } else {
            state
                .nested_display_order(panel, &group_names)
                .into_iter()
                .filter_map(|(visible_index, depth)| visible_group_indices.get(visible_index).map(|index| (*index, depth)))
                .collect()
        };
        let parents = if domain_hierarchy_actions { std::collections::BTreeMap::new() } else { state.group_parents(panel) };
        let mut hierarchy_orders = std::collections::BTreeMap::new();
        let root_entries = if domain_hierarchy_actions {
            domain_root_sequence.map(<[HierarchyOrderEntry]>::to_vec).unwrap_or_default()
        } else {
            state.hierarchy_order.get(panel).and_then(|orders| orders.get("")).cloned().unwrap_or_default()
        };
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
                state.ordered_hierarchy_entries(panel, &group.name, &item_keys, &child_groups)
            };
            hierarchy_orders.insert(group.name.clone(), entries);
        }
        hierarchy_orders.insert(String::new(), root_entries);
        let (render_segments, hierarchy, hierarchy_identities) =
            build_hierarchy_projection(groups, &display, &parents, &hierarchy_orders, projection_keys, picker_ids)?;

        let mut selected_items = Vec::new();
        if let Some(selection) = state.browser_selection.get(panel) {
            for group in groups {
                for item in &group.items {
                    if item_node_for_identity(&group.name, &item.group_path, &item.key, &hierarchy_identities).is_some_and(|id| selection.contains(&id)) {
                        selected_items.push((group.name.clone(), item.clone()));
                    }
                }
            }
        }

        let mut visible_selection_order = Vec::new();
        let mut hidden_groups = BTreeSet::new();
        let mut hidden_folders = BTreeSet::<(String, Vec<String>)>::new();
        for segment in &render_segments {
            let Some(group) = groups.get(segment.group_index) else { continue };
            let group_matches = group_matches_query(group, query);
            let has_matching_items = group.items.iter().any(|item| item_matches_query(item, query));
            if segment.root_items {
                for (group_index, item_index) in &segment.item_indices {
                    let Some(source_group) = groups.get(*group_index) else { continue };
                    let Some(item) = source_group.items.get(*item_index) else { continue };
                    if item_visible_for_query(item, query, group_matches_query(source_group, query))
                        && let Some(id) = item_node_for_identity(&source_group.name, &item.group_path, &item.key, &hierarchy_identities)
                    {
                        visible_selection_order.push(id);
                    }
                }
                continue;
            }
            if !group_visible_for_query(query, group_matches, has_matching_items) {
                continue;
            }
            if segment.show_header {
                if parents.get(&group.name).is_some_and(|parent| hidden_groups.contains(parent)) {
                    hidden_groups.insert(group.name.clone());
                    continue;
                }
                let open = !query.is_empty() || group_open(state, panel, segment.group_index, &group.name, all_groups_open);
                visible_selection_order.push(picker_group_id(picker_ids, segment.group_index, projection_keys));
                if !open {
                    hidden_groups.insert(group.name.clone());
                }
            } else if hidden_groups.contains(&group.name) {
                continue;
            }
            if !segment.folder_path.is_empty()
                && (1..=segment.folder_path.len()).any(|length| hidden_folders.contains(&(group.name.clone(), segment.folder_path[..length].to_vec())))
            {
                continue;
            }
            let folder_open = segment.folder_path.is_empty() || {
                let path = segment.folder_path.iter().map(|part| escape_hierarchy_key_part(part)).collect::<Vec<_>>().join("/");
                !state.toggled.contains(&format!("{panel}/{}/{}", escape_hierarchy_key_part(&group.name), path))
            };
            if segment.show_folder_header {
                visible_selection_order.push(picker_folder_id(picker_ids, segment.group_index, projection_keys, &segment.folder_path));
                if !folder_open {
                    hidden_folders.insert((group.name.clone(), segment.folder_path.clone()));
                }
            }
            if folder_open {
                for (group_index, item_index) in &segment.item_indices {
                    if let Some(source_group) = groups.get(*group_index)
                        && let Some(item) =
                            source_group.items.get(*item_index).filter(|item| item_visible_for_query(item, query, group_matches_query(source_group, query)))
                        && let Some(id) = item_node_for_identity(&source_group.name, &item.group_path, &item.key, &hierarchy_identities)
                    {
                        visible_selection_order.push(id);
                    }
                }
            }
        }

        Some(Self {
            display,
            group_names,
            parents,
            hierarchy_orders,
            render_segments,
            hierarchy,
            hierarchy_identities,
            selected_items,
            visible_selection_order,
        })
    }
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

use crate::pickers::asset_browser_groups::{header as asset_browser_header, inset_row_rect};

/// Browser-specific layout and presentation, supplied by each asset consumer.
/// Cards use a consistent border and two-pixel inter-card spacing across picker consumers.
#[derive(Clone, Copy)]
pub(crate) struct BrowserOptions {
    /// Canvas hit region used to distinguish drops onto the canvas from drops into the browser.
    pub(crate) canvas: Rect,
    /// Retained for compatibility with the legacy options shape; card borders define spacing.
    #[allow(dead_code)]
    pub(crate) flush: bool,
    /// Centre a partially filled grid row in the available item area.
    pub(crate) center_grid: bool,
    /// Start every group expanded rather than only the first group.
    pub(crate) all_groups_open: bool,
    /// Use nested menu submenus for context actions inside a parent popup.
    pub(crate) compact_context_menus: bool,
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

/// Optional consumer-provided group-row painter: UI, row rectangle, group, open, hovered, opacity.
pub(crate) type PaintGroup = dyn FnMut(&egui::Ui, Rect, &GroupView, bool, bool, f32);
/// Optional full-width list-row painter; shared browser code retains the row's hit target and border.
pub(crate) type PaintListItem<'a> = dyn FnMut(&egui::Ui, Rect, usize, usize, bool) + 'a;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ItemPaintState {
    Item { selected: bool },
    DragGhost,
}
/// Consumer preview painter for a grid card, list thumbnail, or drag ghost.
pub(crate) type PaintItem<'a> = dyn FnMut(&egui::Ui, Rect, usize, usize, ItemPaintState) + 'a;

/// Owns the adapter's three preview callbacks and keeps their dispatch policy together.
pub(super) struct Painters<'borrow, 'callback> {
    group: Option<&'borrow mut PaintGroup>,
    item: &'borrow mut PaintItem<'callback>,
    list_item: Option<&'borrow mut PaintListItem<'callback>>,
}

impl Painters<'_, '_> {
    pub(super) fn has_group(&self) -> bool {
        self.group.is_some()
    }

    pub(super) fn has_list_item(&self) -> bool {
        self.list_item.is_some()
    }

    pub(super) fn group(&mut self, ui: &egui::Ui, rect: Rect, group: &GroupView, open: bool, hovered: bool, opacity: f32) {
        if let Some(paint) = self.group.as_deref_mut() {
            paint(ui, rect, group, open, hovered, opacity);
        }
    }

    pub(super) fn item(&mut self, ui: &egui::Ui, rect: Rect, group_index: usize, item_index: usize, state: ItemPaintState) {
        (self.item)(ui, rect, group_index, item_index, state);
    }

    pub(super) fn list_item(&mut self, ui: &egui::Ui, rect: Rect, group_index: usize, item_index: usize, selected: bool) {
        if let Some(paint) = self.list_item.as_deref_mut() {
            paint(ui, rect, group_index, item_index, selected);
        }
    }
}

pub(crate) struct ItemContextMenuArgs<'a> {
    pub(crate) panel: &'a str,
    pub(crate) item: &'a ItemView,
    pub(crate) group: &'a GroupView,
    pub(crate) group_names: &'a [String],
    pub(crate) selected_items: &'a [(String, ItemView)],
}

/// Consumer adapter for the generic browser. Each asset library projects its own identity and
/// labels into `GroupView`/`ItemView`, paints its own preview, and handles returned `BrowserAction` values with
/// its own commands. Top-level picker menus are deliberately not adapter fields: the containing
/// `picker_shell` owns their shared structure and exposes one ordered domain-extension point. The
/// browser never knows about gradient, pattern, swatch, style, or brush data.
pub(super) type CanonicalHierarchyMove<'a> = dyn FnMut(&DropMoveIntent, &std::collections::BTreeMap<PickerId, PickerNodeIdentity>) -> Result<(), String> + 'a;
pub(super) type RekeyHierarchyAfterMove<'a> =
    dyn FnMut(&PickerBrowserHierarchy, &std::collections::BTreeMap<PickerId, PickerNodeIdentity>, &mut PickerIdRegistry) + 'a;
pub(super) type HierarchyItemMove<'a> = dyn FnMut(&str, &str, &str, usize) -> Result<(), String> + 'a;
pub(super) struct ItemDropRegistrationGeometry<'borrow, 'items> {
    pub(super) painted: &'borrow PaintedCell<'items>,
    pub(super) grid: ItemGrid,
    pub(super) run_index: usize,
}

pub(crate) struct BrowserAdapter<'a> {
    /// Stable key for this consumer's selection, sizing, and group-collapse state.
    pub(crate) panel: &'a str,
    /// Consumer-projected groups and ordered items to display.
    pub(crate) groups: &'a [GroupView],
    /// Opaque stable node tokens supplied by this adapter; shared projection only allocates IDs.
    pub(crate) projection_keys: Option<&'a PickerProjectionKeys>,
    /// Grid geometry and consumer-specific layout options.
    pub(crate) options: BrowserOptions,
    /// Toggleable interactions supported by this consumer.
    pub(crate) capabilities: BrowserCapabilities,
    /// Consumers may replace the standard item menu or omit it entirely.
    pub(crate) item_context_menu: Option<ItemContextMenu>,
    /// Consumer-specific group menu policy, attached to the shared group hit target.
    pub(crate) group_context_menu: Option<GroupContextMenu>,
    /// Optional context-menu policy for nested group headers.
    pub(crate) nested_group_context_menu: Option<NestedGroupContextMenu>,
    /// Authoritative adapter commit for a validated canonical move. The callback receives the
    /// exact source identity and destination position, and owns all domain persistence (including
    /// path-aware folder/item moves). Returning an error rejects the move and is shown to the user.
    /// When absent, the shared browser uses its legacy hierarchy persistence for Group/Item trees.
    pub(crate) canonical_hierarchy_move: Option<&'a mut CanonicalHierarchyMove<'a>>,
    /// Adapter-owned reconciliation for identity tokens whose semantics change on a move.
    pub(crate) rekey_hierarchy_after_move: Option<&'a mut RekeyHierarchyAfterMove<'a>>,
    /// Domain commit for simple flat item moves.
    pub(crate) hierarchy_item_move: Option<&'a mut HierarchyItemMove<'a>>,
    /// Optional override for group-row visuals; absent uses the shared disclosure header.
    pub(crate) paint_group: Option<&'a mut PaintGroup>,
    /// Consumer-specific preview painter; receives selection/drag-ghost state.
    pub(crate) paint_item: &'a mut PaintItem<'a>,
    /// Optional full-row list painter; absent uses the shared row and item thumbnail.
    pub(crate) paint_list_item: Option<&'a mut PaintListItem<'a>>,
}

/// Consumer callback that appends generic events for actions offered on an item.
pub(crate) type ItemContextMenu = for<'a> fn(&mut egui::Ui, &mut PresetUi, ItemContextMenuArgs<'a>, &mut Vec<BrowserAction>);
/// Consumer callback that appends generic events for actions offered on a group header.
pub(crate) type GroupContextMenu = fn(&mut egui::Ui, &mut PresetUi, &str, &GroupView, &mut Vec<BrowserAction>);
/// Consumer callback for rename/delete operations on a nested group, with its full path.
pub(crate) type NestedGroupContextMenu = fn(&mut egui::Ui, &mut PresetUi, &str, &GroupView, &[String], &mut Vec<BrowserAction>);

/// Consumer-provided callbacks for the browser's three context-menu targets.
#[derive(Clone, Copy)]
struct ContextMenus {
    item: Option<ItemContextMenu>,
    group: Option<GroupContextMenu>,
    nested_group: Option<NestedGroupContextMenu>,
    compact: bool,
}

/// Space between preset cards, and the grid's inset from the panel's left edge.
const GRID_GAP: f32 = 0.0;

/// Draw only the reusable library region. It owns groups/items, selection, drag/drop and item/group
/// context targets, but not the containing panel's optional controls or footer.
///
/// `allocated_height` is reserved by the caller or picker shell; the browser owns only the
/// scrolling library region, not search, recents, panel menus, footer controls, or command dispatch.
pub(crate) fn browser(ui: &mut egui::Ui, st: &mut PresetUi, adapter: &mut BrowserAdapter<'_>, allocated_height: f32) -> Vec<BrowserAction> {
    browser_with_root_sequence(ui, st, adapter, allocated_height, None)
}

/// Allocate and paint one visible group header, using the adapter override when present.
fn draw_browser_group_header(ui: &mut egui::Ui, group: &GroupView, open: bool, depth: usize, painters: &mut Painters<'_, '_>) -> (Rect, egui::Response) {
    if painters.has_group() {
        let (allocated_rect, allocated) = ui.allocate_exact_size(vec2(ui.available_width(), GROUP_HEADER_HEIGHT), Sense::hover());
        let rect = Rect::from_min_max(pos2(ui.clip_rect().left(), allocated_rect.top()), pos2(allocated_rect.right(), allocated_rect.bottom()));
        let response = ui.interact(inset_row_rect(rect, depth), allocated.id, Sense::click_and_drag());
        painters.group(ui, rect, group, open, response.hovered() || response.dragged(), 1.0);
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::CollapsingHeader, true, &group.name));
        (rect, response)
    } else {
        asset_browser_header(ui, &group.name, open, depth, None, GROUP_HEADER_HEIGHT)
    }
}

struct NestedFolderRow<'a, 'items> {
    group: &'a HierarchyGroup<'items, ItemView>,
    root_group: &'a GroupView,
    open: bool,
    depth: usize,
    parent_depth: usize,
}

fn draw_nested_folder_row(
    ui: &mut egui::Ui,
    row: NestedFolderRow<'_, '_>,
    ctx: &BrowserCtx<'_>,
    hierarchy: &BrowserHierarchyView<'_>,
    sink: &mut BrowserSink<'_>,
    drag: &mut BrowserDragState,
) {
    let NestedFolderRow { group, root_group, open: group_open, depth: group_depth, parent_depth } = row;
    let BrowserCtx { panel, capabilities, domain_hierarchy_actions, .. } = *ctx;
    let tokens = ctx.tokens;
    let browser_bounds = ui.clip_rect();
    if !drag.canonical_drop.gap_headers.is_empty() {
        ui.add_space((GROUP_LAYOUT_GAP - ui.spacing().item_spacing.y).max(0.0));
    }
    let (rect, response, disclosure, row_depth) = draw_nested_folder_header(ui, group, group_open, group_depth, parent_depth);
    if response.hovered()
        && response.dnd_hover_payload::<AssetDrag>().is_some_and(|payload| matches!(payload.as_ref(), AssetDrag::Group { scope, .. } if scope == panel))
    {
        drag.canonical_drop.non_root_target_hovered.set(true);
    }
    let name = group.path.last().map(String::as_str).unwrap_or("");
    let Some(node_id) = folder_node_for_identity(&root_group.name, &group.path, hierarchy.hierarchy_identities) else { return };
    if capabilities.group_drag
        && let Some(ghost_rect) = browser_drag_ghost_rect(ui, panel, BrowserDragGhostSource::Group(node_id.clone()), response.dragged(), rect, browser_bounds)
    {
        drag.ghosts.nested_groups.push(NestedGroupDragGhost {
            rect: ghost_rect,
            open: group_open,
            name: name.to_owned(),
            depth: row_depth,
            count: group.count,
        });
    }
    if response.clicked() && !disclosure.clicked() {
        let modifiers = ui.input(|input| input.modifiers);
        if capabilities.selection {
            if modifiers.shift {
                sink.select_range(panel, hierarchy.visible_selection_order, &node_id);
            } else {
                sink.select_node(panel, node_id.clone(), modifiers.ctrl);
            }
        }
    }
    if disclosure.clicked() {
        sink.queue_group_toggle(format!(
            "{panel}/{}/{}",
            escape_hierarchy_key_part(&root_group.name),
            group.path.iter().map(|part| escape_hierarchy_key_part(part)).collect::<Vec<_>>().join("/")
        ));
    }
    if sink.is_node_selected(panel, &node_id) {
        ui.painter().rect_filled(inset_row_rect(rect, row_depth), egui::CornerRadius::ZERO, tokens.row_selected.gamma_multiply(0.35));
    }
    drag.canonical_drop.gap_headers.push((node_id.clone(), inset_row_rect(rect, row_depth), row_depth, group_open));
    drag.canonical_drop.node_bounds.borrow_mut().insert(node_id.clone(), inset_row_rect(rect, row_depth));
    if capabilities.canonical_hierarchy_drag {
        if capabilities.group_drag {
            let selection = selected_drag_nodes(sink.state, panel, hierarchy.visible_selection_order, &node_id, hierarchy.hierarchy);
            asset_browser_header_drag_source_with_id(&response, panel, node_id.clone(), name, &selection);
        }
        let hit_index = drag.canonical_drop.hits.borrow().len();
        for hit in group_header_drop_hits(inset_row_rect(rect, row_depth), node_id.clone(), row_depth, hit_index, group_open) {
            drag.canonical_drop.hits.borrow_mut().push((hit, response.clone()));
        }
    } else if let Some(dragged) = asset_browser_header_target_with_capabilities(
            ui,
            &response,
            inset_row_rect(rect, row_depth),
            panel,
            &root_group.name,
            &group.path,
            HeaderDropCapabilities {
                item_drop: capabilities.item_drop && domain_hierarchy_actions,
                // Nested path groups can only accept group drops when the consumer has a domain
                // adapter for those paths. Never consume a release the transient model cannot apply.
                group_drag: capabilities.group_drag && domain_hierarchy_actions,
                reject_group_drop: response.dnd_hover_payload::<AssetDrag>().is_some_and(|payload| {
                    matches!(payload.as_ref(), AssetDrag::Group { scope, name, path } if scope == panel && path.is_empty() && group_is_descendant(hierarchy.parents, name, &root_group.name))
                }),
            },
        )
    {
        let edge = ui.ctx().pointer_interact_pos().is_some_and(|pointer| header_drop_before(rect, pointer));
        drag.group_releases.push(GroupRelease { dragged, target_group: root_group.name.clone(), target_group_path: group.path.clone(), edge });
    }
    if let Some(menu) = ctx.context_menus.nested_group {
        let delete_context = BrowserDeleteContext {
            selection_order: hierarchy.visible_selection_order,
            hierarchy: hierarchy.hierarchy,
            identities: hierarchy.hierarchy_identities,
        };
        draw_context_menu_target(ui, response.clone(), ctx.context_menus.compact, |ui| {
            sink.with_context_menu(|state, events| {
                let start = events.len();
                menu(ui, state, panel, root_group, &group.path, events);
                replace_selected_delete_actions(events, start, state, panel, &node_id, &delete_context);
            });
        });
    }
}

/// Visibility bookkeeping for one ordered pass over the projected render segments.
/// It owns the sets mutated while ancestors and collapsed folders are traversed.
struct SegmentVisibilityState<'a> {
    hidden_groups: &'a mut BTreeSet<String>,
    hidden_folders: &'a mut BTreeSet<(String, Vec<String>)>,
    expanded_groups: &'a mut BTreeSet<String>,
}

/// Apply the existing query, ancestor-collapse and folder-collapse rules to one render segment.
/// `None` means the segment is skipped; `Some(open)` preserves its existing expanded state.
fn browser_segment_open(
    segment: &GroupRenderSegment,
    group: &GroupView,
    groups: &[GroupView],
    query: &str,
    parent_hidden: bool,
    group_is_open: bool,
    visibility: &mut SegmentVisibilityState<'_>,
) -> Option<bool> {
    let GroupRenderSegment { root_items, folder_path, item_indices, show_header, .. } = segment;
    let group_matches = group_matches_query(group, query);
    let has_matching_items = group.items.iter().any(|item| item_matches_query(item, query));
    let root_run_visible = *root_items
        && item_indices.iter().any(|(group_index, item_index)| {
            groups.get(*group_index).is_some_and(|source_group| {
                source_group.items.get(*item_index).is_some_and(|item| item_visible_for_query(item, query, group_matches_query(source_group, query)))
            })
        });
    if (*root_items && !root_run_visible) || (!*root_items && !group_visible_for_query(query, group_matches, has_matching_items)) {
        return None;
    }
    if !*root_items && *show_header && parent_hidden {
        visibility.hidden_groups.insert(group.name.clone());
        return None;
    }
    if !*root_items && visibility.hidden_groups.contains(&group.name) && !*show_header {
        return None;
    }
    if !folder_path.is_empty()
        && (1..=folder_path.len()).any(|length| visibility.hidden_folders.contains(&(group.name.clone(), folder_path[..length].to_vec())))
    {
        return None;
    }
    // A non-empty query should reveal matching items even when their group was collapsed before the search began.
    let open = *root_items || !*show_header || !query.is_empty() || group_is_open;
    if !open {
        visibility.hidden_groups.insert(group.name.clone());
    } else {
        visibility.expanded_groups.insert(group.name.clone());
    }
    Some(open)
}

/// Allocate the item grid/list area and return the geometry shared by every item in this segment.
fn allocate_browser_item_grid(ui: &mut egui::Ui, item_count: usize, layout: ItemGridLayout) -> ItemGrid {
    let ItemGridLayout { content_indent, display_mode, wrap_list, center_grid } = layout;
    let list = display_mode.uses_list_layout();
    let width = ui.available_width();
    let pad = content_indent;
    let gap = if list { 0.0 } else { GRID_GAP };
    let (cell, cols, rows) = if list {
        let row_height = display_mode.item_height();
        list_layout_geometry(width, content_indent, row_height, wrap_list, display_mode.list_width(), item_count)
    } else {
        let size = display_mode.item_height();
        let cell = vec2(size, size);
        let cols = ((width - pad + gap) / (cell.x + gap)).floor().max(1.0) as usize;
        (cell, cols, item_count.div_ceil(cols))
    };
    let grid_width = cols as f32 * cell.x + cols.saturating_sub(1) as f32 * gap;
    let center_offset = if !list && center_grid { ((width - pad - grid_width).max(0.0)) * 0.5 } else { 0.0 };
    // Reserve the exact grid height: the extra two pixels here used to accumulate with row
    // spacing and leave uneven vertical gaps.
    let grid_height = if rows == 0 { 0.0 } else { rows as f32 * cell.y + rows.saturating_sub(1) as f32 * gap };
    let (area, _) = ui.allocate_exact_size(vec2(width, grid_height), Sense::hover());
    ItemGrid {
        area,
        cell,
        columns: cols,
        rows,
        pad,
        center_offset,
        gap,
        // Match group-header alignment: apply the hierarchy inset once from the clip edge.
        origin_x: ui.clip_rect().left() + 1.0,
        column_major: wrap_list,
        item_count,
        list,
    }
}

/// Paint one item in its selected list/grid presentation without changing its hit target.
fn paint_browser_item(
    ui: &mut egui::Ui,
    painted: &PaintedCell<'_>,
    list: bool,
    display_mode: crate::pickers::picker_state::PickerDisplayMode,
    painters: &mut Painters<'_, '_>,
    tokens: &Tokens,
) {
    let response = &painted.response;
    let rect = painted.rect;
    let name = &painted.item.item.name;
    let group_index = painted.item.group_index;
    let item_index = painted.item.item_index;
    let selected = painted.selected;
    let cell_height = rect.height();
    if list {
        if display_mode == crate::pickers::picker_state::PickerDisplayMode::TextOnly {
            asset_browser_list_row(ui, response, rect, name, selected);
            ui.painter().rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.0, tokens.card_border), egui::StrokeKind::Inside);
            let label_rect = Rect::from_min_max(pos2(rect.left() + 8.0, rect.top()), rect.right_bottom());
            let mut label_ui = ui.new_child(egui::UiBuilder::new().max_rect(label_rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
            label_ui.set_clip_rect(label_rect.intersect(ui.clip_rect()));
            crate::widgets::elided_label(&mut label_ui, name, label_rect.width());
        } else if painters.has_list_item() {
            asset_browser_item_info(response, name, selected);
            painters.list_item(ui, rect, group_index, item_index, selected);
            if !selected && response.hovered() {
                ui.painter().rect_filled(rect, 2.0, tokens.hover.gamma_multiply(0.5));
            }
            ui.painter().rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.0, tokens.card_border), egui::StrokeKind::Inside);
        } else {
            asset_browser_list_row(ui, response, rect, name, selected);
            let thumb_size = (cell_height - 2.0).max(0.0);
            let thumb_rect = Rect::from_min_size(rect.min + vec2(1.0, 1.0), vec2(thumb_size, thumb_size));
            painters.item(ui, thumb_rect, group_index, item_index, ItemPaintState::Item { selected });
            ui.painter().text(pos2(thumb_rect.right() + 8.0, rect.center().y), Align2::LEFT_CENTER, name, FontId::proportional(11.5), tokens.text);
            ui.painter().rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.0, tokens.card_border), egui::StrokeKind::Inside);
        }
    } else {
        asset_browser_item_info(response, name, selected);
        painters.item(ui, rect, group_index, item_index, ItemPaintState::Item { selected });
        ui.painter().rect_stroke(
            rect,
            CornerRadius::ZERO,
            Stroke::new(if selected { 2.0 } else { 1.0 }, if selected { tokens.accent } else { tokens.card_border }),
            // Keep the entire border inside the allocated card rect: it consumes card dimensions
            // and cannot overlap neighbours.
            egui::StrokeKind::Inside,
        );
    }
}

/// Prepare the hit target and selection state for one item; painting follows in the segment loop.
fn prepare_browser_item_cell<'a>(
    ui: &mut egui::Ui,
    cell: ItemSegmentCell<'a>,
    grid: ItemGrid,
    ctx: &BrowserCtx<'_>,
    projection: &BrowserProjection,
    sink: &BrowserSink<'_>,
) -> PaintedCell<'a> {
    let ItemSegmentCell { item: item_ref, run_index, depth, root_level } = cell;
    let ItemRef { group_index: item_group_index, item_index, group: item_group, item } = item_ref;
    let rect = grid.cell_rect(run_index);
    let next_rect = grid.next_cell_rect(run_index);
    let response = ui.interact(rect, ui.id().with((ctx.panel, item_group_index, item_index)), Sense::click_and_drag());
    let cursor = if response.is_pointer_button_down_on() {
        Some(egui::CursorIcon::Grabbing)
    } else if ctx.panel == "gradients" {
        Some(egui::CursorIcon::PointingHand)
    } else {
        None
    };
    let response = if let Some(cursor) = cursor { response.on_hover_cursor(cursor) } else { response };
    let node_id = item_node_for_identity(&item_group.name, &item.group_path, &item.key, &projection.hierarchy_identities);
    let item_depth = depth + item.group_path.len() + usize::from(!root_level);
    let selected = node_id.as_ref().is_some_and(|id| sink.is_node_selected(ctx.panel, id));
    PaintedCell { item: item_ref, rect, next_rect, response, node_id, depth: item_depth, selected }
}

fn load_picker_id_registry(ui: &egui::Ui, panel: &str) -> (egui::Id, PickerIdRegistry) {
    let picker_registry_id = ui.id().with(("shared-picker-id-registry", panel));
    #[cfg(test)]
    let picker_scope = 0;
    #[cfg(not(test))]
    let picker_scope = picker_registry_id.value();
    let picker_ids = ui.ctx().data(|data| data.get_temp::<PickerIdRegistry>(picker_registry_id)).unwrap_or_else(|| PickerIdRegistry::with_scope(picker_scope));
    (picker_registry_id, picker_ids)
}

/// Draw the shared browser with an optional domain-owned mixed root sibling sequence.
///
/// Domain adapters can supply `Group` and `RootItem` entries in their authoritative order without
/// storing that order in transient `PresetUi` state. When omitted, the legacy browser path remains
/// unchanged; the supplied sequence is consumed only by domain hierarchy projections.
pub(crate) fn browser_with_root_sequence(
    ui: &mut egui::Ui,
    st: &mut PresetUi,
    adapter: &mut BrowserAdapter<'_>,
    allocated_height: f32,
    domain_root_sequence: Option<&[HierarchyOrderEntry]>,
) -> Vec<BrowserAction> {
    let t = Tokens::get(ui.ctx());
    let panel = adapter.panel;
    let (picker_registry_id, mut picker_ids) = load_picker_id_registry(ui, panel);
    let query = st.search.get(panel).map(|query| query.trim().to_lowercase()).unwrap_or_default();
    let groups = adapter.groups;
    #[cfg(test)]
    let fallback_projection_keys = ui_tests::test_projection_keys(groups);
    #[cfg(test)]
    let projection_keys = adapter.projection_keys.unwrap_or(&fallback_projection_keys);
    #[cfg(not(test))]
    let Some(projection_keys) = adapter.projection_keys else { return Vec::new() };
    let BrowserOptions { canvas, center_grid, all_groups_open, compact_context_menus, domain_hierarchy_actions, root_item_group, .. } = adapter.options;
    let capabilities = adapter.capabilities;
    // Canonical hierarchy drag/drop is opt-in: each adapter enables it only after its projected
    // tree and domain commit callback can represent every hierarchy operation it exposes.
    let context_menus = ContextMenus {
        item: adapter.item_context_menu,
        group: adapter.group_context_menu,
        nested_group: adapter.nested_group_context_menu,
        compact: compact_context_menus,
    };
    let browser_ctx = BrowserCtx { tokens: &t, panel, groups, projection_keys, capabilities, domain_hierarchy_actions, context_menus };
    let canonical_hierarchy_move = &mut adapter.canonical_hierarchy_move;
    let rekey_hierarchy_after_move = &mut adapter.rekey_hierarchy_after_move;
    let hierarchy_item_move = &mut adapter.hierarchy_item_move;
    let mut painters =
        Painters { group: adapter.paint_group.as_deref_mut(), item: &mut *adapter.paint_item, list_item: adapter.paint_list_item.as_deref_mut() };
    let display_mode = st.display_mode(panel);
    let list = display_mode.uses_list_layout();
    let wrap_list = st.wrap_list_enabled(panel) && display_mode != crate::pickers::picker_state::PickerDisplayMode::TextOnly;
    // The rename row is included in the browser allocation, not added above it. Keep its
    // measured height out of the scroll area's minimum so the shell-owned footer stays in place.
    let (rename_row_height, ev) = draw_rename_row(ui, st, panel, browser_ctx.tokens);
    let scroll_height = (allocated_height - rename_row_height).max(0.0);
    let mut drag_state = BrowserDragState::default();
    let mut expanded_groups = BTreeSet::new();
    let Some(mut projection) = BrowserProjection::build(&browser_ctx, st, &query, all_groups_open, domain_root_sequence, &mut picker_ids) else {
        return ev;
    };
    let mut sink = BrowserSink::new(st, ev);
    // Retain the shell's horizontal clip for hierarchy hit arbitration; drag-ghost movement uses
    // the narrower ScrollArea viewport captured inside the closure below.
    let browser_parent_clip = ui.clip_rect();
    // The browser is the flexible middle region of a picker panel. Keep it at the allocated
    // height even when the library is short; otherwise the footer floats up and each consumer's
    // panel geometry depends on how many presets happen to be installed.
    // Foreground ghosts are clipped only to the app viewport; their movement bounds are the
    // ScrollArea viewport below (the asset browser between the shell header and footer).
    let app_window_clip = ui.ctx().content_rect();
    // Drag previews are collected while rows are laid out, then painted from the parent UI.
    // A child created inside ScrollArea inherits its painter's viewport clip, which cannot be
    // expanded to show the preview elsewhere in the picker window.
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
            let mut segment_visibility = SegmentVisibilityState {
                hidden_groups: &mut hidden_groups,
                hidden_folders: &mut hidden_folders,
                expanded_groups: &mut expanded_groups,
            };
            // Resolve hierarchy-on-card candidates across every visible group segment so a
            // group drag can show only one insertion line in the entire shared browser.
            for projected_segment in &projection.render_segments {
                let segment = projected_segment;
                let gi = segment.group_index;
                let depth = segment.depth;
                let root_items = segment.root_items;
                let folder_path = segment.folder_path.as_slice();
                let item_indices = segment.item_indices.as_slice();
                let show_header = segment.show_header;
                let show_folder_header = segment.show_folder_header;
                let Some(g) = groups.get(gi) else { continue };
                let parent_hidden = projection.parents.get(&g.name).is_some_and(|parent| segment_visibility.hidden_groups.contains(parent));
                let group_is_open = if segment.root_items || !segment.show_header || !query.is_empty() {
                    true
                } else {
                    sink.is_group_open(panel, gi, &g.name, all_groups_open)
                };
                let Some(open) = browser_segment_open(segment, g, groups, &query, parent_hidden, group_is_open, &mut segment_visibility) else {
                    continue;
                };
                if show_header {
                    let BrowserCtx { panel, capabilities, projection_keys, context_menus, .. } = browser_ctx;
                    if !drag_state.canonical_drop.gap_headers.is_empty() {
                        ui.add_space((GROUP_LAYOUT_GAP - ui.spacing().item_spacing.y).max(0.0));
                    }
                    let (header_rect, response) = draw_browser_group_header(ui, g, open, depth, &mut painters);
                    let header_hit_rect = inset_row_rect(header_rect, depth);
                    let disclosure_rect = Rect::from_min_max(
                        pos2(header_hit_rect.left(), header_hit_rect.top()),
                        pos2((header_hit_rect.left() + 18.0).min(header_hit_rect.right()), header_hit_rect.bottom()),
                    );
                    let disclosure = ui.interact(disclosure_rect, response.id.with("disclosure"), Sense::click());
                    if capabilities.group_drag && response.is_pointer_button_down_on() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                    } else if capabilities.group_drag && response.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    let group_id = picker_group_id(&mut picker_ids, gi, projection_keys);
                    drag_state.canonical_drop.gap_headers.push((group_id.clone(), header_hit_rect, depth, open));
                    drag_state.canonical_drop.node_bounds.borrow_mut().insert(group_id.clone(), header_hit_rect);
                    let header_released = if capabilities.canonical_hierarchy_drag {
                        if capabilities.group_drag {
                            let selection = selected_drag_nodes(sink.state, panel, &projection.visible_selection_order, &group_id, &projection.hierarchy);
                            asset_browser_header_drag_source_with_id(&response, panel, group_id.clone(), &g.name, &selection);
                        }
                        let hit_index = drag_state.canonical_drop.hits.borrow().len();
                        for hit in group_header_drop_hits(header_hit_rect, group_id.clone(), depth, hit_index, open) {
                            drag_state.canonical_drop.hits.borrow_mut().push((hit, response.clone()));
                        }
                        None
                    } else {
                        asset_browser_header_target_with_capabilities(
                            ui,
                            &response,
                            header_hit_rect,
                            panel,
                            &g.name,
                            &[],
                            HeaderDropCapabilities {
                                item_drop: capabilities.item_drop,
                                group_drag: capabilities.group_drag,
                                reject_group_drop: response.dnd_hover_payload::<AssetDrag>().is_some_and(|payload| {
                                    matches!(payload.as_ref(), AssetDrag::Group { scope, name, path } if scope == panel && path.is_empty() && group_is_descendant(&projection.parents, name, &g.name))
                                }),
                            },
                        )
                    };
                    if response.clicked() && !disclosure.clicked() {
                        let modifiers = ui.input(|input| input.modifiers);
                        if capabilities.selection {
                            if modifiers.shift {
                                sink.select_range(panel, &projection.visible_selection_order, &group_id);
                            } else {
                                sink.select_node(panel, group_id.clone(), modifiers.ctrl);
                            }
                        }
                    }
                    if disclosure.clicked() {
                        let key = format!("{panel}/{}", escape_hierarchy_key_part(&g.name));
                        sink.toggle_group(key);
                    }
                    if sink.is_node_selected(panel, &group_id) {
                        ui.painter().rect_filled(header_hit_rect, egui::CornerRadius::ZERO, browser_ctx.tokens.row_selected.gamma_multiply(0.35));
                    }
                    if let Some(menu) = context_menus.group {
                        let delete_context = BrowserDeleteContext {
                            selection_order: &projection.visible_selection_order,
                            hierarchy: &projection.hierarchy,
                            identities: &projection.hierarchy_identities,
                        };
                        draw_context_menu_target(ui, response.clone(), context_menus.compact, |ui| {
                            sink.with_context_menu(|state, events| {
                                let start = events.len();
                                menu(ui, state, panel, g, events);
                                replace_selected_delete_actions(events, start, state, panel, &group_id, &delete_context);
                            });
                        });
                    }
                    if let Some(dragged) = header_released {
                        apply_root_header_release(
                            ui,
                            &mut sink,
                            &browser_ctx,
                            &projection.hierarchy_orders,
                            RootHeaderReleaseEvent { group: g, header_rect, dragged, browser_item_drop: &mut drag_state.browser_item_drop },
                        );
                    }
                    if capabilities.group_drag && response.dragged() && let Some(pointer) = ui.ctx().pointer_interact_pos() {
                        drag_state.group_drag = Some((gi, pointer));
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                    }
                    if capabilities.group_drag
                        && let Some(ghost_rect) = browser_drag_ghost_rect(
                            ui,
                            panel,
                            BrowserDragGhostSource::Group(group_id.clone()),
                            response.dragged(),
                            header_rect,
                            browser_bounds,
                        )
                    {
                        drag_state.ghosts.groups.push(GroupDragGhost { rect: ghost_rect, group_index: gi, open, depth });
                    }
                    if capabilities.group_drag
                        && !capabilities.canonical_hierarchy_drag
                        && response.drag_stopped()
                        && let Some(pointer) = ui.ctx().pointer_interact_pos()
                    {
                        drag_state.group_release = Some((gi, pointer));
                    }
                    if capabilities.group_drag && capabilities.selection && response.drag_stopped() && !sink.is_node_selected(panel, &group_id) {
                        sink.select_node(panel, group_id, false);
                    }
                    drag_state.headers.push((gi, inset_row_rect(header_rect, depth), g.name.clone()));
                }
                // Keep each group's assets directly beneath its header. The previous separate
                // content pass placed every swatch after every group header, so expanding a group
                // could not reveal its assets in the expected place.
                let folder_group = HierarchyGroup {
                    path: folder_path.to_vec(),
                    children: Vec::new(),
                    count: g.items.iter().filter(|item| item.group_path.starts_with(folder_path)).count(),
                };
                let folder_open = folder_path.is_empty() || {
                    let path = folder_path.iter().map(|part| escape_hierarchy_key_part(part)).collect::<Vec<_>>().join("/");
                    !sink.is_group_toggled(&format!("{panel}/{}/{}", escape_hierarchy_key_part(&g.name), path))
                };
                if open {
                    {
                        let segment_items: Vec<ItemRef<'_>> = item_indices
                            .iter()
                            .filter_map(|(group_index, item_index)| {
                                let source_group = groups.get(*group_index)?;
                                let item = source_group.items.get(*item_index)?;
                                item_visible_for_query(item, &query, group_matches_query(source_group, &query)).then_some(ItemRef {
                                    group_index: *group_index,
                                    item_index: *item_index,
                                    group: source_group,
                                    item,
                                })
                            })
                            .collect();
                        if show_folder_header {
                            let hierarchy_view = BrowserHierarchyView {
                                groups,
                                group_names: &projection.group_names,
                                parents: &projection.parents,
                                hierarchy: &projection.hierarchy,
                                hierarchy_identities: &projection.hierarchy_identities,
                                hierarchy_orders: &projection.hierarchy_orders,
                                visible_selection_order: &projection.visible_selection_order,
                            };
                            draw_nested_folder_row(
                                ui,
                                NestedFolderRow { group: &folder_group, root_group: g, open: folder_open, depth: folder_path.len().saturating_sub(1), parent_depth: depth },
                                &browser_ctx,
                                &hierarchy_view,
                                &mut sink,
                                &mut drag_state,
                            );
                            if folder_open {
                                segment_visibility.hidden_folders.remove(&(g.name.clone(), folder_path.to_vec()));
                            } else {
                                segment_visibility.hidden_folders.insert((g.name.clone(), folder_path.to_vec()));
                            }
                        }
                        let mut draw_items = |ui: &mut egui::Ui, sink: &mut BrowserSink<'_>, items: &[ItemRef<'_>], group_path: &[String]| {
                            let root_level_items = root_items;
                            let grid = allocate_browser_item_grid(
                                ui,
                                items.len(),
                                ItemGridLayout {
                                    content_indent: if root_level_items { 0.0 } else { row_indent(depth + group_path.len() + 1) },
                                    display_mode,
                                    wrap_list,
                                    center_grid,
                                },
                            );
                            if grid.area.height() > 0.0 && !root_level_items {
                                drag_state.group_content_areas.push((g.name.clone(), grid.area));
                            }
                            drag_state.item_insertion_markers.clear();
                            for (run_i, item_ref) in items.iter().copied().enumerate() {
                                let it = item_ref.item;
                                let painted = prepare_browser_item_cell(
                                    ui,
                                    ItemSegmentCell { item: item_ref, run_index: run_i, depth, root_level: root_level_items },
                                    grid,
                                    &browser_ctx,
                                    &projection,
                                    sink,
                                );
                                paint_browser_item(ui, &painted, grid.list, display_mode, &mut painters, browser_ctx.tokens);
                                let released_drag = register_browser_item_drop_targets(
                                    ui,
                                    ItemDropRegistrationGeometry {
                                        painted: &painted,
                                        grid,
                                        run_index: run_i,
                                    },
                                    &browser_ctx,
                                    &projection,
                                    sink,
                                    &mut drag_state,
                                );
                                if let Some(AssetDrag::Preset { scope, key, node_id, .. }) = released_drag
                                    && scope.as_str() == panel
                                    && key.as_str() != it.key.as_str()
                                {
                                    drag_state.browser_item_drop = true;
                                    let after = ui.ctx().pointer_interact_pos().is_some_and(|p| item_drop_after(painted.rect, p, !list));
                                    commit_item_drop(sink, ItemDropCommit {
                                        ctx: &browser_ctx,
                                        projection: &projection,
                                        target: &painted,
                                        source: ItemDropSource { key: &key, node_id: node_id.as_ref() },
                                        root_level_items,
                                        after,
                                    });
                                }
                                let resp = apply_browser_item_gesture(ui, &painted, &browser_ctx, sink, &projection.visible_selection_order);
                                if capabilities.item_drag && resp.dragged() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                                }
                                if capabilities.item_drag
                                    && let Some(ghost_rect) = browser_drag_ghost_rect(
                                        ui,
                                        panel,
                                        BrowserDragGhostSource::Item { group: painted.item.group_index, item: painted.item.item_index },
                                        resp.dragged(),
                                        painted.rect,
                                        browser_bounds,
                                    )
                                {
                                    drag_state.ghosts.items.push(ItemDragGhost {
                                        rect: ghost_rect,
                                        group_index: painted.item.group_index,
                                        item_index: painted.item.item_index,
                                        display_mode,
                                        selected: painted.selected,
                                    });
                                }
                                if capabilities.item_drag
                                    && resp.drag_stopped()
                                    && let Some(pp) = ui.ctx().pointer_interact_pos()
                                    && canvas_drop_target(pp, painted.rect, canvas, browser_bounds)
                                {
                                    drag_state.dropped = Some((it.key.clone(), pp));
                                }
                                if capabilities.item_drag
                                    && capabilities.selection
                                    && resp.drag_stopped()
                                    && let Some(id) = painted.node_id.as_ref()
                                    && !sink.is_node_selected(panel, id)
                                {
                                    sink.select_node(panel, id.clone(), false);
                                }
                                if let Some(menu) = context_menus.item {
                                    let args = ItemContextMenuArgs {
                                        panel,
                                        item: it,
                                        group: painted.item.group,
                                        group_names: &projection.group_names,
                                        selected_items: &projection.selected_items,
                                    };
                                    let delete_context = BrowserDeleteContext {
                                        selection_order: &projection.visible_selection_order,
                                        hierarchy: &projection.hierarchy,
                                        identities: &projection.hierarchy_identities,
                                    };
                        draw_context_menu_target(ui, resp.clone(), context_menus.compact, |ui| {
                                        sink.with_context_menu(|state, events| {
                                            let start = events.len();
                                            menu(ui, state, args, events);
                                            if let Some(node_id) = painted.node_id.as_ref() {
                                                replace_selected_delete_actions(events, start, state, panel, node_id, &delete_context);
                                            }
                                        });
                                    });
                                }
                            }
                            for (marker_rect, after, grid) in drag_state.item_insertion_markers.drain(..) {
                                crate::widgets::drop_line(ui, marker_rect, after, grid, browser_ctx.tokens);
                            }
                        };
                        if folder_open {
                            draw_items(ui, &mut sink, &segment_items, folder_path);
                        }
                    }
                    let group_releases = std::mem::take(&mut drag_state.group_releases);
                    let group_toggles = std::mem::take(&mut sink.pending_group_toggles);
                    apply_group_releases_and_toggles(&mut sink, GroupReleaseBatch { group_releases, group_toggles }, &browser_ctx, &mut drag_state);
                }
            }
            let hierarchy_view = BrowserHierarchyView {
                groups,
                group_names: &projection.group_names,
                parents: &projection.parents,
                hierarchy: &projection.hierarchy,
                hierarchy_identities: &projection.hierarchy_identities,
                hierarchy_orders: &projection.hierarchy_orders,
                visible_selection_order: &projection.visible_selection_order,
            };
            let item_hierarchy_target_wins = resolve_item_hierarchy_drop_target(
                ui,
                &drag_state.item_hierarchy_targets,
                &hierarchy_view,
                ItemHierarchyDropPolicy { panel, capabilities, domain_hierarchy_actions },
                &mut sink,
                ItemHierarchyDropResolution {
                    indent_release: &mut drag_state.indent_release,
                    browser_item_drop: &mut drag_state.browser_item_drop,
                    released: &mut drag_state.group_release,
                },
            );
            register_group_indent_drop_targets(ui, &browser_ctx, &projection, &mut drag_state, &expanded_groups, &mut picker_ids);
            if capabilities.canonical_hierarchy_drag {
                register_canonical_sibling_gap_targets(
                    ui,
                    &browser_ctx,
                    &projection,
                    &drag_state,
                );
            }
            let indent_target_index = resolve_group_indent_target(
                ui,
                &browser_ctx,
                &mut drag_state,
                item_hierarchy_target_wins,
                &projection.parents,
            );
            drag_state.item_hierarchy_target_wins = item_hierarchy_target_wins;
            paint_group_drop_insertion_markers(ui, indent_target_index, &browser_ctx, &projection, &drag_state, browser_ctx.tokens.accent);
            register_canonical_root_end_target(ui, capabilities, &projection.hierarchy, &drag_state.canonical_drop, list);
        });
    let mut ev = sink.into_actions();
    if empty_area_clicked.get() {
        st.browser_selection.entry(panel.to_owned()).or_default().clear();
        st.browser_selection_anchor.remove(panel);
    }
    painters.paint_drag_ghosts(ui, panel, groups, app_window_clip, std::mem::take(&mut drag_state.ghosts), browser_ctx.tokens);
    let browser_view_clip =
        Rect::from_min_max(pos2(browser_parent_clip.left(), browser_parent_clip.top()), pos2(browser_parent_clip.right(), scroll.inner_rect.bottom()))
            .intersect(browser_parent_clip);
    register_canonical_root_leading_target(ui, &browser_ctx, &projection, &drag_state, browser_parent_clip);
    let mut hierarchy_drop_feedback = st.browser_feedback.get(panel).cloned();
    if capabilities.canonical_hierarchy_drag
        && let Some(result) = resolve_canonical_drop(
            ui,
            &browser_ctx,
            &mut projection,
            &drag_state,
            HierarchyCommit {
                state: st,
                picker_ids: &mut picker_ids,
                root_item_group,
                callbacks: CanonicalDropCallbacks {
                    hierarchy_move: canonical_hierarchy_move,
                    rekey_after_move: rekey_hierarchy_after_move,
                    item_move: hierarchy_item_move,
                },
                viewport: CanonicalDropViewport { parent_clip: browser_parent_clip, view_clip: browser_view_clip },
            },
            browser_ctx.tokens.accent,
        )
    {
        hierarchy_drop_feedback = result.err();
    }
    if let Some(message) = hierarchy_drop_feedback.as_ref() {
        st.browser_feedback.insert(panel.to_owned(), message.clone());
        ui.colored_label(ui.visuals().error_fg_color, message);
    } else {
        st.browser_feedback.remove(panel);
    }
    // A nested group header is a distinct target family from the root-group rows. If the pointer
    // is over one, the root resolver must not reinterpret that same release as a root reorder.
    if drag_state.canonical_drop.non_root_target_hovered.get() {
        drag_state.group_release = None;
    }
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("preset-browser-rect", panel)), scroll.inner_rect));
    // Apply the drop now every header rect is known; the next frame draws the new order.
    if !capabilities.canonical_hierarchy_drag {
        ev.extend(apply_legacy_group_release(st, &browser_ctx, &projection, &drag_state));
    }
    let recorded: Vec<(String, Rect)> = drag_state.headers.iter().map(|(_, r, n)| (n.clone(), *r)).collect();
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("preset-group-headers", panel)), recorded));
    if let Some((key, pos)) = drag_state.dropped.filter(|_| !drag_state.browser_item_drop) {
        ev.push(BrowserAction::Drop(key, pos));
    }
    ui.ctx().data_mut(|data| data.insert_temp(picker_registry_id, picker_ids));
    ev
}

#[cfg(test)]
#[path = "asset_browser_ui_tests.rs"]
mod ui_tests;
#[cfg(test)]
pub(crate) use ui_tests::{test_last_browser_rect as last_browser_rect, test_last_group_headers as last_group_headers};
