//! Shared hit testing and interaction semantics for asset browsers.
//!
//! This module owns the egui interaction contract only: consumers translate the resulting
//! gestures and drag payloads into their own state changes or domain commands.

use egui::{Pos2, Rect};

use crate::PhotocraftApp;
use crate::pickers::picker_browser_dragging::{DropHitRegion, PointerRegion};
use crate::pickers::picker_browser_model::{ContainerId, NodeId};
use crate::theme::Tokens;

/// Canvas drop bounds used by preset-browser adapters, excluding the ruler strips.
pub(crate) fn canvas_drop_rect(app: &PhotocraftApp) -> Rect {
    crate::rulers::content_rect(app, app.last_canvas_rect)
}

/// Shared payload for dragging an asset or one of its group headers.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum AssetDrag {
    Preset {
        scope: String,
        key: String,
        /// Exact hierarchy identity for canonical drag arbitration. Legacy consumers may omit it.
        node_id: Option<NodeId>,
    },
    /// Any group node, including a nested group. `path` is empty for a root group.
    Group {
        scope: String,
        name: String,
        path: Vec<String>,
    },
    /// Canonical group source with an exact path-qualified node identity.
    CanonicalGroup { scope: String, node_id: NodeId },
}

/// Shared header hit convention: the upper third inserts before; the rest nests inside.
pub(crate) fn header_drop_before(rect: Rect, pointer: Pos2) -> bool {
    pointer.y < rect.top() + rect.height() / 3.0
}

/// Whether an item drop resolves after the hovered card in its row.
/// Use the same decision for the preview marker and the committed insertion index.
pub(crate) fn item_drop_after(rect: Rect, pointer: Pos2, grid: bool) -> bool {
    if grid { pointer.x > rect.center().x } else { pointer.y > rect.center().y }
}

/// Empty space at the sides of a grid row remains a valid insertion target. This makes the
/// beginning/end of a row reachable without aiming at the one-pixel gap beside a card.
pub(crate) fn item_row_edge_targets(area: Rect, item_rect: Rect, first: bool, last: bool) -> (Option<Rect>, Option<Rect>) {
    let left = first
        .then(|| Rect::from_min_max(Pos2::new(area.left(), item_rect.top()), Pos2::new(item_rect.left(), item_rect.bottom())))
        .filter(|rect| rect.is_positive());
    let right = last
        .then(|| Rect::from_min_max(Pos2::new(item_rect.right(), item_rect.top()), Pos2::new(area.right(), item_rect.bottom())))
        .filter(|rect| rect.is_positive());
    (left, right)
}

/// Capture a card's leading/trailing half as a semantic sibling insertion target. The returned
/// hit rectangle remains the card bounds; indicator geometry is a separate narrow line.
pub(crate) fn card_drop_hit(rect: Rect, pointer: Pos2, grid: bool, target: NodeId, depth: usize, order: usize) -> DropHitRegion {
    let after = item_drop_after(rect, pointer, grid);
    let indicator_rect = if grid {
        let x = if after { rect.right() } else { rect.left() };
        Rect::from_min_max(Pos2::new(x - 2.0, rect.top()), Pos2::new(x + 2.0, rect.bottom()))
    } else {
        let y = if after { rect.bottom() } else { rect.top() };
        Rect::from_min_max(Pos2::new(rect.left(), y - 2.0), Pos2::new(rect.right(), y + 2.0))
    };
    DropHitRegion {
        target: Some(target),
        container: None,
        region: if after { PointerRegion::TrailingEdge } else { PointerRegion::LeadingEdge },
        hit_rect: rect,
        indicator_rect,
        depth,
        order,
    }
}

/// Capture a group header: its leading third inserts before the group and its body appends inside.
pub(crate) fn group_header_drop_hit(
    rect: Rect,
    pointer: Pos2,
    target: NodeId,
    depth: usize,
    order: usize,
) -> DropHitRegion {
    let before = header_drop_before(rect, pointer);
    let indicator_rect = if before {
        group_insertion_marker_rect(rect, true)
    } else {
        rect
    };
    DropHitRegion {
        target: before.then_some(target.clone()),
        container: (!before).then_some(ContainerId::Group(target)),
        region: if before { PointerRegion::LeadingEdge } else { PointerRegion::GroupBody },
        hit_rect: rect,
        indicator_rect,
        depth,
        order,
    }
}

