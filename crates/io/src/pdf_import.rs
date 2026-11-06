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
    /// The default pixel-memory budget when the caller sets none: a zero
    /// [`ImportOptions::flat_max_pixel_bytes`] still bounds the import.
    const DEFAULT_MAX_PIXEL_BYTES: u64 = 2 << 30;

    let requested = if opts.pdf_dpi.is_finite() && opts.pdf_dpi > 0.0 { f64::from(opts.pdf_dpi) } else { 300.0 };
    let pdfium = bind()?;
    let document = pdfium.load_pdf_from_byte_slice(bytes, None).map_err(|e| IoError::Unsupported(format!("PDF: {e}")))?;
    let total = document.pages().len().max(0) as usize;
    let (first, last) = match opts.pdf_pages {
        Some((a, b)) => {
            let a = a.max(1);
            let b = if b == 0 { u32::MAX } else { b.max(a) };
            if u64::from(a) > total as u64 {
                return Err(IoError::Unsupported(format!("PDF page range starts at {a}, but the PDF has {total} pages")));
            }
            ((a as usize).min(total), (b as usize).min(total))
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
    // The aggregate budget: the document keeps every page's raster tiles, while one page at a
    // time is converted through the PDFium bitmap, its RGBA copy and the f32 buffer.
    let page_px = |w: f64, h: f64| (w * dpi / 72.0).round().max(1.0) * (h * dpi / 72.0).round().max(1.0);
    let max_page_pixels = sizes.iter().map(|&(w, h)| page_px(w, h)).fold(0.0_f64, f64::max);
    // Retained backing: a page occupies whole 256 px tiles, including the ones its document X
    // offset steps into. A 1 x 10,000 px page is 40 tiles (10 MiB), not 10,000 pixels.
    const TILE: u64 = 256;
    let mut retained: u64 = 0;
    let mut page_x: u64 = 0;
    for &(w, h) in &sizes {
        let px = (w * dpi / 72.0).round().max(1.0) as u64;
        let py = (h * dpi / 72.0).round().max(1.0) as u64;
        let tiles_x = (page_x + px).div_ceil(TILE) - page_x / TILE;
        retained = retained.saturating_add(tiles_x.saturating_mul(py.div_ceil(TILE)).saturating_mul(TILE * TILE * 4));
        page_x += px + u64::from(GAP);
    }
    // Plus 24 bytes/px (bitmap, RGBA copy, f32 conversion buffer) for the page being converted.
    let estimate = retained.saturating_add((max_page_pixels * 24.0) as u64);
    let budget = if opts.flat_max_pixel_bytes > 0 { opts.flat_max_pixel_bytes } else { DEFAULT_MAX_PIXEL_BYTES };
    if estimate > budget {
        return Err(IoError::Unsupported(format!(
            "PDF import needs about {} MB of pixel memory; the limit is {} MB (lower the import DPI)",
            estimate >> 20,
            budget >> 20
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
        let mut data = Vec::new();
        if data.try_reserve_exact(rgba.len()).is_err() {
            return Err(IoError::Unsupported(format!("PDF page {} needs more memory than is available", page_index + 1)));
        }
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
            children.extend(object_layers(&page, opts, &PageFrame { height_pt: *h_pt, dpi, x, rgba: &rgba, width, height }));
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

/// One rendered page's frame: its point height, DPI, document x, and the RGBA pixels.
struct PageFrame<'a> {
    height_pt: f64,
    dpi: f64,
    x: u32,
    rgba: &'a [u8],
    width: u32,
    height: u32,
}

impl PageFrame<'_> {
    /// Maps a point in PDF page space (points, origin bottom-left) to document pixels: the Y flip
    /// and the page's X offset both belong to this mapping.
    fn to_doc(&self, x_pt: f32, y_pt: f32) -> (f64, f64) {
        let x_px = f64::from(x_pt) * self.dpi / 72.0 + f64::from(self.x);
        let y_px = (self.height_pt - f64::from(y_pt)) * self.dpi / 72.0;
        (x_px.round(), y_px.round())
    }

    /// The rendered page's colour at local pixel (`x`, `y`), or black outside it.
    fn color_at(&self, x: u32, y: u32) -> Color {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        if i + 3 < self.rgba.len() && self.rgba[i + 3] > 0 {
            Color::rgb(f32::from(self.rgba[i]) / 255.0, f32::from(self.rgba[i + 1]) / 255.0, f32::from(self.rgba[i + 2]) / 255.0)
        } else {
            Color::BLACK
        }
    }

    /// The ink colour of the text in `bounds` (page points).
    ///
    /// The box holds both glyphs and the page behind them, so sampling one corner usually finds
    /// background or an antialiased edge. Take the pixel farthest from the box's most common
    /// colour instead: for dark text on a light page that is a glyph core, and for light text on a
    /// dark page it is the glyph rather than the background.
    fn text_color(&self, left: f32, top: f32, right: f32, bottom: f32) -> Color {
        let x0 = (f64::from(left) * self.dpi / 72.0).floor().max(0.0) as u32;
        let x1 = (f64::from(right) * self.dpi / 72.0).ceil().clamp(0.0, f64::from(self.width)) as u32;
        // PDF's Y grows up; the rendered page's Y grows down.
        let y0 = ((self.height_pt - f64::from(top)) * self.dpi / 72.0).floor().max(0.0) as u32;
        let y1 = ((self.height_pt - f64::from(bottom)) * self.dpi / 72.0).ceil().clamp(0.0, f64::from(self.height)) as u32;
        if x1 <= x0 || y1 <= y0 {
            return Color::BLACK;
        }
        // At most ~4096 samples, so a page-sized text box stays cheap.
        let step = ((f64::from(x1 - x0) * f64::from(y1 - y0) / 4096.0).sqrt().ceil() as u32).max(1);
        let mut samples: Vec<[f32; 3]> = Vec::new();
        let mut buckets: std::collections::HashMap<u32, (u32, [f32; 3])> = std::collections::HashMap::new();
        let mut y = y0;
        while y < y1 {
            let mut x = x0;
            while x < x1 {
                let c = self.color_at(x, y);
                let rgb = [c.c[0], c.c[1], c.c[2]];
                let key = (rgb[0] * 31.0) as u32 * 1024 + (rgb[1] * 31.0) as u32 * 32 + (rgb[2] * 31.0) as u32;
                let entry = buckets.entry(key).or_insert((0, [0.0; 3]));
                entry.0 += 1;
                for (sum, v) in entry.1.iter_mut().zip(rgb) {
                    *sum += v;
                }
                samples.push(rgb);
                x += step;
            }
            y += step;
        }
        let Some((_, (count, sum))) = buckets.iter().max_by_key(|(_, (count, _))| *count) else { return Color::BLACK };
        let mode = [sum[0] / *count as f32, sum[1] / *count as f32, sum[2] / *count as f32];
        let mut best = mode;
        let mut best_distance = -1.0f32;
        for rgb in samples {
            let distance = (rgb[0] - mode[0]).powi(2) + (rgb[1] - mode[1]).powi(2) + (rgb[2] - mode[2]).powi(2);
            if distance > best_distance {
                best_distance = distance;
                best = rgb;
            }
        }
        Color::rgb(best[0], best[1], best[2])
    }
}

/// Editable text and shape layers for one rendered page, in document pixels.
#[cfg(not(target_arch = "wasm32"))]
fn object_layers(page: &pdfium_render::prelude::PdfPage, opts: &ImportOptions, frame: &PageFrame) -> Vec<Layer> {
    use pdfium_render::prelude::*;

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
                let (x_px, y_px) = frame.to_doc(bounds.left().value, bounds.top().value);
                // The text layer's size is in points; the transform is in document pixels. The
                // text object does not expose its fill colour; sample the glyph ink from the page.
                let color = frame.text_color(bounds.left().value, bounds.top().value, bounds.right().value, bounds.bottom().value);
                let size_pt = f64::from(text.scaled_font_size().value).max(1.0);
                let font = text.font();
                let family = if font.family().is_empty() { font.name() } else { font.family() };
                let mut layer = Layer::new(
                    "Text",
                    LayerContent::Text(TextLayer {
                        text: content,
                        font_family: family,
                        size_pt: size_pt as f32,
                        color,
                        transform: Affine::translate(x_px, y_px),
                        ..Default::default()
                    }),
                );
                if !text.is_visible() {
                    layer.visible = false;
                }
                layers.push(layer);
            }
            PdfPageObject::Path(path) if opts.pdf_vectors => {
                // PDFium reports the segments in object space; the object's matrix places them on
                // the page. Without it, scaled or rotated paths land in the wrong place.
                let matrix = path.matrix().unwrap_or(PdfMatrix::IDENTITY);
                // The fill and stroke modes are exposed; the colours and dash pattern are not.
                let filled = !matches!(path.fill_mode(), Ok(PdfPathFillMode::None));
                let stroked = path.is_stroked().unwrap_or(false);
                if !filled && !stroked {
                    continue;
                }
                let mut subpaths: Vec<Subpath> = Vec::new();
                let mut knots: Vec<Knot> = Vec::new();
                let mut closed = false;
                let mut bezier: Vec<(f64, f64)> = Vec::new();
                for segment in path.segments().transform(matrix).iter() {
                    let point = || frame.to_doc(segment.x().value, segment.y().value);
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
                    // A closing segment can also be the endpoint that closes a Bézier; process it
                    // above before marking the subpath closed so its control points are consumed.
                    if segment.is_close() {
                        closed = true;
                    }
                }
                if !knots.is_empty() {
                    subpaths.push(Subpath { closed, knots, op: PathOp::Combine });
                }
                if subpaths.is_empty() {
                    continue;
                }
                let stroke = stroked.then(|| ShapeStroke { paint: Fill::Solid(Color::BLACK), width: 1.0, ..Default::default() });
                layers.push(Layer::new(
                    "Path",
                    LayerContent::Shape(ShapeLayer {
                        path: Path::new(subpaths),
                        fill: filled.then_some(Fill::Solid(Color::BLACK)),
                        stroke,
                        ..Default::default()
                    }),
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
///
/// pdfium-render keeps the bindings in a process-wide static: the first import initializes them
/// and later `bind_to_library` calls fail with `PdfiumLibraryBindingsAlreadyInitialized`. Remember
/// the outcome and reuse the initialized bindings, so opening a second PDF in one session works.
#[cfg(not(target_arch = "wasm32"))]
fn bind() -> Result<pdfium_render::prelude::Pdfium, IoError> {
    use pdfium_render::prelude::Pdfium;

    static INIT: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
    let init = INIT.get_or_init(|| {
        let dir = std::env::var_os("PDFIUM_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::env::current_exe().ok().and_then(|exe| exe.parent().map(std::path::Path::to_path_buf)).unwrap_or_default());
        let library = Pdfium::pdfium_platform_library_name_at_path(&dir);
        let platform = Pdfium::pdfium_platform_library_name().to_string_lossy().into_owned();
        match Pdfium::bind_to_library(&library) {
            Ok(bindings) => {
                // Initializes the library inside pdfium-render's process-wide bindings.
                drop(Pdfium::new(bindings));
                Ok(())
            }
            Err(e) => Err(format!("PDF import: cannot load PDFium from {} ({e}); put {platform} next to the executable or set PDFIUM_DIR", library.display())),
        }
    });
    match init {
        // The bindings are already initialized; `default()` re-uses them.
        Ok(()) => Ok(Pdfium::default()),
        Err(message) => Err(IoError::Unsupported(message.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::PageFrame;

    fn frame(dpi: f64, x: u32) -> PageFrame<'static> {
        PageFrame { height_pt: 842.0, dpi, x, rgba: &[], width: 595, height: 842 }
    }

    #[test]
    fn page_points_map_to_document_pixels() {
        assert_eq!(frame(72.0, 0).to_doc(60.0, 420.0), (60.0, 422.0));
        // The same point at four times the resolution.
        assert_eq!(frame(300.0, 0).to_doc(60.0, 420.0), (250.0, 1758.0));
        // Page 2 starts after page 1's width: the document X offset is part of the mapping.
        assert_eq!(frame(300.0, 2480).to_doc(60.0, 420.0), (2730.0, 1758.0));
    }

    #[test]
    fn samples_the_rendered_page() {
        // Two pixels: white then black.
        let rgba = [255u8, 255, 255, 255, 0, 0, 0, 255];
        let page = PageFrame { height_pt: 842.0, dpi: 72.0, x: 0, rgba: &rgba, width: 2, height: 1 };
        assert_eq!(page.color_at(0, 0), photocraft_doc::Color::rgb(1.0, 1.0, 1.0));
        assert_eq!(page.color_at(1, 0), photocraft_doc::Color::BLACK);
        assert_eq!(page.color_at(2, 0), photocraft_doc::Color::BLACK);
    }

    #[test]
    fn text_colour_samples_the_glyph_ink() {
        // An 8 x 4 white page with one black glyph pixel at (2, 1).
        let mut rgba = vec![255u8; 8 * 4 * 4];
        let glyph = (8 + 2) * 4;
        rgba[glyph..glyph + 3].fill(0);
        let page = PageFrame { height_pt: 4.0, dpi: 72.0, x: 0, rgba: &rgba, width: 8, height: 4 };
        // The whole page box contains the glyph: the ink wins over the white background.
        assert_eq!(page.text_color(0.0, 4.0, 8.0, 0.0), photocraft_doc::Color::BLACK);
        // A box that contains only background keeps the background colour.
        assert_eq!(page.text_color(4.0, 4.0, 8.0, 2.0), photocraft_doc::Color::rgb(1.0, 1.0, 1.0));
    }
}
