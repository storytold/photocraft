//! Shared execution of browser actions and translation into domain commands.

use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::pickers::browser::asset_browser::BrowserAction;
pub(crate) fn run(app: &mut PhotocraftApp, id: &str, p: Value) -> Option<Value> {
    match app.run(id, p) {
        Ok(v) => Some(v),
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
            None
        }
    }
}

/// Shared handling of the generic events through `<prefix>.edit` / `.new`.
pub(crate) fn generic(app: &mut PhotocraftApp, panel: &str, prefix: &str, e: &BrowserAction, new_params: Value) -> bool {
    let mut state = std::mem::take(&mut app.ui.presets_ui);
    let handled = generic_with_state(app, &mut state, panel, prefix, e, new_params);
    app.ui.presets_ui = state;
    handled
}

/// Shared handling for callers that temporarily own picker state while composing a panel menu.
pub(crate) fn generic_with_state(
    app: &mut PhotocraftApp,
    state: &mut crate::pickers::state::PresetUi,
    panel: &str,
    prefix: &str,
    e: &BrowserAction,
    new_params: Value,
) -> bool {
    let edit = format!("{prefix}.edit");
    match e {
        BrowserAction::Rename(k, n) => {
            let (command, params) = if prefix == "swatches" {
                ("swatches.rename".to_owned(), json!({"swatch": k, "name": n}))
            } else {
                (edit.clone(), json!({"action": "rename", "preset": k, "name": n}))
            };
            if run(app, &command, params).is_some() {
                // Pattern item keys are stable library IDs, not display names. Keep the selected ID
                // when a pattern is renamed; other standard presets use their names as identity.
                if prefix != "pattern.presets" && state.selected.get(panel) == Some(k) {
                    if let Some(group) = state.selected_group.get(panel).cloned() {
                        state.set_selected_item(panel, &group, &[], n);
                    } else {
                        state.set_selected(panel, n);
                    }
                }
                if prefix != "pattern.presets" {
                    state.rename_hierarchy_item(panel, k, n);
                }
                state.clear_browser_selection(panel);
                true
            } else {
                false
            }
        }
        BrowserAction::RenameItem { group, group_path, key, text } => {
            let (command, params) = if prefix == "swatches" {
                ("swatches.rename".to_owned(), json!({"swatch": key, "name": text}))
            } else {
                (edit.clone(), json!({"action": "rename", "preset": key, "group": group, "name": text}))
            };
            if run(app, &command, params).is_some() {
                if prefix != "pattern.presets" && state.is_selected_item(panel, group, group_path, key) {
                    state.set_selected_item(panel, group, group_path, text);
                }
                // Pattern keys are stable library IDs rather than display names. Keep their
                // hierarchy identity unchanged; other adapters rename the exact folder occurrence.
                if prefix != "pattern.presets" {
                    state.rename_hierarchy_item_at_path(panel, group, group_path, key, text);
                }
                state.clear_browser_selection(panel);
                true
            } else {
                false
            }
        }
        BrowserAction::BeginInlineRenameItem(group, item) => {
            state.renaming = None;
            state.group_rename_queue.clear();
            state.item_rename_queue.clear();
            state.item_renaming = Some(crate::pickers::state::ItemRenameState {
                panel: panel.to_owned(),
                group: group.clone(),
                group_path: item.group_path.clone(),
                key: item.key.clone(),
                text: item.name.clone(),
                focused: false,
            });
            true
        }
        BrowserAction::CancelInlineRenameItem => {
            state.item_renaming = None;
            state.item_rename_queue.clear();
            true
        }
        BrowserAction::BeginInlineRenameGroup { group, group_path } => {
            state.renaming = None;
            state.group_renaming = None;
            state.group_rename_queue.clear();
            state.item_renaming = None;
            state.item_rename_queue.clear();
            state.inline_group_renaming = Some(crate::pickers::state::InlineGroupRenameState {
                panel: panel.to_owned(),
                group: group.clone(),
                group_path: group_path.clone(),
                text: group_path.last().cloned().unwrap_or_else(|| group.clone()),
                focused: false,
            });
            true
        }
        BrowserAction::CancelInlineRenameGroup => {
            state.inline_group_renaming = None;
            true
        }
        // The shared popup layer consumes this combined request and presents group renames
        // before card renames; it should not reach domain dispatch.
        BrowserAction::BeginRenameSelection { .. } => true,
        BrowserAction::DeleteItem { group, group_path, key } => {
            let (command, params) = if prefix == "swatches" {
                ("swatches.delete".to_owned(), json!({"swatch": key}))
            } else {
                (edit.clone(), json!({"action": "delete", "preset": key, "group": group}))
            };
            if run(app, &command, params).is_some() {
                if state.is_selected_item(panel, group, group_path, key) {
                    state.clear_selected(panel);
                }
                if prefix == "swatches" {
                    state.delete_hierarchy_item_in_group(panel, group, key);
                } else {
                    state.delete_hierarchy_item_at_path(panel, group, group_path, key);
                }
                state.clear_browser_selection(panel);
                true
            } else {
                false
            }
        }
        BrowserAction::Move(k, g) => {
            if prefix == "swatches" {
                run(app, "swatches.move", json!({"swatch": k, "to": g})).is_some()
            } else {
                run(app, &edit, json!({"action": "move", "preset": k, "to": g})).is_some()
            }
        }
        BrowserAction::RenameGroup(g, n) => {
            let (command, params) = if prefix == "swatches" {
                ("swatches.renameGroup".to_owned(), json!({"group": g, "name": n}))
            } else {
                (edit.clone(), json!({"action": "renameGroup", "group": g, "name": n}))
            };
            if run(app, &command, params).is_some() {
                state.rename_group(panel, g, n);
                state.clear_browser_selection(panel);
                true
            } else {
                false
            }
        }
        BrowserAction::RenameNestedGroup { group_path, text, .. } => {
            let Some(group) = group_path.last() else { return false };
            let group = group.clone();
            let (command, params) = if prefix == "swatches" {
                ("swatches.renameGroup".to_owned(), json!({"group": group, "name": text}))
            } else {
                (edit.clone(), json!({"action": "renameGroup", "group": group, "name": text}))
            };
            if run(app, &command, params).is_some() {
                state.rename_group(panel, &group, text);
                state.clear_browser_selection(panel);
                true
            } else {
                false
            }
        }
        BrowserAction::DeleteGroup(g) => {
            let deletion_order = state.group_deletion_order(panel, g);
            for group in &deletion_order {
                let (command, params) = if prefix == "swatches" {
                    ("swatches.deleteGroup".to_owned(), json!({"group": group}))
                } else {
                    (edit.clone(), json!({"action": "deleteGroup", "group": group}))
                };
                if run(app, &command, params).is_none() {
                    return false;
                }
                state.delete_group(panel, group);
            }
            if state.selected_group.get(panel).is_some_and(|selected| deletion_order.contains(selected)) {
                state.clear_selected(panel);
            }
            state.clear_browser_selection(panel);
            true
        }
        BrowserAction::DeleteNestedGroup { group_path, .. } => {
            let Some(group) = group_path.last() else { return false };
            let deletion_order = state.group_deletion_order(panel, group);
            for name in &deletion_order {
                let (command, params) = if prefix == "swatches" {
                    ("swatches.deleteGroup".to_owned(), json!({"group": name}))
                } else {
                    (edit.clone(), json!({"action": "deleteGroup", "group": name}))
                };
                if run(app, &command, params).is_none() {
                    return false;
                }
                state.delete_group(panel, name);
            }
            if state.selected_group.get(panel).is_some_and(|selected| deletion_order.contains(selected)) {
                state.clear_selected(panel);
            }
            state.clear_browser_selection(panel);
            true
        }
        BrowserAction::NewGroup => false,
        BrowserAction::CreateGroup(name) => {
            let (command, params) = if prefix == "swatches" {
                ("swatches.newGroup".to_owned(), json!({"name": name}))
            } else {
                (edit.clone(), json!({"action": "newGroup", "name": name}))
            };
            if let Some(r) = run(app, &command, params) {
                let g = r["group"].as_str().unwrap_or_default().to_string();
                state.renaming = Some((panel.into(), format!("group:{g}"), g));
                true
            } else {
                false
            }
        }
        BrowserAction::New => {
            let (command, params) = if prefix == "swatches" {
                let selected_group = state.selected_group.get(panel).cloned().or_else(|| {
                    state.selected.get(panel).and_then(|key| {
                        app.session.presets.swatches.iter().find(|group| group.items.iter().any(|item| &item.name == key)).map(|group| group.name.clone())
                    })
                });
                ("swatches.add".to_owned(), selected_group.map_or_else(|| json!({}), |group| json!({"group": group})))
            } else {
                (format!("{prefix}.new"), new_params)
            };
            if let Some(r) = run(app, &command, params) {
                let name = r.get("name").or(r.get("pattern")).and_then(Value::as_str).unwrap_or_default().to_string();
                state.set_selected(panel, &name);
                if prefix != "pattern.presets" {
                    state.renaming = Some((panel.into(), name.clone(), name));
                }
                true
            } else {
                false
            }
        }
        BrowserAction::SelectItem { .. }
        | BrowserAction::ActivateItem { .. }
        | BrowserAction::Drop(_, _)
        | BrowserAction::AppendDefault
        | BrowserAction::Import
        | BrowserAction::ExportSelected(_) => false,
    }
}
