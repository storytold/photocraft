//! Persistence and projection boundary for the picker-browser hierarchy model.
//!
//! The canonical, asset-neutral tree and its move invariants live in `picker_browser_model`.
//! This module translates that model to and from the existing picker persistence representation.

use std::collections::BTreeMap;

use super::picker_browser_model::{DropPosition, HierarchyError, HierarchyNode, NodeId, PickerBrowserHierarchy};
use super::picker_state::{HierarchyOrderEntry, PresetUi};

/// Adapter-owned domain identity for projecting a moved hierarchy back to persisted picker state.
/// Item identity deliberately stores the persisted item key and original group; its current parent
/// is derived from the canonical tree after the move, not from the ID's original projection path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum HierarchyIdentity {
    Item { group: String, key: String },
    Group(String),
    /// A nested folder identified by its owning root group and full folder path.
    /// This identity is domain-owned and must not be persisted through the legacy
    /// name-keyed `PresetUi` hierarchy representation.
    Folder { group: String, path: Vec<String> },
    /// An item inside a nested folder, qualified by its owning root group and full folder path.
    FolderItem { group: String, path: Vec<String>, key: String },
}

/// Apply one resolved browser drop and persist the resulting canonical child sequences.
///
/// `identities` is the adapter's reverse map for every node in `hierarchy`; it must remain stable
/// for the lifetime of that projected tree. Path-qualified `NodeId`s distinguish equal group names
/// in different branches and remain unchanged while a node moves. No render indices or encoded
/// `NodeId` strings are interpreted here. A successful move rewrites the existing serialized
/// hierarchy representation from the resulting tree, retiring stale source tokens and anchors.
pub(crate) fn commit_hierarchy_move(
    state: &mut PresetUi,
    panel: &str,
    hierarchy: &mut PickerBrowserHierarchy,
    source: &NodeId,
    position: &DropPosition,
    identities: &BTreeMap<NodeId, HierarchyIdentity>,
) -> Result<(), HierarchyError> {
    fn count_nodes(nodes: &[HierarchyNode]) -> usize {
        nodes.iter().map(|node| 1 + count_nodes(node.children())).sum()
    }

    fn validate_identities(nodes: &[HierarchyNode], identities: &BTreeMap<NodeId, HierarchyIdentity>) -> Result<(), HierarchyError> {
        for node in nodes {
            match (node, identities.get(node.id())) {
                (HierarchyNode::Item(_), Some(HierarchyIdentity::Item { .. })) => {}
                (HierarchyNode::Group { children, .. }, Some(HierarchyIdentity::Group(_))) => validate_identities(children, identities)?,
                _ => return Err(HierarchyError::InvalidTree),
            }
        }
        Ok(())
    }

    // Preflight the complete reverse map before either the hierarchy or serialized state changes.
    validate_identities(hierarchy.roots(), identities)?;
    if identities.len() != count_nodes(hierarchy.roots()) {
        return Err(HierarchyError::InvalidTree);
    }
    let resolved = hierarchy.resolve_move(source, position)?;
    hierarchy.apply_resolved_move(resolved)?;

    fn persist_children(
        state: &mut PresetUi,
        panel: &str,
        parent: &str,
        nodes: &[HierarchyNode],
        identities: &BTreeMap<NodeId, HierarchyIdentity>,
    ) -> Result<(), HierarchyError> {
        let mut entries = Vec::with_capacity(nodes.len());
        for node in nodes {
            let entry = match (node, identities.get(node.id())) {
                (HierarchyNode::Item(_), Some(HierarchyIdentity::Item { group, key })) if parent.is_empty() => {
                    HierarchyOrderEntry::RootItem { group: group.clone(), key: key.clone() }
                }
                (HierarchyNode::Item(_), Some(HierarchyIdentity::Item { key, .. })) => HierarchyOrderEntry::Item(key.clone()),
                (HierarchyNode::Group { .. }, Some(HierarchyIdentity::Group(name))) => HierarchyOrderEntry::Group(name.clone()),
                _ => return Err(HierarchyError::InvalidTree),
            };
            entries.push(entry);
        }
        state.set_hierarchy_order(panel, parent, entries);
        Ok(())
    }

    fn persist_tree(
        state: &mut PresetUi,
        panel: &str,
        parent: &str,
        nodes: &[HierarchyNode],
        identities: &BTreeMap<NodeId, HierarchyIdentity>,
    ) -> Result<(), HierarchyError> {
        persist_children(state, panel, parent, nodes, identities)?;
        for node in nodes {
            if !node.is_group() {
                continue;
            }
            let Some(HierarchyIdentity::Group(name)) = identities.get(node.id()) else {
                return Err(HierarchyError::InvalidTree);
            };
            persist_tree(state, panel, name, node.children(), identities)?;
        }
        Ok(())
    }

    persist_tree(state, panel, "", hierarchy.roots(), identities)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{HierarchyIdentity, commit_hierarchy_move};
    use crate::pickers::picker_browser_model::{ContainerId, DropPosition, HierarchyNode, NodeId, PickerBrowserHierarchy};
    use crate::pickers::picker_state::{HierarchyOrderEntry, PresetUi};

    fn identities() -> BTreeMap<NodeId, HierarchyIdentity> {
        BTreeMap::from([
            (NodeId::from("A"), HierarchyIdentity::Group("A".into())),
            (NodeId::from("a1"), HierarchyIdentity::Item { group: "A".into(), key: "a1".into() }),
            (NodeId::from("B"), HierarchyIdentity::Group("B".into())),
            (NodeId::from("b1"), HierarchyIdentity::Item { group: "B".into(), key: "b1".into() }),
            (NodeId::from("a2"), HierarchyIdentity::Item { group: "A".into(), key: "a2".into() }),
            (NodeId::from("root-one"), HierarchyIdentity::Item { group: "Original".into(), key: "root-one".into() }),
            (NodeId::from("C"), HierarchyIdentity::Group("C".into())),
            (NodeId::from("root-two"), HierarchyIdentity::Item { group: "Original".into(), key: "root-two".into() }),
        ])
    }

    fn assert_persisted_tree(state: &PresetUi, panel: &str, hierarchy: &PickerBrowserHierarchy, identities: &BTreeMap<NodeId, HierarchyIdentity>) {
        fn collect(
            parent: &str,
            nodes: &[HierarchyNode],
            identities: &BTreeMap<NodeId, HierarchyIdentity>,
            expected: &mut BTreeMap<String, Vec<HierarchyOrderEntry>>,
        ) {
            let entries = nodes
                .iter()
                .map(|node| match (node, identities.get(node.id())) {
                    (HierarchyNode::Item(_), Some(HierarchyIdentity::Item { group, key })) if parent.is_empty() => {
                        HierarchyOrderEntry::RootItem { group: group.clone(), key: key.clone() }
                    }
                    (HierarchyNode::Item(_), Some(HierarchyIdentity::Item { key, .. })) => HierarchyOrderEntry::Item(key.clone()),
                    (HierarchyNode::Group { .. }, Some(HierarchyIdentity::Group(name))) => HierarchyOrderEntry::Group(name.clone()),
                    _ => panic!("every canonical node must have a matching adapter identity"),
                })
                .collect();
            expected.insert(parent.to_owned(), entries);
            for node in nodes {
                if let HierarchyNode::Group { children, .. } = node {
                    let Some(HierarchyIdentity::Group(name)) = identities.get(node.id()) else { panic!("group identity is required") };
                    collect(name, children, identities, expected);
                }
            }
        }

        let mut expected = BTreeMap::new();
        collect("", hierarchy.roots(), identities, &mut expected);
        assert_eq!(state.hierarchy_order.get(panel), Some(&expected));
    }

    #[test]
    fn commit_persists_group_before_card_in_the_mixed_sibling_sequence() {
        let mut hierarchy = PickerBrowserHierarchy::new(vec![
            HierarchyNode::group("A", vec![HierarchyNode::item("one"), HierarchyNode::item("two")]),
            HierarchyNode::group("B", vec![]),
        ])
        .unwrap();
        let identities = BTreeMap::from([
            (NodeId::from("A"), HierarchyIdentity::Group("A".into())),
            (NodeId::from("one"), HierarchyIdentity::Item { group: "A".into(), key: "one".into() }),
            (NodeId::from("two"), HierarchyIdentity::Item { group: "A".into(), key: "two".into() }),
            (NodeId::from("B"), HierarchyIdentity::Group("B".into())),
        ]);
        let mut state = PresetUi::default();

        commit_hierarchy_move(
            &mut state,
            "gradients",
            &mut hierarchy,
            &NodeId::from("B"),
            &DropPosition::Before { target: NodeId::from("one") },
            &identities,
        )
        .unwrap();

        assert_eq!(
            state.hierarchy_order["gradients"]["A"],
            [HierarchyOrderEntry::Group("B".into()), HierarchyOrderEntry::Item("one".into()), HierarchyOrderEntry::Item("two".into())]
        );
        assert_eq!(state.parent_of("gradients", "B"), Some("A"));
    }

    #[test]
    fn commit_moves_cards_into_groups_and_back_to_root_without_losing_identity() {
        let mut hierarchy = PickerBrowserHierarchy::new(vec![
            HierarchyNode::group("A", vec![HierarchyNode::item("child")]),
            HierarchyNode::item("root-card"),
        ])
        .unwrap();
        let identities = BTreeMap::from([
            (NodeId::from("A"), HierarchyIdentity::Group("A".into())),
            (NodeId::from("child"), HierarchyIdentity::Item { group: "A".into(), key: "child".into() }),
            (NodeId::from("root-card"), HierarchyIdentity::Item { group: "Original".into(), key: "root-card".into() }),
        ]);
        let mut state = PresetUi::default();

        commit_hierarchy_move(
            &mut state,
            "gradients",
            &mut hierarchy,
            &NodeId::from("root-card"),
            &DropPosition::AppendTo { container: ContainerId::Group("A".into()) },
            &identities,
        )
        .unwrap();
        assert_eq!(state.hierarchy_order["gradients"]["A"], [HierarchyOrderEntry::Item("child".into()), HierarchyOrderEntry::Item("root-card".into())]);
        assert_eq!(state.hierarchy_order["gradients"][""], [HierarchyOrderEntry::Group("A".into())]);

        commit_hierarchy_move(
            &mut state,
            "gradients",
            &mut hierarchy,
            &NodeId::from("root-card"),
            &DropPosition::AppendTo { container: ContainerId::Root },
            &identities,
        )
        .unwrap();
        let [HierarchyOrderEntry::Group(group), HierarchyOrderEntry::RootItem { group: source_group, key }] = state.hierarchy_order["gradients"][""].as_slice() else {
            panic!("the lifted card must be persisted as one typed root item");
        };
        assert_eq!(group, "A");
        assert_eq!(source_group, "Original");
        assert_eq!(key, "root-card");
        assert_eq!(state.hierarchy_order["gradients"]["A"], [HierarchyOrderEntry::Item("child".into())]);
        assert_eq!(state.parent_of("gradients", "A"), None);
    }

    #[test]
    fn repeated_mixed_moves_keep_every_persisted_sibling_sequence_in_sync() {
        let mut hierarchy = PickerBrowserHierarchy::new(vec![
            HierarchyNode::group("A", vec![HierarchyNode::item("a1"), HierarchyNode::group("B", vec![HierarchyNode::item("b1")]), HierarchyNode::item("a2")]),
            HierarchyNode::item("root-one"),
            HierarchyNode::group("C", vec![]),
            HierarchyNode::item("root-two"),
        ])
        .unwrap();
        let identities = identities();
        let mut state = PresetUi::default();
        let moves = [
            ("root-one", DropPosition::Before { target: "b1".into() }),
            ("B", DropPosition::After { target: "a1".into() }),
            ("C", DropPosition::PrependTo { group: "A".into() }),
            ("a2", DropPosition::AppendTo { container: ContainerId::Root }),
            ("b1", DropPosition::Before { target: "root-two".into() }),
            ("A", DropPosition::AppendTo { container: ContainerId::Root }),
        ];

        for (source, position) in moves {
            commit_hierarchy_move(&mut state, "gradients", &mut hierarchy, &NodeId::from(source), &position, &identities)
                .unwrap_or_else(|error| panic!("{source} -> {position:?}: {error:?}"));
            assert_persisted_tree(&state, "gradients", &hierarchy, &identities);
        }
    }

    #[test]
    fn incomplete_adapter_identity_map_rejects_before_changing_tree_or_persistence() {
        let mut hierarchy = PickerBrowserHierarchy::new(vec![HierarchyNode::group("A", vec![HierarchyNode::item("a1")]), HierarchyNode::item("root-one")]).unwrap();
        let original_hierarchy = hierarchy.clone();
        let mut state = PresetUi::default();
        state.set_hierarchy_order("gradients", "", vec![HierarchyOrderEntry::Group("A".into())]);
        let original_state = state.clone();
        let incomplete = BTreeMap::from([
            (NodeId::from("A"), HierarchyIdentity::Group("A".into())),
            (NodeId::from("root-one"), HierarchyIdentity::Item { group: "Original".into(), key: "root-one".into() }),
        ]);

        assert_eq!(
            commit_hierarchy_move(
                &mut state,
                "gradients",
                &mut hierarchy,
                &NodeId::from("root-one"),
                &DropPosition::PrependTo { group: "A".into() },
                &incomplete,
            ),
            Err(crate::pickers::picker_browser_model::HierarchyError::InvalidTree)
        );
        assert_eq!(hierarchy, original_hierarchy);
        assert_eq!(state, original_state);
    }
}
