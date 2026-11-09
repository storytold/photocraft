//! Regression tests for selection-driven browser actions.

use std::collections::{BTreeMap, BTreeSet};

use super::asset_browser::BrowserAction;
use super::browser_actions::generic_with_state;
use super::context_menu::{BrowserDeleteContext, SelectionMenuModel, selected_delete_actions};
use super::identity::PickerNodeIdentity;
use super::path_identity_model::{HierarchyNode, PickerBrowserHierarchy, PickerId};

#[test]
fn selected_parent_group_coalesces_descendant_delete_actions() {
    let parent = PickerId::group("Parent");
    let nested = PickerId::group_path(["Parent"], "Nested");
    let inner = PickerId::group_path(["Parent", "Nested"], "Inner");
    let deep_item = PickerId::item_in_path(["Parent", "Nested", "Inner"], "deep-item");
    let root_item = PickerId::item("Parent", "root-item");
    let sibling = PickerId::group("Sibling");
    let hierarchy = PickerBrowserHierarchy::new(vec![
        HierarchyNode::group(
            parent.clone(),
            vec![
                HierarchyNode::item(root_item.clone()),
                HierarchyNode::group(nested.clone(), vec![HierarchyNode::group(inner.clone(), vec![HierarchyNode::item(deep_item.clone())])]),
            ],
        ),
        HierarchyNode::group(sibling.clone(), vec![HierarchyNode::item(PickerId::item("Sibling", "keep-me"))]),
    ])
    .unwrap();
    let identities = BTreeMap::from([
        (parent.clone(), PickerNodeIdentity::Group("Parent".into())),
        (nested.clone(), PickerNodeIdentity::Folder { group: "Parent".into(), path: vec!["Nested".into()] }),
        (inner.clone(), PickerNodeIdentity::Folder { group: "Parent".into(), path: vec!["Nested".into(), "Inner".into()] }),
        (deep_item.clone(), PickerNodeIdentity::FolderItem { group: "Parent".into(), path: vec!["Nested".into(), "Inner".into()], key: "deep-item".into() }),
        (root_item.clone(), PickerNodeIdentity::Item { group: "Parent".into(), key: "root-item".into() }),
        (sibling, PickerNodeIdentity::Group("Sibling".into())),
    ]);
    let selection = BTreeSet::from([parent.clone(), nested, inner, deep_item, root_item]);
    let context = BrowserDeleteContext { hierarchy: &hierarchy, identities: &identities };

    let actions = selected_delete_actions(&selection, &parent, &context).unwrap();

    assert!(matches!(actions.as_slice(), [BrowserAction::DeleteGroup(group)] if group == "Parent"));
}

#[test]
fn selected_nested_group_coalesces_its_descendant_delete_actions() {
    let parent = PickerId::group("Parent");
    let nested = PickerId::group_path(["Parent"], "Nested");
    let deep = PickerId::group_path(["Parent", "Nested"], "Deep");
    let deep_item = PickerId::item_in_path(["Parent", "Nested", "Deep"], "deep-item");
    let hierarchy = PickerBrowserHierarchy::new(vec![HierarchyNode::group(
        parent.clone(),
        vec![HierarchyNode::group(nested.clone(), vec![HierarchyNode::group(deep.clone(), vec![HierarchyNode::item(deep_item.clone())])])],
    )])
    .unwrap();
    let identities = BTreeMap::from([
        (parent, PickerNodeIdentity::Group("Parent".into())),
        (nested.clone(), PickerNodeIdentity::Folder { group: "Parent".into(), path: vec!["Nested".into()] }),
        (deep.clone(), PickerNodeIdentity::Folder { group: "Parent".into(), path: vec!["Nested".into(), "Deep".into()] }),
        (deep_item.clone(), PickerNodeIdentity::FolderItem { group: "Parent".into(), path: vec!["Nested".into(), "Deep".into()], key: "deep-item".into() }),
    ]);
    let selection = BTreeSet::from([nested.clone(), deep, deep_item]);
    let context = BrowserDeleteContext { hierarchy: &hierarchy, identities: &identities };

    let actions = selected_delete_actions(&selection, &nested, &context).unwrap();

    assert!(matches!(actions.as_slice(), [BrowserAction::DeleteNestedGroup { group, group_path }] if group == "Parent" && group_path == &["Nested"]));
}

