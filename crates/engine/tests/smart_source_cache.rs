//! #10030: the decoded-source cache counts the bytes it holds, and Purge All and closing a
//! document with smart objects release them. One integration-test process keeps the global
//! cache isolated from parallel unit tests; the tests here take turns on it.
use std::sync::Mutex;

use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType};
use photocraft_doc::LayerContent;
use photocraft_engine::{Session, file_cmds, smart_cmds};
use serde_json::json;

static TURN: Mutex<()> = Mutex::new(());

const SIDE: u32 = 256;

fn png() -> Vec<u8> {
    let px = (0..SIDE * SIDE).flat_map(|i| [(i % SIDE) as u8, (i / SIDE) as u8, ((i * 7) % 251) as u8, 255]);
    let img = Image::from_raw(SIDE, SIDE, ChannelLayout::Rgba, SampleType::U8, px.collect()).unwrap();
    photocraft_codecs::encode(&img, Format::Png, &EncodeOptions::default()).unwrap()
}

/// Every cache `edit.purge.all` clears, emptied.
fn empty_caches() {
    smart_cmds::purge_source_cache();
    smart_cmds::purge_stack_cache();
    photocraft_compose::purge_effect_cache();
    photocraft_compose::masks::purge_cache();
    photocraft_compose::shape_split::purge_cache();
}

fn session_with_smart_object(bytes: &[u8]) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": SIDE, "height": SIDE})).unwrap();
    file_cmds::place_bytes(&mut s, "art.png", bytes.to_vec(), None, &json!({})).unwrap();
    s
}

fn decode(s: &Session, bytes: &[u8]) {
    smart_cmds::source_image("art.png", bytes, s.active().unwrap().doc.pixel_format()).unwrap();
}

#[test]
fn the_cache_counts_real_tile_bytes() {
    let _turn = TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    empty_caches();
    let bytes = png();
    let s = session_with_smart_object(&bytes);
    decode(&s, &bytes);
    let held = smart_cmds::source_cache_bytes();
    let px = (SIDE * SIDE) as usize;
    assert!(held >= 4 * px && held < 5 * px, "{held} bytes for {px} 8-bit RGBA pixels");
    assert_eq!(smart_cmds::purge_source_cache(), held);
    assert_eq!(smart_cmds::source_cache_bytes(), 0);
}

#[test]
fn purge_all_reports_and_empties_the_source_cache() {
    let _turn = TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    empty_caches();
    let bytes = png();
    let mut s = session_with_smart_object(&bytes);
    // Nothing else holds anything to purge: only the source cache enables Purge All.
    s.execute("edit.purge.histories", json!({})).unwrap();
    s.clipboard = None;
    assert!(!s.is_enabled("edit.purge.all"));
    decode(&s, &bytes);
    assert!(smart_cmds::source_cache_bytes() > 0);
    assert!(s.is_enabled("edit.purge.all"));
    let purged = s.execute("edit.purge.all", json!({})).unwrap();
    assert!(purged["purged"].as_array().unwrap().contains(&json!("smart object source cache")), "{purged}");
    assert_eq!(smart_cmds::source_cache_bytes(), 0);
    assert!(!s.is_enabled("edit.purge.all"));
}

#[test]
fn closing_a_document_with_a_smart_object_empties_the_source_cache() {
    let _turn = TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    empty_caches();
    let bytes = png();
    let mut s = session_with_smart_object(&bytes);
    decode(&s, &bytes);
    assert!(smart_cmds::source_cache_bytes() > 0);
    s.execute("file.close", json!({})).unwrap();
    assert_eq!(smart_cmds::source_cache_bytes(), 0);

    // A document without smart objects leaves it alone.
    let mut plain = Session::new();
    plain.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let s = session_with_smart_object(&bytes);
    decode(&s, &bytes);
    plain.execute("file.close", json!({})).unwrap();
    assert!(smart_cmds::source_cache_bytes() > 0);
    empty_caches();
}

/// Two original native archive levels whose innermost Smart layer needs native preparation.
fn nested_source(depth: photocraft_doc::SampleType, size: photocraft_geom::Size, automatic: bool) -> Vec<u8> {
    use photocraft_doc::{BlendMode, ColorMode, Document, Layer, SmartFilter, SmartObject, SmartSource};
    use photocraft_geom::{Affine, Rect};
    let mut leaf = Document::new("Original nested pixels", size, ColorMode::Rgb, depth);
    let mut pixels = Layer::raster("pixels", leaf.pixel_format());
    pixels.surface_mut().unwrap().fill_rect(leaf.bounds(), &[0.2, 0.4, 0.6, 1.]);
    pixels.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 4, 4), &[0.8, 0.6, 0.3, 0.5]);
    leaf.layers.push(pixels);
    for level in 0..2 {
        let bytes = smart_cmds::encode_source(&leaf).unwrap();
        let mut object = SmartObject::new(SmartSource::Embedded { file_name: "nested.pcraft".into(), bytes: bytes.into() }, Affine::IDENTITY, None);
        if level == 0 {
            object.smart_filters.push(SmartFilter {
                command: if automatic { "image.autoContrast" } else { "filter.blur.gaussianBlur" }.into(),
                params: if automatic { json!({}) } else { json!({"radius":1.25}) },
                blend: BlendMode::Normal,
                opacity: 1.,
                visible: true,
            });
        }
        let mut wrapper = Document::new(format!("Nested level {level}"), size, ColorMode::Rgb, depth);
        wrapper.layers.push(Layer::new("nested", LayerContent::Smart(object)));
        leaf = wrapper;
    }
    smart_cmds::encode_source(&leaf).unwrap()
}

