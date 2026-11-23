//! The compositor's WGSL must compile with the platform's own shader compiler, not only with naga.
//!
//! On DX12 wgpu translates the WGSL to HLSL and compiles it with FXC, unless `dxcompiler.dll`
//! happens to be on `PATH`, where it prefers DXC — which is why the first instance below forces
//! FXC. FXC's flow analysis stops following shapes WGSL allows (a switch whose every case returns,
//! past thirteen cases) and reports `error X3507: 'blend_channel': Not all control paths return a
//! value`. naga validates that same shader, so nothing caught it but the real compiler: no
//! compositor pipeline built, and DX12 users composited every document on the CPU for the session.
//!
//! The Windows CI runners have no GPU: their adapter is WARP over DX12, so this runs FXC there.

use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::Document;
use photocraft_geom::Size;
use photocraft_gpu::{Compositor, render_to_vec};

use crate::GPU_LOCK;

#[test]
fn every_compositor_pipeline_builds() {
    let _lock = GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    // FXC first: it is the compiler that rejected the shader, and the lowest shader model wgpu has.
    // Every backend then covers the rest of their own translations (Vulkan, Metal, DX12 through DXC).
    // `InstanceFlags::DEBUG` (the debug-build default) makes wgpu pass `D3DCOMPILE_DEBUG |
    // D3DCOMPILE_SKIP_OPTIMIZATION`, and FXC then accepts what it rejects in a release build, so
    // the flags are cleared here: a release build is what ships.
    let describe = |backends: wgpu::Backends, compiler: wgpu::Dx12Compiler| {
        let mut desc = wgpu::InstanceDescriptor { backends, flags: wgpu::InstanceFlags::empty(), ..wgpu::InstanceDescriptor::new_without_display_handle() };
        desc.backend_options.dx12.shader_compiler = compiler;
        desc
    };
    // One instance per backend: `Backends::all()` only enables the others on one instance, and
    // `request_adapter` then returns the single default adapter, so the other backends' own
    // translation went untested. One adapter per backend keeps a second, unrelated adapter of the
    // same backend (a dual-GPU laptop's integrated GPU) out of the run. The second pass names DXC
    // rather than `Auto`: without `dxcompiler.dll`, as on the Windows runners, `Auto` falls back to
    // FXC and built every pipeline with FXC a second time (#15929), while a DXC that does not load
    // leaves DX12 out of that pass.
    for (backends, compiler) in [(wgpu::Backends::DX12, wgpu::Dx12Compiler::Fxc), (wgpu::Backends::all(), wgpu::Dx12Compiler::default_dynamic_dxc())] {
        let required = cfg!(windows) && matches!(compiler, wgpu::Dx12Compiler::Fxc);
        for backend in backends.iter() {
            let instance = wgpu::Instance::new(describe(backend, compiler.clone()));
            // The Windows runners always have a WARP adapter on DX12, so a missing one there means
            // the check this test exists for silently did not run. Any other backend is a plain
            // skip: `Backends::all()` also names backends this platform does not provide, and
            // DX12 through DXC needs a `dxcompiler.dll`.
            let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
                assert!(!required, "no adapter on Windows: the DX12/FXC pipeline check did not run");
                continue;
            };
            let Ok((device, _queue)) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())) else {
                assert!(!required, "no device on Windows: the DX12/FXC pipeline check did not run");
                continue;
            };
            let info = adapter.get_info();
            let format = Compositor::preferred_acc_format(&adapter);
            // Draws build only the pipelines they use; this builds every one.
            // GLES shares one adapter-context lock. Building every pipeline on
            // available_parallelism() threads panics with "Could not lock adapter
            // context. This is most-likely a deadlock." on the artcraft k8s runners,
            // which expose both Vulkan and GL on llvmpipe and can report dozens of CPUs.
            let threads = match info.backend {
                wgpu::Backend::Gl => 1,
                _ => std::thread::available_parallelism().map_or(1, std::num::NonZero::get),
            };
            if let Err(e) = Compositor::try_new_with_format_threads(&device, format, threads) {
                panic!("{:?} adapter {} (driver {}): {e}", info.backend, info.name, info.driver_info);
            }
        }
    }
}

/// #15439: a compositor builds a pipeline when a draw first needs it, so a process that draws one
/// layer compiles a few shaders, not all of them (FXC on the Windows runners' WARP adapter took
/// 3 to 7.5 s per process for all of them). Siblings share what any of them built.
#[test]
fn a_draw_builds_only_the_pipelines_it_uses() {
    let _lock = GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let instance = crate::instance();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
        return eprintln!("skipping: no adapter");
    };
    let Ok((device, queue)) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())) else {
        return eprintln!("skipping: no device");
    };
    let mut comp = match Compositor::try_new_with_format(&device, Compositor::preferred_acc_format(&adapter)) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipping: {e}"),
    };
    let (built, all) = comp.pipeline_counts();
    assert_eq!(built, 0, "creating a compositor builds no pipeline");
    let doc = Document::with_background("one layer", Size::new(16, 16), ColorMode::Rgb, SampleType::U8, Color::rgba(0.25, 0.5, 0.75, 1.0));
    render_to_vec(&mut comp, &device, &queue, &doc, doc.bounds()).expect("one layer renders on the GPU");
    let (one_layer, _) = comp.pipeline_counts();
    assert!(one_layer > 0 && one_layer * 4 < all, "one layer built {one_layer} of {all} pipelines");
    // Drawing it again, or from a sibling, builds nothing new.
    render_to_vec(&mut comp, &device, &queue, &doc, doc.bounds()).expect("second render");
    let mut sibling = comp.sibling();
    render_to_vec(&mut sibling, &device, &queue, &doc, doc.bounds()).expect("sibling render");
    assert_eq!(sibling.pipeline_counts(), (one_layer, all));
}