#[test]
fn rename_preserves_selected_group_folder_and_card_while_delete_coalesces_them() {
    use super::selection::{SelectionTarget, resolve};
    use crate::pickers::browser::asset_browser::{GroupView, ItemView};

    let group = PickerId::group("Parent");
    let folder = PickerId::group_path(["Parent"], "Nested");
    let card = PickerId::item_in_path(["Parent", "Nested"], "card");
    let hierarchy = PickerBrowserHierarchy::new(vec![HierarchyNode::group(
        group.clone(),
        vec![HierarchyNode::group(folder.clone(), vec![HierarchyNode::item(card.clone())])],
    )])
    .unwrap();
    let identities = BTreeMap::from([
        (group.clone(), PickerNodeIdentity::Group("Parent".into())),
        (folder.clone(), PickerNodeIdentity::Folder { group: "Parent".into(), path: vec!["Nested".into()] }),
        (card.clone(), PickerNodeIdentity::FolderItem { group: "Parent".into(), path: vec!["Nested".into()], key: "card".into() }),
    ]);
    let selected = BTreeSet::from([group, folder, card]);
    let groups = [GroupView {
        name: "Parent".into(),
        root_only: false,
        items: vec![ItemView { key: "card".into(), name: "Card".into(), group_path: vec!["Nested".into()] }],
    }];

    let resolved = resolve(&selected, &hierarchy, &identities, &groups);

    assert_eq!(
        resolved.targets,
        [
            SelectionTarget::Group("Parent".into()),
            SelectionTarget::Folder { group: "Parent".into(), path: vec!["Nested".into()] },
            SelectionTarget::Item { group: "Parent".into(), path: vec!["Nested".into()], key: "card".into() },
        ]
    );
    assert!(matches!(
        super::selection_operations::rename_selection(&resolved),
        Some(BrowserAction::BeginRenameSelection { targets })
            if targets == [
                SelectionTarget::Item { group: "Parent".into(), path: vec!["Nested".into()], key: "card".into() },
                SelectionTarget::Folder { group: "Parent".into(), path: vec!["Nested".into()] },
                SelectionTarget::Group("Parent".into()),
            ]
    ));
    assert!(matches!(super::selection_operations::delete_selection(&resolved).as_slice(), [BrowserAction::DeleteGroup(name)] if name == "Parent"));
    let reverse_visual_order = vec![
        SelectionTarget::Item { group: "Parent".into(), path: vec!["Nested".into()], key: "card".into() },
        SelectionTarget::Folder { group: "Parent".into(), path: vec!["Nested".into()] },
        SelectionTarget::Group("Parent".into()),
    ];
    assert!(matches!(
        super::selection_operations::delete_targets_actions(&reverse_visual_order).as_slice(),
        [BrowserAction::DeleteGroup(name)] if name == "Parent"
    ));
}

#[test]
fn folder_selection_is_one_rename_target_but_covers_its_descendant_cards_for_other_operations() {
    use super::selection::{SelectionTarget, resolve};
    use crate::pickers::browser::asset_browser::{GroupView, ItemView};

    let parent = PickerId::group("Parent");
    let folder = PickerId::group_path(["Parent"], "Folder");
    let card = PickerId::item_in_path(["Parent", "Folder"], "inside");
    let hierarchy = PickerBrowserHierarchy::new(vec![HierarchyNode::group(
        parent.clone(),
        vec![HierarchyNode::group(folder.clone(), vec![HierarchyNode::item(card.clone())])],
    )])
    .unwrap();
    let identities = BTreeMap::from([
        (parent, PickerNodeIdentity::Group("Parent".into())),
        (folder.clone(), PickerNodeIdentity::Folder { group: "Parent".into(), path: vec!["Folder".into()] }),
        (card, PickerNodeIdentity::FolderItem { group: "Parent".into(), path: vec!["Folder".into()], key: "inside".into() }),
    ]);
    let selected = BTreeSet::from([folder]);
    let groups = [GroupView {
        name: "Parent".into(),
        root_only: false,
        items: vec![ItemView { key: "inside".into(), name: "Inside".into(), group_path: vec!["Folder".into()] }],
    }];

    let resolved = resolve(&selected, &hierarchy, &identities, &groups);

    assert_eq!(resolved.targets, [SelectionTarget::Folder { group: "Parent".into(), path: vec!["Folder".into()] }]);
    assert!(resolved.selected_items.is_empty(), "a selected folder does not implicitly select each card for rename");
    assert_eq!(resolved.cards, [("Parent".into(), "inside".into())], "folder contents remain available to operations that expand containers");
    assert!(matches!(
        super::selection_operations::rename_selection(&resolved),
        Some(BrowserAction::BeginRenameSelection { targets }) if targets == resolved.targets
    ));
    assert!(matches!(
        super::selection_operations::delete_selection(&resolved).as_slice(),
        [BrowserAction::DeleteNestedGroup { group, group_path }] if group == "Parent" && group_path == &["Folder"]
    ));
}