/// Capture an append target spanning a group's visible subtree (e.g. the indent column).
pub(crate) fn group_append_hit(hit_rect: Rect, indicator_rect: Rect, group: NodeId, depth: usize, order: usize) -> DropHitRegion {
    DropHitRegion {
        target: None,
        container: Some(ContainerId::Group(group)),
        region: PointerRegion::GroupBody,
        hit_rect,
        indicator_rect,
        depth,
        order,
    }
}

/// Capture the explicit end of the root sibling sequence, including an empty browser's drop area.
pub(crate) fn root_append_hit(rect: Rect, indicator_rect: Rect, order: usize) -> DropHitRegion {
    DropHitRegion {
        target: None,
        container: Some(ContainerId::Root),
        region: PointerRegion::ContainerEnd,
        hit_rect: rect,
        indicator_rect,
        depth: 0,
        order,
    }
}

/// Semantic destination for dragging a root or nested group onto a hierarchy header.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum HierarchyDrop {
    MoveGroup { group: String, group_path: Vec<String>, before: Option<String> },
    MoveInto { group: String, group_path: Vec<String>, into: Option<(String, Vec<String>)>, before: Option<String> },
}

/// Which kinds of drops a particular hierarchy header accepts.
#[derive(Clone, Copy)]
pub(crate) struct HeaderDropCapabilities {
    pub(crate) item_drop: bool,
    pub(crate) group_drag: bool,
    /// The target is inside the source group's own descendant hierarchy.
    pub(crate) reject_group_drop: bool,
}

fn paint_group_drop_outline(ui: &egui::Ui, rect: Rect, color: egui::Color32) {
    if !rect.is_positive() {
        return;
    }
    let painter = ui.painter();
    // Paint the border inward from the target's actual bounds: no outer inset or rounded corners.
    for border in group_drop_outline_rects(rect) {
        painter.rect_filled(border, egui::CornerRadius::ZERO, color);
    }
}

fn group_drop_outline_rects(rect: Rect) -> [Rect; 4] {
    [
        Rect::from_min_max(rect.min, Pos2::new(rect.right(), (rect.top() + 4.0).min(rect.bottom()))),
        Rect::from_min_max(Pos2::new(rect.left(), (rect.bottom() - 4.0).max(rect.top())), rect.max),
        Rect::from_min_max(rect.min, Pos2::new((rect.left() + 4.0).min(rect.right()), rect.bottom())),
        Rect::from_min_max(Pos2::new((rect.right() - 4.0).max(rect.left()), rect.top()), rect.max),
    ]
}

pub(crate) fn paint_group_insertion_line(ui: &egui::Ui, rect: Rect, before: bool, color: egui::Color32) {
    ui.painter().rect_filled(group_insertion_marker_rect(rect, before), egui::CornerRadius::ZERO, color);
}

fn group_insertion_marker_rect(rect: Rect, before: bool) -> Rect {
    let center_y = if before { rect.top() - 1.0 } else { rect.bottom() + 1.0 };
    Rect::from_min_max(Pos2::new(rect.left(), center_y - 2.0), Pos2::new(rect.right(), center_y + 2.0))
}

#[cfg(test)]
mod insertion_marker_tests {
    use super::{card_drop_hit, group_append_hit, group_drop_outline_rects, group_header_drop_hit, group_insertion_marker_rect, root_append_hit};
    use crate::pickers::picker_browser_model::{ContainerId, NodeId};
    use crate::pickers::picker_browser_dragging::PointerRegion;
    use egui::{Pos2, Rect};

    #[test]
    fn group_insertion_marker_is_four_pixels_tall_and_centered_in_the_gap() {
        let row = Rect::from_min_max(Pos2::new(10.0, 30.0), Pos2::new(210.0, 60.0));

        let marker = group_insertion_marker_rect(row, true);

        assert_eq!(marker.height(), 4.0);
        assert_eq!(marker.center().y, row.top() - 1.0);
    }

