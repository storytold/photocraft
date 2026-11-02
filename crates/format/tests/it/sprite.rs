use std::{collections::BTreeMap, sync::Arc};

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, sprite::*};
use photocraft_geom::{Rect, Size};
use photocraft_raster::Surface;

#[test]
fn native_sprite_preserves_exact_indices_delays_metadata_and_remaps_all_layer_references() {
    let mut doc = Document::new("Sprite", Size::new(2, 1), ColorMode::Indexed, SampleType::U8);
    let mut indices = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false));
    let rect = Rect::from_xywh(-1, 0, 2, 1);
    indices.write_interleaved(rect, &[1, 2]);
    let palette = Arc::new(vec![
        PaletteEntry::default(),
        PaletteEntry { rgba: [255, 0, 0, 255], name: "First red".into() },
        PaletteEntry { rgba: [255, 0, 0, 255], name: "Distinct second red".into() },
    ]);
    let pixels = pixels::render_indices(&indices, rect, &palette, Some(0)).unwrap();
    let mut layer = Layer::raster("Ink", PixelFormat::RGBA8);
    layer.content = photocraft_doc::LayerContent::Raster(pixels.clone());
    let id = layer.id;
    doc.layers.push(layer);
    let cel = Arc::new(SpriteCel { rect, pixels, indices: Some(indices), opacity: 127, z_index: -2, linked_frame: None, precise: None });
    let first = Arc::new(SpriteFrame {
        duration_ms: 17,
        palette: palette.clone(),
        cels: [(id, cel.clone())].into(),
        metadata: vec![
            SpriteChunk { kind: 0x2020, target: MetadataTarget::Cel(id), bytes: Arc::new(vec![1, 0, 0, 0, 2, 0, b'O', b'K']) },
            SpriteChunk { kind: 0x2004, target: MetadataTarget::Layer(id), bytes: Arc::new(vec![1, 2, 3, 4]) },
        ],
    });
    let second = Arc::new(SpriteFrame {
        duration_ms: 83,
        palette,
        cels: BTreeMap::from([(id, Arc::new(SpriteCel { linked_frame: Some(0), ..cel.as_ref().clone() }))]),
        metadata: vec![],
    });
    doc.sprite = Some(Sprite {
        frames: vec![first, second],
        current: 0,
        layers: vec![SpriteLayer { id, flags: 19, opacity: 128, uuid: Some([7; 16]) }],
        indexed: true,
        transparent_index: 0,
        pixel_aspect: [2, 1],
        grid: [-1, -2, 16, 16],
        tags: vec![SpriteTag { name: "Walk".into(), from: 0, to: 1, direction: 2, repeat: 3, color: [12, 34, 56] }],
        profile: None,
    });
    doc.restore_sprite_frame().unwrap();
    let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
    let back = photocraft_format::load_from_bytes_with(&bytes, &photocraft_format::LoadOptions { preserve_ids: false, ..Default::default() }).unwrap();
    let loaded_id = back.layers[0].id;
    assert_ne!(loaded_id, id);
    let sprite = back.sprite.as_ref().unwrap();
    assert_eq!(sprite.layers[0].id, loaded_id);
    assert_eq!(sprite.frames[0].metadata[0].target, MetadataTarget::Cel(loaded_id));
    assert_eq!(sprite.frames[0].metadata[1].target, MetadataTarget::Layer(loaded_id));
    assert_eq!(sprite.tilemap_descriptor(loaded_id), Some([1, 2, 3, 4].as_slice()));
    assert_eq!(sprite.frames[1].cels[&loaded_id].linked_frame, Some(0));
    assert_eq!(sprite.frames[0].cels[&loaded_id].indices.as_ref().unwrap().to_interleaved(rect), [1, 2]);
    assert_eq!(sprite.frames.iter().map(|f| f.duration_ms).collect::<Vec<_>>(), [17, 83]);
    assert_eq!(sprite.frames[0].palette[1].rgba, sprite.frames[0].palette[2].rgba);
    assert_ne!(sprite.frames[0].palette[1].name, sprite.frames[0].palette[2].name);
    assert_eq!(sprite.pixel_aspect, [2, 1]);
    assert_eq!(sprite.grid, [-1, -2, 16, 16]);
    assert_eq!(sprite.tags[0].repeat, 3);
    assert_eq!(sprite.frames[0].cels[&loaded_id].z_index, -2);
    assert_eq!(sprite.frames[0].cels[&loaded_id].opacity, 127);
    assert_eq!(sprite.layers[0].uuid, Some([7; 16]));
}

#[test]
fn native_save_captures_displayed_cel_edits_and_versions_only_sprite_bundles() {
    use photocraft_doc::{LayerContent, sprite::*};
    let mut doc = Document::new("Painted animation", Size::new(2, 1), ColorMode::Rgb, SampleType::U8);
    let mut layer = Layer::raster("Ink", PixelFormat::RGBA8);
    layer.surface_mut().unwrap().write_pixel(0, 0, &[1.0, 0.0, 0.0, 1.0]);
    let id = layer.id;
    let pixels = layer.surface().unwrap().clone();
    doc.layers.push(layer);
    doc.sprite = Some(Sprite {
        current: 0,
        indexed: false,
        transparent_index: 0,
        pixel_aspect: [1, 1],
        grid: [0; 4],
        tags: vec![],
        profile: None,
        layers: vec![SpriteLayer { id, flags: 3, opacity: 255, uuid: None }],
        frames: vec![Arc::new(SpriteFrame {
            duration_ms: 100,
            palette: Arc::new(vec![]),
            metadata: vec![],
            cels: [(
                id,
                Arc::new(SpriteCel { rect: Rect::from_xywh(0, 0, 1, 1), pixels, indices: None, opacity: 255, z_index: 0, linked_frame: None, precise: None }),
            )]
            .into(),
        })],
    });
    doc.restore_sprite_frame().unwrap();
    if let LayerContent::Raster(surface) = &mut doc.layers[0].content {
        surface.write_pixel(0, 0, &[0.0, 1.0, 0.0, 1.0]);
    }
    let original = doc.clone();
    let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
    assert_eq!(doc, original, "saving captures into a copy");
    let read_version = |bytes: &[u8]| {
        let zip = photocraft_format::zip::ZipReader::new(bytes).unwrap();
        let raw = zip.read_by_name("manifest.json", usize::MAX).unwrap();
        serde_json::from_slice::<serde_json::Value>(&raw).unwrap()["format_version"].as_u64().unwrap()
    };
    assert_eq!(read_version(&bytes), 2);
    let back = photocraft_format::load_from_bytes(&bytes).unwrap();
    let sprite = back.sprite.as_ref().unwrap();
    assert_eq!(sprite.frames[0].cels[&back.layers[0].id].pixels.pixel(0, 0), [0.0, 1.0, 0.0, 1.0]);
    doc.sprite = None;
    assert_eq!(read_version(&photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap()), 1);
}
