use super::{PhotocraftApp, gpu_canvas, gpu_status};
#[cfg(feature = "uv-workbench")]
use crate::uv_gpu;

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
        #[cfg(feature = "uv-workbench")]
        {
            match uv_gpu::Renderer::new(rs.clone(), gpu.health().clone()) {
                Ok(renderer) => self.uv_cache.gpu = Some(renderer),
                Err(error) => log::warn!("3D GPU preview unavailable: {error}"),
            }
        }
        self.gpu = Some(gpu);
        self.prefs_rt.gpu_style = None;
    }

    /// Begin preparing the compositor after the GPU canvas is initialized.
    pub fn start_compositor(&mut self, ctx: &egui::Context) {
        self.start_compositor_inner(ctx, false);
    }

    pub(crate) fn retry_compositor(&mut self, ctx: &egui::Context) {
        self.start_compositor_inner(ctx, true);
    }

    fn start_compositor_inner(&mut self, ctx: &egui::Context, retry: bool) {
        if self.session.prefs().performance.effective_rendering_mode() == photocraft_engine::prefs::RenderingMode::Cpu {
            return;
        }
        let Some(gpu) = self.gpu.clone() else { return };
        let compiler = self.services.shader_compiler.as_deref().unwrap_or("");
        let key = gpu.compositor_memo_key(compiler);
        let mut memo = self.load_gpu_memo();
        let bypass = retry || std::env::var("PHOTOCRAFT_GPU_REBUILD").is_ok_and(|v| v == "1");
        if bypass {
            if memo.remove(&key) {
                self.save_gpu_memo(&memo);
            }
        } else if let Some(entry) = memo.get(&key) {
            gpu.remember_compositor_failure(&entry.error);
            return;
        }
        gpu.start_compositor(ctx);
    }

    fn load_gpu_memo(&mut self) -> crate::gpu_memo::GpuMemo {
        self.services
            .load_gpu_memo
            .as_mut()
            .and_then(|load| load())
            .and_then(|text| crate::gpu_memo::GpuMemo::parse(&text).map_err(|e| log::info!("ignoring GPU compositor memo: {e}")).ok())
            .unwrap_or_default()
    }

    fn save_gpu_memo(&mut self, memo: &crate::gpu_memo::GpuMemo) {
        if let (Ok(text), Some(save)) = (memo.serialize(), self.services.save_gpu_memo.as_mut())
            && let Err(error) = save(&text)
        {
            log::info!("could not save GPU compositor memo: {error}");
        }
    }

    fn record_compositor_completion(&mut self) {
        let Some(gpu) = self.gpu.clone() else { return };
        let Some(result) = gpu.take_compositor_completion() else { return };
        // A completed CPU redraw can leave a current texture and a temporary build reason.
        // Recompute those documents on their next repaint, including any hidden documents.
        for id in self.render_path.compositor_building_docs() {
            if let Some(doc) = self.session.documents().iter().find(|st| st.doc.id.0 == id).map(|st| st.doc.id) {
                crate::canvas::background_redraw::cancel(self, doc);
                self.canvases.remove(&(doc, crate::canvas::GPU_OUTPUT));
            }
        }
        let compiler = self.services.shader_compiler.as_deref().unwrap_or("");
        let key = gpu.compositor_memo_key(compiler);
        let mut memo = self.load_gpu_memo();
        let changed = match result {
            Ok(()) => memo.remove(&key),
            Err((error, true)) if gpu.fault().is_none() => {
                let now: chrono::DateTime<chrono::Utc> = std::time::SystemTime::now().into();
                memo.remember(key, error, now.to_rfc3339());
                true
            }
            Err(_) => false,
        };
        if changed {
            self.save_gpu_memo(&memo);
        }
    }

    /// Whether the document canvas currently draws on the GPU (false on the CPU path, and after
    /// the device was lost).
    pub fn gpu_active(&self) -> bool {
        self.gpu.is_some()
    }

    /// Attach only the optional 3D renderer (also useful for isolated, windowless validation).
    #[cfg(feature = "uv-workbench")]
    pub fn set_uv_wgpu(&mut self, rs: eframe::egui_wgpu::RenderState) -> Result<(), String> {
        let health = self.gpu_health().unwrap_or_else(|| photocraft_gpu::DeviceHealth::watch(&rs.device));
        self.uv_cache.gpu = Some(uv_gpu::Renderer::new(rs, health)?);
        Ok(())
    }

    #[cfg(feature = "uv-workbench")]
    pub fn uv_gpu_active(&self) -> bool {
        self.uv_cache.gpu.is_some() || self.native_uv_gpu_available
    }

    /// Actual resident map dimensions, ordered by material (colour, normal).
    #[cfg(feature = "uv-workbench")]
    pub fn uv_gpu_texture_sizes(&self) -> Vec<([u32; 2], [u32; 2])> {
        self.uv_cache.gpu.as_ref().map_or_else(Vec::new, |g| g.texture_sizes())
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
        self.record_compositor_completion();
    }
}
