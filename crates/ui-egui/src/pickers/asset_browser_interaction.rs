//! Shared hit testing and interaction semantics for asset browsers.
//!
//! This module owns the egui interaction contract only: consumers translate the resulting
//! gestures and drag payloads into their own state changes or domain commands.

use egui::{Pos2, Rect};

use crate::PhotocraftApp;
use crate::pickers::picker_browser_dragging::{DropHitRegion, PointerRegion};
use crate::pickers::picker_browser_model::{ContainerId, HierarchyNode, NodeId, PickerBrowserHierarchy};
use crate::theme::Tokens;

/// Canvas drop bounds used by preset-browser adapters, excluding the ruler strips.
pub(crate) fn canvas_drop_rect(app: &PhotocraftApp) -> Rect {
    crate::rulers::content_rect(app, app.last_canvas_rect)
}

/// Shared payload for dragging an asset or one of its group headers.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum AssetDrag {
    Preset {
        scope: String,
        key: String,
        /// Exact hierarchy identity for canonical drag arbitration. Legacy consumers may omit it.
        node_id: Option<NodeId>,
    },
    /// Any group node, including a nested group. `path` is empty for a root group.
    Group {
        scope: String,
        name: String,
        path: Vec<String>,
    },
    /// Canonical group source with an exact path-qualified node identity.
    CanonicalGroup { scope: String, node_id: NodeId },
}

impl AssetDrag {
    /// Return the path-qualified canonical source identity for this browser scope.
    /// Legacy name/path payloads intentionally do not produce canonical move requests.
    pub(crate) fn canonical_node_id(&self, expected_scope: &str) -> Option<&NodeId> {
        match self {
            Self::Preset { scope, node_id: Some(node_id), .. } | Self::CanonicalGroup { scope, node_id }
                if scope == expected_scope => Some(node_id),
            _ => None,
        }
    }
}

/// Shared header hit convention for legacy hierarchy adapters: dropping on a group header means
/// dropping into that group. Sibling reordering is handled by the gaps between headers.
pub(crate) fn header_drop_before(_rect: Rect, _pointer: Pos2) -> bool {
    false
}

/// Whether an item drop resolves after the hovered card in its row.
/// Use the same decision for the preview marker and the committed insertion index.
pub(crate) fn item_drop_after(rect: Rect, pointer: Pos2, grid: bool) -> bool {
    if grid { pointer.x > rect.center().x } else { pointer.y > rect.center().y }
}

/// Empty space after the last card in a grid row remains a valid append target.
/// Prepend is deliberately limited to the leading half of the card itself.
pub(crate) fn item_row_edge_targets(area: Rect, item_rect: Rect, last: bool) -> Option<Rect> {
    let right = last
        .then(|| Rect::from_min_max(Pos2::new(item_rect.right(), item_rect.top()), Pos2::new(area.right(), item_rect.bottom())))
        .filter(|rect| rect.is_positive());
    right
}

/// Capture a card's leading/trailing half as a semantic sibling insertion target. The returned
/// hit rectangle remains the card bounds; indicator geometry is a separate narrow line.
pub(crate) fn card_drop_hit(rect: Rect, pointer: Pos2, grid: bool, target: NodeId, depth: usize, order: usize) -> DropHitRegion {
    let after = item_drop_after(rect, pointer, grid);
    let indicator_rect = if grid {
        let x = if after { rect.right() } else { rect.left() };
        Rect::from_min_max(Pos2::new(x - 2.0, rect.top()), Pos2::new(x + 2.0, rect.bottom()))
    } else {
        let y = if after { rect.bottom() } else { rect.top() };
        Rect::from_min_max(Pos2::new(rect.left(), y - 2.0), Pos2::new(rect.right(), y + 2.0))
    };
    DropHitRegion {
        target: Some(target),
        container: None,
        region: if after { PointerRegion::TrailingEdge } else { PointerRegion::LeadingEdge },
        hit_rect: rect,
        indicator_rect,
        depth,
        order,
    }
}

/// Register both halves of a card so the debug view and hit arbitration use the same full-time
/// geometry even when the pointer is not currently over the card.
pub(crate) fn card_drop_hits(rect: Rect, grid: bool, target: NodeId, depth: usize, order: usize) -> [DropHitRegion; 2] {
    let (leading, trailing) = if grid {
        let middle = rect.center().x;
        (
            Rect::from_min_max(rect.min, Pos2::new(middle, rect.bottom())),
            Rect::from_min_max(Pos2::new(middle, rect.top()), rect.max),
        )
    } else {
        let middle = rect.center().y;
        (
            Rect::from_min_max(rect.min, Pos2::new(rect.right(), middle)),
            Rect::from_min_max(Pos2::new(rect.left(), middle), rect.max),
        )
    };
    let leading_hit = card_drop_hit(rect, if grid { Pos2::new(rect.left(), rect.center().y) } else { Pos2::new(rect.center().x, rect.top()) }, grid, target.clone(), depth, order);
    let trailing_hit = card_drop_hit(
        rect,
        if grid { Pos2::new(rect.right(), rect.center().y) } else { Pos2::new(rect.center().x, rect.bottom()) },
        grid,
        target,
        depth,
        order.saturating_add(1),
    );
    [DropHitRegion { hit_rect: leading, ..leading_hit }, DropHitRegion { hit_rect: trailing, ..trailing_hit }]
}

