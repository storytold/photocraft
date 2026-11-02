//! Editable Aseprite documents, native persistence and independent rendering oracles (#3811).

use photocraft_doc::LayerContent;

const LAYERS: &[u8] = include_bytes!("../fixtures/aseprite/layers.aseprite");

#[test]
fn aseprite_application_authored_frames_and_tilemaps_match_application_rendered_oracles() {
    for (name, source, png) in [
        (
            "application-indexed",
            include_bytes!("../fixtures/aseprite/application-indexed.aseprite").as_slice(),
            include_bytes!("../fixtures/aseprite/application-indexed.png").as_slice(),
        ),
        (
            "application-tiles",
            include_bytes!("../fixtures/aseprite/application-tiles.aseprite").as_slice(),
            include_bytes!("../fixtures/aseprite/application-tiles.png").as_slice(),
        ),
        (
            "application-blends",
            include_bytes!("../fixtures/aseprite/application-blends.aseprite").as_slice(),
            include_bytes!("../fixtures/aseprite/application-blends.png").as_slice(),
        ),
    ] {
        let doc = photocraft_io::import("source.aseprite", source).unwrap().document;
        let oracle = photocraft_compose::flatten(&photocraft_io::import("oracle.png", png).unwrap().document);
        let native = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let restored = photocraft_format::load_from_bytes(&native).unwrap();
        let output = photocraft_io::export(&restored, "aseprite", &Default::default()).unwrap();
        let back = photocraft_io::import("export.aseprite", &output.bytes).unwrap().document;
        for mut candidate in [doc, restored, back] {
            let count = candidate.sprite.as_ref().unwrap().frames.len();
            let width = candidate.size.width as usize;
            assert_eq!(oracle.rect.width() as usize, width * count);
            for frame in 0..count {
                candidate.select_sprite_frame(frame).unwrap();
                let actual = photocraft_compose::flatten(&candidate);
                for (i, a) in actual.px.iter().enumerate() {
                    let b = oracle.px[i / width * width * count + frame * width + i % width];
                    for c in 0..4 {
                        let (a, b) = if c == 3 { (a[c], b[c]) } else { (a[c] * a[3], b[c] * b[3]) };
                        let tolerance = if name == "application-blends" { 2.0 } else { 1.0 } / 255.0 + 1e-6;
                        assert!((a - b).abs() <= tolerance, "{name} frame {frame} pixel {i} channel {c}: {a} vs {b}");
                    }
                }
            }
        }
        if let Ok(dir) = std::env::var("PCRAFT_ASEPRITE_APP_OUTPUT_DIR") {
            std::fs::write(std::path::Path::new(&dir).join(format!("{name}-photocraft.aseprite")), output.bytes).unwrap();
        }
    }
}

#[test]
fn aseprite_tilemaps_keep_ids_tilesets_properties_and_all_transform_pixels_through_native_saves() {
    let bytes = include_bytes!("../fixtures/aseprite/tiles.aseprite");
    let doc = photocraft_io::import("tiles.aseprite", bytes).unwrap().document;
    assert!(doc.layers[0].locks.all);
    let colors = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [255, 255, 0, 255]];
    let order = [[0, 1, 2, 3], [1, 0, 3, 2], [2, 3, 0, 1], [3, 2, 1, 0], [0, 2, 1, 3], [2, 0, 3, 1], [1, 3, 0, 2], [3, 1, 2, 0]];
    for (tile, indices) in order.iter().enumerate() {
        let expected: Vec<u8> = indices.iter().flat_map(|i| colors[*i]).collect();
        assert_eq!(doc.layers[0].surface().unwrap().to_interleaved(photocraft_geom::Rect::from_xywh(tile as i32 * 2, 0, 2, 2)), expected, "transform {tile}");
    }
    let native = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
    let restored = photocraft_format::load_from_bytes(&native).unwrap();
    let output = photocraft_io::export(&restored, "aseprite", &Default::default()).unwrap();
    let back = photocraft_io::import("back.aseprite", &output.bytes).unwrap().document;
    assert_eq!(back.layers[0].surface().unwrap(), doc.layers[0].surface().unwrap());
    for candidate in [&doc, &restored, &back] {
        let metadata = &candidate.sprite.as_ref().unwrap().frames[0].metadata;
        let tileset = metadata.iter().position(|m| m.kind == 0x2023).unwrap();
        let note = metadata.iter().skip(tileset + 1).find(|m| m.kind == 0x2020).unwrap();
        assert!(note.bytes.windows(b"Tileset note".len()).any(|b| b == b"Tileset note"), "tileset user data stays attached after each save boundary");
        assert_eq!(note.target, metadata[tileset].target);
    }
    let sprite = back.sprite.as_ref().unwrap();
    assert!(sprite.tilemap_descriptor(back.layers[0].id).is_some());
    assert!(sprite.frames[0].metadata.iter().any(|m| m.kind == 0x2023));
    if let Ok(path) = std::env::var("PCRAFT_ASEPRITE_TILE_OUTPUT") {
        std::fs::write(path, &output.bytes).unwrap();
    }
}

