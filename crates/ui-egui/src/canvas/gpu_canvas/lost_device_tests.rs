//! A device lost between two polls (a DirectX 12 device removal, #13529) makes wgpu panic inside
//! `Queue::write_*` before its lost callback has run. The canvas must turn that into a recorded
//! loss, not a panic that closes the app.

use super::*;

#[test]
fn queue_writes_on_a_lost_device_record_the_loss_instead_of_panicking() {
    let _gpu = photocraft_testkit::gpu_lock();
    let rs = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| egui_kittest::wgpu::create_render_state(wgpu_setup(), Default::default())));
    let Ok(rs) = rs else {
        eprintln!("skipping lost-device test: no GPU adapter");
        return;
    };
    let gpu = GpuCanvas::new(&rs);
    let rgba = vec![255u8; 8 * 8 * 4];
    gpu.upload_full(1, [8, 8], &rgba);
    assert!(gpu.health.is_ok());
    // wgpu's `Device::destroy` leaves later queue writes quiet (they are accepted), so the panic of a
    // device removed by the driver is injected: a write that panics like wgpu does.
    let panicked = gpu.health.guard(|| -> () { std::panic::panic_any("Error in Queue::write_texture: Parent device is lost".to_string()) });
    assert_eq!(panicked, None);
    let wrote = gpu.upload_rect(1, [0, 0], [8, 8], &rgba);
    assert!(!wrote, "a write on a lost device must not report success");
    assert!(!gpu.health.is_ok(), "the lost device must be recorded");
    assert!(gpu.fault().is_some_and(|f| f.is_lost()));
    // And never touched again: every upload path now skips quietly.
    gpu.upload_full(1, [8, 8], &rgba);
    assert!(!gpu.upload_rect(1, [0, 0], [8, 8], &rgba));
}

/// The guard itself: a panic inside GPU work (a wgpu-style message) marks the device lost, the
/// work is skipped afterwards, and no panic escapes.
#[test]
fn a_panicking_gpu_frame_marks_the_device_lost() {
    let health = photocraft_gpu::DeviceHealth::new();
    let ran = health.guard(|| -> u32 { std::panic::panic_any("Error in Queue::write_texture: Parent device is lost".to_string()) });
    assert_eq!(ran, None);
    assert!(health.device_lost());
    assert!(health.fault().is_some_and(|f| f.is_lost()));
    assert_eq!(health.guard(|| 1), None, "no GPU work once the device is lost");
    let ok = photocraft_gpu::DeviceHealth::new();
    assert_eq!(ok.guard(|| 7), Some(7));
}
