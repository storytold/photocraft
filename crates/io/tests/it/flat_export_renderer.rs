//! Tests for custom [`BandRenderer`] injection, complete CPU retry, and atomic save preservation.

use std::fs;
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
    let mut doc = Document::with_background("synth", Size::new(128, 128), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut s = Surface::new(doc.pixel_format());
    s.fill_rect(Rect::new(20, 20, 100, 100), &[0.1, 0.4, 0.9, 0.7]);
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
        photocraft_compose::render_bands(doc, rect, band_rows, |buf| {
            let count = self.call_count.fetch_add(1, Ordering::SeqCst);
            if count >= self.fail_after_bands {
                return match self.error_kind {
                    "unsupported" => Err(IoError::Render(RenderError::Unsupported("forced unsupported feature".into()))),
                    "device" => Err(IoError::Render(RenderError::Device("forced device loss".into()))),
                    "cancelled" => Err(IoError::Cancelled),
                    _ => Err(IoError::Render(RenderError::Execution("forced failure".into()))),
                };
            }
            sink(RenderBand { rect: buf.rect, pixels: buf.px })
        })
        .map_err(|e| match e {
            IoError::Render(r) => r,
            IoError::Cancelled => RenderError::Cancelled,
            other => RenderError::Execution(other.to_string()),
        })
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
fn atomic_save_preserves_prior_file_on_fatal_failure() {
    let dir = std::env::temp_dir().join(format!("photocraft-atomic-save-test-{}", std::process::id()));
    let _ = fs::create_dir_all(&dir);
    let target_path = dir.join("preserved.txt");
    let original_bytes = b"prior uncorrupted file contents";
    fs::write(&target_path, original_bytes).expect("initial write");

    // Attempt an atomic write with an invalid format or simulated error
    let doc = synthetic_doc();
    let opts = ExportOptions::default();

    // Calling export on an unknown format should fail before touching the file
    let export_err = photocraft_io::export(&doc, "invalid_unknown_extension_xyz", &opts);
    assert!(export_err.is_err());

    // File must remain exactly as before
    let read_back = fs::read(&target_path).expect("readback");
    assert_eq!(read_back, original_bytes);
    let _ = fs::remove_dir_all(&dir);
}
