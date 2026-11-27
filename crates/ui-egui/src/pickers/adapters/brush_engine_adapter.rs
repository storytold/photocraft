//! Brush-library projection and command translation for the shared picker browser.
//!
//! The engine intentionally keeps its flat, name-addressed brush preset model. This adapter
//! projects that model into ordinary shared-browser groups and stores empty containers/order in
//! `PresetUi.layout`; no brush-specific hierarchy is required in the engine.

use std::collections::{BTreeMap, BTreeSet};

use photocraft_engine::paint::BrushPreset;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::pickers::browser::asset_browser::{BrowserAction, GroupView, ItemView};
use crate::pickers::browser::dragging_model::DropMoveIntent;
use crate::pickers::browser::identity::{PickerIdentityKey, PickerNodeIdentity, PickerProjectionKeys, ProjectionNodeRef};
use crate::pickers::browser::path_identity_model::{ContainerId, DropPosition, PickerBrowserHierarchy, PickerId};
use crate::pickers::state::{HierarchyOrderEntry, PresetUi};

pub const UNGROUPED: &str = "My Brushes";
pub(crate) const ROOT_ITEMS_GROUP: &str = "__photocraft_root_items__";

/// A root group or a nested folder as it exists in the engine's flat preset list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct BrushContainer {
    /// Stable key used by the shared browser's in-memory IDs and persisted layout.
    pub(crate) key: String,
    /// Engine group name (empty only for root cards).
    pub(crate) engine_group: String,
    /// Engine folder path, outermost first.
    pub(crate) folder: Vec<String>,
    /// Current user-visible label.
    pub(crate) label: String,
    /// The container is a root engine group rather than a folder.
    pub(crate) root_group: bool,
    /// Root-level cards are projected through the browser's mixed root sequence.
    pub(crate) root_items: bool,
}

#[derive(Clone)]
pub(super) struct BrushGroupView<'a> {
    pub(super) view: GroupView,
    pub(super) presets: Vec<&'a BrushPreset>,
}

pub(crate) struct BrushBrowserProjection<'a> {
    pub(crate) group_views: Vec<BrushGroupView<'a>>,
    pub(crate) groups: Vec<GroupView>,
    pub(crate) keys: PickerProjectionKeys,
    pub(crate) containers: BTreeMap<String, BrushContainer>,
}

fn hex(value: &str) -> String {
    value.as_bytes().iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(value: &str) -> Option<String> {
    if value.len() % 2 != 0 {
        return None;
    }
    let bytes = (0..value.len()).step_by(2).map(|index| u8::from_str_radix(&value[index..index + 2], 16).ok()).collect::<Option<Vec<_>>>()?;
    String::from_utf8(bytes).ok()
}

pub(crate) fn root_group_key(name: &str) -> String {
    format!("brush-group:{}", hex(name))
}

fn folder_key(group: &str, path: &[String]) -> String {
    format!("brush-folder:{}:{}", hex(group), path.iter().map(|part| hex(part)).collect::<Vec<_>>().join("."))
}

fn key_label(key: &str) -> String {
    if let Some(encoded) = key.strip_prefix("brush-group:") {
        return unhex(encoded).unwrap_or_else(|| key.to_owned());
    }
    if let Some(encoded) = key.strip_prefix("brush-folder:") {
        let (_, path) = encoded.split_once(':').unwrap_or(("", encoded));
        if let Some(last) = path.rsplit('.').next().filter(|part| !part.is_empty()) {
            return unhex(last).unwrap_or_else(|| key.to_owned());
        }
    }
    key.to_owned()
}

fn saved_child_key(state: &PresetUi, panel: &str, parent: &str, label: &str) -> Option<String> {
    state.layout.hierarchy_order.get(panel)?.get(parent)?.iter().find_map(|entry| match entry {
        HierarchyOrderEntry::Group(key) if key_label(key) == label => Some(key.clone()),
        _ => None,
    })
}

fn add_container(containers: &mut BTreeMap<String, BrushContainer>, container: BrushContainer) {
    containers.entry(container.key.clone()).or_insert(container);
}

fn container_for_preset<'a>(containers: &'a BTreeMap<String, BrushContainer>, preset: &BrushPreset) -> Option<&'a BrushContainer> {
    if preset.group.is_empty() && preset.folder.is_empty() {
        return containers.get(ROOT_ITEMS_GROUP);
    }
    containers.values().find(|container| container.engine_group == preset.group && container.folder == preset.folder)
}

