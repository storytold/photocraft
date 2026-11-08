//! Asset-neutral picker browser model, rendering, and interaction routing.
//!
//! Consumers project their domain assets into `GroupView` and `ItemView`, provide preview painters,
//! and translate `BrowserAction` into domain commands. This module owns the common browser presentation only.

use std::collections::BTreeSet;

use egui::{Align2, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, pos2, vec2};

use crate::pickers::browser::browser_gestures::apply_browser_item_gesture;
use crate::pickers::browser::browser_interaction::{
    AssetDrag, BrowserDeleteContext, BrowserDragGhostSource, GroupDragGhost, ItemDragGhost, NestedGroupDragGhost, asset_browser_header_drag_source_with_id,
    browser_drag_ghost_rect, draw_context_menu_target, draw_rename_row, item_row_edge_targets, replace_selected_delete_actions,
};
pub(crate) use crate::pickers::browser::browser_interaction::{
    begin_item_rename, delete_item_actions, item_context_targets, move_item_actions, standard_group_context_menu, standard_item_context_menu,
};
use crate::pickers::browser::context_menu::selected_delete_actions;
use crate::pickers::browser::dragging::{
    BrowserDragState, canvas_drop_target, register_browser_item_drop_targets, register_card, register_group_indent_drop_targets, register_group_objects,
    register_indentation_objects, register_root_objects, resolve_canonical_drop,
};
use crate::pickers::browser::dragging_model::DropMoveIntent;
use crate::pickers::browser::groups::{GROUP_HEADER_HEIGHT, group_open, nested_header as draw_nested_folder_header, row_indent};
use crate::pickers::browser::hierarchy::HierarchyGroup;
use crate::pickers::browser::identity::{PickerIdentityKey, PickerNodeIdentity, PickerProjectionKeys, ProjectionNodeRef};
use crate::pickers::browser::items::{
    group_matches_query, group_visible_for_query, item_grid_position, item_info as asset_browser_item_info, item_matches_query, item_visible_for_query,
    list_layout_geometry, list_row as asset_browser_list_row,
};
use crate::pickers::browser::path_identity_model::{ContainerId, PickerBrowserHierarchy, PickerId, PickerIdRegistry};
use crate::pickers::browser::projection::{GroupRenderSegment, build_hierarchy_projection};
pub(crate) use crate::pickers::browser::projection::{GroupView, ItemView};
use crate::pickers::state::escape_hierarchy_key_part;
use crate::pickers::state::{HierarchyOrderEntry, PresetUi};
use crate::theme::Tokens;

pub(crate) const MIN_THUMB: f32 = 20.0;

#[cfg(test)]
use crate::pickers::browser::browser_interaction::advance_item_rename;
#[cfg(test)]
use crate::pickers::browser::dragging::item_occurrence_position;
// 27px of header content plus a one-pixel border on each vertical edge.
const GROUP_LAYOUT_GAP: f32 = 0.0;
// Group-header edge bands own their adjacent row spacing; there is no separate gap target.

/// Geometry shared by every item in one rendered grid/list segment.
#[derive(Clone, Copy, Debug)]
struct ItemGridLayout {
    content_indent: f32,
    root_level_items: bool,
    display_mode: crate::pickers::state::PickerDisplayMode,
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
    pub(super) response: egui::Response,
    pub(super) node_id: Option<PickerId>,
    pub(super) depth: usize,
    pub(super) selected: bool,
}

