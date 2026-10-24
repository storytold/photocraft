//! Hue Curves (#6241): Hue vs Hue / Saturation / Luminance curves as an adjustment layer and as
//! Image › Adjustments, with undo, `.pcraft` round trip and the PSD fallback.

use photocraft_doc::{Adjustment, LayerContent, Rect};
use photocraft_engine::Session;
use serde_json::{Value, json};

const RED: [f32; 3] = [0.9, 0.1, 0.1];
const GREEN: [f32; 3] = [0.15, 0.8, 0.2];
const BLUE: [f32; 3] = [0.1, 0.2, 0.9];

/// A 3×1 RGB document with a red, a green and a blue pixel.
fn session(depth: u64) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 3, "height": 1, "depth": depth})).unwrap();
    s.edit("setup", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let data: Vec<_> = [RED, GREEN, BLUE].iter().flat_map(|c| photocraft_raster::from_rgba(&fmt, [c[0], c[1], c[2], 1.0])).collect();
        surf.write_region(Rect::from_xywh(0, 0, 3, 1), &data);
        Ok(())
    })
    .unwrap();
    s
}

fn composite(s: &Session) -> Vec<[f32; 3]> {
    let d = &s.active().unwrap().doc;
    photocraft_compose::render(d, Rect::from_xywh(0, 0, 3, 1)).px.iter().map(|p| [p[0], p[1], p[2]]).collect()
}

fn pixels(s: &Session) -> Vec<[f32; 3]> {
    let d = s.active().unwrap();
    let surf = d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap();
    (0..3).map(|x| surf.rgba(x, 0)).map(|p| [p[0], p[1], p[2]]).collect()
}

fn hue(c: [f32; 3]) -> f32 {
    photocraft_compose::adjust::rgb_to_hsl(c).0 * 360.0
}

fn sat(c: [f32; 3]) -> f32 {
    photocraft_compose::adjust::rgb_to_hsl(c).1
}

fn close(a: [f32; 3], b: [f32; 3], tol: f32) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() <= tol)
}

/// Reds to orange, greens at half saturation; every other primary anchored.
fn params() -> Value {
    json!({
        "hue": [[0, 30], [60, 0], [120, 0], [180, 0], [240, 0], [300, 0]],
        "saturation": [[0, 1], [60, 1], [120, 0.5], [180, 1], [240, 1], [300, 1]],
    })
}

#[test]
fn adjustment_layer_turns_red_orange_and_greens_dull_and_leaves_blue() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        let before = composite(&s);
        let steps = s.active().unwrap().history.past_len();
        s.execute("layer.newAdjustmentLayer.hueCurves", params()).unwrap();
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "one history step");
        let d = s.active().unwrap();
        let layer = d.doc.layer(d.active_layer.unwrap()).unwrap();
        assert_eq!(layer.name, "Hue Curves 1");
        assert!(matches!(&layer.content, LayerContent::Adjustment(Adjustment::HueCurves { .. })));
        let after = composite(&s);
        let tol = if depth == 8 { 1.5 } else { 0.5 };
        assert!((hue(after[0]) - (hue(before[0]) + 30.0)).abs() < tol, "{depth}: red to orange: {:?}", after[0]);
        assert!((sat(after[1]) - sat(before[1]) * 0.5).abs() < 0.02, "{depth}: green dulled: {:?}", after[1]);
        // Any adjustment layer requantizes a 16-bit composite by up to a level (a neutral Curves
        // layer too), so "untouched" means within that.
        assert!(close(after[2], before[2], 3e-5), "{depth}: blue untouched: {:?} vs {:?}", after[2], before[2]);

        // Undo takes the layer away again.
        s.execute("edit.undo", json!({})).unwrap();
        let d = s.active().unwrap();
        assert!(!d.doc.layers.iter().any(|l| matches!(l.content, LayerContent::Adjustment(_))));
        assert_eq!(composite(&s), before);
    }
}

#[test]
fn neutral_hue_curves_change_nothing() {
    for depth in [8, 32] {
        let mut s = session(depth);
        let before = composite(&s);
        s.execute("layer.newAdjustmentLayer.hueCurves", json!({})).unwrap();
        assert_eq!(composite(&s), before);
        s.execute("layer.setAdjustment", json!({"hue": [[0, 0], [120, 0], [240, 0]], "saturation": [[90, 1]]})).unwrap();
        assert_eq!(composite(&s), before);
    }
    // Destructive on 16-bit pixels: the pixels stay exactly as they were.
    let mut s = session(16);
    let before = pixels(&s);
    s.execute("image.adjustments.hueCurves", json!({"hue": [[0, 0], [180, 0]]})).unwrap();
    assert_eq!(pixels(&s), before);
}

