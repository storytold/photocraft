//! VectorCraft documents (`.vectorcraft`, and `.drawcraft` from before the rename), read only: the
//! first artboard is drawn from the PDF the file carries when VectorCraft saved it with Create
//! PDF-Compatible File, as one layer at 72 pixels per inch (one pixel per point). `craft-doc`
//! reads the file and `craft-pdf` draws the page. A file without that PDF is refused with an
//! error that names the option. Placed, the file becomes an embedded or linked smart object,
//! drawn again from the PDF at its placement scale ([`import_scaled`]).

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use craft_doc::{Limits, RenditionKind};
use craft_pdf::interpret::InterpreterSettings;
use craft_pdf::raster::{self, Options, Raster, RenderCache};
use craft_pdf::syntax::page::Page;
use photocraft_codecs::{ChannelLayout, Image, SampleType};
use photocraft_raster::Interrupt;

use crate::{ImportResult, IoError};

/// Extensions recognized as VectorCraft documents.
pub const EXTENSIONS: &[&str] = &["vectorcraft", "drawcraft"];

/// Pixels per inch of the drawn artboard at scale 1: one pixel per PDF point.
const PPI: f32 = 72.0;

pub fn has_extension(name: &str) -> bool {
    name.rsplit(['/', '\\']).next().and_then(|n| n.rsplit_once('.')).is_some_and(|(_, ext)| EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
}

/// Whether `bytes` start like a VectorCraft document (plain or gzip JSON).
pub fn is_vectorcraft(bytes: &[u8]) -> bool {
    craft_doc::sniff(bytes) == Some(craft_doc::Kind::VectorCraft)
}

pub(crate) fn import(name: &str, bytes: &[u8], ctl: &Interrupt) -> Result<ImportResult, IoError> {
    import_scaled(name, bytes, 1.0, ctl)
}

/// The first artboard drawn at `scale` times the size [`import`] draws it at (one pixel per
/// point, or less for an artboard past `craft-pdf`'s raster limits), and at most at the largest
/// scale within those limits ([`scale_limit`]). The document's resolution is 72 pixels per inch
/// times the pixels per point it was drawn at.
pub fn import_scaled(name: &str, bytes: &[u8], scale: f32, ctl: &Interrupt) -> Result<ImportResult, IoError> {
    if !scale.is_finite() || scale <= 0.0 {
        return Err(err(&format!("can't draw the artboard at scale {scale}")));
    }
    let pdf = carried_pdf(bytes)?;
    let pages = pdf.pages();
    let (page, w, h) = first_page(pages)?;
    let opened_scale = max_scale(w, h).min(1.0);
    let (open_w, open_h) = opened_size(w, h);
    let relative = scale.min(relative_limit(open_w, open_h));
    let drawn_scale = opened_scale * relative;
    // The viewport is the size `import` draws at times `relative`, so a page whose size in points
    // is fractional or under one point shows the same part of the artboard at every scale.
    let side = |opened: u32| (opened as f32 * relative).round().max(1.0) as u32;
    let mut size = [side(open_w), side(open_h)];
    if size.iter().any(|s| *s > raster::MAX_SIDE) || u64::from(size[0]) * u64::from(size[1]) > raster::MAX_PIXELS {
        // Rounding up passed a limit at its edge; the sizes rounded down are inside it.
        size = [((open_w as f32 * relative).floor().max(1.0)) as u32, ((open_h as f32 * relative).floor().max(1.0)) as u32];
    }
    let options = Options { scale: [drawn_scale; 2], size: Some(size), ..Options::default() };
    let drawn = match render(page, &options, ctl) {
        Err(raster::Error::Cancelled) => return Err(IoError::Cancelled),
        r => r.map_err(|e| err(&format!("its PDF can't be drawn ({e})")))?,
    };
    let (width, height) = (drawn.width, drawn.height);
    let diagnostics = drawn.diagnostics;
    let mut rgba: Vec<u8> = drawn.bytes().to_vec();
    unpremultiply(&mut rgba);
    let img = Image::from_raw(width, height, ChannelLayout::Rgba, SampleType::U8, rgba)?;
    let mut r = crate::flat::image_to_document(name, &img)?;
    r.document.resolution_dpi = PPI * drawn_scale;
    // craft-pdf draws in sRGB.
    r.document.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
    if drawn_scale < scale {
        r.warnings.push(format!(
            "VectorCraft: the first artboard ({w:.0} × {h:.0} pt) is larger than {} pixels on a side or {} pixels in all at {:.1} pixels per inch, so it was drawn at {:.1} pixels per inch ({width} × {height} pixels).",
            raster::MAX_SIDE,
            raster::MAX_PIXELS,
            PPI * scale,
            PPI * drawn_scale
        ));
    }
    if pages.len() > 1 {
        r.warnings.push(format!("VectorCraft: only the first of the file's {} artboards was imported.", pages.len()));
    }
    if !diagnostics.is_empty() {
        r.warnings.push("VectorCraft: some of the artwork in the file's PDF could not be drawn.".into());
    }
    // Save never writes back over the VectorCraft file.
    r.source_read_only = true;
    Ok(r)
}

/// The largest `scale` [`import_scaled`] draws the file's first artboard at without reaching
/// `craft-pdf`'s raster limits, read from the carried PDF without drawing it.
pub fn scale_limit(bytes: &[u8]) -> Result<f32, IoError> {
    let pdf = carried_pdf(bytes)?;
    let pages = pdf.pages();
    let (_, w, h) = first_page(pages)?;
    let (open_w, open_h) = opened_size(w, h);
    Ok(relative_limit(open_w, open_h))
}

/// The largest scale relative to a `w` × `h` pixel opened size that keeps the drawn size within
/// `craft-pdf`'s raster limits.
fn relative_limit(w: u32, h: u32) -> f32 {
    max_scale(w as f32, h as f32)
}

/// The size in pixels [`import`] draws the file's first artboard at, read from the carried PDF
/// without drawing it.
pub fn drawn_size(bytes: &[u8]) -> Result<(u32, u32), IoError> {
    let pdf = carried_pdf(bytes)?;
    let pages = pdf.pages();
    let (_, w, h) = first_page(pages)?;
    Ok(opened_size(w, h))
}

/// The size in pixels of a `w` × `h` point page drawn at scale 1: `craft-pdf` floors the scaled
/// page size.
fn opened_size(w: f32, h: f32) -> (u32, u32) {
    let scale = max_scale(w, h).min(1.0);
    ((w * scale).floor().max(1.0) as u32, (h * scale).floor().max(1.0) as u32)
}

fn err(e: &dyn std::fmt::Display) -> IoError {
    IoError::VectorCraft(e.to_string())
}

/// The PDF the file carries, opened.
fn carried_pdf(bytes: &[u8]) -> Result<craft_pdf::syntax::Pdf, IoError> {
    let pdf = craft_doc::renditions(bytes, &Limits::default()).map_err(|e| err(&e))?.into_iter().find(|r| r.kind == RenditionKind::Pdf).ok_or_else(|| {
        IoError::VectorCraft("the file carries no PDF; save it in VectorCraft with Create PDF-Compatible File turned on, then try again".into())
    })?;
    craft_pdf::open(Arc::new(pdf.bytes), None).map_err(|e| err(&format!("its PDF can't be read ({e})")))
}

/// The first page (the first artboard) and its size in points.
fn first_page<'a>(pages: &'a [Page<'a>]) -> Result<(&'a Page<'a>, f32, f32), IoError> {
    let page = pages.first().ok_or_else(|| err(&"its PDF has no pages"))?;
    let (w, h) = page.render_dimensions();
    if !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0 {
        return Err(err(&"its first artboard has no size"));
    }
    Ok((page, w, h))
}

/// The largest scale (pixels per point) that keeps a `w` × `h` point page within `craft-pdf`'s
/// raster limits.
pub fn max_scale(w: f32, h: f32) -> f32 {
    let side = raster::MAX_SIDE as f32;
    let area = (raster::MAX_PIXELS as f64 / (f64::from(w) * f64::from(h))).sqrt() as f32;
    // A hair under the exact limit, so the floored size stays inside it after rounding.
    (side / w).min(side / h).min(area) * (1.0 - 1e-6)
}

/// Draw `page`, stopping when `ctl` is canceled. The renderer polls a flag between operators;
/// off wasm, a scoped thread sets that flag from `ctl` while the page is drawn.
fn render(page: &Page<'_>, options: &Options, ctl: &Interrupt) -> Result<Raster, raster::Error> {
    if ctl.cancelled() {
        return Err(raster::Error::Cancelled);
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    let settings = InterpreterSettings { cancelled: Some(cancelled.clone()), ..InterpreterSettings::default() };
    let draw = || raster::render(page, &RenderCache::new(), &settings, options);
    if cfg!(target_arch = "wasm32") {
        return draw();
    }
    let done = AtomicBool::new(false);
    std::thread::scope(|sc| {
        // Without the thread (the OS refused one), the page is drawn without polling `ctl`.
        let _watch = std::thread::Builder::new().spawn_scoped(sc, || {
            while !done.load(Ordering::Relaxed) {
                if ctl.cancelled() {
                    cancelled.store(true, Ordering::Relaxed);
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        });
        let r = draw();
        done.store(true, Ordering::Relaxed);
        r
    })
}

/// Premultiplied RGBA8 (what `craft-pdf` draws) to straight alpha, in place.
fn unpremultiply(rgba: &mut [u8]) {
    for px in rgba.as_chunks_mut::<4>().0 {
        let a = u32::from(px[3]);
        if a != 0 && a != 255 {
            for c in &mut px[..3] {
                *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
}
