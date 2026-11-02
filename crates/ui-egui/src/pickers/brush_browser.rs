//! Shared brush preset browser used by the Brushes tab and the right-click Brush Picker.

use std::collections::BTreeMap;

use egui::{Color32, RichText, pos2, vec2};
use photocraft_engine::paint::BrushPreset;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::brush_preview;
use crate::pickers::asset_browser::{self, BrowserAction, BrowserAdapter, BrowserCapabilities, BrowserOptions, GroupView, ItemPaintState, ItemView};
use crate::pickers::asset_browser_hierarchy::{HierarchyGroup, HierarchyNode, hierarchy_tree};
pub(crate) use crate::pickers::asset_browser_interaction::AssetDrag as BrushDrag;
use crate::pickers::asset_browser_interaction::{ContextMenuAction, HierarchyDrop, draw_group_context_menu, draw_item_context_menu, hierarchy_group_drop};
use crate::pickers::picker_browser_dragging::DropMoveIntent;
use crate::pickers::picker_browser_hierarchy::HierarchyIdentity;
use crate::pickers::picker_browser_model::{ContainerId, DropPosition, NodeId};
use crate::pickers::picker_state::{NestedGroupRenameState, PresetUi};
use crate::theme::Tokens;

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
pub struct BrushesPanelState {
    /// Collapsed groups: a group's label, or for a nested group the root label and its path joined
    /// with `/` ([`crate::brushes_tab::group_view_key`]).
    pub collapsed: Vec<String>,
    /// Case-insensitive name filter.
    pub filter: String,
    pub view: BrushesView,
    pub renaming: Option<Renaming>,
}

impl Default for BrushesPanelState {
    fn default() -> Self {
        Self { collapsed: Vec::new(), filter: String::new(), view: BrushesView::default(), renaming: None }
    }
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
    /// Delete a root group (`folder` empty) or one of its nested groups, with their presets.
    DeleteGroup {
        group: String,
        folder: Vec<String>,
    },
}

/// How a preset list is laid out: the Brushes tab's, or the Brush Preset picker's (narrower,
/// denser).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ListLayout {
    /// The scroll area's id and its height limit.
    pub id: &'static str,
    /// Maximum height available to the scrolling list.
    pub max_height: f32,
    /// Width of a grid cell (its height is 6 points more) and the grid's left indent.
    pub cell: f32,
    /// Horizontal inset applied to the grid and nested-group contents.
    pub indent: f32,
    /// The Brush Preset picker's cards: a tip cell and a stroke cell on top and the name across
    /// the bottom (or, with only the tip and the name on, in the stroke's cell). Replaces the
    /// list/grid choice in the picker.
    pub cards: bool,
}

/// The Brushes tab's list.
pub const PANEL_LIST: ListLayout = ListLayout { id: "brush-presets", max_height: 400.0, cell: 52.0, indent: 20.0, cards: false };
/// Card scale shared with the right-click picker's footer slider.
pub(crate) const SCALE_RANGE: std::ops::RangeInclusive<f32> = 0.15..=2.0;

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
    grouped_presets(presets)
        .into_iter()
        .map(|(name, indices)| {
            let presets: Vec<&BrushPreset> = indices.into_iter().filter_map(|i| presets.get(i)).collect();
            let items =
                presets.iter().map(|preset| ItemView { key: preview_identity(preset), name: preset.name.clone(), group_path: preset.folder.clone() }).collect();
            BrushGroupView { view: GroupView { name, root_only: false, items }, presets }
        })
        .collect()
}

