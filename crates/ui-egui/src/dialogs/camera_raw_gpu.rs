//! Camera Raw's preview develop on the GPU (#6916). The proxy develops in wgpu passes
//! (`photocraft_gpu::CameraRawDevelop`) on the canvas's device. Without a selection or the
//! open-time re-tone, and with a display transform the GPU can apply, the frame stays there: it is
//! converted for the preview into a texture egui draws and counted into the histogram, and its
//! pixels are read back only when the scopes, clipping warnings or samplers need them
//! ([`CameraRawGpu::develop_view`]). Otherwise it is read back once for the histogram, scopes and
//! display conversion. Calibration, Rendering Mode CPU,
//! `PHOTOCRAFT_CPU_CAMERA_RAW`, no GPU canvas, a software adapter, a device fault or a failed
//! develop all keep the CPU develop on the worker.
//!
//! The pipelines build on a thread the first time a dialog opens (DX12's shader compiler takes
//! about a second, #6419); frames develop on the CPU until they are ready.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use eframe::wgpu;
use photocraft_algo::camera_raw::CameraRaw;
use photocraft_gpu::{CameraRawDevelop, DeviceHealth};

use crate::PhotocraftApp;

/// The built pass, shared by every Camera Raw dialog on the device (never built on the web).
#[derive(Clone)]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) struct Pass(Arc<Mutex<CameraRawDevelop>>);

/// A device's pass: unset while it builds, `None` when the device can't run it.
pub(crate) type Slot = Arc<OnceLock<Option<Pass>>>;

impl Pass {
    /// Whether `adapter` may run the pass: not a software adapter (the CPU develop is faster
    /// there), one that renders `Rgba32Float`, and not on the web (no blocking readback yet).
    fn usable(adapter: &wgpu::Adapter) -> bool {
        if cfg!(target_arch = "wasm32") {
            return false;
        }
        if adapter.get_info().device_type == wgpu::DeviceType::Cpu {
            log::info!("RAW Develop develops on the CPU: software graphics adapter");
            return false;
        }
        if !CameraRawDevelop::supported(adapter) {
            log::info!("RAW Develop develops on the CPU: the adapter can't render Rgba32Float");
            return false;
        }
        true
    }

    /// Builds the pass into `slot`, on a thread where there are threads.
    pub(crate) fn start(adapter: &wgpu::Adapter, device: &wgpu::Device, slot: &Slot) {
        if !Self::usable(adapter) {
            let _ = (slot.set(None), device);
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (on, out) = (device.clone(), slot.clone());
            let spawned = std::thread::Builder::new().name("camera-raw-pipeline".into()).spawn(move || {
                let pass = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Self::build_on(&on))).unwrap_or_else(|_| {
                    log::warn!("building the RAW Develop pipeline panicked; RAW Develop develops on the CPU");
                    None
                });
                let _ = out.set(pass);
            });
            if spawned.is_err() {
                let _ = slot.set(Self::build_on(device));
            }
        }
    }

    /// The pass on `device`, whatever its adapter.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn build_on(device: &wgpu::Device) -> Option<Self> {
        match CameraRawDevelop::new(device) {
            Ok(pass) => Some(Pass(Arc::new(Mutex::new(pass)))),
            Err(e) => {
                log::warn!("{e}; RAW Develop develops on the CPU");
                None
            }
        }
    }
}

/// The dialog's GPU develop, if the app may use one.
pub(crate) fn for_app(app: &PhotocraftApp) -> Option<CameraRawGpu> {
    let cpu = app.session.prefs().performance.effective_rendering_mode() == photocraft_engine::prefs::RenderingMode::Cpu;
    if cpu || std::env::var_os("PHOTOCRAFT_CPU_CAMERA_RAW").is_some() {
        return None;
    }
    app.gpu.as_ref()?.camera_raw()
}

/// A key for a dialog's proxy pixels, unique in the process: the pass keeps the last source it
/// uploaded, and a new dialog's proxy may reuse a closed one's memory.
pub(crate) fn source_key() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// The source key and the settings the black and white points depend on.
type PointsKey = (u64, [u32; 5]);
/// The last [`PointsKey`] and its points.
type Points = Option<(PointsKey, Option<(f32, f32)>)>;

