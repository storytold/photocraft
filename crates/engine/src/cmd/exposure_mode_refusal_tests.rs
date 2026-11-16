//! Image > Adjustments > Exposure is refused in the CMYK, Lab, Indexed Color and Multichannel
//! documents measured in #14630, Bitmap stays refused, RGB and Grayscale keep working at 8, 16 and
//! 32 bit, and Duotone (not measured) keeps today's behaviour.

use crate::Session;
use photocraft_geom::Rect;
use serde_json::json;

const EXPOSURE: &str = "image.adjustments.exposure";

fn new_doc(mode: &str, depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "mode": mode, "depth": depth, "background": "white"})).expect("file.new");
    s
}

fn pixel(s: &Session) -> [f32; 4] {
    photocraft_compose::render(&s.active().expect("an active document").doc, Rect::new(2, 2, 3, 3)).px[0]
}

fn assert_refused(mut s: Session, what: &str) {
    assert!(!s.is_enabled(EXPOSURE), "{what}: the menu entry is greyed out");
    let steps = s.active().expect("an active document").history.past_len();
    let before = pixel(&s);
    assert!(s.execute(EXPOSURE, json!({"exposure": -1})).is_err(), "{what}: running it is refused");
    assert_eq!(s.active().expect("an active document").history.past_len(), steps, "{what}: no history state");
    assert_eq!(pixel(&s), before, "{what}: no pixel changes");
}

#[test]
fn cmyk_and_lab_documents_refuse_exposure() {
    for depth in [8, 16] {
        assert_refused(new_doc("cmyk", depth), &format!("CMYK {depth} bit"));
        assert_refused(new_doc("lab", depth), &format!("Lab {depth} bit"));
    }
}

#[test]
fn indexed_multichannel_and_bitmap_documents_refuse_exposure() {
    let mut indexed = new_doc("rgb", 8);
    indexed.execute("image.mode.indexedColor", json!({"palette": "exact", "dither": "none", "forced": "none"})).expect("indexed color");
    assert_refused(indexed, "Indexed Color");

    let mut multi = new_doc("rgb", 8);
    multi.execute("image.mode.multichannel", json!({})).expect("multichannel");
    assert_refused(multi, "Multichannel");

    let mut bitmap = new_doc("grayscale", 8);
    bitmap.execute("image.mode.bitmap", json!({"method": "threshold"})).expect("test step");
    assert_refused(bitmap, "Bitmap");
}

#[test]
fn rgb_and_grayscale_documents_keep_exposure_at_every_depth() {
    for mode in ["rgb", "grayscale"] {
        for depth in [8, 16, 32] {
            let mut s = new_doc(mode, depth);
            assert!(s.is_enabled(EXPOSURE), "{mode} {depth} bit");
            let steps = s.active().expect("an active document").history.past_len();
            let before = pixel(&s);
            s.execute(EXPOSURE, json!({"exposure": -1})).expect("test step");
            assert_eq!(s.active().expect("an active document").history.past_len(), steps + 1, "{mode} {depth} bit: one history state");
            assert!(pixel(&s)[0] < before[0], "{mode} {depth} bit darkens: {:?} -> {:?}", before, pixel(&s));
        }
    }
}

#[test]
fn the_exposure_adjustment_layer_and_other_adjustments_are_unchanged() {
    let s = new_doc("cmyk", 8);
    assert!(s.is_enabled("layer.newAdjustmentLayer.exposure"), "the adjustment layer is not this item");
    assert!(s.is_enabled("image.adjustments.levels"), "other adjustments are not this item");
}

/// Duotone was not measured, so it keeps today's behaviour (the layer check alone).
#[test]
fn duotone_documents_keep_todays_behaviour() {
    let mut s = new_doc("grayscale", 8);
    s.edit("duotone", |d, _| {
        d.mode = photocraft_color::ColorMode::Duotone;
        Ok(())
    })
    .expect("test step");
    assert!(s.is_enabled(EXPOSURE), "Duotone is not refused by this item");
}

/// The refusal names the document's mode as the Image › Mode menu does.
#[test]
fn the_refusal_names_the_documents_mode() {
    let s = new_doc("cmyk", 8);
    let why = crate::adjust_cmds::exposure_enabled(&s).unwrap_err();
    assert!(why.contains("CMYK Color"), "{why}");
}

