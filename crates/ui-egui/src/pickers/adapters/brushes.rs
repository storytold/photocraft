//! Shared brush preset browser used by the Brushes tab and the right-click Brush Picker.

use std::collections::BTreeMap;

use egui::{Color32, RichText};
use photocraft_engine::paint::BrushPreset;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::brush_preview;
use crate::pickers::browser::asset_browser::{
    self, BrowserAction, BrowserAdapter, BrowserCapabilities, BrowserOptions, GroupView, ItemPaintState, ItemView, begin_item_rename,
};
use crate::pickers::browser::dragging_model::DropMoveIntent;
use crate::pickers::browser::hierarchy::{HierarchyGroup, HierarchyNode, hierarchy_tree};
use crate::pickers::browser::hierarchy_utils::{HierarchyDrop, hierarchy_group_drop};
use crate::pickers::browser::identity::{PickerIdentityKey, PickerNodeIdentity, PickerProjectionKeys, ProjectionNodeRef};
use crate::pickers::browser::path_identity_model::{
    ContainerId, DropPosition, HierarchyNode as PickerHierarchyNode, PickerBrowserHierarchy, PickerId, PickerIdRegistry,
};
use crate::pickers::state::{HierarchyOrderEntry, NestedGroupRenameState, PresetUi};
use crate::theme::Tokens;

type BrushDropDestination = (String, Vec<String>, Option<(&'static str, String)>);

/// Reconcile Brush item and folder tokens after a successful hierarchy move. Both identities are
/// qualified by the owning group/path; the stable preset name survives the move and lets us rekey
/// before the authoritative preset vector is updated.
fn brush_item_identity_key(name: &str, group: &str, path: &[String]) -> PickerIdentityKey {
    PickerIdentityKey::from_parts("brush-item", std::iter::once(name).chain(std::iter::once(group)).chain(path.iter().map(String::as_str)))
}

fn rekey_brush_hierarchy(
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
                PickerNodeIdentity::Group(group) => visit(node.children(), Some(group), None, identities, presets, registry),
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
pub(crate) fn run_or_status(app: &mut PhotocraftApp, id: &str, p: Value) -> bool {
    match app.run(id, p) {
        Ok(_) => true,
        Err(e) => {
            app.ui.status = e;
            false
        }
    }
}

/// Clear the compact Texture picker selection when the active brush changes.
pub(crate) fn clear_texture_pattern_selection(app: &mut PhotocraftApp) {
    app.ui.presets_ui.selected.remove("brush-texture-patterns");
}

pub(crate) fn full_uv() -> egui::Rect {
    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0))
}

/// Group label for presets saved without a group.
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

/// How the Brushes tab lists presets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BrushesView {
    /// Rows: tip, size, stroke preview, name.
    #[default]
    List,
    /// Tip thumbnails with their sizes.
    Grid,
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

/// Brushes panel view state (serde, so the control channel can read and drive it).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[derive(Default)]
pub struct BrushesPanelState {
    /// Collapsed groups: a group's label, or for a nested group the root label and its path joined
    /// with `/` ([`crate::brushes_tab::group_view_key`]).
    pub collapsed: Vec<String>,
    /// Case-insensitive name filter.
    pub filter: String,
    /// Legacy two-state projection for serialized/control compatibility; shared picker display
    /// preferences are authoritative once present.
    pub view: BrushesView,
    pub renaming: Option<Renaming>,
}

/// What a preset list does after drawing (the presets are borrowed while it draws).
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

/// Scroll-area settings for a brush preset browser.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ListLayout {
    /// The scroll area's id and its height limit.
    pub id: &'static str,
    /// Maximum height available to the scrolling list.
    pub max_height: f32,
}

/// The Brushes tab's list.
pub const PANEL_LIST: ListLayout = ListLayout { id: "brush-presets", max_height: 400.0 };

/// The group key (`BrushPreset::group`) behind a panel label: ungrouped presets show as
/// [`UNGROUPED`].
pub fn group_key(presets: &[BrushPreset], label: &str) -> String {
    if label == UNGROUPED && !presets.iter().any(|p| p.group == UNGROUPED) { String::new() } else { label.to_string() }
}

