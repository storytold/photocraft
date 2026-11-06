//! PDF import: PDFium renders every page and each page becomes an artboard.
//!
//! The document resolution is the import DPI, so pixels ÷ DPI × 72 pt is the PDF page's size
//! again. Pages are laid out side by side with a small gap; more than [`MAX_PAGES`] pages are
//! capped so the union canvas stays bounded.

use crate::{ImportOptions, ImportResult, IoError};
use photocraft_doc::{Artboard, Document, Group, Layer, LayerContent};
use photocraft_geom::Rect;

/// `%PDF-` magic. `.ai` files are PDFs too, but the dispatch keeps them out of this importer.
pub fn is_pdf(bytes: &[u8]) -> bool {
    bytes.starts_with(b"%PDF-")
}

/// Native PDF import is not available in the web build.
#[cfg(target_arch = "wasm32")]
pub fn import(_name: &str, _bytes: &[u8], _opts: &ImportOptions) -> Result<ImportResult, IoError> {
    Err(IoError::Unsupported("PDF import needs the desktop build".into()))
}

/// Renders every PDF page at [`ImportOptions::pdf_dpi`] and opens it as one artboard per page.
#[cfg(not(target_arch = "wasm32"))]
pub fn import(name: &str, bytes: &[u8], opts: &ImportOptions) -> Result<ImportResult, IoError> {
    use image::ImageEncoder;
    use pdfium_render::prelude::*;

    /// The most pages one import renders; more would make the union canvas enormous.
    const MAX_PAGES: usize = 16;
    /// Gap between the artboards, in document pixels.
    const GAP: u32 = 80;

    let dpi = if opts.pdf_dpi.is_finite() && opts.pdf_dpi > 0.0 { opts.pdf_dpi } else { 300.0 };
    let pdfium = bind()?;
    let document = pdfium.load_pdf_from_byte_slice(bytes, None).map_err(|e| IoError::Unsupported(format!("PDF: {e}")))?;
    let total = document.pages().len().max(0) as usize;
    let count = total.min(MAX_PAGES);
    if count == 0 {
        return Err(IoError::Unsupported("PDF has no pages".into()));
    }
    // Render first, so a malformed later page fails before any document is built.
    let mut rendered = Vec::with_capacity(count);
    for index in 0..count {
        let page = document.pages().get(index as i32).map_err(|e| IoError::Unsupported(format!("PDF: {e}")))?;
        let width_pt = f64::from(page.width().value);
        // A page can be arbitrarily large; cap the raster so a hostile PDF cannot request a huge
        // allocation. 20,000 px is far above a poster at 300 dpi.
        let target_width = (width_pt * f64::from(dpi) / 72.0).round().clamp(1.0, 20_000.0) as i32;
        let config = PdfRenderConfig::new().set_target_width(target_width);
        let bitmap = page.render_with_config(&config).map_err(|e| IoError::Unsupported(format!("PDF render: {e}")))?;
        let (width, height) = (bitmap.width().max(0) as u32, bitmap.height().max(0) as u32);
        rendered.push((width, height, bitmap.as_rgba_bytes()));
    }
    let doc_width = rendered.iter().map(|(w, _, _)| *w).sum::<u32>() + GAP * (count as u32 - 1);
    let doc_height = rendered.iter().map(|(_, h, _)| *h).max().unwrap_or(1);
    let mut doc: Option<Document> = None;
    let mut x = 0u32;
    for (index, (width, height, rgba)) in rendered.into_iter().enumerate() {
        // Reuse the well-tested flat importer: the page is composited into the union canvas at its
        // artboard position, encoded as PNG, and decoded into a document-wide raster layer.
        let mut canvas = image::RgbaImage::new(doc_width, doc_height);
        let page = image::RgbaImage::from_raw(width, height, rgba)
            .ok_or_else(|| IoError::Unsupported("PDF: the rendered bitmap does not match its buffer".into()))?;
        image::imageops::overlay(&mut canvas, &page, i64::from(x), 0);
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(canvas.as_raw(), doc_width, doc_height, image::ExtendedColorType::Rgba8)
            .map_err(|e| IoError::Unsupported(format!("PDF: encoding the rendered page: {e}")))?;
        let mut result = crate::flat::import_flat(name, &png)?;
        let layer = result.document.layers.pop().ok_or_else(|| IoError::Unsupported("PDF: the page decoded to an empty layer".into()))?;
        let rect = Rect::new(x as i32, 0, (x + width) as i32, height as i32);
        let group = Group { children: vec![layer], expanded: true, artboard: Some(Artboard::new(rect)), frame_stroke: None };
        let artboard = Layer::new(format!("Page {}", index + 1), LayerContent::Group(group));
        match &mut doc {
            None => {
                result.document.layers = vec![artboard];
                doc = Some(result.document);
            }
            Some(d) => d.layers.push(artboard),
        }
        x += width + GAP;
    }
    let mut doc = doc.ok_or_else(|| IoError::Unsupported("PDF has no pages".into()))?;
    doc.resolution_dpi = dpi;
    doc.name = name.to_string();
    let mut warnings = Vec::new();
    if total > count {
        warnings.push(format!("PDF has {total} pages; imported the first {count} (the limit is {MAX_PAGES})"));
    }
    Ok(ImportResult { warnings, document: doc, source_read_only: false, preview_only: false })
}

/// Loads the PDFium shared library from the executable's folder (`PDFIUM_DIR` overrides it).
#[cfg(not(target_arch = "wasm32"))]
fn bind() -> Result<pdfium_render::prelude::Pdfium, IoError> {
    use pdfium_render::prelude::Pdfium;

    let dir = std::env::var_os("PDFIUM_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf))
            .unwrap_or_default()
    });
    let library = Pdfium::pdfium_platform_library_name_at_path(&dir);
    let bindings = Pdfium::bind_to_library(&library).map_err(|e| {
        IoError::Unsupported(format!(
            "PDF import: cannot load PDFium from {} ({e}); put pdfium.dll next to the executable or set PDFIUM_DIR",
            library.display()
        ))
    })?;
    Ok(Pdfium::new(bindings))
}
