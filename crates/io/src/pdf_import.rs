//! PDF import: PDFium renders every page and each page becomes an artboard.
//!
//! The document resolution is chosen so the page's physical size is preserved even when the
//! raster is capped: pixels ÷ resolution × 72 pt is the PDF page's size. Pages are laid out side
//! by side; page count, per-page pixels, union pixels and the aggregate pixel budget are bounded.

use crate::{ImportOptions, ImportResult, IoError};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Artboard, Document, Group, Layer, LayerContent};
use photocraft_geom::{Rect, Size};
use photocraft_raster::{Interrupt, Surface};

/// `%PDF-` magic. `.ai` files are PDFs too, but the dispatch keeps them out of this importer.
pub fn is_pdf(bytes: &[u8]) -> bool {
    bytes.starts_with(b"%PDF-")
}

/// Native PDF import is not available in the web build.
#[cfg(target_arch = "wasm32")]
pub fn import(_name: &str, _bytes: &[u8], _opts: &ImportOptions, _ctl: &Interrupt) -> Result<ImportResult, IoError> {
    Err(IoError::Unsupported("PDF import needs the desktop build".into()))
}

/// Renders every PDF page and opens it as one artboard per page.
#[cfg(not(target_arch = "wasm32"))]
pub fn import(name: &str, bytes: &[u8], opts: &ImportOptions, ctl: &Interrupt) -> Result<ImportResult, IoError> {
    use pdfium_render::prelude::*;

    /// The most pages one import renders.
    const MAX_PAGES: usize = 16;
    /// Gap between the artboards, in document pixels.
    const GAP: u32 = 80;
    /// Per-page and per-artboard-strip pixel budgets; a poster at 300 dpi is far below these.
    const MAX_PAGE_PIXELS: f64 = 100_000_000.0;
    const MAX_UNION_PIXELS: u64 = 200_000_000;

    let requested = if opts.pdf_dpi.is_finite() && opts.pdf_dpi > 0.0 { f64::from(opts.pdf_dpi) } else { 300.0 };
    let pdfium = bind()?;
    let document = pdfium.load_pdf_from_byte_slice(bytes, None).map_err(|e| IoError::Unsupported(format!("PDF: {e}")))?;
    let total = document.pages().len().max(0) as usize;
    let count = total.min(MAX_PAGES);
    if count == 0 {
        return Err(IoError::Unsupported("PDF has no pages".into()));
    }
    // The page sizes in points first: the effective DPI must fit the budgets, and capping the
    // raster must not change the page's physical size.
    let mut sizes = Vec::with_capacity(count);
    for index in 0..count {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let page = document.pages().get(index as i32).map_err(|e| IoError::Unsupported(format!("PDF: {e}")))?;
        let (w, h) = (f64::from(page.width().value), f64::from(page.height().value));
        if w <= 0.0 || h <= 0.0 {
            return Err(IoError::Unsupported(format!("PDF page {} has no area", index + 1)));
        }
        sizes.push((w, h));
    }
    let dpi = effective_dpi(requested, &sizes, GAP, MAX_PAGE_PIXELS, MAX_UNION_PIXELS);
    let doc_width = sizes.iter().map(|(w, _)| (w * dpi / 72.0).round() as u64).sum::<u64>() + u64::from(GAP) * (count as u64 - 1);
    let doc_height = sizes.iter().map(|(_, h)| (h * dpi / 72.0).round() as u64).max().unwrap_or(1);
    if doc_width == 0 || doc_height == 0 {
        return Err(IoError::Unsupported("PDF pages have no area".into()));
    }
    // The aggregate budget: the rendered pages and the document's raster tiles both hold pixels.
    let page_pixels = sizes.iter().map(|(w, h)| (w * dpi / 72.0).round().max(1.0) * (h * dpi / 72.0).round().max(1.0)).sum::<f64>();
    // The RGBA bitmap, the f32 conversion buffer (16 bytes/px) and the document tiles.
    let estimate = (page_pixels * 24.0) as u64;
    if opts.flat_max_pixel_bytes > 0 && estimate > opts.flat_max_pixel_bytes {
        return Err(IoError::Unsupported(format!(
            "PDF import needs about {} MB of pixel memory; the limit is {} MB (lower the import DPI)",
            estimate >> 20,
            opts.flat_max_pixel_bytes >> 20
        )));
    }
    let mut doc = Document::new(name, Size::new(doc_width as u32, doc_height as u32), ColorMode::Rgb, SampleType::U8);
    doc.resolution_dpi = dpi as f32;
    let mut x = 0u32;
    for (index, (w_pt, h_pt)) in sizes.iter().enumerate() {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let target_width = (w_pt * dpi / 72.0).round().max(1.0) as i32;
        let target_height = (h_pt * dpi / 72.0).round().max(1.0) as i32;
        // A one-pixel-wide page would otherwise produce an arbitrarily tall bitmap before the
        // union check runs; reject the final dimensions up front.
        if f64::from(target_width) * f64::from(target_height) > MAX_PAGE_PIXELS {
            return Err(IoError::Unsupported(format!(
                "PDF page {} is too large at {dpi} dpi ({} x {} px); lower the import DPI",
                index + 1,
                target_width,
                target_height
            )));
        }
        let page = document.pages().get(index as i32).map_err(|e| IoError::Unsupported(format!("PDF: {e}")))?;
        let bitmap =
            page.render_with_config(&PdfRenderConfig::new().set_target_width(target_width)).map_err(|e| IoError::Unsupported(format!("PDF render: {e}")))?;
        let (width, height) = (bitmap.width().max(0) as u32, bitmap.height().max(0) as u32);
        // The rounded target width can render a taller bitmap than the pre-check predicted.
        if f64::from(width) * f64::from(height) > MAX_PAGE_PIXELS {
            return Err(IoError::Unsupported(format!(
                "PDF page {} rendered larger than the {MAX_PAGE_PIXELS} px budget ({} x {}); lower the import DPI",
                index + 1,
                width,
                height
            )));
        }
        // The page's pixels go straight into the document-wide layer surface: no full-strip
        // canvas and no PNG round-trip, so the peak is the page bitmap plus its tiles.
        let mut surface = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
        let rgba = bitmap.as_rgba_bytes();
        let mut data = Vec::with_capacity(rgba.len());
        for chunk in rgba.as_chunks::<4>().0 {
            for &v in &chunk[..3] {
                data.push(f32::from(v) / 255.0);
            }
            data.push(f32::from(chunk[3]) / 255.0);
        }
        surface.write_region(Rect::from_xywh(x as i32, 0, width, height), &data);
        let layer = Layer::new(format!("Page {}", index + 1), LayerContent::Raster(surface));
        let rect = Rect::new(x as i32, 0, (x + width) as i32, height as i32);
        let group = Group { children: vec![layer], expanded: true, artboard: Some(Artboard::new(rect)), frame_stroke: None };
        doc.layers.push(Layer::new(format!("Page {}", index + 1), LayerContent::Group(group)));
        x += width + GAP;
        ctl.progress((index + 1) as f32 / count as f32);
    }
    let mut warnings = Vec::new();
    if total > count {
        let reason = if count < total.min(MAX_PAGES) { "the rest would make the import canvas too large" } else { "the page limit is 16" };
        warnings.push(format!("PDF has {total} pages; imported the first {count} ({reason})"));
    }
    Ok(ImportResult { warnings, document: doc, source_read_only: false, preview_only: false })
}

