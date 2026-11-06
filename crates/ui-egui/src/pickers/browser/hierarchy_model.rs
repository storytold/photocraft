//! Persistence and projection boundary for the picker-browser hierarchy model.
//!
//! The canonical, asset-neutral tree and its move invariants live in `hierarchy_model`.
//! This module translates that model to and from the existing picker persistence representation.

use std::collections::BTreeMap;

use super::super::state::{HierarchyOrderEntry, PresetUi};
use super::identity::PickerNodeIdentity;
use super::path_identity_model::{DropPosition, HierarchyError, HierarchyNode, PickerBrowserHierarchy, PickerId};

/// Apply one resolved browser drop and persist the resulting canonical child sequences.
///
/// `projection` maps opaque runtime IDs to the metadata required to translate the existing
/// name/key-based preference representation. Path-qualified values distinguish equal names in
/// different branches; no encoded `PickerId` strings are interpreted here. A successful move
/// rewrites the existing serialized hierarchy representation from the resulting tree.
pub(crate) fn commit_hierarchy_move(
    state: &mut PresetUi,
    panel: &str,
    hierarchy: &mut PickerBrowserHierarchy,
    source: &PickerId,
    position: &DropPosition,
    identities: &BTreeMap<PickerId, PickerNodeIdentity>,
) -> Result<(), HierarchyError> {
    fn count_nodes(nodes: &[HierarchyNode]) -> usize {
        nodes.iter().map(|node| 1 + count_nodes(node.children())).sum()
    }

    fn validate_identities(nodes: &[HierarchyNode], identities: &BTreeMap<PickerId, PickerNodeIdentity>) -> Result<(), HierarchyError> {
        for node in nodes {
            match (node, identities.get(node.id())) {
                (HierarchyNode::Item(_), Some(PickerNodeIdentity::Item { .. })) => {}
                (HierarchyNode::Group { children, .. }, Some(PickerNodeIdentity::Group(_))) => validate_identities(children, identities)?,
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
        identities: &BTreeMap<PickerId, PickerNodeIdentity>,
    ) -> Result<(), HierarchyError> {
        let mut entries = Vec::with_capacity(nodes.len());
        for node in nodes {
            let entry = match (node, identities.get(node.id())) {
                (HierarchyNode::Item(_), Some(PickerNodeIdentity::Item { group, key })) if parent.is_empty() => {
                    HierarchyOrderEntry::RootItem { group: group.clone(), key: key.clone() }
                }
                (HierarchyNode::Item(_), Some(PickerNodeIdentity::Item { key, .. })) => HierarchyOrderEntry::Item(key.clone()),
                (HierarchyNode::Group { .. }, Some(PickerNodeIdentity::Group(name))) => HierarchyOrderEntry::Group(name.clone()),
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
        identities: &BTreeMap<PickerId, PickerNodeIdentity>,
    ) -> Result<(), HierarchyError> {
        persist_children(state, panel, parent, nodes, identities)?;
        for node in nodes {
            if !node.is_group() {
                continue;
            }
            let Some(PickerNodeIdentity::Group(name)) = identities.get(node.id()) else {
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

    use super::commit_hierarchy_move;
    use crate::pickers::browser::identity::PickerNodeIdentity;
    use crate::pickers::browser::path_identity_model::{ContainerId, DropPosition, HierarchyNode, PickerBrowserHierarchy, PickerId};
    use crate::pickers::state::{HierarchyOrderEntry, PresetUi};

    fn identities() -> BTreeMap<PickerId, PickerNodeIdentity> {
        BTreeMap::from([
            (PickerId::from("A"), PickerNodeIdentity::Group("A".into())),
            (PickerId::from("a1"), PickerNodeIdentity::Item { group: "A".into(), key: "a1".into() }),
            (PickerId::from("B"), PickerNodeIdentity::Group("B".into())),
            (PickerId::from("b1"), PickerNodeIdentity::Item { group: "B".into(), key: "b1".into() }),
            (PickerId::from("a2"), PickerNodeIdentity::Item { group: "A".into(), key: "a2".into() }),
            (PickerId::from("root-one"), PickerNodeIdentity::Item { group: "Original".into(), key: "root-one".into() }),
            (PickerId::from("C"), PickerNodeIdentity::Group("C".into())),
            (PickerId::from("root-two"), PickerNodeIdentity::Item { group: "Original".into(), key: "root-two".into() }),
        ])
    }

    fn assert_persisted_tree(state: &PresetUi, panel: &str, hierarchy: &PickerBrowserHierarchy, identities: &BTreeMap<PickerId, PickerNodeIdentity>) {
        fn collect(
            parent: &str,
            nodes: &[HierarchyNode],
            identities: &BTreeMap<PickerId, PickerNodeIdentity>,
            expected: &mut BTreeMap<String, Vec<HierarchyOrderEntry>>,
        ) {
            let entries = nodes
                .iter()
                .map(|node| match (node, identities.get(node.id())) {
                    (HierarchyNode::Item(_), Some(PickerNodeIdentity::Item { group, key })) if parent.is_empty() => {
                        HierarchyOrderEntry::RootItem { group: group.clone(), key: key.clone() }
                    }
                    (HierarchyNode::Item(_), Some(PickerNodeIdentity::Item { key, .. })) => HierarchyOrderEntry::Item(key.clone()),
                    (HierarchyNode::Group { .. }, Some(PickerNodeIdentity::Group(name))) => HierarchyOrderEntry::Group(name.clone()),
                    _ => panic!("every canonical node must have a matching adapter identity"),
                })
                .collect();
            expected.insert(parent.to_owned(), entries);
            for node in nodes {
                if let HierarchyNode::Group { children, .. } = node {
                    let Some(PickerNodeIdentity::Group(name)) = identities.get(node.id()) else { panic!("group identity is required") };
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
            (PickerId::from("A"), PickerNodeIdentity::Group("A".into())),
            (PickerId::from("one"), PickerNodeIdentity::Item { group: "A".into(), key: "one".into() }),
            (PickerId::from("two"), PickerNodeIdentity::Item { group: "A".into(), key: "two".into() }),
            (PickerId::from("B"), PickerNodeIdentity::Group("B".into())),
        ]);
        let mut state = PresetUi::default();

        commit_hierarchy_move(
            &mut state,
            "gradients",
            &mut hierarchy,
            &PickerId::from("B"),
            &DropPosition::Before { target: PickerId::from("one") },
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
        let mut hierarchy =
            PickerBrowserHierarchy::new(vec![HierarchyNode::group("A", vec![HierarchyNode::item("child")]), HierarchyNode::item("root-card")]).unwrap();
        let identities = BTreeMap::from([
            (PickerId::from("A"), PickerNodeIdentity::Group("A".into())),
            (PickerId::from("child"), PickerNodeIdentity::Item { group: "A".into(), key: "child".into() }),
            (PickerId::from("root-card"), PickerNodeIdentity::Item { group: "Original".into(), key: "root-card".into() }),
        ]);
        let mut state = PresetUi::default();

        commit_hierarchy_move(
            &mut state,
            "gradients",
            &mut hierarchy,
            &PickerId::from("root-card"),
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
            &PickerId::from("root-card"),
            &DropPosition::AppendTo { container: ContainerId::Root },
            &identities,
        )
        .unwrap();
        let [HierarchyOrderEntry::Group(group), HierarchyOrderEntry::RootItem { group: source_group, key }] = state.hierarchy_order["gradients"][""].as_slice()
        else {
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
            commit_hierarchy_move(&mut state, "gradients", &mut hierarchy, &PickerId::from(source), &position, &identities)
                .unwrap_or_else(|error| panic!("{source} -> {position:?}: {error:?}"));
            assert_persisted_tree(&state, "gradients", &hierarchy, &identities);
        }
    }

    #[test]
    fn incomplete_adapter_identity_map_rejects_before_changing_tree_or_persistence() {
        let mut hierarchy =
            PickerBrowserHierarchy::new(vec![HierarchyNode::group("A", vec![HierarchyNode::item("a1")]), HierarchyNode::item("root-one")]).unwrap();
        let original_hierarchy = hierarchy.clone();
        let mut state = PresetUi::default();
        state.set_hierarchy_order("gradients", "", vec![HierarchyOrderEntry::Group("A".into())]);
        let original_state = state.clone();
        let incomplete = BTreeMap::from([
            (PickerId::from("A"), PickerNodeIdentity::Group("A".into())),
            (PickerId::from("root-one"), PickerNodeIdentity::Item { group: "Original".into(), key: "root-one".into() }),
        ]);

        assert_eq!(
            commit_hierarchy_move(
                &mut state,
                "gradients",
                &mut hierarchy,
                &PickerId::from("root-one"),
                &DropPosition::PrependTo { group: "A".into() },
                &incomplete,
            ),
            Err(crate::pickers::browser::path_identity_model::HierarchyError::InvalidTree)
        );
        assert_eq!(hierarchy, original_hierarchy);
        assert_eq!(state, original_state);
    }
}
