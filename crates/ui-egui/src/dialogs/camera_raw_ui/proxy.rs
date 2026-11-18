#[cfg(not(target_arch = "wasm32"))]
use super::PARALLEL_MIN_PIXELS;
use super::open::raw_stage;
use craft::imaging::raw::RawDevelopSettings as CameraRaw;
#[cfg(test)]
use craft::imaging::ui::raw_develop::RawPreviewRequest;
use photocraft_algo::camera_raw::DevelopInputs;
use photocraft_algo::histogram::RgbHistogram;

/// What every develop of one open raw shares: the proxy and the inputs the source keeps between
/// requests. A request adds the settings.
#[derive(Clone)]
pub(crate) struct ProxyBase {
    pub(crate) proxy: std::sync::Arc<Vec<[f32; 4]>>,
    pub(crate) selection: Option<std::sync::Arc<Vec<f32>>>,
    pub(crate) raw_curve: Option<std::sync::Arc<Vec<[f32; 2]>>>,
    pub(crate) float: bool,
    pub(crate) size: (usize, usize),
    pub(crate) full_w: usize,
    pub(crate) display: std::sync::Arc<photocraft_engine::display_color::CanvasDisplay>,
    pub(crate) gpu: Option<crate::camera_raw_gpu::CameraRawGpu>,
    pub(crate) source_key: u64,
    /// The develop's inputs besides the settings (seed, pixel scale, encoding gamma).
    pub(crate) inputs: DevelopInputs,
}

/// Everything one develop of the proxy reads, owned, so it can run off the UI thread.
pub(crate) struct ProxyJob {
    pub(crate) base: ProxyBase,
    /// The settings as the request carries them.
    pub(crate) params: CameraRaw,
}

/// A developed proxy, ready to show.
pub(crate) struct ProxyFrame {
    pub(crate) px: Vec<[f32; 4]>,
    pub(crate) histogram: RgbHistogram,
    pub(crate) histogram_ms: f64,
    /// The developed pixels as the canvas shows them: the after raster's pixels.
    pub(crate) image: egui::ColorImage,
    pub(crate) ms: f64,
    pub(crate) develop_ms: f64,
    pub(crate) backend: &'static str,
}

/// `px` (straight RGBA, `w` × `h`) as the canvas shows it, converted in bands across cores (#4682),
/// not as a whole copy on one thread.
pub(crate) fn display_pixels(px: &[[f32; 4]], w: usize, h: usize, display: &photocraft_engine::display_color::CanvasDisplay) -> egui::ColorImage {
    crate::dialogs::camera_raw_detail_ui::display_image(photocraft_geom::Rect::new(0, 0, w as i32, h as i32), display, |r, out| {
        let start = (r.y0.max(0) as usize).saturating_mul(w);
        if let Some(src) = px.get(start..start.saturating_add(out.len())) {
            out.copy_from_slice(src);
        }
    })
}

impl ProxyJob {
    /// The develop of `request` over `base`: its settings as they are.
    #[cfg(test)]
    pub(crate) fn from_request(request: &RawPreviewRequest, base: &ProxyBase) -> Self {
        Self { base: base.clone(), params: request.settings.clone() }
    }

    pub(crate) fn run(self) -> ProxyFrame {
        let ProxyJob { base, params } = self;
        let ProxyBase { proxy, selection, raw_curve, float, size, full_w, display, gpu, source_key, inputs } = base;
        let t0 = crate::gpu_canvas::now_ms();
        let (pw, ph) = size;
        let mut p = params;
        let inputs = DevelopInputs { pixel_scale: (pw as f32 / full_w.max(1) as f32).min(1.0), ..inputs };
        // The open-time dialog re-tones the proxy for each job; otherwise the GPU reads the
        // proxy as it is and uploads it once.
        let retoned = raw_curve.as_ref().map(|curve| {
            let mut px = proxy.to_vec();
            photocraft_io::raw::retone(&mut px, pw, ph, curve, raw_stage(&p));
            px
        });
        if retoned.is_some() {
            (p.shadows, p.highlights, p.clarity, p.texture, p.dehaze) = (0.0, 0.0, 0.0, 0.0, 0.0);
        }
        let develop_start = crate::gpu_canvas::now_ms();
        let key = retoned.is_none().then_some(source_key);
        let input = retoned.as_deref().unwrap_or(&proxy);
        let (mut px, backend) =
            match gpu.as_ref().and_then(|g| g.develop(crate::camera_raw_gpu::ProxyPixels { px: input, key, w: pw, h: ph }, &p, &inputs, float)) {
                Some(out) => (out, "gpu"),
                None => {
                    let mut px = retoned.unwrap_or_else(|| proxy.to_vec());
                    photocraft_algo::camera_raw::develop(&mut px, pw, ph, &p, &inputs, float);
                    (px, "cpu")
                }
            };
        let develop_ms = crate::gpu_canvas::now_ms() - develop_start;
        if let Some(mask) = &selection {
            let blend = |(out, (original, coverage)): (&mut [f32; 4], (&[f32; 4], &f32))| {
                for (out, original) in out.iter_mut().zip(original) {
                    *out = *original + (*out - *original) * coverage;
                }
            };
            // Each pixel is independent, so bands of the proxy blend in parallel (#211, P36).
            #[cfg(not(target_arch = "wasm32"))]
            {
                use rayon::prelude::*;
                px.par_iter_mut().zip(proxy.par_iter().zip(mask.par_iter())).with_min_len(PARALLEL_MIN_PIXELS).for_each(blend);
            }
            #[cfg(target_arch = "wasm32")]
            px.iter_mut().zip(proxy.iter().zip(mask.iter())).for_each(blend);
        }
        let histogram_start = crate::gpu_canvas::now_ms();
        let histogram = RgbHistogram::from_rgba(&px);
        let histogram_ms = crate::gpu_canvas::now_ms() - histogram_start;
        let image = display_pixels(&px, pw, ph, &display);
        ProxyFrame { px, histogram, histogram_ms, image, ms: crate::gpu_canvas::now_ms() - t0, develop_ms, backend }
    }
}
