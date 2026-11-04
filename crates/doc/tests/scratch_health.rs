//! Scratch health walks every nested mask surface in an isolated integration-test process.
#![cfg(not(target_arch = "wasm32"))]

use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerMask, Size, mask_paint};
use photocraft_geom::Rect;
use photocraft_raster::{Surface, spill};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

struct ScratchReset(PathBuf);

impl Drop for ScratchReset {
    fn drop(&mut self) {
        let _ = spill::configure(None, 0);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn document_scratch_health_includes_fill_maps_in_layer_masks() {
    let dir = std::env::temp_dir().join(format!("photocraft-doc-scratch-health-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    spill::configure(Some(&dir), 1).unwrap();
    let _reset = ScratchReset(dir);

    let mut document = Document::new("Scratch", Size::new(256, 256), ColorMode::Rgb, SampleType::U8);
    let mut fill_map = Surface::new(PixelFormat::GRAY8);
    fill_map.fill_rect(Rect::new(0, 0, 256, 256), &[0.75]);
    let mut mask = LayerMask::hide_all();
    mask.paint = Some(mask_paint::Stack {
        base: Surface::new(PixelFormat::GRAY8),
        layers: vec![mask_paint::Paint {
            name: "Fill".into(),
            visible: true,
            opacity: 1.0,
            blend: BlendMode::Normal,
            surface: Surface::new(PixelFormat::GRAY8),
            fill_map: Some(fill_map),
            fill_map_size: [256, 256],
            fill_bounds: None,
            kind: mask_paint::EffectKind::Fill { value: 1.0, rotation: 0.0, repeat_u: false, repeat_v: false, mirror_u: false, mirror_v: false },
        }],
        active: 0,
    });
    let mut layer = Layer::raster("Masked", PixelFormat::RGBA8);
    layer.mask = Some(mask);
    document.layers.push(layer);

    spill::settle();
    assert_eq!(spill::stats().resident_bytes, 0, "the fill-map tile is on scratch storage");
    let mut file = spill::scratch_handle_for_tests().unwrap();
    let mut valid = Vec::new();
    file.seek(SeekFrom::Start(0)).unwrap();
    file.read_to_end(&mut valid).unwrap();
    assert!(!valid.is_empty(), "the scratch file contains the fill-map tile");
    let mut corrupt = valid.clone();
    corrupt[0] ^= 0xff;
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(&corrupt).unwrap();
    file.sync_all().unwrap();

    let fill_map = document.layers[0].mask.as_ref().unwrap().paint.as_ref().unwrap().layers[0].fill_map.as_ref().unwrap();
    let _ = fill_map.read_region(Rect::new(0, 0, 1, 1));
    assert!(document.scratch_health().is_err(), "the nested fill-map corruption is detected");

    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(&valid).unwrap();
    file.sync_all().unwrap();
    for (_, tile) in fill_map.tiles() {
        tile.try_bytes().unwrap();
    }
    assert!(document.scratch_health().is_ok(), "restoring the tile clears its per-document health error");
}