/// Reverse identities for every canonical node projected from the Brush library.
fn brush_hierarchy_identities(groups: &[GroupView]) -> BTreeMap<NodeId, HierarchyIdentity> {
    let mut identities = BTreeMap::new();
    for group in groups {
        identities.insert(NodeId::group(&group.name), HierarchyIdentity::Group(group.name.clone()));
        for item in &group.items {
            let path = &item.group_path[..item.group_path.len().min(crate::pickers::asset_browser_hierarchy::MAX_HIERARCHY_DEPTH)];
            let mut parent = Vec::new();
            for part in path {
                parent.push(part.clone());
                let id = NodeId::group_path(std::iter::once(group.name.clone()).chain(parent[..parent.len() - 1].iter().cloned()), part.clone());
                identities.insert(id, HierarchyIdentity::Folder { group: group.name.clone(), path: parent.clone() });
            }
            let id = if path.is_empty() {
                NodeId::item(&group.name, &item.key)
            } else {
                NodeId::item_in_path(std::iter::once(group.name.clone()).chain(path.iter().cloned()), item.key.clone())
            };
            let identity = if path.is_empty() {
                HierarchyIdentity::Item { group: group.name.clone(), key: item.key.clone() }
            } else {
                HierarchyIdentity::FolderItem { group: group.name.clone(), path: path.to_vec(), key: item.key.clone() }
            };
            identities.insert(id, identity);
        }
    }
    identities
}

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
fn brush_drop_intent_to_action(presets: &[BrushPreset], identities: &BTreeMap<NodeId, HierarchyIdentity>, intent: &DropMoveIntent) -> Result<Action, String> {
    let source = identities.get(&intent.source).ok_or_else(|| "The dragged Brush no longer exists.".to_owned())?;
    let source_item = match source {
        HierarchyIdentity::Item { key, .. } | HierarchyIdentity::FolderItem { key, .. } => Some(key.as_str()),
        _ => None,
    };
    let source_group = match source {
        HierarchyIdentity::Item { group, .. }
        | HierarchyIdentity::FolderItem { group, .. }
        | HierarchyIdentity::Group(group)
        | HierarchyIdentity::Folder { group, .. } => group,
    };
    let source_folder = match source {
        HierarchyIdentity::FolderItem { path, .. } | HierarchyIdentity::Folder { path, .. } => path.clone(),
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

    let destination = |id: &NodeId| -> Result<(String, Vec<String>, Option<(&'static str, String)>), String> {
        match identities.get(id) {
            Some(HierarchyIdentity::Item { group, key }) => Ok((group.clone(), Vec::new(), Some(("item", key.clone())))),
            Some(HierarchyIdentity::FolderItem { group, path, key }) => Ok((group.clone(), path.clone(), Some(("item", key.clone())))),
            Some(HierarchyIdentity::Group(group)) => Ok((group.clone(), Vec::new(), Some(("group", group.clone())))),
            Some(HierarchyIdentity::Folder { group, path }) => {
                Ok((group.clone(), path[..path.len().saturating_sub(1)].to_vec(), Some(("folder", path.last().cloned().unwrap_or_default()))))
            }
            None => Err("The Brush drop target no longer exists.".to_owned()),
        }
    };

    match &intent.position {
        DropPosition::PrependTo { group: container } => {
            let (group, path) = match identities.get(container) {
                Some(HierarchyIdentity::Group(group)) => (group.clone(), Vec::new()),
                Some(HierarchyIdentity::Folder { group, path }) => (group.clone(), path.clone()),
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
                return Err("A Brush preset cannot be moved outside its group.".to_owned());
            }
            if source_folder.is_empty() {
                Ok(Action::MoveGroup { group: source_group_name, folder: Vec::new(), before: None })
            } else {
                Ok(Action::MoveInto { group: source_group_name, folder: source_folder, into: None, before: None, before_item: None })
            }
        }
        DropPosition::AppendTo { container: ContainerId::Group(container) } => {
            let (group, path) = match identities.get(container) {
                Some(HierarchyIdentity::Group(group)) => (group.clone(), Vec::new()),
                Some(HierarchyIdentity::Folder { group, path }) => (group.clone(), path.clone()),
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
                        let (_, key) = target_kind.expect("item target");
                        if !after {
                            let before = resolve_before_item_name(presets, &key)?;
                            Ok(Action::MoveBefore { name, group: target_group_name, folder: Some(target_parent), before })
                        } else {
                            let target_index = presets
                                .iter()
                                .position(|preset| preview_identity(preset) == key)
                                .ok_or_else(|| "The target Brush no longer exists.".to_owned())?;
                            let index = brush_item_index(
                                presets,
                                &target_group,
                                target_index + 1,
                                match source {
                                    HierarchyIdentity::Item { key, .. } | HierarchyIdentity::FolderItem { key, .. } => key,
                                    _ => unreachable!(),
                                },
                            );
                            Ok(Action::Move { name, group: target_group_name, folder: Some(target_parent), index: Some(index) })
                        }
                    }
                    Some("folder") => {
                        let (_, leaf) = target_kind.expect("folder target");
                        let before_index = if after {
                            match brush_next_sibling_after_folder(presets, &target_group, &target_parent, &leaf, None) {
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
                            path.push(leaf);
                            presets.iter().position(|preset| preset.group == target_group && preset.folder.starts_with(&path))
                        };
                        let index = before_index.map(|index| {
                            brush_item_index(
                                presets,
                                &target_group,
                                index,
                                match source {
                                    HierarchyIdentity::Item { key, .. } | HierarchyIdentity::FolderItem { key, .. } => key,
                                    _ => unreachable!(),
                                },
                            )
                        });
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
                        let (_, leaf) = target_kind.expect("folder target");
                        let (before, before_item) = if after {
                            match brush_next_sibling_after_folder(
                                presets,
                                &target_group,
                                &target_parent,
                                &leaf,
                                (!source_folder.is_empty() && source_group == &target_group).then_some(source_folder.as_slice()),
                            ) {
                                Some(BrushSiblingAnchor::Folder(next)) => (Some(next), None),
                                Some(BrushSiblingAnchor::Item { key, .. }) => (None, Some(resolve_before_item_name(presets, &key)?)),
                                None => (None, None),
                            }
                        } else {
                            (Some(leaf), None)
                        };
                        Ok(move_into(Some((target_group_name, target_parent)), before, before_item))
                    }
                    Some("item") => {
                        let (_, key) = target_kind.expect("item target");
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
                                .position(|preset| preview_identity(preset) == key)
                                .ok_or_else(|| "The target Brush no longer exists.".to_owned())?;
                            match presets
                                .iter()
                                .skip(after_index + 1)
                                .filter(|preset| preset.group == target_group && preset.folder.starts_with(&target_parent) && !source_is_preset(preset))
                                .next()
                            {
                                Some(next) if next.folder == target_parent => (None, Some(next.name.clone())),
                                Some(next) => (Some(next.folder[target_parent.len()].clone()), None),
                                None => (None, None),
                            }
                        } else {
                            (None, Some(resolve_before_item_name(presets, &key)?))
                        };
                        Ok(move_into(Some((target_group_name, target_parent)), before, before_item))
                    }
                    _ => Err("The Brush drop target cannot receive this group move.".to_owned()),
                }
            }
        }
    }
}

/// Project the brush library into the shared asset-browser model while retaining stable group/item
/// ordering, identity keys, and nested group paths.
#[cfg(test)]
pub(crate) fn browser_group_views(presets: &[BrushPreset]) -> Vec<GroupView> {
    brush_group_views(presets).into_iter().map(|group| group.view).collect()
}

/// Render the right-click picker's library through the shared browser. Generic search, expansion,
/// selection, rename and brush-specific presentation state live in the namespaced `PresetUi`.
/// Occurrence keys remain separate from display names because imported libraries can contain
/// duplicate names even though engine commands still address presets by name.
pub(crate) fn picker_browser(
    ui: &mut egui::Ui,
    presets: &[BrushPreset],
    current: Option<&str>,
    preset_ui: &mut PresetUi,
    allocated_height: f32,
) -> Vec<Action> {
    const PANEL: &str = "brush-picker";

    // Keep each projection paired with its source presets. Joining by display name picks the first
    // match when two groups contain a brush with the same name, painting the wrong preview.
    let group_views = brush_group_views(presets);
    let groups: Vec<GroupView> = group_views
        .iter()
        .map(|group| GroupView {
            name: group.view.name.clone(),
            root_only: false,
            items: group
                .view
                .items
                .iter()
                .map(|item| ItemView { key: item.key.clone(), name: item.name.clone(), group_path: item.group_path.clone() })
                .collect(),
        })
        .collect();
    let identities = brush_hierarchy_identities(&groups);
    let mut hierarchy_actions = Vec::new();
    let mut canonical_move = |intent: &DropMoveIntent| {
        let action = brush_drop_intent_to_action(presets, &identities, intent)?;
        hierarchy_actions.push(action);
        Ok(())
    };

    preset_ui.sizes.insert(PANEL.to_string(), 40.0);
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
    let brush_state = &mut preset_ui.brush_picker;
    // The footer slider is bounded by the browser's actual width. Otherwise a large scale makes
    // a single card wider than the scroll area's clip region, and the item can spill into the
    // picker frame. Keep the same cap in the slider so it stops at the container edge.
    let show = (brush_state.show_name, brush_state.show_stroke, brush_state.show_tip);
    let scale_max = max_picker_card_scale(ui.available_width(), show);
    let scale = brush_state.scale.clamp(*SCALE_RANGE.start(), scale_max);
    brush_state.scale = scale;
    let scale_h = scale.max(CARD_MIN_H_SCALE);
    let (card_w, card_h) = if tip_only(show) {
        (CARD_TIP_ONLY_W * scale, CARD_TIP_ONLY_H * scale_h)
    } else if tip_and_name(show) {
        (CARD_TIP_NAME_W * scale, CARD_TIP_NAME_H * scale_h)
    } else {
        (CARD_MAX_W * scale, (CARD_MAX_H - if brush_state.show_name { 0.0 } else { CARD_NAME_H }) * scale_h)
    };

    let mut paint_item = |ui: &egui::Ui, rect: egui::Rect, group_index: usize, item_index: usize, state: ItemPaintState| {
        let Some(preset) = group_views.get(group_index).and_then(|group| group.presets.get(item_index)).copied() else { return };
        let selected = matches!(state, ItemPaintState::Item { selected: true });
        paint_picker_card(ui, rect, preset, selected, false, show, scale);
    };
    let mut paint_list_item = |ui: &egui::Ui, rect: egui::Rect, group_index: usize, item_index: usize, selected: bool| {
        let Some(preset) = group_views.get(group_index).and_then(|group| group.presets.get(item_index)).copied() else { return };
        paint_brush_list_row(ui, rect, preset, selected);
    };
    let mut adapter = BrowserAdapter {
        panel: PANEL,
        groups: &groups,
        options: BrowserOptions {
            canvas: egui::Rect::NOTHING,
            flush: false,
            center_grid: tip_only(show),
            grid_indent: 8.0,
            all_groups_open: true,
            // The brush picker is an egui Area, not a parent Menu; use a normal context popup so
            // its menu actions can extend beyond the picker without being dismissed as a submenu.
            compact_context_menus: false,
            item_size: Some(vec2(card_w, card_h)),
            list_item_height: Some(44.0),
            domain_hierarchy_actions: true,
            root_item_group: None,
        },
        // Brush folders are projected from each item's full `group_path` by the canonical tree.
        // The shared resolver emits path-aware domain actions; Brush dispatches those through its
        // engine commands rather than persisting folder structure in generic `PresetUi` state.
        capabilities: BrowserCapabilities {
            item_drag: true,
            item_drop: true,
            group_drag: true,
            selection: true,
            activation: true,
            canonical_hierarchy_drag: true,
        },
        item_context_menu: Some(brush_item_context_menu),
        group_context_menu: Some(brush_group_context_menu),
        nested_group_context_menu: Some(brush_nested_group_context_menu),
        canonical_hierarchy_move: Some(&mut canonical_move),
        hierarchy_item_move: None,
        paint_group: None,
        paint_item: &mut paint_item,
        paint_list_item: Some(&mut paint_list_item),
    };
    let browser_actions = asset_browser::browser(ui, preset_ui, &mut adapter, allocated_height);
    drop(adapter);
    drop(canonical_move);
    paint_brush_hierarchy_drag_label(ui, PANEL, egui::Id::new(("brush-drag-label", PANEL)));

    let mut actions = browser_actions_to_actions(presets, browser_actions);
    actions.extend(hierarchy_actions);
    actions
}