fn nested_session(bytes: &[u8], depth: photocraft_doc::SampleType) -> Session {
    use photocraft_doc::{BlendMode, ColorMode, Document, Layer, SmartFilter, SmartObject, SmartSource};
    use photocraft_geom::{Affine, Size};
    let mut doc = Document::new("Outer native object", Size::new(16, 16), ColorMode::Rgb, depth);
    let mut object = SmartObject::new(SmartSource::Embedded { file_name: "outer.pcraft".into(), bytes: bytes.to_vec().into() }, Affine::IDENTITY, None);
    object.smart_filters.push(SmartFilter {
        command: "filter.blur.gaussianBlur".into(),
        params: json!({"radius":0.5}),
        blend: BlendMode::Normal,
        opacity: 1.,
        visible: true,
    });
    let mut layer = Layer::new("outer", LayerContent::Smart(object));
    assert!(smart_cmds::refresh_layer(&doc, &mut layer).unwrap());
    doc.layers.push(layer);
    let mut session = Session::new();
    session.add_document(doc, None);
    session
}

fn deny_nested_blur(command: &str, params: &serde_json::Value) -> photocraft_engine::Result<()> {
    if command == "filter.blur.gaussianBlur" && params == &json!({"radius":1.25}) {
        Err(photocraft_engine::EngineError::Other("denied original nested blur parameters".into()))
    } else {
        Ok(())
    }
}

fn deny_original_auto(command: &str, _: &serde_json::Value) -> photocraft_engine::Result<()> {
    if command == "image.autoContrast" { Err(photocraft_engine::EngineError::Other("denied original auto action".into())) } else { Ok(()) }
}

fn deny_generated_adjustment(command: &str, _: &serde_json::Value) -> photocraft_engine::Result<()> {
    if matches!(command, "image.adjustments.levels" | "image.adjustments.curves") {
        Err(photocraft_engine::EngineError::Other("denied generated fixed parameters".into()))
    } else {
        Ok(())
    }
}

#[test]
fn nested_sources_reuse_pixels_through_filter_edits_transform_and_undo_at_every_depth() {
    use photocraft_doc::SampleType;
    use photocraft_geom::Size;
    let _turn = TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    for depth in SampleType::ALL {
        empty_caches();
        let bytes = nested_source(depth, Size::new(16, 16), false);
        let fmt = photocraft_color::PixelFormat::new(photocraft_doc::ColorMode::Rgb, depth, true);
        let first = smart_cmds::source_image("nested.pcraft", &bytes, fmt).unwrap();
        for _ in 0..4 {
            let next = smart_cmds::source_image("nested.pcraft", &bytes, fmt).unwrap();
            assert!(std::sync::Arc::ptr_eq(&first.surface, &next.surface));
        }
        let mut session = nested_session(&bytes, depth);
        for (id, params) in [
            ("layer.smartFilter.setVisible", json!({"index":0,"visible":false})),
            ("layer.smartFilter.setVisible", json!({"index":0,"visible":true})),
            ("layer.smartFilter.setParams", json!({"index":0,"params":{"radius":2.}})),
            ("layer.translate", json!({"dx":1,"dy":1})),
            ("edit.undo", json!({})),
        ] {
            session.execute(id, params).unwrap();
            assert_eq!(smart_cmds::source_decode_count(&bytes, fmt), 1, "{depth:?}: {id} must reuse its nested source");
        }
        session.authorize = Some(deny_nested_blur);
        let before = session.active().unwrap().doc.clone();
        let error = session.execute("layer.smartFilter.setVisible", json!({"index":0,"visible":false})).unwrap_err();
        assert!(error.to_string().contains("denied original nested blur parameters"));
        assert_eq!(session.active().unwrap().doc, before, "a warm stage cannot bypass changed authority");
        assert_eq!(smart_cmds::source_decode_count(&bytes, fmt), 1, "authorization replay must not decode");
    }
}

