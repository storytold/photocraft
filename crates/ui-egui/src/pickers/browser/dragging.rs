//! Object-relative drag targets for the picker browser.
//!
//! Layout registers the objects it paints. Each object's hit areas resolve to one structural
//! destination; gaps are only where insertion indicators are drawn, never the source of meaning.

use egui::{Pos2, Rect, pos2};

use crate::pickers::browser::asset_browser::{
    BrowserCtx, BrowserProjection, BrowserSink, HierarchyCommit, ItemDropRegistrationGeometry, ItemRef, item_node_for_identity, select_dropped_item,
    selected_drag_nodes,
};
use crate::pickers::browser::browser_interaction::{AssetDrag, BrowserDragGhosts, asset_browser_item_drag_source_with_id, asset_browser_take_release};
use crate::pickers::browser::dragging_model::{
    DragSession, DropCandidate, DropCommitResult, DropIndicator, paint_candidate, resolve_drop_position_candidate_with_selection,
};
use crate::pickers::browser::groups::ROW_INDENT;
use crate::pickers::browser::hierarchy_model::commit_hierarchy_move;
use crate::pickers::browser::hierarchy_utils::{hierarchy_drop_item_destination, hierarchy_move_sources, is_same_or_descendant, plan_hierarchy_item_moves};
use crate::pickers::browser::identity::{PickerNodeIdentity, PickerNodeRole};
use crate::pickers::browser::path_identity_model::{ContainerId, HierarchyNode, PickerBrowserHierarchy, PickerId};

const GROUP_EDGE_HEIGHT: f32 = 6.0;
const GROUP_BOUNDARY_GAP: f32 = 2.0;
const ROOT_END_MIN_HEIGHT: f32 = 32.0;

fn last_visible_rect(node: &HierarchyNode, bounds: &std::collections::BTreeMap<PickerId, Rect>) -> Option<Rect> {
    if node.is_group() {
        node.children()
            .iter()
            .filter_map(|child| last_visible_rect(child, bounds))
            .max_by(|left, right| left.bottom().total_cmp(&right.bottom()))
            .or_else(|| bounds.get(node.id()).copied())
    } else {
        bounds.get(node.id()).copied()
    }
}

/// Return the header's own middle region, excluding the semantic top and bottom edge bands.
fn group_header_body(rect: Rect, requested_edge: f32) -> Rect {
    let edge = requested_edge.max(0.0).min(rect.height() * 0.5);
    Rect::from_min_max(pos2(rect.left(), rect.top() + edge), pos2(rect.right(), rect.bottom() - edge))
}

fn group_header_edge_hits(header: Rect, after: f32, edge: f32, open: bool) -> (Rect, Rect) {
    (
        Rect::from_min_max(pos2(header.left(), header.top() - edge), pos2(header.right(), header.top() + edge)),
        if open {
            // Give the open-group trailing target a little extra reach below the
            // subtree without moving or resizing its visible insertion marker.
            Rect::from_min_max(pos2(header.left(), after), pos2(header.right(), after + edge + 3.0))
        } else {
            Rect::from_min_max(pos2(header.left(), after - edge), pos2(header.right(), after + edge))
        },
    )
}

/// The painted browser object that owns a hit area.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DropObject {
    Card(PickerId),
    GroupHeader(PickerId),
    CardRowEnd(PickerId),
    RootSequence(Option<PickerId>),
    IndentationLane(PickerId),
}

/// Meaning of a hit area relative to its owning object.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DropArea {
    Before,
    Into,
    After,
    Append,
}

/// Semantic overlap class for hit arbitration. Boundary regions are intentional split targets,
/// not artificially deep nodes; their precedence is explicit rather than encoded with MAX depth.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum DropHitPriority {
    Object,
    SiblingBoundary,
    IndentationLane,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ObjectDropHit {
    object: DropObject,
    area: DropArea,
    hit: Rect,
    indicator: Rect,
    depth: usize,
    priority: DropHitPriority,
    order: usize,
}

