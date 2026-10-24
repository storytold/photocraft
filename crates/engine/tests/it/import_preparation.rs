use std::sync::Arc;

use photocraft_color::{BlendMode, Color, ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, SmartFilter, SmartObject, SmartSource};
use photocraft_engine::{import_prepare::prepare_imported_document, jobs::JobCtx, smart_cmds};
use photocraft_geom::{Affine, Rect, Size};

fn smart_document(depth: SampleType) -> Document {
    let source = Document::with_background("Source", Size::new(4, 3), ColorMode::Rgb, depth, Color::rgb(1.0, 0.0, 0.0));
    wrap(&source)
}

fn wrap(source: &Document) -> Document {
    let mut doc = Document::new("Native smart", source.size, source.mode, source.depth);
    let object = SmartObject::new(
        SmartSource::Embedded { file_name: "contents.pcraft".into(), bytes: Arc::new(smart_cmds::encode_source(source).unwrap()) },
        Affine::IDENTITY,
        None,
    );
    doc.layers.push(Layer::new("Smart", LayerContent::Smart(object)));
    doc
}

#[test]
fn missing_native_smart_filter_cache_is_ready_before_the_first_edit() {
    for depth in SampleType::ALL {
        let mut doc = smart_document(depth);
        if let LayerContent::Smart(object) = &mut doc.layers[0].content {
            object.smart_filters.push(SmartFilter {
                command: "image.adjustments.invert".into(),
                params: serde_json::json!({}),
                blend: BlendMode::Normal,
                opacity: 1.0,
                visible: true,
            });
        }
        prepare_imported_document(&mut doc, &JobCtx::new()).unwrap();
        let pixels = doc.layers[0].surface().unwrap().read_region(Rect::new(0, 0, 1, 1));
        assert_eq!(pixels, vec![0.0, 1.0, 1.0, 1.0], "native invert, {depth:?}");
        let saved = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let mut reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        prepare_imported_document(&mut reopened, &JobCtx::new()).unwrap();
        assert_eq!(reopened, doc);
    }
}

#[test]
fn valid_caches_with_unavailable_source_or_filter_are_preserved() {
    let mut doc = smart_document(SampleType::U8);
    let mut cache = photocraft_raster::Surface::new(doc.pixel_format());
    cache.write_pixel(0, 0, &[0.0, 0.0, 1.0, 1.0]);
    if let LayerContent::Smart(object) = &mut doc.layers[0].content {
        object.cache = Some(cache);
        object.source = SmartSource::Linked { path: "unavailable-source.pcraft".into() };
        object.smart_filters.push(SmartFilter {
            command: "unknown.stored.filter".into(),
            params: serde_json::json!({}),
            blend: BlendMode::Normal,
            opacity: 1.0,
            visible: true,
        });
    }
    let before = doc.clone();
    prepare_imported_document(&mut doc, &JobCtx::new()).unwrap();
    assert_eq!(doc, before, "preserve upstream's cached-filter fallback");
}

#[test]
fn nested_native_contents_are_prepared_innermost_first() {
    let source = smart_document(SampleType::U16);
    let mut outer = wrap(&wrap(&source));
    prepare_imported_document(&mut outer, &JobCtx::new()).unwrap();
    assert_eq!(outer.layers[0].surface().unwrap().read_region(Rect::new(0, 0, 1, 1)), vec![1.0, 0.0, 0.0, 1.0]);
    let LayerContent::Smart(object) = &outer.layers[0].content else { panic!("native smart object expected") };
    let SmartSource::Embedded { bytes, .. } = &object.source else { panic!("embedded native contents expected") };
    let inner = photocraft_format::load_from_bytes(bytes).unwrap();
    assert!(inner.layers[0].surface().is_some());
}

#[test]
fn preparation_failure_and_cancellation_do_not_partially_modify_pending_data() {
    let mut doc = smart_document(SampleType::U8);
    let mut unavailable = doc.layers[0].clone();
    unavailable.id = photocraft_doc::LayerId::fresh();
    if let LayerContent::Smart(object) = &mut unavailable.content {
        object.source = SmartSource::Linked { path: "missing-and-not-authorized.pcraft".into() };
    }
    doc.layers.push(unavailable);
    let before = doc.clone();
    assert!(prepare_imported_document(&mut doc, &JobCtx::new()).is_err());
    assert_eq!(doc, before);
    let cancelled = JobCtx::new();
    cancelled.cancel();
    assert!(matches!(prepare_imported_document(&mut doc, &cancelled), Err(photocraft_engine::EngineError::Cancelled)));
    assert_eq!(doc, before);
}

#[test]
fn oversized_sparse_source_is_refused_before_native_composition_allocates() {
    let source = Document::new("Too large to prepare", Size::new(300_000, 300_000), ColorMode::Rgb, SampleType::F32);
    let mut doc = wrap(&source);
    let before = doc.clone();
    assert!(prepare_imported_document(&mut doc, &JobCtx::new()).is_err());
    assert_eq!(doc, before);
}

