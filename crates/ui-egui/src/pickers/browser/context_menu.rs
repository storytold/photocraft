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
}

/// Draw the picker-level menu on empty browser space. Selection actions are the same resolved
/// targets used by the docking panel menu, so right-click and panel-menu operations stay aligned.
pub(crate) fn picker_context_menu(
    ui: &mut egui::Ui,
    panel: &str,
    groups: &[GroupView],
    selection_actions: &[BrowserAction],
    delete_actions: &[BrowserAction],
    selection: &crate::pickers::state::BrowserSelectionSummary,
    events: &mut Vec<BrowserAction>,
) {
    let Some((asset, plural, supports_append, supports_import, supports_export)) = (match panel {
        "gradients" => Some((tl!("Gradient"), tl!("Gradients"), true, true, true)),
        "patterns" => Some((tl!("Pattern"), tl!("Patterns"), false, true, true)),
        "swatches" => Some((tl!("Swatch"), tl!("Swatches"), false, true, true)),
        "styles" => Some((tl!("Style"), tl!("Styles"), false, false, false)),
        "shapes" => Some((tl!("Shape"), tl!("Shapes"), false, false, false)),
        "toolPresets" => Some((tl!("Tool Preset"), tl!("Tool Presets"), false, false, false)),
        "cloneSource" => Some((tl!("Clone Source"), tl!("Clone Sources"), false, false, false)),
        _ => None,
    }) else {
        return;
    };
    let rename_items = selection_actions.iter().find_map(|action| match action {
        BrowserAction::BeginRenameItems(items) => Some(items),
        _ => None,
    });
    let rename_groups = selection_actions.iter().find_map(|action| match action {
        BrowserAction::BeginRenameGroups(groups) => Some(groups.as_slice()),
        BrowserAction::BeginRenameGroup(group) => Some(std::slice::from_ref(group)),
        _ => None,
    });
    let selected_items: Vec<String> = rename_items.into_iter().flatten().map(|(_, item)| item.key.clone()).collect();
    let export_items: Vec<String> = if selection.contains_card {
        selection.cards.iter().map(|(_, key)| key.clone()).collect()
    } else {
        selected_items.clone()
    };
    let delete_items: Vec<_> = delete_actions.iter().filter(|action| matches!(action, BrowserAction::DeleteItem { .. })).collect();
    let delete_groups: Vec<_> =
        delete_actions.iter().filter(|action| matches!(action, BrowserAction::DeleteGroup(_) | BrowserAction::DeleteNestedGroup { .. })).collect();
    let has_item_selection = !selected_items.is_empty();
    let has_group_selection = rename_groups.is_some_and(|groups| !groups.is_empty());
    let show_group_actions = has_group_selection && !has_item_selection;
    let rename_count = if show_group_actions { rename_groups.map_or(0, |groups| groups.len()) } else { selected_items.len() };
    let delete_count = if show_group_actions { delete_groups.len() } else { delete_items.len() };

    if crate::menu_keys::menu_item_button(ui, &format!("Append Default {plural}…"), supports_append).clicked() {
        events.push(BrowserAction::AppendDefault);
        ui.close();
    }
    if crate::menu_keys::menu_item_button(ui, &format!("Import {plural}…"), supports_import).clicked() {
        events.push(BrowserAction::Import);
        ui.close();
    }
    if crate::menu_keys::menu_item_button(ui, &format!("Export Selected {plural}…"), supports_export && !export_items.is_empty()).clicked() {
        events.push(BrowserAction::ExportSelected(export_items));
        ui.close();
    }
    let rename_label = if show_group_actions {
        format!("Rename Group{}…", if rename_count == 1 { "" } else { "s" })
    } else {
        format!("Rename {asset}{}…", if rename_count == 1 { "" } else { "s" })
    };
    let rename_action = if show_group_actions {
        rename_groups.map(|groups| BrowserAction::BeginRenameGroups(groups.to_vec()))
    } else {
        rename_items.cloned().map(BrowserAction::BeginRenameItems)
    };
    if crate::menu_keys::menu_item_button(ui, &rename_label, rename_action.is_some()).clicked() {
        if let Some(action) = rename_action {
            events.push(action);
        }
        ui.close();
    }
    let delete_label = if show_group_actions {
        format!("Delete Group{}…", if delete_count == 1 { "" } else { "s" })
    } else {
        format!("Delete {asset}{}…", if delete_count == 1 { "" } else { "s" })
    };
    if crate::menu_keys::menu_item_button(ui, &delete_label, delete_count > 0).clicked() {
        if show_group_actions {
            events.extend(delete_groups.into_iter().cloned());
        } else {
            events.extend(delete_items.into_iter().cloned());
        }
        ui.close();
    }
    ui.separator();
    if crate::menu_keys::menu_item_button(ui, &format!("New {asset} Preset…"), true).clicked() {
        events.push(BrowserAction::New);
        ui.close();
    }
    if crate::menu_keys::menu_item_button(ui, &format!("New {asset} Group…"), true).clicked() {
        events.push(BrowserAction::NewGroup);
        ui.close();
    }
    let move_targets = rename_items.map(|items| items.iter().cloned().collect::<Vec<_>>()).unwrap_or_default();
    if !move_targets.is_empty() {
        ui.separator();
        ui.menu_button(tl!("Move to"), |ui| {
            for destination in groups.iter().map(|group| &group.name) {
                if move_targets.iter().all(|(source, _)| source == destination) {
                    continue;
                }
                if ui.button(destination).clicked() {
                    events.extend(move_item_actions(&move_targets, destination));
                    ui.close();
                }
            }
        });
    }
}

