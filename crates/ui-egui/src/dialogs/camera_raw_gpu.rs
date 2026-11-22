//! Camera Raw's preview develop on the GPU (#6916). When every stage the settings use reads
//! only its own pixel, the proxy develops in one wgpu pass (`photocraft_gpu::CameraRawDevelop`)
//! on the canvas's device and is read back for the histogram, scopes and display conversion.
//! Dehaze, Highlights and Shadows develop on the GPU too, with their analyses on the CPU
//! (`CameraRawDevelop::develop_spatial`). Texture, Clarity, Detail, Rendering Mode CPU,
//! `PHOTOCRAFT_CPU_CAMERA_RAW`, no GPU canvas, a software adapter, a device fault or a failed
//! develop all keep the CPU develop on the worker.
//!
//! The pipeline builds on a thread the first time a dialog opens (DX12's shader compiler takes
//! about 0.35 s, #6419); frames develop on the CPU until it is ready.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use craft::imaging::raw::RawDevelopSettings as CameraRaw;
use eframe::wgpu;
use photocraft_algo::camera_raw::DevelopInputs;
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

/// The source key and the settings the black and white points depend on: the ones before
/// Contrast, Contrast, Calibration (Whites up measures after it), and whether Whites is raised
/// (the Whites tones are only measured then).
type PointsKey = (u64, [u32; 12], bool);

/// The [`PointsKey`] of `p` on the source `key`.
#[cfg(not(target_arch = "wasm32"))]
fn points_key(key: u64, p: &CameraRaw, inputs: &DevelopInputs) -> PointsKey {
    let [h0, h1, h2] = p.calibration_hue;
    let [s0, s1, s2] = p.calibration_sat;
    (key, [p.temperature, p.tint, p.exposure, p.contrast, inputs.encoding_gamma, p.shadow_tint, h0, h1, h2, s0, s1, s2].map(f32::to_bits), p.whites > 0.0)
}
/// The last [`PointsKey`] and its points.
type Points = Option<(PointsKey, Option<photocraft_algo::camera_raw::TonePoints>)>;

/// The pixels of one develop: the proxy (`w` × `h`) and, when they are the dialog's unchanged proxy, the [`source_key`]
/// that lets them upload once.
#[derive(Clone, Copy)]
pub(crate) struct ProxyPixels<'a> {
    pub(crate) px: &'a [[f32; 4]],
    pub(crate) key: Option<u64>,
    pub(crate) w: usize,
    pub(crate) h: usize,
}

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
    /// Contrast and Whites up adapt to the image; its tones change only with these settings.
    points: Arc<Mutex<Points>>,
}

impl CameraRawGpu {
    pub(crate) fn new(device: wgpu::Device, queue: wgpu::Queue, health: DeviceHealth, pass: Slot) -> Self {
        CameraRawGpu { device, queue, health, pass, failed: Arc::default(), points: Arc::default() }
    }

