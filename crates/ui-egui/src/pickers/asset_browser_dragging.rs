//! Drag/drop targeting, arbitration, and release resolution for the shared asset browser.
//!
//! `asset_browser` remains the frame authority: it lays out rows, supplies the current browser
//! projection, and calls these operations at the appropriate points in the frame. Pointer/gesture
//! primitives and drag-ghost presentation live in `asset_browser_interaction`.
// dragging behaviour the picker should have
// in the browser:
// 1. groups and cards are the only elements
// 2. groups always span a line
// 3. the display of the cards depends on the thumbnail view mode.

// behaviour:
// 1. cards have hit areas; left and right. dragging something on these hitareas makes a vertical hitmarker appear in the inbetween gap.
// 2. groups also have hitareas: top, bottom and middle. the top and bottom hit areas make hitmarkers appear in the inbetween gap, the middle shows the outline.
// (this is regarding the header, not the contained area)

// Dragging something on the left and right, or top and bottom hit areas creates a sibling relationship (before/after). Dropping something onto the middle prepends it into the group.

// Regarding the bottom hit area: When a group is open, the bottom hit area doesn't show up at the header, but shows up beneath its contained items.

// there are some additional hitareas:
// 1. end of a card list. there is an extended hitarea behind the cards so we can append something to the end of a row
// 2. End of the root. Dragging something to the end shows either the last horizontal or vertical indicator, depending on what the last element is
// 3. The identations. The identations on the left side are an additional hitarea that can do group appendages these are the vertical areas alongside a group on the left side which also are the visual identation and append it to the indented group.

use egui::{Pos2, Rect, Sense, pos2};

use crate::pickers::asset_browser::{
    BrowserAction, BrowserCapabilities, BrowserCtx, BrowserHierarchyView, BrowserProjection, BrowserSink, HierarchyCommit, ItemDropRegistrationGeometry,
    ItemGrid, ItemHierarchyDropPolicy, ItemHierarchyDropResolution, ItemRef, PaintedCell, group_is_descendant, hierarchy_action, hierarchy_drop_accepts,
    item_node_for_identity, picker_group_id, select_dropped_item, selected_drag_nodes,
};
use crate::pickers::asset_browser_groups::ROW_INDENT;
use crate::pickers::asset_browser_interaction::{AssetDrag, BrowserDragGhosts, asset_browser_take_release};
use crate::pickers::asset_browser_interaction::{GroupGapDropSpec, group_gap_drop_hits, group_gap_upper_anchor, header_drop_before, hierarchy_group_drop};
use crate::pickers::picker_browser_hierarchy::commit_hierarchy_move;

use crate::pickers::asset_browser_interaction::{
    ItemDropTarget, asset_browser_item_drag_source_with_id, asset_browser_item_gesture, asset_browser_item_hierarchy_target, asset_browser_item_target,
    card_drop_hit, card_drop_hits, card_leading_marker_rect, card_trailing_marker_rect, group_append_hit, item_row_edge_targets, paint_group_insertion_line,
    root_append_hit, root_prepend_hit,
};
use crate::pickers::picker_browser_dragging::{
    DragSession, DropCommitResult, DropHitRegion, PointerRegion, arbitrate_hit_index, hierarchy_move_sources, paint_candidate,
};
use crate::pickers::picker_browser_identity::{PickerNodeIdentity, PickerNodeRole};
use crate::pickers::picker_browser_model::PickerBrowserHierarchy;
use crate::pickers::picker_browser_model::{ContainerId, DropPosition, HierarchyNode, PickerId};
use crate::pickers::picker_browser_projection::ItemView;
use crate::pickers::picker_state::{HierarchyOrderEntry, PresetUi};

pub(super) struct ItemDropSource<'a> {
    pub(super) key: &'a str,
    pub(super) node_id: Option<&'a PickerId>,
}

pub(super) struct ItemDropCommit<'a, 'ctx> {
    pub(super) ctx: &'a BrowserCtx<'ctx>,
    pub(super) projection: &'a BrowserProjection,
    pub(super) target: &'a PaintedCell<'ctx>,
    pub(super) source: ItemDropSource<'a>,
    pub(super) root_level_items: bool,
    pub(super) after: bool,
}

