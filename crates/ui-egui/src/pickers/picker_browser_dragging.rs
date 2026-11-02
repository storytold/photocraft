//! Drag/drop resolution for the canonical picker-browser hierarchy.
//!
//! Hit testing supplies a [`PointerRegion`] with geometry captured from the existing layout. This
//! module maps that region to one exact [`DropPosition`]. The indicator is carried by the same
//! candidate, so preview and release can consume the same resolved result.

use egui::{Pos2, Rect};

use crate::pickers::picker_browser_model::{ContainerId, DropPosition, HierarchyError, NodeId, PickerBrowserHierarchy};

/// The geometric part of a hit target, independent of its semantic destination.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PointerRegion {
    /// Leading edge of a node: insert before it.
    LeadingEdge,
    /// Trailing edge of a node: insert after it.
    TrailingEdge,
    /// Interior of a group header: insert as its first child.
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
    /// Structural depth; deeper targets win when hit areas overlap.
    pub(crate) depth: usize,
    /// Stable registration order used to break equal-depth overlaps.
    pub(crate) order: usize,
}

/// Shape and exact bounds of the drop preview.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum DropIndicator {
    /// A horizontal or vertical insertion marker immediately before/after a node.
    InsertionLine { rect: Rect },
    /// Highlight the receiving group when the operation inserts inside it.
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
    /// The move would exceed the supported group nesting depth.
    DepthExceeded,
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

/// A move request that has passed canonical hierarchy validation.
///
/// Adapters receive this value rather than reconstructing a destination from pointer geometry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DropMoveIntent {
    pub(crate) source: NodeId,
    pub(crate) position: DropPosition,
}

/// Outcome of handing a validated move intent to its domain adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DropCommitResult<E> {
    /// There was no current hit candidate to commit.
    NoCandidate,
    /// The current candidate failed canonical hierarchy validation.
    Rejected(DropRejection),
    /// The adapter was called and declined or failed the move.
    Failed(E),
    /// The adapter accepted the exact validated move intent.
    Committed,
}

impl DropCandidate {
    /// Whether this candidate is eligible for commit.
    pub(crate) fn is_valid(&self) -> bool {
        self.validity == DropValidity::Valid
    }
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

    /// Return the exact source/destination pair only when the current candidate is valid.
    pub(crate) fn validated_intent(&self) -> Result<Option<DropMoveIntent>, DropRejection> {
        let Some(candidate) = self.candidate.as_ref() else {
            return Ok(None);
        };
        if let DropValidity::Invalid(reason) = &candidate.validity {
            return Err(reason.clone());
        }
        Ok(Some(DropMoveIntent { source: self.source.clone(), position: candidate.position.clone() }))
    }

    /// Hand one validated intent to the adapter and preserve its failure as an explicit result.
    /// The adapter owns domain acceptance and commit; this session never mutates the hierarchy.
    pub(crate) fn commit_with<E>(&self, commit: impl FnOnce(&DropMoveIntent) -> Result<(), E>) -> DropCommitResult<E> {
        match self.validated_intent() {
            Ok(None) => DropCommitResult::NoCandidate,
            Err(reason) => DropCommitResult::Rejected(reason),
            Ok(Some(intent)) => match commit(&intent) {
                Ok(()) => DropCommitResult::Committed,
                Err(error) => DropCommitResult::Failed(error),
            },
        }
    }
}

/// Return the winning hit's index so the browser can take the payload from that exact response.
/// Deepest targets win; equal-depth targets prefer later (more specific) registrations.
pub(crate) fn arbitrate_hit_index(pointer: Pos2, hits: &[DropHitRegion]) -> Option<usize> {
    hits.iter()
        .enumerate()
        .filter(|(_, hit)| hit.hit_rect.contains(pointer))
        .max_by_key(|(_, hit)| (hit.depth, hit.order))
        .map(|(index, _)| index)
}

