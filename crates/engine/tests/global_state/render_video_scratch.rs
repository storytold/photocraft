//! Render Video never writes a frame whose scratch tiles can't be read (#14163): a PNG sequence
//! flattens each frame for its Alpha Channel choice before the checked export pipeline sees it, so
//! the placeholder pixels a failed read gives would otherwise be encoded as real pixels.
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer};
use photocraft_engine::Session;
use photocraft_geom::{Size, TileCoord};
use photocraft_raster::spill;
use serde_json::{Value, json};
use std::io::{Read, Seek, SeekFrom, Write};

/// How many frames Render Video handed its writer for `params`, and whether it succeeded.
fn render(session: &Session, params: &Value) -> (usize, bool) {
    let mut written = 0;
    let r = photocraft_engine::render_video_with(
        session,
        params,
        |_, _| {
            written += 1;
            Ok(())
        },
        |_, _| true,
    );
    (written, r.is_ok())
}

#[test]
fn render_video_refuses_unreadable_scratch_tiles() {
    photocraft_raster::spill::use_test_volume_space();
    let _scratch = crate::lock(&crate::PROCESS);
    let scratch = std::env::temp_dir().join(format!("photocraft-render-video-scratch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    let mut session = Session::new();
    session.edit_prefs(|p| {
        p.scratch_disks.disks = vec![photocraft_engine::prefs::ScratchDisk { path: scratch.to_string_lossy().into(), enabled: true }];
        p.performance.memory_usage_mb = 1;
    });
    assert!(spill::enabled());
    let mut doc = Document::new("Scratch frames", Size::new(2048, 256), ColorMode::Rgb, SampleType::U8);
    let mut pixels = Layer::raster("Pixels", PixelFormat::RGBA8);
    for i in 0..8 {
        let mut bytes = pixels.surface_mut().unwrap().tile_mut(TileCoord::new(i, 0)).bytes_mut();
        for (j, b) in bytes.iter_mut().enumerate() {
            *b = (j as u32).wrapping_mul(17).wrapping_add(i as u32 * 43) as u8;
        }
    }
    doc.layers.push(pixels);
    session.add_document(doc, None);
    // The default (no alpha), an alpha choice and its ".png" spelling flatten the frame first; JPEG
    // goes straight to the export pipeline.
    let cases = [
        json!({"format": "png"}),
        json!({"format": "png", "alpha": "straight"}),
        json!({"format": ".png", "alpha": "premultipliedColor", "matte": "#00ff00"}),
        json!({"format": "jpg"}),
    ];
    for params in &cases {
        assert_eq!(render(&session, params), (1, true), "{params} with readable tiles");
    }

    // Force every tile to disk, then damage only this test's scratch file, as
    // `artboard_export_scratch` does (see there for why the budget is set again until it holds).
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

    for params in &cases {
        assert_eq!(render(&session, params), (0, false), "{params} wrote a frame with unreadable tiles");
    }

    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(&good).unwrap();
    file.sync_all().unwrap();
    drop((session, file));
    spill::configure(None, 0).unwrap();
    let _ = std::fs::remove_dir_all(scratch);
}
