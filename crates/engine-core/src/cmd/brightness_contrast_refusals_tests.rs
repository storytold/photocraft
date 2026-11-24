//! #14359: Image › Adjustments › Brightness/Contrast refuses a hidden pixel layer (the item is
//! greyed: "The command "Brightness/Contrast" is not currently available.") and stops on an empty
//! pixel layer ("Could not complete the Brightness/Contrast command because the active layer is
//! empty."), as the reference application does. Neither adds a history state or changes a pixel.
//! A visible pixel layer, a targeted layer mask, an alpha channel or Quick Mask target and a smart
//! object keep working, and the other adjustments keep their checks.

use photocraft_geom::Rect;
use serde_json::{Value, json};

use crate::Session;
use crate::adjust_cmds::{BRIGHTNESS_CONTRAST_EMPTY, BRIGHTNESS_CONTRAST_UNAVAILABLE};

const BC: &str = "image.adjustments.brightnessContrast";

/// A 16 x 16 document with the white Background and a grey layer "Paint" filling 4,4 to 12,12,
/// and alpha channel 0 saved from a selection over 2,2 to 8,8.
fn session() -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 16, "height": 16, "background": "white"})).unwrap();
    s.execute("select.rect", json!({"x": 2, "y": 2, "width": 6, "height": 6})).unwrap();
    s.execute("select.saveSelection", json!({"name": "Alpha 1"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Paint"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    // Painted directly, since Edit › Fill lives in photocraft-engine-paint.
    s.edit("grey square", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        surf.fill_rect(Rect::new(4, 4, 12, 12), &[0.5, 0.5, 0.5, 1.0]);
        Ok(())
    })
    .unwrap();
    s
}

fn steps(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

/// The active layer's pixel at 8,8 (inside the grey square).
fn layer_pixel(s: &Session) -> [f32; 4] {
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(8, 8)
}

/// The active layer's mask at 8,8.
fn mask_pixel(s: &Session) -> f32 {
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface.rgba(8, 8)[0]
}

/// Alpha channel 0 at 4,4 (inside the saved selection).
fn alpha_pixel(s: &Session) -> f32 {
    s.active().unwrap().doc.channels[0].surface.rgba(4, 4)[0]
}

/// Darkens in the legacy mode, so white masks and channels change too.
fn darken(target: Value) -> Value {
    let mut p = json!({"brightness": -100, "legacy": true});
    if !target.is_null() {
        p["target"] = target;
    }
    p
}

fn message(r: crate::Result<Value>) -> String {
    r.expect_err("Brightness/Contrast is refused").to_string()
}

#[test]
fn a_hidden_pixel_layer_greys_brightness_contrast_and_changes_nothing() {
    let mut s = session();
    s.execute("layer.setProps", json!({"visible": false})).unwrap();
    assert_eq!(s.disabled_reason(BC).as_deref(), Some(BRIGHTNESS_CONTRAST_UNAVAILABLE), "greyed");
    let listed = crate::commands::find(BC).map(|c| (c.enabled)(&s));
    assert_eq!(listed, Some(Err(BRIGHTNESS_CONTRAST_UNAVAILABLE.to_string())), "the command list's check");
    let (before, pixel) = (steps(&s), layer_pixel(&s));
    assert!(message(s.execute(BC, json!({"brightness": 50}))).contains(BRIGHTNESS_CONTRAST_UNAVAILABLE));
    assert_eq!(steps(&s), before, "no history state");
    assert_eq!(layer_pixel(&s), pixel, "no pixel change");
    assert!(s.is_enabled("image.adjustments.levels"), "the other adjustments keep their checks");
}

#[test]
fn a_hidden_layer_named_by_layer_is_refused() {
    let mut s = session();
    let paint = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.setProps", json!({"layer": paint.0, "visible": false})).unwrap();
    let background = s.active().unwrap().doc.layers[0].id;
    s.execute("layer.select", json!({"layer": background.0})).unwrap();
    assert!(s.is_enabled(BC), "the visible Background is active");
    let p = json!({"brightness": 50, "layer": paint.0});
    assert_eq!(s.disabled_reason_with(BC, &p).as_deref(), Some(BRIGHTNESS_CONTRAST_UNAVAILABLE));
    let pixel = |s: &Session| s.active().unwrap().doc.layer(paint).unwrap().surface().unwrap().rgba(8, 8);
    let (before, was) = (steps(&s), pixel(&s));
    assert!(message(s.execute(BC, p)).contains(BRIGHTNESS_CONTRAST_UNAVAILABLE));
    assert_eq!(steps(&s), before, "no history state");
    assert_eq!(pixel(&s), was, "the hidden layer is unchanged");
}

#[test]
fn an_empty_pixel_layer_stops_with_the_empty_layer_alert() {
    let mut s = session();
    s.execute("layer.new.layer", json!({"name": "Empty"})).unwrap();
    assert!(s.is_enabled(BC), "the item stays enabled; the alert comes on running it");
    let before = steps(&s);
    assert_eq!(message(s.execute(BC, json!({"brightness": 50}))), BRIGHTNESS_CONTRAST_EMPTY);
    assert_eq!(steps(&s), before, "no history state");
}

#[test]
fn a_targeted_layer_mask_takes_it_even_on_an_empty_layer() {
    let mut s = session();
    s.execute("layer.new.layer", json!({"name": "Empty"})).unwrap();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s.execute("channel.target.layerMask", json!({})).unwrap();
    assert!(s.is_enabled(BC), "{:?}", s.disabled_reason(BC));
    let (before, mask) = (steps(&s), mask_pixel(&s));
    s.execute(BC, darken(Value::Null)).expect("the adjustment goes to the mask");
    assert_eq!(steps(&s), before + 1, "one history state");
    assert!(mask_pixel(&s) < mask, "the mask got darker");
}

#[test]
fn an_alpha_channel_or_quick_mask_target_keeps_working_over_a_hidden_layer() {
    // Targeted in the Channels panel.
    let mut s = session();
    s.execute("layer.setProps", json!({"visible": false})).unwrap();
    s.execute("channel.target", json!({"channel": {"alpha": 0}})).unwrap();
    assert!(s.is_enabled(BC), "{:?}", s.disabled_reason(BC));
    let (alpha, pixel) = (alpha_pixel(&s), layer_pixel(&s));
    s.execute(BC, darken(Value::Null)).expect("the alpha channel is adjusted");
    assert!(alpha_pixel(&s) < alpha, "the alpha channel got darker");
    assert_eq!(layer_pixel(&s), pixel, "the hidden layer is unchanged");

    // Named in the call while the Composite is targeted: the item is greyed for the hidden layer,
    // but the call edits the channel as before.
    let mut s = session();
    s.execute("layer.setProps", json!({"visible": false})).unwrap();
    s.execute("channel.target.composite", json!({})).unwrap();
    let p = darken(json!({"channel": 0}));
    assert!(s.is_enabled_with(BC, &p), "{:?}", s.disabled_reason_with(BC, &p));
    let alpha = alpha_pixel(&s);
    s.execute(BC, p).expect("the named alpha channel is adjusted");
    assert!(alpha_pixel(&s) < alpha, "the alpha channel got darker");

    // Quick Mask over a hidden layer.
    let mut s = session();
    s.execute("layer.setProps", json!({"visible": false})).unwrap();
    s.execute("select.rect", json!({"x": 2, "y": 2, "width": 6, "height": 6})).unwrap();
    s.execute("select.editInQuickMaskMode", json!({"on": true})).unwrap();
    assert!(s.is_enabled(BC), "{:?}", s.disabled_reason(BC));
    let before = steps(&s);
    s.execute(BC, darken(Value::Null)).expect("the Quick Mask is adjusted");
    assert_eq!(steps(&s), before + 1, "one history state");
}

#[test]
fn a_visible_pixel_layer_and_a_smart_object_still_work() {
    let mut s = session();
    let (before, pixel) = (steps(&s), layer_pixel(&s));
    s.execute(BC, json!({"brightness": 50})).unwrap();
    assert_eq!(steps(&s), before + 1, "one history state");
    assert_ne!(layer_pixel(&s), pixel, "the layer got brighter");

    // A smart object, hidden too (not measured, so it keeps today's behaviour), records it as a
    // smart filter.
    let mut s = session();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    s.execute("layer.setProps", json!({"visible": false})).unwrap();
    assert!(s.is_enabled(BC), "{:?}", s.disabled_reason(BC));
    s.execute(BC, json!({"brightness": 50})).expect("recorded as a smart filter");
    let st = s.active().unwrap();
    let Some(photocraft_doc::LayerContent::Smart(sm)) = st.doc.layer(st.active_layer.unwrap()).map(|l| &l.content) else { panic!("a smart object") };
    assert_eq!(sm.smart_filters.len(), 1);
}
