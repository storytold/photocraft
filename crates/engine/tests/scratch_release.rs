//! One integration-test process keeps the process-wide scratch pool isolated from parallel unit
//! tests (#10479).
use photocraft_engine::Session;
use photocraft_raster::scratch;
use serde_json::json;
use std::sync::{Mutex, PoisonError};

static SERIAL: Mutex<()> = Mutex::new(());

#[test]
fn closing_a_document_leaves_none_of_its_buffers_in_the_pool() {
    let _g = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
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
    let _g = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 1024, "height": 1024})).unwrap();
    scratch::give(vec![0u8; scratch::MIN_POOLED_BYTES]);
    s.execute("edit.fill", json!({"contents": "color", "color": "#808080"})).unwrap();
    assert!(scratch::retained_bytes() > 0);
}