impl ObjectDropHit {
    fn destination(&self) -> Option<(crate::pickers::browser::path_identity_model::DropPosition, DropIndicator)> {
        use crate::pickers::browser::path_identity_model::DropPosition;

        let (position, indicator) = match (&self.object, self.area) {
            (DropObject::Card(id), DropArea::Before) | (DropObject::GroupHeader(id), DropArea::Before) => {
                (DropPosition::Before { target: id.clone() }, DropIndicator::InsertionLine { rect: self.indicator })
            }
            (DropObject::Card(id), DropArea::After) | (DropObject::CardRowEnd(id), DropArea::Append) | (DropObject::GroupHeader(id), DropArea::After) => {
                (DropPosition::After { target: id.clone() }, DropIndicator::InsertionLine { rect: self.indicator })
            }
            (DropObject::GroupHeader(id), DropArea::Into) => {
                (DropPosition::PrependTo { group: id.clone() }, DropIndicator::ContainerHighlight { rect: self.indicator })
            }
            (DropObject::RootSequence(Some(first)), DropArea::Before) => {
                (DropPosition::Before { target: first.clone() }, DropIndicator::InsertionLine { rect: self.indicator })
            }
            (DropObject::RootSequence(_), DropArea::Before | DropArea::Append) => {
                (DropPosition::AppendTo { container: ContainerId::Root }, DropIndicator::RootAppendLine { rect: self.indicator })
            }
            (DropObject::IndentationLane(id), DropArea::Append) => {
                (DropPosition::AppendTo { container: ContainerId::Group(id.clone()) }, DropIndicator::InsertionLine { rect: self.indicator })
            }
            _ => return None,
        };
        Some((position, indicator))
    }

    fn candidate(&self, source: &PickerId, selection: &[PickerId], hierarchy: &PickerBrowserHierarchy) -> Option<DropCandidate> {
        let (position, indicator) = self.destination()?;
        Some(resolve_drop_position_candidate_with_selection(source, selection, position, indicator, hierarchy))
    }
}

fn push_with_priority(drag: &BrowserDragState, object: DropObject, area: DropArea, hit: Rect, indicator: Rect, depth: usize, priority: DropHitPriority) {
    if !hit.is_positive() || !indicator.is_positive() {
        return;
    }
    let mut hits = drag.canonical_drop.object_hits.borrow_mut();
    let order = hits.len();
    hits.push(ObjectDropHit { object, area, hit, indicator, depth, priority, order });
}

fn push(drag: &BrowserDragState, object: DropObject, area: DropArea, hit: Rect, indicator: Rect, depth: usize) {
    push_with_priority(drag, object, area, hit, indicator, depth, DropHitPriority::Object);
}

fn push_boundary(drag: &BrowserDragState, object: DropObject, area: DropArea, hit: Rect, indicator: Rect, depth: usize) {
    push_with_priority(drag, object, area, hit, indicator, depth, DropHitPriority::SiblingBoundary);
}

/// Register the two edge regions owned by a card. Grid cards split horizontally; list rows split
/// vertically, so each layout follows its visible insertion axis.
pub(super) fn register_card(drag: &BrowserDragState, id: PickerId, rect: Rect, depth: usize, list: bool, row_end: Option<Rect>) {
    let (leading, trailing, leading_marker, trailing_marker) = if list {
        (
            Rect::from_min_max(rect.min, pos2(rect.right(), rect.center().y)),
            Rect::from_min_max(pos2(rect.left(), rect.center().y), rect.max),
            horizontal_marker(rect, rect.top()),
            horizontal_marker(rect, rect.bottom()),
        )
    } else {
        (
            Rect::from_min_max(rect.min, pos2(rect.center().x, rect.bottom())),
            Rect::from_min_max(pos2(rect.center().x, rect.top()), rect.max),
            vertical_marker(rect.left() - 1.0, rect),
            vertical_marker(rect.right() + 1.0, rect),
        )
    };
    push(drag, DropObject::Card(id.clone()), DropArea::Before, leading, leading_marker, depth);
    push(drag, DropObject::Card(id.clone()), DropArea::After, trailing, trailing_marker, depth);
    if let Some(hit) = row_end {
        push(drag, DropObject::CardRowEnd(id), DropArea::Append, hit, trailing_marker, depth);
    }
}