/// Capture a group header as a drop-onto target. Every point on the header prepends inside it.
pub(crate) fn group_header_drop_hit(
    rect: Rect,
    _pointer: Pos2,
    target: NodeId,
    depth: usize,
    order: usize,
) -> DropHitRegion {
    DropHitRegion {
        target: None,
        container: Some(ContainerId::Group(target)),
        region: PointerRegion::GroupBody,
        hit_rect: rect,
        indicator_rect: rect,
        depth,
        order,
    }
}

/// Capture a drop-at-end target spanning a group's visible subtree (e.g. the indent column).
pub(crate) fn group_append_hit(hit_rect: Rect, indicator_rect: Rect, group: NodeId, depth: usize, order: usize) -> DropHitRegion {
    DropHitRegion {
        target: None,
        container: Some(ContainerId::Group(group)),
        region: PointerRegion::ContainerEnd,
        hit_rect,
        indicator_rect,
        depth,
        order,
    }
}

/// Split the six-pixel separation before a group into upper- and lower-level drop targets.
/// The upper half appends after the last direct child of the preceding group; the lower half
/// inserts before the following group. Each half paints over its own adjacent element's width.
pub(crate) fn group_gap_drop_hits(
    rect: Rect,
    upper: NodeId,
    lower: NodeId,
    upper_bounds: Rect,
    lower_bounds: Rect,
    upper_depth: usize,
    lower_depth: usize,
    order: usize,
) -> Vec<DropHitRegion> {
    debug_assert!((rect.height() - 6.0).abs() < f32::EPSILON);
    let middle = rect.center().y;
    let upper_line = Rect::from_min_max(Pos2::new(upper_bounds.left(), rect.top()), Pos2::new(upper_bounds.right(), middle));
    let lower_line = Rect::from_min_max(Pos2::new(lower_bounds.left(), middle), Pos2::new(lower_bounds.right(), rect.bottom()));
    vec![
            DropHitRegion {
                target: Some(upper),
                container: None,
                region: PointerRegion::TrailingEdge,
                hit_rect: Rect::from_min_max(rect.min, Pos2::new(rect.right(), middle)),
                indicator_rect: upper_line,
                depth: upper_depth,
                order,
            },
            DropHitRegion {
                target: Some(lower),
                container: None,
                region: PointerRegion::LeadingEdge,
                hit_rect: Rect::from_min_max(Pos2::new(rect.left(), middle), rect.max),
                indicator_rect: lower_line,
                depth: lower_depth,
                order: order.saturating_add(1),
            },
    ]
}

/// Return the last direct card in an upper group's child sequence, or the group itself when empty.
/// This is a fallback for gap layouts where the lower group's parent sequence has no preceding node.
pub(crate) fn group_gap_upper_anchor(hierarchy: &PickerBrowserHierarchy, upper_group: &NodeId) -> (NodeId, usize) {
    let Some(group) = hierarchy.node(upper_group).filter(|node| node.is_group()) else {
        return (upper_group.clone(), 0);
    };
    let depth = hierarchy
        .parent_of(upper_group)
        .map(|mut parent| {
            let mut depth = 0;
            while let ContainerId::Group(id) = parent {
                depth += 1;
                parent = hierarchy.parent_of(&id).unwrap_or(ContainerId::Root);
            }
            depth
        })
        .unwrap_or(0);
    group.children().iter().rev().find_map(|node| match node {
        HierarchyNode::Item(id) => Some((id.clone(), depth + 1)),
        HierarchyNode::Group { .. } => None,
    }).unwrap_or((upper_group.clone(), depth))
}

/// Capture the explicit end of the root sibling sequence, including an empty browser's drop area.
pub(crate) fn root_append_hit(rect: Rect, indicator_rect: Rect, order: usize) -> DropHitRegion {
    DropHitRegion {
        target: None,
        container: Some(ContainerId::Root),
        region: PointerRegion::ContainerEnd,
        hit_rect: rect,
        indicator_rect,
        depth: 0,
        order,
    }
}

/// Capture the leading edge of the root sibling sequence, immediately before its first node.
pub(crate) fn root_prepend_hit(rect: Rect, indicator_rect: Rect, first: NodeId, order: usize) -> DropHitRegion {
    DropHitRegion {
        target: Some(first),
        container: None,
        region: PointerRegion::LeadingEdge,
        hit_rect: rect,
        indicator_rect,
        depth: 0,
        order,
    }
}

/// Semantic destination for dragging a root or nested group onto a hierarchy header.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum HierarchyDrop {
    MoveGroup { group: String, group_path: Vec<String>, before: Option<String> },
    MoveInto { group: String, group_path: Vec<String>, into: Option<(String, Vec<String>)>, before: Option<String> },
}

