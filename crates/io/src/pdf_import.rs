use crate::{ImportResult, IoError};
use std::panic;

/// Maximum dimension for a rendered PDF page (matches SVG's MAX_SIDE).
pub const MAX_SIDE: u32 = 16_384;
/// Maximum pixels for a rendered PDF page (matches SVG's MAX_RASTER_PIXELS).
pub const MAX_RASTER_PIXELS: u64 = 24_000_000;

/// Import a PDF file. Renders the first page to an image and creates a document from it.
pub fn import_pdf(name: &str, bytes: &[u8], _max_group_depth: usize) -> Result<ImportResult, IoError> {
    import_pdf_page(name, bytes, 0)
}

/// Import a specific page from a PDF file (0-indexed).
fn import_pdf_page(name: &str, bytes: &[u8], page_index: usize) -> Result<ImportResult, IoError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Wrap in catch_unwind to handle missing pdfium library (which causes a panic in pdfium-render)
        match panic::catch_unwind(|| import_pdf_page_impl(name, bytes, page_index)) {
            Ok(result) => result,
            Err(e) => {
                // Check if it's the "No Pdfium library found" panic
                if let Some(s) = e.downcast_ref::<String>()
                    && s.contains("No Pdfium library found")
                {
                    return Err(IoError::Other("PDF import requires pdfium library (not found at runtime)".to_string()));
                }
                if let Some(s) = e.downcast_ref::<&str>()
                    && s.contains("No Pdfium library found")
                {
                    return Err(IoError::Other("PDF import requires pdfium library (not found at runtime)".to_string()));
                }
                // Re-panic for any other panic
                panic::resume_unwind(e);
            }
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (name, bytes, page_index);
        Err(IoError::Other("PDF import not supported on wasm32".to_string()))
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn import_pdf_page_impl(name: &str, bytes: &[u8], page_index: usize) -> Result<ImportResult, IoError> {
    use pdfium_render::prelude::*;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_doc::{Document, Layer, LayerContent};
    use photocraft_geom::{Rect, Size};
    use photocraft_raster::Surface;

    // Cargo places downloaded PDFium beside the executable, which may differ from the working directory.
    let bindings = std::env::current_exe().ok().and_then(|executable| executable.parent().map(|directory| directory.to_path_buf())).and_then(|directory| {
        let packaged_dirs =
            [directory.parent().map(|contents| contents.join("Frameworks")), directory.parent().map(|prefix| prefix.join("lib").join("photocraft"))];
        packaged_dirs
            .into_iter()
            .flatten()
            .chain(directory.ancestors().take(3).map(std::path::Path::to_path_buf))
            .find_map(|candidate| Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(&candidate)).ok())
    });
    let pdfium = bindings.map(Pdfium::new).unwrap_or_default();

    // Load the PDF from the byte slice
    let document = pdfium.load_pdf_from_byte_slice(bytes, None).map_err(|e| IoError::Other(format!("failed to load PDF: {e}")))?;

    // Get the number of pages
    let page_count = document.pages().len();
    if page_count == 0 {
        return Err(IoError::Other("PDF has no pages".to_string()));
    }

    // Clamp page index
    let page_count_i32 = page_count;
    let page_index_i32 = i32::try_from(page_index).unwrap_or(page_count_i32 - 1).min(page_count_i32 - 1).max(0);

    // Get the page
    let page = document.pages().get(page_index_i32).map_err(|_| IoError::Other(format!("failed to get page {page_index_i32} of {page_count}")))?;

    // Get page dimensions in points (1/72 inch)
    let width_pt = page.width();
    let height_pt = page.height();

    // Convert to pixels at 72 DPI (1 point = 1 pixel at 72 DPI)
    // PdfPoints wraps f32, access via .value
    let mut width_px = width_pt.value.ceil() as u32;
    let mut height_px = height_pt.value.ceil() as u32;

    // Scale down if too large (matching SVG's behavior)
    let scale = (MAX_SIDE as f32 / width_px as f32).min(MAX_SIDE as f32 / height_px as f32).min(1.0);
    if scale < 1.0 {
        width_px = (width_px as f32 * scale).round() as u32;
        height_px = (height_px as f32 * scale).round() as u32;
    }

    // Check pixel budget
    if u64::from(width_px) * u64::from(height_px) > MAX_RASTER_PIXELS {
        return Err(IoError::Other(format!("PDF page ({width_px} × {height_px} px) exceeds maximum raster size of {MAX_RASTER_PIXELS} pixels")));
    }

    // Render the page
    let render_config = PdfRenderConfig::new().set_target_width(width_px as i32).set_maximum_height(height_px as i32);
    let bitmap = page.render_with_config(&render_config).map_err(|e| IoError::Other(format!("failed to render PDF page: {e}")))?;

    // Convert bitmap to image
    let image = bitmap.as_image().map_err(|e| IoError::Other(format!("failed to convert PDF bitmap to image: {e}")))?;

    // Convert to RGBA8
    let rgba_image = image.into_rgba8();

    let (w, h) = rgba_image.dimensions();
    let bounds = Rect::new(0, 0, w as i32, h as i32);

    // Create a surface from the image data
    let mut surface = Surface::new(PixelFormat::RGBA8);
    let mut data = vec![0.0f32; (w * h * 4) as usize];
    for (pixel, out) in rgba_image.pixels().zip(data.as_chunks_mut::<4>().0.iter_mut()) {
        let [r, g, b, a] = pixel.0;
        out[0] = f32::from(r) / 255.0;
        out[1] = f32::from(g) / 255.0;
        out[2] = f32::from(b) / 255.0;
        out[3] = f32::from(a) / 255.0;
    }
    surface.write_region(bounds, &data);
    surface.prune();

    // Create document
    let mut doc = Document::new(name, Size::new(w, h), ColorMode::Rgb, SampleType::U8);
    doc.layers.push(Layer::new("Background", LayerContent::Raster(surface)));

    let mut warnings = Vec::new();
    if page_count > 1 {
        warnings.push(format!("PDF: only the first page of {page_count} was imported (page {})", page_index + 1));
    }
    if scale < 1.0 {
        warnings.push(format!("PDF: the {width_pt:.0} × {height_pt:.0} pt page was scaled down to fit {MAX_SIDE} px"));
    }

    Ok(ImportResult { document: doc, warnings, source_read_only: false, preview_only: false })
}

