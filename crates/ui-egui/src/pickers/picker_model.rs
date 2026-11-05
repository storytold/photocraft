//! Canonical, asset-neutral data model for picker browsers.
//!
//! This module is intentionally independent of the current picker implementation. It defines
//! identity, ownership and visual placement without adapting or mutating any existing library.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

/// Stable identity of an asset. The value is assigned by the owning library and must survive
/// renames, moves and reordering. The picker never derives identity from an asset's name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct AssetId(pub(crate) u128);

/// Stable identity of a source library (brushes, gradients, patterns, and so on).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct LibraryId(pub(crate) u128);

/// Stable identity of a browser folder. A folder is presentation structure, not asset ownership.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct GroupId(pub(crate) u128);

/// Minimal picker-facing asset data. The owning domain remains authoritative for the payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Asset {
    pub(crate) id: AssetId,
    pub(crate) library: LibraryId,
    pub(crate) label: String,
}

/// A named visual group. Its children define its parentage and sibling order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Group {
    pub(crate) id: GroupId,
    pub(crate) label: String,
    pub(crate) children: Vec<Node>,
}

/// Groups and assets are peers in every ordered child list, including the root list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Node {
    Group(GroupId),
    Asset(AssetId),
}

/// A complete picker projection. Hash maps provide direct identity lookup; child vectors provide
/// deterministic display order. Neither map iteration order nor labels carry semantic meaning.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PickerModel {
    pub(crate) roots: Vec<Node>,
    pub(crate) groups: HashMap<GroupId, Group>,
    pub(crate) assets: HashMap<AssetId, Asset>,
}

/// A malformed or internally inconsistent picker model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ModelError {
    MissingGroup(GroupId),
    MissingAsset(AssetId),
    GroupKeyMismatch(GroupId),
    AssetKeyMismatch(AssetId),
    DuplicateGroup(GroupId),
    DuplicateAsset(AssetId),
    GroupCycle(GroupId),
    UnplacedGroup(GroupId),
    UnplacedAsset(AssetId),
}

impl PickerModel {
    pub(crate) fn asset(&self, id: AssetId) -> Option<&Asset> {
        self.assets.get(&id)
    }

    pub(crate) fn group(&self, id: GroupId) -> Option<&Group> {
        self.groups.get(&id)
    }

    /// Check that every record is reachable exactly once and all references resolve.
    /// This is intended for construction boundaries and tests, not per-frame rendering.
    pub(crate) fn validate(&self) -> Result<(), ModelError> {
        for (id, asset) in &self.assets {
            if *id != asset.id {
                return Err(ModelError::AssetKeyMismatch(*id));
            }
        }
        for (id, group) in &self.groups {
            if *id != group.id {
                return Err(ModelError::GroupKeyMismatch(*id));
            }
        }

        let mut seen_groups = HashSet::with_capacity(self.groups.len());
        let mut seen_assets = HashSet::with_capacity(self.assets.len());
        let mut active_groups = HashSet::new();
        let mut pending = Vec::with_capacity(self.roots.len());
        pending.extend(self.roots.iter().rev().copied().map(Visit::Node));
        while let Some(visit) = pending.pop() {
            match visit {
                Visit::ExitGroup(id) => {
                    active_groups.remove(&id);
                }
                Visit::Node(Node::Asset(id)) => {
                    if !self.assets.contains_key(&id) {
                        return Err(ModelError::MissingAsset(id));
                    }
                    if !seen_assets.insert(id) {
                        return Err(ModelError::DuplicateAsset(id));
                    }
                }
                Visit::Node(Node::Group(id)) => {
                    if active_groups.contains(&id) {
                        return Err(ModelError::GroupCycle(id));
                    }
                    if !seen_groups.insert(id) {
                        return Err(ModelError::DuplicateGroup(id));
                    }
                    let group = self.groups.get(&id).ok_or(ModelError::MissingGroup(id))?;
                    active_groups.insert(id);
                    pending.push(Visit::ExitGroup(id));
                    pending.extend(group.children.iter().rev().copied().map(Visit::Node));
                }
            }
        }

        if let Some(id) = self.groups.keys().find(|id| !seen_groups.contains(id)) {
            return Err(ModelError::UnplacedGroup(*id));
        }
        if let Some(id) = self.assets.keys().find(|id| !seen_assets.contains(id)) {
            return Err(ModelError::UnplacedAsset(*id));
        }
        Ok(())
    }
}

enum Visit {
    Node(Node),
    ExitGroup(GroupId),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(id: u128, library: u128, label: &str) -> Asset {
        Asset { id: AssetId(id), library: LibraryId(library), label: label.into() }
    }

    #[test]
    fn root_assets_and_nested_groups_share_one_ordered_node_model() {
        let mut model = PickerModel::default();
        model.assets.insert(AssetId(1), asset(1, 10, "Loose brush"));
        model.assets.insert(AssetId(2), asset(2, 20, "Gradient"));
        model.assets.insert(AssetId(3), asset(3, 10, "Brush in folder"));
        model.groups.insert(GroupId(7), Group { id: GroupId(7), label: "Favorites".into(), children: vec![Node::Asset(AssetId(3))] });
        model.roots = vec![Node::Asset(AssetId(1)), Node::Group(GroupId(7)), Node::Asset(AssetId(2))];

        assert_eq!(model.validate(), Ok(()));
        assert_eq!(model.asset(AssetId(2)).map(|item| item.library), Some(LibraryId(20)));
        assert_eq!(model.group(GroupId(7)).map(|group| group.children.len()), Some(1));
    }

    #[test]
    fn validation_rejects_duplicate_placement() {
        let mut model = PickerModel::default();
        model.assets.insert(AssetId(1), asset(1, 10, "Duplicated"));
        model.roots = vec![Node::Asset(AssetId(1)), Node::Asset(AssetId(1))];

        assert_eq!(model.validate(), Err(ModelError::DuplicateAsset(AssetId(1))));
    }

    #[test]
    fn validation_rejects_unplaced_records() {
        let mut model = PickerModel::default();
        model.assets.insert(AssetId(1), asset(1, 10, "Not in tree"));

        assert_eq!(model.validate(), Err(ModelError::UnplacedAsset(AssetId(1))));
    }

    #[test]
    fn validation_rejects_group_cycles() {
        let mut model = PickerModel::default();
        model.groups.insert(GroupId(7), Group { id: GroupId(7), label: "Cycle".into(), children: vec![Node::Group(GroupId(7))] });
        model.roots = vec![Node::Group(GroupId(7))];

        assert_eq!(model.validate(), Err(ModelError::GroupCycle(GroupId(7))));
    }
}
