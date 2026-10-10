//! Shared contents and independent placed layers through PSD/PSB persistence.

use std::sync::Arc;

use photocraft_color::{BlendMode, ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, SmartContentsId, SmartFilter, SmartObject, SmartSource};
use photocraft_geom::{Affine, Size};
use photocraft_io::{ExportOptions, export, import, linked, smart_map};
use photocraft_psd::{PsdFile, TaggedBlock};
use photocraft_raster::Surface;

fn fixture() -> Document {
    let mut source = Document::new("contents", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    let mut pixels = Surface::new(source.pixel_format());
    pixels.fill_rect(source.bounds(), &[1.0, 0.0, 0.0, 1.0]);
    source.layers.push(Layer::new("red", LayerContent::Raster(pixels.clone())));
    let bytes = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
    let mut doc = Document::new("instances", Size::new(12, 4), ColorMode::Rgb, SampleType::U8);
    let smart = SmartObject::new(SmartSource::Embedded { file_name: "contents.pcraft".into(), bytes: Arc::new(bytes) }, Affine::IDENTITY, Some(pixels));
    doc.layers.push(Layer::new("original", LayerContent::Smart(smart)));
    doc
}

fn smart<'a>(doc: &'a Document, name: &str) -> &'a SmartObject {
    let layer = doc.layers.iter().find(|l| l.name == name).unwrap();
    let LayerContent::Smart(sm) = &layer.content else { panic!("{name} is no longer a smart object") };
    sm
}

fn smart_mut(layer: &mut Layer) -> &mut SmartObject {
    let LayerContent::Smart(sm) = &mut layer.content else { panic!("expected smart object") };
    sm
}

fn round_trip(doc: &Document, extension: &str) -> (PsdFile, Document) {
    let out = export(doc, extension, &ExportOptions::default()).unwrap();
    assert!(!out.warnings.iter().any(|w| w.contains("smart object written as pixels")), "{:?}", out.warnings);
    let file = PsdFile::from_bytes(&out.bytes).unwrap();
    let back = import(extension, &out.bytes).unwrap().document;
    (file, back)
}

fn sold(file: &PsdFile, name: &str) -> smart_map::Placed {
    let layer = file.layers().iter().find(|l| l.name() == name).unwrap();
    smart_map::parse_sold(&layer.block(b"SoLd").unwrap().data).unwrap()
}

fn gaussian(radius: f64) -> SmartFilter {
    SmartFilter {
        command: "filter.blur.gaussianBlur".into(),
        params: serde_json::json!({ "radius": radius }),
        blend: BlendMode::Normal,
        opacity: 1.0,
        visible: true,
    }
}

#[test]
fn psd_and_psb_keep_shared_contents_and_identical_independent_copies_distinct() {
    let mut doc = fixture();
    let mut shared = doc.layers[0].duplicate();
    shared.name = "shared".into();
    let sm = smart_mut(&mut shared);
    sm.transform = Affine::translate(4.0, 0.0);
    sm.cache = sm.cache.as_ref().map(|c| c.translated(4, 0, c.content_bounds()));
    sm.smart_filters.push(gaussian(2.0));
    let mut independent = doc.layers[0].duplicate();
    independent.name = "independent".into();
    smart_mut(&mut independent).contents_id = SmartContentsId::fresh();
    doc.layers.extend([shared, independent]);

    for extension in ["psd", "psb"] {
        let (file, back) = round_trip(&doc, extension);
        let a = sold(&file, "original");
        let b = sold(&file, "shared");
        let c = sold(&file, "independent");
        assert_eq!(a.idnt, b.idnt);
        assert_ne!(a.idnt, c.idnt);
        assert_eq!([a.placed, b.placed, c.placed].into_iter().collect::<std::collections::HashSet<_>>().len(), 3);
        let payloads: Vec<_> = file.global_blocks.iter().filter(|b| &b.key == b"lnk2").flat_map(|b| linked::parse_linked_files(&b.data)).collect();
        assert_eq!(payloads.len(), 2);
        assert_eq!(payloads[0].bytes, payloads[1].bytes, "independent identical contents still have separate payload identities");
        assert_eq!(smart(&back, "original").contents_id, smart(&back, "shared").contents_id);
        assert_ne!(smart(&back, "original").contents_id, smart(&back, "independent").contents_id);
        assert_eq!(smart(&back, "shared").transform, Affine::translate(4.0, 0.0));
        assert_eq!(smart(&back, "shared").smart_filters[0].params["radius"].as_f64(), Some(2.0));
        assert!(smart(&back, "original").smart_filters.is_empty());
        assert!(smart(&back, "independent").smart_filters.is_empty());
    }
}

