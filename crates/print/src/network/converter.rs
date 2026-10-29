//! PhotoCraft's [`DocumentConverter`]: the print page the engine laid out, as PWG raster, for a
//! printer that lists raster but not PDF.
//!
//! The registry decides *whether* to convert (the printer does not list the document's format) and
//! *at what resolution* (the job's own, else the printer's listed one nearest 300 dpi); this is the
//! *how*. PhotoCraft has no PDF renderer, and does not need one: the document sent is the print-ready
//! PDF the engine wrote from the page it laid out, so the converter asks the engine to render that
//! same page again at the resolution chosen ([`RenderFn`]) and packs it with craft-print's neutral
//! [`pwg`] encoder. It checks that the PDF it is given is the one it was made for.

use std::sync::Arc;

use craft_print::{ConvertContext, Document, DocumentConverter, DocumentFormat, Duplex, ErrorKind, OwnedDocument, PrintError, pwg};

/// What to render: the registry's resolved choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderSpec {
    /// Dots per inch, horizontally and vertically.
    pub dpi: (u32, u32),
    /// Colour (RGB) or grey.
    pub colour: bool,
}

/// Renders the print pages (one for PhotoCraft) at `spec`. An error is the reason, in words.
pub type RenderFn = dyn Fn(&RenderSpec) -> Result<Vec<pwg::RasterPage>, String> + Send + Sync;

/// PDF (the one this converter was made for) to PWG raster.
#[derive(Clone)]
pub struct RenderedPages {
    pdf: Arc<Vec<u8>>,
    render: Arc<RenderFn>,
}

impl RenderedPages {
    /// A converter for `pdf`, whose pages `render` makes at the resolution asked.
    pub fn new(pdf: Arc<Vec<u8>>, render: Arc<RenderFn>) -> Self {
        Self { pdf, render }
    }
}

fn failed(why: impl Into<String>) -> PrintError {
    PrintError::new(ErrorKind::Failed, why)
}

impl DocumentConverter for RenderedPages {
    fn targets(&self, from: DocumentFormat) -> Vec<DocumentFormat> {
        if from == DocumentFormat::Pdf { vec![DocumentFormat::PwgRaster] } else { Vec::new() }
    }

    fn convert(&self, document: Document<'_>, target: DocumentFormat, context: &ConvertContext) -> Result<OwnedDocument, PrintError> {
        if document.format != DocumentFormat::Pdf || target != DocumentFormat::PwgRaster {
            return Err(PrintError::new(
                ErrorKind::UnsupportedFormat,
                format!("this converter makes PWG raster from PhotoCraft's print PDF, not {} from {}", target.mime(), document.format.mime()),
            ));
        }
        if document.bytes != self.pdf.as_slice() {
            return Err(failed("this converter was made for another print page"));
        }
        if context.cancel.is_cancelled() {
            return Err(PrintError::new(ErrorKind::Cancelled, "the print job was cancelled before the page was rendered"));
        }
        let (dx, dy) = (context.resolution.x_dpi.max(1), context.resolution.y_dpi.max(1));
        let pages = (self.render)(&RenderSpec { dpi: (dx, dy), colour: context.colour }).map_err(failed)?;
        if pages.is_empty() {
            return Err(failed("the print has no page to render"));
        }
        if context.cancel.is_cancelled() {
            return Err(PrintError::new(ErrorKind::Cancelled, "the print job was cancelled while the page was rendered"));
        }
        let duplex = match context.duplex {
            None | Some(Duplex::Off) => pwg::Duplex::Off,
            Some(Duplex::LongEdge) => pwg::Duplex::LongEdge,
            Some(Duplex::ShortEdge) => pwg::Duplex::ShortEdge,
        };
        let raster = pwg::encode(&pages, &pwg::PwgOptions { dpi: (dx, dy), duplex }).map_err(|e| failed(e.to_string()))?;
        Ok(OwnedDocument::new(DocumentFormat::PwgRaster, raster))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_print::Resolution;

    fn pages(spec: &RenderSpec) -> Result<Vec<pwg::RasterPage>, String> {
        let (w, h) = (4u32, 2u32);
        let bpp = if spec.colour { 3 } else { 1 };
        Ok(vec![pwg::RasterPage { width: w, height: h, gray: !spec.colour, pixels: vec![200; (w * h) as usize * bpp] }])
    }

    #[test]
    fn makes_pwg_raster_from_its_own_pdf_only() {
        let pdf = Arc::new(b"%PDF-1.4 test".to_vec());
        let converter = RenderedPages::new(Arc::clone(&pdf), Arc::new(pages));
        assert_eq!(converter.targets(DocumentFormat::Pdf), vec![DocumentFormat::PwgRaster]);
        assert!(converter.targets(DocumentFormat::Png).is_empty());
        let context = ConvertContext::new(Resolution::new(300, 300), true);
        let out = converter.convert(Document::pdf(&pdf), DocumentFormat::PwgRaster, &context).unwrap();
        assert_eq!(out.format, DocumentFormat::PwgRaster);
        assert!(out.bytes.starts_with(b"RaS2"), "a PWG raster stream");
        let other = converter.convert(Document::pdf(b"%PDF-1.4 other"), DocumentFormat::PwgRaster, &context).unwrap_err();
        assert_eq!(other.kind(), ErrorKind::Failed);
        let wrong = converter.convert(Document::new(DocumentFormat::Png, &pdf), DocumentFormat::PwgRaster, &context).unwrap_err();
        assert_eq!(wrong.kind(), ErrorKind::UnsupportedFormat);
    }

    #[test]
    fn a_render_failure_is_a_failure_before_anything_is_sent() {
        let pdf = Arc::new(b"%PDF".to_vec());
        let converter = RenderedPages::new(Arc::clone(&pdf), Arc::new(|_: &RenderSpec| Err("needs CMYK".to_owned())));
        let context = ConvertContext::new(Resolution::new(300, 300), false);
        let e = converter.convert(Document::pdf(&pdf), DocumentFormat::PwgRaster, &context).unwrap_err();
        assert!(e.message().contains("needs CMYK"));
    }

    #[test]
    fn cancelling_stops_before_rendering() {
        let pdf = Arc::new(b"%PDF".to_vec());
        let converter = RenderedPages::new(Arc::clone(&pdf), Arc::new(pages));
        let context = ConvertContext::new(Resolution::new(300, 300), false);
        context.cancel.cancel();
        let e = converter.convert(Document::pdf(&pdf), DocumentFormat::PwgRaster, &context).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::Cancelled);
    }
}
