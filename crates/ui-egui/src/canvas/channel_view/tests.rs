use super::*;
use photocraft_engine::Session;
use photocraft_engine::channel_cmds::ChannelTarget;
use serde_json::json;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 4})).unwrap();
    s
}

#[test]
fn composite_bands_keep_the_downsample_grid() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 41, "height": 700})).unwrap();
    s.execute("edit.fill", json!({"color": "#398db2"})).unwrap();
    let doc = &s.active().unwrap().doc;
    let rect = doc.bounds();
    let full = photocraft_compose::render(doc, rect);
    let full_ref = &full;
    for factor in [1, 2, 3, 5] {
        let expected: Vec<_> = (0..rect.height())
            .step_by(factor as usize)
            .flat_map(|y| {
                (0..rect.width()).step_by(factor as usize).map(move |x| {
                    let p = full_ref.get(rect.x0 + x as i32, rect.y0 + y as i32);
                    let a = p[3].clamp(0.0, 1.0);
                    [p[0] * a + 1.0 - a, p[1] * a + 1.0 - a, p[2] * a + 1.0 - a, 1.0]
                })
            })
            .collect();
        assert_eq!(composite(doc, rect, factor), expected, "factor {factor}");
    }
}

#[test]
fn channel_eyes_reuse_composite_until_pixels_change() {
    let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 32})).unwrap();
    app.run("edit.fill", json!({"color": "#4080c0"})).unwrap();
    app.sync_views();
    let ctx = egui::Context::default();
    app.run("channel.setVisible", json!({"channel": "red", "visible": false})).unwrap();
    assert!(ensure(&mut app, &ctx, 0).is_some());
    let id = app.session.active().unwrap().doc.id.0;
    let first = app.channel_views.get_mut(&id).unwrap().source.as_mut().unwrap();
    let original_sample = first.pixels[0];
    // Mark the cache: a recomposite overwrites it, while a freed buffer's address can come back.
    first.pixels[0] = [0.0, 1.0, 0.0, 1.0];

    app.run("channel.setVisible", json!({"channel": "green", "visible": false})).unwrap();
    assert!(ensure(&mut app, &ctx, 0).is_some());
    let reused = app.channel_views.get(&id).unwrap().source.as_ref().unwrap();
    assert_eq!(reused.pixels[0], [0.0, 1.0, 0.0, 1.0], "visibility does not recompose the pixels");

    app.run("edit.fill", json!({"color": "#e02010"})).unwrap();
    assert!(ensure(&mut app, &ctx, 0).is_some());
    // A partial damage upload discards the source; a full redraw may replace it.
    // In either case, cached samples must never refer to the pre-edit pixels.
    if let Some(source) = app.channel_views.get(&id).unwrap().source.as_ref() {
        assert_ne!(source.pixels[0], original_sample, "a full redraw replaces stale pixels");
    }
    app.run("channel.setVisible", json!({"channel": "red", "visible": true})).unwrap();
    assert!(ensure(&mut app, &ctx, 0).is_some());
    let refreshed = app.channel_views.get(&id).unwrap().source.as_ref().unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(refreshed.revision, st.revision);
    assert_ne!(refreshed.pixels[0], original_sample, "new pixels must be recomposited");
    let expected = render(&st.doc, &st.channel_view, st.doc.bounds(), 1, Look::default(), st.active_layer).unwrap();
    let mut unused = None;
    let actual = render_with_source(
        &st.doc,
        &st.channel_view,
        st.doc.bounds(),
        1,
        Look::default(),
        st.active_layer,
        CompositeSource { cached: Some(&refreshed.pixels), fresh: &mut unused, display: None },
    )
    .unwrap();
    assert_eq!(actual, expected, "reused pixels must preserve channel colours");
    assert!(unused.is_none(), "reuse must not create a second composite");
}

#[test]
fn plain_composite_draws_nothing() {
    let s = session();
    let st = s.active().unwrap();
    assert!(render(&st.doc, &st.channel_view, st.doc.bounds(), 1, Look::default(), st.active_layer).is_none());
}

#[test]
fn quick_mask_overlay_is_red_over_masked_areas() {
    let mut s = session();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
    s.execute("select.editInQuickMaskMode", json!({})).unwrap();
    let st = s.active().unwrap();
    let px = render(&st.doc, &st.channel_view, st.doc.bounds(), 1, Look::default(), st.active_layer).unwrap();
    assert_eq!(px[0], Color32::TRANSPARENT, "selected: no overlay");
    assert_eq!(px[6], Color32::from_rgba_premultiplied(128, 0, 0, 128), "masked: 50% red");
}

