//! Asset-browser contributions to the shared docking panel menu.

use crate::PhotocraftApp;
use crate::pickers::browser::asset_browser::BrowserAction;
use crate::pickers::state::PresetUi;

fn browser_actions_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui, state: &mut PresetUi, panel: &str) {
    let Some((asset, prefix, new_params)) = (match panel {
        "gradients" => Some((tl!("Gradient"), "gradient.presets", serde_json::json!({"name": "Custom"}))),
        "patterns" => Some((tl!("Pattern"), "pattern.presets", serde_json::json!({}))),
        "swatches" => Some((tl!("Swatch"), "swatches", serde_json::json!({}))),
        "styles" => Some((tl!("Style"), "style.presets", serde_json::json!({}))),
        "shapes" => Some((tl!("Shape"), "shape.presets", serde_json::json!({}))),
        "toolPresets" => Some((tl!("Tool Preset"), "tool.presets", serde_json::json!({}))),
        "cloneSource" => Some((tl!("Clone Source"), "cloneSource", serde_json::json!({}))),
        _ => None,
    }) else {
        return;
    };
    let selection_actions =
        ui.ctx().data(|data| data.get_temp::<Vec<BrowserAction>>(egui::Id::new(("preset-browser-selection-actions", panel))).unwrap_or_default());
    let delete_actions =
        ui.ctx().data(|data| data.get_temp::<Vec<BrowserAction>>(egui::Id::new(("preset-browser-delete-selection", panel))).unwrap_or_default());
    let has_selection = state.browser_selection.get(panel).is_some_and(|selection| !selection.is_empty());
    let rename_items = selection_actions.iter().find(|action| matches!(action, BrowserAction::BeginRenameItems(_)));
    let rename_groups = selection_actions.iter().find(|action| matches!(action, BrowserAction::BeginRenameGroup(_) | BrowserAction::BeginRenameGroups(_)));
    let last_selected_is_group = ui.ctx().data(|data| data.get_temp::<bool>(egui::Id::new(("preset-browser-last-selection-is-group", panel))).unwrap_or(false));
    let show_group_actions = match (rename_items.is_some(), rename_groups.is_some()) {
        (true, false) => false,
        (false, true) => true,
        (true, true) | (false, false) => last_selected_is_group,
    };
    let rename_group_count = match rename_groups {
        Some(BrowserAction::BeginRenameGroup(_)) => 1,
        Some(BrowserAction::BeginRenameGroups(groups)) => groups.len(),
        _ => 0,
    };
    let delete_items: Vec<_> = delete_actions.iter().filter(|action| matches!(action, BrowserAction::DeleteItem { .. })).collect();
    let delete_groups: Vec<_> = delete_actions.iter().filter(|action| matches!(action, BrowserAction::DeleteGroup(_))).collect();

    if crate::menu_keys::menu_item_button(ui, &format!("New {asset} Preset…"), true).clicked() {
        if panel == "gradients" {
            crate::pickers::adapters::gradients::new_gradient(app, state);
        } else {
            crate::pickers::browser::browser_actions::generic_with_state(app, state, panel, prefix, &BrowserAction::New, new_params);
        }
        ui.close();
    }
    if crate::menu_keys::menu_item_button(ui, &format!("New {asset} Group…"), true).clicked() {
        crate::pickers::browser::browser_actions::generic_with_state(app, state, panel, prefix, &BrowserAction::NewGroup, serde_json::json!({}));
        ui.close();
    }
    ui.separator();

    let rename_item_label =
        format!("Rename {asset}{}…", if let Some(BrowserAction::BeginRenameItems(items)) = rename_items { if items.len() > 1 { "s" } else { "" } } else { "" });
    let rename_group_label = format!("Rename Group{}…", if rename_group_count > 1 { "s" } else { "" });
    if show_group_actions {
        if crate::menu_keys::menu_item_button(ui, &rename_group_label, has_selection && rename_groups.is_some()).clicked() {
            if let Some(action) = rename_groups {
                crate::pickers::browser::browser_actions::generic_with_state(app, state, panel, prefix, action, serde_json::json!({}));
            }
            ui.close();
        }
    } else if crate::menu_keys::menu_item_button(ui, &rename_item_label, has_selection && rename_items.is_some()).clicked() {
        if let Some(action) = rename_items {
            crate::pickers::browser::browser_actions::generic_with_state(app, state, panel, prefix, action, serde_json::json!({}));
        }
        ui.close();
    }
    let delete_item_label = format!("Delete {}{}…", asset, if delete_items.len() > 1 { "s" } else { "" });
    let delete_group_label = format!("Delete Group{}…", if delete_groups.len() > 1 { "s" } else { "" });
    if show_group_actions {
        if crate::menu_keys::menu_item_button(ui, &delete_group_label, has_selection && !delete_groups.is_empty()).clicked() {
            for action in delete_groups {
                crate::pickers::browser::browser_actions::generic_with_state(app, state, panel, prefix, action, serde_json::json!({}));
            }
            ui.close();
        }
    } else if crate::menu_keys::menu_item_button(ui, &delete_item_label, has_selection && !delete_items.is_empty()).clicked() {
        for action in delete_items {
            crate::pickers::browser::browser_actions::generic_with_state(app, state, panel, prefix, action, serde_json::json!({}));
        }
        ui.close();
    }
    ui.separator();
}

