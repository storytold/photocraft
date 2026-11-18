//! The crate's integration tests, linked into one test binary.

mod scratch;
mod scratch_capacity;
mod scratch_recovery;
mod scratch_slow_folder;
mod scratch_translate;
mod spill;

use std::sync::{Mutex, MutexGuard, PoisonError};

/// The scratch pool and the spill configuration are process-wide: tests that touch either take turns.
pub(crate) static SCRATCH: Mutex<()> = Mutex::new(());

/// Takes one of the locks above; a failed test that held it does not fail the next.
pub(crate) fn lock(lock: &'static Mutex<()>) -> MutexGuard<'static, ()> {
    lock.lock().unwrap_or_else(PoisonError::into_inner)
}
