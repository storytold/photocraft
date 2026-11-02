use super::*;
use crate::{Document, Layer, LayerContent, Timeline};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Size;

fn indexed() -> Document {
    let mut doc = Document::new("Indexed animation", Size::new(2, 2), ColorMode::Indexed, SampleType::U8);
    let layer = Layer::raster("Ink", PixelFormat::RGBA8);
    let id = layer.id;
    doc.layers.push(layer);
    let palette = Arc::new(vec![
        PaletteEntry { rgba: [0, 0, 0, 255], name: "Black".into() },
        PaletteEntry { rgba: [255, 0, 0, 255], name: "Red A".into() },
        PaletteEntry { rgba: [255, 0, 0, 255], name: "Red B (same color, distinct index)".into() },
        PaletteEntry { rgba: [0, 0, 255, 255], name: "Blue".into() },
        PaletteEntry { rgba: [0, 255, 0, 255], name: "Transparent green".into() },
    ]);
    let rect = Rect::from_xywh(0, 0, 2, 1);
    let mut indices = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false));
    indices.write_interleaved(rect, &[1, 2]);
    let pixels = pixels::render_indices(&indices, rect, &palette, Some(4)).unwrap();
    let first = SpriteCel { rect, pixels, indices: Some(indices), opacity: 255, z_index: 0, linked_frame: None, precise: None };
    let second = SpriteCel {
        rect: rect.translate(-1, 1),
        pixels: first.pixels.translated(-1, 1, rect),
        indices: first.indices.as_ref().map(|s| s.translated(-1, 1, rect)),
        linked_frame: Some(0),
        ..first.clone()
    };
    doc.sprite = Some(Sprite {
        frames: vec![
            Arc::new(SpriteFrame { duration_ms: 17, palette: palette.clone(), cels: [(id, Arc::new(first))].into(), metadata: vec![] }),
            Arc::new(SpriteFrame { duration_ms: 83, palette, cels: [(id, Arc::new(second))].into(), metadata: vec![] }),
        ],
        current: 0,
        layers: vec![SpriteLayer { id, flags: 3, opacity: 255, uuid: None }],
        indexed: true,
        transparent_index: 4,
        pixel_aspect: [1, 1],
        grid: [0; 4],
        tags: vec![],
        profile: None,
    });
    doc.timeline = Some(Timeline::new(2, 10.0));
    doc.restore_sprite_frame().unwrap();
    doc
}

fn indices(doc: &Document, frame: usize) -> Vec<u8> {
    let cel = doc.sprite.as_ref().unwrap().frames[frame].cels.values().next().unwrap();
    cel.indices.as_ref().unwrap().to_interleaved(cel.rect)
}