pub(super) struct RootHeaderReleaseEvent<'a> {
    pub(super) group: &'a crate::pickers::picker_browser_projection::GroupView,
    pub(super) header_rect: Rect,
    pub(super) dragged: AssetDrag,
    pub(super) browser_item_drop: &'a mut bool,
}

pub(super) struct GroupReleaseBatch {
    pub(super) group_releases: Vec<GroupRelease>,
    pub(super) group_toggles: Vec<String>,
}

pub(crate) fn item_occurrence_position(groups: &[crate::pickers::picker_browser_projection::GroupView], key: &str) -> Option<(usize, usize)> {
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

pub(super) fn commit_item_drop(sink: &mut BrowserSink<'_>, commit: ItemDropCommit<'_, '_>) {
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
            sink.state.move_hierarchy_entry(panel, "", entries, source_entry, before.as_ref());
            select_dropped_item(sink.state, panel, &source_group, &[], key);
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
            move_item_sibling_order(sink.state, panel, hierarchy_orders, &source_group.name, &target_group.name, key, before.as_ref());
        }
        select_dropped_item(sink.state, panel, &target_group.name, &[], key);
        sink.actions.push(BrowserAction::MoveItem(key.to_owned(), target_group.name.clone(), Some(index)));
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
        select_dropped_item(sink.state, panel, &target_group.name, &target_item.group_path, key);
        sink.actions.push(BrowserAction::MoveItemToGroup {
            key: key.to_owned(),
            group: target_group.name.clone(),
            group_path: target_item.group_path.clone(),
            before,
        });
    }
}

pub(super) fn apply_group_releases_and_toggles(sink: &mut BrowserSink<'_>, batch: GroupReleaseBatch, ctx: &BrowserCtx<'_>, drag_state: &mut BrowserDragState) {
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
                select_dropped_item(sink.state, panel, &target_group, &target_group_path, &key);
                sink.actions.push(BrowserAction::MoveItemToGroup { key, group: target_group, group_path: target_group_path, before });
                drag_state.browser_item_drop = true;
            }
            AssetDrag::Group { scope, name, path } if scope == panel && domain_hierarchy_actions => {
                if let Some(action) = hierarchy_group_drop(&name, &path, &target_group, &target_group_path, edge) {
                    sink.actions.push(hierarchy_action(action));
                }
            }
            _ => {}
        }
    }
    for key in group_toggles {
        if !sink.state.toggled.remove(&key) {
            sink.state.toggled.insert(key);
        }
    }
}

pub(super) fn apply_root_header_release(
    ui: &egui::Ui,
    sink: &mut BrowserSink<'_>,
    ctx: &BrowserCtx<'_>,
    hierarchy_orders: &std::collections::BTreeMap<String, Vec<HierarchyOrderEntry>>,
    release: RootHeaderReleaseEvent<'_>,
) {
    let BrowserCtx { panel, groups, domain_hierarchy_actions, .. } = *ctx;
    let RootHeaderReleaseEvent { group, header_rect, dragged, browser_item_drop } = release;
    match dragged {
        AssetDrag::Preset { scope, key, .. } if scope.as_str() == panel => {
            select_dropped_item(sink.state, panel, &group.name, &[], &key);
            sink.actions.push(BrowserAction::MoveItem(key.clone(), group.name.clone(), Some(0)));
            *browser_item_drop = true;
            if !domain_hierarchy_actions
                && let Some((source_index, _)) = item_occurrence_position(groups, &key)
                && let Some(source_group) = groups.get(source_index)
            {
                move_item_sibling_order(
                    sink.state,
                    panel,
                    hierarchy_orders,
                    &source_group.name,
                    &group.name,
                    &key,
                    hierarchy_orders.get(&group.name).and_then(|entries| entries.first()),
                );
            }
        }
        AssetDrag::Group { scope, path, .. } if scope.as_str() == panel && path.is_empty() && domain_hierarchy_actions => {}
        AssetDrag::Group { scope, name, path } if scope.as_str() == panel && domain_hierarchy_actions => {
            let edge = ui.ctx().pointer_interact_pos().is_some_and(|p| header_drop_before(header_rect, p));
            if let Some(action) = hierarchy_group_drop(&name, &path, &group.name, &[], edge) {
                sink.actions.push(hierarchy_action(action));
            }
        }
        _ => {}
    }
}