#[cfg(test)]
mod tests {
    use super::{MAX_RASTER_PIXELS, MAX_SIDE, import_pdf};
    use crate::ImportResult;
    use crate::is_pdf_input;

    #[test]
    fn pdf_constants_match_svg() {
        // Ensure PDF limits match SVG limits for consistency
        assert_eq!(MAX_SIDE, 16_384);
        assert_eq!(MAX_RASTER_PIXELS, 24_000_000);
    }

    #[test]
    fn is_pdf_input_detects_pdf_magic_bytes() {
        // PDF files start with %PDF-
        assert!(is_pdf_input("test.pdf", b"%PDF-1.4\n%content"));
        assert!(is_pdf_input("test.PDF", b"%PDF-1.7\n%content"));
        assert!(is_pdf_input("document.pdf", b"%PDF-2.0\n%content"));
    }

    #[test]
    fn is_pdf_input_detects_by_extension() {
        // When magic bytes don't match but extension is .pdf and codecs don't detect it
        assert!(is_pdf_input("test.pdf", b"not a pdf"));
        assert!(is_pdf_input("test.PDF", b"not a pdf"));
    }

    #[test]
    fn is_pdf_input_rejects_non_pdf_files() {
        // Non-PDF files should be rejected
        // Note: magic bytes take precedence over extension
        // A file with PDF magic bytes but wrong extension is still detected as PDF
        assert!(is_pdf_input("test.txt", b"%PDF-1.4\n%content")); // Magic bytes win
        assert!(is_pdf_input("test.png", b"%PDF-1.4\n%content")); // Magic bytes win
        // A real PNG would be detected by codecs and take precedence
        assert!(!is_pdf_input("test.pdf", b"\x89PNG\r\n\x1a\n")); // PNG magic bytes, codecs detect it
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn pdf_import_with_minimal_pdf() {
        // A minimal valid PDF (single page, empty)
        // This is a minimal PDF 1.4 with one empty page
        let minimal_pdf = br#"%PDF-1.4
1 0 obj
<< /Type /Catalog /Pages 2 0 R >>
endobj
2 0 obj
<< /Type /Pages /Kids [3 0 R] /Count 1 >>
endobj
3 0 obj
<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> >>
endobj
xref
0 4
0000000000 65535 f
0000000010 00000 n
0000000060 00000 n
0000000117 00000 n
trailer
<< /Size 4 /Root 1 0 R >>
startxref
175
%%EOF"#;

        // This should either succeed or fail gracefully
        // (depending on whether pdfium is available at runtime)
        let result = import_pdf("minimal.pdf", minimal_pdf, 10);
        match result {
            Ok(ImportResult { document, .. }) => {
                // If it succeeds, verify the document has the expected properties
                assert_eq!(document.name, "minimal.pdf");
                assert!(!document.layers.is_empty());
                // Should have a Background layer
                assert!(document.layers.iter().any(|l| l.name == "Background"));
            }
            Err(e) => {
                // If it fails with pdfium library not found, skip the test (pdfium not installed)
                // Otherwise, it should be a proper PDF parse error
                let err_msg = format!("{e}");
                if err_msg.contains("pdfium library") || err_msg.contains("No Pdfium library found") {
                    eprintln!("Skipping test: pdfium library not available");
                    return;
                }
                // Any other error is unexpected for a valid PDF
                panic!("Unexpected error for valid PDF: {e}");
            }
        }
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn pdf_import_returns_error_for_invalid_pdf() {
        // Invalid PDF content
        let result = import_pdf("invalid.pdf", b"%PDF-1.4\nnot really a pdf", 10);
        match result {
            Err(e) => {
                let err_msg = format!("{e}");
                // If pdfium is not available, skip the test
                if err_msg.contains("pdfium library") || err_msg.contains("No Pdfium library found") {
                    eprintln!("Skipping test: pdfium library not available");
                    return;
                }
                // Otherwise, it should be a PDF parse error
                assert!(err_msg.contains("failed to load PDF"));
            }
            Ok(_) => panic!("Expected error for invalid PDF"),
        }
    }

    #[test]
    fn pdf_import_validates_input() {
        // Empty bytes should fail
        let result = import_pdf("empty.pdf", b"", 10);
        assert!(result.is_err());

        // Non-PDF bytes should fail
        let result = import_pdf("fake.pdf", b"not a pdf", 10);
        assert!(result.is_err());
    }
}