#[test]
fn selected_groups_are_kept_together_in_the_group_rename_batch() {
    let groups = vec!["First".to_owned(), "Second".to_owned(), "Third".to_owned()];
    let selection = crate::pickers::state::BrowserSelectionSnapshot {
        contains_group: true,
        groups: groups.clone(),
        targets: groups.iter().cloned().map(super::selection::SelectionTarget::Group).collect(),
        ..Default::default()
    };
    let menu = SelectionMenuModel::new("Brush", &selection);

    assert_eq!(menu.rename_label, "Rename Groups…");
    assert!(
        matches!(menu.rename_action, Some(BrowserAction::BeginRenameSelection { targets }) if targets == groups.into_iter().map(super::selection::SelectionTarget::Group).collect::<Vec<_>>())
    );
}

#[test]
fn group_rename_dialog_receives_every_selected_group_in_sequence() {
    let groups = vec!["First".to_owned(), "Second".to_owned(), "Third".to_owned()];
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(420.0, 320.0)).build_ui_state(
        |ui, actions: &mut Vec<BrowserAction>| {
            let mut paint = |_: &egui::Ui, _: egui::Rect, _: usize, _: usize, _: crate::pickers::browser::asset_browser::ItemPaintState| {};
            super::popup_dialogs::show_rename_selection_dialog(ui, "action-tests", &[], actions, &mut paint);
        },
        vec![BrowserAction::BeginRenameSelection { targets: groups.iter().cloned().map(super::selection::SelectionTarget::Group).collect() }],
    );
    crate::PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
    harness.run_steps(2);

    let request = egui::Id::new(("picker-rename-group-dialog", "action-tests"));
    let target = harness.ctx.data(|data| data.get_temp::<super::selection::SelectionTarget>(request.with("target"))).unwrap();
    let queue = harness.ctx.data(|data| data.get_temp::<Vec<super::selection::SelectionTarget>>(request.with("queue"))).unwrap();
    assert_eq!(target, super::selection::SelectionTarget::Group("First".into()));
    assert_eq!(queue, [super::selection::SelectionTarget::Group("Second".into()), super::selection::SelectionTarget::Group("Third".into())]);
}

#[test]
fn deleting_a_group_action_deletes_its_nested_groups_in_the_generic_picker() {
    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    for group in app.session.presets.gradients.clone() {
        app.run("gradient.presets.edit", serde_json::json!({"action": "deleteGroup", "group": group.name})).unwrap();
    }
    for name in ["A", "B", "C", "Sibling"] {
        app.run("gradient.presets.edit", serde_json::json!({"action": "newGroup", "name": name})).unwrap();
    }
    let mut state = crate::pickers::state::PresetUi::default();
    state.set_hierarchy_order(
        "gradients",
        "",
        vec![crate::pickers::state::HierarchyOrderEntry::Group("A".into()), crate::pickers::state::HierarchyOrderEntry::Group("Sibling".into())],
    );
    state.set_hierarchy_order("gradients", "A", vec![crate::pickers::state::HierarchyOrderEntry::Group("B".into())]);
    state.set_hierarchy_order("gradients", "B", vec![crate::pickers::state::HierarchyOrderEntry::Group("C".into())]);

    assert!(generic_with_state(&mut app, &mut state, "gradients", "gradient.presets", &BrowserAction::DeleteGroup("A".into()), serde_json::json!({}),));

    let remaining = app.session.presets.gradients.iter().map(|group| group.name.as_str()).collect::<Vec<_>>();
    assert_eq!(remaining, ["Sibling"]);
    assert_eq!(state.hierarchy_order["gradients"][""], [crate::pickers::state::HierarchyOrderEntry::Group("Sibling".into())]);
}

#[test]
fn renaming_a_nested_gradient_header_renames_that_group_not_its_parent() {
    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    for group in app.session.presets.gradients.clone() {
        app.run("gradient.presets.edit", serde_json::json!({"action": "deleteGroup", "group": group.name})).unwrap();
    }
    for name in ["Parent", "Folder"] {
        app.run("gradient.presets.edit", serde_json::json!({"action": "newGroup", "name": name})).unwrap();
    }
    let mut state = crate::pickers::state::PresetUi::default();
    state.set_hierarchy_order("gradients", "", vec![crate::pickers::state::HierarchyOrderEntry::Group("Parent".into())]);
    state.set_hierarchy_order("gradients", "Parent", vec![crate::pickers::state::HierarchyOrderEntry::Group("Folder".into())]);
    state.browser_selection.insert("gradients".into(), BTreeSet::from([PickerId::group_path(["Parent"], "Folder")]));
    state.browser_selection_snapshot.insert("gradients".into(), Default::default());

    assert!(generic_with_state(
        &mut app,
        &mut state,
        "gradients",
        "gradient.presets",
        &BrowserAction::RenameNestedGroup { group: "Parent".into(), group_path: vec!["Folder".into()], text: "Renamed Folder".into() },
        serde_json::json!({}),
    ));

    assert!(app.session.presets.gradients.iter().any(|group| group.name == "Parent"));
    assert!(app.session.presets.gradients.iter().any(|group| group.name == "Renamed Folder"));
    assert!(!app.session.presets.gradients.iter().any(|group| group.name == "Folder"));
    assert_eq!(state.hierarchy_order["gradients"]["Parent"], [crate::pickers::state::HierarchyOrderEntry::Group("Renamed Folder".into())]);
    assert!(state.browser_selection["gradients"].is_empty(), "renaming invalidates the old folder identity");
    assert!(!state.browser_selection_snapshot.contains_key("gradients"), "the derived menu snapshot is invalidated with its source selection");
}

