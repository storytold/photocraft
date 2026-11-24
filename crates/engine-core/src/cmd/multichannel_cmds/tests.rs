use super::*;
use photocraft_color::SampleType;

fn session(mode: &str, depth: u64) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 12, "height": 8, "depth": depth, "mode": mode})).unwrap();
    s.edit("setup", |doc, active| {
        let b = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let ch = surf.channels();
        let mut data = Vec::new();
        for y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                for c in 0..ch - 1 {
                    data.push(((x * 3 + y * 5 + c as i32 * 7) % 16) as f32 / 15.0);
                }
                data.push(1.0);
            }
        }
        surf.write_region(b, &data);
        Ok(())
    })
    .unwrap();
    s
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

fn pixels(s: &Session) -> Vec<f32> {
    let d = doc(s);
    d.layers[0].surface().unwrap().read_region(d.bounds())
}

fn tol(depth: u64) -> f32 {
    if depth == 8 { 0.5 / 255.0 + 1e-6 } else { 1e-4 }
}

#[test]
fn rgb_becomes_cmy_inks_and_back_exactly() {
    for depth in [8, 16, 32] {
        let mut s = session("rgb", depth);
        let before = pixels(&s);
        assert!(s.is_enabled("image.mode.multichannel"));
        let r = s.execute("image.mode.multichannel", json!({})).unwrap();
        assert_eq!(r["channels"], json!(["Cyan", "Magenta", "Yellow"]));
        let d = doc(&s);
        assert_eq!(d.mode, ColorMode::Multichannel);
        assert!(d.layers.is_empty());
        assert_eq!(ink_names(d), ["Cyan", "Magenta", "Yellow"]);
        // Cyan ink = the red data read as ink.
        let cyan = d.channels[0].surface.pixel(4, 3)[0];
        let red = before[(3 * 12 + 4) * 4];
        assert!((cyan - (1.0 - red)).abs() <= tol(depth), "{depth}: {cyan} vs {red}");
        assert!(!s.is_enabled("image.mode.multichannel"), "already multichannel");
        // The display prints the inks: a white RGB pixel shows paper.
        let shown = photocraft_compose::flatten(doc(&s));
        assert!(shown.px.iter().all(|p| p[3] == 1.0));
        let r = s.execute("image.mode.rgb", json!({})).unwrap();
        assert_eq!(r["converted"], "channels");
        assert_eq!(doc(&s).mode, ColorMode::Rgb);
        assert!(doc(&s).channels.is_empty());
        let after = pixels(&s);
        assert_eq!(after.len(), before.len());
        for (a, b) in after.iter().zip(&before) {
            assert!((a - b).abs() <= 2.0 * tol(depth), "{depth}: {a} vs {b}");
        }
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(doc(&s).mode, ColorMode::Multichannel);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(doc(&s).mode, ColorMode::Rgb);
        assert_eq!(pixels(&s), before);
    }
}

#[test]
fn cmyk_lab_and_gray_channel_sets() {
    for (mode, back, names) in [
        ("cmyk", "image.mode.cmyk", vec!["Cyan", "Magenta", "Yellow", "Black"]),
        ("lab", "image.mode.lab", vec!["Alpha 1", "Alpha 2", "Alpha 3"]),
        ("grayscale", "image.mode.grayscale", vec!["Black"]),
    ] {
        for depth in [8, 16] {
            let mut s = session(mode, depth);
            let before = pixels(&s);
            let r = s.execute("image.mode.multichannel", json!({})).unwrap();
            assert_eq!(r["channels"], json!(names), "{mode}");
            assert_eq!(doc(&s).depth, if depth == 8 { SampleType::U8 } else { SampleType::U16 });
            s.execute(back, json!({})).unwrap();
            for (a, b) in pixels(&s).iter().zip(&before) {
                assert!((a - b).abs() <= 2.0 * tol(depth), "{mode}/{depth}: {a} vs {b}");
            }
        }
    }
}

