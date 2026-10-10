//! Helpers shared by the modules of this test binary.

/// Concurrent wgpu instances in one process segfault on some drivers (RADV), so every GPU test
/// takes this lock and holds it until its device is dropped. All of this crate's integration
/// tests share one process, so the lock must be this one shared lock, not one per file.
pub static GPU_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn gpu_lock() -> std::sync::MutexGuard<'static, ()> {
    GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