#[test]
fn deleting_a_nested_gradient_header_recursively_deletes_its_subtree() {
    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    for group in app.session.presets.gradients.clone() {
        app.run("gradient.presets.edit", serde_json::json!({"action": "deleteGroup", "group": group.name})).unwrap();
    }
    for name in ["Parent", "Folder", "Child", "Sibling"] {
        app.run("gradient.presets.edit", serde_json::json!({"action": "newGroup", "name": name})).unwrap();
    }
    let mut state = crate::pickers::state::PresetUi::default();
    state.set_hierarchy_order(
        "gradients",
        "",
        vec![crate::pickers::state::HierarchyOrderEntry::Group("Parent".into()), crate::pickers::state::HierarchyOrderEntry::Group("Sibling".into())],
    );
    state.set_hierarchy_order("gradients", "Parent", vec![crate::pickers::state::HierarchyOrderEntry::Group("Folder".into())]);
    state.set_hierarchy_order("gradients", "Folder", vec![crate::pickers::state::HierarchyOrderEntry::Group("Child".into())]);

    assert!(generic_with_state(
        &mut app,
        &mut state,
        "gradients",
        "gradient.presets",
        &BrowserAction::DeleteNestedGroup { group: "Parent".into(), group_path: vec!["Folder".into()] },
        serde_json::json!({}),
    ));

    let remaining = app.session.presets.gradients.iter().map(|group| group.name.as_str()).collect::<Vec<_>>();
    assert_eq!(remaining, ["Parent", "Sibling"]);
    assert_eq!(
        state.hierarchy_order["gradients"][""],
        [crate::pickers::state::HierarchyOrderEntry::Group("Parent".into()), crate::pickers::state::HierarchyOrderEntry::Group("Sibling".into())]
    );
    assert!(!state.hierarchy_order["gradients"].contains_key("Folder"));
    assert!(!state.hierarchy_order["gradients"].contains_key("Child"));
}

#[test]
fn mixed_gradient_deletion_removes_selected_group_and_uncovered_card_once() {
    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    for group in app.session.presets.gradients.clone() {
        app.run("gradient.presets.edit", serde_json::json!({"action": "deleteGroup", "group": group.name})).unwrap();
    }
    for group in ["A", "B"] {
        app.run("gradient.presets.edit", serde_json::json!({"action": "newGroup", "name": group})).unwrap();
    }
    for (group, name) in [("A", "Gradient A"), ("B", "Gradient B")] {
        app.run("gradient.presets.new", serde_json::json!({"name": name, "group": group})).unwrap();
    }

    let mut state = crate::pickers::state::PresetUi::default();
    assert!(generic_with_state(&mut app, &mut state, "gradients", "gradient.presets", &BrowserAction::DeleteGroup("B".into()), serde_json::json!({}),));
    assert!(generic_with_state(
        &mut app,
        &mut state,
        "gradients",
        "gradient.presets",
        &BrowserAction::DeleteItem { group: "A".into(), group_path: Vec::new(), key: "Gradient A".into() },
        serde_json::json!({}),
    ));

    assert!(!app.session.presets.gradients.iter().any(|group| group.name == "B"));
    assert!(app.session.presets.gradients.iter().find(|group| group.name == "A").is_some_and(|group| group.items.is_empty()));
    let deleted_group_b = app
        .session
        .journal
        .iter()
        .filter(|(command, params)| command == "gradient.presets.edit" && params["action"] == "deleteGroup" && params["group"] == "B")
        .count();
    let deleted_gradient_a = app
        .session
        .journal
        .iter()
        .filter(|(command, params)| command == "gradient.presets.edit" && params["action"] == "delete" && params["preset"] == "Gradient A")
        .count();
    assert_eq!(deleted_group_b, 1, "the selected group is deleted exactly once");
    assert_eq!(deleted_gradient_a, 1, "the uncovered card is deleted exactly once");
    assert!(app.session.presets.gradients.iter().all(|group| group.items.iter().all(|item| item.name != "Gradient A" && item.name != "Gradient B")));
}
