//! Command translation shared by the standard preset-browser adapters.

use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::pickers::asset_browser::BrowserAction;
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
    let edit = format!("{prefix}.edit");
    match e {
        BrowserAction::Rename(k, n) => {
            if run(app, &edit, json!({"action": "rename", "preset": k, "name": n})).is_some() {
                // Pattern item keys are stable library IDs, not display names. Keep the selected ID
                // when a pattern is renamed; other standard presets use their names as identity.
                if prefix != "pattern.presets" && app.ui.presets_ui.selected.get(panel) == Some(k) {
                    if let Some(group) = app.ui.presets_ui.selected_group.get(panel).cloned() {
                        app.ui.presets_ui.set_selected_item(panel, &group, &[], n);
                    } else {
                        app.ui.presets_ui.set_selected(panel, n);
                    }
                }
                if prefix != "pattern.presets" {
                    app.ui.presets_ui.rename_hierarchy_item(panel, k, n);
                }
                true
            } else {
                false
            }
        }
        BrowserAction::RenameItem { group, group_path, key, text } => {
            if run(app, &edit, json!({"action": "rename", "preset": key, "group": group, "name": text})).is_some() {
                if prefix != "pattern.presets" && app.ui.presets_ui.is_selected_item(panel, group, group_path, key) {
                    app.ui.presets_ui.set_selected_item(panel, group, group_path, text);
                }
                // Pattern keys are stable library IDs rather than display names. Keep their
                // hierarchy identity unchanged; other adapters rename the exact folder occurrence.
                if prefix != "pattern.presets" {
                    app.ui.presets_ui.rename_hierarchy_item_at_path(panel, group, group_path, key, text);
                }
                true
            } else { false }
        }
        BrowserAction::Delete(k) => {
            if run(app, &edit, json!({"action": "delete", "preset": k})).is_some() {
                app.ui.presets_ui.clear_selected(panel);
                app.ui.presets_ui.delete_hierarchy_item(panel, k);
                true
            } else {
                false
            }
        }
        BrowserAction::DeleteItem { group, group_path, key } => {
            if run(app, &edit, json!({"action": "delete", "preset": key, "group": group})).is_some() {
                if app.ui.presets_ui.is_selected_item(panel, group, group_path, key) {
                    app.ui.presets_ui.clear_selected(panel);
                }
                app.ui.presets_ui.delete_hierarchy_item_at_path(panel, group, group_path, key);
                true
            } else { false }
        }
        BrowserAction::Move(k, g) => run(app, &edit, json!({"action": "move", "preset": k, "to": g})).is_some(),
        BrowserAction::MoveItem(k, g, index) => {
            // Patterns have no engine reorder command: their shared-browser sibling sequence is
            // the persisted view order and has already been updated by the drag resolver.
            if prefix == "pattern.presets" {
                return true;
            }
            let mut params = json!({"action": "move", "preset": k, "to": g});
            if let Some(index) = index {
                params["index"] = json!(index);
            }
            run(app, &edit, params).is_some()
        }
        BrowserAction::RenameGroup(g, n) => {
            if run(app, &edit, json!({"action": "renameGroup", "group": g, "name": n})).is_some() {
                app.ui.presets_ui.rename_group(panel, g, n);
                true
            } else {
                false
            }
        }
        BrowserAction::DeleteGroup(g) => {
            if run(app, &edit, json!({"action": "deleteGroup", "group": g})).is_some() {
                app.ui.presets_ui.delete_group(panel, g);
                true
            } else {
                false
            }
        }
        BrowserAction::NewGroup => {
            if let Some(r) = run(app, &edit, json!({"action": "newGroup", "name": "Group"})) {
                let g = r["group"].as_str().unwrap_or_default().to_string();
                app.ui.presets_ui.renaming = Some((panel.into(), format!("group:{g}"), g));
                true
            } else {
                false
            }
        }
        BrowserAction::New => {
            if let Some(r) = run(app, &format!("{prefix}.new"), new_params) {
                let name = r.get("name").or(r.get("pattern")).and_then(Value::as_str).unwrap_or_default().to_string();
                app.ui.presets_ui.set_selected(panel, &name);
                if prefix != "pattern.presets" {
                    app.ui.presets_ui.renaming = Some((panel.into(), name.clone(), name));
                }
                true
            } else {
                false
            }
        }
        BrowserAction::Select(_)
        | BrowserAction::SelectItem { .. }
        | BrowserAction::Activate(_)
        | BrowserAction::ActivateItem { .. }
        | BrowserAction::Drop(_, _)
        | BrowserAction::RenameNestedGroup { .. }
        | BrowserAction::DeleteNestedGroup { .. }
        | BrowserAction::MoveItemToGroup { .. }
        | BrowserAction::MoveGroup { .. }
        | BrowserAction::MoveInto { .. } => false,
    }
}
