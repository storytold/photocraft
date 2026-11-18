use super::*;
use photocraft_color::SampleType;

fn session(mode: &str, depth: u64) -> Session {
    let mut s = Session::new();
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

/// The printed RGB result is tagged with the profile its inks mixed in (`profile`, else the working
/// RGB); a profile that can't be used is refused before the document changes; the
/// channel-for-channel conversion keeps its tag as it was.
#[test]
fn printed_rgb_result_is_tagged_and_a_bad_profile_changes_nothing() {
    let tag = |s: &Session| doc(s).icc_profile.as_ref().and_then(|b| craft_cms::Profile::parse(b).ok());
    let mut s = session("rgb", 8);
    s.execute("image.mode.multichannel", json!({})).unwrap();
    s.edit("drop", |doc, _| {
        doc.channels.truncate(2);
        Ok(())
    })
    .unwrap();
    // An RGB profile built on a lookup table has no tone curves to mix the inks in.
    let dir = tempfile::tempdir().unwrap();
    let lut = dir.path().join("lut-rgb.icc");
    std::fs::write(&lut, lut_rgb_profile().to_bytes().as_slice()).unwrap();
    let lut = lut.to_string_lossy().into_owned();
    assert!(craft_cms::Profile::parse(&std::fs::read(&lut).unwrap()).unwrap().a2b[0].is_some());
    for bad in [json!({"profile": "no-such-profile"}), json!({"profile": "coated-cmyk"}), json!({"profile": lut})] {
        assert!(s.execute("image.mode.rgb", bad.clone()).is_err(), "{bad}");
        assert_eq!(doc(&s).mode, ColorMode::Multichannel, "{bad}");
        assert_eq!(doc(&s).channels.len(), 2, "{bad}");
    }
    let r = s.execute("image.mode.rgb", json!({"profile": "adobe-rgb-compat"})).unwrap();
    assert_eq!(r["converted"], "appearance");
    assert!(tag(&s).unwrap().same_colors(craft_cms::Builtin::AdobeRgbCompat.profile()));
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("image.mode.rgb", json!({})).unwrap();
    assert!(tag(&s).unwrap().same_colors(&s.color.working(ColorMode::Rgb)));
    // A non-RGB target ignores `profile` as before.
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.execute("image.mode.grayscale", json!({"profile": "coated-cmyk"})).is_ok());

    let mut s = session("rgb", 8);
    s.execute("image.mode.multichannel", json!({})).unwrap();
    let before = doc(&s).icc_profile.clone();
    assert_eq!(s.execute("image.mode.rgb", json!({"profile": "adobe-rgb-compat"})).unwrap()["converted"], "channels");
    assert_eq!(doc(&s).icc_profile, before);
}

/// Adobe RGB (1998) with an A2B0 table (identity curves around a 2³ grid of the matrix's XYZ).
fn lut_rgb_profile() -> craft_cms::Profile {
    use craft_cms::Curve;
    use craft_cms::pipeline::Stage;
    use craft_cms::profile::{Lut, LutKind};
    let mut p = craft_cms::Builtin::AdobeRgbCompat.profile().clone();
    let m = p.matrix.unwrap();
    let mut data = Vec::new();
    for r in 0..2 {
        for g in 0..2 {
            for b in 0..2 {
                let rgb = [r as f64, g as f64, b as f64];
                data.extend((0..3).map(|i| ((0..3).map(|j| m[i][j] * rgb[j]).sum::<f64>() / 2.0) as f32));
            }
        }
    }
    let ident = || Stage::Curves(vec![Curve::Identity; 3]);
    let clut = craft_cms::Clut::new(vec![2, 2, 2], 3, data).unwrap();
    p.a2b[0] = Some(Lut { kind: LutKind::Lut16, inputs: 3, outputs: 3, stages: vec![ident(), Stage::Clut(clut), ident()] });
    p.description = "LUT RGB".into();
    p.with_encoded_bytes()
}

/// CMYK, Lab and Grayscale results are built from sRGB values, so their inks mix in sRGB light
/// whatever the working RGB; an RGB result mixes in the working RGB's light.
#[test]
fn only_an_rgb_result_mixes_in_the_working_rgb_light() {
    let converted = |working: &str, to: &str| {
        let mut s = session("rgb", 16);
        s.execute("edit.colorSettings", json!({"workingRgb": working})).unwrap();
        s.execute("image.mode.multichannel", json!({})).unwrap();
        s.edit("drop", |doc, _| {
            doc.channels.truncate(2);
            Ok(())
        })
        .unwrap();
        assert_eq!(s.execute(&format!("image.mode.{to}"), json!({})).unwrap()["converted"], "appearance");
        pixels(&s)
    };
    let close = |a: &[f32], b: &[f32]| a.iter().zip(b).all(|(x, y)| (x - y).abs() <= 1e-4);
    for to in ["cmyk", "lab", "grayscale"] {
        assert!(close(&converted("adobe-rgb-compat", to), &converted("srgb", to)), "{to}");
    }
    assert!(!close(&converted("adobe-rgb-compat", "rgb"), &converted("srgb", "rgb")));
}
