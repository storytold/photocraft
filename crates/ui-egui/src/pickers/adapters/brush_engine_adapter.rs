//! Brush-specific projection and translation between the flat engine brush library and the shared asset browser.

use std::collections::BTreeMap;

use photocraft_engine::{BrushPresetHierarchy, paint::BrushPreset};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::pickers::browser::asset_browser::{BrowserAction, GroupView, ItemView};
use crate::pickers::browser::dragging_model::DropMoveIntent;
use crate::pickers::browser::identity::{PickerIdentityKey, PickerNodeIdentity, PickerProjectionKeys, ProjectionNodeRef};
use crate::pickers::browser::path_identity_model::{
    ContainerId, DropPosition, HierarchyNode as PickerHierarchyNode, PickerBrowserHierarchy, PickerId, PickerIdRegistry,
};
use crate::pickers::state::HierarchyOrderEntry;

type BrushDropDestination = (String, Vec<String>, Option<(&'static str, String)>);

/// Reconcile Brush item and folder tokens after a successful hierarchy move. Both identities are
/// qualified by the owning group/path; the stable preset name survives the move and lets us rekey
/// before the authoritative preset vector is updated.
pub(super) fn brush_item_identity_key(name: &str, group: &str, path: &[String]) -> PickerIdentityKey {
    PickerIdentityKey::from_parts("brush-item", std::iter::once(name).chain(std::iter::once(group)).chain(path.iter().map(String::as_str)))
}

pub(super) fn rekey_brush_hierarchy(
    hierarchy: &PickerBrowserHierarchy,
    projection: &BTreeMap<PickerId, PickerNodeIdentity>,
    presets: &[BrushPreset],
    registry: &mut PickerIdRegistry,
) {
    fn visit(
        nodes: &[PickerHierarchyNode],
        parent_group: Option<&str>,
        parent_folder: Option<&[String]>,
        identities: &BTreeMap<PickerId, PickerNodeIdentity>,
        presets: &[BrushPreset],
        registry: &mut PickerIdRegistry,
    ) {
        for node in nodes {
            let Some(identity) = identities.get(node.id()) else { continue };
            match identity {
                PickerNodeIdentity::Group(group) => {
                    if let Some(owner) = parent_group {
                        // A root brush group dropped into another group becomes a folder in the
                        // engine's preset model. Keep the dragged node's runtime ID by rekeying
                        // its old root identity to the new owner/path before the next projection.
                        let mut next_path = parent_folder.map_or_else(Vec::new, ToOwned::to_owned);
                        next_path.push(group.clone());
                        registry.rekey(
                            PickerIdentityKey::from_parts("brush-group", [group.as_str()]),
                            PickerIdentityKey::from_parts("brush-folder", std::iter::once(owner).chain(next_path.iter().map(String::as_str))),
                            node.id(),
                        );
                        visit(node.children(), Some(owner), Some(&next_path), identities, presets, registry);
                    } else {
                        visit(node.children(), Some(group), None, identities, presets, registry);
                    }
                }
                PickerNodeIdentity::Folder { group, path } => {
                    let owner = parent_group.unwrap_or(group);
                    let Some(leaf) = path.last() else { continue };
                    let mut next_path = parent_folder.map_or_else(Vec::new, ToOwned::to_owned);
                    next_path.push(leaf.clone());
                    if owner != group || next_path != *path {
                        registry.rekey(
                            PickerIdentityKey::from_parts("brush-folder", std::iter::once(group.as_str()).chain(path.iter().map(String::as_str))),
                            PickerIdentityKey::from_parts("brush-folder", std::iter::once(owner).chain(next_path.iter().map(String::as_str))),
                            node.id(),
                        );
                    }
                    visit(node.children(), Some(owner), Some(&next_path), identities, presets, registry);
                }
                PickerNodeIdentity::Item { group, key } | PickerNodeIdentity::FolderItem { group, path: _, key } => {
                    let Some(preset) = preset_by_identity(presets, key) else { continue };
                    let old_path = match identity {
                        PickerNodeIdentity::FolderItem { path, .. } => path.as_slice(),
                        _ => &[],
                    };
                    // A card dropped at the hierarchy root becomes an ungrouped Brush.
                    let owner = parent_group.unwrap_or("");
                    let next_path = parent_folder.unwrap_or_default();
                    if owner != group || next_path != old_path {
                        registry.rekey(
                            brush_item_identity_key(&preset.name, group, old_path),
                            brush_item_identity_key(&preset.name, owner, next_path),
                            node.id(),
                        );
                    }
                }
            }
        }
    }

    visit(hierarchy.roots(), None, None, projection, presets, registry);
}

/// Run a brush command, reporting failures in the status bar and returning whether it succeeded.
pub fn run_or_status(app: &mut PhotocraftApp, id: &str, p: Value) -> bool {
    match app.run(id, p) {
        Ok(_) => true,
        Err(e) => {
            app.ui.status = e;
            false
        }
    }
}

pub const UNGROUPED: &str = "My Brushes";

/// Preset indices in panel order: groups in order of first appearance.
pub fn grouped_presets(presets: &[BrushPreset]) -> Vec<(String, Vec<usize>)> {
    let mut out: Vec<(String, Vec<usize>)> = Vec::new();
    for (i, p) in presets.iter().enumerate() {
        let g = if p.group.is_empty() { UNGROUPED } else { p.group.as_str() };
        match out.iter_mut().find(|(n, _)| n == g) {
            Some((_, v)) => v.push(i),
            None => out.push((g.to_string(), vec![i])),
        }
    }
    out
}

/// A rename in progress in the Brushes tab (a preset, or a group when `group` is set).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Renaming {
    pub group: bool,
    /// The preset (or group) being renamed.
    pub name: String,
    /// With `group`: the nested path beneath `name` (empty = the root group itself).
    pub folder: Vec<String>,
    /// The text typed so far.
    pub text: String,
}

/// Commands produced by the brush-library adapter and applied to the existing engine API.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// A click: make the preset current.
    Select(String),
    /// A double-click: make the preset current and dismiss the picker showing it (Photoshop).
    Choose(String),
    /// View only: open or close a group (by its [`group_view_key`]; [`preset_list`]
    /// applies it to its view state).
    ToggleGroup(String),
    /// Report an operation that cannot be represented safely by the name-based engine API.
    Status(String),
    Move {
        name: String,
        group: String,
        /// The nested path in `group` (`None`: the command's default).
        folder: Option<Vec<String>>,
        index: Option<usize>,
        /// Mixed root sibling position when moving a preset to the top level.
        root_index: Option<usize>,
    },
    /// Move a preset before a sibling in a specific nested group.
    MoveBefore {
        name: String,
        group: String,
        folder: Option<Vec<String>>,
        before: String,
    },
    /// Move a group (`folder` empty) among root groups, or a nested group among its siblings;
    /// `before` is a root group or sibling group's name.
    MoveGroup {
        group: String,
        folder: Vec<String>,
        before: Option<String>,
        /// Mixed root sibling position when reordering a root group.
        root_index: Option<usize>,
    },
    /// Move a group (`folder` empty) or nested group into another group: `into` is (root group,
    /// group path, `[]` = directly in the root group), or `None` for the top level. `before` is a
    /// sibling group (or root group) there to land before.
    MoveInto {
        group: String,
        folder: Vec<String>,
        into: Option<(String, Vec<String>)>,
        before: Option<String>,
        before_item: Option<String>,
        /// Mixed root sibling position when moving a group/folder to the top level.
        root_index: Option<usize>,
    },
    /// View only: show the rename bar for a preset or group ([`preset_list`] applies it).
    BeginRename(Renaming),
    /// Commit a rename typed in the rename bar.
    Rename(Renaming),
    Delete(String),
    /// Delete one exact preset occurrence, qualified by its owning group and folder path.
    DeleteOccurrence {
        name: String,
        group: String,
        folder: Vec<String>,
    },
    /// Delete a root group (`folder` empty) or one of its nested groups, with their presets.
    DeleteGroup {
        group: String,
        folder: Vec<String>,
    },
    /// Open the existing brush-preset creation dialog.
    NewPreset,
    /// Create a group by moving the explicitly selected presets into it.
    CreateGroup {
        group: String,
        presets: Vec<String>,
    },
}

/// Stable collapse key for a root brush group or nested group path.
pub fn group_view_key(label: &str, group_path: &[String]) -> String {
    let label = crate::pickers::state::escape_hierarchy_key_part(label);
    if group_path.is_empty() {
        label
    } else {
        format!("{label}/{}", group_path.iter().map(|part| crate::pickers::state::escape_hierarchy_key_part(part)).collect::<Vec<_>>().join("/"))
    }
}

/// Legacy Brushes-tab API name; the path addresses nested groups.
pub fn folder_view_key(label: &str, group_path: &[String]) -> String {
    group_view_key(label, group_path)
}

/// Remap collapse state when a group path changes.
pub(crate) fn remap_collapsed_prefix(collapsed: &mut [String], from: &str, to: &str) {
    for key in collapsed {
        if key == from {
            *key = to.to_string();
        } else if key.strip_prefix(from).is_some_and(|tail| tail.starts_with('/')) {
            key.replace_range(..from.len(), to);
        }
    }
}

/// Remove collapse state for a deleted group and all descendants.
pub(crate) fn remove_collapsed_prefix(collapsed: &mut Vec<String>, prefix: &str) {
    collapsed.retain(|key| key != prefix && !key.strip_prefix(prefix).is_some_and(|tail| tail.starts_with('/')));
}

