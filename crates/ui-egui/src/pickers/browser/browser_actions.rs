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
                // The browser snapshot represents an explicit multi-selection. A single
                // selection made by clicking a card or by creating a new container lives in
                // the same active-selection fields used by the rest of the picker. Treat an
                // absent/empty snapshot as that active selection, not as "nothing selected".
                let selection = state
                    .browser_selection_snapshot
                    .get(panel)
                    .filter(|selection| !selection.targets.is_empty())
                    .cloned()
                    .unwrap_or_else(|| generic_active_selection(app, state, panel, prefix));
                if !g.is_empty() {
                    if let Err(error) = move_selection_into_new_group(app, state, panel, prefix, &g, &selection) {
                        app.ui.status = error;
                        app.ui.status_error = true;
                    }
                    expand_creation_destination(state, panel, &g, &[]);
                    state.clear_browser_selection(panel);
                    state.clear_selected(panel);
                    state.selected_group.insert(panel.to_owned(), g.clone());
                    state.selected_group_path.insert(panel.to_owned(), Vec::new());
                }
                state.renaming = Some((panel.into(), format!("group:{g}"), g));
                true
            } else {
                false
            }
        }
        BrowserAction::NewNamed(name) => {
            let mut params = new_params;
            params["name"] = json!(name);
            generic_with_state(app, state, panel, prefix, &BrowserAction::New, params)
        }
        BrowserAction::New => {
            // Selection order is authoritative: the final selected node determines the insertion
            // destination, whether it is a card, group, or folder.
            let destination = state
                .browser_selection_snapshot
                .get(panel)
                .filter(|selection| !selection.targets.is_empty())
                .and_then(|selection| selection.targets.last().cloned())
                .or_else(|| generic_active_selection(app, state, panel, prefix).targets.into_iter().last());
            let destination_item = match &destination {
                Some(crate::pickers::browser::selection::SelectionTarget::Item { group, key, path }) => Some((group.clone(), key.clone(), path.clone())),
                _ => None,
            };
            let root_destination = destination.is_none();
            let selected_root_item = destination_item.as_ref().filter(|(group, key, path)| {
                path.is_empty()
                    && state.hierarchy_order.get(panel).and_then(|orders| orders.get("")).is_some_and(|entries| {
                        entries.contains(&crate::pickers::state::HierarchyOrderEntry::RootItem { group: group.clone(), key: key.clone() })
                    })
            });
            let root_owner = root_destination.then(|| generic_root_item_group(app, prefix)).flatten();
            let (destination_group, destination_folder, insertion_index) = if let Some(owner) = root_owner.clone() {
                let len = generic_group_len(app, prefix, &owner);
                (Some(owner), Vec::new(), Some(len))
            } else {
                match destination {
                    Some(crate::pickers::browser::selection::SelectionTarget::Group(group)) => {
                        let len = generic_group_len(app, prefix, &group);
                        (Some(group), Vec::new(), Some(len))
                    }
                    Some(crate::pickers::browser::selection::SelectionTarget::Folder { group, path }) => {
                        let len = generic_group_len(app, prefix, &group);
                        (Some(group), path, Some(len))
                    }
                    Some(crate::pickers::browser::selection::SelectionTarget::Item { group, key, path }) => {
                        let index = generic_item_index(app, prefix, &group, &key).map(|index| index + 1);
                        (Some(group), path, index)
                    }
                    None => (None, Vec::new(), None),
                }
            };
            let (command, params) = if prefix == "swatches" {
                let empty = app.session.presets.swatches.iter().all(|group| group.items.is_empty());
                let mut params = if empty { json!({"name": "Swatch 1", "color": {"model": "rgb", "values": [255, 0, 0]}}) } else { json!({}) };
                if let Some(name) = new_params.get("name").and_then(Value::as_str) {
                    params["name"] = json!(name);
                }
                if let Some(group) = destination_group.clone().or_else(|| state.selected_group.get(panel).cloned()) {
                    params["group"] = json!(group);
                }
                if let Some(index) = insertion_index {
                    params["index"] = json!(index);
                }
                ("swatches.add".to_owned(), params)
            } else {
                let mut params = new_params;
                if let Some(group) = destination_group.clone() {
                    params["group"] = json!(group);
                }
                (format!("{prefix}.new"), params)
            };
            if let Some(r) = run(app, &command, params) {
                let name = r.get("name").or(r.get("pattern")).and_then(Value::as_str).unwrap_or_default().to_string();
                let group = r.get("group").and_then(Value::as_str).map(str::to_owned).or(destination_group).or_else(|| generic_item_group(app, prefix, &name));
                // Non-swatch domain commands append when creating. Move the new card to the
                // requested sibling position after creation; swatches accept an index directly.
                if prefix != "swatches"
                    && let (Some(group), Some(index)) = (group.as_deref(), insertion_index)
                {
                    let _ = run(app, &edit, json!({"action":"move", "preset":name, "group":group, "to":group, "index":index}));
                }
                state.clear_browser_selection(panel);
                // Creation replaces the previous selection rather than adding to it. Keep the
                // new card as the sole legacy/active selection, just as CreateGroup selects its
                // newly-created group after clearing the old card and browser selections.
                state.clear_selected(panel);
                if let Some(group) = group.as_deref() {
                    state.set_selected_item(panel, group, &destination_folder, &name);
                    if root_destination || selected_root_item.is_some() {
                        let after = selected_root_item.map(|(_, key, _)| key.as_str());
                        insert_generic_root_item(state, panel, &generic_group_names(app, prefix), group, &name, after);
                    } else if destination_folder.is_empty() {
                        let after = destination_item.as_ref().and_then(|(anchor_group, anchor_key, anchor_path)| {
                            (anchor_group == group && anchor_path.is_empty()).then_some(anchor_key.as_str())
                        });
                        insert_generic_group_item(app, state, panel, prefix, group, &name, after);
                    } else if let Some(folder) = destination_folder.last() {
                        let mut entries = state.hierarchy_order.get(panel).and_then(|orders| orders.get(folder)).cloned().unwrap_or_default();
                        let entry = crate::pickers::state::HierarchyOrderEntry::Item(name.clone());
                        entries.retain(|existing| existing != &entry);
                        let after = destination_item.as_ref().and_then(|(anchor_group, anchor_key, anchor_path)| {
                            (anchor_group == group && anchor_path == &destination_folder).then_some(anchor_key)
                        });
                        let at = after
                            .and_then(|key| entries.iter().position(|existing| existing == &crate::pickers::state::HierarchyOrderEntry::Item(key.clone())))
                            .map(|index| index + 1)
                            .unwrap_or(entries.len());
                        entries.insert(at, entry);
                        state.set_hierarchy_order(panel, folder, entries);
                    }
                } else {
                    state.set_selected(panel, &name);
                }
                if prefix != "pattern.presets" {
                    state.renaming = Some((panel.into(), name.clone(), name));
                }
                if let Some(group) = group.as_deref()
                    && !root_destination
                    && selected_root_item.is_none()
                {
                    expand_creation_destination(state, panel, group, &destination_folder);
                }
                true
            } else {
                false
            }
        }
        BrowserAction::AppendDefault if prefix == "style.presets" || prefix == "shape.presets" => {
            run(app, &format!("{prefix}.reset"), json!({"append": true})).is_some()
        }
        BrowserAction::SelectItem { .. }
        | BrowserAction::ActivateItem { .. }
        | BrowserAction::Drop(_, _)
        | BrowserAction::AppendDefault
        | BrowserAction::Import
        | BrowserAction::ExportSelected(_)
        | BrowserAction::ExportAll(_)
        | BrowserAction::CopyItem { .. } => false,
    }
}

