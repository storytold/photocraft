//! Embedded interpretation prerequisite: no new linked creation or picker workflow.
use super::*;
use photocraft_doc::HistoricalXcfSampleOrder as Order;
use photocraft_geom::Size;

fn fixture() -> Vec<u8> {
    crate::jobs::historical_fixture::historical_rgba16()
}
fn canvas() -> Document {
    Document::new("historical interpretation", Size::new(3, 1), ColorMode::Rgb, SampleType::U16)
}
fn embedded(bytes: Vec<u8>, order: Option<Order>) -> SmartObject {
    let mut object = SmartObject::new(SmartSource::Embedded { file_name: "source.xcf".into(), bytes: Arc::new(bytes) }, Affine::translate(1., 0.), None);
    object.historical_xcf_sample_order = order;
    object
}
fn check_pixel(surface: &Surface, little: bool) {
    let words = if little { [0x3412u16, 0x4523, 0x5634, 0xffff] } else { [0x1234u16, 0x2345, 0x3456, 0xffff] };
    let expected = words.map(|v| f32::from(v) / 65535.);
    let pixel = surface.read_region(Rect::new(1, 0, 2, 1));
    for (actual, expected) in pixel.iter().zip(expected) {
        assert!((actual - expected).abs() <= 1. / 65535., "{actual} versus {expected}");
    }
}

#[test]
fn embedded_order_survives_cold_render_stack_and_native_storage_without_changing_legacy_default() {
    let _lock = crate::edit_menu_cmds::PURGE_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let doc = canvas();
    // Same bytes and transform deliberately warm the caches with the opposite interpretation.
    for mode in [None, Some(photocraft_doc::StackMode::Mean)] {
        for order in [Some(Order::LittleEndian), Some(Order::BigEndian), None] {
            let mut object = embedded(fixture(), order);
            object.stack_mode = mode;
            let little = order == Some(Order::LittleEndian);
            check_pixel(&render(&doc, &object).unwrap().unwrap(), little);
            let mut saved = doc.clone();
            saved.layers.push(Layer::new("source", LayerContent::Smart(object)));
            let bytes = photocraft_format::save_to_bytes(&saved, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
            let LayerContent::Smart(object) = &reopened.layers[0].content else { panic!("smart source") };
            assert_eq!(object.historical_xcf_sample_order, order);
            purge_source_cache();
            purge_stack_cache();
            check_pixel(&render(&reopened, object).unwrap().unwrap(), little);
        }
    }
}

#[test]
fn changed_link_is_rejected_before_normal_warning_bounds_and_filter_cache_reuse() {
    let _lock = crate::edit_menu_cmds::PURGE_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.xcf");
    let bytes = fixture();
    std::fs::write(&path, &bytes).unwrap();
    let mut object = embedded(bytes.clone(), Some(Order::BigEndian));
    object.source = SmartSource::Linked { path: path.to_string_lossy().into_owned() };
    object.historical_xcf_link_digest = Some(*blake3::hash(&bytes).as_bytes());
    let doc = canvas();
    check_pixel(&render(&doc, &object).unwrap().unwrap(), false);
    assert!(super::super::rendering::source_box(&doc, &object).is_some());
    for replacement in [
        {
            let mut changed = bytes.clone();
            let last = changed.len() - 8;
            changed[last] ^= 1;
            changed
        },
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1" fill="red"/></svg>"##.to_vec(),
    ] {
        std::fs::write(&path, replacement).unwrap();
        assert!(render(&doc, &object).unwrap_err().to_string().contains("bytes changed"));
        assert!(render_with_warnings(&doc, &object).unwrap_err().to_string().contains("bytes changed"));
        assert!(render_below_filter(&doc, &object, 0).unwrap_err().to_string().contains("bytes changed"));
        assert!(super::super::rendering::source_box(&doc, &object).is_none());
        let mut document = doc.clone();
        let layer = Layer::new("linked", LayerContent::Smart(object.clone()));
        let id = layer.id;
        document.layers.push(layer);
        let before = document.clone();
        assert!(refresh(&mut document, id).is_err());
        assert_eq!(document, before);
        assert!(contents_of(&document, id).is_err());
    }
    std::fs::write(&path, bytes).unwrap();
    object.historical_xcf_link_digest = None;
    assert!(render(&doc, &object).is_err(), "an annotation alone cannot approve linked bytes");
}

#[test]
fn decoded_embedded_contents_use_order_and_native_edit_clears_obsolete_interpretation() {
    let mut session = crate::session();
    let mut doc = canvas();
    let object = embedded(fixture(), Some(Order::LittleEndian));
    let layer = Layer::new("embedded", LayerContent::Smart(object));
    let id = layer.id;
    doc.layers.push(layer);
    session.add_document(doc, None);
    let result = session.execute("layer.smartObjects.editContents", json!({"layer":id.0})).unwrap();
    let child = result["document"].as_u64().unwrap() as usize;
    let pixel = session.documents()[child].doc.layers[0].surface().unwrap().read_region(Rect::new(0, 0, 1, 1));
    assert!((pixel[0] - f32::from(0x3412u16) / 65535.).abs() <= 1. / 65535.);
    assert!(commit_child(&mut session, child).unwrap());
    let LayerContent::Smart(object) = &session.documents()[0].doc.layer(id).unwrap().content else { panic!("smart source") };
    assert_eq!(object.historical_xcf_sample_order, None);
    assert_eq!(object.historical_xcf_link_digest, None);
}
