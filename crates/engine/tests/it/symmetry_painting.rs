//! Painting symmetry: strokes mirror across the axis, the live preview matches the commit, and
//! paint on the axis is laid once.

use photocraft_engine::Session;
use photocraft_engine::brush_cmds::LiveStroke;
use photocraft_paint::StrokePoint;
use serde_json::json;

#[test]
fn symmetry_paints_mirrored_pixels_and_disable_stops_it() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 80, "height": 60, "background": "white"})).unwrap();
    s.execute("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": false, "knots": [[40, 0], [40, 60]]}]}})).unwrap();
    s.execute("paint.symmetryFromPath", json!({"name": "work"})).unwrap();
    let params = json!({"points": [[20, 20], [30, 30]], "size": 5, "hardness": 1.0, "color": "#ff0000", "smoothing": 0.0});
    let mut live = LiveStroke::begin(&s, &json!({"points": [[20, 20]], "size": 5, "hardness": 1.0, "color": "#ff0000", "smoothing": 0.0})).unwrap();
    live.push(&[StrokePoint::new(30.0, 30.0, 1.0)]).unwrap();
    s.execute("paint.stroke", params).unwrap();
    let committed = &s.active().unwrap().doc;
    let id = s.active().unwrap().active_layer.unwrap();
    let surface = committed.layer(id).unwrap().surface().unwrap();
    assert!(surface.rgba(20, 20)[1] < 0.2);
    assert!(surface.rgba(60, 20)[1] < 0.2);
    let preview = live.doc.layer(id).unwrap().surface().unwrap();
    for (x, y) in [(20, 20), (60, 20), (30, 30), (50, 30), (40, 20)] {
        let expected = surface.rgba(x, y);
        let actual = preview.rgba(x, y);
        assert!((0..4).all(|i| (expected[i] - actual[i]).abs() < 0.02), "preview/commit at ({x},{y}): {actual:?} vs {expected:?}");
    }
    s.execute("paint.symmetryDisable", json!({})).unwrap();
    s.execute("paint.stroke", json!({"points": [[15, 35]], "size": 5, "hardness": 1.0, "color": "#0000ff"})).unwrap();
    assert!(s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().rgba(65, 35)[0] > 0.8, "white remains without a mirrored blue stroke");
}

#[test]
fn stroke_on_axis_is_not_painted_twice_at_half_opacity() {
    let mut plain = Session::new();
    plain.execute("file.new", json!({"width": 80, "height": 60, "background": "white"})).unwrap();
    let mut symmetric = Session::new();
    symmetric.execute("file.new", json!({"width": 80, "height": 60, "background": "white"})).unwrap();
    symmetric.execute("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": false, "knots": [[40, 0], [40, 60]]}]}})).unwrap();
    symmetric.execute("paint.symmetryFromPath", json!({"name": "work"})).unwrap();
    let params = json!({"points": [[40, 15], [40, 40]], "size": 8, "hardness": 1.0, "opacity": 0.5, "color": "#000000", "smoothing": 0.0});
    let mut preview =
        LiveStroke::begin(&symmetric, &json!({"points": [[40, 15]], "size": 8, "hardness": 1.0, "opacity": 0.5, "color": "#000000", "smoothing": 0.0}))
            .unwrap();
    preview.push(&[StrokePoint::new(40.0, 40.0, 1.0)]).unwrap();
    plain.execute("paint.stroke", params.clone()).unwrap();
    symmetric.execute("paint.stroke", params).unwrap();
    let pixels = |session: &Session| {
        let st = session.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(40, 28)
    };
    let expected = pixels(&plain);
    let actual = pixels(&symmetric);
    let preview_pixel = {
        let st = symmetric.active().unwrap();
        preview.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(40, 28)
    };
    assert!(expected[0] > 0.2 && expected[0] < 0.8, "half opacity should produce a gray stroke: {expected:?}");
    assert!((0..4).all(|i| (actual[i] - expected[i]).abs() < 0.02), "symmetry must not darken on-axis paint: {actual:?} vs {expected:?}");
    assert!((0..4).all(|i| (preview_pixel[i] - actual[i]).abs() < 0.02), "preview must match commit: {preview_pixel:?} vs {actual:?}");
}

#[test]
fn mixed_axis_and_off_axis_stroke_unions_coverage_once() {
    let mut plain = Session::new();
    plain.execute("file.new", json!({"width": 80, "height": 60, "background": "white"})).unwrap();
    let mut symmetric = Session::new();
    symmetric.execute("file.new", json!({"width": 80, "height": 60, "background": "white"})).unwrap();
    symmetric.execute("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": false, "knots": [[40, 0], [40, 60]]}]}})).unwrap();
    symmetric.execute("paint.symmetryFromPath", json!({"name": "work"})).unwrap();
    let params = json!({"points": [[40, 10], [40, 25], [30, 40]], "size": 7, "hardness": 1.0, "opacity": 0.5, "color": "#000000", "smoothing": 0.0});
    let preview = LiveStroke::begin(&symmetric, &params).unwrap();
    plain.execute("paint.stroke", params.clone()).unwrap();
    symmetric.execute("paint.stroke", params).unwrap();
    let pixel = |session: &Session, x, y| {
        let st = session.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)
    };
    let axis_plain = pixel(&plain, 40, 18);
    let axis_symmetric = pixel(&symmetric, 40, 18);
    assert!(
        (0..4).all(|i| (axis_plain[i] - axis_symmetric[i]).abs() < 0.02),
        "shared axis must have one opacity ceiling: {axis_plain:?} vs {axis_symmetric:?}"
    );
    assert!(pixel(&symmetric, 50, 40)[0] < 0.9, "the off-axis part must be mirrored");
    let st = symmetric.active().unwrap();
    let preview_surface = preview.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    for (x, y) in [(40, 18), (30, 40), (50, 40)] {
        let expected = pixel(&symmetric, x, y);
        let actual = preview_surface.rgba(x, y);
        assert!((0..4).all(|i| (expected[i] - actual[i]).abs() < 0.02), "preview/commit at ({x},{y}): {actual:?} vs {expected:?}");
    }
}