#[test]
fn aseprite_independent_animation_writer_fixture_survives_native_and_aseprite_export() {
    use photocraft_doc::sprite::MetadataTarget;
    let source = include_bytes!("../fixtures/aseprite/animation.aseprite");
    let doc = photocraft_io::import("animation.aseprite", source).unwrap().document;
    assert_eq!(doc.mode, photocraft_color::ColorMode::Indexed);
    let native = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
    let restored = photocraft_format::load_from_bytes(&native).unwrap();
    let out = photocraft_io::export(&restored, "aseprite", &Default::default()).unwrap();
    let mut back = photocraft_io::import("back.aseprite", &out.bytes).unwrap().document;
    let s = back.sprite.as_ref().unwrap();
    let id = back.layers[0].id;
    assert_eq!(s.frames.iter().map(|f| f.duration_ms).collect::<Vec<_>>(), [17, 83, 29]);
    assert_eq!(s.frames[0].cels[&id].indices.as_ref().unwrap().to_interleaved(photocraft_geom::Rect::from_xywh(-1, 0, 2, 2)), [1, 2, 3, 0]);
    assert_eq!(s.frames[1].cels[&id].linked_frame, Some(0));
    assert!(s.frames[2].cels.is_empty());
    assert_eq!(s.frames[0].palette[1].name, "Red A");
    assert_eq!(s.frames[0].palette[2].name, "Red B");
    assert_eq!(s.tags[0].repeat, 2);
    assert!(s.frames[0].metadata.iter().any(|m| m.target == MetadataTarget::Tags));
    assert!(s.frames[0].metadata.iter().any(|m| m.target == MetadataTarget::Layer(id)));
    assert!(s.frames[0].metadata.iter().any(|m| m.target == MetadataTarget::Cel(id)));
    // Golden pixels are hand-computable straight-alpha values, independent of our decoder.
    for (frame, at) in [(0, (-1, 0)), (1, (0, 0))] {
        back.select_sprite_frame(frame).unwrap();
        let image = back.layers[0].surface().unwrap();
        assert_eq!(image.to_interleaved(photocraft_geom::Rect::from_xywh(at.0, at.1, 2, 2)), [255, 0, 0, 255, 255, 0, 0, 255, 0, 0, 255, 128, 0, 0, 0, 0]);
    }
    if let Ok(path) = std::env::var("PCRAFT_ASEPRITE_ORACLE_OUTPUT") {
        std::fs::write(path, &out.bytes).unwrap();
    }
}

#[test]
fn aseprite_layers_open_with_their_tree_offsets_and_original_pixels() {
    let r = photocraft_io::import("layers.aseprite", LAYERS).unwrap();
    assert!(!r.source_read_only);
    assert!(!r.preview_only);
    let doc = r.document;
    assert_eq!((doc.size.width, doc.size.height), (2, 2));
    assert_eq!(doc.layers.len(), 2);
    assert_eq!(doc.layers[0].name, "Base");
    let LayerContent::Group(group) = &doc.layers[1].content else { panic!("editable group") };
    assert_eq!(group.children[0].name, "Top");
    let top = group.children[0].surface().unwrap();
    assert_eq!(top.pixel(0, 0), [0.0, 0.0, 0.0, 0.0]);
    assert_eq!(top.pixel(1, 0), [0.0, 0.0, 1.0, 128.0 / 255.0]);
    assert_eq!(top.pixel(1, 1), [0.0, 1.0, 0.0, 1.0]);
    let flat = photocraft_compose::flatten(&doc);
    let expected = [[1.0, 0.0, 0.0, 1.0], [127.0 / 255.0, 0.0, 128.0 / 255.0, 1.0], [1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]];
    for (a, b) in flat.px.iter().zip(expected) {
        for c in 0..4 {
            assert!((a[c] - b[c]).abs() < 1.0 / 255.0 + 1e-6, "{a:?} != {b:?}")
        }
    }
}

