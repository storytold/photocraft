//! #14416: Image › Adjustments › Color Balance is unavailable ("The command "Color Balance" is not
//! currently available.") in Grayscale, Bitmap and Multichannel documents, on a hidden pixel layer
//! and on a targeted layer mask, and stops on an empty pixel layer, as the reference application
//! does; none of these adds a history state or changes a pixel. RGB, CMYK, Lab, Indexed, a smart
//! object and Lock transparent pixels keep working, the other adjustments keep their targets, and
//! the unmeasured cases keep today's behaviour.

use serde_json::json;

use super::black_white_refusals_tests::{layer_pixel, pixel, session, steps};
use crate::Session;
use crate::adjust_cmds::{COLOR_BALANCE_EMPTY, COLOR_BALANCE_UNAVAILABLE};

const CB: &str = "image.adjustments.colorBalance";

/// Midtones all the way to red, so the red half of "Paint" visibly changes.
fn params() -> serde_json::Value {
    json!({"midtones": [100, 0, 0], "preserveLuminosity": false})
}

/// The active layer's mask at 4,4.
fn mask_pixel(s: &Session) -> [f32; 4] {
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface.rgba(4, 4)
}

/// Color Balance is greyed with `why`, running it is refused with it, and neither the history
/// nor what `probe` reads changes.
fn assert_refused(s: &mut Session, why: &str, probe: fn(&Session) -> [f32; 4], case: &str) {
    assert_eq!(s.disabled_reason(CB).as_deref(), Some(why), "{case}: greyed");
    let (before, was) = (steps(s), probe(s));
    let err = s.execute(CB, params()).expect_err("Color Balance is refused").to_string();
    assert!(err.contains(why), "{case}: {err}");
    assert_eq!(steps(s), before, "{case}: no history state");
    assert_eq!(probe(s), was, "{case}: no pixel change");
}

/// Color Balance is available and runs as one history state that changes the red half.
fn assert_works(s: &mut Session, case: &str) {
    assert!(s.is_enabled(CB), "{case}: {:?}", s.disabled_reason(CB));
    let (before, was) = (steps(s), pixel(s));
    s.execute(CB, params()).unwrap_or_else(|e| panic!("{case}: {e}"));
    assert_eq!(steps(s), before + 1, "{case}: one history state");
    assert_ne!(pixel(s), was, "{case}: the pixels changed");
}

/// A layer mask on "Paint", targeted in the Channels panel.
fn target_a_mask(s: &mut Session) {
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s.execute("channel.target.layerMask", json!({})).unwrap();
}

/// The first ink of a Multichannel document at 4,4.
fn ink(s: &Session) -> [f32; 4] {
    s.active().unwrap().doc.channels[0].surface.rgba(4, 4)
}

#[test]
fn grayscale_and_multichannel_documents_grey_color_balance() {
    for (mode, probe) in [("grayscale", pixel as fn(&Session) -> [f32; 4]), ("multichannel", ink)] {
        let mut s = session();
        s.execute(&format!("image.mode.{mode}"), json!({})).unwrap();
        let listed = crate::commands::find(CB).map(|c| (c.enabled)(&s));
        assert_eq!(listed, Some(Err(COLOR_BALANCE_UNAVAILABLE.to_string())), "{mode}: the command list's check");
        assert_refused(&mut s, COLOR_BALANCE_UNAVAILABLE, probe, mode);
    }
}

#[test]
fn a_bitmap_document_greys_color_balance() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 16, "height": 16, "mode": "gray", "background": "white"})).unwrap();
    s.execute("image.mode.bitmap", json!({})).unwrap();
    assert_refused(&mut s, "not available in Bitmap mode", pixel, "Bitmap");
}

#[test]
fn a_hidden_pixel_layer_greys_color_balance() {
    let mut s = session();
    s.execute("layer.setProps", json!({"visible": false})).unwrap();
    assert_refused(&mut s, COLOR_BALANCE_UNAVAILABLE, pixel, "hidden pixel layer");
    assert!(s.is_enabled("image.adjustments.levels"), "the other adjustments keep their checks");
}

