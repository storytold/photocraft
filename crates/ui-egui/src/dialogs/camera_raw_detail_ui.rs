//! Full-resolution refinement of Camera Raw's interactive proxy. Native work is off the
//! UI thread; only one develop runs at a time, and obsolete results never replace new edits.
//! The source asks for the develop it wants ([`DetailPreview::want`]); the after picture collects
//! it each pass ([`DetailPreview::refresh`]) and shows it once it is the wanted one.
use craft::imaging::raw::RawDevelopSettings as CameraRaw;
use craft::imaging::scopes::ClippingMask;
#[cfg(test)]
use craft::imaging::scopes::clipping;
use craft::ui::canvas::PixelRect as RasterRect;
use egui::{Color32, ColorImage, Context};
use photocraft_algo::camera_raw::DevelopInputs;
use photocraft_geom::Rect as PixelRect;
use photocraft_raster::Surface;
use std::sync::mpsc::TryRecvError;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::Receiver,
};

#[derive(Clone)]
pub(crate) enum Coverage {
    Selection(Surface),
    Mask(photocraft_doc::LayerMask),
    None,
}

type DetailResult = Result<(u64, Surface), String>;

/// The develop the source last asked for: its order (the source's develop counter, which changes
/// with every settings change, unlike the request's revision) and what it runs with.
struct Wanted {
    order: u64,
    params: CameraRaw,
    inputs: DevelopInputs,
    ctx: Context,
}

pub(crate) struct DetailPreview {
    source: Surface,
    /// The open-time dialog's camera tone curve: Shadows, Highlights, Clarity, Texture and Dehaze
    /// then act on the raw data, in linear light before it (see [`develop`]).
    pub(crate) raw_curve: Option<Arc<Vec<[f32; 2]>>>,
    area: PixelRect,
    coverage: Coverage,
    result: Option<(u64, Surface)>,
    pending: Option<(u64, Receiver<DetailResult>)>,
    cancel: Arc<AtomicBool>,
    pub(crate) error: Option<String>,
    failed_revision: Option<u64>,
    /// The clipping mask of `result`'s area, made on first ask and kept with its revision.
    mask: Option<(u64, Arc<ClippingMask>)>,
    /// The develop of the settings in force; kept while the view needs none, so zooming back in
    /// does not develop the same settings again.
    wanted: Option<Wanted>,
    /// Whether the view shows more pixels than the proxy has, so the wanted develop is shown.
    active: bool,
    /// Whether `result` is the wanted develop, so the after picture shows it.
    pub(crate) ready: bool,
}

impl Drop for DetailPreview {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl DetailPreview {
    pub(crate) fn new(source: Surface, area: PixelRect, coverage: Coverage) -> Self {
        Self {
            source,
            raw_curve: None,
            area,
            coverage,
            result: None,
            pending: None,
            cancel: Arc::new(AtomicBool::new(false)),
            error: None,
            failed_revision: None,
            mask: None,
            wanted: None,
            active: false,
            ready: false,
        }
    }

    pub(crate) fn pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Asks for the develop `order` of `params`: it starts once the running one is done, and the
    /// after picture shows it once it lands. The same settings keep the develop already wanted: the
    /// workspace asks again for every pan and zoom, and the refinement covers the whole area.
    pub(crate) fn want(&mut self, ctx: &Context, order: u64, params: &CameraRaw, inputs: &DevelopInputs) {
        if !self.wanted.as_ref().is_some_and(|w| w.params == *params && w.inputs == *inputs) {
            self.wanted = Some(Wanted { order, params: params.clone(), inputs: *inputs, ctx: ctx.clone() });
        }
        self.active = true;
        self.refresh();
    }

    /// The view needs no refinement: the after picture shows the proxy, and nothing new starts.
    pub(crate) fn unwant(&mut self) {
        self.active = false;
        self.ready = false;
    }

    /// The order of the develop `result` holds, for the after picture's revision.
    pub(crate) fn shown(&self) -> Option<u64> {
        self.result.as_ref().filter(|_| self.ready).map(|(order, _)| *order)
    }

    /// Collects a finished develop, starts the wanted one when none runs and it is not there yet,
    /// and says whether the result is the wanted develop.
    pub(crate) fn refresh(&mut self) {
        self.poll();
        let wanted = self.wanted.as_ref().map(|w| w.order);
        self.ready = self.active && wanted.is_some() && self.result.as_ref().map(|(order, _)| *order) == wanted;
        if !self.active || self.ready || self.pending.is_some() {
            return;
        }
        if let Some(Wanted { order, params, inputs, ctx }) = self.wanted.take() {
            self.request(&ctx, order, &params, &inputs);
            self.wanted = Some(Wanted { order, params, inputs, ctx });
        }
    }

