//! Layer identities are unique within a document, not across embedded smart documents.

use std::cell::Cell;
use std::collections::HashSet;
use std::sync::Arc;

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{CompLayerState, DocId, Document, Layer, LayerComp, LayerContent, SmartContentsId, SmartObject, SmartSource};
use photocraft_format::{ByteSource, FormatError, LoadOptions, SaveOptions, load_bytes_source, load_from_bytes_with, save_to_bytes, zip};
use photocraft_geom::{Affine, Rect, Size};

fn sample() -> Document {
    let mut doc = Document::new("layer identities", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    let mut first = Layer::raster("same name", doc.pixel_format());
    first.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 1, 1), &[1.0, 0.0, 0.0, 1.0]);
    let leaf = || Layer::raster("same name", doc.pixel_format());
    doc.layers = vec![first, Layer::group("left", vec![leaf(), Layer::group("nested", vec![leaf()])]), Layer::group("right", vec![leaf()])];
    doc.layer_comps.push(LayerComp {
        id: 1,
        name: "every layer".into(),
        comment: String::new(),
        apply_visibility: true,
        apply_position: false,
        apply_appearance: false,
        states: doc.walk().into_iter().map(|(_, _, l)| CompLayerState { layer: l.id, visible: Some(l.visible), position: None, appearance: None }).collect(),
    });
    doc
}

fn rewrite_manifest(bytes: &[u8], edit: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    let reader = zip::ZipReader::new(bytes).unwrap();
    let mut manifest: serde_json::Value = serde_json::from_slice(&reader.read_by_name("manifest.json", usize::MAX).unwrap()).unwrap();
    edit(&mut manifest);
    let mut writer = zip::ZipWriter::new();
    writer.add("manifest.json", &serde_json::to_vec(&manifest).unwrap()).unwrap();
    for entry in reader.entries.iter().filter(|e| e.name != "manifest.json") {
        writer.add(&entry.name, &reader.read(entry, usize::MAX).unwrap()).unwrap();
    }
    writer.finish().unwrap()
}

fn assert_duplicate(error: FormatError, id: u64) {
    assert!(matches!(&error, FormatError::Corrupt(message) if message == &format!("duplicate layer id {id}")), "{error}");
}

#[test]
fn duplicate_layer_ids_are_rejected_across_the_whole_tree() {
    let doc = sample();
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    // Top-level siblings, ancestor/descendant, nested siblings, and unrelated groups must all
    // share one identity namespace. Reidentifying cannot recover an ambiguous comp target.
    for (source, target) in [
        ("/document/layers/1/id", "/document/layers/2/id"),
        ("/document/layers/1/id", "/document/layers/1/content/children/1/content/children/0/id"),
        ("/document/layers/1/content/children/0/id", "/document/layers/1/content/children/1/id"),
        ("/document/layers/1/content/children/0/id", "/document/layers/2/content/children/0/id"),
        ("/document/layers/0/id", "/document/layers/1/content/children/1/content/children/0/id"),
    ] {
        let mut duplicate = 0;
        let damaged = rewrite_manifest(&bytes, |m| {
            duplicate = m.pointer(source).unwrap().as_u64().unwrap();
            *m.pointer_mut(target).unwrap() = duplicate.into();
        });
        for preserve_ids in [true, false] {
            let error = load_from_bytes_with(&damaged, &LoadOptions { preserve_ids, ..Default::default() }).unwrap_err();
            assert_duplicate(error, duplicate);
        }
    }
}

#[test]
fn duplicate_extreme_layer_ids_are_not_silently_remapped() {
    let bytes = save_to_bytes(&sample(), &SaveOptions::default()).unwrap();
    let damaged = rewrite_manifest(&bytes, |m| {
        m["document"]["layers"][0]["id"] = u64::MAX.into();
        m["document"]["layers"][2]["id"] = u64::MAX.into();
    });
    for preserve_ids in [true, false] {
        let error = load_from_bytes_with(&damaged, &LoadOptions { preserve_ids, ..Default::default() }).unwrap_err();
        assert_duplicate(error, u64::MAX);
    }
}

