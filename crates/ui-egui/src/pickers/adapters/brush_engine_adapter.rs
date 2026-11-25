//! Brush-specific projection and translation between the engine brush library and the shared asset browser.

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

/// Reconcile group/folder tokens after a successful hierarchy move. Preset item tokens are
/// persistent IDs and therefore remain valid across any group or folder move.
pub(super) fn brush_item_identity_key(id: &str, _group: &str, _path: &[String]) -> PickerIdentityKey {
    PickerIdentityKey::from_parts("brush-item", [id])
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
                    let Some(leaf) = path.last() else { continue };
                    if let Some(owner) = parent_group {
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
                    } else {
                        // Promoting a nested folder to the root turns it into a root group.
                        // Transfer its runtime ID across the identity-kind change so a selected
                        // folder remains selected after the next projection.
                        registry.rekey(
                            PickerIdentityKey::from_parts("brush-folder", std::iter::once(group.as_str()).chain(path.iter().map(String::as_str))),
                            PickerIdentityKey::from_parts("brush-group", [leaf.as_str()]),
                            node.id(),
                        );
                        visit(node.children(), Some(leaf), None, identities, presets, registry);
                    }
                }
                PickerNodeIdentity::Item { key, .. } | PickerNodeIdentity::FolderItem { key, .. } => {
                    // Preset IDs are location-independent, so moving an item never changes its
                    // identity. The registry's original key remains valid after projection.
                    let _ = preset_by_identity(presets, key);
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
#[allow(private_interfaces)]
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
    NewPreset {
        group: String,
        folder: Vec<String>,
        index: usize,
        root_index: Option<usize>,
    },
    /// Append the built-in brush library (a no-op because it is already included).
    AppendDefault,
    /// Import brush presets from a supported preset file.
    Import,
    /// Export selected brush preset occurrences.
    ExportSelected(Vec<String>),
    /// Create a group by moving the explicitly selected presets into it.
    CreateGroup {
        group: String,
        targets: Vec<crate::pickers::browser::selection::SelectionTarget>,
    },
    /// Same shared folder creation intent, with the browser surface whose hierarchy order defines
    /// the placement anchor (the floating Brush Picker has independent transient state).
    CreateGroupInPanel {
        panel: String,
        group: String,
        targets: Vec<crate::pickers::browser::selection::SelectionTarget>,
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
    // Root group order and root card order are persisted together. Project groups in that same
    // sequence instead of appending empty groups after every populated group; otherwise the view
    // and the drop-anchor sequence disagree whenever an empty group sits between populated ones.
    let mut ordered = Vec::with_capacity(groups.len());
    for entry in &hierarchy.root_order {
        let photocraft_engine::BrushPresetRootEntry::Group(name) = entry else { continue };
        if let Some(index) = groups.iter().position(|group| group.view.name == *name) {
            ordered.push(groups.remove(index));
        }
    }
    ordered.extend(groups);
    ordered
}

pub(crate) fn browser_groups(presets: &[BrushPreset], hierarchy: &photocraft_engine::BrushPresetHierarchy) -> Vec<GroupView> {
    brush_group_views(presets, hierarchy).into_iter().map(|group| group.view).collect()
}

/// One coherent snapshot of brush data for the shared browser. These fields are derived together
/// from the compatibility storage model so rendering, selection, and drag planning share a view.
pub(crate) struct BrushBrowserProjection<'a> {
    pub group_views: Vec<BrushGroupView<'a>>,
    pub groups: Vec<GroupView>,
    pub root_sequence: Vec<HierarchyOrderEntry>,
    pub keys: PickerProjectionKeys,
}

pub(crate) fn project_brush_browser<'a>(presets: &'a [BrushPreset], hierarchy: &BrushPresetHierarchy) -> BrushBrowserProjection<'a> {
    let group_views = brush_group_views(presets, hierarchy);
    let groups: Vec<GroupView> = group_views.iter().map(|group| group.view.clone()).collect();
    let root_sequence = brush_root_sequence(presets, hierarchy);
    let keys = brush_projection_keys(&groups, hierarchy);
    BrushBrowserProjection { group_views, groups, root_sequence, keys }
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
            keys.insert(ProjectionNodeRef::Item { group_index, item_index }, brush_item_identity_key(&item.id, &group.name, folder_path), reverse);
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
    Container(String),
    Item { key: String, index: usize },
}

/// A UI sibling target, independent of whether the engine stores it as a root group or a folder.
/// Both are containers with a stable identity and a sibling boundary; only translating the
/// resulting anchor back to an engine command is storage-specific.
#[derive(Clone, Debug, PartialEq, Eq)]
enum BrushSiblingTarget {
    RootGroup { group: String },
    RootItem { key: String },
    Folder { group: String, parent: Vec<String>, leaf: String },
}

impl BrushSiblingTarget {
    fn from_identity(identity: &PickerNodeIdentity) -> Option<Self> {
        match identity {
            PickerNodeIdentity::Group(group) => Some(Self::RootGroup { group: group.clone() }),
            PickerNodeIdentity::Item { group, key } if group.is_empty() => Some(Self::RootItem { key: key.clone() }),
            PickerNodeIdentity::Folder { group, path } => {
                let (leaf, parent) = path.split_last()?;
                Some(Self::Folder { group: group.clone(), parent: parent.to_vec(), leaf: leaf.clone() })
            }
            _ => None,
        }
    }
}

/// Resolve the next sibling boundary for either kind of container. Empty containers are valid
/// targets: their position comes from hierarchy metadata rather than from a preset range.
fn brush_sibling_after_target(
    presets: &[BrushPreset],
    hierarchy: &BrushPresetHierarchy,
    root_sequence: &[HierarchyOrderEntry],
    target: &BrushSiblingTarget,
    exclude_folder: Option<&[String]>,
) -> Option<BrushSiblingAnchor> {
    match target {
        BrushSiblingTarget::RootGroup { group } => {
            let index = root_sequence.iter().position(|entry| matches!(entry, HierarchyOrderEntry::Group(candidate) if candidate == group))?;
            root_sequence.iter().skip(index + 1).find_map(|entry| match entry {
                HierarchyOrderEntry::Group(next) => Some(BrushSiblingAnchor::Container(next.clone())),
                HierarchyOrderEntry::RootItem { key, .. } => {
                    let index = presets.iter().position(|preset| preview_identity(preset) == *key)?;
                    Some(BrushSiblingAnchor::Item { key: key.clone(), index })
                }
                HierarchyOrderEntry::Item(_) => None,
            })
        }
        BrushSiblingTarget::RootItem { key } => {
            let index = root_sequence.iter().position(|entry| matches!(entry, HierarchyOrderEntry::RootItem { key: candidate, .. } if candidate == key))?;
            root_sequence.iter().skip(index + 1).find_map(|entry| match entry {
                HierarchyOrderEntry::Group(next) => Some(BrushSiblingAnchor::Container(next.clone())),
                HierarchyOrderEntry::RootItem { key, .. } => {
                    let index = presets.iter().position(|preset| preview_identity(preset) == *key)?;
                    Some(BrushSiblingAnchor::Item { key: key.clone(), index })
                }
                HierarchyOrderEntry::Item(_) => None,
            })
        }
        BrushSiblingTarget::Folder { group, parent, leaf } => brush_next_sibling_after_folder(presets, hierarchy, group, parent, leaf, exclude_folder),
    }
}