#[test]
fn aseprite_native_and_psd_round_trips_preserve_pixels_and_layers() {
    let r = photocraft_io::import("layers.ase", LAYERS).unwrap();
    let doc = r.document;
    for ext in ["pcraft", "psd"] {
        let out = photocraft_io::export(&doc, ext, &Default::default()).unwrap();
        let back = photocraft_io::import(&format!("back.{ext}"), &out.bytes).unwrap().document;
        assert_eq!(back.layers.len(), doc.layers.len());
        assert_eq!(back.layers[1].children().unwrap()[0].name, "Top");
        for (a, b) in photocraft_compose::flatten(&doc).px.iter().zip(photocraft_compose::flatten(&back).px) {
            for c in 0..4 {
                assert!((a[c] - b[c]).abs() < 1.0 / 255.0 + 1e-6, "{ext}: {a:?} != {b:?}")
            }
        }
    }
    assert!(photocraft_io::import("renamed.psd", LAYERS).is_ok());
    assert!(photocraft_io::import("not-a-sprite.aseprite", b"ASEF\0\x01").is_err());
}

#[test]
fn aseprite_independent_writer_grayscale_cel_has_exact_samples() {
    let bytes = include_bytes!("../fixtures/aseprite/gray.aseprite");
    let r = photocraft_io::import("gray.aseprite", bytes).unwrap();
    assert_eq!(r.document.mode, photocraft_color::ColorMode::Grayscale);
    let surface = r.document.layers[0].surface().unwrap();
    let region = photocraft_geom::Rect::from_xywh(-1, 1, 3, 1);
    assert_eq!(surface.to_interleaved(region), [32, 0, 128, 128, 240, 255]);
    let out = photocraft_io::export(&r.document, "pcraft", &Default::default()).unwrap();
    let back = photocraft_io::import("back.pcraft", &out.bytes).unwrap().document;
    assert_eq!(back.layers[0].surface().unwrap().to_interleaved(region), surface.to_interleaved(region));
}

/// Opt-in import timing and boundary-tile accounting on a 25 MP source; not a CI budget.
#[test]
#[ignore = "25 MP release-mode import benchmark"]
fn aseprite_import_25mp_release_benchmark() {
    // Retain the original Base layer, replace its cel with a large compressed checkerboard.
    let mut pixels = vec![0; 5000 * 5000 * 4];
    for (i, p) in pixels.chunks_exact_mut(4).enumerate() {
        p.copy_from_slice(if (i / 5000 / 32 + i % 5000 / 32) % 2 == 0 { &[32, 64, 96, 255] } else { &[192, 160, 128, 255] });
    }
    let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&pixels, 6);
    drop(pixels);
    let mut bytes = LAYERS[..128].to_vec();
    let layer = &LAYERS[144..172];
    let mut cel = vec![0; 26];
    let cel_len = (cel.len() + compressed.len()) as u32;
    cel[..4].copy_from_slice(&cel_len.to_le_bytes());
    cel[4..6].copy_from_slice(&0x2005u16.to_le_bytes());
    cel[12] = 255;
    cel[13..15].copy_from_slice(&2u16.to_le_bytes());
    cel[22..24].copy_from_slice(&5000u16.to_le_bytes());
    cel[24..26].copy_from_slice(&5000u16.to_le_bytes());
    let frame_len = (16 + layer.len() + cel.len() + compressed.len()) as u32;
    bytes.extend(frame_len.to_le_bytes());
    bytes.extend(0xf1fau16.to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend([0; 8]);
    bytes.extend(layer);
    bytes.extend(cel);
    bytes.extend(compressed);
    let len = bytes.len() as u32;
    bytes[..4].copy_from_slice(&len.to_le_bytes());
    bytes[8..10].copy_from_slice(&5000u16.to_le_bytes());
    bytes[10..12].copy_from_slice(&5000u16.to_le_bytes());
    let start = std::time::Instant::now();
    let r = photocraft_io::import("25mp.aseprite", &bytes).unwrap();
    let elapsed = start.elapsed();
    let surface = r.document.layers[0].surface().unwrap();
    assert_eq!(surface.pixel(0, 0), [32.0 / 255.0, 64.0 / 255.0, 96.0 / 255.0, 1.0]);
    assert_eq!(surface.content_bounds(), r.document.bounds());
    eprintln!("25 MP Aseprite import: {elapsed:?}, {} source bytes", bytes.len());
}
