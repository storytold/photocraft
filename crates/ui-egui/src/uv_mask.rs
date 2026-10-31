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
pub(crate) fn overlay(
    app: &PhotocraftApp,
    index: usize,
    doc: &photocraft_doc::Document,
    rect: photocraft_geom::Rect,
    output_size: [u32; 2],
    pixels: &mut [[f32; 4]],
    factor: u32,
) {
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
        crate::channel_view::render(doc, &view, rect, factor, look, st.active_layer)
    };
    if let Some(px) = px {
        for (out, p) in pixels.iter_mut().zip(px) {
            let rgba = p.to_array().map(|v| v as f32 / 255.0);
            for i in 0..3 {
                out[i] = rgba[i] + out[i] * (1.0 - rgba[3]);
            }
            out[3] = 1.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