    #[test]
    fn group_drop_outline_is_four_pixels_thick_on_all_sides() {
        let target = Rect::from_min_max(Pos2::new(10.0, 30.0), Pos2::new(210.0, 90.0));

        let [top, bottom, left, right] = group_drop_outline_rects(target);

        assert_eq!(top.height(), 4.0);
        assert_eq!(bottom.height(), 4.0);
        assert_eq!(left.width(), 4.0);
        assert_eq!(right.width(), 4.0);
        assert_eq!(left.left(), target.left());
        assert_eq!(right.right(), target.right());
    }

    #[test]
    fn card_and_group_hit_regions_keep_hit_and_indicator_geometry_separate() {
        let card = Rect::from_min_max(Pos2::new(10.0, 30.0), Pos2::new(110.0, 70.0));
        let hit = card_drop_hit(card, Pos2::new(90.0, 40.0), true, NodeId::from("card"), 1, 2);
        assert_eq!(hit.hit_rect, card);
        assert_eq!(hit.region, PointerRegion::TrailingEdge);
        assert_eq!(hit.indicator_rect.width(), 4.0);

        let header = Rect::from_min_max(Pos2::new(10.0, 80.0), Pos2::new(210.0, 110.0));
        let before = group_header_drop_hit(header, Pos2::new(20.0, 85.0), NodeId::from("group"), 0, 0);
        assert_eq!(before.region, PointerRegion::LeadingEdge);
        assert_eq!(before.indicator_rect.width(), header.width());
        assert_eq!(before.indicator_rect.height(), 4.0);

        let body = group_header_drop_hit(header, Pos2::new(100.0, 100.0), NodeId::from("group"), 0, 0);
        assert_eq!(body.region, PointerRegion::GroupBody);
        assert_eq!(body.container, Some(ContainerId::Group(NodeId::from("group"))));
    }

    #[test]
    fn append_hit_regions_name_their_destination_container() {
        let rect = Rect::from_min_max(Pos2::new(10.0, 10.0), Pos2::new(30.0, 80.0));
        let group = group_append_hit(rect, rect, NodeId::from("group"), 2, 4);
        assert_eq!(group.region, PointerRegion::GroupBody);
        assert_eq!(group.container, Some(ContainerId::Group(NodeId::from("group"))));
        let root = root_append_hit(rect, rect, 5);
        assert_eq!(root.region, PointerRegion::ContainerEnd);
        assert_eq!(root.container, Some(ContainerId::Root));
    }
}

/// Resolve a hierarchy-header drop without coupling to a consumer's command type.
pub(crate) fn hierarchy_group_drop(src_group: &str, src_path: &[String], target_group: &str, target_path: &[String], edge: bool) -> Option<HierarchyDrop> {
    if src_group == target_group && target_path.starts_with(src_path) {
        return None;
    }
    let group = src_group.to_string();
    let group_path = src_path.to_vec();
    if !edge {
        let parent = src_path.split_last().map(|(_, p)| p);
        if src_group == target_group && parent == Some(target_path) {
            return None;
        }
        return Some(HierarchyDrop::MoveInto { group, group_path, into: Some((target_group.to_string(), target_path.to_vec())), before: None });
    }
    match target_path.split_last() {
        None if src_path.is_empty() => Some(HierarchyDrop::MoveGroup { group, group_path, before: Some(target_group.to_string()) }),
        None => Some(HierarchyDrop::MoveInto { group, group_path, into: None, before: Some(target_group.to_string()) }),
        Some((leaf, parent)) => {
            let siblings = src_group == target_group && src_path.split_last().map(|(_, p)| p) == Some(parent);
            if siblings {
                Some(HierarchyDrop::MoveGroup { group, group_path, before: Some(leaf.clone()) })
            } else {
                Some(HierarchyDrop::MoveInto { group, group_path, into: Some((target_group.to_string(), parent.to_vec())), before: Some(leaf.clone()) })
            }
        }
    }
}

