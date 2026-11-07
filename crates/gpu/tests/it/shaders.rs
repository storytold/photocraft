//! The compositor's shaders parse and validate (naga, as wgpu does), with no GPU: an invalid
//! operator shader would otherwise only show as operators being unavailable, and the operator
//! parity tests skip on such a device.

fn validate(name: &str, src: &str) {
    let module = match wgpu::naga::front::wgsl::parse_str(src) {
        Ok(m) => m,
        Err(e) => panic!("{name}: {}", e.emit_to_string(src)),
    };
    let mut v = wgpu::naga::valid::Validator::new(wgpu::naga::valid::ValidationFlags::all(), wgpu::naga::valid::Capabilities::all());
    if let Err(e) = v.validate(&module) {
        panic!("{name}: {}", e.emit_to_string(src));
    }
}

#[test]
fn the_operator_shader_validates() {
    validate("operator.wgsl", include_str!("../../src/operator.wgsl"));
}

/// FXC, the Direct3D 12 compiler wgpu uses by default, rejects what naga accepts (a vector
/// component assigned at run time, X3500): the operator pipelines must build with it, or every
/// layer operator falls back to the CPU on Direct3D 12. Windows only; skips without a DX12 adapter.
#[cfg(windows)]
#[test]
fn the_operator_shader_builds_with_fxc() {
    let _lock = crate::GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
    desc.backends = wgpu::Backends::DX12;
    // As a release build compiles: with the debug-build default flags, wgpu asks FXC to skip
    // optimisation, and FXC then accepts shaders a release build rejects (#8734).
    desc.flags = wgpu::InstanceFlags::empty();
    desc.backend_options.dx12.shader_compiler = wgpu::Dx12Compiler::Fxc;
    let Ok(adapter) = pollster::block_on(wgpu::Instance::new(desc).request_adapter(&wgpu::RequestAdapterOptions::default())) else {
        eprintln!("skipping: no DX12 adapter");
        return;
    };
    // With an adapter, a device that can't be made is a failure, not a skip. The operator
    // pipelines are built on their own: the compositor's own FXC failure is #8733 (#8734).
    let (device, _queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("a DX12 device");
    assert!(photocraft_gpu::Compositor::operator_pipelines_build(&device), "the operator pipelines build with FXC");
}