/// A shared browser projection paired with its source presets, so specialised brush rendering can
/// retain the original item association instead of joining the projection back by name.
struct BrushGroupView<'a> {
    view: GroupView,
    presets: Vec<&'a BrushPreset>,
}

fn brush_group_views(presets: &[BrushPreset]) -> Vec<BrushGroupView<'_>> {
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
    groups
}

/// Brush owns the identity policy: stable preset names are qualified by their current group/path,
/// while the reverse metadata retains the preview occurrence key for exact command targeting.
fn brush_projection_keys(groups: &[GroupView]) -> PickerProjectionKeys {
    let mut keys = PickerProjectionKeys::default();
    for (group_index, group) in groups.iter().enumerate() {
        keys.insert(
            ProjectionNodeRef::Group(group_index),
            PickerIdentityKey::from_parts("brush-group", [group.name.as_str()]),
            PickerNodeIdentity::Group(group.name.clone()),
        );
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
    }
    keys
}

/// Persisted mixed root order follows the engine's single preset vector: each ungrouped preset is
/// a root card, while each named group occupies the position of its first preset.
fn brush_root_sequence(presets: &[BrushPreset]) -> Vec<HierarchyOrderEntry> {
    let mut seen_groups = std::collections::BTreeSet::new();
    let mut sequence = Vec::new();
    for preset in presets {
        if preset.group.is_empty() {
            sequence.push(HierarchyOrderEntry::RootItem { group: String::new(), key: preview_identity(preset) });
        } else if seen_groups.insert(preset.group.clone()) {
            sequence.push(HierarchyOrderEntry::Group(preset.group.clone()));
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

fn brush_item_index(presets: &[BrushPreset], group: &str, before_index: usize, source_key: &str) -> usize {
    presets[..before_index].iter().filter(|preset| preset.group == group && preview_identity(preset) != source_key).count()
}

/// Translate one validated canonical move into an existing authoritative Brush command action.
/// Moves that the name/path command API cannot express exactly are rejected rather than persisted
/// into the generic picker hierarchy.
fn brush_drop_intent_to_action(
    presets: &[BrushPreset],
    identities: &BTreeMap<PickerId, PickerNodeIdentity>,
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
                Ok(Action::Move { name, group: group_name, folder: Some(path), index: Some(index) })
            } else {
                let first = presets.iter().find(|preset| {
                    preset.group == group
                        && preset.folder.starts_with(&path)
                        && !((source_group == &group && !source_folder.is_empty()) && preset.folder.starts_with(&source_folder))
                });
                let (before, before_item) = first.map_or((None, None), |preset| {
                    if preset.folder == path { (None, Some(preset.name.clone())) } else { (preset.folder.get(path.len()).cloned(), None) }
                });
                Ok(Action::MoveInto { group: source_group_name, folder: source_folder, into: Some((group_name, path)), before, before_item })
            }
        }
        DropPosition::AppendTo { container: ContainerId::Root } => {
            if source_item.is_some() {
                let index = presets.iter().filter(|preset| preset.group.is_empty() && preview_identity(preset) != source_item.unwrap_or_default()).count();
                return Ok(Action::Move { name: source_name.unwrap_or_default(), group: String::new(), folder: Some(Vec::new()), index: Some(index) });
            }
            if source_folder.is_empty() {
                Ok(Action::MoveGroup { group: source_group_name, folder: Vec::new(), before: None })
            } else {
                Ok(Action::MoveInto { group: source_group_name, folder: source_folder, into: None, before: None, before_item: None })
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
                Ok(Action::Move { name, group, folder: Some(path), index: None })
            } else {
                Ok(Action::MoveInto { group: source_group_name, folder: source_folder, into: Some((group, path)), before: None, before_item: None })
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
                            Ok(Action::Move { name, group: target_group_name, folder: Some(target_parent), index: Some(index) })
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
                        Ok(Action::Move { name, group: target_group_name, folder: Some(target_parent), index })
                    }
                    _ => Err("A Brush preset cannot be inserted at the top-level group boundary.".to_owned()),
                }
            } else {
                let move_group = |before: Option<String>| Action::MoveGroup { group: source_group_name.clone(), folder: source_folder.clone(), before };
                let move_into = |into: Option<(String, Vec<String>)>, before: Option<String>, before_item: Option<String>| Action::MoveInto {
                    group: source_group_name.clone(),
                    folder: source_folder.clone(),
                    into,
                    before,
                    before_item,
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
                        let (before, before_item) = if after {
                            let after_index = presets
                                .iter()
                                .position(|preset| preview_identity(preset) == key.as_str())
                                .ok_or_else(|| "The target Brush no longer exists.".to_owned())?;
                            match presets
                                .iter()
                                .skip(after_index + 1)
                                .find(|preset| preset.group == target_group && preset.folder.starts_with(&target_parent) && !source_is_preset(preset))
                            {
                                Some(next) if next.folder == target_parent => (None, Some(next.name.clone())),
                                Some(next) => (Some(next.folder[target_parent.len()].clone()), None),
                                None => (None, None),
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

/// Render the right-click picker's library through the shared browser. Generic search, expansion,
/// selection, rename and brush-specific presentation state live in the namespaced `PresetUi`.
/// Occurrence keys remain separate from display names because imported libraries can contain
/// duplicate names even though engine commands still address presets by name.
/// Keep brush-specific policy identical across the docked panel and floating picker.
macro_rules! brush_browser_adapter {
    ($panel:expr, $groups:expr, $projection_keys:expr, $options:expr, $canonical_move:expr, $rekey:expr, $paint_item:expr, $paint_list_item:expr) => {
        BrowserAdapter {
            panel: $panel,
            groups: $groups,
            projection_keys: Some($projection_keys),
            options: $options,
            capabilities: BrowserCapabilities { item_drag: true, item_drop: true, group_drag: true, selection: true, activation: true },
            item_context_menu: Some(asset_browser::standard_item_context_menu),
            group_context_menu: Some(asset_browser::standard_group_context_menu),
            nested_group_context_menu: Some(asset_browser::standard_nested_group_context_menu),
            canonical_hierarchy_move: Some($canonical_move),
            rekey_hierarchy_after_move: Some($rekey),
            hierarchy_item_move: None,
            paint_group: None,
            paint_item: $paint_item,
            paint_list_item: $paint_list_item,
        }
    };
}

/// Brush-specific footer controls shared by the docked Brushes tab and floating right-click picker.
/// The shell owns their placement; this adapter only translates the three controls into browser actions.
pub(crate) fn draw_brush_footer_controls(ui: &mut egui::Ui, state: &mut PresetUi, presets: &[BrushPreset], panel: &str) -> Vec<BrowserAction> {
    let selection = state.browser_selection_snapshot.get(panel);
    let mut delete_targets = selection
        .into_iter()
        .flat_map(|selection| selection.cards.iter())
        .filter_map(|(_, key)| preset_by_identity(presets, key).map(|preset| (key, preset)))
        .map(|(key, preset)| BrowserAction::DeleteItem { group: preset.group.clone(), group_path: preset.folder.clone(), key: key.clone() })
        .collect::<Vec<_>>();
    if delete_targets.is_empty()
        && let Some(key) = state.selected.get(panel)
        && let Some(preset) = preset_by_identity(presets, key)
    {
        delete_targets.push(BrowserAction::DeleteItem { group: preset.group.clone(), group_path: preset.folder.clone(), key: key.clone() });
    }

    let mut actions = Vec::new();
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let delete = ui.add_enabled_ui(!delete_targets.is_empty(), |ui| crate::icons::button(ui, "trash", 24.0, false, tl!("Delete"))).inner;
        if delete.clicked() {
            actions.extend(delete_targets);
        }
        if crate::icons::button(ui, "plus", 24.0, false, tl!("Create new brush preset")).clicked() {
            actions.push(BrowserAction::New);
        }
        if crate::icons::button(ui, "folder-plus", 24.0, false, tl!("Create new group")).clicked() {
            actions.push(BrowserAction::NewGroup);
        }
    });
    actions
}

pub(crate) fn picker_browser_with_shell(
    ui: &mut egui::Ui,
    presets: &[BrushPreset],
    current: Option<&str>,
    preset_ui: &mut PresetUi,
    shell: crate::pickers::shell::ShellOptions<'_>,
) -> Vec<Action> {
    const PANEL: &str = "brush-picker";

    // Keep each projection paired with its source presets. Joining by display name picks the first
    // match when two groups contain a brush with the same name, painting the wrong preview.
    let group_views = brush_group_views(presets);
    let root_sequence = brush_root_sequence(presets);
    let groups: Vec<GroupView> = group_views
        .iter()
        .map(|group| GroupView {
            name: group.view.name.clone(),
            root_only: group.view.root_only,
            items: group
                .view
                .items
                .iter()
                .map(|item| ItemView { key: item.key.clone(), name: item.name.clone(), group_path: item.group_path.clone() })
                .collect(),
        })
        .collect();
    let projection_keys = brush_projection_keys(&groups);
    let mut hierarchy_actions = Vec::new();

    match current {
        Some(name) => {
            let matching: Vec<_> = presets.iter().filter(|preset| preset.name.eq_ignore_ascii_case(name)).collect();
            if matching.len() == 1 {
                preset_ui.set_selected(PANEL, &preview_identity(matching[0]));
            } else {
                preset_ui.clear_selected(PANEL);
            }
        }
        None => {
            preset_ui.clear_selected(PANEL);
        }
    }
    let mut paint_item = |ui: &egui::Ui, rect: egui::Rect, group_index: usize, item_index: usize, state: ItemPaintState| {
        let Some(preset) = group_views.get(group_index).and_then(|group| group.presets.get(item_index)).copied() else { return };
        let _ = state;
        paint_brush_thumbnail(ui, rect, preset);
    };
    let options = BrowserOptions {
        canvas: egui::Rect::NOTHING,
        flush: false,
        center_grid: false,
        all_groups_open: false,
        // The brush picker is an egui Area, not a parent Menu; use a normal context popup so
        // its menu actions can extend beyond the picker without being dismissed as a submenu.
        compact_context_menus: false,
        domain_hierarchy_actions: true,
        root_item_group: None,
    };
    let browser_actions = {
        let mut rekey = |hierarchy: &PickerBrowserHierarchy, projection: &BTreeMap<PickerId, PickerNodeIdentity>, registry: &mut PickerIdRegistry| {
            rekey_brush_hierarchy(hierarchy, projection, presets, registry);
        };
        let mut canonical_move = |intent: &DropMoveIntent, projection: &BTreeMap<PickerId, PickerNodeIdentity>| {
            hierarchy_actions.push(brush_drop_intent_to_action(presets, projection, intent)?);
            Ok(())
        };
        let mut adapter = brush_browser_adapter!(PANEL, &groups, &projection_keys, options, &mut canonical_move, &mut rekey, &mut paint_item, None);
        crate::pickers::shell::picker_shell(ui, preset_ui, &mut adapter, crate::pickers::shell::ShellOptions { root_sequence: Some(root_sequence), ..shell })
    };
    apply_inline_rename_actions(preset_ui, PANEL, &browser_actions);
    let mut actions = browser_actions_to_actions(presets, browser_actions, preset_ui.browser_selection_snapshot.get(PANEL));
    actions.extend(hierarchy_actions);
    actions
}

/// Translate the shared browser's asset-neutral events into brush preset commands.
fn browser_actions_to_actions(
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
            BrowserAction::Move(key, group) => {
                occurrence_name(presets, &key).map(|name| Action::Move { name, group: group_key(presets, &group), folder: Some(Vec::new()), index: None })
            }
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

/// Paint only the brush tip inside the shared asset-browser cell. The browser owns card
/// geometry, labels, selection, clipping, and list/grid presentation.
fn paint_brush_thumbnail(ui: &egui::Ui, rect: egui::Rect, preset: &BrushPreset) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let brush = &preset.brush;
    let identity = preview_identity(preset);
    let color = Tokens::get(ui.ctx()).text;
    let texture =
        brush_preview::tip_texture(ui.ctx(), &format!("brushes-tip:{identity}"), &brush.tip, (brush.hardness, brush.angle, brush.roundness), 36, color);
    let side = rect.width().min(rect.height()).max(1.0) * 0.8 * crate::brush_sections::thumb_scale(brush.size);
    let thumb = egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(side));
    ui.painter().with_clip_rect(rect.intersect(ui.clip_rect())).image(texture.id(), thumb, full_uv(), Color32::WHITE);
}

/// Deepest nested group the panel draws; deeper groups show at this depth.
pub const MAX_VIEW_DEPTH: usize = photocraft_engine::brush_preset_cmds::MAX_FOLDER_DEPTH;

/// A preview slot must identify the preset occurrence, not only its display name: different groups
/// can contain brushes with the same name and different tips/settings. The address is only a
/// per-frame cache discriminator; the preview cache is rebuilt from the current borrowed library.
pub(crate) fn preview_identity(preset: &BrushPreset) -> String {
    format!("{:x}@{}", std::ptr::from_ref(preset) as usize, preset.name)
}

/// Resolve a browser's opaque occurrence key back to the exact borrowed preset.
pub(crate) fn preset_by_identity<'a>(presets: &'a [BrushPreset], key: &str) -> Option<&'a BrushPreset> {
    presets.iter().find(|preset| preview_identity(preset) == key)
}

