//! Layer geometry for snapping (View › Snap To › Layers and the Move tool's box).
//!
//! The snap targets and the snapping itself are the framework's (`SnapTargets`); this module
//! gives the app the layer bounds they are built from.

use photocraft_doc::{Document, LayerContent, LayerId};

/// Bounds of a layer for snapping: what Free Transform would show (`None` for empty layers,
/// groups and maskless adjustment layers).
pub fn layer_rect(doc: &Document, id: LayerId) -> Option<[f64; 4]> {
    rect_of(doc, doc.layer(id)?)
}

fn rect_of(doc: &Document, l: &photocraft_doc::Layer) -> Option<[f64; 4]> {
    snappable(l).then(|| crate::transform_cmds::transform_bounds(doc, l)).and_then(to_rect)
}

/// Bounds of a layer as a Snap To Layers target: all of its content, whatever the selection
/// holds of it (#6290: clipped to the selection, every layer's edges and centre were the
/// selection's, so a moved selection snapped back to where it started). An artboard is a group
/// whose edges and centre are the board's own.
pub fn target_rect_of(doc: &Document, l: &photocraft_doc::Layer) -> Option<[f64; 4]> {
    if let Some(a) = l.artboard() {
        return to_rect(a.rect);
    }
    snappable(l).then(|| crate::transform_cmds::whole_layer_bounds(doc, l)).and_then(to_rect)
}

fn snappable(l: &photocraft_doc::Layer) -> bool {
    !(l.is_group() || (matches!(l.content, LayerContent::Adjustment(_)) && l.mask.is_none()))
}

fn to_rect(b: photocraft_geom::Rect) -> Option<[f64; 4]> {
    (!b.is_empty()).then_some([b.x0 as f64, b.y0 as f64, b.x1 as f64, b.y1 as f64])
}

/// Union of rectangles (`None` when empty).
pub fn union(rects: impl IntoIterator<Item = [f64; 4]>) -> Option<[f64; 4]> {
    rects.into_iter().reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])])
}
