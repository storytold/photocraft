//! #11800: a Perspective Crop with a rectangular quad keeps shape and Color Fill layers as they
//! are (a shape moves with the content), in one undo step; any other quad still rasterizes them.

use photocraft_doc::{LayerContent, LayerId};
use serde_json::json;

use crate::Session;

/// 800 × 500 with a Levels adjustment, a Color Fill and a rectangle shape at 200,300–350,400.
fn doc() -> (Session, LayerId, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 500})).unwrap();
    s.execute("layer.newAdjustmentLayer.levels", json!({})).unwrap();
    s.execute("layer.newFillLayer.solidColor", json!({"color": "#3366cc"})).unwrap();
    let fill = s.active().unwrap().active_layer.unwrap();
    s.execute("shape.create", json!({"kind": "rect", "rect": [200, 300, 150, 100], "fill": "#cc3333"})).unwrap();
    let shape = s.active().unwrap().active_layer.unwrap();
    (s, fill, shape)
}

fn crop(s: &mut Session, corners: serde_json::Value) {
    s.execute("image.perspectiveCrop", json!({"corners": corners, "width": 602, "height": 402})).unwrap();
}

fn crop_to(s: &mut Session, corners: serde_json::Value, w: u32, h: u32) {
    s.execute("image.perspectiveCrop", json!({"corners": corners, "width": w, "height": h})).unwrap();
}

