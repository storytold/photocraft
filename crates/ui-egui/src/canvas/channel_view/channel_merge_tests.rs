//! Channel-edit regressions retained across the shared UI migration.
use super::*;
use photocraft_engine::color_cmds::ColorExt;

#[test]
fn filter_mask_views_use_the_filter_masks_own_feather() {
    for (layer_feather, filter_feather) in [(4.0, 0.0), (0.0, 4.0), (4.0, 4.0)] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 37, "height": 21})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("edit.fill", json!({"color": "#336699"})).unwrap();
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        s.execute("filter.blur.gaussianBlur", json!({"radius": 2})).unwrap();
        s.execute("layer.layerMask.hideAll", json!({})).unwrap();
        s.execute("paint.gradient", json!({"from": [0, 0], "to": [37, 0], "colors": ["#000000", "#ffffff"], "target": "filterMask"})).unwrap();
        s.edit("mask feathers", |doc, active| {
            let layer = doc.layer_mut(active.unwrap()).unwrap();
            layer.mask.as_mut().unwrap().feather = layer_feather;
            let photocraft_doc::LayerContent::Smart(sm) = &mut layer.content else { panic!("smart object") };
            sm.filter_mask.as_mut().unwrap().feather = filter_feather;
            Ok(())
        })
        .unwrap();
        s.execute("view.layerMask", json!({"mode": "gray", "mask": "filter"})).unwrap();
        let st = s.active().unwrap();
        let bounds = st.doc.bounds();
        let mut layer = st.doc.layer(st.active_layer.unwrap()).unwrap().clone();
        layer.mask = photocraft_engine::mask_view_cmds::viewed_mask(&layer, true).cloned();
        layer.vector_mask = None;
        let effective = photocraft_compose::masks::combined_mask(&layer, bounds).unwrap_or_else(|| layer.mask.as_ref().unwrap().surface.clone());
        let softened = effective.sample_channel(0, 0, 0);
        let gray = render(&st.doc, &st.channel_view, bounds, 1, Look::default(), st.active_layer).unwrap();
        assert_eq!(gray[0], Color32::from_gray(q(softened)), "layer feather {layer_feather}, filter feather {filter_feather}");

        s.execute("view.layerMask", json!({"mode": "overlay", "mask": "filter"})).unwrap();
        let st = s.active().unwrap();
        let overlay = render(&st.doc, &st.channel_view, bounds, 1, Look::default(), st.active_layer).unwrap();
        let alpha = 0.5 * (1.0 - softened);
        let red = AlphaChannel::DEFAULT_COLOR.to_rgb();
        assert_eq!(overlay[0], Color32::from_rgba_premultiplied(q(red[0] * alpha), q(red[1] * alpha), q(red[2] * alpha), q(alpha)));
    }
}

#[test]
fn display_conversion_precedes_spot_overlay_on_a_solo_colour_channel() {
    let mut doc = Document::with_background(
        "channel display",
        photocraft_doc::Size::new(4, 4),
        ColorMode::Rgb,
        photocraft_color::SampleType::U8,
        photocraft_color::Color::rgb(0.2, 0.4, 0.6),
    );
    doc.icc_profile = Some(craft::cms::Builtin::DisplayP3.profile().to_bytes());
    let mut surface = Surface::new(photocraft_color::PixelFormat::GRAY8);
    surface.fill_rect(doc.bounds(), &[1.0]);
    let mut spot = AlphaChannel::new("spot", surface);
    spot.spot = Some((photocraft_color::Color::rgb(1.0, 0.0, 0.0), 1.0));
    doc.channels.push(spot);
    let display = photocraft_engine::Session::new().color().canvas_display(&doc).unwrap();
    assert!(!display.is_identity());
    let mut view = ChannelView { color_hidden: vec![true, false, true], ..Default::default() };
    let tinted = Look { in_color: true, ..Look::default() };
    let raw = render(&doc, &view, doc.bounds(), 1, tinted, None).unwrap();
    let converted = render_for_display(&doc, &view, doc.bounds(), 1, tinted, Some(&display), &Screen::default()).unwrap();
    assert_ne!(raw, converted, "the colour-channel base uses the document profile");
    view.alpha_visible = vec![true];
    let overlay = render_for_display(&doc, &view, doc.bounds(), 1, tinted, Some(&display), &Screen::default()).unwrap();
    assert!(overlay.iter().all(|pixel| *pixel == Color32::RED), "screen-space spot colour stays red");
}

