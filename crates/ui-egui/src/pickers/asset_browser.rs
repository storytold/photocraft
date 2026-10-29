//! Asset-neutral picker browser model, rendering, and interaction routing.
//!
//! Consumers project their domain assets into `GroupView` and `ItemView`, provide preview painters,
//! and translate `BrowserAction` into domain commands. This module owns the common browser presentation only.

use std::collections::BTreeSet;

use egui::{Align2, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, pos2, vec2};

use crate::pickers::asset_browser_groups::group_open;
use crate::pickers::asset_browser_hierarchy::{HierarchyFolder, draw_hierarchy, hierarchy_tree};
use crate::pickers::asset_browser_interaction::{
    AssetDrag, ContextMenuAction, HeaderDropCapabilities, HierarchyDrop, asset_browser_header_target_with_capabilities, asset_browser_item_gesture,
    asset_browser_item_target, context_menu_target, draw_group_context_menu, draw_item_context_menu, header_drop_before, hierarchy_folder_drop,
    item_row_edge_targets, paint_group_insertion_line,
};
use crate::pickers::asset_browser_items::{item_info as asset_browser_item_info, list_row as asset_browser_list_row};
use crate::pickers::picker_state::PresetUi;
use crate::pickers::picker_state::escape_hierarchy_key_part;
use crate::theme::Tokens;

pub(crate) const MIN_THUMB: f32 = 20.0;
const GROUP_HEADER_HEIGHT: f32 = 30.0;
const GROUP_DRAG_GHOST_OPACITY: f32 = 0.4;

/// Asset-neutral item identity and label consumed by the shared picker browser.
pub(crate) struct ItemView {
    /// Stable consumer-defined key used for selection and emitted events.
    pub(crate) key: String,
    /// Display label shown in the item card or list row.
    pub(crate) name: String,
    /// Optional path beneath the containing group. Empty paths keep existing flat libraries flat.
    pub(crate) folder: Vec<String>,
}

/// Asset-neutral group and its ordered items, ready for rendering by the shared browser.
pub(crate) struct GroupView {
    /// Group label and identity used for collapse state and group actions.
    pub(crate) name: String,
    /// Items in the order they should appear within this group.
    pub(crate) items: Vec<ItemView>,
}

/// Test helper for the shared browser's group/item/folder query contract.
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
                .map(|item| ItemView { key: item.key.clone(), name: item.name.clone(), folder: item.folder.clone() })
                .collect();
            group_visible_for_query(&query, group_matches, !items.is_empty()).then(|| GroupView { name: group.name.clone(), items })
        })
        .collect()
}

fn group_matches_query(group: &GroupView, query: &str) -> bool {
    !query.is_empty() && group.name.to_lowercase().contains(query)
}

fn item_matches_query(item: &ItemView, query: &str) -> bool {
    !query.is_empty() && (item.name.to_lowercase().contains(query) || item.folder.iter().any(|part| part.to_lowercase().contains(query)))
}

fn group_visible_for_query(query: &str, group_matches: bool, has_matching_items: bool) -> bool {
    query.is_empty() || group_matches || has_matching_items
}

fn item_visible_for_query(item: &ItemView, query: &str, group_matches: bool) -> bool {
    query.is_empty() || group_matches || item_matches_query(item, query)
}

fn item_occurrence_position(groups: &[GroupView], key: &str) -> Option<(usize, usize)> {
    groups
        .iter()
        .enumerate()
        .find_map(|(group_index, group)| group.items.iter().position(|item| item.key.as_str() == key).map(|item_index| (group_index, item_index)))
}

