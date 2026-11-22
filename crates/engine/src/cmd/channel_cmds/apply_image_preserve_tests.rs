//! #14473: Apply Image with Preserve Transparency gives a semi-transparent pixel the full blend
//! result and keeps its alpha, as the reference application does.

use crate::Session;
use serde_json::{Value, json};

/// Target "target" (document 0): white Background, layer "Top" transparent except a 0/255/0
/// square at alpha 128 over (10,4)-(19,13). Source "source" (document 1): 128/128/128.
fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 20, "depth": depth, "name": "target"})).unwrap();
    s.execute("file.new", json!({"width": 40, "height": 20, "depth": depth, "name": "source"})).unwrap();
    s.execute("edit.fill", json!({"color": "#808080"})).unwrap();
    s.set_active(0);
    s.execute("layer.new.layer", json!({"name": "Top"})).unwrap();
    s.execute("select.rect", json!({"x": 10, "y": 4, "width": 10, "height": 10, "mode": "replace"})).unwrap();
    s.execute("edit.fill", json!({"color": "#00ff00", "opacity": 50})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s
}

fn px(s: &Session, x: i32, y: i32) -> Vec<f32> {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().pixel(x, y)
}

fn apply(s: &mut Session, extra: Value) {
    let mut p = json!({"source": {"document": 1, "layer": "merged", "channel": "composite"}});
    if let (Some(p), Some(extra)) = (p.as_object_mut(), extra.as_object()) {
        p.extend(extra.clone());
    }
    s.execute("image.applyImage", p).unwrap();
}

/// Checks RGBA in 0..=255 levels, one level of slack (the fill's 50% alpha is 127.5).
fn assert_rgba(got: &[f32], want: [f32; 4], what: &str) {
    let got: Vec<f32> = got.iter().map(|v| v * 255.0).collect();
    assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 1.01), "{what}: {got:?}, want {want:?}");
}

const GRAY: f32 = 128.0;
const HALF: f32 = 127.5;

#[test]
fn normal_with_preserve_transparency_takes_the_source_and_keeps_alpha() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        apply(&mut s, json!({"blending": "normal", "preserveTransparency": true}));
        assert_rgba(&px(&s, 15, 8), [GRAY, GRAY, GRAY, HALF], &format!("depth {depth}: green pixel"));
        assert_rgba(&px(&s, 2, 2), [0.0; 4], &format!("depth {depth}: transparent pixel"));
    }
}

#[test]
fn multiply_with_preserve_transparency_blends_fully_and_keeps_alpha() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        apply(&mut s, json!({"blending": "multiply", "preserveTransparency": true}));
        assert_rgba(&px(&s, 15, 8), [0.0, GRAY, 0.0, HALF], &format!("depth {depth}: green pixel"));
        assert_rgba(&px(&s, 2, 2), [0.0; 4], &format!("depth {depth}: transparent pixel"));
    }
}

#[test]
fn opacity_and_selection_still_scale_the_preserved_blend() {
    let mut s = session(8);
    // 50% Normal: the colour moves half way to the source, the alpha stays.
    apply(&mut s, json!({"blending": "normal", "opacity": 50, "preserveTransparency": true}));
    assert_rgba(&px(&s, 15, 8), [64.0, 191.5, 64.0, HALF], "opacity 50");
    s.undo();
    // Outside the selection nothing changes.
    s.execute("select.rect", json!({"x": 10, "y": 4, "width": 5, "height": 10, "mode": "replace"})).unwrap();
    apply(&mut s, json!({"blending": "normal", "preserveTransparency": true}));
    assert_rgba(&px(&s, 12, 8), [GRAY, GRAY, GRAY, HALF], "inside the selection");
    assert_rgba(&px(&s, 17, 8), [0.0, 255.0, 0.0, HALF], "outside the selection");
}

#[test]
fn a_mask_still_scales_the_preserved_blend() {
    let mut s = session(8);
    // A 50% gray alpha channel as the mask halves the effect, like 50% opacity.
    s.execute("channel.new", json!({"fill": "white"})).unwrap();
    s.execute("channel.target", json!({"channel": 0})).unwrap();
    s.execute("edit.fill", json!({"color": "#808080"})).unwrap();
    s.execute("channel.target", json!({"channel": "composite"})).unwrap();
    apply(&mut s, json!({"blending": "normal", "preserveTransparency": true, "mask": {"document": 0, "channel": 0}}));
    assert_rgba(&px(&s, 15, 8), [64.0, 191.5, 64.0, HALF], "mask at 50%");
}

#[test]
fn without_preserve_transparency_the_results_stay() {
    let mut s = session(8);
    apply(&mut s, json!({"blending": "normal"}));
    assert_rgba(&px(&s, 15, 8), [GRAY, GRAY, GRAY, 255.0], "normal, green pixel");
    assert_rgba(&px(&s, 2, 2), [GRAY, GRAY, GRAY, 255.0], "normal, transparent pixel");
    s.undo();
    apply(&mut s, json!({"blending": "multiply"}));
    assert_rgba(&px(&s, 15, 8), [64.0, GRAY, 64.0, 255.0], "multiply, green pixel");
    assert_rgba(&px(&s, 2, 2), [GRAY, GRAY, GRAY, 255.0], "multiply, transparent pixel");
    s.undo();
    apply(&mut s, json!({"blending": "normal", "opacity": 50}));
    assert_rgba(&px(&s, 15, 8), [85.0, 170.0, 85.0, 191.5], "normal 50%, green pixel");
    assert_rgba(&px(&s, 2, 2), [GRAY, GRAY, GRAY, HALF], "normal 50%, transparent pixel");
}