/// A Quick Mask targeted in the Channels panel is a gray plane in any mode: not measured, so it keeps
/// working: Exposure still edits it in a CMYK document.
#[test]
fn a_quick_mask_in_a_cmyk_document_keeps_todays_behaviour() {
    let mut s = new_doc("cmyk", 8);
    s.execute("select.editInQuickMaskMode", json!({"on": true})).expect("test step");
    assert!(s.is_enabled(EXPOSURE), "the Quick Mask is not refused by this item");
    let steps = s.active().expect("an active document").history.past_len();
    s.execute(EXPOSURE, json!({"exposure": -1, "target": "quickMask"})).expect("test step");
    assert_eq!(s.active().expect("an active document").history.past_len(), steps + 1, "one history state");
}

fn docs() -> Vec<(&'static str, Session)> {
    let mut indexed = new_doc("rgb", 8);
    indexed.execute("image.mode.indexedColor", json!({"palette": "exact", "dither": "none", "forced": "none"})).expect("indexed color");
    let mut multi = new_doc("rgb", 8);
    multi.execute("image.mode.multichannel", json!({})).expect("multichannel");
    vec![("cmyk", new_doc("cmyk", 8)), ("lab", new_doc("lab", 8)), ("indexed", indexed), ("multichannel", multi)]
}

/// An explicit alpha channel or Quick Mask target in the params is a gray plane in any mode, so the
/// document-mode refusal does not apply, whatever the Channels panel shows; a pixel target is still
/// refused.
#[test]
fn an_explicit_alpha_target_is_not_refused_in_other_modes() {
    for (mode, mut s) in docs() {
        s.execute("channel.new", json!({"name": "A", "fill": "white"})).expect("test step");
        s.execute("channel.target", json!({"channel": "composite"})).expect("test step");
        let steps = s.active().expect("an active document").history.past_len();
        let r = s.execute(EXPOSURE, json!({"exposure": -1, "target": {"channel": 0}}));
        assert!(r.is_ok(), "{mode} alpha: {r:?}");
        assert_eq!(s.active().expect("an active document").history.past_len(), steps + 1, "{mode}: one history state");
        let e = s.execute(EXPOSURE, json!({"exposure": -1, "target": "pixels"})).expect_err("pixels stay refused");
        assert!(e.to_string().contains("needs an RGB or Grayscale document"), "{mode}: the document-mode refusal, got {e}");
    }
}

#[test]
fn an_explicit_quick_mask_target_is_not_refused_in_other_modes() {
    for (mode, mut s) in docs() {
        s.execute("select.editInQuickMaskMode", json!({"on": true})).expect("test step");
        // The panel shows a colour channel (not the Quick Mask), so only the params name the plane.
        s.execute("channel.target", json!({"colors": [0]})).unwrap_or_else(|e| panic!("{mode}: showing a colour channel failed: {e:?}"));
        let steps = s.active().expect("an active document").history.past_len();
        let r = s.execute(EXPOSURE, json!({"exposure": -1, "target": "quickMask"}));
        assert!(r.is_ok(), "{mode} quick mask: {r:?}");
        assert_eq!(s.active().expect("an active document").history.past_len(), steps + 1, "{mode}: one history state");
    }
}

/// The explicit-target bypass covers only the modes the refusal covers: in RGB an explicit channel
/// target on a shape layer is still refused by the pixel-layer check.
#[test]
fn an_explicit_channel_target_on_a_shape_layer_is_still_refused_in_rgb() {
    for mode in ["rgb", "grayscale"] {
        let mut s = new_doc(mode, 8);
        s.execute("channel.new", json!({"name": "A", "fill": "white"})).expect("test step");
        s.execute("channel.target", json!({"channel": "composite"})).expect("test step");
        s.execute("shape.create", json!({"kind": "rect", "rect": [1, 1, 4, 4], "fill": "#ff0000"})).expect("test step");
        let steps = s.active().expect("an active document").history.past_len();
        let r = s.execute(EXPOSURE, json!({"exposure": -1, "target": {"channel": 0}}));
        assert!(r.is_err(), "{mode}: a shape layer is not a pixel layer: {r:?}");
        assert_eq!(s.active().expect("an active document").history.past_len(), steps, "{mode}: no history state");
    }
}

