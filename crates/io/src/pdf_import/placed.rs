//! A PDF page drawn the way another application placed it as a smart object (#15650).
//!
//! Photoshop stores a placed PDF's frame (`Sz  `): the box the page was cropped to, in the
//! units the layer's corners map from. Its default crop for Illustrator artwork is the
//! artwork's bounding box, not the page. The page is drawn on transparency, cropped to the box
//! whose proportions match the frame (the artwork's bounds or the whole page) and scaled to fill
//! the frame exactly. With a stored resolution (`Rslt`) the box must also have the frame's size
//! in points at it; when both boxes do, the nearer wins. Other page boxes (trim, bleed, art) are
//! not tried.

use craft_pdf::raster::{self, RenderCache};
use craft_pdf::syntax::page::Page;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_geom::Size;
use photocraft_raster::{Interrupt, Surface};

use super::{page_size, render_error, render_page, write_raster};
use crate::IoError;
use crate::smart_map::PlacedPdf;

/// The longer side of the render that finds the artwork's bounds, in pixels.
const PROBE_SIDE: f64 = 2048.0;

/// A box on the page in points, top-left origin as rendered.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Box2 {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

impl Box2 {
    fn width(self) -> f64 {
        self.x1 - self.x0
    }
    fn height(self) -> f64 {
        self.y1 - self.y0
    }
}

/// The placed page of `bytes`, cropped as it was placed and drawn into a transparent frame (the
/// stored `Sz  `, rounded up to whole pixels). An error when the PDF can't be drawn completely or
/// neither the artwork's bounds nor the page has the frame's proportions: the caller then keeps
/// the pixels the file was saved with. Running out of memory is [`IoError::OutOfMemory`], the
/// one failure that may not happen again.
pub fn import_placed(name: &str, bytes: &[u8], placed: &PlacedPdf, ctl: &Interrupt) -> Result<Document, IoError> {
    let (frame_w, frame_h) = placed.frame;
    if !frame_w.is_finite() || !frame_h.is_finite() || frame_w < 1.0 || frame_h < 1.0 {
        return Err(IoError::Unsupported("the placed PDF's frame has no area".into()));
    }
    let (width, height) = (frame_w.ceil() as u32, frame_h.ceil() as u32);
    if width > raster::MAX_SIDE || height > raster::MAX_SIDE || u64::from(width) * u64::from(height) > raster::MAX_PIXELS {
        return Err(IoError::Unsupported("the placed PDF's frame is too large to draw".into()));
    }
    let pdf = craft_pdf::open_slice_with_options(bytes, None, &craft_pdf::OpenOptions::default()).map_err(|e| match e {
        craft_pdf::OpenError::Allocation => IoError::OutOfMemory("the placed PDF needs more memory than is available".into()),
        e => super::open_error(e),
    })?;
    let number = placed.page.max(1) as usize;
    let page = pdf.pages().get(number - 1).ok_or_else(|| IoError::Unsupported(format!("the placed PDF has no page {number}")))?;
    let cache = RenderCache::new();
    let crop = placed_box(page, &cache, number, placed, ctl)?.ok_or_else(|| {
        IoError::Unsupported("can't tell how the placed PDF was cropped: neither its artwork nor its page has the proportions it was placed with".into())
    })?;
    let scale = [frame_w / crop.width(), frame_h / crop.height()];
    let options = raster::Options {
        scale: scale.map(|s| s as f32),
        size: Some([width, height]),
        offset: [(crop.x0 * scale[0]) as f32, (crop.y0 * scale[1]) as f32],
        background: [0; 4],
        color_replacement: None,
    };
    let rendered = render(page, &cache, &options, number, ctl)?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let mut surface = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
    write_raster(&mut surface, 0, width, rendered.bytes(), true, "the placed PDF", ctl).map_err(|e| match e {
        IoError::Unsupported(why) => IoError::OutOfMemory(why),
        e => e,
    })?;
    surface.prune();
    let mut doc = Document::new(name, Size::new(width, height), ColorMode::Rgb, SampleType::U8);
    doc.layers.push(Layer::new(format!("Page {number}"), LayerContent::Raster(surface)));
    Ok(doc)
}

/// [`render_page`] for the placed path: a render missing an image or other content is refused
/// rather than drawn in place of the saved pixels, and only an allocation failure is
/// [`IoError::OutOfMemory`].
fn render<'a>(page: &'a Page<'a>, cache: &RenderCache<'a>, options: &raster::Options, number: usize, ctl: &Interrupt) -> Result<raster::Raster, IoError> {
    let rendered = render_page(page, cache, options, ctl).map_err(|e| render_failure(number, options.scale[0], e))?;
    let missing = match rendered.diagnostics {
        d if d.image_decode_failed => "an image could not be decoded",
        d if d.content_truncated => "some content was omitted (a decode or memory limit)",
        _ => return Ok(rendered),
    };
    Err(IoError::Unsupported(format!("PDF page {number} can't be drawn completely: {missing}")))
}

fn render_failure(number: usize, scale: f32, error: raster::Error) -> IoError {
    match error {
        raster::Error::Allocation => IoError::OutOfMemory(format!("PDF page {number} needs more memory than is available")),
        e => render_error(number, 72.0 * f64::from(scale), e),
    }
}

/// Whether a `width` × `height` box (in pixels of the render it was measured in) has the
/// frame's proportions, allowing for the antialiased pixel at each edge.
fn fits(width: f64, height: f64, frame: (f64, f64)) -> bool {
    let expected = width * frame.1 / frame.0;
    (height - expected).abs() <= 2.0 + 0.01 * height
}