fn past(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

fn content(s: &Session, id: LayerId) -> LayerContent {
    s.active().unwrap().doc.layer(id).unwrap().content.clone()
}

fn shape_bounds(s: &Session, id: LayerId) -> (f64, f64, f64, f64) {
    match content(s, id) {
        LayerContent::Shape(sh) => sh.path.control_bounds().expect("a path"),
        other => panic!("not a shape: {other:?}"),
    }
}

#[test]
fn a_rectangular_crop_keeps_shape_and_fill_layers() {
    let (mut s, fill, shape) = doc();
    let before = s.active().unwrap().history.past_len();
    crop(&mut s, json!([[99, 49], [701, 49], [701, 451], [99, 451]]));
    assert_eq!(s.active().unwrap().history.past_len(), before + 1, "one step");
    assert!(matches!(content(&s, fill), LayerContent::Fill(_)), "still a fill layer");
    let (x0, y0, x1, y1) = shape_bounds(&s, shape);
    for (got, want) in [(x0, 101.0), (y0, 251.0), (x1, 251.0), (y1, 351.0)] {
        assert!((got - want).abs() <= 1.0, "shape bounds {:?}", (x0, y0, x1, y1));
    }
    // The Color Fill still covers the new canvas, to its corners.
    let d = &s.active().unwrap().doc;
    assert_eq!((d.size.width, d.size.height), (602, 402));
    for (x, y) in [(0, 0), (601, 0), (0, 401), (601, 401), (5, 5), (300, 20)] {
        let px = photocraft_compose::render(d, photocraft_geom::Rect::new(x, y, x + 1, y + 1)).into_px();
        assert!((px[0][2] - 0.8).abs() < 0.02, "the fill's colour at ({x}, {y}): {:?}", px[0]);
    }
    // One undo restores the layers as they were.
    s.undo();
    assert!(matches!(content(&s, fill), LayerContent::Fill(_)));
    assert_eq!(shape_bounds(&s, shape), (200.0, 300.0, 350.0, 400.0));
}

#[test]
fn a_skewed_crop_still_rasterizes_them() {
    let (mut s, fill, shape) = doc();
    let before = past(&s);
    crop(&mut s, json!([[120, 49], [701, 60], [680, 451], [99, 430]]));
    assert!(matches!(content(&s, fill), LayerContent::Raster(_)));
    assert!(matches!(content(&s, shape), LayerContent::Raster(_)));
    assert_eq!(past(&s), before + 1, "one step");
    s.undo();
    assert!(matches!(content(&s, fill), LayerContent::Fill(_)));
    assert_eq!(shape_bounds(&s, shape), (200.0, 300.0, 350.0, 400.0));
}

/// A rectangle at an angle maps affinely too: the shape stays a shape, its corners taken where the
/// rotated frame's axes take them.
#[test]
fn a_rotated_rectangle_keeps_shape_and_fill_layers() {
    let (mut s, fill, shape) = doc();
    // u = (400, 200), v = (-100, 200): perpendicular, |u| = 447.2, |v| = 223.6.
    let (tl, u, v) = ([200.0, 50.0], [400.0, 200.0], [-100.0, 200.0]);
    crop_to(&mut s, json!([tl, [600, 250], [500, 450], [100, 250]]), 447, 224);
    assert!(matches!(content(&s, fill), LayerContent::Fill(_)), "still a fill layer");
    let (lu, lv) = (f64::hypot(u[0], u[1]), f64::hypot(v[0], v[1]));
    let map = |p: [f64; 2]| {
        let d = [p[0] - tl[0], p[1] - tl[1]];
        ((d[0] * u[0] + d[1] * u[1]) / lu * 447.0 / lu, (d[0] * v[0] + d[1] * v[1]) / lv * 224.0 / lv)
    };
    let pts = [[200.0, 300.0], [350.0, 300.0], [350.0, 400.0], [200.0, 400.0]].map(map);
    let want = pts.iter().fold((f64::MAX, f64::MAX, f64::MIN, f64::MIN), |b, p| (b.0.min(p.0), b.1.min(p.1), b.2.max(p.0), b.3.max(p.1)));
    let got = shape_bounds(&s, shape);
    for (g, w) in [(got.0, want.0), (got.1, want.1), (got.2, want.2), (got.3, want.3)] {
        assert!((g - w).abs() <= 1.0, "shape bounds {got:?}, expected {want:?}");
    }
}

/// A small rectangle far from the origin still counts as one: the corners' rounding to f64 is
/// larger there than its sides' right angle allows without a scale-aware tolerance.
#[test]
fn a_rectangle_far_from_the_origin_keeps_the_fill_layer() {
    let (mut s, fill, _) = doc();
    let q = json!([[9999980.266, 9999980.347], [9999980.826, 9999980.787], [9999979.066, 9999983.027], [9999978.506, 9999982.587]]);
    s.execute("image.perspectiveCrop", json!({"corners": q})).unwrap();
    assert!(matches!(content(&s, fill), LayerContent::Fill(_)), "still a fill layer");
}

/// A type layer is rasterized by a rectangle crop; the shape and fill layers beside it are kept,
/// and it all stays one step.
#[test]
fn a_rectangular_crop_rasterizes_only_the_type_layer() {
    let (mut s, fill, shape) = doc();
    let text = LayerId(s.execute("type.create", json!({"x": 300, "y": 100, "text": "A", "size": 24})).unwrap()["layer"].as_u64().unwrap());
    let before = past(&s);
    crop(&mut s, json!([[99, 49], [701, 49], [701, 451], [99, 451]]));
    assert!(matches!(content(&s, text), LayerContent::Raster(_)), "type rasterized");
    assert!(matches!(content(&s, fill), LayerContent::Fill(_)), "still a fill layer");
    assert!(matches!(content(&s, shape), LayerContent::Shape(_)), "still a shape layer");
    assert_eq!(past(&s), before + 1, "one step");
    s.undo();
    assert!(matches!(content(&s, text), LayerContent::Text(_)));
}

/// A kept shape's stroke scales with the crop, as in Image Size: by the mean of the axis scales.
#[test]
fn a_scaled_rectangular_crop_scales_the_stroke_width() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 500})).unwrap();
    let id = LayerId(s.execute("shape.create", json!({"rect": [200, 300, 150, 100], "stroke": {"width": 4}})).unwrap()["layer"].as_u64().unwrap());
    // 602 x 402 onto 1204 x 402: x2 across, x1 down, mean 1.5.
    crop_to(&mut s, json!([[99, 49], [701, 49], [701, 451], [99, 451]]), 1204, 402);
    let LayerContent::Shape(sh) = content(&s, id) else { panic!("not a shape") };
    let w = sh.stroke.expect("a stroke").width;
    assert!((w - 6.0).abs() < 1e-3, "stroke width {w}");
}

/// A skewed crop with a shape layer rasterizes every layer's vector data, as Rasterize All Layers
/// does: a raster layer's vector mask becomes a pixel mask, so it follows the homography with the
/// pixels instead of an affine approximation.
#[test]
fn a_skewed_crop_rasterizes_vector_masks_too() {
    let (mut s, _, _) = doc();
    s.execute("layer.new.layer", json!({})).unwrap();
    let raster = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.vectorMask.revealAll", json!({})).unwrap();
    assert!(s.active().unwrap().doc.layer(raster).unwrap().vector_mask.is_some());
    crop(&mut s, json!([[120, 49], [701, 60], [680, 451], [99, 430]]));
    let l = s.active().unwrap().doc.layer(raster).unwrap();
    assert!(l.vector_mask.is_none() && l.mask.is_some(), "the vector mask became a pixel mask");
}
