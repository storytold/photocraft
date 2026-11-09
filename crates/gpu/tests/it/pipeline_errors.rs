//! #6419: the compositor's pipelines are built on several threads. A pipeline the device can't
//! build is still an `Err` from `try_new_with_format`, not a panic from wgpu's default error
//! handler (error scopes are per thread). Skips when no GPU adapter exists.

use std::sync::Arc;

use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::Document;
use photocraft_geom::Size;
use photocraft_gpu::{Compositor, render_to_vec};

use crate::GPU_LOCK;

fn device() -> Option<(wgpu::Adapter, wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
        eprintln!("skipping: no adapter");
        return None;
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
    Some((adapter, device, queue))
}

#[test]
fn a_pipeline_built_on_another_thread_fails_as_an_error() {
    let _lock = GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((adapter, device, _queue)) = device() else { return };
    // Never a render target, so every accumulation pipeline fails validation.
    let Err(e) = Compositor::try_new_with_format(&device, wgpu::TextureFormat::Rgb9e5Ufloat) else {
        panic!("a compositor accumulating into Rgb9e5Ufloat");
    };
    assert!(e.0.contains("couldn't build the compositor's pipelines"), "{e:?}");
    // The device is still usable for a compositor it can build.
    assert!(Compositor::try_new_with_format(&device, Compositor::preferred_acc_format(&adapter)).is_ok());
}

/// `new_with_format` on a device whose errors go to the caller's handler: a pipeline that failed
/// to build stays in the compositor as an invalid handle, so a draw reports GPU errors instead of
/// panicking on a missing pipeline.
#[test]
fn a_compositor_with_failed_pipelines_draws_without_panicking() {    let _lock = GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((_adapter, device, queue)) = device() else { return };
    // As the app does (`DeviceHealth::watch`): GPU errors go to a handler, not wgpu's default panic.
    device.on_uncaptured_error(Arc::new(|_: wgpu::Error| {}));
    let mut comp = Compositor::new_with_format(&device, wgpu::TextureFormat::Rgb9e5Ufloat);
    let doc = Document::with_background("t", Size::new(8, 8), ColorMode::Rgb, SampleType::U8, Color::rgba(0.5, 0.5, 0.5, 1.0));
    // Drawing into an unrenderable format can't succeed, but must not panic.
    let _ = render_to_vec(&mut comp, &device, &queue, &doc, doc.bounds());
}

/// The compositor's pipelines can be built on a dedicated build thread pool (#9969): building
/// there renders exactly like the synchronous build, with the adapter's own format and on any
/// backend that the synchronously built compositor renders on. The instance follows the
/// environment (so `WGPU_BACKEND=gl` exercises the GL build, which the suite forces to one
/// thread: `parallel_pipelines`).
#[test]
fn a_compositor_built_off_the_ui_thread_renders() {
    let _lock = GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    let format = Compositor::preferred_acc_format(&adapter);
    let mut threaded = match Compositor::try_new_with_format_threads(&device, format, 4) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipping: {e}"),
    };
    let mut sync = Compositor::try_new_with_format(&device, format).unwrap();
    let doc = Document::with_background("t", Size::new(24, 18), ColorMode::Rgb, SampleType::U8, Color::rgba(0.5, 0.5, 0.5, 1.0));
    let a = render_to_vec(&mut threaded, &device, &queue, &doc, doc.bounds()).unwrap();
    let b = render_to_vec(&mut sync, &device, &queue, &doc, doc.bounds()).unwrap();
    assert_eq!(a, b, "the threaded build renders like the synchronous one");
    let cpu = photocraft_compose::flatten(&doc);
    assert!(
        a.iter().zip(&cpu.px).all(|(p, c)| (0..4).all(|k| (p[k] - c[k]).abs() <= 1.0 / 255.0)),
        "the background is drawn"
    );
}

/// #9173: the compositor builds its pipelines on several threads (#6419). On Direct3D 12 with FXC,
/// wgpu-hal caches each compiled shader as an `ID3DBlob` whose reference count isn't atomic
/// (gfx-rs/wgpu#10547), so two threads building pipelines from the same shader could free a blob
/// the cache still holds; dropping the device then aborted the process (`null reference
/// produced` in `IUnknown::drop`). Each thread now builds from shaders no other thread uses.
///
/// Stress test, ignored by default (it builds many compositors). Run it on WARP with FXC, e.g.
/// `WGPU_BACKEND=dx12 WGPU_DX12_COMPILER=fxc WGPU_ADAPTER_NAME="basic render"
/// PC_BUILD_ROUNDS=10 PC_BUILDS_PER_DEVICE=50
/// cargo test -p photocraft-gpu --test it parallel_compositor_builds -- --ignored`.
/// Windows' full page heap makes the race abort more reliably.
#[test]
#[ignore = "stress test: builds many compositors"]
fn parallel_compositor_builds_leave_the_shader_cache_intact() {
    let _lock = GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let var = |name: &str, default: u32| std::env::var(name).ok().and_then(|v| v.parse::<u32>().ok()).filter(|&n| n > 0).unwrap_or(default);
    let (rounds, builds) = (var("PC_BUILD_ROUNDS", 4), var("PC_BUILDS_PER_DEVICE", 8));
    for round in 0..rounds {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let Ok(adapter) = pollster::block_on(wgpu::util::initialize_adapter_from_env_or_default(&instance, None)) else {
            eprintln!("skipping: no adapter");
            return;
        };
        if round == 0 {
            let info = adapter.get_info();
            eprintln!("shader cache stress: {:?} ({}), {rounds} devices x {builds} builds", info.backend, info.name);
        }
        let Ok((device, _queue)) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())) else {
            eprintln!("skipping: no device");
            return;
        };
        let format = Compositor::preferred_acc_format(&adapter);
        // The first build compiles every shader; the rest are cache hits, so their threads take
        // and release references to the cached shaders densely. The builds start together, so
        // overlapping `Compositor::build` calls are exercised too (the process-wide lock).
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..builds)
                .map(|_| {
                    scope.spawn(|| {
                        let built = Compositor::try_new_with_format(&device, format);
                        assert!(built.is_ok(), "round {round}: {:?}", built.err());
                    })
                })
                .collect();
            for handle in handles {
                handle.join().expect("compositor build thread panicked");
            }
        });
        // Dropping the device releases every cached blob: a freed one aborts here.
    }
}
