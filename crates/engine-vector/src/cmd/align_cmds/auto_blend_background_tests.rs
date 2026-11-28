//! #14214: Auto-Blend Layers leaves a selected Background as the Background, without a mask.

use super::*;

/// The issue's grey scene with six coloured blocks and noise, multiplied by `gain`, written
/// into `id` over x `x0..x1` of a 400 x 300 canvas.
fn paint_scene(s: &mut Session, id: LayerId, x0: i32, x1: i32, gain: f32) {
    s.edit("scene", |doc, _| {
        let surf = doc.layer_mut(id).unwrap().surface_mut().unwrap();
        let blocks: [(Rect, [f32; 3]); 6] = [
            (Rect::new(30, 40, 90, 110), [0.9, 0.1, 0.1]),
            (Rect::new(120, 60, 190, 140), [0.1, 0.8, 0.2]),
            (Rect::new(200, 170, 260, 250), [0.1, 0.2, 0.9]),
            (Rect::new(250, 30, 320, 90), [0.9, 0.8, 0.1]),
            (Rect::new(300, 150, 370, 230), [0.8, 0.2, 0.8]),
            (Rect::new(60, 190, 140, 260), [0.1, 0.8, 0.8]),
        ];
        let mut seed = 7u64;
        for y in 0..300 {
            for x in x0..x1 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let noise = ((seed >> 40) as f32 / (1u64 << 24) as f32 - 0.5) * 0.06;
                let c = blocks.iter().find(|(r, _)| r.contains(x, y)).map_or([0.5; 3], |(_, c)| *c);
                let px = c.map(|v| ((v + noise) * gain).clamp(0.0, 1.0));
                surf.fill_rect(Rect::new(x, y, x + 1, y + 1), &[px[0], px[1], px[2], 1.0]);
            }
        }
        Ok(())
    })
    .unwrap();
}

/// The issue's document: Left (the scene at x 0..240) as the Background, its uncovered area
/// white, and Right (the scene at x 160..400, darkened) above it.
fn left_background_and_right() -> (Session, LayerId, LayerId) {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
    let bg = s.active().unwrap().doc.background().unwrap().id;
    paint_scene(&mut s, bg, 0, 240, 1.0);
    s.execute("layer.new.layer", json!({"name": "Right"})).unwrap();
    let right = s.active().unwrap().active_layer.unwrap();
    paint_scene(&mut s, right, 160, 400, 0.8);
    s.execute("layer.select", json!({"layer": bg.0})).unwrap();
    s.execute("layer.select", json!({"layer": right.0, "mode": "add"})).unwrap();
    (s, bg, right)
}

#[test]
fn a_panorama_keeps_the_background_unmasked() {
    let (mut s, bg, right) = left_background_and_right();
    let before = s.active().unwrap().history.past_len();
    s.execute("edit.autoBlendLayers", json!({"method": "panorama"})).unwrap();
    let st = s.active().unwrap();
    assert_eq!(st.history.past_len(), before + 1, "one history state");
    assert_eq!(st.history.undo_label(), Some("Auto-Blend Layers"));
    let doc = &st.doc;
    assert!(doc.is_background(bg), "the Background stays the Background");
    let b = doc.layer(bg).unwrap();
    assert_eq!(b.name, "Background");
    assert!(b.locks.transparency && b.locks.position, "still locked");
    assert!(b.mask.is_none(), "no layer mask");
    let m = doc.layer(right).unwrap().mask.as_ref().expect("Right gets a mask");
    let shown = (0..300).flat_map(|y| (0..400).map(move |x| (x, y))).filter(|&(x, y)| m.value(x, y) > 0.0).count();
    assert_eq!(shown, 0, "the mask hides Right completely (visible bounds 0,0,0,0)");
    s.undo();
    let doc = &s.active().unwrap().doc;
    assert!(doc.layer(right).unwrap().mask.is_none(), "undo removes Right's mask");
    assert!(doc.is_background(bg));
}

/// A Background in Stack Images is not measured: it still becomes "Layer 0" with a mask.
#[test]
fn a_stack_still_turns_the_background_into_a_layer() {
    let (mut s, bg, right) = left_background_and_right();
    s.execute("edit.autoBlendLayers", json!({"method": "stack", "seamlessTones": false})).unwrap();
    let doc = &s.active().unwrap().doc;
    assert!(!doc.is_background(bg));
    assert_eq!(doc.layer(bg).unwrap().name, "Layer 0");
    assert!(doc.layer(bg).unwrap().mask.is_some() && doc.layer(right).unwrap().mask.is_some());
}

/// A Background with two other layers is not measured either: it still becomes "Layer 0".
#[test]
fn three_layers_still_turn_the_background_into_a_layer() {
    let (mut s, bg, right) = left_background_and_right();
    s.execute("layer.new.layer", json!({"name": "Third"})).unwrap();
    let third = s.active().unwrap().active_layer.unwrap();
    paint_scene(&mut s, third, 80, 320, 0.9);
    s.execute("layer.select", json!({"layer": bg.0})).unwrap();
    for id in [right, third] {
        s.execute("layer.select", json!({"layer": id.0, "mode": "add"})).unwrap();
    }
    s.execute("edit.autoBlendLayers", json!({"method": "panorama"})).unwrap();
    let doc = &s.active().unwrap().doc;
    assert!(!doc.is_background(bg));
    assert!(doc.layer(bg).unwrap().mask.is_some());
}