/// Which kinds of drops a particular hierarchy header accepts.
#[derive(Clone, Copy)]
pub(crate) struct HeaderDropCapabilities {
    pub(crate) item_drop: bool,
    pub(crate) group_drag: bool,
    /// The target is inside the source group's own descendant hierarchy.
    pub(crate) reject_group_drop: bool,
}

fn paint_group_drop_outline(ui: &egui::Ui, rect: Rect, color: egui::Color32) {
    if !rect.is_positive() {
        return;
    }
    let painter = ui.painter();
    // Paint the border inward from the target's actual bounds: no outer inset or rounded corners.
    for border in group_drop_outline_rects(rect) {
        painter.rect_filled(border, egui::CornerRadius::ZERO, color);
    }
}

fn group_drop_outline_rects(rect: Rect) -> [Rect; 4] {
    [
        Rect::from_min_max(rect.min, Pos2::new(rect.right(), (rect.top() + 4.0).min(rect.bottom()))),
        Rect::from_min_max(Pos2::new(rect.left(), (rect.bottom() - 4.0).max(rect.top())), rect.max),
        Rect::from_min_max(rect.min, Pos2::new((rect.left() + 4.0).min(rect.right()), rect.bottom())),
        Rect::from_min_max(Pos2::new((rect.right() - 4.0).max(rect.left()), rect.top()), rect.max),
    ]
}

pub(crate) fn paint_group_insertion_line(ui: &egui::Ui, rect: Rect, before: bool, color: egui::Color32) {
    ui.painter().rect_filled(group_insertion_marker_rect(rect, before), egui::CornerRadius::ZERO, color);
}

fn group_insertion_marker_rect(rect: Rect, before: bool) -> Rect {
    let center_y = if before { rect.top() - 1.0 } else { rect.bottom() + 1.0 };
    Rect::from_min_max(Pos2::new(rect.left(), center_y - 2.0), Pos2::new(rect.right(), center_y + 2.0))
}

#[cfg(test)]
mod insertion_marker_tests {
    use super::{card_drop_hit, card_drop_hits, group_append_hit, group_drop_outline_rects, group_gap_drop_hits, group_gap_upper_anchor, group_header_drop_hit, group_insertion_marker_rect, root_append_hit};
    use crate::pickers::picker_browser_model::{ContainerId, DropPosition, HierarchyNode, NodeId, PickerBrowserHierarchy};
    use crate::pickers::picker_browser_dragging::{DropIndicator, PointerRegion, resolve_drop_candidate};
    use egui::{Pos2, Rect};

    #[test]
    fn group_insertion_marker_is_four_pixels_tall_and_centered_in_the_gap() {
        let row = Rect::from_min_max(Pos2::new(10.0, 30.0), Pos2::new(210.0, 60.0));

        let marker = group_insertion_marker_rect(row, true);

        assert_eq!(marker.height(), 4.0);
        assert_eq!(marker.center().y, row.top() - 1.0);
    }

    #[test]
    fn group_drop_outline_is_four_pixels_thick_on_all_sides() {
        let target = Rect::from_min_max(Pos2::new(10.0, 30.0), Pos2::new(210.0, 90.0));

        let [top, bottom, left, right] = group_drop_outline_rects(target);

        assert_eq!(top.height(), 4.0);
        assert_eq!(bottom.height(), 4.0);
        assert_eq!(left.width(), 4.0);
        assert_eq!(right.width(), 4.0);
        assert_eq!(left.left(), target.left());
        assert_eq!(right.right(), target.right());
    }

    #[test]
    fn card_and_group_hit_regions_keep_hit_and_indicator_geometry_separate() {
        let card = Rect::from_min_max(Pos2::new(10.0, 30.0), Pos2::new(110.0, 70.0));
        let hit = card_drop_hit(card, Pos2::new(90.0, 40.0), true, NodeId::from("card"), 1, 2);
        assert_eq!(hit.hit_rect, card);
        assert_eq!(hit.region, PointerRegion::TrailingEdge);
        assert_eq!(hit.indicator_rect.width(), 4.0);

        let halves = card_drop_hits(card, true, NodeId::from("card"), 1, 2);
        assert_eq!(halves[0].region, PointerRegion::LeadingEdge);
        assert_eq!(halves[0].hit_rect.width(), 50.0);
        assert_eq!(halves[1].region, PointerRegion::TrailingEdge);
        assert_eq!(halves[1].hit_rect.width(), 50.0);

        let header = Rect::from_min_max(Pos2::new(10.0, 80.0), Pos2::new(210.0, 110.0));
        for pointer in [Pos2::new(20.0, 85.0), Pos2::new(100.0, 100.0), Pos2::new(200.0, 109.0)] {
            let hit = group_header_drop_hit(header, pointer, NodeId::from("group"), 0, 0);
            assert_eq!(hit.region, PointerRegion::GroupBody);
            assert_eq!(hit.container, Some(ContainerId::Group(NodeId::from("group"))));
            assert_eq!(hit.indicator_rect, header);
        }
    }

