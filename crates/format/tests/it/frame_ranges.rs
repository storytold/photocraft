use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, FrameSpan, Layer, Size, Timeline};

/// A paint layer's frame range, its eye kept while the playhead hides it, and a range inside a
/// group come back from a `.pcraft`; a layer without one stays on every frame.
#[test]
fn frame_ranges_pcraft_roundtrip() {
    let mut doc = Document::new("Shot", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    doc.timeline = Some(Timeline::new(24, 24.0));
    let mut hidden_by_playhead = Layer::raster("Frame 12", doc.pixel_format());
    hidden_by_playhead.span = Some(FrameSpan::new(11, 12));
    hidden_by_playhead.visible = false;
    let mut nested = Layer::raster("Wire", doc.pixel_format());
    nested.span = Some(FrameSpan { start: 3, end: 9, eye: false });
    nested.visible = false;
    doc.layers.push(Layer::raster("Plate", doc.pixel_format()));
    doc.layers.push(hidden_by_playhead);
    doc.layers.push(Layer::group("Paint", vec![nested]));
    let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
    let back = photocraft_format::load_from_bytes(&bytes).unwrap();
    assert_eq!(back, doc);
    assert!(back.layers[0].span.is_none(), "the plate stays on every frame");
}
