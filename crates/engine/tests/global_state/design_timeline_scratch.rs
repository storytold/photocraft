//! A design timeline whose own pixels became unreadable on the scratch disk can still be stopped:
//! the image's pixels are fine, so the user keeps it and loses only the timeline.
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer};
use photocraft_engine::Session;
use photocraft_geom::{Size, TileCoord};
use photocraft_raster::spill;
use serde_json::json;
use std::io::{Read, Seek, SeekFrom, Write};

#[test]
fn a_timeline_with_unreadable_pixels_can_be_stopped() {
    photocraft_raster::spill::use_test_volume_space();
    let _scratch = crate::lock(&crate::PROCESS);
    let root = std::env::temp_dir().join(format!("photocraft-design-scratch-{}", std::process::id()));
    let scratch = root.join("scratch");
    let _ = std::fs::remove_dir_all(&root);
    let mut session = Session::new();
    session.edit_prefs(|p| {
        p.scratch_disks.disks = vec![photocraft_engine::prefs::ScratchDisk { path: scratch.to_string_lossy().into(), enabled: true }];
        p.performance.memory_usage_mb = 1;
    });
    let mut doc = Document::new("Timeline scratch", Size::new(1024, 256), ColorMode::Rgb, SampleType::U8);
    let mut pixels = Layer::raster("Pixels", PixelFormat::RGBA8);
    for i in 0..4 {
        let mut bytes = pixels.surface_mut().unwrap().tile_mut(TileCoord::new(i, 0)).bytes_mut();
        for (j, b) in bytes.iter_mut().enumerate() {
            *b = (j as u32).wrapping_mul(13).wrapping_add(i as u32 * 31) as u8;
        }
    }
    // An empty layer stays when the pixels go: a document keeps one layer.
    doc.layers.push(Layer::raster("Empty", PixelFormat::RGBA8));
    doc.layers.push(pixels);
    session.add_document(doc, None);
    session.execute("design.start", json!({})).unwrap();
    // Only the timeline's base keeps the pixels now.
    session.execute("layer.delete", json!({})).unwrap();

    for _ in 0..20 {
        spill::configure(Some(&scratch), 1).unwrap();
        spill::settle();
        if spill::stats().resident_bytes == 0 {
            break;
        }
    }
    assert_eq!(spill::stats().resident_bytes, 0);
    let mut file = spill::scratch_handle_for_tests().unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    let mut good = Vec::new();
    file.read_to_end(&mut good).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(&vec![0; good.len()]).unwrap();
    file.sync_all().unwrap();
    // Reading a damaged tile is what marks it failed.
    let base = session.active().unwrap().doc.design_timeline.clone().unwrap().base.clone();
    base.for_each_surface(|s| {
        for (_, tile) in s.tiles() {
            let _ = tile.try_bytes();
        }
    });

    let refused = session.execute("layer.new.layer", json!({})).unwrap_err().to_string();
    assert!(refused.contains("stop it"), "the refusal says how to go on: {refused}");
    session.execute("design.stop", json!({})).unwrap();
    assert!(session.active().unwrap().doc.design_timeline.is_none());
    session.execute("layer.new.layer", json!({})).unwrap();

    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(&good).unwrap();
    file.sync_all().unwrap();
    drop((session, file, base));
    spill::configure(None, 0).unwrap();
    let _ = std::fs::remove_dir_all(root);
}
