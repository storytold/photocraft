//! Asset-browser contributions to the shared docking panel menu.

use crate::PhotocraftApp;
use crate::pickers::browser::asset_browser::BrowserAction;
use crate::pickers::browser::context_menu::SelectionMenuModel;
use crate::pickers::state::PresetUi;

fn browser_actions_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui, state: &mut PresetUi, panel: &str) {
    let Some((asset, prefix)) = (match panel {
        "gradients" => Some((tl!("Gradient"), "gradient.presets")),
        "patterns" => Some((tl!("Pattern"), "pattern.presets")),
        "swatches" => Some((tl!("Swatch"), "swatches")),
        "styles" => Some((tl!("Style"), "style.presets")),
        "shapes" => Some((tl!("Shape"), "shape.presets")),
        "toolPresets" => Some((tl!("Tool Preset"), "tool.presets")),
        "cloneSource" => Some((tl!("Clone Source"), "cloneSource")),
        _ => None,
    }) else {
        return;
    };
    let selection_summary = state.browser_selection_snapshot.get(panel).cloned().unwrap_or_default();
    let menu = SelectionMenuModel::new(&asset, &selection_summary);

    let new_preset_enabled = !matches!(panel, "patterns" | "cloneSource");
    if crate::menu_keys::menu_item_button(ui, &format!("New {asset} Preset…"), new_preset_enabled).clicked() {
        if panel == "gradients" {
            crate::pickers::browser::popup_dialogs::request_new_gradient_preset(ui.ctx());
        } else if panel == "swatches" {
            crate::pickers::browser::popup_dialogs::request_new_preset(ui.ctx(), panel, &asset);
        } else {
            crate::pickers::browser::popup_dialogs::request_new_preset(ui.ctx(), panel, &asset);
        }
        ui.close();
    }
    if crate::menu_keys::menu_item_button(ui, &format!("New {asset} Group…"), true).clicked() {
        crate::pickers::browser::popup_dialogs::request_new_group(ui.ctx(), panel);
        ui.close();
    }
    ui.separator();

    if crate::menu_keys::menu_item_button(ui, &menu.rename_label, menu.rename_action.is_some()).clicked() {
        if let Some(action) = menu.rename_action.clone() {
            match action {
                BrowserAction::BeginRenameSelection { .. } => {
                    crate::pickers::browser::popup_dialogs::request_rename_selection(ui.ctx(), panel, action);
                }
                _ => {}
            }
        }
        ui.close();
    }
    let mut requested_deletions = Vec::new();
    if crate::menu_keys::menu_item_button(ui, &menu.delete_label, !menu.delete_actions.is_empty()).clicked() {
        requested_deletions.extend(menu.delete_actions.iter().cloned());
    }
    if !requested_deletions.is_empty() {
        // The confirmation is a separate modal; dismiss this popup immediately so it does not
        // remain open underneath the dialog.
        ui.close();
    }
    let confirmed = crate::pickers::browser::popup_dialogs::confirm_deletions(ui, panel, &[], requested_deletions);
    if !confirmed.is_empty() {
        for action in &confirmed {
            crate::pickers::browser::browser_actions::generic_with_state(app, state, panel, prefix, action, serde_json::json!({}));
        }
        ui.close();
    }
    ui.separator();
}

