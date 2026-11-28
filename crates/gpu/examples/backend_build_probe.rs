//! Reproduces `pipelines::every_compositor_pipeline_builds`: one process, one adapter per backend,
//! several of them with a parallel pipeline build each.

use photocraft_gpu::Compositor;

fn main() {
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    for backends in [wgpu::Backends::DX12, wgpu::Backends::all()] {
        for backend in backends.iter() {
            let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
            descriptor.backends = backend;
            descriptor.flags = wgpu::InstanceFlags::empty();
            let instance = wgpu::Instance::new(descriptor);
            let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
                println!("skip {backend:?}: no adapter");
                continue;
            };
            let Ok((device, _queue)) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())) else {
                println!("skip {backend:?}: no device");
                continue;
            };
            let info = adapter.get_info();
            let format = Compositor::preferred_acc_format(&adapter);
            let started = std::time::Instant::now();
            let built = Compositor::try_new_with_format_threads_on_backend(&device, info.backend, format, threads);
            println!("{backend:?} adapter={} driver={} threads={threads} built={} in {:?}", info.name, info.driver_info, built.is_ok(), started.elapsed());
        }
    }
}
