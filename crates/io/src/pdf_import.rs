//! PDF import: craft-pdf (a Rust PDF reader and renderer built on Hayro) rasterizes the
//! selected pages, and each page becomes one artboard.
//!
//! Raster pages only: editable text and vector import would need craft-pdf's `interpret`
//! device and is not part of this importer. The renderer's hard output bounds (8192 px per
//! side and 64M px per page) and a document-wide strip budget are respected by lowering the
//! effective DPI, so each page keeps its physical size: pixels / resolution * 72 pt.
//!
//! The renderer is pure Rust and runs on every desktop platform: no shared library is loaded
//! or shipped, and a malformed file is reported as an error rather than taking the process down.
//!
//! Pages are drawn with the interpreter's default annotation settings, so annotations and
//! filled-in form fields are rendered; comments are not hidden.

use crate::{ImportOptions, ImportResult, IoError};
#[cfg(not(target_arch = "wasm32"))]
use craft_pdf::interpret::InterpreterSettings;
#[cfg(not(target_arch = "wasm32"))]
use craft_pdf::raster::{self, RenderCache};
#[cfg(not(target_arch = "wasm32"))]
use photocraft_color::{ColorMode, PixelFormat, SampleType};
#[cfg(not(target_arch = "wasm32"))]
use photocraft_doc::{Artboard, Document, Group, Layer, LayerContent};
#[cfg(not(target_arch = "wasm32"))]
use photocraft_geom::{Rect, Size};
use photocraft_raster::Interrupt;
#[cfg(not(target_arch = "wasm32"))]
use photocraft_raster::Surface;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

/// `%PDF-` magic. `.ai` files that start with it are PDFs and render through this importer.
#[cfg(test)]
pub fn is_pdf(bytes: &[u8]) -> bool {
    bytes.starts_with(b"%PDF-")
}

/// PDF import is not available in the web build.
#[cfg(target_arch = "wasm32")]
pub fn import(_name: &str, _bytes: &[u8], _opts: &ImportOptions, _ctl: &Interrupt) -> Result<ImportResult, IoError> {
    Err(IoError::Unsupported("PDF import needs the desktop build".into()))
}

/// The most pages one import renders.
#[cfg(not(target_arch = "wasm32"))]
const MAX_PAGES: usize = 16;
/// Gap between the artboards, in document pixels.
#[cfg(not(target_arch = "wasm32"))]
const GAP: u32 = 80;
/// The document-wide raster budget across every imported page, in pixels.
#[cfg(not(target_arch = "wasm32"))]
const MAX_UNION_PIXELS: u64 = 200_000_000;

#[cfg(not(target_arch = "wasm32"))]
mod placed;
#[cfg(not(target_arch = "wasm32"))]
pub use placed::import_placed;

/// A placed PDF can't be drawn in its frame in the web build.
#[cfg(target_arch = "wasm32")]
pub fn import_placed(_name: &str, _bytes: &[u8], _placed: &crate::smart_map::PlacedPdf, _ctl: &Interrupt) -> Result<photocraft_doc::Document, IoError> {
    Err(IoError::Unsupported("PDF import needs the desktop build".into()))
}

/// How another application placed the PDF `bytes` as a smart object, from the layer's placed
/// block `sold` (`SoLd`/`SoLE` data), when its frame isn't the canvas the PDF opens to by
/// itself. Photoshop stores the placement's crop there, often the artwork's bounds, and the
/// layer's corners map from it: drawing the whole page into them blows the artwork up to the
/// page's size (#15650). PhotoCraft's own placements store the canvas, so they get `None` and
/// keep the ordinary import.
pub fn placement(bytes: &[u8], sold: &[u8]) -> Option<crate::smart_map::PlacedPdf> {
    if !bytes.starts_with(b"%PDF-") {
        return None;
    }
    let placed = crate::smart_map::placed_pdf(&crate::smart_map::sold_descriptor(sold)?)?;
    (default_canvas(bytes)? != (placed.frame.0.round() as u64, placed.frame.1.round() as u64)).then_some(placed)
}

