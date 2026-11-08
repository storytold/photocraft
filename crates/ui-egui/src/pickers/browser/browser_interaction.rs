//! Shared hit testing and interaction semantics for asset browsers.
//!
//! This module owns pointer hit testing, low-level interaction semantics, and
//! drag-ghost capture/presentation. Context-menu policy lives in `context_menu`,
//! while hierarchy target arbitration and release resolution live in `dragging`.

use egui::{Pos2, Rect, pos2};

use crate::PhotocraftApp;
use crate::pickers::browser::asset_browser::{BrowserAction, BrowserCtx, BrowserSink, GroupView, ItemPaintState, PaintedCell, Painters};
use crate::pickers::browser::path_identity_model::PickerId;
#[cfg(test)]
use crate::pickers::state::PresetUi;
use crate::theme::Tokens;

pub(super) use super::context_menu::{BrowserDeleteContext, replace_selected_delete_actions};
pub(crate) use super::context_menu::{
    ContextMenuAction, begin_item_rename, delete_item_actions, draw_context_menu_target, draw_group_context_menu, draw_item_context_menu, item_context_targets,
    move_item_actions, standard_group_context_menu, standard_item_context_menu,
};

/// Source identity for a floating drag preview.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum BrowserDragGhostSource {
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

/// Keep a dragged card/header pressed against the browser viewport edges while the pointer is inside it.
pub(crate) fn browser_drag_ghost_rect(
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
        AssetDrag::Preset { scope, .. } | AssetDrag::CanonicalGroup { scope, .. } => scope == panel,
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

pub(crate) fn browser_drag_ghost_position(pointer: Option<Pos2>, origin: Rect, grab_offset: egui::Vec2, picker_bounds: Rect) -> Rect {
    let Some(pointer) = pointer else { return origin };
    let pointer_rect = Rect::from_min_size(pointer - grab_offset, origin.size());
    if picker_bounds.contains(pointer) { clamp_drag_ghost(pointer_rect, picker_bounds) } else { pointer_rect }
}

pub(crate) fn clamp_drag_ghost(rect: Rect, bounds: Rect) -> Rect {
    let max_x = (bounds.right() - rect.width()).max(bounds.left());
    let max_y = (bounds.bottom() - rect.height()).max(bounds.top());
    let min = pos2(rect.left().clamp(bounds.left(), max_x), rect.top().clamp(bounds.top(), max_y));
    Rect::from_min_size(min, rect.size())
}

/// Drag previews are collected during layout and painted together outside the scroll viewport.
#[derive(Default)]
pub(super) struct BrowserDragGhosts {
    pub(super) groups: Vec<GroupDragGhost>,
    pub(super) nested_groups: Vec<NestedGroupDragGhost>,
    pub(super) items: Vec<ItemDragGhost>,
}

pub(super) struct GroupDragGhost {
    pub(super) rect: Rect,
    pub(super) group_index: usize,
    pub(super) open: bool,
    pub(super) depth: usize,
}

pub(super) struct NestedGroupDragGhost {
    pub(super) rect: Rect,
    pub(super) open: bool,
    pub(super) name: String,
    pub(super) depth: usize,
    pub(super) count: usize,
}

pub(super) struct ItemDragGhost {
    pub(super) rect: Rect,
    pub(super) group_index: usize,
    pub(super) item_index: usize,
    pub(super) display_mode: crate::pickers::state::PickerDisplayMode,
    pub(super) selected: bool,
}

const GROUP_DRAG_GHOST_OPACITY: f32 = 0.4;

impl Painters<'_, '_> {
    /// Paint drag previews outside the scroll viewport using the adapter's preview callbacks.
    pub(super) fn paint_drag_ghosts(
        &mut self,
        ui: &egui::Ui,
        panel: &str,
        groups: &[GroupView],
        app_window_clip: Rect,
        ghosts: BrowserDragGhosts,
        tokens: &Tokens,
    ) {
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
                crate::pickers::browser::groups::paint_with_opacity(&ghost_ui, ghost_rect, open, &group.name, depth, None, GROUP_DRAG_GHOST_OPACITY);
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
            crate::pickers::browser::groups::paint_with_opacity(&ghost_ui, ghost_rect, open, &name, depth, Some(count), GROUP_DRAG_GHOST_OPACITY);
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
            ghost_ui.set_opacity(GROUP_DRAG_GHOST_OPACITY);
            if ghost_mode == crate::pickers::state::PickerDisplayMode::TextOnly {
                if let Some(item) = groups.get(group_index).and_then(|group| group.items.get(item_index)) {
                    if selected {
                        ghost_ui.painter().rect_filled(ghost_rect, egui::CornerRadius::ZERO, tokens.row_selected);
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
                    let thumb_rect = Rect::from_min_size(ghost_rect.min + egui::vec2(1.0, 1.0), egui::vec2(thumb_size, thumb_size));
                    self.item(&ghost_ui, thumb_rect, group_index, item_index, ItemPaintState::DragGhost);
                    ghost_ui.painter().text(
                        pos2(thumb_rect.right() + 8.0, ghost_rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        &item.name,
                        egui::FontId::proportional(11.5),
                        tokens.text,
                    );
                }
            } else {
                self.item(&ghost_ui, ghost_rect, group_index, item_index, ItemPaintState::DragGhost);
            }
            ghost_ui.painter().rect_stroke(ghost_rect, egui::CornerRadius::ZERO, egui::Stroke::new(2.0, tokens.accent), egui::StrokeKind::Middle);
        }
    }
}

#[cfg(test)]
pub(crate) fn advance_item_rename(st: &mut PresetUi, panel: &str) {
    if st.item_renaming.is_none()
        && let Some(index) = st.item_rename_queue.iter().position(|rename| rename.panel == panel)
    {
        st.item_renaming = Some(st.item_rename_queue.remove(index));
    }
}

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
        /// Exact hierarchy identity for canonical drag arbitration.
        node_id: Option<PickerId>,
        /// Selected item identities in visible browser order. Empty means a single-item drag.
        selection: Vec<PickerId>,
    },
    /// Canonical group source with an exact path-qualified node identity and browser selection.
    CanonicalGroup { scope: String, node_id: PickerId, label: String, selection: Vec<PickerId> },
}