/// Draw a configurable item menu and return a request for the consumer to handle.
pub(crate) fn draw_item_context_menu(ui: &mut egui::Ui, rename_label: &str, delete_label: &str) -> Option<ContextMenuAction> {
    if ui.button(rename_label).clicked() {
        ui.close();
        return Some(ContextMenuAction::Rename);
    }
    if ui.button(delete_label).clicked() {
        ui.close();
        return Some(ContextMenuAction::Delete);
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
pub(crate) fn standard_item_context_menu(ui: &mut egui::Ui, state: &mut PresetUi, args: ItemContextMenuArgs<'_>, events: &mut Vec<BrowserAction>) {
    let ItemContextMenuArgs { panel, item, group, group_names, selected_items } = args;
    let targets = item_context_targets(item, group, selected_items);
    let selected = vec![BrowserAction::BeginRenameItems(targets.clone())];
    let deletions = delete_item_actions(&targets);
    let groups: Vec<GroupView> = group_names.iter().map(|name| GroupView { name: name.clone(), root_only: false, items: Vec::new() }).collect();
    let selection = state.browser_selection_summary.get(panel).cloned().unwrap_or_default();
    picker_context_menu(ui, panel, &groups, &selected, &deletions, &selection, events);
}

/// Standard group-menu policy shared by preset panels and specialised adapters.
pub(crate) fn standard_group_context_menu(ui: &mut egui::Ui, state: &mut PresetUi, panel: &str, group: &GroupView, events: &mut Vec<BrowserAction>) {
    let selected = vec![BrowserAction::BeginRenameGroup(group.name.clone())];
    let deletions = vec![BrowserAction::DeleteGroup(group.name.clone())];
    let selection = state.browser_selection_summary.get(panel).cloned().unwrap_or_default();
    picker_context_menu(ui, panel, std::slice::from_ref(group), &selected, &deletions, &selection, events);
}

/// Attach a consumer-provided context menu to an egui response.
pub(crate) fn context_menu_target(resp: egui::Response, menu: impl FnOnce(&mut egui::Ui)) {
    let _ = resp.context_menu(menu);
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use egui::vec2;
    use egui_kittest::{Harness, kittest::Queryable};

    use super::picker_context_menu;
    use crate::pickers::browser::asset_browser::{BrowserAction, GroupView, ItemView};

    #[test]
    fn gradient_context_menu_orders_all_actions_and_dispatches_import() {
        let groups = vec![
            GroupView { name: "Basics".into(), root_only: false, items: Vec::new() },
            GroupView { name: "Grays".into(), root_only: false, items: Vec::new() },
        ];
        let items = vec![("Basics".into(), ItemView { key: "one".into(), name: "One".into(), group_path: Vec::new() })];
        let selection = vec![BrowserAction::BeginRenameItems(items)];
        let deletions = vec![BrowserAction::DeleteItem { group: "Basics".into(), group_path: Vec::new(), key: "one".into() }];
        let events = Rc::new(RefCell::new(Vec::new()));
        let captured = Rc::clone(&events);
        let mut harness = Harness::builder().with_size(vec2(520.0, 420.0)).build_ui(|ui| {
            ui.menu_button("Picker", |ui| {
                picker_context_menu(
                    ui,
                    "gradients",
                    &groups,
                    &selection,
                    &deletions,
                    &crate::pickers::state::BrowserSelectionSummary {
                        contains_card: true,
                        cards: vec![("Basics".into(), "one".into())],
                        ..Default::default()
                    },
                    &mut captured.borrow_mut(),
                );
            });
        });
        crate::PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
        harness.run_steps(2);
        harness.get_by_label("Picker").click();
        harness.run_steps(2);

        let labels = [
            "Append Default Gradients…",
            "Import Gradients…",
            "Export Selected Gradients…",
            "Rename Gradient…",
            "Delete Gradient…",
            "New Gradient Preset…",
            "New Gradient Group…",
        ];
        let mut previous_bottom = None;
        for label in labels {
            let rect = harness.get_by_label(label).rect();
            if let Some(bottom) = previous_bottom {
                assert!(rect.top() >= bottom, "{label} is out of order");
            }
            previous_bottom = Some(rect.bottom());
        }
        let move_rect = harness.get_by_label("Move to").rect();
        assert!(move_rect.top() >= previous_bottom.unwrap(), "Move to must be the final action");
        let row_gap = harness.get_by_label("Import Gradients…").rect().top() - harness.get_by_label("Append Default Gradients…").rect().bottom();
        let section_gap = harness.get_by_label("New Gradient Preset…").rect().top() - harness.get_by_label("Delete Gradient…").rect().bottom();
        assert!(section_gap > row_gap, "creation actions must follow a separator");
        let move_gap = move_rect.top() - harness.get_by_label("New Gradient Group…").rect().bottom();
        assert!(move_gap > row_gap, "Move to must follow a separator");

        harness.get_by_label("Import Gradients…").click();
        harness.run_steps(2);
        assert!(matches!(events.borrow().as_slice(), [BrowserAction::Import]));

        harness.get_by_label("Picker").click();
        harness.run_steps(2);
        harness.get_by_label("Export Selected Gradients…").click();
        harness.run_steps(2);
        assert!(matches!(events.borrow().as_slice(), [BrowserAction::Import, BrowserAction::ExportSelected(names)] if names == &["one", "two"]));

        harness.get_by_label("Picker").click();
        harness.run_steps(2);
        harness.get_by_label("Move to").click();
        harness.run_steps(2);
        harness.get_by_label("Grays").click();
        harness.run_steps(2);
        assert!(matches!(events.borrow().as_slice(), [
            BrowserAction::Import,
            BrowserAction::ExportSelected(_),
            BrowserAction::Move(one, destination),
            BrowserAction::Move(two, destination_again),
        ] if one == "one" && two == "two" && destination == "Grays" && destination_again == "Grays"));
    }

    #[test]
    fn group_context_menu_uses_group_singular_plural_labels() {
        let groups = vec![GroupView { name: "Basics".into(), root_only: false, items: Vec::new() }];
        let selection = vec![BrowserAction::BeginRenameGroups(vec!["Basics".into(), "Grays".into()])];
        let deletions = vec![BrowserAction::DeleteGroup("Basics".into()), BrowserAction::DeleteGroup("Grays".into())];
        let mut harness = Harness::builder().with_size(vec2(520.0, 420.0)).build_ui(|ui| {
            ui.menu_button("Picker", |ui| {
                picker_context_menu(ui, "gradients", &groups, &selection, &deletions, &Default::default(), &mut Vec::new());
            });
        });
        crate::PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
        harness.run_steps(2);
        harness.get_by_label("Picker").click();
        harness.run_steps(2);
        assert!(harness.query_by_label("Rename Groups…").is_some());
        assert!(harness.query_by_label("Delete Groups…").is_some());
        assert!(harness.query_by_label("Rename Gradients…").is_none());
    }

    #[test]
    fn single_group_context_menu_uses_singular_labels() {
        let groups = vec![GroupView { name: "Basics".into(), root_only: false, items: Vec::new() }];
        let selection = vec![BrowserAction::BeginRenameGroup("Basics".into())];
        let deletions = vec![BrowserAction::DeleteGroup("Basics".into())];
        let mut harness = Harness::builder().with_size(vec2(520.0, 420.0)).build_ui(|ui| {
            ui.menu_button("Picker", |ui| {
                picker_context_menu(ui, "gradients", &groups, &selection, &deletions, &Default::default(), &mut Vec::new());
            });
        });
        crate::PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
        harness.run_steps(2);
        harness.get_by_label("Picker").click();
        harness.run_steps(2);
        assert!(harness.query_by_label("Rename Group…").is_some());
        assert!(harness.query_by_label("Delete Group…").is_some());
    }
}
