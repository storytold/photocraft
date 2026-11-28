//! #14401: Image › Adjustments › Black & White is unavailable ("The command "Black & White" is not
//! currently available.") in Grayscale, CMYK, Lab and Indexed documents and on a hidden pixel
//! layer, and stops on an empty pixel layer, as the reference application does; none of these
//! adds a history state or changes a pixel. RGB at every depth, the Background, Lock transparent
//! pixels, a selection and a smart object keep working, and the unmeasured cases keep today's
//! behaviour.

use serde_json::{Value, json};

use crate::Session;
use crate::adjust_cmds::{BLACK_WHITE_EMPTY, BLACK_WHITE_UNAVAILABLE, black_white_enabled, has_adjustable};

const BW: &str = "image.adjustments.blackWhite";

/// A 16 x 16 RGB document at `depth` with a white Background and a layer "Paint" active, its left
/// half (200,50,50).
fn session_at(depth: u32) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 16, "height": 16, "depth": depth, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Paint"})).unwrap();
    fill_left_half(&mut s);
    s
}

/// Paints the active layer's left half (200,50,50), as Edit › Fill of that selection would (edit.fill is
/// a paint command, not installed in the core crate).
fn fill_left_half(s: &mut Session) {
    s.edit("Fill", |doc, active| {
        let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        surface.fill_rect(photocraft_geom::Rect::new(0, 0, 8, 16), &[200.0 / 255.0, 50.0 / 255.0, 50.0 / 255.0, 1.0]);
        Ok(())
    })
    .unwrap();
}

pub(super) fn session() -> Session {
    session_at(8)
}

