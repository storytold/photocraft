use super::*;
use photocraft_doc::{Color, ColorMode, SampleType, Size};

fn document(name: &str, depth: SampleType) -> Document {
    let mut doc = Document::with_background(name, Size::new(512, 512), ColorMode::Rgb, depth, Color::WHITE);
    // Background creation is lazy on current main. Materialize every tile so these tests
    // exercise real budget pressure instead of counting a shared solid fill as 0 bytes.
    if let Some(surface) = doc.layers.first_mut().and_then(|layer| layer.surface_mut()) {
        for y in [0, 256] {
            for x in [0, 256] {
                surface.write_pixel(x, y, &[0.25, 0.5, 0.75, 1.0]);
            }
        }
    }
    doc
}

fn small_cache(disk_mb: u32) -> Session {
    let mut s = Session::new();
    s.prefs.edit(|p| {
        p.performance.memory_usage_mb = 1;
        p.scratch_disks.budget_mb = disk_mb;
    });
    s
}

fn replace_pixels(s: &mut Session, name: &str, depth: SampleType) {
    s.edit(name, |doc, active| {
        let id = doc.id;
        *doc = document(name, depth);
        doc.id = id;
        *active = doc.top_layer();
        Ok(())
    })
    .unwrap();
}

#[cfg(not(target_arch = "wasm32"))]
fn drain(s: &mut Session) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        s.poll_history_cache();
        if !s.history_cache_busy() && !s.history_cache.dirty {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "cache worker did not complete");
        std::thread::yield_now();
    }
}

#[test]
fn shared_tiles_are_counted_once_and_scratch_starts_empty() {
    let mut s = Session::new();
    let doc = document("shared", SampleType::U8);
    s.add_document(doc.clone(), None);
    let one = s.history_resident_bytes();
    s.add_document(doc, None);
    assert_eq!(s.history_resident_bytes(), one, "documents share their COW pixels");
    assert_eq!(s.history_disk_bytes(), 0);
    assert!(!s.history_cache_busy());
    #[cfg(not(target_arch = "wasm32"))]
    assert!(s.history_cache.archive.is_none(), "no scratch store before a spill");
}

