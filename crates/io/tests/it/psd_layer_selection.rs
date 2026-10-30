//! Saved layer targeting is file metadata, independent of the pixels and of runtime layer ids.

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerId, SavedLayerSelection};
use photocraft_geom::Size;
use photocraft_io::{PsdExportOptions, document_to_psd, document_to_psd_with, import};
use photocraft_psd::resources::{ImageResource, ids};
use photocraft_psd::{Compression, LayerRecord, PsdFile, SectionType, TaggedBlock, Version, testgen};

fn record(name: &str, id: u32, section: Option<SectionType>) -> LayerRecord {
    let mut blocks = vec![TaggedBlock::unicode_name(name), TaggedBlock::layer_id(id)];
    if let Some(section) = section {
        blocks.push(TaggedBlock::section_divider(section, None, None));
    }
    LayerRecord { name: name.as_bytes().to_vec(), blocks, ..Default::default() }
}

fn flat_file() -> PsdFile {
    let mut file = testgen::small(Version::Psd, Compression::Raw);
    *file.layers_mut() = (0..5).map(|i| record(&format!("layer {i}"), 10 + i, None)).collect();
    targeting(&mut file, None, None);
    file
}

fn grouped_file() -> PsdFile {
    let mut file = flat_file();
    *file.layers_mut() = vec![
        record("Background", 10, None),
        record("</outer>", 11, Some(SectionType::BoundingDivider)),
        record("child", 12, None),
        record("</inner>", 13, Some(SectionType::BoundingDivider)),
        record("deep child", 14, None),
        record("inner", 15, Some(SectionType::ClosedFolder)),
        record("outer", 16, Some(SectionType::OpenFolder)),
        record("top", 17, None),
    ];
    file
}

fn targeting(file: &mut PsdFile, target: Option<u16>, selected: Option<&[u32]>) {
    file.resources.retain(|r| !matches!(r.id, ids::LAYER_STATE | ids::LAYER_SELECTION_IDS));
    if let Some(target) = target {
        file.resources.push(ImageResource::new(ids::LAYER_STATE, target.to_be_bytes().to_vec()));
    }
    if let Some(selected) = selected {
        let mut data = (selected.len() as u16).to_be_bytes().to_vec();
        data.extend(selected.iter().flat_map(|id| id.to_be_bytes()));
        file.resources.push(ImageResource::new(ids::LAYER_SELECTION_IDS, data));
    }
}

fn open(file: &PsdFile) -> (Document, Vec<String>) {
    let imported = import("target.psd", &file.to_bytes().unwrap()).unwrap();
    (imported.document, imported.warnings)
}

fn layer(doc: &Document, name: &str) -> LayerId {
    doc.walk().into_iter().find(|(_, _, l)| l.name == name).unwrap().2.id
}

fn assert_targeting(doc: &Document, active: Option<&str>, selected: &[&str]) {
    let saved = doc.saved_layer_selection.as_ref().expect("saved targeting");
    assert_eq!(saved.active.map(|id| doc.layer(id).unwrap().name.as_str()), active);
    assert_eq!(saved.selected.iter().map(|id| doc.layer(*id).unwrap().name.as_str()).collect::<Vec<_>>(), selected);
}

fn assert_resources(file: &PsdFile, target: Option<u16>, selected: Option<&[u32]>) {
    assert_eq!(file.resource(ids::LAYER_STATE).map(|r| r.data.clone()), target.map(|n| n.to_be_bytes().to_vec()));
    let data = selected.map(|ids| {
        let mut data = (ids.len() as u16).to_be_bytes().to_vec();
        data.extend(ids.iter().flat_map(|id| id.to_be_bytes()));
        data
    });
    assert_eq!(file.resource(ids::LAYER_SELECTION_IDS).map(|r| r.data.clone()), data);
    for id in [ids::LAYER_STATE, ids::LAYER_SELECTION_IDS] {
        assert!(file.resources.iter().filter(|r| r.id == id).count() <= 1);
    }
}