/// Project the flat preset vector into browser groups while retaining the exact source preset
/// for each card. The UI uses this association only for brush-specific painting.
pub(super) struct BrushGroupView<'a> {
    pub(super) view: GroupView,
    pub(super) presets: Vec<&'a BrushPreset>,
}

pub(super) fn brush_group_views<'a>(presets: &'a [BrushPreset], hierarchy: &photocraft_engine::BrushPresetHierarchy) -> Vec<BrushGroupView<'a>> {
    let mut groups: Vec<BrushGroupView<'_>> = Vec::new();
    for preset in presets {
        let group = match groups.iter_mut().find(|group| group.view.name == preset.group) {
            Some(group) => group,
            None => {
                groups.push(BrushGroupView {
                    view: GroupView { name: preset.group.clone(), root_only: preset.group.is_empty(), items: Vec::new() },
                    presets: Vec::new(),
                });
                groups.last_mut().expect("just pushed")
            }
        };
        group.view.items.push(ItemView { key: preview_identity(preset), name: preset.name.clone(), group_path: preset.folder.clone() });
        group.presets.push(preset);
    }
    for name in &hierarchy.groups {
        if !groups.iter().any(|group| group.view.name == *name) {
            groups.push(BrushGroupView { view: GroupView { name: name.clone(), root_only: false, items: Vec::new() }, presets: Vec::new() });
        }
    }
    groups
}

pub(crate) fn browser_groups(presets: &[BrushPreset], hierarchy: &photocraft_engine::BrushPresetHierarchy) -> Vec<GroupView> {
    brush_group_views(presets, hierarchy).into_iter().map(|group| group.view).collect()
}

fn insert_brush_folder_keys(keys: &mut PickerProjectionKeys, group_index: usize, group: &GroupView, hierarchy: &photocraft_engine::BrushPresetHierarchy) {
    for folder in hierarchy.folders.iter().filter(|folder| folder.group == group.name) {
        if folder.path.is_empty() || folder.path.len() > crate::pickers::browser::hierarchy::MAX_HIERARCHY_DEPTH {
            continue;
        }
        keys.insert(
            ProjectionNodeRef::Folder { group_index, path: folder.path.clone() },
            PickerIdentityKey::from_parts("brush-folder", std::iter::once(group.name.as_str()).chain(folder.path.iter().map(String::as_str))),
            PickerNodeIdentity::Folder { group: group.name.clone(), path: folder.path.clone() },
        );
        let before_key =
            folder.before_item.as_deref().and_then(|name| group.items.iter().find(|item| item.name.eq_ignore_ascii_case(name)).map(|item| item.key.clone()));
        keys.set_folder_anchor(group_index, folder.path.clone(), before_key);
    }
}

pub fn group_key(presets: &[BrushPreset], label: &str) -> String {
    if label == UNGROUPED && !presets.iter().any(|p| p.group == UNGROUPED) { String::new() } else { label.to_string() }
}

pub(super) fn brush_projection_keys(groups: &[GroupView], hierarchy: &photocraft_engine::BrushPresetHierarchy) -> PickerProjectionKeys {
    let mut keys = PickerProjectionKeys::default();
    for (group_index, group) in groups.iter().enumerate() {
        keys.insert(
            ProjectionNodeRef::Group(group_index),
            PickerIdentityKey::from_parts("brush-group", [group.name.as_str()]),
            PickerNodeIdentity::Group(group.name.clone()),
        );
        let populated_paths: Vec<Vec<String>> = group
            .items
            .iter()
            .flat_map(|item| {
                (1..=item.group_path.len().min(crate::pickers::browser::hierarchy::MAX_HIERARCHY_DEPTH)).map(|depth| item.group_path[..depth].to_vec())
            })
            .fold(Vec::new(), |mut paths, path| {
                if !paths.contains(&path) {
                    paths.push(path);
                }
                paths
            });
        let hierarchy_is_complete =
            populated_paths.iter().all(|path| hierarchy.folders.iter().any(|folder| folder.group == group.name && folder.path == *path));
        if hierarchy_is_complete {
            insert_brush_folder_keys(&mut keys, group_index, group, hierarchy);
        }
        for (item_index, item) in group.items.iter().enumerate() {
            let folder_path = &item.group_path[..item.group_path.len().min(crate::pickers::browser::hierarchy::MAX_HIERARCHY_DEPTH)];
            let reverse = if folder_path.is_empty() {
                PickerNodeIdentity::Item { group: group.name.clone(), key: item.key.clone() }
            } else {
                PickerNodeIdentity::FolderItem { group: group.name.clone(), path: folder_path.to_vec(), key: item.key.clone() }
            };
            keys.insert(ProjectionNodeRef::Item { group_index, item_index }, brush_item_identity_key(&item.name, &group.name, folder_path), reverse);
            for depth in 1..=folder_path.len() {
                keys.insert(
                    ProjectionNodeRef::Folder { group_index, path: item.group_path[..depth].to_vec() },
                    PickerIdentityKey::from_parts(
                        "brush-folder",
                        std::iter::once(group.name.as_str()).chain(item.group_path[..depth].iter().map(String::as_str)),
                    ),
                    PickerNodeIdentity::Folder { group: group.name.clone(), path: item.group_path[..depth].to_vec() },
                );
            }
        }
        if !hierarchy_is_complete {
            insert_brush_folder_keys(&mut keys, group_index, group, hierarchy);
        }
    }
    keys
}

/// Persisted mixed root order follows the engine's single preset vector: each ungrouped preset is
/// a root card, while each named group occupies the position of its first preset.
pub(super) fn brush_root_sequence(presets: &[BrushPreset], hierarchy: &photocraft_engine::BrushPresetHierarchy) -> Vec<HierarchyOrderEntry> {
    let mut seen_groups = std::collections::BTreeSet::new();
    let mut derived = Vec::new();
    for preset in presets {
        if preset.group.is_empty() && preset.folder.is_empty() {
            derived.push(HierarchyOrderEntry::RootItem { group: String::new(), key: preview_identity(preset) });
        } else if seen_groups.insert(preset.group.clone()) {
            derived.push(HierarchyOrderEntry::Group(preset.group.clone()));
        }
    }
    for group in &hierarchy.groups {
        if seen_groups.insert(group.clone()) {
            derived.push(HierarchyOrderEntry::Group(group.clone()));
        }
    }
    let mut sequence = Vec::new();
    for entry in &hierarchy.root_order {
        let projected = match entry {
            photocraft_engine::BrushPresetRootEntry::Group(group) => hierarchy.groups.contains(group) || presets.iter().any(|preset| preset.group == *group),
            photocraft_engine::BrushPresetRootEntry::Preset(name) => {
                presets.iter().any(|preset| preset.group.is_empty() && preset.folder.is_empty() && preset.name == *name)
            }
        };
        if !projected {
            continue;
        }
        let entry = match entry {
            photocraft_engine::BrushPresetRootEntry::Group(group) => HierarchyOrderEntry::Group(group.clone()),
            photocraft_engine::BrushPresetRootEntry::Preset(name) => {
                let Some(preset) = presets.iter().find(|preset| preset.group.is_empty() && preset.folder.is_empty() && preset.name == *name) else { continue };
                HierarchyOrderEntry::RootItem { group: String::new(), key: preview_identity(preset) }
            }
        };
        if !sequence.contains(&entry) {
            sequence.push(entry);
        }
    }
    for entry in derived {
        if !sequence.contains(&entry) {
            sequence.push(entry);
        }
    }
    sequence
}

/// Reverse identities for every canonical node projected from the Brush library.
#[derive(Clone, Debug, PartialEq, Eq)]
enum BrushSiblingAnchor {
    Folder(String),
    Item { key: String, index: usize },
}

/// Find the first mixed sibling after a folder's entire subtree. Folder order and direct-item
/// order share the preset sequence, so looking only for the next folder skips valid item anchors.
fn brush_next_sibling_after_folder(
    presets: &[BrushPreset],
    group: &str,
    parent: &[String],
    after: &str,
    exclude_folder: Option<&[String]>,
) -> Option<BrushSiblingAnchor> {
    let mut target_path = parent.to_vec();
    target_path.push(after.to_owned());
    let target_end = presets.iter().rposition(|preset| preset.group == group && preset.folder.starts_with(&target_path))?;
    presets.iter().enumerate().skip(target_end + 1).find_map(|(index, preset)| {
        if preset.group != group || exclude_folder.is_some_and(|path| preset.folder.starts_with(path)) {
            return None;
        }
        if preset.folder == parent {
            Some(BrushSiblingAnchor::Item { key: preview_identity(preset), index })
        } else if preset.folder.len() > parent.len() && preset.folder.starts_with(parent) {
            Some(BrushSiblingAnchor::Folder(preset.folder[parent.len()].clone()))
        } else {
            None
        }
    })
}

fn empty_folder_before_item(
    hierarchy: &BrushPresetHierarchy,
    group: &str,
    parent: &[String],
    before_item: Option<&str>,
    exclude_folder: Option<&[String]>,
) -> Option<String> {
    hierarchy
        .folders
        .iter()
        .find(|folder| {
            folder.group == group
                && folder.path.len() == parent.len() + 1
                && folder.path.starts_with(parent)
                && !exclude_folder.is_some_and(|path| folder.path.starts_with(path))
                && folder.before_item.as_deref() == before_item
        })
        .and_then(|folder| folder.path.last().cloned())
}

