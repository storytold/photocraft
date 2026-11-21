use super::recover_native_embedded_caches;
use crate::jobs::JobCtx;
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Affine, Document, Layer, LayerContent, Size, SmartObject, SmartSource};
use photocraft_io::memory::{MemorySnapshot, with_mock_memory};
use std::sync::Arc;

fn missing_cache() -> Document {
    let image = photocraft_codecs::Image::from_u8(2, 2, photocraft_codecs::ChannelLayout::Rgba, [71, 84, 203, 255].repeat(4)).unwrap();
    let bytes = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let mut doc = Document::new("Test", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    let source = SmartSource::Embedded { file_name: "source.png".into(), bytes: Arc::new(bytes) };
    doc.layers.push(Layer::new("Embedded art", LayerContent::Smart(SmartObject::new(source, Affine::IDENTITY, None))));
    doc
}

#[test]
fn insufficient_native_cache_memory_leaves_the_warning_before_decode() {
    let mut doc = missing_cache();
    let no_headroom = MemorySnapshot { available_ram: Some(0), ..Default::default() };
    crate::allocation::fail_next_for_test();
    let recovered = with_mock_memory(no_headroom, || recover_native_embedded_caches(&mut doc, &JobCtx::new())).unwrap();
    assert!(crate::allocation::checkpoint("decoder checkpoint was unused").is_err(), "recovery must not even start the decoder");
    assert!(!recovered);
    let warnings = photocraft_io::open_warnings(&doc);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("Embedded art") && warnings[0].contains("no saved render cache"));
}

#[test]
fn each_recovered_tile_reduces_the_next_candidates_budget() {
    let mut doc = missing_cache();
    let LayerContent::Smart(smart) = doc.layers[0].content.clone() else { panic!("expected a Smart Object") };
    doc.layers.push(Layer::new("Second art", LayerContent::Smart(smart)));
    let one_tile = photocraft_raster::stored_tiles_bytes(1, &doc.pixel_format());
    // The widest source costs two RGB8 tiles, its composite one, and its padded output four.
    // Leave enough room for either recovery alone but not the previous layer's retained tile.
    let room = one_tile * 7 + 4096;
    let available_ram = Some(photocraft_io::memory::LOW_MEMORY_RESERVE + room);
    let memory = MemorySnapshot { available_ram, ..Default::default() };

    assert!(with_mock_memory(memory, || recover_native_embedded_caches(&mut doc, &JobCtx::new())).unwrap());
    assert!(matches!(&doc.layers[0].content, LayerContent::Smart(smart) if smart.cache.is_some()));
    assert!(matches!(&doc.layers[1].content, LayerContent::Smart(smart) if smart.cache.is_none()));
    let warnings = photocraft_io::open_warnings(&doc);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("Second art"));
}

#[test]
fn an_unverified_source_is_budgeted_for_16_bit_rgb_before_decode() {
    let mut doc = missing_cache();
    doc.mode = ColorMode::Grayscale;
    let pixels = [10000, 20000, 30000, 65535];
    let image = photocraft_codecs::Image::from_u16(2, 2, photocraft_codecs::ChannelLayout::Rgba, &pixels.repeat(4)).unwrap();
    let bytes = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let LayerContent::Smart(smart) = &mut doc.layers[0].content else { panic!("expected a Smart Object") };
    smart.source = SmartSource::Embedded { file_name: "rgb16.png".into(), bytes: Arc::new(bytes) };

    let gray_tile = photocraft_raster::stored_tiles_bytes(1, &doc.pixel_format());
    let available_ram = Some(photocraft_io::memory::LOW_MEMORY_RESERVE + gray_tile * 6 + 4096);
    let memory = MemorySnapshot { available_ram, ..Default::default() };
    crate::allocation::fail_next_for_test();
    let recovered = with_mock_memory(memory, || recover_native_embedded_caches(&mut doc, &JobCtx::new())).unwrap();
    assert!(crate::allocation::checkpoint("decoder checkpoint was unused").is_err(), "the Gray8 placement estimate alone would permit decoding");
    assert!(!recovered);
    assert!(photocraft_io::open_warnings(&doc)[0].contains("Embedded art"));

    // The same saved source fits a matching 16-bit document with enough headroom.
    doc.mode = ColorMode::Rgb;
    doc.depth = SampleType::U16;
    let memory = MemorySnapshot { available_ram: Some(512 << 20), ..Default::default() };
    assert!(with_mock_memory(memory, || recover_native_embedded_caches(&mut doc, &JobCtx::new())).unwrap());
    let LayerContent::Smart(smart) = &doc.layers[0].content else { panic!("expected a Smart Object") };
    let restored = smart.cache.as_ref().unwrap();
    assert_eq!(restored.format().sample, SampleType::U16);
    for (channel, value) in pixels.into_iter().enumerate() {
        assert_eq!((restored.sample_channel(0, 0, channel) * f32::from(u16::MAX)).round() as u16, value);
    }
    assert!(photocraft_io::open_warnings(&doc).is_empty());
}
