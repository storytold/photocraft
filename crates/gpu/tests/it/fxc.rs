//! The compositor builds on DX12 with FXC, the shader compiler the Windows app uses unless a
//! `dxcompiler.dll` ships beside it (#712). FXC rejects HLSL that DXC, Vulkan and Metal accept,
//! such as a function whose `switch` returns in every case (X3507, "not all control paths return
//! a value"); the app then composited every document on the CPU. The other GPU tests skip when the
//! compositor doesn't build, so they never noticed. Windows only; skips without a DX12 adapter.

#![cfg(windows)]

use photocraft_gpu::Compositor;

use crate::GPU_LOCK;

#[test]
fn the_compositor_builds_with_fxc_on_every_dx12_adapter() {
    let _lock = GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    // No `InstanceFlags::DEBUG`, as in the release app: in a debug build it makes wgpu pass
    // `D3DCOMPILE_SKIP_OPTIMIZATION`, and FXC's X3507 only shows up when it optimises.
    let mut desc = wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12,
        flags: wgpu::InstanceFlags::empty(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    };
    desc.backend_options.dx12.shader_compiler = wgpu::Dx12Compiler::Fxc;
    let instance = wgpu::Instance::new(desc);
    let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::DX12));
    if adapters.is_empty() {
        eprintln!("skipping: no DX12 adapter");
    }
    for adapter in adapters {
        let name = adapter.get_info().name;
        let Ok((device, _queue)) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())) else {
            eprintln!("skipping {name}: no device");
            continue;
        };
        if let Err(e) = Compositor::try_new_with_format(&device, Compositor::preferred_acc_format(&adapter)) {
            panic!("{name}: {}", e.0);
        }
    }
}
