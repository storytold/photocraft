//! Photoshop menu enablement the engine's per-command checks don't express: the Background layer
//! (no styles, masks, clipping, arranging, transforms without a selection), layer-kind specific
//! items (Rasterize ›, Combine Shapes ›), and single-layer documents (Merge Visible, Flatten Image).
//! Applied on top of `menus::is_enabled`, so menus, shortcuts and the Layers context menu grey the
//! same items Photoshop greys.

use photocraft_doc::{Document, Layer, LayerContent, LayerId};

/// Commands Photoshop greys while the Background layer is the active layer.
fn background_blocks(id: &str, has_selection: bool) -> bool {
    let prefixes = ["layer.layerMask.", "layer.vectorMask.", "layer.arrange.", "layer.matting."];
    if prefixes.iter().any(|p| id.starts_with(p)) {
        return true;
    }
    if id.starts_with("layer.layerStyle.") {
        // Global Light and Hide All Effects are document-wide.
        return !matches!(id, "layer.layerStyle.globalLight" | "layer.layerStyle.hideAllEffects" | "layer.layerStyle.copyLayerStyle");
    }
    // Transforms need a selection on the Background (they then move the selected pixels).
    if !has_selection && (id == "edit.freeTransform" || (id.starts_with("edit.transform.") && id != "edit.transform.again")) {
        return true;
    }
    matches!(
        id,
        "layer.createClippingMask"
            | "layer.releaseClippingMask"
            | "layer.mergeDown"
            | "layer.lockLayers"
            | "layer.maskAllObjects"
            | "edit.contentAwareScale"
            | "edit.puppetWarp"
            | "edit.perspectiveWarp"
    )
}

/// Commands that only apply to some layer kinds.
fn kind_blocks(id: &str, l: &Layer) -> bool {
    let c = &l.content;
    match id {
        "layer.rasterize.type" | "layer.rasterize.rasterizeType" => !matches!(c, LayerContent::Text(_)),
        "layer.rasterize.shape" | "layer.rasterize.rasterizeShape" => !matches!(c, LayerContent::Shape(_)),
        "layer.rasterize.fillContent" => !matches!(c, LayerContent::Fill(_)),
        "layer.rasterize.smartObject" => !matches!(c, LayerContent::Smart(_)),
        "layer.rasterize.vectorMask" => l.vector_mask.is_none(),
        "layer.rasterize.layer" => {
            matches!(c, LayerContent::Raster(_) | LayerContent::Group(_) | LayerContent::Adjustment(_)) && l.vector_mask.is_none() && l.effects.items.is_empty()
        }
        "layer.layerStyle.copyLayerStyle" | "layer.layerStyle.clear" => l.effects.items.is_empty(),
        _ => id.starts_with("layer.combineShapes.") && !matches!(c, LayerContent::Shape(_)),
    }
}

/// Is `id` greyed in Photoshop for this document state, regardless of the engine's own check?
pub fn disabled_for(doc: &Document, active: Option<&Layer>, id: &str) -> bool {
    let only_background = doc.layers.len() == 1 && doc.layers.first().is_some_and(|b| crate::doc_props_ui::is_background(doc, b));
    if only_background && matches!(id, "layer.mergeVisible" | "layer.flattenImage" | "layer.mergeLayers" | "layer.rasterize.allLayers") {
        return true;
    }
    if id == "layer.delete.hiddenLayers" && doc.walk().iter().all(|(_, _, l)| l.visible) {
        return true;
    }
    let Some(l) = active else { return false };
    if crate::doc_props_ui::is_background(doc, l) && background_blocks(id, doc.selection.is_some()) {
        return true;
    }
    kind_blocks(id, l)
}

/// Rasterize commands that act on every selected layer (not just the active one).
const SELECTION_RASTERIZE: [&str; 5] =
    ["layer.rasterize.layer", "layer.rasterize.type", "layer.rasterize.shape", "layer.rasterize.fillContent", "layer.rasterize.smartObject"];

/// [`disabled_for`] with the layer selection in mind: with several layers selected, a Rasterize
/// command is greyed only when none of them can be rasterized by it (the active layer alone must
/// not decide, since the command runs on all of them).
pub fn disabled_for_selection(doc: &Document, selected: &[LayerId], active: Option<&Layer>, id: &str) -> bool {
    if selected.len() > 1 && SELECTION_RASTERIZE.contains(&id) {
        return selected.iter().filter_map(|l| doc.layer(*l)).all(|l| disabled_for(doc, Some(l), id));
    }
    disabled_for(doc, active, id)
}

