#[cfg(not(target_arch = "wasm32"))]
use super::PARALLEL_MIN_PIXELS;
use super::open::raw_stage;
use super::{CameraRaw, CameraRawDialog, RgbHistogram};
use photocraft_algo::camera_raw::geometry::UprightFix;

/// Everything one develop of the proxy reads, owned, so it can run off the UI thread.
pub(super) struct ProxyJob {
    pub(super) proxy: std::sync::Arc<Vec<[f32; 4]>>,
    pub(super) selection: Option<std::sync::Arc<Vec<f32>>>,
    pub(super) raw_curve: Option<std::sync::Arc<Vec<[f32; 2]>>>,
    pub(super) params: CameraRaw,
    pub(super) float: bool,
    pub(super) size: (usize, usize),
    pub(super) full_w: usize,
    /// The canvas's blocks of the proxy, when it does not fill it: Geometry develops only them.
    pub(super) canvas_frame: Option<[usize; 4]>,
    /// The layer has no transparency (no alpha, or the Background): what Geometry uncovers is
    /// shown white, as the filter makes it.
    pub(super) opaque: bool,
    pub(super) display: std::sync::Arc<photocraft_engine::display_color::CanvasDisplay>,
    pub(super) gpu: Option<crate::camera_raw_gpu::CameraRawGpu>,
    pub(super) source_key: u64,
    /// Where Upright's analysis comes from: the full-resolution canvas, as the filter sees it.
    pub(super) upright: Option<UprightSource>,
}

/// The full-resolution canvas Upright analyses, and the analyses done so far (one per mode; the
/// layer does not change while the dialog is open). The proxy is a smaller copy whose lines
/// differ, so it is handed the full canvas's analysis and shows the correction OK applies.
#[derive(Clone)]
pub(super) struct UprightSource {
    pub(super) source: photocraft_raster::Surface,
    pub(super) frame: photocraft_geom::Rect,
    pub(super) done: std::sync::Arc<std::sync::Mutex<Vec<UprightFix>>>,
}

impl UprightSource {
    /// The analysis `p`'s Upright mode needs, from the full canvas (cached), or `None` for the
    /// modes that don't analyse.
    pub(super) fn fix(&self, p: &CameraRaw) -> Option<UprightFix> {
        let mode = photocraft_algo::camera_raw::geometry::Upright::parse(&p.perspective_upright)?;
        let mut done = self.done.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(f) = done.iter().find(|f| f.mode == mode) {
            return Some(*f);
        }
        let f = photocraft_engine::lens_cmds::upright_fix(&self.source, self.frame, p)?;
        done.push(f);
        Some(f)
    }
}

/// The pixels the masks first see in a develop (before their local adjustments), so the
/// overlay shows a mask's coverage as it applied.
pub(crate) struct Unmasked {
    pub(crate) px: Vec<[f32; 4]>,
    pub(crate) w: usize,
    pub(crate) h: usize,
    /// The part of the proxy they are: the canvas's blocks when Geometry developed only those.
    pub(crate) frame: Option<[usize; 4]>,
    /// The settings of that develop.
    pub(crate) params: CameraRaw,
    /// Where the canvas sits in these pixels when they reach past it (`[x0, y0, x1, y1]`): the
    /// masks, drawn on the canvas, were [`reframed`](photocraft_algo::camera_raw::masks::reframed)
    /// to it.
    pub(crate) masks_frame: Option<[f32; 4]>,
}

/// A developed proxy, ready to show.
pub(super) struct ProxyFrame {
    pub(super) px: Vec<[f32; 4]>,
    pub(super) unmasked: Option<Unmasked>,
    pub(super) histogram: RgbHistogram,
    pub(super) histogram_ms: f64,
    pub(super) image: egui::ColorImage,
    pub(super) display: std::sync::Arc<photocraft_engine::display_color::CanvasDisplay>,
    pub(super) ms: f64,
    pub(super) develop_ms: f64,
    pub(super) backend: &'static str,
}

impl ProxyJob {
    /// The settings the proxy develops with: radii scaled to the proxy, and Upright's analysis
    /// of the full canvas rather than of the proxy.
    pub(super) fn develop_params(&self) -> CameraRaw {
        let mut p = self.params.clone();
        p.pixel_scale = (self.size.0 as f32 / self.full_w.max(1) as f32).min(1.0);
        if !p.geometry_neutral()
            && let Some(fix) = self.upright.as_ref().and_then(|u| u.fix(&p))
        {
            p.upright_fix = Some(fix);
        }
        p
    }