#[test]
fn duplicate_layer_ids_are_rejected_before_fetching_payloads() {
    struct ManifestOnly {
        json: Vec<u8>,
        payload_reads: Cell<usize>,
    }
    impl ByteSource for ManifestOnly {
        fn get(&self, path: &str, _max: usize) -> photocraft_format::Result<Vec<u8>> {
            if path == "manifest.json" {
                return Ok(self.json.clone());
            }
            self.payload_reads.set(self.payload_reads.get() + 1);
            Err(FormatError::Corrupt("payload fetched before validating layer ids".into()))
        }
    }
    let doc = sample();
    let duplicate = doc.layers[1].id.0;
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let damaged = rewrite_manifest(&bytes, |m| m["document"]["layers"][2]["id"] = duplicate.into());
    let reader = zip::ZipReader::new(&damaged).unwrap();
    let src = ManifestOnly { json: reader.read_by_name("manifest.json", usize::MAX).unwrap(), payload_reads: Cell::new(0) };
    for preserve_ids in [true, false] {
        assert_duplicate(load_bytes_source(&src, &LoadOptions { preserve_ids, ..Default::default() }).unwrap_err(), duplicate);
        assert_eq!(src.payload_reads.get(), 0);
    }
}

#[test]
fn unique_nested_layer_ids_preserve_comp_targets_in_both_load_modes() {
    let doc = sample();
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let original_ids: HashSet<_> = doc.walk().into_iter().map(|(_, _, l)| l.id).collect();
    for preserve_ids in [true, false] {
        let loaded = load_from_bytes_with(&bytes, &LoadOptions { preserve_ids, ..Default::default() }).unwrap();
        let walk = loaded.walk();
        let ids: HashSet<_> = walk.iter().map(|(_, _, l)| l.id).collect();
        assert_eq!(ids.len(), walk.len());
        for ((_, _, layer), state) in walk.iter().zip(&loaded.layer_comps[0].states) {
            assert_eq!(state.layer, layer.id);
            assert_eq!(loaded.layer(state.layer), Some(*layer));
        }
        if preserve_ids {
            assert_eq!(loaded, doc);
        } else {
            assert!(ids.is_disjoint(&original_ids));
        }
    }
}

#[test]
fn layer_identity_validation_keeps_smart_document_and_contents_scopes_independent() {
    let mut doc = sample();
    let mut inner = Document::new("inside", doc.size, doc.mode, doc.depth);
    inner.layers.push(doc.layers[0].clone());
    let bytes = Arc::new(save_to_bytes(&inner, &SaveOptions::default()).unwrap());
    let mut smart = SmartObject::new(SmartSource::Embedded { file_name: "inside.pcraft".into(), bytes: bytes.clone() }, Affine::IDENTITY, None);
    // These typed identities may have the same number without identifying the same thing.
    smart.contents_id = SmartContentsId(doc.layers[0].id.0);
    doc.id = DocId(doc.layers[0].id.0);
    let first = Layer::new("instance", LayerContent::Smart(smart));
    let second = first.duplicate();
    doc.layers.extend([first, second]);
    let outer = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    for preserve_ids in [true, false] {
        let opts = LoadOptions { preserve_ids, ..Default::default() };
        let loaded = load_from_bytes_with(&outer, &opts).unwrap();
        let instances: Vec<_> = loaded.layers.iter().filter_map(|l| if let LayerContent::Smart(s) = &l.content { Some(s) } else { None }).collect();
        assert_eq!(instances.len(), 2);
        assert_eq!(instances[0].contents_id, instances[1].contents_id);
        for instance in instances {
            let SmartSource::Embedded { bytes: stored, .. } = &instance.source else { panic!("embedded source expected") };
            assert_eq!(stored, &bytes);
            let restored = load_from_bytes_with(stored, &opts).unwrap();
            assert_eq!(restored.layers.len(), 1);
            if preserve_ids {
                assert_eq!(restored, inner);
                assert_eq!(restored.layers[0].id, loaded.layers[0].id);
            }
        }
        if preserve_ids {
            assert_eq!(loaded, doc);
        }
    }
}

#[test]
fn an_empty_layer_tree_loads_in_both_modes() {
    let doc = Document::new("empty", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    for preserve_ids in [true, false] {
        let loaded = load_from_bytes_with(&bytes, &LoadOptions { preserve_ids, ..Default::default() }).unwrap();
        assert!(loaded.layers.is_empty());
    }
}