/// Project the flat engine library and layout-declared empty containers into the shared browser.
pub(crate) fn project_brush_browser<'a>(presets: &'a [BrushPreset], state: &mut PresetUi, panel: &str) -> BrushBrowserProjection<'a> {
    let mut containers = BTreeMap::<String, BrushContainer>::new();
    let mut engine_groups = Vec::<String>::new();
    for preset in presets.iter().filter(|preset| !preset.group.is_empty()) {
        if !engine_groups.contains(&preset.group) {
            engine_groups.push(preset.group.clone());
        }
    }
    let root_presets: Vec<&BrushPreset> = presets.iter().filter(|preset| preset.group.is_empty() && preset.folder.is_empty()).collect();

    // Existing layout keys are authoritative. Reusing the key by sibling label keeps a folder's
    // picker ID stable when the shared hierarchy moves it to another parent.
    let mut root_group_keys = BTreeMap::<String, String>::new();
    for group in &engine_groups {
        let key = saved_child_key(state, panel, "", group).unwrap_or_else(|| root_group_key(group));
        root_group_keys.insert(group.clone(), key.clone());
        add_container(
            &mut containers,
            BrushContainer { key, engine_group: group.clone(), folder: Vec::new(), label: group.clone(), root_group: true, root_items: false },
        );
    }

    // Seed live folder paths from the flat preset list. A saved child Group entry wins over a
    // location-derived fallback key, so reparented folders retain their in-memory identity.
    let mut folder_paths = Vec::<(String, Vec<String>)>::new();
    for preset in presets.iter().filter(|preset| !preset.group.is_empty()) {
        for depth in 1..=preset.folder.len().min(crate::pickers::browser::hierarchy::MAX_HIERARCHY_DEPTH) {
            let path = preset.folder[..depth].to_vec();
            if !folder_paths.contains(&(preset.group.clone(), path.clone())) {
                folder_paths.push((preset.group.clone(), path));
            }
        }
    }
    for (group, path) in folder_paths {
        let mut parent_path = path[..path.len() - 1].to_vec();
        let parent_key = if parent_path.is_empty() {
            root_group_keys.get(&group).cloned().unwrap_or_else(|| root_group_key(&group))
        } else {
            containers
                .values()
                .find(|container| container.engine_group == group && container.folder == parent_path)
                .map(|container| container.key.clone())
                .unwrap_or_else(|| folder_key(&group, &parent_path))
        };
        let leaf = path.last().cloned().unwrap_or_default();
        let key = saved_child_key(state, panel, &parent_key, &leaf).unwrap_or_else(|| folder_key(&group, &path));
        add_container(
            &mut containers,
            BrushContainer { key, engine_group: group, folder: path, label: leaf, root_group: false, root_items: false },
        );
        parent_path.clear();
    }

    // Materialize layout-only groups/folders recursively. These entries are the durable source of
    // truth for empty containers because main's engine model deliberately has no empty groups.
    let legacy_root_items_alias = !engine_groups.iter().any(|group| group == UNGROUPED) && !root_presets.is_empty();
    let mut pending = state
        .layout
        .hierarchy_order
        .get(panel)
        .map(|orders| orders.iter().flat_map(|(parent, entries)| {
            entries.iter().filter_map(move |entry| match entry {
                HierarchyOrderEntry::Group(key)
                    if !(parent.is_empty() && key_label(key) == UNGROUPED && legacy_root_items_alias) =>
                {
                    Some((parent.clone(), key.clone()))
                }
                _ => None,
            })
        }).collect::<Vec<_>>())
        .unwrap_or_default();
    for _ in 0..pending.len().saturating_add(1) {
        let mut changed = false;
        let mut remaining = Vec::new();
        for (parent_key, key) in pending {
            if containers.contains_key(&key) {
                continue;
            }
            if parent_key.is_empty() {
                let label = key_label(&key);
                let group = label.clone();
                add_container(
                    &mut containers,
                    BrushContainer { key, engine_group: group, folder: Vec::new(), label, root_group: true, root_items: false },
                );
                changed = true;
            } else if let Some(parent) = containers.get(&parent_key).cloned() {
                let label = key_label(&key);
                let mut folder = parent.folder.clone();
                folder.push(label.clone());
                add_container(
                    &mut containers,
                    BrushContainer { key, engine_group: parent.engine_group, folder, label, root_group: false, root_items: false },
                );
                changed = true;
            } else {
                remaining.push((parent_key, key));
            }
        }
        pending = remaining;
        if !changed || pending.is_empty() {
            break;
        }
    }

    // Infer missing parent links for folders created from engine presets before using the shared
    // layout's parent map. Otherwise a first-run folder would be appended to the root sequence.
    for container in containers.values().filter(|container| !container.root_items && !container.folder.is_empty()) {
        if state.group_parents(panel).contains_key(&container.key) { continue; }
        let parent_folder = &container.folder[..container.folder.len() - 1];
        let parent_key = if parent_folder.is_empty() {
            root_group_keys.get(&container.engine_group).cloned().unwrap_or_else(|| root_group_key(&container.engine_group))
        } else {
            containers.values().find(|candidate| candidate.engine_group == container.engine_group && candidate.folder == parent_folder)
                .map(|candidate| candidate.key.clone()).unwrap_or_else(|| folder_key(&container.engine_group, parent_folder))
        };
        let mut entries = state.layout.hierarchy_order.get(panel).and_then(|orders| orders.get(&parent_key)).cloned().unwrap_or_default();
        let entry = HierarchyOrderEntry::Group(container.key.clone());
        if !entries.contains(&entry) { entries.push(entry); }
        state.set_hierarchy_order(panel, &parent_key, entries);
    }

    // Root cards use one reserved backing group; the label is not an engine group name.
    add_container(
        &mut containers,
        BrushContainer { key: ROOT_ITEMS_GROUP.into(), engine_group: String::new(), folder: Vec::new(), label: UNGROUPED.into(), root_group: false, root_items: true },
    );

    // Ensure every live root group and root card is represented in the persisted mixed sequence.
    let mut root_order = state.layout.hierarchy_order.get(panel).and_then(|orders| orders.get("")).cloned().unwrap_or_default();
    if !engine_groups.iter().any(|group| group == UNGROUPED) && !root_presets.is_empty() {
        root_order.retain(|entry| !matches!(entry, HierarchyOrderEntry::Group(key) if key_label(key) == UNGROUPED));
    }
    let parents = state.group_parents(panel);
    if root_order.is_empty() {
        for preset in presets {
            if preset.group.is_empty() && preset.folder.is_empty() {
                root_order.push(HierarchyOrderEntry::RootItem { group: ROOT_ITEMS_GROUP.into(), key: preset.name.clone() });
            } else if preset.folder.is_empty() {
                let key = root_group_keys.get(&preset.group).cloned().unwrap_or_else(|| root_group_key(&preset.group));
                let entry = HierarchyOrderEntry::Group(key);
                if !root_order.contains(&entry) { root_order.push(entry); }
            }
        }
    }
    for container in containers.values().filter(|container| container.root_group && parents.get(&container.key).is_none()) {
        let key = container.key.clone();
        if !root_order.contains(&HierarchyOrderEntry::Group(key.clone())) {
            root_order.push(HierarchyOrderEntry::Group(key));
        }
    }
    for preset in &root_presets {
        let entry = HierarchyOrderEntry::RootItem { group: ROOT_ITEMS_GROUP.into(), key: preset.name.clone() };
        if !root_order.iter().any(|existing| matches!(existing, HierarchyOrderEntry::RootItem { key, .. } if key == &preset.name)) {
            root_order.push(entry);
        }
    }
    // A card moved back to the root is normalized to the reserved root-item owner, regardless of
    // the owner key captured by the previous in-memory hierarchy commit.
    for entry in &mut root_order {
        if let HierarchyOrderEntry::RootItem { group, key } = entry
            && root_presets.iter().any(|preset| preset.name == *key)
        {
            *group = ROOT_ITEMS_GROUP.into();
        }
    }
    state.set_hierarchy_order(panel, "", root_order);

    let mut group_views = Vec::<BrushGroupView<'_>>::new();
    for container in containers.values() {
        let group_presets: Vec<&BrushPreset> = if container.root_items {
            root_presets.clone()
        } else {
            presets.iter().filter(|preset| preset.group == container.engine_group && preset.folder == container.folder).collect()
        };
        let items = group_presets
            .iter()
            .map(|preset| ItemView { key: preset.name.clone(), name: preset.name.clone(), group_path: Vec::new() })
            .collect::<Vec<_>>();
        group_views.push(BrushGroupView {
            view: GroupView { name: container.key.clone(), label: container.label.clone(), root_only: container.root_items, items },
            presets: group_presets,
        });
    }

    // First-time child sequences follow the engine's flat order; later frames keep the user's
    // canonical mixed item/group sequence unchanged.
    let parents = state.group_parents(panel);
    let mut children_by_parent = BTreeMap::<String, Vec<String>>::new();
    for container in containers.values().filter(|container| !container.root_items) {
        let parent = parents.get(&container.key).cloned().unwrap_or_default();
        children_by_parent.entry(parent).or_default().push(container.key.clone());
    }
    for container in containers.values() {
        if container.root_items { continue; }
        let child_groups = children_by_parent.get(&container.key).cloned().unwrap_or_default();
        if !state.layout.hierarchy_order.get(panel).is_some_and(|orders| orders.contains_key(&container.key)) {
            let mut entries = Vec::new();
            for preset in presets.iter().filter(|preset| preset.group == container.engine_group && preset.folder.starts_with(&container.folder)) {
                if preset.folder.len() == container.folder.len() {
                    let entry = HierarchyOrderEntry::Item(preset.name.clone());
                    if !entries.contains(&entry) { entries.push(entry); }
                } else {
                    let child_path = preset.folder[..container.folder.len() + 1].to_vec();
                    if let Some(child) = containers.values().find(|child| child.engine_group == container.engine_group && child.folder == child_path) {
                        let entry = HierarchyOrderEntry::Group(child.key.clone());
                        if !entries.contains(&entry) { entries.push(entry); }
                    }
                }
            }
            for child in child_groups {
                let entry = HierarchyOrderEntry::Group(child);
                if !entries.contains(&entry) { entries.push(entry); }
            }
            state.set_hierarchy_order(panel, &container.key, entries);
        }
    }

    for group in &mut group_views {
        let item_keys = group.view.items.iter().map(|item| item.key.clone()).collect::<Vec<_>>();
        let ranks = if group.view.root_only {
            state.layout.hierarchy_order.get(panel).and_then(|orders| orders.get("")).map(|entries| {
                entries.iter().enumerate().filter_map(|(index, entry)| match entry {
                    HierarchyOrderEntry::RootItem { group: owner, key } if owner == ROOT_ITEMS_GROUP => Some((key.clone(), index)),
                    _ => None,
                }).collect::<BTreeMap<_, _>>()
            }).unwrap_or_default()
        } else {
            let child_groups = children_by_parent.get(&group.view.name).cloned().unwrap_or_default();
            let entries = state.ordered_hierarchy_entries(panel, &group.view.name, &item_keys, &child_groups);
            entries.iter().enumerate().filter_map(|(index, entry)| match entry {
                HierarchyOrderEntry::Item(name) => Some((name.clone(), index)),
                _ => None,
            }).collect::<BTreeMap<_, _>>()
        };
        let mut paired = group.view.items.drain(..).zip(group.presets.drain(..)).collect::<Vec<_>>();
        paired.sort_by_key(|(item, _)| ranks.get(&item.key).copied().unwrap_or(usize::MAX));
        group.view.items = paired.iter().map(|(item, _)| item.clone()).collect();
        group.presets = paired.into_iter().map(|(_, preset)| preset).collect();
    }

    let names = group_views.iter().map(|group| group.view.name.clone()).collect::<Vec<_>>();
    let order = state.nested_display_order(panel, &names);
    let mut ordered = Vec::with_capacity(group_views.len());
    for (index, _) in order {
        if let Some(group) = group_views.get(index) { ordered.push(group.clone()); }
    }
    for group in group_views {
        if !ordered.iter().any(|existing| existing.view.name == group.view.name) { ordered.push(group); }
    }
    let groups = ordered.iter().map(|group| group.view.clone()).collect::<Vec<_>>();
    let keys = brush_projection_keys(&groups);
    BrushBrowserProjection { group_views: ordered, groups, keys, containers }
}

