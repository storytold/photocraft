//! Tests for custom [`BandRenderer`] injection, complete CPU retry, and renderer error handling.

use std::sync::atomic::{AtomicUsize, Ordering};

use photocraft_codecs::Format;
use photocraft_color::{BlendMode, Color, ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_geom::{Rect, Size};
use photocraft_io::{
    BandRenderer, CpuRenderer, ExportOptions, IoError, RenderBand, RenderError, export_flat, export_flat_with_renderer, export_png_streamed_with_renderer,
};
use photocraft_raster::{Interrupt, Surface};

fn synthetic_doc() -> Document {
    let mut doc = Document::with_background("synth", Size::new(128, 300), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut s = Surface::new(doc.pixel_format());
    s.fill_rect(Rect::new(20, 20, 100, 260), &[0.1, 0.4, 0.9, 0.7]);
    let mut top = Layer::new("top", LayerContent::Raster(s));
    top.opacity = 0.8;
    top.blend = BlendMode::Multiply;
    doc.layers.push(top);
    doc
}

struct FailingRenderer {
    fail_after_bands: usize,
    call_count: AtomicUsize,
    error_kind: &'static str,
}

impl FailingRenderer {
    fn new(fail_after_bands: usize, error_kind: &'static str) -> Self {
        Self { fail_after_bands, call_count: AtomicUsize::new(0), error_kind }
    }
}

impl BandRenderer for FailingRenderer {
    fn render_bands(&mut self, doc: &Document, rect: Rect, band_rows: i32, sink: &mut dyn FnMut(RenderBand) -> Result<(), IoError>) -> Result<(), RenderError> {
        let rows = if band_rows > 0 { band_rows } else { 32 };
        photocraft_compose::render_bands(doc, rect, rows, |buf| {
            let count = self.call_count.fetch_add(1, Ordering::SeqCst);
            if count >= self.fail_after_bands {
                return match self.error_kind {
                    "unsupported" => Err(IoError::Render(RenderError::Unsupported("forced unsupported feature".into()))),
                    "device" => Err(IoError::Render(RenderError::Device("forced device loss".into()))),
                    "cancelled" => Err(IoError::Cancelled),
                    _ => Err(IoError::Render(RenderError::Execution("forced failure".into()))),
                };
            }
            sink(RenderBand { rect: buf.rect, pixels: buf.into_px() })
        })
        .map_err(|e| match e {
            IoError::Render(r) => r,
            IoError::Cancelled => RenderError::Cancelled,
            other => RenderError::sink(other),
        })
    }
}

struct GeometryViolatingRenderer {
    violation: &'static str,
}

impl BandRenderer for GeometryViolatingRenderer {
    fn render_bands(
        &mut self,
        _doc: &Document,
        rect: Rect,
        _band_rows: i32,
        sink: &mut dyn FnMut(RenderBand) -> Result<(), IoError>,
    ) -> Result<(), RenderError> {
        let (w, h) = (rect.width(), rect.height());
        match self.violation {
            "wrong_pixels" => {
                sink(RenderBand { rect: Rect::new(0, 0, w as i32, h as i32), pixels: vec![[0.0; 4]; (w as usize * h as usize).saturating_sub(10)] })
                    .map_err(RenderError::sink)
            }
            "out_of_order" => {
                // Yield band starting at y=10 instead of 0
                sink(RenderBand { rect: Rect::new(0, 10, w as i32, 20), pixels: vec![[0.0; 4]; w as usize * 10] }).map_err(RenderError::sink)
            }
            "wrong_width" => {
                sink(RenderBand { rect: Rect::new(0, 0, (w as i32).saturating_sub(5), 10), pixels: vec![[0.0; 4]; (w as usize).saturating_sub(5) * 10] })
                    .map_err(RenderError::sink)
            }
            _ => Ok(()),
        }
    }
}

#[test]
fn custom_cpu_renderer_matches_default_export() {
    let doc = synthetic_doc();
    let opts = ExportOptions::default();

    let def = export_flat(&doc, Format::Png, &opts).expect("default export");
    let mut custom = CpuRenderer;
    let res = export_flat_with_renderer(&doc, Format::Png, &opts, &Interrupt::NONE, Some(&mut custom)).expect("custom export");

    assert_eq!(def.bytes, res.bytes);
    assert_eq!(def.warnings, res.warnings);
}

#[test]
fn forced_unsupported_renderer_restarts_complete_export_on_cpu() {
    let doc = synthetic_doc();
    let opts = ExportOptions::default();

    let expected = export_flat(&doc, Format::Png, &opts).expect("baseline CPU export");

    let mut failing = FailingRenderer::new(0, "unsupported");
    let res = export_flat_with_renderer(&doc, Format::Png, &opts, &Interrupt::NONE, Some(&mut failing)).expect("export should retry and succeed on CPU");

    assert_eq!(expected.bytes, res.bytes, "output after retry must match clean CPU output");
    assert!(
        res.warnings.iter().any(|w| w.contains("GPU rendering unavailable") && w.contains("falling back to CPU")),
        "fallback warning must be reported: {:?}",
        res.warnings
    );
}

#[test]
fn forced_mid_band_failure_restarts_complete_export_on_cpu() {
    let doc = synthetic_doc();
    let opts = ExportOptions::default();

    let expected = export_flat(&doc, Format::Png, &opts).expect("baseline CPU export");

    // Fail on the second band (mid-render)
    let mut failing = FailingRenderer::new(1, "device");
    let res = export_flat_with_renderer(&doc, Format::Png, &opts, &Interrupt::NONE, Some(&mut failing)).expect("export should restart completely on CPU");

    assert_eq!(expected.bytes, res.bytes, "output after mid-band restart must match pure CPU output");
    assert!(res.warnings.iter().any(|w| w.contains("falling back to CPU")), "fallback warning must be present: {:?}", res.warnings);
}

#[test]
fn cancellation_is_not_retried() {
    let doc = synthetic_doc();
    let opts = ExportOptions::default();

    let mut failing = FailingRenderer::new(0, "cancelled");
    let res = export_flat_with_renderer(&doc, Format::Png, &opts, &Interrupt::NONE, Some(&mut failing));

    assert!(matches!(res, Err(IoError::Cancelled)), "cancelled render must not restart on CPU");
}

#[test]
fn encoder_and_sink_failures_are_not_retried_on_cpu() {
    struct SinkFailingRenderer;
    impl BandRenderer for SinkFailingRenderer {
        fn render_bands(
            &mut self,
            _doc: &Document,
            _rect: Rect,
            _band_rows: i32,
            _sink: &mut dyn FnMut(RenderBand) -> Result<(), IoError>,
        ) -> Result<(), RenderError> {
            Err(RenderError::sink(IoError::Unsupported("test sink failure".into())))
        }
    }

    let doc = synthetic_doc();
    let opts = ExportOptions::default();
    // A non-cancellation sink error with no interrupt: a CPU retry would succeed with `Ok`, so
    // this assertion fails if the error is ever retried on the CPU.
    let mut renderer = SinkFailingRenderer;
    let res = export_flat_with_renderer(&doc, Format::Png, &opts, &Interrupt::NONE, Some(&mut renderer));
    assert!(matches!(res, Err(IoError::Unsupported(_))), "sink error must propagate without a CPU retry: {res:?}");
}

#[test]
fn band_geometry_violations_are_rejected() {
    let doc = synthetic_doc();
    let opts = ExportOptions::default();

    for violation in ["wrong_pixels", "out_of_order", "wrong_width"] {
        let mut renderer = GeometryViolatingRenderer { violation };
        let res = export_flat_with_renderer(&doc, Format::Png, &opts, &Interrupt::NONE, Some(&mut renderer));
        assert!(
            matches!(res, Err(IoError::Render(RenderError::Execution(_)))),
            "violation '{violation}' must be rejected before codec consumes corrupt data: {res:?}"
        );
    }
}

#[test]
fn streamed_png_retries_on_cpu_upon_renderer_failure() {
    let doc = synthetic_doc();
    let opts = ExportOptions::default();

    let expected = export_png_streamed_with_renderer(&doc, &opts, &Interrupt::NONE, 16, None).expect("baseline streamed PNG");

    let mut failing = FailingRenderer::new(1, "device");
    let res = export_png_streamed_with_renderer(&doc, &opts, &Interrupt::NONE, 16, Some(&mut failing)).expect("streamed PNG should retry on CPU");

    assert_eq!(expected.bytes, res.bytes);
    assert!(res.warnings.iter().any(|w| w.contains("falling back to CPU")));
}

#[test]
fn unknown_export_format_is_rejected() {
    let doc = synthetic_doc();
    let opts = ExportOptions::default();

    // `export` takes a name or bare extension; an unknown one fails before any codec runs.
    let export_err = photocraft_io::export(&doc, "invalid_unknown_extension_xyz", &opts);
    assert!(export_err.is_err());
}