fn brush_item_index(presets: &[BrushPreset], group: &str, before_index: usize, source_key: &str) -> usize {
    presets[..before_index].iter().filter(|preset| preset.group == group && preview_identity(preset) != source_key).count()
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RootSiblingAnchor {
    Group(String),
    Item(String),
}

fn root_sibling_after_item(presets: &[BrushPreset], target_key: &str, skip_groups: &[&str]) -> Option<RootSiblingAnchor> {
    let target_index = presets.iter().position(|preset| preview_identity(preset) == target_key)?;
    let target_group = presets[target_index].group.as_str();
    let mut seen = std::collections::BTreeSet::new();
    for preset in presets.iter().skip(target_index + 1) {
        if (!target_group.is_empty() && preset.group == target_group) || skip_groups.contains(&preset.group.as_str()) {
            continue;
        }
        if preset.group.is_empty() {
            if preset.folder.is_empty() {
                return Some(RootSiblingAnchor::Item(preset.name.clone()));
            }
        } else if seen.insert(preset.group.as_str()) {
            return Some(RootSiblingAnchor::Group(preset.group.clone()));
        }
    }
    None
}

fn root_item_index_at_boundary(presets: &[BrushPreset], boundary: usize, source_key: &str) -> usize {
    presets.iter().take(boundary).filter(|preset| preset.group.is_empty() && preset.folder.is_empty() && preview_identity(preset) != source_key).count()
}

fn root_entry_for_identity(identity: &PickerNodeIdentity) -> Option<HierarchyOrderEntry> {
    match identity {
        PickerNodeIdentity::Group(group) => Some(HierarchyOrderEntry::Group(group.clone())),
        PickerNodeIdentity::Item { group, key } if group.is_empty() => Some(HierarchyOrderEntry::RootItem { group: String::new(), key: key.clone() }),
        _ => None,
    }
}

fn root_sibling_index(
    root_sequence: &[HierarchyOrderEntry],
    source: &PickerId,
    target: Option<&PickerId>,
    after: bool,
    identities: &BTreeMap<PickerId, PickerNodeIdentity>,
) -> Option<usize> {
    let source_entry = identities.get(source).and_then(root_entry_for_identity);
    let source_index = source_entry.as_ref().and_then(|entry| root_sequence.iter().position(|candidate| candidate == entry));
    let insertion = if let Some(target) = target {
        let target_entry = identities.get(target).and_then(root_entry_for_identity)?;
        root_sequence.iter().position(|candidate| candidate == &target_entry)? + usize::from(after)
    } else {
        root_sequence.len()
    };
    Some(insertion.saturating_sub(usize::from(source_index.is_some_and(|index| index < insertion))))
}

fn root_card_index(root_sequence: &[HierarchyOrderEntry], root_index: usize, source: &PickerId, identities: &BTreeMap<PickerId, PickerNodeIdentity>) -> usize {
    let source_entry = identities.get(source).and_then(root_entry_for_identity);
    root_sequence
        .iter()
        .filter(|entry| source_entry.as_ref() != Some(*entry))
        .take(root_index)
        .filter(|entry| matches!(entry, HierarchyOrderEntry::RootItem { .. }))
        .count()
}

/// Translate one validated canonical move into an existing authoritative Brush command action.
/// Moves that the name/path command API cannot express exactly are rejected rather than persisted
/// into the generic picker hierarchy.
pub(super) fn brush_drop_intent_to_action(
    presets: &[BrushPreset],
    hierarchy: &BrushPresetHierarchy,
    identities: &BTreeMap<PickerId, PickerNodeIdentity>,
    root_sequence: &[HierarchyOrderEntry],
    intent: &DropMoveIntent,
) -> Result<Action, String> {
    let source = identities.get(&intent.source).ok_or_else(|| "The dragged Brush no longer exists.".to_owned())?;
    let source_item = match source {
        PickerNodeIdentity::Item { key, .. } | PickerNodeIdentity::FolderItem { key, .. } => Some(key.as_str()),
        _ => None,
    };
    let source_group = match source {
        PickerNodeIdentity::Item { group, .. }
        | PickerNodeIdentity::FolderItem { group, .. }
        | PickerNodeIdentity::Group(group)
        | PickerNodeIdentity::Folder { group, .. } => group,
    };
    let source_folder = match source {
        PickerNodeIdentity::FolderItem { path, .. } | PickerNodeIdentity::Folder { path, .. } => path.clone(),
        _ => Vec::new(),
    };
    let source_name = source_item.map(|key| occurrence_name(presets, key).ok_or_else(|| "The dragged Brush name is ambiguous.".to_owned())).transpose()?;
    if source_name.as_deref().is_some_and(|name| is_ambiguous_preset_name(presets, name)) {
        return Err("The dragged Brush name is ambiguous in the library.".to_owned());
    }
    let source_group_name = group_key(presets, source_group);
    if source_item.is_none() && source_folder.is_empty() && source_group_name.is_empty() {
        return Err("The ungrouped Brush collection cannot be moved as a group.".to_owned());
    }

    let destination = |id: &PickerId| -> Result<BrushDropDestination, String> {
        match identities.get(id) {
            Some(PickerNodeIdentity::Item { group, key }) => Ok((group.clone(), Vec::new(), Some(("item", key.clone())))),
            Some(PickerNodeIdentity::FolderItem { group, path, key }) => Ok((group.clone(), path.clone(), Some(("item", key.clone())))),
            Some(PickerNodeIdentity::Group(group)) => Ok((group.clone(), Vec::new(), Some(("group", group.clone())))),
            Some(PickerNodeIdentity::Folder { group, path }) => {
                Ok((group.clone(), path[..path.len().saturating_sub(1)].to_vec(), Some(("folder", path.last().cloned().unwrap_or_default()))))
            }
            None => Err("The Brush drop target no longer exists.".to_owned()),
        }
    };

    let root_drop = match &intent.position {
        DropPosition::AppendTo { container: ContainerId::Root } => root_sibling_index(root_sequence, &intent.source, None, true, identities),
        DropPosition::Before { target } => root_sibling_index(root_sequence, &intent.source, Some(target), false, identities),
        DropPosition::After { target } => root_sibling_index(root_sequence, &intent.source, Some(target), true, identities),
        _ => None,
    };
    if let Some(root_index) = root_drop {
        return match source {
            PickerNodeIdentity::Item { .. } | PickerNodeIdentity::FolderItem { .. } => {
                let name = source_name.ok_or_else(|| "The dragged Brush name is ambiguous.".to_owned())?;
                Ok(Action::Move {
                    name,
                    group: String::new(),
                    folder: Some(Vec::new()),
                    index: Some(root_card_index(root_sequence, root_index, &intent.source, identities)),
                    root_index: Some(root_index),
                })
            }
            PickerNodeIdentity::Group(_) if source_folder.is_empty() => {
                Ok(Action::MoveGroup { group: source_group_name, folder: Vec::new(), before: None, root_index: Some(root_index) })
            }
            PickerNodeIdentity::Group(_) | PickerNodeIdentity::Folder { .. } => Ok(Action::MoveInto {
                group: source_group_name,
                folder: source_folder,
                into: None,
                before: None,
                before_item: None,
                root_index: Some(root_index),
            }),
        };
    }

    match &intent.position {
        DropPosition::PrependTo { group: container } => {
            let (group, path) = match identities.get(container) {
                Some(PickerNodeIdentity::Group(group)) => (group.clone(), Vec::new()),
                Some(PickerNodeIdentity::Folder { group, path }) => (group.clone(), path.clone()),
                _ => return Err("The Brush drop target is not a folder.".to_owned()),
            };
            let group_name = group_key(presets, &group);
            if let Some(name) = source_name {
                let first = presets.iter().enumerate().find(|(_, preset)| {
                    preset.group == group
                        && preset.folder.starts_with(&path)
                        && !(source_item.is_none() && source_group == &group && !source_folder.is_empty() && preset.folder.starts_with(&source_folder))
                        && !(source_group == &group && source_item == Some(preview_identity(preset).as_str()))
                });
                let index = first
                    .map(|(index, _)| brush_item_index(presets, &group, index, source_item.unwrap_or_default()))
                    .or_else(|| {
                        presets
                            .iter()
                            .position(|preset| preview_identity(preset) == source_item.unwrap_or_default())
                            .map(|index| brush_item_index(presets, &group, index, source_item.unwrap_or_default()))
                    })
                    .unwrap_or_else(|| presets.iter().filter(|preset| preset.group == group).count());
                Ok(Action::Move { name, group: group_name, folder: Some(path), index: Some(index), root_index: None })
            } else {
                let first = presets.iter().find(|preset| {
                    preset.group == group
                        && preset.folder.starts_with(&path)
                        && !((source_group == &group && !source_folder.is_empty()) && preset.folder.starts_with(&source_folder))
                });
                let (before, before_item) = first.map_or((None, None), |preset| {
                    if preset.folder == path { (None, Some(preset.name.clone())) } else { (preset.folder.get(path.len()).cloned(), None) }
                });
                Ok(Action::MoveInto { group: source_group_name, folder: source_folder, into: Some((group_name, path)), before, before_item, root_index: None })
            }
        }
        DropPosition::AppendTo { container: ContainerId::Root } => {
            if source_item.is_some() {
                let index = presets.iter().filter(|preset| preset.group.is_empty() && preview_identity(preset) != source_item.unwrap_or_default()).count();
                return Ok(Action::Move {
                    name: source_name.unwrap_or_default(),
                    group: String::new(),
                    folder: Some(Vec::new()),
                    index: Some(index),
                    root_index: None,
                });
            }
            if source_folder.is_empty() {
                Ok(Action::MoveGroup { group: source_group_name, folder: Vec::new(), before: None, root_index: None })
            } else {
                Ok(Action::MoveInto { group: source_group_name, folder: source_folder, into: None, before: None, before_item: None, root_index: None })
            }
        }
        DropPosition::AppendTo { container: ContainerId::Group(container) } => {
            let (group, path) = match identities.get(container) {
                Some(PickerNodeIdentity::Group(group)) => (group.clone(), Vec::new()),
                Some(PickerNodeIdentity::Folder { group, path }) => (group.clone(), path.clone()),
                _ => return Err("The Brush drop target is not a folder.".to_owned()),
            };
            let group = group_key(presets, &group);
            if let Some(name) = source_name {
                Ok(Action::Move { name, group, folder: Some(path), index: None, root_index: None })
            } else {
                Ok(Action::MoveInto {
                    group: source_group_name,
                    folder: source_folder,
                    into: Some((group, path)),
                    before: None,
                    before_item: None,
                    root_index: None,
                })
            }
        }
        DropPosition::Before { target } | DropPosition::After { target } => {
            let after = matches!(&intent.position, DropPosition::After { .. });
            let (target_group, target_parent, target_kind) = destination(target)?;
            let target_group_name = group_key(presets, &target_group);
            if let Some(name) = source_name {
                match target_kind.as_ref().map(|(kind, _)| *kind) {
                    Some("item") => {
                        let Some((_, key)) = target_kind.as_ref() else {
                            return Err("The Brush drop target is not an item.".to_owned());
                        };
                        let source_key = source_item.unwrap_or_default();
                        if !after {
                            let before = resolve_before_item_name(presets, key)?;
                            Ok(Action::MoveBefore { name, group: target_group_name, folder: Some(target_parent), before })
                        } else {
                            let target_index = presets
                                .iter()
                                .position(|preset| preview_identity(preset) == key.as_str())
                                .ok_or_else(|| "The target Brush no longer exists.".to_owned())?;
                            let index = brush_item_index(presets, &target_group, target_index + 1, source_key);
                            Ok(Action::Move { name, group: target_group_name, folder: Some(target_parent), index: Some(index), root_index: None })
                        }
                    }
                    Some("folder") => {
                        let Some((_, leaf)) = target_kind.as_ref() else {
                            return Err("The Brush drop target is not a folder.".to_owned());
                        };
                        let before_index = if after {
                            match brush_next_sibling_after_folder(presets, &target_group, &target_parent, leaf, None) {
                                Some(BrushSiblingAnchor::Folder(next)) => {
                                    let mut path = target_parent.clone();
                                    path.push(next);
                                    presets.iter().position(|preset| preset.group == target_group && preset.folder.starts_with(&path))
                                }
                                Some(BrushSiblingAnchor::Item { index, .. }) => Some(index),
                                None => None,
                            }
                        } else {
                            let mut path = target_parent.clone();
                            path.push(leaf.clone());
                            presets.iter().position(|preset| preset.group == target_group && preset.folder.starts_with(&path))
                        };
                        let index = before_index.map(|index| brush_item_index(presets, &target_group, index, source_item.unwrap_or_default()));
                        Ok(Action::Move { name, group: target_group_name, folder: Some(target_parent), index, root_index: None })
                    }
                    Some("group") => {
                        let target_index = presets
                            .iter()
                            .position(|preset| preset.group == target_group)
                            .ok_or_else(|| "The target Brush group no longer exists.".to_owned())?;
                        let boundary = if after {
                            presets.iter().rposition(|preset| preset.group == target_group).map_or(target_index, |index| index + 1)
                        } else {
                            target_index
                        };
                        let index = root_item_index_at_boundary(presets, boundary, source_item.unwrap_or_default());
                        Ok(Action::Move { name, group: String::new(), folder: Some(Vec::new()), index: Some(index), root_index: None })
                    }
                    _ => Err("A Brush preset cannot be inserted at the top-level group boundary.".to_owned()),
                }
            } else {
                let move_group =
                    |before: Option<String>| Action::MoveGroup { group: source_group_name.clone(), folder: source_folder.clone(), before, root_index: None };
                let move_into = |into: Option<(String, Vec<String>)>, before: Option<String>, before_item: Option<String>| Action::MoveInto {
                    group: source_group_name.clone(),
                    folder: source_folder.clone(),
                    into,
                    before,
                    before_item,
                    root_index: None,
                };
                match target_kind.as_ref().map(|(kind, _)| *kind) {
                    Some("group") if source_folder.is_empty() => {
                        let before = if after {
                            let labels: Vec<String> = grouped_presets(presets).into_iter().map(|(label, _)| label).collect();
                            labels.iter().position(|label| group_key(presets, label) == target_group_name).and_then(|index| {
                                labels.iter().skip(index + 1).map(|label| group_key(presets, label)).find(|label| label != &source_group_name)
                            })
                        } else {
                            Some(target_group_name)
                        };
                        Ok(move_group(before))
                    }
                    Some("group") => {
                        let before = if after {
                            let labels: Vec<String> = grouped_presets(presets).into_iter().map(|(label, _)| label).collect();
                            labels
                                .iter()
                                .position(|label| group_key(presets, label) == target_group_name)
                                .and_then(|index| labels.get(index + 1))
                                .cloned()
                                .map(|label| group_key(presets, &label))
                        } else {
                            Some(target_group_name)
                        };
                        Ok(move_into(None, before, None))
                    }
                    Some("folder") => {
                        let Some((_, leaf)) = target_kind.as_ref() else {
                            return Err("The Brush drop target is not a folder.".to_owned());
                        };
                        let (before, before_item) = if after {
                            match brush_next_sibling_after_folder(
                                presets,
                                &target_group,
                                &target_parent,
                                leaf,
                                (!source_folder.is_empty() && source_group == &target_group).then_some(source_folder.as_slice()),
                            ) {
                                Some(BrushSiblingAnchor::Folder(next)) => (Some(next), None),
                                Some(BrushSiblingAnchor::Item { key, .. }) => (None, Some(resolve_before_item_name(presets, &key)?)),
                                None => (None, None),
                            }
                        } else {
                            (Some(leaf.clone()), None)
                        };
                        Ok(move_into(Some((target_group_name, target_parent)), before, before_item))
                    }
                    Some("item") => {
                        let Some((_, key)) = target_kind.as_ref() else {
                            return Err("The Brush drop target is not an item.".to_owned());
                        };
                        let source_is_preset = |preset: &&BrushPreset| {
                            if source_folder.is_empty() {
                                preset.group == *source_group
                            } else {
                                preset.group == *source_group && preset.folder.starts_with(&source_folder)
                            }
                        };
                        if target_group.is_empty() && target_parent.is_empty() {
                            let action = if after {
                                match root_sibling_after_item(presets, key, &[source_group]) {
                                    Some(RootSiblingAnchor::Group(group)) => {
                                        Action::MoveGroup { group: source_group_name, folder: source_folder, before: Some(group), root_index: None }
                                    }
                                    Some(RootSiblingAnchor::Item(item)) => Action::MoveInto {
                                        group: source_group_name,
                                        folder: source_folder,
                                        into: None,
                                        before: None,
                                        before_item: Some(item),
                                        root_index: None,
                                    },
                                    None => Action::MoveGroup { group: source_group_name, folder: source_folder, before: None, root_index: None },
                                }
                            } else {
                                Action::MoveInto {
                                    group: source_group_name,
                                    folder: source_folder,
                                    into: None,
                                    before: None,
                                    before_item: Some(resolve_before_item_name(presets, key)?),
                                    root_index: None,
                                }
                            };
                            return Ok(action);
                        }
                        let (before, before_item) = if after {
                            let after_index = presets
                                .iter()
                                .position(|preset| preview_identity(preset) == key.as_str())
                                .ok_or_else(|| "The target Brush no longer exists.".to_owned())?;
                            let next = presets
                                .iter()
                                .skip(after_index + 1)
                                .find(|preset| preset.group == target_group && preset.folder.starts_with(&target_parent) && !source_is_preset(preset));
                            let empty_folder = empty_folder_before_item(
                                hierarchy,
                                &target_group,
                                &target_parent,
                                next.map(|preset| preset.name.as_str()),
                                (!source_folder.is_empty() && source_group == &target_group).then_some(source_folder.as_slice()),
                            );
                            if let Some(folder) = empty_folder {
                                (Some(folder), None)
                            } else {
                                match next {
                                    Some(next) if next.folder == target_parent => (None, Some(next.name.clone())),
                                    Some(next) => (Some(next.folder[target_parent.len()].clone()), None),
                                    None => (None, None),
                                }
                            }
                        } else {
                            (None, Some(resolve_before_item_name(presets, key)?))
                        };
                        Ok(move_into(Some((target_group_name, target_parent)), before, before_item))
                    }
                    _ => Err("The Brush drop target cannot receive this group move.".to_owned()),
                }
            }
        }
    }
}

pub fn browser_actions_to_actions(
    presets: &[BrushPreset],
    browser_actions: Vec<BrowserAction>,
    selection: Option<&crate::pickers::state::BrowserSelectionSnapshot>,
) -> Vec<Action> {
    browser_actions
        .into_iter()
        .filter_map(|action| match action {
            BrowserAction::SelectItem { key, .. } => occurrence_name(presets, &key).map(Action::Select),
            BrowserAction::ActivateItem { key, .. } => occurrence_name(presets, &key).map(Action::Choose),
            BrowserAction::Rename(key, text) => {
                occurrence_name(presets, &key).map(|name| Action::Rename(Renaming { group: false, name, folder: Vec::new(), text }))
            }
            BrowserAction::RenameItem { group: _, group_path, key, text } => {
                occurrence_name(presets, &key).map(|name| Action::Rename(Renaming { group: false, name, folder: group_path, text }))
            }
            BrowserAction::BeginRenameSelection { .. }
            | BrowserAction::BeginInlineRenameItem(_, _)
            | BrowserAction::CancelInlineRenameItem
            | BrowserAction::BeginInlineRenameGroup { .. }
            | BrowserAction::CancelInlineRenameGroup => None,
            BrowserAction::DeleteItem { group: _, group_path: _, key } => preset_by_identity(presets, &key).map(|preset| Action::DeleteOccurrence {
                name: preset.name.clone(),
                group: preset.group.clone(),
                folder: preset.folder.clone(),
            }),
            BrowserAction::Move(key, group) => occurrence_name(presets, &key).map(|name| Action::Move {
                name,
                group: group_key(presets, &group),
                folder: Some(Vec::new()),
                index: None,
                root_index: None,
            }),
            BrowserAction::RenameGroup(group, text) => {
                Some(Action::Rename(Renaming { group: true, name: group_key(presets, &group), folder: Vec::new(), text }))
            }
            BrowserAction::DeleteGroup(group) => Some(Action::DeleteGroup { group: group_key(presets, &group), folder: Vec::new() }),
            BrowserAction::RenameNestedGroup { group, group_path, text } => {
                Some(Action::Rename(Renaming { group: true, name: group_key(presets, &group), folder: group_path, text }))
            }
            BrowserAction::DeleteNestedGroup { group, group_path } => Some(Action::DeleteGroup { group: group_key(presets, &group), folder: group_path }),
            BrowserAction::New => Some(Action::NewPreset),
            BrowserAction::CreateGroup(group) => Some(Action::CreateGroup {
                group,
                presets: selection
                    .into_iter()
                    .flat_map(|selection| selection.cards.iter())
                    .filter_map(|(_, key)| preset_by_identity(presets, key).map(|preset| preset.name.clone()))
                    .collect(),
            }),
            BrowserAction::Drop(_, _)
            | BrowserAction::NewGroup
            | BrowserAction::AppendDefault
            | BrowserAction::Import
            | BrowserAction::ExportSelected(_)
            | BrowserAction::ExportAll(_)
            | BrowserAction::CopyItem { .. } => None,
        })
        .collect()
}

pub fn preview_identity(preset: &BrushPreset) -> String {
    format!("{:x}@{}", std::ptr::from_ref(preset) as usize, preset.name)
}

/// Resolve a browser's opaque occurrence key back to the exact borrowed preset.
pub fn preset_by_identity<'a>(presets: &'a [BrushPreset], key: &str) -> Option<&'a BrushPreset> {
    presets.iter().find(|preset| preview_identity(preset) == key)
}