/// Paint the indicator carried by a valid resolved candidate. Invalid destinations intentionally
/// remain visually quiet rather than drawing distracting error-colored bars.
pub(crate) fn paint_candidate(ui: &egui::Ui, candidate: &DropCandidate, color: egui::Color32, clip_rect: Rect) {
    if !candidate.is_valid() {
        return;
    }
    // Drop feedback must sit above group headers/cards, especially for the root-leading marker
    // which intentionally shares the first header's top border. Keep it clipped to the browser.
    let layer = egui::LayerId::new(egui::Order::Foreground, ui.layer_id().id);
    let painter = ui.ctx().layer_painter(layer).with_clip_rect(clip_rect);
    match &candidate.indicator {
        DropIndicator::InsertionLine { rect } | DropIndicator::RootAppendLine { rect } => {
            painter.rect_filled(*rect, egui::CornerRadius::ZERO, color);
        }
        DropIndicator::ContainerHighlight { rect } => {
            for border in container_outline_rects(*rect) {
                painter.rect_filled(border, egui::CornerRadius::ZERO, color);
            }
        }
    }
}

/// Inward-facing four-pixel border used when a validated candidate appends into a group.
/// Keeping the geometry separate makes the preview's thickness independently reviewable.
fn container_outline_rects(rect: Rect) -> [Rect; 4] {
    [
        Rect::from_min_max(rect.min, Pos2::new(rect.right(), (rect.top() + 4.0).min(rect.bottom()))),
        Rect::from_min_max(Pos2::new(rect.left(), (rect.bottom() - 4.0).max(rect.top())), rect.max),
        Rect::from_min_max(rect.min, Pos2::new((rect.left() + 4.0).min(rect.right()), rect.bottom())),
        Rect::from_min_max(Pos2::new((rect.right() - 4.0).max(rect.left()), rect.top()), rect.max),
    ]
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
            let ContainerId::Group(group) = hit.container.clone()? else { return None };
            let rect = hit.indicator_rect;
            (DropPosition::PrependTo { group }, DropIndicator::ContainerHighlight { rect })
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
        Err(HierarchyError::DepthExceeded) => DropValidity::Invalid(DropRejection::DepthExceeded),
    };
    Some(DropCandidate { position, indicator, validity })
}

#[cfg(test)]
mod tests {
    use super::{DropCommitResult, DropHitRegion, DropIndicator, DropRejection, DropValidity, DragSession, PointerRegion, arbitrate_hit_index, container_outline_rects, resolve_drop_candidate};
    use crate::pickers::picker_browser_model::{ContainerId, DropPosition, HierarchyNode, NodeId, PickerBrowserHierarchy};
    use egui::{Pos2, Rect};