/// Register group-header top/middle/bottom areas. An open group's bottom is placed after its
/// last visible descendant, while the middle area always belongs to the header itself.
pub(super) fn register_group_objects(projection: &BrowserProjection, drag: &BrowserDragState) {
    let bounds = drag.canonical_drop.node_bounds.borrow();
    for (id, header, depth, open) in &drag.canonical_drop.group_headers {
        let body = group_header_body(*header, GROUP_EDGE_HEIGHT);
        let last_descendant = if *open {
            projection
                .hierarchy
                .node(id)
                .into_iter()
                .flat_map(|node| node.children())
                .filter_map(|child| last_visible_rect(child, &bounds))
                .max_by(|left, right| left.bottom().total_cmp(&right.bottom()))
        } else {
            None
        };
        let bottom = last_descendant.map_or(header.bottom(), |rect| rect.bottom());
        // The relocated bottom band belongs to the group, not to whichever child happens to be
        // last or how wide that child is. It spans the full group row beneath the visible subtree;
        // explicit boundary priority arbitrates any overlap with the final card's own After edge.
        let edge = GROUP_EDGE_HEIGHT;
        let group_end = bottom + if *open { GROUP_BOUNDARY_GAP } else { 0.0 };
        let (top_hit, bottom_hit) = group_header_edge_hits(*header, group_end, edge, *open);
        push_boundary(drag, DropObject::GroupHeader(id.clone()), DropArea::Before, top_hit, horizontal_marker(*header, header.top()), *depth);
        push(drag, DropObject::GroupHeader(id.clone()), DropArea::Into, body, *header, *depth);
        // Keep the visual insertion line in the actual row gap. The larger hit band is
        // intentionally lower so it can meet the trailing hit area of the final child.
        let marker_y = if *open { group_end + edge * 0.5 - 5.0 } else { bottom };
        push_boundary(drag, DropObject::GroupHeader(id.clone()), DropArea::After, bottom_hit, horizontal_marker(*header, marker_y), *depth);
    }
}

/// Turn the already-laid-out indentation lanes into group-append targets.
pub(super) fn register_indentation_objects(projection: &BrowserProjection, drag: &BrowserDragState) {
    for (name, response) in &drag.indent_drop_targets {
        let id = projection
            .hierarchy_identities
            .iter()
            .find_map(|(id, identity)| matches!(identity, PickerNodeIdentity::Group(group) if group == name).then(|| id.clone()));
        let Some(id) = id else { continue };
        let lane = response.rect;
        // The hit area remains the gutter rect, but its visual indicator follows the group's
        // full-width header row rather than inheriting the narrower hit-area bounds.
        let marker_bounds = drag.headers.iter().find(|(_, _, header_name)| header_name == name).map(|(_, rect, _)| *rect).unwrap_or(lane);
        let marker = horizontal_marker(marker_bounds, lane.bottom());
        let mut depth = 0;
        let mut parent = projection.parents.get(name);
        while let Some(parent_name) = parent {
            depth += 1;
            parent = projection.parents.get(parent_name);
        }
        push_with_priority(drag, DropObject::IndentationLane(id), DropArea::Append, lane, marker, depth, DropHitPriority::IndentationLane);
    }
}