#[test]
fn selecting_frames_preserves_duplicate_indices_offsets_links_and_variable_delays() {
    let mut doc = indexed();
    let before = doc.sprite.clone().unwrap();
    doc.select_sprite_frame(1).unwrap();
    assert_eq!(indices(&doc, 1), [1, 2]);
    assert_eq!(doc.layers[0].surface().unwrap().pixel(-1, 1), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(doc.sprite.as_ref().unwrap().frame_duration_ms(), 83);
    doc.select_sprite_frame(0).unwrap();
    assert_eq!(doc.sprite.as_ref().unwrap().duration_ms(), 100);
    assert_eq!(doc.sprite.as_ref().unwrap(), &before);
    assert!(doc.select_sprite_frame(2).is_err());
    assert_eq!(doc.sprite.as_ref().unwrap(), &before);
}

#[test]
fn painting_a_linked_image_changes_its_source_and_links_without_mutating_history() {
    let mut doc = indexed();
    let history = doc.clone();
    doc.select_sprite_frame(1).unwrap();
    let LayerContent::Raster(surface) = &mut doc.layers[0].content else { panic!("raster") };
    surface.write_pixel(-1, 1, &[0.0, 0.0, 1.0, 1.0]);
    doc.capture_sprite_frame().unwrap();
    assert_eq!(indices(&doc, 0), [3, 2]);
    assert_eq!(indices(&doc, 1), [3, 2]);
    assert_eq!(indices(&history, 0), [1, 2]);
    assert_eq!(indices(&history, 1), [1, 2]);
    assert_eq!(doc.sprite.as_ref().unwrap().frames[1].cels.values().next().unwrap().linked_frame, Some(0));
    doc.select_sprite_frame(0).unwrap();
    assert_eq!(doc.layers[0].surface().unwrap().pixel(0, 0), [0.0, 0.0, 1.0, 1.0]);
}

#[test]
fn palette_edits_recolor_the_selected_frame_without_reassigning_duplicate_indices() {
    let mut doc = indexed();
    doc.color_table.as_mut().unwrap().colors[1] = [0, 0, 255];
    doc.capture_sprite_frame().unwrap();
    assert_eq!(indices(&doc, 0), [1, 2]);
    assert_eq!(doc.layers[0].surface().unwrap().pixel(0, 0), [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(doc.layers[0].surface().unwrap().pixel(1, 0), [1.0, 0.0, 0.0, 1.0]);
    doc.select_sprite_frame(1).unwrap();
    assert_eq!(doc.layers[0].surface().unwrap().pixel(-1, 1), [1.0, 0.0, 0.0, 1.0], "other frame keeps its palette");
}

#[test]
fn expanding_an_indexed_cel_fills_unpainted_tiles_with_the_transparent_index() {
    let mut doc = indexed();
    let LayerContent::Raster(surface) = &mut doc.layers[0].content else { panic!("raster") };
    surface.write_pixel(512, 0, &[0.0, 0.0, 1.0, 1.0]);
    doc.capture_sprite_frame().unwrap();
    let values = indices(&doc, 0);
    assert_eq!(&values[..2], [1, 2]);
    assert_eq!(values[256], 4);
    assert_eq!(values[512], 3);
    assert_eq!(doc.layers[0].surface().unwrap().pixel(256, 0)[3], 0.0);
}

#[test]
fn cyclic_links_are_refused_and_forward_links_are_valid() {
    let mut doc = indexed();
    let sprite = doc.sprite.as_mut().unwrap();
    let id = sprite.layers[0].id;
    Arc::make_mut(Arc::make_mut(&mut sprite.frames[0]).cels.get_mut(&id).unwrap()).linked_frame = Some(1);
    assert!(sprite.validate().unwrap_err().contains("cycle"));
    Arc::make_mut(Arc::make_mut(&mut sprite.frames[1]).cels.get_mut(&id).unwrap()).linked_frame = None;
    sprite.validate().unwrap();
}

#[test]
fn changing_transparent_index_refreshes_every_frame_without_reassigning_samples() {
    let mut doc = indexed();
    doc.color_table.as_mut().unwrap().transparent = Some(2);
    doc.capture_sprite_frame().unwrap();
    assert_eq!(indices(&doc, 0), [1, 2]);
    assert_eq!(indices(&doc, 1), [1, 2]);
    doc.select_sprite_frame(1).unwrap();
    assert_eq!(doc.layers[0].surface().unwrap().pixel(0, 1)[3], 0.0);
    assert_eq!(doc.layers[0].surface().unwrap().pixel(-1, 1)[3], 1.0);
}

#[test]
fn removing_palette_transparency_is_refused_instead_of_silently_restoring_it() {
    let mut doc = indexed();
    let before = doc.sprite.clone();
    doc.color_table.as_mut().unwrap().transparent = None;
    assert!(doc.capture_sprite_frame().unwrap_err().contains("transparent palette index"));
    assert_eq!(doc.sprite, before);
}

#[test]
fn indexed_validation_checks_used_samples_and_sparse_defaults_against_each_frame_palette() {
    let mut doc = indexed();
    let sprite = doc.sprite.as_mut().unwrap();
    let id = sprite.layers[0].id;
    let cel = Arc::make_mut(Arc::make_mut(&mut sprite.frames[0]).cels.get_mut(&id).unwrap());
    cel.indices.as_mut().unwrap().write_interleaved(Rect::from_xywh(0, 0, 1, 1), &[255]);
    assert!(sprite.validate().unwrap_err().contains("missing palette entry"));

    let mut doc = indexed();
    let sprite = doc.sprite.as_mut().unwrap();
    let id = sprite.layers[0].id;
    let cel = Arc::make_mut(Arc::make_mut(&mut sprite.frames[0]).cels.get_mut(&id).unwrap());
    cel.indices = Some(Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false), &[1.0]));
    assert!(sprite.validate().unwrap_err().contains("default pixel"));

    let mut doc = indexed();
    let sprite = doc.sprite.as_mut().unwrap();
    let id = sprite.layers[0].id;
    let cel = Arc::make_mut(Arc::make_mut(&mut sprite.frames[0]).cels.get_mut(&id).unwrap());
    cel.indices.as_mut().unwrap().write_interleaved(Rect::from_xywh(50, 50, 1, 1), &[255]);
    sprite.validate().unwrap();
    Arc::make_mut(&mut Arc::make_mut(&mut sprite.frames[1]).palette).truncate(2);
    assert!(sprite.validate().unwrap_err().contains("missing palette entry"), "shared samples are checked against every palette size");
}

#[test]
fn sprite_equality_preserves_value_semantics_with_shared_and_distinct_frames() {
    let doc = indexed();
    let original = doc.sprite.unwrap();
    let mut copy = original.clone();
    assert_eq!(original, copy);
    copy.frames[0] = Arc::new(copy.frames[0].as_ref().clone());
    assert_eq!(original, copy);
    Arc::make_mut(&mut copy.frames[0]).duration_ms += 1;
    assert_ne!(original, copy);
    let mut copy = original.clone();
    copy.grid[0] = 1;
    assert_ne!(original, copy);
    let mut copy = original.clone();
    copy.current = 1;
    assert_ne!(original, copy);
}