/// Keep unsupported import/export entries visible while their formats are not implemented.
fn unavailable_asset_actions_menu(ui: &mut egui::Ui, plural: &str) {
    let _ = crate::menu_keys::menu_item_button(ui, &format!("Import {plural}…"), false);
    let _ = crate::menu_keys::menu_item_button(ui, &format!("Export Selected {plural}…"), false);
    let _ = crate::menu_keys::menu_item_button(ui, &format!("Export {plural}…"), false);
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
            "gradients" => crate::pickers::adapters::gradients::panel_menu(&mut app, ui, state),
            "patterns" => crate::pickers::adapters::patterns::panel_menu(&mut app, ui, state),
            "swatches" => crate::pickers::adapters::swatches::panel_menu(&mut app, ui, state),
            "styles" => {
                if crate::menu_keys::menu_item_button(ui, tl!("Append Default Styles…"), true).clicked() {
                    let _ = crate::pickers::browser::browser_actions::run(&mut app, "style.presets.reset", serde_json::json!({"append": true}));
                    ui.close();
                }
                unavailable_asset_actions_menu(ui, "Styles");
            }
            "shapes" => {
                if crate::menu_keys::menu_item_button(ui, tl!("Append Default Shapes…"), true).clicked() {
                    let _ = crate::pickers::browser::browser_actions::run(&mut app, "shape.presets.reset", serde_json::json!({"append": true}));
                    ui.close();
                }
                unavailable_asset_actions_menu(ui, "Shapes");
            }
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
    use crate::pickers::browser::asset_browser::{GroupView, ItemView};
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
            "Export Selected Gradients…",
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

        for label in ["Rename Gradients…", "Delete Gradients…"] {
            assert!(harness.get_by_label(label).accesskit_node().is_disabled(), "{label} is disabled without a selection");
        }
        assert!(harness.get_by_label("Export Selected Gradients…").accesskit_node().is_disabled(), "export is disabled without a selection");
        assert!(harness.query_by_label("Rename Groups…").is_none(), "group actions are hidden without a group selection");
        assert!(harness.query_by_label("Delete Groups…").is_none(), "group actions are hidden without a group selection");
    }

    #[test]
    fn gradients_panel_menu_enables_export_for_selected_items() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let target = ItemView { key: "Sunset".into(), name: "Sunset".into(), group_path: Vec::new() };
        let mut harness = Harness::builder().with_size(vec2(520.0, 520.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                app.ui.presets_ui.browser_selection.insert(
                    "gradients".into(),
                    std::collections::BTreeSet::from([crate::pickers::browser::path_identity_model::PickerId::item("Basics", "Sunset")]),
                );
                app.ui.presets_ui.browser_selection_snapshot.insert(
                    "gradients".into(),
                    crate::pickers::state::BrowserSelectionSnapshot {
                        contains_selected_card: true,
                        contains_card: true,
                        selected_cards: vec![("Basics".into(), "Sunset".into())],
                        cards: vec![("Basics".into(), "Sunset".into())],
                        selected_items: vec![("Basics".into(), target.clone())],
                        targets: vec![crate::pickers::browser::selection::SelectionTarget::Item {
                            group: "Basics".into(),
                            path: Vec::new(),
                            key: "Sunset".into(),
                        }],
                        ..Default::default()
                    },
                );
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

        assert!(!harness.get_by_label("Export Selected Gradients…").accesskit_node().is_disabled());
    }

    #[test]
    fn patterns_panel_menu_uses_shared_selection_labels_and_pattern_actions() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut harness = Harness::builder().with_size(vec2(520.0, 520.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                app.ui.presets_ui.browser_selection_snapshot.insert(
                    "patterns".into(),
                    crate::pickers::state::BrowserSelectionSnapshot {
                        contains_selected_card: true,
                        contains_card: true,
                        selected_cards: vec![("Basic".into(), "Pattern A".into()), ("Basic".into(), "Pattern B".into())],
                        cards: vec![("Basic".into(), "Pattern A".into()), ("Basic".into(), "Pattern B".into())],
                        targets: vec![
                            crate::pickers::browser::selection::SelectionTarget::Item { group: "Basic".into(), path: Vec::new(), key: "Pattern A".into() },
                            crate::pickers::browser::selection::SelectionTarget::Item { group: "Basic".into(), path: Vec::new(), key: "Pattern B".into() },
                        ],
                        ..Default::default()
                    },
                );
                crate::pickers::shell::picker_menu_popup(ui, "Panel menu", egui::Vec2::ZERO, |ui| {
                    assert!(shared_picker_menu(app, ui, "patterns"));
                });
            },
            app,
        );
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
        harness.run_steps(2);
        harness.get_by_label("Panel menu").click();
        harness.run_steps(2);

        let labels = [
            "New Pattern Preset…",
            "New Pattern Group…",
            "Rename Patterns…",
            "Delete Patterns…",
            "Append Default Patterns…",
            "Import Patterns…",
            "Export Selected Patterns…",
            "Export Patterns…",
        ];
        let mut previous_bottom = None;
        for label in labels {
            let rect = harness.get_by_label(label).rect();
            if let Some(bottom) = previous_bottom {
                assert!(rect.top() >= bottom, "{label} must follow the previous menu entry");
            }
            previous_bottom = Some(rect.bottom());
        }
        assert!(!harness.get_by_label("Append Default Patterns…").accesskit_node().is_disabled());
        assert!(!harness.get_by_label("Export Selected Patterns…").accesskit_node().is_disabled());
        assert!(!harness.get_by_label("Export Patterns…").accesskit_node().is_disabled());
        assert!(harness.get_by_label("New Pattern Preset…").accesskit_node().is_disabled());
    }

    #[test]
    fn styles_and_shapes_expose_default_append_while_import_export_remain_disabled() {
        for panel in ["styles", "shapes"] {
            let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            let mut harness = Harness::builder().with_size(vec2(520.0, 520.0)).build_ui_state(
                |ui, app: &mut PhotocraftApp| {
                    crate::pickers::shell::picker_menu_popup(ui, "Panel menu", egui::Vec2::ZERO, |ui| {
                        assert!(shared_picker_menu(app, ui, panel));
                    });
                },
                app,
            );
            PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
            harness.run_steps(2);
            harness.get_by_label("Panel menu").click();
            harness.run_steps(2);

            let plural = if panel == "styles" { "Styles" } else { "Shapes" };
            let append_label = format!("Append Default {plural}…");
            let append = harness.get_by_label(&append_label);
            assert!(!append.accesskit_node().is_disabled());
            for action in [format!("Import {plural}…"), format!("Export Selected {plural}…"), format!("Export {plural}…")] {
                assert!(harness.get_by_label(&action).accesskit_node().is_disabled(), "{panel} keeps unsupported action {action} visible but disabled");
            }
        }
    }

    #[test]
    fn swatches_panel_menu_matches_the_requested_action_order() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut harness = Harness::builder().with_size(vec2(520.0, 520.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                let selected = ItemView { key: "Black".into(), name: "Black".into(), group_path: Vec::new() };
                app.ui.presets_ui.browser_selection_snapshot.insert(
                    "swatches".into(),
                    crate::pickers::state::BrowserSelectionSnapshot {
                        contains_selected_card: true,
                        contains_card: true,
                        selected_items: vec![("Grays".into(), selected)],
                        selected_cards: vec![("Grays".into(), "Black".into())],
                        cards: vec![("Grays".into(), "Black".into())],
                        targets: vec![crate::pickers::browser::selection::SelectionTarget::Item {
                            group: "Grays".into(),
                            path: Vec::new(),
                            key: "Black".into(),
                        }],
                        ..Default::default()
                    },
                );
                crate::pickers::shell::picker_menu_popup(ui, "Panel menu", egui::Vec2::ZERO, |ui| {
                    assert!(shared_picker_menu(app, ui, "swatches"));
                });
            },
            app,
        );
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
        harness.run_steps(2);
        harness.get_by_label("Panel menu").click();
        harness.run_steps(2);

        let labels = [
            "New Swatch Preset…",
            "New Swatch Group…",
            "Show Recents",
            "Append Default Swatches…",
            "Import Swatches…",
            "Export Selected Swatches…",
            "Export Swatches for Exchange…",
        ];
        let mut previous_bottom = None;
        for label in labels {
            let rect = harness.get_by_label(label).rect();
            if let Some(bottom) = previous_bottom {
                assert!(rect.top() >= bottom, "{label} must follow the previous menu entry");
            }
            previous_bottom = Some(rect.bottom());
        }
        assert!(!harness.get_by_label("New Swatch Preset…").accesskit_node().is_disabled());
        assert!(!harness.get_by_label("Export Selected Swatches…").accesskit_node().is_disabled());
        assert!(harness.query_by_label("Replace Swatches…").is_none());
        assert!(harness.query_by_label("Reset Swatches").is_none());
        assert!(harness.query_by_label("Export Swatches…").is_none());
        let selected = harness.get_by_label("Export Selected Swatches…").rect();
        let exchange = harness.get_by_label("Export Swatches for Exchange…").rect();
        assert!(exchange.top() >= selected.bottom(), "Exchange export follows selected export");
    }

    #[test]
    fn gradient_menu_disables_rename_and_delete_when_canonical_selection_is_empty() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut state = PresetUi::default();
        state.browser_selection.insert("gradients".into(), std::collections::BTreeSet::new());
        let mut harness = Harness::builder().with_size(vec2(520.0, 520.0)).build_ui_state(
            |ui, (app, state): &mut (PhotocraftApp, PresetUi)| {
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

        for label in ["Rename Gradients…", "Delete Gradients…"] {
            assert!(harness.get_by_label(label).accesskit_node().is_disabled(), "{label} must be disabled with an empty visible selection");
        }
    }

    #[test]
    fn gradient_menu_opens_the_new_preset_name_dialog_without_creating_a_preset_yet() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let before = app.session.presets.gradients.iter().map(|group| group.items.len()).sum::<usize>();
        let mut harness = Harness::builder().with_size(vec2(420.0, 360.0)).build_ui_state(
            |ui, (app, state): &mut (PhotocraftApp, PresetUi)| {
                crate::pickers::shell::picker_menu_popup(ui, "Panel menu", egui::Vec2::ZERO, |ui| {
                    super::browser_actions_menu(app, ui, state, "gradients");
                });
                let _ = crate::pickers::browser::popup_dialogs::show_new_gradient_preset_dialog(ui);
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
        assert_eq!(after, before, "opening the name dialog must not create a preset before confirmation");
        assert!(harness.query_by_label("Name").is_some(), "the name field must be visible");
        assert!(harness.query_by_label("OK").is_some(), "the dialog must expose confirmation");
        assert!(harness.output().shapes.iter().any(|shape| matches!(&shape.shape, egui::epaint::Shape::Text(text) if text.galley.text() == "Gradient Name")));
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
        state.browser_selection_snapshot.insert(
            "gradients".into(),
            crate::pickers::state::BrowserSelectionSnapshot {
                contains_selected_card: true,
                contains_card: true,
                selected_cards: vec![("Basics".into(), "one".into()), ("Basics".into(), "two".into())],
                cards: vec![("Basics".into(), "one".into()), ("Basics".into(), "two".into())],
                selected_items: targets.clone(),
                targets: targets
                    .iter()
                    .map(|(group, item)| crate::pickers::browser::selection::SelectionTarget::Item {
                        group: group.clone(),
                        path: item.group_path.clone(),
                        key: item.key.clone(),
                    })
                    .collect(),
                ..Default::default()
            },
        );
        let mut harness = Harness::builder().with_size(vec2(640.0, 360.0)).build_ui_state(
            |ui, (app, state): &mut (PhotocraftApp, PresetUi)| {
                crate::pickers::shell::picker_menu_popup(ui, "Panel menu", egui::Vec2::ZERO, |ui| {
                    browser_actions_menu(app, ui, state, "gradients");
                });
                let groups = [GroupView { name: "Basics".into(), root_only: false, items: targets.iter().map(|(_, item)| item.clone()).collect() }];
                let mut actions = Vec::new();
                let mut paint = |ui: &egui::Ui, rect: egui::Rect, _: usize, _: usize, _: crate::pickers::browser::asset_browser::ItemPaintState| {
                    ui.painter().rect_filled(rect, 0.0, egui::Color32::WHITE);
                };
                crate::pickers::browser::popup_dialogs::show_rename_selection_dialog(ui, "gradients", &groups, &mut actions, &mut paint);
            },
            (app, state),
        );
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
        harness.run_steps(2);
        harness.get_by_label("Panel menu").click();
        harness.run_steps(2);
        assert!(
            harness.get_by_label("Export Selected Gradients…").rect().top() > harness.get_by_label("Delete Gradients…").rect().top(),
            "export stays in the trailing domain-action section after selection actions"
        );
        harness.get_by_label("Rename Gradients…").click();
        harness.run_steps(2);
        assert!(
            harness.ctx.data(|data| data.get_temp::<bool>(egui::Id::new(("picker-rename-card-dialog", "gradients", "request"))).unwrap_or(false)),
            "the shared card dialog should be active for an item-only selection"
        );

        assert!(harness.query_by_label("Name").is_some(), "card rename uses the popup name field");
        assert!(harness.state().1.item_renaming.is_none(), "card rename no longer uses the old inline rename column");
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
        state.browser_selection_snapshot.insert(
            "gradients".into(),
            crate::pickers::state::BrowserSelectionSnapshot {
                groups: vec!["Basics".into(), "Grays".into()],
                contains_group: true,
                targets: vec![
                    crate::pickers::browser::selection::SelectionTarget::Group("Basics".into()),
                    crate::pickers::browser::selection::SelectionTarget::Group("Grays".into()),
                ],
                ..Default::default()
            },
        );
        let mut harness = Harness::builder().with_size(vec2(420.0, 360.0)).build_ui_state(
            |ui, (app, state): &mut (PhotocraftApp, PresetUi)| {
                crate::pickers::shell::picker_menu_popup(ui, "Panel menu", egui::Vec2::ZERO, |ui| {
                    browser_actions_menu(app, ui, state, "gradients");
                });
                let mut actions = Vec::new();
                let mut paint = |_: &egui::Ui, _: egui::Rect, _: usize, _: usize, _: crate::pickers::browser::asset_browser::ItemPaintState| {};
                crate::pickers::browser::popup_dialogs::show_rename_selection_dialog(ui, "gradients", &[], &mut actions, &mut paint);
            },
            (app, state),
        );
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
        harness.run_steps(2);
        harness.get_by_label("Panel menu").click();
        harness.run_steps(2);
        harness.get_by_label("Rename Groups…").click();
        harness.run_steps(4);

        assert!(harness.query_by_label("Rename Gradient…").is_none(), "group selection hides card rename");
        assert!(harness.query_by_label("Delete Gradient…").is_none(), "group selection hides card delete");
        assert!(harness.query_by_label("Name").is_some(), "group rename uses the popup name field");
        assert!(harness.state().1.renaming.is_none(), "group rename no longer uses the old inline rename column");
        assert!(harness.output().shapes.iter().any(|shape| matches!(&shape.shape, egui::epaint::Shape::Text(text) if text.galley.text() == "Rename Group")));
    }

    #[test]
    fn single_selected_gradient_group_uses_singular_panel_menu_labels() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut state = PresetUi::default();
        state
            .browser_selection
            .insert("gradients".into(), std::collections::BTreeSet::from([crate::pickers::browser::path_identity_model::PickerId::group("Basics")]));
        state.browser_selection_snapshot.insert(
            "gradients".into(),
            crate::pickers::state::BrowserSelectionSnapshot {
                groups: vec!["Basics".into()],
                contains_group: true,
                targets: vec![crate::pickers::browser::selection::SelectionTarget::Group("Basics".into())],
                ..Default::default()
            },
        );
        let mut harness = Harness::builder().with_size(vec2(420.0, 360.0)).build_ui_state(
            |ui, (app, state): &mut (PhotocraftApp, PresetUi)| {
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

        assert!(harness.query_by_label("Rename Group…").is_some());
        assert!(harness.query_by_label("Delete Group…").is_some());
        assert!(harness.query_by_label("Rename Groups…").is_none());
    }
}
