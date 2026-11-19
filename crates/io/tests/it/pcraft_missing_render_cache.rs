//! Report native smart objects that have lost their saved display pixels (#15799).

use std::sync::Arc;

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, SmartObject, SmartSource};
use photocraft_geom::{Affine, Size};
use photocraft_raster::Surface;

#[test]
fn opening_a_native_document_warns_about_each_smart_object_without_saved_pixels() {
    let mut document = Document::new("missing-cache.pcraft", Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
    let embedded = SmartSource::Embedded { file_name: "source.png".into(), bytes: Arc::new(vec![1, 2, 3]) };
    let linked = SmartSource::Linked { path: "missing-source.png".into() };
    let mut pixels = Surface::new(document.pixel_format());
    pixels.write_pixel(0, 0, &[0.0, 0.0, 0.0, 1.0]);
    let smart = |name: &str, source: SmartSource, cache| Layer::new(name, LayerContent::Smart(SmartObject::new(source, Affine::IDENTITY, cache)));
    document.layers.extend([
        smart("Cached original", embedded.clone(), Some(pixels)),
        smart("Uncached embedded", embedded, None),
        Layer::group("Placed art", vec![smart("Uncached link", linked, None)]),
    ]);

    let bytes = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
    let opened = photocraft_io::import("missing-cache.pcraft", &bytes).unwrap();

    assert_eq!(opened.document, document, "warning must leave the editable source and the stored pixels intact");
    assert_eq!(opened.warnings.len(), 2, "{:?}", opened.warnings);
    for layer in ["Uncached embedded", "Uncached link"] {
        assert!(opened.warnings.iter().any(|warning| warning.contains(layer) && warning.contains("no saved render cache")), "{:?}", opened.warnings);
    }
}
