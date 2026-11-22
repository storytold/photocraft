//! #14511: when Image › Calculations reads a single layer as Source 1, the layer's transparent
//! pixels and the pixels its layer mask hides let Source 2 through instead of counting as black,
//! as the reference application does. Merged sources, the Transparency and Layer Mask channels,
//! alpha channels, Source 2 and Apply Image keep their results.

use photocraft_color::PixelFormat;
use photocraft_doc::LayerMask;

use super::*;

/// The issue's points: (50,50) | (300,50) | (50,200) | (225,150).
const POINTS: [(i32, i32); 4] = [(50, 50), (300, 50), (50, 200), (225, 150)];

/// Source 2 in every case: the target's Background, Green (100 | 150 | 100 | 150).
fn source2() -> Value {
    json!({"layer": {"background": true}, "channel": "green"})
}

fn rgba(fmt: &PixelFormat, r: f32, g: f32, b: f32, a: f32) -> Vec<f32> {
    from_rgba(fmt, [r / 255.0, g / 255.0, b / 255.0, a / 255.0])
}

/// The issue's "target" document, 400 x 300 RGB 8 bit: Background left half 200/100/50, right
/// half 50/150/250; "Top" transparent but for a 100 x 100 square of 0/255/0 at alpha 128 at
/// 150,100; "Masked" (the top layer) solid red with a layer mask showing only the left half;
/// "Alpha 1" black.
fn target(s: &mut Session) {
    s.execute("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
    s.edit("target", |doc, active| {
        let fmt = doc.pixel_format();
        let bg = doc.background_mut().and_then(Layer::surface_mut).unwrap();
        bg.fill_rect(Rect::new(0, 0, 200, 300), &rgba(&fmt, 200.0, 100.0, 50.0, 255.0));
        bg.fill_rect(Rect::new(200, 0, 400, 300), &rgba(&fmt, 50.0, 150.0, 250.0, 255.0));
        let mut top = Layer::raster("Top", fmt);
        top.surface_mut().unwrap().fill_rect(Rect::new(150, 100, 250, 200), &rgba(&fmt, 0.0, 255.0, 0.0, 128.0));
        doc.insert_above(None, top);
        let mut masked = Layer::raster("Masked", fmt);
        masked.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 400, 300), &rgba(&fmt, 255.0, 0.0, 0.0, 255.0));
        let mut mask = LayerMask::reveal_all();
        mask.surface.fill_rect(Rect::new(200, 0, 400, 300), &from_rgba(&PixelFormat::GRAY8, [0.0, 0.0, 0.0, 1.0]));
        masked.mask = Some(mask);
        *active = Some(doc.insert_above(None, masked));
        Ok(())
    })
    .unwrap();
    s.execute("channel.new", json!({"fill": "black", "name": "Alpha 1"})).unwrap();
    s.execute("channel.target", json!({"channel": "composite"})).unwrap();
}