/// Register root sequence boundaries after every visible object has reported its bounds.
pub(super) fn register_root_objects(ui: &mut egui::Ui, hierarchy: &PickerBrowserHierarchy, drag: &BrowserDragState, list: bool, parent_clip: Option<Rect>) {
    let bounds = drag.canonical_drop.node_bounds.borrow();
    if let Some(first) = hierarchy.roots().first()
        && let Some(rect) = bounds.get(first.id()).copied()
    {
        let clip = parent_clip.unwrap_or_else(|| ui.clip_rect());
        let hit = Rect::from_min_max(pos2(clip.left(), clip.top()), pos2(clip.right(), rect.top() + 1.0));
        let marker = if first.is_group() {
            horizontal_marker(rect, rect.top())
        } else if list {
            horizontal_marker(rect, rect.top())
        } else {
            vertical_marker(rect.left() - 1.0, rect)
        };
        push(drag, DropObject::RootSequence(Some(first.id().clone())), DropArea::Before, hit, marker, 0);
    } else {
        let clip = parent_clip.unwrap_or_else(|| ui.clip_rect());
        push(drag, DropObject::RootSequence(None), DropArea::Before, clip, horizontal_marker(clip, clip.top()), 0);
    }
    let clip = ui.clip_rect();
    let end_height = (clip.bottom() - ui.cursor().top()).max(ROOT_END_MIN_HEIGHT);
    let (end_rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), end_height), egui::Sense::hover());
    let end_top = end_rect.top();
    let hit = end_rect.intersect(clip);
    let marker = match hierarchy.roots().last() {
        Some(last) if last.is_group() => {
            let row = bounds.get(last.id()).copied();
            let visible_bottom = last_visible_rect(last, &bounds).map(|rect| rect.bottom());
            row.zip(visible_bottom).map(|(row, bottom)| horizontal_marker(row, bottom)).unwrap_or_else(|| horizontal_marker(clip, end_top))
        }
        Some(last) if !last.is_group() && !list => {
            bounds.get(last.id()).copied().map(|rect| vertical_marker(rect.right() + 1.0, rect)).unwrap_or_else(|| horizontal_marker(clip, end_top))
        }
        Some(last) if !last.is_group() => {
            bounds.get(last.id()).copied().map(|rect| horizontal_marker(rect, rect.bottom())).unwrap_or_else(|| horizontal_marker(clip, end_top))
        }
        _ => horizontal_marker(clip, end_top),
    };
    push(drag, DropObject::RootSequence(None), DropArea::Append, hit, marker, 0);
}

fn vertical_marker(x: f32, rect: Rect) -> Rect {
    Rect::from_min_max(pos2(x - 2.0, rect.top()), pos2(x + 2.0, rect.bottom()))
}

fn horizontal_marker(bounds: Rect, y: f32) -> Rect {
    Rect::from_min_max(pos2(bounds.left(), y - 2.0), pos2(bounds.right(), y + 2.0))
}

fn arbitrate(pointer: Pos2, hits: &[ObjectDropHit]) -> Option<usize> {
    hits.iter().enumerate().filter(|(_, hit)| hit.hit.contains(pointer)).max_by_key(|(_, hit)| (hit.priority, hit.depth, hit.order)).map(|(index, _)| index)
}

