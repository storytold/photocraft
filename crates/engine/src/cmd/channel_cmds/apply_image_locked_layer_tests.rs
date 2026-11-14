//! Apply Image on a fully locked or pixel-locked layer: it applies, and the layer keeps its
//! transparency as with Preserve transparency (#14476).

use super::*;

/// The spec's test document "target" (400 x 300): layer "Top" is empty except a 100 x 100 square
/// at (150,100) of 0/255/0 at alpha 128. Document "source" is 128/128/128 on the left half and
/// white on the right half.
fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300, "depth": depth, "name": "target"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Top"})).unwrap();
    s.execute("select.rect", json!({"x": 150, "y": 100, "width": 100, "height": 100, "mode": "replace"})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#00ff00", "opacity": 128.0 / 255.0 * 100.0})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("file.new", json!({"width": 400, "height": 300, "depth": depth, "name": "source"})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#ffffff"})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 200, "height": 300, "mode": "replace"})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#808080"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.set_active(0);
    s
}

fn apply() -> Value {
    json!({"source": {"document": 1}, "blending": "normal", "opacity": 100})
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

fn lock(s: &mut Session, locks: &Value) {
    s.execute("layer.setProps", json!({"locks": locks})).unwrap();
}

const LOCKS: [&str; 2] = [r#"{"all": true}"#, r#"{"pixels": true}"#];

fn locks() -> impl Iterator<Item = Value> {
    LOCKS.iter().map(|l| serde_json::from_str(l).unwrap())
}

#[test]
fn a_locked_layer_takes_the_image_in_one_step_and_keeps_its_transparency() {
    for depth in [8, 16, 32] {
        for locks in locks() {
            let mut s = session(depth);
            lock(&mut s, &locks);
            let before = px(&s, 175, 125);
            let steps = s.active().unwrap().history.past_len();
            s.execute("image.applyImage", apply()).unwrap();
            let d = s.active().unwrap();
            assert_eq!(d.history.past_len(), steps + 1, "{depth} bit {locks}: one history step");
            assert_eq!(d.history.undo_label(), Some("Apply Image"), "{depth} bit {locks}");
            assert!(!close(&px(&s, 175, 125), &before), "{depth} bit {locks}: (175,125) changes");
            assert_eq!(px(&s, 50, 50), vec![0.0; 4], "{depth} bit {locks}: (50,50) stays transparent");
            assert!((px(&s, 175, 125)[3] - before[3]).abs() < 1e-5, "{depth} bit {locks}: the square keeps its alpha");
        }
    }
}

#[test]
fn a_locked_layer_gets_the_preserve_transparency_result() {
    for depth in [8, 16, 32] {
        for extra in [json!({}), json!({"blending": "multiply"}), json!({"opacity": 50})] {
            let with = |p: Value| {
                let mut p = p;
                for (k, v) in extra.as_object().unwrap() {
                    p[k] = v.clone();
                }
                p
            };
            let mut want = session(depth);
            let mut p = with(apply());
            p["preserveTransparency"] = json!(true);
            want.execute("image.applyImage", p).unwrap();
            for locks in locks() {
                let mut s = session(depth);
                lock(&mut s, &locks);
                s.execute("image.applyImage", with(apply())).unwrap();
                assert!(close(&pixels(&s), &pixels(&want)), "{depth} bit {locks} {extra}");
            }
        }
    }
}

#[test]
fn the_locks_stay_and_other_commands_keep_refusing() {
    for locks in locks() {
        let mut s = session(8);
        lock(&mut s, &locks);
        let set = layer(&s).locks;
        s.execute("image.applyImage", apply()).unwrap();
        assert_eq!(layer(&s).locks, set, "{locks}: the lock flags are unchanged");
        let after = pixels(&s);
        let steps = s.active().unwrap().history.past_len();
        let fill = s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap_err().to_string();
        assert!(fill.contains("is locked"), "{locks}: Fill refuses: {fill}");
        let stroke = s.execute("paint.stroke", json!({"points": [[160, 110], [240, 190]], "size": 20})).unwrap_err().to_string();
        assert!(stroke.contains("is locked"), "{locks}: a brush stroke refuses: {stroke}");
        assert!(close(&pixels(&s), &after), "{locks}: nothing else painted");
        assert_eq!(s.active().unwrap().history.past_len(), steps, "{locks}: no more steps");
        s.undo();
        assert!(close(&pixels(&s), &pixels(&session(8))), "{locks}: undo restores the layer");
    }
}

#[test]
fn a_lock_from_a_group_still_refuses() {
    // Not measured in the reference application: keeps today's refusal, also when the layer is
    // locked itself as well.
    for own in [None, Some(json!({"pixels": true})), Some(json!({"all": true}))] {
        let mut s = session(8);
        let top = s.active().unwrap().active_layer.unwrap();
        if let Some(own) = &own {
            lock(&mut s, own);
        }
        s.execute("layer.new.groupFromLayers", json!({})).unwrap();
        lock(&mut s, &json!({"all": true}));
        s.set_active_layer(Some(top));
        let before = pixels(&s);
        let err = s.execute("image.applyImage", apply()).unwrap_err().to_string();
        assert!(err.contains("is locked"), "{own:?}: {err}");
        assert!(close(&pixels(&s), &before), "{own:?}");
    }
}

#[test]
fn an_unlocked_layer_keeps_source_over() {
    let mut s = session(8);
    s.execute("image.applyImage", apply()).unwrap();
    let p = px(&s, 50, 50);
    let g = 128.0 / 255.0;
    assert!(p[..3].iter().all(|v| (v - g).abs() < 0.003) && (p[3] - 1.0).abs() < 1e-5, "{p:?}");
}
