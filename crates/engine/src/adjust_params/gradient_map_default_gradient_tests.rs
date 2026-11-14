use serde_json::json;

use super::*;
use crate::Session;

fn sample_u8_rgb(s: &Session, x: i32, y: i32) -> [u8; 3] {
    let doc = &s.active().unwrap().doc;
    let id = s.active().unwrap().active_layer.unwrap();
    let photocraft_doc::LayerContent::Raster(surf) = &doc.layer(id).unwrap().content else {
        panic!("expected raster layer");
    };
    let px = surf.rgba(x, y);
    [(px[0].clamp(0.0, 1.0) * 255.0).round() as u8, (px[1].clamp(0.0, 1.0) * 255.0).round() as u8, (px[2].clamp(0.0, 1.0) * 255.0).round() as u8]
}

#[test]
fn command_without_stops_or_source_uses_foreground_to_background_smooth_100_classic() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "depth": 8, "background": "white"})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 16})).unwrap();
    s.execute("edit.fill", json!({"color": "#101010"})).unwrap();
    s.execute("select.rect", json!({"x": 8, "y": 0, "width": 8, "height": 16})).unwrap();
    s.execute("edit.fill", json!({"color": "#f0f0f0"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();

    s.tools.foreground = [0.0, 160.0 / 255.0, 0.0, 1.0];
    s.tools.background = [1.0, 1.0, 0.0, 1.0];

    s.execute("image.adjustments.gradientMap", json!({})).unwrap();

    let dark = sample_u8_rgb(&s, 2, 2);
    let light = sample_u8_rgb(&s, 12, 2);
    assert!((i16::from(dark[0]) - 9).abs() <= 1 && (i16::from(dark[1]) - 164).abs() <= 1 && dark[2] <= 1, "expected gray 16 -> ~(9, 164, 0), got {dark:?}");
    assert!(
        (i16::from(light[0]) - 246).abs() <= 1 && (i16::from(light[1]) - 252).abs() <= 1 && light[2] <= 1,
        "expected gray 240 -> ~(246, 252, 0), got {light:?}"
    );
}

#[test]
fn smart_filter_keeps_foreground_background_when_colours_change_later() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "depth": 8, "background": "white"})).unwrap();
    s.execute("edit.fill", json!({"color": "#101010"})).unwrap();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();

    s.tools.foreground = [0.0, 160.0 / 255.0, 0.0, 1.0];
    s.tools.background = [1.0, 1.0, 0.0, 1.0];

    s.execute("image.adjustments.gradientMap", json!({})).unwrap();

    // Change foreground/background afterwards and verify the smart filter keeps its baked colours.
    s.tools.foreground = [1.0, 0.0, 0.0, 1.0];
    s.tools.background = [0.0, 0.0, 1.0, 1.0];

    let doc = &s.active().unwrap().doc;
    let id = s.active().unwrap().active_layer.unwrap();
    let photocraft_doc::LayerContent::Smart(sm) = &doc.layer(id).unwrap().content else {
        panic!("expected smart object layer");
    };
    let sf = &sm.smart_filters[0];
    assert!(
        sf.params.get("source").is_some_and(Value::is_object) && sf.params.get("stops").is_some_and(Value::is_array),
        "smart filter must record resolved gradient params, got {}",
        sf.params
    );
    assert_eq!(sf.params["source"]["stops"][0][1], json!("#00a000"));
    assert_eq!(sf.params["source"]["stops"][1][1], json!("#ffff00"));
}

#[test]
fn adjustment_layer_keeps_black_to_white_default() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    s.tools.foreground = [1.0, 0.0, 0.0, 1.0];
    s.tools.background = [0.0, 0.0, 1.0, 1.0];

    s.execute("layer.newAdjustmentLayer.gradientMap", json!({})).unwrap();

    let doc = &s.active().unwrap().doc;
    let id = s.active().unwrap().active_layer.unwrap();
    let photocraft_doc::LayerContent::Adjustment(Adjustment::GradientMap { stops, source, .. }) = &doc.layer(id).unwrap().content else {
        panic!("expected GradientMap adjustment layer");
    };
    assert_eq!(stops, &vec![(0.0, [0.0; 3]), (1.0, [1.0; 3])]);
    assert!(source.is_none());
}
