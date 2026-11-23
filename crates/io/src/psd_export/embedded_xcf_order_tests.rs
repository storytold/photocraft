//! PSD persistence prerequisite for embedded historical source interpretation only.
use super::*;
use photocraft_doc::{HistoricalXcfSampleOrder as Order, SmartObject, SmartSource};
use photocraft_geom::{Affine, Rect, Size};
use std::sync::Arc;

/// Original one-pixel v7 RGB16 wire data with a non-symmetric multibyte sample.
fn xcf(little: bool) -> Vec<u8> {
    let mut bytes = b"gimp xcf v007\0".to_vec();
    let word = |bytes: &mut Vec<u8>, value: u32| bytes.extend_from_slice(&value.to_be_bytes());
    for value in [1, 1, 0, 250, 17, 1] {
        word(&mut bytes, value);
    }
    bytes.push(0);
    for value in [0, 0, 59, 0, 0, 1, 1, 1, 2] {
        word(&mut bytes, value);
    }
    bytes.extend_from_slice(b"X\0");
    for value in [0, 0, 93, 0, 1, 1, 8, 113, 0, 1, 1, 129, 0] {
        word(&mut bytes, value);
    }
    assert_eq!(bytes.len(), 129);
    for sample in [0x1234u16, 0x5678, 0x9abc, 0xffff] {
        bytes.extend_from_slice(&if little { sample.to_le_bytes() } else { sample.to_be_bytes() });
    }
    bytes
}

fn document(source: SmartSource, order: Option<Order>) -> Document {
    let mut doc = Document::new("order", Size::new(4, 3), ColorMode::Rgb, SampleType::U16);
    let mut cache = Surface::new(doc.pixel_format());
    cache.write_region(Rect::new(0, 0, 2, 2), &[0.25, 0.5, 0.75, 1.].repeat(4));
    let mut smart = SmartObject::new(source, Affine::IDENTITY, Some(cache));
    smart.historical_xcf_sample_order = order;
    doc.layers.push(Layer::new("Historical", LayerContent::Smart(smart)));
    doc
}

fn smart(doc: &Document) -> &SmartObject {
    let LayerContent::Smart(smart) = &doc.layers[0].content else { panic!("smart source was reduced to pixels") };
    smart
}

fn exporter(doc: &Document) -> Ex<'static> {
    exporter_with_interrupt(doc, Interrupt::NONE)
}

fn exporter_with_interrupt<'a>(doc: &Document, ctl: Interrupt<'a>) -> Ex<'a> {
    Ex {
        ctl,
        fmt: doc.pixel_format(),
        dpi: doc.resolution_dpi,
        document: None,
        read_linked_files: false,
        text_styles: Default::default(),
        text_index: Default::default(),
        mask_fmt: PixelFormat::new(ColorMode::Grayscale, SampleType::U16, false),
        cc: 3,
        cmyk: false,
        version: Version::Psd,
        next_id: 0,
        used_ids: Default::default(),
        layer_ids: Default::default(),
        canvas: doc.bounds(),
        records: Vec::new(),
        warnings: Vec::new(),
        guides: Default::default(),
        comps: None,
        smart: SmartOut::new(doc, 0),
        patterns: Vec::new(),
        background: None,
        stage: None,
    }
}

#[test]
fn embedded_order_roundtrips_psd_psb_and_native_with_exact_source_samples() {
    for (order, little) in [(Order::LittleEndian, true), (Order::BigEndian, false)] {
        let bytes = Arc::new(xcf(little));
        let doc = document(SmartSource::Embedded { file_name: "historical.xcf".into(), bytes: bytes.clone() }, Some(order));
        for force_psb in [false, true] {
            let (file, warnings) = document_to_psd_with(&doc, &PsdExportOptions { force_psb, ..Default::default() }).unwrap();
            assert!(!warnings.iter().any(|warning| warning.contains("written as pixels")), "{warnings:?}");
            let record = &file.layer_info.as_ref().unwrap().layers[0];
            assert_eq!(record.block(&crate::smart_xcf_order::PSD_KEY).unwrap().data, crate::smart_xcf_order::encode(order));
            let placed = crate::smart_map::parse_sold(&record.block(b"SoLd").unwrap().data).unwrap();
            assert_eq!(crate::smart_map::stored_size(&placed.descriptor), Some((1., 1.)), "source geometry, not the 2x2 stored cache");
            let imported = crate::import("roundtrip.psd", &file.to_bytes().unwrap()).unwrap().document;
            let object = smart(&imported);
            assert_eq!(object.historical_xcf_sample_order, Some(order));
            let SmartSource::Linked { path: uuid } = &object.source else { panic!("PSD embedded UUID") };
            let (name, retained) = crate::linked::embedded_payload(&imported.metadata, uuid).unwrap();
            assert_eq!(retained, bytes.as_slice());
            assert_eq!(object.historical_xcf_link_digest, Some(*blake3::hash(retained).as_bytes()));
            let decoded = crate::smart_xcf_order::decode(&name, retained, order, &Interrupt::NONE).unwrap();
            assert_eq!(decoded.layers[0].surface().unwrap().read_region(decoded.bounds()), [0x1234u16, 0x5678, 0x9abc, 0xffff].map(|v| f32::from(v) / 65535.));
            let reopened = photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(&imported, &Default::default()).unwrap()).unwrap();
            assert_eq!(smart(&reopened).historical_xcf_sample_order, Some(order));
            assert_eq!(smart(&reopened).historical_xcf_link_digest, object.historical_xcf_link_digest);
        }
    }
}