/// Resolve and commit the exact object under the pointer. Preview and release share one candidate.
pub(super) fn resolve_canonical_drop(
    ui: &egui::Ui,
    ctx: &BrowserCtx<'_>,
    projection: &mut BrowserProjection,
    drag: &BrowserDragState,
    commit: HierarchyCommit<'_, '_>,
    accent: egui::Color32,
) -> Option<Result<(), String>> {
    let pointer = ui.ctx().pointer_interact_pos()?;
    let dragged = egui::DragAndDrop::payload::<AssetDrag>(ui.ctx()).map(|payload| payload.as_ref().clone())?;
    let identities = &projection.hierarchy_identities;
    let source = source_node_for_drag(&dragged, ctx.panel, identities)?;
    let mut hits = drag.canonical_drop.object_hits.borrow().clone();
    hits.retain_mut(|hit| {
        if !ctx.capabilities.item_drop
            && matches!(&dragged, AssetDrag::Preset { .. })
            && matches!((&hit.object, hit.area), (DropObject::GroupHeader(_), DropArea::Into) | (DropObject::IndentationLane(_), DropArea::Append))
        {
            return false;
        }
        let clip = if matches!(hit.object, DropObject::RootSequence(Some(_))) && hit.area == DropArea::Before {
            commit.viewport.parent_clip
        } else {
            commit.viewport.view_clip
        };
        hit.hit = hit.hit.intersect(clip);
        hit.hit.is_positive()
    });
    let hit = hits.get(arbitrate(pointer, &hits)?)?;
    let selection = match &dragged {
        AssetDrag::Preset { selection, .. } | AssetDrag::CanonicalGroup { selection, .. } => selection.clone(),
    };
    let candidate = hit.candidate(&source, &selection, &projection.hierarchy)?;
    let layer_id = match dragged {
        AssetDrag::Preset { .. } => ("preset-drag", ctx.panel),
        AssetDrag::CanonicalGroup { .. } => ("preset-group-drag", ctx.panel),
    };
    paint_candidate(ui, &candidate, accent, commit.viewport.view_clip, egui::LayerId::new(egui::Order::Foreground, egui::Id::new(layer_id)));
    let released = asset_browser_take_release(ui.ctx())?;
    if source_node_for_drag(&released, ctx.panel, identities).as_ref() != Some(&source) {
        return Some(Err("Move rejected: the dragged source changed before release.".to_owned()));
    }

    let sources = if selection.is_empty() { vec![source.clone()] } else { selection.clone() };
    let moved_sources = hierarchy_move_sources(&sources, &candidate.position);
    let mut session = DragSession::new(source.clone(), selection.clone());
    session.candidate = Some(candidate.clone());
    let hierarchy = &mut projection.hierarchy;
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
        let hierarchy_sources = hierarchy_move_sources(&sources, &intent.position);
        if !identities.values().all(|identity| matches!(identity.role(), PickerNodeRole::Card | PickerNodeRole::AssetGroup)) {
            return Err("This asset adapter does not support path-aware hierarchy moves.".to_owned());
        }
        let item_moves = plan_hierarchy_item_moves(hierarchy, &sources, &intent.position, identities, commit.root_item_group)?;
        let mut staged_hierarchy = hierarchy.clone();
        let mut staged_state = commit.state.clone();
        for source in &hierarchy_sources {
            commit_hierarchy_move(&mut staged_state, ctx.panel, &mut staged_hierarchy, source, &intent.position, identities)
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
            select_dropped_item(commit.state, ctx.panel, &group, &path, key);
        }
        if selection.is_empty()
            && identities.get(&source).is_some_and(|identity| matches!(identity.role(), PickerNodeRole::AssetGroup | PickerNodeRole::StructuralFolder))
        {
            commit.state.select_browser_node(ctx.panel, source, false);
        }
    }
    Some(match result {
        DropCommitResult::Committed | DropCommitResult::NoCandidate => Ok(()),
        DropCommitResult::Rejected(reason) => Err(format!("Move rejected: {reason:?}")),
        DropCommitResult::Failed(error) => Err(format!("Move rejected: {error:?}")),
    })
}

/// Register the expanded groups' indentation-column drop targets.
pub(super) fn register_group_indent_drop_targets(
    ui: &mut egui::Ui,
    ctx: &BrowserCtx<'_>,
    projection: &BrowserProjection,
    drag_state: &mut BrowserDragState,
    expanded_groups: &std::collections::BTreeSet<String>,
) {
    if ctx.capabilities.group_drag || ctx.capabilities.item_drop {
        for (_group_index, header, group_name) in &drag_state.headers {
            if !expanded_groups.contains(group_name) {
                continue;
            }
            let bottom = drag_state
                .headers
                .iter()
                .filter(|(_, _, name)| name != group_name && is_same_or_descendant(&projection.parents, name, group_name))
                .map(|(_, rect, _)| rect.bottom())
                .chain(
                    drag_state
                        .group_content_areas
                        .iter()
                        .filter(|(name, rect)| rect.is_positive() && is_same_or_descendant(&projection.parents, name, group_name))
                        .map(|(_, rect)| rect.bottom()),
                )
                .max_by(f32::total_cmp);
            if let Some(bottom) = bottom {
                // The parent lane owns the gutter across the first child's leading edge as well
                // as the visible subtree below it, so the top of the lane absorbs that boundary.
                let column = Rect::from_min_max(pos2(header.left(), header.bottom() - GROUP_EDGE_HEIGHT), pos2(header.left() + ROW_INDENT, bottom));
                if column.is_positive() {
                    let response = ui.interact(column, ui.id().with(("group-indent-column", ctx.panel, group_name.as_str())), egui::Sense::hover());
                    drag_state.indent_drop_targets.push((group_name.clone(), response));
                }
            }
        }
    }
}