pub(crate) fn brush_projection_keys(groups: &[GroupView]) -> PickerProjectionKeys {
    let mut keys = PickerProjectionKeys::default();
    for (group_index, group) in groups.iter().enumerate() {
        keys.insert(
            ProjectionNodeRef::Group(group_index),
            PickerIdentityKey::from_parts("brush-container", [group.name.as_str()]),
            PickerNodeIdentity::Group(group.name.clone()),
        );
        for (item_index, item) in group.items.iter().enumerate() {
            keys.insert(
                ProjectionNodeRef::Item { group_index, item_index },
                PickerIdentityKey::from_parts("brush-item", [item.key.as_str()]),
                PickerNodeIdentity::Item { group: group.name.clone(), key: item.key.clone() },
            );
        }
    }
    keys
}

pub(crate) fn browser_groups(presets: &[BrushPreset], state: &mut PresetUi, panel: &str) -> Vec<GroupView> {
    project_brush_browser(presets, state, panel).groups
}

pub fn group_view_key(label: &str, path: &[String]) -> String {
    if path.is_empty() { root_group_key(label) } else { folder_key(label, path) }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Renaming {
    pub group: bool,
    pub name: String,
    pub folder: Vec<String>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
#[allow(private_interfaces)]
pub enum Action {
    Select(String),
    Choose(String),
    ToggleGroup(String),
    Status(String),
    Rename(Renaming),
    RenameItem { panel: String, name: String, text: String },
    RenameGroup { panel: String, group: String, text: String },
    Delete(String),
    DeleteOccurrence { name: String, group: String, folder: Vec<String> },
    DeleteGroup { panel: String, group: String },
    MoveItem { panel: String, name: String, destination: BrushContainer, before: Option<String> },
    MoveItemToKey { panel: String, name: String, destination: String, before: Option<String> },
    MoveGroup { panel: String, source: BrushContainer, destination: Option<BrushContainer>, before: Option<BrushContainer>, reorder: bool },
    NewPreset { group: String, folder: Vec<String>, index: usize },
    NewAtSelection { panel: String, selection: Option<crate::pickers::state::BrowserSelectionSnapshot> },
    NewNamed { panel: String, name: String, selection: Option<crate::pickers::state::BrowserSelectionSnapshot> },
    Import,
    ExportSelected(Vec<String>),
    CreateGroup { group: String, targets: Vec<crate::pickers::browser::selection::SelectionTarget> },
    CreateGroupInPanel { panel: String, group: String, targets: Vec<crate::pickers::browser::selection::SelectionTarget> },
}

pub fn preview_identity(preset: &BrushPreset) -> String {
    // Main guarantees name uniqueness for name-addressed preset commands. Names are stable across
    // moves, unlike the former address-based preview identity or location-qualified keys.
    preset.name.clone()
}

pub fn preset_by_identity<'a>(presets: &'a [BrushPreset], key: &str) -> Option<&'a BrushPreset> {
    presets.iter().find(|preset| preset.name == key)
}