/// Translate the shared browser's asset-neutral events into brush preset commands.
fn browser_actions_to_actions(presets: &[BrushPreset], browser_actions: Vec<BrowserAction>) -> Vec<Action> {
    browser_actions
        .into_iter()
        .filter_map(|action| match action {
            BrowserAction::Select(key) => occurrence_name(presets, &key).map(Action::Select),
            BrowserAction::SelectItem { key, .. } => occurrence_name(presets, &key).map(Action::Select),
            BrowserAction::Activate(key) => occurrence_name(presets, &key).map(Action::Choose),
            BrowserAction::ActivateItem { key, .. } => occurrence_name(presets, &key).map(Action::Choose),
            BrowserAction::Rename(key, text) => {
                occurrence_name(presets, &key).map(|name| Action::Rename(Renaming { group: false, name, folder: Vec::new(), text }))
            }
            BrowserAction::RenameItem { group: _, group_path, key, text } => {
                occurrence_name(presets, &key).map(|name| Action::Rename(Renaming { group: false, name, folder: group_path, text }))
            }
            BrowserAction::Delete(key) => occurrence_name(presets, &key).map(Action::Delete),
            BrowserAction::DeleteItem { key, .. } => occurrence_name(presets, &key).map(Action::Delete),
            BrowserAction::Move(key, group) => {
                occurrence_name(presets, &key).map(|name| Action::Move { name, group: group_key(presets, &group), folder: Some(Vec::new()), index: None })
            }
            BrowserAction::MoveItem(key, group, index) => {
                occurrence_name(presets, &key).map(|name| Action::Move { name, group: group_key(presets, &group), folder: Some(Vec::new()), index })
            }
            BrowserAction::MoveItemToGroup { key, group, group_path, before: Some(before) } => occurrence_name(presets, &key)
                .zip(occurrence_name(presets, &before))
                .map(|(name, before)| Action::MoveBefore { name, group: group_key(presets, &group), folder: Some(group_path), before }),
            BrowserAction::MoveItemToGroup { key, group, group_path, before: None } => {
                occurrence_name(presets, &key).map(|name| Action::Move { name, group: group_key(presets, &group), folder: Some(group_path), index: None })
            }
            BrowserAction::RenameGroup(group, text) => {
                Some(Action::Rename(Renaming { group: true, name: group_key(presets, &group), folder: Vec::new(), text }))
            }
            BrowserAction::DeleteGroup(group) => Some(Action::DeleteGroup { group: group_key(presets, &group), folder: Vec::new() }),
            BrowserAction::RenameNestedGroup { group, group_path, text } => {
                Some(Action::Rename(Renaming { group: true, name: group_key(presets, &group), folder: group_path, text }))
            }
            BrowserAction::DeleteNestedGroup { group, group_path } => Some(Action::DeleteGroup { group: group_key(presets, &group), folder: group_path }),
            BrowserAction::MoveGroup { group, group_path, before } => Some(Action::MoveGroup {
                group: group_key(presets, &group),
                before: if group_path.is_empty() { before.map(|name| group_key(presets, &name)) } else { before },
                folder: group_path,
            }),
            BrowserAction::MoveInto { group, group_path, into, before, before_item } => {
                let before_is_group = into.is_none();
                let before_item = match before_item.map(|key| resolve_before_item_name(presets, &key)) {
                    Some(Ok(name)) => Some(name),
                    Some(Err(message)) => return Some(Action::Status(message)),
                    None => None,
                };
                Some(Action::MoveInto {
                    group: group_key(presets, &group),
                    folder: group_path,
                    into: into.map(|(target, path)| (group_key(presets, &target), path)),
                    before: if before_is_group { before.map(|name| group_key(presets, &name)) } else { before },
                    before_item,
                })
            }
            BrowserAction::Drop(_, _) | BrowserAction::NewGroup | BrowserAction::New => None,
        })
        .collect()
}

fn brush_item_context_menu(
    ui: &mut egui::Ui,
    st: &mut PresetUi,
    panel: &str,
    item: &ItemView,
    group: &GroupView,
    _group_names: &[String],
    actions: &mut Vec<BrowserAction>,
) {
    match draw_item_context_menu(ui, tl!("Rename Brush…"), tl!("Delete Brush"), None) {
        Some(ContextMenuAction::Rename) => {
            if st.group_renaming.as_ref().is_some_and(|rename| rename.panel == panel) {
                st.group_renaming = None;
            }
            st.item_renaming = Some(crate::pickers::picker_state::ItemRenameState {
                panel: panel.to_string(),
                group: group.name.clone(),
                group_path: item.group_path.clone(),
                key: item.key.clone(),
                text: item.name.clone(),
            });
        }
        Some(ContextMenuAction::Delete) => {
            actions.push(BrowserAction::DeleteItem { group: group.name.clone(), group_path: item.group_path.clone(), key: item.key.clone() })
        }
        Some(ContextMenuAction::MoveTo(_)) | None => {}
    }
}