fn generic_active_selection(
    app: &PhotocraftApp,
    state: &crate::pickers::state::PresetUi,
    panel: &str,
    prefix: &str,
) -> crate::pickers::browser::selection::BrowserSelection {
    use crate::pickers::browser::selection::{BrowserSelection, SelectionTarget};

    if let Some(key) = state.selected.get(panel) {
        if let Some(group) = state.selected_group.get(panel).cloned().or_else(|| generic_item_group(app, prefix, key)) {
            return BrowserSelection {
                targets: vec![SelectionTarget::Item { group, path: state.selected_group_path.get(panel).cloned().unwrap_or_default(), key: key.clone() }],
                ..Default::default()
            };
        }
    }
    match state.selected_group.get(panel) {
        Some(group) => {
            let path = state.selected_group_path.get(panel).cloned().unwrap_or_default();
            BrowserSelection {
                targets: vec![if path.is_empty() { SelectionTarget::Group(group.clone()) } else { SelectionTarget::Folder { group: group.clone(), path } }],
                ..Default::default()
            }
        }
        None => BrowserSelection::default(),
    }
}

#[derive(Clone, Debug)]
pub(crate) struct FolderNodePath {
    pub(crate) full: Vec<String>,
    pub(crate) parent: Vec<String>,
    pub(crate) entry: crate::pickers::state::HierarchyOrderEntry,
}

#[derive(Clone, Debug)]
pub(crate) struct FolderCreationPlan {
    pub(crate) parent: Vec<String>,
    pub(crate) before: Option<crate::pickers::state::HierarchyOrderEntry>,
    pub(crate) after: bool,
    pub(crate) selected: Vec<(crate::pickers::browser::selection::SelectionTarget, FolderNodePath)>,
}

fn folder_node_path(
    state: &crate::pickers::state::PresetUi,
    panel: &str,
    target: &crate::pickers::browser::selection::SelectionTarget,
    root_item_group: Option<&str>,
) -> Option<FolderNodePath> {
    use crate::pickers::browser::selection::SelectionTarget;
    use crate::pickers::state::HierarchyOrderEntry as Entry;

    let parents = state.group_parents(panel);
    let ancestors = |group: &str| {
        let mut path = Vec::new();
        let mut current = group;
        let mut seen = std::collections::BTreeSet::new();
        while let Some(parent) = parents.get(current) {
            if !seen.insert(parent.clone()) {
                break;
            }
            path.push(parent.clone());
            current = parent;
        }
        path.reverse();
        path
    };

    let (full, entry) = match target {
        SelectionTarget::Group(group) => {
            let mut full = ancestors(group);
            full.push(group.clone());
            (full, Entry::Group(group.clone()))
        }
        SelectionTarget::Folder { group, path } => {
            if path.is_empty() {
                return None;
            }
            let mut full = ancestors(group);
            full.push(group.clone());
            full.extend(path.iter().cloned());
            let name = path.last()?.clone();
            (full, Entry::Group(name))
        }
        SelectionTarget::Item { group, path, key } => {
            let is_root_item =
                path.is_empty() && (group.is_empty() || root_item_group == Some(group.as_str()) && is_lifted_root_item(state, panel, group, key));
            let mut full = if is_root_item { Vec::new() } else { ancestors(group) };
            if !is_root_item {
                full.push(group.clone());
            }
            full.extend(path.iter().cloned());
            full.push(key.clone());
            (full, Entry::Item(key.clone()))
        }
    };
    if let SelectionTarget::Item { key, group, path } = target {
        let is_root_item = path.is_empty() && (group.is_empty() || root_item_group == Some(group.as_str()) && is_lifted_root_item(state, panel, group, key));
        if is_root_item {
            return Some(FolderNodePath { parent: Vec::new(), full: vec![key.clone()], entry: Entry::RootItem { group: group.clone(), key: key.clone() } });
        }
    }
    let parent = full[..full.len().saturating_sub(1)].to_vec();
    Some(FolderNodePath { full, parent, entry })
}

fn is_lifted_root_item(state: &crate::pickers::state::PresetUi, panel: &str, group: &str, key: &str) -> bool {
    state
        .hierarchy_order
        .get(panel)
        .and_then(|orders| orders.get(""))
        .is_some_and(|entries| entries.contains(&crate::pickers::state::HierarchyOrderEntry::RootItem { group: group.to_owned(), key: key.to_owned() }))
}

pub(crate) fn folder_creation_plan(
    state: &crate::pickers::state::PresetUi,
    panel: &str,
    selection: &crate::pickers::browser::selection::BrowserSelection,
    root_item_group: Option<&str>,
) -> Option<FolderCreationPlan> {
    use crate::pickers::browser::selection::SelectionTarget;

    let mut nodes = selection
        .targets
        .iter()
        .filter_map(|target| folder_node_path(state, panel, target, root_item_group).map(|path| (target.clone(), path)))
        .collect::<Vec<_>>();
    if nodes.is_empty() {
        return None;
    }
    if nodes.len() == 1 {
        let (_target, node) = nodes.remove(0);
        return Some(FolderCreationPlan { parent: node.parent.clone(), before: Some(node.entry.clone()), after: true, selected: Vec::new() });
    }
    let mut common = nodes[0].1.parent.clone();
    for (_, node) in nodes.iter().skip(1) {
        let shared = common.iter().zip(&node.parent).take_while(|(a, b)| a == b).count();
        common.truncate(shared);
    }
    // The shallowest named common ancestor is the first shared container below the browser root.
    // This intentionally lifts a deeper first selection out of every intervening parent.
    let parent = common.first().cloned().into_iter().collect::<Vec<_>>();
    let first = &nodes[0].1;
    let branch = first.full.get(parent.len()).cloned();
    let before = branch.map(|branch| {
        if !matches!(nodes[0].0, SelectionTarget::Item { .. }) || first.full.len() > parent.len() + 1 {
            crate::pickers::state::HierarchyOrderEntry::Group(branch)
        } else {
            first.entry.clone()
        }
    });

    // A selected ancestor already carries its descendants into the new folder. Avoid trying to
    // move a descendant again after its selected parent has been reparented.
    let selected_paths = nodes.iter().map(|(_, node)| node.full.clone()).collect::<Vec<_>>();
    nodes.retain(|(_, node)| !selected_paths.iter().any(|other| other.len() < node.full.len() && node.full.starts_with(other)));
    Some(FolderCreationPlan { parent, before, after: false, selected: nodes })
}

