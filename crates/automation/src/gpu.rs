//! Native GPU renderer adapter and session integration for headless automation.

use std::sync::Arc;
use std::time::Duration;

use photocraft_doc::{DocId, Document, Rect};
use photocraft_gpu::context::{AbandonedInit, GpuContextError, NativeContext, NativeContextOptions};
use photocraft_gpu::{Compositor, readback};
use photocraft_io::{BandRenderer, IoError, RenderBand, RenderError};

use crate::AutomationError;

/// Requests a GPU context, returning the initialization thread when the request times out.
type RequestContext = fn(&NativeContextOptions) -> (Result<NativeContext, GpuContextError>, Option<AbandonedInit>);

/// Headless GPU renderer implementing [`BandRenderer`] with device and compositor reuse.
pub struct GpuRenderer {
    context: Option<Arc<NativeContext>>,
    compositor: Option<Compositor>,
    last_doc_id: Option<DocId>,
    options: NativeContextOptions,
    max_staging_bytes: usize,
    init_error: Option<String>,
    diagnostics: Vec<String>,
    adapter_reported: bool,
    abandoned_init: Option<AbandonedInit>,
    request: RequestContext,
}

impl GpuRenderer {
    /// Create and immediately initialize a GPU renderer.
    ///
    /// When initialization times out, this waits up to 30 seconds for the initialization thread
    /// to exit before it returns the error.
    pub fn new() -> Result<Self, AutomationError> {
        let mut r = Self::lazy();
        if let Err(e) = r.init_now() {
            // `init_now` has already waited for the initialization thread; dropping `r` does not wait again.
            r.abandoned_init = None;
            return Err(AutomationError::Other(format!("failed to initialize GPU renderer: {e}")));
        }
        Ok(r)
    }

    /// Initialize now. When initialization times out, wait up to [`ABANDONED_INIT_WAIT`] for its
    /// thread to exit before returning the error.
    fn init_now(&mut self) -> Result<(), RenderError> {
        let res = self.ensure_init();
        if res.is_err()
            && let Some(init) = &self.abandoned_init
        {
            init.wait(ABANDONED_INIT_WAIT);
        }
        res
    }

    /// Create a GPU renderer that lazily requests an adapter on first render.
    pub fn lazy() -> Self {
        Self {
            context: None,
            compositor: None,
            last_doc_id: None,
            options: NativeContextOptions::default(),
            max_staging_bytes: readback::DEFAULT_MAX_STAGING_BYTES,
            init_error: None,
            diagnostics: Vec::new(),
            adapter_reported: false,
            abandoned_init: None,
            request: NativeContext::request_tracked,
        }
    }

    /// Create with custom [`NativeContextOptions`].
    pub fn with_options(options: NativeContextOptions) -> Self {
        Self {
            context: None,
            compositor: None,
            last_doc_id: None,
            options,
            max_staging_bytes: readback::DEFAULT_MAX_STAGING_BYTES,
            init_error: None,
            diagnostics: Vec::new(),
            adapter_reported: false,
            abandoned_init: None,
            request: NativeContext::request_tracked,
        }
    }

    /// Set maximum in-flight staging bytes for bounded readback.
    pub fn set_max_staging_bytes(&mut self, bytes: usize) {
        self.max_staging_bytes = bytes;
    }

    fn push_diag(&mut self, msg: String) {
        const MAX_DIAGNOSTICS: usize = 16;
        if self.diagnostics.len() < MAX_DIAGNOSTICS && !self.diagnostics.contains(&msg) {
            self.diagnostics.push(msg);
        }
    }