#[test]
fn embedded_source_preview_caches_keep_opposite_orders_separate_and_legacy_default_compatible() {
    let bytes = xcf(true);
    let doc = document(SmartSource::Embedded { file_name: "same.xcf".into(), bytes: Arc::new(bytes.clone()) }, None);
    let mut ex = exporter(&doc);
    let contents = smart(&doc).contents_id;
    let hash = embed_source_hash(&bytes);
    for (order, expected) in [(Some(Order::LittleEndian), 0x1234u16), (Some(Order::BigEndian), 0x3412), (None, 0x3412)] {
        let source = ex.embed_hashed(contents, "same.xcf", &bytes, false, hash, order).unwrap();
        assert_eq!(source.size, (1., 1.));
        let (surface, bounds) = ex.source_composite(&source.uuid, order).unwrap();
        assert_eq!(bounds, Rect::new(0, 0, 1, 1));
        assert!((surface.read_region(bounds)[0] - f32::from(expected) / 65535.).abs() <= 1. / 65535.);
    }
    assert_eq!(ex.smart.converted.len(), 3);
    assert_eq!(ex.smart.composites.len(), 3);
}

#[test]
fn unknown_hints_are_retained_and_known_hints_clear_on_reset_or_rasterization() {
    let bytes = Arc::new(xcf(true));
    let doc = document(SmartSource::Embedded { file_name: "source.xcf".into(), bytes }, Some(Order::LittleEndian));
    let file = document_to_psd(&doc).unwrap();
    let imported = crate::import("source.psd", &file.to_bytes().unwrap()).unwrap().document;
    for rasterize in [false, true] {
        let mut reset = imported.clone();
        let LayerContent::Smart(object) = &mut reset.layers[0].content else { panic!("smart source") };
        object.historical_xcf_sample_order = None;
        object.historical_xcf_link_digest = None;
        let cache = object.cache.clone().unwrap();
        if rasterize {
            reset.layers[0].content = LayerContent::Raster(cache);
        }
        let saved = document_to_psd(&reset).unwrap();
        assert!(saved.layer_info.as_ref().unwrap().layers[0].block(&crate::smart_xcf_order::PSD_KEY).is_none());
    }
    for (payload, retained) in [(vec![0, 0, 0, 2, 0, 0, 0, 1], vec![0, 0, 0, 2, 0, 0, 0, 1]), (vec![1, 2, 3], vec![1, 2, 3, 0])] {
        let mut unknown = file.clone();
        unknown.layer_info.as_mut().unwrap().layers[0].blocks.retain(|b| b.key != crate::smart_xcf_order::PSD_KEY);
        unknown.layer_info.as_mut().unwrap().layers[0].blocks.push(TaggedBlock::new(crate::smart_xcf_order::PSD_KEY, payload));
        let unknown = crate::import("unknown.psd", &unknown.to_bytes().unwrap()).unwrap().document;
        assert_eq!(smart(&unknown).historical_xcf_sample_order, None);
        let saved = document_to_psd(&unknown).unwrap();
        assert!(saved.layer_info.as_ref().unwrap().layers[0].blocks.iter().any(|b| b.key == crate::smart_xcf_order::PSD_KEY && b.data == retained));
    }
    // The private hint is not a layer kind and must not promote an empty raster to adjustment.
    let mut empty = Document::new("empty", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    empty.layers.push(Layer::raster("Empty", empty.pixel_format()));
    let mut file = document_to_psd(&empty).unwrap();
    let record = &mut file.layer_info.as_mut().unwrap().layers[0];
    record.flags = LayerFlags(LayerFlags::BIT4_USEFUL | LayerFlags::PIXEL_DATA_IRRELEVANT);
    record.blocks.push(TaggedBlock::new(crate::smart_xcf_order::PSD_KEY, crate::smart_xcf_order::encode(Order::LittleEndian)));
    let back = crate::import("empty.psd", &file.to_bytes().unwrap()).unwrap().document;
    assert!(matches!(back.layers[0].content, LayerContent::Raster(_)));
}

#[test]
fn external_link_does_not_gain_embedded_order_semantics() {
    let path = std::env::temp_dir().join(format!("photocraft-psd-external-xcf-{}.xcf", std::process::id()));
    let bytes = xcf(true);
    std::fs::write(&path, &bytes).unwrap();
    let mut doc = document(SmartSource::Linked { path: path.to_string_lossy().into_owned() }, Some(Order::LittleEndian));
    let LayerContent::Smart(object) = &mut doc.layers[0].content else { panic!("smart source") };
    object.historical_xcf_link_digest = Some(*blake3::hash(&bytes).as_bytes());
    let (file, warnings) = document_to_psd_with(&doc, &PsdExportOptions { read_linked_files: true, ..Default::default() }).unwrap();
    assert!(warnings.iter().any(|w| w.contains("verified linked-source workflow")), "{warnings:?}");
    let record = &file.layer_info.as_ref().unwrap().layers[0];
    assert!(record.block(b"SoLd").is_none());
    assert!(record.block(&crate::smart_xcf_order::PSD_KEY).is_none());
    let unchanged = std::fs::read(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(unchanged, bytes);
}
