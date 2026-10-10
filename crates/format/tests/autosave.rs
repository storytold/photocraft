//! Background autosave and crash recovery.

mod common;
use std::sync::Arc;

use common::*;
use photocraft_color::{ColorMode, SampleType};
use photocraft_format::*;

#[test]
fn autosave_then_recover() {
    let dir = temp_dir("recovery");
    let doc = Arc::new(rich_doc(ColorMode::Rgb, SampleType::U16));
    let saver = Autosaver::new(&dir, "doc-1");
    saver.request(doc.clone(), 7, Some("/work/a.pcraft".into()), SaveOptions::default()).unwrap();
    let r = saver.flush().expect("a save ran");
    let stats = r.unwrap();
    assert!(stats.tiles_written > 0);
    let entries = list_recovery(&dir);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].info.revision, 7);
    assert_eq!(entries[0].info.original_path.as_deref(), Some("/work/a.pcraft"));
    assert_eq!(entries[0].info.document_name, "Rich");
    assert_eq!(recover(&entries[0]).unwrap(), *doc);
    discard_recovery(&dir, &entries[0]).unwrap();
    assert!(list_recovery(&dir).is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

/// A queued snapshot can fail later; callers must observe and retry that revision.
#[test]
fn background_save_outcomes_report_failure_then_recovery() {
    let root = temp_dir("retry-failed-write");
    let blocked = root.join("recovery");
    std::fs::write(&blocked, b"not a directory").unwrap();
    let saver = Autosaver::new(&blocked, "doc");
    let doc = Arc::new(rich_doc(ColorMode::Rgb, SampleType::U8));
    saver.request(doc.clone(), 7, None, SaveOptions::default()).unwrap();

    let receive = |saver: &Autosaver| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while std::time::Instant::now() < deadline {
            if let Some(outcome) = saver.take_completion() {
                return (outcome.revision, outcome.result);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("autosave worker did not report its completed write within 30 seconds");
    };
    let (revision, result) = receive(&saver);
    assert_eq!(revision, 7);
    assert!(result.is_err(), "the recovery location is not a directory");

    std::fs::remove_file(&blocked).unwrap();
    std::fs::create_dir(&blocked).unwrap();
    saver.request(doc.clone(), 7, None, SaveOptions::default()).unwrap();
    let (revision, result) = receive(&saver);
    assert_eq!(revision, 7);
    result.unwrap();
    assert_eq!(list_recovery(&blocked).len(), 1);
    assert_eq!(recover(&list_recovery(&blocked)[0]).unwrap(), *doc);
    drop(saver);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn repeated_autosaves_are_incremental_and_coalesced() {
    let dir = temp_dir("coalesce");
    let saver = Autosaver::new(&dir, "k");
    let mut doc = rich_doc(ColorMode::Rgb, SampleType::U8);
    for i in 0..5 {
        let id = doc.layers[1].id;
        doc.layer_mut(id).unwrap().surface_mut().unwrap().write_pixel(i, 0, &[1.0, 0.0, 0.0, 1.0]);
        saver.request(Arc::new(doc.clone()), i as u64, None, SaveOptions::default()).unwrap();
    }
    saver.flush().unwrap().unwrap();
    let e = list_recovery(&dir);
    assert_eq!(e[0].info.revision, 4, "newest snapshot wins");
    assert_eq!(recover(&e[0]).unwrap(), doc);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn discard_removes_everything() {
    let dir = temp_dir("discard");
    let saver = Autosaver::new(&dir, "x y/z");
    saver.request(Arc::new(rich_doc(ColorMode::Grayscale, SampleType::U8)), 1, None, SaveOptions::default()).unwrap();
    let path = saver.bundle_path();
    assert!(path.file_name().unwrap().to_string_lossy().starts_with("x_y_z"));
    saver.discard().unwrap();
    assert!(list_recovery(&dir).is_empty());
    assert!(!path.exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn list_ignores_junk() {
    let dir = temp_dir("junk");
    std::fs::write(dir.join("bogus.json"), b"{}").unwrap();
    std::fs::write(dir.join("other.txt"), b"x").unwrap();
    assert!(list_recovery(&dir).is_empty());
    assert!(list_recovery(&dir.join("missing")).is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn worker_failure_is_acknowledged_and_same_revision_can_retry() {
    let dir = temp_dir("retry");
    let blocked = dir.join("blocked");
    std::fs::write(&blocked, b"not a directory").unwrap();
    let saver = Autosaver::new(&blocked, "doc");
    let doc = Arc::new(rich_doc(ColorMode::Rgb, SampleType::U8));
    saver.request(doc.clone(), 9, None, SaveOptions::default()).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        if let Some(completion) = saver.take_completion() {
            assert_eq!(completion.revision, 9);
            assert!(completion.result.is_err());
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    std::fs::remove_file(&blocked).unwrap();
    saver.request(doc.clone(), 9, None, SaveOptions::default()).unwrap();
    saver.flush().unwrap().unwrap();
    assert_eq!(recover(&list_recovery(&blocked)[0]).unwrap(), *doc);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn checkpoint_preserves_current_undo_redo_and_lazy_lease_at_every_depth() {
    use photocraft_ops::{History, HistoryState};
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let dir = temp_dir("checkpoint");
        let current = Arc::new(rich_doc(ColorMode::Rgb, depth));
        let mut before = (*current).clone();
        before.name = "Before".into();
        let mut after = (*current).clone();
        after.name = "After".into();
        let mut checkpoint = History::default().checkpoint();
        checkpoint.current_label = "Current edit".into();
        checkpoint.undo.push(HistoryState::from_document("Before edit", Arc::new(before.clone())));
        checkpoint.redo.push(HistoryState::from_document("After edit", Arc::new(after.clone())));
        let saver = Autosaver::new(&dir, "history");
        saver.request_checkpoint(current.clone(), checkpoint, 12, None, SaveOptions::default()).unwrap();
        saver.flush().unwrap().unwrap();
        let entry = list_recovery(&dir).remove(0);
        let (restored, history) = recover_checkpoint(&entry).unwrap();
        assert_eq!(restored, *current);
        assert_eq!(history.current_label, "Current edit");
        let restored_before = history.undo[0].load_document().unwrap();
        assert_eq!(*restored_before, before);
        let mut allocations = std::collections::HashSet::new();
        assert!(photocraft_ops::document_bytes(&restored, &mut allocations) > 0);
        assert_eq!(photocraft_ops::document_bytes(&restored_before, &mut allocations), 0, "unchanged recovered pixels and blobs share live allocations");
        assert_eq!(*history.redo[0].load_document().unwrap(), after);
        let saver = Autosaver::new(&dir, "history");
        saver.request(current.clone(), 13, None, SaveOptions::default()).unwrap();
        saver.flush().unwrap().unwrap();
        // A replacement checkpoint and normal save retirement cannot remove
        // immutable objects still needed by recovered cold undo/redo states.
        discard_recovery(&dir, &entry).unwrap();
        assert!(list_recovery(&dir).is_empty());
        assert_eq!(*history.undo[0].load_document().unwrap(), before);
        drop(history);
    }
}

#[test]
fn failed_history_load_preserves_previous_complete_checkpoint() {
    #[derive(Debug)]
    struct Broken;
    impl photocraft_ops::ArchivedDocument for Broken {
        fn load(&self) -> std::result::Result<Arc<photocraft_doc::Document>, String> {
            Err("cold history unavailable".into())
        }
    }
    let dir = temp_dir("preserve");
    let doc = Arc::new(rich_doc(ColorMode::Rgb, SampleType::U16));
    let saver = Autosaver::new(&dir, "safe");
    saver.request(doc.clone(), 1, None, SaveOptions::default()).unwrap();
    saver.flush().unwrap().unwrap();
    let saver = Autosaver::new(&dir, "safe");
    let mut checkpoint = photocraft_ops::History::default().checkpoint();
    checkpoint.undo.push(photocraft_ops::HistoryState::from_archive("Broken", false, Arc::new(Broken)));
    saver.request_checkpoint(doc.clone(), checkpoint, 2, None, SaveOptions::default()).unwrap();
    assert!(saver.flush().unwrap().is_err());
    let entries = list_recovery(&dir);
    assert_eq!(entries[0].info.revision, 1);
    assert_eq!(recover(&entries[0]).unwrap(), *doc);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn malicious_key_is_reported_and_cannot_delete_outside_storage() {
    let dir = temp_dir("unsafe-key");
    let metadata = serde_json::json!({"key":"../victim", "document_name":"Bad", "original_path":null, "saved_at":0, "revision":1});
    std::fs::write(dir.join("bad.json"), serde_json::to_vec(&metadata).unwrap()).unwrap();
    let (entries, errors) = list_recovery_checked(&dir);
    assert!(entries.is_empty());
    assert_eq!(errors.len(), 1);
    assert!(dir.join("bad.json").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn legacy_sidecar_recovery_remains_supported() {
    let dir = temp_dir("legacy");
    let doc = rich_doc(ColorMode::Rgb, SampleType::U8);
    PcraftWriter::new().save_dir(&doc, &dir.join("old.pcraft"), &SaveOptions::default()).unwrap();
    let metadata = serde_json::json!({"key":"old", "document_name":"Rich", "original_path":null, "saved_at":0, "revision":1});
    std::fs::write(dir.join("old.json"), serde_json::to_vec(&metadata).unwrap()).unwrap();
    let (current, history) = recover_checkpoint(&list_recovery(&dir)[0]).unwrap();
    assert_eq!(current, doc);
    assert!(history.undo.is_empty());
    assert!(history.redo.is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn blocked_worker_retains_only_newest_pending_snapshot() {
    #[derive(Debug)]
    struct Blocked {
        doc: Arc<photocraft_doc::Document>,
        entered: std::sync::mpsc::Sender<()>,
        release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    }
    impl photocraft_ops::ArchivedDocument for Blocked {
        fn load(&self) -> std::result::Result<Arc<photocraft_doc::Document>, String> {
            self.entered.send(()).map_err(|e| e.to_string())?;
            self.release.lock().map_err(|e| e.to_string())?.recv().map_err(|e| e.to_string())?;
            Ok(self.doc.clone())
        }
    }
    let dir = temp_dir("bounded");
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let base = Arc::new(rich_doc(ColorMode::Rgb, SampleType::U8));
    let mut history = photocraft_ops::History::default().checkpoint();
    history.undo.push(photocraft_ops::HistoryState::from_archive(
        "Wait",
        false,
        Arc::new(Blocked { doc: base.clone(), entered: entered_tx, release: std::sync::Mutex::new(release_rx) }),
    ));
    let saver = Autosaver::new(&dir, "bound");
    saver.request_checkpoint(base.clone(), history, 0, None, SaveOptions::default()).unwrap();
    entered_rx.recv_timeout(std::time::Duration::from_secs(15)).unwrap();
    let mut weak = Vec::new();
    for revision in 1..=30 {
        let mut document = (*base).clone();
        document.name = format!("Revision {revision}");
        let doc = Arc::new(document);
        weak.push(Arc::downgrade(&doc));
        saver.request(doc, revision, None, SaveOptions::default()).unwrap();
    }
    assert!(weak.iter().take(29).all(|doc| doc.strong_count() == 0));
    assert_eq!(weak[29].strong_count(), 1);
    release_tx.send(()).unwrap();
    saver.flush().unwrap().unwrap();
    let entries = list_recovery(&dir);
    assert_eq!(entries[0].info.revision, 30);
    assert_eq!(recover(&entries[0]).unwrap().name, "Revision 30");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn view_context_roundtrips_and_large_context_is_rejected() {
    let dir = temp_dir("context");
    let saver = Autosaver::new(&dir, "view");
    let doc = Arc::new(rich_doc(ColorMode::Rgb, SampleType::U8));
    let context = serde_json::json!({"zoom":1.5,"center":[42.0,84.0],"tool":"brush","tabOrder":2});
    saver
        .request_checkpoint_with_context(doc.clone(), photocraft_ops::History::default().checkpoint(), 1, None, SaveOptions::default(), context.clone())
        .unwrap();
    saver.flush().unwrap().unwrap();
    let (_, _, recovered_context) = recover_checkpoint_with_context(&list_recovery(&dir)[0]).unwrap();
    assert_eq!(recovered_context, context);
    let saver = Autosaver::new(&dir, "view");
    assert!(
        saver
            .request_checkpoint_with_context(
                doc,
                photocraft_ops::History::default().checkpoint(),
                2,
                None,
                SaveOptions::default(),
                serde_json::json!({"oversized":"x".repeat(70_000)})
            )
            .is_err()
    );
    saver.discard().unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn symlinked_descriptors_roots_and_objects_are_rejected() {
    use std::os::unix::fs::symlink;
    let dir = temp_dir("symlinks");
    let outside = temp_dir("outside");
    let doc = Arc::new(rich_doc(ColorMode::Rgb, SampleType::U8));
    let saver = Autosaver::new(&dir, "safe");
    saver.request(doc, 1, None, SaveOptions::default()).unwrap();
    saver.flush().unwrap().unwrap();
    let entry = list_recovery(&dir).remove(0);
    let sidecar = dir.join("safe.json");
    let saved_sidecar = outside.join("descriptor.json");
    std::fs::rename(&sidecar, &saved_sidecar).unwrap();
    symlink(&saved_sidecar, &sidecar).unwrap();
    assert!(list_recovery_checked(&dir).0.is_empty());
    assert!(recover(&entry).is_err());
    assert!(discard_recovery(&dir, &entry).is_err());
    std::fs::remove_file(&sidecar).unwrap();
    std::fs::rename(&saved_sidecar, &sidecar).unwrap();
    let object = std::fs::read_dir(entry.bundle.join("tiles")).unwrap().next().unwrap().unwrap().path();
    let outside_object = outside.join("object.zst");
    std::fs::rename(&object, &outside_object).unwrap();
    symlink(&outside_object, &object).unwrap();
    assert!(recover(&entry).is_err());
    std::fs::remove_file(&object).unwrap();
    std::fs::rename(&outside_object, &object).unwrap();
    let outside_root = outside.join("bundle");
    std::fs::rename(&entry.bundle, &outside_root).unwrap();
    symlink(&outside_root, &entry.bundle).unwrap();
    assert!(recover(&entry).is_err());
    assert!(discard_recovery(&dir, &entry).is_err());
    assert!(outside_root.exists());
    std::fs::remove_dir_all(dir).unwrap();
    std::fs::remove_dir_all(outside).unwrap();
}

#[test]
fn autosave_repairs_corrupt_existing_objects_before_acknowledging() {
    let dir = temp_dir("repair-object");
    let saver = Autosaver::new(&dir, "repair");
    let doc = Arc::new(rich_doc(ColorMode::Rgb, SampleType::U16));
    saver.request(doc.clone(), 1, None, SaveOptions::default()).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        if let Some(completion) = saver.take_completion() {
            completion.result.unwrap();
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    let entry = list_recovery(&dir).remove(0);
    let object = std::fs::read_dir(entry.bundle.join("tiles")).unwrap().next().unwrap().unwrap().path();
    std::fs::write(&object, b"truncated").unwrap();
    assert!(recover(&entry).is_err());
    // Reuse the same worker: fingerprint changes invalidate its verified cache.
    saver.request(doc.clone(), 2, None, SaveOptions::default()).unwrap();
    saver.flush().unwrap().unwrap();
    assert_eq!(recover(&list_recovery(&dir)[0]).unwrap(), *doc);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn missing_descriptor_does_not_consume_a_failed_recovery_source() {
    let dir = temp_dir("missing-descriptor");
    let doc = rich_doc(ColorMode::Rgb, SampleType::U16);
    let bundle = dir.join("retained.pcraft");
    PcraftWriter::new().save_dir(&doc, &bundle, &SaveOptions::default()).unwrap();
    let entry = RecoveryEntry {
        info: photocraft_format::autosave::RecoveryInfo {
            key: "retained".into(),
            document_name: doc.name.clone(),
            original_path: None,
            saved_at: 0,
            revision: 1,
        },
        bundle: bundle.clone(),
    };
    assert!(recover_checkpoint(&entry).is_err());
    assert_eq!(load_path(&bundle).unwrap(), doc, "a failed load never implicitly retires its source");
    std::fs::remove_dir_all(dir).unwrap();
}