    #[test]
    fn card_edge_indicators_cover_both_halves_in_grid_and_list_layouts() {
        let card = Rect::from_min_max(Pos2::new(10.0, 30.0), Pos2::new(110.0, 70.0));
        for grid in [false, true] {
            let (leading_pointer, trailing_pointer) = if grid {
                (Pos2::new(11.0, 50.0), Pos2::new(109.0, 50.0))
            } else {
                (Pos2::new(50.0, 31.0), Pos2::new(50.0, 69.0))
            };
            let leading = card_drop_hit(card, leading_pointer, grid, NodeId::from("card"), 2, 4);
            let trailing = card_drop_hit(card, trailing_pointer, grid, NodeId::from("card"), 2, 4);
            let halves = card_drop_hits(card, grid, NodeId::from("card"), 2, 4);

            assert_eq!(leading.region, PointerRegion::LeadingEdge);
            assert_eq!(trailing.region, PointerRegion::TrailingEdge);
            assert_eq!(leading.indicator_rect, halves[0].indicator_rect);
            assert_eq!(trailing.indicator_rect, halves[1].indicator_rect);
            assert_eq!(leading.hit_rect, card);
            assert_eq!(trailing.hit_rect, card);
            assert_eq!(leading.indicator_rect.width(), if grid { 4.0 } else { card.width() });
            assert_eq!(leading.indicator_rect.height(), if grid { card.height() } else { 4.0 });
            assert_eq!(trailing.indicator_rect.width(), if grid { 4.0 } else { card.width() });
            assert_eq!(trailing.indicator_rect.height(), if grid { card.height() } else { 4.0 });
            assert_eq!(halves[0].hit_rect.union(halves[1].hit_rect), card);
            assert_eq!(halves[0].order, 4);
            assert_eq!(halves[1].order, 5);
        }
    }

    #[test]
    fn group_gap_halves_use_the_adjacent_elements_widths() {
        let gap = Rect::from_min_max(Pos2::new(10.0, 100.0), Pos2::new(210.0, 106.0));
        let upper_bounds = Rect::from_min_max(Pos2::new(31.0, 40.0), Pos2::new(131.0, 80.0));
        let lower_bounds = Rect::from_min_max(Pos2::new(31.0, 106.0), Pos2::new(231.0, 135.0));
        let split = group_gap_drop_hits(
            gap,
            NodeId::from("upper"),
            NodeId::from("lower"),
            upper_bounds,
            lower_bounds,
            0,
            1,
            4,
        );
        assert_eq!(split.len(), 2);
        assert_eq!(split[0].region, PointerRegion::TrailingEdge);
        assert_eq!(split[0].target, Some(NodeId::from("upper")));
        assert_eq!(split[0].container, None);
        assert!(split[0].hit_rect.is_positive());
        assert_eq!(split[0].hit_rect.min, gap.min);
        assert_eq!(split[0].hit_rect.max, Pos2::new(gap.right(), gap.center().y));
        assert_eq!(split[0].hit_rect.height(), 3.0);
        assert_eq!(split[1].region, PointerRegion::LeadingEdge);
        assert_eq!(split[1].target, Some(NodeId::from("lower")));
        assert_eq!(split[1].hit_rect.height(), 3.0);
        assert_eq!(split[0].indicator_rect, Rect::from_min_max(Pos2::new(31.0, 100.0), Pos2::new(131.0, 103.0)));
        assert_eq!(split[1].indicator_rect, Rect::from_min_max(Pos2::new(31.0, 103.0), Pos2::new(231.0, 106.0)));

        for (upper_depth, lower_depth) in [(0, 1), (0, 0), (1, 0)] {
            let split = group_gap_drop_hits(
                gap,
                NodeId::from("upper"),
                NodeId::from("lower"),
                upper_bounds,
                lower_bounds,
                upper_depth,
                lower_depth,
                4,
            );
            assert_eq!(split.len(), 2);
            assert_eq!(split[0].region, PointerRegion::TrailingEdge);
            assert_eq!(split[0].target, Some(NodeId::from("upper")));
            assert_eq!(split[0].container, None);
            assert_eq!(split[1].region, PointerRegion::LeadingEdge);
            assert_eq!(split[1].target, Some(NodeId::from("lower")));
        }

        // Nested groups use the same child-level append anchor.
        let nested = group_gap_drop_hits(
            gap,
            NodeId::from("upper-folder"),
            NodeId::from("child-folder"),
            upper_bounds,
            lower_bounds,
            1,
            2,
            4,
        );
        assert_eq!(nested[0].region, PointerRegion::TrailingEdge);
        assert_eq!(nested[0].target, Some(NodeId::from("upper-folder")));
        assert_eq!(nested[0].container, None);
    }