/// Register a card's canonical drag source and retain its rendered bounds for the object-owned edges.
pub(super) fn register_browser_item_drop_targets(
    geometry: ItemDropRegistrationGeometry<'_, '_>,
    ctx: &BrowserCtx<'_>,
    projection: &BrowserProjection,
    sink: &BrowserSink<'_>,
    drag_state: &mut BrowserDragState,
) {
    let ItemDropRegistrationGeometry { painted, .. } = geometry;
    let panel = ctx.panel;
    let capabilities = ctx.capabilities;
    let response = &painted.response;
    let rect = painted.rect;
    let ItemRef { group: item_group, item, .. } = painted.item;
    if capabilities.item_drag
        && let Some(source_id) = item_node_for_identity(&item_group.name, &item.group_path, &item.key, &projection.hierarchy_identities)
    {
        let selected_nodes = selected_drag_nodes(sink.state, panel, &projection.visible_selection_order, &source_id, &projection.hierarchy);
        asset_browser_item_drag_source_with_id(response, panel, &item.key, source_id.clone(), &selected_nodes);
    }
    if let Some(target_id) = item_node_for_identity(&item_group.name, &item.group_path, &item.key, &projection.hierarchy_identities) {
        drag_state.canonical_drop.node_bounds.borrow_mut().insert(target_id, rect);
    }
}

/// Canvas application drops must not steal releases that land anywhere in the browser viewport.
pub(crate) fn canvas_drop_target(pointer: Pos2, item: Rect, canvas: Rect, browser: Rect) -> bool {
    !item.contains(pointer) && !browser.contains(pointer) && canvas.contains(pointer)
}

/// Mutable drag/drop data collected during layout and consumed in the frame's ordered resolution tail.
#[derive(Default)]
pub(super) struct BrowserDragState {
    pub(super) headers: Vec<(usize, Rect, String)>,
    pub(super) group_content_areas: Vec<(String, Rect)>,
    pub(super) indent_drop_targets: Vec<(String, egui::Response)>,
    pub(super) dropped: Option<(String, Pos2)>,
    pub(super) ghosts: BrowserDragGhosts,
    pub(super) canonical_drop: CanonicalDropState,
}

/// Canonical hierarchy-drop geometry and hover state shared by target registration.
#[derive(Default)]
pub(super) struct CanonicalDropState {
    pub(super) group_headers: Vec<(PickerId, Rect, usize, bool)>,
    pub(super) node_bounds: std::cell::RefCell<std::collections::BTreeMap<PickerId, Rect>>,
    pub(super) object_hits: std::cell::RefCell<Vec<ObjectDropHit>>,
}

#[cfg(test)]
pub(super) fn test_drop_geometry(drag: &BrowserDragState) -> Vec<(String, DropArea, Rect, Rect)> {
    drag.canonical_drop.object_hits.borrow().iter().map(|hit| (format!("{:?}", hit.object), hit.area, hit.hit, hit.indicator)).collect()
}

pub(super) fn source_node_for_drag(drag: &AssetDrag, panel: &str, identities: &std::collections::BTreeMap<PickerId, PickerNodeIdentity>) -> Option<PickerId> {
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
    None
}

#[cfg(test)]
pub(crate) fn item_occurrence_position(groups: &[crate::pickers::browser::projection::GroupView], key: &str) -> Option<(usize, usize)> {
    groups
        .iter()
        .enumerate()
        .find_map(|(group_index, group)| group.items.iter().position(|item| item.key.as_str() == key).map(|item_index| (group_index, item_index)))
}

