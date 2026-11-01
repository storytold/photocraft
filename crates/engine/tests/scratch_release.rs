//! One integration-test process keeps the process-wide scratch pool isolated from parallel unit
//! tests (#10479).
use photocraft_engine::Session;
use photocraft_engine::jobs::Started;
use photocraft_raster::scratch;
use serde_json::json;
use std::sync::{Mutex, PoisonError};

static SERIAL: Mutex<()> = Mutex::new(());

/// Holds the test lock with the pool's idle release off, so a slow runner can't empty the pool
/// between a command and the check after it; the release comes back on when dropped.
struct Quiet {
    _serial: std::sync::MutexGuard<'static, ()>,
}

impl Quiet {
    fn new() -> Self {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        scratch::set_idle_release(false);
        Quiet { _serial }
    }
}

impl Drop for Quiet {
    fn drop(&mut self) {
        scratch::set_idle_release(true);
    }
}

#[test]
fn closing_a_document_leaves_none_of_its_buffers_in_the_pool() {
    let _q = Quiet::new();
    for close in ["file.close", "file.closeAll"] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 1024, "height": 1024})).unwrap();
        s.execute("edit.fill", json!({"contents": "color", "color": "#808080"})).unwrap();
        s.execute("filter.blur.gaussianBlur", json!({"radius": 3})).unwrap();
        // Dispatch holds the active document across the command; its tiles must not land in
        // the pool after the release.
        s.execute(close, json!({})).unwrap();
        assert_eq!(scratch::retained_bytes(), 0, "{close}");
    }
}

#[test]
fn commands_that_keep_their_documents_keep_the_pool() {
    let _q = Quiet::new();
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 1024, "height": 1024})).unwrap();
    scratch::give(vec![0u8; scratch::MIN_POOLED_BYTES]);
    s.execute("edit.fill", json!({"contents": "color", "color": "#808080"})).unwrap();
    assert!(scratch::retained_bytes() > 0);
}

#[test]
fn a_cancelled_job_of_a_closed_document_returns_its_buffers_once_it_exits() {
    let _q = Quiet::new();
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 2048, "height": 2048})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#808080"})).unwrap();
    let Started::Job(_) = s.start("filter.blur.gaussianBlur", json!({"radius": 60})).unwrap() else {
        panic!("Gaussian Blur runs as a background job");
    };
    // Closing cancels the job, but its worker still holds the document until it exits.
    s.close(0);
    scratch::give(vec![0u8; scratch::MIN_POOLED_BYTES]);
    s.join_cancelled_jobs();
    assert_eq!(scratch::retained_bytes(), 0);
}
