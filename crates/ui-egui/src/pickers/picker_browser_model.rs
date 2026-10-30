//! Canonical, asset-neutral hierarchy model for picker browsers.
//!
//! This is the browser's runtime model, not a replacement for an asset library's persistence
//! schema. Adapters project domain assets into this tree and translate validated moves back into
//! domain commands. Root and group containers both hold an ordered mixture of cards and groups.

use std::fmt;

/// Maximum number of nested groups, with the root group at depth zero.
pub(crate) const MAX_GROUP_DEPTH: usize = 32;

/// Stable identity of one card or group in a browser tree.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum NodeId {
    /// An adapter-neutral identity used by model tests and generic consumers.
    Adapter(String),
    /// A group identified by its domain key.
    Group(String),
    /// A group identified by its original hierarchy path. The path remains stable while the node
    /// is moved within a projected tree, and distinguishes equal names under different parents.
    GroupPath(Vec<String>),
    /// An item identified by both its owning asset group and stable item key.
    Item { group: String, key: String },
    /// An item identified by its owning hierarchy path and stable item key.
    ItemPath { group_path: Vec<String>, key: String },
}

impl NodeId {
    /// Create an identity from an adapter-owned stable key.
    pub(crate) fn new(value: impl Into<String>) -> Self {
        Self::Adapter(value.into())
    }

    /// Construct a typed group identity.
    pub(crate) fn group(name: impl Into<String>) -> Self {
        Self::Group(name.into())
    }

    /// Construct a group identity from its original root-to-parent path and own name.
    pub(crate) fn group_path(path: impl IntoIterator<Item = impl Into<String>>, name: impl Into<String>) -> Self {
        let mut path: Vec<String> = path.into_iter().map(Into::into).collect();
        path.push(name.into());
        Self::GroupPath(path)
    }

    /// Construct a typed item identity. Group and item keys may contain any delimiter.
    pub(crate) fn item(group: impl Into<String>, key: impl Into<String>) -> Self {
        Self::Item { group: group.into(), key: key.into() }
    }

    /// Construct an item identity using its owning group's original root-to-group path.
    pub(crate) fn item_in_path(path: impl IntoIterator<Item = impl Into<String>>, key: impl Into<String>) -> Self {
        Self::ItemPath { group_path: path.into_iter().map(Into::into).collect(), key: key.into() }
    }
}

impl From<&str> for NodeId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for NodeId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Adapter(value) => fmt::Display::fmt(value, f),
            Self::Group(name) => write!(f, "group:{name}"),
            Self::GroupPath(path) => {
                write!(f, "group-path")?;
                for segment in path {
                    write!(f, ":{}:{segment}", segment.len())?;
                }
                Ok(())
            }
            Self::Item { group, key } => write!(f, "item:{}:{group}:{}:{key}", group.len(), key.len()),
            Self::ItemPath { group_path, key } => {
                write!(f, "item-path")?;
                for segment in group_path {
                    write!(f, ":{}:{segment}", segment.len())?;
                }
                write!(f, ":{}:{key}", key.len())
            }
        }
    }
}

/// One node in the ordered hierarchy. Items cannot own children by construction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum HierarchyNode {
    /// A draggable asset card.
    Item(NodeId),
    /// A container that may hold cards and other groups.
    Group { id: NodeId, children: Vec<HierarchyNode> },
}

impl HierarchyNode {
    /// Construct an item node.
    pub(crate) fn item(id: impl Into<NodeId>) -> Self {
        Self::Item(id.into())
    }

    /// Construct a group node with ordered children.
    pub(crate) fn group(id: impl Into<NodeId>, children: Vec<Self>) -> Self {
        Self::Group { id: id.into(), children }
    }

    /// Stable identity supplied by the asset adapter.
    pub(crate) fn id(&self) -> &NodeId {
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
    Group(NodeId),
}

/// Exact structural destination for a drag operation.
///
/// Before and after are always sibling insertions: the target node determines the destination
/// container. Append is the only operation that names a container directly, and covers both
/// appending to a non-empty group and inserting into an empty group or at the end of root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DropPosition {
    /// Insert immediately before the target in its current parent's ordered children.
    Before { target: NodeId },
    /// Insert immediately after the target in its current parent's ordered children.
    After { target: NodeId },
    /// Insert as the final child of a container.
    AppendTo { container: ContainerId },
}

