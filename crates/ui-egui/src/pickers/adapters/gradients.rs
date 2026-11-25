//! Gradient-library adapter for the shared preset browser.

use egui::{Rect, pos2};
use serde_json::json;

use crate::PhotocraftApp;
use crate::pickers::browser::asset_browser::{
    BrowserAction, BrowserAdapter, BrowserCapabilities, BrowserOptions, GroupView, ItemPaintState, ItemView, standard_group_context_menu,
    standard_item_context_menu,
};
use crate::pickers::browser::browser_actions::{expand_creation_destination, generic, run};
use crate::pickers::browser::browser_interaction::canvas_drop_rect;
use crate::pickers::browser::identity::{PickerIdentityKey, PickerNodeIdentity, PickerProjectionKeys, ProjectionNodeRef};
use crate::pickers::browser::inline_rename::ItemRenameState;
use crate::pickers::footer::FooterOptions;
use crate::pickers::preset_previews::c32;
use crate::pickers::shell::{ShellOptions, picker_shell};
use crate::pickers::state::HierarchyOrderEntry;

/// The engine's fallback owner for gradients created when the library has no groups. The shared
/// hierarchy treats it as an invisible backing group while its cards live at the root.
// Keep the backing owner distinct from ordinary user-created groups. `Presets` was previously
// used for this purpose, which made a perfectly valid user group with that name disappear.
const ROOT_ITEMS_GROUP: &str = "__photocraft_root_items__";

fn gradient_projection_keys(groups: &[GroupView]) -> PickerProjectionKeys {
    let mut keys = PickerProjectionKeys::default();
    for (group_index, group) in groups.iter().enumerate() {
        keys.insert(
            ProjectionNodeRef::Group(group_index),
            PickerIdentityKey::from_parts("gradient-group", [group.name.as_str()]),
            PickerNodeIdentity::Group(group.name.clone()),
        );
        for (item_index, item) in group.items.iter().enumerate() {
            keys.insert(
                ProjectionNodeRef::Item { group_index, item_index },
                PickerIdentityKey::from_parts("gradient-preset", [item.key.as_str()]),
                PickerNodeIdentity::Item { group: group.name.clone(), key: item.key.clone() },
            );
        }
    }
    keys
}

fn insert_root_gradient_after(
    state: &mut crate::pickers::state::PresetUi,
    anchor: Option<&crate::pickers::browser::selection::SelectionTarget>,
    existing_root_items: &[String],
    name: &str,
) {
    use crate::pickers::browser::selection::SelectionTarget;

    let mut root = state.hierarchy_order.get("gradients").and_then(|orders| orders.get("")).cloned().unwrap_or_default();
    let entry = HierarchyOrderEntry::RootItem { group: ROOT_ITEMS_GROUP.into(), key: name.into() };
    root.retain(|existing| existing != &entry);
    let represented: std::collections::BTreeSet<&str> = root
        .iter()
        .filter_map(|entry| match entry {
            HierarchyOrderEntry::RootItem { key, .. } => Some(key.as_str()),
            _ => None,
        })
        .collect();
    let mut missing: Vec<_> = existing_root_items
        .iter()
        .filter(|key| key.as_str() != name && !represented.contains(key.as_str()))
        .map(|key| HierarchyOrderEntry::RootItem { group: ROOT_ITEMS_GROUP.into(), key: key.clone() })
        .collect();
    missing.append(&mut root);
    let mut root = missing;
    let after = anchor.and_then(|target| match target {
        SelectionTarget::Item { group, key, path } if group == ROOT_ITEMS_GROUP && path.is_empty() => {
            let selected = HierarchyOrderEntry::RootItem { group: ROOT_ITEMS_GROUP.into(), key: key.clone() };
            root.iter().position(|existing| existing == &selected).map(|index| index + 1)
        }
        _ => None,
    });
    let at = after.unwrap_or(root.len()).min(root.len());
    root.insert(at, entry);
    state.set_hierarchy_order("gradients", "", root);
}

fn insert_group_gradient_after(
    state: &mut crate::pickers::state::PresetUi,
    group: &str,
    anchor: Option<&crate::pickers::browser::selection::SelectionTarget>,
    existing_items: &[String],
    name: &str,
) {
    use crate::pickers::browser::selection::SelectionTarget;

    let mut order = state.hierarchy_order.get("gradients").and_then(|orders| orders.get(group)).cloned().unwrap_or_default();
    let entry = HierarchyOrderEntry::Item(name.into());
    order.retain(|existing| existing != &entry);
    let represented: std::collections::BTreeSet<&str> = order
        .iter()
        .filter_map(|entry| match entry {
            HierarchyOrderEntry::Item(key) => Some(key.as_str()),
            _ => None,
        })
        .collect();
    let mut missing: Vec<_> = existing_items
        .iter()
        .filter(|key| key.as_str() != name && !represented.contains(key.as_str()))
        .map(|key| HierarchyOrderEntry::Item(key.clone()))
        .collect();
    missing.append(&mut order);
    let mut order = missing;
    let after = anchor.and_then(|target| match target {
        SelectionTarget::Item { group: selected_group, key, path } if selected_group == group && path.is_empty() => {
            order.iter().position(|existing| existing == &HierarchyOrderEntry::Item(key.clone())).map(|index| index + 1)
        }
        _ => None,
    });
    let at = after.unwrap_or(order.len()).min(order.len());
    order.insert(at, entry);
    state.set_hierarchy_order("gradients", group, order);
}

/// Draw a gradient swatch, showing transparency over the standard checkerboard.
pub(crate) fn paint_gradient(ui: &egui::Ui, rect: Rect, stops: &[(f32, [f32; 4])]) {
    let painter = ui.painter_at(rect);
    if stops.iter().any(|stop| stop.1[3] < 0.999) {
        crate::widgets::checker(&painter, rect, (rect.height() / 4.0).clamp(3.0, 8.0));
    }
    let x = |t: f32| rect.left() + rect.width() * t.clamp(0.0, 1.0);
    let mut points = stops.to_vec();
    if points.first().is_some_and(|stop| stop.0 > 0.0) {
        points.insert(0, (0.0, points[0].1));
    }
    if let Some(last) = points.last().copied().filter(|stop| stop.0 < 1.0) {
        points.push((1.0, last.1));
    }

    let mut mesh = egui::Mesh::default();
    for pair in points.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        let index = mesh.vertices.len() as u32;
        let (start_color, end_color) = (c32(start.1), c32(end.1));
        mesh.colored_vertex(pos2(x(start.0), rect.top()), start_color);
        mesh.colored_vertex(pos2(x(end.0), rect.top()), end_color);
        mesh.colored_vertex(pos2(x(end.0), rect.bottom()), end_color);
        mesh.colored_vertex(pos2(x(start.0), rect.bottom()), start_color);
        mesh.add_triangle(index, index + 1, index + 2);
        mesh.add_triangle(index, index + 2, index + 3);
    }
    painter.add(mesh);
}

