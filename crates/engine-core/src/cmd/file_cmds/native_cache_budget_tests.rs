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

#[test]
fn insertion_paths_reject_unrestored_filtered_png_and_embedded_raw_layers() {
    use photocraft_color::BlendMode;
    use photocraft_doc::SmartFilter;
    use photocraft_raw::testgen::{DngSpec, mosaic, scene};
    use serde_json::json;

    let scratch = tempfile::tempdir().unwrap();

    // 1. Embedded PNG with a visible Smart Filter and no saved cache.
    let mut filtered_png = missing_cache();
    filtered_png.layers[0].name = "Filtered PNG".into();
    let LayerContent::Smart(smart) = &mut filtered_png.layers[0].content else { panic!("expected a Smart Object") };
    smart.filters_enabled = true;
    smart.smart_filters.push(SmartFilter {
        command: "filter.blur.gaussianBlur".into(),
        params: json!({ "radius": 1.0 }),
        blend: BlendMode::Normal,
        opacity: 1.0,
        visible: true,
    });
    let filtered_bundle = photocraft_format::save_to_bytes(&filtered_png, &Default::default()).unwrap();
    let filtered_path = scratch.path().join("filtered-png.pcraft");
    std::fs::write(&filtered_path, &filtered_bundle).unwrap();

    // 2. Embedded camera RAW (DNG) with no saved cache, both with .dng and with a non-RAW name.
    let (w, h) = (8, 8);
    let dng_bytes = Arc::new(DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 65535)).build());
    assert_eq!(photocraft_formats::detect("", &dng_bytes), Some(photocraft_formats::Format::Raw));

    let make_raw_bundle = |layer_name: &str, file_name: &str, with_cache: bool| {
        let mut doc = Document::new("RawTest", Size::new(w as u32, h as u32), ColorMode::Rgb, SampleType::U16);
        let cache = with_cache.then(|| {
            let mut surface = photocraft_raster::Surface::new(doc.pixel_format());
            surface.fill_rect(photocraft_geom::Rect::new(0, 0, w as i32, h as i32), &[0.25, 0.5, 0.75, 1.0]);
            surface
        });
        let source = SmartSource::Embedded { file_name: file_name.into(), bytes: Arc::clone(&dng_bytes) };
        doc.layers.push(Layer::new(layer_name, LayerContent::Smart(SmartObject::new(source, Affine::IDENTITY, cache))));
        photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap()
    };

    let raw_bundle = make_raw_bundle("Embedded RAW", "photo.dng", false);
    let raw_path = scratch.path().join("embedded-raw.pcraft");
    std::fs::write(&raw_path, &raw_bundle).unwrap();

    let renamed_raw_bundle = make_raw_bundle("Renamed RAW", "raw.bin", false);
    let renamed_raw_path = scratch.path().join("renamed-raw.pcraft");
    std::fs::write(&renamed_raw_path, &renamed_raw_bundle).unwrap();

    for (label, layer_name, path, bundle) in [
        ("filtered PNG", "Filtered PNG", &filtered_path, &filtered_bundle),
        ("embedded RAW", "Embedded RAW", &raw_path, &raw_bundle),
        ("renamed RAW", "Renamed RAW", &renamed_raw_path, &renamed_raw_bundle),
    ] {
        let expected = format!("couldn't rebuild the cache for \"{layer_name}\"");

        let import_err = super::super::open::import_smart_source("bundle.pcraft", bundle, &JobCtx::new()).unwrap_err();
        assert!(import_err.to_string().contains(&expected), "{label}: {import_err}");

        let mut host = crate::session();
        host.execute("file.new", json!({ "width": 8, "height": 8 })).unwrap();
        let place_err = host.execute("file.placeEmbedded", json!({ "path": path })).unwrap_err();
        assert!(place_err.to_string().contains(&expected), "{label}: {place_err}");

        let mut opener = crate::session();
        let open_err = opener.execute("file.openAsSmartObject", json!({ "path": path })).unwrap_err();
        assert!(open_err.to_string().contains(&expected), "{label}: {open_err}");

        // Direct native File > Open still opens editable data with the missing-cache warning.
        let mut reader = crate::session();
        let report = super::super::open::open_bytes_as(&mut reader, "bundle.pcraft", bundle, None, None).unwrap();
        let warnings = report["warnings"].as_array().unwrap();
        assert_eq!(warnings.len(), 1, "{label}: {warnings:?}");
        assert!(warnings[0].as_str().unwrap().contains(layer_name), "{label}: {warnings:?}");
        let LayerContent::Smart(opened_smart) = &reader.active().unwrap().doc.layers[0].content else { panic!("expected a Smart Object") };
        assert!(opened_smart.cache.is_none(), "{label}: recovery eligibility rules must stay unchanged");
    }

    // An embedded RAW Smart Object whose cache is present is accepted by every insertion path.
    let cached_raw_bundle = make_raw_bundle("Cached RAW", "photo.dng", true);
    let cached_raw_path = scratch.path().join("cached-raw.pcraft");
    std::fs::write(&cached_raw_path, &cached_raw_bundle).unwrap();
    super::super::open::import_smart_source("cached-raw.pcraft", &cached_raw_bundle, &JobCtx::new()).unwrap();
    let mut host = crate::session();
    host.execute("file.new", json!({ "width": 8, "height": 8 })).unwrap();
    host.execute("file.placeEmbedded", json!({ "path": &cached_raw_path })).unwrap();
    let mut opener = crate::session();
    opener.execute("file.openAsSmartObject", json!({ "path": &cached_raw_path })).unwrap();
}