/// One dialog's handle on the pass (cheap to clone into each proxy job).
#[derive(Clone)]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) struct CameraRawGpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    health: DeviceHealth,
    pass: Slot,
    /// Set after a develop failed: the dialog stays on the CPU.
    failed: Arc<AtomicBool>,
    /// Whites and Blacks adapt to the image; its points change only with these settings.
    points: Arc<Mutex<Points>>,
    /// The pass's slot the preview shows ([`NONE_SHOWN`] when it shows a CPU image): the next
    /// frame develops into the other one.
    shown: Arc<AtomicUsize>,
    /// The display transform as the GPU applies it, for the display key it was made for.
    shaper: Arc<Mutex<Shaper>>,
    textures: Arc<Textures>,
    /// The buffer frames kept on the GPU are read back through.
    readback: Arc<Mutex<Option<wgpu::Buffer>>>,
}

/// A frame kept on the GPU ([`CameraRawGpu::develop_view`]); none on the web, which has no
/// blocking readback for its pixels.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) type GpuFrame = photocraft_gpu::CameraRawFrame;
#[cfg(target_arch = "wasm32")]
#[derive(Clone)]
pub(crate) enum GpuFrame {}

/// The pass's slot `frame` is in.
pub(crate) fn slot(frame: &GpuFrame) -> usize {
    #[cfg(target_arch = "wasm32")]
    match *frame {}
    #[cfg(not(target_arch = "wasm32"))]
    frame.slot
}

/// egui's renderer, shared with the canvas.
pub(crate) type Renderer = Arc<egui::mutex::RwLock<eframe::egui_wgpu::Renderer>>;

/// The display key and its transform's matrix/shaper (`None`: not one).
type Shaper = Option<(u64, Option<photocraft_cms::GpuMatrixShaper>)>;

/// [`CameraRawGpu::shown`] when no slot is on screen.
const NONE_SHOWN: usize = usize::MAX;

/// The pass's slots registered with egui, with the generation of the slot's textures. Freed
/// with the dialog's last handle.
struct Textures {
    renderer: Renderer,
    ids: Mutex<[Option<(u64, egui::TextureId)>; 2]>,
}

impl Drop for Textures {
    fn drop(&mut self) {
        let ids = self.ids.get_mut().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut renderer = self.renderer.write();
        for (_, id) in ids.iter().flatten() {
            renderer.free_texture(id);
        }
    }
}

impl CameraRawGpu {
    pub(crate) fn new(device: wgpu::Device, queue: wgpu::Queue, health: DeviceHealth, pass: Slot, renderer: Renderer) -> Self {
        CameraRawGpu {
            device,
            queue,
            health,
            pass,
            failed: Arc::default(),
            points: Arc::default(),
            shown: Arc::new(AtomicUsize::new(NONE_SHOWN)),
            shaper: Arc::default(),
            textures: Arc::new(Textures { renderer, ids: Mutex::new([None, None]) }),
            readback: Arc::default(),
        }
    }