#[test]
fn quick_mask_without_selection_shows_no_overlay_in_either_mode() {
    // #2743: nothing selected is a colourless mask, whichever way the colour reads.
    for (before, after) in [(None, Some("selected")), (Some("selected"), None), (Some("selected"), Some("masked"))] {
        let mut s = session();
        if let Some(i) = before {
            s.execute("channel.options", json!({"channel": "quickMask", "indicates": i})).unwrap();
        }
        s.execute("select.editInQuickMaskMode", json!({})).unwrap();
        let st = s.active().unwrap();
        let px = render(&st.doc, &st.channel_view, st.doc.bounds(), 1, Look::default(), st.active_layer).unwrap();
        assert!(px.iter().all(|p| *p == Color32::TRANSPARENT), "{before:?}");
        if let Some(i) = after {
            s.execute("channel.options", json!({"channel": "quickMask", "indicates": i})).unwrap();
            let st = s.active().unwrap();
            let px = render(&st.doc, &st.channel_view, st.doc.bounds(), 1, Look::default(), st.active_layer).unwrap();
            assert!(px.iter().all(|p| *p == Color32::TRANSPARENT), "{before:?} → {i}");
        }
    }
    // Selected Areas with Select All: the whole canvas is coloured.
    let mut s = session();
    s.execute("channel.options", json!({"channel": "quickMask", "indicates": "selected"})).unwrap();
    s.execute("select.all", json!({})).unwrap();
    s.execute("select.editInQuickMaskMode", json!({})).unwrap();
    let st = s.active().unwrap();
    let px = render(&st.doc, &st.channel_view, st.doc.bounds(), 1, Look::default(), st.active_layer).unwrap();
    assert!(px.iter().all(|p| *p == Color32::from_rgba_premultiplied(128, 0, 0, 128)));
}

#[test]
fn layer_mask_views_and_damage_on_the_downsampled_grid() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 37, "height": 21})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s.execute("paint.gradient", json!({"from": [0, 0], "to": [37, 0], "colors": ["#000000", "#ffffff"], "target": "mask"})).unwrap();
    s.execute("view.layerMask", json!({"mode": "gray"})).unwrap();
    let st = s.active().unwrap();
    let b = st.doc.bounds();
    let full = render(&st.doc, &st.channel_view, b, 1, Look::default(), st.active_layer).unwrap();
    assert!(full[0].r() < 10 && full[36].r() > 245 && full[0].a() == 255, "opaque grayscale ramp");
    // A sub-rect starting on the factor grid matches the same pixels of the full render.
    for f in [1u32, 2, 3] {
        let all = render(&st.doc, &st.channel_view, b, f, Look::default(), st.active_layer).unwrap();
        let w = b.width().div_ceil(f) as usize;
        let r = Rect::new(3 * f as i32, 2 * f as i32, 30, 19);
        let part = render(&st.doc, &st.channel_view, r, f, Look::default(), st.active_layer).unwrap();
        let pw = r.width().div_ceil(f) as usize;
        for (i, p) in part.iter().enumerate() {
            assert_eq!(*p, all[(i / pw + 2) * w + i % pw + 3], "factor {f}, pixel {i}");
        }
    }
    s.execute("view.layerMask", json!({"mode": "overlay"})).unwrap();
    let st = s.active().unwrap();
    let px = render(&st.doc, &st.channel_view, b, 1, Look::default(), st.active_layer).unwrap();
    assert!(px[0].r() >= 124 && px[0].g() == 0 && px[0].a() >= 124, "about 50% red over the hidden end: {:?}", px[0]);
    assert!(px[36].a() <= 4, "next to nothing over the revealed end: {:?}", px[36]);
}

#[test]
fn layer_mask_views_show_the_feathered_mask() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 37, "height": 21})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s.execute("paint.gradient", json!({"from": [0, 0], "to": [37, 0], "colors": ["#000000", "#ffffff"], "target": "mask"})).unwrap();
    s.execute("layer.layerMask.edit", json!({"feather": 4})).unwrap();
    s.execute("view.layerMask", json!({"mode": "gray"})).unwrap();

    let st = s.active().unwrap();
    let bounds = st.doc.bounds();
    let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
    let raw = layer.mask.as_ref().unwrap().surface.sample_channel(0, 0, 0);
    let effective = photocraft_compose::masks::combined_mask(layer, bounds).unwrap();
    let softened = effective.sample_channel(0, 0, 0);
    assert!((softened - raw).abs() > 0.01, "the fixture has a visible feather at its edge");

    let gray = render(&st.doc, &st.channel_view, bounds, 1, Look::default(), st.active_layer).unwrap();
    assert_eq!(gray[0], Color32::from_gray(q(softened)), "gray view uses the compositor's feathered mask");

    s.execute("view.layerMask", json!({"mode": "overlay"})).unwrap();
    let st = s.active().unwrap();
    let overlay = render(&st.doc, &st.channel_view, bounds, 1, Look::default(), st.active_layer).unwrap();
    let alpha = 0.5 * (1.0 - softened);
    let red = AlphaChannel::DEFAULT_COLOR.to_rgb();
    assert_eq!(overlay[0], Color32::from_rgba_premultiplied(q(red[0] * alpha), q(red[1] * alpha), q(red[2] * alpha), q(alpha)));
}

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

