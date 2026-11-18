//! #14533: Image › Calculations run by a script reads a source layer name that does not exist as
//! the top layer, and `"gray"` as the grey composite of a colour source, as the reference
//! application does. An unknown channel still fails; Apply Image keeps its own rules (#14506).

use photocraft_color::PixelFormat;
use photocraft_doc::LayerMask;

use super::*;

/// The issue's points: (50,50) | (300,50) | (50,200) | (225,150).
const POINTS: [(i32, i32); 4] = [(50, 50), (300, 50), (50, 200), (225, 150)];

/// The issue's "target" document, 400 x 300 at `depth` bits: Background left half 200/100/50,
/// right half 50/150/250; "Top" a 100 x 100 square at 150,100 of 0/255/0 at 50 % alpha; "Masked"
/// (the top layer) solid red with a layer mask showing only the left half; alpha "Alpha 1".
fn session(mode: &str, depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300, "mode": mode, "depth": depth, "background": "white"})).unwrap();
    s.edit("test document", |doc, active| {
        let fmt = doc.pixel_format();
        let px = |rgba: [f32; 4]| from_rgba(&fmt, rgba);
        let bg = doc.background_mut().and_then(Layer::surface_mut).unwrap();
        bg.fill_rect(Rect::new(0, 0, 200, 300), &px([200.0 / 255.0, 100.0 / 255.0, 50.0 / 255.0, 1.0]));
        bg.fill_rect(Rect::new(200, 0, 400, 300), &px([50.0 / 255.0, 150.0 / 255.0, 250.0 / 255.0, 1.0]));
        let mut top = Layer::raster("Top", fmt);
        top.surface_mut().unwrap().fill_rect(Rect::new(150, 100, 250, 200), &px([0.0, 1.0, 0.0, 0.5]));
        doc.insert_above(None, top);
        let mut masked = Layer::raster("Masked", fmt);
        masked.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 400, 300), &px([1.0, 0.0, 0.0, 1.0]));
        let mut mask = LayerMask::reveal_all();
        mask.surface.fill_rect(Rect::new(200, 0, 400, 300), &from_rgba(&PixelFormat::GRAY8, [0.0, 0.0, 0.0, 1.0]));
        masked.mask = Some(mask);
        *active = Some(doc.insert_above(None, masked));
        Ok(())
    })
    .unwrap();
    s.execute("channel.new", json!({"fill": "black", "name": "Alpha 1"})).unwrap();
    s.execute("channel.target", json!({"channel": "composite"})).ok();
    s
}

fn rgb() -> Session {
    session("rgb", 8)
}

/// Calculations into a new channel; its values at [`POINTS`] and on a coarse grid.
fn calculate(s: &mut Session, params: Value) -> Result<Vec<f32>> {
    let r = s.execute("image.calculations", params)?;
    let i = r["channel"].as_u64().unwrap() as usize;
    let surf = &s.active().unwrap().doc.channels[i].surface;
    let grid = (0..300).step_by(25).flat_map(|y| (0..400).step_by(25).map(move |x| (x, y)));
    Ok(POINTS.into_iter().chain(grid).map(|(x, y)| surf.sample_channel(x, y, 0)).collect())
}

