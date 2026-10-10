//! Helpers shared by the modules of this test binary.

pub mod layout_doc;

/// Concurrent wgpu instances and devices in one process crash on some drivers (Mesa llvmpipe over
/// GL, RADV, see #194; with the NVIDIA and lavapipe Vulkan drivers both installed,
/// `wgpu::Instance::new` on several test threads segfaults inside libvulkan; Windows has shown
/// `STATUS_ACCESS_VIOLATION`). All of this crate's integration tests share one process, so every
/// test that creates a wgpu instance, a device or a `.wgpu()` harness holds this one lock for its
/// whole run, harness and devices included. A per-file lock would not serialize across files.
static GPU_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn gpu_lock() -> std::sync::MutexGuard<'static, ()> {
    GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