fn brush_group_context_menu(ui: &mut egui::Ui, st: &mut PresetUi, panel: &str, group: &GroupView, actions: &mut Vec<BrowserAction>) {
    match draw_group_context_menu(ui, tl!("Rename Group…"), tl!("Delete Group")) {
        Some(ContextMenuAction::Rename) => {
            if st.group_renaming.as_ref().is_some_and(|rename| rename.panel == panel) {
                st.group_renaming = None;
            }
            st.renaming = Some((panel.to_string(), format!("group:{}", group.name), group.name.clone()));
        }
        Some(ContextMenuAction::Delete) => actions.push(BrowserAction::DeleteGroup(group.name.clone())),
        Some(ContextMenuAction::MoveTo(_)) | None => {}
    }
}

fn brush_nested_group_context_menu(
    ui: &mut egui::Ui,
    st: &mut PresetUi,
    panel: &str,
    group: &GroupView,
    group_path: &[String],
    actions: &mut Vec<BrowserAction>,
) {
    match draw_group_context_menu(ui, tl!("Rename Group…"), tl!("Delete Group")) {
        Some(ContextMenuAction::Rename) => {
            st.renaming = None;
            st.group_renaming = Some(NestedGroupRenameState {
                panel: panel.to_string(),
                group: group.name.clone(),
                group_path: group_path.to_vec(),
                text: group_path.last().cloned().unwrap_or_default(),
            });
        }
        Some(ContextMenuAction::Delete) => actions.push(BrowserAction::DeleteNestedGroup { group: group.name.clone(), group_path: group_path.to_vec() }),
        Some(ContextMenuAction::MoveTo(_)) | None => {}
    }
}

/// Horizontal room reserved for the stroke preview gaps and the preset name in a list row.
const STROKE_NAME_ROOM: f32 = 140.0;

fn paint_brush_list_row(ui: &egui::Ui, rect: egui::Rect, preset: &BrushPreset, selected: bool) {
    let t = Tokens::get(ui.ctx());
    let bg = if selected { t.accent_soft } else { t.card };
    ui.painter().rect_filled(rect, 2.0, bg);
    let brush = &preset.brush;
    let identity = preview_identity(preset);
    let tip = brush_preview::tip_texture(ui.ctx(), &format!("brushes-tip:{identity}"), &brush.tip, (brush.hardness, brush.angle, brush.roundness), 36, t.text);
    let cell = egui::Rect::from_min_size(rect.left_top() + vec2(22.0, 2.0), vec2(36.0, 30.0));
    let tip_rect = egui::Rect::from_center_size(cell.center(), vec2(30.0, 30.0) * crate::brush_sections::thumb_scale(brush.size));
    ui.painter().image(tip.id(), tip_rect, full_uv(), Color32::WHITE);
    ui.painter().text(
        pos2(cell.center().x, rect.bottom() - 1.0),
        egui::Align2::CENTER_BOTTOM,
        format!("{}", brush.size.round() as i64),
        egui::FontId::proportional(9.0),
        t.text_faint,
    );
    let width = (rect.width() - (cell.right() - rect.left()) - STROKE_NAME_ROOM).clamp(60.0, 170.0).round();
    let stroke = brush_preview::stroke_texture(ui.ctx(), &format!("brushes-stroke-{width}:{identity}"), brush, width as u32, 36, t.text);
    let stroke_rect = egui::Rect::from_min_size(pos2(cell.right() + 8.0, rect.top() + 4.0), vec2(width, 36.0));
    ui.painter().image(stroke.id(), stroke_rect, full_uv(), Color32::WHITE);
    crate::layer_row_ui::label(
        ui.painter(),
        stroke_rect.right() + 12.0,
        rect.center().y,
        rect.right() - 4.0,
        &preset.name,
        egui::FontId::proportional(12.0),
        t.text_dim,
    );
}

/// Paint the brush-specific contents of a Brushes-tab grid cell. Selection and hit targets belong
/// to `asset_browser`; this callback only draws the tip and its size label.
fn paint_brush_grid_item(ui: &egui::Ui, rect: egui::Rect, preset: &BrushPreset, selected: bool) {
    let t = Tokens::get(ui.ctx());
    if selected {
        ui.painter().rect_filled(rect, 3.0, t.accent_soft);
    }
    if !ui.is_rect_visible(rect) {
        return;
    }
    let brush = &preset.brush;
    let identity = preview_identity(preset);
    let tip = brush_preview::tip_texture(ui.ctx(), &format!("brushes-tip:{identity}"), &brush.tip, (brush.hardness, brush.angle, brush.roundness), 36, t.text);
    let side = (rect.width() - 14.0).max(1.0);
    let tip_rect =
        egui::Rect::from_center_size(pos2(rect.center().x, rect.top() + side / 2.0 + 4.0), vec2(side, side) * crate::brush_sections::thumb_scale(brush.size));
    ui.painter().image(tip.id(), tip_rect, full_uv(), Color32::WHITE);
    ui.painter().text(
        pos2(rect.center().x, rect.bottom() - 7.0),
        egui::Align2::CENTER_CENTER,
        format!("{}", brush.size.round() as i64),
        egui::FontId::proportional(9.5),
        t.text_dim,
    );
}