#[cfg(test)]
#[derive(Clone, Debug)]
struct BrowserDropGeometrySnapshot {
    hits: Vec<(String, crate::pickers::browser::dragging::DropArea, Rect, Rect)>,
    indentation_lanes: Vec<(String, Rect)>,
    browser_parent_clip: Rect,
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
    /// Move an item into a top-level group through adapters that expose the legacy item command.
    Move(String, String),
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
    /// Begin in-place renaming for the supplied item occurrences.
    BeginRenameItems(Vec<(String, ItemView)>),
    /// Begin in-place renaming of a top-level group.
    BeginRenameGroup(String),
    /// Begin in-place renaming of multiple top-level groups, one after another.
    BeginRenameGroups(Vec<String>),
    DeleteItem {
        group: String,
        group_path: Vec<String>,
        key: String,
    },
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

/// All derived model state needed by one browser frame. Keeping projection, filtering and visible
/// selection order together prevents the rendering pass from rebuilding subtly different views of
/// the same hierarchy.
pub(super) struct BrowserProjection {
    pub(super) group_names: Vec<String>,
    pub(super) parents: std::collections::BTreeMap<String, String>,
    render_segments: Vec<GroupRenderSegment>,
    pub(super) hierarchy: PickerBrowserHierarchy,
    pub(super) hierarchy_identities: std::collections::BTreeMap<PickerId, PickerNodeIdentity>,
    selected_items: Vec<(String, ItemView)>,
    pub(super) visible_selection_order: Vec<PickerId>,
}

/// Borrowed hierarchy data shared by hierarchy interaction helpers for one browser frame.
pub(super) struct BrowserHierarchyView<'a> {
    pub(super) hierarchy: &'a PickerBrowserHierarchy,
    pub(super) hierarchy_identities: &'a std::collections::BTreeMap<PickerId, PickerNodeIdentity>,
    pub(super) visible_selection_order: &'a [PickerId],
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

        Some(Self { group_names, parents, render_segments, hierarchy, hierarchy_identities, selected_items, visible_selection_order })
    }
}

use crate::pickers::browser::groups::{header as asset_browser_header, inset_row_rect};

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
}

