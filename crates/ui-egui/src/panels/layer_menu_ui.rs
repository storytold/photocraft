//! The Layers panel's right-click menu entries in Photoshop's order. Items are command ids from the
//! menu catalog; the panel greys unavailable ones with the main menus' enablement and runs them as
//! menu invocations, so dialogs open like in the menu bar.

use photocraft_doc::{LabelColor, Layer, LayerContent};

/// One entry: label, command id. `None` = separator.
pub type Entry = Option<(&'static str, &'static str)>;

/// English action key; each menu translates it using its existing display context.
pub(crate) fn mask_toggle_label(enabled: bool) -> &'static str {
    if enabled { "Disable Layer Mask" } else { "Enable Layer Mask" }
}

/// The command behind "Add Layer Mask" (the panel button and this menu). Like Photoshop, an active
/// selection becomes the mask; ⌥ inverts it (Hide Selection, or Hide All without a selection).
pub fn add_mask_command(has_selection: bool, alt: bool) -> &'static str {
    match (has_selection, alt) {
        (true, false) => "layer.layerMask.revealSelection",
        (true, true) => "layer.layerMask.hideSelection",
        (false, false) => "layer.layerMask.revealAll",
        (false, true) => "layer.layerMask.hideAll",
    }
}

/// What the Layers panel "Add a mask" button does for `layer` (#2075). Like Photoshop it never
/// replaces a mask: a layer without a layer mask gets one ([`add_mask_command`]); a layer that
/// already has one gets a vector mask instead (⌥: Hide All); a layer with both (or a shape layer,
/// whose path is already its vector mask, with a layer mask) gets nothing (`None`).
pub fn mask_button_command(layer: Option<&Layer>, has_selection: bool, alt: bool) -> Option<&'static str> {
    let l = layer?;
    if l.mask.is_none() {
        Some(add_mask_command(has_selection, alt))
    } else if l.vector_mask.is_none() && !matches!(l.content, LayerContent::Shape(_)) {
        Some(if alt { "layer.vectorMask.hideAll" } else { "layer.vectorMask.revealAll" })
    } else {
        None
    }
}

/// Commands that act on the active layer only and have no sensible meaning for a selection: Blending
/// Options (one dialog), Copy Layer Style (copy from which layer?) and the per-layer mask items.
/// With several layers selected they would hit whichever layer is active, not the one that was
/// right-clicked, so the menu greys them. Adding a mask, Paste/Clear Layer Style, Rasterize and
/// Export As (one file per layer, #3629) run on every selected layer instead.
pub fn single_layer_only(id: &str) -> bool {
    let adds_mask = matches!(id, "layer.layerMask.revealAll" | "layer.layerMask.hideAll" | "layer.layerMask.revealSelection" | "layer.layerMask.hideSelection");
    matches!(id, "layer.layerStyle.blendingOptions" | "layer.layerStyle.copyLayerStyle") || (id.starts_with("layer.layerMask.") && !adds_mask)
}