pub(crate) fn group_label_for_key(_presets: &[BrushPreset], key: &str) -> Option<String> {
    Some(key_label(key))
}

pub fn run_or_status(app: &mut PhotocraftApp, command: &str, params: Value) -> bool {
    match app.run(command, params) {
        Ok(_) => {
            app.ui.status_error = false;
            true
        }
        Err(error) => {
            app.ui.status = error;
            app.ui.status_error = true;
            false
        }
    }
}

fn rename_item(app: &mut PhotocraftApp, name: &str, text: &str) {
    if run_or_status(app, "brush.presets.rename", json!({ "name": name, "newName": text })) {
        app.ui.presets_ui.rename_item_key(name, text);
    }
}

pub(crate) fn container_key_for_preset(projection: &BrushBrowserProjection<'_>, preset: &BrushPreset) -> String {
    container_for_preset(&projection.containers, preset).map(|container| container.key.clone()).unwrap_or_else(|| {
        if preset.group.is_empty() { ROOT_ITEMS_GROUP.into() } else { root_group_key(&preset.group) }
    })
}

pub(crate) fn active_brush_selection(state: &PresetUi, panel: &str) -> Option<crate::pickers::state::BrowserSelectionSnapshot> {
    use crate::pickers::browser::selection::{BrowserSelection, SelectionTarget};

    if let Some(key) = state.selected.get(panel) {
        let group = state.selected_group.get(panel).cloned().unwrap_or_else(|| ROOT_ITEMS_GROUP.into());
        return Some(BrowserSelection { targets: vec![SelectionTarget::Item { group, path: Vec::new(), key: key.clone() }], ..Default::default() });
    }
    let group = state.selected_group.get(panel)?.clone();
    Some(BrowserSelection { targets: vec![SelectionTarget::Group(group)], ..Default::default() })
}

pub(crate) fn new_preset_destination(
    presets: &[BrushPreset],
    state: &mut PresetUi,
    panel: &str,
    selection: Option<&crate::pickers::state::BrowserSelectionSnapshot>,
) -> (String, Vec<String>, usize) {
    use crate::pickers::browser::selection::SelectionTarget;

    let projection = project_brush_browser(presets, state, panel);
    let target = selection.and_then(|selection| selection.targets.last());
    match target {
        Some(SelectionTarget::Item { key, .. }) => {
            if let Some(preset) = preset_by_identity(presets, key) {
                let group = preset.group.clone();
                let folder = preset.folder.clone();
                let index = presets.iter().filter(|candidate| candidate.group == group).take_while(|candidate| candidate.name != preset.name).count() + 1;
                (group, folder, index)
            } else {
                (String::new(), Vec::new(), 0)
            }
        }
        Some(SelectionTarget::Group(key)) | Some(SelectionTarget::Folder { group: key, .. }) => {
            projection.containers.get(key).map(|container| {
                let last = presets.iter().enumerate().filter(|(_, preset)| {
                    preset.group == container.engine_group && preset.folder.starts_with(&container.folder)
                }).map(|(index, _)| index).max();
                let index = last.map(|last| presets[..=last].iter().filter(|preset| preset.group == container.engine_group).count())
                    .unwrap_or_else(|| presets.iter().filter(|preset| preset.group == container.engine_group).count());
                (container.engine_group.clone(), container.folder.clone(), index)
            }).unwrap_or_else(|| (String::new(), Vec::new(), 0))
        }
        None => (String::new(), Vec::new(), presets.iter().filter(|preset| preset.group.is_empty()).count()),
    }
}