impl Default for BrowserCapabilities {
    fn default() -> Self {
        Self { item_drag: true, item_drop: true, group_drag: true, selection: true, activation: true }
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
    /// path-aware folder/item moves). Returning an error rejects the move without changing UI state.
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
    let BrowserCtx { panel, capabilities, .. } = *ctx;
    let tokens = ctx.tokens;
    let browser_bounds = ui.clip_rect();
    if !drag.canonical_drop.group_headers.is_empty() {
        ui.add_space((GROUP_LAYOUT_GAP - ui.spacing().item_spacing.y).max(0.0));
    }
    let (rect, response, disclosure, row_depth) = draw_nested_folder_header(ui, group, group_open, group_depth, parent_depth);
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
    drag.canonical_drop.group_headers.push((node_id.clone(), inset_row_rect(rect, row_depth), row_depth, group_open));
    drag.canonical_drop.node_bounds.borrow_mut().insert(node_id.clone(), inset_row_rect(rect, row_depth));
    if capabilities.group_drag {
        let selection = selected_drag_nodes(sink.state, panel, hierarchy.visible_selection_order, &node_id, hierarchy.hierarchy);
        asset_browser_header_drag_source_with_id(&response, panel, node_id.clone(), name, &selection);
    }
    if let Some(menu) = ctx.context_menus.nested_group {
        let delete_context = BrowserDeleteContext { hierarchy: hierarchy.hierarchy, identities: hierarchy.hierarchy_identities };
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
    let ItemGridLayout { content_indent, root_level_items, display_mode, wrap_list, center_grid } = layout;
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
    let origin_x = if root_level_items { ui.clip_rect().left() + 1.0 } else { area.left() + 1.0 };
    ItemGrid { area, cell, columns: cols, rows, pad, center_offset, gap, origin_x, column_major: wrap_list, item_count, list }
}

/// Paint one item in its selected list/grid presentation without changing its hit target.
fn paint_browser_item(
    ui: &mut egui::Ui,
    painted: &PaintedCell<'_>,
    list: bool,
    display_mode: crate::pickers::state::PickerDisplayMode,
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
        if display_mode == crate::pickers::state::PickerDisplayMode::TextOnly {
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
    PaintedCell { item: item_ref, rect, response, node_id, depth: item_depth, selected }
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
    let wrap_list = st.wrap_list_enabled(panel) && display_mode != crate::pickers::state::PickerDisplayMode::TextOnly;
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
            let mut segment_visibility =
                SegmentVisibilityState { hidden_groups: &mut hidden_groups, hidden_folders: &mut hidden_folders, expanded_groups: &mut expanded_groups };
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
                    if !drag_state.canonical_drop.group_headers.is_empty() {
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
                    drag_state.canonical_drop.group_headers.push((group_id.clone(), header_hit_rect, depth, open));
                    drag_state.canonical_drop.node_bounds.borrow_mut().insert(group_id.clone(), header_hit_rect);
                    if capabilities.group_drag {
                        let selection = selected_drag_nodes(sink.state, panel, &projection.visible_selection_order, &group_id, &projection.hierarchy);
                        asset_browser_header_drag_source_with_id(&response, panel, group_id.clone(), &g.name, &selection);
                    }
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
                        let delete_context = BrowserDeleteContext { hierarchy: &projection.hierarchy, identities: &projection.hierarchy_identities };
                        draw_context_menu_target(ui, response.clone(), context_menus.compact, |ui| {
                            sink.with_context_menu(|state, events| {
                                let start = events.len();
                                menu(ui, state, panel, g, events);
                                replace_selected_delete_actions(events, start, state, panel, &group_id, &delete_context);
                            });
                        });
                    }
                    if capabilities.group_drag
                        && let Some(ghost_rect) =
                            browser_drag_ghost_rect(ui, panel, BrowserDragGhostSource::Group(group_id.clone()), response.dragged(), header_rect, browser_bounds)
                    {
                        drag_state.ghosts.groups.push(GroupDragGhost { rect: ghost_rect, group_index: gi, open, depth });
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
                                hierarchy: &projection.hierarchy,
                                hierarchy_identities: &projection.hierarchy_identities,
                                visible_selection_order: &projection.visible_selection_order,
                            };
                            draw_nested_folder_row(
                                ui,
                                NestedFolderRow {
                                    group: &folder_group,
                                    root_group: g,
                                    open: folder_open,
                                    depth: folder_path.len().saturating_sub(1),
                                    parent_depth: depth,
                                },
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
                                    root_level_items,
                                    display_mode,
                                    wrap_list,
                                    center_grid,
                                },
                            );
                            if grid.area.height() > 0.0 && !root_level_items {
                                drag_state.group_content_areas.push((g.name.clone(), grid.area));
                            }
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
                                register_browser_item_drop_targets(
                                    ItemDropRegistrationGeometry { painted: &painted },
                                    &browser_ctx,
                                    &projection,
                                    sink,
                                    &mut drag_state,
                                );
                                if let Some(node_id) = painted.node_id.clone() {
                                    let row_end = if !grid.list {
                                        let last = run_i + 1 == grid.item_count || (run_i + 1) % grid.columns == 0;
                                        item_row_edge_targets(grid.area, painted.rect, last)
                                    } else {
                                        None
                                    };
                                    register_card(&drag_state, node_id, painted.rect, painted.depth, grid.list, row_end);
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
                                    let delete_context =
                                        BrowserDeleteContext { hierarchy: &projection.hierarchy, identities: &projection.hierarchy_identities };
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
                        };
                        if folder_open {
                            draw_items(ui, &mut sink, &segment_items, folder_path);
                        }
                    }
                    let group_toggles = std::mem::take(&mut sink.pending_group_toggles);
                    for key in group_toggles {
                        if !sink.state.toggled.remove(&key) {
                            sink.state.toggled.insert(key);
                        }
                    }
                }
            }
            register_group_indent_drop_targets(ui, &browser_ctx, &projection, &mut drag_state, &expanded_groups);
            register_group_objects(&projection, &drag_state);
            register_indentation_objects(&projection, &drag_state);
            register_root_objects(ui, &projection.hierarchy, &drag_state, list, Some(browser_parent_clip));
            #[cfg(test)]
            ui.ctx().data_mut(|data| {
                data.insert_temp(
                    egui::Id::new(("preset-browser-drop-geometry", panel)),
                    BrowserDropGeometrySnapshot {
                        hits: crate::pickers::browser::dragging::test_drop_geometry(&drag_state),
                        indentation_lanes: drag_state.indent_drop_targets.iter().map(|(name, response)| (name.clone(), response.rect)).collect(),
                        browser_parent_clip,
                    },
                );
            });
        });
    let mut ev = sink.into_actions();
    let mut selection_actions = Vec::new();
    if let Some(selection) = st.browser_selection.get(panel).filter(|selection| !selection.is_empty()) {
        let selected_items: Vec<(String, ItemView)> = groups
            .iter()
            .flat_map(|group| {
                group.items.iter().filter_map(|item| {
                    item_node_for_identity(&group.name, &item.group_path, &item.key, &projection.hierarchy_identities)
                        .filter(|id| selection.contains(id))
                        .map(|_| (group.name.clone(), item.clone()))
                })
            })
            .collect();
        if !selected_items.is_empty() {
            selection_actions.push(BrowserAction::BeginRenameItems(selected_items));
        }
        let mut selected_groups = Vec::new();
        for (id, identity) in &projection.hierarchy_identities {
            if selection.contains(id)
                && let PickerNodeIdentity::Group(group) = identity
            {
                selected_groups.push(group.clone());
            }
        }
        if !selected_groups.is_empty() {
            selection_actions.push(BrowserAction::BeginRenameGroups(selected_groups));
        }
    }
    ui.ctx().data_mut(|data| {
        data.insert_temp(egui::Id::new(("preset-browser-selection-actions", panel)), selection_actions);
    });
    let last_selected_kind = st
        .browser_selection_anchor
        .get(panel)
        .and_then(|anchor| projection.hierarchy_identities.get(anchor))
        .map(|identity| matches!(identity, PickerNodeIdentity::Group(_) | PickerNodeIdentity::Folder { .. }));
    if let Some(is_group) = last_selected_kind {
        ui.ctx().data_mut(|data| data.insert_temp(egui::Id::new(("preset-browser-last-selection-is-group", panel)), is_group));
    }
    let selected_delete_actions = st
        .browser_selection
        .get(panel)
        .and_then(|selection| {
            projection.hierarchy_identities.keys().find(|id| selection.contains(*id)).and_then(|clicked| {
                let context = BrowserDeleteContext { hierarchy: &projection.hierarchy, identities: &projection.hierarchy_identities };
                selected_delete_actions(selection, clicked, &context)
            })
        })
        .unwrap_or_default();
    ui.ctx().data_mut(|data| {
        data.insert_temp(egui::Id::new(("preset-browser-delete-selection", panel)), selected_delete_actions);
    });
    if empty_area_clicked.get() {
        st.browser_selection.entry(panel.to_owned()).or_default().clear();
        st.browser_selection_anchor.remove(panel);
    }
    painters.paint_drag_ghosts(ui, panel, groups, app_window_clip, std::mem::take(&mut drag_state.ghosts), browser_ctx.tokens);
    let browser_view_clip =
        Rect::from_min_max(pos2(browser_parent_clip.left(), browser_parent_clip.top()), pos2(browser_parent_clip.right(), scroll.inner_rect.bottom()))
            .intersect(browser_parent_clip);
    let _ = resolve_canonical_drop(
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
    );
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("preset-browser-rect", panel)), scroll.inner_rect));
    let recorded: Vec<(String, Rect)> = drag_state.headers.iter().map(|(_, r, n)| (n.clone(), *r)).collect();
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("preset-group-headers", panel)), recorded));
    if let Some((key, pos)) = drag_state.dropped {
        ev.push(BrowserAction::Drop(key, pos));
    }
    ui.ctx().data_mut(|data| data.insert_temp(picker_registry_id, picker_ids));
    ev
}

#[cfg(test)]
#[path = "browser_ui_tests.rs"]
mod ui_tests;
#[cfg(test)]
pub(crate) use ui_tests::{test_last_browser_rect as last_browser_rect, test_last_group_headers as last_group_headers};
