//! Shared hit testing and interaction semantics for asset browsers.
//!
//! This module owns the egui interaction contract only: consumers translate the resulting
//! gestures and drag payloads into their own state changes or domain commands.

use egui::{Pos2, Rect};

use crate::PhotocraftApp;
use crate::theme::Tokens;

/// Canvas drop bounds used by preset-browser adapters, excluding the ruler strips.
pub(crate) fn canvas_drop_rect(app: &PhotocraftApp) -> Rect {
    crate::rulers::content_rect(app, app.last_canvas_rect)
}

/// Shared payload for dragging an asset or one of its group/folder headers.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum AssetDrag {
    Preset { scope: String, key: String },
    Group { scope: String, name: String },
    Folder { scope: String, group: String, folder: Vec<String> },
}

/// Shared header hit convention: the upper third inserts before; the rest nests inside.
pub(crate) fn header_drop_before(rect: Rect, pointer: Pos2) -> bool {
    pointer.y < rect.top() + rect.height() / 3.0
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

/// Semantic destination for dragging a group or nested folder onto a hierarchy header.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum HierarchyDrop {
    MoveGroup { group: String, folder: Vec<String>, before: Option<String> },
    MoveInto { group: String, folder: Vec<String>, into: Option<(String, Vec<String>)>, before: Option<String> },
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
    use super::{group_drop_outline_rects, group_insertion_marker_rect};
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
}

/// Resolve a hierarchy-header drop without coupling to a consumer's command type.
pub(crate) fn hierarchy_folder_drop(src_group: &str, src_folder: &[String], target_group: &str, target_folder: &[String], edge: bool) -> Option<HierarchyDrop> {
    if src_group == target_group && target_folder.starts_with(src_folder) {
        return None;
    }
    let group = src_group.to_string();
    let folder = src_folder.to_vec();
    if !edge {
        let parent = src_folder.split_last().map(|(_, p)| p);
        if src_group == target_group && parent == Some(target_folder) {
            return None;
        }
        return Some(HierarchyDrop::MoveInto { group, folder, into: Some((target_group.to_string(), target_folder.to_vec())), before: None });
    }
    match target_folder.split_last() {
        None if src_folder.is_empty() => Some(HierarchyDrop::MoveGroup { group, folder, before: Some(target_group.to_string()) }),
        None => Some(HierarchyDrop::MoveInto { group, folder, into: None, before: Some(target_group.to_string()) }),
        Some((leaf, parent)) => {
            let siblings = src_group == target_group && src_folder.split_last().map(|(_, p)| p) == Some(parent);
            if siblings {
                Some(HierarchyDrop::MoveGroup { group, folder, before: Some(leaf.clone()) })
            } else {
                Some(HierarchyDrop::MoveInto { group, folder, into: Some((target_group.to_string(), parent.to_vec())), before: Some(leaf.clone()) })
            }
        }
    }
}

/// Build the shared payload for a hierarchy header.
pub(crate) fn asset_browser_header_payload(scope: &str, group: &str, folder: &[String]) -> AssetDrag {
    if folder.is_empty() {
        AssetDrag::Group { scope: scope.to_string(), name: group.to_string() }
    } else {
        AssetDrag::Folder { scope: scope.to_string(), group: group.to_string(), folder: folder.to_vec() }
    }
}