pub(super) fn steps(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

/// The active layer's pixel at 4,4 (inside the red half).
pub(super) fn pixel(s: &Session) -> [f32; 4] {
    layer_pixel(s, s.active().unwrap().active_layer.unwrap())
}

/// Layer `id`'s pixel at 4,4.
pub(super) fn layer_pixel(s: &Session, id: photocraft_doc::LayerId) -> [f32; 4] {
    s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().rgba(4, 4)
}

/// Black & White is greyed with the reference alert, running it is refused with it, and nothing
/// changes.
fn assert_unavailable(s: &mut Session, case: &str) {
    assert_eq!(s.disabled_reason(BW).as_deref(), Some(BLACK_WHITE_UNAVAILABLE), "{case}: greyed");
    let (before, was) = (steps(s), pixel(s));
    let err = s.execute(BW, json!({})).expect_err("Black & White is refused").to_string();
    assert!(err.contains(BLACK_WHITE_UNAVAILABLE), "{case}: {err}");
    assert_eq!(steps(s), before, "{case}: no history state");
    assert_eq!(pixel(s), was, "{case}: no pixel change");
}

/// Black & White is available and runs as one history state that turns the red half gray.
fn assert_works(s: &mut Session, case: &str) {
    assert!(s.is_enabled(BW), "{case}: {:?}", s.disabled_reason(BW));
    let (before, was) = (steps(s), pixel(s));
    s.execute(BW, json!({})).unwrap_or_else(|e| panic!("{case}: {e}"));
    assert_eq!(steps(s), before + 1, "{case}: one history state");
    let now = pixel(s);
    assert_ne!(now, was, "{case}: the pixels changed");
    assert!((now[0] - now[1]).abs() < 1e-3 && (now[1] - now[2]).abs() < 1e-3, "{case}: gray, got {now:?}");
}

#[test]
fn grayscale_cmyk_lab_and_indexed_documents_grey_black_white() {
    for (mode, params) in [
        ("grayscale", json!({})),
        ("cmyk", json!({})),
        ("lab", json!({})),
        ("indexedColor", json!({"palette": "exact", "forced": "none", "dither": "none", "transparency": false})),
    ] {
        let mut s = session();
        s.execute(&format!("image.mode.{mode}"), params).unwrap();
        let listed = crate::commands::find(BW).map(|c| (c.enabled)(&s));
        assert_eq!(listed, Some(Err(BLACK_WHITE_UNAVAILABLE.to_string())), "{mode}: the command list's check");
        assert_unavailable(&mut s, mode);
    }
}

#[test]
fn a_hidden_pixel_layer_greys_black_white() {
    let mut s = session();
    s.execute("layer.setProps", json!({"visible": false})).unwrap();
    assert_unavailable(&mut s, "hidden pixel layer");
    assert!(s.is_enabled("image.adjustments.levels"), "the other adjustments keep their checks");
}

#[test]
fn an_empty_pixel_layer_stops_with_the_empty_layer_alert() {
    let mut s = session();
    s.execute("layer.new.layer", json!({"name": "Empty"})).unwrap();
    assert!(s.is_enabled(BW), "the item stays enabled; the alert comes on running it");
    let before = steps(&s);
    let err = s.execute(BW, json!({})).expect_err("Black & White is refused").to_string();
    assert_eq!(err, BLACK_WHITE_EMPTY);
    assert_eq!(steps(&s), before, "no history state");
}

#[test]
fn explicit_pixels_in_cmyk_are_refused_over_a_targeted_alpha_channel() {
    let mut s = session();
    s.execute("select.rect", json!({"x": 2, "y": 2, "width": 6, "height": 6})).unwrap();
    s.execute("select.saveSelection", json!({"name": "Alpha 1"})).unwrap();
    s.execute("image.mode.cmyk", json!({})).unwrap();
    s.execute("channel.target", json!({"channel": {"alpha": 0}})).unwrap();
    assert!(s.is_enabled(BW), "the alpha channel keeps the shared check: {:?}", s.disabled_reason(BW));
    let p = json!({"target": "pixels"});
    assert_eq!(s.disabled_reason_with(BW, &p).as_deref(), Some(BLACK_WHITE_UNAVAILABLE), "the layer's pixels in CMYK");
    let (before, was) = (steps(&s), pixel(&s));
    let err = s.execute(BW, p).expect_err("Black & White is refused").to_string();
    assert!(err.contains(BLACK_WHITE_UNAVAILABLE), "{err}");
    assert_eq!(steps(&s), before, "no history state");
    assert_eq!(pixel(&s), was, "no pixel change");
}

#[test]
fn a_hidden_layer_named_by_layer_is_refused() {
    let mut s = session();
    let paint = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.setProps", json!({"layer": paint.0, "visible": false})).unwrap();
    let background = s.active().unwrap().doc.layers[0].id;
    s.execute("layer.select", json!({"layer": background.0})).unwrap();
    assert!(s.is_enabled(BW), "the visible Background is active");
    let p = json!({"layer": paint.0});
    assert_eq!(s.disabled_reason_with(BW, &p).as_deref(), Some(BLACK_WHITE_UNAVAILABLE));
    let (before, was) = (steps(&s), layer_pixel(&s, paint));
    let err = s.execute(BW, p).expect_err("Black & White is refused").to_string();
    assert!(err.contains(BLACK_WHITE_UNAVAILABLE), "{err}");
    assert_eq!(steps(&s), before, "no history state");
    assert_eq!(layer_pixel(&s, paint), was, "the hidden layer is unchanged");
}

#[test]
fn a_visible_layer_named_by_layer_works_while_a_hidden_one_is_active() {
    let mut s = session();
    let paint = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.new.layer", json!({"name": "Hidden"})).unwrap();
    s.execute("layer.setProps", json!({"visible": false})).unwrap();
    assert_eq!(s.disabled_reason(BW).as_deref(), Some(BLACK_WHITE_UNAVAILABLE), "the hidden layer is active");
    let p = json!({"layer": paint.0});
    assert_eq!(s.disabled_reason_with(BW, &p), None, "the visible layer named");
    let before = steps(&s);
    s.execute(BW, p).expect("Black & White runs on the named layer");
    assert_eq!(steps(&s), before + 1, "one history state");
    let now = layer_pixel(&s, paint);
    assert!((now[0] - now[1]).abs() < 1e-3 && (now[1] - now[2]).abs() < 1e-3, "the named layer turned gray, got {now:?}");
}

#[test]
fn an_empty_layer_in_duotone_keeps_todays_behaviour() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 16, "height": 16, "mode": "gray", "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Empty"})).unwrap();
    s.execute("image.mode.duotone", json!({"type": "monotone"})).unwrap();
    let before = steps(&s);
    s.execute(BW, json!({})).expect("Duotone was not measured, so the empty layer is not refused");
    assert_eq!(steps(&s), before + 1, "a history state, as before");
}

