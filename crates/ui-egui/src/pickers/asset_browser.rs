//! Asset-neutral picker browser model, rendering, and interaction routing.
//!
//! Consumers project their domain assets into `GroupView` and `ItemView`, provide preview painters,
//! and translate `BrowserAction` into domain commands. This module owns the common browser presentation only.

use std::collections::BTreeSet;

use egui::{Align2, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, pos2, vec2};

use crate::pickers::asset_browser_groups::{ROW_INDENT, group_open, row_indent};
use crate::pickers::asset_browser_hierarchy::HierarchyGroup;
use crate::pickers::asset_browser_interaction::{
    AssetDrag, ContextMenuAction, GroupGapDropSpec, HeaderDropCapabilities, HierarchyDrop, ItemDropTarget, asset_browser_header_drag_source_with_id,
    asset_browser_header_target_with_capabilities, asset_browser_item_drag_source_with_id, asset_browser_item_gesture, asset_browser_item_hierarchy_target,
    asset_browser_item_target, asset_browser_take_release, card_drop_hit, card_drop_hits, card_leading_marker_rect, card_trailing_marker_rect,
    context_menu_target, draw_group_context_menu, draw_item_context_menu, group_append_hit, group_gap_drop_hits, group_gap_upper_anchor,
    group_header_drop_hits, header_drop_before, hierarchy_group_drop, item_drop_after, item_row_edge_targets, paint_group_insertion_line, root_append_hit,
    root_prepend_hit,
};
use crate::pickers::asset_browser_items::{item_info as asset_browser_item_info, list_row as asset_browser_list_row};
use crate::pickers::picker_browser_dragging::{
    DragSession, DropCommitResult, DropHitRegion, DropMoveIntent, PointerRegion, arbitrate_hit_index, hierarchy_move_sources, paint_candidate,
};
use crate::pickers::picker_browser_hierarchy::commit_hierarchy_move;
use crate::pickers::picker_browser_identity::{PickerNodeIdentity, PickerNodeRole, PickerProjectionKeys, ProjectionNodeRef};
use crate::pickers::picker_browser_model::{ContainerId, DropPosition, HierarchyNode, PickerBrowserHierarchy, PickerId, PickerIdRegistry};
use crate::pickers::picker_browser_projection::{GroupRenderSegment, build_hierarchy_projection};
pub(crate) use crate::pickers::picker_browser_projection::{GroupView, ItemView};
use crate::pickers::picker_state::escape_hierarchy_key_part;
use crate::pickers::picker_state::{HierarchyOrderEntry, ItemRenameState, PresetUi};
use crate::theme::Tokens;

pub(crate) const MIN_THUMB: f32 = 20.0;
// 27px of header content plus a one-pixel border on each vertical edge.
const GROUP_HEADER_HEIGHT: f32 = 29.0;
const GROUP_LAYOUT_GAP: f32 = 0.0;
// The six-pixel band immediately before a group is split into two exact three-pixel targets.
const GROUP_GAP_DROP_HEIGHT: f32 = 6.0;
const GROUP_DRAG_GHOST_OPACITY: f32 = 0.4;

/// Geometry shared by every item in one rendered grid/list segment.
#[derive(Clone, Copy, Debug)]
struct ItemGridLayout {
    content_indent: f32,
    display_mode: crate::pickers::picker_state::PickerDisplayMode,
    wrap_list: bool,
    center_grid: bool,
}

#[derive(Clone, Copy, Debug)]
struct ItemGrid {
    area: Rect,
    cell: egui::Vec2,
    columns: usize,
    rows: usize,
    pad: f32,
    center_offset: f32,
    gap: f32,
    origin_x: f32,
    column_major: bool,
    item_count: usize,
    list: bool,
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
struct ItemRef<'a> {
    group_index: usize,
    item_index: usize,
    group: &'a GroupView,
    item: &'a ItemView,
}

/// Result of painting an item occurrence; keeps source identity, geometry and interaction together.
struct PaintedCell<'a> {
    item: ItemRef<'a>,
    rect: Rect,
    next_rect: Option<Rect>,
    response: egui::Response,
    node_id: Option<PickerId>,
    depth: usize,
    selected: bool,
}

/// One occurrence's position and hierarchy depth within a rendered item segment.
struct ItemSegmentCell<'a> {
    item: ItemRef<'a>,
    run_index: usize,
    depth: usize,
    root_level: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum BrowserDragGhostSource {
    Group(PickerId),
    Item { group: usize, item: usize },
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

fn last_visible_descendant_bounds(node: &HierarchyNode, bounds: &std::collections::BTreeMap<PickerId, Rect>) -> Option<Rect> {
    if node.is_group() { node.children().iter().rev().find_map(|child| last_visible_descendant_bounds(child, bounds)) } else { bounds.get(node.id()).copied() }
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
    if source_group != target_group
        && let Some(entries) = orders.get(source_group)
    {
        let remaining = entries.iter().filter(|entry| *entry != &moving).cloned().collect();
        st.set_hierarchy_order(panel, source_group, remaining);
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
struct BrowserSink<'a> {
    state: &'a mut PresetUi,
    actions: Vec<BrowserAction>,
    pending_group_toggles: Vec<String>,
}

struct ItemDropSource<'a> {
    key: &'a str,
    node_id: Option<&'a PickerId>,
}

struct ItemDropCommit<'a, 'ctx> {
    ctx: &'a BrowserCtx<'ctx>,
    projection: &'a BrowserProjection,
    target: &'a PaintedCell<'ctx>,
    source: ItemDropSource<'a>,
    root_level_items: bool,
    after: bool,
}

struct CanonicalDropViewport {
    parent_clip: Rect,
    view_clip: Rect,
}

struct CanonicalDropCallbacks<'a, 'cb> {
    hierarchy_move: &'a mut Option<&'cb mut CanonicalHierarchyMove<'cb>>,
    rekey_after_move: &'a mut Option<&'cb mut RekeyHierarchyAfterMove<'cb>>,
    item_move: &'a mut Option<&'cb mut HierarchyItemMove<'cb>>,
}

struct HierarchyCommit<'a, 'cb> {
    state: &'a mut PresetUi,
    picker_ids: &'a mut PickerIdRegistry,
    root_item_group: Option<&'a str>,
    callbacks: CanonicalDropCallbacks<'a, 'cb>,
    viewport: CanonicalDropViewport,
}

struct RootHeaderReleaseEvent<'a> {
    group: &'a GroupView,
    header_rect: Rect,
    dragged: AssetDrag,
    browser_item_drop: &'a mut bool,
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

    fn select_node(&mut self, panel: &str, id: PickerId, toggle: bool) {
        self.state.select_browser_node(panel, id, toggle);
    }

    fn select_range(&mut self, panel: &str, order: &[PickerId], id: &PickerId) {
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

    fn emit(&mut self, action: BrowserAction) {
        self.actions.push(action);
    }

    fn commit_item_drop(&mut self, commit: ItemDropCommit<'_, '_>) {
        let ItemDropCommit { ctx, projection, target, source: ItemDropSource { key, node_id: source_node_id }, root_level_items, after } = commit;
        let panel = ctx.panel;
        let groups = ctx.groups;
        let hierarchy_orders = &projection.hierarchy_orders;
        let hierarchy_identities = &projection.hierarchy_identities;
        let domain_hierarchy_actions = ctx.domain_hierarchy_actions;
        let target_group_index = target.item.group_index;
        let target_item_index = target.item.item_index;
        let target_group = target.item.group;
        let target_item = target.item.item;

        let mut index = target_item_index + usize::from(after);
        if let Some((source_group, source_index)) = item_occurrence_position(groups, key)
            && source_group == target_group_index
            && source_index < index
        {
            index = index.saturating_sub(1);
        }
        if root_level_items {
            let source_identity = source_node_id.and_then(|id| hierarchy_identities.get(id)).cloned().or_else(|| {
                item_occurrence_position(groups, key).and_then(|(source_group_index, _)| {
                    groups.get(source_group_index).map(|source_group| PickerNodeIdentity::Item { group: source_group.name.clone(), key: key.to_owned() })
                })
            });
            if let Some(PickerNodeIdentity::Item { group: source_group, key: source_key }) = source_identity {
                let source_entry = HierarchyOrderEntry::RootItem { group: source_group.clone(), key: source_key.clone() };
                let target_entry = HierarchyOrderEntry::RootItem { group: target_group.name.clone(), key: target_item.key.clone() };
                let entries = hierarchy_orders.get("").cloned().unwrap_or_default();
                let before = entries
                    .iter()
                    .position(|entry| entry == &target_entry)
                    .and_then(|position| if after { entries.get(position + 1).cloned() } else { entries.get(position).cloned() });
                self.state.move_hierarchy_entry(panel, "", entries, source_entry, before.as_ref());
                select_dropped_item(self.state, panel, &source_group, &[], key);
            }
        } else if target_item.group_path.is_empty() {
            if !domain_hierarchy_actions
                && let Some((source_group_index, _)) = item_occurrence_position(groups, key)
                && let Some(source_group) = groups.get(source_group_index)
            {
                let entries = hierarchy_orders.get(&target_group.name).cloned().unwrap_or_default();
                let target = HierarchyOrderEntry::Item(target_item.key.clone());
                let before = entries
                    .iter()
                    .position(|entry| entry == &target)
                    .and_then(|position| if after { entries.get(position + 1).cloned() } else { entries.get(position).cloned() });
                move_item_sibling_order(self.state, panel, hierarchy_orders, &source_group.name, &target_group.name, key, before.as_ref());
            }
            select_dropped_item(self.state, panel, &target_group.name, &[], key);
            self.actions.push(BrowserAction::MoveItem(key.to_owned(), target_group.name.clone(), Some(index)));
        } else {
            let before = if after {
                target_group
                    .items
                    .iter()
                    .skip(target_item_index + 1)
                    .find(|candidate| candidate.group_path == target_item.group_path && candidate.key != key)
                    .map(|candidate| candidate.key.clone())
            } else {
                Some(target_item.key.clone())
            };
            select_dropped_item(self.state, panel, &target_group.name, &target_item.group_path, key);
            self.actions.push(BrowserAction::MoveItemToGroup {
                key: key.to_owned(),
                group: target_group.name.clone(),
                group_path: target_item.group_path.clone(),
                before,
            });
        }
    }

    /// Apply the group releases and folder toggles collected while rendering a segment.
    fn apply_group_releases_and_toggles(&mut self, batch: GroupReleaseBatch, ctx: &BrowserCtx<'_>, drag_state: &mut BrowserDragState) {
        let GroupReleaseBatch { group_releases, group_toggles } = batch;
        let BrowserCtx { groups, panel, domain_hierarchy_actions, .. } = *ctx;
        for release in group_releases {
            let GroupRelease { dragged, target_group, target_group_path, edge } = release;
            match dragged {
                AssetDrag::Preset { scope, key, .. } if scope == panel => {
                    let before = groups
                        .iter()
                        .find(|group| group.name == target_group)
                        .and_then(|group| group.items.iter().find(|item| item.group_path == target_group_path))
                        .map(|item| item.key.clone());
                    select_dropped_item(self.state, panel, &target_group, &target_group_path, &key);
                    self.actions.push(BrowserAction::MoveItemToGroup { key, group: target_group, group_path: target_group_path, before });
                    drag_state.browser_item_drop = true;
                }
                AssetDrag::Group { scope, name, path } if scope == panel && domain_hierarchy_actions => {
                    if let Some(action) = hierarchy_group_drop(&name, &path, &target_group, &target_group_path, edge) {
                        self.actions.push(hierarchy_action(action));
                    }
                }
                _ => {}
            }
        }
        for key in group_toggles {
            if !self.state.toggled.remove(&key) {
                self.state.toggled.insert(key);
            }
        }
    }

    /// Adapter context menus still have a legacy callback contract that mutates picker state and
    /// appends actions together. Keep that coupling confined to the synchronous menu invocation.
    fn with_context_menu<R>(&mut self, draw: impl FnOnce(&mut PresetUi, &mut Vec<BrowserAction>) -> R) -> R {
        draw(self.state, &mut self.actions)
    }

    fn into_actions(self) -> Vec<BrowserAction> {
        self.actions
    }
}

fn hierarchy_action(drop: HierarchyDrop) -> BrowserAction {
    match drop {
        HierarchyDrop::MoveGroup { group, group_path, before } => BrowserAction::MoveGroup { group, group_path, before },
        HierarchyDrop::MoveInto { group, group_path, into, before } => BrowserAction::MoveInto { group, group_path, into, before, before_item: None },
    }
}

/// Keep the domain-level selected preset in sync with item moves, regardless of which drop
/// target produced the move action. Browser multi-selection is separate transient UI state.
fn select_dropped_item(st: &mut PresetUi, panel: &str, group: &str, group_path: &[String], key: &str) {
    st.set_selected_item(panel, group, group_path, key);
}

/// Resolve the moved item's destination from the drop position, not its stable source identity.
/// Item identities deliberately retain their original group/path across hierarchy moves.
fn hierarchy_drop_item_destination(
    source: &PickerId,
    position: &DropPosition,
    hierarchy: &PickerBrowserHierarchy,
    identities: &std::collections::BTreeMap<PickerId, PickerNodeIdentity>,
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
        // Root-level items retain their original owning library group. The target of a root
        // insertion may belong to a different group, so it must never determine selection owner.
        ContainerId::Root => match identities.get(source)? {
            PickerNodeIdentity::Item { group, .. }
            | PickerNodeIdentity::FolderItem { group, .. }
            | PickerNodeIdentity::Group(group)
            | PickerNodeIdentity::Folder { group, .. } => Some((group.clone(), Vec::new())),
        },
    }
}

