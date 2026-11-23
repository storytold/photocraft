//! Tests of `paint.adjustmentBrush`, the Adjustment Brush Tool's stroke (#12519). Reference
//! values measured in the reference application: a 100 px soft dab centred at (600, 450) on an
//! 1200 x 900 document.

use photocraft_doc::{Adjustment, ColorMode, LayerContent, SampleType};
use serde_json::{Value, json};

use super::adjustment_brush_cmds::{ENTRIES, INDEXED_ALERT, NEW_LAYER_LABEL, STROKE_LABEL, enabled_in, starting_adjustment};
use crate::Session;

const CMD: &str = "paint.adjustmentBrush";

fn doc() -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 1200, "height": 900, "background": "white"})).unwrap();
    s
}

fn dab(kind: &str) -> Value {
    json!({"kind": kind, "points": [[600, 450, 1.0]], "size": 100, "hardness": 0.0, "opacity": 1.0, "flow": 1.0})
}

fn active(s: &Session) -> &photocraft_doc::Layer {
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap()
}

fn mask_at(s: &Session, x: i32, y: i32) -> f32 {
    active(s).mask.as_ref().expect("a layer mask").surface.sample_channel(x, y, 0) * 255.0
}

fn layer_count(s: &Session) -> usize {
    s.active().unwrap().doc.walk().len()
}

fn past(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

#[test]
fn first_stroke_makes_a_masked_adjustment_layer_in_one_history_state() {
    let mut s = doc();
    let before = (layer_count(&s), past(&s));
    let bg = s.active().unwrap().active_layer.unwrap();
    let r = s.execute(CMD, dab("levels")).unwrap();
    assert_eq!(r["created"], json!(true));
    assert_eq!(layer_count(&s), before.0 + 1);
    assert_eq!(past(&s), before.1 + 1, "one history state");
    let st = s.active().unwrap();
    assert_eq!(st.history.undo_label(), Some(NEW_LAYER_LABEL));
    let l = active(&s);
    assert_eq!(l.name, "Levels 1");
    assert!(matches!(l.content, LayerContent::Adjustment(Adjustment::Levels { .. })));
    assert_eq!((l.blend, l.opacity, l.clipped), (photocraft_doc::BlendMode::Normal, 1.0, false));
    // Directly above the Background.
    let order: Vec<_> = st.doc.layers.iter().map(|l| l.id).collect();
    assert_eq!(order, vec![bg, l.id]);
    // Black outside the dab, white at its centre.
    assert_eq!(mask_at(&s, 10, 10), 0.0);
    assert_eq!(mask_at(&s, 680, 450), 0.0);
    assert!(mask_at(&s, 600, 450) >= 250.0, "centre {}", mask_at(&s, 600, 450));
    // The mask is the target, white over black.
    assert!(st.channel_view.target == crate::channel_cmds::ChannelTarget::LayerMask);
    assert_eq!((s.tools.foreground, s.tools.background), ([1.0, 1.0, 1.0, 1.0], [0.0, 0.0, 0.0, 1.0]));
}

#[test]
fn a_soft_dab_falls_off_like_the_reference() {
    let mut s = doc();
    s.execute(CMD, dab("levels")).unwrap();
    // The reference row: 550: 22, 570: 119, 590: 237, 600: 254, 620: 172, 640: 52, 660: 2, 670: 0.
    let row: Vec<f32> = [550, 570, 590, 600, 620, 640, 660, 670].iter().map(|&x| mask_at(&s, x, 450)).collect();
    for w in row[..4].windows(2) {
        assert!(w[0] <= w[1], "rises to the centre: {row:?}");
    }
    for w in row[3..].windows(2) {
        assert!(w[0] >= w[1], "falls off past it: {row:?}");
    }
    assert!(row[7] <= 3.0 && row[0] < 128.0 && row[3] > 240.0, "{row:?}");
}

#[test]
fn layer_names_count_per_label() {
    let mut s = doc();
    s.execute(CMD, dab("levels")).unwrap();
    s.execute(CMD, dab("blackWhite")).unwrap();
    assert_eq!(active(&s).name, "Black & White 1");
    s.execute("layer.select", json!({"layer": {"background": true}})).unwrap();
    s.execute(CMD, dab("levels")).unwrap();
    assert_eq!(active(&s).name, "Levels 2");
}

#[test]
fn the_selection_becomes_the_mask_and_the_dab_is_not_clipped_to_it() {
    let mut s = doc();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 400, "height": 900})).unwrap();
    let before = past(&s);
    s.execute(CMD, json!({"kind": "curves", "points": [[400, 450, 1.0]], "size": 100, "hardness": 0.0})).unwrap();
    assert_eq!(past(&s), before + 1, "still one history state");
    assert!(s.active().unwrap().doc.selection.is_none(), "the selection is gone");
    assert_eq!(mask_at(&s, 100, 100), 255.0, "inside the selection");
    assert_eq!(mask_at(&s, 900, 100), 0.0, "outside it");
    // The dab reaches past the selection edge at x 400.
    assert!(mask_at(&s, 420, 450) > 100.0, "{}", mask_at(&s, 420, 450));
}

