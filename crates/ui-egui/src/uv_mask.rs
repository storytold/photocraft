//! Native channel/mask display projected through each material UV, independent of source maps.
use crate::PhotocraftApp;

/// Whether `material` has any source map bound to the displayed mask document. Mask display may
/// come from Color, Roughness, Normal, or another UV channel; keep this predicate shared by the
/// preview renderer and the paint hit-target gate.
pub(crate) fn has_mask_binding(app: &PhotocraftApp, material: usize, document: photocraft_doc::DocId) -> bool {
    photocraft_uv::preview::Channel::MAPS.iter().any(|kind| app.session.uv.map_bindings(*kind).get(material) == Some(&Some(document)))
}

pub(crate) fn source(app: &PhotocraftApp, material: usize) -> Option<usize> {
    let index = app.session.active_index()?;
    let st = app.session.documents().get(index)?;
    if st.channel_view.is_plain(&st.doc) {
        return None;
    }
    has_mask_binding(app, material, st.doc.id).then_some(index)
}
pub(crate) fn key(app: &PhotocraftApp, index: usize) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    if let Some(st) = app.session.documents().get(index) {
        serde_json::to_string(&st.channel_view).unwrap_or_default().hash(&mut h);
        st.active_layer.hash(&mut h);
    }
    let look = crate::channel_view::Look::of(&app.session);
    look.in_color.hash(&mut h);
    look.mask_rgb.map(f32::to_bits).hash(&mut h);
    look.mask_opacity.to_bits().hash(&mut h);
    crate::uv_mask_paint_preview::sampled_fill_proxy(app, index).map(|proxy| proxy.key).hash(&mut h);
    h.finish()
}
fn blend_overlay(pixels: &mut [[f32; 4]], overlay: Option<Vec<egui::Color32>>) {
    if let Some(px) = overlay.filter(|px| px.len() == pixels.len()) {
        for (out, p) in pixels.iter_mut().zip(px) {
            let rgba = p.to_array().map(|v| v as f32 / 255.0);
            for i in 0..3 {
                out[i] = rgba[i] + out[i] * (1.0 - rgba[3]);
            }
            out[3] = 1.0;
        }
    }
}
pub(crate) fn overlay(
    app: &PhotocraftApp,
    index: usize,
    doc: &photocraft_doc::Document,
    rect: photocraft_geom::Rect,
    output_size: [u32; 2],
    pixels: &mut [[f32; 4]],
) {
    let output_size = [output_size[0].min(doc.size.width), output_size[1].min(doc.size.height)];
    let Some(st) = app.session.documents().get(index) else {
        return;
    };
    let mut view = st.channel_view.clone();
    if view.target == photocraft_engine::channel_cmds::ChannelTarget::LayerMask
        && let Some(layer) = st.active_layer.filter(|id| doc.layer(*id).is_some_and(|layer| layer.mask.is_some()))
    {
        view.layer_mask =
            Some(photocraft_engine::mask_view_cmds::LayerMaskView { layer, filter: false, mode: photocraft_engine::mask_view_cmds::MaskViewMode::Gray });
    }
    let look = crate::channel_view::Look::of(&app.session);
    let live_fill = crate::uv_mask_paint_preview::sampled_fill_proxy(app, index)
        .filter(|proxy| view.layer_mask.is_some_and(|shown| shown.layer == proxy.layer) && view.shown_layer_mask(doc).is_some());
    let px = if let Some(proxy) = live_fill {
        crate::channel_view::render_fill_mask_proxy(doc, &view, rect, output_size, look, &proxy)
    } else {
        crate::channel_view::render_reduced_grid(doc, &view, rect, output_size, look, st.active_layer)
    };
    blend_overlay(pixels, px);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reduced_mask_overlay_matches_full_grid_for_partial_damage() {
        use photocraft_geom::Rect;
        use serde_json::json;
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width":1001,"height":777})).unwrap();
        app.session.execute("layer.layerMask.hideAll", json!({})).unwrap();
        app.session
            .edit("seed mask edge", |doc, active| {
                doc.layer_mut(active.unwrap()).unwrap().mask.as_mut().unwrap().surface.fill_rect(Rect::new(501, 389, 1001, 777), &[1.0]);
                Ok(())
            })
            .unwrap();
        for mode in ["gray", "overlay"] {
            app.session.execute("view.layerMask", json!({"mode":mode})).unwrap();
            let doc = app.session.active().unwrap().doc.clone();
            let full = photocraft_compose::render_reduced_damage(&doc, 512, 397, doc.bounds());
            let partial = photocraft_compose::render_reduced_damage(&doc, 512, 397, Rect::new(475, 360, 530, 420));
            let mut full_pixels = vec![[0.1, 0.2, 0.3, 1.0]; full.px.len()];
            let mut partial_pixels = vec![[0.1, 0.2, 0.3, 1.0]; partial.px.len()];
            overlay(&app, 0, &doc, full.rect, [512, 397], &mut full_pixels);
            overlay(&app, 0, &doc, partial.rect, [512, 397], &mut partial_pixels);
            if mode == "gray" {
                // Output cell (256, 199) covers native columns 500..502 and rows 389..391:
                // exactly half of its samples lie in the white quadrant.
                assert_eq!(full_pixels[199 * 512 + 256], [128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 1.0]);
            } else {
                assert_ne!(full_pixels[0][0], full_pixels[0][1], "Overlay mode must retain its colored rubylith");
            }
            assert!(full_pixels.windows(2).any(|pair| pair[0] != pair[1]), "the overlay must show the mask boundary");
            for y in 0..partial.rect.height() as usize {
                let start = (partial.rect.y0 as usize + y) * 512 + partial.rect.x0 as usize;
                let width = partial.rect.width() as usize;
                assert_eq!(&partial_pixels[y * width..(y + 1) * width], &full_pixels[start..start + width]);
            }
        }
    }
    #[test]
    fn overlay_skips_mismatched_pixel_grids_and_blends_matching_grids() {
        let original = [[0.1, 0.2, 0.3, 0.4], [0.4, 0.3, 0.2, 0.1]];
        let mut mismatched = original;
        blend_overlay(&mut mismatched, Some(vec![egui::Color32::WHITE]));
        assert_eq!(mismatched, original, "a partial overlay must not shear across output rows");

        let mut matching = original;
        blend_overlay(&mut matching, Some(vec![egui::Color32::WHITE; 2]));
        assert!(matching.iter().all(|pixel| *pixel == [1.0, 1.0, 1.0, 1.0]));
    }
    #[test]
    fn uv_mask_cache_follows_native_overlay_display_options() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let first = key(&app, 0);
        app.session.prefs.edit(|p| p.layer_mask_overlay.color = "#00ff00".into());
        let green = key(&app, 0);
        assert_ne!(first, green);
        app.session.prefs.edit(|p| p.layer_mask_overlay.opacity = 75.0);
        assert_ne!(green, key(&app, 0));
    }
}