/// The issue's "source" document, 400 x 300 RGB 8 bit: Background 128/128/128 and a layer "S1",
/// white on the right half and transparent on the left.
fn source_document(s: &mut Session) {
    s.execute("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
    s.edit("source", |doc, active| {
        let fmt = doc.pixel_format();
        let bg = doc.background_mut().and_then(Layer::surface_mut).unwrap();
        bg.fill_rect(Rect::new(0, 0, 400, 300), &rgba(&fmt, 128.0, 128.0, 128.0, 255.0));
        let mut s1 = Layer::raster("S1", fmt);
        s1.surface_mut().unwrap().fill_rect(Rect::new(200, 0, 400, 300), &rgba(&fmt, 255.0, 255.0, 255.0, 255.0));
        *active = Some(doc.insert_above(None, s1));
        Ok(())
    })
    .unwrap();
}

fn session() -> Session {
    let mut s = Session::new();
    target(&mut s);
    s
}

/// Calculations into a new channel; its values at [`POINTS`] on the 0..255 scale.
fn calculate(s: &mut Session, source1: Value, extra: Value) -> [u8; 4] {
    let mut params = json!({"source1": source1, "source2": source2(), "blending": "normal"});
    if let (Some(p), Some(e)) = (params.as_object_mut(), extra.as_object()) {
        p.extend(e.clone());
    }
    let r = s.execute("image.calculations", params).unwrap();
    let i = r["channel"].as_u64().unwrap() as usize;
    let surf = &s.active().unwrap().doc.channels[i].surface;
    POINTS.map(|(x, y)| (surf.sample_channel(x, y, 0) * 255.0).round() as u8)
}

#[test]
fn source_2_is_the_backgrounds_green() {
    let mut s = session();
    assert_eq!(calculate(&mut s, source2(), json!({})), [100, 150, 100, 150]);
}

#[test]
fn a_transparent_layer_lets_source_2_through() {
    let mut s = session();
    // Inside the alpha-128 square the layer's own Red (0) counts, not a mix with Source 2.
    assert_eq!(calculate(&mut s, json!({"layer": "Top", "channel": "red"}), json!({})), [100, 150, 100, 0]);
    assert_eq!(calculate(&mut s, json!({"layer": "Top", "channel": "red"}), json!({"opacity": 50})), [100, 150, 100, 75]);
}

#[test]
fn pixels_the_layer_mask_hides_let_source_2_through() {
    let mut s = session();
    assert_eq!(calculate(&mut s, json!({"layer": "Masked", "channel": "red"}), json!({})), [255, 150, 255, 150]);
}

#[test]
fn a_layer_of_another_document_lets_source_2_through_too() {
    // The "source" document opens first (document 0); "target" is active.
    let mut s = Session::new();
    source_document(&mut s);
    target(&mut s);
    assert_eq!(s.active_index(), Some(1));
    assert_eq!(calculate(&mut s, json!({"document": 0, "layer": "S1", "channel": "red"}), json!({})), [100, 255, 100, 255]);
}

#[test]
fn a_layers_grey_composite_lets_source_2_through() {
    let mut s = session();
    let [a, b, c, _] = calculate(&mut s, json!({"layer": "Top", "channel": "composite"}), json!({}));
    assert_eq!([a, b, c], [100, 150, 100]);
}

#[test]
fn merged_transparency_mask_and_alpha_sources_keep_their_values() {
    let mut s = session();
    // Transparency and Layer Mask read 0 where the layer is transparent or hidden: they are the
    // values themselves, not a missing pixel.
    assert_eq!(calculate(&mut s, json!({"layer": "Top", "channel": "transparency"}), json!({})), [0, 0, 0, 128]);
    assert_eq!(calculate(&mut s, json!({"layer": "Masked", "channel": "mask"}), json!({})), [255, 0, 255, 0]);
    assert_eq!(calculate(&mut s, json!({"channel": "Alpha 1"}), json!({})), [0, 0, 0, 0]);
    // The merged image: Masked's red covers the left half, the Background's red shows on the
    // right, and the half-transparent green square sits over the right Background at 225,150.
    let merged = calculate(&mut s, json!({"layer": "merged", "channel": "red"}), json!({}));
    let red_at = |s: &mut Session, x: i32, y: i32| {
        let px = s.execute("document.pixel", json!({"x": x, "y": y})).unwrap();
        (px[0].as_f64().unwrap() * 255.0).round() as u8
    };
    let want = POINTS.map(|(x, y)| red_at(&mut s, x, y));
    assert_eq!(merged, want);
}

#[test]
fn a_disabled_layer_mask_keeps_todays_result() {
    // Not measured: the mask is not read, as today.
    let mut s = session();
    s.edit("disable mask", |doc, active| {
        let id = active.unwrap();
        doc.layer_mut(id).unwrap().mask.as_mut().unwrap().enabled = false;
        Ok(())
    })
    .unwrap();
    assert_eq!(calculate(&mut s, json!({"layer": "Masked", "channel": "red"}), json!({})), [255, 255, 255, 255]);
}

#[test]
fn a_feathered_layer_mask_keeps_todays_result() {
    // Not measured: a feathered mask's edge is partly visible where its raw value is 0, so the
    // mask is not read, as today (CodeRabbit on #15572).
    let mut s = session();
    s.edit("feather mask", |doc, active| {
        let id = active.unwrap();
        doc.layer_mut(id).unwrap().mask.as_mut().unwrap().feather = 10.0;
        Ok(())
    })
    .unwrap();
    assert_eq!(calculate(&mut s, json!({"layer": "Masked", "channel": "red"}), json!({})), [255, 255, 255, 255]);
}

#[test]
fn a_layer_as_source_2_keeps_todays_result() {
    // Not measured: Source 2's transparent pixels still read as black.
    let mut s = session();
    let r = s
        .execute("image.calculations", json!({"source1": source2(), "source2": {"layer": "Top", "channel": "red"}, "blending": "normal", "opacity": 50}))
        .unwrap();
    let i = r["channel"].as_u64().unwrap() as usize;
    let v = s.active().unwrap().doc.channels[i].surface.sample_channel(50, 50, 0) * 255.0;
    assert_eq!(v.round(), 50.0, "half of Source 1's 100 over a black Source 2");
}
