//! PDF pages become raster artboards at the selected resolution (default 144 ppi). PDF export writes lossless, colour-managed
//! raster pages, in Layers-panel order, retaining physical size and transparency.
use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use flate2::{Compression, write::ZlibEncoder};
use hayro::{RenderCache, RenderSettings, hayro_interpret::InterpreterSettings, hayro_syntax::Pdf};
use photocraft_cms::{Builtin, ColorSpace, Intent, Profile, Transform};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Artboard, Document, Group, Layer, LayerContent};
use photocraft_geom::{Rect, Size};
use photocraft_raster::{Interrupt, Surface};

use crate::{ExportOptions, ExportResult, ImportResult, IoError, XmpEmbed};

const DPI: f32 = 144.0;
const MAX_PAGES: usize = 100;
const MAX_SIDE: u32 = 16384;
const MAX_PIXELS: u64 = 64_000_000;
const MAX_TOTAL_PIXELS: u64 = 512_000_000;

#[path = "pdf_guard.rs"]
mod guard;

/// Raster resolution selected in the PDF open dialog (pixels per inch).
#[derive(Clone, Copy, Debug)]
pub struct ImportOptions {
    pub resolution: f32,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self { resolution: DPI }
    }
}

impl ImportOptions {
    pub fn validate(self) -> Result<(), IoError> {
        if !self.resolution.is_finite() || !(1.0..=2400.0).contains(&self.resolution) {
            return Err(error("PDF resolution must be between 1 and 2400 ppi"));
        }
        Ok(())
    }
}

// Hayro can panic or abort on malformed inputs. WebAssembly cannot unwind, and
// its shared editor instance has no isolated renderer with a hard memory budget.
// Reject before parsing, including the picker/preview paths; export remains available.
fn require_native_import() -> Result<(), IoError> {
    if cfg!(target_arch = "wasm32") {
        return Err(error(
            "PDF import is available in the desktop app. This web editor cannot safely isolate the PDF renderer; existing documents are unchanged.",
        ));
    }
    Ok(())
}

fn error(message: impl ToString) -> IoError {
    IoError::Pdf(message.to_string())
}

fn check_size(w: u32, h: u32) -> Result<u64, IoError> {
    let n = u64::from(w) * u64::from(h);
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE || n > MAX_PIXELS {
        return Err(error("page exceeds the 16,384-pixel side or 64-megapixel limit; reduce its size or resolution in a PDF editor"));
    }
    Ok(n)
}

/// Open every page, with a bounded raster allocation and no source overwrite path.
pub fn import_pdf(name: &str, bytes: &[u8], ctl: &Interrupt) -> Result<ImportResult, IoError> {
    import_pdf_pages(name, bytes, None, ctl)
}

/// Import only the requested zero-based pages, in binder order. None imports every page.
pub fn import_pdf_pages(name: &str, bytes: &[u8], selected: Option<&[usize]>, ctl: &Interrupt) -> Result<ImportResult, IoError> {
    import_pdf_pages_with(name, bytes, selected, ImportOptions::default(), ctl)
}

pub fn import_pdf_pages_with(name: &str, bytes: &[u8], selected: Option<&[usize]>, options: ImportOptions, ctl: &Interrupt) -> Result<ImportResult, IoError> {
    require_native_import()?;
    options.validate()?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    if bytes.len() > 256 * 1024 * 1024 {
        return Err(error("PDF exceeds the 256 MB import limit"));
    }
    catch_unwind(AssertUnwindSafe(|| import_inner(name, bytes, selected, options, ctl)))
        .map_err(|_| error("the PDF renderer could not process this document"))?
}

/// Read page sizes without rasterizing any page.
pub fn page_sizes(bytes: &[u8]) -> Result<Vec<(f32, f32)>, IoError> {
    require_native_import()?;
    catch_unwind(AssertUnwindSafe(|| page_sizes_inner(bytes))).map_err(|_| error("the PDF parser could not inspect this document"))?
}

fn page_sizes_inner(bytes: &[u8]) -> Result<Vec<(f32, f32)>, IoError> {
    if bytes.len() > 256 * 1024 * 1024 {
        return Err(error("PDF exceeds the 256 MB import limit"));
    }
    let pdf = Pdf::new(Arc::new(bytes.to_vec())).map_err(|e| error(format!("cannot open PDF ({e:?})")))?;
    if pdf.pages().is_empty() || pdf.pages().len() > MAX_PAGES {
        return Err(error("PDF must contain between 1 and 100 pages"));
    }
    Ok(pdf.pages().iter().map(|page| page.render_dimensions()).collect())
}