    #[test]
    fn six_pixel_append_gap_resolves_two_exact_three_pixel_zones() {
        for left in [0.0, 13.0, 127.0] {
            for top in [0.0, 41.0, 900.0] {
                for width in [1.0, 64.0, 200.0, 1024.0] {
                    let gap = Rect::from_min_size(Pos2::new(left, top), egui::vec2(width, 6.0));
                    let upper_bounds = Rect::from_min_size(Pos2::new(left + 3.0, top - 40.0), egui::vec2(width * 0.4, 30.0));
                    let lower_bounds = Rect::from_min_size(Pos2::new(left + 3.0, top + 6.0), egui::vec2(width * 0.8, 29.0));
                    let hits = group_gap_drop_hits(gap, NodeId::from("upper"), NodeId::from("lower"), upper_bounds, lower_bounds, 0, 1, 8);
                    assert_eq!(hits.len(), 2);
                    assert_eq!(hits[0].hit_rect, Rect::from_min_max(gap.min, Pos2::new(gap.right(), top + 3.0)));
                    assert_eq!(hits[1].hit_rect, Rect::from_min_max(Pos2::new(left, top + 3.0), gap.max));
                    assert_eq!(hits[0].hit_rect.height(), 3.0);
                    assert_eq!(hits[1].hit_rect.height(), 3.0);
                    assert_eq!(hits[0].indicator_rect, Rect::from_min_max(Pos2::new(left + 3.0, top), Pos2::new(left + 3.0 + width * 0.4, top + 3.0)));
                    assert_eq!(hits[1].indicator_rect, Rect::from_min_max(Pos2::new(left + 3.0, top + 3.0), Pos2::new(left + 3.0 + width * 0.8, top + 6.0)));
                    assert_eq!(hits[0].depth, 0);
                    assert_eq!(hits[1].depth, 1);
                    assert_eq!(hits[0].order + 1, hits[1].order);

                    let hierarchy = PickerBrowserHierarchy::new(vec![
                        HierarchyNode::group("upper", vec![HierarchyNode::item("upper-card")]),
                        HierarchyNode::group("lower", vec![HierarchyNode::item("lower-card")]),
                        HierarchyNode::item("moving-card"),
                    ])
                    .unwrap();
                    let upper = resolve_drop_candidate(&NodeId::from("moving-card"), &hits[0], &hierarchy).unwrap();
                    let lower = resolve_drop_candidate(&NodeId::from("moving-card"), &hits[1], &hierarchy).unwrap();
                    assert_eq!(upper.position, DropPosition::After { target: NodeId::from("upper") });
                    assert_eq!(lower.position, DropPosition::Before { target: NodeId::from("lower") });
                    assert_eq!(upper.indicator, DropIndicator::InsertionLine { rect: hits[0].indicator_rect });
                    assert_eq!(lower.indicator, DropIndicator::InsertionLine { rect: hits[1].indicator_rect });
                    assert!(upper.is_valid());
                    assert!(lower.is_valid());
                }
            }
        }
    }

    #[test]
    fn group_gap_anchor_prefers_last_card_in_upper_group_and_falls_back_to_group() {
        use crate::pickers::picker_browser_model::{HierarchyNode as Node, PickerBrowserHierarchy};
        let hierarchy = PickerBrowserHierarchy::new(vec![
            Node::group("outer", vec![Node::group("inner", vec![Node::group("child", vec![]), Node::item("first"), Node::item("last")])]),
        ]).unwrap();
        assert_eq!(group_gap_upper_anchor(&hierarchy, &NodeId::from("inner")), (NodeId::from("last"), 2));
        let empty = PickerBrowserHierarchy::new(vec![Node::group("outer", vec![Node::group("empty", vec![])])]).unwrap();
        assert_eq!(group_gap_upper_anchor(&empty, &NodeId::from("empty")), (NodeId::from("empty"), 1));
    }

    #[test]
    fn group_gap_upper_anchor_uses_the_last_direct_card_at_each_nesting_level() {
        use crate::pickers::picker_browser_model::HierarchyNode as Node;

        let hierarchy = PickerBrowserHierarchy::new(vec![
            Node::group(
                "root",
                vec![
                    Node::item("root-first"),
                    Node::group("nested", vec![Node::item("nested-first"), Node::group("deep", vec![]), Node::item("nested-last")]),
                    Node::item("root-last"),
                ],
            ),
        ])
        .unwrap();

        assert_eq!(group_gap_upper_anchor(&hierarchy, &NodeId::from("root")), (NodeId::from("root-last"), 1));
        assert_eq!(group_gap_upper_anchor(&hierarchy, &NodeId::from("nested")), (NodeId::from("nested-last"), 2));
        assert_eq!(group_gap_upper_anchor(&hierarchy, &NodeId::from("deep")), (NodeId::from("deep"), 2));
    }

    #[test]
    fn group_body_and_root_end_hits_name_their_destination_container() {
        let rect = Rect::from_min_max(Pos2::new(10.0, 10.0), Pos2::new(30.0, 80.0));
        let group = group_append_hit(rect, rect, NodeId::from("group"), 2, 4);
        assert_eq!(group.region, PointerRegion::ContainerEnd);
        assert_eq!(group.container, Some(ContainerId::Group(NodeId::from("group"))));
        let root = root_append_hit(rect, rect, 5);
        assert_eq!(root.region, PointerRegion::ContainerEnd);
        assert_eq!(root.container, Some(ContainerId::Root));
    }
}

