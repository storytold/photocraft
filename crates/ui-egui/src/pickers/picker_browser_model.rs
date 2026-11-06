//! Canonical, asset-neutral hierarchy model for picker browsers.
//!
//! This is the browser's runtime model, not a replacement for an asset library's persistence
//! schema. Adapters project domain assets into this tree and translate validated moves back into
//! domain commands. Root and group containers both hold an ordered mixture of cards and groups.

#[cfg(test)]
use std::fmt;
#[cfg(test)]
use std::hash::{Hash, Hasher};
#[cfg(test)]
use std::sync::{Mutex, OnceLock};

use super::picker_browser_identity::PickerIdentityKey;

/// Maximum number of nested groups, with the root group at depth zero.
pub(crate) const MAX_GROUP_DEPTH: usize = 32;

/// Opaque identity allocated by one picker session. It intentionally contains no domain data.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct PickerId {
    scope: u64,
    value: u64,
}

/// One allocator belongs to one picker instance and allocates IDs for cards and groups together.
#[derive(Clone, Debug, Default)]
pub(crate) struct PickerIdRegistry {
    ids: std::collections::BTreeMap<PickerIdentityKey, PickerId>,
    scope: u64,
    next: u64,
}

impl PickerIdRegistry {
    pub(crate) fn with_scope(scope: u64) -> Self {
        Self { ids: std::collections::BTreeMap::new(), scope, next: 0 }
    }

    pub(crate) fn id_for(&mut self, key: PickerIdentityKey) -> PickerId {
        if let Some(id) = self.ids.get(&key) {
            return id.clone();
        }
        #[cfg(test)]
        if self.scope == 0 {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            key.hash(&mut hasher);
            let mut value = hasher.finish().max(1);
            while self.ids.values().any(|id| id.value == value) {
                value = value.wrapping_add(1).max(1);
            }
            let id = PickerId { scope: self.scope, value };
            self.ids.insert(key, id.clone());
            return id;
        }
        self.next = self.next.wrapping_add(1).max(1);
        let id = PickerId { scope: self.scope, value: self.next };
        self.ids.insert(key, id.clone());
        id
    }

    pub(crate) fn rekey(&mut self, old: PickerIdentityKey, new: PickerIdentityKey, id: &PickerId) {
        // Keep both keys until the next projection confirms which domain identity survived.
        // Adapter callbacks can enqueue a domain action before the application model commits it;
        // retaining the old key makes a rejected action harmless, while a successful projection
        // resolves through `new` and prunes the stale alias.
        if self.ids.get(&old) == Some(id) && self.ids.get(&new).is_none_or(|existing| existing == id) {
            self.ids.insert(new, id.clone());
        }
    }

    pub(crate) fn retain_keys(&mut self, live: &std::collections::BTreeSet<PickerIdentityKey>) {
        self.ids.retain(|key, _| live.contains(key));
    }
}

impl PickerId {
    #[cfg(test)]
    fn test(value: u64, label: impl Into<String>) -> Self {
        let id = Self { scope: 0, value };
        test_id_labels().lock().unwrap().insert((id.scope, id.value), label.into());
        id
    }

    #[cfg(test)]
    fn test_key(key: PickerIdentityKey, label: impl Into<String>) -> Self {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hasher);
        Self::test(hasher.finish().max(1), label)
    }

    // Test-only conveniences keep the pure hierarchy model tests readable. Production projection
    // must obtain IDs from PickerIdRegistry instead.
    #[cfg(test)]
    pub(crate) fn new(value: impl AsRef<str>) -> Self {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        value.as_ref().hash(&mut hasher);
        Self::test(hasher.finish().max(1), value.as_ref())
    }

    #[cfg(test)]
    pub(crate) fn group(name: impl AsRef<str>) -> Self {
        let name = name.as_ref();
        Self::test_key(PickerIdentityKey::from_parts("test-group", [name]), name)
    }
    #[cfg(test)]
    pub(crate) fn group_path(path: impl IntoIterator<Item = impl Into<String>>, name: impl Into<String>) -> Self {
        let parts = path.into_iter().map(Into::into).collect::<Vec<String>>();
        let name = name.into();
        let group = parts.first().cloned().unwrap_or_default();
        let folder_path: Vec<String> = parts.into_iter().skip(1).chain(std::iter::once(name.clone())).collect();
        Self::test_key(PickerIdentityKey::from_parts("test-folder", std::iter::once(group.as_str()).chain(folder_path.iter().map(String::as_str))), name)
    }
    #[cfg(test)]
    pub(crate) fn item(group: impl Into<String>, key: impl Into<String>) -> Self {
        let group = group.into();
        let key = key.into();
        Self::test_key(PickerIdentityKey::from_parts("test-item", [group.as_str(), key.as_str()]), key)
    }
    #[cfg(test)]
    pub(crate) fn item_in_path(path: impl IntoIterator<Item = impl Into<String>>, key: impl Into<String>) -> Self {
        let parts = path.into_iter().map(Into::into).collect::<Vec<String>>();
        let group = parts.first().cloned().unwrap_or_default();
        let folder_path: Vec<String> = parts.into_iter().skip(1).collect();
        let key = key.into();
        Self::test_key(
            PickerIdentityKey::from_parts(
                "test-folder-item",
                std::iter::once(group.as_str()).chain(folder_path.iter().map(String::as_str)).chain(std::iter::once(key.as_str())),
            ),
            key,
        )
    }
}

