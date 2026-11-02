//! Renderer contract and band provider interface for headless composition and export.

use photocraft_doc::Document;
use photocraft_geom::Rect;

use crate::IoError;

/// A horizontal band of rendered straight-alpha RGBA float samples (`[R, G, B, A]`).
///
/// Semantics:
/// - Color space: working color space of the document composite (sRGB for RGB documents,
///   sRGB working composite for Lab and Grayscale, or profile-converted composite for CMYK).
/// - Alpha representation: straight (unpremultiplied) alpha in `pixels[..][3]`, where `1.0` is
///   fully opaque and `0.0` is fully transparent. Color channels `pixels[..][0..3]` carry
///   unpremultiplied sample values.
/// - Memory layout: contiguous row-major vector of length `rect.width() * rect.height()`.
///   Sample at `(x, y)` is located at `(y - rect.y0) * rect.width() + (x - rect.x0)`.
/// - Geometry: `rect` spans document coordinates, matching full document width across horizontal
///   slices from top (`y = 0`) to bottom (`y = height`).
#[derive(Clone, Debug)]
pub struct RenderBand {
    /// Document-coordinate bounding rectangle of this band.
    pub rect: Rect,
    /// Interleaved straight-alpha samples `[r, g, b, a]`, row-major.
    pub pixels: Vec<[f32; 4]>,
}

/// Errors raised by a [`BandRenderer`].
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// The document or requested feature is not supported by this renderer.
    #[error("unsupported document or feature: {0}")]
    Unsupported(String),
    /// The rendering backend, hardware adapter, or pipeline is unavailable.
    #[error("rendering backend unavailable: {0}")]
    Unavailable(String),
    /// A GPU device error, device loss, or driver crash occurred.
    #[error("rendering device failure: {0}")]
    Device(String),
    /// The rendering operation was cancelled by caller request.
    #[error("rendering was cancelled")]
    Cancelled,
    /// Rendering execution failed within the rendering provider.
    #[error("rendering failed: {0}")]
    Execution(String),
    /// Downstream sink callback returned an I/O, encoder, or cancellation error.
    #[error("sink error: {0}")]
    Sink(#[source] Box<IoError>),
}

impl RenderError {
    /// Create a new sink error wrapping [`IoError`].
    pub fn sink(err: IoError) -> Self {
        match err {
            IoError::Cancelled => RenderError::Cancelled,
            other => RenderError::Sink(Box::new(other)),
        }
    }

    /// Whether this error represents an availability or capability limitation of the renderer,
    /// where a complete export attempt should restart on the reference CPU compositor.
    ///
    /// Sink errors (codec failures, disk full, cancellation) are fatal to the export and
    /// MUST NEVER be retried on the CPU compositor.
    pub fn is_retryable(&self) -> bool {
        match self {
            RenderError::Unsupported(_) | RenderError::Unavailable(_) | RenderError::Device(_) | RenderError::Execution(_) => true,
            RenderError::Cancelled | RenderError::Sink(_) => false,
        }
    }
}

/// A provider of rendered composite bands for flat export.
pub trait BandRenderer {
    /// Renders `doc` over `rect` in horizontal bands, passing each band to `sink`.
    /// `band_rows`: preferred rows per band (0 to let provider choose).
    fn render_bands(&mut self, doc: &Document, rect: Rect, band_rows: i32, sink: &mut dyn FnMut(RenderBand) -> Result<(), IoError>) -> Result<(), RenderError>;
}

/// Default reference band renderer using `photocraft-compose`.
#[derive(Clone, Copy, Debug, Default)]
pub struct CpuRenderer;

impl BandRenderer for CpuRenderer {
    fn render_bands(&mut self, doc: &Document, rect: Rect, band_rows: i32, sink: &mut dyn FnMut(RenderBand) -> Result<(), IoError>) -> Result<(), RenderError> {
        photocraft_compose::render_bands(doc, rect, band_rows, |buf| sink(RenderBand { rect: buf.rect, pixels: buf.px })).map_err(RenderError::sink)
    }
}