pub fn disabled(app: &crate::PhotocraftApp, id: &str) -> bool {
    let Some(st) = app.session.active() else { return false };
    let active = st.active_layer.and_then(|a| st.doc.layer(a));
    // Free Transform Path works on the Background too: it moves the path, not the pixels.
    let transform = id == "edit.freeTransform" || (id.starts_with("edit.transform.") && id != "edit.transform.again");
    if transform && crate::vector_ui::free_transform_path(app).is_some() {
        return false;
    }
    // Only the Rasterize commands depend on the selection; skip walking the layers for the rest.
    if SELECTION_RASTERIZE.contains(&id) {
        return disabled_for_selection(&st.doc, &st.selected_layers(), active, id);
    }
    disabled_for(&st.doc, active, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn session() -> photocraft_engine::Session {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 20, "height": 20})).unwrap();
        s
    }

    fn off(s: &photocraft_engine::Session, id: &str) -> bool {
        let st = s.active().unwrap();
        disabled_for(&st.doc, st.active_layer.and_then(|a| st.doc.layer(a)), id)
    }

    #[test]
    fn background_greys_styles_masks_and_transforms() {
        let mut s = session();
        for id in [
            "layer.layerStyle.dropShadow",
            "layer.layerMask.revealAll",
            "layer.arrange.bringForward",
            "layer.mergeDown",
            "edit.freeTransform",
            "edit.transform.scale",
            "layer.flattenImage",
            "layer.mergeVisible",
            "layer.createClippingMask",
            "layer.rasterize.layer",
            "layer.combineShapes.unite",
            "layer.delete.hiddenLayers",
        ] {
            assert!(off(&s, id), "{id} should be greyed on a lone Background");
        }
        assert!(!off(&s, "layer.layerStyle.globalLight"));
        // A selection makes Free Transform available on the Background.
        s.execute("select.all", json!({})).unwrap();
        assert!(!off(&s, "edit.freeTransform"));
    }

    /// With several layers selected, Rasterize is judged by the whole selection, not by the layer
    /// that happens to be active (or was right-clicked).
    #[test]
    fn rasterize_follows_the_whole_selection() {
        let mut s = session();
        s.execute("layer.new.layer", json!({})).unwrap();
        let raster = s.active().unwrap().active_layer.unwrap();
        let text = photocraft_doc::LayerId(s.execute("type.create", json!({"text": "Hi", "size": 12, "x": 2, "y": 12})).unwrap()["layer"].as_u64().unwrap());
        let sel = |s: &photocraft_engine::Session, ids: &[photocraft_doc::LayerId], id: &str| {
            let st = s.active().unwrap();
            disabled_for_selection(&st.doc, ids, ids.last().and_then(|a| st.doc.layer(*a)), id)
        };
        // The raster layer is active, but the type layer in the selection can be rasterized.
        let ids = [text, raster];
        assert!(!sel(&s, &ids, "layer.rasterize.layer"));
        assert!(!sel(&s, &ids, "layer.rasterize.type"));
        assert!(sel(&s, &ids, "layer.rasterize.shape"), "no shape layer is selected");
        assert!(sel(&s, &ids, "layer.rasterize.smartObject"));
        // Nothing selected can be rasterized: greyed, as for a single raster layer.
        let bg = s.active().unwrap().doc.layers[0].id;
        assert!(sel(&s, &[bg, raster], "layer.rasterize.layer"));
        // One selected layer is judged by itself, and other commands are not affected.
        assert!(sel(&s, &[raster], "layer.rasterize.layer") && !sel(&s, &[text], "layer.rasterize.layer"));
        let st = s.active().unwrap();
        let active = ids.last().and_then(|a| st.doc.layer(*a));
        for id in ["layer.layerStyle.dropShadow", "layer.mergeDown", "layer.rasterize.vectorMask"] {
            assert_eq!(sel(&s, &ids, id), disabled_for(&st.doc, active, id), "{id}: decided by the active layer, as before");
        }
    }

    #[test]
    fn ordinary_layers_keep_their_items() {
        let mut s = session();
        s.execute("layer.new.layer", json!({})).unwrap();
        for id in ["layer.layerStyle.dropShadow", "layer.layerMask.revealAll", "layer.mergeDown", "edit.freeTransform", "layer.flattenImage"] {
            assert!(!off(&s, id), "{id}");
        }
        assert!(off(&s, "layer.rasterize.type"));
        assert!(off(&s, "layer.layerStyle.copyLayerStyle"));
    }
}