#[test]
fn disk_disabled_retires_cold_history_and_keeps_current() {
    let mut s = small_cache(0);
    s.add_document(document("original", SampleType::U8), None);
    for name in ["one", "two", "three", "four"] {
        replace_pixels(&mut s, name, SampleType::U8);
        s.poll_history_cache();
    }
    assert_eq!(s.active().unwrap().doc.name, "four");
    assert_eq!(s.history_disk_bytes(), 0);
    assert!(!s.history_cache_busy());
    assert!(s.active().unwrap().history.past_len() <= 1);
    assert!(s.undo(), "adjacent undo stays available");
    assert_eq!(s.active().unwrap().doc.name, "four");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn spilled_history_restores_all_sample_depths() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut s = small_cache(8);
        s.add_document(document("original", depth), None);
        let mut expected = vec![s.active().unwrap().doc.clone()];
        for name in ["one", "two", "three"] {
            replace_pixels(&mut s, name, depth);
            expected.push(s.active().unwrap().doc.clone());
            drain(&mut s);
        }
        assert!(s.history_disk_bytes() > 0, "older states must reach scratch");
        assert!(s.active().unwrap().history.archived_states() > 0);
        for index in [2, 1, 0] {
            assert!(s.undo());
            assert_eq!(s.active().unwrap().doc.name, "three");
            assert_eq!(s.active().unwrap().doc.depth, depth);
            let mut expected_doc = (*expected[index]).clone();
            expected_doc.name = "three".into();
            assert_eq!(s.active().unwrap().doc.as_ref(), &expected_doc);
            drain(&mut s);
        }
        for index in [1, 2, 3] {
            assert!(s.redo());
            assert_eq!(s.active().unwrap().doc.name, "three");
            assert_eq!(s.active().unwrap().doc.depth, depth);
            let mut expected_doc = (*expected[index]).clone();
            expected_doc.name = "three".into();
            assert_eq!(s.active().unwrap().doc.as_ref(), &expected_doc);
            drain(&mut s);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn invalid_scratch_path_never_loses_the_completed_edit() {
    let sentinel = std::env::temp_dir().join(format!("photocraft-cache-blocked-{}-{}", std::process::id(), photocraft_doc::DocId::fresh().0));
    std::fs::write(&sentinel, b"not a directory").unwrap();
    let mut s = small_cache(8);
    s.prefs.edit(|p| p.scratch_disks.disks = vec![crate::prefs::ScratchDisk { path: sentinel.to_string_lossy().into_owned(), enabled: true }]);
    s.add_document(document("original", SampleType::U8), None);
    for name in ["one", "two", "three"] {
        replace_pixels(&mut s, name, SampleType::U8);
        drain(&mut s);
    }
    assert_eq!(s.active().unwrap().doc.name, "three");
    assert_eq!(s.history_disk_bytes(), 0);
    assert!(s.take_history_cache_notice().is_some());
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc.name, "three");
    std::fs::remove_file(sentinel).unwrap();
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn reducing_disk_budget_reclaims_archives_without_changing_current() {
    let mut s = small_cache(8);
    s.add_document(document("original", SampleType::U8), None);
    for name in ["one", "two", "three"] {
        replace_pixels(&mut s, name, SampleType::U8);
        drain(&mut s);
    }
    assert!(s.history_disk_bytes() > 0);
    let current = s.active().unwrap().doc.clone();
    s.execute("prefs.set", serde_json::json!({"path": "scratchDisks.budgetMb", "value": 0})).unwrap();
    drain(&mut s);
    assert_eq!(s.history_disk_bytes(), 0);
    assert!(Arc::ptr_eq(&s.active().unwrap().doc, &current));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn close_and_purge_reclaim_unused_scratch() {
    for close in [false, true] {
        let mut s = small_cache(8);
        s.add_document(document("original", SampleType::U8), None);
        for name in ["one", "two", "three"] {
            replace_pixels(&mut s, name, SampleType::U8);
            drain(&mut s);
        }
        assert!(s.history_disk_bytes() > 0);
        if close {
            drop(s.close(0));
        } else {
            s.execute("edit.purge.histories", serde_json::json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc.name, "three");
        }
        drain(&mut s);
        assert_eq!(s.history_disk_bytes(), 0, "close={close}");
    }
}

#[test]
fn undo_command_decode_failure_preserves_document_and_history_cursors() {
    #[derive(Debug)]
    struct CorruptArchive;
    impl photocraft_ops::ArchivedDocument for CorruptArchive {
        fn load(&self) -> std::result::Result<Arc<Document>, String> {
            Err("scratch snapshot checksum mismatch".into())
        }
    }
    let mut s = Session::new();
    s.add_document(document("original", SampleType::U8), None);
    replace_pixels(&mut s, "edited", SampleType::U8);
    let previous = s.active().unwrap().history.resident_state(0).unwrap();
    assert!(s.active_mut().unwrap().history.replace_resident(&previous, Arc::new(CorruptArchive)));
    let current = s.active().unwrap().doc.clone();
    let labels = s.active().unwrap().history.entries();
    let revision = s.active().unwrap().revision;
    assert!(s.execute("edit.undo", serde_json::json!({})).is_err());
    let active = s.active().unwrap();
    assert!(Arc::ptr_eq(&active.doc, &current));
    assert_eq!(active.revision, revision);
    assert_eq!(active.history.entries(), labels);
    assert!(active.history.can_undo());
    assert!(!active.history.can_redo());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn slow_scratch_write_does_not_accumulate_an_unbounded_edit_backlog() {
    let mut s = small_cache(8);
    s.add_document(document("original", SampleType::U8), None);
    let source = s.active().unwrap().doc.clone();
    let (release, blocked) = std::sync::mpsc::channel::<()>();
    let (completed, rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _ = blocked.recv();
        let _ = completed.send(Ok(Completed::Collected));
    });
    s.history_cache.pending = Some(Pending { source: Some(source), rx, worker });
    for name in ["one", "two", "three", "four", "five", "six"] {
        replace_pixels(&mut s, name, SampleType::U8);
        assert!(s.history_cache_busy(), "worker is deterministically blocked");
        assert_eq!(s.active().unwrap().doc.name, name);
        let allowed = s.prefs().performance.history_budget_bytes().max(s.history_pinned_bytes());
        assert!(s.history_resident_bytes() <= allowed, "cold backlog exceeded pinned working set");
    }
    assert!(s.take_history_cache_notice().is_some());
    // Disable future writes, then release the fake worker and allow normal poll cleanup.
    s.prefs.edit(|p| p.scratch_disks.budget_mb = 0);
    release.send(()).unwrap();
    drain(&mut s);
    assert_eq!(s.active().unwrap().doc.name, "six");
    assert!(!s.history_cache_busy());
}

#[test]
fn ten_thousand_nudges_keep_individual_steps_without_pixel_copies() {
    for (depth, steps) in [(SampleType::U8, 10_000), (SampleType::U16, 1_000), (SampleType::F32, 1_000)] {
        let mut s = Session::new();
        s.execute("prefs.set", serde_json::json!({"path":"performance.historyStates", "value":10000})).unwrap();
        let mut doc = Document::with_background("nudges", Size::new(512, 512), ColorMode::Rgb, depth, Color::WHITE);
        let layer = doc.layers.first_mut().unwrap();
        layer.locks = Default::default();
        let surface = layer.surface_mut().unwrap();
        surface.write_pixel(20, 20, &[0.2, 0.4, 0.6, 1.0]);
        s.add_document(doc, None);
        let initial = s.active().unwrap().doc.clone();
        let one = s.history_resident_bytes();
        let started = std::time::Instant::now();
        for i in 0..steps {
            s.execute("layer.translate", serde_json::json!({"dx": if i % 2 == 0 { 1 } else { -1 }, "dy":0})).unwrap();
        }
        assert_eq!(s.active().unwrap().history.past_len(), steps);
        assert!(s.history_resident_bytes() <= 3 * one, "moves retain a source and current pixels, not one image per keypress");
        assert_eq!(s.history_disk_bytes(), 0);
        assert!(s.history_metadata_bytes() < steps * 256 + 1024, "compact step descriptors stay small");
        eprintln!("descriptor_bytes={} for {steps} steps", s.history_metadata_bytes());
        let final_doc = s.active().unwrap().doc.clone();
        for _ in 0..steps {
            assert!(s.try_undo().unwrap());
        }
        assert_eq!(s.active().unwrap().doc.as_ref(), initial.as_ref());
        for _ in 0..steps {
            assert!(s.try_redo().unwrap());
        }
        assert_eq!(s.active().unwrap().doc.as_ref(), final_doc.as_ref());
        eprintln!("nudge depth={depth:?} steps={steps} payload={} initial={} elapsed={:?}", s.history_resident_bytes(), one, started.elapsed());
    }
}

#[test]
fn a_large_stroke_remains_one_exact_step_between_compact_moves() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut s = Session::new();
        let mut doc = document("stroke", depth);
        doc.layers.first_mut().unwrap().locks = Default::default();
        s.add_document(doc, None);
        s.execute("layer.translate", serde_json::json!({"dx":1})).unwrap();
        let before = s.active().unwrap().doc.clone();
        let payload = s.history_resident_bytes();
        let points: Vec<_> = (0..16).map(|i| serde_json::json!([if i % 2 == 0 { 0 } else { 500 }, i * 32, 1.0])).collect();
        s.execute("paint.stroke", serde_json::json!({"points":points,"size":32,"smoothing":0,"color":[0.9,0.1,0.3,1.0]})).unwrap();
        let painted = s.active().unwrap().doc.clone();
        assert!(s.history_resident_bytes() > payload, "paint must retain changed tiles");
        assert_eq!(s.active().unwrap().history.past_len(), 2);
        s.execute("layer.translate", serde_json::json!({"dx":-1})).unwrap();
        assert!(s.try_undo().unwrap());
        assert_eq!(s.active().unwrap().doc.as_ref(), painted.as_ref());
        assert!(s.try_undo().unwrap());
        assert_eq!(s.active().unwrap().doc.as_ref(), before.as_ref());
        assert!(s.try_redo().unwrap());
        assert_eq!(s.active().unwrap().doc.as_ref(), painted.as_ref());
    }
}
