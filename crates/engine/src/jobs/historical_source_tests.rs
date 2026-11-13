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

fn complete_open(session: &mut Session, started: Result<Started>) -> Result<Value> {
    match started? {
        Started::Done(value) => Ok(value),
        Started::Job(job) => session.wait_job(job),
    }
}

#[test]
fn interactive_wrapped_bytes_ask_before_full_inflate_then_resume_validates_stream() {
    for cooperative in [false, true] {
        for corrupt in [false, true] {
            let bytes = wrapped(corrupt);
            let mut session = Session::new();
            session.set_cooperative_jobs(cooperative);
            session.color.settings.ask_on_missing = false;
            session.color.settings.ask_on_mismatch = false;
            let asynchronous = !cooperative && !cfg!(target_arch = "wasm32");
            let started = session.start_open_for_user_with_background("wrapped.xcf.gz", OpenSource::Bytes(bytes.clone()), false).unwrap();
            assert_eq!(matches!(&started, Started::Job(_)), asynchronous, "native compressed probing must use the worker even when background was false");
            let question = complete_open(&mut session, Ok(started)).unwrap();
            let id = question["heldHistoricalXcfOpen"].as_u64().unwrap();
            assert!(session.holds_historical_xcf_open(id));
            assert_eq!(Arc::strong_count(&bytes), 2, "hold the original encoded buffer without copying");
            assert!(session.documents().is_empty());
            let resumed = session.resume_historical_xcf_open(id, photocraft_io::HistoricalXcfSampleOrder::BigEndian, false);
            if asynchronous {
                assert!(matches!(&resumed, Ok(Started::Job(_))), "the accepted continuation owns its worker lifecycle: {resumed:?}");
                assert!(!session.holds_historical_xcf_open(id));
                assert!(session.documents().is_empty(), "a worker cannot admit a tab before its result is collected");
            }
            let result = complete_open(&mut session, resumed);
            if corrupt {
                assert!(result.is_err(), "the retained stream is fully validated only after the choice");
                assert!(session.documents().is_empty());
                if asynchronous {
                    assert!(!session.holds_historical_xcf_open(id), "accepted jobs consume their source even when decoding fails");
                    assert!(!session.cancel_historical_xcf_open(id));
                } else {
                    assert!(session.holds_historical_xcf_open(id), "inline startup refusal leaves its exact source retryable");
                    assert!(session.cancel_historical_xcf_open(id));
                }
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
    let resumed = session.resume_historical_xcf_open(id, photocraft_io::HistoricalXcfSampleOrder::BigEndian, false).unwrap();
    assert!(matches!(&resumed, Started::Job(_)), "retained compressed bytes resume on a native worker");
    assert!(!session.holds_historical_xcf_open(id));
    assert!(session.documents().is_empty());
    complete_open(&mut session, Ok(resumed)).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"replacement is not an XCF", "resuming the exact retained source must not touch its changed locator");
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
    let resumed = session.resume_historical_xcf_open(id, photocraft_io::HistoricalXcfSampleOrder::BigEndian, false).unwrap();
    assert!(matches!(&resumed, Started::Job(_)), "CRC validation belongs to the accepted native worker");
    assert!(!session.holds_historical_xcf_open(id));
    assert!(complete_open(&mut session, Ok(resumed)).is_err(), "the corrupt retained CRC must fail after the explicit choice");
    assert!(session.documents().is_empty());
    assert!(!session.cancel_historical_xcf_open(id), "the failed accepted job has already released its encoded source");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn verified_path_question_retains_exact_source_and_explicit_order_needs_no_capture() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.xcf.gz");
    let bytes = wrapped(false);
    std::fs::write(&path, bytes.as_slice()).unwrap();
    for order in [None, Some(photocraft_io::HistoricalXcfSampleOrder::BigEndian)] {
        let options = photocraft_io::ImportOptions { historical_xcf_sample_order: order, ..Default::default() };
        let mut identity = None;
        let mut retained = None;
        let result =
            import_path("source.xcf.gz", path.to_str().unwrap(), &options, None, &photocraft_raster::Interrupt::NONE, &mut identity, Some(&mut retained));
        assert_eq!(identity, Some(photocraft_io::FileIdentity::of_path(&path).unwrap()));
        if order.is_none() {
            assert!(matches!(result, Err(photocraft_io::IoError::HistoricalXcfSampleOrderRequired(_))));
            assert_eq!(retained.unwrap().as_slice(), bytes.as_slice(), "the question must carry the exact verified encoded source");
        } else {
            assert!(result.is_ok());
            assert!(retained.is_none(), "ordinary explicit-order imports do not retain another encoded buffer");
        }
    }
}