/// Layer Mask Display Options: the rubylith takes their colour and opacity, alone over the
/// composite and with a colour channel hidden.
#[test]
fn layer_mask_overlay_takes_its_display_options() {
    let mut s = session();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    s.execute("layer.layerMask.hideAll", json!({})).unwrap();
    s.execute("view.layerMask", json!({"mode": "overlay"})).unwrap();
    let green = Look { mask_rgb: [0.0, 1.0, 0.0], mask_opacity: 0.25, ..Look::default() };
    let st = s.active().unwrap();
    let b = st.doc.bounds();
    assert_eq!(render(&st.doc, &st.channel_view, b, 1, Look::default(), st.active_layer).unwrap()[0], Color32::from_rgba_premultiplied(128, 0, 0, 128));
    assert_eq!(render(&st.doc, &st.channel_view, b, 1, green, st.active_layer).unwrap()[0], Color32::from_rgba_premultiplied(0, 64, 0, 64));
    // Red hidden: the white picture shows cyan, then green at 25 % over it.
    s.execute("channel.setVisible", json!({"channel": "red", "visible": false})).unwrap();
    let st = s.active().unwrap();
    assert_eq!(render(&st.doc, &st.channel_view, b, 1, green, st.active_layer).unwrap()[0], Color32::from_rgb(0, 255, 191));
}

/// `cargo test --release -p photocraft-ui-egui --lib mask_view_switch_24mp -- --ignored --nocapture`
#[test]
#[ignore = "benchmark"]
fn mask_view_switch_24mp() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 6000, "height": 4000})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#d0402b"})).unwrap();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s.execute("paint.gradient", json!({"from": [0, 0], "to": [6000, 0], "colors": ["#000000", "#ffffff"], "target": "mask"})).unwrap();
    let factor = 6000u32.div_ceil(MAX_SIDE);
    for mode in ["gray", "overlay"] {
        s.execute("view.layerMask", json!({"mode": mode})).unwrap();
        let st = s.active().unwrap();
        let mut best = f64::MAX;
        for _ in 0..8 {
            let t = std::time::Instant::now();
            let px = render(&st.doc, &st.channel_view, st.doc.bounds(), factor, Look::default(), st.active_layer).unwrap();
            let img = egui::ColorImage::new([3000, 2000], px);
            std::hint::black_box(img);
            best = best.min(t.elapsed().as_secs_f64() * 1000.0);
        }
        println!("24 MP mask view {mode}: {best:.1} ms (render at 1/{factor})");
    }
}

