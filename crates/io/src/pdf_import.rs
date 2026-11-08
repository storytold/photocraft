//! PDF page rasterisation for File › Open and Place.
//!
//! PDFs are imported as a flattened raster of their first page. Text, vector objects, and later
//! pages are not represented as editable document content.

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings, render};
use photocraft_raster::Interrupt;

use crate::{ImportResult, IoError};

const MAX_SIDE: u32 = 65_535;
const MAX_PIXELS: u64 = 64 * 1024 * 1024;

fn check_dimensions(width: f32, height: f32) -> Result<(), IoError> {
    let pixels = f64::from(width) * f64::from(height);
    if !width.is_finite()
        || !height.is_finite()
        || width < 1.0
        || height < 1.0
        || width > MAX_SIDE as f32
        || height > MAX_SIDE as f32
        || pixels > MAX_PIXELS as f64
    {
        return Err(IoError::Pdf(format!(
            "first page is {width} × {height} points; raster imports are limited to {MAX_SIDE} px per side and {MAX_PIXELS} pixels"
        )));
    }
    Ok(())
}

pub(crate) fn import(name: &str, bytes: &[u8], ctl: &Interrupt) -> Result<ImportResult, IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let pdf = Pdf::new(bytes.to_vec()).map_err(|e| IoError::Pdf(format!("could not read document ({e:?})")))?;
    let pages = pdf.pages();
    let Some(page) = pages.first() else {
        return Err(IoError::Pdf("the document has no pages".into()));
    };
    let (width, height) = page.render_dimensions();
    check_dimensions(width, height)?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let cache = RenderCache::new();
    let pixmap =
        render(page, &cache, &InterpreterSettings::default(), &RenderSettings::default(), &PixmapSettings { x_scale: 1.0, y_scale: 1.0, bg_color: WHITE });
    let png = pixmap.into_png().map_err(|e| IoError::Pdf(format!("could not encode the rendered page ({e})")))?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let mut result = crate::flat::import_flat(name, &png)?;
    if pages.len() > 1 {
        result.warnings.push(format!("PDF has {} pages; only the first page was opened", pages.len()));
    }
    result.warnings.push("PDF content was rasterised; vector objects, text and layers are not editable".into());
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_pdf() -> Vec<u8> {
        let page = crate::pdf::PrintPage {
            paper: (8.0, 6.0),
            image: crate::pdf::PrintImage { width: 2, height: 2, channels: 3, data: (0..4).flat_map(|_| [255, 0, 0]).collect(), icc: None },
            rect: (0.0, 0.0, 8.0, 6.0),
            marks: Default::default(),
            description: None,
            label: None,
        };
        crate::pdf::print_pdf(&page)
    }

    #[test]
    fn imports_a_pdf_by_signature_and_rasterises_its_first_page() {
        let bytes = sample_pdf();
        let result = crate::import("picture.bin", &bytes).unwrap();
        assert_eq!(result.document.size.width, 8);
        assert_eq!(result.document.size.height, 6);
        assert!(result.warnings.iter().any(|w| w.contains("rasterised")));
    }

    #[test]
    fn malformed_pdf_gets_a_pdf_error_instead_of_an_image_codec_error() {
        let error = crate::import("broken.pdf", b"%PDF-not a document").unwrap_err();
        assert!(matches!(error, IoError::Pdf(_)));
    }

    #[test]
    fn rejects_invalid_or_oversized_page_dimensions_before_allocating() {
        assert!(check_dimensions(f32::NAN, 10.0).is_err());
        assert!(check_dimensions(10.0, f32::INFINITY).is_err());
        assert!(check_dimensions(0.0, 10.0).is_err());
        assert!(check_dimensions(65_536.0, 10.0).is_err());
        assert!(check_dimensions(10_000.0, 10_000.0).is_err());
        assert!(check_dimensions(100.0, 100.0).is_ok());
    }
}