/// Build the shared payload for a hierarchy header.
pub(crate) fn asset_browser_header_payload(scope: &str, group: &str, group_path: &[String]) -> AssetDrag {
    AssetDrag::Group { scope: scope.to_string(), name: group.to_string(), path: group_path.to_vec() }
}

/// Register a draggable group source without installing any legacy target/release consumer.
pub(crate) fn asset_browser_header_drag_source(response: &egui::Response, scope: &str, group: &str, group_path: &[String]) {
    response.dnd_set_drag_payload(asset_browser_header_payload(scope, group, group_path));
}

/// Register a canonical hierarchy node as a draggable header using its exact typed identity.
pub(crate) fn asset_browser_header_drag_source_with_id(response: &egui::Response, scope: &str, node_id: NodeId) {
    response.dnd_set_drag_payload(AssetDrag::CanonicalGroup { scope: scope.to_owned(), node_id });
}

/// Register a card source with its exact canonical identity (keys may repeat across groups).
pub(crate) fn asset_browser_item_drag_source_with_id(response: &egui::Response, scope: &str, key: &str, node_id: NodeId) {
    response.dnd_set_drag_payload(AssetDrag::Preset { scope: scope.to_string(), key: key.to_string(), node_id: Some(node_id) });
}

/// Take the browser's one-shot release payload after a canonical hit has won arbitration.
/// Call at most once per frame; legacy response helpers retain their existing ownership outside
/// the canonical resolver.
pub(crate) fn asset_browser_take_release(ctx: &egui::Context) -> Option<AssetDrag> {
    if !ctx.input(|input| input.pointer.any_released()) {
        return None;
    }
    egui::DragAndDrop::take_payload::<AssetDrag>(ctx).map(|payload| payload.as_ref().clone())
}

/// Shared drag target for group headers, including scoped drop indicators.
pub(crate) fn asset_browser_header_target_with_capabilities(
    ui: &egui::Ui,
    response: &egui::Response,
    rect: Rect,
    scope: &str,
    group: &str,
    group_path: &[String],
    capabilities: HeaderDropCapabilities,
) -> Option<AssetDrag> {
    let HeaderDropCapabilities { item_drop, group_drag, reject_group_drop } = capabilities;
    if !item_drop && !group_drag {
        return None;
    }
    if group_drag {
        response.dnd_set_drag_payload(asset_browser_header_payload(scope, group, group_path));
    }
    let hovered = response.dnd_hover_payload::<AssetDrag>().map(|payload| payload.as_ref().clone());
    let pointer = ui.ctx().pointer_interact_pos();
    let edge = pointer.is_some_and(|p| header_drop_before(rect, p));
    let source = match hovered.as_ref() {
        Some(AssetDrag::Group { scope: drag_scope, name, path }) if group_drag && drag_scope == scope => Some((name.as_str(), path.as_slice())),
        _ => None,
    };
    let hierarchy_drop = (!reject_group_drop)
        .then_some(())
        .and_then(|()| source)
        .and_then(|(source_group, source_path)| hierarchy_group_drop(source_group, source_path, group, group_path, edge));
    if let Some(dragged) = hovered.as_ref() {
        match dragged {
            AssetDrag::Preset { scope: drag_scope, .. } if item_drop && drag_scope == scope => {
                let t = Tokens::get(ui.ctx());
                paint_group_drop_outline(ui, rect, t.accent);
            }
            AssetDrag::Group { .. } if group_drag && hierarchy_drop.is_some() => {
                if matches!(hierarchy_drop.as_ref(), Some(HierarchyDrop::MoveInto { before: None, .. })) {
                    let t = Tokens::get(ui.ctx());
                    paint_group_drop_outline(ui, rect, t.accent);
                } else {
                    crate::widgets::drop_line(ui, rect, false, false, &Tokens::get(ui.ctx()));
                }
            }
            _ => {}
        }
    }
    let accepts_drop = match hovered.as_ref() {
        Some(AssetDrag::Preset { scope: drag_scope, .. }) => item_drop && drag_scope == scope,
        Some(AssetDrag::Group { scope: drag_scope, .. }) if group_drag && drag_scope == scope && !reject_group_drop => hierarchy_drop.is_some(),
        _ => false,
    };
    if accepts_drop { response.dnd_release_payload::<AssetDrag>().map(|payload| payload.as_ref().clone()) } else { None }
}