/// Generic browser interactions; each asset consumer translates these into its own commands.
#[derive(Clone)]
pub(crate) enum BrowserAction {
    /// Select an item without activating or dismissing its picker.
    Select(String),
    /// Activate an item, typically on a double-click.
    Activate(String),
    /// Drop an item onto a canvas position in screen coordinates.
    Drop(String, Pos2),
    /// Rename an item from its stable key to a new display name.
    Rename(String, String),
    /// Delete the item identified by its stable key.
    Delete(String),
    /// Move an item into the named group.
    Move(String, String),
    /// Move an item into a group at a specific final insertion index.
    MoveItem(String, String, Option<usize>),
    /// Rename a group from its current name to a new name.
    RenameGroup(String, String),
    /// Delete a group and its contents.
    DeleteGroup(String),
    /// Rename a nested folder identified by its owning group and full path.
    RenameFolder { group: String, folder: Vec<String>, text: String },
    /// Delete a nested folder and its contents, identified by its owning group and full path.
    DeleteFolder { group: String, folder: Vec<String> },
    /// Request creation of a new group.
    NewGroup,
    /// Request creation of a new item.
    New,
    /// Move an item into a nested folder; an empty path means directly into the group.
    /// `before` identifies the destination sibling to insert before; `None` appends.
    MoveItemToFolder { key: String, group: String, folder: Vec<String>, before: Option<String> },
    /// Reorder a group or nested folder among siblings. An empty `folder` identifies the group;
    /// `before=None` appends it after its siblings.
    MoveGroup { group: String, folder: Vec<String>, before: Option<String> },
    /// Move a group or nested folder into a destination hierarchy. `into=None` promotes it to a
    /// top-level group; an empty destination folder means directly inside that group. `before=None`
    /// appends within the destination.
    MoveInto { group: String, folder: Vec<String>, into: Option<(String, Vec<String>)>, before: Option<String> },
}

fn hierarchy_action(drop: HierarchyDrop) -> BrowserAction {
    match drop {
        HierarchyDrop::MoveGroup { group, folder, before } => BrowserAction::MoveGroup { group, folder, before },
        HierarchyDrop::MoveInto { group, folder, into, before } => BrowserAction::MoveInto { group, folder, into, before },
    }
}