#[test]
fn inside_an_active_group_the_layer_goes_into_the_group() {
    let mut s = doc();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("layer.groupLayers", json!({})).unwrap();
    let group = s.active().unwrap().active_layer.unwrap();
    // An open group takes new layers, as Layer › New Adjustment Layer puts them.
    s.execute("layer.setExpanded", json!({"expanded": true})).unwrap();
    s.execute(CMD, dab("invert")).unwrap();
    let id = active(&s).id;
    let st = s.active().unwrap();
    let children = st.doc.layer(group).and_then(|g| g.children()).unwrap();
    assert!(children.iter().any(|l| l.id == id), "the new layer is inside the group");
}

#[test]
fn starting_values_are_read_back_from_the_new_layer() {
    for e in ENTRIES.iter().filter(|e| enabled_in(e.kind, ColorMode::Rgb, SampleType::U8)) {
        let mut s = doc();
        s.execute(CMD, dab(e.kind)).unwrap();
        let LayerContent::Adjustment(adj) = &active(&s).content else { panic!("{}: an adjustment layer", e.kind) };
        assert_eq!(adj.label(), e.label, "{}", e.kind);
        assert_eq!(*adj, starting_adjustment(e.kind, ColorMode::Rgb).unwrap(), "{}", e.kind);
    }
}

#[test]
fn starting_values_match_the_reference() {
    let close = |a: f32, b: f32| (a - b).abs() < 1e-3;
    let adj = |kind: &str| starting_adjustment(kind, ColorMode::Rgb).unwrap();
    let Adjustment::Levels { master, .. } = adj("levels") else { panic!() };
    assert!(close(master.in_black * 255.0, 20.0) && close(master.in_white * 255.0, 220.0) && close(master.gamma, 1.0));
    assert!(close(master.out_black, 0.0) && close(master.out_white, 1.0));
    let Adjustment::Curves { master, .. } = adj("curves") else { panic!() };
    let pts: Vec<(f32, f32)> = master.iter().map(|p| (p.input * 255.0, p.output * 255.0)).collect();
    assert_eq!(pts.len(), 3);
    assert!(close(pts[1].0, 127.0) && close(pts[1].1, 191.0), "{pts:?}");
    assert_eq!(adj("exposure"), Adjustment::Exposure { exposure: 1.0, offset: 0.0, gamma: 1.0 });
    let Adjustment::HueSaturation { hue, saturation, lightness, colorize, .. } = adj("hueSaturation") else { panic!() };
    assert_eq!((hue, saturation, lightness, colorize), (0.0, 30.0, 0.0, false));
    let Adjustment::ColorBalance { shadows, midtones, highlights, preserve_luminosity } = adj("colorBalance") else { panic!() };
    assert_eq!((shadows, midtones, highlights, preserve_luminosity), ([0.0; 3], [20.0, -15.0, 15.0], [0.0; 3], true));
    assert_eq!(adj("blackWhite"), Adjustment::BlackWhite { weights: [40.0, 60.0, 40.0, 60.0, 20.0, 80.0], tint: None });
    let Adjustment::PhotoFilter { density, preserve_luminosity, .. } = adj("photoFilter") else { panic!() };
    assert!(close(density, 0.25) && preserve_luminosity);
    let Adjustment::ChannelMixer { matrix, monochrome } = adj("channelMixer") else { panic!() };
    assert!(!monochrome);
    let want = [[0.8, 0.1, 0.1, 0.05], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]];
    assert!(matrix.iter().flatten().zip(want.iter().flatten()).all(|(a, b)| close(*a, *b)), "{matrix:?}");
    assert!(matches!(adj("colorLookup"), Adjustment::ColorLookup { lut: Some(_), .. }));
    let Adjustment::SelectiveColor { relative, adjustments } = adj("selectiveColor") else { panic!() };
    assert!(relative);
    assert_eq!(adjustments[0], [-20.0, 100.0, -60.0, 40.0]);
    assert_eq!(adj("invert"), Adjustment::Invert);
    assert_eq!(adj("posterize"), Adjustment::Posterize { levels: 4 });
    assert!(matches!(adj("threshold"), Adjustment::Threshold { level } if close(level * 255.0, 128.0)));
    let Adjustment::GradientMap { stops, reverse, dither, .. } = adj("gradientMap") else { panic!() };
    assert_eq!((stops.first().map(|s| s.1), stops.last().map(|s| s.1), reverse, dither), (Some([0.0; 3]), Some([1.0; 3]), false, false));
}

