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
fn a_hidden_smart_object_still_takes_exposure() {
    // Not measured: only hidden pixel layers are refused; a hidden smart object keeps today's
    // behaviour and takes Exposure as a smart filter (CodeRabbit on #15522).
    let mut s = session(8);
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    s.execute("layer.setProps", json!({"visible": false})).unwrap();
    assert!(s.is_enabled(EXPOSURE), "{:?}", s.disabled_reason(EXPOSURE));
    exposure(&mut s).expect("recorded as a smart filter");
    let st = s.active().unwrap();
    let Some(photocraft_doc::LayerContent::Smart(sm)) = st.doc.layer(st.active_layer.unwrap()).map(|l| &l.content) else { panic!("a smart object") };
    assert_eq!(sm.smart_filters.len(), 1);
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

/// The value of alpha channel 0 at 4,4 (inside the saved selection).
fn alpha_pixel(s: &Session) -> f32 {
    s.active().unwrap().doc.channels[0].surface.rgba(4, 4)[0]
}

/// [`session`] with alpha channel 0 saved from a selection over 2,2 to 8,8.
fn with_alpha(depth: u32) -> Session {
    let mut s = session(depth);
    s.execute("select.rect", json!({"x": 2, "y": 2, "width": 6, "height": 6})).unwrap();
    s.execute("select.saveSelection", json!({"name": "Alpha 1"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s
}

#[test]
fn an_explicit_alpha_channel_target_keeps_working_over_a_hidden_layer() {
    // The Composite is targeted and the active layer is hidden, so the menu item is greyed, but a
    // call that names an alpha channel edits that channel as before.
    for depth in [8, 16, 32] {
        let mut s = with_alpha(depth);
        s.execute("layer.setProps", json!({"visible": false})).unwrap();
        s.execute("channel.target.composite", json!({})).unwrap();
        assert_eq!(s.disabled_reason(EXPOSURE).as_deref(), Some(EXPOSURE_UNAVAILABLE), "{depth} bit: the hidden layer greys the item");
        let p = json!({"exposure": -1.0, "target": {"channel": 0}});
        assert!(s.is_enabled_with(EXPOSURE, &p), "{depth} bit: {:?}", s.disabled_reason_with(EXPOSURE, &p));
        let (before, alpha, pixel) = (steps(&s), alpha_pixel(&s), layer_pixel(&s));
        s.execute(EXPOSURE, p).unwrap_or_else(|e| panic!("{depth} bit: {e}"));
        assert_eq!(steps(&s), before + 1, "{depth} bit: one history state");
        assert_ne!(alpha_pixel(&s), alpha, "{depth} bit: the alpha channel got darker");
        assert_eq!(layer_pixel(&s), pixel, "{depth} bit: the hidden layer is unchanged");
    }
}

#[test]
fn a_smart_object_filter_mask_target_keeps_working() {
    let mut s = session(8);
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    s.execute("filter.blur.gaussianBlur", json!({"radius": 1})).unwrap();
    let p = json!({"exposure": -2.0, "target": "filterMask"});
    assert!(s.is_enabled_with(EXPOSURE, &p), "{:?}", s.disabled_reason_with(EXPOSURE, &p));
    let before = steps(&s);
    s.execute(EXPOSURE, p).expect("Exposure edits the filter mask as before");
    assert_eq!(steps(&s), before + 1, "one history state");
    let st = s.active().unwrap();
    let Some(photocraft_doc::LayerContent::Smart(sm)) = st.doc.layer(st.active_layer.unwrap()).map(|l| &l.content) else { panic!("a smart object") };
    assert_eq!(sm.smart_filters.len(), 1, "the filter mask was adjusted: no second smart filter");
    assert!(sm.filter_mask.is_some(), "the edit went into the filter mask");
}

#[test]
fn an_explicit_pixels_target_on_an_empty_layer_stops_while_a_channel_is_targeted() {
    // `"target":"pixels"` edits the layer even with an alpha channel targeted in the Channels
    // panel, so an empty layer still stops with the empty-layer alert and adds no history state.
    for depth in [8, 16, 32] {
        let mut s = with_alpha(depth);
        s.execute("layer.new.layer", json!({"name": "Empty"})).unwrap();
        s.execute("channel.target", json!({"channel": {"alpha": 0}})).unwrap();
        assert!(crate::channel_cmds::edits_channel(&s), "{depth} bit: the alpha channel is the panel's target");
        let (before, alpha) = (steps(&s), alpha_pixel(&s));
        assert!(message(s.execute(EXPOSURE, json!({"exposure": 1.0, "target": "pixels"}))).contains(EXPOSURE_EMPTY), "{depth} bit");
        assert_eq!(steps(&s), before, "{depth} bit: no history state");
        assert_eq!(alpha_pixel(&s), alpha, "{depth} bit: the alpha channel is unchanged");
    }
}

#[test]
fn a_hidden_layer_named_by_layer_is_refused() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        let paint = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.setProps", json!({"layer": paint.0, "visible": false})).unwrap();
        let background = s.active().unwrap().doc.layers[0].id;
        s.execute("layer.select", json!({"layer": background.0})).unwrap();
        assert!(s.is_enabled(EXPOSURE), "{depth} bit: the visible Background is active");
        let p = json!({"exposure": 1.0, "layer": paint.0});
        assert_eq!(s.disabled_reason_with(EXPOSURE, &p).as_deref(), Some(EXPOSURE_UNAVAILABLE), "{depth} bit");
        let before = steps(&s);
        let pixel = |s: &Session| s.active().unwrap().doc.layer(paint).unwrap().surface().unwrap().rgba(8, 8);
        let was = pixel(&s);
        assert!(message(s.execute(EXPOSURE, p)).contains(EXPOSURE_UNAVAILABLE), "{depth} bit");
        assert_eq!(steps(&s), before, "{depth} bit: no history state");
        assert_eq!(pixel(&s), was, "{depth} bit: the hidden layer is unchanged");
    }
}

#[test]
fn a_shown_layer_mask_and_a_mask_param_grey_exposure() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
        s.execute("channel.target.composite", json!({})).unwrap();
        // With the Composite targeted, a call that names the mask is greyed as it is refused.
        assert!(s.is_enabled(EXPOSURE), "{depth} bit: the layer's pixels are the target");
        assert_eq!(s.disabled_reason_with(EXPOSURE, &json!({"target": "mask"})).as_deref(), Some(EXPOSURE_UNAVAILABLE), "{depth} bit");
        // Showing the mask alone (⌥-click its thumbnail) makes it the target: the item is greyed,
        // in the command list too, and running it changes nothing.
        s.execute("view.layerMask", json!({"mode": "gray"})).unwrap();
        assert_eq!(s.disabled_reason(EXPOSURE).as_deref(), Some(EXPOSURE_UNAVAILABLE), "{depth} bit");
        let listed = crate::commands::find(EXPOSURE).map(|c| (c.enabled)(&s));
        assert_eq!(listed, Some(Err(EXPOSURE_UNAVAILABLE.to_string())), "{depth} bit: command list");
        let (before, mask) = (steps(&s), mask_pixel(&s));
        assert!(exposure(&mut s).is_err(), "{depth} bit: refused");
        assert_eq!(steps(&s), before, "{depth} bit: no history state");
        assert_eq!(mask_pixel(&s), mask, "{depth} bit: the mask is unchanged");
    }
}