/// Interface ▸ Show Channels in Color (#2385): a solo RGB / CMYK channel tints the canvas
/// instead of staying gray, like the panel thumbnails wired in #1986.
#[test]
fn show_channels_in_color_tints_the_canvas() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4, "height": 4, "mode": "rgb"})).unwrap();
    s.execute("edit.fill", json!({"color": "#4080c0"})).unwrap();
    s.execute("channel.target", json!({"channel": "green"})).unwrap();
    let st = s.active().unwrap();
    let b = st.doc.bounds();
    assert_eq!(render(&st.doc, &st.channel_view, b, 1, Look::default(), st.active_layer).unwrap()[0], Color32::from_gray(128));
    assert_eq!(render(&st.doc, &st.channel_view, b, 1, Look { in_color: true, ..Look::default() }, st.active_layer).unwrap()[0], Color32::from_rgb(0, 128, 0));

    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4, "height": 4, "mode": "cmyk"})).unwrap();
    s.execute("edit.fill", json!({"color": "#4080c0"})).unwrap();
    s.execute("channel.target", json!({"channel": "cyan"})).unwrap();
    let st = s.active().unwrap();
    let px = render(&st.doc, &st.channel_view, st.doc.bounds(), 1, Look { in_color: true, ..Look::default() }, st.active_layer).unwrap()[0];
    assert!(px.r() < 100 && px.g() > 150 && px.b() > 150, "cyan ink, not gray: {px:?}");
    let gray = render(&st.doc, &st.channel_view, st.doc.bounds(), 1, Look::default(), st.active_layer).unwrap()[0];
    assert_eq!(gray.r(), gray.g(), "off stays grayscale");
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
    doc.icc_profile = Some(photocraft_cms::Builtin::DisplayP3.profile().to_bytes());
    let mut surface = Surface::new(photocraft_color::PixelFormat::GRAY8);
    surface.fill_rect(doc.bounds(), &[1.0]);
    let mut spot = AlphaChannel::new("spot", surface);
    spot.spot = Some((photocraft_color::Color::rgb(1.0, 0.0, 0.0), 1.0));
    doc.channels.push(spot);
    let display = photocraft_engine::Session::new().color.canvas_display(&doc).unwrap();
    assert!(!display.is_identity());
    let mut view = ChannelView { color_hidden: vec![true, false, true], ..Default::default() };
    let tinted = Look { in_color: true, ..Look::default() };
    let raw = render(&doc, &view, doc.bounds(), 1, tinted, None).unwrap();
    let converted = render_for_display(&doc, &view, doc.bounds(), 1, tinted, None, Some(&display)).unwrap();
    assert_ne!(raw, converted, "the colour-channel base uses the document profile");
    view.alpha_visible = vec![true];
    let overlay = render_for_display(&doc, &view, doc.bounds(), 1, tinted, None, Some(&display)).unwrap();
    assert!(overlay.iter().all(|pixel| *pixel == Color32::RED), "screen-space spot colour stays red");
}

#[test]
fn solo_channel_keeps_its_display_pixels_through_transform_start_and_cancel() {
    use photocraft_cms::Builtin;
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
        app.session.color.set_displays(Ok(vec![
            Display { id: 1, name: "sRGB".into(), frame: [0.0, 0.0, 100.0, 100.0], profile_name: None, icc: icc("srgb") },
            Display { id: 4, name: "Display P3".into(), frame: [100.0, 0.0, 100.0, 100.0], profile_name: None, icc: icc("display-p3") },
        ]));
        app.session.color.main_display = Some(1);
        app.run("prefs.set", json!({"values":{"interface.showChannelsInColor":true}})).unwrap();
        app.run("channel.target", json!({"channel":"green"})).unwrap();
        let ctx = egui::Context::default();
        let before = [1, 4].map(|output| ensure_for_display(&mut app, &ctx, 0, Some(output)).unwrap());
        assert_ne!(before[0], before[1], "each monitor keeps its own channel texture");
        let mut delta = ctx.tex_manager().write().take_delta();
        let pixels = |id| {
            let egui::ImageData::Color(image) = &delta.set.get(&id).unwrap()[0].image;
            image.pixels.clone()
        };
        let original = before.map(pixels);
        delta.clear();
        let cache = app.channel_views.values().next().unwrap();
        assert_eq!(cache.outputs.len(), 2);
        assert!(cache.source.is_some(), "the monitors share one sampled composite");
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
        assert_eq!([1, 4].map(|output| ensure_for_display(&mut app, &ctx, 0, Some(output)).unwrap()), before);
        let mut settled = ctx.tex_manager().write().take_delta();
        assert!(settled.set.is_empty(), "cancel reuses unchanged display textures");
        settled.clear();
    }
}

/// Lab: a and b tint over a neutral Lightness; Lightness itself stays gray (Adobe's exception).
#[test]
fn show_channels_in_color_lab_lightness_stays_gray() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4, "height": 4, "mode": "lab"})).unwrap();
    s.execute("edit.fill", json!({"color": "#4080c0"})).unwrap();
    let b = s.active().unwrap().doc.bounds();
    s.execute("channel.target", json!({"channel": {"color": 0}})).unwrap();
    let st = s.active().unwrap();
    let px = render(&st.doc, &st.channel_view, b, 1, Look { in_color: true, ..Look::default() }, st.active_layer).unwrap()[0];
    let spread = px.r().max(px.g().max(px.b())) as i32 - px.r().min(px.g().min(px.b())) as i32;
    assert!(spread <= 3, "Lightness stays gray: {px:?}");
    s.execute("channel.target", json!({"channel": {"color": 1}})).unwrap();
    let st = s.active().unwrap();
    let px = render(&st.doc, &st.channel_view, b, 1, Look { in_color: true, ..Look::default() }, st.active_layer).unwrap()[0];
    assert!(px.r() != px.g() || px.g() != px.b(), "a channel is tinted, not gray: {px:?}");
}