/// The canvas size [`import`] gives `bytes` with default options (every page side by side at
/// 300 dpi), worked out from the page sizes without rendering. `None` when the PDF doesn't open.
#[cfg(not(target_arch = "wasm32"))]
pub fn default_canvas(bytes: &[u8]) -> Option<(u64, u64)> {
    let pdf = craft_pdf::open_slice_with_options(bytes, None, &craft_pdf::OpenOptions::default()).ok()?;
    let sizes = pdf.pages().iter().take(MAX_PAGES).enumerate().map(|(index, page)| page_size(page, index + 1)).collect::<Result<Vec<_>, _>>().ok()?;
    if sizes.is_empty() {
        return None;
    }
    let dpi = effective_dpi(f64::from(ImportOptions::default().pdf_dpi), &sizes, u64::from(raster::MAX_SIDE), raster::MAX_PIXELS, MAX_UNION_PIXELS);
    Some(strip_size(&sizes, dpi))
}

/// PDF import is not available in the web build.
#[cfg(target_arch = "wasm32")]
pub fn default_canvas(_bytes: &[u8]) -> Option<(u64, u64)> {
    None
}

/// A page's rendered size in points (after its crop box, rotation and user unit).
#[cfg(not(target_arch = "wasm32"))]
fn page_size(page: &craft_pdf::syntax::page::Page<'_>, number: usize) -> Result<(f64, f64), IoError> {
    let (width, height) = page.render_dimensions();
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err(IoError::Unsupported(format!("PDF page {number} has no area")));
    }
    Ok((f64::from(width), f64::from(height)))
}

/// The pixel size of the strip of pages [`import`] lays out at `dpi`: side by side, [`GAP`] apart.
#[cfg(not(target_arch = "wasm32"))]
fn strip_size(sizes: &[(f64, f64)], dpi: f64) -> (u64, u64) {
    let width = sizes.iter().map(|&(w, _)| u64::from(page_pixels(w, dpi))).fold(0u64, u64::saturating_add);
    let gaps = u64::from(GAP).saturating_mul(sizes.len().saturating_sub(1) as u64);
    let height = sizes.iter().map(|&(_, h)| u64::from(page_pixels(h, dpi))).max().unwrap_or(0);
    (width.saturating_add(gaps), height)
}

