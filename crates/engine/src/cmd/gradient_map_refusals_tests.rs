//! #14696: `image.adjustments.gradientMap` availability and refusals.

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::LayerContent;
use serde_json::{Value, json};

use crate::Session;

fn gradient_params() -> Value {
    json!({ "stops": [[0.0, "#ff0000"], [1.0, "#0000ff"]] })
}

fn new_doc(depth: u64, mode: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "depth": depth, "mode": mode, "background": "white"})).unwrap();
    s.execute("paint.stroke", json!({"points": [[2, 2], [14, 14]], "color": "#408040", "size": 6})).unwrap();
    s
}

fn active_rgba(s: &Session, x: i32, y: i32) -> [f32; 4] {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)
}

#[test]
fn gradient_map_disabled_and_refused_in_32bit_documents() {
    for mode in ["rgb", "gray"] {
        let mut s = new_doc(32, mode);
        assert_eq!(s.active().unwrap().doc.depth, SampleType::F32);
        let steps = s.active().unwrap().history.past_len();
        let before = active_rgba(&s, 8, 8);

        assert!(!s.is_enabled("image.adjustments.gradientMap"), "{mode} 32-bit must grey out Gradient Map");
        let err = s.execute("image.adjustments.gradientMap", gradient_params()).unwrap_err();
        assert!(err.to_string().contains("32-bit"), "{mode} 32-bit error: {err}");
        assert_eq!(s.active().unwrap().history.past_len(), steps, "{mode} 32-bit must not add history");
        assert_eq!(active_rgba(&s, 8, 8), before, "{mode} 32-bit must not change pixels");
    }
}

#[test]
fn gradient_map_disabled_and_refused_on_hidden_pixel_layer() {
    let mut s = new_doc(8, "rgb");
    let id = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
    s.execute("paint.stroke", json!({"points": [[2, 2], [14, 14]], "color": "#808080", "size": 6})).unwrap();
    s.execute("layer.setProps", json!({"layer": id, "visible": false})).unwrap();

    let steps = s.active().unwrap().history.past_len();
    let before = active_rgba(&s, 8, 8);

    assert!(!s.is_enabled("image.adjustments.gradientMap"), "hidden pixel layer must grey out Gradient Map");
    let err = s.execute("image.adjustments.gradientMap", gradient_params()).unwrap_err();
    assert!(err.to_string().contains("hidden"), "hidden pixel layer error: {err}");
    assert_eq!(s.active().unwrap().history.past_len(), steps, "hidden pixel layer must not add history");
    assert_eq!(active_rgba(&s, 8, 8), before, "hidden pixel layer must not change pixels");
}

#[test]
fn gradient_map_refuses_empty_pixel_layer() {
    let mut s = new_doc(8, "rgb");
    s.execute("layer.new.layer", json!({})).unwrap();
    let steps = s.active().unwrap().history.past_len();

    let err = s.execute("image.adjustments.gradientMap", gradient_params()).unwrap_err();
    assert_eq!(err.to_string(), "Could not complete the Gradient Map command because the active layer is empty.");
    assert_eq!(s.active().unwrap().history.past_len(), steps, "empty pixel layer must not add history");
}

#[test]
fn gradient_map_disabled_and_refused_in_indexed_and_bitmap_documents() {
    // Indexed Color
    let mut s = new_doc(8, "rgb");
    s.execute("image.mode.indexedColor", json!({"palette": "exact", "dither": "none", "forced": "none"})).unwrap();
    assert_eq!(s.active().unwrap().doc.mode, ColorMode::Indexed);
    let steps = s.active().unwrap().history.past_len();
    let before = active_rgba(&s, 8, 8);
    assert!(!s.is_enabled("image.adjustments.gradientMap"), "Indexed must grey out Gradient Map");
    assert!(s.execute("image.adjustments.gradientMap", gradient_params()).is_err(), "Indexed must refuse Gradient Map");
    assert_eq!(s.active().unwrap().history.past_len(), steps);
    assert_eq!(active_rgba(&s, 8, 8), before);

    // Bitmap
    let mut s = new_doc(8, "gray");
    s.execute("image.mode.bitmap", json!({"method": "threshold"})).unwrap();
    assert_eq!(s.active().unwrap().doc.mode, ColorMode::Bitmap);
    let steps = s.active().unwrap().history.past_len();
    let before = active_rgba(&s, 8, 8);
    assert!(!s.is_enabled("image.adjustments.gradientMap"), "Bitmap must grey out Gradient Map");
    assert!(s.execute("image.adjustments.gradientMap", gradient_params()).is_err(), "Bitmap must refuse Gradient Map");
    assert_eq!(s.active().unwrap().history.past_len(), steps);
    assert_eq!(active_rgba(&s, 8, 8), before);
}

