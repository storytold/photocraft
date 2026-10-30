//! Drag/drop resolution for the canonical picker-browser hierarchy.
//!
//! Hit testing supplies a [`PointerRegion`] with geometry captured from the existing layout. This
//! module maps that region to one exact [`DropPosition`]. The indicator is carried by the same
//! candidate, so preview and release can consume the same resolved result.

#![allow(dead_code)] // Staged as the new drag contract; existing browser adapters are not migrated yet.

use egui::{Pos2, Rect};

use crate::pickers::picker_browser_hierarchy::{ContainerId, DropPosition, HierarchyError, NodeId, PickerBrowserHierarchy};

/// The geometric part of a hit target, independent of its semantic destination.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PointerRegion {
    /// Leading edge of a node: insert before it.
    LeadingEdge,
    /// Trailing edge of a node: insert after it.
    TrailingEdge,
    /// Interior of a group body: append as its last child.
    GroupBody,
    /// Explicit end-of-container area, including the empty-container drop area.
    ContainerEnd,
}

/// Geometry captured by the renderer for one possible drop target.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DropHitRegion {
    /// Identity of the node under the pointer, when the region is node-relative.
    pub(crate) target: Option<NodeId>,
    /// Container used by group-body and container-end regions.
    pub(crate) container: Option<ContainerId>,
    /// Which part of the rendered target was hit.
    pub(crate) region: PointerRegion,
    /// Pointer hit area used by the renderer's hit-test arbitration.
    pub(crate) hit_rect: Rect,
    /// Exact geometry for the indicator (which need not equal the hit area).
    pub(crate) indicator_rect: Rect,
}

/// Shape and exact bounds of the drop preview.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum DropIndicator {
    /// A horizontal or vertical insertion marker immediately before/after a node.
    InsertionLine { rect: Rect },
    /// Highlight the receiving group when the operation appends inside it.
    ContainerHighlight { rect: Rect },
    /// Marker for appending at the end of the root sequence.
    RootAppendLine { rect: Rect },
}

/// Why a resolved structural position is not currently allowed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DropRejection {
    /// The source cannot be inserted into itself or a descendant.
    WouldCreateCycle,
    /// The target is the dragged node itself.
    SameNode,
    /// The receiving container does not accept children.
    UnsupportedContainer,
    /// The source or target disappeared from the current hierarchy.
    StaleNode,
    /// The hierarchy itself is malformed.
    InvalidHierarchy,
}

/// Whether the candidate may be committed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DropValidity {
    /// The operation passed semantic validation.
    Valid,
    /// The operation is visible for feedback but cannot be committed.
    Invalid(DropRejection),
}

/// A fully resolved drag destination and its corresponding visual preview.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DropCandidate {
    /// Exact structural mutation to apply on release.
    pub(crate) position: DropPosition,
    /// Visual marker derived from the same position and hit-region geometry.
    pub(crate) indicator: DropIndicator,
    /// Commit eligibility, kept separate from the meaning of the position.
    pub(crate) validity: DropValidity,
}

/// Transient pointer state for an active drag. It does not mirror or mutate the hierarchy.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DragSession {
    /// Stable identity of the dragged node.
    pub(crate) source: NodeId,
    /// Last pointer position reported by egui.
    pub(crate) pointer: Option<Pos2>,
    /// Current resolved candidate; preview and release must use this same value.
    pub(crate) candidate: Option<DropCandidate>,
}

impl DragSession {
    /// Start tracking a drag from a hierarchy node.
    pub(crate) fn new(source: NodeId) -> Self {
        Self { source, pointer: None, candidate: None }
    }

    /// Update the pointer and resolve a candidate from the hit region under it.
    pub(crate) fn resolve(&mut self, pointer: Pos2, hit: &DropHitRegion, hierarchy: &PickerBrowserHierarchy) -> Option<&DropCandidate> {
        self.pointer = Some(pointer);
        self.candidate = resolve_drop_candidate(&self.source, hit, hierarchy);
        self.candidate.as_ref()
    }

    /// Return the candidate to commit, but only when it is valid.
    pub(crate) fn commit_position(&self) -> Option<&DropPosition> {
        self.candidate.as_ref().filter(|candidate| candidate.validity == DropValidity::Valid).map(|candidate| &candidate.position)
    }
}