#[cfg(test)]
fn test_id_labels() -> &'static Mutex<std::collections::BTreeMap<(u64, u64), String>> {
    static LABELS: OnceLock<Mutex<std::collections::BTreeMap<(u64, u64), String>>> = OnceLock::new();
    LABELS.get_or_init(|| Mutex::new(std::collections::BTreeMap::new()))
}

#[cfg(test)]
impl fmt::Display for PickerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(label) = test_id_labels().lock().unwrap().get(&(self.scope, self.value)) {
            f.write_str(label)
        } else {
            write!(f, "picker:{}:{}", self.scope, self.value)
        }
    }
}

#[cfg(test)]
impl From<&str> for PickerId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

#[cfg(test)]
impl From<String> for PickerId {
    fn from(value: String) -> Self {
        Self::from(value.as_str())
    }
}

/// One node in the ordered hierarchy. Items cannot own children by construction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum HierarchyNode {
    /// A draggable asset card.
    Item(PickerId),
    /// A container that may hold cards and other groups.
    Group { id: PickerId, children: Vec<HierarchyNode> },
}

impl HierarchyNode {
    /// Construct an item node.
    pub(crate) fn item(id: impl Into<PickerId>) -> Self {
        Self::Item(id.into())
    }

    /// Construct a group node with ordered children.
    pub(crate) fn group(id: impl Into<PickerId>, children: Vec<Self>) -> Self {
        Self::Group { id: id.into(), children }
    }

    /// Stable identity supplied by the asset adapter.
    pub(crate) fn id(&self) -> &PickerId {
        match self {
            Self::Item(id) | Self::Group { id, .. } => id,
        }
    }

    /// Whether this node is a group container.
    pub(crate) fn is_group(&self) -> bool {
        matches!(self, Self::Group { .. })
    }

    /// Ordered children; items always return an empty slice.
    pub(crate) fn children(&self) -> &[HierarchyNode] {
        match self {
            Self::Item(_) => &[],
            Self::Group { children, .. } => children,
        }
    }
}

/// A container that can receive an appended child.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ContainerId {
    /// The top-level browser sequence.
    Root,
    /// The child sequence owned by a group.
    Group(PickerId),
}

/// Exact structural destination for a drag operation.
///
/// Before and after are sibling insertions. Prepend and append name a container directly and
/// distinguish the first-child drop target from an explicit end-of-container drop target.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DropPosition {
    /// Insert immediately before the target in its current parent's ordered children.
    Before { target: PickerId },
    /// Insert immediately after the target in its current parent's ordered children.
    After { target: PickerId },
    /// Insert as the first child of a group.
    PrependTo { group: PickerId },
    /// Insert as the final child of a container.
    AppendTo { container: ContainerId },
}

/// A drop resolved against one specific hierarchy snapshot.
///
/// The insertion index is already adjusted for removal when source and destination are siblings.
/// Keep its fields private so callers cannot manufacture partially resolved moves.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ResolvedMove {
    source: PickerId,
    source_parent: ContainerId,
    source_index: usize,
    source_snapshot: Vec<PickerId>,
    destination: ContainerId,
    insertion_index: usize,
    destination_snapshot: Vec<PickerId>,
}

impl ResolvedMove {
    /// Destination sibling sequence after source removal has been accounted for.
    pub(crate) fn destination(&self) -> &ContainerId {
        &self.destination
    }

    /// Final insertion index within `destination` after source removal.
    pub(crate) fn insertion_index(&self) -> usize {
        self.insertion_index
    }
}

/// Errors produced when a hierarchy mutation cannot be applied safely.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum HierarchyError {
    /// The source node does not exist.
    MissingSource(PickerId),
    /// The requested target node does not exist.
    MissingTarget(PickerId),
    /// The requested destination group does not exist.
    MissingContainer(PickerId),
    /// An item was used as a destination container.
    ItemCannotContainChildren(PickerId),
    /// A group cannot be moved into itself or one of its descendants.
    Cycle(PickerId),
    /// The input tree contains duplicate identities or an inconsistent adapter mapping.
    InvalidTree,
    /// The hierarchy exceeds the picker's supported nesting depth.
    DepthExceeded,
}

/// Ordered hierarchy. Every identity occurs exactly once; the root is an implicit container.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct PickerBrowserHierarchy {
    roots: Vec<HierarchyNode>,
}

impl PickerBrowserHierarchy {
    /// Construct and validate a hierarchy from its root-level sequence.
    pub(crate) fn new(roots: Vec<HierarchyNode>) -> Result<Self, HierarchyError> {
        let tree = Self { roots };
        tree.validate()?;
        Ok(tree)
    }

    /// Read the ordered root-level sequence.
    pub(crate) fn roots(&self) -> &[HierarchyNode] {
        &self.roots
    }

