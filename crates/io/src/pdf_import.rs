//! PDF import: PDFium renders the first page to a PNG and the flat importer builds the document.
//!
//! The page's print size is preserved by setting the document resolution to the import DPI:
//! pixels ÷ DPI × 72 pt is the PDF page's size again. Multi-page import, vector/text extraction
//! and page management are the next steps (see PLAN.md in the local PDF project).

use crate::{ImportOptions, ImportResult, IoError};

/// `%PDF-` magic. `.ai` files are PDFs too, but the dispatch keeps them out of this importer.
pub fn is_pdf(bytes: &[u8]) -> bool {
    bytes.starts_with(b"%PDF-")
}

/// Native PDF import is not available in the web build.
#[cfg(target_arch = "wasm32")]
pub fn import(_name: &str, _bytes: &[u8], _opts: &ImportOptions) -> Result<ImportResult, IoError> {
    Err(IoError::Unsupported("PDF import needs the desktop build".into()))
}

/// Renders the PDF's first page at [`ImportOptions::pdf_dpi`] and opens it as a flat document.
#[cfg(not(target_arch = "wasm32"))]
pub fn import(name: &str, bytes: &[u8], opts: &ImportOptions) -> Result<ImportResult, IoError> {
    use image::ImageEncoder;
    use pdfium_render::prelude::*;

    let dpi = if opts.pdf_dpi.is_finite() && opts.pdf_dpi > 0.0 { opts.pdf_dpi } else { 300.0 };
    let pdfium = bind()?;
    let document = pdfium.load_pdf_from_byte_slice(bytes, None).map_err(|e| IoError::Unsupported(format!("PDF: {e}")))?;
    let page_count = document.pages().len();
    let page = document.pages().get(0).map_err(|e| IoError::Unsupported(format!("PDF: {e}")))?;
    let width_pt = f64::from(page.width().value);
    // A page can be arbitrarily large; cap the raster so a hostile PDF cannot request a huge
    // allocation. 20,000 px is far above a poster at 300 dpi.
    let target_width = (width_pt * f64::from(dpi) / 72.0).round().clamp(1.0, 20_000.0) as i32;
    let config = PdfRenderConfig::new().set_target_width(target_width);
    let bitmap = page.render_with_config(&config).map_err(|e| IoError::Unsupported(format!("PDF render: {e}")))?;
    let (width, height) = (bitmap.width().max(0) as u32, bitmap.height().max(0) as u32);
    let rgba = bitmap.as_rgba_bytes();
    let image = image::RgbaImage::from_raw(width, height, rgba)
        .ok_or_else(|| IoError::Unsupported("PDF: the rendered bitmap does not match its buffer".into()))?;
    // Reuse the well-tested flat importer (the PNG still carries the alpha and colour exactly).
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(image.as_raw(), width, height, image::ExtendedColorType::Rgba8)
        .map_err(|e| IoError::Unsupported(format!("PDF: encoding the rendered page: {e}")))?;
    let mut result = crate::flat::import_flat(name, &png)?;
    result.document.resolution_dpi = dpi;
    result.document.name = name.to_string();
    if page_count > 1 {
        result
            .warnings
            .push(format!("PDF has {page_count} pages; this build imports page 1 (multi-page import is the next step)"));
    }
    Ok(result)
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
