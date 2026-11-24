//! #14749: Image › Adjustments › Hue/Saturation is unavailable where the reference application
//! refuses it (a hidden pixel layer, a targeted layer mask, Grayscale at 8 and 16 bit and
//! Multichannel documents), stops with the empty-layer alert on an empty layer, and keeps working
//! everywhere else it worked (32-bit Grayscale included: not measured). The other adjustments keep today's behaviour.

use serde_json::{Value, json};

use crate::Session;
use crate::adjust_cmds::{HUE_SATURATION_EMPTY, HUE_SATURATION_UNAVAILABLE};

const HS: &str = "image.adjustments.hueSaturation";

/// A 40 x 30 document with a white Background and a red pixel layer above it, active.
fn doc() -> Session {
    doc_at(8)
}

fn doc_at(depth: u32) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 40, "height": 30, "depth": depth, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s
}

fn steps(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

fn params() -> Value {
    json!({"saturation": -50})
}

/// Hue/Saturation is greyed and refused with the reference alert, and nothing changes.
fn assert_refused(s: &mut Session, case: &str) {
    assert!(!s.is_enabled(HS), "{case}: Hue/Saturation is greyed");
    let before = steps(s);
    let err = s.execute(HS, params()).unwrap_err().to_string();
    assert!(err.contains(HUE_SATURATION_UNAVAILABLE), "{case}: {err}");
    assert_eq!(steps(s), before, "{case}: no history state");
}

/// Hue/Saturation is available and runs, adding one history state.
fn assert_works(s: &mut Session, case: &str) {
    assert!(s.is_enabled(HS), "{case}: Hue/Saturation is available");
    let before = steps(s);
    s.execute(HS, params()).unwrap_or_else(|e| panic!("{case}: {e}"));
    assert_eq!(steps(s), before + 1, "{case}: one history state");
}

#[test]
fn a_hidden_pixel_layer_refuses_hue_saturation() {
    let mut s = doc();
    s.execute("layer.setProps", json!({"visible": false})).unwrap();
    assert_refused(&mut s, "hidden pixel layer");
}

#[test]
fn a_targeted_layer_mask_refuses_hue_saturation() {
    let mut s = doc();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s.execute("channel.target", json!({"channel": "mask"})).unwrap();
    assert_refused(&mut s, "targeted layer mask");
    assert!(!s.is_enabled_with(HS, &json!({"target": "mask"})), "an explicit mask target is refused too");
}

#[test]
fn grayscale_and_multichannel_documents_refuse_hue_saturation() {
    for depth in [8, 16] {
        let mut s = doc_at(depth);
        s.execute("image.mode.grayscale", json!({})).unwrap();
        assert_refused(&mut s, &format!("Grayscale {depth} bit"));
        let err = s.execute(HS, json!({"colorize": true, "hue": 30, "saturation": 25})).unwrap_err().to_string();
        assert!(err.contains(HUE_SATURATION_UNAVAILABLE), "Grayscale {depth} bit with Colorize: {err}");
    }
    let mut s = doc();
    s.execute("image.mode.multichannel", json!({})).unwrap();
    assert!(!s.is_enabled(HS), "Multichannel: Hue/Saturation is greyed");
    assert!(s.execute(HS, params()).is_err(), "Multichannel: refused");
}

/// Grayscale was measured at 8 and 16 bit only: a 32-bit Grayscale document keeps Hue/Saturation,
/// Colorize included.
#[test]
fn a_32_bit_grayscale_document_keeps_hue_saturation() {
    let mut s = doc_at(32);
    s.execute("image.mode.grayscale", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.depth, photocraft_color::SampleType::F32);
    assert_works(&mut s, "Grayscale 32 bit");
    let before = steps(&s);
    s.execute(HS, json!({"colorize": true, "hue": 30, "saturation": 25})).unwrap_or_else(|e| panic!("Grayscale 32 bit with Colorize: {e}"));
    assert_eq!(steps(&s), before + 1, "Grayscale 32 bit with Colorize: one history state");
}

#[test]
fn an_empty_layer_stops_with_the_empty_layer_alert() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 40, "height": 30, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let before = steps(&s);
    let err = s.execute(HS, params()).unwrap_err().to_string();
    assert!(err.contains(HUE_SATURATION_EMPTY), "{err}");
    assert_eq!(steps(&s), before, "no history state");
}

/// A layer whose pixels were all erased is empty too: the Eraser clears alpha and keeps the colour
/// bytes, which a byte-wise bounds scan would count as content.
#[test]
fn an_erased_layer_stops_with_the_empty_layer_alert() {
    let mut s = doc();
    let erase = json!({"points": [[-10, 15], [50, 15]], "size": 120, "erase": true, "brush": {"hardness": 1.0}});
    s.execute("paint.stroke", erase).unwrap();
    let before = steps(&s);
    let err = s.execute(HS, params()).unwrap_err().to_string();
    assert!(err.contains(HUE_SATURATION_EMPTY), "{err}");
    assert_eq!(steps(&s), before, "no history state");
    // A pixel painted again makes the layer non-empty.
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    assert_works(&mut s, "painted again");
}

#[test]
fn hue_saturation_keeps_working_where_it_worked() {
    let mut s = doc();
    assert_works(&mut s, "visible pixel layer");
    let mut s = doc();
    s.execute("layer.setProps", json!({"locks": {"transparency": true}})).unwrap();
    assert_works(&mut s, "Lock transparent pixels");
    for depth in [16, 32] {
        assert_works(&mut doc_at(depth), &format!("RGB {depth} bit"));
    }
    for mode in ["cmyk", "lab", "indexedColor"] {
        let mut s = doc();
        s.execute(&format!("image.mode.{mode}"), json!({})).unwrap();
        assert!(s.is_enabled(HS), "{mode}: Hue/Saturation is available");
        s.execute(HS, params()).unwrap_or_else(|e| panic!("{mode}: {e}"));
    }
    let mut s = doc();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    assert!(s.is_enabled(HS), "smart object: Hue/Saturation is available (as a smart filter)");
    s.execute(HS, params()).unwrap();
}

#[test]
fn locked_pixels_stay_refused() {
    // Already refused before #14749, when the adjustment runs; this item keeps that.
    for locks in [json!({"pixels": true}), json!({"all": true})] {
        let mut s = doc();
        s.execute("layer.setProps", json!({"locks": locks})).unwrap();
        let before = steps(&s);
        assert!(s.execute(HS, params()).is_err(), "{locks}: refused");
        assert_eq!(steps(&s), before, "{locks}: no history state");
    }
}

#[test]
fn the_other_adjustments_keep_todays_behaviour() {
    // This item changes Hue/Saturation only: Brightness/Contrast still runs on a hidden layer and
    // in Grayscale today (its own refusals are #14359).
    let mut s = doc();
    s.execute("layer.setProps", json!({"visible": false})).unwrap();
    assert!(s.is_enabled("image.adjustments.brightnessContrast"));
    let mut s = doc();
    s.execute("image.mode.grayscale", json!({})).unwrap();
    assert!(s.is_enabled("image.adjustments.brightnessContrast"));
}
