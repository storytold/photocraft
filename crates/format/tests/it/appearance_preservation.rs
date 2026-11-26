//! Native persistence of imported appearance policy; no renderer or source format is needed.
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, ShapeLayer, Size, TextLayer};
use photocraft_format::{load_from_bytes, manifest::ContentM, read_manifest, save_to_bytes};
use photocraft_raster::Surface;

fn document(mode: ColorMode, depth: SampleType, flags: (bool, bool, bool), cached: bool) -> Document {
    let mut document = Document::new("Saved appearance", Size::new(3, 3), mode, depth);
    let format = PixelFormat::new(mode, depth, true);
    let mut surface = Surface::new(format);
    let pixel = match (mode, depth) {
        (ColorMode::Grayscale, SampleType::F32) => vec![-2.5, 0.5],
        (ColorMode::Grayscale, _) => vec![0.25, 0.5],
        (_, SampleType::F32) => vec![-2.5, 3.25, 0.125, 0.5],
        _ => vec![0.25, 0.75, 0.125, 0.5],
    };
    surface.write_pixel(-1, 2, &pixel);
    let cache = cached.then_some(surface);
    document.layers.push(Layer::new(
        "Fixed source name",
        LayerContent::Text(TextLayer {
            text: "Text".into(),
            cache: cache.clone(),
            keep_layer_name: flags.0,
            cache_uses_raster_blending: flags.1,
            ..Default::default()
        }),
    ));
    document.layers.push(Layer::new("Saved whole shape", LayerContent::Shape(ShapeLayer { cache, cache_preserves_appearance: flags.2, ..Default::default() })));
    document
}

#[test]
fn imported_appearance_policy_and_cached_pixels_roundtrip_at_native_models_and_depths() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for flags in [(true, false, false), (false, true, false), (false, false, true), (true, true, true)] {
                let document = document(mode, depth, flags, true);
                let saved = save_to_bytes(&document, &Default::default()).unwrap();
                let restored = load_from_bytes(&saved).unwrap();
                assert_eq!(restored, document, "model={mode:?}, depth={depth:?}, flags={flags:?}");
                let LayerContent::Text(text) = &restored.layers[0].content else { panic!("text") };
                let LayerContent::Shape(shape) = &restored.layers[1].content else { panic!("shape") };
                assert_eq!((text.keep_layer_name, text.cache_uses_raster_blending, shape.cache_preserves_appearance), flags);
                assert_eq!(text.cache.as_ref().unwrap().pixel(-1, 2), shape.cache.as_ref().unwrap().pixel(-1, 2));
                if depth == SampleType::F32 {
                    assert_eq!(text.cache.as_ref().unwrap().pixel(-1, 2)[0], -2.5);
                }
            }
        }
    }
}

#[test]
fn appearance_policy_is_preserved_independently_of_cache_presence() {
    let document = document(ColorMode::Rgb, SampleType::U8, (true, true, true), false);
    let restored = load_from_bytes(&save_to_bytes(&document, &Default::default()).unwrap()).unwrap();
    assert_eq!(restored, document);
    let LayerContent::Text(text) = &restored.layers[0].content else { panic!("text") };
    let LayerContent::Shape(shape) = &restored.layers[1].content else { panic!("shape") };
    assert!(text.cache.is_none() && shape.cache.is_none());
    assert!(text.keep_layer_name && text.cache_uses_raster_blending && shape.cache_preserves_appearance);
}

#[test]
fn older_native_content_defaults_to_existing_policy_and_default_saves_omit_new_fields() {
    let ordinary = document(ColorMode::Rgb, SampleType::U8, (false, false, false), true);
    let manifest = read_manifest(&save_to_bytes(&ordinary, &Default::default()).unwrap()).unwrap();
    for layer in &manifest.document.layers {
        let content = serde_json::to_value(&layer.content).unwrap();
        for key in ["keep_layer_name", "cache_uses_raster_blending", "cache_preserves_appearance"] {
            assert!(content.get(key).is_none(), "false policy adds no new manifest key");
        }
    }
    let imported = document(ColorMode::Rgb, SampleType::U8, (true, true, true), true);
    let manifest = read_manifest(&save_to_bytes(&imported, &Default::default()).unwrap()).unwrap();
    for layer in &manifest.document.layers {
        let mut content = serde_json::to_value(&layer.content).unwrap();
        let object = content.as_object_mut().unwrap();
        object.remove("keep_layer_name");
        object.remove("cache_uses_raster_blending");
        object.remove("cache_preserves_appearance");
        match serde_json::from_value::<ContentM>(content).unwrap() {
            ContentM::Text { keep_layer_name, cache_uses_raster_blending, cache, .. } => {
                assert!(!keep_layer_name && !cache_uses_raster_blending);
                assert!(cache.is_some());
            }
            ContentM::Shape { cache_preserves_appearance, cache, .. } => {
                assert!(!cache_preserves_appearance);
                assert!(cache.is_some());
            }
            _ => panic!("text or shape"),
        }
    }
}
