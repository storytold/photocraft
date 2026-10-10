//! Subprocess termination deliberately bypasses Autosaver and history drops.
#![cfg(not(target_arch = "wasm32"))]
mod common;
use photocraft_color::{ColorMode, SampleType};
use photocraft_format::{Autosaver, PcraftWriter, SaveOptions, list_recovery, recover_checkpoint};
use photocraft_ops::{History, HistoryState};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[test]
fn abrupt_exit_checkpoint_child() {
    let Some(root) = std::env::var_os("PHOTOCRAFT_CRASH_TEST_ROOT") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let depth = match std::env::var("PHOTOCRAFT_CRASH_TEST_DEPTH").unwrap().as_str() {
        "8" => SampleType::U8,
        "16" => SampleType::U16,
        "32" => SampleType::F32,
        _ => panic!("invalid test depth"),
    };
    let current = Arc::new(common::rich_doc(ColorMode::Rgb, depth));
    let mut before = (*current).clone();
    before.name = "Before crash edit".into();
    let mut after = (*current).clone();
    after.name = "After crash edit".into();
    // Use native bundles as exact per-process oracles, including IDs and pixels.
    let mut writer = PcraftWriter::new();
    for (name, doc) in [("current", current.as_ref()), ("before", &before), ("after", &after)] {
        writer.save_path(doc, &root.join(format!("{name}.pcraft")), &SaveOptions::default()).unwrap();
    }
    let mut history = History::default().checkpoint();
    history.current_label = "Current".into();
    history.undo.push(HistoryState::from_document("Before", Arc::new(before)));
    history.redo.push(HistoryState::from_document("After", Arc::new(after)));
    let saver = Autosaver::new(root.join("Recovery"), "abrupt");
    saver.request_checkpoint(current, history, 42, None, SaveOptions::default()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(completion) = saver.take_completion() {
            completion.result.unwrap();
            // process::exit skips all Rust destructors: no flush or clean shutdown.
            std::process::exit(0);
        }
        assert!(Instant::now() < deadline, "checkpoint failed to complete");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn abrupt_process_exit_restores_exact_current_undo_and_redo_at_every_depth() {
    for depth in ["8", "16", "32"] {
        let root = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "abrupt_exit_checkpoint_child", "--nocapture"])
            .env("PHOTOCRAFT_CRASH_TEST_ROOT", root.path())
            .env("PHOTOCRAFT_CRASH_TEST_DEPTH", depth)
            .status()
            .unwrap();
        assert!(status.success(), "checkpoint subprocess failed at depth {depth}");
        let entries = list_recovery(&root.path().join("Recovery"));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].info.revision, 42);
        let expected = |name| photocraft_format::load_path(&root.path().join(format!("{name}.pcraft"))).unwrap();
        let (current, checkpoint) = recover_checkpoint(&entries[0]).unwrap();
        assert_eq!(current, expected("current"));
        let mut history = History::from_checkpoint(checkpoint).unwrap();
        let current = Arc::new(current);
        let before = history.try_undo(current.clone()).unwrap().unwrap().0;
        assert_eq!(*before, expected("before"));
        let current_again = history.try_redo(before).unwrap().unwrap().0;
        assert_eq!(*current_again, *current);
        let after = history.try_redo(current_again).unwrap().unwrap().0;
        assert_eq!(*after, expected("after"));
        // Restarting recovery twice must never consume the durable descriptor.
        assert_eq!(recover_checkpoint(&entries[0]).unwrap().0, *current);
    }
}
