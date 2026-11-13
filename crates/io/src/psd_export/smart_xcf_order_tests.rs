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

#[test]
fn embedded_and_linked_historical_sources_keep_order_bytes_and_geometry_through_psd_psb() {
    let folder = std::env::temp_dir().join(format!("photocraft-psd-xcf-order-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    for (order, little) in [(Order::LittleEndian, true), (Order::BigEndian, false)] {
        let bytes = Arc::new(xcf(little));
        let path = folder.join(format!("{order}.xcf"));
        std::fs::write(&path, bytes.as_slice()).unwrap();
        for source in [
            SmartSource::Embedded { file_name: "historical.xcf".into(), bytes: bytes.clone() },
            SmartSource::Linked { path: path.to_string_lossy().into_owned() },
        ] {
            let doc = document(source, Some(order));
            for force_psb in [false, true] {
                let (file, warnings) = document_to_psd_with(&doc, &PsdExportOptions { force_psb, read_linked_files: true, ..Default::default() });
                assert!(!warnings.iter().any(|warning| warning.contains("written as pixels")), "{warnings:?}");
                let record = &file.layer_info.as_ref().unwrap().layers[0];
                assert_eq!(record.block(&crate::smart_xcf_order::PSD_KEY).unwrap().data, crate::smart_xcf_order::encode(order));
                let placed = crate::smart_map::parse_sold(&record.block(b"SoLd").unwrap().data).unwrap();
                assert_eq!(
                    crate::smart_map::stored_size(&placed.descriptor),
                    Some((1., 1.)),
                    "source geometry must come from decoding, not the 2x2 display cache"
                );
                let imported = crate::import("roundtrip.psd", &file.to_bytes().unwrap()).unwrap().document;
                let object = smart(&imported);
                assert_eq!(object.historical_xcf_sample_order, Some(order));
                let SmartSource::Linked { path: uuid } = &object.source else { panic!("PSD linked table") };
                let (name, retained) = crate::linked::embedded_payload(&imported.metadata, uuid).unwrap();
                assert_eq!(retained, bytes.as_slice());
                let decoded = crate::smart_xcf_order::decode(&name, retained, object.historical_xcf_sample_order).unwrap();
                assert_eq!(
                    decoded.layers[0].surface().unwrap().read_region(decoded.bounds()),
                    [0x1234u16, 0x5678, 0x9abc, 0xffff].map(|sample| f32::from(sample) / 65535.)
                );
            }
        }
    }
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn unknown_and_malformed_order_blocks_remain_verbatim_and_do_not_set_a_choice() {
    let bytes = Arc::new(xcf(true));
    let doc = document(SmartSource::Embedded { file_name: "source.xcf".into(), bytes }, None);
    let file = document_to_psd(&doc).unwrap();
    assert!(file.layer_info.as_ref().unwrap().layers[0].block(&crate::smart_xcf_order::PSD_KEY).is_none());
    assert_eq!(smart(&crate::import("standard.psd", &file.to_bytes().unwrap()).unwrap().document).historical_xcf_sample_order, None);
    // Layer blocks keep the pad byte inside odd data lengths for PSD reader compatibility.
    // Unknown versions/values stay byte-identical; malformed odd payloads retain canonical padding.
    for (raw, expected) in [
        (vec![0, 0, 0, 2, 0, 0, 0, 1], vec![0, 0, 0, 2, 0, 0, 0, 1]),
        (vec![0, 0, 0, 1, 0, 0, 0, 9], vec![0, 0, 0, 1, 0, 0, 0, 9]),
        (vec![1, 2, 3], vec![1, 2, 3, 0]),
    ] {
        let mut file = file.clone();
        file.layer_info.as_mut().unwrap().layers[0].blocks.push(TaggedBlock::new(crate::smart_xcf_order::PSD_KEY, raw.clone()));
        let mut imported = crate::import("unknown.psd", &file.to_bytes().unwrap()).unwrap().document;
        assert_eq!(smart(&imported).historical_xcf_sample_order, None);
        for choice in [None, Some(Order::LittleEndian)] {
            let LayerContent::Smart(object) = &mut imported.layers[0].content else { panic!("smart") };
            object.historical_xcf_sample_order = choice;
            let saved = document_to_psd(&imported).unwrap();
            let blocks = &saved.layer_info.as_ref().unwrap().layers[0].blocks;
            assert!(blocks.iter().any(|block| block.key == crate::smart_xcf_order::PSD_KEY && block.data == expected));
            let reopened = crate::import("unknown-roundtrip.psd", &saved.to_bytes().unwrap()).unwrap().document;
            assert_eq!(smart(&reopened).historical_xcf_sample_order, choice);
        }
    }
}

#[test]
fn known_order_blocks_are_removed_when_source_choice_is_reset_or_layer_is_rasterized() {
    let doc = document(SmartSource::Embedded { file_name: "source.xcf".into(), bytes: Arc::new(xcf(true)) }, Some(Order::LittleEndian));
    let file = document_to_psd(&doc).unwrap();
    let imported = crate::import("known.psd", &file.to_bytes().unwrap()).unwrap().document;
    for rasterize in [false, true] {
        let mut reset = imported.clone();
        let LayerContent::Smart(object) = &mut reset.layers[0].content else { panic!("smart") };
        object.historical_xcf_sample_order = None;
        let cache = object.cache.clone();
        if rasterize {
            reset.layers[0].content = LayerContent::Raster(cache.unwrap());
        }
        let saved = document_to_psd(&reset).unwrap();
        assert!(saved.layer_info.as_ref().unwrap().layers[0].block(&crate::smart_xcf_order::PSD_KEY).is_none());
    }
    // This private metadata is not interpreted on ordinary raster layers.
    let mut raster = imported;
    let cache = smart(&raster).cache.clone().unwrap();
    raster.layers[0].content = LayerContent::Raster(cache);
    let mut file = document_to_psd(&raster).unwrap();
    file.layer_info.as_mut().unwrap().layers[0]
        .blocks
        .push(TaggedBlock::new(crate::smart_xcf_order::PSD_KEY, crate::smart_xcf_order::encode(Order::BigEndian)));
    let back = crate::import("raster.psd", &file.to_bytes().unwrap()).unwrap().document;
    assert!(matches!(back.layers[0].content, LayerContent::Raster(_)));
}

#[test]
fn private_order_metadata_cannot_turn_an_empty_raster_into_an_unknown_adjustment() {
    let mut doc = Document::new("empty raster", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    doc.layers.push(Layer::raster("Empty", doc.pixel_format()));
    for bytes in [crate::smart_xcf_order::encode(Order::LittleEndian), vec![0, 0, 0, 2, 0, 0, 0, 1], vec![1, 2, 3]] {
        let mut file = document_to_psd(&doc).unwrap();
        let record = &mut file.layer_info.as_mut().unwrap().layers[0];
        assert!(record.rect.is_empty());
        record.flags = LayerFlags(LayerFlags::BIT4_USEFUL | LayerFlags::PIXEL_DATA_IRRELEVANT);
        record.blocks.push(TaggedBlock::new(crate::smart_xcf_order::PSD_KEY, bytes.clone()));
        let imported = crate::import("empty-raster.psd", &file.to_bytes().unwrap()).unwrap().document;
        assert!(matches!(imported.layers[0].content, LayerContent::Raster(_)));
        assert!(imported.layers[0].psd_blocks.iter().any(|(key, raw)| key == &crate::smart_xcf_order::PSD_KEY && raw.as_slice() == bytes.as_slice()));
    }
}

fn exporter(doc: &Document) -> Ex<'static> {
    Ex {
        ctl: Interrupt::NONE,
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
fn psd_source_conversion_decoding_and_composite_caches_distinguish_sample_order() {
    let bytes = xcf(true);
    let doc = document(SmartSource::Embedded { file_name: "same.xcf".into(), bytes: Arc::new(bytes.clone()) }, None);
    let mut ex = exporter(&doc);
    let contents = smart(&doc).contents_id;
    let hash = embed_source_hash(&bytes);
    for (order, expected) in [(Order::LittleEndian, 0x1234u16), (Order::BigEndian, 0x3412u16)] {
        let source = ex.embed_hashed(contents, "same.xcf", &bytes, true, hash, Some(order)).unwrap();
        assert_eq!(source.size, (1., 1.));
        let (surface, bounds) = ex.source_composite(&source.uuid, Some(order)).unwrap();
        assert_eq!(bounds, Rect::new(0, 0, 1, 1));
        let actual = surface.read_region(bounds);
        assert!((actual[0] - f32::from(expected) / 65535.).abs() <= 1. / 65535., "{order}: {actual:?}");
    }
    assert_eq!(ex.smart.converted.len(), 2);
    assert_eq!(ex.smart.docs.len(), 2);
    assert_eq!(ex.smart.composites.len(), 2);
    assert!(ex.embed_hashed(contents, "same.xcf", &bytes, true, hash, None).is_err(), "an unchosen source must not reuse an approved interpretation");
    assert_eq!(ex.smart.converted.len(), 3);
    let uuid = contents_uuid(contents, &bytes);
    assert!(ex.source_composite(&uuid, None).is_none());
}