    fn hit(region: PointerRegion, target: Option<&str>, container: Option<ContainerId>) -> DropHitRegion {
        DropHitRegion {
            target: target.map(NodeId::from),
            container,
            region,
            hit_rect: Rect::from_min_max(Pos2::new(10.0, 20.0), Pos2::new(110.0, 40.0)),
            indicator_rect: Rect::from_min_max(Pos2::new(10.0, 20.0), Pos2::new(110.0, 24.0)),
            depth: 0,
            order: 0,
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
    fn dragging_a_group_onto_a_card_uses_that_cards_parent_sequence() {
        let candidate = resolve_drop_candidate(
            &NodeId::from("other"),
            &hit(PointerRegion::TrailingEdge, Some("card"), None),
            &hierarchy(),
        )
        .unwrap();
        assert_eq!(candidate.position, DropPosition::After { target: "card".into() });
        assert_eq!(candidate.validity, DropValidity::Valid);
        assert!(candidate.is_valid());
    }

    #[test]
    fn group_body_prepends_while_container_end_appends() {
        let body = resolve_drop_candidate(&NodeId::from("card"), &hit(PointerRegion::GroupBody, None, Some(ContainerId::Group("other".into()))), &hierarchy())
            .unwrap();
        assert_eq!(body.position, DropPosition::PrependTo { group: NodeId::from("other") });
        assert!(matches!(body.indicator, DropIndicator::ContainerHighlight { .. }));

        let group = resolve_drop_candidate(
            &NodeId::from("other"),
            &hit(PointerRegion::GroupBody, None, Some(ContainerId::Group("folder".into()))),
            &hierarchy(),
        ).unwrap();
        assert_eq!(group.position, DropPosition::PrependTo { group: NodeId::from("folder") });
        assert!(group.is_valid());

        let root_end = resolve_drop_candidate(&NodeId::from("card"), &hit(PointerRegion::ContainerEnd, None, Some(ContainerId::Root)), &hierarchy()).unwrap();
        assert_eq!(root_end.position, DropPosition::AppendTo { container: ContainerId::Root });
        assert!(matches!(root_end.indicator, DropIndicator::RootAppendLine { .. }));

        let group_end = resolve_drop_candidate(
            &NodeId::from("card"),
            &hit(PointerRegion::ContainerEnd, None, Some(ContainerId::Group("other".into()))),
            &hierarchy(),
        ).unwrap();
        assert_eq!(group_end.position, DropPosition::AppendTo { container: ContainerId::Group("other".into()) });
        assert!(matches!(group_end.indicator, DropIndicator::InsertionLine { .. }));
    }

    #[test]
    fn self_drop_is_marked_invalid_without_changing_its_structural_meaning() {
        let candidate = resolve_drop_candidate(&NodeId::from("card"), &hit(PointerRegion::LeadingEdge, Some("card"), None), &hierarchy()).unwrap();
        assert_eq!(candidate.position, DropPosition::Before { target: "card".into() });
        assert_eq!(candidate.validity, DropValidity::Invalid(DropRejection::SameNode));
        assert!(!candidate.is_valid());
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
        assert!(!candidate.is_valid());
    }

    #[test]
    fn arbitration_prefers_deepest_then_latest_hit() {
        let pointer = Pos2::new(20.0, 25.0);
        let mut shallow = hit(PointerRegion::LeadingEdge, Some("other"), None);
        let mut deep = hit(PointerRegion::TrailingEdge, Some("other"), None);
        shallow.depth = 1;
        deep.depth = 2;
        assert_eq!(arbitrate_hit_index(pointer, &[deep.clone(), shallow.clone()]), Some(0));
        deep.depth = 1;
        assert_eq!(arbitrate_hit_index(pointer, &[shallow, deep.clone()]), Some(1));
    }

    #[test]
    fn overlapping_drop_regions_always_select_exactly_one_deepest_latest_candidate() {
        let pointer = Pos2::new(20.0, 25.0);
        for depth_count in 1..=5 {
            for order_count in 1..=5 {
                let mut hits = Vec::new();
                for depth in 0..depth_count {
                    for order in 0..order_count {
                        let mut candidate = hit(PointerRegion::LeadingEdge, Some("target"), None);
                        candidate.depth = depth;
                        candidate.order = order;
                        hits.push(candidate);
                    }
                }
                let expected = hits.iter().enumerate().max_by_key(|(_, candidate)| (candidate.depth, candidate.order)).map(|(index, _)| index);
                assert_eq!(arbitrate_hit_index(pointer, &hits), expected, "depth_count={depth_count}, order_count={order_count}");
                assert!(arbitrate_hit_index(Pos2::new(500.0, 500.0), &hits).is_none());
            }
        }
    }

    #[test]
    fn root_append_is_an_explicit_container_end_candidate() {
        let candidate = resolve_drop_candidate(
            &NodeId::from("card"),
            &hit(PointerRegion::ContainerEnd, None, Some(ContainerId::Root)),
            &hierarchy(),
        )
        .unwrap();
        assert_eq!(candidate.position, DropPosition::AppendTo { container: ContainerId::Root });
        assert_eq!(candidate.validity, DropValidity::Valid);
    }

    #[test]
    fn group_append_preview_uses_an_inward_four_pixel_outline() {
        let target = Rect::from_min_max(Pos2::new(10.0, 20.0), Pos2::new(210.0, 80.0));

        let [top, bottom, left, right] = container_outline_rects(target);

        assert_eq!(top.height(), 4.0);
        assert_eq!(bottom.height(), 4.0);
        assert_eq!(left.width(), 4.0);
        assert_eq!(right.width(), 4.0);
        assert_eq!(top.top(), target.top());
        assert_eq!(bottom.bottom(), target.bottom());
        assert_eq!(left.left(), target.left());
        assert_eq!(right.right(), target.right());
    }

    #[test]
    fn commit_hands_the_exact_validated_intent_to_the_adapter() {
        let hierarchy = hierarchy();
        let mut session = DragSession::new(NodeId::from("card"));
        session.resolve(Pos2::new(20.0, 25.0), &hit(PointerRegion::LeadingEdge, Some("other"), None), &hierarchy);

        let result = session.commit_with(|intent| {
            assert_eq!(intent.source, NodeId::from("card"));
            assert_eq!(intent.position, DropPosition::Before { target: "other".into() });
            Ok::<(), &'static str>(())
        });

        assert_eq!(result, DropCommitResult::Committed);
    }

    #[test]
    fn invalid_or_missing_candidates_never_reach_the_adapter() {
        let hierarchy = hierarchy();
        let mut invalid = DragSession::new(NodeId::from("card"));
        invalid.resolve(Pos2::new(20.0, 25.0), &hit(PointerRegion::LeadingEdge, Some("card"), None), &hierarchy);
        let mut called = false;
        let rejected = invalid.commit_with(|_| {
            called = true;
            Ok::<(), ()>(())
        });
        assert_eq!(rejected, DropCommitResult::Rejected(DropRejection::SameNode));
        assert!(!called);

        let empty = DragSession::new(NodeId::from("card"));
        assert_eq!(empty.commit_with(|_| Ok::<(), ()>(())), DropCommitResult::NoCandidate);
    }

    #[test]
    fn every_source_and_drop_region_uses_model_validation_before_adapter_commit() {
        let hierarchy = PickerBrowserHierarchy::new(vec![
            HierarchyNode::group(
                "A",
                vec![HierarchyNode::item("a1"), HierarchyNode::group("B", vec![HierarchyNode::item("b1")]), HierarchyNode::item("a2")],
            ),
            HierarchyNode::item("root-card"),
            HierarchyNode::group("C", vec![]),
        ])
        .unwrap();
        fn collect_ids(nodes: &[HierarchyNode], output: &mut Vec<NodeId>) {
            for node in nodes {
                output.push(node.id().clone());
                collect_ids(node.children(), output);
            }
        }
        let mut ids = Vec::new();
        collect_ids(hierarchy.roots(), &mut ids);

        let mut hits = Vec::new();
        for target in &ids {
            hits.push(hit(PointerRegion::LeadingEdge, Some(&target.to_string()), None));
            hits.push(hit(PointerRegion::TrailingEdge, Some(&target.to_string()), None));
            if hierarchy.node(target).is_some_and(HierarchyNode::is_group) {
                hits.push(hit(PointerRegion::GroupBody, None, Some(ContainerId::Group(target.clone()))));
                hits.push(hit(PointerRegion::ContainerEnd, None, Some(ContainerId::Group(target.clone()))));
            }
        }
        hits.push(hit(PointerRegion::ContainerEnd, None, Some(ContainerId::Root)));

        for source in &ids {
            for hit in &hits {
                let Some(candidate) = resolve_drop_candidate(source, hit, &hierarchy) else { panic!("every well-formed hit region resolves") };
                let model_accepts = hierarchy.validate_position(source, &candidate.position).is_ok();
                assert_eq!(candidate.is_valid(), model_accepts, "source={source:?}, hit={hit:?}");

                let mut session = DragSession::new(source.clone());
                session.resolve(Pos2::new(20.0, 25.0), hit, &hierarchy);
                let mut adapter_called = false;
                let result = session.commit_with(|intent| {
                    adapter_called = true;
                    assert_eq!(&intent.source, source);
                    assert_eq!(intent.position, candidate.position);
                    Ok::<(), ()>(())
                });
                if model_accepts {
                    assert_eq!(result, DropCommitResult::Committed, "source={source:?}, hit={hit:?}");
                    assert!(adapter_called);
                } else {
                    assert!(matches!(result, DropCommitResult::Rejected(_)), "source={source:?}, hit={hit:?}");
                    assert!(!adapter_called, "invalid candidates must not reach the adapter: source={source:?}, hit={hit:?}");
                }
            }
        }
    }

    #[test]
    fn adapter_failure_is_preserved_as_a_commit_result() {
        let hierarchy = hierarchy();
        let mut session = DragSession::new(NodeId::from("card"));
        session.resolve(Pos2::new(20.0, 25.0), &hit(PointerRegion::LeadingEdge, Some("other"), None), &hierarchy);

        assert_eq!(session.commit_with(|_| Err("adapter rejected move")), DropCommitResult::Failed("adapter rejected move"));
    }
}