#[test]
fn a_targeted_layer_mask_greys_color_balance() {
    let mut s = session();
    target_a_mask(&mut s);
    assert_refused(&mut s, COLOR_BALANCE_UNAVAILABLE, mask_pixel, "targeted layer mask");
    // A recorded call that names the mask is refused the same way.
    s.execute("channel.target.composite", json!({})).unwrap();
    let (before, was) = (steps(&s), mask_pixel(&s));
    let mut p = params();
    p["target"] = json!("mask");
    let err = s.execute(CB, p).expect_err("Color Balance is refused").to_string();
    assert!(err.contains(COLOR_BALANCE_UNAVAILABLE), "{err}");
    assert_eq!((steps(&s), mask_pixel(&s)), (before, was), "no history state, mask unchanged");
}

#[test]
fn brightness_contrast_still_applies_to_a_targeted_layer_mask() {
    let mut s = session();
    target_a_mask(&mut s);
    let (before, was) = (steps(&s), mask_pixel(&s));
    s.execute("image.adjustments.brightnessContrast", json!({"brightness": -100, "legacy": true})).unwrap();
    assert_eq!(steps(&s), before + 1, "one history state");
    assert_ne!(mask_pixel(&s), was, "the mask darkened");
}

#[test]
fn an_empty_pixel_layer_stops_with_the_empty_layer_alert() {
    let mut s = session();
    s.execute("layer.new.layer", json!({"name": "Empty"})).unwrap();
    assert!(s.is_enabled(CB), "the item stays enabled; the alert comes on running it");
    let before = steps(&s);
    let err = s.execute(CB, params()).expect_err("Color Balance is refused").to_string();
    assert_eq!(err, COLOR_BALANCE_EMPTY);
    assert_eq!(steps(&s), before, "no history state");
}