/// The canvas texture cache is keyed by the preference, so toggling it rebuilds the view.
#[test]
fn ensure_honours_show_channels_in_color() {
    let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    app.sync_views();
    app.run("channel.target", json!({"channel": "green"})).unwrap();
    let ctx = egui::Context::default();
    assert!(ensure(&mut app, &ctx, 0).is_some());
    let key_off = app.channel_views.values().next().unwrap().outputs.get(&None).unwrap().key;
    app.session.execute("prefs.set", json!({"values": {"interface.showChannelsInColor": true}})).unwrap();
    assert!(ensure(&mut app, &ctx, 0).is_some());
    let key_on = app.channel_views.values().next().unwrap().outputs.get(&None).unwrap().key;
    assert_ne!(key_off, key_on, "the preference changes the cached view key");
    // So do the Layer Mask Display Options.
    app.session.execute("channel.options", json!({"channel": "mask", "color": "#00ff00", "opacity": 70})).unwrap();
    assert!(ensure(&mut app, &ctx, 0).is_some());
    assert_ne!(app.channel_views.values().next().unwrap().outputs.get(&None).unwrap().key, key_on, "the overlay options change the cached view key");
}

#[test]
fn solo_alpha_is_grayscale_and_colour_channels_split() {
    let mut s = session();
    s.execute("edit.fill", json!({"color": "#ff8000"})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 2, "height": 4})).unwrap();
    s.execute("channel.new", json!({"fill": "selection"})).unwrap();
    let st = s.active().unwrap();
    assert_eq!(st.channel_view.target, ChannelTarget::Alpha(0));
    let px = render(&st.doc, &st.channel_view, st.doc.bounds(), 1, Look::default(), st.active_layer).unwrap();
    assert_eq!((px[0], px[5]), (Color32::WHITE, Color32::BLACK));
    // Green alone: grayscale of the green channel; with blue hidden only.
    s.execute("channel.target", json!({"channel": "green"})).unwrap();
    let st = s.active().unwrap();
    let px = render(&st.doc, &st.channel_view, st.doc.bounds(), 1, Look::default(), st.active_layer).unwrap();
    assert_eq!(px[0], Color32::from_gray(128));
    s.execute("channel.target", json!({"channel": "composite"})).unwrap();
    s.execute("channel.setVisible", json!({"channel": "red", "visible": false})).unwrap();
    let st = s.active().unwrap();
    let px = render(&st.doc, &st.channel_view, st.doc.bounds(), 2, Look::default(), st.active_layer).unwrap();
    assert_eq!(px.len(), 4 * 2);
    assert_eq!(px[0], Color32::from_rgb(0, 128, 0));
}

#[test]
fn rgb_off_shows_alpha_quick_mask_or_the_layer_mask() {
    let mut s = session();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("channel.new", json!({"fill": "white"})).unwrap();
    s.execute("channel.setVisible", json!({"channel": "composite", "visible": true})).unwrap();
    s.execute("channel.setVisible", json!({"channel": "composite", "visible": false})).unwrap();
    let st = s.active().unwrap();
    let px = render(&st.doc, &st.channel_view, st.doc.bounds(), 1, Look::default(), st.active_layer).unwrap();
    assert_eq!(px[0], Color32::WHITE, "Alpha alone is the channel, not the red image");

    let mut q = session();
    q.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    q.execute("select.editInQuickMaskMode", json!({})).unwrap();
    q.execute("channel.setVisible", json!({"channel": "composite", "visible": false})).unwrap();
    let st = q.active().unwrap();
    let px = render(&st.doc, &st.channel_view, st.doc.bounds(), 1, Look::default(), st.active_layer).unwrap();
    assert_eq!(px[0], Color32::WHITE, "Quick Mask alone is grayscale, not the red composite");

    let mut m = session();
    m.execute("layer.new.layer", json!({})).unwrap();
    m.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    m.execute("layer.layerMask.hideAll", json!({})).unwrap();
    m.execute("channel.setVisible", json!({"channel": "composite", "visible": false})).unwrap();
    let st = m.active().unwrap();
    assert_eq!(st.channel_view.visible_colors(3), 0);
    let px = render(&st.doc, &st.channel_view, st.doc.bounds(), 1, Look::default(), st.active_layer).unwrap();
    assert_eq!(px[0], Color32::WHITE, "a mask with its eye off must not leak through");
    m.execute("view.layerMask", json!({"mode":"overlay"})).unwrap();
    let st = m.active().unwrap();
    let px = render(&st.doc, &st.channel_view, st.doc.bounds(), 1, Look::default(), st.active_layer).unwrap();
    assert_eq!(px[0], Color32::BLACK, "showing the mask eye reveals its grayscale values");
}