fn paint_picker_card(ui: &egui::Ui, rect: egui::Rect, preset: &BrushPreset, selected: bool, hovered: bool, show: (bool, bool, bool), scale: f32) {
    let (show_name, show_stroke, show_tip) = show;
    let tip_only = tip_only(show);
    let name_in_top = tip_and_name(show);
    let compact = scale <= CARD_COMPACT_SCALE;
    let scale_h = scale.max(CARD_MIN_H_SCALE);
    let t = Tokens::get(ui.ctx());
    // The shared browser draws the selection outline around `rect`, so the card background must
    // use that same horizontal extent rather than leaving an unoutlined inset on either side.
    let card = rect;
    ui.painter().rect_filled(card, 3.0, darker(t.card, CARD_BG_SHADE));
    if selected {
        ui.painter().rect_filled(card, 3.0, t.accent_soft);
    } else if hovered {
        ui.painter().rect_filled(card, 3.0, t.hover);
    }
    let pad = CARD_PAD * scale;
    let pad_y = CARD_PAD * scale_h;
    let name_h = if show_name && !name_in_top { CARD_NAME_H * scale_h } else { 0.0 };
    let top_h = if tip_only {
        (CARD_TIP_ONLY_H - CARD_PAD * 2.0) * scale_h
    } else if name_in_top {
        (CARD_TIP_NAME_H - CARD_PAD * 2.0) * scale_h
    } else if show_tip || show_stroke {
        (CARD_MAX_H - CARD_PAD * 2.0 - CARD_NAME_H) * scale_h
    } else {
        0.0
    };
    let inner = card.shrink2(vec2(pad, pad_y));
    let top = egui::Rect::from_min_size(inner.left_top(), vec2(inner.width(), top_h));
    let mut x = top.left();
    let brush = &preset.brush;
    let identity = preview_identity(preset);
    if show_tip {
        let tip_w = if tip_only { top.width() } else { (CARD_TIP_W * scale).min(top.width() * 0.5) };
        let cell = egui::Rect::from_min_size(top.left_top(), vec2(tip_w, top_h));
        let tip =
            brush_preview::tip_texture(ui.ctx(), &format!("brushes-tip:{identity}"), &brush.tip, (brush.hardness, brush.angle, brush.roundness), 36, t.text);
        let number =
            (!compact).then(|| ui.painter().layout_no_wrap(format!("{}", brush.size.round() as i64), egui::FontId::proportional(CARD_NUM_FONT), t.text_faint));
        let tip_rect = card_tip_rect(cell, number.as_ref().map(|g| g.size().y), brush.size, scale);
        ui.painter().image(tip.id(), tip_rect, full_uv(), Color32::WHITE);
        if let Some(galley) = number {
            ui.painter().galley(pos2(cell.center().x - galley.size().x / 2.0, cell.bottom() - 1.0 - galley.size().y), galley, t.text_faint);
        }
        x = cell.right();
    }
    if show_stroke || name_in_top {
        let cell = egui::Rect::from_min_max(pos2(x, top.top()), top.right_bottom());
        if show_stroke {
            let width = ((cell.width() - 16.0 * scale).clamp(24.0 * scale, 260.0 * scale)).min(cell.width()).max(1.0).round();
            // Card scale changes the destination rectangle, not the brush's stroke. Keep one
            // canonical preview per preset so dragging the footer slider doesn't rasterize and
            // upload a new texture for every intermediate width/height.
            let stroke = picker_stroke_texture(ui.ctx(), &identity, brush, t.text);
            let stroke_rect = egui::Rect::from_min_size(pos2(cell.left() + 8.0 * scale, cell.center().y - 18.0 * scale_h), vec2(width, 36.0 * scale_h));
            ui.painter().image(stroke.id(), stroke_rect, full_uv(), Color32::WHITE);
        } else {
            crate::layer_row_ui::label(
                ui.painter(),
                cell.left() + 8.0 * scale,
                cell.center().y,
                cell.right() - 6.0 * scale,
                &preset.name,
                egui::FontId::proportional(CARD_NAME_FONT),
                t.text_dim,
            );
        }
    }
    if name_h > 0.0 {
        let name_rect = egui::Rect::from_min_size(pos2(inner.left(), top.bottom()), vec2(inner.width(), name_h));
        crate::layer_row_ui::label(
            ui.painter(),
            name_rect.left() + 6.0 * scale,
            name_rect.center().y,
            name_rect.right() - 6.0 * scale,
            &preset.name,
            egui::FontId::proportional(CARD_NAME_FONT),
            t.text_dim,
        );
    }
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
    let label = crate::pickers::picker_state::escape_hierarchy_key_part(label);
    if group_path.is_empty() {
        label
    } else {
        format!("{label}/{}", group_path.iter().map(|part| crate::pickers::picker_state::escape_hierarchy_key_part(part)).collect::<Vec<_>>().join("/"))
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
                run_or_status(app, "brush.presets.delete", json!({ "name": name }));
            }
            Action::DeleteGroup { group, folder } => {
                let label = group_label_for_key(&app.session.tools.presets, &group);
                let mut p = json!({ "group": group });
                if !folder.is_empty() {
                    p["folder"] = json!(folder);
                }
                if run_or_status(app, "brush.presets.deleteGroup", p)
                    && let Some(label) = label
                {
                    remove_collapsed_prefix(&mut app.ui.brushes_panel.collapsed, &group_view_key(&label, &folder));
                }
            }
        }
    }
}

/// The gap between grid cells, across and down.
#[cfg(test)]
pub(crate) const GRID_GAP: f32 = 3.0;
/// The picker's cards pack tighter than the grid: their horizontal gap (the vertical one stays
/// [`GRID_GAP`]).
#[cfg(test)]
pub(crate) const CARD_GAP: f32 = 1.0;

/// The grid's columns in a list `width` points wide: how many `cell`-wide cells separated by
/// `gap` fit past the `indent`, and the left margin that centres them in the room past it. At
/// least one column.
#[cfg(test)]
pub(crate) fn grid_columns(width: f32, cell: f32, indent: f32, gap: f32) -> (f32, usize) {
    let room = if width.is_finite() { (width - indent).max(0.0) } else { 0.0 };
    let cols = if cell.is_finite() && cell > 0.0 { ((room + gap) / (cell + gap)).floor() } else { 1.0 };
    // `as` saturates (and NaN becomes 0), so this is a whole number from 1 to a sane bound.
    let cols = (cols as usize).clamp(1, 256);
    let used = cols as f32 * cell + (cols - 1) as f32 * gap;
    // Whole points keep the tiles' edges crisp.
    let left = indent + ((room - used) / 2.0).max(0.0).floor();
    (if left.is_finite() { left } else { 0.0 }, cols)
}

/// Padding, tip cell width and name row height, at scale 1; the footer slider multiplies them
/// (heights down to [`CARD_MIN_H_SCALE`]).
pub(crate) const CARD_PAD: f32 = 4.0;
pub(crate) const CARD_TIP_W: f32 = 46.0;
/// The tip thumbnail's base size at scale 1, before the footer slider and the per-preset
/// [`crate::brush_sections::thumb_scale`]: the tips read small at 28 px, so the base carries a
/// 60 % bump.
const CARD_TIP_BASE: f32 = 28.0 * 1.6;
pub(crate) const CARD_NAME_H: f32 = 20.0;
/// A full card's height at scale 1: the tip/stroke row takes what the name row leaves. The
/// footer slider multiplies it down to [`CARD_MIN_H_SCALE`].
pub(crate) const CARD_MAX_H: f32 = 70.0;
/// The widest a lone card grows at scale 1; past it the room is shared between columns.
pub(crate) const CARD_MAX_W: f32 = 300.0;
/// Tip alone: a grid of small [`CARD_TIP_ONLY_W`] × [`CARD_TIP_ONLY_H`] cells (scale 1).
pub(crate) const CARD_TIP_ONLY_W: f32 = 60.0;
pub(crate) const CARD_TIP_ONLY_H: f32 = 70.0;
/// Tip + name, no stroke: a grid of [`CARD_TIP_NAME_W`] × [`CARD_TIP_NAME_H`] cells (scale 1).
pub(crate) const CARD_TIP_NAME_W: f32 = 150.0;
pub(crate) const CARD_TIP_NAME_H: f32 = 70.0;
/// The footer scale's floor for heights and padding (the widths keep shrinking).
pub(crate) const CARD_MIN_H_SCALE: f32 = 0.60;
/// At or below this scale the tips drop their size numbers and fill their cell whole.
pub(crate) const CARD_COMPACT_SCALE: f32 = 0.70;
/// stay readable from the smallest card to the largest.
const CARD_NAME_FONT: f32 = 12.0;
pub(crate) const CARD_NUM_FONT: f32 = 11.0;
/// The cards' own fill: [`Tokens::card`] (what the picker's popup is filled with) this much
/// darker, so the cards read as tiles against it.
const CARD_BG_SHADE: f32 = 0.90;