/// Small page preview for the import picker; it does not create an editing document.
pub fn page_preview(bytes: Arc<Vec<u8>>, index: usize) -> Result<(u32, u32, Vec<u8>), IoError> {
    require_native_import()?;
    if bytes.len() > 256 * 1024 * 1024 {
        return Err(error("PDF exceeds the 256 MB import limit"));
    }
    catch_unwind(AssertUnwindSafe(|| {
        let pdf = Pdf::new(bytes).map_err(|e| error(format!("cannot preview PDF ({e:?})")))?;
        if pdf.pages().is_empty() || pdf.pages().len() > MAX_PAGES {
            return Err(error("PDF must contain between 1 and 100 pages"));
        }
        guard::preflight(&pdf, &Interrupt::default())?;
        let page = pdf.pages().get(index).ok_or_else(|| error("page number is out of range"))?;
        let (w, h) = page.render_dimensions();
        if !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0 {
            return Err(error("invalid PDF page dimensions"));
        }
        let scale = (420.0 / w).min(420.0 / h);
        let (width, height) = ((w * scale).ceil() as u16, (h * scale).ceil() as u16);
        let settings =
            RenderSettings { x_scale: scale, y_scale: scale, width: Some(width), height: Some(height), bg_color: hayro::vello_cpu::color::palette::css::WHITE };
        let pixels = hayro::render(page, &RenderCache::new(), &InterpreterSettings::default(), &settings);
        Ok((u32::from(width), u32::from(height), pixels.data_as_u8_slice().to_vec()))
    }))
    .map_err(|_| error("the PDF renderer could not preview this page"))?
}

fn import_inner(name: &str, bytes: &[u8], selected: Option<&[usize]>, options: ImportOptions, ctl: &Interrupt) -> Result<ImportResult, IoError> {
    let pdf = Pdf::new(Arc::new(bytes.to_vec()))
        .map_err(|e| error(format!("cannot open PDF ({e:?}); password-protected files must first be unlocked in a PDF editor")))?;
    let pages = pdf.pages();
    if pages.is_empty() || pages.len() > MAX_PAGES {
        return Err(error("PDF must contain between 1 and 100 pages"));
    }
    guard::preflight(&pdf, ctl)?;
    let dpi = options.resolution;
    let indices: Vec<usize> = match selected {
        Some(s) if s.is_empty() || s.iter().any(|&i| i >= pages.len()) => return Err(error("select at least one valid PDF page")),
        Some(s) => (0..pages.len()).filter(|i| s.contains(i)).collect(),
        None => (0..pages.len()).collect(),
    };
    let mut sizes = Vec::new();
    let mut total = 0u64;
    let mut canvas_height = 0u32;
    let mut canvas_width = 0u32;
    for &index in &indices {
        let page = &pages[index];
        let (w, h) = page.render_dimensions();
        if !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0 {
            return Err(error("invalid PDF page dimensions"));
        }
        let (w, h) = ((w * dpi / 72.0).ceil() as u32, (h * dpi / 72.0).ceil() as u32);
        total += check_size(w, h)?;
        if total > MAX_TOTAL_PIXELS {
            return Err(error("PDF exceeds 512 megapixels across its pages; split it into smaller PDFs first"));
        }
        canvas_height = canvas_height.checked_add(h + 32).ok_or_else(|| error("PDF canvas is too tall"))?;
        if canvas_height > 300_000 {
            return Err(error("PDF canvas exceeds 300,000 pixels; split it into smaller PDFs first"));
        }
        canvas_width = canvas_width.max(w);
        sizes.push((w, h));
    }
    let mut doc = Document::new(name, Size::new(canvas_width, canvas_height - 32), ColorMode::Rgb, SampleType::U8);
    doc.resolution_dpi = dpi;
    doc.icc_profile = Some(Builtin::Srgb.profile().to_bytes());
    let cache = RenderCache::new();
    let mut y = 0i32;
    for (done, (&index, &(w, h))) in indices.iter().zip(&sizes).enumerate() {
        let page = &pages[index];
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let settings = RenderSettings {
            x_scale: dpi / 72.0,
            y_scale: dpi / 72.0,
            width: Some(w as u16),
            height: Some(h as u16),
            bg_color: hayro::vello_cpu::color::palette::css::WHITE,
        };
        let pixels = hayro::render(page, &cache, &InterpreterSettings::default(), &settings);
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let rect = Rect::new(0, y, w as i32, y + h as i32);
        let mut surface = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
        surface.write_interleaved(rect, pixels.data_as_u8_slice());
        surface.prune();
        let layer = Layer::new("Page image", LayerContent::Raster(surface));
        doc.layers.push(Layer::new(
            format!("Page {}", index + 1),
            LayerContent::Group(Group { children: vec![layer], expanded: true, artboard: Some(Artboard::new(rect)) }),
        ));
        y += h as i32 + 32;
        ctl.progress((done + 1) as f32 / indices.len() as f32);
    }
    // Layer storage is bottom-to-top; PDF order follows the top-to-bottom Layers panel.
    doc.layers.reverse();
    Ok(ImportResult {
        document: doc,
        warnings: vec![format!(
            "Imported {} PDF page(s) as raster artboards at {dpi} ppi. PDF text, vectors, forms and links are not editable objects. Save As PDF exports each artboard as a page; save .pcraft to retain editing layers. The original PDF is not overwritten automatically.",
            indices.len()
        )],
        source_read_only: true,
        preview_only: false,
    })
}

