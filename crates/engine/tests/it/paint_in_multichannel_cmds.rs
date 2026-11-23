//! Tests that run paint commands, moved from the engine-core `multichannel_cmds` tests: 4 test(s).

use photocraft_doc::Document;
use photocraft_engine::Session;
use serde_json::json;

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
    use photocraft_engine::channel_cmds::ChannelTarget;
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
fn psd_stores_ink_as_dark() {
    let mut s = session("grayscale", 8);
    s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
    s.execute("image.mode.multichannel", json!({})).unwrap();
    let file = photocraft_io::document_to_psd(doc(&s)).unwrap();
    let merged = file.decode_merged().unwrap();
    assert!(merged.iter().all(|&b| b == 0), "solid black ink is stored as 0");
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