/// Whether a name-based engine command cannot identify one preset occurrence unambiguously.
pub(crate) fn is_ambiguous_preset_name(presets: &[BrushPreset], name: &str) -> bool {
    presets.iter().filter(|preset| preset.name.eq_ignore_ascii_case(name)).take(2).count() > 1
}

fn occurrence_name(presets: &[BrushPreset], key: &str) -> Option<String> {
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

/// The collapse key of a group beneath the root shown as `label` (`label` itself for the root).
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

/// Keep collapse state attached to a group when its display path is renamed.
pub(crate) fn remap_collapsed_prefix(collapsed: &mut [String], from: &str, to: &str) {
    for key in collapsed {
        if key == from {
            *key = to.to_string();
        } else if key.strip_prefix(from).is_some_and(|tail| tail.starts_with('/')) {
            key.replace_range(..from.len(), to);
        }
    }
}

pub(crate) fn remove_collapsed_prefix(collapsed: &mut Vec<String>, prefix: &str) {
    collapsed.retain(|key| key != prefix && !key.strip_prefix(prefix).is_some_and(|tail| tail.starts_with('/')));
}

fn group_label_for_key(presets: &[BrushPreset], key: &str) -> Option<String> {
    grouped_presets(presets).into_iter().find(|(label, _)| group_key(presets, label) == key).map(|(label, _)| label)
}

/// A group's content in panel order: its presets and nested groups.
pub type Node<'a> = HierarchyNode<'a, BrushPreset>;
pub type GroupTreeNode<'a> = HierarchyNode<'a, BrushPreset>;
pub type GroupNode<'a> = HierarchyGroup<'a, BrushPreset>;

/// The tree of a group's presets (`items`, in library order): each nested group appears where its
/// first preset is, and presets keep their order. Deeper paths are cut at [`MAX_VIEW_DEPTH`].
pub fn group_tree<'a>(items: &[&'a BrushPreset]) -> Vec<GroupTreeNode<'a>> {
    hierarchy_tree(items.iter().map(|p| (p.folder.as_slice(), *p)).collect(), MAX_VIEW_DEPTH)
}

