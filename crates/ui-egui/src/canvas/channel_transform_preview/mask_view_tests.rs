use super::*;
use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::Size;

#[test]
fn ordinary_mask_target_starts_without_changing_eyes_and_previews_only_the_mask() {
    for linked in [false, true] {
        let mut a = super::tests::app();
        a.run("layer.layerMask.revealAll", json!({})).unwrap();
        a.session
            .edit("mask", |doc, active| {
                let mask = doc.layer_mut(active.unwrap()).unwrap().mask.as_mut().unwrap();
                mask.linked = linked;
                mask.surface.fill_rect(photocraft_geom::Rect::new(0, 0, 12, 24), &[0.0]);
                Ok(())
            })
            .unwrap();
        a.ui.mask_target = true;
        let before = a.session.active().unwrap().doc.clone();
        let eyes = a.session.active().unwrap().channel_view.clone();
        let ctx = egui::Context::default();
        crate::transform_tool::begin(&mut a, &ctx).unwrap();
        assert_eq!(a.session.active().unwrap().channel_view, eyes);
        assert_eq!(a.ui.transform.as_ref().unwrap().target, Some(json!("mask")));
        crate::channel_transform_preview::edit(&mut a, |t| {
            t.quad = t.quad.map(|[x, y]| [x * 1.25, y]);
        });
        texture(&mut a, &ctx, 0, None).unwrap();
        let shown = a.transform_preview.as_ref().unwrap().channel.as_ref().unwrap().shown.clone();
        let id = a.session.active().unwrap().active_layer.unwrap();
        assert_eq!(shown.layer(id).unwrap().surface(), before.layer(id).unwrap().surface());
        assert_ne!(shown.layer(id).unwrap().mask, before.layer(id).unwrap().mask);
        assert_eq!(a.session.active().unwrap().doc, before, "preview never edits the source");
        crate::transform_tool::commit(&mut a);
        assert!(!a.ui.status_error, "{}", a.ui.status);
        assert_eq!(a.session.active().unwrap().doc.layer(id).unwrap().mask, shown.layer(id).unwrap().mask);
        assert_eq!(a.session.active().unwrap().channel_view, eyes);
    }
}

#[test]
fn preview_uploads_match_canvas_sampling_for_full_reduced_and_capped_sources() {
    for (large, max_side, magnification) in
        [(false, 4096, egui::TextureFilter::Nearest), (false, 16, egui::TextureFilter::Linear), (true, 4096, egui::TextureFilter::Linear)]
    {
        let mut a = super::tests::app();
        if large {
            a.session.add_document(Document::with_background("reduced mask", Size::new(3000, 2000), ColorMode::Rgb, SampleType::U8, Color::WHITE), None);
            a.sync_views();
        }
        a.run("layer.layerMask.revealAll", json!({})).unwrap();
        a.run("view.layerMask", json!({"mode":"overlay"})).unwrap();
        let ctx = egui::Context::default();
        ctx.input_mut(|i| i.max_texture_side = max_side);
        crate::transform_tool::begin(&mut a, &ctx).unwrap();
        let idx = a.session.active_index().unwrap();
        let id = texture(&mut a, &ctx, idx, None).unwrap();
        assert_eq!(ctx.tex_manager().read().meta(id).unwrap().options.magnification, magnification, "large={large}, max_side={max_side}");
        crate::channel_transform_preview::edit(&mut a, |t| {
            t.quad[2][0] += 1.0;
        });
        let again = texture(&mut a, &ctx, idx, None).unwrap();
        assert_eq!(again, id, "a geometry change retains the texture");
        assert_eq!(ctx.tex_manager().read().meta(id).unwrap().options.magnification, magnification, "updates retain sampling");
    }
}