/// Export a normal canvas as one page, or each artboard as a separate page.
pub fn export_pdf(doc: &Document, opts: &ExportOptions) -> Result<ExportResult, IoError> {
    export_pdf_documents(&[doc], opts)
}

/// Export documents in tab order, retaining each page's own size, resolution and colour profile.
pub fn export_pdf_documents(docs: &[&Document], opts: &ExportOptions) -> Result<ExportResult, IoError> {
    let mut pages = Vec::new();
    let mut total = 0;
    for doc in docs {
        if !doc.resolution_dpi.is_finite() || doc.resolution_dpi <= 0.0 {
            return Err(error("document resolution must be finite and positive"));
        }
        let boards = doc.artboards();
        if boards.is_empty() {
            pages.push((*doc, None, doc.bounds()));
        } else {
            pages.extend(boards.iter().rev().map(|(id, _, board)| (*doc, Some(*id), board.rect)));
        }
        if pages.len() > MAX_PAGES {
            return Err(error("PDF must contain between 1 and 100 pages"));
        }
    }
    if pages.is_empty() {
        return Err(error("PDF must contain between 1 and 100 pages"));
    }
    for (_, _, rect) in &pages {
        total += check_size(rect.width(), rect.height())?;
        if total > MAX_TOTAL_PIXELS {
            return Err(error("PDF export exceeds 512 megapixels across its pages"));
        }
    }
    let mut output_pages = Vec::new();
    for (doc, id, rect) in &pages {
        let gray = doc.mode == ColorMode::Grayscale;
        let source = if matches!(doc.mode, ColorMode::Rgb | ColorMode::Grayscale) {
            doc.icc_profile.as_deref().and_then(|p| Profile::parse(p).ok()).filter(|p| p.color_space == if gray { ColorSpace::Gray } else { ColorSpace::Rgb })
        } else {
            None
        };
        let transform = source.as_ref().map(|p| Transform::new(p, Builtin::Srgb.profile(), Intent::RelativeColorimetric, true)).transpose().map_err(error)?;
        let mut one = (*doc).clone();
        if let Some(id) = id {
            one.layers.retain(|l| l.id == *id);
            for layer in &mut one.layers {
                layer.visible = true;
            }
        }
        let mut rgb = ZlibEncoder::new(Vec::new(), Compression::default());
        let mut alpha = ZlibEncoder::new(Vec::new(), Compression::default());
        photocraft_compose::render_bands(&one, *rect, 64, |band| -> Result<(), IoError> {
            let stride = if gray { 1 } else { 3 };
            let mut colors = Vec::with_capacity(band.px.len() * 3);
            if let Some(t) = &transform {
                let source: Vec<f32> = band.px.iter().flat_map(|p| p.iter().take(stride).copied()).collect();
                colors.resize(band.px.len() * 3, 0.0);
                t.convert_f32(&source, stride, &mut colors, 3, false);
            } else {
                colors.extend(band.px.iter().flat_map(|p| p.iter().take(3).copied()));
            }
            let bytes: Vec<u8> = colors.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect();
            rgb.write_all(&bytes).map_err(error)?;
            let bytes: Vec<u8> = band.px.iter().map(|p| (p[3].clamp(0.0, 1.0) * 255.0).round() as u8).collect();
            alpha.write_all(&bytes).map_err(error)?;
            Ok(())
        })?;
        let rgb = rgb.finish().map_err(error)?;
        let alpha = alpha.finish().map_err(error)?;
        let (w, h) = (rect.width(), rect.height());
        let (pw, ph) = (f64::from(w) * 72.0 / f64::from(doc.resolution_dpi), f64::from(h) * 72.0 / f64::from(doc.resolution_dpi));
        if !pw.is_finite() || !ph.is_finite() || pw > 14400.0 || ph > 14400.0 {
            return Err(error("PDF page exceeds 200 inches; increase the document resolution"));
        }
        output_pages.push(crate::pdf_writer::RasterPage {
            paper: (pw, ph),
            content: format!("q {pw:.6} 0 0 {ph:.6} 0 0 cm /Im0 Do Q"),
            image: crate::pdf_writer::RasterImage {
                width: w,
                height: h,
                channels: 3,
                data: rgb,
                jpeg: false,
                icc: Some(Builtin::Srgb.profile().to_bytes().to_vec()),
                alpha: Some(alpha),
            },
        });
    }
    let metadata = if opts.xmp == XmpEmbed::All && docs.len() == 1 { docs.first().and_then(|d| d.metadata.xmp.as_ref()) } else { None };
    Ok(ExportResult {
        bytes: crate::pdf_writer::write_pdf(&output_pages, metadata.map(String::as_str)),
        warnings: vec![format!(
            "Exported {} raster PDF page(s) in 8-bit sRGB with lossless compression. Editing layers and PDF text/vectors are not retained; use .pcraft to keep editing layers.",
            pages.len()
        )],
    })
}