#[test]
fn rgb_at_8_16_and_32_bit_works() {
    for depth in [8, 16, 32] {
        assert_works(&mut session_at(depth), &format!("RGB {depth} bit"));
    }
}

#[test]
fn a_visible_pixel_layer_works() {
    assert_works(&mut session(), "visible pixel layer");
}

#[test]
fn the_background_works() {
    let mut s = session();
    s.execute("layer.delete", json!({})).unwrap();
    fill_left_half(&mut s);
    let st = s.active().unwrap();
    assert!(st.doc.is_background(st.active_layer.unwrap()), "the Background is active");
    assert_works(&mut s, "Background");
}

#[test]
fn lock_transparent_pixels_works() {
    let mut s = session();
    s.execute("layer.setProps", json!({"locks": {"transparency": true}})).unwrap();
    assert_works(&mut s, "Lock transparent pixels");
}

#[test]
fn a_selection_works() {
    let mut s = session();
    s.execute("select.rect", json!({"x": 2, "y": 2, "width": 4, "height": 4})).unwrap();
    assert_works(&mut s, "selection");
}

#[test]
fn a_smart_object_records_a_smart_filter() {
    let mut s = session();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    assert!(s.is_enabled(BW), "{:?}", s.disabled_reason(BW));
    let before = steps(&s);
    s.execute(BW, json!({})).expect("recorded as a smart filter");
    assert_eq!(steps(&s), before + 1, "one history state");
    let st = s.active().unwrap();
    let Some(photocraft_doc::LayerContent::Smart(sm)) = st.doc.layer(st.active_layer.unwrap()).map(|l| &l.content) else { panic!("a smart object") };
    assert_eq!(sm.smart_filters.len(), 1);
}

#[test]
fn unmeasured_modes_and_targets_keep_todays_behaviour() {
    // Multichannel and Duotone keep the shared check.
    let mut s = session();
    s.execute("image.mode.multichannel", json!({})).unwrap();
    assert_eq!(black_white_enabled(&s), has_adjustable(&s), "Multichannel");
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 16, "height": 16, "mode": "gray", "background": "white"})).unwrap();
    s.execute("image.mode.duotone", json!({"type": "monotone"})).unwrap();
    assert_eq!(black_white_enabled(&s), has_adjustable(&s), "Duotone");
    // Also on a hidden pixel layer there, which Duotone keeps.
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 16, "height": 16, "mode": "gray", "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Paint"})).unwrap();
    s.execute("layer.setProps", json!({"visible": false})).unwrap();
    s.execute("image.mode.duotone", json!({"type": "monotone"})).unwrap();
    assert_eq!(has_adjustable(&s), Ok(()), "Duotone, hidden pixel layer");
    assert_eq!(black_white_enabled(&s), Ok(()), "Duotone, hidden pixel layer");
    // Bitmap keeps the refusal all Image › Adjustments but Invert have there.
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 16, "height": 16, "mode": "gray", "background": "white"})).unwrap();
    s.execute("image.mode.bitmap", json!({})).unwrap();
    assert_eq!(s.disabled_reason(BW).as_deref(), Some("not available in Bitmap mode"));

    // A targeted layer mask on a hidden layer, and an alpha channel in a Grayscale document.
    let mut s = session();
    s.execute("layer.setProps", json!({"visible": false})).unwrap();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s.execute("channel.target.layerMask", json!({})).unwrap();
    assert!(s.is_enabled(BW), "layer mask: {:?}", s.disabled_reason(BW));
    let mut s = session();
    s.execute("select.rect", json!({"x": 2, "y": 2, "width": 6, "height": 6})).unwrap();
    s.execute("select.saveSelection", json!({"name": "Alpha 1"})).unwrap();
    s.execute("image.mode.grayscale", json!({})).unwrap();
    s.execute("channel.target", json!({"channel": {"alpha": 0}})).unwrap();
    assert_eq!(black_white_enabled(&s), has_adjustable(&s), "alpha channel in Grayscale");
    assert!(s.is_enabled(BW), "alpha channel in Grayscale: {:?}", s.disabled_reason(BW));
    let _: Value = s.execute(BW, json!({})).expect("the alpha channel is adjusted");
}