/// Maximum footer scale that keeps a picker card within the available browser width.
pub(crate) fn max_picker_card_scale(width: f32, show: (bool, bool, bool)) -> f32 {
    let base_width = if tip_only(show) {
        CARD_TIP_ONLY_W
    } else if tip_and_name(show) {
        CARD_TIP_NAME_W
    } else {
        CARD_MAX_W
    };
    // The grid's base inset is 8 points. Keep the minimum slider scale available for narrow
    // pickers; for normal widths the upper bound is exactly where the card meets the container.
    ((width - 8.0).max(0.0) / base_width).clamp(*SCALE_RANGE.start(), *SCALE_RANGE.end())
}

/// Return the picker's canonical stroke texture. Card scale changes only the destination rectangle;
/// keeping the source dimensions fixed prevents a resize drag from rasterizing a new preview each
/// frame.
fn picker_stroke_texture(ctx: &egui::Context, identity: &str, brush: &photocraft_engine::BrushSettings, color: Color32) -> egui::TextureHandle {
    brush_preview::stroke_texture(ctx, &format!("brushes-stroke:{identity}"), brush, 260, 36, color)
}

/// The picker's cards in a list `width` points wide, `indent` past its left edge: how wide each
/// card is and how many sit in a row. A lone card fills the room up to [`CARD_MAX_W`] × `scale`
/// (wider lists leave the rest empty); from two columns on, the cards share the room, filling it
/// fully. `scale` is the picker's footer slider ([`crate::pickers::brush_picker::body`]), already floored
/// so the widths never reach zero.
/// Is the card showing the tip alone (no name, no stroke)? Then it's a small tip cell in a grid.
fn tip_only(show: (bool, bool, bool)) -> bool {
    show == (false, false, true)
}

/// Is the card showing the tip and the name with the stroke off? Then it's a tip-and-name cell.
fn tip_and_name(show: (bool, bool, bool)) -> bool {
    show == (true, false, true)
}

/// `c` darker by `f` in gamma space, kept opaque — [`Color32::gamma_multiply`] fades the alpha
/// along with the channels, so it can't darken a fill over its own color.
fn darker(c: Color32, f: f32) -> Color32 {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    Color32::from_rgba_unmultiplied((r as f32 * f).round() as u8, (g as f32 * f).round() as u8, (b as f32 * f).round() as u8, a)
}

/// Where a card's tip thumbnail goes in its `cell`. `number_h` is the height of the size number
/// under it, `None` on compact cards (no number: the tip takes the cell whole). Otherwise the
/// number keeps a band at the bottom of the cell and the tip is centred in what's left above it,
/// at [`CARD_TIP_BASE`] × `scale` or that room's side, whichever is smaller, times the preset's
/// [`crate::brush_sections::thumb_scale`]: big brushes still read bigger than small ones, and
/// none of them reaches down over the number.
pub(crate) fn card_tip_rect(cell: egui::Rect, number_h: Option<f32>, size: f32, scale: f32) -> egui::Rect {
    let Some(number_h) = number_h else {
        return egui::Rect::from_center_size(cell.center(), egui::Vec2::splat((cell.width().min(cell.height()) - 2.0).max(1.0)));
    };
    // The number sits 1 pt off the cell's bottom, with a 1 pt gap above it.
    let room = egui::Rect::from_min_max(cell.min, pos2(cell.right(), (cell.bottom() - number_h - 2.0).max(cell.top())));
    let side = (CARD_TIP_BASE * scale).min(room.width().min(room.height())).max(0.0) * crate::brush_sections::thumb_scale(size);
    egui::Rect::from_center_size(room.center(), egui::Vec2::splat(if side.is_finite() { side.max(1.0) } else { 1.0 }))
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
    let groups: Vec<GroupView> = group_views
        .iter()
        .map(|group| GroupView {
            name: group.view.name.clone(),
            root_only: false,
            items: group
                .view
                .items
                .iter()
                .map(|item| ItemView { key: item.key.clone(), name: item.name.clone(), group_path: item.group_path.clone() })
                .collect(),
        })
        .collect();
    let identities = brush_hierarchy_identities(&groups);
    let mut hierarchy_actions = Vec::new();
    let mut canonical_move = |intent: &DropMoveIntent| {
        let action = brush_drop_intent_to_action(presets, &identities, intent)?;
        hierarchy_actions.push(action);
        Ok(())
    };
    let list = st.view == BrushesView::List;
    let mut paint_item = |ui: &egui::Ui, rect: egui::Rect, group_index: usize, item_index: usize, state: ItemPaintState| {
        let Some(preset) = group_views.get(group_index).and_then(|group| group.presets.get(item_index)).copied() else { return };
        let selected = matches!(state, ItemPaintState::Item { selected: true });
        if list {
            paint_brush_list_row(ui, rect, preset, false);
        } else {
            paint_brush_grid_item(ui, rect, preset, selected);
        }
    };
    let mut paint_list_item = |ui: &egui::Ui, rect: egui::Rect, group_index: usize, item_index: usize, selected: bool| {
        if let Some(preset) = group_views.get(group_index).and_then(|group| group.presets.get(item_index)).copied() {
            paint_brush_list_row(ui, rect, preset, selected);
        }
    };
    let mut adapter = BrowserAdapter {
        panel: PANEL,
        groups: &groups,
        options: BrowserOptions {
            canvas: egui::Rect::NOTHING,
            flush: false,
            center_grid: true,
            grid_indent: layout.indent,
            all_groups_open: true,
            compact_context_menus: false,
            item_size: Some(vec2(layout.cell, layout.cell + 6.0)),
            list_item_height: Some(44.0),
            domain_hierarchy_actions: true,
            root_item_group: None,
        },
        // Use the same canonical path-aware hierarchy as the floating Brush picker.
        capabilities: BrowserCapabilities {
            item_drag: true,
            item_drop: true,
            group_drag: true,
            selection: true,
            activation: true,
            canonical_hierarchy_drag: true,
        },
        item_context_menu: Some(brush_item_context_menu),
        group_context_menu: Some(brush_group_context_menu),
        nested_group_context_menu: Some(brush_nested_group_context_menu),
        canonical_hierarchy_move: Some(&mut canonical_move),
        hierarchy_item_move: None,
        paint_group: None,
        paint_item: &mut paint_item,
        paint_list_item: list.then_some(&mut paint_list_item as &mut crate::pickers::asset_browser::PaintListItem<'_>),
    };
    let browser_actions = asset_browser::browser(ui, preset_ui, &mut adapter, layout.max_height);
    drop(adapter);
    drop(canonical_move);
    sync_browser_state_to_brush_state(st, preset_ui, presets, PANEL);
    paint_brush_hierarchy_drag_label(ui, PANEL, egui::Id::new(("brush-drag-label", layout.id)));
    let mut actions = browser_actions_to_actions(presets, browser_actions);
    actions.extend(hierarchy_actions);
    actions
}

/// Bridge the Brushes panel's persisted public state to the shared transient browser state. The
/// legacy JSON shape stays stable while the browser owns the live interaction model.
fn sync_brush_state_to_browser(st: &BrushesPanelState, browser: &mut PresetUi, presets: &[BrushPreset], current: Option<&str>, panel: &str) {
    browser.search.insert(panel.to_string(), st.filter.clone());
    browser.sizes.insert(panel.to_string(), if st.view == BrushesView::List { asset_browser::MIN_THUMB } else { 40.0 });
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

/// Keep brush group labels visible while dragging hierarchy nodes. Item artwork is painted
/// by the shared browser so its ghost stays identical to the source card.
fn paint_brush_hierarchy_drag_label(ui: &egui::Ui, scope: &str, id: egui::Id) {
    let Some(drag) = egui::DragAndDrop::payload::<BrushDrag>(ui.ctx()) else { return };
    let (drag_scope, name) = match &*drag {
        BrushDrag::Group { scope, name, path } => {
            (scope.as_str(), path.last().cloned().unwrap_or_else(|| if name.is_empty() { UNGROUPED.to_string() } else { name.clone() }))
        }
        BrushDrag::CanonicalGroup { scope, node_id } => {
            let name = match node_id {
                NodeId::Group(name) => name.clone(),
                NodeId::GroupPath(path) => path.last().cloned().unwrap_or_default(),
                _ => return,
            };
            (scope.as_str(), name)
        }
        BrushDrag::Preset { .. } => return,
    };
    if drag_scope != scope {
        return;
    }
    let Some(pointer) = ui.ctx().pointer_interact_pos() else { return };
    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    egui::Area::new(id).order(egui::Order::Tooltip).fixed_pos(pointer + vec2(12.0, 8.0)).interactable(false).show(ui.ctx(), |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| ui.label(name));
    });
}

