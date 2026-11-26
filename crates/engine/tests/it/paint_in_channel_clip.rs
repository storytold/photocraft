//! Tests that run paint commands, moved from the engine-core `channel_clip` tests: 2 test(s).

use photocraft_engine::Session;
use serde_json::json;

const W: i32 = 40;

const H: i32 = 20;

fn px(s: &Session, x: i32, y: i32) -> Vec<f32> {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().pixel(x, y)
}

fn near(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3)
}

#[test]
fn cmyk_channels_copy_and_paste_as_tone() {
    // #5787: copying a CMYK channel gives its tone (white where there is no ink), and pasting a
    // tone into a CMYK channel writes it back as ink.
    for depth in [8, 16] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": W, "height": H, "mode": "cmyk", "depth": depth, "background": "white"})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": W / 2, "height": H})).unwrap();
        s.execute("edit.fill", json!({"color": "#00ffff"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        let (left, right) = (px(&s, 5, 5), px(&s, 30, 5));
        s.execute("channel.target", json!({"channel": "cyan"})).unwrap();
        s.execute("edit.copy", json!({})).unwrap();
        let clip = &s.clipboard.as_ref().unwrap().surface;
        assert!((clip.pixel(30, 5)[0] - 1.0).abs() < 1e-3, "depth {depth}: no ink copies as white");
        assert!((clip.pixel(5, 5)[0] - (1.0 - left[0])).abs() < 1e-3, "depth {depth}: ink copies dark");
        // Cyan pasted into Black: black gets cyan's ink, and white stays white.
        s.execute("channel.target", json!({"channel": "black"})).unwrap();
        s.execute("edit.paste", json!({})).unwrap();
        assert!(near(&px(&s, 30, 5), &right), "depth {depth}: no ink stays no ink, {:?}", px(&s, 30, 5));
        let p = px(&s, 5, 5);
        assert!((p[3] - left[0]).abs() < 2e-3 && near(&p[..3], &left[..3]), "depth {depth}: black = old cyan, {p:?} vs {left:?}");
    }
}

#[test]
fn spot_channels_copy_and_paste_as_tone() {
    // #5787: a spot channel stores ink but copies as its tone, and a tone pasted into it goes back
    // to ink: spot -> Black prints the spot's ink in black, Cyan -> spot gives the spot Cyan's ink.
    for depth in [8, 16] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": W, "height": H, "mode": "cmyk", "depth": depth, "background": "white"})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": W / 2, "height": H})).unwrap();
        s.execute("edit.fill", json!({"color": "#00ffff"})).unwrap();
        let cyan = px(&s, 5, 5)[0];
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 10, "height": H})).unwrap();
        let spot = s.execute("channel.newSpot", json!({"color": "#ff00ff", "solidity": 100})).unwrap()["channel"].as_u64().unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("channel.target", json!({"channel": spot})).unwrap();
        s.execute("edit.copy", json!({})).unwrap();
        let clip = &s.clipboard.as_ref().unwrap().surface;
        assert!(clip.pixel(5, 5)[0] < 1e-3 && (clip.pixel(30, 5)[0] - 1.0).abs() < 1e-3, "depth {depth}: ink copies dark");
        let expected = clip.clone();
        // An explicit spot target must keep the same tone while multiple colour rows are selected.
        s.execute("channel.target", json!({"colors": [0, 2]})).unwrap();
        s.execute("edit.copy", json!({"target": {"channel": spot}})).unwrap();
        assert_eq!(s.clipboard.as_ref().unwrap().surface, expected, "depth {depth}: explicit spot target overrides the colour set");
        s.execute("channel.target", json!({"channel": "black"})).unwrap();
        s.execute("edit.paste", json!({})).unwrap();
        assert!((px(&s, 5, 5)[3] - 1.0).abs() < 2e-3 && px(&s, 30, 5)[3] < 2e-3, "depth {depth}: black = the spot's ink");
        s.execute("channel.target", json!({"channel": "cyan"})).unwrap();
        s.execute("edit.copy", json!({})).unwrap();
        s.execute("channel.target", json!({"channel": spot})).unwrap();
        s.execute("edit.paste", json!({})).unwrap();
        let ink = |x| s.active().unwrap().doc.channels[spot as usize].surface.sample_channel(x, 5, 0);
        assert!(near(&[ink(5), ink(15), ink(30)], &[cyan, cyan, 0.0]), "depth {depth}: spot = Cyan's ink, {:?}", [ink(5), ink(15), ink(30)]);
    }
}
