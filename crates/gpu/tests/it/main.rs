//! The crate's integration tests, linked into one test binary.

/// Concurrent wgpu instances in one process segfault on some drivers (RADV), so every test that
/// creates a device takes this lock and holds it until the device is dropped.
static GPU_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The wgpu instance every test uses (`WGPU_BACKEND` picks its backends), kept for the whole
/// run: the last instance going away tears the Vulkan loader down, and an instance made while
/// that happens can crash in libvulkan.
pub(crate) fn instance() -> wgpu::Instance {
    static KEPT: std::sync::OnceLock<wgpu::Instance> = std::sync::OnceLock::new();
    KEPT.get_or_init(|| wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env())).clone()
}

mod bounded_readback;
mod camera_raw;
mod device_loss;
mod parity;
mod pipeline_errors;
mod timing;
mod uniform_limits;
mod vector_mask;
