//! Image > Adjustments > Exposure is refused in the CMYK, Lab, Indexed Color and Multichannel
//! documents measured in #14630, Bitmap stays refused, RGB and Grayscale keep working at 8, 16 and
//! 32 bit, and Duotone (not measured) keeps today's behaviour.

use crate::Session;
use photocraft_geom::Rect;
use serde_json::json;

const EXPOSURE: &str = "image.adjustments.exposure";

fn new_doc(mode: &str, depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "mode": mode, "depth": depth, "background": "white"})).unwrap();
    s
}

fn pixel(s: &Session) -> [f32; 4] {
    photocraft_compose::render(&s.active().unwrap().doc, Rect::new(2, 2, 3, 3)).px[0]
}

fn assert_refused(mut s: Session, what: &str) {
    assert!(!s.is_enabled(EXPOSURE), "{what}: the menu entry is greyed out");
    let steps = s.active().unwrap().history.past_len();
    let before = pixel(&s);
    assert!(s.execute(EXPOSURE, json!({"exposure": -1})).is_err(), "{what}: running it is refused");
    assert_eq!(s.active().unwrap().history.past_len(), steps, "{what}: no history state");
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
    indexed.execute("image.mode.indexedColor", json!({"palette": "exact", "dither": "none", "forced": "none"})).unwrap();
    assert_refused(indexed, "Indexed Color");

    let mut multi = new_doc("rgb", 8);
    multi.execute("image.mode.multichannel", json!({})).unwrap();
    assert_refused(multi, "Multichannel");

    let mut bitmap = new_doc("grayscale", 8);
    bitmap.execute("image.mode.bitmap", json!({"method": "threshold"})).unwrap();
    assert_refused(bitmap, "Bitmap");
}

#[test]
fn rgb_and_grayscale_documents_keep_exposure_at_every_depth() {
    for mode in ["rgb", "grayscale"] {
        for depth in [8, 16, 32] {
            let mut s = new_doc(mode, depth);
            assert!(s.is_enabled(EXPOSURE), "{mode} {depth} bit");
            let steps = s.active().unwrap().history.past_len();
            let before = pixel(&s);
            s.execute(EXPOSURE, json!({"exposure": -1})).unwrap();
            assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "{mode} {depth} bit: one history state");
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
    .unwrap();
    assert!(s.is_enabled(EXPOSURE), "Duotone is not refused by this item");
}

/// The refusal names the document's mode, like the other RGB-or-Grayscale checks.
#[test]
fn the_refusal_names_the_documents_mode() {
    let s = new_doc("cmyk", 8);
    let why = crate::adjust_cmds::exposure_enabled(&s).unwrap_err();
    assert!(why.contains("CMYK Color"), "{why}");
}

/// A targeted Quick Mask is a gray plane in any mode: not measured, so it keeps today's behaviour
/// and Exposure still edits it in a CMYK document.
#[test]
fn a_quick_mask_in_a_cmyk_document_keeps_todays_behaviour() {
    let mut s = new_doc("cmyk", 8);
    s.execute("select.editInQuickMaskMode", json!({"on": true})).unwrap();
    assert!(s.is_enabled(EXPOSURE), "the Quick Mask is not refused by this item");
    let steps = s.active().unwrap().history.past_len();
    s.execute(EXPOSURE, json!({"exposure": -1, "target": "quickMask"})).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "one history state");
}