    pub(super) fn run(self) -> ProxyFrame {
        let t0 = crate::gpu_canvas::now_ms();
        let (pw, ph) = self.size;
        let mut p = self.develop_params();
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
        // With Geometry on, the filter develops (and warps about the centre of) the canvas only;
        // the layer past it stays as it is. The preview does the same on the canvas's blocks.
        let frame = self.canvas_frame.filter(|_| !p.geometry_neutral());
        // The GPU runs only without active masks (they are spatial); its result is then what the
        // masks see, unless Vignette or Grain (after the masks) changed it.
        let gpu = self.gpu.as_ref().filter(|_| p.masks.is_empty() || (p.vignette_amount == 0.0 && p.grain_amount <= 0.0));
        let keep = !p.masks.is_empty();
        // The masks are drawn on the canvas; developed with the layer past it, they keep their
        // place on the canvas.
        let masks_frame = self.canvas_frame.filter(|_| frame.is_none() && !p.masks.is_empty()).map(|c| c.map(|v| v as f32));
        if let Some(f) = masks_frame {
            p.masks = photocraft_algo::camera_raw::masks::reframed(&p.masks, f, pw, ph);
        }
        let (mut px, backend, unmasked) = if let Some([x0, y0, x1, y1]) = frame {
            let (fw, fh) = (x1 - x0, y1 - y0);
            let mut sub: Vec<[f32; 4]> = (y0..y1).flat_map(|y| input.get(y * pw + x0..y * pw + x1).unwrap_or(&[]).iter().copied()).collect();
            let (backend, kept) = match gpu.and_then(|g| g.develop(&sub, None, fw, fh, &p, self.float)) {
                Some(out) => {
                    sub = out;
                    ("gpu", keep.then(|| sub.clone()))
                }
                None => ("cpu", photocraft_algo::camera_raw::develop_keeping_unmasked(&mut sub, fw, fh, &p, self.float)),
            };
            let mut px = input.to_vec();
            for (y, row) in (y0..y1).zip(sub.chunks_exact(fw.max(1))) {
                if let Some(dst) = px.get_mut(y * pw + x0..y * pw + x1) {
                    dst.copy_from_slice(row);
                }
            }
            (px, backend, kept.map(|px| (px, fw, fh, frame)))
        } else {
            match gpu.and_then(|g| g.develop(input, key, pw, ph, &p, self.float)) {
                Some(out) => {
                    let kept = keep.then(|| (out.clone(), pw, ph, None));
                    (out, "gpu", kept)
                }
                None => {
                    let mut px = retoned.unwrap_or_else(|| self.proxy.to_vec());
                    let kept = photocraft_algo::camera_raw::develop_keeping_unmasked(&mut px, pw, ph, &p, self.float).map(|u| (u, pw, ph, None));
                    (px, "cpu", kept)
                }
            }
        };
        let unmasked = unmasked.map(|(px, w, h, frame)| Unmasked { px, w, h, frame, params: p.clone(), masks_frame });
        let develop_ms = crate::gpu_canvas::now_ms() - develop_start;
        if self.opaque && !p.geometry_neutral() {
            over_white(&mut px);
        }
        if let Some(mask) = &self.selection {
            // Alpha-aware, as the filter mixes its result by the selection: what Geometry made
            // transparent lends no colour to what stays of the original.
            let blend = |(out, (original, coverage)): (&mut [f32; 4], (&[f32; 4], &f32))| {
                photocraft_engine::lens_cmds::mix_by_coverage(out, original, *coverage, true);
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
        ProxyFrame { px, unmasked, histogram, histogram_ms, image, display: self.display, ms: crate::gpu_canvas::now_ms() - t0, develop_ms, backend }
    }
}

/// Developed pixels as an opaque layer keeps them: what the warp left transparent turns white
/// (`lens_cmds::camera_raw_surface_on` does the same when the filter is applied).
pub(super) fn over_white(px: &mut [[f32; 4]]) {
    for q in px.iter_mut() {
        let a = if q[3].is_finite() { q[3].clamp(0.0, 1.0) } else { 1.0 };
        for v in &mut q[..3] {
            *v = *v * a + 1.0 - a;
        }
        q[3] = 1.0;
    }
}
