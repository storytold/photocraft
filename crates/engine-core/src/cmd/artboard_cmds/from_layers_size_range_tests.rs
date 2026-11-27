//! Artboard from Layers takes a width and height of 4 to 32000 px and clamps a scripted size
//! outside that range, as its dialog's fields do when you leave them (#14577). 0 or no size keeps
//! the selected layers' bounds on that axis.

use super::*;
use photocraft_geom::Rect;

/// 100 x 80 RGB at `depth` with a pixel layer "Red" holding a square at 10,10 to 30,30.
fn session(depth: u32) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 100, "height": 80, "depth": depth, "background": "white"})).unwrap();
    s.edit("Red", |doc, active| {
        let fmt = doc.pixel_format();
        let mut l = Layer::raster("Red", fmt);
        l.surface_mut().unwrap().fill_rect(Rect::new(10, 10, 30, 30), &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
        let id = doc.insert_above(None, l);
        *active = Some(id);
        Ok(id)
    })
    .unwrap();
    s
}

/// The new board's `[x, y, width, height]` and the canvas size after Artboard from Layers.
fn board(depth: u32, params: Value) -> ([i64; 4], (u32, u32)) {
    let mut s = session(depth);
    let out = s.execute("layer.new.artboardFromLayers", params.clone()).unwrap_or_else(|e| panic!("{params} at {depth} bit: {e}"));
    let rect: Vec<i64> = out["rect"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect();
    let doc = &s.active().unwrap().doc;
    (rect.try_into().unwrap(), (doc.size.width, doc.size.height))
}

#[test]
fn a_scripted_size_past_the_range_is_clamped_to_4_to_32000() {
    for depth in [8, 16, 32] {
        let (rect, canvas) = board(depth, json!({"width": 99999, "height": 1}));
        assert_eq!(rect, [10, 10, 32_000, 4], "99999 -> 32000, 1 -> 4 at {depth} bit");
        assert!(canvas.0 <= 32_100, "the canvas grows only to the clamped board at {depth} bit: {canvas:?}");
        let (rect, _) = board(depth, json!({"width": 2, "height": 50_000}));
        assert_eq!(rect, [10, 10, 4, 32_000], "2 -> 4, 50000 -> 32000 at {depth} bit");
    }
}

#[test]
fn sizes_at_the_ends_of_the_range_are_kept() {
    let (rect, _) = board(8, json!({"width": 4, "height": 32_000}));
    assert_eq!(rect, [10, 10, 4, 32_000]);
    let (rect, _) = board(8, json!({"width": 640, "height": 480}));
    assert_eq!(rect, [10, 10, 640, 480]);
}

#[test]
fn zero_or_no_size_keeps_the_layers_bounds() {
    for params in [json!({}), json!({"width": 0, "height": 0}), json!({"width": 0})] {
        let (rect, _) = board(8, params.clone());
        assert_eq!(rect, [10, 10, 20, 20], "{params}");
    }
    let (rect, _) = board(8, json!({"width": 0, "height": 1}));
    assert_eq!(rect, [10, 10, 20, 4], "0 keeps the width, 1 is clamped to 4");
}

#[test]
fn a_negative_size_is_still_refused() {
    let mut s = session(8);
    let err = s.execute("layer.new.artboardFromLayers", json!({"width": -5})).unwrap_err();
    assert!(err.to_string().contains("must not be negative"), "{err}");
}