/// Renders the selected PDF pages and opens each as one artboard.
#[cfg(not(target_arch = "wasm32"))]
pub fn import(name: &str, bytes: &[u8], opts: &ImportOptions, ctl: &Interrupt) -> Result<ImportResult, IoError> {
    /// The default pixel-memory budget when the caller sets none: a zero
    /// [`ImportOptions::flat_max_pixel_bytes`] still bounds the import.
    const DEFAULT_MAX_PIXEL_BYTES: u64 = 2 << 30;

    ctl.check().map_err(|_| IoError::Cancelled)?;
    let requested = if opts.pdf_dpi.is_finite() && opts.pdf_dpi > 0.0 { f64::from(opts.pdf_dpi) } else { 300.0 };
    let pdf = craft_pdf::open_slice_with_options(bytes, None, &craft_pdf::OpenOptions::default()).map_err(open_error)?;
    let total = pdf.pages().len();
    if total == 0 {
        return Err(IoError::Unsupported("PDF has no pages".into()));
    }
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
    let page_count = last - first + 1;
    let count = page_count.min(MAX_PAGES);

    // Page sizes in points first: the effective DPI must fit the renderer's bounds without
    // changing any page's physical size.
    let mut sizes = Vec::with_capacity(count);
    for (index, page) in pdf.pages().iter().skip(first - 1).take(count).enumerate() {
        sizes.push(page_size(page, first + index)?);
        ctl.check().map_err(|_| IoError::Cancelled)?;
    }
    let dpi = effective_dpi(requested, &sizes, u64::from(raster::MAX_SIDE), raster::MAX_PIXELS, MAX_UNION_PIXELS);

    let (doc_width, doc_height) = strip_size(&sizes, dpi);
    let mut retained: u64 = 0;
    for &(width_pt, height_pt) in &sizes {
        let width = page_pixels(width_pt, dpi);
        let height = page_pixels(height_pt, dpi);
        let tiles_x = (u64::from(width)).div_ceil(256);
        let tiles_y = (u64::from(height)).div_ceil(256);
        retained = retained.saturating_add(tiles_x.saturating_mul(tiles_y).saturating_mul(256 * 256 * 4));
    }
    if doc_width == 0 || doc_height == 0 {
        return Err(IoError::Unsupported("PDF pages have no area".into()));
    }
    if doc_width > u64::from(u32::MAX) || doc_height > u64::from(u32::MAX) {
        return Err(IoError::Unsupported("PDF import is too large at this DPI; lower the import DPI".into()));
    }
    // Retained tiles plus the converting page's raster (4 B/px); the f32 conversion runs in
    // 256-row bands, so it adds only a constant-sized buffer rather than a second full page.
    let max_page = sizes.iter().map(|&(w, h)| u64::from(page_pixels(w, dpi)) * u64::from(page_pixels(h, dpi))).max().unwrap_or(0);
    let estimate = retained.saturating_add(max_page.saturating_mul(4));
    let budget = if opts.flat_max_pixel_bytes > 0 { opts.flat_max_pixel_bytes } else { DEFAULT_MAX_PIXEL_BYTES };
    if estimate > budget {
        return Err(IoError::Unsupported(format!(
            "PDF import needs about {} MB of pixel memory; the limit is {} MB (lower the import DPI)",
            estimate >> 20,
            budget >> 20
        )));
    }

    let mut warnings = Vec::new();
    if first > 1 || last < total || count < page_count {
        let shown_last = first + count - 1;
        let reason = if count < page_count { " (the page limit is 16)" } else { "" };
        warnings.push(format!("PDF has {total} pages; imported pages {first}-{shown_last}{reason}"));
    }
    if dpi + 0.5 < requested {
        warnings.push(format!("PDF was rendered at {:.0} dpi to fit the import limits", dpi.max(1.0)));
    }

    let mut doc = Document::new(name, Size::new(doc_width as u32, doc_height as u32), ColorMode::Rgb, SampleType::U8);
    doc.resolution_dpi = dpi as f32;
    #[cfg(feature = "pdf-editable")]
    let mut editable_text = false;
    let mut x: u32 = 0;
    for (index, &(width_pt, height_pt)) in sizes.iter().enumerate() {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let page_number = first + index;
        let page = &pdf.pages()[page_number - 1];
        let width = page_pixels(width_pt, dpi);
        let height = page_pixels(height_pt, dpi);
        // The DPI solve keeps these inside the renderer's bounds; re-check rather than panic if
        // rounding on an edge moves one pixel past them.
        if width > raster::MAX_SIDE || height > raster::MAX_SIDE || u64::from(width) * u64::from(height) > raster::MAX_PIXELS {
            return Err(IoError::Unsupported(format!("PDF page {page_number} is too large at {dpi:.0} dpi; lower the import DPI")));
        }
        let rect = Rect::new(x as i32, 0, (x + width) as i32, height as i32);
        #[cfg(feature = "pdf-editable")]
        if opts.pdf_editable {
            match crate::pdf_editable::page(page, rect, ctl) {
                Ok(layers) => {
                    if layers.iter().any(|layer| matches!(&layer.content, LayerContent::Text(_))) {
                        editable_text = true;
                    }
                    let group = Group { children: layers, expanded: true, artboard: Some(Artboard::new(rect)), frame_stroke: None };
                    doc.layers.push(Layer::new(format!("Page {page_number}"), LayerContent::Group(group)));
                    x = x.saturating_add(width).saturating_add(GAP);
                    ctl.progress((index + 1) as f32 / count as f32);
                    continue;
                }
                Err(crate::pdf_editable::EditableError::Cancelled) => return Err(IoError::Cancelled),
                Err(crate::pdf_editable::EditableError::Unsupported(reason)) => {
                    warnings.push(format!("PDF page {page_number}: {reason}; the page was rasterised"));
                }
            }
        }
        #[cfg(not(feature = "pdf-editable"))]
        if opts.pdf_editable {
            warnings.push(format!("PDF page {page_number}: editable PDF import is not built into this binary; the page was rasterised"));
        }
        // A cache per page, deliberately: a shared one would keep decoded images and fonts alive
        // across up to 16 pages outside the pixel budget, and would hide an image-decode warning
        // on a later page after the page that first hit it.
        let cache = RenderCache::new();
        let options = raster::Options {
            scale: [(dpi / 72.0) as f32; 2],
            size: Some([width, height]),
            offset: [0.0, 0.0],
            // PDF pages composite onto white, as every PDF viewer and the previous importer did.
            // craft-pdf returns premultiplied RGBA8; an opaque background makes alpha 1, so the
            // bytes below can be written as straight alpha. A transparent background would need
            // un-premultiplying (dividing each colour by alpha) first.
            background: [255, 255, 255, 255],
            color_replacement: None,
            // craft-pdf's own default: up to 2^31 coverage values per page.
            max_coverage: None,
        };
        let rendered = render_page(page, &cache, &options, ctl).map_err(|error| render_error(page_number, dpi, error))?;
        if rendered.diagnostics.image_decode_failed {
            warnings.push(format!("PDF page {page_number}: an image could not be decoded and is missing"));
        }
        // craft-pdf's hayro backend does not emit this flag yet (its README says so); keep the
        // warning ready so a backend that starts reporting font substitution needs no change here.
        if rendered.diagnostics.unsupported_font {
            warnings.push(format!("PDF page {page_number}: a font could not be used and was substituted, so text may look different"));
        }
        if rendered.diagnostics.content_truncated {
            warnings.push(format!("PDF page {page_number}: some content was omitted (a decode or memory limit)"));
        }
        ctl.check().map_err(|_| IoError::Cancelled)?;

        let mut surface = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
        write_raster(&mut surface, x as i32, width, rendered.bytes(), false, &format!("PDF page {page_number}"), ctl)?;
        let layer = Layer::new(format!("Page {page_number}"), LayerContent::Raster(surface));
        let group = Group { children: vec![layer], expanded: true, artboard: Some(Artboard::new(rect)), frame_stroke: None };
        doc.layers.push(Layer::new(format!("Page {page_number}"), LayerContent::Group(group)));
        x = x.saturating_add(width).saturating_add(GAP);
        ctl.progress((index + 1) as f32 / count as f32);
    }
    #[cfg(feature = "pdf-editable")]
    if editable_text {
        warnings.push("PDF text uses fonts embedded in the PDF; it may change font when edited or reopened".into());
    }

    Ok(ImportResult { warnings, document: doc, source_read_only: false, preview_only: false })
}