/// Legacy Brushes-tab API name retained while its callers migrate to the group terminology.
pub fn folder_tree<'a>(items: &[&'a BrushPreset]) -> Vec<Node<'a>> {
    group_tree(items)
}

/// Where a preset dropped onto `target` lands: (root group, nested group path, index after the
/// dragged preset is taken out). `after` = dropped on the target's lower (list) or right (grid)
/// half.
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
        Ok(_) => clear_texture_pattern_selection(app),
        Err(e) => app.ui.status = e,
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
            Action::Move { name, group, folder, index } => {
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
            Action::MoveGroup { group, folder, before } => {
                let mut p = json!({ "group": group, "before": before });
                if !folder.is_empty() {
                    p["folder"] = json!(folder);
                }
                run_or_status(app, "brush.presets.moveGroup", p);
            }
            Action::MoveInto { group, folder, into, before, before_item } => {
                if before_item.as_ref().is_some_and(|name| is_ambiguous_preset_name(&app.session.tools.presets, name)) {
                    app.ui.status =
                        format!("Cannot position the group before `{}` because multiple presets share that name.", before_item.as_deref().unwrap_or_default());
                    continue;
                }
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
                run_or_status(app, "brush.presets.moveGroup", p);
            }
            Action::Rename(r) => {
                let text = r.text.trim().to_string();
                let old = r.folder.last().unwrap_or(&r.name);
                if !text.is_empty() && text != *old {
                    if r.group {
                        let label = group_label_for_key(&app.session.tools.presets, &r.name);
                        let mut p = json!({ "group": r.name, "newName": text });
                        if !r.folder.is_empty() {
                            p["folder"] = json!(r.folder);
                        }
                        if run_or_status(app, "brush.presets.renameGroup", p)
                            && let Some(label) = label
                        {
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
                            remap_collapsed_prefix(&mut app.ui.brushes_panel.collapsed, &from, &to);
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
                    clear_brush_browser_selections(&mut app.ui.presets_ui);
                }
            }
            Action::DeleteOccurrence { name, group, folder } => {
                if run_or_status(app, "brush.presets.delete", json!({ "name": name, "group": group, "folder": folder })) {
                    clear_brush_browser_selections(&mut app.ui.presets_ui);
                }
            }
            Action::DeleteGroup { group, folder } => {
                let label = group_label_for_key(&app.session.tools.presets, &group);
                let mut p = json!({ "group": group });
                if !folder.is_empty() {
                    p["folder"] = json!(folder);
                }
                if run_or_status(app, "brush.presets.deleteGroup", p) {
                    if let Some(label) = label {
                        for panel in ["brushes", "brush-picker"] {
                            if app.ui.presets_ui.selected_group.get(panel) == Some(&label)
                                && app.ui.presets_ui.selected_group_path.get(panel).is_some_and(|path| path.starts_with(&folder))
                            {
                                app.ui.presets_ui.clear_selected(panel);
                            }
                        }
                        remove_collapsed_prefix(&mut app.ui.brushes_panel.collapsed, &group_view_key(&label, &folder));
                    }
                    clear_brush_browser_selections(&mut app.ui.presets_ui);
                }
            }
            Action::NewPreset => {
                crate::brush_preset_dialog::open(app, None);
            }
            Action::CreateGroup { group, presets } => {
                if presets.is_empty() {
                    app.ui.status = "Select at least one brush before creating a group.".into();
                    continue;
                }
                if brush_group_views(&app.session.tools.presets).iter().any(|view| view.view.name.eq_ignore_ascii_case(&group)) {
                    app.ui.status = format!("A brush group named `{group}` already exists.");
                    continue;
                }
                if let Some(name) = presets.iter().find(|name| is_ambiguous_preset_name(&app.session.tools.presets, name)) {
                    app.ui.status = format!("Cannot create the group because brush `{name}` has a duplicate name.");
                    continue;
                }
                // shortcut: groups exist only while they contain presets; upgrade when empty groups are persisted.
                for name in presets {
                    if !run_or_status(app, "brush.presets.move", json!({ "name": name, "group": &group, "folder": [] })) {
                        break;
                    }
                }
            }
        }
    }
}