/// Whether a name-based engine command cannot identify one preset occurrence unambiguously.
pub fn is_ambiguous_preset_name(presets: &[BrushPreset], name: &str) -> bool {
    presets.iter().filter(|preset| preset.name.eq_ignore_ascii_case(name)).take(2).count() > 1
}

pub(super) fn occurrence_name(presets: &[BrushPreset], key: &str) -> Option<String> {
    preset_by_identity(presets, key).map(|preset| preset.name.clone())
}

/// Resolve a stable browser occurrence key to the engine's name-based anchor, rejecting names
/// that cannot identify exactly one preset in the library.
fn resolve_before_item_name(presets: &[BrushPreset], key: &str) -> Result<String, String> {
    let name = occurrence_name(presets, key).ok_or_else(|| "Cannot position the group: the destination brush no longer exists.".to_string())?;
    if is_ambiguous_preset_name(presets, &name) {
        return Err(format!("Cannot position the group before `{name}` because multiple presets share that name."));
    }
    Ok(name)
}

pub(super) fn group_label_for_key(presets: &[BrushPreset], key: &str) -> Option<String> {
    grouped_presets(presets).into_iter().find(|(label, _)| group_key(presets, label) == key).map(|(label, _)| label)
}

pub fn drop_target(presets: &[BrushPreset], dragged: &str, target: &str, after: bool) -> Option<(String, Vec<String>, usize)> {
    let resolve = |key: &str| {
        preset_by_identity(presets, key).or_else(|| {
            let mut matches = presets.iter().filter(|preset| preset.name == key);
            let first = matches.next()?;
            matches.next().is_none().then_some(first)
        })
    };
    let dragged_preset = resolve(dragged)?;
    let t = resolve(target)?;
    let members: Vec<&BrushPreset> = presets.iter().filter(|p| p.group == t.group).collect();
    let tp = members.iter().position(|preset| std::ptr::eq(*preset, t))?;
    let mut idx = tp + usize::from(after);
    if let Some(dp) = members.iter().position(|preset| std::ptr::eq(*preset, dragged_preset))
        && dp < idx
    {
        idx -= 1;
    }
    Some((t.group.clone(), t.folder.clone(), idx))
}