pub fn browser_actions_to_actions(
    presets: &[BrushPreset],
    browser_actions: Vec<BrowserAction>,
    selection: Option<&crate::pickers::state::BrowserSelectionSnapshot>,
) -> Vec<Action> {
    browser_actions_to_actions_for_panel(presets, "brushes", browser_actions, selection)
}

pub fn browser_actions_to_actions_for_panel(
    presets: &[BrushPreset],
    panel: &str,
    browser_actions: Vec<BrowserAction>,
    selection: Option<&crate::pickers::state::BrowserSelectionSnapshot>,
) -> Vec<Action> {
    browser_actions
        .into_iter()
        .filter_map(|action| match action {
            BrowserAction::SelectItem { key, .. } => Some(Action::Select(key)),
            BrowserAction::ActivateItem { key, .. } => Some(Action::Choose(key)),
            BrowserAction::Rename(key, text) => Some(Action::RenameItem { panel: panel.to_owned(), name: key, text }),
            BrowserAction::RenameItem { key, text, .. } => Some(Action::RenameItem { panel: panel.to_owned(), name: key, text }),
            BrowserAction::RenameGroup(group, text) => Some(Action::RenameGroup { panel: panel.to_owned(), group, text }),
            BrowserAction::RenameNestedGroup { group, text, .. } => Some(Action::RenameGroup { panel: panel.to_owned(), group, text }),
            BrowserAction::DeleteItem { key, .. } => preset_by_identity(presets, &key).map(|preset| Action::DeleteOccurrence {
                name: preset.name.clone(), group: preset.group.clone(), folder: preset.folder.clone(),
            }),
            BrowserAction::DeleteGroup(group) => Some(Action::DeleteGroup { panel: panel.to_owned(), group }),
            BrowserAction::DeleteNestedGroup { group, .. } => Some(Action::DeleteGroup { panel: panel.to_owned(), group }),
            BrowserAction::Move(key, destination) => Some(Action::MoveItemToKey { panel: panel.to_owned(), name: key, destination, before: None }),
            BrowserAction::New => Some(Action::NewAtSelection { panel: panel.to_owned(), selection: selection.cloned() }),
            BrowserAction::NewNamed(name) => Some(Action::NewNamed { panel: panel.to_owned(), name, selection: selection.cloned() }),
            BrowserAction::CreateGroup(group) => Some(Action::CreateGroupInPanel {
                panel: panel.to_owned(), group, targets: selection.map(|selection| selection.targets.clone()).unwrap_or_default(),
            }),
            BrowserAction::AppendDefault => None,
            BrowserAction::Import => Some(Action::Import),
            BrowserAction::ExportSelected(keys) => Some(Action::ExportSelected(keys)),
            BrowserAction::ExportAll(_) => Some(Action::ExportSelected(presets.iter().map(|preset| preset.name.clone()).collect())),
            BrowserAction::Drop(_, _) | BrowserAction::NewGroup | BrowserAction::CopyItem { .. }
            | BrowserAction::BeginInlineRenameItem(..) | BrowserAction::CancelInlineRenameItem
            | BrowserAction::BeginInlineRenameGroup { .. } | BrowserAction::CancelInlineRenameGroup
            | BrowserAction::BeginRenameSelection { .. } => None,
        })
        .collect()
}

fn set_current_preset(app: &mut PhotocraftApp, name: &str) {
    run_or_status(app, "tools.setBrush", json!({ "preset": name }));
    app.ui.presets_ui.selected.remove("brush-texture-patterns");
}

fn rename_container(app: &mut PhotocraftApp, panel: &str, key: &str, text: &str) {
    let presets = app.session.tools.presets.clone();
    let projection = project_brush_browser(&presets, &mut app.ui.presets_ui, panel);
    let Some(container) = projection.containers.get(key).cloned() else {
        app.ui.status = "The brush group or folder no longer exists.".into();
        app.ui.status_error = true;
        return;
    };
    let new_key = if container.root_group { root_group_key(text) } else { folder_key(&container.engine_group, &container.folder[..container.folder.len().saturating_sub(1)].iter().cloned().chain(std::iter::once(text.to_owned())).collect::<Vec<_>>()) };
    let populated = presets.iter().any(|preset| preset.group == container.engine_group && preset.folder.starts_with(&container.folder));
    if populated {
        let mut params = json!({ "group": container.engine_group, "newName": text });
        if !container.folder.is_empty() { params["folder"] = json!(container.folder); }
        if let Err(error) = app.run("brush.presets.renameGroup", params) {
            app.ui.status = error;
            app.ui.status_error = true;
            return;
        }
    }
    app.ui.presets_ui.rename_group(panel, key, &new_key);
    app.ui.status_error = false;
}

