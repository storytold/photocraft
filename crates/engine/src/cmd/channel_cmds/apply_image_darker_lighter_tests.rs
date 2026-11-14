//! Apply Image with Darker Color and Lighter Color (#14478): the whole pixel comes from whichever of
//! target and source has the lower (higher) luminosity; one plane takes Darken (Lighten).
use super::*;

const W: u32 = 400;
const H: u32 = 300;

fn halves(s: &mut Session, left: &str, right: &str) {
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": W / 2, "height": H, "mode": "replace"})).unwrap();
    s.execute("edit.fill", json!({"color": left})).unwrap();
    s.execute("select.rect", json!({"x": W / 2, "y": 0, "width": W / 2, "height": H, "mode": "replace"})).unwrap();
    s.execute("edit.fill", json!({"color": right})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
}

/// The measured test documents: "target" (0) RGB 200/100/50 | 50/150/250 and "source" (1)
/// 128/128/128 | 255/255/255, with "target" active.
fn session(mode: &str, depth: u32) -> Session {
    let mut s = Session::new();
    for (left, right) in [("#c86432", "#3296fa"), ("#808080", "#ffffff")] {
        s.execute("file.new", json!({"width": W, "height": H, "mode": mode, "depth": depth})).unwrap();
        halves(&mut s, left, right);
    }
    s.set_active(0);
    s
}

fn px(s: &Session, x: i32, y: i32) -> Vec<f32> {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().pixel(x, y)
}

fn assert_rgb(s: &Session, x: i32, want: [u8; 3], what: &str) {
    let p = px(s, x, 50);
    let ok = p[..3].iter().zip(want).all(|(v, w)| (v - f32::from(w) / 255.0).abs() < 0.003);
    assert!(ok, "{what} at ({x},50): {p:?}, want {want:?}");
}

fn apply(s: &mut Session, blending: &str) -> Result<Value> {
    s.execute("image.applyImage", json!({"source": {"document": 1}, "blending": blending}))
}

#[test]
fn darker_and_lighter_color_take_the_whole_pixel_by_luminosity() {
    for depth in [8, 16, 32] {
        let mut s = session("rgb", depth);
        apply(&mut s, "darkerColor").unwrap();
        assert_rgb(&s, 50, [200, 100, 50], &format!("Darker Color, {depth} bit"));
        assert_rgb(&s, 300, [50, 150, 250], &format!("Darker Color, {depth} bit"));
        s.undo();
        apply(&mut s, "lighterColor").unwrap();
        assert_rgb(&s, 50, [128, 128, 128], &format!("Lighter Color, {depth} bit"));
        assert_rgb(&s, 300, [255, 255, 255], &format!("Lighter Color, {depth} bit"));
    }
}

#[test]
fn the_labels_parse_too() {
    let mut s = session("rgb", 8);
    apply(&mut s, "Darker Color").unwrap();
    assert_rgb(&s, 50, [200, 100, 50], "Darker Color");
    s.undo();
    apply(&mut s, "lighter_color").unwrap();
    assert_rgb(&s, 50, [128, 128, 128], "Lighter Color");
}

#[test]
fn a_single_colour_channel_takes_darken_and_lighten() {
    let mut s = session("rgb", 8);
    s.execute("channel.target", json!({"channel": "red"})).unwrap();
    // Red 200 vs 128: Darken gives 128 (Darker Color on the whole pixel would keep 200).
    apply(&mut s, "darkerColor").unwrap();
    assert_rgb(&s, 50, [128, 100, 50], "Darker Color onto Red");
    assert_rgb(&s, 300, [50, 150, 250], "Darker Color onto Red");
    s.undo();
    s.execute("channel.target", json!({"channel": "blue"})).unwrap();
    apply(&mut s, "lighterColor").unwrap();
    assert_rgb(&s, 50, [200, 100, 128], "Lighter Color onto Blue");
    assert_rgb(&s, 300, [50, 150, 255], "Lighter Color onto Blue");
}

#[test]
fn an_alpha_channel_and_a_layer_mask_take_darken_and_lighten() {
    let mut s = session("rgb", 8);
    let a = s.execute("channel.new", json!({"fill": "white"})).unwrap()["channel"].as_u64().unwrap() as usize;
    // Into an alpha channel: white onto the source's gray (Darken) is the gray.
    s.execute("image.applyImage", json!({"source": {"document": 1}, "blending": "darkerColor", "target": {"channel": a}})).unwrap();
    let alpha = |s: &Session, x| s.active().unwrap().doc.channels[a].surface.sample_channel(x, 50, 0);
    assert!((alpha(&s, 50) - 128.0 / 255.0).abs() < 0.003 && (alpha(&s, 300) - 1.0).abs() < 0.003);
    s.execute("channel.target", json!({"channel": "composite"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("layer.layerMask.hideAll", json!({})).unwrap();
    s.execute("channel.target", json!({"channel": "mask"})).unwrap();
    // Into a black layer mask: Lighten takes the source's gray and white.
    s.execute("image.applyImage", json!({"source": {"document": 1}, "blending": "lighterColor"})).unwrap();
    let d = s.active().unwrap();
    let m = &d.doc.layer(d.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface;
    assert!((m.sample_channel(50, 50, 0) - 128.0 / 255.0).abs() < 0.003, "{}", m.sample_channel(50, 50, 0));
    assert!((m.sample_channel(300, 50, 0) - 1.0).abs() < 0.003);
}

#[test]
fn a_grayscale_layer_takes_darken_and_lighten() {
    let mut s = Session::new();
    for color in ["#c8c8c8", "#808080"] {
        s.execute("file.new", json!({"width": 40, "height": 20, "mode": "grayscale"})).unwrap();
        s.execute("edit.fill", json!({"color": color})).unwrap();
    }
    s.set_active(0);
    apply(&mut s, "darkerColor").unwrap();
    assert!((px(&s, 5, 5)[0] - 128.0 / 255.0).abs() < 0.003, "{:?}", px(&s, 5, 5));
    s.undo();
    apply(&mut s, "lighterColor").unwrap();
    assert!((px(&s, 5, 5)[0] - 200.0 / 255.0).abs() < 0.003, "{:?}", px(&s, 5, 5));
}

#[test]
fn other_modes_and_refusals_are_unchanged() {
    let mut s = session("rgb", 8);
    // Darken still mixes the channels: min per channel.
    apply(&mut s, "darken").unwrap();
    assert_rgb(&s, 50, [128, 100, 50], "Darken");
    s.undo();
    for bad in ["hue", "luminosity", "dissolve", "darkerColour", ""] {
        assert!(apply(&mut s, bad).is_err(), "{bad} is refused");
    }
    assert_rgb(&s, 50, [200, 100, 50], "nothing changed");
    // Calculations keeps refusing them.
    for blending in ["darkerColor", "lighterColor"] {
        let e = s.execute("image.calculations", json!({"source1": {}, "source2": {}, "blending": blending})).unwrap_err();
        assert!(e.to_string().contains("unsupported blending"), "{e}");
    }
    assert!(s.active().unwrap().doc.channels.is_empty());
}

#[test]
fn a_cmyk_composite_target_is_refused_and_left_alone() {
    let mut s = session("cmyk", 8);
    let before = px(&s, 50, 50);
    assert!(apply(&mut s, "darkerColor").is_err());
    assert_eq!(px(&s, 50, 50), before);
}