fn source_node_for_drag(drag: &AssetDrag, panel: &str, identities: &std::collections::BTreeMap<PickerId, PickerNodeIdentity>) -> Option<PickerId> {
    if let AssetDrag::CanonicalGroup { scope, node_id, .. } = drag {
        return (scope == panel
            && identities.get(node_id).is_some_and(|identity| matches!(identity.role(), PickerNodeRole::AssetGroup | PickerNodeRole::StructuralFolder)))
        .then(|| node_id.clone());
    }
    if let AssetDrag::Preset { scope, key, node_id, .. } = drag {
        return node_id
            .as_ref()
            .filter(|node_id| {
                scope == panel && matches!(identities.get(*node_id), Some(PickerNodeIdentity::Item { key: identity_key, .. } | PickerNodeIdentity::FolderItem { key: identity_key, .. }) if identity_key == key)
            })
            .cloned();
    }
    identities.iter().find_map(|(id, identity)| {
        let matches = match (drag, identity) {
            (AssetDrag::Group { scope, name, .. }, PickerNodeIdentity::Group(group)) => scope == panel && name == group,
            _ => false,
        };
        matches.then(|| id.clone())
    })
}

/// Return the selected visible nodes that should move as independent hierarchy roots.
/// Selecting both a group and one of its descendants moves the group subtree only.
fn selected_drag_nodes(state: &PresetUi, panel: &str, visible_order: &[PickerId], source: &PickerId, hierarchy: &PickerBrowserHierarchy) -> Vec<PickerId> {
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
    sources: &[PickerId],
    position: &DropPosition,
    identities: &std::collections::BTreeMap<PickerId, PickerNodeIdentity>,
    root_item_group: Option<&str>,
) -> Result<Vec<(String, String, String, usize)>, String> {
    let mut planned_hierarchy = hierarchy.clone();
    let mut item_moves = Vec::new();
    for source in hierarchy_move_sources(sources, position) {
        if let Some(PickerNodeIdentity::Item { group, key }) = identities.get(&source) {
            let resolved = planned_hierarchy.resolve_move(&source, position).map_err(|error| format!("Could not resolve item move: {error:?}"))?;
            let destination_group = match resolved.destination() {
                ContainerId::Group(destination_id) => {
                    let Some(PickerNodeIdentity::Group(destination_group)) = identities.get(destination_id) else {
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

fn item_node_for_identity(
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

fn picker_group_id(registry: &mut PickerIdRegistry, group_index: usize, keys: &PickerProjectionKeys) -> PickerId {
    registry.id_for(keys.key(&ProjectionNodeRef::Group(group_index)).expect("adapter supplies group identity").clone())
}

fn picker_folder_id(registry: &mut PickerIdRegistry, group_index: usize, keys: &PickerProjectionKeys, path: &[String]) -> PickerId {
    registry.id_for(keys.key(&ProjectionNodeRef::Folder { group_index, path: path.to_vec() }).expect("adapter supplies folder identity").clone())
}

fn folder_node_for_identity(group: &str, path: &[String], identities: &std::collections::BTreeMap<PickerId, PickerNodeIdentity>) -> Option<PickerId> {
    identities.iter().find_map(|(id, identity)| {
        matches!(identity, PickerNodeIdentity::Folder { group: owner, path: candidate } if owner == group && candidate == path).then(|| id.clone())
    })
}

fn group_is_descendant(parents: &std::collections::BTreeMap<String, String>, source: &str, target: &str) -> bool {
    is_group_descendant(parents, source, target)
}

fn hierarchy_drop_accepts(
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
struct BrowserProjection {
    display: Vec<(usize, usize)>,
    group_names: Vec<String>,
    parents: std::collections::BTreeMap<String, String>,
    hierarchy_orders: std::collections::BTreeMap<String, Vec<HierarchyOrderEntry>>,
    render_segments: Vec<GroupRenderSegment>,
    hierarchy: PickerBrowserHierarchy,
    hierarchy_identities: std::collections::BTreeMap<PickerId, PickerNodeIdentity>,
    selected_items: Vec<(String, ItemView)>,
    visible_selection_order: Vec<PickerId>,
}

/// Borrowed hierarchy data shared by hierarchy interaction helpers for one browser frame.
struct BrowserHierarchyView<'a> {
    groups: &'a [GroupView],
    group_names: &'a [String],
    parents: &'a std::collections::BTreeMap<String, String>,
    hierarchy: &'a PickerBrowserHierarchy,
    hierarchy_identities: &'a std::collections::BTreeMap<PickerId, PickerNodeIdentity>,
    hierarchy_orders: &'a std::collections::BTreeMap<String, Vec<HierarchyOrderEntry>>,
    visible_selection_order: &'a [PickerId],
}

struct ItemHierarchyDropPolicy<'a> {
    panel: &'a str,
    capabilities: BrowserCapabilities,
    domain_hierarchy_actions: bool,
}

struct ItemHierarchyDropResolution<'a> {
    indent_release: &'a mut Option<(String, String)>,
    browser_item_drop: &'a mut bool,
    released: &'a mut Option<(usize, Pos2)>,
}

/// Immutable adapter configuration shared by the browser's render and interaction helpers.
/// Mutable frame state, projection data, and callbacks deliberately remain separate.
#[derive(Clone, Copy)]
struct BrowserCtx<'a> {
    tokens: &'a Tokens,
    panel: &'a str,
    groups: &'a [GroupView],
    projection_keys: &'a PickerProjectionKeys,
    capabilities: BrowserCapabilities,
    domain_hierarchy_actions: bool,
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

fn draw_rename_row(ui: &mut egui::Ui, state: &mut PresetUi, panel: &str, tokens: &Tokens) -> (f32, Vec<BrowserAction>) {
    let row_top = ui.cursor().top();
    advance_item_rename(state, panel);
    let item_rename = state.item_renaming.clone().filter(|rename| rename.panel == panel);
    let nested_group_rename = state.group_renaming.clone().filter(|rename| rename.panel == panel);
    let has_rename_row = item_rename.is_some() || nested_group_rename.is_some() || state.renaming.as_ref().is_some_and(|r| r.0 == panel);
    let mut actions = Vec::new();

    if let Some(mut rename) = item_rename {
        if state.renaming.as_ref().is_some_and(|r| r.0 == panel) {
            state.renaming = None;
        }
        let mut text = rename.text.clone();
        let done = inline_name_editor(ui, &mut text, tokens);
        rename.text = text.clone();
        state.item_renaming = Some(rename.clone());
        if let Some(confirm) = done {
            state.item_renaming = None;
            if confirm && !text.trim().is_empty() {
                actions.push(BrowserAction::RenameItem { group: rename.group, group_path: rename.group_path, key: rename.key, text: text.trim().to_owned() });
            }
        }
    } else if let Some(mut rename) = nested_group_rename {
        // The typed nested-group target is authoritative while this editor is active. Remove any stale
        // legacy target for the same panel so it cannot reappear after commit or cancellation.
        if state.renaming.as_ref().is_some_and(|r| r.0 == panel) {
            state.renaming = None;
        }
        let mut text = rename.text.clone();
        let done = inline_name_editor(ui, &mut text, tokens);
        rename.text = text.clone();
        state.group_renaming = Some(rename.clone());
        if let Some(confirm) = done {
            state.group_renaming = None;
            if confirm && !text.trim().is_empty() {
                actions.push(BrowserAction::RenameNestedGroup { group: rename.group, group_path: rename.group_path, text: text.trim().to_owned() });
            }
        }
    } else if let Some((panel_name, key, mut text)) = state.renaming.clone().filter(|rename| rename.0 == panel) {
        // Starting a regular item/group rename supersedes any pending nested-group target for this panel.
        if state.group_renaming.as_ref().is_some_and(|rename| rename.panel == panel) {
            state.group_renaming = None;
        }
        let done = inline_name_editor(ui, &mut text, tokens);
        state.renaming = Some((panel_name, key.clone(), text.clone()));
        if let Some(confirm) = done {
            state.renaming = None;
            if confirm && !text.trim().is_empty() {
                let text = text.trim().to_owned();
                match key.strip_prefix("group:") {
                    Some(group) => actions.push(BrowserAction::RenameGroup(group.to_owned(), text)),
                    None => actions.push(BrowserAction::Rename(key, text)),
                }
            }
        }
    }

    let height = if has_rename_row { (ui.cursor().top() - row_top).max(0.0) } else { 0.0 };
    (height, actions)
}

/// Shared focus and confirmation behavior for all inline rename targets.
fn inline_name_editor(ui: &mut egui::Ui, text: &mut String, tokens: &Tokens) -> Option<bool> {
    let mut done = None;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Name:")).color(tokens.text_dim).size(11.5));
        let response = ui.add(egui::TextEdit::singleline(text).desired_width(ui.available_width() - 4.0));
        if !response.has_focus() && !response.lost_focus() {
            response.request_focus();
        }
        if response.lost_focus() {
            done = Some(ui.input(|input| input.key_pressed(egui::Key::Enter)));
        }
    });
    done
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

use crate::pickers::asset_browser_groups::{header as asset_browser_header, inset_row_rect, paint as paint_asset_browser_header};

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
struct Painters<'borrow, 'callback> {
    group: Option<&'borrow mut PaintGroup>,
    item: &'borrow mut PaintItem<'callback>,
    list_item: Option<&'borrow mut PaintListItem<'callback>>,
}

impl Painters<'_, '_> {
    fn has_group(&self) -> bool {
        self.group.is_some()
    }

    fn has_list_item(&self) -> bool {
        self.list_item.is_some()
    }

    fn group(&mut self, ui: &egui::Ui, rect: Rect, group: &GroupView, open: bool, hovered: bool, opacity: f32) {
        if let Some(paint) = self.group.as_deref_mut() {
            paint(ui, rect, group, open, hovered, opacity);
        }
    }

    fn item(&mut self, ui: &egui::Ui, rect: Rect, group_index: usize, item_index: usize, state: ItemPaintState) {
        (self.item)(ui, rect, group_index, item_index, state);
    }

    fn list_item(&mut self, ui: &egui::Ui, rect: Rect, group_index: usize, item_index: usize, selected: bool) {
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
type CanonicalHierarchyMove<'a> = dyn FnMut(&DropMoveIntent, &std::collections::BTreeMap<PickerId, PickerNodeIdentity>) -> Result<(), String> + 'a;
type RekeyHierarchyAfterMove<'a> = dyn FnMut(&PickerBrowserHierarchy, &std::collections::BTreeMap<PickerId, PickerNodeIdentity>, &mut PickerIdRegistry) + 'a;
type HierarchyItemMove<'a> = dyn FnMut(&str, &str, &str, usize) -> Result<(), String> + 'a;
struct ItemHierarchyTarget {
    response: egui::Response,
    rect: Rect,
    grid: bool,
    accepts: bool,
    item_index: usize,
    group_name: String,
    group_path: Vec<String>,
}

/// Mutable drag/drop data collected during layout and consumed in the frame's ordered resolution tail.
#[derive(Default)]
struct BrowserDragState {
    headers: Vec<(usize, Rect, String)>,
    group_content_areas: Vec<(String, Rect)>,
    indent_drop_targets: Vec<(String, egui::Response)>,
    item_hierarchy_target_wins: bool,
    indent_release: Option<(String, String)>,
    dropped: Option<(String, Pos2)>,
    browser_item_drop: bool,
    group_drag: Option<(usize, Pos2)>,
    group_release: Option<(usize, Pos2)>,
    group_releases: Vec<GroupRelease>,
    ghosts: BrowserDragGhosts,
    canonical_drop: CanonicalDropState,
    item_hierarchy_targets: Vec<ItemHierarchyTarget>,
    item_insertion_markers: Vec<(Rect, bool, bool)>,
}

/// Canonical hierarchy-drop geometry and hover state shared by target registration.
#[derive(Default)]
struct CanonicalDropState {
    gap_headers: Vec<(PickerId, Rect, usize, bool)>,
    node_bounds: std::cell::RefCell<std::collections::BTreeMap<PickerId, Rect>>,
    hits: std::cell::RefCell<Vec<(DropHitRegion, egui::Response)>>,
    non_root_target_hovered: std::cell::Cell<bool>,
}

struct ItemDropRegistrationGeometry<'borrow, 'items> {
    painted: &'borrow PaintedCell<'items>,
    grid: ItemGrid,
    run_index: usize,
}

/// Drag previews are collected during layout and painted together outside the scroll viewport.
#[derive(Default)]
struct BrowserDragGhosts {
    groups: Vec<GroupDragGhost>,
    nested_groups: Vec<NestedGroupDragGhost>,
    items: Vec<ItemDragGhost>,
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

struct BrowserDeleteContext<'a> {
    selection_order: &'a [PickerId],
    hierarchy: &'a PickerBrowserHierarchy,
    identities: &'a std::collections::BTreeMap<PickerId, PickerNodeIdentity>,
}

/// Resolve a selected browser node set to independent domain deletions. Deleting a selected group
/// already deletes its subtree, so selected descendants must not produce duplicate operations.
fn selected_delete_actions(selection: &BTreeSet<PickerId>, clicked: &PickerId, context: &BrowserDeleteContext<'_>) -> Option<Vec<BrowserAction>> {
    if !selection.contains(clicked) {
        return None;
    }
    let mut actions = Vec::new();
    for id in context.selection_order.iter().filter(|id| selection.contains(*id)) {
        let mut parent = context.hierarchy.parent_of(id);
        let mut covered = false;
        while let Some(ContainerId::Group(parent_id)) = parent {
            if selection.contains(&parent_id) {
                covered = true;
                break;
            }
            parent = context.hierarchy.parent_of(&parent_id);
        }
        if covered {
            continue;
        }
        match context.identities.get(id) {
            Some(PickerNodeIdentity::Group(group)) => actions.push(BrowserAction::DeleteGroup(group.clone())),
            Some(PickerNodeIdentity::Folder { group, path }) => {
                actions.push(BrowserAction::DeleteNestedGroup { group: group.clone(), group_path: path.clone() })
            }
            Some(PickerNodeIdentity::Item { group, key }) => {
                actions.push(BrowserAction::DeleteItem { group: group.clone(), group_path: Vec::new(), key: key.clone() })
            }
            Some(PickerNodeIdentity::FolderItem { group, path, key }) => {
                actions.push(BrowserAction::DeleteItem { group: group.clone(), group_path: path.clone(), key: key.clone() });
            }
            None => {}
        }
    }
    (!actions.is_empty()).then_some(actions)
}

fn replace_selected_delete_actions(
    events: &mut Vec<BrowserAction>,
    start: usize,
    state: &PresetUi,
    panel: &str,
    clicked: &PickerId,
    context: &BrowserDeleteContext<'_>,
) {
    if !events.get(start..).is_some_and(|new_events| {
        new_events
            .iter()
            .any(|action| matches!(action, BrowserAction::DeleteItem { .. } | BrowserAction::DeleteGroup(_) | BrowserAction::DeleteNestedGroup { .. }))
    }) {
        return;
    }
    let Some(selection) = state.browser_selection.get(panel) else { return };
    if let Some(actions) = selected_delete_actions(selection, clicked, context) {
        events.truncate(start);
        events.extend(actions);
    }
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
pub(crate) fn standard_item_context_menu(ui: &mut egui::Ui, st: &mut PresetUi, args: ItemContextMenuArgs<'_>, ev: &mut Vec<BrowserAction>) {
    let ItemContextMenuArgs { panel, item, group, group_names, selected_items } = args;
    let targets = item_context_targets(item, group, selected_items);
    let count = targets.len();
    let delete_label = if count > 1 { format!("{} ({count})", tl!("Delete")) } else { tl!("Delete").to_string() };
    let all_in_current_group = targets.iter().all(|(source_group, _)| source_group == &group.name);
    match draw_item_context_menu(
        ui,
        tl!(
            "RenameÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¦"
        ),
        &delete_label,
        Some((tl!("Move to"), group_names, if all_in_current_group { &group.name } else { "" })),
    ) {
        Some(ContextMenuAction::Rename) => begin_item_rename(st, panel, &targets),
        Some(ContextMenuAction::Delete) => ev.extend(delete_item_actions(&targets)),
        Some(ContextMenuAction::MoveTo(destination)) => ev.extend(move_item_actions(&targets, &destination)),
        None => {}
    }
}

/// Apply the shared rename/delete policy for a standard browser group context menu.
pub(crate) fn standard_group_context_menu(ui: &mut egui::Ui, st: &mut PresetUi, panel: &str, group: &GroupView, ev: &mut Vec<BrowserAction>) {
    match draw_group_context_menu(
        ui,
        tl!(
            "Rename GroupÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¦"
        ),
        tl!("Delete Group"),
    ) {
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

/// Return the visual grid coordinate for an item in row-major thumbnails or column-major wrapped lists.
fn item_grid_position(index: usize, rows: usize, cols: usize, column_major: bool) -> (f32, f32) {
    if column_major {
        let rows = rows.max(1);
        ((index / rows) as f32, (index % rows) as f32)
    } else {
        ((index % cols.max(1)) as f32, (index / cols.max(1)) as f32)
    }
}
fn list_layout_geometry(available_width: f32, indent: f32, height: f32, wrap: bool, fixed_width: Option<f32>, item_count: usize) -> (egui::Vec2, usize, usize) {
    let content_width = (available_width - indent).max(0.0);
    // `available_width` is already constrained by the ScrollArea viewport, so its
    // scrollbar gutter is preserved. The row origin is inset separately; subtract
    // that same inset here so the row still ends at the header's right edge.
    let width = if wrap { fixed_width.map(|width| width.min(content_width)).unwrap_or(content_width) } else { content_width };
    let columns = if wrap { (content_width / width.max(1.0)).floor().max(1.0) as usize } else { 1 };
    let rows = item_count.div_ceil(columns);
    (vec2(width, height), columns, rows)
}

/// Canvas application drops must not steal releases that land anywhere in the browser viewport.
fn canvas_drop_target(pointer: Pos2, item: Rect, canvas: Rect, browser: Rect) -> bool {
    !item.contains(pointer) && !browser.contains(pointer) && canvas.contains(pointer)
}
struct GroupRelease {
    dragged: AssetDrag,
    target_group: String,
    target_group_path: Vec<String>,
    edge: bool,
}

struct GroupDragGhost {
    rect: Rect,
    group_index: usize,
    open: bool,
    depth: usize,
}

struct NestedGroupDragGhost {
    rect: Rect,
    open: bool,
    name: String,
    depth: usize,
    count: usize,
}

struct ItemDragGhost {
    rect: Rect,
    group_index: usize,
    item_index: usize,
    display_mode: crate::pickers::picker_state::PickerDisplayMode,
    selected: bool,
}

impl Painters<'_, '_> {
    fn paint_drag_ghosts(&mut self, ui: &egui::Ui, panel: &str, groups: &[GroupView], app_window_clip: Rect, ghosts: BrowserDragGhosts, tokens: &Tokens) {
        for ghost in ghosts.groups {
            let GroupDragGhost { rect: ghost_rect, group_index, open, depth } = ghost;
            let Some(group) = groups.get(group_index) else { continue };
            let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new(("preset-group-drag", panel)));
            let mut ghost_ui = egui::Ui::new(
                ui.ctx().clone(),
                egui::Id::new(("preset-group-drag-ui", panel)),
                egui::UiBuilder::new().layer_id(layer).max_rect(ghost_rect.expand(1.0)),
            );
            ghost_ui.set_clip_rect(app_window_clip);
            if self.has_group() {
                self.group(&ghost_ui, ghost_rect, group, open, false, GROUP_DRAG_GHOST_OPACITY);
            } else {
                crate::pickers::asset_browser_groups::paint_with_opacity(&ghost_ui, ghost_rect, open, &group.name, depth, None, GROUP_DRAG_GHOST_OPACITY);
            }
        }
        for ghost in ghosts.nested_groups {
            let NestedGroupDragGhost { rect: ghost_rect, open, name, depth, count } = ghost;
            let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new(("preset-group-drag", panel)));
            let mut ghost_ui = egui::Ui::new(
                ui.ctx().clone(),
                egui::Id::new(("preset-group-drag-ui", panel)),
                egui::UiBuilder::new().layer_id(layer).max_rect(ghost_rect.expand(1.0)),
            );
            ghost_ui.set_clip_rect(app_window_clip);
            crate::pickers::asset_browser_groups::paint_with_opacity(&ghost_ui, ghost_rect, open, &name, depth, Some(count), GROUP_DRAG_GHOST_OPACITY);
        }
        for ghost in ghosts.items {
            let ItemDragGhost { rect: ghost_rect, group_index, item_index, display_mode: ghost_mode, selected } = ghost;
            let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new(("preset-drag", panel)));
            let mut ghost_ui = egui::Ui::new(
                ui.ctx().clone(),
                egui::Id::new(("preset-drag-ui", panel)),
                egui::UiBuilder::new().layer_id(layer).max_rect(ghost_rect.expand(1.0)),
            );
            ghost_ui.set_clip_rect(app_window_clip);
            ghost_ui.set_opacity(0.4);
            if ghost_mode == crate::pickers::picker_state::PickerDisplayMode::TextOnly {
                if let Some(item) = groups.get(group_index).and_then(|group| group.items.get(item_index)) {
                    if selected {
                        ghost_ui.painter().rect_filled(ghost_rect, CornerRadius::ZERO, tokens.row_selected);
                    }
                    let label_rect = Rect::from_min_max(pos2(ghost_rect.left() + 8.0, ghost_rect.top()), ghost_rect.right_bottom());
                    let mut label_ui = ghost_ui.new_child(egui::UiBuilder::new().max_rect(label_rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
                    label_ui.set_clip_rect(label_rect.intersect(app_window_clip));
                    crate::widgets::elided_label(&mut label_ui, &item.name, label_rect.width());
                }
            } else if ghost_mode.uses_list_layout() {
                if self.has_list_item() {
                    self.list_item(&ghost_ui, ghost_rect, group_index, item_index, selected);
                } else if let Some(item) = groups.get(group_index).and_then(|group| group.items.get(item_index)) {
                    let thumb_size = (ghost_rect.height() - 2.0).max(0.0);
                    let thumb_rect = Rect::from_min_size(ghost_rect.min + vec2(1.0, 1.0), vec2(thumb_size, thumb_size));
                    self.item(&ghost_ui, thumb_rect, group_index, item_index, ItemPaintState::DragGhost);
                    ghost_ui.painter().text(
                        pos2(thumb_rect.right() + 8.0, ghost_rect.center().y),
                        Align2::LEFT_CENTER,
                        &item.name,
                        FontId::proportional(11.5),
                        tokens.text,
                    );
                }
            } else {
                self.item(&ghost_ui, ghost_rect, group_index, item_index, ItemPaintState::DragGhost);
            }
            ghost_ui.painter().rect_stroke(ghost_rect, CornerRadius::ZERO, Stroke::new(2.0, tokens.accent), egui::StrokeKind::Middle);
        }
    }
}

/// Register canonical insertion targets at sibling boundaries involving group headers.
///
/// These targets are resolved only after every visible node has its final layout bounds.
fn register_canonical_sibling_gap_targets(ui: &mut egui::Ui, ctx: &BrowserCtx<'_>, projection: &BrowserProjection, drag: &BrowserDragState) {
    let panel = ctx.panel;
    let hierarchy = &projection.hierarchy;
    let canonical_node_bounds = &drag.canonical_drop.node_bounds;
    let canonical_drop_hits = &drag.canonical_drop.hits;
    let gap_headers = &drag.canonical_drop.gap_headers;
    for pair in gap_headers.windows(2) {
        let (upper_id, upper_rect, _upper_depth, upper_open) = &pair[0];
        let (lower_id, lower_rect, lower_depth, _) = &pair[1];
        let gap_rect = Rect::from_min_max(
            // A parent/child boundary spans two indentation levels. Keep the hit
            // target reachable from the parent's gutter as well as the child's row;
            // otherwise dragging at the visual boundary between them misses the gap.
            pos2(upper_rect.left().min(lower_rect.left()), lower_rect.top() - GROUP_GAP_DROP_HEIGHT),
            pos2(lower_rect.right(), lower_rect.top()),
        );
        if gap_rect.is_positive() {
            let response = ui.interact(gap_rect, ui.id().with(("group-gap", panel, upper_id, lower_id)), Sense::hover());
            let upper_anchor = group_gap_upper_anchor(hierarchy, lower_id);
            let upper_parent = hierarchy.parent_of(lower_id).unwrap_or(ContainerId::Root);
            let anchor_is_parent = upper_anchor.as_ref().is_some_and(|(anchor, _)| upper_parent == ContainerId::Group(anchor.clone()));
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
                .unwrap_or(*upper_rect);
            // An expanded group's final card can occupy the same row as the next
            // group's insertion band. Restrict that band to the trailing whitespace
            // after the visible subtree, so a drop on a real card remains card-relative.
            let upper_content_right = if *upper_open && !anchor_is_parent {
                upper_anchor
                    .as_ref()
                    .and_then(|(anchor, _)| hierarchy.node(anchor))
                    .and_then(|node| last_visible_descendant_bounds(node, &canonical_node_bounds.borrow()))
                    .map_or(gap_rect.left(), |bounds| bounds.right())
            } else {
                gap_rect.left()
            };
            let upper_target = upper_anchor.as_ref().map(|(anchor, _)| anchor.clone()).unwrap_or_else(|| upper_id.clone());
            let lower_bounds = canonical_node_bounds.borrow().get(lower_id).copied().unwrap_or(*lower_rect);
            let mut hits = group_gap_drop_hits(GroupGapDropSpec {
                rect: gap_rect,
                upper: upper_target,
                lower: lower_id.clone(),
                upper_bounds,
                lower_bounds,
                upper_depth: upper_anchor.as_ref().map(|(_, depth)| *depth).unwrap_or(parent_depth),
                lower_depth: *lower_depth,
                order: canonical_drop_hits.borrow().len(),
            });
            if anchor_is_parent {
                // The parent indentation gutter is visually part of the upper
                // container, not the lower child's leading-edge column. Route the
                // whole gutter strip to the parent-append half; retain the ordinary
                // vertical split to the right of the child's indentation.
                let gutter_right = lower_rect.left().min(gap_rect.right());
                hits[0].hit_rect = Rect::from_min_max(gap_rect.min, pos2(gutter_right, gap_rect.bottom()));
                hits[1].hit_rect.min.x = gutter_right;
                // Both sibling-level markers meet at the lower header's top border.
                // The gap is a hit affordance above that boundary, not the marker's
                // vertical placement.
                for hit in &mut hits {
                    hit.indicator_rect =
                        Rect::from_min_max(pos2(hit.indicator_rect.left(), lower_rect.top() - 2.0), pos2(hit.indicator_rect.right(), lower_rect.top() + 2.0));
                }
                hits[0].order = usize::MAX;
            }
            for hit in &mut hits {
                hit.hit_rect.min.x = hit.hit_rect.min.x.max(upper_content_right);
                // The trailing blank strip of the final card is also a row-append
                // target. In the whitespace after the whole group, the group boundary
                // is the more specific structural destination and must win that tie.
                hit.depth = usize::MAX - 1;
            }
            if anchor_is_parent {
                // Apply this after the common structural-boundary priority above:
                // the parent's gutter append must outrank the overlapping subtree
                // append target, not be demoted by the generic gap-hit assignment.
                hits[0].depth = usize::MAX;
            }
            // This is the upper half of a group boundary, so its feedback is the
            // horizontal sibling line at the lower group's top edge even when the
            // preceding sibling is a card. Card-edge markers are for card-to-card
            // insertion, not for this group boundary.
            // If the lower group is first in its sibling sequence, append to its parent.
            // Otherwise insert after the actual preceding sibling (card or group).
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
    fn collect_group_item_sibling_pairs(nodes: &[HierarchyNode], out: &mut Vec<(PickerId, PickerId)>) {
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
        fn collect_group_nodes(node: &HierarchyNode, out: &mut Vec<PickerId>) {
            out.push(node.id().clone());
            for child in node.children() {
                collect_group_nodes(child, out);
            }
        }
        let Some(group_node) = hierarchy.node(&group_id) else {
            continue;
        };
        collect_group_nodes(group_node, &mut subtree_nodes);
        let subtree_bounds: Vec<(PickerId, Rect)> =
            subtree_nodes.iter().filter_map(|id| canonical_node_bounds.borrow().get(id).copied().map(|rect| (id.clone(), rect))).collect();
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
        let append_marker = Rect::from_min_max(pos2(group_header.left(), marker_y - 2.0), pos2(group_header.right(), marker_y + 2.0));
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
    // A root card (or nested item) can be the immediate preceding sibling of a
    // group. Header-only gap pairing above cannot see that boundary when there is
    // no earlier group header, so register the same two-sided group boundary from
    // the actual hierarchy sibling sequence.
    fn collect_item_group_sibling_pairs(nodes: &[HierarchyNode], out: &mut Vec<(PickerId, PickerId)>) {
        for pair in nodes.windows(2) {
            if matches!(pair, [HierarchyNode::Item(_), HierarchyNode::Group { .. }]) {
                out.push((pair[0].id().clone(), pair[1].id().clone()));
            }
        }
        for node in nodes {
            if node.is_group() {
                collect_item_group_sibling_pairs(node.children(), out);
            }
        }
    }
    let mut item_group_pairs = Vec::new();
    collect_item_group_sibling_pairs(hierarchy.roots(), &mut item_group_pairs);
    for (item_id, group_id) in item_group_pairs {
        let Some(item_bounds) = canonical_node_bounds.borrow().get(&item_id).copied() else {
            continue;
        };
        let Some(group_bounds) = canonical_node_bounds.borrow().get(&group_id).copied() else {
            continue;
        };
        let gap = Rect::from_min_max(
            pos2(item_bounds.left().min(group_bounds.left()), group_bounds.top() - GROUP_GAP_DROP_HEIGHT),
            pos2(group_bounds.right(), group_bounds.top()),
        );
        if !gap.is_positive() {
            continue;
        }
        let response = ui.interact(gap, ui.id().with(("group-card-sibling-gap", panel, &item_id, &group_id)), Sense::hover());
        let order = canonical_drop_hits.borrow().len();
        let mut hits = group_gap_drop_hits(GroupGapDropSpec {
            rect: gap,
            upper: item_id.clone(),
            lower: group_id,
            upper_bounds: item_bounds,
            lower_bounds: group_bounds,
            upper_depth: 0,
            lower_depth: 0,
            order,
        });
        // Keep the two halves mutually exclusive: upper appends after the card
        // (its right edge); lower inserts before the group (its upper boundary).
        hits[0].indicator_rect = card_trailing_marker_rect(item_bounds, true);
        // This structural boundary is more specific than the overlapping card
        // trailing edge and group leading edge targets.
        hits[0].depth = usize::MAX;
        hits[1].depth = usize::MAX;
        for hit in hits {
            canonical_drop_hits.borrow_mut().push((hit, response.clone()));
        }
    }
}

/// Resolve the canonical hierarchy target and commit a validated release. The returned `None`
/// means no release was consumed; `Some(Ok(()))` clears old feedback and `Some(Err(_))` reports a
/// rejected or failed move. Keeping the one-shot payload handling here makes target arbitration,
/// source verification and domain persistence one ordered operation.
fn resolve_canonical_drop(
    ui: &egui::Ui,
    ctx: &BrowserCtx<'_>,
    projection: &mut BrowserProjection,
    drag_state: &BrowserDragState,
    commit: HierarchyCommit<'_, '_>,
    accent: egui::Color32,
) -> Option<Result<(), String>> {
    let panel = ctx.panel;
    let hierarchy = &mut projection.hierarchy;
    let identities = &projection.hierarchy_identities;
    let drop_hits = drag_state.canonical_drop.hits.borrow();
    let pointer = ui.ctx().pointer_interact_pos()?;
    let hits: Vec<DropHitRegion> = drop_hits
        .iter()
        .filter_map(|(hit, _)| {
            let mut visible = hit.clone();
            // Header visuals extend into the ScrollArea's left margin. Use the parent's horizontal
            // clip for the root-leading target, and the scroll viewport for every other region.
            let clip = if visible.depth == usize::MAX && visible.order == usize::MAX && visible.region == PointerRegion::LeadingEdge {
                commit.viewport.parent_clip
            } else {
                commit.viewport.view_clip
            };
            visible.hit_rect = visible.hit_rect.intersect(clip);
            visible.hit_rect.is_positive().then_some(visible)
        })
        .collect();
    let index = arbitrate_hit_index(pointer, &hits)?;
    drop_hits.get(index)?;

    let dragged = egui::DragAndDrop::payload::<AssetDrag>(ui.ctx()).map(|payload| payload.as_ref().clone())?;
    let source = source_node_for_drag(&dragged, panel, identities)?;
    let selection = match &dragged {
        AssetDrag::Preset { selection, .. } | AssetDrag::CanonicalGroup { selection, .. } => selection.clone(),
        AssetDrag::Group { .. } => Vec::new(),
    };
    let mut session = DragSession::new(source.clone(), selection.clone());
    let candidate = session.resolve(pointer, &hits[index], hierarchy).cloned()?;
    let ghost_layer_id = match dragged {
        AssetDrag::Preset { .. } => ("preset-drag", panel),
        AssetDrag::Group { .. } | AssetDrag::CanonicalGroup { .. } => ("preset-group-drag", panel),
    };
    let drop_feedback_layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new(ghost_layer_id));
    paint_candidate(ui, &candidate, accent, commit.viewport.view_clip, drop_feedback_layer);

    // Resolve and validate the exact candidate before taking egui's global one-shot payload.
    let released = asset_browser_take_release(ui.ctx())?;
    if source_node_for_drag(&released, panel, identities).as_ref() != Some(&source) {
        return Some(Err("Move rejected: the dragged source changed before release.".to_owned()));
    }

    let moved_sources = {
        let sources = if selection.is_empty() { vec![source.clone()] } else { selection.clone() };
        hierarchy_move_sources(&sources, &candidate.position)
    };
    let result = session.commit_with(|intent| {
        if let Some(commit_move) = commit.callbacks.hierarchy_move.as_deref_mut() {
            let mut moved = hierarchy.clone();
            let sources = if intent.selection.is_empty() { vec![intent.source.clone()] } else { intent.selection.clone() };
            let moved_ok = hierarchy_move_sources(&sources, &intent.position)
                .into_iter()
                .try_for_each(|source| {
                    let resolved = moved.resolve_move(&source, &intent.position)?;
                    moved.apply_resolved_move(resolved)
                })
                .is_ok();
            let result = commit_move(intent, identities);
            if result.is_ok()
                && moved_ok
                && let Some(rekey) = commit.callbacks.rekey_after_move.as_deref_mut()
            {
                rekey(&moved, identities, commit.picker_ids);
            }
            return result;
        }
        let sources = if intent.selection.is_empty() { vec![intent.source.clone()] } else { intent.selection.clone() };
        let hierarchy_sources = hierarchy_move_sources(&sources, &intent.position);
        let legacy_persistable = identities.values().all(|identity| matches!(identity.role(), PickerNodeRole::Card | PickerNodeRole::AssetGroup));
        if !legacy_persistable {
            return Err("This asset adapter does not support path-aware hierarchy moves.".to_owned());
        }
        let item_moves = plan_hierarchy_item_moves(hierarchy, &sources, &intent.position, identities, commit.root_item_group)?;
        // Preflight the complete hierarchy and preference update on private copies before any
        // adapter command is dispatched. If any later source is invalid, neither the domain nor
        // the live browser hierarchy has been touched.
        let mut staged_hierarchy = hierarchy.clone();
        let mut staged_state = commit.state.clone();
        for source in &hierarchy_sources {
            commit_hierarchy_move(&mut staged_state, panel, &mut staged_hierarchy, source, &intent.position, identities)
                .map_err(|error| format!("Could not persist hierarchy move: {error:?}"))?;
        }
        if !item_moves.is_empty() {
            let Some(commit_item) = commit.callbacks.item_move.as_deref_mut() else {
                return Err("This asset adapter does not support authoritative item moves.".to_owned());
            };
            for (source_group, key, destination_group, index) in item_moves {
                commit_item(&source_group, &key, &destination_group, index)?;
            }
        }
        *hierarchy = staged_hierarchy;
        *commit.state = staged_state;
        Ok(())
    });

    if matches!(result, DropCommitResult::Committed) {
        if moved_sources.contains(&source)
            && let Some(PickerNodeIdentity::Item { key, .. } | PickerNodeIdentity::FolderItem { key, .. }) = identities.get(&source)
            && let Some((group, path)) = hierarchy_drop_item_destination(&source, &candidate.position, hierarchy, identities)
        {
            select_dropped_item(commit.state, panel, &group, &path, key);
        }
        if selection.is_empty()
            && identities.get(&source).is_some_and(|identity| matches!(identity.role(), PickerNodeRole::AssetGroup | PickerNodeRole::StructuralFolder))
        {
            commit.state.select_browser_node(panel, source, false);
        }
    }

    Some(match result {
        DropCommitResult::Committed | DropCommitResult::NoCandidate => Ok(()),
        DropCommitResult::Rejected(reason) => Err(format!("Move rejected: {reason:?}")),
        DropCommitResult::Failed(error) => Err(format!("Move rejected: {error:?}")),
    })
}

/// Apply the legacy group-header release after all visible headers and indentation columns have
/// been collected. Canonical mode deliberately never enters this path.
fn apply_legacy_group_release(state: &mut PresetUi, ctx: &BrowserCtx<'_>, projection: &BrowserProjection, drag_state: &BrowserDragState) -> Vec<BrowserAction> {
    let BrowserCtx { panel, groups, domain_hierarchy_actions, .. } = *ctx;
    let BrowserProjection { parents, group_names, hierarchy_orders, .. } = projection;
    let BrowserDragState { headers, indent_release, group_release: released, .. } = drag_state;
    let mut actions = Vec::new();
    if let Some((from_name, parent)) = indent_release.clone() {
        if !group_is_descendant(parents, &from_name, &parent) {
            if let Some(old_parent) = parents.get(&from_name).filter(|old_parent| *old_parent != &parent) {
                state.remove_hierarchy_entry(panel, old_parent, &HierarchyOrderEntry::Group(from_name.clone()));
            }
            state.nest_group(panel, group_names, &from_name, Some(&parent));
            let entries = hierarchy_orders.get(&parent).cloned().unwrap_or_default();
            state.move_hierarchy_entry(panel, &parent, entries, HierarchyOrderEntry::Group(from_name.clone()), None);
        }
    } else if let Some((from, pointer)) = *released {
        let Some(from_name) = groups.get(from).map(|group| group.name.clone()) else { return actions };
        if let Some((_, rect, target)) = headers.iter().find(|(index, rect, _)| *index != from && rect.contains(pointer)) {
            let edge = header_drop_before(*rect, pointer);
            if domain_hierarchy_actions && !group_is_descendant(parents, &from_name, target) {
                if let Some(action) = hierarchy_group_drop(&from_name, &[], target, &[], edge) {
                    actions.push(hierarchy_action(action));
                }
            } else if !domain_hierarchy_actions {
                let destination_parent = if edge { parents.get(target).cloned() } else { Some(target.clone()) };
                let creates_cycle = destination_parent.as_deref().is_some_and(|parent| parent == from_name || group_is_descendant(parents, &from_name, parent));
                if !creates_cycle {
                    if let Some(old_parent) = parents.get(&from_name).filter(|old_parent| Some(*old_parent) != destination_parent.as_ref()) {
                        state.remove_hierarchy_entry(panel, old_parent, &HierarchyOrderEntry::Group(from_name.clone()));
                    }
                    state.nest_group(panel, group_names, &from_name, destination_parent.as_deref());
                    if let Some(parent) = destination_parent {
                        let entries = hierarchy_orders.get(&parent).cloned().unwrap_or_default();
                        let before = edge.then(|| HierarchyOrderEntry::Group(target.clone()));
                        state.move_hierarchy_entry(panel, &parent, entries, HierarchyOrderEntry::Group(from_name), before.as_ref());
                    } else {
                        state.move_group(panel, group_names, &from_name, Some(target));
                    }
                }
            }
        } else {
            let before = group_drop_before(headers, from, &from_name, pointer, parents);
            if before != Some(from) {
                let before_name = before.and_then(|index| groups.get(index).map(|group| group.name.clone()));
                if domain_hierarchy_actions {
                    actions.push(BrowserAction::MoveGroup { group: from_name, group_path: Vec::new(), before: before_name });
                } else {
                    let source_parent = parents.get(&from_name).cloned();
                    let source_rect = headers.iter().find(|(index, _, _)| *index == from).map(|(_, rect, _)| *rect);
                    let promote_to_root = source_parent.is_some() && source_rect.is_some_and(|rect| pointer.x < rect.left() + ROW_INDENT * 0.5);
                    if let Some(parent) = source_parent.clone().filter(|_| !promote_to_root) {
                        let entries = hierarchy_orders.get(&parent).cloned().unwrap_or_default();
                        let before = before_name.as_ref().map(|name| HierarchyOrderEntry::Group(name.clone()));
                        state.move_hierarchy_entry(panel, &parent, entries, HierarchyOrderEntry::Group(from_name), before.as_ref());
                    } else {
                        if let Some(old_parent) = source_parent.as_deref() {
                            state.remove_hierarchy_entry(panel, old_parent, &HierarchyOrderEntry::Group(from_name.clone()));
                        }
                        state.nest_group(panel, group_names, &from_name, None);
                        state.move_group(panel, group_names, &from_name, before_name.as_deref());
                    }
                }
            }
        }
    }
    actions
}

/// Draw only the reusable library region. It owns groups/items, selection, drag/drop and item/group
/// context targets, but not the containing panel's optional controls or footer.
///
/// `allocated_height` is reserved by the caller or picker shell; the browser owns only the
/// scrolling library region, not search, recents, panel menus, footer controls, or command dispatch.
pub(crate) fn browser(ui: &mut egui::Ui, st: &mut PresetUi, adapter: &mut BrowserAdapter<'_>, allocated_height: f32) -> Vec<BrowserAction> {
    browser_with_root_sequence(ui, st, adapter, allocated_height, None)
}

/// Resolve item-level hierarchy drops before indentation-column arbitration.
fn resolve_item_hierarchy_drop_target(
    ui: &mut egui::Ui,
    item_hierarchy_targets: &[ItemHierarchyTarget],
    hierarchy: &BrowserHierarchyView<'_>,
    policy: ItemHierarchyDropPolicy<'_>,
    sink: &mut BrowserSink<'_>,
    resolution: ItemHierarchyDropResolution<'_>,
) -> bool {
    let BrowserHierarchyView { groups, group_names, parents, hierarchy_orders, .. } = hierarchy;
    let ItemHierarchyDropPolicy { panel, capabilities, domain_hierarchy_actions } = policy;
    let ItemHierarchyDropResolution { indent_release, browser_item_drop, released } = resolution;
    // Match the existing indentation-column arbitration: the last eligible
    // (topmost/deepest) hit target wins both its preview and its release.
    let best_item_hierarchy_target = best_drop_target_index(&item_hierarchy_targets, |target| {
        target.accepts
            && target
                .response
                .dnd_hover_payload::<AssetDrag>()
                .is_some_and(|payload| matches!(payload.as_ref(), AssetDrag::Group { scope, .. } if scope == panel))
    });
    // The indentation column overlaps the leading edge of nested cards. Treat these as
    // competing destinations, not independent drop zones: when a card wins, its vertical
    // insertion marker and release must suppress the column's horizontal append marker.
    let item_hierarchy_target_wins = best_item_hierarchy_target.is_some();
    if item_hierarchy_target_wins {
        *indent_release = None;
    }
    if !capabilities.canonical_hierarchy_drag
        && let Some(target) = best_item_hierarchy_target.and_then(|index| item_hierarchy_targets.get(index))
        && let Some((dragged, after)) = asset_browser_item_hierarchy_target(ui, &target.response, target.rect, target.grid, true)
        && let Some(target_group) = groups.iter().find(|group| group.name == target.group_name)
    {
        *browser_item_drop = true;
        let moving_group_path = match &dragged {
            AssetDrag::Group { name, path, .. } if *name == target.group_name && !path.is_empty() => Some(path.clone()),
            _ => None,
        };
        match dragged {
            AssetDrag::Group { scope, name, path } if scope == panel && domain_hierarchy_actions => {
                let (before, before_item) =
                    item_hierarchy_insertion_anchor(&target_group.items, target.item_index, &target.group_path, after, moving_group_path.as_deref());
                sink.actions.push(BrowserAction::MoveInto {
                    group: name,
                    group_path: path,
                    into: Some((target.group_name.clone(), target.group_path.clone())),
                    before,
                    before_item,
                });
                // The nested card/header target has consumed this release. Do not let the
                // source root header also resolve a between-root-groups reorder.
                *released = None;
            }
            AssetDrag::Group { scope, name, path } if scope == panel && path.is_empty() && !domain_hierarchy_actions => {
                if let Some(target_item) = target_group.items.get(target.item_index).map(|item| item.key.clone()) {
                    let entries = hierarchy_orders.get(&target.group_name).cloned().unwrap_or_default();
                    let target_entry = HierarchyOrderEntry::Item(target_item);
                    let before = entries
                        .iter()
                        .position(|entry| entry == &target_entry)
                        .and_then(|position| if after { entries.get(position + 1).cloned() } else { entries.get(position).cloned() });
                    if let Some(old_parent) = parents.get(&name).filter(|parent| *parent != &target.group_name) {
                        sink.state.remove_hierarchy_entry(panel, old_parent, &HierarchyOrderEntry::Group(name.clone()));
                    }
                    sink.state.nest_group(panel, group_names, &name, Some(&target.group_name));
                    sink.state.move_hierarchy_entry(panel, &target.group_name, entries, HierarchyOrderEntry::Group(name), before.as_ref());
                    *released = None;
                }
            }
            _ => {}
        }
    }
    item_hierarchy_target_wins
}

/// Register the expanded groups' indentation-column drop targets.
fn register_group_indent_drop_targets(
    ui: &mut egui::Ui,
    ctx: &BrowserCtx<'_>,
    projection: &BrowserProjection,
    drag_state: &mut BrowserDragState,
    expanded_groups: &BTreeSet<String>,
    picker_ids: &mut PickerIdRegistry,
) {
    let BrowserCtx { panel, groups, projection_keys, capabilities, .. } = *ctx;
    let BrowserProjection { display, parents, .. } = projection;
    let BrowserDragState { headers, group_content_areas, indent_drop_targets, canonical_drop, .. } = drag_state;
    // Each expanded group owns a drop area spanning its full header width. Its vertical
    // span covers the group's complete visible subtree and ends before the next sibling.
    if capabilities.group_drag {
        for (group_index, header, group_name) in headers.iter() {
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
                    let response = ui.interact(column, ui.id().with(("group-indent-column", panel, group_name.as_str())), Sense::hover());
                    if capabilities.canonical_hierarchy_drag {
                        let append_indicator = Rect::from_min_max(pos2(header.left(), bottom - 2.0), pos2(header.right(), bottom + 2.0));
                        let depth = display
                            .iter()
                            .find_map(|(index, depth)| groups.get(*index).is_some_and(|group| group.name == *group_name).then_some(*depth))
                            .unwrap_or(0);
                        let hit_index = canonical_drop.hits.borrow().len();
                        let hit =
                            group_append_hit(column, append_indicator, picker_group_id(&mut *picker_ids, *group_index, projection_keys), depth, hit_index);
                        canonical_drop.hits.borrow_mut().push((hit, response.clone()));
                    }
                    indent_drop_targets.push((group_name.clone(), response));
                }
            }
        }
    }
}

/// Inputs collected during segment rendering and consumed after that segment's item content.
struct GroupReleaseBatch {
    group_releases: Vec<GroupRelease>,
    group_toggles: Vec<String>,
}

/// Choose the eligible indentation-column target and consume its legacy group release.
fn resolve_group_indent_target(
    ui: &egui::Ui,
    ctx: &BrowserCtx<'_>,
    drag_state: &mut BrowserDragState,
    item_hierarchy_target_wins: bool,
    parents: &std::collections::BTreeMap<String, String>,
) -> Option<usize> {
    let indent_drop_targets = &drag_state.indent_drop_targets;
    // `dnd_release_payload` takes the one global egui payload. Resolve the card target
    // first, then allow the indentation column to consume it only when no card target
    // won and the pointer is actually over an eligible column. Calling it while merely
    // registering each column lets an overlapping column steal a card release.
    let indent_target_index = best_indent_drop_target_index(indent_drop_targets, item_hierarchy_target_wins, |(parent, response)| {
        if ctx.capabilities.canonical_hierarchy_drag {
            ui.ctx().pointer_interact_pos().is_some_and(|pointer| response.rect.contains(pointer))
        } else {
            response.dnd_hover_payload::<AssetDrag>().is_some_and(|payload| {
                    matches!(payload.as_ref(), AssetDrag::Group { scope, name, path } if scope == ctx.panel && path.is_empty() && name != parent && !group_is_descendant(parents, name, parent))
                })
        }
    });
    let indent_target = indent_target_index.and_then(|index| indent_drop_targets.get(index));
    if indent_target.is_none()
        && indent_drop_targets.iter().any(|(_, response)| {
            response.dnd_hover_payload::<AssetDrag>().is_some_and(|payload| matches!(payload.as_ref(), AssetDrag::Group { scope, .. } if scope == ctx.panel))
        })
    {
        drag_state.canonical_drop.non_root_target_hovered.set(true);
    }
    if !ctx.capabilities.canonical_hierarchy_drag
        && let Some((parent, response)) = indent_target
        && let Some(AssetDrag::Group { scope, name, path }) = response.dnd_release_payload::<AssetDrag>().map(|payload| payload.as_ref().clone())
        && scope == ctx.panel
        && path.is_empty()
        && name != *parent
    {
        drag_state.indent_release = Some((name, parent.clone()));
    }
    indent_target_index
}

/// Paint the legacy append and between-header insertion markers for group drops.
fn paint_group_drop_insertion_markers(
    ui: &mut egui::Ui,
    indent_target_index: Option<usize>,
    ctx: &BrowserCtx<'_>,
    projection: &BrowserProjection,
    drag_state: &BrowserDragState,
    accent: egui::Color32,
) {
    let BrowserCtx { groups, capabilities, .. } = *ctx;
    let parents = &projection.parents;
    let BrowserDragState { headers, group_content_areas, indent_drop_targets, group_drag, item_hierarchy_target_wins, .. } = drag_state;
    let headers = headers.as_slice();
    let group_content_areas = group_content_areas.as_slice();
    let group_drag = *group_drag;
    let indent_target = indent_target_index.and_then(|index| indent_drop_targets.get(index));
    // Keep the legacy append marker below the parent's complete visible subtree. The
    // canonical path instead highlights the full receiving group because group-body
    // drops now insert at its first child position.
    let indent_target_hovered = indent_target.is_some();
    if indent_target_hovered && let Some((parent, _)) = indent_target {
        let end = headers
            .iter()
            .filter(|(_, _, name)| name == parent || group_is_descendant(parents, parent, name))
            .map(|(_, rect, _)| *rect)
            .chain(group_content_areas.iter().filter(|(name, _)| name == parent || group_is_descendant(parents, parent, name)).map(|(_, rect)| *rect))
            .max_by(|a, b| a.bottom().total_cmp(&b.bottom()));
        if let (Some(anchor), Some((_, header, _))) = (end, headers.iter().find(|(_, _, name)| name == parent)) {
            // Keep the user-facing append marker aligned to the destination header,
            // not to the narrower indentation hit target or full-width content area.
            let marker_anchor = Rect::from_min_max(pos2(header.left(), anchor.bottom() - header.height()), pos2(header.right(), anchor.bottom()));
            if !capabilities.canonical_hierarchy_drag {
                paint_group_insertion_line(ui, marker_anchor, false, accent);
            }
        }
    }
    // The regular group drag marker handles between-header reordering.
    if !capabilities.canonical_hierarchy_drag
        && !indent_target_hovered
        && !item_hierarchy_target_wins
        && let Some((from, p)) = group_drag
    {
        // Header targets already paint their feedback through the shared interaction
        // helper. Only draw the between-groups insertion marker when the pointer is in a
        // gap/outside a target header.
        if !headers.iter().any(|(i, r, _)| *i != from && r.contains(p))
            && let Some(from_name) = groups.get(from).map(|group| group.name.as_str())
        {
            let before = group_drop_before(headers, from, from_name, p, parents);
            // The source header resolves to a no-op on release; never show a marker
            // suggesting that dropping it there would move it.
            if before != Some(from) {
                let source_parent = parents.get(from_name);
                let source_rect = headers.iter().find(|(index, _, _)| *index == from).map(|(_, rect, _)| *rect);
                let promote_to_root = source_parent.is_some() && source_rect.is_some_and(|rect| p.x < rect.left() + ROW_INDENT * 0.5);
                let destination_parent = if promote_to_root { None } else { source_parent };
                let anchor = match before {
                    Some(i) => headers.iter().find(|(x, _, _)| *x == i).map(|(_, r, _)| *r),
                    None => headers
                        .iter()
                        .rev()
                        .find(|(_, _, name)| parents.get(name).map(String::as_str) == destination_parent.map(String::as_str))
                        .map(|(_, r, _)| *r),
                };
                if let Some(r) = anchor {
                    paint_group_insertion_line(ui, r, before.is_some(), accent);
                }
            }
        }
    }
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

/// Allocate and paint a nested-folder header, returning its hit and disclosure responses.
fn draw_nested_folder_header(
    ui: &mut egui::Ui,
    group: &HierarchyGroup<'_, ItemView>,
    group_open: bool,
    group_depth: usize,
    parent_depth: usize,
) -> (Rect, egui::Response, egui::Response, usize) {
    let (allocated_rect, allocated) = ui.allocate_exact_size(vec2(ui.available_width(), GROUP_HEADER_HEIGHT), Sense::hover());
    let rect = Rect::from_min_max(pos2(ui.clip_rect().left(), allocated_rect.top()), pos2(allocated_rect.right(), allocated_rect.bottom()));
    let row_depth = group_depth + 1 + parent_depth;
    let response = ui.interact(inset_row_rect(rect, row_depth), allocated.id, Sense::click_and_drag());
    let nested_header_rect = inset_row_rect(rect, row_depth);
    let disclosure_rect =
        Rect::from_min_max(nested_header_rect.min, pos2((nested_header_rect.left() + 18.0).min(nested_header_rect.right()), nested_header_rect.bottom()));
    let disclosure = ui.interact(disclosure_rect, response.id.with("disclosure"), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::CollapsingHeader, true, group.path.last().map(String::as_str).unwrap_or("")));
    paint_asset_browser_header(ui, rect, group_open, group.path.last().map(String::as_str).unwrap_or(""), row_depth, response.hovered(), Some(group.count));
    (rect, response, disclosure, row_depth)
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

fn apply_root_header_release(
    ui: &egui::Ui,
    sink: &mut BrowserSink<'_>,
    ctx: &BrowserCtx<'_>,
    hierarchy_orders: &std::collections::BTreeMap<String, Vec<HierarchyOrderEntry>>,
    release: RootHeaderReleaseEvent<'_>,
) {
    let BrowserCtx { panel, groups, domain_hierarchy_actions, .. } = *ctx;
    let RootHeaderReleaseEvent { group, header_rect, dragged, browser_item_drop } = release;
    let BrowserSink { state, actions: events, .. } = sink;
    match dragged {
        AssetDrag::Preset { scope, key, .. } if scope.as_str() == panel => {
            select_dropped_item(state, panel, &group.name, &[], &key);
            events.push(BrowserAction::MoveItem(key.clone(), group.name.clone(), Some(0)));
            *browser_item_drop = true;
            if !domain_hierarchy_actions
                && let Some((source_index, _)) = item_occurrence_position(groups, &key)
                && let Some(source_group) = groups.get(source_index)
            {
                move_item_sibling_order(
                    state,
                    panel,
                    hierarchy_orders,
                    &source_group.name,
                    &group.name,
                    &key,
                    hierarchy_orders.get(&group.name).and_then(|entries| entries.first()),
                );
            }
        }
        // Top-level group source headers are handled once by the final release resolver below,
        // which also knows the between-header insertion order.
        AssetDrag::Group { scope, path, .. } if scope.as_str() == panel && path.is_empty() && domain_hierarchy_actions => {}
        AssetDrag::Group { scope, name, path } if scope.as_str() == panel && domain_hierarchy_actions => {
            let edge = ui.ctx().pointer_interact_pos().is_some_and(|p| header_drop_before(header_rect, p));
            if let Some(action) = hierarchy_group_drop(&name, &path, &group.name, &[], edge) {
                events.push(hierarchy_action(action));
            }
        }
        _ => {}
    }
}

/// Visibility bookkeeping for one ordered pass over the projected render segments.
/// It owns the sets mutated while ancestors and collapsed folders are traversed.
struct SegmentVisibilityState<'a> {
    hidden_groups: &'a mut BTreeSet<String>,
    hidden_folders: &'a mut BTreeSet<(String, Vec<String>)>,
    expanded_groups: &'a mut BTreeSet<String>,
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

fn register_browser_item_drop_targets(
    ui: &mut egui::Ui,
    geometry: ItemDropRegistrationGeometry<'_, '_>,
    ctx: &BrowserCtx<'_>,
    projection: &BrowserProjection,
    sink: &BrowserSink<'_>,
    drag_state: &mut BrowserDragState,
) -> Option<AssetDrag> {
    let ItemDropRegistrationGeometry { painted, grid, run_index } = geometry;
    let item_count = grid.item_count;
    let list = grid.list;
    let BrowserCtx { panel, capabilities, domain_hierarchy_actions, .. } = *ctx;
    let ItemRef { group_index: item_group_index, item_index, group: item_group, item } = painted.item;
    let response = &painted.response;
    let rect = painted.rect;
    let next_rect = painted.next_rect;
    let item_depth = painted.depth;
    let hierarchy_accepts = response.dnd_hover_payload::<AssetDrag>().is_some_and(|payload| {
        hierarchy_drop_accepts(payload.as_ref(), panel, &item_group.name, &item.group_path, &projection.parents, domain_hierarchy_actions)
    });
    let hierarchy_drag =
        response.dnd_hover_payload::<AssetDrag>().is_some_and(|payload| matches!(payload.as_ref(), AssetDrag::Group { scope, .. } if scope == panel));
    if hierarchy_drag && !hierarchy_accepts {
        // Invalid/self/unsupported card targets must not fall through and reinterpret the same
        // release as a root-group reorder.
        drag_state.canonical_drop.non_root_target_hovered.set(true);
    }
    if capabilities.group_drag {
        drag_state.item_hierarchy_targets.push(ItemHierarchyTarget {
            response: response.clone(),
            rect,
            grid: !list,
            accepts: hierarchy_accepts,
            item_index,
            group_name: item_group.name.clone(),
            group_path: item.group_path.clone(),
        });
    }
    if capabilities.canonical_hierarchy_drag {
        if capabilities.item_drag
            && let Some(source_id) = item_node_for_identity(&item_group.name, &item.group_path, &item.key, &projection.hierarchy_identities)
        {
            let selected_nodes = selected_drag_nodes(sink.state, panel, &projection.visible_selection_order, &source_id, &projection.hierarchy);
            asset_browser_item_drag_source_with_id(response, panel, &item.key, source_id, &selected_nodes);
        }
        if let Some(target_id) = item_node_for_identity(&item_group.name, &item.group_path, &item.key, &projection.hierarchy_identities) {
            drag_state.canonical_drop.node_bounds.borrow_mut().insert(target_id.clone(), rect);
            let hit_index = drag_state.canonical_drop.hits.borrow().len();
            // The same hit widget backs cards and list rows. In list mode its leading/trailing
            // halves are top/bottom; card_drop_hits owns that orientation and preview.
            for hit in card_drop_hits(rect, !list, target_id, item_depth, hit_index) {
                drag_state.canonical_drop.hits.borrow_mut().push((hit, response.clone()));
            }
        }
    }
    let mut released_drag = if capabilities.item_drag && !hierarchy_drag && !capabilities.canonical_hierarchy_drag {
        asset_browser_item_target(
            ui,
            ItemDropTarget { response, rect, next_rect, insertion_markers: &mut drag_state.item_insertion_markers, scope: panel, key: &item.key, grid: !list },
        )
    } else {
        None
    };
    // Grid rows leave horizontal whitespace after their last card, so expose that strip as an
    // extra target. A list row already spans the available width and its lower half is trailing.
    // Prepend stays within the card/row's leading half in both layouts.
    if capabilities.item_drag && !list && (capabilities.canonical_hierarchy_drag || released_drag.is_none()) {
        let last_in_row = run_index + 1 == item_count || (run_index + 1) % grid.columns == 0;
        let right_edge = item_row_edge_targets(grid.area, rect, last_in_row);
        let pointer = ui.ctx().pointer_interact_pos();
        if capabilities.canonical_hierarchy_drag
            && let Some(edge_rect) = right_edge
        {
            let edge_response = ui.interact(edge_rect, ui.id().with((panel, item_group_index, item_index, "right")), Sense::hover());
            if let Some(target_id) = item_node_for_identity(&item_group.name, &item.group_path, &item.key, &projection.hierarchy_identities) {
                let hit_index = drag_state.canonical_drop.hits.borrow().len();
                let mut hit = card_drop_hit(rect, Pos2::new(rect.right(), rect.center().y), true, target_id, item_depth, hit_index);
                hit.hit_rect = edge_rect;
                drag_state.canonical_drop.hits.borrow_mut().push((hit, edge_response));
            }
        }
        let edge_target = right_edge.filter(|edge_rect| pointer.is_some_and(|pointer| edge_rect.contains(pointer))).map(|edge_rect| ("right", edge_rect));
        if let Some((side, edge_rect)) = edge_target {
            let edge_response = ui.interact(edge_rect, ui.id().with((panel, item_group_index, item_index, side)), Sense::hover());
            let edge_hierarchy_accepts = edge_response.dnd_hover_payload::<AssetDrag>().is_some_and(|payload| {
                hierarchy_drop_accepts(payload.as_ref(), panel, &item_group.name, &item.group_path, &projection.parents, domain_hierarchy_actions)
            });
            let edge_hierarchy_drag = edge_response
                .dnd_hover_payload::<AssetDrag>()
                .is_some_and(|payload| matches!(payload.as_ref(), AssetDrag::Group { scope, .. } if scope == panel));
            if capabilities.group_drag {
                drag_state.item_hierarchy_targets.push(ItemHierarchyTarget {
                    response: edge_response.clone(),
                    rect,
                    grid: true,
                    accepts: edge_hierarchy_accepts,
                    item_index,
                    group_name: item_group.name.clone(),
                    group_path: item.group_path.clone(),
                });
            }
            if !edge_hierarchy_drag && !capabilities.canonical_hierarchy_drag {
                released_drag = asset_browser_item_target(
                    ui,
                    ItemDropTarget {
                        response: &edge_response,
                        rect,
                        next_rect,
                        insertion_markers: &mut drag_state.item_insertion_markers,
                        scope: panel,
                        key: &item.key,
                        grid: true,
                    },
                );
            }
        }
    }
    released_drag
}

fn apply_browser_item_gesture(
    ui: &egui::Ui,
    painted: &PaintedCell<'_>,
    ctx: &BrowserCtx<'_>,
    sink: &mut BrowserSink<'_>,
    visible_selection_order: &[PickerId],
) -> egui::Response {
    let ItemRef { group, item, .. } = painted.item;
    let response = painted.response.clone();
    let BrowserCtx { panel, capabilities, .. } = *ctx;
    let hover = if item.group_path.is_empty() { item.name.clone() } else { format!("{} / {}", item.group_path.join(" / "), item.name) };
    let response = response.on_hover_text(hover);
    let (selected, activated) = asset_browser_item_gesture(&response, capabilities.selection, capabilities.activation);
    if selected && let Some(id) = painted.node_id.as_ref() {
        let modifiers = ui.input(|input| input.modifiers);
        if modifiers.shift {
            sink.select_range(panel, visible_selection_order, id);
        } else {
            sink.select_node(panel, id.clone(), modifiers.ctrl);
        }
    }
    if activated {
        sink.emit(BrowserAction::ActivateItem { group: group.name.clone(), group_path: item.group_path.clone(), key: item.key.clone() });
    } else if selected {
        sink.emit(BrowserAction::SelectItem { group: group.name.clone(), group_path: item.group_path.clone(), key: item.key.clone() });
    }
    response
}

fn draw_context_menu_target(ui: &mut egui::Ui, response: egui::Response, compact: bool, draw_menu: impl FnOnce(&mut egui::Ui)) {
    if compact {
        compact_context_menu_target(ui, response, draw_menu);
    } else {
        context_menu_target(response, draw_menu);
    }
}

/// Register the root-leading insertion target after the first visible hierarchy node is laid out.
fn register_canonical_root_leading_target(
    ui: &mut egui::Ui,
    ctx: &BrowserCtx<'_>,
    projection: &BrowserProjection,
    drag: &BrowserDragState,
    browser_parent_clip: Rect,
) {
    let panel = ctx.panel;
    let capabilities = ctx.capabilities;
    let hierarchy = &projection.hierarchy;
    let canonical_node_bounds = &drag.canonical_drop.node_bounds;
    let canonical_drop_hits = &drag.canonical_drop.hits;
    if capabilities.canonical_hierarchy_drag
        && let Some(first) = hierarchy.roots().first()
        && let Some(first_bounds) = canonical_node_bounds.borrow().get(first.id()).copied()
    {
        let hit_rect =
            Rect::from_min_max(pos2(browser_parent_clip.left(), browser_parent_clip.top()), pos2(browser_parent_clip.right(), first_bounds.top() + 0.75));
        if hit_rect.is_positive() {
            let indicator = if first.is_group() {
                Rect::from_min_max(pos2(browser_parent_clip.left(), first_bounds.top()), pos2(browser_parent_clip.right(), first_bounds.top() + 4.0))
            } else {
                card_leading_marker_rect(first_bounds)
            };
            let response = ui.interact(hit_rect, ui.id().with(("root-leading-group-drop", panel)), Sense::hover());
            canonical_drop_hits.borrow_mut().push((root_prepend_hit(hit_rect, indicator, first.id().clone(), usize::MAX), response));
        }
    }
}

/// Register the canonical root-end target after all visible rows have final bounds.
fn register_canonical_root_end_target(
    ui: &mut egui::Ui,
    capabilities: BrowserCapabilities,
    hierarchy: &PickerBrowserHierarchy,
    canonical_drop: &CanonicalDropState,
    list: bool,
) {
    if capabilities.canonical_hierarchy_drag {
        // The leading target is registered before rows are laid out. Resolve a card's
        // actual edge now that its final grid bounds are available.
        if let Some(first) = hierarchy.roots().first()
            && !first.is_group()
            && let Some(bounds) = canonical_drop.node_bounds.borrow().get(first.id()).copied()
            && let Some((hit, _)) =
                canonical_drop.hits.borrow_mut().iter_mut().find(|(hit, _)| hit.region == PointerRegion::LeadingEdge && hit.target.as_ref() == Some(first.id()))
        {
            hit.indicator_rect = card_leading_marker_rect(bounds);
        }
        // The whole trailing blank area is the root append target, not just a narrow
        // sixteen-pixel strip immediately after the final node. This lets users drop in
        // the whitespace below the last group while keeping the indicator at its top edge.
        let root_end_height = (ui.clip_rect().bottom() - ui.cursor().top()).max(32.0);
        let (root_end_rect, root_end_response) = ui.allocate_exact_size(vec2(ui.available_width(), root_end_height), Sense::hover());
        let indicator = match hierarchy.roots().last() {
            Some(last) if !last.is_group() => {
                canonical_drop.node_bounds.borrow().get(last.id()).copied().map(|bounds| card_trailing_marker_rect(bounds, !list)).unwrap_or_else(|| {
                    Rect::from_min_max(pos2(ui.clip_rect().left(), root_end_rect.top() - 2.0), pos2(ui.clip_rect().right(), root_end_rect.top() + 2.0))
                })
            }
            _ => Rect::from_min_max(pos2(ui.clip_rect().left(), root_end_rect.top() - 2.0), pos2(ui.clip_rect().right(), root_end_rect.top() + 2.0)),
        };
        let hit_index = canonical_drop.hits.borrow().len();
        let visible_root_end = root_end_rect.intersect(ui.clip_rect());
        if visible_root_end.is_positive() {
            canonical_drop.hits.borrow_mut().push((root_append_hit(visible_root_end, indicator, hit_index), root_end_response));
        }
    }
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
    let mut painters = Painters {
        group: adapter.paint_group.as_mut().map(|paint| &mut **paint),
        item: &mut *adapter.paint_item,
        list_item: adapter.paint_list_item.as_mut().map(|paint| &mut **paint),
    };
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
                                    sink.commit_item_drop(ItemDropCommit {
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
                                    && !sink.is_node_selected(panel, &id)
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
                    sink.apply_group_releases_and_toggles(GroupReleaseBatch { group_releases, group_toggles }, &browser_ctx, &mut drag_state);
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

#[cfg(test)]
#[path = "asset_browser_ui_tests.rs"]
mod ui_tests;
#[cfg(test)]
pub(crate) use ui_tests::{test_last_browser_rect as last_browser_rect, test_last_group_headers as last_group_headers};