fn clear_brush_browser_selections(state: &mut PresetUi) {
    for panel in ["brushes", "brush-picker"] {
        state.clear_browser_selection(panel);
    }
}

/// What dropping one group on another does in the Brushes panel: on the header's upper edge it
/// lands before that group (reordering siblings); anywhere else it nests inside that group.
/// `None` means the drop is a no-op, such as dropping onto itself or into a descendant.
pub fn group_drop(src_group: &str, src_path: &[String], group: &str, target_path: &[String], edge: bool) -> Option<Action> {
    match hierarchy_group_drop(src_group, src_path, group, target_path, edge)? {
        HierarchyDrop::MoveGroup { group, group_path, before } => Some(Action::MoveGroup { group, folder: group_path, before }),
        HierarchyDrop::MoveInto { group, group_path, into, before } => Some(Action::MoveInto { group, folder: group_path, into, before, before_item: None }),
    }
}

/// Legacy Brushes-tab API name; both arguments are group paths.
pub fn folder_drop(src_group: &str, src_path: &[String], group: &str, target_path: &[String], edge: bool) -> Option<Action> {
    group_drop(src_group, src_path, group, target_path, edge)
}

/// Render the Brushes tab through the shared browser. Only the brush-specific projection, preview
/// painters, and context-menu policy remain here; common hierarchy, selection, filtering, rename,
/// and drag/drop behavior are owned by `asset_browser`.
pub fn preset_list(
    ui: &mut egui::Ui,
    presets: &[BrushPreset],
    current: Option<&str>,
    st: &mut BrushesPanelState,
    preset_ui: &mut PresetUi,
    layout: ListLayout,
) -> Vec<Action> {
    const PANEL: &str = "brushes";
    if presets.is_empty() {
        ui.label(RichText::new(tl!("No brush presets")).color(Tokens::get(ui.ctx()).text_faint));
        return Vec::new();
    }

    sync_brush_state_to_browser(st, preset_ui, presets, current, PANEL);
    let group_views = brush_group_views(presets);
    let root_sequence = brush_root_sequence(presets);
    let groups: Vec<GroupView> = group_views
        .iter()
        .map(|group| GroupView {
            name: group.view.name.clone(),
            root_only: group.view.root_only,
            items: group
                .view
                .items
                .iter()
                .map(|item| ItemView { key: item.key.clone(), name: item.name.clone(), group_path: item.group_path.clone() })
                .collect(),
        })
        .collect();
    let projection_keys = brush_projection_keys(&groups);
    let mut hierarchy_actions = Vec::new();
    let mut paint_item = |ui: &egui::Ui, rect: egui::Rect, group_index: usize, item_index: usize, state: ItemPaintState| {
        let Some(preset) = group_views.get(group_index).and_then(|group| group.presets.get(item_index)).copied() else { return };
        let _ = state;
        paint_brush_thumbnail(ui, rect, preset);
    };
    let mut footer = |ui: &mut egui::Ui, state: &mut PresetUi| draw_brush_footer_controls(ui, state, presets, PANEL);
    let browser_actions = {
        let mut rekey = |hierarchy: &PickerBrowserHierarchy, projection: &BTreeMap<PickerId, PickerNodeIdentity>, registry: &mut PickerIdRegistry| {
            rekey_brush_hierarchy(hierarchy, projection, presets, registry);
        };
        let mut canonical_move = |intent: &DropMoveIntent, projection: &BTreeMap<PickerId, PickerNodeIdentity>| {
            hierarchy_actions.push(brush_drop_intent_to_action(presets, projection, intent)?);
            Ok(())
        };
        let mut adapter = brush_browser_adapter!(
            PANEL,
            &groups,
            &projection_keys,
            BrowserOptions {
                canvas: egui::Rect::NOTHING,
                flush: false,
                center_grid: true,
                all_groups_open: true,
                compact_context_menus: false,
                domain_hierarchy_actions: true,
                root_item_group: None,
            },
            &mut canonical_move,
            &mut rekey,
            &mut paint_item,
            None
        );
        crate::pickers::shell::picker_shell(
            ui,
            preset_ui,
            &mut adapter,
            crate::pickers::shell::ShellOptions {
                custom_footer: Some((&mut footer, 28.0)),
                root_sequence: Some(root_sequence),
                max_total_height: Some(layout.max_height),
                ..Default::default()
            },
        )
    };
    apply_inline_rename_actions(preset_ui, PANEL, &browser_actions);
    sync_browser_state_to_brush_state(st, preset_ui, presets, PANEL);
    let mut actions = browser_actions_to_actions(presets, browser_actions, preset_ui.browser_selection_snapshot.get(PANEL));
    actions.extend(hierarchy_actions);
    actions
}