/// Shared drag target for group/folder headers, including scoped drop indicators.
pub(crate) fn asset_browser_header_target_with_capabilities(
    ui: &egui::Ui,
    response: &egui::Response,
    rect: Rect,
    scope: &str,
    group: &str,
    folder: &[String],
    capabilities: HeaderDropCapabilities,
) -> Option<AssetDrag> {
    let HeaderDropCapabilities { item_drop, group_drag, reject_group_drop } = capabilities;
    if !item_drop && !group_drag {
        return None;
    }
    if group_drag {
        response.dnd_set_drag_payload(asset_browser_header_payload(scope, group, folder));
    }
    let hovered = response.dnd_hover_payload::<AssetDrag>().map(|payload| payload.as_ref().clone());
    if let Some(dragged) = hovered.as_ref() {
        let pointer = ui.ctx().pointer_interact_pos();
        let edge = pointer.is_some_and(|p| header_drop_before(rect, p));
        let source = match dragged {
            AssetDrag::Group { scope: drag_scope, name } if group_drag && drag_scope == scope => Some((name.as_str(), &[][..])),
            AssetDrag::Folder { scope: drag_scope, group: source_group, folder: source_folder } if group_drag && drag_scope == scope => {
                Some((source_group.as_str(), source_folder.as_slice()))
            }
            _ => None,
        };
        let hierarchy_drop = (!reject_group_drop)
            .then_some(())
            .and_then(|()| source)
            .and_then(|(source_group, source_folder)| hierarchy_folder_drop(source_group, source_folder, group, folder, edge));
        match dragged {
            AssetDrag::Preset { scope: drag_scope, .. } if item_drop && drag_scope == scope => {
                let t = Tokens::get(ui.ctx());
                paint_group_drop_outline(ui, rect, t.accent);
            }
            AssetDrag::Group { .. } | AssetDrag::Folder { .. } if group_drag && hierarchy_drop.is_some() => {
                if matches!(hierarchy_drop, Some(HierarchyDrop::MoveInto { before: None, .. })) {
                    let t = Tokens::get(ui.ctx());
                    paint_group_drop_outline(ui, rect, t.accent);
                } else {
                    paint_group_insertion_line(ui, rect, true, Tokens::get(ui.ctx()).accent);
                }
            }
            _ => {}
        }
    }
    response.dnd_release_payload::<AssetDrag>().map(|payload| payload.as_ref().clone()).filter(|drag| match drag {
        AssetDrag::Preset { scope: drag_scope, .. } => item_drop && drag_scope == scope,
        AssetDrag::Group { scope: drag_scope, name } if group_drag && drag_scope == scope && !reject_group_drop => {
            let pointer = ui.ctx().pointer_interact_pos();
            let edge = pointer.is_some_and(|p| header_drop_before(rect, p));
            hierarchy_folder_drop(name, &[], group, folder, edge).is_some()
        }
        AssetDrag::Folder { scope: drag_scope, group: source_group, folder: source_folder } if group_drag && drag_scope == scope && !reject_group_drop => {
            let pointer = ui.ctx().pointer_interact_pos();
            let edge = pointer.is_some_and(|p| header_drop_before(rect, p));
            hierarchy_folder_drop(source_group, source_folder, group, folder, edge).is_some()
        }
        _ => false,
    })
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
    response.dnd_set_drag_payload(AssetDrag::Preset { scope: scope.to_string(), key: key.to_string() });
    let hovered = response.dnd_hover_payload::<AssetDrag>().map(|payload| payload.as_ref().clone());
    if matches!(&hovered, Some(AssetDrag::Preset { scope: drag_scope, key: dragged }) if drag_scope.as_str() == scope && dragged.as_str() != key) {
        let after = ui.ctx().pointer_interact_pos().is_some_and(|p| if grid { p.x > rect.center().x } else { p.y > rect.center().y });
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
        .filter(|drag| matches!(drag, AssetDrag::Preset { scope: drag_scope, key: dragged } if drag_scope == scope && dragged != key))
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
mod hierarchy_folder_drop_tests {
    use super::{HierarchyDrop, hierarchy_folder_drop};

    fn path(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| (*part).to_string()).collect()
    }

    #[test]
    fn rejects_dropping_a_folder_on_itself_or_a_descendant() {
        let source = path(&["Brushes", "Favorites"]);

        assert_eq!(hierarchy_folder_drop("Library", &source, "Library", &source, false), None);
        assert_eq!(hierarchy_folder_drop("Library", &source, "Library", &path(&["Brushes", "Favorites", "Soft"]), true), None);
    }

    #[test]
    fn rejects_dropping_a_folder_into_its_current_parent() {
        assert_eq!(hierarchy_folder_drop("Library", &path(&["Brushes", "Favorites"]), "Library", &path(&["Brushes"]), false), None);
    }

    #[test]
    fn allows_reordering_a_folder_before_a_sibling() {
        assert_eq!(
            hierarchy_folder_drop("Library", &path(&["Brushes", "Favorites"]), "Library", &path(&["Brushes", "Recent"]), true),
            Some(HierarchyDrop::MoveGroup { group: "Library".to_string(), folder: path(&["Brushes", "Favorites"]), before: Some("Recent".to_string()) }),
        );
    }

    #[test]
    fn allows_moving_a_folder_into_another_folder() {
        assert_eq!(
            hierarchy_folder_drop("Library", &path(&["Brushes", "Favorites"]), "Library", &path(&["Textures"]), false),
            Some(HierarchyDrop::MoveInto {
                group: "Library".to_string(),
                folder: path(&["Brushes", "Favorites"]),
                into: Some(("Library".to_string(), path(&["Textures"]))),
                before: None,
            }),
        );
    }

    #[test]
    fn allows_moving_a_folder_into_another_groups_folder() {
        assert_eq!(
            hierarchy_folder_drop("Library", &path(&["Brushes", "Favorites"]), "Other", &path(&["Textures"]), false),
            Some(HierarchyDrop::MoveInto {
                group: "Library".to_string(),
                folder: path(&["Brushes", "Favorites"]),
                into: Some(("Other".to_string(), path(&["Textures"]))),
                before: None,
            }),
        );
    }
}