    /// Develops `px` like [`Self::develop`] but keeps the frame on the GPU, shown through
    /// `display`. `None` when [`Self::develop`] would be, and when `display`'s transform isn't a
    /// matrix/shaper (a CLUT or proofing profile): then the frame is read back and converted on
    /// the CPU. With `read` the pixels come back in the same submission
    /// (`CameraRawFrame::read_back`). Native only: the web can't read a frame back when the
    /// scopes need it.
    #[cfg(not(target_arch = "wasm32"))]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn develop_view(
        &self,
        px: &[[f32; 4]],
        key: Option<u64>,
        w: usize,
        h: usize,
        p: &CameraRaw,
        float: bool,
        display: &photocraft_engine::display_color::CanvasDisplay,
        read: bool,
    ) -> Option<GpuFrame> {
        if p.is_identity() || self.failed.load(Ordering::Relaxed) || !self.health.is_ok() {
            return None;
        }
        let Some(Some(pass)) = self.pass.get() else { return None };
        let shaper = {
            let mut cache = self.shaper.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if cache.as_ref().is_none_or(|(k, _)| *k != display.key) {
                *cache = Some((display.key, display.transform.as_ref().and_then(|t| t.gpu_matrix_shaper())));
            }
            match (&display.transform, cache.as_ref().and_then(|(_, s)| s.clone())) {
                (None, _) => None,
                (Some(_), Some(s)) => Some(s),
                (Some(_), None) => return None,
            }
        };
        let view = photocraft_gpu::CameraRawDisplay {
            encode_srgb: display.encode_srgb,
            shaper: shaper.as_ref().map(|s| photocraft_gpu::CameraRawShaper { matrix: s.matrix_rows(), input: s.input_table(), output: s.output_table() }),
            key: display.key,
        };
        let points = self.points(px, key, w, h, p);
        let stages = photocraft_algo::camera_raw::per_pixel(p, w, h, float, points)?;
        let size = [u32::try_from(w).ok()?, u32::try_from(h).ok()?];
        let avoid = Some(self.shown.load(Ordering::Acquire)).filter(|s| *s != NONE_SHOWN);
        let mut pass = pass.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        match pass.develop_view(&self.device, &self.queue, Some(&self.health), px, key, size, &stages, &view, avoid, read) {
            Ok(frame) => Some(frame),
            Err(e) => {
                log::warn!("{}; RAW Develop develops on the CPU", e.0);
                self.failed.store(true, Ordering::Relaxed);
                None
            }
        }
    }

    /// Notes which slot the preview shows (`None`: a CPU image), so the next frame develops into
    /// the other.
    pub(crate) fn show(&self, slot: Option<usize>) {
        self.shown.store(slot.unwrap_or(NONE_SHOWN), Ordering::Release);
    }

    /// `frame`'s display texture as egui knows it, registered the first time its slot is shown
    /// and again when the slot's textures are new.
    pub(crate) fn texture_id(&self, frame: &GpuFrame) -> egui::TextureId {
        #[cfg(target_arch = "wasm32")]
        match *frame {}
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut ids = self.textures.ids.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut renderer = self.textures.renderer.write();
            let filter = wgpu::FilterMode::Linear;
            match ids.get_mut(frame.slot) {
                Some(Some((generation, id))) => {
                    if *generation != frame.generation {
                        renderer.update_egui_texture_from_wgpu_texture(&self.device, &frame.shown, filter, *id);
                        *generation = frame.generation;
                    }
                    *id
                }
                Some(entry) => {
                    let id = renderer.register_native_texture(&self.device, &frame.shown, filter);
                    *entry = Some((frame.generation, id));
                    id
                }
                None => renderer.register_native_texture(&self.device, &frame.shown, filter),
            }
        }
    }

    /// A frame's developed pixels (blocking), or `None` after a GPU error.
    pub(crate) fn read(&self, frame: &GpuFrame) -> Option<Vec<[f32; 4]>> {
        #[cfg(target_arch = "wasm32")]
        match *frame {}
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut buffer = self.readback.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            match photocraft_gpu::read_camera_raw_pixels(&self.device, &self.queue, Some(&self.health), &frame.pixels, &mut buffer) {
                Ok(px) => Some(px),
                Err(e) => {
                    log::warn!("{}; RAW Develop develops this frame on the CPU", e.0);
                    None
                }
            }
        }
    }

    /// Develops `px` (`w × h`) with `p`, or `None` when the CPU must: Calibration, the pass still
    /// building, a fault, or a GPU error (logged; the dialog then stays on the CPU).
    /// `key` names the pixels when they are the dialog's unchanged proxy ([`source_key`]), so they
    /// upload once. Neutral settings are `None` too: the CPU develop returns them at once, a GPU
    /// round trip would only cost.
    pub(crate) fn develop(&self, px: &[[f32; 4]], key: Option<u64>, w: usize, h: usize, p: &CameraRaw, float: bool) -> Option<Vec<[f32; 4]>> {
        if p.is_identity() || self.failed.load(Ordering::Relaxed) || !self.health.is_ok() {
            return None;
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (px, key, w, h, p, float);
            None
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(Some(pass)) = self.pass.get() else { return None };
            let points = self.points(px, key, w, h, p);
            let stages = photocraft_algo::camera_raw::per_pixel(p, w, h, float, points)?;
            let size = [u32::try_from(w).ok()?, u32::try_from(h).ok()?];
            let mut pass = pass.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            match pass.develop(&self.device, &self.queue, Some(&self.health), px, key, size, &stages) {
                Ok(out) => Some(out),
                Err(e) => {
                    log::warn!("{}; RAW Develop develops on the CPU", e.0);
                    self.failed.store(true, Ordering::Relaxed);
                    None
                }
            }
        }
    }

    /// The image's black and white points for Whites and Blacks, measured on the CPU. With a
    /// spatial stage before them the GPU measures them after it instead (Detail comes after).
    #[cfg(not(target_arch = "wasm32"))]
    fn points(&self, px: &[[f32; 4]], key: Option<u64>, w: usize, h: usize, p: &CameraRaw) -> Option<(f32, f32)> {
        if (p.whites == 0.0 && p.blacks == 0.0) || p.spatial_tone() {
            return None;
        }
        let Some(key) = key else { return photocraft_algo::camera_raw::white_black_points(px, w, h, p) };
        let key = (key, [p.temperature, p.tint, p.exposure, p.contrast, p.encoding_gamma].map(f32::to_bits));
        let mut cache = self.points.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((k, points)) = *cache
            && k == key
        {
            return points;
        }
        let points = photocraft_algo::camera_raw::white_black_points(px, w, h, p);
        *cache = Some((key, points));
        points
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_algo::camera_raw::develop;
    use serde_json::json;

    /// The pass evaluates develop's expressions in f32 (see `photocraft-gpu`'s Camera Raw tests).
    const TOL: f32 = 1.0 / 4096.0;

    /// The app on egui_kittest's device (a software adapter where there is one), or `None`.
    fn gpu_app(depth: u32) -> Option<(PhotocraftApp, egui::Context)> {
        let Ok(rs) = std::panic::catch_unwind(|| egui_kittest::wgpu::create_render_state(crate::gpu_canvas::wgpu_setup(), Default::default())) else {
            eprintln!("skipping: no GPU adapter");
            return None;
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.set_wgpu(rs);
        app.gpu.as_ref()?;
        app.run("file.new", json!({"width": 300, "height": 200, "depth": depth})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("filter.render.clouds", json!({"seed": 5})).unwrap();
        app.run("select.rect", json!({"x": 40, "y": 30, "width": 120, "height": 90})).unwrap();
        app.run("edit.fill", json!({"color": "#cc4422"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        Some((app, egui::Context::default()))
    }

    /// The canvas's pass, built even on a software adapter (which the app leaves on the CPU).
    fn force_pass(app: &PhotocraftApp) {
        app.gpu.as_ref().expect("GPU canvas").force_camera_raw_pass();
    }

    fn developed(app: &mut PhotocraftApp, ctx: &egui::Context, p: CameraRaw) -> (&'static str, Vec<[f32; 4]>, Vec<[f32; 4]>) {
        let d = app.camera_raw.as_mut().expect("dialog");
        d.params = p;
        d.render(ctx);
        let mut cpu = d.proxy.to_vec();
        let mut q = d.params.clone();
        q.pixel_scale = (d.pw as f32 / d.full_w.max(1) as f32).min(1.0);
        develop(&mut cpu, d.pw, d.ph, &q, d.float);
        (d.develop_backend, d.processed.pixels().to_vec(), cpu)
    }

    #[test]
    fn the_pass_is_built_when_camera_raw_first_opens_not_with_the_canvas() {
        let _gpu = photocraft_testkit::gpu_lock();
        let Some((mut app, ctx)) = gpu_app(8) else { return };
        let canvas = app.gpu.clone().expect("GPU canvas");
        assert!(!canvas.camera_raw_started(), "no RAW Develop pipeline at launch (#6419)");
        crate::camera_raw_ui::open(&mut app, &ctx).unwrap();
        assert!(canvas.camera_raw_started(), "built (or refused) from the first open");
        assert_eq!(app.camera_raw.as_ref().map(|d| d.develop_backend), Some("cpu"), "the first frame doesn't wait for the pipeline");
        canvas.wait_camera_raw_pass();
        let software = canvas.adapter_info().device_type == wgpu::DeviceType::Cpu;
        let d = app.camera_raw.as_ref().expect("dialog");
        assert_eq!(d.gpu.is_none(), software, "a software adapter keeps the CPU develop");
    }

    #[test]
    fn every_stage_but_calibration_develops_on_the_gpu_like_the_cpu() {
        let _gpu = photocraft_testkit::gpu_lock();
        for depth in [8, 16, 32] {
            let Some((mut app, ctx)) = gpu_app(depth) else { return };
            force_pass(&app);
            crate::camera_raw_ui::open(&mut app, &ctx).unwrap();
            if app.camera_raw.as_ref().is_some_and(|d| d.gpu.is_none()) {
                eprintln!("skipping: the device can't run the RAW Develop pass");
                return;
            }
            let per_pixel = CameraRaw {
                temperature: 20.0,
                exposure: 0.4,
                contrast: 30.0,
                whites: 25.0,
                blacks: -20.0,
                vibrance: 40.0,
                curve_lights: 25.0,
                grade_shadows: photocraft_algo::camera_raw::Wheel { hue: 220.0, sat: 40.0, lum: 0.0 },
                vignette_amount: -40.0,
                grain_amount: 30.0,
                ..Default::default()
            };
            let (backend, gpu, cpu) = developed(&mut app, &ctx, per_pixel.clone());
            assert_eq!(backend, "gpu", "{depth}-bit");
            let worst = gpu.iter().zip(&cpu).flat_map(|(a, b)| (0..4).map(move |c| (a[c] - b[c]).abs())).fold(0.0f32, f32::max);
            assert!(worst <= TOL, "{depth}-bit: off by {worst}");
            let described = app.camera_raw.as_ref().map(|d| d.describe(&app.ui.camera_raw_scope, &app.ui.camera_raw_preview)).unwrap();
            assert_eq!(described["developBackend"], "gpu");
            // Shadows and Highlights develop on the GPU too, Whites and Blacks measured after them.
            let toned = CameraRaw { shadows: 40.0, highlights: -35.0, ..per_pixel.clone() };
            let (backend, gpu, cpu) = developed(&mut app, &ctx, toned.clone());
            assert_eq!(backend, "gpu", "{depth}-bit, Shadows and Highlights");
            let worst = gpu.iter().zip(&cpu).flat_map(|(a, b)| (0..4).map(move |c| (a[c] - b[c]).abs())).fold(0.0f32, f32::max);
            assert!(worst <= TOL, "{depth}-bit, Shadows and Highlights: off by {worst}");
            // So do Dehaze, Clarity and Texture, with Whites and Blacks measured after them.
            for spatial in [
                CameraRaw { clarity: 30.0, texture: 25.0, dehaze: 20.0, ..toned.clone() },
                CameraRaw { dehaze: -25.0, ..per_pixel.clone() },
                CameraRaw { clarity: -40.0, texture: 35.0, ..per_pixel.clone() },
            ] {
                let (backend, gpu, cpu) = developed(&mut app, &ctx, spatial.clone());
                assert_eq!(backend, "gpu", "{depth}-bit, {spatial:?}");
                let worst = gpu.iter().zip(&cpu).flat_map(|(a, b)| (0..4).map(move |c| (a[c] - b[c]).abs())).fold(0.0f32, f32::max);
                assert!(worst <= TOL, "{depth}-bit, {spatial:?}: off by {worst}");
            }
            // And Detail: sharpening and noise reduction, alone (Whites and Blacks measured on
            // the CPU before it) and after every other stage.
            for detail in [
                CameraRaw { sharpen_amount: 60.0, sharpen_masking: 30.0, ..per_pixel.clone() },
                CameraRaw { noise_luminance: 40.0, noise_color: 50.0, ..per_pixel.clone() },
                CameraRaw { sharpen_amount: 80.0, noise_luminance: 30.0, noise_color: 40.0, clarity: 30.0, texture: 25.0, dehaze: 20.0, ..toned.clone() },
            ] {
                let (backend, gpu, cpu) = developed(&mut app, &ctx, detail.clone());
                assert_eq!(backend, "gpu", "{depth}-bit, {detail:?}");
                let worst = gpu.iter().zip(&cpu).flat_map(|(a, b)| (0..4).map(move |c| (a[c] - b[c]).abs())).fold(0.0f32, f32::max);
                assert!(worst <= TOL, "{depth}-bit, {detail:?}: off by {worst}");
            }
            // Calibration develops the whole frame on the CPU (#5128).
            let calibrated = CameraRaw { sharpen_amount: 40.0, shadow_tint: 30.0, ..per_pixel.clone() };
            let (backend, shown, cpu) = developed(&mut app, &ctx, calibrated);
            assert_eq!((backend, shown == cpu), ("cpu", true), "{depth}-bit");
            // A device fault: the CPU, from the next frame on.
            app.gpu.as_ref().unwrap().health().mark(photocraft_gpu::Fault::Lost("test".into()));
            let (backend, shown, cpu) = developed(&mut app, &ctx, per_pixel);
            assert_eq!((backend, shown == cpu), ("cpu", true), "{depth}-bit after a fault");
        }
    }

    #[test]
    fn neutral_settings_skip_the_gpu_round_trip() {
        let _gpu = photocraft_testkit::gpu_lock();
        let Some((mut app, ctx)) = gpu_app(8) else { return };
        force_pass(&app);
        crate::camera_raw_ui::open(&mut app, &ctx).unwrap();
        if app.camera_raw.as_ref().is_some_and(|d| d.gpu.is_none()) {
            eprintln!("skipping: the device can't run the RAW Develop pass");
            return;
        }
        let (backend, _, _) = developed(&mut app, &ctx, CameraRaw { exposure: 0.5, ..Default::default() });
        assert_eq!(backend, "gpu", "a per-pixel setting develops on the GPU");
        let (backend, shown, cpu) = developed(&mut app, &ctx, CameraRaw::default());
        assert_eq!((backend, shown == cpu), ("cpu", true), "default settings: the CPU returns them unchanged");
    }

    #[test]
    fn shadows_on_a_two_pixel_proxy_develop_on_the_cpu_and_keep_the_gpu() {
        let _gpu = photocraft_testkit::gpu_lock();
        let Some((mut app, ctx)) = gpu_app(8) else { return };
        force_pass(&app);
        crate::camera_raw_ui::open(&mut app, &ctx).unwrap();
        app.gpu.as_ref().expect("GPU canvas").wait_camera_raw_pass();
        let Some(gpu) = app.camera_raw.as_ref().and_then(|d| d.gpu.as_ref()) else {
            eprintln!("skipping: the device can't run the RAW Develop pass");
            return;
        };
        // A layer about 450 times wider than tall has a 2 px tall proxy: its pyramid has one level.
        let thin = vec![[0.4, 0.3, 0.2, 1.0]; 40 * 2];
        assert!(gpu.develop(&thin, None, 40, 2, &CameraRaw { shadows: 50.0, ..Default::default() }, false).is_none());
        let px = vec![[0.4, 0.3, 0.2, 1.0]; 64 * 48];
        assert!(gpu.develop(&px, None, 64, 48, &CameraRaw { exposure: 0.5, ..Default::default() }, false).is_some(), "the GPU develop stays on");
    }

    #[test]
    fn rendering_mode_cpu_keeps_the_cpu_develop() {
        let _gpu = photocraft_testkit::gpu_lock();
        let Some((mut app, ctx)) = gpu_app(8) else { return };
        force_pass(&app);
        app.run("prefs.set", json!({"path": "performance.renderingMode", "value": "cpu"})).unwrap();
        crate::camera_raw_ui::open(&mut app, &ctx).unwrap();
        let (backend, shown, cpu) = developed(&mut app, &ctx, CameraRaw { exposure: 0.5, ..Default::default() });
        assert_eq!((backend, shown == cpu), ("cpu", true));
    }

    /// Reads a frame's display texture back (`Rgba8Unorm`: the bytes egui samples as sRGB).
    fn display_bytes(gpu: &CameraRawGpu, frame: &GpuFrame) -> Vec<[u8; 4]> {
        let texture = &frame.display;
        let (w, h) = (texture.width(), texture.height());
        let row = (w * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(row) * u64::from(h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        let index = gpu.queue.submit([encoder.finish()]);
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        gpu.device.poll(wgpu::PollType::Wait { submission_index: Some(index), timeout: None }).unwrap();
        let data = buffer.slice(..).get_mapped_range().unwrap();
        (0..h as usize).flat_map(|y| data[y * row as usize..][..w as usize * 4].as_chunks::<4>().0.to_vec()).collect()
    }

    /// Without a selection the frame stays on the GPU: its histogram is the CPU's count of its
    /// pixels, its display texture holds exactly the bytes the CPU conversion makes, and the pixels
    /// come back only when a readout asks, at each depth.
    #[test]
    fn the_preview_shows_and_counts_on_the_gpu_like_the_cpu() {
        let _gpu = photocraft_testkit::gpu_lock();
        for depth in [8, 16, 32] {
            let Some((mut app, ctx)) = gpu_app(depth) else { return };
            force_pass(&app);
            crate::camera_raw_ui::open(&mut app, &ctx).unwrap();
            if app.camera_raw.as_ref().is_some_and(|d| d.gpu.is_none()) {
                eprintln!("skipping: the device can't run the RAW Develop pass");
                return;
            }
            for p in [
                CameraRaw { temperature: 20.0, exposure: 0.4, contrast: 30.0, whites: 25.0, blacks: -20.0, vibrance: 40.0, ..Default::default() },
                CameraRaw { shadows: 40.0, highlights: -35.0, clarity: 30.0, sharpen_amount: 60.0, noise_color: 40.0, ..Default::default() },
            ] {
                let d = app.camera_raw.as_mut().expect("dialog");
                d.params = p.clone();
                d.render(&ctx);
                let described = d.describe(&app.ui.camera_raw_scope, &app.ui.camera_raw_preview);
                assert_eq!(
                    (&described["developBackend"], &described["previewOnGpu"], &described["pixelsOnGpu"]),
                    (&json!("gpu"), &json!(true), &json!(true)),
                    "{depth}-bit {p:?}"
                );
                let gpu = d.gpu.clone().expect("GPU");
                let frame = d.processed.gpu_frame().expect("a frame on the GPU").clone();
                let shown = display_bytes(&gpu, &frame);
                let px = d.processed.pixels().to_vec();
                assert!(!d.processed.on_gpu(), "read back once asked");
                assert_eq!(d.after_histogram, photocraft_algo::histogram::RgbHistogram::from_rgba(&px), "{depth}-bit {p:?}");
                let image = crate::camera_raw_ui::CameraRawDialog::image(&px, d.pw, d.ph, &d.display);
                assert_eq!(shown.len(), image.pixels.len());
                for (i, (got, want)) in shown.iter().zip(&image.pixels).enumerate() {
                    assert_eq!(*got, want.to_array(), "{depth}-bit {p:?}: pixel {i}");
                }
                let mut cpu = d.proxy.to_vec();
                let q = CameraRaw { pixel_scale: (d.pw as f32 / d.full_w.max(1) as f32).min(1.0), ..p.clone() };
                develop(&mut cpu, d.pw, d.ph, &q, d.float);
                let worst = px.iter().zip(&cpu).flat_map(|(a, b)| (0..4).map(move |c| (a[c] - b[c]).abs())).fold(0.0f32, f32::max);
                assert!(worst <= TOL, "{depth}-bit {p:?}: off by {worst}");
            }
            // A sampler's readout reads the frame back too.
            let d = app.camera_raw.as_mut().expect("dialog");
            d.params.exposure = -0.3;
            d.render(&ctx);
            assert!(d.processed.on_gpu());
            let readout = d.readout([0.5, 0.5], false).expect("a readout");
            assert!(!d.processed.on_gpu());
            let (x, y) = ((0.5 * d.pw as f32) as usize, (0.5 * d.ph as f32) as usize);
            let q = d.processed.pixels()[y * d.pw + x];
            assert_eq!(readout["values"], json!([q[0] * 255.0, q[1] * 255.0, q[2] * 255.0]));
            // While something on screen reads the pixels, the develop reads them back with the
            // frame (on the worker), and still draws it from the GPU.
            d.wants_pixels = true;
            d.params.exposure = 0.2;
            d.render(&ctx);
            let described = d.describe(&app.ui.camera_raw_scope, &app.ui.camera_raw_preview);
            assert_eq!((&described["previewOnGpu"], &described["pixelsOnGpu"]), (&json!(true), &json!(false)), "{depth}-bit: read back with the frame");
        }
    }

    /// The slots take turns: the next frame develops beside the one on screen, which keeps its
    /// pixels.
    #[test]
    fn the_next_frame_develops_beside_the_one_on_screen() {
        let _gpu = photocraft_testkit::gpu_lock();
        let Some((mut app, ctx)) = gpu_app(8) else { return };
        force_pass(&app);
        crate::camera_raw_ui::open(&mut app, &ctx).unwrap();
        let d = app.camera_raw.as_mut().expect("dialog");
        let Some(gpu) = d.gpu.clone() else { return };
        let first = CameraRaw { exposure: 0.8, ..Default::default() };
        d.params = first.clone();
        d.render(&ctx);
        let shown = d.processed.gpu_frame().expect("on the GPU").clone();
        d.params = CameraRaw { exposure: -0.8, ..Default::default() };
        d.render(&ctx);
        let next = d.processed.gpu_frame().expect("on the GPU").clone();
        assert_ne!(shown.slot, next.slot);
        let kept = gpu.read(&shown).expect("the first frame");
        let mut cpu = d.proxy.to_vec();
        develop(&mut cpu, d.pw, d.ph, &CameraRaw { pixel_scale: (d.pw as f32 / d.full_w.max(1) as f32).min(1.0), ..first }, d.float);
        let worst = kept.iter().zip(&cpu).flat_map(|(a, b)| (0..4).map(move |c| (a[c] - b[c]).abs())).fold(0.0f32, f32::max);
        assert!(worst <= TOL, "the first frame kept its pixels: off by {worst}");
    }

    /// A selection blends on the CPU, so that frame is read back and converted there; a failed
    /// readback of a frame kept on the GPU develops it on the CPU instead.
    #[test]
    fn a_selection_reads_back_and_a_failed_readback_develops_on_the_cpu() {
        let _gpu = photocraft_testkit::gpu_lock();
        let Some((mut app, ctx)) = gpu_app(8) else { return };
        force_pass(&app);
        app.run("select.rect", json!({"x": 20, "y": 20, "width": 100, "height": 80})).unwrap();
        crate::camera_raw_ui::open(&mut app, &ctx).unwrap();
        if app.camera_raw.as_ref().is_some_and(|d| d.gpu.is_none()) {
            return;
        }
        let p = CameraRaw { exposure: 0.5, contrast: 20.0, ..Default::default() };
        let d = app.camera_raw.as_mut().expect("dialog");
        d.params = p.clone();
        d.render(&ctx);
        let described = d.describe(&app.ui.camera_raw_scope, &app.ui.camera_raw_preview);
        assert_eq!((&described["developBackend"], &described["previewOnGpu"]), (&json!("gpu"), &json!(false)));
        app.run("select.deselect", json!({})).unwrap();
        let mut app = gpu_app(8).map(|(a, _)| a).expect("app");
        force_pass(&app);
        crate::camera_raw_ui::open(&mut app, &ctx).unwrap();
        let d = app.camera_raw.as_mut().expect("dialog");
        d.params = p.clone();
        d.render(&ctx);
        assert!(d.processed.on_gpu());
        app.gpu.as_ref().unwrap().health().mark(photocraft_gpu::Fault::Lost("test".into()));
        let d = app.camera_raw.as_mut().expect("dialog");
        let mut cpu = d.proxy.to_vec();
        develop(&mut cpu, d.pw, d.ph, &CameraRaw { pixel_scale: (d.pw as f32 / d.full_w.max(1) as f32).min(1.0), ..p }, d.float);
        assert_eq!(d.processed.pixels(), &cpu[..], "the CPU develops the frame the GPU can't read back");
    }
}