fn delete_container(app: &mut PhotocraftApp, panel: &str, key: &str) {
    let presets = app.session.tools.presets.clone();
    let projection = project_brush_browser(&presets, &mut app.ui.presets_ui, panel);
    let Some(container) = projection.containers.get(key).cloned() else {
        app.ui.status = "The brush group or folder no longer exists.".into();
        app.ui.status_error = true;
        return;
    };
    let populated = presets.iter().any(|preset| preset.group == container.engine_group && preset.folder.starts_with(&container.folder));
    if populated {
        let mut params = json!({ "group": container.engine_group });
        if !container.folder.is_empty() { params["folder"] = json!(container.folder); }
        if let Err(error) = app.run("brush.presets.deleteGroup", params) {
            app.ui.status = error;
            app.ui.status_error = true;
            return;
        }
    }
    app.ui.presets_ui.delete_group(panel, key);
    app.ui.presets_ui.clear_browser_selection(panel);
    app.ui.status_error = false;
}

fn move_item(app: &mut PhotocraftApp, name: &str, target: &BrushContainer, before: Option<&str>) {
    let mut params = json!({ "name": name, "group": target.engine_group, "folder": target.folder });
    if let Some(before) = before { params["before"] = json!(before); }
    run_or_status(app, "brush.presets.move", params);
}

fn move_group(app: &mut PhotocraftApp, source_container: &BrushContainer, target: Option<&BrushContainer>, before: Option<&BrushContainer>, reorder: bool) {
    let presets = app.session.tools.presets.clone();
    let source_presets: Vec<&BrushPreset> = presets.iter().filter(|preset| preset.group == source_container.engine_group && preset.folder.starts_with(&source_container.folder)).collect();
    if source_presets.is_empty() {
        return; // Empty containers are pure layout nodes.
    }

    let destination_has_contents = target.is_none_or(|target| {
        presets.iter().any(|preset| preset.group == target.engine_group && preset.folder.starts_with(&target.folder))
    });
    if !destination_has_contents && let Some(target) = target {
        // Main has no empty-group command. Populate an empty destination with ordinary item moves;
        // a moved root group/folder becomes a named child folder as moveGroup would do.
        let leaf = source_container.folder.last().cloned().unwrap_or_else(|| source_container.label.clone());
        for preset in source_presets {
            let tail = preset.folder.get(source_container.folder.len()..).unwrap_or_default();
            let folder = target.folder.iter().cloned().chain(std::iter::once(leaf.clone())).chain(tail.iter().cloned()).collect::<Vec<_>>();
            run_or_status(app, "brush.presets.move", json!({ "name": preset.name, "group": target.engine_group, "folder": folder }));
            if app.ui.status_error { return; }
        }
        return;
    }

    let mut params = json!({ "group": source_container.engine_group });
    if !source_container.folder.is_empty() { params["folder"] = json!(source_container.folder); }
    if reorder {
        if let Some(before_container) = before {
            params["before"] = json!(if source_container.folder.is_empty() { before_container.engine_group.clone() } else { before_container.label.clone() });
        }
    } else if let Some(target) = target {
        params["into"] = json!({ "group": target.engine_group, "folder": target.folder });
        if let Some(before_container) = before {
            params["before"] = json!(before_container.label);
        }
    } else {
        // Promoting a folder to the root uses `into: {}`. A root group reorder has no `into`.
        params["into"] = json!({});
    }
    run_or_status(app, "brush.presets.moveGroup", params);
}

pub fn apply(app: &mut PhotocraftApp, actions: Vec<Action>) {
    for action in actions {
        match action {
            Action::Select(name) => {
                if app.session.tools.current_preset.as_deref() != Some(name.as_str()) { set_current_preset(app, &name); }
            }
            Action::Choose(name) => {
                if app.session.tools.current_preset.as_deref() != Some(name.as_str()) { set_current_preset(app, &name); }
                if app.ui.brush_picker.is_some() { app.ui.brush_picker = None; }
            }
            Action::ToggleGroup(_) => {}
            Action::Status(message) => { app.ui.status = message; app.ui.status_error = true; }
            Action::Rename(rename) if rename.group => rename_container(app, "brushes", &rename.name, &rename.text),
            Action::Rename(rename) => rename_item(app, &rename.name, &rename.text),
            Action::RenameItem { name, text, .. } => rename_item(app, &name, &text),
            Action::RenameGroup { panel, group, text } => rename_container(app, &panel, &group, &text),
            Action::Delete(name) => {
                if run_or_status(app, "brush.presets.delete", json!({ "name": name })) { app.ui.presets_ui.delete_item_key(&name); }
            }
            Action::DeleteOccurrence { name, .. } => {
                if run_or_status(app, "brush.presets.delete", json!({ "name": name })) { app.ui.presets_ui.delete_item_key(&name); }
            }
            Action::DeleteGroup { panel, group } => delete_container(app, &panel, &group),
            Action::MoveItem { name, destination, before, .. } => move_item(app, &name, &destination, before.as_deref()),
            Action::MoveItemToKey { panel, name, destination, before } => {
                let presets = app.session.tools.presets.clone();
                let projection = project_brush_browser(&presets, &mut app.ui.presets_ui, &panel);
                if let Some(target) = projection.containers.get(&destination).cloned() { move_item(app, &name, &target, before.as_deref()); }
                else { app.ui.status = "The brush destination no longer exists.".into(); app.ui.status_error = true; }
            }
            Action::MoveGroup { source, destination, before, reorder, .. } => move_group(app, &source, destination.as_ref(), before.as_ref(), reorder),
            Action::NewPreset { group, folder, index } => {
                crate::brush_preset_dialog::open_at(app, None, Some((group, folder, index)));
            }
            Action::NewAtSelection { panel, selection } => {
                let destination = new_preset_destination(&app.session.tools.presets, &mut app.ui.presets_ui, &panel, selection.as_ref());
                crate::brush_preset_dialog::open_at_panel(app, &panel, None, Some(destination));
            }
            Action::NewNamed { panel, name, selection } => {
                let destination = new_preset_destination(&app.session.tools.presets, &mut app.ui.presets_ui, &panel, selection.as_ref());
                crate::brush_preset_dialog::open_at_panel_named(app, &panel, None, Some(destination), Some(&name));
            }
            Action::Import => {
                if let Err(error) = crate::preset_files_ui::import_brushes(app)
                    && error != crate::file_dialog::CANCELLED
                {
                    app.ui.status = error;
                    app.ui.status_error = true;
                }
            }
            Action::ExportSelected(keys) => {
                if let Err(error) = crate::brush_panel::export_selected_brushes(app, &keys)
                    && error != crate::file_dialog::CANCELLED
                {
                    app.ui.status = error;
                    app.ui.status_error = true;
                }
            }
            Action::CreateGroup { group, targets } => create_brush_group(app, "brushes", &group, &targets),
            Action::CreateGroupInPanel { panel, group, targets } => create_brush_group(app, &panel, &group, &targets),
        }
    }
}

