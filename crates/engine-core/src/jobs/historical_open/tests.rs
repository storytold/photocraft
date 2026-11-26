use super::*;
use crate::color_cmds::ColorExt;
use crate::jobs::{JobCtx, PendingPsdPreflight, historical_fixture::historical_rgba16};
use std::sync::Arc;

fn session() -> Session {
    let mut session = crate::session();
    session.color_mut().settings.ask_on_missing = false;
    session.color_mut().settings.ask_on_mismatch = false;
    session
}

fn finish(session: &mut Session, started: Started) -> Value {
    match started {
        Started::Done(value) => value,
        Started::Job(id) => session.wait_job(id).unwrap(),
    }
}

fn hold(session: &mut Session, bytes: Arc<Vec<u8>>, limit: u64) -> Result<u64> {
    let settings = Settings { options: ImportOptions::default(), as_ext: None, origin: None };
    let reply = session.hold_historical_open(
        "held.xcf".into(),
        OpenSource::Bytes(bytes),
        HistoricalXcfSampleOrderRequest { version: 10, bits_per_sample: 16 },
        settings,
        limit,
    )?;
    Ok(reply["heldHistoricalXcfOpen"].as_u64().unwrap())
}

fn preflight(bytes: Arc<Vec<u8>>) -> PendingPsdPreflight {
    let source = OpenSource::Bytes(bytes);
    PendingPsdPreflight {
        id: 0,
        name: "pending.psd".into(),
        fingerprint: SourceFingerprint::of(&source).unwrap(),
        source,
        required: 20,
        soft_limit: 10,
        hard_limit: 40,
        available_ram: None,
        confirm_damage: true,
    }
}

#[test]
fn historical_and_psd_decisions_share_capacity_count_and_keep_other_held_payloads() {
    let mut session = session();
    let bytes = Arc::new(vec![0; 16]);
    let psd = session.store_pending_preflight_with_limit(preflight(bytes.clone()), 16).unwrap();
    let color = session.hold_open(
        photocraft_doc::Document::new("color", photocraft_doc::Size::new(1, 1), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8),
        false,
    );
    let historical = hold(&mut session, bytes.clone(), 16).unwrap();
    assert!(hold(&mut session, Arc::new(vec![0; 1]), 16).is_err());
    assert!(session.cancel_historical_xcf_open(historical));
    assert!(!session.cancel_historical_xcf_open(historical));
    assert!(session.take_pending_preflight(psd).is_some());
    assert!(session.take_held_open(color).is_some());
    assert!(session.historical_xcf_open_request(historical).is_none());
    for _ in 0..Session::MAX_PENDING_PREFLIGHTS - 1 {
        session.store_pending_preflight_with_limit(preflight(bytes.clone()), 16).unwrap();
    }
    let historical = hold(&mut session, bytes.clone(), 16).unwrap();
    assert!(session.store_pending_preflight_with_limit(preflight(bytes.clone()), 16).is_err());
    assert!(hold(&mut session, bytes.clone(), 16).is_err());
    assert!(session.cancel_historical_xcf_open(historical));
    assert!(session.store_pending_preflight_with_limit(preflight(bytes), 16).is_ok());
}

#[test]
fn pending_preflights_charge_the_retained_allocation_not_the_visible_length() {
    let mut session = session();
    let mut source = Vec::with_capacity(128);
    source.push(7);
    source.clear();
    let capacity = source.capacity() as u64;
    let source = Arc::new(source);
    assert!(hold(&mut session, source.clone(), capacity - 1).is_err());
    assert!(session.store_pending_preflight_with_limit(preflight(source.clone()), capacity - 1).is_err());
    assert_eq!(session.pending_preflight_bytes().unwrap(), 0);
    let historical = hold(&mut session, source.clone(), capacity).unwrap();
    let psd = session.store_pending_preflight_with_limit(preflight(source), capacity).unwrap();
    assert_eq!(session.pending_preflight_bytes().unwrap(), capacity);
    assert!(session.take_pending_preflight(psd).is_some());
    assert_eq!(session.pending_preflight_bytes().unwrap(), 0);
    assert!(hold(&mut session, Arc::new(vec![0; 1]), capacity).is_err());
    assert!(session.cancel_historical_xcf_open(historical));
    assert!(hold(&mut session, Arc::new(vec![0; 1]), capacity).is_ok());
}

