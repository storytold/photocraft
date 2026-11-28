//! Dodge in an 8-bit Grayscale document and on a layer mask uses the reference's single-channel
//! curve (#12689); RGB documents and the cases the reference wasn't measured in keep the RGB
//! curve. The helpers live in `burn_grayscale_tests`.

use super::burn_grayscale_tests::{
    GREYS, assert_near, click_each, curve_is_monotone_and_bounded, doc, keeps_rgb_curve_where_not_measured, live_matches_commit, masked,
};
use serde_json::json;

/// Measured in the reference: one default Dodge click in an 8-bit Grayscale document.
const REFERENCE: [f32; 6] = [46.0, 117.0, 138.0, 187.0, 202.0, 227.0];

#[test]
fn dodge_in_8_bit_grayscale_matches_the_reference() {
    let got = click_each(&mut doc("gray", 8), "paint.dodge", &json!({}));
    assert_near(&got, &REFERENCE, 2.0, "Dodge, 8-bit Grayscale");
}

#[test]
fn dodge_on_a_layer_mask_matches_grayscale() {
    let mut s = masked(8);
    let before = s.active().unwrap().history.past_len();
    let got = click_each(&mut s, "paint.dodge", &json!({"target": "mask"}));
    assert_near(&got, &REFERENCE, 2.0, "Dodge, layer mask");
    let h = &s.active().unwrap().history;
    assert_eq!((h.past_len(), h.undo_label()), (before + GREYS.len(), Some("Dodge Tool")));
}

#[test]
fn dodge_on_an_adjustment_layers_white_mask_keeps_it_white() {
    let mut s = doc("gray", 8);
    s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    let before = s.active().unwrap().history.past_len();
    s.execute("paint.dodge", json!({"points": [[30, 30]], "size": 40, "hardness": 100, "target": "mask"})).unwrap();
    let d = s.active().unwrap();
    let mask = &d.doc.layer(d.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface;
    assert_eq!(mask.rgba(30, 30)[0], 1.0);
    assert_eq!((d.history.past_len(), d.history.undo_label()), (before + 1, Some("Dodge Tool")));
}

#[test]
fn dodge_keeps_rgb_results() {
    // Pinned from main d5582062a0: neutral greys and a colour in an 8-bit RGB document.
    let got = click_each(&mut doc("rgb", 8), "paint.dodge", &json!({}));
    assert_near(&got, &[46.0, 140.0, 161.0, 198.0, 209.0, 228.0], 0.0, "Dodge, RGB greys");
}

#[test]
fn dodge_keeps_the_rgb_curve_where_the_reference_was_not_measured() {
    keeps_rgb_curve_where_not_measured("paint.dodge", false);
}

#[test]
fn live_dodge_in_grayscale_matches_the_commit() {
    live_matches_commit("paint.dodge");
}

#[test]
fn single_channel_dodge_curve_is_monotone_and_bounded() {
    curve_is_monotone_and_bounded(false);
}