    /// Takes a finished develop off the worker: kept when it is the wanted one, dropped when the
    /// settings moved on while it ran. A failure of the wanted develop is reported and not retried.
    fn poll(&mut self) {
        let Some((requested, rx)) = &self.pending else { return };
        let requested = *requested;
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("RAW Develop detail preview worker stopped".into()),
        };
        self.pending = None;
        let wanted = self.wanted.as_ref().map(|w| w.order);
        match result {
            Ok((order, surface)) if Some(order) == wanted => {
                self.result = Some((order, surface));
                self.mask = None;
                self.error = None;
            }
            Ok(_) => {} // A setting changed while the worker was developing this one.
            Err(e) => {
                if Some(requested) == wanted {
                    self.error = Some(e);
                }
                self.failed_revision = Some(requested);
            }
        }
    }

    /// Starts the full-resolution develop of `revision` unless one runs, one is ready or it failed.
    /// `inputs` carry the seed and encoding gamma; the refinement runs at pixel scale 1.
    pub(crate) fn request(&mut self, ctx: &Context, revision: u64, params: &CameraRaw, inputs: &DevelopInputs) {
        if self.pending.is_some() || self.result.as_ref().is_some_and(|(r, _)| *r == revision) || self.failed_revision == Some(revision) {
            return;
        }
        self.result = None;
        self.mask = None;
        self.error = None;
        // The engine's full-image pipeline needs multiple temporary float buffers. Never start
        // an unbounded allocation just because a huge sparse document was zoomed into.
        if self.area.width() as u64 * self.area.height() as u64 > 64_000_000 {
            self.error = Some("Full-resolution RAW Develop preview is limited to 64 megapixels; the proxy remains available".into());
            self.failed_revision = Some(revision);
            return;
        }
        let source = self.source.clone();
        let raw_curve = self.raw_curve.clone();
        let area = self.area;
        let coverage = self.coverage.clone();
        let params = params.clone();
        let inputs = DevelopInputs { pixel_scale: 1.0, ..*inputs };
        let cancel = self.cancel.clone();
        let ctx = ctx.clone();
        let (tx, rx) = std::sync::mpsc::channel::<DetailResult>();
        let work = move || {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                develop(&source, raw_curve.as_deref().map(Vec::as_slice), area, &params, &inputs, &coverage)
            }))
            .map(|surface| (revision, surface))
            .map_err(|_| "RAW Develop detail preview failed; the proxy remains available".into());
            if !cancel.load(Ordering::Relaxed) {
                let _ = tx.send(result);
                ctx.request_repaint();
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            // A closed dialog can leave one non-interruptible engine stage finishing. Serialize
            // that work with the next dialog instead of multiplying its peak memory.
            static WORKER: std::sync::Mutex<()> = std::sync::Mutex::new(());
            match std::thread::Builder::new().name("camera-raw-preview".into()).spawn(move || {
                let _guard = WORKER.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                work();
            }) {
                Ok(_) => self.pending = Some((revision, rx)),
                Err(e) => {
                    self.error = Some(format!("Cannot start RAW Develop preview: {e}"));
                    self.failed_revision = Some(revision);
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            // No browser worker/executor is available here. Preserve the interactive proxy
            // instead of blocking its UI with the full-image engine pipeline.
            drop(work);
            drop(rx);
            self.error = Some("Full-resolution filtered preview requires a native RAW Develop worker; the web preview keeps the proxy".into());
            self.failed_revision = Some(revision);
        }
    }

    /// Writes the refined develop of `region` (document pixels) into `out`, one texel per `factor`
    /// pixels each way, as the canvas shows it (sRGBA, premultiplied): each texel the average of its
    /// block, transparent where the block leaves the refined area. `false` when the refinement is not
    /// the wanted develop, and the caller then reads the proxy. Only `out`'s texels are converted, so
    /// a view of the whole image at Fit is served as well as a 1:1 crop.
    pub(crate) fn read(&self, region: RasterRect, factor: u32, display: &photocraft_engine::display_color::CanvasDisplay, out: &mut ColorImage) -> bool {
        let Some((_, surface)) = self.result.as_ref().filter(|_| self.ready) else { return false };
        let [out_w, out_h] = out.size;
        let area = self.area;
        let image = display_image(PixelRect::new(0, 0, out_w as i32, out_h as i32), display, |texels, px| {
            block_averages(surface, area, region, factor, texels, px);
        });
        if image.size == out.size {
            out.pixels = image.pixels;
        }
        true
    }

    #[cfg(test)]
    /// The clipped channels of every pixel of the refined area, with the revision they were made
    /// from; `None` while the area is not refined for the current revision. Made once per revision.
    pub(crate) fn clipping(&mut self) -> Option<(Arc<ClippingMask>, u64)> {
        let (revision, surface) = self.result.as_ref().filter(|_| self.ready)?;
        if let Some((r, mask)) = &self.mask
            && r == revision
        {
            return Some((mask.clone(), *r));
        }
        let (w, h) = (self.area.width() as usize, self.area.height() as usize);
        let mut mask = ClippingMask::new([w, h]);
        if let Some(band_rows) = CROP_BAND_PIXELS.checked_div(w) {
            // Read in bands of rows: a whole-area float copy of a large refinement would be gigabytes.
            let band_rows = band_rows.max(1);
            let mut band = vec![[0.0f32; 4]; band_rows * w];
            let mut y = 0;
            while y < h {
                let rows = band_rows.min(h - y);
                let y0 = self.area.y0 + y as i32;
                surface.read_rgba_into(PixelRect::new(self.area.x0, y0, self.area.x1, y0 + rows as i32), &mut band[..rows * w]);
                for (i, pixel) in band[..rows * w].iter().enumerate() {
                    let clipped = clipping(*pixel);
                    mask.shadows[y * w + i] = clipped.shadows;
                    mask.highlights[y * w + i] = clipped.highlights;
                }
                y += rows;
            }
        }
        let mask = Arc::new(mask);
        let revision = *revision;
        self.mask = Some((revision, mask.clone()));
        Some((mask, revision))
    }

    #[cfg(test)]
    pub(crate) fn sample(&self, p: [f32; 2], revision: u64, before: bool, params: &CameraRaw) -> Option<[f32; 4]> {
        if !self.ready || !p.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)) {
            return None;
        }
        let source = if before || params.is_identity() {
            &self.source
        } else if let Some((r, source)) = &self.result
            && *r == revision
        {
            source
        } else {
            return None;
        };
        let x = self.area.x0 + ((p[0] * self.area.width() as f32) as i32).min((self.area.width() - 1) as i32);
        let y = self.area.y0 + ((p[1] * self.area.height() as f32) as i32).min((self.area.height() - 1) as i32);
        Some(source.rgba(x, y))
    }
}