#[test]
fn solo_channel_keeps_its_display_pixels_through_transform_start_and_cancel() {
    use craft::cms::Builtin;
    use photocraft_engine::color_cmds::resolve_profile;
    use photocraft_engine::display_color::Display;

    for linear in [false, true] {
        let mut doc = Document::with_background(
            "channel display",
            photocraft_doc::Size::new(16, 16),
            ColorMode::Rgb,
            photocraft_color::SampleType::F32,
            if linear { photocraft_color::Color::rgb(0.001, 0.002, 0.003) } else { photocraft_color::Color::rgb(0.2, 0.4, 0.6) },
        );
        if linear {
            doc.icc_profile = Some(Builtin::LinearSrgb.profile().to_bytes());
        }
        let mut session = Session::new();
        session.add_document(doc, None);
        let mut app = PhotocraftApp::new(session, crate::Services::default());
        app.sync_views();
        let icc = |id: &str| Some(resolve_profile(id, None, None).unwrap().to_bytes());
        app.session.color_mut().set_displays(Ok(vec![
            Display { id: 1, name: "sRGB".into(), frame: [0.0, 0.0, 100.0, 100.0], profile_name: None, icc: icc("srgb") },
            Display { id: 4, name: "Display P3".into(), frame: [100.0, 0.0, 100.0, 100.0], profile_name: None, icc: icc("display-p3") },
        ]));
        app.session.color_mut().main_display = Some(1);
        app.run("prefs.set", json!({"values":{"interface.showChannelsInColor":true}})).unwrap();
        app.run("channel.target", json!({"channel":"green"})).unwrap();
        let ctx = egui::Context::default();
        let before = [1, 4].map(|output| {
            let doc = app.session.active().unwrap().doc.clone();
            let overlay = Overlay::of(&app, 0, &doc, Some(output)).unwrap();
            (overlay.key, overlay.render(&doc, doc.bounds(), 1).unwrap())
        });
        assert_ne!(before[0].0, before[1].0, "each monitor has a separate display cache key");
        let original = before.clone().map(|(_, pixels)| pixels);
        assert_ne!(original[0], original[1], "monitor profiles produce different channel pixels");
        crate::transform_tool::begin(&mut app, &ctx).unwrap();
        let during = [1, 4].map(|output| crate::channel_transform_preview::texture(&mut app, &ctx, 0, Some(output)).unwrap());
        let mut delta = ctx.tex_manager().write().take_delta();
        for (id, expected) in during.into_iter().zip(&original) {
            let egui::ImageData::Color(image) = &delta.set.get(&id).unwrap()[0].image;
            assert_eq!(&image.pixels, expected, "the channel must not change when Free Transform starts; linear={linear}");
        }
        delta.clear();
        crate::transform_tool::cancel(&mut app);
        let after = [1, 4].map(|output| {
            let doc = app.session.active().unwrap().doc.clone();
            let overlay = Overlay::of(&app, 0, &doc, Some(output)).unwrap();
            (overlay.key, overlay.render(&doc, doc.bounds(), 1).unwrap())
        });
        assert_eq!(after, before, "cancel preserves both the display pixels and their cache keys");
    }
}

#[test]
fn bounded_cmyk_channel_previews_keep_native_inks_at_every_depth() {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 17, "height": 131, "mode": "cmyk", "depth": depth})).unwrap();
        s.edit("native ink stripes", |doc, active| {
            let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            for y in 0..131 {
                for x in 0..17 {
                    surface.write_pixel(x, y, &[x as f32 / 16.0, y as f32 / 130.0, 0.4, 0.0, 1.0]);
                }
            }
            Ok(())
        })
        .unwrap();
        s.execute("channel.target", json!({"channel": "black"})).unwrap();
        let st = s.active().unwrap();
        let doc = &st.doc;
        for factor in [1, 2, 3] {
            let full = inks(doc, doc.bounds(), factor, 0);
            for rows in [1, 5] {
                assert_eq!(inks(doc, doc.bounds(), factor, rows), full, "{depth}-bit, stride {factor}, band {rows}");
            }
            let bounded = render_for_display_bounded(doc, &st.channel_view, doc.bounds(), factor, Look::default(), None, &Screen::default()).unwrap();
            assert!(bounded.iter().all(|px| *px == Color32::WHITE), "K stays empty: {depth}-bit, stride {factor}");
            assert_eq!(Some(bounded), render(doc, &st.channel_view, doc.bounds(), factor, Look::default(), st.active_layer));
        }
    }
}