fn create_brush_group(app: &mut PhotocraftApp, panel: &str, name: &str, targets: &[crate::pickers::browser::selection::SelectionTarget]) {
    let presets = app.session.tools.presets.clone();
    let projection = project_brush_browser(&presets, &mut app.ui.presets_ui, panel);
    let name = name.trim();
    if name.is_empty() || name == UNGROUPED {
        app.ui.status = "Enter a valid brush group name.".into();
        app.ui.status_error = true;
        return;
    }
    let explicit_items = targets.iter().filter_map(|target| match target {
        crate::pickers::browser::selection::SelectionTarget::Item { key, .. } => Some(key.clone()),
        _ => None,
    }).collect::<Vec<_>>();
    let selected_containers = explicit_items.iter().filter_map(|key| preset_by_identity(&presets, key).map(|preset| container_key_for_preset(&projection, preset))).collect::<BTreeSet<_>>();
    let group_target = targets.iter().find_map(|target| match target {
        crate::pickers::browser::selection::SelectionTarget::Group(key) => Some(key.clone()),
        _ => None,
    });
    let parent_key = if selected_containers.len() == 1 {
        selected_containers.iter().next().cloned()
    } else if explicit_items.is_empty() {
        group_target
    } else {
        None
    };
    let parent = parent_key.as_ref().and_then(|key| projection.containers.get(key)).cloned();
    let create_folder = parent.as_ref().is_some_and(|parent| !parent.root_items);
    if create_folder
        && parent.as_ref().is_some_and(|parent| saved_child_key(&app.ui.presets_ui, panel, &parent.key, name).is_some())
    {
        app.ui.status = format!("A folder named `{name}` already exists in this location.");
        app.ui.status_error = true;
        return;
    }
    if !create_folder && (presets.iter().any(|preset| preset.group == name) || projection.containers.values().any(|container| container.root_group && container.label == name)) {
        app.ui.status = format!("A brush group named `{name}` already exists.");
        app.ui.status_error = true;
        return;
    }
    let mut selected_names = explicit_items;
    let mut seen_names = BTreeSet::new();
    selected_names.retain(|name| seen_names.insert(name.clone()));
    // Validate every source before the first command: the main engine has no batch transaction.
    if selected_names.iter().any(|name| preset_by_identity(&presets, name).is_none()) {
        app.ui.status = "A selected brush no longer exists; no brushes were moved.".into();
        app.ui.status_error = true;
        return;
    }
    let group_name = name.to_owned();
    let folder_path = parent.as_ref().map(|parent| parent.folder.iter().cloned().chain(std::iter::once(group_name.clone())).collect::<Vec<_>>()).unwrap_or_default();
    let destination_group = parent.as_ref().filter(|_| create_folder).map(|parent| parent.engine_group.clone()).unwrap_or_else(|| group_name.clone());
    let destination_folder = if create_folder { folder_path.clone() } else { Vec::new() };
    if destination_folder.len() > crate::pickers::browser::hierarchy::MAX_HIERARCHY_DEPTH {
        app.ui.status = format!("Brush folders can be nested at most {} levels deep.", crate::pickers::browser::hierarchy::MAX_HIERARCHY_DEPTH);
        app.ui.status_error = true;
        return;
    }
    for preset_name in &selected_names {
        if let Err(error) = app.run("brush.presets.move", json!({ "name": preset_name, "group": destination_group.clone(), "folder": destination_folder.clone() })) {
            app.ui.status = error;
            app.ui.status_error = true;
            return;
        }
    }
    let key = if create_folder {
        folder_key(&destination_group, &folder_path)
    } else {
        root_group_key(name)
    };
    let parent_sequence = if create_folder { parent.as_ref().map(|parent| parent.key.as_str()).unwrap_or("") } else { "" };
    let mut order = app.ui.presets_ui.layout.hierarchy_order.get(panel).and_then(|orders| orders.get(parent_sequence)).cloned().unwrap_or_default();
    if !order.contains(&HierarchyOrderEntry::Group(key.clone())) { order.push(HierarchyOrderEntry::Group(key.clone())); }
    app.ui.presets_ui.set_hierarchy_order(panel, parent_sequence, order);
    app.ui.presets_ui.set_hierarchy_order(panel, &key, selected_names.iter().cloned().map(HierarchyOrderEntry::Item).collect());
    app.ui.presets_ui.selected.remove(panel);
    app.ui.presets_ui.selected_group.insert(panel.to_owned(), key);
    app.ui.presets_ui.selected_group_path.insert(panel.to_owned(), Vec::new());
    app.ui.presets_ui.clear_browser_selection(panel);
    app.ui.status_error = false;
}

fn group_key_for_id(id: &PickerId, identities: &BTreeMap<PickerId, PickerNodeIdentity>) -> Option<String> {
    match identities.get(id)? {
        PickerNodeIdentity::Group(key) => Some(key.clone()),
        _ => None,
    }
}