/// Draw the standard Gradients panel using the shared browser and shell.
pub(crate) fn gradients_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let (foreground, background) = (app.session.tools.foreground, app.session.tools.background);
    let library = app.session.presets.gradients.clone();
    // The reserved fallback group is the durable owner for root cards. Unlike generic picker
    // layout state, the gradient library itself is saved across restarts, so membership in this
    // group is the source of truth for root-level placement.
    let groups: Vec<GroupView> = library
        .iter()
        .map(|group| GroupView {
            name: group.name.clone(),
            root_only: group.name == ROOT_ITEMS_GROUP,
            items: group.items.iter().map(|item| ItemView { key: item.name.clone(), name: item.name.clone(), group_path: Vec::new() }).collect(),
        })
        .collect();
    let mut state = std::mem::take(&mut app.ui.presets_ui);
    let root_items: std::collections::BTreeSet<String> =
        library.iter().find(|group| group.name == ROOT_ITEMS_GROUP).into_iter().flat_map(|group| group.items.iter().map(|item| item.name.clone())).collect();
    // Preserve the canonical mixed root sequence written by commit_hierarchy_move. Rebuilding
    // RootItems first on every frame silently discarded the requested insertion point, making a
    // successful drop between groups or at the bottom appear to have failed (the card jumped to
    // the beginning of the list, often outside the current viewport).
    let visible_groups: std::collections::BTreeSet<String> = groups.iter().filter(|group| !group.root_only).map(|group| group.name.clone()).collect();
    let mut root_order: Vec<HierarchyOrderEntry> = state
        .hierarchy_order
        .get("gradients")
        .and_then(|orders| orders.get(""))
        .into_iter()
        .flatten()
        .filter_map(|entry| match entry {
            // commit_hierarchy_move records the item's original library group in the UI token,
            // while the domain adapter has physically moved it into the reserved owner group.
            // Match by durable key and normalize the backing group without changing its position.
            HierarchyOrderEntry::RootItem { key, .. } if root_items.contains(key) => {
                Some(HierarchyOrderEntry::RootItem { group: ROOT_ITEMS_GROUP.into(), key: key.clone() })
            }
            HierarchyOrderEntry::Group(name) if visible_groups.contains(name) => Some(entry.clone()),
            _ => None,
        })
        .collect();
    let represented_root_items: std::collections::BTreeSet<String> = root_order
        .iter()
        .filter_map(|entry| match entry {
            HierarchyOrderEntry::RootItem { key, .. } if root_items.contains(key) => Some(key.clone()),
            _ => None,
        })
        .collect();
    // Newly discovered durable root cards (for example after an app restart) have no transient
    // mixed-order token. Seed them before the existing group sequence; subsequent drag commits
    // then preserve their explicitly chosen positions.
    let mut missing_root_items: Vec<HierarchyOrderEntry> =
        root_items.difference(&represented_root_items).map(|key| HierarchyOrderEntry::RootItem { group: ROOT_ITEMS_GROUP.into(), key: key.clone() }).collect();
    missing_root_items.append(&mut root_order);
    let root_order = missing_root_items;
    state.set_hierarchy_order("gradients", "", root_order);
    let projection_keys = gradient_projection_keys(&groups);
    let canvas = canvas_drop_rect(app);
    let events = {
        let mut commit_item_move = |source_group: &str, key: &str, destination_group: &str, index: usize| {
            run(app, "gradient.presets.edit", json!({"action": "move", "preset": key, "group": source_group, "to": destination_group, "index": index}))
                .map(|_| ())
                .ok_or_else(|| "The Gradient library rejected the item move.".to_owned())
        };
        let mut adapter = BrowserAdapter {
            panel: "gradients",
            groups: &groups,
            projection_keys: Some(&projection_keys),
            options: BrowserOptions {
                canvas,
                flush: false,
                center_grid: false,
                compact_context_menus: false,
                domain_hierarchy_actions: false,
                root_item_group: Some(ROOT_ITEMS_GROUP.to_owned()),
            },
            // Cross-group membership uses the Gradient domain command; sibling/group order is saved
            // in the canonical picker layout state.
            capabilities: BrowserCapabilities::default(),
            item_context_menu: Some(standard_item_context_menu),
            group_context_menu: Some(standard_group_context_menu),
            nested_group_context_menu: None,
            canonical_hierarchy_move: None,
            rekey_hierarchy_after_move: None,
            hierarchy_item_move: Some(&mut commit_item_move),
            paint_group: None,
            paint_item: &mut |ui, rect, group_index, item_index, _paint_state: ItemPaintState| {
                let group = &groups[group_index];
                let key = &group.items[item_index].key;
                let stops = library
                    .iter()
                    .find(|g| g.name == group.name)
                    .and_then(|g| g.items.iter().find(|item| &item.name == key))
                    .map(|item| item.resolve(foreground, background))
                    .unwrap_or_default();
                paint_gradient(ui, rect, &stops);
            },
            paint_list_item: None,
        };
        picker_shell(ui, &mut state, &mut adapter, ShellOptions { footer: Some(FooterOptions::standard(tl!("Create new gradient"))), ..Default::default() })
    };
    if let Some(name) = crate::pickers::browser::popup_dialogs::show_new_gradient_preset_dialog(ui) {
        create_named_gradient(app, &mut state, &name);
    }
    app.ui.presets_ui = state;

    for event in events {
        match &event {
            BrowserAction::SelectItem { group, group_path, key } => {
                app.ui.presets_ui.set_selected_item("gradients", group, group_path, key);
                run(app, "gradient.presets.select", json!({"preset": key}));
            }
            BrowserAction::Drop(key, _) => {
                app.ui.presets_ui.set_selected("gradients", key);
                run(app, "gradient.presets.apply", json!({"preset": key}));
            }
            BrowserAction::ActivateItem { group, group_path, key } => {
                app.ui.presets_ui.set_selected_item("gradients", group, group_path, key);
                run(app, "gradient.presets.apply", json!({"preset": key}));
            }
            BrowserAction::New => {
                let mut state = std::mem::take(&mut app.ui.presets_ui);
                new_gradient(app, &mut state);
                app.ui.presets_ui = state;
            }
            BrowserAction::AppendDefault => {
                run(app, "gradient.presets.reset", json!({"append": true}));
            }
            BrowserAction::Import => crate::preset_files_ui::import_gradients_dialog(app),
            BrowserAction::ExportSelected(names) => export_selected_gradients_dialog(app, &names),
            _ => {
                generic(app, "gradients", "gradient.presets", &event, json!({"name": "Custom"}));
            }
        }
    }
}

