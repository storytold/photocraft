use super::{PhotocraftApp, gpu_canvas, gpu_status};

impl PhotocraftApp {
    /// Draw the document canvas on the GPU (custom WGSL shader) instead of via egui textures.
    /// Call from the app creator with `cc.wgpu_render_state`; without it the CPU path is used.
    pub fn set_wgpu(&mut self, rs: eframe::egui_wgpu::RenderState) {
        // Preferences › Performance › cache tile size (PHOTOCRAFT_GPU_TILE still overrides).
        let tile = self.session.prefs().performance.cache_tile_size;
        // Escaped driver/setup panics must leave the session and CPU canvas alive.
        self.perf.gpu_info.set_adapter(&rs.adapter.get_info());
        // The browser has no adapter list (egui-wgpu only has it natively).
        #[cfg(not(target_arch = "wasm32"))]
        self.perf.gpu_info.set_available(&rs.available_adapters);
        self.gpu_render_state = Some(rs.clone());
        let gpu = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| gpu_canvas::GpuCanvas::with_tile(&rs, Some(tile)))) {
            Ok(gpu) => gpu,
            Err(payload) => {
                let detail = payload
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
                    .unwrap_or_else(|| "GPU canvas initialization failed".into());
                self.perf.gpu_info.canvas = "cpu".into();
                self.perf.gpu_info.fallback = Some(detail.clone());
                gpu_status::queue_fallback_notice(self, detail);
                self.watch_window_device(&rs.device);
                return;
            }
        };
        self.perf.gpu_info.canvas = "gpu".into();
        self.window_device = Some((rs.device.clone(), gpu.health().clone()));
        self.gpu = Some(gpu);
        self.prefs_rt.gpu_style = None;
    }

    /// Build the GPU compositor in the background, so the first document doesn't wait for its
    /// pipelines to compile: on DX12, FXC compiles them one at a time, which froze the app for
    /// seconds the first time a file was opened (#6419, #8733). The canvas draws with the CPU
    /// compositor until the build is done, and redraws on the GPU then.
    ///
    /// Does nothing without a GPU canvas (and in the browser, which has no threads).
    pub fn warm_up_gpu_compositor(&mut self, ctx: &egui::Context) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(gpu) = &self.gpu {
            gpu.warm_up(ctx);
        }
        #[cfg(target_arch = "wasm32")]
        let _ = ctx;
    }

    /// Whether the document canvas currently draws on the GPU (false on the CPU path, and after
    /// the device was lost).
    pub fn gpu_active(&self) -> bool {
        self.gpu.is_some()
    }

    /// The GPU canvas's device health flag (tests inject faults through it).
    pub fn gpu_health(&self) -> Option<photocraft_gpu::DeviceHealth> {
        self.gpu.as_ref().map(|g| g.health().clone())
    }

    /// Watch the device the window draws with when the GPU canvas doesn't (the CPU canvas), so
    /// its loss is handled like the GPU canvas's (see `gpu_status::check_window`).
    pub fn watch_window_device(&mut self, device: &eframe::wgpu::Device) {
        self.window_device = Some((device.clone(), photocraft_gpu::DeviceHealth::watch(device)));
    }

    /// Whether the window's graphics device was lost, so the app saves its documents for
    /// recovery and closes once they are saved (#3438).
    pub fn window_lost(&self) -> bool {
        self.window_lost
    }

    /// Destroy the window's graphics device, as a driver reset does (`ui.gpu.simulateLoss`
    /// with `destroy`, tests). Returns whether there was one.
    pub fn destroy_window_device(&mut self) -> bool {
        self.window_device.as_ref().map(|(device, _)| device.destroy()).is_some()
    }

    /// Run `hook` once the app has rendered its first frames (and a document opened at launch
    /// has drawn): the desktop app clears its crash-safe GPU startup marker there.
    pub fn on_started(&mut self, hook: impl FnOnce(&mut PhotocraftApp) + 'static) {
        self.started = Some(Box::new(hook));
    }

    /// Per-frame GPU health check (called from `logic`; tests call it directly): falls back to
    /// the CPU canvas after a device loss and runs the started hook.
    pub fn check_gpu(&mut self, ctx: &egui::Context) {
        gpu_status::check(self, ctx);
    }
}
