//! #14632: Image › Adjustments › Exposure refuses a hidden pixel layer and a targeted layer mask
//! (the item is greyed: "The command "Exposure" is not currently available."), and stops on an
//! empty pixel layer ("Could not complete the Exposure command because the active layer is
//! empty."), as the reference application does. None of them adds a history state or changes a
//! pixel. A visible pixel layer, Lock Transparent Pixels, a selection and a smart object keep
//! working, and the other adjustments keep their checks.

use serde_json::{Value, json};

use crate::Session;
use crate::adjust_cmds::{EXPOSURE_EMPTY, EXPOSURE_UNAVAILABLE};

const EXPOSURE: &str = "image.adjustments.exposure";

/// A 16 x 16 document with the white Background and a grey layer "Paint" filling 4,4 to 12,12.
fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "depth": depth, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Paint"})).unwrap();
    s.execute("select.rect", json!({"x": 4, "y": 4, "width": 8, "height": 8})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": [0.5, 0.5, 0.5, 1.0]})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s
}

fn steps(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

/// The active layer's pixel at 8,8 (inside the grey square).
fn layer_pixel(s: &Session) -> [f32; 4] {
    let st = s.active().unwrap();
    let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
    layer.surface().unwrap().rgba(8, 8)
}

fn mask_pixel(s: &Session) -> f32 {
    let st = s.active().unwrap();
    let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
    layer.mask.as_ref().unwrap().surface.rgba(8, 8)[0]
}

fn exposure(s: &mut Session) -> crate::Result<Value> {
    s.execute(EXPOSURE, json!({"exposure": 1.0}))
}

fn message(r: crate::Result<Value>) -> String {
    r.expect_err("Exposure is refused").to_string()
}

#[test]
fn a_hidden_pixel_layer_greys_exposure_and_changes_nothing() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        s.execute("layer.setProps", json!({"visible": false})).unwrap();
        assert_eq!(s.disabled_reason(EXPOSURE).as_deref(), Some(EXPOSURE_UNAVAILABLE), "{depth} bit: greyed");
        let (before, pixel) = (steps(&s), layer_pixel(&s));
        assert!(message(exposure(&mut s)).contains(EXPOSURE_UNAVAILABLE), "{depth} bit");
        assert_eq!(steps(&s), before, "{depth} bit: no history state");
        assert_eq!(layer_pixel(&s), pixel, "{depth} bit: no pixel change");
    }
}

#[test]
fn an_empty_pixel_layer_stops_with_the_empty_layer_alert() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        s.execute("layer.new.layer", json!({"name": "Empty"})).unwrap();
        assert!(s.is_enabled(EXPOSURE), "{depth} bit: the item stays enabled, the alert comes on running it");
        let before = steps(&s);
        assert!(message(exposure(&mut s)).contains(EXPOSURE_EMPTY), "{depth} bit");
        assert_eq!(steps(&s), before, "{depth} bit: no history state");
    }
}

#[test]
fn a_targeted_layer_mask_greys_exposure_and_keeps_the_mask() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
        s.execute("channel.target.layerMask", json!({})).unwrap();
        assert_eq!(s.disabled_reason(EXPOSURE).as_deref(), Some(EXPOSURE_UNAVAILABLE), "{depth} bit: greyed");
        let (before, mask) = (steps(&s), mask_pixel(&s));
        assert!(exposure(&mut s).is_err(), "{depth} bit: refused");
        assert_eq!(steps(&s), before, "{depth} bit: no history state");
        assert_eq!(mask_pixel(&s), mask, "{depth} bit: the mask is unchanged");
        // A mask named in the params is refused the same way.
        s.execute("channel.target.composite", json!({})).unwrap();
        assert!(s.execute(EXPOSURE, json!({"exposure": 1.0, "target": "mask"})).is_err(), "{depth} bit: target mask param refused");
        assert_eq!(mask_pixel(&s), mask, "{depth} bit: the mask is unchanged");
    }
}

#[test]
fn brightness_contrast_still_works_on_a_targeted_mask() {
    let mut s = session(8);
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s.execute("channel.target.layerMask", json!({})).unwrap();
    assert!(s.is_enabled("image.adjustments.brightnessContrast"), "only Exposure changes its checks");
}

#[test]
fn a_visible_layer_lock_transparent_and_a_selection_still_work() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        assert!(s.is_enabled(EXPOSURE), "{depth} bit");
        let (before, pixel) = (steps(&s), layer_pixel(&s));
        exposure(&mut s).unwrap();
        assert_eq!(steps(&s), before + 1, "{depth} bit: one history state");
        assert_ne!(layer_pixel(&s), pixel, "{depth} bit: the layer got brighter");

        let mut s = session(depth);
        s.execute("layer.lockLayers", json!({"transparency": true})).unwrap();
        exposure(&mut s).unwrap();
        assert_ne!(layer_pixel(&s), [0.5, 0.5, 0.5, 1.0], "{depth} bit: Lock Transparent Pixels works");

        let mut s = session(depth);
        s.execute("select.rect", json!({"x": 6, "y": 6, "width": 4, "height": 4})).unwrap();
        exposure(&mut s).unwrap();
        assert_ne!(layer_pixel(&s), [0.5, 0.5, 0.5, 1.0], "{depth} bit: inside a selection works");
    }
}

#[test]
fn a_smart_object_still_takes_exposure() {
    let mut s = session(8);
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    assert!(s.is_enabled(EXPOSURE));
    exposure(&mut s).expect("recorded as a smart filter");
}

#[test]
fn an_alpha_channel_target_keeps_working_over_a_hidden_empty_layer() {
    let mut s = session(8);
    s.execute("select.rect", json!({"x": 2, "y": 2, "width": 6, "height": 6})).unwrap();
    s.execute("select.saveSelection", json!({"name": "Alpha 1"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Empty"})).unwrap();
    s.execute("layer.setProps", json!({"visible": false})).unwrap();
    s.execute("channel.target", json!({"channel": {"alpha": 0}})).unwrap();
    assert!(crate::channel_cmds::edits_channel(&s), "the alpha channel is the target");
    assert!(s.is_enabled(EXPOSURE), "a targeted alpha channel is not a hidden or empty layer");
    exposure(&mut s).expect("Exposure edits the alpha channel as before");
}