fn set_current_preset(app: &mut PhotocraftApp, name: &str) {
    if is_ambiguous_preset_name(&app.session.tools.presets, name) {
        app.ui.status = format!("Cannot select brush `{name}` because multiple presets share that name.");
        return;
    }
    match app.run("tools.setBrush", json!({ "preset": name })) {
        Ok(_) => {
            app.ui.presets_ui.selected.remove("brush-texture-patterns");
        }
        Err(e) => app.ui.status = e,
    }
}

fn clear_brush_browser_selections(app: &mut PhotocraftApp) {
    for panel in ["brushes", "brush-picker"] {
        app.ui.presets_ui.clear_browser_selection(panel);
    }
}

fn remap_brush_group_expansion(app: &mut PhotocraftApp, from: &str, to: &str) {
    remap_collapsed_prefix(&mut app.ui.brushes_panel.collapsed, from, to);
    for panel in ["brushes", "brush-picker"] {
        let from = format!("{panel}/{from}");
        let to = format!("{panel}/{to}");
        let remapped: Vec<_> = app
            .ui
            .presets_ui
            .layout
            .toggled
            .iter()
            .filter_map(|key| {
                (key == &from || key.strip_prefix(&from).is_some_and(|tail| tail.starts_with('/'))).then(|| format!("{to}{}", &key[from.len()..]))
            })
            .collect();
        app.ui.presets_ui.layout.toggled.retain(|key| key != &from && !key.strip_prefix(&from).is_some_and(|tail| tail.starts_with('/')));
        app.ui.presets_ui.layout.toggled.extend(remapped);
    }
}

fn remove_brush_group_expansion(app: &mut PhotocraftApp, prefix: &str) {
    remove_collapsed_prefix(&mut app.ui.brushes_panel.collapsed, prefix);
    for panel in ["brushes", "brush-picker"] {
        let prefix = format!("{panel}/{prefix}");
        app.ui.presets_ui.layout.toggled.retain(|key| key != &prefix && !key.strip_prefix(&prefix).is_some_and(|tail| tail.starts_with('/')));
    }
}