#[cfg(test)]
mod preview_regression_tests {
    use super::{CARD_MAX_W, CARD_TIP_NAME_W, CARD_TIP_ONLY_W, max_picker_card_scale, paint_picker_card};
    use photocraft_engine::paint::BrushPreset;

    #[test]
    fn resizing_picker_preview_reuses_the_cached_stroke_texture() {
        let ctx = egui::Context::default();
        let preset = BrushPreset {
            name: "Regression brush".into(),
            brush: Default::default(),
            builtin: false,
            group: "Test".into(),
            folder: Vec::new(),
            captures: Default::default(),
        };

        // Exercise the actual card painter over successive UI frames, rather than testing a helper
        // that could remain correct while the production call site regresses to scale-keyed slots.
        let mut renders_after_first_frame = None;
        for scale in [0.6_f32, 0.75, 1.0, 1.25, 1.5, 2.0] {
            let mut output = ctx.run_ui(Default::default(), |ui| {
                paint_picker_card(ui, ui.max_rect(), &preset, false, false, (true, true, true), scale);
            });
            output.textures_delta.clear();
            let renders = crate::brush_preview::render_count(&ctx);
            if let Some(expected) = renders_after_first_frame {
                assert_eq!(renders, expected, "resizing to scale {scale} re-rendered a preview");
            } else {
                renders_after_first_frame = Some(renders);
            }
        }
    }

    #[test]
    fn picker_card_width_never_exceeds_its_container() {
        let modes = [
            ((false, false, true), CARD_TIP_ONLY_W),
            ((true, false, true), CARD_TIP_NAME_W),
            ((true, true, true), CARD_MAX_W),
            ((false, true, true), CARD_MAX_W),
            ((true, true, false), CARD_MAX_W),
        ];

        for width in [64.0_f32, 96.0, 160.0, 240.0, 360.0, 640.0] {
            for (show, base_card_width) in modes {
                let scale = max_picker_card_scale(width, show);
                let card_width = base_card_width * scale;
                assert!(card_width <= width, "card width {card_width} exceeds container width {width} for mode {show:?} at scale {scale}");
            }
        }
    }
}

#[cfg(test)]
mod hierarchy_anchor_identity_tests {
    use super::{Action, HierarchyIdentity, Renaming, browser_actions_to_actions, brush_drop_intent_to_action, picker_browser, preview_identity};
    use crate::PhotocraftApp;
    use crate::pickers::asset_browser::BrowserAction;
    use crate::pickers::picker_browser_dragging::DropMoveIntent;
    use crate::pickers::picker_browser_model::{DropPosition, NodeId};
    use crate::pickers::picker_state::PresetUi;
    use photocraft_engine::paint::BrushPreset;
    use serde_json::json;

    fn preset(name: &str, group: &str) -> BrushPreset {
        BrushPreset { name: name.into(), brush: Default::default(), builtin: false, group: group.into(), folder: Vec::new(), captures: Default::default() }
    }

    #[test]
    fn group_path_rename_state_keeps_the_legacy_folder_json_key() {
        let rename: Renaming = serde_json::from_value(json!({ "group": true, "name": "Kit", "folder": ["Soft"], "text": "Gentle" })).unwrap();
        assert_eq!(rename.folder, ["Soft"]);

        let saved = serde_json::to_value(rename).unwrap();
        assert_eq!(saved["folder"], json!(["Soft"]));
        assert!(saved.get("groupPath").is_none());
    }

    #[test]
    fn hierarchy_anchor_converts_preview_identity_to_engine_preset_name() {
        let presets = [preset("Parent A", "Parent"), preset("Anchor brush", "Parent"), preset("Child brush", "Child")];
        let anchor_key = preview_identity(&presets[1]);

        let actions = browser_actions_to_actions(
            &presets,
            vec![BrowserAction::MoveInto {
                group: "Child".into(),
                group_path: Vec::new(),
                into: Some(("Parent".into(), Vec::new())),
                before: None,
                before_item: Some(anchor_key),
            }],
        );

        assert!(matches!(actions.as_slice(), [Action::MoveInto { before_item: Some(name), .. }] if name == "Anchor brush"));
    }

    #[test]
    fn hierarchy_anchor_rejects_duplicate_engine_names() {
        let presets = [preset("Anchor brush", "Parent"), preset("Anchor brush", "Other"), preset("Child brush", "Child")];
        let anchor_key = preview_identity(&presets[0]);

        let actions = browser_actions_to_actions(
            &presets,
            vec![BrowserAction::MoveInto {
                group: "Child".into(),
                group_path: Vec::new(),
                into: Some(("Parent".into(), Vec::new())),
                before: None,
                before_item: Some(anchor_key),
            }],
        );

        assert!(matches!(actions.as_slice(), [Action::Status(message)] if message.contains("multiple presets share that name")));
    }

    #[test]
    fn after_folder_item_move_anchors_before_the_next_direct_preset() {
        let presets = [
            BrushPreset { folder: vec!["A".into()], ..preset("A child", "Kit") },
            preset("Direct sibling", "Kit"),
            BrushPreset { folder: vec!["C".into()], ..preset("C child", "Kit") },
            BrushPreset { folder: vec!["Source".into()], ..preset("Moving brush", "Kit") },
        ];
        let source_key = preview_identity(&presets[3]);
        let source = NodeId::item_in_path(["Kit", "Source"], source_key.clone());
        let target = NodeId::group_path(["Kit"], "A");
        let identities = [
            (source.clone(), HierarchyIdentity::FolderItem { group: "Kit".into(), path: vec!["Source".into()], key: source_key }),
            (target.clone(), HierarchyIdentity::Folder { group: "Kit".into(), path: vec!["A".into()] }),
        ]
        .into_iter()
        .collect();

        let action = brush_drop_intent_to_action(&presets, &identities, &DropMoveIntent { source, position: DropPosition::After { target } }).unwrap();
        assert!(matches!(action, Action::Move { name, folder: Some(path), index: Some(1), .. } if name == "Moving brush" && path.is_empty()));
    }