fn generic_group_names(app: &PhotocraftApp, prefix: &str) -> Vec<String> {
    match prefix {
        "gradient.presets" => app.session.presets.gradients.iter().map(|group| group.name.clone()).filter(|name| name != "__photocraft_root_items__").collect(),
        "pattern.presets" => app.session.presets.pattern_groups.iter().map(|group| group.name.clone()).collect(),
        "style.presets" => app.session.presets.styles.iter().map(|group| group.name.clone()).collect(),
        "shape.presets" => app.session.presets.shapes.iter().map(|group| group.name.clone()).collect(),
        "swatches" => app.session.presets.swatches.iter().map(|group| group.name.clone()).collect(),
        _ => Vec::new(),
    }
}

fn generic_root_item_group(app: &PhotocraftApp, prefix: &str) -> Option<String> {
    if prefix == "gradient.presets" { Some("__photocraft_root_items__".into()) } else { generic_group_names(app, prefix).into_iter().next() }
}

fn insert_generic_root_item(state: &mut crate::pickers::state::PresetUi, panel: &str, group_names: &[String], owner: &str, key: &str, after: Option<&str>) {
    use crate::pickers::state::HierarchyOrderEntry as Entry;

    let mut root = state.hierarchy_order.get(panel).and_then(|orders| orders.get("")).cloned().unwrap_or_default();
    let mut represented_groups: std::collections::BTreeSet<String> = root
        .iter()
        .filter_map(|entry| match entry {
            Entry::Group(name) => Some(name.clone()),
            _ => None,
        })
        .collect();
    for (group_index, depth) in state.nested_display_order(panel, group_names) {
        let Some(group_name) = group_names.get(group_index) else { continue };
        if depth == 0 && represented_groups.insert(group_name.clone()) {
            root.push(Entry::Group(group_name.clone()));
        }
    }
    let entry = Entry::RootItem { group: owner.to_owned(), key: key.to_owned() };
    root.retain(|existing| existing != &entry);
    let at = after
        .and_then(|anchor| root.iter().position(|existing| existing == &Entry::RootItem { group: owner.to_owned(), key: anchor.to_owned() }))
        .map(|index| index + 1)
        .unwrap_or(root.len());
    root.insert(at, entry);
    state.set_hierarchy_order(panel, "", root);
}

fn insert_generic_group_item(
    app: &PhotocraftApp,
    state: &mut crate::pickers::state::PresetUi,
    panel: &str,
    prefix: &str,
    group: &str,
    key: &str,
    after: Option<&str>,
) {
    use crate::pickers::state::HierarchyOrderEntry as Entry;

    // Resolve the complete live sibling sequence first. A partial or absent saved order must not
    // make the new card jump ahead of existing cards when the browser next projects the hierarchy.
    let mut entries = sibling_entries(app, state, panel, prefix, group);
    let entry = Entry::Item(key.to_owned());
    entries.retain(|existing| existing != &entry);
    let at = after
        .and_then(|anchor| entries.iter().position(|existing| existing == &Entry::Item(anchor.to_owned())))
        .map(|index| index + 1)
        .unwrap_or(entries.len());
    entries.insert(at, entry);
    state.set_hierarchy_order(panel, group, entries);
}