/// Render the shared asset picker's menu points and its domain-specific extension.
///
/// The generic display/search controls live in `picker_panel_menu`; this module connects those
/// controls to the asset-browser panels and their domain-owned import/export/reset actions.
/// Taking picker state temporarily keeps its borrow independent from domain actions on the app.
pub(crate) fn shared_picker_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui, panel: &str) -> bool {
    if !crate::pickers::panels::shared_docking(&format!("window.panel.{panel}")) {
        return false;
    }
    let mut state = std::mem::take(&mut app.ui.presets_ui);
    let app_cell = std::cell::RefCell::new(app);
    let mut leading = |ui: &mut egui::Ui, state: &mut crate::pickers::state::PresetUi, _panel: &str| {
        browser_actions_menu(&mut app_cell.borrow_mut(), ui, state, panel);
    };
    let mut trailing = |ui: &mut egui::Ui, state: &mut crate::pickers::state::PresetUi, _panel: &str| {
        let mut app = app_cell.borrow_mut();
        match panel {
            "gradients" => crate::pickers::adapters::gradients::panel_menu(&mut app, ui),
            "patterns" => crate::pickers::adapters::patterns::panel_menu(&mut app, ui),
            "swatches" => crate::pickers::adapters::swatches::panel_menu(&mut app, ui, state),
            _ => {}
        }
    };
    let leading = Some(&mut leading as crate::pickers::shell::MenuContentsCallback<'_>);
    let trailing = Some(&mut trailing as crate::pickers::shell::MenuContentsCallback<'_>);
    crate::pickers::shell::picker_menu_contents_ordered(ui, &mut state, panel, crate::pickers::shell::PickerMenuOptions::for_panel(panel), leading, trailing);
    app_cell.into_inner().ui.presets_ui = state;
    true
}

#[cfg(test)]
mod tests {
    use super::{browser_actions_menu, shared_picker_menu};
    use crate::PhotocraftApp;
    use crate::pickers::browser::asset_browser::{BrowserAction, ItemView};
    use crate::pickers::state::PresetUi;
    use egui::vec2;
    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };

    #[test]
    fn gradients_panel_menu_renders_contract_labels_in_order() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut harness = Harness::builder().with_size(vec2(520.0, 520.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                crate::pickers::shell::picker_menu_popup(ui, "Panel menu", egui::Vec2::ZERO, |ui| {
                    assert!(shared_picker_menu(app, ui, "gradients"));
                });
            },
            app,
        );
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
        harness.run_steps(2);
        harness.get_by_label("Panel menu").click();
        harness.run_steps(2);

        let labels = [
            "New Gradient Preset…",
            "New Gradient Group…",
            "Rename Gradient…",
            "Delete Gradient…",
            "Text Only",
            "Small Thumbnail",
            "Large Thumbnail",
            "Small List",
            "Large List",
            "Wrap List",
            "Show Search Bar",
            "Show Recents",
            "Append Default Gradients…",
            "Import Gradients…",
        ];
        let mut previous_bottom = None;
        for label in labels {
            let rect = harness.get_by_label(label).rect();
            if let Some(bottom) = previous_bottom {
                assert!(rect.top() >= bottom, "{label} must follow the previous menu entry");
            }
            previous_bottom = Some(rect.bottom());
        }

        let gap = |before: &str, after: &str| harness.get_by_label(after).rect().top() - harness.get_by_label(before).rect().bottom();
        let row_gap = gap("New Gradient Preset…", "New Gradient Group…");
        for (before, after) in [
            ("New Gradient Group…", "Rename Gradient…"),
            ("Delete Gradient…", "Text Only"),
            ("Large List", "Wrap List"),
            ("Wrap List", "Show Search Bar"),
            ("Show Recents", "Append Default Gradients…"),
        ] {
            assert!(gap(before, after) > row_gap, "{before} and {after} must be separate menu sections");
        }
        assert!(gap("Show Search Bar", "Show Recents") <= row_gap, "search and recents share one section");

        for label in ["Rename Gradient…", "Delete Gradient…"] {
            assert!(harness.get_by_label(label).accesskit_node().is_disabled(), "{label} is disabled without a selection");
        }
        assert!(harness.query_by_label("Rename Group…").is_none(), "only the last-selected asset kind is shown");
        assert!(harness.query_by_label("Delete Group…").is_none(), "only the last-selected asset kind is shown");
    }

    #[test]
    fn gradient_menu_disables_rename_and_delete_when_selection_is_empty_even_if_actions_are_stale() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut state = PresetUi::default();
        state.browser_selection.insert("gradients".into(), std::collections::BTreeSet::new());
        let target = ItemView { key: "one".into(), name: "One".into(), group_path: Vec::new() };
        let mut harness = Harness::builder().with_size(vec2(520.0, 520.0)).build_ui_state(
            |ui, (app, state): &mut (PhotocraftApp, PresetUi)| {
                ui.ctx().data_mut(|data| {
                    data.insert_temp(
                        egui::Id::new(("preset-browser-selection-actions", "gradients")),
                        vec![BrowserAction::BeginRenameItems(vec![("Basics".into(), target.clone())])],
                    );
                    data.insert_temp(
                        egui::Id::new(("preset-browser-delete-selection", "gradients")),
                        vec![BrowserAction::DeleteItem { group: "Basics".into(), group_path: Vec::new(), key: "one".into() }],
                    );
                });
                crate::pickers::shell::picker_menu_popup(ui, "Panel menu", egui::Vec2::ZERO, |ui| {
                    browser_actions_menu(app, ui, state, "gradients");
                });
            },
            (app, state),
        );
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
        harness.run_steps(2);
        harness.get_by_label("Panel menu").click();
        harness.run_steps(2);

        for label in ["Rename Gradient…", "Delete Gradient…"] {
            assert!(harness.get_by_label(label).accesskit_node().is_disabled(), "{label} must be disabled with an empty visible selection");
        }
    }

    #[test]
    fn gradient_menu_creates_a_preset_through_the_domain_aware_browser_action() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let before = app.session.presets.gradients.iter().map(|group| group.items.len()).sum::<usize>();
        let mut harness = Harness::builder().with_size(vec2(420.0, 360.0)).build_ui_state(
            |ui, (app, state): &mut (PhotocraftApp, PresetUi)| {
                crate::pickers::shell::picker_menu_popup(ui, "Panel menu", egui::Vec2::ZERO, |ui| {
                    super::browser_actions_menu(app, ui, state, "gradients");
                });
            },
            (app, PresetUi::default()),
        );
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
        harness.run_steps(2);
        harness.get_by_label("Panel menu").click();
        harness.run_steps(2);
        harness.get_by_label("New Gradient Preset…").click();
        harness.run_steps(2);

        let after = harness.state().0.session.presets.gradients.iter().map(|group| group.items.len()).sum::<usize>();
        assert_eq!(after, before + 1);
        assert!(harness.state().1.item_renaming.is_some(), "the created gradient enters the shared inline rename flow");
    }

    #[test]
    fn gradient_menu_renames_the_complete_selection_through_shared_actions() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut state = PresetUi::default();
        state.browser_selection.insert(
            "gradients".into(),
            std::collections::BTreeSet::from([
                crate::pickers::browser::path_identity_model::PickerId::item("Basics", "one"),
                crate::pickers::browser::path_identity_model::PickerId::item("Basics", "two"),
            ]),
        );
        let targets = vec![
            ("Basics".into(), ItemView { key: "one".into(), name: "One".into(), group_path: Vec::new() }),
            ("Basics".into(), ItemView { key: "two".into(), name: "Two".into(), group_path: Vec::new() }),
        ];
        let mut harness = Harness::builder().with_size(vec2(420.0, 360.0)).build_ui_state(
            |ui, (app, state): &mut (PhotocraftApp, PresetUi)| {
                ui.ctx().data_mut(|data| {
                    data.insert_temp(egui::Id::new(("preset-browser-selection-actions", "gradients")), vec![BrowserAction::BeginRenameItems(targets.clone())]);
                });
                crate::pickers::shell::picker_menu_popup(ui, "Panel menu", egui::Vec2::ZERO, |ui| {
                    browser_actions_menu(app, ui, state, "gradients");
                });
            },
            (app, state),
        );
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
        harness.run_steps(2);
        harness.get_by_label("Panel menu").click();
        harness.run_steps(2);
        harness.get_by_label("Rename Gradients…").click();
        harness.run_steps(2);

        assert_eq!(harness.state().1.item_renaming.as_ref().map(|rename| rename.key.as_str()), Some("one"));
        assert_eq!(harness.state().1.item_rename_queue.len(), 1);
        assert_eq!(harness.state().1.item_rename_queue[0].key, "two");
    }

    #[test]
    fn gradient_menu_queues_all_selected_groups_for_sequential_rename() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut state = PresetUi::default();
        state.browser_selection.insert(
            "gradients".into(),
            std::collections::BTreeSet::from([
                crate::pickers::browser::path_identity_model::PickerId::group("Basics"),
                crate::pickers::browser::path_identity_model::PickerId::group("Grays"),
            ]),
        );
        let mut harness = Harness::builder().with_size(vec2(420.0, 360.0)).build_ui_state(
            |ui, (app, state): &mut (PhotocraftApp, PresetUi)| {
                ui.ctx().data_mut(|data| {
                    data.insert_temp(
                        egui::Id::new(("preset-browser-selection-actions", "gradients")),
                        vec![BrowserAction::BeginRenameGroups(vec!["Basics".into(), "Grays".into()])],
                    );
                    data.insert_temp(egui::Id::new(("preset-browser-last-selection-is-group", "gradients")), true);
                });
                crate::pickers::shell::picker_menu_popup(ui, "Panel menu", egui::Vec2::ZERO, |ui| {
                    browser_actions_menu(app, ui, state, "gradients");
                });
            },
            (app, state),
        );
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
        harness.run_steps(2);
        harness.get_by_label("Panel menu").click();
        harness.run_steps(2);
        harness.get_by_label("Rename Groups…").click();
        harness.run_steps(2);

        assert!(harness.query_by_label("Rename Gradient…").is_none(), "group selection hides card rename");
        assert!(harness.query_by_label("Delete Gradient…").is_none(), "group selection hides card delete");
        assert_eq!(harness.state().1.renaming.as_ref().map(|rename| rename.1.as_str()), Some("group:Basics"));
        assert_eq!(harness.state().1.group_rename_queue, [("gradients".into(), "group:Grays".into(), "Grays".into())]);
    }
}