    #[test]
    fn after_folder_subtree_move_uses_the_next_direct_preset_anchor() {
        let presets = [
            BrushPreset { folder: vec!["A".into()], ..preset("A child", "Kit") },
            preset("Direct sibling", "Kit"),
            BrushPreset { folder: vec!["C".into()], ..preset("C child", "Kit") },
            BrushPreset { folder: vec!["Source".into()], ..preset("Source child", "Kit") },
        ];
        let source = NodeId::group_path(["Kit"], "Source");
        let target = NodeId::group_path(["Kit"], "A");
        let identities = [
            (source.clone(), HierarchyIdentity::Folder { group: "Kit".into(), path: vec!["Source".into()] }),
            (target.clone(), HierarchyIdentity::Folder { group: "Kit".into(), path: vec!["A".into()] }),
        ]
        .into_iter()
        .collect();

        let action = brush_drop_intent_to_action(&presets, &identities, &DropMoveIntent { source, position: DropPosition::After { target } }).unwrap();
        assert!(matches!(action, Action::MoveInto { group, folder, into: Some((target, path)), before: None, before_item: Some(anchor) }
            if group == "Kit" && folder == vec!["Source".to_string()] && target == "Kit" && path.is_empty() && anchor == "Direct sibling"));
    }

    #[test]
    fn dropping_a_brush_onto_a_group_places_it_before_the_first_child() {
        let presets = [preset("First", "Kit"), preset("Second", "Kit"), preset("Moving", "Other")];
        let source_key = preview_identity(&presets[2]);
        let source = NodeId::item("Other", source_key.clone());
        let target = NodeId::group("Kit");
        let identities =
            [(source.clone(), HierarchyIdentity::Item { group: "Other".into(), key: source_key }), (target.clone(), HierarchyIdentity::Group("Kit".into()))]
                .into_iter()
                .collect();

        let action =
            brush_drop_intent_to_action(&presets, &identities, &DropMoveIntent { source, position: DropPosition::PrependTo { group: target } }).unwrap();
        assert!(matches!(action, Action::Move { name, group, folder: Some(path), index: Some(0) }
            if name == "Moving" && group == "Kit" && path.is_empty()));
    }

    #[test]
    fn dropping_a_brush_onto_its_own_folder_moves_it_to_the_first_child_slot() {
        let presets =
            [BrushPreset { folder: vec!["Folder".into()], ..preset("First", "Kit") }, BrushPreset { folder: vec!["Folder".into()], ..preset("Moving", "Kit") }];
        let source_key = preview_identity(&presets[1]);
        let source = NodeId::item_in_path(["Kit", "Folder"], source_key.clone());
        let target = NodeId::group_path(["Kit"], "Folder");
        let identities = [
            (source.clone(), HierarchyIdentity::FolderItem { group: "Kit".into(), path: vec!["Folder".into()], key: source_key }),
            (target.clone(), HierarchyIdentity::Folder { group: "Kit".into(), path: vec!["Folder".into()] }),
        ]
        .into_iter()
        .collect();

        let action =
            brush_drop_intent_to_action(&presets, &identities, &DropMoveIntent { source, position: DropPosition::PrependTo { group: target } }).unwrap();
        assert!(matches!(action, Action::Move { name, group, folder: Some(path), index: Some(0) }
            if name == "Moving" && group == "Kit" && path == vec!["Folder".to_string()]));
    }

    #[test]
    fn dropping_a_folder_onto_a_group_anchors_before_its_first_child_folder() {
        let presets = [
            BrushPreset { folder: vec!["First folder".into()], ..preset("First child", "Kit") },
            preset("Direct child", "Kit"),
            BrushPreset { folder: vec!["Source".into()], ..preset("Source child", "Other") },
        ];
        let source = NodeId::group_path(["Other"], "Source");
        let target = NodeId::group("Kit");
        let identities = [
            (source.clone(), HierarchyIdentity::Folder { group: "Other".into(), path: vec!["Source".into()] }),
            (target.clone(), HierarchyIdentity::Group("Kit".into())),
        ]
        .into_iter()
        .collect();

        let action =
            brush_drop_intent_to_action(&presets, &identities, &DropMoveIntent { source, position: DropPosition::PrependTo { group: target } }).unwrap();
        assert!(matches!(action, Action::MoveInto { group, folder, into: Some((target, path)), before: Some(before), before_item: None }
            if group == "Other" && folder == vec!["Source".to_string()] && target == "Kit" && path.is_empty() && before == "First folder"));
    }

    #[test]
    fn brush_picker_browser_moves_a_root_group_into_a_nested_group_once() {
        use egui::{Event, Modifiers, PointerButton, vec2};
        use egui_kittest::{Harness, kittest::Queryable};

        let presets = vec![
            BrushPreset {
                name: "Nested brush".into(),
                brush: Default::default(),
                builtin: false,
                group: "Group A".into(),
                folder: vec!["Nested".into()],
                captures: Default::default(),
            },
            BrushPreset {
                name: "Moving brush".into(),
                brush: Default::default(),
                builtin: false,
                group: "Group B".into(),
                folder: Vec::new(),
                captures: Default::default(),
            },
        ];
        let mut h = Harness::builder().with_size(vec2(420.0, 520.0)).build_ui_state(
            move |ui, state: &mut (PresetUi, Vec<Action>)| {
                if ui.ctx().fonts(|fonts| fonts.definitions().families.contains_key(&egui::FontFamily::Name("semibold".into()))) {
                    state.1.extend(picker_browser(ui, &presets, None, &mut state.0, 500.0));
                }
            },
            (PresetUi::default(), Vec::new()),
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
        h.run_steps(3);

        let source = crate::pickers::asset_browser::last_group_headers(&h.ctx, "brush-picker")
            .into_iter()
            .find(|(name, _)| name == "Group B")
            .map(|(_, rect)| rect.center())
            .expect("the source root group header is visible");
        let target = h.get_by_label("Nested").rect().center();
        h.event(Event::PointerMoved(source));
        h.run_steps(1);
        h.event(Event::PointerButton { pos: source, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(1);
        for step in 1..=6 {
            h.event(Event::PointerMoved(source + (target - source) * (step as f32 / 6.0)));
            h.run_steps(1);
        }
        h.event(Event::PointerButton { pos: target, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(3);

        assert!(matches!(
            h.state().1.as_slice(),
            [Action::MoveInto { group, folder, into: Some((target, path)), before: None, before_item: Some(anchor) }]
                if group == "Group B" && folder.is_empty() && target == "Group A" && path == &["Nested".to_string()] && anchor == "Nested brush"
        ), "unexpected brush browser actions: {:?}", h.state().1);
        assert_eq!(h.state().1.len(), 1, "the nested header resolves exactly one group move");
    }
}