#[test]
fn gradient_map_works_on_supported_modes_depths_locks_and_smart_objects() {
    for (depth, mode) in [(8, "rgb"), (16, "rgb"), (8, "gray"), (8, "cmyk"), (8, "lab")] {
        let mut s = new_doc(depth, mode);
        let steps = s.active().unwrap().history.past_len();
        let before = active_rgba(&s, 8, 8);
        assert!(s.is_enabled("image.adjustments.gradientMap"), "{mode} {depth}-bit must enable Gradient Map");
        s.execute("image.adjustments.gradientMap", gradient_params()).unwrap();
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "{mode} {depth}-bit must add one history step");
        assert_ne!(active_rgba(&s, 8, 8), before, "{mode} {depth}-bit must update pixels");
    }

    // Lock Transparent Pixels
    let mut s = new_doc(8, "rgb");
    let id = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
    s.execute("paint.stroke", json!({"points": [[2, 2], [14, 14]], "color": "#408040", "size": 6})).unwrap();
    s.execute("layer.setProps", json!({"layer": id, "lockTransparency": true})).unwrap();
    let steps = s.active().unwrap().history.past_len();
    assert!(s.is_enabled("image.adjustments.gradientMap"), "Lock Transparent Pixels must enable Gradient Map");
    s.execute("image.adjustments.gradientMap", gradient_params()).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1);

    // Lock Position
    let mut s = new_doc(8, "rgb");
    let id = s.active().unwrap().active_layer.unwrap().0;
    s.execute("layer.setProps", json!({"layer": id, "lockPosition": true})).unwrap();
    let steps = s.active().unwrap().history.past_len();
    assert!(s.is_enabled("image.adjustments.gradientMap"), "Lock Position must enable Gradient Map");
    s.execute("image.adjustments.gradientMap", gradient_params()).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1);

    // Smart Object (recorded as a smart filter)
    let mut s = new_doc(8, "rgb");
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let steps = s.active().unwrap().history.past_len();
    assert!(s.is_enabled("image.adjustments.gradientMap"), "Smart Object must enable Gradient Map");
    s.execute("image.adjustments.gradientMap", gradient_params()).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
    let d = s.active().unwrap();
    let l = d.doc.layer(d.active_layer.unwrap()).unwrap();
    match &l.content {
        LayerContent::Smart(sm) => assert_eq!(sm.smart_filters.len(), 1, "smart object must record one smart filter"),
        other => panic!("expected smart object, got {}", other.kind_name()),
    }
}

#[test]
fn gradient_map_adjustment_layer_and_other_adjustments_still_work_in_32bit() {
    let mut s = new_doc(32, "rgb");
    assert!(s.is_enabled("layer.newAdjustmentLayer.gradientMap"), "32-bit must still enable Gradient Map adjustment layer");
    s.execute("layer.newAdjustmentLayer.gradientMap", gradient_params()).unwrap();

    let mut s = new_doc(32, "rgb");
    let before = active_rgba(&s, 8, 8);
    assert!(s.is_enabled("image.adjustments.invert"), "32-bit must still enable Invert");
    s.execute("image.adjustments.invert", json!({})).unwrap();
    assert_ne!(active_rgba(&s, 8, 8), before);
}