    fn ensure_init(&mut self) -> Result<(), RenderError> {
        if let Some(err) = &self.init_error {
            return Err(RenderError::Unavailable(err.clone()));
        }

        if self.context.is_none() {
            let (requested, abandoned) = (self.request)(&self.options);
            self.abandoned_init = abandoned;
            match requested {
                Ok(ctx) => match ctx.create_compositor() {
                    Ok(comp) => {
                        if !self.adapter_reported {
                            self.adapter_reported = true;
                            let is_software = ctx.is_software();
                            let adapter_type = if is_software { "software rasterizer" } else { "hardware" };
                            let msg = format!("GPU adapter: {} ({:?}, {adapter_type})", ctx.adapter_name(), ctx.backend());
                            self.push_diag(msg);
                        }
                        self.context = Some(Arc::new(ctx));
                        self.compositor = Some(comp);
                    }
                    Err(e) => {
                        let msg = format!("could not build GPU compositor: {e}");
                        self.push_diag(msg.clone());
                        self.init_error = Some(msg.clone());
                        return Err(RenderError::Unavailable(msg));
                    }
                },
                Err(e) => {
                    let msg = format!("no GPU adapter available: {e}");
                    self.push_diag(msg.clone());
                    self.init_error = Some(msg.clone());
                    return Err(RenderError::Unavailable(msg));
                }
            }
        }
        Ok(())
    }

    /// The underlying GPU context if initialized.
    pub fn context(&self) -> Option<&Arc<NativeContext>> {
        self.context.as_ref()
    }

    /// The initialization thread that was still running when initialization timed out.
    pub fn abandoned_init(&self) -> Option<&AbandonedInit> {
        self.abandoned_init.as_ref()
    }

    /// Whether this renderer is healthy.
    pub fn is_healthy(&self) -> bool {
        self.context.as_ref().is_some_and(|c| c.is_healthy())
    }

    /// Whether the active adapter is a software rasterizer.
    pub fn is_software(&self) -> bool {
        self.context.as_ref().is_some_and(|c| c.is_software())
    }

    /// Adapter name reported by the driver.
    pub fn adapter_name(&self) -> Option<&str> {
        self.context.as_ref().map(|c| c.adapter_name())
    }

    /// Invalidate compositor caches for a closed or modified document.
    pub fn forget_doc(&mut self, doc: DocId) {
        if let Some(comp) = &mut self.compositor {
            comp.forget_doc(doc);
        }
        if self.last_doc_id == Some(doc) {
            self.last_doc_id = None;
        }
    }

    /// Invalidate all compositor caches (resident pages, effect maps, pattern textures).
    pub fn reset_cache(&mut self) {
        if let Some(comp) = &mut self.compositor {
            comp.reset_cache();
        }
        self.last_doc_id = None;
    }
}

/// How long dropping a renderer waits for its timed-out initialization thread to leave the
/// graphics driver.
const ABANDONED_INIT_WAIT: Duration = Duration::from_secs(30);

impl Drop for GpuRenderer {
    fn drop(&mut self) {
        if let Some(init) = self.abandoned_init.take() {
            init.wait(ABANDONED_INIT_WAIT);
        }
    }
}