#[test]
fn a_later_stroke_paints_the_active_layers_mask() {
    let mut s = doc();
    s.execute(CMD, dab("levels")).unwrap();
    let (id, n, depth) = (active(&s).id, layer_count(&s), past(&s));
    assert_eq!(mask_at(&s, 900, 450), 0.0);
    let r = s.execute(CMD, json!({"kind": "levels", "points": [[900, 450, 1.0]], "size": 100, "hardness": 0.0})).unwrap();
    assert_eq!(r["created"], json!(false));
    assert_eq!((active(&s).id, layer_count(&s), past(&s)), (id, n, depth + 1), "no new layer, one more state");
    assert_eq!(s.active().unwrap().history.undo_label(), Some(STROKE_LABEL));
    assert!(mask_at(&s, 900, 450) >= 250.0);
}

#[test]
fn a_layer_made_by_new_adjustment_layer_takes_the_stroke_too() {
    let mut s = doc();
    s.execute("layer.newAdjustmentLayer.curves", json!({})).unwrap();
    let (id, n) = (active(&s).id, layer_count(&s));
    s.execute(CMD, dab("curves")).unwrap();
    assert_eq!((active(&s).id, layer_count(&s)), (id, n));
}

#[test]
fn another_kind_makes_a_new_layer_above_the_active_one() {
    let mut s = doc();
    s.execute(CMD, dab("levels")).unwrap();
    let first = active(&s).id;
    s.execute(CMD, dab("exposure")).unwrap();
    let st = s.active().unwrap();
    let order: Vec<_> = st.doc.layers.iter().map(|l| l.id).collect();
    assert_eq!(order.len(), 3);
    assert_eq!(order[1], first);
    assert_eq!(active(&s).name, "Exposure 1");
}

#[test]
fn subtract_paints_black_and_swaps_the_colours() {
    let mut s = doc();
    s.execute(CMD, json!({"kind": "levels", "points": [[600, 450], [800, 450]], "size": 100, "hardness": 1.0})).unwrap();
    assert!(mask_at(&s, 700, 450) >= 250.0);
    s.execute(CMD, json!({"kind": "levels", "subtract": true, "points": [[700, 450]], "size": 40, "hardness": 1.0})).unwrap();
    assert_eq!(mask_at(&s, 700, 450), 0.0);
    assert!(mask_at(&s, 610, 450) >= 250.0, "the rest stays");
    assert_eq!((s.tools.foreground, s.tools.background), ([0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]));
    s.execute(CMD, json!({"kind": "levels", "points": [[700, 450]], "size": 40, "hardness": 1.0})).unwrap();
    assert!(mask_at(&s, 700, 450) >= 250.0, "Add paints white again");
    assert_eq!(s.tools.foreground, [1.0, 1.0, 1.0, 1.0]);
}