/// Resolve a hierarchy-header drop without coupling to a consumer's command type.
pub(crate) fn hierarchy_group_drop(src_group: &str, src_path: &[String], target_group: &str, target_path: &[String], edge: bool) -> Option<HierarchyDrop> {
    if src_group == target_group && target_path.starts_with(src_path) {
        return None;
    }
    let group = src_group.to_string();
    let group_path = src_path.to_vec();
    if !edge {
        let parent = src_path.split_last().map(|(_, p)| p);
        if src_group == target_group && parent == Some(target_path) {
            return None;
        }
        return Some(HierarchyDrop::MoveInto { group, group_path, into: Some((target_group.to_string(), target_path.to_vec())), before: None });
    }
    match target_path.split_last() {
        None if src_path.is_empty() => Some(HierarchyDrop::MoveGroup { group, group_path, before: Some(target_group.to_string()) }),
        None => Some(HierarchyDrop::MoveInto { group, group_path, into: None, before: Some(target_group.to_string()) }),
        Some((leaf, parent)) => {
            let siblings = src_group == target_group && src_path.split_last().map(|(_, p)| p) == Some(parent);
            if siblings {
                Some(HierarchyDrop::MoveGroup { group, group_path, before: Some(leaf.clone()) })
            } else {
                Some(HierarchyDrop::MoveInto { group, group_path, into: Some((target_group.to_string(), parent.to_vec())), before: Some(leaf.clone()) })
            }
        }
    }
}

/// Build the shared payload for a hierarchy header.
pub(crate) fn asset_browser_header_payload(scope: &str, group: &str, group_path: &[String]) -> AssetDrag {
    AssetDrag::Group { scope: scope.to_string(), name: group.to_string(), path: group_path.to_vec() }
}

/// Register a draggable group source without installing any legacy target/release consumer.
pub(crate) fn asset_browser_header_drag_source(response: &egui::Response, scope: &str, group: &str, group_path: &[String]) {
    response.dnd_set_drag_payload(asset_browser_header_payload(scope, group, group_path));
}

/// Register a canonical hierarchy node as a draggable header using its exact typed identity.
pub(crate) fn asset_browser_header_drag_source_with_id(response: &egui::Response, scope: &str, node_id: NodeId) {
    response.dnd_set_drag_payload(AssetDrag::CanonicalGroup { scope: scope.to_owned(), node_id });
}

/// Register a card source with its exact canonical identity (keys may repeat across groups).
pub(crate) fn asset_browser_item_drag_source_with_id(response: &egui::Response, scope: &str, key: &str, node_id: NodeId) {
    response.dnd_set_drag_payload(AssetDrag::Preset { scope: scope.to_string(), key: key.to_string(), node_id: Some(node_id) });
}

/// Take the browser's one-shot release payload after a canonical hit has won arbitration.
/// Call at most once per frame; legacy response helpers retain their existing ownership outside
/// the canonical resolver.
pub(crate) fn asset_browser_take_release(ctx: &egui::Context) -> Option<AssetDrag> {
    if !ctx.input(|input| input.pointer.any_released()) {
        return None;
    }
    egui::DragAndDrop::take_payload::<AssetDrag>(ctx).map(|payload| payload.as_ref().clone())
}

/// Shared drag target for group headers, including scoped drop indicators.
pub(crate) fn asset_browser_header_target_with_capabilities(
    ui: &egui::Ui,
    response: &egui::Response,
    rect: Rect,
    scope: &str,
    group: &str,
    group_path: &[String],
    capabilities: HeaderDropCapabilities,
) -> Option<AssetDrag> {
    let HeaderDropCapabilities { item_drop, group_drag, reject_group_drop } = capabilities;
    if !item_drop && !group_drag {
        return None;
    }
    if group_drag {
        response.dnd_set_drag_payload(asset_browser_header_payload(scope, group, group_path));
    }
    let hovered = response.dnd_hover_payload::<AssetDrag>().map(|payload| payload.as_ref().clone());
    let pointer = ui.ctx().pointer_interact_pos();
    let edge = pointer.is_some_and(|p| header_drop_before(rect, p));
    let source = match hovered.as_ref() {
        Some(AssetDrag::Group { scope: drag_scope, name, path }) if group_drag && drag_scope == scope => Some((name.as_str(), path.as_slice())),
        _ => None,
    };
    let hierarchy_drop = (!reject_group_drop)
        .then_some(())
        .and_then(|()| source)
        .and_then(|(source_group, source_path)| hierarchy_group_drop(source_group, source_path, group, group_path, edge));
    if let Some(dragged) = hovered.as_ref() {
        match dragged {
            AssetDrag::Preset { scope: drag_scope, .. } if item_drop && drag_scope == scope => {
                let t = Tokens::get(ui.ctx());
                paint_group_drop_outline(ui, rect, t.accent);
            }
            AssetDrag::Group { .. } if group_drag && hierarchy_drop.is_some() => {
                if matches!(hierarchy_drop.as_ref(), Some(HierarchyDrop::MoveInto { before: None, .. })) {
                    let t = Tokens::get(ui.ctx());
                    paint_group_drop_outline(ui, rect, t.accent);
                } else {
                    let t = Tokens::get(ui.ctx());
                    // Header insertion lines span the complete header width. The generic card
                    // marker deliberately leaves a four-pixel end-cap inset, which reads as a
                    // rectangular notch on a full-width hierarchy row. Paint on the foreground
                    // layer as well, so header glyphs and later row content cannot cover it.
                    let layer = egui::LayerId::new(egui::Order::Foreground, ui.layer_id().id);
                    let painter = ui.ctx().layer_painter(layer).with_clip_rect(ui.clip_rect());
                    let y = rect.top();
                    painter.rect_filled(
                        Rect::from_min_max(Pos2::new(rect.left(), y - 1.0), Pos2::new(rect.right(), y + 1.0)),
                        egui::CornerRadius::ZERO,
                        t.accent,
                    );
                    painter.rect_filled(
                        Rect::from_min_max(Pos2::new(rect.left(), rect.top()), Pos2::new(rect.left() + 2.0, rect.bottom())),
                        egui::CornerRadius::ZERO,
                        t.accent,
                    );
                }
            }
            _ => {}
        }
    }
    let accepts_drop = match hovered.as_ref() {
        Some(AssetDrag::Preset { scope: drag_scope, .. }) => item_drop && drag_scope == scope,
        Some(AssetDrag::Group { scope: drag_scope, .. }) if group_drag && drag_scope == scope && !reject_group_drop => hierarchy_drop.is_some(),
        _ => false,
    };
    if accepts_drop { response.dnd_release_payload::<AssetDrag>().map(|payload| payload.as_ref().clone()) } else { None }
}