/// Whether `b` (in points) is the frame's size at `dpi`, allowing for the antialiased pixel at
/// each edge of the frame.
fn sized(b: Box2, frame: (f64, f64), dpi: f64) -> bool {
    let near = |points: f64, pixels: f64| (points * dpi / 72.0 - pixels).abs() <= 2.0 + 0.01 * pixels;
    near(b.width(), frame.0) && near(b.height(), frame.1)
}

/// The box the page was placed with: its artwork's bounds or the whole page, whichever has the
/// frame's proportions and, with a stored resolution, its size in points at it (the nearer
/// when both do). Without a resolution the artwork wins. `None` when neither does.
fn placed_box<'a>(page: &'a Page<'a>, cache: &RenderCache<'a>, number: usize, placed: &PlacedPdf, ctl: &Interrupt) -> Result<Option<Box2>, IoError> {
    let frame = placed.frame;
    let (page_w, page_h) = page_size(page, number)?;
    let whole = Box2 { x0: 0.0, y0: 0.0, x1: page_w, y1: page_h };
    let probe = PROBE_SIDE / page_w.max(page_h);
    let size = [(page_w * probe).floor().max(1.0) as u32, (page_h * probe).floor().max(1.0) as u32];
    let art = match opaque_bounds(page, cache, number, probe, [0.0, 0.0], size, ctl)? {
        Some(art) if fits(art.width(), art.height(), frame) => {
            let coarse = Box2 { x0: art.x0 / probe, y0: art.y0 / probe, x1: art.x1 / probe, y1: art.y1 / probe };
            refine(page, cache, number, coarse, probe, frame, ctl)?
        }
        _ => None,
    };
    let page_fits = fits(page_w * probe, page_h * probe, frame);
    let candidates = [art, page_fits.then_some(whole)].into_iter().flatten();
    Ok(match placed.dpi {
        None => candidates.into_iter().next(),
        Some(dpi) => {
            let off = |b: &Box2| (b.width() * dpi / 72.0 / frame.0).ln().abs();
            candidates.filter(|&b| sized(b, frame, dpi)).min_by(|a, b| off(a).total_cmp(&off(b)))
        }
    })
}

/// `coarse` (artwork bounds measured at `probe` pixels per point) measured again with the
/// artwork at least as wide as the frame and [`PROBE_SIDE`], so the crop is exact to a fraction
/// of a pixel of the result. `None` when that render would be too large, finds nothing, or
/// measured this finely the artwork doesn't have the frame's proportions after all.
fn refine<'a>(
    page: &'a Page<'a>,
    cache: &RenderCache<'a>,
    number: usize,
    coarse: Box2,
    probe: f64,
    frame: (f64, f64),
    ctl: &Interrupt,
) -> Result<Option<Box2>, IoError> {
    let scale = frame.0.max(frame.1).clamp(PROBE_SIDE, 2.0 * PROBE_SIDE) / coarse.width().max(coarse.height());
    let margin = 2.0 / probe;
    let x0 = ((coarse.x0 - margin).max(0.0) * scale).floor();
    let y0 = ((coarse.y0 - margin).max(0.0) * scale).floor();
    let size = [((coarse.x1 + margin) * scale).ceil() - x0, ((coarse.y1 + margin) * scale).ceil() - y0];
    if !scale.is_finite() || size.iter().any(|&s| !(1.0..=f64::from(raster::MAX_SIDE)).contains(&s)) || size[0] * size[1] > raster::MAX_PIXELS as f64 {
        return Ok(None);
    }
    let found = opaque_bounds(page, cache, number, scale, [x0, y0], size.map(|s| s as u32), ctl)?;
    Ok(found.filter(|b| fits(b.width(), b.height(), frame)).map(|b| Box2 {
        x0: (x0 + b.x0) / scale,
        y0: (y0 + b.y0) / scale,
        x1: (x0 + b.x1) / scale,
        y1: (y0 + b.y1) / scale,
    }))
}

/// The pixel bounds of everything drawn on a transparent render of the page at `scale` pixels
/// per point, from `offset` over `size` pixels; `None` when nothing is drawn.
fn opaque_bounds<'a>(
    page: &'a Page<'a>,
    cache: &RenderCache<'a>,
    number: usize,
    scale: f64,
    offset: [f64; 2],
    size: [u32; 2],
    ctl: &Interrupt,
) -> Result<Option<Box2>, IoError> {
    let options = raster::Options { scale: [scale as f32; 2], size: Some(size), offset: offset.map(|o| o as f32), background: [0; 4], color_replacement: None };
    let rendered = render(page, cache, &options, number, ctl)?;
    let width = size[0] as usize;
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for (y, row) in rendered.bytes().chunks_exact(width * 4).enumerate() {
        let px = row.as_chunks::<4>().0;
        let Some(first) = px.iter().position(|p| p[3] > 0) else { continue };
        let last = px.iter().rposition(|p| p[3] > 0).unwrap_or(first);
        (x0, x1) = (x0.min(first), x1.max(last + 1));
        (y0, y1) = (y0.min(y), y + 1);
    }
    Ok((x0 < x1).then_some(Box2 { x0: x0 as f64, y0: y0 as f64, x1: x1 as f64, y1: y1 as f64 }))
}

#[cfg(test)]
mod tests;