/// The context menu entries for a layer (Photoshop 2026 order, trimmed to the layer kind).
pub fn entries(l: &Layer, multi: bool, has_selection: bool) -> Vec<Entry> {
    let mut v: Vec<Entry> = vec![Some((tl!("Blending Options…"), "layer.layerStyle.blendingOptions"))];
    v.push(None);
    v.push(Some((if multi { tl!("Duplicate Layers…") } else { tl!("Duplicate Layer…") }, "layer.duplicate")));
    v.push(Some((if multi { tl!("Delete Layers") } else { tl!("Delete Layer") }, "layer.delete")));
    // Groups the clicked layer, or the whole selection when several layers are selected, after
    // Photoshop's New Group dialog.
    v.push(Some((tl!("Group from Layers…"), "layer.new.groupFromLayers")));
    v.push(None);
    v.push(Some((tl!("Quick Export as PNG"), "layer.quickExportAsPng")));
    v.push(Some((tl!("Export As…"), "layer.exportAs")));
    v.push(None);
    v.push(Some((tl!("Artboard from Layers…"), "layer.new.artboardFromLayers")));
    v.push(Some((tl!("Frame from Layers…"), "layer.new.frameFromLayers")));
    if !multi && matches!(l.content, LayerContent::Shape(_) | LayerContent::Text(_)) {
        v.push(Some((tl!("Convert to Frame…"), "layer.frame.convert")));
    }
    v.push(None);
    v.push(Some((tl!("Convert to Smart Object"), "layer.smartObjects.convertToSmartObject")));
    if multi {
        // One item for the whole selection, whatever the clicked layer is: Rasterize acts on every
        // selected layer. Edit Contents and Convert to Layers work on one layer, so they are not
        // offered here.
        v.push(Some(("Rasterize Layers", "layer.rasterize.layer")));
    } else {
        match &l.content {
            LayerContent::Text(_) => {
                v.push(Some(("Rasterize Type", "layer.rasterize.type")));
                v.push(Some((tl!("Create Work Path"), "type.createWorkPath")));
                v.push(Some((tl!("Convert to Shape"), "type.convertToShape")));
            }
            LayerContent::Shape(_) => v.push(Some(("Rasterize Layer", "layer.rasterize.shape"))),
            LayerContent::Smart(_) => {
                v.push(Some((tl!("Edit Contents"), "layer.smartObjects.editContents")));
                v.push(Some((tl!("Reset Transform"), "layer.smartObjects.resetTransform")));
                v.push(Some((tl!("Replace Contents…"), "layer.smartObjects.replaceContents")));
                v.push(Some((tl!("Relink to File…"), "layer.smartObjects.relinkToFile")));
                v.push(Some((tl!("Convert to Layers"), "layer.smartObjects.convertToLayers")));
                v.push(Some(("Rasterize Layer", "layer.rasterize.smartObject")));
            }
            LayerContent::Fill(_) => v.push(Some(("Rasterize Layer", "layer.rasterize.fillContent"))),
            _ => v.push(Some(("Rasterize Layer", "layer.rasterize.layer"))),
        }
    }
    // Photoshop offers it under Rasterize Layer for a layer with a style (it bakes the effects).
    if multi || !l.effects.items.is_empty() {
        v.push(Some(("Rasterize Layer Style", "layer.rasterize.layerStyle")));
    }
    v.push(None);
    // A selection has no single mask to toggle, apply or delete, so its menu offers Add Layer Mask only.
    if let Some(mask) = l.mask.as_ref().filter(|_| !multi) {
        v.push(Some((tl!(mask_toggle_label(mask.enabled)), "layer.layerMask.enabled")));
        v.push(Some((tl!("Apply Layer Mask"), "layer.layerMask.apply")));
        v.push(Some((tl!("Delete Layer Mask"), "layer.layerMask.delete")));
    } else {
        v.push(Some((tl!("Add Layer Mask"), add_mask_command(has_selection, false))));
    }
    v.push(Some((
        if l.clipped { tl!("Release Clipping Mask") } else { tl!("Create Clipping Mask") },
        if l.clipped { "layer.releaseClippingMask" } else { "layer.createClippingMask" },
    )));
    v.push(None);
    v.push(Some((tl!("Link Layers"), "layer.linkLayers")));
    v.push(Some((tl!("Select Linked Layers"), "layer.selectLinkedLayers")));
    v.push(None);
    v.push(Some((tl!("Copy Layer Style"), "layer.layerStyle.copyLayerStyle")));
    v.push(Some((tl!("Paste Layer Style"), "layer.layerStyle.pasteLayerStyle")));
    v.push(Some((tl!("Clear Layer Style"), "layer.layerStyle.clear")));
    v.push(None);
    if multi {
        v.push(Some((tl!("Merge Layers"), "layer.mergeLayers")));
    } else if matches!(l.content, LayerContent::Group(_)) {
        // Photoshop's Merge Group: a group merges itself, not into the layer below (#3832).
        v.push(Some((tl!("Merge Group"), "layer.mergeGroup")));
    } else {
        v.push(Some((tl!("Merge Down"), "layer.mergeDown")));
    }
    v.push(Some((tl!("Merge Visible"), "layer.mergeVisible")));
    v.push(Some((tl!("Flatten Image"), "layer.flattenImage")));
    v
}

/// The layer style menu of an effect row ("Effects" or one effect) under a layer (#2846), in
/// Photoshop's order. It acts on that layer, like Layer ▸ Layer Style.
pub fn effects_entries() -> Vec<Entry> {
    vec![
        Some((tl!("Blending Options…"), "layer.layerStyle.blendingOptions")),
        None,
        Some((tl!("Copy Layer Style"), "layer.layerStyle.copyLayerStyle")),
        Some((tl!("Paste Layer Style"), "layer.layerStyle.pasteLayerStyle")),
        Some((tl!("Clear Layer Style"), "layer.layerStyle.clear")),
        None,
        Some((tl!("Global Light…"), "layer.layerStyle.globalLight")),
        Some((tl!("Create Layer"), "layer.layerStyle.createLayer")),
        Some((tl!("Hide All Effects"), "layer.layerStyle.hideAllEffects")),
        Some((tl!("Scale Effects…"), "layer.layerStyle.scaleEffects")),
    ]
}

pub(crate) fn color_name(color: LabelColor) -> &'static str {
    crate::i18n::tr_ctx(crate::i18n::current(), "layerLabel", color.label())
}

#[cfg(test)]
mod tests;
