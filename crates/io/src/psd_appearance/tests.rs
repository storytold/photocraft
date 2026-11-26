use super::*;
use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Fill, Layer, LayerContent, Path, ShapeLayer, Subpath, TextLayer};
use photocraft_geom::{Rect, Size};
use photocraft_raster::Surface;
use std::sync::Arc;

fn saved_pixels() -> Surface {
    let mut pixels = Surface::new(PixelFormat::RGBA8);
    pixels.fill_rect(Rect::new(0, 0, 8, 6), &[17. / 255., 31. / 255., 57. / 255., 1.]);
    pixels
}

fn roundtrip(document: &Document) -> Document {
    let saved = crate::export(document, "appearance.psd", &Default::default()).unwrap();
    crate::import("appearance.psd", &saved.bytes).unwrap().document
}

#[test]
fn tagged_shape_keeps_its_saved_cache_instead_of_becoming_a_soft_fill() {
    let mut document = Document::new("saved shape", Size::new(8, 6), ColorMode::Rgb, SampleType::U8);
    let pixels = saved_pixels();
    let shape = ShapeLayer {
        path: Path { subpaths: vec![Subpath::polygon(&[(1., 1.), (6., 1.), (3., 5.)])], ..Default::default() },
        fill: Some(Fill::Solid(Color::WHITE)),
        density: 0.4,
        feather: 1.0,
        cache: Some(pixels.clone()),
        cache_preserves_appearance: true,
        ..Default::default()
    };
    document.layers.push(Layer::new("Source shape", LayerContent::Shape(shape)));
    let reopened = roundtrip(&document);
    let layer = reopened.layers.iter().find(|layer| layer.name == "Source shape").unwrap();
    let LayerContent::Shape(shape) = &layer.content else { panic!("shape cache was lost to {}", layer.content.kind_name()) };
    assert!(shape.cache_preserves_appearance);
    assert!(!shape.path.subpaths.is_empty());
    assert_eq!(shape.cache.as_ref().unwrap().read_region(document.bounds()), pixels.read_region(document.bounds()));
}

#[test]
fn future_policy_payloads_and_unknown_layer_blocks_survive_known_tag_regeneration() {
    let mut document = Document::new("saved type", Size::new(8, 6), ColorMode::Rgb, SampleType::U8);
    let text = TextLayer {
        text: "Title".into(),
        font_family: "Inter".into(),
        cache: Some(saved_pixels()),
        cache_uses_raster_blending: true,
        keep_layer_name: true,
        ..Default::default()
    };
    let unknown =
        vec![(PSD_KEY, Arc::new(vec![0, 0, 0, 9, 0, 0, 0, 1])), (FIXED_TEXT_NAME_KEY, Arc::new(vec![0, 0, 0, 9])), (*b"utst", Arc::new(vec![17, 31, 57, 83]))];
    let mut layer = Layer::new("Title", LayerContent::Text(text));
    layer.psd_blocks = unknown.clone();
    document.layers.push(layer);
    for retain_policy in [true, false] {
        document = roundtrip(&document);
        let layer = document.layers.iter_mut().find(|layer| layer.name == "Title").unwrap();
        let LayerContent::Text(text) = &mut layer.content else { panic!("editable type") };
        assert_eq!(text.keep_layer_name, retain_policy);
        assert_eq!(text.cache_uses_raster_blending, retain_policy);
        for (key, payload) in &unknown {
            assert!(layer.psd_blocks.iter().any(|(actual_key, actual)| actual_key == key && actual == payload));
        }
        assert!(!layer.psd_blocks.iter().any(|(key, payload)| known_tag(key, payload)));
        text.keep_layer_name = false;
        text.cache_uses_raster_blending = false;
    }
}