/// The full-size result of `params` over `area`. With a raw file's tone `curve`, the raw-stage
/// sliders are applied first to the whole area, from the whole frame's maps as Open computes them
/// ([`photocraft_io::raw::retone`]), then the rest of the pipeline: at 1:1 the preview shows close
/// to what Open will make. It retones the developed pixels, which already carry the black point,
/// so it differs from Open only in the deepest shadows, as `retone` documents.
fn develop(source: &Surface, curve: Option<&[[f32; 2]]>, area: PixelRect, params: &CameraRaw, inputs: &DevelopInputs, coverage: &Coverage) -> Surface {
    let tone = photocraft_io::raw::Tone {
        shadows: params.shadows,
        highlights: params.highlights,
        clarity: params.clarity,
        texture: params.texture,
        dehaze: params.dehaze,
    };
    let (retoned, params) = match curve {
        Some(curve) if !tone.is_neutral() => {
            let (w, h) = (area.width() as usize, area.height() as usize);
            let mut px = vec![[0.0f32; 4]; w * h];
            source.read_rgba_into(area, &mut px);
            photocraft_io::raw::retone(&mut px, w, h, curve, tone);
            let fmt = source.format();
            let n = fmt.channels();
            // Written back in bands of rows, so only one band-sized buffer lives next to `px`
            // (a whole-area one is another 4 bytes per channel and pixel). Each band is read from
            // the source first to keep its stored alpha.
            let mut out = source.clone();
            let band = (65_536 / w.max(1)).max(1);
            let mut data = Vec::new();
            for (b, rows) in px.chunks(band * w.max(1)).enumerate() {
                let y0 = area.y0 + (b * band) as i32;
                let rect = PixelRect::new(area.x0, y0, area.x1, y0 + (rows.len() / w.max(1)) as i32);
                source.read_region_into(rect, &mut data);
                for (o, q) in data.chunks_exact_mut(n).zip(rows) {
                    let alpha = if fmt.alpha { o.get(n - 1).copied() } else { None };
                    photocraft_raster::from_rgba_into(&fmt, *q, o);
                    if let (Some(a), Some(last)) = (alpha, o.get_mut(n - 1)) {
                        *last = a;
                    }
                }
                out.write_region(rect, &data);
            }
            let mut rest = params.clone();
            (rest.shadows, rest.highlights, rest.clarity, rest.texture, rest.dehaze) = (0.0, 0.0, 0.0, 0.0, 0.0);
            (Some(out), rest)
        }
        _ => (None, params.clone()),
    };
    let base = retoned.as_ref().unwrap_or(source);
    let mut result = photocraft_engine::lens_cmds::camera_raw_surface(base, area, &params, inputs);
    if matches!(coverage, Coverage::None) {
        return result;
    }
    let width = area.width() as usize;
    let channels = source.channels();
    let mut original = Vec::new();
    let mut row = Vec::new();
    for y in area.y0..area.y1 {
        let rect = PixelRect::new(area.x0, y, area.x1, y + 1);
        source.read_region_into(rect, &mut original);
        result.read_region_into(rect, &mut row);
        for (x, (out, original)) in row.chunks_exact_mut(channels).zip(original.chunks_exact(channels)).take(width).enumerate() {
            let x = area.x0 + x as i32;
            let alpha = match coverage {
                Coverage::Selection(mask) => mask.sample_channel(x, y, 0),
                Coverage::Mask(mask) => mask.value(x, y),
                Coverage::None => 1.0,
            }
            .clamp(0.0, 1.0);
            for (out, original) in out.iter_mut().zip(original) {
                *out = *original + (*out - *original) * alpha;
            }
        }
        result.write_region(rect, &row);
    }
    result
}