fn generic_group_item_keys(app: &PhotocraftApp, prefix: &str, group: &str) -> Vec<String> {
    match prefix {
        "gradient.presets" => app
            .session
            .presets
            .gradients
            .iter()
            .find(|value| value.name == group)
            .map(|value| value.items.iter().map(|item| item.name.clone()).collect())
            .unwrap_or_default(),
        "pattern.presets" => app.session.presets.pattern_groups.iter().find(|value| value.name == group).map(|value| value.items.clone()).unwrap_or_default(),
        "style.presets" => app
            .session
            .presets
            .styles
            .iter()
            .find(|value| value.name == group)
            .map(|value| value.items.iter().map(|item| item.name.clone()).collect())
            .unwrap_or_default(),
        "shape.presets" => app
            .session
            .presets
            .shapes
            .iter()
            .find(|value| value.name == group)
            .map(|value| value.items.iter().map(|item| item.name.clone()).collect())
            .unwrap_or_default(),
        "swatches" => app
            .session
            .presets
            .swatches
            .iter()
            .find(|value| value.name == group)
            .map(|value| value.items.iter().map(|item| item.name.clone()).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn sibling_entries(
    app: &PhotocraftApp,
    state: &crate::pickers::state::PresetUi,
    panel: &str,
    prefix: &str,
    parent: &str,
) -> Vec<crate::pickers::state::HierarchyOrderEntry> {
    let names = generic_group_names(app, prefix);
    let parents = state.group_parents(panel);
    let child_groups = names.iter().filter(|name| parents.get(*name).map(String::as_str).unwrap_or("") == parent).cloned().collect::<Vec<_>>();
    let item_keys = if parent.is_empty() { Vec::new() } else { generic_group_item_keys(app, prefix, parent) };
    let mut entries = state.ordered_hierarchy_entries(panel, parent, &item_keys, &child_groups);
    if parent.is_empty() && prefix == "gradient.presets" {
        if let Some(root) = app.session.presets.gradients.iter().find(|group| group.name == "__photocraft_root_items__") {
            for item in &root.items {
                let entry = crate::pickers::state::HierarchyOrderEntry::RootItem { group: root.name.clone(), key: item.name.clone() };
                if !entries.contains(&entry) {
                    entries.push(entry);
                }
            }
        }
    }
    entries
}

fn move_selection_into_new_group(
    app: &mut PhotocraftApp,
    state: &mut crate::pickers::state::PresetUi,
    panel: &str,
    prefix: &str,
    new_group: &str,
    selection: &crate::pickers::browser::selection::BrowserSelection,
) -> Result<(), String> {
    use crate::pickers::browser::selection::SelectionTarget;
    use crate::pickers::state::HierarchyOrderEntry as Entry;

    let root_item_group = generic_root_item_group(app, prefix);
    let Some(plan) = folder_creation_plan(state, panel, selection, root_item_group.as_deref()) else {
        if selection.targets.is_empty() {
            let mut root = sibling_entries(app, state, panel, prefix, "");
            let entry = Entry::Group(new_group.to_owned());
            root.retain(|existing| existing != &entry);
            root.push(entry);
            state.set_hierarchy_order(panel, "", root);
        }
        return Ok(());
    };
    let parent_name = plan.parent.last().map(String::as_str).unwrap_or("");
    // Preserve explicitly persisted folder tokens (including empty folders) while inserting the
    // new group. The domain-derived projection only knows real preset groups and cards, so using
    // it alone would silently discard virtual folder siblings.
    let mut parent_entries = state
        .hierarchy_order
        .get(panel)
        .and_then(|orders| orders.get(parent_name))
        .cloned()
        .unwrap_or_else(|| sibling_entries(app, state, panel, prefix, parent_name));
    let mut insertion_at = plan
        .before
        .as_ref()
        .and_then(|before| parent_entries.iter().position(|entry| entry == before))
        .map(|index| index + usize::from(plan.after))
        .unwrap_or(parent_entries.len());
    let mut new_children = Vec::new();

    for (target, node) in &plan.selected {
        match target {
            SelectionTarget::Group(group) => {
                let old_parent = node.parent.last().map(String::as_str).unwrap_or("");
                let mut old_entries = sibling_entries(app, state, panel, prefix, old_parent);
                old_entries.retain(|entry| entry != &Entry::Group(group.clone()));
                state.set_hierarchy_order(panel, old_parent, old_entries);
                if old_parent == parent_name
                    && let Some(position) = parent_entries.iter().position(|entry| entry == &Entry::Group(group.clone()))
                {
                    if position < insertion_at {
                        insertion_at -= 1;
                    }
                    parent_entries.remove(position);
                }
                new_children.push(Entry::Group(group.clone()));
            }
            SelectionTarget::Folder { path, .. } => {
                let Some(folder_name) = path.last() else { continue };
                let old_parent = node.parent.last().map(String::as_str).unwrap_or("");
                let entry = Entry::Group(folder_name.clone());
                // Nested folders are virtual hierarchy entries, not engine preset groups. Resolve
                // their old sibling list from the canonical saved sequence so normalization does
                // not replace the folder token with its backing cards.
                let mut old_entries = state
                    .hierarchy_order
                    .get(panel)
                    .and_then(|parents| parents.get(old_parent))
                    .cloned()
                    .unwrap_or_else(|| sibling_entries(app, state, panel, prefix, old_parent));
                old_entries.retain(|existing| existing != &entry);
                state.set_hierarchy_order(panel, old_parent, old_entries);
                if old_parent == parent_name
                    && let Some(position) = parent_entries.iter().position(|existing| existing == &entry)
                {
                    if position < insertion_at {
                        insertion_at -= 1;
                    }
                    parent_entries.remove(position);
                }
                new_children.push(entry);
            }
            SelectionTarget::Item { group, path, key } => {
                let source_parent = node.parent.last().map(String::as_str).unwrap_or("");
                let item_index = new_children.iter().filter(|entry| matches!(entry, Entry::Item(_))).count();
                let (command, params) = if prefix == "swatches" {
                    ("swatches.move".to_owned(), json!({"swatch": key, "group": group, "to": new_group, "position": item_index}))
                } else {
                    (format!("{prefix}.edit"), json!({"action":"move", "preset":key, "group":group, "to":new_group, "index":item_index}))
                };
                run(app, &command, params).ok_or_else(|| app.ui.status.clone())?;
                let mut old_entries = sibling_entries(app, state, panel, prefix, source_parent);
                let old_entry = if root_item_group.as_deref() == Some(group.as_str()) && path.is_empty() && is_lifted_root_item(state, panel, group, key) {
                    Entry::RootItem { group: group.clone(), key: key.clone() }
                } else {
                    Entry::Item(key.clone())
                };
                old_entries.retain(|entry| entry != &old_entry);
                state.set_hierarchy_order(panel, source_parent, old_entries);
                if source_parent == parent_name
                    && let Some(position) = parent_entries.iter().position(|entry| entry == &old_entry)
                {
                    if position < insertion_at {
                        insertion_at -= 1;
                    }
                    parent_entries.remove(position);
                }
                new_children.push(Entry::Item(key.clone()));
            }
        }
    }

    parent_entries.retain(|entry| entry != &Entry::Group(new_group.to_owned()));
    parent_entries.insert(insertion_at.min(parent_entries.len()), Entry::Group(new_group.to_owned()));
    state.set_hierarchy_order(panel, parent_name, parent_entries);
    state.set_hierarchy_order(panel, new_group, new_children);
    Ok(())
}

fn generic_group_len(app: &PhotocraftApp, prefix: &str, group: &str) -> usize {
    match prefix {
        "gradient.presets" => app.session.presets.gradients.iter().find(|g| g.name == group).map_or(0, |g| g.items.len()),
        "pattern.presets" => app.session.presets.pattern_groups.iter().find(|g| g.name == group).map_or(0, |g| g.items.len()),
        "style.presets" => app.session.presets.styles.iter().find(|g| g.name == group).map_or(0, |g| g.items.len()),
        "shape.presets" => app.session.presets.shapes.iter().find(|g| g.name == group).map_or(0, |g| g.items.len()),
        "swatches" => app.session.presets.swatches.iter().find(|g| g.name == group).map_or(0, |g| g.items.len()),
        "tool.presets" if group.is_empty() => app.session.presets.tool_presets.len(),
        _ => 0,
    }
}

/// Expand the complete hierarchy path containing a newly-created card. Group openness is an
/// explicit persisted set; folder openness is the absence of a collapsed-folder key. Only the
/// destination and its ancestors change, leaving every unrelated disclosure untouched.
pub(crate) fn expand_creation_destination(state: &mut crate::pickers::state::PresetUi, panel: &str, group: &str, folder: &[String]) {
    if panel == "gradients" && group == "__photocraft_root_items__" {
        return;
    }
    let parents = state.group_parents(panel);
    let mut ancestors = Vec::new();
    let mut current = group.to_owned();
    let mut seen = std::collections::BTreeSet::new();
    while seen.insert(current.clone()) {
        ancestors.push(current.clone());
        let Some(parent) = parents.get(&current) else { break };
        current = parent.clone();
    }
    for ancestor in ancestors.iter().rev() {
        let key = format!("{panel}/{}", crate::pickers::state::escape_hierarchy_key_part(ancestor));
        state.expanded_groups.insert(key);
    }

    let group_key = crate::pickers::state::escape_hierarchy_key_part(group);
    for depth in 1..=folder.len() {
        let path = folder[..depth].iter().map(|part| crate::pickers::state::escape_hierarchy_key_part(part)).collect::<Vec<_>>().join("/");
        state.toggled.remove(&format!("{panel}/{group_key}/{path}"));
    }
}

fn generic_item_group(app: &PhotocraftApp, prefix: &str, key: &str) -> Option<String> {
    match prefix {
        "gradient.presets" => app.session.presets.gradients.iter().find(|g| g.items.iter().any(|item| item.name == key)).map(|g| g.name.clone()),
        "pattern.presets" => app.session.presets.pattern_groups.iter().find(|g| g.items.iter().any(|item| item == key)).map(|g| g.name.clone()),
        "style.presets" => app.session.presets.styles.iter().find(|g| g.items.iter().any(|item| item.name == key)).map(|g| g.name.clone()),
        "shape.presets" => app.session.presets.shapes.iter().find(|g| g.items.iter().any(|item| item.name == key)).map(|g| g.name.clone()),
        "swatches" => app.session.presets.swatches.iter().find(|g| g.items.iter().any(|item| item.name == key)).map(|g| g.name.clone()),
        "tool.presets" => app.session.presets.tool_presets.iter().any(|preset| preset.name == key).then(String::new),
        _ => None,
    }
}

fn generic_item_index(app: &PhotocraftApp, prefix: &str, group: &str, key: &str) -> Option<usize> {
    match prefix {
        "gradient.presets" => app.session.presets.gradients.iter().find(|g| g.name == group)?.items.iter().position(|i| i.name == key),
        "pattern.presets" => app.session.presets.pattern_groups.iter().find(|g| g.name == group)?.items.iter().position(|i| i == key),
        "style.presets" => app.session.presets.styles.iter().find(|g| g.name == group)?.items.iter().position(|i| i.name == key),
        "shape.presets" => app.session.presets.shapes.iter().find(|g| g.name == group)?.items.iter().position(|i| i.name == key),
        "swatches" => app.session.presets.swatches.iter().find(|g| g.name == group)?.items.iter().position(|i| i.name == key),
        "tool.presets" if group.is_empty() => app.session.presets.tool_presets.iter().position(|preset| preset.name == key),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{expand_creation_destination, folder_node_path, generic};
    use crate::PhotocraftApp;
    use crate::pickers::browser::{
        asset_browser::BrowserAction,
        selection::{BrowserSelection, SelectionTarget},
    };
    use serde_json::json;

    #[test]
    fn generic_creation_expands_nested_destinations_for_every_standard_asset_picker() {
        for panel in ["patterns", "styles", "shapes", "swatches"] {
            let mut state = crate::pickers::state::PresetUi::default();
            state.set_hierarchy_order(
                panel,
                "",
                vec![
                    crate::pickers::state::HierarchyOrderEntry::Group("Parent".into()),
                    crate::pickers::state::HierarchyOrderEntry::Group("Sibling".into()),
                    crate::pickers::state::HierarchyOrderEntry::Group("Other".into()),
                ],
            );
            state.set_hierarchy_order(panel, "Parent", vec![crate::pickers::state::HierarchyOrderEntry::Group("Destination".into())]);
            state.expanded_groups.insert(format!("{panel}/Sibling"));
            state.toggled.insert(format!("{panel}/Destination/Folder"));

            expand_creation_destination(&mut state, panel, "Destination", &["Folder".into()]);

            assert!(crate::pickers::browser::groups::group_open(&state, panel, "Parent"), "the destination's ancestor must open for {panel}");
            assert!(crate::pickers::browser::groups::group_open(&state, panel, "Destination"), "the destination group must open for {panel}");
            assert!(crate::pickers::browser::groups::group_open(&state, panel, "Sibling"), "an unrelated group that was open must remain open for {panel}");
            assert!(!crate::pickers::browser::groups::group_open(&state, panel, "Other"), "an unrelated collapsed group must remain closed for {panel}");
            assert!(!state.toggled.contains(&format!("{panel}/Destination/Folder")), "the destination folder must open for {panel}");
        }
    }

    #[test]
    fn only_explicitly_lifted_cards_are_treated_as_root_items_during_folder_planning() {
        use crate::pickers::state::HierarchyOrderEntry as Entry;

        let mut state = crate::pickers::state::PresetUi::default();
        state.set_hierarchy_order("swatches", "", vec![Entry::Group("A".into()), Entry::Group("B".into())]);
        let regular = SelectionTarget::Item { group: "A".into(), path: Vec::new(), key: "regular".into() };
        let regular_path = folder_node_path(&state, "swatches", &regular, Some("A")).unwrap();
        assert_eq!(regular_path.parent, vec!["A".to_owned()]);
        assert_eq!(regular_path.entry, Entry::Item("regular".into()));

        state.set_hierarchy_order(
            "swatches",
            "",
            vec![Entry::Group("A".into()), Entry::RootItem { group: "A".into(), key: "root".into() }, Entry::Group("B".into())],
        );
        let root = SelectionTarget::Item { group: "A".into(), path: Vec::new(), key: "root".into() };
        let root_path = folder_node_path(&state, "swatches", &root, Some("A")).unwrap();
        assert!(root_path.parent.is_empty());
        assert_eq!(root_path.entry, Entry::RootItem { group: "A".into(), key: "root".into() });

        let ungrouped = SelectionTarget::Item { group: String::new(), path: Vec::new(), key: "ungrouped".into() };
        let ungrouped_path = folder_node_path(&state, "swatches", &ungrouped, None).unwrap();
        assert!(ungrouped_path.parent.is_empty());
        assert_eq!(ungrouped_path.entry, Entry::RootItem { group: String::new(), key: "ungrouped".into() });
    }

    #[test]
    fn generic_creation_after_a_group_card_updates_the_mixed_sibling_order() {
        use crate::pickers::state::HierarchyOrderEntry as Entry;

        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let group = app.session.presets.swatches[0].name.clone();
        app.run("swatches.add", json!({"name":"Anchor", "group":group.clone()})).unwrap();
        app.run("swatches.add", json!({"name":"Other", "group":group.clone()})).unwrap();
        app.ui.presets_ui.browser_selection_snapshot.insert(
            "swatches".into(),
            BrowserSelection { targets: vec![SelectionTarget::Item { group: group.clone(), path: Vec::new(), key: "Anchor".into() }], ..Default::default() },
        );

        assert!(generic(&mut app, "swatches", "swatches", &BrowserAction::New, json!({})));

        let created = app.ui.presets_ui.selected["swatches"].clone();
        let siblings = &app.ui.presets_ui.hierarchy_order["swatches"][&group];
        let anchor_index = siblings.iter().position(|entry| entry == &Entry::Item("Anchor".into())).unwrap();
        assert_eq!(siblings.get(anchor_index + 1), Some(&Entry::Item(created)));
        assert_eq!(siblings.get(anchor_index + 2), Some(&Entry::Item("Other".into())));
    }

    #[test]
    fn generic_creation_without_selection_appends_a_root_item_after_root_groups() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let first_group = app.session.presets.swatches.first().map(|group| group.name.clone()).unwrap_or_default();
        let root_groups =
            app.session.presets.swatches.iter().map(|group| crate::pickers::state::HierarchyOrderEntry::Group(group.name.clone())).collect::<Vec<_>>();
        assert!(generic(&mut app, "swatches", "swatches", &BrowserAction::New, json!({})));

        let created = app.ui.presets_ui.selected.get("swatches").cloned().expect("the new swatch becomes selected");
        let group = app.ui.presets_ui.selected_group.get("swatches").cloned().expect("the new swatch retains its backing group");
        assert_eq!(group, if first_group.is_empty() { "Swatches".into() } else { first_group });
        assert!(!app.ui.presets_ui.expanded_groups.contains(&format!("swatches/{}", crate::pickers::state::escape_hierarchy_key_part(&group))));
        let root = &app.ui.presets_ui.hierarchy_order["swatches"][""];
        assert_eq!(&root[..root_groups.len()], root_groups.as_slice());
        assert_eq!(root.last(), Some(&crate::pickers::state::HierarchyOrderEntry::RootItem { group, key: created }));
    }

    #[test]
    fn generic_creation_after_a_root_card_stays_at_root_and_inserts_immediately_after_it() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        assert!(generic(&mut app, "swatches", "swatches", &BrowserAction::New, json!({})));
        let first = app.ui.presets_ui.selected["swatches"].clone();
        assert!(generic(&mut app, "swatches", "swatches", &BrowserAction::New, json!({})));
        let second = app.ui.presets_ui.selected["swatches"].clone();
        let owner = app.ui.presets_ui.selected_group["swatches"].clone();
        app.ui.presets_ui.browser_selection_snapshot.insert(
            "swatches".into(),
            BrowserSelection { targets: vec![SelectionTarget::Item { group: owner.clone(), path: Vec::new(), key: first.clone() }], ..Default::default() },
        );

        assert!(generic(&mut app, "swatches", "swatches", &BrowserAction::New, json!({})));

        let created = app.ui.presets_ui.selected["swatches"].clone();
        let root = &app.ui.presets_ui.hierarchy_order["swatches"][""];
        let first_index =
            root.iter().position(|entry| entry == &crate::pickers::state::HierarchyOrderEntry::RootItem { group: owner.clone(), key: first.clone() }).unwrap();
        assert_eq!(root.get(first_index + 1), Some(&crate::pickers::state::HierarchyOrderEntry::RootItem { group: owner.clone(), key: created.clone() }));
        assert!(root.contains(&crate::pickers::state::HierarchyOrderEntry::RootItem { group: owner, key: second }));
    }

    #[test]
    fn creating_a_tool_preset_inserts_after_the_selected_preset() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("tool.presets.new", json!({"name":"First", "tool":"brush", "options":{}})).unwrap();
        app.ui.presets_ui.browser_selection_snapshot.insert(
            "toolPresets".into(),
            BrowserSelection { targets: vec![SelectionTarget::Item { group: String::new(), path: Vec::new(), key: "First".into() }], ..Default::default() },
        );

        assert!(generic(&mut app, "toolPresets", "tool.presets", &BrowserAction::New, json!({"name":"Second", "tool":"brush", "options":{}})));

        let presets = &app.session.presets.tool_presets;
        let first = presets.iter().position(|preset| preset.name == "First").unwrap();
        assert_eq!(presets.get(first + 1).map(|preset| preset.name.as_str()), Some("Second"));
        assert_eq!(app.ui.presets_ui.selected.get("toolPresets").map(String::as_str), Some("Second"));
        assert_eq!(app.ui.presets_ui.selected_group.get("toolPresets").map(String::as_str), Some(""));
        assert!(app.ui.presets_ui.selected_group_path.get("toolPresets").is_some_and(Vec::is_empty));
        assert!(app.ui.presets_ui.browser_selection.get("toolPresets").is_none_or(|selected| selected.is_empty()));
        assert!(!app.ui.presets_ui.browser_selection_snapshot.contains_key("toolPresets"));
    }

    #[test]
    fn creating_a_tool_preset_after_multiple_selected_cards_uses_the_last_card_in_index_order() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.presets.tool_presets.clear();
        for name in ["First", "Second", "Third"] {
            app.run("tool.presets.new", json!({"name":name, "tool":"brush", "options":{}})).unwrap();
        }
        app.ui.presets_ui.browser_selection_snapshot.insert(
            "toolPresets".into(),
            BrowserSelection {
                targets: vec![
                    SelectionTarget::Item { group: String::new(), path: Vec::new(), key: "First".into() },
                    SelectionTarget::Item { group: String::new(), path: Vec::new(), key: "Second".into() },
                ],
                ..Default::default()
            },
        );

        assert!(generic(&mut app, "toolPresets", "tool.presets", &BrowserAction::New, json!({"name":"Created", "tool":"brush", "options":{}})));

        let names = app.session.presets.tool_presets.iter().map(|preset| preset.name.as_str()).collect::<Vec<_>>();
        assert_eq!(names, ["First", "Second", "Created", "Third"]);
        assert_eq!(app.ui.presets_ui.selected.get("toolPresets").map(String::as_str), Some("Created"));
        assert_eq!(app.ui.presets_ui.selected_group.get("toolPresets").map(String::as_str), Some(""));
        assert!(app.ui.presets_ui.browser_selection.get("toolPresets").is_none_or(|selected| selected.is_empty()));
        assert!(!app.ui.presets_ui.browser_selection_snapshot.contains_key("toolPresets"));
    }

    #[test]
    fn creating_an_item_uses_the_last_selected_element_even_when_it_is_a_group() {
        let mut app = empty_gradient_library();
        add_gradient_group(&mut app, "A");
        add_gradient_group(&mut app, "B");
        add_gradient(&mut app, "A", "Selected card");
        add_gradient(&mut app, "B", "Existing card");
        app.ui.presets_ui.browser_selection_snapshot.insert(
            "gradients".into(),
            BrowserSelection {
                targets: vec![SelectionTarget::Item { group: "A".into(), path: Vec::new(), key: "Selected card".into() }, SelectionTarget::Group("B".into())],
                ..Default::default()
            },
        );

        assert!(generic(&mut app, "gradients", "gradient.presets", &BrowserAction::NewNamed("Created".into()), json!({}),));

        let group_a = app.session.presets.gradients.iter().find(|group| group.name == "A").unwrap();
        let group_b = app.session.presets.gradients.iter().find(|group| group.name == "B").unwrap();
        assert_eq!(group_a.items.iter().map(|item| item.name.as_str()).collect::<Vec<_>>(), ["Selected card"]);
        assert_eq!(group_b.items.iter().map(|item| item.name.as_str()).collect::<Vec<_>>(), ["Existing card", "Created"]);
        assert_eq!(app.ui.presets_ui.selected.get("gradients").map(String::as_str), Some("Created"));
        assert_eq!(app.ui.presets_ui.selected_group.get("gradients").map(String::as_str), Some("B"));
        assert!(app.ui.presets_ui.selected_group_path.get("gradients").is_some_and(Vec::is_empty));
        assert!(app.ui.presets_ui.browser_selection.get("gradients").is_none_or(|selected| selected.is_empty()));
        assert!(!app.ui.presets_ui.browser_selection_snapshot.contains_key("gradients"));
    }

    fn empty_gradient_library() -> crate::PhotocraftApp {
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        for group in app.session.presets.gradients.clone() {
            app.run("gradient.presets.edit", json!({"action":"deleteGroup", "group":group.name})).unwrap();
        }
        app
    }

    fn add_gradient_group(app: &mut crate::PhotocraftApp, name: &str) {
        app.run("gradient.presets.edit", json!({"action":"newGroup", "name":name})).unwrap();
    }

    fn add_gradient(app: &mut crate::PhotocraftApp, group: &str, name: &str) {
        app.run("gradient.presets.new", json!({"group":group, "name":name})).unwrap();
    }

    #[test]
    fn creating_a_folder_with_no_selection_appends_at_root() {
        let mut app = empty_gradient_library();
        add_gradient_group(&mut app, "Existing");
        let mut state = crate::pickers::state::PresetUi::default();
        state.set_hierarchy_order("gradients", "", vec![crate::pickers::state::HierarchyOrderEntry::Group("Existing".into())]);

        assert!(super::generic_with_state(&mut app, &mut state, "gradients", "gradient.presets", &BrowserAction::CreateGroup("New".into()), json!({})));

        assert_eq!(
            state.hierarchy_order["gradients"][""],
            [crate::pickers::state::HierarchyOrderEntry::Group("Existing".into()), crate::pickers::state::HierarchyOrderEntry::Group("New".into()),]
        );
        assert!(!state.hierarchy_order["gradients"].contains_key("New"));
    }

    #[test]
    fn creating_a_folder_replaces_legacy_selection_and_clears_browser_multi_selection() {
        let mut app = empty_gradient_library();
        let mut state = crate::pickers::state::PresetUi::default();
        state.set_selected_item("gradients", "Old group", &[], "Old card");
        state.select_browser_node("gradients", crate::pickers::browser::path_identity_model::PickerId::from("old-a"), false);
        state.select_browser_node("gradients", crate::pickers::browser::path_identity_model::PickerId::from("old-b"), true);
        state.browser_selection_snapshot.insert("gradients".into(), BrowserSelection::default());

        assert!(super::generic_with_state(&mut app, &mut state, "gradients", "gradient.presets", &BrowserAction::CreateGroup("New".into()), json!({})));

        assert!(!state.browser_node_selected("gradients", &crate::pickers::browser::path_identity_model::PickerId::from("old-a")));
        assert!(!state.browser_node_selected("gradients", &crate::pickers::browser::path_identity_model::PickerId::from("old-b")));
        assert!(!state.browser_selection_anchor.contains_key("gradients"));
        assert!(!state.browser_selection_snapshot.contains_key("gradients"));
        assert!(!state.selected.contains_key("gradients"), "the previous card selection should be discarded");
        assert_eq!(state.selected_group.get("gradients").map(String::as_str), Some("New"));
        assert!(state.selected_group_path.get("gradients").is_some_and(Vec::is_empty));
    }

    #[test]
    fn consecutive_folder_creation_uses_the_previously_created_folder_as_the_sibling_anchor() {
        let mut app = empty_gradient_library();
        for name in ["A", "B"] {
            add_gradient_group(&mut app, name);
        }
        let mut state = crate::pickers::state::PresetUi::default();
        state.set_hierarchy_order(
            "gradients",
            "",
            vec![crate::pickers::state::HierarchyOrderEntry::Group("A".into()), crate::pickers::state::HierarchyOrderEntry::Group("B".into())],
        );
        state.selected_group.insert("gradients".into(), "A".into());
        state.selected_group_path.insert("gradients".into(), Vec::new());

        assert!(super::generic_with_state(&mut app, &mut state, "gradients", "gradient.presets", &BrowserAction::CreateGroup("First".into()), json!({})));
        assert_eq!(state.selected_group.get("gradients").map(String::as_str), Some("First"));
        assert!(!state.browser_selection_snapshot.contains_key("gradients"));

        assert!(super::generic_with_state(&mut app, &mut state, "gradients", "gradient.presets", &BrowserAction::CreateGroup("Second".into()), json!({})));

        assert_eq!(
            state.hierarchy_order["gradients"][""],
            [
                crate::pickers::state::HierarchyOrderEntry::Group("A".into()),
                crate::pickers::state::HierarchyOrderEntry::Group("First".into()),
                crate::pickers::state::HierarchyOrderEntry::Group("Second".into()),
                crate::pickers::state::HierarchyOrderEntry::Group("B".into()),
            ]
        );
        assert_eq!(state.selected_group.get("gradients").map(String::as_str), Some("Second"));
    }

    #[test]
    fn creating_a_folder_from_one_group_places_it_after_the_group_as_a_sibling() {
        let mut app = empty_gradient_library();
        for name in ["A", "B"] {
            add_gradient_group(&mut app, name);
        }
        let mut state = crate::pickers::state::PresetUi::default();
        state.set_hierarchy_order(
            "gradients",
            "",
            vec![crate::pickers::state::HierarchyOrderEntry::Group("A".into()), crate::pickers::state::HierarchyOrderEntry::Group("B".into())],
        );
        state
            .browser_selection_snapshot
            .insert("gradients".into(), BrowserSelection { targets: vec![SelectionTarget::Group("A".into())], ..Default::default() });

        assert!(super::generic_with_state(&mut app, &mut state, "gradients", "gradient.presets", &BrowserAction::CreateGroup("New".into()), json!({})));

        assert_eq!(
            state.hierarchy_order["gradients"][""],
            [
                crate::pickers::state::HierarchyOrderEntry::Group("A".into()),
                crate::pickers::state::HierarchyOrderEntry::Group("New".into()),
                crate::pickers::state::HierarchyOrderEntry::Group("B".into()),
            ]
        );
        assert_eq!(state.hierarchy_order["gradients"]["New"], []);
    }

    #[test]
    fn creating_a_folder_from_one_card_places_it_after_the_card_without_moving_it() {
        let mut app = empty_gradient_library();
        add_gradient_group(&mut app, "A");
        add_gradient(&mut app, "A", "First");
        add_gradient(&mut app, "A", "Second");
        let mut state = crate::pickers::state::PresetUi::default();
        state.set_hierarchy_order("gradients", "", vec![crate::pickers::state::HierarchyOrderEntry::Group("A".into())]);
        state.set_hierarchy_order(
            "gradients",
            "A",
            vec![crate::pickers::state::HierarchyOrderEntry::Item("First".into()), crate::pickers::state::HierarchyOrderEntry::Item("Second".into())],
        );
        state.browser_selection_snapshot.insert(
            "gradients".into(),
            BrowserSelection { targets: vec![SelectionTarget::Item { group: "A".into(), path: Vec::new(), key: "First".into() }], ..Default::default() },
        );

        assert!(super::generic_with_state(&mut app, &mut state, "gradients", "gradient.presets", &BrowserAction::CreateGroup("New".into()), json!({})));

        assert_eq!(
            state.hierarchy_order["gradients"]["A"],
            [
                crate::pickers::state::HierarchyOrderEntry::Item("First".into()),
                crate::pickers::state::HierarchyOrderEntry::Group("New".into()),
                crate::pickers::state::HierarchyOrderEntry::Item("Second".into()),
            ]
        );
        assert_eq!(state.hierarchy_order["gradients"]["New"], []);
        assert!(app.session.presets.gradients.iter().any(|group| group.name == "A" && group.items.iter().any(|item| item.name == "First")));
    }

    #[test]
    fn creating_a_folder_from_multi_selection_uses_the_shallowest_common_ancestor_and_lifts_the_first_branch() {
        let mut app = empty_gradient_library();
        for name in ["A", "B", "C"] {
            add_gradient_group(&mut app, name);
        }
        add_gradient(&mut app, "B", "First");
        add_gradient(&mut app, "C", "Second");
        let mut state = crate::pickers::state::PresetUi::default();
        state.set_hierarchy_order("gradients", "", vec![crate::pickers::state::HierarchyOrderEntry::Group("A".into())]);
        state.set_hierarchy_order(
            "gradients",
            "A",
            vec![crate::pickers::state::HierarchyOrderEntry::Group("B".into()), crate::pickers::state::HierarchyOrderEntry::Group("C".into())],
        );
        state.set_hierarchy_order("gradients", "B", vec![crate::pickers::state::HierarchyOrderEntry::Item("First".into())]);
        state.set_hierarchy_order("gradients", "C", vec![crate::pickers::state::HierarchyOrderEntry::Item("Second".into())]);
        state.browser_selection_snapshot.insert(
            "gradients".into(),
            BrowserSelection {
                targets: vec![
                    SelectionTarget::Item { group: "B".into(), path: Vec::new(), key: "First".into() },
                    SelectionTarget::Item { group: "C".into(), path: Vec::new(), key: "Second".into() },
                ],
                ..Default::default()
            },
        );

        assert!(super::generic_with_state(&mut app, &mut state, "gradients", "gradient.presets", &BrowserAction::CreateGroup("New".into()), json!({})));

        assert_eq!(
            state.hierarchy_order["gradients"]["A"],
            [
                crate::pickers::state::HierarchyOrderEntry::Group("New".into()),
                crate::pickers::state::HierarchyOrderEntry::Group("B".into()),
                crate::pickers::state::HierarchyOrderEntry::Group("C".into()),
            ]
        );
        assert_eq!(
            state.hierarchy_order["gradients"]["New"],
            [crate::pickers::state::HierarchyOrderEntry::Item("First".into()), crate::pickers::state::HierarchyOrderEntry::Item("Second".into()),]
        );
        assert!(
            app.session
                .presets
                .gradients
                .iter()
                .any(|group| group.name == "New" && group.items.iter().map(|item| item.name.as_str()).collect::<Vec<_>>() == ["First", "Second"])
        );
    }

    #[test]
    fn creating_a_folder_from_a_folder_and_a_card_moves_both_into_the_new_folder() {
        let mut app = empty_gradient_library();
        add_gradient_group(&mut app, "A");
        add_gradient_group(&mut app, "B");
        add_gradient(&mut app, "A", "Inside");
        add_gradient(&mut app, "B", "Other");
        let mut state = crate::pickers::state::PresetUi::default();
        state.set_hierarchy_order(
            "gradients",
            "",
            vec![crate::pickers::state::HierarchyOrderEntry::Group("A".into()), crate::pickers::state::HierarchyOrderEntry::Group("B".into())],
        );
        state.set_hierarchy_order("gradients", "A", vec![crate::pickers::state::HierarchyOrderEntry::Group("Nested".into())]);
        state.set_hierarchy_order("gradients", "Nested", vec![crate::pickers::state::HierarchyOrderEntry::Item("Inside".into())]);
        state.set_hierarchy_order("gradients", "B", vec![crate::pickers::state::HierarchyOrderEntry::Item("Other".into())]);
        state.browser_selection_snapshot.insert(
            "gradients".into(),
            BrowserSelection {
                targets: vec![
                    SelectionTarget::Folder { group: "A".into(), path: vec!["Nested".into()] },
                    SelectionTarget::Item { group: "B".into(), path: Vec::new(), key: "Other".into() },
                ],
                ..Default::default()
            },
        );

        assert!(super::generic_with_state(&mut app, &mut state, "gradients", "gradient.presets", &BrowserAction::CreateGroup("New".into()), json!({})));

        assert_eq!(
            state.hierarchy_order["gradients"][""],
            [
                crate::pickers::state::HierarchyOrderEntry::Group("New".into()),
                crate::pickers::state::HierarchyOrderEntry::Group("A".into()),
                crate::pickers::state::HierarchyOrderEntry::Group("B".into()),
            ]
        );
        assert!(state.hierarchy_order.get("gradients").and_then(|orders| orders.get("A")).is_none_or(Vec::is_empty));
        assert_eq!(
            state.hierarchy_order["gradients"]["New"],
            [crate::pickers::state::HierarchyOrderEntry::Group("Nested".into()), crate::pickers::state::HierarchyOrderEntry::Item("Other".into()),]
        );
        assert_eq!(state.hierarchy_order["gradients"]["Nested"], [crate::pickers::state::HierarchyOrderEntry::Item("Inside".into())]);
        assert!(app.session.presets.gradients.iter().any(|group| group.name == "New" && group.items.iter().any(|item| item.name == "Other")));
    }

    #[test]
    fn creating_a_folder_from_a_selected_folder_and_its_descendant_moves_the_ancestor_only() {
        let mut app = empty_gradient_library();
        add_gradient_group(&mut app, "A");
        add_gradient(&mut app, "A", "Inside");
        let mut state = crate::pickers::state::PresetUi::default();
        state.set_hierarchy_order("gradients", "", vec![crate::pickers::state::HierarchyOrderEntry::Group("A".into())]);
        state.set_hierarchy_order("gradients", "A", vec![crate::pickers::state::HierarchyOrderEntry::Group("Nested".into())]);
        state.set_hierarchy_order("gradients", "Nested", vec![crate::pickers::state::HierarchyOrderEntry::Item("Inside".into())]);
        state.browser_selection_snapshot.insert(
            "gradients".into(),
            BrowserSelection {
                targets: vec![
                    SelectionTarget::Folder { group: "A".into(), path: vec!["Nested".into()] },
                    SelectionTarget::Item { group: "A".into(), path: vec!["Nested".into()], key: "Inside".into() },
                ],
                ..Default::default()
            },
        );

        assert!(super::generic_with_state(&mut app, &mut state, "gradients", "gradient.presets", &BrowserAction::CreateGroup("New".into()), json!({})));

        assert_eq!(state.hierarchy_order["gradients"]["A"], [crate::pickers::state::HierarchyOrderEntry::Group("New".into())]);
        assert_eq!(state.hierarchy_order["gradients"]["New"], [crate::pickers::state::HierarchyOrderEntry::Group("Nested".into())]);
        assert_eq!(state.hierarchy_order["gradients"]["Nested"], [crate::pickers::state::HierarchyOrderEntry::Item("Inside".into())]);
        assert!(app.session.presets.gradients.iter().any(|group| group.name == "A" && group.items.iter().any(|item| item.name == "Inside")));
    }

    #[test]
    fn creating_a_folder_after_an_empty_nested_folder_preserves_its_sibling_position() {
        let mut app = empty_gradient_library();
        add_gradient_group(&mut app, "A");
        let mut state = crate::pickers::state::PresetUi::default();
        state.set_hierarchy_order("gradients", "", vec![crate::pickers::state::HierarchyOrderEntry::Group("A".into())]);
        state.set_hierarchy_order(
            "gradients",
            "A",
            vec![crate::pickers::state::HierarchyOrderEntry::Group("Empty".into()), crate::pickers::state::HierarchyOrderEntry::Group("Following".into())],
        );
        state.browser_selection_snapshot.insert(
            "gradients".into(),
            BrowserSelection { targets: vec![SelectionTarget::Folder { group: "A".into(), path: vec!["Empty".into()] }], ..Default::default() },
        );

        assert!(super::generic_with_state(&mut app, &mut state, "gradients", "gradient.presets", &BrowserAction::CreateGroup("New".into()), json!({})));

        assert_eq!(
            state.hierarchy_order["gradients"]["A"],
            [
                crate::pickers::state::HierarchyOrderEntry::Group("Empty".into()),
                crate::pickers::state::HierarchyOrderEntry::Group("New".into()),
                crate::pickers::state::HierarchyOrderEntry::Group("Following".into()),
            ]
        );
        assert!(state.hierarchy_order["gradients"]["New"].is_empty());
    }
}