fn apply_inline_rename_actions(state: &mut PresetUi, panel: &str, actions: &[BrowserAction]) {
    for action in actions {
        match action {
            BrowserAction::BeginInlineRenameItem(group, item) => {
                begin_item_rename(state, panel, &[(group.clone(), item.clone())]);
            }
            BrowserAction::CancelInlineRenameItem => {
                state.item_renaming = None;
                state.item_rename_queue.clear();
                state.renaming = None;
            }
            BrowserAction::RenameItem { .. } | BrowserAction::Rename(_, _) => {
                state.renaming = state.item_renaming.as_ref().map(|rename| (panel.to_string(), rename.key.clone(), rename.text.clone()));
            }
            _ => {}
        }
    }
}

/// Bridge the Brushes panel's persisted public state to the shared transient browser state. The
/// legacy JSON shape stays stable while the browser owns the live interaction model.
fn sync_brush_state_to_browser(st: &BrushesPanelState, browser: &mut PresetUi, presets: &[BrushPreset], current: Option<&str>, panel: &str) {
    browser.search.insert(panel.to_string(), st.filter.clone());
    if !browser.display_modes.contains_key(panel) {
        browser.sizes.insert(panel.to_string(), if st.view == BrushesView::List { asset_browser::MIN_THUMB } else { 40.0 });
    }
    browser.flatten_group_nesting(panel);
    browser.toggled.retain(|key| !key.starts_with(&format!("{panel}/")));
    let group_labels: Vec<String> = grouped_presets(presets).into_iter().map(|(label, _)| label).collect();
    browser.toggled.extend(st.collapsed.iter().map(|key| {
        let key = if group_labels.iter().any(|label| label == key) { group_view_key(key, &[]) } else { key.clone() };
        format!("{panel}/{key}")
    }));

    match current {
        Some(name) => {
            let mut matching = presets.iter().filter(|preset| preset.name.eq_ignore_ascii_case(name));
            if let Some(preset) = matching.next().filter(|_| matching.next().is_none()) {
                browser.set_selected(panel, &preview_identity(preset));
            } else {
                browser.clear_selected(panel);
            }
        }
        None => {
            browser.clear_selected(panel);
        }
    }

    let has_browser_rename =
        browser.renaming.as_ref().is_some_and(|rename| rename.0 == panel) || browser.group_renaming.as_ref().is_some_and(|rename| rename.panel == panel);
    if !has_browser_rename && let Some(rename) = &st.renaming {
        let group_label = group_label_for_key(presets, &rename.name).unwrap_or_else(|| rename.name.clone());
        if rename.group && !rename.folder.is_empty() {
            browser.renaming = None;
            browser.group_renaming =
                Some(NestedGroupRenameState { panel: panel.to_string(), group: group_label, group_path: rename.folder.clone(), text: rename.text.clone() });
        } else {
            let key = if rename.group {
                format!("group:{group_label}")
            } else {
                presets.iter().find(|preset| preset.name.eq_ignore_ascii_case(&rename.name)).map_or_else(|| rename.name.clone(), preview_identity)
            };
            browser.renaming = Some((panel.to_string(), key, rename.text.clone()));
            browser.group_renaming = None;
        }
    }
}