pub(crate) fn export_selected_gradients_dialog(app: &mut PhotocraftApp, names: &[String]) {
    use photocraft_psd::grd::{GrdColor, GrdGradient, GrdStop};
    let gradients: Vec<_> = app
        .session
        .presets
        .gradients
        .iter()
        .flat_map(|group| group.items.iter())
        .filter(|gradient| names.contains(&gradient.name))
        .map(|gradient| GrdGradient {
            name: gradient.name.clone(),
            noise: false,
            stops: gradient
                .stops
                .iter()
                .map(|(location, color)| GrdStop {
                    location: *location,
                    midpoint: 0.5,
                    color: match color {
                        photocraft_engine::presets::gradients::StopColor::Foreground => GrdColor::Foreground,
                        photocraft_engine::presets::gradients::StopColor::Background => GrdColor::Background,
                        photocraft_engine::presets::gradients::StopColor::Rgb(rgb) => GrdColor::Rgb(*rgb),
                    },
                })
                .collect(),
            opacity: gradient.opacity.iter().map(|(location, opacity)| (*location, *opacity, 0.5)).collect(),
        })
        .collect();
    if gradients.is_empty() {
        return;
    }
    let bytes = photocraft_psd::grd::write(&gradients);
    let count = gradients.len();
    let result = app.pick_save("Gradients.grd", move |app, path| {
        let write = app.services.write.as_mut().ok_or("no writer configured")?;
        write(&path, &bytes)?;
        app.ui.status_error = false;
        app.ui.status = crate::i18n::fmt(
            &crate::i18n::trn(crate::i18n::current(), count as u64, "Exported {n} gradient to {file}", "Exported {n} gradients to {file}"),
            &[("n", &count.to_string()), ("file", &crate::file_open::display_name(&path))],
        );
        Ok(json!({"path": path, "count": count}))
    });
    if let Err(error) = result
        && error != crate::file_dialog::CANCELLED
    {
        app.ui.status = error;
        app.ui.status_error = true;
    }
}

/// Create a gradient through the same domain-aware path from the footer or the shared panel menu.
pub(crate) fn new_gradient(app: &mut PhotocraftApp, state: &mut crate::pickers::state::PresetUi) {
    use crate::pickers::browser::selection::SelectionTarget;
    let anchor = state.browser_selection_snapshot.get("gradients").and_then(|selection| selection.targets.last().cloned()).or_else(|| {
        if let Some(key) = state.selected.get("gradients") {
            let group = state
                .selected_group
                .get("gradients")
                .cloned()
                .or_else(|| app.session.presets.gradients.iter().find(|g| g.items.iter().any(|item| &item.name == key)).map(|g| g.name.clone()))?;
            Some(SelectionTarget::Item { group, path: Vec::new(), key: key.clone() })
        } else {
            state.selected_group.get("gradients").cloned().map(SelectionTarget::Group)
        }
    });
    let root_anchor = anchor.clone();
    let (destination_group, destination_folder, insertion_index) = match anchor {
        Some(SelectionTarget::Group(group)) => {
            let len = app.session.presets.gradients.iter().find(|g| g.name == group).map_or(0, |g| g.items.len());
            (group, Vec::new(), len)
        }
        Some(SelectionTarget::Folder { group, path }) => {
            let len = app.session.presets.gradients.iter().find(|g| g.name == group).map_or(0, |g| g.items.len());
            (group, path, len)
        }
        Some(SelectionTarget::Item { group, key, .. }) => {
            let index = app
                .session
                .presets
                .gradients
                .iter()
                .find(|g| g.name == group)
                .and_then(|g| g.items.iter().position(|item| item.name == key))
                .map_or(0, |i| i + 1);
            (group, Vec::new(), index)
        }
        None => {
            (ROOT_ITEMS_GROUP.to_owned(), Vec::new(), app.session.presets.gradients.iter().find(|g| g.name == ROOT_ITEMS_GROUP).map_or(0, |g| g.items.len()))
        }
    };
    let root_level = destination_group == ROOT_ITEMS_GROUP;
    if let Some(result) = run(app, "gradient.presets.new", json!({"name": "Custom", "group": destination_group})) {
        let name = result.get("name").and_then(serde_json::Value::as_str).unwrap_or_default().to_owned();
        if !name.is_empty() {
            let len = app.session.presets.gradients.iter().find(|g| g.name == destination_group).map_or(0, |g| g.items.len());
            if insertion_index < len.saturating_sub(1) {
                let _ = run(
                    app,
                    "gradient.presets.edit",
                    json!({"action":"move", "preset":name, "group":destination_group, "to":destination_group, "index":insertion_index}),
                );
            }
            state.clear_browser_selection("gradients");
            state.set_selected_item("gradients", &destination_group, &[], &name);
            let group_items = app
                .session
                .presets
                .gradients
                .iter()
                .find(|group| group.name == destination_group)
                .map_or_else(Vec::new, |group| group.items.iter().map(|item| item.name.clone()).collect());
            if root_level {
                insert_root_gradient_after(state, root_anchor.as_ref(), &group_items, &name);
            } else {
                insert_group_gradient_after(state, &destination_group, root_anchor.as_ref(), &group_items, &name);
            }
            expand_creation_destination(state, "gradients", &destination_group, &destination_folder);
            state.item_renaming = Some(ItemRenameState {
                panel: "gradients".into(),
                group: destination_group,
                group_path: Vec::new(),
                key: name.clone(),
                text: name,
                focused: false,
            });
        }
    }
}

/// Create a named preset after the panel's New Gradient Preset dialog is accepted.
fn create_named_gradient(app: &mut PhotocraftApp, state: &mut crate::pickers::state::PresetUi, name: &str) {
    use crate::pickers::browser::selection::SelectionTarget;
    let anchor = state.browser_selection_snapshot.get("gradients").and_then(|selection| selection.targets.last().cloned()).or_else(|| {
        if let Some(key) = state.selected.get("gradients") {
            let group = state
                .selected_group
                .get("gradients")
                .cloned()
                .or_else(|| app.session.presets.gradients.iter().find(|g| g.items.iter().any(|item| &item.name == key)).map(|g| g.name.clone()))?;
            Some(SelectionTarget::Item { group, path: Vec::new(), key: key.clone() })
        } else {
            state.selected_group.get("gradients").cloned().map(SelectionTarget::Group)
        }
    });
    let root_anchor = anchor.clone();
    let (destination_group, destination_folder, insertion_index) = match anchor {
        Some(SelectionTarget::Group(group)) => {
            (group.clone(), Vec::new(), app.session.presets.gradients.iter().find(|g| g.name.as_str() == group.as_str()).map_or(0, |g| g.items.len()))
        }
        Some(SelectionTarget::Folder { group, path }) => {
            (group.clone(), path, app.session.presets.gradients.iter().find(|g| g.name.as_str() == group.as_str()).map_or(0, |g| g.items.len()))
        }
        Some(SelectionTarget::Item { group, key, .. }) => {
            let index = app
                .session
                .presets
                .gradients
                .iter()
                .find(|g| g.name.as_str() == group.as_str())
                .and_then(|g| g.items.iter().position(|item| item.name == *key))
                .map_or(0, |i| i + 1);
            (group.clone(), Vec::new(), index)
        }
        None => {
            (ROOT_ITEMS_GROUP.to_owned(), Vec::new(), app.session.presets.gradients.iter().find(|g| g.name == ROOT_ITEMS_GROUP).map_or(0, |g| g.items.len()))
        }
    };
    let root_level = destination_group == ROOT_ITEMS_GROUP;
    if let Some(result) = run(app, "gradient.presets.new", json!({"name": name, "group": destination_group})) {
        let created_name = result.get("name").and_then(serde_json::Value::as_str).unwrap_or_default().to_owned();
        if !created_name.is_empty() {
            if insertion_index + 1 < app.session.presets.gradients.iter().find(|g| g.name == destination_group).map_or(0, |g| g.items.len()) {
                let _ = run(
                    app,
                    "gradient.presets.edit",
                    json!({"action":"move", "preset":created_name, "group":destination_group, "to":destination_group, "index":insertion_index}),
                );
            }
            state.clear_browser_selection("gradients");
            state.set_selected_item("gradients", &destination_group, &[], &created_name);
            let group_items = app
                .session
                .presets
                .gradients
                .iter()
                .find(|group| group.name == destination_group)
                .map_or_else(Vec::new, |group| group.items.iter().map(|item| item.name.clone()).collect());
            if root_level {
                insert_root_gradient_after(state, root_anchor.as_ref(), &group_items, &created_name);
            } else {
                insert_group_gradient_after(state, &destination_group, root_anchor.as_ref(), &group_items, &created_name);
            }
            expand_creation_destination(state, "gradients", &destination_group, &destination_folder);
        }
    }
}