fn parent_container_key(
    container: &ContainerId,
    identities: &BTreeMap<PickerId, PickerNodeIdentity>,
) -> Option<String> {
    match container {
        ContainerId::Root => None,
        ContainerId::Group(id) => group_key_for_id(id, identities),
    }
}

/// Translate the standard shared hierarchy intent into the main engine's moveGroup command shape.
/// The returned action captures the pre-drop engine coordinates; `commit_hierarchy_move` owns the
/// layout update and the adapter never reconstructs a second brush hierarchy.
pub(crate) fn brush_group_move_actions(
    panel: &str,
    projection: &BrushBrowserProjection<'_>,
    hierarchy: &PickerBrowserHierarchy,
    identities: &BTreeMap<PickerId, PickerNodeIdentity>,
    intent: &DropMoveIntent,
) -> Result<Vec<Action>, String> {
    let sources = if intent.selection.is_empty() { vec![intent.source.clone()] } else { intent.selection.clone() };
    let selected_source_keys = sources.iter().filter_map(|id| group_key_for_id(id, identities)).collect::<BTreeSet<_>>();
    let (destination_key, before_key, destination_parent) = match &intent.position {
        DropPosition::Before { target } => {
            let parent = hierarchy.parent_of(target).ok_or_else(|| "The brush drop target no longer exists.".to_owned())?;
            let key = parent_container_key(&parent, identities);
            (key, group_key_for_id(target, identities), Some(parent))
        }
        DropPosition::After { target } => {
            let parent = hierarchy.parent_of(target).ok_or_else(|| "The brush drop target no longer exists.".to_owned())?;
            let siblings = hierarchy.children_of(&parent).ok_or_else(|| "The brush drop target's parent no longer exists.".to_owned())?;
            let target_index = siblings.iter().position(|node| node.id() == target).ok_or_else(|| "The brush drop target no longer exists.".to_owned())?;
            let next_group = siblings.iter().skip(target_index + 1).find_map(|node| {
                group_key_for_id(node.id(), identities).filter(|key| !selected_source_keys.contains(key))
            });
            (parent_container_key(&parent, identities), next_group, Some(parent))
        }
        DropPosition::PrependTo { group } | DropPosition::AppendTo { container: ContainerId::Group(group) } => {
            let key = group_key_for_id(group, identities).ok_or_else(|| "The brush destination group no longer exists.".to_owned())?;
            (Some(key), None, Some(ContainerId::Group(group.clone())))
        }
        DropPosition::AppendTo { container: ContainerId::Root } => (None, None, Some(ContainerId::Root)),
    };

    let destination = destination_key.as_ref().and_then(|key| projection.containers.get(key)).cloned();
    let before = before_key.as_ref().and_then(|key| projection.containers.get(key)).cloned();
    let mut actions = Vec::new();
    for source_id in sources {
        let source_key = group_key_for_id(&source_id, identities).ok_or_else(|| "The dragged brush group no longer exists.".to_owned())?;
        if before_key.as_deref() == Some(source_key.as_str()) { continue; }
        let source = projection.containers.get(&source_key).cloned().ok_or_else(|| "The dragged brush group no longer exists.".to_owned())?;
        let source_parent = hierarchy.parent_of(&source_id).ok_or_else(|| "The dragged brush group no longer exists.".to_owned())?;
        let reorder = destination_parent.as_ref().is_some_and(|parent| parent == &source_parent);
        if !reorder {
            let leaf = source.folder.last().cloned().unwrap_or_else(|| source.label.clone());
            if let Some(target) = destination.as_ref() {
                let mut new_folder = target.folder.clone();
                new_folder.push(leaf.clone());
                if projection.containers.values().any(|container| container.engine_group == target.engine_group && container.folder == new_folder && container.key != source.key) {
                    return Err(format!("A brush folder named `{leaf}` already exists in the destination."));
                }
            } else if !source.folder.is_empty() && projection.containers.values().any(|container| container.root_group && container.label == leaf && container.key != source.key) {
                return Err(format!("A brush group named `{leaf}` already exists at the root."));
            }
        }
        actions.push(Action::MoveGroup { panel: panel.to_owned(), source, destination: destination.clone(), before: before.clone(), reorder });
    }
    Ok(actions)
}

/// Resolve one item drop to an ordinary `brush.presets.move` action. `index` is the canonical
/// destination card index; `before` is preferred because main's engine preserves folder-local order.
pub(crate) fn brush_item_move_action(
    panel: &str,
    projection: &BrushBrowserProjection<'_>,
    groups: &[GroupView],
    source_name: &str,
    destination_key: &str,
    index: usize,
) -> Result<Action, String> {
    let destination = projection.containers.get(destination_key).cloned().ok_or_else(|| "The brush destination no longer exists.".to_owned())?;
    let before = groups
        .iter()
        .find(|group| group.name == destination_key)
        .map(|group| group.items.iter().filter(|item| item.key != source_name).map(|item| item.key.clone()).collect::<Vec<_>>())
        .and_then(|items| items.get(index).cloned());
    Ok(Action::MoveItem { panel: panel.to_owned(), name: source_name.to_owned(), destination, before })
}

/// Expand a new-preset destination and its ancestors in the shared saved layout.
pub(crate) fn expand_brush_destination(app: &mut PhotocraftApp, panel: &str, group: &str, folder: &[String]) {
    let presets = app.session.tools.presets.clone();
    let projection = project_brush_browser(&presets, &mut app.ui.presets_ui, panel);
    for container in projection.containers.values().filter(|container| container.engine_group == group && container.folder.len() <= folder.len() && folder.starts_with(&container.folder)) {
        app.ui.presets_ui.expanded_groups.insert(format!("{panel}/{}", crate::pickers::state::escape_hierarchy_key_part(&container.key)));
    }
}