#[cfg(test)]
mod object_geometry_tests {
    use super::{DropArea, DropHitPriority, DropObject, ObjectDropHit, group_header_body, group_header_edge_hits};
    use crate::pickers::browser::dragging_model::DropIndicator;
    use crate::pickers::browser::path_identity_model::{DropPosition, HierarchyNode, PickerBrowserHierarchy, PickerId};
    use egui::{Pos2, Rect};

    #[test]
    fn group_header_edges_own_the_adjacent_spacing_and_open_subtree_end() {
        let header = Rect::from_min_max(Pos2::new(20.0, 40.0), Pos2::new(220.0, 70.0));
        let (top, bottom) = group_header_edge_hits(header, 150.0, 6.0, false);
        let (_, open_bottom) = group_header_edge_hits(header, 150.0, 6.0, true);
        let body = group_header_body(header, 6.0);

        assert_eq!(top, Rect::from_min_max(Pos2::new(20.0, 34.0), Pos2::new(220.0, 46.0)));
        assert_eq!(bottom, Rect::from_min_max(Pos2::new(20.0, 144.0), Pos2::new(220.0, 156.0)));
        assert_eq!(open_bottom, Rect::from_min_max(Pos2::new(20.0, 150.0), Pos2::new(220.0, 156.0)));
        assert_eq!(body, Rect::from_min_max(Pos2::new(20.0, 46.0), Pos2::new(220.0, 64.0)));
        assert!(top.contains(Pos2::new(100.0, 37.0)), "the header owns the space immediately above it");
        assert!(bottom.contains(Pos2::new(150.0, 153.0)), "an open header owns trailing space after its visible subtree");
        assert!(!open_bottom.contains(Pos2::new(150.0, 149.0)), "an open group's After region starts beneath its visible subtree");
        assert!(bottom.contains(Pos2::new(100.0, 153.0)), "the group's After area spans the full row beneath its visible subtree");
        assert!(!body.contains(Pos2::new(100.0, 45.0)), "the middle/Into area remains confined to the header");
    }

    #[test]
    fn group_header_areas_map_directly_to_before_into_and_after_positions() {
        let hierarchy =
            PickerBrowserHierarchy::new(vec![HierarchyNode::group("parent", vec![HierarchyNode::group("child", vec![])]), HierarchyNode::item("source")])
                .unwrap();
        let hit = |object, area| ObjectDropHit {
            object,
            area,
            hit: Rect::from_min_max(Pos2::ZERO, Pos2::new(10.0, 10.0)),
            indicator: Rect::from_min_max(Pos2::ZERO, Pos2::new(10.0, 2.0)),
            depth: 0,
            priority: DropHitPriority::SiblingBoundary,
            order: 0,
        };

        let before = hit(DropObject::GroupHeader(PickerId::from("child")), DropArea::Before).candidate(&PickerId::from("source"), &[], &hierarchy).unwrap();
        let into = hit(DropObject::GroupHeader(PickerId::from("parent")), DropArea::Into).candidate(&PickerId::from("source"), &[], &hierarchy).unwrap();
        let after = hit(DropObject::GroupHeader(PickerId::from("parent")), DropArea::After).candidate(&PickerId::from("source"), &[], &hierarchy).unwrap();

        assert_eq!(before.position, DropPosition::Before { target: PickerId::from("child") });
        assert_eq!(into.position, DropPosition::PrependTo { group: PickerId::from("parent") });
        assert_eq!(after.position, DropPosition::After { target: PickerId::from("parent") });
        assert!(matches!(before.indicator, DropIndicator::InsertionLine { .. }));
        assert!(matches!(into.indicator, DropIndicator::ContainerHighlight { .. }));
    }