/// A Grayscale document with alpha channel 0 saved from a selection, the composite targeted.
fn grayscale_with_an_alpha_channel() -> Session {
    let mut s = session();
    s.execute("select.rect", json!({"x": 2, "y": 2, "width": 6, "height": 6})).unwrap();
    s.execute("select.saveSelection", json!({"name": "Alpha 1"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("image.mode.grayscale", json!({})).unwrap();
    s.execute("channel.target.composite", json!({})).unwrap();
    s
}

#[test]
fn an_explicit_alpha_channel_in_grayscale_works_over_the_targeted_composite() {
    let mut s = grayscale_with_an_alpha_channel();
    assert_eq!(s.disabled_reason(CB).as_deref(), Some(COLOR_BALANCE_UNAVAILABLE), "the composite is greyed");
    let mut p = params();
    p["target"] = json!({"channel": 0});
    assert_eq!(s.disabled_reason_with(CB, &p), None, "the alpha channel named");
    let before = steps(&s);
    s.execute(CB, p).expect("the alpha channel is adjusted");
    assert_eq!(steps(&s), before + 1, "one history state");
}

#[test]
fn explicit_pixels_in_grayscale_are_refused_over_a_targeted_alpha_channel() {
    let mut s = grayscale_with_an_alpha_channel();
    s.execute("channel.target", json!({"channel": {"alpha": 0}})).unwrap();
    assert!(s.is_enabled(CB), "the alpha channel keeps the shared check: {:?}", s.disabled_reason(CB));
    let mut p = params();
    p["target"] = json!("pixels");
    assert_eq!(s.disabled_reason_with(CB, &p).as_deref(), Some(COLOR_BALANCE_UNAVAILABLE), "the layer's pixels in Grayscale");
    let (before, was) = (steps(&s), pixel(&s));
    let err = s.execute(CB, p).expect_err("Color Balance is refused").to_string();
    assert!(err.contains(COLOR_BALANCE_UNAVAILABLE), "{err}");
    assert_eq!((steps(&s), pixel(&s)), (before, was), "no history state, no pixel change");
}

#[test]
fn a_hidden_layer_named_by_layer_is_refused() {
    let mut s = session();
    let paint = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.setProps", json!({"layer": paint.0, "visible": false})).unwrap();
    let background = s.active().unwrap().doc.layers[0].id;
    s.execute("layer.select", json!({"layer": background.0})).unwrap();
    assert!(s.is_enabled(CB), "the visible Background is active");
    let mut p = params();
    p["layer"] = json!(paint.0);
    assert_eq!(s.disabled_reason_with(CB, &p).as_deref(), Some(COLOR_BALANCE_UNAVAILABLE));
    let (before, was) = (steps(&s), layer_pixel(&s, paint));
    let err = s.execute(CB, p).expect_err("Color Balance is refused").to_string();
    assert!(err.contains(COLOR_BALANCE_UNAVAILABLE), "{err}");
    assert_eq!((steps(&s), layer_pixel(&s, paint)), (before, was), "no history state, the hidden layer is unchanged");
}

#[test]
fn rgb_cmyk_lab_and_indexed_documents_work() {
    for (mode, p) in [
        ("rgb", None),
        ("cmyk", Some(json!({}))),
        ("lab", Some(json!({}))),
        ("indexedColor", Some(json!({"palette": "exact", "forced": "none", "dither": "none", "transparency": false}))),
    ] {
        let mut s = session();
        if let Some(p) = p {
            s.execute(&format!("image.mode.{mode}"), p).unwrap();
        }
        assert_works(&mut s, mode);
    }
}

#[test]
fn lock_transparent_pixels_works() {
    let mut s = session();
    s.execute("layer.setProps", json!({"locks": {"transparency": true}})).unwrap();
    assert_works(&mut s, "Lock transparent pixels");
}

#[test]
fn a_smart_object_records_a_smart_filter() {
    let mut s = session();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    assert!(s.is_enabled(CB), "{:?}", s.disabled_reason(CB));
    let before = steps(&s);
    s.execute(CB, params()).expect("recorded as a smart filter");
    assert_eq!(steps(&s), before + 1, "one history state");
    let st = s.active().unwrap();
    let Some(photocraft_doc::LayerContent::Smart(sm)) = st.doc.layer(st.active_layer.unwrap()).map(|l| &l.content) else { panic!("a smart object") };
    assert_eq!(sm.smart_filters.len(), 1);
}

#[test]
fn unmeasured_cases_keep_todays_behaviour() {
    // A layer inside a hidden group.
    let mut s = session();
    let paint = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.groupLayers", json!({})).unwrap();
    s.execute("layer.setProps", json!({"visible": false})).unwrap();
    s.execute("layer.select", json!({"layer": paint.0})).unwrap();
    assert_works(&mut s, "layer in a hidden group");

    // A layer that is empty only inside the selection.
    let mut s = session();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 4, "height": 4})).unwrap();
    assert!(s.is_enabled(CB), "{:?}", s.disabled_reason(CB));
    let before = steps(&s);
    s.execute(CB, params()).expect("runs over the empty selection");
    assert_eq!(steps(&s), before + 1, "one history state");

    // An alpha channel in a Grayscale document.
    let mut s = session();
    s.execute("select.rect", json!({"x": 2, "y": 2, "width": 6, "height": 6})).unwrap();
    s.execute("select.saveSelection", json!({"name": "Alpha 1"})).unwrap();
    s.execute("image.mode.grayscale", json!({})).unwrap();
    s.execute("channel.target", json!({"channel": {"alpha": 0}})).unwrap();
    assert!(s.is_enabled(CB), "alpha channel in Grayscale: {:?}", s.disabled_reason(CB));
    s.execute(CB, params()).expect("the alpha channel is adjusted");

    // The Quick Mask in a Grayscale document.
    let mut s = session();
    s.execute("image.mode.grayscale", json!({})).unwrap();
    s.execute("select.editInQuickMaskMode", json!({"on": true})).unwrap();
    assert!(s.is_enabled(CB), "Quick Mask in Grayscale: {:?}", s.disabled_reason(CB));
    s.execute(CB, params()).expect("the Quick Mask is adjusted");
}
