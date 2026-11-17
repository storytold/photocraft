//! A headless context does not watch the desktop's appearance (#15499). Each watcher holds a
//! session-bus connection that its context keeps open for the whole process, so a test suite that
//! builds a context per test exhausted the desktop's bus and took the session down.
#![cfg(target_os = "linux")]

use craft::ui::runtime::env::SystemReader;
use photocraft_ui_egui::PhotocraftApp;

/// Threads of this process running a desktop appearance watcher. `craft-platform` names the
/// worker `desktop-appearance`; the kernel keeps the first 15 bytes.
fn watcher_threads() -> usize {
    std::fs::read_dir("/proc/self/task")
        .unwrap()
        .filter_map(|task| std::fs::read_to_string(task.ok()?.path().join("comm")).ok())
        .filter(|name| name.trim_end().starts_with("desktop-appear"))
        .count()
}

#[test]
fn headless_contexts_start_no_appearance_watcher() {
    // A watcher started directly must be counted, or a renamed worker would pass this test.
    let control = SystemReader::start(|| {});
    assert!(control.watching(), "the platform starts a watcher on Linux");
    let before = watcher_threads();
    assert!(before >= 1, "the control watcher's thread is counted");
    for _ in 0..3 {
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, Default::default());
    }
    assert_eq!(watcher_threads(), before, "a headless context must not open a desktop appearance watcher");
    drop(control);
}