/// A drop resolved against one specific hierarchy snapshot.
///
/// The insertion index is already adjusted for removal when source and destination are siblings.
/// Keep its fields private so callers cannot manufacture partially resolved moves.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ResolvedMove {
    source: NodeId,
    source_parent: ContainerId,
    source_index: usize,
    source_snapshot: Vec<NodeId>,
    destination: ContainerId,
    insertion_index: usize,
    destination_snapshot: Vec<NodeId>,
}

/// Errors produced when a hierarchy mutation cannot be applied safely.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum HierarchyError {
    /// The source node does not exist.
    MissingSource(NodeId),
    /// The requested target node does not exist.
    MissingTarget(NodeId),
    /// The requested destination group does not exist.
    MissingContainer(NodeId),
    /// An item was used as a destination container.
    ItemCannotContainChildren(NodeId),
    /// A group cannot be moved into itself or one of its descendants.
    Cycle(NodeId),
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
    pub(crate) fn node(&self, id: &NodeId) -> Option<&HierarchyNode> {
        fn find<'a>(nodes: &'a [HierarchyNode], id: &NodeId) -> Option<&'a HierarchyNode> {
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
    pub(crate) fn parent_of(&self, id: &NodeId) -> Option<ContainerId> {
        fn find(nodes: &[HierarchyNode], id: &NodeId, parent: &ContainerId) -> Option<ContainerId> {
            for node in nodes {
                if node.id() == id {
                    return Some(parent.clone());
                }
                if node.is_group() {
                    if let Some(found) = find(node.children(), id, &ContainerId::Group(node.id().clone())) {
                        return Some(found);
                    }
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
    pub(crate) fn move_node(&mut self, source: &NodeId, position: &DropPosition) -> Result<(), HierarchyError> {
        let resolved = self.resolve_move(source, position)?;
        self.apply_resolved_move(resolved)
    }

    /// Resolve a semantic drop into an exact source and destination in the current tree.
    pub(crate) fn resolve_move(&self, source: &NodeId, position: &DropPosition) -> Result<ResolvedMove, HierarchyError> {
        self.validate_position(source, position)?;
        let destination = match position {
            DropPosition::Before { target } | DropPosition::After { target } => {
                let parent = self.parent_of(target).ok_or_else(|| HierarchyError::MissingTarget(target.clone()))?;
                (parent, Some((target.clone(), matches!(position, DropPosition::After { .. }))))
            }
            DropPosition::AppendTo { container } => (container.clone(), None),
        };

        // Resolve the target's sibling index before removal. Adjust it when source and destination
        // are the same sequence so the result remains immediately adjacent to the target.
        let target_index = destination
            .1
            .as_ref()
            .and_then(|(target, after)| self.children(&destination.0)?.iter().position(|node| node.id() == target).map(|index| (index, *after)));
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
        let source_snapshot = self
            .children(&source_parent)
            .ok_or_else(|| HierarchyError::MissingSource(source.clone()))?
            .iter()
            .map(|node| node.id().clone())
            .collect();
        let mut insertion_index =
            target_index.map(|(index, after)| index + usize::from(after)).unwrap_or_else(|| self.children(&destination.0).map_or(0, <[_]>::len));
        if source_parent == destination.0 && source_index < insertion_index {
            insertion_index -= 1;
        }
        let destination_snapshot = self.children(&destination.0)
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
        let current_source_index = self.children(&resolved.source_parent)
            .and_then(|nodes| nodes.iter().position(|node| node.id() == &resolved.source));
        if current_source_index != Some(resolved.source_index) {
            return Err(HierarchyError::MissingSource(resolved.source));
        }
        let current_source: Vec<NodeId> = self.children(&resolved.source_parent)
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
        let current_destination: Vec<NodeId> = self.children(&resolved.destination)
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
    pub(crate) fn validate_position(&self, source: &NodeId, position: &DropPosition) -> Result<(), HierarchyError> {
        let source_node = self.node(source).ok_or_else(|| HierarchyError::MissingSource(source.clone()))?;
        let destination = match position {
            DropPosition::Before { target } | DropPosition::After { target } => {
                if target == source {
                    return Err(HierarchyError::Cycle(source.clone()));
                }
                self.parent_of(target).ok_or_else(|| HierarchyError::MissingTarget(target.clone()))?
            }
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

    fn node_depth(&self, id: &NodeId) -> usize {
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

    fn subtree_group_height(&self, id: &NodeId) -> usize {
        let Some(root) = self.node(id) else { return 0 };
        let mut pending: Vec<(&HierarchyNode, usize)> = root.children().iter().map(|node| (node, usize::from(node.is_group()))).collect();
        let mut height = 0;
        while let Some((node, depth)) = pending.pop() {
            height = height.max(depth);
            pending.extend(node.children().iter().map(|child| (child, depth.saturating_add(usize::from(child.is_group())))));
        }
        height
    }

    fn is_descendant_of(&self, candidate: &NodeId, ancestor: &NodeId) -> bool {
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

    fn remove_from(&mut self, container: &ContainerId, id: &NodeId) -> Option<HierarchyNode> {
        let children = self.children_mut(container)?;
        let index = children.iter().position(|node| node.id() == id)?;
        Some(children.remove(index))
    }
}

fn find_group_mut<'a>(nodes: &'a mut [HierarchyNode], id: &NodeId) -> Option<&'a mut HierarchyNode> {
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
    use super::{ContainerId, DropPosition, HierarchyError, HierarchyNode as Node, NodeId, PickerBrowserHierarchy};

    fn tree() -> PickerBrowserHierarchy {
        PickerBrowserHierarchy::new(vec![
            Node::group("A", vec![Node::item("one"), Node::group("B", vec![Node::item("two")]), Node::item("three")]),
            Node::group("C", vec![]),
        ])
        .unwrap()
    }

    #[test]
    fn adapter_identities_keep_group_and_item_keys_typed() {
        assert_ne!(NodeId::group("A:B"), NodeId::group("A/B"));
        assert_ne!(NodeId::item("First", "same"), NodeId::item("Second", "same"));
        assert_ne!(NodeId::item("A:B", "C"), NodeId::item("A", "B:C"));
        assert_ne!(NodeId::group("same"), NodeId::item("", "same"));
    }

    #[test]
    fn path_qualified_group_and_item_ids_distinguish_equal_names_in_different_branches() {
        let left = NodeId::group_path(["left"], "shared");
        let right = NodeId::group_path(["right"], "shared");
        assert_ne!(left, right);
        assert_ne!(NodeId::group_path([], "shared"), left);
        assert_ne!(NodeId::item_in_path(["left", "shared"], "same"), NodeId::item_in_path(["right", "shared"], "same"));
        assert_ne!(NodeId::group_path(["a:b"], "c"), NodeId::group_path(["a"], "b:c"));
        assert_ne!(NodeId::group_path(["left"], "shared").to_string(), NodeId::group_path(["right"], "shared").to_string());
    }

    #[test]
    fn resolved_move_rejects_a_changed_destination_snapshot() {
        let mut hierarchy = PickerBrowserHierarchy::new(vec![Node::item("moving"), Node::item("target"), Node::item("other")]).unwrap();
        let resolved = hierarchy.resolve_move(&NodeId::from("moving"), &DropPosition::Before { target: "target".into() }).unwrap();
        hierarchy.move_node(&NodeId::from("target"), &DropPosition::After { target: "other".into() }).unwrap();
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
        let resolved = hierarchy
            .resolve_move(&NodeId::from("moving"), &DropPosition::Before { target: "target".into() })
            .unwrap();

        hierarchy.move_node(&NodeId::from("first"), &DropPosition::After { target: "last".into() }).unwrap();
        let before = hierarchy.clone();

        assert_eq!(hierarchy.apply_resolved_move(resolved), Err(HierarchyError::InvalidTree));
        assert_eq!(hierarchy, before);
    }

    fn ids(nodes: &[Node]) -> Vec<String> {
        nodes.iter().map(|node| node.id().to_string()).collect()
    }

    #[test]
    fn before_and_after_are_sibling_insertions_at_the_targets_parent() {
        let mut hierarchy = tree();
        hierarchy.move_node(&NodeId::from("three"), &DropPosition::Before { target: NodeId::from("one") }).unwrap();
        assert_eq!(ids(hierarchy.node(&NodeId::from("A")).unwrap().children()), ["three", "one", "B"]);
        hierarchy.move_node(&NodeId::from("three"), &DropPosition::After { target: NodeId::from("B") }).unwrap();
        assert_eq!(ids(hierarchy.node(&NodeId::from("A")).unwrap().children()), ["one", "B", "three"]);
    }

    #[test]
    fn append_supports_empty_containers_and_root_level_cards() {
        let mut hierarchy = tree();
        hierarchy.move_node(&NodeId::from("one"), &DropPosition::AppendTo { container: ContainerId::Group("C".into()) }).unwrap();
        hierarchy.move_node(&NodeId::from("two"), &DropPosition::AppendTo { container: ContainerId::Root }).unwrap();
        assert_eq!(ids(hierarchy.node(&NodeId::from("C")).unwrap().children()), ["one"]);
        assert_eq!(ids(hierarchy.roots()), ["A", "C", "two"]);
    }

    #[test]
    fn rejects_cycles_and_item_containers_without_mutating_the_tree() {
        let mut hierarchy = tree();
        let before = hierarchy.clone();
        assert_eq!(
            hierarchy.move_node(&NodeId::from("A"), &DropPosition::AppendTo { container: ContainerId::Group("B".into()) }),
            Err(HierarchyError::Cycle("A".into()))
        );
        assert_eq!(
            hierarchy.move_node(&NodeId::from("one"), &DropPosition::AppendTo { container: ContainerId::Group("two".into()) }),
            Err(HierarchyError::ItemCannotContainChildren("two".into()))
        );
        assert_eq!(hierarchy, before);
    }

    #[test]
    fn same_container_move_keeps_the_requested_adjacency() {
        let mut hierarchy = tree();
        hierarchy.move_node(&NodeId::from("one"), &DropPosition::After { target: NodeId::from("three") }).unwrap();
        assert_eq!(ids(hierarchy.node(&NodeId::from("A")).unwrap().children()), ["B", "three", "one"]);
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

            hierarchy.move_node(&NodeId::from("moving"), &position).unwrap();

            assert_eq!(ids(hierarchy.node(&NodeId::from("parent")).unwrap().children()), expected);
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
                Node::group(
                    "parent",
                    vec![Node::item("first"), Node::group("moving", vec![Node::item("nested-card")]), Node::item("last")],
                ),
                Node::item("root-first"),
                Node::item("root-last"),
            ])
            .unwrap();

            hierarchy.move_node(&NodeId::from("moving"), &position).unwrap();

            assert_eq!(ids(hierarchy.roots()), expected);
            assert_eq!(ids(hierarchy.node(&NodeId::from("parent")).unwrap().children()), ["first", "last"]);
        }
    }

    #[test]
    fn same_parent_group_moves_adjust_the_insertion_index_after_removal() {
        let mut hierarchy = PickerBrowserHierarchy::new(vec![Node::group(
            "parent",
            vec![Node::item("first"), Node::group("moving", vec![]), Node::item("interior"), Node::item("last")],
        )])
        .unwrap();

        hierarchy.move_node(&NodeId::from("moving"), &DropPosition::After { target: NodeId::from("last") }).unwrap();
        assert_eq!(ids(hierarchy.node(&NodeId::from("parent")).unwrap().children()), ["first", "interior", "last", "moving"]);

        hierarchy.move_node(&NodeId::from("moving"), &DropPosition::Before { target: NodeId::from("first") }).unwrap();
        assert_eq!(ids(hierarchy.node(&NodeId::from("parent")).unwrap().children()), ["moving", "first", "interior", "last"]);
    }

    #[test]
    fn constructor_rejects_duplicate_ids_and_excessive_depth() {
        assert_eq!(
            PickerBrowserHierarchy::new(vec![Node::item("duplicate"), Node::group("duplicate", vec![])]),
            Err(HierarchyError::InvalidTree)
        );
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
        hierarchy
            .move_node(&NodeId::from("extra"), &DropPosition::AppendTo { container: ContainerId::Group("group-0".into()) })
            .unwrap();
        assert_eq!(hierarchy.parent_of(&NodeId::from("extra")), Some(ContainerId::Group("group-0".into())));
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
            hierarchy.move_node(&NodeId::from("group-32"), &DropPosition::AppendTo { container: ContainerId::Group("destination".into()) }),
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
        hierarchy
            .move_node(&NodeId::from("group-32"), &DropPosition::AppendTo { container: ContainerId::Root })
            .unwrap();
        assert_eq!(hierarchy.parent_of(&NodeId::from("group-32")), Some(ContainerId::Root));
    }
}