#[test]
fn entries_grey_out_per_mode_and_depth() {
    let off = |mode, depth| -> Vec<&str> { ENTRIES.iter().map(|e| e.kind).filter(|k| !enabled_in(k, mode, depth)).collect() };
    let missing = ["light", "colorVibrance", "clarityDehaze", "grain"];
    let with = |more: &[&'static str]| -> Vec<&'static str> { ENTRIES.iter().map(|e| e.kind).filter(|k| missing.contains(k) || more.contains(k)).collect() };
    assert_eq!(off(ColorMode::Rgb, SampleType::U8), with(&[]));
    assert_eq!(off(ColorMode::Rgb, SampleType::U16), with(&[]));
    assert_eq!(off(ColorMode::Rgb, SampleType::F32), with(&["invert", "posterize", "threshold", "gradientMap"]));
    assert_eq!(
        off(ColorMode::Grayscale, SampleType::U8),
        with(&["hueSaturation", "colorBalance", "blackWhite", "photoFilter", "channelMixer", "colorLookup", "selectiveColor"])
    );
    assert_eq!(off(ColorMode::Cmyk, SampleType::U8), with(&["exposure", "blackWhite"]));
    assert_eq!(off(ColorMode::Lab, SampleType::U8), with(&["exposure", "blackWhite", "channelMixer", "selectiveColor"]));
}

#[test]
fn a_greyed_entry_is_refused_and_an_offered_one_works_in_other_modes() {
    for (mode, refused, works) in [
        ("image.mode.grayscale", "hueSaturation", "levels"),
        ("image.mode.cmyk", "exposure", "curves"),
        ("image.mode.lab", "channelMixer", "invert"),
        ("image.mode.bits32", "posterize", "exposure"),
        ("image.mode.bits16", "", "gradientMap"),
    ] {
        let mut s = doc();
        s.execute(mode, json!({})).unwrap();
        if !refused.is_empty() {
            let before = layer_count(&s);
            assert!(s.execute(CMD, dab(refused)).is_err(), "{mode}: {refused}");
            assert_eq!(layer_count(&s), before);
        }
        s.execute(CMD, dab(works)).unwrap_or_else(|e| panic!("{mode}: {works}: {e}"));
        assert!(matches!(active(&s).content, LayerContent::Adjustment(_)));
    }
}

#[test]
fn indexed_color_refuses_the_stroke_with_the_alert() {
    let mut s = doc();
    s.execute("image.mode.indexedColor", json!({})).unwrap();
    let (n, depth) = (layer_count(&s), past(&s));
    let e = s.execute(CMD, dab("levels")).unwrap_err().to_string();
    assert!(e.contains(INDEXED_ALERT), "{e}");
    assert_eq!((layer_count(&s), past(&s)), (n, depth), "nothing changes");
}

#[test]
fn bad_params_are_errors_not_panics() {
    let mut s = crate::session();
    assert!(s.execute(CMD, dab("levels")).is_err(), "no document");
    let mut s = doc();
    for p in [
        json!(null),
        json!([]),
        json!("levels"),
        json!({}),
        json!({"kind": "levels"}),
        json!({"kind": "levels", "points": []}),
        json!({"kind": "levels", "points": "x"}),
        json!({"kind": "levels", "points": [[f64::MAX, 1.0]]}),
        json!({"kind": "levels", "points": [[1e30, -1e30]]}),
        json!({"kind": "light", "points": [[10, 10]]}),
        json!({"kind": "grain", "points": [[10, 10]]}),
        json!({"kind": "nope", "points": [[10, 10]]}),
        json!({"kind": 7, "points": [[10, 10]]}),
        json!({"kind": "levels", "subtract": "yes", "points": [[10, 10]]}),
        json!({"kind": "levels", "points": [[10, 10]], "size": -5}),
        json!({"kind": "levels", "points": [[10, 10]], "size": 1e12}),
        json!({"kind": "levels", "points": [[10, 10]], "zoom": -1}),
        json!({"kind": "levels", "points": [[10, 10]], "preset": "no such preset"}),
        json!({"kind": "levels", "points": [[10, 10]], "brush": {"size": "big"}}),
    ] {
        let _ = s.execute(CMD, p.clone());
    }
    for p in [json!({"kind": "light", "points": [[10, 10]]}), json!({"kind": "nope", "points": [[10, 10]]}), json!({"kind": "levels", "subtract": 1})] {
        let n = layer_count(&s);
        assert!(s.execute(CMD, p).is_err());
        assert_eq!(layer_count(&s), n, "a refused stroke leaves no layer behind");
    }
    // Painting off the canvas still makes the layer, with nothing painted.
    let mut s = doc();
    s.execute(CMD, json!({"kind": "levels", "points": [[-5000, -5000]]})).unwrap();
    assert_eq!(mask_at(&s, 0, 0), 0.0);
}