#[test]
fn third_of_five_layers_stays_active_after_a_byte_round_trip() {
    let mut source = flat_file();
    targeting(&mut source, Some(2), Some(&[12]));
    let (doc, warnings) = open(&source);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_targeting(&doc, Some("layer 2"), &["layer 2"]);
    let output = document_to_psd(&doc);
    assert_resources(&output, Some(2), Some(&[12]));
    assert_targeting(&open(&output).0, Some("layer 2"), &["layer 2"]);
}

#[test]
fn grouped_record_indices_include_dividers_and_keep_the_primary_distinct_from_the_set() {
    for (index, id, name) in [(0, 10, "Background"), (4, 14, "deep child"), (5, 15, "inner"), (6, 16, "outer")] {
        let mut source = grouped_file();
        targeting(&mut source, Some(index), Some(&[10, 14, 15, 16]));
        let (doc, warnings) = open(&source);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_targeting(&doc, Some(name), &["Background", "deep child", "inner", "outer"]);
        let output = document_to_psd(&doc);
        assert_resources(&output, Some(index), Some(&[10, 14, 15, 16]));
        assert_eq!(output.layers()[usize::from(index)].layer_id(), Some(id));
    }
}

#[test]
fn explicit_empty_selection_overrides_a_cached_target_and_exports_no_target() {
    let mut source = grouped_file();
    targeting(&mut source, Some(6), Some(&[]));
    let (doc, _) = open(&source);
    assert_targeting(&doc, None, &[]);
    let output = document_to_psd(&doc);
    assert_resources(&output, None, Some(&[]));
    assert_targeting(&open(&output).0, None, &[]);
}

#[test]
fn missing_metadata_stays_absent_and_a_legacy_index_needs_no_layer_ids() {
    let mut source = flat_file();
    let (doc, _) = open(&source);
    assert!(doc.saved_layer_selection.is_none());
    assert_resources(&document_to_psd(&doc), None, None);
    for record in source.layers_mut() {
        record.blocks.retain(|b| &b.key != b"lyid");
    }
    targeting(&mut source, Some(2), None);
    let (doc, warnings) = open(&source);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_targeting(&doc, Some("layer 2"), &["layer 2"]);
    let output = document_to_psd(&doc);
    let assigned = output.layers()[2].layer_id().unwrap();
    assert_resources(&output, Some(2), Some(&[assigned]));
}

#[test]
fn selection_ids_can_restore_without_an_index_and_keep_order_without_duplicates() {
    let mut source = flat_file();
    targeting(&mut source, None, Some(&[12, 10, 12, 11]));
    let (doc, warnings) = open(&source);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_targeting(&doc, Some("layer 1"), &["layer 2", "layer 0", "layer 1"]);
    assert_resources(&document_to_psd(&doc), Some(1), Some(&[12, 10, 11]));
}

#[test]
fn selection_ids_win_over_a_conflicting_target() {
    let mut source = flat_file();
    targeting(&mut source, Some(4), Some(&[10, 12]));
    let (doc, warnings) = open(&source);
    assert_targeting(&doc, Some("layer 2"), &["layer 0", "layer 2"]);
    assert!(warnings.iter().any(|w| w.contains("outside the saved selection")), "{warnings:?}");
}

#[test]
fn missing_ids_recover_the_index_or_leave_the_default_fallback_available() {
    let mut source = flat_file();
    targeting(&mut source, Some(2), Some(&[999]));
    let (doc, warnings) = open(&source);
    assert_targeting(&doc, Some("layer 2"), &["layer 2"]);
    assert!(warnings.iter().any(|w| w.contains("missing, ambiguous")), "{warnings:?}");
    targeting(&mut source, None, Some(&[999]));
    let (doc, _) = open(&source);
    assert!(doc.saved_layer_selection.is_none());
}