    /// Find a node by stable identity.
    pub(crate) fn node(&self, id: &PickerId) -> Option<&HierarchyNode> {
        fn find<'a>(nodes: &'a [HierarchyNode], id: &PickerId) -> Option<&'a HierarchyNode> {
            for node in nodes {
                if node.id() == id {
                    return Some(node);
                }
                if let Some(found) = find(node.children(), id) {
                    return Some(found);
                }
            }
            None
        }
        find(&self.roots, id)
    }

    /// Return the immediate parent container of a node (`Root` for a top-level node).
    pub(crate) fn parent_of(&self, id: &PickerId) -> Option<ContainerId> {
        fn find(nodes: &[HierarchyNode], id: &PickerId, parent: &ContainerId) -> Option<ContainerId> {
            for node in nodes {
                if node.id() == id {
                    return Some(parent.clone());
                }
                if node.is_group()
                    && let Some(found) = find(node.children(), id, &ContainerId::Group(node.id().clone()))
                {
                    return Some(found);
                }
            }
            None
        }
        find(&self.roots, id, &ContainerId::Root)
    }

    /// Read the ordered direct children of a container, including the implicit root sequence.
    pub(crate) fn children_of(&self, container: &ContainerId) -> Option<&[HierarchyNode]> {
        self.children(container)
    }

    /// Apply one exact insertion operation. Validation occurs before the tree is changed.
    pub(crate) fn move_node(&mut self, source: &PickerId, position: &DropPosition) -> Result<(), HierarchyError> {
        let resolved = self.resolve_move(source, position)?;
        self.apply_resolved_move(resolved)
    }

    /// Resolve a semantic drop into an exact source and destination in the current tree.
    pub(crate) fn resolve_move(&self, source: &PickerId, position: &DropPosition) -> Result<ResolvedMove, HierarchyError> {
        self.validate_position(source, position)?;
        let destination = match position {
            DropPosition::Before { target } | DropPosition::After { target } => {
                let parent = self.parent_of(target).ok_or_else(|| HierarchyError::MissingTarget(target.clone()))?;
                (parent, Some((target.clone(), matches!(position, DropPosition::After { .. }))), false)
            }
            DropPosition::PrependTo { group } => (ContainerId::Group(group.clone()), None, true),
            DropPosition::AppendTo { container } => (container.clone(), None, false),
        };

        // Resolve the target's sibling index before removal. Adjust it when source and destination
        // are the same sequence so the result remains immediately adjacent to the target.
        let target_index = if destination.2 {
            Some((0, false))
        } else {
            destination
                .1
                .as_ref()
                .and_then(|(target, after)| self.children(&destination.0)?.iter().position(|node| node.id() == target).map(|index| (index, *after)))
        };
        if let Some((target, _)) = &destination.1
            && target_index.is_none()
        {
            return Err(HierarchyError::MissingTarget(target.clone()));
        }

        let source_parent = self.parent_of(source).ok_or_else(|| HierarchyError::MissingSource(source.clone()))?;
        let source_index = self
            .children(&source_parent)
            .and_then(|nodes| nodes.iter().position(|node| node.id() == source))
            .ok_or_else(|| HierarchyError::MissingSource(source.clone()))?;
        let source_snapshot =
            self.children(&source_parent).ok_or_else(|| HierarchyError::MissingSource(source.clone()))?.iter().map(|node| node.id().clone()).collect();
        let mut insertion_index =
            target_index.map(|(index, after)| index + usize::from(after)).unwrap_or_else(|| self.children(&destination.0).map_or(0, <[_]>::len));
        if source_parent == destination.0 && source_index < insertion_index {
            insertion_index -= 1;
        }
        let destination_snapshot = self
            .children(&destination.0)
            .ok_or_else(|| match &destination.0 {
                ContainerId::Root => HierarchyError::InvalidTree,
                ContainerId::Group(id) => HierarchyError::MissingContainer(id.clone()),
            })?
            .iter()
            .map(|node| node.id().clone())
            .collect();

        Ok(ResolvedMove {
            source: source.clone(),
            source_parent,
            source_index,
            source_snapshot,
            destination: destination.0,
            insertion_index,
            destination_snapshot,
        })
    }

    /// Apply a previously resolved move, rejecting it if the hierarchy changed in the meantime.
    pub(crate) fn apply_resolved_move(&mut self, resolved: ResolvedMove) -> Result<(), HierarchyError> {
        let current_source_index = self.children(&resolved.source_parent).and_then(|nodes| nodes.iter().position(|node| node.id() == &resolved.source));
        if current_source_index != Some(resolved.source_index) {
            return Err(HierarchyError::MissingSource(resolved.source));
        }
        let current_source: Vec<PickerId> = self
            .children(&resolved.source_parent)
            .ok_or_else(|| match &resolved.source_parent {
                ContainerId::Root => HierarchyError::InvalidTree,
                ContainerId::Group(id) => HierarchyError::MissingContainer(id.clone()),
            })?
            .iter()
            .map(|node| node.id().clone())
            .collect();
        if current_source != resolved.source_snapshot {
            return Err(HierarchyError::InvalidTree);
        }
        let current_destination: Vec<PickerId> = self
            .children(&resolved.destination)
            .ok_or_else(|| match &resolved.destination {
                ContainerId::Root => HierarchyError::InvalidTree,
                ContainerId::Group(id) => HierarchyError::MissingContainer(id.clone()),
            })?
            .iter()
            .map(|node| node.id().clone())
            .collect();
        if current_destination != resolved.destination_snapshot {
            return Err(HierarchyError::InvalidTree);
        }
        self.validate_position(&resolved.source, &DropPosition::AppendTo { container: resolved.destination.clone() })?;
        let destination_len = self.children(&resolved.destination).map_or(0, <[_]>::len);
        let post_removal_len = destination_len.saturating_sub(usize::from(resolved.source_parent == resolved.destination));
        if resolved.insertion_index > post_removal_len {
            return Err(HierarchyError::InvalidTree);
        }
        let moving = self.remove_from(&resolved.source_parent, &resolved.source).ok_or_else(|| HierarchyError::MissingSource(resolved.source.clone()))?;
        let children = self.children_mut(&resolved.destination).ok_or_else(|| match &resolved.destination {
            ContainerId::Root => HierarchyError::InvalidTree,
            ContainerId::Group(id) => HierarchyError::MissingContainer(id.clone()),
        })?;
        children.insert(resolved.insertion_index, moving);
        Ok(())
    }

    /// Check a proposed move without mutating the hierarchy.
    pub(crate) fn validate_position(&self, source: &PickerId, position: &DropPosition) -> Result<(), HierarchyError> {
        let source_node = self.node(source).ok_or_else(|| HierarchyError::MissingSource(source.clone()))?;
        let destination = match position {
            DropPosition::Before { target } | DropPosition::After { target } => {
                if target == source {
                    return Err(HierarchyError::Cycle(source.clone()));
                }
                self.parent_of(target).ok_or_else(|| HierarchyError::MissingTarget(target.clone()))?
            }
            DropPosition::PrependTo { group } => ContainerId::Group(group.clone()),
            DropPosition::AppendTo { container } => container.clone(),
        };
        if let ContainerId::Group(group) = &destination {
            let Some(container_node) = self.node(group) else {
                return Err(HierarchyError::MissingContainer(group.clone()));
            };
            if !container_node.is_group() {
                return Err(HierarchyError::ItemCannotContainChildren(group.clone()));
            }
            if source_node.is_group() && (group == source || self.is_descendant_of(group, source)) {
                return Err(HierarchyError::Cycle(source.clone()));
            }
        }
        let source_depth = match (&destination, source_node.is_group()) {
            (ContainerId::Root, _) => 0,
            (ContainerId::Group(group), true) => self.node_depth(group).saturating_add(1),
            (ContainerId::Group(group), false) => self.node_depth(group),
        };
        if source_depth.saturating_add(self.subtree_group_height(source)) > MAX_GROUP_DEPTH {
            return Err(HierarchyError::DepthExceeded);
        }
        Ok(())
    }

    /// Verify that every node identity is unique.
    pub(crate) fn validate(&self) -> Result<(), HierarchyError> {
        let mut pending: Vec<(&HierarchyNode, usize)> = self.roots.iter().rev().map(|node| (node, 0)).collect();
        let mut seen = std::collections::BTreeSet::new();
        while let Some((node, depth)) = pending.pop() {
            if !seen.insert(node.id().clone()) {
                return Err(HierarchyError::InvalidTree);
            }
            if node.is_group() && depth > MAX_GROUP_DEPTH {
                return Err(HierarchyError::DepthExceeded);
            }
            let child_group_depth = depth.saturating_add(usize::from(node.is_group()));
            pending.extend(node.children().iter().rev().map(|child| (child, if child.is_group() { child_group_depth } else { depth })));
        }
        Ok(())
    }

    fn node_depth(&self, id: &PickerId) -> usize {
        let mut depth = 0usize;
        let mut current = id.clone();
        while let Some(ContainerId::Group(parent)) = self.parent_of(&current) {
            depth = depth.saturating_add(1);
            current = parent;
            // A validated tree is acyclic and bounded; this guard also keeps this helper safe if
            // its invariant is ever weakened by a future internal mutation.
            if depth > MAX_GROUP_DEPTH {
                break;
            }
        }
        depth
    }

    fn subtree_group_height(&self, id: &PickerId) -> usize {
        let Some(root) = self.node(id) else { return 0 };
        let mut pending: Vec<(&HierarchyNode, usize)> = root.children().iter().map(|node| (node, usize::from(node.is_group()))).collect();
        let mut height = 0;
        while let Some((node, depth)) = pending.pop() {
            height = height.max(depth);
            pending.extend(node.children().iter().map(|child| (child, depth.saturating_add(usize::from(child.is_group())))));
        }
        height
    }

    fn is_descendant_of(&self, candidate: &PickerId, ancestor: &PickerId) -> bool {
        let mut parent = self.parent_of(candidate);
        while let Some(ContainerId::Group(id)) = parent {
            if &id == ancestor {
                return true;
            }
            parent = self.parent_of(&id);
        }
        false
    }

    fn children(&self, container: &ContainerId) -> Option<&[HierarchyNode]> {
        match container {
            ContainerId::Root => Some(&self.roots),
            ContainerId::Group(id) => self.node(id).filter(|node| node.is_group()).map(HierarchyNode::children),
        }
    }

    fn children_mut(&mut self, container: &ContainerId) -> Option<&mut Vec<HierarchyNode>> {
        match container {
            ContainerId::Root => Some(&mut self.roots),
            ContainerId::Group(id) => find_group_mut(&mut self.roots, id).and_then(|node| match node {
                HierarchyNode::Group { children, .. } => Some(children),
                HierarchyNode::Item(_) => None,
            }),
        }
    }

    fn remove_from(&mut self, container: &ContainerId, id: &PickerId) -> Option<HierarchyNode> {
        let children = self.children_mut(container)?;
        let index = children.iter().position(|node| node.id() == id)?;
        Some(children.remove(index))
    }
}