/// Contribute the Gradients-specific actions to the shared docking header menu.
pub(crate) fn panel_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui, state: &crate::pickers::state::PresetUi) {
    if crate::menu_keys::menu_item_button(ui, tl!("Append Default Gradients…"), true).clicked() {
        run(app, "gradient.presets.reset", json!({"append": true}));
        ui.close();
    }
    if crate::menu_keys::menu_item_button(ui, tl!("Import Gradients…"), true).clicked() {
        crate::preset_files_ui::import_gradients_dialog(app);
        ui.close();
    }
    let selection = state.browser_selection_snapshot.get("gradients").cloned().unwrap_or_default();
    let menu = crate::pickers::browser::context_menu::SelectionMenuModel::new("Gradient", &selection);
    let selected_names = menu.export_items;
    if crate::menu_keys::menu_item_button(ui, tl!("Export Selected Gradients…"), !selected_names.is_empty()).clicked() {
        export_selected_gradients_dialog(app, &selected_names);
        ui.close();
    }
}

#[cfg(test)]
mod tests {
    use egui::{Event, Modifiers, PointerButton, vec2};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::{ROOT_ITEMS_GROUP, gradients_panel};
    use crate::PhotocraftApp;
    use crate::pickers::browser::asset_browser::last_group_headers;
    use crate::pickers::browser::browser_actions::expand_creation_destination;
    use crate::pickers::state::HierarchyOrderEntry;

    fn expand_gradient_groups(app: &mut PhotocraftApp) {
        app.ui.presets_ui.expanded_groups.extend(
            app.session
                .presets
                .gradients
                .iter()
                .filter(|group| group.name != ROOT_ITEMS_GROUP)
                .map(|group| format!("gradients/{}", crate::pickers::state::escape_hierarchy_key_part(&group.name))),
        );
        app.ui.presets_ui.group_expansion_version = 1;
    }

    #[test]
    fn expanding_a_nested_gradient_destination_changes_only_its_ancestor_path() {
        let mut state = crate::pickers::state::PresetUi::default();
        state.set_hierarchy_order(
            "gradients",
            "",
            vec![HierarchyOrderEntry::Group("Parent".into()), HierarchyOrderEntry::Group("Sibling".into()), HierarchyOrderEntry::Group("Other".into())],
        );
        state.set_hierarchy_order("gradients", "Parent", vec![HierarchyOrderEntry::Group("Child".into())]);
        state.expanded_groups.insert("gradients/Sibling".into());
        state.toggled.insert("gradients/Child/Folder%2FName".into());

        expand_creation_destination(&mut state, "gradients", "Child", &["Folder/Name".into()]);

        assert!(state.expanded_groups.contains("gradients/Parent"));
        assert!(state.expanded_groups.contains("gradients/Child"));
        assert!(state.expanded_groups.contains("gradients/Sibling"), "an unrelated open group remains open");
        assert!(!state.expanded_groups.contains("gradients/Other"), "an unrelated closed group remains closed");
        assert!(!state.toggled.contains("gradients/Child/Folder%2FName"), "the destination folder is expanded too");
    }

    #[test]
    fn creating_a_root_gradient_inserts_its_hierarchy_token_after_the_selected_card() {
        let mut state = crate::pickers::state::PresetUi::default();
        state.set_hierarchy_order(
            "gradients",
            "",
            vec![
                HierarchyOrderEntry::RootItem { group: ROOT_ITEMS_GROUP.into(), key: "Root A".into() },
                HierarchyOrderEntry::Group("Basics".into()),
                HierarchyOrderEntry::RootItem { group: ROOT_ITEMS_GROUP.into(), key: "Root B".into() },
            ],
        );
        let anchor = crate::pickers::browser::selection::SelectionTarget::Item { group: ROOT_ITEMS_GROUP.into(), path: Vec::new(), key: "Root B".into() };

        super::insert_root_gradient_after(&mut state, Some(&anchor), &["Root A".into(), "Root B".into()], "Created");

        assert_eq!(
            state.hierarchy_order["gradients"][""],
            [
                HierarchyOrderEntry::RootItem { group: ROOT_ITEMS_GROUP.into(), key: "Root A".into() },
                HierarchyOrderEntry::Group("Basics".into()),
                HierarchyOrderEntry::RootItem { group: ROOT_ITEMS_GROUP.into(), key: "Root B".into() },
                HierarchyOrderEntry::RootItem { group: ROOT_ITEMS_GROUP.into(), key: "Created".into() },
            ]
        );
    }

    #[test]
    fn creating_a_root_gradient_repairs_missing_root_tokens_before_inserting_after_the_anchor() {
        let mut state = crate::pickers::state::PresetUi::default();
        state.set_hierarchy_order("gradients", "", vec![HierarchyOrderEntry::Group("Basics".into())]);
        let anchor = crate::pickers::browser::selection::SelectionTarget::Item { group: ROOT_ITEMS_GROUP.into(), path: Vec::new(), key: "Root B".into() };

        super::insert_root_gradient_after(&mut state, Some(&anchor), &["Root A".into(), "Root B".into()], "Created");

        assert_eq!(
            state.hierarchy_order["gradients"][""],
            [
                HierarchyOrderEntry::RootItem { group: ROOT_ITEMS_GROUP.into(), key: "Root A".into() },
                HierarchyOrderEntry::RootItem { group: ROOT_ITEMS_GROUP.into(), key: "Root B".into() },
                HierarchyOrderEntry::RootItem { group: ROOT_ITEMS_GROUP.into(), key: "Created".into() },
                HierarchyOrderEntry::Group("Basics".into()),
            ]
        );
    }

    #[test]
    fn export_selection_includes_cards_from_selected_groups_and_mixed_selections() {
        let selection = crate::pickers::state::BrowserSelectionSnapshot {
            contains_card: true,
            contains_group: true,
            cards: vec![("Basics".into(), "Blue 01".into()), ("Blues".into(), "Blue 02".into())],
            groups: vec!["Basics".into()],
            ..Default::default()
        };
        let menu = crate::pickers::browser::context_menu::SelectionMenuModel::new("Gradient", &selection);
        assert_eq!(menu.export_items, ["Blue 01", "Blue 02"]);
    }

