//! Rasterizing an unavailable Smart Object must preserve its source representation (#4955).

use crate::Session;
use photocraft_doc::{LayerContent, SmartObject, SmartSource};
use photocraft_geom::Affine;
use serde_json::json;
use std::sync::Arc;

const COMMANDS: [&str; 3] = ["layer.rasterize.smartObject", "layer.rasterize.layer", "layer.smartObjects.rasterize"];

fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "depth": depth})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    s
}

fn missing_source_refuses_rasterization(embedded: bool) {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.png");
    assert!(!missing.exists());
    for depth in [8, 16, 32] {
        for command in COMMANDS {
            let mut s = session(depth);
            let st = s.active_mut().unwrap();
            let id = st.active_layer.unwrap();
            let layer = Arc::make_mut(&mut st.doc).layer_mut(id).unwrap();
            let source = if embedded {
                SmartSource::Embedded { file_name: "broken.png".into(), bytes: Arc::new(b"not a PNG".to_vec()) }
            } else {
                SmartSource::Linked { path: missing.to_string_lossy().into_owned() }
            };
            let mut smart = SmartObject::new(source, Affine::IDENTITY, None);
            smart.psd_raw = Some(Arc::new(b"synthetic placed data".to_vec()));
            layer.content = LayerContent::Smart(smart);
            layer.psd_blocks = vec![(*b"SoLd", Arc::new(vec![1, 2])), (*b"SoLE", Arc::new(vec![3, 4])), (*b"PlLd", Arc::new(vec![5, 6]))];
            let before = st.doc.clone();
            let before_bytes = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
            let history = (st.history.past_len(), st.history.entries());
            let target = (st.active_layer, st.selected_layers.clone(), st.layer_anchor);
            let revision = st.revision;
            let journal = s.journal.clone();
            let result = s.execute(command, json!({"layer": id.0}));
            let st = s.active().unwrap();
            eprintln!(
                "unavailable smart/{embedded}/{depth}/{command}: result={result:?}, kind={}, undo_delta={}",
                st.doc.layer(id).unwrap().content.kind_name(),
                st.history.past_len() - history.0
            );
            assert!(result.is_err(), "unavailable uncached content must refuse rasterization: {result:?}");
            let error = result.unwrap_err().to_string();
            assert!(error.contains(if embedded { "can't read smart object contents" } else { "unavailable" }), "actionable source error: {error}");
            assert!(Arc::ptr_eq(&st.doc, &before), "the failed command keeps the exact original document snapshot");
            assert_eq!(st.doc, before);
            assert_eq!((st.history.past_len(), st.history.entries()), history);
            assert_eq!((st.active_layer, st.selected_layers.clone(), st.layer_anchor), target);
            assert_eq!(st.revision, revision);
            assert_eq!(s.journal, journal);
            assert_eq!(
                photocraft_format::save_to_bytes(&st.doc, &Default::default()).unwrap(),
                before_bytes,
                "saving after refusal preserves the source and PSD representation"
            );
        }
    }
}

#[test]
fn missing_linked_uncached_smart_object_is_not_rasterized() {
    missing_source_refuses_rasterization(false);
}

#[test]
fn invalid_embedded_uncached_smart_object_is_not_rasterized() {
    missing_source_refuses_rasterization(true);
}

#[test]
fn available_cached_pixels_can_rasterize_without_the_linked_source() {
    let dir = tempfile::tempdir().unwrap();
    for depth in [8, 16, 32] {
        for command in COMMANDS {
            let mut s = session(depth);
            let st = s.active_mut().unwrap();
            let id = st.active_layer.unwrap();
            let layer = Arc::make_mut(&mut st.doc).layer_mut(id).unwrap();
            let LayerContent::Raster(pixels) = &layer.content else { panic!("red fill fixture must be a pixel layer") };
            let cache = pixels.clone();
            let expected = cache.rgba(2, 2);
            layer.content = LayerContent::Smart(SmartObject::new(
                SmartSource::Linked { path: dir.path().join("missing.png").to_string_lossy().into_owned() },
                Affine::IDENTITY,
                Some(cache),
            ));
            layer.psd_blocks = vec![(*b"SoLd", Arc::new(vec![1, 2])), (*b"test", Arc::new(vec![3, 4]))];
            let before = st.doc.clone();
            let past = st.history.past_len();
            s.execute(command, json!({"layer": id.0})).unwrap();
            let st = s.active().unwrap();
            let layer = st.doc.layer(id).unwrap();
            let LayerContent::Raster(pixels) = &layer.content else { panic!("cache should become raster pixels") };
            assert_eq!(pixels.rgba(2, 2), expected);
            assert_eq!(pixels.format(), before.pixel_format());
            assert_eq!(layer.psd_blocks, vec![(*b"test", Arc::new(vec![3, 4]))]);
            assert_eq!(st.history.past_len(), past + 1);
            assert!(s.undo());
            assert_eq!(s.active().unwrap().doc, before);
            assert!(s.redo());
            assert!(matches!(s.active().unwrap().doc.layer(id).unwrap().content, LayerContent::Raster(_)));
        }
    }
}

#[test]
fn available_embedded_source_refreshes_before_rasterization() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        let st = s.active_mut().unwrap();
        let id = st.active_layer.unwrap();
        let layer = Arc::make_mut(&mut st.doc).layer_mut(id).unwrap();
        let LayerContent::Smart(smart) = &mut layer.content else { panic!("conversion must create a Smart Object") };
        let expected = smart.cache.as_ref().unwrap().rgba(2, 2);
        smart.cache = None;
        let before = st.doc.clone();
        s.execute("layer.rasterize.smartObject", json!({"layer": id.0})).unwrap();
        let LayerContent::Raster(pixels) = &s.active().unwrap().doc.layer(id).unwrap().content else { panic!("valid contents should refresh and rasterize") };
        assert_eq!(pixels.rgba(2, 2), expected);
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc, before);
    }
}