/// Pixels per band [`display_image`] reads and colour-converts at once: at least `craft-cms`'s
/// parallel threshold (16K pixels), and small enough that a band's float scratch is 1 MiB.
const CROP_BAND_PIXELS: usize = 64 * 1024;

/// The texels `texels` (one per `factor` pixels each way from `region`'s origin) of `surface`'s
/// `area` into `out`, straight RGBA: each the alpha-weighted average of the pixels of its block that
/// lie in `area`, transparent where none does. Reads the area's rows once per band.
fn block_averages(surface: &Surface, area: PixelRect, region: RasterRect, factor: u32, texels: PixelRect, out: &mut [[f32; 4]]) {
    let f = i64::from(factor.max(1));
    let tw = texels.width() as usize;
    out.fill([0.0; 4]);
    if tw == 0 {
        return;
    }
    // Document columns of the texel row, clipped to the area.
    let x0 = (i64::from(region.x0) + i64::from(texels.x0) * f).max(i64::from(area.x0));
    let x1 = (i64::from(region.x0) + i64::from(texels.x1) * f).min(i64::from(area.x1));
    if x1 <= x0 {
        return;
    }
    let (Ok(x0), Ok(x1)) = (i32::try_from(x0), i32::try_from(x1)) else { return };
    let mut row = vec![[0.0f32; 4]; (x1 - x0) as usize];
    let mut sums = vec![[0.0f32; 5]; tw];
    for (ty, texel_row) in (texels.y0..texels.y1).zip(out.chunks_exact_mut(tw)) {
        sums.fill([0.0; 5]);
        let top = i64::from(region.y0) + i64::from(ty) * f;
        let (y0, y1) = (top.max(i64::from(area.y0)), (top + f).min(i64::from(area.y1)));
        for y in y0..y1 {
            let Ok(y) = i32::try_from(y) else { continue };
            surface.read_rgba_into(PixelRect::new(x0, y, x1, y + 1), &mut row);
            for (i, p) in row.iter().enumerate() {
                let x = i64::from(x0) + i as i64;
                let column = (x - i64::from(region.x0)) / f - i64::from(texels.x0);
                if let Some(sum) = usize::try_from(column).ok().and_then(|c| sums.get_mut(c)) {
                    let a = p[3];
                    sum[0] += p[0] * a;
                    sum[1] += p[1] * a;
                    sum[2] += p[2] * a;
                    sum[3] += a;
                    sum[4] += 1.0;
                }
            }
        }
        for (o, sum) in texel_row.iter_mut().zip(&sums) {
            if sum[4] > 0.0 && sum[3] > 0.0 {
                *o = [sum[0] / sum[3], sum[1] / sum[3], sum[2] / sum[3], sum[3] / sum[4]];
            }
        }
    }
}

/// The visible pixels of `surface`, as the canvas shows them: the tests' whole-area reference for [`display_image`].
#[cfg(test)]
fn crop_image(surface: &Surface, area: PixelRect, display: &photocraft_engine::display_color::CanvasDisplay) -> ColorImage {
    display_image(area, display, |rect, out| surface.read_rgba_into(rect, out))
}