#[test]
fn opt_in_open_retains_exact_options_and_failed_startup_keeps_request_retryable() {
    let mut session = session();
    let source = Arc::new(historical_rgba16());
    let options = ImportOptions { max_group_depth: 3, pdf_dpi: 144., psd_approved_estimate: Some(999), ..Default::default() };
    let started = session.start_open_for_user_with_options("source.xcf", OpenSource::Bytes(source.clone()), &options, false).unwrap();
    let question = finish(&mut session, started);
    let id = question["heldHistoricalXcfOpen"].as_u64().unwrap();
    assert_eq!(session.historical_xcf_open_request(id), Some(HistoricalXcfSampleOrderRequest { version: 10, bits_per_sample: 16 }));
    let held = session.jobs.historical_opens.iter().find(|held| held.id == id).unwrap();
    assert!(matches!(&held.source, OpenSource::Bytes(bytes) if Arc::ptr_eq(bytes, &source)));
    let mut expected = options;
    expected.psd_approved_estimate = None;
    expected.historical_xcf_order = HistoricalXcfOrder::RequireExplicit;
    assert_eq!(held.settings.options, expected);
    assert!(session.documents().is_empty());
    let cancelled = JobCtx::new();
    cancelled.cancel();
    session.set_inline_job_ctx(Some(cancelled));
    assert!(matches!(session.resume_historical_xcf_open(id, HistoricalXcfSampleOrder::BigEndian, false), Err(EngineError::Cancelled)));
    assert!(session.holds_historical_xcf_open(id));
    assert!(session.documents().is_empty());
    session.set_inline_job_ctx(None);
    let started = session.resume_historical_xcf_open(id, HistoricalXcfSampleOrder::BigEndian, false).unwrap();
    let result = finish(&mut session, started);
    assert!(result.get("document").is_some());
    assert!(!session.holds_historical_xcf_open(id));
    assert!(session.resume_historical_xcf_open(id, HistoricalXcfSampleOrder::LittleEndian, false).is_err());
    assert_eq!(session.documents().len(), 1);
    let doc = &session.active().unwrap().doc;
    assert_eq!(doc.layers[0].surface().unwrap().read_region(doc.bounds())[0], f32::from(0x1234u16) / 65535.);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn simultaneous_open_jobs_keep_distinct_requests_when_completed_and_answered_out_of_order() {
    let mut session = session();
    let first = Arc::new(historical_rgba16());
    let mut little = historical_rgba16();
    let pixel = little.len() - 8;
    for word in little[pixel..].as_chunks_mut::<2>().0 {
        word.swap(0, 1);
    }
    let second = Arc::new(little);
    let a = session.start_open_for_user_with_options("a.xcf", OpenSource::Bytes(first.clone()), &ImportOptions::default(), true).unwrap();
    let b = session.start_open_for_user_with_options("b.xcf", OpenSource::Bytes(second.clone()), &ImportOptions::default(), true).unwrap();
    assert!(matches!(a, Started::Job(_)) && matches!(b, Started::Job(_)));
    let b = finish(&mut session, b)["heldHistoricalXcfOpen"].as_u64().unwrap();
    let a = finish(&mut session, a)["heldHistoricalXcfOpen"].as_u64().unwrap();
    assert_ne!(a, b);
    assert!(session.documents().is_empty());
    let started = session.resume_historical_xcf_open(b, HistoricalXcfSampleOrder::LittleEndian, true).unwrap();
    assert!(!session.holds_historical_xcf_open(b), "accepted workers own the request");
    assert!(session.holds_historical_xcf_open(a));
    finish(&mut session, started);
    assert!(session.cancel_historical_xcf_open(a));
    assert!(!session.cancel_historical_xcf_open(b));
    assert_eq!(session.documents().len(), 1);
    let doc = &session.active().unwrap().doc;
    assert_eq!(doc.name, "b.xcf");
    assert_eq!(doc.layers[0].surface().unwrap().read_region(doc.bounds())[0], f32::from(0x1234u16) / 65535.);
}

#[cfg(any(unix, windows))]
#[test]
fn forced_open_as_resumes_verified_wrapped_bytes_without_reopening_replaced_path() {
    use std::io::Write;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("unmarked.data");
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&historical_rgba16()).unwrap();
    let original = encoder.finish().unwrap();
    std::fs::write(&path, &original).unwrap();
    let origin = path.to_str().unwrap().to_owned();
    let mut session = session();
    let started =
        session.start_open_as_for_user_with_options("unmarked.data", OpenSource::Path(origin.clone()), "XCF.GZ", &ImportOptions::default(), false).unwrap();
    let id = finish(&mut session, started)["heldHistoricalXcfOpen"].as_u64().unwrap();
    let held = session.jobs.historical_opens.iter().find(|held| held.id == id).unwrap();
    assert_eq!(held.settings.as_ext.as_deref(), Some("xcf.gz"));
    assert!(matches!(&held.source, OpenSource::Bytes(bytes) if bytes.as_slice() == original));
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, b"replacement is not an image").unwrap();
    let started = session.resume_historical_xcf_open(id, HistoricalXcfSampleOrder::BigEndian, false).unwrap();
    finish(&mut session, started);
    let state = session.active().unwrap();
    assert!(state.source_read_only);
    assert_eq!(state.protected_import_source.as_deref(), Some(origin.as_str()));
    assert_eq!(state.doc.layers[0].surface().unwrap().read_region(state.doc.bounds())[0], f32::from(0x1234u16) / 65535.);
    assert_eq!(std::fs::read(&path).unwrap(), b"replacement is not an image");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn accepted_failed_continuation_consumes_only_its_request_without_admitting_pixels() {
    let mut session = session();
    let mut truncated = historical_rgba16();
    truncated.pop(); // Structural records still parse; the selected tile is short.
    let bad = session.start_open_for_user_with_options("bad.xcf", OpenSource::Bytes(Arc::new(truncated)), &ImportOptions::default(), true).unwrap();
    let bad = finish(&mut session, bad)["heldHistoricalXcfOpen"].as_u64().unwrap();
    let good = session.start_open_for_user_with_options("good.xcf", OpenSource::Bytes(Arc::new(historical_rgba16())), &ImportOptions::default(), true).unwrap();
    let good = finish(&mut session, good)["heldHistoricalXcfOpen"].as_u64().unwrap();
    let Started::Job(job) = session.resume_historical_xcf_open(bad, HistoricalXcfSampleOrder::BigEndian, true).unwrap() else {
        panic!("native continuation must own its background job");
    };
    assert!(!session.holds_historical_xcf_open(bad));
    assert!(session.wait_job(job).is_err());
    assert!(session.documents().is_empty());
    assert!(session.holds_historical_xcf_open(good));
    assert!(!session.cancel_historical_xcf_open(bad));
    assert!(session.cancel_historical_xcf_open(good));
}

