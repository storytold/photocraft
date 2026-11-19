//! Artboards to Files and Artboards to PDF never write a board whose scratch tiles can't be read:
//! the PNG-8 and PDF paths flatten the board themselves, outside the checked export pipeline, so
//! the placeholder pixels a failed read gives would otherwise be encoded as real pixels.
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Artboard, ArtboardBackground, Document, Layer, LayerContent};
use photocraft_engine::Session;
use photocraft_geom::{Rect, Size, TileCoord};
use photocraft_raster::spill;
use serde_json::json;
use std::io::{Read, Seek, SeekFrom, Write};

#[test]
fn artboard_exports_refuse_unreadable_scratch_tiles() {
    photocraft_raster::spill::use_test_volume_space();
    let _scratch = crate::lock(&crate::PROCESS);
    let root = std::env::temp_dir().join(format!("photocraft-artboard-scratch-{}", std::process::id()));
    let scratch = root.join("scratch");
    let out = root.join("out");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&out).unwrap();
    let mut session = Session::new();
    session.edit_prefs(|p| {
        p.scratch_disks.disks = vec![photocraft_engine::prefs::ScratchDisk { path: scratch.to_string_lossy().into(), enabled: true }];
        p.performance.memory_usage_mb = 1;
    });
    assert!(spill::enabled());
    let mut doc = Document::new("Scratch boards", Size::new(2048, 256), ColorMode::Rgb, SampleType::U8);
    let mut pixels = Layer::raster("Pixels", PixelFormat::RGBA8);
    for i in 0..8 {
        let mut bytes = pixels.surface_mut().unwrap().tile_mut(TileCoord::new(i, 0)).bytes_mut();
        for (j, b) in bytes.iter_mut().enumerate() {
            *b = (j as u32).wrapping_mul(17).wrapping_add(i as u32 * 43) as u8;
        }
    }
    let mut board = Layer::group("Board", vec![pixels]);
    if let LayerContent::Group(g) = &mut board.content {
        g.artboard = Some(Artboard { rect: Rect::new(0, 0, 2048, 256), background: ArtboardBackground::Transparent, preset: String::new() });
    }
    doc.layers.push(board);
    session.add_document(doc, None);
    let dir = out.to_string_lossy().into_owned();
    // Readable: every path writes.
    for format in ["png8", "pdf"] {
        session.execute("file.export.artboardsToFiles", json!({"dir": dir, "format": format})).unwrap();
    }
    let pdf = out.join("whole.pdf").to_string_lossy().into_owned();
    session.execute("file.export.artboardsToPdf", json!({"path": pdf})).unwrap();
    for entry in std::fs::read_dir(&out).unwrap() {
        std::fs::remove_file(entry.unwrap().path()).unwrap();
    }

    // Force every tile to disk, then damage only this test's scratch file. The session's memory
    // worker refreshes the budget from the preferences every two seconds; a refresh between
    // `configure` and `settle` would leave tiles resident, so set the one-byte budget again until
    // a settle has emptied memory (bounded).
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

    for format in ["png8", "pdf", "png24", "png"] {
        let r = session.execute("file.export.artboardsToFiles", json!({"dir": dir, "format": format}));
        assert!(r.is_err(), "{format} wrote a board with unreadable tiles: {r:?}");
    }
    assert!(session.execute("file.export.artboardsToPdf", json!({"path": pdf})).is_err(), "Artboards to PDF wrote unreadable tiles");
    let written: Vec<_> = std::fs::read_dir(&out).unwrap().map(|e| e.unwrap().path()).collect();
    assert!(written.is_empty(), "{written:?}");

    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(&good).unwrap();
    file.sync_all().unwrap();
    drop((session, file));
    spill::configure(None, 0).unwrap();
    let _ = std::fs::remove_dir_all(root);
}
