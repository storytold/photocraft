//! Generate › Image Assets never writes an asset built from scratch tiles that failed to read
//! (#15259). It builds each asset from a copy of the document, outside the edit and export
//! guards, so the placeholder pixels a failed read gives would otherwise be encoded as real ones.
//! The placeholder is transparent, so the asset is a group whose other layer still reads: a
//! fully unreadable layer would only be refused as empty. The scratch disk is process-wide, so
//! this runs in the `global_state` binary and holds its lock.

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerId};
use photocraft_engine::cmd::web_cmds::generate_assets;
use photocraft_geom::{Rect, Size, TileCoord};
use photocraft_raster::spill;
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::Path;

/// Spills every tile, then zeroes the scratch file. Returns its handle and the bytes to restore.
fn damage_scratch() -> (std::fs::File, Vec<u8>) {
    spill::settle();
    assert_eq!(spill::stats().resident_bytes, 0, "every tile is on the scratch disk");
    let mut file = spill::scratch_handle_for_tests().unwrap();
    let mut good = Vec::new();
    file.seek(SeekFrom::Start(0)).unwrap();
    file.read_to_end(&mut good).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(&vec![0; good.len()]).unwrap();
    file.sync_all().unwrap();
    (file, good)
}

/// Writes `good` back and reads every tile of layer `id` again, which clears their failure.
fn restore_scratch(mut file: std::fs::File, good: &[u8], doc: &Document, id: LayerId) {
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(good).unwrap();
    file.sync_all().unwrap();
    for (_, tile) in doc.layer(id).unwrap().surface().unwrap().tiles() {
        tile.try_bytes().unwrap();
    }
    assert!(spill::check_integrity().is_ok());
    assert!(doc.scratch_health().is_ok());
}

/// Whether nothing is at `path`: a failed lookup for any other reason is not taken as absence.
fn absent(path: &Path) -> bool {
    matches!(std::fs::symlink_metadata(path), Err(e) if e.kind() == ErrorKind::NotFound)
}

#[test]
fn image_assets_refuse_unreadable_scratch_tiles() {
    let _scratch = crate::lock(&crate::PROCESS);
    let root = std::env::temp_dir().join(format!("photocraft-image-assets-scratch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let out = root.join("assets");
    let dir = out.to_string_lossy().into_owned();
    spill::configure(Some(&root.join("scratch")), 1).unwrap();
    let mut doc = Document::new("Assets", Size::new(2048, 256), ColorMode::Rgb, SampleType::U8);
    let mut layer = Layer::raster("Pixels", PixelFormat::RGBA8);
    let pixels = layer.id;
    for i in 0..8 {
        let mut bytes = layer.surface_mut().unwrap().tile_mut(TileCoord::new(i, 0)).bytes_mut();
        for (j, b) in bytes.iter_mut().enumerate() {
            *b = (j as u32).wrapping_mul(29).wrapping_add(i as u32 * 71) as u8 | 0x80;
        }
    }
    let group = Layer::group("asset.png, asset.webp", vec![layer]);
    let id = group.id;
    doc.layers.push(group);
    // A layer added after the damage is written to the scratch disk afresh, so it reads.
    let damage_with_readable_layer = |doc: &mut Document| {
        let damaged = damage_scratch();
        let mut fresh = Layer::raster("Fresh", PixelFormat::RGBA8);
        fresh.surface_mut().unwrap().fill_rect(Rect::from_xywh(0, 0, 16, 16), &[1.0, 0.0, 0.0, 1.0]);
        doc.layer_mut(id).unwrap().children_mut().unwrap().push(fresh);
        damaged
    };
    let remove_readable_layer = |doc: &mut Document| drop(doc.layer_mut(id).unwrap().children_mut().unwrap().pop());

    // Readable: both the optimizer branch and the WebP branch write.
    let (files, errors) = generate_assets(&doc, &dir);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(files.len(), 2);
    let expected: Vec<Vec<u8>> = ["asset.png", "asset.webp"].iter().map(|f| std::fs::read(out.join(f)).unwrap()).collect();
    std::fs::remove_dir_all(&out).unwrap();

    // The optimizer branch (PNG, JPEG, GIF): the first asset's reads fail while it is built, so it
    // is refused before writing; the second finds the tiles already unreadable and is refused
    // before building. The output folder is made only for a file about to be written, so a run
    // that writes nothing leaves none.
    doc.layer_mut(id).unwrap().name = "asset.png, second.jpg".into();
    let (file, good) = damage_with_readable_layer(&mut doc);
    let (files, errors) = generate_assets(&doc, &dir);
    assert!(files.is_empty(), "{files:?}");
    assert_eq!(errors.iter().map(|e| e["asset"].as_str().unwrap()).collect::<Vec<_>>(), ["asset.png", "second.jpg"]);
    assert!(doc.scratch_health().is_err());
    assert!(absent(&out), "no asset file or folder is written");
    remove_readable_layer(&mut doc);
    restore_scratch(file, &good, &doc, pixels);

    // The WebP branch, with the same failure during the build.
    doc.layer_mut(id).unwrap().name = "asset.webp".into();
    let (file, good) = damage_with_readable_layer(&mut doc);
    let (files, errors) = generate_assets(&doc, &dir);
    assert!(files.is_empty(), "{files:?}");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0]["asset"], "asset.webp");
    assert!(absent(&out), "no asset file or folder is written");
    remove_readable_layer(&mut doc);
    restore_scratch(file, &good, &doc, pixels);

    // Once the tiles read again, the next run writes the same files as before the failure.
    doc.layer_mut(id).unwrap().name = "asset.png, asset.webp".into();
    let (files, errors) = generate_assets(&doc, &dir);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(files.len(), 2);
    for (f, bytes) in ["asset.png", "asset.webp"].iter().zip(&expected) {
        assert_eq!(&std::fs::read(out.join(f)).unwrap(), bytes, "{f}");
    }

    drop(doc);
    spill::configure(None, 0).unwrap();
    let _ = std::fs::remove_dir_all(root);
}