fn steps(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

fn channels(s: &Session) -> usize {
    s.active().unwrap().doc.channels.len()
}

#[test]
fn an_unknown_source_1_layer_reads_the_top_layer() {
    let mut s = rgb();
    let source2 = json!({"layer": "merged", "channel": "green"});
    let masked = calculate(&mut s, json!({"source1": {"layer": "Masked", "channel": "red"}, "source2": source2, "blending": "normal"})).unwrap();
    let before = steps(&s);
    let nope = calculate(&mut s, json!({"source1": {"layer": "Nope", "channel": "red"}, "source2": source2, "blending": "normal"})).unwrap();
    assert_eq!(nope, masked);
    assert_eq!(steps(&s), before + 1, "one Calculations step");
    // The top layer, not the merged image or the Background.
    let merged = calculate(&mut s, json!({"source1": {"layer": "merged", "channel": "red"}, "source2": source2, "blending": "normal"})).unwrap();
    assert_ne!(nope, merged);
}

#[test]
fn an_unknown_source_2_layer_reads_the_top_layer() {
    // Source 1 is the Background's Green (100 | 150, never 0), so Multiply shows Source 2.
    let mut s = rgb();
    let source1 = json!({"layer": {"background": true}, "channel": "green"});
    let masked = calculate(&mut s, json!({"source1": source1, "source2": {"layer": "Masked", "channel": "red"}, "blending": "multiply"})).unwrap();
    let nope = calculate(&mut s, json!({"source1": source1, "source2": {"layer": "Nope", "channel": "red"}, "blending": "multiply"})).unwrap();
    assert_eq!(nope, masked);
    // Not the merged image: on the right half Masked's mask shows the Background's red (50).
    let merged = calculate(&mut s, json!({"source1": source1, "source2": {"layer": "merged", "channel": "red"}, "blending": "multiply"})).unwrap();
    assert_ne!(nope, merged);
}

#[test]
fn gray_names_the_grey_composite_of_a_colour_source() {
    let mut s = rgb();
    let composite = calculate(&mut s, json!({"source1": {"channel": "composite"}, "source2": {"channel": "composite"}, "blending": "normal"})).unwrap();
    for name in ["gray", "Gray", "GRAY"] {
        let gray = calculate(&mut s, json!({"source1": {"channel": name}, "source2": {"channel": name}, "blending": "normal"})).unwrap();
        assert_eq!(gray, composite, "{name}");
    }
    // A layer's Gray too.
    let layer = calculate(&mut s, json!({"source1": {"layer": "Top", "channel": "composite"}, "source2": {"channel": "red"}})).unwrap();
    let gray = calculate(&mut s, json!({"source1": {"layer": "Top", "channel": "gray"}, "source2": {"channel": "red"}})).unwrap();
    assert_eq!(gray, layer);
}

#[test]
fn an_alpha_channel_named_gray_is_still_read_by_its_name() {
    let mut s = rgb();
    s.execute("channel.new", json!({"fill": "white", "name": "Gray"})).unwrap();
    let alpha = calculate(&mut s, json!({"source1": {"channel": "Gray"}, "source2": {"channel": "Gray"}, "blending": "normal"})).unwrap();
    assert!(alpha.iter().all(|v| *v == 1.0), "{alpha:?}");
}

#[test]
fn an_unknown_channel_still_fails_and_changes_nothing() {
    let mut s = rgb();
    let (before, chans) = (steps(&s), channels(&s));
    for bad in [json!("Nope"), json!("grey"), json!({"color": "gray"})] {
        let r = s.execute("image.calculations", json!({"source1": {"channel": bad}, "source2": {"channel": "red"}}));
        assert!(r.is_err(), "{bad}");
    }
    assert_eq!((steps(&s), channels(&s)), (before, chans));
}

#[test]
fn apply_image_keeps_its_errors() {
    // Apply Image's unknown layer is its own item (#14506); "gray" is not one of its names.
    let mut s = rgb();
    let before = steps(&s);
    for source in [json!({"layer": "Nope", "channel": "red"}), json!({"channel": "gray"})] {
        let r = s.execute("image.applyImage", json!({"source": source, "blending": "normal"}));
        assert!(r.is_err(), "{source}");
    }
    assert_eq!(steps(&s), before);
}

#[test]
fn documents_that_were_not_measured_keep_the_error() {
    // Other modes and depths, and a document with only a Background.
    for (mode, depth) in [("grayscale", 8), ("rgb", 16), ("rgb", 32), ("cmyk", 8), ("lab", 8)] {
        let mut s = session(mode, depth);
        let r = s.execute("image.calculations", json!({"source1": {"layer": "Nope"}, "source2": {}}));
        assert!(r.is_err(), "{mode} {depth}");
    }
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "background": "white"})).unwrap();
    let r = s.execute("image.calculations", json!({"source1": {"layer": "Nope"}, "source2": {}}));
    assert!(r.is_err(), "only a Background");
    // A transparent document with no Background (CodeRabbit on #15570).
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "background": "transparent"})).unwrap();
    assert!(s.active().unwrap().doc.background().is_none());
    let r = s.execute("image.calculations", json!({"source1": {"layer": "Nope"}, "source2": {}}));
    assert!(r.is_err(), "no Background");
}

#[test]
fn gray_stays_unknown_outside_8_bit_rgb() {
    // Not measured: other colour modes and depths keep the unknown-channel error.
    for (mode, depth) in [("rgb", 16), ("rgb", 32), ("cmyk", 8), ("lab", 8)] {
        let mut s = session(mode, depth);
        let r = s.execute("image.calculations", json!({"source1": {"channel": "gray"}, "source2": {}}));
        assert!(r.is_err(), "{mode} {depth}");
    }
}
