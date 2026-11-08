//! Context-menu rendering and policy shared by asset browsers.

use std::collections::BTreeSet;

use crate::pickers::browser::asset_browser::{BrowserAction, GroupView, ItemContextMenuArgs, ItemView};
use crate::pickers::browser::identity::PickerNodeIdentity;
use crate::pickers::browser::path_identity_model::{ContainerId, PickerBrowserHierarchy, PickerId};
use crate::pickers::state::{ItemRenameState, PresetUi};

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
    pub(super) hierarchy: &'a PickerBrowserHierarchy,
    pub(super) identities: &'a std::collections::BTreeMap<PickerId, PickerNodeIdentity>,
}

fn hierarchy_selection_order(hierarchy: &PickerBrowserHierarchy) -> Vec<PickerId> {
    fn visit(node: &super::path_identity_model::HierarchyNode, order: &mut Vec<PickerId>) {
        order.push(node.id().clone());
        for child in node.children() {
            visit(child, order);
        }
    }
    let mut order = Vec::new();
    for root in hierarchy.roots() {
        visit(root, &mut order);
    }
    order
}

/// Resolve selected nodes to independent domain deletions, omitting descendants covered by a selected group.
pub(super) fn selected_delete_actions(selection: &BTreeSet<PickerId>, clicked: &PickerId, context: &BrowserDeleteContext<'_>) -> Option<Vec<BrowserAction>> {
    if !selection.contains(clicked) {
        return None;
    }
    let mut actions = Vec::new();
    let selection_order = hierarchy_selection_order(context.hierarchy);
    for id in selection_order.iter().filter(|id| selection.contains(*id)) {
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
pub(crate) fn standard_item_context_menu(ui: &mut egui::Ui, _state: &mut PresetUi, args: ItemContextMenuArgs<'_>, events: &mut Vec<BrowserAction>) {
    let ItemContextMenuArgs { panel: _, item, group, group_names, selected_items } = args;
    let targets = item_context_targets(item, group, selected_items);
    let count = targets.len();
    let delete_label = if count > 1 { format!("{} ({count})", tl!("Delete")) } else { tl!("Delete").to_string() };
    let all_in_current_group = targets.iter().all(|(source_group, _)| source_group == &group.name);
    match draw_item_context_menu(ui, tl!("Rename…"), &delete_label, Some((tl!("Move to"), group_names, if all_in_current_group { &group.name } else { "" })))
    {
        Some(ContextMenuAction::Rename) => events.push(BrowserAction::BeginRenameItems(targets)),
        Some(ContextMenuAction::Delete) => events.extend(delete_item_actions(&targets)),
        Some(ContextMenuAction::MoveTo(destination)) => events.extend(move_item_actions(&targets, &destination)),
        None => {}
    }
}

/// Standard group-menu policy shared by preset panels and specialised adapters.
pub(crate) fn standard_group_context_menu(ui: &mut egui::Ui, _state: &mut PresetUi, _panel: &str, group: &GroupView, events: &mut Vec<BrowserAction>) {
    match draw_group_context_menu(ui, tl!("Rename Group…"), tl!("Delete Group")) {
        Some(ContextMenuAction::Rename) => events.push(BrowserAction::BeginRenameGroup(group.name.clone())),
        Some(ContextMenuAction::Delete) => events.push(BrowserAction::DeleteGroup(group.name.clone())),
        _ => {}
    }
}

/// Attach a consumer-provided context menu to an egui response.
pub(crate) fn context_menu_target(resp: egui::Response, menu: impl FnOnce(&mut egui::Ui)) {
    let _ = resp.context_menu(menu);
}