/// The bypass needs the named plane to exist: a missing alpha index, a huge one, or a Quick Mask that is
/// not there get the document-mode refusal, and never panic.
#[test]
fn a_target_naming_no_plane_keeps_the_mode_refusal() {
    let targets = [json!({"channel": 99}), json!({"channel": usize::MAX}), json!({"channel": u64::MAX}), json!("quickMask")];
    for (mode, mut s) in docs() {
        for t in &targets {
            let steps = s.active().expect("an active document").history.past_len();
            let e = s.execute(EXPOSURE, json!({"exposure": -1, "target": t})).expect_err("a missing plane is refused");
            assert!(e.to_string().contains("needs an RGB or Grayscale document"), "{mode} {t}: the document-mode refusal, got {e}");
            assert_eq!(s.active().expect("an active document").history.past_len(), steps, "{mode} {t}: no history state");
        }
    }
}

/// With the Channels panel on an existing alpha channel (or the Quick Mask) the panel must not decide for a
/// call whose params name a pixels target or a plane that does not exist: those get the mode refusal.
#[test]
fn a_params_target_beats_the_channels_panel_in_other_modes() {
    for (mode, mut s) in docs() {
        s.execute("channel.new", json!({"name": "A", "fill": "white"})).expect("test step");
        s.execute("channel.target", json!({"channel": 0})).expect("test step");
        for t in [json!("pixels"), json!({"channel": 99}), json!({"channel": u64::MAX}), json!("quickMask")] {
            let steps = s.active().expect("an active document").history.past_len();
            let e = s.execute(EXPOSURE, json!({"exposure": -1, "target": t})).expect_err("refused despite the panel");
            assert!(e.to_string().contains("needs an RGB or Grayscale document"), "{mode} {t}: the document-mode refusal, got {e}");
            assert_eq!(s.active().expect("an active document").history.past_len(), steps, "{mode} {t}: no history state");
        }
        // The panel still governs a call with no target of its own, and a named existing plane still works.
        assert!(s.execute(EXPOSURE, json!({"exposure": -1})).is_ok(), "{mode}: the panel's alpha takes a targetless call");
        assert!(s.execute(EXPOSURE, json!({"exposure": -1, "target": {"channel": 0}})).is_ok(), "{mode}: the named alpha works");
    }
}

#[test]
fn a_params_target_beats_the_quick_mask_panel_in_other_modes() {
    for (mode, mut s) in docs() {
        s.execute("select.editInQuickMaskMode", json!({"on": true})).expect("test step");
        let e = s.execute(EXPOSURE, json!({"exposure": -1, "target": "pixels"})).expect_err("pixels refused");
        assert!(e.to_string().contains("needs an RGB or Grayscale document"), "{mode}: got {e}");
        let e = s.execute(EXPOSURE, json!({"exposure": -1, "target": {"channel": 7}})).expect_err("missing alpha refused");
        assert!(e.to_string().contains("needs an RGB or Grayscale document"), "{mode}: got {e}");
        assert!(s.execute(EXPOSURE, json!({"exposure": -1})).is_ok(), "{mode}: the Quick Mask takes a targetless call");
    }
}

/// A present `target` key is explicit even when it is null or unrecognised: `target_of` reads it as Pixels, so
/// the panel (an existing alpha here) must not let Exposure edit layer pixels in these modes.
#[test]
fn a_null_or_typo_target_gets_the_mode_refusal_despite_the_panel() {
    for (mode, mut s) in docs() {
        s.execute("channel.new", json!({"name": "A", "fill": "white"})).expect("test step");
        s.execute("channel.target", json!({"channel": 0})).expect("test step");
        for t in [json!(null), json!("pixles"), json!(7), json!(true)] {
            let steps = s.active().expect("an active document").history.past_len();
            let e = s.execute(EXPOSURE, json!({"exposure": -1, "target": t})).expect_err("refused despite the panel");
            assert!(e.to_string().contains("needs an RGB or Grayscale document"), "{mode} {t}: the document-mode refusal, got {e}");
            assert_eq!(s.active().expect("an active document").history.past_len(), steps, "{mode} {t}: no history state");
        }
    }
}
