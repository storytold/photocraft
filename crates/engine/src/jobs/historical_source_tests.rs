#![allow(clippy::unwrap_used)]

use super::historical_xcf_fixture as fixture;
use super::*;
use std::io::Write;

fn wrapped(corrupt_tail: bool) -> Arc<Vec<u8>> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&fixture::historical_pixel(false)).unwrap();
    let mut bytes = encoder.finish().unwrap();
    if corrupt_tail {
        let crc = bytes.len() - 8;
        bytes[crc] ^= 1;
    }
    Arc::new(bytes)
}

#[test]
fn interactive_wrapped_bytes_ask_before_full_inflate_then_resume_validates_stream() {
    for corrupt in [false, true] {
        let bytes = wrapped(corrupt);
        let mut session = Session::new();
        session.color.settings.ask_on_missing = false;
        session.color.settings.ask_on_mismatch = false;
        let Started::Done(question) = session.start_open_for_user_with_background("wrapped.xcf.gz", OpenSource::Bytes(bytes.clone()), false).unwrap() else {
            panic!("inline question")
        };
        let id = question["heldHistoricalXcfOpen"].as_u64().unwrap();
        assert!(session.holds_historical_xcf_open(id));
        assert_eq!(Arc::strong_count(&bytes), 2, "hold the original encoded buffer without copying");
        assert!(session.documents().is_empty());
        let result = session.resume_historical_xcf_open(id, photocraft_io::HistoricalXcfSampleOrder::BigEndian, false);
        if corrupt {
            assert!(result.is_err(), "the retained stream is fully validated only after the choice");
            assert!(session.holds_historical_xcf_open(id));
            assert!(session.cancel_historical_xcf_open(id));
        } else {
            result.unwrap();
            assert!(!session.holds_historical_xcf_open(id));
            let doc = &session.active().unwrap().doc;
            assert_eq!(
                doc.layers[0].surface().unwrap().read_region(doc.bounds()),
                vec![0x1234 as f32 / 65535., 0x4567 as f32 / 65535., 0x89ab as f32 / 65535., 1.]
            );
        }
        assert_eq!(Arc::strong_count(&bytes), 1);
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn interactive_wrapped_path_retains_verified_bytes_and_resume_never_reopens_path() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.xcf.gz");
    std::fs::write(&path, wrapped(false).as_slice()).unwrap();
    let mut session = Session::new();
    session.color.settings.ask_on_missing = false;
    session.color.settings.ask_on_mismatch = false;
    let Started::Done(question) = session.start_open_for_user_with_background("source.xcf.gz", OpenSource::Path(path.to_str().unwrap().into()), false).unwrap()
    else {
        panic!("inline question")
    };
    let id = question["heldHistoricalXcfOpen"].as_u64().unwrap();
    // A later authorized change to the locator cannot replace the exact already-read source.
    std::fs::write(&path, b"replacement is not an XCF").unwrap();
    session.resume_historical_xcf_open(id, photocraft_io::HistoricalXcfSampleOrder::BigEndian, false).unwrap();
    let doc = &session.active().unwrap().doc;
    assert_eq!(doc.layers[0].surface().unwrap().read_region(doc.bounds()), vec![0x1234 as f32 / 65535., 0x4567 as f32 / 65535., 0x89ab as f32 / 65535., 1.]);
    assert!(!session.holds_historical_xcf_open(id));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn interactive_wrapped_path_asks_before_corrupt_tail_is_inflated() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("corrupt.xcf.gz");
    std::fs::write(&path, wrapped(true).as_slice()).unwrap();
    let mut session = Session::new();
    let Started::Done(question) =
        session.start_open_for_user_with_background("corrupt.xcf.gz", OpenSource::Path(path.to_str().unwrap().into()), false).unwrap()
    else {
        panic!("inline question")
    };
    let id = question["heldHistoricalXcfOpen"].as_u64().unwrap();
    assert!(session.resume_historical_xcf_open(id, photocraft_io::HistoricalXcfSampleOrder::BigEndian, false).is_err());
    assert!(session.documents().is_empty());
    assert!(session.cancel_historical_xcf_open(id));
}
