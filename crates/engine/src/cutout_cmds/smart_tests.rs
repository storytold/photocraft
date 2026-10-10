//! Placed photos keep their embedded/linked contents and cached appearance when masked.

use super::*;
use photocraft_doc::LayerId;
use photocraft_geom::Rect;

fn placed(depth: u32) -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":200,"height":160,"depth":depth})).unwrap();
    s.edit("fixture", |doc, _| {
        let surface = doc.layers[0].surface_mut().unwrap();
        let mut pixels = Vec::new();
        for y in 0_i32..160 {
            for x in 0_i32..200 {
                let n = ((x * 13 + y * 7) % 17) as f32 / 255.0;
                let color = if (x - 100).pow(2) + (y - 80).pow(2) < 45 * 45 { [0.85 + n, 0.2 + n, 0.12 + n, 1.0] } else { [0.23 + n, 0.4 + n, 0.68 + n, 1.0] };
                pixels.extend_from_slice(&color[..surface.channels()]);
            }
        }
        surface.write_region(Rect::new(0, 0, 200, 160), &pixels);
        Ok(())
    })
    .unwrap();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    (s, id)
}

#[test]
fn smart_background_removal_preserves_contents_at_all_depths_and_undoes_once() {
    for depth in [8, 16, 32] {
        let (mut s, id) = placed(depth);
        let before = s.active().unwrap().doc.layer(id).unwrap().content.clone();
        let steps = s.active().unwrap().history.past_len();
        assert!(s.is_enabled(CMD));
        s.execute(CMD, json!({})).unwrap();
        let state = s.active().unwrap();
        let layer = state.doc.layer(id).unwrap();
        assert_eq!(layer.content, before, "depth {depth}: keep source, transform and cache");
        let mask = layer.mask.as_ref().unwrap();
        assert!(mask.value(100, 80) > 0.8, "depth {depth}: subject centre");
        assert!(mask.value(0, 0) < 0.2, "depth {depth}: background corner");
        assert_eq!(state.history.past_len(), steps + 1);
        s.execute("edit.undo", json!({})).unwrap();
        assert!(s.active().unwrap().doc.layer(id).unwrap().mask.is_none());
        assert_eq!(s.active().unwrap().doc.layer(id).unwrap().content, before);
        s.execute("edit.redo", json!({})).unwrap();
        assert!(s.active().unwrap().doc.layer(id).unwrap().mask.is_some());
        assert_eq!(s.active().unwrap().doc.layer(id).unwrap().content, before);
    }
}

#[test]
fn smart_locks_and_missing_cache_fail_without_an_edit() {
    let (mut s, id) = placed(8);
    s.edit("lock fixture", |doc, _| {
        doc.layer_mut(id).unwrap().locks.pixels = true;
        Ok(())
    })
    .unwrap();
    let before = s.active().unwrap().history.past_len();
    assert!(!s.is_enabled(CMD));
    assert!(s.execute(CMD, json!({})).is_err());
    assert_eq!(s.active().unwrap().history.past_len(), before);
    s.edit("cache fixture", |doc, _| {
        let layer = doc.layer_mut(id).unwrap();
        layer.locks.pixels = false;
        if let LayerContent::Smart(smart) = &mut layer.content {
            smart.cache = None;
        }
        Ok(())
    })
    .unwrap();
    let before = s.active().unwrap().history.past_len();
    assert!(!s.is_enabled(CMD));
    let error = s.execute(CMD, json!({})).unwrap_err().to_string();
    assert!(error.contains("rendered image"), "{error}");
    assert_eq!(s.active().unwrap().history.past_len(), before);
    assert!(s.active().unwrap().doc.layer(id).unwrap().mask.is_none());
}