    #[test]
    fn object_edges_cover_parent_child_and_group_card_sibling_boundaries() {
        let hierarchy = PickerBrowserHierarchy::new(vec![
            HierarchyNode::group(
                "parent",
                vec![HierarchyNode::group("first-child", vec![]), HierarchyNode::item("first-card"), HierarchyNode::item("last-card")],
            ),
            HierarchyNode::group("moving-group", vec![]),
        ])
        .unwrap();
        let hit = |object, area| ObjectDropHit {
            object,
            area,
            hit: Rect::from_min_max(Pos2::ZERO, Pos2::new(10.0, 10.0)),
            indicator: Rect::from_min_max(Pos2::ZERO, Pos2::new(10.0, 2.0)),
            depth: 0,
            priority: DropHitPriority::SiblingBoundary,
            order: 0,
        };

        let source_group = PickerId::from("moving-group");
        let first_child = hit(DropObject::GroupHeader(PickerId::from("first-child")), DropArea::Before).candidate(&source_group, &[], &hierarchy).unwrap();
        let group_before_card = hit(DropObject::Card(PickerId::from("first-card")), DropArea::Before).candidate(&source_group, &[], &hierarchy).unwrap();
        let card_after_card = hit(DropObject::Card(PickerId::from("first-card")), DropArea::After).candidate(&source_group, &[], &hierarchy).unwrap();

        assert_eq!(first_child.position, DropPosition::Before { target: PickerId::from("first-child") });
        assert_eq!(group_before_card.position, DropPosition::Before { target: PickerId::from("first-card") });
        assert_eq!(card_after_card.position, DropPosition::After { target: PickerId::from("first-card") });
        assert!(first_child.is_valid(), "the parent-to-first-child boundary is a sibling insertion in the parent");
        assert!(group_before_card.is_valid(), "a group can be inserted before a card in the receiving group");
        assert!(card_after_card.is_valid(), "a group can be inserted after a card in the receiving group");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pickers::browser::path_identity_model::DropPosition;

    #[test]
    fn object_areas_resolve_to_structural_positions() {
        let id = PickerId::from("target");
        let cases = [
            (DropObject::Card(id.clone()), DropArea::Before, DropPosition::Before { target: id.clone() }),
            (DropObject::Card(id.clone()), DropArea::After, DropPosition::After { target: id.clone() }),
            (DropObject::GroupHeader(id.clone()), DropArea::Before, DropPosition::Before { target: id.clone() }),
            (DropObject::GroupHeader(id.clone()), DropArea::Into, DropPosition::PrependTo { group: id.clone() }),
            (DropObject::GroupHeader(id.clone()), DropArea::After, DropPosition::After { target: id.clone() }),
            (DropObject::RootSequence(Some(id.clone())), DropArea::Before, DropPosition::Before { target: id.clone() }),
            (DropObject::CardRowEnd(id.clone()), DropArea::Append, DropPosition::After { target: id.clone() }),
            (DropObject::RootSequence(None), DropArea::Append, DropPosition::AppendTo { container: ContainerId::Root }),
            (DropObject::IndentationLane(id.clone()), DropArea::Append, DropPosition::AppendTo { container: ContainerId::Group(id.clone()) }),
        ];
        for (object, area, expected) in cases {
            let hit = ObjectDropHit { object, area, hit: Rect::EVERYTHING, indicator: Rect::EVERYTHING, depth: 0, priority: DropHitPriority::Object, order: 0 };
            let (position, _) = hit.destination().unwrap();
            assert_eq!(position, expected);
        }
    }

    #[test]
    fn arbitration_prefers_deeper_then_later_object() {
        let hit = |depth, order| ObjectDropHit {
            object: DropObject::RootSequence(None),
            area: DropArea::Append,
            hit: Rect::from_min_max(Pos2::ZERO, Pos2::new(20.0, 20.0)),
            indicator: Rect::EVERYTHING,
            depth,
            priority: DropHitPriority::Object,
            order,
        };
        assert_eq!(arbitrate(Pos2::new(10.0, 10.0), &[hit(0, 1), hit(1, 0), hit(1, 2)]), Some(2));
        assert_eq!(arbitrate(Pos2::new(30.0, 30.0), &[]), None);

        let mut boundary = hit(0, 0);
        boundary.priority = DropHitPriority::SiblingBoundary;
        assert_eq!(arbitrate(Pos2::new(10.0, 10.0), &[hit(99, 9), boundary]), Some(1));
    }
}
