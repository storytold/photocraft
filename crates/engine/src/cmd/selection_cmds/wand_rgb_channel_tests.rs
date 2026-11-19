use photocraft_doc::{ColorMode, Document, Layer, SampleType, Size};

use super::*;

fn swatches(depth: SampleType) -> Session {
    let mut doc = Document::new("Channel swatches", Size::new(4, 1), ColorMode::Rgb, depth);
    let mut layer = Layer::raster("Swatches", doc.pixel_format());
    let surface = layer.surface_mut().unwrap();
    for (x, pixel) in [[0.25, 0.75, 0.50, 1.0], [0.75, 0.25, 0.50, 1.0], [0.25, 0.75, 0.75, 1.0], [0.25, 0.75, 0.50, 0.0]].iter().enumerate() {
        surface.write_pixel(x as i32, 0, pixel);
    }
    doc.layers.push(layer);
    let mut s = Session::new();
    s.add_document(doc, None);
    s
}

fn magic_wand_at_first_pixel(s: &mut Session, sample_all_layers: bool) {
    s.execute("select.magicWand", json!({"x": 0, "y": 0, "tolerance": 0, "antiAlias": false, "sampleAllLayers": sample_all_layers})).unwrap();
}

fn coverage(s: &Session, x: i32) -> f32 {
    s.active().unwrap().doc.selection.as_ref().map_or(0.0, |mask| mask.sample_channel(x, 0, 0))
}

#[test]
fn magic_wand_matches_the_selected_blue_component_at_every_depth() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut s = swatches(depth);
        magic_wand_at_first_pixel(&mut s, false);
        assert_eq!(coverage(&s, 1), 0.0, "{depth:?}: composite compares all three components");
        for channel in ["red", "green"] {
            s.execute("channel.setVisible", json!({"channel": channel, "visible": false})).unwrap();
        }
        magic_wand_at_first_pixel(&mut s, false);
        assert_eq!(coverage(&s, 1), 0.0, "{depth:?}: hiding eyes does not select a color target");
        s.execute("channel.target", json!({"channel": "blue"})).unwrap();
        magic_wand_at_first_pixel(&mut s, false);
        assert_eq!(coverage(&s, 1), 1.0, "{depth:?}: blue matches despite unequal red and green");
        assert_eq!(coverage(&s, 2), 0.0, "{depth:?}: unequal blue breaks the region");
    }
}

#[test]
fn magic_wand_samples_the_selected_component_from_all_layers_when_requested() {
    let mut s = swatches(SampleType::U8);
    s.edit("Cover the third swatch", |doc, _| {
        let mut cover = Layer::raster("Cover", doc.pixel_format());
        cover.surface_mut().unwrap().write_pixel(2, 0, &[0.0, 0.5, 0.5, 1.0]);
        doc.layers.push(cover);
        Ok(())
    })
    .unwrap();

    s.execute("channel.target", json!({"channel": "blue"})).unwrap();
    magic_wand_at_first_pixel(&mut s, false);
    assert_eq!(coverage(&s, 2), 0.0, "active layer's third swatch has a different blue value");
    magic_wand_at_first_pixel(&mut s, true);
    assert_eq!(coverage(&s, 2), 1.0, "visible cover has the same blue value as the seed");
}

#[test]
fn magic_wand_still_compares_alpha_for_the_selected_component() {
    let mut s = swatches(SampleType::U8);
    s.edit("Match the third swatch's blue", |doc, _| {
        doc.layers[0].surface_mut().unwrap().write_pixel(2, 0, &[0.25, 0.75, 0.5, 1.0]);
        Ok(())
    })
    .unwrap();
    s.execute("channel.target", json!({"channel": "blue"})).unwrap();
    magic_wand_at_first_pixel(&mut s, false);
    assert_eq!(coverage(&s, 2), 1.0, "all three opaque swatches share a blue value");
    assert_eq!(coverage(&s, 3), 0.0, "the transparent neighbor has the same blue but different alpha");
}