/// Writes a rendered RGBA8 raster `width` pixels wide into `surface` from column `x`, in row
/// bands: the f32 buffer stays bounded instead of a second 16-bytes-per-pixel copy of the page,
/// and cancellation is polled per band. `premultiplied` output is divided back to straight alpha.
/// `what` names the raster in the out-of-memory error.
#[cfg(not(target_arch = "wasm32"))]
fn write_raster(surface: &mut Surface, x: i32, width: u32, bytes: &[u8], premultiplied: bool, what: &str, ctl: &Interrupt) -> Result<(), IoError> {
    const BAND_ROWS: usize = 256;
    let row_bytes = width as usize * 4;
    let band_bytes = row_bytes.saturating_mul(BAND_ROWS);
    let mut data: Vec<f32> = Vec::new();
    data.try_reserve_exact(band_bytes).map_err(|_| IoError::Unsupported(format!("{what} needs more memory than is available")))?;
    for (band, rows) in bytes.chunks(band_bytes).enumerate() {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        data.clear();
        if premultiplied {
            for &[r, g, b, a] in rows.as_chunks::<4>().0 {
                if a == 0 {
                    data.extend([0.0; 4]);
                } else {
                    let a = f32::from(a);
                    data.extend([r, g, b].map(|c| (f32::from(c) / a).min(1.0)));
                    data.push(a / 255.0);
                }
            }
        } else {
            data.extend(rows.iter().map(|&value| f32::from(value) / 255.0));
        }
        let band_height = (rows.len() / row_bytes) as u32;
        surface.write_region(Rect::from_xywh(x, (band * BAND_ROWS) as i32, width, band_height), &data);
    }
    Ok(())
}

