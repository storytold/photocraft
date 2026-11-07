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
    // The Color Fill still covers the new canvas.
    let d = &s.active().unwrap().doc;
    assert_eq!((d.size.width, d.size.height), (602, 402));
    let px = photocraft_compose::render(d, photocraft_geom::Rect::new(5, 5, 6, 6)).into_px();
    assert!((px[0][2] - 0.8).abs() < 0.02, "the fill's colour: {:?}", px[0]);
    // One undo restores the layers as they were.
    s.undo();
    assert!(matches!(content(&s, fill), LayerContent::Fill(_)));
    assert_eq!(shape_bounds(&s, shape), (200.0, 300.0, 350.0, 400.0));
}

#[test]
fn a_skewed_crop_still_rasterizes_them() {
    let (mut s, fill, shape) = doc();
    crop(&mut s, json!([[120, 49], [701, 60], [680, 451], [99, 430]]));
    assert!(matches!(content(&s, fill), LayerContent::Raster(_)));
    assert!(matches!(content(&s, shape), LayerContent::Raster(_)));
}
