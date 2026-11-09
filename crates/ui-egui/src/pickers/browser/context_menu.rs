//! Context-menu rendering and policy shared by asset browsers.

#[cfg(test)]
use std::collections::BTreeSet;

use crate::pickers::browser::asset_browser::{BrowserAction, GroupView, ItemContextMenuArgs, ItemView};
use crate::pickers::browser::identity::PickerNodeIdentity;
use crate::pickers::browser::inline_rename::ItemRenameState;
use crate::pickers::browser::path_identity_model::{PickerBrowserHierarchy, PickerId};
use crate::pickers::state::PresetUi;

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

/// Selection-dependent labels, enabled states and targets shared by both browser menus.
pub(crate) struct SelectionMenuModel {
    pub(crate) rename_label: String,
    pub(crate) delete_label: String,
    pub(crate) rename_action: Option<BrowserAction>,
    pub(crate) delete_actions: Vec<BrowserAction>,
    pub(crate) export_items: Vec<String>,
}

impl SelectionMenuModel {
    pub(crate) fn new(asset: &str, selection: &crate::pickers::state::BrowserSelectionSnapshot) -> Self {
        let rename_count = selection.targets.len();
        let delete_actions = crate::pickers::browser::selection_operations::delete_selection(selection);
        let delete_count = delete_actions.len();
        let has_headers = selection.targets.iter().any(|target| {
            matches!(target, crate::pickers::browser::selection::SelectionTarget::Group(_) | crate::pickers::browser::selection::SelectionTarget::Folder { .. })
        });
        let has_cards = selection.targets.iter().any(|target| matches!(target, crate::pickers::browser::selection::SelectionTarget::Item { .. }));
        let mixed_selection = has_headers && has_cards;
        let delete_has_headers = delete_actions.iter().any(|action| matches!(action, BrowserAction::DeleteGroup(_) | BrowserAction::DeleteNestedGroup { .. }));
        let delete_has_cards = delete_actions.iter().any(|action| matches!(action, BrowserAction::DeleteItem { .. }));
        let mixed_delete = delete_has_headers && delete_has_cards;
        let rename_label = if mixed_selection {
            format!("Rename {asset}s and Groups…")
        } else if has_headers {
            format!("Rename Group{}…", if rename_count == 1 { "" } else { "s" })
        } else {
            format!("Rename {asset}{}…", if rename_count == 1 { "" } else { "s" })
        };
        let delete_label = if mixed_delete {
            format!("Delete {asset}s and Groups…")
        } else if delete_has_headers {
            format!("Delete Group{}…", if delete_count == 1 { "" } else { "s" })
        } else {
            format!("Delete {asset}{}…", if delete_count == 1 { "" } else { "s" })
        };
        let rename_action = crate::pickers::browser::selection_operations::rename_selection(selection);
        let export_items = crate::pickers::browser::selection_operations::export_selection(selection);

        Self { rename_label, delete_label, rename_action, delete_actions, export_items }
    }
}

/// Draw the selection-dependent actions shared by card, group, empty-space and panel menus.
/// Callers add only their surface-specific creation/import/export entries around this block.
pub(crate) fn selection_actions_menu(
    ui: &mut egui::Ui,
    asset: &str,
    selection: &crate::pickers::state::BrowserSelectionSnapshot,
    events: &mut Vec<BrowserAction>,
) {
    let menu = SelectionMenuModel::new(asset, selection);
    if crate::menu_keys::menu_item_button(ui, &menu.rename_label, menu.rename_action.is_some()).clicked() {
        if let Some(action) = menu.rename_action {
            events.push(action);
        }
        ui.close();
    }
    if crate::menu_keys::menu_item_button(ui, &menu.delete_label, !menu.delete_actions.is_empty()).clicked() {
        events.extend(menu.delete_actions);
        ui.close();
    }
}

