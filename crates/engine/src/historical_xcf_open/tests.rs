use super::super::historical_xcf_fixture as fixture;
use super::*;

fn pixel(s: &Session) -> [f32; 4] {
    let doc = &s.active().unwrap().doc;
    let photocraft_doc::LayerContent::Raster(surface) = &doc.layers[0].content else { panic!("native raster") };
    let mut value = [0.; 4];
    surface.read_pixel(0, 0, &mut value);
    value
}

#[test]
fn explicit_orders_open_native_samples_and_unset_is_not_guessed() {
    for (little, order) in [(true, photocraft_io::HistoricalXcfSampleOrder::LittleEndian), (false, photocraft_io::HistoricalXcfSampleOrder::BigEndian)] {
        let bytes = Arc::new(fixture::historical_pixel(little));
        let mut s = Session::new();
        s.set_cooperative_jobs(true);
        let options = photocraft_io::ImportOptions { historical_xcf_sample_order: Some(order), ..super::super::import_options(s.prefs()) };
        assert!(matches!(s.start_open_with_options("renamed.bin", OpenSource::Bytes(bytes.clone()), &options).unwrap(), Started::Done(_)));
        assert_eq!(pixel(&s), [0x1234 as f32 / 65535., 0x4567 as f32 / 65535., 0x89ab as f32 / 65535., 1.]);
        assert!(s.active().unwrap().source_read_only);
        let mut unchosen = Session::new();
        unchosen.set_cooperative_jobs(true);
        assert!(unchosen.start_open_with_options("same.xcf", OpenSource::Bytes(bytes), &Default::default()).is_err());
        assert!(unchosen.documents().is_empty());
    }
}

#[test]
#[cfg(target_pointer_width = "64")]
fn full_import_options_preserve_the_restrictive_psd_approved_estimate() {
    let gb = 1u64 << 30;
    let bytes = crate::jobs::psd_open_fixture::make_psd(false);
    let snapshot = photocraft_io::memory::MemorySnapshot { available_ram: Some(8 * gb), swap_free: None, commit_headroom: None };
    photocraft_io::memory::with_mock_memory(snapshot, || {
        // Charge the real tiny PSD above the soft limit without allocating a threshold-sized image.
        photocraft_io::psd_budget::with_test_estimate_charge(4 * gb + 1, || {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":2,"height":2})).unwrap();
            let control = s.active().unwrap().doc.clone();
            let options = photocraft_io::ImportOptions { psd_high_memory_approved: true, psd_approved_estimate: Some(1 << 20), ..Default::default() };
            let error = crate::file_cmds::open_bytes_as_with_import_options(&mut s, "tiny.psd", &bytes, None, None, &options)
                .expect_err("approval for a smaller estimate cannot admit this file");
            assert!(matches!(error, EngineError::PsdApprovalRequired { required, .. } if required > 4 * gb));
            assert_eq!(s.documents().len(), 1);
            assert!(Arc::ptr_eq(&control, &s.active().unwrap().doc));
            // The legacy boolean wrapper still grants unbound approval, exactly as before.
            crate::file_cmds::open_bytes_as_with_options(&mut s, "tiny.psd", &bytes, None, None, true).unwrap();
            assert_eq!(s.documents().len(), 2);
        });
    });
}

