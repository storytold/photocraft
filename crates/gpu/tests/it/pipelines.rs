//! #10575 / #8733: every compositor pipeline builds on the shader compilers real machines use.
//!
//! The app's default on Windows is DX12 through FXC (`d3dcompiler_47.dll`). FXC rejects shaders
//! that naga validates and that Vulkan, Metal and DXC accept (`error X3507: 'blend_channel': Not
//! all control paths return a value`), and every document then renders on the CPU for the whole
//! session. Nothing but a real FXC compile catches that, so this builds the compositor on a DX12
//! instance with FXC forced. It skips where no DX12 adapter exists (non-Windows, headless Linux CI).

use photocraft_gpu::Compositor;

use crate::GPU_LOCK;

/// A DX12 instance that compiles with FXC, as a release build does. `InstanceFlags::empty()`:
/// with `DEBUG` (the debug-build default) wgpu passes `D3DCOMPILE_SKIP_OPTIMIZATION`, and FXC then
/// accepts shaders it rejects when optimizing, so a debug-flag test would pass on broken shaders.
fn fxc_instance() -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12,
        flags: wgpu::InstanceFlags::empty(),
        backend_options: wgpu::BackendOptions {
            dx12: wgpu::Dx12BackendOptions { shader_compiler: wgpu::Dx12Compiler::Fxc, ..Default::default() },
            ..Default::default()
        },
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    })
}

#[test]
fn every_compositor_pipeline_builds_on_dx12_with_fxc() {
    let _lock = GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let instance = fxc_instance();
    let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::DX12));
    if adapters.is_empty() {
        eprintln!("skipping: no DX12 adapter");
        return;
    }
    let mut built = 0;
    for adapter in adapters {
        let info = adapter.get_info();
        let Ok((device, _queue)) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())) else {
            eprintln!("skipping {}: no device", info.name);
            continue;
        };
        let format = Compositor::preferred_acc_format(&adapter);
        match Compositor::try_new_with_format(&device, format) {
            Ok(_) => built += 1,
            Err(e) => panic!("DX12 adapter {} (FXC): {e}", info.name),
        }
    }
    eprintln!("built the compositor on {built} DX12 adapter(s) with FXC");
}