#[test]
fn historical_answer_command_validates_before_consuming_and_is_not_recorded() {
    let mut session = session();
    let id = hold(&mut session, Arc::new(historical_rgba16()), Session::MAX_PENDING_PREFLIGHT_BYTES).unwrap();
    let before = session.journal.len();
    for params in [
        json!({}),
        json!({"id": true}),
        json!({"id": id}),
        json!({"id": id, "cancel": true, "historicalXcfSampleOrder": "big-endian"}),
        json!({"id": id, "historicalXcfSampleOrder": "native"}),
        json!({"id": id, "cancel": true, "background": "true"}),
    ] {
        assert!(matches!(session.execute("file.resolveHistoricalXcfOpen", params), Err(EngineError::BadParams { .. })));
        assert!(session.holds_historical_xcf_open(id));
    }
    let result = session.execute("file.resolveHistoricalXcfOpen", json!({"id": id, "cancel": true})).unwrap();
    assert_eq!(result["cancelled"], true);
    assert!(!session.holds_historical_xcf_open(id));
    assert_eq!(session.journal.len(), before);
}

#[test]
fn historical_answer_command_stale_cancel_never_consumes_another_request() {
    let mut session = session();
    let bytes = Arc::new(historical_rgba16());
    let first = hold(&mut session, bytes.clone(), Session::MAX_PENDING_PREFLIGHT_BYTES).unwrap();
    let second = hold(&mut session, bytes, Session::MAX_PENDING_PREFLIGHT_BYTES).unwrap();
    session.execute("file.resolveHistoricalXcfOpen", json!({"id": first, "cancel": true})).unwrap();
    let stale = session.execute("file.resolveHistoricalXcfOpen", json!({"id": first, "cancel": true})).unwrap();
    assert_eq!(stale["alreadyReleased"], true);
    assert!(session.holds_historical_xcf_open(second));
    assert!(session.execute("file.resolveHistoricalXcfOpen", json!({"id": first, "historicalXcfSampleOrder": "little-endian"})).is_err());
    assert!(session.holds_historical_xcf_open(second));
}