    #[test]
    fn creating_a_gradient_with_no_groups_places_the_card_at_root_without_a_visible_backing_group() {
        let mut session = photocraft_engine::Session::new();
        for group in session.presets.gradients.clone() {
            session.execute("gradient.presets.edit", serde_json::json!({"action": "deleteGroup", "group": group.name})).unwrap();
        }
        assert!(session.presets.gradients.is_empty());
        let app = PhotocraftApp::new(session, crate::Services::default());
        let mut h = Harness::builder().with_size(vec2(360.0, 900.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().fonts(|fonts| fonts.definitions().families.contains_key(&egui::FontFamily::Name("semibold".into()))) {
                    gradients_panel(app, ui);
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
        h.run_steps(3);

        let create = h.get_by_label("Create new gradient").rect().center();
        h.event(Event::PointerMoved(create));
        h.event(Event::PointerButton { pos: create, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(1);
        h.event(Event::PointerButton { pos: create, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(4);

        assert!(h.query_by_label("Name").is_some(), "creating a gradient from the footer must open the name dialog");
        h.get_by_label("OK").click();
        h.run_steps(4);

        let library = &h.state().session.presets.gradients;
        assert_eq!(library.len(), 1);
        assert_eq!(library[0].name, ROOT_ITEMS_GROUP);
        assert_eq!(library[0].items.len(), 1);
        let root = &h.state().ui.presets_ui.hierarchy_order["gradients"][""];
        assert_eq!(root, &[HierarchyOrderEntry::RootItem { group: ROOT_ITEMS_GROUP.into(), key: library[0].items[0].name.clone() }]);
        assert!(last_group_headers(&h.ctx, "gradients").is_empty(), "the domain backing group must not render as a folder");
        let card = h.get_by_label("Custom").rect();
        let image = h.render().expect("render the gradient asset browser");
        let ppp = h.ctx.pixels_per_point();
        let sample = |x: f32| image.get_pixel((x * ppp).floor() as u32, (card.center().y * ppp).floor() as u32).0;
        let left = sample(card.left() + card.width() * 0.25);
        let right = sample(card.left() + card.width() * 0.75);
        assert_ne!(left[..3], right[..3], "the gradient card must paint visibly different endpoint colours; card={card:?}");

        // The UI mirror remains serde-safe, while durable root membership is stored in the
        // gradient library's reserved owner group rather than depending on this transient state.
        let restored_ui: crate::state::UiState = serde_json::from_value(serde_json::to_value(&h.state().ui).unwrap()).unwrap();
        h.state_mut().ui = restored_ui;
        h.run_steps(3);
        assert!(last_group_headers(&h.ctx, "gradients").is_empty(), "the backing folder must remain hidden after restoring UiState");
        assert!(h.query_all_by_label("Custom").next().is_some(), "the root card must remain visible after restoring UiState");
        h.state_mut().ui.presets_ui = Default::default();
        h.run_steps(3);
        assert!(last_group_headers(&h.ctx, "gradients").is_empty(), "root placement must come from the saved gradient library, not transient UI state");
        assert!(h.query_all_by_label("Custom").next().is_some(), "the card must stay at root even when picker layout state is reset");
    }

    #[test]
    fn creating_a_gradient_after_a_multi_selection_uses_the_last_selected_card_not_a_group() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        for group in app.session.presets.gradients.clone() {
            app.run("gradient.presets.edit", serde_json::json!({"action":"deleteGroup", "group":group.name})).unwrap();
        }
        app.run("gradient.presets.edit", serde_json::json!({"action":"newGroup", "name":"Basics"})).unwrap();
        for name in ["First", "Second", "Third"] {
            app.run("gradient.presets.new", serde_json::json!({"group":"Basics", "name":name})).unwrap();
        }
        let mut state = crate::pickers::state::PresetUi::default();
        state.set_selected_item("gradients", "Basics", &[], "First");
        state.select_browser_node("gradients", crate::pickers::browser::path_identity_model::PickerId::from("old-first"), false);
        state.select_browser_node("gradients", crate::pickers::browser::path_identity_model::PickerId::from("old-second"), true);
        state.browser_selection_snapshot.insert(
            "gradients".into(),
            crate::pickers::browser::selection::BrowserSelection {
                targets: vec![
                    crate::pickers::browser::selection::SelectionTarget::Item { group: "Basics".into(), path: Vec::new(), key: "First".into() },
                    crate::pickers::browser::selection::SelectionTarget::Item { group: "Basics".into(), path: Vec::new(), key: "Second".into() },
                ],
                ..Default::default()
            },
        );

        super::new_gradient(&mut app, &mut state);

        let names =
            app.session.presets.gradients.iter().find(|group| group.name == "Basics").unwrap().items.iter().map(|item| item.name.as_str()).collect::<Vec<_>>();
        assert_eq!(names, ["First", "Second", "Custom", "Third"]);
        assert_eq!(state.selected.get("gradients").map(String::as_str), Some("Custom"));
        assert_eq!(state.selected_group.get("gradients").map(String::as_str), Some("Basics"));
        assert!(state.selected_group_path.get("gradients").is_some_and(Vec::is_empty));
        assert!(state.browser_selection.get("gradients").is_none_or(|selected| selected.is_empty()));
        assert!(!state.browser_selection_snapshot.contains_key("gradients"));
    }

    #[test]
    fn creating_a_gradient_after_a_single_selected_card_inserts_immediately_after_it() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        for group in app.session.presets.gradients.clone() {
            app.run("gradient.presets.edit", serde_json::json!({"action":"deleteGroup", "group":group.name})).unwrap();
        }
        app.run("gradient.presets.edit", serde_json::json!({"action":"newGroup", "name":"Basics"})).unwrap();
        for name in ["First", "Second", "Third"] {
            app.run("gradient.presets.new", serde_json::json!({"group":"Basics", "name":name})).unwrap();
        }
        let mut state = crate::pickers::state::PresetUi::default();
        state.set_selected_item("gradients", "Basics", &[], "First");

        super::new_gradient(&mut app, &mut state);

        let names: Vec<_> =
            app.session.presets.gradients.iter().find(|group| group.name == "Basics").unwrap().items.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(names, ["First", "Custom", "Second", "Third"]);
        assert_eq!(
            state.hierarchy_order["gradients"]["Basics"],
            [
                HierarchyOrderEntry::Item("First".into()),
                HierarchyOrderEntry::Item("Custom".into()),
                HierarchyOrderEntry::Item("Second".into()),
                HierarchyOrderEntry::Item("Third".into()),
            ]
        );
    }

    #[test]
    fn creating_a_root_gradient_after_a_selected_card_preserves_the_mixed_root_sequence() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        for group in app.session.presets.gradients.clone() {
            app.run("gradient.presets.edit", serde_json::json!({"action":"deleteGroup", "group":group.name})).unwrap();
        }
        for name in ["First", "Second", "Third"] {
            app.run("gradient.presets.new", serde_json::json!({"group":ROOT_ITEMS_GROUP, "name":name})).unwrap();
        }
        app.run("gradient.presets.edit", serde_json::json!({"action":"newGroup", "name":"Basics"})).unwrap();
        let mut state = crate::pickers::state::PresetUi::default();
        state.set_hierarchy_order("gradients", "", vec![HierarchyOrderEntry::Group("Basics".into())]);
        state.set_selected_item("gradients", ROOT_ITEMS_GROUP, &[], "First");

        super::new_gradient(&mut app, &mut state);

        let names: Vec<_> =
            app.session.presets.gradients.iter().find(|group| group.name == ROOT_ITEMS_GROUP).unwrap().items.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(names, ["First", "Custom", "Second", "Third"]);
        assert_eq!(
            state.hierarchy_order["gradients"][""],
            [
                HierarchyOrderEntry::RootItem { group: ROOT_ITEMS_GROUP.into(), key: "First".into() },
                HierarchyOrderEntry::RootItem { group: ROOT_ITEMS_GROUP.into(), key: "Custom".into() },
                HierarchyOrderEntry::RootItem { group: ROOT_ITEMS_GROUP.into(), key: "Second".into() },
                HierarchyOrderEntry::RootItem { group: ROOT_ITEMS_GROUP.into(), key: "Third".into() },
                HierarchyOrderEntry::Group("Basics".into()),
            ]
        );
    }

    #[test]
    fn creating_a_named_gradient_replaces_the_previous_selection() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        for group in app.session.presets.gradients.clone() {
            app.run("gradient.presets.edit", serde_json::json!({"action":"deleteGroup", "group":group.name})).unwrap();
        }
        app.run("gradient.presets.edit", serde_json::json!({"action":"newGroup", "name":"Basics"})).unwrap();
        for name in ["First", "Second"] {
            app.run("gradient.presets.new", serde_json::json!({"group":"Basics", "name":name})).unwrap();
        }
        let mut state = crate::pickers::state::PresetUi::default();
        state.set_selected_item("gradients", "Basics", &[], "First");
        state.select_browser_node("gradients", crate::pickers::browser::path_identity_model::PickerId::from("old-first"), false);
        state.select_browser_node("gradients", crate::pickers::browser::path_identity_model::PickerId::from("old-second"), true);
        state.browser_selection_snapshot.insert(
            "gradients".into(),
            crate::pickers::browser::selection::BrowserSelection {
                targets: vec![
                    crate::pickers::browser::selection::SelectionTarget::Item { group: "Basics".into(), path: Vec::new(), key: "First".into() },
                    crate::pickers::browser::selection::SelectionTarget::Item { group: "Basics".into(), path: Vec::new(), key: "Second".into() },
                ],
                ..Default::default()
            },
        );

        super::create_named_gradient(&mut app, &mut state, "Named");

        assert_eq!(state.selected.get("gradients").map(String::as_str), Some("Named"));
        assert_eq!(state.selected_group.get("gradients").map(String::as_str), Some("Basics"));
        assert!(state.selected_group_path.get("gradients").is_some_and(Vec::is_empty));
        assert_eq!(
            state.hierarchy_order["gradients"]["Basics"],
            [HierarchyOrderEntry::Item("First".into()), HierarchyOrderEntry::Item("Second".into()), HierarchyOrderEntry::Item("Named".into()),]
        );
        assert!(state.browser_selection.get("gradients").is_none_or(|selected| selected.is_empty()));
        assert!(!state.browser_selection_snapshot.contains_key("gradients"));
    }

    #[test]
    fn user_group_named_presets_remains_visible() {
        let mut session = photocraft_engine::Session::new();
        session.execute("gradient.presets.edit", serde_json::json!({"action": "newGroup", "name": "Presets"})).unwrap();
        session.execute("gradient.presets.new", serde_json::json!({"name": "User Gradient", "group": "Presets"})).unwrap();
        let app = PhotocraftApp::new(session, crate::Services::default());
        let mut h = Harness::builder().with_size(vec2(360.0, 900.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().fonts(|fonts| fonts.definitions().families.contains_key(&egui::FontFamily::Name("semibold".into()))) {
                    gradients_panel(app, ui);
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
        h.run_steps(3);

        let header = last_group_headers(&h.ctx, "gradients").into_iter().find(|(name, _)| name == "Presets").expect("the Presets group header is visible").1;
        let disclosure = egui::pos2(header.left() + 9.0, header.center().y);
        h.event(Event::PointerMoved(disclosure));
        h.event(Event::PointerButton { pos: disclosure, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(1);
        h.event(Event::PointerButton { pos: disclosure, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(3);
        let card = h.query_all_by_label("User Gradient").next().expect("the preset card is accessible").rect();
        let image = h.render().expect("render the grouped gradient preset");
        let ppp = h.ctx.pixels_per_point();
        let sample = |x: f32| image.get_pixel((x * ppp).floor() as u32, (card.center().y * ppp).floor() as u32).0;
        let left = sample(card.left() + card.width() * 0.25);
        let right = sample(card.left() + card.width() * 0.75);
        assert_ne!(left[..3], right[..3], "the grouped gradient card must actually paint its gradient");
    }

    #[test]
    fn confirming_the_new_gradient_preset_dialog_creates_the_named_preset() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut h = Harness::builder().with_size(vec2(360.0, 900.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().fonts(|fonts| fonts.definitions().families.contains_key(&egui::FontFamily::Name("semibold".into()))) {
                    gradients_panel(app, ui);
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
        crate::pickers::browser::popup_dialogs::request_new_gradient_preset(&h.ctx);
        h.run_steps(3);

        assert!(h.query_by_label("Name").is_some(), "the real gradient panel must display the name dialog");
        h.get_by_label("OK").click();
        h.run_steps(4);

        assert!(
            h.state().session.presets.gradients.iter().any(|group| group.items.iter().any(|item| item.name == "Custom")),
            "accepting the dialog creates the named gradient preset"
        );
        assert_eq!(h.state().ui.presets_ui.selected.get("gradients").map(String::as_str), Some("Custom"));
        assert!(h.state().ui.presets_ui.browser_selection.get("gradients").is_none_or(|selected| selected.is_empty()));
    }

    #[test]
    fn confirming_a_gradient_in_a_collapsed_nonfirst_group_reveals_it_without_opening_others() {
        let mut session = photocraft_engine::Session::new();
        for group in session.presets.gradients.clone() {
            session.execute("gradient.presets.edit", serde_json::json!({"action": "deleteGroup", "group": group.name})).unwrap();
        }
        session.execute("gradient.presets.edit", serde_json::json!({"action": "newGroup", "name": "Presets"})).unwrap();
        session.execute("gradient.presets.new", serde_json::json!({"group": "Presets", "name": "First"})).unwrap();
        session.execute("gradient.presets.edit", serde_json::json!({"action": "newGroup", "name": "Destination"})).unwrap();
        session.execute("gradient.presets.new", serde_json::json!({"group": "Destination", "name": "Existing"})).unwrap();
        session.execute("gradient.presets.edit", serde_json::json!({"action": "newGroup", "name": "Unrelated"})).unwrap();
        session.execute("gradient.presets.new", serde_json::json!({"group": "Unrelated", "name": "Other"})).unwrap();
        let mut app = PhotocraftApp::new(session, crate::Services::default());
        app.ui.presets_ui.selected_group.insert("gradients".into(), "Destination".into());
        // Open the destination before the UI click below so the test can collapse it explicitly.
        app.ui.presets_ui.expanded_groups.insert("gradients/Destination".into());
        let mut h = Harness::builder().with_size(vec2(420.0, 900.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().fonts(|fonts| fonts.definitions().families.contains_key(&egui::FontFamily::Name("semibold".into()))) {
                    gradients_panel(app, ui);
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
        h.run_steps(3);

        let headers = last_group_headers(&h.ctx, "gradients");
        let header = headers.iter().find(|(name, _)| name == "Destination").expect("destination group is rendered").1;
        let disclosure = egui::pos2(header.left() + 9.0, header.center().y);
        h.event(Event::PointerMoved(disclosure));
        h.event(Event::PointerButton { pos: disclosure, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(1);
        h.event(Event::PointerButton { pos: disclosure, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(3);
        assert!(h.query_all_by_label("Existing").next().is_none(), "the destination group is genuinely collapsed in the rendered browser");
        assert!(h.query_all_by_label("Other").next().is_none(), "the unrelated non-first group starts collapsed");

        let create = h.get_by_label("Create new gradient").rect().center();
        h.event(Event::PointerMoved(create));
        h.event(Event::PointerButton { pos: create, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(1);
        h.event(Event::PointerButton { pos: create, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(3);
        assert!(h.query_by_label("Name").is_some(), "the real footer opens the new-gradient dialog");
        h.get_by_label("OK").click();
        h.run_steps(4);

        assert!(h.query_all_by_label("Custom").next().is_some(), "the newly appended gradient card is visible without manually expanding the group");
        assert!(h.query_all_by_label("Existing").next().is_some(), "the pre-existing card is visible after the group expands");
        assert!(h.query_all_by_label("Other").next().is_none(), "expanding the destination must not expand unrelated groups");
    }

    #[test]
    fn dropping_a_gradient_before_the_first_group_moves_it_into_the_persistent_root_owner() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        expand_gradient_groups(&mut app);
        let mut h = Harness::builder().with_size(vec2(520.0, 900.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().fonts(|fonts| fonts.definitions().families.contains_key(&egui::FontFamily::Name("semibold".into()))) {
                    gradients_panel(app, ui);
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
        h.run_steps(3);

        let headers = last_group_headers(&h.ctx, "gradients");
        assert!(headers.len() >= 2);
        let (source_group, _) = headers.first().unwrap();
        let source_key =
            h.state().session.presets.gradients.iter().find(|group| &group.name == source_group).and_then(|group| group.items.first()).unwrap().name.clone();
        let source = h.get_by_label(&source_key).rect().center();
        let first_header = headers.first().unwrap().1;
        let target = egui::pos2(first_header.center().x, first_header.top() - 5.0);

        h.event(Event::PointerMoved(source));
        h.run_steps(1);
        h.event(Event::PointerButton { pos: source, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(1);
        for step in 1..=8 {
            h.event(Event::PointerMoved(source + (target - source) * (step as f32 / 8.0)));
            h.run_steps(1);
        }
        h.event(Event::PointerButton { pos: target, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(3);

        let root_owner =
            h.state().session.presets.gradients.iter().find(|group| group.name == ROOT_ITEMS_GROUP).expect("the domain root owner is created on demand");
        assert_eq!(root_owner.items.first().map(|item| item.name.as_str()), Some(source_key.as_str()));
        assert!(last_group_headers(&h.ctx, "gradients").iter().all(|(name, _)| name != ROOT_ITEMS_GROUP));

        // Clearing picker-only state simulates an application restart: domain membership still
        // reconstructs the root item and the backing group remains invisible.
        h.state_mut().ui.presets_ui = Default::default();
        h.run_steps(3);
        assert!(last_group_headers(&h.ctx, "gradients").iter().all(|(name, _)| name != ROOT_ITEMS_GROUP));
        assert!(h.query_all_by_label(&source_key).next().is_some());
    }

    #[test]
    fn dropping_a_gradient_between_groups_or_after_the_last_group_moves_it_to_root() {
        for drop_position in ["gap_upper", "gap_lower", "root_end"] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            expand_gradient_groups(&mut app);
            let mut h = Harness::builder().with_size(vec2(520.0, 900.0)).build_ui_state(
                |ui, app: &mut PhotocraftApp| {
                    if ui.ctx().fonts(|fonts| fonts.definitions().families.contains_key(&egui::FontFamily::Name("semibold".into()))) {
                        gradients_panel(app, ui);
                    }
                },
                app,
            );
            PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
            h.run_steps(3);

            let initial_headers = last_group_headers(&h.ctx, "gradients");
            assert!(initial_headers.len() >= 2);
            let (source_group, _) = initial_headers.first().unwrap();
            let source_key = h
                .state()
                .session
                .presets
                .gradients
                .iter()
                .find(|group| &group.name == source_group)
                .and_then(|group| {
                    group.items.iter().find(|item| {
                        !h.state()
                            .session
                            .presets
                            .gradients
                            .iter()
                            .find(|owner| owner.name == ROOT_ITEMS_GROUP)
                            .is_some_and(|owner| owner.items.iter().any(|root_item| root_item.name == item.name))
                    })
                })
                .unwrap()
                .name
                .clone();
            if h.query_all_by_label(&source_key).next().is_none() {
                let header = initial_headers.first().unwrap().1.center();
                h.event(Event::PointerMoved(header));
                h.event(Event::PointerButton { pos: header, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
                h.run_steps(1);
                h.event(Event::PointerButton { pos: header, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
                h.run_steps(2);
            }
            // Expanding a collapsed source group changes every later row's position. Always use
            // the freshly rendered headers for the drop coordinate.
            let headers = last_group_headers(&h.ctx, "gradients");
            let source = h.get_by_label(&source_key).rect().center();
            let target = match drop_position {
                "gap_upper" | "gap_lower" => {
                    let second_header = headers[1].1;
                    let offset = if drop_position == "gap_upper" { -5.0 } else { -1.0 };
                    egui::pos2(second_header.center().x, second_header.top() + offset)
                }
                "root_end" => {
                    let (last_group, last_header) = headers.last().unwrap();
                    let last_key = h
                        .state()
                        .session
                        .presets
                        .gradients
                        .iter()
                        .find(|group| &group.name == last_group)
                        .and_then(|group| group.items.last())
                        .unwrap()
                        .name
                        .clone();
                    if h.query_all_by_label(&last_key).next().is_none() {
                        let disclosure = egui::pos2(last_header.left() + 9.0, last_header.center().y);
                        h.event(Event::PointerMoved(disclosure));
                        h.event(Event::PointerButton { pos: disclosure, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
                        h.run_steps(1);
                        h.event(Event::PointerButton { pos: disclosure, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
                        h.run_steps(2);
                    }
                    let last_card = h.get_by_label(&last_key).rect();
                    // Drop well into the blank trailing area, not merely the narrow gap after the card.
                    egui::pos2(last_card.center().x, last_card.bottom() + 48.0)
                }
                _ => {
                    assert!(matches!(drop_position, "gap_upper" | "gap_lower" | "root_end"), "unexpected drop position: {drop_position}");
                    egui::Pos2::ZERO
                }
            };
            h.event(Event::PointerMoved(source));
            h.run_steps(1);
            h.event(Event::PointerButton { pos: source, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
            h.run_steps(1);
            for step in 1..=8 {
                h.event(Event::PointerMoved(source + (target - source) * (step as f32 / 8.0)));
                h.run_steps(1);
            }
            h.event(Event::PointerButton { pos: target, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
            h.run_steps(3);

            let root_owner = h
                .state()
                .session
                .presets
                .gradients
                .iter()
                .find(|group| group.name == ROOT_ITEMS_GROUP)
                .unwrap_or_else(|| panic!("root owner should be created for drop_position={drop_position}"));
            assert!(
                root_owner.items.iter().any(|item| item.name == source_key),
                "drop_position={drop_position}: expected {source_key:?} in root owner, library={:?}",
                h.state().session.presets.gradients
            );
            let root_order = &h.state().ui.presets_ui.hierarchy_order["gradients"][""];
            let moved = HierarchyOrderEntry::RootItem { group: ROOT_ITEMS_GROUP.into(), key: source_key.clone() };
            let moved_index = root_order.iter().position(|entry| entry == &moved).expect("the mixed root sequence retains the moved card");
            match drop_position {
                "gap_upper" | "gap_lower" => {
                    let first_group_index = root_order.iter().position(|entry| *entry == HierarchyOrderEntry::Group(headers[0].0.clone())).unwrap();
                    let second_group_index = root_order.iter().position(|entry| *entry == HierarchyOrderEntry::Group(headers[1].0.clone())).unwrap();
                    assert!(
                        first_group_index < moved_index && moved_index < second_group_index,
                        "drop_position={drop_position}: card must remain between the first two groups: {root_order:?}"
                    );
                }
                "root_end" => assert_eq!(root_order.last(), Some(&moved), "the card must remain at the root sequence's end: {root_order:?}"),
                _ => assert!(matches!(drop_position, "gap_upper" | "gap_lower" | "root_end"), "unexpected drop position: {drop_position}"),
            }
            assert!(last_group_headers(&h.ctx, "gradients").iter().all(|(name, _)| name != ROOT_ITEMS_GROUP));
        }
    }

    #[test]
    fn dropping_a_gradient_at_the_bottom_of_a_scrolled_browser_appends_to_root() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        expand_gradient_groups(&mut app);
        let mut h = Harness::builder().with_size(vec2(520.0, 420.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().fonts(|fonts| fonts.definitions().families.contains_key(&egui::FontFamily::Name("semibold".into()))) {
                    gradients_panel(app, ui);
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
        h.run_steps(3);

        let scroll_to_bottom = |h: &mut Harness<'static, PhotocraftApp>| {
            let viewport = crate::pickers::browser::asset_browser::last_browser_rect(&h.ctx, "gradients").unwrap();
            h.event(Event::PointerMoved(viewport.center()));
            h.event(Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, -2400.0),
                phase: egui::TouchPhase::Move,
                modifiers: Modifiers::NONE,
            });
            h.run_steps(3);
        };
        scroll_to_bottom(&mut h);

        let (last_group, last_header) = last_group_headers(&h.ctx, "gradients").last().cloned().unwrap();
        let last_key =
            h.state().session.presets.gradients.iter().find(|group| group.name == last_group).and_then(|group| group.items.last()).unwrap().name.clone();
        if h.query_all_by_label(&last_key).next().is_none() {
            let point = egui::pos2(last_header.left() + 9.0, last_header.center().y);
            h.event(Event::PointerMoved(point));
            h.event(Event::PointerButton { pos: point, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
            h.run_steps(1);
            h.event(Event::PointerButton { pos: point, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
            h.run_steps(2);
            scroll_to_bottom(&mut h);
        }
        let source = h.get_by_label(&last_key).rect().center();
        let viewport = crate::pickers::browser::asset_browser::last_browser_rect(&h.ctx, "gradients").unwrap();
        let target = egui::pos2(viewport.center().x, viewport.bottom() - 8.0);
        assert!(viewport.contains(target));
        h.event(Event::PointerMoved(source));
        h.run_steps(1);
        h.event(Event::PointerButton { pos: source, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(1);
        for step in 1..=8 {
            h.event(Event::PointerMoved(source + (target - source) * (step as f32 / 8.0)));
            h.run_steps(1);
        }
        h.event(Event::PointerButton { pos: target, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(3);

        let root_owner = h
            .state()
            .session
            .presets
            .gradients
            .iter()
            .find(|group| group.name == ROOT_ITEMS_GROUP)
            .expect("dropping at the root end creates the durable owner group");
        assert!(root_owner.items.iter().any(|item| item.name == last_key));
        let root_order = &h.state().ui.presets_ui.hierarchy_order["gradients"][""];
        assert_eq!(root_order.last(), Some(&HierarchyOrderEntry::RootItem { group: ROOT_ITEMS_GROUP.into(), key: last_key }));
    }

    #[test]
    fn gradients_group_drop_before_card_persists_the_unified_order() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        expand_gradient_groups(&mut app);
        assert!(app.session.presets.gradients.len() >= 2, "the fixture needs two gradient groups");
        let mut h = Harness::builder().with_size(vec2(360.0, 900.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().fonts(|fonts| fonts.definitions().families.contains_key(&egui::FontFamily::Name("semibold".into()))) {
                    gradients_panel(app, ui);
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
        h.run_steps(3);

        let headers = last_group_headers(&h.ctx, "gradients");
        assert!(headers.len() >= 2, "gradient group headers are visible");
        let (target_group, _) = &headers[0];
        let (moving_group, moving_header) = headers.last().unwrap();
        let target_item =
            h.state().session.presets.gradients.iter().find(|group| &group.name == target_group).and_then(|group| group.items.first()).unwrap().name.clone();
        let target_card = h.get_by_label(&target_item).rect();
        let target = egui::pos2(target_card.left() + 1.0, target_card.center().y);

        h.event(egui::Event::PointerMoved(moving_header.center()));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: moving_header.center(), button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(1);
        for step in 1..=6 {
            h.event(egui::Event::PointerMoved(moving_header.center() + (target - moving_header.center()) * (step as f32 / 6.0)));
            h.run_steps(1);
        }
        h.event(egui::Event::PointerButton { pos: target, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(3);

        let state = &h.state().ui.presets_ui;
        assert_eq!(state.parent_of("gradients", moving_group), Some(target_group.as_str()));
        assert_eq!(state.hierarchy_order["gradients"][target_group].first(), Some(&HierarchyOrderEntry::Group(moving_group.clone())));
        assert_eq!(state.hierarchy_order["gradients"][target_group].get(1), Some(&HierarchyOrderEntry::Item(target_item)));
        let names: Vec<String> = headers.iter().map(|(name, _)| name.clone()).collect();
        assert_eq!(state.display_order("gradients", &names).len(), names.len());
    }
}