/// Shared item hit target and insertion indicator.
pub(crate) fn asset_browser_item_target(
    ui: &egui::Ui,
    response: &egui::Response,
    rect: Rect,
    next_rect: Option<Rect>,
    insertion_markers: &mut Vec<(Rect, bool, bool)>,
    scope: &str,
    key: &str,
    grid: bool,
) -> Option<AssetDrag> {
    response.dnd_set_drag_payload(AssetDrag::Preset { scope: scope.to_string(), key: key.to_string(), node_id: None });
    let hovered = response.dnd_hover_payload::<AssetDrag>().map(|payload| payload.as_ref().clone());
    if matches!(&hovered, Some(AssetDrag::Preset { scope: drag_scope, key: dragged, .. }) if drag_scope.as_str() == scope && dragged.as_str() != key) {
        let after = ui.ctx().pointer_interact_pos().is_some_and(|p| item_drop_after(rect, p, grid));
        // Across a row break, use the edge of the card under the pointer rather than
        // jumping to the other row. This keeps exactly one marker visible at a time.
        if after {
            if let Some(next) = next_rect.filter(|next| !grid || next.top() == rect.top()) {
                insertion_markers.push((next, false, grid));
            } else {
                insertion_markers.push((rect, true, grid));
            }
        } else {
            insertion_markers.push((rect, false, grid));
        }
    }
    response
        .dnd_release_payload::<AssetDrag>()
        .map(|payload| payload.as_ref().clone())
        .filter(|drag| matches!(drag, AssetDrag::Preset { scope: drag_scope, key: dragged, .. } if drag_scope == scope && dragged != key))
}

/// A group dragged onto an item is inserted immediately before or after that card in its
/// containing group. The returned boolean uses the same edge decision as the visible card line.
pub(crate) fn asset_browser_item_hierarchy_target(
    ui: &egui::Ui,
    response: &egui::Response,
    rect: Rect,
    grid: bool,
    accepts: bool,
) -> Option<(AssetDrag, bool)> {
    if !accepts {
        return None;
    }
    let hovered = response.dnd_hover_payload::<AssetDrag>().map(|payload| payload.as_ref().clone());
    let hierarchy_drag = matches!(hovered, Some(AssetDrag::Group { .. }));
    if hierarchy_drag {
        let after = ui.ctx().pointer_interact_pos().is_some_and(|pointer| item_drop_after(rect, pointer, grid));
        crate::widgets::drop_line(ui, rect, after, grid, &Tokens::get(ui.ctx()));
        return response.dnd_release_payload::<AssetDrag>().map(|payload| payload.as_ref().clone()).map(|drag| (drag, after));
    }
    None
}

