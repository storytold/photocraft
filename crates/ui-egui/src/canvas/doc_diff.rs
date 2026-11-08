//! Where two versions of a document composite differently, worked out by comparing them: the
//! canvas's fallback when no tool or preview says what it changed. Without it such a change
//! recomposited the whole document in one frame, over a second on a gigapixel canvas, long
//! enough for a Wayland compositor to drop the window.
//!
//! Conservative by design: an area is claimed only when every difference found is one whose
//! reach is known (a pixel layer's changed tiles, or a layer's properties, bounded by
//! [`photocraft_compose::change_bounds`]). Anything else (layers added, removed or reordered, a
//! change outside the layers, an adjustment layer, no difference found at all) is `None`, and the
//! canvas redraws everything as before. Pixel layers share unchanged tiles between versions, so
//! comparing them is a pointer comparison per tile, never a pixel scan.

use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_geom::Rect;
use photocraft_raster::Surface;

/// The canvas area where `new` can composite differently from `old`, or `None` when that can't be
/// bounded (or nothing differs that this can see): the caller redraws everything then. The caller
/// grows the area by the reach of layer effects, as for any damage.
pub(crate) fn changed_area(old: &Document, new: &Document) -> Option<Rect> {
    if std::ptr::eq(old, new) || old.size != new.size {
        return None;
    }
    // Everything but the layers (and the selection, drawn over the canvas, not in it) must be
    // equal: a change elsewhere (a channel, the mode, a profile) isn't one this can bound. The
    // whole struct is compared, so a field added later counts too.
    if !same_outside_layers(old, new) {
        return None;
    }
    let canvas = old.bounds();
    let mut area = Changed::Nothing;
    diff_list(&old.layers, &new.layers, canvas, &mut area);
    match area {
        // Not clipped to the canvas yet: the caller widens it by the effects' reach first, then
        // the refresh clips it, so effects of pixels just past the edge are redrawn too.
        Changed::Area(r) => Some(r),
        // No visible difference here is still a change the caller saw: something this doesn't
        // look at (a display setting, a view state) may have changed, so redraw everything.
        Changed::Nothing | Changed::Unknown => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Changed {
    Nothing,
    Area(Rect),
    Unknown,
}

impl Changed {
    fn add(&mut self, r: Option<Rect>) {
        *self = match (*self, r) {
            (Changed::Unknown, _) | (_, None) => Changed::Unknown,
            (_, Some(r)) if r.is_empty() => *self,
            (Changed::Nothing, Some(r)) => Changed::Area(r),
            (Changed::Area(a), Some(r)) => Changed::Area(a.union(&r)),
        };
    }
}

fn same_outside_layers(old: &Document, new: &Document) -> bool {
    let strip = |d: &Document| Document { layers: Vec::new(), selection: None, ..d.clone() };
    strip(old) == strip(new)
}

/// Compare two sibling lists layer by layer. The same layers in the same order, or `Unknown`.
fn diff_list(old: &[Layer], new: &[Layer], canvas: Rect, area: &mut Changed) {
    if old.len() != new.len() || old.iter().zip(new).any(|(a, b)| a.id != b.id) {
        area.add(None);
        return;
    }
    for (a, b) in old.iter().zip(new) {
        if *area == Changed::Unknown {
            return;
        }
        diff_layer(a, b, canvas, area);
    }
}

fn diff_layer(a: &Layer, b: &Layer, canvas: Rect, area: &mut Changed) {
    if same_props(a, b) {
        match (&a.content, &b.content) {
            // Only pixels changed: where their tiles differ. Blending, clipping and effects act
            // pixel by pixel (effects spread further, which the caller's effect reach covers).
            (LayerContent::Raster(sa), LayerContent::Raster(sb)) => area.add(tile_diff(sa, sb)),
            // Only children changed: where they did.
            (LayerContent::Group(ga), LayerContent::Group(gb)) => diff_list(&ga.children, &gb.children, canvas, area),
            // `same_props` compares every other kind of content in full: nothing differs.
            _ => {}
        }
        return;
    }
    // Shown, hidden, restyled, re-masked, moved, or content other than pixels: the layer's whole
    // reach, before and after. `change_bounds` is `None` (unbounded) for adjustment layers and
    // groups holding one that reaches below.
    area.add(photocraft_compose::change_bounds(a, canvas));
    area.add(photocraft_compose::change_bounds(b, canvas));
}

/// The layers equal except for a pixel layer's own pixels and a group's children.
fn same_props(a: &Layer, b: &Layer) -> bool {
    fn shell(l: &Layer) -> Layer {
        let mut l = l.clone();
        match &mut l.content {
            LayerContent::Raster(s) => *s = Surface::new(s.format()),
            LayerContent::Group(g) => g.children.clear(),
            _ => {}
        }
        l
    }
    shell(a) == shell(b)
}

/// Where two pixel surfaces differ, by tile identity: a tile both share unchanged is the same
/// allocation. `None` when they can't be compared that way (another format or default pixel).
fn tile_diff(a: &Surface, b: &Surface) -> Option<Rect> {
    if a.format() != b.format() || a.default_pixel() != b.default_pixel() {
        return None;
    }
    let mut r = Rect::EMPTY;
    // Tiles past the canvas edge count too: an effect (a shadow, a stroke) of pixels there can
    // reach onto the canvas once the caller widens the area by the effects' reach; the refresh
    // clips the result to the canvas.
    let mut add = |t: Rect| {
        if !t.is_empty() {
            r = if r.is_empty() { t } else { r.union(&t) };
        }
    };
    for (c, ta) in a.tiles() {
        if b.tile(*c).is_none_or(|tb| !std::sync::Arc::ptr_eq(ta, tb)) {
            add(c.rect());
        }
    }
    for (c, _) in b.tiles() {
        if a.tile(*c).is_none() {
            add(c.rect());
        }
    }
    Some(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_engine::Session;
    use serde_json::json;

    fn doc_with_two_layers() -> (Session, photocraft_doc::LayerId, photocraft_doc::LayerId) {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 2000, "height": 1000, "background": "transparent"})).unwrap();
        let a = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        let b = s.active().unwrap().active_layer.unwrap();
        (s, a, b)
    }

    fn paint(s: &mut Session, id: photocraft_doc::LayerId, r: Rect) {
        s.edit("paint", |doc, _| {
            doc.layer_mut(id).unwrap().surface_mut().unwrap().fill_rect(r, &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn painting_claims_only_the_changed_tiles() {
        let (mut s, a, _) = doc_with_two_layers();
        paint(&mut s, a, Rect::new(10, 10, 50, 50));
        let before = s.active().unwrap().doc.clone();
        paint(&mut s, a, Rect::new(1500, 600, 1520, 620));
        let after = s.active().unwrap().doc.clone();
        let r = changed_area(&before, &after).expect("a pixel change is bounded");
        assert!(r.contains(1500, 600) && r.contains(1519, 619), "{r:?}");
        assert!(!r.contains(20, 20), "the untouched tile isn't claimed: {r:?}");
    }

    #[test]
    fn hiding_a_layer_claims_its_bounds() {
        let (mut s, _, b) = doc_with_two_layers();
        paint(&mut s, b, Rect::new(100, 100, 300, 200));
        let before = s.active().unwrap().doc.clone();
        s.execute("layer.hideLayers", json!({"layer": b.0})).unwrap();
        let after = s.active().unwrap().doc.clone();
        let r = changed_area(&before, &after).expect("hiding a pixel layer is bounded");
        assert!(r.contains(100, 100) && r.contains(299, 199), "{r:?}");
        assert!(r.width() < 2000, "not the whole canvas: {r:?}");
    }

    #[test]
    fn anything_it_cannot_bound_redraws_everything() {
        let (mut s, a, _) = doc_with_two_layers();
        let before = s.active().unwrap().doc.clone();
        // A new layer: the structure changed.
        s.execute("layer.new.layer", json!({})).unwrap();
        assert_eq!(changed_area(&before, &s.active().unwrap().doc.clone()), None);
        // Nothing differs at all: the caller saw a change this can't see.
        assert_eq!(changed_area(&before, &before.as_ref().clone()), None);
        // An adjustment layer reaches everything beneath it: changing one isn't bounded, even
        // alongside a pixel change that is.
        s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
        let adjustment = s.active().unwrap().active_layer.unwrap();
        let before = s.active().unwrap().doc.clone();
        s.edit("hide", |doc, _| {
            doc.layer_mut(adjustment).unwrap().visible = false;
            Ok(())
        })
        .unwrap();
        paint(&mut s, a, Rect::new(10, 10, 50, 50));
        assert_eq!(changed_area(&before, &s.active().unwrap().doc.clone()), None);
    }
}
