//! Adapter-owned stable identities and reverse metadata for shared picker projection.
//!
//! The allocator treats `PickerIdentityKey` as an opaque token. Domain adapters create tokens and
//! supply the reverse metadata used by their own action/persistence translation; the shared tree
//! builder only copies this metadata alongside the runtime IDs it allocates.

use std::collections::{BTreeMap, BTreeSet};

/// Stable identity token supplied by one domain adapter. Its contents are never interpreted here.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct PickerIdentityKey(String);

impl PickerIdentityKey {
    /// Collision-free token encoding for adapter-owned components without imposing their meaning.
    pub(crate) fn from_parts<'a>(namespace: &str, parts: impl IntoIterator<Item = &'a str>) -> Self {
        let mut encoded = format!("{}:{}", namespace.len(), namespace);
        for part in parts {
            encoded.push_str(&format!("{}:{}", part.len(), part));
        }
        Self(encoded)
    }
}

/// Structural coordinate in the current adapter projection; it is not an identity.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum ProjectionNodeRef {
    Group(usize),
    Folder { group_index: usize, path: Vec<String> },
    Item { group_index: usize, item_index: usize },
}

/// Adapter-supplied reverse metadata for translating a transient `PickerId` back to its domain
/// target. It is never used to allocate IDs; stable identity comes exclusively from the opaque
/// `PickerIdentityKey` supplied by the adapter.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum PickerNodeIdentity {
    Item { group: String, key: String },
    Group(String),
    Folder { group: String, path: Vec<String> },
    FolderItem { group: String, path: Vec<String>, key: String },
}

/// Structural role needed by the generic renderer, independent of an adapter's domain identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PickerNodeRole {
    AssetGroup,
    StructuralFolder,
    Card,
}

impl PickerNodeIdentity {
    pub(crate) fn role(&self) -> PickerNodeRole {
        match self {
            Self::Group(_) => PickerNodeRole::AssetGroup,
            Self::Folder { .. } => PickerNodeRole::StructuralFolder,
            Self::Item { .. } | Self::FolderItem { .. } => PickerNodeRole::Card,
        }
    }
}

#[derive(Clone, Debug)]
struct ProjectionIdentity {
    key: PickerIdentityKey,
    reverse: PickerNodeIdentity,
}

/// Stable identity and reverse translation supplied by an adapter for one projected view.
#[derive(Clone, Debug, Default)]
pub(crate) struct PickerProjectionKeys {
    nodes: BTreeMap<ProjectionNodeRef, ProjectionIdentity>,
}

impl PickerProjectionKeys {
    pub(crate) fn insert(&mut self, node: ProjectionNodeRef, key: PickerIdentityKey, reverse: PickerNodeIdentity) {
        self.nodes.insert(node, ProjectionIdentity { key, reverse });
    }

    pub(crate) fn key(&self, node: &ProjectionNodeRef) -> Option<&PickerIdentityKey> {
        self.nodes.get(node).map(|identity| &identity.key)
    }

    pub(crate) fn reverse(&self, node: &ProjectionNodeRef) -> Option<&PickerNodeIdentity> {
        self.nodes.get(node).map(|identity| &identity.reverse)
    }

    pub(crate) fn all_keys(&self) -> BTreeSet<PickerIdentityKey> {
        self.nodes.values().map(|identity| identity.key.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{PickerIdentityKey, PickerNodeIdentity, PickerNodeRole, PickerProjectionKeys, ProjectionNodeRef};

    #[test]
    fn structural_role_is_separate_from_adapter_reverse_identity() {
        let group = PickerNodeIdentity::Group("Library".into());
        let folder = PickerNodeIdentity::Folder { group: "Library".into(), path: vec!["Nested".into()] };
        let card = PickerNodeIdentity::FolderItem { group: "Library".into(), path: vec!["Nested".into()], key: "asset-id".into() };
        assert_eq!(group.role(), PickerNodeRole::AssetGroup);
        assert_eq!(folder.role(), PickerNodeRole::StructuralFolder);
        assert_eq!(card.role(), PickerNodeRole::Card);
    }

    #[test]
    fn adapter_projection_keeps_identity_tokens_opaque_and_reverse_metadata_adapter_owned() {
        let node = ProjectionNodeRef::Item { group_index: 3, item_index: 7 };
        let token = PickerIdentityKey::from_parts("patterns", ["library-id", "pattern-id"]);
        let reverse = PickerNodeIdentity::Item { group: "Pattern group".into(), key: "pattern-id".into() };
        let mut projection = PickerProjectionKeys::default();
        projection.insert(node.clone(), token.clone(), reverse.clone());

        assert_eq!(projection.key(&node), Some(&token));
        assert_eq!(projection.reverse(&node), Some(&reverse));
        assert_eq!(projection.all_keys().into_iter().collect::<Vec<_>>(), [token]);
    }

    #[test]
    fn adapter_token_parts_are_unambiguous() {
        assert_ne!(PickerIdentityKey::from_parts("domain", ["ab", "c"]), PickerIdentityKey::from_parts("domain", ["a", "bc"]));
        assert_ne!(PickerIdentityKey::from_parts("domain", ["a/b", "c"]), PickerIdentityKey::from_parts("domain", ["a", "b/c"]));
    }
}