/// Turn a preset list's actions into commands.
pub fn apply(app: &mut PhotocraftApp, acts: Vec<Action>) {
    for a in acts {
        match a {
            Action::Select(name) => set_current_preset(app, &name),
            Action::Choose(name) => {
                // The double-click's first click already picked it.
                if is_ambiguous_preset_name(&app.session.tools.presets, &name) || app.session.tools.current_preset.as_deref() != Some(name.as_str()) {
                    set_current_preset(app, &name);
                }
            }
            Action::ToggleGroup(_) | Action::BeginRename(_) => {}
            Action::Status(message) => app.ui.status = message,
            Action::Move { name, group, folder, index, root_index } => {
                if is_ambiguous_preset_name(&app.session.tools.presets, &name) {
                    app.ui.status = format!("Cannot move brush `{name}` because multiple presets share that name.");
                    continue;
                }
                let mut p = json!({ "name": name, "group": group });
                if let Some(path) = folder {
                    p["folder"] = json!(path);
                }
                if let Some(i) = index {
                    p["index"] = json!(i);
                }
                if let Some(i) = root_index {
                    p["rootIndex"] = json!(i);
                }
                run_or_status(app, "brush.presets.move", p);
            }
            Action::MoveBefore { name, group, folder, before } => {
                if is_ambiguous_preset_name(&app.session.tools.presets, &name) || is_ambiguous_preset_name(&app.session.tools.presets, &before) {
                    app.ui.status = format!("Cannot reorder brush `{name}` because the dragged brush or destination `{before}` has a duplicate name.");
                    continue;
                }
                let mut p = json!({ "name": name, "group": group, "before": before });
                if let Some(path) = folder {
                    p["folder"] = json!(path);
                }
                run_or_status(app, "brush.presets.move", p);
            }
            Action::MoveGroup { group, folder, before, root_index } => {
                let mut p = json!({ "group": group, "before": before });
                if !folder.is_empty() {
                    p["folder"] = json!(folder);
                }
                if let Some(i) = root_index {
                    p["rootIndex"] = json!(i);
                }
                run_or_status(app, "brush.presets.moveGroup", p);
            }
            Action::MoveInto { group, folder, into, before, before_item, root_index } => {
                if before_item.as_ref().is_some_and(|name| is_ambiguous_preset_name(&app.session.tools.presets, name)) {
                    app.ui.status =
                        format!("Cannot position the group before `{}` because multiple presets share that name.", before_item.as_deref().unwrap_or_default());
                    continue;
                }
                // Empty groups have no preset from which to derive their display label; their
                // engine group key is already the stable label used by the browser projection.
                let source_label = Some(group_label_for_key(&app.session.tools.presets, &group).unwrap_or_else(|| group.clone()));
                let expansion_move = source_label.as_ref().map(|label| {
                    let from = group_view_key(label, &folder);
                    let (destination_label, destination_path) = match into.as_ref() {
                        Some((destination_group, destination_path)) => {
                            let destination_label =
                                group_label_for_key(&app.session.tools.presets, destination_group).unwrap_or_else(|| destination_group.clone());
                            let leaf = folder.last().unwrap_or(label).clone();
                            let mut path = destination_path.clone();
                            path.push(leaf);
                            (destination_label, path)
                        }
                        None if !folder.is_empty() => (folder.last().cloned().unwrap_or_default(), Vec::new()),
                        None => (label.clone(), folder.clone()),
                    };
                    (from, group_view_key(&destination_label, &destination_path))
                });
                let into = match into {
                    Some((g, f)) => json!({ "group": g, "folder": f }),
                    None => json!({}),
                };
                let mut p = json!({ "group": group, "into": into });
                if !folder.is_empty() {
                    p["folder"] = json!(folder);
                }
                if let Some(b) = before {
                    p["before"] = json!(b);
                }
                if let Some(item) = before_item {
                    p["beforeItem"] = json!(item);
                }
                if let Some(i) = root_index {
                    p["rootIndex"] = json!(i);
                }
                if run_or_status(app, "brush.presets.moveGroup", p)
                    && let Some((from, to)) = expansion_move
                {
                    remap_brush_group_expansion(app, &from, &to);
                }
            }
            Action::Rename(r) => {
                let text = r.text.trim().to_string();
                let old = r.folder.last().unwrap_or(&r.name);
                if !text.is_empty() && text != *old {
                    if r.group {
                        let label = group_label_for_key(&app.session.tools.presets, &r.name).unwrap_or_else(|| r.name.clone());
                        let mut p = json!({ "group": r.name, "newName": text });
                        if !r.folder.is_empty() {
                            p["folder"] = json!(r.folder);
                        }
                        if run_or_status(app, "brush.presets.renameGroup", p) {
                            let from = group_view_key(&label, &r.folder);
                            let to = if r.folder.is_empty() {
                                group_view_key(&text, &[])
                            } else {
                                let mut folder = r.folder.clone();
                                if let Some(last) = folder.last_mut() {
                                    *last = text.clone();
                                }
                                group_view_key(&label, &folder)
                            };
                            remap_brush_group_expansion(app, &from, &to);
                        }
                    } else {
                        if is_ambiguous_preset_name(&app.session.tools.presets, &r.name) {
                            app.ui.status = format!("Cannot rename brush `{}` because multiple presets share that name.", r.name);
                            continue;
                        }
                        run_or_status(app, "brush.presets.rename", json!({ "name": r.name, "newName": text }));
                    }
                }
            }
            Action::Delete(name) => {
                if is_ambiguous_preset_name(&app.session.tools.presets, &name) {
                    app.ui.status = format!("Cannot delete brush `{name}` because multiple presets share that name.");
                    continue;
                }
                if run_or_status(app, "brush.presets.delete", json!({ "name": name })) {
                    clear_brush_browser_selections(app);
                }
            }
            Action::DeleteOccurrence { name, group, folder } => {
                if run_or_status(app, "brush.presets.delete", json!({ "name": name, "group": group, "folder": folder })) {
                    clear_brush_browser_selections(app);
                }
            }
            Action::DeleteGroup { group, folder } => {
                let label = group_label_for_key(&app.session.tools.presets, &group).unwrap_or_else(|| group.clone());
                let mut p = json!({ "group": group });
                if !folder.is_empty() {
                    p["folder"] = json!(folder);
                }
                if run_or_status(app, "brush.presets.deleteGroup", p) {
                    for panel in ["brushes", "brush-picker"] {
                        if app.ui.presets_ui.selected_group.get(panel) == Some(&label)
                            && app.ui.presets_ui.selected_group_path.get(panel).is_some_and(|path| path.starts_with(&folder))
                        {
                            app.ui.presets_ui.clear_selected(panel);
                        }
                    }
                    remove_brush_group_expansion(app, &group_view_key(&label, &folder));
                    clear_brush_browser_selections(app);
                }
            }
            Action::NewPreset => {
                crate::brush_preset_dialog::open(app, None);
            }
            Action::CreateGroup { group, presets } => {
                let presets = if presets.is_empty() { app.session.tools.current_preset.iter().cloned().collect::<Vec<_>>() } else { presets };
                if presets.is_empty() {
                    run_or_status(app, "brush.presets.createGroup", json!({ "group": group }));
                    continue;
                }
                if app.session.tools.presets.iter().any(|preset| preset.group.eq_ignore_ascii_case(&group))
                    || app.session.tools.preset_hierarchy.groups.iter().any(|existing| existing.eq_ignore_ascii_case(&group))
                {
                    app.ui.status = format!("A brush group named `{group}` already exists.");
                    continue;
                }
                if let Some(name) = presets.iter().find(|name| is_ambiguous_preset_name(&app.session.tools.presets, name)) {
                    app.ui.status = format!("Cannot create the group because brush `{name}` has a duplicate name.");
                    continue;
                }
                for name in presets {
                    if !run_or_status(app, "brush.presets.move", json!({ "name": name, "group": &group, "folder": [] })) {
                        break;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod hierarchy_tests {
    use super::{Action, brush_drop_intent_to_action, brush_group_views, brush_projection_keys, brush_root_sequence, preview_identity};
    use crate::pickers::browser::dragging_model::DropMoveIntent;
    use crate::pickers::browser::identity::{PickerNodeIdentity, ProjectionNodeRef};
    use crate::pickers::browser::path_identity_model::{ContainerId, DropPosition, PickerId, PickerIdRegistry};
    use crate::pickers::state::HierarchyOrderEntry;
    use photocraft_engine::{BrushPresetFolder, BrushPresetHierarchy};

    fn preset(name: &str, group: &str, folder: &[&str]) -> photocraft_engine::paint::BrushPreset {
        photocraft_engine::paint::BrushPreset {
            name: name.into(),
            brush: Default::default(),
            builtin: false,
            group: group.into(),
            folder: folder.iter().map(|part| (*part).to_owned()).collect(),
            captures: Default::default(),
        }
    }

    #[test]
    fn projection_keeps_empty_groups_and_nested_folders_without_fake_cards() {
        let hierarchy = BrushPresetHierarchy {
            groups: vec!["Empty".into()],
            folders: vec![
                BrushPresetFolder { group: "Empty".into(), path: vec!["Parent".into()], before_item: None },
                BrushPresetFolder { group: "Empty".into(), path: vec!["Parent".into(), "Child".into()], before_item: None },
            ],
            root_order: Vec::new(),
        };
        let views = brush_group_views(&[], &hierarchy);
        assert_eq!(views.len(), 1);
        assert!(views[0].view.items.is_empty());
        let keys = brush_projection_keys(&[views[0].view.clone()], &hierarchy);
        assert!(keys.key(&ProjectionNodeRef::Folder { group_index: 0, path: vec!["Parent".into()] }).is_some());
        assert!(keys.key(&ProjectionNodeRef::Folder { group_index: 0, path: vec!["Parent".into(), "Child".into()] }).is_some());
        assert_eq!(brush_root_sequence(&[], &hierarchy).len(), 1);
    }

    #[test]
    fn root_projection_preserves_an_empty_group_between_root_cards() {
        let presets = vec![preset("Root A", "", &[]), preset("Root B", "", &[])];
        let hierarchy = BrushPresetHierarchy {
            groups: vec!["Empty".into()],
            folders: Vec::new(),
            root_order: vec![
                photocraft_engine::BrushPresetRootEntry::Preset("Root A".into()),
                photocraft_engine::BrushPresetRootEntry::Group("Empty".into()),
                photocraft_engine::BrushPresetRootEntry::Preset("Root B".into()),
            ],
        };
        assert_eq!(
            brush_root_sequence(&presets, &hierarchy),
            [
                HierarchyOrderEntry::RootItem { group: String::new(), key: preview_identity(&presets[0]) },
                HierarchyOrderEntry::Group("Empty".into()),
                HierarchyOrderEntry::RootItem { group: String::new(), key: preview_identity(&presets[1]) },
            ]
        );
    }

    #[test]
    fn group_can_be_dropped_before_or_after_a_root_card() {
        let presets = vec![preset("Root A", "", &[]), preset("Root B", "", &[]), preset("Source", "Source", &[])];
        let source = PickerId::group("Source");
        let target = PickerId::item("", &preview_identity(&presets[0]));
        let identities = std::collections::BTreeMap::from([
            (source.clone(), PickerNodeIdentity::Group("Source".into())),
            (target.clone(), PickerNodeIdentity::Item { group: String::new(), key: preview_identity(&presets[0]) }),
        ]);
        let root_sequence = vec![
            HierarchyOrderEntry::RootItem { group: String::new(), key: preview_identity(&presets[0]) },
            HierarchyOrderEntry::RootItem { group: String::new(), key: preview_identity(&presets[1]) },
            HierarchyOrderEntry::Group("Source".into()),
        ];
        for (position, expected_index) in [(DropPosition::Before { target: target.clone() }, 0), (DropPosition::After { target: target.clone() }, 1)] {
            let action = brush_drop_intent_to_action(
                &presets,
                &BrushPresetHierarchy::default(),
                &identities,
                &root_sequence,
                &DropMoveIntent { source: source.clone(), selection: Vec::new(), position },
            )
            .unwrap();
            assert!(matches!(action, Action::MoveGroup { ref group, root_index: Some(index), .. } if group == "Source" && index == expected_index));
        }
    }

    #[test]
    fn empty_group_can_be_dropped_between_root_cards_and_at_both_ends() {
        let presets = vec![preset("Root A", "", &[]), preset("Root B", "", &[])];
        let source = PickerId::group("Empty");
        let root_a = PickerId::item("", &preview_identity(&presets[0]));
        let root_b = PickerId::item("", &preview_identity(&presets[1]));
        let identities = std::collections::BTreeMap::from([
            (source.clone(), PickerNodeIdentity::Group("Empty".into())),
            (root_a.clone(), PickerNodeIdentity::Item { group: String::new(), key: preview_identity(&presets[0]) }),
            (root_b.clone(), PickerNodeIdentity::Item { group: String::new(), key: preview_identity(&presets[1]) }),
        ]);
        let root_sequence = vec![
            HierarchyOrderEntry::RootItem { group: String::new(), key: preview_identity(&presets[0]) },
            HierarchyOrderEntry::RootItem { group: String::new(), key: preview_identity(&presets[1]) },
            HierarchyOrderEntry::Group("Empty".into()),
        ];

        for (position, expected_index) in [
            (DropPosition::Before { target: root_a.clone() }, 0),
            (DropPosition::After { target: root_a.clone() }, 1),
            (DropPosition::Before { target: root_b.clone() }, 1),
            (DropPosition::After { target: root_b.clone() }, 2),
        ] {
            let action = brush_drop_intent_to_action(
                &presets,
                &BrushPresetHierarchy::default(),
                &identities,
                &root_sequence,
                &DropMoveIntent { source: source.clone(), selection: Vec::new(), position },
            )
            .unwrap();
            assert!(matches!(action, Action::MoveGroup { ref group, root_index: Some(index), .. } if group == "Empty" && index == expected_index));
        }
    }

    #[test]
    fn moving_an_empty_collapsed_group_into_another_group_preserves_its_expansion_state() {
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.tools.presets.clear();
        app.session.tools.preset_hierarchy = BrushPresetHierarchy {
            groups: vec!["Empty".into(), "Target".into()],
            folders: Vec::new(),
            root_order: vec![photocraft_engine::BrushPresetRootEntry::Group("Empty".into()), photocraft_engine::BrushPresetRootEntry::Group("Target".into())],
        };
        app.ui.brushes_panel.collapsed = vec!["Empty".into(), "Empty/Child".into()];
        app.ui.presets_ui.layout.toggled.extend([
            "brushes/Empty".into(),
            "brushes/Empty/Child".into(),
            "brush-picker/Empty".into(),
            "brush-picker/Empty/Child".into(),
        ]);

        super::apply(
            &mut app,
            vec![Action::MoveInto {
                group: "Empty".into(),
                folder: Vec::new(),
                into: Some(("Target".into(), Vec::new())),
                before: None,
                before_item: None,
                root_index: None,
            }],
        );

        assert_eq!(app.ui.brushes_panel.collapsed, ["Target/Empty", "Target/Empty/Child"]);
        for panel in ["brushes", "brush-picker"] {
            assert!(app.ui.presets_ui.layout.toggled.contains(&format!("{panel}/Target/Empty")));
            assert!(app.ui.presets_ui.layout.toggled.contains(&format!("{panel}/Target/Empty/Child")));
            assert!(!app.ui.presets_ui.layout.toggled.contains(&format!("{panel}/Empty")));
        }
        assert_eq!(app.session.tools.preset_hierarchy.groups, ["Target"]);
        assert_eq!(app.session.tools.preset_hierarchy.folders[0].path, ["Empty"]);
    }

    #[test]
    fn promoting_a_collapsed_nested_group_to_root_preserves_its_expansion_state() {
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.tools.presets.clear();
        app.session.tools.preset_hierarchy = BrushPresetHierarchy {
            groups: vec!["Target".into()],
            folders: vec![
                photocraft_engine::BrushPresetFolder { group: "Target".into(), path: vec!["Parent".into()], before_item: None },
                photocraft_engine::BrushPresetFolder { group: "Target".into(), path: vec!["Parent".into(), "Empty".into()], before_item: None },
                photocraft_engine::BrushPresetFolder { group: "Target".into(), path: vec!["Parent".into(), "Empty".into(), "Child".into()], before_item: None },
            ],
            root_order: vec![photocraft_engine::BrushPresetRootEntry::Group("Target".into())],
        };
        app.ui.brushes_panel.collapsed = vec!["Target/Parent/Empty".into(), "Target/Parent/Empty/Child".into()];
        app.ui.presets_ui.layout.toggled.extend([
            "brushes/Target/Parent/Empty".into(),
            "brushes/Target/Parent/Empty/Child".into(),
            "brush-picker/Target/Parent/Empty".into(),
            "brush-picker/Target/Parent/Empty/Child".into(),
        ]);

        super::apply(
            &mut app,
            vec![Action::MoveInto {
                group: "Target".into(),
                folder: vec!["Parent".into(), "Empty".into()],
                into: None,
                before: None,
                before_item: None,
                root_index: None,
            }],
        );

        assert_eq!(app.ui.brushes_panel.collapsed, ["Empty", "Empty/Child"]);
        for panel in ["brushes", "brush-picker"] {
            assert!(app.ui.presets_ui.layout.toggled.contains(&format!("{panel}/Empty")));
            assert!(app.ui.presets_ui.layout.toggled.contains(&format!("{panel}/Empty/Child")));
            assert!(!app.ui.presets_ui.layout.toggled.contains(&format!("{panel}/Target/Parent/Empty")));
        }
        assert!(app.session.tools.preset_hierarchy.groups.contains(&"Empty".to_owned()));
        assert!(!app.session.tools.preset_hierarchy.folders.iter().any(|folder| folder.group == "Target" && folder.path == ["Parent", "Empty"]));
    }

    #[test]
    fn renaming_and_deleting_an_empty_group_remaps_and_clears_expansion_state() {
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.tools.presets.clear();
        app.session.tools.preset_hierarchy = BrushPresetHierarchy {
            groups: vec!["Empty".into()],
            folders: Vec::new(),
            root_order: vec![photocraft_engine::BrushPresetRootEntry::Group("Empty".into())],
        };
        app.ui.brushes_panel.collapsed = vec!["Empty".into(), "Empty/Child".into()];
        app.ui.presets_ui.layout.toggled.extend(["brushes/Empty".into(), "brushes/Empty/Child".into(), "brush-picker/Empty".into()]);

        super::apply(&mut app, vec![Action::Rename(super::Renaming { group: true, name: "Empty".into(), folder: Vec::new(), text: "Renamed".into() })]);
        assert_eq!(app.ui.brushes_panel.collapsed, ["Renamed", "Renamed/Child"]);
        assert!(app.ui.presets_ui.layout.toggled.contains("brushes/Renamed"));
        assert!(app.ui.presets_ui.layout.toggled.contains("brushes/Renamed/Child"));
        assert!(app.ui.presets_ui.layout.toggled.contains("brush-picker/Renamed"));

        super::apply(&mut app, vec![Action::DeleteGroup { group: "Renamed".into(), folder: Vec::new() }]);
        assert!(app.ui.brushes_panel.collapsed.is_empty());
        assert!(!app.ui.presets_ui.layout.toggled.iter().any(|key| key.starts_with("brushes/Renamed") || key.starts_with("brush-picker/Renamed")));
    }

    #[test]
    fn root_cards_can_be_moved_to_the_absolute_start_and_end() {
        let presets = vec![preset("Root A", "", &[]), preset("Root B", "", &[]), preset("Root C", "", &[])];
        let source = PickerId::item("", &preview_identity(&presets[0]));
        let first = PickerId::item("", &preview_identity(&presets[0]));
        let last = PickerId::item("", &preview_identity(&presets[2]));
        let identities = std::collections::BTreeMap::from([
            (source.clone(), PickerNodeIdentity::Item { group: String::new(), key: preview_identity(&presets[0]) }),
            (first.clone(), PickerNodeIdentity::Item { group: String::new(), key: preview_identity(&presets[0]) }),
            (last.clone(), PickerNodeIdentity::Item { group: String::new(), key: preview_identity(&presets[2]) }),
        ]);
        let root_sequence =
            presets.iter().map(|preset| HierarchyOrderEntry::RootItem { group: String::new(), key: preview_identity(preset) }).collect::<Vec<_>>();

        for (source_id, position, expected) in [
            (last.clone(), DropPosition::Before { target: first.clone() }, 0),
            (source.clone(), DropPosition::After { target: last.clone() }, 2),
            (source.clone(), DropPosition::AppendTo { container: ContainerId::Root }, 2),
        ] {
            let action = brush_drop_intent_to_action(
                &presets,
                &BrushPresetHierarchy::default(),
                &identities,
                &root_sequence,
                &DropMoveIntent { source: source_id, selection: Vec::new(), position },
            )
            .unwrap();
            assert!(matches!(action, Action::Move { root_index: Some(index), .. } if index == expected));
        }
    }

    #[test]
    fn root_card_can_be_dropped_before_or_after_a_group_as_a_sibling() {
        let presets = vec![preset("Target", "Target", &[]), preset("Root", "", &[]), preset("Source", "Source", &[])];
        let source = PickerId::item("Source", &preview_identity(&presets[2]));
        let target = PickerId::group("Target");
        let identities = std::collections::BTreeMap::from([
            (source.clone(), PickerNodeIdentity::Item { group: "Source".into(), key: preview_identity(&presets[2]) }),
            (target.clone(), PickerNodeIdentity::Group("Target".into())),
        ]);
        let root_sequence = vec![
            HierarchyOrderEntry::Group("Target".into()),
            HierarchyOrderEntry::RootItem { group: String::new(), key: preview_identity(&presets[1]) },
            HierarchyOrderEntry::Group("Source".into()),
        ];
        for position in [DropPosition::Before { target: target.clone() }, DropPosition::After { target: target.clone() }] {
            let action = brush_drop_intent_to_action(
                &presets,
                &BrushPresetHierarchy::default(),
                &identities,
                &root_sequence,
                &DropMoveIntent { source: source.clone(), selection: Vec::new(), position },
            )
            .unwrap();
            assert!(
                matches!(action, Action::Move { ref name, ref group, ref folder, index: Some(0), root_index: Some(_)} if name == "Source" && group.is_empty() && folder.as_deref() == Some(&[][..]))
            );
        }
    }

    #[test]
    fn card_after_last_nested_folder_is_inserted_as_a_parent_sibling() {
        let presets = vec![preset("Nested card", "Parent", &["Nested"]), preset("Source", "Other", &[])];
        let source = PickerId::item("Other", &preview_identity(&presets[1]));
        let target = PickerId::group_path(["Nested"], "Nested");
        let identities = std::collections::BTreeMap::from([
            (source.clone(), PickerNodeIdentity::Item { group: "Other".into(), key: preview_identity(&presets[1]) }),
            (target.clone(), PickerNodeIdentity::Folder { group: "Parent".into(), path: vec!["Nested".into()] }),
        ]);
        let action = brush_drop_intent_to_action(
            &presets,
            &BrushPresetHierarchy::default(),
            &identities,
            &[],
            &DropMoveIntent { source, selection: Vec::new(), position: DropPosition::After { target } },
        )
        .unwrap();
        assert!(
            matches!(action, Action::Move { ref name, ref group, ref folder, index: None, .. } if name == "Source" && group == "Parent" && folder.as_deref() == Some(&[][..]))
        );
    }

    #[test]
    fn empty_group_dropped_after_a_card_is_inserted_before_the_next_empty_folder() {
        let presets = vec![preset("A", "Parent", &[]), preset("B", "Parent", &[])];
        let hierarchy = BrushPresetHierarchy {
            groups: vec!["Parent".into(), "Source".into()],
            folders: vec![BrushPresetFolder { group: "Parent".into(), path: vec!["Existing Empty".into()], before_item: Some("B".into()) }],
            root_order: Vec::new(),
        };
        let source = PickerId::group("Source");
        let target = PickerId::item("Parent", &preview_identity(&presets[0]));
        let identities = std::collections::BTreeMap::from([
            (source.clone(), PickerNodeIdentity::Group("Source".into())),
            (target.clone(), PickerNodeIdentity::Item { group: "Parent".into(), key: preview_identity(&presets[0]) }),
        ]);

        let action = brush_drop_intent_to_action(
            &presets,
            &hierarchy,
            &identities,
            &[],
            &DropMoveIntent { source, selection: Vec::new(), position: DropPosition::After { target } },
        )
        .unwrap();

        assert!(matches!(action, Action::MoveInto { ref group, ref into, ref before, before_item: None, .. }
            if group == "Source" && into.as_ref().is_some_and(|(destination, path)| destination == "Parent" && path.is_empty()) && before.as_deref() == Some("Existing Empty")));
    }

    #[test]
    fn moving_an_empty_folder_after_its_previous_card_does_not_anchor_before_itself() {
        let presets = vec![preset("A", "Parent", &[]), preset("B", "Parent", &[])];
        let hierarchy = BrushPresetHierarchy {
            groups: vec!["Parent".into()],
            folders: vec![BrushPresetFolder { group: "Parent".into(), path: vec!["Empty".into()], before_item: Some("B".into()) }],
            root_order: Vec::new(),
        };
        let source = PickerId::group_path(["Parent"], "Empty");
        let target = PickerId::item("Parent", &preview_identity(&presets[0]));
        let identities = std::collections::BTreeMap::from([
            (source.clone(), PickerNodeIdentity::Folder { group: "Parent".into(), path: vec!["Empty".into()] }),
            (target.clone(), PickerNodeIdentity::Item { group: "Parent".into(), key: preview_identity(&presets[0]) }),
        ]);

        let action = brush_drop_intent_to_action(
            &presets,
            &hierarchy,
            &identities,
            &[],
            &DropMoveIntent { source, selection: Vec::new(), position: DropPosition::After { target } },
        )
        .unwrap();

        assert!(matches!(action, Action::MoveInto { ref group, ref folder, ref into, before: None, ref before_item, .. }
            if group == "Parent" && folder == &["Empty".to_owned()] && into.as_ref().is_some_and(|(destination, path)| destination == "Parent" && path.is_empty()) && before_item.as_deref() == Some("B")));
    }

    #[test]
    fn empty_folder_projection_uses_its_direct_card_anchor() {
        let presets = vec![preset("A", "Parent", &[]), preset("B", "Parent", &[])];
        let hierarchy = BrushPresetHierarchy {
            groups: vec!["Parent".into()],
            folders: vec![BrushPresetFolder { group: "Parent".into(), path: vec!["Empty".into()], before_item: Some("B".into()) }],
            root_order: vec![photocraft_engine::BrushPresetRootEntry::Group("Parent".into())],
        };
        let view = brush_group_views(&presets, &hierarchy)[0].view.clone();
        let keys = brush_projection_keys(std::slice::from_ref(&view), &hierarchy);
        let orders = std::collections::BTreeMap::from([
            ("".to_owned(), vec![HierarchyOrderEntry::Group("Parent".into())]),
            ("Parent".to_owned(), vec![HierarchyOrderEntry::Item(preview_identity(&presets[0])), HierarchyOrderEntry::Item(preview_identity(&presets[1]))]),
        ]);
        let (segments, tree, identities) = crate::pickers::browser::projection::build_hierarchy_projection(
            &[view],
            &[(0, 0)],
            &std::collections::BTreeMap::new(),
            &orders,
            &keys,
            &mut PickerIdRegistry::default(),
        )
        .unwrap();
        let parent =
            identities.iter().find_map(|(id, identity)| matches!(identity, PickerNodeIdentity::Group(name) if name == "Parent").then_some(id)).unwrap();
        let children = tree.node(parent).unwrap().children();
        let labels = children
            .iter()
            .map(|node| match identities.get(node.id()).unwrap() {
                PickerNodeIdentity::Item { key, .. } => format!("item:{key}"),
                PickerNodeIdentity::Folder { path, .. } => format!("folder:{}", path.join("/")),
                other => format!("other:{other:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(labels, [format!("item:{}", preview_identity(&presets[0])), "folder:Empty".into(), format!("item:{}", preview_identity(&presets[1]))]);
        assert!(segments.iter().any(|segment| segment.folder_path == ["Empty"]));
    }

    #[test]
    fn empty_folder_projection_keeps_folder_sibling_order_before_populated_folders() {
        let presets = vec![preset("Inside", "Parent", &["Filled"]), preset("Root", "Parent", &[])];
        let hierarchy = BrushPresetHierarchy {
            groups: vec!["Parent".into()],
            folders: vec![
                BrushPresetFolder { group: "Parent".into(), path: vec!["Empty".into()], before_item: None },
                BrushPresetFolder { group: "Parent".into(), path: vec!["Filled".into()], before_item: None },
            ],
            root_order: vec![photocraft_engine::BrushPresetRootEntry::Group("Parent".into())],
        };
        let view = brush_group_views(&presets, &hierarchy)[0].view.clone();
        let keys = brush_projection_keys(std::slice::from_ref(&view), &hierarchy);
        let orders = std::collections::BTreeMap::from([
            ("".to_owned(), vec![HierarchyOrderEntry::Group("Parent".into())]),
            ("Parent".to_owned(), vec![HierarchyOrderEntry::Item(preview_identity(&presets[0])), HierarchyOrderEntry::Item(preview_identity(&presets[1]))]),
        ]);
        let (_, tree, identities) = crate::pickers::browser::projection::build_hierarchy_projection(
            &[view],
            &[(0, 0)],
            &std::collections::BTreeMap::new(),
            &orders,
            &keys,
            &mut PickerIdRegistry::default(),
        )
        .unwrap();
        let parent =
            identities.iter().find_map(|(id, identity)| matches!(identity, PickerNodeIdentity::Group(name) if name == "Parent").then_some(id)).unwrap();
        let children = tree.node(parent).unwrap().children();
        let folders = children
            .iter()
            .filter_map(|node| match identities.get(node.id()) {
                Some(PickerNodeIdentity::Folder { path, .. }) => path.last().cloned(),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(folders, ["Empty", "Filled"]);
    }
}