/// Find the first mixed sibling after a folder's entire subtree. Folder order and direct-item
/// order share the preset sequence, so looking only for the next folder skips valid item anchors.
fn brush_next_sibling_after_folder(
    presets: &[BrushPreset],
    hierarchy: &BrushPresetHierarchy,
    group: &str,
    parent: &[String],
    after: &str,
    exclude_folder: Option<&[String]>,
) -> Option<BrushSiblingAnchor> {
    let mut target_path = parent.to_vec();
    target_path.push(after.to_owned());
    let target_end = presets.iter().rposition(|preset| preset.group == group && preset.folder.starts_with(&target_path));
    // Empty folders have no preset range to anchor against. Their persisted `before_item`
    // relation is the authoritative boundary, and folders sharing that boundary retain their
    // metadata order. Resolve that boundary directly instead of treating an empty folder as the
    // end of its parent.
    if target_end.is_none() {
        let siblings: Vec<_> = hierarchy
            .folders
            .iter()
            .filter(|folder| folder.group == group && folder.path.len() == parent.len() + 1 && folder.path.starts_with(parent))
            .collect();
        if let Some(index) = siblings.iter().position(|folder| folder.path == target_path) {
            let anchor = siblings[index].before_item.as_deref();
            if let Some(next) = siblings
                .iter()
                .skip(index + 1)
                .find(|folder| folder.before_item.as_deref() == anchor && !exclude_folder.is_some_and(|path| folder.path.starts_with(path)))
            {
                return next.path.last().cloned().map(BrushSiblingAnchor::Container);
            }
            if let Some(name) = anchor
                && let Some((index, preset)) =
                    presets.iter().enumerate().find(|(_, preset)| preset.group == group && preset.folder == parent && preset.name == name)
            {
                return Some(BrushSiblingAnchor::Item { key: preview_identity(preset), index });
            }
            return None;
        }
    }
    let target_end = target_end?;
    presets.iter().enumerate().skip(target_end + 1).find_map(|(index, preset)| {
        if preset.group != group || exclude_folder.is_some_and(|path| preset.folder.starts_with(path)) {
            return None;
        }
        if preset.folder == parent {
            Some(BrushSiblingAnchor::Item { key: preview_identity(preset), index })
        } else if preset.folder.len() > parent.len() && preset.folder.starts_with(parent) {
            Some(BrushSiblingAnchor::Container(preset.folder[parent.len()].clone()))
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

/// Resolve a folder's insertion boundary in the group's flat preset vector. Empty folders have
/// no descendant range, so use their persisted card anchor (or the next sibling's boundary).
fn brush_folder_boundary_index(presets: &[BrushPreset], hierarchy: &BrushPresetHierarchy, group: &str, parent: &[String], leaf: &str) -> Option<usize> {
    let mut path = parent.to_vec();
    path.push(leaf.to_owned());
    if let Some(index) = presets.iter().position(|preset| preset.group == group && preset.folder.starts_with(&path)) {
        return Some(index);
    }

    let siblings = hierarchy
        .folders
        .iter()
        .filter(|folder| folder.group == group && folder.path.len() == parent.len() + 1 && folder.path.starts_with(parent))
        .collect::<Vec<_>>();
    let target = siblings.iter().position(|folder| folder.path == path)?;
    let direct_anchor = |name: &str| presets.iter().position(|preset| preset.group == group && preset.folder == parent && preset.name == name);
    if let Some(anchor) = siblings[target].before_item.as_deref().and_then(direct_anchor) {
        return Some(anchor);
    }
    siblings.iter().skip(target + 1).find_map(|folder| {
        presets
            .iter()
            .position(|preset| preset.group == group && preset.folder.starts_with(&folder.path))
            .or_else(|| folder.before_item.as_deref().and_then(direct_anchor))
    })
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
    let source_name = source_item.map(|key| preset_by_identity(presets, key).map(|preset| preset.id.clone()).ok_or_else(|| "The dragged Brush no longer exists.".to_owned())).transpose()?;
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
        DropPosition::Before { target } | DropPosition::After { target }
            if matches!(
                identities.get(target).and_then(BrushSiblingTarget::from_identity),
                Some(BrushSiblingTarget::RootGroup { .. } | BrushSiblingTarget::RootItem { .. })
            ) =>
        {
            let after = matches!(&intent.position, DropPosition::After { .. });
            root_sibling_index(root_sequence, &intent.source, Some(target), after, identities)
        }
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
                        let sibling_target = identities
                            .get(target)
                            .and_then(BrushSiblingTarget::from_identity)
                            .ok_or_else(|| "The Brush drop target is not a sibling container.".to_owned())?;
                        let BrushSiblingTarget::Folder { leaf, .. } = &sibling_target else {
                            return Err("The Brush drop target is not a folder.".to_owned());
                        };
                        let before_index = if after {
                            match brush_sibling_after_target(presets, hierarchy, root_sequence, &sibling_target, None) {
                                Some(BrushSiblingAnchor::Container(next)) => {
                                    brush_folder_boundary_index(presets, hierarchy, &target_group, &target_parent, &next)
                                }
                                Some(BrushSiblingAnchor::Item { index, .. }) => Some(index),
                                None => None,
                            }
                        } else {
                            brush_folder_boundary_index(presets, hierarchy, &target_group, &target_parent, leaf)
                                .or_else(|| Some(presets.iter().filter(|preset| preset.group == target_group).count()))
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
                        let sibling_target = identities
                            .get(target)
                            .and_then(BrushSiblingTarget::from_identity)
                            .ok_or_else(|| "The Brush drop target is not a sibling container.".to_owned())?;
                        let (before, before_item) = if after {
                            match brush_sibling_after_target(
                                presets,
                                hierarchy,
                                root_sequence,
                                &sibling_target,
                                (!source_folder.is_empty() && source_group == &target_group).then_some(source_folder.as_slice()),
                            ) {
                                Some(BrushSiblingAnchor::Container(next)) => (Some(next), None),
                                Some(BrushSiblingAnchor::Item { key, .. }) => (None, Some(resolve_before_item_name(presets, &key)?)),
                                None => (None, None),
                            }
                        } else {
                            let Some(BrushSiblingTarget::Folder { leaf, .. }) =
                                BrushSiblingTarget::from_identity(identities.get(target).ok_or_else(|| "The Brush drop target no longer exists.".to_owned())?)
                            else {
                                return Err("The Brush drop target is not a folder.".to_owned());
                            };
                            (Some(leaf), None)
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

/// Translate a canonical drag into the ordered engine operations for every selected move root.
/// The brush command API mutates one preset/group at a time, so plan against a staged Session and
/// refresh occurrence identities after each command. This preserves the shared browser's batch
/// semantics, including reverse execution for after/prepend drops.
pub(super) fn brush_drop_intent_to_actions(
    presets: &[BrushPreset],
    hierarchy: &BrushPresetHierarchy,
    identities: &BTreeMap<PickerId, PickerNodeIdentity>,
    intent: &DropMoveIntent,
) -> Result<Vec<Action>, String> {
    use crate::pickers::browser::hierarchy_utils::hierarchy_move_sources;

    #[derive(Clone)]
    enum StableNode {
        Group(String),
        Folder { group: String, path: Vec<String> },
        Item { name: String, group: String, folder: Vec<String> },
    }

    let requested = if intent.selection.is_empty() { vec![intent.source.clone()] } else { intent.selection.clone() };
    let mut relevant = requested.clone();
    match &intent.position {
        DropPosition::Before { target } | DropPosition::After { target } => relevant.push(target.clone()),
        DropPosition::PrependTo { group } => relevant.push(group.clone()),
        DropPosition::AppendTo { container: ContainerId::Group(group) } => relevant.push(group.clone()),
        DropPosition::AppendTo { container: ContainerId::Root } => {}
    }
    relevant.sort();
    relevant.dedup();

    let mut stable = BTreeMap::<PickerId, StableNode>::new();
    for id in relevant {
        let identity = identities.get(&id).ok_or_else(|| "A selected Brush or drop target no longer exists.".to_owned())?;
        let node = match identity {
            PickerNodeIdentity::Group(group) => StableNode::Group(group.clone()),
            PickerNodeIdentity::Folder { group, path } => StableNode::Folder { group: group.clone(), path: path.clone() },
            PickerNodeIdentity::Item { key, .. } | PickerNodeIdentity::FolderItem { key, .. } => {
                let preset = preset_by_identity(presets, key).ok_or_else(|| "A selected Brush no longer exists.".to_owned())?;
                StableNode::Item { name: preset.name.clone(), group: preset.group.clone(), folder: preset.folder.clone() }
            }
        };
        stable.insert(id, node);
    }

    let covers = |ancestor: &StableNode, child: &StableNode| match (ancestor, child) {
        (StableNode::Group(group), StableNode::Folder { group: child_group, .. }) | (StableNode::Group(group), StableNode::Item { group: child_group, .. }) => {
            group == child_group
        }
        (StableNode::Folder { group, path }, StableNode::Folder { group: child_group, path: child_path }) => {
            group == child_group && child_path.starts_with(path) && child_path.len() > path.len()
        }
        (StableNode::Folder { group, path }, StableNode::Item { group: child_group, folder, .. }) => group == child_group && folder.starts_with(path),
        _ => false,
    };

    let roots = requested
        .iter()
        .filter(|id| {
            let Some(node) = stable.get(*id) else { return false };
            !requested.iter().any(|other| other != *id && stable.get(other).is_some_and(|ancestor| covers(ancestor, node)))
        })
        .cloned()
        .collect::<Vec<_>>();
    let ordered = hierarchy_move_sources(&roots, &intent.position);

    let mut staged = photocraft_engine::Session::new();
    staged.tools.presets = presets.to_vec();
    staged.tools.preset_hierarchy = hierarchy.clone();
    let mut actions = Vec::with_capacity(ordered.len());

    for source in ordered {
        let mut current_identities = BTreeMap::new();
        for (id, node) in &stable {
            let identity = match node {
                StableNode::Group(group) => PickerNodeIdentity::Group(group.clone()),
                StableNode::Folder { group, path } => PickerNodeIdentity::Folder { group: group.clone(), path: path.clone() },
                StableNode::Item { name, group, folder } => {
                    let preset = staged
                        .tools
                        .presets
                        .iter()
                        .find(|preset| preset.name == *name && preset.group == *group && preset.folder == *folder)
                        .ok_or_else(|| format!("The Brush `{name}` no longer exists while planning the move."))?;
                    if is_ambiguous_preset_name(&staged.tools.presets, name) {
                        return Err(format!("Cannot move Brush `{name}` because multiple presets share that name."));
                    }
                    let key = preview_identity(preset);
                    if preset.folder.is_empty() {
                        PickerNodeIdentity::Item { group: preset.group.clone(), key }
                    } else {
                        PickerNodeIdentity::FolderItem { group: preset.group.clone(), path: preset.folder.clone(), key }
                    }
                }
            };
            current_identities.insert(id.clone(), identity);
        }
        let root_sequence = brush_root_sequence(&staged.tools.presets, &staged.tools.preset_hierarchy);
        let single_intent = DropMoveIntent { source: source.clone(), selection: Vec::new(), position: intent.position.clone() };
        let action = brush_drop_intent_to_action(&staged.tools.presets, &staged.tools.preset_hierarchy, &current_identities, &root_sequence, &single_intent)?;
        let result = execute_staged_brush_move(&mut staged, &action)?;
        if let Some(node) = stable.get_mut(&source) {
            match node {
                StableNode::Item { group, folder, .. } => {
                    if let Some(next_group) = result.get("group").and_then(Value::as_str) {
                        *group = next_group.to_owned();
                    }
                    if let Some(next_folder) = result.get("folder").and_then(Value::as_array) {
                        *folder = next_folder.iter().filter_map(Value::as_str).map(str::to_owned).collect();
                    }
                }
                StableNode::Group(_) | StableNode::Folder { .. } => {
                    if let (Some(next_group), Some(next_folder)) = (result.get("group").and_then(Value::as_str), result.get("folder").and_then(Value::as_array))
                    {
                        let path = next_folder.iter().filter_map(Value::as_str).map(str::to_owned).collect::<Vec<_>>();
                        *node =
                            if path.is_empty() { StableNode::Group(next_group.to_owned()) } else { StableNode::Folder { group: next_group.to_owned(), path } };
                    }
                }
            }
        }
        actions.push(action);
    }
    Ok(actions)
}

fn execute_staged_brush_move(session: &mut photocraft_engine::Session, action: &Action) -> Result<Value, String> {
    let (command, params) = match action {
        Action::Move { name, group, folder, index, root_index } => {
            let mut params = json!({ "name": name, "group": group });
            if let Some(folder) = folder {
                params["folder"] = json!(folder);
            }
            if let Some(index) = index {
                params["index"] = json!(index);
            }
            if let Some(index) = root_index {
                params["rootIndex"] = json!(index);
            }
            ("brush.presets.move", params)
        }
        Action::MoveBefore { name, group, folder, before } => {
            let mut params = json!({ "name": name, "group": group, "before": before });
            if let Some(folder) = folder {
                params["folder"] = json!(folder);
            }
            ("brush.presets.move", params)
        }
        Action::MoveGroup { group, folder, before, root_index } => {
            let mut params = json!({ "group": group });
            if !folder.is_empty() {
                params["folder"] = json!(folder);
            }
            if let Some(before) = before {
                params["before"] = json!(before);
            }
            if let Some(index) = root_index {
                params["rootIndex"] = json!(index);
            }
            ("brush.presets.moveGroup", params)
        }
        Action::MoveInto { group, folder, into, before, before_item, root_index } => {
            let into = into.as_ref().map_or_else(|| json!({}), |(group, folder)| json!({ "group": group, "folder": folder }));
            let mut params = json!({ "group": group, "into": into });
            if !folder.is_empty() {
                params["folder"] = json!(folder);
            }
            if let Some(before) = before {
                params["before"] = json!(before);
            }
            if let Some(before_item) = before_item {
                params["beforeItem"] = json!(before_item);
            }
            if let Some(index) = root_index {
                params["rootIndex"] = json!(index);
            }
            ("brush.presets.moveGroup", params)
        }
        _ => return Err("The Brush drag planner produced a non-move action.".to_owned()),
    };
    session.execute(command, params).map_err(|error| error.to_string())
}

#[allow(private_interfaces)]
pub fn browser_actions_to_actions(
    presets: &[BrushPreset],
    hierarchy: &BrushPresetHierarchy,
    browser_actions: Vec<BrowserAction>,
    selection: Option<&crate::pickers::state::BrowserSelectionSnapshot>,
) -> Vec<Action> {
    browser_actions
        .into_iter()
        .filter_map(|action| match action {
            BrowserAction::SelectItem { key, .. } => preset_by_identity(presets, &key).map(|preset| Action::Select(preset.id.clone())),
            BrowserAction::ActivateItem { key, .. } => preset_by_identity(presets, &key).map(|preset| Action::Choose(preset.id.clone())),
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
            BrowserAction::New | BrowserAction::NewNamed(_) => {
                use crate::pickers::browser::selection::SelectionTarget;
                let target = selection.and_then(|selection| selection.targets.last());
                let root_sequence = brush_root_sequence(presets, hierarchy);
                let (group, folder, index) = match target {
                    Some(SelectionTarget::Group(group)) => {
                        let group = group_key(presets, group);
                        let index = brush_folder_insertion_index(presets, &group, &[]);
                        (group, Vec::new(), index)
                    }
                    Some(SelectionTarget::Folder { group, path }) => {
                        let group = group_key(presets, group);
                        let index = brush_folder_insertion_index(presets, &group, path);
                        (group, path.clone(), index)
                    }
                    Some(SelectionTarget::Item { key, .. }) => {
                        if let Some(preset) = preset_by_identity(presets, key) {
                            let group = preset.group.clone();
                            let folder = preset.folder.clone();
                            // The engine's move index is relative to every preset in the group,
                            // not just the selected card's folder.
                            let index = presets.iter().filter(|item| item.group == group).position(|item| item.name == preset.name).unwrap_or(0) + 1;
                            (group, folder, index)
                        } else {
                            (String::new(), Vec::new(), presets.iter().filter(|preset| preset.group.is_empty() && preset.folder.is_empty()).count())
                        }
                    }
                    None => (String::new(), Vec::new(), presets.iter().filter(|preset| preset.group.is_empty() && preset.folder.is_empty()).count()),
                };
                let root_index = match target {
                    Some(SelectionTarget::Item { key, .. }) if group.is_empty() && folder.is_empty() => root_sequence
                        .iter()
                        .position(|entry| matches!(entry, HierarchyOrderEntry::RootItem { key: candidate, .. } if candidate == key))
                        .map(|index| index + 1),
                    None if group.is_empty() && folder.is_empty() => Some(root_sequence.len()),
                    _ => None,
                };
                Some(Action::NewPreset { group, folder, index, root_index })
            }
            BrowserAction::AppendDefault => Some(Action::AppendDefault),
            BrowserAction::Import => Some(Action::Import),
            BrowserAction::ExportSelected(keys) => Some(Action::ExportSelected(keys)),
            BrowserAction::CreateGroup(group) => {
                Some(Action::CreateGroup { group, targets: selection.map(|selection| selection.targets.clone()).unwrap_or_default() })
            }
            BrowserAction::Drop(_, _) | BrowserAction::NewGroup | BrowserAction::ExportAll(_) | BrowserAction::CopyItem { .. } => None,
        })
        .collect()
}

/// Convert shared browser actions while retaining the panel whose hierarchy state supplied the
/// selection. The legacy conversion defaults to the docked Brushes tab for non-picker callers.
pub fn browser_actions_to_actions_for_panel(
    presets: &[BrushPreset],
    hierarchy: &BrushPresetHierarchy,
    panel: &str,
    browser_actions: Vec<BrowserAction>,
    selection: Option<&crate::pickers::state::BrowserSelectionSnapshot>,
) -> Vec<Action> {
    browser_actions_to_actions(presets, hierarchy, browser_actions, selection)
        .into_iter()
        .map(|action| match action {
            Action::CreateGroup { group, targets } => Action::CreateGroupInPanel { panel: panel.to_owned(), group, targets },
            action => action,
        })
        .collect()
}

/// Creation actions need the active single-card selection too. The shared browser snapshot only
/// contains explicitly selected browser IDs and is absent for the ordinary one-card selection.
pub(crate) fn active_brush_selection(state: &crate::pickers::state::PresetUi, panel: &str) -> Option<crate::pickers::state::BrowserSelectionSnapshot> {
    use crate::pickers::browser::selection::{BrowserSelection, SelectionTarget};

    if let Some(key) = state.selected.get(panel) {
        let group = state.selected_group.get(panel).cloned()?;
        let path = state.selected_group_path.get(panel).cloned().unwrap_or_default();
        return Some(BrowserSelection { targets: vec![SelectionTarget::Item { group, path, key: key.clone() }], ..Default::default() });
    }
    let group = state.selected_group.get(panel)?.clone();
    let path = state.selected_group_path.get(panel).cloned().unwrap_or_default();
    Some(BrowserSelection {
        targets: vec![if path.is_empty() { SelectionTarget::Group(group) } else { SelectionTarget::Folder { group, path } }],
        ..Default::default()
    })
}

pub fn preview_identity(preset: &BrushPreset) -> String {
    preset.id.clone()
}

/// Group-relative insertion boundary immediately after the selected folder's contents.
/// The engine's `brush.presets.move` index is group-wide, not folder-relative.
pub(crate) fn brush_folder_insertion_index(presets: &[BrushPreset], group: &str, folder: &[String]) -> usize {
    if folder.is_empty() {
        return presets.iter().filter(|preset| preset.group == group).count();
    }
    let last = presets.iter().enumerate().filter(|(_, preset)| preset.group == group && preset.folder.starts_with(folder)).map(|(index, _)| index).max();
    match last {
        Some(last) => presets[..=last].iter().filter(|preset| preset.group == group).count(),
        None => presets.iter().filter(|preset| preset.group == group).count(),
    }
}

/// Resolve a browser's opaque occurrence key back to the exact borrowed preset.
pub fn preset_by_identity<'a>(presets: &'a [BrushPreset], key: &str) -> Option<&'a BrushPreset> {
    presets.iter().find(|preset| preset.id == key)
}

/// Whether a name-based engine command cannot identify one preset occurrence unambiguously.
pub fn is_ambiguous_preset_name(presets: &[BrushPreset], name: &str) -> bool {
    presets.iter().filter(|preset| preset.name.eq_ignore_ascii_case(name)).take(2).count() > 1
}

pub(super) fn occurrence_name(presets: &[BrushPreset], key: &str) -> Option<String> {
    preset_by_identity(presets, key).map(|preset| preset.name.clone())
}

/// Resolve a stable browser occurrence key to the engine's stable preset ID.
fn resolve_before_item_name(presets: &[BrushPreset], key: &str) -> Result<String, String> {
    preset_by_identity(presets, key).map(|preset| preset.id.clone()).ok_or_else(|| "Cannot position the group: the destination brush no longer exists.".to_string())
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

fn set_current_preset(app: &mut PhotocraftApp, id: &str) {
    match app.run("tools.setBrush", json!({ "presetId": id })) {
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
    for panel in ["brushes", "brush-picker"] {
        let from_key = format!("{panel}/{from}");
        let to_key = format!("{panel}/{to}");
        let moving_root_group_into_folder = !from.contains('/') && to.contains('/');
        let promoting_folder_to_root_group = from.contains('/') && !to.contains('/');
        let was_group_expanded = app.ui.presets_ui.expanded_groups.remove(&from_key);
        let was_folder_collapsed = app.ui.presets_ui.toggled.remove(&from_key);

        let remapped_folders: Vec<_> = app
            .ui
            .presets_ui
            .toggled
            .iter()
            .filter_map(|key| key.strip_prefix(&from_key).filter(|tail| tail.starts_with('/')).map(|tail| format!("{to_key}{tail}")))
            .collect();
        app.ui.presets_ui.toggled.retain(|key| !key.strip_prefix(&from_key).is_some_and(|tail| tail.starts_with('/')));
        app.ui.presets_ui.toggled.extend(remapped_folders);

        if moving_root_group_into_folder {
            // A closed root group becomes a closed folder; an open root group remains open.
            if !was_group_expanded {
                app.ui.presets_ui.toggled.insert(to_key);
            }
        } else if promoting_folder_to_root_group {
            // A closed folder becomes a closed root group; an open folder becomes expanded.
            if !was_folder_collapsed {
                app.ui.presets_ui.expanded_groups.insert(to_key);
            }
        } else {
            if was_group_expanded {
                app.ui.presets_ui.expanded_groups.insert(to_key.clone());
            }
            if was_folder_collapsed {
                app.ui.presets_ui.toggled.insert(to_key);
            }
        }
    }
}

fn remove_brush_group_expansion(app: &mut PhotocraftApp, prefix: &str) {
    for panel in ["brushes", "brush-picker"] {
        let prefix = format!("{panel}/{prefix}");
        app.ui.presets_ui.layout.toggled.retain(|key| key != &prefix && !key.strip_prefix(&prefix).is_some_and(|tail| tail.starts_with('/')));
        app.ui.presets_ui.expanded_groups.retain(|key| key != &prefix && !key.strip_prefix(&prefix).is_some_and(|tail| tail.starts_with('/')));
    }
}

/// Expand the selected destination and each ancestor before opening the new-preset dialog.
pub(crate) fn expand_brush_destination(app: &mut PhotocraftApp, group: &str, folder: &[String]) {
    let root_key = crate::pickers::state::escape_hierarchy_key_part(group);
    for panel in ["brushes", "brush-picker"] {
        app.ui.presets_ui.expanded_groups.insert(format!("{panel}/{root_key}"));
        for depth in 1..=folder.len() {
            let path = folder[..depth].iter().map(|part| crate::pickers::state::escape_hierarchy_key_part(part)).collect::<Vec<_>>().join("/");
            app.ui.presets_ui.layout.toggled.remove(&format!("{panel}/{root_key}/{path}"));
        }
    }
}

#[derive(Clone)]
struct BrushSelectionNode {
    target: crate::pickers::browser::selection::SelectionTarget,
    group: String,
    folder: Vec<String>,
    preset: Option<BrushPreset>,
}

fn brush_selection_node(presets: &[BrushPreset], target: &crate::pickers::browser::selection::SelectionTarget) -> Option<BrushSelectionNode> {
    use crate::pickers::browser::selection::SelectionTarget;
    let (group, folder, preset) = match target {
        SelectionTarget::Group(label) => (group_key(presets, label), Vec::new(), None),
        SelectionTarget::Folder { group, path } => (group_key(presets, group), path.clone(), None),
        SelectionTarget::Item { key, .. } => {
            let preset = preset_by_identity(presets, key)?.clone();
            (preset.group.clone(), preset.folder.clone(), Some(preset))
        }
    };
    Some(BrushSelectionNode { target: target.clone(), group, folder, preset })
}

fn next_brush_sibling_anchor(presets: &[BrushPreset], hierarchy: &BrushPresetHierarchy, node: &BrushSelectionNode) -> (Option<String>, Option<String>) {
    use crate::pickers::browser::selection::SelectionTarget;
    let parent = &node.folder;
    if let SelectionTarget::Folder { path, .. } = &node.target {
        let Some(leaf) = path.last() else { return (None, None) };
        let siblings = photocraft_engine::brush_preset_cmds::subfolder_order(presets, &node.group, &path[..path.len() - 1]);
        let Some(index) = siblings.iter().position(|name| name == leaf) else { return (None, None) };
        if let Some(next) = siblings.get(index + 1) {
            return (Some(next.clone()), None);
        }
        let mut selected_path = path.clone();
        let selected_anchor =
            hierarchy.folders.iter().find(|folder| folder.group == node.group && folder.path == selected_path).and_then(|folder| folder.before_item.clone());
        if let Some(anchor) = selected_anchor {
            let next_with_same_anchor = siblings.iter().skip(index + 1).find(|name| {
                selected_path.pop();
                selected_path.push((*name).clone());
                hierarchy
                    .folders
                    .iter()
                    .any(|folder| folder.group == node.group && folder.path == selected_path && folder.before_item.as_deref() == Some(anchor.as_str()))
            });
            return next_with_same_anchor.cloned().map_or((None, Some(anchor.clone())), |next| (Some(next), None));
        }
        return (None, None);
    }
    let Some(preset) = &node.preset else { return (None, None) };
    let direct_items: Vec<&BrushPreset> = presets.iter().filter(|item| item.group == node.group && item.folder == *parent).collect();
    let Some(index) = direct_items.iter().position(|item| item.name == preset.name) else { return (None, None) };
    let next_item = direct_items.get(index + 1).map(|item| item.name.clone());
    let folders = photocraft_engine::brush_preset_cmds::subfolder_order(presets, &node.group, parent);
    for folder in folders {
        let mut path = parent.clone();
        path.push(folder.clone());
        let anchor = hierarchy
            .folders
            .iter()
            .find(|candidate| candidate.group == node.group && candidate.path == path)
            .and_then(|candidate| candidate.before_item.as_ref());
        if anchor == next_item.as_ref() {
            return (Some(folder), None);
        }
    }
    (None, next_item)
}

fn create_brush_folder(app: &mut PhotocraftApp, panel: &str, name: &str, targets: &[crate::pickers::browser::selection::SelectionTarget]) {
    // Item identities are occurrence keys tied to the addresses in the live preset vector. Resolve
    // them before cloning that vector; cloning first invalidates every selected card key.
    use crate::pickers::browser::selection::BrowserSelection;
    use crate::pickers::state::HierarchyOrderEntry as Entry;

    let report_plan_error = |app: &mut PhotocraftApp, message: String| {
        app.ui.status = message;
        app.ui.status_error = true;
    };

    if targets.is_empty() {
        let new_group = match app.run("brush.presets.createGroup", json!({"group": name})) {
            Ok(result) => result["group"].as_str().unwrap_or(name).to_owned(),
            Err(error) => {
                app.ui.status = error;
                app.ui.status_error = true;
                return;
            }
        };
        expand_brush_destination(app, &new_group, &[]);
        for panel in ["brushes", "brush-picker"] {
            app.ui.presets_ui.clear_browser_selection(panel);
            app.ui.presets_ui.clear_selected(panel);
            app.ui.presets_ui.selected_group.insert(panel.to_owned(), new_group.clone());
            app.ui.presets_ui.selected_group_path.insert(panel.to_owned(), Vec::new());
        }
        return;
    }

    // Placement and selected-ancestor normalization belong to the shared asset browser. Brush
    // adapter only translates that canonical plan into the engine's group/folder commands.
    let selection = BrowserSelection { targets: targets.to_vec(), ..Default::default() };
    let Some(plan) = crate::pickers::browser::browser_actions::folder_creation_plan(&app.ui.presets_ui, panel, &selection, None) else {
        report_plan_error(app, "Could not determine the selected brush hierarchy position.".into());
        return;
    };
    let single = plan.after;
    let mut nodes = if single {
        targets.iter().filter_map(|target| brush_selection_node(&app.session.tools.presets, target)).collect::<Vec<_>>()
    } else {
        plan.selected.iter().filter_map(|(target, _)| brush_selection_node(&app.session.tools.presets, target)).collect::<Vec<_>>()
    };
    let expected_nodes = if single { targets.len() } else { plan.selected.len() };
    if nodes.len() != expected_nodes {
        report_plan_error(app, "One or more selected brush hierarchy nodes could not be resolved; no folder was created.".into());
        return;
    }
    // Keep root anchors from the live preset vector before cloning it for staged command planning.
    let root_sequence = brush_root_sequence(&app.session.tools.presets, &app.session.tools.preset_hierarchy);
    let presets = app.session.tools.presets.clone();
    let hierarchy = app.session.tools.preset_hierarchy.clone();
    let parent = plan.parent.clone();
    let mut before = None;
    let mut before_item = None;
    let mut root_index = None;
    if single {
        let Some(node) = nodes.first() else {
            report_plan_error(app, "The selected brush or folder no longer exists.".into());
            return;
        };
        if parent.is_empty() {
            let sequence = &root_sequence;
            let Some(entry) = plan.before.as_ref() else {
                report_plan_error(app, "Could not resolve the selected root position for the new brush group.".into());
                return;
            };
            let root_entry = match entry {
                Entry::Group(group) => HierarchyOrderEntry::Group(group.clone()),
                Entry::Item(key) => sequence
                    .iter()
                    .find(|candidate| matches!(candidate, HierarchyOrderEntry::RootItem { key: root_key, .. } if root_key == key))
                    .cloned()
                    .unwrap_or_else(|| entry.clone()),
                Entry::RootItem { group, key } => HierarchyOrderEntry::RootItem { group: group.clone(), key: key.clone() },
            };
            root_index = Some(sequence.iter().position(|candidate| candidate == &root_entry).map_or(sequence.len(), |index| index + usize::from(plan.after)));
        } else {
            (before, before_item) = next_brush_sibling_anchor(&presets, &hierarchy, node);
        }
        // A single selected node is only the sibling-position anchor; it is not moved into the folder.
        nodes.clear();
    } else {
        let Some(entry) = plan.before.as_ref() else {
            report_plan_error(app, "Could not resolve the selected position for the new brush folder.".into());
            return;
        };
        if parent.is_empty() {
            let sequence = &root_sequence;
            let root_entry = match entry {
                Entry::Group(group) => HierarchyOrderEntry::Group(group.clone()),
                Entry::Item(key) => sequence
                    .iter()
                    .find(|candidate| matches!(candidate, HierarchyOrderEntry::RootItem { key: root_key, .. } if root_key == key))
                    .cloned()
                    .unwrap_or_else(|| entry.clone()),
                Entry::RootItem { group, key } => HierarchyOrderEntry::RootItem { group: group.clone(), key: key.clone() },
            };
            root_index = Some(sequence.iter().position(|candidate| candidate == &root_entry).unwrap_or(sequence.len()));
        } else {
            match entry {
                Entry::Group(branch) => before = Some(branch.clone()),
                Entry::Item(key) => match resolve_before_item_name(&app.session.tools.presets, key) {
                    Ok(name) => before_item = Some(name),
                    Err(error) => {
                        app.ui.status = error;
                        app.ui.status_error = true;
                        return;
                    }
                },
                Entry::RootItem { .. } => {
                    report_plan_error(app, "A root brush card cannot anchor a nested folder move.".into());
                    return;
                }
            }
        }
    }

    let (new_group, new_folder) = if parent.is_empty() {
        (name.to_owned(), Vec::new())
    } else {
        let group = parent[0].clone();
        let mut folder = parent[1..].to_vec();
        folder.push(name.to_owned());
        (group, folder)
    };

    let mut operations = Vec::<Value>::new();
    if parent.is_empty() {
        operations.push(json!({"id":"brush.presets.createGroup", "params":{"group":name}}));
    } else {
        operations.push(json!({"id":"brush.presets.createFolder", "params":{"group":new_group, "folder":new_folder}}));
    }
    if let Some(index) = root_index {
        operations.push(json!({"id":"brush.presets.moveGroup", "params":{"group":new_group, "rootIndex":index}}));
    } else if !parent.is_empty() && (before.is_some() || before_item.is_some()) {
        let parent_folder = new_folder[..new_folder.len().saturating_sub(1)].to_vec();
        let mut params = json!({"group":new_group, "folder":new_folder, "into":{"group":new_group, "folder":parent_folder}});
        if let Some(before) = before {
            params["before"] = json!(before);
        }
        if let Some(before_item) = before_item {
            params["beforeItem"] = json!(before_item);
        }
        operations.push(json!({"id":"brush.presets.moveGroup", "params":params}));
    }

    for node in nodes {
        match node.target {
            crate::pickers::browser::selection::SelectionTarget::Group(_) | crate::pickers::browser::selection::SelectionTarget::Folder { .. } => {
                let mut params = json!({"group":node.group, "into":{"group":new_group, "folder":new_folder}});
                if !node.folder.is_empty() {
                    params["folder"] = json!(node.folder);
                }
                operations.push(json!({"id":"brush.presets.moveGroup", "params":params}));
            }
            crate::pickers::browser::selection::SelectionTarget::Item { .. } => {
                let Some(preset) = node.preset else { continue };
                if is_ambiguous_preset_name(&app.session.tools.presets, &preset.name) {
                    app.ui.status = format!("Cannot move brush `{}` because multiple presets share that name.", preset.name);
                    app.ui.status_error = true;
                    return;
                }
                operations.push(json!({"id":"brush.presets.move", "params":{"name":preset.name, "group":new_group, "folder":new_folder}}));
            }
        }
    }
    if let Err(error) = app.run("brush.presets.batch", json!({"operations":operations})) {
        app.ui.status = error;
        app.ui.status_error = true;
        return;
    }
    expand_brush_destination(app, &new_group, &new_folder);
    // The brush browser has two surfaces backed by the same library. Creation replaces the
    // previous selection on both, whether the new container is a root group or a nested folder.
    for panel in ["brushes", "brush-picker"] {
        app.ui.presets_ui.clear_browser_selection(panel);
        app.ui.presets_ui.clear_selected(panel);
        app.ui.presets_ui.selected_group.insert(panel.to_owned(), new_group.clone());
        app.ui.presets_ui.selected_group_path.insert(panel.to_owned(), new_folder.clone());
    }
}

/// Turn a preset list's actions into commands.
pub fn apply(app: &mut PhotocraftApp, acts: Vec<Action>) {
    for a in acts {
        match a {
            Action::Select(name) => set_current_preset(app, &name),
            Action::Choose(name) => {
                // The double-click's first click already picked it.
                if app.session.tools.presets.iter().find(|preset| preset.id == name).is_none_or(|preset| app.session.tools.current_preset.as_deref() != Some(preset.name.as_str())) {
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
            Action::NewPreset { group, folder, index, root_index } => {
                crate::brush_preset_dialog::open_at(app, None, Some((group, folder, index, root_index)));
            }
            Action::AppendDefault => {
                run_or_status(app, "brush.presets.appendDefault", json!({}));
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
            Action::CreateGroup { group, targets } => create_brush_folder(app, "brushes", &group, &targets),
            Action::CreateGroupInPanel { panel, group, targets } => create_brush_folder(app, &panel, &group, &targets),
        }
    }
}

#[cfg(test)]
mod hierarchy_tests {
    use super::{
        Action, active_brush_selection, apply, browser_actions_to_actions, browser_actions_to_actions_for_panel, brush_drop_intent_to_action,
        brush_drop_intent_to_actions, brush_group_views, brush_item_identity_key, brush_projection_keys, brush_root_sequence, preview_identity,
        rekey_brush_hierarchy,
    };
    use crate::pickers::browser::asset_browser::BrowserAction;
    use crate::pickers::browser::dragging_model::DropMoveIntent;
    use crate::pickers::browser::identity::{PickerIdentityKey, PickerNodeIdentity, ProjectionNodeRef};
    use crate::pickers::browser::path_identity_model::{ContainerId, DropPosition, HierarchyNode, PickerBrowserHierarchy, PickerId, PickerIdRegistry};
    use crate::pickers::browser::selection::{BrowserSelection, SelectionTarget};
    use crate::pickers::state::HierarchyOrderEntry;
    use photocraft_engine::{BrushPresetFolder, BrushPresetHierarchy};
    use serde_json::json;

    fn preset(name: &str, group: &str, folder: &[&str]) -> photocraft_engine::paint::BrushPreset {
        photocraft_engine::paint::BrushPreset {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            brush: Default::default(),
            builtin: false,
            group: group.into(),
            folder: folder.iter().map(|part| (*part).to_owned()).collect(),
            captures: Default::default(),
        }
    }

    fn app_with_brush_presets(names: &[&str]) -> crate::PhotocraftApp {
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.tools.presets.clear();
        app.session.tools.preset_hierarchy = BrushPresetHierarchy::default();
        for name in names {
            app.run("brush.presets.save", json!({"name":name})).unwrap();
        }
        app
    }

    fn move_brush_to(app: &mut crate::PhotocraftApp, name: &str, group: &str, folder: &[&str]) {
        app.run("brush.presets.move", json!({"name":name, "group":group, "folder":folder})).unwrap();
    }

    #[test]
    fn promoting_a_selected_brush_folder_to_root_preserves_its_group_identity() {
        let presets = vec![preset("Inside", "Parent", &["Folder"]), preset("Other", "Parent", &[])];
        let old_folder_key = PickerIdentityKey::from_parts("brush-folder", ["Parent", "Folder"]);
        let old_item_key = brush_item_identity_key(&presets[0].id, "Parent", &["Folder".into()]);
        let mut registry = PickerIdRegistry::default();
        let folder_id = registry.id_for(old_folder_key.clone());
        let item_id = registry.id_for(old_item_key);
        let identities = std::collections::BTreeMap::from([
            (folder_id.clone(), PickerNodeIdentity::Folder { group: "Parent".into(), path: vec!["Folder".into()] }),
            (item_id.clone(), PickerNodeIdentity::FolderItem { group: "Parent".into(), path: vec!["Folder".into()], key: preview_identity(&presets[0]) }),
        ]);
        let moved = PickerBrowserHierarchy::new(vec![HierarchyNode::group(folder_id.clone(), vec![HierarchyNode::item(item_id.clone())])]).unwrap();
        let mut state = crate::pickers::state::PresetUi::default();
        state.select_browser_node("brushes", folder_id.clone(), false);
        state.select_browser_node("brushes", item_id.clone(), true);

        rekey_brush_hierarchy(&moved, &identities, &presets, &mut registry);

        assert!(state.browser_node_selected("brushes", &folder_id), "the moved group remains selected");
        assert!(state.browser_node_selected("brushes", &item_id), "the selected child remains selected with its group");
        assert_eq!(registry.id_for(PickerIdentityKey::from_parts("brush-group", ["Folder"])), folder_id);
        assert_eq!(
            registry.id_for(brush_item_identity_key(&presets[0].id, "Folder", &[])),
            item_id,
            "the selected group's contents must retain their IDs when the folder becomes a root group"
        );
    }

    #[test]
    fn creating_a_brush_folder_after_a_selected_group_keeps_the_group_as_a_sibling() {
        let mut app = app_with_brush_presets(&[]);
        for group in ["A", "B"] {
            app.run("brush.presets.createGroup", json!({"group":group})).unwrap();
        }
        apply(&mut app, vec![Action::CreateGroup { group: "New".into(), targets: vec![SelectionTarget::Group("A".into())] }]);

        assert!(!app.ui.status_error, "{}", app.ui.status);
        assert_eq!(
            app.session.tools.preset_hierarchy.root_order,
            [
                photocraft_engine::BrushPresetRootEntry::Group("A".into()),
                photocraft_engine::BrushPresetRootEntry::Group("New".into()),
                photocraft_engine::BrushPresetRootEntry::Group("B".into()),
            ]
        );
        assert!(app.session.tools.preset_hierarchy.groups.contains(&"A".to_owned()));
        assert!(app.session.tools.preset_hierarchy.groups.contains(&"New".to_owned()));
    }

    #[test]
    fn creating_a_brush_folder_without_selection_appends_at_root() {
        let mut app = app_with_brush_presets(&[]);
        for group in ["A", "B"] {
            app.run("brush.presets.createGroup", json!({"group":group})).unwrap();
        }

        apply(&mut app, vec![Action::CreateGroup { group: "New".into(), targets: Vec::new() }]);

        assert!(!app.ui.status_error, "{}", app.ui.status);
        assert_eq!(
            app.session.tools.preset_hierarchy.root_order,
            [
                photocraft_engine::BrushPresetRootEntry::Group("A".into()),
                photocraft_engine::BrushPresetRootEntry::Group("B".into()),
                photocraft_engine::BrushPresetRootEntry::Group("New".into()),
            ]
        );
        assert_eq!(app.ui.presets_ui.selected_group.get("brushes").map(String::as_str), Some("New"));
    }

    #[test]
    fn shared_group_creation_retains_the_browser_panel_context() {
        let actions =
            browser_actions_to_actions_for_panel(&[], &BrushPresetHierarchy::default(), "brush-picker", vec![BrowserAction::CreateGroup("New".into())], None);
        assert_eq!(actions, [Action::CreateGroupInPanel { panel: "brush-picker".into(), group: "New".into(), targets: Vec::new() }]);

        let mut app = app_with_brush_presets(&[]);
        apply(&mut app, actions);
        assert!(app.session.tools.preset_hierarchy.groups.contains(&"New".to_owned()));
    }

    #[test]
    fn consecutive_brush_folder_creation_uses_the_created_group_as_the_next_sibling_anchor() {
        let mut app = app_with_brush_presets(&[]);
        for group in ["A", "B"] {
            app.run("brush.presets.createGroup", json!({"group":group})).unwrap();
        }
        app.ui.presets_ui.selected_group.insert("brushes".into(), "A".into());
        app.ui.presets_ui.selected_group_path.insert("brushes".into(), Vec::new());
        let first_selection = active_brush_selection(&app.ui.presets_ui, "brushes").unwrap();
        apply(&mut app, vec![Action::CreateGroup { group: "First".into(), targets: first_selection.targets }]);

        let second_selection = active_brush_selection(&app.ui.presets_ui, "brushes").unwrap();
        assert_eq!(second_selection.targets, [SelectionTarget::Group("First".into())]);
        apply(&mut app, vec![Action::CreateGroup { group: "Second".into(), targets: second_selection.targets }]);

        assert!(!app.ui.status_error, "{}", app.ui.status);
        assert_eq!(
            app.session.tools.preset_hierarchy.root_order,
            [
                photocraft_engine::BrushPresetRootEntry::Group("A".into()),
                photocraft_engine::BrushPresetRootEntry::Group("First".into()),
                photocraft_engine::BrushPresetRootEntry::Group("Second".into()),
                photocraft_engine::BrushPresetRootEntry::Group("B".into()),
            ]
        );
        assert_eq!(app.ui.presets_ui.selected_group.get("brushes").map(String::as_str), Some("Second"));
    }

    #[test]
    fn creating_a_brush_folder_clears_multi_selection_on_both_picker_surfaces() {
        let mut app = app_with_brush_presets(&[]);
        app.run("brush.presets.createGroup", json!({"group":"Ink"})).unwrap();
        let target = SelectionTarget::Group("Ink".into());
        for panel in ["brushes", "brush-picker"] {
            app.ui.presets_ui.select_browser_node(panel, PickerId::from(format!("{panel}-first")), false);
            app.ui.presets_ui.select_browser_node(panel, PickerId::from(format!("{panel}-second")), true);
            app.ui.presets_ui.browser_selection_snapshot.insert(panel.into(), BrowserSelection { targets: vec![target.clone()], ..Default::default() });
        }

        apply(&mut app, vec![Action::CreateGroup { group: "New".into(), targets: vec![target] }]);

        assert!(!app.ui.status_error, "{}", app.ui.status);
        for panel in ["brushes", "brush-picker"] {
            assert!(app.ui.presets_ui.browser_selection.get(panel).is_some_and(|selection| selection.is_empty()), "{panel} node selection should be cleared");
            assert!(!app.ui.presets_ui.browser_selection_anchor.contains_key(panel), "{panel} selection anchor should be cleared");
            assert!(!app.ui.presets_ui.browser_selection_snapshot.contains_key(panel), "{panel} selection snapshot should be cleared");
            assert!(!app.ui.presets_ui.selected.contains_key(panel), "{panel} previous item selection should be discarded");
            assert_eq!(app.ui.presets_ui.selected_group.get(panel).map(String::as_str), Some("New"), "{panel} should select the newly created group");
            assert!(app.ui.presets_ui.selected_group_path.get(panel).is_some_and(Vec::is_empty), "{panel} new group should be selected at the root");
        }
    }

    #[test]
    fn creating_a_brush_folder_after_a_selected_card_keeps_the_card_in_place() {
        let mut app = app_with_brush_presets(&["First", "Second"]);
        app.run("brush.presets.createGroup", json!({"group":"Ink"})).unwrap();
        move_brush_to(&mut app, "First", "Ink", &[]);
        move_brush_to(&mut app, "Second", "Ink", &[]);
        let first = app.session.tools.presets.iter().find(|preset| preset.name == "First").unwrap();
        let target = SelectionTarget::Item { group: "Ink".into(), path: Vec::new(), key: preview_identity(first) };

        apply(&mut app, vec![Action::CreateGroup { group: "New".into(), targets: vec![target] }]);

        assert!(!app.ui.status_error, "{}", app.ui.status);
        let first = app.session.tools.presets.iter().find(|preset| preset.name == "First").unwrap();
        assert_eq!(first.group, "Ink");
        assert!(first.folder.is_empty());
        assert!(app.session.tools.preset_hierarchy.folders.iter().any(|folder| folder.group == "Ink" && folder.path == ["New"]));
    }

    #[test]
    fn creating_a_brush_folder_after_a_root_card_uses_the_shared_sibling_position() {
        let mut app = app_with_brush_presets(&["Root", "Other"]);
        app.run("brush.presets.createGroup", json!({"group":"Group"})).unwrap();
        app.run("brush.presets.move", json!({"name":"Root", "group":"", "folder":[], "rootIndex":0})).unwrap();
        app.session.tools.preset_hierarchy.root_order = vec![
            photocraft_engine::BrushPresetRootEntry::Preset("Root".into()),
            photocraft_engine::BrushPresetRootEntry::Preset("Other".into()),
            photocraft_engine::BrushPresetRootEntry::Group("Group".into()),
        ];
        let root_key = preview_identity(app.session.tools.presets.iter().find(|preset| preset.name == "Root").unwrap());
        let target = SelectionTarget::Item { group: String::new(), path: Vec::new(), key: root_key.clone() };
        assert_eq!(
            brush_root_sequence(&app.session.tools.presets, &app.session.tools.preset_hierarchy).first(),
            Some(&HierarchyOrderEntry::RootItem { group: String::new(), key: root_key })
        );

        apply(&mut app, vec![Action::CreateGroup { group: "New".into(), targets: vec![target] }]);

        assert!(!app.ui.status_error, "{}", app.ui.status);
        // The engine's moveGroup command rebuilds the preset vector, so occurrence keys are
        // intentionally refreshed after the command; locate the same card by its stable name.
        let current_root_key = preview_identity(app.session.tools.presets.iter().find(|preset| preset.name == "Root").unwrap());
        let sequence = brush_root_sequence(&app.session.tools.presets, &app.session.tools.preset_hierarchy);
        let root_index = sequence.iter().position(|entry| matches!(entry, HierarchyOrderEntry::RootItem { key, .. } if key == &current_root_key)).unwrap();
        assert_eq!(sequence.get(root_index + 1), Some(&HierarchyOrderEntry::Group("New".into())));
    }

    #[test]
    fn dragging_multiple_brush_cards_moves_the_whole_selection_in_browser_order() {
        let presets = vec![
            preset("First", "Ink", &[]),
            preset("Second", "Ink", &[]),
            preset("Anchor", "Ink", &[]),
            preset("Imported duplicate", "Other", &[]),
            preset("Imported duplicate", "Elsewhere", &[]),
        ];
        let hierarchy = BrushPresetHierarchy {
            groups: vec!["Ink".into(), "Other".into(), "Elsewhere".into()],
            folders: Vec::new(),
            root_order: vec![
                photocraft_engine::BrushPresetRootEntry::Group("Ink".into()),
                photocraft_engine::BrushPresetRootEntry::Group("Other".into()),
                photocraft_engine::BrushPresetRootEntry::Group("Elsewhere".into()),
            ],
        };
        let first = PickerId::from("first");
        let second = PickerId::from("second");
        let anchor = PickerId::from("anchor");
        let identities = std::collections::BTreeMap::from([
            (first.clone(), PickerNodeIdentity::Item { group: "Ink".into(), key: preview_identity(&presets[0]) }),
            (second.clone(), PickerNodeIdentity::Item { group: "Ink".into(), key: preview_identity(&presets[1]) }),
            (anchor.clone(), PickerNodeIdentity::Item { group: "Ink".into(), key: preview_identity(&presets[2]) }),
        ]);
        let actions = brush_drop_intent_to_actions(
            &presets,
            &hierarchy,
            &identities,
            &DropMoveIntent { source: first.clone(), selection: vec![first, second], position: DropPosition::After { target: anchor } },
        )
        .unwrap();

        let mut app = app_with_brush_presets(&[]);
        app.session.tools.presets = presets;
        app.session.tools.preset_hierarchy = hierarchy;
        apply(&mut app, actions);

        assert!(!app.ui.status_error, "{}", app.ui.status);
        assert_eq!(
            app.session.tools.presets.iter().map(|preset| preset.name.as_str()).collect::<Vec<_>>(),
            ["Anchor", "First", "Second", "Imported duplicate", "Imported duplicate"]
        );
    }

    #[test]
    fn dragging_multiple_brush_cards_from_different_groups_keeps_their_selection_order() {
        let presets = vec![preset("First", "Ink", &[]), preset("Second", "Dry", &[]), preset("Anchor", "", &[])];
        let hierarchy = BrushPresetHierarchy {
            groups: vec!["Ink".into(), "Dry".into()],
            folders: Vec::new(),
            root_order: vec![
                photocraft_engine::BrushPresetRootEntry::Group("Ink".into()),
                photocraft_engine::BrushPresetRootEntry::Group("Dry".into()),
                photocraft_engine::BrushPresetRootEntry::Preset("Anchor".into()),
            ],
        };
        let first = PickerId::from("first");
        let second = PickerId::from("second");
        let anchor = PickerId::from("anchor");
        let identities = std::collections::BTreeMap::from([
            (first.clone(), PickerNodeIdentity::Item { group: "Ink".into(), key: preview_identity(&presets[0]) }),
            (second.clone(), PickerNodeIdentity::Item { group: "Dry".into(), key: preview_identity(&presets[1]) }),
            (anchor.clone(), PickerNodeIdentity::Item { group: String::new(), key: preview_identity(&presets[2]) }),
        ]);
        let actions = brush_drop_intent_to_actions(
            &presets,
            &hierarchy,
            &identities,
            &DropMoveIntent { source: first.clone(), selection: vec![first, second], position: DropPosition::After { target: anchor } },
        )
        .unwrap();

        let mut app = app_with_brush_presets(&[]);
        app.session.tools.presets = presets;
        app.session.tools.preset_hierarchy = hierarchy;
        apply(&mut app, actions);

        assert!(!app.ui.status_error, "{}", app.ui.status);
        assert_eq!(app.session.tools.presets.iter().map(|preset| preset.name.as_str()).collect::<Vec<_>>(), ["Anchor", "First", "Second"]);
        assert!(app.session.tools.presets.iter().all(|preset| preset.group.is_empty()));
    }

    #[test]
    fn dragging_multiple_empty_brush_groups_moves_the_whole_selection_in_browser_order() {
        let groups = ["First", "Second", "Anchor", "Last"];
        let hierarchy = BrushPresetHierarchy {
            groups: groups.iter().map(|group| (*group).to_owned()).collect(),
            folders: Vec::new(),
            root_order: groups.iter().map(|group| photocraft_engine::BrushPresetRootEntry::Group((*group).to_owned())).collect(),
        };
        let ids = groups.iter().map(|group| (group.to_string(), PickerId::from(*group))).collect::<Vec<_>>();
        let identities = ids.iter().map(|(group, id)| (id.clone(), PickerNodeIdentity::Group(group.clone()))).collect::<std::collections::BTreeMap<_, _>>();
        let actions = brush_drop_intent_to_actions(
            &[],
            &hierarchy,
            &identities,
            &DropMoveIntent {
                source: ids[0].1.clone(),
                selection: vec![ids[0].1.clone(), ids[1].1.clone()],
                position: DropPosition::After { target: ids[2].1.clone() },
            },
        )
        .unwrap();

        let mut app = app_with_brush_presets(&[]);
        app.session.tools.preset_hierarchy = hierarchy;
        apply(&mut app, actions);

        assert!(!app.ui.status_error, "{}", app.ui.status);
        assert_eq!(
            app.session.tools.preset_hierarchy.root_order,
            [
                photocraft_engine::BrushPresetRootEntry::Group("Anchor".into()),
                photocraft_engine::BrushPresetRootEntry::Group("First".into()),
                photocraft_engine::BrushPresetRootEntry::Group("Second".into()),
                photocraft_engine::BrushPresetRootEntry::Group("Last".into()),
            ]
        );
    }

    #[test]
    fn creating_a_brush_folder_from_cards_in_sibling_folders_uses_the_shallowest_common_ancestor() {
        let mut app = app_with_brush_presets(&["First", "Second"]);
        app.run("brush.presets.createGroup", json!({"group":"Ink"})).unwrap();
        app.run("brush.presets.createFolder", json!({"group":"Ink", "folder":["Left"]})).unwrap();
        app.run("brush.presets.createFolder", json!({"group":"Ink", "folder":["Right"]})).unwrap();
        move_brush_to(&mut app, "First", "Ink", &["Left"]);
        move_brush_to(&mut app, "Second", "Ink", &["Right"]);
        let first = app.session.tools.presets.iter().find(|preset| preset.name == "First").unwrap();
        let second = app.session.tools.presets.iter().find(|preset| preset.name == "Second").unwrap();
        let targets = vec![
            SelectionTarget::Item { group: "Ink".into(), path: vec!["Left".into()], key: preview_identity(first) },
            SelectionTarget::Item { group: "Ink".into(), path: vec!["Right".into()], key: preview_identity(second) },
        ];

        apply(&mut app, vec![Action::CreateGroup { group: "New".into(), targets }]);

        assert!(!app.ui.status_error, "{}", app.ui.status);
        assert!(
            app.session
                .tools
                .presets
                .iter()
                .filter(|preset| ["First", "Second"].contains(&preset.name.as_str()))
                .all(|preset| preset.group == "Ink" && preset.folder == ["New"])
        );
        assert_eq!(
            app.session
                .tools
                .preset_hierarchy
                .folders
                .iter()
                .filter(|folder| folder.group == "Ink" && folder.path.len() == 1)
                .filter_map(|folder| folder.path.first().map(String::as_str))
                .collect::<Vec<_>>(),
            ["New", "Left", "Right"],
            "the empty source folders remain in their original order after the selected cards move out"
        );
    }

    #[test]
    fn creating_a_brush_folder_from_multiple_cards_uses_the_first_card_as_its_sibling_anchor() {
        let mut app = app_with_brush_presets(&["First", "Second", "InExisting"]);
        app.run("brush.presets.createGroup", json!({"group":"Ink"})).unwrap();
        app.run("brush.presets.createFolder", json!({"group":"Ink", "folder":["Existing"]})).unwrap();
        for name in ["First", "Second"] {
            move_brush_to(&mut app, name, "Ink", &[]);
        }
        move_brush_to(&mut app, "InExisting", "Ink", &["Existing"]);
        let first = app.session.tools.presets.iter().find(|preset| preset.name == "First").unwrap();
        let second = app.session.tools.presets.iter().find(|preset| preset.name == "Second").unwrap();
        let targets = vec![
            SelectionTarget::Item { group: "Ink".into(), path: Vec::new(), key: preview_identity(first) },
            SelectionTarget::Item { group: "Ink".into(), path: Vec::new(), key: preview_identity(second) },
        ];

        apply(&mut app, vec![Action::CreateGroup { group: "New".into(), targets }]);

        assert!(!app.ui.status_error, "{}", app.ui.status);
        assert_eq!(
            app.session
                .tools
                .preset_hierarchy
                .folders
                .iter()
                .filter(|folder| folder.group == "Ink" && folder.path.len() == 1)
                .map(|folder| folder.path[0].as_str())
                .collect::<Vec<_>>(),
            ["New", "Existing"]
        );
        assert!(app.session.tools.presets.iter().filter(|preset| ["First", "Second"].contains(&preset.name.as_str())).all(|preset| preset.folder == ["New"]));
    }

    #[test]
    fn creating_a_brush_folder_from_multiple_cards_keeps_preceding_empty_folder_in_place() {
        let mut app = app_with_brush_presets(&["First", "Second", "Third"]);
        app.run("brush.presets.createGroup", json!({"group":"Ink"})).unwrap();
        for name in ["First", "Second", "Third"] {
            move_brush_to(&mut app, name, "Ink", &[]);
        }
        app.session.tools.preset_hierarchy.folders.push(BrushPresetFolder {
            group: "Ink".into(),
            path: vec!["Empty".into()],
            before_item: Some("First".into()),
        });
        let first = app.session.tools.presets.iter().find(|preset| preset.name == "First").unwrap();
        let second = app.session.tools.presets.iter().find(|preset| preset.name == "Second").unwrap();
        let targets = vec![
            SelectionTarget::Item { group: "Ink".into(), path: Vec::new(), key: preview_identity(first) },
            SelectionTarget::Item { group: "Ink".into(), path: Vec::new(), key: preview_identity(second) },
        ];

        apply(&mut app, vec![Action::CreateGroup { group: "New".into(), targets }]);

        assert!(!app.ui.status_error, "{}", app.ui.status);
        assert_eq!(
            app.session.tools.preset_hierarchy.folders.iter().filter(|folder| folder.group == "Ink").map(|folder| folder.path[0].as_str()).collect::<Vec<_>>(),
            ["Empty", "New"]
        );
        assert_eq!(
            app.session
                .tools
                .preset_hierarchy
                .folders
                .iter()
                .filter(|folder| folder.group == "Ink")
                .map(|folder| folder.before_item.as_deref())
                .collect::<Vec<_>>(),
            [Some("Third"), Some("Third")],
            "folder anchors should skip the selected cards as they move into the new folder"
        );
        assert!(app.session.tools.presets.iter().filter(|preset| ["First", "Second"].contains(&preset.name.as_str())).all(|preset| preset.folder == ["New"]));
    }

    #[test]
    fn creating_a_brush_folder_uses_the_shallowest_named_ancestor_for_nested_selections() {
        let mut app = app_with_brush_presets(&["First", "Second"]);
        app.run("brush.presets.createGroup", json!({"group":"Ink"})).unwrap();
        app.run("brush.presets.createFolder", json!({"group":"Ink", "folder":["Parent"]})).unwrap();
        app.run("brush.presets.createFolder", json!({"group":"Ink", "folder":["Parent", "Left"]})).unwrap();
        app.run("brush.presets.createFolder", json!({"group":"Ink", "folder":["Parent", "Right"]})).unwrap();
        move_brush_to(&mut app, "First", "Ink", &["Parent", "Left"]);
        move_brush_to(&mut app, "Second", "Ink", &["Parent", "Right"]);
        let first = app.session.tools.presets.iter().find(|preset| preset.name == "First").unwrap();
        let second = app.session.tools.presets.iter().find(|preset| preset.name == "Second").unwrap();
        let targets = vec![
            SelectionTarget::Item { group: "Ink".into(), path: first.folder.clone(), key: preview_identity(first) },
            SelectionTarget::Item { group: "Ink".into(), path: second.folder.clone(), key: preview_identity(second) },
        ];

        apply(&mut app, vec![Action::CreateGroup { group: "New".into(), targets }]);

        assert!(!app.ui.status_error, "{}", app.ui.status);
        assert!(
            app.session
                .tools
                .presets
                .iter()
                .filter(|preset| ["First", "Second"].contains(&preset.name.as_str()))
                .all(|preset| preset.group == "Ink" && preset.folder == ["New"])
        );
        assert!(app.session.tools.preset_hierarchy.folders.iter().any(|folder| folder.group == "Ink" && folder.path == ["New"]));
    }

    #[test]
    fn creating_a_brush_folder_from_a_group_and_its_card_moves_only_the_group() {
        let mut app = app_with_brush_presets(&["Inside"]);
        app.run("brush.presets.createGroup", json!({"group":"Ink"})).unwrap();
        move_brush_to(&mut app, "Inside", "Ink", &[]);
        let inside = app.session.tools.presets.iter().find(|preset| preset.name == "Inside").unwrap();
        let targets =
            vec![SelectionTarget::Group("Ink".into()), SelectionTarget::Item { group: "Ink".into(), path: Vec::new(), key: preview_identity(inside) }];

        apply(&mut app, vec![Action::CreateGroup { group: "New".into(), targets }]);

        assert!(!app.ui.status_error, "{}", app.ui.status);
        let inside = app.session.tools.presets.iter().find(|preset| preset.name == "Inside").unwrap();
        assert_eq!(inside.group, "New");
        assert_eq!(inside.folder, ["Ink"]);
        assert_eq!(photocraft_engine::brush_preset_cmds::subfolder_order(&app.session.tools.presets, "New", &[]), ["Ink"]);
    }

    #[test]
    fn creating_a_brush_uses_the_last_selected_card_in_browser_order() {
        let presets = vec![preset("First", "Ink", &[]), preset("Second", "Ink", &[]), preset("Nested", "Ink", &["Fine"])];
        let selection = BrowserSelection {
            targets: vec![
                SelectionTarget::Item { group: "Ink".into(), path: Vec::new(), key: preview_identity(&presets[0]) },
                SelectionTarget::Item { group: "Ink".into(), path: vec!["Fine".into()], key: preview_identity(&presets[2]) },
            ],
            ..Default::default()
        };
        let actions = browser_actions_to_actions(&presets, &BrushPresetHierarchy::default(), vec![BrowserAction::New], Some(&selection));
        assert_eq!(actions, [Action::NewPreset { group: "Ink".into(), folder: vec!["Fine".into()], index: 3, root_index: None }]);
    }

    #[test]
    fn creating_a_brush_after_a_single_selected_card_uses_its_position_before_the_next_folder() {
        let mut app = app_with_brush_presets(&["First", "Second", "Third"]);
        app.run("brush.presets.createGroup", json!({"group":"Ink"})).unwrap();
        for name in ["First", "Second", "Third"] {
            move_brush_to(&mut app, name, "Ink", &[]);
        }
        app.session.tools.preset_hierarchy.folders.push(BrushPresetFolder {
            group: "Ink".into(),
            path: vec!["Folder".into()],
            before_item: Some("Second".into()),
        });
        let first = app.session.tools.presets.iter().find(|preset| preset.name == "First").unwrap();
        app.ui.presets_ui.set_selected_item("brushes", "Ink", &[], &preview_identity(first));

        let selection = active_brush_selection(&app.ui.presets_ui, "brushes").unwrap();
        let actions = browser_actions_to_actions(&app.session.tools.presets, &app.session.tools.preset_hierarchy, vec![BrowserAction::New], Some(&selection));
        assert_eq!(actions, [Action::NewPreset { group: "Ink".into(), folder: Vec::new(), index: 1, root_index: None }]);

        app.run("brush.presets.save", json!({"name":"Created"})).unwrap();
        app.run("brush.presets.move", json!({"name":"Created", "group":"Ink", "folder":[], "index":1})).unwrap();

        let names: Vec<_> = app.session.tools.presets.iter().filter(|preset| preset.group == "Ink").map(|preset| preset.name.as_str()).collect();
        assert_eq!(names, ["First", "Created", "Second", "Third"]);
        assert_eq!(
            app.session.tools.preset_hierarchy.folders.iter().find(|folder| folder.group == "Ink" && folder.path == ["Folder"]).unwrap().before_item.as_deref(),
            Some("Second")
        );
    }

    #[test]
    fn creating_a_root_brush_after_a_card_uses_the_mixed_root_index_before_the_next_group() {
        let presets = vec![preset("First", "", &[]), preset("Second", "", &[])];
        let hierarchy = BrushPresetHierarchy {
            groups: vec!["Folder".into()],
            folders: Vec::new(),
            root_order: vec![
                photocraft_engine::BrushPresetRootEntry::Preset("First".into()),
                photocraft_engine::BrushPresetRootEntry::Group("Folder".into()),
                photocraft_engine::BrushPresetRootEntry::Preset("Second".into()),
            ],
        };
        let selection = BrowserSelection {
            targets: vec![SelectionTarget::Item { group: String::new(), path: Vec::new(), key: preview_identity(&presets[0]) }],
            ..Default::default()
        };

        let actions = browser_actions_to_actions(&presets, &hierarchy, vec![BrowserAction::New], Some(&selection));

        assert_eq!(actions, [Action::NewPreset { group: String::new(), folder: Vec::new(), index: 1, root_index: Some(1) }]);
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
        app.ui.presets_ui.layout.toggled.extend(["brushes/Empty/Child".into(), "brush-picker/Empty/Child".into()]);

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

        for panel in ["brushes", "brush-picker"] {
            assert!(app.ui.presets_ui.layout.toggled.contains(&format!("{panel}/Target/Empty")));
            assert!(app.ui.presets_ui.layout.toggled.contains(&format!("{panel}/Target/Empty/Child")));
            assert!(!app.ui.presets_ui.expanded_groups.contains(&format!("{panel}/Target")));
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

        for panel in ["brushes", "brush-picker"] {
            assert!(!app.ui.presets_ui.expanded_groups.contains(&format!("{panel}/Empty")));
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
        app.ui.presets_ui.expanded_groups.extend(["brushes/Empty".into(), "brush-picker/Empty".into()]);
        app.ui.presets_ui.layout.toggled.extend(["brushes/Empty/Child".into(), "brush-picker/Empty/Child".into()]);

        super::apply(&mut app, vec![Action::Rename(super::Renaming { group: true, name: "Empty".into(), folder: Vec::new(), text: "Renamed".into() })]);
        assert!(app.ui.presets_ui.expanded_groups.contains("brushes/Renamed"));
        assert!(app.ui.presets_ui.expanded_groups.contains("brush-picker/Renamed"));
        assert!(app.ui.presets_ui.layout.toggled.contains("brushes/Renamed/Child"));
        assert!(app.ui.presets_ui.layout.toggled.contains("brush-picker/Renamed/Child"));

        super::apply(&mut app, vec![Action::DeleteGroup { group: "Renamed".into(), folder: Vec::new() }]);
        assert!(!app.ui.presets_ui.layout.toggled.iter().any(|key| key.starts_with("brushes/Renamed") || key.starts_with("brush-picker/Renamed")));
        assert!(!app.ui.presets_ui.expanded_groups.iter().any(|key| key.starts_with("brushes/Renamed") || key.starts_with("brush-picker/Renamed")));
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
    fn dropping_after_an_empty_folder_uses_its_persisted_sibling_boundary() {
        let presets = vec![preset("B", "Parent", &[]), preset("Source card", "Other", &["Source"])];
        let source = PickerId::group_path(["Source"], "Other");
        let target = PickerId::group_path(["Empty"], "Parent");
        let identities = std::collections::BTreeMap::from([
            (source.clone(), PickerNodeIdentity::Folder { group: "Other".into(), path: vec!["Source".into()] }),
            (target.clone(), PickerNodeIdentity::Folder { group: "Parent".into(), path: vec!["Empty".into()] }),
        ]);

        for (following_folder, expected_before, expected_before_item) in [(Some("Following"), Some("Following"), None), (None, None, Some("B"))] {
            let mut folders = vec![BrushPresetFolder { group: "Parent".into(), path: vec!["Empty".into()], before_item: Some("B".into()) }];
            if let Some(name) = following_folder {
                folders.push(BrushPresetFolder { group: "Parent".into(), path: vec![name.into()], before_item: Some("B".into()) });
            }
            let hierarchy = BrushPresetHierarchy { groups: vec!["Parent".into(), "Other".into()], folders, root_order: Vec::new() };
            let action = brush_drop_intent_to_action(
                &presets,
                &hierarchy,
                &identities,
                &[],
                &DropMoveIntent { source: source.clone(), selection: Vec::new(), position: DropPosition::After { target: target.clone() } },
            )
            .unwrap();

            assert!(matches!(action, Action::MoveInto { ref group, ref folder, ref into, ref before, ref before_item, .. }
                if group == "Other"
                    && folder == &["Source".to_owned()]
                    && into.as_ref().is_some_and(|(destination, path)| destination == "Parent" && path.is_empty())
                    && before.as_deref() == expected_before
                    && before_item.as_deref() == expected_before_item));
        }
    }

    #[test]
    fn moving_one_brush_after_an_empty_folder_preserves_other_brush_order() {
        let presets = vec![preset("A", "Parent", &[]), preset("B", "Parent", &[]), preset("C", "Parent", &[]), preset("Source", "Other", &[])];
        let hierarchy = BrushPresetHierarchy {
            groups: vec!["Parent".into(), "Other".into()],
            folders: vec![BrushPresetFolder { group: "Parent".into(), path: vec!["Empty".into()], before_item: Some("B".into()) }],
            root_order: vec![photocraft_engine::BrushPresetRootEntry::Group("Parent".into()), photocraft_engine::BrushPresetRootEntry::Group("Other".into())],
        };
        let source = PickerId::item("Other", &preview_identity(&presets[3]));
        let target = PickerId::group_path(["Empty"], "Parent");
        let identities = std::collections::BTreeMap::from([
            (source.clone(), PickerNodeIdentity::Item { group: "Other".into(), key: preview_identity(&presets[3]) }),
            (target.clone(), PickerNodeIdentity::Folder { group: "Parent".into(), path: vec!["Empty".into()] }),
        ]);
        let actions = brush_drop_intent_to_actions(
            &presets,
            &hierarchy,
            &identities,
            &DropMoveIntent { source, selection: Vec::new(), position: DropPosition::After { target } },
        )
        .unwrap();

        let mut app = app_with_brush_presets(&[]);
        app.session.tools.presets = presets;
        app.session.tools.preset_hierarchy = hierarchy;
        apply(&mut app, actions);

        assert!(!app.ui.status_error, "{}", app.ui.status);
        assert_eq!(
            app.session
                .tools
                .presets
                .iter()
                .filter(|preset| preset.group == "Parent" && preset.folder.is_empty())
                .map(|preset| preset.name.as_str())
                .collect::<Vec<_>>(),
            ["A", "Source", "B", "C"]
        );
        assert_eq!(
            app.session
                .tools
                .preset_hierarchy
                .folders
                .iter()
                .find(|folder| folder.group == "Parent" && folder.path == ["Empty"])
                .unwrap()
                .before_item
                .as_deref(),
            Some("B")
        );
    }

    #[test]
    fn moving_one_brush_before_an_empty_folder_preserves_other_brush_order() {
        let presets = vec![preset("A", "Parent", &[]), preset("B", "Parent", &[]), preset("C", "Parent", &[]), preset("D", "Parent", &[])];
        let hierarchy = BrushPresetHierarchy {
            groups: vec!["Parent".into()],
            folders: vec![BrushPresetFolder { group: "Parent".into(), path: vec!["Empty".into()], before_item: Some("B".into()) }],
            root_order: vec![photocraft_engine::BrushPresetRootEntry::Group("Parent".into())],
        };
        let source = PickerId::item("Parent", &preview_identity(&presets[2]));
        let target = PickerId::group_path(["Empty"], "Parent");
        let identities = std::collections::BTreeMap::from([
            (source.clone(), PickerNodeIdentity::Item { group: "Parent".into(), key: preview_identity(&presets[2]) }),
            (target.clone(), PickerNodeIdentity::Folder { group: "Parent".into(), path: vec!["Empty".into()] }),
        ]);
        let actions = brush_drop_intent_to_actions(
            &presets,
            &hierarchy,
            &identities,
            &DropMoveIntent { source, selection: Vec::new(), position: DropPosition::Before { target } },
        )
        .unwrap();

        let mut app = app_with_brush_presets(&[]);
        app.session.tools.presets = presets;
        app.session.tools.preset_hierarchy = hierarchy;
        apply(&mut app, actions);

        assert!(!app.ui.status_error, "{}", app.ui.status);
        assert_eq!(
            app.session.tools.presets.iter().map(|preset| preset.name.as_str()).collect::<Vec<_>>(),
            ["A", "C", "B", "D"],
            "only dragged card should cross the empty folder's persisted boundary"
        );
        assert_eq!(
            app.session.tools.preset_hierarchy.folders[0].before_item.as_deref(),
            Some("B"),
            "moving a card must not rewrite the empty folder's own anchor"
        );
    }

    #[test]
    fn sibling_target_wrapper_resolves_empty_root_groups_and_nested_folders() {
        let presets = vec![preset("Root card", "", &[]), preset("B", "Parent", &[])];
        let hierarchy = BrushPresetHierarchy {
            groups: vec!["Empty root".into(), "Next root".into(), "Parent".into()],
            folders: vec![
                BrushPresetFolder { group: "Parent".into(), path: vec!["Empty nested".into()], before_item: Some("B".into()) },
                BrushPresetFolder { group: "Parent".into(), path: vec!["Next nested".into()], before_item: Some("B".into()) },
            ],
            root_order: Vec::new(),
        };
        let root_sequence = vec![
            HierarchyOrderEntry::Group("Empty root".into()),
            HierarchyOrderEntry::RootItem { group: String::new(), key: preview_identity(&presets[0]) },
            HierarchyOrderEntry::Group("Next root".into()),
        ];

        let root_target = super::BrushSiblingTarget::from_identity(&PickerNodeIdentity::Group("Empty root".into())).unwrap();
        assert_eq!(
            super::brush_sibling_after_target(&presets, &hierarchy, &root_sequence, &root_target, None),
            Some(super::BrushSiblingAnchor::Item { key: preview_identity(&presets[0]), index: 0 })
        );

        let nested_target =
            super::BrushSiblingTarget::from_identity(&PickerNodeIdentity::Folder { group: "Parent".into(), path: vec!["Empty nested".into()] }).unwrap();
        assert_eq!(
            super::brush_sibling_after_target(&presets, &hierarchy, &root_sequence, &nested_target, None),
            Some(super::BrushSiblingAnchor::Container("Next nested".into()))
        );
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