#[test]
fn set_adjustment_edits_one_curve_and_undoes() {
    let mut s = session(8);
    s.execute("layer.newAdjustmentLayer.hueCurves", params()).unwrap();
    let first = composite(&s);
    s.execute("layer.setAdjustment", json!({"luminance": [[240, -60], [0, 0], [120, 0]]})).unwrap();
    let edited = composite(&s);
    assert!(edited[2].iter().sum::<f32>() < first[2].iter().sum::<f32>() - 0.1, "blue darker: {:?}", edited[2]);
    assert!(close(edited[0], first[0], 1e-6), "the hue curve stays");
    let d = s.active().unwrap();
    let LayerContent::Adjustment(Adjustment::HueCurves { hue, saturation, luminance }) = &d.doc.layer(d.active_layer.unwrap()).unwrap().content else {
        panic!()
    };
    assert_eq!((hue.len(), saturation.len(), luminance.len()), (6, 6, 3));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(composite(&s), first);
}

#[test]
fn destructive_hue_curves_on_pixels() {
    let mut s = session(16);
    let before = pixels(&s);
    s.execute("image.adjustments.hueCurves", params()).unwrap();
    let after = pixels(&s);
    assert!((hue(after[0]) - (hue(before[0]) + 30.0)).abs() < 0.5, "{:?}", after[0]);
    assert!(close(after[2], before[2], 1e-4));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(pixels(&s), before);
}

#[test]
fn hue_curves_survive_pcraft_save_and_reopen() {
    let mut s = session(8);
    s.execute("layer.newAdjustmentLayer.hueCurves", json!({"hue": [[350, 25], [10, 25], [180, -40]], "luminance": [[200, 30]]})).unwrap();
    let doc = &s.active().unwrap().doc;
    let bytes = photocraft_format::save_to_bytes(doc, &photocraft_format::SaveOptions::default()).unwrap();
    let back = photocraft_format::load_from_bytes(&bytes).unwrap();
    let curves = |d: &photocraft_doc::Document| d.layers.iter().find_map(|l| matches!(l.content, LayerContent::Adjustment(_)).then(|| l.content.clone()));
    assert_eq!(curves(&back), curves(doc));
    let rect = Rect::from_xywh(0, 0, 3, 1);
    assert_eq!(photocraft_compose::render(&back, rect).px, photocraft_compose::render(doc, rect).px);
}

#[test]
fn psd_save_keeps_the_look_as_a_color_lookup() {
    let mut s = session(8);
    s.execute("layer.newAdjustmentLayer.hueCurves", params()).unwrap();
    let want = composite(&s);
    let doc = &s.active().unwrap().doc;
    let out = photocraft_io::export(doc, "psd", &Default::default()).unwrap();
    assert!(out.warnings.iter().any(|w| w.contains("Hue Curves") && w.contains("Color Lookup")), "{:?}", out.warnings);
    let back = photocraft_io::import("x.psd", &out.bytes).unwrap().document;
    assert!(back.layers.iter().any(|l| matches!(&l.content, LayerContent::Adjustment(Adjustment::ColorLookup { name, .. }) if name == "Hue Curves")));
    let got: Vec<[f32; 3]> = photocraft_compose::render(&back, Rect::from_xywh(0, 0, 3, 1)).px.iter().map(|p| [p[0], p[1], p[2]]).collect();
    for (w, g) in want.iter().zip(&got) {
        assert!(close(*w, *g, 0.03), "{w:?} vs {g:?}");
    }
}

#[test]
fn bad_hue_curves_params_are_errors_not_panics() {
    let mut s = session(8);
    let before = pixels(&s);
    let layers = s.active().unwrap().doc.layers.len();
    let steps = s.active().unwrap().history.past_len();
    for p in [json!({"hue": "red"}), json!({"saturation": [[0]]}), json!({"luminance": [[0, null, 3]]}), json!([1, 2])] {
        assert!(s.execute("layer.newAdjustmentLayer.hueCurves", p.clone()).is_err(), "{p}");
        assert!(s.execute("image.adjustments.hueCurves", p.clone()).is_err(), "{p}");
        let d = s.active().unwrap();
        assert_eq!(d.doc.layers.len(), layers, "{p}: no layer added");
        assert_eq!(d.history.past_len(), steps, "{p}: no history step");
        assert_eq!(pixels(&s), before, "{p}: pixels untouched");
    }
}