fn group_is_descendant(st: &PresetUi, panel: &str, source: &str, target: &str) -> bool {
    let Some(parents) = st.group_parent.get(panel) else { return false };
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
/// `flush` lays the grid out as Photoshop's Swatches panel: the cells touch, square-cornered and
/// without a border of their own, so no screen is spent on gaps (#5103); gradients, patterns and
/// the other preset grids keep their [`GRID_GAP`] and card borders.
#[derive(Clone, Copy)]
pub(crate) struct BrowserOptions {
    /// Canvas hit region used to distinguish drops onto the canvas from drops into the browser.
    pub(crate) canvas: Rect,
    /// Use touching, square-cornered grid cells rather than spaced preset cards.
    pub(crate) flush: bool,
    /// Centre a partially filled grid row in the available item area.
    pub(crate) center_grid: bool,
    /// Left inset for grid items before nested-folder indentation is added.
    pub(crate) grid_indent: f32,
    /// Start every group expanded rather than only the first group.
    pub(crate) all_groups_open: bool,
    /// Use nested menu submenus for context actions inside a parent popup.
    pub(crate) compact_context_menus: bool,
    /// Optional fixed grid-cell dimensions for consumers with custom preview geometry.
    pub(crate) item_size: Option<egui::Vec2>,
    /// Optional allocated list-row height. Defaults to 24px when not set.
    pub(crate) list_item_height: Option<f32>,
    /// Emit group/folder moves as domain actions instead of applying transient browser ordering.
    pub(crate) domain_hierarchy_actions: bool,
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
    /// Stable key for this consumer's selection, sizing, and folder-collapse state.
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
    /// Optional context-menu policy for nested folder headers. Receives the owning group and full folder path.
    pub(crate) folder_context_menu: Option<FolderContextMenu>,
    /// Optional override for group-row visuals; the default is a folder row with disclosure.
    pub(crate) paint_group: Option<&'a mut PaintGroup>,
    /// Consumer-specific item painter. The state distinguishes regular selection styling from a drag ghost.
    pub(crate) paint_item: &'a mut PaintItem<'a>,
    /// Optional specialised list-row painter. It paints the full row; shared code retains all hit targets.
    pub(crate) paint_list_item: Option<&'a mut PaintListItem<'a>>,
}

/// Consumer callback that appends generic events for actions offered on an item.
pub(crate) type ItemContextMenu = fn(&mut egui::Ui, &mut PresetUi, &str, &ItemView, &GroupView, &[String], &mut Vec<BrowserAction>);
/// Consumer callback that appends generic events for actions offered on a group header.
pub(crate) type GroupContextMenu = fn(&mut egui::Ui, &mut PresetUi, &str, &GroupView, &mut Vec<BrowserAction>);
/// Consumer callback for rename/delete operations on a nested folder, with its full path.
pub(crate) type FolderContextMenu = fn(&mut egui::Ui, &mut PresetUi, &str, &GroupView, &[String], &mut Vec<BrowserAction>);

/// Default menu policy for the standard preset panels. Keeping this separate from the browser
/// lets specialised consumers supply their own operations without forking the shared hit targets.
pub(crate) fn standard_item_context_menu(
    ui: &mut egui::Ui,
    st: &mut PresetUi,
    panel: &str,
    item: &ItemView,
    group: &GroupView,
    group_names: &[String],
    ev: &mut Vec<BrowserAction>,
) {
    match draw_item_context_menu(ui, tl!("Rename…"), tl!("Delete"), Some((tl!("Move to"), group_names, &group.name))) {
        Some(ContextMenuAction::Rename) => st.renaming = Some((panel.to_string(), item.key.clone(), item.name.clone())),
        Some(ContextMenuAction::Delete) => ev.push(BrowserAction::Delete(item.key.clone())),
        Some(ContextMenuAction::MoveTo(group)) => ev.push(BrowserAction::Move(item.key.clone(), group)),
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
type FolderRelease = (AssetDrag, String, Vec<String>, bool);

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
        grid_indent,
        all_groups_open,
        compact_context_menus,
        item_size,
        list_item_height,
        domain_hierarchy_actions,
    } = adapter.options;
    let capabilities = adapter.capabilities;
    let item_context_menu = adapter.item_context_menu;
    let group_context_menu = adapter.group_context_menu;
    let folder_context_menu = adapter.folder_context_menu;
    let paint_group = &mut adapter.paint_group;
    let paint_item = &mut adapter.paint_item;
    let paint_list_item = &mut adapter.paint_list_item;
    let max_h = allocated_height;
    let mut ev = Vec::new();
    let size = *st.sizes.get(panel).unwrap_or(&40.0);
    let list = size <= MIN_THUMB + 0.5;
    let selected = st.selected.get(panel).cloned();
    let rename_row_top = ui.cursor().top();
    let folder_rename = st.folder_renaming.clone().filter(|rename| rename.panel == panel);
    let has_rename_row = folder_rename.is_some() || st.renaming.as_ref().is_some_and(|r| r.0 == panel);
    // Rename field (Photoshop opens a small dialog; inline keeps the panel modeless).
    if let Some(mut rename) = folder_rename {
        // The typed folder target is authoritative while this editor is active. Remove any stale
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
        st.folder_renaming = Some(rename.clone());
        if let Some(ok) = done {
            st.folder_renaming = None;
            if ok && !text.trim().is_empty() {
                ev.push(BrowserAction::RenameFolder { group: rename.group, folder: rename.folder, text: text.trim().to_string() });
            }
        }
    } else if let Some((p, key, mut text)) = st.renaming.clone().filter(|r| r.0 == panel) {
        // Starting a regular item/group rename supersedes any pending folder target for this panel.
        if st.folder_renaming.as_ref().is_some_and(|rename| rename.panel == panel) {
            st.folder_renaming = None;
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
    let mut dropped: Option<(String, Pos2)> = None;
    let mut browser_item_drop = false;
    // Folder drag (#3658): the order the folders are drawn in, and what the pointer is doing this
    // frame (the insertion line and the drop are resolved once every header is known).
    let mut drag: Option<(usize, Pos2)> = None;
    let mut released: Option<(usize, Pos2)> = None;
    let mut folder_releases: Vec<FolderRelease> = Vec::new();
    let group_names: Vec<String> = groups.iter().map(|g| g.name.clone()).collect();
    let display = st.nested_display_order(panel, &group_names);
    // The browser is the flexible middle region of a picker panel. Keep it at the allocated
    // height even when the library is short; otherwise the footer floats up and each consumer's
    // panel geometry depends on how many presets happen to be installed.
    let scroll = egui::ScrollArea::vertical()
        .id_salt(("preset-scroll", panel))
        .max_height(scroll_height)
        .min_scrolled_height(scroll_height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
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
            for &(gi, depth) in &display {
                let Some(g) = groups.get(gi) else { continue };
                let group_matches = group_matches_query(g, &query);
                let has_matching_items = g.items.iter().any(|item| item_matches_query(item, &query));
                if !group_visible_for_query(&query, group_matches, has_matching_items) {
                    continue;
                }
                let parent = st.group_parent.get(panel).and_then(|m| m.get(&g.name));
                if parent.is_some_and(|p| hidden_groups.contains(p)) {
                    hidden_groups.insert(g.name.clone());
                    continue;
                }
                // A non-empty query should reveal matching items even when their group was
                // collapsed before the search began.
                let open = !query.is_empty() || group_open(st, panel, gi, &g.name, all_groups_open);
                if !open {
                    hidden_groups.insert(g.name.clone());
                }
                let (hr, hresp) = if let Some(paint) = paint_group.as_deref_mut() {
                    let (r, allocated) = ui.allocate_exact_size(vec2(ui.available_width(), GROUP_HEADER_HEIGHT), Sense::hover());
                    let response = ui.interact(inset_row_rect(r, depth), allocated.id, Sense::click_and_drag());
                    paint(ui, r, g, open, response.hovered() || response.dragged(), 1.0);
                    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
                    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::CollapsingHeader, true, &g.name));
                    (r, response)
                } else {
                    asset_browser_header(ui, &g.name, open, depth, None, GROUP_HEADER_HEIGHT)
                };
                if capabilities.group_drag && hresp.is_pointer_button_down_on() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                } else if capabilities.group_drag && hresp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                let header_released = asset_browser_header_target_with_capabilities(
                    ui,
                    &hresp,
                    inset_row_rect(hr, depth),
                    panel,
                    &g.name,
                    &[],
                    HeaderDropCapabilities {
                        item_drop: capabilities.item_drop,
                        group_drag: capabilities.group_drag,
                        reject_group_drop: hresp.dnd_hover_payload::<AssetDrag>().is_some_and(|payload| {
                            matches!(payload.as_ref(), AssetDrag::Group { scope, name } if scope == panel && group_is_descendant(st, panel, name, &g.name))
                        }),
                    },
                );
                if hresp.clicked() {
                    let k = format!("{panel}/{}", escape_hierarchy_key_part(&g.name));
                    if !st.toggled.remove(&k) {
                        st.toggled.insert(k);
                    }
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
                        AssetDrag::Preset { scope, key } if scope.as_str() == panel => {
                            ev.push(BrowserAction::MoveItem(key, g.name.clone(), None));
                            browser_item_drop = true;
                        }
                        // Top-level group source headers are handled once by the final release
                        // resolver below, which also knows the between-header insertion order.
                        AssetDrag::Group { scope, .. } if scope.as_str() == panel && domain_hierarchy_actions => {}
                        AssetDrag::Folder { scope, group, folder } if scope.as_str() == panel && domain_hierarchy_actions => {
                            let edge = ui.ctx().pointer_interact_pos().is_some_and(|p| header_drop_before(hr, p));
                            if let Some(action) = hierarchy_folder_drop(&group, &folder, &g.name, &[], edge) {
                                ev.push(hierarchy_action(action));
                            }
                        }
                        _ => {}
                    }
                }
                if capabilities.group_drag
                    && hresp.dragged()
                    && let Some(p) = ui.ctx().pointer_interact_pos()
                {
                    drag = Some((gi, p));
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                    // Reuse the actual header painter on a tooltip layer so the drag ghost stays
                    // visually identical to the painted row, including consumer-specific styling.
                    let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::new(("preset-group-drag", panel)));
                    let gr = Rect::from_center_size(p, vec2(hr.width(), hr.height()));
                    let ghost_ui = ui.new_child(egui::UiBuilder::new().layer_id(layer).max_rect(gr));
                    if let Some(paint) = paint_group.as_deref_mut() {
                        paint(&ghost_ui, gr, g, open, false, GROUP_DRAG_GHOST_OPACITY);
                    } else {
                        crate::pickers::asset_browser_groups::paint_with_opacity(&ghost_ui, gr, open, &g.name, depth, None, GROUP_DRAG_GHOST_OPACITY);
                    }
                }
                if capabilities.group_drag
                    && hresp.drag_stopped()
                    && let Some(p) = ui.ctx().pointer_interact_pos()
                {
                    released = Some((gi, p));
                }
                headers.push((gi, inset_row_rect(hr, depth), g.name.clone()));
                // Keep each group's assets directly beneath its header. The previous separate
                // content pass placed every swatch after every group header, so expanding a group
                // could not reveal its assets in the expected place.
                if open {
                    let mut folder_toggles = Vec::new();
                    {
                        let tree = hierarchy_tree(
                            g.items
                                .iter()
                                .filter(|item| item_visible_for_query(item, &query, group_matches))
                                .map(|item| (item.folder.as_slice(), item))
                                .collect(),
                            crate::pickers::asset_browser_hierarchy::MAX_HIERARCHY_DEPTH,
                        );
                        let toggled_folders = st.toggled.clone();
                        let mut is_folder_open = move |folder: &HierarchyFolder<'_, ItemView>| {
                            let path = folder.path.iter().map(|part| escape_hierarchy_key_part(part)).collect::<Vec<_>>().join("/");
                            !toggled_folders.contains(&format!("{panel}/{}/{}", escape_hierarchy_key_part(&g.name), path))
                        };
                        let st_cell = std::cell::RefCell::new(&mut *st);
                        let ev_cell = std::cell::RefCell::new(&mut ev);
                        let mut draw_folder = |ui: &mut egui::Ui, folder: &HierarchyFolder<'_, ItemView>, folder_open: bool, folder_depth: usize| {
                            let (fr, allocated) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
                            let row_depth = folder_depth + 1 + depth;
                            let fresp = ui.interact(inset_row_rect(fr, row_depth), allocated.id, Sense::click_and_drag());
                            fresp.widget_info(|| {
                                egui::WidgetInfo::labeled(egui::WidgetType::CollapsingHeader, true, folder.path.last().map(String::as_str).unwrap_or(""))
                            });
                            paint_asset_browser_header(
                                ui,
                                fr,
                                folder_open,
                                folder.path.last().map(String::as_str).unwrap_or(""),
                                row_depth,
                                fresp.hovered(),
                                Some(folder.count),
                            );
                            if let Some(dragged) = asset_browser_header_target_with_capabilities(
                                ui,
                                &fresp,
                                inset_row_rect(fr, row_depth),
                                panel,
                                &g.name,
                                &folder.path,
                                HeaderDropCapabilities {
                                    item_drop: capabilities.item_drop,
                                    group_drag: capabilities.group_drag,
                                    reject_group_drop: fresp.dnd_hover_payload::<AssetDrag>().is_some_and(|payload| {
                                        matches!(payload.as_ref(), AssetDrag::Group { scope, name } if scope == panel && group_is_descendant(&**st_cell.borrow(), panel, name, &g.name))
                                    }),
                                },
                            ) {
                                let edge = ui.ctx().pointer_interact_pos().is_some_and(|p| header_drop_before(fr, p));
                                folder_releases.push((dragged, g.name.clone(), folder.path.clone(), edge));
                            }
                            if fresp.clicked() {
                                let path = folder.path.iter().map(|part| escape_hierarchy_key_part(part)).collect::<Vec<_>>().join("/");
                                folder_toggles.push(format!("{panel}/{}/{}", escape_hierarchy_key_part(&g.name), path));
                            }
                            if let Some(menu) = folder_context_menu {
                                if compact_context_menus {
                                    compact_context_menu_target(ui, fresp.clone(), |ui| {
                                        menu(ui, &mut st_cell.borrow_mut(), panel, g, &folder.path, &mut ev_cell.borrow_mut())
                                    });
                                } else {
                                    context_menu_target(fresp.clone(), |ui| {
                                        menu(ui, &mut st_cell.borrow_mut(), panel, g, &folder.path, &mut ev_cell.borrow_mut())
                                    });
                                }
                            }
                        };
                        let mut draw_items = |ui: &mut egui::Ui, items: &[&ItemView], folder_depth: usize| {
                            let w = ui.available_width();
                            let group_indent = depth as f32 * 14.0;
                            let indent = folder_depth as f32 * 14.0;
                            let pad = group_indent + (if list { 0.0 } else { grid_indent }) + indent;
                            let gap = if list || flush { 0.0 } else { GRID_GAP };
                            let (cell, cols) = if list {
                                let row_h = list_item_height.unwrap_or(24.0).max(1.0);
                                (vec2((w - group_indent - indent).max(0.0), row_h), 1)
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
                            let mut insertion_markers = Vec::new();
                            for (run_i, it) in items.iter().enumerate() {
                                let ii = g.items.iter().position(|original| std::ptr::eq(original, *it)).unwrap_or(run_i);
                                let (cx, cy) = ((run_i % cols) as f32, (run_i / cols) as f32);
                                let r = Rect::from_min_size(area.min + vec2(pad + center_offset + cx * (cell.x + gap), cy * (cell.y + gap)), cell);
                                // A one-pixel inset on every card yields a two-pixel visible gap
                                // between neighbours while retaining the same gap at group edges.
                                let r = if !list && !flush { r.shrink(1.0) } else { r };
                                let next_rect = (run_i + 1 < items.len()).then(|| {
                                    let next_i = run_i + 1;
                                    let (next_x, next_y) = ((next_i % cols) as f32, (next_i / cols) as f32);
                                    let next =
                                        Rect::from_min_size(area.min + vec2(pad + center_offset + next_x * (cell.x + gap), next_y * (cell.y + gap)), cell);
                                    if !list && !flush { next.shrink(1.0) } else { next }
                                });
                                let resp = ui.interact(r, ui.id().with((panel, gi, ii)), Sense::click_and_drag());
                                let is_sel = selected.as_deref() == Some(&it.key);
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
                                    // Only selected cards are outlined; centring the stroke on the inset
                                    // card edge leaves one pixel of it on either side.
                                    if is_sel {
                                        ui.painter().rect_stroke(r, CornerRadius::ZERO, Stroke::new(2.0, t.accent), egui::StrokeKind::Middle);
                                    }
                                }
                                let mut released_drag = if capabilities.item_drag {
                                    asset_browser_item_target(ui, &resp, r, next_rect, &mut insertion_markers, panel, &it.key, !list)
                                } else {
                                    None
                                };
                                // On grid rows, the unused space before the first card and after
                                // the last card is also a drop target. In particular, this lets an
                                // incomplete row accept a drop at the panel's right edge.
                                if capabilities.item_drag && !list && released_drag.is_none() {
                                    let first_in_row = run_i % cols == 0;
                                    let last_in_row = run_i + 1 == items.len() || (run_i + 1) % cols == 0;
                                    let (left_edge, right_edge) = item_row_edge_targets(area, r, first_in_row, last_in_row);
                                    let pointer = ui.ctx().pointer_interact_pos();
                                    let edge_target =
                                        left_edge.filter(|rect| pointer.is_some_and(|pointer| rect.contains(pointer))).map(|rect| ("left", rect)).or_else(
                                            || right_edge.filter(|rect| pointer.is_some_and(|pointer| rect.contains(pointer))).map(|rect| ("right", rect)),
                                        );
                                    if let Some((side, edge_rect)) = edge_target {
                                        let edge_response = ui.interact(edge_rect, ui.id().with((panel, gi, ii, side)), Sense::hover());
                                        released_drag =
                                            asset_browser_item_target(ui, &edge_response, r, next_rect, &mut insertion_markers, panel, &it.key, true);
                                    }
                                }
                                if let Some(AssetDrag::Preset { scope, key }) = released_drag
                                    && scope.as_str() == panel
                                    && key.as_str() != it.key.as_str()
                                {
                                    browser_item_drop = true;
                                    let after = ui.ctx().pointer_interact_pos().is_some_and(|p| if list { p.y > r.center().y } else { p.x > r.center().x });
                                    let mut index = ii + usize::from(after);
                                    if let Some((source_group, source_index)) = item_occurrence_position(groups, &key)
                                        && source_group == gi
                                        && source_index < index
                                    {
                                        index = index.saturating_sub(1);
                                    }
                                    if it.folder.is_empty() {
                                        ev_cell.borrow_mut().push(BrowserAction::MoveItem(key, g.name.clone(), Some(index)));
                                    } else {
                                        let before = if after {
                                            g.items
                                                .iter()
                                                .skip(ii + 1)
                                                .find(|candidate| candidate.folder == it.folder && candidate.key != key)
                                                .map(|candidate| candidate.key.clone())
                                        } else {
                                            Some(it.key.clone())
                                        };
                                        ev_cell.borrow_mut().push(BrowserAction::MoveItemToFolder {
                                            key,
                                            group: g.name.clone(),
                                            folder: it.folder.clone(),
                                            before,
                                        });
                                    }
                                }
                                let hover = if it.folder.is_empty() { it.name.clone() } else { format!("{} / {}", it.folder.join(" / "), it.name) };
                                let resp = resp.on_hover_text(hover);
                                let (selected, activated) = asset_browser_item_gesture(&resp, capabilities.selection, capabilities.activation);
                                if activated {
                                    ev_cell.borrow_mut().push(BrowserAction::Activate(it.key.clone()));
                                } else if selected {
                                    ev_cell.borrow_mut().push(BrowserAction::Select(it.key.clone()));
                                }
                                if capabilities.item_drag
                                    && resp.dragged()
                                    && let Some(pp) = ui.ctx().pointer_interact_pos()
                                {
                                    // Keep the source card's size and preview, floating above the
                                    // browser content while the shared hit targets continue working.
                                    let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::new(("preset-drag", panel)));
                                    let grab_offset = ui
                                        .input(|input| input.pointer.press_origin())
                                        .filter(|origin| r.contains(*origin))
                                        .map(|origin| origin - r.min)
                                        .unwrap_or_else(|| r.size() * 0.5);
                                    let ghost_rect = Rect::from_min_size(pp - grab_offset, r.size());
                                    // Extend the child clip by half the selection stroke so the outer
                                    // half is not clipped at the card's bounds.
                                    let mut ghost_ui = ui.new_child(egui::UiBuilder::new().layer_id(layer).max_rect(ghost_rect.expand(1.0)));
                                    ghost_ui.set_opacity(0.4);
                                    paint_item(&ghost_ui, ghost_rect, gi, ii, ItemPaintState::DragGhost);
                                    ghost_ui.painter().rect_stroke(ghost_rect, CornerRadius::ZERO, Stroke::new(2.0, t.accent), egui::StrokeKind::Middle);
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
                                            menu(ui, &mut st_cell.borrow_mut(), panel, it, g, &group_names, &mut ev_cell.borrow_mut())
                                        });
                                    } else {
                                        context_menu_target(resp.clone(), |ui| {
                                            menu(ui, &mut st_cell.borrow_mut(), panel, it, g, &group_names, &mut ev_cell.borrow_mut())
                                        });
                                    }
                                }
                            }
                            for (marker_rect, after, grid) in insertion_markers {
                                crate::widgets::drop_line(ui, marker_rect, after, grid, &t);
                            }
                        };
                        draw_hierarchy(
                            ui,
                            &tree,
                            0,
                            crate::pickers::asset_browser_hierarchy::MAX_HIERARCHY_DEPTH,
                            &mut is_folder_open,
                            &mut draw_folder,
                            &mut draw_items,
                        );
                    }
                    for (dragged, target_group, target_folder, edge) in folder_releases.drain(..) {
                        match dragged {
                            AssetDrag::Preset { scope, key } if scope == panel => {
                                ev.push(BrowserAction::MoveItemToFolder { key, group: target_group, folder: target_folder, before: None });
                                browser_item_drop = true;
                            }
                            AssetDrag::Group { scope, name } if scope == panel && domain_hierarchy_actions => {
                                if let Some(action) = hierarchy_folder_drop(&name, &[], &target_group, &target_folder, edge) {
                                    ev.push(hierarchy_action(action));
                                }
                            }
                            AssetDrag::Folder { scope, group, folder } if scope == panel && domain_hierarchy_actions => {
                                if let Some(action) = hierarchy_folder_drop(&group, &folder, &target_group, &target_folder, edge) {
                                    ev.push(hierarchy_action(action));
                                }
                            }
                            _ => {}
                        }
                    }
                    for key in folder_toggles {
                        if !st.toggled.remove(&key) {
                            st.toggled.insert(key);
                        }
                    }
                }
            }
            // Give the same before-vs-into feedback as the Brushes hierarchy while dragging a
            // group. The target is resolved after all header rectangles have been collected.
            if let Some((from, p)) = drag {
                // Header targets already paint their feedback through the shared interaction
                // helper. Only draw the between-groups insertion marker when the pointer is in a
                // gap/outside a target header.
                if !headers.iter().any(|(i, r, _)| *i != from && r.contains(p)) {
                    let before = group_drop_before(&headers, from, p.y);
                    let anchor = match before {
                        Some(i) => headers.iter().find(|(x, _, _)| *x == i).map(|(_, r, _)| *r),
                        None => headers.last().map(|(_, r, _)| *r),
                    };
                    if let Some(r) = anchor {
                        paint_group_insertion_line(ui, r, before.is_some(), t.accent);
                    }
                }
            }
        });
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("preset-browser-rect", panel)), scroll.inner_rect));
    // Apply the drop now every header rect is known; the next frame draws the new order.
    if let Some((from, p)) = released {
        let from_name = groups.get(from).map(|g| g.name.clone());
        if let Some(from_name) = from_name {
            if let Some((_, rect, target)) = headers.iter().find(|(i, rect, _)| *i != from && rect.contains(p)) {
                if domain_hierarchy_actions && !group_is_descendant(st, panel, &from_name, target) {
                    let edge = header_drop_before(*rect, p);
                    if let Some(action) = hierarchy_folder_drop(&from_name, &[], target, &[], edge) {
                        ev.push(hierarchy_action(action));
                    }
                } else {
                    let edge = header_drop_before(*rect, p);
                    st.drop_group_on_header(panel, &group_names, &from_name, target, edge);
                }
            } else {
                let before = group_drop_before(&headers, from, p.y);
                if before != Some(from) {
                    let before_name = before.and_then(|i| groups.get(i).map(|g| g.name.clone()));
                    if domain_hierarchy_actions {
                        ev.push(BrowserAction::MoveGroup { group: from_name, folder: Vec::new(), before: before_name });
                    } else {
                        st.nest_group(panel, &group_names, &from_name, None);
                        st.move_group(panel, &group_names, &from_name, before_name.as_deref());
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

/// The folder (by library index) a header dragged to `y` lands before (`None` = last). `headers`
/// is in display order; mirrors `dock::drop_before`.
fn group_drop_before(headers: &[(usize, Rect, String)], from: usize, y: f32) -> Option<usize> {
    let hit = headers.iter().find(|(_, r, _)| y < r.center().y).map(|(i, _, _)| *i);
    match hit {
        Some(i) if i == from => Some(from),
        Some(i) => {
            let after_self = headers.iter().position(|(x, _, _)| *x == from).zip(headers.iter().position(|(x, _, _)| *x == i)).is_some_and(|(a, b)| b == a + 1);
            if after_self { Some(from) } else { Some(i) }
        }
        None => None,
    }
}

/// Folder header rects drawn last frame (name → rect), in display order; for tests and automation.
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
mod filter_tests {
    use super::{GroupView, ItemView, filter_groups, item_occurrence_position};

    fn group(name: &str, items: Vec<ItemView>) -> GroupView {
        GroupView { name: name.into(), items }
    }

    fn item(key: &str, name: &str, folder: &[&str]) -> ItemView {
        ItemView { key: key.into(), name: name.into(), folder: folder.iter().map(|part| (*part).into()).collect() }
    }

    #[test]
    fn filtering_keeps_identity_order_and_matches_group_or_folder_names() {
        let groups = vec![
            group("Warm", vec![item("sunset-id", "Sunset", &["Favorites", "Sky"]), item("ember-id", "Ember", &[])]),
            group("Cool", vec![item("ocean-id", "Ocean", &[])]),
        ];

        let by_folder = filter_groups(&groups, "sky");
        assert_eq!(by_folder.len(), 1);
        assert_eq!(by_folder[0].items.iter().map(|item| item.key.as_str()).collect::<Vec<_>>(), ["sunset-id"]);

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
}