#[test]
fn historical_choice_is_consumed_once_and_cancel_releases_exact_source() {
    let mut s = Session::new();
    let bytes = Arc::new(fixture::historical_pixel(true));
    let Started::Done(question) = s.start_open_for_user_with_background("old.xcf", OpenSource::Bytes(bytes.clone()), false).unwrap() else { panic!("inline") };
    let id = question["heldHistoricalXcfOpen"].as_u64().unwrap();
    assert!(s.documents().is_empty());
    assert!(Arc::strong_count(&bytes) > 1);
    assert!(s.cancel_historical_xcf_open(id));
    assert_eq!(Arc::strong_count(&bytes), 1);
    assert!(s.resume_historical_xcf_open(id, photocraft_io::HistoricalXcfSampleOrder::LittleEndian, false).is_err());
    let Started::Done(question) = s.start_open_for_user_with_background("old.xcf", OpenSource::Bytes(bytes), false).unwrap() else { panic!("inline") };
    let id = question["heldHistoricalXcfOpen"].as_u64().unwrap();
    s.resume_historical_xcf_open(id, photocraft_io::HistoricalXcfSampleOrder::LittleEndian, false).unwrap();
    assert!(!s.cancel_historical_xcf_open(id));
    assert_eq!(s.documents().len(), 1);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn cancelling_before_poll_does_not_publish_a_historical_source_capability() {
    let mut s = Session::new();
    let Started::Job(job) = s.start_open_for_user("old.xcf", OpenSource::Bytes(Arc::new(fixture::historical_pixel(true)))).unwrap() else { panic!("worker") };
    s.cancel_job(job);
    let events = s.poll_jobs();
    assert!(events.iter().any(|event| event.id == job && matches!(&event.outcome, crate::jobs::JobOutcome::Cancelled)));
    assert!(s.jobs.historical_xcf.is_empty());
    assert!(s.documents().is_empty());
}

#[test]
fn pending_source_count_is_bounded_without_discarding_earlier_choices() {
    let mut s = Session::new();
    let mut ids = Vec::new();
    for _ in 0..16 {
        let Started::Done(question) =
            s.start_open_for_user_with_background("old.xcf", OpenSource::Bytes(Arc::new(fixture::historical_pixel(true))), false).unwrap()
        else {
            panic!("inline")
        };
        ids.push(question["heldHistoricalXcfOpen"].as_u64().unwrap());
    }
    assert!(s.start_open_for_user_with_background("excess.xcf", OpenSource::Bytes(Arc::new(fixture::historical_pixel(true))), false).is_err());
    for id in ids {
        assert!(s.cancel_historical_xcf_open(id));
    }
    assert!(s.jobs.historical_xcf.is_empty());
}

#[test]
fn pending_sources_charge_retained_and_incoming_capacity_after_truncation() {
    let mut s = Session::new();
    let question = photocraft_io::HistoricalXcfSampleOrderRequest { version: 7, bits_per_sample: 16 };
    let mut buffer = Vec::with_capacity(128);
    buffer.extend([1u8, 2, 3, 4]);
    buffer.truncate(1);
    let first = Arc::new(buffer);
    let held = hold_with_budget(&mut s, "first.xcf".into(), first.clone(), question, Default::default(), 200).unwrap();
    let mut buffer = Vec::with_capacity(128);
    buffer.push(0);
    assert!(hold_with_budget(&mut s, "second.xcf".into(), Arc::new(buffer), question, Default::default(), 200).is_err());
    assert_eq!(s.jobs.historical_xcf.len(), 1, "the earlier source survives the refused second allocation");
    let mut buffer = Vec::with_capacity(512);
    buffer.push(0);
    assert!(hold_with_budget(&mut Session::new(), "large.xcf".into(), Arc::new(buffer), question, Default::default(), 200).is_err());
    assert!(s.cancel_historical_xcf_open(held["heldHistoricalXcfOpen"].as_u64().unwrap()));
    assert_eq!(Arc::strong_count(&first), 1);
}

#[test]
fn invalid_order_is_rejected_before_file_open_as_reads_or_mutates() {
    let mut s = Session::new();
    for value in [json!("native"), json!(true), json!(17)] {
        let error = s.execute("file.openAs", json!({"path":"does-not-exist.xcf","historicalXcfSampleOrder":value})).unwrap_err();
        assert!(error.to_string().contains("historicalXcfSampleOrder"));
        assert!(s.documents().is_empty());
        assert!(s.jobs().is_empty());
    }
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn file_open_as_forwards_both_explicit_orders_and_keeps_source_read_only() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("renamed.dat");
    for (little, order) in [(true, "little-endian"), (false, "big-endian")] {
        std::fs::write(&path, fixture::historical_pixel(little)).unwrap();
        let mut s = Session::new();
        s.execute("file.openAs", json!({"path":path,"as":"xcf","historicalXcfSampleOrder":order})).unwrap();
        assert_eq!(pixel(&s)[0], 0x1234 as f32 / 65535.);
        assert!(s.active().unwrap().source_read_only);
        assert!(s.active().unwrap().path.is_none());
    }
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn historical_snapshot_refuses_a_replaced_source_and_cancelled_read() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.xcf");
    std::fs::write(&path, fixture::historical_pixel(true)).unwrap();
    let handle = std::fs::File::open(&path).unwrap();
    let identity = photocraft_io::FileIdentity::of_handle(&handle).unwrap();
    std::fs::write(&path, b"replacement").unwrap();
    let error = snapshot(path.to_str().unwrap(), &identity, &photocraft_raster::Interrupt::NONE).unwrap_err();
    assert!(matches!(error, EngineError::SourceChanged { .. }));
    let cancelled = || true;
    let ctl = photocraft_raster::Interrupt::cancel_only(&cancelled);
    assert!(matches!(snapshot(path.to_str().unwrap(), &identity, &ctl), Err(EngineError::Cancelled)));
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn historical_snapshot_observes_cancellation_between_read_chunks() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.xcf");
    std::fs::write(&path, fixture::historical_pixel(true)).unwrap();
    let handle = std::fs::File::open(&path).unwrap();
    let identity = photocraft_io::FileIdentity::of_handle(&handle).unwrap();
    let cancelled = AtomicBool::new(false);
    let saw_progress = AtomicBool::new(false);
    let cancel = || cancelled.load(Ordering::Relaxed);
    let progress = |fraction: f32| {
        if fraction > 0.0 {
            saw_progress.store(true, Ordering::Relaxed);
            cancelled.store(true, Ordering::Relaxed);
        }
    };
    let ctl = photocraft_raster::Interrupt::new(&cancel, &progress);
    assert!(matches!(snapshot(path.to_str().unwrap(), &identity, &ctl), Err(EngineError::Cancelled)));
    assert!(saw_progress.load(Ordering::Relaxed), "cancellation occurs after a read, not at the initial checkpoint");
}

#[test]
fn refused_historical_start_retains_exact_source_and_choice_identity_for_retry() {
    let mut session = Session::new();
    session.execute("file.new", json!({"width":2,"height":2})).unwrap();
    let original = session.active().unwrap().doc.clone();
    let bytes = Arc::new(fixture::historical_pixel(false));
    let mut ids = Vec::new();
    for name in ["retained.xcf", "other.xcf"] {
        let Started::Done(question) = session.start_open_for_user_with_background(name, OpenSource::Bytes(bytes.clone()), false).unwrap() else {
            panic!("inline question")
        };
        ids.push(question["heldHistoricalXcfOpen"].as_u64().unwrap());
    }
    let retained = Arc::strong_count(&bytes);
    let cancelled = crate::jobs::JobCtx::new();
    cancelled.cancel();
    session.set_inline_job_ctx(Some(cancelled));
    let error = session.resume_historical_xcf_open(ids[0], photocraft_io::HistoricalXcfSampleOrder::BigEndian, false).unwrap_err();
    assert!(matches!(error, EngineError::Cancelled));
    assert_eq!(session.jobs.historical_xcf.iter().map(|held| held.id).collect::<Vec<_>>(), ids);
    assert!(Arc::ptr_eq(&session.jobs.historical_xcf[0].bytes, &bytes));
    assert_eq!(Arc::strong_count(&bytes), retained, "failed startup must neither lose nor duplicate the retained allocation");
    assert!(session.jobs.historical_xcf[0].options.historical_xcf_sample_order.is_none(), "the unanswered source options remain unchanged until retry");
    assert_eq!(session.documents().len(), 1);
    assert!(Arc::ptr_eq(&original, &session.active().unwrap().doc));
    session.set_inline_job_ctx(None);
    session.resume_historical_xcf_open(ids[0], photocraft_io::HistoricalXcfSampleOrder::BigEndian, false).unwrap();
    assert!(!session.holds_historical_xcf_open(ids[0]));
    assert!(session.holds_historical_xcf_open(ids[1]));
    assert_eq!(pixel(&session), [0x1234 as f32 / 65535., 0x4567 as f32 / 65535., 0x89ab as f32 / 65535., 1.]);
    assert_eq!(session.documents().len(), 2);
    assert!(Arc::ptr_eq(&original, &session.documents()[0].doc));
    assert!(session.cancel_historical_xcf_open(ids[1]));
    assert_eq!(Arc::strong_count(&bytes), 1);
}