#[test]
fn synchronous_open_prepares_native_data_and_failed_open_preserves_the_session() {
    let mut session = photocraft_engine::Session::new();
    session.color.settings.ask_on_missing = false;
    let doc = smart_document(SampleType::U8);
    let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
    photocraft_engine::file_cmds::open_bytes_as(&mut session, "native.pcraft", &bytes, None, None).unwrap();
    assert!(session.active().unwrap().doc.layers[0].surface().is_some());
    let previous = session.active().unwrap().doc.clone();
    let mut broken = doc;
    if let LayerContent::Smart(object) = &mut broken.layers[0].content {
        object.source = SmartSource::Linked { path: "missing-source.pcraft".into() };
    }
    let bytes = photocraft_format::save_to_bytes(&broken, &Default::default()).unwrap();
    assert!(photocraft_engine::file_cmds::open_bytes_as(&mut session, "broken.pcraft", &bytes, None, None).is_err());
    assert_eq!(session.documents().len(), 1);
    assert_eq!(session.active().unwrap().doc, previous);
}

#[test]
fn stale_pixel_format_and_shapes_use_normal_native_preparation() {
    use photocraft_doc::{Fill, Path, ShapeLayer, Subpath};
    let mut doc = smart_document(SampleType::U16);
    if let LayerContent::Smart(object) = &mut doc.layers[0].content {
        object.cache = Some(photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8));
    }
    let shape = ShapeLayer {
        path: Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (3.0, 0.0), (3.0, 3.0)])]),
        fill: Some(Fill::Solid(Color::rgb(0.0, 1.0, 0.0))),
        ..Default::default()
    };
    let mut expected = shape.clone();
    photocraft_engine::vector_cmds::refresh_shape(&doc, &mut expected);
    doc.layers.push(Layer::new("Shape", LayerContent::Shape(shape)));
    prepare_imported_document(&mut doc, &JobCtx::new()).unwrap();
    assert_eq!(doc.layers[0].surface().unwrap().format(), doc.pixel_format());
    assert_eq!(doc.layers[1].surface(), expected.cache.as_ref());
}

#[test]
fn already_ready_native_documents_do_not_acquire_a_preparation_layer_count_limit() {
    let mut doc = Document::new("Many native layers", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    for _ in 0..65_537 {
        doc.layers.push(Layer::raster("Ready", doc.pixel_format()));
    }
    prepare_imported_document(&mut doc, &JobCtx::new()).unwrap();
    assert_eq!(doc.layers.len(), 65_537);
}

#[test]
fn background_open_and_held_color_choice_use_the_same_native_preparation() {
    use photocraft_engine::jobs::{OpenSource, Started};
    let doc = smart_document(SampleType::U8);
    let bytes = Arc::new(photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap());
    for ask in [false, true] {
        let mut session = photocraft_engine::Session::new();
        session.color.settings.ask_on_missing = ask;
        let opened = match session.start_open("native.pcraft", OpenSource::Bytes(bytes.clone())).unwrap() {
            Started::Done(value) => value,
            Started::Job(id) => session.wait_job(id).unwrap(),
        };
        if ask {
            assert!(session.documents().is_empty());
            let held = opened["held"].as_u64().unwrap();
            let (mut pending, _) = session.take_held_open(held).unwrap();
            session.color.resolve_open(&mut pending, "assignWorking").unwrap();
            session.prepare_imported_document(&mut pending, &JobCtx::new()).unwrap();
            session.add_document(pending, None);
        }
        assert_eq!(session.documents().len(), 1);
        let ready = &session.active().unwrap().doc;
        assert_eq!(ready.layers[0].surface().unwrap().read_region(Rect::new(0, 0, 1, 1)), vec![1.0, 0.0, 0.0, 1.0]);
        assert!(!session.active().unwrap().is_dirty());
    }
}

#[test]
fn missing_native_text_cache_uses_the_normal_text_model_and_renderer() {
    use photocraft_doc::TextLayer;
    let mut doc = Document::new("Native text", Size::new(160, 48), ColorMode::Rgb, SampleType::U8);
    let text = TextLayer { text: "PhotoCraft λ".into(), font_family: "Arial".into(), transform: Affine::translate(2.0, 24.0), ..Default::default() };
    let mut expected = text.clone();
    photocraft_engine::type_cmds::refresh(&doc, &mut expected);
    doc.layers.push(Layer::new("Type", LayerContent::Text(text)));
    prepare_imported_document(&mut doc, &JobCtx::new()).unwrap();
    assert_eq!(doc.layers[0].surface(), expected.cache.as_ref());
    let LayerContent::Text(actual) = &doc.layers[0].content else { panic!("editable native text expected") };
    assert_eq!(actual.text, expected.text);
    assert_eq!(actual.psd_raw, expected.psd_raw);
}