#[test]
fn warm_auto_sources_recheck_both_original_actions_and_generated_commands() {
    use photocraft_doc::SampleType;
    use photocraft_geom::Size;
    let _turn = TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    empty_caches();
    let bytes = nested_source(SampleType::U8, Size::new(16, 16), true);
    let mut session = nested_session(&bytes, SampleType::U8);
    let fmt = session.active().unwrap().doc.pixel_format();
    assert_eq!(smart_cmds::source_decode_count(&bytes, fmt), 1);
    for gate in [deny_original_auto, deny_generated_adjustment] {
        session.authorize = Some(gate);
        let before = session.active().unwrap().doc.clone();
        assert!(session.execute("layer.smartFilter.setVisible", json!({"index":0,"visible":false})).is_err());
        assert_eq!(session.active().unwrap().doc, before);
        assert_eq!(smart_cmds::source_decode_count(&bytes, fmt), 1);
    }
    session.authorize = None;
    session.execute("layer.smartFilter.setVisible", json!({"index":0,"visible":false})).unwrap();
    assert_eq!(smart_cmds::source_decode_count(&bytes, fmt), 1);
}

#[test]
fn conversion_seed_cannot_hide_an_original_pending_auto_action() {
    use photocraft_doc::{BlendMode, ColorMode, Document, Layer, SampleType, SmartFilter, SmartObject, SmartSource};
    use photocraft_geom::{Affine, Size};
    let _turn = TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    empty_caches();
    let mut leaf = Document::new("Original auto input", Size::new(8, 8), ColorMode::Rgb, SampleType::U8);
    let mut pixels = Layer::raster("pixels", leaf.pixel_format());
    pixels.surface_mut().unwrap().fill_rect(leaf.bounds(), &[0.3, 0.5, 0.7, 1.]);
    let cached = pixels.surface().unwrap().clone();
    leaf.layers.push(pixels);
    let mut object = SmartObject::new(
        SmartSource::Embedded { file_name: "leaf.pcraft".into(), bytes: smart_cmds::encode_source(&leaf).unwrap().into() },
        Affine::IDENTITY,
        Some(cached),
    );
    object.smart_filters.push(SmartFilter { command: "image.autoContrast".into(), params: json!({}), blend: BlendMode::Normal, opacity: 1., visible: true });
    let mut doc = Document::new("Pending raw action with cached pixels", leaf.size, leaf.mode, leaf.depth);
    doc.layers.push(Layer::group("pending group", vec![Layer::new("pending action", LayerContent::Smart(object))]));
    let mut session = Session::new();
    session.add_document(doc, None);
    session.execute("filter.convertForSmartFilters", json!({})).unwrap();
    let state = session.active().unwrap();
    let LayerContent::Smart(object) = &state.doc.layers[0].content else { panic!("converted Smart") };
    let SmartSource::Embedded { bytes, .. } = &object.source else { panic!("embedded conversion") };
    let bytes = bytes.clone();
    let fmt = state.doc.pixel_format();
    assert_eq!(smart_cmds::source_decode_count(&bytes, fmt), 0);
    session.authorize = Some(deny_original_auto);
    let before = session.active().unwrap().doc.clone();
    let error = session.execute("filter.blur.gaussianBlur", json!({"radius":2.})).unwrap_err();
    assert!(error.to_string().contains("denied original auto action"));
    assert_eq!(session.active().unwrap().doc, before);
    session.authorize = None;
    session.execute("filter.blur.gaussianBlur", json!({"radius":2.})).unwrap();
    session.execute("layer.smartFilter.setVisible", json!({"index":0,"visible":false})).unwrap();
    assert_eq!(smart_cmds::source_decode_count(&bytes, fmt), 1);
    session.authorize = Some(deny_original_auto);
    assert!(session.execute("layer.smartFilter.setVisible", json!({"index":0,"visible":true})).is_err());
    assert_eq!(smart_cmds::source_decode_count(&bytes, fmt), 1);
}

#[test]
#[ignore = "manual 24 MP nested-source release timing; run with --release --ignored --nocapture"]
fn nested_source_24mp_release_timing() {
    use photocraft_color::PixelFormat;
    use photocraft_doc::{ColorMode, SampleType};
    use photocraft_geom::Size;
    use std::time::Instant;
    let _turn = TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    empty_caches();
    let bytes = nested_source(SampleType::U8, Size::new(6000, 4000), false);
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::U8, true);
    let start = Instant::now();
    let first = smart_cmds::source_image("nested.pcraft", &bytes, fmt).unwrap();
    let cold = start.elapsed();
    let start = Instant::now();
    // The work removed from a warm hit: decode and prepare the nested source again.
    let decoded = smart_cmds::decode_source("nested.pcraft", &bytes).unwrap();
    let revalidation = start.elapsed();
    assert_eq!(decoded.size, Size::new(6000, 4000));
    drop(decoded);
    let mut warm = Vec::new();
    for _ in 0..10 {
        let start = Instant::now();
        let next = smart_cmds::source_image("nested.pcraft", &bytes, fmt).unwrap();
        warm.push(start.elapsed());
        assert!(std::sync::Arc::ptr_eq(&first.surface, &next.surface));
    }
    warm.sort();
    assert_eq!(smart_cmds::source_decode_count(&bytes, fmt), 1);
    println!(
        "24 MP nested native source: cold={cold:?}; decode/preparation reference={revalidation:?}; warm median={:?}; warm max={:?}; bytes={}; decodes=1",
        warm[5],
        warm[9],
        bytes.len()
    );
}