/// The highest DPI not above `requested` that keeps every page and the artboard strip within
/// their pixel budgets. Rendering at that DPI keeps each page's physical size unchanged.
#[cfg(not(target_arch = "wasm32"))]
fn effective_dpi(requested: f64, sizes: &[(f64, f64)], gap: u32, max_page: f64, max_union: u64) -> f64 {
    let mut dpi = requested;
    for (w, h) in sizes {
        dpi = dpi.min(72.0 * (max_page / (w * h)).sqrt());
    }
    let n = sizes.len() as f64;
    let sum_w: f64 = sizes.iter().map(|(w, _)| w).sum();
    let max_h = sizes.iter().map(|(_, h)| h).fold(0.0_f64, |a, &h| a.max(h));
    // (sum_w * k + gaps) * (max_h * k) <= max_union, where k = dpi / 72: solve a k² + b k - c = 0.
    let gaps = f64::from(gap) * (n - 1.0);
    let a = sum_w * max_h;
    let b = gaps * max_h;
    let k = if a > 0.0 { (-b + (b * b + 4.0 * a * max_union as f64).sqrt()) / (2.0 * a) } else { f64::INFINITY };
    dpi.min(72.0 * k).max(1.0)
}

/// Loads the PDFium shared library from the executable's folder (`PDFIUM_DIR` overrides it).
#[cfg(not(target_arch = "wasm32"))]
fn bind() -> Result<pdfium_render::prelude::Pdfium, IoError> {
    use pdfium_render::prelude::Pdfium;

    let dir = std::env::var_os("PDFIUM_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_exe().ok().and_then(|exe| exe.parent().map(std::path::Path::to_path_buf)).unwrap_or_default());
    let library = Pdfium::pdfium_platform_library_name_at_path(&dir);
    let platform = Pdfium::pdfium_platform_library_name().to_string_lossy().into_owned();
    let bindings = Pdfium::bind_to_library(&library).map_err(|e| {
        IoError::Unsupported(format!(
            "PDF import: cannot load PDFium from {} ({e}); put {platform} next to the executable or set PDFIUM_DIR",
            library.display()
        ))
    })?;
    Ok(Pdfium::new(bindings))
}