/// Shared item hit target and insertion indicator.
pub(crate) fn asset_browser_item_target(
    ui: &egui::Ui,
    response: &egui::Response,
    rect: Rect,
    next_rect: Option<Rect>,
    insertion_markers: &mut Vec<(Rect, bool, bool)>,
    scope: &str,
    key: &str,
    grid: bool,
) -> Option<AssetDrag> {
    response.dnd_set_drag_payload(AssetDrag::Preset { scope: scope.to_string(), key: key.to_string(), node_id: None });
    let hovered = response.dnd_hover_payload::<AssetDrag>().map(|payload| payload.as_ref().clone());
    if matches!(&hovered, Some(AssetDrag::Preset { scope: drag_scope, key: dragged, .. }) if drag_scope.as_str() == scope && dragged.as_str() != key) {
        let after = ui.ctx().pointer_interact_pos().is_some_and(|p| item_drop_after(rect, p, grid));
        // Across a row break, use the edge of the card under the pointer rather than
        // jumping to the other row. This keeps exactly one marker visible at a time.
        if after {
            if let Some(next) = next_rect.filter(|next| !grid || next.top() == rect.top()) {
                insertion_markers.push((next, false, grid));
            } else {
                insertion_markers.push((rect, true, grid));
            }
        } else {
            insertion_markers.push((rect, false, grid));
        }
    }
    response
        .dnd_release_payload::<AssetDrag>()
        .map(|payload| payload.as_ref().clone())
        .filter(|drag| matches!(drag, AssetDrag::Preset { scope: drag_scope, key: dragged, .. } if drag_scope == scope && dragged != key))
}

/// A group dragged onto an item is inserted immediately before or after that card in its
/// containing group. The returned boolean uses the same edge decision as the visible card line.
pub(crate) fn asset_browser_item_hierarchy_target(
    ui: &egui::Ui,
    response: &egui::Response,
    rect: Rect,
    grid: bool,
    accepts: bool,
) -> Option<(AssetDrag, bool)> {
    if !accepts {
        return None;
    }
    let hovered = response.dnd_hover_payload::<AssetDrag>().map(|payload| payload.as_ref().clone());
    let hierarchy_drag = matches!(hovered, Some(AssetDrag::Group { .. }));
    if hierarchy_drag {
        let after = ui.ctx().pointer_interact_pos().is_some_and(|pointer| item_drop_after(rect, pointer, grid));
        crate::widgets::drop_line(ui, rect, after, grid, &Tokens::get(ui.ctx()));
        return response.dnd_release_payload::<AssetDrag>().map(|payload| payload.as_ref().clone()).map(|drag| (drag, after));
    }
    None
}

