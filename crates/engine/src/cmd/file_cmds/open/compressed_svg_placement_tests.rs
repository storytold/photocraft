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
    Arc::make_mut(&mut session.active_mut().unwrap().doc).icc_profile = Some(craft_cms::Builtin::LinearSrgb.profile().to_bytes());
    session
}

fn smart(session: &Session, result: &Value) -> photocraft_doc::SmartObject {
    let layer = session.active().unwrap().doc.layer(LayerId(result["layer"].as_u64().unwrap())).unwrap();
    let LayerContent::Smart(smart) = &layer.content else { panic!("placed vector smart object") };
    smart.clone()
}

#[test]
fn compressed_byte_candidates_capture_the_destination_without_decoding_the_header() {
    let session = session(32);
    for bytes in [vec![0x1f, 0x8b], b"BZh".to_vec(), b"\xfd7zXZ\0".to_vec()] {
        assert!(!photocraft_io::is_svg_input("", &bytes), "the malformed source is not positively identified as SVG");
        let input = PlaceInput::new(&session, "", bytes, None, &json!({})).unwrap();
        assert!(Arc::ptr_eq(input.doc.as_ref().unwrap(), &session.active().unwrap().doc));
    }
    let input = PlaceInput::new(&session, "raster.png", b"\x89PNG\r\n\x1a\n".to_vec(), None, &json!({})).unwrap();
    assert!(input.doc.is_none(), "ordinary raster placement keeps its existing inexpensive path");
}

#[test]
fn renamed_and_nameless_gzip_placement_keeps_sharp_vector_profile_pixels_and_original_bytes() {
    let compressed = gzip(SVG);
    let params = json!({"smartObject": true, "fit": false, "scale": 200, "center": [8, 8]});
    for depth in [8, 16, 32] {
        let mut reference = session(depth);
        let plain = super::place_bytes(&mut reference, "reference.svg", SVG.to_vec(), None, &params).unwrap();
        let expected = smart(&reference, &plain);
        for name in ["logo.gz", "download.bin", ""] {
            let mut actual = session(depth);
            let before = actual.active().unwrap().doc.clone();
            let history = actual.active().unwrap().history.past_len();
            let placed = super::place_bytes(&mut actual, name, compressed.clone(), None, &params).unwrap();
            let placed_smart = smart(&actual, &placed);
            assert_eq!(placed_smart.transform, expected.transform);
            assert_eq!(placed_smart.cache, expected.cache, "full vector/profile cache parity at {depth}-bit for {name}");
            assert!((placed_smart.cache.as_ref().unwrap().pixel(10, 6)[0] - 0.216).abs() < 0.01, "destination linear profile remains effective");
            assert!(matches!(&placed_smart.source, photocraft_doc::SmartSource::Embedded { bytes, .. } if **bytes == compressed));
            assert_eq!(actual.active().unwrap().history.past_len(), history + 1);
            let saved = photocraft_format::save_to_bytes(&actual.active().unwrap().doc, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
            let LayerContent::Smart(restored) = &reopened.layer(LayerId(placed["layer"].as_u64().unwrap())).unwrap().content else {
                panic!("persisted vector source")
            };
            assert!(matches!(&restored.source, photocraft_doc::SmartSource::Embedded { bytes, .. } if **bytes == compressed));
            assert_eq!(restored.transform, placed_smart.transform);
            actual.execute("edit.undo", json!({})).unwrap();
            assert_eq!(actual.active().unwrap().doc, before);
        }
    }
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
