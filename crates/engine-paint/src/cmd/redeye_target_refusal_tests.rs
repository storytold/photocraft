//! #12574: `paint.redEye` refuses a hidden target layer, a mask or channel target, an adjustment
//! layer and several selected layers (when no `layer` is named) with the reference's alert text, and
//! leaves the document and History unchanged. A named layer, or one visible pixel layer alone,
//! still gets its pupil corrected.

use serde_json::{Value, json};

use super::*;

/// A document with a red pupil on its own layer ("pupil"), the active one.
fn eye() -> (Session, u64) {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 64, "height": 64, "mode": "rgb", "depth": 8})).unwrap();
    let id = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
    s.execute("paint.pencil", json!({"points": [[32, 32]], "size": 14, "hardness": 100, "color": "#f21818"})).unwrap();
    (s, id)
}

fn click() -> Value {
    json!({"x": 32, "y": 32})
}

fn refused(s: &mut Session, params: Value, want: &str) {
    let (doc, steps) = (s.active().unwrap().doc.clone(), s.active().unwrap().history.past_len());
    let err = s.execute("paint.redEye", params).unwrap_err().to_string();
    assert_eq!(err, want);
    assert!(std::sync::Arc::ptr_eq(&doc, &s.active().unwrap().doc), "document unchanged");
    assert_eq!(s.active().unwrap().history.past_len(), steps, "no history state");
}

#[test]
fn a_hidden_target_layer() {
    let (mut s, id) = eye();
    s.execute("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
    refused(&mut s, click(), crate::hidden_target::HIDDEN);
}

#[test]
fn a_targeted_layer_mask_and_an_adjustment_layer() {
    let (mut s, _) = eye();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    refused(&mut s, json!({"x": 32, "y": 32, "target": "mask"}), NOT_PIXEL_REFUSAL);
    let (mut s, _) = eye();
    s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    refused(&mut s, click(), NOT_PIXEL_REFUSAL);
}

/// The Done-when says "a mask or channel target": an alpha channel and the Quick Mask are refused
/// too, with no `layer` named, and the document and History stay as they were.
#[test]
fn an_alpha_channel_and_the_quick_mask() {
    let (mut s, _) = eye();
    s.execute("channel.new", json!({"name": "Sky sel", "fill": "white"})).unwrap();
    refused(&mut s, json!({"x": 32, "y": 32, "target": {"channel": 0}}), NOT_PIXEL_REFUSAL);
    let (mut s, _) = eye();
    s.execute("select.editInQuickMaskMode", json!({"on": true})).unwrap();
    refused(&mut s, json!({"x": 32, "y": 32, "target": "quickMask"}), NOT_PIXEL_REFUSAL);
}

#[test]
fn several_selected_layers_unless_one_is_named() {
    let (mut s, pupil) = eye();
    let top = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.select", json!({"layers": [pupil, top]})).unwrap();
    refused(&mut s, click(), MANY_LAYERS_REFUSAL);
    let named = s.execute("paint.redEye", json!({"x": 32, "y": 32, "layer": pupil})).unwrap();
    assert!(named["pixels"].as_u64().unwrap() > 0, "a named layer is corrected: {named}");
}

/// `"layer": null` names no layer, as everywhere else: it neither skips the several-layers refusal
/// nor stops a lone layer from being corrected.
#[test]
fn a_null_layer_names_no_layer() {
    let (mut s, _) = eye();
    let r = s.execute("paint.redEye", json!({"x": 32, "y": 32, "layer": null})).unwrap();
    assert!(r["pixels"].as_u64().unwrap() > 0, "a null layer falls back to the active one: {r}");
    let (mut s, pupil) = eye();
    let top = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.select", json!({"layers": [pupil, top]})).unwrap();
    refused(&mut s, json!({"x": 32, "y": 32, "layer": null}), MANY_LAYERS_REFUSAL);
}

#[test]
fn one_visible_pixel_layer_is_corrected() {
    let (mut s, _) = eye();
    let r = s.execute("paint.redEye", click()).unwrap();
    assert!(r["pixels"].as_u64().unwrap() > 0, "{r}");
}

/// A selection moved but not yet dropped floats; a refused click must not drop it (#13169's
/// admission step). An adjustment layer can't float anything, so only the other two reach it.
#[test]
fn a_refused_click_leaves_a_floating_selection_floating() {
    for case in ["mask", "several"] {
        let (mut s, pupil) = eye();
        let mut params = click();
        if case == "mask" {
            s.execute("layer.layerMask.revealAll", json!({})).unwrap();
            params = json!({"x": 32, "y": 32, "target": "mask"});
        } else {
            let top = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
            s.execute("layer.select", json!({"layers": [top, pupil]})).unwrap();
        }
        s.execute("select.rect", json!({"x": 20, "y": 20, "width": 20, "height": 20})).unwrap();
        s.execute("select.float", json!({"dx": 3, "dy": 2, "target": if case == "mask" { "mask" } else { "pixels" }})).unwrap();
        let steps = s.active().unwrap().history.past_len();
        assert!(crate::float_cmds::floating(s.active().unwrap()).is_some(), "{case}: the piece floats");
        s.execute("paint.redEye", params).unwrap_err();
        let st = s.active().unwrap();
        assert!(crate::float_cmds::floating(st).is_some(), "{case}: the refused click leaves the piece floating");
        assert_eq!(st.history.past_len(), steps, "{case}: no history state");
    }
}
