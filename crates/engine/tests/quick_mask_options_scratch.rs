//! Quick Mask options must remain unchanged when a document edit cannot read scratch pixels.
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer};
use photocraft_engine::Session;
use photocraft_geom::{Size, TileCoord};
use photocraft_raster::spill;
use serde_json::json;
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::Arc;

#[test]
fn failed_quick_mask_edit_keeps_options_and_preference() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = Session::new();
    session.edit_prefs(|prefs| {
        prefs.scratch_disks.disks = vec![photocraft_engine::prefs::ScratchDisk { path: dir.path().to_string_lossy().into(), enabled: true }];
        prefs.performance.memory_usage_mb = 1;
    });
    let mut doc = Document::new("Scratch", Size::new(2048, 256), ColorMode::Rgb, SampleType::U8);
    let mut layer = Layer::raster("Pixels", PixelFormat::RGBA8);
    let layer_id = layer.id;
    for i in 0..8 {
        let mut bytes = layer.surface_mut().unwrap().tile_mut(TileCoord::new(i, 0)).bytes_mut();
        for (j, byte) in bytes.iter_mut().enumerate() {
            *byte = (j as u32).wrapping_mul(17).wrapping_add(i as u32 * 43) as u8;
        }
    }
    doc.layers.push(layer);
    session.add_document(doc, None);
    session.select_layer(layer_id).unwrap();
    session.execute("select.rect", json!({"x":0,"y":0,"width":100,"height":100})).unwrap();
    session.execute("select.editInQuickMaskMode", json!({"on":true})).unwrap();
    let before = session.active().unwrap().doc.clone();
    let options = session.quick_mask_options;
    let selected = session.prefs().quick_mask_selected_areas;
    let revision = session.active().unwrap().revision;

    spill::configure(Some(dir.path()), 1).unwrap();
    spill::settle();
    let mut file = spill::scratch_handle_for_tests().unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    let mut good = Vec::new();
    file.read_to_end(&mut good).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(&vec![0; good.len()]).unwrap();
    file.sync_all().unwrap();
    let tile = before.layer(layer_id).unwrap().surface().unwrap().tiles().next().unwrap().1;
    assert!(tile.try_bytes().is_err());

    let result = session.execute("channel.options", json!({"channel":"quickMask","indicates":"selected","color":"#0000ff","opacity":30}));
    let options_after = session.quick_mask_options;
    let selected_after = session.prefs().quick_mask_selected_areas;
    let doc_unchanged = Arc::ptr_eq(&session.active().unwrap().doc, &before);
    let revision_after = session.active().unwrap().revision;

    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(&good).unwrap();
    file.sync_all().unwrap();
    for (_, tile) in before.layer(layer_id).unwrap().surface().unwrap().tiles() {
        let _ = tile.try_bytes();
    }
    let recovered = session.execute("channel.options", json!({"channel":"quickMask","indicates":"selected","color":"#0000ff","opacity":30}));
    let recovered_options = session.quick_mask_options;
    let recovered_selected = session.prefs().quick_mask_selected_areas;
    let recovered_indication = session.active().unwrap().doc.quick_mask.as_ref().unwrap().indicates;
    drop((session, before, file));
    spill::configure(None, 0).unwrap();

    assert!(result.is_err());
    assert_eq!(options_after, options);
    assert_eq!(selected_after, selected);
    assert!(doc_unchanged);
    assert_eq!(revision_after, revision);
    assert!(recovered.is_ok());
    assert!(recovered_selected);
    assert_eq!(recovered_options.indicates, photocraft_doc::ColorIndicates::SelectedAreas);
    assert_eq!(recovered_indication, recovered_options.indicates);
}