/// Common click/double-click semantics for asset rows and cards.
pub(crate) fn asset_browser_item_gesture(response: &egui::Response, selection: bool, activation: bool) -> (bool, bool) {
    let activated = activation && response.double_clicked();
    let selected = selection && !activated && response.clicked();
    (selected, activated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_row_edge_targets_cover_only_the_empty_space_after_the_last_card() {
        let area = Rect::from_min_max(Pos2::new(10.0, 20.0), Pos2::new(310.0, 70.0));
        let first = Rect::from_min_max(Pos2::new(30.0, 25.0), Pos2::new(80.0, 65.0));
        let last = Rect::from_min_max(Pos2::new(210.0, 25.0), Pos2::new(260.0, 65.0));

        let no_right = item_row_edge_targets(area, first, false);
        let right = item_row_edge_targets(area, last, true);

        assert_eq!(no_right, None);
        assert_eq!(right, Some(Rect::from_min_max(Pos2::new(260.0, 25.0), Pos2::new(310.0, 65.0))));
    }
}

#[cfg(test)]
mod canonical_drag_tests {
    use super::AssetDrag;
    use crate::pickers::picker_browser_model::NodeId;

    #[test]
    fn canonical_source_identity_is_scope_checked_and_path_qualified() {
        let item = AssetDrag::Preset { scope: "brushes".to_owned(), key: "soft".to_owned(), node_id: Some(NodeId::group_path(["Favorites"], "soft")) };
        let group = AssetDrag::CanonicalGroup { scope: "brushes".to_owned(), node_id: NodeId::group_path(["Favorites"], "Favorites") };
        let legacy = AssetDrag::Group { scope: "brushes".to_owned(), name: "Favorites".to_owned(), path: Vec::new() };

        assert_eq!(item.canonical_node_id("brushes"), Some(&NodeId::group_path(["Favorites"], "soft")));
        assert_eq!(group.canonical_node_id("brushes"), Some(&NodeId::group_path(["Favorites"], "Favorites")));
        assert_eq!(item.canonical_node_id("patterns"), None);
        assert_eq!(legacy.canonical_node_id("brushes"), None);
    }
}

/// Generic actions returned by shared item and group context menus.
pub(crate) enum ContextMenuAction {
    Rename,
    Delete,
    MoveTo(String),
}

/// Draw a configurable item menu and return a request for the consumer to handle.
pub(crate) fn draw_item_context_menu(
    ui: &mut egui::Ui,
    rename_label: &str,
    delete_label: &str,
    move_to: Option<(&str, &[String], &str)>,
) -> Option<ContextMenuAction> {
    if ui.button(rename_label).clicked() {
        ui.close();
        return Some(ContextMenuAction::Rename);
    }
    if ui.button(delete_label).clicked() {
        ui.close();
        return Some(ContextMenuAction::Delete);
    }
    if let Some((label, group_names, current_group)) = move_to {
        let mut action = None;
        ui.menu_button(label, |ui| {
            for group in group_names.iter().filter(|name| name.as_str() != current_group) {
                if ui.button(group).clicked() {
                    action = Some(ContextMenuAction::MoveTo(group.clone()));
                    ui.close();
                }
            }
        });
        return action;
    }
    None
}

/// Draw a group's rename/delete menu.
pub(crate) fn draw_group_context_menu(ui: &mut egui::Ui, rename_label: &str, delete_label: &str) -> Option<ContextMenuAction> {
    if ui.button(rename_label).clicked() {
        ui.close();
        Some(ContextMenuAction::Rename)
    } else if ui.button(delete_label).clicked() {
        ui.close();
        Some(ContextMenuAction::Delete)
    } else {
        None
    }
}

/// Attach a consumer-provided context menu to an egui response.
pub(crate) fn context_menu_target(resp: egui::Response, menu: impl FnOnce(&mut egui::Ui)) {
    let _ = resp.context_menu(menu);
}

#[cfg(test)]
mod hierarchy_group_drop_tests {
    use super::{HierarchyDrop, hierarchy_group_drop};

    fn path(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| (*part).to_string()).collect()
    }

    #[test]
    fn rejects_dropping_a_group_on_itself_or_a_descendant() {
        let source = path(&["Brushes", "Favorites"]);

        assert_eq!(hierarchy_group_drop("Library", &source, "Library", &source, false), None);
        assert_eq!(hierarchy_group_drop("Library", &source, "Library", &path(&["Brushes", "Favorites", "Soft"]), true), None);
    }

    #[test]
    fn rejects_dropping_a_group_into_its_current_parent() {
        assert_eq!(hierarchy_group_drop("Library", &path(&["Brushes", "Favorites"]), "Library", &path(&["Brushes"]), false), None);
    }

    #[test]
    fn allows_reordering_a_group_before_a_sibling() {
        assert_eq!(
            hierarchy_group_drop("Library", &path(&["Brushes", "Favorites"]), "Library", &path(&["Brushes", "Recent"]), true),
            Some(HierarchyDrop::MoveGroup { group: "Library".to_string(), group_path: path(&["Brushes", "Favorites"]), before: Some("Recent".to_string()) }),
        );
    }

    #[test]
    fn allows_moving_a_group_into_another_group() {
        assert_eq!(
            hierarchy_group_drop("Library", &path(&["Brushes", "Favorites"]), "Library", &path(&["Textures"]), false),
            Some(HierarchyDrop::MoveInto {
                group: "Library".to_string(),
                group_path: path(&["Brushes", "Favorites"]),
                into: Some(("Library".to_string(), path(&["Textures"]))),
                before: None,
            }),
        );
    }

    #[test]
    fn allows_moving_a_group_into_another_root_groups_nested_group() {
        assert_eq!(
            hierarchy_group_drop("Library", &path(&["Brushes", "Favorites"]), "Other", &path(&["Textures"]), false),
            Some(HierarchyDrop::MoveInto {
                group: "Library".to_string(),
                group_path: path(&["Brushes", "Favorites"]),
                into: Some(("Other".to_string(), path(&["Textures"]))),
                before: None,
            }),
        );
    }
}