impl AssetDrag {
    /// Return the path-qualified canonical source identity for this browser scope.
    #[cfg(test)]
    pub(crate) fn canonical_node_id(&self, expected_scope: &str) -> Option<&PickerId> {
        match self {
            Self::Preset { scope, node_id: Some(node_id), .. } | Self::CanonicalGroup { scope, node_id, .. } if scope == expected_scope => Some(node_id),
            _ => None,
        }
    }
}

/// Empty space after the last card in a grid row remains a valid append target.
/// Prepend is deliberately limited to the leading half of the card itself.
pub(crate) fn item_row_edge_targets(area: Rect, item_rect: Rect, last: bool) -> Option<Rect> {
    last.then(|| Rect::from_min_max(Pos2::new(item_rect.right(), item_rect.top()), Pos2::new(area.right(), item_rect.bottom())))
        .filter(|rect| rect.is_positive())
}

/// Register a draggable group source without installing any legacy target/release consumer.
/// Register a canonical hierarchy node as a draggable header using its exact typed identity.
pub(crate) fn asset_browser_header_drag_source_with_id(response: &egui::Response, scope: &str, node_id: PickerId, label: &str, selection: &[PickerId]) {
    response.dnd_set_drag_payload(AssetDrag::CanonicalGroup { scope: scope.to_owned(), node_id, label: label.to_owned(), selection: selection.to_vec() });
}

/// Register a card source with its exact canonical identity (keys may repeat across groups).
pub(crate) fn asset_browser_item_drag_source_with_id(response: &egui::Response, scope: &str, key: &str, node_id: PickerId, selection: &[PickerId]) {
    let selection = selection.to_vec();
    response.dnd_set_drag_payload(AssetDrag::Preset { scope: scope.to_string(), key: key.to_string(), node_id: Some(node_id), selection });
}

/// Take the browser's one-shot release payload after a canonical hit has won arbitration.
/// Call at most once per frame.
pub(crate) fn asset_browser_take_release(ctx: &egui::Context) -> Option<AssetDrag> {
    if !ctx.input(|input| input.pointer.any_released()) {
        return None;
    }
    egui::DragAndDrop::take_payload::<AssetDrag>(ctx).map(|payload| payload.as_ref().clone())
}

/// Common click/double-click semantics for asset rows and cards.
pub(crate) fn asset_browser_item_gesture(response: &egui::Response, selection: bool, activation: bool) -> (bool, bool) {
    let activated = activation && response.double_clicked();
    let selected = selection && !activated && response.clicked();
    (selected, activated)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_row_edge_targets_cover_only_the_empty_space_after_the_last_card() {
        let area = Rect::from_min_max(Pos2::new(10.0, 20.0), Pos2::new(310.0, 70.0));
        let first = Rect::from_min_max(Pos2::new(30.0, 25.0), Pos2::new(80.0, 65.0));
        let last = Rect::from_min_max(Pos2::new(210.0, 25.0), Pos2::new(260.0, 65.0));

        let no_right = item_row_edge_targets(area, first, false);
        let right = item_row_edge_targets(area, last, true);

        assert_eq!(no_right, None);
        assert_eq!(right, Some(Rect::from_min_max(Pos2::new(260.0, 25.0), Pos2::new(310.0, 65.0))));
    }
}

#[cfg(test)]
mod canonical_drag_tests {
    use super::AssetDrag;
    use crate::pickers::browser::path_identity_model::PickerId;

    #[test]
    fn canonical_source_identity_is_scope_checked_and_path_qualified() {
        let item = AssetDrag::Preset {
            scope: "brushes".to_owned(),
            key: "soft".to_owned(),
            node_id: Some(PickerId::group_path(["Favorites"], "soft")),
            selection: Vec::new(),
        };
        let group_selection = vec![PickerId::group("Favorites"), PickerId::item("Other", "soft")];
        let group = AssetDrag::CanonicalGroup {
            scope: "brushes".to_owned(),
            node_id: PickerId::group_path(["Favorites"], "Favorites"),
            label: "Favorites".to_owned(),
            selection: group_selection.clone(),
        };

        assert_eq!(item.canonical_node_id("brushes"), Some(&PickerId::group_path(["Favorites"], "soft")));
        assert_eq!(group.canonical_node_id("brushes"), Some(&PickerId::group_path(["Favorites"], "Favorites")));
        assert!(matches!(&group, AssetDrag::CanonicalGroup { selection, .. } if selection == &group_selection));
        assert_eq!(item.canonical_node_id("patterns"), None);
    }
}