#[test]
fn duplicate_ids_including_dividers_never_select_an_arbitrary_record() {
    let mut source = grouped_file();
    source.layers_mut()[1].blocks.retain(|b| &b.key != b"lyid");
    source.layers_mut()[1].blocks.push(TaggedBlock::layer_id(10));
    targeting(&mut source, Some(0), Some(&[10, 14]));
    let (doc, warnings) = open(&source);
    assert_targeting(&doc, Some("deep child"), &["deep child"]);
    assert!(warnings.iter().any(|w| w.contains("ambiguous")), "{warnings:?}");
    source.layers_mut()[7].blocks.retain(|b| &b.key != b"lyid");
    source.layers_mut()[7].blocks.push(TaggedBlock::layer_id(14));
    targeting(&mut source, Some(4), Some(&[14]));
    let (doc, _) = open(&source);
    assert_targeting(&doc, Some("deep child"), &["deep child"]);
}

#[test]
fn malformed_resources_warn_and_fall_back_without_losing_the_document() {
    let mut source = flat_file();
    targeting(&mut source, Some(2), None);
    source.resources.push(ImageResource::new(ids::LAYER_SELECTION_IDS, vec![0, 2, 0, 0, 0, 12]));
    let (doc, warnings) = open(&source);
    assert_targeting(&doc, Some("layer 2"), &["layer 2"]);
    assert!(warnings.iter().any(|w| w.contains("1069") && w.contains("could not be read")), "{warnings:?}");
    targeting(&mut source, None, Some(&[10, 11]));
    source.resources.push(ImageResource::new(ids::LAYER_STATE, vec![0]));
    let (doc, warnings) = open(&source);
    assert_targeting(&doc, Some("layer 1"), &["layer 0", "layer 1"]);
    assert!(warnings.iter().any(|w| w.contains("1024") && w.contains("could not be read")), "{warnings:?}");
}

#[test]
fn divider_and_out_of_range_indices_are_ignored() {
    let mut source = grouped_file();
    for index in [1, 3, u16::MAX] {
        targeting(&mut source, Some(index), None);
        let (doc, warnings) = open(&source);
        assert!(doc.saved_layer_selection.is_none());
        assert!(warnings.iter().any(|w| w.contains("does not name an imported layer")), "{warnings:?}");
    }
}

#[test]
fn layer_id_zero_is_not_a_sentinel() {
    let mut source = flat_file();
    source.layers_mut()[0].blocks.retain(|b| &b.key != b"lyid");
    source.layers_mut()[0].blocks.push(TaggedBlock::layer_id(0));
    targeting(&mut source, Some(0), Some(&[0]));
    let (doc, warnings) = open(&source);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_targeting(&doc, Some("layer 0"), &["layer 0"]);
    assert_resources(&document_to_psd(&doc), Some(0), Some(&[0]));
}

#[test]
fn duplicate_resources_use_the_first_copy_and_export_never_reuses_raw_targeting() {
    let mut source = flat_file();
    targeting(&mut source, Some(2), Some(&[12]));
    source.resources.push(ImageResource::new(ids::LAYER_STATE, vec![0, 4]));
    source.resources.push(ImageResource::new(ids::LAYER_SELECTION_IDS, vec![0, 1, 0, 0, 0, 14]));
    let (mut doc, _) = open(&source);
    assert_targeting(&doc, Some("layer 2"), &["layer 2"]);
    for id in [ids::LAYER_STATE, ids::LAYER_SELECTION_IDS] {
        doc.metadata.psd_resources.push((id, String::new(), std::sync::Arc::new(vec![0, 0])));
    }
    assert_resources(&document_to_psd(&doc), Some(2), Some(&[12]));
    doc.saved_layer_selection = None;
    assert_resources(&document_to_psd(&doc), None, None);
}

#[test]
fn new_layers_use_assigned_psd_ids_at_every_supported_depth_and_mode() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for force_psb in [false, true] {
                let mut doc = Document::new("selection", Size::new(1, 1), mode, depth);
                doc.layers = (0..5).map(|i| Layer::raster(format!("layer {i}"), doc.pixel_format())).collect();
                let selected = vec![doc.layers[0].id, doc.layers[2].id];
                doc.saved_layer_selection = Some(SavedLayerSelection { active: Some(doc.layers[2].id), selected });
                let (output, warnings) = document_to_psd_with(&doc, &PsdExportOptions { force_psb, ..Default::default() });
                assert!(warnings.is_empty(), "{mode:?}/{depth:?}/{force_psb}: {warnings:?}");
                assert_eq!(output.header.version.is_psb(), force_psb);
                assert_resources(&output, Some(2), Some(&[output.layers()[0].layer_id().unwrap(), output.layers()[2].layer_id().unwrap()]));
                assert_targeting(&open(&output).0, Some("layer 2"), &["layer 0", "layer 2"]);
            }
        }
    }
}