#[test]
fn duplicated_imported_layer_keeps_source_but_gets_its_own_placement() {
    let (before, mut doc) = round_trip(&fixture(), "psd");
    let original_files = before.global_blocks.iter().find(|b| &b.key == b"lnk2").unwrap().data.clone();
    let mut duplicate = doc.layers[0].duplicate();
    duplicate.name = "duplicate".into();
    smart_mut(&mut duplicate).smart_filters.push(gaussian(3.0));
    // An imported UUID may have been generated under a different allocator lifetime and equal
    // the duplicate's first candidate. Preserve it for the original even when emitted second.
    let old = sold(&before, "original");
    let colliding = smart_map::uuid_from(format!("{}:{}", old.idnt, duplicate.id.0).as_bytes());
    let spec = smart_map::PlacedSpec {
        idnt: &old.idnt,
        placed: &colliding,
        transform: Affine::IDENTITY,
        perspective: None,
        size: (2.0, 2.0),
        dpi: 72.0,
        warp: None,
        filter_fx: None,
    };
    let original_raw = smart_map::sold_bytes(Some(&old.descriptor), &spec, &mut Vec::new());
    for layer in [&mut doc.layers[0], &mut duplicate] {
        smart_mut(layer).psd_raw = Some(Arc::new(original_raw.clone()));
        for (key, data) in &mut layer.psd_blocks {
            if key == b"SoLd" {
                *data = Arc::new(original_raw.clone());
            }
        }
    }
    doc.layers.push(duplicate);

    for reversed in [false, true] {
        let mut ordered = doc.clone();
        if reversed {
            ordered.layers.reverse();
        }
        let (after, back) = round_trip(&ordered, "psd");
        let original = sold(&after, "original");
        let duplicate = sold(&after, "duplicate");
        assert_eq!(original.idnt, duplicate.idnt);
        assert_eq!(original.placed, colliding);
        assert_ne!(original.placed, duplicate.placed);
        let filter_cache = after.global_blocks.iter().find(|b| &b.key == b"FEid").unwrap();
        let filter_cache = photocraft_psd::filter_effects::FilterEffects::parse(&filter_cache.data).unwrap();
        assert_eq!(filter_cache.items.len(), 1);
        assert_eq!(filter_cache.items[0].id, duplicate.placed);
        assert_eq!(after.layers().iter().find(|l| l.name() == "original").unwrap().block(b"SoLd").unwrap().data, original_raw);
        assert_eq!(after.global_blocks.iter().find(|b| &b.key == b"lnk2").unwrap().data, original_files);
        assert_eq!(smart(&back, "original").contents_id, smart(&back, "duplicate").contents_id);
        assert_eq!(smart(&back, "duplicate").smart_filters[0].params["radius"].as_f64(), Some(3.0));
        assert!(smart(&back, "original").smart_filters.is_empty());
    }
}

#[test]
fn missing_source_identifiers_do_not_join_imported_contents() {
    let (mut file, _) = round_trip(&fixture(), "psd");
    let spec = smart_map::PlacedSpec {
        idnt: "",
        placed: "placement",
        transform: Affine::IDENTITY,
        perspective: None,
        size: (2.0, 2.0),
        dpi: 72.0,
        warp: None,
        filter_fx: None,
    };
    let data = smart_map::sold_bytes(None, &spec, &mut Vec::new());
    let mut record = file.layers()[0].clone();
    record.blocks.retain(|b| !matches!(&b.key, b"SoLd" | b"SoLE" | b"PlLd"));
    record.blocks.push(TaggedBlock::new(*b"SoLd", data));
    *file.layers_mut() = vec![record.clone(), record];
    let (doc, _) = photocraft_io::psd_to_document(&file);
    let LayerContent::Smart(a) = &doc.layers[0].content else { panic!("first smart object missing") };
    let LayerContent::Smart(b) = &doc.layers[1].content else { panic!("second smart object missing") };
    assert_ne!(a.contents_id, b.contents_id);
    let (_, back) = round_trip(&doc, "psd");
    let LayerContent::Smart(a) = &back.layers[0].content else { panic!("first smart object lost on save") };
    let LayerContent::Smart(b) = &back.layers[1].content else { panic!("second smart object lost on save") };
    assert_ne!(a.contents_id, b.contents_id);
}

#[test]
fn independent_unavailable_link_retains_separate_preserved_record() {
    let (mut file, _) = round_trip(&fixture(), "psd");
    let block = file.global_blocks.iter_mut().find(|b| &b.key == b"lnk2").unwrap();
    // An opaque external item: the source UUID is known, but no embedded bytes can be loaded.
    block.data[8..12].copy_from_slice(b"liFE");
    let original_record = block.data.clone();
    let (mut doc, _) = photocraft_io::psd_to_document(&file);
    let mut independent = doc.layers[0].duplicate();
    independent.name = "independent".into();
    let sm = smart_mut(&mut independent);
    sm.contents_id = SmartContentsId::fresh();
    sm.psd_raw = None;
    independent.psd_blocks.retain(|(key, _)| !matches!(key, b"SoLd" | b"SoLE" | b"PlLd"));
    // Encounter the independent copy first: the original imported identity must keep its UUID.
    doc.layers.insert(0, independent);

    let (after, back) = round_trip(&doc, "psd");
    assert_eq!(sold(&file, "original").idnt, sold(&after, "original").idnt);
    assert_ne!(sold(&after, "original").idnt, sold(&after, "independent").idnt);
    let links = &after.global_blocks.iter().find(|b| &b.key == b"lnk2").unwrap().data;
    assert!(links.starts_with(&original_record));
    assert_eq!(linked::block_uuids(links).len(), 2);
    assert_ne!(smart(&back, "original").contents_id, smart(&back, "independent").contents_id);
    let (again, _) = round_trip(&doc, "psd");
    assert_eq!(after.to_bytes().unwrap(), again.to_bytes().unwrap());
}