#[test]
fn flattens_layers_and_keeps_alpha_channels() {
    let mut s = session("rgb", 8);
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
    s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
    s.execute("select.saveSelection", json!({})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    let alpha = doc(&s).channels.len();
    assert_eq!(alpha, 1);
    s.execute("image.mode.multichannel", json!({})).unwrap();
    let d = doc(&s);
    assert_eq!(d.channels.len(), 4);
    assert!(d.channels[3].spot.is_none(), "the saved selection stays an alpha channel");
    // The black fill on the upper layer is solid ink in all three plates.
    for channel in d.channels.iter().take(3) {
        assert!(channel.surface.pixel(1, 1)[0] > 0.99);
    }
    // Back to RGB: the alpha channel survives, the inks become colour channels.
    s.execute("image.mode.rgb", json!({})).unwrap();
    assert_eq!(doc(&s).channels.len(), 1);
    assert!(doc(&s).layers[0].surface().unwrap().pixel(1, 1)[0] < 0.01);
}

#[test]
fn targeted_alpha_channel_stays_targeted_across_conversions() {
    use crate::channel_cmds::ChannelTarget;
    let mut s = session("rgb", 8);
    s.execute("channel.new", json!({})).unwrap();
    let view = |s: &Session| s.active().unwrap().channel_view.clone();
    assert_eq!(view(&s).target, ChannelTarget::Alpha(0));
    assert_eq!(view(&s).alpha_visible, [true]);
    s.execute("image.mode.multichannel", json!({})).unwrap();
    // The alpha channel now follows the three inks.
    assert_eq!(view(&s).target, ChannelTarget::Alpha(3));
    assert_eq!(view(&s).alpha_visible, [false, false, false, true]);
    let cyan = doc(&s).channels[0].surface.pixel(4, 3)[0];
    s.execute("edit.fill", json!({"color": "#ffffff"})).unwrap();
    assert_eq!(doc(&s).channels[0].surface.pixel(4, 3)[0], cyan, "the fill must not touch the Cyan plate");
    assert_eq!(doc(&s).channels[3].name, "Alpha 1");
    // Back to RGB: the inks leave, the alpha channel is index 0 again.
    s.execute("image.mode.rgb", json!({})).unwrap();
    assert_eq!(view(&s).target, ChannelTarget::Alpha(0));
    assert_eq!(view(&s).alpha_visible, [true]);
    // A targeted ink becomes the composite.
    s.execute("image.mode.multichannel", json!({})).unwrap();
    s.active_mut().unwrap().channel_view.target = ChannelTarget::Alpha(1);
    s.execute("image.mode.rgb", json!({})).unwrap();
    assert_eq!(view(&s).target, ChannelTarget::Composite);
}

#[test]
fn mismatched_channel_count_converts_the_printed_look() {
    let mut s = session("rgb", 8);
    s.execute("image.mode.multichannel", json!({})).unwrap();
    // Drop the Yellow plate: two inks can't be RGB channels.
    s.edit("drop", |doc, _| {
        doc.channels.truncate(2);
        Ok(())
    })
    .unwrap();
    let shown = photocraft_compose::flatten(doc(&s));
    let r = s.execute("image.mode.rgb", json!({})).unwrap();
    assert_eq!(r["converted"], "appearance");
    let px = doc(&s).layers[0].surface().unwrap().pixel(5, 2);
    let want = shown.get(5, 2);
    for c in 0..3 {
        assert!((px[c] - want[c]).abs() < 1.0 / 255.0 + 1e-4);
    }
}

#[test]
fn duotone_inks_become_channels() {
    let mut s = session("grayscale", 8);
    s.execute("image.mode.duotone", json!({"type": "duotone"})).unwrap();
    let inks = doc(&s).duotone.as_ref().unwrap().inks.len();
    let r = s.execute("image.mode.multichannel", json!({})).unwrap();
    assert_eq!(r["channels"].as_array().unwrap().len(), inks);
}

#[test]
fn psd_round_trip_keeps_names_inks_and_values() {
    for depth in [8, 16] {
        let mut s = session("cmyk", depth);
        s.execute("image.mode.multichannel", json!({})).unwrap();
        let d = doc(&s).clone();
        let out = photocraft_io::export(&d, "x.psd", &photocraft_io::ExportOptions::default()).unwrap();
        let file = photocraft_io::document_to_psd(&d).unwrap();
        assert_eq!(file.header.color_mode.as_u16(), 7);
        assert_eq!(file.header.channels, 4);
        // Photoshop stores ink as dark: a solid-ink sample is 0.
        let back = photocraft_io::import("x.psd", &out.bytes).unwrap().document;
        assert_eq!(back.mode, ColorMode::Multichannel);
        assert_eq!(back.depth, d.depth);
        assert!(back.layers.is_empty());
        assert_eq!(ink_names(&back), ink_names(&d));
        for (a, b) in back.channels.iter().zip(&d.channels) {
            assert_eq!(a.spot.map(|(c, _)| c.to_rgba8()), b.spot.map(|(c, _)| c.to_rgba8()));
            let (va, vb) = (a.surface.read_region(d.bounds()), b.surface.read_region(d.bounds()));
            for (x, y) in va.iter().zip(&vb) {
                assert!((x - y).abs() <= tol(depth), "{depth}");
            }
        }
    }
}

#[test]
fn psd_stores_ink_as_dark() {
    let mut s = session("grayscale", 8);
    s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
    s.execute("image.mode.multichannel", json!({})).unwrap();
    let file = photocraft_io::document_to_psd(doc(&s)).unwrap();
    let merged = file.decode_merged().unwrap();
    assert!(merged.iter().all(|&b| b == 0), "solid black ink is stored as 0");
}

#[test]
fn pcraft_round_trip() {
    let mut s = session("rgb", 16);
    s.execute("image.mode.multichannel", json!({})).unwrap();
    let d = doc(&s).clone();
    let bytes = photocraft_format::save_to_bytes(&d, &Default::default()).unwrap();
    let back = photocraft_format::load_from_bytes(&bytes).unwrap();
    assert_eq!(back.mode, ColorMode::Multichannel);
    assert_eq!(back.channels, d.channels);
    assert_eq!(photocraft_compose::flatten(&back), photocraft_compose::flatten(&d));
}

#[test]
fn flat_export_prints_the_inks() {
    let mut s = session("grayscale", 8);
    s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
    s.execute("image.mode.multichannel", json!({})).unwrap();
    let out = photocraft_io::export(doc(&s), "x.png", &photocraft_io::ExportOptions::default()).unwrap();
    let back = photocraft_io::import("x.png", &out.bytes).unwrap().document;
    let px = photocraft_compose::flatten(&back).get(3, 3);
    // Solid process black ink, not paper.
    assert!(px[0] < 0.3 && px[3] > 0.99, "{px:?}");
}

#[test]
fn deleting_one_process_channel_keeps_other_planes_and_is_one_edit() {
    let cases = [
        ("rgb", 8, &["Cyan", "Yellow"][..]),
        ("rgb", 16, &["Cyan", "Yellow"]),
        ("rgb", 32, &["Cyan", "Yellow"]),
        ("cmyk", 8, &["Cyan", "Yellow", "Black"]),
        ("cmyk", 16, &["Cyan", "Yellow", "Black"]),
        ("lab", 8, &["Alpha 1", "Alpha 3"]),
        ("lab", 16, &["Alpha 1", "Alpha 3"]),
    ];
    for (mode, depth, names) in cases {
        let mut s = session(mode, depth);
        s.execute("channel.new", json!({"name":"Saved mask", "fill":"white"})).unwrap();
        s.execute("select.rect", json!({"x":0, "y":0, "width":4, "height":4})).unwrap();
        s.execute("channel.newSpot", json!({"name":"Separate ink", "color":"#224466", "solidity":62})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("channel.setVisible", json!({"channel":1, "visible":false})).unwrap();
        let solo = mode != "rgb" || depth != 32;
        if !solo {
            s.execute("channel.setVisible", json!({"channel":{"color":2}, "visible":true})).unwrap();
        }
        let before = doc(&s).clone();
        assert_eq!(before.channels[1].surface.sample_channel(1, 1, 0), 1.0, "the preserved spot has a painted plate");
        let before_pixels = pixels(&s);
        let active = s.active().unwrap().active_layer;
        let history = s.active().unwrap().history.past_len();
        let before_view = s.active().unwrap().channel_view.clone();

        assert_eq!(s.execute("channel.delete", json!({"channel":{"color":1}})).unwrap(), Value::Null);
        let after = doc(&s);
        assert_eq!(after.mode, ColorMode::Multichannel);
        assert_eq!(after.depth, before.depth);
        assert!(after.layers.is_empty());
        assert_eq!(after.channels.iter().take(names.len()).map(|ink| ink.name.as_str()).collect::<Vec<_>>(), names);
        assert!(after.channels[..names.len()].iter().all(|ink| ink.spot.is_some()));
        assert_eq!(&after.channels[names.len()..], &before.channels);
        assert_retained_ink_samples(&before, after, &before_pixels, tol(depth));
        assert_eq!(s.active().unwrap().channel_view.target, ChannelTarget::Alpha(names.len()));
        assert_eq!(s.active().unwrap().channel_view.alpha_visible, [vec![false; names.len()], before_view.alpha_visible].concat());
        let color_hidden = if solo { vec![true; 3] } else { Vec::new() };
        assert_eq!(s.active().unwrap().channel_view.color_hidden, color_hidden, "keep solo alpha and discard partial native choices");
        assert_eq!(s.active().unwrap().history.past_len(), history + 1);
        if mode == "rgb" && depth == 16 {
            let bytes = photocraft_format::save_to_bytes(after, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
            assert_eq!(reopened.mode, ColorMode::Multichannel);
            assert_eq!(reopened.channels, after.channels);
        }

        assert!(s.undo());
        assert_eq!(doc(&s).mode, before.mode);
        assert_eq!(doc(&s).layers.iter().map(|layer| layer.id).collect::<Vec<_>>(), before.layers.iter().map(|layer| layer.id).collect::<Vec<_>>());
        assert_eq!(s.active().unwrap().active_layer, active);
        assert_eq!(pixels(&s), before_pixels);
        assert_eq!(doc(&s).channels, before.channels);
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
}

fn assert_retained_ink_samples(before: &Document, after: &Document, before_pixels: &[f32], tolerance: f32) {
    let retained = (0..before.mode.color_channels()).filter(|&component| component != 1);
    let stride = before.pixel_format().channels();
    for (ink, component) in after.channels.iter().zip(retained) {
        let samples = ink.surface.read_region(after.bounds());
        assert_eq!(samples.len(), before_pixels.len() / stride);
        for (&actual, px) in samples.iter().zip(before_pixels.chunks_exact(stride)) {
            let expected = if before.mode == ColorMode::Cmyk { px[component] } else { 1.0 - px[component] };
            assert!((actual - expected).abs() <= tolerance, "{:?}/{:?}, component {component}: {actual} vs {expected}", before.mode, before.depth);
        }
    }
}

#[test]
fn deleting_a_process_channel_requires_an_explicit_in_range_index() {
    let mut s = session("rgb", 8);
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.past_len();
    for channel in [json!(0), json!("composite"), json!("red"), json!({"color":3}), json!({"color":-1}), json!({"color":u64::MAX})] {
        assert!(s.execute("channel.delete", json!({"channel":channel})).is_err(), "{channel}");
        assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &before));
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
    assert!(s.is_enabled("channel.delete"), "the RGB document needs no saved alpha channels");
    s.execute("channel.target", json!({"channel":{"color":0}})).unwrap();
    assert_eq!(s.active().unwrap().channel_view.target, ChannelTarget::Color(0));
    s.execute("channel.delete", json!({"channel":{"color":0}})).unwrap();
    assert_eq!(ink_names(doc(&s)), ["Magenta", "Yellow"]);
    assert_eq!(s.active().unwrap().channel_view.target, ChannelTarget::Composite, "only the Multichannel view remains");
    assert!(s.active().unwrap().channel_view.color_hidden.is_empty());

    let mut gray = session("grayscale", 8);
    gray.execute("channel.new", json!({"name":"Gray mask"})).unwrap();
    assert!(gray.execute("channel.delete", json!({"channel":{"color":0}})).is_err());
    assert_eq!(doc(&gray).mode, ColorMode::Grayscale);
    assert_eq!(doc(&gray).channels[0].name, "Gray mask");
}

#[test]
fn deleting_a_process_channel_refuses_an_over_budget_canvas_atomically() {
    let mut s = crate::session();
    let area = photocraft_geom::Size::new(100_000, 100_000);
    s.add_document(Document::new("Sparse canvas", area, ColorMode::Rgb, SampleType::U8), None);
    let st = s.active().unwrap();
    let before = st.doc.clone();
    let revision = st.revision;
    let history = st.history.past_len();
    let error = s.execute("channel.delete", json!({"channel":{"color":1}})).unwrap_err().to_string();
    assert!(error.contains("working memory"), "{error}");
    let after = s.active().unwrap();
    assert!(std::sync::Arc::ptr_eq(&after.doc, &before));
    assert_eq!(after.revision, revision);
    assert_eq!(after.history.past_len(), history);
}
