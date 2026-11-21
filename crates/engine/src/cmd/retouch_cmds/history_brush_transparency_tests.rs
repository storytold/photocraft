//! #14701: the History Brush in Normal mode restores the source state's pixels, transparency
//! included, instead of laying them over what is there now. Painting a moved subject's new place
//! from the state before the move clears it again (no "ghost" double image), at the stroke's
//! opacity, and the live stroke shows what the commit paints. Other modes still composite.

use serde_json::json;

use super::*;

const W: i32 = 48;
const H: i32 = 16;
const RED: [f32; 4] = [0.8, 0.1, 0.1, 1.0];

/// A `depth` RGB document with a layer over the Background holding a red subject on its left
/// half (History row 2), then the subject moved to the right half (row 3, the current state).
fn moved(depth: u64) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": W, "height": H, "depth": depth, "mode": "rgb"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    for left in [true, false] {
        s.edit("setup", |doc, active| {
            let b = doc.bounds();
            let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            let fmt = surf.format();
            let data: Vec<f32> = (b.y0..b.y1)
                .flat_map(|y| (b.x0..b.x1).map(move |x| (x, y)))
                .flat_map(|(x, _)| photocraft_raster::from_rgba(&fmt, if (x < W / 2) == left { RED } else { [0.0; 4] }))
                .collect();
            surf.write_region(b, &data);
            Ok(())
        })
        .unwrap();
    }
    s
}

/// A stroke across the whole layer from History row 2 (the subject still on the left).
fn stroke(extra: Value) -> Value {
    let mut p = json!({"points": [[0, 8], [W, 8]], "size": 40, "hardness": 100, "state": 2});
    if let (Some(p), Some(extra)) = (p.as_object_mut(), extra.as_object()) {
        p.extend(extra.clone());
    }
    p
}

fn layer_rgba(doc: &Document, s: &Session, x: i32, y: i32) -> [f32; 4] {
    doc.layer(s.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)
}

fn close(a: [f32; 4], b: [f32; 4], tol: f32) -> bool {
    a.iter().zip(b).all(|(a, b)| (a - b).abs() <= tol)
}

#[test]
fn history_brush_restores_transparency_from_the_source_state() {
    for depth in [8u64, 16, 32] {
        let mut s = moved(depth);
        s.execute("paint.historyBrush", stroke(json!({}))).unwrap();
        let doc = s.active().unwrap().doc.clone();
        let (left, right) = (layer_rgba(&doc, &s, 8, 8), layer_rgba(&doc, &s, 40, 8));
        assert!(close(left, RED, 0.01), "depth {depth}: the subject comes back on the left: {left:?}");
        assert!(right[3] < 0.01, "depth {depth}: the moved subject is cleared, no ghost: {right:?}");
    }
}

#[test]
fn history_brush_restores_transparency_at_the_stroke_opacity() {
    for depth in [8u64, 16, 32] {
        let mut s = moved(depth);
        s.execute("paint.historyBrush", stroke(json!({"opacity": 50}))).unwrap();
        let doc = s.active().unwrap().doc.clone();
        let (left, right) = (layer_rgba(&doc, &s, 8, 8), layer_rgba(&doc, &s, 40, 8));
        assert!((left[3] - 0.5).abs() < 0.01 && close([left[0], left[1], left[2], 1.0], RED, 0.01), "depth {depth}: half the subject back: {left:?}");
        assert!((right[3] - 0.5).abs() < 0.01 && close([right[0], right[1], right[2], 1.0], RED, 0.01), "depth {depth}: half cleared: {right:?}");
    }
}

#[test]
fn live_history_brush_shows_the_restored_transparency() {
    for depth in [8u64, 32] {
        let s = moved(depth);
        let live = super::super::LiveRetouch::begin(&s, "paint.historyBrush", &stroke(json!({}))).unwrap();
        let (left, right) = (layer_rgba(&live.doc, &s, 8, 8), layer_rgba(&live.doc, &s, 40, 8));
        assert!(close(left, RED, 0.01), "depth {depth}: {left:?}");
        assert!(right[3] < 0.01, "depth {depth}: the live stroke clears the moved subject: {right:?}");
    }
}

#[test]
fn other_modes_and_locked_transparency_keep_compositing() {
    // Multiply of a transparent source changes nothing: the moved subject stays.
    let mut s = moved(8);
    s.execute("paint.historyBrush", stroke(json!({"mode": "multiply"}))).unwrap();
    let doc = s.active().unwrap().doc.clone();
    assert!(close(layer_rgba(&doc, &s, 40, 8), RED, 0.01));
    // Locked transparency keeps the layer's alpha: nothing to clear, nothing to restore.
    let mut s = moved(8);
    s.edit("lock", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().locks.transparency = true;
        Ok(())
    })
    .unwrap();
    s.execute("paint.historyBrush", stroke(json!({}))).unwrap();
    let doc = s.active().unwrap().doc.clone();
    assert!(close(layer_rgba(&doc, &s, 40, 8), RED, 0.01));
    assert!(layer_rgba(&doc, &s, 8, 8)[3] < 0.01);
}
