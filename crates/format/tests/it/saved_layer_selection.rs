//! Saved layer targeting survives native bundles without becoming document pixel data.

use photocraft_doc::{Color, ColorMode, Document, Layer, LayerId, SampleType, SavedLayerSelection, Size};
use photocraft_format::{LoadOptions, PcraftWriter, SaveOptions, load_from_bytes, load_from_bytes_with, load_path, read_manifest, save_to_bytes, zip};

use crate::common::temp_dir;

fn document(depth: SampleType) -> Document {
    let mut doc = Document::with_background("selection", Size::new(2, 1), ColorMode::Rgb, depth, Color::WHITE);
    let child = Layer::raster("nested", doc.pixel_format());
    doc.layers.push(Layer::group("group", vec![child]));
    doc.layers.push(Layer::raster("top", doc.pixel_format()));
    doc
}

fn named(doc: &Document, name: &str) -> LayerId {
    doc.walk().into_iter().find(|(_, _, layer)| layer.name == name).unwrap().2.id
}

fn targeting(doc: &Document) -> SavedLayerSelection {
    let nested = named(doc, "nested");
    SavedLayerSelection { active: Some(nested), selected: vec![named(doc, "top"), nested] }
}

#[test]
fn layer_targeting_roundtrips_zip_and_directory_at_several_depths() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut doc = document(depth);
        for selection in [Some(targeting(&doc)), Some(SavedLayerSelection::default()), None] {
            doc.saved_layer_selection = selection;
            let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
            assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
            assert_eq!(read_manifest(&bytes).unwrap().format_version, 1);
            let dir = temp_dir("saved-selection");
            PcraftWriter::new().save_dir(&doc, &dir, &SaveOptions::default()).unwrap();
            assert_eq!(load_path(&dir).unwrap(), doc);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
}

#[test]
fn layer_targeting_follows_remapped_nested_ids() {
    let mut doc = document(SampleType::U8);
    doc.saved_layer_selection = Some(targeting(&doc));
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let back = load_from_bytes_with(&bytes, &LoadOptions { preserve_ids: false, ..Default::default() }).unwrap();
    assert_ne!(named(&back, "nested"), named(&doc, "nested"));
    assert_ne!(named(&back, "top"), named(&doc, "top"));
    assert_eq!(back.saved_layer_selection, Some(targeting(&back)));
}

#[test]
fn older_manifest_without_layer_targeting_keeps_the_top_default() {
    let mut doc = document(SampleType::U8);
    doc.saved_layer_selection = Some(targeting(&doc));
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let original = zip::ZipReader::new(&bytes).unwrap();
    let mut older = zip::ZipWriter::new();
    for entry in &original.entries {
        let mut data = original.read(entry, usize::MAX).unwrap();
        if entry.name == "manifest.json" {
            let mut manifest: serde_json::Value = serde_json::from_slice(&data).unwrap();
            assert!(manifest["document"].as_object_mut().unwrap().remove("saved_layer_selection").is_some());
            data = serde_json::to_vec(&manifest).unwrap();
        }
        older.add(&entry.name, &data).unwrap();
    }
    let mut back = load_from_bytes(&older.finish().unwrap()).unwrap();
    assert!(back.saved_layer_selection.is_none());
    assert_eq!(back.take_saved_layer_selection(), SavedLayerSelection { active: back.top_layer(), selected: back.top_layer().into_iter().collect() });
    doc.saved_layer_selection = None;
    let manifest = serde_json::to_value(read_manifest(&save_to_bytes(&doc, &SaveOptions::default()).unwrap()).unwrap()).unwrap();
    assert!(manifest["document"].get("saved_layer_selection").is_none(), "absent metadata stays absent on disk");
}

#[test]
fn dangling_layer_ids_do_not_become_an_explicit_deselection() {
    let mut doc = document(SampleType::U8);
    for selection in [
        SavedLayerSelection { active: Some(LayerId(u64::MAX)), selected: Vec::new() },
        SavedLayerSelection { active: None, selected: vec![LayerId(u64::MAX)] },
        SavedLayerSelection { active: doc.top_layer(), selected: vec![LayerId(u64::MAX)] },
    ] {
        doc.saved_layer_selection = Some(selection);
        let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
        for preserve_ids in [false, true] {
            let mut back = load_from_bytes_with(&bytes, &LoadOptions { preserve_ids, ..Default::default() }).unwrap();
            assert!(back.saved_layer_selection.is_none());
            let top = back.top_layer();
            assert_eq!(back.take_saved_layer_selection(), SavedLayerSelection { active: top, selected: top.into_iter().collect() });
        }
    }
    doc.saved_layer_selection = Some(SavedLayerSelection::default());
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let mut back = load_from_bytes_with(&bytes, &LoadOptions { preserve_ids: false, ..Default::default() }).unwrap();
    assert_eq!(back.take_saved_layer_selection(), SavedLayerSelection::default());
}

#[test]
fn surviving_selection_is_normalized_after_id_remapping() {
    let mut doc = document(SampleType::U8);
    let nested = named(&doc, "nested");
    doc.saved_layer_selection = Some(SavedLayerSelection { active: doc.top_layer(), selected: vec![LayerId(u64::MAX), nested, nested] });
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let back = load_from_bytes_with(&bytes, &LoadOptions { preserve_ids: false, ..Default::default() }).unwrap();
    let nested = named(&back, "nested");
    assert_eq!(back.saved_layer_selection, Some(SavedLayerSelection { active: Some(nested), selected: vec![nested] }));
}

#[test]
fn targeting_changes_update_manifest_and_reuse_cached_tiles() {
    let mut doc = document(SampleType::U16);
    let mut writer = PcraftWriter::new();
    let (first, initial) = writer.save_zip(&doc, &SaveOptions::default()).unwrap();
    assert!(initial.tiles_total > 0);
    doc.saved_layer_selection = Some(targeting(&doc));
    let (second, changed) = writer.save_zip(&doc, &SaveOptions::default()).unwrap();
    assert_ne!(first, second);
    assert_eq!(changed.tiles_written, 0);
    assert_eq!(changed.tiles_reused, changed.tiles_total);
    assert_eq!(changed.blobs_written, 0);
    assert_eq!(load_from_bytes(&second).unwrap().saved_layer_selection, doc.saved_layer_selection);

    let dir = temp_dir("saved-selection-cache");
    writer.save_dir(&doc, &dir, &SaveOptions::default()).unwrap();
    doc.saved_layer_selection = Some(SavedLayerSelection::default());
    let changed = writer.save_dir(&doc, &dir, &SaveOptions::default()).unwrap();
    assert_eq!(changed.tiles_written, 0);
    assert_eq!(changed.tiles_reused, changed.tiles_total);
    assert_eq!(load_path(&dir).unwrap().saved_layer_selection, doc.saved_layer_selection);
    std::fs::remove_dir_all(dir).unwrap();
}