/// `area` as the canvas shows it, with `read` filling each band's float pixels. Converted in
/// bands of rows, in parallel off wasm, not one row or one whole copy at a time: a row is below
/// `craft-cms`'s parallel threshold, so a per-row conversion ran the display transform (any
/// document profile other than the monitor's, such as an Adobe RGB photo on sRGB) on one core of
/// the UI thread for every edit (#4682).
pub(crate) fn display_image(
    area: PixelRect,
    display: &photocraft_engine::display_color::CanvasDisplay,
    read: impl Fn(PixelRect, &mut [[f32; 4]]) + Sync,
) -> ColorImage {
    let (width, height) = (area.width() as usize, area.height() as usize);
    let mut pixels = vec![Color32::TRANSPARENT; width * height];
    if width == 0 {
        return ColorImage::new([width, height], pixels);
    }
    let band_rows = (CROP_BAND_PIXELS / width).max(1);
    let band = |index: usize, out: &mut [Color32]| {
        let y0 = area.y0.saturating_add((index * band_rows) as i32);
        let rect = PixelRect::new(area.x0, y0, area.x1, y0.saturating_add((out.len() / width) as i32));
        let mut buf = photocraft_compose::Buffer { rect, px: vec![[0.0; 4]; out.len()] };
        read(rect, &mut buf.px);
        let img = display.to_rgba8(&buf);
        for (o, &[r, g, b, a]) in out.iter_mut().zip(img.pixels.as_chunks::<4>().0) {
            *o = Color32::from_rgba_unmultiplied(r, g, b, a);
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        pixels.par_chunks_mut(band_rows * width).enumerate().for_each(|(i, out)| band(i, out));
    }
    #[cfg(target_arch = "wasm32")]
    for (i, out) in pixels.chunks_mut(band_rows * width).enumerate() {
        band(i, out);
    }
    ColorImage::new([width, height], pixels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;

    fn exposure_only(exposure: f32) -> CameraRaw {
        let mut params = CameraRaw::default();
        params.exposure = exposure;
        params
    }

    /// The refinement's inputs: full resolution, so pixel scale 1.
    fn inputs() -> DevelopInputs {
        DevelopInputs { pixel_scale: 1.0, ..Default::default() }
    }

    #[test]
    fn refinement_uses_the_engine_at_source_resolution_and_respects_selection() {
        let area = PixelRect::new(10, -5, 42, 19);
        let mut source = Surface::new(PixelFormat::RGBA8);
        let mut values = vec![0.0; 32 * 24 * 4];
        for (i, p) in values.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            p.copy_from_slice(&[if i % 2 == 0 { 0.1 } else { 0.8 }, 0.3, 0.6, 1.0]);
        }
        source.write_region(area, &values);
        let mut params = CameraRaw::default();
        (params.exposure, params.texture, params.grain_amount) = (0.5, 15.0, 30.0);
        let expected = photocraft_engine::lens_cmds::camera_raw_surface(&source, area, &params, &inputs());
        assert_eq!(develop(&source, None, area, &params, &inputs(), &Coverage::None), expected);
        let mask = Surface::with_default(PixelFormat::GRAY8, &[0.0]);
        assert_eq!(develop(&source, None, area, &params, &inputs(), &Coverage::Selection(mask)), source);
    }
    #[test]
    fn a_raw_open_refines_raw_stage_sliders_from_the_whole_frame() {
        // Shadows on a raw being opened: the full-size refinement retones the whole frame (as
        // Open does) and runs the rest of the pipeline after, instead of the filter's Shadows.
        let area = PixelRect::new(0, 0, 96, 64);
        let mut source = Surface::new(PixelFormat::RGBA16);
        let mut values = vec![0.0; 96 * 64 * 4];
        for (i, p) in values.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let v = if (i % 96) < 48 { 0.12 } else { 0.7 };
            p.copy_from_slice(&[v, v * 0.9, v * 0.8, 1.0]);
        }
        source.write_region(area, &values);
        let curve = [[0.01, 0.01], [0.05, 0.12], [0.3, 0.6], [1.0, 1.0]];
        let mut params = CameraRaw::default();
        (params.shadows, params.exposure) = (80.0, 0.25);
        let refined = develop(&source, Some(&curve), area, &params, &inputs(), &Coverage::None);
        let mut px = vec![[0.0f32; 4]; 96 * 64];
        source.read_rgba_into(area, &mut px);
        let tone = photocraft_io::raw::Tone { shadows: 80.0, ..Default::default() };
        photocraft_io::raw::retone(&mut px, 96, 64, &curve, tone);
        let mut retoned = source.clone();
        let flat: Vec<f32> = px.iter().flatten().copied().collect();
        retoned.write_region(area, &flat);
        let expected = photocraft_engine::lens_cmds::camera_raw_surface(&retoned, area, &exposure_only(0.25), &inputs());
        let (a, b) = (refined.rgba(10, 30), expected.rgba(10, 30));
        assert!(a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 2e-4), "{a:?} vs {b:?}");
        assert!(
            refined.rgba(10, 30)[0] > source.rgba(10, 30)[0] + 0.02,
            "the shadow side is lifted: {:?} from {:?}, retoned {:?}",
            refined.rgba(10, 30),
            source.rgba(10, 30),
            retoned.rgba(10, 30)
        );
        // Without a raw curve Shadows is the filter's.
        let filter = develop(&source, None, area, &params, &inputs(), &Coverage::None);
        assert_eq!(filter, photocraft_engine::lens_cmds::camera_raw_surface(&source, area, &params, &inputs()));
    }

    #[test]
    fn stale_refinements_and_failed_old_revisions_do_not_replace_new_settings() {
        let source = Surface::with_default(PixelFormat::RGBA8, &[0.2, 0.4, 0.6, 1.0]);
        let mut preview = DetailPreview::new(source.clone(), PixelRect::new(0, 0, 16, 16), Coverage::None);
        preview.wanted = Some(Wanted { order: 2, params: CameraRaw::default(), inputs: inputs(), ctx: Context::default() });
        preview.active = true;
        let (tx, rx) = std::sync::mpsc::channel();
        preview.pending = Some((1, rx));
        tx.send(Ok((1, source.clone()))).unwrap();
        preview.poll();
        assert!(preview.result.is_none());
        assert!(!preview.pending());
        let (tx, rx) = std::sync::mpsc::channel();
        preview.pending = Some((1, rx));
        tx.send(Err("old failure".into())).unwrap();
        preview.poll();
        assert_eq!(preview.failed_revision, Some(1), "a failure must not suppress the new revision");
        assert!(preview.error.is_none(), "an old failure is not the wanted develop's");
        let (tx, rx) = std::sync::mpsc::channel();
        preview.pending = Some((2, rx));
        tx.send(Ok((2, source))).unwrap();
        preview.refresh();
        assert_eq!(preview.result.as_ref().map(|(r, _)| *r), Some(2));
        assert!(preview.ready && preview.shown() == Some(2), "the wanted develop is shown");
        assert!(preview.error.is_none());
        preview.unwant();
        assert!(!preview.ready && preview.shown().is_none(), "a view the proxy serves shows the proxy");
        // Zoomed in again with the same settings: the develop that landed is shown, none starts.
        preview.want(&Context::default(), 9, &CameraRaw::default(), &inputs());
        assert!(preview.shown() == Some(2) && !preview.pending(), "the same settings are not developed again");
    }

    /// The refinement is collected outside tests too (#17012): asked for, it lands and is shown, and the
    /// develop of newer settings replaces it.
    #[test]
    fn a_wanted_develop_lands_and_follows_newer_settings() {
        let area = PixelRect::new(0, 0, 8, 4);
        let source = Surface::with_default(PixelFormat::RGBA8, &[0.25, 0.25, 0.25, 1.0]);
        let mut preview = DetailPreview::new(source, area, Coverage::None);
        let ctx = Context::default();
        let wait = |preview: &mut DetailPreview, order: u64| {
            for _ in 0..3000 {
                preview.refresh();
                if preview.shown() == Some(order) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            panic!("the develop {order} never landed: pending {}, error {:?}", preview.pending(), preview.error);
        };
        preview.want(&ctx, 1, &exposure_only(0.0), &inputs());
        wait(&mut preview, 1);
        let first = preview.result.as_ref().unwrap().1.rgba(3, 2);
        // A pan asks again with the same settings: the landed develop stays shown, nothing is developed again.
        preview.want(&ctx, 7, &exposure_only(0.0), &inputs());
        assert!(preview.shown() == Some(1) && !preview.pending(), "the same settings are not developed again");
        preview.want(&ctx, 2, &exposure_only(1.0), &inputs());
        assert!(!preview.ready, "the old develop is not shown for new settings");
        wait(&mut preview, 2);
        let second = preview.result.as_ref().unwrap().1.rgba(3, 2);
        assert!(second[0] > first[0] + 0.1, "the newer settings are developed: {second:?} after {first:?}");
    }

    /// A reduced read averages each texel's block, so a view of the whole refined image (Fit) reads it too.
    #[test]
    fn a_reduced_read_averages_blocks_and_leaves_the_outside_transparent() {
        let area = PixelRect::new(0, 0, 4, 2);
        let mut source = Surface::new(PixelFormat::RGBA8);
        let values: Vec<f32> = (0..8).flat_map(|i| if i % 2 == 0 { [1.0, 1.0, 1.0, 1.0] } else { [0.0, 0.0, 0.0, 1.0] }).collect();
        source.write_region(area, &values);
        let mut doc = photocraft_doc::Document::new("test", photocraft_geom::Size::new(4, 2), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        doc.icc_profile = Some(craft::cms::Builtin::Srgb.profile().to_bytes());
        let display = photocraft_engine::color_cmds::ColorState::default().canvas_display(&doc).unwrap();
        let mut preview = DetailPreview::new(source.clone(), area, Coverage::None);
        preview.result = Some((1, source));
        preview.ready = true;
        let mut out = ColorImage::new([3, 1], vec![Color32::RED; 3]);
        assert!(preview.read(RasterRect::new(0, 0, 6, 2), 2, &display, &mut out));
        let grey = out.pixels[0];
        assert!(grey.r() > 110 && grey.r() < 150 && grey.a() == 255, "white and black average to mid grey: {grey:?}");
        assert_eq!(out.pixels[1], grey);
        assert_eq!(out.pixels[2], Color32::TRANSPARENT, "past the refined area");
    }

    #[test]
    fn full_resolution_sampling_and_crops_keep_fine_detail_at_every_supported_depth() {
        use photocraft_color::{ColorMode, PixelFormat, SampleType};
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
                let format = PixelFormat { mode, sample: depth, alpha: true };
                let area = PixelRect::new(-12, 8, 28, 38);
                let mut source = Surface::new(format);
                let channels = format.channels();
                let mut pixels = vec![0.0; 40 * 30 * channels];
                for (i, pixel) in pixels.chunks_exact_mut(channels).enumerate() {
                    pixel.fill(if i % 2 == 0 { 0.1 } else { 0.9 });
                    pixel[channels - 1] = 1.0;
                }
                source.write_region(area, &pixels);
                let mut params = CameraRaw::default();
                (params.exposure, params.sharpen_amount, params.grain_amount) = (0.5, 20.0, 30.0);
                let full = develop(&source, None, area, &params, &inputs(), &Coverage::None);
                let expected = photocraft_engine::lens_cmds::camera_raw_surface(&source, area, &params, &inputs());
                assert_eq!(full, expected);
                let mut preview = DetailPreview::new(source, area, Coverage::None);
                preview.result = Some((1, full));
                preview.ready = true;
                let dark = preview.sample([0.0, 0.0], 1, false, &params).unwrap();
                let light = preview.sample([1.0 / 40.0, 0.0], 1, false, &params).unwrap();
                assert!(light[0] - dark[0] > 0.5, "native alternating pixels survive: {mode:?}/{depth:?}");
                let mut doc =
                    photocraft_doc::Document::new("test", photocraft_geom::Size::new(40, 30), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
                doc.icc_profile = Some(craft::cms::Builtin::Srgb.profile().to_bytes());
                let display = photocraft_engine::color_cmds::ColorState::default().canvas_display(&doc).unwrap();
                assert_eq!(crop_image(&expected, PixelRect::new(-12, 8, -10, 9), &display).size, [2, 1]);
                assert!(preview.sample([f32::NAN, 0.0], 1, false, &params).is_none());
                assert!(preview.sample([0.5, 0.5], 2, false, &params).is_none());
            }
        }
    }

    #[test]
    fn refined_crop_uses_linear_document_display_encoding() {
        let area = PixelRect::new(0, 0, 1, 1);
        let mut source =
            Surface::new(photocraft_color::PixelFormat { mode: photocraft_color::ColorMode::Rgb, sample: photocraft_color::SampleType::F32, alpha: true });
        source.write_region(area, &[0.25, 0.25, 0.25, 1.0]);
        let mut doc = photocraft_doc::Document::new("test", photocraft_geom::Size::new(1, 1), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::F32);
        doc.icc_profile = Some(craft::cms::Builtin::LinearSrgb.profile().to_bytes());
        let mut color = photocraft_engine::color_cmds::ColorState::default();
        color.settings.monitor_profile = "srgb".into();
        let display = color.canvas_display(&doc).unwrap();
        let image = crop_image(&source, area, &display);
        assert!(image.pixels[0].r() > 130, "linear 0.25 should be shown as encoded sRGB");
    }

    #[test]
    fn huge_sparse_images_keep_the_proxy_without_starting_an_unbounded_worker() {
        let source = Surface::new(PixelFormat::RGBA8);
        let mut preview = DetailPreview::new(source, PixelRect::new(0, 0, 8001, 8000), Coverage::None);
        preview.request(&Context::default(), 1, &exposure_only(1.0), &inputs());
        assert!(!preview.pending());
        assert!(preview.error.as_deref().unwrap().contains("64 megapixels"));
        assert_eq!(preview.failed_revision, Some(1));
    }

    #[test]
    fn read_converts_only_the_asked_part_and_clipping_follows_the_refined_area() {
        let area = PixelRect::new(4, 2, 8, 4);
        let mut source = Surface::new(PixelFormat::RGBA8);
        let values: Vec<f32> = (0..8).flat_map(|i| if i % 4 == 0 { [1.0, 1.0, 1.0, 1.0] } else { [0.5, 0.5, 0.5, 1.0] }).collect();
        source.write_region(area, &values);
        let mut doc = photocraft_doc::Document::new("test", photocraft_geom::Size::new(16, 8), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        doc.icc_profile = Some(craft::cms::Builtin::Srgb.profile().to_bytes());
        let display = photocraft_engine::color_cmds::ColorState::default().canvas_display(&doc).unwrap();
        let mut preview = DetailPreview::new(source.clone(), area, Coverage::None);
        let mut out = ColorImage::new([4, 2], vec![Color32::RED; 8]);
        assert!(!preview.read(RasterRect::new(2, 2, 6, 4), 1, &display, &mut out), "nothing refined yet: the caller reads the proxy");
        assert!(preview.clipping().is_none());
        preview.result = Some((3, source));
        preview.ready = true;
        // Columns 2 and 3 lie left of the area and stay transparent; column 4 is the first refined one.
        assert!(preview.read(RasterRect::new(2, 2, 6, 4), 1, &display, &mut out));
        assert_eq!(out.pixels[0], Color32::TRANSPARENT);
        assert_eq!(out.pixels[1], Color32::TRANSPARENT);
        assert_eq!(out.pixels[2], Color32::WHITE);
        assert_ne!(out.pixels[3], Color32::WHITE);
        let (mask, revision) = preview.clipping().unwrap();
        assert_eq!((revision, mask.size), (3, [4, 2]));
        assert_eq!(mask.at(0, 0).highlights, craft::imaging::scopes::ChannelMask::RGB);
        assert!(mask.at(1, 0).highlights.is_empty());
    }

    #[test]
    fn banded_crop_matches_one_conversion_of_the_whole_area() {
        // Several bands, a ragged last one and an area off the image origin, through a real
        // transform (an Adobe RGB document on an sRGB monitor) and without one.
        let area = PixelRect::new(-3, 5, 1021, 5 + 2 * (CROP_BAND_PIXELS / 1024) as i32 + 7);
        let mut source = Surface::new(PixelFormat { mode: photocraft_color::ColorMode::Rgb, sample: photocraft_color::SampleType::U8, alpha: true });
        let n = (area.width() * area.height()) as usize;
        let px: Vec<f32> = (0..n).flat_map(|i| [(i % 251) as f32 / 255.0, (i % 241) as f32 / 255.0, (i % 239) as f32 / 255.0, (i % 7) as f32 / 6.0]).collect();
        source.write_region(area, &px);
        for profile in [craft::cms::Builtin::AdobeRgbCompat, craft::cms::Builtin::Srgb] {
            let mut doc =
                photocraft_doc::Document::new("test", photocraft_geom::Size::new(1024, 1024), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
            doc.icc_profile = Some(profile.profile().to_bytes());
            let mut color = photocraft_engine::color_cmds::ColorState::default();
            color.settings.monitor_profile = "srgb".into();
            let display = color.canvas_display(&doc).unwrap();
            assert_eq!(display.transform.is_some(), profile == craft::cms::Builtin::AdobeRgbCompat);
            let mut whole = photocraft_compose::Buffer { rect: area, px: vec![[0.0; 4]; n] };
            source.read_rgba_into(area, &mut whole.px);
            let expected = display.to_rgba8(&whole);
            let image = crop_image(&source, area, &display);
            assert_eq!(image.size, [area.width() as usize, area.height() as usize]);
            let expected: Vec<Color32> = expected.pixels.as_chunks::<4>().0.iter().map(|&[r, g, b, a]| Color32::from_rgba_unmultiplied(r, g, b, a)).collect();
            assert_eq!(image.pixels, expected, "{profile:?}");
        }
        assert_eq!(
            crop_image(
                &source,
                PixelRect::new(0, 0, 0, 4),
                &photocraft_engine::color_cmds::ColorState::default()
                    .canvas_display(&photocraft_doc::Document::new(
                        "t",
                        photocraft_geom::Size::new(1, 1),
                        photocraft_doc::ColorMode::Rgb,
                        photocraft_doc::SampleType::U8
                    ))
                    .unwrap()
            )
            .size,
            [0, 4]
        );
    }
}