fn find_group_mut<'a>(nodes: &'a mut [HierarchyNode], id: &PickerId) -> Option<&'a mut HierarchyNode> {
    for node in nodes {
        if node.id() == id {
            return node.is_group().then_some(node);
        }
        if let HierarchyNode::Group { children, .. } = node
            && let Some(found) = find_group_mut(children, id)
        {
            return Some(found);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{ContainerId, DropPosition, HierarchyError, HierarchyNode as Node, PickerBrowserHierarchy, PickerId, PickerIdRegistry, PickerIdentityKey};

    #[test]
    fn picker_registry_reuses_ids_and_allocates_cards_and_groups_from_one_sequence() {
        let mut registry = PickerIdRegistry::with_scope(41);
        let group_key = PickerIdentityKey::from_parts("test", ["library"]);
        let card_key = PickerIdentityKey::from_parts("test", ["card"]);
        let folder_key = PickerIdentityKey::from_parts("test", ["nested"]);
        let group = registry.id_for(group_key.clone());
        let card = registry.id_for(card_key.clone());
        let folder = registry.id_for(folder_key.clone());

        assert_eq!(registry.id_for(group_key), group);
        assert_eq!(registry.id_for(card_key), card);
        assert_eq!(registry.id_for(folder_key), folder);
        assert!(group != card && card != folder && group != folder);
    }

    #[test]
    fn picker_instances_have_independent_identity_scopes() {
        let key = PickerIdentityKey::from_parts("test", ["same opaque adapter token"]);
        let left = PickerIdRegistry::with_scope(101).id_for(key.clone());
        let right = PickerIdRegistry::with_scope(202).id_for(key);

        assert_ne!(left, right);
        assert_eq!(left.value, right.value);
        assert_ne!(left.scope, right.scope);
    }

    #[test]
    fn registry_rekey_preserves_identity_when_a_node_changes_domain_path() {
        let mut registry = PickerIdRegistry::with_scope(9);
        let old = PickerIdentityKey::from_parts("adapter", ["old-folder"]);
        let new = PickerIdentityKey::from_parts("adapter", ["new-folder"]);
        let id = registry.id_for(old.clone());

        registry.rekey(old.clone(), new.clone(), &id);

        assert_eq!(registry.id_for(new.clone()), id);
        assert_eq!(registry.id_for(old.clone()), id, "the old key remains valid until the domain move is observed");
        registry.retain_keys(&[new.clone()].into_iter().collect());
        assert_eq!(registry.id_for(new), id);
        assert_ne!(registry.id_for(old), id, "the stale pre-move key is retired after the new projection is observed");
    }

    #[test]
    fn registry_rekey_does_not_overwrite_an_existing_identity_token() {
        let mut registry = PickerIdRegistry::with_scope(10);
        let old = PickerIdentityKey::from_parts("adapter", ["source"]);
        let occupied = PickerIdentityKey::from_parts("adapter", ["destination"]);
        let moving = registry.id_for(old.clone());
        let existing = registry.id_for(occupied.clone());
        assert_ne!(moving, existing);

        registry.rekey(old.clone(), occupied.clone(), &moving);

        assert_eq!(registry.id_for(old), moving);
        assert_eq!(registry.id_for(occupied), existing, "a rekey collision must never alias two live nodes");
    }

    fn tree() -> PickerBrowserHierarchy {
        PickerBrowserHierarchy::new(vec![
            Node::group("A", vec![Node::item("one"), Node::group("B", vec![Node::item("two")]), Node::item("three")]),
            Node::group("C", vec![]),
        ])
        .unwrap()
    }

    #[test]
    fn adapter_identities_keep_group_and_item_keys_typed() {
        assert_ne!(PickerId::group("A:B"), PickerId::group("A/B"));
        assert_ne!(PickerId::item("First", "same"), PickerId::item("Second", "same"));
        assert_ne!(PickerId::item("A:B", "C"), PickerId::item("A", "B:C"));
        assert_ne!(PickerId::group("same"), PickerId::item("", "same"));
    }

    #[test]
    fn path_qualified_group_and_item_ids_distinguish_equal_names_in_different_branches() {
        let left = PickerId::group_path(["left"], "shared");
        let right = PickerId::group_path(["right"], "shared");
        assert_ne!(left, right);
        assert_ne!(PickerId::group_path(std::iter::empty::<String>(), "shared"), left);
        assert_ne!(PickerId::item_in_path(["left", "shared"], "same"), PickerId::item_in_path(["right", "shared"], "same"));
        assert_ne!(PickerId::group_path(["a:b"], "c"), PickerId::group_path(["a"], "b:c"));
    }

    #[test]
    fn resolved_move_rejects_a_changed_destination_snapshot() {
        let mut hierarchy = PickerBrowserHierarchy::new(vec![Node::item("moving"), Node::item("target"), Node::item("other")]).unwrap();
        let resolved = hierarchy.resolve_move(&PickerId::from("moving"), &DropPosition::Before { target: "target".into() }).unwrap();
        hierarchy.move_node(&PickerId::from("target"), &DropPosition::After { target: "other".into() }).unwrap();
        let before = hierarchy.clone();
        assert_eq!(hierarchy.apply_resolved_move(resolved), Err(HierarchyError::InvalidTree));
        assert_eq!(hierarchy, before);
    }

    #[test]
    fn resolved_move_rejects_source_sibling_changes_even_when_source_index_is_unchanged() {
        let mut hierarchy = PickerBrowserHierarchy::new(vec![
            Node::group("source-parent", vec![Node::item("moving"), Node::item("first"), Node::item("last")]),
            Node::group("destination", vec![Node::item("target")]),
        ])
        .unwrap();
        let resolved = hierarchy.resolve_move(&PickerId::from("moving"), &DropPosition::Before { target: "target".into() }).unwrap();

        hierarchy.move_node(&PickerId::from("first"), &DropPosition::After { target: "last".into() }).unwrap();
        let before = hierarchy.clone();

        assert_eq!(hierarchy.apply_resolved_move(resolved), Err(HierarchyError::InvalidTree));
        assert_eq!(hierarchy, before);
    }

    fn ids(nodes: &[Node]) -> Vec<String> {
        nodes.iter().map(|node| node.id().to_string()).collect()
    }

    fn mixed_tree() -> PickerBrowserHierarchy {
        PickerBrowserHierarchy::new(vec![
            Node::item("root-card-a"),
            Node::group(
                "root-group-a",
                vec![
                    Node::item("nested-card-a"),
                    Node::group("nested-group-a", vec![Node::item("deep-card-a"), Node::item("deep-card-b")]),
                    Node::item("nested-card-b"),
                ],
            ),
            Node::group("root-group-b", vec![Node::item("nested-card-c"), Node::group("nested-group-b", vec![])]),
            Node::item("root-card-b"),
        ])
        .unwrap()
    }

    fn all_ids(nodes: &[Node], output: &mut Vec<PickerId>) {
        for node in nodes {
            output.push(node.id().clone());
            all_ids(node.children(), output);
        }
    }

    fn assert_same_nodes(actual: &PickerBrowserHierarchy, expected: &[PickerId]) {
        let mut actual_ids = Vec::new();
        all_ids(actual.roots(), &mut actual_ids);
        actual_ids.sort();
        let mut expected_ids = expected.to_vec();
        expected_ids.sort();
        assert_eq!(actual_ids, expected_ids);
        assert_eq!(actual.validate(), Ok(()));
    }

    #[test]
    fn before_and_after_are_sibling_insertions_at_the_targets_parent() {
        let mut hierarchy = tree();
        hierarchy.move_node(&PickerId::from("three"), &DropPosition::Before { target: PickerId::from("one") }).unwrap();
        assert_eq!(ids(hierarchy.node(&PickerId::from("A")).unwrap().children()), ["three", "one", "B"]);
        hierarchy.move_node(&PickerId::from("three"), &DropPosition::After { target: PickerId::from("B") }).unwrap();
        assert_eq!(ids(hierarchy.node(&PickerId::from("A")).unwrap().children()), ["one", "B", "three"]);
    }

    #[test]
    fn append_supports_empty_containers_and_root_level_cards() {
        let mut hierarchy = tree();
        hierarchy.move_node(&PickerId::from("one"), &DropPosition::AppendTo { container: ContainerId::Group("C".into()) }).unwrap();
        hierarchy.move_node(&PickerId::from("two"), &DropPosition::AppendTo { container: ContainerId::Root }).unwrap();
        assert_eq!(ids(hierarchy.node(&PickerId::from("C")).unwrap().children()), ["one"]);
        assert_eq!(ids(hierarchy.roots()), ["A", "C", "two"]);
    }

    #[test]
    fn prepend_places_cards_and_groups_at_the_first_mixed_child_position() {
        let mut hierarchy = tree();
        hierarchy.move_node(&PickerId::from("three"), &DropPosition::PrependTo { group: PickerId::from("A") }).unwrap();
        hierarchy.move_node(&PickerId::from("C"), &DropPosition::PrependTo { group: PickerId::from("A") }).unwrap();
        assert_eq!(ids(hierarchy.node(&PickerId::from("A")).unwrap().children()), ["C", "three", "one", "B"]);
    }

    #[test]
    fn rejects_cycles_and_item_containers_without_mutating_the_tree() {
        let mut hierarchy = tree();
        let before = hierarchy.clone();
        assert_eq!(
            hierarchy.move_node(&PickerId::from("A"), &DropPosition::AppendTo { container: ContainerId::Group("B".into()) }),
            Err(HierarchyError::Cycle("A".into()))
        );
        assert_eq!(
            hierarchy.move_node(&PickerId::from("one"), &DropPosition::AppendTo { container: ContainerId::Group("two".into()) }),
            Err(HierarchyError::ItemCannotContainChildren("two".into()))
        );
        assert_eq!(hierarchy, before);
    }

    #[test]
    fn same_container_move_keeps_the_requested_adjacency() {
        let mut hierarchy = tree();
        hierarchy.move_node(&PickerId::from("one"), &DropPosition::After { target: PickerId::from("three") }).unwrap();
        assert_eq!(ids(hierarchy.node(&PickerId::from("A")).unwrap().children()), ["B", "three", "one"]);
    }

    #[test]
    fn root_group_moves_before_and_after_first_interior_and_last_cards() {
        let cases = [
            (DropPosition::Before { target: "first".into() }, ["moving", "first", "interior", "last"]),
            (DropPosition::After { target: "first".into() }, ["first", "moving", "interior", "last"]),
            (DropPosition::Before { target: "interior".into() }, ["first", "moving", "interior", "last"]),
            (DropPosition::After { target: "interior".into() }, ["first", "interior", "moving", "last"]),
            (DropPosition::Before { target: "last".into() }, ["first", "interior", "moving", "last"]),
            (DropPosition::After { target: "last".into() }, ["first", "interior", "last", "moving"]),
        ];

        for (position, expected) in cases {
            let mut hierarchy = PickerBrowserHierarchy::new(vec![
                Node::group("parent", vec![Node::item("first"), Node::item("interior"), Node::item("last")]),
                Node::group("moving", vec![Node::item("nested-card")]),
            ])
            .unwrap();

            hierarchy.move_node(&PickerId::from("moving"), &position).unwrap();

            assert_eq!(ids(hierarchy.node(&PickerId::from("parent")).unwrap().children()), expected);
            assert_eq!(ids(hierarchy.roots()), ["parent"]);
        }
    }

    #[test]
    fn nested_group_moves_back_to_root_before_and_after_root_cards() {
        let cases = [
            (DropPosition::Before { target: "root-first".into() }, ["parent", "moving", "root-first", "root-last"]),
            (DropPosition::After { target: "root-first".into() }, ["parent", "root-first", "moving", "root-last"]),
        ];

        for (position, expected) in cases {
            let mut hierarchy = PickerBrowserHierarchy::new(vec![
                Node::group("parent", vec![Node::item("first"), Node::group("moving", vec![Node::item("nested-card")]), Node::item("last")]),
                Node::item("root-first"),
                Node::item("root-last"),
            ])
            .unwrap();

            hierarchy.move_node(&PickerId::from("moving"), &position).unwrap();

            assert_eq!(ids(hierarchy.roots()), expected);
            assert_eq!(ids(hierarchy.node(&PickerId::from("parent")).unwrap().children()), ["first", "last"]);
        }
    }

    #[test]
    fn same_parent_group_moves_adjust_the_insertion_index_after_removal() {
        let mut hierarchy = PickerBrowserHierarchy::new(vec![Node::group(
            "parent",
            vec![Node::item("first"), Node::group("moving", vec![]), Node::item("interior"), Node::item("last")],
        )])
        .unwrap();

        hierarchy.move_node(&PickerId::from("moving"), &DropPosition::After { target: PickerId::from("last") }).unwrap();
        assert_eq!(ids(hierarchy.node(&PickerId::from("parent")).unwrap().children()), ["first", "interior", "last", "moving"]);

        hierarchy.move_node(&PickerId::from("moving"), &DropPosition::Before { target: PickerId::from("first") }).unwrap();
        assert_eq!(ids(hierarchy.node(&PickerId::from("parent")).unwrap().children()), ["moving", "first", "interior", "last"]);
    }

    #[test]
    fn constructor_rejects_duplicate_ids_and_excessive_depth() {
        assert_eq!(PickerBrowserHierarchy::new(vec![Node::item("duplicate"), Node::group("duplicate", vec![])]), Err(HierarchyError::InvalidTree));
        let item = Node::item("item-without-children");
        assert!(!item.is_group());
        assert!(item.children().is_empty());
        let mut nested = Node::item("leaf");
        for depth in 0..=super::MAX_GROUP_DEPTH + 1 {
            nested = Node::group(format!("group-{depth}"), vec![nested]);
        }
        assert_eq!(PickerBrowserHierarchy::new(vec![nested]), Err(HierarchyError::DepthExceeded));
    }

    #[test]
    fn deepest_group_may_contain_items_and_accept_an_item_move() {
        let mut nested = Node::item("leaf");
        for depth in 0..=super::MAX_GROUP_DEPTH {
            nested = Node::group(format!("group-{depth}"), vec![nested]);
        }
        let mut hierarchy = PickerBrowserHierarchy::new(vec![nested, Node::item("extra")]).unwrap();
        hierarchy.move_node(&PickerId::from("extra"), &DropPosition::AppendTo { container: ContainerId::Group("group-0".into()) }).unwrap();
        assert_eq!(hierarchy.parent_of(&PickerId::from("extra")), Some(ContainerId::Group("group-0".into())));
    }

    #[test]
    fn move_rejects_a_subtree_that_would_exceed_the_depth_limit() {
        let mut nested = Node::item("leaf");
        for depth in 0..=super::MAX_GROUP_DEPTH {
            nested = Node::group(format!("group-{depth}"), vec![nested]);
        }
        let mut hierarchy = PickerBrowserHierarchy::new(vec![Node::group("destination", vec![]), nested]).unwrap();
        let before = hierarchy.clone();
        assert_eq!(
            hierarchy.move_node(&PickerId::from("group-32"), &DropPosition::AppendTo { container: ContainerId::Group("destination".into()) }),
            Err(HierarchyError::DepthExceeded)
        );
        assert_eq!(hierarchy, before);
    }

    #[test]
    fn moving_a_max_depth_group_to_root_starts_it_at_depth_zero() {
        let mut nested = Node::item("leaf");
        for depth in 0..=super::MAX_GROUP_DEPTH {
            nested = Node::group(format!("group-{depth}"), vec![nested]);
        }
        let mut hierarchy = PickerBrowserHierarchy::new(vec![nested, Node::group("destination", vec![])]).unwrap();
        hierarchy.move_node(&PickerId::from("group-32"), &DropPosition::AppendTo { container: ContainerId::Root }).unwrap();
        assert_eq!(hierarchy.parent_of(&PickerId::from("group-32")), Some(ContainerId::Root));
    }

    #[test]
    fn every_node_pair_supports_consistent_before_and_after_reordering_or_rejects_atomically() {
        let original = mixed_tree();
        let mut node_ids = Vec::new();
        all_ids(original.roots(), &mut node_ids);

        for source in &node_ids {
            for target in &node_ids {
                for position in [DropPosition::Before { target: target.clone() }, DropPosition::After { target: target.clone() }] {
                    let mut hierarchy = original.clone();
                    let result = hierarchy.move_node(source, &position);
                    match result {
                        Ok(()) => {
                            let target_parent = hierarchy.parent_of(target).expect("target remains in the tree");
                            assert_eq!(hierarchy.parent_of(source), Some(target_parent.clone()), "source={source:?}, target={target:?}, position={position:?}");
                            let siblings = hierarchy.children_of(&target_parent).expect("target parent exists");
                            let source_index = siblings.iter().position(|node| node.id() == source).expect("source is a sibling of target");
                            let target_index = siblings.iter().position(|node| node.id() == target).expect("target remains a sibling");
                            if matches!(&position, DropPosition::Before { .. }) {
                                assert_eq!(source_index + 1, target_index, "source={source:?}, target={target:?}");
                            } else {
                                assert!(matches!(&position, DropPosition::After { .. }), "expected a before/after position: {position:?}");
                                assert_eq!(target_index + 1, source_index, "source={source:?}, target={target:?}");
                            }
                            assert_same_nodes(&hierarchy, &node_ids);
                        }
                        Err(_) => assert_eq!(hierarchy, original, "rejected source={source:?}, target={target:?}, position={position:?}"),
                    }
                }
            }
        }
    }

    #[test]
    fn every_node_can_be_prepend_or_append_moved_to_every_container_without_losing_structure() {
        let original = mixed_tree();
        let mut node_ids = Vec::new();
        all_ids(original.roots(), &mut node_ids);
        let containers = std::iter::once(ContainerId::Root)
            .chain(node_ids.iter().filter(|id| original.node(id).is_some_and(Node::is_group)).cloned().map(ContainerId::Group))
            .collect::<Vec<_>>();

        for source in &node_ids {
            for container in &containers {
                let positions = match container {
                    ContainerId::Root => vec![DropPosition::AppendTo { container: ContainerId::Root }],
                    ContainerId::Group(group) => {
                        vec![DropPosition::PrependTo { group: group.clone() }, DropPosition::AppendTo { container: container.clone() }]
                    }
                };
                for position in positions {
                    let mut hierarchy = original.clone();
                    match hierarchy.move_node(source, &position) {
                        Ok(()) => {
                            assert_eq!(hierarchy.parent_of(source), Some(container.clone()), "source={source:?}, position={position:?}");
                            let siblings = hierarchy.children_of(container).expect("valid destination container");
                            if matches!(&position, DropPosition::PrependTo { .. }) {
                                assert_eq!(siblings.first().map(Node::id), Some(source));
                            } else {
                                assert!(matches!(&position, DropPosition::AppendTo { .. }), "expected a prepend/append position: {position:?}");
                                assert_eq!(siblings.last().map(Node::id), Some(source));
                            }
                            assert_same_nodes(&hierarchy, &node_ids);
                        }
                        Err(_) => assert_eq!(hierarchy, original, "rejected source={source:?}, position={position:?}"),
                    }
                }
            }
        }
    }

    #[test]
    fn mixed_move_sequences_preserve_identity_parentage_and_order_across_operation_types() {
        let mut hierarchy = mixed_tree();
        let expected_ids = {
            let mut ids = Vec::new();
            all_ids(hierarchy.roots(), &mut ids);
            ids
        };

        let moves = [
            ("root-card-a", DropPosition::PrependTo { group: "root-group-a".into() }),
            ("nested-group-a", DropPosition::After { target: "nested-card-b".into() }),
            ("root-group-b", DropPosition::PrependTo { group: "nested-group-a".into() }),
            ("deep-card-a", DropPosition::AppendTo { container: ContainerId::Root }),
            ("nested-card-c", DropPosition::Before { target: "root-card-b".into() }),
            ("root-group-a", DropPosition::AppendTo { container: ContainerId::Root }),
            ("root-card-b", DropPosition::PrependTo { group: "nested-group-b".into() }),
        ];

        for (source, position) in moves {
            hierarchy.move_node(&PickerId::from(source), &position).unwrap_or_else(|error| panic!("{source} -> {position:?}: {error:?}"));
            assert_same_nodes(&hierarchy, &expected_ids);
        }

        assert_eq!(hierarchy.parent_of(&PickerId::from("root-card-a")), Some(ContainerId::Group("root-group-a".into())));
        assert_eq!(hierarchy.parent_of(&PickerId::from("root-group-b")), Some(ContainerId::Group("nested-group-a".into())));
        assert_eq!(hierarchy.parent_of(&PickerId::from("deep-card-a")), Some(ContainerId::Root));
        assert_eq!(hierarchy.parent_of(&PickerId::from("root-group-a")), Some(ContainerId::Root));
        assert_eq!(hierarchy.parent_of(&PickerId::from("root-card-b")), Some(ContainerId::Group("nested-group-b".into())));
    }

    #[test]
    fn missing_sources_targets_and_containers_are_rejected_without_partial_mutation() {
        let original = mixed_tree();
        let invalid_positions = [
            DropPosition::Before { target: "missing".into() },
            DropPosition::After { target: "missing".into() },
            DropPosition::PrependTo { group: "missing".into() },
            DropPosition::AppendTo { container: ContainerId::Group("missing".into()) },
            DropPosition::PrependTo { group: "root-card-a".into() },
            DropPosition::AppendTo { container: ContainerId::Group("root-card-a".into()) },
        ];

        for source in [PickerId::from("missing"), PickerId::from("root-card-a"), PickerId::from("root-group-a")] {
            for position in &invalid_positions {
                let mut hierarchy = original.clone();
                assert!(hierarchy.move_node(&source, position).is_err(), "source={source:?}, position={position:?}");
                assert_eq!(hierarchy, original, "rejected moves must be atomic: source={source:?}, position={position:?}");
            }
        }
    }
}
