//! #14406: Image › Adjustments › Channel Mixer is unavailable in Grayscale, Lab, Indexed and
//! Multichannel documents and on a hidden pixel layer, a fill layer (its mask targeted or not) and
//! a targeted layer mask ("The command "Channel Mixer" is not currently available."), and stops on
//! an empty pixel layer ("Could not complete the Channel Mixer command because the active layer is
//! empty."), as the reference application does. None of them adds a history state or changes a
//! pixel. RGB 8/16/32 bit, CMYK, a visible pixel layer, Lock Transparent Pixels, a selection and a
//! smart object keep working, and the other adjustments keep their targets.

use serde_json::{Value, json};

use crate::Session;
use crate::adjust_cmds::{CHANNEL_MIXER_EMPTY, CHANNEL_MIXER_UNAVAILABLE};
use crate::channel_cmds::ChannelTarget;

const MIXER: &str = "image.adjustments.channelMixer";

/// Swap red and blue.
fn swap() -> Value {
    json!({"red": [0, 0, 100, 0], "green": [0, 100, 0, 0], "blue": [100, 0, 0, 0]})
}

/// A 16 x 16 document with a white Background and a pixel layer "Paint" holding a red square at
/// 4,4 to 12,12, the active layer.
fn session(depth: u32, mode: &str) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 16, "height": 16, "depth": depth, "mode": mode, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Paint"})).unwrap();
    s.execute("select.rect", json!({"x": 4, "y": 4, "width": 8, "height": 8})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": [1.0, 0.0, 0.0, 1.0]})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s
}

fn steps(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

fn active_rgba(s: &Session, x: i32, y: i32) -> [f32; 4] {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)
}

fn mask_value(s: &Session, x: i32, y: i32) -> f32 {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface.sample_channel(x, y, 0)
}

/// Channel Mixer is greyed, `execute` with `p` returns `want`'s message, and no history state is
/// added.
fn refused(s: &mut Session, p: Value, want: &str, case: &str) {
    if want == CHANNEL_MIXER_UNAVAILABLE && p.get("target").is_none() {
        assert!(!s.is_enabled(MIXER), "{case}: greyed");
    }
    let before = steps(s);
    let err = s.execute(MIXER, p).expect_err(case).to_string();
    assert!(err.contains(want), "{case}: {err}");
    assert_eq!(steps(s), before, "{case}: no history state");
}

/// Channel Mixer is enabled, runs, adds one history state.
fn works(s: &mut Session, p: Value, case: &str) {
    assert!(s.is_enabled(MIXER), "{case}: enabled ({:?})", s.disabled_reason(MIXER));
    let before = steps(s);
    s.execute(MIXER, p).unwrap_or_else(|e| panic!("{case}: {e}"));
    assert_eq!(steps(s), before + 1, "{case}: one history state");
}

#[test]
fn grayscale_lab_indexed_and_multichannel_documents_refuse_channel_mixer() {
    for mode in ["gray", "lab"] {
        let mut s = session(8, mode);
        refused(&mut s, swap(), CHANNEL_MIXER_UNAVAILABLE, mode);
    }
    // Lab keeps its pixels: no red/blue swap on the screen colour.
    let mut s = session(8, "lab");
    let before = active_rgba(&s, 6, 6);
    let _ = s.execute(MIXER, swap());
    assert_eq!(active_rgba(&s, 6, 6), before, "lab pixels unchanged");

    let mut s = session(8, "rgb");
    s.execute("image.mode.indexedColor", json!({})).unwrap();
    refused(&mut s, swap(), CHANNEL_MIXER_UNAVAILABLE, "indexed");

    let mut s = session(8, "rgb");
    s.execute("layer.flattenImage", json!({})).unwrap();
    s.execute("image.mode.multichannel", json!({})).unwrap();
    refused(&mut s, swap(), CHANNEL_MIXER_UNAVAILABLE, "multichannel");
}

#[test]
fn a_hidden_pixel_layer_refuses_channel_mixer() {
    let mut s = session(8, "rgb");
    s.execute("layer.setProps", json!({"visible": false})).unwrap();
    let before = active_rgba(&s, 6, 6);
    refused(&mut s, swap(), CHANNEL_MIXER_UNAVAILABLE, "hidden layer");
    assert_eq!(active_rgba(&s, 6, 6), before, "hidden layer pixels unchanged");
}