/// Convert a geometric hit region into one semantic operation and its matching indicator.
pub(crate) fn resolve_drop_candidate(source: &NodeId, hit: &DropHitRegion, hierarchy: &PickerBrowserHierarchy) -> Option<DropCandidate> {
    let (position, indicator) = match hit.region {
        PointerRegion::LeadingEdge => {
            let target = hit.target.clone()?;
            (DropPosition::Before { target }, DropIndicator::InsertionLine { rect: hit.indicator_rect })
        }
        PointerRegion::TrailingEdge => {
            let target = hit.target.clone()?;
            (DropPosition::After { target }, DropIndicator::InsertionLine { rect: hit.indicator_rect })
        }
        PointerRegion::GroupBody => {
            let container = hit.container.clone()?;
            if container == ContainerId::Root {
                return None;
            }
            let rect = hit.indicator_rect;
            (DropPosition::AppendTo { container }, DropIndicator::ContainerHighlight { rect })
        }
        PointerRegion::ContainerEnd => {
            let container = hit.container.clone()?;
            let rect = hit.indicator_rect;
            let indicator = if container == ContainerId::Root { DropIndicator::RootAppendLine { rect } } else { DropIndicator::InsertionLine { rect } };
            (DropPosition::AppendTo { container }, indicator)
        }
    };
    let validity = match hierarchy.validate_position(source, &position) {
        Ok(()) => DropValidity::Valid,
        Err(HierarchyError::Cycle(id)) if matches!(&position, DropPosition::Before { target } | DropPosition::After { target } if target == &id) => {
            DropValidity::Invalid(DropRejection::SameNode)
        }
        Err(HierarchyError::Cycle(_)) => DropValidity::Invalid(DropRejection::WouldCreateCycle),
        Err(HierarchyError::ItemCannotContainChildren(_) | HierarchyError::MissingContainer(_)) => DropValidity::Invalid(DropRejection::UnsupportedContainer),
        Err(HierarchyError::MissingSource(_) | HierarchyError::MissingTarget(_)) => DropValidity::Invalid(DropRejection::StaleNode),
        Err(HierarchyError::InvalidTree) => DropValidity::Invalid(DropRejection::InvalidHierarchy),
    };
    Some(DropCandidate { position, indicator, validity })
}

#[cfg(test)]
mod tests {
    use super::{DropHitRegion, DropIndicator, DropRejection, DropValidity, PointerRegion, resolve_drop_candidate};
    use crate::pickers::picker_browser_hierarchy::{ContainerId, DropPosition, HierarchyNode, NodeId, PickerBrowserHierarchy};
    use egui::{Pos2, Rect};

    fn hit(region: PointerRegion, target: Option<&str>, container: Option<ContainerId>) -> DropHitRegion {
        DropHitRegion {
            target: target.map(NodeId::from),
            container,
            region,
            hit_rect: Rect::from_min_max(Pos2::new(10.0, 20.0), Pos2::new(110.0, 40.0)),
            indicator_rect: Rect::from_min_max(Pos2::new(10.0, 20.0), Pos2::new(110.0, 24.0)),
        }
    }

    fn hierarchy() -> PickerBrowserHierarchy {
        PickerBrowserHierarchy::new(vec![HierarchyNode::group("folder", vec![HierarchyNode::item("card")]), HierarchyNode::group("other", vec![])]).unwrap()
    }

    #[test]
    fn node_edges_resolve_to_sibling_insertions_and_share_indicator_bounds() {
        let leading = resolve_drop_candidate(&NodeId::from("card"), &hit(PointerRegion::LeadingEdge, Some("other"), None), &hierarchy()).unwrap();
        assert_eq!(leading.position, DropPosition::Before { target: "other".into() });
        assert_eq!(leading.indicator, DropIndicator::InsertionLine { rect: hit(PointerRegion::LeadingEdge, Some("other"), None).indicator_rect });

        let trailing = resolve_drop_candidate(&NodeId::from("card"), &hit(PointerRegion::TrailingEdge, Some("other"), None), &hierarchy()).unwrap();
        assert_eq!(trailing.position, DropPosition::After { target: "other".into() });
    }

    #[test]
    fn group_body_and_container_end_are_explicit_append_operations() {
        let body = resolve_drop_candidate(&NodeId::from("card"), &hit(PointerRegion::GroupBody, None, Some(ContainerId::Group("other".into()))), &hierarchy())
            .unwrap();
        assert_eq!(body.position, DropPosition::AppendTo { container: ContainerId::Group("other".into()) });
        assert!(matches!(body.indicator, DropIndicator::ContainerHighlight { .. }));

        let root_end = resolve_drop_candidate(&NodeId::from("card"), &hit(PointerRegion::ContainerEnd, None, Some(ContainerId::Root)), &hierarchy()).unwrap();
        assert_eq!(root_end.position, DropPosition::AppendTo { container: ContainerId::Root });
        assert!(matches!(root_end.indicator, DropIndicator::RootAppendLine { .. }));
    }

    #[test]
    fn self_drop_is_marked_invalid_without_changing_its_structural_meaning() {
        let candidate = resolve_drop_candidate(&NodeId::from("card"), &hit(PointerRegion::LeadingEdge, Some("card"), None), &hierarchy()).unwrap();
        assert_eq!(candidate.position, DropPosition::Before { target: "card".into() });
        assert_eq!(candidate.validity, DropValidity::Invalid(DropRejection::SameNode));
    }

    #[test]
    fn group_drop_into_descendant_is_invalid_even_when_target_is_not_the_source() {
        let hierarchy =
            PickerBrowserHierarchy::new(vec![HierarchyNode::group("parent", vec![HierarchyNode::group("child", vec![HierarchyNode::item("nested-card")])])])
                .unwrap();
        let candidate =
            resolve_drop_candidate(&NodeId::from("parent"), &hit(PointerRegion::GroupBody, None, Some(ContainerId::Group("child".into()))), &hierarchy)
                .unwrap();
        assert_eq!(candidate.validity, DropValidity::Invalid(DropRejection::WouldCreateCycle));
    }
}