/// Copy shared expansion and rename edits back into the Brushes panel's serialized state.
fn sync_browser_state_to_brush_state(st: &mut BrushesPanelState, browser: &PresetUi, presets: &[BrushPreset], panel: &str) {
    let group_labels: Vec<String> = grouped_presets(presets).into_iter().map(|(label, _)| label).collect();
    let expanded_keys: Vec<String> = browser
        .toggled
        .iter()
        .filter_map(|key| {
            let key = key.strip_prefix(&format!("{panel}/"))?;
            Some(group_labels.iter().find(|label| group_view_key(label, &[]) == key).cloned().unwrap_or_else(|| key.to_string()))
        })
        .collect();
    let mut collapsed: Vec<String> = st.collapsed.iter().filter(|key| expanded_keys.contains(key)).cloned().collect();
    for key in expanded_keys {
        if !collapsed.contains(&key) {
            collapsed.push(key);
        }
    }
    st.collapsed = collapsed;
    st.filter = browser.search.get(panel).cloned().unwrap_or_default();

    st.renaming = if let Some(rename) = browser.group_renaming.as_ref().filter(|rename| rename.panel == panel) {
        Some(Renaming { group: true, name: group_key(presets, &rename.group), folder: rename.group_path.clone(), text: rename.text.clone() })
    } else if let Some((_, key, text)) = browser.renaming.as_ref().filter(|rename| rename.0 == panel) {
        if let Some(group) = key.strip_prefix("group:") {
            Some(Renaming { group: true, name: group_key(presets, group), folder: Vec::new(), text: text.clone() })
        } else {
            Some(Renaming { group: false, name: occurrence_name(presets, key).unwrap_or_else(|| key.clone()), folder: Vec::new(), text: text.clone() })
        }
    } else {
        None
    };
}
