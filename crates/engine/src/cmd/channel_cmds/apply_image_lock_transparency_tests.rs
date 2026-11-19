//! Apply Image on a layer with Lock Transparent Pixels: the lock acts as Preserve Transparency.

use super::*;

/// A 40 x 20 document whose layer "Top" is empty except a half-transparent green 10 x 10 square
/// at (10,5), and a second document "source" filled with 50% gray.
fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 20, "depth": depth})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Top"})).unwrap();
    s.execute("select.rect", json!({"x": 10, "y": 5, "width": 10, "height": 10, "mode": "replace"})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#00ff00", "opacity": 50})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("file.new", json!({"width": 40, "height": 20, "depth": depth, "name": "source"})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#808080"})).unwrap();
    s.set_active(0);
    s
}

fn layer(s: &Session) -> &Layer {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap()
}

fn pixels(s: &Session) -> Vec<f32> {
    let d = s.active().unwrap();
    layer(s).surface().unwrap().read_region(d.doc.bounds())
}

fn px(s: &Session, x: i32, y: i32) -> Vec<f32> {
    layer(s).surface().unwrap().pixel(x, y)
}

fn close(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
}

/// The layer's pixels after Apply Image with `extra` params, unlocked and with Preserve transparency.
fn preserved(depth: u32, extra: &Value) -> Vec<f32> {
    let mut s = session(depth);
    let mut p = json!({"source": {"document": 1}, "blending": "normal", "preserveTransparency": true});
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    s.execute("image.applyImage", p).unwrap();
    pixels(&s)
}

#[test]
fn transparency_lock_acts_as_preserve_transparency() {
    for depth in [8, 16, 32] {
        for extra in [json!({}), json!({"blending": "multiply"}), json!({"opacity": 50})] {
            let want = preserved(depth, &extra);
            let mut s = session(depth);
            s.execute("layer.lockLayers", json!({"transparency": true})).unwrap();
            let mut p = json!({"source": {"document": 1}, "blending": "normal"});
            for (k, v) in extra.as_object().unwrap() {
                p[k] = v.clone();
            }
            s.execute("image.applyImage", p).unwrap();
            assert!(close(&pixels(&s), &want), "{depth} bit {extra}: the lock must keep the layer's transparency");
            assert_eq!(px(&s, 30, 2), vec![0.0; 4], "{depth} bit {extra}: a transparent pixel stays transparent");
            assert!((px(&s, 15, 10)[3] - px(&session(depth), 15, 10)[3]).abs() < 1e-5, "{depth} bit {extra}: the square keeps its alpha");
        }
    }
}

#[test]
fn transparency_lock_inside_a_selection_acts_as_preserve_transparency() {
    let sel = |s: &mut Session| s.execute("select.rect", json!({"x": 0, "y": 0, "width": 15, "height": 20, "mode": "replace"})).unwrap();
    let mut want = session(8);
    sel(&mut want);
    want.execute("image.applyImage", json!({"source": {"document": 1}, "preserveTransparency": true})).unwrap();
    let mut s = session(8);
    s.execute("layer.lockLayers", json!({"transparency": true})).unwrap();
    sel(&mut s);
    s.execute("image.applyImage", json!({"source": {"document": 1}})).unwrap();
    assert!(close(&pixels(&s), &pixels(&want)));
    assert_eq!(px(&s, 2, 2), vec![0.0; 4]);
}

#[test]
fn lock_stays_on_and_one_history_step_is_added() {
    let mut s = session(8);
    s.execute("layer.lockLayers", json!({"transparency": true})).unwrap();
    let steps = s.active().unwrap().history.past_len();
    s.execute("image.applyImage", json!({"source": {"document": 1}})).unwrap();
    assert!(layer(&s).locks.transparency, "the lock stays on");
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "one Apply Image step");
    s.undo();
    assert!(close(&pixels(&s), &pixels(&session(8))), "undo restores the layer");
}

#[test]
fn unlocked_layer_keeps_source_over() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        s.execute("image.applyImage", json!({"source": {"document": 1}, "blending": "normal"})).unwrap();
        let g = 128.0 / 255.0;
        let p = px(&s, 30, 2);
        assert!(p[..3].iter().all(|v| (v - g).abs() < 0.003) && (p[3] - 1.0).abs() < 1e-5, "{depth} bit: {p:?}");
    }
}
