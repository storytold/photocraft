#[cfg(not(target_arch = "wasm32"))]
use super::PARALLEL_MIN_PIXELS;
use super::open::raw_stage;
use super::{CameraRaw, CameraRawDialog, RgbHistogram};
use crate::camera_raw_gpu::{CameraRawGpu, GpuFrame};

/// Everything one develop of the proxy reads, owned, so it can run off the UI thread.
pub(super) struct ProxyJob {
    pub(super) proxy: std::sync::Arc<Vec<[f32; 4]>>,
    pub(super) selection: Option<std::sync::Arc<Vec<f32>>>,
    pub(super) raw_curve: Option<std::sync::Arc<Vec<[f32; 2]>>>,
    pub(super) params: CameraRaw,
    pub(super) float: bool,
    pub(super) size: (usize, usize),
    pub(super) full_w: usize,
    pub(super) display: std::sync::Arc<photocraft_engine::display_color::CanvasDisplay>,
    pub(super) gpu: Option<CameraRawGpu>,
    pub(super) source_key: u64,
    /// Keep a GPU frame on the GPU for the dialog to show; `false` when the caller reads the
    /// pixels at once (another develop may then reuse the frame's slot).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(super) view: bool,
    /// Read a frame kept on the GPU back here too: the dialog shows something that reads its
    /// pixels (the vectorscope, a clipping warning, samplers, the pointer over the preview), and
    /// the UI thread shouldn't wait for them.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(super) pixels: bool,
}

/// What the preview draws: an image converted on the CPU, or a frame converted on the GPU.
pub(super) enum Shown {
    Image(egui::ColorImage),
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Gpu(Box<GpuFrame>),
}

/// A developed proxy, ready to show.
pub(super) struct ProxyFrame {
    pub(super) px: Processed,
    pub(super) histogram: RgbHistogram,
    /// The CPU count's time; 0 when the GPU counted it within the develop.
    pub(super) histogram_ms: f64,
    pub(super) shown: Shown,
    pub(super) display: std::sync::Arc<photocraft_engine::display_color::CanvasDisplay>,
    pub(super) ms: f64,
    pub(super) develop_ms: f64,
    pub(super) backend: &'static str,
}

/// A frame's developed pixels (straight RGBA): developed on the CPU or read back, or still on the
/// GPU, read back the first time something asks for them ([`Self::pixels`]).
#[derive(Default)]
pub(crate) struct Processed {
    px: std::sync::OnceLock<Vec<[f32; 4]>>,
    gpu: Option<OnGpu>,
}

/// Where a frame kept on the GPU is, and how to develop it on the CPU if reading it fails.
struct OnGpu {
    gpu: CameraRawGpu,
    frame: GpuFrame,
    cpu: Box<dyn Fn() -> Vec<[f32; 4]> + Send + Sync>,
}

impl Processed {
    pub(crate) fn cpu(px: Vec<[f32; 4]>) -> Self {
        Processed { px: px.into(), gpu: None }
    }

    /// The pixels, read back from the GPU the first time (blocking, a couple of milliseconds for
    /// the proxy), or developed on the CPU when that fails.
    pub(crate) fn pixels(&self) -> &[[f32; 4]] {
        self.px.get_or_init(|| self.gpu.as_ref().map_or_else(Vec::new, |g| g.gpu.read(&g.frame).unwrap_or_else(|| (g.cpu)())))
    }

    /// The frame kept on the GPU, if it was.
    #[cfg(test)]
    pub(crate) fn gpu_frame(&self) -> Option<&GpuFrame> {
        self.gpu.as_ref().map(|g| &g.frame)
    }

    /// Whether the pixels are still only on the GPU.
    pub(crate) fn on_gpu(&self) -> bool {
        self.gpu.is_some() && self.px.get().is_none()
    }
}