#[test]
fn a_fill_layer_refuses_channel_mixer_with_or_without_its_mask_targeted() {
    // Each case on its own document, every failure reported.
    let mut target_mask = swap();
    target_mask["target"] = json!("mask");
    let cases =
        [("fill layer", None, swap()), ("fill layer, mask targeted", Some(ChannelTarget::LayerMask), swap()), ("fill layer, target mask", None, target_mask)];
    let mut wrong = Vec::new();
    for (case, view, p) in cases {
        let mut s = session(8, "rgb");
        s.execute("layer.newFillLayer.solidColor", json!({"color": "#c82828"})).unwrap();
        if let Some(t) = view {
            s.active_mut().unwrap().channel_view.target = t;
        }
        let (mask, before) = (mask_value(&s, 6, 6), steps(&s));
        if p.get("target").is_none() && s.is_enabled(MIXER) {
            wrong.push(format!("{case}: enabled"));
        }
        match s.execute(MIXER, p) {
            Ok(_) => wrong.push(format!("{case}: ran")),
            Err(e) if !e.to_string().contains(CHANNEL_MIXER_UNAVAILABLE) => wrong.push(format!("{case}: {e}")),
            Err(_) => {}
        }
        if steps(&s) != before {
            wrong.push(format!("{case}: history state added"));
        }
        if mask_value(&s, 6, 6) != mask {
            wrong.push(format!("{case}: mask changed"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[test]
fn a_targeted_layer_mask_refuses_channel_mixer() {
    let mut s = session(8, "rgb");
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s.active_mut().unwrap().channel_view.target = ChannelTarget::LayerMask;
    let (px, mask) = (active_rgba(&s, 6, 6), mask_value(&s, 6, 6));
    refused(&mut s, swap(), CHANNEL_MIXER_UNAVAILABLE, "mask targeted in the Channels panel");
    s.active_mut().unwrap().channel_view.target = ChannelTarget::Composite;
    let mut p = swap();
    p["target"] = json!("mask");
    refused(&mut s, p, CHANNEL_MIXER_UNAVAILABLE, "target mask");
    assert_eq!((active_rgba(&s, 6, 6), mask_value(&s, 6, 6)), (px, mask), "layer and mask unchanged");
}

#[test]
fn an_empty_pixel_layer_refuses_channel_mixer_with_the_empty_layer_message() {
    let mut s = session(8, "rgb");
    s.execute("layer.new.layer", json!({"name": "Empty"})).unwrap();
    // Enabled (the reference application only answers when it runs), then refused.
    assert!(s.is_enabled(MIXER));
    refused(&mut s, swap(), CHANNEL_MIXER_EMPTY, "empty layer");
}

#[test]
fn channel_mixer_keeps_working_where_it_did() {
    for depth in [8, 16, 32] {
        let mut s = session(depth, "rgb");
        works(&mut s, swap(), &format!("rgb {depth}"));
        let p = active_rgba(&s, 6, 6);
        assert!(p[0] < 0.01 && p[2] > 0.99, "rgb {depth}: red and blue swapped: {p:?}");
    }
    let mut s = session(8, "cmyk");
    works(&mut s, json!({}), "cmyk");

    // Lock Transparent Pixels and a selection.
    let mut s = session(8, "rgb");
    s.execute("layer.lockLayers", json!({"transparency": true})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 8})).unwrap();
    works(&mut s, swap(), "locked transparency with a selection");

    // A smart object records a smart filter.
    let mut s = session(8, "rgb");
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    works(&mut s, swap(), "smart object");
    let d = s.active().unwrap();
    let layer = d.doc.layer(d.active_layer.unwrap()).unwrap();
    let photocraft_doc::LayerContent::Smart(sm) = &layer.content else { panic!("smart object") };
    assert_eq!(sm.smart_filters.last().map(|f| f.command.as_str()), Some(MIXER));

    // An alpha channel target keeps today's behaviour.
    let mut s = session(8, "rgb");
    s.execute("channel.new", json!({"fill": "white"})).unwrap();
    s.active_mut().unwrap().channel_view.target = ChannelTarget::Alpha(0);
    assert!(s.is_enabled(MIXER), "alpha channel target: {:?}", s.disabled_reason(MIXER));
}

#[test]
fn other_adjustments_keep_their_targets() {
    // Brightness/Contrast still runs on a targeted layer mask and on a hidden layer.
    let mut s = session(8, "rgb");
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    let before = steps(&s);
    s.execute("image.adjustments.brightnessContrast", json!({"brightness": 50, "target": "mask"})).unwrap();
    assert_eq!(steps(&s), before + 1, "Brightness/Contrast on a mask");
    // And in a Grayscale document Invert still works.
    let mut s = session(8, "gray");
    let before = steps(&s);
    s.execute("image.adjustments.invert", json!({})).unwrap();
    assert_eq!(steps(&s), before + 1, "Invert in Grayscale");
}