    /// Develops `px` (`w × h`) with `p`, or `None` when the CPU must: Texture, Clarity or Detail, the pass
    /// still building, a fault, or a GPU error (logged; the dialog then stays on the CPU).
    /// `key` names the pixels when they are the dialog's unchanged proxy ([`source_key`]), so they
    /// upload once. Neutral settings are `None` too: the CPU develop returns them at once, a GPU
    /// round trip would only cost. `inputs` are the develop's seed, pixel scale and encoding gamma.
    pub(crate) fn develop(&self, pixels: ProxyPixels<'_>, p: &CameraRaw, inputs: &DevelopInputs, float: bool) -> Option<Vec<[f32; 4]>> {
        let ProxyPixels { px, key, w, h } = pixels;
        let spatial = photocraft_algo::camera_raw::spatial(p);
        // Spatial stages develop on the GPU when they are Dehaze, Highlights and Shadows; Texture,
        // Clarity and Detail keep the CPU.
        if (spatial && !photocraft_gpu::spatial_supported(p)) || p.is_identity() || self.failed.load(Ordering::Relaxed) || !self.health.is_ok() {
            return None;
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (px, key, w, h, p, inputs, float);
            None
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(Some(pass)) = self.pass.get() else { return None };
            if spatial {
                let size = [u32::try_from(w).ok()?, u32::try_from(h).ok()?];
                let mut pass = pass.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                return match pass.develop_spatial(&self.device, &self.queue, Some(&self.health), px, key, size, p, inputs, float) {
                    Ok(out) => out,
                    Err(e) => {
                        log::warn!("{}; RAW Develop develops on the CPU", e.0);
                        self.failed.store(true, Ordering::Relaxed);
                        None
                    }
                };
            }
            let points = self.points(px, key, w, h, p, inputs);
            let stages = photocraft_algo::camera_raw::per_pixel(p, inputs, w, h, float, points)?;
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

    /// The image's tones Contrast and Whites up adapt to, measured on the CPU and cached per
    /// source and the settings they depend on.
    #[cfg(not(target_arch = "wasm32"))]
    fn points(
        &self,
        px: &[[f32; 4]],
        key: Option<u64>,
        w: usize,
        h: usize,
        p: &CameraRaw,
        inputs: &DevelopInputs,
    ) -> Option<photocraft_algo::camera_raw::TonePoints> {
        if p.whites <= 0.0 && p.contrast == 0.0 {
            return None;
        }
        let Some(key) = key else { return photocraft_algo::camera_raw::white_black_points(px, w, h, p, inputs) };
        let key = points_key(key, p, inputs);
        let mut cache = self.points.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((k, points)) = *cache
            && k == key
        {
            return points;
        }
        let points = photocraft_algo::camera_raw::white_black_points(px, w, h, p, inputs);
        *cache = Some((key, points));
        points
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera_raw_ui::proxy::ProxyJob;
    use craft::imaging::ui::raw_develop::RawPreviewRequest;
    use craft::ui::canvas::PixelRect;
    use photocraft_algo::camera_raw::develop;
    use serde_json::json;

    /// `CameraRaw` with the named fields set (the type is `#[non_exhaustive]`, so no struct expression), or a copy of a base with them set.
    macro_rules! cr {
        ($($f:ident : $v:expr),* $(,)?) => {{
            let mut s = CameraRaw::default();
            $(s.$f = $v;)*
            s
        }};
        ($base:expr => $($f:ident : $v:expr),* $(,)?) => {{
            let mut s: CameraRaw = $base;
            $(s.$f = $v;)*
            s
        }};
    }

    /// Raising Whites after a Contrast-only preview measures the Whites tones: the cached points
    /// of the Contrast-only settings (Whites tones left at zero) are not reused.
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn raising_whites_misses_the_points_cache() {
        let inputs = DevelopInputs::default();
        let contrast = cr! { contrast: 40.0 };
        let raised = cr!(contrast.clone() => whites: 30.0);
        assert_ne!(points_key(1, &contrast, &inputs), points_key(1, &raised, &inputs));
        assert_eq!(points_key(1, &raised, &inputs), points_key(1, &cr!(raised.clone() => whites: 60.0), &inputs));
        assert_ne!(points_key(1, &raised, &inputs), points_key(1, &cr!(raised.clone() => shadow_tint: 20.0), &inputs), "Calibration is measured");
    }

    /// The pass evaluates develop's expressions in f32 (see `photocraft-gpu`'s Camera Raw tests).
    const TOL: f32 = 1.0 / 4096.0;

    /// The app on egui_kittest's device (a software adapter where there is one), or `None`.
    fn gpu_app(depth: u32) -> Option<PhotocraftApp> {
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
        Some(app)
    }

    /// Opens RAW Develop on the active layer, on the app's own context.
    fn open_raw(app: &mut PhotocraftApp) {
        let ctx = app.ctx().clone();
        crate::camera_raw_ui::open(app, &ctx).unwrap();
    }

    /// Whether the open session has a GPU develop.
    fn has_gpu(app: &PhotocraftApp) -> bool {
        app.raw.as_ref().is_some_and(|session| session.gpu.is_some())
    }

    /// The canvas's pass, built even on a software adapter (which the app leaves on the CPU).
    fn force_pass(app: &PhotocraftApp) {
        app.gpu.as_ref().expect("GPU canvas").force_camera_raw_pass();
    }

    /// Runs the source's develop of `p` (the body of the job `PcRawSource::develop` returns) and the
    /// CPU develop of the same proxy: where it ran, what it made, what the CPU makes.
    fn developed(app: &PhotocraftApp, p: CameraRaw) -> (&'static str, Vec<[f32; 4]>, Vec<[f32; 4]>) {
        let base = app.raw.as_ref().expect("session").source.proxy_base();
        let request = RawPreviewRequest::new(1, p.clone(), None, PixelRect::default(), 1.0);
        let frame = ProxyJob::from_request(&request, &base).run();
        let mut cpu = base.proxy.to_vec();
        let (w, h) = base.size;
        let inputs = DevelopInputs { pixel_scale: (w as f32 / base.full_w.max(1) as f32).min(1.0), ..base.inputs };
        develop(&mut cpu, w, h, &p, &inputs, base.float);
        (frame.backend, frame.px, cpu)
    }

    #[test]
    fn the_pass_is_built_when_camera_raw_first_opens_not_with_the_canvas() {
        let _gpu = photocraft_testkit::gpu_lock();
        let Some(mut app) = gpu_app(8) else { return };
        let canvas = app.gpu.clone().expect("GPU canvas");
        assert!(!canvas.camera_raw_started(), "no RAW Develop pipeline at launch (#6419)");
        open_raw(&mut app);
        assert!(canvas.camera_raw_started(), "built (or refused) from the first open");
        assert_eq!(crate::camera_raw_ui::describe(&app)["developBackend"], "cpu", "the first frame doesn't wait for the pipeline");
        canvas.wait_camera_raw_pass();
        let software = canvas.adapter_info().device_type == wgpu::DeviceType::Cpu;
        assert_eq!(!has_gpu(&app), software, "a software adapter keeps the CPU develop");
    }

    #[test]
    fn per_pixel_settings_develop_on_the_gpu_like_the_cpu_and_the_rest_stays_on_the_cpu() {
        let _gpu = photocraft_testkit::gpu_lock();
        for depth in [8, 16, 32] {
            let Some(mut app) = gpu_app(depth) else { return };
            force_pass(&app);
            open_raw(&mut app);
            if !has_gpu(&app) {
                eprintln!("skipping: the device can't run the RAW Develop pass");
                return;
            }
            let per_pixel = cr! {
                temperature: 20.0,
                tint: -10.0,
                exposure: 0.4,
                contrast: 30.0,
                whites: 25.0,
                blacks: -20.0,
                vibrance: 40.0,
                curve_lights: 25.0,
                grade_shadows: photocraft_algo::camera_raw::Wheel { hue: 220.0, sat: 40.0, lum: 0.0 },
                vignette_amount: -40.0,
                grain_amount: 30.0,
            };
            let (backend, gpu, cpu) = developed(&app, per_pixel.clone());
            assert_eq!(backend, "gpu", "{depth}-bit");
            let worst = gpu.iter().zip(&cpu).flat_map(|(a, b)| (0..4).map(move |c| (a[c] - b[c]).abs())).fold(0.0f32, f32::max);
            assert!(worst <= TOL, "{depth}-bit: off by {worst}");
            // PLAN: the old `describe["developBackend"] == "gpu"` check read the dialog's own render; a `Job` can only be run by the host (no public runner), so the helper runs the job's body and the session's reported backend is not asserted here.
            // Dehaze, Highlights and Shadows develop on the GPU too.
            for spatial in
                [cr!(per_pixel.clone() => shadows: 40.0, highlights: -35.0), cr!(per_pixel.clone() => dehaze: 25.0), cr!(per_pixel.clone() => dehaze: -25.0)]
            {
                let (backend, gpu, cpu) = developed(&app, spatial.clone());
                assert_eq!(backend, "gpu", "{depth}-bit, {spatial:?}");
                let worst = gpu.iter().zip(&cpu).flat_map(|(a, b)| (0..4).map(move |c| (a[c] - b[c]).abs())).fold(0.0f32, f32::max);
                assert!(worst <= 1.0 / 255.0, "{depth}-bit, {spatial:?}: off by {worst}");
            }
            // Texture, Clarity and Detail develop the whole frame on the CPU.
            for cpu_only in [cr!(per_pixel.clone() => clarity: 30.0), cr!(per_pixel.clone() => shadows: 30.0, sharpen_amount: 40.0)] {
                let (backend, shown, cpu) = developed(&app, cpu_only);
                assert_eq!((backend, shown == cpu), ("cpu", true), "{depth}-bit");
            }
            // A device fault: the CPU, from the next frame on.
            app.gpu.as_ref().unwrap().health().mark(photocraft_gpu::Fault::Lost("test".into()));
            let (backend, shown, cpu) = developed(&app, per_pixel);
            assert_eq!((backend, shown == cpu), ("cpu", true), "{depth}-bit after a fault");
        }
    }

    #[test]
    fn neutral_settings_skip_the_gpu_round_trip() {
        let _gpu = photocraft_testkit::gpu_lock();
        let Some(mut app) = gpu_app(8) else { return };
        force_pass(&app);
        open_raw(&mut app);
        if !has_gpu(&app) {
            eprintln!("skipping: the device can't run the RAW Develop pass");
            return;
        }
        let (backend, _, _) = developed(&app, cr!(exposure: 0.5));
        assert_eq!(backend, "gpu", "a per-pixel setting develops on the GPU");
        let (backend, shown, cpu) = developed(&app, CameraRaw::default());
        assert_eq!((backend, shown == cpu), ("cpu", true), "default settings: the CPU returns them unchanged");
    }

    #[test]
    fn rendering_mode_cpu_keeps_the_cpu_develop() {
        let _gpu = photocraft_testkit::gpu_lock();
        let Some(mut app) = gpu_app(8) else { return };
        force_pass(&app);
        app.run("prefs.set", json!({"path": "performance.renderingMode", "value": "cpu"})).unwrap();
        open_raw(&mut app);
        let (backend, shown, cpu) = developed(&app, cr!(exposure: 0.5));
        assert_eq!((backend, shown == cpu), ("cpu", true));
    }
}
