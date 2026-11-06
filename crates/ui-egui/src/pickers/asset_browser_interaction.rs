//! Shared hit testing and interaction semantics for asset browsers.
//!
//! This module owns pointer hit testing, low-level interaction semantics, inline rename, context
//! menus, and drag-ghost capture/presentation. Hierarchy target arbitration and release resolution
//! live in `asset_browser_dragging`.

use std::collections::BTreeSet;

use egui::{Pos2, Rect, pos2};

use crate::PhotocraftApp;
use crate::pickers::asset_browser::{BrowserAction, GroupView, ItemContextMenuArgs, ItemPaintState, ItemView, Painters};
use crate::pickers::picker_browser_dragging::{DropHitRegion, PointerRegion};
use crate::pickers::picker_browser_identity::PickerNodeIdentity;
use crate::pickers::picker_browser_model::{ContainerId, HierarchyNode, PickerBrowserHierarchy, PickerId};
use crate::pickers::picker_state::{ItemRenameState, PresetUi};
use crate::theme::Tokens;

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
    pub(super) display_mode: crate::pickers::picker_state::PickerDisplayMode,
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
            ghost_ui.set_opacity(GROUP_DRAG_GHOST_OPACITY);
            if ghost_mode == crate::pickers::picker_state::PickerDisplayMode::TextOnly {
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

/// Draw the browser's inline rename row and return the resulting generic actions.
pub(crate) fn draw_rename_row(ui: &mut egui::Ui, state: &mut PresetUi, panel: &str, tokens: &Tokens) -> (f32, Vec<BrowserAction>) {
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

pub(crate) fn advance_item_rename(st: &mut PresetUi, panel: &str) {
    if st.item_renaming.is_none()
        && let Some(index) = st.item_rename_queue.iter().position(|rename| rename.panel == panel)
    {
        st.item_renaming = Some(st.item_rename_queue.remove(index));
    }
}

/// Attach a context menu, using a nested submenu when the browser is inside a compact picker menu.
pub(crate) fn draw_context_menu_target(ui: &mut egui::Ui, response: egui::Response, compact: bool, draw_menu: impl FnOnce(&mut egui::Ui)) {
    if compact {
        compact_context_menu_target(ui, response, draw_menu);
    } else {
        context_menu_target(response, draw_menu);
    }
}

fn compact_context_menu_target(ui: &mut egui::Ui, response: egui::Response, menu: impl FnOnce(&mut egui::Ui)) {
    // Compact pickers live inside an egui menu popup. A separate context popup is treated as
    // outside that parent and dismisses it. Use the same nested-SubMenu mechanism as the gear.
    let submenu_id = egui::containers::menu::SubMenu::id_from_widget_id(response.id);
    // Do not set `open_item` before calling `SubMenu::show`: MenuState validates that item
    // against the previous frame's visible submenu, so a newly requested submenu gets cleared.
    // Showing it on the right-click frame lets SubMenu register itself normally and persist.
    let right_clicked = response.secondary_clicked();
    let open = egui::containers::menu::MenuState::from_ui(ui, |state, _| state.open_item == Some(submenu_id));
    if right_clicked || open {
        egui::containers::menu::SubMenu::new().show(ui, &response, menu);
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
        /// Exact hierarchy identity for canonical drag arbitration. Legacy consumers may omit it.
        node_id: Option<PickerId>,
        /// Selected item identities in visible browser order. Empty means a single-item drag.
        selection: Vec<PickerId>,
    },
    /// Any group node, including a nested group. `path` is empty for a root group.
    Group { scope: String, name: String, path: Vec<String> },
    /// Canonical group source with an exact path-qualified node identity and browser selection.
    CanonicalGroup { scope: String, node_id: PickerId, label: String, selection: Vec<PickerId> },
}

impl AssetDrag {
    /// Return the path-qualified canonical source identity for this browser scope.
    /// Legacy name/path payloads intentionally do not produce canonical move requests.
    #[cfg(test)]
    pub(crate) fn canonical_node_id(&self, expected_scope: &str) -> Option<&PickerId> {
        match self {
            Self::Preset { scope, node_id: Some(node_id), .. } | Self::CanonicalGroup { scope, node_id, .. } if scope == expected_scope => Some(node_id),
            _ => None,
        }
    }
}

/// Shared header hit convention for legacy hierarchy adapters: dropping on a group header means
/// dropping into that group. Sibling reordering is handled by the gaps between headers.
pub(crate) fn header_drop_before(_rect: Rect, _pointer: Pos2) -> bool {
    false
}

/// Whether an item drop resolves after the hovered card in its row.
/// Use the same decision for the preview marker and the committed insertion index.
pub(crate) fn item_drop_after(rect: Rect, pointer: Pos2, grid: bool) -> bool {
    if grid { pointer.x > rect.center().x } else { pointer.y > rect.center().y }
}

/// Empty space after the last card in a grid row remains a valid append target.
/// Prepend is deliberately limited to the leading half of the card itself.
pub(crate) fn item_row_edge_targets(area: Rect, item_rect: Rect, last: bool) -> Option<Rect> {
    last.then(|| Rect::from_min_max(Pos2::new(item_rect.right(), item_rect.top()), Pos2::new(area.right(), item_rect.bottom())))
        .filter(|rect| rect.is_positive())
}

/// Capture a card's leading/trailing half as a semantic sibling insertion target. The returned
/// hit rectangle remains the card bounds; indicator geometry is a separate narrow line.
pub(crate) fn card_drop_hit(rect: Rect, pointer: Pos2, grid: bool, target: PickerId, depth: usize, order: usize) -> DropHitRegion {
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

/// Register both halves of a card so the debug view and hit arbitration use the same full-time
/// geometry even when the pointer is not currently over the card.
pub(crate) fn card_drop_hits(rect: Rect, grid: bool, target: PickerId, depth: usize, order: usize) -> [DropHitRegion; 2] {
    let (leading, trailing) = if grid {
        let middle = rect.center().x;
        (Rect::from_min_max(rect.min, Pos2::new(middle, rect.bottom())), Rect::from_min_max(Pos2::new(middle, rect.top()), rect.max))
    } else {
        let middle = rect.center().y;
        (Rect::from_min_max(rect.min, Pos2::new(rect.right(), middle)), Rect::from_min_max(Pos2::new(rect.left(), middle), rect.max))
    };
    let leading_hit = card_drop_hit(
        rect,
        if grid { Pos2::new(rect.left(), rect.center().y) } else { Pos2::new(rect.center().x, rect.top()) },
        grid,
        target.clone(),
        depth,
        order,
    );
    let trailing_hit = card_drop_hit(
        rect,
        if grid { Pos2::new(rect.right(), rect.center().y) } else { Pos2::new(rect.center().x, rect.bottom()) },
        grid,
        target,
        depth,
        order.saturating_add(1),
    );
    [DropHitRegion { hit_rect: leading, ..leading_hit }, DropHitRegion { hit_rect: trailing, ..trailing_hit }]
}

/// An open group's entire header receives drops into the group, including its lower half.
/// Closed groups retain the lower sibling-after target because they have no visible body.
pub(crate) fn group_header_drop_hits(rect: Rect, target: PickerId, depth: usize, order: usize, open: bool) -> Vec<DropHitRegion> {
    if open {
        return vec![DropHitRegion {
            target: None,
            container: Some(ContainerId::Group(target)),
            region: PointerRegion::GroupBody,
            hit_rect: rect,
            indicator_rect: rect,
            depth,
            order,
        }];
    }
    let middle = rect.center().y;
    let after_marker = Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - 2.0), Pos2::new(rect.right(), rect.bottom() + 2.0));
    vec![
        DropHitRegion {
            target: None,
            container: Some(ContainerId::Group(target.clone())),
            region: PointerRegion::GroupBody,
            hit_rect: Rect::from_min_max(rect.min, Pos2::new(rect.right(), middle)),
            indicator_rect: rect,
            depth,
            order,
        },
        DropHitRegion {
            target: Some(target),
            container: None,
            region: PointerRegion::TrailingEdge,
            hit_rect: Rect::from_min_max(Pos2::new(rect.left(), middle), rect.max),
            indicator_rect: after_marker,
            depth,
            order: order.saturating_add(1),
        },
    ]
}

/// Capture a drop-at-end target spanning a group's visible subtree (e.g. the indent column).
pub(crate) fn group_append_hit(hit_rect: Rect, indicator_rect: Rect, group: PickerId, depth: usize, order: usize) -> DropHitRegion {
    DropHitRegion { target: None, container: Some(ContainerId::Group(group)), region: PointerRegion::ContainerEnd, hit_rect, indicator_rect, depth, order }
}

/// Split the vertical band before a group into upper- and lower-level drop targets.
/// The upper half appends after the preceding sibling; the lower half inserts before the following
/// group. Each half paints over its own adjacent element's width.
pub(crate) struct GroupGapDropSpec {
    pub(crate) rect: Rect,
    pub(crate) upper: PickerId,
    pub(crate) lower: PickerId,
    pub(crate) upper_bounds: Rect,
    pub(crate) lower_bounds: Rect,
    pub(crate) upper_depth: usize,
    pub(crate) lower_depth: usize,
    pub(crate) order: usize,
}

pub(crate) fn group_gap_drop_hits(spec: GroupGapDropSpec) -> Vec<DropHitRegion> {
    let GroupGapDropSpec { rect, upper, lower, upper_bounds, lower_bounds, upper_depth, lower_depth, order } = spec;
    debug_assert!(rect.height() >= 2.0);
    let middle = rect.center().y;
    // The six-pixel gap is the hit area, not the marker thickness. Both halves resolve to the
    // same sibling boundary, so both markers belong on the lower header's top edge, not at the
    // gap center.
    let marker_top = middle - 2.0;
    let marker_bottom = middle + 2.0;
    let upper_line = Rect::from_min_max(Pos2::new(upper_bounds.left(), marker_top + 3.0), Pos2::new(upper_bounds.right(), marker_bottom + 3.0));
    let lower_line = Rect::from_min_max(Pos2::new(lower_bounds.left(), marker_top + 3.0), Pos2::new(lower_bounds.right(), marker_bottom + 3.0));
    vec![
        DropHitRegion {
            target: Some(upper),
            container: None,
            region: PointerRegion::TrailingEdge,
            hit_rect: Rect::from_min_max(rect.min, Pos2::new(rect.right(), middle)),
            indicator_rect: upper_line,
            depth: upper_depth,
            order,
        },
        DropHitRegion {
            target: Some(lower),
            container: None,
            region: PointerRegion::LeadingEdge,
            hit_rect: Rect::from_min_max(Pos2::new(rect.left(), middle), rect.max),
            indicator_rect: lower_line,
            depth: lower_depth,
            order: order.saturating_add(1),
        },
    ]
}

/// Trailing-edge marker for appending after a card, oriented for its layout.
pub(crate) fn card_trailing_marker_rect(rect: Rect, grid: bool) -> Rect {
    if grid {
        let x = rect.right();
        Rect::from_min_max(Pos2::new(x - 2.0, rect.top()), Pos2::new(x + 2.0, rect.bottom()))
    } else {
        let y = rect.bottom();
        Rect::from_min_max(Pos2::new(rect.left(), y - 2.0), Pos2::new(rect.right(), y + 2.0))
    }
}

/// Vertical leading-edge marker for inserting before a card in the grid layout.
pub(crate) fn card_leading_marker_rect(rect: Rect) -> Rect {
    let x = rect.left();
    Rect::from_min_max(Pos2::new(x - 2.0, rect.top()), Pos2::new(x + 2.0, rect.bottom()))
}

/// Return the last visible hierarchy node immediately before a lower group in depth-first order.
/// Its parent is the meeting container, so inserting after it preserves the node's sibling level.
pub(crate) fn group_gap_upper_anchor(hierarchy: &PickerBrowserHierarchy, lower_group: &PickerId) -> Option<(PickerId, usize)> {
    fn flatten(nodes: &[HierarchyNode], depth: usize, out: &mut std::collections::BTreeMap<PickerId, usize>) {
        for node in nodes {
            out.insert(node.id().clone(), depth);
            if node.is_group() {
                flatten(node.children(), depth + 1, out);
            }
        }
    }
    let mut depths = std::collections::BTreeMap::new();
    flatten(hierarchy.roots(), 0, &mut depths);
    let parent = hierarchy.parent_of(lower_group)?;
    let siblings = hierarchy.children_of(&parent)?;
    let index = siblings.iter().position(|node| node.id() == lower_group)?;
    if let Some(previous) = index.checked_sub(1).and_then(|previous| siblings.get(previous)) {
        return Some((previous.id().clone(), *depths.get(previous.id())?));
    }
    match parent {
        ContainerId::Group(parent) => Some((parent.clone(), *depths.get(&parent)?)),
        ContainerId::Root => None,
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
        depth: usize::MAX,
        order,
    }
}

/// Capture the leading edge of the root sibling sequence, immediately before its first node.
pub(crate) fn root_prepend_hit(rect: Rect, indicator_rect: Rect, first: PickerId, order: usize) -> DropHitRegion {
    DropHitRegion {
        target: Some(first),
        container: None,
        region: PointerRegion::LeadingEdge,
        hit_rect: rect,
        indicator_rect,
        // This one-pixel boundary overlaps the first node's own leading-edge hit. It is the
        // root sibling boundary, so it must win arbitration over that card/header body edge.
        depth: usize::MAX,
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
    use super::{
        GroupGapDropSpec, card_drop_hit, card_drop_hits, card_trailing_marker_rect, group_append_hit, group_drop_outline_rects, group_gap_drop_hits,
        group_gap_upper_anchor, group_header_drop_hits, group_insertion_marker_rect, root_append_hit, root_prepend_hit,
    };
    use crate::pickers::picker_browser_dragging::{DropIndicator, PointerRegion, resolve_drop_candidate};
    use crate::pickers::picker_browser_model::{ContainerId, DropPosition, HierarchyNode, PickerBrowserHierarchy, PickerId};
    use egui::{Pos2, Rect};

    #[test]
    fn group_insertion_marker_is_four_pixels_tall_and_centered_in_the_gap() {
        let row = Rect::from_min_max(Pos2::new(10.0, 30.0), Pos2::new(210.0, 60.0));

        let marker = group_insertion_marker_rect(row, true);

        assert_eq!(marker.height(), 4.0);
        assert_eq!(marker.center().y, row.top() - 1.0);
    }

    #[test]
    fn card_trailing_marker_matches_grid_or_list_orientation() {
        let card = Rect::from_min_max(Pos2::new(40.0, 20.0), Pos2::new(140.0, 80.0));
        let grid_marker = card_trailing_marker_rect(card, true);
        assert_eq!(grid_marker, Rect::from_min_max(Pos2::new(138.0, 20.0), Pos2::new(142.0, 80.0)));
        let list_marker = card_trailing_marker_rect(card, false);
        assert_eq!(list_marker, Rect::from_min_max(Pos2::new(40.0, 78.0), Pos2::new(140.0, 82.0)));
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
        let hit = card_drop_hit(card, Pos2::new(90.0, 40.0), true, PickerId::from("card"), 1, 2);
        assert_eq!(hit.hit_rect, card);
        assert_eq!(hit.region, PointerRegion::TrailingEdge);
        assert_eq!(hit.indicator_rect.width(), 4.0);

        let halves = card_drop_hits(card, true, PickerId::from("card"), 1, 2);
        assert_eq!(halves[0].region, PointerRegion::LeadingEdge);
        assert_eq!(halves[0].hit_rect.width(), 50.0);
        assert_eq!(halves[1].region, PointerRegion::TrailingEdge);
        assert_eq!(halves[1].hit_rect.width(), 50.0);

        let header = Rect::from_min_max(Pos2::new(10.0, 80.0), Pos2::new(210.0, 110.0));
        let [into, after] = group_header_drop_hits(header, PickerId::from("group"), 0, 0, false).try_into().unwrap();
        assert_eq!(into.region, PointerRegion::GroupBody);
        assert_eq!(into.container, Some(ContainerId::Group(PickerId::from("group"))));
        assert_eq!(into.hit_rect, Rect::from_min_max(header.min, Pos2::new(header.right(), header.center().y)));
        assert_eq!(into.indicator_rect, header);
        assert_eq!(after.region, PointerRegion::TrailingEdge);
        assert_eq!(after.target, Some(PickerId::from("group")));
        assert_eq!(after.container, None);
        assert_eq!(after.hit_rect, Rect::from_min_max(Pos2::new(header.left(), header.center().y), header.max));
        assert_eq!(after.indicator_rect, Rect::from_min_max(Pos2::new(header.left(), header.bottom() - 2.0), Pos2::new(header.right(), header.bottom() + 2.0)));
        assert_eq!(after.indicator_rect.width(), header.width());
        assert_eq!(after.order, into.order + 1);

        let open = group_header_drop_hits(header, PickerId::from("group"), 0, 0, true);
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].region, PointerRegion::GroupBody);
        assert_eq!(open[0].container, Some(ContainerId::Group(PickerId::from("group"))));
        assert_eq!(open[0].hit_rect, header, "an open group has no lower sibling drop zone");
    }

    #[test]
    fn card_edge_indicators_cover_both_halves_in_grid_and_list_layouts() {
        let card = Rect::from_min_max(Pos2::new(10.0, 30.0), Pos2::new(110.0, 70.0));
        for grid in [false, true] {
            let (leading_pointer, trailing_pointer) =
                if grid { (Pos2::new(11.0, 50.0), Pos2::new(109.0, 50.0)) } else { (Pos2::new(50.0, 31.0), Pos2::new(50.0, 69.0)) };
            let leading = card_drop_hit(card, leading_pointer, grid, PickerId::from("card"), 2, 4);
            let trailing = card_drop_hit(card, trailing_pointer, grid, PickerId::from("card"), 2, 4);
            let halves = card_drop_hits(card, grid, PickerId::from("card"), 2, 4);

            assert_eq!(leading.region, PointerRegion::LeadingEdge);
            assert_eq!(trailing.region, PointerRegion::TrailingEdge);
            assert_eq!(leading.indicator_rect, halves[0].indicator_rect);
            assert_eq!(trailing.indicator_rect, halves[1].indicator_rect);
            assert_eq!(leading.hit_rect, card);
            assert_eq!(trailing.hit_rect, card);
            assert_eq!(leading.indicator_rect.width(), if grid { 4.0 } else { card.width() });
            assert_eq!(leading.indicator_rect.height(), if grid { card.height() } else { 4.0 });
            assert_eq!(trailing.indicator_rect.width(), if grid { 4.0 } else { card.width() });
            assert_eq!(trailing.indicator_rect.height(), if grid { card.height() } else { 4.0 });
            assert_eq!(halves[0].hit_rect.union(halves[1].hit_rect), card);
            assert_eq!(halves[0].order, 4);
            assert_eq!(halves[1].order, 5);
        }
    }

    #[test]
    fn group_gap_halves_use_the_adjacent_elements_widths() {
        let gap = Rect::from_min_max(Pos2::new(10.0, 100.0), Pos2::new(210.0, 106.0));
        let upper_bounds = Rect::from_min_max(Pos2::new(31.0, 40.0), Pos2::new(131.0, 80.0));
        let lower_bounds = Rect::from_min_max(Pos2::new(31.0, 106.0), Pos2::new(231.0, 135.0));
        let split = group_gap_drop_hits(GroupGapDropSpec {
            rect: gap,
            upper: PickerId::from("upper"),
            lower: PickerId::from("lower"),
            upper_bounds,
            lower_bounds,
            upper_depth: 0,
            lower_depth: 1,
            order: 4,
        });
        assert_eq!(split.len(), 2);
        assert_eq!(split[0].region, PointerRegion::TrailingEdge);
        assert_eq!(split[0].target, Some(PickerId::from("upper")));
        assert_eq!(split[0].container, None);
        assert!(split[0].hit_rect.is_positive());
        assert_eq!(split[0].hit_rect.min, gap.min);
        assert_eq!(split[0].hit_rect.max, Pos2::new(gap.right(), gap.center().y));
        assert_eq!(split[0].hit_rect.height(), 3.0);
        assert_eq!(split[1].region, PointerRegion::LeadingEdge);
        assert_eq!(split[1].target, Some(PickerId::from("lower")));
        assert_eq!(split[1].hit_rect.height(), 3.0);
        assert_eq!(split[0].indicator_rect, Rect::from_min_max(Pos2::new(31.0, 104.0), Pos2::new(131.0, 108.0)));
        assert_eq!(split[1].indicator_rect, Rect::from_min_max(Pos2::new(31.0, 104.0), Pos2::new(231.0, 108.0)));
        assert_eq!(split[0].indicator_rect.height(), 4.0);
        assert_eq!(split[1].indicator_rect.height(), 4.0);

        for (upper_depth, lower_depth) in [(0, 1), (0, 0), (1, 0)] {
            let split = group_gap_drop_hits(GroupGapDropSpec {
                rect: gap,
                upper: PickerId::from("upper"),
                lower: PickerId::from("lower"),
                upper_bounds,
                lower_bounds,
                upper_depth,
                lower_depth,
                order: 4,
            });
            assert_eq!(split.len(), 2);
            assert_eq!(split[0].region, PointerRegion::TrailingEdge);
            assert_eq!(split[0].target, Some(PickerId::from("upper")));
            assert_eq!(split[0].container, None);
            assert_eq!(split[1].region, PointerRegion::LeadingEdge);
            assert_eq!(split[1].target, Some(PickerId::from("lower")));
        }

        // Nested groups use the same child-level append anchor.
        let nested = group_gap_drop_hits(GroupGapDropSpec {
            rect: gap,
            upper: PickerId::from("upper-folder"),
            lower: PickerId::from("child-folder"),
            upper_bounds,
            lower_bounds,
            upper_depth: 1,
            lower_depth: 2,
            order: 4,
        });
        assert_eq!(nested[0].region, PointerRegion::TrailingEdge);
        assert_eq!(nested[0].target, Some(PickerId::from("upper-folder")));
        assert_eq!(nested[0].container, None);
    }

    #[test]
    fn six_pixel_append_gap_resolves_two_exact_three_pixel_zones() {
        for left in [0.0, 13.0, 127.0] {
            for top in [0.0, 41.0, 900.0] {
                for width in [1.0, 64.0, 200.0, 1024.0] {
                    let gap = Rect::from_min_size(Pos2::new(left, top), egui::vec2(width, 6.0));
                    let upper_bounds = Rect::from_min_size(Pos2::new(left + 3.0, top - 40.0), egui::vec2(width * 0.4, 30.0));
                    let lower_bounds = Rect::from_min_size(Pos2::new(left + 3.0, top + 6.0), egui::vec2(width * 0.8, 29.0));
                    let hits = group_gap_drop_hits(GroupGapDropSpec {
                        rect: gap,
                        upper: PickerId::from("upper"),
                        lower: PickerId::from("lower"),
                        upper_bounds,
                        lower_bounds,
                        upper_depth: 0,
                        lower_depth: 1,
                        order: 8,
                    });
                    assert_eq!(hits.len(), 2);
                    assert_eq!(hits[0].hit_rect, Rect::from_min_max(gap.min, Pos2::new(gap.right(), top + 3.0)));
                    assert_eq!(hits[1].hit_rect, Rect::from_min_max(Pos2::new(left, top + 3.0), gap.max));
                    assert_eq!(hits[0].hit_rect.height(), 3.0);
                    assert_eq!(hits[1].hit_rect.height(), 3.0);
                    assert_eq!(hits[0].indicator_rect, Rect::from_min_max(Pos2::new(left + 3.0, top + 4.0), Pos2::new(left + 3.0 + width * 0.4, top + 8.0)));
                    assert_eq!(hits[1].indicator_rect, Rect::from_min_max(Pos2::new(left + 3.0, top + 4.0), Pos2::new(left + 3.0 + width * 0.8, top + 8.0)));
                    assert_eq!(hits[0].indicator_rect.height(), 4.0);
                    assert_eq!(hits[1].indicator_rect.height(), 4.0);
                    assert_eq!(hits[0].depth, 0);
                    assert_eq!(hits[1].depth, 1);
                    assert_eq!(hits[0].order + 1, hits[1].order);

                    let hierarchy = PickerBrowserHierarchy::new(vec![
                        HierarchyNode::group("upper", vec![HierarchyNode::item("upper-card")]),
                        HierarchyNode::group("lower", vec![HierarchyNode::item("lower-card")]),
                        HierarchyNode::item("moving-card"),
                    ])
                    .unwrap();
                    let upper = resolve_drop_candidate(&PickerId::from("moving-card"), &hits[0], &hierarchy).unwrap();
                    let lower = resolve_drop_candidate(&PickerId::from("moving-card"), &hits[1], &hierarchy).unwrap();
                    assert_eq!(upper.position, DropPosition::After { target: PickerId::from("upper") });
                    assert_eq!(lower.position, DropPosition::Before { target: PickerId::from("lower") });
                    assert_eq!(upper.indicator, DropIndicator::InsertionLine { rect: hits[0].indicator_rect });
                    assert_eq!(lower.indicator, DropIndicator::InsertionLine { rect: hits[1].indicator_rect });
                    assert!(upper.is_valid());
                    assert!(lower.is_valid());
                }
            }
        }
    }

    #[test]
    fn group_gap_anchor_uses_the_actual_previous_mixed_sibling() {
        use crate::pickers::picker_browser_model::{HierarchyNode as Node, PickerBrowserHierarchy};
        let hierarchy = PickerBrowserHierarchy::new(vec![Node::group(
            "outer",
            vec![
                Node::group("inner", vec![Node::group("child", vec![]), Node::item("first"), Node::item("last")]),
                Node::item("meeting-card"),
                Node::group("lower", vec![]),
            ],
        )])
        .unwrap();
        assert_eq!(group_gap_upper_anchor(&hierarchy, &PickerId::from("lower")), Some((PickerId::from("meeting-card"), 1)));
        assert_eq!(group_gap_upper_anchor(&hierarchy, &PickerId::from("inner")), Some((PickerId::from("outer"), 0)));
        let empty = PickerBrowserHierarchy::new(vec![Node::group("outer", vec![Node::group("empty", vec![])])]).unwrap();
        assert_eq!(group_gap_upper_anchor(&empty, &PickerId::from("empty")), Some((PickerId::from("outer"), 0)));
    }

    #[test]
    fn group_gap_upper_anchor_tracks_the_previous_group_or_card_at_each_nesting_level() {
        use crate::pickers::picker_browser_model::HierarchyNode as Node;

        let hierarchy = PickerBrowserHierarchy::new(vec![Node::group(
            "root",
            vec![
                Node::item("root-first"),
                Node::group("nested", vec![Node::item("nested-first"), Node::group("deep", vec![]), Node::item("nested-last")]),
                Node::item("root-last"),
                Node::group("lower-root", vec![]),
            ],
        )])
        .unwrap();

        assert_eq!(group_gap_upper_anchor(&hierarchy, &PickerId::from("lower-root")), Some((PickerId::from("root-last"), 1)));
        assert_eq!(group_gap_upper_anchor(&hierarchy, &PickerId::from("deep")), Some((PickerId::from("nested-first"), 2)));
    }

    #[test]
    fn group_body_and_root_end_hits_name_their_destination_container() {
        let rect = Rect::from_min_max(Pos2::new(10.0, 10.0), Pos2::new(30.0, 80.0));
        let group = group_append_hit(rect, rect, PickerId::from("group"), 2, 4);
        assert_eq!(group.region, PointerRegion::ContainerEnd);
        assert_eq!(group.container, Some(ContainerId::Group(PickerId::from("group"))));
        let root = root_append_hit(rect, rect, 5);
        assert_eq!(root.region, PointerRegion::ContainerEnd);
        assert_eq!(root.container, Some(ContainerId::Root));
    }

    #[test]
    fn root_prepend_boundary_wins_the_overlap_with_the_first_node() {
        use crate::pickers::picker_browser_dragging::arbitrate_hit_index;

        let first = Rect::from_min_max(Pos2::new(10.0, 20.0), Pos2::new(210.0, 50.0));
        let pointer = Pos2::new(80.0, 20.0);
        let node_edge = card_drop_hit(first, pointer, false, PickerId::from("first"), 0, 0);
        let root_edge = root_prepend_hit(first, first, PickerId::from("first"), 1);

        assert_eq!(root_edge.target, Some(PickerId::from("first")));
        assert_eq!(root_edge.region, PointerRegion::LeadingEdge);
        assert_eq!(root_edge.depth, usize::MAX);
        assert_eq!(arbitrate_hit_index(pointer, &[node_edge, root_edge]), Some(1));
    }

    #[test]
    fn card_drop_midpoint_is_the_leading_half_in_both_layouts() {
        let card = Rect::from_min_max(Pos2::new(10.0, 20.0), Pos2::new(110.0, 60.0));
        let grid_midpoint = Pos2::new(card.center().x, card.center().y);
        let list_midpoint = Pos2::new(card.center().x, card.center().y);

        assert_eq!(card_drop_hit(card, grid_midpoint, true, PickerId::from("card"), 0, 0).region, PointerRegion::LeadingEdge);
        assert_eq!(card_drop_hit(card, list_midpoint, false, PickerId::from("card"), 0, 0).region, PointerRegion::LeadingEdge);
        assert_eq!(
            card_drop_hit(card, Pos2::new(card.center().x + 0.01, card.center().y), true, PickerId::from("card"), 0, 0).region,
            PointerRegion::TrailingEdge
        );
        assert_eq!(
            card_drop_hit(card, Pos2::new(card.center().x, card.center().y + 0.01), false, PickerId::from("card"), 0, 0).region,
            PointerRegion::TrailingEdge
        );
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
        .and(source)
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
                    let t = Tokens::get(ui.ctx());
                    // Header insertion lines span the complete header width. The generic card
                    // marker deliberately leaves a four-pixel end-cap inset, which reads as a
                    // rectangular notch on a full-width hierarchy row. Paint on the foreground
                    // layer as well, so header glyphs and later row content cannot cover it.
                    let layer = egui::LayerId::new(egui::Order::Foreground, ui.layer_id().id);
                    let painter = ui.ctx().layer_painter(layer).with_clip_rect(ui.clip_rect());
                    let y = rect.top();
                    painter.rect_filled(
                        Rect::from_min_max(Pos2::new(rect.left(), y - 1.0), Pos2::new(rect.right(), y + 1.0)),
                        egui::CornerRadius::ZERO,
                        t.accent,
                    );
                    painter.rect_filled(
                        Rect::from_min_max(Pos2::new(rect.left(), rect.top()), Pos2::new(rect.left() + 2.0, rect.bottom())),
                        egui::CornerRadius::ZERO,
                        t.accent,
                    );
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

/// Inputs for the shared item hit target and insertion indicator.
pub(crate) struct ItemDropTarget<'a> {
    pub(crate) response: &'a egui::Response,
    pub(crate) rect: Rect,
    pub(crate) next_rect: Option<Rect>,
    pub(crate) insertion_markers: &'a mut Vec<(Rect, bool, bool)>,
    pub(crate) scope: &'a str,
    pub(crate) key: &'a str,
    pub(crate) grid: bool,
}

/// Shared item hit target and insertion indicator.
pub(crate) fn asset_browser_item_target(ui: &egui::Ui, target: ItemDropTarget<'_>) -> Option<AssetDrag> {
    let ItemDropTarget { response, rect, next_rect, insertion_markers, scope, key, grid } = target;
    response.dnd_set_drag_payload(AssetDrag::Preset { scope: scope.to_string(), key: key.to_string(), node_id: None, selection: Vec::new() });
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
    use crate::pickers::picker_browser_model::PickerId;

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
        let legacy = AssetDrag::Group { scope: "brushes".to_owned(), name: "Favorites".to_owned(), path: Vec::new() };

        assert_eq!(item.canonical_node_id("brushes"), Some(&PickerId::group_path(["Favorites"], "soft")));
        assert_eq!(group.canonical_node_id("brushes"), Some(&PickerId::group_path(["Favorites"], "Favorites")));
        assert!(matches!(&group, AssetDrag::CanonicalGroup { selection, .. } if selection == &group_selection));
        assert_eq!(item.canonical_node_id("patterns"), None);
        assert_eq!(legacy.canonical_node_id("brushes"), None);
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

/// Projection data needed to expand a context-menu deletion to the active browser selection.
pub(super) struct BrowserDeleteContext<'a> {
    pub(super) selection_order: &'a [PickerId],
    pub(super) hierarchy: &'a PickerBrowserHierarchy,
    pub(super) identities: &'a std::collections::BTreeMap<PickerId, PickerNodeIdentity>,
}

/// Resolve selected nodes to independent domain deletions, omitting descendants covered by a selected group.
pub(super) fn selected_delete_actions(selection: &BTreeSet<PickerId>, clicked: &PickerId, context: &BrowserDeleteContext<'_>) -> Option<Vec<BrowserAction>> {
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

pub(super) fn replace_selected_delete_actions(
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

pub(crate) fn begin_item_rename(state: &mut PresetUi, panel: &str, targets: &[(String, ItemView)]) {
    let mut renames = targets.iter().map(|(group, item)| ItemRenameState {
        panel: panel.to_owned(),
        group: group.clone(),
        group_path: item.group_path.clone(),
        key: item.key.clone(),
        text: item.name.clone(),
    });
    state.item_renaming = renames.next();
    state.item_rename_queue = renames.collect();
}

/// Standard item-menu policy shared by preset panels and specialised adapters.
pub(crate) fn standard_item_context_menu(ui: &mut egui::Ui, state: &mut PresetUi, args: ItemContextMenuArgs<'_>, events: &mut Vec<BrowserAction>) {
    let ItemContextMenuArgs { panel, item, group, group_names, selected_items } = args;
    let targets = item_context_targets(item, group, selected_items);
    let count = targets.len();
    let delete_label = if count > 1 { format!("{} ({count})", tl!("Delete")) } else { tl!("Delete").to_string() };
    let all_in_current_group = targets.iter().all(|(source_group, _)| source_group == &group.name);
    match draw_item_context_menu(ui, tl!("Rename…"), &delete_label, Some((tl!("Move to"), group_names, if all_in_current_group { &group.name } else { "" })))
    {
        Some(ContextMenuAction::Rename) => begin_item_rename(state, panel, &targets),
        Some(ContextMenuAction::Delete) => events.extend(delete_item_actions(&targets)),
        Some(ContextMenuAction::MoveTo(destination)) => events.extend(move_item_actions(&targets, &destination)),
        None => {}
    }
}

/// Standard group-menu policy shared by preset panels and specialised adapters.
pub(crate) fn standard_group_context_menu(ui: &mut egui::Ui, state: &mut PresetUi, panel: &str, group: &GroupView, events: &mut Vec<BrowserAction>) {
    match draw_group_context_menu(ui, tl!("Rename Group…"), tl!("Delete Group")) {
        Some(ContextMenuAction::Rename) => state.renaming = Some((panel.to_string(), format!("group:{}", group.name), group.name.clone())),
        Some(ContextMenuAction::Delete) => events.push(BrowserAction::DeleteGroup(group.name.clone())),
        _ => {}
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
