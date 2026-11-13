//! Historical source interpretation persists independently of rendered smart-object cache pixels.
use std::sync::Arc;

use photocraft_doc::{ColorMode, Document, HistoricalXcfSampleOrder, Layer, LayerContent, SampleType, Size, SmartObject, SmartSource};
use photocraft_format::{load_from_bytes, save_to_bytes, zip};
use photocraft_geom::Affine;
use photocraft_raster::Surface;

fn sample(order: Option<HistoricalXcfSampleOrder>) -> Document {
    let mut document = Document::new("historical smart sources", Size::new(1, 1), ColorMode::Rgb, SampleType::F32);
    for source in [
        SmartSource::Embedded { file_name: "source.xcf".into(), bytes: Arc::new(vec![0x12, 0x34, 0xab, 0xcd]) },
        SmartSource::Linked { path: "unread-source.xcf".into() },
    ] {
        let mut cache = Surface::new(document.pixel_format());
        cache.write_region(document.bounds(), &[-0.25, 0.5, 2., 1.]);
        let mut smart = SmartObject::new(source, Affine::IDENTITY, Some(cache));
        assert_eq!(smart.historical_xcf_sample_order, None, "new objects have no inferred sample order");
        smart.historical_xcf_sample_order = order;
        document.layers.push(Layer::new("Smart", LayerContent::Smart(smart)));
    }
    document
}

#[test]
fn historical_order_wire_values_are_explicit_stable_and_shared_with_display() {
    use HistoricalXcfSampleOrder::{BigEndian, LittleEndian};
    let mut orders = std::collections::HashSet::new();
    for (order, wire) in [(BigEndian, "big-endian"), (LittleEndian, "little-endian")] {
        orders.insert(order);
        assert_eq!(order.to_string(), wire);
        assert_eq!(serde_json::to_value(order).unwrap(), wire);
        assert_eq!(serde_json::from_value::<HistoricalXcfSampleOrder>(wire.into()).unwrap(), order);
    }
    assert_eq!(orders.len(), 2);
    for invalid in ["native", "BigEndian", "little_endian", ""] {
        assert!(serde_json::from_value::<HistoricalXcfSampleOrder>(invalid.into()).is_err());
    }
}

#[test]
fn native_smart_sources_roundtrip_both_explicit_orders_and_unset_without_touching_samples() {
    for order in [None, Some(HistoricalXcfSampleOrder::BigEndian), Some(HistoricalXcfSampleOrder::LittleEndian)] {
        let document = sample(order);
        let bytes = save_to_bytes(&document, &Default::default()).unwrap();
        let reopened = load_from_bytes(&bytes).unwrap();
        assert_eq!(reopened, document);
        let archive = zip::ZipReader::new(&bytes).unwrap();
        let manifest: serde_json::Value = serde_json::from_slice(&archive.read_by_name("manifest.json", 1 << 20).unwrap()).unwrap();
        for layer in manifest["document"]["layers"].as_array().unwrap() {
            assert_eq!(layer["content"]["historical_xcf_sample_order"], serde_json::to_value(order).unwrap());
            if order.is_none() {
                assert!(layer["content"].get("historical_xcf_sample_order").is_none(), "unset is omitted for older readers");
            }
        }
    }
}

#[test]
fn older_native_smart_objects_without_order_remain_unset_instead_of_guessing() {
    let document = sample(Some(HistoricalXcfSampleOrder::LittleEndian));
    let bytes = save_to_bytes(&document, &Default::default()).unwrap();
    let archive = zip::ZipReader::new(&bytes).unwrap();
    let mut manifest: serde_json::Value = serde_json::from_slice(&archive.read_by_name("manifest.json", 1 << 20).unwrap()).unwrap();
    for layer in manifest["document"]["layers"].as_array_mut().unwrap() {
        layer["content"].as_object_mut().unwrap().remove("historical_xcf_sample_order");
    }
    let mut older = zip::ZipWriter::new();
    older.add("manifest.json", &serde_json::to_vec(&manifest).unwrap()).unwrap();
    for entry in archive.entries.iter().filter(|entry| entry.name != "manifest.json") {
        older.add(&entry.name, &archive.read(entry, usize::MAX).unwrap()).unwrap();
    }
    let mut expected = document;
    for layer in &mut expected.layers {
        let LayerContent::Smart(smart) = &mut layer.content else { panic!("smart source") };
        smart.historical_xcf_sample_order = None;
    }
    assert_eq!(load_from_bytes(&older.finish().unwrap()).unwrap(), expected);
}