/// Common click/double-click semantics for asset rows and cards.
pub(crate) fn asset_browser_item_gesture(response: &egui::Response, selection: bool, activation: bool) -> (bool, bool) {
    let activated = activation && response.double_clicked();
    let selected = selection && !activated && response.clicked();
    (selected, activated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_row_edge_targets_cover_only_the_empty_sides_of_first_and_last_cards() {
        let area = Rect::from_min_max(Pos2::new(10.0, 20.0), Pos2::new(310.0, 70.0));
        let first = Rect::from_min_max(Pos2::new(30.0, 25.0), Pos2::new(80.0, 65.0));
        let last = Rect::from_min_max(Pos2::new(210.0, 25.0), Pos2::new(260.0, 65.0));

        let (left, _) = item_row_edge_targets(area, first, true, false);
        let (no_left, _) = item_row_edge_targets(area, first, false, false);
        let (_, right) = item_row_edge_targets(area, last, false, true);
        let (_, no_right) = item_row_edge_targets(area, last, false, false);

        assert_eq!(left, Some(Rect::from_min_max(Pos2::new(10.0, 25.0), Pos2::new(30.0, 65.0))));
        assert_eq!(no_left, None);
        assert_eq!(right, Some(Rect::from_min_max(Pos2::new(260.0, 25.0), Pos2::new(310.0, 65.0))));
        assert_eq!(no_right, None);
    }
}

/// Generic actions returned by shared item and group context menus.
pub(crate) enum ContextMenuAction {
    Rename,
    Delete,
    MoveTo(String),
}

/// Draw a configurable item menu and return a request for the consumer to handle.
pub(crate) fn draw_item_context_menu(
    ui: &mut egui::Ui,
    rename_label: &str,
    delete_label: &str,
    move_to: Option<(&str, &[String], &str)>,
) -> Option<ContextMenuAction> {
    if ui.button(rename_label).clicked() {
        ui.close();
        return Some(ContextMenuAction::Rename);
    }
    if ui.button(delete_label).clicked() {
        ui.close();
        return Some(ContextMenuAction::Delete);
    }
    if let Some((label, group_names, current_group)) = move_to {
        let mut action = None;
        ui.menu_button(label, |ui| {
            for group in group_names.iter().filter(|name| name.as_str() != current_group) {
                if ui.button(group).clicked() {
                    action = Some(ContextMenuAction::MoveTo(group.clone()));
                    ui.close();
                }
            }
        });
        return action;
    }
    None
}

/// Draw a group's rename/delete menu.
pub(crate) fn draw_group_context_menu(ui: &mut egui::Ui, rename_label: &str, delete_label: &str) -> Option<ContextMenuAction> {
    if ui.button(rename_label).clicked() {
        ui.close();
        Some(ContextMenuAction::Rename)
    } else if ui.button(delete_label).clicked() {
        ui.close();
        Some(ContextMenuAction::Delete)
    } else {
        None
    }
}

/// Attach a consumer-provided context menu to an egui response.
pub(crate) fn context_menu_target(resp: egui::Response, menu: impl FnOnce(&mut egui::Ui)) {
    let _ = resp.context_menu(menu);
}

#[cfg(test)]
mod hierarchy_group_drop_tests {
    use super::{HierarchyDrop, hierarchy_group_drop};

    fn path(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| (*part).to_string()).collect()
    }

    #[test]
    fn rejects_dropping_a_group_on_itself_or_a_descendant() {
        let source = path(&["Brushes", "Favorites"]);

        assert_eq!(hierarchy_group_drop("Library", &source, "Library", &source, false), None);
        assert_eq!(hierarchy_group_drop("Library", &source, "Library", &path(&["Brushes", "Favorites", "Soft"]), true), None);
    }

    #[test]
    fn rejects_dropping_a_group_into_its_current_parent() {
        assert_eq!(hierarchy_group_drop("Library", &path(&["Brushes", "Favorites"]), "Library", &path(&["Brushes"]), false), None);
    }

    #[test]
    fn allows_reordering_a_group_before_a_sibling() {
        assert_eq!(
            hierarchy_group_drop("Library", &path(&["Brushes", "Favorites"]), "Library", &path(&["Brushes", "Recent"]), true),
            Some(HierarchyDrop::MoveGroup { group: "Library".to_string(), group_path: path(&["Brushes", "Favorites"]), before: Some("Recent".to_string()) }),
        );
    }

    #[test]
    fn allows_moving_a_group_into_another_group() {
        assert_eq!(
            hierarchy_group_drop("Library", &path(&["Brushes", "Favorites"]), "Library", &path(&["Textures"]), false),
            Some(HierarchyDrop::MoveInto {
                group: "Library".to_string(),
                group_path: path(&["Brushes", "Favorites"]),
                into: Some(("Library".to_string(), path(&["Textures"]))),
                before: None,
            }),
        );
    }

    #[test]
    fn allows_moving_a_group_into_another_root_groups_nested_group() {
        assert_eq!(
            hierarchy_group_drop("Library", &path(&["Brushes", "Favorites"]), "Other", &path(&["Textures"]), false),
            Some(HierarchyDrop::MoveInto {
                group: "Library".to_string(),
                group_path: path(&["Brushes", "Favorites"]),
                into: Some(("Other".to_string(), path(&["Textures"]))),
                before: None,
            }),
        );
    }
}