impl ProxyJob {
    pub(super) fn run(self) -> ProxyFrame {
        let t0 = crate::gpu_canvas::now_ms();
        let (pw, ph) = self.size;
        let mut p = self.params;
        p.pixel_scale = (pw as f32 / self.full_w.max(1) as f32).min(1.0);
        // The frame stays on the GPU when nothing but the develop changes it (not on the web,
        // which can't read it back when the scopes need it).
        #[cfg(not(target_arch = "wasm32"))]
        if self.view && self.raw_curve.is_none() && self.selection.is_none() {
            let develop_start = crate::gpu_canvas::now_ms();
            if let Some(gpu) = &self.gpu
                && let Some(mut frame) = gpu.develop_view(&self.proxy, Some(self.source_key), pw, ph, &p, self.float, &self.display, self.pixels)
            {
                let develop_ms = crate::gpu_canvas::now_ms() - develop_start;
                let histogram = frame.histogram.clone();
                let (proxy, settings, float) = (self.proxy.clone(), p.clone(), self.float);
                let cpu = move || {
                    let mut px = proxy.to_vec();
                    photocraft_algo::camera_raw::develop(&mut px, pw, ph, &settings, float);
                    px
                };
                let read = frame.read_back.take().map_or_else(std::sync::OnceLock::new, std::sync::OnceLock::from);
                let px = Processed { px: read, gpu: Some(OnGpu { gpu: gpu.clone(), frame: frame.clone(), cpu: Box::new(cpu) }) };
                return ProxyFrame {
                    px,
                    histogram,
                    histogram_ms: 0.0,
                    shown: Shown::Gpu(Box::new(frame)),
                    display: self.display,
                    ms: crate::gpu_canvas::now_ms() - t0,
                    develop_ms,
                    backend: "gpu",
                };
            }
        }
        // The open-time dialog re-tones the proxy for each job; otherwise the GPU reads the
        // proxy as it is and uploads it once.
        let retoned = self.raw_curve.as_ref().map(|curve| {
            let mut px = self.proxy.to_vec();
            photocraft_io::raw::retone(&mut px, pw, ph, curve, raw_stage(&p));
            px
        });
        if retoned.is_some() {
            (p.shadows, p.highlights, p.clarity, p.texture, p.dehaze) = (0.0, 0.0, 0.0, 0.0, 0.0);
        }
        let develop_start = crate::gpu_canvas::now_ms();
        let key = retoned.is_none().then_some(self.source_key);
        let input = retoned.as_deref().unwrap_or(&self.proxy);
        let (mut px, backend) = match self.gpu.as_ref().and_then(|g| g.develop(input, key, pw, ph, &p, self.float)) {
            Some(out) => (out, "gpu"),
            None => {
                let mut px = retoned.unwrap_or_else(|| self.proxy.to_vec());
                photocraft_algo::camera_raw::develop(&mut px, pw, ph, &p, self.float);
                (px, "cpu")
            }
        };
        let develop_ms = crate::gpu_canvas::now_ms() - develop_start;
        if let Some(mask) = &self.selection {
            let blend = |(out, (original, coverage)): (&mut [f32; 4], (&[f32; 4], &f32))| {
                for (out, original) in out.iter_mut().zip(original) {
                    *out = *original + (*out - *original) * coverage;
                }
            };
            // Each pixel is independent, so bands of the proxy blend in parallel (#211, P36).
            #[cfg(not(target_arch = "wasm32"))]
            {
                use rayon::prelude::*;
                px.par_iter_mut().zip(self.proxy.par_iter().zip(mask.par_iter())).with_min_len(PARALLEL_MIN_PIXELS).for_each(blend);
            }
            #[cfg(target_arch = "wasm32")]
            px.iter_mut().zip(self.proxy.iter().zip(mask.iter())).for_each(blend);
        }
        let histogram_start = crate::gpu_canvas::now_ms();
        let histogram = RgbHistogram::from_rgba(&px);
        let histogram_ms = crate::gpu_canvas::now_ms() - histogram_start;
        let image = CameraRawDialog::image(&px, pw, ph, &self.display);
        ProxyFrame {
            px: Processed::cpu(px),
            histogram,
            histogram_ms,
            shown: Shown::Image(image),
            display: self.display,
            ms: crate::gpu_canvas::now_ms() - t0,
            develop_ms,
            backend,
        }
    }
}