/// Draw the picker-level menu on empty browser space. Selection actions are the same resolved
/// targets used by the docking panel menu, so right-click and panel-menu operations stay aligned.
pub(crate) fn picker_context_menu(
    ui: &mut egui::Ui,
    panel: &str,
    groups: &[GroupView],
    selection: &crate::pickers::state::BrowserSelectionSnapshot,
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
    let menu = SelectionMenuModel::new(asset, selection);
    if crate::menu_keys::menu_item_button(ui, &format!("Append Default {plural}…"), supports_append).clicked() {
        events.push(BrowserAction::AppendDefault);
        ui.close();
    }
    if crate::menu_keys::menu_item_button(ui, &format!("Import {plural}…"), supports_import).clicked() {
        events.push(BrowserAction::Import);
        ui.close();
    }
    if crate::menu_keys::menu_item_button(ui, &format!("Export Selected {plural}…"), supports_export && !menu.export_items.is_empty()).clicked() {
        events.push(BrowserAction::ExportSelected(menu.export_items.clone()));
        ui.close();
    }
    selection_actions_menu(ui, asset, selection, events);
    ui.separator();
    if crate::menu_keys::menu_item_button(ui, &format!("New {asset} Preset…"), true).clicked() {
        events.push(BrowserAction::New);
        ui.close();
    }
    if crate::menu_keys::menu_item_button(ui, &format!("New {asset} Group…"), true).clicked() {
        events.push(BrowserAction::NewGroup);
        ui.close();
    }
    let move_targets = selection.selected_items.clone();
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

/// Resolve selected nodes to independent domain deletions, omitting descendants covered by a selected group.
#[cfg(test)]
pub(super) fn selected_delete_actions(selection: &BTreeSet<PickerId>, clicked: &PickerId, context: &BrowserDeleteContext<'_>) -> Option<Vec<BrowserAction>> {
    if !selection.contains(clicked) {
        return None;
    }
    let targets = crate::pickers::browser::selection::resolve_targets(selection, context.hierarchy, context.identities);
    let actions = crate::pickers::browser::selection_operations::delete_targets_actions(&targets);
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
    if selection.contains(clicked) {
        let targets = crate::pickers::browser::selection::resolve_targets(selection, context.hierarchy, context.identities);
        let actions = crate::pickers::browser::selection_operations::delete_targets_actions(&targets);
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
        focused: false,
    });
    state.item_renaming = renames.next();
    state.item_rename_queue = renames.collect();
}

