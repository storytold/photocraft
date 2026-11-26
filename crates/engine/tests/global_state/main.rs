//! Tests that assert on process-wide state the rest of the engine's tests change (the scratch pool,
//! spill configuration, simulated memory, global caches, compositor settings). They live apart from
//! `it` so plain `cargo test` runs them without the rest of the suite.

mod artboard_export_scratch;
mod color_settings_presets;
mod lut_row_cache_purge;
mod mask_cache_purge;
mod memory_gate;
mod render_video_scratch;
mod scratch_disk;
mod scratch_pause;
mod scratch_release;
mod shape_kernel_budget;
mod smart_source_cache;
mod smart_source_peak;
mod spot_dot_gain;
mod text_gamma;

use std::sync::{Mutex, MutexGuard, PoisonError};

/// Every test here changes or asserts on process-wide state (scratch pool, spill configuration,
/// simulated memory, global caches, compositor settings) that the others touch too, so the tests
/// take turns.
pub(crate) static PROCESS: Mutex<()> = Mutex::new(());

/// Takes the lock; a failed test that held it does not fail the next.
pub(crate) fn lock(lock: &'static Mutex<()>) -> MutexGuard<'static, ()> {
    lock.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Empties every global cache that Purge All reports, so a test that asserts on what Purge All
/// finds starts from nothing whatever ran before it in this process.
pub(crate) fn purge_all_caches() {
    let mut session = photocraft_engine::Session::new();
    if session.is_enabled("edit.purge.all") {
        let _ = session.execute("edit.purge.all", serde_json::json!({}));
    }
}
