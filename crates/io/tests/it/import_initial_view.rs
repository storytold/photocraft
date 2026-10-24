use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{AlphaChannel, Document, Layer, LayerId, LayerMask};
use photocraft_geom::Size;
use photocraft_io::{NativeImportTarget, NativeInitialView};

#[test]
fn ordinary_native_import_has_no_initial_view_override() {
    let doc = Document::new("Native", Size::new(8, 6), ColorMode::Rgb, SampleType::U8);
    let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
    let imported = photocraft_io::import("native.pcraft", &bytes).unwrap();
    assert_eq!(imported.initial_view, None);
    assert_eq!(imported.document, doc);
    assert!(NativeInitialView::default().validate(&doc).is_ok());
}

#[test]
fn initial_view_validates_native_ids_masks_channels_and_sampler_capacity() {
    let mut doc = Document::new("Initial view", Size::new(8, 6), ColorMode::Rgb, SampleType::U16);
    let mut layer = Layer::raster("Layer", doc.pixel_format());
    layer.mask = Some(LayerMask::reveal_all());
    let id = layer.id;
    doc.layers.push(layer);
    doc.channels.push(AlphaChannel::new("Alpha", photocraft_raster::Surface::new(photocraft_color::PixelFormat::GRAY8)));
    let valid = NativeInitialView {
        selected_layers: vec![id],
        active_layer: Some(id),
        target: NativeImportTarget::LayerMask,
        samplers: vec![[0.0, 0.0], [7.9, 5.9]],
        ..Default::default()
    };
    assert!(valid.validate(&doc).is_ok());
    assert!(NativeInitialView { target: NativeImportTarget::AlphaChannel(0), ..valid.clone() }.validate(&doc).is_ok());
    for bad in [
        NativeInitialView { active_layer: Some(LayerId(u64::MAX)), ..valid.clone() },
        NativeInitialView { selected_layers: vec![id, id], ..valid.clone() },
        NativeInitialView { target: NativeImportTarget::AlphaChannel(usize::MAX), ..valid.clone() },
        NativeInitialView { selected_path: Some(usize::MAX), ..valid.clone() },
        NativeInitialView { samplers: vec![[1.0, 1.0]; 11], ..valid.clone() },
        NativeInitialView { samplers: vec![[f64::NAN, 0.0]], ..valid.clone() },
        NativeInitialView { samplers: vec![[8.0, 0.0]], ..valid.clone() },
    ] {
        assert!(bad.validate(&doc).is_err());
    }
    doc.layers[0].mask = None;
    assert!(valid.validate(&doc).is_err());
}

#[test]
fn initial_view_rejects_ambiguous_ids_and_excessive_hierarchy() {
    let mut doc = Document::new("Invalid hierarchy", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    let layer = Layer::raster("Repeated", doc.pixel_format());
    doc.layers = vec![layer.clone(), layer];
    assert!(NativeInitialView::default().validate(&doc).is_err());
    doc.layers.truncate(1);
    for _ in 0..=photocraft_doc::MAX_GROUP_DEPTH {
        doc.layers = vec![Layer::group("Nested", std::mem::take(&mut doc.layers))];
    }
    assert!(NativeInitialView::default().validate(&doc).is_err());
}