/// Renders one page, forwarding `ctl`'s cancellation to craft-pdf while it interprets content.
///
/// craft-pdf checks `InterpreterSettings::cancelled` between content operators, so a dense
/// vector page (CAD, maps) can be stopped part-way through. `Interrupt` is a borrowed callback
/// while the setting needs an owned flag, so a scoped watcher thread copies one into the other
/// every few milliseconds until the render returns.
#[cfg(not(target_arch = "wasm32"))]
fn render_page<'a>(
    page: &'a craft_pdf::syntax::page::Page<'a>,
    cache: &RenderCache<'a>,
    options: &raster::Options,
    ctl: &Interrupt,
) -> Result<raster::Raster, raster::Error> {
    if ctl.cancelled() {
        return Err(raster::Error::Cancelled);
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    let settings = InterpreterSettings { cancelled: Some(Arc::clone(&cancelled)), ..InterpreterSettings::default() };
    std::thread::scope(|scope| {
        let flag = Arc::clone(&cancelled);
        let done = Arc::clone(&finished);
        let ctl = *ctl;
        scope.spawn(move || {
            while !done.load(Ordering::Relaxed) {
                if ctl.cancelled() {
                    flag.store(true, Ordering::Relaxed);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        let rendered = raster::render(page, cache, &settings, options);
        finished.store(true, Ordering::Relaxed);
        rendered
    })
}

/// One page's raster width or height in pixels at `dpi`, at least one.
#[cfg(not(target_arch = "wasm32"))]
fn page_pixels(points: f64, dpi: f64) -> u32 {
    (points * dpi / 72.0).round().max(1.0) as u32
}

/// The highest DPI not above `requested` that keeps every page inside the renderer's side and
/// pixel bounds and the whole strip inside `max_union_pixels`. Rendering at that DPI keeps each
/// page's physical size unchanged; unreachable bounds report 1 DPI and the render call then fails
/// with a clear message.
#[cfg(not(target_arch = "wasm32"))]
fn effective_dpi(requested: f64, sizes: &[(f64, f64)], max_side: u64, max_page_pixels: u64, max_union_pixels: u64) -> f64 {
    let mut dpi = requested.clamp(1.0, 2400.0);
    loop {
        let mut union = 0u64;
        let mut fits = true;
        for &(width_pt, height_pt) in sizes {
            let width = u64::from(page_pixels(width_pt, dpi));
            let height = u64::from(page_pixels(height_pt, dpi));
            if width > max_side || height > max_side || width.saturating_mul(height) > max_page_pixels {
                fits = false;
                break;
            }
            union = union.saturating_add(width.saturating_mul(height));
        }
        if fits && union <= max_union_pixels {
            return dpi;
        }
        if dpi <= 1.0 {
            return 1.0;
        }
        // The strip's gaps contribute a constant width, so a geometric step converges without
        // needing a closed form per page.
        dpi = (dpi * 0.9).max(1.0);
    }
}

/// Maps a craft-pdf open failure to the importer's error.
#[cfg(not(target_arch = "wasm32"))]
fn open_error(error: craft_pdf::OpenError) -> IoError {
    use craft_pdf::OpenError;
    match error {
        OpenError::InputLimit { limit } => IoError::Unsupported(format!("PDF is larger than the {limit}-byte input limit")),
        OpenError::Allocation => IoError::Unsupported("PDF needs more memory than is available".into()),
        OpenError::BackendPanic => IoError::Unsupported("PDF reader failed on this file; it may be damaged".into()),
        OpenError::Load(error) => IoError::Unsupported(format!("PDF: {error:?}")),
    }
}

/// Maps a craft-pdf render failure to the importer's error.
#[cfg(not(target_arch = "wasm32"))]
fn render_error(page: usize, dpi: f64, error: raster::Error) -> IoError {
    use raster::Error;
    match error {
        Error::Cancelled => IoError::Cancelled,
        Error::OutputLimit => IoError::Unsupported(format!("PDF page {page} is too large at {dpi:.0} dpi; lower the import DPI")),
        Error::InvalidPage => IoError::Unsupported(format!("PDF page {page} has an empty or invalid page box")),
        Error::Allocation => IoError::Unsupported(format!("PDF page {page} needs more memory than is available")),
        Error::BackendPanic => IoError::Unsupported(format!("PDF renderer failed on page {page}; it may be damaged")),
        Error::InvalidOptions | Error::BufferLength => IoError::Unsupported(format!("PDF page {page} could not be rendered")),
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use lopdf::{Document as PdfDocument, Object, Stream, dictionary};

    use super::*;

    /// A one-page PDF whose content stream is `operators` short strokes: rendering costs time
    /// per operator, not per pixel.
    fn dense_operator_pdf(operators: usize) -> Result<Vec<u8>, lopdf::Error> {
        let mut pdf = PdfDocument::with_version("1.5");
        let pages_id = pdf.new_object_id();
        let mut content = String::with_capacity(operators.saturating_mul(24));
        content.push_str("0 0 1 RG 0.5 w\n");
        for i in 0..operators {
            let x = (i % 200) as f32 * 0.5;
            let y = ((i / 200) % 100) as f32 * 0.5;
            content.push_str(&format!("{x:.1} {y:.1} m {:.1} {y:.1} l S\n", x + 0.4));
        }
        let content_id = pdf.add_object(Stream::new(dictionary! {}, content.into_bytes()));
        let page_id = pdf.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => Object::Array(vec![Object::Real(0.0), Object::Real(0.0), Object::Real(200.0), Object::Real(100.0)]),
            "Contents" => content_id,
            "Resources" => dictionary! {},
        });
        pdf.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => Object::Array(vec![Object::Reference(page_id)]),
                "Count" => 1,
            }),
        );
        let catalog = pdf.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        pdf.trailer.set("Root", catalog);
        let mut bytes = Vec::new();
        pdf.save_to(&mut bytes)?;
        Ok(bytes)
    }

    /// Hundreds of thousands of path operators take far longer than the watcher's polling
    /// interval, so the interrupt lands mid-render and the page reports Cancelled. A million
    /// strokes leaves a wide margin even on a fast, optimised test build.
    #[test]
    fn a_dense_page_stops_when_the_interrupt_fires_mid_render() {
        let pdf = dense_operator_pdf(1_000_000).expect("writing the dense PDF");
        let document = craft_pdf::open_slice_with_options(&pdf, None, &craft_pdf::OpenOptions::default()).expect("the dense PDF opens");
        let cache = RenderCache::new();
        let options = raster::Options {
            scale: [1.0, 1.0],
            size: Some([200, 100]),
            offset: [0.0, 0.0],
            background: [255, 255, 255, 255],
            color_replacement: None,
            max_coverage: None,
        };
        let polls = AtomicUsize::new(0);
        // `render_page` checks once before spawning; the watcher then polls every 5 ms. Ten
        // polls put the cancellation about 45 ms into a render that takes much longer.
        let cancel = || polls.fetch_add(1, Ordering::SeqCst) >= 9;
        let ctl = Interrupt::cancel_only(&cancel);
        let error = match render_page(&document.pages()[0], &cache, &options, &ctl) {
            Err(error) => error,
            Ok(_) => panic!("the dense page rendered to completion before the interrupt fired"),
        };
        assert!(matches!(render_error(1, 72.0, error), IoError::Cancelled), "the render error maps to Cancelled");
        assert!(polls.load(Ordering::SeqCst) >= 10, "the watcher must keep polling the interrupt");
    }
}
