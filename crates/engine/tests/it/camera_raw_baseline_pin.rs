//! Pre-cutover baseline pins for `filter.cameraRaw` (pixel layer, smart filter, alpha, identity; #587 / #778).

use photocraft_doc::LayerContent;
use photocraft_engine::Session;
use serde_json::json;

fn active_pixel(s: &Session, x: i32, y: i32) -> [f32; 4] {
    let st = s.active().unwrap();
    let layer = st.active_layer.and_then(|id| st.doc.layer(id)).unwrap();
    layer.surface().unwrap().rgba(x, y)
}

#[test]
fn camera_raw_filter_preserves_identity_alpha_and_smart_filter_parity() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "depth": 16})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 16, "height": 16})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#4080c0", "opacity": 50.0})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();

    let before = active_pixel(&s, 8, 8);
    assert!((before[3] - 0.5).abs() < 0.02, "expected ~50% alpha: {before:?}");

    // Identity filter leaves non-RAW pixels bit-identical.
    s.execute("filter.cameraRaw", json!({})).unwrap();
    assert_eq!(active_pixel(&s, 8, 8), before);

    // Non-trivial filter preserves alpha and matches Smart Filter rendering.
    let params = json!({
        "temperature": 15.0,
        "tint": -10.0,
        "exposure": 0.5,
        "contrast": 20.0,
        "highlights": -25.0,
        "shadows": 30.0,
        "vibrance": 15.0,
    });

    let mut smart_session = Session::new();
    smart_session.execute("file.new", json!({"width": 16, "height": 16, "depth": 16})).unwrap();
    smart_session.execute("layer.new.layer", json!({})).unwrap();
    smart_session.execute("select.rect", json!({"x": 0, "y": 0, "width": 16, "height": 16})).unwrap();
    smart_session.execute("edit.fill", json!({"contents": "color", "color": "#4080c0", "opacity": 50.0})).unwrap();
    smart_session.execute("select.deselect", json!({})).unwrap();
    smart_session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();

    s.execute("filter.cameraRaw", params.clone()).unwrap();
    smart_session.execute("filter.cameraRaw", params).unwrap();

    let direct = active_pixel(&s, 8, 8);
    assert!((direct[3] - before[3]).abs() < 1e-4, "alpha must be preserved: {before:?} -> {direct:?}");
    assert_ne!(direct[0..3], before[0..3], "RGB must change under non-default settings");

    let smart_st = smart_session.active().unwrap();
    let smart_layer = smart_st.active_layer.and_then(|id| smart_st.doc.layer(id)).unwrap();
    let LayerContent::Smart(so) = &smart_layer.content else {
        panic!("expected smart object");
    };
    assert_eq!(so.smart_filters.len(), 1);
    assert_eq!(so.smart_filters[0].command, "filter.cameraRaw");
    let rendered = smart_layer.surface().unwrap().rgba(8, 8);
    for c in 0..4 {
        assert!((rendered[c] - direct[c]).abs() < 1e-3, "channel {c}: smart {rendered:?} vs direct {direct:?}");
    }
}

#[test]
fn camera_raw_filter_legacy_baseline_pixels() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "depth": 16})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 16})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#4080c0"})).unwrap();
    s.execute("select.rect", json!({"x": 8, "y": 0, "width": 8, "height": 16})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#c06020"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();

    s.execute(
        "filter.cameraRaw",
        json!({
            "temperature": 15.0,
            "tint": -10.0,
            "exposure": 0.5,
            "contrast": 20.0,
            "highlights": -25.0,
            "shadows": 30.0,
            "whites": 10.0,
            "blacks": -10.0,
            "vibrance": 15.0,
            "saturation": -5.0,
        }),
    )
    .unwrap();

    let quantize = |px: [f32; 4]| px.map(|v| (v.clamp(0.0, 1.0) * 65535.0).round() as u16);
    let got = [quantize(active_pixel(&s, 2, 2)), quantize(active_pixel(&s, 8, 8)), quantize(active_pixel(&s, 13, 13))];
    assert_eq!(got, [[19650, 39061, 56622, 65535], [57965, 29220, 10084, 65535], [57965, 29220, 10084, 65535]]);
}