impl BandRenderer for GpuRenderer {
    fn render_bands(
        &mut self,
        doc: &Document,
        rect: Rect,
        band_rows: i32,
        ctl: &photocraft_raster::Interrupt<'_>,
        sink: &mut dyn FnMut(RenderBand) -> Result<(), IoError>,
    ) -> Result<(), RenderError> {
        let max_staging = self.max_staging_bytes;
        self.ensure_init()?;

        let ctx = self.context.clone().ok_or_else(|| RenderError::Unavailable("GPU context missing".into()))?;
        if !ctx.is_healthy() {
            let fault = ctx.health().fault().map_or_else(|| "GPU device lost".into(), |f| f.to_string());
            return Err(RenderError::Device(fault));
        }

        let comp = self.compositor.as_mut().ok_or_else(|| RenderError::Unavailable("GPU compositor missing".into()))?;
        if let Err(unsupported) = comp.supports(doc) {
            self.push_diag(format!("GPU rendering unsupported for document: {}", unsupported.0));
            return Err(RenderError::Unsupported(unsupported.0));
        }

        if self.last_doc_id != Some(doc.id) {
            if let Some(prev) = self.last_doc_id {
                comp.forget_doc(prev);
            }
            self.last_doc_id = Some(doc.id);
        }

        let device = ctx.device();
        let queue = ctx.queue();

        readback::render_bands(comp, device, queue, doc, rect, band_rows, max_staging, ctl, |band_rect, pixels| sink(RenderBand { rect: band_rect, pixels }))
            .map_err(|e| {
                let render_err = match e {
                    readback::ReadbackError::Unsupported(s) => RenderError::Unsupported(s),
                    readback::ReadbackError::DeviceLost(s) => RenderError::Device(s),
                    readback::ReadbackError::AllocationFailed(s) => RenderError::Unavailable(s),
                    readback::ReadbackError::MappingFailed(s) => RenderError::Device(s),
                    readback::ReadbackError::Cancelled => RenderError::Cancelled,
                    readback::ReadbackError::Execution(s) => RenderError::Execution(s),
                    readback::ReadbackError::Sink(io) => RenderError::sink(io),
                };
                if render_err.is_retryable() {
                    self.push_diag(format!("GPU readback error: {render_err}"));
                }
                render_err
            })
    }

    fn drain_diagnostics(&mut self) -> Vec<String> {
        std::mem::take(&mut self.diagnostics)
    }

    fn push_diagnostic(&mut self, diagnostic: String) {
        self.push_diag(diagnostic);
    }

    fn forget_document(&mut self, doc: DocId) {
        self.forget_doc(doc);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// A timed-out renderer whose initialization thread runs until something starts waiting for
    /// it, and a flag the thread sets only when a wait released it. The thread also ends without
    /// setting the flag once every handle to it is dropped.
    fn renderer_with_held_init() -> (GpuRenderer, Arc<AtomicBool>) {
        let finished = Arc::new(AtomicBool::new(false));
        let flag = finished.clone();
        let init = AbandonedInit::spawn(move |waiting| {
            if waiting.recv().is_ok() {
                flag.store(true, Ordering::SeqCst);
            }
        })
        .unwrap();
        let mut r = GpuRenderer::lazy();
        r.init_error = Some("no GPU adapter available: timed out".into());
        r.abandoned_init = Some(init);
        (r, finished)
    }

    #[test]
    fn eager_initialization_waits_for_a_timed_out_thread_before_returning() {
        let (mut r, finished) = renderer_with_held_init();
        let init = r.abandoned_init().cloned().unwrap();
        assert!(!init.is_finished());
        assert!(r.init_now().is_err());
        assert!(finished.load(Ordering::SeqCst), "eager initialization waits for its initialization thread");
        assert!(init.is_finished());
    }

    #[test]
    fn dropping_a_renderer_waits_for_its_running_initialization_thread() {
        let (r, finished) = renderer_with_held_init();
        let init = r.abandoned_init().cloned().unwrap();
        assert!(!init.is_finished());
        drop(r);
        assert!(finished.load(Ordering::SeqCst), "dropping the renderer waits for its initialization thread");
        assert!(init.is_finished());
    }

    #[test]
    fn a_timed_out_initialization_keeps_its_thread() {
        // The thread runs until the renderer waits for it, so the request is still running when it returns.
        fn timed_out(_: &NativeContextOptions) -> (Result<NativeContext, GpuContextError>, Option<AbandonedInit>) {
            let init = AbandonedInit::spawn(|waiting| {
                let _ = waiting.recv();
            });
            (Err(GpuContextError::NoAdapter("timed out".into())), init.ok())
        }
        let mut r = GpuRenderer::lazy();
        r.request = timed_out;
        assert!(matches!(r.ensure_init(), Err(RenderError::Unavailable(ref msg)) if msg.contains("timed out")));
        let init = r.abandoned_init().cloned().expect("a timed-out renderer keeps its initialization thread");
        assert!(!init.is_finished());
        drop(r);
        assert!(init.is_finished());
    }
}
