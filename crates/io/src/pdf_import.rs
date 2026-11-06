//! PDF import: PDFium renders the selected pages and each page becomes an artboard.
//!
//! The document resolution is chosen so the page's physical size is preserved even when the
//! raster is capped: pixels ÷ resolution × 72 pt is the PDF page's size. Pages are laid out side
//! by side; page count, per-page pixels, union pixels and the aggregate pixel budget are bounded.
//!
//! Text and vector page objects can be imported as editable layers (`ImportOptions::pdf_text` /
//! `pdf_vectors`); their page-space points are mapped to document pixels through the effective DPI.

use crate::{ImportOptions, ImportResult, IoError};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Artboard, Color, Document, Fill, Group, Knot, Layer, LayerContent, Path, PathOp, ShapeLayer, ShapeStroke, Subpath, TextLayer};
use photocraft_geom::{Affine, Point, Rect, Size};
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

/// Renders the selected PDF pages and opens each as one artboard.
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
    let (first, last) = match opts.pdf_pages {
        Some((a, b)) => {
            let a = (a.max(1) as usize).min(total.max(1));
            let b = (b.max(1) as usize).min(total.max(1));
            (a.min(b), a.max(b))
        }
        None => (1, total),
    };
    if total == 0 || first > total {
        return Err(IoError::Unsupported("PDF has no pages in the requested range".into()));
    }
    let page_count = last - first + 1;
    let count = page_count.min(MAX_PAGES);
    // The page sizes in points first: the effective DPI must fit the budgets, and capping the
    // raster must not change the page's physical size.
    let mut sizes = Vec::with_capacity(count);
    for index in 0..count {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let page_index = first - 1 + index;
        let page = document.pages().get(page_index as i32).map_err(|e| IoError::Unsupported(format!("PDF: {e}")))?;
        let (w, h) = (f64::from(page.width().value), f64::from(page.height().value));
        if w <= 0.0 || h <= 0.0 {
            return Err(IoError::Unsupported(format!("PDF page {} has no area", page_index + 1)));
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
        let page_index = first - 1 + index;
        let target_width = (w_pt * dpi / 72.0).round().max(1.0) as i32;
        let target_height = (h_pt * dpi / 72.0).round().max(1.0) as i32;
        // A one-pixel-wide page would otherwise produce an arbitrarily tall bitmap before the
        // union check runs; reject the final dimensions up front.
        if f64::from(target_width) * f64::from(target_height) > MAX_PAGE_PIXELS {
            return Err(IoError::Unsupported(format!(
                "PDF page {} is too large at {dpi} dpi ({} x {} px); lower the import DPI",
                page_index + 1,
                target_width,
                target_height
            )));
        }
        let page = document.pages().get(page_index as i32).map_err(|e| IoError::Unsupported(format!("PDF: {e}")))?;
        let bitmap =
            page.render_with_config(&PdfRenderConfig::new().set_target_width(target_width)).map_err(|e| IoError::Unsupported(format!("PDF render: {e}")))?;
        let (width, height) = (bitmap.width().max(0) as u32, bitmap.height().max(0) as u32);
        // The rounded target width can render a taller bitmap than the pre-check predicted.
        if f64::from(width) * f64::from(height) > MAX_PAGE_PIXELS {
            return Err(IoError::Unsupported(format!(
                "PDF page {} rendered larger than the {MAX_PAGE_PIXELS} px budget ({} x {}); lower the import DPI",
                page_index + 1,
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
        let layer = Layer::new(format!("Page {}", page_index + 1), LayerContent::Raster(surface));
        let rect = Rect::new(x as i32, 0, (x + width) as i32, height as i32);
        let mut children = vec![layer];
        if opts.pdf_text || opts.pdf_vectors {
            children.extend(object_layers(&page, *h_pt, dpi, x, opts));
        }
        let group = Group { children, expanded: true, artboard: Some(Artboard::new(rect)), frame_stroke: None };
        doc.layers.push(Layer::new(format!("Page {}", page_index + 1), LayerContent::Group(group)));
        x += width + GAP;
        ctl.progress((index + 1) as f32 / count as f32);
    }
    let mut warnings = Vec::new();
    if first > 1 || last < total || count < page_count {
        let shown_last = first + count - 1;
        let reason = if count < page_count { " (the page limit is 16)" } else { "" };
        warnings.push(format!("PDF has {total} pages; imported pages {first}-{shown_last}{reason}"));
    }
    Ok(ImportResult { warnings, document: doc, source_read_only: false, preview_only: false })
}

/// Editable text and shape layers for one rendered page, in document pixels.
#[cfg(not(target_arch = "wasm32"))]
fn object_layers(page: &pdfium_render::prelude::PdfPage, height_pt: f64, dpi: f64, x: u32, opts: &ImportOptions) -> Vec<Layer> {
    use pdfium_render::prelude::*;

    let to_x = |v: f32| f64::from(v) * dpi / 72.0;
    let objects = page.objects();
    let mut layers = Vec::new();
    for object in objects.iter() {
        match object {
            PdfPageObject::Text(text) if opts.pdf_text => {
                let content = text.text();
                if content.trim().is_empty() {
                    continue;
                }
                let Ok(bounds) = text.bounds() else { continue };
                let x_px = to_x(bounds.left().value).round() + f64::from(x);
                let y_px = (height_pt - to_x(bounds.top().value)).round();
                // The text layer's size is in points; the transform is in document pixels.
                let size_pt = f64::from(text.scaled_font_size().value).max(1.0);
                layers.push(Layer::new(
                    "Text",
                    LayerContent::Text(TextLayer {
                        text: content,
                        font_family: text.font().family(),
                        size_pt: size_pt as f32,
                        color: Color::BLACK,
                        transform: Affine::translate(x_px, y_px),
                        ..Default::default()
                    }),
                ));
            }
            PdfPageObject::Path(path) if opts.pdf_vectors => {
                let mut subpaths: Vec<Subpath> = Vec::new();
                let mut knots: Vec<Knot> = Vec::new();
                let mut closed = false;
                let mut bezier: Vec<(f64, f64)> = Vec::new();
                for segment in path.segments().iter() {
                    let point = || (to_x(segment.x().value).round(), (height_pt - to_x(segment.y().value)).round());
                    if segment.is_close() {
                        closed = true;
                        continue;
                    }
                    match segment.segment_type() {
                        PdfPathSegmentType::MoveTo => {
                            if !knots.is_empty() {
                                subpaths.push(Subpath { closed, knots: std::mem::take(&mut knots), op: PathOp::Combine });
                                closed = false;
                            }
                            let (px, py) = point();
                            knots.push(Knot::corner(px, py));
                        }
                        PdfPathSegmentType::LineTo => {
                            let (px, py) = point();
                            knots.push(Knot::corner(px, py));
                        }
                        PdfPathSegmentType::BezierTo => {
                            // PDF draws a cubic Bézier as three consecutive BezierTo points: the
                            // two control points, then the end point.
                            bezier.push(point());
                            if bezier.len() == 3 {
                                let ((c1x, c1y), (c2x, c2y), (ex, ey)) = (bezier[0], bezier[1], bezier[2]);
                                if let Some(prev) = knots.last_mut() {
                                    prev.out_ctrl = Point::new(c1x, c1y);
                                }
                                knots.push(Knot::smooth(Point::new(ex, ey), Point::new(c2x, c2y), Point::new(ex, ey)));
                                bezier.clear();
                            }
                        }
                        PdfPathSegmentType::Unknown => {}
                    }
                }
                if !knots.is_empty() {
                    subpaths.push(Subpath { closed, knots, op: PathOp::Combine });
                }
                if subpaths.is_empty() {
                    continue;
                }
                let stroke = path.is_stroked().unwrap_or(false).then(|| ShapeStroke { paint: Fill::Solid(Color::BLACK), width: 1.0, ..Default::default() });
                layers.push(Layer::new(
                    "Path",
                    LayerContent::Shape(ShapeLayer { path: Path::new(subpaths), fill: Some(Fill::Solid(Color::BLACK)), stroke, ..Default::default() }),
                ));
            }
            _ => {}
        }
    }
    layers
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
