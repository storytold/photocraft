use super::*;
use std::io::Write;

const SVG: &[u8] = br#"<svg xmlns='http://www.w3.org/2000/svg' width='8' height='8'><rect width='8' height='8' fill='#808080'/><rect x='0.25' width='0.5' height='8' fill='red'/></svg>"#;

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

fn session(depth: u64) -> Session {
    let mut session = Session::new();
    session.execute("file.new", json!({"width": 32, "height": 32, "depth": depth})).unwrap();
    Arc::make_mut(session.active_mut().unwrap().doc_mut()).icc_profile = Some(craft_cms::Builtin::LinearSrgb.profile().to_bytes());
    session
}

fn smart(session: &Session, result: &Value) -> photocraft_doc::SmartObject {
    let layer = session.active().unwrap().doc.layer(LayerId(result["layer"].as_u64().unwrap())).unwrap();
    let LayerContent::Smart(smart) = &layer.content else { panic!("placed vector smart object") };
    smart.clone()
}

#[test]
fn compressed_byte_candidates_defer_to_the_importer_without_probing_during_placement_setup() {
    let session = session(32);
    for bytes in [vec![0x1f, 0x8b], b"BZh".to_vec(), b"\xfd7zXZ\0".to_vec()] {
        assert!(!photocraft_io::is_svg_input("", &bytes), "the malformed source is not positively identified as SVG");
        let input = PlaceInput::new(&session, "", bytes, None, &json!({})).unwrap();
        assert!(input.doc.is_none(), "compressed inputs use ordinary import and raster placement");
    }
    let input = PlaceInput::new(&session, "raster.png", b"\x89PNG\r\n\x1a\n".to_vec(), None, &json!({})).unwrap();
    assert!(input.doc.is_none(), "ordinary raster placement keeps its existing inexpensive path");
}

#[test]
fn renamed_and_nameless_svgz_placement_uses_raster_pixels_and_preserves_original_bytes() {
    let compressed = gzip(SVG);
    let params = json!({"smartObject": true, "fit": false, "scale": 200, "center": [8, 8]});
    for depth in [8, 16, 32] {
        let mut reference = session(depth);
        let plain = super::place_bytes(&mut reference, "reference.svg", SVG.to_vec(), None, &params).unwrap();
        let vector = smart(&reference, &plain);
        let encoded = super::place_bytes(&mut reference, "reference.svgz", compressed.clone(), None, &params).unwrap();
        let expected = smart(&reference, &encoded);
        assert_ne!(vector.cache, expected.cache, "SVGZ uses the raster resampling path");
        for name in ["logo.gz", "download.bin", ""] {
            let mut actual = session(depth);
            let before = actual.active().unwrap().doc.clone();
            let history = actual.active().unwrap().history.past_len();
            let placed = super::place_bytes(&mut actual, name, compressed.clone(), None, &params).unwrap();
            let placed_smart = smart(&actual, &placed);
            assert_eq!(placed_smart.transform, expected.transform);
            assert_eq!(placed_smart.cache, expected.cache, "ordinary SVGZ cache parity at {depth}-bit for {name}");
            assert!(matches!(&placed_smart.source, photocraft_doc::SmartSource::Embedded { bytes, .. } if **bytes == compressed));
            assert_eq!(actual.active().unwrap().history.past_len(), history + 1);
            let saved = photocraft_format::save_to_bytes(&actual.active().unwrap().doc, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
            let LayerContent::Smart(restored) = &reopened.layer(LayerId(placed["layer"].as_u64().unwrap())).unwrap().content else {
                panic!("persisted vector source")
            };
            assert!(matches!(&restored.source, photocraft_doc::SmartSource::Embedded { bytes, .. } if **bytes == compressed));
            assert_eq!(restored.transform, placed_smart.transform);
            assert_eq!(restored.cache, placed_smart.cache);
            actual.execute("edit.undo", json!({})).unwrap();
            assert_eq!(actual.active().unwrap().doc, before);
        }
    }
}

#[test]
fn scaled_linked_svgz_refresh_keeps_the_initial_cache_after_native_reopen() {
    let folder = tempfile::tempdir().unwrap();
    let source = folder.path().join("art.svgz");
    std::fs::write(&source, gzip(SVG)).unwrap();
    let mut s = session(32);
    let placed = s.execute("file.placeLinked", json!({"path":source,"scale":200,"fit":false})).unwrap();
    let original = smart(&s, &placed).cache.unwrap();
    s.execute("layer.smartObjects.updateModifiedContent", json!({"layer":placed["layer"]})).unwrap();
    assert_eq!(smart(&s, &placed).cache.as_ref(), Some(&original));
    let saved = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap();
    let document = photocraft_format::load_from_bytes(&saved).unwrap();
    let mut reopened = Session::new();
    reopened.add_document(document, None);
    assert_eq!(smart(&reopened, &placed).cache.as_ref(), Some(&original));
    reopened.execute("layer.smartObjects.updateModifiedContent", json!({"layer":placed["layer"]})).unwrap();
    assert_eq!(smart(&reopened, &placed).cache.as_ref(), Some(&original));
}

#[test]
fn ordinary_gzip_archives_and_explicit_affinity_names_fail_without_placing() {
    for (name, bytes) in [("notes.gz", gzip(b"not an SVG or XCF")), ("drawing.af", gzip(SVG))] {
        let mut session = session(8);
        let before = session.active().unwrap().doc.clone();
        let history = session.active().unwrap().history.past_len();
        assert!(super::place_bytes(&mut session, name, bytes, None, &json!({})).is_err());
        assert!(Arc::ptr_eq(&session.active().unwrap().doc, &before));
        assert_eq!(session.active().unwrap().history.past_len(), history);
    }
}

#[test]
fn a_warm_svgz_cache_does_not_admit_the_same_bytes_under_a_declared_xcf_alias() {
    let bytes = gzip(SVG);
    let pixels = PixelFormat::RGBA8;
    let source = crate::smart_cmds::source_image("art.svgz", &bytes, pixels).unwrap();
    assert_eq!(source.bounds.width(), 8);
    assert!(crate::smart_cmds::source_image("art.xcf.gz", &bytes, pixels).is_err());
    let stack = photocraft_doc::StackMode::Mean;
    assert!(crate::smart_cmds::stack_image("art.svgz", &bytes, pixels, stack).is_ok());
    assert!(crate::smart_cmds::stack_image("art.xcf.gz", &bytes, pixels, stack).is_err());
    let session = session(8);
    let doc = &session.active().unwrap().doc;
    let bytes = Arc::new(bytes);
    let mut smart = photocraft_doc::SmartObject::new(
        photocraft_doc::SmartSource::Embedded { file_name: "art.svgz".into(), bytes: Arc::clone(&bytes) },
        photocraft_geom::Affine::IDENTITY,
        None,
    );
    assert!(crate::smart_cmds::render(doc, &smart).unwrap().is_some());
    smart.source = photocraft_doc::SmartSource::Embedded { file_name: "art.xcf.gz".into(), bytes };
    assert!(crate::smart_cmds::render(doc, &smart).is_err());
}