const GROUP_GAP_DROP_HEIGHT: f32 = 6.0;

/// Resolve item-level hierarchy drops before indentation-column arbitration.
pub(super) fn resolve_item_hierarchy_drop_target(
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
    let best_item_hierarchy_target = best_drop_target_index(item_hierarchy_targets, |target| {
        target.accepts
            && target
                .response
                .dnd_hover_payload::<AssetDrag>()
                .is_some_and(|payload| matches!(payload.as_ref(), AssetDrag::Group { scope, .. } if scope == panel))
    });
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

/// Apply shared click/double-click selection semantics and emit the corresponding browser action.
pub(super) fn apply_browser_item_gesture(
    ui: &egui::Ui,
    painted: &PaintedCell<'_>,
    ctx: &BrowserCtx<'_>,
    sink: &mut BrowserSink<'_>,
    visible_selection_order: &[PickerId],
) -> egui::Response {
    let group = painted.item.group;
    let item = painted.item.item;
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

/// Arbitrate the indentation-column target and consume its legacy group release when it wins.
pub(super) fn resolve_group_indent_target(
    ui: &egui::Ui,
    ctx: &BrowserCtx<'_>,
    drag_state: &mut BrowserDragState,
    item_hierarchy_target_wins: bool,
    parents: &std::collections::BTreeMap<String, String>,
) -> Option<usize> {
    let indent_drop_targets = &drag_state.indent_drop_targets;
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
pub(super) fn paint_group_drop_insertion_markers(
    ui: &mut egui::Ui,
    indent_target_index: Option<usize>,
    ctx: &BrowserCtx<'_>,
    projection: &BrowserProjection,
    drag_state: &BrowserDragState,
    accent: egui::Color32,
) {
    let groups = ctx.groups;
    let capabilities = ctx.capabilities;
    let parents = &projection.parents;
    let headers = drag_state.headers.as_slice();
    let group_content_areas = drag_state.group_content_areas.as_slice();
    let group_drag = drag_state.group_drag;
    let indent_target = indent_target_index.and_then(|index| drag_state.indent_drop_targets.get(index));
    let indent_target_hovered = indent_target.is_some();
    if let Some((parent, _)) = indent_target {
        let end = headers
            .iter()
            .filter(|(_, _, name)| name == parent || group_is_descendant(parents, parent, name))
            .map(|(_, rect, _)| *rect)
            .chain(group_content_areas.iter().filter(|(name, _)| name == parent || group_is_descendant(parents, parent, name)).map(|(_, rect)| *rect))
            .max_by(|a, b| a.bottom().total_cmp(&b.bottom()));
        if let (Some(anchor), Some((_, header, _))) = (end, headers.iter().find(|(_, _, name)| name == parent)) {
            let marker_anchor = Rect::from_min_max(pos2(header.left(), anchor.bottom() - header.height()), pos2(header.right(), anchor.bottom()));
            if !capabilities.canonical_hierarchy_drag {
                paint_group_insertion_line(ui, marker_anchor, false, accent);
            }
        }
    }
    if !capabilities.canonical_hierarchy_drag
        && !indent_target_hovered
        && !drag_state.item_hierarchy_target_wins
        && let Some((from, pointer)) = group_drag
        && !headers.iter().any(|(index, rect, _)| *index != from && rect.contains(pointer))
        && let Some(from_name) = groups.get(from).map(|group| group.name.as_str())
    {
        let before = group_drop_before(headers, from, from_name, pointer, parents);
        if before != Some(from) {
            let source_parent = parents.get(from_name);
            let source_rect = headers.iter().find(|(index, _, _)| *index == from).map(|(_, rect, _)| *rect);
            let promote_to_root = source_parent.is_some() && source_rect.is_some_and(|rect| pointer.x < rect.left() + ROW_INDENT * 0.5);
            let destination_parent = if promote_to_root { None } else { source_parent };
            let anchor = match before {
                Some(index) => headers.iter().find(|(candidate, _, _)| *candidate == index).map(|(_, rect, _)| *rect),
                None => headers
                    .iter()
                    .rev()
                    .find(|(_, _, name)| parents.get(name).map(String::as_str) == destination_parent.map(String::as_str))
                    .map(|(_, rect, _)| *rect),
            };
            if let Some(rect) = anchor {
                paint_group_insertion_line(ui, rect, before.is_some(), accent);
            }
        }
    }
}

/// Register the root-leading insertion target after the first visible hierarchy node is laid out.
pub(super) fn register_canonical_root_leading_target(
    ui: &mut egui::Ui,
    ctx: &BrowserCtx<'_>,
    projection: &BrowserProjection,
    drag: &BrowserDragState,
    browser_parent_clip: Rect,
) {
    if ctx.capabilities.canonical_hierarchy_drag
        && let Some(first) = projection.hierarchy.roots().first()
        && let Some(first_bounds) = drag.canonical_drop.node_bounds.borrow().get(first.id()).copied()
    {
        let hit_rect =
            Rect::from_min_max(pos2(browser_parent_clip.left(), browser_parent_clip.top()), pos2(browser_parent_clip.right(), first_bounds.top() + 0.75));
        if hit_rect.is_positive() {
            let indicator = if first.is_group() {
                Rect::from_min_max(pos2(browser_parent_clip.left(), first_bounds.top()), pos2(browser_parent_clip.right(), first_bounds.top() + 4.0))
            } else {
                card_leading_marker_rect(first_bounds)
            };
            let response = ui.interact(hit_rect, ui.id().with(("root-leading-group-drop", ctx.panel)), egui::Sense::hover());
            drag.canonical_drop.hits.borrow_mut().push((root_prepend_hit(hit_rect, indicator, first.id().clone(), usize::MAX), response));
        }
    }
}

/// Register the canonical root-end target after all visible rows have final bounds.
pub(super) fn register_canonical_root_end_target(
    ui: &mut egui::Ui,
    capabilities: BrowserCapabilities,
    hierarchy: &PickerBrowserHierarchy,
    canonical_drop: &CanonicalDropState,
    list: bool,
) {
    if capabilities.canonical_hierarchy_drag {
        if let Some(first) = hierarchy.roots().first()
            && !first.is_group()
            && let Some(bounds) = canonical_drop.node_bounds.borrow().get(first.id()).copied()
            && let Some((hit, _)) =
                canonical_drop.hits.borrow_mut().iter_mut().find(|(hit, _)| hit.region == PointerRegion::LeadingEdge && hit.target.as_ref() == Some(first.id()))
        {
            hit.indicator_rect = card_leading_marker_rect(bounds);
        }
        let root_end_height = (ui.clip_rect().bottom() - ui.cursor().top()).max(32.0);
        let (root_end_rect, root_end_response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), root_end_height), egui::Sense::hover());
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

/// Register the expanded groups' indentation-column drop targets.
pub(super) fn register_group_indent_drop_targets(
    ui: &mut egui::Ui,
    ctx: &BrowserCtx<'_>,
    projection: &BrowserProjection,
    drag_state: &mut BrowserDragState,
    expanded_groups: &std::collections::BTreeSet<String>,
    picker_ids: &mut crate::pickers::picker_browser_model::PickerIdRegistry,
) {
    if ctx.capabilities.group_drag {
        for (group_index, header, group_name) in &drag_state.headers {
            if !expanded_groups.contains(group_name) {
                continue;
            }
            let bottom = drag_state
                .headers
                .iter()
                .filter(|(_, _, name)| name == group_name || group_is_descendant(&projection.parents, group_name, name))
                .map(|(_, rect, _)| rect.bottom())
                .chain(
                    drag_state
                        .group_content_areas
                        .iter()
                        .filter(|(name, _)| name == group_name || group_is_descendant(&projection.parents, group_name, name))
                        .map(|(_, rect)| rect.bottom()),
                )
                .max_by(f32::total_cmp);
            if let Some(bottom) = bottom {
                let column = Rect::from_min_max(pos2(header.left(), header.bottom()), pos2(header.left() + ROW_INDENT, bottom));
                if column.is_positive() {
                    let response = ui.interact(column, ui.id().with(("group-indent-column", ctx.panel, group_name.as_str())), egui::Sense::hover());
                    if ctx.capabilities.canonical_hierarchy_drag {
                        let append_indicator = Rect::from_min_max(pos2(header.left(), bottom - 2.0), pos2(header.right(), bottom + 2.0));
                        let depth = projection
                            .display
                            .iter()
                            .find_map(|(index, depth)| ctx.groups.get(*index).is_some_and(|group| group.name == *group_name).then_some(*depth))
                            .unwrap_or(0);
                        let hit_index = drag_state.canonical_drop.hits.borrow().len();
                        let hit = group_append_hit(column, append_indicator, picker_group_id(picker_ids, *group_index, ctx.projection_keys), depth, hit_index);
                        drag_state.canonical_drop.hits.borrow_mut().push((hit, response.clone()));
                    }
                    drag_state.indent_drop_targets.push((group_name.clone(), response));
                }
            }
        }
    }
}

/// Register legacy and canonical card-edge targets for one painted item occurrence.
pub(super) fn register_browser_item_drop_targets(
    ui: &mut egui::Ui,
    geometry: ItemDropRegistrationGeometry<'_, '_>,
    ctx: &BrowserCtx<'_>,
    projection: &BrowserProjection,
    sink: &BrowserSink<'_>,
    drag_state: &mut BrowserDragState,
) -> Option<AssetDrag> {
    let ItemDropRegistrationGeometry { painted, grid, run_index } = geometry;
    let ItemGrid { item_count, list, .. } = grid;
    let panel = ctx.panel;
    let capabilities = ctx.capabilities;
    let domain_hierarchy_actions = ctx.domain_hierarchy_actions;
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
    if capabilities.item_drag && !list && (capabilities.canonical_hierarchy_drag || released_drag.is_none()) {
        let last_in_row = run_index + 1 == item_count || (run_index + 1) % grid.columns == 0;
        let right_edge = item_row_edge_targets(grid.area, rect, last_in_row);
        let pointer = ui.ctx().pointer_interact_pos();
        if capabilities.canonical_hierarchy_drag
            && let Some(edge_rect) = right_edge
        {
            let edge_response = ui.interact(edge_rect, ui.id().with((panel, item_group_index, item_index, "right")), egui::Sense::hover());
            if let Some(target_id) = item_node_for_identity(&item_group.name, &item.group_path, &item.key, &projection.hierarchy_identities) {
                let hit_index = drag_state.canonical_drop.hits.borrow().len();
                let mut hit = card_drop_hit(rect, Pos2::new(rect.right(), rect.center().y), true, target_id, item_depth, hit_index);
                hit.hit_rect = edge_rect;
                drag_state.canonical_drop.hits.borrow_mut().push((hit, edge_response));
            }
        }
        let edge_target = right_edge.filter(|edge_rect| pointer.is_some_and(|pointer| edge_rect.contains(pointer))).map(|edge_rect| ("right", edge_rect));
        if let Some((side, edge_rect)) = edge_target {
            let edge_response = ui.interact(edge_rect, ui.id().with((panel, item_group_index, item_index, side)), egui::Sense::hover());
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

/// Return the last visible card bounds in a hierarchy subtree.
pub(crate) fn last_visible_descendant_bounds(node: &HierarchyNode, bounds: &std::collections::BTreeMap<PickerId, Rect>) -> Option<Rect> {
    if node.is_group() { node.children().iter().rev().find_map(|child| last_visible_descendant_bounds(child, bounds)) } else { bounds.get(node.id()).copied() }
}

/// Pick the last eligible target, matching the browser's existing reverse-order drop arbitration.
pub(crate) fn best_drop_target_index<T>(targets: &[T], mut eligible: impl FnMut(&T) -> bool) -> Option<usize> {
    targets.iter().rposition(&mut eligible)
}

/// An item target outranks the indentation-column target when both hit regions overlap.
pub(crate) fn best_indent_drop_target_index<T>(targets: &[T], item_target_wins: bool, eligible: impl FnMut(&T) -> bool) -> Option<usize> {
    (!item_target_wins).then(|| best_drop_target_index(targets, eligible)).flatten()
}

/// Find the next hierarchy sibling at the destination level after a card. A following direct
/// preset is a stable item anchor; a following child group is a sibling-group anchor. This keeps
/// a drop immediately after a card ahead of a nested group that happens to follow it in storage.
pub(crate) fn item_hierarchy_insertion_anchor(
    items: &[ItemView],
    index: usize,
    group_path: &[String],
    after: bool,
    moving_group_path: Option<&[String]>,
) -> (Option<String>, Option<String>) {
    if !after {
        return (None, items.get(index).map(|item| item.key.clone()));
    }
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

/// Canvas application drops must not steal releases that land anywhere in the browser viewport.
pub(crate) fn canvas_drop_target(pointer: Pos2, item: Rect, canvas: Rect, browser: Rect) -> bool {
    !item.contains(pointer) && !browser.contains(pointer) && canvas.contains(pointer)
}

/// Resolve the sibling group a dragged header lands before (`None` means the last sibling).
/// Root and nested groups use the same sibling-level positioning rule.
pub(crate) fn group_drop_before(
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

pub(super) struct ItemHierarchyTarget {
    pub(super) response: egui::Response,
    pub(super) rect: Rect,
    pub(super) grid: bool,
    pub(super) accepts: bool,
    pub(super) item_index: usize,
    pub(super) group_name: String,
    pub(super) group_path: Vec<String>,
}

/// Mutable drag/drop data collected during layout and consumed in the frame's ordered resolution tail.
#[derive(Default)]
pub(super) struct BrowserDragState {
    pub(super) headers: Vec<(usize, Rect, String)>,
    pub(super) group_content_areas: Vec<(String, Rect)>,
    pub(super) indent_drop_targets: Vec<(String, egui::Response)>,
    pub(super) item_hierarchy_target_wins: bool,
    pub(super) indent_release: Option<(String, String)>,
    pub(super) dropped: Option<(String, Pos2)>,
    pub(super) browser_item_drop: bool,
    pub(super) group_drag: Option<(usize, Pos2)>,
    pub(super) group_release: Option<(usize, Pos2)>,
    pub(super) group_releases: Vec<GroupRelease>,
    pub(super) ghosts: BrowserDragGhosts,
    pub(super) canonical_drop: CanonicalDropState,
    pub(super) item_hierarchy_targets: Vec<ItemHierarchyTarget>,
    pub(super) item_insertion_markers: Vec<(Rect, bool, bool)>,
}

pub(super) struct GroupRelease {
    pub(super) dragged: AssetDrag,
    pub(super) target_group: String,
    pub(super) target_group_path: Vec<String>,
    pub(super) edge: bool,
}

/// Canonical hierarchy-drop geometry and hover state shared by target registration.
#[derive(Default)]
pub(super) struct CanonicalDropState {
    pub(super) gap_headers: Vec<(PickerId, Rect, usize, bool)>,
    pub(super) node_bounds: std::cell::RefCell<std::collections::BTreeMap<PickerId, Rect>>,
    pub(super) hits: std::cell::RefCell<Vec<(DropHitRegion, egui::Response)>>,
    pub(super) non_root_target_hovered: std::cell::Cell<bool>,
}

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
pub(crate) fn plan_hierarchy_item_moves(
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

/// Register canonical insertion targets at sibling boundaries involving group headers.
///
/// These targets are resolved only after every visible node has its final layout bounds.
pub(super) fn register_canonical_sibling_gap_targets(ui: &mut egui::Ui, ctx: &BrowserCtx<'_>, projection: &BrowserProjection, drag: &BrowserDragState) {
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
pub(super) fn resolve_canonical_drop(
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
pub(super) fn apply_legacy_group_release(
    state: &mut PresetUi,
    ctx: &BrowserCtx<'_>,
    projection: &BrowserProjection,
    drag_state: &BrowserDragState,
) -> Vec<BrowserAction> {
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