/// Standard item-menu policy shared by preset panels and specialised adapters.
pub(crate) fn standard_item_context_menu(ui: &mut egui::Ui, state: &mut PresetUi, args: ItemContextMenuArgs<'_>, events: &mut Vec<BrowserAction>) {
    let ItemContextMenuArgs { panel, group_names, .. } = args;
    let selection = state.browser_selection_snapshot.get(panel).cloned().unwrap_or_default();
    selection_actions_menu(ui, card_type_name(panel), &selection, events);
    let move_targets = selection.selected_items;
    if !move_targets.is_empty() {
        ui.menu_button(tl!("Move to"), |ui| {
            for destination in group_names {
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

/// Standard group-menu policy shared by preset panels and specialised adapters.
pub(crate) fn standard_group_context_menu(ui: &mut egui::Ui, state: &mut PresetUi, panel: &str, group: &GroupView, events: &mut Vec<BrowserAction>) {
    let selection = state.browser_selection_snapshot.get(panel).cloned().unwrap_or_default();
    let _ = group;
    selection_actions_menu(ui, card_type_name(panel), &selection, events);
}

fn card_type_name(panel: &str) -> &'static str {
    match panel {
        "gradients" => tl!("Gradient"),
        "patterns" => tl!("Pattern"),
        "swatches" => tl!("Swatch"),
        "styles" => tl!("Style"),
        "shapes" => tl!("Shape"),
        "toolPresets" => tl!("Tool Preset"),
        "cloneSource" => tl!("Clone Source"),
        "brushes" => tl!("Brush"),
        _ => tl!("Preset"),
    }
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
        let items = vec![
            ("Basics".into(), ItemView { key: "one".into(), name: "One".into(), group_path: Vec::new() }),
            ("Basics".into(), ItemView { key: "two".into(), name: "Two".into(), group_path: Vec::new() }),
        ];
        let selection = crate::pickers::browser::selection::BrowserSelection {
            contains_card: true,
            contains_selected_card: true,
            selected_cards: vec![("Basics".into(), "one".into()), ("Basics".into(), "two".into())],
            cards: vec![("Basics".into(), "one".into()), ("Basics".into(), "two".into())],
            selected_items: items,
            targets: vec![
                crate::pickers::browser::selection::SelectionTarget::Item { group: "Basics".into(), path: Vec::new(), key: "one".into() },
                crate::pickers::browser::selection::SelectionTarget::Item { group: "Basics".into(), path: Vec::new(), key: "two".into() },
            ],
            ..Default::default()
        };
        let events = Rc::new(RefCell::new(Vec::new()));
        let captured = Rc::clone(&events);
        let mut harness = Harness::builder().with_size(vec2(520.0, 420.0)).build_ui(|ui| {
            ui.menu_button("Picker", |ui| {
                picker_context_menu(ui, "gradients", &groups, &selection, &mut captured.borrow_mut());
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
            "Rename Gradients…",
            "Delete Gradients…",
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
        let move_rect = harness.get_by_label("Move to ⏵").rect();
        assert!(move_rect.top() >= previous_bottom.unwrap(), "Move to must be the final action");
        let row_gap = harness.get_by_label("Import Gradients…").rect().top() - harness.get_by_label("Append Default Gradients…").rect().bottom();
        let section_gap = harness.get_by_label("New Gradient Preset…").rect().top() - harness.get_by_label("Delete Gradients…").rect().bottom();
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
        harness.get_by_label("Move to ⏵").click();
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
        let selection = crate::pickers::browser::selection::BrowserSelection {
            groups: vec!["Basics".into(), "Grays".into()],
            contains_group: true,
            targets: vec![
                crate::pickers::browser::selection::SelectionTarget::Group("Basics".into()),
                crate::pickers::browser::selection::SelectionTarget::Group("Grays".into()),
            ],
            ..Default::default()
        };
        let mut harness = Harness::builder().with_size(vec2(520.0, 420.0)).build_ui(|ui| {
            ui.menu_button("Picker", |ui| {
                picker_context_menu(ui, "gradients", &groups, &selection, &mut Vec::new());
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
        let selection = crate::pickers::browser::selection::BrowserSelection {
            groups: vec!["Basics".into()],
            contains_group: true,
            targets: vec![crate::pickers::browser::selection::SelectionTarget::Group("Basics".into())],
            ..Default::default()
        };
        let mut harness = Harness::builder().with_size(vec2(520.0, 420.0)).build_ui(|ui| {
            ui.menu_button("Picker", |ui| {
                picker_context_menu(ui, "gradients", &groups, &selection, &mut Vec::new());
            });
        });
        crate::PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
        harness.run_steps(2);
        harness.get_by_label("Picker").click();
        harness.run_steps(2);
        assert!(harness.query_by_label("Rename Group…").is_some());
        assert!(harness.query_by_label("Delete Group…").is_some());
    }

    #[test]
    fn mixed_card_and_group_selection_keeps_group_and_card_actions() {
        let groups =
            vec![GroupView { name: "A".into(), root_only: false, items: Vec::new() }, GroupView { name: "B".into(), root_only: false, items: Vec::new() }];
        let summary = crate::pickers::browser::selection::BrowserSelection {
            contains_selected_card: true,
            contains_card: true,
            contains_group: true,
            selected_cards: vec![("A".into(), "one".into())],
            cards: vec![("A".into(), "one".into())],
            groups: vec!["B".into()],
            selected_items: vec![("A".into(), ItemView { key: "one".into(), name: "One".into(), group_path: Vec::new() })],
            targets: vec![
                crate::pickers::browser::selection::SelectionTarget::Group("B".into()),
                crate::pickers::browser::selection::SelectionTarget::Item { group: "A".into(), path: Vec::new(), key: "one".into() },
            ],
            ..Default::default()
        };
        let mut harness = Harness::builder().with_size(vec2(520.0, 420.0)).build_ui(|ui| {
            ui.menu_button("Picker", |ui| {
                picker_context_menu(ui, "gradients", &groups, &summary, &mut Vec::new());
            });
        });
        crate::PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
        harness.run_steps(2);
        harness.get_by_label("Picker").click();
        harness.run_steps(2);

        assert!(harness.query_by_label("Rename Gradients and Groups…").is_some());
        assert!(harness.query_by_label("Delete Gradients and Groups…").is_some());
        assert!(harness.query_by_label("Rename Gradients…").is_none());
    }

    #[test]
    fn mixed_gradient_selection_deletes_selected_group_and_uncovered_card() {
        use std::collections::{BTreeMap, BTreeSet};

        use super::{BrowserDeleteContext, selected_delete_actions};
        use crate::pickers::browser::identity::PickerNodeIdentity;
        use crate::pickers::browser::path_identity_model::{HierarchyNode, PickerBrowserHierarchy, PickerId};

        let group_a = PickerId::group("A");
        let card_a = PickerId::item("A", "gradient-a");
        let group_b = PickerId::group("B");
        let card_b = PickerId::item("B", "gradient-b");
        let hierarchy = PickerBrowserHierarchy::new(vec![
            HierarchyNode::group(group_a.clone(), vec![HierarchyNode::item(card_a.clone())]),
            HierarchyNode::group(group_b.clone(), vec![HierarchyNode::item(card_b.clone())]),
        ])
        .unwrap();
        let identities = BTreeMap::from([
            (group_a.clone(), PickerNodeIdentity::Group("A".into())),
            (card_a.clone(), PickerNodeIdentity::Item { group: "A".into(), key: "gradient-a".into() }),
            (group_b.clone(), PickerNodeIdentity::Group("B".into())),
            (card_b.clone(), PickerNodeIdentity::Item { group: "B".into(), key: "gradient-b".into() }),
        ]);
        let selection = BTreeSet::from([card_b, group_b, card_a.clone()]);
        let context = BrowserDeleteContext { hierarchy: &hierarchy, identities: &identities };

        let actions = selected_delete_actions(&selection, &card_a, &context).unwrap();

        assert!(matches!(actions.as_slice(), [
            BrowserAction::DeleteItem { group, key, .. },
            BrowserAction::DeleteGroup(group_b),
        ] if group == "A" && key == "gradient-a" && group_b == "B"));
    }
}
