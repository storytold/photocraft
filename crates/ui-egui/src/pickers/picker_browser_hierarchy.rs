//! Canonical, asset-neutral hierarchy model for picker browsers.
//!
//! This is the browser's runtime model, not a replacement for an asset library's persistence
//! schema. Adapters project domain assets into this tree and translate validated moves back into
//! domain commands. Root and group containers both hold an ordered mixture of cards and groups.

use std::fmt;

/// Stable identity of one card or group in a browser tree.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct NodeId(String);

impl NodeId {
    /// Create an identity from an adapter-owned stable key.
    pub(crate) fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Return the underlying adapter key.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
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
        self.0.fmt(f)
    }
}

/// The kind of hierarchy node. Only groups may own children.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NodeKind {
    /// A draggable asset card.
    Item,
    /// A container that may hold cards and other groups.
    Group,
}

/// One node in the ordered hierarchy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HierarchyNode {
    /// Stable identity supplied by the asset adapter.
    pub(crate) id: NodeId,
    /// Whether this node is a card or a group.
    pub(crate) kind: NodeKind,
    /// Ordered children; this must be empty for item nodes.
    pub(crate) children: Vec<HierarchyNode>,
}

impl HierarchyNode {
    /// Construct an item node.
    pub(crate) fn item(id: impl Into<NodeId>) -> Self {
        Self { id: id.into(), kind: NodeKind::Item, children: Vec::new() }
    }

    /// Construct a group node with ordered children.
    pub(crate) fn group(id: impl Into<NodeId>, children: Vec<Self>) -> Self {
        Self { id: id.into(), kind: NodeKind::Group, children }
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
    /// The input tree contains duplicate identities or an item with children.
    InvalidTree,
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
                if &node.id == id {
                    return Some(node);
                }
                if let Some(found) = find(&node.children, id) {
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
                if &node.id == id {
                    return Some(parent.clone());
                }
                if node.kind == NodeKind::Group {
                    if let Some(found) = find(&node.children, id, &ContainerId::Group(node.id.clone())) {
                        return Some(found);
                    }
                }
            }
            None
        }
        find(&self.roots, id, &ContainerId::Root)
    }

    /// Apply one exact insertion operation. Validation occurs before the tree is changed.
    pub(crate) fn move_node(&mut self, source: &NodeId, position: &DropPosition) -> Result<(), HierarchyError> {
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
            .and_then(|(target, after)| self.children(&destination.0)?.iter().position(|node| &node.id == target).map(|index| (index, *after)));
        if destination.1.is_some() && target_index.is_none() {
            return Err(HierarchyError::MissingTarget(destination.1.as_ref().expect("checked").0.clone()));
        }

        let source_parent = self.parent_of(source).ok_or_else(|| HierarchyError::MissingSource(source.clone()))?;
        let source_index = self
            .children(&source_parent)
            .and_then(|nodes| nodes.iter().position(|node| &node.id == source))
            .ok_or_else(|| HierarchyError::MissingSource(source.clone()))?;
        let mut insertion_index =
            target_index.map(|(index, after)| index + usize::from(after)).unwrap_or_else(|| self.children(&destination.0).map_or(0, <[_]>::len));
        if source_parent == destination.0 && source_index < insertion_index {
            insertion_index -= 1;
        }

        let moving = self.remove_from(&source_parent, source).ok_or_else(|| HierarchyError::MissingSource(source.clone()))?;
        let children = self.children_mut(&destination.0).ok_or_else(|| match &destination.0 {
            ContainerId::Root => HierarchyError::InvalidTree,
            ContainerId::Group(id) => HierarchyError::MissingContainer(id.clone()),
        })?;
        children.insert(insertion_index.min(children.len()), moving);
        debug_assert!(self.validate().is_ok());
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
            if container_node.kind != NodeKind::Group {
                return Err(HierarchyError::ItemCannotContainChildren(group.clone()));
            }
            if source_node.kind == NodeKind::Group && (group == source || self.is_descendant_of(group, source)) {
                return Err(HierarchyError::Cycle(source.clone()));
            }
        }
        Ok(())
    }

    /// Verify unique identities and the rule that only groups own children.
    pub(crate) fn validate(&self) -> Result<(), HierarchyError> {
        fn visit(nodes: &[HierarchyNode], seen: &mut std::collections::BTreeSet<NodeId>) -> Result<(), HierarchyError> {
            for node in nodes {
                if !seen.insert(node.id.clone()) || (node.kind == NodeKind::Item && !node.children.is_empty()) {
                    return Err(HierarchyError::InvalidTree);
                }
                visit(&node.children, seen)?;
            }
            Ok(())
        }
        visit(&self.roots, &mut std::collections::BTreeSet::new())
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
            ContainerId::Group(id) => self.node(id).filter(|node| node.kind == NodeKind::Group).map(|node| node.children.as_slice()),
        }
    }

    fn children_mut(&mut self, container: &ContainerId) -> Option<&mut Vec<HierarchyNode>> {
        match container {
            ContainerId::Root => Some(&mut self.roots),
            ContainerId::Group(id) => find_group_mut(&mut self.roots, id).map(|node| &mut node.children),
        }
    }

    fn remove_from(&mut self, container: &ContainerId, id: &NodeId) -> Option<HierarchyNode> {
        let children = self.children_mut(container)?;
        let index = children.iter().position(|node| &node.id == id)?;
        Some(children.remove(index))
    }
}

fn find_group_mut<'a>(nodes: &'a mut [HierarchyNode], id: &NodeId) -> Option<&'a mut HierarchyNode> {
    for node in nodes {
        if &node.id == id {
            return (node.kind == NodeKind::Group).then_some(node);
        }
        if let Some(found) = find_group_mut(&mut node.children, id) {
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

    fn ids(nodes: &[Node]) -> Vec<&str> {
        nodes.iter().map(|node| node.id.as_str()).collect()
    }

    #[test]
    fn before_and_after_are_sibling_insertions_at_the_targets_parent() {
        let mut hierarchy = tree();
        hierarchy.move_node(&NodeId::from("three"), &DropPosition::Before { target: NodeId::from("one") }).unwrap();
        assert_eq!(ids(&hierarchy.node(&NodeId::from("A")).unwrap().children), ["three", "one", "B"]);
        hierarchy.move_node(&NodeId::from("three"), &DropPosition::After { target: NodeId::from("B") }).unwrap();
        assert_eq!(ids(&hierarchy.node(&NodeId::from("A")).unwrap().children), ["one", "B", "three"]);
    }

    #[test]
    fn append_supports_empty_containers_and_root_level_cards() {
        let mut hierarchy = tree();
        hierarchy.move_node(&NodeId::from("one"), &DropPosition::AppendTo { container: ContainerId::Group("C".into()) }).unwrap();
        hierarchy.move_node(&NodeId::from("two"), &DropPosition::AppendTo { container: ContainerId::Root }).unwrap();
        assert_eq!(ids(&hierarchy.node(&NodeId::from("C")).unwrap().children), ["one"]);
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
        assert_eq!(ids(&hierarchy.node(&NodeId::from("A")).unwrap().children), ["B", "three", "one"]);
    }
}