#[test]
fn changed_and_deleted_targets_are_resolved_against_the_saved_document() {
    let mut source = flat_file();
    targeting(&mut source, Some(2), Some(&[12]));
    let (mut doc, _) = open(&source);
    let second = layer(&doc, "layer 1");
    doc.saved_layer_selection = Some(SavedLayerSelection { active: Some(second), selected: vec![second] });
    assert_resources(&document_to_psd(&doc), Some(1), Some(&[11]));
    doc.layers.remove(1);
    assert_resources(&document_to_psd(&doc), Some(3), Some(&[14]));
}

#[test]
fn layerless_files_and_exports_never_target_synthetic_or_discarded_layers() {
    let mut source = testgen::merged_only(Version::Psd, photocraft_psd::ColorMode::Rgb, 8, Compression::Raw, 1, 1);
    targeting(&mut source, Some(0), Some(&[1]));
    let (doc, _) = open(&source);
    assert!(doc.saved_layer_selection.is_none());
    for mode in [ColorMode::Multichannel, ColorMode::Bitmap, ColorMode::Indexed] {
        let mut doc = Document::new("layerless", Size::new(1, 1), mode, SampleType::U8);
        if mode == ColorMode::Indexed {
            doc.color_table = Some(photocraft_doc::ColorTable { colors: vec![[0, 0, 0], [255, 255, 255]], transparent: None });
        }
        let l = Layer::raster("discarded", doc.pixel_format());
        doc.saved_layer_selection = Some(SavedLayerSelection { active: Some(l.id), selected: vec![l.id] });
        doc.layers.push(l);
        for id in [ids::LAYER_STATE, ids::LAYER_SELECTION_IDS] {
            doc.metadata.psd_resources.push((id, String::new(), std::sync::Arc::new(vec![0, 0])));
        }
        let output = document_to_psd(&doc);
        assert!(output.layers().is_empty());
        assert_resources(&output, None, None);
    }
}

/// These expected selections and raw record indices come from Photoshop-authored files, not
/// from PhotoCraft's exporter. See the pins and SHA256 manifests in `xtask/`.
#[cfg(feature = "corpus")]
#[test]
fn pinned_photoshop_files_restore_the_saved_non_top_group_child_and_empty_selection() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    for (path, index, selected) in [
        ("photoshop/adjustments/rgb8/baseline-layer-copy.psd", Some(0), vec![1]),
        ("psd-tools/group.psd", Some(3), vec![3]),
        ("psd-tools/group.psb", Some(3), vec![3]),
        ("psd-tools/hidden-groups.psd", Some(6), vec![6]),
        ("psd-tools/artboard.psd", Some(3), vec![10]),
        ("psd-tools/transparency/transparency-group.psd", Some(14), vec![33]),
        ("psd-tools/empty-group.psd", None, vec![]),
    ] {
        let bytes = std::fs::read(root.join(path)).expect("selection corpus missing: run cargo xtask corpus --all");
        let source = PsdFile::from_bytes(&bytes).unwrap();
        let imported = import(path, &bytes).unwrap();
        let doc = imported.document;
        let saved = doc.saved_layer_selection.as_ref().expect("Photoshop's saved targeting");
        assert_eq!(saved.selected.iter().map(|id| doc.layer(*id).unwrap().psd_id.unwrap()).collect::<Vec<_>>(), selected, "{path}");
        assert_eq!(saved.active.map(|id| doc.layer(id).unwrap().psd_id.unwrap()), selected.last().copied(), "{path}");
        if let Some(index) = index {
            assert_eq!(source.layers()[usize::from(index)].layer_id(), selected.last().copied(), "{path}");
        }
        assert_resources(&document_to_psd(&doc), index, Some(&selected));
    }
}
